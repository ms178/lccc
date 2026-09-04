# CPU tuning model — follow-up work (living document)

Owner: whoever picks up the `cpu_model` line of work next.  Read
`docs/CPU_MODEL_AUDIT.md` first: it lists what PR #397/#399 claimed, what
was actually merged, and what v3 wires today.  Build with
`bash scripts/build_lccc_fast.sh` (fastbuild, `-j2`, needs the swap file
from `scripts/ensure_swap.sh` on a 4 GiB host).

## Accomplished (session `cpu-model-v3`)

* **Consumers** for the v2 rows (none existed at the base commit):
  block-copy strategy and vector width (`memory.rs`), if-conversion arm
  budget (`if_convert.rs`, both forms), reassoc throughput bound
  (`reassoc_accum.rs`), multiply-by-constant synthesis (`isel.rs` via
  `mul_const_plan`; ×10/×12/×−3/… now LEA/SHL/ADD/NEG chains, ×2 as
  `add r,r`).  Each reproduces the historical constant on `Generic`.
* **`rep_movsb_threshold_for(vector_bytes)`**: glibc's crossover depends on
  the vector width of the competing loop (2048 SSE2 / 8192 AVX / 16384
  AVX-512 / 2112 FSRM); the row stores the 32-B figure, the decision scales.
* **Straight-line copies** up to 8 vector stores (Clang
  `MaxStoresPerMemcpy`) instead of a 2–4 iteration loop.
* **Three pre-existing bugs fixed**: AVX emitted at `-march=x86-64`
  (SIGILL on pre-AVX hosts), missing `vzeroupper` arming after YMM copy
  loops, liveness peephole deleting the `%rcx` count before `rep movsb` /
  `rep stosb` (also broke inline-asm string ops).
* **`-march=` names**: `meteorlake`, `gracemont`, `alderlake-n`,
  `sierraforest`, `grandridge`, `clearwaterforest`.
* **Tooling/tests**: `scripts/tune_oracle.py` (`dump`/`matrix`/`compare`),
  block-copy probes in `tests/bench/cpu_model/tune_probe.c`, unit tests
  (`is_string_instruction`, two dataflow shapes), runtime regressions
  `cpu_model_memcpy_raptorlake.c`, `cpu_model_memcpy_strategy.c`,
  `cpu_model_mul_const_plan.c` (50 constants × 6 type/sign shapes × 18
  inputs, verified identical to GCC at `-O1/-O2/-O3`, `generic`,
  `gracemont`, `znver1`, `raptorlake`), `peephole_rep_string_liveness.c`.
* **Audit of the LLVM fork's `05-raptorlake.patch`** against live
  uops.info data; two of its tuning bits are contradicted by measurement
  (`FastMOVBE` on the P-core; `LoadLatency 4`), the rest is already
  modelled (`CPU_MODEL_AUDIT.md` §5).

## To do — ordered by (expected gain × breadth × confidence) / cost

1. **Return-value materialisation after in-place ALU.**  Every
   `long f(long x){return x*10;}` ends in `…, %rdi; movq %rdi, %rax; ret`
   because the RA homes the result in the argument register and copies at
   the return.  GCC/Clang compute straight into `%rax`.  This costs one
   µop per leaf function *and* blocks the distinct-source multiply plans
   (×7 = `lea (,r,8); sub src` needs src ≠ dst): with dst = `%rax` and src
   = `%rdi` every plan in `mul_const_plan` becomes reachable.  Fix in the
   allocator's return-value preference (prefer `%rax` for the value that
   flows into `ret`), then drop the `distinct_ok` fallback in isel.  Bench:
   `tests/regression/cpu_model_mul_const_plan.c` instruction count
   (`imul` count 472 → expect < 300 at generic, < 200 at gracemont).
2. **Index-only LEA (`lea (,%r,8), %d`) in MachInst.**  `MachInst::Lea`
   requires a base; `MulStep::LeaScale` is lowered as `mov; shl`.  Add
   `base: Option<MachReg>` (14 construction sites), emit `lea (,r,s)` for
   ×2/4/8-then-add/sub shapes → one instruction shorter than today and
   identical to GCC for ×7, ×17, ×24, ×40.
