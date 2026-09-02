//! Stack-slot coloring for multi-block SSA values.
//!
//! Register allocation and stack allocation have different boundary rules.
//! A register can sometimes be handed from a value used at point P to a value
//! defined at P because the emitter reads before it writes. A stack slot is
//! more conservative: hidden accumulator stores/reloads can occur around that
//! IR point, so two slot lifetimes touching at P are considered interfering.
//! This closed-boundary rule avoids the historical SQLite collision while
//! still recovering the win that matters: values whose live ranges are
//! genuinely disjoint share one slot.
//!
//! Two interference regimes are implemented:
//!
//! * **Hull (fat-interval) — the default and the sound model.** Each value's
//!   segments are collapsed to their convex hull `[min start, max end]`, and
//!   two values interfere exactly when their hulls overlap (closed
//!   boundaries). This is the same model the linear-scan register allocator
//!   uses (`LivenessResult::intervals`), so a shared slot is only ever given
//!   to values whose complete live ranges are disjoint. It is immune to a
//!   liveness hole being under-recorded: a value live across a CFG hole it
//!   failed to record still owns the whole span in its hull, and a neighbour
//!   inside that span cannot be granted the slot.
//!
//! * **Per-segment (CFG-hole aware) — opt in with `CCC_TIER2_SEGMENTS=1` for
//!   A/B measurement only.** Mutually exclusive CFG arms (`if (c) {v1} else
//!   {v2}`) have disjoint live *segments* even when their hulls overlap, so
//!   per-segment coloring packs more values into one frame region. This is
//!   the maximum sharing but is only sound when the liveness model records
//!   *every* hole exactly (including hidden emitter slot touches), so it is
//!   never the default. The 2026-09-02 preboot-ZSTD corruption was
//!   originally misattributed to this regime; the true cause was a slot-width
//!   mismatch (a 4-byte slot accessed by `movq`), fixed at the classifier.

use crate::backend::liveness::LivenessResult;
use crate::backend::state::StackSlot;
use crate::common::fx_hash::FxHashMap;
use std::collections::BTreeMap;

/// Convex hull `[min start, max end]` of a segment list. An empty list
/// degenerates to a zero-length interval at program point 0 (never matches a
/// real occupant, which always has `start >= 0`).
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

/// Closed-boundary interference over two CONVEX HULLS.
///
/// "Closed" means two lifetimes touching at exactly one program point
/// interfere: a stack slot may be written by a hidden accumulator store at
/// the very point the previous occupant was last read, so sharing a slot at a
/// touching boundary is unsafe even though an interval-graph register
/// allocator would allow the hand-off at a point.
#[inline]
fn hulls_interfere(a: &[(u32, u32)], b: &[(u32, u32)]) -> bool {
    let (as_, ae) = hull(a);
    let (bs, be) = hull(b);
    as_ <= be && bs <= ae
}

/// Closed-boundary interference over two PER-SEGMENT lists.
///
/// Two values interfere iff any segment of one touches any segment of the
/// other. Holes (gaps between a value's segments) do not themselves cause
/// interference — that is the property that makes this regime pack more, and
/// the property that makes it require exact liveness.
#[inline]
fn segments_interfere(a: &[(u32, u32)], b: &[(u32, u32)]) -> bool {
    let (mut ai, mut bi) = (0usize, 0usize);
    while ai < a.len() && bi < b.len() {
        let (as_, ae) = a[ai];
        let (bs, be) = b[bi];
        // Closed boundary: touching points interfere.
        if as_ <= be && bs <= ae {
            return true;
        }
        if ae < bs {
            ai += 1;
        } else {
            bi += 1;
        }
    }
    false
}

