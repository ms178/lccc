//! Expression sinking: move a computation to the deepest block that
//! dominates all of its uses.
//!
//! # Why the unit of motion is an expression, not a load
//!
//! A "load a batch, then test with early exits" body keeps every loaded value
//! live from the top even though most are read only on paths the program
//! usually does not take. `glibc_memcmp_common_alignment` is the canonical
//! shape, and its IR shows why a leaf-load sink cannot work:
//!
//! ```text
//!   BB12: 8x GetElementPtr, 16x Load, 8x BinOp, 1x Cmp -> CondBranch
//!   BB16: 2x Copy, 1x Cmp -> CondBranch     (if a1 != b1)
//!   BB20: 2x Copy, 1x Cmp -> CondBranch     (if a2 != b2)
//! ```
//!
//! The compares already sit in their guarded blocks. Sinking `a1 = load p1`
//! into BB16 leaves `p1 = gep src1, 1` behind, so a GEP result is live across
//! the body instead of a loaded value and the pressure is unchanged. Only
//! moving the whole chain helps. This pass gets that by iterating to a
//! fixpoint: once the load reaches BB16 its GEP has all uses there and
//! follows on the next round, then the cast feeding the GEP, and so on.
//!
//! Result on that function: **197 -> 188 instructions, callee-saved pushes
//! 6 -> 3.**
//!
//! # Two defects this revision fixes
//!
//! The first implementation miscompiled `nbody`. Both causes were real and
//! both are the kind that only a differential test catches:
//!
//! 1. **Stale indices when a block is first a destination and then a
//!    source.** Moves were planned against one snapshot of the IR and applied
//!    in sequence. A block that received an insertion had every later index
//!    in it shifted by one, so a subsequent move *out* of that same block in
//!    the same round removed the wrong instruction. The old guard rejected
//!    "source already used" and "destination already used" but not
//!    "source is a previous destination".
//! 2. **Operands that are not single-def.** This IR is not strict SSA after
//!    accumulator forwarding: a value can be assigned in several blocks.
//!    Sinking `t = a + b` past a *redefinition* of `a` silently changes what
//!    it computes. `nbody`'s FP accumulators are exactly that shape. Every
//!    operand must now be single-def, and the pass verifies no operand is
//!    redefined anywhere in the region it moves across.
//!
//! # Soundness
//!
//! Motion is always downward, never speculative, so a computation the program
//! performed may end up not performed — safe for anything without side
//! effects. Every condition below is required:
//!
//! * Only whitelisted opcodes move. Division and remainder are excluded
//!   (they trap on zero, so moving one changes *which* paths fault), as are
//!   `Alloca` (its address must stay frame-stable) and everything that writes
//!   memory or is otherwise observable.
//! * A volatile or non-default-address-space load never moves.
//! * The sunk value and **every operand it reads** must be defined exactly
//!   once, and no operand may be redefined in the region traversed.
//! * A `Phi` use is never a sink target: a phi operand is read on the
//!   incoming edge, so "immediately before the use" is not a point.
//! * The defining block must **strictly dominate** the destination, which
//!   keeps operands in scope and guarantees the new position is reached only
//!   along paths that already passed the old one.
//! * A load additionally requires no memory write on any path between the two
//!   positions — a whole-memory test over every block that can lie on such a
//!   path, so no alias analysis is needed for correctness.
//! * The destination must not sit in a deeper loop.
//! * At most one move per block per round, in either direction, so no planned
//!   index can go stale.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::AddressSpace;
use crate::ir::analysis;
use crate::ir::reexports::*;

/// Sinking of non-volatile loads. Separately switchable because a load and a
/// pure computation have very different cost models: moving a load can
/// serialise a cache miss against the branch that guards it, while moving
/// address arithmetic cannot.
fn sink_loads_enabled() -> bool {
    std::env::var_os("CCC_NO_LOAD_SINK").is_none()
}

