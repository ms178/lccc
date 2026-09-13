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

/// Blocks reachable from the entry, following terminator and `asm goto` edges.
fn reachable_set(func: &IrFunction) -> std::collections::HashSet<usize> {
    let mut pos: FxHashMap<u32, usize> = FxHashMap::default();
    for (i, b) in func.blocks.iter().enumerate() {
        pos.insert(b.label.0, i);
    }
    let mut seen = std::collections::HashSet::new();
    if func.blocks.is_empty() {
        return seen;
    }
    let mut stack = vec![0usize];
    seen.insert(0usize);
    while let Some(bi) = stack.pop() {
        let mut succ: Vec<u32> = Vec::new();
        collect_successor_labels(&func.blocks[bi].terminator, &mut succ);
        for inst in &func.blocks[bi].instructions {
            if let crate::ir::instruction::Instruction::InlineAsm { goto_labels, .. } = inst {
                succ.extend(goto_labels.iter().map(|(_, l)| l.0));
            }
        }
        for sl in succ {
            if let Some(&sb) = pos.get(&sl) {
                if seen.insert(sb) {
                    stack.push(sb);
                }
            }
        }
    }
    seen
}

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
    relayout_blocks_loop_aware_impl(func, BlockLayoutStart::Existing)
}

/// RPO-start variant for size-optimized (`-Os`/`-Oz`) compilation.
///
/// The throughput pipeline starts from the function's existing block order
/// (see the comment in [`relayout_blocks_loop_aware_impl`]): that preserves
/// the fall-through topology earlier passes built, which keeps hot branches
/// not-taken — a measured ~19 % runtime win on `zlib_ng_adler32`.
///
/// That same property costs code size, and the cost is severe on 16-bit
/// real-mode boot images (`-m16 -Os`, the linux-cachymod setup code with its
/// hard 32 KiB `_end` gate): starting from RPO instead keeps mutually
/// branching blocks adjacent, which maximizes short rel8 branches. Measured
/// on the real-mode setup corpus this is worth ~570 B of `.text` — the
/// difference between passing the gate with headroom and overflowing it by
/// 2.5 KiB after the 4 KiB `.pecompat` alignment cliff. `-Os` users have
/// already chosen size over speed, so the RPO start is the right trade there.
pub(crate) fn relayout_blocks_loop_aware_rpo(func: &mut IrFunction) -> usize {
    relayout_blocks_loop_aware_impl(func, BlockLayoutStart::Rpo)
}

/// Which initial linearization [`relayout_blocks_loop_aware_impl`] compacts.
#[derive(Clone, Copy)]
enum BlockLayoutStart {
    /// Keep the function's current block order (reachable first, unreachable
    /// parked at the end) — the throughput default.
    Existing,
    /// Recompute reverse post-order from the entry — the size-optimized
    /// default (`-Os`/`-Oz`).
    Rpo,
}

