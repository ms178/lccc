# PR836/PR837 reconciliation — 2026-10-15

The initial PR836 commit `3671c7c9` has the exact S93 source tree. Its current
reviewed head at inventory time is `d9c76ec5`: five additional commits, not the
same delivery. Main advanced to `84a62201` by merging PR837 (`036f60ee`).
S93 was hash-verified before rebase; S94 saved the rebased state without
transferring the earlier FAST certificate.

## Decisions

- Retain PR837's upper-vector call observation and lazy liveness cache. The
  PR836 vector changes merge around those implementations; do not replace the
  module wholesale with the older branch's eager implementation.
- Adopt PR836's five review commits, including raw warm-up correctness checks,
  architectural frame read extents, fixed-register constraints, corrected
  packed-denominator homes and structural AArch64 relocation provenance.
  Adoption is not evidence of universal correctness or runtime improvement.
- Resolve the temporary-file conflict using the stronger fallible leased API.
  Both branches use Arc/Weak directory lifetimes; the PR836 implementation also
  refuses insecure flat-path fallback and retains a kept *pending* directory.
  Keeping the old infallible API would preserve the known insecure fallback.
- Preserve PR837's unmanaged-directory and explicit-guard lifetime coverage
  through two tests adapted to the new pool. The explicit sibling case also
  checks private directory mode0700 on Unix.
- The first full unit compilation exposed two integration errors: PR837's
  native wide-call witness still called the removed `TempFile::new`. Switch
  only those test reservations to `try_new(...).expect(...)`; do not resurrect
  an insecure production API or disable the typed wide-argument test.

## Executed evidence

Rust1.99.1, GCC16.2.0-3, Clang23.1.3, mold2.42.1; active6GiB swap.
The production build uses fastbuild/-O1/-j2 with warnings denied: PASS.
Full unit compilation before the integration adaptation: FAIL, two E0599
errors. After adaptation: **4,836 passed,0 failed,9 ignored**,9.33s test time.
The ninth ignore is PR837's explicitly opt-in synthetic cost benchmark, not a
new correctness waiver. Full/slow CI was not invoked; final FAST status must
come from a matching current-tree receipt.

A separate adversarial report audit already reproduces fifteen failing
subcases across five of six test methods: cached timing medians/ratios and
malformed pair records can still certify unsupported performance claims.
That fix and its tests are separate work; Python correctness flags alone do
not establish a paired timing result.
