//! Phi elimination: lower SSA phi nodes to copies in predecessor blocks.
//!
//! This pass runs after all SSA optimizations and before backend codegen.
//! It converts each Phi instruction into Copy instructions placed at the end
//! of each predecessor block (before the terminator).
//!
//! Smart temporary allocation:
//! When a block has multiple phis, we analyze the copy graph per-predecessor
//! to determine which copies actually need temporaries (only those involved
//! in cycles, e.g., swap patterns) and which can be direct copies. This
//! dramatically reduces the number of temporaries and copy instructions,
//! especially for large switch statements where most phis pass through
//! values unchanged.
//!
//! For non-conflicting phis (the common case), we emit direct copies:
//!   pred_block:
//!     %phi1_dest = copy src1
//!     %phi2_dest = copy src2
//!     `<terminator>`
//!
//! For conflicting phis (cycles), we use shared temporaries and a two-phase
//! copy sequence to avoid the lost-copy problem:
//!   pred_block:
//!     %tmp1 = copy src1  // save source before it's overwritten
//!     `<terminator>`
//!   target_block:
//!     %phi1_dest = copy %tmp1  // restore from temporary
//!     ... rest of block ...
//!
//! Critical edge splitting:
//! When a predecessor block has multiple successors (e.g., a CondBranch) and
//! the target block has phis, placing copies at the end of the predecessor
//! would execute them on ALL outgoing paths, not just the edge to the phi's
//! block. This corrupts values used on other paths. To fix this, we split
//! the critical edge by inserting a new trampoline block that contains only
//! the phi copies and branches unconditionally to the target.
//!
//! Self-loop latch absorption:
//! The critical-edge rule above is over-broad for a *backedge into the block
//! that holds the phis*. Such a block is its own predecessor, so it always has
//! at least two successors (itself plus the loop exits) and the rule always
//! fires, even though the copies it protects belong to the loop's own
//! induction values. The result is a copy-only latch block and one extra
//! unconditional branch per iteration -- precisely the cost loop rotation is
//! meant to remove, reintroduced one pass later.
//!
//! Splitting is only *needed* when a copy destination is observable on one of
//! the block's other outgoing edges. So instead of splitting unconditionally,
//! the backedge copies are buffered and then validated as a parallel copy
//! (`self_loop_copies_can_hoist`). If every destination is invisible outside
//! the block and the copies commute, they are appended to the block itself and
//! the latch disappears. If any destination escapes, the trampoline is built
//! exactly as before, so the transform is strictly a refinement of the old
//! behaviour. Rotation makes this fire in practice: rotating lifts the exit
//! value into a distinct SSA value, so the accumulator the exit phi reads is
//! no longer a phi destination. Set `CCC_NO_PHI_SELFLOOP_HOIST=1` to disable.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IrFunction, IrModule, Operand, Terminator, Value,
};

/// Eliminate all phi nodes in the module by lowering them to copies.
pub fn eliminate_phis(module: &mut IrModule) {
    // Compute the global max block ID across ALL functions to avoid label collisions
    // when creating trampoline blocks. Labels are module-wide (.LBB0, .LBB1, ...).
    let mut next_block_id = 0u32;
    for func in &module.functions {
        for block in &func.blocks {
            if block.label.0 >= next_block_id {
                next_block_id = block.label.0 + 1;
            }
        }
    }
    if std::env::var("LCCC_DEBUG_LABELS").is_ok() {
        eprintln!(
            "[PHI] Starting phi_eliminate with next_block_id = {}",
            next_block_id
        );
    }

    for func in &mut module.functions {
        if func.is_declaration || func.blocks.is_empty() {
            continue;
        }
        if std::env::var("LCCC_DEBUG_LABELS").is_ok() {
            eprintln!(
                "[PHI] Processing function {}, current labels: {:?}",
                func.name,
                func.blocks.iter().map(|b| b.label.0).collect::<Vec<_>>()
            );
        }
        eliminate_phis_in_function(func, &mut next_block_id);
        if std::env::var("LCCC_DEBUG_LABELS").is_ok() {
            eprintln!(
                "[PHI] After processing {}, labels: {:?}, next_block_id now {}",
                func.name,
                func.blocks.iter().map(|b| b.label.0).collect::<Vec<_>>(),
                next_block_id
            );
        }
    }
}

/// Returns the number of distinct successor block IDs for a block.
/// Accounts for both terminator targets and InlineAsm goto_labels.
fn successor_count(block: &BasicBlock) -> usize {
    let mut seen: Vec<BlockId> = Vec::new();
    match &block.terminator {
        Terminator::Return(_) | Terminator::Unreachable => {}
        Terminator::Branch(label) => {
            seen.push(*label);
        }
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            seen.push(*true_label);
            if true_label != false_label {
                seen.push(*false_label);
            }
        }
        Terminator::IndirectBranch {
            possible_targets, ..
        } => {
            seen.extend_from_slice(possible_targets);
        }
        Terminator::Switch { cases, default, .. } => {
            seen.push(*default);
            for &(_, label) in cases {
                if !seen.contains(&label) {
                    seen.push(label);
                }
            }
        }
    }
    // InlineAsm goto_labels are implicit control flow edges.
    for inst in &block.instructions {
        if let Instruction::InlineAsm { goto_labels, .. } = inst {
            for (_, label) in goto_labels {
                if !seen.contains(label) {
                    seen.push(*label);
                }
            }
        }
    }
    seen.len()
}

