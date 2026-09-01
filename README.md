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
- **Engineering backlog:** [`backlog.md`](backlog.md) — ranked by measured
  impact, with reproducer and blocker per item
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
| **LCCC (ms178)** | `main` @ `be93d266` | previous report plus: order-preserving block layout, machine-level loop inversion, escaping-IV widening, the vectorized-loop-counter fix, register-copy folding at all four widths, `range_fold` boolean-widening repair, and MachInst instruction selection at 85.1% coverage |
| **GCC** | 14.2.0 (Debian 14.2.0-19) | external reference, stock |

`-O2`, identical sources, same machine, same run window (2026-09-01). Driver:
`tests/benchmark/run_benchmarks.py` — 9 paired timed rounds + 1 excluded
warm-up per kernel, randomized compiler order every round, MAD outliers
**retained and reported, never silently discarded**. All 33 outputs are
byte-identical to the GCC baseline; a checksum mismatch disqualifies the row
before any timing is recorded. Frozen raw JSON with every per-round sample:
[`engineering/evidence/benchmarks/2026-09-01-be93d266/`](engineering/evidence/benchmarks/2026-09-01-be93d266/README.md).

> **Ratios are not comparable across reports.** A ratio is only stable when
> both arms were measured in the same window; drift on a shared VM that hits
> the two compilers unequally moves it. Attribution of any individual change
> must come from a paired same-window A/B with kill switches.

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
| `fib` | recursive Fibonacci / recurrence recognition | 2.5 | 176.5 | **0.014** (73.13× faster) |
| `constant_recursion` | constant recursive specialization | 2.3 | 149.5 | **0.015** (64.59× faster) |
| `ackermann` | Ackermann / deep recursion | 2.4 | 149.7 | **0.016** (63.39× faster) |
| `libm_round_family` | glibc libm scalar rounding entry points (vroundsd inline) | 255.3 | 1116.0 | **0.230** (4.35× faster) |
| `matmul` | dense matrix multiply / FP and cache | 5.4 | 7.3 | **0.747** (1.34× faster) |
| `bitops` | bit manipulation / integer selection | 299.4 | 389.9 | **0.767** (1.30× faster) |
| `double_reduction` | two independent accumulators per loop (multi-reduction) | 97.5 | 111.1 | **0.874** (1.14× faster) |
| `gzip_crc32` | GNU gzip CRC-32 scalar table loop | 161.7 | 170.1 | **0.950** (1.05× faster) |
| `binary_search` | sorted-table binary search / branch-heavy lookup | 2.2 | 2.2 | **0.994** |
| `ring_fifo` | masked ring FIFO enqueue/dequeue / dependent loads | 2.1 | 2.1 | **1.001** |
| `qsort` | quicksort via libc / branches | 128.7 | 126.6 | **1.011** |
| `tce_sum` | tail-recursive accumulator / TCE | 2.2 | 2.3 | **1.013** |
| `mandelbrot` | Mandelbrot / FP branch-heavy inner loop | 1564.2 | 1504.8 | **1.038** (1.04× slower) |
| `strlen_bench` | string operations / byte loops | 241.3 | 237.9 | **1.041** (1.04× slower) |
| `linux_find_bit` | Linux sparse find_next_andnot_bit | 15.4 | 14.8 | **1.045** (1.04× slower) |
| `ascii_case_fold` | ASCII parser case-fold byte loop / branch selection | 2.4 | 2.3 | **1.049** (1.05× slower) |
| `switch_dispatch` | switch lowering / dispatch | 539.8 | 510.9 | **1.051** (1.05× slower) |
| `sieve` | sieve of Eratosthenes / stores | 40.1 | 37.9 | **1.052** (1.05× slower) |
| `histogram` | 256-bin histogram / indexed increment and reduction | 2.6 | 2.5 | **1.065** (1.07× slower) |
| `binary_trees` | binary trees / allocation and recursion | 1437.5 | 1317.0 | **1.087** (1.09× slower) |
| `fannkuch` | Fannkuch-Redux / permutations | 2983.7 | 2561.7 | **1.166** (1.17× slower) |
| `hash_table` | hash table / pointer chasing | 12035.6 | 10399.2 | **1.181** (1.18× slower) |
| `loop_patterns` | scalar loop transforms | 85.4 | 71.4 | **1.195** (1.20× slower) |
| `zlib_ng_adler32` | zlib-ng Adler-32 NMAX accumulator | 50.2 | 39.6 | **1.239** (1.24× slower) |
| `sqlite_varint` | SQLite 1–9 byte varint decoder | 34.4 | 27.1 | **1.250** (1.25× slower) |
| `nbody` | N-body simulation / FP structs | 396.8 | 306.6 | **1.281** (1.28× slower) |
| `aarch64_select_patterns` | conditional increment, narrow compare, and select pressure | 170.5 | 129.7 | **1.310** (1.31× slower) |
| `glibc_memcmp` | glibc aligned-word memcmp path | 12.2 | 9.2 | **1.330** (1.33× slower) |
| `arith_loop` | 32-variable arithmetic loop / register pressure | 143.5 | 100.1 | **1.430** (1.43× slower) |
| `struct_copy` | struct copy / ABI and memory | 39.4 | 27.4 | **1.439** (1.44× slower) |
| `spectral_norm` | spectral norm / dense floating point | 298.7 | 200.4 | **1.491** (1.49× slower) |
| `expat_xml_scan` | Expat UTF-8 XML name-token scan | 86.9 | 41.5 | **2.089** (2.09× slower) |
| `tls_seg_access` | glibc TLS access shapes (THREAD_SELF/SETMEM, %fs segment) | 24.6 | 11.4 | **2.161** (2.16× slower) |

