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
| **LCCC (ms178)** | `main` @ `2e434473` | previous report plus: induction-variable widening through the element-scaling chain (it previously fired only for byte arrays), order-preserving block layout, machine-level loop inversion, the vectorized-loop-counter fix, register-copy folding at all four widths, and MachInst instruction selection at 85.1% coverage |
| **GCC** | 14.2.0 (Debian 14.2.0-19) | external reference, stock |

`-O2`, identical sources, same machine, same run window (2026-09-01). Driver:
`tests/benchmark/run_benchmarks.py` — 9 paired timed rounds + 1 excluded
warm-up per kernel, randomized compiler order every round, MAD outliers
**retained and reported, never silently discarded**. All 33 outputs are
byte-identical to the GCC baseline; a checksum mismatch disqualifies the row
before any timing is recorded. Frozen raw JSON with every per-round sample:
[`engineering/evidence/benchmarks/2026-09-01b-2e434473/`](engineering/evidence/benchmarks/2026-09-01b-2e434473/README.md).

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
| `fib` | recursive Fibonacci / recurrence recognition | 2.4 | 171.8 | **0.014** (70.77× faster) |
| `constant_recursion` | constant recursive specialization | 2.5 | 156.8 | **0.015** (64.90× faster) |
| `ackermann` | Ackermann / deep recursion | 2.5 | 151.5 | **0.016** (61.69× faster) |
| `libm_round_family` | glibc libm scalar rounding entry points (vroundsd inline) | 257.3 | 1118.0 | **0.227** (4.40× faster) |
| `matmul` | dense matrix multiply / FP and cache | 5.5 | 7.2 | **0.764** (1.31× faster) |
| `bitops` | bit manipulation / integer selection | 312.9 | 398.6 | **0.796** (1.26× faster) |
| `double_reduction` | two independent accumulators per loop (multi-reduction) | 96.1 | 113.3 | **0.876** (1.14× faster) |
| `gzip_crc32` | GNU gzip CRC-32 scalar table loop | 159.3 | 167.7 | **0.950** (1.05× faster) |
| `qsort` | quicksort via libc / branches | 128.3 | 126.8 | **1.010** |
| `strlen_bench` | string operations / byte loops | 256.7 | 246.2 | **1.026** (1.03× slower) |
| `binary_search` | sorted-table binary search / branch-heavy lookup | 2.3 | 2.2 | **1.029** (1.03× slower) |
| `tce_sum` | tail-recursive accumulator / TCE | 2.2 | 2.1 | **1.030** (1.03× slower) |
| `ring_fifo` | masked ring FIFO enqueue/dequeue / dependent loads | 2.3 | 2.2 | **1.031** (1.03× slower) |
| `mandelbrot` | Mandelbrot / FP branch-heavy inner loop | 1567.5 | 1505.5 | **1.040** (1.04× slower) |
| `sieve` | sieve of Eratosthenes / stores | 38.1 | 36.6 | **1.043** (1.04× slower) |
| `ascii_case_fold` | ASCII parser case-fold byte loop / branch selection | 2.5 | 2.4 | **1.048** (1.05× slower) |
| `linux_find_bit` | Linux sparse find_next_andnot_bit | 16.0 | 15.1 | **1.055** (1.05× slower) |
| `switch_dispatch` | switch lowering / dispatch | 541.9 | 509.7 | **1.065** (1.07× slower) |
| `histogram` | 256-bin histogram / indexed increment and reduction | 2.7 | 2.5 | **1.072** (1.07× slower) |
| `hash_table` | hash table / pointer chasing | 12917.6 | 11450.4 | **1.124** (1.12× slower) |
| `loop_patterns` | scalar loop transforms | 86.8 | 75.4 | **1.153** (1.15× slower) |
| `fannkuch` | Fannkuch-Redux / permutations | 2997.3 | 2585.9 | **1.167** (1.17× slower) |
| `binary_trees` | binary trees / allocation and recursion | 1576.5 | 1356.8 | **1.178** (1.18× slower) |
| `sqlite_varint` | SQLite 1–9 byte varint decoder | 33.3 | 27.4 | **1.218** (1.22× slower) |
| `zlib_ng_adler32` | zlib-ng Adler-32 NMAX accumulator | 52.2 | 40.7 | **1.294** (1.29× slower) |
| `aarch64_select_patterns` | conditional increment, narrow compare, and select pressure | 168.7 | 128.8 | **1.306** (1.31× slower) |
| `nbody` | N-body simulation / FP structs | 405.0 | 311.0 | **1.308** (1.31× slower) |
| `glibc_memcmp` | glibc aligned-word memcmp path | 12.4 | 9.3 | **1.331** (1.33× slower) |
| `arith_loop` | 32-variable arithmetic loop / register pressure | 144.4 | 100.2 | **1.406** (1.41× slower) |
| `struct_copy` | struct copy / ABI and memory | 40.6 | 27.8 | **1.443** (1.44× slower) |
| `spectral_norm` | spectral norm / dense floating point | 304.2 | 206.2 | **1.494** (1.49× slower) |
| `expat_xml_scan` | Expat UTF-8 XML name-token scan | 88.2 | 42.7 | **2.074** (2.07× slower) |
| `tls_seg_access` | glibc TLS access shapes (THREAD_SELF/SETMEM, %fs segment) | 25.5 | 11.6 | **2.195** (2.19× slower) |

