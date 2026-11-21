//! Shared stack slots for single-block vector temporaries.
//!
//! Pass 1 of the stack layout protects every vector intrinsic result with a
//! unique slot, because reduction vectorization chains (init -> load -> add ->
//! PHI) across blocks can be corrupted by premature slot reuse. A function with
//! a single block has no PHI and no cross-block chain, so the only lifetime that
//! matters is the instruction-index interval `[def, last use]` inside that
//! block. Those values can share slots by interval colouring.
//!
//! Eligibility is deliberately narrow. A value qualifies only when it:
//! * is the result of a 16/32-byte vector intrinsic and is defined exactly once;
//! * is used at least once, after its definition, in the same block;
//! * is not a PHI operand, not a Copy source or destination, and not an
//!   inline-asm output;
//! * is not in the emitter's deferred-store or memory-fold sets. A deferred
//!   value's store is pending and can flush at a later, unrelated point, so its
//!   slot write is not confined to `[def, last use]`. Memory-folded values never
//!   write their slot at all.
//!
//! Dying values (`vector_dying_values`) are kept. Their store, if any, is issued
//! at the definition: a deferred one is the only pending store, and it is
//! excluded above. A destructive consumer that reuses a dying operand's register
//! cancels that operand's pending store (`consumed_pending`), so no write can
//! reach the slot after the last use.

use crate::backend::liveness::{
    for_each_operand_in_instruction, for_each_operand_in_terminator,
    for_each_value_use_in_instruction,
};
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::{Instruction, IrFunction, Operand};

/// Interval of one pooled value: definition index, last-use index, slot size.
pub(super) type PoolInterval = (usize, usize, i64);

/// Value sets that exclude a vector temporary from pooling.
pub(super) struct PoolExclusions<'a> {
    pub defer: &'a FxHashSet<u32>,
    pub memfold: &'a FxHashSet<u32>,
    pub asm_output: &'a FxHashSet<u32>,
}

/// Live intervals of the pool-eligible vector temporaries of a single-block
/// function. Returns an empty map for any function with more than one block
/// or with a PHI.
pub(super) fn single_block_pool_intervals(
    func: &IrFunction,
    ex: &PoolExclusions<'_>,
) -> FxHashMap<u32, PoolInterval> {
    let mut out: FxHashMap<u32, PoolInterval> = FxHashMap::default();
    if func.blocks.len() != 1 {
        return out;
    }
    let block = &func.blocks[0];
    let insts = &block.instructions;
    if insts
        .iter()
        .any(|inst| matches!(inst, Instruction::Phi { .. }))
    {
        return out;
    }

    let mut def_count: FxHashMap<u32, u32> = FxHashMap::default();
    let mut def_at: FxHashMap<u32, (usize, i64)> = FxHashMap::default();
    let mut excluded: FxHashSet<u32> = FxHashSet::default();
    for (idx, inst) in insts.iter().enumerate() {
        if let Some(d) = inst.dest() {
            *def_count.entry(d.0).or_insert(0) += 1;
        }
        match inst {
            Instruction::Intrinsic {
                dest: Some(d), op, ..
            } => match op.vector_home_width() {
                Some(32) => {
                    def_at.insert(d.0, (idx, 32));
                }
                Some(16) => {
                    def_at.insert(d.0, (idx, 16));
                }
                _ => {}
            },
            Instruction::Copy { dest, src } => {
                excluded.insert(dest.0);
                if let Operand::Value(v) = src {
                    excluded.insert(v.0);
                }
            }
            _ => {}
        }
    }

    let n = insts.len();
    let mut last_use: FxHashMap<u32, usize> = FxHashMap::default();
    for (idx, inst) in insts.iter().enumerate() {
        for_each_operand_in_instruction(inst, |op| {
            if let Operand::Value(v) = op {
                last_use.insert(v.0, idx);
            }
        });
        for_each_value_use_in_instruction(inst, |v| {
            last_use.insert(v.0, idx);
        });
    }
    for_each_operand_in_terminator(&block.terminator, |op| {
        if let Operand::Value(v) = op {
            last_use.insert(v.0, n);
        }
    });

    for (&id, &(def_idx, size)) in &def_at {
        if def_count.get(&id) != Some(&1) {
            continue;
        }
        if excluded.contains(&id)
            || ex.defer.contains(&id)
            || ex.memfold.contains(&id)
            || ex.asm_output.contains(&id)
        {
            continue;
        }
        let Some(&use_idx) = last_use.get(&id) else {
            continue;
        };
        if use_idx <= def_idx {
            continue;
        }
        out.insert(id, (def_idx, use_idx, size));
    }
    out
}

