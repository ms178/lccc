//! If-conversion pass: converts simple branch+phi diamonds to Select instructions.
//!
//! This pass identifies diamond-shaped CFG patterns:
//!
//! ```text
//!     pred_block:
//!         ...
//!         condbranch %cond, true_block, false_block
//!
//!     true_block:
//!         (0-1 simple instructions)
//!         branch merge_block
//!
//!     false_block:
//!         (0-1 simple instructions)
//!         branch merge_block
//!
//!     merge_block:
//!         %result = phi [true_val, true_block], [false_val, false_block]
//!         ...
//! ```
//!
//! And converts them to:
//!
//! ```text
//!     pred_block:
//!         ...
//!         %result = select %cond, true_val, false_val
//!         branch merge_block
//! ```
//!
//! This eliminates branches in favor of conditional moves (cmov/csel),
//! which is critical for performance in tight loops with simple conditionals
//! (e.g., `x >= wsize ? x - wsize : 0` in zlib's slide_hash).
//!
//! Safety: Only converts when both arms are side-effect-free (no stores, calls,
//! or memory operations). This ensures the Select semantics (evaluate both
//! operands) match the original branch semantics.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::analysis;
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IrBinOp, IrCmpOp, IrFunction, Operand, Terminator, Value,
};
use crate::passes::loop_analysis;

/// Run if-conversion on a single function.
pub(crate) fn if_convert_function(func: &mut IrFunction) -> usize {
    let num_blocks = func.blocks.len();
    if num_blocks < 3 {
        return 0; // Need at least 3 blocks (pred + one arm + merge)
    }

    let mut total = 0;

    // Iterate to a fixpoint since converting one diamond may expose another.
    // Store sinking runs in the same fixpoint: each round it turns
    // store-in-arm decision trees into phi-driven single stores, which the
    // diamond/triangle converters then fold into Selects.
    loop {
        let sunk = sink_conditional_stores(func);
        let converted = if_convert_once(func);
        if sunk == 0 && converted == 0 {
            break;
        }
        total += converted;
    }

    total
}

/// Canonical address key of a pointer VALUE: `GEP(base, offset)` reduces
/// to (resolved-base, canonical-offset) — the offset chain (Cast(IV),
/// Shl(IV, k), Copy) is traced to its underlying index so two GEPs built
/// from the same IV and base (but with distinct SSA ids — exactly what the
/// conditional-reduction diamond produces per arm) compare equal.
fn canonical_addr_key_impl(
    defs: &FxHashMap<u32, Instruction>,
    copy_of: &FxHashMap<Value, Value>,
    ptr: &Value,
) -> String {
    let resolve = |v: &Value| -> Value {
        let mut cur = *v;
        for _ in 0..64 {
            match copy_of.get(&cur) {
                Some(&next) if next != cur => cur = next,
                _ => break,
            }
        }
        cur
    };
    let root = resolve(ptr);
    match defs.get(&root.0) {
        Some(Instruction::GetElementPtr { base, offset, .. }) => {
            // Base identity: GlobalAddr SSA ids are per-use-site (the
            // frontend emits a fresh one per source reference), so two GEPs
            // over the same global have DIFFERENT base value ids.
            // Canonicalize the base to its symbol when it resolves to a
            // GlobalAddr; everything else keeps the SSA id.
            let base_root = resolve(base);
            let base_key: String = match defs.get(&base_root.0) {
                Some(Instruction::GlobalAddr { name, .. }) => format!("sym({})", name),
                _ => format!("val({})", base_root.0),
            };
            let mut off_root = offset.clone();
            for _ in 0..16 {
                let vid = match &off_root {
                    Operand::Value(v) => v.0,
                    Operand::Const(c) => return format!("gep({}@{:?})", base_key, c),
                };
                let next = match defs.get(&vid) {
                    Some(Instruction::Cast {
                        src: Operand::Value(v),
                        ..
                    })
                    | Some(Instruction::Copy {
                        src: Operand::Value(v),
                        ..
                    })
                    | Some(Instruction::BinOp {
                        op: IrBinOp::Shl,
                        lhs: Operand::Value(v),
                        ..
                    }) => *v,
                    _ => break,
                };
                off_root = Operand::Value(next);
            }
            format!("gep({}@{:?})", base_key, off_root)
        }
        _ => format!("val({})", root.0),
    }
}

/// Rewrite loads in single-predecessor arm blocks that re-load an address the
/// branch predecessor already loads. The arm always executes immediately after
/// the pred on any path that reaches it, so when both blocks are free of
/// memory writes the pred's identical load produces exactly the value the arm
/// load would see; replacing it with a Copy eliminates a redundant memory
/// access and often makes the arm side-effect-free enough for if-conversion
/// (e.g. `if (a[i] > 0) s += a[i]` reloading `a[i]` in the then-arm).
fn rewrite_covered_arm_loads(
    func: &mut IrFunction,
    label_to_idx: &FxHashMap<BlockId, usize>,
    preds: &analysis::FlatAdj,
) -> usize {
    /// Free of memory writes and other externally visible effects.
    /// Loads are fine (they don't change memory). Conservative: intrinsics,
    /// calls, stores, atomics, allocas are all treated as barriers.
    fn is_pure(inst: &Instruction) -> bool {
        matches!(
            inst,
            Instruction::Load { .. }
                | Instruction::BinOp { .. }
                | Instruction::UnaryOp { .. }
                | Instruction::Cmp { .. }
                | Instruction::Cast { .. }
                | Instruction::Copy { .. }
                | Instruction::GetElementPtr { .. }
                | Instruction::GlobalAddr { .. }
                | Instruction::Select { .. }
                | Instruction::Phi { .. }
        )
    }

    let mut rewrites = 0;
    let debug = std::env::var("CCC_DEBUG_IFCONV").is_ok();

    // Resolve pointer copies to their root value: `p = q; load [p]` loads the
    // same address as `load [q]`. SSA guarantees the root dominates both uses.
    let mut copy_of: FxHashMap<Value, Value> = FxHashMap::default();
    for b in &func.blocks {
        for i in &b.instructions {
            if let Instruction::Copy {
                dest,
                src: Operand::Value(src),
            } = i
            {
                copy_of.insert(*dest, *src);
            }
        }
    }
    let resolve = |v: &Value| -> Value {
        let mut cur = *v;
        for _ in 0..64 {
            match copy_of.get(&cur) {
                Some(&next) if next != cur => cur = next,
                _ => break,
            }
        }
        cur
    };

    // Cloned defs: the rewrite loop below mutates func.blocks, so the map
    // must not hold borrows into it.
    let defs: FxHashMap<u32, Instruction> = func
        .blocks
        .iter()
        .flat_map(|b| b.instructions.iter())
        .filter_map(|i| i.dest().map(|d| (d.0, i.clone())))
        .collect();
    let canonical_addr_key = |ptr: &Value| -> String {
        canonical_addr_key_impl(&defs, &copy_of, ptr)
    };

    for pred_idx in 0..func.blocks.len() {
        let (true_label, false_label) = match &func.blocks[pred_idx].terminator {
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => (*true_label, *false_label),
            _ => continue,
        };
        // The pred must not write memory between its load and the arm.
        if !func.blocks[pred_idx].instructions.iter().all(is_pure) {
            continue;
        }
        // Collect the pred's loads: (ptr, ty, seg) -> dest. Linear-scan Vec;
        // preds hold only a handful of loads.
        let mut pred_loads: Vec<(String, IrType, AddressSpace, Value)> = Vec::new();
        for inst in &func.blocks[pred_idx].instructions {
            if let Instruction::Load {
                dest,
                ptr,
                ty,
                seg_override,
                ..
            } = inst
            {
                let key = canonical_addr_key(ptr);
                // On a duplicate, keep the last load (same value either way).
                if let Some(entry) = pred_loads
                    .iter_mut()
                    .find(|(p, t, s, _)| *p == key && *t == *ty && *s == *seg_override)
                {
                    entry.3 = *dest;
                } else {
                    pred_loads.push((key, *ty, *seg_override, *dest));
                }
            }
        }
        if pred_loads.is_empty() {
            continue;
        }
        for label in [true_label, false_label] {
            let Some(&arm_idx) = label_to_idx.get(&label) else {
                continue;
            };
            if arm_idx == pred_idx {
                continue;
            }
            // The arm must be reached only from this pred, and must not write
            // memory before (or after) its loads.
            if preds.len(arm_idx) != 1 || preds.row(arm_idx)[0] as usize != pred_idx {
                continue;
            }
            if !func.blocks[arm_idx].instructions.iter().all(is_pure) {
                continue;
            }
            // Collect rewrites first (immutable), then apply.
            let mut pending: Vec<(usize, Value, Value)> = Vec::new();
            for (inst_pos, inst) in func.blocks[arm_idx].instructions.iter().enumerate() {
                if let Instruction::Load {
                    dest,
                    ptr,
                    ty,
                    seg_override,
                    ..
                } = inst
                {
                    let key = canonical_addr_key(ptr);
                    if let Some((_, _, _, covering)) = pred_loads
                        .iter()
                        .find(|(p, t, s, _)| *p == key && *t == *ty && *s == *seg_override)
                    {
                        pending.push((inst_pos, *dest, *covering));
                    } else if debug {
                        eprintln!(
                            "[IFCONV] arm block {} load ptr={} ty={:?} not covered by pred loads {:?}",
                            arm_idx,
                            ptr.0,
                            ty,
                            pred_loads
                                .iter()
                                .map(|(p, t, _, d)| (p.as_str(), *t, d.0))
                                .collect::<Vec<_>>()
                        );
                        for b in &func.blocks {
                            for i in &b.instructions {
                                if i.dest() == Some(*ptr)
                                    || pred_loads.iter().any(|(_, _, _, d)| i.dest() == Some(*d))
                                {
                                    eprintln!("[IFCONV]   def: {:?}", i);
                                }
                            }
                        }
                    }
                }
            }
            for (inst_pos, dest, covering) in pending {
                func.blocks[arm_idx].instructions[inst_pos] = Instruction::Copy {
                    dest,
                    src: Operand::Value(covering),
                };
                rewrites += 1;
            }
        }
    }
    rewrites
}

