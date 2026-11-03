//! Value use-block analysis, used-value collection, and dead parameter alloca detection.
//!
//! These functions build the foundational data structures that drive the
//! three-tier slot allocation: which blocks each value is used in, which
//! values are referenced at all, and which parameter allocas are dead.

use crate::backend::liveness::{
    for_each_operand_in_instruction, for_each_operand_in_terminator,
    for_each_value_use_in_instruction,
};
use crate::backend::regalloc::{PhysReg, RaConfig};
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::{Instruction, IrFunction, Operand};

/// Compute the "use-block map" for all values in the function.
/// For each value, records the set of block indices where that value is referenced.
///
/// Phi node operands are attributed to their SOURCE block (not the Phi's block).
/// This is critical for coalescing in dispatch-style code (switch/computed goto):
/// a value defined in block A that flows through a Phi to block B is semantically
/// "used at the end of block A" (where the branch resolves the Phi). Attributing
/// Phi uses to the source block allows such values to be considered block-local
/// to A, enabling stack slot coalescing across case handlers.
pub(super) fn compute_value_use_blocks(func: &IrFunction) -> FxHashMap<u32, Vec<usize>> {
    let mut uses: FxHashMap<u32, Vec<usize>> = FxHashMap::default();

    let record_use = |id: u32, block_idx: usize, uses: &mut FxHashMap<u32, Vec<usize>>| {
        let blocks = uses.entry(id).or_default();
        if blocks.last() != Some(&block_idx) {
            blocks.push(block_idx);
        }
    };

    // Build BlockId -> block_index map for Phi source block resolution.
    let block_id_to_idx: FxHashMap<u32, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(idx, b)| (b.label.0, idx))
        .collect();

    for (block_idx, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            // Handle Phi nodes specially: attribute each operand's use to its
            // source block rather than the block containing the Phi. This reflects
            // SSA semantics where Phi inputs are "evaluated" at the end of the
            // predecessor block, not at the start of the Phi's block.
            if let Instruction::Phi { incoming, .. } = inst {
                for (op, src_block_id) in incoming {
                    if let Operand::Value(v) = op {
                        if let Some(&src_idx) = block_id_to_idx.get(&src_block_id.0) {
                            record_use(v.0, src_idx, &mut uses);
                        } else {
                            // Fallback: if source block not found, attribute to Phi's block
                            record_use(v.0, block_idx, &mut uses);
                        }
                    }
                }
            } else {
                for_each_operand_in_instruction(inst, |op| {
                    if let Operand::Value(v) = op {
                        record_use(v.0, block_idx, &mut uses);
                    }
                });
            }
            for_each_value_use_in_instruction(inst, |v| {
                record_use(v.0, block_idx, &mut uses);
            });
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                record_use(v.0, block_idx, &mut uses);
            }
        });
    }
    uses
}

/// Collect all Value IDs referenced as operands anywhere in the function body.
pub(super) fn collect_used_values(func: &IrFunction) -> FxHashSet<u32> {
    let mut used: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    used.insert(v.0);
                }
            });
            for_each_value_use_in_instruction(inst, |v| {
                used.insert(v.0);
            });
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                used.insert(v.0);
            }
        });
    }
    used
}

/// Find param allocas that can safely skip stack slot allocation.
///
/// A param alloca is dead when BOTH conditions hold:
/// 1. No instruction references the alloca value (mem2reg promoted it away)
/// 2. The corresponding ParamRef dest IS register-assigned, so emit_store_params
///    can store the ABI arg register directly to the callee-saved register
///
/// Without condition 2, the param value would have nowhere to live: the alloca
/// slot was eliminated, the ABI register may be clobbered, and no callee-saved
/// register was assigned. This would cause emit_param_ref to fall back to
/// reading the (already-clobbered) ABI register.

/// True when the function contains any instruction that clobbers the
/// caller-saved registers: a call, or an operation lowered to one.
///
/// Used to decide whether a caller-saved GPR is a safe home for an incoming
/// parameter. Memcpy/memset and variadic argument handling are included
/// because they are lowered to `rep movsb` / helper sequences that destroy
/// rdi/rsi, exactly like a call would.
pub(super) fn function_makes_calls(func: &IrFunction) -> bool {
    if func.is_variadic {
        return true;
    }
    func.blocks.iter().any(|b| {
        b.instructions.iter().any(|i| match i {
            Instruction::Call { .. }
            | Instruction::Memcpy { .. }
            | Instruction::CallIndirect { .. }
            | Instruction::VaArgStruct { .. }
            | Instruction::VaArg { .. }
            | Instruction::VaStart { .. }
            | Instruction::VaCopy { .. }
            | Instruction::VaEnd { .. } => true,
            Instruction::AtomicLoad { ty, .. }
            | Instruction::AtomicStore { ty, .. }
            | Instruction::AtomicRmw { ty, .. }
            | Instruction::AtomicCmpxchg { ty, .. }
            | Instruction::AtomicInc { ty, .. } => {
                matches!(
                    ty,
                    crate::common::types::IrType::I128 | crate::common::types::IrType::U128
                )
            }
            _ => false,
        })
    })
}

/// x86-64 caller-saved GPR allocator IDs.  In a leaf function these are stable
/// parameter homes: emit_store_params first captures every stack-backed ABI
/// argument, then emits all register-backed parameters as one ordered parallel
/// copy (breaking cycles through reserved RAX).  Therefore even ABI-overlapping
/// homes r8/r9/rdi/rsi/rdx cannot destroy a later incoming value.
pub(super) fn is_leaf_caller_gpr(phys: PhysReg) -> bool {
    matches!(phys.0, 10..=16)
}

