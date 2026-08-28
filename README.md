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

> **Screening evidence.** Wall-clock, paired medians; every binary's output is
> checksummed against the GCC baseline byte-for-byte (a mismatch disqualifies
> the number). Measured on a shared 2-vCPU VM (hypervisor detected, no PMU —
> `perf` absent), taskset-pinned; figures rank code-generation work and are
> *not* microarchitectural claims. Reproduce on bare metal (the 14700KF
> target) before treating any figure as a hardware result.

## Compared revisions

| Compiler | Revision | Notes |
|---|---|---|
| **LCCC (ms178)** | `main` @ `1b3994e7` | segment-aware regalloc + Tier-2 graph coloring, general complete unrolling, VEX 3-operand scalar-FP, widening + masked conditional-sum reduction vectorization, stencil/map vectorizers, FMA3 ISA gate + tri-state FP contraction, DSE, backedge PRE, opt-in loop rotation, peephole SP-displacement model, canonical-GEP vectorizer gate |
| **GCC** | 14.2.0 (Debian 14.2.0-19) | external reference |

`-O2`, identical sources, same machine, same run window (2026-08-28). Driver:
`tests/benchmark/run_benchmarks.py` (canonical runner) — 15 paired timed
rounds + 2 excluded warm-ups per kernel (`hash_table`: 8 + 1 for the VM
window budget; realized CI ±0.5 %, CV 5.5 %, one MAD outlier retained — see
evidence), randomized compiler order in every round. Frozen raw JSON +
independently verified merge:
[`engineering/evidence/benchmarks/2026-08-28-1b3994e7/`](engineering/evidence/benchmarks/2026-08-28-1b3994e7/README.md).

## Results (33 kernels; ratio = LCCC/GCC, < 1 means LCCC faster)

The ratio column is the runner's **median of paired per-round ratios**
(numerator/denominator within each round, order randomized), not the
quotient of the two median columns; the two definitions agree in aggregate
(0.737 vs 0.738) but can differ per row by ~1–2 %.

† sub-2 ms median — wall-timer overhead dominates; indicative only. The
runner's noise threshold is 20 ms; the remaining sub-20 ms rows (matmul,
glibc_memcmp, linux_find_bit, tls_seg_access) carry tight paired CIs (≤0.6 %
width) and disassembly-verified mechanisms where cited. All 33 outputs are
byte-identical to GCC.

| Kernel | What it stresses | LCCC (ms) | GCC (ms) | LCCC/GCC |
|---|---|---:|---:|---:|
| `fib` † | binary recursion → rec2iter | 1.2 | 129.7 | **0.010** (104.7× faster) |
| `constant_recursion` † | constant recursive specialization | 1.0 | 61.3 | **0.016** (~61× faster) |
| `ackermann` † | deep recursion | 1.1 | 61.6 | **0.018** (~56× faster) |
| `libm_round_family` | libm round intrinsics | 202.4 | 490.8 | **0.413** (2.42× faster) |
| `bitops` | integer selection, popcount idioms | 167.7 | 299.7 | **0.558** (1.79× faster) |
| `gzip_crc32` | gzip CRC-32 table loop | 135.5 | 155.3 | **0.873** (1.15× faster) |
| `matmul` | loop-nest FP + reduction FMA | 5.3 | 5.6 | **0.938** |
| `double_reduction` | two independent FP reductions | 105.4 | 109.8 | **0.955** |
| `qsort` | libc branches | 109.3 | 112.5 | **0.974** |
| `switch_dispatch` | switch lowering | 466.6 | 477.9 | **0.975** |
| `binary_search` † | branch-heavy lookup | 1.0 | 1.0 | **0.991** |
| `tce_sum` † | tail-recursive accumulator | 0.8 | 0.8 | **1.003** |
| `glibc_memcmp` | aligned-word memcmp path | 6.1 | 5.9 | **1.030** |
| `ring_fifo` † | masked ring FIFO, dependent loads | 0.9 | 0.9 | **1.040** |
| `strlen_bench` | string byte loops | 217.4 | 210.1 | **1.041** |
| `histogram` † | indexed increment/reduction | 1.6 | 1.5 | **1.048** |
| `binary_trees` | allocation + recursion | 2071.7 | 1953.6 | **1.057** |
| `loop_patterns` | scalar loop transforms | 45.7 | 43.6 | **1.059** |
| `ascii_case_fold` † | ASCII case-fold byte loop | 0.9 | 0.9 | **1.066** |
| `hash_table` | pointer chasing | 21179.3 | 19549.1 | **1.090** |
| `aarch64_select_patterns` | select/compare pressure | 123.7 | 106.3 | **1.165** |
| `sqlite_varint` | varint decoder | 26.1 | 21.5 | **1.192** |
| `sieve` | branchy int stores | 49.9 | 42.0 | **1.200** |
| `arith_loop` | 32-variable register pressure | 114.0 | 92.6 | **1.227** |
| `mandelbrot` | FP branch-heavy loop | 1102.7 | 894.4 | **1.232** |
| `nbody` | N-body FP structs | 264.0 | 214.5 | **1.233** |
| `fannkuch` | permutations | 2880.4 | 2259.6 | **1.274** |
| `spectral_norm` | dense FP | 237.3 | 181.7 | **1.305** |
| `struct_copy` | struct copy / ABI | 29.7 | 21.8 | **1.346** |
| `zlib_ng_adler32` | Adler-32 DO8 kernel | 56.3 | 37.4 | **1.504** |
| `linux_find_bit` | sparse bit search | 15.3 | 10.1 | **1.508** |
| `expat_xml_scan` | XML name-token scan | 62.8 | 34.9 | **1.803** |
| `tls_seg_access` | glibc TLS access shapes (`%fs`) | 19.6 | 9.1 | **2.146** |