/// Sink per-predecessor conditional stores to one phi-driven store in the
/// common successor.  When EVERY predecessor of a block M ends with a plain
/// store to the same canonical address (allowing distinct SSA ids for the
/// per-path GEPs), exactly one of those stores executes per visit of M —
/// replace them with a phi of the stored values plus a single store at the
/// top of M.
///
/// This unblocks diamond if-conversion for the store-arm form
/// (`if (c) d[i]=x; else d[i]=y;` — the classic clamp/branchy-store loop
/// shape): the phi turns the store into dataflow the diamond converter
/// turns into Selects.  The rewrite is value-exact and order-preserving in
/// every context (no loop precondition needed): the sunk store executes at
/// the same visit with the same value and address as before, after all of
/// M's phis and before any of M's other instructions, and every other
/// write in a pred still precedes it.
fn sink_conditional_stores(func: &mut IrFunction) -> usize {
    let num_blocks = func.blocks.len();
    if num_blocks < 3 {
        return 0;
    }
    let label_to_idx = analysis::build_label_map(func);
    let (preds, _succs) = analysis::build_cfg(func, &label_to_idx);
    let defs: FxHashMap<u32, Instruction> = func
        .blocks
        .iter()
        .flat_map(|b| b.instructions.iter())
        .filter_map(|i| i.dest().map(|d| (d.0, i.clone())))
        .collect();
    let mut copy_of = FxHashMap::default();
    for b in &func.blocks {
        for i in &b.instructions {
            if let Instruction::Copy {
                dest,
                src: Operand::Value(src),
            } = i
            {
                copy_of.insert(*dest, *src);
            }
        }
    }

    struct SinkCand {
        pred_idx: usize,
        ty: IrType,
        key: String,
        val: Operand,
        ptr: Value,
    }

    let mut sunk = 0;
    for merge_idx in 0..num_blocks {
        let pred_list: Vec<usize> = (0..num_blocks)
            .filter(|&p| preds.row(merge_idx).iter().any(|&q| q as usize == p))
            .collect();
        if pred_list.len() < 2 || pred_list.contains(&merge_idx) {
            continue;
        }
        let merge_label = func.blocks[merge_idx].label;
        let mut cands: Vec<SinkCand> = Vec::new();
        let mut ok = true;
        for &p in &pred_list {
            let block = &func.blocks[p];
            if !matches!(block.terminator, Terminator::Branch(l) if l == merge_label) {
                ok = false;
                break;
            }
            let Some(Instruction::Store {
                val,
                ptr,
                ty,
                volatile,
                seg_override,
            }) = block.instructions.last()
            else {
                ok = false;
                break;
            };
            if *volatile || *seg_override != AddressSpace::Default {
                ok = false;
                break;
            }
            cands.push(SinkCand {
                pred_idx: p,
                ty: *ty,
                key: canonical_addr_key_impl(&defs, &copy_of, ptr),
                val: *val,
                ptr: *ptr,
            });
        }
        if !ok {
            continue;
        }
        let ty = cands[0].ty;
        if !cands
            .iter()
            .all(|c| c.ty == ty && c.key == cands[0].key)
        {
            continue;
        }
        // Rebuild ONE address chain inside M (fresh dests): the store GEP
        // from the first candidate, cloned through Copy/Cast/Shl/Add/Mul
        // nodes so every SSA def dominates M.  Chain leaves (parameters,
        // globals, IV phis, constants) dominate M by construction; a leaf
        // that is any other instruction makes the sink bail (fail closed).
        let Some((gep_val, chain_insts)) =
            clone_addr_chain_into_merge(&defs, &copy_of, cands[0].ptr, func)
        else {
            continue;
        };

        // Phi + GEP chain + store at the top of M (after M's existing
        // phis), in that order: the store uses the chain's fresh GEP.
        let phi_dest = Value(func.next_value_id);
        func.next_value_id += 1;
        let incoming: Vec<(Operand, BlockId)> = cands
            .iter()
            .map(|c| (c.val, func.blocks[c.pred_idx].label))
            .collect();
        let has_spans = !func.blocks[merge_idx].source_spans.is_empty();
        let pos = func.blocks[merge_idx]
            .instructions
            .iter()
            .position(|i| !matches!(i, Instruction::Phi { .. }))
            .unwrap_or(func.blocks[merge_idx].instructions.len());
        let mut insert_at = pos;
        func.blocks[merge_idx]
            .instructions
            .insert(insert_at, Instruction::Phi { dest: phi_dest, ty, incoming });
        insert_at += 1;
        let chain_len = chain_insts.len();
        for inst in chain_insts {
            func.blocks[merge_idx].instructions.insert(insert_at, inst);
            insert_at += 1;
        }
        func.blocks[merge_idx].instructions.insert(
            insert_at,
            Instruction::Store {
                val: Operand::Value(phi_dest),
                ptr: gep_val,
                ty,
                volatile: false,
                seg_override: AddressSpace::Default,
            },
        );
        if has_spans {
            for _ in 0..(1 + chain_len + 1) {
                func.blocks[merge_idx]
                    .source_spans
                    .insert(pos, crate::common::source::Span::dummy());
            }
        }
        // Drop the per-pred stores (and their spans).
        for c in &cands {
            func.blocks[c.pred_idx].instructions.pop();
            if !func.blocks[c.pred_idx].source_spans.is_empty() {
                func.blocks[c.pred_idx].source_spans.pop();
            }
        }
        sunk += cands.len();
    }
    sunk
}

/// Clone an address computation into the merge block with fresh SSA dests.
/// Walks Copy/Cast/Shl/Add/Mul chains from `ptr` down to dominating leaves;
/// bails when any link is outside that grammar.
fn clone_addr_chain_into_merge(
    defs: &FxHashMap<u32, Instruction>,
    copy_of: &FxHashMap<Value, Value>,
    ptr: Value,
    func: &mut IrFunction,
) -> Option<(Value, Vec<Instruction>)> {
    fn resolve(copy_of: &FxHashMap<Value, Value>, v: &Value) -> Value {
        let mut cur = *v;
        for _ in 0..64 {
            match copy_of.get(&cur) {
                Some(&next) if next != cur => cur = next,
                _ => break,
            }
        }
        cur
    }

    let mut emitted: Vec<Instruction> = Vec::new();
    let mut memo: FxHashMap<Value, Value> = FxHashMap::default();
    let mut next_val = func.next_value_id;

    fn clone_operand(
        operand: &Operand,
        defs: &FxHashMap<u32, Instruction>,
        copy_of: &FxHashMap<Value, Value>,
        memo: &mut FxHashMap<Value, Value>,
        emitted: &mut Vec<Instruction>,
        next_val: &mut u32,
    ) -> Option<Operand> {
        match operand {
            Operand::Const(c) => Some(Operand::Const(*c)),
            Operand::Value(v) => Some(Operand::Value(clone_value(
                *v, defs, copy_of, memo, emitted, next_val,
            )?)),
        }
    }

    fn clone_value(
        v: Value,
        defs: &FxHashMap<u32, Instruction>,
        copy_of: &FxHashMap<Value, Value>,
        memo: &mut FxHashMap<Value, Value>,
        emitted: &mut Vec<Instruction>,
        next_val: &mut u32,
    ) -> Option<Value> {
        if let Some(&nv) = memo.get(&v) {
            return Some(nv);
        }
        let resolved = resolve(copy_of, &v);
        let Some(def) = defs.get(&resolved.0) else {
            // No def: a parameter or other block-boundary value — dominates
            // the merge block.
            memo.insert(v, resolved);
            return Some(resolved);
        };
        let nv = Value(*next_val);
        match def {
            Instruction::Copy { src, .. } => {
                let ns = clone_operand(src, defs, copy_of, memo, emitted, next_val)?;
                *next_val += 1;
                emitted.push(Instruction::Copy { dest: nv, src: ns });
            }
            Instruction::Cast {
                src, to_ty, from_ty, ..
            } => {
                let ns = clone_operand(src, defs, copy_of, memo, emitted, next_val)?;
                *next_val += 1;
                emitted.push(Instruction::Cast {
                    dest: nv,
                    src: ns,
                    to_ty: *to_ty,
                    from_ty: *from_ty,
                });
            }
            Instruction::BinOp {
                op: op @ (IrBinOp::Shl | IrBinOp::Add | IrBinOp::Mul),
                lhs,
                rhs,
                ty,
                ..
            } => {
                let nl = clone_operand(lhs, defs, copy_of, memo, emitted, next_val)?;
                let nr = clone_operand(rhs, defs, copy_of, memo, emitted, next_val)?;
                *next_val += 1;
                emitted.push(Instruction::BinOp { dest: nv, op: *op, lhs: nl, rhs: nr, ty: *ty });
            }
            Instruction::Phi { .. } | Instruction::GlobalAddr { .. } | Instruction::ParamRef { .. } => {
                // Dominating leaves: keep the original value.
                memo.insert(v, resolved);
                return Some(resolved);
            }
            _ => return None,
        }
        memo.insert(v, nv);
        Some(nv)
    }

    let resolved = resolve(copy_of, &ptr);
    let Some(Instruction::GetElementPtr {
        base,
        offset,
        ty,
        dest: _,
    }) = defs.get(&resolved.0)
    else {
        return None;
    };
    let nb = clone_value(*base, defs, copy_of, &mut memo, &mut emitted, &mut next_val)?;
    let noff = clone_operand(offset, defs, copy_of, &mut memo, &mut emitted, &mut next_val)?;
    let gep_val = Value(next_val);
    next_val += 1;
    emitted.push(Instruction::GetElementPtr {
        dest: gep_val,
        base: nb,
        offset: noff,
        ty: *ty,
    });
    func.next_value_id = next_val;
    Some((gep_val, emitted))
}

