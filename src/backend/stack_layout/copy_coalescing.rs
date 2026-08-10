//! Copy coalescing and immediately-consumed value analysis.
//!
//! Copy coalescing identifies Copy instructions where the destination can
//! share the source's stack slot (eliminating a separate allocation).
//! The immediately-consumed analysis identifies values that are produced
//! and consumed in adjacent instructions, allowing them to skip stack
//! slot allocation entirely by staying in the accumulator register cache.

use crate::ir::reexports::{
    Instruction,
    IrConst,
    IrFunction,
    Operand,
    Terminator,
};
use crate::common::types::IrType;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::backend::regalloc::PhysReg;
use crate::backend::liveness::{
    for_each_operand_in_instruction, for_each_value_use_in_instruction,
    for_each_operand_in_terminator,
};

/// Return true unless the CFG-aware coalescer is explicitly disabled for
/// bisection.  v4 made this default after project gates, source regressions,
/// and two differential fuzz families established that the graph proof is
/// materially better than the v3 textual interval approximation.
fn cfg_copy_coalesce_enabled() -> bool {
    std::env::var("CCC_NO_CFG_COPY_COALESCE").is_err()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CoalesceClass {
    /// Eight-byte scalar stack representation: integer or pointer values only.
    Scalar,
}

fn scalar_type(ty: IrType) -> bool {
    matches!(
        ty,
        IrType::I8
            | IrType::U8
            | IrType::I16
            | IrType::U16
            | IrType::I32
            | IrType::U32
            | IrType::I64
            | IrType::U64
            | IrType::Ptr
    )
}

fn scalar_const(c: IrConst) -> bool {
    matches!(
        c,
        IrConst::I8(_)
            | IrConst::I16(_)
            | IrConst::I32(_)
            | IrConst::I64(_)
            | IrConst::Zero
    )
}

/// Classify values that have the same scalar stack representation in all
/// relevant paths.  Floats, vectors, i128/F128, allocas, and opaque results
/// are intentionally excluded: a missed coalescing opportunity is harmless,
/// whereas widening a slot-sharing rule beyond codegen's exact ABI paths is not.
fn collect_scalar_values(func: &IrFunction) -> FxHashMap<u32, CoalesceClass> {
    let mut classes: FxHashMap<u32, CoalesceClass> = FxHashMap::default();
    let mut allocas: FxHashSet<u32> = FxHashSet::default();

    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca { dest, .. } = inst {
                allocas.insert(dest.0);
            }
            let class = match inst {
                Instruction::BinOp { dest, ty, .. }
                | Instruction::UnaryOp { dest, ty, .. }
                | Instruction::Cast { dest, to_ty: ty, .. }
                | Instruction::Load { dest, ty, .. }
                | Instruction::Select { dest, ty, .. }
                | Instruction::AtomicLoad { dest, ty, .. }
                | Instruction::AtomicRmw { dest, ty, .. }
                | Instruction::AtomicCmpxchg { dest, ty, .. }
                | Instruction::ParamRef { dest, ty, .. } if scalar_type(*ty) => Some(dest.0),
                Instruction::Cmp { dest, .. }
                | Instruction::GetElementPtr { dest, .. }
                | Instruction::GlobalAddr { dest, .. }
                | Instruction::LabelAddr { dest, .. } => Some(dest.0),
                Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. }
                    if info.dest.is_some() && scalar_type(info.return_type) => info.dest.map(|v| v.0),
                _ => None,
            };
            if let Some(id) = class {
                classes.insert(id, CoalesceClass::Scalar);
            }
        }
    }

    // Copy and phi-lowering chains have no type field. Propagate the scalar
    // classification to a fixed point through values and scalar constants.
    let mut changed = true;
    while changed {
        changed = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::Copy { dest, src } = inst {
                    if allocas.contains(&dest.0) || classes.contains_key(&dest.0) {
                        continue;
                    }
                    let source_is_scalar = match src {
                        Operand::Value(v) => classes.contains_key(&v.0),
                        Operand::Const(c) => scalar_const(*c),
                    };
                    if source_is_scalar {
                        classes.insert(dest.0, CoalesceClass::Scalar);
                        changed = true;
                    }
                }
            }
        }
    }

    for id in allocas {
        classes.remove(&id);
    }
    classes
}

fn instruction_value_uses(inst: &Instruction) -> Vec<u32> {
    let mut uses = Vec::new();
    for_each_operand_in_instruction(inst, |op| {
        if let Operand::Value(v) = op {
            uses.push(v.0);
        }
    });
    for_each_value_use_in_instruction(inst, |v| uses.push(v.0));
    uses.sort_unstable();
    uses.dedup();
    uses
}

fn terminator_value_uses(term: &Terminator) -> Vec<u32> {
    let mut uses = Vec::new();
    for_each_operand_in_terminator(term, |op| {
        if let Operand::Value(v) = op {
            uses.push(v.0);
        }
    });
    uses.sort_unstable();
    uses.dedup();
    uses
}

fn insert_interference(
    graph: &mut FxHashMap<u32, FxHashSet<u32>>,
    lhs: u32,
    rhs: u32,
) {
    if lhs == rhs {
        return;
    }
    graph.entry(lhs).or_default().insert(rhs);
    graph.entry(rhs).or_default().insert(lhs);
}

