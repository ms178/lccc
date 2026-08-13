//! PGO block/function placement. The backend's fall-through-aware conditional
//! emitter consumes block order and inverts the branch when the hot edge is the
//! physical fall-through.
use crate::ir::reexports::{BlockId, IrModule};
use crate::pgo::ProfileData;
use crate::common::fx_hash::{FxHashMap, FxHashSet};

pub fn layout_module(m: &mut IrModule, p: &ProfileData, u: &str) {
    // PGO v4: block layout is on by default with an active profile.
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
        if fp.total_count.saturating_mul(100) >= max && fp.total_count > 0 {
            f.section = Some(".text.hot".into());
        } else if fp.total_count.saturating_mul(10_000) < max {
            f.section = Some(".text.unlikely".into());
        }
        let mut counts = FxHashMap::<BlockId, u64>::default();
        for b in &f.blocks {
            counts.insert(b.label, fp.block_count(b.label));
        }
        let edge_w = |src: BlockId, dst: BlockId| -> u64 {
            fp.edge_count(src.0, dst.0)
                .max(fp.edge_count(crate::pgo::instrument::VENTRY, dst.0))
        };
        let mut succ = FxHashMap::<BlockId, Vec<BlockId>>::default();
        for b in &f.blocks {
            succ.insert(b.label, successors(&b.terminator));
        }
        let entry = f.blocks[0].label;
        let mut ordered = Vec::with_capacity(f.blocks.len());
        let mut placed = FxHashSet::default();
        let mut cur = entry;
        loop {
            if !placed.insert(cur) {
                break;
            }
            let Some(block) = f.blocks.iter().find(|b| b.label == cur) else {
                break;
            };
            ordered.push(block.clone());
            let next = succ
                .get(&cur)
                .into_iter()
                .flatten()
                .filter(|x| !placed.contains(x))
                .max_by_key(|&&x| {
                    let e = edge_w(cur, x);
                    if e > 0 {
                        e
                    } else {
                        counts.get(&x).copied().unwrap_or(0)
                    }
                })
                .copied();
            let Some(n) = next else {
                break;
            };
            cur = n;
        }
        let mut rest: Vec<_> = f
            .blocks
            .iter()
            .filter(|b| !placed.contains(&b.label))
            .cloned()
            .collect();
        rest.sort_by_key(|b| std::cmp::Reverse(counts.get(&b.label).copied().unwrap_or(0)));
        ordered.extend(rest);
        if ordered.len() == f.blocks.len() {
            f.blocks = ordered;
        }

        // PGO switch case ordering: sort each Switch's cases by the frequency
        // of the (switch block -> case block) edge so compare-and-branch
        // chains test the most likely cases first.
        for b in &mut f.blocks {
            let src = b.label.0;
            if let crate::ir::reexports::Terminator::Switch { cases, .. } = &mut b.terminator {
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
fn successors(t: &crate::ir::reexports::Terminator) -> Vec<BlockId> {
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