/// Single pass of if-conversion. Returns number of diamonds converted.
fn if_convert_once(func: &mut IrFunction) -> usize {
    let num_blocks = func.blocks.len();
    if num_blocks < 3 {
        return 0;
    }

    // Build CFG
    let label_to_idx = analysis::build_label_map(func);
    let (preds, _succs) = analysis::build_cfg(func, &label_to_idx);

    // Eliminate arm-block reloads of addresses the branch pred already loads;
    // this both removes redundant memory traffic and unblocks diamond/triangle
    // detection below (loads make arms fail the side-effect-free check).
    let rewrites = rewrite_covered_arm_loads(func, &label_to_idx, &preds);

    // Detector context: natural loops (for the speculative arm-load gate) and
    // the def/copy maps.  Built AFTER the covered-load rewrites so the SSA
    // defs are the ones the detectors will actually see.
    let ctx = IfConvCtx::build(func);

    // Collect diamond candidates
    let mut diamonds: Vec<DiamondInfo> = Vec::new();

    for pred_idx in 0..num_blocks {
        if let Some(diamond) = detect_diamond(&ctx, pred_idx) {
            diamonds.push(diamond);
        } else if let Some(triangle) = detect_triangle(&ctx, pred_idx) {
            diamonds.push(triangle);
        }
    }

    if diamonds.is_empty() {
        return rewrites;
    }

    // Apply conversions. Track modified blocks to avoid applying overlapping diamonds
    // (e.g., nested ternaries where converting one invalidates another).
    let mut converted = 0;
    let mut modified_blocks: crate::common::fx_hash::FxHashSet<usize> =
        crate::common::fx_hash::FxHashSet::default();
    for diamond in &diamonds {
        // Skip if any of the diamond's blocks were already modified
        if modified_blocks.contains(&diamond.pred_idx)
            || modified_blocks.contains(&diamond.true_idx)
            || modified_blocks.contains(&diamond.false_idx)
            || modified_blocks.contains(&diamond.merge_idx)
        {
            continue;
        }
        if apply_diamond(func, diamond) {
            modified_blocks.insert(diamond.pred_idx);
            modified_blocks.insert(diamond.true_idx);
            modified_blocks.insert(diamond.false_idx);
            modified_blocks.insert(diamond.merge_idx);
            converted += 1;
        }
    }

    // Clean up: run a quick dead block pass to remove the now-empty arm blocks
    // (they'll have no instructions and just an unconditional branch)
    // This is handled by the CFG simplification pass that runs after us.

    converted + rewrites
}

/// Information about a detected diamond pattern.
struct DiamondInfo {
    /// The block containing the CondBranch
    pred_idx: usize,
    /// The true-branch block index
    true_idx: usize,
    /// The false-branch block index
    false_idx: usize,
    /// The merge block index (with the Phi)
    merge_idx: usize,
    /// The condition operand from the CondBranch
    cond: Operand,
    /// Instructions to hoist from the true arm (before the Select)
    true_arm_insts: Vec<Instruction>,
    /// Instructions to hoist from the false arm (before the Select)
    false_arm_insts: Vec<Instruction>,
    /// Phi nodes in the merge block that can be converted to Select.
    /// Each entry is (phi_dest, phi_ty, true_val, false_val).
    phi_selects: Vec<(Value, IrType, Operand, Operand)>,
    /// True when the merge block has exactly the two diamond edges as
    /// predecessors (the converted phis are removed entirely); false for a
    /// partial conversion (converted phis keep their other incoming edges,
    /// fed by a fresh Select destination).
    full_merge: bool,
}

/// Check if a block contains only simple, side-effect-free instructions
/// that are safe to speculatively execute.
///
/// IMPORTANT: Load is NOT included here because loads can trap (segfault)
/// on invalid pointers. Hoisting a load past a null-pointer guard would
/// cause a crash. For example, `if (!p || !p[0])` has a diamond where
/// one arm loads `*p` — if-converting this would execute the load
/// unconditionally, crashing when `p` is NULL.
fn is_side_effect_free(block: &BasicBlock) -> bool {
    for inst in &block.instructions {
        match inst {
            // Division and remainder can trap with SIGFPE on divide-by-zero.
            // They must not be speculatively executed past a guard condition.
            Instruction::BinOp { op, .. } if op.can_trap() => return false,
            Instruction::BinOp { .. }
            | Instruction::UnaryOp { .. }
            | Instruction::Cmp { .. }
            | Instruction::Cast { .. }
            | Instruction::Copy { .. }
            | Instruction::GetElementPtr { .. }
            | Instruction::GlobalAddr { .. }
            | Instruction::Select { .. } => {}
            // Load can trap on invalid pointers — not safe to speculate.
            // Store, Call, atomics, etc. have write side effects.
            _ => return false,
        }
    }
    true
}

/// Analysis context shared by the diamond/triangle detectors for one pass
/// over the function: the CFG bundle (with dominators), the natural loops,
/// and the SSA def/copy maps for address canonicalization.
struct IfConvCtx<'a> {
    func: &'a IrFunction,
    label_to_idx: FxHashMap<BlockId, usize>,
    preds: analysis::FlatAdj,
    loops: Vec<loop_analysis::NaturalLoop>,
    /// Dest -> defining instruction (cloned; stable across the pass).
    defs: FxHashMap<u32, Instruction>,
    /// Copy chains dest -> src.
    copy_of: FxHashMap<Value, Value>,
}

impl<'a> IfConvCtx<'a> {
    fn build(func: &'a IrFunction) -> Self {
        let num_blocks = func.blocks.len();
        let label_to_idx = analysis::build_label_map(func);
        let cfg = analysis::CfgAnalysis::build(func);
        let loops =
            loop_analysis::find_natural_loops(num_blocks, &cfg.preds, &cfg.succs, &cfg.idom);
        let defs = func
            .blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .filter_map(|i| i.dest().map(|d| (d.0, i.clone())))
            .collect();
        let mut copy_of = FxHashMap::default();
        for b in &func.blocks {
            for i in &b.instructions {
                if let Instruction::Copy {
                    dest,
                    src: Operand::Value(src),
                } = i
                {
                    copy_of.insert(*dest, *src);
                }
            }
        }
        IfConvCtx {
            func,
            label_to_idx,
            preds: cfg.preds,
            loops,
            defs,
            copy_of,
        }
    }

    /// Follow copy chains to their root value.
    fn resolve(&self, v: &Value) -> Value {
        let mut cur = *v;
        for _ in 0..64 {
            match self.copy_of.get(&cur) {
                Some(&next) if next != cur => cur = next,
                _ => break,
            }
        }
        cur
    }
}

/// Whether a load in an arm block may be executed unconditionally (hoisted
/// into the branch pred).  Loads trap on invalid addresses, so the general
/// answer is NO — but a load inside a natural loop whose address is that
/// loop's own induction variable scaled against a loop-invariant base is the
/// exact shape every vectorizing compiler predicates (GCC/Clang make these
/// loads unconditional in the vector loop): the address is walked by the
/// loop itself, the converted form executes at the same iterations, and a
/// zero-trip loop executes neither form.  The load never moves out of the
/// loop (its address is IV-derived, so LICM cannot hoist it), and the only
/// behavioral difference is a trap on an iteration where the scalar form
/// took the other arm — an address in the same range the loop itself walks.
fn speculative_load_ok(ctx: &IfConvCtx<'_>, block_idx: usize, ptr: Value) -> bool {
    let root = ctx.resolve(&ptr);
    let Some(Instruction::GetElementPtr { base, offset, .. }) = ctx.defs.get(&root.0) else {
        return false;
    };
    // The innermost natural loop containing the arm block (nested loops:
    // the innermost one owns the induction variable).
    let mut containing: Option<&loop_analysis::NaturalLoop> = None;
    for l in &ctx.loops {
        if l.contains(block_idx) {
            let better = match containing {
                Some(cur) => l.body.len() < cur.body.len(),
                None => true,
            };
            if better {
                containing = Some(l);
            }
        }
    }
    let Some(loop_) = containing else {
        return false;
    };
    // Base must be loop-invariant: defined outside the loop body
    // (parameters, globals, and allocas all qualify).
    let base_root = ctx.resolve(base);
    let base_def_in_loop = ctx.func.blocks.iter().enumerate().any(|(bi, b)| {
        loop_.body.contains(&bi) && b.instructions.iter().any(|i| i.dest() == Some(base_root))
    });
    if base_def_in_loop {
        return false;
    }
    // Offset must be derived from the loop's induction variable (or be a
    // constant): fixed point from the header phis through Cast/Copy/Shl/
    // Add/Mul inside the loop body.
    let mut iv_derived = FxHashSet::default();
    for inst in &ctx.func.blocks[loop_.header].instructions {
        if let Instruction::Phi { dest, ty, .. } = inst {
            if matches!(*ty, IrType::I32 | IrType::U32 | IrType::I64 | IrType::U64) {
                iv_derived.insert(*dest);
            }
        }
    }
    if iv_derived.is_empty() {
        return false;
    }
    let mut changed = true;
    while changed {
        changed = false;
        for &bi in &loop_.body {
            for inst in &ctx.func.blocks[bi].instructions {
                let derived = match inst {
                    Instruction::Cast { dest, src, .. } | Instruction::Copy { dest, src } => {
                        matches!(src, Operand::Value(v) if iv_derived.contains(v))
                            .then_some(*dest)
                    }
                    Instruction::BinOp {
                        dest,
                        op: IrBinOp::Shl | IrBinOp::Add | IrBinOp::Mul,
                        lhs,
                        rhs,
                        ..
                    } => {
                        let l = matches!(lhs, Operand::Value(v) if iv_derived.contains(v));
                        let r = matches!(rhs, Operand::Value(v) if iv_derived.contains(v));
                        (l || r).then_some(*dest)
                    }
                    _ => None,
                };
                if let Some(d) = derived {
                    if iv_derived.insert(d) {
                        changed = true;
                    }
                }
            }
        }
    }
    match offset {
        Operand::Const(_) => true,
        Operand::Value(v) => iv_derived.contains(&ctx.resolve(v)),
    }
}