/// Assign shared slots to the pooled members of `members` (id, slot size).
///
/// Greedy first-fit colouring by definition index is optimal for interval
/// graphs. A colour is reusable when its previous occupant's last use is
/// strictly before the new definition, so a slot is never handed to a value
/// whose definition could land on an operand still being read. Each colour is
/// allocated once at its largest member size. Returns the ids placed.
pub(super) fn assign_pool_slots(
    value_locations: &mut FxHashMap<u32, crate::backend::state::StackSlot>,
    pool: &FxHashMap<u32, PoolInterval>,
    members: &[(u32, i64)],
    non_local_space: &mut i64,
    assign_slot: &impl Fn(i64, i64, i64) -> (i64, i64),
) -> FxHashSet<u32> {
    let mut placed: FxHashSet<u32> = FxHashSet::default();
    let mut order: Vec<(usize, usize, u32)> = members
        .iter()
        .filter_map(|&(id, _)| pool.get(&id).map(|&(d, u, _)| (d, u, id)))
        .collect();
    if order.is_empty() {
        return placed;
    }
    order.sort_unstable();

    struct Colour {
        last_use: usize,
        size: i64,
        ids: Vec<u32>,
    }
    let mut colours: Vec<Colour> = Vec::new();
    for &(def_idx, use_idx, id) in &order {
        let size = pool.get(&id).map(|p| p.2).unwrap_or(0);
        match colours.iter_mut().find(|c| c.last_use < def_idx) {
            Some(c) => {
                c.last_use = use_idx;
                c.size = c.size.max(size);
                c.ids.push(id);
            }
            None => colours.push(Colour {
                last_use: use_idx,
                size,
                ids: vec![id],
            }),
        }
    }

    for colour in colours {
        // 16-byte alignment matches the other vector slots (legacy-SSE
        // memory-form consumers read vector homes directly).
        let (slot, new_space) = assign_slot(*non_local_space, colour.size, 16);
        *non_local_space = new_space;
        for id in colour.ids {
            value_locations.insert(id, crate::backend::state::StackSlot(slot));
            placed.insert(id);
        }
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::state::StackSlot;

    /// Deterministic xorshift; no external crates, reproducible failures.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    /// Place every member through a stub allocator that records each slot
    /// request, so the test can check the shared-slot geometry directly.
    fn place(
        members: &[(u32, i64)],
        pool: &FxHashMap<u32, PoolInterval>,
    ) -> (FxHashMap<u32, StackSlot>, FxHashSet<u32>, i64) {
        let mut locs: FxHashMap<u32, StackSlot> = FxHashMap::default();
        let mut space = 0i64;
        let alloc = |space: i64, size: i64, _align: i64| (-(space + size), space + size);
        let placed = assign_pool_slots(&mut locs, pool, members, &mut space, &alloc);
        (locs, placed, space)
    }

    fn overlaps(a: PoolInterval, b: PoolInterval) -> bool {
        // Same rule the colourer uses: b may reuse a's colour only when a's
        // last use is strictly before b's definition.
        !(a.1 < b.0 || b.1 < a.0)
    }

    #[test]
    fn strictly_ordered_intervals_share_one_slot() {
        let mut pool = FxHashMap::default();
        pool.insert(1, (2usize, 4usize, 32i64));
        pool.insert(2, (5usize, 9usize, 32i64));
        let (locs, placed, space) = place(&[(1, 32), (2, 32)], &pool);
        assert_eq!(placed.len(), 2);
        assert_eq!(locs[&1].0, locs[&2].0, "disjoint intervals share a slot");
        assert_eq!(space, 32, "one 32-byte slot, not two");
    }

    #[test]
    fn touching_intervals_do_not_share() {
        // a's last use is at index 5 and b is defined at index 5: the two
        // instructions are the same point, so b's write could clobber a's read.
        let mut pool = FxHashMap::default();
        pool.insert(1, (0usize, 5usize, 32i64));
        pool.insert(2, (5usize, 8usize, 32i64));
        let (locs, _, space) = place(&[(1, 32), (2, 32)], &pool);
        assert_ne!(locs[&1].0, locs[&2].0);
        assert_eq!(space, 64);
    }

    #[test]
    fn mixed_widths_use_the_largest_member() {
        let mut pool = FxHashMap::default();
        pool.insert(1, (0usize, 1usize, 16i64));
        pool.insert(2, (2usize, 3usize, 32i64));
        let (locs, _, space) = place(&[(1, 16), (2, 32)], &pool);
        assert_eq!(locs[&1].0, locs[&2].0);
        assert_eq!(space, 32, "colour is sized to its largest member");
    }

    #[test]
    fn non_members_are_not_placed() {
        let mut pool = FxHashMap::default();
        pool.insert(1, (0usize, 3usize, 32i64));
        let (locs, placed, space) = place(&[(1, 32), (7, 32)], &pool);
        assert!(placed.contains(&1));
        assert!(!placed.contains(&7));
        assert!(!locs.contains_key(&7));
        assert_eq!(space, 32);
    }

    #[test]
    fn greedy_colouring_is_optimal_on_random_interval_sets() {
        // Interval graphs are perfect: first-fit by start time uses exactly
        // max-overlap colours. Check the slot count equals the maximum number
        // of simultaneously live intervals, and that no two overlapping
        // intervals ever share a slot.
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for _case in 0..200 {
            let n = 1 + (rng.next() % 40) as usize;
            let mut pool = FxHashMap::default();
            let mut members = Vec::new();
            for id in 0..n as u32 {
                let d = (rng.next() % 64) as usize;
                let u = d + 1 + (rng.next() % 16) as usize;
                let size = if rng.next() % 2 == 0 { 16 } else { 32 };
                pool.insert(id, (d, u, size));
                members.push((id, size));
            }
            let (locs, placed, _) = place(&members, &pool);
            assert_eq!(placed.len(), n);
            for a in 0..n as u32 {
                for b in (a + 1)..n as u32 {
                    if locs[&a].0 == locs[&b].0 {
                        assert!(
                            !overlaps(pool[&a], pool[&b]),
                            "overlapping intervals {a} and {b} share a slot"
                        );
                    }
                }
            }
            // Maximum clique of the interval graph: peak simultaneous liveness.
            let mut events: Vec<(usize, i32)> = Vec::new();
            for iv in pool.values() {
                events.push((iv.0, 1));
                events.push((iv.1 + 1, -1));
            }
            events.sort_unstable();
            let (mut live, mut peak) = (0i32, 0i32);
            for (_, delta) in events {
                live += delta;
                peak = peak.max(live);
            }
            let colours: FxHashSet<i64> = locs.values().map(|s| s.0).collect();
            assert_eq!(colours.len(), peak as usize, "first-fit is optimal here");
        }
    }
}
