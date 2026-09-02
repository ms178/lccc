# FOLLOWUP 2026-09-02c — MachInst typed scalar-float layer (FMov/FAlu) + vectorizer-orphan ICE fix

Base: `10386f4` (origin/main). Session snapshots: **S07-typed-float-layer**, **S08-vec-dce-ice-fix**, S09 (this doc).
Deliverable: `/home/user/ms178-1.patch` (APPLIES-CLEAN, verified in fresh worktree at base).

## 1. What was built

### 1.1 Typed scalar-float MachInst layer (the structural gap)

Before: `MachReg` had no XMM representation; **every** float instruction fell back to
untyped text emission (`is_mi_unsafe_value` rejected any xmm-domain operand). The
typed layer was integer-only — nbody's MI-STREAM contained zero xmm registers.

Now (S07):
- **`MachInst::FMov`** — scalar SSE moves, S32→`movss` / S64→`movsd`. Reg↔reg emits
  the **VEX 3-operand form** (`vmovsd %s,%s,%d`), matching the text path's
  merging-move false-dependence fix (nbody 3.1× story). Reg↔mem/slot legacy forms.
- **`MachInst::FAlu`** — `v{add,sub,mul,div}s{s,d} src2, %src1, %dst` (VEX
  three-operand: no destructive dest, so dest-aliases-operand needs no staging).
  Sub/Div keep lhs in src1 (operand order pinned by tests); commutative swap when
  only the rhs is xmm-homed (mirrors the text path's swap exactly).
- **Emitters**: xmm8–15 (`PhysReg 26..=33`) added to `reg_name`; strict `freg_name`
  (no silent fallback — house rule).
- **Admission** (double-gated, isel `lower_instruction_typed_inner` +
  emit-side `float_typed_candidate` must agree):
  - Store/Load F32/F64/D32/D64: value xmm-homed (xmm2..xmm15), address = direct
    alloca slot or GPR-held pointer; rejects folded GEP/global addrs (the never-
    materialized-pointer segfault class), OverAligned slots, xmm-based pointers,
    constants, F128/x87, accumulator/i128/vector values.
  - Copy: both sides xmm-homed.
  - BinOp Add/Sub/Mul/SDiv F32/F64: dest xmm-homed; lhs xmm + rhs xmm/slot(vreg →
    flush-time StackSlot resolution), or commutative rhs-xmm/lhs-slot swap.
- **Kill switches**: `CCC_MI_DISABLE_KINDS=float-mov` / `float-alu`
  (`machinst_disabled_kinds` widened u16→u32).
- Slot-homed rhs in FAlu travels as `MachReg::Vreg` and resolves via
  `resolve_stack_vregs`; anything unresolvable trips the existing replay fallback.

### 1.2 Pre-existing ICE found + fixed via godbolt oracles (S08)

`for (i) gd[i] = gd[i]*k + c;` over a **global** array ICEd at -O2 and -O3:

```
x86 codegen: value 36 has no register, stack slot, Copy, or GlobalAddr definition
```

Root cause chain (all verified by instrumentation, then removed):
1. Vectorizer hoists `VecBroadcastF64x4` setup into the preheader, then **bails**
   out of the loop (twice — two orphaned broadcasts), leaving the loop scalar.
2. `IntrinsicOp::is_pure()` **missed the entire modern `Vec*` family** (Broadcast/
   Zero/Load/Add/Mul/Fma/Madd/Sub/Div/Sqrt): the `VecBroadcastF64x4` token at
   intrinsics.rs:1300 lives in `produces_vector_value()`, not `is_pure()` (ends
   :1265). So DCE rooted the dead broadcasts as side-effecting.
3. No DCE ran after the vectorizer anyway.
4. Codegen: `avx_store_dest` on a homeless vector value falls through to the
   pointer interpretation → panic.

Fix: (a) `is_pure()` now delegates to `produces_vector_value()` **minus**
`VecStoreI64x2` (the one memory-writing op that list contains for slot-sizing
reasons — left untouched because regalloc + vector_temp_promotion special-case
it); (b) one `dce::eliminate_dead_code` sweep immediately after the
vectorize+post-vec-unroll block in `passes/mod.rs`; (c) regression test
`vectorize_global_array_bailout_ice.c`; (d) two DCE unit tests (pure unused
broadcast removed; `VecStoreF64x4` never removed by purity).

## 2. Evidence

| Check | Result |
|---|---|
| unit `cargo test --lib` | **1535/0/6** (was 1517: +11 machinst FMov/FAlu, +5 FAlu isel, +2 DCE) |
| regression suite | **569/0/15** (567 + 2 new tests) |
| benchmark outputs | **152/0/0** |
| `ir_verify_sweep.py` | **569×2 = 1138 configs, 0 violations** |
| census `CCC_ISEL_STATS` | **88.05% → 88.99%** (7951→8739 lowered; +788). Note ~284 float moves were previously *invisible* rejections (rejected before stats), so real gain ≈ +3.6pp |
| assembler differential | FMov/FAlu corpus through real GAS (2.44) — accepted |
| forced-loop stress | nbody/mandelbrot/spectral_norm/fannkuch output **bit-identical to gcc**; spectral_norm ON/OFF binaries byte-identical (md5) |
| codegen diff ON/OFF (all 38 benchmarks, forced) | only nbody+mandelbrot differ: 2+1 copy lines `movapd`→`vmovsd` (1-uop, no merge dependence) — layer is otherwise text-neutral |
| godbolt `dot8` | lccc 46 vs gcc 21 / clang 14 / icc 24 / icx 10 — see §4 gaps |

A/B timing on this 2-core sandbox is ±40% noise (spectral_norm "15% regression"
was measured on **byte-identical binaries**); codegen-diff equality is the
stronger evidence and it shows neutral-or-better.

## 3. Known traps re-verified

- Never emit legacy `movsd %xmmA,%xmmB` reg-reg (merging move → false
  dependence). VEX 3-operand only.
- Folded GEP/global addresses have no materialized register — any new
  Load/Store admission MUST reject `folded_gep_values` / `folded_global_addrs`
  (this bit the first FMov iteration: segfault reading an unwritten GPR).
- `value_to_reg` maps XMM homes to `Vreg` (integer-era assumption) — float
  lowerings must construct xmm operands explicitly.
- `CCC_MI_FORCE_LOOPS=1` full-suite has 2 PRE-EXISTING integer failures
  (`ra09_selfop_xor`, `range_check_fold` — byte-identical codegen with the
  float layer on/off). Do not attribute them to the float layer; the 32-inst
  loop profitability gate exists for this reason.

## 4. Follow-up backlog (data-driven, ranked)

1. **Call typed lowering (522 rejections, 48% of the rest)** — biggest single
   class; every Call flushes the run. Plan: typed arg moves + `MachInst::Call`
   + return home + caller-saved spills as typed stores (interval data already
   exists in `caller_save_intervals`). Subset first: ≤6 int/ptr args, no
   variadic/struct/i128, named symbol.
2. **Vectorizer map-loop bailout over global arrays** (§1.2's trigger): `scale_add`
   stays scalar where gcc/clang emit 4-wide packed loops. Find the legality check
   that requires alloca/param bases.
3. **dot8 gap (46 vs 10–24)**: (a) no full unroll of known-2-trip loops at -O3;
   (b) no FMA (`vfmadd*`) despite `-march=x86-64-v3` — `vmulsd+vaddsd` pairs
   survive; check FMA3 gating + reduction-accumulator fusion.
4. **FCmp (Cmp(float) 11)** — text path stages via xmm0/xmm1 scratch; needs
   scratch-reg representation in MachInst (PhysReg ids 17–19 are free; 18/19 →
   xmm0/xmm1, never allocatable). Same unlock enables FMov mem↔mem relays and
   slot-homed float stores (the remaining 123 Store(float)).
5. **Memcpy (92) / Store(other) (79)** typed sequences.
6. Loop-gate re-evaluation: with FMov/FAlu text-neutral, raising
   `CCC_MI_MAX_LOOP_INSTS` only needs the 2 integer forced-loop failures fixed
   first.
7. mold/lld linker-oracle script (still missing from scripts/).

## 5. File map (this session)

- `machinst.rs`: +FMov, +FAluOp/+FAlu variants
- `machinst_emit.rs`: xmm8–15 names, `freg_name`, FMov/FAlu arms (VEX forms, loud traps)
- `isel.rs`: `is_xmm_phys`; float arms in Store/Load/Copy/BinOp
- `emit.rs`: `MI_FLOAT_MOV`/`MI_FLOAT_ALU` bits (u32 mask), `float_typed_candidate`,
  bypass in `try_lower_machinst`, FMov/FAlu in `resolve_stack_vregs` +
  `has_unresolvable_vreg`
- `machinst_tests.rs`: +16 tests (golden/rsp/self-move/5 traps/isel admission ×2/
  class-coverage floats/corpus)
- `ir/intrinsics.rs`: `is_pure` Vec-family fix
- `passes/dce.rs`: +2 tests; `passes/mod.rs`: post-vectorize DCE
- `tests/regression/`: `machinst_float_scalar_layer.c`,
  `vectorize_global_array_bailout_ice.c`
