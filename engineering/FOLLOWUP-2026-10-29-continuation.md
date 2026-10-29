# Continuation: resilient restore and hybrid performance package

## Recovery and verified state

The harness reset between turns removed `/var/tmp`, `/swapfile`, Rust, installed
packages, `.git`, `target/`, and executable mode bits in the restored workspace
source copies. `/home/user/ms178-1.patch`, the package patch, source tarballs,
bundle and ledger survived. `ms178/lccc` main remained at
`ed928575365e1c56a03e040d53ab5dc5af83a860` and `ms178/archpkgbuilds` main at
`c61a04bc93de1a6937043bfe9dae9e178ba2ab37`. A fresh clone plus the saved LCCC
patch produced an exact binary-diff match; a fresh sparse package clone plus its
saved patch recovered the package. The latter's embedded LCCC patch and checksum
were then refreshed to the final LCCC snapshot, addressing the previous mismatch.
The package patch and a package-tree tarball plus prerequisite git bundle are
saved in `/home/user` and `/home/user/artifacts`. Both base refs must be
rechecked before the next session; never assume `.git` or mode bits persist.

The 63/63 complete corpus correctness-only comparison from the previous turn
remains in `/home/user/corpus-full-correctness.json`; it is not timing or PMU
evidence. Focused hybrid-runner tests, CI lane-plan and local/hosted parity
passed after restore. The actual rebuilt linker and compiler responded to `-v`
in Fish. A new LCCC build completed with fastbuild, Rust `-O1`, `-j2`, debug=0,
incremental=0, and 4 GiB active swap; the fresh toolchain was Rust 1.99.1.

Unlike the previous session's risky in-place Debian Sid libc upgrade, the
continuation used the repository's digest-verified `harness-cache-7`: GCC 16.2,
Clang 23.1.4 and binutils 2.47 in a private disposable prefix, with Debian
Trixie multilib and Valgrind. The oracle verifier passed, including execution
of a 32-bit program. `ci_export_gcc_runtime.sh` constructed an additive
`LCCC_SYSROOT` from this exact private GCC. Putting a symlink to the **actual**
GCC 16 driver at a private `gcc` path aligned default-GCC headers with the
oracle; the complete IVSR domain gate then passed. This is a temporary host
configuration, not a fix for the documented mixed-include `#include_next` bug.

A full `ci_local.sh --fast` was started with these private oracles. Before it
was intentionally stopped to record this follow-up in the final source tree,
104 gates had passed with zero failures; that partial run is NOT a green CI
stamp. Start the entire gate on the final committed tree and keep its result.
The source snapshot remains `UNGATED` until such a stamp exists.

## Remaining high-priority work

* Run the full fast CI gate to completion, diagnose any failures, and retain
  exact log, environment and matching tree stamp. Some external tool or host
  capabilities may still be missing; do not relabel a partial pass as green.
* Test the PKGBUILD with real Arch `makepkg` and a real P/E-capable i7-14700KF
  PMU, including actual `perf stat` event naming/CSV/running percentages and
  permission-denied failure paths. The VM has no hardware PMU. The automated
  perf transport test uses a fake tool and is not performance evidence.
* Fix and test the mixed-GCC include-next search-provenance defect, rather than
  depending on a single matching `gcc` version in PATH. The correctness and
  benchmark oracle must not loop between GCC wrapper headers or silently fall
  back to a different include path after the next-search is exhausted.
* Measure real whole-application workloads and evaluate cache/TLB/top-down
  events separately only when the actual PMU supports them. Do not infer such
  results from the 63 synthetic/workload-derived kernel correctness cases.
