# Cross-backend SWOT and cross-pollination plan — 2026-11-01

Scope: the four code-generation back ends of `lccc` — x86-64 (`src/backend/x86`),
i686 (`src/backend/i686`), AArch64 (`src/backend/arm`), RISC-V 64 (`src/backend/riscv`).
The question this document answers: *which back end is best at what, what is each
one missing that another one already has, and in which order should the missing
pieces be transplanted?*

Priority is i686 and x86-64 (the user's stated preference), with AArch64/RISC-V
gaps in scope as well.

## 1. Method

Every claim below is either (a) a number produced by the commands listed in this
section, run against this tree, or (b) a probe against an oracle. Nothing is
estimated from memory.

| measurement | command |
|---|---|
| back-end size | `find src/backend/<b> -name '*.rs' \| xargs wc -l \| tail -1` |
| peephole shape | `ls src/backend/x86/codegen/peephole/passes/*.rs \| wc -l`, `wc -l src/backend/<b>/codegen/peephole.rs` |
| pass inventory | `grep -c '^fn \|^pub fn ' src/backend/<b>/codegen/peephole.rs` |
| bisection switches | `grep -rho 'CCC_NO_[A-Z_]*' src/backend/<b> \| sort -u \| wc -l` |
| unit tests | `grep -rc '#\[test\]' src/backend/<b> --include=*.rs` |
| feature probes | `grep -rl '<pass-fn>' src/backend/<b>` for every name in the matrix below |
| `asm` operand | `asm_validate::validate` diagnostics and emitted assembly, checked with `tests/regression/check_asm_operand_diagnostics.sh` and `tests/regression/check_asm_flag_outputs.sh` |
| x86 semantics oracle | `gcc-16` / `gcc-12` (flag-output grammar, `-dM -E` macro set) |
| AArch64/RISC-V semantics oracle | `zig cc -target aarch64-linux-gnu\|riscv64-linux-gnu` (clang) + GNU `as` |

Oracle caveat that shaped §5.1: GCC diagnoses a bad `=@cc` suffix during
*expansion*, not while parsing, so `gcc -fsyntax-only` accepts `"=@cceq"` on
x86-64 while `gcc -c` rejects it with `unknown 'asm' flag output '=@cceq'`.
Clang rejects the whole `@cc` class on AArch64 and RISC-V.

## 2. Inventory

| | x86-64 | i686 | AArch64 | RISC-V 64 |
|---|---|---|---|---|
| source lines | 238,997 | 75,076 | 51,466 | 33,704 |
| files | 100+ | 60 | 40 | 38 |
| peephole structure | modular tree, 47 pass files | one 25,029-line file, 208 top-level fns | one 8,627-line file, 83 fns | one 3,331-line file, 38 fns |
| bisection switches (`CCC_NO_*`) | 37 | 21 | 22 | 9 |
| unit tests in back end | 2,416 | 606 | 246 | 180 |
| register allocator | shared `regalloc.rs` + x86-specific pressure model | shared, ILP32 pool of six | shared, 32-GPR model | shared, 32-GPR model |

The 47-file tree is the single largest structural asset in the compiler. It means
x86-64 passes are individually unit-testable, individually bisectable
(`CCC_NO_<PASS>`), and individually reviewable; the other three back ends carry a
monolith where adding one pass means editing a 8k–25k-line file.

## 3. SWOT

### 3.1 x86-64

**Strengths**
- Deepest peephole pipeline (47 passes): `load_op_fuse`, `lea_add_fuse`,
  `memory_fold`, `store_forwarding`, `flag_peepholes`, `pushf_elim`,
  `fma_forms`, `bmi2_mask`, `load_bswap_movbe`, `vex_promote`,
  `vector_broadcast`, `identical_blocks`, `jump_thread`, `tail_call`.
- Only back end with a complete, defended `=@cc` implementation
  (`store_condition_code_output`, incl. `asm_cc_branch_output_ptrs` branch-only
  elision for `asm goto`).
- Highest unit-test density (2,416) and the only back end whose passes have a
  per-pass kill switch on almost every stage.
- Richest ISA modelling (`arch_table.rs` target/arch tables, VEX/EVEX
  promotion, APX gating, `-march` dependency closure).

**Weaknesses**
- Largest surface by an order of magnitude (239k lines, 100+ files): reviews are
  expensive, and the same defect class can exist in several passes.
- `push_pop.rs::instruction_modifies_reg_id` models `LineKind::Other{dest_reg}`
  with the rule "`dest_reg == 0 && reg_id == 2`", carrying a `TODO: could be
  more precise`; the `mul`/`div` implicit-operand clobber set (`rax`/`rdx`,
  `rdx:rax`) is not modelled at all. Every caller of that predicate inherits the
  over-approximation (caught by CI as "fewer pushes elided than possible", not as
  wrong code).
- Pass ordering is implicit in `peephole/mod.rs`; there is no machine-readable
  manifest, so "which pass runs before `dead_code`" is answered by reading code.

**Opportunities**
- The modular passes are the transplant *source* for the other three back ends:
  `identical_blocks`/`jump_thread` (CFG clean-up), `store_forwarding`,
  `fuse_div_rem_pairs`-style RMW folding, and `flag_peepholes` are all
  target-shaped, not x86-shaped in idea.
- A pass manifest (name, phase, kill switch, invariants) would make the other
  back ends' monoliths expressible as trees too.

**Threats**
- Size makes regressions cheap to introduce and slow to bisect; the kill-switch
  set is the mitigation and it is already 37 entries wide.
- Any refactor touching shared helpers (`x86_common.rs`, `inline_asm.rs`) hits
  both x86 back ends at once — as the `%b`/`%h` register-allocator defect in
  §5.2 shows, which needed a different answer on each of them.

### 3.2 i686

**Strengths**
- Best *CFG-level* peephole set of the four: `thread_trivial_blocks`,
  `merge_duplicate_epilogues_terminal`, `else_hoist_diamonds`,
  `fold_global_fnptr_calls`, `forward_slot_loads_cfg` (a real `SlotState` dataflow
  lattice over the CFG, plus `collapse_slot_rmw_i686`).
- Best *frame/calling-convention* clean-up: `eliminate_dead_frame_allocation`,
  `eliminate_unused_callee_saves`.
- Best *x87* handling: `fold_x87_gp_pair_staging/unstaging` removes the
  GP↔x87 staging that the ILP32 float ABI forces on every `double` round-trip.
- `fuse_div_rem_pairs` and `fuse_setcc_branch` exist only here.
- x86-64-derived inline-asm machinery is complete: `=@cc` with slot and
  through-pointer stores, x87 `st(0)`/`st(1)` outputs, register pairs for
  `long long`.

**Weaknesses**
- One 25,029-line file with 208 top-level functions and only 21 kill switches:
  bisecting a pass means editing the monolith or rebuilding with a patch.
- Only 606 unit tests for 75k lines (vs. 2,416 for 239k on x86-64 — a
  comparable density, but far fewer behavioural tests around the CFG passes).
- Byte-register pressure: the six-register pool (`ecx, edx, esi, edi, eax, ebx`)
  contains three registers (`esi`, `edi`, `ebp`) with **no 8-bit low form** on
  IA-32, while the shared `x86_common::reg_to_8l` only knows the 64-bit
  spellings (`rsi→sil`). Fixed in §5.2 by restricting byte-printed,
  byte-typed and `=@cc` operands to the four byte-addressable registers before
  allocation, but the pressure itself remains: four is the ceiling, so a fifth
  byte operand is unallocatable by construction (as it is in GCC).
- No `-mregparm`/`fastcall` argument-passing clean-up of the kind x86-64 gets for
  free (SysV has enough registers that the problem never appears).

**Opportunities**
- The CFG passes are the best transplant *donors* in the tree: AArch64 and
  RISC-V have no diamond hoisting, no trivial-block threading, no global
  function-pointer folding.
- Splitting the monolith along the x86-64 pattern is mechanical but must be done
  pass-by-pass with the kill switches preserved.

**Threats**
- ILP32 makes every ABI-sensitive change a two-configuration problem (`-m32`
  cdecl vs. the internal register pool), and i686 has the least test coverage per
  ABI corner (`long long`, `double`, 16-byte atomics).

### 3.3 AArch64

**Strengths**
- Best *instruction-selection fusions* for a fixed-width RISC ISA:
  `fuse_and_cmp_zero` (AND→`ANDS`+test), `fuse_fp_adjacent_pairs` (`ldp`/`stp`
  formation), `eliminate_redundant_sxtw`.
- Best *slot-memory* analysis: `forward_fp_slot_loads`,
  `eliminate_repeated_slot_loads`, `propagate_address_aliases`,
  `eliminate_unused_x9_address_moves`, `eliminate_dead_call_staging_moves`.
- Address-alias propagation is unique in the tree — no other back end has it.
- Cleanest inline-asm scratch discipline (`ARM_GP_SCRATCH` ordering, RA-home
  awareness, `x17` reserved for 128-bit staging).

**Weaknesses**
- **Flag outputs were a silent miscompile** until this change: `=@cc<cond>` was
  classified as `ConditionCode`, the store path ignored it, and the operand's
  value was whatever the scratch register happened to hold. There was no
  parse-time check either, so `"=@ccz"` (an x86 spelling) was accepted too.
- No condition-branch fusion: nothing turns `cmp; b.<inv>; b` into `cbz`/
  `cbnz`, and nothing fuses `subs`+test the way `fuse_and_cmp_zero` fuses
  `ands`.
- No diamond/trivial-block CFG clean-up (no `else_hoist_diamonds`, no
  `thread_trivial_blocks`), and no global dead-store elimination over
  address-taken slots (`global_store_forwarding`/`global_dead_store_elimination`
  exist only on RISC-V).
- Monolithic peephole (8,627 lines), 246 unit tests.

**Opportunities**
- i686's CFG passes (`else_hoist_diamonds`, `thread_trivial_blocks`,
  `merge_duplicate_epilogues_terminal`) map onto AArch64 almost unchanged: they
  rewrite block structure, not instructions.
