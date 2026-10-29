//! Redundant reload forwarding across a single predecessor edge.
//!
//! When a block B has exactly one predecessor P, and P ends with a Load of a
//! given pointer, and B reloads the same pointer (same type) with no
//! intervening memory clobber on either side, the reload is redundant — the
//! value was just loaded on the only incoming path. Replace it with a Copy of
//! the earlier load's destination:
//!
//! ```text
//! P:  %a = load %ptr        B:  %b = load %ptr      (redundant)
//!     ...                     ...
//! ```
//!
//! This pattern shows up in conditional reductions like
//! `if (arr[i] > 0) s += arr[i]`, where the frontend emits one load for the
//! condition and a second load in the taken arm. Forwarding the value removes
//! a per-iteration load and, because the arm becomes load-free, lets
//! if-conversion turn the diamond into a branchless `select`.
//!
//! Safety: we require B to have a single predecessor (so the earlier load is
//! guaranteed to have executed), identical pointer value and load type, and no
//! memory clobber (store/call/memcpy/atomic/fence) between the two loads.
//!
//! MULTI-DEF GUARD (gvn/copy_prop class): both the predecessor load's dest
//! and its pointer must be single-definition. A post-phi coalescing web can
//! otherwise change the value or address between the two loads. Entry blocks,
//! self edges and asm-goto predecessors need separate control-flow proof and
//! are conservatively excluded.
//!
//! Use the centralized memory-write classification rather than a partial
//! opcode list: inline asm, variadic state, atomics and memory-writing
//! intrinsics can invalidate the loaded value just as ordinary stores can.

use crate::common::types::AddressSpace;
use crate::ir::analysis;
use crate::ir::reexports::{Instruction, IrFunction, Operand, Value};

fn is_memory_barrier(inst: &Instruction) -> bool {
    inst.may_write_memory()
        || matches!(
            inst,
            Instruction::Fence { .. }
                | Instruction::AtomicLoad { .. }
                | Instruction::DynAlloca { .. }
                | Instruction::StackRestore { .. }
        )
}

