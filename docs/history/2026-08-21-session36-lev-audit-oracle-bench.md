# Session 36 — Lev audit, x86 CRC peephole, codegen oracle, benchmark expansion

**Date:** 2026-08-21 UTC  
**Base:** `ms178/lccc` `dffe0b19bea3bf4d23effbf7c1a82e5cda49888a`  
**Primary target class:** x86-64 Raptor Lake / x86-64-v3, while preserving cross-target correctness.

## Environment and reproducibility

The constrained VM was prepared for the mandated fast REACT loop:

- 4 GiB swap installed at `/swapfile` and activated.
- local LCCC built with `scripts/build_lccc_fast.sh` / Cargo `fastbuild` (`-O1`, no LTO, incremental, 2 jobs).
- local `.cargo/config.toml` selected `clang -fuse-ld=mold`; this is a host-only build accelerator and is excluded from the patch.
- i686 differential tests enabled by installing 32-bit glibc support.
- `perf` is unavailable in this KVM sandbox, so runtime data are explicitly labelled as **VM wall-clock screening**, not Raptor Lake PMU evidence. Static assembly and instruction/load/store metrics are the reliable local signal; Compiler Explorer provides GCC 16.2, Clang 22.1, ICC 2021.10 and latest ICX.

## Lev Kropp commits from 2026-08-20 onward audited

Lev's branch was fetched as `levkropp/main`. Merge-base with `ms178/main` is old (`0bce973e`), so each commit was treated as untrusted and assessed semantically rather than by conflict text.

| Commit | Decision | Rationale |
|---|---|---|
| `6a42dd1e` docs/roadmap | Reject for ms178 | Documentation only; reflects Lev's AArch64 RA branch assumptions, not the current ms178 state. |
| `9c48a277` AArch64 F64 callee-saved allocation | Hold | Target-specific and plausible, but requires AArch64 runtime validation. This VM lacks a usable `aarch64-linux-gnu-gcc` due to an apt multilib conflict; do not integrate blind. |
| `ed36c446` substitution GlobalAddr CSE + backend hazards | Reject / superseded | ms178 already contains class-aware GlobalAddr CSE (`97893efe`, `7bc196e8`) with more precise use-class handling and x86 validations. Lev's older generic version risks regressing those fixes. |
| `e3b21b8f` AArch64 fmsub/fnmsub | Hold | AArch64 only, behind gating, but touches shared `generation.rs`; needs AArch64 QEMU + GCC differential before adoption. |
| `62cc4bb1` roadmap correction | Reject | Documentation correction for Lev's branch; no code. |
| `1b4bac8b` aggregate hoist + default full unroll | Reject for now | The generic hoist does not apply to ms178's more conservative `aggregate_copy_forward`; enabling full unroll by default was already rejected in `2026-08-21-session33-levkropp-aug20-gaddr-cse.md` because the x86 gzip/expat policy favors bounded 2..=8 unrolling. Revisit with a register-pressure-aware cost model and zlib-ng/gzip/expat A/B. |
| `aadab341` AArch64 spill slot store forwarding | Hold | Large AArch64-only text peephole. The idea is useful but must be audited against call clobbers, aliased slots, and QEMU differential tests. |
| `e20cdf6e` AArch64 escape-analysis DSE | Hold | Soundness-sensitive DSE. Port only after extracting the AArch64-specific liveness/alias assumptions into tests; do not import the whole text pass. |
| `de451e57` AArch64 ldp/stp fusion | Hold | Useful AArch64 code-size/win candidate, but not x86-64 Raptor Lake and needs byte/offset/alias testing. |
| `b712c79c` AArch64 repeated slot load dedup | Hold | Similar to existing x86 memory-fold work, but target-specific; test with AArch64 cross compiler before adoption. |
| `d0a30aa0` roadmap dead-ends | Reject | Docs only. |
| `8ef1978f` AArch64 conditional-sum reduction | Reject as-is / port later | Adds vectorization intended for NEON. The generic vectorizer touch points are interesting, but x86 already has its own AVX2/SSE paths; this should be reimplemented generically rather than dragging AArch64 emission into x86 builds. |
| `89111a1f` AArch64 zero-extension move chains | Hold | AArch64-only; may inspire x86 equivalents, but no direct adoption. |
| `2e57bcf2` AArch64 Mul/Load/Add fmadd | Hold | Target-specific fused operation; requires FP-contract and accumulator semantics audit. |
| `8b139820` AArch64 i32 max reductions | Hold/rework | The `find_max` reduction shape is worth adding to the generic/x86 vectorizer later, but this commit is NEON-specific. |
| `dbd22184` benchmark refresh docs | Reject | Benchmark numbers belong to Lev's branch and include AArch64/recursion claims not applicable to this ms178 baseline. |
| `e03da2f1` aggregate/array init DSE fix | Hold, do not cherry-pick mechanically | The reported bug class is real, but ms178's `aggregate_copy_forward` already fails closed for unknown suffixes and escapes. Lev's periodic-overlap refinement may improve DSE precision; however, it should be rederived on top of ms178 with unit tests for loop-variant pointers, multi-suffix phis, calls, intrinsics, volatile roots, and `fib_rec2iter`. |

