# LCCC — Lightning Fast Claude's C Compiler

> An optimized fork of [CCC](https://github.com/anthropics/claudes-c-compiler) based on additions made by Lev Kropp (https://www.levkropp.com/lccc/).

---

## What is LCCC?

CCC (Claude's C Compiler) is a zero-dependency C compiler written entirely in Rust by Claude Opus 4.6 and Arena.ai Agents,
capable of compiling real projects — gzip, zlib-ng, expat, SQLite, the Linux kernel and glibc — for x86-64, AArch64,
RISC-V 64, and i686, with its own assembler and linker.

LCCC is a performance fork and a personal AI agent research project.

---

## Licensing

LCCC uses a dual-license model to separate original contributions from CCC-derived code.

**LCCC contributions** (new files, regalloc changes, benchmarks, docs) —
MIT OR Apache-2.0 OR BSD-2-Clause (your choice). See `LICENSE-MIT`, `LICENSE-APACHE`, `LICENSE-BSD`.

**CCC-derived code** (frontend, SSA IR, optimizer, backends, assembler, linker) —
CC0 1.0 Universal (public domain). CCC was released as CC0 by Anthropic.

See [`LICENSING.md`](LICENSING.md) for the full breakdown and per-file guidance.

# LCCC Performance Report

**Generated:** 2026-08-11

**Measurement date:** 2026-08-10

**Repository benchmark source revision:** `7121253f4ea4d8a9cdeb6718f4b63cea07a932cc`

**Patch base for `ms178-1.patch`:** `05b3770f2a69b31a08b3d36d36f3c71f3809d63f`

> This report uses the latest **completed and correctness-matching** measurements available in the workspace. Benchmarking was paused before the final patch handoff; no new measurements were fabricated after that pause.

## Executive summary

Latest paired results cover **18 workloads**. All selected LCCC/GCC pairs produced matching stdout.

| Aggregate | Result |
|---|---:|
| Geometric mean LCCC/GCC | **1.147×** |
| Arithmetic mean LCCC/GCC | **2.893×** |
| Median LCCC/GCC | **1.274×** |
| LCCC faster than GCC | **2/18** |
| Within ±5% of GCC | **2/18** |
| Slower than GCC | **14/18** |
| Correctness matches | **18/18** |

Ratios are `LCCC/GCC`; lower is better. A ratio below `1.0×` means LCCC is faster.

## Latest runtime measurements

| Workload | Latest experiment | Reps | LCCC best | LCCC mean ± σ | GCC best | GCC mean ± σ | Ratio | Correct |
|---|---|---:|---:|---:|---:|---:|---:|:---:|
| `arith_loop` | `baseline-full-corrected` | 3 | 0.106241 s | 0.107838 s ± 0.001541 s | 0.102438 s | 0.104707 s ± 0.002000 s | 1.037× | yes |
| `fib` | `ipcp-eval` | 3 | 0.002242 s | 0.002304 s ± 0.000060 s | 0.170275 s | 0.181121 s ± 0.015124 s | 0.013× | yes |
| `matmul` | `scalar-fp-shape2` | 2 | 0.009067 s | 0.009106 s ± 0.000055 s | 0.007436 s | 0.007438 s ± 0.000003 s | 1.219× | yes |
| `qsort` | `baseline-full-corrected` | 3 | 0.139382 s | 0.142193 s ± 0.003186 s | 0.125386 s | 0.126878 s ± 0.001495 s | 1.112× | yes |
| `sieve` | `sib-sieve2` | 2 | 0.052701 s | 0.053023 s ± 0.000456 s | 0.039275 s | 0.039295 s ± 0.000028 s | 1.342× | yes |
| `tce_sum` | `ipcp-eval` | 3 | 0.002226 s | 0.002262 s ± 0.000037 s | 0.002232 s | 0.002279 s ± 0.000068 s | 0.997× | yes |
| `nbody` | `xmm-relay-fold` | 2 | 1.393552 s | 1.395825 s ± 0.003214 s | 0.306050 s | 0.306742 s ± 0.000980 s | 4.553× | yes |
| `binary_trees` | `baseline-full-corrected` | 3 | 1.541272 s | 1.583437 s ± 0.036831 s | 1.253126 s | 1.273384 s ± 0.031801 s | 1.230× | yes |
| `spectral_norm` | `xmm-relay-fold` | 2 | 2.034054 s | 2.045861 s ± 0.016698 s | 0.201029 s | 0.201412 s ± 0.000541 s | 10.118× | yes |
| `mandelbrot` | `scalar-fp-shape2` | 2 | 4.288573 s | 4.290641 s ± 0.002925 s | 1.489785 s | 1.492256 s ± 0.003494 s | 2.879× | yes |
| `hash_table` | `baseline-full-corrected` | 3 | 10.643701 s | 11.253258 s ± 0.978189 s | 9.313629 s | 9.541095 s ± 0.200607 s | 1.143× | yes |
| `strlen_bench` | `baseline-full-corrected` | 3 | 0.257594 s | 0.259167 s ± 0.001995 s | 0.223083 s | 0.223949 s ± 0.001043 s | 1.155× | yes |
| `switch_dispatch` | `baseline-full-corrected` | 3 | 0.740460 s | 0.740649 s ± 0.000221 s | 0.502586 s | 0.502988 s ± 0.000373 s | 1.473× | yes |
| `struct_copy` | `memcpy-load-forward` | 2 | 0.499828 s | 0.505257 s ± 0.007678 s | 0.027234 s | 0.027472 s ± 0.000337 s | 18.353× | yes |
| `loop_patterns` | `scalar-fp-shape2` | 2 | 0.156491 s | 0.156893 s ± 0.000568 s | 0.079630 s | 0.079634 s ± 0.000006 s | 1.965× | yes |
| `fannkuch` | `baseline-full-corrected` | 3 | 3.352670 s | 3.365284 s ± 0.021226 s | 2.541956 s | 2.543153 s ± 0.001112 s | 1.319× | yes |
| `ackermann` | `ipcp-eval` | 3 | 0.002224 s | 0.002297 s ± 0.000078 s | 0.150480 s | 0.152068 s ± 0.001499 s | 0.015× | yes |
| `bitops` | `scalar-fp-shape2` | 2 | 0.843678 s | 0.962960 s ± 0.168689 s | 0.392352 s | 0.392465 s ± 0.000159 s | 2.150× | yes |

## Latest compile time and binary size

| Workload | LCCC compile | GCC compile | LCCC binary | GCC binary | LCCC `.text` | GCC `.text` |
|---|---:|---:|---:|---:|---:|---:|
| `arith_loop` | 0.026717 s | 0.051943 s | 16,224 B | 15,992 B | 2,131 B | 2,388 B |
| `fib` | 0.028979 s | 0.077066 s | 16,216 B | 15,984 B | 968 B | 2,204 B |
| `matmul` | 0.028685 s | 0.048015 s | 16,296 B | 16,064 B | 1,521 B | 1,792 B |
| `qsort` | 0.028220 s | 0.041362 s | 16,256 B | 16,064 B | 1,136 B | 1,519 B |
| `sieve` | 0.035647 s | 0.044118 s | 16,264 B | 16,072 B | 1,246 B | 1,601 B |
| `tce_sum` | 0.023367 s | 0.035293 s | 16,192 B | 15,960 B | 899 B | 1,339 B |
| `nbody` | 0.047986 s | 0.078405 s | 20,640 B | 16,336 B | 5,605 B | 2,241 B |
| `binary_trees` | 0.120317 s | 0.159718 s | 28,584 B | 20,248 B | 16,619 B | 8,169 B |
| `spectral_norm` | 0.041137 s | 0.067802 s | 16,192 B | 16,096 B | 2,700 B | 2,099 B |
| `mandelbrot` | 0.026036 s | 0.041369 s | 16,192 B | 15,960 B | 1,707 B | 1,602 B |
| `hash_table` | 0.033386 s | 0.053648 s | 16,288 B | 16,104 B | 1,578 B | 1,940 B |
| `strlen_bench` | 0.052667 s | 0.059277 s | 16,280 B | 16,144 B | 2,272 B | 2,292 B |
| `switch_dispatch` | 0.027157 s | 0.044964 s | 16,224 B | 15,968 B | 1,442 B | 1,750 B |
| `struct_copy` | 0.039787 s | 0.051235 s | 16,192 B | 15,960 B | 1,868 B | 1,814 B |
| `loop_patterns` | 0.139026 s | 0.058563 s | 16,256 B | 16,024 B | 1,728 B | 2,063 B |
| `fannkuch` | 0.026136 s | 0.055038 s | 16,376 B | 16,168 B | 1,519 B | 1,949 B |
| `ackermann` | 0.061118 s | 0.048614 s | 16,192 B | 15,992 B | 880 B | 1,787 B |
| `bitops` | 0.045169 s | 0.051973 s | 16,192 B | 15,952 B | 1,761 B | 1,712 B |

## Performance deltas versus the corrected full baseline

| Workload | Baseline ratio | Latest ratio | Ratio improvement | Latest observation |
|---|---:|---:|---:|---|
| `ackermann` | 6.840× | 0.015× | +99.8% | Bounded pure recursive constant evaluator folds `ackermann(3,11)`. |
| `tce_sum` | 3.799× | 0.997× | +73.7% | Closed-form tail-sum lowering plus post-TCE constant propagation. |
| `struct_copy` | 34.112× | 18.353× | +46.2% | Static aggregate inlining plus memcpy-chain/load forwarding. |
| `nbody` | 6.530× | 4.553× | +30.3% | Scalar-FP XMM allocation heuristic and XMM relay folding. |
| `spectral_norm` | 12.546× | 10.118× | +19.3% | Scalar-FP XMM allocation heuristic and XMM relay folding. |
| `mandelbrot` | 3.510× | 2.879× | +18.0% | Scalar-FP location heuristic reduces XMM/GPR relays. |
| `loop_patterns` | 2.204× | 1.965× | +10.8% | Scalar-FP heuristic and backend cleanup. |
| `sieve` | 1.502× | 1.342× | +10.6% | Late LEA-to-SIB folding removes address temporaries. |
| `arith_loop` | 1.037× | 1.037× | +0.0% | No new targeted optimization measurement. |
| `qsort` | 1.112× | 1.112× | +0.0% | No new targeted optimization measurement. |
| `binary_trees` | 1.230× | 1.230× | +0.0% | No new targeted optimization measurement. |
| `hash_table` | 1.143× | 1.143× | +0.0% | No new targeted optimization measurement. |
| `strlen_bench` | 1.155× | 1.155× | +0.0% | No new targeted optimization measurement. |
| `switch_dispatch` | 1.473× | 1.473× | +0.0% | No new targeted optimization measurement. |
| `fannkuch` | 1.319× | 1.319× | +0.0% | No new targeted optimization measurement. |
| `matmul` | 1.219× | 1.219× | -0.0% | No material improvement in the latest targeted run. |
| `fib` | 0.013× | 0.013× | -0.8% | Existing recursion-to-iteration win retained. |
| `bitops` | 1.774× | 2.150× | -21.2% | No new targeted optimization measurement. |

## Largest remaining measured gaps

| Rank | Workload | Latest ratio | LCCC best | GCC best | Primary evidence |
|---:|---|---:|---:|---:|---|
| 1 | `struct_copy` | 18.353× | 0.499828 s | 0.027234 s | Assembly/benchmark evidence in `reports/` |
| 2 | `spectral_norm` | 10.118× | 2.034054 s | 0.201029 s | Assembly/benchmark evidence in `reports/` |
| 3 | `nbody` | 4.553× | 1.393552 s | 0.306050 s | Assembly/benchmark evidence in `reports/` |
| 4 | `mandelbrot` | 2.879× | 4.288573 s | 1.489785 s | Assembly/benchmark evidence in `reports/` |
| 5 | `bitops` | 2.150× | 0.843678 s | 0.392352 s | Assembly/benchmark evidence in `reports/` |
| 6 | `loop_patterns` | 1.965× | 0.156491 s | 0.079630 s | Assembly/benchmark evidence in `reports/` |
| 7 | `switch_dispatch` | 1.473× | 0.740460 s | 0.502586 s | Assembly/benchmark evidence in `reports/` |
| 8 | `sieve` | 1.342× | 0.052701 s | 0.039275 s | Assembly/benchmark evidence in `reports/` |

## Accepted optimization results

- **Tail-recursive sum:** `3.799×` baseline → `0.997×` latest paired result.
- **Constant recursive evaluation:** Ackermann benchmark `6.840×` baseline → `0.015×` latest paired result.
- **Sieve address generation:** `1.502×` baseline → `1.342×` after LEA/SIB folding.
- **N-body:** `6.530×` baseline → `4.553×` after scalar-FP allocation/relay work.
- **Spectral norm:** `12.546×` baseline → `10.118×` after scalar-FP allocation/relay work.
- **Struct copy:** `34.112×` baseline → `18.353×` after aggregate inlining and memcpy forwarding.

## Correctness and rejected experiments

- The latest accepted paired rows above are **18/18 stdout matches**.
- Rust unit baseline before the final handoff: **575 passed, 0 failed, 6 ignored**.
- The direct scalar-FP XMM spike was rejected because it produced incorrect `nbodymandelbrot` results despite promising isolated timings.
- The broad aggregate memcpy-forwarding prototype was rejected and removed because it produced an incorrect zero result; only the narrower memcpy-chain and load-forwarding transformations are retained.
- No result from a correctness-mismatching experiment is included in the aggregate metrics.

## Measurement environment

- CPU: Intel Xeon Processor @ 2.60 GHz, KVM guest; 1 physical vCPU / 2 SMT-visible CPUs.
- This is **not** the target Intel Core i7-14700KF/Raptor Lake platform.
- GCC: Debian GCC 14.2.0, `-O2`.
- LCCC generated binaries: `-O2`.
- LCCC compiler build: `CARGO_PROFILE_RELEASE_OPT_LEVEL=1 CARGO_BUILD_JOBS=2 cargo build --release -j2`.
- Affinity: `taskset -c 0`.
- `perf`: unavailable in the sandbox; no hardware-counter claims are made.
- Clang, ICC, and ICX were not installed; this report is GCC-only.
- Timing uses wall-clock best-of-N measurements from `tests/benchmark/run_benchmarks.py`.

## Benchmark data provenance

| Data file | Role | Repetitions |
|---|---|---:|
| `reports/baseline-full-corrected-2026-08-10.json` | Full 18-workload corrected baseline | 3 |
| `reports/ipcp-eval-2026-08-10.json` | Closed-form/IPCP recursion update | 3 |
| `reports/scalar-fp-shape2-2026-08-10.json` | Scalar-FP function-shape update | 2 |
| `reports/xmm-relay-fold-2026-08-10.json` | Latest FP relay update for P0 FP workloads | 2 |
| `reports/sib-sieve2-2026-08-10.json` | Latest SIB/LEA sieve update | 2 |
| `reports/memcpy-load-forward-2026-08-10.json` | Latest struct-copy update | 2 |

## Reproduction commands

```bash
# Build compiler under the low-memory policy
source "$HOME/.cargo/env"
export CARGO_BUILD_JOBS=2 CARGO_PROFILE_RELEASE_OPT_LEVEL=1
cargo build --release -j2

# Full suite; generated programs use LCCC/GCC -O2
taskset -c 0 python3 tests/benchmark/run_benchmarks.py --reps 3 \
  --json reports/new-run.json
```

## Patch

All accepted source, regression-test, and benchmark-runner changes are combined in:

```text
../ms178-1.patch
```

Performance claims on the requested i7-14700KF still require rerunning the suite on that physical target with hardware counters and controlled frequency/SMT affinity.
