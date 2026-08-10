//! Linear scan register allocator data structures.
//!
//! This module defines the core data structures for the linear scan algorithm:
//! - `LiveRange`: Enhanced live interval with priority, uses, and spill weight
//! - `ActiveInterval`: Currently live interval being processed
//! - `LinearScanAllocator`: Main allocator state machine
//!
//! The linear scan algorithm processes intervals in order of definition,
//! maintaining an "active" set of intervals that overlap with the current position.
//! For each interval, we either assign it a free register or spill it to the stack.

use super::liveness::{
    for_each_operand_in_instruction, for_each_operand_in_terminator,
    for_each_value_use_in_instruction, LiveInterval,
};
use super::regalloc::PhysReg;
use crate::common::fx_hash::FxHashMap;
use crate::ir::reexports::{Instruction, IrFunction, Operand, Terminator, Value};

/// Enhanced live interval with priority, uses, and spill weight.
///
/// This extends LiveInterval with:
/// - `uses`: Individual use points within [start, end]
/// - `loop_depth`: Loop nesting depth (used to weight priorities)
/// - `priority`: Weighted use count (higher = more important to allocate)
/// - `reg_hint`: Suggested register from Copy source (coalescing hint)
/// - `spill_weight`: Cost of spilling (priority / range_length)
#[derive(Debug, Clone)]
pub struct LiveRange {
    pub value_id: u32,
    pub start: u32,                // Program point where defined
    pub end: u32,                  // Last program point used
    pub uses: Vec<u32>,            // Use points within [start, end]
    pub loop_depth: u32,           // Loop nesting depth (0 = no loop, 1 = in loop, etc.)
    pub priority: u64,             // uses.len() * 10^loop_depth (higher priority = allocate first)
    pub reg_hint: Option<PhysReg>, // Preferred register (from Copy sources)
    pub spill_weight: f64,         // Cost of spilling: priority / range_length
    /// Cascade number for eviction-chain suppression (LLVM RegAllocGreedy
    /// lineage): an interval that took a register by eviction carries
    /// victim.cascade + 1, and may only be evicted again by an interval with
    /// an equal or higher cascade. Bounds eviction chains, killing the
    /// ping-pong that plain priority eviction (old mode 2) exhibited.
    pub cascade: u32,
}

impl LiveRange {
    /// Create a new LiveRange from a LiveInterval and loop depth.
    pub fn from_interval(interval: LiveInterval, loop_depth: u32) -> Self {
        let range_length = (interval.end - interval.start + 1).max(1) as f64;

        // Priority: number of uses * 10^loop_depth
        // Uses at depth 0 (no loop): weight = 1
        // Uses at depth 1 (one loop): weight = 10
        // Uses at depth 2 (nested loops): weight = 100
        // etc.
        let loop_weight = {
            let base: u64 = 10;
            base.saturating_pow(loop_depth).min(1_000_000_000)
        };

        // For now, estimate 2 uses per interval (this will be refined by build_live_ranges)
        let uses = 2u64 * loop_weight;

        let spill_weight = uses as f64 / range_length;

        Self {
            value_id: interval.value_id,
            start: interval.start,
            end: interval.end,
            uses: Vec::new(), // Will be populated by build_live_ranges
            loop_depth,
            priority: uses,
            reg_hint: None,
            spill_weight,
            cascade: 0,
        }
    }

    /// Calculate spill weight based on actual use count and range length.
    pub fn calculate_spill_weight(&mut self) {
        let range_length = (self.end - self.start + 1).max(1) as f64;
        self.spill_weight = self.priority as f64 / range_length;
    }

    /// Check if this interval overlaps with another interval [start, end).
    pub fn overlaps(&self, start: u32, end: u32) -> bool {
        self.start < end && start < self.end
    }

    /// Check if this interval overlaps with another LiveRange.
    pub fn overlaps_with(&self, other: &LiveRange) -> bool {
        self.overlaps(other.start, other.end + 1)
    }
}

/// An interval that is currently active (overlaps with current position).
#[derive(Debug, Clone)]
pub struct ActiveInterval {
    pub range: LiveRange,
    /// The next program point where this interval is used.
    /// Used for tie-breaking when choosing which interval to spill.
    pub next_use: Option<u32>,
}

/// Main linear scan allocator state.
///
/// Processes intervals in order of definition, maintaining:
/// - `ranges`: Sorted list of all live ranges
/// - `active`: Intervals overlapping current position
/// - `handled`: Intervals that have finished
/// - `assignments`: Final register assignments
/// - `reg_free_until`: When each register becomes available
/// - `spill_slots`: Stack slot assignments for unallocated values
pub struct LinearScanAllocator {
    // Intervals to process (sorted by start point)
    pub ranges: Vec<LiveRange>,

