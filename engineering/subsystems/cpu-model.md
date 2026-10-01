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
| `break_popcnt_dep` / `break_lzcnt_dep` / `break_tzcnt_dep` | `float_ops.rs` (PR #397; LZCNT/TZCNT split in the 2026-09-30 audit) |
| `bypass_div64` | `alu.rs::emit_div64` + `machinst_emit.rs` `MachInst::Div` (2026-09-30 audit) |
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

---

## 5. Red-team audit of PR #397 against the complete uops.info dataset (2026-09-30)

**Method.** `scripts/uops_xml.py` indexes the full `instructions.xml` (15 604
instruction pages, dataset date 2026-03-29). `scripts/cpu_model_audit.py` dumps
every row (`LCCC_DUMP_TUNE=1`) and re-derives 22 measurable fields per row from
the published measurement column. Result before the fixes: **295 verified, 18
MISMATCH**; after: **407 verified, 0 MISMATCH, 0 allow-listed**. Re-run it
after *every* row edit (`scripts/cpu_model_audit.py`, exit 1 on drift; a
deliberate deviation needs a reasoned line in `scripts/cpu_model_audit_allow.txt`).

**Defects found and fixed (all data-driven):**

| # | Defect | Evidence | Fix |
|---|---|---|---|
| 1 | `lzcnt_tzcnt_false_dep` was one bit | `[uops.info]` TZCNT_R64_R64 ZEN5 op1→op1 = **1**, LZCNT/POPCNT 0 | split into `lzcnt_false_dep` / `tzcnt_false_dep`; Zen 5 breaks TZCNT, not LZCNT (consumer `float_ops.rs`) |
| 2 | `div64_rtp_x100` stored the *port-computed* `TP_ports` | ICL/ADL-P DIV_R64: ports 3.0, **measured 10.0**; SKL 8.25 vs 21.06; SNB 11 vs 22.1 | rows now hold measured `TP_loop` (SNB 22, HSW..SKX 21, ICL/ADL 10, ADL-E 6, Zen3+ 7) |
| 3 | no 32-bit divide data at all | DIV_R32 SKL 23–28 / TP 6 vs DIV_R64 35–90 / TP 21 | `div32_{latency_min,latency,rtp_x100}` on every row |
| 4 | Haswell/Broadwell `vec_int_alu_pipes = 3` | VPADDD ymm HSW/BDW **1\*p15** (p015 only from SKL) | 2 (Skylake re-widened to 3) |
| 5 | Meteor Lake aliased to the Raptor Lake row | MTL-P VMUL{PS,PD,SS,SD} **3** vs ADL-P/EMR 4; 198 of 3779 shared pages differ | new `MeteorLake` row (Redwood Cove) + `CRESTMONT_MTL` E-core; CPUID 0xAA/0xAC/0xB5 map to it |
| 6 | stale numbers after the dataset refresh | ARL-P IMUL 4 (was 3), PMULLD 3 µops (was 2), ADL-E DIV 11–44 (43), Zen3/4 DIV max 19 (18), SKX 89 (90), ADL-E DIV TP 6.0 (was an unsourced 15) | rows updated |

**Raptor Lake vs Alder Lake — what is *measured*, and what is not.** uops.info
has no RPL column; Raptor Cove's instruction timings are Golden Cove's. This is
now checkable instead of asserted: ADL-P vs EMR (Golden Cove server) agree on
every scalar/256-bit timing (the 913 differing pages are AVX-512 forms and
non-temporal-store noise). The RPL row therefore differs from ADL only in what
hardware documentation supports: 2 MiB/16-way L2, 4 MiB E-cluster L2, larger
L3, clock-scaled DRAM latency (cache *latency* cells are scale figures, not
re-measured in this session — see follow-ups).

**LLVM fork `05-raptorlake.patch` — verdicts, each against data:**

| Fork change | Verdict | Why |
|---|---|---|
| drop `TuningLZCNTFalseDeps` on ADL/RPL | already modelled | op1→op1 = 0 on ADL-P (and SKL+) |
| `FeatureERMSB`/`FeatureFSRM` on RPL | already modelled | rows carry `erms`/`fsrm`, `rep_movsb_threshold` 2112 |
| `FMul`/`FMul64` latency 4→3 for **RPL** | **rejected for RPL, adopted for MTL** | ADL-P 4, EMR 4, MTL-P 3 |
| `Div32` 15→13, `Div64` 18→16 | consistent, now *stronger* | measured ranges 10–15 / 14–18 + measured TP 6 / 10 |
| `LoadLatency` 5→4, `MispredictPenalty` 14→19 | not adopted | not measurable by uops.info; 17 is the low end of the published range (see §2) |
| `TuningFastMOVBE` | rejected (data) | MOVBE_R64_M64 ADL-P 3 µops, rTP 1.0 |
| `TuningSBBDepBreaking` | not adopted | uops.info SBB pages measure distinct-register forms only; host probe (`scripts/uarch_probe.sh`) answers it for the Intel P-core lineage (see evidence below) |
| `TuningPrefer256Bit` | moot | RPL has no AVX-512; `prefer_vector_bits` already 256 |
| Gracemont `Prefer128Bit`/`NoDomainDelay` | not modelled | `simd_datapath_bits: 128` already drives 128-bit preference |

**New decision, with a consumer: `bypass_div64()`** (LLVM `idivq-to-divl`,
derived from measured throughput, not lineage). `DIV r64` TP ≥ 2× `DIV r32` TP
and best-case latency above DIV r32's worst: on for SNB/IVB/HSW/BDW/SKL/SKX,
off for ICL+, all Zen, Gracemont and `Generic`. Emitted as
`mov/or/shr $32; jnz; divl; … slow: [c]qto/xor; [i]divq` by both division
emitters; the dead `xor %edx,%edx`/`cqto` before the guard is dropped.
**It is a run-time speculation, not a proof.**  The guard tests the operands at run time; no
range analysis in the compiler establishes that they fit — so the bypass is emitted for *every*
64-bit divide on a bypass-enabled row, and `CCC_NO_DIV64_BYPASS=1` is the process-wide kill switch
that restores the plain `cqto`/`xorl %edx,%edx` + `[i]divq` sequence.  The decision itself is the
pure function `bypass_div64_enabled_for(tune, kill_switch)`; the emitters take the row explicitly
(`emit_machinst_with_tune`), so a test can pin any row without touching process state, and the
guard's labels come from the emitter's single label allocator (`AsmOutput::fresh_label`), not from
hand-written `1:`/`2:`.

Regression: `cpu_model_div64_bypass_skylake` (edge grid + 200 000 random
width-mixed operands, checked against `__int128`) and
`cpu_model_div64_nobypass_raptorlake` (same source, guard absent).

## 6. Follow-up work (open, prioritised)

1. **Register-allocation defect visible in the new asm**: `unsigned long f(a,b){return a/b;}`
   pushes `%rbx`/`%r12` and copies both arguments into them before a single
   `div`; GCC/Clang emit `mov %rdi,%rax; xor %edx,%edx; div %rsi; ret`. The
   allocator reserves callee-saved registers for values that do not live across
   any call. Pre-existing, affects every division.
2. Wire `avoid_lea3_on_critical_path`, `memset_strategy`, `libcall_above_bytes`
   (still no consumer; see §1 boxes).
3. Extend the bypass to `i32`→`i16` (`TuningSlowDivide32`-style) only if a
   measured core needs it (none in the dataset).
4. Re-measure cache latencies (L2/L3/DRAM cells) with a pointer-chase on real
   RPL/MTL hardware; they are scale figures and unverified here. The host of
   this session is an Ice Lake-SP VM, so only Sunny Cove cells can be checked
   (`scripts/uarch_probe.sh` covers LEA/IMUL/POPCNT/LZCNT/TZCNT/SBB/DIV).
5. CPU-model CI gate: run `scripts/cpu_model_audit.py` in `ci_local.sh` once the
   uops.info XML can be cached in CI (142 MB; fetch is skipped offline).
6. Consider a `-mtune=generic` **default flip** of `bypass_div64` (LLVM's generic
   enables it) once a benchmark on Zen3+/ICL+ shows the extra 3 µops are free.
