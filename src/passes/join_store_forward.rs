//! Forward an exact pointer store through a two-way control-flow join.
//!
//! When both incoming blocks store a pointer value to the same pointer-sized
//! location, a non-volatile reload at the start of the join can select those
//! values with a phi instead of depending on a store-to-load round trip. This
//! is useful for pointer-chasing code, but the proof is independent of any
//! particular source pattern.
//!
//! The proof is deliberately narrow: exactly two distinct predecessors must
//! branch directly to the join, each must execute an exact same-address,
//! same-type store with no later write or ordering barrier, and the join must
//! not write or cross an ordering barrier before the reload. Address, source
//! and destination values must not have multiple definitions, and incoming
//! sources must be pointer-typed. No alias-disambiguation guess is needed.
//!
//! Entry/self edges, asm-goto predecessors, non-default address spaces,
//! volatile or atomic operations, and pointers defined in the join are
//! refused. Existing phi nodes remain contiguous, and the new phi names every
//! predecessor exactly once. Disable independently with
//! `CCC_DISABLE_PASSES=join_store_forward`.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::analysis;
use crate::ir::reexports::{BlockId, Instruction, IrFunction, Operand, Terminator, Value};

fn barrier(inst: &Instruction) -> bool {
    inst.may_write_memory()
        || matches!(
            inst,
            Instruction::Fence { .. }
                | Instruction::AtomicLoad { .. }
                | Instruction::DynAlloca { .. }
                | Instruction::StackRestore { .. }
                | Instruction::Load { volatile: true, .. }
        )
}

/// Follow only single-definition copies, then require an explicit pointer type.
fn pointer_value(
    mut v: Value,
    defs: &FxHashMap<u32, (usize, usize, &Instruction)>,
    multi_def: &FxHashSet<u32>,
) -> bool {
    for _ in 0..32 {
        if multi_def.contains(&v.0) {
            return false;
        }
        let Some((_, _, inst)) = defs.get(&v.0) else {
            return false;
        };
        match inst {
            Instruction::Copy {
                src: Operand::Value(next),
                ..
            } => v = *next,
            _ => return inst.result_type() == Some(IrType::Ptr),
        }
    }
    false
}

struct Rewrite {
    block: usize,
    load: usize,
    dest: Value,
    incoming: Vec<(Operand, BlockId)>,
}

