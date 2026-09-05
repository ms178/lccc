//! Linear scan register allocator data structures.
//!
//! Core types:
//! - [`LiveRange`]: live interval plus uses, loop depth, priority, hints, cascade
//! - [`ActiveInterval`]: a range that currently occupies a physical register
//! - [`LinearScanAllocator`]: Poletto–Sarkar scan with loop-aware eviction
//!
//! # Invariants
//!
//! - Program points are dense: one per instruction, then one per terminator,
//!   in block order. [`build_live_ranges`] and [`pgo_point_weights`] share
//!   this numbering.
//! - `LiveRange.uses` is sorted ascending and unique after
//!   [`build_live_ranges`]. [`next_use_after`] / [`LinearScanAllocator::future_uses`]
//!   binary-search it.
//! - SSA use-before-def: every operand use at point `p` precedes the single
//!   definition at `p`. Die-at-birth sharing (`a.end == b.start` and `a`'s
//!   last *recorded* use is at `a.end`) is therefore sound.
//! - The rotation path in [`LinearScanAllocator::find_free_register`] never
//!   applies die-at-birth sharing. Only the hint path does, and only for
//!   emitters that compute into the destination with the operand pre-loaded.
//!   Applying it to every GPR pair produced `or %r9,%r9` in
//!   `sqlite_get_varint` (measured miscompile).
//! - Eviction may not steal a register while a still-active co-holder
//!   conflicts with the incoming range ([`LinearScanAllocator::register_steal_is_safe`]).
//! - Under lifetime-demotion, a spilled value pays a slot access at every
//!   remaining use. There is no reload-at-next-use in this module; that is
//!   a backend / `regalloc.rs` policy.
//!
//! # Spill / eviction cost model
//!
//! Incoming I vs victim V at scan point `I.start`:
//! - `cost(I) = I.uses.len()` — I is not in a register; every use is future
//!   slot traffic if I is demoted.
//! - `cost(V) = |{ u in V.uses | u > I.start }|` — V is in a register *now*;
//!   a use at `I.start` is a register read, not a spill.
//!
//! These two formulas are *supposed* to differ. Unifying them is a bug.

use super::liveness::{
    for_each_operand_in_instruction, for_each_operand_in_terminator,
    for_each_value_use_in_instruction, LiveInterval,
};
use super::regalloc::PhysReg;
use crate::common::fx_hash::FxHashMap;
use crate::common::types::IrType;
use crate::ir::intrinsics::IntrinsicOp;
use crate::ir::reexports::{Instruction, IrBinOp, IrFunction, IrUnaryOp, Operand, Terminator};
use std::sync::OnceLock;

/// Enhanced live interval with priority, uses, and spill weight.
///
/// Extends [`LiveInterval`] with:
/// - `uses`: individual use points within `[start, end]` (sorted, unique)
/// - `loop_depth`: max of def-block depth and hottest use-site depth
/// - `priority`: profile-weighted uses × `10^min(depth, 4)`
/// - `reg_hint`: caller-supplied preferred physical register
/// - `follow_value`: producer whose assigned register we want to inherit
/// - `spill_weight`: `priority / live_span` (higher = more painful to spill)
/// - `cascade`: eviction-chain generation (LLVM RegAllocGreedy lineage)
/// - `segments`: hole-aware live coverage as sorted, disjoint, CLOSED point
///   intervals (RA-05). Empty = no segment data: every conflict test falls
///   back to the fat envelope and the allocator behaves exactly as before
///   this field existed. Populated by `regalloc.rs` for GPR/XMM scans from
///   `liveness.segments` (coalesce-owner unions for phi webs).
/// - `use_weights` / `suffix_cost`: the **position-relative cost model**
///   (see [`LiveRange::remaining_cost`]).
#[derive(Debug, Clone)]
pub struct LiveRange {
    pub value_id: u32,
    pub start: u32,
    pub end: u32,
    pub uses: Vec<u32>,
    pub loop_depth: u32,
    pub priority: u64,
    /// Preferred physical register. Honour only if it is in the current
    /// allocation class and [`LiveRange::conflicts_with`] allows the share.
    /// Callers / later passes may populate this; [`build_live_ranges`] does
    /// not (it uses [`LiveRange::follow_value`] instead).
    pub reg_hint: Option<PhysReg>,
    /// Producer to follow. Resolved at allocation time: the producer is
    /// defined earlier, so its assignment is already known. Sharing is
    /// accepted only when the producer's final *recorded* use is this
    /// consumer's definition (use-before-def).
    pub follow_value: Option<u32>,
    pub spill_weight: f64,
    /// An interval that won its register by eviction carries
    /// `victim.cascade + 1` and may only be evicted by an interval with
    /// equal or higher cascade. Bounds ping-pong (used by mode 5).
    pub cascade: u32,
    /// Hole-aware live coverage: sorted, disjoint, CLOSED intervals of
    /// program points. Always a subset of `[start, end]`. See the struct doc.
    pub segments: Vec<(u32, u32)>,
    /// Per-use execution frequency, parallel to `uses`.
    ///
    /// `use_weights[i]` is the estimated execution count of the block that
    /// contains `uses[i]` — `10^min(depth(block(uses[i])), 4)` scaled by the
    /// PGO factor of that point. **Not** a single scalar for the whole range:
    /// `priority` multiplies every use by `10^max_depth`, which values a
    /// range with one inner-loop use exactly like a range with twenty.
    /// GCC (`REG_FREQ_FROM_BB`) and LLVM (`MachineBlockFrequencyInfo`) both
    /// weight per use site; this field is that model.
    ///
    /// Empty = no per-use data. Every accessor then degrades to the
    /// unit-weight count, i.e. exactly the pre-existing behaviour, so
    /// hand-built ranges (unit tests, synthetic vector intervals, phase-2b
    /// span ranges) are bit-identical.
    pub use_weights: Vec<u64>,
    /// Suffix sums of `use_weights`: `suffix_cost[i] == Σ_{j≥i} use_weights[j]`,
    /// with a terminating `0`. Length is `use_weights.len() + 1` whenever
    /// `use_weights` is non-empty. Makes [`LiveRange::remaining_cost`] a
    /// binary search plus one index.
    pub suffix_cost: Vec<u64>,
    /// Policy multiplier applied on top of the measured use costs.
    ///
    /// The allocator's policy layer (`regalloc.rs`) boosts values whose real
    /// read frequency is invisible in the IR use chain — folded SIB indices,
    /// GEP bases folded into addressing, coalesce-group leaders. Those
    /// boosts historically only touched `priority`; the cost model needs the
    /// same information, and multiplying is the composable way to express
    /// "this value is read k× more often than the IR shows".
    pub cost_boost: u64,
    /// Number of program points actually covered by `segments` (Σ of
    /// `e - s + 1`). `0` = unknown, use the fat envelope. The *occupied*
    /// length, not the envelope, is the correct denominator for a
    /// density-style spill weight: a value with a huge envelope but a tiny
    /// live coverage barely occupies the register.
    pub occupancy_len: u32,
}


impl LiveRange {
    /// Create a placeholder range. [`build_live_ranges`] overwrites uses,
    /// priority, hints and spill weight with the real facts.
    pub fn from_interval(interval: LiveInterval, loop_depth: u32) -> Self {
        let loop_weight = loop_depth_weight(loop_depth);
        Self {
            value_id: interval.value_id,
            start: interval.start,
            end: interval.end,
            uses: Vec::new(),
            loop_depth,
            priority: loop_weight,
            reg_hint: None,
            follow_value: None,
            spill_weight: loop_weight as f64 / live_span(interval.start, interval.end),
            cascade: 0,
            segments: Vec::new(),
            use_weights: Vec::new(),
            suffix_cost: Vec::new(),
            cost_boost: 1,
            occupancy_len: 0,
        }
    }

    /// Install hole-aware live coverage (sorted, disjoint, CLOSED point
    /// intervals). Callers sort/normalize; this only records. Every segment
    /// must lie within `[start, end]` — the interference tests rely on the
    /// fat envelope as a conservative superset for mixed fat/segment pairs.
    pub fn set_segments(&mut self, segments: Vec<(u32, u32)>) {
        self.occupancy_len = segments
            .iter()
            .map(|&(s, e)| e.saturating_sub(s).saturating_add(1))
            .fold(0u32, |a, b| a.saturating_add(b));
        self.segments = segments;
    }

    /// Install `uses` together with their per-site execution frequencies.
    ///
    /// `weights[i]` belongs to `uses[i]` *before* sorting; the pair is sorted
    /// jointly and duplicate points are merged by **summing** their weights
    /// (an instruction reading the same value twice really does pay twice).
    /// Then the suffix-cost prefix table is built.
    pub fn set_uses_weighted(&mut self, uses: Vec<u32>, weights: Vec<u64>) {
        debug_assert_eq!(
            uses.len(),
            weights.len(),
            "set_uses_weighted: uses and weights must be parallel"
        );
        let mut pairs: Vec<(u32, u64)> = uses.into_iter().zip(weights).collect();
        pairs.sort_unstable_by_key(|&(p, _)| p);
        let mut pts: Vec<u32> = Vec::with_capacity(pairs.len());
        let mut wts: Vec<u64> = Vec::with_capacity(pairs.len());
        for (p, w) in pairs {
            if pts.last() == Some(&p) {
                let last = wts.last_mut().expect("pts and wts stay parallel");
                *last = last.saturating_add(w);
            } else {
                pts.push(p);
                wts.push(w);
            }
        }
        self.uses = pts;
        self.rebuild_cost_table(wts);
    }

    /// Recompute `suffix_cost` from `weights` (which becomes `use_weights`).
    fn rebuild_cost_table(&mut self, weights: Vec<u64>) {
        let n = weights.len();
        let mut suffix = vec![0u64; n + 1];
        for i in (0..n).rev() {
            suffix[i] = suffix[i + 1].saturating_add(weights[i]);
        }
        self.use_weights = weights;
        self.suffix_cost = suffix;
    }

    /// Weighted cost of every use **strictly after** `pos`.
    ///
    /// This is the quantity a spill decision at scan position `pos` actually
    /// trades away: uses at or before `pos` are sunk cost, already paid as
    /// register reads. Comparing two ranges by their *global* totals (what
    /// `priority` does) systematically over-protects a victim whose uses are
    /// nearly all behind the scan point — the exact pathology the repo
    /// previously papered over with a bolted-on "zero-future-use dead
    /// victim" override (measured, reverted twice: see the note on
    /// `select_evict_victim`). Making the cost *position-relative* subsumes
    /// that special case inside one consistent order.
    ///
    /// Falls back to the unweighted future-use count when no per-use data is
    /// attached, so unenriched ranges behave exactly as before.
    pub fn remaining_cost(&self, pos: u32) -> u64 {
        debug_assert_uses_sorted(&self.uses);
        let i = self.uses.partition_point(|&u| u <= pos);
        if self.suffix_cost.len() == self.uses.len() + 1 {
            self.suffix_cost[i]
        } else {
            (self.uses.len() - i) as u64
        }
    }

    /// Weighted cost of every use in the range (the `pos = 0`-exclusive
    /// total). Equals `uses.len()` when no per-use data is attached.
    pub fn total_cost(&self) -> u64 {
        if let Some(&total) = self.suffix_cost.first() {
            if self.suffix_cost.len() == self.uses.len() + 1 {
                return total;
            }
        }
        self.uses.len() as u64
    }

    /// The value the allocator loses by demoting this range at `pos`:
    /// the remaining weighted use cost, scaled by the policy multiplier.
    ///
    /// Saturating so a pathological boost cannot wrap into a *low* cost and
    /// make a hot value look free to spill.
    pub fn spill_cost_at(&self, pos: u32) -> u64 {
        self.remaining_cost(pos)
            .saturating_mul(self.cost_boost.max(1))
    }

    /// Multiply the policy cost boost (see [`LiveRange::cost_boost`]).
    /// Idempotent-safe under repeated calls only in the sense that the
    /// caller decides the factor; the allocator never calls this twice for
    /// the same reason.
    pub fn boost_cost(&mut self, factor: u64) {
        self.cost_boost = self.cost_boost.saturating_mul(factor.max(1));
    }

    /// Number of program points this range actually occupies a register for.
    /// Segment coverage when known, the fat envelope otherwise.
    pub fn occupied_points(&self) -> u32 {
        if self.occupancy_len > 0 {
            self.occupancy_len
        } else {
            self.end.saturating_sub(self.start).saturating_add(1)
        }
    }

    /// Install `uses`, enforcing the sorted+unique invariant that the
    /// binary-search helpers depend on.
    ///
    /// Drops any attached per-use cost table: the new use list invalidates
    /// it, and a stale parallel array would silently mis-price the range.
    /// Callers that have frequencies should use
    /// [`LiveRange::set_uses_weighted`].
    pub fn set_uses(&mut self, mut uses: Vec<u32>) {
        uses.sort_unstable();
        uses.dedup();
        self.uses = uses;
        self.use_weights.clear();
        self.suffix_cost.clear();
    }