/// Replace one occurrence of `old_target` with `new_target` in a block's
/// terminator or InlineAsm goto_labels.
fn retarget_block_edge_once(block: &mut BasicBlock, old_target: BlockId, new_target: BlockId) {
    match &mut block.terminator {
        Terminator::Branch(t) => {
            if *t == old_target {
                *t = new_target;
                return;
            }
        }
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            // Only retarget one edge to avoid changing both sides of a diamond
            if *true_label == old_target {
                *true_label = new_target;
                return;
            } else if *false_label == old_target {
                *false_label = new_target;
                return;
            }
        }
        Terminator::IndirectBranch {
            possible_targets, ..
        } => {
            for t in possible_targets.iter_mut() {
                if *t == old_target {
                    *t = new_target;
                    return;
                }
            }
        }
        Terminator::Switch { cases, default, .. } => {
            if *default == old_target {
                *default = new_target;
                return;
            } else {
                for (_, t) in cases.iter_mut() {
                    if *t == old_target {
                        *t = new_target;
                        return;
                    }
                }
            }
        }
        _ => {}
    }
    // Check InlineAsm goto_labels for implicit control flow edges.
    for inst in &mut block.instructions {
        if let Instruction::InlineAsm { goto_labels, .. } = inst {
            for (_, label) in goto_labels.iter_mut() {
                if *label == old_target {
                    *label = new_target;
                    return;
                }
            }
        }
    }
}

struct TrampolineBlock {
    label: BlockId,
    copies: Vec<Instruction>,
    branch_target: BlockId,
    pred_idx: usize,
    old_target: BlockId,
}

/// Get or create a trampoline block for a (pred, target) critical edge.
fn get_or_create_trampoline(
    trampoline_map: &mut FxHashMap<(usize, BlockId), usize>,
    trampolines: &mut Vec<TrampolineBlock>,
    pred_idx: usize,
    target_block_id: BlockId,
    next_block_id: &mut u32,
) -> usize {
    *trampoline_map
        .entry((pred_idx, target_block_id))
        .or_insert_with(|| {
            let idx = trampolines.len();
            let label = BlockId(*next_block_id);
            if std::env::var("LCCC_DEBUG_LABELS").is_ok() {
                eprintln!(
                    "[PHI] Creating trampoline block with BlockId({}), next will be {}",
                    label.0,
                    *next_block_id + 1
                );
            }
            *next_block_id += 1;
            trampolines.push(TrampolineBlock {
                label,
                copies: Vec::new(),
                branch_target: target_block_id,
                pred_idx,
                old_target: target_block_id,
            });
            idx
        })
}

/// Determine which phi copies on a given predecessor edge need temporaries.
///
/// A copy `dest_i = src_i` needs a temporary if `src_i` is the destination
/// of another phi copy (i.e., another phi writes to the value we're reading).
/// This detects interference in the copy graph (e.g., swap patterns: x=y, y=x).
///
/// Returns a set of phi indices that need temporaries.
fn find_conflicting_phis(copies: &[(u32, Option<u32>)]) -> FxHashSet<usize> {
    // Build set of all phi destinations for this edge.
    let dest_set: FxHashSet<u32> = copies.iter().map(|(d, _)| *d).collect();

    // A phi copy needs a temporary if its source is overwritten by another phi
    // destination on this same edge. Specifically, phi i needs a temp if
    // src_i is in dest_set AND src_i != dest_i (self-copies don't conflict).
    let mut needs_temp: FxHashSet<usize> = FxHashSet::default();

    for (i, &(dest_i, src_i_opt)) in copies.iter().enumerate() {
        if let Some(src_i) = src_i_opt {
            if src_i != dest_i && dest_set.contains(&src_i) {
                needs_temp.insert(i);
            }
        }
    }

    // Conservative safety net: also mark phis whose destination is read by a
    // conflicting phi. This ensures the two-phase ordering (temp saves in
    // Pass 1, direct copies in Pass 2) remains correct even for chain
    // patterns like `a = b, b = c, c = a` where multiple values are involved
    // in cycles. This may over-approximate slightly (e.g., marking a phi
    // whose destination is read but already saved by another temp), but
    // correctness is paramount.
    if !needs_temp.is_empty() {
        let conflicting_sources: FxHashSet<u32> =
            needs_temp.iter().filter_map(|&i| copies[i].1).collect();
        for (j, &(dest_j, _)) in copies.iter().enumerate() {
            if conflicting_sources.contains(&dest_j) {
                needs_temp.insert(j);
            }
        }
    }

    needs_temp
}

/// Phi information extracted from a block.
struct PhiInfo {
    dest: Value,
    incoming: Vec<(Operand, BlockId)>,
}

/// Context shared across phi elimination for a single function.
struct PhiElimCtx<'a> {
    label_to_idx: FxHashMap<BlockId, usize>,
    multi_succ: Vec<bool>,
    is_indirect_branch: Vec<bool>,
    pred_copies: FxHashMap<usize, Vec<Instruction>>,
    /// Backedge copies for blocks that are their own phi target, buffered so
    /// they can be validated as a whole before deciding between absorbing them
    /// into the block and splitting the edge. Indexed by block.
    self_loop_copies: Vec<Vec<Instruction>>,
    target_copies: Vec<Vec<Instruction>>,
    trampolines: Vec<TrampolineBlock>,
    trampoline_map: FxHashMap<(usize, BlockId), usize>,
    next_block_id: &'a mut u32,
    next_value: u32,
}

