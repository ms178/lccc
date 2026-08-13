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

**Workload-derived benchmark kernels** — retain the per-file upstream license
and provenance (for example GPL/LGPL/MIT/Zlib/public-domain material); they
are documented in [`tests/benchmark/WORKLOAD_PROVENANCE.md`](tests/benchmark/WORKLOAD_PROVENANCE.md).

See [`LICENSING.md`](LICENSING.md) for the full breakdown and per-file guidance.

# LCCC Performance Report

> **Screening evidence.** The numbers below were produced by the canonical
> runner [`tests/benchmark/run_benchmarks.py`](tests/benchmark/run_benchmarks.py)
> in a KVM-exposed VM (`hypervisor_detected: true`) with **no PMU**; they rank
> code-generation work and are *not* microarchitectural claims. Reproduce on a
> bare-metal Intel i7-14700KF with PMU evidence before treating any number as a
> hardware result.

## Baseline fingerprints

- **LCCC**: current `main`, `lccc-pgo-v1` profile format, built
  `cargo build --release` (Rust opt-level 1, two jobs), swap on.
- **GCC**: gcc (Debian 14.2.0-19) 14.2.0.
- **Original CCC**: built from `anthropics/claudes-c-compiler` (`main`) with the
  same Rust policy, `-O2`.
- **Flags**: `-O2` for every compiler (uniform).
- **Environment**: Linux 6.1, Intel Xeon 2.60 GHz vCPU, `taskset` pinned to one
  CPU, 5 paired timed rounds + 2 excluded warm-ups, randomized compiler order
  per round, seed `20260813`.
- **Correctness**: all 25 reported benchmarks produced **byte-identical output**
  between LCCC and GCC. The original CCC matched both on every benchmark it
  ran; `tce_sum` failed to run under the original compiler. Outliers are counted
  and reported, never discarded.

## A/B benchmark: LCCC vs GCC -O2

| Benchmark | LCCC (ms) | GCC (ms) | LCCC/GCC |
|---|---:|---:|---:|
| fib | 2.6 | 175.0 | **0.015× (67× faster)** |
| ackermann | 2.5 | 151.7 | **0.016× (60× faster)** |
| constant_recursion | 2.4 | 149.1 | **0.016× (62× faster)** |
| tce_sum | 2.2 | 2.2 | 0.97× |
| qsort | 138.7 | 125.6 | 1.10× |
| strlen_bench | 282.1 | 260.8 | 1.08× |
| arith_loop | 113.1 | 101.6 | 1.13× |
| matmul | 9.2 | 7.5 | 1.22× |
| fannkuch | 3235 | 2608 | 1.24× |
| binary_trees | 1714 | 1416 | 1.22× |
| glibc_memcmp | 11.6 | 8.9 | 1.30× |
| sieve | 51.4 | 42.1 | 1.23× |
| gzip_crc32 | 244.1 | 168.3 | 1.45× |
| switch_dispatch | 742.1 | 508.1 | 1.46× |
| zlib_ng_adler32 | 67.6 | 38.9 | 1.74× |
| loop_patterns | 133.7 | 74.4 | 1.81× |
| linux_find_bit | 26.3 | 14.5 | 1.81× |
| bitops | 745.1 | 390.0 | 1.91× |
| sqlite_varint | 54.2 | 26.0 | 2.07× |
| mandelbrot | 4760 | 1489 | 3.20× |
| expat_xml_scan | 131.2 | 40.6 | 3.24× |
| nbody | 1218 | 313 | 3.89× |
| spectral_norm | 1880 | 202 | 9.29× |
| struct_copy | 583.7 | 27.5 | 21.18× |

**Aggregate (n = 25 correct pairs):** geometric mean **1.08×**, arithmetic mean
2.60× (pulled up by the struct_copy outlier). LCCC is **faster than GCC on
tail-recursion and constant-recursion** workloads (fib/ackermann ~60–67× via
TCE + recursion-to-iteration), at parity on `tce_sum`, and within ~1.1–1.5× of
GCC on most integer/ALU/loop kernels.

## A/B benchmark: LCCC vs the original Claude's C Compiler

The same corpus was compiled with the **original upstream CCC** (before LCCC's
optimizations), so the table shows what LCCC improved over its ancestor.
`LCCC/CCC < 1` means LCCC is faster.

