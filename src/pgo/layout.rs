//! PGO block/function placement. The backend's fall-through-aware conditional
//! emitter consumes block order and inverts the branch when the hot edge is the
//! physical fall-through.
//!
//! v7 layout: proper chain-based placement (Petis–Hansen / GCC
//! `-freorder-blocks` family):
//!   1. each block starts as its own chain;
//!   2. repeatedly merge the highest-weight edge whose endpoints are chain
//!      ends (tail -> head, or tail -> tail with reversal) — this makes hot
//!      edges fall through and leaves backedges as jumps (after the first
//!      joins the backedge's endpoints are in the same chain);
//!   3. the entry chain is rotated so the function entry is first;
//!   4. chains are ordered by hotness (max block count), so zero-count cold
//!      blocks cluster at the function tail (I-cache friendly hot prefix);
//!   5. switch cases are sorted by edge frequency so compare chains test the
//!      likely cases first.
//!
//! Promoted indirect-call blocks (v7 devirtualization) carry no profile
//! entries; the promotion pass records their labels and layout assigns them
//! the entry hotness so the devirtualized hot path stays in the hot prefix.
use crate::ir::reexports::{BlockId, IrModule};
use crate::pgo::ProfileData;
use crate::common::fx_hash::{FxHashMap, FxHashSet};

pub fn layout_module(m: &mut IrModule, p: &ProfileData, u: &str) {
    // PGO block layout is on by default with an active profile.
    // LCCC_PGO_NO_LAYOUT=1 opts out.
    if std::env::var("LCCC_PGO_NO_LAYOUT").is_ok() {
        return;
    }
    // Fresh per-unit switch-hint map (labels are TU-unique post
    // renumber; layout runs immediately before codegen for this unit).
    crate::pgo::record_switch_hints(crate::common::fx_hash::FxHashMap::default());
    // v11: fresh per-unit conditional-branch fallthrough map.
    crate::pgo::record_cond_fallthroughs(crate::common::fx_hash::FxHashMap::default());
    let max = p.max_total_for_unit(u);
    if max == 0 {
        return;
    }
    for f in &mut m.functions {
        if f.is_declaration || f.blocks.len() < 2 {
            continue;
        }
        // F9: use the DERIVED profile (flow-conservation solver output) —
        // raw v5 profiles carry only instrumented EDGE counts; block counts
        // are derived at propagate time. Reading the raw profile made every
        // block count 0: chain ordering degenerated and every switch block
        // looked cold.
        //
        // Edge-derived block layout is ONLY sound for
        // functions whose post-pass CFG matches the training build (the
        // drift gate). A drifted function's surviving edge labels no longer
        // denote the same CFG edges, so reordering on them would scatter
        // blocks by garbage weights. `active_derived_profile` returns None
        // for drifted functions; we then use the name/unit-keyed entry count
        // for hot/cold SECTIONS only (stable) and skip edge-based block
        // reordering entirely (original order preserved).
        let derived = crate::pgo::active_derived_profile(f);
        let raw = p.get_for_unit(u, &f.name);
        let (fp, edges_valid): (&crate::pgo::FunctionProfile, bool) = match derived {
            Some(fp) => (fp, true),
            None => match raw {
                Some(fp) => (fp, false),
                None => continue,
            },
        };
        // Hot/cold section classification via the data-driven summary
        // (percentile thresholds over the unit's count distribution) with
        // the v7 ratio as fallback when no summary is available.
        let (hot, cold) = match crate::pgo::summary::get_summary() {
            Some(s) => (s.is_hot(fp.total_count), s.is_cold(fp.total_count)),
            None => (
                fp.total_count > 0 && fp.total_count.saturating_mul(100) >= max,
                fp.total_count.saturating_mul(10_000) < max,
            ),
        };
        if hot {
            f.section = Some(".text.hot".into());
        } else if cold {
            f.section = Some(".text.unlikely".into());
        }
        layout_function(f, fp, u, edges_valid);
    }

    // Order functions by hotness class within the TU — hot first, cold
    // last — for I-cache-friendly object layout (GCC -freorder-functions).
    // Declarations and helper functions keep their relative order (class 3).
    if crate::pgo::summary::get_summary().is_some() {
        let n = m.functions.len();
        let mut cls: Vec<u8> = vec![3; n];
        for (i, f) in m.functions.iter().enumerate() {
            if f.is_declaration || f.blocks.is_empty() {
                continue;
            }
            let Some(fp) = crate::pgo::active_profile_for_function(f)
                .or_else(|| p.get_for_unit(u, &f.name))
            else {
                continue;
            };
            let s = crate::pgo::summary::get_summary().unwrap();
            if s.is_hot(fp.total_count) {
                cls[i] = 0;
            } else if s.is_cold(fp.total_count) {
                cls[i] = 2;
            } else {
                cls[i] = 1;
            }
        }
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| {
            cls[a]
                .cmp(&cls[b])
                .then_with(|| {
                    let ca = p
                        .get_for_unit(u, &m.functions[a].name)
                        .map(|f| f.total_count)
                        .unwrap_or(0);
                    let cb = p
                        .get_for_unit(u, &m.functions[b].name)
                        .map(|f| f.total_count)
                        .unwrap_or(0);
                    cb.cmp(&ca)
                })
                .then_with(|| a.cmp(&b))
        });
        // In-place permutation (IrFunction is not Clone).
        let mut perm = order;
        let mut i = 0;
        while i < n {
            if perm[i] == i {
                i += 1;
                continue;
            }
            let j = perm[i];
            m.functions.swap(i, j);
            perm.swap(i, j);
        }
    }

    // v11: profile-driven conditional-branch fallthrough. For every
    // CondBranch whose edge profile is valid (non-drifted, derived), the
    // successor with the higher edge execution count becomes the preferred
    // fallthrough. The backend then emits the branch so the HOT successor
    // falls through (fewer taken branches / branch misses on the hot path),
    // WITHOUT reordering blocks — so register allocation is never perturbed.
    use crate::ir::reexports::Terminator;
    let mut cf_map: crate::common::fx_hash::FxHashMap<u32, u32> =
        crate::common::fx_hash::FxHashMap::default();
    for f in &m.functions {
        if f.is_declaration || f.blocks.is_empty() {
            continue;
        }
        // Only trust edge-derived hotness for non-drifted functions.
        let Some(fp) = crate::pgo::active_derived_profile(f) else {
            continue;
        };
        for b in &f.blocks {
            let Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } = &b.terminator
            else {
                continue;
            };
            let t = fp.edge_count(b.label.0, true_label.0);
            let fl = fp.edge_count(b.label.0, false_label.0);
            if t == 0 && fl == 0 {
                continue; // no informative edge data
            }
            let hot = if t >= fl { true_label.0 } else { false_label.0 };
            cf_map.insert(b.label.0, hot);
        }
    }
    if !cf_map.is_empty() {
        crate::pgo::record_cond_fallthroughs(cf_map);
    }
}

