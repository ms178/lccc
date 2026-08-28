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

## Documentation

- **Build & quickstart:** [`docs/getting-started.md`](docs/getting-started.md)
- **Architecture:** [`docs/architecture.md`](docs/architecture.md) · passes: [`docs/optimization-passes.md`](docs/optimization-passes.md) (tiers: `src/passes/README.md`)
- **Engineering home (live):** [`engineering/README.md`](engineering/README.md) — start at
  [`engineering/agent/README.md`](engineering/agent/README.md). Current state:
  [`engineering/STATE.md`](engineering/STATE.md); work queue:
  [`engineering/tasks/`](engineering/tasks/README.md); item catalog:
  [`engineering/agent/BACKLOG.md`](engineering/agent/BACKLOG.md);
  decision & negative-results ledger:
  [`engineering/DECISIONS.md`](engineering/DECISIONS.md).
- **Benchmarks:** [`docs/benchmarks.md`](docs/benchmarks.md) · canonical runner:
  `tests/benchmark/run_benchmarks.py`

Repo: https://github.com/ms178/lccc

# LCCC Performance Report

> **Screening evidence.** Wall-clock, paired medians, checksums verified against a
> GCC baseline byte-for-byte. Measured in a shared 2-core VM (no PMU); numbers
> rank code-generation work and are *not* microarchitectural claims. Reproduce
> on bare metal (the 14700KF target) before treating any figure as a hardware result.

## Compared revisions

| Compiler | Revision | Notes |
|---|---|---|
| **LCCC (ms178)** | `main` + sessions 73–83 (this tree) | segment-aware regalloc, general complete unrolling, VEX 3-operand scalar-FP, widening + masked conditional-sum reduction vectorization, stencil/map vectorizers, FMA3 ISA gate, tri-state FP contraction, DSE, backedge PRE, opt-in loop rotation |
| **GCC** | 14.2.0 (Debian 14.2.0-19) | external reference |

All compilers at `-O2`, identical sources, identical machine, same run window.
Driver: `tests/benchmark/run_benchmarks.py` (canonical runner): paired median
wall time, randomized compiler order, warm-ups excluded; every binary's
output is checksummed against the GCC baseline (a mismatch disqualifies the
number). Measured 2026-08-25; historical session-by-session deltas live in
git history, distilled lessons in [`engineering/DECISIONS.md`](engineering/DECISIONS.md).

## Results (ratio = LCCC/GCC; < 1 means LCCC faster)

| Kernel | What it stresses | LCCC/GCC | Verdict |
|---|---|---:|---|
| `fib` | binary recursion → rec2iter | **0.009** (109× faster) | pass |
| `ackermann` | deep recursion | **0.022** (45× faster) | pass |
| `libm_round_family` | libm round intrinsics | **0.411** (2.43× faster) | pass |
| `matmul` | loop-nest FP + reduction FMA | **0.46** (2.17× faster) | pass |
| `bitops` | integer selection, popcount idioms | **0.603** (1.7× faster) | pass |
| `fp_memfold_stencil5` | FP stencil memory folding | **0.88** | pass |
| `hash_table` | pointer chasing | **0.92** | pass |
| `binary_trees` | allocation + recursion | **0.92** | pass |
| `qsort` | libc branches | **0.97** (1.03× faster) | pass |
| `tce_sum` | tail-recursive accumulator | **0.961** (1.04× faster) | pass |
| `reduction_vecreg` | register-resident FP reductions | **0.96** | pass |
| `glibc_memcmp` | aligned-word memcmp path | **1.00** | pass |
| `binary_search` | branch-heavy lookup | **1.00** | pass |
| `double_reduction` | two independent FP reductions | **1.00** | pass |
| `loop_patterns` | scalar loop transforms | **0.979** (1.02× faster) | pass |
| `gzip_crc32` | gzip CRC-32 table loop | **0.862** (1.16× faster) | pass |
| `histogram` | indexed increment/reduction | **0.80** (1.25× faster) | pass |
| `arith_loop` | 32-variable register pressure | **1.242** (1.24× slower) | pass |
| `mandelbrot` | FP branch-heavy loop | **1.234** (1.23× slower) | pass |
| `sieve` | branchy int stores | **1.258** (1.26× slower) | pass |
| `nbody` | N-body FP structs | **1.262** (1.26× slower) | pass |
| `sqlite_varint` | varint decoder | **1.360** (1.36× slower) | pass |
| `spectral_norm` | dense FP | **1.301** (1.30× slower) | pass |
| `fannkuch` | permutations | **1.391** (1.39× slower) | pass |
| `linux_find_bit` | sparse bit search | **1.441** (1.44× slower) | pass |
| `expat_xml_scan` | XML name-token scan | **1.686** (1.69× slower) | pass |
| `zlib_ng_adler32` | Adler-32 DO8 kernel | **1.63** (1.63× slower) | pass |

