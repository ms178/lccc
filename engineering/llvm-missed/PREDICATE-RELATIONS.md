# Predicate relations and complete deferral follow-up

## Actual scope

Retrieve **50 previously unreviewed reports** and revisit **all 56 prior deferred inventory entries**, including the 43 from scalar logic and 13 older entries. The result is **5 new limited covered scopes and 10 prior deferrals advanced to limited coverage**. **91 of the 106 entries remain blocked or refused in the stated scope. This is not completion of all deferrals, 50 new optimization families, zero defects, or global optimality.**

Raw API records are ignored workspace data in `work/predicate-relations/`. Hashes/counts in `predicate-relations-sources.json` identify those retrieved records, not upstream correctness. Linked Alive2/Compiler Explorer programs and attachments were not fetched/executed. Historical source manifests retain their original dispositions; current inventory and coverage follow-ups record this review separately.

## Implementation contracts

- Consumer-local scalar select substitution follows only identical/inverse integer predicates, copies, and bounded canonical Boolean casts. Existing path facts simplify zero additions and redundant low masks. Shared producers are not mutated, moved or cloned.
- Commutative operations and Eq/Ne of symmetric correlated selects read existing arm values directly. Constant/equal-arm correlated comparisons preserve signedness and produce canonical I32 Booleans. Constant-choice arithmetic is folded only when both outcomes are constants.
- Contiguous OR/AND observations against the original operand become unsigned bounds. Sparse masks refuse. Low-mask minus bounded x becomes XOR only with an explicit known-bit proof.
- Guarded increment and top-aligned contiguous-mask conjunctions are exact over wrapping rings; missing guards refuse. Identical disjoint nonzero constant choices observed through AND compare canonical predicates. Sum-of-two-zero/one values compared to one becomes XOR, not a sext-mask reinterpretation.
- Fixed valid shifts propagate known bits. Nonzero power-of-two masks supply one known-one bit to the existing unique-predecessor facts engine. Multi-bit nonzero masks do not imply any individual bit. OR/LShr ordering requires explicit valid count bounds, never backward inference from poison.

Matching uses the owner's 512-definition budget and local depth limits. No new environment knob, unsafe code, target legalization or CFG/memory transformation. Existing CH/MAJ policy and disabled-pass controls remain authoritative. No timing/speedup claim; whole-pass disabled comparisons are explicitly not an immutable pre-implementation baseline.

## Regression contracts

Rust tests cover all eight scalar integer carriers; byte-ring mask boundaries, guarded increments, constant choices, signed/unsigned comparisons, symmetric selects, shared producers, shift known bits, invalid guards and unrelated predicates. The native fixture has 31 total unsigned contracts at 32/64 bits (62 forms), independent reduced references and 1,024 edge/random rounds. Its GCC16/optional pinned Clang23.1.3 checker exercises O0/O1/O2/O3/Os, debug, v3, i386, IR/SSA/RA verification, mandatory mask removal and disabled-pass controls. GitHub CI, not this document, supplies execution receipts.

The inventory test checks the exact 50+56 identity sets, all follow-ups and hashes, and executable counterexamples to dangerous generalizations. Those counterexamples are not substitutes for production optimization tests.

## Per-report decisions

### New reports

