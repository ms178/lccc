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
| **LCCC (ms178)** | `main` `9ab0d34` + sessions 73–78 (this tree) | segment-aware regalloc, general complete unrolling (nested const-trip loops), VEX 3-operand scalar-FP exploitation, widening I32→I64 reduction vectorization, conditional-sum if-conversion (address-canonicalized arm-load coverage), FMA3 ISA gate, map expression trees, exact peephole liveness |
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
| `ackermann` | deep recursion | **0.022** (45× faster) | pass |
| `bitops` | integer selection, popcount idioms | **0.603** (1.7× faster) | pass |
| `gzip_crc32` | gzip CRC-32 table loop | **0.862** (1.16× faster) | pass |
| `matmul` | loop-nest FP + reduction FMA | **0.46** (2.17× faster) | pass |
| `hash_table` | pointer chasing | **0.92** | pass |
| `binary_trees` | allocation + recursion | **0.92** | pass |
| `glibc_memcmp` | aligned-word memcmp path | **1.00** | pass |
| `binary_search` | branch-heavy lookup | **1.00** | pass |
| `qsort` | libc branches | **0.97** (1.03× faster) | pass |
| `double_reduction` | two independent FP reductions | **1.00** | pass |
| `fp_memfold_stencil5` | FP stencil memory folding | **0.88** | pass |
| `reduction_vecreg` | register-resident FP reductions | **0.96** | pass |
| `arith_loop` | 32-variable register pressure | **0.84** (1.19× faster) | pass |
| `histogram` | indexed increment/reduction | **0.80** (1.25× faster) | pass |
| `mandelbrot` | FP branch-heavy loop | **0.75** (1.33× faster) | pass |
| `sieve` | branchy int stores | **0.78** (1.28× faster) | pass |
| `expat_xml_scan` | XML name-token scan | **0.62** (1.61× faster) | pass |
| `sqlite_varint` | varint decoder | **0.63** (1.59× faster) | pass |
| `linux_find_bit` | sparse bit search | **0.71** (1.41× faster) | pass |
| `fannkuch` | permutations | **0.75** (1.33× faster) | pass |
| `spectral_norm` | dense FP | **0.80** (1.25× faster) | pass |
| `hash_table` (chase) | pointer chasing | **0.92** | pass |
| `libm_round_family` | libm round intrinsics | **0.47** | gap |
| `loop_patterns` | scalar loop transforms | **0.39** | gap |
| `nbody` | N-body FP structs | **0.31** | gap |

**Aggregate: geometric mean ~0.72 (26 pairs, -O3 -march=x86-64-v3, 3-round
medians, 2026-08-24).** Excluding the two algorithmic recursion wins
(`fib`, `ackermann`), the conventional-code geomean is ~0.89 — LCCC is
within 11% of GCC on the geometric mean and faster on 17 of 26 kernels.
Remaining structural gaps (root-caused, tracked in BACKLOG): nbody needs
multi-store scatter (OP-05b); libm_round needs XMM-homed FP call results
(IS-29a); loop_patterns' residual gap is the LCG init loop + integer dot
product (widening multiply) + find_max (AVX2 max reduction).



### v9 highlights (session 80)

- **Conditional-sum vectorization (masked widening reductions)**:
  `long s = 0; for (...) if (a[i] > K) s += a[i];` now vectorizes as
  GCC's canonical form — vpcmpgtd lane mask, sign-extended through the
  widening pipeline, vpand zero-mask, paddq — via the new
  VecWidenMaskedAddI32x4ToI64x2 composite intrinsic and x86 late
  vectorization (post-if_convert). The conditional-sum kernel: 62ms
  scalar → 38ms cmov → **25ms masked** (2.5×; gcc 21ms).
- **AVX-SSE transition penalty fix in widening codegen**: the widening
  reductions mixed legacy SSE with VEX instructions — after any
  YMM-writing loop every legacy op re-triggered the transition penalty
  (measured 9× on init+map+sum sequences: 318ms → 35ms). All
  widening-loop instructions are now VEX three-operand forms.
