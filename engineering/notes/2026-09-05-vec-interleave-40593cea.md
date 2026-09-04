# Session follow-up: vec_interleave — vector-reduction accumulator interleaving

Base: `40593cea` (ms178/lccc main, PR #403). Work on top: one new IR pass,
one pipeline hook, one emitter extension set, three regression tests.

Validation: lib suite **1863 passed / 0 failed / 6 ignored** (incl. 6 new
unit tests). Full regression suite **PASS=614 FAIL=0 SKIP=15** (608 baseline
+ 2 new numeric tests + upstream additions). Structural asm test green.
Measured (min-of-6, VM noise accounted for): dot product F64 — L1-resident
**0.16 ns/elem vs GCC 0.30** (1.9× faster, 7.6× vs lccc's single chain);
L2 0.35 vs 0.36 (tie with GCC, 3.5× vs kill switch); L3 0.48 vs 0.48
(tie, 2.6× vs kill switch).

## What was built

`src/passes/vec_interleave.rs` — splits every canonical 2-block vector
reduction loop the vectorizer produces into IF independent accumulator
chains feeding a displacement-folded main loop, keeping the original loop
verbatim as the epilogue (ICX's `vfmadd231pd 32(%rsi,%rax), %ymm1, %ymm5`
style, Rule 15). Exact for any limit (`limit_main = limit & ~(IF*step-1)`),
no divisibility assumption, FP only under `fp_reassoc`, integer always,
`max` chains reuse the seed (single-max-acc only for v1). Kill switch
`CCC_NO_VEC_INTERLEAVE=1`, factor override `CCC_VEC_INTERLEAVE=2|4|8`,
`CCC_DISABLE_PASSES=vec_interleave`.

Wiring: pipeline after `vectorize`, before `vec_load_sink`, gated to
X86_64 (AArch64's vector-load emitter reads args[0..2] and would silently
drop a folded displacement — miscompile risk, deliberately excluded).

Emitter: `emit_avx_reduction_fma` accepts the optional trailing constant
displacements (args[5]/args[6] for the A/B memory operands);
`VecLoadI32x8/I32x4/I64x2` and `VecWidenAddI32x4ToI64x2` gained the same
optional args[2]/args[3] displacement the F64/F32 loads already had. The
2-arg / 5-arg forms emit byte-identical output to before.

## Audit fixes applied to the lost Fable draft

1. `next_value_id` seeding replaced with the sound `max_value_id()+1`
   bound (the cached hint is documented "0 = not computed" at this
   pipeline point) + `.max()` write-back; labels seeded from the block max.
2. Direct value-uses of the IV (GEP bases, dest_ptr) in slices k>0 were
   renamed to `ivm` instead of the shifted `iv_k`; fixed by detecting
   needs_iv_k through BOTH operand and value-use visits and patching the
   rename map when `iv_k` is materialised.
3. Dead/unsafe table rows removed: `VecAddI64x4` (x86 emitter has no
   lowering — a combine emitted into a no-op), `VecLoadI64x4` and
   `VecLoadWidenI32ToI64x2` (unimplemented emitters — a folded displacement
   would be silently dropped).
4. `VecWidenAddI32x4ToI64x2` added to the displacement slots (2,3) with
   matching emitter support.
5. `lp.body.contains(&header)` guard; the max+multi-acc rejection now
   documented as deliberate conservatism.
6. `fp_reassoc` plumbed from the pipeline (the draft read it only in tests).

Bugs I introduced and fixed during the session (self-audit): the first
emitter patch also rewrote `VecWidenMaskedAddI32x4ToI64x2`'s load, whose
args[3] is the MASK — a constant mask would have been misread as a
displacement. Reverted; only the four intended emitters carry the new
disp slots, each with an actual disp-reading lowering.

## Tests

- Unit: acc table consistency, FMA acc-slot contract, group-mask
  exactness, full synthetic-IR transform + structural verifier + 4-chain
  displacement assertions, fp_reassoc gate, kill switch.
- `tests/regression/vec_interleave.c` (+.flags `-O3 -march=x86-64-v3`):
  integer dot/sum/widen, 1000-size sweep + full size, exact GCC-oracle
  compare (integer reassociation is bit-exact).
- `tests/regression/vec_interleave_fp.c` (+.flags fast-math, .env
  LCCC_NO_COMPARE=1): FP dot/sum/dotf with tolerance gate against a
  volatile ordered scalar reference, 24 sizes incl. 0..3 and vector
  boundaries.
- `tests/regression/check_vec_interleave_codegen.sh`: structural asm
  assertions on simd_fp_oracle.c p17/p18 — >= 4 chains, >= 3
  displacement-folded memory operands, kill-switch control restores the
  single-chain form.

## Measurement notes (important for future sessions)

- This sandbox VM is noisy; single-run timings vary ~2-3×. Always use
  min-of-N (6+) with a fixed clock_gettime harness before claiming
  performance numbers.
- 16 MB working sets are L3-bandwidth-bound (~33 GB/s) — both compilers
  tie there; the interleave's win is compute-bound (L1/L2) territory,
  exactly the latency-bound case the pass targets.
- The post-vectorize unroll pass does not double-unroll the interleaved
  main loop (verified in asm: exactly 4 chains, no 8× blow-up).

## To-do / next session

- Consider relaxing the max/multi-acc rejection (seed-reuse correctness
  is proven; needs tests + a workload that wants it).
- The combine block spills one temporary (executed once per loop nest —
  negligible, but a vhaddpd tree would be cleaner).
- Watch for the two movslq IV sign-extensions in the main loop; keeping
  the IV 64-bit (IVSR/iv_widen interplay) would shave 2 µops/iter.
- AArch64 NEON interleave would need emitter disp support + benchmarks
  before un-gating.
- Benchmark more shapes (matmul, sum-of-squares, mixed 2-acc) with the
  min-of-N harness; add godbolt evidence if the oracle tooling is up.
