# Session 66 — Backedge PRE made production-profitable, with FP regression fixed by cost model + coalescing

Date: 2026-08-23
Base: latest `ms178/lccc` main after session-65 merge.

## Goal

Redo Backedge PRE properly: identify the FP regression root cause, fix backend
issues where possible, and integrate only the profitable shapes.

## Implementation

Added `src/passes/backedge_pre.rs`, derived from levkropp's prototype but with a
production cost model and backend support:

1. **Integer recurrences enabled by default.**
   The pass carries `f(next)` to the next iteration's `f(phi)` and removes the
   redundant top expression.
2. **Cross-block FP phi coalescing in regalloc.**
   Backedge PRE often produces an FP source def in a condition block followed by
   the phi-copy in a unique latch successor. `detect_phi_coalesce_groups` now
   accepts that safe FP-only shape and `apply_phi_coalesce_assignments`
   revalidates both the source block tail and copy block prefix. A broader
   integer cross-block variant was rejected by GCC torture `20041011-1.c`; the
   production rule is deliberately narrower. This removes the extra FP backedge
   copy that caused the first Mandelbrot regression.
3. **Early bottom-expression scheduling.**
   The pass moves the carried bottom expression to the earliest safe point after
   its operands are available, shortening the carried value's critical path.
4. **FP profitability guard.**
   Singly-used FP expressions remain disabled by default because Mandelbrot still
   regresses after coalescing: the carried square is produced late by the
   magnitude test and lengthens the recurrence. Multi-use FP expressions are
   enabled; broad FP experimentation remains available with `CCC_BEPRE_FP=1`.
5. **Constant preheader incoming fold.**
   `f(init)` is folded when both init operands are constants, avoiding useless
   preheader work like `1*1` or `0.0*0.0`.
6. **Kill switches.**
   `CCC_DISABLE_PASSES=bepre` or `CCC_NO_BEPRE=1` disables the pass.

## Evidence

### Integer recurrence

Regression/perf shape:

- `tests/regression/backedge_pre_int_recurrence.c`
- `tests/regression/check_backedge_pre_int_codegen.sh`

Codegen:

```text
Backedge PRE on : 1 imul in run()
Backedge PRE off: 2 imul in run()
```

Runtime on sandbox VM, 200M iterations, randomized 7-run A/B:

```text
/tmp/bepre_int_on2  median 0.1619 s, min 0.1595 s
/tmp/bepre_int_off2 median 0.1850 s, min 0.1829 s
speedup: 1.14x
```

### FP regression analysis and fix

The original FP regression was real. Debugging showed the first backend issue:

```text
[PHI_COALESCE] BLOCKED phi_dest=Value(99) src=Value(52): def block/index 6:6 != copy 7:4
```

After the cross-block phi-coalescing fix:

```text
[PHI_COALESCE] Coalescing phi_dest=Value(99) with backedge_src=Value(52) source block 6 idx 6 copy block 7 idx 4
```

The extra FP backedge copy disappeared. However, Mandelbrot still regressed with
broad FP PRE because the carried square is produced late on the magnitude-test
critical path. Therefore default FP PRE is cost-modeled:

- multi-use FP top expressions: enabled;
- singly-used FP top expressions: disabled unless `CCC_BEPRE_FP=1`.

Structural FP guardrails:

```text
backedge_pre_fp_multiuse.c: vmulsd default=2, bepre-off=3
mandelbrot default assembly == CCC_DISABLE_PASSES=bepre assembly
```

Runtime for the long multi-use FP benchmark is small but positive within VM
noise:

```text
/tmp/fp_default_bench median 0.7857 s
/tmp/fp_off_bench     median 0.7864 s
speedup ≈ 1.001x
```

The important result is that the known Mandelbrot FP regression is eliminated by
default while a structurally profitable FP PRE class remains enabled.

## Correctness validation

```text
scripts/build_lccc_fast.sh
cargo test --profile fastbuild --lib range_check --locked -j 2
# 3 passed; 0 failed

scripts/x86_gcc_torture_slice.sh 20041114-1.c 20070919-1.c 20180112-1.c
# 3 pass / 0 fail

First 500 native x86 GCC torture execute slice:
# TORT500 479 pass / 21 fail
# fail set unchanged from previous baseline
```

Regression validation:

```text
PASS tests/regression/backedge_pre_int_recurrence.c
PASS tests/regression/backedge_pre_fp_multiuse.c
PASS tests/regression/pointer_compound_assign_signed_index.c
PASS tests/regression/path_range_var_minus_one_uintmax.c
PASS tests/regression/vla_struct_assignment_runtime_memcpy.c
PASS tests/regression/check_backedge_pre_int_codegen.sh
PASS tests/regression/check_backedge_pre_fp_codegen.sh
```

## Follow-up

- To enable Mandelbrot-style singly-used FP PRE, regalloc/scheduling must reduce
  the recurrence critical path, not merely remove the visible backedge copy.
- Re-test on AArch64 with hardware/QEMU + counters; levkropp's original report
  may have been target-specific.
