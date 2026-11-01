//! Phi-web coalescing: give non-interfering phi destinations one name before
//! phi elimination.
//!
//! A loop whose body splits into several phi-carrying blocks (a merge loop
//! after header tail duplication: header `H`, arm bodies `A` and `B`, each
//! with phis for `a`, `b` and `tail`) has one phi *web* per block and
//! variable.  Phi elimination lowers every web to its own multi-definition
//! variable and the register allocator then picks a register per web, so each
//! transfer between arms pays a full permutation of moves and the exit phis
//! (fed from every web) end up in callee-saved registers.  GCC's out-of-SSA
//! unions those webs and keeps `a`, `b` and `tail` in one register each.
//!
//! This pass does the union explicitly.  A phi destination `d` is merged with
//! an incoming value `s` that is itself a phi destination when no member of
//! `d`'s class interferes with any member of `s`'s class; each class is then
//! renamed to one leader, so the copies between members become self-copies
//! that phi elimination already drops, and the copies that remain (the
//! genuine `tail = a; a = next` shifts) go through its parallel-copy planner.
//!
//! Pairs stay inside one innermost loop (or end in a loop-free join); see
//! the comment at the filter.  Pairs are unioned pass-through first (incoming phi's block dominates the
//! phi's block), role shifts last; see the comment at the sort.
//!
//! # Interference
//!
//! Only phi destinations are classed, and a phi defines its value at the entry
//! of its block, which makes the strict-SSA test (Budimlić et al.) exact and
//! cheap: two phi values of the same block always interfere; otherwise, if
//! `def(u)` dominates `def(v)`, they interfere iff `u` is live into `v`'s
//! block; if neither definition dominates the other they cannot interfere.
//! Liveness is computed per candidate value by a backward walk from its uses
//! (a phi use is a use at the end of the incoming edge's predecessor).
//!
//! # Why the result is still lowered correctly
//!
//! Renaming widens where a variable is observable: a copy for one edge now
//! writes a name that other paths read.  Phi elimination places every copy on
//! its own edge (splitting critical edges into trampolines) and validates the
//! self-loop hoist against the renamed IR, so that stays sound.  The two edge
//! kinds it cannot split -- `IndirectBranch` and `asm goto` predecessors --
//! make the pass refuse the function.
//!
//! The pass leaves the function out of SSA (several phis share a destination)
//! and must run immediately before phi elimination.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::analysis;
use crate::ir::instruction::{
    Instruction, Operand, Terminator, replace_values_in_inst_map, replace_values_in_terminator_map,
};
use crate::ir::reexports::IrFunction;

/// Largest class the pass builds; bounds the pairwise interference checks.
const MAX_CLASS: usize = 32;

/// Innermost natural loop (by header, smallest body) of every block.
fn innermost_loops(cfg: &analysis::CfgAnalysis) -> Vec<Option<usize>> {
    use crate::passes::loop_analysis::{find_natural_loops, merge_loops_by_header};
    let loops = merge_loops_by_header(find_natural_loops(
        cfg.num_blocks,
        &cfg.preds,
        &cfg.succs,
        &cfg.idom,
    ));
    let mut inner: Vec<Option<usize>> = vec![None; cfg.num_blocks];
    for (li, l) in loops.iter().enumerate() {
        for &b in &l.body {
            let smaller = match inner[b] {
                Some(prev) => l.body.len() < loops[prev].body.len(),
                None => true,
            };
            if smaller {
                inner[b] = Some(li);
            }
        }
    }
    inner
}