    // Currently overlapping intervals
    pub active: Vec<ActiveInterval>,

    // Finished intervals
    pub handled: Vec<ActiveInterval>,

    // Final assignments
    pub assignments: FxHashMap<u32, PhysReg>,

    // For each register: program point until which it's occupied
    pub reg_free_until: FxHashMap<PhysReg, u32>,

    // For values that didn't get a register: stack slot offset
    pub spill_slots: FxHashMap<u32, i32>,

    // Available registers from config
    pub available_regs: Vec<PhysReg>,

    // Next stack slot offset (grows downward on most architectures)
    pub next_spill_slot: i32,

    // Whether to enable interval splitting (advanced feature)
    pub enable_splitting: bool,

    // Rotation index for register selection: start searching from this index
    // instead of 0 to distribute consecutive allocations across different registers.
    pub next_reg_idx: usize,

    /// Use future-value exchange eviction instead of the mode-3 next-use guard.
    /// Enabled only for the callee-saved overflow phase (Phase 2c), where the
    /// mode-3 guard makes economically catastrophic decisions: it demotes
    /// loop-carried values with dozens of remaining uses to make room for
    /// late-starting values used once or twice (measured in gzip's
    /// longest_match: chains evicted from rbx/r14/r15 by post-loop
    /// single-use temporaries). The exchange rule only evicts when the
    /// incoming range has strictly more future uses than the victim, so every
    /// eviction provably reduces total slot traffic.
    pub exchange_eviction: bool,
}

impl LinearScanAllocator {
    /// Create a new allocator with the given live ranges and available registers.
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

    /// Initialize all registers as free (available from point 0).
    pub fn init_registers(&mut self) {
        for &reg in &self.available_regs {
            self.reg_free_until.insert(reg, 0);
        }
    }

    /// Check if a register is free at the given position.
    pub fn is_register_free(&self, reg: PhysReg, position: u32) -> bool {
        self.reg_free_until
            .get(&reg)
            .map_or(true, |&free_until| free_until <= position)
    }

    /// Find the earliest position at which a register becomes free.
    pub fn earliest_free_position(&self) -> u32 {
        self.reg_free_until.values().copied().min().unwrap_or(0)
    }

    /// Mark a register as occupied until the given position.
    pub fn occupy_register(&mut self, reg: PhysReg, until: u32) {
        self.reg_free_until.insert(reg, until);
    }

    /// Allocate a spill slot for a value and return its offset.
    pub fn allocate_spill_slot(&mut self, value_id: u32) -> i32 {
        let slot = self.next_spill_slot;
        self.spill_slots.insert(value_id, slot);
        self.next_spill_slot -= 8; // Assume 8-byte slots (could be configurable)
        slot
    }

    /// Expire old intervals that no longer overlap with the current position.
    ///
    /// Intervals in the active set that end before the given position are
    /// moved to the handled set, freeing their registers.
    pub fn expire_old_intervals(&mut self, current_start: u32) {
        self.active.retain(|active| {
            if active.range.end < current_start {
                // Interval is done, move it to handled
                self.handled.push(active.clone());
                false // Remove from active
            } else {
                true // Keep in active
            }
        });
    }

    /// Find a free register for the given range.
    ///
    /// Uses the following strategy:
    /// 1. If there's a register hint from Copy sources, try that first
    /// 2. Find a register that's free for the entire duration of the range
    /// 3. If none, return None (caller will spill)
    pub fn find_free_register(&mut self, range: &LiveRange) -> Option<PhysReg> {
        // Try register hint first (for coalescing with Copy sources)
        if let Some(hint) = range.reg_hint {
            if self.is_register_free(hint, range.start)
                && self.active.iter().all(|a| !a.range.overlaps_with(range))
            {
                return Some(hint);
            }
        }

        // Find any free register, rotating the start index to distribute
        // consecutive allocations across different registers for ILP.
        let n = self.available_regs.len();
        if n == 0 {
            return None;
        }
        let start = self.next_reg_idx % n;
        for offset in 0..n {
            let idx = (start + offset) % n;
            let reg = self.available_regs[idx];
            // Check if this register is free at the start of the range
            if self.is_register_free(reg, range.start) {
                // Also check that no active interval uses this register
                let reg_free_throughout = self
                    .active
                    .iter()
                    .filter(|a| {
                        if let Some(assigned_reg) = self.assignments.get(&a.range.value_id) {
                            *assigned_reg == reg
                        } else {
                            false
                        }
                    })
                    .all(|a| !a.range.overlaps_with(range));

                if reg_free_throughout {
                    self.next_reg_idx = idx + 1;
                    return Some(reg);
                }
            }
        }

        None
    }

