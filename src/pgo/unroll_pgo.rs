//! Profile-guided loop unrolling.
//!
//! Decisions are based on the ESTIMATED TRIP COUNT where the
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

/// Estimated average trip count of the loop whose header is `f.blocks[idx]`,
/// derived from the profile via flow conservation: trip = backedge / entry.
///
/// The unroller runs DURING the pre-pass (inside `run_passes`), at which point
/// `active_derived_profile` is not yet published (`propagate_profile` runs
/// after the pass pipeline). The raw profile only carries instrumented
/// NON-tree edges; a loop's backedge is the maximum-spanning-tree edge (weight
/// 2000) and is therefore NOT instrumented, so `edge_count(backedge)` is 0 on
/// the raw profile — the "derived trip count" path was dead during
/// unrolling and always fell back to the entry-count heuristic. Fix: derive
/// the tree/backedge counts on the current (pre-pass) CFG right here, exactly
/// as the PGO inliner already does for per-call-site hotness.
fn trip_count(f: &IrFunction, idx: usize) -> Option<u64> {
    let header = f.blocks.get(idx)?.label.0;
    // Pre-pass: clone the raw (name-keyed) profile and derive the tree edges
    // on the CURRENT pre-pass CFG. The pre-pass CFG is stable across
    // generate/use, so the derived backedge count is correct.
    let mut fp = crate::pgo::prepass_profile(&f.name)?;
    crate::pgo::profile::derive_block_counts(f, &mut fp);
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
    if std::env::var("LCCC_PGO_NO_UNROLL").is_ok() {
        return None; // let the size/feature-driven unroller decide on its own
    }
    let _ = p?;
    if size_opt() {
        return Some(false);
    }
    if let Some(t) = trip_count(f, idx) {
        // Profile-accurate unrolling (GCC/LLVM scale unrolling by the
        // estimated trip count): high-trip loops unroll, low-trip loops do
        // not — no code bloat on cold/once-only paths.
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
    // Pre-pass fallback: entry-count heuristic for functions
    // without a usable derived backedge (e.g. no profile for this function).
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
/// Exact per-loop PGO profitability veto. Absence of profile data leaves the
/// vectorizer's static model unchanged; profile data may veto but never force
/// an otherwise illegal transform.
pub fn should_vectorize_loop(f: &IrFunction, header_idx: usize, body_insts: usize) -> Option<bool> {
    if std::env::var("LCCC_PGO_NO_VECTOR_GATE").is_ok() || !crate::pgo::prepass_is_active() {
        return None;
    }
    Some(vectorize_profitable(trip_count(f, header_idx)?, body_insts))
}

#[inline]
fn vectorize_profitable(trip: u64, body_insts: usize) -> bool {
    trip >= 8 && !(body_insts > 80 && trip < 32)
}

#[cfg(test)]
mod vector_gate_tests {
    use super::vectorize_profitable;
    #[test] fn short_loops_are_rejected() {
        assert!(!vectorize_profitable(7, 8)); assert!(vectorize_profitable(8, 8));
    }
    #[test] fn large_loops_need_amortization() {
        assert!(!vectorize_profitable(16, 81)); assert!(vectorize_profitable(32, 81));
    }
}