/// Arm-block convertibility with the speculative-load extension: the pure
/// whitelist above plus loads whose address passes `speculative_load_ok`.
fn arm_is_speculatable(ctx: &IfConvCtx<'_>, block: &BasicBlock, block_idx: usize) -> bool {
    for inst in &block.instructions {
        match inst {
            Instruction::Load {
                ptr,
                volatile,
                seg_override,
                ..
            } => {
                if *volatile || *seg_override != AddressSpace::Default {
                    return false;
                }
                if !speculative_load_ok(ctx, block_idx, *ptr) {
                    return false;
                }
            }
            Instruction::BinOp { op, .. } if op.can_trap() => return false,
            Instruction::BinOp { .. }
            | Instruction::UnaryOp { .. }
            | Instruction::Cmp { .. }
            | Instruction::Cast { .. }
            | Instruction::Copy { .. }
            | Instruction::GetElementPtr { .. }
            | Instruction::GlobalAddr { .. }
            | Instruction::Select { .. } => {}
            _ => return false,
        }
    }
    true
}

/// Check if a condition operand is a known constant or can be trivially resolved
/// to a constant within the block. When the condition is constant, the branch
/// should be folded by cfg_simplify rather than converted to a Select by
/// if_convert. Converting a constant-condition branch to Select delays dead
/// code elimination: the Select needs additional optimization iterations
/// (simplify + constfold + cfg_simplify) to fold away, and if the diminishing-
/// returns heuristic terminates the optimization loop early, dead code paths
/// (e.g., kernel's conditional calls to restore_tpidr2_context guarded by
/// system_supports_sme() which returns false) survive to the final output.
fn is_constant_condition(block: &BasicBlock, cond: &Operand) -> bool {
    match cond {
        Operand::Const(_) => true,
        Operand::Value(v) => {
            // Check if the value is defined as a constant Copy, or as a Cmp/Select
            // where all operands are constants, within the same block.
            for inst in &block.instructions {
                match inst {
                    Instruction::Copy {
                        dest,
                        src: Operand::Const(_),
                    } if *dest == *v => {
                        return true;
                    }
                    Instruction::Cmp { dest, lhs, rhs, .. } if *dest == *v => {
                        let lhs_const =
                            matches!(lhs, Operand::Const(_)) || is_value_const_in_block(block, lhs);
                        let rhs_const =
                            matches!(rhs, Operand::Const(_)) || is_value_const_in_block(block, rhs);
                        if lhs_const && rhs_const {
                            return true;
                        }
                    }
                    Instruction::Select {
                        dest,
                        true_val,
                        false_val,
                        ..
                    } if *dest == *v => {
                        // Select(cond, x, x) where both arms are the same constant
                        if same_value_or_both_zero(true_val, false_val) {
                            return true;
                        }
                    }
                    _ => {}
                }
            }
            false
        }
    }
}

/// Check if an operand is a constant within the block (either directly or via Copy).
fn is_value_const_in_block(block: &BasicBlock, op: &Operand) -> bool {
    match op {
        Operand::Const(_) => true,
        Operand::Value(v) => {
            for inst in &block.instructions {
                if let Instruction::Copy {
                    dest,
                    src: Operand::Const(_),
                } = inst
                {
                    if *dest == *v {
                        return true;
                    }
                }
            }
            false
        }
    }
}

/// Check if two operands are the same value or both integer-zero constants.
/// Used to detect Select(cond, x, x) patterns where the result is the same
/// regardless of the condition, making the condition effectively constant.
fn same_value_or_both_zero(a: &Operand, b: &Operand) -> bool {
    match (a, b) {
        (Operand::Const(ca), Operand::Const(cb)) => {
            ca.to_i64() == Some(0) && cb.to_i64() == Some(0)
        }
        (Operand::Value(va), Operand::Value(vb)) => va.0 == vb.0,
        _ => false,
    }
}

/// Whether `v` is consumed only by control flow (never materialized as data),
/// so the branchy form is at least as good as a cmov Select. Recursively
/// follows boolean-preserving edges: `Cmp(Eq/Ne, 0/1)`, `Select` conditions,
/// `Copy`, `Cast`, and `Phi` incoming values.
fn value_only_controls_branch(func: &IrFunction, v: Value, visited: &mut FxHashSet<u32>) -> bool {
    if !visited.insert(v.0) {
        return true; // cycle: nothing data-like seen yet
    }
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Cmp {
                    dest, op, lhs, rhs, ..
                } => {
                    if matches!(lhs, Operand::Value(x) if x.0 == v.0)
                        || matches!(rhs, Operand::Value(x) if x.0 == v.0)
                    {
                        // Only a boolean negation (v == 0 / v != 0 / v == 1 / v != 1)
                        // preserves branch-likeness; any other comparison is data.
                        let other = if matches!(lhs, Operand::Value(x) if x.0 == v.0) {
                            rhs
                        } else {
                            lhs
                        };
                        let is_bool_cmp = matches!(op, IrCmpOp::Eq | IrCmpOp::Ne)
                            && matches!(other, Operand::Const(c) if c.to_i64() == Some(0) || c.to_i64() == Some(1));
                        if !is_bool_cmp || !value_only_controls_branch(func, *dest, visited) {
                            return false;
                        }
                    }
                }
                Instruction::Select {
                    dest,
                    cond,
                    true_val,
                    false_val,
                    ..
                } => {
                    if matches!(cond, Operand::Value(x) if x.0 == v.0) {
                        if !value_only_controls_branch(func, *dest, visited) {
                            return false;
                        }
                    } else if matches!(true_val, Operand::Value(x) if x.0 == v.0)
                        || matches!(false_val, Operand::Value(x) if x.0 == v.0)
                    {
                        return false; // selected as a data value
                    }
                }
                Instruction::Copy { dest, src, .. } => {
                    if matches!(src, Operand::Value(x) if x.0 == v.0)
                        && !value_only_controls_branch(func, *dest, visited)
                    {
                        return false;
                    }
                }
                Instruction::Cast { dest, src, .. } => {
                    if matches!(src, Operand::Value(x) if x.0 == v.0)
                        && !value_only_controls_branch(func, *dest, visited)
                    {
                        return false;
                    }
                }
                Instruction::Phi { dest, incoming, .. } => {
                    for (op, _) in incoming {
                        if matches!(op, Operand::Value(x) if x.0 == v.0)
                            && !value_only_controls_branch(func, *dest, visited)
                        {
                            return false;
                        }
                    }
                }
                other => {
                    let mut used = false;
                    other.for_each_used_value(|u| {
                        if u == v.0 {
                            used = true;
                        }
                    });
                    if used {
                        return false; // data use (binop operand, store, call arg, ...)
                    }
                }
            }
        }
        match &block.terminator {
            // A branch condition is control flow — fine.
            Terminator::CondBranch { cond, .. } if matches!(cond, Operand::Value(x) if x.0 == v.0) =>
                {}
            // Return/Switch/IndirectBranch use the value as data.
            Terminator::Return(Some(op)) if matches!(op, Operand::Value(x) if x.0 == v.0) => {
                return false;
            }
            Terminator::Switch { val, .. } if matches!(val, Operand::Value(x) if x.0 == v.0) => {
                return false;
            }
            Terminator::IndirectBranch { target, .. } if matches!(target, Operand::Value(x) if x.0 == v.0) =>
            {
                return false;
            }
            _ => {}
        }
    }
    true
}

/// Skip if-conversion for a short-circuit `||` diamond whose result is
/// consumed only by control flow. `cond ? 1 : rhs` (`||`) materializes as a
/// cmov chain, but its whole point is short-circuit evaluation — a branch is
/// cheaper and lets the backend fuse each compare straight into a jump (no
/// setcc+movzbl+test+cmov per link). Keeping the branchy form is the
/// difference between GCC's Expat codegen (`cmpb $X,%dl; ja`) and a wall of
/// setcc/cmov.
///
/// Only the `||` form (true arm == constant 1) is skipped. The `&&` form
/// (false arm == constant 0) is intentionally still converted: if-converting
/// it exposes the `Select(cond, Cmp, 0)` pattern the range-check fold
/// collapses into a single `sub`+`cmp`, which then fuses into the `||` branch.
fn skip_boolean_diamond(
    func: &IrFunction,
    phi_selects: &[(Value, IrType, Operand, Operand)],
) -> bool {
    if phi_selects.is_empty() {
        return false;
    }
    // Every converted phi must be the `||` shape: `cond ? 1 : rhs`.
    let all_or = phi_selects
        .iter()
        .all(|(_, _, tv, _fv)| matches!(tv, Operand::Const(c) if c.to_i64() == Some(1)));
    if !all_or {
        return false;
    }
    let mut visited = FxHashSet::default();
    phi_selects
        .iter()
        .all(|(dest, _, _, _)| value_only_controls_branch(func, *dest, &mut visited))
}