fn eliminate_phis_in_function(func: &mut IrFunction, next_block_id: &mut u32) {
    let mut ctx = PhiElimCtx {
        label_to_idx: func
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.label, i))
            .collect(),
        multi_succ: func.blocks.iter().map(|b| successor_count(b) > 1).collect(),
        is_indirect_branch: func
            .blocks
            .iter()
            .map(|b| matches!(&b.terminator, Terminator::IndirectBranch { .. }))
            .collect(),
        pred_copies: FxHashMap::default(),
        self_loop_copies: vec![Vec::new(); func.blocks.len()],
        target_copies: vec![Vec::new(); func.blocks.len()],
        trampolines: Vec::new(),
        trampoline_map: FxHashMap::default(),
        next_block_id,
        next_value: if func.next_value_id > 0 {
            func.next_value_id
        } else {
            func.max_value_id() + 1
        },
    };

    let block_phis = collect_block_phis(func);

    for (block_idx, phis) in block_phis.iter().enumerate() {
        if phis.is_empty() {
            continue;
        }
        let target_block_id = func.blocks[block_idx].label;
        if phis.len() == 1 {
            emit_single_phi_copies(&phis[0], target_block_id, &mut ctx);
        } else {
            emit_multi_phi_copies(phis, block_idx, target_block_id, &mut ctx);
        }
    }

    resolve_self_loop_copies(func, &mut ctx);
    apply_phi_transformations(func, &mut ctx);
    func.next_value_id = ctx.next_value;
}

/// Collect PhiInfo from all blocks.
fn collect_block_phis(func: &IrFunction) -> Vec<Vec<PhiInfo>> {
    func.blocks
        .iter()
        .map(|block| {
            block
                .instructions
                .iter()
                .filter_map(|inst| {
                    if let Instruction::Phi { dest, incoming, .. } = inst {
                        Some(PhiInfo {
                            dest: *dest,
                            incoming: incoming.clone(),
                        })
                    } else {
                        None
                    }
                })
                .collect()
        })
        .collect()
}

/// Emit copies for a block with a single phi (no temporaries needed).
fn emit_single_phi_copies(phi: &PhiInfo, target_block_id: BlockId, ctx: &mut PhiElimCtx) {
    for (src, pred_label) in &phi.incoming {
        let pred_idx = match ctx.label_to_idx.get(pred_label) {
            Some(&idx) => idx,
            None => continue,
        };
        // Skip self-copies
        if let Operand::Value(v) = src {
            if v.0 == phi.dest.0 {
                continue;
            }
        }
        let copy_inst = Instruction::Copy {
            dest: phi.dest,
            src: *src,
        };
        place_copy(ctx, pred_idx, target_block_id, copy_inst);
    }
}

/// Emit copies for a block with multiple phis, using smart temporary allocation.
/// Shared temporaries are only allocated for phis involved in copy cycles.
fn emit_multi_phi_copies(
    phis: &[PhiInfo],
    block_idx: usize,
    target_block_id: BlockId,
    ctx: &mut PhiElimCtx,
) {
    // Collect unique predecessor labels.
    let mut pred_label_set: FxHashSet<BlockId> = FxHashSet::default();
    let mut pred_labels: Vec<BlockId> = Vec::new();
    for phi in phis {
        for (_, pred_label) in &phi.incoming {
            if pred_label_set.insert(*pred_label) {
                pred_labels.push(*pred_label);
            }
        }
    }

    // Precompute per-phi source lookup tables.
    let phi_src_maps: Vec<FxHashMap<BlockId, &Operand>> = phis
        .iter()
        .map(|phi| phi.incoming.iter().map(|(src, pl)| (*pl, src)).collect())
        .collect();

    // Find which phis are globally conflicting (need temporaries on any edge).
    let globally_needs_temp =
        find_globally_conflicting_phis(phis, &pred_labels, &phi_src_maps, &ctx.label_to_idx);

    // Allocate shared temporaries.
    let mut phi_temps: Vec<Option<Value>> = vec![None; phis.len()];
    for &i in &globally_needs_temp {
        phi_temps[i] = Some(Value(ctx.next_value));
        ctx.next_value += 1;
    }

    // Emit target block copies for conflicting phis (temp -> dest).
    for (i, phi) in phis.iter().enumerate() {
        if let Some(tmp) = phi_temps[i] {
            ctx.target_copies[block_idx].push(Instruction::Copy {
                dest: phi.dest,
                src: Operand::Value(tmp),
            });
        }
    }

    // Emit copies for each predecessor edge.
    for pred_label in &pred_labels {
        let pred_idx = match ctx.label_to_idx.get(pred_label) {
            Some(&idx) => idx,
            None => continue,
        };
        let edge_copies = build_edge_copies(phis, &phi_temps, &phi_src_maps, pred_label);
        if std::env::var_os("CCC_DEBUG_PHIELIM").is_some() {
            eprintln!(
                "[PHIELIM] block {} phis={:?} pred {} copies={:?}",
                target_block_id.0,
                phis.iter().map(|p| p.dest.0).collect::<Vec<_>>(),
                pred_label.0,
                edge_copies
            );
        }
        place_copies(ctx, pred_idx, target_block_id, edge_copies);
    }
}