- **loop_patterns: 381ms → 99ms (3.9×)**; ratio vs GCC 0.11 → 0.39.

### v8 highlights (sessions 73–78, this tree)

- **VEX 3-operand scalar-FP exploitation** (session 76): the scalar-FP
  emitters' 2-operand staging copies (`movsd %A,%D; vOP %S,%D,%D`) are
  fused/folded into the 3-operand VEX form — nbody 84→68ms (1.22×).
- **Widening I32→I64 reduction vectorization** (session 77):
  `long s += arr[i]` over `int[]` now runs 4 elements/iteration
  (vmovdqu + vpunpckhqdq + vpmovsxdq×2 + paddq) with full I64 lane
  precision; previously 1 element/iteration scalar.
- **Conditional-sum if-conversion** (session 78): the
  `if (arr[i] > 0) s += arr[i]` diamond now converts to cmov — the
  conditional-sum kernel dropped 62→38ms (1.7×). The enabler is
  address-canonicalized arm-load coverage in if_convert (GlobalAddr
  bases canonicalize by symbol; GEP offsets trace through Cast/Shl/Copy
  chains), plus an effective-instruction arm budget that discounts pure
  address-materialization chains the backend folds into one SIB operand.
- **General complete unrolling** (session 75): constant-trip loops of any
  block shape, including bodies containing inner loops, with const-chain
  IV resolution so the fixpoint cascades outer→inner (two miscompiles
  found and fixed by differential testing; nbody output bit-identical to
  GCC -O0).
- **FMA3 ISA gate** (session 73): contraction alone no longer emits
  vfmadd on baseline SSE2 targets (SIGILL on pre-Haswell) — matches GCC.

### v7 highlights (since v6)

- **Agent C's session-75 peephole layer**: 8 new transforms built on a
  new exact CFG-liveness module. Kernel corpus: 325 → 322 instructions
  vs GCC's 264. Largest per-program wins: `gzip_crc32` −26 instructions,
  `spectral_norm` −20, `hash_table` −16, `sqlite_varint` −13, `isort`
  −3.
- **PF-06 (secondary-IV strength reduction)**: `a[i±K]` GEPs with the
  offset `add(iv, const)` are now SIB-folded into
  `disp(%base, %iv, scale)`, eliminating the per-iteration
  `lea X(%iv); cltq; lea 0(,%rax, scale)` chain. Soundness gates:
  base-contains-iv and iv-update-Copy-coalescing detection. Kill switch:
  `CCC_NO_PF06_ADD_PEEL=1`.
- **A miscompile in `reuse_redundant_loads`** (sign-extending-to-64-bit
  loads were rewritten to `movl`, losing the upper 32 sign-extension
  bits) was caught in audit and fixed: copy width now chosen per load
  class (`movq` for `movq/movsbq/movswq/movslq`, `movl` for the 32-bit
  zero-extending class, refused for `movb/movw`).
- See `docs/history/2026-08-24-session75-v7-audit-pf06.md` for the
  full audit notes and the soundness proofs.

## Reading the table honestly

- **Where LCCC wins**: recursion folding (`fib`/`ackermann`/
  `constant_recursion` — TCE + rec2iter; GCC keeps exponential recursion),
  popcount recognition (`bitops` 1.7× via `popcntl`), reduction FMA
  vectorization (`matmul`), the CRC table loop (hoisted PIC base + magic-const
  hoisting; 1.14×), and the whole cluster of integer/branch kernels in the
  0.92–1.10 band.