/// Detect a diamond pattern starting from a block with a CondBranch terminator.
fn detect_diamond(ctx: &IfConvCtx<'_>, pred_idx: usize) -> Option<DiamondInfo> {
    let func = &ctx.func;
    let label_to_idx = &ctx.label_to_idx;
    let preds = &ctx.preds;
    let pred_block = &func.blocks[pred_idx];

    // Must end with a CondBranch
    let (cond, true_label, false_label) = match &pred_block.terminator {
        Terminator::CondBranch {
            cond,
            true_label,
            false_label,
        } => (cond, true_label, false_label),
        _ => return None,
    };

    // Don't convert branches with constant conditions to Select.
    // cfg_simplify will fold these more efficiently (single pass vs multi-iteration).
    if is_constant_condition(pred_block, cond) {
        return None;
    }

    let true_idx = *label_to_idx.get(true_label)?;
    let false_idx = *label_to_idx.get(false_label)?;

    // The two arms must be different blocks
    if true_idx == false_idx {
        return None;
    }

    let true_block = &func.blocks[true_idx];
    let false_block = &func.blocks[false_idx];

    // Both arms must end with unconditional branches to the same merge block
    let true_target = match &true_block.terminator {
        Terminator::Branch(label) => *label_to_idx.get(label)?,
        _ => return None,
    };
    let false_target = match &false_block.terminator {
        Terminator::Branch(label) => *label_to_idx.get(label)?,
        _ => return None,
    };

    if true_target != false_target {
        return None; // Different merge blocks
    }
    let merge_idx = true_target;

    // The merge block must not be one of the arms
    if merge_idx == true_idx || merge_idx == false_idx || merge_idx == pred_idx {
        return None;
    }

    // The arm blocks should have exactly one predecessor each (the pred block).
    // If they have other predecessors, other code flows into them and we can't
    // eliminate the blocks.
    if preds.len(true_idx) != 1 || preds.len(false_idx) != 1 {
        return None;
    }
    if preds.row(true_idx)[0] as usize != pred_idx || preds.row(false_idx)[0] as usize != pred_idx {
        return None;
    }

    // Both arms must be side-effect-free.  Loads qualify only through the
    // speculative gate (loop-scoped, IV-addressed — see speculative_load_ok).
    if !arm_is_speculatable(ctx, true_block, true_idx)
        || !arm_is_speculatable(ctx, false_block, false_idx)
    {
        return None;
    }

    // Limit the number of instructions in each arm to prevent over-speculation.
    // cmov is only profitable when the arms are cheap (a few instructions).
    // Arm budget from the CPU tuning model: `mispredict_penalty / 2`
    // (a coin-flip prior on the branch outcome; 8 on the Generic row, which
    // reproduces the historical constant, 6 on Zen3/Zen4 whose 13-cycle
    // penalty makes speculation less valuable, 8 on Raptor Lake).  C's type
    // system generates Load + Cast chains (parameter loads + sign
    // extensions) that inflate the count; a typical arm: Load, Cast, Load,
    // Cast, BinOp, Cast = 6 insts.  See docs/CPU_MODEL_AUDIT.md §4.
    let max_arm_insts: usize = crate::backend::x86::cpu_model::active().if_convert_arm_budget();
    // EFFECTIVE instruction count: address-materialization chains (IV Cast,
    // Shl scaling, base Copy, the GEP itself) are pure address math the
    // backend folds into one SIB operand — they add nothing speculative.
    // Count only what actually executes speculatively. The conditional
    // reduction diamond (`if (arr[i] > 0) s += arr[i]`) is exactly this
    // shape: each arm = {Cast,Shl,Copy,Shl,GEP,Load,Cast,Add} — 8 raw,
    // 3 effective (Load, Cast, Add).
    let effective_arm_len = |block: &BasicBlock| -> usize {
        block
            .instructions
            .iter()
            .filter(|inst| {
                !matches!(
                    *inst,
                    Instruction::GetElementPtr { .. }
                        | Instruction::Copy { .. }
                        | Instruction::GlobalAddr { .. }
                        | Instruction::BinOp {
                            op: IrBinOp::Shl,
                            ..
                        }
                )
            })
            .count()
    };
    if effective_arm_len(true_block) > max_arm_insts
        || effective_arm_len(false_block) > max_arm_insts
    {
        return None;
    }

    // The merge block must have Phi nodes that reference both arms.
    // Collect phi nodes we can convert.
    let merge_block = &func.blocks[merge_idx];
    let mut phi_selects = Vec::with_capacity(16);

    for inst in &merge_block.instructions {
        if let Instruction::Phi { dest, ty, incoming } = inst {
            // Find the values from each arm
            let mut true_val = None;
            let mut false_val = None;

            for (op, label) in incoming {
                let src_idx = label_to_idx.get(label).copied();
                if src_idx == Some(true_idx) {
                    true_val = Some(*op);
                } else if src_idx == Some(false_idx) {
                    false_val = Some(*op);
                }
                // Other predecessors are fine - they just won't be converted
            }

            match (true_val, false_val) {
                (Some(tv), Some(fv)) => {
                    // Support integer, pointer, F32, and F64 types.
                    // F32/F64 work because all backends implement Select by
                    // moving bit patterns through integer registers (cmov/csel/branch).
                    // Skip long double (F128) and 128-bit integers — they need
                    // multi-register handling that Select doesn't support.
                    if !ty.is_long_double() && !ty.is_128bit() {
                        phi_selects.push((*dest, *ty, tv, fv));
                    } else {
                        // There's an unconvertible phi (F128/I128) referencing
                        // both arms. We must NOT partially convert this diamond
                        // or the remaining phi nodes will reference removed blocks.
                        return None;
                    }
                }
                _ => {
                    // Phi doesn't have entries from both arms - can't convert
                    // This could happen if the phi also has entries from other preds
                }
            }
        }
    }

    if phi_selects.is_empty() {
        return None; // No convertible phis
    }

    // Keep short-circuit boolean diamonds branchy (see skip_boolean_diamond).
    if skip_boolean_diamond(func, &phi_selects) {
        return None;
    }

    // The merge block should only be reached from the two arms (and not from pred directly).
    // If the merge block has other predecessors, we need to preserve the Phi nodes for those.
    let merge_preds_from_diamond = preds
        .row(merge_idx)
        .iter()
        .filter(|&&p| p as usize == true_idx || p as usize == false_idx)
        .count();

    // If the merge block has predecessors other than the two arms, the
    // conversion is PARTIAL: the converted phis are rewritten to keep their
    // other incoming edges (a fresh Select feeds a preserved Phi).  This is
    // what lets nested conditionals convert inside-out: the inner
    // diamond/triangle converts partially (the outer arm still feeds the
    // merge phi), then the fixpoint loop converts the outer one fully.
    let full_merge = preds.len(merge_idx) == 2;
    if !full_merge && merge_preds_from_diamond != 2 {
        return None;
    }

    Some(DiamondInfo {
        pred_idx,
        true_idx,
        false_idx,
        merge_idx,
        cond: *cond,
        true_arm_insts: true_block.instructions.clone(),
        false_arm_insts: false_block.instructions.clone(),
        phi_selects,
        full_merge,
    })
}

