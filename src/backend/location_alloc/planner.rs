//! GLA eligibility analysis and plan construction — split from the former monolithic `location_alloc.rs`.
//! See the module docs in [`super`] for the full design and policy record.

use super::*;

/// Values a sound Phase-1 splitter must not touch: register-class owners
/// (intrinsics, inline asm), allocas themselves, and x86 fixed-register
/// operands (div/rem pair, variable shift count) whose post-split coloring is
/// governed by dedicated allocator machinery.
pub(super) fn collect_ineligible(func: &IrFunction) -> FxHashSet<u32> {
    let mut bad: FxHashSet<u32> = FxHashSet::default();
    for b in &func.blocks {
        for inst in &b.instructions {
            match inst {
                Instruction::Alloca { dest, .. } | Instruction::DynAlloca { dest, .. } => {
                    bad.insert(dest.0);
                }
                Instruction::Intrinsic { .. } | Instruction::InlineAsm { .. } => {
                    if let Some(d) = inst.dest() {
                        bad.insert(d.0);
                    }
                    crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                        if let Operand::Value(v) = op {
                            bad.insert(v.0);
                        }
                    });
                    crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                        bad.insert(v.0);
                    });
                }
                // Div/rem incarnations are fused as (numerator, denominator,
                // signedness, barriers); splitting an operand mints a fresh
                // incarnation the fusion stamps cannot match, and on i686 the
                // %eax/%edx contract is a hard pin. Exclude both operands.
                Instruction::BinOp {
                    op: IrBinOp::UDiv | IrBinOp::SDiv | IrBinOp::URem | IrBinOp::SRem,
                    lhs,
                    rhs,
                    ..
                } => {
                    for op in [lhs, rhs] {
                        if let Operand::Value(v) = op {
                            bad.insert(v.0);
                        }
                    }
                }
                // A non-constant shift count must be in %cl on x86 / the
                // equivalent fixed register elsewhere.
                Instruction::BinOp {
                    op: IrBinOp::Shl | IrBinOp::LShr | IrBinOp::AShr,
                    rhs: Operand::Value(v),
                    ..
                } => {
                    bad.insert(v.0);
                }
                _ => {}
            }
        }
    }
    bad
}

/// Recurrence-carried header φ destinations. Such a web re-enters its header
/// on a back edge carrying a member of the same web (the a..h sha256
/// schedule, every accumulator reduction). An anchor store on its
/// definition executes on the carried chain every iteration; the production
/// allocator already has measured, protected policy for these
/// (`span_recurrence`), and a memory piece here is never an improvement.
pub(super) fn collect_recurrence_phi_dests(
    func: &IrFunction,
    label_map: &FxHashMap<BlockId, usize>,
    preds: &analysis::FlatAdj,
    succs: &analysis::FlatAdj,
    idom: &[usize],
) -> FxHashSet<u32> {
    let mut out = FxHashSet::default();
    let loops = loop_analysis::find_natural_loops(func.blocks.len(), preds, succs, idom);
    for lp in &loops {
        let h = lp.header;
        for inst in &func.blocks[h].instructions {
            let Instruction::Phi { dest, incoming, .. } = inst else {
                continue;
            };
            let recurrent = incoming.iter().any(|(op, pred)| {
                let Some(&pred_idx) = label_map.get(pred) else {
                    return false;
                };
                lp.body.contains(&pred_idx)
                    && matches!(op, Operand::Value(v) if defined_in_loop(func, v.0, &lp.body))
            });
            if recurrent {
                out.insert(dest.0);
            }
        }
    }
    out
}

pub(super) fn defined_in_loop(func: &IrFunction, vid: u32, body: &FxHashSet<usize>) -> bool {
    func.blocks
        .iter()
        .enumerate()
        .filter(|(bi, _)| body.contains(bi))
        .any(|(_, b)| {
            b.instructions
                .iter()
                .any(|i| i.dest().is_some_and(|d| d.0 == vid))
        })
}

/// A definition cheaply reproducible without register history. Returns a
/// fresh template carrying `dest` when every input is static.
pub(super) fn rematerializable_template(inst: &Instruction) -> Option<Instruction> {
    match inst {
        Instruction::GlobalAddr { dest, name } => Some(Instruction::GlobalAddr {
            dest: *dest,
            name: name.clone(),
        }),
        Instruction::Copy {
            dest,
            src: src @ Operand::Const(_),
        } => Some(Instruction::Copy {
            dest: *dest,
            src: src.clone(),
        }),
        _ => None,
    }
}

