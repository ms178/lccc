# Semantic names

Name each pass for its invariant or rewrite, not its delivery order. Keep the
shared `bit_idioms_` prefix for sibling discovery: `facts` owns bounded facts;
`guards` owns Boolean/range/PHI reductions; `count_merge` owns count composition.
Tests use the same stem plus `_tests`. Fixture and checker stems match.

Evidence cohorts use short kebab-case names: `zero-choice`,
`integer-observations`, `bit-demand`, `ordered-arithmetic`, `hot-kernels`,
`scalar-facts`, and `guarded-folds`. Reports use the cohort stem; measurements
append `-runtime`, `-correctness`, or `-sources`. New cohorts follow this rule.

Historical measurement payloads retain their original commands, symbols,
source-tree hashes and artifact names. Renaming an archive does not rerun its
experiment. Do not rewrite recorded evidence to impersonate new measurements;
use the semantic cohort's report to locate the archived experiment.
