# CPU tuning model — red-team audit of PR #397 / #399 and the v3 consumers

Session: `cpu-model-v3` (base `f744b007`, upstream `ms178/lccc` main after
PR #399).  Scope: `src/backend/x86/cpu_model.rs`, everything it *claims* to
drive, and the alignment with the LLVM cost model — in particular the
owner's LLVM fork patch `05-raptorlake.patch` (`X86.td`,
`X86SchedAlderlakeP.td`, `X86TargetTransformInfo.cpp`).

Every number below was re-read from the primary source in this session
(`scripts/uops_info_probe.py` against the live uops.info tables; glibc
`sysdeps/x86/dl-cacheinfo.h`; Intel ARK; Intel ORM; Agner Fog 2024).  Where
this document and the LLVM fork disagree, the measurement and the reason
are both given.

## 1. What PR #399 merged versus what its message claimed

PR #399 (`ae05c68`) shipped the v2 **row data** and its decision functions,
`scripts/uops_info_probe.py` and the snapshot script.  It did **not** ship
the consumers, the bug fixes, the regression tests or the two documents the
row comments reference.  Grep at the base commit:

| Decision function | Consumers at `f744b007` | Consumer after this session |
|---|---|---|
| `memcpy_strategy`, `block_copy_vector_bytes`, `rep_movsb_threshold_for` | 0 | `x86/codegen/memory.rs` block copies > 64 B |
| `if_convert_arm_budget` | 0 | `passes/if_convert.rs` (diamond + triangle forms) |
| `issue_width` | 0 | `passes/reassoc_accum.rs` throughput bound |
| `mul_const_plan` / `mul_const_op_budget` | 0 | `x86/codegen/isel.rs` Mul-by-constant |
| `prefer_shlx` / `shlx_saves_move` | isel (PR #397) | unchanged |
| `break_popcnt_dep` / `break_lzcnt_tzcnt_dep` | float_ops (PR #397) | unchanged |
| `memset_strategy`, `libcall_above_bytes`, `fma/fadd_reduction_accumulators`, `avoid_lea3_on_critical_path`, `on_core_bytes` | 0 | still 0 — see `FOLLOWUP_CPU_MODEL.md` (each has a concrete plan or a documented reason not to wire it) |

`docs/CPU_MODEL_AUDIT.md`, `docs/FOLLOWUP_CPU_MODEL.md`,
`scripts/tune_oracle.py` and the three runtime regressions named in the
v2 write-up were absent from the tree.  All are delivered now.

## 2. Findings against PR #397 (`f9f9085`) — still valid, re-verified

| # | Severity | Finding | State |
|---|----------|---------|-------|
| 1 | defect | `fma_reduction_accumulators()` / `fadd_reduction_accumulators()` computed, dumped, consumed by nothing. | Still unconsumed; §6 explains why wiring it into `loop_unroll::choose_unroll_factor` would be a *fake* consumer (every row rounds to the same factor). |
| 2 | defect | `raptorlake`/`meteorlake` resolved to the Alder Lake row. | Fixed in v2 (dedicated row); `-march=meteorlake` now also accepted (§4). |
| 3 | defect | Gracemont-only parts resolved to the P-core row. | Fixed in v2; `-march=gracemont|sierraforest|grandridge|clearwaterforest|alderlake-n` now accepted with the correct ISA profile (§4). |
| 4 | defect | Referenced audit/follow-up documents did not exist. | This file + `FOLLOWUP_CPU_MODEL.md`. |
| 5 | data | `lsd_uops = 192` (ADL-P) unsourced; ICL LSD = 0 wrong. | 144 / 70 with provenance (v2). |
| 6 | gap | No cache/latency/mispredict/width numbers. | v2 rows; v3 consumers. |
| 7 | gap | 3-op LEA modelled by latency only. | `lea3_uops`, `lea3_rtp_x100` (v2); decision still unconsumed (FOLLOWUP §2). |
| 8 | envelope | `Generic.fma_pipes = 2` though Zen1 has 1. | Fixed (v2). |
| 9 | test gap | Nothing pinned that `Generic` reproduces pre-model constants. | `generic_is_the_conservative_envelope_and_preserves_defaults` (v2) + the consumers in v3 reproduce the historical constants on `Generic` (arm budget 8, width 4, vector loop for 4096 B). |

## 3. Pre-existing bugs fixed while wiring the consumers

### 3.1 Miscompile: AVX at `-march=x86-64` for block copies > 64 B
`emit_memcpy_impl_impl` emitted `vmovdqu %ymm0/%ymm1` **unconditionally**
in the loop path and in the 32-byte remainder arm; `avx2_enabled` was only
consulted for the exact-64-byte case.  A 4096-byte struct assignment at
plain `-march=x86-64` SIGILLs on a pre-AVX host and breaks the `-march`
contract everywhere.  Now the width follows
`block_copy_vector_bytes(avx2_enabled)` (16 B SSE2 / 32 B YMM; SNB/IVB stay
at 16 B because their 256-bit unaligned load is split).  Regression:
`tests/regression/cpu_model_memcpy_strategy.c` (baseline),
`cpu_model_memcpy_raptorlake.c` (FSRM row).

### 3.2 Missing `vzeroupper` arming after the YMM copy loop
The loop wrote 256-bit registers without setting `dirty_upper_ymm`, so the
epilogue `vzeroupper` was not emitted and every later legacy-SSE scalar FP
instruction paid the AVX→SSE transition (SKL+: false dependency on each
op; HSW: ~70-cycle state switch).  Fixed (both loop and straight-line
forms arm it; the 16-byte tail stays in the VEX domain once a YMM is dirty).

### 3.3 Latent miscompile: string instructions invisible to liveness
`codegen/peephole/passes/liveness.rs` had no transfer function for
`rep movs*`/`rep stos*`/`movs*`/`stos*`/`lods*`/`scas*`/`cmps*`.  They name
no register, so `movq $4096, %rcx; rep movsb; ret` was analysed as a dead
write to `%rcx` and **the count set-up was deleted**.  This affected the
existing `-mno-sse` kernel path (`rep movsb` for every block copy) and any
inline-asm string op.  Fix: `is_string_instruction()` (prefix-aware;
rejects `movsbl %al,%eax`, `movslq`, `movsd %xmm`, `repz ret`) → reads and
writes `{rax, rcx, rsi, rdi}`.  Unit tests for the predicate and for the
two dataflow shapes; runtime regression `peephole_rep_string_liveness.c`
(inline asm, independent of the memcpy lowering).

## 4. Consumers wired in v3 (each reproduces the historical default on `Generic`)

| Decision | Row field(s) | Consumer | Generic | Skylake | Raptor Lake | Gracemont | Zen3 |
|---|---|---|---|---|---|---|---|
| Block copy 65 B … 8×vb | — | `memory.rs` | straight-line vector moves (≤ 8 stores, alternating scratch regs) | same | same | same | same |
| Block copy above that | `erms`, `fsrm`, `rep_movsb_threshold_for(vb)` | `memory.rs` | vector loop | `rep movsb` ≥ 2048 B (16-B loop) / ≥ 8192 B (32-B loop) | `rep movsb` ≥ 2112 B | `rep movsb` ≥ 2112 B | `rep movsb` ≥ 2048 / 8192 B |
| Vector width of the copy | `avx256_unaligned_split` × `-march` | `memory.rs` | 16 B at baseline, 32 B with AVX2 | 32 B | 32 B | 32 B | 32 B |
| Branch→select arm budget | `mispredict_penalty` | `if_convert.rs` | 8 | 8 | 8 (17/2) | 6 (13/2) | 6 (13/2) |
| Adler chain-break throughput bound | `dispatch_width` | `reassoc_accum.rs` | 4 | 4 | 6 | 5 | 6 |
| Mul-by-constant synthesis | `imul64_latency` → `mul_const_op_budget` | `isel.rs` | 2 steps | 2 | 2 | **4** | 2 |
| SHLX selection | `shift_cl_uops` | isel (PR #397) | always | always | always | when it saves a mov | when it saves a mov |
| False-dep xor breaks | `popcnt_false_dep`, `lzcnt_tzcnt_false_dep` | float_ops (PR #397) | on | popcnt only | off | off | off |

`-march=` names added: `meteorlake`, `gracemont`, `alderlake-n` (Alder
Lake ISA), `sierraforest`, `grandridge` (+AVX-IFMA/NE-CONVERT/VNNI-INT8/
CMPCCXADD, = GCC `PTA_SIERRAFOREST`), `clearwaterforest` (+AVX-VNNI-INT16).
`cpu_model::resolve` maps them to the Gracemont row, `meteorlake` to the
Raptor Lake class.

Observed output (`scripts/tune_oracle.py matrix`, `-O2 -march=x86-64-v3`):

| function | generic | skylake | raptorlake | znver3 |
|---|---|---|---|---|
| `copy_200` | 6×`vmovdqu ymm` pairs + `movq` tail, `vzeroupper` (16 insns) | same | same | same |
| `copy_2112` | ymm loop | ymm loop | `movl $2112,%ecx; rep movsb; ret` | ymm loop |
| `copy_8192` | ymm loop | `rep movsb` | `rep movsb` | `rep movsb` |
| `x*10` | `lea (r,r,4); add r,r` | same | same | same |
| `x*12` | `lea (r,r,2); shl $2` | same | same | same |
| `x*-3` | `lea (r,r,2); neg` | same | same | same |
| `x*100` | `imul $100` | same | same | same |

## 5. Alignment with LLVM and with the owner's `05-raptorlake.patch`

| LLVM-fork change | uops.info / primary source (this session) | lccc v3 |
|---|---|---|
| `raptorlake` gets `FeatureERMSB`, `FeatureFSRM` | CPUID: Golden/Raptor Cove report ERMS+FSRM (ADL already has both) | `erms=true, fsrm=true` on ADL **and** RPL rows; drives `rep movsb` ≥ 2112 B |
| `TuningLZCNTFalseDeps` removed from ADL | `TZCNT_R64_R64`/`LZCNT` ADL-P: lat 1→1 = 0 (no output dep) | `lzcnt_tzcnt_false_dep=false` on ICL+, ADL, RPL, Zen — and on SKL, where uops.info also shows none (GCC 16.2 still emits the xor there) |
| `TuningPOPCNTFalseDeps` removed from Gracemont | `POPCNT_R64_R64` ADL-E: no measured dep | `popcnt_false_dep=false` on the Gracemont row |
| `TuningFastMOVBE` for RPL and Gracemont | **`MOVBE_R64_M64` ADL-P: 3 µops (p06+p1+p23A), rTP 1.00** — *worse* than SKL (rTP 0.50) and worse than `mov`+`bswap` (2 µops, rTP 0.5). ADL-E: lat 5, single µop. ARL-P: rTP 0.33. | Not adopted for the P-core: the measurement contradicts it for the 64-bit load form. lccc has no `movbe` emitter today; when one is added it must be gated per row (`movbe_load_uops`), *not* by lineage. |
| `TuningSBBDepBreaking` for RPL | `sbb r,r` is flag-dependent on every Intel core (Agner); LLVM already carries the bit from SNB tuning | No consumer in lccc (no `sbb r,r` idiom emitted); nothing to align. |
| `TuningPrefer256Bit` for RPL | RPL has no AVX-512 | Not applicable. |
| `MispredictPenalty = 19` (ADL-P model) | chipsandcheese Golden Cove: 17 cycles from the µop cache, 20+ from L1i; Agner: "~17" | **17** kept: the arm budget uses `penalty/2`, and lccc's policy is the low end of the measured range so speculation budgets are never optimistic. 19 would raise the budget from 8 to 9 instructions per arm. |
| `LoadLatency = 4` (ADL-P model) | Intel ORM §Golden Cove: L1D load-to-use **5** cycles (simple addressing); uops.info `MOV (R64, M64)` ADL-P ≤ 5 | **5** kept (`load_latency`, `cache.l1d.latency`). The fork's 4 is Skylake's number. |
| `WriteDiv32` 13 / `WriteDiv64` 16 | `DIV_R64` ADL-P: latency 14–18, rTP 3.0 (3×p1); ICL/TGL/RKL identical | `div64_latency_min=14`, `div64_latency=18`, `div64_rtp_x100=300` — the range, not a midpoint |
| `getBranchMispredictPenalty()` reads the sched model instead of a constant 14 | — | Same architecture: `if_convert` reads the row, never a constant (the row is `Generic` = 16 → budget 8 when untuned) |
| Gracemont `TuningPrefer128Bit`, `TuningNoDomainDelay` | Gracemont executes 256-bit ops as 2×128; no bypass delay between int/fp SIMD domains (Agner) | Not modelled: lccc's vectoriser has no VF cost hook yet (FOLLOWUP §6); block copies keep 32-B moves because per-byte throughput is identical and the instruction count halves. |

Where the fork is right and upstream LLVM is not (`raptorlake` = plain
`AlderlakePModel` with ADL tuning), lccc already had the dedicated row
(v2) and now the consumers that make it observable (§4).

## 6. Why some derived facts remain unconsumed (and what would make them real)

* **Reduction accumulators** (`fma_latency × fma_pipes`, `fadd_latency × 2`):
  8 on SKL/ICL/ADL/RPL/Zen3+, 10 on HSW and Zen2, 6 on Generic, 5 on
  Zen1.  `loop_unroll::choose_unroll_factor` unrolls tiny bodies ×8; taking
  `max(fma_acc, fadd_acc)` rounded to a power of two and clamped to [4, 8]
  yields **8 on every row** — a consumer with no observable divergence is a
  fake consumer.  The real consumer is a reduction *splitter* (vectoriser
  `accumulator_*` path under `-ffast-math`/integer), which does not exist
  yet.  Plan in FOLLOWUP §1.
* **`avoid_lea3_on_critical_path`**: needs a MachInst-level dependency
  walk (LLVM `X86FixupLEAs`); FOLLOWUP §2.
* **`memset_strategy`**: the backend has no inline memset today
  (`call memset@PLT` for every size); FOLLOWUP §3 (the liveness fix
  already models `rep stosb`).

## 7. How to inspect

```
LCCC_DUMP_TUNE=1   lccc -O2 -mtune=raptorlake -S x.c      # one row
LCCC_DUMP_TUNE=all lccc -O2 -S x.c                        # every row
scripts/tune_oracle.py dump --all --json
scripts/tune_oracle.py matrix --tunes generic,skylake,raptorlake,gracemont,znver3 --flags=-O2
scripts/tune_oracle.py compare --flags="-O2 -march=x86-64-v3 -mtune=raptorlake" --oracles gcc16.2,clang,icx
scripts/uops_info_probe.py DIV_R64 MOVBE_R64_M64 LEA_B_I_D8_R64 SHL_R64_CL
LCCC_BIN=target/fastbuild/lccc scripts/run_regression_suite.sh cpu_model
```