**Aggregate: geometric mean ~0.85 (27 pairs, -O2, paired medians,
2026-08-25).** Conventional-code geomean (excluding the algorithmic
recursion wins `fib`/`ackermann`) is ~0.95. Every number above was produced
by a **correct** binary; correctness is enforced before speed is recorded
(checksums byte-identical to GCC).

## Where LCCC wins

- **Recursion folding**: `fib`/`ackermann` (TCE + rec2iter; GCC keeps the
  exponential recursion).
- **Reduction vectorization**: packed accumulators with one horizontal
  reduction at exit; widening I32→I64 and masked conditional sums.
- **Scalar-FP code quality**: VEX 3-operand staging; FP compare-to-branch
  fusion; direct xmm0 returns (libm round family 2.43×).
- **PGO**: `-fprofile-generate`/`-fprofile-use` (inlining, unrolling,
  switch lowering) with conservative layout that never perturbs RA.
- **Peephole layer with exact CFG liveness**: each rule kill-switch gated
  for bisection.

## Root causes of the remaining gap (tracked, in priority order)

1. **Loop rotation default-off** — every inner loop pays a double-jump
   preheader (`cmp; jge; .Lbody; …; jmp .Lhead`) vs GCC's rotated
   test-and-branch: ~1 branch/iter suite-wide. The pass is
   correctness-clean for the canonical shape and opt-in
   (`CCC_LOOP_ROTATE=1`); hardening is TASK-PF-17.
2. **RA-06 reload-at-use / arithmetic-chain copy webs** — adler32 1.63×,
   the largest codec gap; lifetime demotion spills whole ranges.
   TASK-RA-06A.
3. **Non-reduction FP vectorization** — spectral/nbody/mandelbrot need
   multi-store scatter + computed-invariant dot analysis. TASK-OP-05B.
4. **Marching-pointer slot-homing** — 151 of nbody's 159 stack refs are
   slot-homed IVSR pointer recurrences. RA-01b.
5. **Hash-multiply `imul` chains** (expat) and the branchy `__ffs` tree
   (find_bit → gcc `andn`+`cmov`).

## Kill switches (for bisection / soundness fallback)

All are env vars; the authoritative list with polarity lives in
[`engineering/agent/RULES.md`](engineering/agent/RULES.md). Highlights:

| Switch | Disables |
|---|---|
| `CCC_NO_TIER2_GRAPH` | Tier-2 graph coloring (restores scan-only) |
| `CCC_NO_SEGMENT_FILL` | segment-aware residual fill |
| `CCC_NO_GLOBAL_ADDR_REMAT` | global-address rematerialization |
| `CCC_NO_STENCIL_VEC` / `CCC_NO_MAP_VEC` / `CCC_NO_MAP_VECREG` / `CCC_NO_VECREG` | vectorizer tiers |
| `CCC_NO_PF06_ADD_PEEL` | PF-06 add(iv,const) SIB displacement peeling |
| `CCC_LOOP_ROTATE=1` / `CCC_NO_LOOP_ROTATE=1` | opt-in loop rotation |
| `CCC_NO_DSE` | dead-store elimination |
| `CCC_VERIFY_REGALLOC=1` | hard-verify RA invariants over a corpus (catches interference bugs the suite misses) |
| `CCC_RA_EXPLAIN=fn` / `CCC_TRACE_ALLOCSTATS[=filter]` | deterministic spill/allocation reports |
| `LCCC_DUMP_IR=1` | post-optimization IR dump |
| `CCC_NO_GEP_FOLD` | all GEP folding (strictest fallback) |
| `LCCC_NO_PEEPHOLE` | verbatim pre-peephole assembly (x86-64 + i686 text pipelines); `CCC_NO_PEEPHOLE` disables the structured peephole passes |
