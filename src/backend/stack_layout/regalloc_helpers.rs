//! Register allocation helpers and parameter alloca lookup.
//!
//! Shared utilities that eliminate duplicated regalloc setup boilerplate
//! across all four backends (x86-64, i686, AArch64, RISC-V 64).

use super::super::regalloc::{PhysReg, RaConfig};
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrFunction, Operand, Value};
use std::sync::Arc;

// ── Register allocation helpers ───────────────────────────────────────────

/// Run register allocation and merge ASM-clobbered callee-saved registers.
///
/// This shared helper eliminates duplicated regalloc setup boilerplate across
/// all four backends (x86-64, i686, AArch64, RISC-V 64).  Each backend supplies its callee-saved
/// register list and pre-collected ASM clobber list; this function handles the
/// common steps: filtering available registers, running the allocator, storing
/// results, merging clobbers into `used_callee_saved`, and building the
/// `reg_assigned` set.
///
/// Returns `(reg_assigned, cached_liveness)` for use by `calculate_stack_space_common`.
pub fn run_regalloc_and_merge_clobbers(
    func: &IrFunction,
    available_regs: Vec<PhysReg>,
    caller_saved_regs: Vec<PhysReg>,
    asm_clobbered_regs: &[PhysReg],
    reg_assignments: &mut FxHashMap<u32, PhysReg>,
    used_callee_saved: &mut Vec<PhysReg>,
    allow_inline_asm_regalloc: bool,
) -> (
    FxHashMap<u32, PhysReg>,
    Option<super::super::liveness::LivenessResult>,
    FxHashMap<u8, Vec<(u32, u32)>>,
    Vec<super::super::regalloc::AccumulatorAssignment>,
    FxHashSet<u32>,
) {
    run_regalloc_and_merge_clobbers_ex(
        func,
        available_regs,
        caller_saved_regs,
        asm_clobbered_regs,
        reg_assignments,
        used_callee_saved,
        allow_inline_asm_regalloc,
        None,
        Vec::new(),
        Vec::new(),
        crate::common::fx_hash::FxHashMap::default(),
        Vec::new(),
        &Arc::new(RaConfig::from_process_env()),
        &mut FxHashMap::default(),
    )
}

/// Extended variant taking the set of values codegen will never materialize
/// (folded global addresses) and the ABI argument registers present in the
/// caller-saved pool; see RegAllocConfig::never_materialized /
/// RegAllocConfig::call_arg_regs / RegAllocConfig::indirect_target_regs.
fn collect_abi_reg_hints(
    func: &IrFunction,
    available_regs: &[PhysReg],
    caller_saved_regs: &[PhysReg],
    ra_config: &RaConfig,
) -> FxHashMap<u32, PhysReg> {
    let mut hints = FxHashMap::default();
    if ra_config.no_abi_reg_hints {
        return hints;
    }
    if func.uses_sret
        || func.params.iter().any(|param| {
            param.struct_size.is_some()
                || param.ty.is_float()
                || param.ty.is_long_double()
                || matches!(param.ty, IrType::I128 | IrType::U128)
        })
    {
        return hints;
    }

    // Restrict this first implementation to signatures where parameter index
    // equals integer ABI-slot index. Mixed FP/aggregate signatures require the
    // full call-ABI classifier and deliberately receive no hint.
    let mapping: Vec<Option<PhysReg>> = if !crate::common::types::target_is_32bit()
        && available_regs.iter().any(|r| r.0 == 1)
        && caller_saved_regs.iter().any(|r| r.0 == 10)
    {
        // SysV AMD64: rcx is intentionally unavailable to the allocator.
        vec![
            Some(PhysReg(14)), // rdi
            Some(PhysReg(15)), // rsi
            Some(PhysReg(16)), // rdx, when the function's scratch audit admits it
            None,              // rcx
            Some(PhysReg(12)), // r8
            Some(PhysReg(13)), // r9
        ]
    } else if available_regs.iter().any(|r| r.0 == 19) && caller_saved_regs.iter().any(|r| r.0 == 4)
    {
        // AArch64's current caller pool opens x4..x8. x0..x3 remain scratch.
        vec![
            None,
            None,
            None,
            None,
            Some(PhysReg(4)),
            Some(PhysReg(5)),
            Some(PhysReg(6)),
            Some(PhysReg(7)),
        ]
    } else if available_regs.iter().any(|r| r.0 == 11)
        && caller_saved_regs.iter().any(|r| r.0 == 12)
    {
        // riscv64: the caller pool is a0–a7 (PhysReg 12..19) and integer
        // ABI slot i IS a_i — pinning the param home to its incoming
        // register makes the ParamRef a no-op for the common shape.
        vec![
            Some(PhysReg(12)),
            Some(PhysReg(13)),
            Some(PhysReg(14)),
            Some(PhysReg(15)),
            Some(PhysReg(16)),
            Some(PhysReg(17)),
            Some(PhysReg(18)),
            Some(PhysReg(19)),
        ]
    } else {
        return hints;
    };

    for block in &func.blocks {
        for inst in &block.instructions {
            let Instruction::ParamRef {
                dest, param_idx, ..
            } = inst
            else {
                continue;
            };
            let Some(Some(reg)) = mapping.get(*param_idx) else {
                continue;
            };
            if available_regs.contains(reg) || caller_saved_regs.contains(reg) {
                hints.insert(dest.0, *reg);
            }
        }
    }
    hints
}

/// SysV x86-64: prefer, for each value passed to direct calls in one
/// consistent GP argument slot `k`, that slot's register, so it is computed
/// where the call reads it instead of in a scratch register copied at the
/// call (`leaq (%rdi,%rsi),%r9; ...; movq %r9,%rdi`).
///
/// These are soft preferences (`RegAllocConfig::reg_prefs`), not hints: a
/// `reg_hints` entry is the ParamRef ordered-copy contract and makes its
/// range immune to eviction, exchange and hot-web stealing, which an
/// outgoing argument must not be (lz4's hash-table base took %r15 and
/// pushed the hot loop's end pointer to the stack). A preference only picks
/// among free registers; the allocator's interference, pool and staging
/// hazard (`arg_reg_hazard`) checks still apply. A value that already holds
/// a contract hint gets none: its entry copy is elided, and the call's
/// staging copy out of the incoming register costs the same one move
/// (`fill(d, n)`: `movq %rsi,%rdx; movl $7,%esi; jmp memset`, GCC's code).
/// Calls the backend expands inline (fixed-size memcpy/memset) stage
/// nothing and give no preference.
fn collect_outgoing_arg_prefs(
    func: &IrFunction,
    available_regs: &[PhysReg],
    caller_saved_regs: &[PhysReg],
    ra_config: &RaConfig,
    hints: &FxHashMap<u32, PhysReg>,
) -> FxHashMap<u32, PhysReg> {
    let mut prefs = FxHashMap::default();
    if ra_config.no_abi_reg_hints
        || crate::common::types::target_is_32bit()
        || !available_regs.iter().any(|r| r.0 == 1)
        || !caller_saved_regs.iter().any(|r| r.0 == 10)
    {
        return prefs;
    }
    // SysV AMD64 integer argument slots; rcx is unavailable to the allocator.
    let mapping = [
        Some(PhysReg(14)),
        Some(PhysReg(15)),
        Some(PhysReg(16)),
        None,
        Some(PhysReg(12)),
        Some(PhysReg(13)),
    ];
    // value -> Some(slot) when every direct-call use agrees, None on conflict.
    let mut slot_of: FxHashMap<u32, Option<usize>> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            let Instruction::Call { func: name, info } = inst else {
                continue;
            };
            if crate::backend::generation::inline_memcpy_len(name, info).is_some()
                || crate::backend::generation::x86_inline_memset_len(name, info).is_some()
            {
                continue;
            }
            let Some(slots) = super::super::regalloc::x86_64_call_int_slots(info) else {
                continue;
            };
            for (arg, slot) in info.args.iter().zip(slots) {
                let Operand::Value(v) = arg else {
                    continue;
                };
                let entry = slot_of.entry(v.0).or_insert(slot);
                if *entry != slot {
                    *entry = None;
                }
            }
        }
    }
    for (v, slot) in slot_of {
        if hints.contains_key(&v) {
            continue;
        }
        let Some(Some(reg)) = slot.and_then(|k| mapping.get(k)) else {
            continue;
        };
        if available_regs.contains(reg) || caller_saved_regs.contains(reg) {
            prefs.insert(v, *reg);
        }
    }
    prefs
}

/// Result tuple of [`run_regalloc_and_merge_clobbers_ex`]: final homes, the
/// liveness the allocator computed, caller-save spans, accumulator
/// assignments and the allocation-eligible value set.
pub type RegallocMergeResult = (
    FxHashMap<u32, PhysReg>,
    Option<super::super::liveness::LivenessResult>,
    FxHashMap<u8, Vec<(u32, u32)>>,
    Vec<super::super::regalloc::AccumulatorAssignment>,
    FxHashSet<u32>,
);

#[expect(clippy::too_many_arguments)]
pub fn run_regalloc_and_merge_clobbers_ex(
    func: &IrFunction,
    available_regs: Vec<PhysReg>,
    caller_saved_regs: Vec<PhysReg>,
    asm_clobbered_regs: &[PhysReg],
    reg_assignments: &mut FxHashMap<u32, PhysReg>,
    used_callee_saved: &mut Vec<PhysReg>,
    allow_inline_asm_regalloc: bool,
    never_materialized: Option<crate::common::fx_hash::FxHashSet<u32>>,
    call_arg_regs: Vec<PhysReg>,
    indirect_target_regs: Vec<PhysReg>,
    folded_index_uses: crate::common::fx_hash::FxHashMap<u32, Vec<u32>>,
    fold_base_regs: Vec<PhysReg>,
    ra_config: &Arc<RaConfig>,
    phi_chain_out: &mut FxHashMap<u32, u32>,
) -> RegallocMergeResult {
    run_regalloc_and_merge_clobbers_ipa(
        func,
        available_regs,
        caller_saved_regs,
        asm_clobbered_regs,
        reg_assignments,
        used_callee_saved,
        allow_inline_asm_regalloc,
        never_materialized,
        call_arg_regs,
        indirect_target_regs,
        folded_index_uses,
        fold_base_regs,
        ra_config,
        phi_chain_out,
        Vec::new(),
        &mut Vec::new(),
    )
}