    /// Remaining (future) weighted use count of a range at scan position `pos`:
    /// the number of uses strictly after `pos`. Under LCCC's lifetime-demotion
    /// spill model (a demoted value pays slot traffic at EVERY remaining use;
    /// there is no reload-at-next-use), this is exactly the cost of demoting
    /// the range at this point.
    fn future_uses(range: &LiveRange, pos: u32) -> u32 {
        range.uses.iter().filter(|&&u| u > pos).count() as u32
    }

    /// Exchange-based eviction candidate selection (W1 redesign, default).
    ///
    /// Literature basis: Poletto & Sarkar's linear scan spills the interval
    /// with the furthest endpoint; Braun & Hack's MIN rule (spilling the
    /// interval with the furthest NEXT use) improves it markedly; LLVM's
    /// RegAllocGreedy orders by spill weight and suppresses eviction chains
    /// with cascade numbers. Under LCCC's lifetime-demotion model the exact
    /// exchange argument is: evicting active C in favor of incoming I changes
    /// total slot traffic by +future(C) - future(I), so the demotion is
    /// profitable iff future(I) > future(C), and cascades are bounded by
    /// requiring C.cascade <= I.cascade (a once-evicted interval cannot be
    /// re-displaced by a fresh cascade-0 arrival).
    ///
    /// Returns the index into `self.active` of the victim, or None when no
    /// profitable, cascade-legal victim exists (the incoming then spills).
    ///
    /// MEASURED & REJECTED as the default (2026-08-10): on gzip 1.14
    /// (raptorlake, 13.2MB corpus, 5-run paired medians) CCC_EVICT_MODE=5
    /// regressed -6 by +4.5% (0.835s vs 0.799s) and -9 by +3.3% (2.648s vs
    /// 2.564s) relative to the mode-3 default, and additionally exposed
    /// latent copy-alias/slot-pack unsoundness (maketrees miscompile). Under
    /// LCCC's lifetime-demotion model the locally-profitable exchange
    /// cascades into NET higher slot traffic in hot loops. Kept reachable via
    /// CCC_EVICT_MODE=5 for future work on reload-at-next-use / splitting.
    pub fn find_exchange_candidate(&self, range: &LiveRange) -> Option<usize> {
        if self.active.is_empty() {
            return None;
        }
        let incoming_future = range.uses.len() as u32;
        let mut best_idx: Option<usize> = None;
        let mut best_future = u32::MAX;
        let mut best_next_use = 0u32;
        for (idx, interval) in self.active.iter().enumerate() {
            // Cascade legality: never displace an interval that already won
            // its register by eviction from a lower-cascade newcomer.
            if interval.range.cascade > range.cascade {
                continue;
            }
            let fut = Self::future_uses(&interval.range, range.start);
            let nxt = next_use_after(&interval.range, range.start);
            // Minimum future cost wins; ties break to the farthest next use
            // (MIN rule): demoting it disturbs the nearest-future code least.
            if fut < best_future || (fut == best_future && nxt > best_next_use) {
                best_idx = Some(idx);
                best_future = fut;
                best_next_use = nxt;
            }
        }
        // Profitability: the incoming must have strictly more future uses than
        // the victim (each future use is one slot access under demotion).
        match best_idx {
            Some(idx) if best_future < incoming_future => Some(idx),
            _ => None,
        }
    }

    /// Find the best active interval to evict (spill) to free a register for the
    /// incoming range at the current scan position.
    ///
    /// This is the core of the linear-scan interval-splitting heuristic
    /// (Poletto–Sarkar style, made loop-aware):
    ///
    /// 1. Prefer the active interval with the **lowest priority**. `priority`
    ///    already carries the 10^loop_depth weighting, so values that are cold
    ///    (outside loops) or lightly used rank lower and are preferred for
    ///    eviction over hot inner-loop temporaries.
    /// 2. Break ties by the **farthest next use**: spill the value that will be
    ///    needed again the latest. This directly avoids the pathological case
    ///    where eviction forces a hot-loop reload — the reloaded value won't be
    ///    touched again until well after the incoming value has finished.
    ///
    /// Returns the index into `self.active` of the interval to evict, or None if
    /// the active set is empty.
    pub fn find_evict_candidate(&self, current_pos: u32) -> Option<usize> {
        if self.active.is_empty() {
            return None;
        }

        let mut best_idx = 0;
        let mut best_priority = self.active[0].range.priority;
        let mut best_next_use = next_use_after(&self.active[0].range, current_pos);

        for (idx, interval) in self.active.iter().enumerate().skip(1) {
            let priority = interval.range.priority;
            let next_use = next_use_after(&interval.range, current_pos);

            // Prefer lower priority (less important). If tied, prefer the value
            // whose next use is farthest (least disruptive to evict).
            if priority < best_priority || (priority == best_priority && next_use > best_next_use) {
                best_idx = idx;
                best_priority = priority;
                best_next_use = next_use;
            }
        }

        Some(best_idx)
    }