/// Detect a triangle pattern: pred branches to arm and merge directly.
///
/// ```text
///     pred: CondBranch(cond, arm, merge)   -- or (cond, merge, arm)
///     arm:  side-effect-free instructions + Branch(merge)
///     merge: phi [arm_val, arm], [pred_val, pred]
/// ```
///
/// This handles ternaries like `a >= t ? a - t : 0` where the false arm
/// is a constant and doesn't need its own block.
fn detect_triangle(ctx: &IfConvCtx<'_>, pred_idx: usize) -> Option<DiamondInfo> {
    let func = &ctx.func;
    let label_to_idx = &ctx.label_to_idx;
    let preds = &ctx.preds;
    let pred_block = &func.blocks[pred_idx];

    let (cond, true_label, false_label) = match &pred_block.terminator {
        Terminator::CondBranch {
            cond,
            true_label,
            false_label,
        } => (cond, true_label, false_label),
        _ => return None,
    };

    // Don't convert branches with constant conditions to Select.
    // cfg_simplify will fold these more efficiently (single pass vs multi-iteration).
    if is_constant_condition(pred_block, cond) {
        return None;
    }

    let true_idx = *label_to_idx.get(true_label)?;
    let false_idx = *label_to_idx.get(false_label)?;

    if true_idx == false_idx {
        return None;
    }

    // Determine which arm goes to a separate block and which goes directly to merge.
    // Case 1: true arm is a block, false arm goes directly to merge
    // Case 2: false arm is a block, true arm goes directly to merge
    let (arm_idx, merge_idx, arm_is_true) = {
        let true_block = &func.blocks[true_idx];
        let false_block = &func.blocks[false_idx];

        let true_target = match &true_block.terminator {
            Terminator::Branch(label) => label_to_idx.get(label).copied(),
            _ => None,
        };
        let false_target = match &false_block.terminator {
            Terminator::Branch(label) => label_to_idx.get(label).copied(),
            _ => None,
        };

        if let Some(tt) = true_target {
            if tt == false_idx {
                // true arm branches to false_idx which is the merge block
                (true_idx, false_idx, true)
            } else if let Some(ft) = false_target {
                if ft == true_idx {
                    // false arm branches to true_idx which is the merge block
                    (false_idx, true_idx, false)
                } else {
                    return None;
                }
            } else {
                return None;
            }
        } else if let Some(ft) = false_target {
            if ft == true_idx {
                (false_idx, true_idx, false)
            } else {
                return None;
            }
        } else {
            return None;
        }
    };

    // arm block must have exactly one predecessor (pred)
    if preds.len(arm_idx) != 1 || preds.row(arm_idx)[0] as usize != pred_idx {
        return None;
    }

    // merge must have the pred and the arm as predecessors; other
    // predecessors are allowed (partial conversion, see detect_diamond).
    let has_pred = preds.row(merge_idx).iter().any(|&p| p as usize == pred_idx);
    let has_arm = preds.row(merge_idx).iter().any(|&p| p as usize == arm_idx);
    if !has_pred || !has_arm {
        return None;
    }

    let arm_block = &func.blocks[arm_idx];

    // arm must be side-effect-free (speculative gate for loads)
    if !arm_is_speculatable(ctx, arm_block, arm_idx) {
        return None;
    }

    // Same tuning-model budget as the diamond form above.
    let max_arm_insts: usize = crate::backend::x86::cpu_model::active().if_convert_arm_budget();
    if arm_block.instructions.len() > max_arm_insts {
        return None;
    }

    // Collect phi nodes from merge block
    let merge_block = &func.blocks[merge_idx];
    let mut phi_selects = Vec::with_capacity(16);

    for inst in &merge_block.instructions {
        if let Instruction::Phi { dest, ty, incoming } = inst {
            let mut arm_val = None;
            let mut pred_val = None;

            for (op, label) in incoming {
                let src_idx = label_to_idx.get(label).copied();
                if src_idx == Some(arm_idx) {
                    arm_val = Some(*op);
                } else if src_idx == Some(pred_idx) {
                    pred_val = Some(*op);
                }
            }

            if let (Some(av), Some(pv)) = (arm_val, pred_val) {
                // Support integer, pointer, F32, and F64 types.
                // Skip long double (F128) and 128-bit integers.
                if !ty.is_long_double() && !ty.is_128bit() {
                    // Map to true/false values based on which arm the block is
                    if arm_is_true {
                        phi_selects.push((*dest, *ty, av, pv));
                    } else {
                        phi_selects.push((*dest, *ty, pv, av));
                    }
                } else {
                    // Unconvertible phi — bail out to avoid partial conversion.
                    return None;
                }
            }
        }
    }

    if phi_selects.is_empty() {
        return None;
    }

    // Keep short-circuit boolean diamonds branchy (see skip_boolean_diamond).
    if skip_boolean_diamond(func, &phi_selects) {
        return None;
    }

    // For a triangle, we set the missing arm to merge_idx with empty instructions.
    // apply_diamond will hoist the arm instructions and the empty side is a no-op.
    let (true_idx_out, false_idx_out, true_insts, false_insts) = if arm_is_true {
        (
            arm_idx,
            merge_idx,
            arm_block.instructions.clone(),
            Vec::new(),
        )
    } else {
        (
            merge_idx,
            arm_idx,
            Vec::new(),
            arm_block.instructions.clone(),
        )
    };

    Some(DiamondInfo {
        pred_idx,
        true_idx: true_idx_out,
        false_idx: false_idx_out,
        merge_idx,
        cond: *cond,
        true_arm_insts: true_insts,
        false_arm_insts: false_insts,
        phi_selects,
        full_merge: preds.len(merge_idx) == 2,
    })
}