/// Whether an intrinsic participates in the SIMD location pipeline. Central
/// result metadata covers every vector producer, including newly added load
/// forms. Horizontal reductions are the explicit exception: their result is
/// scalar, but their SIMD inputs still require the same location machinery.
fn has_vector_location_contract(op: &crate::ir::intrinsics::IntrinsicOp) -> bool {
    use crate::ir::intrinsics::IntrinsicOp as O;
    op.vector_result_width().is_some()
        || matches!(
            op,
            O::VecHorizontalAddF64x4
                | O::VecHorizontalAddF64x2
                | O::VecHorizontalAddI32x8
                | O::VecHorizontalAddI32x4
                | O::VecHorizontalAddI64x4
                | O::VecHorizontalAddI64x2
                | O::VecHorizontalAddF32x8
                | O::VecHorizontalAddF32x4
                | O::VecHorizontalMaxI32x4
                | O::VecHorizontalMaxI32x8
                | O::VecHorizontalMinI32x8
                | O::HorizontalAddF64x4
                | O::HorizontalAddF64x2
                | O::HorizontalAddI32x8
                | O::HorizontalAddI32x4
        )
}

/// [`run_regalloc_and_merge_clobbers_ex`] plus the x86-64 IPA-RA inputs:
/// `ipa_regs` (caller-saved registers every call of `func` preserves, see
/// `RegAllocConfig::ipa_regs`) and, on return, `ipa_used` (those the
/// allocator homed values in).
#[expect(clippy::too_many_arguments)]
pub fn run_regalloc_and_merge_clobbers_ipa(
    func: &IrFunction,
    available_regs: Vec<PhysReg>,
    caller_saved_regs: Vec<PhysReg>,
    asm_clobbered_regs: &[PhysReg],
    reg_assignments: &mut FxHashMap<u32, PhysReg>,
    used_callee_saved: &mut Vec<PhysReg>,
    allow_inline_asm_regalloc: bool,
    never_materialized: Option<crate::common::fx_hash::FxHashSet<u32>>,
    call_arg_regs: Vec<PhysReg>,
    indirect_target_regs: Vec<PhysReg>,
    folded_index_uses: crate::common::fx_hash::FxHashMap<u32, Vec<u32>>,
    fold_base_regs: Vec<PhysReg>,
    ra_config: &Arc<RaConfig>,
    phi_chain_out: &mut FxHashMap<u32, u32>,
    ipa_regs: Vec<PhysReg>,
    ipa_used: &mut Vec<PhysReg>,
) -> RegallocMergeResult {
    // `phi_chain_out` receives the allocator's blessed same-value classes
    // (see RegAllocResult::phi_chain); targets whose emitters are not
    // alias-aware pass an ignored scratch map and keep the pre-alias
    // freshness behavior bit-for-bit.
    // Detect x86-64 target by checking for x86 callee-saved PhysReg IDs (1-5).
    // On x86-64, provide XMM registers for F64 allocation.
    let has_scalar_fp = func.blocks.iter().any(|block| {
        block.instructions.iter().any(|inst| match inst {
            Instruction::BinOp { ty, .. }
            | Instruction::UnaryOp { ty, .. }
            | Instruction::Cmp { ty, .. }
            | Instruction::Load { ty, .. }
            | Instruction::Store { ty, .. } => ty.is_float() && *ty != IrType::F128,
            Instruction::Cast { from_ty, to_ty, .. } => {
                (from_ty.is_float() || to_ty.is_float())
                    && *from_ty != IrType::F128
                    && *to_ty != IrType::F128
            }
            _ => false,
        })
    });
    let has_memcpy_or_vector_intrinsic = func.blocks.iter().any(|block| {
        block.instructions.iter().any(|inst| {
            if matches!(inst, Instruction::Memcpy { .. }) {
                return true;
            }
            let Instruction::Intrinsic { op, .. } = inst else {
                return false;
            };
            has_vector_location_contract(op)
        })
    });
    // The mature accumulator backend has a complete stack path for scalar FP,
    // while its XMM location contract is not yet unified with aggregate memcpy
    // and intrinsic producers. For ordinary scalar-FP functions, stack-backed
    // values were observed to avoid XMM↔GPR relays and be faster in some
    // microbenchmarks, but the stack path has a correctness bug (nbody O2
    // miscompile: LCCC O2 outputs -0.321 vs GCC -0.169, while O0 and XMM O2 are
    // correct).  The XMM path is correct and must be the default until the
    // stack bug is root-caused.  Keep the environment escape hatch for
    // experiments: CCC_DISABLE_SCALAR_FP_XMM=1 forces stack path, and
    // CCC_ENABLE_SCALAR_FP_XMM=1 (legacy) also enables XMM.
    let disable_scalar_fp_xmm = has_scalar_fp
        && !has_memcpy_or_vector_intrinsic
        && ra_config.disable_scalar_fp_xmm
        && !ra_config.enable_scalar_fp_xmm;
    // x86-64 detection: callee-saved pool contains rbx=PhysReg(1). This MUST
    // exclude 32-bit targets: i686's pool is ebx/esi/edi/ebp = PhysReg(0..3),
    // where PhysReg(1) is %esi -- the old `any(r.0 == 1)` check matched it and
    // handed the i686 backend XMM PhysRegs (20+) it cannot emit. The slot
    // assigner then skipped stack slots for those F64 values, and every read
    // went to a nonexistent home: fptest.c returned garbage at O2 (exit 1 vs
    // GCC's 23.0). 64-bit gate fixes it (fptest exit 0).
    let xmm_regs = if available_regs.iter().any(|r| r.0 == 1)
        // PhysReg IDs are target-local: RISC-V also has s1=1. Require
        // x86-64's caller pool marker (%r10=10) before opening XMM homes.
        && caller_saved_regs.iter().any(|r| r.0 == 10)
        && !crate::common::types::target_is_32bit()
        && !ra_config.no_xmm_regalloc
        && !disable_scalar_fp_xmm
    {
        // x86-64: xmm2-xmm7 for F64 values. xmm2 is normally a safe stable
        // home (plain scalar FP codegen only touches xmm0/xmm1 as scratch),
        // but it is clobbered by a handful of intrinsic emitters
        // (pblendvb, 128-bit VNNI, F128 bit helpers) that use it as an
        // implicit scratch register. Exclude it from the pool only when the
        // function actually contains such an emitter; otherwise keep all six
        // registers (removing xmm2 unconditionally caused spills and a ~3x
        // slowdown in FP-struct loops such as nbody).
        let clobbers_xmm2 = func.blocks.iter().any(|block| {
            block.instructions.iter().any(|inst| {
                if let Instruction::Intrinsic { op, .. } = inst {
                    use crate::ir::intrinsics::IntrinsicOp as O;
                    // Scalar-sequential rather than one alternation
                    // because the last row is conditional: the SSE2
                    // lowering of the 4-lane i32 product stages through a
                    // third register, and it is only reachable when
                    // SSE4.1 (`pmulld`) is absent -- so an SSE4.1+ target
                    // keeps xmm2 in the pool. Every other row here has an
                    // unconditional emitter and is therefore unconditional
                    // too.
                    let sse2_i32_mul_lowering =
                        !crate::passes::vectorize::x86_sse41_available_pub();
                    matches!(
                        op,
                        O::Pblendvb128
                            | O::Dpbusd128
                            | O::Dpbusds128
                            | O::Dpwusd128
                            | O::Dpwusds128
                            | O::Dpbssd128
                            | O::Dpbssds128
                            | O::Dpbsud128
                            | O::Dpbsuds128
                            | O::Dpbuud128
                            | O::Dpbuuds128
                            | O::Dpwuud128
                            | O::Dpwuuds128
                            | O::Dpwssd128
                            | O::Dpwssds128
                            | O::F128Fabs
                            | O::F128Neg
                            | O::F128Copysign
                            | O::VecMulI64x2
                            // Legacy-SSE2 lowering of the hoisted 2-wide
                            // matmul group needs a third scratch (product,
                            // hoisted factor, unaligned C addend).
                            | O::FmaF64x2Hoisted
                            | O::MulAddF64x2Hoisted
                            | O::MulAddF64x4HoistedSIB
                            // SSE2 value-shaped 4-lane widening reductions:
                            // the dual sign-unpack (psrad/punpck pair) stages
                            // the low and high 64-bit halves through two
                            // scratch registers on top of the raw vector, so
                            // the third scratch is %xmm2 exactly like the
                            // VecMulI32x4 SSE2 lowering. Always reachable
                            // (SSE2-only op, no higher-ISA alternative), so
                            // unconditional.
                            | O::VecWidenAddValueI32x4ToI64x2
                            | O::VecWidenMaskedAddValueI32x4ToI64x2
                            // SSE2 masked equal-width step: %xmm2 stages a
                            // slot-homed guard splat beside the lane copy.
                            | O::VecMaskedAddValueI32x4
                    ) || (sse2_i32_mul_lowering && matches!(op, O::VecMulI32x4))
                } else {
                    false
                }
            })
        });
        if clobbers_xmm2 {
            // xmm2 clobbered: use xmm3-xmm15 (13 regs).
            (21..=33).map(PhysReg).collect::<Vec<_>>()
        } else {
            // xmm2-xmm15 (14 regs).
            (20..=33).map(PhysReg).collect::<Vec<_>>()
        }
    } else if available_regs.iter().any(|r| r.0 == 28) {
        // AArch64: v16-v23 are caller-saved SIMD/FP registers.  Keep their
        // allocator IDs disjoint from x0-x30; the ARM emitter maps 40..47 to
        // d16..d23.
        //
        // The d24..d31 promotion pool (allocator IDs 48..55) is assigned
        // 48+i for the first eight loop-promoted F64 values (regalloc.rs).
        // Reserve only that prefix; the unused tail joins the general scan
        // so nbody's three velocity accumulators do not idle d27-d31 while
        // d16-d23 overflow (levkropp 9f304050). CCC_NO_PROMOTED_FP_TAIL
        // restores the all-or-nothing reservation for A/B.
        let mut regs: Vec<PhysReg> = if ra_config.no_promoted_fp_tail {
            if func.loop_promoted_f64_values.is_empty() {
                (40..=55).map(PhysReg).collect()
            } else {
                (40..=47).map(PhysReg).collect()
            }
        } else {
            let n_promoted = func.loop_promoted_f64_values.len().min(8) as u8;
            let mut r: Vec<PhysReg> = (40..=47).map(PhysReg).collect();
            r.extend(((48 + n_promoted)..=55).map(PhysReg));
            r
        };
        // d8..d14 (allocator IDs 32..38) are the callee-saved FP registers.
        // They are appended last so the cheaper caller-saved pool fills first;
        // the prologue saves only the ones actually assigned. d15/v15 stays
        // reserved (scratch in the FMA intrinsic path). CCC_NO_FP_CALLEE_SAVED
        // disables for A/B.
        if !ra_config.no_fp_callee_saved {
            regs.extend((32..=38).map(PhysReg));
        }
        regs
    } else {
        Vec::new()
    };
    let reg_hints = collect_abi_reg_hints(func, &available_regs, &caller_saved_regs, ra_config);
    let reg_prefs = collect_outgoing_arg_prefs(
        func,
        &available_regs,
        &caller_saved_regs,
        ra_config,
        &reg_hints,
    );

    // ── Static-chain register reservation (GNU C nested functions) ───────
    //
    // `SetStaticChain` is lowered as a DIRECT write of the ABI chain register
    // (`%r10` on x86-64, `%ecx` on i686) immediately before the call, outside
    // the allocator's model: it has no IR dest, so nothing tells the scan that
    // the register dies there. Any value the scan homed in the chain register
    // and that is still live across the call is silently destroyed.
    //
    // gcc.c-torture/execute/920501-7.c hit exactly this at -O1..-Os. In the
    // recursive nested function `y`, the argument `a - 1` was homed in `%r10`:
    //
    //     movl 48(%rsp), %r10d
    //     subl $1, %r10d          # a-1 lives in %r10
    //     movq %r11, %r10         # SetStaticChain -- clobbers a-1
    //     movq %r11, %rdi         # arg0 <- (relayed through the now-equal r11)
    //     call x.y                # y(chain) instead of y(a-1): infinite
    //                             # recursion -> SIGSEGV at ANY depth
    //
    // Reserving the register for functions that actually make nested calls is
    // the right trade: the construct is rare, and per-point modelling of one
    // hard register would add interference surface to every ordinary function
    // for no benefit. Functions with only `GetStaticChain` (a nested callee
    // that never calls a sibling) are unaffected — the chain is read once at
    // entry into a normal home, and reserving there would cost a register for
    // nothing.
    let (available_regs, caller_saved_regs, call_arg_regs, indirect_target_regs) =
        reserve_static_chain_reg(
            func,
            available_regs,
            caller_saved_regs,
            call_arg_regs,
            indirect_target_regs,
        );

    let accumulator_policy = if caller_saved_regs.iter().any(|r| r.0 == 10)
        || (caller_saved_regs.is_empty() && available_regs.iter().any(|r| r.0 == 1))
        // riscv64 with the caller-saved a0–a7 pool open (s11 + a0 in the
        // combined pools is a unique signature): keep the same LhsFirst
        // policy the callee-only pool ran.
        || (available_regs.iter().any(|r| r.0 == 11) && caller_saved_regs.iter().any(|r| r.0 == 12))
    {
        super::super::regalloc::AccumulatorPolicy {
            operand_order: super::super::regalloc::AccumulatorOperandOrder::LhsFirst,
            return_consumes_accumulator: crate::common::types::target_is_32bit(),
        }
    } else {
        super::super::regalloc::AccumulatorPolicy {
            operand_order: super::super::regalloc::AccumulatorOperandOrder::AccumulatorCentric,
            return_consumes_accumulator: crate::common::types::target_is_32bit(),
        }
    };
    // A leaf's loop state belongs in the volatile pool first, as GCC, Clang
    // and ICX allocate it: every callee-saved home costs a save/restore pair
    // per call.  x86-64 (the backend that publishes an indirect-call target,
    // `%r10` = PhysReg(11)) has r11/r10/r8/r9/rdi/rsi; AArch64 x4-x8/x13/x14
    // (plus x9-x12/x15 in pure-ALU bodies); RISC-V a0-a7 whenever its
    // caller pool is open at all.  The pools hold values only where the
    // emitters' scratch use leaves them alone, which is a property of the
    // register, not of the value, so loop values are as safe there as the
    // temporaries already homed in them.  i686 (3 scratch registers,
    // %eax/%ecx/%edx hazards) keeps the Phase-1 hot-loop promotion.
    // Measured on AArch64 -O2 `sum`/`count_if`/`max` style leaves: the
    // x19-x22 pairs (4 stp/ldp + frame) disappear with the loop bodies
    // unchanged.
    let leaf_caller_saved_homes = !crate::common::types::target_is_32bit()
        && caller_saved_regs.len() >= 4
        && (indirect_target_regs.iter().any(|r| r.0 == 11)
            || matches!(crate::common::types::target_elf_machine(), 183 | 243));
    let (param_volatile_pool, param_arrival_regs) =
        if crate::common::types::target_elf_machine() == 183 {
            (
                aarch64_param_volatile_pool(func, &caller_saved_regs),
                aarch64_param_arrival_regs(func, &caller_saved_regs),
            )
        } else {
            (Vec::new(), Vec::new())
        };
    let mut config = super::super::regalloc::RegAllocConfig {
        ipa_regs,
        available_regs,
        accumulator_policy,
        caller_saved_regs,
        leaf_caller_saved_homes,
        param_volatile_pool,
        param_arrival_regs,
        call_arg_regs,
        indirect_target_regs,
        allow_inline_asm_regalloc,
        xmm_regs,
        never_materialized: never_materialized.unwrap_or_default(),
        folded_index_uses,
        fold_base_regs,
        fold_readmit: FxHashSet::default(),
        reg_hints,
        reg_prefs,
        ra_config: Arc::clone(ra_config),
    };
    // Debug: CCC_NO_REGALLOC forces pure slot-based codegen (A/B experiments).
    // CCC_NO_REGALLOC_FUNC=<name,...> does the same for the listed functions
    // only, so a whole-TU miscompile that disappears under CCC_NO_REGALLOC
    // (the preboot ZSTD decoder was one) can be bisected to a single
    // function without touching the source. Names are compared exactly.
    let no_regalloc = ra_config.no_regalloc
        || ra_config
            .no_regalloc_func
            .as_deref()
            .map(|v| v.split(',').any(|n| n.trim() == func.name))
            .unwrap_or(false);
    let alloc_result = if no_regalloc {
        super::super::regalloc::RegAllocResult {
            ipa_used: Vec::new(),
            assignments: Default::default(),
            accumulator_assignments:
                super::super::regalloc::analyze_accumulator_assignments_with_config(
                    func,
                    config.accumulator_policy,
                    ra_config,
                ),
            used_regs: Vec::new(),
            caller_save_spans: Default::default(),
            liveness: None,
            phi_chain: FxHashMap::default(),
            eligible: FxHashSet::default(),
            fold_deferred: FxHashMap::default(),
        }
    } else {
        super::super::regalloc::allocate_registers_with_fold_rounds(func, &mut config)
    };
    *reg_assignments = alloc_result.assignments;
    *ipa_used = alloc_result.ipa_used;
    *used_callee_saved = alloc_result.used_regs;
    let accumulator_assignments = alloc_result.accumulator_assignments;
    let caller_save_spans = alloc_result.caller_save_spans;
    let cached_liveness = alloc_result.liveness;
    let eligible = alloc_result.eligible;
    *phi_chain_out = alloc_result.phi_chain;

    // Merge inline-asm clobbered callee-saved registers into the save/restore
    // list (they need to be preserved per the ABI even though we don't allocate
    // values to them).
    for phys in asm_clobbered_regs {
        if !used_callee_saved.iter().any(|r| r.0 == phys.0) {
            used_callee_saved.push(*phys);
        }
    }
    used_callee_saved.sort_by_key(|r| r.0);

    let reg_assigned: FxHashMap<u32, PhysReg> = reg_assignments.clone();
    (
        reg_assigned,
        cached_liveness,
        caller_save_spans,
        accumulator_assignments,
        eligible,
    )
}

