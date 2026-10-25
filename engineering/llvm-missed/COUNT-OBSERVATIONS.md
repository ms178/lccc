# Consumer-local count observations

## Source audit, not issue-count accounting

Eight original LLVM issue bodies and all accessible conversation comments were
read through the paginated GitHub REST API. `count-observations-sources.json`
identifies the exact archived issue-plus-comments records, collection time and
comment counters. Raw records remain in ignored
`work/llvm-missed/count-observations/`. External Compiler Explorer links and
inaccessible history are not additional audited evidence.

| Original report | Contract, discussion and disposition |
|---|---|
| [210673](https://github.com/llvm/llvm-project/issues/210673) | SimplifyCFG speculates the count out of a branch, losing CVP's useful edge. The discussion warns that delaying speculation can hurt vectorization. Implement scalar consumer-local count bounds instead of rearranging the pipeline: e.g. `x<256 ? popcount(x)<=8 : 1` becomes one. Inverse/swapped unsigned guards and CLZ/CTZ known-bit bounds are included. Arbitrary CFG dominance and vector facts are not. |
| [115751](https://github.com/llvm/llvm-project/issues/115751) | The Rust power-of-two guard leaves redundant division/remainder zero checks; discussion suggests LVI/CVP. Implement only the exact scalar preimage: an existing population count known positive implies its *same SSA source and word type* is nonzero, and an exactly-zero count implies zero. The defined C division/remainder controls remove unreachable zero-sentinel arms. This is not Rust panic lowering or general power-of-two division strength reduction. |
| [47250](https://github.com/llvm/llvm-project/issues/47250) | The count-equals-one branch shape has a backend instruction/branch-cost issue. Later comments show changed LLVM code and reference x86 ispow2 tests. A smaller scalar proof does not certify that branch choice, POPCNT profitability or all targets. Deferred; count-equals-one controls run with and without POPCNT. |
| [83772](https://github.com/llvm/llvm-project/issues/83772) | QEMU-style power-of-two CFG canonicalization depends on count cost; comments distinguish native POPCNT/CSSC from software expansion and the zero-or-single-bit variant. The explicit CFG control is an oracle check, not a newly implemented target-cost policy. Deferred. |
| [76810](https://github.com/llvm/llvm-project/issues/76810) | Parity can be expressed using an isolated low bit and alternating mask (CTZ), or ordered even/odd partitions (CLZ). Total zero counts are even at the admitted widths. Materializing a wide mask or replacing fast scans can lose. Defined-zero parity controls retained; target-aware profitability and this transformation family deferred. |
| [131444](https://github.com/llvm/llvm-project/issues/131444) | LLVM can mark the shift by trailing-zero count exact. At zero a total count produces a width-sized shift and LLVM poison, unlike C promotions or x86 masked shifts. Do not import exact/poison assumptions. The fixture explicitly guards zero before shifting; it is a semantic control, not proof of the original poison-sensitive fold. Deferred. |
| [213877](https://github.com/llvm/llvm-project/issues/213877) | Odd multiplication preserves trailing-zero count, but the report is specifically a false-to-true nonzero upgrade lost through selection. Existing odd-product recognition is not a new implementation of arbitrary nonzero path facts. Native odd-product controls supplement the earlier deferred review without changing its disposition. |
| [231124](https://github.com/llvm/llvm-project/issues/231124) | Split half scans, CFG/PHI thresholds and a custom zero sentinel; original IR also has a misspelled result. The conversation narrows proposed work to select form rather than PHI thresholds. A zero result of 32 must not become the natural wide count's 64. Preserve that sentinel in an independent native control; whole family deferred. |

Seven previously unreviewed reports join the ledger; the eighth is a follow-up
on an existing deferral. Totals become **144 reviewed scopes: 52 limited and 92
deferred**, leaving **2,967 unreviewed inventory rows**. Neither upstream closure
nor a passing control is credited as a new covered scope.

## Legality and implementation

`bit_idioms_count_observe.rs` shares the existing scalar-facts context, depth-8
walks, 512-definition visit budget and exact `bit_idioms` disable policy. Only
scalar integer words/carriers of 8/16/32/64 bits qualify. Counts are in 0..64,
so even signed-byte casts preserve them. Critically, the unary instruction type
is the **input word**; a frontend builtin may expose an I32 result for a U64
input without emitting a cast. Treating these as one type silently misses the
real C mechanism. Explicit casts/copies are bounded; floating casts are refused.

Known-zero/known-one masks give population-count bounds. Leading/trailing
bounds use the most/least possible set-bit positions and preserve a total
count's full-width zero sentinel. Contradictory masks refuse. Nonzero scan
variants require a real known-one bit: their undefined-at-zero contract is not
used to invent a source fact. Multi-bit `x&mask != 0` does not identify a known
one bit and is deliberately insufficient for that proof.

Ordered predicates are monotone on the nonnegative count interval. Equal
endpoint truth proves the observation; Eq/Ne instead require an exact interval
or exclusion. Signed comparisons, negative constants, reversed operands and
both unsigned edge polarities are handled without conflating signed bounds.
Facts apply only to a selected consumer or an observation in a block with a
unique incoming conditional edge, not globally to shared producers. Terminal
proofs replace only the condition with a constant; ordinary CFG cleanup owns
successor/PHI maintenance, after the frozen-definition sweep.

The inverse population-count proof requires exact source identity and word
type. It removes redundant zero checks but neither clones nor mutates the
count producer. Loads/stores and volatile effects remain in place. Exact
zero/full observations also bridge the frontend's result type to the input
word: `popcount(x)==0`, `popcount(x)==W` and total `clz/ctz(x)==W` become source
comparisons. This last bridge admits only private count/cast/copy chains, so it
does not lengthen source liveness alongside a still-shared count.

No new speculative NZ scan, variable shift, LLVM poison, global analysis cache,
backend instruction policy, vector permission or SSE feature gate is introduced.

## Reproducible evidence and limitations

`tests/regression/check_count_observations.py` checks independent loop-reference
native results across 51 exported kernel forms, volatile evaluation counts, O0/O1/O2/O3/Os, debug and POPCNT
modes, SSA/IR/register-allocation verifiers, exact GCC 16.2 and optional Clang
23.1.3, disabled-policy controls, and an optional immutable baseline. It pins
actual constant-return mechanisms and refuses to credit missing bodies or a
baseline that already folds. Near bounds, signed guards, different sources,
shared scans, nonzero scan policy and custom split sentinels are controls.
The fixture uses 69,888 input pairs, not exhaustive 64-bit enumeration.

Twelve unit tests exhaust all byte words with all 256 partial-known-bit masks for
three count kinds, exercise all scalar carriers and comparison predicates,
check both guard edges/orders, input/result width separation, private exact
observations, shared refusals, cyclic copies and exhausted budgets. These are
finite tests of the real matcher, not a machine-checked proof for every IR.

`count-observations-correctness.json` records the current native/static receipt.
Instruction counts include RET and are not cycle measurements. The dedicated
`scripts/benchmark_count_observations.py` compiles kernel and driver in separate
translation units without LTO, preventing a same-TU constant-return optimization
from deleting calls. It records two warmups, alternating order, 15 paired wall
samples and modern vendor controls. The three cases are synthetic bounded-count,
zero-preimage and nonzero-implication calls, not gzip or application speedups.
A sample below 200 ms is flagged. No PMU, CPU isolation, universal speedup or
expert endorsement is claimed. Hosted compiler/oracle lanes remain responsible
for broader corpus and ICX coverage; their results must not be inferred from
these local receipts.

### Static outcomes on the reviewed base

| 64-bit probe | Frozen baseline / disabled | Candidate |
|---|---:|---:|
| bounded population count | 27 | 2 |
| bounded leading count | 14 | 2 |
| bounded trailing count | 16 | 2 |
| count zero observation | 27 | 5 |
| full population observation | 26 | 4 |
| guarded division | 32 | 28 |
| guarded remainder | 33 | 29 |
| nonzero implication | 27 | 25 |
| shared count plus bounded observation | 30 | 22 |
| multiple-predecessor division control | 37 | 37 |
| custom split zero32 control | 15 | 15 |

All native modes and both pinned vendor oracles agree on
`245f21e2c05c4eb8 139776`. The gzip longest-match workload's emitted main
remains 428 instructions, with a 120-byte frame adjustment and 65 non-push/pop
RSP-referencing instructions on both sides. This is a structural stack/code-size
nonregression check, not an assertion that those 65 references are spills or a
measurement of gzip throughput.
