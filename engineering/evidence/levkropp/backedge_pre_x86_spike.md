# Backedge PRE x86 spike — FP regression root cause and corrected production model

Date: 2026-08-23
Base: latest `ms178/lccc` main after session-66 work.
Candidate seed: `levkropp/lccc@a08f676858efc56c93ff0e08bc1299c1d30b5233`.

## Root cause of the original FP regression

Naively carrying Mandelbrot's `zr*zr` across the backedge removes one static
`vmulsd`, but it creates a new loop-carried FP value. Before this session, the
backedge source was defined in the branch block before the latch-copy block, so
`detect_phi_coalesce_groups` rejected it:

```text
[PHI_COALESCE] BLOCKED phi_dest=Value(99) src=Value(52): def block/index 6:6 != copy 7:4
```

The backend then emitted an extra loop-carried FP copy and the transform was
slower despite one fewer multiply.

## Regalloc/codegen fix

The phi coalescer now accepts the specific safe cross-block destructive-update
shape for FP sources (the only production user):

- the copy block has exactly one predecessor;
- that predecessor is the source-def block;
- the source definition has FP type;
- the phi value is not used after the source def and before the copy;
- the source is not used outside the source/copy block pair;
- caller-saved clobber checks cover both windows.

A broader integer cross-block version was tested and immediately caught by GCC
torture `20041011-1.c`; it is intentionally not enabled. Same-block integer phi
coalescing remains unchanged.

With `CCC_BEPRE_FP=1`, Mandelbrot now coalesces the carried square:

```text
[PHI_COALESCE] Coalescing phi_dest=Value(99) with backedge_src=Value(52) source block 6 idx 6 copy block 7 idx 4
```

and the extra `movapd %xmm15,%xmm11` backedge copy disappears.

## Remaining FP profitability problem

Even after coalescing, Mandelbrot FP PRE is still slower on this x86 VM:

```text
CCC_BEPRE_FP=1: median 1.98 s
baseline       : median 1.73 s
```

The reason is critical-path placement: the carried square is produced by the
magnitude-test computation late in the loop. Feeding it into the next iteration
lengthens the recurrence dependency versus recomputing the square at the top of
the next iteration. A local scheduler that hoists the bottom square immediately
after `tr` is available improved the shape but still did not beat baseline.

Therefore Mandelbrot-style singly-used FP-square PRE remains disabled by default.

## Corrected default FP cost model

Default FP PRE now fires only when the top FP expression has multiple direct
uses. That captures cases where the removed expression feeds more than one chain
and avoids the single-use Mandelbrot regression. The broad FP path remains
available for research with `CCC_BEPRE_FP=1`.

Structural guardrails:

```text
# Multi-use FP recurrence: default PRE reduces FP multiply count.
backedge_pre_fp_multiuse.c: vmulsd default=2, CCC_DISABLE_PASSES=bepre=3

# Mandelbrot: default PRE must not perturb known-regressing FP shape.
mandelbrot default assembly == CCC_DISABLE_PASSES=bepre assembly
```

Runtime evidence for the long multi-use FP microbenchmark is small but positive
within VM noise:

```text
/tmp/fp_default_bench median 0.7857 s
/tmp/fp_off_bench     median 0.7864 s
speedup ≈ 1.001x
```

This is intentionally conservative: the pass records a structural FP win without
risking known real-workload regressions.

## Integer recurrence win remains solid

```text
Backedge PRE on : 1 imul in run()
Backedge PRE off: 2 imul in run()

/tmp/bepre_int_on2  median 0.1619 s
/tmp/bepre_int_off2 median 0.1850 s
speedup: 1.14x
```

## Decision

Integrate Backedge PRE with:

- integer PRE enabled;
- multi-use FP PRE enabled;
- singly-used FP PRE off by default unless `CCC_BEPRE_FP=1`;
- cross-block phi coalescing to remove safe branch-separated latch copies;
- structural regressions for integer and FP codegen guardrails.
