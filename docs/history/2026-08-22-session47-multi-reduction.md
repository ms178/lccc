# Session 47 — multi-reduction vectorization (two independent accumulators)

Base: `ms178/lccc` main `8b7e3bc` (PR #183, post sessions 44–46). Build:
`fastbuild`, Rust `-O1`, no LTO, two jobs, 8 GB swap. Host: constrained VM,
no PMU (deterministic checksums + assembly + Callgrind-free wall screens).

## What this session changed

The reduction vectorizer rejected any loop with a second zero-init phi
("Reject loops with multiple reduction phis").  Real statistical /
complex-correlation loops — `vBv += u[i]*v[i]; vv += v[i]*v[i]`, two running
sums, two independent dot products — therefore stayed scalar.  This session
adds **two-accumulator (multi-reduction) vectorization** for the Sum and
DotProduct kinds on the AVX2 (4/8-wide) and SSE2 (2-wide) paths, while keeping
the single-accumulator path bit-identical (A/B-verified on the 36-file corpus).

### Evidence (pinned, paired-median, `run_benchmarks.py --reps 11`)

| `double_reduction` (new) | LCCC | GCC | LCCC/GCC |
|---|---:|---:|---:|
| base `8b7e3bc` (scalar) | 409.44 ms | 109.27 ms | **3.75× slower** |
| this session (multi) | 119.22 ms | 105.67 ms | **1.085× slower** |

~3.5× faster than LCCC's own scalar baseline; the residual 8% to GCC is the
pre-existing vector-accumulator stack-home issue (see Follow-up), not this
transform.

## Design (audited)

- `ReductionPattern.second: Option<SecondaryAccumulator>` carries a second
  accumulator's phi, add site, array GEPs, derived set and loads.
- `analyze_secondary_accumulator` requires: same kind, element type,
  accumulator type and body block; a cast/copy-closure disjoint from the
  primary's (independence — rejects Adler-style `sum2 += sum1`); IV-based GEPs.
  Widening (element != accumulator) and >2 accumulators fail closed.
- `reduction_pattern_is_sound` now takes slices of adds/phis and one union
  derived/load set, so both chains are validated together.  With one
  accumulator this reproduces the historical check exactly.
- `rewrite_reduction_body` rewrites one accumulator's body (vec zero + vec
  load[/mul] + vec add, phi entry/backedge rewire) and is called per
  accumulator in **descending add-index order** so insertions never shift a
  still-to-be-patched lower index.
- `insert_reduction_remainder_loop` gained a `second` parameter: a second
  horizontal reduce, a second scalar remainder phi/chain, and a second
  use-rewiring pass (`rewrite_accumulator_uses_outside_loop`, extracted from
  the former Step 7).
- Transform-wide sweeps (contiguity precondition, byte-IV strength reduction,
  element-index GEP scaling) cover both accumulators via
  `reduction_array_geps`.

## Red-team audit findings fixed during development

1. The per-kind sound checks ran before the second accumulator was known and
   rejected its loads as "foreign" — moved to a single union check after both
   accumulators are analyzed.
2. `rewrite_reduction_body`'s dot-product fallbacks used
   `vec_mul_op.unwrap_or(vec_add_op)` — a silent miscompile if ever mis-wired;
   replaced with `expect()` so a configuration error fails loudly.
3. The benchmark's first draft kept accumulators live across an outer loop
   (non-zero-init phis) — the reduction pattern is only recognized for
   zero-init phis, so the kernel was reshaped to reset accumulators per pass.
4. `-m32` (`check_load_widen_cast_no_relay`) and `check_tier2_graph_default`
   failures were **confirmed pre-existing on base `8b7e3bc`** (i686 relay;
   huft frame 1816=1816) and are unrelated to this change.

## Validation

- **1045** `cargo test --lib` (0 failed), **391 + 1 skip** regression
  (+1 new `check_multi_reduction_vectorize.sh`), **50/50** correctness,
  **360/360** O2/O3/Os differential fuzz.
- Targeted runtime A/B vs GCC: int double dot/sum (n = 0..300 incl. remainder),
  swapped statement order, 3-accumulator reject, dependent `sum2 += sum1`
  reject, `long long` dot (SSE2 path), FP `vBv/vv` under `-ffast-math`.
  All byte-identical to GCC.
- 36-file benchmark/pattern corpus: byte-identical to base except the new
  `double_reduction` (which now vectorizes).
- Warning-clean build (`-D warnings`).

## Follow-up

1. ~~**Vector accumulators still live in stack slots.**~~ **DONE in session 48.**
   Width-aware reduction homes now include I32x8/I32x4 and I64x2 sum/dot
   webs; integer zero/horizontal/multiply emitters honor assignments and the
   XMM2 scratch quarantine no longer disables a pool beginning at XMM3.
   `double_reduction` is 0.7251× its stack-home control (95% CI
   0.6007..0.8807) and 0.895× GCC in the current VM screen.
2. **Shared-load dedup.** `b += v[i]*w[i]` after `a += u[i]*v[i]` emits the
   `v[i]` vector load twice; CSE the per-accumulator VecLoads by GEP.
3. **>2 accumulators.** The analyzer rejects three-plus accumulators; a
   `Vec<SecondaryAccumulator>` generalization is mechanical but was out of
   scope.
4. Re-validate on the i7-14700KF with PMU; the 3.5× screening gain is a VM
   wall-clock claim only.
