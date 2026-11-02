//! GLA location vocabulary and register-pressure analysis — split from the former monolithic `location_alloc.rs`.
//! See the module docs in [`super`] for the full design and policy record.

use super::*;

/// Where a value's bits live during a piece.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Location {
    /// In some GPR; the downstream colorer picks which.
    Register,
    /// In the function's frame, in the per-value capture slot.
    Stack,
    /// Re-derived on demand by cloning a source-less definition.
    Rematerialize,
}

/// A half-open intra-block span `[start, end)` over local instruction points
/// (point `n` = the terminator) during which a logical value occupies
/// `location`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LocationPiece {
    pub(crate) vid: u32,
    pub(crate) block: usize,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) location: Location,
}

// ─────────────────────────────────────────────────────────────────────────────
// Eligibility
// ─────────────────────────────────────────────────────────────────────────────

/// Loop-depth frequency weight — the same currency the production allocator
/// uses (`spill_cost_at` / `priority`: 10^min(depth,4)).
pub(super) fn block_frequency(depth: u32) -> u64 {
    10u64.saturating_pow(depth.min(4))
}

// ─────────────────────────────────────────────────────────────────────────────
// Coalesced color classes and pressure points
// ─────────────────────────────────────────────────────────────────────────────

/// Union-find over SSA values that the production colorer coalesces into a
/// single register for free: φ results with their incoming operands (the
/// allocator's hole-aware φ coalescing; RA machinery must not be rebuilt
/// here). Counting each SSA name separately systematically over-states
/// pressure on loop-heavy code (every loop-carried value is a φ web) and is
/// what made pre-alloc splitting fire on functions the colorer actually
/// fits. Unioning φ webs can only UNDER-count pressure relative to the
/// colorer, which is the required fail-closed direction: when in doubt the
/// planner splits nothing. φ coalescing can fail in the colorer (it inserts
/// edge copies); treating it as always-coalesced is therefore a deliberately
/// conservative planning approximation, not an exact analysis.
pub(super) struct ColorClasses {
    pub(super) parent: FxHashMap<u32, u32>,
}

impl ColorClasses {
    pub(super) fn phi_webs(func: &IrFunction) -> Self {
        fn find_up(parent: &mut FxHashMap<u32, u32>, x: u32) -> u32 {
            let mut root = x;
            while let Some(&p) = parent.get(&root) {
                if p == root {
                    break;
                }
                root = p;
            }
            // path compression
            let mut cur = x;
            while let Some(&p) = parent.get(&cur) {
                if p == cur || p == root {
                    break;
                }
                parent.insert(cur, root);
                cur = p;
            }
            root
        }
        let mut parent: FxHashMap<u32, u32> = FxHashMap::default();
        let union = |parent: &mut FxHashMap<u32, u32>, a: u32, b: u32| {
            parent.entry(a).or_insert(a);
            parent.entry(b).or_insert(b);
            let ra = find_up(parent, a);
            let rb = find_up(parent, b);
            if ra != rb {
                // deterministic root: smaller id
                let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
                parent.insert(hi, lo);
            }
        };
        for b in &func.blocks {
            for inst in &b.instructions {
                if let Instruction::Phi { dest, incoming, .. } = inst {
                    for (op, _pred) in incoming {
                        if let Operand::Value(Value(src)) = op {
                            union(&mut parent, dest.0, *src);
                        }
                    }
                }
            }
        }
        ColorClasses { parent }
    }

    pub(super) fn root(&self, mut v: u32) -> u32 {
        while let Some(&p) = self.parent.get(&v) {
            if p == v {
                break;
            }
            v = p;
        }
        v
    }
}

/// Residency pressure per local point: number of coalesced color CLASSES of
/// `values` live at each point, built from hole-aware segments (a value with
/// a hole in this block is not counted across the hole).
pub(super) struct Pressure {
    pub(super) per_block: Vec<Vec<u32>>,
    /// `peak` of each row, kept current by every mutator (`relieve`, `cap`):
    /// the planner asks for a block's peak per candidate value, and a
    /// rescan of the row per question was quadratic in the block length.
    peaks: Vec<(usize, u32)>,
}

