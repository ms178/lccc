# Rust 2024 migration, strict warning gate, and CI evidence hardening

**Date:** 2026-09-06
**Base:** `5d4a5462b520c9407f2898f156680663e902dbcc` (`ms178/lccc` `main`)
**Scope:** repository-wide Rust-2024 migration; warning-free Rust build/test
contract; dynamic Rust toolchain selection; and an evidence-driven response to
the two failing workflows associated with PR #423.

This is a maintenance/correctness change. It deliberately does **not** claim a
C-code performance win or use a CI timing result as a benchmark result.

## Outcomes

| Area | Result |
|---|---|
| Rust language edition | Package manifest upgraded from Rust 2021 to **Rust 2024**. |
| Compatibility floor | `rust-version = "1.98.1"`; the tested installed stable compiler is `rustc 1.98.1 (48a229cea 2026-09-01)`. |
| Active channel policy | `rust-toolchain.toml` follows `stable`, with the minimal profile plus `rustfmt` and `clippy`. The manifest, rather than a duplicated script literal, selects the channel. |
| Warnings | Fastbuild and release compiler builds deny warnings by default; strict Clippy and all-target test compilation are clean. |
| Rust formatting | `cargo fmt --all -- --check` is clean and is now a CI gate. |
| CI cache correctness | CI and benchmark workflows no longer restore a source-independent `target/` cache. Every measured/tested compiler binary is rebuilt from the checked-out source. |
| Remote-failure visibility | Regression failures now create individual GitHub check annotations and upload the raw runner log, host/toolchain/CPU fingerprint, binary hashes, and JSON result. |
| Pages setup | Current Pages action majors are used; `configure-pages@v6` receives `enablement: true` so a fork with no pre-existing Pages site can be initialized rather than failing its setup query. |

## Rust 2024 migration

`cargo fix --edition` initially identified 225 compatibility diagnostics and
made mechanical edits across 58 Rust source files. The resulting source was
then manually reviewed where a mechanical edit cannot demonstrate intended
borrow lifetime or naming quality.

### Explicit lifetime/drop-order corrections

Rust 2024 changes temporary tail-expression destruction order. Three affected
sites now have explicit short borrow scopes rather than relying on the prior
end-of-block temporary lifetime:

* `src/backend/generation.rs`
* `src/frontend/sema/analysis.rs`
* `src/ir/lowering/struct_init.rs`

In particular, the semantic-analysis path completes its `defined_structs`
`RefCell` borrow before emitting a diagnostic, and structure initialization
clones the layout while the layout borrow is active. These are behavior-neutral
lifetime clarifications, not broad `allow` attributes.

`gen` became a reserved Rust-2024 keyword. The liveness code uses the clear,
domain-specific `gen_set` name instead of the cargo-fix-generated `r#gen`
escape.

### Strict lint cleanup

Enabling warning denial for every target exposed test-only compiler warnings
that an ordinary production build did not see. They were fixed rather than
suppressed:

* test function names with prose emphasis (`SURVIVES`, `NOT`, `ADDRESS`, etc.)
  were normalized to snake case;
* test modules were placed after the production helpers they exercise, fixing
  `clippy::items_after_test_module` without relaxing the lint;
* `filter(...).next_back()` was replaced with its direct `rfind` equivalent;
* throwaway owned-string comparison, no-op bit masking, a no-op tag OR, and a
  heap `Vec` used only as a fixed scratch array were simplified;
* `lccc-ld` uses a Rust-2024-compatible `if`-`let` chain for the LTO-plugin
  input check.

Existing narrowly-scoped Clippy allowances remain only where they express a
known design tradeoff (for example, a deliberately long backend helper
signature). The enforced `-D warnings` result is the gate: no warnings are
accepted by the normal build/test/Clippy paths.

## Toolchain and bootstrap policy

`scripts/rust_toolchain.sh` is the shared selector. It reads the `channel`
field from the repository `rust-toolchain.toml`, falls back to `stable` only
when a worktree is unavailable, and preserves explicit caller intent in this
order:

1. `LCCC_RUST_TOOLCHAIN`,
2. a pre-existing `RUSTUP_TOOLCHAIN`,
3. the repository manifest channel.

The fastbuild, release `-O1`/`-j2`, session restore, bootstrap, and boot-size
bisection scripts use it. The `SymStr` migration helper also finds the
persisted Cargo proxy and selects the manifest channel when invoked outside the
repository directory. Restore/bootstrap now explicitly install `rustfmt` and
`clippy`, which are not present in the minimal Rust profile by default.

