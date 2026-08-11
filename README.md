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

# LCCC v3 Performance & Size Report

## Baseline Fingerprints

- **GCC**: gcc (GCC) 16.1.1 20260803 (CachyOS), x86_64-pc-linux-gnu
- **Rust**: rustc 1.97.1 (8bab26f4f 2026-07-14)
- **LCCC**: v3 build from dd99046c + ms178-1.patch (100 lines), release -O2 -j2
- **Hardware target**: Intel i7-14700KF (Raptor Lake), no AVX-512

## A/B Benchmark: LCCC v3 vs GCC 16.1.1 (7 rounds, -O3 -march=raptorlake)

| Benchmark | LCCC (ms) | GCC (ms) | Ratio | Δ vs baseline |
|---|---:|---:|---:|---|
| arith_loop | 107 | 104 | 1.04× | -7% |
| fib | 2.5 | 151 | 0.017× (58×f) | — |
| matmul | 9.0 | 5.0 | 1.83× | -3% |
| sieve | 56 | 37 | 1.46× | -4% |
| tce_sum | 2.1 | 1.8 | 1.19× | — |
| **nbody** | **1197** | 229 | **5.23×** | **-36%!** |
| binary_trees | 1647 | 1400 | 1.18× | — |
| **spectral_norm** | **1878** | 196 | **9.57×** | **-26%!** |
| **mandelbrot** | **4760** | 1123 | **4.23×** | **-11%!** |
| hash_table | 12354 | 9641 | 1.26× | — |
| strlen_bench | 256 | 223 | 1.15× | — |
| switch_dispatch | 741 | 494 | 1.50× | — |
| struct_copy | 585 | 42 | 14.02× | — |
| loop_patterns | 134 | 68 | 2.00× | — |
| fannkuch | 3068 | 2864 | 1.07× | — |
| ackermann | 2.4 | 149 | 0.016× (61×f) | — |
| constant_recursion | 2.4 | 149 | 0.016× (61×f) | — |
| bitops | 754 | 318 | 2.35× | — |
| gzip_crc32 | 244 | 168 | 1.46× | — |
| zlib_ng_adler32 | 67 | 39 | 1.75× | — |
| expat_xml_scan | 130 | 31 | 4.14× | — |
| sqlite_varint | 54 | 23 | 2.38× | — |
| linux_find_bit | 26 | 12 | 2.18× | — |
| glibc_memcmp | 12 | 10 | 1.15× | — |

**Geometric mean**: 1.16× slower (improved from 1.20× baseline)

**Key wins**: nbody -36%, spectral_norm -26%, mandelbrot -11%, arith_loop -7%

## Optimizations Applied in v3

1. **RHS XMM register direct**: When RHS of FP binop is in xmm3-xmm7, operate directly
   (saves 2-3 instructions per FP chain operation)
2. **AVX2 memcpy 32B**: vmovdqu ymm0 replaces 2x movdqu xmm0 (halves uops)
3. **FP regression test**: fp_domain_crossing.c (6 categories, all PASS)

## Root Cause of Remaining Gap

The primary remaining gap is **no auto-vectorization**. GCC uses AVX2 4-wide double
operations (vdivpd, vmulpd, vaddsd with ymm registers) for FP reductions. LCCC is
fully scalar. This accounts for:

- spectral_norm 9.57× (GCC vectorizes the inner loop 4-wide)
- nbody 5.23× (GCC vectorizes the force computation)
- mandelbrot 4.23× (GCC vectorizes the iteration loop)

## Future Work (v4+)

- Auto-vectorizer for FP reductions (~5-10× on FP benchmarks)
- LHS XMM direct with proper register cache invalidation
- Loop unrolling pass
- Struct copy / ABI optimization (14× gap)
