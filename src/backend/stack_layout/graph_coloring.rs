//! Hole-aware stack-slot coloring for multi-block SSA values.
//!
//! Register allocation and stack allocation have different boundary rules.
//! A register can sometimes be handed from a value used at point P to a value
//! defined at P because the emitter reads before it writes. A stack slot is
//! more conservative: hidden accumulator stores/reloads can occur around that
//! IR point, so two slot lifetimes touching at P are considered interfering.
//! This closed-boundary rule avoids the historical SQLite collision while
//! still recovering the large win that matters: mutually exclusive CFG arms
//! have disjoint liveness *segments* even when their fat intervals overlap.

use crate::backend::liveness::LivenessResult;
use crate::backend::state::StackSlot;
use crate::common::fx_hash::FxHashMap;
use std::collections::BTreeMap;

#[inline]
fn hull(a: &[(u32, u32)]) -> (u32, u32) {
    let mut s = u32::MAX;
    let mut e = 0u32;
    for &(x, y) in a {
        s = s.min(x);
        e = e.max(y);
    }
    if s == u32::MAX {
        (0, 0)
    } else {
        (s, e)
    }
}

/// Interference test for two slot lifetimes.
///
/// The shipped test compares the CONVEX HULL of each value's segments, not the
/// segments individually. Per-segment ("CFG hole") sharing — letting two
/// values whose segment lists do not overlap share one slot even though their
/// hulls do — was the direct cause of the -O2 preboot-ZSTD miscompile
/// ("ZSTD-compressed data is corrupt", `errcode=20` at
/// `zstd_decompress_block.c:242`): a value live across a hole it did not
/// record had its slot handed to a second value on a different CFG path, so
/// the join block read the wrong bytes. The hull test can only ever make more
/// values interfere, never fewer, so it cannot reintroduce a slot collision.
#[inline]
fn segments_interfere(a: &[(u32, u32)], b: &[(u32, u32)]) -> bool {
    let (as_, ae) = hull(a);
    let (bs, be) = hull(b);
    as_ <= be && bs <= ae
}

fn insert_union(into: &mut Vec<(u32, u32)>, added: &[(u32, u32)]) {
    into.extend_from_slice(added);
    into.sort_unstable();
    let source = std::mem::take(into);
    for (start, end) in source {
        if let Some(last) = into.last_mut() {
            if start <= last.1.saturating_add(1) {
                last.1 = last.1.max(end);
                continue;
            }
        }
        into.push((start, end));
    }
}

/// Color multi-block values into shared stack slots.
///
/// Values are partitioned by exact slot size. Protected values and values
/// missing liveness data receive unique slots. Remaining values are ordered by
/// decreasing segment span/segment count and greedily assigned to the first
/// color whose occupancy union does not interfere.
pub(super) fn color_stack_slots(
    state: &mut crate::backend::state::CodegenState,
    func: &crate::ir::reexports::IrFunction,
    liveness: &LivenessResult,
    multi_block_values: &[(u32, i64)],
    non_local_space: &mut i64,
    assign_slot: &impl Fn(i64, i64, i64) -> (i64, i64),
) {
    let mut segments: FxHashMap<u32, Vec<(u32, u32)>> = FxHashMap::default();
    for segment in &liveness.segments {
        segments
            .entry(segment.value_id)
            .or_default()
            .push((segment.start, segment.end));
    }
    // Liveness can omit a zero-length definition or retain only use-driven
    // pieces. Emission still writes the value's home at every definition,
    // including post-phi multi-def Copies. Add those write points explicitly.
    let mut pp = 0u32;
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                segments.entry(dest.0).or_default().push((pp, pp));
            }
            pp += 1;
        }
        pp += 1; // terminator program point
    }
    for pieces in segments.values_mut() {
        pieces.sort_unstable();
        let source = std::mem::take(pieces);
        insert_union(pieces, &source);
    }

    let mut by_size: BTreeMap<i64, Vec<u32>> = BTreeMap::new();
    for &(value, size) in multi_block_values {
        if state.protected_slot_values.contains(&value) || !segments.contains_key(&value) {
            let (slot, new_space) = assign_slot(*non_local_space, size, 0);
            state.value_locations.insert(value, StackSlot(slot));
            *non_local_space = new_space;
        } else {
            by_size.entry(size).or_default().push(value);
        }
    }

    // Iterate size classes DESCENDING (8/16/32-byte before 4-byte): wider
    // slots claim the low, naturally aligned offsets first, and the trailing
    // 4-byte class fills 4-mod-8 gaps. Ascending order would burn 4 bytes of
    // padding at every 4→8-byte transition.
    for (size, values) in by_size.iter_mut().rev() {
        values.sort_unstable_by(|a, b| {
            let ap = &segments[a];
            let bp = &segments[b];
            let aspan: u64 = ap
                .iter()
                .map(|&(s, e)| e.saturating_sub(s) as u64 + 1)
                .sum();
            let bspan: u64 = bp
                .iter()
                .map(|&(s, e)| e.saturating_sub(s) as u64 + 1)
                .sum();
            bspan
                .cmp(&aspan)
                .then(bp.len().cmp(&ap.len()))
                .then(a.cmp(b))
        });

        let mut colors: Vec<(i64, Vec<(u32, u32)>)> = Vec::new();
        for &value in values.iter() {
            let live = &segments[&value];
            if let Some((slot, occupied)) = colors
                .iter_mut()
                .find(|(_, occupied)| !segments_interfere(live, occupied))
            {
                state.value_locations.insert(value, StackSlot(*slot));
                let (hs, he) = hull(live);
                let (os, oe) = hull(occupied);
                occupied.clear();
                occupied.push((hs.min(os), he.max(oe)));
                continue;
            }

            let (slot, new_space) = assign_slot(*non_local_space, *size, 0);
            *non_local_space = new_space;
            state.value_locations.insert(value, StackSlot(slot));
            colors.push((slot, live.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_boundaries_and_overlapping_hulls_interfere() {
        assert!(segments_interfere(&[(1, 4)], &[(4, 8)]));
        // CFG holes whose convex hulls overlap now interfere: a value live
        // across an unrecorded hole must not share a slot with a neighbour.
        assert!(segments_interfere(&[(1, 3), (9, 11)], &[(4, 8)]));
        assert!(segments_interfere(&[(1, 3), (9, 11)], &[(8, 9)]));
        // Truly disjoint hulls still do not interfere.
        assert!(!segments_interfere(&[(1, 3)], &[(5, 8)]));
    }

    #[test]
    fn union_normalizes_adjacent_pieces() {
        let mut value = vec![(1, 3), (8, 10)];
        insert_union(&mut value, &[(4, 7), (12, 13)]);
        assert_eq!(value, vec![(1, 10), (12, 13)]);
    }
}
