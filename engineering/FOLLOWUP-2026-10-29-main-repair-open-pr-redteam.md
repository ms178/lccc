# Main repair and open-PR red-team follow-up (2026-10-29)

## Findings from the open-PR audit

The checkout was the newly updated `main` at `d465f1c9`, after PR #1022 merged.
The branch's first fastbuild compile reproduced a release-blocking defect:
`vectorize_strided_i32_reductions_before_unroll` called
`strided_i32_reduce::try_transform` with four arguments, while the matcher
accepts three. This is the exact E0061 reported by open PR #1023. The unused
`ModuleSymbolFacts` value was not part of the matcher's proof; the helper and
caller now omit it, and the helper documents why the matcher does not need it.

The hosted checks for PR #1023 also reported `unused doc comment` above a
`thread_local!` macro. Rustdoc does not attach `///` comments to macro
invocations. The explanatory block is now ordinary source comments, while the
public setter/getter have their own valid item documentation. The per-TU
`CCC_NO_I686_SIGN_JCC` policy in that PR is retained: `run_passes` resolves it
once and installs it in thread-local state, and the kill-switch unit test uses
`ScopedFlag`, preventing process-global test-environment races.

## Open PR inventory and remaining scope

* **#1023** (main-repair/i686 switch): its two independent failures were
  reproduced from hosted annotations. The exact signature mismatch and the
  warning are repaired here. Focused build, Clippy, unit, and x86/i686 codegen
  checks are recorded below. The first hosted run on #1023 remains a failed
  historical run; the new PR's CI is required to validate these repairs.
* **#1021** (unsigned saturating subtraction / dormant reducer): its retrieved
  hosted check rollup is green, but its branch is conflicting with current
  `main`. The PR body itself lists follow-ups it does not solve: default-SSE2
  handling of `byte_load_reassembly`, SQLite rotate/address rematerialization,
  Linux list-sort branch layout and redundant endpoint testing, and the
  `shift_add` alias case. The pre-unroll reducer on current main is AVX2-gated;
  it does not establish that the report's default-flag SSE2 benchmark is fixed.
  These are not represented as solved by this compiler-build repair.
* `backlog.md` contains multiple tiers of independent driver, linker, warning,
  language-conformance, compile-time, ABI, and generated-runtime projects. This
  patch does not mark them closed or claim a whole-backlog audit. They require
  separate evidence and acceptance tests rather than a blanket rewrite.

## Validation performed

* Restored Rust 1.99.1, GCC 16.2.0, Clang 23.1.4 and binutils 2.47 from the
  reviewed `harness-cache-7`; created and activated a 4 GiB swap file before
  building.
* Before the repair, `cargo build --profile fastbuild --bin lccc --bin lccc-ld`
  failed with E0061 at the four-argument matcher call. After the repair, the
  same fastbuild command passed with warnings denied by the project profile.
* `cargo clippy --profile fastbuild --bin lccc --bin lccc-ld -- -D warnings`
  passed with no warnings.
* `cargo test --profile fastbuild --lib sign_test_jcc_tests --
  --test-threads=1`: 10 passed, 0 failed.
* `check_sign_test_cmov.sh` passed x86-64 output parity at O0/O1/O2/O3/Os with
  GCC 16.2.0 and Clang 23.1.4, including codegen and kill-switch assertions.
  i686 assembly generation and kill-switch codegen passed; its optional runtime
  parity was skipped because GCC cannot link 32-bit executables on this host.
* `check_vector_remainder_codegen.sh` passed with GCC 16.2.0, including the
  pre-unroll transform assertion and output/codegen checks.
* Formatting and `git diff --check` passed. No local CI aggregator was run.

ICX 2026.2 is not installed locally. Hosted ICX integration checks on PR #1021
passed, but they are not validation of this repair; rely on the new PR's hosted
CI for that lane. No hardware PMU is exposed here, so no runtime-speed claim is
made.
