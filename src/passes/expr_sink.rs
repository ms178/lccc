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
//! into BB16 while leaving `p1 = gep src1, 1` behind merely trades one live
//! value for another. A fixpoint of single-instruction moves is worse: it
//! temporarily makes both ends live, which lets register allocation commit to
//! a spill shape before the feeder follows.
//!
//! `OPT-42` therefore moves a maximal single-use feeder chain atomically.
//! Crucially, it does **not** judge the move by a one-edge crossing count.
//! It compares before/after hole-aware live ranges from the backend's own
//! liveness analysis, weighted by static block frequency, and requires a
//! strict Pareto improvement in total live range length, call crossings, and
//! peak pressure. This catches the old failure mode: an external input can be
//! made live one instruction earlier than the root result was, even when both
//! values cross the same CFG edge.
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

/// The atomic OP-42 transaction.  `indices` are source-block instruction
/// indices in program order; they need not be contiguous because every
/// selected feeder is independently pure and has exactly one in-chain use.
struct ChainPlan {
    source: usize,
    target: usize,
    insert_at: usize,
    indices: Vec<usize>,
    /// Chain destinations plus external operands.  These are the values whose
    /// liveness can change when the transaction moves.
    affected_values: FxHashSet<u32>,
    /// Earliest selected source instruction that reads each external value.
    /// Any redefinition after that point and before insertion changes the
    /// value observed by the moved chain.
    external_first_use: FxHashMap<u32, usize>,
}

/// A deliberately parameter-free live-range cost vector.
///
/// The dimensions are compared lexicographically as a Pareto relation rather
/// than folded into an invented "one call equals N instructions" scalar:
/// a move may not make *any* dimension worse, and must make at least one
/// dimension strictly better.  That makes this a guard derived from actual
/// liveness, not a target-specific tuning constant.
#[derive(Clone, Copy, Debug)]
struct LiveRangeCost {
    weighted_points: f64,
    weighted_call_crossings: f64,
    peak_pressure: usize,
}

fn chain_sink_enabled() -> bool {
    std::env::var_os("CCC_NO_EXPR_CHAIN_SINK").is_none()
}

/// Research-only A/B override.  It bypasses profitability, never safety; it
/// exists so the cost model can be falsified against the same corpus.
fn force_chain_sink() -> bool {
    std::env::var_os("CCC_FORCE_EXPR_CHAIN_SINK").is_some()
}

fn trace_chain_sink() -> bool {
    std::env::var_os("CCC_TRACE_EXPR_CHAIN_SINK").is_some()
}

/// Every block that can be visited after `from` and before the *first* arrival
/// at `to`.  Unlike `blocks_between`, this deliberately does not expand `to`.
/// A loop latch reached only after the target must not look like an intervening
/// redefinition: the moved computation has already executed by then.
fn blocks_before_first_target(
    succs: &analysis::FlatAdj,
    preds: &analysis::FlatAdj,
    n: usize,
    from: usize,
    to: usize,
) -> FxHashSet<usize> {
    let mut forward = FxHashSet::default();
    let mut stack = vec![from];
    while let Some(block) = stack.pop() {
        if !forward.insert(block) || block == to {
            continue;
        }
        for &succ in succs.row(block) {
            let succ = succ as usize;
            if succ < n {
                stack.push(succ);
            }
        }
    }

    let mut backward = FxHashSet::default();
    let mut stack = vec![to];
    while let Some(block) = stack.pop() {
        if !backward.insert(block) {
            continue;
        }
        for &pred in preds.row(block) {
            let pred = pred as usize;
            if pred < n {
                stack.push(pred);
            }
        }
    }
    forward.intersection(&backward).copied().collect()
}

