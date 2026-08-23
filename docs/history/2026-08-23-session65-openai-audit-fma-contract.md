# Session 65 (2026-08-23) — OpenAI Audit Response + FMA Contraction Default Fix

## Scope

This session responds to the OpenAI-generated audit of `ms178/lccc` (HEAD
`1dc38664…`), validates its claims against the actual codebase, fixes two
P0 issues the audit surfaced (one correctness, one code-quality), updates
`engineering/agent/BACKLOG.md` with a detailed verdict, and records the
follow-up work for future sessions.

## 1. Environment bootstrap (constrained VM)

- 2 vCPU, ~2 GB RAM, 8 GB swap created at `/swapfile` (mkswap+swapon).
- Host packages: `ninja-build cmake mold clang lld llvm-19 zstd pkg-config
  libssl-dev gcc-multilib libc6-dev-i386`.
- Rust/Cargo 1.98.0 installed via rustup into `/home/user/.cargo` (the repo
  pins 1.98.0 in `rust-toolchain.toml`).
- Built with `scripts/build_lccc_fast.sh` (fastbuild profile, opt-level 1,
  no LTO, incremental, codegen-units 256) per project policy for fast REACT
  cycles. Build time ~1m45s cold.
- Binary landed at `target/fastbuild/lccc` (58 MB).

## 2. Audit verdict (see BACKLOG §12 for the long form)

The OpenAI report was ~85% accurate. Specifically:

- **RA fat-interval scan, no reload-at-next-use, FP GPR bounce, pattern-only
  vectorizer, missing µarch scheduling, weak perf-CI** — all real issues,
  all already tracked (RA-05, RA-06, IS-29/AB-01, OP-05, IS-21, MS-01).
- **FMA "does not exist in the backend"** — disproved; `vfmadd231sd/ss`
  and packed `vfmadd{132,213,231}{ps,pd}` are emitted through
  `emit_scalar_fma231{,_with}` in `float_ops.rs` and through the vector
  reduction / matmul paths. The audit looked only at default `-O2` where
  contraction was incorrectly on.
- **FP contraction default** — the report did not evaluate whether
  default-on FMA is GCC-compliant. Measured against GCC 14.2.0 (Debian
  14.2.0-19) on this host: `gcc -O2 -march=x86-64-v3` does **not** contract
  loop-carried reductions into FMA; it keeps `vmulsd`+`vaddsd` separate.
  Only `-ffp-contract=fast` / `-ffast-math` produces FMA in loops.
  LCCC's previous default (`fp_contract_fast=true` at plain `-O2`) was a
  conformance bug and caused the large numeric divergence on `nbody`
  (LCCC 59.05 vs GCC -0.17) because it fused across SSA temporaries that
  represented separate C statements (different rounding, changed
  numerical stability of the N-body force accumulation).
- **Wholesale SGSA rewrite** — directionally correct but the report
  understates how much of the "greedy allocator" machinery already
  exists (segments, Tier-2 graph coloring, hot-loop homes, ABI hints,
  destructive-FMA coalescing, split_ranges). A clean-slate rewrite would
  be 4–8 weeks of high-risk work; incremental migration (RA-05 → RA-06)
  is strictly better.

## 3. Fixes landed

### 3.1 `src/backend/x86/codegen/float_ops.rs` — scalar FMA operand staging

`load_fp_to_reg` previously hard-coded the GPR-staging path to go through
`%rax` → `%xmm1` with a `movaps %name, %xmm` regardless of the caller's
chosen destination register. That is wrong when the caller asks to load
into `%xmm1` (the FMA emitter does exactly this for the multiply LHS):
we would emit `movq %rax, %xmm1` and then `movaps %xmm1, %xmm1`, clobbering
the accumulator that was already loaded into `%xmm1`, and using `movaps`
for scalar FP moves risks domain-transition stalls and false dependencies
on the upper lanes.

Replaced with:
- GPR selection sensitive to the target XMM (`%rax` for `xmm0`, `%rcx` for
  every other XMM — matches existing convention elsewhere in the backend).
- Type-appropriate move: `movsd` for F64, `movss` for F32, `movq`/`movd`
  for GPR→XMM moves (never `movaps` for scalar operands).
- Operand-type awareness in the register-home fast path (the previous
  code used `movaps` unconditionally).

### 3.2 `src/driver/pipeline.rs` + `src/driver/cli.rs` — default FP contraction

- `fp_contract_fast` now defaults to `false` at plain `-O2`, matching GCC
  14.2's observable behavior (no cross-statement FMA without an explicit
  flag).
- `-ffast-math` still turns FMA contraction on (GCC parity).
- `-fno-fast-math` now correctly resets to default (off) instead of
  silently re-enabling contraction; previously it set the flag to `true`
  unconditionally, which was wrong.
- `-ffp-contract=off` / `-ffp-contract=on` / `-ffp-contract=fast` are all
  honored. `-ffp-contract=on` currently behaves identically to `off`
  because LCCC does not yet carry source-expression boundary metadata on
  IR values; OP-36 (expression ids) will allow a sound `on` mode that
  contracts within a single C expression like GCC does.
- Updated the CLI unit test `fp_contract_is_independent_and_last_option_wins`
  to reflect the new default.

### 3.3 `tests/regression/check_fma_dest_coalesce_codegen.sh`

This shell test asserts structural properties of the FMA output (that
destructive FMA coalescing fires and avoids the scratch-path). It was
relying on FMA being enabled at default `-O2`; with the default flipped
it now explicitly passes `-ffp-contract=fast` so the test continues to
measure what it was designed to measure.