/// Build an SSA-style, CFG-aware interference graph after phi lowering.
///
/// Unlike the former textual interval heuristic, this solves backward liveness
/// over actual successor edges and then uses copy-aware interference insertion:
/// for `d = copy s`, `d` interferes with every value live after the copy except
/// `s` itself.  This is the standard move-coalescing rule: source and
/// destination may share a location at the handoff, but any other reaching
/// definition/path that keeps them simultaneously live creates an edge and
/// blocks the merge.
fn cfg_copy_interference(
    func: &IrFunction,
    tracked: &FxHashSet<u32>,
) -> Option<FxHashMap<u32, FxHashSet<u32>>> {
    use crate::ir::analysis;

    let label_map = analysis::build_label_map(func);
    let (_preds, succs) = analysis::build_cfg(func, &label_map);
    let succs: Vec<Vec<usize>> = (0..func.blocks.len())
        .map(|idx| succs.row(idx).iter().map(|&v| v as usize).collect())
        .collect();

    let mut block_use: Vec<FxHashSet<u32>> = vec![FxHashSet::default(); func.blocks.len()];
    let mut block_def: Vec<FxHashSet<u32>> = vec![FxHashSet::default(); func.blocks.len()];
    for (block_idx, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            for value in instruction_value_uses(inst) {
                if tracked.contains(&value) && !block_def[block_idx].contains(&value) {
                    block_use[block_idx].insert(value);
                }
            }
            if let Some(dest) = inst.dest() {
                if tracked.contains(&dest.0) {
                    block_def[block_idx].insert(dest.0);
                }
            }
        }
        for value in terminator_value_uses(&block.terminator) {
            if tracked.contains(&value) && !block_def[block_idx].contains(&value) {
                block_use[block_idx].insert(value);
            }
        }
    }

    let mut live_in: Vec<FxHashSet<u32>> = vec![FxHashSet::default(); func.blocks.len()];
    let mut live_out: Vec<FxHashSet<u32>> = vec![FxHashSet::default(); func.blocks.len()];
    let mut changed = true;
    let mut iterations = 0usize;
    // A conservative cap guards malformed irreducible CFGs. Failure to
    // converge merely under-optimizes because callers reject the empty graph.
    while changed && iterations < 256 {
        changed = false;
        iterations += 1;
        for block_idx in (0..func.blocks.len()).rev() {
            let mut out = FxHashSet::default();
            for &succ in &succs[block_idx] {
                out.extend(live_in[succ].iter().copied());
            }
            let mut input = block_use[block_idx].clone();
            for value in &out {
                if !block_def[block_idx].contains(value) {
                    input.insert(*value);
                }
            }
            if out != live_out[block_idx] {
                live_out[block_idx] = out;
                changed = true;
            }
            if input != live_in[block_idx] {
                live_in[block_idx] = input;
                changed = true;
            }
        }
    }
    if changed {
        // Do not use partially converged dataflow for a correctness proof.
        return None;
    }

    let mut graph: FxHashMap<u32, FxHashSet<u32>> = FxHashMap::default();
    for (block_idx, block) in func.blocks.iter().enumerate() {
        let mut live = live_out[block_idx].clone();
        for value in terminator_value_uses(&block.terminator) {
            if tracked.contains(&value) {
                live.insert(value);
            }
        }
        for inst in block.instructions.iter().rev() {
            let copy_source = match inst {
                Instruction::Copy { src: Operand::Value(source), .. } => Some(source.0),
                _ => None,
            };
            if let Some(dest) = inst.dest() {
                if tracked.contains(&dest.0) {
                    for &other in &live {
                        if copy_source != Some(other) {
                            insert_interference(&mut graph, dest.0, other);
                        }
                    }
                    live.remove(&dest.0);
                }
            }
            for value in instruction_value_uses(inst) {
                if tracked.contains(&value) {
                    live.insert(value);
                }
            }
        }
    }
    Some(graph)
}

fn find_root(parent: &mut FxHashMap<u32, u32>, value: u32) -> u32 {
    let mut root = value;
    while parent.get(&root).copied() != Some(root) {
        root = parent.get(&root).copied().unwrap_or(root);
    }
    let mut cursor = value;
    while parent.get(&cursor).copied().is_some_and(|p| p != root) {
        let next = parent[&cursor];
        parent.insert(cursor, root);
        cursor = next;
    }
    root
}

fn class_members(parent: &mut FxHashMap<u32, u32>, root: u32) -> Vec<u32> {
    let values: Vec<u32> = parent.keys().copied().collect();
    values.into_iter()
        .filter(|value| find_root(parent, *value) == root)
        .collect()
}

fn classes_interfere(
    parent: &mut FxHashMap<u32, u32>,
    interference: &FxHashMap<u32, FxHashSet<u32>>,
    lhs_root: u32,
    rhs_root: u32,
) -> bool {
    let lhs = class_members(parent, lhs_root);
    let rhs = class_members(parent, rhs_root);
    lhs.iter().any(|value| {
        interference.get(value).is_some_and(|neighbors| {
            rhs.iter().any(|other| neighbors.contains(other))
        })
    })
}

/// CFG-aware stack-copy coalescing.
///
/// This replaces only the stack-slot alias decision.  It does not change phi
/// lowering order, register allocation, or emitted copy semantics; it merely
/// lets provably non-interfering scalar copy webs use one stack home.  Every
/// alias is marked forceable because the proof is graph-based rather than the
/// old linear `def <= last_use` approximation.
fn build_cfg_copy_alias_map(
    func: &IrFunction,
    multi_def_values: &FxHashSet<u32>,
    reg_assigned: &FxHashMap<u32, PhysReg>,
) -> (FxHashMap<u32, u32>, FxHashSet<u32>) {
    let classes = collect_scalar_values(func);
    let mut candidates: Vec<(u32, u32)> = Vec::new();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Copy { dest, src: Operand::Value(source) } = inst {
                if dest.0 == source.0
                    || reg_assigned.contains_key(&dest.0)
                    || reg_assigned.contains_key(&source.0)
                    || !classes.contains_key(&dest.0)
                    || !classes.contains_key(&source.0)
                {
                    continue;
                }
                candidates.push((dest.0, source.0));
            }
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    if candidates.is_empty() {
        return (FxHashMap::default(), FxHashSet::default());
    }

    let tracked: FxHashSet<u32> = candidates
        .iter()
        .flat_map(|(dest, source)| [*dest, *source])
        .collect();
    let Some(interference) = cfg_copy_interference(func, &tracked) else {
        // A non-converged dataflow result is never used as an optimization proof.
        return (FxHashMap::default(), FxHashSet::default());
    };

    let mut parent: FxHashMap<u32, u32> = tracked.iter().map(|&id| (id, id)).collect();
    for (dest, source) in candidates {
        let dest_root = find_root(&mut parent, dest);
        let source_root = find_root(&mut parent, source);
        if dest_root == source_root
            || classes_interfere(&mut parent, &interference, dest_root, source_root)
        {
            continue;
        }
        // Keep a multi-def phi destination as the group owner. It has the
        // cross-block lifetime that phi incoming values must feed. Otherwise
        // use the lower ID to make the output deterministic.
        let owner = match (
            multi_def_values.contains(&dest_root),
            multi_def_values.contains(&source_root),
        ) {
            (true, false) => dest_root,
            (false, true) => source_root,
            _ => dest_root.min(source_root),
        };
        let other = if owner == dest_root { source_root } else { dest_root };
        parent.insert(other, owner);
    }

    let mut groups: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    for &value in &tracked {
        let root = find_root(&mut parent, value);
        groups.entry(root).or_default().push(value);
    }

    let mut aliases = FxHashMap::default();
    let mut force_aliases = FxHashSet::default();
    for (root, mut members) in groups {
        members.sort_unstable();
        if members.len() < 2 {
            continue;
        }
        let preferred_root = members.iter().copied()
            .filter(|id| multi_def_values.contains(id))
            .min()
            .unwrap_or(root);
        for member in members {
            if member != preferred_root {
                aliases.insert(member, preferred_root);
                force_aliases.insert(member);
            }
        }
    }

    if std::env::var("CCC_DEBUG_SLOT_COALESCE").is_ok() && !aliases.is_empty() {
        let mut pairs: Vec<(u32, u32)> = aliases.iter().map(|(&a, &b)| (a, b)).collect();
        pairs.sort_unstable();
        eprintln!(
            "[CFG-SLOT-COALESCE] fn={} candidates={} aliases={} pairs={:?}",
            func.name,
            tracked.len(),
            pairs.len(),
            pairs,
        );
    }
    (aliases, force_aliases)
}