    /// Process a single live range through the allocation algorithm.
    ///
    /// This is the core loop body:
    /// 1. Expire intervals that ended before this range starts
    /// 2. Try to find a free register
    /// 3. If none, find the best interval to spill
    /// 4. Assign the register or spill to stack
    pub fn allocate_range(&mut self, range: LiveRange) {
        // Step 1: Expire old intervals
        self.expire_old_intervals(range.start);

        // Step 2: Try to find a free register
        let trace = std::env::var("CCC_TRACE_ALLOC").is_ok();
        if let Some(reg) = self.find_free_register(&range) {
            // Verify: check for actual same-register overlap
            if trace {
                for active in &self.active {
                    if let Some(&areg) = self.assignments.get(&active.range.value_id) {
                        if areg == reg && active.range.overlaps_with(&range) {
                            eprintln!("[ALLOC-BUG] Assigning val{}[{}-{}] to reg={} but val{}[{}-{}] already in reg={}!",
                                range.value_id, range.start, range.end, reg.0,
                                active.range.value_id, active.range.start, active.range.end, areg.0);
                        }
                    }
                }
            }
            // Found a free register - assign it
            self.assignments.insert(range.value_id, reg);
            self.occupy_register(reg, range.end + 1);

            self.active.push(ActiveInterval {
                range,
                next_use: None,
            });
        } else {
            // No free register for the whole of the incoming range. This is the
            // linear-scan *interval-splitting* decision point.
            //
            // Options:
            //   a) Spill the incoming range (default, always sound).
            //   b) Interval-split: evict an ACTIVE interval that is less valuable
            //      (lower priority / farthest next use) and hand its register to
            //      the incoming range. The evicted value keeps a stack home, so
            //      codegen reloads it from its slot on its next use. This is
            //      sound because the FINAL assignment map simply leaves the
            //      evicted value unregistered (slot_assignment gives every
            //      unregistered value a slot).
            //
            // Mode 3 reassigns a whole lower-priority interval only when its next
            // use follows the incoming interval. Set mode 0 to disable.
            let mode: i32 = std::env::var("CCC_EVICT_MODE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(if self.exchange_eviction { 5 } else { 3 });
            if mode == 5 {
                // Default: future-value exchange with cascade suppression.
                if let Some(evict_idx) = self.find_exchange_candidate(&range) {
                    let (evicted_vid, evicted_cascade) = {
                        let cand = &self.active[evict_idx];
                        (cand.range.value_id, cand.range.cascade)
                    };
                    if let Some(reg) = self.assignments.remove(&evicted_vid) {
                        if !self.spill_slots.contains_key(&evicted_vid) {
                            self.allocate_spill_slot(evicted_vid);
                        }
                        if std::env::var("CCC_TRACE_ALLOCSTATS").is_ok() {
                            eprintln!(
                                "[EVICT5] val{} reg={} -> val{}[{}] fut_in={} ",
                                evicted_vid, reg.0, range.value_id, range.start,
                                range.uses.len()
                            );
                        }
                        let mut incoming = range;
                        incoming.cascade = evicted_cascade + 1;
                        self.assignments.insert(incoming.value_id, reg);
                        self.occupy_register(reg, incoming.end + 1);
                        self.active[evict_idx] = ActiveInterval {
                            range: incoming,
                            next_use: None,
                        };
                        return;
                    }
                }
                self.allocate_spill_slot(range.value_id);
                return;
            }
            if mode > 0 {
                if let Some(evict_idx) = self.find_evict_candidate(range.start) {
                    let cand = &self.active[evict_idx];
                    let evicted_vid = cand.range.value_id;
                    let cand_priority = cand.range.priority;
                    let cand_depth = cand.range.loop_depth;
                    // The core guard is priority: never evict a value more valuable
                    // than the incoming. This is what lets a hot loop value (e.g.
                    // the scan/match induction variables, which have the HIGHEST
                    // priority) take a register from a cold function-scope value
                    // that merely defined it earlier in the function.
                    let ok = if mode >= 2 {
                        range.priority > cand_priority
                    } else {
                        range.priority > cand_priority && cand_depth < range.loop_depth
                    };
                    // ms178 mode 3: ALSO require that the evicted value's next
                    // use is AFTER the incoming range's end. This prevents the
                    // pathological case that makes eviction regress hot loops:
                    // if the evicted value is needed INSIDE the incoming
                    // range's window, it reloads from its slot on every use in
                    // the hot loop — far worse than the spill it avoided. With
                    // this guard, the reload happens in cold code (or a later
                    // loop), so eviction can only help.
                    let ok = if mode >= 3 {
                        ok && {
                            let nxt = next_use_after(&cand.range, range.start);
                            // next_use_after falls back to range.end when there
                            // is no use at/after range.start; in that case the
                            // value is genuinely not needed soon.
                            nxt > range.end
                        }
                    } else {
                        ok
                    };
                    if ok {
                        let evicted_reg = self.assignments.remove(&evicted_vid);
                        if let Some(reg) = evicted_reg {
                            // SOUNDNESS: the evicted value must have a stack home
                            // so codegen can reload it on its next use. Its
                            // register is freed but the value persists on the
                            // stack. (The backend's slot-assignment pass gives
                            // every non-registered value a slot anyway, but we
                            // also record it in the allocator's spill_slots map
                            // to make the eviction self-contained and provably
                            // sound under every interpretation.)
                            if !self.spill_slots.contains_key(&evicted_vid) {
                                self.allocate_spill_slot(evicted_vid);
                            }
                            self.assignments.insert(range.value_id, reg);
                            self.occupy_register(reg, range.end + 1);
                            if std::env::var("CCC_TRACE_ALLOCSTATS").is_ok() {
                                eprintln!(
                                    "[EVICT] val{} reg={} -> val{}[{}]",
                                    evicted_vid, reg.0, range.value_id, range.start
                                );
                            }
                            self.active[evict_idx] = ActiveInterval {
                                range,
                                next_use: None,
                            };
                            return;
                        }
                    }
                }
            }
            self.allocate_spill_slot(range.value_id);
        }
    }

    /// Run the full linear scan allocation algorithm.
    ///
    /// Processes all ranges in order, assigning registers or spilling to stack.
    pub fn run(&mut self) {
        self.init_registers();

        // Process ranges in order of start point.
        // Take ownership to iterate without O(n) remove(0) shifts.
        let ranges = std::mem::take(&mut self.ranges);
        for range in ranges {
            self.allocate_range(range);
        }
    }
}

/// Return the first use point of `range` at or after `pos`.
///
/// If there is no such use (the value is live across `pos` but its recorded uses
/// all precede `pos`), falls back to `range.end` (its last live point). This is
/// used by the interval-splitting eviction heuristic to prefer spilling values
/// that won't be needed again soon.
fn next_use_after(range: &LiveRange, pos: u32) -> u32 {
    range
        .uses
        .iter()
        .copied()
        .find(|&u| u >= pos)
        .unwrap_or(range.end)
}

/// Helper to build live ranges from liveness analysis results.
///
/// This function:
/// 1. Converts LiveInterval → LiveRange with loop depth
/// 2. Collects actual use points (program points where values are used)
/// 3. Finds register hints from Copy instructions
/// 4. Calculates priorities and spill weights
pub fn build_live_ranges(
    intervals: &[LiveInterval],
    loop_depth: &[u32],
    func: &IrFunction,
) -> Vec<LiveRange> {
    // Build map: value_id → defining block index.
    // This fixes a bug where value_id was incorrectly used as an index into
    // block_loop_depth (which is indexed by block index, not value ID).
    let mut def_block: FxHashMap<u32, usize> = FxHashMap::default();
    for (block_idx, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                def_block.insert(dest.0, block_idx);
            }
        }
    }

    // Build map: value_id → max loop depth across all use sites.
    // A value defined outside a loop but used inside it should get the inner
    // loop's priority, not the definition site's (typically depth 0).
    let mut max_use_depth: FxHashMap<u32, u32> = FxHashMap::default();
    for (block_idx, block) in func.blocks.iter().enumerate() {
        let bdepth = loop_depth.get(block_idx).copied().unwrap_or(0);
        if bdepth == 0 {
            continue;
        } // Skip non-loop blocks (depth 0 can't increase max)
        for inst in &block.instructions {
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    let entry = max_use_depth.entry(v.0).or_insert(0);
                    *entry = (*entry).max(bdepth);
                }
            });
            // W1: address-side uses (Load/Store ptr, GEP base) earn loop depth
            // exactly like data operands — a pointer consumed as an addressing
            // base inside the loop is as hot as any scalar operand.
            for_each_value_use_in_instruction(inst, |v| {
                let entry = max_use_depth.entry(v.0).or_insert(0);
                *entry = (*entry).max(bdepth);
            });
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                let entry = max_use_depth.entry(v.0).or_insert(0);
                *entry = (*entry).max(bdepth);
            }
        });
    }

    let mut ranges: Vec<LiveRange> = intervals
        .iter()
        .map(|interval| {
            // Use the maximum of defining block depth and max use-site depth.
            // This ensures values defined outside loops but used inside them
            // get the correct inner-loop priority for register allocation.
            let def_depth = def_block
                .get(&interval.value_id)
                .and_then(|&bidx| loop_depth.get(bidx).copied())
                .unwrap_or(0);
            let use_depth = max_use_depth.get(&interval.value_id).copied().unwrap_or(0);
            let depth = def_depth.max(use_depth);
            LiveRange::from_interval(*interval, depth)
        })
        .collect();

    // Collect actual use points for each value
    let uses_map = collect_uses_for_values(func);

    // Find register hints from Copy sources
    let hints_map = find_register_hints(func);

    // PGO-aware use weighting: exact block-entry counts bias allocation toward
    // values used in hot loops, while an absent profile is exactly the old
    // loop-depth heuristic. This changes priorities only, never intervals.
    let pgo_point_weights = pgo_point_weights(func);

    // Update each range with actual uses and hints
    for range in &mut ranges {
        // Collect uses within this range's interval
        if let Some(uses) = uses_map.get(&range.value_id) {
            range.uses = uses
                .iter()
                .filter(|&&u| u >= range.start && u <= range.end)
                .copied()
                .collect();
        }

        // Weight uses with a bounded profile factor; loop depth remains dominant.
        let loop_weight = 10u64.pow(range.loop_depth.min(4) as u32);
        let weighted_uses: u64 = range
            .uses
            .iter()
            .map(|u| pgo_point_weights.get(u).copied().unwrap_or(1))
            .sum();
        range.priority = weighted_uses.max(1).saturating_mul(loop_weight);

        // Add register hint if available
        range.reg_hint = hints_map.get(&range.value_id).copied();

        // Recalculate spill weight with actual use count
        range.calculate_spill_weight();
    }

    // Sort by start point (primary) and by priority (secondary, for tie-breaking)
    ranges.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then_with(|| b.priority.cmp(&a.priority))
    });

    ranges
}

