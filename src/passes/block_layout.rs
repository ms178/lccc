//! Block layout: reorder function blocks into reverse post-order.
//!
//! Several passes (vectorization, inlining, switch outlining) append new
//! blocks at the end of `func.blocks`, leaving the linearized block order
//! unrelated to execution order. The backend's liveness analysis numbers
//! instructions in block order, so an out-of-order loop exit block stretches
//! live intervals across unrelated code — including calls — which defeats
//! register allocation (a vector or FP accumulator that should stay in a
//! register for the whole loop gets stack-homed instead).
//!
//! Re-layout in reverse post-order keeps each loop's blocks and its exit
//! block contiguous. Block identity is the `label` field (terminators
//! reference labels, not positions), so reordering the list is
//! semantics-preserving.
//!
//! # Why RPO alone is not enough
//!
//! RPO is a *topological* order; it has no notion of which edge is HOT. In a
//! search loop it happily places the cold `return` block between the body and
//! the latch:
//!
//! ```text
//! .LBB1:  cmpq %rdx, %rbx ; jae .LBB5     # loop test, not taken
//! .LBB2:  movzbl (%rdi,%rbx), %eax
//!         cmpl %r8d, %eax  ; jne .LBB4    # TAKEN every iteration
//! .LBB3:  <return the match>              # cold, executes at most once
//! .LBB4:  leaq 1(%rbx), %rbx ; jmp .LBB1  # TAKEN every iteration
//! .LBB5:  <return null>
//! ```
//!
//! Three branches per iteration, **two of them taken**, where Clang emits two
//! branches with one taken. The instruction count is not the problem — the
//! layout is.
//!
//! [`relayout_blocks_loop_aware`] fixes it by preferring, at every step, the
//! successor that stays inside the current loop. The cold returns sink past
//! the latch, the body falls straight into the latch, and the backend's
//! existing branch inversion (`comparison.rs`, which flips the condition when
//! the true-target is the next block) turns the taken `jne .LBB4` into a
//! not-taken `je .LBB3` for free. Same instructions, one fewer taken branch
//! per iteration.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::instruction::{BasicBlock, Terminator};
use crate::ir::reexports::IrFunction;

/// Reverse post-order of the block indices, entry first.
/// Unreachable blocks keep their original relative order at the end.
fn reverse_postorder(func: &IrFunction) -> Vec<usize> {
    let mut pos_of_label: FxHashMap<u32, usize> = FxHashMap::default();
    for (idx, block) in func.blocks.iter().enumerate() {
        pos_of_label.insert(block.label.0, idx);
    }
    let entry = func.blocks[0].label.0;
    let mut visited: FxHashSet<u32> = FxHashSet::default();
    let mut postorder: Vec<u32> = Vec::with_capacity(func.blocks.len());
    let mut stack: Vec<(u32, bool)> = vec![(entry, false)];
    while let Some((label, processed)) = stack.pop() {
        if processed {
            postorder.push(label);
            continue;
        }
        if !visited.insert(label) {
            continue;
        }
        stack.push((label, true));
        let mut succs: Vec<u32> = Vec::new();
        if let Some(&idx) = pos_of_label.get(&label) {
            collect_successor_labels(&func.blocks[idx].terminator, &mut succs);
            for inst in &func.blocks[idx].instructions {
                if let crate::ir::instruction::Instruction::InlineAsm { goto_labels, .. } = inst {
                    succs.extend(goto_labels.iter().map(|(_, l)| l.0));
                }
            }
        }
        for &sl in &succs {
            if pos_of_label.contains_key(&sl) && !visited.contains(&sl) {
                stack.push((sl, false));
            }
        }
    }
    postorder.reverse();
    let mut order: Vec<usize> = postorder.iter().map(|l| pos_of_label[l]).collect();
    let mut seen: FxHashSet<usize> = order.iter().copied().collect();
    for idx in 0..func.blocks.len() {
        if seen.insert(idx) {
            order.push(idx);
        }
    }
    order
}

/// Commit `new_order` to `func.blocks`. Returns 1 if anything moved.
fn apply_order(func: &mut IrFunction, new_order: Vec<usize>) -> usize {
    debug_assert_eq!(new_order.len(), func.blocks.len());
    if new_order
        .iter()
        .enumerate()
        .all(|(new_pos, &old_pos)| new_pos == old_pos)
    {
        return 0;
    }
    let mut old: Vec<Option<BasicBlock>> = func.blocks.drain(..).map(Some).collect();
    func.blocks = new_order
        .into_iter()
        .map(|i| old[i].take().expect("each block placed exactly once"))
        .collect();
    1
}