- RISC-V's `global_dead_store_elimination` + `global_store_forwarding` are
  address-taken-slot algorithms with no target-specific assumptions.
- `cbz`/`cbnz` fusion is ARM's analogue of the x86 `fuse_setcc_branch` that
  i686 already has.

**Threats**
- Fixed-width encodings mean every fusion must respect ±1 MiB branch ranges
  (`rewrite_far_jumps_to_near` on RISC-V exists precisely for this); the AArch64
  peephole has no comparable range-checking pass, so a future `cbz` fusion can
  silently turn a reachable branch into an out-of-range one.

### 3.4 RISC-V 64

**Strengths**
- Only back end with **global** (cross-block) store forwarding and dead-store
  elimination over slots: `global_store_forwarding`,
  `global_dead_store_elimination`, `propagate_register_copies`.
- Only back end with branch *relaxation*: `rewrite_far_jumps_to_near` and
  `fuse_branch_over_jump` with explicit ±4 KiB range checks — the single best
  answer in the tree to "the peephole made the code bigger and now a branch does
  not reach".
- Extension-folding passes (`eliminate_redundant_sext_w`,
  `eliminate_redundant_zext_byte`) are themselves documented cross-pollinations
  from AArch64.
- Smallest surface (33,704 lines) and therefore the fastest to review.

**Weaknesses**
- Thinnest peephole overall: 38 top-level functions, 3,331 lines, 9 kill
  switches, 180 unit tests.
- No FP pair formation, no address-alias propagation, no slot-load dedup
  (`ldp`/`stp` and repeated-load elimination exist only on AArch64).
- `=@cc` was mapped to `RvConstraintKind::GpReg` with an in-tree comment
  admitting it "will store incorrect results". RISC-V has no condition register,
  so the correct behaviour is a diagnostic — implemented in this change.
- No CFG clean-up beyond `merge_identical_blocks`/`coalesce_entry_copies`.

**Opportunities**
- Donor→recipient in the *other* direction: AArch64 slot-load dedup and FP-pair
  formation would pay off directly; RISC-V's relaxation framework is the model
  for AArch64 `cbz` fusion.
- `propagate_address_aliases` (AArch64) presupposes exactly the slot dataflow
  RISC-V already has, so it is the cheapest large win available.