/// Build the copy alias map: dest_id -> root_id for Copy instructions where
/// dest and src can share the same stack slot.
///
/// Safety: only coalesces when the Copy is the SOLE use of the source value,
/// guaranteeing the source is dead after the Copy (avoids the "lost copy"
/// problem in phi parallel copy groups).
/// Returns `(copy_alias, phi_web_aliases)` where phi_web_aliases contains value IDs
/// that were coalesced via phi-web analysis and need force-overwrite in resolve_copy_aliases.
pub(super) fn build_copy_alias_map(
    func: &IrFunction,
    def_block: &FxHashMap<u32, usize>,
    multi_def_values: &FxHashSet<u32>,
    reg_assigned: &FxHashMap<u32, PhysReg>,
    use_blocks_map: &FxHashMap<u32, Vec<usize>>,
    cached_liveness: &Option<crate::backend::liveness::LivenessResult>,
) -> (FxHashMap<u32, u32>, FxHashSet<u32>) {
    if cfg_copy_coalesce_enabled() {
        return build_cfg_copy_alias_map(func, multi_def_values, reg_assigned);
    }

    // Count uses of each value across all instructions.
    let mut use_count: FxHashMap<u32, u32> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    *use_count.entry(v.0).or_insert(0) += 1;
                }
            });
            for_each_value_use_in_instruction(inst, |v| {
                *use_count.entry(v.0).or_insert(0) += 1;
            });
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                *use_count.entry(v.0).or_insert(0) += 1;
            }
        });
    }

    // Build last-use instruction point map (NOT using LiveInterval.end which
    // includes live-through extensions from backward dataflow). We compute actual
    // last-instruction-use points by scanning all instructions directly.
    // This correctly identifies values whose last explicit use IS the Copy instruction.
    let mut last_use_instr: FxHashMap<u32, u32> = FxHashMap::default();
    let mut copy_program_points: FxHashMap<(usize, usize), u32> = FxHashMap::default();
    if cached_liveness.is_some() {
        let mut pp: u32 = 0;
        for (blk_idx, block) in func.blocks.iter().enumerate() {
            for (inst_idx, inst) in block.instructions.iter().enumerate() {
                // Record program point for Copy instructions.
                if matches!(inst, Instruction::Copy { .. }) {
                    copy_program_points.insert((blk_idx, inst_idx), pp);
                }
                // Track last-use for all operands (reads of values).
                for_each_operand_in_instruction(inst, |op| {
                    if let Operand::Value(v) = op {
                        last_use_instr.insert(v.0, pp);
                    }
                });
                for_each_value_use_in_instruction(inst, |v| {
                    last_use_instr.insert(v.0, pp);
                });
                pp += 1;
            }
            // Terminators also use operands.
            for_each_operand_in_terminator(&block.terminator, |op| {
                if let Operand::Value(v) = op {
                    last_use_instr.insert(v.0, pp);
                }
            });
            pp += 1;
        }
    }

    // Collect Copy instructions eligible for aliasing.
    let mut raw_aliases: Vec<(u32, u32)> = Vec::new();
    for (blk_idx, block) in func.blocks.iter().enumerate() {
        for (inst_idx, inst) in block.instructions.iter().enumerate() {
            if let Instruction::Copy { dest, src: Operand::Value(src_val) } = inst {
                let d = dest.0;
                let s = src_val.0;
                // Never alias a multi-defined source (the aliased value must be
                // single-def; multi-def values have complex liveness that makes
                // slot sharing unsafe).
                if multi_def_values.contains(&s) {
                    continue;
                }
                if reg_assigned.contains_key(&d) || reg_assigned.contains_key(&s) {
                    continue;
                }
                // Coalesce if Copy is the sole use of the source, OR if the
                // source's live interval ends at/before this Copy (src is dead after).
                let sole_use = use_count.get(&s).copied().unwrap_or(0) == 1;
                if !sole_use {
                    // Block-local dead-after-copy check: is src unused after this
                    // Copy within this block, AND not used in any other block?
                    // (The global last_use_instr doesn't work because phi sources
                    // may have Copies in multiple predecessor blocks.)
                    let src_use_blocks = use_blocks_map.get(&s);
                    let src_only_in_this_block = src_use_blocks
                        .map(|blks| blks.iter().all(|&b| b == blk_idx))
                        .unwrap_or(true);
                    if !src_only_in_this_block {
                        continue;
                    }
                    // Check: is src used AFTER this Copy instruction in this block?
                    let mut used_after = false;
                    for later_inst in &block.instructions[inst_idx + 1..] {
                        let mut found = false;
                        for_each_operand_in_instruction(later_inst, |op| {
                            if let Operand::Value(v) = op {
                                if v.0 == s { found = true; }
                            }
                        });
                        for_each_value_use_in_instruction(later_inst, |v| {
                            if v.0 == s { found = true; }
                        });
                        if found { used_after = true; break; }
                    }
                    // Also check the terminator.
                    if !used_after {
                        for_each_operand_in_terminator(&block.terminator, |op| {
                            if let Operand::Value(v) = op {
                                if v.0 == s { used_after = true; }
                            }
                        });
                    }
                    if used_after {
                        continue;
                    }
                }

                let src_def_blk = def_block.get(&s).copied();
                let src_in_copy_block = src_def_blk == Some(blk_idx);
                let dest_cross_block = use_blocks_map.get(&d)
                    .map(|blks| blks.iter().any(|&b| b != blk_idx))
                    .unwrap_or(false);

                if src_in_copy_block && dest_cross_block {
                    // Phi-copy pattern: src is defined and killed in this block
                    // (sole use = this copy), but dest is used in other blocks.
                    //
                    // dest may be multi-defined (phi elimination creates one Copy
                    // per predecessor that defines the phi dest). That's fine here
                    // because dest is the slot OWNER (root), not the aliased value.
                    // All definitions of dest write to the same slot, making the
                    // backedge copy a same-slot no-op.
                    //
                    // Reversed aliasing (src → dest) is safe: src gets dest's slot,
                    // which is already live across all of dest's uses. After aliasing,
                    // the copy becomes a same-slot no-op (skipped by generate_copy).
                    // This eliminates the double-slot pattern that arises from phi
                    // elimination in loops with spilled variables.
                    raw_aliases.push((s, d)); // src uses dest's (wider-live) slot
                    continue;
                }

                // Standard same-block coalescing: dest uses src's slot.
                // Dest must not be multi-defined (would make slot-sharing unsafe
                // for the standard direction), and dest's uses must all be in the
                // same block as source's definition.
                if multi_def_values.contains(&d) {
                    continue;
                }
                if dest_cross_block {
                    continue;
                }
                raw_aliases.push((d, s));
            }
        }
    }

    // ── Phi-web coalescing ──────────────────────────────────────────────────
    //
    // Force phi web members to share the same stack slot. For Copy(dest, src)
    // where dest is multi-def (phi dest), coalesce src→dest if src is NOT live
    // at any program point where dest is written by a DIFFERENT Copy.
    //
    // Uses liveness intervals for precise interference: src interferes with a
    // Copy(dest, src') at program point P' if src.start <= P' <= src.end.
    // This correctly handles switch statements (case arms are mutually exclusive
    // so their intervals don't overlap) and loop patterns.
    let mut phi_web_aliases: FxHashSet<u32> = FxHashSet::default();
    if !std::env::var("CCC_NO_PHI_WEB_COALESCE").is_ok() {
        // Phase 1: Collect all Copy(dest, src) pairs with program points.
        // Program points match liveness.rs numbering (1 per instruction + 1 per terminator).
        let mut phi_copies: FxHashMap<u32, Vec<(u32, usize, u32)>> = FxHashMap::default(); // dest → [(src, blk, pp)]
        let mut dest_copy_points: FxHashMap<u32, Vec<u32>> = FxHashMap::default(); // dest → [program_points]
        let already_aliased: FxHashSet<u32> = raw_aliases.iter().map(|&(a, _)| a).collect();
        {
            let mut pp: u32 = 0;
            for (blk_idx, block) in func.blocks.iter().enumerate() {
                for inst in &block.instructions {
                    if let Instruction::Copy { dest, src: Operand::Value(src_val) } = inst {
                        let d = dest.0;
                        let s = src_val.0;
                        if multi_def_values.contains(&d)
                            && !reg_assigned.contains_key(&s)
                            && !already_aliased.contains(&s)
                        {
                            phi_copies.entry(d).or_default().push((s, blk_idx, pp));
                            dest_copy_points.entry(d).or_default().push(pp);
                        }
                    }
                    pp += 1;
                }
                pp += 1; // terminator
            }
        }

        // Phase 2: Build interval map from liveness data.
        let interval_map: FxHashMap<u32, (u32, u32)> = cached_liveness
            .as_ref()
            .map(|lr| lr.intervals.iter().map(|iv| (iv.value_id, (iv.start, iv.end))).collect())
            .unwrap_or_default();

        // Phase 3: For each phi web, check interference using liveness intervals.
        for (dest_id, sources) in &phi_copies {
            if sources.len() < 2 { continue; }
            if reg_assigned.contains_key(dest_id) { continue; }

            // All program points where dest is written by any Copy.
            let all_copy_points = match dest_copy_points.get(dest_id) {
                Some(pts) => pts,
                None => continue,
            };

            for &(src_id, _src_blk, src_copy_pp) in sources {
                // SOUNDNESS (maketrees miscompile, 2026-08): only loop-carried
                // WEB MEMBERS may be aliased into the web. An external feed
                // value (single-def, defined outside the web — e.g. the
                // GlobalAddr of the final-iteration separator string) has its
                // own home; aliasing it in makes the elided phi Copy leave
                // the web home holding a STALE earlier value, which later
                // reads of the dest then return. Web membership = src is
                // itself a phi dest (multi-def loop-carried value).
                if !phi_copies.contains_key(&src_id)
                    && !multi_def_values.contains(&src_id)
                {
                    continue;
                }
                // Get src's liveness interval.
                let src_interval = interval_map.get(&src_id);

                // Interference check: is src live at any program point where dest
                // is written by a DIFFERENT Copy?
                //
                // For single-def sources: use_blocks check is sufficient —
                // if src has no uses in dest's other def blocks, no interference.
                //
                // For multi-def sources: the use_blocks check is too conservative
                // because multi-def values have definitions (not just uses) spread
                // across blocks. Instead, check: does src's definition set overlap
                // with dest's OTHER def blocks? If src is DEFINED in a block where
                // dest has a DIFFERENT Copy, the two values co-exist there.
                // But if src is defined there BY THE SAME Copy pattern (it's also
                // a phi dest being written), the write order is deterministic
                // and they won't conflict.
                //
                // Simple safe heuristic: src doesn't interfere if ALL of src's
                // use blocks are either (a) the Copy's block, or (b) blocks where
                // src is defined (def_block or multi-def blocks for src).
                // Collect dest's other def blocks (blocks with Copy(dest, X) where X ≠ src).
                let other_def_blks: Vec<usize> = sources.iter()
                    .filter(|&&(_, _, pp)| pp != src_copy_pp)
                    .map(|&(_, blk, _)| blk)
                    .collect();

                // Check: does src have uses in dest's other def blocks that are
                // NOT themselves Copy(dest, src) instructions?
                //
                // Key insight: use_blocks_map counts Copy(dest, src) as a "use of src"
                // in the Copy's block. But if src and dest share a slot, that Copy
                // becomes a no-op — so it's not real interference. We must exclude
                // uses that come from Copy instructions writing to dest.
                //
                // Build set of blocks where src has NON-COPY uses (actual computation
                // that reads src for purposes other than feeding this phi).
                let src_non_copy_use_blks: Vec<usize> = {
                    let mut non_copy_blks = Vec::new();
                    for (blk_idx, block) in func.blocks.iter().enumerate() {
                        let mut has_non_copy_use = false;
                        for inst in &block.instructions {
                            // Skip Copy instructions that write to dest (these are
                            // the phi copies we're trying to coalesce).
                            if let Instruction::Copy { dest: copy_dest, src: Operand::Value(copy_src) } = inst {
                                if copy_dest.0 == *dest_id && copy_src.0 == src_id {
                                    continue; // This is the phi Copy — not real interference
                                }
                            }
                            // Check if this instruction uses src as an operand.
                            let mut uses_src = false;
                            for_each_operand_in_instruction(inst, |op| {
                                if let Operand::Value(v) = op {
                                    if v.0 == src_id { uses_src = true; }
                                }
                            });
                            for_each_value_use_in_instruction(inst, |v| {
                                if v.0 == src_id { uses_src = true; }
                            });
                            if uses_src { has_non_copy_use = true; break; }
                        }
                        if !has_non_copy_use {
                            // Also check terminator.
                            for_each_operand_in_terminator(&block.terminator, |op| {
                                if let Operand::Value(v) = op {
                                    if v.0 == src_id { has_non_copy_use = true; }
                                }
                            });
                        }
                        if has_non_copy_use {
                            non_copy_blks.push(blk_idx);
                        }
                    }
                    non_copy_blks
                };

                let interferes = src_non_copy_use_blks.iter().any(|use_blk| {
                    other_def_blks.contains(use_blk)
                });

                if !interferes {
                    raw_aliases.push((src_id, *dest_id));
                    phi_web_aliases.insert(src_id);
                }
            }
        }
    }

    // Build alias map with transitive resolution: follow chains to find root.
    // Safety limit on chain depth guards against pathological cycles.
    const MAX_ALIAS_CHAIN_DEPTH: usize = 100;
    let mut copy_alias: FxHashMap<u32, u32> = FxHashMap::default();
    for (dest_id, src_id) in raw_aliases {
        let mut root = src_id;
        let mut depth = 0;
        while let Some(&parent) = copy_alias.get(&root) {
            root = parent;
            depth += 1;
            if depth > MAX_ALIAS_CHAIN_DEPTH { break; }
        }
        if root != dest_id {
            copy_alias.insert(dest_id, root);
        }
    }

    // Remove aliases where root or dest is an alloca (alloca slots are special).
    let alloca_ids: FxHashSet<u32> = func.blocks.iter()
        .flat_map(|b| b.instructions.iter())
        .filter_map(|inst| {
            if let Instruction::Alloca { dest, .. } = inst { Some(dest.0) } else { None }
        })
        .collect();
    copy_alias.retain(|dest_id, root_id| {
        !alloca_ids.contains(root_id) && !alloca_ids.contains(dest_id)
    });

    // Remove aliases for InlineAsm output pointer values. InlineAsm Phase 4 reads
    // output pointers from stack slots AFTER the asm executes; if aliased, the
    // root's slot may be reused between the Copy and the InlineAsm, corrupting
    // the pointer read in Phase 4.
    let mut asm_output_ptrs: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::InlineAsm { outputs, .. } = inst {
                for (_, v, _) in outputs {
                    asm_output_ptrs.insert(v.0);
                }
            }
        }
    }
    if !asm_output_ptrs.is_empty() {
        copy_alias.retain(|dest_id, _| !asm_output_ptrs.contains(dest_id));
    }

    if std::env::var("CCC_DEBUG_SLOT_COALESCE").is_ok() && !copy_alias.is_empty() {
        let mut aliases: Vec<(u32, u32)> = copy_alias.iter()
            .map(|(&dest, &root)| (dest, root))
            .collect();
        aliases.sort_unstable();
        eprintln!(
            "[SLOT-COALESCE] fn={} aliases={} phi_web_aliases={} pairs={:?}",
            func.name,
            aliases.len(),
            phi_web_aliases.len(),
            aliases,
        );
    }

    (copy_alias, phi_web_aliases)
}