    /// Recalculate spill weight from the current priority and span.
    pub fn calculate_spill_weight(&mut self) {
        self.spill_weight = self.priority as f64 / live_span(self.start, self.end);
    }

    /// Half-open overlap test against `[start, end)`.
    pub fn overlaps(&self, start: u32, end: u32) -> bool {
        self.start < end && start < self.end
    }

    /// True iff the last recorded use sits exactly at `point`.
    ///
    /// Ends extended by liveness (live-through, GEP-fold bases, F128 source
    /// pointers, …) are *not* recorded uses; those ranges stay conservative
    /// in [`LiveRange::conflicts_with`].
    #[inline]
    fn last_use_is_at(&self, point: u32) -> bool {
        self.uses.last() == Some(&point)
    }

    /// Whether `self` and `other` may **not** share one physical register.
    ///
    /// Ranges conflict when their live-point sets intersect, except the
    /// use-before-def adjacency: if `self` dies exactly where `other` is
    /// born *and* `self` is genuinely used at that point, the read precedes
    /// the write and they may coalesce. An artificially extended end fails
    /// [`LiveRange::last_use_is_at`] and never shares.
    #[inline]
    pub fn conflicts_with(&self, other: &LiveRange) -> bool {
        if self.start > other.end || other.start > self.end {
            return false;
        }
        if self.end == other.start && self.last_use_is_at(self.end) {
            return false;
        }
        if other.end == self.start && other.last_use_is_at(other.end) {
            return false;
        }
        true
    }

    /// Hole-aware generalization of [`LiveRange::conflicts_with`] (RA-05).
    ///
    /// Two ranges conflict when any live segment of one properly covers a
    /// live point of the other, except the use-before-def adjacency: a
    /// single shared point `p` (one coverage ending at `p`, the other
    /// starting at `p`) is not a conflict when the dying side's last
    /// *recorded* use is exactly `p` — the read at `p` precedes the other's
    /// def at `p` within the same instruction. This is the same rule the
    /// fat test applies at whole-envelope granularity; segment coverage
    /// only removes phantom conflicts inside holes (points where liveness
    /// proved the value dead on every path).
    ///
    /// When either side has no segment data the fat envelopes are used as
    /// single-segment coverage, which reproduces [`LiveRange::conflicts_with`]
    /// exactly (both sides fat pairs are routed through the original test so
    /// existing behavior is bit-identical).
    pub fn conflicts_with_segments(&self, other: &LiveRange) -> bool {
        if self.segments.is_empty() && other.segments.is_empty() {
            return self.conflicts_with(other);
        }
        // Materialize fat coverage for whichever side lacks segments; a
        // stack array keeps this allocation-free.
        let fat_a = [(self.start, self.end)];
        let fat_b = [(other.start, other.end)];
        let a: &[(u32, u32)] = if self.segments.is_empty() {
            &fat_a
        } else {
            &self.segments
        };
        let b: &[(u32, u32)] = if other.segments.is_empty() {
            &fat_b
        } else {
            &other.segments
        };
        let (mut ai, mut bi) = (0usize, 0usize);
        while ai < a.len() && bi < b.len() {
            let (sa, ea) = a[ai];
            let (sb, eb) = b[bi];
            if sa <= eb && sb <= ea {
                // Shared closed span [max(sa,sb), min(ea,eb)].
                let shared_start = sa.max(sb);
                let shared_end = ea.min(eb);
                if shared_end > shared_start {
                    return true;
                }
                // Exactly one shared point: the use-before-def adjacency.
                // Legal only when the dying side's globally last recorded use
                // is that point (the read precedes the other side's def
                // within the same instruction). A side that merely has a
                // segment boundary there (live-through a hole, reviving
                // later) keeps its value in this register across the hole
                // and must not be clobbered.
                if ea == sb {
                    if !self.last_use_is_at(ea) {
                        return true;
                    }
                } else if eb == sa {
                    if !other.last_use_is_at(eb) {
                        return true;
                    }
                } else {
                    // Both segments end at the shared point (degenerate
                    // zero-width coverage); build_segments never produces
                    // this. Fail closed.
                    return true;
                }
            }
            if ea <= eb {
                ai += 1;
            } else {
                bi += 1;
            }
        }
        false
    }
}

/// Inclusive live-span length. Inverted intervals do not underflow.
#[inline]
fn live_span(start: u32, end: u32) -> f64 {
    end.saturating_sub(start).saturating_add(1).max(1) as f64
}

/// `10^min(depth, 4)`. Cap keeps the ranking strict: depths 5+ must not
/// collapse into one saturated weight and flatten eviction.
#[inline]
pub(crate) fn loop_depth_weight(loop_depth: u32) -> u64 {
    10u64.pow(loop_depth.min(4) as u32)
}

#[inline]
fn debug_assert_uses_sorted(uses: &[u32]) {
    debug_assert!(
        uses.windows(2).all(|w| w[0] <= w[1]),
        "LiveRange.uses must be sorted ascending (LiveRange::set_uses / build_live_ranges)"
    );
}

/// An interval that currently occupies a physical register.
#[derive(Debug, Clone)]
pub struct ActiveInterval {
    pub range: LiveRange,
    /// Physical register occupied by this interval. Retained after eviction so
    /// `handled` can verify the complete assignment history.
    pub phys_reg: PhysReg,
    /// Last program point for which this interval occupied `phys_reg`. This is
    /// shortened to the incoming range's start when the value is evicted.
    pub occupancy_end: u32,
    /// First use at or after the assignment point. Informational / API;
    /// eviction recomputes against the *current* scan position.
    pub next_use: Option<u32>,
    /// Half-open `[start, end)` occupancy spans this interval held on
    /// `phys_reg`, truncated at eviction. Under the fat model this is the
    /// single span `[start, occupancy_end + 1)`; with segment coverage each
    /// closed segment `[s, e]` contributes `[s, e + 1)`. Used by
    /// [`LinearScanAllocator::verify_handled_history`] to validate
    /// segment-aware sharing (RA-05).
    pub occupancy_spans: Vec<(u32, u32)>,
}

/// Linear-scan allocator state.
///
/// After [`LinearScanAllocator::run`]:
/// - `ranges` is empty (consumed)
/// - `assignments` maps allocated values to physical registers
/// - `spill_slots` maps demoted values to a placeholder slot id
/// - `active` ∪ `handled` hold every range that received a register
///   (evicted ranges sit in `handled` with a spill slot and no assignment)
pub struct LinearScanAllocator {
    pub ranges: Vec<LiveRange>,
    pub active: Vec<ActiveInterval>,
    /// Retired intervals. Public API; iteration order is not semantic.
    pub handled: Vec<ActiveInterval>,
    pub assignments: FxHashMap<u32, PhysReg>,
    /// One past the last use of the most recent occupant of each register.
    pub reg_free_until: FxHashMap<PhysReg, u32>,
    /// Segment-aware occupancy (RA-05): per register, the union of the
    /// half-open `[start, end+1)` spans of every current holder's live
    /// segments (fat envelope when a holder has no segment data), merged
    /// into a sorted disjoint list. Only maintained when `segment_mode` is
    /// set; the fat `reg_free_until` above is maintained in BOTH modes so
    /// `earliest_free_position` and non-segment callers keep their exact
    /// pre-existing semantics.
    pub reg_occupancy: FxHashMap<PhysReg, Vec<(u32, u32)>>,
    /// Whether the scan consults `reg_occupancy` / segment conflicts.
    /// Enabled by [`LinearScanAllocator::new`] when any incoming range
    /// carries segment coverage and the `CCC_NO_SEGMENT_SCAN` kill switch
    /// is unset. With no segmented ranges the allocator is bit-identical
    /// to the pre-RA-05 fat scan.
    pub segment_mode: bool,
    pub spill_slots: FxHashMap<u32, i32>,
    pub available_regs: Vec<PhysReg>,
    pub next_spill_slot: i32,
    /// Reserved. Live-range *splitting* is an IR pre-pass (`split_ranges.rs`),
    /// not this flag. Kept so existing setters do not break.
    pub enable_splitting: bool,
    /// Rotation cursor: consecutive unhinted assignments start at different
    /// registers to expose ILP on Raptor Lake's extra integer ports.
    pub next_reg_idx: usize,
}

/// Cached `CCC_NO_SEGMENT_SCAN` kill switch (RA-05 segment-aware scan).
fn segment_scan_enabled() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("CCC_NO_SEGMENT_SCAN").is_none())
}

impl LinearScanAllocator {
    pub fn new(ranges: Vec<LiveRange>, available_regs: Vec<PhysReg>) -> Self {
        // RA-05: enter segment mode only when the worklist actually carries
        // hole-aware coverage AND the kill switch is unset. Unit tests and
        // unenriched scans (Phase 2b, synthetic vector intervals) run the
        // exact fat kernel.
        let segment_mode = segment_scan_enabled() && ranges.iter().any(|r| !r.segments.is_empty());
        Self {
            ranges,
            active: Vec::new(),
            handled: Vec::new(),
            assignments: FxHashMap::default(),
            reg_free_until: FxHashMap::default(),
            reg_occupancy: FxHashMap::default(),
            segment_mode,
            spill_slots: FxHashMap::default(),
            available_regs,
            next_spill_slot: 0,
            enable_splitting: false,
            next_reg_idx: 0,
        }
    }

    /// Mark every register in the current pool free from point 0.
    /// Clears first so a second call with a different pool cannot leave
    /// stale occupancy from the previous class (GPR → XMM, phase 2 → 2c).
    pub fn init_registers(&mut self) {
        self.reg_free_until.clear();
        for &reg in &self.available_regs {
            self.reg_free_until.insert(reg, 0);
        }
    }

    pub fn earliest_free_position(&self) -> u32 {
        self.reg_free_until.values().copied().min().unwrap_or(0)
    }

    pub fn occupy_register(&mut self, reg: PhysReg, until: u32) {
        self.reg_free_until.insert(reg, until);
    }

    /// Allocator-local “this value is demoted” record. Slot *packing* and
    /// type-accurate sizes live in the backend stack-layout pass; the
    /// 8-byte stride is an ABI-neutral placeholder.
    ///
    /// Idempotent: a value already demoted keeps its slot (eviction + a
    /// later defensive call must not punch a second hole in the frame).
    pub fn allocate_spill_slot(&mut self, value_id: u32) -> i32 {
        if let Some(&existing) = self.spill_slots.get(&value_id) {
            return existing;
        }
        debug_assert!(
            self.next_spill_slot > i32::MIN + 8,
            "spill-slot arena exhausted"
        );
        let slot = self.next_spill_slot;
        self.spill_slots.insert(value_id, slot);
        self.next_spill_slot = self.next_spill_slot.saturating_sub(8);
        slot
    }

    /// Drop intervals whose last live point is strictly before `current_start`.
    ///
    /// Occupancy is already in `reg_free_until` (`end + 1` at assignment),
    /// so expiry does not poke that map. `swap_remove` is used because
    /// `active` order is not semantic (the worklist order is `ranges`).
    pub fn expire_old_intervals(&mut self, current_start: u32) {
        let mut i = 0;
        while i < self.active.len() {
            if self.active[i].range.end < current_start {
                let expired = self.active.swap_remove(i);
                self.handled.push(expired);
            } else {
                i += 1;
            }
        }
    }

    /// Pick a register for `range`.
    ///
    /// 1. Honour `reg_hint` / `follow_value` if the register is in this
    ///    pool and [`Self::register_compatible`] (the only path that
    ///    applies use-before-def adjacency).
    /// 2. Otherwise a register whose last occupant died *strictly before*
    ///    `range.start`, rotating the start index for ILP. In segment mode
    ///    (RA-05) "free" means no occupancy span of the register overlaps
    ///    the range's live segments — a register held across a liveness
    ///    hole can be reused inside that hole. Touching spans are still
    ///    rejected here: the rotation path never applies die-at-birth
    ///    sharing, exactly like the fat `free_until <= start` test it
    ///    generalizes.
    pub fn find_free_register(&mut self, range: &LiveRange) -> Option<PhysReg> {
        if let Some(hint) = range.reg_hint {
            if self.is_allocatable_reg(hint) && self.register_compatible(hint, range) {
                return Some(hint);
            }
        } else if let Some(producer) = range.follow_value {
            if let Some(&hint) = self.assignments.get(&producer) {
                if self.is_allocatable_reg(hint) && self.register_compatible(hint, range) {
                    return Some(hint);
                }
            }
        }

        let n = self.available_regs.len();
        if n == 0 {
            return None;
        }
        let start = self.next_reg_idx % n;
        for offset in 0..n {
            let idx = (start + offset) % n;
            let reg = self.available_regs[idx];
            if self.segment_mode {
                if self.register_free_for_segments(reg, range) {
                    self.next_reg_idx = idx + 1;
                    return Some(reg);
                }
            } else {
                let free_until = self.reg_free_until.get(&reg).copied().unwrap_or(0);
                if free_until <= range.start {
                    self.next_reg_idx = idx + 1;
                    return Some(reg);
                }
            }
        }
        None
    }