**Threats**
- The `zicsr`/`zalrsc`/`zca`-style `.attribute` emission seen in the clang
  oracle output has no counterpart here; any pass that introduces new
  instruction forms must keep the emitted attribute set consistent or the object
  will claim less than it uses.

## 4. Capability matrix (measured)

| capability | x86-64 | i686 | AArch64 | RISC-V |
|---|---|---|---|---|
| modular per-pass files + kill switch per pass | ✓ (47 files) | — | — | — |
| `=@cc<cond>` flag outputs | ✓ | ✓ | ✓ (this change) | refused (this change) |
| `__GCC_ASM_FLAG_OUTPUTS__` | ✓ (this change) | ✓ (this change) | ✓ (this change) | undefined (this change) |
| trivial-block threading | ✓ (`jump_thread`) | ✓ | — | — |
| diamond (if/else) hoisting | — | ✓ | — | — |
| duplicate-epilogue merging | ✓ (`epilogue_merge`) | ✓ | — | — |
| dead frame allocation removal | — | ✓ | — | — |
| unused callee-save removal | ✓ | ✓ | — | — |
| slot-load deduplication | ✓ (`memory_fold`) | ✓ (`forward_slot_loads_cfg`) | ✓ | — |
| FP pair (`ldp`/`stp`) formation | — (x86 has no pairs) | — | ✓ | — |
| address-alias propagation | — | — | ✓ | — |
| global (cross-block) store forwarding | ✓ | — | — | ✓ |
| global dead-store elimination | ✓ | — | — | ✓ |
| branch-range relaxation | n/a (rel32) | n/a | — | ✓ |
| `ands`+test fusion | — | — | ✓ | — |
| `setcc`+branch fusion | ✓ (`compare_branch`) | ✓ | — | — |
| x87 GP-pair staging fold | — | ✓ | n/a | n/a |

Legend: ✓ present, — absent, n/a not applicable to the ISA.

Correction to the user's premise worth recording: the pass names that *look*
i686-exclusive (`else_hoist_diamonds`, `collapse_slot_rmw`, `fold_global_fnptr_calls`,
`forward_slot_loads_cfg`) are i686-exclusive in *name* only for the CFG ones —
x86-64 covers the same ground with `jump_thread` + `epilogue_merge` +
`store_forwarding`. The genuinely i686-exclusive capabilities are diamond
hoisting, dead-frame-allocation removal and x87 GP-pair staging.

## 5. Work delivered in this change

### 5.1 AArch64 flag outputs implemented; RISC-V flag outputs refused (defect)

*Donor:* x86-64/i686 (`store_condition_code_output`). *Recipient:* AArch64.

- `src/common/asm_constraints.rs` — new `AARCH64_FLAG_CONDITIONS` table (the 16
  ARM `ConditionHolds` conditions GCC accepts after `@cc`) and
  `aarch64_flag_condition`, documented against the GCC manual section that lists
  flag outputs as x86/ARM/s390-only.
- `src/frontend/parser/asm_validate.rs` — `is_x86: bool` replaced by
  `AsmTarget { X86, Aarch64, Other }` with `from_elf_machine` (EM_386/EM_X86_64 →
  X86, EM_AARCH64 → Aarch64, everything else → Other). `check_constraint` now
  validates the suffix against the *target's* table and refuses `@cc` outright on
  targets with no flag outputs.
- `src/backend/arm/codegen/asm_emitter.rs` — `store_output_from_reg` materialises
  a `ConditionCode` operand with `cset w<n>, <cond>` before the ordinary store;
  the stale TODO claiming "stores incorrect results" is gone.
- `src/backend/arm/codegen/inline_asm.rs` — `to_w_reg` (x-form → w-form, needed
  because `CSET` has no 64-bit form and `w`-writes zero-extend).
- `src/backend/riscv/codegen/asm_emitter.rs` — comments corrected to state why
  the arm is unreachable.
- `src/frontend/preprocessor/predefined_macros.rs` — `__GCC_ASM_FLAG_OUTPUTS__`
  is now defined on x86-64/i686/AArch64 and explicitly undefined on RISC-V. This
  is what makes the Linux kernel's `CC_SET`/`CC_OUT` take the flag-output path on
  the three targets that implement it.

Why refusal (not implementation) is right for RISC-V: RISC-V has no condition
register, GCC's manual lists only x86, ARM/AArch64 and s390 as flag-output
targets, and both `gcc` (x86 precedent: `unknown 'asm' flag output`) and clang
(`invalid output constraint '=@ccz'`) refuse the operand. Implementing it would
mean inventing a condition the ISA does not have.

### 5.2 Fixed: `%b`/`%h` operands could be given a register that cannot print them

**The defect.** An `asm` template names operands through modifiers, and two of
them ask for a *narrower view* of the register than the operand's natural width.
`%b<n>` asks for the low byte; on x86-64 every general register has one (`sil`,
`dil`, `bpl`, `spl` and `r8b`-`r15b` are REX encodings), but on IA-32 only
`al`, `bl`, `cl` and `dl` are byte-addressable -- `esi`, `edi` and `ebp` have no
8-bit form under *any* name. `%h<n>` asks for the high byte, which exists only
for `rax`/`rbx`/`rcx`/`rdx` (`ah`, `bh`, `ch`, `dh`) on both x86 targets;
`x86_common::reg_to_8h` answers every other register with its *low* byte name.

Neither fact was visible to the allocator. `assign_scratch_reg` walks the
general pool and hands out the next free register, so on i686 the third
byte-printed operand got `esi`, `reg_to_8l` passed the name through unchanged
(correctly -- there is no mapping to invent), the template printed `%esi`, and
the assembler rejected `movb %esi ...`. On x86-64 a `%h` operand could be given
`rsi`, and `reg_to_8h` printed `%sil`: the **low** byte where the template asked
for the high one, i.e. silent wrong code, not a build error.

The constraint string cannot express either requirement, so the template is the
only place it is written down.

**The fix.** `inline_asm::operand_reg_views` parses the dialect-expanded
template once, before any register is assigned, and reports per operand whether
it is printed through the low byte, the high byte, or its natural width. It
handles `%%` literals, `[name]` operands, multi-digit indices and saturating
numeric parse; a reference it cannot attribute leaves the operand at its natural
width, i.e. exactly the previous behaviour; `%h` dominates `%b` for the same
operand because it is the stricter of the two.

