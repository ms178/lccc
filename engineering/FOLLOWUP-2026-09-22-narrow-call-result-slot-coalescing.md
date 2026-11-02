# Follow-up: a narrow call result must not share a stack slot with a wide phi

Date: 2026-09-22 (session 59)
Base: `1ff53e8` (`ms178/lccc` main, level with `origin/main`: 0 behind / 0 ahead).

A real miscompile, found by differential-testing an ACPICA function extracted
from Linux 6.18, root-caused to the CFG copy coalescer, and fixed with a
two-line soundness gate that changes codegen in exactly one place across the
whole regression corpus.

---

## 1. Symptom

`tests/regression/acpica_name_string_outparam.c` is a self-contained
extraction of `acpi_ex_get_name_string()` / `acpi_ex_name_segment()` /
`acpi_ex_allocate_name_string()` from `drivers/acpi/acpica/exnames.c` v6.18,
plus verbatim upstream stubs (`acpi_ut_valid_name_char` is copied from
`utascii.c:60`, not approximated), a bump allocator, address-free trace
counters, and 16 AML name cases. It is compared byte-for-byte against a GCC
oracle by `scripts/run_regression_suite.sh`.

With lccc at `-O1`/`-O2`/`-O3` the three **field-type** cases
(`ACPI_TYPE_LOCAL_REGION_FIELD` / `_BANK_FIELD` / `_INDEX_FIELD`) failed:

```
field-region           FAIL: length 2, want 4
```

`out_name_string` was correct (`FLD0`); `out_name_length` was wrong. `-O0` and
`-Os` were clean. The same binary was **non-deterministic across processes**
(7/8 runs failing at `-O1`, 6/8 at `-O2`) yet **fully deterministic under
`setarch -R`** — the signature of a read of uninitialized stack.

## 2. Why the obvious suspects were all innocent

Each of these was checked and each was *correct*; the list is recorded because
every one of them is a plausible-sounding wrong answer:

* **IR, at every stage.** `CCC_DUMP_EACH_PASS=1` shows the length computation
  as `Sub(Load(alloca_v9), ParamRef(v5))` from `cleanup-copyprop1` all the way
  to the pre-codegen dump. No middle-end pass ever broke it.
* **The callee.** An instrumented build proved `acpi_ex_name_segment` advanced
  its out-parameter by exactly 4 (`in=0x405daf out=0x405db3 idx=4`) and stored
  it through the address the caller passed (`p_addr == p_slot`).
* **The caller's reads.** `caller_get_and_free` reloads both out-parameters
  from its stack slots (`movq 24(%rsp),%r11`, `movl 16(%rsp),%r10d`).
* **Argument passing.** `%rsi` is never re-materialized before the call, but
  nothing clobbers it between entry and the call, so the value is right.
* **Control flow.** The trace counters match the GCC oracle exactly
  (`trace=10366 exit=92883 alloc=16 free=16 arena_used=179`), so the same
  blocks ran the same number of times.
* **The assembler.** `objdump -d` of the failing binary matches the `.s`
  instruction for instruction.

Two traps worth naming:

* **Byte-identical binaries behaved differently.** `/tmp/single_lcccld` and
  `/tmp/copy_one` had the same md5 and different exit codes. The difference
  was the length of `argv[0]`, which shifts the initial stack — i.e. proof of
  an uninitialized read, not of a compiler that changes its mind.
* **Probes that touch memory "fixed" it.** Every `volatile` probe around the
  call made the failure disappear. Only non-`volatile` global probes
  (`p_val`, `q_len`, `s_in`) preserved the bug and produced usable data.

## 3. Root cause

Bisection with the compiler's own kill switches:

| switch | result |
|---|---|
| `CCC_DISABLE_PASSES=redundantloads` / `load_forward` | passes (red herring: changes IR shape) |
| `CCC_NO_SLOT_COALESCE=1` | passes |
| `CCC_NO_CFG_COPY_COALESCE=1` | **passes** |
| `CCC_SLOT_COALESCE_DROP=40,155` (one pair) | **passes** |
| dropping any other coalesced pair | still fails |

The pair is `v40`, the `u32` result of `call acpi_ex_name_segment`, and `v155`,
the `status` phi — which the type map widens to `I64` because one incoming is
`Const(I64(2))` (`AE_NO_MEMORY`). The whole codegen difference:

```asm
 call acpi_ex_name_segment          call acpi_ex_name_segment
-movl %eax, 56(%rsp)                +movl %eax, %r11d      ; 4-byte result
 jmp .LBB53                         +movq %r11, 56(%rsp)   ; full-width store
                                    jmp .LBB53
```

`.LBB53` reloads with `movq 56(%rsp), %r11` — **8 bytes out of a slot whose
low 4 bytes were just written**. The upper half is whatever the frame held,
which is exactly the ASLR/`argv[0]` sensitivity observed.

This is the same class as the ZSTD corruption documented at
`slot_assignment.rs:1240`. That fix ("width unification") rests on a stated
premise:

> the dest then stores through `movq` into the root's 8-byte slot, so every
> access to the shared slot is 8 bytes wide and no stale upper half can
> survive.

**A call result violates the premise.** Its spill comes straight out of the
ABI return register at the *call's* return width, and the SysV ABI leaves the
upper half of `%rax` undefined after a sub-8-byte return — unlike a 32-bit ALU
result, which is zero-extended into the full register. So the premise holds
only for ALU-produced narrow values, not for values arriving from an ABI
register.

### 3.1 Correction: what the width guard actually does

An earlier revision of this document claimed the guard "saw both members as
non-small". That was an unverified inference and it is wrong. Reading
`resolve_copy_aliases` directly:

* The 4→8 unification removes `dest_id` from `small_slot_values`, after which
  `is_small_slot(dest)` is false and `store_rax_to` does emit the widening
  `movq`. **When the unification happens, the spill is correctly widened** —
  that half of the mechanism is self-consistent.
* The width guard below it reads `small_slot_values` *after* that rewrite, so
  `contains(&dest) != contains(&root)` is false by construction and the guard
  can only ever fire in the 8→4 direction. It is dead in the 4→8 direction.
  That much is true.
* The failing case is therefore **not** a 4→8 unification. It is one where no
  unification happened at all: the call result kept its own 4-byte slot (hence
  `movl %eax, 56(%rsp)`) while a consumer reloaded that slot with `movq`.

**Making the dead guard live is not the fix — it is a regression.** Vetoing the
4→8 direction (snapshot both classes, refuse when the producer is an
ABI-register producer) was implemented and measured: it reproduces the correct
`-O1`/`-O2`/`-O3`/`-Os` assembly, but **segfaults 3/3 at `-O0`**, where the
pristine and shipped compilers are 0/3. The unification is load-bearing — the
phi still reads the value at 8 bytes, so refusing to share the slot leaves a
4-byte slot being read as 8. The guard's deadness in that direction is a
consequence of the intended behaviour, not the defect. Reverted; the shipped
compiler is byte-identical to the pre-change one (md5
`fed60cde22dc1858bd9adec089bf62bc`).

### 3.2 Also retracted: the peephole-phase hypothesis

An intermediate hypothesis placed the defect in peephole phase 1, because
`CCC_NO_PEEPHOLE_PHASE1=1` makes the reproducer pass. That is wrong too: with
phase 1 disabled the listing *still* contains the narrow store followed by a
wide reload (`movl %eax, 80(%rsp)` / `movq 80(%rsp), %r11`) — only at a
different slot, in a different frame (`subq $128` instead of
`pushq %rbp; subq $104`). Phase 1 perturbs layout and register numbering until
the stale bytes happen to be benign; it does not introduce the store. The
defect is entirely in the backend's slot classification and spill width.

## 4. Fix

`src/backend/stack_layout/copy_coalescing.rs`,
`collect_unsound_coalesce_ids()`: a call destination whose return type is a
sub-8-byte GPR scalar is now excluded from copy coalescing, alongside the
existing inline-asm exclusion.

Scoped as narrowly as the defect allows:

* **Pointer/64-bit returns keep coalescing** — they are already spilled with
  `movq`, so they fill the slot. That is the common case (every allocator
  call), so the restriction costs nothing there.
* Floats are already outside the coalescer's `scalar_type` class.

## 5. Validation

