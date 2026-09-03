# x86: condition tests must be sized to the condition width (`testl`, not `testq`, on 32-bit conditions)

**Date:** 2026-09-04 · **Branch:** `wip/audit358-359` · **Base:** `598d89eb`
**Defect origin:** pre-#366 select codegen; flagged as a latent defect in
`AUDIT-2026-09-03-natural-width-conditional-selects.md`, task 4.

## Summary

The x86-64 backend tested a *register-resident select/branch condition* with
`testq %r64, %r64` regardless of the condition's width. The SysV x86-64 ABI
leaves the bits above a parameter's width **undefined**, so a 32-bit condition
carried in `%edi` can have garbage in `%rdi` bits 32–63. `testq %rdi, %rdi`
then sets/clears `ZF` from that garbage: `c ? a : b` (or `if (c)`) on `c == 0`
could select/branch the *true* arm purely because the caller left stale high
bits. This is a latent miscompile, invisible to normal GCC-oracle differentials
(GCC callers zero-extend via 32-bit argument moves) but real for any caller
that passes a narrow argument without zero-extension — legal under the ABI.

## Root cause

Two register-direct condition-test sites hard-coded the 64-bit test:

1. `emit_select_impl` → `test_select_cond_in_place`: the in-place select-condition
   test used `testq %reg, %reg` for any GPR-homed condition.
2. `emit_cond_branch_blocks_impl` (register-direct path): the same `testq` for
   a branch condition.

The stack-slot path already sized correctly (`emit_cmp_zero_mem` emits `cmpl`
for I32-homed booleans), and the accumulator path is safe only because 32-bit
loads into `%eax` architecturally zero-extend — the raw *parameter register*
does not have that guarantee.

The condition's IR type was already available everywhere via
`self.value_types` (built by `compute_value_type_map`).

## Fix

`comparison.rs` gains `emit_cond_gpr_test(val_id, reg)`, which sizes the test
to the condition's IR type through the existing `cmp_width_info` +
`typed_phys_reg_name` helpers:

| condition type              | emitted test         |
|-----------------------------|----------------------|
| I8 / U8 (bools)             | `testb %r8, %r8`     |
| I16 / U16                   | `testw %r16, %r16`   |
| I32 / U32                   | `testl %r32, %r32`   |
| I64 / U64 / Ptr / unknown   | `testq %r64, %r64`   |

Both register-direct sites (select in-place test and branch register-direct
test) now call the helper. The memory (`cmpl` at the slot) and accumulator
(`testq %rax`) paths are unchanged and already width-correct.

Flags semantics are untouched (only `ZF`, read by the subsequent `cmovcc` /
`jcc`), so no instruction-count or scheduling change results — this is purely a
correctness-width fix.

## Evidence

### Assembly shape (before → after)

`int sel(unsigned c, int a, int b) { return c ? a : b; }` at `-O3`:

```asm
; before                      ; after
  testq %rdi, %rdi              testl %edi, %edi
  movq  %rdx, %r8               movq  %rdx, %r8
  cmovnel %esi, %r8d            cmovnel %esi, %r8d
  movl  %r8d, %eax              movl  %r8d, %eax
  ret                           ret
```

`long sell(long c, ...)` keeps `testq %rdi, %rdi` (verified). Sub-int and
32-bit select/branch conditions all emit the width-exact test.

### Dirty-caller runtime proof

Driver passes `c` with low 32 bits zero and garbage bit 32 set (legal SysV
call), expecting `sel` to return `b == 222`:

```
FIXED_BINARY rc=0 (returns 222)          BUGGY_BINARY rc=1 (returns 111)
```

The "buggy" object is the fixed source with the single instruction regressed
back to `testq %rdi, %rdi` — proving the scenario is real and that the width
fix is exactly what cures it.

### Differential

New `tests/regression/narrow_cond_width.c` (32/64-bit `?:` and `if` selects,
sign/unsigned, sub-int and constant arms) matches GCC output byte-for-byte at
`-O0/-O1/-O2/-O3`. Battery instruction counts are unchanged (width fix, not a
size fix).

### Gates (on the fixed tree)

- `scripts/run_regression_suite.sh`: **PASS=581 FAIL=0 SKIP=15** (AB-diff
  failures 0).
- `-O3` regression sweep: PASS=581 FAIL=0 SKIP=15.
- `tests/regression/check_narrow_cond_width.sh` (new): shape assertions for
  `testl`/`testq` widths + the dirty-caller runtime proof — green.

## Test coverage added

- `tests/regression/narrow_cond_width.c` — semantic differential across
  widths, opt levels, and the suite's A/B small-slot / Tier-2 sharing runs.
- `tests/regression/check_narrow_cond_width.sh` — asserts the emitted width of
  every register-direct condition test and drives the dirty-caller proof
  against the lccc-built object.

## Notes / next steps

This commit is the width-correctness half of audit task 4. The remaining
task-4 work (preloaded `bsr`/`bsf` branchless clz/ctz shapes, `wl`'s spurious
`movl %eax,%eax`) plus tasks 1–3 and 5 remain open, each as its own gated
commit.