/// Filter a callee-saved register list by removing ASM-clobbered entries.
/// Returns the filtered list suitable for passing to `run_regalloc_and_merge_clobbers`.
pub fn filter_available_regs(callee_saved: &[PhysReg], asm_clobbered: &[PhysReg]) -> Vec<PhysReg> {
    let mut available = callee_saved.to_vec();
    if !asm_clobbered.is_empty() {
        let clobbered_set: FxHashSet<u8> = asm_clobbered.iter().map(|r| r.0).collect();
        available.retain(|r| !clobbered_set.contains(&r.0));
    }
    available
}

// ── Utility ───────────────────────────────────────────────────────────────

/// Per-function index from parameter ordinal to its authoritative home
/// `Alloca`.  Built once per function, queried in unconditional O(1).
///
/// `func.param_alloca_values[i]` names the SSA value that lowering chose as
/// the home of parameter `i`.  That home is only usable while the entry block
/// still contains an `Instruction::Alloca` defining it: mem2reg may have
/// promoted the parameter and removed the `Alloca`, and an unrelated local
/// must never be mistaken for a parameter home
/// (gcc.c-torture/execute/20180112-1.c).  The original lookup answered that
/// question with a full scan of the entry block for *every* parameter.  A
/// positional O(1) probe covered the common aligned layout but still fell
/// back to the scan whenever a home was removed, a prefix was inserted, or
/// inlining shifted the layout.  Since every backend queries every parameter
/// (often several times), that fallback was O(params x instructions) per
/// function -- quadratic for parameter-heavy code.
///
/// `ParamAllocaIndex` removes the fallback entirely: [`build`](Self::build)
/// walks the entry block exactly once, O(instructions + params), and
/// [`get`](Self::get) is a bounds check plus an array read.  Semantics are
/// exactly the original full scan: for parameter `i`, the FIRST `Alloca` in
/// the entry block whose `dest` equals `param_alloca_values[i]` supplies both
/// the home `Value` and its `IrType`; with no such `Alloca` (mem2reg, or no
/// entry block) the answer is `None`, as it is for an out-of-range ordinal.
/// Duplicated home ids and duplicated `dest`s are handled by construction, so
/// the index stays exact even for IR that violates SSA dest uniqueness.
///
/// Two build paths: `aligned_homes` handles the layout lowering emits with
/// ONE `Vec` allocation and no hash table, and `general_homes` handles every
/// other layout with that same `Vec` plus one hash table and one entry-block
/// pass.  (The fast path is "hash-table-free", not heap-free: the index must
/// own its `homes` array.)  Both are proven exact in their documentation and
/// by the exhaustive/randomized equivalence tests below.
///
/// When the aligned probe fails at instruction `k`, the general pass re-walks
/// that prefix.  This is bounded by `params` extra instruction examinations
/// (at 4096 params: <= 4096 examinations, i.e. microseconds against a
/// multi-second compile) and buys a much simpler, exhaustively provable
/// implementation than a fused single-pass design that would have to
/// retroactively seed the general state.  The complexity witness below pins
/// that bound so the redundancy can never grow into a second full scan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParamAllocaIndex {
    homes: Vec<Option<(Value, IrType)>>,
}

