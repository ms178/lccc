# Scalar count composition

This review follows the semantic naming policy in `NAMING.md`. Pass code is
`bit_idioms_count_merge.rs`, not a numbered delivery file. The reviewed base
includes the current aggregate-copy forwarding work on main. No upstream work
was overwritten during the rebase; the existing guarded-folds implementation
was replayed before the rename and count changes.

## Original reports and disposition

Six issue bodies and their accessible comments were read through the GitHub
REST API. `word-counts-sources.json` records URLs, timestamps and byte hashes.
The ignored `work/word-idioms/` directory holds the reviewed source records.
Titles and upstream status are not evidence of an LCCC implementation.

| Report | Reduced contract / disposition |
|---|---|
| [173691](https://github.com/llvm/llvm-project/issues/173691) | Implemented equal-width scalar `min(clz(x),clz(y))` and `min(ctz(x),ctz(y))` as a count of `x|y`. Both count kinds/widths must match; scan results have exactly the comparison/select users. |
| [90000](https://github.com/llvm/llvm-project/issues/90000) | Implemented constant caps strictly below the scalar width. Trailing counts inject bit `cap`; leading counts inject bit `width-1-cap`. The original leading example is off by one, as its comments also note. Variable caps, negative signed caps and width-sized shifts are refused. |
| [49562](https://github.com/llvm/llvm-project/issues/49562) | The split 64-bit population count still expands into two half scans in the default-target probe. Retained as a semantic control, not a claimed new fold. Wide promotion needs separate target-cost/32-bit lowering work. |
| [45243](https://github.com/llvm/llvm-project/issues/45243) | Conditional byte rotate remains a missed narrowing/select combination in the probe. Comments explicitly distinguish IR canonicalization from narrow x86 selection. Not conflated with count composition. |
| [51090](https://github.com/llvm/llvm-project/issues/51090) | The original 32-bit `x<<y | x>>(64-y)` has no count for which both C shifts are defined. It cannot supply a positive UB-free witness. A masked, defined 64-bit rotate is a control; its current cast/count normalization remains separate work. |
| [150954](https://github.com/llvm/llvm-project/issues/150954) | Narrow bit-length/CLZ conjugation remains a target/narrowing issue. The defined nonzero control is checked against a loop oracle, without claiming a new BSR optimization. |

These are two new transformation families and four native forms, not six new
optimizations or global closure of six reports. The remaining four reports are
explicit deferred work, not silently marked solved.

## Integration and legality

The matcher runs within the existing bounded scalar-facts observation context,
with the same early/late placement and exact `bit_idioms` disable policy.
A result rewrite allocates one fresh SSA value and preserves source spans via
the existing replacement path. Producers are not mutated or duplicated.

Only scalar integer 8/16/32/64-bit values of identical type are admitted. Both
strict and non-strict signed/unsigned comparisons are allowed: zero counts are
nonnegative and at most 64, representable even in signed byte results. Arm and
comparison identities must describe a minimum, never a maximum. The comparison
must have one user; each old scan must have exactly its comparison/select users.
Shared scans, mixed leading/trailing counts, mixed zero policies, width changes,
extra comparison users and unrelated arms are refused.

A merged total count stays total unless an original nonzero policy or existing
known-one-bit facts prove the union nonzero. A capped count is always nonzero:
the injected bit is within the operand width. This removes unnecessary zero
fallbacks without a new source-language UB or LLVM poison assertion. Total
zero/zero inputs retain the full-width result. No variable shift is introduced.
Vectors are excluded rather than assuming that OR/scan beats a native vector min.

## Evidence

The immutable renamed/rebased baseline demonstrates the missing mechanisms:

| Function | Baseline / disabled | Count composition |
|---|---:|---:|
| minimum trailing count | 18 | 6 |
| minimum leading count | 22 | 7 |
| capped trailing count | 11 | 4 |
| capped leading count | 13 | 5 |

These are static default-target instruction counts including RET, not PMU
measurements. Dedicated native checks cover optimization/debug modes, exact
GCC 16.2 and Clang 23.1.3, IR/SSA/register-allocation verification, disabled-pass
controls, both count families and refusal forms. The root fixture is a complete
single translation unit for the generic corpus runner.

Unit tests exhaust byte-width unions and every byte cap, then cover all eight
scalar integer types, eight ordered predicates and both zero policies. Shared,
mixed, maximum, width-mismatch and invalid-cap controls exercise the actual
matcher. Independent Z3 checks produce 24 expected outcomes with no UNKNOWN,
including SAT witnesses for the wrong leading sentinel and hardware-masked
out-of-range caps. These algebra checks are not compiler certificates.

Fresh runtime and full-workload evidence is recorded separately from the
renamed historical archives. Old measurement payloads retain their original
commands and hashes; neither renaming nor rebasing fabricates a fresh speedup.

[WORD-COUNTS-BENCH.md](WORD-COUNTS-BENCH.md) reports the fresh matched-ISA
paired experiment, all retained controls (including a shifted identical-binary
control), and the full-workload correctness outcomes.