`tests/regression/check_rust_toolchain_selector.sh` is a committed, pure-shell
regression test covering manifest parsing, both override levels and the
missing-manifest fallback. It does not need to download a toolchain and is run
by CI before the compiler build.

The historical UTF-8 helper reproducer also resolves the current configured
Rust channel before invoking the rustup proxy from a temporary directory. Its
extracted historical snippet intentionally remains `--edition=2021`: it is
semantic evidence for a historical source blob, not source that belongs to the
Rust-2024 LCCC crate.

## CI changes

### Warning and formatting gates

The release build invoked by CI remains the project-required compiler build:
Rust optimization level 1, exactly two Cargo jobs, and thin LTO. The wrapper
now adds `-D warnings` unless a human deliberately sets
`LCCC_ALLOW_WARNINGS=1` for a scratch refactor. The fastbuild and bisection
paths use the same default warning policy.

The test job invokes `cargo test --profile fastbuild --all-targets --locked -j 2`
with `RUSTFLAGS=-D warnings`, keeping test compilation at the same Rust-O1,
two-job policy. The Clippy job now runs:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --locked -j 2 -- -D warnings
```

and is no longer marked `continue-on-error`.

### Source/binary identity and diagnostic evidence

The prior cache key used only `Cargo.lock` while caching `target/`. That is not
a sufficient identity for a compiler under test: source revisions can differ
while the dependency lock is the same. Although Cargo fingerprints normally
catch source changes, a regression gate must not depend on restored-target
mtime/fingerprint behavior to establish which compiler was tested.

Therefore the workflows retain dependency caches (`~/.cargo/registry` and
`~/.cargo/git`) but no longer cache `target/`. The release compiler used by the
regression corpus and benchmark job is constructed from the current checkout.

Before the regression run, CI writes `ci-regression-environment.txt` containing
OS, locale, Rust/GCC/GAS/ld versions, exposed CPU flags, and SHA-256 hashes of
`target/release/lccc` and `lccc-ld`. The corpus output is tee'd to
`regression-results.log`. On a corpus failure, a following `if: failure()` step
reads the JSON and emits one normal GitHub `::error` annotation per failed test
with its phase and compact detail. The artifact contains all of those records,
not merely an opaque process exit code.

This is observability and source-identity hardening, **not** a claim that a
cache was proven to be the remote failure mechanism.

### Pages workflow

The Pages workflow uses current action majors (`checkout@v7`,
`configure-pages@v6`, `upload-pages-artifact@v5`, and `deploy-pages@v5`).
The failed Pages build job `101410086806` stopped specifically at **Setup
Pages** with `Get Pages site failed` / `HttpError: Not Found`; its action
message explicitly directed the repository owner to enable Pages or use the
`enablement` parameter. `enablement: true` is now supplied to
`configure-pages`, which is the supported path for an initially disabled
Pages site. A new remote run is still required to verify GitHub-side
enablement and deployment permissions.

## PR #423 Test Suite investigation

### What GitHub exposed

The public REST metadata identifies two failing Test Suite jobs:

| Commit/run/job | Failed step | Duration | Public detail |
|---|---|---:|---|
| `419e6dc2`, run `34004385348`, job `101408830315` | SSA regression corpus | 71 s | `Process completed with exit code 2` |
| merge `5d4a5462`, run `34004858905`, job `101410086832` | SSA regression corpus | 62 s | `Process completed with exit code 2` |

`run_regression.py` returns the number of failed cases, so an exit status of 2
is consistent with two failed corpus cases after normal execution, not a
missing compiler binary (which would terminate almost immediately). GitHub's
public check annotations name neither case, and anonymous raw-job-log and
artifact download are restricted. No unsupported inference about the test names
has been made here.

### Reproduction matrix

All reproduced corpus commands used `CCC_VALIDATE_SSA=1`, `-j 2`, the full
678-case corpus, and the checked-in runner. The exact PR source was also kept
in a detached worktree for the historical compiler builds.

| Compiler and reference configuration | Result |
|---|---|
| Current Rust-2024 tree, required release wrapper (`-O1`, `-j2`, warnings denied) | **665 pass, 0 fail, 13 skip-compare, 678 total** in 57 s |
| Exact PR source, default Cargo release (`-O3`), Rust 1.98.1 | **665 / 0 / 13 / 678** |
| Exact PR source, default Cargo release (`-O3`), Rust 1.98.0 (the former manifest pin) | **665 / 0 / 13 / 678** in 54 s |
| Exact PR compiler against GCC 13.3 differential oracle | **662 pass, 0 fail, 16 skip-compare, 678 total** |
| Exact PR compiler, default GCC 14.2, five sequential full-corpus stress runs | **5 × (665 / 0 / 13 / 678)**; 53–54 s each |
| Exact PR compiler, each of the two PR-added focused cases, 200 compile/run iterations | `peephole_utf8_inline_asm`: 200/200; `i686_over_aligned_struct_arg`: 200/200; one binary digest per case |

The historical UTF-8 reproducer passed under the dynamically selected stable
channel and demonstrated the expected pre-fix byte-to-char recoding. The
current release compiler passed the separate byte-exact inline-assembly UTF-8
gate: both original UTF-8 markers occurred in the `#APP` region, neither
Latin-1-style recoding occurred, and the generated executable exited zero.