    /// Segment-mode rotation check: no occupancy span of `reg` may share
    /// even a single point with `range`'s live coverage (half-open spans;
    /// touching spans are rejected because the rotation path never applies
    /// the die-at-birth exception — same conservatism as the fat test
    /// `reg_free_until[reg] <= range.start`).
    fn register_free_for_segments(&self, reg: PhysReg, range: &LiveRange) -> bool {
        let Some(occupied) = self.reg_occupancy.get(&reg) else {
            return true;
        };
        let fat = [(range.start, range.end)];
        let cov: &[(u32, u32)] = if range.segments.is_empty() {
            &fat
        } else {
            &range.segments
        };
        // Both lists are sorted; a merge scan is sufficient.
        let (mut oi, mut ci) = (0usize, 0usize);
        while oi < occupied.len() && ci < cov.len() {
            let (os, oe) = occupied[oi]; // half-open [os, oe)
            let (cs, ce) = cov[ci]; // closed [cs, ce] -> span [cs, ce+1)
            if os < ce + 1 && cs < oe {
                return false;
            }
            if oe <= cs {
                oi += 1;
            } else {
                ci += 1;
            }
        }
        true
    }

    /// Whether `reg` belongs to the current allocation class.
    #[inline]
    fn is_allocatable_reg(&self, reg: PhysReg) -> bool {
        self.available_regs.contains(&reg)
    }

    /// Whether `reg` may hold `range` for its whole lifetime.
    ///
    /// 1. **Occupancy.** Fat mode: `reg_free_until[reg] - 1` is the last
    ///    occupant's end. Strictly past `range.start` ⇒ the occupant is
    ///    still needed after `range`'s def. Equality is the die-at-birth
    ///    case and is decided by (2). Segment mode (RA-05): no occupancy
    ///    span may properly overlap `range`'s live coverage (single-point
    ///    use-before-def adjacency is permitted by (2)'s conflict test).
    /// 2. **Interference.** No active holder of `reg` may conflict with
    ///    `range` ([`LiveRange::conflicts_with_segments`] in segment mode,
    ///    [`LiveRange::conflicts_with`] otherwise).
    ///
    /// `|active|` is bounded by the register file plus die-at-birth
    /// extras; a per-register index would add sync surface for tens of
    /// comparisons.
    fn register_compatible(&self, reg: PhysReg, range: &LiveRange) -> bool {
        if self.segment_mode {
            if !self.register_free_for_segments(reg, range) {
                // A proper span overlap is a hard reject. The single-point
                // die-at-birth case surfaces as a touching span here and is
                // adjudicated by the active-holder conflict test below.
                let touching_only = self.register_touches_only(reg, range);
                if !touching_only {
                    return false;
                }
            }
        } else {
            let free_until = self.reg_free_until.get(&reg).copied().unwrap_or(0);
            let occupant_end = free_until.saturating_sub(1);
            if occupant_end > range.start {
                return false;
            }
        }
        self.active
            .iter()
            .filter(|a| self.assignments.get(&a.range.value_id) == Some(&reg))
            .all(|a| {
                if self.segment_mode {
                    !a.range.conflicts_with_segments(range)
                } else {
                    !a.range.conflicts_with(range)
                }
            })
    }

    /// Segment-mode helper: whether every overlap between `reg`'s occupancy
    /// spans and `range`'s coverage is a single shared boundary point
    /// (candidate die-at-birth adjacency — final legality is decided by the
    /// active-holder conflict test in [`Self::register_compatible`]).
    /// Segment-mode helper: whether every overlap between `reg`'s occupancy
    /// spans and `range`'s coverage is a single shared boundary point
    /// (candidate die-at-birth adjacency — final legality is decided by the
    /// active-holder conflict test in [`Self::register_compatible`]).
    ///
    /// Deliberately NO boundary-anchor restriction here: an earlier revision
    /// only allowed the single shared unit at the range's global first/last
    /// live point, but that gate interacted with the follow-hint pairing to
    /// miscompile the preboot decompressor (misc.c zstd). Genuine mid-range
    /// clobbers are caught by the conflict tests and, as the authoritative
    /// backstop, by the post-RA class-union overlap repair in regalloc.rs.
    /// Segment-mode helper: whether every overlap between `reg`'s occupancy
    /// spans and `range`'s coverage is a single shared boundary point
    /// (candidate die-at-birth adjacency — final legality is decided by the
    /// active-holder conflict test in [`Self::register_compatible`]).
    ///
    /// Deliberately NO boundary-anchor restriction here: an earlier revision
    /// only allowed the single shared unit at the range's global first/last
    /// live point, but that gate interacted with the follow-hint pairing to
    /// miscompile the preboot decompressor (misc.c zstd). Genuine mid-range
    /// clobbers are caught by the conflict tests and, as the authoritative
    /// backstop, by the post-RA class-union overlap repair in regalloc.rs.
    /// Segment-mode helper: whether every overlap between `reg`'s occupancy
    /// spans and `range`'s coverage is a single shared boundary point
    /// (candidate die-at-birth adjacency — final legality is decided by the
    /// active-holder conflict test in [`Self::register_compatible`]).
    ///
    /// Deliberately NO boundary-anchor restriction here: an earlier revision
    /// only allowed the single shared unit at the range's global first/last
    /// live point, but that gate interacted with the follow-hint pairing to
    /// miscompile the preboot decompressor (misc.c zstd). Genuine mid-range
    /// clobbers are caught by the conflict tests and, as the authoritative
    /// backstop, by the post-RA class-union overlap repair in regalloc.rs.
    fn register_touches_only(&self, reg: PhysReg, range: &LiveRange) -> bool {
        let Some(occupied) = self.reg_occupancy.get(&reg) else {
            return true;
        };
        let fat = [(range.start, range.end)];
        let cov: &[(u32, u32)] = if range.segments.is_empty() {
            &fat
        } else {
            &range.segments
        };
        let (mut oi, mut ci) = (0usize, 0usize);
        while oi < occupied.len() && ci < cov.len() {
            let (os, oe) = occupied[oi]; // half-open [os, oe)
            let (cs, ce) = cov[ci]; // closed [cs, ce] -> span [cs, ce+1)
            if os < ce + 1 && cs < oe {
                // Overlap span [max(os,cs), min(oe, ce+1)).
                let lo = os.max(cs);
                let hi = oe.min(ce + 1);
                if hi - lo > 1 {
                    return false;
                }
            }
            // Advance whichever side ends first. Advancing the COVERAGE on
            // an occupant that ends inside it (put_dec: the allowed
            // die-at-birth unit (41,43)x(42,49] advanced ci, exhausted the
            // coverage list and returned true WITHOUT ever examining the
            // seeded (48,50) span — the follow hint then shared the
            // register with the seed) silently skips every later occupancy
            // span.
            if oe <= cs {
                oi += 1;
            } else if ce + 1 <= os {
                ci += 1;
            } else if oe <= ce + 1 {
                oi += 1;
            } else {
                ci += 1;
            }
        }
        true
    }

    /// Uses of `range` strictly after `pos`. Under lifetime-demotion this
    /// is the slot-traffic cost of demoting an interval that is *currently
    /// in a register* at `pos`.
    fn future_uses(range: &LiveRange, pos: u32) -> u32 {
        debug_assert_uses_sorted(&range.uses);
        let uses = range.uses.as_slice();
        let i = uses.partition_point(|&u| u <= pos);
        (uses.len() - i) as u32
    }

    /// Whether giving `evicted_vid`'s register to `incoming` is sound.
    ///
    /// Die-at-birth sharing leaves two active intervals on one physical
    /// register. Evicting only one of them and handing that register to a
    /// third interval that conflicts with the surviving partner is a
    /// silent miscompile.
    ///
    /// Corollary used by [`Self::occupy_register`]: a still-active
    /// co-holder that does *not* conflict with `incoming` must be the
    /// die-at-birth partner and therefore dies at `incoming.start`.
    /// Occupying until `incoming.end + 1` cannot go live too early.
    fn register_steal_is_safe(&self, evicted_vid: u32, incoming: &LiveRange) -> bool {
        let Some(&reg) = self.assignments.get(&evicted_vid) else {
            return false;
        };
        if !self.is_allocatable_reg(reg) {
            return false;
        }
        let conflicts = |holder: &LiveRange| {
            if self.segment_mode {
                holder.conflicts_with_segments(incoming)
            } else {
                holder.conflicts_with(incoming)
            }
        };
        if self.active.iter().any(|a| {
            a.range.value_id != evicted_vid
                && self.assignments.get(&a.range.value_id) == Some(&reg)
                && conflicts(&a.range)
        }) {
            return false;
        }
        // Cross-wave / cross-phase steals are NOT filtered here. The former
        // occupancy-based filters (a victim-coverage "explained" test plus
        // a fat seed horizon) miscompiled the preboot decompressor: an
        // occupant span lying inside the victim's coverage can belong to a
        // still-assigned SEEDED value that try_evict does not cut, so the
        // "explained" steal handed the register to `incoming` on top of it
        // (misc.c: every zstd pattern failed; the RA verifier only caught
        // part of the fallout because die-at-birth single-unit shares are
        // exempt there too). Soundness for cross-phase sharing is instead
        // guaranteed by the post-RA class-union overlap repair backstop in
        // regalloc.rs, which demotes the colder class of any genuinely
        // overlapping pair to a stack slot.
        true
    }

    /// Exchange eviction (mode 5): evict the legal victim with the fewest
    /// *future* uses, provided that is strictly less than the incoming's
    /// total uses (see module-level cost model). Cascade-illegal and
    /// steal-unsafe victims are skipped.
    ///
    /// Measured & rejected as the *default* (gzip 1.14, Raptor Lake):
    /// locally profitable exchanges cascaded into net-higher slot traffic
    /// in `longest_match`. Kept behind `CCC_EVICT_MODE=5` / Phase 2c.
    pub fn find_exchange_candidate(&self, range: &LiveRange) -> Option<usize> {
        if self.active.is_empty() {
            return None;
        }
        // Incoming is not in a register: every recorded use is future cost.
        let incoming_future = range.uses.len() as u32;
        let mut best_idx: Option<usize> = None;
        let mut best_future = u32::MAX;
        let mut best_next_use = 0u32;
        for (idx, interval) in self.active.iter().enumerate() {
            if interval.range.cascade > range.cascade {
                continue;
            }
            if !self.register_steal_is_safe(interval.range.value_id, range) {
                continue;
            }
            // ABI-hinted ParamRefs are a codegen contract (see
            // `select_evict_victim`); exchanging them out of their ABI
            // register breaks the ordered-copy elision.
            if interval.range.reg_hint.is_some() {
                continue;
            }
            let fut = Self::future_uses(&interval.range, range.start);
            let nxt = next_use_after(&interval.range, range.start);
            // Min future cost; ties → farthest next use (Braun–Hack MIN).
            if fut < best_future || (fut == best_future && nxt > best_next_use) {
                best_idx = Some(idx);
                best_future = fut;
                best_next_use = nxt;
            }
        }
        match best_idx {
            Some(idx) if best_future < incoming_future => Some(idx),
            _ => None,
        }
    }

    /// Lowest-priority active interval, ties → farthest next use, then
    /// lowest spill weight. No steal filter (public API).
    pub fn find_evict_candidate(&self, current_pos: u32) -> Option<usize> {
        self.find_evict_candidate_filtered(current_pos, None)
    }

    fn find_evict_candidate_filtered(
        &self,
        current_pos: u32,
        incoming: Option<&LiveRange>,
    ) -> Option<usize> {
        if self.active.is_empty() {
            return None;
        }
        let mut best_idx: Option<usize> = None;
        let mut best_priority = u64::MAX;
        let mut best_next_use = 0u32;
        let mut best_spill_weight = f64::INFINITY;

        for (idx, interval) in self.active.iter().enumerate() {
            if let Some(incoming) = incoming {
                if !self.register_steal_is_safe(interval.range.value_id, incoming) {
                    continue;
                }
            }
            let priority = interval.range.priority;
            let next_use = next_use_after(&interval.range, current_pos);
            let sw = interval.range.spill_weight;
            let better = match best_idx {
                None => true,
                Some(_) => {
                    priority < best_priority
                        || (priority == best_priority && next_use > best_next_use)
                        || (priority == best_priority
                            && next_use == best_next_use
                            && sw < best_spill_weight)
                }
            };
            if better {
                best_idx = Some(idx);
                best_priority = priority;
                best_next_use = next_use;
                best_spill_weight = sw;
            }
        }
        best_idx
    }