/// Run redundant-load forwarding on a function. Returns the number of loads
/// replaced by copies.
pub(crate) fn run(func: &mut IrFunction) -> usize {
    // Map label -> block index for predecessor lookups.
    let label_to_idx = analysis::build_label_map(func);
    let (preds, _succs) = analysis::build_cfg(func, &label_to_idx);
    let multi_def = super::gvn::find_multi_def_values(func);

    // dest value of a load -> (ptr value id, type) for loads that end a block
    // with no trailing memory clobber. Built per-block below.
    let mut rewrites: Vec<(usize, usize, Value, Value)> = Vec::new(); // (block, inst, dest, src)

    for (bi, block) in func.blocks.iter().enumerate() {
        // The function entry has an implicit caller edge, even if the CFG
        // also has one backedge. A self edge cannot provide a dominating load.
        let pred_row = preds.row(bi);
        if bi == 0 || pred_row.len() != 1 {
            continue;
        }
        let pred_idx = pred_row[0] as usize;
        if pred_idx == bi {
            continue;
        }
        let pred_block = &func.blocks[pred_idx];
        // asm-goto can transfer to B before a later load in its block. The
        // predecessor's terminator alone does not prove that load executed.
        if pred_block.instructions.iter().any(|inst| {
            matches!(inst, Instruction::InlineAsm { goto_labels, .. } if !goto_labels.is_empty())
        }) {
            continue;
        }

        // Find the last Load in the predecessor with no memory clobber after it.
        // Scan backwards from the end.
        let mut pred_load: Option<(u32, u32, crate::common::types::IrType)> = None; // (ptr id, dest id, ty)
        for inst in pred_block.instructions.iter().rev() {
            match inst {
                Instruction::Load {
                    dest,
                    ptr,
                    ty,
                    seg_override,
                    volatile,
                    ..
                } => {
                    if *seg_override == AddressSpace::Default
                        && !*volatile
                        && !ty.is_float()
                        && !ty.is_128bit()
                        && !ty.is_long_double()
                    {
                        pred_load = Some((ptr.0, dest.0, *ty));
                    }
                    break;
                }
                // A memory clobber after the load means the value may be stale.
                _ if is_memory_barrier(inst) => break,
                _ => {}
            }
        }
        let Some((pred_ptr, pred_dest, pred_ty)) = pred_load else {
            continue;
        };

        // Find the first Load in this block with no memory clobber before it.
        for (ii, inst) in block.instructions.iter().enumerate() {
            match inst {
                Instruction::Load {
                    dest,
                    ptr,
                    ty,
                    seg_override,
                    volatile,
                    ..
                } => {
                    if *seg_override == AddressSpace::Default
                        && !*volatile
                        && ptr.0 == pred_ptr
                        && *ty == pred_ty
                        && dest.0 != pred_dest
                        && !multi_def.contains(&pred_dest)
                        && !multi_def.contains(&pred_ptr)
                    {
                        rewrites.push((bi, ii, *dest, Value(pred_dest)));
                    }
                    break; // Only the first load is safe to forward.
                }
                _ if is_memory_barrier(inst) => break,
                _ => {}
            }
        }
    }

    let count = rewrites.len();
    for (bi, ii, dest, src) in rewrites {
        func.blocks[bi].instructions[ii] = Instruction::Copy {
            dest,
            src: Operand::Value(src),
        };
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::reexports::{AtomicOrdering, BasicBlock, BlockId, IntrinsicOp, Terminator};

    fn load(dest: u32) -> Instruction {
        Instruction::Load {
            dest: Value(dest),
            ptr: Value(0),
            ty: IrType::U32,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }

    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: vec![],
        }
    }

    fn fixture(pred_tail: Vec<Instruction>, succ_head: Vec<Instruction>) -> IrFunction {
        let mut f = IrFunction::new("load_forward".into(), IrType::U32, vec![], false);
        let mut pred = vec![
            Instruction::ParamRef {
                dest: Value(0),
                param_idx: 0,
                ty: IrType::Ptr,
            },
            load(1),
        ];
        pred.extend(pred_tail);
        let mut succ = succ_head;
        succ.push(load(2));
        f.blocks = vec![
            block(0, pred, Terminator::Branch(BlockId(1))),
            block(1, succ, Terminator::Return(Some(Operand::Value(Value(2))))),
        ];
        f
    }

    fn asm(goto_labels: Vec<(String, BlockId)>) -> Instruction {
        Instruction::InlineAsm {
            template: String::new(),
            outputs: vec![],
            inputs: vec![],
            clobbers: vec!["memory".into()],
            operand_types: vec![],
            goto_labels,
            input_symbols: vec![],
            seg_overrides: vec![],
        }
    }

    #[test]
    fn forwards_across_a_clean_single_predecessor() {
        let mut f = fixture(vec![], vec![]);
        assert_eq!(run(&mut f), 1);
        assert!(matches!(
            f.blocks[1].instructions[0],
            Instruction::Copy {
                dest: Value(2),
                src: Operand::Value(Value(1))
            }
        ));
    }

    #[test]
    fn refuses_all_memory_and_ordering_barriers_on_either_side() {
        let barriers = vec![
            Instruction::AtomicInc {
                ptr: Operand::Value(Value(0)),
                offset: 0,
                ty: IrType::U32,
                ordering: AtomicOrdering::Relaxed,
            },
            Instruction::VaStart {
                va_list_ptr: Value(0),
            },
            Instruction::Intrinsic {
                dest: None,
                op: IntrinsicOp::VecStoreI32x4,
                dest_ptr: None,
                args: vec![Operand::Value(Value(3)), Operand::Value(Value(0))],
            },
            asm(vec![]),
            Instruction::Fence {
                ordering: AtomicOrdering::SeqCst,
            },
            Instruction::AtomicLoad {
                dest: Value(4),
                ptr: Operand::Value(Value(0)),
                ty: IrType::U32,
                ordering: AtomicOrdering::Acquire,
            },
            Instruction::StackRestore { ptr: Value(0) },
        ];
        for barrier in barriers {
            for before_edge in [true, false] {
                let mut f = if before_edge {
                    fixture(vec![barrier.clone()], vec![])
                } else {
                    fixture(vec![], vec![barrier.clone()])
                };
                assert_eq!(
                    run(&mut f),
                    0,
                    "barrier {barrier:?}, predecessor={before_edge}"
                );
                assert!(matches!(
                    f.blocks[1].instructions.last(),
                    Some(Instruction::Load { .. })
                ));
            }
        }
    }

    #[test]
    fn refuses_redefined_pointer_or_loaded_value() {
        for dest in [0, 1] {
            for before_edge in [true, false] {
                let redefine = Instruction::Copy {
                    dest: Value(dest),
                    src: Operand::Value(Value(3)),
                };
                let mut f = if before_edge {
                    fixture(vec![redefine], vec![])
                } else {
                    fixture(vec![], vec![redefine])
                };
                assert_eq!(run(&mut f), 0, "value {dest}, predecessor={before_edge}");
            }
        }
    }

    #[test]
    fn asm_goto_may_enter_before_the_predecessor_load() {
        let mut f = fixture(vec![], vec![]);
        f.blocks[0]
            .instructions
            .insert(1, asm(vec![("out".into(), BlockId(1))]));
        f.blocks[0].terminator = Terminator::Return(None);
        assert_eq!(run(&mut f), 0);
        assert!(matches!(
            f.blocks[1].instructions[0],
            Instruction::Load { .. }
        ));
    }

    #[test]
    fn entry_and_self_edges_do_not_supply_a_dominating_load() {
        let mut f = IrFunction::new("entry".into(), IrType::U32, vec![], false);
        f.blocks = vec![
            block(0, vec![load(2)], Terminator::Branch(BlockId(1))),
            block(
                1,
                vec![
                    Instruction::Fence {
                        ordering: AtomicOrdering::SeqCst,
                    },
                    load(1),
                ],
                Terminator::Branch(BlockId(0)),
            ),
        ];
        assert_eq!(run(&mut f), 0);
        assert!(matches!(
            f.blocks[0].instructions[0],
            Instruction::Load { .. }
        ));

        let mut f = IrFunction::new("self_loop".into(), IrType::U32, vec![], false);
        f.blocks = vec![
            block(0, vec![], Terminator::Return(None)),
            block(1, vec![load(2), load(1)], Terminator::Branch(BlockId(1))),
        ];
        assert_eq!(run(&mut f), 0);
        assert!(matches!(
            f.blocks[1].instructions[0],
            Instruction::Load { .. }
        ));
    }
}
