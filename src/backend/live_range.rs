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
        }
    }

    /// Install `uses`, enforcing the sorted+unique invariant that the
    /// binary-search helpers depend on.
    pub fn set_uses(&mut self, mut uses: Vec<u32>) {
        uses.sort_unstable();
        uses.dedup();
        self.uses = uses;
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
}

/// Inclusive live-span length. Inverted intervals do not underflow.
#[inline]
fn live_span(start: u32, end: u32) -> f64 {
    end.saturating_sub(start).saturating_add(1).max(1) as f64
}

/// `10^min(depth, 4)`. Cap keeps the ranking strict: depths 5+ must not
/// collapse into one saturated weight and flatten eviction.
#[inline]
fn loop_depth_weight(loop_depth: u32) -> u64 {
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
    /// First use at or after the assignment point. Informational / API;
    /// eviction recomputes against the *current* scan position.
    pub next_use: Option<u32>,
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
    pub spill_slots: FxHashMap<u32, i32>,
    pub available_regs: Vec<PhysReg>,
    pub next_spill_slot: i32,
    /// Reserved. Live-range *splitting* is an IR pre-pass (`split_ranges.rs`),
    /// not this flag. Kept so existing setters do not break.
    pub enable_splitting: bool,
    /// Rotation cursor: consecutive unhinted assignments start at different
    /// registers to expose ILP on Raptor Lake's extra integer ports.
    pub next_reg_idx: usize,
    /// When true, default eviction mode is 5 (exchange) instead of 3.
    /// Phase 2c sets this for the callee-saved overflow wave.
    pub exchange_eviction: bool,
}

impl LinearScanAllocator {
    pub fn new(ranges: Vec<LiveRange>, available_regs: Vec<PhysReg>) -> Self {
        Self {
            ranges,
            active: Vec::new(),
            handled: Vec::new(),
            assignments: FxHashMap::default(),
            reg_free_until: FxHashMap::default(),
            spill_slots: FxHashMap::default(),
            available_regs,
            next_spill_slot: 0,
            enable_splitting: false,
            next_reg_idx: 0,
            exchange_eviction: false,
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
    ///    `range.start`, rotating the start index for ILP.
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
            let free_until = self.reg_free_until.get(&reg).copied().unwrap_or(0);
            if free_until <= range.start {
                self.next_reg_idx = idx + 1;
                return Some(reg);
            }
        }
        None
    }

    /// Whether `reg` belongs to the current allocation class.
    #[inline]
    fn is_allocatable_reg(&self, reg: PhysReg) -> bool {
        self.available_regs.contains(&reg)
    }