/// Replace proven join reloads with pointer phis. Returns the number replaced.
pub(crate) fn run(func: &mut IrFunction) -> usize {
    // Most functions have no pointer store. Avoid CFG and def-use allocation
    // for them, including large scalar or floating-point kernels.
    if !func
        .blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .any(|inst| {
            matches!(
                inst,
                Instruction::Store {
                    ty: IrType::Ptr,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                    ..
                }
            )
        })
    {
        return 0;
    }
    let label_to_idx = analysis::build_label_map(func);
    let (preds, _) = analysis::build_cfg(func, &label_to_idx);
    let multi_def = super::gvn::find_multi_def_values(func);
    let mut defs = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Some(dest) = inst.dest() {
                defs.insert(dest.0, (bi, ii, inst));
            }
        }
    }
    let mut rewrites = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        let pred = preds.row(bi);
        if bi == 0 || pred.len() != 2 || pred[0] == pred[1] {
            continue;
        }
        let mut candidate = None;
        for (ii, inst) in block.instructions.iter().enumerate() {
            match inst {
                Instruction::Load {
                    dest,
                    ptr,
                    ty: IrType::Ptr,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                } => {
                    candidate = Some((ii, *dest, *ptr));
                    break;
                }
                Instruction::Load { .. } => break,
                _ if barrier(inst) => break,
                _ => {}
            }
        }
        let Some((load, dest, ptr)) = candidate else {
            continue;
        };
        if dest == ptr
            || multi_def.contains(&dest.0)
            || multi_def.contains(&ptr.0)
            || !matches!(defs.get(&ptr.0), Some((db, _, _)) if *db != bi)
        {
            continue;
        }
        let mut incoming = Vec::with_capacity(2);
        for &pred_idx in pred {
            let pred_idx = pred_idx as usize;
            if pred_idx == bi {
                break;
            }
            let predecessor = &func.blocks[pred_idx];
            if !matches!(predecessor.terminator, Terminator::Branch(target) if target == block.label)
                || predecessor.instructions.iter().any(|inst| {
                    matches!(inst, Instruction::InlineAsm { goto_labels, .. } if !goto_labels.is_empty())
                })
            {
                break;
            }
            let mut source = None;
            for (store_idx, inst) in predecessor.instructions.iter().enumerate().rev() {
                match inst {
                    Instruction::Store {
                        val: Operand::Value(value),
                        ptr: store_ptr,
                        ty: IrType::Ptr,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    } if *store_ptr == ptr
                        && *value != dest
                        && pointer_value(*value, &defs, &multi_def)
                        && [ptr, *value].iter().all(|v| {
                            matches!(
                                defs.get(&v.0),
                                Some((db, di, _)) if *db != pred_idx || *di < store_idx
                            )
                        }) =>
                    {
                        source = Some(*value);
                        break;
                    }
                    _ if barrier(inst) => break,
                    _ => {}
                }
            }
            let Some(source) = source else {
                break;
            };
            incoming.push((Operand::Value(source), predecessor.label));
        }
        if incoming.len() == 2 {
            rewrites.push(Rewrite {
                block: bi,
                load,
                dest,
                incoming,
            });
        }
    }
    let count = rewrites.len();
    for rewrite in rewrites {
        let block = &mut func.blocks[rewrite.block];
        let keep_spans =
            !block.source_spans.is_empty() && block.source_spans.len() == block.instructions.len();
        let span = keep_spans.then(|| block.source_spans.remove(rewrite.load));
        block.instructions.remove(rewrite.load);
        let phi_end = block
            .instructions
            .iter()
            .take_while(|inst| matches!(inst, Instruction::Phi { .. }))
            .count();
        block.instructions.insert(
            phi_end,
            Instruction::Phi {
                dest: rewrite.dest,
                ty: IrType::Ptr,
                incoming: rewrite.incoming,
            },
        );
        if let Some(span) = span {
            block.source_spans.insert(phi_end, span);
        } else {
            block.source_spans.clear();
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::source::Span;
    use crate::ir::reexports::{AtomicOrdering, BasicBlock, IrConst};

    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: vec![],
        }
    }

    fn load(dest: u32, ptr: u32) -> Instruction {
        Instruction::Load {
            dest: Value(dest),
            ptr: Value(ptr),
            ty: IrType::Ptr,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }

    fn store(ptr: u32, value: u32) -> Instruction {
        Instruction::Store {
            val: Operand::Value(Value(value)),
            ptr: Value(ptr),
            ty: IrType::Ptr,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }

    fn fixture() -> IrFunction {
        let mut func = IrFunction::new("join".into(), IrType::Ptr, vec![], false);
        let params = (0..4)
            .map(|n| Instruction::ParamRef {
                dest: Value(n),
                param_idx: n as usize,
                ty: if n == 3 { IrType::I32 } else { IrType::Ptr },
            })
            .collect();
        func.blocks = vec![
            block(
                0,
                params,
                Terminator::CondBranch {
                    cond: Operand::Value(Value(3)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            block(
                1,
                vec![store(0, 1), load(4, 1)],
                Terminator::Branch(BlockId(3)),
            ),
            block(
                2,
                vec![store(0, 2), load(5, 2)],
                Terminator::Branch(BlockId(3)),
            ),
            block(
                3,
                vec![load(6, 0)],
                Terminator::Return(Some(Operand::Value(Value(6)))),
            ),
        ];
        func.next_value_id = 7;
        func
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
    fn forwards_both_stored_pointers_and_keeps_phi_prefix_and_spans() {
        let mut f = fixture();
        f.blocks[3].instructions.insert(
            0,
            Instruction::Phi {
                dest: Value(7),
                ty: IrType::Ptr,
                incoming: vec![
                    (Operand::Value(Value(1)), BlockId(1)),
                    (Operand::Value(Value(2)), BlockId(2)),
                ],
            },
        );
        f.next_value_id = 8;
        f.blocks[3].source_spans = vec![Span::dummy(); f.blocks[3].instructions.len()];
        assert_eq!(run(&mut f), 1);
        assert!(matches!(
            f.blocks[3].instructions[0],
            Instruction::Phi { dest: Value(7), .. }
        ));
        assert!(matches!(
            &f.blocks[3].instructions[1],
            Instruction::Phi { dest: Value(6), ty: IrType::Ptr, incoming }
                if incoming.as_slice() == &[
                    (Operand::Value(Value(1)), BlockId(1)),
                    (Operand::Value(Value(2)), BlockId(2))
                ]
        ));
        assert_eq!(
            f.blocks[3].source_spans.len(),
            f.blocks[3].instructions.len()
        );
        assert_eq!(run(&mut f), 0);
    }

    #[test]
    fn forwards_a_single_definition_pointer_copy() {
        let mut f = fixture();
        f.blocks[1].instructions.insert(
            0,
            Instruction::Copy {
                dest: Value(7),
                src: Operand::Value(Value(1)),
            },
        );
        f.blocks[1].instructions[1] = store(0, 7);
        f.next_value_id = 8;
        assert_eq!(run(&mut f), 1);
        assert!(matches!(
            &f.blocks[3].instructions[0],
            Instruction::Phi { incoming, .. }
                if incoming[0] == (Operand::Value(Value(7)), BlockId(1))
        ));
    }

    #[test]
    fn any_intervening_memory_or_ordering_barrier_prevents_forwarding() {
        let barriers = vec![
            store(1, 2),
            Instruction::Fence {
                ordering: AtomicOrdering::SeqCst,
            },
            Instruction::AtomicLoad {
                dest: Value(8),
                ptr: Operand::Value(Value(0)),
                ty: IrType::Ptr,
                ordering: AtomicOrdering::Acquire,
            },
            Instruction::StackRestore { ptr: Value(0) },
            Instruction::Load {
                dest: Value(8),
                ptr: Value(0),
                ty: IrType::Ptr,
                seg_override: AddressSpace::Default,
                volatile: true,
            },
            asm(vec![]),
        ];
        for barrier in barriers {
            for join_side in [false, true] {
                let mut f = fixture();
                if join_side {
                    f.blocks[3].instructions.insert(0, barrier.clone());
                } else {
                    f.blocks[1].instructions.push(barrier.clone());
                }
                assert_eq!(run(&mut f), 0, "{barrier:?}, join={join_side}");
            }
        }
    }

    #[test]
    fn refuses_partial_or_inexact_store_and_load_shapes() {
        for mode in 0..7 {
            let mut f = fixture();
            match mode {
                0 => {
                    f.blocks[1].instructions.remove(0);
                }
                1 => f.blocks[1].instructions[0] = store(1, 1),
                2 => {
                    if let Instruction::Store { volatile, .. } = &mut f.blocks[1].instructions[0] {
                        *volatile = true;
                    }
                }
                3 => {
                    if let Instruction::Store { seg_override, .. } =
                        &mut f.blocks[1].instructions[0]
                    {
                        *seg_override = AddressSpace::SegFs;
                    }
                }
                4 => {
                    if let Instruction::Store { ty, .. } = &mut f.blocks[1].instructions[0] {
                        *ty = IrType::U32;
                    }
                }
                5 => {
                    if let Instruction::Load { volatile, .. } = &mut f.blocks[3].instructions[0] {
                        *volatile = true;
                    }
                }
                _ => {
                    if let Instruction::Load { seg_override, .. } = &mut f.blocks[3].instructions[0]
                    {
                        *seg_override = AddressSpace::SegGs;
                    }
                }
            }
            assert_eq!(run(&mut f), 0, "mode {mode}");
        }
    }

    #[test]
    fn refuses_non_ssa_or_untyped_sources_and_early_edges() {
        for mode in 0..9 {
            let mut f = fixture();
            match mode {
                0 => f.blocks[1].instructions.push(Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Value(Value(1)),
                }),
                1 => f.blocks[1].instructions.push(Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(2)),
                }),
                2 => f.blocks[3].instructions.push(Instruction::Copy {
                    dest: Value(6),
                    src: Operand::Value(Value(2)),
                }),
                3 => {
                    f.blocks[1].instructions[0] = Instruction::Store {
                        val: Operand::Const(IrConst::I64(0)),
                        ptr: Value(0),
                        ty: IrType::Ptr,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    }
                }
                4 => f.blocks[1].instructions[0] = store(0, 3),
                5 => f.blocks[1]
                    .instructions
                    .insert(0, asm(vec![("out".into(), BlockId(3))])),
                6 => {
                    f.blocks[1].terminator = Terminator::CondBranch {
                        cond: Operand::Value(Value(3)),
                        true_label: BlockId(3),
                        false_label: BlockId(2),
                    }
                }
                7 => {
                    let pointer_def = f.blocks[0].instructions.remove(0);
                    f.blocks[3].instructions.insert(0, pointer_def);
                }
                _ => {
                    f.blocks[1].instructions[0] = store(0, 7);
                    f.blocks[1].instructions.push(Instruction::Copy {
                        dest: Value(7),
                        src: Operand::Value(Value(1)),
                    });
                }
            };
            assert_eq!(run(&mut f), 0, "mode {mode}");
        }
    }
}