/// Live regions of `vid` that the remat segment cap counts: its
/// hole-aware segments, except a segment lying wholly inside a block's
/// leading φ group and ending AT a φ that `vid` feeds. That piece is the
/// φ's own edge read (all φs of a block read their incomings in parallel
/// at its head), kept in the liveness so a copy phi elimination places at
/// the φ (`target_copies`) stays covered; it is not a separate region the
/// value is resident across. Counting it made every φ-incoming remat look
/// two-segment and fall to the cap once φ incomings stopped being
/// (over-approximately) live into the φ block.
pub(super) fn live_region_count(
    live: &LivenessResult,
    func: &IrFunction,
    ix: &UseIndex,
    vid: u32,
) -> usize {
    let nblocks = func.blocks.len();
    let feeds = |inst: Option<&Instruction>| {
        matches!(
            inst,
            Some(Instruction::Phi { incoming, .. })
                if incoming.iter().any(|(op, _)| matches!(op, Operand::Value(v) if v.0 == vid))
        )
    };
    ix.segments(live, vid)
        .filter(|iv| {
            let bi = live.block_ends[..nblocks].partition_point(|&ge| ge < iv.start);
            let Some(b) = func.blocks.get(bi) else {
                return true;
            };
            if iv.end > live.block_ends[bi] {
                return true;
            }
            let lo = (iv.start - live.block_starts[bi]) as usize;
            let hi = (iv.end - live.block_starts[bi]) as usize;
            let in_phi_group =
                (lo..=hi).all(|i| matches!(b.instructions.get(i), Some(Instruction::Phi { .. })));
            !(in_phi_group && feeds(b.instructions.get(hi)))
        })
        .count()
}

/// Locate a value's unique static definition: (block, instruction index,
/// true when the def is a φ at that block's head).
pub(super) fn find_def(func: &IrFunction, vid: u32) -> Option<(usize, usize, bool)> {
    for (bi, b) in func.blocks.iter().enumerate() {
        for (ii, inst) in b.instructions.iter().enumerate() {
            if inst.dest().is_some_and(|d| d.0 == vid) {
                return Some((bi, ii, matches!(inst, Instruction::Phi { .. })));
            }
        }
    }
    None
}

/// Whether `vid` is (re-)defined inside block `bi`. The memory-region
/// closure must never propagate INTO such a block: a reload before a
/// definition's capture anchor would read a stale slot. Exact SSA liveness
/// never reports a value live-in to its own def block (φ incomings are
/// live only on their edges), so this is defence in depth against any
/// conservative live-in -- a liveness over-approximation is safe for
/// spilling but not for memory-state propagation.
///
/// Reference scan behind [`UseIndex::defines`] (pinned by
/// `use_index_matches_the_scans`).
#[cfg(test)]
pub(super) fn block_defines_value(func: &IrFunction, bi: usize, vid: u32) -> bool {
    func.blocks[bi]
        .instructions
        .iter()
        .any(|i| i.dest().is_some_and(|d| d.0 == vid))
}

/// Sorted, de-duplicated intra-block READ points of `vid` (local instruction
/// indices; the terminator point is `n`). φ operands are EDGE reads at the
/// predecessor and are intentionally excluded. Folded hidden reads (SIB/GEP
/// bases with no IR operand) are folded in so a value the addressing mode
/// re-reads cannot be evicted across the access.
pub(super) fn block_use_points(
    live: &LivenessResult,
    func: &IrFunction,
    bi: usize,
    vid: u32,
) -> Vec<usize> {
    let block = &func.blocks[bi];
    let n = block.instructions.len();
    let mut pts = Vec::new();
    let mut seen = FxHashSet::default();
    for (ii, inst) in block.instructions.iter().enumerate() {
        if matches!(inst, Instruction::Phi { .. }) {
            continue;
        }
        let mut hits = false;
        crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
            if matches!(op, Operand::Value(v) if v.0 == vid) {
                hits = true;
            }
        });
        crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
            if v.0 == vid {
                hits = true;
            }
        });
        if hits && seen.insert(ii) {
            pts.push(ii);
        }
    }
    if terminator_uses_value(&block.terminator, vid) && seen.insert(n) {
        pts.push(n);
    }
    // Folded reads at global points: map to local points in this block.
    // `gp >= gs` is mandatory — a saturating_sub would map a read in an
    // EARLIER block to phantom local point 0 (inside the φ prefix).
    let gs = live.block_starts[bi];
    let ge = live.block_ends[bi];
    if let Some(folded) = live.folded_read_points.get(&vid) {
        for &gp in folded {
            if gp < gs || gp > ge {
                continue;
            }
            let local = (gp - gs) as usize;
            if local <= n && seen.insert(local) {
                pts.push(local);
            }
        }
    }
    pts.sort_unstable();
    pts.dedup();
    pts
}