/// Destination block and insertion point for a root whose value is used below
/// its defining block.  This is intentionally the same frequency/loop-entry
/// legality policy as scalar expression sinking.
fn root_sink_target(
    func: &IrFunction,
    source: usize,
    root_idx: usize,
    uses: &FxHashMap<u32, Vec<UseSite>>,
    def_count: &FxHashMap<u32, u32>,
    dom: &[FxHashSet<usize>],
    depth: &[usize],
    freq: &[f64],
    loops: &[crate::passes::loop_analysis::NaturalLoop],
) -> Option<(u32, usize, usize)> {
    let inst = func.blocks.get(source)?.instructions.get(root_idx)?;
    if !is_sinkable(inst) {
        return None;
    }
    let dest = inst.dest()?;
    if def_count.get(&dest.0).copied().unwrap_or(0) != 1 {
        return None;
    }
    let root_uses = uses.get(&dest.0)?;
    if root_uses.is_empty() || root_uses.iter().any(|site| site.is_phi) {
        return None;
    }

    let mut common = dom[root_uses[0].block].clone();
    for site in &root_uses[1..] {
        common = common.intersection(&dom[site.block]).copied().collect();
    }
    let &target = common.iter().max_by_key(|&&block| dom[block].len())?;
    if target == source || !dom[target].contains(&source) {
        return None;
    }
    if depth.get(target).copied().unwrap_or(0) > depth.get(source).copied().unwrap_or(0) {
        return None;
    }
    // Dominance alone does not exclude entering a different loop at equal
    // nesting depth; doing so was the measured expat regression.
    if loops
        .iter()
        .any(|loop_| loop_.body.contains(&target) && !loop_.body.contains(&source))
    {
        return None;
    }
    if freq[target] > freq[source] * 1.0001 {
        return None;
    }

    let first_use = root_uses
        .iter()
        .filter(|site| site.block == target && site.idx != usize::MAX)
        .map(|site| site.idx)
        .min()
        .unwrap_or(func.blocks[target].instructions.len());
    let phi_end = func.blocks[target]
        .instructions
        .iter()
        .position(|inst| !matches!(inst, Instruction::Phi { .. }))
        .unwrap_or(func.blocks[target].instructions.len());
    if first_use < phi_end {
        return None;
    }
    Some((dest.0, target, first_use))
}

/// Collect the maximal tree of same-block, unique-definition feeders whose
/// sole use is their in-chain consumer.  We deliberately do not pull a value
/// with two consumers: moving it would either leave a use above the target or
/// turn a local chain into a multi-block code-motion problem.
fn collect_single_use_chain(
    func: &IrFunction,
    source: usize,
    root_idx: usize,
    uses: &FxHashMap<u32, Vec<UseSite>>,
    def_count: &FxHashMap<u32, u32>,
    def_sites: &FxHashMap<u32, Vec<(usize, usize)>>,
) -> Option<Vec<usize>> {
    let mut selected = FxHashSet::default();
    let mut work = vec![root_idx];
    selected.insert(root_idx);

    while let Some(consumer_idx) = work.pop() {
        let consumer = &func.blocks[source].instructions[consumer_idx];
        for operand in operands_of(consumer) {
            let Some(defs) = def_sites.get(&operand) else {
                continue;
            };
            if defs.len() != 1 || def_count.get(&operand).copied().unwrap_or(0) != 1 {
                continue;
            }
            let (def_block, def_idx) = defs[0];
            if def_block != source || def_idx >= consumer_idx || selected.contains(&def_idx) {
                continue;
            }
            let feeder = &func.blocks[source].instructions[def_idx];
            if !is_sinkable(feeder) {
                continue;
            }
            let Some(feeder_uses) = uses.get(&operand) else {
                continue;
            };
            if feeder_uses.len() != 1
                || feeder_uses[0].is_phi
                || feeder_uses[0].block != source
                || feeder_uses[0].idx != consumer_idx
            {
                continue;
            }
            selected.insert(def_idx);
            work.push(def_idx);
        }
    }

    if selected.len() < 2 {
        return None;
    }
    let mut indices: Vec<usize> = selected.into_iter().collect();
    indices.sort_unstable();
    Some(indices)
}

