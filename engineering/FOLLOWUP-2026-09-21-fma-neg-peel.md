# FOLLOWUP 2026-09-21 — session 572: builtin FMA operand-negation absorption

## What landed

PR #571 (the FMA form algebra + the four signed families from negation
SOURCE shapes) left one documented gap: the BUILTIN sign variants —
`__builtin_fma(-a, b, -c)` and its five siblings — still materialised
their operand negations as `vxorpd`/`vxorps` instructions in front of a
plain `vfmadd`. The gate's own comment carried the open item ("absorbing
them needs const-pool visibility in the peephole — the open item in
FOLLOWUP-2026-09-19E"). GCC folds every one of the six spellings to a
single instruction at -O1 and above; lccc spent up to eight.

This session closes it at a different layer than the peephole: a new IR
pass (`src/passes/fma_neg_peel.rs`) peels single-use float `Neg`
instructions off the arguments of a plain `FmaScalarF{32,64}` and folds
the signs into two new target-neutral intrinsic variants
(`FmaScalarF{32,64}Signed(negate_product, negate_addend)`), so the
register allocator simply sees the true live ranges.

Why the IR layer and not the peephole or the generation pass:

* The text-level fold would need const-pool value visibility
  (`LineStore` has none) to prove the `vxorpd` operand is the sign mask.
* The generation-layer fold (like `MulAddFusion`) would read the
  pre-negation operand LATER than its IR liveness recorded, needing
  gap-fusion-style register/slot clash analysis at every emit site.
* The IR rewrite in phi-SSA form is correct by construction: the new
  operand dominates the use by transitivity (src dominates the Neg, the
  Neg dominates the fma's use), and `eliminate_phis` still runs after
  `run_passes`, so the pass reasons in proper SSA.

## Rules (and their tests)

* Single-use float Neg of the matching width only; chains peel
  transitively (`fma(-(-a), b, c)` → plain); two product-side negations
  cancel back to the plain intrinsic; integer Neg is rejected by the
  width check (forward discriminator: an EVEX- or typing-level change
  that reintroduces mistyped shapes must not fold silently).
* Multi-use Negs stay materialised (the b_shared_neg control).
* NOT contract-gated (C99 builtin semantics; `-ffp-contract=off` still
  fuses, verified). `-mno-fma` fails closed to the libcall (verified).
* Placement: last IR transform at -O1 and above, after every vectorizer;
  -O0 keeps GCC's materialised-negation shape (verified against the
  oracle at both tiers).
* Kill switches `CCC_NO_FMA_NEG_PEEL` / `CCC_DISABLE_PASSES=fmanegpeel`,
  resolved once in `run_passes` (env-hygiene budget stays 157).

## ISA tables (oracle/spec-verified this session)

| (np, na) | x86-64/i686    | AArch64 | RISC-V |
|----------|----------------|---------|--------|
| (F,F)    | vfmadd231s{d}  | fmadd   | fmadd  |
| (F,T)    | vfmsub231s{d}  | fnmsub  | fmsub  |
| (T,F)    | vfnmadd231s{d} | fmsub   | fnmsub |
| (T,T)    | vfnmsub231s{d} | fnmadd  | fnmadd |

AArch64 confirmed against ARM64 gcc 12.4 on the godbolt oracle (the
famously confusing inversion: `sub` negates the product, `n` the
addend); x86 against local gcc; RV against the unprivileged spec table
(negated-product spellings invert the madd/sub suffix meaning). The
x86/ARM tables already in the tree were re-verified correct — the
red-team suspicion of an inversion was checked and dismissed with the
oracle, not with memory.

## Verification

* Unit tests: 12 new (adjacent, non-adjacent, chained, cancellations,
  multi-use keep, width mismatch, f32 width, gate-off, span alignment);
  3069 total, 0 failed.
* Gate (`check_vector_copy_elimination.sh`): 36/0 — 35 one-instruction
  kernels (7 new), 28 contracted twins (5 new), the multi-use
  anti-vacuity ratchet (exactly one kernel-body vxorpd), all corpus
  ratchets unchanged, copy budget 0.
* Differential vs libm fma: 3360 f64 + 810 f32 operand combinations
  (±0, ±Inf, both NaN signs, overflow, subnormals) — zero mismatches.
* i686 -mfma: byte-exact VEX3 `vfnmsub231sd` object (runtime execution
  remains env-blocked: no 32-bit multilib; same status as the corpus).
* AArch64: all four families match the oracle's selections.
* Corpus 723/3-env/17 (baseline-identical), ci_local --fast 53/0/4,
  benchmark output oracle 204/204, rustfmt clean, clippy
  (lib/tests/bins run separately — the combined check OOMs the 4 GB
  box) green, CCC_VALIDATE_SSA green at -O1/-O2 for all shapes.

## Residuals (designed, documented)

1. `fma(-a, -a, c)` (the SAME negated value in two argument positions)
   keeps both negations: the single-use discipline rejects it. GCC folds
   it; the spelling is pathological and the generalisation needs
   per-position "all uses within this fma" fixpoint reasoning across
   chain levels. Not worth the complexity today.
2. ~~The vectorized counterpart~~ — CLOSED the same session: MapExpr::Fma
   (commit "vectorizer: element-wise builtin fma loops vectorize").
   Element-wise `__builtin_fma` loops — plain and every sign spelling —
   now vectorize to the packed 132 families through two new intrinsics
   (VecMaddF{64x4,32x8}Signed) and a loop-body mini-peel in the map
   parser. Residual within the residual: an INVARIANT Neg (hoisted out
   of the loop) broadcasts its negated value instead of folding into
   the family, and 128-bit (non-AVX2) targets keep the loop scalar
   (the map emitter has no 128-bit packed-FMA wiring; the SLP
   VecFmaF64x2 family exists but is register-convention-incompatible).
3. RISC-V never materialises FmaScalar today (the libcall fold is
   x86/ARM-gated), so the RV lowering arm is forward-compatibility: it
   exists so the target-neutral variant is lowerable on every backend,
   mirroring the plain variants' status in that file. If the RV fold is
   ever enabled, the arm is ready and its table is spec-verified.

## Merge-conflict root cause (PR #570, recorded for posterity)

Both PR #569 (NonZero bitcount) and PR #570 (FP negation) anchored a
new dispatch block immediately after the F128 Neg early-return in
`emit_unaryop_impl`; the conflicts were textual adjacency, not semantic
overlap (disjoint in (type, op) space — the resolution kept both
blocks). The squashed upstream merge (fb8c8b9d) carries the full
resolution; `git diff origin/main pr570-merge` is empty (tree-exact
confirmation of complete absorption).