fn relayout_blocks_loop_aware_impl(func: &mut IrFunction, start: BlockLayoutStart) -> usize {
    let n = func.blocks.len();
    if n < 2 {
        return 0;
    }

    // START FROM THE EXISTING ORDER, NOT FROM RPO.
    //
    // RPO is a *topological* order. It is a correct linearization and a
    // terrible layout: it discards the ordering the earlier passes
    // deliberately produced, and the backend then inverts every conditional
    // whose fall-through changed. Measured on `zlib_ng_adler32`, relayout in
    // RPO costs **19%** against not relaying out at all (55.8 vs 46.9 ms) --
    // a cost this pass has been paying since before it was loop-aware.
    //
    // The pass exists for one reason, stated in its own docstring: passes
    // append new blocks at the END of `func.blocks`, so a loop's exit block
    // can end up far from the loop and stretch live intervals across
    // unrelated code. That is a *contiguity* problem, and contiguity is all
    // that needs fixing. Everything else about the incoming order is
    // information -- keep it.
    //
    // Unreachable blocks are still parked at the end, and the loop-contiguity
    // compaction below is unchanged; only the starting point differs.
    //
    // `-Os`/`-Oz` (and therefore the `-m16` real-mode setup code) selects the
    // RPO start via the size policy; see [`relayout_blocks_loop_aware_rpo`].
    let mut order: Vec<usize> = match start {
        BlockLayoutStart::Existing => {
            let reachable = reachable_set(func);
            let mut v: Vec<usize> = (0..n).filter(|b| reachable.contains(b)).collect();
            v.extend((0..n).filter(|b| !reachable.contains(b)));
            v
        }
        BlockLayoutStart::Rpo => reverse_postorder(func),
    };

    let cfg = crate::ir::analysis::CfgAnalysis::build(func);
    let all_loops_for_depth = crate::passes::loop_analysis::merge_loops_by_header(
        crate::passes::loop_analysis::find_natural_loops(
            cfg.num_blocks,
            &cfg.preds,
            &cfg.succs,
            &cfg.idom,
        ),
    );
    // depth[b] = how many natural loops contain b. A block at depth 0 executes
    // O(1) times for the whole function; a block at depth >= 1 executes once
    // per iteration of whatever still encloses it.
    let mut depth = vec![0usize; n];
    for lp in &all_loops_for_depth {
        for &b in &lp.body {
            if b < n {
                depth[b] += 1;
            }
        }
    }

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
        // Already contiguous: nothing is interleaved, so nothing to sink.
        if last - first + 1 == lp.body.len() {
            continue;
        }

        // Only blocks that LEAVE the loop for good may be sunk.
        //
        // Sinking a block past the loop changes which successor of its
        // predecessor falls through, and the backend inverts the conditional
        // accordingly. That is a win exactly when the sunk block is cold, and
        // a loss when it is not -- on `zlib_ng_adler32` an earlier version of
        // this pass moved a block that re-enters the loop, flipping the hot
        // edge from fall-through to taken and costing **14.6%** (47.7 -> 55.9
        // ms), while the same transform won 40% on `memchr`.
        //
        // The discriminator has to be provable, not a guess about
        // probability. A block that is not in the loop body and whose every
        // successor is also outside the loop can be reached at most ONCE per
        // loop entry: control that arrives there never comes back. Sinking it
        // therefore cannot lengthen any path that executes per-iteration. A
        // block that re-enters the loop is on the iteration path by
        // definition and is left exactly where it is.
        //
        // `memchr`'s displaced block is a `return` -- no successors at all, so
        // trivially exit-only. adler32's is not, and is now left alone.
        let leaves_for_good = |bi: usize| -> bool {
            if in_loop[bi] {
                return false;
            }
            // AND it must be outside EVERY loop, not just this one.
            //
            // "Leaves this loop" does not imply cold. `zlib_ng_adler32` is
            // NMAX-chunked: the inner DO8 loop's exit block sits inside the
            // OUTER loop, so it runs once per outer iteration -- hot. Sinking
            // it flipped the hot edge from fall-through to taken and cost
            // 15.6% (47.2 -> 55.9 ms), and the earlier "exit-only" rule did
            // not catch it because the block genuinely never re-enters the
            // inner loop.
            //
            // Depth 0 is the provable statement: the block executes O(1) times
            // for the whole function, so no per-iteration path can get longer.
            // `memchr`'s displaced `return` is depth 0; adler32's is not.
            if depth[bi] != 0 {
                return false;
            }
            let mut succ: Vec<u32> = Vec::new();
            collect_successor_labels(&func.blocks[bi].terminator, &mut succ);
            for inst in &func.blocks[bi].instructions {
                if let crate::ir::instruction::Instruction::InlineAsm { goto_labels, .. } = inst {
                    succ.extend(goto_labels.iter().map(|(_, l)| l.0));
                }
            }
            succ.iter().all(|sl| {
                match func.blocks.iter().position(|b| b.label.0 == *sl) {
                    // An unresolvable target (a nested function's `asm goto`)
                    // is not a re-entry into this loop.
                    None => true,
                    Some(sb) => !in_loop[sb],
                }
            })
        };

        // Stable partition of the span [first, last]: loop blocks and any
        // interloper that can re-enter the loop keep their relative order and
        // stay put; only exit-only blocks are sunk past the loop.
        let span: Vec<usize> = order[first..=last].to_vec();
        let mut keep: Vec<usize> = Vec::with_capacity(span.len());
        let mut sink: Vec<usize> = Vec::new();
        for b in span {
            if !in_loop[b] && leaves_for_good(b) {
                sink.push(b);
            } else {
                keep.push(b);
            }
        }
        if sink.is_empty() {
            continue; // nothing provably cold to move
        }
        keep.extend(sink);
        order.splice(first..=last, keep);
    }

    apply_order(func, order)
}

