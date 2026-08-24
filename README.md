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

## Engineering docs

All live performance/RA/codegen research lives in [`engineering/`](engineering/README.md).
Start at [`engineering/agent/README.md`](engineering/agent/README.md).

## Current documentation

Live docs (August 2026): `docs/getting-started.md`, `docs/architecture.md`,
`docs/register-allocator.md`, `lccc-improvements/register-allocation/`
(including `VALIDATION_ZLIB_GZIP_EXPAT.md`). `docs/history/` and
`lccc-improvements/benchmarks/BENCHMARK_RESULTS*.md` are archives.
Canonical bench: `tests/benchmark/run_benchmarks.py`. Repo:
https://github.com/ms178/lccc


# LCCC Performance Report

> **Screening evidence.** Wall-clock, best-of-3, checksums verified against a
> GCC baseline byte-for-byte. Measured in a shared 2-core VM (no PMU); numbers
> rank code-generation work and are *not* microarchitectural claims. Reproduce
> on bare metal before treating any figure as a hardware result.

## Compared revisions

| Compiler | Revision | Notes |
|---|---|---|
| **LCCC (ms178)** | `main` `daf3f48` + session-69 patch (this tree) | segment-aware regalloc, stencil vectorizer, FpContract tri-state, FP return/dest-aliasing fixes, flag-lifetime peepholes, PIC table-base hoist, x86-64 magic-const hoisting |
| **GCC** | 14.2.0 (Debian 14.2.0-19) | external reference |

All compilers at `-O2`, identical sources, identical machine, same run window.
Driver: `tests/benchmark/run_benchmarks.py` (canonical runner): paired median
wall time, randomized compiler order, warm-ups excluded; every binary's
output is checksummed against the GCC baseline (a mismatch disqualifies the
number). Measured 2026-08-24 in the shared 2-core VM (no PMU); screening
evidence only — reproduce on bare metal before treating any figure as a
hardware result.

## Results (ratio = LCCC/GCC; < 1 means LCCC faster)

| Kernel | What it stresses | LCCC/GCC | Verdict |
|---|---|---:|---|
| `fib` | binary recursion → rec2iter | **0.009** (109× faster) | pass |
| `constant_recursion` | constant recursive specialization | **0.017** (59× faster) | pass |
| `ackermann` | deep recursion | **0.019** (53× faster) | pass |
| `bitops` | integer selection, popcount idioms | **0.603** (1.7× faster) | pass |
| `gzip_crc32` | gzip CRC-32 table loop | **0.872** (1.15× faster) | pass |
| `ring_fifo` | dependent loads | **0.925** | pass |
| `matmul` | loop-nest FP + reduction FMA | **0.944** | pass |
| `tce_sum` | tail-recursive accumulator | **0.967** | pass |
| `double_reduction` | two independent FP reductions | **0.969** | pass |
| `qsort` | libc branches | **0.973** | pass |
| `switch_dispatch` | switch lowering | 1.023 | pass |
| `ascii_case_fold` | parser byte loop | 1.028 | pass |
| `glibc_memcmp` | aligned-word memcmp path | 1.031 | pass |
| `binary_search` | branch-heavy lookup | 1.032 | pass |
| `strlen_bench` | byte loops | 1.040 | pass |
| `arith_loop` | 32-variable register pressure | 1.049 | pass |
| `histogram` | indexed increment/reduction | 1.057 | pass |
| `binary_trees` | allocation + recursion | 1.064 | pass |
| `hash_table` | pointer chasing | 1.068 | pass |
| `sieve` | branchy int stores | 1.304 | pass |
| `sqlite_varint` | varint decoder | 1.361 | pass |
| `struct_copy` | aggregate copy + ABI | 1.493 | pass |
| `zlib_ng_adler32` | Adler-32 NMAX accumulator | 1.580 | pass |
| `expat_xml_scan` | XML name-token scan | 1.609 | pass |
| `mandelbrot` | FP branch-heavy loop | 1.653 | pass |
| `fannkuch` | permutations | 1.784 | pass |
| `linux_find_bit` | sparse bit search | 1.809 | pass |
| `loop_patterns` | scalar loop transforms | 2.435 | pass |
| `nbody` | N-body FP structs | 3.889 | pass |
| `spectral_norm` | dense FP | 4.779 | pass |

**Aggregate (30 correct pairs): geometric mean 0.8235.** Excluding the three
algorithmic recursion wins, the conventional-code geomean is **1.29**.

## Reading the table honestly

- **Where LCCC wins**: recursion folding (`fib`/`ackermann`/
  `constant_recursion` — TCE + rec2iter; GCC keeps exponential recursion),
  popcount recognition (`bitops` 1.7× via `popcntl`), reduction FMA
  vectorization (`matmul`), the CRC table loop (hoisted PIC base + magic-const
  hoisting; 1.15×), and the whole cluster of integer/branch kernels in the
  0.92–1.07 band.
- **Where GCC still wins**: non-reduction dense FP (`spectral_norm` 4.8×,
  `nbody` 3.9×, `mandelbrot` 1.65× — general loop-nest vectorization is the
  tracked structural gap; the stencil vectorizer covers element-wise shapes
  only), and a codec/parser cluster (`loop_patterns` 2.4×, `find_bit` 1.8×,
  `fannkuch` 1.8×, `expat` 1.6×, `adler32` 1.6× — RA-06 arithmetic-chain copy
  webs and secondary-IV strength reduction are the mapped fixes; see
  [`engineering/agent/BACKLOG.md`](engineering/agent/BACKLOG.md) §15.5).
- Every number above was produced by a **correct** binary; correctness is
  enforced before speed is recorded (checksums byte-identical to GCC).

## Where LCCC wins

- **Tail-call elimination (TCE)** and **binary recursion-to-iteration**:
  fib-class recursion runs ~23× faster than GCC (GCC keeps the exponential
  recursion).
- **Reduction vectorization**: simple `sum`/`dot` reductions go 4-wide AVX2
  where GCC `-O2` stays scalar.
- **Profile-guided optimization**: `-fprofile-generate`/`-fprofile-use` fully
  integrated (inlining, unrolling, layout, switch lowering) with no
  PGO-induced regressions on the workload kernels.

## Root cause of the remaining gap vs GCC

The 2–5× FP deficit is concentrated in three mechanisms, in priority order:

1. **FP values round-trip through the accumulator** (`%rax`/`x0`) on cast and
   copy paths instead of staying in SSE/NEON registers.
2. **No FMA contraction on x86** — GCC fuses `a*b+c` into `vfmadd*`;
   LCCC emits separate mul/add (AArch64 already fuses via `fmadd`).
3. **No loop-nest vectorization** — only the innermost reduction idiom is
   vectorized; matmul/spectral/nbody inner loops stay scalar.

## Future work (see `hotspots/` and `ideas/`)

- Struct-by-value ABI and wide (vectorized) aggregate copies.
- Broadening the auto-vectorizer (non-reduction FP loops, FMA fusion).
- Instruction scheduling for the Raptor Lake port/resource model.
- Sample-based PGO and PGO value specialization.
- Use-def-chain shared optimizer context.
