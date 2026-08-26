//! Late redundant-load elimination within basic blocks.
//!
//! After strength reduction, array/struct field accesses appear as
//! `Load(GEP(base, const_off))` with plain constant offsets. When the
//! frontend emits the same field access in several statements (e.g.
//! `bodies[j].mass` in each of three velocity updates), the duplicate loads
//! survive GVN because GEP CSE is disabled (distinct GEP value ids key the
//! value numbers). This pass merges loads of the same linear-form address
//! within a block when every intervening store is provably non-aliasing
//! (`alias::forms_disjoint`).
//!
//! Adapted from levkropp/lccc (Aug 19, 2026) with two hard gates this tree
//! requires:
//!  * **volatile loads never merge** — each volatile read is an observable
//!    side effect (C11 5.1.2.3); collapsing two reads into one changes the
//!    program's observable behavior.  (The upstream original predates the
//!    IR volatile flags entirely.)
//!  * volatile STORES are memory writes like any other store for the
//!    aliasing math — the form-based retain handles them; no special case
//!    is needed for soundness of merging *non-volatile* loads across them.
//!
//! A duplicate's SSA uses are rewritten function-wide after analysis; the
//! canonical load is earlier in the same block, so every dominated use stays
//! dominated (SSA use sites of the duplicate are, by SSA construction,
//! dominated by the duplicate's block, hence by the canonical def's block).

use super::alias;
use crate::common::fx_hash::FxHashMap;
use crate::ir::reexports::{Instruction, IrFunction, Operand, Terminator, Value};