/// Identify values that can skip stack slot allocation because they are
/// produced and consumed in adjacent instructions within the same block.
///
/// A value V defined at instruction I can skip its slot if:
/// 1. V has exactly one use as an Operand (loaded via operand_to_rax/rcx)
/// 2. That use is at instruction I+1 (or in the block terminator if I is last)
/// 3. V is the FIRST Operand of the consumer (loaded first into the accumulator)
/// 4. V is NOT used as a Value reference (ptr in Store/Load, base in GEP, etc.)
/// 5. V is not i128/f128 (these need 16-byte slots with special handling)
/// 6. V is not from a Copy instruction (copy aliasing needs the root's slot)
/// 7. V is not from an Alloca (allocas always need addressable slots)
///
/// The codegen accumulator cache ensures correctness: store_rax_to sets the
/// cache, and the next instruction's operand_to_rax finds V there.
pub(super) fn compute_immediately_consumed(func: &IrFunction, lhs_first_binop: bool) -> FxHashSet<u32> {
    let mut result = FxHashSet::default();

    // First pass: count uses per value (both Operand and Value-ref uses).
    let mut operand_use_count: FxHashMap<u32, u32> = FxHashMap::default();
    let mut has_value_ref_use: FxHashSet<u32> = FxHashSet::default();

    for block in &func.blocks {
        for inst in &block.instructions {
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    *operand_use_count.entry(v.0).or_insert(0) += 1;
                }
            });
            for_each_value_use_in_instruction(inst, |v| {
                has_value_ref_use.insert(v.0);
            });
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                *operand_use_count.entry(v.0).or_insert(0) += 1;
            }
        });
    }

    // Collect all copy-alias roots: values that serve as the slot source for copies.
    // These must keep their slots since aliased copies will use them.
    let mut copy_alias_roots: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Copy { src: Operand::Value(v), .. } = inst {
                copy_alias_roots.insert(v.0);
            }
        }
    }

    // Second pass: check adjacency and first-operand conditions.
    for block in &func.blocks {
        let insts = &block.instructions;
        for (i, inst) in insts.iter().enumerate() {
            let dest = match inst.dest() {
                Some(d) => d,
                None => continue,
            };

            // Only acc-preserving producers are safe: after these execute,
            // the accumulator cache still holds the result. Cache-invalidating
            // instructions (Call, Atomic*, DynAlloca, etc.) clear the cache
            // after store_rax_to, so the next instruction can't find the value.
            if !is_acc_preserving_producer(inst) { continue; }
            // Skip i128/f128 (special 16-byte handling uses emit_load_acc_pair /
            // emit_store_acc_pair which bypass the normal accumulator cache).
            if involves_i128_or_f128(inst) { continue; }
            // Skip if value has Value-ref uses (ptr/base in Store/Load/GEP).
            if has_value_ref_use.contains(&dest.0) { continue; }
            // Skip if value is a copy-alias root (other values share its slot).
            if copy_alias_roots.contains(&dest.0) { continue; }
            // Must have exactly one Operand use.
            let use_cnt = operand_use_count.get(&dest.0).copied().unwrap_or(0);
            if use_cnt != 1 { continue; }
            if use_cnt != 1 { continue; }

            // Check if the single use is in the immediately next instruction
            // or in the block terminator (if this is the last instruction).
            if i + 1 < insts.len() {
                // Use must be in instruction i+1, as the first Operand.
                let next = &insts[i + 1];
                if is_safe_sole_consumer(next, dest.0, lhs_first_binop) {
                    result.insert(dest.0);
                }
            } else {
                // Last instruction: use must be in the terminator, as the sole Operand.
                if is_sole_operand_of_terminator(&block.terminator, dest.0) {
                    result.insert(dest.0);
                }
            }
        }
    }

    result
}

