# Follow-up: PR791 independent review

Date: 2026-10-11. Base: `6dd8eeeaa98aa5314b7a06264ae5753734c493e7`.
The companion audit explains agreement, qualifications, proofs and measured
results. This document records completed work and the next bounded tasks.

## Completed

- Re-cloned current ms178 main after the workspace/toolchain restore; installed
  and activated 6 GiB swap. Built both baseline and candidate with the required
  fastbuild O1 / two-job recipe, Rust 1.99.0, debug info disabled and incremental
  compilation disabled for the approximately 2 GiB host.
- Reproduced the baseline CLZ zero diamond, guarded OR1 cost, width-level IR
  semantic mismatch and mixed-rewrite source-span failure. Preserved failure
  logs before production changes. Did not treat the review's scoring or its
  historical non-regression claim as evidence.
- Implemented width-checked CLZ/CTZ/bswap/bitreverse recognition, one deferred
  expansion queue, atomic paired mask destination checks, preserved source
  spans, exact safe nonzero count intrinsics and scoped guard elimination.
- Added direct-mask/reversed-arm CTZ normalization, absent from the review but
  required to make its proposed positive TZCNT control actually pass.
- Added ten Rust tests across the recognizer, CVP and simplifier, including
  hostile widths/edges and literal carriers, both CTZ condition polarities, signed count carriers,
  4096 count inputs per mode, same-block rewrite permutations and a cross-block
  mask producer whose consumer follows an expanding CTZ.
- Extended the existing salvage gate rather than adding a disconnected test
  runner: 28 executable configurations, GCC output comparisons, explicit
  baseline execution, i686 paired-register execution, and baseline/v3 assembly
  contracts. The new codegen checker is part of the patch.
- Used the existing Compiler Explorer tool for GCC 16.2, Clang 23.1.0,
  ICC 2021.10.0 and the ICX latest channel. Local runtime references used the
  actually installed GCC 14.2; these two reference roles are not conflated.
- Ran seven workload-derived kernels before/after with strict correctness and
  **no timings**. The first candidate exposed a Linux find-bit codegen
  regression from CTZ's high-bit OR. Fixed its dominance-scoped bypass and
  verified the original Linux binary was restored byte-for-byte. Six of seven
  final workload binaries are identical to baseline; bitops loses two static
  instructions. All seven agree with the local GCC reference.
- Preserved cumulative source/patch/archive/bundle checkpoints. One publication
  attempt correctly refused the workspace file-count budget; archived this
  session's raw oracle directories instead of deleting user files or relaxing
  the safeguard, then reran publication successfully.

## Delivery and validation discipline

The focused tests passed: **4290 Rust tests, 0 failures, 7 ignored**, and all
28 C configurations. These checks do not replace the final
`scripts/ci_local.sh --fast` gate. The deliverable publisher requires its
matching exact-tree pass stamp; interim checkpoints are explicitly UNGATED.

The final gate receipt belongs **outside** the tree it authenticates:
`LCCC-DELIVERY.json`, `$WORKSPACE/artifacts/SNAPSHOT_LEDGER.md`, and the persisted
`artifacts/pr791-review/ci-fast.log` / `ci_local.pass`. Consult those for the
actual final gate result and tree hash (`WORKSPACE` denotes the delivered
workspace root, not the source checkout). No slow gates were authorized.

## Remaining work, in priority order

| Priority | Gap | Next bounded experiment / acceptance condition |
|---|---|---|
| P1 | Truncated count trees remain inefficient | `clz_trunc` is still 42 static instructions versus ICC's 5; `ctz_trunc` is 54 versus 30. Prove partial-count/range simplifications, keep the executable counterexamples, and revise the current negative opcode shape checks only alongside those proofs. Never restore arbitrary cast peeling. |
| P1 | Guard elimination is deliberately conservative | Select fallback W currently creates a defined CLZ only when the Select carrier and intrinsic operand widths match. Wider/narrower result carriers need an explicit result Cast (not a different-width intrinsic). Guard aliases through widening are handled by CVP, not broadened blindly in the simplifier. Add full pass-level type/zero tests before widening recognition. |
| P1 | Some zero guards still survive | `bitops` loses MOV/OR, but its CFG/Phi zero guard survives later lowering as test/cmov. Standalone guarded controls also retain explicit zero-return branches. A separate CFG-to-defined-intrinsic fold could remove those, while preserving bare-NonZero safety and source spans. This session does not claim the globally minimal sequence. |
| P2 | Cross-target coverage | i686 and x86-64 execute the count regressions; Rust/fast-suite checks cover additional paths. No new AArch64/RISC-V hardware execution or exhaustive count-lowering matrix is claimed. Add bounded cross-target semantic tests when suitable executors are available. |
| P2 | Runtime performance | Static reductions are established, not speedups. Use the existing paired runner on a quiet host, report per-workload distributions, and keep the strict output checks. No PMU/Raptor-Lake claims can be made from this VM session. |
| P2 | Whole-project integration | The seven checks are extracted kernels (gzip, zlib-ng, Expat, SQLite, Linux, glibc, plus bitops), not rebuilt full applications, glibc integration, Linux boot, or a new whole-codebase audit. Those remain separate work. |

## Reproduction

```sh
# Baseline for this patch:
git checkout 6dd8eeeaa98aa5314b7a06264ae5753734c493e7
# Apply the cumulative ms178-1.patch, then:
CARGO_PROFILE_FASTBUILD_DEBUG=0 CARGO_INCREMENTAL=0 scripts/build_lccc_fast.sh
CCC=target/fastbuild/lccc bash tests/regression/check_a9987_salvage.sh

# The complete authorized gate, against a frozen source/docs tree:
CARGO_PROFILE_FASTBUILD_DEBUG=0 CARGO_INCREMENTAL=0 CI_LOCAL_JOBS=2 \
  bash scripts/ci_local.sh --fast

# Correctness-only workload distribution; no synthetic geomean or PMU claims:
python3 tests/benchmark/run_benchmarks.py \
  --only gzip_crc32,zlib_ng_adler32,expat_xml_scan,sqlite_varint,linux_find_bit,glibc_memcmp,bitops \
  --compilers lccc,gcc --opt=-O2 --skip-perf --no-pmu-probe --strict \
  --artifact-dir target/pr791-workloads --json target/pr791-workloads.json

# Call per source: a function filter must actually exist in that source.
python3 scripts/codegen_oracle.py tests/regression/count_zero_intermediate_casts.c \
  --local target/fastbuild/lccc --flags='-O2 -march=x86-64-v3' \
  --oracles gcc16.2,clang,icc,icx \
  --function clz_full --function ctz_full --function clz_trunc --function ctz_trunc \
  --artifact-dir target/pr791-oracle-count --json target/pr791-oracle-count.json
python3 scripts/codegen_oracle.py tests/regression/clz_safe_guards.c \
  --local target/fastbuild/lccc --flags='-O2 -march=x86-64-v3' \
  --oracles gcc16.2,clang,icc,icx \
  --function clz_guard32 --function clz_guard31 --function clz_guard_live \
  --function clz_guard_wide --function ctz_guard64 \
  --artifact-dir target/pr791-oracle-guard --json target/pr791-oracle-guard.json
```

Compact checked results, source hashes, actual compiler labels, output strings
and workload binary hashes: `engineering/evidence/pr791-review-2026-10-11.json`.
Full session logs and raw oracle manifests remain in
`artifacts/pr791-review`; initial raw oracle files are compressed together to
respect the persisted-workspace budget. Intermediate failed/aborted runs are
not final validation receipts.