/// Portable complexity witness for [`ParamAllocaIndex::build_with_stats`].
///
/// Wall-clock ratios are not portable across CI machines, so this counts the
/// two resources the algorithm must bound: instruction *examinations* and
/// *heap allocations*.  A failed aligned probe examines a prefix that the
/// general pass then re-examines, so the former is an examination count, not
/// a count of distinct instructions.  The contracts are
/// `instructions_examined <= params + entry_instructions` (LINEAR) and
/// `allocations <= 3` (O(1) in `params`), both pinned by unit tests on
/// adversarial shifted layouts.  Each counter is one `usize` increment and
/// `build` instantiates them as ZST no-ops, so optimized builds fold the
/// accounting away completely.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ParamHomeStats {
    pub instructions_examined: usize,
    /// Heap allocations performed by the build.  This is the second bounded
    /// resource: the aligned path needs exactly one (`homes`), the general
    /// path exactly three (`homes`, one map, one flat adjacency vector).  The
    /// old general shape allocated one small `Vec` per distinct home id, i.e.
    /// up to `params` extra allocations; the flat chain makes the cost
    /// independent of `params` and the unit tests pin the exact count.
    pub allocations: usize,
}

/// Monomorphized instrumentation hook.  `build` instantiates it as the ZST
/// [`NoStats`], whose `note_instruction` is a no-op: after inlining, the
/// production build contains ZERO accounting instructions.  Only
/// `build_with_stats` instantiates the counting version; there is no global
/// mutable state, no lock, and no `cfg`-dependent behaviour to diverge.
pub trait ParamHomeCounter {
    fn note_instruction(&mut self);
    fn note_allocation(&mut self);
}

/// The production counter: a zero-sized no-op.
#[derive(Debug, Default)]
pub struct NoStats;

impl ParamHomeCounter for NoStats {
    #[inline(always)]
    fn note_instruction(&mut self) {}
    #[inline(always)]
    fn note_allocation(&mut self) {}
}

impl ParamHomeCounter for ParamHomeStats {
    #[inline(always)]
    fn note_instruction(&mut self) {
        self.instructions_examined += 1;
    }
    #[inline(always)]
    fn note_allocation(&mut self) {
        self.allocations += 1;
    }
}

impl ParamAllocaIndex {
    /// Build the index for `func`.
    ///
    /// The aligned layout (the one lowering emits) takes an O(params) path
    /// with exactly one allocation and no hash table.  Everything else takes
    /// one O(params + instructions) pass with exactly three allocations.
    /// There is no per-query fallback and no per-query scan in either case.
    pub fn build(func: &IrFunction) -> Self {
        Self {
            homes: build_homes(func, &mut NoStats),
        }
    }

    /// Same index, plus the exact number of entry-block instructions examined.
    /// `build` is the zero-overhead production entry point; this one exists so
    /// tests and tools can assert the complexity bound instead of timing a CI
    /// machine.  Both instantiate the SAME algorithm, so the measurement is
    /// of the shipped code path and not of a test-only copy.
    pub fn build_with_stats(func: &IrFunction) -> (Self, ParamHomeStats) {
        let mut stats = ParamHomeStats::default();
        let homes = build_homes(func, &mut stats);
        (Self { homes }, stats)
    }

    /// O(1) lookup of the home of `param_idx`.
    #[inline]
    pub fn get(&self, param_idx: usize) -> Option<(Value, IrType)> {
        self.homes.get(param_idx).copied().flatten()
    }

    /// Number of indexed parameter ordinals.
    #[inline]
    pub fn len(&self) -> usize {
        self.homes.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.homes.is_empty()
    }
}

fn build_homes<C: ParamHomeCounter>(
    func: &IrFunction,
    counter: &mut C,
) -> Vec<Option<(Value, IrType)>> {
    // One buffer is shared by BOTH paths.  Previously the aligned probe
    // allocated its own `Vec`, and a failed probe dropped it immediately:
    // every shifted-layout function paid for a dead allocation that the
    // allocation witness exposed.  Allocating here instead means the general
    // path reuses the same capacity (`clear` + `resize`, no reallocation).
    // This costs nothing on the hot aligned path and removes one heap object
    // from every shifted compilation, with no extra instruction pass.
    let n = func.param_alloca_values.len();
    let mut homes: Vec<Option<(Value, IrType)>> = Vec::with_capacity(n);
    if n > 0 {
        counter.note_allocation();
    }
    if aligned_homes(func, counter, &mut homes) {
        return homes;
    }
    general_homes(func, counter, homes)
}

/// The single-pass positional path, valid ONLY for the aligned layout.
///
/// Fills the caller's buffer (already reserved for `n` entries) and returns
/// `true` when the first `n` entry-block instructions are `Alloca`s whose
/// destinations equal `param_alloca_values[0..n]` in order, and those home
/// ids are strictly increasing.  On failure the buffer is left empty, with
/// its capacity intact, for `general_homes` to reuse.  No allocation and no
/// second pass is needed on either outcome.
///
/// ## Exactness proof (against the `reference_full_scan` test oracle)
///
/// The reference answer for ordinal `i` is the FIRST `Alloca` in the entry
/// block whose `dest == want[i]`.  This path answers with instruction `i`,
/// which is an `Alloca` with `dest == want[i]`.  It can disagree only if some
/// instruction `j < i` is also an `Alloca` with `dest == want[i]`.  For every
/// `j < n`, instruction `j` is an `Alloca` with `dest == want[j]` by the
/// checked condition; strict increase gives `want[j] < want[i]`, hence
/// `dest != want[i]`.  So no earlier match exists, and instruction `i` is
/// the authoritative first match.  Instructions at indices `>= n` cannot
/// precede it.  The `IrType` returned is likewise the authoritative one.
///
/// Strict increase is required because duplicate home ids would break the
/// `want[j] < want[i]` step; such IR (malformed, or two parameters sharing a
/// home) is routed to `general_homes`, which handles duplicates by
/// construction.  Requiring *strictly increasing* rather than merely
/// distinct costs nothing (lowering assigns ascending SSA ids) and avoids a
/// hash set, which would defeat the point of this path.
fn aligned_homes<C: ParamHomeCounter>(
    func: &IrFunction,
    counter: &mut C,
    homes: &mut Vec<Option<(Value, IrType)>>,
) -> bool {
    homes.clear();
    let wants = &func.param_alloca_values;
    let n = wants.len();
    if n == 0 {
        return true;
    }
    if wants.windows(2).any(|w| w[0].0 >= w[1].0) {
        return false;
    }
    let Some(block) = func.blocks.first() else {
        return false;
    };
    if block.instructions.len() < n {
        return false;
    }
    for (i, want) in wants.iter().enumerate() {
        counter.note_instruction();
        let Instruction::Alloca { dest, ty, .. } = &block.instructions[i] else {
            homes.clear();
            return false;
        };
        if *dest != *want {
            homes.clear();
            return false;
        }
        // `homes` was created with capacity >= n, so this push cannot
        // reallocate.  A failed probe leaves an empty buffer of exactly the
        // right capacity for `general_homes` to reuse.
        homes.push(Some((*dest, *ty)));
    }
    true
}

/// The general one-pass path, for any entry-block layout.
///
/// `dest -> head ordinal` is one hash map, and the ordinals sharing a `dest`
/// form a singly linked chain through one flat `next` vector.  The previous
/// shape mapped each home id to a small vector of ordinals: it allocated one
/// such vector per distinct home id (up to `params` allocations on an
/// adversarial function) and stored 24-byte vector headers in the table.
/// The chain stores one
/// `usize` per entry, needs exactly three allocations regardless of `params`,
/// and touches contiguous memory while walking the ordinals.
///
/// Order within a chain is reverse insertion order, which is immaterial: all
/// ordinals sharing a home id are filled from the SAME FIRST matching
/// `Alloca`, so every permutation yields the identical reference answer.
/// No ordinal is filled twice, so `unresolved` decrements at most `params`
/// times; `saturating_sub` additionally makes underflow impossible by
/// construction.
fn general_homes<C: ParamHomeCounter>(
    func: &IrFunction,
    counter: &mut C,
    mut homes: Vec<Option<(Value, IrType)>>,
) -> Vec<Option<(Value, IrType)>> {
    let wants = &func.param_alloca_values;
    let n = wants.len();
    if n == 0 {
        return homes;
    }
    // Reuse the aligned probe's buffer: capacity >= n, so `resize` cannot
    // allocate.  Only the map and the flat adjacency vector are new heap
    // objects, giving exactly three allocations independent of `params`.
    homes.clear();
    homes.resize(n, None);
    counter.note_allocation();
    let mut head: FxHashMap<Value, usize> =
        FxHashMap::with_capacity_and_hasher(n, Default::default());
    counter.note_allocation();
    let mut next: Vec<usize> = vec![usize::MAX; n];
    for (i, value) in wants.iter().enumerate() {
        match head.entry(*value) {
            std::collections::hash_map::Entry::Occupied(mut slot) => {
                next[i] = *slot.get();
                *slot.get_mut() = i;
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(i);
            }
        }
    }
    let mut unresolved = n;
    if let Some(block) = func.blocks.first() {
        'walk: for inst in &block.instructions {
            counter.note_instruction();
            let Instruction::Alloca { dest, ty, .. } = inst else {
                continue;
            };
            let Some(&first) = head.get(dest) else {
                continue;
            };
            let mut o = first;
            loop {
                if homes[o].is_none() {
                    homes[o] = Some((*dest, *ty));
                    debug_assert!(unresolved > 0);
                    unresolved = unresolved.saturating_sub(1);
                    if unresolved == 0 {
                        break 'walk;
                    }
                }
                if next[o] == usize::MAX {
                    break;
                }
                o = next[o];
            }
        }
    }
    homes
}

