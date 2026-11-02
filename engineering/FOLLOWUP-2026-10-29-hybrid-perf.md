# Follow-up: Arch PKGBUILD and P/E-core corpus perf (2026-10-29)

## Scope and baseline

This session addresses the separate `ms178/archpkgbuilds` local LCCC recipe,
not a generated-code optimization. LCCC upstream base was
`ed928575365e1c56a03e040d53ab5dc5af83a860`; the package repository base was
`c61a04bc93de1a6937043bfe9dae9e178ba2ab37`. Rebase the patches to the then
current upstreams before another session. The recipe already had an unrelated
`ms178-1.patch` file which it did not apply. That older file was intentionally
not overwritten or silently applied: a direct `git apply --check` of it against
this session's LCCC main failed extensively. The new recipe applies the separate
`lccc-hybrid-perf.patch`; `/home/user/ms178-1.patch` is this session's **LCCC
source** cumulative patch, not the pre-existing package-repo patch. The separate
`archpkgbuilds.patch` contains the recipe and its source patch.

## Completed and evidence

* Root cause of the Fish question: Cargo builds `lccc-ld` and its driver handles
  `-v`, but `package_lccc()` omitted it. Install it as an ordinary executable
  and fail packaging if either main executable is missing; no shell alias. The
  actual fastbuild binaries were invoked under Fish: `lccc -v` displayed LCCC
  and `lccc-ld -v` displayed `GNU ld (LCCC built-in) 2.42`.
* The opt-in package check calls the canonical complete benchmark registry with
  the just-built LCCC and installed GCC, per allowed P/E class, then per-program
  perf stat; a source-tree `--only` exists only for focused development. See
  `docs/hybrid-perf-benchmark.md` for semantic details and PMU constraints.
* `scripts/test_benchmark_hybrid_perf.py`: six passing tests include fake hybrid
  topology, override rejection, unsupported/zero/multiplexed counter rejection,
  a real gzip-CRC corpus compile/run under GCC with a **fake perf process**, and
  rejection of changed benchmark stdout. Fake values are transport test data,
  not performance evidence. CI and `ci_local.sh` run the test; parity passed.
* A real fastbuild (Rust 1.99.1, profile `fastbuild`, Rust `-O1`, Cargo `-j 2`,
  debug=0 and incremental disabled as memory-saving overrides) completed. The
  initial build with 512 MiB swap was SIGKILLed; increasing swap to 4 GiB and
  disabling debug/incremental enabled it. Swap is ephemeral to this host.
* Real LCCC vs GCC correctness-only runs for `gzip_crc32`, `zlib_ng_adler32`,
  and `expat_xml_scan` passed; no timing claim was made. A three-case wrapper
  run with real LCCC correctly failed its PMU preflight (`cycles` unsupported)
  and retained the report, rather than fabricating counters.
* The package recipe's focused smoke test exercises disabled/enabled toggle,
  P/E argument forwarding, no subset selection, required linker and a Fish
  invocation, but uses mocks for the staged package binaries and Python entry.
  Actual source linker `-v` is independently checked as above.

## Open validation and follow-up

1. Run `ci_local.sh --fast` to completion on the exact final LCCC tree with the
   required project toolchains. A running/partial job and an ungated snapshot
   are **not** a pass; inspect its log and stamp. Fix only reproduced failures.
2. On real Arch, run `makepkg -s` both off and on, then install and query the
   actual packaged executables from Fish. Validate the local recipe in a clean
   chroot as well. This Debian VM does not provide an Arch makepkg test.
3. On the i7-14700KF verify kernel `cpu_core`/`cpu_atom` CPU lists and collect
   separate full-corpus P/E reports with actual hardware PMU access. Confirm
   event names, `--all-user`, CSV format and running percentage with the local
   perf version; confirm the user-owned permissions without changing policy
   silently. A VM and synthetic counters cannot validate hybrid hardware.
4. Investigate short-program startup bias, SMT sibling interference, thermal
   drift, multiplexing, frequency variation, confidence intervals and worst
   regression. Add optional calibrated batches/counter statistics only after
   comparing identical measured object code and preserving correctness checks.
5. Expand the event menu carefully (cache/TLB, top-down P-core metrics) with
   explicit PMU-specific support/availability and additional independent runs;
   do not pretend unsupported E-core events or multiplexed results are valid.
6. Full gzip/zlib-ng/Expat applications and other real workloads are not proven
   by the extracted kernels. Compare whole-program behavior and codegen on real
   hardware before any performance claim; ICX/ICC, GCC 16.2, linker oracles,
   full codebase audit, corpus expansion and algorithmic improvements remain
   outside this task and were not completed or benchmarked in this session.

## Harness toolchain incident and recovery

A first partial `ci_local.sh --fast` run encountered missing GCC 16 and 32-bit
headers. Debian Sid GCC/G++ 16.2.0 with multilib was then installed. **Caution:**
this upgraded the live Trixie host libc to Sid 2.43, contrary to the safety
policy in `scripts/ensure_i386_multilib_safe.sh`; do not repeat that live-upgrade
method on a persistent or production host. Prefer an isolated toolchain/sysroot.
A trivial GCC `-m32` PIE then crashed at 0x1. `objdump` showed the 2.44 linker
had rewritten the `Scrt1.o` `R_386_GOT32X` indirect `push main` into a jump
straight to `main`, so `main` returned to address 1. The same source linked
with `-no-pie` worked. Upgrading sandbox binutils from 2.44 to Sid
2.47.50.20261021 preserved the indirect push and the identical GCC 16 `-m32`
PIE passed. This diagnoses a mixed-toolchain environment failure, not a proof
about LCCC's own linker. Check `gcc-16 -dumpfullversion`, `ld -v`, and execute
both 32- and 64-bit probes before trusting later CI runs; retain the exact
system package versions in measurement records. The virtual system's PMU is
still unavailable, irrespective of these compiler-toolchain changes.

## Additional corpus and CI investigation

The complete registered corpus correctness-only run finished 63/63 correct
against Debian GCC 16.2 (`--skip-perf --strict`, no timing claim). The initial
CI wiring added a separate step and the lane-plan audit correctly rejected it;
the test now runs in the already planned benchmark step, and both lane-plan
and local/hosted invocation parity checks pass.

A partial fast-CI run previously exposed a mixed-GCC `#include_next` failure:
with an older system GCC and GCC 16 internal headers, wrapper traversal could
fail to reach the libc endpoint. The earlier workaround of putting GCC 16 first
in `PATH` was not treated as a fix. On the current tree, include resolution
carries the search-chain origin with each opened file and resumes at the next
slot. The focused gate now adds a cross-version regression using the installed
GCC 12 and GCC 16.2 `stdint.h`/`limits.h` wrappers plus a minimal libc endpoint;
it checks both path orders, `-m64`/`-m32`, macro expansion and visited-header
line markers against LCCC, GCC 16.2 and Clang 23.1.4. All 12 compiler/target/
order cells passed locally. This validates the current implementation for
those measured wrappers; it is not an exhaustive proof for every GCC header
layout. GCC 14 headers and ICX were unavailable in this sandbox, and the full
hosted CI run remains the acceptance gate for the change.
