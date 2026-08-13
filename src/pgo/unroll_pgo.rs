//! Profile-guided loop unrolling.
//!
//! v8: decisions are based on the ESTIMATED TRIP COUNT where the
//! flow-conservation solver has derived edge counts (post-pass):
//! `trip = backedge_count / entry_count` (average iterations per call).
//! Hot high-trip-count loops unroll; cold or low-trip loops do not — no code
//! bloat on cold paths (GCC/LLVM both scale unrolling by profile-derived
//! trip counts). The pre-pass path (no CFG-derived edges yet) falls back to
//! the entry-count heuristic.
use crate::ir::reexports::IrFunction;
use crate::pgo::ProfileData;

fn size_opt() -> bool {
    std::env::var("CFLAGS")
        .map(|x| x.contains("-Os") || x.contains("-Oz"))
        .unwrap_or(false)
}

fn successors(t: &crate::ir::reexports::Terminator) -> Vec<crate::ir::reexports::BlockId> {
    use crate::ir::reexports::Terminator;
    match t {
        Terminator::Branch(x) => vec![*x],
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => vec![*true_label, *false_label],
        Terminator::Switch { cases, default, .. } => cases
            .iter()
            .map(|x| x.1)
            .chain(std::iter::once(*default))
            .collect(),
        Terminator::IndirectBranch {
            possible_targets, ..
        } => possible_targets.clone(),
        _ => vec![],
    }
}

/// Estimated average trip count of the loop whose header is `f.blocks[idx]`.
/// Uses the derived edge counts: trip = backedge / entry.
fn trip_count(f: &IrFunction, idx: usize) -> Option<u64> {
    let header = f.blocks.get(idx)?.label.0;
    let fp = crate::pgo::active_profile_for_function(f)?;
    let entry = fp.total_count.max(1);
    let mut back = 0u64;
    for b in &f.blocks {
        if b.label.0 == header {
            continue;
        }
        for s in successors(&b.terminator) {
            if s.0 == header {
                back = back.max(fp.edge_count(b.label.0, header));
            }
        }
    }
    if back == 0 {
        return None;
    }
    Some(back / entry)
}

pub fn should_unroll_loop(
    f: &IrFunction,
    idx: usize,
    size: usize,
    p: Option<&ProfileData>,
) -> Option<bool> {
    let _ = p?;
    if size_opt() {
        return Some(false);
    }
    if let Some(t) = trip_count(f, idx) {
        if t >= 16 && size <= 24 {
            return Some(true);
        }
        if t >= 8 && size <= 40 {
            return Some(true);
        }
        if t < 3 {
            return Some(false);
        }
        return None;
    }
    // Pre-pass fallback: entry-count heuristic (v7 behavior).
    let n = if crate::pgo::prepass_is_active() {
        crate::pgo::total_count_for(&f.name)
    } else {
        let fp = crate::pgo::active_profile_for_function(f)?;
        let l = f.blocks.get(idx)?.label;
        fp.block_count(l)
    };
    if n > 1000 && size <= 24 {
        Some(true)
    } else if n < 50 {
        Some(false)
    } else {
        None
    }
}
pub fn vectorize_gate(_: &IrFunction, _: Option<&ProfileData>) -> bool {
    true
}