3. **Reduction accumulator splitting.**  `fma_reduction_accumulators()`
   (8 on SKL+/RPL, 10 on HSW/Zen2, 5 on Zen1) has no real consumer (see
   AUDIT §6 for why the unroll factor is not one).  Implement an
   accumulator splitter in `passes/vectorize.rs` (`accumulator_*`) for
   integer and `-ffast-math` FP reductions, `k = row value` partial sums,
   horizontal reduction in the exit block.  Bench `k_adler32*.c`, a
   `dot`/`sum` kernel; the SKL/Zen1 split must differ.
4. **3-operand LEA fixup pass.**  `avoid_lea3_on_critical_path()` is derived
   but no pass splits `lea d(b,i,s)` on loop-carried chains.  LLVM's
   `X86FixupLEAs` does it for SNB–ADL; lccc should touch only chains the
   loop analysis marks recurrence-critical and skip ICL/SPR (P-core LEA is
   1 cycle, no E-core).  Needs a MachInst dependency walk.
5. **memset lowering.**  Constant-size `memset` still goes to
   `call memset@PLT` for every size.  Mirror the memcpy strategy: ≤ 8
   vector stores straight-line (`vpxor` + `vmovdqu`), `rep stosb` ≥
   `rep_stosb_threshold` (2048) on ERMS rows, vector loop otherwise,
   libcall above `libcall_above_bytes()`.  The liveness fix already models
   `rep stosb`.
6. **Non-temporal / libcall upper bound for huge constant copies.**
   glibc streams with NT stores above `L3/4` (`libcall_above_bytes`);
   `memcpy_call_strategy` returns `LibCall` there but the block-copy
   emitter has no libcall path that preserves live caller-saved registers.
7. **Vectoriser VF cost hook.**  Gracemont double-pumps 256-bit ops and
   has `pmulld` at 2 µops/10 cycles on HSW–ADL vs 1 µop on Zen; the
   vector/scalar and interleave decisions should see `pmulld_*` and a
   `prefer_vector_bits` per row (128 on Gracemont per LLVM `Prefer128Bit`,
   256 on every P-core/Zen; 512 only for SPR/Zen4/Zen5 once AVX-512
   lowering exists).
8. **`movbe` emitter, gated per row.**  uops.info: `MOVBE r64,m64` is
   3 µops / rTP 1.0 on Golden Cove (worse than load+`bswap`), 1 µop on
   Gracemont, rTP 0.33 on Lion Cove.  Add `movbe_load_uops` to the row
   *with* the emitter (bswap-of-load in `alu.rs`), never before.
9. **Branch-probability-aware if-conversion.**  Budget =
   `penalty × P(mispredict) / (1 + cmov_latency)` once PGO or
   `block_layout` static heuristics provide `P`; feed `cmov_uops`
   (2 on SNB–HSW) into the select cost.
10. **JCC erratum padding** for SKL-class rows
    (`-mbranches-within-32B-boundaries` semantics in the assembler; GAS
    2.47 is the encoding oracle).
11. **Prefetch distance / tiling** from `cache` + `dram_latency_cycles`
    (`on_core_bytes()`); no consumer yet.
12. **Hardware validation on the i7-14700KF.**  Run
    `tests/bench/cpu_model/tune_probe.c` and the copy matrix under
    `perf stat -e cycles,instructions,branch-misses` for
    `-mtune=raptorlake|alderlake|generic`; re-derive `rep_movsb_threshold`
    if the measured crossover differs from 2112 (glibc's number is for
    `memmove-vec-unaligned-erms`, which aligns the destination first — the
    inline form does not).  Record under `engineering/evidence/`.
13. **Regression-suite oracle flags.**  `cpu_model_memcpy_raptorlake.flags`
    passes `-mtune=raptorlake` to the GCC oracle too; GCC < 13 rejects the
    name and the runner reports SKIP.  Add a `.gccflags` sidecar (oracle
    flag override) to `run_regression_suite.sh` so the lccc side keeps the
    RPL row while old oracles get `-mtune=alderlake`.

## Known limits (documented, not hidden)

* Rows model one representative SKU per generation for L3 size; `native`
  is exact, `-mtune=<name>` is representative.
* Mispredict penalties are the low end of published ranges (deliberately
  pessimistic so speculation budgets never grow on an optimistic number).
* uops.info has no Raptor Lake column; RPL instruction numbers are ADL-P's
  by construction (same core) — also what LLVM's `AlderlakePModel` assumes
  for `raptorlake`.  The RPL row differs in caches, DRAM cycles and the
  E-core cluster (4 MiB L2, 12 E-cores on the i7-14700KF).
* `mul_const_plan` distinct-source shapes are skipped when the RA homes
  the result in the multiplicand's register (item 1).