/// Static-frequency greedy chain layout (profile-free block placement).
///
/// The throughput pipeline's historical start (`BlockLayoutStart::Existing`)
/// preserves the order passes happened to append blocks in; on
/// control-flow-heavy functions (rbtree insert/search, csv scanners) that
/// order scatters loop bodies across the whole function, so the backend's
/// fall-through machinery (`next_block_label`) never fires and every block
/// transition costs an explicit `jmp`. Measured on `linux_rbtree` main:
/// 108 unconditional jumps for 178 blocks, ZERO of them to the physically
/// adjacent block.
///
/// This pass builds a placement chain the way GCC/LLVM do without profiles:
/// walk from the entry, at each block place the *estimated-hottest*
/// unplaced successor next (it becomes the fall-through), and when the
/// current trace is stuck (all successors placed), start the next trace at
/// the unplaced block with the strongest claim (a placed predecessor and a
/// high static frequency). Edge estimates are structural, not guessed:
///
/// * an edge that stays inside a natural loop carries the iteration weight
///   (loops run many times);
/// * an edge that leaves a loop is cold;
/// * an edge that *enters* a loop from outside is hot (it leads to the
///   iterations);
/// * everything else (straight-line, if/else arms) is neutral, and ties
///   keep the TRUE successor first — the same default the backend's
///   conditional emitter prefers (`pref_true`), so the two components agree
///   on which edge becomes fall-through instead of fighting over it.
///
/// Loop contiguity is a *consequence* of the weights (the in-loop successor
/// always outranks the exit), not a post-hoc patch, so live intervals of
/// loop-carried values stay compact — the property the loop-aware
/// compaction exists to protect. That compaction still runs afterwards as
/// a safety net for the one shape the chain cannot keep contiguous by
/// itself: a `break` whose block has no other successor pulls the join
/// block into the middle of the loop; the compaction sinks it (depth-0
/// exit-only, the same provable-cold rule as always).
///
/// Determinism: every choice is a total order — `(weight, block index)` —
/// so repeated compilations produce identical placement.
///
/// Kill switch: `CCC_NO_STATIC_CHAIN` (any value) restores the historical
/// `Existing`-start behavior, preserving the A/B bisection surface.
pub(crate) fn relayout_blocks_static_chain(func: &mut IrFunction) -> usize {
    if std::env::var("CCC_NO_STATIC_CHAIN").is_ok() {
        return relayout_blocks_loop_aware(func);
    }
    let n = func.blocks.len();
    if n < 3 {
        return relayout_blocks_loop_aware(func);
    }

    let reachable = reachable_set(func);
    // Original label positions, snapshotted BEFORE any reordering here: the
    // dual-layout selection in the backend reconstructs the pre-chain order
    // from the final permutation (labels are renumbered later in the
    // pipeline, so the permutation must be computed against the labels the
    // function had when this pass ran).
    let orig_label_pos: FxHashMap<u32, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label.0, i))
        .collect();
    let cfg = crate::ir::analysis::CfgAnalysis::build(func);
    let loops = crate::passes::loop_analysis::merge_loops_by_header(
        crate::passes::loop_analysis::find_natural_loops(
            cfg.num_blocks,
            &cfg.preds,
            &cfg.succs,
            &cfg.idom,
        ),
    );
    // Static nesting depth per block and the innermost loop containing it.
    let mut depth = vec![0u32; n];
    for lp in &loops {
        for &b in &lp.body {
            if b < n {
                depth[b] += 1;
            }
        }
    }
    let mut innermost: Vec<Option<usize>> = vec![None; n];
    for (li, lp) in loops.iter().enumerate() {
        for &b in &lp.body {
            if b >= n {
                continue;
            }
            match innermost[b] {
                None => innermost[b] = Some(li),
                Some(prev) => {
                    if loops[li].body.len() < loops[prev].body.len() {
                        innermost[b] = Some(li);
                    }
                }
            }
        }
    }

    // Successor indices (terminator edges + `asm goto` labels), deduplicated.
    let pos_of_label: FxHashMap<u32, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label.0, i))
        .collect();
    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); n];
    for b in 0..n {
        let mut labels: Vec<u32> = Vec::new();
        collect_successor_labels(&func.blocks[b].terminator, &mut labels);
        for inst in &func.blocks[b].instructions {
            if let crate::ir::instruction::Instruction::InlineAsm { goto_labels, .. } = inst {
                labels.extend(goto_labels.iter().map(|(_, l)| l.0));
            }
        }
        for l in labels {
            if let Some(&t) = pos_of_label.get(&l) {
                if !succs[b].contains(&t) {
                    succs[b].push(t);
                }
            }
        }
    }

    // Edge weight: which successor of `s` deserves the fall-through slot.
    let weight = |s: usize, t: usize| -> u32 {
        if let Some(li) = innermost[s] {
            if loops[li].body.contains(&t) {
                9 // stays inside the loop: the iteration path
            } else if depth[t] > depth[s] {
                8 // enters a deeper nest (cannot be an exit of `s`'s loop)
            } else {
                1 // leaves the loop: executes at most once per entry
            }
        } else if depth[t] > depth[s] {
            8 // entering a loop from outside: leads to the iterations
        } else if succs[s].len() == 1 {
            5 // straight-line edge: the only way forward
        } else {
            4 // plain if/else arm: neutral, tie-break keeps TRUE first
        }
    };
    // Static frequency estimate for trace-head choice: nesting dominates.
    let freq = |b: usize| -> u64 { 10u64.saturating_pow(depth[b].min(9) as u32) };

    let mut is_placed = vec![false; n];
    let mut placed_pred = vec![0usize; n]; // placed predecessors of each block
    // Position of the most recently placed predecessor (LIFO recency for
    // the trace-head choice): continuing the region that just closed — a
    // loop-exit continuation — beats an older frontier block at equal
    // frequency (measured shape: a loop's exit block vs the function's
    // early-exit arm both sit at depth 0; recency puts the exit right
    // after the loop, where the latch's fall-through lands).
    let mut last_pred_pos = vec![0usize; n];
    let mut placed: Vec<usize> = Vec::with_capacity(n);
    let mut place = |b: usize,
                     is_placed: &mut Vec<bool>,
                     placed_pred: &mut Vec<usize>,
                     last_pred_pos: &mut Vec<usize>,
                     placed: &mut Vec<usize>| {
        let pos = placed.len();
        is_placed[b] = true;
        placed.push(b);
        for &t in &succs[b] {
            placed_pred[t] += 1;
            last_pred_pos[t] = pos;
        }
    };

    let entry = 0usize;
    debug_assert!(reachable.contains(&entry));
    place(
        entry,
        &mut is_placed,
        &mut placed_pred,
        &mut last_pred_pos,
        &mut placed,
    );
    let reachable_count = reachable.iter().filter(|b| **b < n).count();
    let mut cur = entry;
    while placed.len() < reachable_count {
        let cands: Vec<usize> = succs[cur]
            .iter()
            .copied()
            .filter(|&t| !is_placed[t])
            .collect();
        let next = if let Some(&best) = cands
            .iter()
            .max_by_key(|&&t| (weight(cur, t), std::cmp::Reverse(t)))
        {
            best
        } else {
            // Trace head: frontier blocks (a placed predecessor keeps the
            // chain geodesic) first, then nesting depth, then LIFO recency
            // of the newest placed predecessor, then original position —
            // all static, all total.
            (0..n)
                .filter(|b| reachable.contains(b) && !is_placed[*b])
                .max_by_key(|&b| {
                    (
                        placed_pred[b] > 0,
                        freq(b),
                        last_pred_pos[b],
                        std::cmp::Reverse(b),
                    )
                })
                .expect("reachable_count guards non-empty candidate set")
        };
        place(
            next,
            &mut is_placed,
            &mut placed_pred,
            &mut last_pred_pos,
            &mut placed,
        );
        cur = next;
    }

    // LAZINESS GATE (measured, not folkloric): the incoming block order is
    // itself information — passes such as if-conversion, unrolling and
    // loop-inversion append blocks in an order that already reflects their
    // intended fall-throughs, and the register allocator's intervals are
    // linearized over whatever order survives here. Replacing a good order
    // with an equally-good-but-different one buys zero branches and
    // re-rolls the allocator's dice (measured: `sqlite_varint` main +12
    // instructions, `zlib_ng_adler32` +7 — both from slot round-trips the
    // old order did not have, with the SAME branch count). So the chain is
    // applied only when it strictly reduces the weighted count of
    // non-adjacent successor edges — the same structural frequency
    // estimates the chain itself uses, evaluated on both orders with one
    // metric. Cost(weight) per edge: the edge's estimate when its target
    // is not the physically next block (an explicit taken branch);
    // fall-through/adjacent edges cost nothing.
    let position_of: Vec<usize> = {
        let mut pos = vec![0usize; n];
        for (i, &b) in placed.iter().enumerate() {
            pos[b] = i;
        }
        pos
    };
    let chain_cost = {
        let mut cost: u64 = 0;
        for (i, &s) in placed.iter().enumerate() {
            for &t in &succs[s] {
                if position_of[t] != i + 1 {
                    cost += weight(s, t) as u64;
                }
            }
        }
        cost
    };
    let current_cost = {
        // Current order = identity over block indices: successor `t` is
        // adjacent to block `i` exactly when `t == i + 1`.
        let mut cost: u64 = 0;
        for s in 0..n {
            for &t in &succs[s] {
                if t != s + 1 {
                    cost += weight(s, t) as u64;
                }
            }
        }
        cost
    };
    if std::env::var("CCC_DEBUG_CHAIN").is_ok() {
        eprintln!(
            "[CHAIN] {}: blocks={} current_cost={} chain_cost={} gain={}",
            func.name,
            n,
            current_cost,
            chain_cost,
            current_cost.saturating_sub(chain_cost)
        );
        eprintln!(
            "[CHAIN-ORDER] {}",
            placed
                .iter()
                .map(|b| b.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        let succ_dump: Vec<String> = (0..n)
            .map(|b| {
                format!(
                    "{}:{}",
                    b,
                    succs[b]
                        .iter()
                        .map(|x| x.to_string())
                        .collect::<Vec<_>>()
                        .join("/")
                )
            })
            .collect();
        eprintln!("[CHAIN-SUCCS] {}", succ_dump.join(" "));
    }
    // Minimum improvement for the chain to fire. Below 4 weighted units the
    // reorder buys no meaningful branch saving — it is a pure register-
    // allocator dice re-roll (measured: `sqlite_varint` main, gain 1, static
    // +12 instructions and slot round-trips inside the hot loop, dynamic
    // icount loss; two gain-0 functions similarly). Above it the branch win
    // is real and the allocator outcome is small-stakes noise in both
    // directions (measured winners down to gain 1: matmul -7; losers up to
    // gain 19: zstd +1 — the static metric does not predict the re-roll,
    // so the gate only refuses to pay risk for nothing). `zlib_ng_adler32`
    // at gain 28 won 1.5% dynamic retired instructions under the
    // deterministic QEMU icount A/B despite +7 static.
    let gain = current_cost.saturating_sub(chain_cost);
    if gain < 4 {
        return relayout_blocks_loop_aware_impl(func, BlockLayoutStart::Existing);
    }

    // Park unreachable blocks at the end (original relative order).
    for b in 0..n {
        if !is_placed[b] {
            placed.push(b);
        }
    }

    let moved = apply_order(func, placed);
    // Safety net: sink provably-cold blocks that a single-successor `break`
    // pulled into a loop span (no-op when the chain kept loops contiguous).
    let compacted = relayout_blocks_loop_aware_impl(func, BlockLayoutStart::Existing);
    // Dual-layout selection record: the PRE-CHAIN order is preserved for the
    // backend, which emits the function with both orders and keeps the one
    // with fewer instructions (see `emit_functions_and_sections`). The
    // register allocator's response to a reorder is not predictable from
    // static structure — measured, the same branch-quality reorder won
    // -42 instructions on `linux_rbtree` and lost +38 on `strlen_bench` —
    // so the keep/revert decision is made by measuring both compilations.
    // The recorded permutation maps FINAL positions to ORIGINAL positions
    // (computed after the compaction so it matches the emitted order).
    let final_perm: Vec<usize> = func
        .blocks
        .iter()
        .map(|b| {
            orig_label_pos
                .get(&b.label.0)
                .copied()
                .unwrap_or(usize::MAX)
        })
        .collect();
    let identity: Vec<usize> = (0..n).collect();
    if final_perm != identity {
        record_chain_candidate(&func.name, final_perm);
    }
    moved + compacted
}

// ── Dual-layout selection side channel ─────────────────────────────────────
//
// The backend consumes this after codegen-level block order is final:
// `final_perm[j]` = the ORIGINAL position of the block that ended up at
// position `j`. Keyed by function name (stable across the label renumber
// that runs between this pass and codegen). Same LazyLock+Mutex shape as
// the pgo hint maps: codegen is single-threaded and per-TU sequential.
static CHAIN_CANDIDATES: std::sync::LazyLock<std::sync::Mutex<FxHashMap<String, Vec<usize>>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(FxHashMap::default()));