    /// Mode-aware victim search: consider **every** interval that passes
    /// the soundness / profitability guards, then rank.
    ///
    /// Pick-then-reject (v1 / repo) chose the globally cheapest victim and
    /// abandoned eviction entirely when that one failed the mode-3
    /// next-use window. A slightly costlier victim with `next_use` past
    /// `incoming.end` is a strictly better decision than spilling a hot
    /// incoming.
    ///
    /// Guards (all must pass, unless the victim is already dead at the scan
    /// point — see below):
    /// - steal-safe
    /// - the incoming must be *strictly more expensive to spill* than the
    ///   victim (never evict a hotter value)
    /// - mode 1 only: `victim.loop_depth < incoming.loop_depth`
    /// - mode ≥ 3: victim's next use is strictly after `incoming.end`
    ///
    /// # Cost order (mode 6 vs modes 1–3)
    ///
    /// Modes 1–3 rank by the **global** `priority`
    /// (`Σ uses × 10^max_loop_depth`). Mode 6 ranks by
    /// [`LiveRange::spill_cost_at`] at the scan point: the *remaining*
    /// per-use-frequency-weighted cost. Two independent corrections:
    ///
    /// 1. **Per-use frequency.** `priority` multiplies every use by
    ///    `10^max_depth`, so one inner-loop use makes a range as expensive
    ///    as twenty. The cost model weights each use by its own block.
    /// 2. **Position relativity.** A victim's uses before the scan point are
    ///    sunk cost — they were served from a register. Comparing global
    ///    totals over-protects a nearly-dead victim.
    ///
    /// Correction (2) is what the twice-reverted "zero-future-use dead
    /// victim" experiments were groping for. Both attempts bolted a special
    /// case on top of the *unchanged* global order (as a ranking override,
    /// then as a last resort), which perturbed the eviction cascade in
    /// unrolled kernels: the zlib-ng Adler DO8 loop regressed 59.9 → 70.4 ms.
    /// Mode 6 instead makes the whole order consistent, so a dead victim is
    /// simply the cheapest one rather than an out-of-band override, and the
    /// profitability guard uses the same currency as the ranking.
    /// `CCC_NO_DEAD_EVICT` no longer changes behaviour and is kept only for
    /// the documented experiment trail.
    fn select_evict_victim(&self, incoming: &LiveRange, mode: i32) -> Option<usize> {
        if self.active.is_empty() || mode <= 0 {
            return None;
        }
        let cost_order = mode >= 6;
        let pos = incoming.start;
        let incoming_cost = incoming.spill_cost_at(pos);
        let mut best_idx: Option<usize> = None;
        let mut best_priority = u64::MAX;
        let mut best_next_use = 0u32;
        let mut best_spill_weight = f64::INFINITY;

        for (idx, interval) in self.active.iter().enumerate() {
            if !self.register_steal_is_safe(interval.range.value_id, incoming) {
                continue;
            }
            // ABI-hinted values (leading ParamRefs) are a codegen contract:
            // the ordered parallel-copy emitter elides the entry copy when a
            // param's home equals its ABI register. Evicting such a value
            // hands its ABI register to someone else and another param may
            // take it, after which the elided copy is missing and the param
            // is read from the wrong register (sqlite_vdbe_peephole
            // corruption: `op` read from %dil instead of %sil). Never evict
            // a hinted value; spill the incoming instead.
            if interval.range.reg_hint.is_some() {
                continue;
            }
            // Rank key. Mode 6: position-relative weighted spill cost.
            // Modes 1-3: the historical global priority.
            let priority = if cost_order {
                interval.range.spill_cost_at(pos)
            } else {
                interval.range.priority
            };
            let bar = if cost_order {
                incoming_cost
            } else {
                incoming.priority
            };
            if bar <= priority {
                continue;
            }
            if mode < 2 && interval.range.loop_depth >= incoming.loop_depth {
                continue;
            }
            let nxt = next_use_after(&interval.range, incoming.start);
            if mode >= 3 && nxt <= incoming.end {
                continue;
            }
            let sw = interval.range.spill_weight;
            let better = best_idx.is_none()
                || priority < best_priority
                || (priority == best_priority && nxt > best_next_use)
                || (priority == best_priority && nxt == best_next_use && sw < best_spill_weight);
            if better {
                best_idx = Some(idx);
                best_priority = priority;
                best_next_use = nxt;
                best_spill_weight = sw;
            }
        }
        best_idx
    }

    /// Allocate one range: expire → hint/free register → evict or spill.
    pub fn allocate_range(&mut self, range: LiveRange) {
        self.expire_old_intervals(range.start);

        if let Some(reg) = self.find_free_register(&range) {
            if alloc_trace_enabled() {
                for active in &self.active {
                    if let Some(&areg) = self.assignments.get(&active.range.value_id) {
                        if areg == reg
                            && (if self.segment_mode {
                                active.range.conflicts_with_segments(&range)
                            } else {
                                active.range.conflicts_with(&range)
                            })
                        {
                            eprintln!(
                                "[ALLOC-BUG] Assigning val{}[{}-{}] to reg={} but val{}[{}-{}] already in reg={}!",
                                range.value_id, range.start, range.end, reg.0,
                                active.range.value_id, active.range.start, active.range.end, areg.0
                            );
                        }
                    }
                }
            }
            debug_assert!(
                self.register_compatible(reg, &range),
                "find_free_register returned an interfering register"
            );
            self.commit_assignment(range, reg);
            return;
        }

        let mode = evict_mode();
        let victim = if mode == 5 {
            self.find_exchange_candidate(&range)
        } else {
            self.select_evict_victim(&range, mode)
        };
        if let Some(evict_idx) = victim {
            if self.try_evict(evict_idx, range) {
                return;
            }
        } else {
            // `range` moved only on the success path above.
            self.allocate_spill_slot(range.value_id);
            return;
        }
        // try_evict refused (defensive): incoming was moved into it and
        // returned, so this path is unreachable. Keep a spill for the
        // type-checker if the signature ever changes.
        // (try_evict takes `range` by value; on failure it must not
        // consume — see try_evict.)
    }

    fn commit_assignment(&mut self, range: LiveRange, reg: PhysReg) {
        let next = next_use_after(&range, range.start);
        self.assignments.insert(range.value_id, reg);
        // First point at which the register is free again. Saturating:
        // a range ending at u32::MAX must not wrap to 0 and look free.
        self.occupy_register(reg, range.end.saturating_add(1));
        let occupancy_end = range.end;
        // RA-05: record exact occupancy spans (half-open). Fat coverage is
        // the single span [start, end+1); segment coverage contributes
        // [s, e+1) per closed segment.
        let occupancy_spans: Vec<(u32, u32)> = if range.segments.is_empty() {
            vec![(range.start, range.end.saturating_add(1))]
        } else {
            range
                .segments
                .iter()
                .map(|&(s, e)| (s, e.saturating_add(1)))
                .collect()
        };
        if self.segment_mode {
            self.insert_occupancy(reg, &occupancy_spans);
        }
        self.active.push(ActiveInterval {
            range,
            phys_reg: reg,
            occupancy_end,
            next_use: Some(next),
            occupancy_spans,
        });
    }

    /// Merge half-open spans into a register's sorted occupancy union.
    fn insert_occupancy(&mut self, reg: PhysReg, spans: &[(u32, u32)]) {
        let entry = self.reg_occupancy.entry(reg).or_default();
        let mut merged: Vec<(u32, u32)> = Vec::with_capacity(entry.len() + spans.len());
        let (mut i, mut j) = (0usize, 0usize);
        while i < entry.len() || j < spans.len() {
            let next = if j == spans.len() || (i < entry.len() && entry[i] <= spans[j]) {
                let v = entry[i];
                i += 1;
                v
            } else {
                let v = spans[j];
                j += 1;
                v
            };
            if let Some(last) = merged.last_mut() {
                if next.0 <= last.1 {
                    last.1 = last.1.max(next.1);
                    continue;
                }
            }
            merged.push(next);
        }
        *entry = merged;
    }

    /// Subtract `removed` (sorted, disjoint, half-open) from a register's
    /// occupancy union. Only the evicted victim's post-cut coverage is
    /// removed — other holders keep theirs (they never properly overlap
    /// the victim, at most a single die-at-birth point). The boundary point
    /// of each removed span is *kept*: a later range born exactly at the
    /// cut point is still rejected by the rotation check, matching the fat
    /// model's cross-wave seed conservatism.
    fn subtract_from_occupancy(&mut self, reg: PhysReg, removed: &[(u32, u32)]) {
        if removed.is_empty() {
            return;
        }
        let Some(entry) = self.reg_occupancy.get_mut(&reg) else {
            return;
        };
        // Shrink each removed span to its interior (rs+1, re): keep the
        // boundary point rs in the union.
        let interiors: Vec<(u32, u32)> = removed
            .iter()
            .map(|&(rs, re)| (rs + 1, re))
            .filter(|&(s, e)| e > s)
            .collect();
        if interiors.is_empty() {
            return;
        }
        let mut out: Vec<(u32, u32)> = Vec::with_capacity(entry.len());
        'outer: for &(es, ee) in entry.iter() {
            let mut start = es;
            for &(rs, re) in &interiors {
                if re <= start || rs >= ee {
                    continue; // no overlap with the remaining piece
                }
                if rs > start {
                    out.push((start, rs));
                }
                start = start.max(re);
                if start >= ee {
                    continue 'outer;
                }
            }
            if start < ee {
                out.push((start, ee));
            }
        }
        *entry = out;
    }

    /// Evict `active[evict_idx]`, give its register to `incoming`.
    ///
    /// Returns false and leaves state unchanged if the victim has no
    /// assignment or the steal is unsound. On success the victim is
    /// demoted to a spill slot and moved to `handled`.
    ///
    /// `swap_remove` reorders `active`; order is not semantic.
    fn try_evict(&mut self, evict_idx: usize, incoming: LiveRange) -> bool {
        if evict_idx >= self.active.len() {
            return false;
        }
        let evicted_vid = self.active[evict_idx].range.value_id;
        let evicted_cascade = self.active[evict_idx].range.cascade;
        if !self.register_steal_is_safe(evicted_vid, &incoming) {
            return false;
        }
        let Some(reg) = self.assignments.remove(&evicted_vid) else {
            return false;
        };

        self.allocate_spill_slot(evicted_vid);

        if allocstats_enabled() {
            eprintln!(
                "[EVICT] val{} reg={} -> val{}[{}] fut_in={}",
                evicted_vid,
                reg.0,
                incoming.value_id,
                incoming.start,
                incoming.uses.len()
            );
        }

        let mut incoming = incoming;
        incoming.cascade = evicted_cascade.saturating_add(1);
        let mut evicted = self.active.swap_remove(evict_idx);
        evicted.occupancy_end = incoming.start;
        // RA-05: the victim keeps its pre-cut occupancy; everything from the
        // incoming's start onward is handed to the incoming range.
        if self.segment_mode {
            let cut = incoming.start;
            let full_spans = evicted.occupancy_spans.clone();
            evicted.occupancy_spans = full_spans
                .iter()
                .map(|&(s, e)| (s, e.min(cut.max(s))))
                .filter(|&(s, e)| e > s)
                .collect();
            let removed: Vec<(u32, u32)> = full_spans
                .iter()
                .map(|&(s, e)| (s.max(cut), e))
                .filter(|&(s, e)| e > s)
                .collect();
            self.subtract_from_occupancy(reg, &removed);
        }
        self.handled.push(evicted);
        self.commit_assignment(incoming, reg);
        true
    }

    /// Full scan. Idempotent: a second call reallocates from a clean slate.
    /// Sorts here so callers that skip [`build_live_ranges`] cannot feed an
    /// unsorted worklist. Tie-break `value_id` keeps the assignment
    /// deterministic across equal start+priority.
    pub fn run(&mut self) {
        self.run_with_seed(&FxHashMap::default());
    }

    /// Like [`Self::run`], but the given `(register, spans)` pairs are
    /// applied on top of the cleared occupancy BEFORE the allocation loop.
    /// Lets a second allocation wave see the homes already handed out by an
    /// earlier wave (e.g. Phase 2's arg-register-free pass for call-argument
    /// values) so it cannot reuse a register that is still occupied by one of
    /// those values.
    ///
    /// RA-precise-seed (v12): each register's seed is a list of half-open
    /// `[start, end)` occupancy spans — the *actual* live ranges of the
    /// values earlier waves homed there. The previous fat `[0, until)` seed
    /// starved the whole function of a register whenever an earlier wave
    /// homed a value whose `end` reached a late call (e.g. printf at function
    /// end), forcing every early loop value into a stack slot. Precise spans
    /// let wave 4 reuse that register inside the loop so long as the new
    /// value's range does not overlap any seeded span.
    pub fn run_with_seed(&mut self, seed: &FxHashMap<PhysReg, Vec<(u32, u32)>>) {
        self.active.clear();
        self.handled.clear();
        self.assignments.clear();
        self.spill_slots.clear();
        self.next_spill_slot = 0;
        self.next_reg_idx = 0;
        self.init_registers();
        self.reg_occupancy.clear();
        for (reg, spans) in seed {
            // Fat-mode invariant: reg_free_until is the latest occupied point
            // (max end across spans). Fat mode cannot represent holes, so a
            // multi-span seed collapses to [0, max_end) there; segment mode
            // records each span precisely.
            let max_end = spans.iter().map(|&(_, e)| e).max().unwrap_or(0);
            if max_end > 0 {
                let cur = self.reg_free_until.entry(*reg).or_insert(0);
                *cur = (*cur).max(max_end);
            }
            if self.segment_mode && !spans.is_empty() {
                let mut sorted: Vec<(u32, u32)> = spans.iter().copied().collect();
                sorted.sort_unstable_by_key(|&(s, _)| s);
                self.insert_occupancy(*reg, &sorted);
            }
        }

        let mut ranges = std::mem::take(&mut self.ranges);
        ranges.sort_by(|a, b| {
            a.start
                .cmp(&b.start)
                .then_with(|| b.priority.cmp(&a.priority))
                .then_with(|| a.value_id.cmp(&b.value_id))
        });
        for range in ranges {
            self.allocate_range(range);
        }
        if std::env::var_os("CCC_VERIFY_REGALLOC").is_some() {
            self.verify_handled_history();
        }
    }

    /// Validate the actual register-occupancy history, including ranges that
    /// expired or were evicted and therefore disappeared from final
    /// `assignments`. `occupancy_end` records the eviction cut point, so legal
    /// lifetime demotion is distinguished from a true overlapping assignment.
    ///
    /// RA-05: in segment mode the check runs on the exact half-open
    /// `occupancy_spans`. Two spans may share at most ONE point, and only
    /// when the earlier-ending side provably released the register there:
    /// either it was evicted (cut) at that boundary, or the point is its
    /// last recorded use (use-before-def with the other side's def). Any
    /// wider overlap is a real double-assignment.
    fn verify_handled_history(&self) {
        if self.segment_mode {
            self.verify_handled_history_segments();
            return;
        }
        let mut by_reg: FxHashMap<PhysReg, Vec<(u32, u32, u32)>> = FxHashMap::default();
        for interval in self.handled.iter().chain(self.active.iter()) {
            if interval.occupancy_end <= interval.range.start {
                continue;
            }
            by_reg.entry(interval.phys_reg).or_default().push((
                interval.range.start,
                interval.occupancy_end,
                interval.range.value_id,
            ));
        }
        for (reg, intervals) in &mut by_reg {
            intervals.sort_unstable();
            let mut previous: Option<(u32, u32, u32)> = None;
            for &(start, end, value) in intervals.iter() {
                if let Some((prev_start, prev_end, prev_value)) = previous {
                    assert!(
                        start >= prev_end,
                        "linear-scan history overlap: r{} v{}[{}, {}) vs v{}[{}, {})",
                        reg.0,
                        prev_value,
                        prev_start,
                        prev_end,
                        value,
                        start,
                        end
                    );
                    if end <= prev_end {
                        continue;
                    }
                }
                previous = Some((start, end, value));
            }
        }
    }

    /// Segment-mode history verifier (RA-05). See
    /// [`Self::verify_handled_history`].
    fn verify_handled_history_segments(&self) {
        // (start, end, value, cut_at_end, last_use)
        type Rec = (u32, u32, u32, bool, Option<u32>);
        let mut by_reg: FxHashMap<PhysReg, Vec<Rec>> = FxHashMap::default();
        for interval in self.handled.iter().chain(self.active.iter()) {
            let evicted = interval.occupancy_end < interval.range.end;
            let last_use = interval.range.uses.last().copied();
            if interval.occupancy_spans.is_empty() {
                // Defensive: a committed interval always records spans.
                if interval.occupancy_end <= interval.range.start {
                    continue;
                }
                by_reg.entry(interval.phys_reg).or_default().push((
                    interval.range.start,
                    interval.occupancy_end.saturating_add(1),
                    interval.range.value_id,
                    evicted,
                    last_use,
                ));
                continue;
            }
            for &(s, e) in &interval.occupancy_spans {
                if e <= s {
                    continue;
                }
                // `cut_at_end` marks spans whose end IS the eviction cut:
                // the holder's register reads provably stop there.
                let cut_at_end = evicted && e == interval.occupancy_end;
                by_reg.entry(interval.phys_reg).or_default().push((
                    s,
                    e,
                    interval.range.value_id,
                    cut_at_end,
                    last_use,
                ));
            }
        }
        for (reg, mut spans) in &mut by_reg {
            spans.sort_unstable();
            for w in spans.windows(2) {
                let (prev, next) = (w[0], w[1]);
                if prev.1 == prev.0 || next.1 == next.0 {
                    continue;
                }
                if next.0 >= prev.1 {
                    continue; // apart or touching: fine
                }
                // Overlap. Half-open spans overlap in [next.0, prev.1).
                let shared_len = prev.1 - next.0;
                if shared_len == 1 && (prev.3 || prev.4 == Some(next.0)) {
                    // Single shared point and the earlier side provably
                    // released the register there: evicted at the cut, or
                    // its last recorded use (use-before-def).
                    continue;
                }
                panic!(
                    "segment-scan history overlap: r{} v{}[{}, {}) vs v{}[{}, {})",
                    reg.0, prev.2, prev.0, prev.1, next.2, next.0, next.1
                );
            }
        }
    }
}

