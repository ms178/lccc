# Scalar logic and bounded observations

## Scope

This cohort reviews **50 distinct previously unreviewed LLVM reports** selected from the existing inventory for scalar bitwise, shift, count and integer-observation relevance. It implements seven reduced report scopes through three bounded mechanisms; the other 43 reports remain explicit deferrals. This is not fifty new optimization families or fifty whole issues solved.

All issue bodies and accessible issue-conversation comments were retrieved through the GitHub REST API. `scalar-logic-sources.json` records source identities, counts and dispositions. Ignored raw records live in `work/scalar-logic/`. External linked programs, proofs and pages were not executed or treated as trusted instructions.

The dispositions below are historical receipts for this cohort. Current follow-ups, including ten limited advances from prior deferrals, are recorded in [predicate relations](PREDICATE-RELATIONS.md) and the cumulative ledger.

## Per-report disposition

| Report | Reviewed scope / decision |
|---|---|
| [35970](https://github.com/llvm/llvm-project/issues/35970) | Deferred: the motivating cancellation crosses zext, shifts and two widths; opaque leaves intentionally do not move casts through shifts. |
| [38139](https://github.com/llvm/llvm-project/issues/38139) | Deferred: OR-mask equality versus its source needs a range/comparison fold; per-bit synthesis does not reinterpret an integer comparison as bitwise logic. |
| [38926](https://github.com/llvm/llvm-project/issues/38926) | Deferred: reuse of a masked value across stores and truncation needs dominance-aware cross-width CSE, not producer-network synthesis. |
| [41333](https://github.com/llvm/llvm-project/issues/41333) | Deferred: total CLZ after shifting a constant saturates at the width. The discussion distinguishes nonzero, exact and no-wrap variants; no flag is invented. |
| [41757](https://github.com/llvm/llvm-project/issues/41757) | Deferred: variable-width conditional sign extension needs bounds on len and a separate sign-extension recognizer; signed C shifts also need defined witnesses. |
| [41801](https://github.com/llvm/llvm-project/issues/41801) | Deferred: removing a dynamic mask before arithmetic/right shift requires demanded-bit reasoning tied to the same count; shifts remain opaque here. |
| [41908](https://github.com/llvm/llvm-project/issues/41908) | Deferred: the six upstream mask/shift forms include truncation and count-sum constraints; no blanket mask removal is admitted. |
| [42219](https://github.com/llvm/llvm-project/issues/42219) | Deferred: multiply/rotate ordering is a pipeline and target-cost issue, not a bitwise-network identity; retain existing rotate recognition. |
| [44992](https://github.com/llvm/llvm-project/issues/44992) | Implemented limited: private same-width (x xor C)+1 combined with wrapping add/sub is replaced by the complemented-constant xor and opposite add/sub. |
| [45478](https://github.com/llvm/llvm-project/issues/45478) | Deferred: narrowing a Boolean observation to bit zero and CFG range-set folding need distinct observation/control-flow proofs. |
| [45820](https://github.com/llvm/llvm-project/issues/45820) | Implemented limited: the closed bitwise DAG ((a&b)^b) / ((a&b)^a) synthesizes XOR, accounting for the internally shared AND once. Outside users refuse. |
| [46405](https://github.com/llvm/llvm-project/issues/46405) | Deferred: the motivating short-circuit expression needs CFG/Boolean conversion reasoning; its bitwise analogue is not claimed as closure of the report. |
| [47868](https://github.com/llvm/llvm-project/issues/47868) | Deferred: signed max, subtraction and a mask require signed range/select reasoning, including negative inputs; not a pure Boolean identity. |
| [47946](https://github.com/llvm/llvm-project/issues/47946) | Deferred: common-destination switch and bitfield-mask simplification require switch-domain reasoning rather than a local bitwise root. |
| [54856](https://github.com/llvm/llvm-project/issues/54856) | Deferred: combining a power-of-two unsigned bound with a contiguous mask comparison needs range-interval proofs; sparse masks cannot be generalized. |
| [55016](https://github.com/llvm/llvm-project/issues/55016) | Deferred: shifting a negative constant by an offset depends on exact-shift and count bounds. Those LLVM promises are absent from LCCC IR. |
| [55468](https://github.com/llvm/llvm-project/issues/55468) | Deferred: never infer a global range backward from a C shift. The discussion warns that shifts can be speculated and yield LLVM poison. |
| [55739](https://github.com/llvm/llvm-project/issues/55739) | Implemented limited: comparisons of a masked shifted power of two to zero reduce to count equality/inequality when an explicit scalar AND proves the count is in range, including value-preserving bounded count casts. |
| [56124](https://github.com/llvm/llvm-project/issues/56124) | Implemented limited: a left-shifted power of two observed through a low contiguous mask reduces to a count threshold under that explicit bound. |
| [57666](https://github.com/llvm/llvm-project/issues/57666) | Deferred: AND of two constant-choice selects observed as zero needs relational reasoning across selects; selects and comparisons stay opaque. |
| [58313](https://github.com/llvm/llvm-project/issues/58313) | Implemented limited: the eager scalar bitwise XOR networks in the discussion are synthesized exactly. Short-circuit CFG restructuring is not included. |
| [58624](https://github.com/llvm/llvm-project/issues/58624) | Deferred: weakening XOR to OR is not an unconditional Boolean identity; it requires a dominating NAND assumption and relational facts. |
| [59555](https://github.com/llvm/llvm-project/issues/59555) | Deferred: NaN-tag range coalescing is integer interval reasoning across two comparisons, not FP arithmetic or a pure bitwise replacement. |
| [59773](https://github.com/llvm/llvm-project/issues/59773) | Deferred: multiply-used sext/not to select depends on Boolean provenance and sharing costs. No cast or select is synthesized here. |
| [61120](https://github.com/llvm/llvm-project/issues/61120) | Implemented limited: comparisons of two private same-width bitwise NOT values reverse operand order, preserving the original signed/unsigned predicate. |
| [61671](https://github.com/llvm/llvm-project/issues/61671) | Deferred: the proposed total CLZ rewrite is invalid at zero, as the discussion corrects. A nonzero precondition must be proved separately. |
| [62898](https://github.com/llvm/llvm-project/issues/62898) | Deferred: signed min/max sign observations and logical versus bitwise forms require select/CFG correlation. The source also repeats the smin body as smax. |
| [63749](https://github.com/llvm/llvm-project/issues/63749) | Deferred: folding the guarded x+1 bound needs relational carry/unsigned interval reasoning; no no-wrap condition is inferred from a Boolean graph. |
| [63848](https://github.com/llvm/llvm-project/issues/63848) | Deferred: 64-to-32 shift splitting depends on target shift costs, count ranges and legalization; three-leaf scalar logic does not narrow shifts. |
| [64238](https://github.com/llvm/llvm-project/issues/64238) | Deferred: proving a constant-table load range needs memory/constant-global analysis with a dominating index bound. |
| [64859](https://github.com/llvm/llvm-project/issues/64859) | Deferred: equal-to-one observations of added Boolean extensions require arithmetic Boolean-cardinality reasoning; new bitwise synthesis treats extensions as opaque. |
| [69333](https://github.com/llvm/llvm-project/issues/69333) | Deferred: comparing an OR against a logical shift needs cross-operation unsigned bounds. The issue discussion also corrects an initially over-reduced example. |
| [73417](https://github.com/llvm/llvm-project/issues/73417) | Deferred: tuple ordering combines sext/zext arithmetic, comparisons and selects; the canonical Boolean comparison must be recovered before any bitwise synthesis. |
| [78038](https://github.com/llvm/llvm-project/issues/78038) | Deferred: equality of min/max or symmetric selects needs correlated select predicates, widths and signedness. This change does not assume independent selects are min/max. |
| [79696](https://github.com/llvm/llvm-project/issues/79696) | Deferred: precise XOR constant ranges after CLZ need interval analysis beyond the existing known-bit representation. |
| [79823](https://github.com/llvm/llvm-project/issues/79823) | Deferred: narrow software popcount multiplication/mask formulas need per-target size/latency assessment against native POPCNT and the existing fallback. |
| [80100](https://github.com/llvm/llvm-project/issues/80100) | Deferred: sparse-mask count lowering requires target-specific popcount/ctz cost and zero-policy handling; no hardware intrinsic is expanded into multiplication here. |
| [82075](https://github.com/llvm/llvm-project/issues/82075) | Deferred: nonzero narrow CLZ promotion belongs in target legalization; target-independent logic does not add shifts/casts around scans. |
| [83699](https://github.com/llvm/llvm-project/issues/83699) | Implemented limited: closed mask-extract/update/reinsert DAGs synthesize shorter AND/OR/XOR forms, including complementary literal masks. Arbitrary externally shared graphs refuse. |
| [84038](https://github.com/llvm/llvm-project/issues/84038) | Deferred: sign-conditioned logical shifts emulate AShr, but recovering that requires select, signed comparison and shared-count proofs. |
| [84656](https://github.com/llvm/llvm-project/issues/84656) | Deferred: replacing a scan loop by total CTZ needs loop-exit/PHI/zero-case reasoning; no loop or memory transformations are added. |
| [84763](https://github.com/llvm/llvm-project/issues/84763) | Deferred: low-bit multiply versus CTZ/shift is target-cost sensitive and needs an explicit zero policy. Do not introduce a scan on software-scan targets. |
| [85823](https://github.com/llvm/llvm-project/issues/85823) | Deferred: the example needs a relational nonzero-mask fact across a branch/call. No new closure is claimed for the preexisting facts pass. |
| [85939](https://github.com/llvm/llvm-project/issues/85939) | Deferred: cross-width right shifts and casts need a count-range proof for both widths plus sign-extension preservation. |
| [86874](https://github.com/llvm/llvm-project/issues/86874) | Deferred: 15-x to xor needs x<=15 or LLVM nuw. Generic wrapping subtraction is not the same identity; target instruction-selection costs also matter. |
| [96455](https://github.com/llvm/llvm-project/issues/96455) | Deferred: total narrow leading-ones promotion and BSR/LZCNT speculation differ by target and zero policy; preserve the current count lowering. |
| [96690](https://github.com/llvm/llvm-project/issues/96690) | Deferred: Unicode range coalescing through XOR/subtract is not uniformly profitable (discussion identifies AArch64 conditional compare); requires a range cost model. |
| [98848](https://github.com/llvm/llvm-project/issues/98848) | Deferred: simplifying the ternary sign observation needs substituting condition facts into select arms, not eager bitwise evaluation of a control-flow graph. |
| [102946](https://github.com/llvm/llvm-project/issues/102946) | Deferred: shift-offset folding must prove the adjusted count cannot wrap or reach the word width; do not assume Zig intCast/LLVM nuw facts exist in LCCC. |
| [108559](https://github.com/llvm/llvm-project/issues/108559) | Deferred: dropping paired divisions from comparisons requires exactness/divisibility, not just a common divisor. LCCC does not represent the reported exact flag. |

## Mechanisms and boundaries

`bit_idioms_logic.rs` synthesizes AND/OR/XOR/NOT functions over at most three
same-width scalar word leaves. Its eight-bit table is the exact per-bit truth
function, not a Boolean approximation of whole-word truthiness. Zero/all-ones
are constants; complementary literal masks share a column. The deterministic
recipe search is initialized once, permits at most three operations, and only
accepts a strict reduction in the closed producer DAG's unique operation count.
A 32-visit walk bounds matching. Internal shared edges are counted once per
edge, while unique producer instructions are charged once. External producer
users refuse the fold. Opaque inputs are existing SSA values: loads, arithmetic,
shifts, casts, selects and comparisons are never cloned, speculated or moved.

The scalar-facts owner's use accounting now includes both canonical instruction
visitors (ordinary operands and bare value uses), plus terminators. This repairs
a preexisting gap for pointer/intrinsic/memcpy uses before relying on a closed
producer proof. Replacements preserve the owner's fresh-ID, source-span and
SSA policy; the existing `bit_idioms` disable switch disables the new work too.
No new environment knob, unsafe block, production panic or census allowance
is introduced.

`bit_idioms_complements.rs` uses wrapping identities, not LLVM `nsw`/`nuw`:
private `(x ^ C) + 1 + y` becomes `y - (x ^ ~C)`; subtracting that offset uses
addition instead. Comparing two private bitwise complements reverses the
operand order while preserving signedness and the comparison predicate.

`bit_idioms_shift_observe.rs` observes a shifted power of two through a
single-bit mask, or a left shift through a low contiguous mask. Its count must
already be AND-bounded below the word width. The IR lowerer independently
normalizes shift counts to I32, so bounded count casts/copies are followed only
to prove the bound survives. The replacement compares the unchanged original
count operand in its actual scalar type; it does not pretend an I32 count is U64. It does not infer a shift range from C undefined behavior, invent LLVM
poison/no-wrap guarantees, normalize an unbounded count, or admit AShr.
Shared producers, non-power bases, sparse masks and unknown counts refuse.

## Contracts and evidence policy

The Rust tests exercise every admitted synthesized truth function, all eight
bit valuations, deterministic random full words and eight signed/unsigned
8/16/32/64-bit types. They require at least fifty distinct admitted truth
functions but count this as one synthesis mechanism, not fifty pattern families.
Other tests exhaust all bit positions/counts in the bounded shift contracts,
byte-ring complement arithmetic, signed/unsigned complement comparisons,
internal/external sharing, complementary masks, equal costs and exhausted walk
or leaf budgets.

`llvm_scalar_logic.c` adds **25 named total contracts at 32 and 64 bits: 50
native forms**. Each compares against an independent reduced expression over
edge and deterministic random inputs. These are reduced contracts from the
implemented mechanisms, not fifty distinct newly solved reports.
`check_llvm_scalar_logic.py` runs them against GCC 16, and Clang 23.1.3 when
that oracle is supplied, at O0/O1/O2/O3/Os, debug, x86-64-v3 and i386 with
IR/SSA/register-allocation verification. It also executes the disabled-pass
control and checks that bounded observations do not retain shift instructions.
The frontend CI lane supplies the pinned Clang oracle; the backend lane and
local-CI mirror run the same gate against GCC. The general regression corpus
also discovers the fixture.

Instruction inventories compare against the disabled *entire* bit-idiom pass;
that is not an immutable pre-change compiler, so it is not attributed solely
to this cohort. The optional `--baseline` argument records an explicitly supplied
before compiler separately. No new timing, hardware-independent optimality,
or measured runtime speedup is claimed. GitHub CI is responsible for validation;
source collection and formatting are not pass receipts.

### Specialized Boolean policy

Shared/complemented-selector pairs of ANDs under OR/XOR remain opaque to the
new synthesizer. The established CH/distributive matcher owns those shapes:
its BMI1 ANDN deferral and majority-prefix deferral include target and consumer
cost information that a target-neutral operation count cannot replace. An
outer synthesis may read such a value, but never absorbs or rewrites its
internal network. The existing CH/MAJ codegen gate remains unchanged in its
runtime, ANDN census, register-only and store-forward requirements; failures
are additionally exposed as API-readable annotations.