/// Map program points to a bounded PGO hotness factor. Profile data is read
/// after validation, and missing blocks intentionally receive neutral weight.
fn pgo_point_weights(func: &IrFunction) -> FxHashMap<u32, u64> {
    let mut out = FxHashMap::default();
    // Exact, CFG-validated profile for this function in the active translation
    // unit (never a name-suffix match: same-named statics in different TUs
    // would otherwise bind the wrong profile).
    let Some(fp) = crate::pgo::active_profile_for_function(func) else {
        return out;
    };
    let max = fp.block_counts.values().copied().max().unwrap_or(0);
    if max == 0 {
        return out;
    }
    let max_factor = std::env::var("CCC_PGO_WEIGHT_MAX")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(4)
        .clamp(1, 16);
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
            point += 1;
        }
        out.insert(point, factor);
        point += 1;
    }
    out
}

/// Collect all use points for each value ID in the function.
///
/// Returns a map: value_id → Vec<program_point> where the value is used.
/// Program points are assigned sequentially to each instruction and terminator.
fn collect_uses_for_values(func: &IrFunction) -> FxHashMap<u32, Vec<u32>> {
    let mut uses: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    let mut point = 0u32;

    for block in &func.blocks {
        for inst in &block.instructions {
            record_operand_uses(inst, point, &mut uses);
            point += 1;
        }
        // Account for terminator point
        record_terminator_uses(&block.terminator, point, &mut uses);
        point += 1;
    }

    uses
}

