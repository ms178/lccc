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
    // x86-64 is the only backend that publishes an indirect-call target
    // register (`%r10` = PhysReg(11)); it is also the backend whose
    // caller-saved pool (r11/r10/r8/r9/rdi/rsi) is wide enough for a leaf's
    // loop state. i686 (3 scratch registers, %eax/%ecx/%edx hazards) keeps
    // the Phase-1 hot-loop promotion.
    let leaf_caller_saved_homes = !crate::common::types::target_is_32bit()
        && indirect_target_regs.iter().any(|r| r.0 == 11)
        && caller_saved_regs.len() >= 4;
    let mut config = super::super::regalloc::RegAllocConfig {
        ipa_regs,
        available_regs,
        accumulator_policy,
        caller_saved_regs,
        leaf_caller_saved_homes,
        call_arg_regs,
        indirect_target_regs,
        allow_inline_asm_regalloc,
        xmm_regs,
        never_materialized: never_materialized.unwrap_or_default(),
        folded_index_uses,
        fold_base_regs,
        fold_readmit: FxHashSet::default(),
        reg_hints,
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

/// Find the nth alloca instruction in the entry block (used for parameter storage).
pub fn find_param_alloca(func: &IrFunction, param_idx: usize) -> Option<(Value, IrType)> {
    // Parameter allocas are authoritative in `func.param_alloca_values`; using
    // "the Nth alloca in the entry block" is unsound after mem2reg removes a
    // parameter home.  A later local alloca (notably a volatile local) can then
    // be mistaken for the parameter slot, making emit_store_params overwrite it
    // with the incoming argument and later reload the local as the pointer
    // parameter (gcc.c-torture/execute/20180112-1.c).  Look up the indexed
    // parameter-home value and verify that its Alloca still exists.
    let want = *func.param_alloca_values.get(param_idx)?;
    func.blocks.first().and_then(|block| {
        block.instructions.iter().find_map(|inst| match inst {
            Instruction::Alloca { dest, ty, .. } if *dest == want => Some((*dest, *ty)),
            _ => None,
        })
    })
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