/// Check if an instruction is an "acc-preserving" producer: after execution,
/// the accumulator register cache still holds the result value. Only these
/// instructions can participate in the skip-slot optimization as producers.
///
/// Cache-invalidating instructions (Call, Store, Atomic*, DynAlloca, InlineAsm,
/// etc.) call invalidate_all() after execution, clearing the cache.
fn is_acc_preserving_producer(inst: &Instruction) -> bool {
    matches!(inst,
        Instruction::Load { .. }
        | Instruction::BinOp { .. }
        | Instruction::UnaryOp { .. }
        | Instruction::Cmp { .. }
        | Instruction::Cast { .. }
        | Instruction::GetElementPtr { .. }
        | Instruction::GlobalAddr { .. }
        | Instruction::Select { .. }
        | Instruction::LabelAddr { .. }
    )
}

/// Check if an instruction involves I128/U128/F128 types in any operand position.
/// These use emit_load_acc_pair / emit_store_acc_pair which bypass the normal
/// accumulator cache, so they cannot participate in skip-slot optimization.
fn involves_i128_or_f128(inst: &Instruction) -> bool {
    fn is_wide(ty: IrType) -> bool {
        matches!(ty, IrType::I128 | IrType::U128 | IrType::F128)
    }
    match inst {
        Instruction::Cast { from_ty, to_ty, .. } => is_wide(*from_ty) || is_wide(*to_ty),
        Instruction::UnaryOp { ty, .. } => is_wide(*ty),
        Instruction::BinOp { ty, .. } => is_wide(*ty),
        Instruction::Cmp { ty, .. } => is_wide(*ty),
        Instruction::Load { ty, .. } => is_wide(*ty),
        _ => {
            // For other instructions, just check the result type.
            matches!(inst.result_type(), Some(ty) if is_wide(ty))
        }
    }
}

