//! Fold phi-only relay blocks into their successor right before phi
//! elimination.
//!
//! An if/else at the end of a loop body leaves a latch that does nothing but
//! merge values for the header:
//!
//! ```text
//! P1: ... ; br J             P2: ... ; br J
//! J:  %x = phi [a1, P1], [a2, P2]       ; phis only
//!     br H
//! H:  %h = phi [init, E], [%x, J]
//! ```
//!
//! Phi elimination lowers that to copies into `%x` at the end of each `Pi`,
//! a copy `%x -> %h` plus a `jmp` in `J`.  Worse, two values live across the
//! relay (the old and new instance of a loop-carried pointer) cannot share a
//! register on both of its edges, so the allocator pays copies on both sides.
//! Folded, each `Pi` feeds `H` directly and every edge gets one parallel copy
//! (`%h = phi [init, E], [a1, P1], [a2, P2]`), the shape GCC's out-of-SSA
//! sees after its jump threader removed the relay.
//!
//! It runs this late because the single-latch shape is what the loop passes
//! (rotation, LICM, header rematerialisation) are tuned for. After it, only
//! the copy placement and machine-level loop inversion see the latches, and
//! `loop_invert` bottom-tests every `Branch(H)` latch on its own -- each former
//! relay predecessor gets its own copy of the exit test instead of a `jmp` to
//! the top test (before that generalization, folding a relay silently turned
//! a rotated loop back into a top-tested one).
//!
//! Conditions (all fail-closed):
//! * `J` holds only phis and ends in `Branch(H)` with `H != J`.
//! * Each `J` phi is used only as the `J` arm of `H` phis.
//! * Every predecessor of `J` ends in a plain `Branch(J)` (copies placed at
//!   its end then belong to exactly this edge), is not `J`, and is not
//!   already a predecessor of `H` (that would create two arms for one block).
//! * Every `H` phi has exactly one arm from `J`.  An arm value not defined
//!   in `J` is defined above it, so it dominates every predecessor of `J`.
//! * The copies added (predecessors x `H` phis) stay within
//!   [`MAX_FOLDED_ARMS`].
//!
//! Disable with `CCC_DISABLE_PASSES=phi_relay_fold`.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::analysis;
use crate::ir::reexports::{BlockId, Instruction, IrFunction, Operand, Terminator};

/// Upper bound on phi arms one fold may create, so a wide switch-like relay
/// does not trade one copy block for a quadratic copy fan-out.
const MAX_FOLDED_ARMS: usize = 32;

/// Number of uses of every value outside phi arms, plus the phi arms
/// (consumer block index, from-label) that read it.
struct Uses {
    plain: FxHashMap<u32, usize>,
    arms: FxHashMap<u32, Vec<(usize, BlockId)>>,
}

fn collect_uses(func: &IrFunction) -> Uses {
    let mut plain: FxHashMap<u32, usize> = FxHashMap::default();
    let mut arms: FxHashMap<u32, Vec<(usize, BlockId)>> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                for (op, from) in incoming {
                    if let Operand::Value(v) = op {
                        arms.entry(v.0).or_default().push((bi, *from));
                    }
                }
            } else {
                inst.for_each_used_value(|v| *plain.entry(v).or_default() += 1);
            }
        }
        block
            .terminator
            .for_each_used_value(|v| *plain.entry(v).or_default() += 1);
    }
    Uses { plain, arms }
}

/// Fold every eligible relay of `func`; returns the number folded.
pub(crate) fn run(func: &mut IrFunction) -> usize {
    let mut folded = 0;
    // Each fold deletes a block and rewires edges; recompute from scratch
    // (relays are rare and functions small enough that this is cheap).
    while let Some(plan) = find_relay(func) {
        apply(func, plan);
        folded += 1;
    }
    folded
}

struct Plan {
    relay: usize,
    header: usize,
    preds: Vec<usize>,
}