| check | result |
|---|---|
| reproducer, `-O0/-O1/-O2/-O3/-Os`, 5 runs each incl. a second `argv[0]` | 0 failures, `ALL-OK` at every level |
| `scripts/run_regression_suite.sh` | **PASS=729 FAIL=3 SKIP=17, AB-diff failures: 0** |
| codegen A/B, `tests/benchmark/**` + `tests/bench` (88 files, `-O2 -march=x86-64-v3`) | instructions 11577 → 11577 (**+0**), stack refs 750 → 750 (**+0**), 0 files changed |
| codegen A/B, `tests/regression/*.c` (739 files compiled) | instructions 156315 → 156317 (**+2, +0.0013%**), stack refs 22176 → 22176 (**+0**), **1 file changed** |

The single changed file is the reproducer itself, and the change is the
`movl`+`movq` pair above. The fix is surgical: it alters codegen only where
the bug was, at a cost of one instruction.

The 3 failures are pre-existing environment gaps, not regressions: the host
has no 32-bit libc headers (`/usr/include/bits/libc-header-start.h` is absent,
only the `x86_64-linux-gnu` multiarch variant exists) and no `qemu-i386`, so
every `-m32` test fails at *build* time.

## 6. Suite hardening

`run_regression_suite.sh` already runs an A/B differential against
`CCC_NO_SMALL_SLOTS=1`, which is the width-partition invariant this bug
violated — but it did not fire, because the mismatch was introduced by
*coalescing* two values into one slot, not by small-slot assignment. The new
regression test closes that hole from the test side: it is a plain
differential test whose GCC comparison fails loudly on the old compiler.

## 7. Open items

* The `movq` reload of a value whose type is 4 bytes is still emitted
  elsewhere; it is only *safe* while every writer of that slot is full width.
  Making the reload width follow the value type would remove the invariant
  this fix defends, and is worth doing as a separate change with its own A/B.
* The kernel build/boot gate could not be re-run this session
  (`boot gate: SKIP (KERNEL_DIR not set or tree not prepared)`).

## 8. Adjudication of the external review

An external review of this patch was assessed by re-deriving each claim from
the code and from measurements, not by accepting it. Verdicts:

| Claim | Verdict | Evidence |
|---|---|---|
| The width guard is dead in the 4→8 direction | **Correct** | `resolve_copy_aliases` reads `small_slot_values` after removing `dest_id`, so the comparison is false by construction. |
| Vetoing 4→8 is the right fix | **Refuted** | Implemented and measured: `-O0` segfaults 3/3 (pristine 0/3). The unification is load-bearing. See §3.1. |
| The fix does not cover a narrow **signed** result | **Incorrect** | `scalar_type` includes `I8`/`I16`/`I32`, and the condition is `scalar_type(ty) && ty.size() <= 4`, so signed narrow returns are already excluded. |
| `ParamRef` producers form a residual hole | **Plausible, unproven** | The `resolve_copy_aliases` comment states the zero-extension premise for ALU results only; an incoming argument register is defined only at the argument's width. No corpus test triggers it. Left open (§7). |
| `mixed_width_slots.py` is orphaned and should gate CI | **Half correct** | It is orphaned — and it was also *dead*: it matched only `(-?\d+)\(%rbp\)` while lccc emits `%rsp`-relative slots at `-O1` and above, so it could never report anything. Fixed. But it reports **510 mixed-width slots across 132 files** on the currently-green suite, so it cannot be a hard CI gate. It ships as an audit tool with an opt-in `--gate`. |
| The regression test is argv[0]-sensitive | **Correct** | 10/10 fail at a fixed argv[0]; flipping to argv[0] length ~40 makes it pass (lengths ~1/5/20/80 still fail). Under the real runner it is red on the pre-fix compiler (`FAILED (3 failures)`) and green on the fixed one, so it currently gates correctly — but the sensitivity is real and should be pinned. |
| The document's guard claim was unverified | **Correct** | Rewritten in §3.1. |
| The defect is a peephole-phase transform | **Refuted** | §3.2. |

Net: the review's most valuable contribution was noticing the guard's deadness
and the orphaned auditor. Its two proposed remedies — veto the 4→8 direction,
and gate CI on the auditor — are both measurably wrong, and adopting either
would have shipped a segfault or a permanently-red gate.