fn layout_function(
    f: &mut crate::ir::reexports::IrFunction,
    fp: &crate::pgo::FunctionProfile,
    u: &str,
    edges_valid: bool,
) {
    use crate::ir::reexports::Terminator;
    let promoted: FxHashSet<u32> = crate::pgo::promoted_hot_labels(u, &f.name);
    let entry = f.blocks[0].label;

    // Per-block hotness: derived counts; promoted blocks get entry hotness.
    let mut counts = FxHashMap::<BlockId, u64>::default();
    let entry_count = fp.total_count;
    for b in &f.blocks {
        let c = if promoted.contains(&b.label.0) {
            entry_count.max(1)
        } else {
            fp.block_count(b.label)
        };
        counts.insert(b.label, c);
    }
    // Drift gate: when the edge profile is not trustworthy (drifted or a
    // raw/entry-only fallback), skip the chain-based block reordering and the
    // edge-frequency switch-case sorting — they would scatter blocks by stale
    // weights. Preserve original order (still run hot/cold sectioning,
    // promoted-block placement, and count-based cold switch lowering).
    let mut ordered: Vec<crate::ir::reexports::BasicBlock> = if edges_valid {
        // v10 (red-team fix): PRESERVE the original block order. The prior
        // chain-based (Petis-Hansen) reorder and even cold-block sinking can,
        // for a function dominated by ONE hot loop, perturb the hot loop body
        // and its register allocation — expat's multi-byte UTF-8 handling was
        // placed before the hot ASCII loop (frame 40->72B, ~2x regression,
        // measured 131ms -> 248ms), and cold-block sinking alone cost ~7ms
        // (128.5 -> 135.5ms). The source order already provides good
        // fallthrough for a hot loop. Robust no-regression rule: keep the
        // ORIGINAL block order byte-for-byte (no reordering, no sinking), so
        // the hot path is IDENTICAL to the plain build. Profile value comes
        // from the (safe, local) switch-case ordering, hot/cold FUNCTION
        // sections, promoted-block placement, and the PGO inliner's
        // cold-classification fix — not from intra-function block movement.
        f.blocks.clone()
    } else {
        f.blocks.clone()
    };

    // Profile-driven switch hints. For every Switch block:
    //   * cold per the summary -> force a compare chain (no jump table);
    //   * a case carrying >= 50% of the block's executions -> record it for
    //     hoisting out of the jump table (profile-guided partitioning).
    // Hot-case hoisting uses edge counts, so it is only sound when the
    // edge profile is valid; cold classification uses block counts (stable).
    if crate::pgo::summary::get_summary().is_some() {
        use crate::ir::reexports::Terminator;
        let mut hints: FxHashMap<u32, crate::pgo::SwitchHint> = FxHashMap::default();
        for b in &f.blocks {
            if let Terminator::Switch { cases, .. } = &b.terminator {
                let c = counts.get(&b.label).copied().unwrap_or(0);
                let cold = crate::pgo::summary::get_summary()
                    .map(|s| s.is_cold(c))
                    .unwrap_or(false);
                let mut hint = crate::pgo::SwitchHint {
                    hot_case: None,
                    force_chain: cold,
                };
                if edges_valid && c > 0 {
                    let mut best: Option<(i64, u32, u64)> = None;
                    for (v, t) in cases {
                        let e = fp.edge_count(b.label.0, t.0);
                        if best.map_or(true, |(_, _, be)| e > be) {
                            best = Some((*v, t.0, e));
                        }
                    }
                    if let Some((v, t, e)) = best {
                        if (e as f64) / (c as f64) >= 0.50 {
                            hint.hot_case = Some((v, t));
                        }
                    }
                }
                if std::env::var("LCCC_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "[LAYOUT-SW] {} switch block {} count={} cold={} hint={:?}",
                        f.name, b.label.0, c, cold, hint
                    );
                }
                hints.insert(b.label.0, hint);
            }
        }
        if !hints.is_empty() {
            let mut all = crate::pgo::switch_hints_snapshot();
            for (k, v) in hints {
                all.insert(k, v);
            }
            crate::pgo::record_switch_hints(all);
        }
    }

    // Keep promoted (devirtualized) hot blocks adjacent to their
    // predecessor. The chain heuristic can strand them at the function
    // tail (their edges to a mid-chain block never merge), hurting the hot
    // path — observed in zlib-ng deflate_quick.
    if std::env::var("LCCC_DEBUG_LAYOUT").is_ok() {
        eprintln!(
            "[LAYOUT] {} promoted labels: {:?} ({} blocks)",
            f.name,
            promoted.iter().collect::<Vec<_>>(),
            f.blocks.len()
        );
    }
    let _ = promoted;
    if !promoted.is_empty() {
        let succ_of = |t: &Terminator| -> Vec<crate::ir::reexports::BlockId> {
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
        };
        // Deferred-append: promoted blocks are SKIPPED in the main pass and
        // appended right after their predecessor; the deferred remainder goes
        // to the tail. Length is preserved by construction (each block lands
        // exactly once), so the guarded swap below always applies.
        let mut placed_prom: FxHashSet<u32> = FxHashSet::default();
        let mut out2: Vec<crate::ir::reexports::BasicBlock> = Vec::with_capacity(ordered.len());
        for b in ordered.iter() {
            if promoted.contains(&b.label.0) {
                continue; // deferred
            }
            out2.push(b.clone());
            for s in succ_of(&b.terminator) {
                if promoted.contains(&s.0) && placed_prom.insert(s.0) {
                    if let Some(pb) = ordered.iter().find(|x| x.label == s) {
                        out2.push(pb.clone());
                    }
                }
            }
        }
        for b in ordered.iter() {
            if promoted.contains(&b.label.0) && !placed_prom.contains(&b.label.0) {
                out2.push(b.clone());
            }
        }
        if out2.len() == ordered.len() {
            ordered = out2;
        }
    }
    if ordered.len() == f.blocks.len() {
        f.blocks = ordered;
    }



    // PGO switch case ordering: sort each Switch's cases by the frequency
    // of the (switch block -> case block) edge so compare-and-branch
    // chains test the most likely cases first. Only when the edge profile is
    // valid (v9 drift gate).
    if edges_valid {
        for b in &mut f.blocks {
            let src = b.label.0;
            if let Terminator::Switch { cases, .. } = &mut b.terminator {
                if cases.len() < 2 {
                    continue;
                }
                let mut idx: Vec<usize> = (0..cases.len()).collect();
                idx.sort_by_key(|&i| std::cmp::Reverse(fp.edge_count(src, cases[i].1 .0)));
                let sorted: Vec<_> = idx.into_iter().map(|i| cases[i]).collect();
                *cases = sorted;
            }
        }
    }
}