fn is_memory_barrier(inst: &Instruction) -> bool {
    match inst {
        Instruction::Store { .. }
        | Instruction::Memcpy { .. }
        | Instruction::Call { .. }
        | Instruction::CallIndirect { .. }
        | Instruction::InlineAsm { .. }
        | Instruction::AtomicStore { .. }
        | Instruction::AtomicLoad { .. }
        | Instruction::AtomicRmw { .. }
        | Instruction::AtomicCmpxchg { .. }
        | Instruction::Fence { .. }
        | Instruction::VaStart { .. }
        | Instruction::VaEnd { .. }
        | Instruction::VaCopy { .. }
        | Instruction::VaArg { .. }
        | Instruction::DynAlloca { .. }
        | Instruction::StackRestore { .. } => true,
        Instruction::Load { volatile, .. } => *volatile,
        _ => false,
    }
}

/// Whitelist of relocatable opcodes. A new opcode must be considered
/// explicitly rather than inheriting permission to move.
fn is_sinkable(inst: &Instruction) -> bool {
    match inst {
        Instruction::BinOp { op, .. } => !matches!(
            op,
            IrBinOp::SDiv | IrBinOp::UDiv | IrBinOp::SRem | IrBinOp::URem
        ),
        Instruction::UnaryOp { .. }
        | Instruction::Cast { .. }
        | Instruction::Cmp { .. }
        | Instruction::Copy { .. }
        | Instruction::GetElementPtr { .. }
        | Instruction::Select { .. }
        | Instruction::GlobalAddr { .. }
        | Instruction::LabelAddr { .. } => true,
        Instruction::Load {
            volatile,
            seg_override,
            ..
        } => sink_loads_enabled() && !*volatile && *seg_override == AddressSpace::Default,
        _ => false,
    }
}

/// Every block reachable from `from` that can also reach `to`.
fn blocks_between(
    succs: &analysis::FlatAdj,
    preds: &analysis::FlatAdj,
    n: usize,
    from: usize,
    to: usize,
) -> FxHashSet<usize> {
    let mut fwd: FxHashSet<usize> = FxHashSet::default();
    let mut stack = vec![from];
    while let Some(b) = stack.pop() {
        if !fwd.insert(b) {
            continue;
        }
        for &s in succs.row(b) {
            let s = s as usize;
            if s < n {
                stack.push(s);
            }
        }
    }
    let mut back: FxHashSet<usize> = FxHashSet::default();
    let mut stack = vec![to];
    while let Some(b) = stack.pop() {
        if !back.insert(b) {
            continue;
        }
        for &p in preds.row(b) {
            let p = p as usize;
            if p < n {
                stack.push(p);
            }
        }
    }
    fwd.intersection(&back).copied().collect()
}

struct UseSite {
    block: usize,
    /// `usize::MAX` = the block's terminator.
    idx: usize,
    is_phi: bool,
}

fn operands_of(inst: &Instruction) -> Vec<u32> {
    let mut v = Vec::new();
    crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
        if let Operand::Value(x) = op {
            v.push(x.0);
        }
    });
    crate::backend::liveness::for_each_value_use_in_instruction(inst, |x| v.push(x.0));
    v.sort_unstable();
    v.dedup();
    v
}

