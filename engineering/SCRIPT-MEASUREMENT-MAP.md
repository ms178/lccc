# Measurement-tool audit: scope and interpretation

This map distinguishes actual measurements from static surveys and diagnostic tools. It is not a claim that every line of the repository has been exhaustively reviewed.

| Tool | Purpose / admissible use | Important limitation |
|---|---|---|
| `tests/benchmark/run_benchmarks.py` | Authoritative 45-workload registry; compilation, output checks, randomized paired native timing, affinity/environment capture, bootstrap intervals | Process startup dominates short cases. Original compile records lacked executable hashes and did not reject missing/non-executable artifacts at compilation; lifecycle repair is separately regression-tested. |
| `scripts/benchmark_codegen.py` | All 45 **registry IDs**, retaining workload-specific flags, through the codegen oracle | Whole-translation-unit static counts, not runtime. Two registry entries can share a source but use different macro definitions. |
| `scripts/codegen_oracle.py` | Per-function/TU instruction, memory-reference and branch inventory; local and remote records | `spills`/stack references do not prove allocator spill causality; static instruction counts are not weighted dynamic costs. |
| `scripts/godbolt.py`, `scripts/godbolt_cache.py` | Pinned compiler discovery/audit; cached compile/execute requests; reproducible raw artifacts | ICX's `latest` alias is not proof of the vendor's latest release. HTTP 429 is failure, never an empty successful oracle. Client-cache isolation does not defeat server caching. |
| `scripts/oracle_profile.py` | Haswell-family feature admission and ICC flag-dialect translation | Feature macros prove an admitted ISA profile, not semantic correctness or native performance. |
| `scripts/oracle_asm.py` | Assembly retrieval for focused comparisons | Compiler exit, actual nonempty output, private temporary paths and propagated failures matter; repaired earlier and covered by FAST contracts. |
| `scripts/tune_oracle.py` | Inspect target-tuning tables and focused assembly differences | Not an automatic runtime-tuning acceptance mechanism. |
| `scripts/callgrind_corpus.py` | Exact 45-workload bounded instrumentation screen, including distinct macro-sized variants | Uses checked-in reduced sizes; not the default-size native ranking. Current non-size registry link requirements are covered by common `-lm`; future registry flags need corresponding review. |
| `scripts/callgrind_ab.py` | Correctness-gated A/B simulated Ir/cache/branch counts, fixed geometry, equal-length paths, raw counter provenance | Includes program/library/startup effects. Simulated events are not physical PMU events, cycles or Raptor Lake/EPYC measurements. |
| `scripts/perf_ab.py` | Focused legacy A/B experiments and diagnostic instruction metrics | Its low-tail/floor conventions differ from canonical paired medians; do not mix its results into the canonical native aggregate. |
| `scripts/codegen_sweep_compare.py` | Broad before/after emitted-assembly comparison, sibling flags/environment, explicit rejection buckets | Assembly differences are not necessarily regressions; equal rejection is not a successful semantic test. This is not the registry-aware 45-shape authority. |
| `scripts/check_benchmark_outputs.sh` | Benchmark-output semantic oracle | Slow CI gate; not run as part of local FAST. The separately requested 45-shape experiment checks outputs itself. |
| `scripts/differential_corpus.sh` | Differential corpus semantics | Not a substitute for the complete local FAST gate. |
| `scripts/ci_local.sh --fast` | Local acceptance contract including rustfmt, strict Clippy, unit/contract/architecture gates | Explicitly not full hosted CI. Never run `--slow` or full CI on this VM. |
| `scripts/zstd_preboot_oracle.sh` | Linux preboot decompressor, separate-buffer and optional in-place guest layout | Different workload from the user-repository zstd package. Formerly swallowed all four execution failures. Real kernel execution requires its kernel tree/payloads; mocked status contracts do not constitute a boot test. Hardcoded synthetic `/tmp/zstd-oracle` paths remain a separate portability issue. |
| `scripts/zstd_oracle_cc.sh` | Recompile/reuse the generated kernel preboot driver | Not a full packaged-zstd benchmark. |
| `scripts/build_lccc_fast.sh` | Constrained compiler build, fastbuild O1 and two jobs | Build performance and generated-code runtime are different objectives. |
| `scripts/lccc-snapshot.sh` | Commit and atomically publish patch/archive/bundle/ledger, with restore checks and FAST gating | An UNGATED emergency checkpoint is not accepted delivery. |

## Research instruments added for this audit

- `zstd_driver.c`: one shared GCC16-built timing driver, separately linked against each real patched zstd static library. Measures API calls with a monotonic clock; checks every iteration outside its timed region. Thus it isolates library generated code, not CLI startup, disk I/O, or the package's LLVM24 PGO/LTO/BOLT pipeline.
- `zstd_matrix.py`: six corpora × eight compression settings × compression/decompression, nine seeded randomized paired rounds, calibration, explicit 20 ms floor, raw commands/values/hashes and bootstrap intervals. GCC cross-decodes each compressor's frames before and after timed compression runs.
- `zstd_cli_matrix.py`: independent real-CLI byte-equality checks for levels, threads, long-distance mode, stream hints, checksums, dictionaries, patch-from, format wrappers, malformed frames, memory ceilings, boundaries and upstream goldens. This is correctness evidence, not a throughput measurement.

## Acceptance discipline

The 45-shape extraction is not the full zstd library. Conversely, one full-zstd roundtrip is not broad validation. Smaller assembly, lower simulated Ir and faster native execution are separate findings. Report failed/unsupported cases, floor-dominated cases, individual ratios, uncertainty, GM/AM and provenance; do not turn missing counters, failed subprocesses, incomplete surveys or mixed compiler identities into a win.