/// PhysReg id of the ABI static-chain register, or `None` when this target's
/// chain register is not in any allocatable pool.
///
/// | target  | chain register | allocator id |
/// |---------|----------------|--------------|
/// | x86-64  | `%r10`         | `PhysReg(11)` |
/// | i686    | `%ecx`         | `PhysReg(4)`  |
/// | AArch64 | `x18`          | not allocatable |
/// | RISC-V  | `t2`           | not allocatable |
///
/// AArch64 and RISC-V keep their chain register outside the allocator's
/// register file entirely, so they need no reservation.
pub fn static_chain_phys_reg() -> Option<PhysReg> {
    if crate::common::types::target_is_32bit() {
        // i686 `%ecx`. (The 32-bit ARM/RISC-V configurations do not reach the
        // x86 pools; the caller only removes ids that are actually present.)
        Some(PhysReg(4))
    } else {
        // x86-64 `%r10`.
        Some(PhysReg(11))
    }
}

/// Whether `func` performs a direct call to a nested function, i.e. contains a
/// `SetStaticChain` that writes the ABI chain register before a call.
///
/// `GetStaticChain` alone (a nested callee reading its own chain at entry) is
/// deliberately NOT a trigger: that is an ordinary def with a normal home.
pub fn func_sets_static_chain(func: &IrFunction) -> bool {
    func.blocks.iter().any(|b| {
        b.instructions
            .iter()
            .any(|i| matches!(i, Instruction::SetStaticChain { .. }))
    })
}

/// Remove the static-chain register from every allocatable pool when `func`
/// makes a direct nested-function call. See the call site for the miscompile
/// this prevents (gcc.c-torture/execute/920501-7.c).
fn reserve_static_chain_reg(
    func: &IrFunction,
    available: Vec<PhysReg>,
    caller_saved: Vec<PhysReg>,
    call_arg: Vec<PhysReg>,
    indirect_target: Vec<PhysReg>,
) -> (Vec<PhysReg>, Vec<PhysReg>, Vec<PhysReg>, Vec<PhysReg>) {
    if !func_sets_static_chain(func) {
        return (available, caller_saved, call_arg, indirect_target);
    }
    let Some(chain) = static_chain_phys_reg() else {
        return (available, caller_saved, call_arg, indirect_target);
    };
    let drop =
        |v: Vec<PhysReg>| -> Vec<PhysReg> { v.into_iter().filter(|r| r.0 != chain.0).collect() };
    (
        drop(available),
        drop(caller_saved),
        drop(call_arg),
        drop(indirect_target),
    )
}

/// Parameters whose vector value is read through ONE entry-prefix
/// `VecLoad` of the parameter alloca (the shape `vec_ssa::hoist_param_loads`
/// produces).
///
/// Index = parameter index; `Some((load_dest, bytes))` when the parameter
/// alloca is 16 or 32 bytes, is mentioned by exactly one instruction in the
/// whole function, that instruction is a pure full-width zero-offset
/// `VecLoad*` in the entry block, and nothing but `Alloca`/`ParamRef`/other
/// such loads precedes it.  At that program point the incoming argument
/// register is provably untouched (the prologue reads it, nothing in the
/// prefix writes an argument register), so the x86 prologue may move the
/// register straight into the load's home and the load emits nothing
/// (`CodegenState::vec_param_preloaded`).
pub fn x86_vector_param_loads(func: &IrFunction) -> Vec<Option<(Value, usize)>> {
    use crate::backend::stack_layout::copy_coalescing::is_pure_vec_load;
    let n = func.params.len();
    let mut out: Vec<Option<(Value, usize)>> = vec![None; n];
    if func.param_alloca_values.is_empty() || func.blocks.is_empty() {
        return out;
    }
    let mut size_of: crate::common::fx_hash::FxHashMap<u32, usize> =
        crate::common::fx_hash::FxHashMap::default();
    for inst in &func.blocks[0].instructions {
        if let Instruction::Alloca {
            dest,
            size,
            volatile: false,
            semantic_volatile: false,
            ..
        } = inst
        {
            if matches!(*size, 16 | 32) {
                size_of.insert(dest.0, *size);
            }
        }
    }
    // Mention census over the whole function.
    let mut mentions: crate::common::fx_hash::FxHashMap<u32, usize> =
        crate::common::fx_hash::FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            inst.for_each_used_value(|v| {
                if size_of.contains_key(&v) {
                    *mentions.entry(v).or_insert(0) += 1;
                }
            });
        }
        block.terminator.for_each_used_value(|v| {
            if size_of.contains_key(&v) {
                *mentions.entry(v).or_insert(0) += 1;
            }
        });
    }
    let alloca_to_param: crate::common::fx_hash::FxHashMap<u32, usize> = func
        .param_alloca_values
        .iter()
        .enumerate()
        .take(n)
        .map(|(i, v)| (v.0, i))
        .collect();
    for inst in &func.blocks[0].instructions {
        match inst {
            Instruction::Alloca { .. } | Instruction::ParamRef { .. } => continue,
            Instruction::Intrinsic {
                dest: Some(d),
                op,
                dest_ptr: None,
                args,
            } if is_pure_vec_load(op) && args.len() == 2 => {
                let Operand::Value(base) = args[0] else {
                    return out;
                };
                let zero_off = matches!(&args[1], Operand::Const(c) if c.to_i64() == Some(0));
                let Some(&bytes) = size_of.get(&base.0) else {
                    return out;
                };
                let Some(&pi) = alloca_to_param.get(&base.0) else {
                    return out;
                };
                if !zero_off
                    || op.vector_home_width() != Some(bytes as u32)
                    || mentions.get(&base.0).copied() != Some(1)
                {
                    return out;
                }
                out[pi] = Some((*d, bytes));
            }
            _ => return out,
        }
    }
    out
}

/// AArch64: the caller-saved registers a parameter web may be homed in
/// (`RegAllocConfig::param_volatile_pool`).
///
/// The ARM emitter copies each register parameter out of its argument
/// register at its `ParamRef` (`emit_param_ref_impl`), one parameter after
/// the other, so a home must never be a register some parameter still
/// arrives in: the copy into it would destroy that parameter before its own
/// `ParamRef` reads it. With `k` integer-class parameters those are
/// x0..x{k-1} (AAPCS64 C.9: one register each, in order), plus x8 when the
/// function receives a hidden result pointer. Only scalar parameter lists
/// are modelled -- an aggregate, 128-bit or `_Float128` parameter takes a
/// register pair, an alignment gap or a by-reference copy, and such a
/// function keeps the callee-saved-only rule (empty pool). Variadic
/// functions have no caller-saved pool at all.
///
/// The ParamRefs must also form the entry prefix (block 0, preceded only by
/// allocas and other ParamRefs): a value born before a ParamRef could take
/// one of these registers and the copy into it would run first.
pub(crate) fn aarch64_param_volatile_pool(
    func: &IrFunction,
    caller_saved_regs: &[PhysReg],
) -> Vec<PhysReg> {
    let Some(arrival) = aarch64_scalar_param_arrivals(func, caller_saved_regs) else {
        return Vec::new();
    };
    let int_params = arrival.iter().flatten().count();
    let arrives_in = |r: u8| usize::from(r) < int_params || (r == 8 && func.uses_sret);
    caller_saved_regs
        .iter()
        .copied()
        .filter(|r| !arrives_in(r.0))
        .collect()
}

/// AArch64: the parameters whose argument register is itself a pool
/// register (x4-x7), keyed by their `ParamRef` destination
/// (`RegAllocConfig::param_arrival_regs`); the conditions are those of
/// [`aarch64_param_volatile_pool`], empty otherwise.
///
/// Such a parameter can stay where it arrives: its `ParamRef` then emits at
/// most the in-place extension of a narrow argument (dropped by the peephole
/// where the upper bits are unobservable), where a home from the volatile
/// pool costs a copy (and the argument register, never a pool home for
/// anyone else, idles). No other
/// `ParamRef` writes the register (every home is either a volatile-pool
/// register, which no parameter arrives in, or the parameter's own argument
/// register), and nothing else is born before the last `ParamRef`.
pub(crate) fn aarch64_param_arrival_regs(
    func: &IrFunction,
    caller_saved_regs: &[PhysReg],
) -> Vec<(u32, PhysReg)> {
    let Some(arrival) = aarch64_scalar_param_arrivals(func, caller_saved_regs) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for inst in func.blocks.first().map_or(&[][..], |b| &b.instructions[..]) {
        if let Instruction::ParamRef {
            dest, param_idx, ..
        } = inst
        {
            if let Some(Some(r)) = arrival.get(*param_idx) {
                let reg = PhysReg(*r);
                if caller_saved_regs.contains(&reg) {
                    out.push((dest.0, reg));
                }
            }
        }
    }
    out
}

/// Each parameter's AArch64 integer argument register (`Some(n)` for xn,
/// `None` for an FP-class or stack-passed one) when the function qualifies
/// for parameter homes in the caller-saved pool: not variadic, a non-empty
/// pool, every parameter a scalar integer, pointer, `float` or `double`
/// (AAPCS64 C.9/C.1: the j-th integer-class parameter takes x{j} while
/// j < 8), and the ParamRefs forming the entry prefix of block 0.
fn aarch64_scalar_param_arrivals(
    func: &IrFunction,
    caller_saved_regs: &[PhysReg],
) -> Option<Vec<Option<u8>>> {
    if func.is_variadic || caller_saved_regs.is_empty() {
        return None;
    }
    let mut prefix = true;
    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            match inst {
                Instruction::ParamRef { .. } if bi == 0 && prefix => {}
                Instruction::ParamRef { .. } => return None,
                Instruction::Alloca { .. } => {}
                _ => prefix = false,
            }
        }
        prefix = false;
    }
    let mut next_gpr = 0u8;
    let mut arrival = Vec::with_capacity(func.params.len());
    for p in &func.params {
        if p.struct_size.is_some() || p.is_f128_sse || p.fp_agg_class.is_some() {
            return None;
        }
        match p.ty {
            IrType::I8
            | IrType::U8
            | IrType::I16
            | IrType::U16
            | IrType::I32
            | IrType::U32
            | IrType::I64
            | IrType::U64
            | IrType::Ptr => {
                arrival.push((next_gpr < 8).then_some(next_gpr));
                next_gpr = next_gpr.saturating_add(1);
            }
            IrType::F32 | IrType::F64 => arrival.push(None),
            _ => return None,
        }
    }
    Some(arrival)
}

#[cfg(test)]
mod static_chain_reservation_tests {
    use super::*;
    use crate::ir::instruction::{BasicBlock, Operand, Terminator};
    use crate::ir::reexports::{BlockId, Value};

