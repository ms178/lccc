# Current compiler state

State refreshed against **`main`** (`ms178/lccc`) with Rust 2024 Edition / Rust 1.80–1.98.1 modernization, expanded 39-benchmark workload corpus, and verified Godbolt/Compiler Explorer competition oracles (GCC 16.2, Clang 23.1, ICC 2021.10, ICX latest).

The item catalog is [`agent/BACKLOG.md`](agent/BACKLOG.md); the active queue is [`tasks/`](tasks/README.md); the negative-results ledger is [`DECISIONS.md`](DECISIONS.md).

---

## What is production

- **Rust Modernization (Rust 1.80–1.98.1 / 2024 Edition):**
  - `std::sync::LazyLock` adopted across static caches, gates, and debug flags in passes, backend, and frontend.
  - Modern `Option::is_none_or`, `Option::is_some_and`, `Result::is_ok_and` throughout IR lowering, peepholes, passes, and backends.
  - Bounds-checked `slice::split_at_checked` in instruction decoders and encoders.
  - Clean unsigned integer `div_ceil` and `align_up` throughout stack layout and ELF writers.
- **C frontend** → SSA IR → `-O0` skip / `-O1` light / `-O2` full / `-O3` +unroll / `-Os`/`-Oz` size (`src/passes/README.md` is the authoritative tier list).
- **Linear-scan RA** in `src/backend/live_range.rs` (scan) + `src/backend/regalloc.rs` (policy). **Segment-aware interference is the scan's primary model** (RA-05a landed): `LiveRange::segs` + `segments_conflict`, coalesce-leader piece/use union, Phase-2f residual fill generalized to all targets and default-ON. Kill switches: `CCC_NO_SEGMENT_FILL`. Tier-2 hole-aware graph coloring is **production default** for eligible subsets (`CCC_NO_TIER2_GRAPH` restores scan-only).
- **MachInst Window Register Allocator:** SSA-driven MachInst window allocation with 85%+ instruction selection coverage.
- **ABI physical hints** (RA-26) retain leading ParamRefs across safe call-free x86 CFG leaves; ordered caller homes; stack-arg/mixed/calling shapes fail closed (`CCC_NO_LEAF_PARAM_GPR`, `CCC_NO_EMPTY_LOCAL_FRAME_ELISION`).
- **RA verifier**: `CCC_VERIFY_REGALLOC=1` hard-verifies segment interference, final assignments, and eviction occupancy history.
- **-O0** uses canonical stack homes on all four backends because phi elimination leaves non-SSA multi-def webs (RA-27).
- **Liveness** `src/backend/liveness.rs` — worklist backward dataflow (no `MAX_ITERATIONS` cap), fat `intervals` plus hole-aware `segments`.
- **FMA / FP contract**: `FpContract { Off, OnExpr, Fast }` threaded cli→pipeline→passes→backend. Default **Off** (GCC `gnu*` parity); `-ffp-contract=fast`/`-ffast-math` enable Fast; `-ffp-contract=on` contracts within a tagged statement root. Scalar and packed `vfmadd231*` emitters production.
- **Vectorizers**: reduction (single/multi/secondary accumulator), widening I32→I64 + masked conditional-sum, stencil (constant-tap affine), elementwise map expression trees, plain-copy; per-natural-loop PGO profitability gate.
- **Loop rotation** `src/passes/loop_rotate.rs` is **opt-in** (`CCC_LOOP_ROTATE=1`): correctness-clean for canonical counted loops.
- **DSE** `src/passes/dse.rs` (same-block, closed-alloca escape analysis, byte-range kills; `CCC_NO_DSE`); backedge PRE; GVN per-object epochs for disjoint non-escaping allocas + `restrict` params; GlobalAddr CSE with oracle-derived placement.
- **Aggregates**: AVX2 64-byte assignment = 2 YMM pairs + `vzeroupper`; 32/48-byte copies stay XMM. SysV all-SSE 16-byte struct returns use xmm0/xmm1.
- **Multi-arch**: x86-64, i686 (natural 4-byte slots, m16 boot pipeline, 32 KiB boot gate PASS), AArch64 (CASP, MOVW `:abs_g*:`, `.org`, PREL64, G1/G2/SABS reloc repair), RISC-V (va_arg struct{long double} end-to-end padding). Assembler + ELF linker in-tree for all four.

