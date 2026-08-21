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
| **LCCC (ms178)** | [`bc992d69`](https://github.com/ms178/lccc/commit/bc992d69) | this fork, `main` `aa8bc98` (PR #178) + session 41/42 codegen fixes (SIB zero-extend tracking, immediate `imul`, BT direct index, 32-bit-write tracking, alias-aware SIMD temp promotion) |
| **GCC** | 14.2.0 (Debian 14.2.0-19) | external reference |

All compilers at `-O2`, identical sources, identical machine, same run window.
Driver: `tests/benchmark/run_benchmarks.py` (canonical runner). Paired
median wall time, randomized compiler order, warm-ups excluded; every binary's
output is checksummed against the GCC baseline (a mismatch disqualifies the
number). Measured 2026-08-21 in the shared 2-core VM (no PMU); screening
evidence only.

## Results (ratio = LCCC/GCC; < 1 means LCCC faster)

| Kernel | What it stresses | LCCC median | GCC median | LCCC/GCC | Correct |
|---|---|---:|---:|---:|:---:|
| `fib` | binary recursion → TCE/rec2iter | 2.52 ms | 173.02 ms | **0.014** (69× faster) | pass |
| `ackermann` | deep recursion | 2.48 ms | 150.72 ms | **0.016** (61× faster) | pass |
| `constant_recursion` | constant recursive specialization | 2.51 ms | 150.87 ms | **0.016** (61× faster) | pass |
| `bitops` | integer selection, popcount idioms | 260.56 ms | 392.50 ms | **0.664** (1.5× faster) | pass |
| `matmul` | loop-nest FP + reduction FMA | 7.75 ms | 9.71 ms | **0.840** (1.2× faster) | pass |
| `binary_search` | branch-heavy lookup | 2.28 ms | 2.37 ms | **0.952** | pass |
| `ascii_case_fold` | parser byte loop | 2.46 ms | 2.41 ms | 0.995 | pass |
| `ring_fifo` | dependent loads | 2.25 ms | 2.24 ms | 0.997 | pass |
| `qsort` | libc branches | 127.54 ms | 126.64 ms | 1.010 | pass |
| `strlen_bench` | byte loops | 235.44 ms | 233.14 ms | 1.013 | pass |
| `tce_sum` | tail-recursive accumulator | 2.24 ms | 2.14 ms | 1.052 | pass |
| `switch_dispatch` | switch lowering | 528.84 ms | 504.43 ms | 1.052 | pass |
| `gzip_crc32` | gzip CRC-32 table loop | 179.92 ms | 168.86 ms | 1.067 | pass |
| `arith_loop` | 32-variable register pressure | 112.85 ms | 101.74 ms | 1.112 | pass |
| `histogram` | indexed increment/reduction | 2.88 ms | 2.52 ms | 1.123 | pass |
| `hash_table` | pointer chasing | 12.23 s | 10.77 s | 1.129 | pass |
| `binary_trees` | allocation + recursion | 1.639 s | 1.426 s | 1.147 | pass |
| `sieve` | branchy int stores | 46.26 ms | 37.82 ms | 1.224 | pass |
| `glibc_memcmp` | aligned-word memcmp path | 11.75 ms | 9.22 ms | 1.275 | pass |
| `fannkuch` | permutations | 3.314 s | 2.542 s | 1.303 | pass |
| `mandelbrot` | FP branch-heavy loop | 2.107 s | 1.489 s | 1.418 | pass |
| `struct_copy` | aggregate copy + ABI | 41.36 ms | 27.42 ms | 1.514 | pass |
| `zlib_ng_adler32` | Adler-32 NMAX accumulator | 63.07 ms | 39.41 ms | 1.588 | pass |
| `loop_patterns` | scalar loop transforms | 127.44 ms | 78.21 ms | 1.629 | pass |
| `linux_find_bit` | sparse bit search | 25.24 ms | 14.78 ms | 1.697 | pass |
| `sqlite_varint` | varint decoder | 44.92 ms | 26.09 ms | 1.718 | pass |
| `expat_xml_scan` | XML name-token scan | 78.44 ms | 40.47 ms | 1.962 | pass |
| `nbody` | N-body FP structs | 984.30 ms | 305.55 ms | 3.221 | pass |
| `spectral_norm` | dense FP | 791.51 ms | 202.02 ms | 3.908 | pass |

**Aggregate (29 correct pairs): geometric mean 0.8205, arithmetic mean 1.2640.**
Best `fib` 0.0144; worst `spectral_norm` 3.908.

## Reading the table honestly

- **Where LCCC wins**: recursion folding (`fib`/`ackermann`/
  `constant_recursion` — TCE + rec2iter, 60–69× vs GCC's exponential
  recursion), hand-rolled popcount recognition (`bitops` 1.5× via `popcntl`),
  and reduction FMA vectorization (`matmul` 1.2×). These lift the geomean
  below 1.0 even though several codec/parser kernels trail.
- **Where GCC wins**: dense/non-reduction FP (`spectral_norm` 3.9×,
  `nbody` 3.2× — no general loop-nest vectorization, no FMA contraction,
  accumulator round-trips), and a cluster of parser/codec kernels in the
  1.5–2.0× band (`expat_xml_scan` 1.96×, `sqlite_varint` 1.72×,
  `linux_find_bit` 1.70×, `loop_patterns` 1.63×, `zlib_ng_adler32` 1.59×).
  These are the tracked structural gaps — see
  [`engineering/agent/BACKLOG.md`](engineering/agent/BACKLOG.md) (segment
  register allocation, accumulator-centric ISel, SysV aggregate ABI, Expat
  single-table classify).
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