fn make_chain_plan(
    func: &IrFunction,
    source: usize,
    root_idx: usize,
    uses: &FxHashMap<u32, Vec<UseSite>>,
    def_count: &FxHashMap<u32, u32>,
    def_sites: &FxHashMap<u32, Vec<(usize, usize)>>,
    dom: &[FxHashSet<usize>],
    depth: &[usize],
    freq: &[f64],
    loops: &[crate::passes::loop_analysis::NaturalLoop],
) -> Option<ChainPlan> {
    let (_, target, insert_at) = root_sink_target(
        func, source, root_idx, uses, def_count, dom, depth, freq, loops,
    )?;
    let indices = collect_single_use_chain(func, source, root_idx, uses, def_count, def_sites)?;

    // Source locations are a parallel array only in debug builds.  Never
    // create a mismatched mapping merely to take an optimization.
    let source_spans = &func.blocks[source].source_spans;
    let target_spans = &func.blocks[target].source_spans;
    let spans_compatible = (source_spans.is_empty() && target_spans.is_empty())
        || (source_spans.len() == func.blocks[source].instructions.len()
            && target_spans.len() == func.blocks[target].instructions.len());
    if !spans_compatible {
        return None;
    }

    let mut chain_values = FxHashSet::default();
    for &idx in &indices {
        let dest = func.blocks[source].instructions[idx].dest()?;
        chain_values.insert(dest.0);
    }

    let mut external_first_use = FxHashMap::default();
    for &idx in &indices {
        for operand in operands_of(&func.blocks[source].instructions[idx]) {
            if !chain_values.contains(&operand) {
                external_first_use
                    .entry(operand)
                    .and_modify(|first: &mut usize| *first = (*first).min(idx))
                    .or_insert(idx);
            }
        }
    }
    let mut affected_values = chain_values;
    affected_values.extend(external_first_use.keys().copied());

    Some(ChainPlan {
        source,
        target,
        insert_at,
        indices,
        affected_values,
        external_first_use,
    })
}

#[inline]
fn defines_value(inst: &Instruction, value: u32) -> bool {
    inst.dest().map(|dest| dest.0 == value).unwrap_or(false)
        || matches!(inst, Instruction::InlineAsm { outputs, .. }
            if outputs.iter().any(|(_, output, _)| output.0 == value))
}

/// A multi-definition value is safe to carry with the chain only when its
/// reaching definition cannot change before the new insertion point.  This is
/// strictly stronger than "the source dominates the target" and deliberately
/// more precise than the old global-single-def ban: a loop latch *after* the
/// target is not crossed and therefore cannot alter this iteration's value.
fn external_values_are_stable(
    func: &IrFunction,
    plan: &ChainPlan,
    region: &FxHashSet<usize>,
) -> bool {
    for (&value, &first_use) in &plan.external_first_use {
        // In the source block, every instruction after the earliest chain use
        // runs before the new target position.  This catches a same-block
        // reassignment, including one hidden between non-contiguous feeders.
        if func.blocks[plan.source]
            .instructions
            .iter()
            .skip(first_use.saturating_add(1))
            .any(|inst| defines_value(inst, value))
        {
            return false;
        }
        // At the target, insertion is before instruction `insert_at`.
        if func.blocks[plan.target].instructions[..plan.insert_at]
            .iter()
            .any(|inst| defines_value(inst, value))
        {
            return false;
        }
        for &block in region {
            if block == plan.source || block == plan.target {
                continue;
            }
            if func.blocks[block]
                .instructions
                .iter()
                .any(|inst| defines_value(inst, value))
            {
                return false;
            }
        }
    }
    true
}

/// A moved non-volatile load must not cross a write/call/atomic/opaque-asm
/// barrier.  The chain may contain several loads; each receives its own source
/// suffix check because the first one crosses more same-block text than the
/// last one.
fn chain_loads_can_move(func: &IrFunction, plan: &ChainPlan, region: &FxHashSet<usize>) -> bool {
    for &idx in &plan.indices {
        if !matches!(
            func.blocks[plan.source].instructions[idx],
            Instruction::Load { .. }
        ) {
            continue;
        }
        if func.blocks[plan.source]
            .instructions
            .iter()
            .skip(idx.saturating_add(1))
            .any(is_memory_barrier)
        {
            return false;
        }
        if func.blocks[plan.target].instructions[..plan.insert_at]
            .iter()
            .any(is_memory_barrier)
        {
            return false;
        }
        for &block in region {
            if block == plan.source || block == plan.target {
                continue;
            }
            if func.blocks[block]
                .instructions
                .iter()
                .any(is_memory_barrier)
            {
                return false;
            }
        }
    }
    true
}

