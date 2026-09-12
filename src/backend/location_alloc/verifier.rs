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
    Ok(())
}