impl Pressure {
    pub(super) fn build(
        live: &LivenessResult,
        func: &IrFunction,
        values: &FxHashSet<u32>,
        classes: &ColorClasses,
    ) -> Self {
        // Bucket each eligible segment into the blocks it overlaps, once.
        // Block point ranges are disjoint and laid out in block order
        // (`LivenessResult::block_starts` is sorted and every point belongs
        // to exactly one block), so the first overlapped block is a binary
        // search on `block_ends` and the walk stops at the first block that
        // starts past the segment.  Scanning every segment for every block
        // instead cost O(blocks x segments), which dominated whole compiles
        // of long branch chains (gcc.c-torture/compile 20001226-1: ~40 s of
        // a 50 s -O0 compile).  Buckets keep segment order, so each point's
        // root list is built from the same pushes as before.
        let nblocks = func.blocks.len();
        let mut overlapping: Vec<Vec<usize>> = vec![Vec::new(); nblocks];
        for (si, iv) in live.segments.iter().enumerate() {
            if !values.contains(&iv.value_id) {
                continue;
            }
            let first = live.block_ends[..nblocks].partition_point(|&ge| ge < iv.start);
            for (bi, bucket) in overlapping.iter_mut().enumerate().skip(first) {
                if live.block_starts[bi] > iv.end {
                    break;
                }
                bucket.push(si);
            }
        }
        let mut per_block = Vec::with_capacity(nblocks);
        for (bi, b) in func.blocks.iter().enumerate() {
            let npts = b.instructions.len() + 1;
            // Roots resident at each point (duplicates deduped afterwards).
            let mut at_point: Vec<Vec<u32>> = (0..npts).map(|_| Vec::new()).collect();
            let gs = live.block_starts[bi];
            let ge = live.block_ends[bi];
            for &si in &overlapping[bi] {
                let iv = &live.segments[si];
                let lo = (iv.start.max(gs) - gs) as usize;
                let hi = (iv.end.min(ge) - gs) as usize;
                let root = classes.root(iv.value_id);
                for pts in at_point.iter_mut().take(hi + 1).skip(lo) {
                    pts.push(root);
                }
            }
            let mut row = Vec::with_capacity(npts);
            for mut pts in at_point {
                pts.sort_unstable();
                pts.dedup();
                row.push(pts.len() as u32);
            }
            per_block.push(row);
        }
        let peaks = per_block.iter().map(|row| Self::row_peak(row)).collect();
        Pressure { per_block, peaks }
    }

    /// Highest residency of `row` and its first point.
    fn row_peak(row: &[u32]) -> (usize, u32) {
        row.iter()
            .enumerate()
            .max_by_key(|&(i, &c)| (c, std::cmp::Reverse(i)))
            .map(|(i, &c)| (i, c))
            .unwrap_or((0, 0))
    }

    pub(super) fn peak(&self, bi: usize) -> (usize, u32) {
        debug_assert_eq!(self.peaks[bi], Self::row_peak(&self.per_block[bi]));
        self.peaks[bi]
    }

    /// Clamp block `bi`'s residency to `budget` (an un-splittable peak).
    pub(super) fn cap(&mut self, bi: usize, budget: u32) {
        for p in self.per_block[bi].iter_mut() {
            if *p > budget {
                *p = budget;
            }
        }
        self.peaks[bi] = Self::row_peak(&self.per_block[bi]);
    }