| Report | Decision | Scoped work / remaining blocker |
|---|---|---|
| [33576](https://github.com/llvm/llvm-project/issues/33576) | reviewed-deferred | Equivalence of select-of-add and add-of-select requires symbolic congruence across shared arithmetic, not merely same-condition arm elimination. |
| [45821](https://github.com/llvm/llvm-project/issues/45821) | reviewed-deferred | Complementary comparisons are canonical zero/one values, not bitwise word complements. The word truth-table synthesizer cannot substitute NOT without a Boolean-only proof. |
| [51748](https://github.com/llvm/llvm-project/issues/51748) | reviewed-deferred | Threading a shift through a select needs proof that both per-arm shifts fold and all counts are valid. Variable operations are not cloned. |
| [57635](https://github.com/llvm/llvm-project/issues/57635) | reviewed-deferred | Eliminating x-1 under x!=0 needs inequality/path nonzero intervals; a generic unsigned decrement wraps at zero. |
| [58589](https://github.com/llvm/llvm-project/issues/58589) | reviewed-deferred | LLVM poison/freeze semantics differ from LCCC total eager integer operations. Removing IsLogical guards globally is not a local source rewrite. |
| [59266](https://github.com/llvm/llvm-project/issues/59266) | reviewed-deferred | Coalescing a Boolean comparison mux into x==select needs CFG/Boolean provenance and a target cost decision; no eager evaluation of short-circuit effects. |
| [59393](https://github.com/llvm/llvm-project/issues/59393) | reviewed-deferred | Reordering two independent select conditions requires decision-tree construction and sharing costs; direct same/inverse-condition substitution does not cover it. |
| [60167](https://github.com/llvm/llvm-project/issues/60167) | reviewed-deferred | The motivating nested logical condition is composite, not the same predicate as its outer select; needs Boolean implication beyond the current local relation. |
| [61365](https://github.com/llvm/llvm-project/issues/61365) | reviewed-deferred | Three-input LEA formation is x86 microarchitecture and instruction-encoding policy, not a target-neutral scalar-select identity. |
| [63321](https://github.com/llvm/llvm-project/issues/63321) | reviewed-deferred | LLVM load range {0,2} and i1 trunc semantics are not arbitrary scalar truthiness; no missing range metadata is fabricated. |
| [63472](https://github.com/llvm/llvm-project/issues/63472) | covered-limited | Constant-choice scalar binop with a same-condition zero/one Boolean folds both constant outcomes; sext(i1) and cross-width signed-mask variants are not claimed. |
| [63756](https://github.com/llvm/llvm-project/issues/63756) | reviewed-deferred | Sinking a shared select across branch/call boundaries needs CFG dominance and effect checks; local consumer substitution does not relocate definitions. |
| [64583](https://github.com/llvm/llvm-project/issues/64583) | reviewed-deferred | Implication across several ordered comparisons needs a relational interval/constraint solver, not merely exact predicate complement matching. |
| [64884](https://github.com/llvm/llvm-project/issues/64884) | reviewed-deferred | Floating copysign/select changes must preserve signed zero, NaNs and FP environment contracts; scalar integer substitution refuses FP carriers. |
| [66101](https://github.com/llvm/llvm-project/issues/66101) | reviewed-deferred | The motivating vector blend/sub mask is target instruction selection; no vector-mask/all-ones policy is inferred from scalar zero/one Booleans. |
| [69123](https://github.com/llvm/llvm-project/issues/69123) | reviewed-deferred | The logical signbit/nonzero conjunction requires CFG/interval-to-signed-comparison formation and cannot be replaced by generic bitwise NOT. |
| [71792](https://github.com/llvm/llvm-project/issues/71792) | reviewed-deferred | Boolean three-input select algebra needs a dedicated zero/one decision table; per-bit word algebra and arbitrary nonzero truthiness are not interchangeable. |
| [72651](https://github.com/llvm/llvm-project/issues/72651) | reviewed-deferred | Select predicate canonicalization for cross-block CSE needs dominance-aware value numbering; flipping every predicate has no guaranteed target benefit. |
| [73904](https://github.com/llvm/llvm-project/issues/73904) | covered-limited | Commutative Add/Mul/And/Or/Xor of symmetric correlated selects becomes the corresponding operation on the original arms, without mutating shared selects. |
| [75004](https://github.com/llvm/llvm-project/issues/75004) | reviewed-deferred | Changing short-circuit block layout requires verifying side effects and duplicating neither calls nor loads; no CFG transform is introduced. |
| [75784](https://github.com/llvm/llvm-project/issues/75784) | reviewed-deferred | The two-inequality condition needs disjunctive path facts; independent known bits cannot prove the selected value is always one of two symbolic values. |
| [79266](https://github.com/llvm/llvm-project/issues/79266) | reviewed-deferred | The original uses LLVM or-disjoint. Without explicit disjoint-bit proof, moving XOR through OR is false on overlapping bits. |
| [81288](https://github.com/llvm/llvm-project/issues/81288) | reviewed-deferred | The original vector sext masks are all-ones and shared; factoring across vector arithmetic needs mask provenance and target-cost checks. |
| [82350](https://github.com/llvm/llvm-project/issues/82350) | reviewed-deferred | Factoring independent predicates into a shared Boolean select is a decision-tree/cost rewrite, not same-condition leaf substitution. |
| [83225](https://github.com/llvm/llvm-project/issues/83225) | reviewed-deferred | Deep correlated decision trees would require rebuilding a shared intermediate select. No shared producer is mutated or cloned to force this reduction. |
| [85833](https://github.com/llvm/llvm-project/issues/85833) | reviewed-deferred | The motivating result is a pointer. Scalar integer substitution deliberately does not invent pointer null/provenance semantics. |
| [86176](https://github.com/llvm/llvm-project/issues/86176) | covered-limited | Scalar constant-choice binop with its canonical Boolean predicate is evaluated per outcome; no variable operation or effect is cloned. |
| [88107](https://github.com/llvm/llvm-project/issues/88107) | reviewed-deferred | The report is interprocedural argument specialization and dead argument removal; local facts cannot substitute across arbitrary calls. |
| [88481](https://github.com/llvm/llvm-project/issues/88481) | reviewed-deferred | Two independent composite Boolean predicates require decision-tree rebuilding with sharing costs; no unbounded BDD search is added. |
| [95994](https://github.com/llvm/llvm-project/issues/95994) | reviewed-deferred | The source contains undef on a PHI edge and a conditional load; preserving both edge semantics and non-speculation requires CFG/PHI work. |
| [96765](https://github.com/llvm/llvm-project/issues/96765) | reviewed-deferred | trunc nuw to i1 relies on a no-wrap flag and i1 type semantics absent from LCCC scalar casts; arbitrary truncation does not imply x in {0,1}. |
| [97044](https://github.com/llvm/llvm-project/issues/97044) | reviewed-deferred | The motivating networks overlap the specialized mux/distributive policy protected after CH/MAJ regressions. A general smaller IR recipe is not automatically a better target shape. |
| [98800](https://github.com/llvm/llvm-project/issues/98800) | covered-limited | A select of two private opposite same-condition scalar selects becomes XOR of canonical predicates and one select; shared choices and noncanonical predicate words refuse. |
| [102355](https://github.com/llvm/llvm-project/issues/102355) | reviewed-deferred | Two disjoint interval unions require range-set costing; no convex-hull replacement is sound across the missing interior point. |
| [104772](https://github.com/llvm/llvm-project/issues/104772) | reviewed-deferred | The issue body only links external reductions. Those linked examples were not fetched; no implementation or equivalence is claimed from the title alone. |
| [110919](https://github.com/llvm/llvm-project/issues/110919) | reviewed-deferred | Equality-of-masked-parts needs consistent whole-word versus trunc-i1 canonicalization and Boolean observation proofs; per-bit NOT is not Boolean NOT. |
| [118108](https://github.com/llvm/llvm-project/issues/118108) | reviewed-deferred | A lower-bound assumption implies a disjunction of high bits, not a fixed known-one bit. Independent known bits cannot represent it. |
| [167173](https://github.com/llvm/llvm-project/issues/167173) | reviewed-deferred | Parity-select arithmetic may fold to XOR only with matching mask, orientation and wrapping/overflow contracts; general subtraction factoring is not introduced here. |
| [172888](https://github.com/llvm/llvm-project/issues/172888) | reviewed-deferred | LCCC has no scalar i1 truncation carrier. Emitting an ordinary integer trunc instead of a canonical comparison would change truth values. |
| [174937](https://github.com/llvm/llvm-project/issues/174937) | reviewed-deferred | The mixed eager/logical Boolean mux must preserve zero/one canonicalization and specialized ANDN policy; it is not generic word-mask synthesis. |
| [180492](https://github.com/llvm/llvm-project/issues/180492) | reviewed-deferred | Shift of two zero/one extensions can fold only after a complete valid-count and Boolean provenance proof; noncanonical word conditions would be unsound. |
| [183584](https://github.com/llvm/llvm-project/issues/183584) | reviewed-deferred | Known one-bit range is useful for facts but does not authorize a non-existent i1 trunc carrier. Keep canonical zero/one comparisons. |
| [186509](https://github.com/llvm/llvm-project/issues/186509) | reviewed-deferred | The motivating concatenated Boolean observation requires a bit-position/cardinality solver; FP comparison leaves stay opaque and no FP rewrite is claimed. |
| [186558](https://github.com/llvm/llvm-project/issues/186558) | reviewed-deferred | FP-select comparison elimination requires NaN and signed-zero-aware constant equality; this cohort admits only integer select carriers. |
| [192094](https://github.com/llvm/llvm-project/issues/192094) | reviewed-deferred | The motivating report is vector correlated propagation. The scalar control is exercised, but scalar range-arm simplification is not vector issue closure. |
| [202066](https://github.com/llvm/llvm-project/issues/202066) | covered-limited | Same-condition scalar-select comparisons with provably constant/equal-arm outcomes fold to a Boolean constant or choice. Arbitrary assume-based variable ordering is not modeled. |
| [202576](https://github.com/llvm/llvm-project/issues/202576) | reviewed-deferred | Unsigned min with all-ones sentinel needs a min/max-specific identity; signed min and non-sentinel constants are not generalized. |
| [205005](https://github.com/llvm/llvm-project/pull/205005) | reviewed-deferred | Min/max predicate ordering in the constraint solver is a distinct mechanism and vectorization proof; no intrinsic min/max representation is added. |
| [223249](https://github.com/llvm/llvm-project/issues/223249) | reviewed-deferred | Self-dependent select equality to finite-set membership can expand one comparison into two; legality and target cost must both be justified. |
| [232620](https://github.com/llvm/llvm-project/issues/232620) | reviewed-deferred | The motivating pointer GEP/select requires pointer provenance and offset factoring; direct scalar same-condition nested selects are tested but do not close the pointer report. |

### Prior deferrals — no omitted entries

| Report | Decision | Scoped work / remaining blocker |
|---|---|---|
| [1884](https://github.com/llvm/llvm-project/issues/1884) | reviewed-deferred | Allocator spill placement/register-pressure objective; requires executable reproduction of the old SSE graph and allocator evidence, not scalar identity rewriting. |
| [3007](https://github.com/llvm/llvm-project/issues/3007) | reviewed-deferred | Loop AddRec strength reduction needs induction-variable legality, overflow and register-pressure costing; attached IL is not a local scalar contract. |
| [6509](https://github.com/llvm/llvm-project/issues/6509) | reviewed-deferred | Cross-register-file value placement is backend/ABI policy; bitcast identities alone do not prove eliminating XMM/GPR moves is profitable. |
| [10156](https://github.com/llvm/llvm-project/issues/10156) | reviewed-deferred | Immediate-compare encoding alternatives require dead-register and exact flags liveness proofs; do not replace comparisons by flag-different INC/DEC globally. |
| [13134](https://github.com/llvm/llvm-project/issues/13134) | reviewed-deferred | The original C++ byte-swapping loop requires memory/alias and loop recognition; the existing SSA byte network matcher does not close it. |
| [13687](https://github.com/llvm/llvm-project/issues/13687) | reviewed-deferred | Machine tail merging is CFG/exception/layout work, not a consumer-local equality; old attached testcase needs a dedicated backend experiment. |
| [35970](https://github.com/llvm/llvm-project/issues/35970) | reviewed-deferred | Mixed-width XOR cancellation requires cast-aware demanded-bit reconstruction; no cast/shift movement is added to this consumer-local change. |
| [38139](https://github.com/llvm/llvm-project/issues/38139) | covered-limited | Contiguous high OR-mask and low AND-mask equality against the original operand become a single unsigned bound; sparse masks refuse. |
| [38926](https://github.com/llvm/llvm-project/issues/38926) | reviewed-deferred | Store/truncation commoning requires cross-width dominance-aware CSE; retain both stores and all observable memory accesses. |
| [41333](https://github.com/llvm/llvm-project/issues/41333) | reviewed-deferred | Total CLZ of a shifted power-of-two must saturate at the width; an unconditional affine result is false after the bit shifts out. |
| [41757](https://github.com/llvm/llvm-project/issues/41757) | reviewed-deferred | Variable-width sign extension needs len bounds and exact signed shift semantics; len=0 and full-width lengths require separate cases. |
| [41801](https://github.com/llvm/llvm-project/issues/41801) | reviewed-deferred | Dynamic mask elimination before a shift must relate its count and demanded bits; arbitrary mask/count independence is insufficient. |
| [41908](https://github.com/llvm/llvm-project/issues/41908) | reviewed-deferred | The original six forms include truncation and count-sum constraints; no common blanket mask-elimination rule is valid. |
| [42219](https://github.com/llvm/llvm-project/issues/42219) | reviewed-deferred | Multiply/rotate scheduling and target multiply-high selection need per-target costing; do not reassociate through truncation without proof. |
| [45243](https://github.com/llvm/llvm-project/issues/45243) | reviewed-deferred | Conditional rotate formation trades shared shifts against a rotate and select; existing target recognition is deliberately preserved, not replaced by generic select factoring. |
| [45478](https://github.com/llvm/llvm-project/issues/45478) | reviewed-deferred | The source returns a canonical Boolean, whereas ~x is a word mask; converting through CFG/bit-zero demand needs its own observation proof. |
| [46405](https://github.com/llvm/llvm-project/issues/46405) | reviewed-deferred | Short-circuit Boolean CFG factoring cannot speculate calls or loads. The eager bitwise counterpart is not a whole-report resolution. |
| [47868](https://github.com/llvm/llvm-project/issues/47868) | reviewed-deferred | Signed max/sub/mask reduction must preserve negative inputs and wrapping boundaries; unsigned modular-remainder recognition alone is insufficient. |
| [47946](https://github.com/llvm/llvm-project/issues/47946) | reviewed-deferred | Switch-domain/complement case coverage requires CFG successor and bitfield-range reasoning; local select substitution does not rewrite switches. |
| [49562](https://github.com/llvm/llvm-project/issues/49562) | reviewed-deferred | Two narrow popcounts to one wide count is legalization/target-cost work; software POPCNT fallback and cross-width result casts need independent evidence. |
| [51090](https://github.com/llvm/llvm-project/issues/51090) | reviewed-deferred | The motivating 32-bit word with 64-y shift counts has no common fully defined C shift domain; do not use poison to infer modular rotate semantics. |
| [54856](https://github.com/llvm/llvm-project/issues/54856) | covered-limited | Conjunction of x < 2^k and a contiguous top-aligned mask != mask becomes x < mask; sparse, misaligned and missing-bound variants refuse. |
| [55016](https://github.com/llvm/llvm-project/issues/55016) | reviewed-deferred | Offset-shift reassociation requires a no-wrap count bound and exact/signed-shift contracts that are not represented by the current scalar matcher. |
| [55468](https://github.com/llvm/llvm-project/issues/55468) | reviewed-deferred | Rejected backward inference from shift poison/undefined behavior: a speculated poison-producing shift is not an unconditional path assumption. |
| [57666](https://github.com/llvm/llvm-project/issues/57666) | covered-limited | Zero observation of AND of identical disjoint nonzero constant-choice selects compares their canonical Boolean predicates; shared producers are retained. |
| [58624](https://github.com/llvm/llvm-project/issues/58624) | reviewed-deferred | XOR to OR requires a dominating disjointness/NAND relation; for x=y=1 XOR=0 but OR=1. Known-one bits do not express arbitrary disjointness. |
| [59555](https://github.com/llvm/llvm-project/issues/59555) | reviewed-deferred | NaN-tag interval coalescing needs an unsigned interval union solver. Treating tag bits as floating-point values would be unsound. |
| [59773](https://github.com/llvm/llvm-project/issues/59773) | reviewed-deferred | LLVM sext(i1) is an all-ones mask, unlike LCCC zero/one Boolean casts. A mask-to-select rule needs explicit mask provenance and target costs. |
| [61671](https://github.com/llvm/llvm-project/issues/61671) | reviewed-deferred | Rejected unconditional CLZ/XOR rewrite at zero. The total count zero result differs from the nonzero contract. |
| [62514](https://github.com/llvm/llvm-project/issues/62514) | reviewed-deferred | The discussion reports a GCC correctness bug in collapsing a select/bswap shape; zero-handling and demanded-byte proofs are mandatory. |
| [62898](https://github.com/llvm/llvm-project/issues/62898) | reviewed-deferred | Signed min/max sign observations require correlated order predicates; the source repeats the smin body as smax, so its spelling cannot be treated as an independent proof. |
| [63749](https://github.com/llvm/llvm-project/issues/63749) | covered-limited | Explicit x != MAX guard plus wrapping x+1 <=u y becomes x <u y; no unguarded increment reassociation. |
| [63848](https://github.com/llvm/llvm-project/issues/63848) | reviewed-deferred | Splitting 64-bit variable shifts into 32-bit halves is target legalization and code-size/latency work; quarter-rate target costs are not generic scalar costs. |
| [64238](https://github.com/llvm/llvm-project/issues/64238) | reviewed-deferred | Constant-global table load bounds require alias-aware memory and index-domain analysis; no load is evaluated or moved here. |
| [64859](https://github.com/llvm/llvm-project/issues/64859) | covered-limited | Equality-to-one of the sum of two canonical zero/one Boolean extensions becomes XOR; negated/all-ones masks refuse. |
| [69333](https://github.com/llvm/llvm-project/issues/69333) | covered-limited | Unsigned (x/z) < (x >> count) is false only with an explicitly proved valid count; unknown counts and signed predicates refuse. |
| [73417](https://github.com/llvm/llvm-project/issues/73417) | reviewed-deferred | The Rust tuple form relies on sext(i1)=-1 versus zext(i1)=1 and cross-width comparisons; zero/one Boolean cardinality is not a signed-mask solver. |
| [78038](https://github.com/llvm/llvm-project/issues/78038) | covered-limited | Eq/Ne of symmetric same-condition scalar selects becomes Eq/Ne of their arms; intrinsic min/max and relational min/max predicates are not claimed. |
| [79696](https://github.com/llvm/llvm-project/issues/79696) | reviewed-deferred | CLZ/XOR value ranges need non-convex interval analysis, not independent per-bit facts. No accidental bounds are inferred from builtin C undefined zero. |
| [79823](https://github.com/llvm/llvm-project/issues/79823) | reviewed-deferred | Software narrow popcount multiplication needs a target fallback cost model and exact truncation proof; native POPCNT is not replaced blindly. |
| [80100](https://github.com/llvm/llvm-project/issues/80100) | reviewed-deferred | Sparse-mask POPCNT/CTZ expansions require target latency and an explicit zero policy; hardware scan instructions are not globally expanded. |
| [82075](https://github.com/llvm/llvm-project/issues/82075) | reviewed-deferred | Narrow nonzero CLZ promotion is target legalization with load range metadata; this change does not invent metadata on arbitrary narrow loads. |
| [84038](https://github.com/llvm/llvm-project/issues/84038) | reviewed-deferred | Recovering AShr from sign-conditioned LShr/NOT requires common-count and signed-predicate correlation, beyond a same-condition leaf substitution. |
| [84656](https://github.com/llvm/llvm-project/issues/84656) | reviewed-deferred | Scan-loop recognition needs PHI/exit/zero-case and count-range proofs; consumer-only rewrites intentionally do not edit loops. |
| [84763](https://github.com/llvm/llvm-project/issues/84763) | reviewed-deferred | Multiply-lowbit versus CTZ/shift is target dependent; y=0 forbids introducing a nonzero scan without an explicit guard. |
| [85823](https://github.com/llvm/llvm-project/issues/85823) | covered-limited | Nonzero observation of a power-of-two masked value supplies the known-one bit to the existing unique-predecessor facts engine; multi-bit masks do not imply individual known ones. |
| [85939](https://github.com/llvm/llvm-project/issues/85939) | reviewed-deferred | Cross-width right-shift comparison needs valid counts for both widths plus extension semantics; sign extension is not zero extension. |
| [86874](https://github.com/llvm/llvm-project/issues/86874) | covered-limited | Low-contiguous-mask minus x becomes XOR only when existing known bits prove x fits that mask; unbounded x and implicit LLVM nuw refuse. |
| [96455](https://github.com/llvm/llvm-project/issues/96455) | reviewed-deferred | Narrow leading-ones promotion needs baseline BSR/LZCNT legality and zero behavior; preserve current scan selection until target-specific evidence exists. |
| [96690](https://github.com/llvm/llvm-project/issues/96690) | reviewed-deferred | Unicode XOR/sub interval coalescing is target-cost sensitive, including AArch64 conditional compare; no architecture-independent speed claim. |
| [98848](https://github.com/llvm/llvm-project/issues/98848) | covered-limited | Scalar select-arm substitution removes addition of an explicitly zero false-arm value; constant-shift known bits can prove a nonzero top-bit extraction equals one. Computed-goto restructuring is not included. |
| [102946](https://github.com/llvm/llvm-project/issues/102946) | reviewed-deferred | Pre-shifting a constant requires count+1 to stay below width without wrapping; masked k alone is not sufficient at k=width-1. |
| [108559](https://github.com/llvm/llvm-project/issues/108559) | reviewed-deferred | Exact paired divisions cannot cancel for arbitrary wrapping integers: distinct numerators may have the same quotient. Exactness/divisibility metadata is absent. |
| [150954](https://github.com/llvm/llvm-project/issues/150954) | reviewed-deferred | CLZ XOR31 plus one crosses a narrowing cast and nonzero builtin contract; baseline BSR behavior at zero cannot be assumed defined. |
| [213877](https://github.com/llvm/llvm-project/issues/213877) | reviewed-deferred | Odd multiplication preserves CTZ/nonzero, but path nonzero facts for arbitrary words and scan legalization remain separate from bitwise known-one inference. |
| [219334](https://github.com/llvm/llvm-project/issues/219334) | reviewed-deferred | General bitreverse conjugation through shared bitwise DAGs needs a bounded graph-substitution and sharing-cost proof; matching one opposite shift does not cover the source networks. |