The `InlineAsmEmitter` trait grew two methods with deliberately conservative
defaults, so AArch64 and RISC-V -- where `%w0`/`%x0` are width names for one
uniform register file -- are byte-for-byte unaffected:

| hook | i686 | x86-64 |
|---|---|---|
| `assign_scratch_reg_for_view` | byte-printed, `I8`/`U8`-typed and `=@cc` operands are allocated from `ecx, edx, eax, ebx` only, with the general pool's two-pass "avoid RA homes, then take them" policy and its `-fPIC` reservation of `ebx` | forwards to `assign_scratch_reg`; the high-byte view is routed to the existing `QReg` class, which *is* the four legacy registers |
| `home_reg_usable_for_view` | a home is reusable only if it is one of those four | a home is reusable unless the view is the high byte and the home is not `rax`/`rbx`/`rcx`/`rdx` |

`assign_one_scratch` takes the view list as one more argument and
`assign_scratch_registers` threads it through both allocation passes, so the
"the input is already fresh in a register, use it in place" shortcut is gated
too: a home without the required view is copied to a scratch register that has
it, instead of being printed through a name that does not exist.

**Why allocation-side rather than store-side staging.** The alternative -- keep
handing out `esi` and stage the narrow access through a `pushl %eax` /
`movl %esi, %eax` / `movb %al, ...` / `popl %eax` sequence -- is always correct
(no `push`/`pop`/`mov` writes flags), but it costs three extra instructions on
the hot path of every byte operand and leaves the template's own `%b<n>` text
unfixable: the substitution layer has no instruction stream to inject a move
into. Allocation-side is also what GCC's i386 port does, and it is one decision
in one place instead of a rule every store path must remember.

**Known limit, unchanged.** A bare `r`/`q` operand whose template is a *single*
character modifier (`%b`, `%h`) with an implicit `0` is not valid GCC asm either;
the allocator cannot restrict a register for a modifier it cannot see. For
i686 `%h` specifically, any of the four byte registers prints a real high byte,
and `ah`-`dh` are all legal operands, so the emission is valid whenever a byte
register was allocated.

**Evidence.** Unit tests cover the parser (modifiers, `%%`, named and multi-digit
operands, strictest-use, totality on malformed and out-of-range input) and both
overrides (i686 never leaves the four byte registers for a byte view, a byte
type or a `=@cc` output, still reaches `esi`/`edi`/`eax`/`ebx` for full-width
operands, and only accepts byte-addressable homes; x86-64 restricts `%h` to the
legacy four and leaves `%b` unrestricted).
`tests/regression/check_asm_flag_outputs.sh` adds two assembly-level assertions:
`%h2` on x86-64 must print a real high byte, and the third `%b` operand on i686
must be a byte register with no `%esi`/`%edi`/`%ebp` in any `movb` of that
function. The GCC oracle used to derive the expected behaviour was
`gcc -m32 -O2 -S` on four `char` outputs with `"=q"`/`%b0..%b3`, which prints
`movb %dl,%dl`, `movb %cl,%cl`, `movb %al,%al`, `movb %bl,%bl` -- the four
legacy registers in that order, never `esi`/`edi`.

### 5.3 Ported: compare-and-branch fusion to AArch64, dead fall-through branch removal everywhere

**Donor.** x86-64's `emit_branch_nonzero` is `testq %rax, %rax` + `jne label`: the
conditional move and the branch are one decision, and the branch goes straight
to its target. RISC-V's peephole already fused the three-instruction spelling
(`beqz t0, .Lskip; jump .Ltarget, t6; .Lskip:` → `bnez t0, .Ltarget`) with a
range check. AArch64 had neither: its `emit_branch_nonzero` emitted
`cbz x0, .Lskip; b label; .Lskip:` and its `fuse_branch_over_branch` only
recognised `b.cc` heads, so every boolean branch kept an extra instruction and a
second taken branch.

**Codegen.** `ArmCodegen::emit_branch_nonzero` now emits a single
`cbnz x0, label`. `cbz`/`cbnz` reach ±1 MB, the same window as `b.cond`, so the
direct form is what the assembler's relaxation would have produced anyway. Four
sites used to pay the three-instruction price on every boolean branch:
`emit_select`, `emit_cond_branch_blocks`, `emit_fused_cmp_branch` and the
conditional-increment paths. RISC-V keeps its
three-instruction spelling on purpose: `jump` is a far pseudo-op (`auipc` +
`jr`), a raw `bnez` only reaches ±4 KiB, and the range-gated peephole is the
only place in that back end that can *prove* the target is near.

**Peephole.** `fuse_branch_over_branch` was restructured around three small
predicates — `is_range_limited_branch`, `range_limited_branch_target`,
`retarget_branch_over_branch` — plus a `fused_branch_head` recogniser, so the
head kinds are data rather than nested `if let` towers. It now folds:

| head | fold |
|---|---|
| `b.cc .Lskip` | `b.!cc .Ltarget` |
| `cbz xM, .Lskip` | `cbnz xM, .Ltarget` |
| `cbnz xM, .Lskip` | `cbz xM, .Ltarget` |
| `tbz xM, #n, .Lskip` | `tbnz xM, #n, .Ltarget` |
| `tbnz xM, #n, .Lskip` | `tbz xM, #n, .Ltarget` |

The range guard is the pre-existing instruction census against
`COND_BRANCH_SAFE_DISTANCE` (200,000 instructions), which is conservative for the
±1 MB window all four heads share. One asymmetry is deliberate and documented in
the code: the `b.cc` head keeps the historical behaviour of trusting an
*unmeasurable* target (the unconditional branch already reached it), while the
compare-and-branch heads fail closed unless the census proves the distance — a
`cbz` reaches 128x less far than the `b` it replaces, so "the `b` reached it" is
not evidence.

**Both RISC targets: dead fall-through branches.** `eliminate_redundant_branches`
(AArch64) and `eliminate_redundant_jumps` (RISC-V) removed a branch only when it
was unconditional *and* its target was the immediately following label. A
conditional branch in the same position is equally dead: taken or not, control
reaches the same instruction, and `cbz`/`tbz`/`bnez` only read their operands, so
dropping the line cannot lose a write. Both passes now share the same
`next_insn`-style search and cover `Branch`, `CondBranch` and `CmpBranch` (ARM)
resp. `Jump` and `Branch` (RISC-V). This is the shape `emit_cond_branch_blocks`
produces whenever the true block is laid out first.