**Net:** no Lev commit was directly cherry-picked this session. The x86 primary target is better served by a small verified local fix plus oracle/benchmark infrastructure than by importing large AArch64 changes untested.

## Implemented x86 optimization: sound `movq` -> `movl` narrowing

`src/backend/x86/codegen/peephole/passes/redundant_ext.rs` now narrows type-erased register copies when the immediately following instruction consumes or overwrites the destination's 32-bit form:

```text
movzbl (%rdx), %r8d
movl  %esi, %eax
movq  %r8, %rcx       ->  movl %r8d, %ecx
xorl  %r8d, %eax      ->  xorl %ecx, %eax
movq  %rax, %r8       ->  movl %eax, %r8d
andl  $255, %r8d
```

This is deliberately local because the text peephole has no IR type information. It:

- requires a register-to-register source and destination;
- requires the next real instruction to use the exact 32-bit destination alias;
- refuses if the same line mentions the 64-bit alias;
- refuses `%esp`/`%ebp` destinations because those are not general allocable homes and later passes may legitimately consume the 64-bit copy;
- invalidates zero-extension state at calls, so stale upper-bit knowledge cannot survive a call;
- leaves memory/immediate sources to other passes.

On the isolated scalar CRC recurrence (`crc32_update`) this removes four 64-bit copies in the hot loop and avoids REX.W/64-bit dependencies. It is an incremental code-quality win, not a claim of beating GCC: GCC 16.2 still wins the compact recurrence (14 static instructions vs LCCC 28 after this patch), primarily through pointer/end loop structure and table-memory operand folding.

### Validation

- Added `tests/regression/check_crc32_zeroext_moves.sh` to reject 64-bit copies of the zero-extended index/XOR value in the CRC hot loop and cap loop-body bloat.
- Added Rust unit tests for the narrowing pattern and for not narrowing opaque memory-load copies.
- Found and fixed two attempted over-generalizations during validation:
  - a first global-state version miscompiled `adler_inline_tail`, `switch_table` and `varargs_abi`;
  - a second version still altered `%rbp` in the existing copy-shift-copyback test.
- Final gates:
  - `cargo test --lib --profile fastbuild -j 2`: **980 passed, 0 failed, 6 ignored**;
  - `tests/regression/run_regression.sh --compare-gcc` at `-O2`: **378 passed, 0 failed**;
  - `tests/correctness/run_correctness.py`: **50/50 passed**;
  - focused `-O3` runtime checks for `adler_inline_tail`, `switch_table`, `varargs_abi`, `gzip_crc32`, `zlib_ng_adler32`, and `expat_xml_scan` all passed.

## Codegen oracle improvements

Added `scripts/codegen_oracle.py`, derived from the existing `scripts/godbolt.py` API but specialized for batch comparison:

- compares local LCCC against GCC 16.2, Clang 22.1, ICC 2021.10 and latest ICX;
- supports multiple `--function` arguments and whole-translation-unit mode;
- writes per-compiler assembly, JSON manifests and Markdown summaries;
- records static instructions, loads, stores, spills, branches and vectors;
- caches remote compiles through `godbolt.py`;
- returns non-zero if any compiler fails, preventing false scoreboards.

Example:

```bash
python3 scripts/codegen_oracle.py /tmp/crc_hot.c \
  --local target/fastbuild/lccc \
  --local-flags "-O3 -march=x86-64-v3 -I$(gcc -print-file-name=include)" \
  --flags "-O3 -march=x86-64-v3" \
  --function crc32_update \
  --artifact-dir results/codegen-oracle/crc \
  --json results/codegen-oracle/crc.json \
  --markdown results/codegen-oracle/crc.md
```

Current CRC scoreboard from that script:

| Compiler | Static instructions | Ratio |
|---|---:|---:|
| GCC 16.2 | 14 | 1.00x |
| LCCC | 28 | 2.00x |
| ICC 2021.10 | 36 | 2.57x |
| Clang 22.1 | 49 | 3.50x |
| ICX latest | 65 | 4.64x |

GCC is the target to beat here; Clang/ICC/ICX are not.

## Benchmark corpus expansion

Added four self-contained, deterministic common-algorithm kernels under `tests/benchmark/programs`:

- `ascii_case_fold.c` — parser-like byte classification and dependent checksum;
- `binary_search.c` — branch-heavy sorted table lookup;
- `ring_fifo.c` — masked enqueue/dequeue with dependent loads/stores;
- `histogram.c` — scattered 256-bin increments and final reduction.

They are registered in `run_benchmarks.py` and documented in `WORKLOAD_PROVENANCE.md` as original LCCC test code (not extracted upstream sources). They expand coverage for addressing, branch lowering, register pressure and memory folding without adding license ambiguity.

A 7-round VM screening run (1 warm-up, pinned, no PMU) showed all new and existing selected workloads correct. Timings are noisy because several kernels run under 20 ms; they must not be treated as Raptor Lake performance evidence. Representative ratios:

| Workload | LCCC/GCC median | Note |
|---|---:|---|
| binary_search | 0.928 | small noisy win |
| histogram | 1.009 | within CI |
| ring_fifo | 1.051 | under 20 ms |
| ascii_case_fold | 1.063 | under 20 ms |
| glibc_memcmp | 1.114 | under 20 ms |
| gzip_crc32 | 1.066 | stable hot loop gap |
| zlib_ng_adler32 | 1.612 | top gap |
| sqlite_varint | 1.705 | top gap |
| linux_find_bit | 1.719 | top gap |
| expat_xml_scan | 1.821 | worst selected gap |

## Regression harness hygiene

`run_regression.sh` now honors `LCCC_NO_COMPARE=1` in a sibling `<test>.txt` file. Seven lccc-conformance tests were previously reported as GCC mismatches even though GCC itself was a defective or inapplicable default-mode oracle:

- `builtin_cpu_supports_raptor` (host CPU differs from fixed Raptor Lake expectation);
- `code16_realmode_encoding` (GCC/PIE link failure on real-mode absolute symbol);
- `fp_domain_crossing` (tolerance-only FP comparison, not byte-exact stdout);
- `has_attribute_in_code` (GCC behavior differs for the probe);
- `kernel_flags_and_builtins` (GCC 14 default mode lacks C23 `typeof_unqual`);
- `sqrt_vex_scalar` (requires `-lm` after source);
- `va_opt_c2x` (requires `-std=c2x` and GCC rejects one shape in default mode).

This removes false noise without weakening any lccc runtime assertion.

Also updated `tests/linker/setup_workloads.sh` from zlib-ng 2.2.4 to 2.3.3, matching `archpkgbuilds/packages/zlib-ng/PKGBUILD` and `WORKLOAD_PROVENANCE.md`; the old 2.2.4 directory is removed on rebuild.

## Validation summary

Commands run and artifacts retained under `/home/user/results`:

- `/home/user/results/final-cargo2.log` — 980 passed / 0 failed / 6 ignored;
- `/home/user/results/final-regression-o2.log` — 378 passed / 0 failed;
- `/home/user/results/final-correctness.log` — 50/50;
- `/home/user/results/final-bench.{log,json,md}` — selected benchmark correctness + VM screening;
- `/home/user/results/codegen-oracle/crc/` — CE assembly and scoreboard for the CRC hot loop.

The canonical patch is `/home/user/ms178-1.patch`, with snapshots under `/home/user/artifacts` and a verified `APPLIES-CLEAN` verdict.

## Follow-up work, prioritized

1. **Beat GCC on `expat_xml_scan`.** Use `codegen_oracle.py` to isolate `xmltok_impl.c` name-scan hot blocks. Inspect branch/cmove lowering, byte-load zero extension, table/memory operands, and block layout. This is the largest selected gap (1.82x).
2. **Beat GCC on `linux_find_bit`.** Investigate `__ffs`/tzcnt/branch shape and the sparse-bitmap loop. Compare `-march=x86-64-v3` against BMI/BMI2 forms; do not use BMI without runtime/static feature validation.
3. **Beat GCC on `sqlite_varint`.** Focus on compare chain, widening, redundant shifts and branch layout. SQLite is a golden workload and the kernel is self-contained.
4. **Beat GCC on `zlib_ng_adler32`.** Separate scalar NMAX path from SWAR/vector path. The current 1.61x gap likely involves accumulator live ranges and 64-bit math scheduling.
5. **Improve CRC table lookup.** GCC uses `crc32table(,%rcx,4)` directly and a pointer/end loop. Add a sound x86 peephole/MachInst fold for `movl table(,%index,4), %reg` after zero-extended index formation, then revisit loop IV structure.
6. **Port Lev's AArch64 ideas only after AArch64 CI is available.** Install or otherwise provide an AArch64 cross compiler without breaking i686 multilib, then run `tests/regression/run_regression_arm.sh` under QEMU. Candidates in order: `de451e57` (ldp/stp), `b712c79c` (slot load dedup), `9c48a277` (F64 callee-saved), then FP fusions.
7. **Rederive `e03da2f1`'s periodic-load DSE on ms178.** Keep the current fail-closed behavior until a patch proves it can retain unread residue stores without deleting same-residue initializing stores. Required tests: loop-variant GEPs, multi-suffix PHIs, memcpy/call/intrinsic escapes, volatile roots, and `tests/fib_rec2iter.c`.
8. **Reassess full unrolling with a real cost model.** Do not enable `1b4bac8b`'s default full unroll blindly. Gate by loop body size, register pressure, memory operations, vectorization opportunity and measured zlib-ng/gzip/expat runtime.
9. **Extend `codegen_oracle.py`.** Add function auto-discovery, per-function best/worst rankings, optional text size via `.text`, and direct integration with `codegen_scoreboard.py` so the oracle drives the ROI queue.
10. **Upgrade VM measurements.** On Raptor Lake, collect cycles, instructions, IPC, branches, branch-misses, L1/LLC misses, ITLB/frontend stalls with `perf stat -M` TopDown where available. Current VM timings are screening only.
11. **Run graduated workloads.** Build zlib-ng 2.3.3, gzip 1.14, expat 2.8.2 and SQLite 3.53.4 from `ms178/archpkgbuilds` recipes, then capture object-level assembly and controlled runtime. The benchmark kernels are not sufficient proof by themselves.
12. **Investigate glibc 2.44 and cachymod kernel integration separately.** Those require the custom toolchain and kernel patch ordering from the user's package repositories; they should not be faked in this constrained VM.

## Snapshot chain

- `S01-x86-movl-zeroext-v2`
- `S02-benchmark-oracle-scripts`
- `S03-regression-oracle-hygiene`
- `S04-final-validation-clean`

Each snapshot refreshed `/home/user/ms178-1.patch`, `/home/user/artifacts/ms178-1.patch`, the git bundle and full source tarball atomically.
