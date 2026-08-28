# TASK-OP-05B — multi-store scatter + computed-invariant dot vectorization

IDs: OP-05b (extends landed OP-05a) · Priority: **P0** · Base: f657de55
· The dominant remaining FP gap: nbody 0.26× of GCC, spectral 0.34×.

## Objective

1. Vectorize bodies with MULTIPLE stores to distinct fields across one or
   two IVs (nbody `advance`: `bodies[i].vx -= …; bodies[j].vx += …`, 6
   stores over fx/fy/fz × i/j) using field-sensitive load/store
   disambiguation. The alias machinery (`alias.rs` forms_disjoint) exists;
   the analyzer currently requires exactly one store.
2. Recognize computed-invariant dot products (spectral `A(i,j)` — the
   expression is affine in j; synthesize the vector index math instead of
   trying to vectorize the integer division).

## Files

`src/passes/vectorize.rs` (analyzer + rewrite), possibly
`src/passes/alias.rs` for field-sensitive paths.

## Acceptance

- nbody: ymm count > 0 (GCC 81), stack refs 159 → <40 (the marching-pointer
  half is RA-01b; coordinate, don't block).
- spectral: instructions 212 → ≤170, checksums bit-identical at -O3 v3.
- No cross-iteration dependence guesses: unprovable pairs stay scalar
  (fail closed), pinned by an aliased-pointer negative regression.

## Validation battery

`cargo test --lib` · regression corpus (nbody, spectral_like_reduction,
simd_* structural gates) · checksum-verified A/B at -O2 and -O3 · the
0..trip-count sweep pattern (bit-exact) used by the stencil vectorizer ·
differential fuzz 300/300 O2/O3.

## Do not

- Do not copy GCC 16's per-iteration horizontal add; one horizontal
  reduction at loop exit (OP-06 contract).
- Do not mix legacy SSE into the widened loop (9× AVX-SSE transition
  penalty measured; all VEX three-operand forms).
- Do not relax the must-execute/alias guards to make the analysis fire
  (session 63: soundness first).