    fn func_with(instructions: Vec<Instruction>) -> IrFunction {
        let mut f = IrFunction::new("t".to_string(), IrType::Void, Vec::new(), false);
        f.blocks = vec![BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        }];
        f
    }

    #[test]
    fn set_static_chain_is_detected_get_alone_is_not() {
        assert!(!func_sets_static_chain(&func_with(vec![])));
        assert!(!func_sets_static_chain(&func_with(vec![
            Instruction::GetStaticChain { dest: Value(1) }
        ])));
        assert!(func_sets_static_chain(&func_with(vec![
            Instruction::SetStaticChain {
                src: Operand::Value(Value(1))
            }
        ])));
    }

    #[test]
    fn chain_register_is_removed_from_every_pool() {
        let chain = static_chain_phys_reg().expect("x86 targets publish a chain reg");
        let pool = vec![PhysReg(1), chain, PhysReg(12)];
        let f = func_with(vec![Instruction::SetStaticChain {
            src: Operand::Value(Value(1)),
        }]);
        let (a, c, ca, it) =
            reserve_static_chain_reg(&f, pool.clone(), pool.clone(), pool.clone(), pool.clone());
        for (name, v) in [
            ("available", &a),
            ("caller_saved", &c),
            ("call_arg", &ca),
            ("indirect_target", &it),
        ] {
            assert!(
                !v.iter().any(|r| r.0 == chain.0),
                "{name} pool must not keep the static-chain register"
            );
            assert_eq!(v.len(), 2, "{name} pool must keep every other register");
        }
    }

    #[test]
    fn pools_are_untouched_without_a_nested_call() {
        let chain = static_chain_phys_reg().expect("x86 targets publish a chain reg");
        let pool = vec![PhysReg(1), chain, PhysReg(12)];
        let f = func_with(vec![Instruction::GetStaticChain { dest: Value(1) }]);
        let (a, c, ca, it) =
            reserve_static_chain_reg(&f, pool.clone(), pool.clone(), pool.clone(), pool.clone());
        assert_eq!(a, pool);
        assert_eq!(c, pool);
        assert_eq!(ca, pool);
        assert_eq!(it, pool);
    }
}

#[cfg(test)]
mod vector_location_contract_tests {
    use super::has_vector_location_contract;
    use crate::ir::intrinsics::IntrinsicOp as O;

    /// Preserve every member of the former hand-maintained list while the
    /// production predicate moves to central result metadata. The three
    /// strided forms are the newly covered regression cases; stores and a
    /// genuine scalar operation are negative controls.
    #[test]
    fn central_classifier_preserves_old_set_and_covers_strided_loads() {
        let former_and_strided = [
            O::VecZeroF64x4,
            O::VecZeroF64x2,
            O::VecZeroI32x8,
            O::VecZeroI32x4,
            O::VecZeroF32x8,
            O::VecZeroF32x4,
            O::VecLoadF64x4,
            O::VecLoadF64x2,
            O::VecLoadI32x8,
            O::VecLoadI32x4,
            O::VecLoadStrided2I32x8,
            O::VecLoadStrided2I32x4,
            O::VecLoadStrided2CompactI32x4,
            O::VecLoadI8x32,
            O::VecLoadI16x16,
            O::VecLoadF32x8,
            O::VecLoadF32x4,
            O::VecAddF64x4,
            O::VecAddF64x2,
            O::VecAddI32x8,
            O::VecAddI32x4,
            O::VecAddI8x32,
            O::VecMulI32x8,
            O::VecMulI32x4,
            O::VecZeroI64x2,
            O::VecLoadI64x2,
            O::VecLoadI32x4Pair,
            O::VecPackI32x4Pair,
            O::VecAddI64x2,
            O::VecMulI64x2,
            O::VecAddF32x8,
            O::VecAddF32x4,
            O::VecMulF64x4,
            O::VecMulF64x2,
            O::VecMulF32x8,
            O::VecMulF32x4,
            O::VecFmaF64x4,
            O::VecFmaF32x8,
            O::VecHorizontalAddF64x4,
            O::VecHorizontalAddF64x2,
            O::VecHorizontalAddI32x8,
            O::VecHorizontalAddI32x4,
            O::VecHorizontalAddF32x8,
            O::VecHorizontalAddF32x4,
            O::LoadF64x4,
            O::LoadF64x2,
            O::LoadI32x8,
            O::LoadI32x4,
            O::AddF64x4,
            O::AddF64x2,
            O::MulF64x4,
            O::MulF64x2,
            O::AddI32x8,
            O::AddI32x4,
            O::HorizontalAddF64x4,
            O::HorizontalAddF64x2,
            O::HorizontalAddI32x8,
            O::HorizontalAddI32x4,
        ];
        for op in former_and_strided {
            assert!(has_vector_location_contract(&op), "lost {op:?}");
        }
        // Central metadata intentionally also covers SIMD producers that the
        // old incomplete list omitted. This only strengthens the guard around
        // the explicitly enabled scalar-FP stack experiment; the default RA
        // configuration does not consult that experiment's opt-in flag.
        assert!(has_vector_location_contract(&O::VecSubI32x4));
        assert!(!has_vector_location_contract(&O::VecStoreI32x4));
        assert!(!has_vector_location_contract(&O::VecStoreI64x2));
        assert!(!has_vector_location_contract(&O::SqrtF64));
    }

    /// The metadata widening is inert unless the scalar-FP stack experiment is
    /// explicitly enabled: this predicate feeds only `disable_scalar_fp_xmm`,
    /// and `ra_config_tests::parser_covers_every_boolean_ra_switch_without_process_environment`
    /// already pins that switch to default-off. What is pinned HERE is the
    /// other half of the argument: every newly covered producer is a genuine
    /// vector *result*, so the widening cannot smuggle in a store or a
    /// scalar-result intrinsic that only has SIMD inputs.
    #[test]
    fn central_classifier_covering_new_producers_implies_vector_results() {
        for op in [O::AddPs128, O::BroadcastLoadF64, O::VecSubI32x4] {
            assert!(has_vector_location_contract(&op), "{op:?} must be covered");
            assert!(
                op.vector_result_width().is_some(),
                "{op:?} must reach the contract through result metadata"
            );
        }
        // A store has SIMD operands but no result, and must stay out.
        for op in [O::VecStoreI32x4, O::VecStoreI64x2] {
            assert!(
                op.vector_result_width().is_none(),
                "{op:?} must not report a vector result"
            );
            assert!(!has_vector_location_contract(&op), "{op:?} must stay out");
        }
    }

    #[test]
    fn every_horizontal_reduction_keeps_vector_location_contract() {
        use std::collections::BTreeSet;

        let horizontal = [
            O::VecHorizontalAddF64x4,
            O::VecHorizontalAddF64x2,
            O::VecHorizontalAddI32x8,
            O::VecHorizontalAddI32x4,
            O::VecHorizontalAddI64x4,
            O::VecHorizontalAddI64x2,
            O::VecHorizontalAddF32x8,
            O::VecHorizontalAddF32x4,
            O::VecHorizontalMaxI32x4,
            O::VecHorizontalMaxI32x8,
            O::VecHorizontalMinI32x8,
            O::HorizontalAddF64x4,
            O::HorizontalAddF64x2,
            O::HorizontalAddI32x8,
            O::HorizontalAddI32x4,
        ];

        // Keep this admission exhaustive as IntrinsicOp evolves.  Parse only
        // variant tokens from the enum, excluding its documentation comments.
        let source = include_str!("../../ir/intrinsics.rs");
        let enum_body = source
            .split_once("pub enum IntrinsicOp")
            .expect("IntrinsicOp enum")
            .1
            .split_once("\n}\n")
            .expect("end of IntrinsicOp enum")
            .0;
        let expected: BTreeSet<String> = enum_body
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .flat_map(|line| line.split([',', '|']))
            .map(str::trim)
            .filter(|token| {
                token.contains("Horizontal")
                    && token.bytes().all(|byte| byte.is_ascii_alphanumeric())
            })
            .map(str::to_owned)
            .collect();
        let actual: BTreeSet<String> = horizontal.iter().map(|op| format!("{op:?}")).collect();
        assert_eq!(actual, expected, "horizontal intrinsic coverage drifted");
        for op in horizontal {
            assert!(has_vector_location_contract(&op), "lost {op:?}");
        }
    }
}

#[cfg(test)]
mod param_alloca_index_tests {
    use super::ParamAllocaIndex;
    use crate::common::types::IrType;
    use crate::ir::instruction::{BasicBlock, Instruction, Terminator};
    use crate::ir::reexports::{BlockId, IrFunction, Value};

    fn alloca(dest: u32, ty: IrType) -> Instruction {
        Instruction::Alloca {
            dest: Value(dest),
            ty,
            size: ty.size() as usize,
            align: 4,
            volatile: false,
            semantic_volatile: false,
        }
    }

    /// The aligned hypothesis, evaluated with the same shared-buffer API the
    /// production `build_homes` uses (capacity reserved for `n` entries).
    fn aligned_hypothesis(f: &IrFunction) -> bool {
        let mut buf: Vec<Option<(Value, IrType)>> = Vec::with_capacity(f.param_alloca_values.len());
        super::aligned_homes(f, &mut super::NoStats, &mut buf)
    }