/// Whether `vid` is an incoming operand of some φ in successor block `s`, on
/// block `bi`'s edge (pred label match).
pub(super) fn phi_incoming_uses(func: &IrFunction, s: usize, bi: usize, vid: u32) -> bool {
    let pred_label = func.blocks[bi].label;
    func.blocks[s].instructions.iter().any(|i| match i {
        Instruction::Phi { incoming, .. } => incoming
            .iter()
            .any(|(op, p)| *p == pred_label && matches!(op, Operand::Value(v) if v.0 == vid)),
        _ => false,
    })
}

/// Whether `vid` reaches ANY successor φ from block `bi`.
///
/// Reference scan behind [`UseIndex::feeds_phi`] (pinned by
/// `use_index_matches_the_scans`).
#[cfg(test)]
pub(super) fn block_feeds_successor_phi(func: &IrFunction, bi: usize, vid: u32) -> bool {
    func.blocks
        .iter()
        .enumerate()
        .any(|(s, _)| phi_incoming_uses(func, s, bi, vid))
}

/// Per-plan index of the facts candidate evaluation asks per (block, value).
///
/// `plan_function` sees `func` and `live` immutably, so one build answers
/// every query of the plan.  Each accessor returns exactly what its scanning
/// counterpart computes -- [`block_use_points`], [`block_feeds_successor_phi`],
/// [`find_def`], [`block_defines_value`] and the `live.segments` filter by
/// value -- which the unit test `use_index_matches_the_scans` pins over every
/// (block, value) pair.  The scans cost O(block) or O(function) per query and
/// were asked per value per block: quadratic in the instruction count, and
/// cubic in the block count for the φ feed (a 4000-call straight-line `main`
/// spent most of its -O2 compile in them).  The scans stay for the
/// materializer, which edits `func` and asks at most `max_splits` times.
pub(super) struct UseIndex {
    /// `(block, value)` -> `block_use_points`.
    points: FxHashMap<(u32, u32), Vec<usize>>,
    /// value -> ascending blocks with a non-empty `points` entry.
    use_blocks: FxHashMap<u32, Vec<u32>>,
    /// `(pred block, value)` pairs for which `block_feeds_successor_phi`.
    feeds: FxHashSet<(u32, u32)>,
    /// value -> ascending pred blocks of `feeds`.
    feed_blocks: FxHashMap<u32, Vec<u32>>,
    /// value -> `find_def` (the first definition in block order).
    defs: FxHashMap<u32, (usize, usize, bool)>,
    /// `(block, value)` pairs for which `block_defines_value`.
    def_blocks: FxHashSet<(u32, u32)>,
    /// value -> indices of its `live.segments`, in segment order.
    segments: FxHashMap<u32, Vec<usize>>,
}

