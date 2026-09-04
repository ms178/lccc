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


## Accomplished (session `cpu-model-v4`)

* **memset lowering shipped** (was item 5): `X86Tune::memset_strategy` and
  `rep_stosb_threshold` finally drive code — overlapping scalar stores,
  straight-line/looped vector stores, `rep stosb` on ERMS rows, libcall
  above ¼ L3; `-mno-sse` and `__memset_chk` covered.  Bodies match Clang;
  `memset(p,0,4096)` is 3 body instructions on Raptor Lake.
* **Large zero-initialisers** route through the same expansion above
  128 bytes (`ZERO_INIT_STORE_LADDER_MAX`): 4096-byte `= {0}` went from
  512 stores to `rep stosb`.
* **Inline libc expansions no longer poison leaf functions**: parameters
  keep their ABI registers, no callee-saved push/pop, no alignment pad;
  the three predicates (emitter, RA, prologue) share one decision function.
  Two hazards this exposed (result capture after clobber, swapped-operand
  staging) were fixed and are pinned by the probe matrix; the unused
  result copy is gone.
* **Tests**: `cpu_model_memset_inline.c`, `cpu_model_memset_raptorlake.c`
  (+ `.flags`); AUDIT §7 records the oracle table.


## P0 found on the way (pre-existing, NOT introduced here — needs its own session)

* **`tests/regression/vec_alias_versioning.c` mismatches GCC** at
  `-O3 -march=x86-64-v3 -ffp-contract=off`: `scale_f[+2B]` (destination
  `y = (float*)((char*)x + 2)`, i.e. a *sub-element* overlap) prints
  `1.00174087e+24` where GCC prints `2.00264183e+23`; the final hash differs.
  Bisected this session: the base commit `648fd97` binary produces the
  identical (wrong) output, so the vectoriser's runtime alias-versioning
  check treats a 2-byte forward overlap as "no overlap" (distance rounded
  to whole elements) and runs the vector body.  Fix in
  `passes/vectorize.rs` (alias check must compare byte ranges, not element
  indices) and add the +1/+2/+3-byte offsets to the check's unit tests.
  `asm_cpuid_inline_outputs` also fails the GCC comparison in this sandbox,
  but only in the initial-APIC-ID byte of EBX (which core ran the process)
  — a test flake, not a codegen issue; mask bits 31:24 in the test.

## To do — ordered by (expected gain × breadth × confidence) / cost

1. **Residual frame in inline-libc leaves.**  `void z(char*p){memset(p,0,15);}`
   is still `subq $24,%rsp … addq $24,%rsp` (5 vs GCC/Clang/ICX 3) and
   `memcpy(d,s,32)` with swapped params shows a dead `movq %rsi, 8(%rsp)`
   pre-store.  The parameter gets a stack slot from the prologue's
   `gpr_prestores` path although it has a register home, and the slot
   defeats `calculate_stack_space` frame elision.  Reproducers:
   `tests/regression/cpu_model_memset_inline.c` (`fz15`), `swap.c` shape in
   AUDIT §7.  Fix in `prologue.rs` (`param_pre_stored` decision) — do not
   touch the elision rule itself.
2. **Runtime fill-byte broadcast with AVX2.**  `memset(p,c,40)` spends
   `movzbl/movabs/imulq/vmovq/vpbroadcastq` (5) where GCC does
   `vmovd %esi,%xmm0; vpbroadcastb %xmm0,%ymm0` (2).  Use `vpbroadcastb`
   when `vex && size >= 16`; keep the `imul` form for the scalar tails and
   the SSE2 baseline (no SSSE3 `pshufb` guarantee).
3. **memset/memcpy-aware IR passes.**  No pass models `memset`: DSE leaves
   `movb $0, 8(%rsp)` behind the `rep stosb` in `zinit`, and any later
   load-forwarding across the call is blocked.  Teach `alias.rs`/`dse.rs`/
   `load_forward.rs` that `memset(p,c,n)` writes exactly `[p, p+n)` and
   reads nothing; then lower the `ZERO_INIT_STORE_LADDER_MAX` cut-over to
   64 B (Clang's behaviour once SROA understands the intrinsic).
4. **Partial-clobber call points.**  Liveness still treats the expansions
   as full caller-saved clobbers; only *parameters* are exempt.  A
   per-point clobber mask ({rdi,rsi,rcx,rax,xmm0,xmm1}) in
   `liveness::instruction_is_call_point` → `spans_any_call` would let
   temporaries stay in r8–r11/rdx across the expansion.
5. **Expansions outside the entry block.**  `x86_params_dead_after_inline_libc_calls`
   is conservative for loop bodies / conditional paths (falls back to
   callee-saved homes).  Needs dominance + back-edge reasoning.
6. **memcpy calls > 32 B.**  `inline_memcpy_len` still caps at 32 B for
   the *call* form while struct copies use the full strategy; unify on
   `memcpy_call_strategy` (LibCall above ¼ L3) and reuse
   `stage_copy_operands`.
7. **Return-value materialisation after in-place ALU** (was #1) —
   unchanged; see AUDIT §6.
8. **Index-only LEA** (was #2), **reduction accumulator splitting** (#3),
   **3-operand LEA fixup** (#4), **vectoriser VF hook** (#7), **`movbe`**
   (#8), **branch-probability if-conversion** (#9), **JCC erratum** (#10),
   **prefetch distance** (#11), **hardware validation** (#12) — unchanged,
   priorities as before.
9. **Non-temporal libcall bound** (was #6) — `memset` now honours
   `LibCall` (keeps the call above ¼ L3); memcpy struct copies still lack
   the libcall path.

## Previous to-do list (session `cpu-model-v3`, kept for reference)


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
