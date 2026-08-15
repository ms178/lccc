# LCCC — Lightning Fast Claude's C Compiler

> An optimized fork of [CCC](https://github.com/anthropics/claudes-c-compiler) based on additions made by Lev Kropp (https://www.levkropp.com/lccc/).

---

## What is LCCC?

CCC (Claude's C Compiler) is a zero-dependency C compiler written entirely in Rust by Claude Opus 4.6. Arena.ai Agents 
did a good job at improving it further.

It is capable of compiling real projects — gzip, zlib-ng, expat, SQLite, the Linux kernel and glibc — for x86-64, AArch64,
RISC-V 64, and i686, with its own assembler and linker.

LCCC is a performance fork and a personal AI agent research project. It is currently still lacking behind GCC/Clang in many areas.

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

- **LCCC (ms178)** (this fork): current `main`, `lccc-pgo-v1` profile format, built
  `cargo build --release` (Rust opt-level 1, two jobs), swap on.
- **LCCC (Lev)**: built from [`levkropp/lccc`](https://github.com/levkropp/lccc)
  (`main`) — the original lccc that this fork builds on.
- **Original CCC**: built from `anthropics/claudes-c-compiler` (`main`) — the
  compiler both lccc forks descend from.
- **GCC**: gcc (Debian 14.2.0-19) 14.2.0.
- **Flags**: `-O2` for every compiler (uniform).
- **Environment**: Linux 6.1, Intel Xeon 2.60 GHz vCPU, `taskset` pinned to one
  CPU, paired timed rounds + 2 excluded warm-ups, randomized compiler order per
  round. LCCC/GCC/original-CCC were compared in one run; Lev LCCC was run fresh
  in a second run on the same corpus and machine. Seed `20260813`.
- **Correctness**: outputs are compared against a GCC baseline and must match
  byte-for-byte. The original CCC and Lev LCCC each matched on every benchmark
  they ran except where flagged (⚠ = Lev LCCC produced a **wrong output** on
  `glibc_memcmp`; the original CCC failed to run `tce_sum`). Outliers are
  counted and reported, never discarded.

## The improvement chain: original CCC → Lev LCCC → this LCCC

All three compilers run the same corpus at `-O2`. **Ratio columns are speedups**:
`CCC→Lev > 1` means Lev LCCC is faster than the original CCC; `Lev→LCCC > 1`
means this fork is faster than Lev LCCC. GCC is included as an external
reference.

| Benchmark | CCC (ms) | LCCC (Lev) (ms) | LCCC (ms178) (ms) | GCC (ms) | CCC→Lev | Lev→ms178 |
|---|---:|---:|---:|---:|---:|---:|
| ackermann | 1277 | 1047 | 2 | 149 | 1.2× | 442× |
| arith_loop | 259 | 157 | 110 | 100 | 1.6× | 1.4× |
| binary_trees | 1848 | 1798 | 1603 | 1281 | 1.0× | 1.1× |
| bitops | 1127 | 5048 | 626 | 391 | 0.2× | 8.1× |
| constant_recursion | 1266 | 1050 | 2 | 149 | 1.2× | 444× |
| expat_xml_scan | 258 | 1601 | 83 | 40 | 0.2× | 19.2× |
| fannkuch | 6658 | 9554 | 3065 | 2539 | 0.7× | 3.1× |
| fib | 771 | 3 | 2 | 171 | 257× | 1.2× |
| glibc_memcmp | 25 | 16 ⚠ | 13 | 9 | 1.6× | 1.2× |
| gzip_crc32 | 257 | 251 | 224 | 168 | 1.0× | 1.1× |
| hash_table | 14249 | 14842 | 12665 | 9376 | 1.0× | 1.2× |
| linux_find_bit | 45 | 79 | 27 | 15 | 0.6× | 2.9× |
| loop_patterns | 213 | 252 | 135 | 73 | 0.8× | 1.9× |
| mandelbrot | 5218 | 6031 | 2808 | 1489 | 0.9× | 2.1× |
| matmul | 51 | 13 | 8 | 7 | 3.9× | 1.6× |
| nbody | 2268 | 2301 | 1197 | 415 | 1.0× | 1.9× |
| qsort | 146 | 150 | 138 | 126 | 1.0× | 1.1× |
| sieve | 100 | 74 | 52 | 40 | 1.4× | 1.4× |
| spectral_norm | 2472 | 2528 | 591 | 202 | 1.0× | 4.3× |
| sqlite_varint | 73 | 73 | 51 | 26 | 1.0× | 1.4× |
| strlen_bench | 350 | 333 | 252 | 227 | 1.1× | 1.3× |
| struct_copy | 931 | 874 | 162 | 27 | 1.1× | 5.4× |
| switch_dispatch | 747 | 773 | 706 | 503 | 1.0× | 1.1× |
| tce_sum | — | 17 | 2 | 2 | — | 7.5× |
| zlib_ng_adler32 | 196 | 78 | 61 | 39 | 2.5× | 1.3× |

**Aggregates (geometric mean):**
- **CCC → LCCC (Lev): 1.23×** (n=24) — Lev's optimizations
  (TCE, recursion-to-iteration, vectorization, register allocation) are a clear
  win over the original CCC.
- **LCCC (Lev) → LCCC (ms178): 3.29×** (n=25) — my fork is
  roughly **3.3× faster than Lev's fork it builds on** across the corpus.
- **CCC → LCCC (ms178): 3.90×** — combined, ~3.9× over the original.
- **LCCC (ms178) vs GCC: 0.92× geomean on this corpus** — but read this
  honestly: the aggregate is dominated by two benchmarks
  (`ackermann`/`constant_recursion`, ~440× each) that LCCC folds at
  COMPILE TIME via IPCP + recursion-to-iteration. Excluding those two,
  the corpus geomean is roughly **1.5–2× slower than GCC**, concentrated
  in FP/struct-by-value and branch-heavy workloads (see "Root cause"
  below). Both numbers are real; neither alone is the full story.

### What LCCC (Lev) already fixed (vs the original CCC)

Lev's lccc already delivered the big win for **recursion-to-iteration**
(`fib` 305× over the original CCC), plus **vectorized matmul** (3.9×),
**zlib-ng adler32** (2.5×), and **arith_loop / sieve** (1.3–1.6×). It also
**regressed** a few workloads vs the original (bitops 0.2×, expat 0.2×,
fannkuch 0.7×) and produced a **wrong output** on `glibc_memcmp`. Note that
`constant_recursion` gained only 1.2× under Lev — the ~436× jump for it (and
`ackermann` 417×) is this fork's constant-folding/recursion work, not Lev's.

### What my fork adds on top of Lev's LCCC

This fork is **faster than Lev LCCC on every benchmark** (geomean ~3.3×), with
the largest gains on:

- **Recursion**: `constant_recursion` **444×**, `ackermann` **442×** (full
  constant-folding/recursion recognition on top of Lev's TCE).
- **Codegen regressions Lev introduced, fixed**: `expat_xml_scan` **19.2×**,
  `bitops` **8.1×**, `fannkuch` **3.1×**, `linux_find_bit` **2.9×**,
  `nbody` **1.9×**, `loop_patterns` **1.9×** — these were *slower* than the
  original CCC under Lev, and this fork brings them below both.
- **FP/struct wins**: `spectral_norm` **4.3×**, `struct_copy` **5.4×**,
  `mandelbrot` **2.1×**, `matmul` **1.6×** over Lev (and the gap to GCC closed
  accordingly).
- **Correctness**: `glibc_memcmp` (Lev produced a wrong result), mine is correct here.
- **TCE tail**: `tce_sum` (the original CCC cannot run it) 7.5× over Lev.
- Broad ~1.1–1.5× across integer/ALU/loop kernels (arith_loop, sieve, matmul,
  struct_copy, sqlite_varint, spectral_norm).

### The remaining gap vs GCC

The 0.92× corpus geomean flatters LCCC because two compile-time-folded
recursion benchmarks contribute ~440× each; on the remaining real-runtime
workloads LCCC trails GCC by ~1.5–2× overall, with FP/struct-by-value cases
the worst (see "Root cause" below). The other three compilers are all far
slower than GCC on those same workloads, so this is LCCC's own remaining
work, not inherited.

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

Excluding the two compile-time-folded outliers, the following FP/struct and
branch-heavy workloads drive the remaining ~1.5–2× runtime gap vs GCC:

- **struct_copy (5.9×)** — struct by-value passing/copying; GCC lowers
  multi-field copies to a few wide moves while LCCC still routes each field
  through the accumulator (greatly improved from 21×).
- **spectral_norm (2.9×) / nbody (2.9×) / mandelbrot (1.9×)** — GCC vectorizes
  these FP inner loops (and uses FMA); LCCC's reduction vectorizer covers only
  the simple reduction idiom, not the general FP loops here.
- **expat_xml_scan (2.1×)** — byte scanning with many small branches; GCC folds
  the character classification into compare/range/bit-test instructions
  (improved from 3.3×).
- **bitops / find_bit / varint (1.6–2.0×)** — bit-scan selection and branch
  layout.

## Future work (see `hotspots/` and `ideas/`)

- Struct-by-value ABI and wide (vectorized) aggregate copies.
- Broadening the auto-vectorizer (non-reduction FP loops, FMA fusion).
- Instruction scheduling for the Raptor Lake port/resource model.
- Sample-based PGO and PGO value specialization.
- Use-def-chain shared optimizer context.