/// Check if value_id is the sole Operand loaded by the given instruction,
/// with guaranteed loading order (no other operand loaded before it).
///
/// Only single-operand consumers are safe by default: Store (val loaded first),
/// Cast, UnaryOp, Copy. Two-operand instructions (BinOp, Cmp) are excluded on
/// x86/ARM because codegen may load the OTHER operand first (e.g. BinOp's
/// rhs_conflicts path, float Cmp's Lt/Le operand swap). GEP excluded because
/// OverAligned base computation clobbers %rax before offset is loaded.
///
/// When `lhs_first_binop` is true (RISC-V), BinOp and Cmp are also safe when
/// value_id is the lhs operand, because the RISC-V backend unconditionally
/// loads lhs before rhs with no register-direct conflict paths.
fn is_safe_sole_consumer(inst: &Instruction, value_id: u32, lhs_first_binop: bool) -> bool {
    match inst {
        // Store: val is always loaded first via emit_load_operand (operand_to_rax)
        Instruction::Store { val: Operand::Value(v), .. } => v.0 == value_id,
        // Single-operand instructions: loaded via operand_to_rax, no other operand
        Instruction::Cast { src: Operand::Value(v), .. } => v.0 == value_id,
        Instruction::UnaryOp { src: Operand::Value(v), .. } => v.0 == value_id,
        Instruction::Copy { src: Operand::Value(v), .. } => v.0 == value_id,
        // BinOp: safe on architectures that always load lhs first (RISC-V)
        Instruction::BinOp { lhs: Operand::Value(v), .. } if lhs_first_binop => v.0 == value_id,
        // Cmp: safe on architectures that always load lhs first (RISC-V)
        Instruction::Cmp { lhs: Operand::Value(v), .. } if lhs_first_binop => v.0 == value_id,
        // All other instructions: not safe (GEP, Call, Select, etc.)
        _ => false,
    }
}

/// Check if value_id is the sole operand of a block terminator.
fn is_sole_operand_of_terminator(term: &Terminator, value_id: u32) -> bool {
    match term {
        Terminator::Return(Some(Operand::Value(v))) => v.0 == value_id,
        Terminator::CondBranch { cond: Operand::Value(v), .. } => v.0 == value_id,
        Terminator::Switch { val: Operand::Value(v), .. } => v.0 == value_id,
        Terminator::IndirectBranch { target: Operand::Value(v), .. } => v.0 == value_id,
        _ => false,
    }
}