pub(super) fn find_dead_param_allocas(
    func: &IrFunction,
    used_values: &FxHashSet<u32>,
    reg_assigned: &FxHashMap<u32, PhysReg>,
    callee_saved_regs: &[PhysReg],
    ra_config: &RaConfig,
) -> FxHashSet<u32> {
    let mut dead = FxHashSet::default();
    if func.param_alloca_values.is_empty() {
        return dead;
    }

    // Build param_idx -> ParamRef dest Value map
    let mut paramref_dests: Vec<Option<u32>> = vec![None; func.param_alloca_values.len()];
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::ParamRef {
                dest, param_idx, ..
            } = inst
            {
                if *param_idx < paramref_dests.len() {
                    paramref_dests[*param_idx] = Some(dest.0);
                }
            }
        }
    }

    // Vector parameters read through one entry-prefix VecLoad whose dest
    // has an XMM home: the x86 prologue moves the incoming register straight
    // into that home (`emit_store_params_impl`, same predicate), so the
    // alloca is never written or read.
    // The predicate is the prologue's, term for term: (1) the helper hit,
    // (2) the load dest has an XMM home, (3) the parameter is passed in an
    // XMM register of exactly the vector's size (`ParamClass::F128SseReg`).
    // A 32-byte vector below the AVX ABI level (or any vector past the
    // eighth XMM) arrives in MEMORY: the prologue captures it through the
    // slot, so the slot must stay.
    let vector_direct = if crate::common::types::target_elf_machine() == 62 {
        super::regalloc_helpers::x86_vector_param_loads(func)
    } else {
        Vec::new()
    };
    let x86_param_classes = if vector_direct.iter().any(Option::is_some) {
        crate::backend::call_abi::classify_params(
            func,
            &crate::backend::call_abi::CallAbiConfig::x86_64(func.calling_convention),
        )
    } else {
        Vec::new()
    };
    for (idx, pv) in func.param_alloca_values.iter().enumerate() {
        if let Some(Some((dest, bytes))) = vector_direct.get(idx) {
            let in_xmm = matches!(
                x86_param_classes.get(idx),
                Some(crate::backend::call_abi::ParamClass::F128SseReg { size, .. })
                    if *size == *bytes && matches!(*size, 16 | 32)
            );
            if in_xmm
                && reg_assigned
                    .get(&dest.0)
                    .is_some_and(|phys| (20..=33).contains(&phys.0))
            {
                dead.insert(pv.0);
                continue;
            }
        }
        if !used_values.contains(&pv.0) {
            match paramref_dests.get(idx).copied().flatten() {
                Some(dest_id) => {
                    // ParamRef survived: eliminate the alloca only when the
                    // ParamRef dest has a stable register home that
                    // emit_store_params can pre-store the ABI arg into:
                    // - a callee-saved GPR (survives calls, never clobbered);
                    // - an XMM register (PhysReg 20+), which the allocator
                    //   only assigns to values that do NOT span calls.
                    // - any caller-saved GPR in a LEAF function. The x86
                    //   prologue captures stack-backed params first and emits
                    //   register homes as an ordered parallel copy, so even an
                    //   ABI-overlapping home cannot clobber a pending argument.
                    // Allowing this removes a store AND reload per parameter.
                    // `int cmp(const void*a, const void*b){return *(int*)a -
                    // *(int*)b;}` went from 11 instructions with a 24-byte
                    // frame to GCC's 3.
                    // An x86-64 body whose only "calls" are inline-expanded
                    // fixed-size memcpy/memset is a leaf for this purpose:
                    // the expansion touches %rdi/%rsi/%rcx/%rax/%xmm0/%xmm1
                    // only and `x86_param_caller_homes_safe` proves no
                    // parameter is read after it, so a caller-saved home is
                    // as stable as in a true leaf.  Without this, the
                    // parameter kept its (dead) alloca slot and the body
                    // round-tripped it through memory: `memset(p, c, 40)`
                    // was `subq $24,%rsp; movq %rsi,8(%rsp); movslq
                    // 8(%rsp),%rax; …` (FOLLOWUP_CPU_MODEL #1).  The same
                    // predicate gates the allocator's ordered parameter
                    // copies, so the two decisions cannot disagree.
                    let leaf = !function_makes_calls(func)
                        || (crate::common::types::target_elf_machine()
                            == 62 // EM_X86_64 (elf::constants is private)
                            && crate::backend::regalloc::x86_param_caller_homes_safe_with_config(func, ra_config));
                    let has_stable_home = reg_assigned
                        .get(&dest_id)
                        .map(|phys| {
                            callee_saved_regs.contains(phys)
                                || phys.0 >= 20
                                || (leaf && is_leaf_caller_gpr(*phys))
                        })
                        .unwrap_or(false);
                    if has_stable_home {
                        dead.insert(pv.0);
                    }
                }
                None => {
                    // The ParamRef is gone (constant-propagated away by IPCP,
                    // or the parameter is genuinely unused after DCE) AND the
                    // alloca has no uses. The parameter value flows nowhere,
                    // so the alloca is dead: skip the entry-store slot write
                    // (nbody's advance(0.01, ...) previously spilled the
                    // constant-folded dt to 0xc8(%rsp) every call).
                    dead.insert(pv.0);
                }
            }
        }
    }
    dead
}