/// Reorder `func.blocks` into reverse post-order from the entry block.
/// Unreachable blocks (if any) keep their original relative order at the end.
/// Returns 1 if the order changed, 0 otherwise.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn relayout_blocks_rpo(func: &mut IrFunction) -> usize {
    if func.blocks.len() < 2 {
        return 0;
    }
    let mut pos_of_label: FxHashMap<u32, usize> = FxHashMap::default();
    for (idx, block) in func.blocks.iter().enumerate() {
        pos_of_label.insert(block.label.0, idx);
    }

    // Iterative post-order DFS from the entry block. Successors are pushed in
    // natural (true, false) order so the stack explores the last successor
    // first; after the final reversal this places loop bodies right after
    // their header and exit blocks right after the loop.
    let entry = func.blocks[0].label.0;
    let mut visited: FxHashSet<u32> = FxHashSet::default();
    let mut postorder: Vec<u32> = Vec::with_capacity(func.blocks.len());
    let mut stack: Vec<(u32, bool)> = vec![(entry, false)];
    while let Some((label, processed)) = stack.pop() {
        if processed {
            postorder.push(label);
            continue;
        }
        if !visited.insert(label) {
            continue;
        }
        stack.push((label, true));
        let mut succs: Vec<u32> = Vec::new();
        if let Some(&idx) = pos_of_label.get(&label) {
            collect_successor_labels(&func.blocks[idx].terminator, &mut succs);
            // InlineAsm goto_labels are implicit control-flow edges.
            for inst in &func.blocks[idx].instructions {
                if let crate::ir::instruction::Instruction::InlineAsm { goto_labels, .. } = inst {
                    succs.extend(goto_labels.iter().map(|(_, l)| l.0));
                }
            }
        }
        for &s in &succs {
            // Non-local goto label aliases are file-unique: a nested
            // function's InlineAsm goto edge can name a block of the
            // ENCLOSING function, which is not in this function's
            // pos_of_label map.  Such an edge is not a real CFG successor
            // here (the branch happens after a full frame restore), so
            // skip it instead of panicking on the map index below.
            if pos_of_label.contains_key(&s) && !visited.contains(&s) {
                stack.push((s, false));
            }
        }
    }

    postorder.reverse();
    let mut new_order: Vec<usize> = postorder.iter().map(|l| pos_of_label[l]).collect();
    let mut placed: FxHashSet<usize> = new_order.iter().copied().collect();
    for idx in 0..func.blocks.len() {
        if placed.insert(idx) {
            new_order.push(idx);
        }
    }

    if new_order
        .iter()
        .enumerate()
        .all(|(new_pos, &old_pos)| new_pos == old_pos)
    {
        return 0;
    }
    let mut old: Vec<Option<BasicBlock>> = func.blocks.drain(..).map(Some).collect();
    func.blocks = new_order
        .into_iter()
        .map(|i| old[i].take().unwrap())
        .collect();
    1
}