    fn function(instructions: Vec<Instruction>, homes: Vec<u32>) -> IrFunction {
        let mut f = IrFunction::new("t".to_string(), IrType::Void, Vec::new(), false);
        f.blocks = vec![BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        }];
        f.param_alloca_values = homes.into_iter().map(Value).collect();
        f
    }

    /// The exact pre-index implementation, used as the oracle.  Nothing in
    /// the production path calls a scan any more; this exists solely so the
    /// tests can prove the index is a semantic identity.
    fn reference_full_scan(f: &IrFunction, param_idx: usize) -> Option<(Value, IrType)> {
        let want = *f.param_alloca_values.get(param_idx)?;
        f.blocks.first().and_then(|block| {
            block.instructions.iter().find_map(|inst| match inst {
                Instruction::Alloca { dest, ty, .. } if *dest == want => Some((*dest, *ty)),
                _ => None,
            })
        })
    }

    #[test]
    fn aligned_homes_are_indexed_with_the_same_answer() {
        let f = function(
            vec![
                alloca(10, IrType::I32),
                alloca(11, IrType::I64),
                alloca(12, IrType::F64),
            ],
            vec![10, 11, 12],
        );
        let index = ParamAllocaIndex::build(&f);
        assert_eq!(index.len(), 3);
        assert!(!index.is_empty());
        assert_eq!(index.get(0), Some((Value(10), IrType::I32)));
        assert_eq!(index.get(1), Some((Value(11), IrType::I64)));
        assert_eq!(index.get(2), Some((Value(12), IrType::F64)));
        assert_eq!(index.get(3), None);
    }

    /// A prefix instruction or a removed parameter home shifts the layout.
    /// Unlike the old O(1) probe, the index does not need a fallback: the
    /// answer is still exact (this is the 20180112-1.c mem2reg class).
    #[test]
    fn shifted_layout_is_exact_without_a_scan() {
        let f = function(
            vec![
                alloca(99, IrType::I32), // unrelated local first
                alloca(10, IrType::I32),
                alloca(11, IrType::I64),
            ],
            vec![10, 11],
        );
        let index = ParamAllocaIndex::build(&f);
        assert_eq!(index.get(0), Some((Value(10), IrType::I32)));
        assert_eq!(index.get(1), Some((Value(11), IrType::I64)));
    }

    /// Exhaustive proof over EVERY small layout (no sampling): all prefix
    /// lengths, all subsets of homes removed by mem2reg, all I32/I64/F32/F64
    /// width assignments, and every index including out-of-range ones.
    #[test]
    fn index_matches_the_reference_scan_over_all_small_layouts() {
        let tys = [IrType::I32, IrType::I64, IrType::F32, IrType::F64];
        let mut layouts = 0usize;
        let mut checks = 0usize;
        for nprefix in 0..=3usize {
            for nparams in 0..=4usize {
                let masks = 1u32 << nparams;
                // One bit per home: is it still present after mem2reg?
                for present in 0..masks {
                    // One bit per home: which width class the alloca has.
                    for widths in 0..masks {
                        let mut instructions = Vec::new();
                        let mut next_dest = 1u32;
                        for i in 0..nprefix {
                            instructions.push(alloca(next_dest, tys[(i + nprefix) % 4]));
                            next_dest += 1;
                        }
                        let mut homes = Vec::new();
                        for i in 0..nparams {
                            let ty = if ((widths >> i) & 1) == 0 {
                                tys[(i + nparams) % 2]
                            } else {
                                tys[2 + (i + nprefix) % 2]
                            };
                            if ((present >> i) & 1) == 1 {
                                homes.push(next_dest);
                            } else {
                                homes.push(9000 + i as u32); // deliberately absent
                            }
                            instructions.push(alloca(next_dest, ty));
                            next_dest += 1;
                        }
                        let f = function(instructions, homes);
                        let index = ParamAllocaIndex::build(&f);
                        assert_eq!(index.len(), nparams);
                        layouts += 1;
                        for i in 0..=nparams + 1 {
                            assert_eq!(
                                index.get(i),
                                reference_full_scan(&f, i),
                                "layout {layouts}: prefix={nprefix} params={nparams} \
                                 present={present:b} widths={widths:b} index={i}"
                            );
                            checks += 1;
                        }
                    }
                }
            }
        }
        // Guard against a vacuous enumeration (e.g. a loop bound regression).
        assert!(
            layouts == 1364 && checks == 7736,
            "expected 1364 layouts / 7736 checks, got {layouts} / {checks}"
        );
    }

    /// Randomized large-layout proof: aligned homes, unrelated prefix
    /// allocas, removed homes, out-of-range indices and mixed types.
    #[test]
    fn index_matches_the_reference_scan_over_random_layouts() {
        let mut seed = 0xC0FF_EE11_DEAD_1234u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let tys = [IrType::I32, IrType::I64, IrType::F32, IrType::F64];
        for _ in 0..5000 {
            let nparams = (next() % 8) as usize;
            let nprefix = (next() % 4) as usize;
            let mut instructions = Vec::new();
            let mut next_dest = 1u32;
            for _ in 0..nprefix {
                instructions.push(alloca(next_dest, tys[(next() % 4) as usize]));
                next_dest += 1;
            }
            let mut homes = Vec::new();
            for i in 0..nparams {
                // Some parameter homes are removed by mem2reg.
                if next() % 5 == 0 {
                    homes.push(9000 + i as u32); // deliberately absent
                } else {
                    homes.push(next_dest);
                }
                instructions.push(alloca(next_dest, tys[(next() % 4) as usize]));
                next_dest += 1;
            }
            let f = function(instructions, homes);
            let index = ParamAllocaIndex::build(&f);
            for i in 0..=nparams + 1 {
                assert_eq!(
                    index.get(i),
                    reference_full_scan(&f, i),
                    "param {i} of {} homes",
                    nparams
                );
            }
        }
    }

    #[test]
    fn aligned_layout_takes_exactly_one_allocation() {
        let f = function(
            vec![
                alloca(10, IrType::I32),
                alloca(11, IrType::I64),
                alloca(12, IrType::F64),
                alloca(13, IrType::I32), // trailing unrelated local is irrelevant
            ],
            vec![10, 11, 12],
        );
        assert!(aligned_hypothesis(&f));
        assert_eq!(
            ParamAllocaIndex::build(&f).get(2),
            Some((Value(12), IrType::F64))
        );
        // The aligned path allocates exactly the `homes` Vec -- no hash table,
        // no adjacency vector.  The old test name said "zero allocation",
        // which was simply false and is now pinned by the witness.
        let (_, stats) = ParamAllocaIndex::build_with_stats(&f);
        assert_eq!(stats.allocations, 1);

        // A prefix alloca, a missing home and non-increasing home ids all
        // route to the general path, which is exactly equivalent.
        let shifted = function(
            vec![
                alloca(99, IrType::I32),
                alloca(10, IrType::I32),
                alloca(11, IrType::I64),
            ],
            vec![10, 11],
        );
        assert!(!aligned_hypothesis(&shifted));
        let promoted = function(vec![alloca(99, IrType::I32)], vec![10]);
        assert!(!aligned_hypothesis(&promoted));
        let duplicates = function(
            vec![alloca(10, IrType::I32), alloca(10, IrType::I64)],
            vec![10, 10],
        );
        assert!(!aligned_hypothesis(&duplicates));
        assert_eq!(
            ParamAllocaIndex::build(&duplicates).get(0),
            reference_full_scan(&duplicates, 0)
        );
        let no_blocks = {
            let mut f = function(vec![], vec![]);
            f.blocks.clear();
            f
        };
        assert_eq!(ParamAllocaIndex::build(&no_blocks).len(), 0);
    }

    #[test]
    fn a_home_removed_by_mem2reg_is_not_replaced_by_a_later_alloca() {
        let f = function(vec![alloca(99, IrType::I32)], vec![10]);
        let index = ParamAllocaIndex::build(&f);
        assert_eq!(index.get(0), None);
    }

    #[test]
    fn the_first_matching_alloca_wins_even_when_a_dest_is_reused() {
        // SSA says dests are unique, but the former scan returned the FIRST
        // match; a malformed IR with a reused dest must not silently pick a
        // different width from a later instruction.
        let f = function(
            vec![
                alloca(10, IrType::F32),
                alloca(77, IrType::I32),
                alloca(10, IrType::I64),
            ],
            vec![10],
        );
        assert_eq!(
            ParamAllocaIndex::build(&f).get(0),
            reference_full_scan(&f, 0)
        );
        assert_eq!(
            ParamAllocaIndex::build(&f).get(0),
            Some((Value(10), IrType::F32))
        );
    }

    #[test]
    fn duplicate_home_ids_resolve_to_the_same_first_alloca() {
        let f = function(
            vec![
                alloca(10, IrType::I64),
                alloca(10, IrType::F64),
                alloca(11, IrType::I32),
            ],
            vec![10, 10, 11],
        );
        let index = ParamAllocaIndex::build(&f);
        for i in 0..=3 {
            assert_eq!(index.get(i), reference_full_scan(&f, i), "parameter {i}");
        }
    }

    #[test]
    fn edge_cases_are_none_or_empty() {
        let f = function(vec![], vec![]);
        let index = ParamAllocaIndex::build(&f);
        assert!(index.is_empty());
        assert_eq!(index.len(), 0);
        assert_eq!(index.get(0), None);
        assert_eq!(index, ParamAllocaIndex::default());

        let mut no_blocks = function(vec![alloca(1, IrType::I32)], vec![1]);
        no_blocks.blocks.clear();
        assert_eq!(ParamAllocaIndex::build(&no_blocks).get(0), None);

        // A later block must not contribute a home: only the entry block is
        // authoritative, exactly as in the original scan.
        let mut two_blocks = function(vec![alloca(99, IrType::I32)], vec![1]);
        two_blocks.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![alloca(1, IrType::I64)],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        assert_eq!(ParamAllocaIndex::build(&two_blocks).get(0), None);
    }

    /// The whole point of the refactor: ONE build answers every query, with
    /// no per-query dependence on the entry block length.  This test uses a
    /// deliberately adversarial layout (no home is aligned, all homes are at
    /// the very end) and checks every ordinal.
    #[test]
    fn one_build_answers_every_ordinal_in_a_shifted_large_layout() {
        const N: usize = 1024;
        let tys = [IrType::I32, IrType::I64, IrType::F32, IrType::F64];
        let mut instructions = Vec::with_capacity(2 * N);
        for i in 0..N {
            instructions.push(alloca(50_000 + i as u32, tys[i % 4]));
        }
        let homes: Vec<u32> = (0..N).map(|i| 100_000 + i as u32).collect();
        for (i, dest) in homes.iter().enumerate() {
            instructions.push(alloca(*dest, tys[(i + 1) % 4]));
        }
        let f = function(instructions, homes.clone());
        let index = ParamAllocaIndex::build(&f);
        for (i, dest) in homes.iter().enumerate() {
            assert_eq!(
                index.get(i),
                Some((Value(*dest), tys[(i + 1) % 4])),
                "parameter {i}"
            );
        }
        assert_eq!(index.get(N), None);
    }

    /// The complexity contract: building the index examines each entry-block
    /// instruction at most once per build path, so the total is bounded by
    /// `params + entry_instructions` -- NEVER `params * entry_instructions`.
    /// This is the machine-independent proof that the old O(N^2) fallback
    /// cannot come back, and it is asserted on the ADVERSARIAL shifted layout
    /// (all homes at the very end, none aligned) rather than the easy case.
    #[test]
    fn build_is_linear_in_instructions_even_for_adversarial_layouts() {
        const N: usize = 4096;
        let tys = [IrType::I32, IrType::I64, IrType::F32, IrType::F64];
        let mut instructions = Vec::with_capacity(2 * N);
        for i in 0..N {
            instructions.push(alloca(50_000 + i as u32, tys[i % 4]));
        }
        let homes: Vec<u32> = (0..N).map(|i| 100_000 + i as u32).collect();
        for (i, dest) in homes.iter().enumerate() {
            instructions.push(alloca(*dest, tys[(i + 1) % 4]));
        }
        let entry_instructions = instructions.len();
        let f = function(instructions, homes.clone());

        let (index, stats) = ParamAllocaIndex::build_with_stats(&f);
        assert!(
            stats.instructions_examined <= N + entry_instructions,
            "examined {} instructions for {} params + {} entry instructions",
            stats.instructions_examined,
            N,
            entry_instructions
        );
        // `params * entry_instructions` is the shape the old fallback had;
        // assert we are orders of magnitude below it so a regression that is
        // merely "linear with a huge constant" cannot slip through.
        assert!(stats.instructions_examined < (N * entry_instructions) / 16);
        for (i, dest) in homes.iter().enumerate() {
            assert_eq!(index.get(i), Some((Value(*dest), tys[(i + 1) % 4])));
        }

        // The aligned path examines exactly `params` instructions.
        let aligned_instructions: Vec<Instruction> = (0..N)
            .map(|i| alloca(200_000 + i as u32, tys[i % 4]))
            .collect();
        let aligned_homes_ids: Vec<u32> = (0..N).map(|i| 200_000 + i as u32).collect();
        let aligned = function(aligned_instructions, aligned_homes_ids);
        let (_, aligned_stats) = ParamAllocaIndex::build_with_stats(&aligned);
        assert_eq!(aligned_stats.instructions_examined, N);
    }

    /// The complexity witness is only evidence about the SHIPPED path if the
    /// instrumented instantiation produces the identical index.  Prove that
    /// identity on every shape that selects a different build path.
    #[test]
    fn instrumented_build_is_identical_to_the_shipped_build() {
        let tys = [IrType::I32, IrType::I64, IrType::F32, IrType::F64];
        let mut cases: Vec<IrFunction> = Vec::new();
        // (a) aligned, (b) shifted by a prefix, (c) promoted away,
        // (d) duplicate home ids, (e) a reused dest with a different width,
        // (f) two blocks, (g) no blocks.
        cases.push(function(
            (0..6)
                .map(|i| alloca(10 + i, tys[i as usize % 4]))
                .collect(),
            (0..6).map(|i| 10 + i).collect(),
        ));
        cases.push(function(
            vec![
                alloca(99, IrType::I32),
                alloca(10, IrType::I32),
                alloca(11, IrType::F64),
            ],
            vec![10, 11],
        ));
        cases.push(function(vec![alloca(99, IrType::I32)], vec![10]));
        cases.push(function(
            vec![
                alloca(10, IrType::I64),
                alloca(10, IrType::F64),
                alloca(11, IrType::I32),
            ],
            vec![10, 10, 11],
        ));
        cases.push(function(
            vec![
                alloca(10, IrType::F32),
                alloca(77, IrType::I32),
                alloca(10, IrType::I64),
            ],
            vec![10],
        ));
        let mut two = function(vec![alloca(99, IrType::I32)], vec![1]);
        two.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![alloca(1, IrType::I64)],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        cases.push(two);
        let mut none = function(vec![alloca(1, IrType::I32)], vec![1]);
        none.blocks.clear();
        cases.push(none);

        for (case, f) in cases.iter().enumerate() {
            let shipped = ParamAllocaIndex::build(f);
            let (witness, _) = ParamAllocaIndex::build_with_stats(f);
            assert_eq!(
                shipped, witness,
                "case {case}: instrumentation changed the index"
            );
            for i in 0..=f.param_alloca_values.len() + 1 {
                assert_eq!(
                    shipped.get(i),
                    reference_full_scan(f, i),
                    "case {case}: ordinal {i}"
                );
            }
        }
    }

    /// A late-failing aligned probe must cost at most `params` extra
    /// examinations, never a second unbounded scan.  The count is pinned
    /// exactly, so a future change that re-walks the whole prefix twice is
    /// caught by a machine-independent assertion rather than a timing test.
    #[test]
    fn failed_aligned_probe_costs_exactly_one_bounded_prefix() {
        const N: usize = 64usize;
        const FAIL_AT: usize = N - 1; // aligned hypothesis survives 63 checks
        let tys = [IrType::I32, IrType::I64, IrType::F32, IrType::F64];
        let homes: Vec<u32> = (0..N).map(|i| 1000 + i as u32).collect();
        let mut instructions: Vec<Instruction> = Vec::with_capacity(2 * N);
        for i in 0..FAIL_AT {
            instructions.push(alloca(homes[i], tys[i % 4]));
        }
        // The probe breaks here: dest differs from wants[FAIL_AT].
        instructions.push(alloca(9999, IrType::I32));
        // Authoritative homes at the very end force the general pass to walk
        // the whole entry block, making the expected count unambiguous.
        for (i, dest) in homes.iter().enumerate() {
            instructions.push(alloca(*dest, tys[(i + 1) % 4]));
        }
        let entry_instructions = instructions.len();
        let f = function(instructions, homes.clone());
        let (index, stats) = ParamAllocaIndex::build_with_stats(&f);
        // FAIL_AT + 1 examinations in the probe, then one full general pass.
        assert_eq!(
            stats.instructions_examined,
            FAIL_AT + 1 + entry_instructions,
            "the fallback must not re-examine more than the proven prefix"
        );
        assert!(stats.instructions_examined <= N + entry_instructions);
        for i in 0..=N {
            assert_eq!(index.get(i), reference_full_scan(&f, i), "ordinal {i}");
        }
    }

    /// The flat adjacency chain replaces up to `params` tiny Vec allocations
    /// with one vector.  Portable proof: the instrumented build counts exactly
    /// three heap objects on the general path for every parameter count from
    /// 1 to 4096, and exactly one on the aligned path.  No wall-clock ratios,
    /// no CI-machine-dependent claims.
    #[test]
    fn allocations_are_constant_in_the_number_of_parameters() {
        let tys = [IrType::I32, IrType::I64, IrType::F32, IrType::F64];
        for n in [1usize, 2, 8, 64, 1024, 4096] {
            let homes: Vec<u32> = (0..n).map(|i| 100_000 + i as u32).collect();
            // One unrelated prefix alloca guarantees the aligned probe fails
            // at instruction 0, so this exercises the general path.
            let mut instructions = vec![alloca(7, IrType::I32)];
            instructions.extend(
                homes
                    .iter()
                    .enumerate()
                    .map(|(i, d)| alloca(*d, tys[i % 4])),
            );
            let entry_instructions = instructions.len();
            let f = function(instructions.clone(), homes.clone());
            assert!(!aligned_hypothesis(&f));
            let (index, stats) = ParamAllocaIndex::build_with_stats(&f);
            assert_eq!(stats.allocations, 3, "general path with {n} params");
            assert!(stats.instructions_examined <= n + entry_instructions);
            for i in 0..=n {
                assert_eq!(
                    index.get(i),
                    reference_full_scan(&f, i),
                    "general path, {n} params, ordinal {i}"
                );
            }

            let aligned = function(instructions[1..].to_vec(), homes.clone());
            assert!(aligned_hypothesis(&aligned));
            let (_, aligned_stats) = ParamAllocaIndex::build_with_stats(&aligned);
            assert_eq!(aligned_stats.allocations, 1, "aligned path with {n} params");
        }
        // The empty function must not allocate at all.
        let (_, empty_stats) = ParamAllocaIndex::build_with_stats(&function(vec![], vec![]));
        assert_eq!(empty_stats.allocations, 0);
    }

    /// Every ordinal sharing a home id forms one chain of length `params`.
    /// Walk it to exhaustion: all ordinals must be filled from the FIRST
    /// matching `Alloca`, later repeated allocas with the same dest must not
    /// change any width, and the labelled early exit must terminate.
    #[test]
    fn duplicate_chains_resolve_from_the_first_matching_alloca() {
        const N: usize = 4096;
        let homes = vec![42u32; N];
        let instructions = vec![
            alloca(1, IrType::I32),  // unrelated prefix
            alloca(42, IrType::F64), // authoritative FIRST match
            alloca(42, IrType::I8),  // must never win
            alloca(99, IrType::I32), // unrelated
            alloca(42, IrType::F32), // must never win
        ];
        let f = function(instructions, homes);
        assert!(!aligned_hypothesis(&f));
        let (index, stats) = ParamAllocaIndex::build_with_stats(&f);
        assert_eq!(stats.allocations, 3);
        // prefix + first matching alloca; the walk stops there.
        assert_eq!(stats.instructions_examined, 2);
        for i in 0..=N {
            assert_eq!(index.get(i), reference_full_scan(&f, i), "ordinal {i}");
            if i < N {
                assert_eq!(index.get(i), Some((Value(42), IrType::F64)));
            }
        }
        // The instrumented and shipped builds must agree on this shape too.
        assert_eq!(index, ParamAllocaIndex::build(&f));
    }

    /// Interleaved non-alloca instructions, non-monotone home ids, several
    /// duplicate ids and missing homes all at once: an end-to-end shape that
    /// no single existing test covers.  Compare every ordinal (including
    /// out-of-range) against the pre-index oracle.
    #[test]
    fn mixed_layout_with_duplicates_and_missing_homes_matches_the_oracle() {
        let tys = [IrType::I32, IrType::I64, IrType::F32, IrType::F64];
        let homes = vec![90u32, 10, 10, 70, 90, 10, 999, 70];
        let instructions = vec![
            alloca(10, tys[0]),
            Instruction::StackSave { dest: Value(3) }, // a non-Alloca instruction
            alloca(70, tys[1]),
            alloca(90, tys[2]),
            alloca(10, tys[3]), // duplicate dest, different width
            alloca(70, tys[0]), // duplicate dest, different width
            alloca(55, tys[1]), // unrelated local
            alloca(90, tys[2]), // duplicate dest
        ];
        let f = function(instructions, homes);
        assert!(!aligned_hypothesis(&f));
        let (index, stats) = ParamAllocaIndex::build_with_stats(&f);
        assert_eq!(stats.allocations, 3);
        assert_eq!(index, ParamAllocaIndex::build(&f));
        for i in 0..=f.param_alloca_values.len() {
            assert_eq!(index.get(i), reference_full_scan(&f, i), "ordinal {i}");
        }
        // The missing home is deliberately unresolved; duplicates all resolve
        // to the first matching alloca, not the later wider/narrower ones.
        assert_eq!(index.get(6), None);
        assert_eq!(index.get(1), Some((Value(10), tys[0])));
        assert_eq!(index.get(5), Some((Value(10), tys[0])));
    }
}