**Evidence.** `src/backend/arm/codegen/peephole.rs` gains four fusion tests
(compare-and-branch head, bit-test head, unmeasurable target fails closed,
`b.cc` regression guard) and one fall-through test; the RISC-V peephole gains a
direct `eliminate_redundant_jumps` test that also pins the "different target is
live" case. None of these can run in the agent sandbox (no Rust toolchain), so
they are compiled and executed by the `Unit tests` and `Clippy` CI jobs.

### 5.4 Ported: trampoline threading to both RISC targets

**Donor.** i686's `thread_trivial_blocks` finds local labels whose whole body is
an unconditional jump, resolves the chains once with a cycle check, retargets
every reference, and then deletes the block once a module-wide reference census
proves nothing else points at it. Neither RISC target had any of it, so every
trampoline left behind by block shuffling, select lowering or identical-block
merging cost an extra fetch and an extra taken branch on every execution.

**What was ported.** `thread_trampolines` on AArch64 and RISC-V builds
`label -> forward target` for every local label whose body is a single
unconditional branch, resolves chains with `resolve_trampoline` (bounded at 16
hops, cycles refused rather than broken), and rewrites the branch targets in one
pass. The analysis borrows the text and produces a list of rewrites; only the
rewrites mutate it, so no `&str` key outlives the analysis and the pass allocates
nothing per label.

**What was deliberately *not* ported: block deletion.** The i686 pass removes the
trampoline label and jump once its census proves the block unreachable from
elsewhere. That requires two proofs this back end does not have in a text
peephole: the block cannot be entered by *fall-through*, and no label address is
observable to code (`adrp`/`adr` on AArch64, `lla` on RISC-V, plus the
`.L`-symbol tricks the assembler tests use). Retargeting alone is
control-flow-equivalent with no such proof: only branch *targets* change, every
rewritten branch keeps its mnemonic, and the trampoline text stays exactly where
it was. The now-unreferenced copies are removed by the ordinary dead-code passes
of later rounds instead.

**Range discipline.** AArch64 retargets `b` to `b`, which keeps the ±128 MB
reach. RISC-V retargets the far `jump` pseudo freely (it reaches the whole text
via `auipc` + `addi` + `jalr`), but the near `j` form (`jal x0`, ±1 MiB) is only
retargeted when `interval_is_near` proves the new target reachable inside the
same conservative ±3 KB, 32-bytes-per-line census that gates
`fuse_branch_over_jump` — that helper was factored out of the fusion pass so both
users share one definition of "near" instead of two drifting copies. The census
still fails closed across directives, global labels and the opaque
volatile-cut region.

**Interaction with the pass added in §5.3.** A trampoline whose body jumps to the
immediately following label is already flattened by the dead-branch pass (the
branch was dead), so it never reaches the threading analysis; the threading pass
therefore handles exactly the non-adjacent case that remains, and the two passes
converge in one round rather than fighting.

**Evidence.** `trampolines_are_threaded_to_the_end_of_the_chain` and
`trampoline_cycles_are_left_alone` on AArch64 (full pipeline), and one RISC-V
test that drives `thread_trampolines` directly for both the two-hop chain and the
cyclic chain, asserting the text is left byte-identical in the cyclic case.

### 5.5 x86-64 ↔ i686: exact register-clobber oracle, and the i686 scratch-save pass resurrected

This round is x86-only: the two x86 back ends share one ISA and one machine
model, so anything one of them knows about an instruction's implicit operands is
true for the other. They had drifted.

**1. `instruction_modifies_reg_id` is now exact (backlog item 8, x86-64).**
The predicate deciding whether a `push %R` may be dropped answered
`dest_reg == 0 && reg_id == 2` when the classified destination was `%rax`,
carrying a `TODO: could be more precise`. The guess is wrong in *both*
directions:

| direction | consequence | example |
|---|---|---|
| too coarse | lost eliminations | `pushq %rdx … <`%rax`-only span> … popq %rdx` was refused, because "a `%rax` writer might write `%rdx`" |
| too narrow | unsound-looking holes | `cpuid`, `rdtsc`, `rep stosq`, `xchg`, `lock xadd` classify with `dest_reg == REG_NONE`, so their implicit writes were invisible |

The predicate now asks the ISA table that the classifier already owns:
`implicit_write_refs` (the exact *write* half of `classify_implicit_operands`,
which both x86 back ends maintain against the architecture) plus the classified
destination plus the two-destination forms that AT&T's "last operand is the
destination" rule cannot express (`xchg`, `xadd`, `cmpxchg`). The two-destination
logic and the prefix-stripping mnemonic helper moved into `types.rs`
(`line_writes_gp_family`, `names_a_second_destination`, `unadorned_mnemonic`) so
the push/pop pass and the fold passes cannot disagree about them again.

**2. The same table gained `pcmpistri`/`pcmpestri` on both back ends.**
SSE4.2's string compares return their index in `%ecx` and the explicit-length
form takes both lengths from `%eax`/`%edx`; neither oracle modelled it, so a
`%rcx` live across `pcmpistri` looked dead. Not emitted by our codegen today,
which is exactly why it is worth writing down now: the table is the fail-closed
substrate every windowed pass reasons with.

**3. Fold precision (x86-64).** The two `cltq`-past-a-neutral-instruction scans
in `local_patterns` used `has_implicit_reg_usage` — a coarse "may touch some
register" veto — to answer "does this line write `%rax`". It said yes for `cqto`
(writes `%rdx`), `mulx` (reads `%rdx`), `shld` (reads `%cl`) and `rep` (writes
`%rcx`), refusing folds that are provably safe. Both sites now ask
`line_writes_gp_family(text, 0)`.

**4. i686: scratch-save elimination (`eliminate_push_pop_pairs`) is live again.**
The pass existed, was found unsafe ("callee-save/leal epilogue interactions"),
and was left wired to nothing while the emitter kept saving `%ecx`/`%edx`
defensively — the 32-bit SWAR popcount alone pays two stack round-trips per call.
Rather than delete it, the invariant it was missing is now enforced explicitly in
`scan_push_pop_span`:

