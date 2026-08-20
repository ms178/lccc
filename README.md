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
| **LCCC (ms178)** | [`f874a6fb`](https://github.com/ms178/lccc/commit/f874a6fb) | this fork, `main` after PR #70 (store/load forwarding, ARM memcpy fix, regalloc perf fix) |
| **LCCC (Lev)** | [`9beb4333`](https://github.com/levkropp/lccc/commit/9beb4333) | [levkropp/lccc](https://github.com/levkropp/lccc) `main` HEAD at measurement time |
| **GCC** | 14.2.0 (Debian 14.2.0-19) | external reference |

All compilers at `-O2`, identical sources, identical machine, same run window.
Driver: `bench/run_bench.sh`-style best-of-3 wall time; every binary's output
is compared against the GCC baseline (a mismatch disqualifies the number).

## Results (seconds, lower is better)

| Kernel | What it stresses | ms178 `f874a6fb` | GCC 14.2 | Lev `9beb4333` | ms178 vs GCC | ms178 vs Lev |
|---|---|---:|---:|---:|---:|---:|
| fib(38) | binary recursion → TCE/rec2iter | **0.003** | 0.068 | 0.352 | **22.7× faster** | **117× faster** |
| csum (adler-like) | serial dependency chains | 0.192 | 0.104 | 0.215 | 1.8× slower | 1.12× faster |
| hashloop (FNV) | int mul/xor + table traffic | 0.159 | 0.083 | 3.304 | 1.9× slower | **21× faster** |
| sieve (3×10M) | branchy int loops, stores | 0.209 | 0.166 | 0.304 | 1.26× slower | 1.45× faster |
| reduction (dot) | FP reduction vectorization | 1.273 | 0.593 | 1.293 | 2.1× slower | 1.02× faster |
| nbody (3M steps) | scalar FP, sqrt, locals | 0.984 | 0.260 | 1.147 | 3.8× slower | 1.17× faster |
| matmul (384³ ikj) | loop-nest FP, addressing | 0.287 | 0.055 | *wrong result* ⚠ | 5.2× slower | n/a ⚠ |
| spectral (N=800) | FP div/sqrt inner loops | 0.200 | 0.044 | 0.422 | 4.5× slower | 2.11× faster |

⚠ Lev `9beb4333` **miscompiles** the matmul kernel (checksum 159 ≠ 0): his
reduction vectorizer lacks this fork's IV-dependent-GEP-base guard, so the
diagonal-sum consumer reads out-of-bounds-adjacent data. The same latent bug
class previously **segfaulted** on this fork too and was fixed in PR #69
(`tests/regression/vectorize_iv_dependent_base.c`).

## Reading the table honestly

- **vs Lev**: this fork is faster on every kernel that Lev compiles correctly
  (1.02×–117×), and correct where he is not (matmul). The 21×/117× outliers are
  optimizations Lev's tree lacks entirely (FNV loop codegen, rec2iter + TCE).
- **vs GCC**: one structural win (recursion folding, 23×), near-parity on
  branchy int code (sieve 1.26×), and a real 2–5× deficit concentrated in FP
  code. Root causes are known and unchanged: accumulator round-trips in FP
  codegen, no x86 FMA contraction, and no general loop-nest vectorization
  (GCC vectorizes matmul's jk plane; LCCC only the reduction idiom).
- Every number above was produced by a **correct** binary; correctness is
  enforced before speed is recorded.

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
