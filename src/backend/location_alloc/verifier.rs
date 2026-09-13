//! GLA post-rewrite structural verification — split from the former monolithic `location_alloc.rs`.
//! See the module docs in [`super`] for the full design and policy record.

use super::*;

// ─────────────────────────────────────────────────────────────────────────────
// Verifier
// ─────────────────────────────────────────────────────────────────────────────

/// Post-rewrite structural verification. Returns an error describing the
/// first violated invariant. The pipeline also runs `CCC_VALIDATE_SSA` after
/// the transform; this checker covers the location-allocation-specific
/// obligations that generic unique-def validation does not see.
pub(crate) fn verify_rewrite(
    after: &IrFunction,
    capture_slots: &FxHashSet<u32>,
) -> Result<(), String> {
    // 1. Unique definition sites.
    let mut defs: FxHashSet<u32> = FxHashSet::default();
    let mut def_sites: FxHashMap<u32, Vec<(usize, usize)>> = FxHashMap::default();
    for (bi, b) in after.blocks.iter().enumerate() {
        for (ii, i) in b.instructions.iter().enumerate() {
            if let Some(d) = i.dest() {
                def_sites.entry(d.0).or_default().push((bi, ii));
                if !defs.insert(d.0) {
                    let sites = def_sites[&d.0]
                        .iter()
                        .map(|(b, p)| format!("b{}[{}]", b, p))
                        .collect::<Vec<_>>()
                        .join(",");
                    return Err(format!("duplicate definition of v{} at {}", d.0, sites));
                }
            }
        }
    }

    // 2. Every load from an internal capture slot is dominated by a store
    // to that slot.
    let label_map = analysis::build_label_map(after);
    let (preds, succs) = analysis::build_cfg(after, &label_map);
    let idom = analysis::compute_dominators(after.blocks.len(), &preds, &succs);
    let dominates = |a_block: usize, b_block: usize| -> bool {
        if a_block == b_block {
            return true;
        }
        let mut cur = b_block;
        for _ in 0..after.blocks.len() + 1 {
            if cur == a_block {
                return true;
            }
            if cur >= idom.len() || idom[cur] == usize::MAX || idom[cur] == cur {
                return false;
            }
            cur = idom[cur];
        }
        false
    };
    let mut slot_stores: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
    for (bi, b) in after.blocks.iter().enumerate() {
        for i in &b.instructions {
            if let Instruction::Store {
                ptr,
                val: Operand::Value(_),
                ..
            } = i
            {
                if capture_slots.contains(&ptr.0) {
                    slot_stores.entry(ptr.0).or_default().push(bi);
                }
            }
        }
    }
    for (bi, b) in after.blocks.iter().enumerate() {
        // Within a block the first capture store must precede every reload
        // from the same slot (block-level dominance is trivial here).
        let mut first_store: FxHashMap<u32, usize> = FxHashMap::default();
        for (ii, i) in b.instructions.iter().enumerate() {
            if let Instruction::Store { ptr, .. } = i {
                if capture_slots.contains(&ptr.0) {
                    first_store.entry(ptr.0).or_insert(ii);
                }
            }
        }
        for (ii, i) in b.instructions.iter().enumerate() {
            if let Instruction::Load { dest, ptr, .. } = i {
                if capture_slots.contains(&ptr.0) {
                    if let Some(&si) = first_store.get(&ptr.0) {
                        if si >= ii {
                            return Err(format!(
                                "reload v{} from slot v{} at block {} point {} precedes its capture store at {}",
                                dest.0, ptr.0, bi, ii, si
                            ));
                        }
                        continue;
                    }
                    let has_store = slot_stores
                        .get(&ptr.0)
                        .is_some_and(|v| v.iter().any(|&sb| dominates(sb, bi)));
                    if !has_store {
                        return Err(format!(
                            "reload v{} from slot v{} in block {} has no dominating store",
                            dest.0, ptr.0, bi
                        ));
                    }
                }
            }
        }
    }

    // 3. Def ids bounded by next_value_id.
    for b in &after.blocks {
        for i in &b.instructions {
            if let Some(d) = i.dest() {
                if d.0 >= after.next_value_id {
                    return Err(format!(
                        "def v{} >= next_value_id {}",
                        d.0, after.next_value_id
                    ));
                }
            }
        }
    }

    // 4. φ prefix invariant.
    for b in &after.blocks {
        let mut seen_non_phi = false;
        for i in &b.instructions {
            if matches!(i, Instruction::Phi { .. }) && seen_non_phi {
                return Err("φ after non-φ instruction".into());
            }
            if !matches!(i, Instruction::Phi { .. }) {
                seen_non_phi = true;
            }
        }
    }

    // 5. Unique block labels; every static terminator edge resolves to a
    // real block. A trampoline that stranded a successor label would
    // manifest here as a dangling branch or a duplicate singleton label.
    let mut block_labels: FxHashSet<u32> = FxHashSet::default();
    for b in &after.blocks {
        if !block_labels.insert(b.label.0) {
            return Err(format!("duplicate block label {}", b.label.0));
        }
        let mut check_target = |t: BlockId| -> Result<(), String> {
            if label_map.get(&t).is_none() {
                return Err(format!(
                    "block {} terminator targets missing block {}",
                    b.label.0, t.0
                ));
            }
            Ok(())
        };
        match &b.terminator {
            Terminator::Branch(t) => check_target(*t)?,
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => {
                check_target(*true_label)?;
                check_target(*false_label)?;
            }
            Terminator::Switch { cases, default, .. } => {
                check_target(*default)?;
                for (_, t) in cases {
                    check_target(*t)?;
                }
            }
            Terminator::IndirectBranch {
                possible_targets, ..
            } => {
                for t in possible_targets {
                    check_target(*t)?;
                }
            }
            Terminator::Return(_) | Terminator::Unreachable => {}
        }
    }

    // 6. Every φ incoming names an actual CFG predecessor of its block,
    // and each (pred, φ) pair is unambiguous per predecessor (phi
    // elimination emits one edge copy per such pair). Trampoline
    // rewiring that relabelled an edge without rerouting the terminator
    // (or vice versa) is rejected here.
    for (bi, b) in after.blocks.iter().enumerate() {
        for i in &b.instructions {
            let Instruction::Phi { incoming, .. } = i else {
                break;
            };
            let mut seen_preds: FxHashSet<u32> = FxHashSet::default();
            for (_, pred) in incoming {
                let Some(&pidx) = label_map.get(pred) else {
                    return Err(format!(
                        "φ in block {} names missing predecessor {}",
                        b.label.0, pred.0
                    ));
                };
                if !preds.row(bi).iter().any(|&p| p == pidx as u32) {
                    return Err(format!(
                        "φ in block {} names {} which is not a CFG predecessor",
                        b.label.0, pred.0
                    ));
                }
                if !seen_preds.insert(pred.0) {
                    return Err(format!(
                        "φ in block {} has two incoming entries for predecessor {}",
                        b.label.0, pred.0
                    ));
                }
            }
        }
    }

    // 7. φ arity vs the CFG: every φ names exactly the block's predecessor
    // SET (compared as sets — build_cfg records both arms of a CondBranch
    // that targets the same block). A trampoline retarget that strands an
    // edge leaves a predecessor with no incoming (phi elimination would
    // feed it undef), which check 6 cannot see.
    for (bi, b) in after.blocks.iter().enumerate() {
        let phis: Vec<_> = b
            .instructions
            .iter()
            .take_while(|i| matches!(i, Instruction::Phi { .. }))
            .collect();
        if phis.is_empty() {
            continue;
        }
        let mut cfg_preds: FxHashSet<u32> = FxHashSet::default();
        for &p in preds.row(bi) {
            let pidx = p as usize;
            if pidx < after.blocks.len() {
                cfg_preds.insert(after.blocks[pidx].label.0);
            }
        }
        for phi in phis {
            let Instruction::Phi { incoming, .. } = phi else {
                unreachable!()
            };
            let named: FxHashSet<u32> = incoming.iter().map(|(_, p)| p.0).collect();
            if named != cfg_preds {
                let missing: Vec<u32> = cfg_preds.difference(&named).copied().collect();
                let extra: Vec<u32> = named.difference(&cfg_preds).copied().collect();
                return Err(format!(
                    "φ in block {} incoming set mismatches CFG preds (missing {missing:?}, extra {extra:?})",
                    b.label.0
                ));
            }
        }
    }

    Ok(())
}