---

## Benchmark Corpus & Performance (39 Workloads)

The benchmark corpus contains **39 deterministic workloads** (33 historical + 6 new workload extracts: `chacha20_block`, `sha256_transform`, `linux_rbtree`, `zstd_count`, `lz4_compress`, `glibc_strstr`).

- **Aggregate LCCC / GCC Geometric Mean:** **`0.8598`** (LCCC faster overall than GCC 14.2 across the 39 benchmarks).
- **Aggregate LCCC / Fastest Ref Geometric Mean:** **`0.8936`**.
- **Correctness:** **39 / 39 (100%)** bit-identical to reference compilers.
- 2026-09-08 re-measurement after OP-05c/OP-05d (`--reps 3`, so indicative
  rather than certified — re-run at `--reps 7+` on a quiesced machine before
  quoting): LCCC/GCC geometric mean **`0.7597`**, correctness still 39/39.

### Elementwise map lowering — hot-loop density (2026-09-08)

Loop kernels must be ranked by **steady-state instructions per input byte**
(`scripts/hot_loop_metric.py`), never by `codegen_oracle.py`'s static
whole-function `insns` column: a compiler that refuses to vectorize emits
one tight scalar loop and "wins" that column while doing 32× less work per
instruction.  `/home/user/work/casefold.c`, `-O3 -march=x86-64-v3`:

| kernel | LCCC | GCC 16.2 | Clang 23.1 | ICC | ICX |
|---|---|---|---|---|---|
| `fold_lower` | **0.3125** | 1.2500 | 0.2422 (4× unrolled) | 2.0000 | 0.6250 |
| `fold_upper` | **0.3125** | 1.2500 | 0.2422 (4× unrolled) | 1.3750 | 0.6250 |
| `clamp_bytes` | 0.2500 | 0.1875 | 0.1172 (4× unrolled) | 0.4375 | 0.3750 |
| `classify_alpha` | **0.3125** (was scalar) | 0.3125 | 0.2109 (4× unrolled) | 4.2500 | 0.5625 |

LCCC wins or ties every non-unrolled competitor: 2.0× vs ICX and 4.0× vs
GCC on the case folds, 1.8× vs ICX and level with GCC on the classifier,
1.5× vs ICX on the clamp.  Clang's remaining lead everywhere is 4× unrolling
of identical per-element work — see the follow-up doc's TODO 3.2/3.3.

The `isalpha` classifier now lowers to three packed instructions
(`vpor` c|32, `vpaddb` bias, `vpcmpgtb`) via the single-bit window union plus
the range fusion; it used to be a scalar loop.

---

## Competition Oracle Matrix (Compiler Explorer)

| Benchmark / Function | LCCC Insns | GCC 16.2 | Clang 23.1 | Intel ICC | Intel ICX (latest) | Status |
|---|---:|---:|---:|---:|---:|---|
| `glibc_strstr` (`two_way_short_needle`) | **75** | 150 | 89 | 94 | 155 | **Best in the world (1.00×)** |
| `zstd_count` | **67** | 77 | 37 | 69 | 68 | **Beats GCC 16.2, ICC, ICX** |
| `sha256_transform` | **231** | 154 | 126 | 171 | 1069 | **Beats ICX by 4.6×** |
| `chacha20_block` | 592 | 232 | 176 | 1098 | **74** | Open gap: AVX2 QR vectorization |

---

## Priority Research Backlog (Next High-Impact Targets)

1. **ChaCha20 Quarter-Round SIMD / Unrolling:** ICX generates 74 instructions via AVX2 vector quarter-rounds; LCCC generates 592 instructions due to scalar quarter-round spill pressure.
2. **LZ4 Match Loop & Store Forwarding:** LZ4 compression loop needs better unaligned word load forwarding and fast hash table index registers.
3. **Loop Rotation Default-Enable (PF-17):** Hardening remaining 15 edge-case loop shapes to enable loop rotation by default (eliminating the extra entry jump in hot loops).
4. **MachInst Instruction Selection Coverage Expansion:** Expand MachInst window allocator to 95%+ of instruction forms.