**Aggregate.** Geometric mean **0.7381** over all 33 pairs — dominated by the
algorithmic recursion folds. Conventional code (30 pairs, recursion folds
excluded): **1.090**. The workload-derived codec/parser subset (7 pairs:
zlib-ng, expat, SQLite, gzip, glibc, Linux, TLS) sits at **1.374** and is
where the remaining gap concentrates. All 33 checksums byte-identical to GCC.

## Where LCCC wins

- **Recursion folding**: `fib` 73.1×, `constant_recursion` and `ackermann`
  ~66× — TCE + rec2iter; the shipped binaries contain zero recursive calls.
- **Scalar-FP code quality**: `libm_round_family` **4.4×**.
- **Reduction vectorization**: `matmul` **1.36×** (AVX2 `vfmadd231pd`),
  `double_reduction` **1.15×**.
- **Integer selection/idioms**: `bitops` **1.31×**.
- **Byte-scan loops** (measured against the Godbolt oracle at
  `-O3 -march=x86-64-v3`, assembled with GAS 2.47): a byte-copy loop compiles
  to **14 instructions against GCC 113, Clang 23.1 91, ICC 92 and ICX 41** —
  best of all five; `memchr` runs **1.39× faster than GCC**.

## Root causes of the remaining gap (tracked, by magnitude)

Ranked by this run. Full backlog with reproducer and blocker per item:
[`backlog.md`](backlog.md).

1. **TLS segment access 2.16×** — the thread pointer is staged in every
   TLS-using function's prologue where GCC addresses `%fs:offset` directly.
2. **Byte-classifier chains 2.09×** (`expat_xml_scan`) — `if (pred) n++` over
   an `a || b || c` predicate emits an **eleven-branch chain per byte**. The
   `if_convert` → `range_fold` → `set_membership` pipeline is starved at the
   first link; the boolean-returning form of the same predicate gets the full
   bit-mask treatment. Backlog **PF-CLS-1**.
3. **Register pressure / copy webs 1.43–1.49×** — `arith_loop`,
   `spectral_norm`, `struct_copy`: lifetime demotion spills whole ranges
   instead of splitting them.
4. **Non-reduction FP vectorization 1.27–1.49×** — `spectral_norm`, `nbody`.
5. **Accumulator recurrences 1.24×** — `zlib_ng_adler32`'s DO8 chain; ICC
   needs 76 instructions where lccc needs 119. Backlog **PF-ADLER-1**.
6. **Near-parity tail** — `hash_table`, `binary_trees`, `switch_dispatch`,
   `histogram`, `memcmp`, `mandelbrot` all ≤1.10×.

## Instruction-selection coverage

`CCC_ISEL_STATS=1` reports how much of code generation flows through the
typed, register-allocated MachInst layer rather than direct text emission — a
fallback that is otherwise **silent**.

Corpus-wide (562 files, 9116 instructions): **85.1%**, up from 53.9%. The
residual, ranked: `Call` 7.8% (**correct as-is** — MachInst has no clobber
modelling, and the buffer flush at a call *is* what keeps caller-saved values
sound), `Store(float)` 1.3% (no XMM register class), `ParamRef(needs-code)`
1.1%, `Memcpy` 1.0%, `Store(other)` 0.9%. A coverage regression test fails if
any class the layer owns drops back out.

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