/// Determine which phi indices are globally conflicting across all predecessor edges.
fn find_globally_conflicting_phis(
    phis: &[PhiInfo],
    pred_labels: &[BlockId],
    phi_src_maps: &[FxHashMap<BlockId, &Operand>],
    label_to_idx: &FxHashMap<BlockId, usize>,
) -> FxHashSet<usize> {
    let mut globally_needs_temp: FxHashSet<usize> = FxHashSet::default();
    for pred_label in pred_labels {
        if !label_to_idx.contains_key(pred_label) {
            continue;
        }
        let copies_info: Vec<(u32, Option<u32>)> = phis
            .iter()
            .enumerate()
            .map(|(i, phi)| {
                let src_val_id = phi_src_maps[i].get(pred_label).and_then(|s| {
                    if let Operand::Value(v) = *s {
                        Some(v.0)
                    } else {
                        None
                    }
                });
                (phi.dest.0, src_val_id)
            })
            .collect();
        for &i in &find_conflicting_phis(&copies_info) {
            globally_needs_temp.insert(i);
        }
    }
    globally_needs_temp
}

/// Build the ordered copy instructions for a single predecessor edge:
/// Pass 1 (temp saves for conflicting phis), then Pass 2 (direct copies).
fn build_edge_copies(
    phis: &[PhiInfo],
    phi_temps: &[Option<Value>],
    phi_src_maps: &[FxHashMap<BlockId, &Operand>],
    pred_label: &BlockId,
) -> Vec<Instruction> {
    let mut copies = Vec::new();

    // Pass 1: Emit temporary saves for conflicting phis (must come first).
    for (i, _phi) in phis.iter().enumerate() {
        if let Some(tmp) = phi_temps[i] {
            if let Some(src) = phi_src_maps[i].get(pred_label) {
                copies.push(Instruction::Copy {
                    dest: tmp,
                    src: *(*src),
                });
            }
        }
    }

    // Pass 2: Emit direct copies for non-conflicting phis.
    for (i, phi) in phis.iter().enumerate() {
        if phi_temps[i].is_none() {
            if let Some(src) = phi_src_maps[i].get(pred_label) {
                if let Operand::Value(v) = *src {
                    if v.0 == phi.dest.0 {
                        continue;
                    } // skip self-copy
                }
                copies.push(Instruction::Copy {
                    dest: phi.dest,
                    src: *(*src),
                });
            }
        }
    }

    copies
}

/// Where copies travelling along the edge `pred_idx -> target_block_id` land.
enum CopyPlacement {
    /// Appended to the predecessor, before its terminator. Safe when the
    /// predecessor has a single successor: the copies cannot run on any path
    /// other than the one to the phi's block.
    Predecessor,
    /// Buffered for self-loop latch absorption. Decided later, once every phi
    /// contributing to the same backedge has been seen.
    SelfLoop,
    /// A freshly split block carrying only these copies.
    Trampoline,
}

fn classify_placement(
    ctx: &PhiElimCtx,
    pred_idx: usize,
    target_block_id: BlockId,
) -> CopyPlacement {
    if !ctx.multi_succ[pred_idx] || ctx.is_indirect_branch[pred_idx] {
        // No critical edge to split: the predecessor reaches the target and
        // nothing else. (An indirect branch is left alone because its target
        // set is not a single edge we can split.)
        return CopyPlacement::Predecessor;
    }
    match ctx.label_to_idx.get(&target_block_id) {
        Some(&target_idx) if target_idx == pred_idx => CopyPlacement::SelfLoop,
        _ => CopyPlacement::Trampoline,
    }
}

/// Place a single copy instruction, using trampolines for critical edges.
fn place_copy(
    ctx: &mut PhiElimCtx,
    pred_idx: usize,
    target_block_id: BlockId,
    copy_inst: Instruction,
) {
    match classify_placement(ctx, pred_idx, target_block_id) {
        CopyPlacement::Predecessor => {
            ctx.pred_copies.entry(pred_idx).or_default().push(copy_inst);
        }
        CopyPlacement::SelfLoop => ctx.self_loop_copies[pred_idx].push(copy_inst),
        CopyPlacement::Trampoline => {
            let tramp_idx = get_or_create_trampoline(
                &mut ctx.trampoline_map,
                &mut ctx.trampolines,
                pred_idx,
                target_block_id,
                ctx.next_block_id,
            );
            ctx.trampolines[tramp_idx].copies.push(copy_inst);
        }
    }
}

/// Place multiple copy instructions, using trampolines for critical edges.
fn place_copies(
    ctx: &mut PhiElimCtx,
    pred_idx: usize,
    target_block_id: BlockId,
    copies: Vec<Instruction>,
) {
    if copies.is_empty() {
        return;
    }
    match classify_placement(ctx, pred_idx, target_block_id) {
        CopyPlacement::Predecessor => {
            ctx.pred_copies.entry(pred_idx).or_default().extend(copies);
        }
        CopyPlacement::SelfLoop => ctx.self_loop_copies[pred_idx].extend(copies),
        CopyPlacement::Trampoline => {
            let tramp_idx = get_or_create_trampoline(
                &mut ctx.trampoline_map,
                &mut ctx.trampolines,
                pred_idx,
                target_block_id,
                ctx.next_block_id,
            );
            ctx.trampolines[tramp_idx].copies.extend(copies);
        }
    }
}

