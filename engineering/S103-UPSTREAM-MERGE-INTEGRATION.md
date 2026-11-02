# S103 — integrate merged PR836 without erasing its follow-ups

Base: `2f477ce01749824ccab73ead7ccac90abd77fa08`. Pre-rebase head:
`864df43bf7115cba0fa8d181be64c37162de0f46`, preserved as S102 against its
actual base `84a62201cc90a41c9a0eefe2faf5196f43370cfa` before rebase.
The pre-merge bundle is also retained externally as
`/home/user/artifacts/pre-836-merge-S102.bundle`; its original manifest is
`/home/user/artifacts/S102-PRE-836-BUNDLE.json` (original filename therein is
historical). These are external workspace recovery artifacts, not repository
paths; the cumulative patch and source archive do not embed that old bundle.

## Alternatives and resolution

Replaying the entire old cumulative lineage could overwrite fixes already
merged through PR836. Instead, rebase only the S95–S102 follow-up series onto
new main, retaining the prior history in S102 and its bundle. New main's
production implementation already incorporates the earlier changes.

- Keep upstream's fallible TempFile API repair, all six additional lifetime
  tests, and both independently scoped S95 tests. Do not replace the upstream
  test suite with the older copy. The only conflict in vector-copy integration
  was `unwrap` versus descriptive `expect` in a test: keep upstream.
- Keep upstream's original PR831 capture at `engineering/PR831-AUDIT.md` and
  its separate current `PR831-COMPILER-AUDIT.md`. Keep the original inventory
  bytes, closed scope and verifier. Adapt the new regression to this restored
  layout, not the superseded S99 relocation. Skip the now-redundant formatting
  commit. Never refresh historical hashes from a rewritten live review.
- Retain upstream's Actions notice budgets, explicit GNU C17 comparison
  controls, registry-required wrapping and independent full-size workload
  contracts. Retain our raw-pair timing validation and atomic completion
  lifecycle. Their combined focused tests pass.
- S97 directive/known-exit fixes remain the only production-code delta beyond
  new main. TempFile changes are tests only.

## Validation at integration

Fastbuild, `-O1`, `-j2`, active 6 GiB swap. Rustfmt passes. Rust library tests:
**4847 passed, 0 failed, 9 ignored**. Focused Python: evidence inventory 8,
timing provenance 9, comparison reports 18, evidence publication 8,
completion lifecycle 4, input contracts 3: **50 passed**. This is not the final
FAST gate. Compressed raw logs accompany this note.

## Refreshed PR selection

All **3 open PRs: 829, 825, 786**. Latest **20 by closed_at**:
**836, 837, 835, 833, 834, 832, 831, 830, 828, 824, 826, 827, 814, 823,
821, 822, 816, 777, 776, 725**. The sorted-updated pagination boundary is older
than the selected closed-at cutoff; the selection proof is retained.

PR836 is now merged, not open; PR819 leaves the latest-20 window. The other
22 heads and update timestamps are unchanged from the S97 capture, so their
bounded dispositions and captured source/test/tooling diffs remain applicable
in `PR-REDTEAM-2026-10-15-DISPOSITION.md` and the REDTEAM97 review archive.
PR836 metadata, complete updated patch and discussion/review responses were
refetched. Absence of submitted reviews is not external approval. This work
still does not certify exhaustive independent review of every hunk of every PR.

All completed S101/S102 main-relative timings use **84a62201**, not this base.
Fresh builds and executable identity checks are required before transferring
any current candidate/vendor evidence; old-main ratios must stay historical.