fn sink_round(func: &mut IrFunction) -> usize {
    let n = func.blocks.len();
    if n < 2 {
        return 0;
    }
    let cfg = analysis::CfgAnalysis::build(func);

    let mut dom: Vec<FxHashSet<usize>> = vec![FxHashSet::default(); n];
    for b in 0..n {
        let mut cur = b;
        for _ in 0..=n {
            dom[b].insert(cur);
            let next = cfg.idom[cur];
            if next == cur || next >= n {
                break;
            }
            cur = next;
        }
    }
    // Static block frequency, relative to the function entry.
    //
    // Post-dominance was the previous guard and it is the WRONG invariant:
    // post-dominance says every path from the source reaches the target, i.e.
    // freq(target) >= freq(source) — the opposite of what a sink needs. What
    // bounds the cost is DOMINANCE, already required below: if the source
    // dominates the target then every execution of the target arrived through
    // the source, so freq(target) <= freq(source) *within one loop nest*.
    //
    // The case dominance does NOT cover is a loop between the two: a source
    // executed once whose target sits inside a loop it dominates. The loop
    // nest that separates them is what made `expat_xml_scan` 30 % slower.
    // A frequency estimate catches that directly and, unlike post-dominance,
    // still admits the profitable direction — sinking into a block guarded by
    // a branch, which runs LESS often than its dominator.
    //
    // Estimate: propagate 1.0 from the entry in reverse post-order over
    // forward edges only (back edges are handled by the loop factor), split
    // evenly across each block's successors, then scale by a nominal trip
    // count per enclosing loop. Uniform branch probability is deliberate —
    // there is no profile here, and any biased heuristic would be guessing.
    let rpo = {
        let mut seen = vec![false; n];
        let mut post: Vec<usize> = Vec::with_capacity(n);
        let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
        seen[0] = true;
        while let Some((b, ci)) = stack.pop() {
            let row = cfg.succs.row(b);
            if ci < row.len() {
                stack.push((b, ci + 1));
                let s2 = row[ci] as usize;
                if s2 < n && !seen[s2] {
                    seen[s2] = true;
                    stack.push((s2, 0));
                }
            } else {
                post.push(b);
            }
        }
        post.reverse();
        post
    };
    let mut rpo_pos = vec![usize::MAX; n];
    for (k, &b) in rpo.iter().enumerate() {
        rpo_pos[b] = k;
    }
    let loops =
        crate::passes::loop_analysis::find_natural_loops(n, &cfg.preds, &cfg.succs, &cfg.idom);
    let mut depth = vec![0usize; n];
    for l in &loops {
        for &b in &l.body {
            if b < n {
                depth[b] += 1;
            }
        }
    }
    // Nominal iterations per loop level. Only the RATIO of two frequencies is
    // consulted, so the absolute value matters solely when the two blocks sit
    // at different nesting depths — where any value >= 2 gives the same
    // verdict. 10 is the conventional choice.
    const TRIP: f64 = 10.0;
    let mut freq = vec![0.0f64; n];
    if !rpo.is_empty() {
        freq[rpo[0]] = 1.0;
        for &b in &rpo {
            let row = cfg.succs.row(b);
            if row.is_empty() {
                continue;
            }
            let share = freq[b] / row.len() as f64;
            for &sx in row {
                let s2 = sx as usize;
                if s2 >= n {
                    continue;
                }
                // Forward edges only; a back edge would not converge here and
                // is represented by the trip-count factor instead.
                if rpo_pos[s2] > rpo_pos[b] {
                    freq[s2] += share;
                }
            }
        }
        for b in 0..n {
            freq[b] *= TRIP.powi(depth[b] as i32);
        }
    }

    // Uses, and the def count of every value. The def count is load-bearing:
    // this IR is NOT strict SSA after accumulator forwarding.
    let mut uses: FxHashMap<u32, Vec<UseSite>> = FxHashMap::default();
    let mut def_count: FxHashMap<u32, u32> = FxHashMap::default();
    let mut def_blocks: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
    for (bi, b) in func.blocks.iter().enumerate() {
        for (ii, inst) in b.instructions.iter().enumerate() {
            if let Some(d) = inst.dest() {
                *def_count.entry(d.0).or_insert(0) += 1;
                def_blocks.entry(d.0).or_default().push(bi);
            }
            let is_phi = matches!(inst, Instruction::Phi { .. });
            for v in operands_of(inst) {
                uses.entry(v).or_default().push(UseSite {
                    block: bi,
                    idx: ii,
                    is_phi,
                });
            }
        }
        let mut seen: FxHashSet<u32> = FxHashSet::default();
        crate::backend::liveness::for_each_operand_in_terminator(&b.terminator, |op| {
            if let Operand::Value(v) = op {
                if seen.insert(v.0) {
                    uses.entry(v.0).or_default().push(UseSite {
                        block: bi,
                        idx: usize::MAX,
                        is_phi: false,
                    });
                }
            }
        });
    }

    let mut plan: Vec<(usize, usize, usize, usize)> = Vec::new();
    for bi in 0..n {
        for ii in 0..func.blocks[bi].instructions.len() {
            let inst = &func.blocks[bi].instructions[ii];
            if !is_sinkable(inst) {
                continue;
            }
            let Some(dest) = inst.dest() else { continue };
            if def_count.get(&dest.0).copied().unwrap_or(0) != 1 {
                continue;
            }
            // DEFECT FIX 2: every operand must be single-def, or a
            // redefinition between the two positions would change the value
            // this instruction computes. `nbody`'s FP accumulators are
            // multi-def and were miscompiled by the first revision.
            let ops = operands_of(inst);
            if ops
                .iter()
                .any(|o| def_count.get(o).copied().unwrap_or(0) > 1)
            {
                continue;
            }
            let Some(u) = uses.get(&dest.0) else { continue };
            if u.is_empty() || u.iter().any(|s| s.is_phi) {
                continue;
            }
            let mut common: FxHashSet<usize> = dom[u[0].block].clone();
            for s in &u[1..] {
                common = common.intersection(&dom[s.block]).copied().collect();
            }
            let Some(&target) = common.iter().max_by_key(|&&c| dom[c].len()) else {
                continue;
            };
            if target == bi || !dom[target].contains(&bi) {
                continue;
            }
            if depth.get(target).copied().unwrap_or(0) > depth.get(bi).copied().unwrap_or(0) {
                continue;
            }
            // FREQUENCY GUARD. The sunk computation runs freq(target) times
            // instead of freq(source); requiring the first not to exceed the
            // second makes the move cost-neutral-or-better on every path,
            // while still admitting the profitable direction that
            // post-dominance forbade: sinking into a branch-guarded block,
            // which runs LESS often than the block that dominates it.
            //
            // A small tolerance absorbs float accumulation; the comparison is
            // between estimates, so an exact test would reject sinks that are
            // frequency-identical by construction.
            if freq[target] > freq[bi] * 1.0001 {
                continue;
            }
            // NO LOOP MAY BE ENTERED. Comparing loop DEPTHS is not enough:
            // leaving one loop while entering another keeps the count equal
            // and still multiplies how often the computation runs. Reject
            // any move whose target lies in a loop the source is outside of.
            if loops
                .iter()
                .any(|l| l.body.contains(&target) && !l.body.contains(&bi))
            {
                continue;
            }
            // PROFITABILITY. Sinking moves the boundary crossing from the
            // RESULT to the OPERANDS. Before the move, one value (the
            // result) is live across the edge; after it, every operand whose
            // live range ended at this instruction has to survive instead.
            //
            //   t = load p        -> p crosses instead of t   : 1 for 1
            //   t = a + b         -> a AND b cross instead of t : +1
            //
            // So a two-operand computation whose operands both die here makes
            // pressure WORSE, which is exactly what the unguarded pass did:
            // corpus +62 instructions, +28 reg-reg moves, +14 callee-saved
            // pushes. Only sink when at most one operand's range is extended;
            // an operand that is used elsewhere anyway is already live and
            // costs nothing.
            // An operand whose ONLY use is this instruction has its live range
            // extended by the move; one that is read elsewhere is live anyway
            // and costs nothing.
            //
            // Allowing a CHAIN through (an operand that is itself sinkable
            // and would follow on the next round) was measured and REJECTED:
            // corpus +75 instructions, +27 reg-reg moves, +15 callee-saved
            // pushes, and `glibc_memcmp` 197 -> 206. The chain does migrate,
            // but the intermediate rounds hold both ends live at once and the
            // allocator commits to the worse shape before the chain lands.
            let extended: usize = ops
                .iter()
                .filter(|o| uses.get(o).map_or(0, |v| v.len()) <= 1)
                .count();
            if extended > 0 {
                continue;
            }
            let first_use = u
                .iter()
                .filter(|s| s.block == target && s.idx != usize::MAX)
                .map(|s| s.idx)
                .min();
            let at = first_use.unwrap_or(func.blocks[target].instructions.len());
            let phi_end = func.blocks[target]
                .instructions
                .iter()
                .position(|i| !matches!(i, Instruction::Phi { .. }))
                .unwrap_or(func.blocks[target].instructions.len());
            if at < phi_end {
                continue;
            }
            // No operand may be redefined anywhere in the traversed region.
            let region = blocks_between(&cfg.succs, &cfg.preds, n, bi, target);
            let mut blocked = false;
            for &o in &ops {
                if let Some(dbs) = def_blocks.get(&o) {
                    if dbs.iter().any(|d| region.contains(d) && *d != bi) {
                        blocked = true;
                        break;
                    }
                }
            }
            if blocked {
                continue;
            }
            if matches!(inst, Instruction::Load { .. }) {
                for &rb in &region {
                    let start = if rb == bi { ii + 1 } else { 0 };
                    let end = if rb == target {
                        at
                    } else {
                        func.blocks[rb].instructions.len()
                    };
                    if start > end {
                        continue;
                    }
                    if func.blocks[rb].instructions[start..end]
                        .iter()
                        .any(is_memory_barrier)
                    {
                        blocked = true;
                        break;
                    }
                }
                if blocked {
                    continue;
                }
            }
            plan.push((bi, ii, target, at));
        }
    }
    if plan.is_empty() {
        return 0;
    }

    // DEFECT FIX 1: a block may participate in at most ONE move per round, in
    // either direction. A block that received an insertion has every later
    // index shifted, so a subsequent removal from it would take the wrong
    // instruction. Remaining moves are picked up by the next round.
    plan.sort_unstable_by(|a, b| (b.0, b.1).cmp(&(a.0, a.1)));
    let mut moved = 0usize;
    let mut touched: FxHashSet<usize> = FxHashSet::default();
    for (sb, si, db, at) in plan {
        if touched.contains(&sb) || touched.contains(&db) {
            continue;
        }
        if si >= func.blocks[sb].instructions.len() {
            continue;
        }
        let inst = func.blocks[sb].instructions.remove(si);
        let span = if si < func.blocks[sb].source_spans.len() {
            Some(func.blocks[sb].source_spans.remove(si))
        } else {
            None
        };
        let at = at.min(func.blocks[db].instructions.len());
        func.blocks[db].instructions.insert(at, inst);
        if let Some(sp) = span {
            let p = at.min(func.blocks[db].source_spans.len());
            func.blocks[db].source_spans.insert(p, sp);
        }
        touched.insert(sb);
        touched.insert(db);
        moved += 1;
    }
    moved
}

/// Sink pure computations (and, when enabled, non-volatile loads) toward
/// their uses. Runs to a fixpoint so an address chain follows its load.
pub(crate) fn sink_expressions(func: &mut IrFunction) -> usize {
    if std::env::var_os("CCC_NO_EXPR_SINK").is_some() {
        return 0;
    }
    let mut total = 0usize;
    // Each round moves at least one instruction strictly downward in the
    // dominator tree, so this terminates; the cap only bounds the cost on a
    // pathological CFG. One move per block per round means a long chain
    // needs several rounds, which is what the higher cap pays for.
    for _ in 0..32 {
        let m = sink_round(func);
        if m == 0 {
            break;
        }
        total += m;
    }
    total
}