/// First use at or after `pos`, or `range.end` if none.
fn next_use_after(range: &LiveRange, pos: u32) -> u32 {
    debug_assert_uses_sorted(&range.uses);
    let uses = range.uses.as_slice();
    let i = uses.partition_point(|&u| u < pos);
    if i < uses.len() {
        uses[i]
    } else {
        range.end
    }
}

fn alloc_trace_enabled() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("CCC_TRACE_ALLOC").is_some())
}

fn allocstats_enabled() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("CCC_TRACE_ALLOCSTATS").is_some())
}

/// Cached `CCC_EVICT_MODE`. First parse wins for the process so a large TU
/// cannot observe a mid-compile env change as two different allocators.
/// Eviction policy selector (`CCC_EVICT_MODE`).
///
/// | mode | victim search | rank key |
/// |------|---------------|----------|
/// | 0 | disabled (always demote the incoming) | — |
/// | 1 | strictly-colder loop depth required | global `priority` |
/// | 2 | no loop-depth guard | global `priority` |
/// | 3 | **default** — victim's next use must be past `incoming.end` | global `priority` |
/// | 5 | exchange ([`LinearScanAllocator::find_exchange_candidate`]) | future-use count |
/// | 6 | same guards as 3 | **position-relative weighted spill cost** |
///
/// Mode 6 is the cost-model-correct order (see
/// [`LinearScanAllocator::select_evict_victim`]). Mode 3 remains the default
/// until mode 6 has a measured win on the benchmark corpus; mode 5 lost gzip
/// and stays opt-in.
fn evict_mode() -> i32 {
    static MODE: OnceLock<Option<i32>> = OnceLock::new();
    let parsed = *MODE.get_or_init(|| {
        std::env::var("CCC_EVICT_MODE")
            .ok()
            .and_then(|s| s.parse().ok())
    });
    parsed.unwrap_or(3)
}

/// LiveInterval → LiveRange: one IR walk for defs, uses, loop depth, hints.
pub fn build_live_ranges(
    intervals: &[LiveInterval],
    loop_depth: &[u32],
    func: &IrFunction,
) -> Vec<LiveRange> {
    let meta = collect_range_metadata(func, loop_depth);
    let pgo_point_weights = pgo_point_weights(func);
    let point_depths = point_loop_depths(func, loop_depth);

    let mut ranges: Vec<LiveRange> = intervals
        .iter()
        .map(|interval| {
            let def_depth = meta
                .def_block
                .get(&interval.value_id)
                .and_then(|&bidx| loop_depth.get(bidx).copied())
                .unwrap_or(0);
            let use_depth = meta
                .max_use_depth
                .get(&interval.value_id)
                .copied()
                .unwrap_or(0);
            LiveRange::from_interval(*interval, def_depth.max(use_depth))
        })
        .collect();

    for range in &mut ranges {
        if let Some(uses) = meta.uses.get(&range.value_id) {
            let kept: Vec<u32> = uses
                .iter()
                .copied()
                .filter(|&u| u >= range.start && u <= range.end)
                .collect();
            // Per-use execution frequency: the loop depth of the block that
            // contains THIS use, times its PGO factor. `point_depths` is the
            // program-point -> block-depth table built alongside the same
            // dense numbering `collect_range_metadata` walks.
            let weights: Vec<u64> = kept
                .iter()
                .map(|&u| {
                    let d = point_depths.get(u as usize).copied().unwrap_or(0);
                    loop_depth_weight(d)
                        .saturating_mul(pgo_point_weights.get(&u).copied().unwrap_or(1))
                })
                .collect();
            range.set_uses_weighted(kept, weights);
        }

        let loop_weight = loop_depth_weight(range.loop_depth);
        let weighted_uses: u64 = range
            .uses
            .iter()
            .map(|u| pgo_point_weights.get(u).copied().unwrap_or(1))
            .sum();
        range.priority = weighted_uses.max(1).saturating_mul(loop_weight);
        range.follow_value = meta.hints.get(&range.value_id).copied();
        range.calculate_spill_weight();
    }

    ranges.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then_with(|| b.priority.cmp(&a.priority))
            .then_with(|| a.value_id.cmp(&b.value_id))
    });
    ranges
}

/// Loop depth of the block owning each program point, in the dense numbering
/// shared by [`build_live_ranges`], [`collect_range_metadata`] and
/// [`pgo_point_weights`] (one point per instruction, then one per terminator,
/// in block order).
///
/// This is the block-frequency table the per-use cost model needs. Keeping it
/// as a flat `Vec` indexed by point makes the per-use lookup a bounds-checked
/// index instead of a hash probe, and makes the numbering contract explicit
/// in one place instead of implicit in three walks.
fn point_loop_depths(func: &IrFunction, loop_depth: &[u32]) -> Vec<u32> {
    let total: usize = func
        .blocks
        .iter()
        .map(|b| b.instructions.len().saturating_add(1))
        .sum();
    let mut out = Vec::with_capacity(total);
    for (block_idx, block) in func.blocks.iter().enumerate() {
        let d = loop_depth.get(block_idx).copied().unwrap_or(0);
        for _ in &block.instructions {
            out.push(d);
        }
        out.push(d); // terminator point
    }
    out
}

struct RangeMetadata {
    def_block: FxHashMap<u32, usize>,
    max_use_depth: FxHashMap<u32, u32>,
    uses: FxHashMap<u32, Vec<u32>>,
    hints: FxHashMap<u32, u32>,
}

/// Single walk: def sites, use points, hottest use-site loop depth, hints.
///
/// Point numbering matches [`pgo_point_weights`].
fn collect_range_metadata(func: &IrFunction, loop_depth: &[u32]) -> RangeMetadata {
    let mut meta = RangeMetadata {
        def_block: FxHashMap::default(),
        max_use_depth: FxHashMap::default(),
        uses: FxHashMap::default(),
        hints: FxHashMap::default(),
    };
    let mut point = 0u32;

    for (block_idx, block) in func.blocks.iter().enumerate() {
        let bdepth = loop_depth.get(block_idx).copied().unwrap_or(0);
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                // SSA: one def. First write wins if IR is broken.
                meta.def_block.entry(dest.0).or_insert(block_idx);
            }
            if let Some((consumer, producer)) = hint_from_instruction(inst) {
                meta.hints.entry(consumer).or_insert(producer);
            }
            record_instruction_uses(inst, point, bdepth, &mut meta);
            point = point.saturating_add(1);
        }
        record_terminator_uses(&block.terminator, point, bdepth, &mut meta);
        point = point.saturating_add(1);
    }
    meta
}

fn record_instruction_uses(inst: &Instruction, point: u32, bdepth: u32, meta: &mut RangeMetadata) {
    // Same visitors as liveness: Intrinsic args, InlineAsm inputs, atomic
    // pointers, SetReturnF*Second, Phi incoming (Phi is an *instruction*
    // in this IR, not a terminator).
    for_each_operand_in_instruction(inst, |op| {
        if let Operand::Value(v) = op {
            record_use(v.0, point, bdepth, meta);
        }
    });
    for_each_value_use_in_instruction(inst, |v| {
        record_use(v.0, point, bdepth, meta);
    });
}

/// Terminator operands via the canonical visitor so a future
/// `Invoke` / `Catch` / `Cleanup` variant cannot be forgotten. Today the
/// IR has four value-carrying terminators (Return, CondBranch,
/// IndirectBranch, Switch); Branch and Unreachable carry none. Phi
/// incoming values are **not** terminator operands — they live on
/// `Instruction::Phi` and are recorded above.
fn record_terminator_uses(term: &Terminator, point: u32, bdepth: u32, meta: &mut RangeMetadata) {
    for_each_operand_in_terminator(term, |op| {
        if let Operand::Value(v) = op {
            record_use(v.0, point, bdepth, meta);
        }
    });
}

