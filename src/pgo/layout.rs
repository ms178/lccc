//! Profile-guided function and block placement. It preserves source block
//! order while using validated profile data to classify function sections,
//! choose conditional fallthroughs, order switch cases, place devirtualized
//! blocks, and align hot loop headers and join points.
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::{BlockId, IrModule};
use crate::pgo::ProfileData;

pub fn layout_module(m: &mut IrModule, p: &ProfileData, u: &str) {
    // PGO block layout is on by default with an active profile.
    // LCCC_PGO_NO_LAYOUT=1 opts out.
    if std::env::var("LCCC_PGO_NO_LAYOUT").is_ok() {
        return;
    }
    // Fresh per-unit switch-hint map (labels are TU-unique post
    // renumber; layout runs immediately before codegen for this unit).
    crate::pgo::record_switch_hints(crate::common::fx_hash::FxHashMap::default());
    // Fresh per-unit conditional-branch fallthrough map.
    crate::pgo::record_cond_fallthroughs(crate::common::fx_hash::FxHashMap::default());
    // Fresh per-unit block-alignment map (profile-driven loop-header /
    // join-point alignment).
    crate::pgo::record_block_aligns(crate::common::fx_hash::FxHashMap::default());
    let max = p.max_total_for_unit(u);
    if max == 0 {
        return;
    }
    for f in &mut m.functions {
        if f.is_declaration {
            continue;
        }
        // Classify single-block functions too; only intra-function layout
        // requires multiple blocks. Use flow-derived block counts because raw
        // profiles contain instrumented edge counts.
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
        // Hot/cold section classification (whole-function I-cache placement).
        // Section splitting trades I-cache space, so it must be STRICTER than
        // the summary's is_hot/is_cold (which drive inlining decisions): a
        // function is HOT iff it reaches at least 10% of the unit's hottest
        // function's entry count (the dominant functions that deserve the
        // contiguous hot region), and COLD iff it is 1000x below the hottest
        // (truly rare code that belongs in .text.unlikely). A percentile-only
        // threshold mislabels a skewed unit's 1%-execution helpers as hot
        // (the [3x1.2M, 10x40K, 1] shape: the 90% percentile lands at 40K,
        // so every helper cleared `c >= hot_threshold`), polluting .text.hot
        // with cold code — the opposite of the I-cache goal.
        let hot = fp.total_count > 0 && fp.total_count.saturating_mul(10) >= max;
        let cold = max > 0 && fp.total_count.saturating_mul(1000) < max;
        if std::env::var("LCCC_PGO_NO_SECTION").is_err() {
            if hot {
                f.section = Some(".text.hot".into());
            } else if cold {
                f.section = Some(".text.unlikely".into());
            }
        }
        if f.blocks.len() >= 2 {
            layout_function(f, fp, u, edges_valid);
        }
    }

    // Order functions by hotness class within the TU — hot first, cold
    // last — for I-cache-friendly object layout (GCC -freorder-functions).
    // Declarations and helper functions keep their relative order (class 3).
    if crate::pgo::summary::get_summary().is_some() && std::env::var("LCCC_PGO_NO_REORDER").is_err()
    {
        let n = m.functions.len();
        let mut cls: Vec<u8> = vec![3; n];
        for (i, f) in m.functions.iter().enumerate() {
            if f.is_declaration || f.blocks.is_empty() {
                continue;
            }
            let Some(fp) =
                crate::pgo::active_profile_for_function(f).or_else(|| p.get_for_unit(u, &f.name))
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

    // Prefer the more frequently executed successor as the fallthrough when
    // derived edge counts are valid. This avoids block reordering and therefore
    // does not perturb register allocation.
    use crate::ir::reexports::Terminator;
    let mut cf_map: crate::common::fx_hash::FxHashMap<u32, u32> =
        crate::common::fx_hash::FxHashMap::default();
    for f in &m.functions {
        if std::env::var("LCCC_PGO_NO_CFALL").is_ok() {
            break;
        }
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

    // ── Profile-driven block alignment ────────────────────────────────────
    // Hot loop headers and join points get 16-byte alignment. Very hot loop
    // headers with larger bodies get 32-byte alignment. Candidates are gated
    // on per-block execution count, so cold blocks receive no padding.
    if std::env::var("LCCC_PGO_NO_ALIGN").is_err() {
        let mut align_map: crate::common::fx_hash::FxHashMap<u32, u8> =
            crate::common::fx_hash::FxHashMap::default();
        for f in &m.functions {
            if f.is_declaration || f.blocks.len() < 3 {
                continue;
            }
            let Some(fp) = crate::pgo::active_derived_profile(f) else {
                continue;
            };
            let entry_count = fp.total_count.max(1);
            // In-degree per block (number of CFG predecessors + entry).
            let mut indeg: crate::common::fx_hash::FxHashMap<u32, u32> =
                crate::common::fx_hash::FxHashMap::default();
            for b in &f.blocks {
                match &b.terminator {
                    Terminator::Branch(x) => *indeg.entry(x.0).or_insert(0) += 1,
                    Terminator::CondBranch {
                        true_label,
                        false_label,
                        ..
                    } => {
                        *indeg.entry(true_label.0).or_insert(0) += 1;
                        *indeg.entry(false_label.0).or_insert(0) += 1;
                    }
                    Terminator::Switch { cases, default, .. } => {
                        for (_, x) in cases {
                            *indeg.entry(x.0).or_insert(0) += 1;
                        }
                        *indeg.entry(default.0).or_insert(0) += 1;
                    }
                    Terminator::IndirectBranch {
                        possible_targets, ..
                    } => {
                        for x in possible_targets {
                            *indeg.entry(x.0).or_insert(0) += 1;
                        }
                    }
                    _ => {}
                }
            }
            let entry = f.blocks.first().map(|b| b.label.0).unwrap_or(0);
            *indeg.entry(entry).or_insert(0) += 1;
            // Block order index, to detect backedges (edge from a LATER block
            // to an EARLIER one = loop backedge; the earlier block is the
            // loop header).
            let pos: crate::common::fx_hash::FxHashMap<u32, usize> = f
                .blocks
                .iter()
                .enumerate()
                .map(|(i, b)| (b.label.0, i))
                .collect();
            let hot = |label: u32| -> bool {
                // Require at least 1/16 of the function entry count.
                let c = fp.block_count(crate::ir::reexports::BlockId(label));
                c > 0 && c.saturating_mul(16) >= entry_count
            };
            // Treat the target of a backward branch as a loop header.
            let mut is_loop_header: crate::common::fx_hash::FxHashSet<u32> =
                crate::common::fx_hash::FxHashSet::default();
            for p in &f.blocks {
                let pp = *pos.get(&p.label.0).unwrap_or(&usize::MAX);
                let mut mark = |t: u32| {
                    if let Some(&tl) = pos.get(&t) {
                        if pp > tl {
                            is_loop_header.insert(t);
                        }
                    }
                };
                match &p.terminator {
                    Terminator::Branch(x) => mark(x.0),
                    Terminator::CondBranch {
                        true_label,
                        false_label,
                        ..
                    } => {
                        mark(true_label.0);
                        mark(false_label.0);
                    }
                    Terminator::Switch { cases, default, .. } => {
                        for (_, x) in cases {
                            mark(x.0);
                        }
                        mark(default.0);
                    }
                    Terminator::IndirectBranch {
                        possible_targets, ..
                    } => {
                        for x in possible_targets {
                            mark(x.0);
                        }
                    }
                    _ => {}
                }
            }
            for b in &f.blocks {
                let l = b.label.0;
                if !hot(l) {
                    continue;
                }
                if is_loop_header.contains(&l) {
                    // 16-byte loop-header alignment only. The former 32-byte
                    // tier for very hot headers with at least eight
                    // instructions bloated hot functions with multi-byte NOP
                    // padding (gzip: longest_match grew ~11%, 7 padding
                    // sites) and measured SLOWER than 16-byte alignment on
                    // gzip compress — GCC's -falign-loops default is 16 and
                    // LLVM aligns hot loops to 16 too.
                    align_map.insert(l, 4);
                } else if indeg.get(&l).copied().unwrap_or(0) >= 2 {
                    // Hot join point (multiple predecessors): align the merge
                    // so the fall-in is decode-friendly.
                    align_map.insert(l, 4);
                }
            }
        }
        if !align_map.is_empty() {
            crate::pgo::record_block_aligns(align_map);
        }
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
    // Preserve block order because reordering can perturb register allocation
    // in hot loops. Profile-guided changes remain local to fallthrough choices,
    // switch ordering, section placement, promoted blocks, and alignment.
    let mut ordered: Vec<crate::ir::reexports::BasicBlock> = f.blocks.clone();

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

    // Keep promoted hot blocks adjacent to their predecessor. These synthetic
    // blocks have no profile entries and can otherwise fall to the function tail.
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

    // Sort switch cases by edge frequency so compare-and-branch chains test
    // likely cases first. Only derived, non-drifted edge counts are valid.
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