#[cfg(test)]
mod cfg_copy_coalesce_tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, IrBinOp, Value};

    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            source_spans: Vec::new(),
            terminator,
        }
    }

    fn scalar_def(dest: u32, value: i32) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op: IrBinOp::Add,
            lhs: Operand::Const(IrConst::I32(value)),
            rhs: Operand::Const(IrConst::I32(0)),
            ty: IrType::I32,
        }
    }

    #[test]
    fn cfg_copy_coalesces_a_straight_line_copy() {
        let mut func = IrFunction::new("straight".to_string(), IrType::I32, vec![], false);
        func.blocks.push(block(
            0,
            vec![
                scalar_def(0, 7),
                Instruction::Copy { dest: Value(1), src: Operand::Value(Value(0)) },
            ],
            Terminator::Return(Some(Operand::Value(Value(1)))),
        ));
        let (aliases, force) = build_cfg_copy_alias_map(
            &func,
            &FxHashSet::default(),
            &FxHashMap::default(),
        );
        assert_eq!(aliases.get(&1), Some(&0));
        assert!(force.contains(&1));
    }

    #[test]
    fn cfg_copy_rejects_a_phi_edge_source_live_on_another_path() {
        // d is a lowered phi: d = x on the left edge, d = y on the right.
        // x is also used after the join, so x and d must never share a slot.
        let mut func = IrFunction::new("diamond".to_string(), IrType::I32, vec![], false);
        func.blocks = vec![
            block(
                0,
                vec![scalar_def(0, 11)],
                Terminator::CondBranch {
                    cond: Operand::Const(IrConst::I32(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            block(
                1,
                vec![Instruction::Copy { dest: Value(2), src: Operand::Value(Value(0)) }],
                Terminator::Branch(BlockId(3)),
            ),
            block(
                2,
                vec![
                    scalar_def(1, 22),
                    Instruction::Copy { dest: Value(2), src: Operand::Value(Value(1)) },
                ],
                Terminator::Branch(BlockId(3)),
            ),
            block(
                3,
                vec![Instruction::BinOp {
                    dest: Value(3),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Value(Value(2)),
                    ty: IrType::I32,
                }],
                Terminator::Return(Some(Operand::Value(Value(3)))),
            ),
        ];
        let mut multi_def = FxHashSet::default();
        multi_def.insert(2);
        let (aliases, _) = build_cfg_copy_alias_map(&func, &multi_def, &FxHashMap::default());
        assert_ne!(aliases.get(&0), Some(&2));
        assert_ne!(aliases.get(&2), Some(&0));
        // y is edge-local and may safely use the phi web's home.
        assert_eq!(aliases.get(&1), Some(&2));
    }

    #[test]
    fn cfg_copy_coalesces_loop_carried_phi_sources() {
        // Lowered loop phi: state is initialized from `init` on entry and
        // from `next` on the back-edge. Neither incoming value is used after
        // its edge copy, so both may share the phi web's stack home.
        let mut func = IrFunction::new("loop_phi".to_string(), IrType::I32, vec![], false);
        func.blocks = vec![
            block(
                0,
                vec![
                    scalar_def(0, 3),
                    Instruction::Copy { dest: Value(2), src: Operand::Value(Value(0)) },
                ],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![Instruction::BinOp {
                    dest: Value(1),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Const(IrConst::I32(1)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(
                2,
                vec![Instruction::Copy { dest: Value(2), src: Operand::Value(Value(1)) }],
                Terminator::Branch(BlockId(1)),
            ),
            block(3, vec![], Terminator::Return(Some(Operand::Value(Value(2))))),
        ];
        let mut multi_def = FxHashSet::default();
        multi_def.insert(2);
        let (aliases, _) = build_cfg_copy_alias_map(&func, &multi_def, &FxHashMap::default());
        assert_eq!(aliases.get(&0), Some(&2));
        // `next` is redefined at the loop header on the next iteration, so
        // the CFG liveness proof conservatively keeps it distinct from the
        // phi web instead of relying on textual interval order.
        assert_ne!(aliases.get(&1), Some(&2));
    }

    #[test]
    fn cfg_copy_excludes_i128_from_scalar_slot_aliasing() {
        let mut func = IrFunction::new("wide".to_string(), IrType::I32, vec![], false);
        func.blocks.push(block(
            0,
            vec![
                Instruction::BinOp {
                    dest: Value(0),
                    op: IrBinOp::Add,
                    lhs: Operand::Const(IrConst::I128(1)),
                    rhs: Operand::Const(IrConst::I128(2)),
                    ty: IrType::I128,
                },
                Instruction::Copy { dest: Value(1), src: Operand::Value(Value(0)) },
            ],
            Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
        ));
        let (aliases, _) = build_cfg_copy_alias_map(
            &func,
            &FxHashSet::default(),
            &FxHashMap::default(),
        );
        assert!(aliases.is_empty());
    }
}

/// True if the intrinsic's codegen reads its args OUTSIDE the cache-aware
/// loaders (sse_load_arg/avx_load_arg_to) — i.e. it invalidates the vector
/// last-store peephole and would observe a deferred (never-written) slot.
pub(crate) fn is_raw_reader_intrinsic(op: &crate::ir::intrinsics::IntrinsicOp) -> bool {
    use crate::ir::intrinsics::IntrinsicOp as O;
    matches!(
        op,
        O::Pblendvb128
            | O::Loadldi128
            | O::Storeldi128
            | O::FmaF64x2
            | O::FmaF64x4
            | O::FmaF64x4Hoisted
            | O::FmaF64x4SIB
            | O::BroadcastLoadF64
            | O::LoadF64x2
            | O::LoadF64x4
            | O::LoadI32x4
            | O::LoadI32x8
            | O::HorizontalAddF64x2
            | O::HorizontalAddF64x4
            | O::HorizontalAddI32x4
            | O::HorizontalAddI32x8
            | O::VecLoadF64x2
            | O::VecLoadF64x4
            | O::VecLoadI32x4
            | O::VecLoadI32x8
            | O::VecAddF64x2
            | O::VecAddF64x4
            | O::VecAddI32x4
            | O::VecAddI32x8
            | O::VecMulF64x2
            | O::VecMulF64x4
            | O::VecHorizontalAddF64x2
            | O::VecHorizontalAddF64x4
            | O::VecHorizontalAddI32x4
            | O::VecHorizontalAddI32x8
    )
}

/// Ops whose codegen goes through `emit_sse_binary_128` / `emit_avx_binary_256`
/// — the two-operand emitters that load the last-stored operand FIRST into
/// %xmm1/%ymm1 when args[1] is still in %xmm0/%ymm0 (the v5 load-order swap).
/// Only these can soundly consume a deferred value from args[1].
fn is_two_operand_binary(op: &crate::ir::intrinsics::IntrinsicOp) -> bool {
    use crate::ir::intrinsics::IntrinsicOp as O;
    matches!(
        op,
        // emit_sse_binary_128
        O::Pcmpeqb128 | O::Pcmpeqd128 | O::Psubusb128 | O::Psubsb128
        | O::Por128 | O::Pand128 | O::Pxor128
        | O::AddPs128 | O::SubPs128 | O::MulPs128
        | O::AddPd128 | O::SubPd128 | O::MulPd128
        | O::Paddw128 | O::Psubw128 | O::Pmulhw128 | O::Pmullw128
        | O::Pmuludq128 | O::Pmuldq128 | O::Pmulld128
        | O::Pmaddwd128 | O::Pmaddubsw128
        | O::Pcmpgtw128 | O::Pcmpgtb128
        | O::Paddd128 | O::Psubd128
        | O::Paddb128 | O::Psubb128 | O::Psubusw128
        | O::Psadbw128
        | O::Pshufb128
        | O::Pmaxub128 | O::Pminub128
        | O::Pmovzxbw128 | O::Pmovzxwd128
        | O::Packssdw128 | O::Packsswb128 | O::Packuswb128
        | O::Punpcklbw128 | O::Punpckhbw128
        | O::Punpcklwd128 | O::Punpckhwd128
        | O::Aesenc128 | O::Aesenclast128 | O::Aesdec128 | O::Aesdeclast128
        | O::AddF64x2 | O::MulF64x2 | O::AddI32x4
        // emit_avx_binary_256
        | O::Paddb256 | O::Paddw256 | O::Paddd256
        | O::Psubb256 | O::Psubw256 | O::Psubusw256
        | O::Psadbw256 | O::Pmaddubsw256 | O::Pmaddwd256
        | O::Pcmpeqb256 | O::Pcmpgtb256 | O::Pshufb256
        | O::Pmaxub256 | O::Pminub256
        | O::Pxor256 | O::Por256 | O::Pand256
        | O::AddF64x4 | O::MulF64x4 | O::AddI32x8
    )
}

/// Values whose vector-result store can be DEFERRED (skipped entirely) — the
/// v5 "accumulator renaming" optimization.
///
/// A vector intrinsic (e.g. `_mm_xor_si128`) computes its result in
/// %xmm0/%ymm0 and normally stores it to its dest slot; the single-entry
/// last-store peephole then lets the immediately-following load of the same
/// value skip the reload. When that following load is the value's ONLY use,
/// the intermediate store is pure overhead: the store is skipped and the
/// register carries the value straight into the consumer (a net store+load
/// elimination).
///
/// Soundness (each condition is load-bearing):
/// 1. V is the dest alloca of a vector intrinsic P (never a user pointer —
///    excludes `_mm_storeu_*`-style ops whose dest_ptr is a real store target).
/// 2. V has exactly one ARG use (intrinsic dest_ptr is a write, not counted).
/// 3. That use is args[0] (or args[1] of a two-operand binary emitter, whose
///    codegen loads the last-stored operand FIRST into %xmm1/%ymm1) of the
///    instruction IMMEDIATELY following P in the same block (adjacency ⇒
///    nothing can clobber the register or read the slot in between).
/// 4. The consumer's args[0] load goes through sse_load_arg/avx_load_arg_to
///    into %xmm0/%ymm0 as its FIRST vector op. Excluded raw readers (FMA /
///    load / horizontal-reduction / auto-vectorizer family, the raw movq
///    load/store helpers, Pblendvb128) bypass the loaders and would read the
///    never-written slot or clobber the held register first.
/// 5. V is not volatile, not written through other paths (no value-ref use,
///    no copy-alias root, not an address-taken alloca).
/// 6. ALL def sites of V qualify individually. The defer set is keyed by
///    slot: codegen skips the store at every instruction writing the slot, so
///    one non-adjacent site (C temporaries reused across two loop bodies)
///    disqualifies the slot entirely — otherwise that site's consumer would
///    read a never-written slot (adler32_ssse3 miscompile, fixed 2026-08).
pub(super) fn compute_vector_defer_values(func: &IrFunction) -> FxHashSet<u32> {
    use crate::ir::intrinsics::IntrinsicOp;
    let mut result = FxHashSet::default();

    // Pass 0: collect alloca dests (and volatile ones) + copy-alias roots.
    let mut allocas: FxHashSet<u32> = FxHashSet::default();
    let mut volatile_allocas: FxHashSet<u32> = FxHashSet::default();
    let mut copy_alias_roots: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Alloca {
                    dest,
                    volatile,
                    semantic_volatile,
                    ..
                } => {
                    allocas.insert(dest.0);
                    if *volatile || *semantic_volatile {
                        volatile_allocas.insert(dest.0);
                    }
                }
                Instruction::Copy { src: Operand::Value(v), .. } => {
                    copy_alias_roots.insert(v.0);
                }
                _ => {}
            }
        }
    }

    // Pass 1: collect every Operand use site (block, inst index) per value,
    // plus global flags for non-ARG uses. Intrinsic dest_ptr is a WRITE (not a
    // use); Call args / Store val / Memcpy / terminator uses are ARG uses but
    // NOT cache-aware (the slot is really read), so the per-producer scan below
    // rejects them.
    let mut uses: FxHashMap<u32, Vec<(usize, usize)>> = FxHashMap::default();
    let mut has_value_ref_use: FxHashSet<u32> = FxHashSet::default();
    let mut non_intrinsic_arg_use: FxHashSet<u32> = FxHashSet::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            match inst {
                Instruction::Intrinsic { args, .. } => {
                    for arg in args {
                        if let Operand::Value(v) = arg {
                            uses.entry(v.0).or_default().push((bi, ii));
                        }
                    }
                }
                other => {
                    for_each_operand_in_instruction(other, |op| {
                        if let Operand::Value(v) = op {
                            uses.entry(v.0).or_default().push((bi, ii));
                            non_intrinsic_arg_use.insert(v.0);
                        }
                    });
                    for_each_value_use_in_instruction(other, |v| {
                        has_value_ref_use.insert(v.0);
                    });
                }
            }
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                uses.entry(v.0).or_default().push((bi, usize::MAX));
                non_intrinsic_arg_use.insert(v.0);
            }
        });
    }

    // Instructions that clear the vector-register peephole (the last-store
    // cache). A cache-aware load AFTER one of these does a REAL memory load,
    // so a deferred store would be observed. Includes the raw-reader
    // intrinsics, which invalidate the peephole by design.
    let is_vec_invalidator = |inst: &Instruction| -> bool {
        match inst {
            Instruction::Call { .. } | Instruction::CallIndirect { .. }
            | Instruction::InlineAsm { .. } | Instruction::Memcpy { .. }
            | Instruction::DynAlloca { .. } | Instruction::Store { .. }
            | Instruction::AtomicLoad { .. } | Instruction::AtomicStore { .. }
            | Instruction::AtomicRmw { .. } | Instruction::AtomicCmpxchg { .. }
            | Instruction::AtomicInc { .. } => true,
            Instruction::Intrinsic { op, .. } => is_raw_reader_intrinsic(op),
            _ => false,
        }
    };

    // Pass 2: per-DEF window analysis. A slot (alloca) may be written by
    // multiple producers (loop unrolling reuses the same __m256i variable),
    // each in possibly different blocks. A producer's store is observable only
    // by the reads that occur after THIS write and before the NEXT write of the
    // same slot (its "window"). The vector last-store cache is SINGLE-ENTRY:
    // a real vector load of any other value evicts it, and a cache hit
    // consumes it — so a store may be deferred only when the def has EXACTLY
    // ONE use, with no intervening vector-loading instruction (intrinsic) and
    // no loop-carried read of the slot in the same block (a use before the
    // def means the block re-enters via a backedge and the slot is the
    // loop-carried home that must be updated every iteration).
    let mut defs: FxHashMap<u32, Vec<(usize, usize)>> = FxHashMap::default();
    let mut def_blocks: FxHashMap<u32, FxHashSet<usize>> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Instruction::Intrinsic {
                dest_ptr: Some(d),
                op,
                ..
            } = inst
            {
                if matches!(
                    op,
                    IntrinsicOp::Storedqu
                        | IntrinsicOp::Storeu256
                        | IntrinsicOp::Store256
                        | IntrinsicOp::Storeldi128
                        | IntrinsicOp::Movntdq
                        | IntrinsicOp::Movntpd
                ) {
                    continue;
                }
                if allocas.contains(&d.0) {
                    defs.entry(d.0).or_default().push((bi, ii));
                    def_blocks.entry(d.0).or_default().insert(bi);
                }
            }
        }
    }

    for (&slot, sites) in &defs {
        // Global soundness: no deferral if the slot is volatile, copy-aliased,
        // address-taken (value-ref use), or read by a non-Intrinsic instruction
        // (call arg / Store val / Memcpy / terminator) anywhere in the function.
        if volatile_allocas.contains(&slot)
            || copy_alias_roots.contains(&slot)
            || has_value_ref_use.contains(&slot)
            || non_intrinsic_arg_use.contains(&slot)
        {
            continue;
        }
        let mut sites = sites.clone();
        sites.sort();
        // Orphan reads: uses of the slot in a block that never writes it.
        let mut orphan = false;
        if let Some(use_sites) = uses.get(&slot) {
            for &(ubi, _) in use_sites {
                if !def_blocks.get(&slot).map(|b| b.contains(&ubi)).unwrap_or(false) {
                    orphan = true;
                    break;
                }
            }
        }
        if orphan {
            continue; // no store may be deferred for this slot
        }
        // The defer set is keyed by SLOT, and codegen skips the result store
        // at EVERY instruction writing the slot. Deferral is therefore sound
        // only when ALL def sites of the slot satisfy the adjacent-consumer
        // contract. If any single site is non-adjacent (a C temporary reused
        // across two loop bodies where one body runs another vector intrinsic
        // between producer and consumer), the slot must not be deferred at
        // all: the non-adjacent site's consumer would otherwise read a slot
        // that was never written (the adler32_ssse3 miscompile).
        let mut all_sites_ok = true;
        for &(bi, i) in &sites {
            let insts = &func.blocks[bi].instructions;
            // Loop-carried / RMW guard: a use of the slot in this block at or
            // before the def means the block re-enters via a backedge (or the
            // def itself is read-modify-write); the slot must stay updated.
            let earlier_use = uses
                .get(&slot)
                .map(|u| u.iter().any(|&(ubi, uii)| ubi == bi && uii <= i))
                .unwrap_or(false);
            if earlier_use {
                all_sites_ok = false;
                break;
            }
            // Window uses: same block, after i, before the next def of the slot.
            let window_uses: Vec<usize> = uses
                .get(&slot)
                .map(|u| {
                    u.iter()
                        .filter(|&&(ubi, uii)| {
                            ubi == bi
                                && uii > i
                                && !sites.iter().any(|&(db, di)| db == bi && di > i && di < uii)
                        })
                        .map(|&(_, uii)| uii)
                        .collect()
                })
                .unwrap_or_default();
            // The single-entry cache can serve EXACTLY ONE use.
            if window_uses.len() != 1 {
                all_sites_ok = false;
                break;
            }
            let u = window_uses[0];
            // No invalidator and NO intervening intrinsic (any intrinsic may
            // perform a real vector load and evict the single-entry cache)
            // strictly between the def and the use.
            let mut ok = true;
            for k in (i + 1)..u {
                let ik = &insts[k];
                if is_vec_invalidator(ik) || matches!(ik, Instruction::Intrinsic { .. }) {
                    ok = false;
                    break;
                }
            }
            if !ok {
                all_sites_ok = false;
                break;
            }
            // The single use must be a cache-aware consumer at a sound position.
            let (cop, cargs) = match insts.get(u) {
                Some(Instruction::Intrinsic { op, args, .. }) => (op, args),
                _ => {
                    all_sites_ok = false;
                    break;
                }
            };
            let pos = cargs
                .iter()
                .position(|a| matches!(a, Operand::Value(v) if v.0 == slot))
                .unwrap_or(usize::MAX);
            let cache_aware = if pos == 0 {
                !is_raw_reader_intrinsic(cop) && !matches!(cop, IntrinsicOp::Pblendvb128)
            } else {
                is_two_operand_binary(cop)
            };
            if !cache_aware {
                all_sites_ok = false;
                break;
            }
            if std::env::var("CCC_DEBUG_VDEFER").is_ok() {
                eprintln!(
                    "[VDEFER] slot={} def=({},{})(uses_in_window={:?}) consumer=({},{})({:?}) site-ok",
                    slot, bi, i, window_uses, bi, u, cop
                );
            }
        }
        if all_sites_ok {
            result.insert(slot);
        }
    }

    result
}