/// Whether self-loop latch absorption is enabled.
fn self_loop_hoist_enabled() -> bool {
    std::env::var("CCC_NO_PHI_SELFLOOP_HOIST").as_deref() != Ok("1")
}

/// Reads performed by each block, split into instruction reads and terminator
/// reads. The split matters because absorbed copies are appended *after* the
/// last instruction but *before* the terminator.
struct BlockUses {
    /// For each value, the blocks whose *instructions* read it.
    ///
    /// A phi's incoming operands are attributed to the block holding the phi
    /// rather than to the predecessor the edge comes from. That is
    /// deliberately conservative: it can only turn a hoist down, never let an
    /// unsound one through, and it avoids modelling per-edge liveness.
    instr_readers: FxHashMap<u32, FxHashSet<usize>>,
    /// For each value, the blocks whose *terminator* reads it.
    ///
    /// Kept separate from `instr_readers` because absorbed copies are appended
    /// after the last instruction but before the terminator, so the two have
    /// different consequences. Terminator reads must not be folded into
    /// `instr_readers`: a value consumed straight from a predecessor by another
    /// block's terminator (typically `Return(v)`) would then look like it never
    /// left the self-loop block.
    term_readers: FxHashMap<u32, FxHashSet<usize>>,
}

impl BlockUses {
    fn from_blocks(blocks: &[BasicBlock]) -> Self {
        let mut instr_readers: FxHashMap<u32, FxHashSet<usize>> = FxHashMap::default();
        let mut term_readers: FxHashMap<u32, FxHashSet<usize>> = FxHashMap::default();
        for (idx, block) in blocks.iter().enumerate() {
            for inst in &block.instructions {
                inst.for_each_used_value(|v| {
                    instr_readers.entry(v).or_default().insert(idx);
                });
            }
            block.terminator.for_each_used_value(|v| {
                term_readers.entry(v).or_default().insert(idx);
            });
        }
        Self {
            instr_readers,
            term_readers,
        }
    }

    /// True when `value` is read only by `block_idx`'s own instructions (or not
    /// at all), i.e. it is invisible to every other block.
    fn only_read_within(&self, value: u32, block_idx: usize) -> bool {
        match self.instr_readers.get(&value) {
            None => true,
            Some(blocks) => blocks.len() == 1 && blocks.contains(&block_idx),
        }
    }

    /// True when no terminator anywhere reads `value`.
    fn no_terminator_reads(&self, value: u32) -> bool {
        !self.term_readers.contains_key(&value)
    }
}

/// Decide, for one self-loop backedge, whether its phi copies can be appended
/// to the block itself instead of being moved into a split latch block.
///
/// The copies run at the end of `block` on *every* path out of it, so they are
/// safe exactly when overwriting their destinations is unobservable everywhere
/// except the backedge. Three conditions are checked, and all three are
/// necessary:
///
/// 1. The backedge is still present in the terminator. Without it the block is
///    not a self-loop and the buffered copies would simply be wrong.
/// 2. No destination is read by any terminator. For the block's own terminator
///    this is an ordering constraint -- the copies are inserted before it, so a
///    terminator reading a destination would branch on the new value. For every
///    other block's terminator it is the same escape condition as (3), and it
///    has to be checked separately because a value can travel straight from
///    this block into another block's `Return` or branch condition without any
///    instruction in between reading it.
/// 3. No destination is read by any other block's instructions. Reads inside the
///    block are fine because they all precede the appended copies; reads
///    elsewhere are reached through a non-backedge edge where the copies also
///    executed.
///
/// Separately, the copies must form a *commuting* parallel copy: no destination
/// may also be a source. Sequentialising a non-commuting set (a swap, or a
/// shift chain) changes its meaning, and the existing temporary machinery in
/// `emit_multi_phi_copies` is what handles those.
///
/// A fourth condition is about *profit*, not correctness. Absorbing appends the
/// copies immediately before the terminator. If the block's branch condition is
/// a `Cmp` in this block and one of the copies redefines an operand of it, the
/// copy lands between that `Cmp` and its consumer, so the x86 emitter's
/// compare-replay must decline (see the redefinition guard in
/// `x86/codegen/comparison.rs`) and emit the compare at its own position rather
/// than fusing it into the branch. That costs an operand reload on every
/// iteration -- more than the removed branch is worth -- so the absorption is
/// skipped in exactly that shape and kept wherever removing the latch pays.
fn self_loop_copies_can_hoist(
    block: &BasicBlock,
    copies: &[Instruction],
    uses: &BlockUses,
    block_idx: usize,
) -> bool {
    if copies.is_empty() {
        return false;
    }

    // Condition 1.
    let self_label = block.label;
    let has_self_edge = match &block.terminator {
        Terminator::Branch(t) => *t == self_label,
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => *true_label == self_label || *false_label == self_label,
        // A switch can carry the backedge too, in a case arm or as the default.
        // Conditions 2 and 3 below are terminator-shape independent, and a
        // destination that no other block reads is unobservable on however many
        // edges leave the block, so a switch needs no special handling beyond
        // finding the self edge.
        Terminator::Switch { cases, default, .. } => {
            *default == self_label || cases.iter().any(|&(_, t)| t == self_label)
        }
        Terminator::Return(_) | Terminator::Unreachable | Terminator::IndirectBranch { .. } => {
            false
        }
    };
    if !has_self_edge {
        return false;
    }

    let mut dests: FxHashSet<u32> = FxHashSet::default();
    let mut srcs: FxHashSet<u32> = FxHashSet::default();
    for copy in copies {
        let Instruction::Copy { dest, src } = copy else {
            return false;
        };
        // A repeated destination means the set is not a parallel copy.
        if !dests.insert(dest.0) {
            return false;
        }
        if let Operand::Value(v) = src {
            srcs.insert(v.0);
        }
    }

    // Commuting check.
    if dests.iter().any(|d| srcs.contains(d)) {
        return false;
    }

    for &dest in &dests {
        // Conditions 2 and 3: the destination must be invisible everywhere
        // except this block's own instructions.
        if !uses.no_terminator_reads(dest) || !uses.only_read_within(dest, block_idx) {
            return false;
        }
    }

    // Condition 4 (profit): keep the branch condition fusable.
    if let Terminator::CondBranch {
        cond: Operand::Value(cond),
        ..
    } = &block.terminator
    {
        for inst in &block.instructions {
            let Instruction::Cmp { dest, lhs, rhs, .. } = inst else {
                continue;
            };
            if dest.0 != cond.0 {
                continue;
            }
            let touched = [lhs, rhs].iter().any(|op| match op {
                Operand::Value(v) => dests.contains(&v.0),
                Operand::Const(_) => false,
            });
            if touched {
                return false;
            }
        }
    }

    true
}