#[inline]
fn record_use(vid: u32, point: u32, bdepth: u32, meta: &mut RangeMetadata) {
    meta.uses.entry(vid).or_default().push(point);
    if bdepth != 0 {
        let entry = meta.max_use_depth.entry(vid).or_insert(0);
        *entry = (*entry).max(bdepth);
    }
}

/// Bounded PGO hotness per program point. Missing profile → empty map →
/// every use weights 1 (pure loop-depth heuristic).
///
/// Default `CCC_PGO_WEIGHT_MAX=1` (neutral). A 4× cap was measured as a
/// +4.7 % gzip-compress regression: it double-counts `10^loop_depth` and
/// makes inner-loop temps near-unevictable (+16 slot accesses in
/// `longest_match`).
fn pgo_point_weights(func: &IrFunction) -> FxHashMap<u32, u64> {
    let mut out = FxHashMap::default();
    let Some(fp) = crate::pgo::active_profile_for_function(func) else {
        return out;
    };
    let max = fp.block_counts.values().copied().max().unwrap_or(0);
    if max == 0 {
        return out;
    }
    let max_factor = pgo_weight_max();
    let span = max_factor - 1;
    let mut point = 0u32;
    for block in &func.blocks {
        let count = fp.block_count(block.label);
        let factor = if count == 0 {
            1
        } else {
            1 + (count.saturating_mul(span) / max).min(span)
        };
        for _ in &block.instructions {
            out.insert(point, factor);
            point = point.saturating_add(1);
        }
        out.insert(point, factor);
        point = point.saturating_add(1);
    }
    out
}

fn pgo_weight_max() -> u64 {
    static MAX: OnceLock<u64> = OnceLock::new();
    *MAX.get_or_init(|| {
        std::env::var("CCC_PGO_WEIGHT_MAX")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(1)
            .clamp(1, 16)
    })
}

/// Producer→consumer hint for one instruction, or `None`.
///
/// Recorded only for emitters that compute **into** the destination with
/// the operand pre-loaded, so die-at-birth sharing deletes a copy instead
/// of clobbering a live operand:
///
/// - FP BinOp **LHS** (`emit_float_binop_into_reg`). RHS would be
///   clobbered by the LHS load (`subsd %xmm4,%xmm4` ≡ 0).
/// - GPR simple ALU **LHS** (`emit_alu_reg_direct`: Add/Sub/And/Or/Xor/Mul,
///   not i128). Same RHS rule (`or %r9,%r9` ≡ 0).
/// - Copy (`emit_copy_value` is a no-op when dest ≡ src).
/// - GPR unary Neg/Not/Bswap (`neg/not/bswap %reg` in place).
/// - Scalar Sqrt/Fabs (`sqrtsd` / `andpd` in place).
fn hint_from_instruction(inst: &Instruction) -> Option<(u32, u32)> {
    match inst {
        Instruction::BinOp {
            dest,
            lhs: Operand::Value(lhs),
            ty,
            ..
        } if *ty == IrType::F64 || *ty == IrType::F32 => Some((dest.0, lhs.0)),
        Instruction::BinOp {
            dest,
            lhs: Operand::Value(lhs),
            ty,
            op,
            ..
        } if ty.is_integer()
            && !matches!(ty, IrType::I128 | IrType::U128)
            && matches!(
                op,
                IrBinOp::Add
                    | IrBinOp::Sub
                    | IrBinOp::And
                    | IrBinOp::Or
                    | IrBinOp::Xor
                    | IrBinOp::Mul
            ) =>
        {
            Some((dest.0, lhs.0))
        }
        Instruction::Copy {
            dest,
            src: Operand::Value(src),
        } => Some((dest.0, src.0)),
        Instruction::UnaryOp {
            dest,
            op,
            src: Operand::Value(src),
            ty,
        } if !ty.is_float()
            && !matches!(ty, IrType::I128 | IrType::U128 | IrType::F128)
            && matches!(op, IrUnaryOp::Neg | IrUnaryOp::Not | IrUnaryOp::Bswap) =>
        {
            Some((dest.0, src.0))
        }
        Instruction::Intrinsic {
            dest: Some(dest),
            op,
            args,
            ..
        } if matches!(
            op,
            IntrinsicOp::SqrtF64
                | IntrinsicOp::SqrtF32
                | IntrinsicOp::FabsF64
                | IntrinsicOp::FabsF32
                | IntrinsicOp::RoundScalarF64(_)
                | IntrinsicOp::RoundScalarF32(_)
        ) =>
        {
            match args.as_slice() {
                [Operand::Value(src)] => Some((dest.0, src.0)),
                _ => None,
            }
        }
        _ => None,
    }
}

#[allow(dead_code)]
fn find_register_hints(func: &IrFunction) -> FxHashMap<u32, u32> {
    let mut hints = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some((consumer, producer)) = hint_from_instruction(inst) {
                hints.entry(consumer).or_insert(producer);
            }
        }
    }
    hints
}

