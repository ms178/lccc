# Follow-up: GCC torture on x86-64 and i686 through lccc + lccc-ld

**Session date:** 2026-09-27
**Base:** `563fb0bc` (upstream `main`, PR #653). The session started on
`19cf2b3c` and was rebased once; the upstream delta, a vectorizer change, did
not interact with this work.
**Scope:** the x86-64 and i686 backends, the standalone `lccc-ld`, the
integrated x86 assembler's layout engine, and the torture scripts. No other
backend was touched.
**Deliverable:** `/home/user/ms178-1.patch`, snapshots `S01…` in
`/home/user/artifacts/SNAPSHOT_LEDGER.md`.

---

## 1. Harness state

* `scripts/ensure_gcc_torture.sh` now provisions the **GCC 16.2.0** release
  testsuite (1692 `execute` and 2003 `compile` sources). It is idempotent
  through a stamp file, downloads via `.part` then rename, and uses `--force`
  to re-extract. `x86_gcc_torture.py` defaults to that corpus and records it
  as `gcc_testsuite` in its JSON.
* **Both arches now link through the standalone `lccc-ld`.** The link is
  `gcc [-m32] -no-pie -B<shim>`, where `<shim>/ld` points at `lccc-ld`, so
  every PASS covers LCCC codegen and `lccc-ld` together. i686 used to link
  only through the driver's built-in linker; that path survives as
  `--link-mode=driver`.
* The runner now follows the DejaGnu rules it had been ignoring:
  * `dg-timeout-factor N` scales the compile and run timeouts.
  * `dg-require-effective-target run_expensive_tests` reports
    `unsupported`, unless `--expensive` or `GCC_TEST_RUN_EXPENSIVE` is set.
    This is gcc's own rule (target-supports.exp).
  * `torture_evidence.py` counts `unsupported` as a skip, not a failure.

### Baseline (before this session's fixes, x86-64, base `19cf2b3c`)

8460 cases (1692 sources × `-O0 -O1 -O2 -O3 -Os`), 1633 s at `-j2`:

| status | count | what it was |
|---|---|---|
| pass | 8409 | |
| run-fail | 3 | `20010106-1.c -O0`, `pr120630.c -O0`, `postmod-1.c -O1` (§2.1, §2.2) |
| compile-fail | 25 | `memclr.c` and `memcpy-a{1,2,4,8}.c` at all levels, all 60 s timeouts (§2.4) |
| reference skips | 23 | the host GCC cannot build or run them |

After the fixes, all 3 run-fails pass, and all 25 expensive cases pass with
`--expensive`: 25/25, where lccc `-O2` compile time is now below gcc's (§2.4).
The final full re-run numbers for both arches are in §4.

---

## 2. Defects fixed (root causes)

### 2.1 x86-64 peephole: a read-modify-write treated as a kill (`eliminate_dead_inplace_ext`)

Two torture failures had this one cause: `20010106-1.c -O0` and
`pr120630.c -O0`.

A dense switch whose smallest case is negative indexes its jump table with
`cltq; subq $min, %rax`. The pass accepted `subq` as a *redefinition* of
`%rax` and deleted the `cltq`. As a result, -1 became `0x00000000ffffffff`
and dispatch went to the default arm.

Acceptance now requires `writes_family_full`, or `is_full_write` for 64-bit
`%rax` lines. A pure full-width redefinition qualifies; an RMW does not.

Tests:
* Unit tests: `narrow_copy_fold_tests.rs` §C.
* Regression test: `tests/regression/switch_negative_table_index_ext.c`.
  It covers int/short/char scrutinees and straddling and all-negative
  ranges, and was mutation-checked against the pre-fix binary.

### 2.2 x86-64 peephole: half-applied multi-line folds on pinned lines (`postmod-1.c -O1`)

This one caused a SIGSEGV.

The prologue pinner marks any entry-block `movq %{rdi..r9}, %{rbx,r12..r15}`
as a "parameter pre-store". Here `%r9` was really a scratch holding
`(long)x`. `fold_movslq_relay` rewrote `movslq %edi, %r9` to
`movslq %edi, %rbx` and then called `mark_nop` on the pinned copy. `mark_nop`
**silently refuses** pinned lines, so the output was
`movslq %edi, %rbx; movq %r9, %rbx`, which reads an undefined `%r9`.

This is a class of bug, not a one-off:
* `replace_line` also drops a pin, because it re-classifies the line.
* A static census found about 40 functions that rewrite one line and delete
  another with no pin check.
* A dynamic census found the unsafe case only in `fold_movslq_relay`. It
  instrumented `mark_nop` with `#[track_caller]` and ran all 1692 `execute`
  sources at `-O1`/`-O2`. Refusals occurred at 4 sites:
  * `narrow_copy_fold.rs:494` (398 refusals) is safe. The renamed uses read
    the source, which the rules keep live, so the kept copy is only
    redundant.
  * `dead_code.rs:482` and `:1259` (306 refusals) are pure deletions. A
    refusal there is only a missed optimisation.
  * `memory_fold.rs:1004` (3 refusals) is unsafe.

Fix:
* A new commit gate, `pub(super) fn all_editable(infos, &[lines…])`
  (`x86/codegen/peephole/types.rs`), is documented at its definition.
* It is applied before the first edit of **all seven** relay folds in
  `memory_fold.rs` (`load`, `leaq`, `cltq`, `extend`, `general`, `movslq`,
  `store`), because all seven share the rewrite-then-delete shape.

Tests:
* 3 unit tests in `passes/mod.rs` `regression_tests`: pinned and unpinned
  direct calls, plus an end-to-end run through the driver's own pinning.
* `tests/regression/prologue_scratch_relay_pinned_copy.c`, mutation-checked:
  the old binary SIGSEGVs.

### 2.3 lccc-ld: i386 userspace links, and two shared-object bugs they exposed

* **`lccc-ld -m elf_i386` userspace links:** `gcc -m32 -B<shim>` used to be
  refused. It now goes through the i686 `link_builtin` for executables and
  `-shared` (`linker_entry.rs`). `--dynamic-linker` is honoured and sets
  `PT_INTERP`; `-pie` is refused with a diagnostic; `-r` still errors, as
  before.
* **ELF header clobbered by `.note`:** the shared layout never placed the
  merged `.note` section, which carries crtbeginS's `.note.gnu.property`.
  Its file offset stayed 0 and the write loop copied it over the ELF header.
  * It is now placed in the RO headers segment, as the executable emitter
    already did.
  * A new invariant check, `emit::check_sections_placed`, runs in both
    emitters: any ALLOC section with bytes at address 0 and offset 0 is now
    a hard link error, not silent corruption.
* **Shared-object PLT was not PIC:** it used `jmp *abs32` and
  `pushl abs32`. Every external call from a `.so` faulted once the object was
  loaded, and so did crtbeginS's `__cxa_finalize@plt` at exit. This affected
  the driver's own `lccc-i686 -shared` as well, whenever the library called
  anything external.
  * `build_plt` now takes `PltAddressing::{Absolute, EbxRelative}`. Shared
    objects use `ff b3`/`ff a3 disp32(%ebx)` relative to
    `_GLOBAL_OFFSET_TABLE_`, as the i386 psABI requires.
  * Verified lazily and with `LD_BIND_NOW=1`, in both directions:
    lccc-ld exe + gcc `.so`, and gcc exe + lccc-ld `.so`.
* Tests:
  * `emit.rs` unit tests: absolute and PIC PLT byte layout with round-trip
    displacements, plus the placement invariant.
  * `run_linker_tests.py` case `i386_gcc_driver_exe_and_shared` (filter
    `i386`). It covers TLS, libm, and a PLT call from a `.so`, and FAILs
    against the pre-fix `lccc-ld`.

### 2.4 Integrated assembler: quadratic layout passes (compile time)

gcc's harness treats `memclr.c` and `memcpy-a*.c` as "expensive" tests.
lccc needed **253 s** for `memclr.c -O2`, where gcc needs 12 s. Almost all of
it went to the integrated assembler: 9 MB of assembly and 41k labels, which
GNU as handles in under 1 s. Profiled with gdb stack sampling (no PMU on this
host):

1. **`relax_jumps`** applied each jump shrink or grow on its own. Each one
   did a `drain`/`splice` of the section, a `retain` over all relocations,
   and a full `shift_after` remap (every label, relocation, jump, marker and
   deferred record, with a string-hash `label_seq` lookup per label). That is
   O(jumps × records) per pass.
   * Now: `apply_jump_transitions` does one rebuild and one remap through a
     prefix-sum map. Back-to-front sequential application saw original
     coordinates, so the composition is exact.
2. **`fixup_alignment_markers`** had the same shape per marker, and became
   the hot spot once (1) was fixed.
   * The sweep is inherently sequential, because each marker's padding
     depends on the markers re-padded before it.
   * Now: edits are queued in original coordinates (`PadQueue`), and every
     position the sweep reads goes through the queue's map. `shift_after`'s
     tie rule becomes a lexicographic `(anchor, seq)` prefix test. Up-set
     shifts preserve record order, and markers are visited in anchor order,
     which is why this works. A queue whose next anchor would break the sort
     is flushed first, so exactness never depends on that argument alone.
   * Back-edge lookups for tight-loop markers use a per-sweep index instead
     of scanning every jump for every marker.

Results:

| input | before | after | GNU as / gcc |
|---|---|---|---|
| `memclr.c -O0` asm → .o | 156 s | 4 s (relax) | < 1 s (as) |
| `memclr.c -O2` asm → .o | 58 s (after relax fix) | 2 s | |
| `memclr.c -O2` full compile | 253 s | 11.5 s | 12.4 s (gcc) |
| `memcpy-a8.c -O2` full compile | > 60 s (timeout) | 10.7 s | 14.0 s (gcc) |

**Output is byte-identical.**
* Old and new builtin assemblers gave identical `.o` for every torture
  `execute` source at `-O0` and `-O2`, on **both arches**: 6735/6735 SAME
  after the relax change, then re-run after the alignment change (§4), plus
  memclr at `-O0` and `-O2`.
* The GAS-2.47 asm-diff gate in `ci_local.sh` covers `.org`, tight-loop and
  `.code16` layouts.
* `pad_queue_tests` pin the tie rule.

### 2.5 Shared stack layout: block-local slots placed below the frame (i686 `memcpy-a*.c -O1`)

Once §2.4 made the expensive tests compile, the i686 leg failed
`memcpy-a{1,2,4,8}.c -O1` (abort). Reduced to one function:

```c
__builtin_memcpy(dst.v + 8, src.v + 8, 5);   /* static dst, global src */
```

emitted `subl $12, %esp … movl %eax, -4(%esp)` for the source pointer, then
`pushl %esi; pushl %edi` (the memcpy's own register save) overwrote it, so
`rep movsb` copied from garbage.

**Root cause.** `classify_value` widens the slot of a pointer that feeds a
`Memcpy` to the copy width, capped at 32. A raw width such as 5, 12 or 20 is
non-canonical. The block-local (Tier-3) pool packed such entries at their
**exact** size, but `finalize_deferred_slots` places each one with the arch
closure, and every closure rounds the allocation up (to 4 on i686, to 8
elsewhere).

Worked example on i686, base 16:
* pool offsets 0 and 5, region `max_block_local_space = 10`;
* placed at depths 24 and **32**;
* frame covers 26.

The x86-64 closure has the same mismatch. `CCC_DEBUG_SLOTS` showed
x86-64 `-O1` slots 4–12 bytes past the raw region for 12/15/20/31-byte
copies. There the frame's 16-byte rounding (or a register home for the
pointer) happened to hide it.

**Fix** (all in `stack_layout/slot_assignment.rs`; behaviour changes only
where frames were unsound):
1. The memcpy widening rounds to a canonical class, `next_power_of_two`
   with a minimum of 8 (8/16/32). This fixes the source. Canonical sizes are
   also the only ones the pool's exact-size free lists may share.
2. A fresh Tier-3 allocation of any non-canonical size is placed 8-aligned
   and reserves its rounded extent. That is what every arch closure
   occupies, so the pool stays exact if such a size ever reappears.
3. `finalize_deferred_slots` returns at least the deepest placement: a slot
   can never again fall outside the frame. It also `debug_assert!`s that the
   pool accounting already covered it. That fires in dev-profile builds;
   fastbuild/CI inherit release and do not have debug assertions.
   * Validation: a temporary unconditional report at the same check,
     never committed, ran over all 1692 `execute` sources at
     `-O0/-O1/-O2/-Os` on both arches (13.5k compiles). It found zero
     placements past the pool end.

Regression test: `tests/regression/i686_memcpy_operand_slot_in_frame.c`
(`-m32 -O1`, copy widths 5–31). It returns 2 on the pre-fix compiler.

### 2.6 Lowering: 64-bit `__builtin_*_overflow` on 32-bit targets (i686 `pr91450-1/-2`, `pr93494`)

The generic overflow lowering computes in a type wide enough for the exact
result. `compute_{signed,unsigned}_overflow` widen a 64-bit multiply to
**I128**.

On i686, I128 is not an arithmetic type: GCC has no `__int128` there, and
the backend carries only the low 64 bits. The results were:

* **Wrong answers at `-O0`:**
  * `__builtin_mul_overflow(i64 2^40, i64 2^30, &i64)` returned 0;
  * `__builtin_add_overflow(~0ULL, 1, &u64)` returned 0.
* **ICE at `-O1+`:** "wide i686 cast reached the scalar cast emitter", for
  narrow-result forms such as `&unsigned short`. The i128 → narrow cast of
  `emit_cast_default` calls `emit_cast_instrs(I64, …)`.

**Fix:** `lower_overflow_u64_pair` (`ir/lowering/expr_builtins_overflow.rs`).
* It forms the exact result as a two's-complement 128-bit pair `(hi, lo)`
  using only 64-bit IR:
  * add/sub carry or borrow out of `lo`;
  * products use `umulhi` from four 32×32→64 partial products, with the
    signed-high correction `- (ahi & blo) - (bhi & alo)`.
* It then tests the result type directly.
* It is taken only on 32-bit targets, and only where the old lowering would
  have needed I128 (`overflow_needs_u64_pair`: a compute type wider than
  64 bits, or a multiply touching a 64-bit operand or result). The cheap
  single-width forms (e.g. SQLite's i64 add) are unchanged, and x86-64 is
  unaffected.
* It covers the generic, type-specific (`__builtin_smulll_overflow` …) and
  `_p` forms.

Test: `tests/regression/i686_overflow_builtins_64bit.c` (`-m32 -O1`).
* 26 edge cases, cross-checked against gcc `-m32` and gcc x86-64.
* It ICEs on the pre-fix compiler.

### 2.7 i686 peephole: re-associated subtract-compare for ordered conditions (i686 `pr45034 -O2/-O3/-Os`)

Pattern 7 in `backend/i686/codegen/peephole.rs` fused
`movl %A,%B; subl $a,%B; cmpl $b,%B; jCC` into `cmpl $(a+b),%A; jCC`. It
claimed identical flags "for every condition code". That is false:
* both compares produce the same 32-bit difference, so ZF, SF and PF agree;
* CF and OF are the borrow/overflow of *different* subtractions.

Example: `if (y < -128 || y > 127)` lowers to
`(unsigned)(y + 128) > 255`, which became `cmpl $127, y; ja`, i.e.
`y >u 127`, and aborted for every negative y. Any
`(unsigned)(x - a) OP b` range check behind a staging copy was exposed
(`isdigit`-style idioms included).

**Fix:** the fusion now requires every reader of the fused compare's flags
to be ZF/SF-only (`je/jne/js/jns`, `sete/…`). It reuses
`flags_reader_window_ok`, which follows the fallthrough chain and
unconditional jumps, not just the adjacent jCC.
* The unit test that enshrined the wrong `ja` fusion
  (`fold_reg_copy_range_check_fuses_sub_cmp`) is replaced by three tests:
  equality still fuses; all ordered/overflow conditions do not; a `jne`
  followed by a fallthrough `ja` does not.

Test: `tests/regression/i686_sub_cmp_range_check.c` (`-m32 -O2`). It aborts
on the pre-fix compiler.

Follow-up (perf): GCC emits `leal -a(%A), %B; cmpl $b, %B` for these range
checks, one instruction shorter than the surviving `movl; subl; cmpl`. That
is a sound rewrite for all conditions (§5).

### 2.8 Lowering: string initializers of every character width

Found by red-teaming the S03 string-bounds fix, not by the torture suite. It
is the same bug class, and the torture suite does not cover it.

*Symptoms.* All of these compared against gcc:
* A `char16_t`/`wchar_t`/`char32_t` array that is a member of a struct got the
  literal's *address*, because only `char`/`unsigned char` members were
  recognised as string targets.
* Multi-dimensional arrays of wide characters (`wchar_t m[3][4] = { L"ab",
  L"wxyz" }`) dropped or shifted their rows in all three storage classes.
* `u"\U0001F600"` counted as one unit in some sizes (FAM, unsized arrays).

*Root cause.* There were five independent implementations of "store a string
literal into an array". Each block-scope, static, compound-struct,
compound-with-pointers and compound-literal path decided on its own which
element types take a string, how many units it has, how UTF-16 encodes, and
where the terminator goes. Every one of them hard-coded the `char` case
somewhere.

*Fix.* A single `StringInit` value (`src/ir/lowering/string_init.rs`) answers
all of these for a literal and a target element type:
* `for_elem_ctype`/`for_elem_ir`/`for_init_ctype` decide whether the pair is a
  character-array initialization (C11 6.7.9p14/p15; the literal's kind and
  the element type must match in width).
* It provides the unit count including surrogate pairs, the stored units
  for a given array bound (the terminator is dropped when the array only
  has room for the characters), a byte-image writer and IR constants.

Every consumer calls it. The per-path copies (`write_string_to_bytes`,
`inline_string_to_values` and the narrow-only checks) are deleted.

### 2.9 Lowering: one planner for array initializers (brace elision, designators, overrides)

*Symptoms* (block-scope, static and file-scope storage, both arches; gcc as
oracle, e.g. `int a[2][3] = { [0][2] = 9, [0] = { 4 } }`, `T m[3][3] = { [2]
= "hi", [0] = "a" }`, `struct { int a[2][2]; int g; } v = { 1, 2, 3, 4, 5
}`):
* rows skipped or doubled when a designator named a row and elements
  followed;
* excess initializers in a braced row spilled into the next row or member;
* a braced row override did not reset the row's earlier elements;
* `_Bool` elements were stored unnormalised in the static byte image;
* multi-dimensional members inside structs smashed the stack. The local path
  stored an `IrType::from_ctype(Array)` value element-wise.

*Root cause.* Each storage path walked the syntactic initializer list with
its own flat-index arithmetic. The old global flatten engine used
`flat = max(flat, end)` after a braced sub-list, which lets deeper excess
move later rows. The static pre-pass that decides whether a struct needs a
relocation-carrying image (`struct_init_has_addr_fields`) consumed items
differently from the byte writer. So an array member's continuation items
were matched against the wrong fields, and the struct was sent down the
wrong path.

*Fix.* `src/ir/lowering/array_init_plan.rs` turns an array initializer into
an ordered list of placements:
* `Scalar{flat, expr}`, `Row{flat, string, row_elems}` and `Zero{flat,
  elems}`, with C11 6.7.9 semantics. Designators address any level. Every
  braced level is bounded by its sub-object (`limit`), so excess is
  diagnosed-and-dropped at that level.
* A braced override zeroes its whole sub-object first. A scalar override
  does not touch its siblings. A string row initializes its whole row.
* A brace-elided member run can meet earlier writes from other runs of the
  same initializer, e.g. `.a[0][2] = 9, .a[0] = { 4 }`. So in such a run
  every braced subobject and every string row resets its range
  (`PlanSink::prior_unknown`). Before this change, statics and locals both
  kept the 9.
* Brace-elided runs (`plan_array_init_elided`, `plan_array_member_run`)
  report how many items they consumed.

Consumers:
* locals: stores plus `zero_fill_range`;
* static byte images: `array_placements_to_bytes`, `_Bool` normalised;
* compound (relocation) images for pointer arrays and structs with pointer
  members;
* compound literals.

Every static walker takes an array member's items through
`plan_array_member_run`, so all of them consume the same number of items.
The walkers are the address pre-pass, the byte writer, the compound walker
and the three compound-with-pointers walkers. The relocation-carrying image
writer (`array_member_run_to_image`) extends the planner to **pointer
elements at any rank**:
* Addresses become relocations; constants become bytes.
* A range an override rewrites drops the relocations recorded in it
  earlier. Placements normally ascend, so that check costs one scan per run
  plus one per real override, not one per element.

Before this, `struct { const char *a[2][2]; int k; } = { s1, 0, 0, s2, 4 }`
took one item per *row* and left `k` holding an address. Designated pointer
elements (`.q[1][0] = &x`) were lost.

Other brace-elision fixes:
* **Top-level relocation-carrying structs** (`global_init_compound_struct.rs`)
  now build every array field of scalars or aggregates as a `FieldImage`.
  Its runs are written in item order by the same walkers, so later runs
  override earlier ones. Before, it counted "flat scalars" per struct
  element and stopped at the first braced element:
  `{ s1, 1, &x, 2, &y, { 3, &z }, 99 }` put `3` in the tail member.
* **Arrays of pointer-carrying structs** with elided element braces
  (`{ {0,"a","b"}, 0, "bc", "d", "e" }`, including arrays of such structs
  inside members) now take items member by member. Before, they took one
  item per field.

About 900 lines of per-path flattening were deleted.

*Tests.* `static_pointer_array_members.c` (x86-64 and i686) covers pointer
members of any rank: elided, designated, overridden (relocation → 0,
constant → relocation), row reset, excess, nested, arrays of pointer structs
with elided braces (1-D and 2-D), unions and arrays of structs. It compares
pointers and never prints them. `tests/regression/gen_array_init_matrix.py`
generates `array_init_matrix.c`: 432 objects crossing the element kinds with the
shapes and storage classes above. Every object is hashed bytewise and
block-scope objects sit on a stack pre-filled with 0x5a. It runs at -O2 and
-O0 on x86-64 and i686 (`{,i686_}array_init_matrix{,_O0}`), and gcc is the
oracle.

---

## 3. Harness and tooling changes

* `x86_gcc_torture.py`:
  * `--link-mode`, i686 via lccc-ld.
  * `dg-timeout-factor`, `run_expensive_tests` → `unsupported`,
    `--expensive`.
  * JSON gains `link_mode`, `gcc_testsuite` and `expensive`.
* `torture_evidence.py`: `unsupported` is a skip.
* `ensure_gcc_torture.sh`: rewritten, pinned to 16.2.0.
* `tools/linker/setup_oracles.sh` and README:
  * Pins: bfd 2.47 (ld only), mold 2.42.1 built with
    `-DMOLD_TARGETS='X86_64;I386'`, lld 23.1.x from apt.llvm.org, wild HEAD.
  * Paths: prefix `/home/user/artifacts/oracles`, wrappers in
    `/home/user/artifacts/bin`, versions in `ORACLES.lock`.
  * Not run this session (§5).

---

## 4. Final torture numbers

Final build: every fix in §2 applied. GCC 16.2 torture, `execute/`, all 5
optimisation levels, `--expensive`, linked through `lccc-ld`
(`scripts/x86_gcc_torture.py --arch <a> --expensive -j2`, 2 vCPU):

| arch | cases | pass | run-fail | compile-fail | reference skips | wall |
|---|---|---|---|---|---|---|
| x86-64 | 8460 | **8437** | 0 | 0 | 5 compile + 18 run | 723.9 s |
| i686 | 8460 | **8425** | 0 | 0 | 20 compile + 15 run | 758.8 s |

"Reference skips" are cases the reference compiler (host gcc 14, `-m32`
for i686) cannot build or run on this host, e.g. nested-function
trampolines on a non-executable stack. They are not lccc results.

Progression on the same suite:
* **x86-64:** 8409 pass, 3 run-fail, 25 compile-fail, 1633 s at base
  `19cf2b3c` (§1) → 8437 pass, 0 fail, 724 s.
* **i686, before §2.5–2.7:** 8409 pass, 7 run-fail, 9 compile-fail:
  * run-fail: `memcpy-a{1,2,4,8} -O1`, `pr45034 -O2/-O3/-Os`;
  * compile-fail: `pr91450-{1,2} -O1..-Os`, `pr93494 -O0`.

---

## 5. To-do (ordered by value/risk)

1. **Over-pinning in the prologue pinner.** The textual rule "any
   `movq %argreg, %callee_saved` before the first branch/call" pins
   scratch copies too. Its refusals include 398 missed copy folds in the
   census.
   * Proper fix: pin only registers that are ABI parameters of *this*
     function, using the `LCCC_PARAM_ABI_READ` / arity information the
     emitter already has.
2. **Audit the remaining ~40 rewrite-then-delete folds** (list in §2.2)
   and route them through `all_editable`, or give `mark_nop` a `#[must_use]`
   bool so a refusal cannot be ignored. Only `fold_movslq_relay` is proven
   unsafe on the corpus, but the others are one shape away.
3. **RMW-as-kill audit** of the remaining `writes_family` acceptance uses in
   `dead_code.rs` and `memory_fold.rs` (same bug class as §2.1).
4. **i686 output has no section headers** (`file`: "no section header").
   gdb, objdump and readelf `-S` are blind. Emit `.shstrtab`, `.symtab` and
   section headers the way the x86-64 emitter does.
5. i686 `lccc-ld -r` and `-pie` (ET_DYN executables). Also: the i686 exe
   emitter ignores `-rpath`/`DT_RUNPATH`, and no `PT_GNU_PROPERTY`/`PT_NOTE`
   is emitted for `.note.gnu.property`.
6. Run `tools/linker/setup_oracles.sh` to build mold, bfd and lld and
   refresh `ORACLES.lock`. Then run the Godbolt/codegen oracles on the
   zlib-ng, gzip and expat workloads.
7. A torture `compile/` leg: compile-only, 2003 sources, with ICE and
   timeout detection.
8. i686 peephole: the ordered-condition range checks that §2.7 stopped
   fusing can use `leal -a(%A), %B; cmpl $b, %B`: the copy and subtract
   collapse soundly into one `lea`, which leaves flags alone. This is what
   GCC emits.
9. Integrated assembler: `resolve_deferred_skips` still applies splices one
   by one (O(skips × records)). Batch it like §2.4 if a workload with many
   `.skip` expressions shows up.

10. **Designator continuation and struct-level overrides** (repros are
    the matrix entries generated with `gen_array_init_matrix.py
    --include-known-gaps`, plus the cases below). These are gcc results;
    lccc differs in each:
    * `struct O { struct I { int a, b; } in; int c, d; }`:
      `{ .in.a = 1, 2, 3, 4 }` gives `1 2 3 4`. The item after a nested
      member designator must continue *inside* `in` (p17). Every walker
      passes only the designated item down, so `2` lands in `c`. Locals
      also lose a later `.p = &g`.
    * `{ .x = 1, 2, 3 }` where `x` is in an anonymous struct: the same
      problem.
    * `struct O arr[2] = { [1].in.b = 8, 9, [0].c = 1 }` gives `arr[1] =
      {0,8,9,0}` and `arr[0].c = 1`.
    * Struct-level overrides: `.s.y = 7, .s = { 1 }` must reset `s.y` (a
      braced override zeroes the whole sub-object); `.a[2] = 5, .a = { 2 }`
      likewise, for a local.

    Proper fix: a canonicalizing aggregate-initializer pass, the struct
    counterpart of the array planner (Clang's `InitListChecker` is the
    reference design). It resolves elision, designator chains,
    continuation, anonymous members, union member selection and overrides
    once, against the type, into a fully braced, designator-ordered form.
    Every storage path then consumes that form, and the per-path struct
    walkers (`struct_init.rs`, `stmt.rs::lower_local_struct_init`,
    `global_init_bytes.rs`, `global_init_compound*.rs`) collapse the same
    way the array walkers did in §2.9.