* the span must contain **no label, branch, call, `ret`, indirect jump or inline
  asm**. The call case is the one a register-based safety test cannot see: under
  cdecl every stack argument sits four bytes higher with the save in place, so
  deleting the pair moves the callee's whole argument list;
* no `%esp`/`%sp` mention and no architectural stack-pointer write (`leave`,
  `enter`) — this is the epilogue interaction that made the original pass
  unsafe: a `leal -8(%ebp), %esp` inside the span would be re-pointed;
* no `pushf`/`popf` (they own the very slot the save created) and no non-inert
  or CFI directive (`.cfi_adjust_cfa_offset` describes the push being removed);
* nested `push`/`pop` pairs must balance inside the span.

On top of that proof, two removal rules, both ported from the x86-64 pass:

1. **untouched span** — the register already holds its incoming value on both
   sides, so save and restore are both dead weight (safe for callee-saved
   registers too, and for `%ebp`, since nothing about the value changes);
2. **dead restore value** — the span writes the register but nothing reads it
   after the `pop`. Callee-saved registers are excluded (the caller owns their
   value at `ret`, which intra-function liveness cannot see), and a
   loop-carried save whose only reader is the next iteration's own `push` is
   decided by re-solving the enclosing function with that single read
   neutralised (`GprLiveness::live_after_dropping_region`, the i686 counterpart
   of the x86-64 `FileLiveness::live_after_dropping_region`), bounded to 64
   re-solves per invocation.

Writing detection is now exact rather than the old union test: the previous
implementation asked `line_references_reg` (reads ∪ writes), so a span that
merely *read* the saved register — `pushl %ecx; imull %ecx, %eax; popl %ecx` —
kept the pair for no reason.

The pass is pipelined in phase 4 (after `optimize_cfg_register_liveness`
establishes the CFG solution and before `eliminate_unused_callee_saves`), with a
bounded fixpoint against the dead-move/dead-store sweeps so the staging lines
that only fed a removed pair are reclaimed in the same run. Kill switch:
`CCC_NO_I686_PUSH_POP`.

**5. The classified destination now yields to the implicit *read* set.** The
destination parser names `%rax` as the primary destination of
`cqto`/`cdq`/`cqo`/`cltd`/`cwtd`/`cwd`, because `parse_dest_reg_fast` returns one
family and those instructions are "the `%rax` family". They do not write `%rax`
at all: `%rax` is their implicit *input* and `%rdx` is their only output, which
the write set reports exactly. Both oracles therefore suppress a classified
destination when the same line implicitly reads that family
(`line_writes_gp_family` on x86-64, `line_writes_family` on i686) — the rule
`passes::helpers::writes_family` has always applied, now shared with the push/pop
paths, and cross-checked by `writes_family_tests::agrees_with_the_shared_oracle`
so the two can never drift apart again (the shared oracle is a strict superset:
it adds the two-destination forms and the barrier kinds). This is what makes
`pushl %eax; cltd; popl %eax` provably redundant rather than merely "kept".

**6. i686: the implicit-operand oracle got the 32-bit-relevant ISA rows it was
missing and the x86-64 fast path.** `pushf`/`popf`/`iret` (they read and write
`%esp`), `int1`/`int3`/`into`, `xbegin`, `monitorx`/`mwaitx`/`skinit`/`clzero`,
`rdpkru`/`wrpkru`/`rdpru`/`wrmsrns`, `pconfig` — plus a leading fast-path match
for the highest-frequency mnemonics whose table entry is `(0, 0)`. The fast path
matters because `line_reg_use_def` asks the oracle once per register family,
i.e. eight times per line, and every windowed pass asks again for every line it
walks; the x86-64 oracle has had this fast path since it was written.

**Evidence.** `push_pop.rs` gains a `register_clobber_tests` module that pins
every row above — the `%rdx`-across-`%rax`-only-span case that used to be
refused, `cqto` vs `cltq` asymmetry, `cpuid`'s four families, `rep stosq`,
one-operand vs two-operand `imulq`, `mulx` reading rather than writing `%rdx`,
`pcmpistri` writing `%rcx`, `xchg`/`xadd`/`lock xadd`/`cmpxchg` two-destination
behaviour, and the call clobber set. The i686 peephole gains eleven behavioural
tests: untouched-span removal, read-only-span removal, the read-only `cltd`
case, dead restore value, live restore kept, callee-saved kept, call-in-span
kept, five stack-pointer dependence shapes refused, and the loop-carried pair
both ways. The x86-64 pair scan gains its own module (`pair_scan_tests`), since
the holes above were all in code no test covered: an untouched span, a label
inside the span, a conditional branch inside it, four stack-pointer shapes
(`movq %rax, %rsp`, a `%rsp`-relative load, `subq` to `%rsp`, `pushfq`/`popfq`)
and a span long enough to need the widened window.

**7. The x86-64 pair scan had three soundness holes, found by red-teaming the
port.** Porting `scan_push_pop_span` to i686 forced an enumeration of everything
the x86-64 pass had to prove, and the enumeration did not match the code: it
hand-listed `Call | Jmp | JmpIndirect | Ret` as barriers and therefore *stepped
over* a `Label` (a second entry point into the span, so the `pop` is reachable
without the `push` — deleting the pair pops a return address on that path), a
`CondJmp` (the pair is unbalanced on the taken side) and any opaque inline-asm
block. `instruction_modifies_stack` had the mirror problem: it listed
`subq`/`addq`-to-`%rsp` and `%rsp`-relative memory operands, so `movq %rax, %rsp`
and `leaq -8(%rbp), %rsp` — the epilogue restores — were not stack dependencies
at all. Both are now one predicate each: `LineInfo::is_barrier` (the crate's
single answer to "may a window step over this line", which already includes
labels, all four branch kinds, `Ret`, `Push`/`Pop`, inline asm and non-inert
directives) and "the text mentions the stack pointer in any spelling". The window
also grows from three lines to eight now that its proof obligations are stated
once.