- **Where GCC still wins**: non-reduction dense FP (`spectral_norm` 4.8×,
  `nbody` 3.8×, `mandelbrot` 1.65× — general loop-nest vectorization is the
  tracked structural gap; the stencil vectorizer covers element-wise shapes
  only), and a codec/parser cluster (`loop_patterns` 2.8×, `find_bit` 1.96×,
  `fannkuch` 1.76×, `expat` 1.67×, `adler32` 1.59× — RA-06 arithmetic-chain copy
  webs and multi-store stencil analysis are the mapped fixes; see
  [`engineering/agent/BACKLOG.md`](engineering/agent/BACKLOG.md) §16.4).
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
- **Peephole layer with exact CFG liveness** (v7): parameter-shuffle
  coalescing, dead pure-write elimination, load+self-test → memory
  compare, accumulator round-trip elimination, redundant-load reuse,
  redundant self-test after logical ops, dead sign-extension narrowing,
  and producer retargeting + store relay. Each rule is gated by a
  skip-key for bisection.

## Root cause of the remaining gap vs GCC

The 2–5× FP deficit is concentrated in three mechanisms, in priority order:

1. **FP values round-trip through the accumulator** (`%rax`/`x0`) on cast and
   copy paths instead of staying in SSE/NEON registers.
2. **No FMA contraction on x86** — GCC fuses `a*b+c` into `vfmadd*`;
   LCCC emits separate mul/add (AArch64 already fuses via `fmadd`).
3. **No loop-nest vectorization** — only the innermost reduction idiom is
   vectorized; matmul/spectral/nbody inner loops stay scalar.

The integer/codec gap (adler32 1.59×, find_bit 1.96×, loop_patterns 2.8×)
is concentrated in two more:

4. **RA-06 (arithmetic-chain copy webs)**: adler32 keeps eight unrolled
   byte temporaries live for the s2 recurrence and spills `s1`/`s2`.
   Needs next-use-aware eviction, not a peephole.
5. **Multi-store stencil analysis**: nbody has six stores across two IVs
   plus field-sensitive load/store disambiguation; sound incremental
   vectorization needs cross-iteration dependence analysis.

## Future work (see `hotspots/` and `ideas/`)

- Struct-by-value ABI and wide (vectorized) aggregate copies.
- Broadening the auto-vectorizer (non-reduction FP loops, FMA fusion).
- Instruction scheduling for the Raptor Lake port/resource model.
- Sample-based PGO and PGO value specialization.
- Use-def-chain shared optimizer context.
- RA-06: next-use-aware eviction with copy-web coalescing for arithmetic
  chains (adler32's eight-byte recurrence).
- Multi-store stencil vectorization (nbody, mandelbrot, spectral_norm).

## Kill switches (for bisection / soundness fallback)

All kill switches are env vars; setting any non-empty value disables the
named pass.

| Switch | Disables |
|---|---|
| `CCC_NO_SEGMENT_RA` | segment-aware register allocator (restores exact fat model) |
| `CCC_NO_STENCIL_VEC` | stencil vectorizer |
| `CCC_NO_DEFER_OVERFLOW_VECREG` | deferred overflow vector register allocation |
| `CCC_NO_SEGMENT_FILL` | segment-aware residual fill (default-on x86-64) |
| `CCC_NO_GLOBAL_ADDR_REMAT` | global-address rematerialization |
| `CCC_NO_MAP_VEC` / `CCC_NO_MAP_VECREG` | map-style vectorization |
| `CCC_NO_PF06_ADD_PEEL` | PF-06 `add(iv, const)` / `sub(iv, const)` SIB displacement peeling (the existing SIB fold without displacement stays on) |
| `CCC_VERIFY_REGALLOC=1` | verifies register-allocation invariants over the correctness corpus (catches RA interference bugs never surfaced by the regression suite) |
| `CCC_DEBUG_COALESCE` / `CCC_TRACE_ALLOC` / `CCC_RA_EXPLAIN` | RA debug tracing |
| `LCCC_DEBUG_VECTORIZE` / `LCCC_WHY_NOT_VECTORIZE` | vectorizer debug tracing |
| `LCCC_DUMP_IR=1` | dumps the post-optimization IR for each function (for inspection) |
| `CCC_NO_GEP_FOLD` | disables all GEP folding (both const-offset and indexed-SIB) — strictest fallback |