### Honest conclusion

The remote two-case failure was **not reproduced** locally with either Rust
1.98.0 or 1.98.1, either GCC 13.3 or 14.2, five independent full-corpus runs,
or repeated targeted compilation. The old/default Rust `-O3` setting is
therefore not a demonstrated cause, and the required `-O1` build policy is not
presented as a magic regression fix.

The changes do remove a real CI correctness hazard (a target cache whose key
omitted source identity) and ensure the next remote failure exposes the exact
case, command phase, compiler digest, toolchain and host feature set. Remote
success/failure after these changes remains the final evidence for the
GitHub-specific component; this record intentionally does not claim that an
unavailable private log has been root-caused.

## Snapshot re-anchor and atomicity

The preceding artifact ledger was anchored to PR #422's base while PR #423 is
now merged in `main`. Publishing a new cumulative patch against that old base
would incorrectly re-export already upstream changes. The tracked short
`.base_ref` marker is reset to `5d4a5462`, and `lccc-snapshot.sh` now accepts
an explicit `LCCC_BASE_REF` to atomically reset the authoritative artifact
baseline after verifying it is an ancestor of the committed snapshot head. The
forthcoming snapshot uses `5d4a5462` as its base, so `/home/user/ms178-1.patch`
contains only this unmerged Rust/CI maintenance unit.

The snapshot tool now builds and patch-checks off-path before publication;
atomically copies all patch variants; creates nonempty tarball and bundle
files before renaming them into place; records SHA-256 values in the ledger;
and advances the sequence only after every artifact and ledger entry is
published. A temporary backup protects the prior `series/` directory during
its directory rename. This removes prior `|| true` paths that could advertise
a snapshot after a failed tarball or bundle generation.

## Validation record

All commands below were run after the migration edits; captured logs and JSON
are retained under `/home/user/artifacts/rust2024/` and
`/home/user/artifacts/ci-423/` in the working environment.

| Gate | Result |
|---|---|
| `cargo metadata --no-deps --format-version 1` | package reports `edition = 2024`, `rust_version = 1.98.1` |
| `bash tests/regression/check_rust_toolchain_selector.sh` | pass: manifest selection, overrides, fallback |
| `python3 scripts/symstr_migrate.py` (dry run, strict diagnostic build) | `round 1: clean build, no errors` |
| `scripts/build_lccc_fast.sh` | pass; Rust 1.98.1 stable, fastbuild/O1, warnings denied |
| `scripts/build_lccc_o1_j2.sh` | pass; release thin-LTO, Rust O1, two jobs, warnings denied (8m17s) |
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --all-targets --locked -j2 -- -D warnings` | pass |
| `RUSTFLAGS='-D warnings' cargo test --profile fastbuild --all-targets --locked -j2` | **1,996 passed, 0 failed, 6 ignored** library tests; all binary targets compiled and ran their empty suites |
| Full SSA regression with the release compiler | **665 passed, 0 failed, 13 skip-compare, 0 skip-run, 678 total** |
| `scripts/check_inline_asm_utf8.py` with release compiler | pass, byte-exact markers preserved and executable passed |
| linker byte-mutator fuzz (`FUZZ_N=128`) | 128 mutants, 0 defect classes |
| linker grammar fuzz (`--iters 64 --seed 20260906`) | 62 link attempts, 18 mutation classes, 0 defects |
| workflow parse (PyYAML) and touched shell/Python syntax checks | pass |
| isolated `lccc-snapshot.sh` self-test | pass: commits a synthetic change, re-anchors its base, produces nonempty patch/tar/bundle/series/ledger, and `git apply --check` accepts the patch at that base |
| `git diff --check` | pass |

## Follow-up boundary

The Rust/CI unit is complete locally and is intentionally isolated from
optimization experiments. The next performance unit must use the refreshed
warning-free compiler build, retain its own correctness and assembly evidence,
and record hardware-sensitive results separately from GitHub-hosted CI timing.