## 4. Validation

- `cargo build --profile fastbuild` → clean.
- `cargo test --lib --profile fastbuild` → 1101 passed, 0 failed, 6 ignored.
- Hand-assembled probes (`/tmp/fma_stmt.c`): LCCC default `-O2` now
  produces `vmulsd` + `vaddsd` for the reduction loop, matching GCC 14.2.
  `-ffp-contract=fast` produces `vfmadd231sd`, also matching GCC.
- FMA shell-regression checks (all pass):
  - `check_fma_dest_coalesce_codegen.sh`
  - `check_vector_dot_fma_codegen.sh`
  - `check_multiple_fp_reductions_codegen.sh`
  - `check_f128_copy_fusion.sh`
  - `check_ptr_to_float_cast.sh`
  - `check_load_widen_cast_no_relay.sh`
  - `check_integer_reduction_vecreg_codegen.sh`
  - `check_reduction_vecreg_codegen.sh`
- Full `tests/regression/run_regression.py` sweep: **434 passed, 18 failed,
  9 skipped-compare**. The 18 failures (arm_matmul_column_stride,
  arm_vla_struct_varargs_byref, float_direct_slot, fma_chain_accum,
  fma_zero_multiplier_semantics, fp_param_regalloc, fp_param_wide,
  loop_promote_affine_alias, memcpy_avx_sse_transition, movb_imm_rex8,
  nbody, param_home_scratch_reg, regparm3_abi_conformance,
  session45_vector_recurrence_stress, simd_sse_float,
  spectral_like_reduction, vectorize_sum_f32, vectorize_trip_count_gate)
  are all pre-existing on `1dc3866…` and were not introduced by this
  session's changes; most are ARM/i686-specific or depend on oracle
  compilers this host does not have.

## 5. New backlog items

Added to `engineering/agent/BACKLOG.md §12.3`:

- **RA-05a (P0)** — drive the linear scan from `liveness.segments` as the
  primary interference representation (xmltok/inflate gating metric).
- **RA-06a (P0)** — call-site splitting + reload-at-next-use on top of
  `split_ranges.rs`.
- **IS-29a (P0)** — XMM-home for `__builtin_fma[f]` / `vroundsd` / other FP
  intrinsic results; kills the libm round-family 2.99× regression.
- **OP-05a (P0)** — generic non-reduction FP vectorizer (nbody/spectral).
- **OP-36 (P1)** — expression-boundary metadata so `-ffp-contract=on` can
  match GCC's behavior.
- **MS-01a (P0)** — codegen oracle CI gate for the six golden workloads.
- **MS-13 (P1)** — replace the `fp_contract_fast` bool with a tri-state
  `FpContract { Off, OnStmts, Fast }`.
- **IS-30 (P1)** — turn text-peephole into a diagnostic for missing RA/
  ISel folds.
- **FE-25 (P1)** — real `-march=native` for Raptor Lake.
- **PERF-3 (P2)** — TLS base CSE + `%fs:sym@tpoff(,%r,scale)` addressing.
- **MS-14 (P2)** — PMU harness on the 14700KF once hardware CI exists.

## 6. Follow-up work for the next AI agent

1. Start with **RA-05a**. It is the highest-leverage change: making
   `collect_gpr_scan_intervals`/`collect_fp_scan_intervals` iterate
   `liveness.segments` instead of `liveness.intervals` and using segment
   (not interval) overlap for interference. Add an env kill switch
   (`CCC_NO_SEGMENT_SCAN=1`) and A/B against gzip longest_match, xmltok,
   inflate, and expat. Acceptance: stack-mem refs on xmltok ≤6× baseline
   GCC (target <4×) and gzip longest_match stack-mem does not rise.
2. Then **RA-06a** — extend `split_ranges.rs` with a call-split pass: for
   any live range that crosses a call and is not already in a callee-saved
   register, insert a store before the call + reload after. Measure
   against the adler32 DO8 kernel (the 1.49× gap from the scoreboard).
3. **IS-29a** — small, self-contained, high-confidence. When lowering
   `IntrinsicOp::FmaScalarF32/F64`, request an XMM home for the dest value
   the same way reduction accumulators get XMM homes, and remove the
   `movq %xmm0, %rax` / `movq %rax, %xmm0` bounce in `store_xmm0_fp_dest`
   when the destination is already XMM-homed.
4. **Do NOT** attempt a clean-slate SGSA rewrite. The incremental path is
   strictly safer and the Tier-2 graph colorer is already in place as a
   second-chance allocator.
5. When running benchmarks in this VM: no PMU access, so use
   `perf stat -e instructions,cycles,cache-misses,branches,branch-misses`
   only where it works (cycles will be noisy under virtualization;
   instruction counts and stack-ref counts from `--save-temps` asm are
   the reliable signal). Pin builds to `-j2` and always
   `scripts/ensure_swap.sh` first; cargo peaks at ~1.7 GB RSS during THIN
   LTO links.
6. Before ending any turn, run:
   - `cargo build --profile fastbuild` (must be clean).
   - `cargo test --lib --profile fastbuild` (must be 1101+ passing).
   - The eight FMA/FP/reduction shell scripts listed in §4.
   - Save a snapshot with `scripts/lccc-snapshot.sh <slug> "<desc>"`.
   - Append a section like this one to
     `docs/history/<date>-session<N>-<topic>.md` so future agents have
     the provenance chain.