| Benchmark | CCC (ms) | LCCC/CCC |
|---|---:|---:|
| fib | 770.8 | **0.003× (~300× faster)** |
| constant_recursion | 1266.3 | **0.002× (~520×)** |
| ackermann | 1277.1 | **0.002× (~510×)** |
| matmul | 50.7 | **0.179× (5.6×)** |
| zlib_ng_adler32 | 196.2 | **0.345× (2.9×)** |
| arith_loop | 259.5 | **0.438× (2.3×)** |
| glibc_memcmp | 24.5 | **0.475× (2.1×)** |
| fannkuch | 6658 | **0.486× (2.1×)** |
| expat_xml_scan | 258.5 | **0.513× (1.95×)** |
| sieve | 99.9 | **0.519× (1.9×)** |
| nbody | 2268 | **0.547× (1.8×)** |
| linux_find_bit | 44.6 | **0.586× (1.7×)** |
| struct_copy | 930.8 | **0.626× (1.6×)** |
| loop_patterns | 213.0 | **0.625× (1.6×)** |
| bitops | 1126.7 | **0.661× (1.5×)** |
| sqlite_varint | 73.4 | **0.740× (1.35×)** |
| spectral_norm | 2472.5 | **0.761× (1.3×)** |
| strlen_bench | 350.2 | **0.805× (1.2×)** |
| hash_table | 14249 | **0.889× (1.1×)** |
| mandelbrot | 5218 | **0.912× (1.1×)** |
| binary_trees | 1848 | **0.944× (1.06×)** |
| qsort | 146.2 | **0.948× (1.05×)** |
| gzip_crc32 | 256.5 | **0.952× (1.05×)** |
| switch_dispatch | 747.4 | **0.993× (~1.0×)** |
| tce_sum | *(original CCC failed to run)* | — |

LCCC is **faster than the original CCC on every benchmark it can run**. The
gains come from LCCC's work: **tail-call elimination** and
**recursion-to-iteration** (fib/ackermann/constant_recursion — the original CCC
executes the exponential recursion, hence ~500×), **AVX2/SSE2 vectorization and
FMA** (matmul, reductions, nbody, spectral_norm), **strength reduction /
register allocation / IVSR** (arith_loop, loop_patterns, sieve), and
**profile-guided inlining and cost-aware devirtualization** (expat, zlib-ng).

## Where LCCC wins

- **Tail-call elimination (TCE)** and **binary recursion-to-iteration**: fib,
  ackermann, constant_recursion — 60–70× faster than GCC (GCC keeps the
  exponential recursion).
- **Reduction vectorization**: LCCC vectorizes simple `sum`/`dot` reductions to
  4-wide AVX2 where GCC `-O3` leaves them scalar (~2.7× faster on the reduction
  kernel).
- **Profile-guided optimization**: `-fprofile-generate`/`-fprofile-use` is fully
  integrated (inlining, unrolling, layout, switch lowering, cost-aware
  devirtualization) with **no PGO-induced regressions** on the workload kernels.

## Root cause of the remaining gap

The largest remaining gaps are FP/struct-by-value and branch-heavy byte scanning:

- **struct_copy (21×)** — struct by-value passing/copying is the single biggest
  gap; GCC lowers multi-field copies to a few wide moves while LCCC routes each
  field through the accumulator.
- **spectral_norm (9.3×) / nbody (3.9×) / mandelbrot (3.2×)** — GCC vectorizes
  these FP inner loops (and uses FMA); LCCC's reduction vectorizer covers only
  the simple reduction idiom, not the general FP loops here.
- **expat_xml_scan (3.3×)** — byte scanning with many small branches; GCC folds
  the character classification into compare/range/bit-test instructions.
- **bitops / find_bit / varint (1.8–2.1×)** — bit-scan selection and branch
  layout.

## Planned work (see `hotspots/` and `ideas/`)

- Struct-by-value ABI and wide (vectorized) aggregate copies.
- Broadening the auto-vectorizer (non-reduction FP loops, FMA fusion).
- Instruction scheduling for the Raptor Lake port/resource model.
- Sample-based PGO and PGO value specialization.
- Use-def-chain shared optimizer context.