fn find_relay(func: &IrFunction) -> Option<Plan> {
    let label_to_idx = analysis::build_label_map(func);
    let (preds, _) = analysis::build_cfg(func, &label_to_idx);
    let uses = collect_uses(func);
    for (ji, j) in func.blocks.iter().enumerate() {
        let Terminator::Branch(h_label) = j.terminator else {
            continue;
        };
        if h_label == j.label
            || j.instructions.is_empty()
            || !j
                .instructions
                .iter()
                .all(|i| matches!(i, Instruction::Phi { .. }))
        {
            continue;
        }
        let Some(&hi) = label_to_idx.get(&h_label) else {
            continue;
        };
        let jpreds: Vec<usize> = preds.row(ji).iter().map(|&p| p as usize).collect();
        let hpreds: FxHashSet<usize> = preds.row(hi).iter().map(|&p| p as usize).collect();
        let distinct: FxHashSet<usize> = jpreds.iter().copied().collect();
        if jpreds.is_empty()
            || distinct.len() != jpreds.len()
            || jpreds.iter().any(|&p| {
                p == ji
                    || hpreds.contains(&p)
                    || !matches!(func.blocks[p].terminator, Terminator::Branch(t) if t == j.label)
            })
        {
            continue;
        }
        let h_phis = func.blocks[hi]
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::Phi { .. }))
            .count();
        if h_phis.saturating_mul(jpreds.len()) > MAX_FOLDED_ARMS {
            continue;
        }
        let arms_ok = func.blocks[hi].instructions.iter().all(|i| match i {
            Instruction::Phi { incoming, .. } => {
                incoming.iter().filter(|(_, from)| *from == j.label).count() == 1
            }
            _ => true,
        });
        let jphis_ok = j.instructions.iter().all(|i| {
            let Some(d) = i.dest() else {
                return false;
            };
            uses.plain.get(&d.0).copied().unwrap_or(0) == 0
                && uses
                    .arms
                    .get(&d.0)
                    .is_none_or(|a| a.iter().all(|&(bi, from)| bi == hi && from == j.label))
        });
        // Every J phi needs an arm for each predecessor, exactly once.
        let jarms_ok = j.instructions.iter().all(|i| match i {
            Instruction::Phi { incoming, .. } => jpreds.iter().all(|&p| {
                let l = func.blocks[p].label;
                incoming.iter().filter(|(_, from)| *from == l).count() == 1
            }),
            _ => false,
        });
        if arms_ok && jphis_ok && jarms_ok {
            return Some(Plan {
                relay: ji,
                header: hi,
                preds: jpreds,
            });
        }
    }
    None
}

