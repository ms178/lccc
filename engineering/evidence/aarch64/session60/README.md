# AArch64 session 60 evidence

* `codegen-oracle.md`: LCCC fastbuild versus ARM64 GCC 16.1, GCC trunk, and
  Clang 22.1. The canonical conditional-increment leaf is 3 instructions for
  every compiler and has no stack traffic.
* `benchmark-vm-screening.md`: paired randomized runtime screening for binaries
  built by fastbuild LCCC/integrated lccc-ld and GCC. This is host-VM evidence,
  not ARM PMU evidence.

See git history (commit range of 2026-08-22, session 60) for legality,
validation, and remaining gaps.
