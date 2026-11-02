# Follow-up: latest-main reconciliation and relocation hardening

## Rebase audit

Fetched main: `3c1f893480dc81f0fdc2bbf7973761c7266d6639` (PR #823).
S24 saved the complete old patch before the fetch. All 158 paths changed by
S23 exist upstream: 146 are byte-identical; the remaining twelve include
upstream refinements, not missing implementation. The integrated commits were
dropped rather than replaying them over their improved upstream versions.

Accepted upstream decisions: preserve the common constant/indexed GEP guard
and added displacement/volatile controls; retain the stricter CTest JUnit
validator; pin CCC_ARM in both CI mirrors; reject PYTHONOPTIMIZE instead of
letting Python assertions vanish; require --run in hosted popcount CI. These
strengthen the previous tests and contracts. No old test was weakened to make
the rebase pass. Existing arm source fixes and their evidence remain upstream.

The next implementation targets are the remaining linker/profile correctness
contracts. This checkpoint is UNGATED: old S23 results do not certify this new
upstream tree. Only fast local CI is authorized; targeted tests precede it.
