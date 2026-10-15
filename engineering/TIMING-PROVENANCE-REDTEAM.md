# Paired timing evidence: red-before/green-after audit

PR836's raw warm-up correctness fix is valid but insufficient for performance
certification. The reporter still trusted cached timing medians/ratios and a
count no greater than the arm's list length. Nine observations do not prove
nine *paired* observations, and an output-correct program can still have an
invented speed claim attached to it.

## Reproductions and decision

The six-method independent probe exposes15 failing subcases in five methods:
a cached0.01 ratio can contradict actual2.0 pairs; a cached100ms median can
hide1ms raw executions; missing/duplicate/reordered/mismatched round ledgers
and invalid raw durations are admitted; `complete:false` is accepted.
The valid paired control passes before and after. Frozen original reporter,
protocol, hashes and logs are in `engineering/evidence/redteam96`.

Reject merely adding another cached flag or comparing list lengths. Recompute
from the authoritative raw round ledger, after proving its per-arm projection
matches the recorded timed-run lists. Require contiguous integer round IDs,
unique/permutation-correct execution orders, registered arms, successful typed
statuses and positive finite durations. Do not include warm-up durations.

The raw medians determine the20ms floor. The actual paired values determine
sample count, median and bootstrap uncertainty. Contradictory cached point
summaries exclude a row from timing aggregates; cached confidence intervals
and cached raw ratios are never copied into the result. Recomputed bootstrap
uses a deterministic data-derived seed; a per-report sample-tuple cache avoids
repeating identical2000-resample calculations without changing the estimator.
The report still retains correctness/exclusion inventories and all raw input.

Existing fixtures now contain real timed rounds instead of cached statistics
alone. Existing conservative correctness checks are retained. Nine provenance
methods also cover forged uncertainty, nonintegral sample counts, bool-as-int
round IDs/statuses, and preservation of genuinely eligible comparisons.

## Validation and limits

Before:6 methods,15 failing subcases. After:9 provenance methods PASS,
18 existing report/hosted/witness methods PASS,11 correctness-safety methods
PASS. FAST discovers the new suite through its existing corpus gate; no gate
waiver or new slow/local workload is introduced.

This fixes a report's trust boundary, not generated code. It does not prove
that old real samples were fabricated, that wall timing is physical-host
performance, or that self-reported raw observations are cryptographically
authenticated. No native speedup follows from this change.