    /// Whether `reg` may hold `range` for its whole lifetime.
    ///
    /// 1. **Occupancy.** `reg_free_until[reg] - 1` is the last occupant's
    ///    end. Strictly past `range.start` ⇒ the occupant is still needed
    ///    after `range`'s def. Equality is the die-at-birth case and is
    ///    decided by (2).
    /// 2. **Interference.** No active holder of `reg` may
    ///    [`LiveRange::conflicts_with`] `range`.
    ///
    /// `|active|` is bounded by the register file plus die-at-birth
    /// extras; a per-register index would add sync surface for tens of
    /// comparisons.
    fn register_compatible(&self, reg: PhysReg, range: &LiveRange) -> bool {
        let free_until = self.reg_free_until.get(&reg).copied().unwrap_or(0);
        let occupant_end = free_until.saturating_sub(1);
        if occupant_end > range.start {
            return false;
        }
        self.active
            .iter()
            .filter(|a| self.assignments.get(&a.range.value_id) == Some(&reg))
            .all(|a| !a.range.conflicts_with(range))
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
        !self.active.iter().any(|a| {
            a.range.value_id != evicted_vid
                && self.assignments.get(&a.range.value_id) == Some(&reg)
                && a.range.conflicts_with(incoming)
        })
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
    /// Guards (all must pass):
    /// - steal-safe
    /// - `incoming.priority > victim.priority` (never evict a hotter value)
    /// - mode 1 only: `victim.loop_depth < incoming.loop_depth`
    /// - mode ≥ 3: victim's next use is strictly after `incoming.end`
    fn select_evict_victim(&self, incoming: &LiveRange, mode: i32) -> Option<usize> {
        if self.active.is_empty() || mode <= 0 {
            return None;
        }
        let mut best_idx: Option<usize> = None;
        let mut best_priority = u64::MAX;
        let mut best_next_use = 0u32;
        let mut best_spill_weight = f64::INFINITY;

        for (idx, interval) in self.active.iter().enumerate() {
            if !self.register_steal_is_safe(interval.range.value_id, incoming) {
                continue;
            }
            if incoming.priority <= interval.range.priority {
                continue;
            }
            if mode < 2 && interval.range.loop_depth >= incoming.loop_depth {
                continue;
            }
            let nxt = next_use_after(&interval.range, incoming.start);
            if mode >= 3 && nxt <= incoming.end {
                continue;
            }
            let priority = interval.range.priority;
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
                        if areg == reg && active.range.conflicts_with(&range) {
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

        let mode = evict_mode(self.exchange_eviction);
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
        self.active.push(ActiveInterval {
            range,
            next_use: Some(next),
        });
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
        let evicted = self.active.swap_remove(evict_idx);
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

    /// Like [`Self::run`], but the given `(register, free-until)` pairs are
    /// applied on top of the cleared occupancy BEFORE the allocation loop.
    /// Lets a second allocation wave see the homes already handed out by an
    /// earlier wave (e.g. Phase 2's arg-register-free pass for call-argument
    /// values) so it cannot reuse a register that is still occupied by one of
    /// those values.
    pub fn run_with_seed(&mut self, seed: &FxHashMap<PhysReg, u32>) {
        self.active.clear();
        self.handled.clear();
        self.assignments.clear();
        self.spill_slots.clear();
        self.next_spill_slot = 0;
        self.next_reg_idx = 0;
        self.init_registers();
        for (reg, until) in seed {
            let cur = self.reg_free_until.entry(*reg).or_insert(0);
            *cur = (*cur).max(*until);
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
fn evict_mode(exchange_eviction: bool) -> i32 {
    static MODE: OnceLock<Option<i32>> = OnceLock::new();
    let parsed = *MODE.get_or_init(|| {
        std::env::var("CCC_EVICT_MODE")
            .ok()
            .and_then(|s| s.parse().ok())
    });
    parsed.unwrap_or(if exchange_eviction { 5 } else { 3 })
}

/// LiveInterval → LiveRange: one IR walk for defs, uses, loop depth, hints.
pub fn build_live_ranges(
    intervals: &[LiveInterval],
    loop_depth: &[u32],
    func: &IrFunction,
) -> Vec<LiveRange> {
    let meta = collect_range_metadata(func, loop_depth);
    let pgo_point_weights = pgo_point_weights(func);

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
            range.set_uses(
                uses.iter()
                    .copied()
                    .filter(|&u| u >= range.start && u <= range.end)
                    .collect(),
            );
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

fn record_instruction_uses(
    inst: &Instruction,
    point: u32,
    bdepth: u32,
    meta: &mut RangeMetadata,
) {
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
fn record_terminator_uses(
    term: &Terminator,
    point: u32,
    bdepth: u32,
    meta: &mut RangeMetadata,
) {
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
        };
        r.set_uses(uses);
        r
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
        let mut allocator =
            LinearScanAllocator::new(vec![lr(1, 0, 10, vec![0, 10], 1)], vec![]);
        allocator.run();
        assert!(allocator.spill_slots.contains_key(&1));
        assert!(allocator.assignments.is_empty());
    }

    #[test]
    fn test_run_is_idempotent_and_sorts() {
        let ranges = vec![
            lr(2, 20, 30, vec![20, 30], 1),
            lr(1, 0, 10, vec![0, 10], 1),
        ];
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
        if let (Some(&rb), Some(&rc)) = (
            allocator.assignments.get(&2),
            allocator.assignments.get(&3),
        ) {
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
            next_use: None,
        });
        a.active.push(ActiveInterval {
            range: lr(2, 0, 100, vec![0, 80, 100], 2),
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
        assert_eq!(LinearScanAllocator::future_uses(&incoming, incoming.start), 2);
    }
}