**8a. Both clobber predicates asked the oracle twice per line.** `line_writes_gp_family`
and `helpers::writes_family` each needed the implicit *read* half and the
implicit *write* half, and each fetched them with a separate
`classify_implicit_operands` call -- a prefix walk plus a table match, on the
hottest predicate in the file (once per line of every push/pop span, and once per
copy/graph edge of every pass). One call yields both halves, and the resulting
answer is identical by construction because the second call was of the same pure
function with the same input. `classify_implicit_operands` is now `pub(super)` so
the passes module can share one classification instead of projecting it twice.

**8. The i686 CFG fixpoint was doing O(rounds x lines) oracle work.** Each round
called `line_reg_use_def` for every line, and that function asked
`line_references_reg` once per register family — nine `classify_implicit_operands`
calls per line per round, plus a `direct_jump_target` parse of every line. The
per-line facts (use/def, neutralised mask, return-use markers, resolved branch
target) do not depend on the iteration, so they are computed once per function
and the rounds only move bits. The union form is also exact rather than
approximate: `line_references_reg(line, r)` *is* "the text names `r`, or the
oracle mentions it", so one oracle call plus eight `contains` tests reproduces it
by construction.

### 5.6 x86-64 ← i686: the in-place byte mask, and two span holes the port exposed

The i686 back end folds `andl $imm, %R` whose only consumer is a condition into
a `testb`; §6 listed that as an i686-exclusive capability. It is now on x86-64
too, but the interesting part of the round is what porting it found: the same
enumerations that had made the i686 pass sound were missing from two x86-64
scans, in the shape of a directive that lies to the unwinder and a trap that is
not a branch.

**1. `passes/mask_test.rs` (new).** Two rewrites of one shape, `and{l,q} $imm,
%R` with `1 <= imm <= 255`:

```text
    andl $255, %r10          testb %r10b, %r10b
    testq %r10, %r10   ->    je .L1                 (7+3 bytes -> 3)
    je .L1

    andl $255, %r10    ->    movzbl %r10b, %r10d    (7 bytes -> 4, flags dead)
```

The proof is per flag, and it is the reason the rule cannot be stated as "the
mask feeds a branch". `and` and `test` both clear CF and OF and both set ZF and
PF from the AND of their operands, and because `imm <= 255` the mask's own
result has no bit above bit 7, so a byte-wide test sees exactly the same
zero-ness. SF is the one flag they disagree on — `andl $255`'s result is at most
255, so its SF is always 0, while `testb`'s SF is bit 7 of the masked byte — so
the rewrite requires *both* that the reader's condition does not consult SF
(`js`/`jns`/`jl`/`jge`/… and their `set` forms are refused) and that every flag
is dead after it. Without the second half, `andl $255,%R; je; js` would answer
"not negative" for 0x80..0xFF.

The earlier `flag_peepholes::fold_copy_and_mask_into_test` does not reach this
shape and does not want to: its consumer must be the very next line, and a
`test` *writes* flags, so the `and; test %R,%R; jcc` sequence that
`(uint8_t)x != 0` lowers to stops its scan (the consumer it finds writes the
flags it needs read). Nor did anything own the single-line encoding rule for the
`%D == %A` case, which is where the seven-byte form survives: the existing
`fold_copy_and_mask_into_movz` needs a preceding copy to fire.

Two obligations come from *deleting* the mask rather than from the flags: the
register's value must be dead after the reader, and nothing between the two lines
may touch it — adjacency is enforced by `next_non_debug`, which steps over
deleted lines and debug attribution but not over `.cfi_*`, alignment or an opaque
directive, since a `.byte 0x9e` in the window is `sahf` and would eat the very
flags the proof is about. Rule 2 freezes out any mask but `0xFF` (`movzx`
reproduces exactly a full-byte mask) and needs the flags dead, because `movzx`
writes none. `CCC_PEEPHOLE_SKIP=mask_test` disables both; the pass is idempotent,
so it is placed after the phase-1 fixed point rather than inside it.

