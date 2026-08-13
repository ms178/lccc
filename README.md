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

- **LCCC**: revision `77b4a18c` (+ current working tree, `lccc-pgo-v1` profile
  format), built `cargo build --release` (Rust opt-level 1, two jobs), swap on.
- **GCC**: gcc (Debian 14.2.0-19) 14.2.0.
- **Flags**: `-O2` for both compilers (uniform).
- **Environment**: Linux 6.1, Intel Xeon 2.60 GHz vCPU, `taskset` pinned to one
  CPU, 9 paired timed rounds + 2 excluded warm-ups, randomized compiler order
  per round, seed `20260813`.
- **Correctness**: all 25 reported benchmarks produced **byte-identical output**
  between LCCC and GCC; outliers are counted and reported, never discarded.

## A/B benchmark: LCCC vs GCC -O2

| Benchmark | LCCC (ms) | GCC (ms) | LCCC/GCC |
|---|---:|---:|---:|
| fib | 2.4 | 170.3 | **0.014× (70× faster)** |
| ackermann | 2.5 | 152.6 | **0.016× (63× faster)** |
| constant_recursion | 2.4 | 150.1 | **0.016× (62× faster)** |
| tce_sum | 2.1 | 2.1 | **0.98×** |
| qsort | 138.5 | 125.7 | 1.09× |
| strlen_bench | 241.4 | 222.0 | 1.09× |
| arith_loop | 109.5 | 99.9 | 1.10× |
| matmul | 8.5 | 7.1 | 1.21× |
| fannkuch | 3073 | 2549 | 1.20× |
| hash_table | 13164 | 10407 | 1.23× |
| binary_trees | 1536 | 1243 | 1.23× |
| glibc_memcmp | 12.3 | 9.4 | 1.35× |
| sieve | 48.6 | 35.4 | 1.36× |
| gzip_crc32 | 244.3 | 168.3 | 1.45× |
| switch_dispatch | 739.7 | 502.4 | 1.47× |
| zlib_ng_adler32 | 68.9 | 39.3 | 1.73× |
| loop_patterns | 134.9 | 77.3 | 1.74× |
| linux_find_bit | 27.7 | 15.2 | 1.83× |
| bitops | 747.5 | 392.0 | 1.90× |
| sqlite_varint | 54.7 | 26.4 | 2.06× |
| mandelbrot | 4747 | 1490 | 3.19× |
| expat_xml_scan | 134.1 | 40.7 | 3.29× |
| nbody | 1187 | 308 | 3.88× |
| spectral_norm | 1878 | 201 | 9.34× |
| struct_copy | 585.5 | 27.9 | 21.06× |

**Aggregate (n = 25 correct pairs):** geometric mean **1.08×**, arithmetic mean
2.59× (pulled up by the struct_copy outlier). LCCC is **faster than GCC on
tail-recursion and constant-recursion** workloads (fib/ackermann ~60–70× via
TCE + recursion-to-iteration), at parity on `tce_sum`, and within ~1.1–1.5× of
GCC on most integer/ALU/loop kernels.

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