#[allow(dead_code)]
fn collect_uses_for_values(func: &IrFunction) -> FxHashMap<u32, Vec<u32>> {
    collect_range_metadata(func, &[]).uses
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: put_dec Wave-4 (vsprintf.c). v39 [41,42] takes %r10-class
    /// PhysReg(10) by rotation (seeded spans [(13,20),(48,50)] leave it free
    /// there); v50 [42,50] (segments [(42,49)], closed) follows v39. The
    /// seed (48,50) — Wave-3's v42 [48,49] — overlaps v50's coverage, so the
    /// follow hint must be refused; before the fix the scan handed v50 the
    /// register anyway and the store index collided with the staged digit.
    #[test]
    fn wave4_follow_vs_seeded_span() {
        let mut v39 = lr_seg(39, 41, 42, vec![42], 10, vec![(41, 42)]);
        v39.follow_value = None;
        // Pin the producer to PhysReg(10) deterministically: the rotation
        // order of a four-register pool is otherwise free to home it
        // elsewhere, which would mask exactly the follow-path refusal this
        // test exists to pin.
        v39.reg_hint = Some(PhysReg(10));
        let mut v50 = lr_seg(50, 42, 50, vec![43, 49], 10, vec![(42, 49)]);
        v50.follow_value = Some(39);
        let v6 = lr_seg(6, 4, 5, vec![5], 10, vec![(4, 5)]);
        let v24 = lr_seg(24, 26, 27, vec![27], 10, vec![(26, 27)]);
        let v32 = lr_seg(32, 34, 35, vec![35], 10, vec![(34, 35)]);
        let v43 = lr_seg(43, 49, 50, vec![50], 10, vec![(49, 50)]);
        let v51 = lr_seg(51, 43, 50, vec![48], 10, vec![(43, 48)]);
        let ranges = vec![v6, v24, v32, v39, v50, v51, v43];
        let mut alloc = LinearScanAllocator::new(
            ranges,
            vec![PhysReg(10), PhysReg(11), PhysReg(12), PhysReg(13)],
        );
        let mut seed: FxHashMap<PhysReg, Vec<(u32, u32)>> = FxHashMap::default();
        seed.insert(PhysReg(10), vec![(48, 50), (13, 20)]);
        seed.insert(PhysReg(11), vec![(35, 42)]);
        alloc.run_with_seed(&seed);
        let got = alloc.assignments.get(&50).copied();
        assert_ne!(
            got,
            Some(PhysReg(10)),
            "v50 must not share PhysReg(10) with seeded v42 [48,50); got {:?}",
            got
        );
    }

    fn lr(value_id: u32, start: u32, end: u32, uses: Vec<u32>, priority: u64) -> LiveRange {
        let mut r = LiveRange {
            value_id,
            start,
            end,
            uses: Vec::new(),
            loop_depth: 0,
            priority,
            reg_hint: None,
            follow_value: None,
            spill_weight: priority as f64 / live_span(start, end),
            cascade: 0,
            segments: Vec::new(),
            use_weights: Vec::new(),
            suffix_cost: Vec::new(),
            cost_boost: 1,
            occupancy_len: 0,
        };
        r.set_uses(uses);
        r
    }

    /// Range with hole-aware segment coverage (closed point intervals).
    fn lr_seg(
        value_id: u32,
        start: u32,
        end: u32,
        uses: Vec<u32>,
        priority: u64,
        segments: Vec<(u32, u32)>,
    ) -> LiveRange {
        let mut r = lr(value_id, start, end, uses, priority);
        r.set_segments(segments);
        r
    }

    #[test]
    fn test_segment_conflicts_hole_is_not_interference() {
        // v1 lives [0,10] and [40,50]; v2 lives only inside v1's hole.
        // Fat envelopes overlap; segment coverage must not conflict.
        let a = lr_seg(1, 0, 50, vec![0, 10, 40, 50], 10, vec![(0, 10), (40, 50)]);
        let b = lr_seg(2, 20, 30, vec![20, 30], 10, vec![(20, 30)]);
        assert!(!a.conflicts_with_segments(&b));
        assert!(!b.conflicts_with_segments(&a));
        // Fat envelope sanity: the old test still sees a conflict.
        assert!(a.conflicts_with(&b));
    }

    #[test]
    fn test_segment_conflicts_proper_overlap_still_conflicts() {
        let a = lr_seg(1, 0, 50, vec![0, 10, 40, 50], 10, vec![(0, 10), (40, 50)]);
        let b = lr_seg(2, 5, 45, vec![5, 45], 10, vec![(5, 45)]);
        assert!(a.conflicts_with_segments(&b));
        assert!(b.conflicts_with_segments(&a));
    }

    #[test]
    fn test_segment_conflicts_die_at_birth_point_share() {
        // v1's last recorded use is exactly where v2 is born: legal share.
        let a = lr_seg(1, 0, 10, vec![0, 10], 10, vec![(0, 10)]);
        let b = lr_seg(2, 10, 20, vec![10, 20], 10, vec![(10, 20)]);
        assert!(!a.conflicts_with_segments(&b));
        assert!(!b.conflicts_with_segments(&a));
    }

    #[test]
    fn test_segment_conflicts_point_share_without_last_use_is_conflict() {
        // v1 is live through point 10 (its last use is earlier); v2 born at
        // 10 would clobber the value v1 still needs at 10.
        let a = lr_seg(1, 0, 10, vec![0, 5], 10, vec![(0, 10)]);
        let b = lr_seg(2, 10, 20, vec![10, 20], 10, vec![(10, 20)]);
        assert!(a.conflicts_with_segments(&b));
        assert!(b.conflicts_with_segments(&a));
    }

    #[test]
    fn test_segment_conflicts_hole_boundary_revival_is_conflict() {
        // v1's coverage revives at 10 with a later segment; a v2 born at 10
        // cannot share the register because v1's value must survive the hole
        // *in that register* (single-home model). The dying side's last use
        // is not 10, so this is a conflict.
        let a = lr_seg(1, 0, 50, vec![0, 5, 40], 10, vec![(0, 10), (40, 50)]);
        let b = lr_seg(2, 10, 30, vec![10, 30], 10, vec![(10, 30)]);
        assert!(a.conflicts_with_segments(&b));
        assert!(b.conflicts_with_segments(&a));
    }

    #[test]
    fn test_segment_conflicts_fat_fallback_is_identical() {
        // Mixed fat/segment pair: the fat side is treated as its envelope.
        let fat = lr(1, 0, 50, vec![0, 50], 10);
        let seg = lr_seg(2, 60, 70, vec![60, 70], 10, vec![(60, 70)]);
        assert!(!fat.conflicts_with_segments(&seg));
        assert!(!seg.conflicts_with_segments(&fat));
        let seg_overlap = lr_seg(2, 40, 70, vec![40, 70], 10, vec![(40, 70)]);
        assert!(fat.conflicts_with_segments(&seg_overlap));
        assert!(seg_overlap.conflicts_with_segments(&fat));
    }

    #[test]
    fn test_segment_scan_shares_register_through_hole() {
        // Two ranges whose fat envelopes overlap but whose segments are
        // disjoint must share one register in segment mode (one-register
        // pool forces the sharing decision).
        let a = lr_seg(1, 0, 50, vec![0, 10, 40, 50], 10, vec![(0, 10), (40, 50)]);
        let b = lr_seg(2, 20, 30, vec![20, 30], 5, vec![(20, 30)]);
        let mut alloc = LinearScanAllocator::new(vec![a, b], vec![PhysReg(1)]);
        assert!(alloc.segment_mode);
        alloc.run();
        assert_eq!(alloc.assignments.get(&1), Some(&PhysReg(1)));
        assert_eq!(alloc.assignments.get(&2), Some(&PhysReg(1)));
    }

    #[test]
    fn test_segment_scan_rejects_register_on_real_overlap() {
        let a = lr_seg(1, 0, 50, vec![0, 10, 40, 50], 10, vec![(0, 10), (40, 50)]);
        let b = lr_seg(2, 5, 45, vec![5, 45], 100, vec![(5, 45)]);
        let mut alloc = LinearScanAllocator::new(vec![a, b], vec![PhysReg(1)]);
        assert!(alloc.segment_mode);
        alloc.run();
        // Real overlap on a one-register pool: the lower-priority range is
        // demoted (eviction guards require the victim's next use strictly
        // after the incoming's end, so here the incoming just spills).
        assert!(alloc.assignments.len() <= 1);
    }

    #[test]
    fn test_segment_scan_unsegmented_worklist_is_fat_mode() {
        let a = lr(1, 0, 50, vec![0, 50], 10);
        let b = lr(2, 20, 30, vec![20, 30], 5);
        let mut alloc = LinearScanAllocator::new(vec![a, b], vec![PhysReg(1)]);
        assert!(!alloc.segment_mode);
        alloc.run();
        // Fat envelopes overlap: only one value can hold the register.
        assert!(alloc.assignments.len() <= 1);
    }

    #[test]
    fn test_segment_scan_verification_accepts_legal_sharing() {
        // Hole sharing (rotation) + die-at-birth point share (hint path) must
        // both pass the segment history verifier.
        let a = lr_seg(1, 0, 10, vec![0, 10], 10, vec![(0, 10)]);
        // b is born exactly at a's last use: legal point share, reachable
        // only through the hint path (rotation never applies die-at-birth).
        let mut b = lr_seg(2, 10, 20, vec![10, 20], 10, vec![(10, 20)]);
        b.reg_hint = Some(PhysReg(1));
        // c lives in a's second hole.
        let c = lr_seg(3, 30, 40, vec![30, 40], 10, vec![(30, 40)]);
        let mut alloc = LinearScanAllocator::new(vec![a, b, c], vec![PhysReg(1)]);
        assert!(alloc.segment_mode);
        alloc.run();
        // The verifier runs under CCC_VERIFY_REGALLOC; call it explicitly.
        alloc.verify_handled_history();
        assert_eq!(alloc.assignments.get(&1), Some(&PhysReg(1)));
        assert_eq!(alloc.assignments.get(&2), Some(&PhysReg(1)));
        assert_eq!(alloc.assignments.get(&3), Some(&PhysReg(1)));
    }

    #[test]
    fn test_live_range_from_interval() {
        let interval = LiveInterval {
            value_id: 42,
            start: 10,
            end: 20,
        };
        let range = LiveRange::from_interval(interval, 1);
        assert_eq!(range.value_id, 42);
        assert_eq!(range.start, 10);
        assert_eq!(range.end, 20);
        assert_eq!(range.loop_depth, 1);
        assert!(range.priority > 0);
        assert!(range.spill_weight > 0.0);
    }

    #[test]
    fn test_from_interval_inverted_span_does_not_underflow() {
        let interval = LiveInterval {
            value_id: 1,
            start: 20,
            end: 10,
        };
        let range = LiveRange::from_interval(interval, 0);
        assert!(range.spill_weight.is_finite());
        assert!(range.spill_weight > 0.0);
    }

    #[test]
    fn test_set_uses_sorts_and_dedups() {
        let mut r = lr(1, 0, 10, vec![], 1);
        r.set_uses(vec![8, 2, 2, 5, 0]);
        assert_eq!(r.uses, vec![0, 2, 5, 8]);
    }

    #[test]
    fn test_overlap_detection() {
        let range = lr(1, 10, 20, vec![], 1);
        assert!(range.overlaps(15, 25));
        assert!(range.overlaps(5, 15));
        assert!(range.overlaps(5, 25));
        assert!(!range.overlaps(21, 30));
        assert!(!range.overlaps(0, 9));
    }

    #[test]
    fn test_conflicts_with_die_at_birth() {
        let producer = lr(1, 0, 5, vec![0, 5], 2);
        let consumer = lr(2, 5, 10, vec![5, 10], 2);
        assert!(!producer.conflicts_with(&consumer));
        assert!(!consumer.conflicts_with(&producer));

        let extended = lr(3, 0, 5, vec![0], 2);
        assert!(extended.conflicts_with(&consumer));

        let overlapping = lr(4, 3, 7, vec![3, 7], 2);
        assert!(producer.conflicts_with(&overlapping));

        let disjoint = lr(5, 20, 30, vec![20, 30], 2);
        assert!(!producer.conflicts_with(&disjoint));
    }

    #[test]
    fn test_priority_weighting() {
        let interval = LiveInterval {
            value_id: 1,
            start: 0,
            end: 100,
        };
        let no_loop = LiveRange::from_interval(interval, 0);
        let in_loop = LiveRange::from_interval(interval, 1);
        let nested_loop = LiveRange::from_interval(interval, 2);
        assert!(in_loop.priority > no_loop.priority);
        assert!(nested_loop.priority > in_loop.priority);
        let d4 = LiveRange::from_interval(interval, 4);
        let d5 = LiveRange::from_interval(interval, 5);
        assert_eq!(d4.priority, d5.priority);
    }

    #[test]
    fn test_spill_weight_calculation() {
        let short_range = lr(1, 0, 10, vec![], 100);
        let long_range = lr(2, 0, 100, vec![], 100);
        assert!(short_range.spill_weight > long_range.spill_weight);
    }

    #[test]
    fn test_linear_scan_basic_allocation() {
        let ranges = vec![
            lr(1, 0, 10, vec![0, 5, 10], 3),
            lr(2, 20, 30, vec![20, 25, 30], 3),
            lr(3, 40, 50, vec![40, 45, 50], 3),
        ];
        let mut allocator = LinearScanAllocator::new(ranges, vec![PhysReg(0), PhysReg(1)]);
        allocator.run();
        assert_eq!(allocator.assignments.len(), 3);
        assert!(allocator.assignments.contains_key(&1));
        assert!(allocator.assignments.contains_key(&2));
        assert!(allocator.assignments.contains_key(&3));
    }

    #[test]
    fn test_linear_scan_spilling() {
        let ranges = vec![
            lr(1, 0, 100, vec![0, 50, 100], 3),
            lr(2, 10, 90, vec![10, 50, 90], 2),
        ];
        let mut allocator = LinearScanAllocator::new(ranges, vec![PhysReg(0)]);
        allocator.run();
        assert!(allocator.assignments.contains_key(&1));
        assert!(allocator.assignments.len() <= 2);
        assert!(allocator.assignments.contains_key(&1) || allocator.spill_slots.contains_key(&1));
        assert!(allocator.assignments.contains_key(&2) || allocator.spill_slots.contains_key(&2));
    }

    #[test]
    fn test_linear_scan_no_registers() {
        let mut allocator = LinearScanAllocator::new(vec![lr(1, 0, 10, vec![0, 10], 1)], vec![]);
        allocator.run();
        assert!(allocator.spill_slots.contains_key(&1));
        assert!(allocator.assignments.is_empty());
    }

    #[test]
    fn test_run_is_idempotent_and_sorts() {
        let ranges = vec![lr(2, 20, 30, vec![20, 30], 1), lr(1, 0, 10, vec![0, 10], 1)];
        let mut allocator = LinearScanAllocator::new(ranges, vec![PhysReg(0)]);
        allocator.run();
        assert_eq!(allocator.assignments.len(), 2);
        allocator.ranges = vec![lr(3, 0, 5, vec![0, 5], 1)];
        allocator.run();
        assert!(allocator.assignments.contains_key(&3));
        assert!(!allocator.assignments.contains_key(&1));
        assert!(!allocator.assignments.contains_key(&2));
        assert!(allocator.spill_slots.is_empty());
    }

    #[test]
    fn test_die_at_birth_hint_reuses_register() {
        let producer = lr(1, 0, 5, vec![0, 5], 2);
        let mut consumer = lr(2, 5, 10, vec![10], 2);
        consumer.follow_value = Some(1);
        let mut allocator =
            LinearScanAllocator::new(vec![producer, consumer], vec![PhysReg(0), PhysReg(1)]);
        allocator.run();
        let r1 = *allocator.assignments.get(&1).expect("producer assigned");
        let r2 = *allocator.assignments.get(&2).expect("consumer assigned");
        assert_eq!(r1, r2, "die-at-birth follow hint must coalesce");
    }

    #[test]
    fn test_evict_does_not_break_shared_register() {
        let a = lr(1, 0, 10, vec![0, 10], 1);
        let mut b = lr(2, 10, 40, vec![10, 20, 40], 100);
        b.follow_value = Some(1);
        let c = lr(3, 10, 30, vec![10, 30], 50);
        let mut allocator = LinearScanAllocator::new(vec![a, b, c], vec![PhysReg(0)]);
        allocator.run();
        assert!(allocator.assignments.contains_key(&2));
        if let (Some(&rb), Some(&rc)) =
            (allocator.assignments.get(&2), allocator.assignments.get(&3))
        {
            panic!(
                "B and overlapping C both assigned (regs {} and {})",
                rb.0, rc.0
            );
        }
        assert!(allocator.spill_slots.contains_key(&3) || !allocator.assignments.contains_key(&3));
    }

    #[test]
    fn test_select_evict_victim_skips_soon_use() {
        // A is cheapest but needed at 15 (inside incoming [10, 50]).
        // B is slightly costlier but not needed until 80 (after incoming).
        // Pick-then-reject would select A, fail the mode-3 window, and
        // spill the incoming. The multi-candidate search must pick B.
        let mut a = LinearScanAllocator::new(vec![], vec![PhysReg(0), PhysReg(1)]);
        a.init_registers();
        a.assignments.insert(1, PhysReg(0));
        a.assignments.insert(2, PhysReg(1));
        a.occupy_register(PhysReg(0), 101);
        a.occupy_register(PhysReg(1), 101);
        a.active.push(ActiveInterval {
            range: lr(1, 0, 100, vec![0, 15, 100], 1),
            phys_reg: PhysReg(0),
            occupancy_end: 100,
            occupancy_spans: vec![(0, 101)],

            next_use: None,
        });
        a.active.push(ActiveInterval {
            range: lr(2, 0, 100, vec![0, 80, 100], 2),
            phys_reg: PhysReg(1),
            occupancy_end: 100,
            occupancy_spans: vec![(0, 101)],

            next_use: None,
        });
        let incoming = lr(3, 10, 50, vec![10, 50], 100);

        let idx3 = a.select_evict_victim(&incoming, 3).expect("legal victim");
        assert_eq!(
            a.active[idx3].range.value_id, 2,
            "mode 3 must skip soon-used A and evict far-next-use B"
        );

        let idx2 = a.select_evict_victim(&incoming, 2).expect("mode-2 victim");
        assert_eq!(
            a.active[idx2].range.value_id, 1,
            "mode 2 has no next-use window: cheapest (A) wins"
        );
    }

    #[test]
    fn test_select_evict_victim_same_point_use_is_not_dead() {
        // A's last use is AT the incoming's start point (use-before-def in
        // the same instruction). Evicting A would hand its register to a
        // value whose definition overwrites it while A's operand is read.
        // The strict mode-3 window hides A (next use 15 <= incoming.end)
        // and no other victim exists, so the incoming is spilled. Guards
        // against reintroducing a "dead victim" search that ignores
        // same-point uses.
        let mut a = LinearScanAllocator::new(vec![], vec![PhysReg(0)]);
        a.init_registers();
        a.assignments.insert(1, PhysReg(0));
        a.occupy_register(PhysReg(0), 51);
        a.active.push(ActiveInterval {
            range: lr(1, 0, 50, vec![0, 15], 1),
            phys_reg: PhysReg(0),
            occupancy_end: 50,
            occupancy_spans: vec![(0, 51)],

            next_use: None,
        });
        let incoming = lr(3, 15, 100, vec![15, 40, 100], 50);
        assert!(
            a.select_evict_victim(&incoming, 3).is_none(),
            "victim whose last use coincides with incoming.start must not be evicted"
        );
    }

    #[test]
    fn test_select_evict_victim_never_evicts_abi_hinted() {
        // A is an ABI-hinted ParamRef (home == ABI register is a codegen
        // contract for the ordered parallel-copy emitter). A is dead at the
        // scan point, which would otherwise make it the ideal victim —
        // the hint protection must outrank the dead-victim exception.
        let mut a = LinearScanAllocator::new(vec![], vec![PhysReg(0)]);
        a.init_registers();
        a.assignments.insert(1, PhysReg(0));
        a.occupy_register(PhysReg(0), 51);
        let mut victim = lr(1, 0, 50, vec![0, 10], 1);
        victim.reg_hint = Some(PhysReg(0));
        a.active.push(ActiveInterval {
            range: victim,
            phys_reg: PhysReg(0),
            occupancy_end: 50,
            occupancy_spans: vec![(0, 51)],

            next_use: None,
        });
        let incoming = lr(3, 15, 100, vec![15, 40, 100], 50);
        assert!(
            a.select_evict_victim(&incoming, 3).is_none(),
            "ABI-hinted ParamRef must never be evicted, dead or not"
        );
    }

    #[test]
    fn test_next_use_after_binary_search() {
        let range = lr(1, 0, 50, vec![0, 10, 20, 40, 50], 5);
        assert_eq!(next_use_after(&range, 0), 0);
        assert_eq!(next_use_after(&range, 11), 20);
        assert_eq!(next_use_after(&range, 40), 40);
        assert_eq!(next_use_after(&range, 51), 50);
        assert_eq!(LinearScanAllocator::future_uses(&range, 20), 2);
        assert_eq!(LinearScanAllocator::future_uses(&range, 50), 0);
    }

    #[test]
    fn test_occupy_does_not_wrap() {
        let mut allocator = LinearScanAllocator::new(vec![], vec![PhysReg(0)]);
        allocator.init_registers();
        allocator.occupy_register(PhysReg(0), u32::MAX);
        assert_eq!(
            allocator.reg_free_until.get(&PhysReg(0)).copied(),
            Some(u32::MAX)
        );
    }

    #[test]
    fn test_allocate_spill_slot_is_stable() {
        let mut allocator = LinearScanAllocator::new(vec![], vec![]);
        let a = allocator.allocate_spill_slot(7);
        let b = allocator.allocate_spill_slot(7);
        let c = allocator.allocate_spill_slot(8);
        assert_eq!(a, b, "double allocate must return the same slot");
        assert_ne!(a, c, "distinct values must not alias slots");
    }

    #[test]
    fn test_exchange_incoming_cost_is_all_uses() {
        // Documents the intentional mode-5 asymmetry: incoming cost is
        // uses.len(), victim cost is uses strictly after scan point.
        let incoming = lr(1, 10, 40, vec![10, 20, 40], 3);
        let victim = lr(2, 0, 50, vec![0, 10, 30, 50], 3);
        assert_eq!(incoming.uses.len(), 3);
        assert_eq!(LinearScanAllocator::future_uses(&victim, incoming.start), 2);
        assert_eq!(
            LinearScanAllocator::future_uses(&incoming, incoming.start),
            2
        );
    }

    // ======================================================================
    // Position-relative cost model (per-use frequency + suffix costs)
    // ======================================================================

    /// Build a range whose uses carry explicit per-site frequencies.
    fn lr_weighted(
        value_id: u32,
        start: u32,
        end: u32,
        uses_weights: Vec<(u32, u64)>,
    ) -> LiveRange {
        let mut r = lr(value_id, start, end, Vec::new(), 1);
        let (u, w): (Vec<u32>, Vec<u64>) = uses_weights.into_iter().unzip();
        r.set_uses_weighted(u, w);
        r
    }

    /// With no per-use data attached every accessor must reproduce the
    /// pre-existing unit-weight behaviour exactly. This is the compatibility
    /// contract that keeps synthetic/hand-built ranges bit-identical.
    #[test]
    fn cost_model_degrades_to_use_counts_without_weights() {
        let r = lr(1, 0, 100, vec![10, 20, 30, 40], 7);
        assert!(r.use_weights.is_empty());
        assert_eq!(r.total_cost(), 4);
        assert_eq!(r.remaining_cost(0), 4);
        assert_eq!(r.remaining_cost(10), 3);
        assert_eq!(r.remaining_cost(30), 1);
        assert_eq!(r.remaining_cost(40), 0);
        assert_eq!(r.remaining_cost(999), 0);
        // Identical to the historical future-use count at every position.
        for pos in 0..=45u32 {
            assert_eq!(
                r.remaining_cost(pos),
                u64::from(LinearScanAllocator::future_uses(&r, pos)),
                "remaining_cost must equal future_uses at pos {pos}"
            );
        }
    }

    #[test]
    fn suffix_costs_are_exact_and_position_relative() {
        let r = lr_weighted(1, 0, 100, vec![(10, 1), (20, 10), (30, 100), (40, 1)]);
        assert_eq!(r.use_weights, vec![1, 10, 100, 1]);
        assert_eq!(r.suffix_cost, vec![112, 111, 101, 1, 0]);
        assert_eq!(r.total_cost(), 112);
        assert_eq!(r.remaining_cost(0), 112);
        assert_eq!(r.remaining_cost(10), 111);
        assert_eq!(r.remaining_cost(19), 111);
        assert_eq!(r.remaining_cost(20), 101);
        assert_eq!(r.remaining_cost(30), 1);
        assert_eq!(r.remaining_cost(40), 0);
    }

    /// Two reads of the same value by one instruction really do cost twice.
    #[test]
    fn duplicate_use_points_sum_their_weights() {
        let r = lr_weighted(1, 0, 50, vec![(10, 3), (10, 4), (20, 5)]);
        assert_eq!(r.uses, vec![10, 20]);
        assert_eq!(r.use_weights, vec![7, 5]);
        assert_eq!(r.total_cost(), 12);
        assert_eq!(r.remaining_cost(10), 5);
    }

    #[test]
    fn unsorted_weighted_uses_are_sorted_jointly() {
        let r = lr_weighted(1, 0, 50, vec![(30, 100), (10, 1), (20, 10)]);
        assert_eq!(r.uses, vec![10, 20, 30]);
        assert_eq!(r.use_weights, vec![1, 10, 100]);
        assert_eq!(r.remaining_cost(15), 110);
    }

    #[test]
    fn set_uses_invalidates_a_stale_cost_table() {
        let mut r = lr_weighted(1, 0, 50, vec![(10, 1000)]);
        assert_eq!(r.total_cost(), 1000);
        r.set_uses(vec![10, 20, 30]);
        assert!(r.use_weights.is_empty(), "stale weights must be dropped");
        assert_eq!(r.total_cost(), 3, "falls back to the unit-weight count");
        assert_eq!(r.remaining_cost(10), 2);
    }

    #[test]
    fn cost_boost_scales_and_saturates() {
        let mut r = lr_weighted(1, 0, 50, vec![(10, 2), (20, 3)]);
        assert_eq!(r.spill_cost_at(0), 5);
        r.boost_cost(64);
        assert_eq!(r.cost_boost, 64);
        assert_eq!(r.spill_cost_at(0), 320);
        assert_eq!(r.spill_cost_at(10), 192);
        // A zero factor must not zero out the cost.
        r.boost_cost(0);
        assert_eq!(r.cost_boost, 64);
        // Saturation, never wraparound into a bogus *low* cost.
        r.boost_cost(u64::MAX);
        assert_eq!(r.spill_cost_at(0), u64::MAX);
    }

    #[test]
    fn occupied_points_prefers_segment_coverage() {
        let mut r = lr(1, 0, 1000, vec![5, 900], 1);
        assert_eq!(r.occupied_points(), 1001, "no segments -> fat envelope");
        r.set_segments(vec![(0, 9), (900, 909)]);
        assert_eq!(r.occupancy_len, 20);
        assert_eq!(r.occupied_points(), 20);
    }

    // ======================================================================
    // Mode 6: position-relative eviction order
    // ======================================================================

    /// The defect mode-6 exists to fix.
    ///
    /// `victim` is a value with many uses that are ALL behind the scan point:
    /// at position 50 it is dead, so demoting it is free. `incoming` has two
    /// live uses ahead of it. The global-priority order (modes 1-3) protects
    /// the victim because its *lifetime total* is larger, and spills the
    /// incoming instead. The position-relative order sees the victim's
    /// remaining cost is 0 and takes the register.
    #[test]
    fn mode6_evicts_the_value_that_is_dead_at_the_scan_point() {
        let regs = vec![PhysReg(0)];
        let mut victim = lr_weighted(1, 0, 200, vec![(1, 100), (2, 100), (3, 100)]);
        victim.priority = 300;
        let mut incoming = lr_weighted(2, 50, 120, vec![(60, 1), (110, 1)]);
        incoming.priority = 2;

        assert_eq!(victim.remaining_cost(50), 0, "victim is dead at the scan point");
        assert_eq!(incoming.remaining_cost(50), 2);

        // Global-priority order (mode 3): 2 <= 300, so the victim is immune
        // and the incoming is demoted.
        let mut a3 = LinearScanAllocator::new(vec![], regs.clone());
        a3.init_registers();
        a3.allocate_range(victim.clone());
        assert_eq!(a3.select_evict_victim(&incoming, 3), None);

        // Position-relative order (mode 6): the victim costs 0 to demote.
        let mut a6 = LinearScanAllocator::new(vec![], regs);
        a6.init_registers();
        a6.allocate_range(victim);
        assert_eq!(a6.select_evict_victim(&incoming, 6), Some(0));
    }

    /// Mode 6 must still refuse to evict a value that is genuinely hotter
    /// from the scan point onward: the profitability guard is a strict
    /// comparison in the same currency as the ranking.
    #[test]
    fn mode6_never_evicts_a_hotter_remaining_value() {
        let regs = vec![PhysReg(0)];
        // Victim keeps a hot inner-loop use ahead of the scan point.
        let victim = lr_weighted(1, 0, 200, vec![(10, 1), (150, 1000)]);
        let incoming = lr_weighted(2, 50, 190, vec![(60, 1), (70, 1)]);
        assert!(victim.remaining_cost(50) > incoming.remaining_cost(50));

        let mut a = LinearScanAllocator::new(vec![], regs);
        a.init_registers();
        a.allocate_range(victim);
        assert_eq!(a.select_evict_victim(&incoming, 6), None);
    }

    /// Per-use frequency, not one scalar for the whole range: a value with a
    /// single inner-loop use must not outrank a value with many.
    #[test]
    fn mode6_ranks_by_per_use_frequency_not_max_depth() {
        let regs = vec![PhysReg(0), PhysReg(1)];
        // `one_hot`: one use at depth 2 (weight 100) plus cold uses.
        let mut one_hot = lr_weighted(1, 0, 300, vec![(100, 100), (101, 1), (102, 1)]);
        // `many_hot`: four uses at depth 2.
        let mut many_hot =
            lr_weighted(2, 0, 300, vec![(100, 100), (101, 100), (102, 100), (103, 100)]);
        // The legacy scalar model gives BOTH `10^2 * n_uses`, and `one_hot`
        // (3 uses) would even look comparable to `many_hot` (4 uses).
        one_hot.priority = 300;
        many_hot.priority = 400;

        let mut a = LinearScanAllocator::new(vec![], regs);
        a.init_registers();
        a.allocate_range(one_hot.clone());
        a.allocate_range(many_hot.clone());

        // An incoming hotter than `one_hot` but colder than `many_hot` must
        // pick `one_hot` as the victim.
        let incoming = lr_weighted(3, 50, 60, vec![(55, 200)]);
        let pick = a.select_evict_victim(&incoming, 6).expect("a victim exists");
        assert_eq!(
            a.active[pick].range.value_id, 1,
            "must evict the range with the lower REMAINING weighted cost"
        );
    }

    /// Mode 6 keeps every soundness guard of mode 3: ABI-hinted values are
    /// never evicted, and the victim's next use must be past `incoming.end`.
    #[test]
    fn mode6_keeps_the_mode3_soundness_guards() {
        let regs = vec![PhysReg(0)];

        // (a) ABI-hinted victim is immune even at zero remaining cost.
        let mut hinted = lr_weighted(1, 0, 200, vec![(1, 100)]);
        hinted.reg_hint = Some(PhysReg(0));
        let incoming = lr_weighted(2, 50, 120, vec![(60, 1000)]);
        let mut a = LinearScanAllocator::new(vec![], regs.clone());
        a.init_registers();
        a.allocate_range(hinted);
        assert_eq!(a.select_evict_victim(&incoming, 6), None);

        // (b) A victim needed DURING the incoming's lifetime is refused.
        let victim = lr_weighted(3, 0, 200, vec![(1, 100), (80, 1)]);
        let incoming2 = lr_weighted(4, 50, 120, vec![(60, 1000), (119, 1000)]);
        assert!(victim.remaining_cost(50) < incoming2.remaining_cost(50));
        let mut b = LinearScanAllocator::new(vec![], regs);
        b.init_registers();
        b.allocate_range(victim);
        assert_eq!(
            b.select_evict_victim(&incoming2, 6),
            None,
            "next use 80 lies inside [50,120]: reload thrash, refuse"
        );
    }

    /// Modes 1-3 must be untouched by the cost model: same victim, same
    /// refusal, whether or not per-use weights happen to be attached.
    #[test]
    fn legacy_modes_are_unaffected_by_attached_weights() {
        let regs = vec![PhysReg(0)];
        let build = |weighted: bool| {
            let mut victim = if weighted {
                lr_weighted(1, 0, 200, vec![(1, 1), (2, 1)])
            } else {
                lr(1, 0, 200, vec![1, 2], 2)
            };
            victim.priority = 2;
            let mut incoming = if weighted {
                lr_weighted(2, 50, 120, vec![(130, 1), (140, 1), (150, 1)])
            } else {
                lr(2, 50, 120, vec![130, 140, 150], 3)
            };
            incoming.priority = 3;
            (victim, incoming)
        };
        for weighted in [false, true] {
            let (victim, incoming) = build(weighted);
            let mut a = LinearScanAllocator::new(vec![], regs.clone());
            a.init_registers();
            a.allocate_range(victim);
            assert_eq!(
                a.select_evict_victim(&incoming, 3),
                Some(0),
                "mode 3 decision must not depend on the cost table (weighted={weighted})"
            );
        }
    }
}
