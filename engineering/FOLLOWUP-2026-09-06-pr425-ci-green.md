# Follow-up — PR #425 Rust 2024 / CI and Clippy green

Date: 2026-09-06 UTC
Base: `54126902d370b0765577d433b4f860f1f3d0701b` (`main`)
PR head under test: `3ae28a4b62727d77c4e05f459ed761bd799d1d7b`
Working branch: `arena/fix-pr425-ci`

## Objective and scope

This session made the Rust 2024 migration in PR #425 mergeable without
weakening the warning, formatting, regression, or linker-fuzz gates. The Rust
2024 changes were retained. The patch is deliberately limited to the
formatting required by the new strict formatting gate, one incorrect regression
expectation uncovered by the full test suite, and CI's fastbuild artifact
contract.

The compiler itself was built with the repository's fastbuild policy throughout
this session: Rust 1.98.1, opt-level 1, two Cargo jobs, incremental compilation,
no LTO. An 8 GiB `/swapfile` was created and activated before building. The
container's `/etc/fstab` is not writable, so activation is valid for this
session but cannot be made persistent by the unprivileged session wrapper.

## Root causes found

### 1. Rustfmt was not clean after the edition migration

`cargo fmt --all -- --check` reported seven formatting regions in:

- `src/backend/x86/codegen/intrinsics.rs`
- `src/backend/x86/codegen/machinst_alloc.rs`
- `src/backend/x86/codegen/machinst_tests.rs`
- `src/passes/simplify.rs`

These were ordinary rustfmt deltas (comment indentation, import ordering,
compact conditionals, and assertion layout), not semantic changes. Running
rustfmt and then rerunning the check produced a clean result.

### 2. A table-driven regalloc test asserted the wrong default

The `RaConfig` parser has always defined `CCC_MI_MAX_LOOP_INSTS`'s default as
`4096`; the malformed-input case in the same test already asserted `4096`.
The Rust 2024 migration added/retained a contradictory empty-environment
expectation of `32` in
`backend::regalloc::ra_config_tests::parser_preserves_numeric_text_and_polarity_contracts`.
The implementation was correct; the test oracle was not. The expectation is
now `4096`. The configured-value assertion remains `41`, so both default and
explicit parsing contracts are covered.

### 3. CI must consume the profile it builds

The CI job now uses `scripts/build_lccc_fast.sh`, which intentionally writes
binaries to `target/fastbuild/`. All subsequent checks in that job—hashing,
regression execution, inline-assembly validation, and linker fuzzing—now use
`target/fastbuild/` as well. Strict Clippy also names `--profile fastbuild` so
that CI follows the same bounded, low-memory build policy as the test job.

This is not a generated-code quality claim: the profile affects the Rust
compiler binary and CI resource use, not the C `-O2`/`-O3` level passed to LCCC
by workload tests.

## Validation completed

All of the following completed successfully on the 1.98.1 toolchain:

- `cargo fmt --all -- --check`
- `./scripts/build_lccc_fast.sh`
- `cargo test --profile fastbuild --all-targets --locked -j 2`
  - 2,022 tests discovered; 2,016 passed, 6 ignored, 0 failed
- `cargo clippy --all-targets --profile fastbuild --locked -j 2 -- -D warnings`
- the original CI-shaped strict Clippy command,
  `cargo clippy --all-targets --locked -j 2 -- -D warnings`
- `bash tests/regression/check_rust_toolchain_selector.sh`
- the full C regression corpus with `CCC_VALIDATE_SSA=1`
  - 681 total; 666 passed, 0 failed, 13 compare-skips, 2 host skips
- `python3 scripts/check_inline_asm_utf8.py --expect preserved`
  - exact UTF-8 preservation passed; legacy behavior remained rejected
- deterministic linker smoke fuzzing
  - 128 mutants, 0 defect classes
  - ELF grammar fuzz: 62 link attempts, 0 defects
- patch integrity via `scripts/lccc-snapshot.sh`
  - every session snapshot reported `APPLIES-CLEAN`

The current canonical deliverable is `/home/user/ms178-1.patch`. The matching
source tarball, git bundle, patch series, and snapshot ledger are under
`/home/user/artifacts/`.

## Deliberately deferred work

The following are not silently represented as completed:

1. Rebase the finished branch again immediately before opening/updating the
   PR if `upstream/main` advances beyond the recorded base. The snapshot is
   currently anchored to the latest `main` observed at session start.
2. Run the large external workload/oracle matrix (gzip, zlib-ng, expat, SQLite,
   glibc, kernel, GCC/Clang/ICX) on a host with stable CPU affinity and usable
   PMU access. This session fixed CI correctness and did not claim a runtime
   win.
3. Decide separately whether the benchmark-only workflow should be moved from
   its ship-quality release compiler build to fastbuild. The ordinary CI job
   fixed here is intentionally fastbuild; benchmark methodology must preserve
   a clearly documented compiler-build profile when its results are compared.
4. Re-run GitHub-hosted Actions after the patch is pushed. Local validation
   covers the commands and artifact paths, but GitHub action versions, runner
   images, permissions, and hosted-service behavior remain external evidence.

## Next-session entry point

Start by fetching `upstream/main`, checking ancestry, and rerunning the
snapshot script with an explicit `LCCC_BASE_REF` if main moved. Then run the
CI-shaped fastbuild build and the full regression corpus before any codegen
experiment. Keep `/home/user/ms178-1.patch` and the artifact ledger refreshed
after each independently validated change; do not use the benchmark oracle to
mask a correctness or warning failure.