fn apply(func: &mut IrFunction, plan: Plan) {
    let j_label = func.blocks[plan.relay].label;
    let h_label = func.blocks[plan.header].label;
    // J phi dest -> its arms by predecessor label.
    let mut jphi: FxHashMap<u32, Vec<(Operand, BlockId)>> = FxHashMap::default();
    for inst in &func.blocks[plan.relay].instructions {
        if let Instruction::Phi { dest, incoming, .. } = inst {
            jphi.insert(dest.0, incoming.clone());
        }
    }
    let pred_labels: Vec<BlockId> = plan.preds.iter().map(|&p| func.blocks[p].label).collect();
    for inst in &mut func.blocks[plan.header].instructions {
        let Instruction::Phi { incoming, .. } = inst else {
            continue;
        };
        let Some(pos) = incoming.iter().position(|(_, from)| *from == j_label) else {
            continue;
        };
        let (via, _) = incoming.remove(pos);
        for &pl in &pred_labels {
            let value = match via {
                Operand::Value(v) => jphi.get(&v.0).map_or(via, |arms| {
                    arms.iter()
                        .find(|(_, from)| *from == pl)
                        .map_or(via, |(op, _)| *op)
                }),
                Operand::Const(_) => via,
            };
            incoming.push((value, pl));
        }
    }
    for &p in &plan.preds {
        func.blocks[p].terminator = Terminator::Branch(h_label);
    }
    func.blocks.remove(plan.relay);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::reexports::{BasicBlock, Value};

    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: Vec::new(),
        }
    }

    fn phi(dest: u32, arms: &[(u32, u32)]) -> Instruction {
        Instruction::Phi {
            dest: Value(dest),
            ty: IrType::I64,
            incoming: arms
                .iter()
                .map(|&(v, b)| (Operand::Value(Value(v)), BlockId(b)))
                .collect(),
        }
    }

    fn param(dest: u32) -> Instruction {
        Instruction::ParamRef {
            dest: Value(dest),
            param_idx: dest as usize,
            ty: IrType::I64,
        }
    }

    /// B0 -> H1; H1: h = phi[v0 @0, x @4], cbr v0 ? B2 : B5; B2 cbr v1 ? B3 : B6;
    /// B3 -> J4 (a = v1), B6 -> J4 (a = v0); J4: x = phi -> H1; B5 returns h.
    fn latch() -> IrFunction {
        let mut f = IrFunction::new("t".into(), IrType::I64, vec![], false);
        f.blocks = vec![
            block(0, vec![param(0), param(1)], Terminator::Branch(BlockId(1))),
            block(
                1,
                vec![phi(10, &[(0, 0), (11, 4)])],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(10)),
                    true_label: BlockId(2),
                    false_label: BlockId(5),
                },
            ),
            block(
                2,
                vec![],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(3),
                    false_label: BlockId(6),
                },
            ),
            block(3, vec![], Terminator::Branch(BlockId(4))),
            block(6, vec![], Terminator::Branch(BlockId(4))),
            block(
                4,
                vec![phi(11, &[(1, 3), (0, 6)])],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                5,
                vec![],
                Terminator::Return(Some(Operand::Value(Value(10)))),
            ),
        ];
        f
    }

    #[test]
    fn relay_arms_move_onto_the_header_phi() {
        let mut f = latch();
        assert_eq!(run(&mut f), 1);
        assert!(f.blocks.iter().all(|b| b.label != BlockId(4)));
        let Instruction::Phi { incoming, .. } = &f.blocks[1].instructions[0] else {
            panic!("header phi expected");
        };
        assert_eq!(
            incoming,
            &vec![
                (Operand::Value(Value(0)), BlockId(0)),
                (Operand::Value(Value(1)), BlockId(3)),
                (Operand::Value(Value(0)), BlockId(6)),
            ]
        );
        for b in [3, 6] {
            let blk = f.blocks.iter().find(|x| x.label == BlockId(b));
            assert!(blk.is_some_and(|x| matches!(x.terminator, Terminator::Branch(BlockId(1)))));
        }
    }

    #[test]
    fn a_relay_phi_used_elsewhere_stays() {
        let mut f = latch();
        // B5 also returns the relay's phi: it is not header-only.
        f.blocks[6].terminator = Terminator::Return(Some(Operand::Value(Value(11))));
        assert_eq!(run(&mut f), 0);
    }

    #[test]
    fn a_conditional_or_shared_predecessor_blocks_the_fold() {
        // B3 conditionally reaches the relay: its copies would run on both
        // edges.
        let mut f = latch();
        f.blocks[3].terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(1)),
            true_label: BlockId(4),
            false_label: BlockId(5),
        };
        f.blocks[5].instructions[0] = phi(11, &[(1, 3), (0, 6)]);
        assert_eq!(run(&mut f), 0);
        // B0 feeds both the relay and the header: two arms for one block.
        let mut f = latch();
        f.blocks[0].terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(0)),
            true_label: BlockId(1),
            false_label: BlockId(4),
        };
        f.blocks[5].instructions[0] = phi(11, &[(1, 3), (0, 6), (0, 0)]);
        assert_eq!(run(&mut f), 0);
    }

    #[test]
    fn non_phi_relays_and_self_loops_are_ignored() {
        let mut f = latch();
        f.blocks[5].instructions.push(Instruction::Copy {
            dest: Value(12),
            src: Operand::Value(Value(11)),
        });
        assert_eq!(run(&mut f), 0);
        let mut f = latch();
        f.blocks[5].terminator = Terminator::Branch(BlockId(4));
        assert_eq!(run(&mut f), 0);
    }
}