/// Loop-aware block placement.
///
/// Starts from reverse post-order and makes the **minimum** deviation needed
/// to restore one property: every natural loop's body must be contiguous.
/// Blocks that are not in the loop but that RPO interleaved with it are pushed
/// out past the loop, preserving their relative order.
///
/// # Why minimal, and not a greedy hot-successor chain
///
/// The first version of this pass built a greedy chain, always extending with
/// the highest-loop-depth unplaced successor. On a search loop that produced
/// exactly the right answer (`memchr` −40%), but on straight-line-heavy code
/// where every candidate sits at the *same* depth the tie-break still walked
/// the CFG chain-first instead of topologically, reshuffling blocks for no
/// reason — the inlined SQLite varint decoder regressed 8.2%. Reordering
/// blocks is never free: it moves code across cache lines and changes which
/// branches are forward and backward.
///
/// So the rule is: if a region has no loop-contiguity problem, its RPO order
/// is left exactly as it was. The only blocks that move are the ones sitting
/// between a loop's blocks while not belonging to that loop.
///
/// # What it buys
///
/// In a search loop RPO lays the cold `return` out between the body and the
/// latch, so the body's branch to the latch is a TAKEN forward jump over it:
///
/// ```text
/// .LBB2:  cmpl %r8d, %eax ; jne .LBB4   # taken every iteration
/// .LBB3:  <return the match>            # cold
/// .LBB4:  leaq 1(%rbx), %rbx ; jmp .LBB1
/// ```
///
/// Sinking `.LBB3` makes the latch fall through, and the backend's existing
/// branch inversion (`comparison.rs` flips the condition when the true-target
/// is the next block) rewrites the branch to a NOT-taken `je .LBB4`. Same
/// instructions, one fewer taken branch per iteration.
///
/// Only the ORDER of `func.blocks` changes; terminators address blocks by
/// label, never by position, so this is semantics-preserving.
///
/// Returns 1 if the order changed, 0 otherwise.
pub(crate) fn relayout_blocks_loop_aware(func: &mut IrFunction) -> usize {
    let n = func.blocks.len();
    if n < 2 {
        return 0;
    }

    let mut order = reverse_postorder(func);

    let cfg = crate::ir::analysis::CfgAnalysis::build(func);
    let mut loops = crate::passes::loop_analysis::find_natural_loops(
        cfg.num_blocks,
        &cfg.preds,
        &cfg.succs,
        &cfg.idom,
    );
    if loops.is_empty() {
        return 0;
    }
    // Innermost first: compacting an inner loop cannot break an outer one,
    // because the outer body is a superset and compaction preserves relative
    // order within it. The reverse is not true.
    loops.sort_by_key(|lp| lp.body.len());

    for lp in &loops {
        // Positions, in current layout order, of this loop's blocks.
        let mut in_loop = vec![false; n];
        for &b in &lp.body {
            if b < n {
                in_loop[b] = true;
            }
        }
        let first = match order.iter().position(|&b| in_loop[b]) {
            Some(p) => p,
            None => continue,
        };
        let last = order
            .iter()
            .rposition(|&b| in_loop[b])
            .expect("first exists, so last does");
        // Already contiguous: leave this region completely alone.
        if last - first + 1 == lp.body.len() {
            continue;
        }

        // Stable partition of the span [first, last]: loop blocks keep their
        // relative order and come first; the interlopers follow, also in
        // their original relative order.
        let span: Vec<usize> = order[first..=last].to_vec();
        let mut body_part: Vec<usize> = Vec::with_capacity(lp.body.len());
        let mut other_part: Vec<usize> = Vec::with_capacity(span.len());
        for b in span {
            if in_loop[b] {
                body_part.push(b);
            } else {
                other_part.push(b);
            }
        }
        body_part.extend(other_part);
        order.splice(first..=last, body_part);
    }

    apply_order(func, order)
}