impl UseIndex {
    pub(super) fn build(live: &LivenessResult, func: &IrFunction) -> Self {
        let mut points: FxHashMap<(u32, u32), Vec<usize>> = FxHashMap::default();
        let mut defs: FxHashMap<u32, (usize, usize, bool)> = FxHashMap::default();
        let mut def_blocks: FxHashSet<(u32, u32)> = FxHashSet::default();
        let mut feeds: FxHashSet<(u32, u32)> = FxHashSet::default();
        // `phi_incoming_uses` matches an incoming edge by label equality.
        let mut label_blocks: FxHashMap<BlockId, Vec<u32>> = FxHashMap::default();
        for (bi, b) in func.blocks.iter().enumerate() {
            label_blocks.entry(b.label).or_default().push(bi as u32);
        }
        for (bi, block) in func.blocks.iter().enumerate() {
            let b = bi as u32;
            for (ii, inst) in block.instructions.iter().enumerate() {
                if let Some(d) = inst.dest() {
                    defs.entry(d.0)
                        .or_insert((bi, ii, matches!(inst, Instruction::Phi { .. })));
                    def_blocks.insert((b, d.0));
                }
                if let Instruction::Phi { incoming, .. } = inst {
                    for (op, pred) in incoming {
                        if let (Operand::Value(v), Some(preds)) = (op, label_blocks.get(pred)) {
                            for &pb in preds {
                                feeds.insert((pb, v.0));
                            }
                        }
                    }
                    continue; // φ operands are edge reads, not block reads
                }
                let mut push = |v: u32| {
                    let pts = points.entry((b, v)).or_default();
                    if pts.last() != Some(&ii) {
                        pts.push(ii);
                    }
                };
                crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                    if let Operand::Value(v) = op {
                        push(v.0);
                    }
                });
                crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| push(v.0));
            }
            let n = block.instructions.len();
            crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
                if let Operand::Value(v) = op {
                    let pts = points.entry((b, v.0)).or_default();
                    if pts.last() != Some(&n) {
                        pts.push(n);
                    }
                }
            });
        }
        // Folded hidden reads: block point ranges are disjoint and ordered,
        // so the one block containing a global point is a binary search
        // (the scan's `gs <= gp <= ge` filter, per block).
        for (&vid, folded) in &live.folded_read_points {
            for &gp in folded {
                let Some(bi) = live.block_index_at(gp) else {
                    continue;
                };
                if bi >= func.blocks.len() {
                    continue;
                }
                let local = (gp - live.block_starts[bi]) as usize;
                if local <= func.blocks[bi].instructions.len() {
                    points.entry((bi as u32, vid)).or_default().push(local);
                }
            }
        }
        let mut use_blocks: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
        for (&(b, v), pts) in points.iter_mut() {
            pts.sort_unstable();
            pts.dedup();
            use_blocks.entry(v).or_default().push(b);
        }
        for blocks in use_blocks.values_mut() {
            blocks.sort_unstable();
        }
        let mut feed_blocks: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
        for &(b, v) in &feeds {
            feed_blocks.entry(v).or_default().push(b);
        }
        for blocks in feed_blocks.values_mut() {
            blocks.sort_unstable();
        }
        let mut segments: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
        for (si, iv) in live.segments.iter().enumerate() {
            segments.entry(iv.value_id).or_default().push(si);
        }
        UseIndex {
            points,
            use_blocks,
            feeds,
            feed_blocks,
            defs,
            def_blocks,
            segments,
        }
    }

    /// [`block_use_points`]`(live, func, bi, vid)`.
    pub(super) fn points(&self, bi: usize, vid: u32) -> &[usize] {
        self.points
            .get(&(bi as u32, vid))
            .map_or(&[], Vec::as_slice)
    }

    /// Blocks in which `vid` has a read point, ascending.
    pub(super) fn use_blocks(&self, vid: u32) -> &[u32] {
        self.use_blocks.get(&vid).map_or(&[], Vec::as_slice)
    }

    /// [`block_feeds_successor_phi`]`(func, bi, vid)`.
    pub(super) fn feeds_phi(&self, bi: usize, vid: u32) -> bool {
        self.feeds.contains(&(bi as u32, vid))
    }

    /// Blocks `bi` for which [`Self::feeds_phi`] holds, ascending.
    pub(super) fn feed_blocks(&self, vid: u32) -> &[u32] {
        self.feed_blocks.get(&vid).map_or(&[], Vec::as_slice)
    }

    /// [`find_def`]`(func, vid)`.
    pub(super) fn def(&self, vid: u32) -> Option<(usize, usize, bool)> {
        self.defs.get(&vid).copied()
    }

    /// [`block_defines_value`]`(func, bi, vid)`.
    pub(super) fn defines(&self, bi: usize, vid: u32) -> bool {
        self.def_blocks.contains(&(bi as u32, vid))
    }

    /// `live.segments` of `vid`, in segment order.
    pub(super) fn segments<'a>(
        &'a self,
        live: &'a LivenessResult,
        vid: u32,
    ) -> impl Iterator<Item = &'a crate::backend::liveness::LiveInterval> + 'a {
        self.segments
            .get(&vid)
            .into_iter()
            .flatten()
            .map(move |&si| &live.segments[si])
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Candidate evaluation and plan construction
// ─────────────────────────────────────────────────────────────────────────────

pub(super) struct Candidate {
    pub(super) vid: u32,
    pub(super) from: usize,
    pub(super) to: usize,
    pub(super) closure: Vec<(usize, usize)>,
    pub(super) benefit: u64,
    pub(super) cost: u64,
    pub(super) distance: u64,
}

impl Candidate {
    /// Better eviction: larger benefit, then farther next use (Belady),
    /// then lower cost, then smaller vid (determinism).
    pub(super) fn beats(&self, other: &Candidate) -> bool {
        (
            self.benefit,
            self.distance,
            u64::MAX - self.cost,
            u32::MAX - self.vid,
        ) > (
            other.benefit,
            other.distance,
            u64::MAX - other.cost,
            u32::MAX - other.vid,
        )
    }
}

/// Greedy global planner: rematerializations first (zero store cost), then
/// hottest-pressure-first Belady-MIN spill gaps across the whole CFG,
/// bounded at every step by the fail-closed [`GlaPolicy`].
pub(super) fn plan_function(
    live: &LivenessResult,
    func: &IrFunction,
    succs: &analysis::FlatAdj,
    eligible: &FxHashSet<u32>,
    remattable: &FxHashSet<u32>,
    policy: &GlaPolicy,
) -> Plan {
    let budget = policy.budget;
    let max_splits = policy.max_splits;
    let depths = &live.block_loop_depth;
    let classes = ColorClasses::phi_webs(func);
    let mut pressure = Pressure::build(live, func, eligible, &classes);
    let ix = UseIndex::build(live, func);
    if split_debug_enabled() && std::env::var("CCC_GLA_TRACE").is_ok() {
        // The derived policy, printed whether or not any block exceeds the
        // budget: the per-target reach band is a planning input, and its
        // gates must observe it even when exact liveness leaves a function
        // under budget everywhere.
        eprintln!(
            "[GLA] {}: policy budget={} reach={} max_splits={}",
            func.name, budget, policy.reach_band, max_splits
        );
        for (bi, b) in func.blocks.iter().enumerate() {
            let (pt, pk) = pressure.peak(bi);
            if pk as usize > budget {
                eprintln!(
                    "[GLA] {}: block{} depth{} peak {}/{} reach={} at point {}/{}",
                    func.name,
                    bi,
                    depths[bi],
                    pk,
                    budget,
                    policy.reach_band,
                    pt,
                    b.instructions.len()
                );
            }
        }
    }
    let mut gaps: Vec<Gap> = Vec::new();
    let mut remats: Vec<u32> = Vec::new();
    let mut entry_memory: FxHashSet<(u32, usize)> = FxHashSet::default();
    let mut used = 0usize;
    let types = collect_value_types(func);

    // ── Full rematerialization ──
    let mut rc: Vec<u32> = remattable.iter().copied().collect();
    rc.sort_unstable();
    for vid in rc {
        if used >= max_splits {
            break;
        }
        let wuses = weighted_use_count(&ix, vid, depths);
        let nseg = live_region_count(live, func, &ix, vid);
        let covers = covers_over_budget_point(live, &ix, func, &pressure, vid, policy);
        if split_debug_enabled() && std::env::var("CCC_GLA_TRACE").is_ok() {
            let ranges: Vec<String> = live
                .segments
                .iter()
                .filter(|iv| iv.value_id == vid)
                .map(|iv| {
                    let sb = live
                        .block_starts
                        .iter()
                        .rposition(|&s| s <= iv.start)
                        .unwrap_or(0);
                    let eb = live
                        .block_starts
                        .iter()
                        .rposition(|&s| s <= iv.end)
                        .unwrap_or(0);
                    format!("b{}..b{}[{}-{}]", sb, eb, iv.start, iv.end)
                })
                .collect();
            eprintln!(
                "[GLA] {}: remat-candidate v{} weighted_uses={} segments={} covers_reachable={} {}",
                func.name,
                vid,
                wuses,
                nseg,
                covers,
                ranges.join(",")
            );
        }
        if wuses > policy.remat_max_weight {
            continue;
        }
        if nseg > policy.remat_max_segments {
            if split_debug_enabled() && std::env::var("CCC_GLA_TRACE").is_ok() {
                eprintln!(
                    "[GLA] {}: remat-candidate v{} rejected: {} segments > {}",
                    func.name, vid, nseg, policy.remat_max_segments
                );
            }
            continue;
        }
        if !covers {
            continue;
        }
        if !types.get(&vid).copied().is_some_and(is_simple_gpr_type) {
            continue;
        }
        for bi in 0..func.blocks.len() {
            let gs = live.block_starts[bi];
            let ge = live.block_ends[bi];
            let row_len = pressure.per_block[bi].len();
            for iv in ix.segments(live, vid) {
                if iv.end < gs || iv.start > ge {
                    continue;
                }
                let lo = (iv.start.max(gs) - gs) as usize;
                let hi = ((iv.end.min(ge) - gs) as usize).min(row_len - 1);
                let no_relief = if policy.net_relief() {
                    remat_no_relief_spans(&ix, func, bi, vid, lo, hi)
                } else {
                    Vec::new()
                };
                let mut p = lo;
                while p <= hi {
                    if point_in_spans(p, &no_relief) {
                        p += 1;
                        continue;
                    }
                    // Run of consecutive relieved points -> one call.
                    let mut q = p;
                    while q <= hi && !point_in_spans(q, &no_relief) {
                        q += 1;
                    }
                    pressure.relieve(bi, p, q);
                    p = q;
                }
            }
        }
        remats.push(vid);
        used += 1;
        // The per-value applied line is emitted once, with its kind, in
        // `run_with_policy` after planning; do not duplicate it here.
    }

    // ── Spill gaps ──
    //
    // Net-negative in Phase-1 A/B without allocation feedback (see
    // `allow_spill_gaps`); the whole fixpoint is skipped unless enabled.
    if !policy.allow_spill_gaps {
        return Plan {
            gaps: Vec::new(),
            remats,
            entry_memory: Default::default(),
        };
    }
    let mut done_block: FxHashSet<(u32, usize)> = FxHashSet::default();
    loop {
        if used >= max_splits {
            break;
        }
        let mut target: Option<(u64, usize)> = None; // (weighted excess, block)
        for bi in 0..func.blocks.len() {
            let (_pt, pk) = pressure.peak(bi);
            if pk as usize <= budget || !policy.peak_is_reachable(pk) {
                // At or under budget: nothing to gain. Grossly over budget:
                // cannot be made colorable by a handful of edits; leave it
                // entirely to the production allocator (folding + callee-
                // saved buys). Editing such blocks only adds traffic.
                continue;
            }
            let score = block_frequency(depths[bi]) * (pk as usize - budget) as u64;
            match target {
                Some((s, b)) if (score, std::cmp::Reverse(bi)) <= (s, std::cmp::Reverse(b)) => {}
                _ => target = Some((score, bi)),
            }
        }
        let Some((_, bi)) = target else { break };
        let n = func.blocks[bi].instructions.len();

        let mut best: Option<Candidate> = None;
        // The candidate scan does not edit the pressure: one peak serves it.
        let peak_pt = pressure.peak(bi).0;
        for &vid in eligible.iter() {
            if remats.contains(&vid) || done_block.contains(&(vid, bi)) {
                continue;
            }
            if !live.is_live_at(vid, live.block_starts[bi] + peak_pt as u32) {
                continue;
            }
            let uses = ix.points(bi, vid);
            if uses.contains(&peak_pt) {
                continue; // read at the peak: bits required now
            }
            let def_local = ix.def(vid).filter(|(db, _, _)| *db == bi);
            if let Some((_, di, false)) = def_local {
                if di == peak_pt {
                    continue; // defined at the peak
                }
            }

            // Gap start: just after the last read ≤ peak, else just past the
            // local def, else block entry (memory arrives on the edge).
            let mut from = uses
                .iter()
                .copied()
                .rfind(|&u| u <= peak_pt)
                .map(|u| u + 1)
                .unwrap_or(0);
            if let Some((_, di, is_phi)) = def_local {
                let floor = if is_phi {
                    first_non_phi(&func.blocks[bi])
                } else {
                    di + 1
                };
                from = from.max(floor);
            }
            // Gap end: first read after the peak in-block, else terminator
            // (cross-block closure).
            let next_in_block = uses.iter().copied().find(|&u| u > peak_pt);
            let to = next_in_block.unwrap_or(n);
            if from >= to {
                continue;
            }
            if let Some((_, di, is_phi)) = def_local {
                let floor = if is_phi {
                    first_non_phi(&func.blocks[bi])
                } else {
                    di + 1
                };
                if to <= floor {
                    continue;
                }
            }
            if let Some(t) = next_in_block {
                if t - from < policy.min_gap && !crosses_call(live, bi, from, t) {
                    continue;
                }
                // Same-block register piece on both sides of the gap:
                // intra-block pre-alloc spilling, which is net-negative
                // against the production allocator's folded spills
                // (see RA-06 decision record). Fail closed unless the A/B
                // override explicitly asks for them.
                if !policy.allow_intra_block_gaps {
                    continue;
                }
            }

            let mut closure: Vec<(usize, usize)> = Vec::new();
            if next_in_block.is_none() && live.is_live_out(bi, vid) {
                closure = memory_closure(live, func, &ix, succs, vid, bi);
                if closure.is_empty() && !ix.feeds_phi(bi, vid) {
                    continue; // value dies here: the colorer drops it
                }
            }

            // Benefit: weighted residency relieved at REACHABLE over-budget
            // points over the local gap and closure prefixes. Grossly-over
            // blocks stay with the production allocator and credit nothing.
            let creditable = |c: u32| c as usize > budget && policy.peak_is_reachable(c);
            let mut benefit = 0u64;
            for p in from..to {
                if pressure.per_block[bi]
                    .get(p)
                    .copied()
                    .is_some_and(&creditable)
                {
                    benefit = benefit.saturating_add(block_frequency(depths[bi]));
                }
            }
            for &(cb, first) in &closure {
                for p in 0..first {
                    if pressure.per_block[cb]
                        .get(p)
                        .copied()
                        .is_some_and(&creditable)
                    {
                        benefit = benefit.saturating_add(block_frequency(depths[cb]));
                    }
                }
            }
            if benefit < policy.min_benefit {
                continue;
            }

            // Cost: one immutable capture store (def-block frequency) plus
            // weighted reload clusters.
            let Some((def_block, _, _)) = ix.def(vid) else {
                continue;
            };
            let mut cost = block_frequency(depths[def_block]);
            if next_in_block.is_some() {
                cost = cost.saturating_add(block_frequency(depths[bi]));
            }
            for &(cb, first) in &closure {
                let ncb = func.blocks[cb].instructions.len();
                let has_use = first < ncb
                    || !ix.points(cb, vid).is_empty()
                    || terminator_uses_value(&func.blocks[cb].terminator, vid)
                    || ix.feeds_phi(cb, vid);
                if has_use {
                    cost = cost.saturating_add(block_frequency(depths[cb]));
                }
            }
            if next_in_block.is_none()
                && ix.feeds_phi(bi, vid)
                && !closure
                    .iter()
                    .any(|(cb, _)| succs.row(bi).iter().any(|s| *s as usize == *cb))
            {
                cost = cost.saturating_add(block_frequency(depths[bi]));
            }

            if benefit.saturating_mul(10) < cost.saturating_mul(policy.benefit_ratio10) {
                continue;
            }

            let distance = next_in_block
                .map(|u| (u - from) as u64)
                .unwrap_or(u64::MAX - block_frequency(depths[bi]));

            let cand = Candidate {
                vid,
                from,
                to,
                closure,
                benefit,
                cost,
                distance,
            };
            if best.as_ref().is_none_or(|b| cand.beats(b)) {
                best = Some(cand);
            }
        }

        let Some(cand) = best else {
            // Relieve the scan of this un-splittable peak so the fixed-point
            // loop targets genuinely solvable peaks instead of spinning.
            pressure.cap(bi, budget as u32);
            continue;
        };

        gaps.push(Gap {
            vid: cand.vid,
            block: bi,
            from: cand.from,
            to: cand.to,
        });
        done_block.insert((cand.vid, bi));
        pressure.relieve(bi, cand.from, cand.to);
        // The seed block is ENTERED in the memory region only when the gap
        // begins at block entry (a value arriving live-in on the edge). A
        // mid-block gap leaves the entry region register-resident; marking
        // it entry-memory would schedule a reload before an early read,
        // potentially ahead of the capture store itself.
        if cand.from == 0 {
            entry_memory.insert((cand.vid, bi));
        }
        for &(cb, first) in &cand.closure {
            entry_memory.insert((cand.vid, cb));
            if first > 0 {
                pressure.relieve(cb, 0, first);
                gaps.push(Gap {
                    vid: cand.vid,
                    block: cb,
                    from: 0,
                    to: first,
                });
                done_block.insert((cand.vid, cb));
            }
        }
        used += 1;
        if split_debug_enabled() {
            eprintln!(
                "[GLA] {}: gap v{} block{} [{},{}) benefit={} cost={} closure={}",
                func.name,
                cand.vid,
                bi,
                cand.from,
                cand.to,
                cand.benefit,
                cand.cost,
                cand.closure.len()
            );
        }
    }

    Plan {
        gaps,
        remats,
        entry_memory,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Planning entry points
// ─────────────────────────────────────────────────────────────────────────────

/// Public entry point. Policy comes from the A/B environment and the
/// compilation's target/optimization tier. Returns the count of logical
/// values rewritten (spilled or rematerialized), 0 when nothing applied.
/// `tls_symbols`: the module's thread-local symbols (their `GlobalAddr`
/// is a multi-instruction thread-pointer computation, see
/// [`GlaPolicy::remat_affordable`]).
pub(crate) fn run(
    func: &mut IrFunction,
    max_splits: usize,
    opt_level: u32,
    tls_symbols: &FxHashSet<String>,
) -> usize {
    if func.blocks.is_empty() || max_splits == 0 {
        return 0;
    }
    run_planned(
        func,
        GlaPolicy::from_env(max_splits, opt_level),
        tls_symbols,
    )
}

/// Entry point with an explicit policy (unit tests use this so their
/// expectations never depend on process-wide environment knobs).
#[cfg(test)]
pub(super) fn run_with_policy(func: &mut IrFunction, policy: GlaPolicy) -> usize {
    run_planned(func, policy, &FxHashSet::default())
}

fn run_planned(func: &mut IrFunction, policy: GlaPolicy, tls_symbols: &FxHashSet<String>) -> usize {
    let max_splits = policy.max_splits;
    if func.blocks.is_empty() || max_splits == 0 || !policy.enabled() {
        return 0;
    }

    let label_map = analysis::build_label_map(func);
    let (preds, succs) = analysis::build_cfg(func, &label_map);
    let idom = analysis::compute_dominators(func.blocks.len(), &preds, &succs);
    let live = crate::backend::liveness::compute_live_intervals(func);
    let all_types = collect_value_types(func);

    let mut ineligible = collect_ineligible(func);
    for v in collect_recurrence_phi_dests(func, &label_map, &preds, &succs, &idom) {
        ineligible.insert(v);
    }
    let eligible: FxHashSet<u32> = all_types
        .iter()
        .filter(|(v, t)| is_simple_gpr_type(**t) && !ineligible.contains(v))
        .map(|(v, _)| *v)
        .collect();

    let mut remattable: FxHashSet<u32> = FxHashSet::default();
    let mut templates: FxHashMap<u32, Instruction> = FxHashMap::default();
    for b in &func.blocks {
        for inst in &b.instructions {
            if let Some(d) = inst.dest() {
                if eligible.contains(&d.0) {
                    if let Some(t) = rematerializable_template(inst)
                        .filter(|t| policy.remat_affordable(t, tls_symbols))
                    {
                        remattable.insert(d.0);
                        templates.insert(d.0, t);
                    }
                }
            }
        }
    }

    let plan = plan_function(&live, func, &succs, &eligible, &remattable, &policy);
    if split_debug_enabled() {
        for vid in &plan.remats {
            let kind = templates
                .get(vid)
                .map(|t| match t {
                    Instruction::GlobalAddr { .. } => "globaladdr",
                    Instruction::Copy {
                        src: Operand::Const(_),
                        ..
                    } => "const",
                    _ => "other",
                })
                .unwrap_or("?");
            eprintln!("[GLA] {}: remat v{} ({})", func.name, vid, kind);
        }
    }
    if plan.gaps.is_empty() && plan.remats.is_empty() {
        return 0;
    }
    materialize(func, &live, &succs, plan, &all_types, &templates)
}