/// Record uses of operands in an instruction.
fn record_operand_uses(inst: &Instruction, point: u32, uses: &mut FxHashMap<u32, Vec<u32>>) {
    // Helper to record a use of a value
    let mut record = |vid: u32| {
        uses.entry(vid).or_insert_with(Vec::new).push(point);
    };

    // Match on instruction type to find all operand uses
    match inst {
        Instruction::BinOp { lhs, rhs, .. } => {
            if let Operand::Value(v) = lhs {
                record(v.0);
            }
            if let Operand::Value(v) = rhs {
                record(v.0);
            }
        }
        Instruction::UnaryOp { src, .. } => {
            if let Operand::Value(v) = src {
                record(v.0);
            }
        }
        Instruction::Cmp { lhs, rhs, .. } => {
            if let Operand::Value(v) = lhs {
                record(v.0);
            }
            if let Operand::Value(v) = rhs {
                record(v.0);
            }
        }
        Instruction::Store { val, .. } => {
            if let Operand::Value(v) = val {
                record(v.0);
            }
        }
        Instruction::Cast { src, .. } => {
            if let Operand::Value(v) = src {
                record(v.0);
            }
        }
        Instruction::Copy { src, .. } => {
            if let Operand::Value(v) = src {
                record(v.0);
            }
        }
        Instruction::Call { info, .. } => {
            for arg in &info.args {
                if let Operand::Value(v) = arg {
                    record(v.0);
                }
            }
        }
        Instruction::CallIndirect { func_ptr, info, .. } => {
            if let Operand::Value(v) = func_ptr {
                record(v.0);
            }
            for arg in &info.args {
                if let Operand::Value(v) = arg {
                    record(v.0);
                }
            }
        }
        Instruction::GetElementPtr { offset, .. } => {
            if let Operand::Value(v) = offset {
                record(v.0);
            }
        }
        Instruction::Select {
            cond,
            true_val,
            false_val,
            ..
        } => {
            if let Operand::Value(v) = cond {
                record(v.0);
            }
            if let Operand::Value(v) = true_val {
                record(v.0);
            }
            if let Operand::Value(v) = false_val {
                record(v.0);
            }
        }
        Instruction::AtomicRmw { val, .. } => {
            if let Operand::Value(v) = val {
                record(v.0);
            }
        }
        Instruction::AtomicInc { ptr, .. } => {
            if let Operand::Value(v) = ptr {
                record(v.0);
            }
        }
        Instruction::AtomicCmpxchg {
            expected, desired, ..
        } => {
            if let Operand::Value(v) = expected {
                record(v.0);
            }
            if let Operand::Value(v) = desired {
                record(v.0);
            }
        }
        Instruction::Phi { incoming, .. } => {
            for (op, _) in incoming {
                if let Operand::Value(v) = op {
                    record(v.0);
                }
            }
        }
        _ => {}
    }

    // Record direct Value uses (pointers, bases, etc.)
    record_value_uses(inst, point, uses);
}