    /// Subtract one unit of residency over half-open `[from,to)`.
    pub(super) fn relieve(&mut self, bi: usize, from: usize, to: usize) {
        let row = &mut self.per_block[bi];
        let to = to.min(row.len());
        if from >= to {
            return;
        }
        for p in row.iter_mut().take(to).skip(from) {
            *p = p.saturating_sub(1);
        }
        self.peaks[bi] = Self::row_peak(row);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Plan vocabulary
// ─────────────────────────────────────────────────────────────────────────────

/// A planned stack gap: `vid` is not register-resident over the half-open
/// local span `[from,to)` of `block`; `to == n` propagates the memory state
/// across outgoing CFG edges (the materializer reconstructs a register name
/// at the next use, wherever it lives).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Gap {
    pub(super) vid: u32,
    pub(super) block: usize,
    pub(super) from: usize,
    pub(super) to: usize,
}

pub(super) struct Plan {
    pub(super) gaps: Vec<Gap>,
    pub(super) remats: Vec<u32>,
    /// (vid, block) where the block is ENTERED in the memory region (the
    /// value arrives via slot, reload happens at the first read cluster).
    pub(super) entry_memory: FxHashSet<(u32, usize)>,
}

impl Plan {
    pub(super) fn gaps_of(&self, vid: u32, bi: usize) -> Vec<(usize, usize)> {
        self.gaps
            .iter()
            .filter(|g| g.vid == vid && g.block == bi)
            .map(|g| (g.from, g.to))
            .collect()
    }
}

/// Closure of a terminal gap across CFG edges: every successor in which
/// `vid` remains live enters the memory region. BFS stops at blocks where
/// the value is re-defined (SSA: its φ-definition block is not live-in) and
/// records, per reached block, the local first-use point (reload point). A
/// live-through block (no local use) propagates onward.
pub(super) fn memory_closure(
    live: &LivenessResult,
    func: &IrFunction,
    ix: &UseIndex,
    succs: &analysis::FlatAdj,
    vid: u32,
    start_block: usize,
) -> Vec<(usize, usize)> {
    let mut visited: FxHashSet<usize> = FxHashSet::default();
    let mut stack = vec![start_block];
    let mut reached = Vec::new();
    while let Some(bi) = stack.pop() {
        for &s in succs.row(bi) {
            let s = s as usize;
            if !visited.insert(s) {
                continue;
            }
            // A (re-)definition of vid joins a new incarnation; the memory
            // region neither enters nor flows through that block.
            if ix.defines(s, vid) {
                continue;
            }
            if !live.is_live_in(s, vid) {
                continue;
            }
            let uses = ix.points(s, vid);
            let first = uses
                .first()
                .copied()
                .unwrap_or_else(|| func.blocks[s].instructions.len());
            reached.push((s, first));
            if uses.is_empty() {
                stack.push(s);
            }
        }
    }
    reached
}

/// Weighted count of dynamic read points (use-density currency), including
/// edge φ reads.
///
/// Sums over the blocks that read `vid` or feed it to a successor φ only
/// (the index lists them): every other block contributes zero.  Saturating
/// addition of non-negative terms is order-independent.
pub(super) fn weighted_use_count(ix: &UseIndex, vid: u32, depths: &[u32]) -> u64 {
    let mut w = 0u64;
    for &bi in ix.use_blocks(vid) {
        let bi = bi as usize;
        w = w.saturating_add(ix.points(bi, vid).len() as u64 * block_frequency(depths[bi]));
    }
    for &bi in ix.feed_blocks(vid) {
        w = w.saturating_add(block_frequency(depths[bi as usize]));
    }
    w
}

/// Local point of `vid`'s definition in block `bi`, if any (φ defs anchor
/// at the first non-φ point — the position past the φ prefix).
pub(super) fn local_def_point(
    ix: &UseIndex,
    func: &IrFunction,
    bi: usize,
    vid: u32,
) -> Option<usize> {
    ix.def(vid).and_then(|(db, di, is_phi)| {
        if db != bi {
            return None;
        }
        Some(if is_phi {
            first_non_phi(&func.blocks[bi])
        } else {
            di
        })
    })
}

/// Half-open local spans of block `bi` over which rematerializing `vid`
/// gives NO net register relief:
///
/// * the single def point (the original source-less definition still
///   executes and occupies a register there);
/// * each resident segment's first-to-last read span, inclusive — one
///   clone result renames every use of that contiguous run and therefore
///   stays live across the whole run, holding a register just like the
///   original value did.
///
/// What DOES get relieved is everything else: the residency carried into
/// a block ahead of its first read (loop-preheader hoisting case: the
/// adler32 table base) and any hole between runs / after the last read
/// before the value was formerly carried out to a successor.
pub(super) fn remat_no_relief_spans(
    ix: &UseIndex,
    func: &IrFunction,
    bi: usize,
    vid: u32,
    lo: usize,
    hi: usize,
) -> Vec<(usize, usize)> {
    let reads = ix.points(bi, vid);
    let mut spans: Vec<(usize, usize)> = reads
        .iter()
        .filter(|&&p| p >= lo && p <= hi)
        .take(1)
        .flat_map(|&first| {
            let last = reads
                .iter()
                .rev()
                .copied()
                .find(|&p| p >= lo && p <= hi)
                .unwrap_or(first);
            std::iter::once((first.min(last), last.max(first) + 1))
        })
        .collect();
    if let Some(d) = local_def_point(ix, func, bi, vid) {
        if d >= lo && d <= hi {
            spans.push((d, d + 1));
        }
    }
    spans
}

#[inline]
pub(super) fn point_in_spans(p: usize, spans: &[(usize, usize)]) -> bool {
    spans.iter().any(|&(s, e)| p >= s && p < e)
}

/// Whether rematerializing `vid` gives NET register relief at a REACHABLE
/// over-budget point (excess no greater than [`reach_band`]). Points grossly
/// over budget cannot be made colorable by editing this value and do not
/// qualify it.
///
/// Net relief is stricter than mere residency. Full rematerialization keeps
/// the original source-less definition (it still occupies a register at its
/// def point) and emits one clone at every use cluster (the clone result
/// occupies a register at that point). The value is only actually removed
/// from the residency curve at points it neither is defined nor read —
/// crediting a def/use point would book register freedom the edit does not
/// provide. This is the nbody -Os discriminator: a 1-2-use global whose
/// only reachable over-budget points are its own use clusters tipped the
/// planner but left generated code with zero fewer stack references and
/// five extra instructions.
///
/// Only the blocks `vid`'s own segments overlap can qualify, so the walk
/// visits exactly those (block ranges are disjoint and ordered: the first is
/// a binary search on `block_ends`), and the per-block gate is the cached
/// peak.  Scanning every block and every segment, plus a whole-row
/// relievability pre-scan (a pure shortcut of the point loop below), per
/// candidate value was quadratic in the function size.  The verdict is an
/// existential over the same (block, segment, point) triples, so the visit
/// order does not matter.
pub(super) fn covers_over_budget_point(
    live: &LivenessResult,
    ix: &UseIndex,
    func: &IrFunction,
    pressure: &Pressure,
    vid: u32,
    policy: &GlaPolicy,
) -> bool {
    let budget = policy.budget;
    let relievable = |c: u32| c as usize > budget && policy.peak_is_reachable(c);
    // Whether segment `iv`'s residency in block `bi` reaches a relievable
    // point.
    let covers_in_block = |bi: usize, iv: &crate::backend::liveness::LiveInterval| {
        // Block-level gate: a block whose PEAK is beyond the reach band
        // cannot be made colorable by editing this (or any handful of)
        // value(s); crediting its reachable buildup points would only
        // perturb the production allocator's folding/callee-save plan
        // (the nbody cascade). Relief is credited only in blocks the
        // edit can plausibly tip under budget.
        let (_, block_peak) = pressure.peak(bi);
        if block_peak as usize <= budget || !policy.peak_is_reachable(block_peak) {
            return false;
        }
        let row = &pressure.per_block[bi];
        let gs = live.block_starts[bi];
        let ge = live.block_ends[bi];
        let lo = (iv.start.max(gs) - gs) as usize;
        let hi = ((iv.end.min(ge) - gs) as usize).min(row.len() - 1);
        // Speed tier: the clone re-occupies a register over the def
        // point and each contiguous read run, so only the carried
        // residency outside those spans is net relief. Debug tier
        // (-O0): values are stack-homed regardless, so even use-point
        // remats eliminate slot store/reload traffic; credit the full
        // resident span.
        let no_relief = if policy.net_relief() {
            remat_no_relief_spans(ix, func, bi, vid, lo, hi)
        } else {
            Vec::new()
        };
        (lo..=hi).any(|p| !point_in_spans(p, &no_relief) && relievable(row[p]))
    };
    let nblocks = pressure.per_block.len();
    for iv in ix.segments(live, vid) {
        let first = live.block_ends[..nblocks].partition_point(|&ge| ge < iv.start);
        for bi in first..nblocks {
            if live.block_starts[bi] > iv.end {
                break;
            }
            if covers_in_block(bi, iv) {
                return true;
            }
        }
    }
    false
}

pub(super) fn crosses_call(live: &LivenessResult, bi: usize, from: usize, to: usize) -> bool {
    let gs = live.block_starts[bi];
    for p in from..to {
        if live.call_points.binary_search(&(gs + p as u32)).is_ok() {
            return true;
        }
    }
    false
}