**Aggregate.** Geometric mean **0.738** over all 33 pairs — dominated by the
algorithmic recursion folds. Conventional code (30 pairs, recursion folds
excluded): **1.096** — GCC is ~10% ahead on this corpus today. The
workload-derived codec/parser subset sits at **1.22**. Correctness is
enforced before speed is recorded: all 33 checksums byte-identical to GCC.

## Where LCCC wins

- **Recursion folding**: `fib` 104.7× (TCE + rec2iter at runtime);
  `ackermann` ~56× and `constant_recursion` ~61× fold entirely at compile
  time — the shipped binaries contain zero recursive calls.
- **Scalar-FP code quality**: libm round family 2.42× — VEX 3-operand
  staging, FP compare-to-branch fusion, direct `xmm0` returns.
- **Integer selection/idioms**: `bitops` 1.79×.
- **gzip CRC-32 table loop** 1.15×.
- **Reduction vectorization**: `matmul` at parity with AVX2 FMA confirmed in
  the hot loop (`vfmadd231pd`); `double_reduction` slightly ahead. The
  canonical-GEP vectorizer gate (PR #278) kept vectorization where it is
  legal; sub-10 ms kernels give noisy cross-day ratios.
- **PGO** (capability, not exercised in this run):
  `-fprofile-generate`/`-fprofile-use` (inlining, unrolling, switch
  lowering) with conservative layout that never perturbs RA.

## Root causes of the remaining gap (tracked, by magnitude)

1. **TLS segment access 2.15×** — LCCC stages the thread pointer
   (`mov %fs:0x0,%rax`) in each TLS-using function's prologue (two of three
   loads are duplicate, non-CSE'd `&tls_slots` computations) where GCC uses
   direct `%fs:offset` / `%fs:(%idx,scale)` operands; the
   link-time-constant-offset path (`e290be59`) does not cover dynamic
   offset forms. Not yet tracked — candidate IS task.
2. **Hash-multiply `imul` chains** (expat 1.80×) and the branchy `__ffs`
   tree (find_bit 1.51× → gcc `andn`+`cmov`).
3. **RA-06 reload-at-use / arithmetic-chain copy webs** — adler32 1.50×,
   struct_copy 1.35×; lifetime demotion spills whole ranges. TASK-RA-06A.
4. **Non-reduction FP vectorization** — spectral 1.31×, fannkuch 1.27×,
   nbody/mandelbrot 1.23× need multi-store scatter + computed-invariant dot
   analysis (TASK-OP-05B); nbody additionally pays marching-pointer
   slot-homing (151 of 159 stack refs; RA-01b).
5. **Loop rotation default-off** — every inner loop pays a double-jump
   preheader vs GCC's rotated test-and-branch: ~1 branch/iter suite-wide.
   Opt-in (`CCC_LOOP_ROTATE=1`); hardening is TASK-PF-17.
6. **Mid-band 1.17–1.23×** — arith_loop (register pressure; RA-06A),
   aarch64_select_patterns, sqlite_varint, sieve (branch layout/ISel).
7. **Near-parity tail**: hash_table 1.09×, binary_trees 1.06×,
   strlen 1.04×, memcmp 1.03×.

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