**2. Neither mask rule survives on a `.loc`.** The rewrite of one line and the
deletion of another changes what dataflow can conclude about the register
afterwards, and a solution computed before the rewrite under-reports liveness
(deleting a definition lets an older value flow further, i.e. *grows* the live
set — the claim in `eliminate_dead_scratch_saves` that "removals only shrink live
sets" was false and is now retracted in-code). Both passes keep a per-register
dirty bit and re-solve the enclosing function on the next query about that
register, which is what the i686 `GprLiveness` discipline already required.

**3. The x86-64 pair scan spanned a `leave`.** `instruction_modifies_stack`
matched `%rsp`/`%esp`/`%sp` in the *text*, and `leave`, `enter`, `pushf`, `popf`
and `iret` name no register. Porting the i686 span proof — which always asked
the oracle for `%esp` — exposed it: `pushq %rbx; leave; popq %rbx` moved the
frame-teardown's restore by a slot. The predicate is now two exact halves,
`line_depends_on_stack_pointer` = textual mention **or**
`implicit_reg_refs(...) & (1 << REG_RSP)`, so the operand-less family is covered
by the same ISA table the clobber oracle uses.

**4. A `.loc` between a `pop` and its `.cfi_adjust_cfa_offset` hid the
adjustment.** Both back ends look past the `pop` for the adjustment that
describes it, and both stopped at the first non-deleted line — which a `-g` build
makes a `.loc`. The pair was then deleted with the unwind information still
claiming the slot, i.e. a debugging build could hand the unwinder a CFA pointing
at live data. Both scans now step over directives that carry no unwind state
(`LineInfo::is_unwind_inert_directive` on x86-64, the same predicate inline in the
i686 monolith) and stop at any `.cfi_*`.

**4a. The untouched-span rule had no `pinned` guard and no CFI guard.** The
scratch-save rule inspects the span and removes the pair when the register is
dead; the older untouched-span rule (`push R … pop R` with nothing writing `R`)
removed it unconditionally. Two lines the pass never consulted say otherwise:
a pinned line is one the program's own contract owns — a C `volatile` access
(`pin_volatile_access_regions` pins in place and keeps the kind), a parameter
pre-store, an inline-asm region — and a `.cfi_` adjustment *after* the `pop`
describes the slot the pair owns, so removing the pair leaves the unwinder
pointing at a byte nobody moved. x86-64 now checks `pinned` on both ends of the
pair and asks `cfi_follows` before removing, which is what the scratch-save
rule already did. The i686 back end needs no `pinned` field for this: it pins a
volatile or inline-asm region by reclassifying every line in it as `InlineAsm`,
so the pair is never classified as one.

**5. Neither scan stopped at a trap or an unnamed control transfer.** `hlt`,
`ud2`, `int3`, `sysret`, `sysexit` and friends own no `LineKind`, so `is_barrier`
answers "may step over" and nothing else vetoed them. The pair is unsound there
for the reason the whole proof exists: `%rsp` at the instant of the trap or of
the system return is observed *outside* the function — by a signal handler, a
debugger reading the frame, or user mode — so removing a pair around it changes
the value that observer sees even though no line in the span names the stack
pointer. `passes::push_pop::leaves_linear_flow` and the i686 `ends_linear_flow`
now end the span. `iret`/`popf` needed no entry: the implicit oracle already
reports their `%rsp`.

**Evidence.** `mask_test.rs` carries 16 tests — the fold with and without the
redundant zero test, the `cmp $0` variant, a `setcc` reader, three SF readers
refused, a later SF reader refused, a live value refused, wide masks
(`256`/`65535`/`-1`) refused, a memory destination refused, an intervening store
refused, both encoding cases, a narrow mask refused by rule 2, live flags
refusing it, idempotence, and every GP family reachable. `push_pop.rs` gains the
in-span `leave`/`enter`/`iretq`/`sysretq` cases, the `.loc`-before-CFI case, the
pinned-pair cases and the CFI-after-`pop` cases;
the i686 module gains the `ud2`/`hlt`/`int3` span cases and the same `.loc` case.
`tests/regression/check_mask_test_fold.sh` is the gate: it asserts the emission
law (a mask whose only consumer is a condition must not survive), it stays quiet
if the emitter already folds the consumer, and it runs the discriminating probes
— `(signed char)x < 0` and `(unsigned char)x > 127` must stay sign tests — under
GCC's oracle at `-O0/-O1/-O2/-O3/-Os` in both 32- and 64-bit mode.

## 6. Ranked cross-pollination backlog

Ordered by (correctness impact × reach) / risk. Every item names donor, recipient
and the mechanism.

| # | item | donor → recipient | impact | risk | notes |
|---|---|---|---|---|---|
| 1 | ~~byte-register-safe asm operand allocation~~ (§5.2) | x86-64 semantics → i686 | **done** — hard error on `%b`/`%h`, wrong-byte `%h` on x86-64 | medium | landed as `operand_reg_views` + two `InlineAsmEmitter` hooks |
| 2 | `else_hoist_diamonds` | i686 → AArch64, RISC-V | shrinks every if/else region | medium | pure CFG rewrite; needs a scheduler that can undo fall-through |
| 3 | `thread_trivial_blocks` ~~(§5.4: retargeting half)~~ / `merge_duplicate_epilogues_terminal` | i686 → AArch64, RISC-V | **partly done** — `b`→`b` trampoline hops removed; block deletion and duplicate epilogues remain | low | retargeting needs no fall-through or label-identity proof; deletion does |
| 4 | `eliminate_unused_callee_saves` / `eliminate_dead_frame_allocation` | i686 → AArch64 | shrinks prologue/epilogue of every leaf-ish function | low | AArch64 prologue is simpler than i686's; slot model already shared |
| 5 | `propagate_address_aliases` + `eliminate_repeated_slot_loads` | AArch64 → RISC-V | repeated slot loads in loops | low | RISC-V has the slot dataflow this pass needs |
| 6 | `global_store_forwarding` + `global_dead_store_elimination` | RISC-V → AArch64 | address-taken slot traffic | medium | needs the cross-block value numbering RISC-V already built |
| 7 | ~~`cbz`/`cbnz` fusion with range checking~~ (§5.3) | i686 `fuse_setcc_branch` + RISC-V `rewrite_far_jumps_to_near` → AArch64 | **done** — one instruction and one taken branch fewer per boolean branch | medium | `cbz`/`b.cond` reach ±1 MB, not ±32 KiB; the census guard was already there |
| 8 | ~~`instruction_modifies_reg_id` precision~~ (§5.5) | new | **done** — exact implicit-write oracle + two-destination forms; `TODO` deleted | medium | x86-only; the same oracle also re-enabled the i686 scratch-save pass |
| 9 | pass manifest for the x86-64 tree | x86-64 → tree-wide | reviewability, generator-friendly | low | precondition for #10 |
| 10 | split the i686/ARM/RISC-V monoliths along the x86-64 pass-file pattern | x86-64 → i686, ARM, RISC-V | bisectability, per-pass tests | high (mechanical churn) | do pass-by-pass, never as one commit |
| 11 | `narrow_dead_and_to_testb` | i686 → x86-64 | **done** (§5.6) — `mask_test`; mask+zero-test and the `movzx` encoding are one instruction shorter per site | medium | the flag window is restated against `flags_dead_after` instead of a hand-written reader class |
| 12 | span-ending predicates (`leaves_linear_flow`, unwind-inert directives) | i686 ↔ x86-64 | **done** (§5.6) — both back ends now refuse a trap, an unnamed transfer or a `.cfi_*` between a `save` and its `restore` | low | the same two holes existed independently in each back end |

## 7. Threats to the plan

- **No local Rust compiler in the agent sandbox.** Rust changes can only be
  compiled by CI, so every iteration costs a full CI round trip. Mitigation: keep
  each commit single-theme, and gate formatting by hand against
  `cargo fmt --check` (max width 100, `fn_call_width` 60) before pushing.
- **Monoliths make review expensive.** A pass added to a 25k-line file is
  reviewed by diff only; the pass-file pattern from x86-64 is the structural fix
  (#10) but is high-churn and must not ride along with semantic changes.
- **Oracle gaps.** No AArch64/RISC-V GCC is reachable from the sandbox, so ARM
  and RISC-V grammar decisions rest on the GCC manual, clang's behaviour and
  in-tree measurements — recorded per item so a later round can re-measure.
- **Flag-output semantics are only observable at runtime.** The new
  `check_asm_flag_outputs.sh` runs the x86-64 case and greps `cset` for AArch64;
  an AArch64 runtime check needs an emulator (`qemu-aarch64`), which is not
  available here — the gate is therefore a *shape* check on that target.