/// Resolve every buffered self-loop backedge: absorb the copies into the block
/// when that is provably safe, otherwise split the edge as before.
fn resolve_self_loop_copies(func: &IrFunction, ctx: &mut PhiElimCtx) {
    if ctx.self_loop_copies.iter().all(Vec::is_empty) {
        return;
    }
    let enabled = self_loop_hoist_enabled();
    let uses = BlockUses::from_blocks(&func.blocks);
    let buffered = std::mem::take(&mut ctx.self_loop_copies);
    let debug = std::env::var("LCCC_DEBUG_PHI_HOIST").is_ok();

    for (block_idx, copies) in buffered.into_iter().enumerate() {
        if copies.is_empty() {
            continue;
        }
        let target_block_id = func.blocks[block_idx].label;
        let hoist = enabled
            && self_loop_copies_can_hoist(&func.blocks[block_idx], &copies, &uses, block_idx);
        if debug {
            let dests: Vec<String> = copies
                .iter()
                .map(|c| match c {
                    Instruction::Copy { dest, src } => format!("v{}={:?}", dest.0, src),
                    _ => "?".to_string(),
                })
                .collect();
            eprintln!(
                "[PHI] self-loop block {}: {} ({} copies: {})",
                target_block_id.0,
                if hoist { "absorb" } else { "split" },
                copies.len(),
                dests.join(", ")
            );
        }
        if hoist {
            ctx.pred_copies.entry(block_idx).or_default().extend(copies);
        } else {
            let tramp_idx = get_or_create_trampoline(
                &mut ctx.trampoline_map,
                &mut ctx.trampolines,
                block_idx,
                target_block_id,
                ctx.next_block_id,
            );
            ctx.trampolines[tramp_idx].copies.extend(copies);
        }
    }
}