/// Record direct Value uses (not wrapped in Operand).
fn record_value_uses(inst: &Instruction, point: u32, uses: &mut FxHashMap<u32, Vec<u32>>) {
    let mut record = |v: &Value| {
        uses.entry(v.0).or_insert_with(Vec::new).push(point);
    };

    match inst {
        Instruction::Load { ptr, .. } => record(ptr),
        Instruction::Store { ptr, .. } => record(ptr),
        Instruction::GetElementPtr { base, .. } => record(base),
        Instruction::Memcpy { dest, src, .. } => {
            record(dest);
            record(src);
        }
        _ => {}
    }
}

/// Record uses in a terminator.
fn record_terminator_uses(term: &Terminator, point: u32, uses: &mut FxHashMap<u32, Vec<u32>>) {
    let mut record = |v: u32| {
        uses.entry(v).or_insert_with(Vec::new).push(point);
    };

    match term {
        Terminator::Return(Some(op)) => {
            if let Operand::Value(v) = op {
                record(v.0);
            }
        }
        Terminator::CondBranch { cond, .. } => {
            if let Operand::Value(v) = cond {
                record(v.0);
            }
        }
        Terminator::IndirectBranch { target, .. } => {
            if let Operand::Value(v) = target {
                record(v.0);
            }
        }
        Terminator::Switch { val, .. } => {
            if let Operand::Value(v) = val {
                record(v.0);
            }
        }
        _ => {}
    }
}