/// Move a validated chain while keeping `instructions` and `source_spans`
/// exactly parallel.  Callers retain two block snapshots if they need to undo
/// the speculative move after evaluating its liveness cost.
fn move_chain(func: &mut IrFunction, plan: &ChainPlan) {
    debug_assert_ne!(plan.source, plan.target);
    debug_assert!(plan.indices.windows(2).all(|pair| pair[0] < pair[1]));
    let track_spans = !func.blocks[plan.source].source_spans.is_empty();
    let mut moved = Vec::with_capacity(plan.indices.len());
    {
        let source = &mut func.blocks[plan.source];
        for &idx in plan.indices.iter().rev() {
            let inst = source.instructions.remove(idx);
            let span = if track_spans {
                Some(source.source_spans.remove(idx))
            } else {
                None
            };
            moved.push((inst, span));
        }
    }
    moved.reverse();

    let target = &mut func.blocks[plan.target];
    let at = plan.insert_at.min(target.instructions.len());
    for (offset, (inst, span)) in moved.into_iter().enumerate() {
        target.instructions.insert(at + offset, inst);
        if let Some(span) = span {
            target.source_spans.insert(at + offset, span);
        }
    }
}

/// Sum hole-aware live range length in dynamic program-point units.  A segment
/// is charged only for program points in blocks where it is live; the block's
/// static frequency gives a loop/branch-sensitive cost without pretending an
/// unprofiled branch has a known bias.
fn weighted_live_length(
    live: &crate::backend::liveness::LivenessResult,
    values: &FxHashSet<u32>,
    frequency: &[f64],
) -> f64 {
    let mut total = 0.0;
    for segment in &live.segments {
        if !values.contains(&segment.value_id) {
            continue;
        }
        for block in 0..live.block_starts.len() {
            let start = segment.start.max(live.block_starts[block]);
            let end = segment.end.min(live.block_ends[block]);
            if start <= end {
                total += (end - start + 1) as f64 * frequency.get(block).copied().unwrap_or(0.0);
            }
        }
    }
    total
}

/// Count dynamic caller-clobber crossings for the affected values.  A call is
/// a discrete pressure cliff rather than an arbitrarily weighted instruction:
/// the Pareto guard below rejects any newly introduced crossing outright.
fn weighted_call_crossings(
    live: &crate::backend::liveness::LivenessResult,
    values: &FxHashSet<u32>,
    frequency: &[f64],
) -> f64 {
    let mut total = 0.0;
    for segment in &live.segments {
        if !values.contains(&segment.value_id) {
            continue;
        }
        for &call in &live.call_points {
            if segment.start < call && call < segment.end {
                let weight = live
                    .block_index_at(call)
                    .and_then(|block| frequency.get(block).copied())
                    .unwrap_or(0.0);
                total += weight;
            }
        }
    }
    total
}

/// The maximum number of simultaneously live hole-aware segments, across all
/// values.  Unlike a boundary crossing count, this sees pressure introduced at
/// the target by unrelated values which were already live there.
fn peak_live_pressure(live: &crate::backend::liveness::LivenessResult) -> usize {
    let mut events = Vec::with_capacity(live.segments.len() * 2);
    for segment in &live.segments {
        events.push((segment.start, 1i32));
        events.push((segment.end.saturating_add(1), -1i32));
    }
    events.sort_unstable_by_key(|event| event.0);
    let mut current = 0i32;
    let mut peak = 0i32;
    let mut i = 0usize;
    while i < events.len() {
        let point = events[i].0;
        while i < events.len() && events[i].0 == point {
            current += events[i].1;
            i += 1;
        }
        peak = peak.max(current);
    }
    peak.max(0) as usize
}

fn live_range_cost(
    live: &crate::backend::liveness::LivenessResult,
    affected_values: &FxHashSet<u32>,
    frequency: &[f64],
) -> LiveRangeCost {
    LiveRangeCost {
        weighted_points: weighted_live_length(live, affected_values, frequency),
        weighted_call_crossings: weighted_call_crossings(live, affected_values, frequency),
        peak_pressure: peak_live_pressure(live),
    }
}

