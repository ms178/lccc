# Subsystem: x86 CPU tuning model (`src/backend/x86/cpu_model.rs`)

**Status: live.** Consolidated 2026-09-16 from `docs/CPU_MODEL_AUDIT.md` (deleted) and
`docs/FOLLOWUP_CPU_MODEL.md` (deleted); validated facts as of upstream
`8ca2fd4`. When a row or policy changes, update *this* file, not a new one.

---

## 1. Model

One row per tuned uarch (`generic`, `skylake`, `haswell`, `raptorlake`,
`gracemont`, `znver3`, …) carrying latency/µop/cache/issue/width data
probed from **uops.info** (`scripts/uops_info_probe.py`), glibc
`dl-cacheinfo.h`, Intel ARK/ORM, Agner. The RaptorLake rows are ADL-P
provenance by construction (no public RPL-P-in-Golden-Cove-derivative
table exists at the row granularity the model needs; deltas are measured
and recorded per field — never inherit silently). **The `Generic` row is
the historical-constant envelope**: `generic_is_the_conservative_envelope_
and_preserves_defaults` pins that untuned builds reproduce the pre-model
constants (arm budget 8, issue width 4, vector loop for 4096 B, `×12` =
`lea (r,r,2); shl $2`). Changing `Generic` silently retunes every CPU and
every oracle baseline — treat rows below it as compatible-superset only.

Decision functions and their *consumers* (a function without a consumer
is documentation, not a model — grep before extending):

| Decision fn | Consumer |
|---|---|
| `memcpy_strategy` / `block_copy_vector_bytes` / `rep_movsb_threshold_for` | `x86/codegen/memory.rs` block copies > 64 B |
| `if_convert_arm_budget` | `passes/if_convert.rs` (diamond + triangle) |
| `issue_width` | `passes/reassoc_accum.rs` throughput bound |
| `mul_const_plan` / `mul_const_op_budget` | `x86/codegen/isel.rs` Mul-by-constant |
| `prefer_shlx` / `shlx_saves_move` | `isel.rs` (PR #397) |
| `break_popcnt_dep` / `break_lzcnt_tzcnt_dep` | `float_ops.rs` (PR #397) |
| `memset_strategy`, `libcall_above_bytes`, `fma/fadd_reduction_accumulators`, `avoid_lea3_on_critical_path`, `on_core_bytes` | **none** — wiring notes below, do not fake-consume |

Boxes to check before wiring one of the unconsumed rows:

- **`fma/fadd_reduction_accumulators`**: `max(fma_acc, fadd_acc)` rounded
  pow-2 in [4,8] is the LLVM `X86LoopUnrollPreferences` shape, BUT lccc's
  unroller is IR-level on the `accumulator_*` fast-math/integer path —
  wiring it into `loop_unroll::choose_unroll_factor` is a **fake**
  consumer (every row rounds to the same factor). Wire the FP-accumulator
  split first or leave the row out.
- **`avoid_lea3_on_critical_path`**: needs a post-RA LEA-form walk
  (LLVM `X86FixupLEAs`); `lea3_uops`/`lea3_rtp_x100` are already in the row.
- **`memset_strategy` / `libcall_above_bytes`**: partially realised via
  `zero_init_region` (below); the row-level `rep stosb` policy already
  models the RPL 4096 B choice.

## 2. Verified policy decisions (measure, do not re-litigate)

- **`MispredictPenalty`**: kept at **17** (chipsandcheese Golden Cove
  ≈17 from µop cache; Agner "~17"). LLVM's ADL-P model says 19; lccc's
  arm budget uses `penalty/2`, and policy is the low end of measured so
  speculation budgets are never optimistic. 19 would raise it 8→9.
- **`movbe`** (LLVM fork set `TuningFastMOVBE` for RPL/Gracemont): NOT
  adopted for the P-core — `MOVBE_R64_M64` on ADL-P is 3 µops
  (p06+p1+p23A), rTP 1.00, worse than SKL (0.50) and worse than
  `mov`+`bswap`. ADL-E lat 5 single µop; ARL-P rTP 0.33. When a movbe
  emitter is added it must be gated **per row** (`movbe_load_uops`), never
  by lineage.
- **`rep movsb`**: ERMS+FSRM on ADL **and** RPL rows; measurable win from
  2112 B up on RPL (the 2048/8192/16384-recorded comparison vs the ymm
  loop — the row decides, `copy_2112` emits `movl $2112,%ecx; rep movsb;
  ret` on raptorlake only).
- **`-march=raptorlake` / `-march=meteorlake` / `-march=gracemont` /
  `alderlake-n`**: resolved to the correct rows (pre-#399 raptorlake and
  meteorlake silently aliased the ADL row; Gracemont-only parts silently
  took the P-core row).
- **`-march=raptorlake` never implies APX/EGPR** — see
  [`x86-apx-evex.md`](x86-apx-evex.md).

## 3. Stop-conditions the rows revealed (live defect history)

1. **AVX emitted at plain `-march=x86-64`** for block copies > 64 B
   (`vmovdqu %ymm` unconditional in loop + remainder arms; only the
   exact-64 case consulted `avx2_enabled`) → SIGILL on pre-AVX hosts.
   Width now follows `block_copy_vector_bytes` (16 B SSE2; 32 B YMM;
   SNB/IVB stay 16 B: their 256-bit unaligned load is split).
   Regression: `cpu_model_memcpy_strategy.c`.
2. **`dirty_upper_ymm`** was not set by the copy loop → missing
   `vzeroupper` around the optimized path (HSW ~70-cycle state switch
   on the transition to an SSE function).
3. **`is_pure_xmm_overwrite` / string-op liveness**: the peephole must
   be prefix-aware (`movsbl`/`movsbw`/`movslq`/`movsd %xmm`/`repz ret`
   do not write through the string pointer); a liveness pass without a
   string-op transfer function deleted the `rep movs*` count set-up
   (`write to %rcx`) — the whole family of
   "prefix-shaped name chase" is why the classifier is an enumerated
   table, not a prefix test (see journal W3 09-13).
4. `zero_init_region`: `struct big b = {0};` (4096 B) was 512 ×
   `movq $0, d(%rsp)`. Rule: store ladder below
   `ZERO_INIT_STORE_LADDER_MAX = 128` (16×8 B = Clang's
   `MaxStoresPerMemset`) so SROA/DSE/store→load forwarding still see
   every store; above → `memset(p,0,n)`, whose expansion follows
   `memset_strategy`. `stage_copy_operands` orders the `%rdi`/`%rsi`
   fills against actual parameter homes (the memcpy dest-first trap from
   j-W2 C1): memset stages the fill value into `%rax` (never a home).

## 4. Tooling

```
LCCC_DUMP_TUNE=1   lccc -O2 -mtune=raptorlake -S x.c    # dump the row
scripts/tune_oracle.py matrix --tunes generic,skylake,raptorlake,gracemont,znver3 --flags=-O2
scripts/uops_info_probe.py                             # refresh rows from uops.info
LCCC_BIN=target/fastbuild/lccc scripts/run_regression_suite.sh cpu_model
```

Probe matrix shapes to keep (`acc.c`/`swap.c`/`pre.c`): accumulator-homed
destinations, swapped-argument memcpy/memset overlaps, pre-existing
state — every regression in this family was found by one of those three,
never by a single-kernel benchmark.