/// Record the chain permutation for one function (called by the layout pass
/// when the chain fired and the laziness gate accepted it).
pub(crate) fn record_chain_candidate(name: &str, final_perm: Vec<usize>) {
    CHAIN_CANDIDATES
        .lock()
        .unwrap()
        .insert(name.to_string(), final_perm);
}

/// Take (and remove) the chain permutation for one function (called by the
/// backend's function-emission loop).
pub(crate) fn take_chain_candidate(name: &str) -> Option<Vec<usize>> {
    CHAIN_CANDIDATES.lock().unwrap().remove(name)
}

/// Drop all records (called at the start of each module's emission so a
/// previous translation unit can never leak into this one).
pub(crate) fn clear_chain_candidates() {
    CHAIN_CANDIDATES.lock().unwrap().clear();
}

/// Count instruction lines in an emitted-text region: the arch-neutral
/// metric for the dual-layout selection. A line is an instruction when it
/// is non-empty, is not a directive (leading `.`), not a comment (`#`),
/// not a label (trailing `:`), and not a cfi marker.
pub(crate) fn count_instruction_lines(region: &str) -> usize {
    region
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty()
                && !t.starts_with('.')
                && !t.starts_with('#')
                && !t.starts_with("cfi_")
                && !t.ends_with(':')
        })
        .count()
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
        assert!(
            pos(3) > pos(4),
            "cold block must sink past the latch: {:?}",
            order
        );
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

    /// The layout must not reorder a function that has no contiguity problem.
    ///
    /// This is the property whose absence cost 19%. The pass used to
    /// linearize every function into reverse post-order, which is a correct
    /// topological order but throws away the ordering earlier passes
    /// produced; the backend then inverts every conditional whose fall-through
    /// changed. On `zlib_ng_adler32` that was 55.8 ms against 46.9 ms for no
    /// relayout at all.
    ///
    /// The shape below is deliberately one RPO would permute: block 3 comes
    /// before block 2 in program order while the CFG reaches 2 first.
    #[test]
    fn a_function_with_contiguous_loops_is_not_reordered_at_all() {
        //   0 -> 1 ; 1: cond(3, 2) ; 2 -> 1 (latch) ; 3: ret
        let before = vec![
            blk(0, br(1)),
            blk(1, cond(3, 2)),
            blk(2, br(1)),
            blk(3, ret()),
        ];
        let expected: Vec<u32> = before.iter().map(|b| b.label.0).collect();
        let mut f = func_of(before);
        assert_eq!(
            relayout_blocks_loop_aware(&mut f),
            0,
            "no contiguity problem, so nothing may move"
        );
        assert_eq!(labels(&f), expected);
    }

    /// A block appended out of order -- the case the pass exists for -- is
    /// still pulled back next to its loop.
    #[test]
    fn an_appended_out_of_order_block_is_still_compacted() {
        //   0 -> 1 ; 1: cond(4, 2) ; 2 -> 3 ; 3 -> 1 (latch) ; 4: ret
        // with the cold return physically sitting between body and latch.
        let mut f = func_of(vec![
            blk(0, br(1)),
            blk(1, cond(4, 2)),
            blk(2, br(3)),
            blk(4, ret()),
            blk(3, br(1)),
        ]);
        assert_eq!(relayout_blocks_loop_aware(&mut f), 1, "must compact");
        let order = labels(&f);
        let pos = |l: u32| order.iter().position(|&x| x == l).unwrap();
        assert_eq!(pos(3), pos(2) + 1, "latch must follow the body: {order:?}");
        assert!(pos(4) > pos(3), "cold return must sink: {order:?}");
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

#[cfg(test)]
mod chain_tests {
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

    /// A loop whose blocks were appended out of order (the pass's founding
    /// defect, in chain form): the chain walks entry -> header -> body ->
    /// latch and the exit lands after the latch, so every in-loop edge is
    /// adjacent and the loop is contiguous.
    #[test]
    fn chain_places_a_scattered_loop_contiguously() {
        // entry(0) -> header(1) cond {2 body, 5 exit}; 2 -> latch(3);
        // 3 -> 1 (back edge). Blocks given in a scrambled order.
        let f = func_of(vec![
            blk(0, cond(1, 6)),
            blk(6, ret()),
            blk(3, br(1)), // latch, appended early
            blk(1, cond(2, 5)),
            blk(5, br(6)), // loop exit path
            blk(2, br(3)), // body
        ]);
        let mut f = f;
        relayout_blocks_static_chain(&mut f);
        // The chain must start with entry and keep {1,2,3} contiguous with
        // the exit (5) right after the loop, before the return (6).
        assert_eq!(labels(&f)[0], 0);
        let l = labels(&f);
        let h = l.iter().position(|&x| x == 1).unwrap();
        assert_eq!(&l[h..h + 3], &[1, 2, 3], "loop body contiguous: {:?}", l);
        assert_eq!(l[h + 3], 5, "exit follows the loop: {:?}", l);
    }

    /// The laziness gate: when the incoming order is already optimal (the
    /// loop contiguous, exits adjacent), the gain is zero and the original
    /// order must be preserved exactly.
    #[test]
    fn chain_keeps_an_already_good_order_untouched() {
        let good = vec![
            blk(0, br(1)),
            blk(1, cond(2, 5)),
            blk(2, br(3)),
            blk(3, br(1)),
            blk(5, ret()),
        ];
        let mut f = func_of(good.clone());
        relayout_blocks_static_chain(&mut f);
        assert_eq!(labels(&f), labels(&func_of(good)));
    }

    /// Unreachable blocks stay parked at the tail, in original relative
    /// order, and never interleave the reachable chain.
    #[test]
    fn chain_parks_unreachable_blocks_at_the_tail() {
        let mut f = func_of(vec![
            blk(0, br(1)),
            blk(1, ret()),
            blk(7, ret()), // unreachable
            blk(8, br(7)), // unreachable
        ]);
        relayout_blocks_static_chain(&mut f);
        let l = labels(&f);
        assert_eq!(&l[..2], &[0, 1]);
        assert_eq!(&l[2..], &[7, 8], "unreachable keep relative order");
    }

    /// The dual-selection permutation round trip: what the layout records
    /// must let the backend reconstruct the pre-chain order exactly. Uses
    /// the permutation the pass ACTUALLY recorded (not a hand-written one),
    /// so the contract tested is the shipped one.
    #[test]
    fn chain_candidate_permutation_reconstructs_the_original_order() {
        let mut f = func_of(vec![
            blk(0, cond(1, 6)),
            blk(6, ret()),
            blk(3, br(1)),
            blk(1, cond(2, 5)),
            blk(5, br(6)),
            blk(2, br(3)),
        ]);
        let original_labels = labels(&f);
        relayout_blocks_static_chain(&mut f);
        let perm = take_chain_candidate("t").expect("chain fired and recorded");
        assert!(
            take_chain_candidate("t").is_none(),
            "take drains the record"
        );
        // Reconstruct exactly as the backend does: perm[j] is the ORIGINAL
        // position of the block at position j, so orig[perm[j]] = chain[j].
        let mut orig: Vec<Option<BasicBlock>> = (0..f.blocks.len()).map(|_| None).collect();
        for (j, &orig_pos) in perm.iter().enumerate() {
            orig[orig_pos] = Some(f.blocks[j].clone());
        }
        let restored: Vec<BasicBlock> = orig
            .into_iter()
            .map(|b| b.expect("total permutation"))
            .collect();
        assert_eq!(
            restored.iter().map(|b| b.label.0).collect::<Vec<_>>(),
            original_labels
        );
    }

    /// count_instruction_lines: the arch-neutral metric must skip labels,
    /// directives, comments, and blank lines.
    #[test]
    fn count_instruction_lines_skips_non_instructions() {
        let text = ".text\nmain:\n\t pushq %rbx\n\t movq %rax, %rbx\n.L1:\n\t # comment\n\n\t ret\n.size main, .-main\n";
        assert_eq!(count_instruction_lines(text), 3);
    }
}
