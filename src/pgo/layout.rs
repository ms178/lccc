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
    let max = p.max_total_for_unit(u);
    if max == 0 {
        return;
    }
    for f in &mut m.functions {
        if f.is_declaration || f.blocks.len() < 2 {
            continue;
        }
        let Some(fp) = p.get_for_unit(u, &f.name) else {
            continue;
        };
        // v8: hot/cold section classification via the data-driven summary
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
        layout_function(f, fp, u);
    }

    // v8: order functions by hotness class within the TU — hot first, cold
    // last — for I-cache-friendly object layout (GCC -freorder-functions).
    // Declarations and helper functions keep their relative order (class 3).
    if crate::pgo::summary::get_summary().is_some() {
        let n = m.functions.len();
        let mut cls: Vec<u8> = vec![3; n];
        for (i, f) in m.functions.iter().enumerate() {
            if f.is_declaration || f.blocks.is_empty() {
                continue;
            }
            let Some(fp) = p.get_for_unit(u, &f.name) else { continue };
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
}

fn layout_function(
    f: &mut crate::ir::reexports::IrFunction,
    fp: &crate::pgo::FunctionProfile,
    u: &str,
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
    // Edge weights: recorded edge counts; synthesized hot edges for promoted
    // blocks (their CFG edges have no profile entries).
    let mut succs = FxHashMap::<BlockId, Vec<BlockId>>::default();
    for b in &f.blocks {
        succs.insert(
            b.label,
            match &b.terminator {
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
            },
        );
    }
    let edge_w = |src: BlockId, dst: BlockId| -> u64 {
        let e = fp
            .edge_count(src.0, dst.0)
            .max(fp.edge_count(crate::pgo::instrument::VENTRY, dst.0));
        if e > 0 {
            return e;
        }
        // Synthesize: promoted block <-> its immediate neighbors are hot.
        if promoted.contains(&src.0) || promoted.contains(&dst.0) {
            return entry_count.max(1);
        }
        0
    };

    // Chain building: repeatedly join the max-weight edge whose endpoints are
    // chain ends. Only edges with positive weight participate; zero-weight
    // blocks stay singleton chains and sink to the cold tail.
    let n = f.blocks.len();
    let labels: Vec<BlockId> = f.blocks.iter().map(|b| b.label).collect();
    let mut idx_of: FxHashMap<BlockId, usize> = FxHashMap::default();
    for (i, l) in labels.iter().enumerate() {
        idx_of.insert(*l, i);
    }
    let mut edges: Vec<(u64, BlockId, BlockId)> = Vec::new();
    for (&s, dsts) in &succs {
        for &d in dsts {
            if s == d {
                continue;
            }
            let w = edge_w(s, d);
            if w > 0 {
                edges.push((w, s, d));
            }
        }
    }
    edges.sort_by(|a, b| b.0.cmp(&a.0).then(a.1 .0.cmp(&b.1 .0)).then(a.2 .0.cmp(&b.2 .0)));

    // chain_of[block] = chain id; chains: Vec<Vec<BlockId>>; head/tail lookup.
    let mut chain_of: Vec<usize> = (0..n).collect();
    let mut chains: Vec<Vec<BlockId>> = labels.iter().map(|l| vec![*l]).collect();
    // head/tail: since we only append at ends, chain[0] is head, last is tail.
    for (w, s, d) in edges {
        let (Some(&si), Some(&di)) = (idx_of.get(&s), idx_of.get(&d)) else {
            continue;
        };
        let ca = chain_of[si];
        let cb = chain_of[di];
        if ca == cb {
            continue;
        }
        let head_a = chains[ca][0];
        let tail_a = *chains[ca].last().unwrap();
        let head_b = chains[cb][0];
        let tail_b = *chains[cb].last().unwrap();
        // Merge forms: (tail_a -> head_b), (tail_a -> tail_b reversed).
        let (mut merged, reversed_b) = if tail_a == s && head_b == d {
            (true, false)
        } else if tail_a == s && tail_b == d {
            (true, true)
        } else {
            (false, false)
        };
        if !merged && head_a == s && head_b == d {
            // prepend: reverse A then append B
            let mut a = std::mem::take(&mut chains[ca]);
            a.reverse();
            a.extend(chains[cb].iter().copied());
            chains[ca] = a;
            merged = true;
        } else if !merged && head_a == s && tail_b == d {
            // reverse both
            let mut a = std::mem::take(&mut chains[ca]);
            a.reverse();
            let mut b = std::mem::take(&mut chains[cb]);
            b.reverse();
            a.extend(b);
            chains[ca] = a;
            merged = true;
        }
        if merged {
            let a = std::mem::take(&mut chains[ca]);
            let b = std::mem::take(&mut chains[cb]);
            let mut out = a;
            if reversed_b {
                out.extend(b.iter().rev().copied());
            } else {
                out.extend(b);
            }
            chains[ca] = out;
            let ca_blocks = chains[ca].clone();
            for &l in &ca_blocks {
                if let Some(&li) = idx_of.get(&l) {
                    chain_of[li] = ca;
                }
            }
        }
        let _ = w;
    }

    // Rotate the entry chain to start at the function entry.
    for chain in chains.iter_mut() {
        if chain.contains(&entry) {
            if let Some(pos) = chain.iter().position(|&l| l == entry) {
                chain.rotate_left(pos);
            }
        }
    }
    // Order chains: entry chain first, then by max count desc, then cold.
    let entry_chain = chain_of[idx_of[&entry]];
    let mut order: Vec<usize> = (0..chains.len()).collect();
    order.sort_by(|&a, &b| {
        let ka = if a == entry_chain { 0 } else { 1 };
        let kb = if b == entry_chain { 0 } else { 1 };
        ka.cmp(&kb).then_with(|| {
            let ma = chains[a].iter().map(|l| counts.get(l).copied().unwrap_or(0)).max().unwrap_or(0);
            let mb = chains[b].iter().map(|l| counts.get(l).copied().unwrap_or(0)).max().unwrap_or(0);
            mb.cmp(&ma)
        })
    });
    let mut ordered: Vec<crate::ir::reexports::BasicBlock> = Vec::with_capacity(n);
    let mut placed = FxHashSet::default();
    for &ci in &order {
        for &l in &chains[ci] {
            if placed.insert(l) {
                if let Some(b) = f.blocks.iter().find(|b| b.label == l) {
                    ordered.push(b.clone());
                }
            }
        }
    }
    // Safety net: any block missed by the chain logic.
    for b in &f.blocks {
        if !placed.contains(&b.label) {
            ordered.push(b.clone());
        }
    }
    // v8: keep promoted (devirtualized) hot blocks adjacent to their
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
    // chains test the most likely cases first.
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