/// Collect the target label IDs of a terminator.
fn collect_successor_labels(term: &Terminator, out: &mut Vec<u32>) {
    match term {
        Terminator::Branch(l) => out.push(l.0),
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            out.push(true_label.0);
            out.push(false_label.0);
        }
        Terminator::IndirectBranch {
            possible_targets, ..
        } => {
            out.extend(possible_targets.iter().map(|l| l.0));
        }
        Terminator::Switch { cases, default, .. } => {
            out.extend(cases.iter().map(|(_, l)| l.0));
            out.push(default.0);
        }
        Terminator::Return(_) | Terminator::Unreachable => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::instruction::BlockId;
    use crate::ir::reexports::{IrConst, Operand, Value};

    fn blk(label: u32, term: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions: Vec::new(),
            terminator: term,
            source_spans: Vec::new(),
        }
    }

    fn br(t: u32) -> Terminator {
        Terminator::Branch(BlockId(t))
    }

    fn cond(t: u32, f: u32) -> Terminator {
        Terminator::CondBranch {
            cond: Operand::Value(Value(99)),
            true_label: BlockId(t),
            false_label: BlockId(f),
        }
    }

    fn ret() -> Terminator {
        Terminator::Return(Some(Operand::Const(IrConst::I32(0))))
    }

    fn func_of(blocks: Vec<BasicBlock>) -> IrFunction {
        let mut f = IrFunction::new("t".into(), IrType::I32, Vec::new(), false);
        f.blocks = blocks;
        f
    }

    fn labels(f: &IrFunction) -> Vec<u32> {
        f.blocks.iter().map(|b| b.label.0).collect()
    }

    /// The defect this pass exists for: a search loop whose cold `return` sits
    /// between the body and the latch, making the body's branch to the latch a
    /// TAKEN forward jump over it.
    ///
    ///   0 entry -> 1
    ///   1 header: i < n ? 2 : 5        (loop)
    ///   2 body:   hit  ? 3 : 4         (loop)
    ///   3 found:  return               (NOT in the loop -- must sink)
    ///   4 latch:  -> 1                 (loop)
    ///   5 exit:   return
    #[test]
    fn a_cold_block_between_the_body_and_the_latch_is_sunk_past_the_loop() {
        let mut f = func_of(vec![
            blk(0, br(1)),
            blk(1, cond(5, 2)),
            blk(2, cond(3, 4)),
            blk(3, ret()),
            blk(4, br(1)),
            blk(5, ret()),
        ]);
        assert_eq!(relayout_blocks_loop_aware(&mut f), 1, "layout must change");

        let order = labels(&f);
        let pos = |l: u32| order.iter().position(|&x| x == l).unwrap();
        assert_eq!(order[0], 0, "entry must stay first");
        // The loop {1,2,4} is contiguous...
        assert_eq!(pos(2), pos(1) + 1);
        assert_eq!(pos(4), pos(2) + 1);
        // ...and the cold return no longer splits it.
        assert!(pos(3) > pos(4), "cold block must sink past the latch: {:?}", order);
    }

    /// The property that keeps this pass from being a random shuffler. When a
    /// loop is already contiguous the order must come back BYTE-IDENTICAL --
    /// the greedy first version reshuffled equal-depth blocks here and cost
    /// 8.2% on the inlined SQLite varint decoder.
    #[test]
    fn an_already_contiguous_loop_is_left_exactly_as_it_was() {
        //   0 -> 1 ; 1: cond(2,4) ; 2 -> 3 ; 3 -> 1 ; 4: ret
        let before = vec![
            blk(0, br(1)),
            blk(1, cond(4, 2)),
            blk(2, br(3)),
            blk(3, br(1)),
            blk(4, ret()),
        ];
        let expected: Vec<u32> = before.iter().map(|b| b.label.0).collect();
        let mut f = func_of(before);
        assert_eq!(
            relayout_blocks_loop_aware(&mut f),
            0,
            "a contiguous loop must not be touched"
        );
        assert_eq!(labels(&f), expected);
    }

    /// Straight-line and branchy code with no loop at all must be untouched:
    /// there is no contiguity to restore, so there is no justification for
    /// moving anything.
    #[test]
    fn a_function_without_loops_is_left_exactly_as_it_was() {
        let before = vec![
            blk(0, cond(1, 2)),
            blk(1, br(3)),
            blk(2, br(3)),
            blk(3, ret()),
        ];
        let expected: Vec<u32> = before.iter().map(|b| b.label.0).collect();
        let mut f = func_of(before);
        assert_eq!(relayout_blocks_loop_aware(&mut f), 0);
        assert_eq!(labels(&f), expected);
    }

    /// Nested loops must both end up contiguous, and the inner one must stay
    /// inside the outer one.
    #[test]
    fn nested_loops_both_become_contiguous() {
        //   0 -> 1
        //   1 outer header : cond(6, 2)
        //   2 inner header : cond(5, 3)
        //   3 inner body   : -> 4          (cold block 7 interleaved after it)
        //   4 inner latch  : -> 2
        //   5 outer latch  : -> 1
        //   6 exit         : ret
        //   7 cold         : ret           (reached from 3)
        let mut f = func_of(vec![
            blk(0, br(1)),
            blk(1, cond(6, 2)),
            blk(2, cond(5, 3)),
            blk(3, cond(7, 4)),
            blk(4, br(2)),
            blk(5, br(1)),
            blk(6, ret()),
            blk(7, ret()),
        ]);
        relayout_blocks_loop_aware(&mut f);
        let order = labels(&f);
        let pos = |l: u32| order.iter().position(|&x| x == l).unwrap();

        for (name, body) in [("inner", vec![2u32, 3, 4]), ("outer", vec![1, 2, 3, 4, 5])] {
            let mut ps: Vec<usize> = body.iter().map(|&l| pos(l)).collect();
            ps.sort_unstable();
            assert_eq!(
                ps.last().unwrap() - ps[0] + 1,
                body.len(),
                "{} loop must be contiguous in {:?}",
                name,
                order
            );
        }
        assert!(pos(7) > pos(5), "cold block must sink past the outer loop");
    }

    /// Reordering must never lose, duplicate or rename a block.
    #[test]
    fn every_block_survives_exactly_once() {
        let mut f = func_of(vec![
            blk(0, br(1)),
            blk(1, cond(5, 2)),
            blk(2, cond(3, 4)),
            blk(3, ret()),
            blk(4, br(1)),
            blk(5, ret()),
        ]);
        relayout_blocks_loop_aware(&mut f);
        let mut got = labels(&f);
        got.sort_unstable();
        assert_eq!(got, vec![0, 1, 2, 3, 4, 5]);
    }

    /// An unreachable block (not yet swept by cfg_simplify) must not be lost.
    #[test]
    fn an_unreachable_block_is_preserved() {
        let mut f = func_of(vec![
            blk(0, br(1)),
            blk(1, cond(5, 2)),
            blk(2, cond(3, 4)),
            blk(3, ret()),
            blk(4, br(1)),
            blk(5, ret()),
            blk(9, ret()), // unreachable
        ]);
        relayout_blocks_loop_aware(&mut f);
        let mut got = labels(&f);
        got.sort_unstable();
        assert_eq!(got, vec![0, 1, 2, 3, 4, 5, 9]);
    }
}