/// Merge `added` into `into` as a sorted, normalized per-segment union.
/// Adjacent or overlapping pieces are fused (a gap of one program point is
/// *not* fused — pieces that merely touch stay separate only when they are
/// disjoint; closed-boundary interference still treats the touching point as
/// shared, so fusing at `+1` is the conservative choice that keeps a later
/// hull collapse identical).
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
/// decreasing total live span and greedily assigned to the first color whose
/// occupancy does not interfere under the selected regime (hull by default,
/// per-segment under `CCC_TIER2_SEGMENTS=1`). Each color's occupancy is
/// maintained under the same regime: hull colors collapse to the merged hull,
/// per-segment colors keep the normalized segment union.
pub(super) fn color_stack_slots(
    state: &mut crate::backend::state::CodegenState,
    func: &crate::ir::reexports::IrFunction,
    liveness: &LivenessResult,
    multi_block_values: &[(u32, i64)],
    non_local_space: &mut i64,
    assign_slot: &impl Fn(i64, i64, i64) -> (i64, i64),
) {
    let per_segment = std::env::var_os("CCC_TIER2_SEGMENTS").is_some();
    if std::env::var("CCC_DEBUG_SLOTS").is_ok() {
        eprintln!(
            "[SLOTS]   color_stack_slots: {} mbv (segment model: {})",
            multi_block_values.len(),
            if per_segment { "per-segment" } else { "hull" }
        );
    }

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
    let mut no_segments: Vec<u32> = Vec::new();
    for &(value, size) in multi_block_values {
        if state.protected_slot_values.contains(&value) || !segments.contains_key(&value) {
            let (slot, new_space) = assign_slot(*non_local_space, size, 0);
            state.value_locations.insert(value, StackSlot(slot));
            *non_local_space = new_space;
            if !segments.contains_key(&value) && !state.protected_slot_values.contains(&value) {
                no_segments.push(value);
            }
        } else {
            by_size.entry(size).or_default().push(value);
        }
    }
    if std::env::var("CCC_DEBUG_SLOTS").is_ok() && !no_segments.is_empty() {
        eprintln!(
            "[SLOTS]   color_stack_slots: {} mbv have NO liveness segments \
             (conservative unique slot): {:?}",
            no_segments.len(),
            no_segments
        );
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
    let mut reused = 0usize;
    let mut assigned = 0usize;
    for &value in values.iter() {
        let live = &segments[&value];
        let interferes = |occupied: &[(u32, u32)]| {
            if per_segment {
                segments_interfere(live, occupied)
            } else {
                hulls_interfere(live, occupied)
            }
        };
        if let Some((slot, occupied)) = colors.iter_mut().find(|(_, occupied)| !interferes(occupied))
        {
            state.value_locations.insert(value, StackSlot(*slot));
            reused += 1;
            if per_segment {
                // Preserve the per-segment structure of the union so later
                // CFG-arm candidates can still use the holes inside it.
                let live_clone = live.clone();
                insert_union(occupied, &live_clone);
            } else {
                // Hull model: the colour's occupancy is its merged hull —
                // the only information the hull test can ever consult.
                let (hs, he) = hull(live);
                let (os, oe) = hull(occupied);
                occupied.clear();
                occupied.push((hs.min(os), he.max(oe)));
            }
            continue;
        }

        let (slot, new_space) = assign_slot(*non_local_space, *size, 0);
        *non_local_space = new_space;
        state.value_locations.insert(value, StackSlot(slot));
        colors.push((slot, live.clone()));
        assigned += 1;
    }
    if std::env::var("CCC_DEBUG_SLOTS").is_ok() {
        eprintln!(
            "[SLOTS]   color_stack_slots: {} distinct slots for {} values (reused {}; {} new)",
            colors.len(),
            multi_block_values.len(),
            reused,
            assigned
        );
    }
}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_boundaries_interfere_under_hull() {
        // Touching at a point interferes (closed boundary).
        assert!(hulls_interfere(&[(1, 4)], &[(4, 8)]));
        // Overlapping hulls interfere even when the segment lists would not.
        assert!(hulls_interfere(&[(1, 3), (9, 11)], &[(4, 8)]));
        // Truly disjoint hulls do not interfere.
        assert!(!hulls_interfere(&[(1, 3)], &[(5, 8)]));
    }

    #[test]
    fn per_segment_model_is_looser_than_hull() {
        // CFG-arm exclusivity: the two segment lists never overlap in time,
        // but their hulls do. The per-segment model (opt-in) allows sharing;
        // the hull model (default) refuses it.
        assert!(!segments_interfere(&[(1, 3), (9, 11)], &[(4, 8)]));
        assert!(hulls_interfere(&[(1, 3), (9, 11)], &[(4, 8)]));
        // Closed boundary still applies per-segment.
        assert!(segments_interfere(&[(1, 3), (9, 11)], &[(8, 9)]));
    }

    #[test]
    fn union_normalizes_adjacent_pieces() {
        let mut value = vec![(1, 3), (8, 10)];
        insert_union(&mut value, &[(4, 7), (12, 13)]);
        assert_eq!(value, vec![(1, 10), (12, 13)]);
    }

    /// True closed-boundary interference of two segment lists, computed by
    /// the definition (pairwise point/touch comparison) — the ground truth
    /// the optimized two-pointer walk must agree with.
    fn naive_interfere(a: &[(u32, u32)], b: &[(u32, u32)]) -> bool {
        for &(as_, ae) in a {
            for &(bs, be) in b {
                if as_ <= be && bs <= ae {
                    return true;
                }
            }
        }
        false
    }

    #[test]
    fn two_pointer_walk_matches_brute_force() {
        let cases: &[&[(u32, u32)]] = &[
            &[],
            &[(0, 0)],
            &[(1, 4)],
            &[(1, 3), (9, 11)],
            &[(1, 10), (20, 30), (40, 50)],
            &[(5, 6), (7, 8), (9, 10)],
            &[(0, 100)],
        ];
        for a in cases {
            for b in cases {
                assert_eq!(
                    segments_interfere(a, b),
                    naive_interfere(a, b),
                    "segments_interfere({a:?},{b:?}) disagrees with brute force"
                );
            }
        }
    }

    /// Exhaustive micro-random cross-check of `segments_interfere` against
    /// the naive definition over short segment lists.
    #[test]
    fn segments_interfere_randomized_against_brute_force() {
        // Deterministic PRNG (SplitMix64) so failures reproduce exactly.
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = seed;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        let rand_seg = |n: &mut dyn FnMut() -> u64, max_end: u32| {
            let s = (n() % (max_end as u64 + 1)) as u32;
            let e = s + (n() % 4) as u32;
            (s, e.min(max_end))
        };
        for _ in 0..20000 {
            let n = (next() % 4) as usize;
            let m = (next() % 4) as usize;
            let max_end = 16;
            let mut a: Vec<(u32, u32)> = (0..n).map(|_| rand_seg(&mut next, max_end)).collect();
            let mut b: Vec<(u32, u32)> = (0..m).map(|_| rand_seg(&mut next, max_end)).collect();
            a.sort_unstable();
            b.sort_unstable();
            assert_eq!(
                segments_interfere(&a, &b),
                naive_interfere(&a, &b),
                "randomized segments_interfere({a:?},{b:?}) disagrees"
            );
        }
    }

    /// Greedy coloring soundness: after `color_stack_slots` places a set of
    /// hull-typed values, no two values on the SAME slot may have overlapping
    /// hulls. This is the invariant whose violation corrupted the preboot
    /// decompressor. We exercise the greedy loop directly (the real
    /// `color_stack_slots` needs full CodegenState, so this tests the shared
    /// decision logic: find-first-non-interfering colour + hull merge).
    #[test]
    fn greedy_hull_coloring_never_overlaps_a_slot() {
        let mut seed: u64 = 0xD1B5_4A32_D192_ED03;
        let mut next = move || {
            seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = seed;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        for _trial in 0..2000 {
            // Random values: each is 1..3 segments inside [0, 64).
            let nvals = 1 + (next() % 12) as usize;
            let mut segs: Vec<Vec<(u32, u32)>> = Vec::new();
            for _ in 0..nvals {
                let k = 1 + (next() % 3) as usize;
                let mut v: Vec<(u32, u32)> = (0..k)
                    .map(|_| {
                        let s = (next() % 64) as u32;
                        (s, s + (next() % 8) as u32)
                    })
                    .collect();
                v.sort_unstable();
                let source = std::mem::take(&mut v);
                insert_union(&mut v, &source);
                segs.push(v);
            }
            // Greedy hull coloring (default regime).
            let mut colors: Vec<Vec<(u32, u32)>> = Vec::new();
            let mut slot_of: Vec<usize> = vec![0; nvals];
            for (i, live) in segs.iter().enumerate() {
                let c = colors
                    .iter()
                    .position(|occ| !hulls_interfere(live, occ));
                let ci = match c {
                    Some(ci) => ci,
                    None => {
                        colors.push(live.clone());
                        colors.len() - 1
                    }
                };
                // Merge member into its colour under the hull regime.
                let occ = &mut colors[ci];
                let (hs, he) = hull(live);
                let (os, oe) = hull(occ);
                occ.clear();
                occ.push((hs.min(os), he.max(oe)));
                slot_of[i] = ci;
            }
            // Post-condition: any two values sharing a colour have disjoint hulls.
            for i in 0..nvals {
                for j in (i + 1)..nvals {
                    if slot_of[i] == slot_of[j] {
                        assert!(
                            !hulls_interfere(&segs[i], &segs[j]),
                            "trial {_trial}: v{i}{:?} and v{j}{:?} share a slot \
                             but their hulls overlap",
                            segs[i],
                            segs[j]
                        );
                    }
                }
            }
        }
    }

    /// The per-segment (opt-in) regime must keep per-segment disjointness on
    /// every colour — i.e. it only ever shares across true CFG holes, never
    /// across a segment overlap.
    #[test]
    fn greedy_segment_coloring_never_overlaps_a_segment() {
        let mut seed: u64 = 0xC0FF_EE00_C0FF_EE00;
        let mut next = move || {
            seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = seed;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        for _trial in 0..2000 {
            let nvals = 1 + (next() % 10) as usize;
            let mut segs: Vec<Vec<(u32, u32)>> = Vec::new();
            for _ in 0..nvals {
                let k = 1 + (next() % 3) as usize;
                let mut v: Vec<(u32, u32)> = (0..k)
                    .map(|_| {
                        let s = (next() % 48) as u32;
                        (s, s + (next() % 6) as u32)
                    })
                    .collect();
                v.sort_unstable();
                let source = std::mem::take(&mut v);
                insert_union(&mut v, &source);
                segs.push(v);
            }
            let mut colors: Vec<Vec<(u32, u32)>> = Vec::new();
            let mut slot_of: Vec<usize> = vec![0; nvals];
            for (i, live) in segs.iter().enumerate() {
                let c = colors.iter().position(|occ| !segments_interfere(live, occ));
                let ci = match c {
                    Some(ci) => ci,
                    None => {
                        colors.push(live.clone());
                        colors.len() - 1
                    }
                };
                let live_clone = live.clone();
                insert_union(&mut colors[ci], &live_clone);
                slot_of[i] = ci;
            }
            for i in 0..nvals {
                for j in (i + 1)..nvals {
                    if slot_of[i] == slot_of[j] {
                        assert!(
                            !segments_interfere(&segs[i], &segs[j]),
                            "trial {_trial}: per-segment colour violation v{i}{:?}, v{j}{:?}",
                            segs[i],
                            segs[j]
                        );
                    }
                }
            }
        }
    }
}
