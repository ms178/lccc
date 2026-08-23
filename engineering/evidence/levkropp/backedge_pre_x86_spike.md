# Backedge PRE x86 spike — levkropp `a08f676`

Date: 2026-08-23
Base: `ms178/lccc@85ad7d73cfe58f5ba4e317ac9fbf70490633e1eb`
Candidate: `levkropp/lccc@a08f676858efc56c93ff0e08bc1299c1d30b5233`

## Experiment

Imported levkropp's `src/passes/backedge_pre.rs` temporarily and ran it late in
`run_passes`, after integer constant hoisting and before dead statics. The pass
compiled and did not change the first-500 x86 GCC torture pass/fail set:

```text
PASSED 479 FAILS 21
```

Then measured `tests/benchmark/programs/mandelbrot.c` on the sandbox VM:

```text
mulsd counts: bepre=0, no-bepre=0
FMA counts:   bepre=3, no-bepre=3

/tmp/mandel_bepre   median 2.1175 s, min 2.1071 s
/tmp/mandel_nobepre median 1.9637 s, min 1.9536 s
/tmp/mandel_gcc     median 1.5007 s, min 1.4920 s
```

Both LCCC binaries produced identical output:

```text
mandelbrot total iterations: 380863975
```

## Decision

Rejected for default integration in this session. On x86-64, the candidate was
~7.8% slower on the exact workload levkropp cited as a win, while not reducing
FMA/multiply counts in the current ms178 backend. This likely reflects different
pass interactions in ms178 main: existing FP/FMA lowering already eliminates the
specific multiply shape, and the added phi/register pressure hurts scheduling or
allocation.

## Follow-up

Do not replay `a08f676` blindly. If revisited, make it target- and cost-model
aware:

1. Detect whether the top expression is actually emitted as a standalone
   instruction in the current backend.
2. Estimate added phi/register pressure and caller/callee-saved pressure.
3. Gate by target and benchmark evidence; re-test AArch64 separately where the
   original report may have been observed.
4. Keep `CCC_DISABLE_PASSES=bepre` or an equivalent kill switch during any
   future spike.