pub(crate) fn run(func: &mut IrFunction) -> usize {
    if std::env::var("CCC_NO_REDUNDANT_LOADS").is_ok() {
        return 0;
    }
    if func.blocks.is_empty() {
        return 0;
    }
    let cfg = crate::ir::analysis::CfgAnalysis::build(func);
    let frames = alias::LoopFrames::build_with_cfg(func, &cfg);

    // Analysis phase: immutable borrow of func plus a defs map over it.
    let mut all_rewrites: FxHashMap<u32, u32> = FxHashMap::default();
    let mut per_block_removals: Vec<Vec<usize>> = Vec::with_capacity(func.blocks.len());
    {
        let mut defs: FxHashMap<u32, &Instruction> = FxHashMap::default();
        for block in func.blocks.iter() {
            for inst in &block.instructions {
                if let Some(dest) = inst.dest() {
                    defs.insert(dest.0, inst);
                }
            }
        }

        for bi in 0..func.blocks.len() {
            let frame = frames.block_frame[bi];
            // (form, type, canonical dest)
            let mut available: Vec<(alias::LinForm, crate::common::types::IrType, Value)> =
                Vec::new();
            let mut removed: Vec<usize> = Vec::new();

            for (ii, inst) in func.blocks[bi].instructions.iter().enumerate() {
                match inst {
                    Instruction::Load {
                        dest,
                        ptr,
                        ty,
                        seg_override,
                        volatile,
                    } => {
                        if *volatile {
                            // Observable side effect: must not be merged away.
                            continue;
                        }
                        if *seg_override != crate::common::types::AddressSpace::Default
                            || ty.is_long_double()
                            || ty.is_128bit()
                        {
                            continue;
                        }
                        let Some(form) = alias::resolve_in_frame(func, &defs, &frames, frame, *ptr)
                        else {
                            continue;
                        };
                        if let Some((_, _, canon)) =
                            available.iter().find(|(f, t, _)| *f == form && *t == *ty)
                        {
                            all_rewrites.insert(dest.0, canon.0);
                            removed.push(ii);
                            continue;
                        }
                        available.push((form, *ty, *dest));
                    }
                    Instruction::Store { ptr, ty, .. } => {
                        if available.is_empty() {
                            continue;
                        }
                        match alias::resolve_in_frame(func, &defs, &frames, frame, *ptr) {
                            Some(sform) => {
                                let ssz = crate::passes::loop_memory_promote::byte_size(*ty);
                                let keep = |entry: &(
                                    alias::LinForm,
                                    crate::common::types::IrType,
                                    Value,
                                )| {
                                    match (
                                        crate::passes::loop_memory_promote::byte_size(entry.1),
                                        ssz,
                                    ) {
                                        // Keep ONLY loads provably disjoint from the store:
                                        // a may-aliasing store clobbers the loaded value,
                                        // so anything not proven disjoint must be
                                        // invalidated. (The inverted predicate kept stale
                                        // loads: gvn_global_partial_store forwarded a word
                                        // load across a byte store to the same address.)
                                        (Some(lsz), Some(ssz)) => {
                                            alias::forms_disjoint(&entry.0, lsz, &sform, ssz, true)
                                        }
                                        // Unknown size: not provably disjoint.
                                        _ => false,
                                    }
                                };
                                available.retain(keep);
                            }
                            None => available.clear(),
                        }
                    }
                    // Calls, atomics, inline asm, variadics: unknown memory
                    // effects, kill everything.  Intrinsics with a write
                    // pointer kill everything (read-only intrinsics are fine).
                    Instruction::Call { .. }
                    | Instruction::CallIndirect { .. }
                    | Instruction::Memcpy { .. }
                    | Instruction::AtomicRmw { .. }
                    | Instruction::AtomicCmpxchg { .. }
                    | Instruction::AtomicStore { .. }
                    | Instruction::Fence { .. }
                    | Instruction::InlineAsm { .. }
                    | Instruction::VaStart { .. }
                    | Instruction::VaEnd { .. }
                    | Instruction::VaCopy { .. } => {
                        available.clear();
                    }
                    Instruction::Intrinsic {
                        dest_ptr: Some(_), ..
                    } => {
                        available.clear();
                    }
                    Instruction::AtomicLoad { .. } => {
                        // Atomics are fenced memory effects: conservative kill.
                        available.clear();
                    }
                    _ => {}
                }
            }
            per_block_removals.push(removed);
        }
    } // defs dropped: mutation phase

    let mut total = 0usize;
    for (bi, removed) in per_block_removals.into_iter().enumerate() {
        if removed.is_empty() {
            continue;
        }
        let mut removed_iter = removed.iter().copied();
        let mut next_remove = removed_iter.next();
        let mut idx = 0usize;
        func.blocks[bi].instructions.retain(|_| {
            let cur = idx;
            idx += 1;
            if Some(cur) == next_remove {
                next_remove = removed_iter.next();
                total += 1;
                false
            } else {
                true
            }
        });
        // Parallel spans array must stay in lockstep with instructions.
        if !func.blocks[bi].source_spans.is_empty() {
            let mut removed_iter2 = removed.iter().copied();
            let mut next2 = removed_iter2.next();
            let mut idx2 = 0usize;
            func.blocks[bi].source_spans.retain(|_| {
                let cur = idx2;
                idx2 += 1;
                if Some(cur) == next2 {
                    next2 = removed_iter2.next();
                    false
                } else {
                    true
                }
            });
        }
    }

    if !all_rewrites.is_empty() {
        for block in func.blocks.iter_mut() {
            for inst in block.instructions.iter_mut() {
                super::tail_call_elim::replace_values_in_inst(inst, &all_rewrites);
            }
            match &mut block.terminator {
                Terminator::CondBranch { cond, .. } => {
                    if let Operand::Value(v) = cond {
                        if let Some(&to) = all_rewrites.get(&v.0) {
                            *v = Value(to);
                        }
                    }
                }
                Terminator::Switch { val: discr, .. } => {
                    if let Operand::Value(v) = discr {
                        if let Some(&to) = all_rewrites.get(&v.0) {
                            *v = Value(to);
                        }
                    }
                }
                Terminator::Return(Some(Operand::Value(v))) => {
                    if let Some(&to) = all_rewrites.get(&v.0) {
                        *v = Value(to);
                    }
                }
                _ => {}
            }
        }
    }

    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volatile_loads_are_never_merged() {
        // Two volatile loads of the same global must both survive: each read
        // is an observable side effect (C11 5.1.2.3).
        let mut func = crate::ir::reexports::IrFunction {
            name: "vol".into(),
            return_type: crate::common::types::IrType::I32,
            params: vec![],
            blocks: vec![crate::ir::reexports::BasicBlock {
                label: crate::ir::reexports::BlockId(1),
                instructions: vec![
                    crate::ir::reexports::Instruction::GlobalAddr {
                        dest: Value(0),
                        name: "g".into(),
                    },
                    crate::ir::reexports::Instruction::Load {
                        dest: Value(1),
                        ptr: Value(0),
                        ty: crate::common::types::IrType::I32,
                        seg_override: crate::common::types::AddressSpace::Default,
                        volatile: true,
                    },
                    crate::ir::reexports::Instruction::Load {
                        dest: Value(2),
                        ptr: Value(0),
                        ty: crate::common::types::IrType::I32,
                        seg_override: crate::common::types::AddressSpace::Default,
                        volatile: true,
                    },
                    crate::ir::reexports::Instruction::BinOp {
                        dest: Value(3),
                        op: crate::ir::reexports::IrBinOp::Add,
                        lhs: Operand::Value(Value(1)),
                        rhs: Operand::Value(Value(2)),
                        ty: crate::common::types::IrType::I32,
                    },
                ],
                terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
                source_spans: Vec::new(),
            }],
            is_variadic: false,
            is_declaration: false,
            is_static: false,
            is_inline: false,
            is_always_inline: false,
            is_noinline: false,
            next_value_id: 4,
            fp_expr_tags: Default::default(),
            next_label: 2,
            section: None,
            visibility: None,
            is_weak: false,
            is_used: false,
            has_inlined_calls: false,
            param_alloca_values: vec![],
            uses_sret: false,
            is_fastcall: false,
            is_naked: false,
            no_instrument: false,
            global_init_label_blocks: vec![],
            ret_eightbyte_classes: vec![],
            ret_is_f128_sse: false,
            is_gnu_inline_def: false,
            loop_promoted_f64_values: Vec::new(),
        };
        let removed = run(&mut func);
        assert_eq!(removed, 0, "volatile loads must not merge");
        let loads = func.blocks[0]
            .instructions
            .iter()
            .filter(|i| matches!(i, crate::ir::reexports::Instruction::Load { .. }))
            .count();
        assert_eq!(loads, 2);
    }
}