/// Apply all phi elimination transformations to the function:
/// remove phis, insert copies, retarget terminators, add trampolines.
fn apply_phi_transformations(func: &mut IrFunction, ctx: &mut PhiElimCtx) {
    for (block_idx, block) in func.blocks.iter_mut().enumerate() {
        // Remove phi instructions (and their spans)
        if !block.source_spans.is_empty() {
            let mut span_idx = 0;
            block.source_spans.retain(|_| {
                let keep = !matches!(
                    block.instructions.get(span_idx),
                    Some(Instruction::Phi { .. })
                );
                span_idx += 1;
                keep
            });
        }
        block
            .instructions
            .retain(|inst| !matches!(inst, Instruction::Phi { .. }));

        // Prepend target copies (these go at the start, replacing the phis)
        if !ctx.target_copies[block_idx].is_empty() {
            let num_copies = ctx.target_copies[block_idx].len();
            let mut new_insts = ctx.target_copies[block_idx].clone();
            new_insts.append(&mut block.instructions);
            block.instructions = new_insts;
            if !block.source_spans.is_empty() {
                let mut new_spans = vec![crate::common::source::Span::dummy(); num_copies];
                new_spans.append(&mut block.source_spans);
                block.source_spans = new_spans;
            }
        }

        // Insert predecessor copies before terminator
        if let Some(copies) = ctx.pred_copies.remove(&block_idx) {
            let num_copies = copies.len();
            block.instructions.extend(copies);
            if !block.source_spans.is_empty() {
                block.source_spans.extend(std::iter::repeat_n(
                    crate::common::source::Span::dummy(),
                    num_copies,
                ));
            }
        }
    }

    // Retarget predecessors that need trampolines
    for trampoline in &ctx.trampolines {
        retarget_block_edge_once(
            &mut func.blocks[trampoline.pred_idx],
            trampoline.old_target,
            trampoline.label,
        );
    }

    // Append trampoline blocks to the function
    for trampoline in std::mem::take(&mut ctx.trampolines) {
        let num_copies = trampoline.copies.len();
        func.blocks.push(BasicBlock {
            label: trampoline.label,
            instructions: trampoline.copies,
            source_spans: vec![crate::common::source::Span::dummy(); num_copies],
            terminator: Terminator::Branch(trampoline.branch_target),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blk(label: u32, insts: Vec<Instruction>, term: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions: insts,
            source_spans: Vec::new(),
            terminator: term,
        }
    }

    fn cp(d: u32, s: u32) -> Instruction {
        Instruction::Copy {
            dest: Value(d),
            src: Operand::Value(Value(s)),
        }
    }

    fn cmp9(lhs: u32) -> Instruction {
        Instruction::Cmp {
            dest: Value(9),
            op: crate::ir::reexports::IrCmpOp::Ne,
            lhs: Operand::Value(Value(lhs)),
            rhs: Operand::Const(crate::ir::reexports::IrConst::I64(0)),
            ty: crate::common::types::IrType::I32,
        }
    }

    fn cond(cond: u32, t: u32, f: u32) -> Terminator {
        Terminator::CondBranch {
            cond: Operand::Value(Value(cond)),
            true_label: BlockId(t),
            false_label: BlockId(f),
        }
    }

    /// The shape latch absorption exists for: a rotated loop whose
    /// next-iteration values are distinct SSA values computed last.
    fn rotated_loop() -> Vec<BasicBlock> {
        vec![
            blk(1, vec![cp(13, 10), cp(15, 11), cp(7, 13)], cond(9, 1, 2)),
            blk(2, vec![cp(8, 7)], Terminator::Return(None)),
        ]
    }

    fn hoists(blocks: &[BasicBlock], block_idx: usize, copies: &[Instruction]) -> bool {
        let uses = BlockUses::from_blocks(blocks);
        self_loop_copies_can_hoist(&blocks[block_idx], copies, &uses, block_idx)
    }

    #[test]
    fn absorbs_block_local_commuting_copies() {
        let blocks = rotated_loop();
        assert!(hoists(&blocks, 0, &[cp(10, 13), cp(11, 15)]));
    }

    #[test]
    fn rejects_when_a_destination_is_read_by_another_block() {
        let mut blocks = rotated_loop();
        // v10 (the induction value) escapes to the exit block.
        blocks[1].instructions.push(cp(8, 10));
        assert!(!hoists(&blocks, 0, &[cp(10, 13), cp(11, 15)]));
    }

    #[test]
    fn rejects_when_the_own_terminator_reads_a_destination() {
        let mut blocks = rotated_loop();
        blocks[0].terminator = cond(10, 1, 2);
        assert!(!hoists(&blocks, 0, &[cp(10, 13), cp(11, 15)]));
    }

    /// A value can flow straight into another block's terminator with no
    /// instruction reading it in between; folding terminator reads into the
    /// instruction-read map hides exactly this and miscompiles.
    #[test]
    fn rejects_when_another_blocks_terminator_reads_a_destination() {
        let mut blocks = rotated_loop();
        blocks[1].terminator = Terminator::Return(Some(Operand::Value(Value(10))));
        assert!(!hoists(&blocks, 0, &[cp(10, 13), cp(11, 15)]));
    }

    #[test]
    fn rejects_non_commuting_copies() {
        let blocks = rotated_loop();
        // A swap: each destination is also a source, so sequentialising changes
        // the meaning.
        assert!(!hoists(&blocks, 0, &[cp(10, 11), cp(11, 10)]));
    }

    /// A destination read that follows its source's definition is still safe to
    /// absorb: the copies are appended after every instruction, so the read
    /// sees the old value. What makes it safe *downstream* is the compare-replay
    /// redefinition guard in `x86/codegen/comparison.rs`, which refuses to
    /// re-emit a compare past a redefinition of its operand. Absorbing here is
    /// what exposed that latent backend bug (ra09_selfop_xor at -O3).
    #[test]
    fn absorbs_when_a_destination_is_read_after_its_source_is_defined() {
        let blocks = vec![
            blk(1, vec![cp(13, 10), cp(7, 10)], cond(9, 1, 2)),
            blk(2, Vec::new(), Terminator::Return(None)),
        ];
        assert!(hoists(&blocks, 0, &[cp(10, 13)]));
    }

    /// Reads at or before the source's own definition are inputs to it, so they
    /// precede the source and cannot overlap its live range.
    #[test]
    fn allows_a_destination_read_by_its_sources_own_definition() {
        let blocks = rotated_loop();
        assert!(hoists(&blocks, 0, &[cp(10, 13)]));
    }

    #[test]
    fn rejects_a_repeated_destination() {
        let blocks = rotated_loop();
        assert!(!hoists(&blocks, 0, &[cp(10, 13), cp(10, 15)]));
    }

    #[test]
    fn rejects_without_a_self_edge() {
        let mut blocks = rotated_loop();
        blocks[0].terminator = Terminator::Branch(BlockId(2));
        assert!(!hoists(&blocks, 0, &[cp(10, 13)]));
    }

    #[test]
    fn absorbs_a_source_defined_outside_the_block() {
        let blocks = rotated_loop();
        // A loop-invariant source is the normal case for a bound or step value;
        // the copies still execute only at the block tail, so the destinations
        // are exactly as observable as before.
        assert!(hoists(&blocks, 0, &[cp(10, 99)]));
    }

    /// Absorbing between a `Cmp` and the branch that consumes it forfeits
    /// compare-replay fusion when a copy redefines one of its operands, which
    /// costs an operand reload per iteration.
    #[test]
    fn rejects_when_a_copy_would_split_a_compare_from_its_branch() {
        let blocks = vec![
            blk(1, vec![cp(13, 10), cmp9(10)], cond(9, 1, 2)),
            blk(2, Vec::new(), Terminator::Return(None)),
        ];
        assert!(!hoists(&blocks, 0, &[cp(10, 13)]));
    }

    /// A compare whose operands the copies do not touch keeps its fusion, so
    /// the absorption still pays.
    #[test]
    fn absorbs_when_the_compare_operands_are_untouched() {
        let blocks = vec![
            blk(1, vec![cp(13, 10), cmp9(13)], cond(9, 1, 2)),
            blk(2, Vec::new(), Terminator::Return(None)),
        ];
        assert!(hoists(&blocks, 0, &[cp(10, 13)]));
    }

    #[test]
    fn absorbs_a_constant_source() {
        use crate::ir::reexports::IrConst;
        let blocks = vec![
            blk(1, vec![cp(7, 10)], cond(9, 1, 2)),
            blk(2, Vec::new(), Terminator::Return(None)),
        ];
        let copies = [Instruction::Copy {
            dest: Value(10),
            src: Operand::Const(IrConst::I32(0)),
        }];
        assert!(hoists(&blocks, 0, &copies));
    }

    #[test]
    fn rejects_an_empty_copy_set() {
        let blocks = rotated_loop();
        assert!(!hoists(&blocks, 0, &[]));
    }

    #[test]
    fn finds_a_self_edge_in_a_switch_arm() {
        let blocks = vec![
            blk(
                1,
                vec![cp(13, 10)],
                Terminator::Switch {
                    val: Operand::Value(Value(9)),
                    cases: vec![(0, BlockId(1)), (1, BlockId(2))],
                    default: BlockId(2),
                    ty: crate::common::types::IrType::I32,
                },
            ),
            blk(2, Vec::new(), Terminator::Return(None)),
        ];
        assert!(hoists(&blocks, 0, &[cp(10, 13)]));
    }

    #[test]
    fn test_no_conflicts_independent_phis() {
        // a = 1, b = 2 — no overlap between dests and sources
        let copies = vec![(10, Some(1)), (20, Some(2))];
        let result = find_conflicting_phis(&copies);
        assert!(
            result.is_empty(),
            "Independent phis should have no conflicts"
        );
    }

    #[test]
    fn test_swap_pattern() {
        // a = b, b = a — classic swap cycle
        let copies = vec![(10, Some(20)), (20, Some(10))];
        let result = find_conflicting_phis(&copies);
        assert!(
            result.contains(&0),
            "First phi in swap should be conflicting"
        );
        assert!(
            result.contains(&1),
            "Second phi in swap should be conflicting"
        );
    }

    #[test]
    fn test_three_way_cycle() {
        // a = b, b = c, c = a — three-way rotation
        let copies = vec![(10, Some(20)), (20, Some(30)), (30, Some(10))];
        let result = find_conflicting_phis(&copies);
        assert_eq!(
            result.len(),
            3,
            "All three phis in a 3-way cycle should be conflicting"
        );
    }

    #[test]
    fn test_mixed_conflicting_and_non_conflicting() {
        // a = b, b = a (conflict), c = 99 (independent)
        let copies = vec![(10, Some(20)), (20, Some(10)), (30, Some(99))];
        let result = find_conflicting_phis(&copies);
        assert!(result.contains(&0));
        assert!(result.contains(&1));
        assert!(
            !result.contains(&2),
            "Independent phi should not be marked conflicting"
        );
    }

    #[test]
    fn test_self_copy_not_conflicting() {
        // a = a — self-copy, not a conflict
        let copies = vec![(10, Some(10)), (20, Some(30))];
        let result = find_conflicting_phis(&copies);
        assert!(result.is_empty(), "Self-copy should not be conflicting");
    }

    #[test]
    fn test_constant_source_not_conflicting() {
        // a = <const>, b = <const> — None sources (constants)
        let copies = vec![(10, None), (20, None)];
        let result = find_conflicting_phis(&copies);
        assert!(
            result.is_empty(),
            "Constant sources should have no conflicts"
        );
    }

    #[test]
    fn test_chain_pattern_conservative() {
        // a = b, b = c — b is dest of phi 1 and source of phi 0
        // phi 0 reads b (which is the dest of phi 1), so phi 0 is directly conflicting.
        // The safety net then marks phi 1 because its dest (b=20) is a source
        // of a conflicting phi.
        let copies = vec![(10, Some(20)), (20, Some(30))];
        let result = find_conflicting_phis(&copies);
        assert!(
            result.contains(&0),
            "Chain phi reading overwritten dest should be conflicting"
        );
        assert!(
            result.contains(&1),
            "Chain phi whose dest is read by conflicting phi should also be marked"
        );
    }
}