/// Parameter-free Pareto acceptance.  Floating-point values are sums of small
/// non-negative integers times powers of ten; this tolerance only absorbs
/// summation order, never a real program-point difference.
fn strictly_improves_live_range(before: LiveRangeCost, after: LiveRangeCost) -> bool {
    const EPSILON: f64 = 1e-9;
    let points_not_worse = after.weighted_points <= before.weighted_points + EPSILON;
    let calls_not_worse = after.weighted_call_crossings <= before.weighted_call_crossings + EPSILON;
    let peak_not_worse = after.peak_pressure <= before.peak_pressure;
    let strictly_better = after.weighted_points + EPSILON < before.weighted_points
        || after.weighted_call_crossings + EPSILON < before.weighted_call_crossings
        || after.peak_pressure < before.peak_pressure;
    points_not_worse && calls_not_worse && peak_not_worse && strictly_better
}

/// Try at most one atomic chain per round.  The accepted mutation invalidates
/// every def/use index and program point, so returning immediately is both
/// simpler and safer; the outer expression-sink fixpoint rebuilds all facts.
#[allow(clippy::too_many_arguments)]
fn try_sink_atomic_chain(
    func: &mut IrFunction,
    cfg: &analysis::CfgAnalysis,
    dom: &[FxHashSet<usize>],
    loops: &[crate::passes::loop_analysis::NaturalLoop],
    depth: &[usize],
    frequency: &[f64],
    uses: &FxHashMap<u32, Vec<UseSite>>,
    def_count: &FxHashMap<u32, u32>,
    def_sites: &FxHashMap<u32, Vec<(usize, usize)>>,
) -> bool {
    if !chain_sink_enabled() {
        return false;
    }
    // Most functions have no eligible feeder chain. Defer the O(IR) liveness
    // construction until one has passed all cheap legality checks, keeping the
    // scalar-only fast path as cheap as it was before OP-42.
    let mut before_live = None;
    for source in 0..func.blocks.len() {
        let instruction_count = func.blocks[source].instructions.len();
        for root_idx in 0..instruction_count {
            let Some(plan) = make_chain_plan(
                func, source, root_idx, uses, def_count, def_sites, dom, depth, frequency, loops,
            ) else {
                continue;
            };
            let region = blocks_before_first_target(
                &cfg.succs,
                &cfg.preds,
                func.blocks.len(),
                plan.source,
                plan.target,
            );
            if !external_values_are_stable(func, &plan, &region)
                || !chain_loads_can_move(func, &plan, &region)
            {
                continue;
            }

            let before = {
                let live = before_live
                    .get_or_insert_with(|| crate::backend::liveness::compute_live_intervals(func));
                live_range_cost(live, &plan.affected_values, frequency)
            };
            // The model is evaluated on the real backend liveness, not a
            // hand-maintained delta formula.  Preserve exact source/debug
            // mappings until a strict profitability proof accepts the move.
            let source_backup = func.blocks[plan.source].clone();
            let target_backup = func.blocks[plan.target].clone();
            move_chain(func, &plan);
            let after_live = crate::backend::liveness::compute_live_intervals(func);
            let after = live_range_cost(&after_live, &plan.affected_values, frequency);
            let accept = force_chain_sink() || strictly_improves_live_range(before, after);
            if trace_chain_sink() {
                eprintln!(
                    "[expr-chain] {} b{}:{} -> b{} ({} insts, {} values): points {:.1}->{:.1}, calls {:.1}->{:.1}, peak {}->{}",
                    if accept { "accept" } else { "reject" },
                    plan.source,
                    root_idx,
                    plan.target,
                    plan.indices.len(),
                    plan.affected_values.len(),
                    before.weighted_points,
                    after.weighted_points,
                    before.weighted_call_crossings,
                    after.weighted_call_crossings,
                    before.peak_pressure,
                    after.peak_pressure,
                );
            }
            if accept {
                return true;
            }
            func.blocks[plan.source] = source_backup;
            func.blocks[plan.target] = target_backup;
        }
    }
    false
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
    // Exact sites are needed by OP-42's atomic feeder collector.  Keeping
    // them alongside the legacy block map avoids rediscovering definitions
    // with a second full walk of the function.
    let mut def_sites: FxHashMap<u32, Vec<(usize, usize)>> = FxHashMap::default();
    for (bi, b) in func.blocks.iter().enumerate() {
        for (ii, inst) in b.instructions.iter().enumerate() {
            if let Some(d) = inst.dest() {
                *def_count.entry(d.0).or_insert(0) += 1;
                def_blocks.entry(d.0).or_default().push(bi);
                def_sites.entry(d.0).or_default().push((bi, ii));
            }
            // InlineAsm outputs are definitions despite `dest()` being None.
            // Treating one as use-only would let an atomic chain cross an asm
            // output that reassigns one of its external operands.
            if let Instruction::InlineAsm { outputs, .. } = inst {
                for (_, output, _) in outputs {
                    *def_count.entry(output.0).or_insert(0) += 1;
                    def_blocks.entry(output.0).or_default().push(bi);
                    def_sites.entry(output.0).or_default().push((bi, ii));
                }
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

    // OP-42 has to be attempted before scalar planning: a candidate with a
    // single-use feeder is intentionally rejected by the scalar `extended`
    // guard below, but can be profitable when its whole chain moves together.
    // One accepted transaction ends this round so every index/liveness fact is
    // rebuilt before another chain is considered.
    if try_sink_atomic_chain(
        func, &cfg, &dom, &loops, &depth, &freq, &uses, &def_count, &def_sites,
    ) {
        return 1;
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
/// their uses. Each round first considers one profitability-proven atomic
/// feeder chain, then applies the established scalar sink plan; rebuilding
/// facts between rounds keeps both transformations index- and liveness-safe.
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

#[cfg(test)]
mod tests {
    //! The source-level memcmp shape is valuable integration coverage, but it
    //! cannot reliably retain every IR detail after the surrounding pipeline.
    //! These focused IR tests pin the actual OP-42 legality/profitability
    //! contracts: the old one-edge model accepted the second shape below.

    use super::*;
    use crate::common::types::IrType;
    use crate::ir::reexports::BasicBlock;

    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: Vec::new(),
        }
    }

    fn function(blocks: Vec<BasicBlock>) -> IrFunction {
        let mut function =
            IrFunction::new("expr_chain_test".into(), IrType::Void, Vec::new(), false);
        function.next_value_id = 16;
        function.blocks = blocks;
        function
    }

    fn param(dest: u32, param_idx: usize) -> Instruction {
        Instruction::ParamRef {
            dest: Value(dest),
            param_idx,
            ty: IrType::Ptr,
        }
    }

    fn gep(dest: u32, base: u32, offset: i64) -> Instruction {
        Instruction::GetElementPtr {
            dest: Value(dest),
            base: Value(base),
            offset: Operand::Const(IrConst::I64(offset)),
            ty: IrType::Ptr,
        }
    }

    fn load(dest: u32, ptr: u32) -> Instruction {
        Instruction::Load {
            dest: Value(dest),
            ptr: Value(ptr),
            ty: IrType::U64,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }

    fn compare_loaded(dest: u32, value: u32) -> Instruction {
        Instruction::Cmp {
            dest: Value(dest),
            op: IrCmpOp::Ne,
            lhs: Operand::Value(Value(value)),
            rhs: Operand::Const(IrConst::I64(0)),
            ty: IrType::U64,
        }
    }

    fn has_dest(block: &BasicBlock, value: u32) -> bool {
        block
            .instructions
            .iter()
            .any(|inst| inst.dest().map(|dest| dest.0 == value).unwrap_or(false))
    }

    #[test]
    fn atomic_chain_sinks_when_the_external_input_is_already_live() {
        // `base` is independently needed in the target.  Moving p/load frees
        // `loaded` across the edge without lengthening base's live range, so
        // all three cost dimensions improve-or-hold and the transaction lands.
        let mut function = function(vec![
            block(
                0,
                vec![param(0, 0), gep(2, 0, 8), load(3, 2)],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![compare_loaded(4, 3), gep(5, 0, 16)],
                Terminator::Return(None),
            ),
        ]);

        assert!(sink_expressions(&mut function) > 0);
        assert!(!has_dest(&function.blocks[0], 2));
        assert!(!has_dest(&function.blocks[0], 3));
        assert!(matches!(
            function.blocks[1].instructions[0],
            Instruction::GetElementPtr { dest: Value(2), .. }
        ));
        assert!(matches!(
            function.blocks[1].instructions[1],
            Instruction::Load { dest: Value(3), .. }
        ));
    }

    #[test]
    fn boundary_neutral_chain_is_rejected_by_live_range_length() {
        // At the source/target edge, one-edge accounting says "loaded stops
        // crossing, base starts crossing": a tie.  It misses that base starts
        // at the GEP, one instruction before loaded exists.  The intermediate
        // block makes that positive length delta unmistakable; the strict
        // Pareto model must leave the chain in block 0.
        let mut function = function(vec![
            block(
                0,
                vec![param(0, 0), gep(2, 0, 8), load(3, 2)],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![Instruction::Copy {
                    dest: Value(6),
                    src: Operand::Const(IrConst::I64(7)),
                }],
                Terminator::Branch(BlockId(2)),
            ),
            block(2, vec![compare_loaded(4, 3)], Terminator::Return(None)),
        ]);

        let changes = sink_expressions(&mut function);
        assert_eq!(
            changes, 0,
            "a boundary-neutral but longer chain must not move"
        );
        assert!(has_dest(&function.blocks[0], 2));
        assert!(has_dest(&function.blocks[0], 3));
    }

    #[test]
    fn loop_latch_redefinition_after_target_is_not_treated_as_intervening() {
        // Value 0 has two definitions: entry and latch.  The latch executes
        // only *after* target block 1, so the GEP/load in this iteration sees
        // exactly the same reaching definition before and after sinking.  The
        // old global-single-def rule rejected this useful loop-carried shape.
        let mut function = function(vec![
            block(
                0,
                vec![param(0, 0), param(1, 1), gep(2, 0, 8), load(3, 2)],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![compare_loaded(4, 3), gep(5, 0, 16)],
                Terminator::Branch(BlockId(2)),
            ),
            block(
                2,
                vec![Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Value(Value(1)),
                }],
                Terminator::Branch(BlockId(0)),
            ),
        ]);

        assert!(sink_expressions(&mut function) > 0);
        assert!(!has_dest(&function.blocks[0], 2));
        assert!(!has_dest(&function.blocks[0], 3));
    }

    fn asm_output(dest: u32) -> Instruction {
        Instruction::InlineAsm {
            template: String::new(),
            outputs: vec![("=r".into(), Value(dest), None)],
            inputs: Vec::new(),
            clobbers: Vec::new(),
            operand_types: vec![IrType::Ptr],
            goto_labels: Vec::new(),
            input_symbols: Vec::new(),
            seg_overrides: vec![AddressSpace::Default],
        }
    }

    #[test]
    fn redefinition_before_target_rejects_the_atomic_transaction() {
        // Here the same multi-def value is reassigned in the source suffix.
        // The original GEP observes value 0 before the copy; a moved GEP would
        // observe it after.  This must be rejected even though the liveness
        // cost vector would otherwise look attractive.
        let mut function = function(vec![
            block(
                0,
                vec![
                    param(0, 0),
                    param(1, 1),
                    gep(2, 0, 8),
                    load(3, 2),
                    Instruction::Copy {
                        dest: Value(0),
                        src: Operand::Value(Value(1)),
                    },
                ],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![compare_loaded(4, 3), gep(5, 0, 16)],
                Terminator::Return(None),
            ),
        ]);

        let changes = sink_expressions(&mut function);
        assert_eq!(changes, 0, "reaching-definition change must block sinking");
        assert!(has_dest(&function.blocks[0], 2));
        assert!(has_dest(&function.blocks[0], 3));
    }

    #[test]
    fn inline_asm_output_redefinition_rejects_the_atomic_transaction() {
        // `dest()` intentionally returns None for inline assembly, even
        // though an output is a definition.  The metadata scan must record it
        // so a pure GEP/load chain cannot be moved past a reassignment of its
        // external base.
        let mut function = function(vec![
            block(
                0,
                vec![param(0, 0), gep(2, 0, 8), load(3, 2), asm_output(0)],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![compare_loaded(4, 3), gep(5, 0, 16)],
                Terminator::Return(None),
            ),
        ]);

        assert_eq!(sink_expressions(&mut function), 0);
        assert!(has_dest(&function.blocks[0], 2));
        assert!(has_dest(&function.blocks[0], 3));
    }
}
