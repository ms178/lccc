# Packed relations and exact order networks

## Actual scope

This follow-up emphasizes real vector IR rather than scalar approximations. LCCC represents packed values with `Vec*` intrinsic webs, not a vector `IrType`. Scalar simplifiers cannot safely infer lane width or Boolean mask semantics from placeholder scalar types after `vecssa`.

The new `vecrelations` stage runs immediately after whole-vector SSA promotion and before vector FMA/temp cleanup. Its independent existing-policy disable token is `CCC_DISABLE_PASSES=vecrelations`; disabling it does not disable vector SSA promotion. No new environment-variable control, target feature or vector opcode is introduced.

Four previously deferred source scopes advance to **limited coverage**, bringing the existing 137-report ledger to 50 limited covered / 87 deferred scopes. This is neither a new 50-report audit nor resolution of every previous gap. General vectorization, memory legality, cross-block relational constraints, target costs and optimality remain open where the ledger says so.

## Packed contracts

Eight integer shapes: 8/16/32/64-bit lanes, in 128/256-bit registers.

- Wrapping `(a+b)-a` and `(a-b)+b`; no FP arithmetic cancellation.
- XOR cancellation, idempotent AND/OR, and self-XOR/subtraction to a full-register zero.
- Absorption `a & (a | b)` / `a | (a & b)`.
- Distributive factoring of two **private** AND/OR producers, three operations to two; private paired XOR cancellation, three to one. Copies are not allowed to conceal shared producers from the private-use guard.
- Nested identical-mask blends keep their original opcode, lane policy and arm orientation. Correlated opposite blends combine directly at bitwise consumers; integer addition additionally demands a matching canonical packed-compare mask.
- Pure FP **bitwise XOR**, including NaN payloads and signed zeros, uses register-image identities. FP add/sub/mul/min/max never enter the algebra table. Observable-FP-environment functions retain the whole-function policy fence, including producers that could raise FP exceptions.
- Unsigned `x < 2^k` / `x <= 2^k-1` compare masks remove `x & (2^k-1)` only in the selected true arm. The same proof applies to canonical bitwise selection (`cmp & (x & low)`), so GNU C mask webs need not already be normalized to blend opcodes. Compare/broadcast/AND opcodes and widths must agree. A byte blend may consume wider compare lanes because each such lane is uniformly all-ones/zero. A **coarser** blend cannot borrow that property from smaller lanes.

Integer lane literals on packed src2 are recognized as splats as well as explicit broadcasts. Returning such a literal materializes a vector broadcast, never a scalar copy; commutative factoring retains the supported src2 immediate position and refuses two-literal rebuilding. FP literal-result cancellation remains conservative. Functions without vector intrinsics bypass use-map and snapshot construction.

Only existing SSA values and bounded copy chains are followed. No loads, stores, calls, PHIs or casts are speculated or rebuilt. Matching is block-local, with fixed-size family tables and bounded copy walks; no decision-tree/BDD search. Shared producers are never mutated. Valid parallel source spans follow replacement instructions. Private factoring allocates checked fresh SSA IDs and preserves the destination of the original root.

## Scalar order and reversal contracts

Existing scalar selects provide min/max provenance, including signedness, operand order and tie choice. Direct min <= either input and max >= either input are true; strict opposite observations are false. Unsigned all-ones sentinel selection reuses an already existing unsigned min only with the exact bound operand and Eq/Ne orientation. These are integer observations, not invented assume/no-wrap metadata or a general ScalarEvolution solver.

Bit reversal is a permutation of a fixed-width word. It commutes with AND/OR/XOR/NOT and conjugates fixed valid left/logical-right shifts into the opposite direction. A bounded private-network builder (24 visits) removes reversal pairs across those operations. It refuses arithmetic, arithmetic-right shifts, casts, invalid/wide/negative/unknown counts and shared internal operations; shared reversal leaves remain untouched for their other users. Checked temporary allocation and source-root definition are retained. This is not arbitrary bitreverse expression sinking. On AArch64, the existing exact portable 32-bit reversal ladder is preserved before pre-inlining scalar simplification, so inlining exposes reversal semantics instead of losing the first stage to adjacent shift reassociation. A bounded single-block SSA analysis projection (at most 512 instructions) resolves the deliberately retained callee parameter homes without promoting the real inliner ABI bridge. Only original root definitions are replaced. The same disable token and native-target gate apply; no early recognition is enabled on unsupported targets.

## Executable evidence contracts

Rust production matcher tests cover all integer lane/register shapes, scalar 8/16/32/64 signed/unsigned carriers, byte-ring exhaustions/boundaries, edge/random lane values, bit-mask and sign-bit selection models, canonical-mask arithmetic, FP payload words, observable-FENV refusal, shared producers, mixed families, coarser blends, noncontiguous masks, invalid counts and SSA freshness.

`packed_vector_relations.c` contains 15 reduced contracts x four unsigned lane types x two register sizes: **120 native forms**. Independent scalar references run on aligned arrays viewed through GNU `may_alias` vector types. The checker uses exact GCC16.2 by default, optional exact Clang23.1.3, baseline SSE2 128-bit and AVX2 256-bit modes, O0/O1/O2/O3/Os/debug, i386 SSE2, IR/SSA/RA verifiers and an **isolated** `vecrelations`-disabled control. It requires at least eight instruction-count improvements and actual XOR elimination in the cancellation witnesses, plus a strict instruction-count reduction for every bounded-mask witness. These are code-shape controls, not runtime claims.

`integer_order_networks.c` exercises all six original reversal shapes, unsigned sentinel/min observations and signed/unsigned extremum observations. The checker runs native vendor, debug/i386/verifier and whole-bit_idioms-disabled controls. The ordinary CI lane additionally checks AArch64 assembly for residual `rbit` in the six conjugation witnesses. Whole-pass-disabled results are not an immutable pre-implementation baseline.

Ordinary frontend CI runs both checkers with pinned Clang23.1.3 and an explicitly supplemental GCC16 snapshot if needed. The Debian exact-version lane requires GCC16.2, retains the packed JSON report, and runs the scalar fixture too. Both are mirrored before the local runner's summary/stamp. Existing independent contract shards, oracle provisioning and compiler packaging remain intact. Hosted CI supplies execution receipts; this document does not assert an unobserved result.

## Original reports and remaining boundaries

Original bodies and all accessible paginated conversation comments are retrieved through GitHub REST, hashed in `packed-relations-sources.json`; raw records remain ignored workspace data. Linked external reductions/proofs were not fetched or executed. Hashes identify collection, not semantic truth.

| Report | Limited advance | Still not claimed |
|---|---|---|
| [192094](https://github.com/llvm/llvm-project/issues/192094) | Real packed unsigned bounded-mask true-arm simplification, including the reported i16 shape | Arbitrary vector assumptions, signed/non-power-of-two ranges, coarser mask selection |
| [219334](https://github.com/llvm/llvm-project/issues/219334) | All six original private bitwise/fixed-shift reversal shapes at IR level | Arithmetic conjugation, shared DAG rebuilding, arbitrary count ranges |
| [202576](https://github.com/llvm/llvm-project/issues/202576) | Exact all-ones unsigned sentinel around existing scalar min-select | Invented min intrinsic semantics, signed/nonmaximum sentinels, unrelated bounds |
| [205005](https://github.com/llvm/llvm-project/issues/205005) | Direct scalar min/max observations before offset rewriting | Upstream loop-vectorizer dereferenceability / constraint-solver memory-safety proof |