/// Apply a diamond conversion: rewrite the CFG to use Select instructions.
fn apply_diamond(func: &mut IrFunction, diamond: &DiamondInfo) -> bool {
    // Safety check: make sure the blocks haven't been modified by a previous conversion
    // in this same pass iteration
    if diamond.pred_idx >= func.blocks.len()
        || diamond.true_idx >= func.blocks.len()
        || diamond.false_idx >= func.blocks.len()
        || diamond.merge_idx >= func.blocks.len()
    {
        return false;
    }

    // Read the merge label before mutating
    let merge_label = func.blocks[diamond.merge_idx].label;

    // 1. Move instructions from both arms into the pred block (before the branch).
    //    Since both arms are side-effect-free, we can execute all their instructions
    //    unconditionally (both paths are computed).
    //    (Labels are read before the mutable borrow of the pred block.)
    let pred_label = func.blocks[diamond.pred_idx].label;
    let true_label = func.blocks[diamond.true_idx].label;
    let false_label = func.blocks[diamond.false_idx].label;
    let pred_block = &mut func.blocks[diamond.pred_idx];

    // Add true arm instructions (with dummy spans)
    let has_spans = !pred_block.source_spans.is_empty();
    for inst in &diamond.true_arm_insts {
        pred_block.instructions.push(inst.clone());
        if has_spans {
            pred_block
                .source_spans
                .push(crate::common::source::Span::dummy());
        }
    }

    // Add false arm instructions
    for inst in &diamond.false_arm_insts {
        pred_block.instructions.push(inst.clone());
        if has_spans {
            pred_block
                .source_spans
                .push(crate::common::source::Span::dummy());
        }
    }

    // 2. Add Select instructions for each Phi.  A full merge converts each
    //    phi into its Select (same destination, phi removed below).  A
    //    partial merge (the merge block has other predecessors) emits the
    //    Select into a FRESH destination and rewrites the surviving phi's
    //    incoming list to carry that value on the pred edge.
    let partial_rewrites: Vec<(Value, Value)> = diamond
        .phi_selects
        .iter()
        .map(|(dest, ty, true_val, false_val)| {
            let select_dest = if diamond.full_merge {
                *dest
            } else {
                let fresh = Value(func.next_value_id);
                func.next_value_id += 1;
                fresh
            };
            pred_block.instructions.push(Instruction::Select {
                dest: select_dest,
                cond: diamond.cond,
                true_val: *true_val,
                false_val: *false_val,
                ty: *ty,
            });
            if has_spans {
                pred_block
                    .source_spans
                    .push(crate::common::source::Span::dummy());
            }
            (*dest, select_dest)
        })
        .collect();

    // 3. Change pred block's terminator to unconditional branch to merge
    pred_block.terminator = Terminator::Branch(merge_label);

    // 4. Remove the converted Phi nodes from the merge block (full merges),
    //    or rewrite their incoming lists to the fresh Select values
    //    (partial merges — the other incoming edges survive).
    let converted_dests: crate::common::fx_hash::FxHashSet<u32> = diamond
        .phi_selects
        .iter()
        .map(|(dest, _, _, _)| dest.0)
        .collect();
    if diamond.full_merge {
        {
            let merge_block = &mut func.blocks[diamond.merge_idx];
            if !merge_block.source_spans.is_empty() {
                let mut idx = 0;
                let insts = &merge_block.instructions;
                // Guard against source_spans having more entries than
                // instructions (a prior pass — IV widen, loop_rotate, DCE —
                // may have removed instructions without shrinking
                // source_spans to match). An out-of-bounds index here was a
                // hard panic that took simd_sse2_arith / simd_vecreg_new_ops
                // down with it. Drop the surplus spans rather than crashing.
                merge_block.source_spans.retain(|_| {
                    let keep = if idx < insts.len() {
                        if let Instruction::Phi { dest, .. } = &insts[idx] {
                            !converted_dests.contains(&dest.0)
                        } else {
                            true
                        }
                    } else {
                        // No matching instruction (span is stale): drop it.
                        false
                    };
                    idx += 1;
                    keep
                });
            }
        }
        func.blocks[diamond.merge_idx].instructions.retain(|inst| {
            if let Instruction::Phi { dest, .. } = inst {
                !converted_dests.contains(&dest.0)
            } else {
                true
            }
        });
    } else {
        let merge_block = &mut func.blocks[diamond.merge_idx];
        for inst in &mut merge_block.instructions {
            let Instruction::Phi {
                dest,
                incoming,
                ty: _,
            } = inst
            else {
                continue;
            };
            let Some(&(_, fresh)) = partial_rewrites
                .iter()
                .find(|(phi_dest, _)| phi_dest.0 == dest.0)
            else {
                continue;
            };
            // The pred edge now carries the Select value; the arm edges
            // disappear with the arms.
            incoming.retain(|(_, lbl)| *lbl != true_label && *lbl != false_label);
            let mut replaced = false;
            for (op, lbl) in incoming.iter_mut() {
                if *lbl == pred_label {
                    *op = Operand::Value(fresh);
                    replaced = true;
                    break;
                }
            }
            if !replaced {
                incoming.push((Operand::Value(fresh), pred_label));
            }
        }
    }

    // 5. Empty the arm blocks (they'll be cleaned up by CFG simplification).
    // Keep them as empty blocks with unconditional branches - CFG simplify
    // will remove them as dead blocks since they'll have no predecessors.
    // For triangle patterns, one arm IS the merge block - don't clear it.
    if diamond.true_idx != diamond.merge_idx {
        func.blocks[diamond.true_idx].instructions.clear();
        func.blocks[diamond.true_idx].source_spans.clear();
    }
    if diamond.false_idx != diamond.merge_idx {
        func.blocks[diamond.false_idx].instructions.clear();
        func.blocks[diamond.false_idx].source_spans.clear();
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::AddressSpace;
    use crate::ir::reexports::{IrBinOp, IrConst};

    #[test]
    fn test_value_only_controls_branch() {
        // v used only as a CondBranch condition => branch-only.
        let mut func = IrFunction::new("t".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::ParamRef {
                dest: Value(0),
                param_idx: 0,
                ty: IrType::I32,
            }],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(0)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
        });
        let mut visited = FxHashSet::default();
        assert!(value_only_controls_branch(&func, Value(0), &mut visited));

        // v returned as data => not branch-only.
        func.blocks[0].terminator = Terminator::Return(Some(Operand::Value(Value(0))));
        let mut visited = FxHashSet::default();
        assert!(!value_only_controls_branch(&func, Value(0), &mut visited));

        // v used by Cmp(Eq, 0) whose result feeds a CondBranch => branch-only.
        let mut func2 = IrFunction::new("t2".to_string(), IrType::I32, vec![], false);
        func2.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::I32,
                },
                Instruction::Cmp {
                    dest: Value(1),
                    op: IrCmpOp::Eq,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(0)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(1)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
        });
        let mut visited = FxHashSet::default();
        assert!(value_only_controls_branch(&func2, Value(0), &mut visited));
    }

    #[test]
    fn test_skip_boolean_diamond_or_shape() {
        // `||` shape: true arm == const 1, result only feeds a branch => skip.
        let mut func = IrFunction::new("t".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::ParamRef {
                dest: Value(0),
                param_idx: 0,
                ty: IrType::I32,
            }],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(0)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::Phi {
                dest: Value(3),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Const(IrConst::I32(1)), BlockId(1)),
                    (Operand::Value(Value(4)), BlockId(2)),
                ],
            }],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(3)),
                true_label: BlockId(4),
                false_label: BlockId(5),
            },
            source_spans: Vec::new(),
        });
        func.next_value_id = 6;

        // `||` shape, branch-only => skipped (not converted).
        let ctx = IfConvCtx::build(&func);
        let diamond = detect_diamond(&ctx, 0);
        assert!(
            diamond.is_none(),
            "branch-only `||` diamond must not be converted"
        );

        // `&&` shape (false arm == const 0) => still converted (range fold needs it).
        let mut func2 = IrFunction::new("t2".to_string(), IrType::I32, vec![], false);
        func2.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::ParamRef {
                dest: Value(0),
                param_idx: 0,
                ty: IrType::I32,
            }],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(0)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
        });
        func2.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
        });
        func2.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
        });
        func2.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::Phi {
                dest: Value(3),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Value(Value(4)), BlockId(1)),
                    (Operand::Const(IrConst::I32(0)), BlockId(2)),
                ],
            }],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(3)),
                true_label: BlockId(4),
                false_label: BlockId(5),
            },
            source_spans: Vec::new(),
        });
        func2.next_value_id = 6;
        let ctx2 = IfConvCtx::build(&func2);
        assert!(
            detect_diamond(&ctx2, 0).is_some(),
            "branch-only `&&` diamond must still be converted for range folding"
        );
    }

    #[test]
    fn test_simple_diamond_conversion() {
        // Build a simple diamond:
        //   block0: condbranch %0, block1, block2
        //   block1: branch block3
        //   block2: branch block3
        //   block3: %3 = phi [const(1), block1], [const(0), block2]; return %3
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);

        // Block 0: condbranch
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(0)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
        });

        // Block 1: true arm
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
        });

        // Block 2: false arm
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
        });

        // Block 3: merge with phi
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::Phi {
                dest: Value(3),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Const(IrConst::I32(1)), BlockId(1)),
                    (Operand::Const(IrConst::I32(0)), BlockId(2)),
                ],
            }],
            terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
            source_spans: Vec::new(),
        });

        func.next_value_id = 4;

        let converted = if_convert_function(&mut func);
        assert_eq!(converted, 1);

        // Block 0 should now have a Select instruction and branch to block3
        assert_eq!(func.blocks[0].instructions.len(), 1);
        match &func.blocks[0].instructions[0] {
            Instruction::Select {
                dest,
                cond,
                true_val,
                false_val,
                ty,
            } => {
                assert_eq!(dest.0, 3);
                assert!(matches!(cond, Operand::Value(Value(0))));
                assert!(matches!(true_val, Operand::Const(IrConst::I32(1))));
                assert!(matches!(false_val, Operand::Const(IrConst::I32(0))));
                assert_eq!(*ty, IrType::I32);
            }
            other => panic!("Expected Select, got {:?}", other),
        }

        // Block 0 should now branch unconditionally to block3
        assert!(matches!(
            func.blocks[0].terminator,
            Terminator::Branch(BlockId(3))
        ));

        // Merge block should have no phi
        assert!(!func.blocks[3]
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Phi { .. })));
    }

    #[test]
    fn test_diamond_with_arm_instructions() {
        // Diamond where the true arm computes a value:
        //   block0: condbranch %0, block1, block2
        //   block1: %1 = sub %0, const(5); branch block3
        //   block2: branch block3
        //   block3: %2 = phi [%1, block1], [const(0), block2]; return %2
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);

        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(0)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
        });

        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![Instruction::BinOp {
                dest: Value(1),
                op: IrBinOp::Sub,
                lhs: Operand::Value(Value(0)),
                rhs: Operand::Const(IrConst::I32(5)),
                ty: IrType::I32,
            }],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
        });

        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
        });

        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::Phi {
                dest: Value(2),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Value(Value(1)), BlockId(1)),
                    (Operand::Const(IrConst::I32(0)), BlockId(2)),
                ],
            }],
            terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
            source_spans: Vec::new(),
        });

        func.next_value_id = 3;

        let converted = if_convert_function(&mut func);
        assert_eq!(converted, 1);

        // Block 0 should have the hoisted BinOp and the Select
        assert_eq!(func.blocks[0].instructions.len(), 2);
        assert!(matches!(
            func.blocks[0].instructions[0],
            Instruction::BinOp { .. }
        ));
        assert!(matches!(
            func.blocks[0].instructions[1],
            Instruction::Select { .. }
        ));
    }

    #[test]
    fn test_no_conversion_with_side_effects() {
        // Diamond where the true arm has a store (side effect) - should NOT convert
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);

        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Alloca {
                dest: Value(10),
                ty: IrType::I32,
                size: 4,
                align: 4,
                volatile: false,
                semantic_volatile: false,
            }],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(0)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
        });

        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                // Side-effecting store!
                Instruction::Store {
                    volatile: false,
                    val: Operand::Const(IrConst::I32(42)),
                    ptr: Value(10),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
            ],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
        });

        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
        });

        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::Phi {
                dest: Value(2),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Const(IrConst::I32(1)), BlockId(1)),
                    (Operand::Const(IrConst::I32(0)), BlockId(2)),
                ],
            }],
            terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
            source_spans: Vec::new(),
        });

        func.next_value_id = 11;

        let converted = if_convert_function(&mut func);
        assert_eq!(converted, 0); // Should NOT convert due to side effects
    }

    /// Helper: one float array parameter reference in block `bi`.
    fn float_param(dest: u32, param_idx: usize) -> Instruction {
        Instruction::ParamRef {
            dest: Value(dest),
            param_idx,
            ty: IrType::F32,
        }
    }

    /// Helper: `dest = shl(src, 2)` (byte scale for a float array index).
    fn shl2(dest: u32, src: u32) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op: IrBinOp::Shl,
            lhs: Operand::Value(Value(src)),
            rhs: Operand::Const(IrConst::I32(2)),
            ty: IrType::I64,
        }
    }

    /// Helper: `dest = gep(base, offset)`.
    fn gep(dest: u32, base: u32, offset: u32) -> Instruction {
        Instruction::GetElementPtr {
            dest: Value(dest),
            base: Value(base),
            offset: Operand::Value(Value(offset)),
            ty: IrType::F32,
        }
    }

    fn block(label: u32, insts: Vec<Instruction>, term: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions: insts,
            terminator: term,
            source_spans: Vec::new(),
        }
    }

    /// A canonical loop: entry -> header(iv phi + exit test) -> body -> latch,
    /// with a ternary diamond inside the body whose TRUE arm loads c[i].
    /// The load is IV-addressed, so it is speculatable and the diamond must
    /// convert to a Select.
    fn iv_load_diamond_loop(offset_src: u32) -> IrFunction {
        let mut f = IrFunction::new("t".to_string(), IrType::I32, vec![], false);
        // b0: entry — params d,a,b,c,n + branch to header.
        f.blocks.push(block(
            0,
            vec![
                float_param(0, 0), // d
                float_param(1, 1), // a
                float_param(2, 2), // b
                float_param(3, 3), // c
                Instruction::ParamRef {
                    dest: Value(4),
                    param_idx: 4,
                    ty: IrType::U64,
                }, // n
            ],
            Terminator::Branch(BlockId(1)),
        ));
        // b1: header — iv phi, exit test, CondBranch(body, exit).
        f.blocks.push(block(
            1,
            vec![Instruction::Phi {
                dest: Value(10),
                ty: IrType::U64,
                incoming: vec![
                    (Operand::Const(IrConst::I64(0)), BlockId(0)),
                    (Operand::Value(Value(11)), BlockId(5)),
                ],
            },
            Instruction::Cmp {
                dest: Value(12),
                op: IrCmpOp::Ult,
                lhs: Operand::Value(Value(10)),
                rhs: Operand::Value(Value(4)),
                ty: IrType::U64,
            }],
            Terminator::CondBranch {
                cond: Operand::Value(Value(12)),
                true_label: BlockId(2),
                false_label: BlockId(6),
            },
        ));
        // b2: body pred — load a[i], load b[i], FP compare, CondBranch.
        f.blocks.push(block(
            2,
            vec![
                shl2(20, 10),
                gep(21, 1, 20),
                Instruction::Load {
                    dest: Value(22),
                    ptr: Value(21),
                    ty: IrType::F32,
                    volatile: false,
                    seg_override: AddressSpace::Default,
                },
                shl2(23, 10),
                gep(24, 2, 23),
                Instruction::Load {
                    dest: Value(25),
                    ptr: Value(24),
                    ty: IrType::F32,
                    volatile: false,
                    seg_override: AddressSpace::Default,
                },
                Instruction::Cmp {
                    dest: Value(26),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(22)),
                    rhs: Operand::Value(Value(25)),
                    ty: IrType::F32,
                },
            ],
            Terminator::CondBranch {
                cond: Operand::Value(Value(26)),
                true_label: BlockId(3),
                false_label: BlockId(4),
            },
        ));
        // b3: true arm — load c[i] (IV-addressed: speculatable).
        f.blocks.push(block(
            3,
            vec![
                shl2(30, 10),
                gep(31, 3, 30),
                Instruction::Load {
                    dest: Value(32),
                    ptr: Value(31),
                    ty: IrType::F32,
                    volatile: false,
                    seg_override: AddressSpace::Default,
                },
            ],
            Terminator::Branch(BlockId(5)),
        ));
        // b4: false arm — value is b[i] (already loaded in the pred).
        f.blocks.push(block(4, vec![], Terminator::Branch(BlockId(5))));
        // b5: merge — phi for d[i], store d[i], iv increment, backedge.
        f.blocks.push(block(
            5,
            vec![
                Instruction::Phi {
                    dest: Value(40),
                    ty: IrType::F32,
                    incoming: vec![
                        (Operand::Value(Value(32)), BlockId(3)),
                        (Operand::Value(Value(25)), BlockId(4)),
                    ],
                },
                shl2(41, 10),
                gep(42, 0, 41),
                Instruction::Store {
                    val: Operand::Value(Value(40)),
                    ptr: Value(42),
                    ty: IrType::F32,
                    volatile: false,
                    seg_override: AddressSpace::Default,
                },
                Instruction::BinOp {
                    dest: Value(11),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(10)),
                    rhs: Operand::Const(IrConst::I64(1)),
                    ty: IrType::U64,
                },
            ],
            Terminator::Branch(BlockId(1)),
        ));
        // b6: exit.
        f.blocks.push(block(6, vec![], Terminator::Return(None)));
        f.next_value_id = 50;
        // The negative test swaps the arm GEP offset to a non-IV value.
        if offset_src != 10 {
            for inst in &mut f.blocks[3].instructions {
                if let Instruction::GetElementPtr { offset, .. } = inst {
                    *offset = Operand::Value(Value(offset_src));
                }
            }
        }
        f
    }

    #[test]
    fn test_iv_addressed_arm_load_converts() {
        let mut f = iv_load_diamond_loop(10);
        let converted = if_convert_function(&mut f);
        assert!(converted > 0, "IV-addressed arm load diamond must convert");
        assert!(
            f.blocks[2]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Select { .. })),
            "expected a Select in the pred block"
        );
        assert!(f.blocks[3].instructions.is_empty());
        assert!(f.blocks[4].instructions.is_empty());
        assert!(!f
            .blocks[5]
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Phi { dest: d, .. } if d.0 == 40)));
    }

    #[test]
    fn test_non_iv_arm_load_does_not_convert() {
        // The arm's GEP offset is the loop bound (a parameter), not an
        // IV-derived value: the load cannot be proven to walk the loop's
        // own range and must stay behind the branch.
        let mut f = iv_load_diamond_loop(4);
        let converted = if_convert_function(&mut f);
        assert_eq!(converted, 0, "non-IV arm load must not be speculated");
        assert!(!f
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .any(|i| matches!(i, Instruction::Select { .. })));
    }

    /// Nested clamp: outer `a < 0` diamond whose false arm holds the inner
    /// `a > 1` TRIANGLE; the merge block has three predecessors (outer true
    /// arm, inner arm, inner pred).  The fixpoint must convert the inner
    /// triangle PARTIALLY (keeping the outer arm's phi entry), then the
    /// outer diamond fully — producing two nested Selects.
    #[test]
    fn test_partial_merge_phi_nested_clamp() {
        let mut f = IrFunction::new("t".to_string(), IrType::I32, vec![], false);
        // b0: entry.
        f.blocks.push(block(
            0,
            vec![float_param(0, 0)],
            Terminator::Branch(BlockId(1)),
        ));
        // b1: outer pred — load a (as a param value, v0), cmp a<0.
        f.blocks.push(block(
            1,
            vec![Instruction::Cmp {
                dest: Value(1),
                op: IrCmpOp::Slt,
                lhs: Operand::Value(Value(0)),
                rhs: Operand::Const(IrConst::F32(0.0)),
                ty: IrType::F32,
            }],
            Terminator::CondBranch {
                cond: Operand::Value(Value(1)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
        ));
        // b2: outer true arm — 0.0.
        f.blocks.push(block(2, vec![], Terminator::Branch(BlockId(6))));
        // b3: inner pred — cmp a>1; triangle: true arm b4, false edge direct.
        f.blocks.push(block(
            3,
            vec![Instruction::Cmp {
                dest: Value(2),
                op: IrCmpOp::Sgt,
                lhs: Operand::Value(Value(0)),
                rhs: Operand::Const(IrConst::F32(1.0)),
                ty: IrType::F32,
            }],
            Terminator::CondBranch {
                cond: Operand::Value(Value(2)),
                true_label: BlockId(4),
                false_label: BlockId(6),
            },
        ));
        // b4: inner true arm — 1.0.
        f.blocks.push(block(4, vec![], Terminator::Branch(BlockId(6))));
        // b5: unused intermediate (keeps label ids realistic).
        // b6: merge — phi with THREE incoming, return it.
        f.blocks.push(block(
            6,
            vec![Instruction::Phi {
                dest: Value(5),
                ty: IrType::F32,
                incoming: vec![
                    (Operand::Const(IrConst::F32(0.0)), BlockId(2)),
                    (Operand::Const(IrConst::F32(1.0)), BlockId(4)),
                    (Operand::Value(Value(0)), BlockId(3)),
                ],
            }],
            Terminator::Return(Some(Operand::Value(Value(5)))),
        ));
        f.next_value_id = 10;

        let converted = if_convert_function(&mut f);
        assert!(converted >= 2, "nested clamp needs two conversions");
        let selects = f
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .filter(|i| matches!(i, Instruction::Select { .. }))
            .count();
        assert_eq!(selects, 2, "expected two nested Selects");
        assert!(f
            .blocks
            .iter()
            .all(|b| !matches!(b.terminator, Terminator::CondBranch { .. })));
        // The outer pred (block 1) must hold the final nested Select.
        assert!(f.blocks[1]
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Select { .. })));
    }

    /// Store sinking: two predecessors store to the same IV-addressed slot;
    /// the sink must turn them into a phi plus ONE store in the merge.
    #[test]
    fn test_store_sinking_merges_conditional_stores() {
        let mut f = IrFunction::new("t".to_string(), IrType::I32, vec![], false);
        // b0: entry — d param, iv param, cond param, branch to pred.
        f.blocks.push(block(
            0,
            vec![
                float_param(0, 0), // d
                Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 1,
                    ty: IrType::U64,
                }, // iv
                Instruction::ParamRef {
                    dest: Value(2),
                    param_idx: 2,
                    ty: IrType::I32,
                }, // cond
            ],
            Terminator::Branch(BlockId(1)),
        ));
        // b1: pred — CondBranch to the two store arms.
        f.blocks.push(block(
            1,
            vec![],
            Terminator::CondBranch {
                cond: Operand::Value(Value(2)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
        ));
        // b2: arm 1 — store 0.0f to d[iv].
        f.blocks.push(block(
            2,
            vec![
                shl2(10, 1),
                gep(11, 0, 10),
                Instruction::Store {
                    val: Operand::Const(IrConst::F32(0.0)),
                    ptr: Value(11),
                    ty: IrType::F32,
                    volatile: false,
                    seg_override: AddressSpace::Default,
                },
            ],
            Terminator::Branch(BlockId(4)),
        ));
        // b3: arm 2 — store 1.0f to d[iv] (separate GEP SSA ids).
        f.blocks.push(block(
            3,
            vec![
                shl2(12, 1),
                gep(13, 0, 12),
                Instruction::Store {
                    val: Operand::Const(IrConst::F32(1.0)),
                    ptr: Value(13),
                    ty: IrType::F32,
                    volatile: false,
                    seg_override: AddressSpace::Default,
                },
            ],
            Terminator::Branch(BlockId(4)),
        ));
        // b4: merge.
        f.blocks.push(block(4, vec![], Terminator::Return(None)));
        f.next_value_id = 20;

        let sunk = sink_conditional_stores(&mut f);
        assert_eq!(sunk, 2, "both per-pred stores must sink");
        assert!(!matches!(
            f.blocks[2].instructions.last(),
            Some(Instruction::Store { .. })
        ));
        assert!(!matches!(
            f.blocks[3].instructions.last(),
            Some(Instruction::Store { .. })
        ));
        let b4 = &f.blocks[4];
        assert!(
            b4.instructions
                .iter()
                .any(|i| matches!(i, Instruction::Phi { .. })),
            "merge block must hold the value phi"
        );
        let stores = b4
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::Store { .. }))
            .count();
        assert_eq!(stores, 1, "exactly one store must remain");
        // The store must sit AFTER the phi and its cloned address chain.
        let phi_pos = b4
            .instructions
            .iter()
            .position(|i| matches!(i, Instruction::Phi { .. }))
            .unwrap();
        let store_pos = b4
            .instructions
            .iter()
            .position(|i| matches!(i, Instruction::Store { .. }))
            .unwrap();
        assert!(phi_pos < store_pos);
    }
}