/// Union non-interfering phi webs of `func` and rename each class to one
/// leader.  Returns the number of phi destinations renamed.
pub(crate) fn coalesce_phi_webs(func: &mut IrFunction) -> usize {
    let n = func.blocks.len();
    if n < 2 {
        return 0;
    }
    let unsplittable = func.blocks.iter().any(|b| {
        matches!(b.terminator, Terminator::IndirectBranch { .. })
            || b.instructions.iter().any(|i| {
                matches!(i, Instruction::InlineAsm { goto_labels, .. } if !goto_labels.is_empty())
            })
    });
    if unsplittable {
        return 0;
    }
    let cfg = analysis::CfgAnalysis::build(func);
    let labels = analysis::build_label_map(func);
    let reachable = |b: usize| b == 0 || cfg.idom[b] != usize::MAX;

    // Phi destinations and their blocks (a value with two definitions is not
    // SSA and is never classed).
    let mut phi_block: FxHashMap<u32, usize> = FxHashMap::default();
    let mut defs: FxHashMap<u32, u32> = FxHashMap::default();
    for (bi, b) in func.blocks.iter().enumerate() {
        for inst in &b.instructions {
            if let Some(d) = inst.dest() {
                *defs.entry(d.0).or_insert(0) += 1;
                if matches!(inst, Instruction::Phi { .. }) && reachable(bi) {
                    phi_block.insert(d.0, bi);
                }
            }
        }
    }
    phi_block.retain(|v, _| defs.get(v) == Some(&1));

    // Candidate pairs: phi destination <- incoming phi destination.
    let mut pairs: Vec<(u32, u32)> = Vec::new();
    for b in &func.blocks {
        for inst in &b.instructions {
            if let Instruction::Phi { dest, incoming, .. } = inst {
                if !phi_block.contains_key(&dest.0) {
                    continue;
                }
                for (op, _) in incoming {
                    if let Operand::Value(s) = op
                        && s.0 != dest.0
                        && phi_block.contains_key(&s.0)
                    {
                        pairs.push((dest.0, s.0));
                    }
                }
            }
        }
    }
    // Loop scope: union only within one innermost loop, or into a block
    // outside every loop (the exit joins of a merge loop).  A web crossing a
    // loop boundary (`r` threaded through an outer loop and two inner ones in
    // fannkuch) removes copies that run once per inner-loop trip, but hands
    // the allocator one long, use-heavy interval that wins a register over
    // the hot inner-loop value it used to leave alone (fannkuch +5.6% Ir with
    // `flips` spilled; percpu_ptr_walk +5.2% with the chain load no longer
    // landing in the phi's register).
    let innermost = innermost_loops(&cfg);
    pairs.retain(|&(d, s)| {
        let ld = innermost[phi_block[&d]];
        ld.is_none() || ld == innermost[phi_block[&s]]
    });
    if pairs.is_empty() {
        return 0;
    }
    // Greedy union order decides which copies survive.  A pair whose incoming
    // phi is defined in a block dominating the phi's block is a variable
    // flowing unchanged into a region (`B.b <- H.b`); a back-edge or sibling
    // pair is typically a role shift (`H.tail <- B.b`, "tail = node").
    // Unioning the pass-through pairs first keeps each variable in one class
    // and leaves only the inherent shift copies (stable within each group).
    pairs.sort_by_key(|&(d, s)| !analysis::dominates_idom(&cfg.idom, phi_block[&s], phi_block[&d]));

    // Use sites of every candidate: (block, is_phi_use_from_pred).
    let candidates: FxHashSet<u32> = pairs.iter().flat_map(|&(a, b)| [a, b]).collect();
    let mut use_blocks: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
    for (bi, b) in func.blocks.iter().enumerate() {
        if !reachable(bi) {
            continue;
        }
        for inst in &b.instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                for (op, from) in incoming {
                    if let Operand::Value(v) = op
                        && candidates.contains(&v.0)
                        && let Some(&p) = labels.get(from)
                    {
                        // Live out of `p`: model it as a use at the end of `p`.
                        use_blocks.entry(v.0).or_default().push(p);
                    }
                }
            } else {
                inst.for_each_used_value(|v| {
                    if candidates.contains(&v) {
                        use_blocks.entry(v).or_default().push(bi);
                    }
                });
            }
        }
        b.terminator.for_each_used_value(|v| {
            if candidates.contains(&v) {
                use_blocks.entry(v).or_default().push(bi);
            }
        });
    }

    // Live-in block set of a phi value: backward from its use blocks, never
    // entering its definition block (a phi is defined at the block entry).
    let mut live_in_cache: FxHashMap<u32, FxHashSet<usize>> = FxHashMap::default();
    let mut live_in = |v: u32| -> FxHashSet<usize> {
        if let Some(s) = live_in_cache.get(&v) {
            return s.clone();
        }
        let def = phi_block[&v];
        let mut set = FxHashSet::default();
        let mut work: Vec<usize> = use_blocks
            .get(&v)
            .map(|u| u.iter().copied().filter(|&b| b != def).collect())
            .unwrap_or_default();
        while let Some(b) = work.pop() {
            if !set.insert(b) {
                continue;
            }
            for &p in cfg.preds.row(b) {
                let p = p as usize;
                if p != def && reachable(p) && !set.contains(&p) {
                    work.push(p);
                }
            }
        }
        live_in_cache.insert(v, set.clone());
        set
    };
    let mut interferes = |u: u32, v: u32| -> bool {
        let (bu, bv) = (phi_block[&u], phi_block[&v]);
        if bu == bv {
            return true;
        }
        if analysis::dominates_idom(&cfg.idom, bu, bv) {
            live_in(u).contains(&bv)
        } else if analysis::dominates_idom(&cfg.idom, bv, bu) {
            live_in(v).contains(&bu)
        } else {
            false
        }
    };

    // Union-find over the candidates with explicit member lists.
    let mut class_of: FxHashMap<u32, u32> = candidates.iter().map(|&v| (v, v)).collect();
    let mut members: FxHashMap<u32, Vec<u32>> = candidates.iter().map(|&v| (v, vec![v])).collect();
    for (d, s) in pairs {
        let (cd, cs) = (class_of[&d], class_of[&s]);
        if cd == cs || members[&cd].len() + members[&cs].len() > MAX_CLASS {
            continue;
        }
        let (md, ms) = (members[&cd].clone(), members[&cs].clone());
        if md.iter().any(|&x| ms.iter().any(|&y| interferes(x, y))) {
            continue;
        }
        let (keep, gone) = if cd < cs { (cd, cs) } else { (cs, cd) };
        let moved = members.remove(&gone).unwrap_or_default();
        for &m in &moved {
            class_of.insert(m, keep);
        }
        if let Some(list) = members.get_mut(&keep) {
            list.extend(moved);
        }
    }

    // Rename every member to its class leader (the smallest value id).
    let rename: FxHashMap<u32, u32> = class_of.into_iter().filter(|&(v, c)| v != c).collect();
    if rename.is_empty() {
        return 0;
    }
    for b in &mut func.blocks {
        for inst in &mut b.instructions {
            replace_values_in_inst_map(inst, &rename);
            if let Instruction::Phi { dest, .. } = inst
                && let Some(&to) = rename.get(&dest.0)
            {
                dest.0 = to;
            }
        }
        replace_values_in_terminator_map(&mut b.terminator, &rename);
    }
    rename.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::instruction::{BasicBlock, BlockId, Value};
    use crate::ir::reexports::IrCmpOp;

    fn phi(dest: u32, arms: &[(u32, u32)]) -> Instruction {
        Instruction::Phi {
            dest: Value(dest),
            ty: IrType::Ptr,
            incoming: arms
                .iter()
                .map(|&(v, b)| (Operand::Value(Value(v)), BlockId(b)))
                .collect(),
        }
    }

    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            source_spans: Vec::new(),
            terminator,
        }
    }

    fn cbr(c: u32, t: u32, f: u32) -> Terminator {
        Terminator::CondBranch {
            cond: Operand::Value(Value(c)),
            true_label: BlockId(t),
            false_label: BlockId(f),
        }
    }

    fn func(blocks: Vec<BasicBlock>) -> IrFunction {
        let mut f = IrFunction::new("w".into(), IrType::Ptr, vec![], false);
        f.blocks = blocks;
        f
    }

    fn phi_dests(f: &IrFunction, label: u32) -> Vec<u32> {
        let b = f.blocks.iter().find(|b| b.label.0 == label);
        b.map(|b| {
            b.instructions
                .iter()
                .filter_map(|i| match i {
                    Instruction::Phi { dest, .. } => Some(dest.0),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
    }

    /// Header H (1) splits into arms A (2, self loop) and B (3, back to H).
    /// Within the outer loop the webs `a` = {10, 30} and `b` = {11, 31} each
    /// get one name; A's phis belong to the inner loop and keep theirs (the
    /// loop scope refuses cross-loop unions).
    #[test]
    fn the_webs_of_a_loop_share_one_name_but_not_with_an_inner_loop() {
        let ld = |d: u32, p: u32| Instruction::Load {
            dest: Value(d),
            ptr: Value(p),
            ty: IrType::Ptr,
            seg_override: crate::common::types::AddressSpace::Default,
            volatile: false,
        };
        let mut f = func(vec![
            block(
                0,
                vec![
                    Instruction::ParamRef {
                        dest: Value(1),
                        param_idx: 0,
                        ty: IrType::Ptr,
                    },
                    Instruction::ParamRef {
                        dest: Value(2),
                        param_idx: 1,
                        ty: IrType::Ptr,
                    },
                ],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![phi(10, &[(1, 0), (30, 3)]), phi(11, &[(2, 0), (32, 3)])],
                cbr(10, 2, 3),
            ),
            block(
                2,
                vec![
                    phi(20, &[(10, 1), (22, 2)]),
                    phi(21, &[(11, 1), (21, 2)]),
                    ld(22, 20),
                ],
                cbr(22, 2, 3),
            ),
            block(
                3,
                vec![
                    phi(30, &[(10, 1), (22, 2)]),
                    phi(31, &[(11, 1), (21, 2)]),
                    ld(32, 31),
                ],
                cbr(32, 1, 4),
            ),
            block(
                4,
                vec![],
                Terminator::Return(Some(Operand::Value(Value(30)))),
            ),
        ]);
        let n = coalesce_phi_webs(&mut f);
        assert_eq!(n, 2, "renamed {n}");
        assert_eq!(phi_dests(&f, 1), vec![10, 11]);
        assert_eq!(phi_dests(&f, 2), vec![20, 21], "inner-loop phis stay apart");
        assert_eq!(phi_dests(&f, 3), vec![10, 11]);
        // The header's `a` arm from B now names the shared variable.
        if let Instruction::Phi { incoming, .. } = &f.blocks[1].instructions[0] {
            assert!(matches!(incoming[1].0, Operand::Value(Value(10))));
        }
    }

    /// The merge loop's arm B: `H.t <- B.b` (tail = node, a back edge) comes
    /// before `B.b <- H.b` in program order; B.b must still join `b`'s web
    /// so the header and both arms agree on one name per variable.
    #[test]
    fn pass_through_pairs_win_over_role_shifts() {
        let ld = |d: u32, p: u32| Instruction::Load {
            dest: Value(d),
            ptr: Value(p),
            ty: IrType::Ptr,
            seg_override: crate::common::types::AddressSpace::Default,
            volatile: false,
        };
        let st = |v: u32, p: u32| Instruction::Store {
            val: Operand::Value(Value(v)),
            ptr: Value(p),
            ty: IrType::Ptr,
            seg_override: crate::common::types::AddressSpace::Default,
            volatile: false,
        };
        let param = |d: u32, i: usize| Instruction::ParamRef {
            dest: Value(d),
            param_idx: i,
            ty: IrType::Ptr,
        };
        // H(1): t = phi[p2 @0, bb @3]; b = phi[p1 @0, bn @3]; -> B
        // B(3): bb = phi[b @1]; tb = phi[t @1]; store bb -> tb; bn = load bb
        let mut f = func(vec![
            block(
                0,
                vec![param(1, 0), param(2, 1)],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![phi(10, &[(2, 0), (30, 3)]), phi(11, &[(1, 0), (32, 3)])],
                cbr(11, 3, 4),
            ),
            block(
                3,
                vec![
                    phi(30, &[(11, 1)]),
                    phi(31, &[(10, 1)]),
                    st(30, 31),
                    ld(32, 30),
                ],
                cbr(32, 1, 4),
            ),
            block(
                4,
                vec![],
                Terminator::Return(Some(Operand::Value(Value(1)))),
            ),
        ]);
        coalesce_phi_webs(&mut f);
        // b's web: {11, 30}; tail's web: {10, 31}.
        assert_eq!(phi_dests(&f, 3), vec![11, 10]);
    }

    /// `p = phi[x, q]` while `q` (a phi of a block `p` dominates) is still
    /// read after `p`'s block: they interfere and keep their own names.
    #[test]
    fn a_phi_live_into_its_partners_block_is_not_merged() {
        let mut f = func(vec![
            block(
                0,
                vec![Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 0,
                    ty: IrType::Ptr,
                }],
                Terminator::Branch(BlockId(1)),
            ),
            // H: p = phi[1, q]; branch to L
            block(
                1,
                vec![phi(10, &[(1, 0), (20, 2)])],
                Terminator::Branch(BlockId(2)),
            ),
            // L: q = phi[p]; reads p after q's definition -> interference.
            block(
                2,
                vec![
                    phi(20, &[(10, 1)]),
                    Instruction::Cmp {
                        dest: Value(21),
                        op: IrCmpOp::Eq,
                        lhs: Operand::Value(Value(10)),
                        rhs: Operand::Value(Value(20)),
                        ty: IrType::Ptr,
                    },
                ],
                cbr(21, 1, 3),
            ),
            block(
                3,
                vec![],
                Terminator::Return(Some(Operand::Value(Value(10)))),
            ),
        ]);
        assert_eq!(coalesce_phi_webs(&mut f), 0);
        assert_eq!(phi_dests(&f, 2), vec![20]);
    }

    /// Two phis of one block are defined simultaneously and never share.
    #[test]
    fn phis_of_one_block_never_share_a_name() {
        let mut f = func(vec![
            block(
                0,
                vec![Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 0,
                    ty: IrType::Ptr,
                }],
                Terminator::Branch(BlockId(1)),
            ),
            // A swap: x = phi[1, y]; y = phi[1, x].
            block(
                1,
                vec![phi(10, &[(1, 0), (11, 1)]), phi(11, &[(1, 0), (10, 1)])],
                cbr(10, 1, 2),
            ),
            block(
                2,
                vec![],
                Terminator::Return(Some(Operand::Value(Value(11)))),
            ),
        ]);
        assert_eq!(coalesce_phi_webs(&mut f), 0);
    }

    /// An indirect branch anywhere makes the function unsplittable: refused.
    #[test]
    fn functions_with_unsplittable_edges_are_refused() {
        let mut f = func(vec![
            block(
                0,
                vec![Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 0,
                    ty: IrType::Ptr,
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![phi(10, &[(1, 0), (20, 2)])],
                Terminator::Branch(BlockId(2)),
            ),
            block(
                2,
                vec![phi(20, &[(10, 1)])],
                Terminator::IndirectBranch {
                    target: Operand::Value(Value(20)),
                    possible_targets: vec![BlockId(1), BlockId(3)],
                },
            ),
            block(
                3,
                vec![],
                Terminator::Return(Some(Operand::Value(Value(20)))),
            ),
        ]);
        assert_eq!(coalesce_phi_webs(&mut f), 0);
    }
}