**Aggregate.** Geometric mean **0.7431** over all 33 pairs — dominated by the
algorithmic recursion folds. Conventional code (30 pairs, recursion folds
excluded): **1.096**. The workload-derived codec/parser subset (7 pairs) sits
at **1.381**. All 33 checksums byte-identical to GCC. Frozen raw JSON:
[`engineering/evidence/benchmarks/2026-09-01b-2e434473/`](engineering/evidence/benchmarks/2026-09-01b-2e434473/README.md).

### Refresh — 2026-09-05, `rebased` @ `f9899f30` (post #419)

Same protocol family, screening-grade cadence: 3 paired reps + 1 warmup,
two same-window batches of 16 kernels, `-O2`, all 32 x86 outputs
byte-identical to GCC. Raw samples:
[`engineering/evidence/benchmarks/2026-09-05-f9899f30/`](engineering/evidence/benchmarks/2026-09-05-f9899f30/results.json).

**Aggregate: geometric mean 0.7390 over 32 pairs; conventional code
(recursion folds excluded) 1.0416.**

| Kernel | LCCC (ms) | GCC (ms) | LCCC/GCC |
|---|---:|---:|---:|
| `fib` | 1.10 | 130.43 | **0.008** (118.07× faster) |
| `ackermann` | 1.11 | 61.66 | **0.018** (55.42× faster) |
| `constant_recursion` | 8.06 | 63.72 | **0.126** (7.9× faster) |
| `libm_round_family` | 202.2 | 490.7 | **0.412** (2.43× faster) |
| `bitops` | 201.6 | 302.1 | **0.667** (1.50× faster) |
| `matmul` | 4.19 | 5.65 | **0.741** (1.35× faster) |
| `gzip_crc32` | 135.8 | 155.3 | **0.874** (1.14× faster) |
| `double_reduction` | 110.1 | 118.8 | **0.927** (1.08× faster) |
| `switch_dispatch` | 467.4 | 478.5 | **0.977** (1.02× faster) |
| `arith_loop` | 92.9 | 92.7 | 1.002 |
| `qsort` | 112.5 | 111.9 | 1.006 |
| `tls_seg_access` | 9.26 | 9.07 | 1.021 |
| `zlib_ng_adler32` | 38.0 | 37.3 | 1.021 |
| `histogram` | 1.63 | 1.57 | 1.042 (1.04× slower) |
| `struct_copy` | 23.0 | 21.9 | 1.053 (1.05× slower) |
| `strlen_bench` | 223.8 | 212.4 | 1.053 (1.05× slower) |
| `loop_patterns` | 49.2 | 46.7 | 1.054 (1.05× slower) |
| `binary_trees` | 2004.2 | 1879.3 | 1.066 (1.07× slower) |
| `sieve` | 52.1 | 48.1 | 1.082 (1.08× slower) |
| `hash_table` | 21868.2 | 19590.8 | 1.116 (1.12× slower) |
| `sqlite_varint` | 25.9 | 21.2 | 1.219 (1.22× slower) |
| `mandelbrot` | 1105.7 | 893.6 | 1.237 (1.24× slower) |
| `nbody` | 272.5 | 214.6 | 1.270 (1.27× slower) |
| `expat_xml_scan` | 47.1 | 34.7 | 1.359 (1.36× slower) |
| `linux_find_bit` | 14.6 | 10.2 | 1.424 (1.42× slower) |
| `fannkuch` | 3209.0 | 2252.1 | 1.425 (1.43× slower) |
| `glibc_memcmp` | 9.11 | 5.91 | 1.542 (1.54× slower) |
| `spectral_norm` | 291.8 | 182.0 | **1.603** (1.60× slower) |

Not shown: `binary_search`, `ring_fifo`, `tce_sum` and
`ascii_case_fold` (near-parity; sub-2 ms medians are wall-timer-bound). Movement vs 2026-09-01 (directional, different window):
expat 2.07→1.36, tls_seg_access 2.20→1.02, arith_loop 1.41→1.00,
struct_copy 1.44→1.05, double_reduction 1.14→0.93. The tracked P0 targets
remain `spectral_norm` (PERF-41) and the scalar-load-sinking family
(OPT-40: glibc_memcmp, linux_find_bit). † sub-2 ms medians are indicative
only.

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