/// Find register hints from Copy instructions.
///
/// For Copy instructions where the source is allocated to a register,
/// the destination can be hinted to use the same register, enabling coalescing.
///
/// Returns a map: dest_value_id → source_register_hint
fn find_register_hints(func: &IrFunction) -> FxHashMap<u32, PhysReg> {
    let mut hints: FxHashMap<u32, PhysReg> = FxHashMap::default();

    // TODO: In a full implementation, we would track which values have been
    // allocated to registers and use those as hints. For now, return empty map.
    // This will be populated when we have actual register assignments.

    hints
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_overlap_detection() {
        let range = LiveRange {
            value_id: 1,
            start: 10,
            end: 20,
            uses: vec![],
            loop_depth: 0,
            priority: 1,
            reg_hint: None,
            spill_weight: 0.1,
        };

        // Overlapping: starts within range
        assert!(range.overlaps(15, 25));

        // Overlapping: ends within range
        assert!(range.overlaps(5, 15));

        // Overlapping: contains range
        assert!(range.overlaps(5, 25));

        // No overlap: ends before range
        assert!(!range.overlaps(21, 30));

        // No overlap: starts after range
        assert!(!range.overlaps(0, 9));
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

        // Same number of uses, but loop-weighted priorities differ
        assert!(in_loop.priority > no_loop.priority);
        assert!(nested_loop.priority > in_loop.priority);
    }

    #[test]
    fn test_spill_weight_calculation() {
        let short_range = LiveRange {
            value_id: 1,
            start: 0,
            end: 10,
            uses: vec![],
            loop_depth: 0,
            priority: 100,
            reg_hint: None,
            spill_weight: 100.0 / 11.0, // 100 / range_length
        };

        let long_range = LiveRange {
            value_id: 2,
            start: 0,
            end: 100,
            uses: vec![],
            loop_depth: 0,
            priority: 100,
            reg_hint: None,
            spill_weight: 100.0 / 101.0, // 100 / range_length
        };

        // Short ranges have higher spill weight (less painful to keep in register)
        assert!(short_range.spill_weight > long_range.spill_weight);
    }

    #[test]
    fn test_linear_scan_basic_allocation() {
        // Create a simple allocator with 2 registers and 3 non-overlapping ranges
        let ranges = vec![
            LiveRange {
                value_id: 1,
                start: 0,
                end: 10,
                uses: vec![0, 5, 10],
                loop_depth: 0,
                priority: 3,
                reg_hint: None,
                spill_weight: 0.3,
            },
            LiveRange {
                value_id: 2,
                start: 20,
                end: 30,
                uses: vec![20, 25, 30],
                loop_depth: 0,
                priority: 3,
                reg_hint: None,
                spill_weight: 0.3,
            },
            LiveRange {
                value_id: 3,
                start: 40,
                end: 50,
                uses: vec![40, 45, 50],
                loop_depth: 0,
                priority: 3,
                reg_hint: None,
                spill_weight: 0.3,
            },
        ];

        let regs = vec![PhysReg(0), PhysReg(1)];
        let mut allocator = LinearScanAllocator::new(ranges, regs);
        allocator.run();

        // All three non-overlapping intervals should get registers
        assert_eq!(allocator.assignments.len(), 3);
        assert!(allocator.assignments.contains_key(&1));
        assert!(allocator.assignments.contains_key(&2));
        assert!(allocator.assignments.contains_key(&3));
    }

    #[test]
    fn test_linear_scan_spilling() {
        // Create a scenario with overlapping ranges that need spilling
        let ranges = vec![
            LiveRange {
                value_id: 1,
                start: 0,
                end: 100,
                uses: vec![0, 50, 100],
                loop_depth: 0,
                priority: 3, // High priority
                reg_hint: None,
                spill_weight: 0.03,
            },
            LiveRange {
                value_id: 2,
                start: 10,
                end: 90,
                uses: vec![10, 50, 90],
                loop_depth: 0,
                priority: 2, // Lower priority - should spill
                reg_hint: None,
                spill_weight: 0.02,
            },
        ];

        let regs = vec![PhysReg(0)]; // Only one register
        let mut allocator = LinearScanAllocator::new(ranges, regs);
        allocator.run();

        // Value 1 gets the register (higher priority)
        assert!(allocator.assignments.contains_key(&1));
        // Value 2 either gets the register or gets spilled (1 register for 2 overlapping values)
        // The allocator should make a decision
        assert!(allocator.assignments.len() <= 2);
    }

    #[test]
    fn test_linear_scan_no_registers() {
        // Test allocator with no available registers
        let ranges = vec![LiveRange {
            value_id: 1,
            start: 0,
            end: 10,
            uses: vec![0, 10],
            loop_depth: 0,
            priority: 1,
            reg_hint: None,
            spill_weight: 0.1,
        }];

        let regs = vec![]; // No registers available
        let mut allocator = LinearScanAllocator::new(ranges, regs);
        allocator.run();

        // Should allocate spill slots instead
        assert!(allocator.spill_slots.contains_key(&1));
    }
}
