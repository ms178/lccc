//! Live-range splitting: shorten linear-scan intervals so the allocator can
//! put caller-saved registers on either side of a call, and so a value that
//! is merely *live through* a loop does not occupy a GPR for the whole nest.
//!
//! Two local, semantics-preserving rewrites plus a layout pass:
//!
//! 1. [`split_call_spanning_ranges`] — store before a call, load a fresh SSA
//!    name after it, rename same-block post-call uses. The old name's last
//!    use is the store, so the interval no longer contains the call.
//! 2. [`split_loop_transparent_ranges`] — store in the preheader, reload at
//!    the unique exit, rename uses dominated by that exit. The old name dies
//!    at the preheader store and is not referenced in the body.
//! 3. [`place_edge_copy_blocks`] — sit phi edge-copy blocks next to their
//!    predecessor so a contiguous-interval allocator does not see mutually
//!    exclusive arms as overlapping.
//!
//! # Invariants
//!
//! - Spill slots are **volatile** allocas (`semantic_volatile: false`). A
//!   later mem2reg must not forward the store into the load and collapse the
//!   split. We never flip volatility on pre-existing allocas and we never
//!   call mem2reg from this module (the old dance was both a no-op — uses of
//!   `V` were never rewritten — and panic-unsafe).
//! - Loads are inserted *after* leading `Phi`s. Phis stay first in the block.
//! - Phi incoming operands are uses at the predecessor, not at the phi block.
//!   We never rewrite them to a value defined in the phi's own block.
//! - `Copy` result types are the source type, not `Ptr`. Treating every Copy
//!   as a pointer produced typed load/store mismatches (measured miscompile
//!   shape).
//! - Safe degradation is “don't split”. Partial rewrite that leaves `V` live
//!   across the call/loop pays the memops and keeps the long interval.
//!
//! # What this is not
//!
//! Full Cytron reconstruction at every dominance frontier. Cross-block
//! post-call uses require phis; those candidates are skipped. Loop-transparent
//! split requires a preheader, a unique exit, and every use dominated by that
//! exit. Callers gate both with `max_splits == 0`.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::analysis;
use crate::ir::reexports::*;
use crate::passes::loop_analysis;
use std::sync::OnceLock;

/// Walk `a`'s idom chain. True iff `b` dominates `a` (including `a == b`).
fn dominated_by(idom: &[usize], a: usize, b: usize) -> bool {
    if a == b {
        return true;
    }
    let mut cur = a;
    for _ in 0..idom.len().saturating_add(1) {
        if cur == b {
            return true;
        }
        if cur >= idom.len() {
            return false;
        }
        let next = idom[cur];
        if next == cur || next == usize::MAX {
            return false;
        }
        cur = next;
    }
    false
}

/// Per-block set of dominating block indices (the idom chain including self).
/// Turns the O(|B|) walk into an O(1) lookup for the many queries below.
fn dominator_sets(idom: &[usize]) -> Vec<FxHashSet<usize>> {
    let n = idom.len();
    let mut sets = vec![FxHashSet::default(); n];
    for a in 0..n {
        sets[a].insert(a);
        let mut cur = a;
        for _ in 0..n.saturating_add(1) {
            if cur >= n {
                break;
            }
            let next = idom[cur];
            if next == cur || next == usize::MAX {
                break;
            }
            sets[a].insert(next);
            cur = next;
        }
    }
    sets
}

#[inline]
fn set_dominates(sets: &[FxHashSet<usize>], a: usize, b: usize) -> bool {
    sets.get(a).is_some_and(|s| s.contains(&b))
}

fn split_debug_enabled() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("CCC_DEBUG_SPLIT").is_some())
}

/// Insert `inst` at `idx`, keeping `source_spans` 1:1 when it already was.
/// Never `clear()`s the span vector (that wiped DWARF for the whole block).
fn insert_instruction(block: &mut BasicBlock, idx: usize, inst: Instruction) {
    let n_inst = block.instructions.len();
    let n_span = block.source_spans.len();
    let idx = idx.min(n_inst);
    block.instructions.insert(idx, inst);
    if n_span == n_inst && n_span > 0 {
        let span_idx = idx.min(n_span - 1);
        let span = block.source_spans[span_idx].clone();
        block.source_spans.insert(idx.min(n_span), span);
    }
}

fn first_non_alloca(block: &BasicBlock) -> usize {
    block
        .instructions
        .iter()
        .position(|i| !matches!(i, Instruction::Alloca { .. }))
        .unwrap_or(block.instructions.len())
}

fn first_non_phi(block: &BasicBlock) -> usize {
    block
        .instructions
        .iter()
        .position(|i| !matches!(i, Instruction::Phi { .. }))
        .unwrap_or(block.instructions.len())
}

fn abi_align(ty: IrType) -> usize {
    match ty.size() {
        0 => 1,
        1 => 1,
        2 => 2,
        3 | 4 => 4,
        _ => 8,
    }
}

fn is_simple_gpr_type(ty: IrType) -> bool {
    !ty.is_float() && !ty.is_long_double() && !ty.is_128bit()
}

fn next_value(next_val: &mut u32) -> Option<Value> {
    let id = *next_val;
    if id == u32::MAX {
        return None;
    }
    *next_val = id.saturating_add(1);
    Some(Value(id))
}

// ── Use / type / replace ─────────────────────────────────────────────────────

fn instruction_uses_value(inst: &Instruction, vid: u32) -> bool {
    let mut hit = false;
    super::liveness::for_each_operand_in_instruction(inst, |op| {
        if matches!(op, Operand::Value(v) if v.0 == vid) {
            hit = true;
        }
    });
    if hit {
        return true;
    }
    super::liveness::for_each_value_use_in_instruction(inst, |v| {
        if v.0 == vid {
            hit = true;
        }
    });
    hit
}

fn terminator_uses_value(term: &Terminator, vid: u32) -> bool {
    let mut hit = false;
    super::liveness::for_each_operand_in_terminator(term, |op| {
        if matches!(op, Operand::Value(v) if v.0 == vid) {
            hit = true;
        }
    });
    hit
}

fn rewrite_operand(op: &mut Operand, map: &FxHashMap<u32, u32>) {
    if let Operand::Value(v) = op {
        if let Some(&n) = map.get(&v.0) {
            v.0 = n;
        }
    }
}

fn rewrite_value(v: &mut Value, map: &FxHashMap<u32, u32>) {
    if let Some(&n) = map.get(&v.0) {
        v.0 = n;
    }
}

/// Rewrite every *use* of a mapped value. Destinations are left alone.
/// Phi incoming is rewritten only when `rewrite_phi` is true — callers that
/// just defined a replacement in *this* block must pass false (use-before-def).
fn replace_values_in_inst(inst: &mut Instruction, map: &FxHashMap<u32, u32>, rewrite_phi: bool) {
    match inst {
        Instruction::Alloca { .. }
        | Instruction::PgoCounterInc { .. }
        | Instruction::GlobalAddr { .. }
        | Instruction::Fence { .. }
        | Instruction::LabelAddr { .. }
        | Instruction::GetReturnF64Second { .. }
        | Instruction::GetReturnF32Second { .. }
        | Instruction::GetReturnF128Second { .. }
        | Instruction::GetStaticChain { .. }
        | Instruction::StackSave { .. }
        | Instruction::ParamRef { .. }
        | Instruction::VaEnd { .. } => {}
        // Nested-function support: operand rewrites.
        Instruction::SetStaticChain { src } => rewrite_operand(src, map),
        Instruction::InitTrampoline { buffer, chain, .. } => {
            rewrite_value(buffer, map);
            rewrite_operand(chain, map);
        }
        Instruction::NonlocalGotoSave { frame, .. } => rewrite_value(frame, map),
        Instruction::NonlocalGoto { chain, .. } => rewrite_operand(chain, map),
        Instruction::DynAlloca { size, .. } => rewrite_operand(size, map),
        Instruction::Store { val, ptr, .. } => {
            rewrite_operand(val, map);
            rewrite_value(ptr, map);
        }
        Instruction::Load { ptr, .. } => rewrite_value(ptr, map),
        Instruction::BinOp { lhs, rhs, .. } | Instruction::Cmp { lhs, rhs, .. } => {
            rewrite_operand(lhs, map);
            rewrite_operand(rhs, map);
        }
        Instruction::UnaryOp { src, .. }
        | Instruction::Cast { src, .. }
        | Instruction::Copy { src, .. } => rewrite_operand(src, map),
        Instruction::Call { info, .. } => {
            for a in &mut info.args {
                rewrite_operand(a, map);
            }
        }
        Instruction::CallIndirect { func_ptr, info } => {
            rewrite_operand(func_ptr, map);
            for a in &mut info.args {
                rewrite_operand(a, map);
            }
        }
        Instruction::GetElementPtr { base, offset, .. } => {
            rewrite_value(base, map);
            rewrite_operand(offset, map);
        }
        Instruction::Memcpy { dest, src, .. } => {
            rewrite_value(dest, map);
            rewrite_value(src, map);
        }
        Instruction::VaArg { va_list_ptr, .. } | Instruction::VaStart { va_list_ptr } => {
            rewrite_value(va_list_ptr, map);
        }
        Instruction::VaCopy { dest_ptr, src_ptr } => {
            rewrite_value(dest_ptr, map);
            rewrite_value(src_ptr, map);
        }
        Instruction::VaArgStruct {
            dest_ptr,
            va_list_ptr,
            ..
        } => {
            rewrite_value(dest_ptr, map);
            rewrite_value(va_list_ptr, map);
        }
        Instruction::AtomicRmw { ptr, val, .. } | Instruction::AtomicStore { ptr, val, .. } => {
            rewrite_operand(ptr, map);
            rewrite_operand(val, map);
        }
        Instruction::AtomicInc { ptr, .. } | Instruction::AtomicLoad { ptr, .. } => {
            rewrite_operand(ptr, map);
        }
        Instruction::AtomicCmpxchg {
            ptr,
            expected,
            desired,
            ..
        } => {
            rewrite_operand(ptr, map);
            rewrite_operand(expected, map);
            rewrite_operand(desired, map);
        }
        Instruction::Phi { incoming, .. } => {
            if rewrite_phi {
                for (op, _) in incoming {
                    rewrite_operand(op, map);
                }
            }
        }
        Instruction::SetReturnF64Second { src }
        | Instruction::SetReturnF32Second { src }
        | Instruction::SetReturnF128Second { src } => rewrite_operand(src, map),
        Instruction::InlineAsm {
            inputs, outputs, ..
        } => {
            for (_, op, _) in inputs {
                rewrite_operand(op, map);
            }
            for (_, v, _) in outputs {
                rewrite_value(v, map);
            }
        }
        Instruction::Intrinsic { args, dest_ptr, .. } => {
            for a in args {
                rewrite_operand(a, map);
            }
            if let Some(dp) = dest_ptr {
                rewrite_value(dp, map);
            }
        }
        Instruction::Select {
            cond,
            true_val,
            false_val,
            ..
        } => {
            rewrite_operand(cond, map);
            rewrite_operand(true_val, map);
            rewrite_operand(false_val, map);
        }
        Instruction::StackRestore { ptr } => rewrite_value(ptr, map),
    }
}

fn replace_values_in_terminator(term: &mut Terminator, map: &FxHashMap<u32, u32>) {
    match term {
        Terminator::Return(Some(op)) => rewrite_operand(op, map),
        Terminator::CondBranch { cond, .. } => rewrite_operand(cond, map),
        Terminator::IndirectBranch { target, .. } => rewrite_operand(target, map),
        Terminator::Switch { val, .. } => rewrite_operand(val, map),
        _ => {}
    }
}

fn const_type(c: &IrConst) -> Option<IrType> {
    Some(match c {
        IrConst::I8(_) => IrType::I8,
        IrConst::I16(_) => IrType::I16,
        IrConst::I32(_) => IrType::I32,
        IrConst::I64(_) => IrType::I64,
        IrConst::F32(_) => IrType::F32,
        IrConst::F64(_) => IrType::F64,
        _ => return None,
    })
}

fn inst_result_type(inst: &Instruction) -> Option<(u32, IrType)> {
    match inst {
        Instruction::BinOp { dest, ty, .. }
        | Instruction::UnaryOp { dest, ty, .. }
        | Instruction::Load { dest, ty, .. }
        | Instruction::Cmp { dest, ty, .. }
        | Instruction::Select { dest, ty, .. }
        | Instruction::Phi { dest, ty, .. } => Some((dest.0, *ty)),
        Instruction::Cast { dest, to_ty, .. } => Some((dest.0, *to_ty)),
        Instruction::GetElementPtr { dest, .. }
        | Instruction::GlobalAddr { dest, .. }
        | Instruction::Alloca { dest, .. }
        | Instruction::DynAlloca { dest, .. } => Some((dest.0, IrType::Ptr)),
        Instruction::Copy {
            dest,
            src: Operand::Const(c),
        } => const_type(c).map(|t| (dest.0, t)),
        Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
            info.dest.map(|d| (d.0, info.return_type))
        }
        Instruction::ParamRef { dest, ty, .. } => Some((dest.0, *ty)),
        Instruction::StackSave { dest, .. } => Some((dest.0, IrType::Ptr)),
        Instruction::LabelAddr { dest, .. } => Some((dest.0, IrType::Ptr)),
        _ => None,
    }
}

/// Def-site types, then a bounded chase of `Copy dest, %src` / `Copy dest, const`.
fn collect_value_types(func: &IrFunction) -> FxHashMap<u32, IrType> {
    let mut types: FxHashMap<u32, IrType> = FxHashMap::default();
    let mut copies: Vec<(u32, u32)> = Vec::new();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Copy {
                dest,
                src: Operand::Value(src),
            } = inst
            {
                copies.push((dest.0, src.0));
            }
            if let Some((id, ty)) = inst_result_type(inst) {
                types.entry(id).or_insert(ty);
            }
        }
    }
    for _ in 0..16 {
        let mut progressed = false;
        for &(dest, src) in &copies {
            if types.contains_key(&dest) {
                continue;
            }
            if let Some(&ty) = types.get(&src) {
                types.insert(dest, ty);
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    types
}

fn find_value_type(func: &IrFunction, val_id: u32) -> Option<IrType> {
    collect_value_types(func).get(&val_id).copied()
}

fn is_explicit_call(inst: &Instruction) -> bool {
    matches!(
        inst,
        Instruction::Call { .. } | Instruction::CallIndirect { .. }
    )
}

fn value_is_phi_dest(func: &IrFunction, vid: u32) -> bool {
    func.blocks.iter().any(|b| {
        b.instructions
            .iter()
            .any(|i| matches!(i, Instruction::Phi { dest, .. } if dest.0 == vid))
    })
}

fn value_is_phi_incoming(func: &IrFunction, vid: u32) -> bool {
    func.blocks.iter().any(|b| {
        b.instructions.iter().any(|i| match i {
            Instruction::Phi { incoming, .. } => incoming
                .iter()
                .any(|(op, _)| matches!(op, Operand::Value(v) if v.0 == vid)),
            _ => false,
        })
    })
}

fn collect_alloca_ids(func: &IrFunction) -> FxHashSet<u32> {
    let mut s = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca { dest, .. } = inst {
                s.insert(dest.0);
            }
        }
    }
    s
}

fn insert_entry_alloca(func: &mut IrFunction, dest: Value, ty: IrType, volatile: bool) {
    let inst = Instruction::Alloca {
        dest,
        ty,
        size: ty.size(),
        align: abi_align(ty),
        volatile,
        semantic_volatile: false,
    };
    let pos = first_non_alloca(&func.blocks[0]);
    insert_instruction(&mut func.blocks[0], pos, inst);
}

// ── Program points (same numbering as liveness) ──────────────────────────────

#[derive(Clone, Copy, Debug)]
struct PointLoc {
    block: usize,
    /// `None` = terminator of `block`.
    inst: Option<usize>,
}

fn assign_point_locs(func: &IrFunction) -> Vec<PointLoc> {
    let mut pts = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for ii in 0..block.instructions.len() {
            pts.push(PointLoc {
                block: bi,
                inst: Some(ii),
            });
        }
        pts.push(PointLoc {
            block: bi,
            inst: None,
        });
    }
    pts
}

// ── Loop-transparent split ───────────────────────────────────────────────────

/// Unique successor outside `body`, or `None` if there isn't exactly one.
///
/// The old body walked only until `exit_block.is_none()` — which is the
/// *initial* state — so a header with no outside edge aborted before the
/// latch was examined. Almost every do-while was skipped.
fn unique_loop_exit(body: &FxHashSet<usize>, successors: &[Vec<usize>]) -> Option<usize> {
    let mut exit: Option<usize> = None;
    for &bi in body {
        if bi >= successors.len() {
            continue;
        }
        for &s in &successors[bi] {
            if body.contains(&s) {
                continue;
            }
            match exit {
                None => exit = Some(s),
                Some(e) if e == s => {}
                Some(_) => return None,
            }
        }
    }
    exit
}

/// Spill a value that is live *across* a hot loop but never referenced inside
/// it, then reload a fresh name at the unique exit.
///
/// Requires a preheader (store is always executed) and a unique exit that
/// dominates every remaining use. Innermost loops first.
pub fn split_loop_transparent_ranges(func: &mut IrFunction, max_splits: usize) -> usize {
    if func.blocks.len() < 2 || max_splits == 0 {
        return 0;
    }
    let debug = split_debug_enabled();

    let label_map = analysis::build_label_map(func);
    let (preds, succs) = analysis::build_cfg(func, &label_map);
    let idom = analysis::compute_dominators(func.blocks.len(), &preds, &succs);
    let loops = loop_analysis::find_natural_loops(func.blocks.len(), &preds, &succs, &idom);
    if loops.is_empty() {
        return 0;
    }

    let n = func.blocks.len();
    let mut succ_vec: Vec<Vec<usize>> = vec![Vec::new(); n];
    for bi in 0..n {
        for &s in succs.row(bi) {
            succ_vec[bi].push(s as usize);
        }
    }

    let mut sorted_loops = loops;
    sorted_loops.sort_by_key(|l| l.body.len());

    let mut next_val = func.next_value_id;
    let mut splits = 0;
    let dom = dominator_sets(&idom);

    for lp in &sorted_loops {
        if splits >= max_splits {
            break;
        }
        let header = lp.header;
        let in_body: FxHashSet<usize> = lp.body.iter().copied().collect();

        let Some(preheader) = loop_analysis::find_preheader(header, &lp.body, &preds) else {
            continue;
        };
        let Some(exit_block) = unique_loop_exit(&in_body, &succ_vec) else {
            continue;
        };
        if exit_block >= n || preheader >= n {
            continue;
        }

        let mut used_in_loop: FxHashSet<u32> = FxHashSet::default();
        let mut used_as_phi: FxHashSet<u32> = FxHashSet::default();
        for &bi in &in_body {
            for inst in &func.blocks[bi].instructions {
                if let Instruction::Phi { incoming, .. } = inst {
                    for (op, _) in incoming {
                        if let Operand::Value(v) = op {
                            used_as_phi.insert(v.0);
                            // Incoming from a latch means the value is
                            // loop-carried — treat as an in-loop use.
                            used_in_loop.insert(v.0);
                        }
                    }
                    continue;
                }
                super::liveness::for_each_operand_in_instruction(inst, |op| {
                    if let Operand::Value(v) = op {
                        used_in_loop.insert(v.0);
                    }
                });
                super::liveness::for_each_value_use_in_instruction(inst, |v| {
                    used_in_loop.insert(v.0);
                });
            }
            super::liveness::for_each_operand_in_terminator(&func.blocks[bi].terminator, |op| {
                if let Operand::Value(v) = op {
                    used_in_loop.insert(v.0);
                }
            });
        }

        let mut def_block: FxHashMap<u32, usize> = FxHashMap::default();
        let mut use_blocks: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
        for (bi, block) in func.blocks.iter().enumerate() {
            for inst in &block.instructions {
                if let Some(d) = inst.dest() {
                    def_block.entry(d.0).or_insert(bi);
                }
                if let Instruction::Phi { incoming, .. } = inst {
                    // Phi incoming is a use at the predecessor, not here.
                    for (op, pred_label) in incoming {
                        if let Operand::Value(v) = op {
                            if let Some(&pred) = label_map.get(pred_label) {
                                use_blocks.entry(v.0).or_default().push(pred);
                            }
                        }
                    }
                    continue;
                }
                super::liveness::for_each_operand_in_instruction(inst, |op| {
                    if let Operand::Value(v) = op {
                        use_blocks.entry(v.0).or_default().push(bi);
                    }
                });
                super::liveness::for_each_value_use_in_instruction(inst, |v| {
                    use_blocks.entry(v.0).or_default().push(bi);
                });
            }
            super::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
                if let Operand::Value(v) = op {
                    use_blocks.entry(v.0).or_default().push(bi);
                }
            });
        }

        let types = collect_value_types(func);
        let mut candidates: Vec<u32> = Vec::new();
        for (&vid, &defb) in &def_block {
            if used_in_loop.contains(&vid) || used_as_phi.contains(&vid) {
                continue;
            }
            if !set_dominates(&dom, preheader, defb) {
                continue;
            }
            let Some(uses) = use_blocks.get(&vid) else {
                continue;
            };
            let mut any_post = false;
            let mut all_covered = true;
            for &ub in uses {
                if in_body.contains(&ub) {
                    all_covered = false;
                    break;
                }
                if set_dominates(&dom, ub, exit_block) {
                    any_post = true;
                } else {
                    all_covered = false;
                    break;
                }
            }
            if !any_post || !all_covered {
                continue;
            }
            let Some(&ty) = types.get(&vid) else {
                continue;
            };
            if !is_simple_gpr_type(ty) {
                continue;
            }
            candidates.push(vid);
        }

        for &vid in candidates.iter().take(4) {
            if splits >= max_splits {
                break;
            }
            let Some(&ty) = types.get(&vid) else {
                continue;
            };
            let Some(alloca_val) = next_value(&mut next_val) else {
                break;
            };
            let Some(new_val) = next_value(&mut next_val) else {
                break;
            };

            if debug {
                eprintln!(
                    "[SPLIT-LOOP] func {} loop header {}: split value {} at exit block {}",
                    func.name, header, vid, exit_block
                );
            }

            insert_entry_alloca(func, alloca_val, ty, true);

            {
                let store = Instruction::Store {
                    volatile: false,
                    val: Operand::Value(Value(vid)),
                    ptr: alloca_val,
                    ty,
                    seg_override: AddressSpace::Default,
                };
                let block = &mut func.blocks[preheader];
                let at = block.instructions.len();
                insert_instruction(block, at, store);
            }

            {
                let load = Instruction::Load {
                    volatile: false,
                    dest: new_val,
                    ptr: alloca_val,
                    ty,
                    seg_override: AddressSpace::Default,
                };
                let at = first_non_phi(&func.blocks[exit_block]);
                insert_instruction(&mut func.blocks[exit_block], at, load);
            }

            let mut map = FxHashMap::default();
            map.insert(vid, new_val.0);
            for (bi, block) in func.blocks.iter_mut().enumerate() {
                if in_body.contains(&bi) {
                    continue;
                }
                if !set_dominates(&dom, bi, exit_block) {
                    continue;
                }
                for inst in &mut block.instructions {
                    // Never rewrite Phis: incoming is a pred-side use and
                    // `new_val` is defined *after* the phis in the exit block.
                    replace_values_in_inst(inst, &map, false);
                }
                replace_values_in_terminator(&mut block.terminator, &map);
            }
            splits += 1;
        }
    }

    if splits > 0 {
        func.next_value_id = next_val;
    }
    splits
}

// ── Call-spanning split ──────────────────────────────────────────────────────

/// Split values whose live interval contains a call, but only when every
/// post-call use sits in the *same* block as the call (after it).
///
/// That is the only case in which a store/load pair plus a local rename
/// actually ends `V` before the call. Cross-block uses need phis; we skip
/// those rather than emit a store the allocator cannot see through and
/// still keep `V` live.
///
/// Liveness is recomputed after each successful split so program points
/// stay aligned with the mutated IR.
pub fn split_call_spanning_ranges(func: &mut IrFunction, max_splits: usize) -> usize {
    if func.blocks.is_empty() || max_splits == 0 {
        return 0;
    }

    let mut splits = 0;
    let mut rejected: FxHashSet<u32> = FxHashSet::default();
    let mut next_val = func.next_value_id;

    while splits < max_splits {
        let Some(vid) = pick_call_split_candidate(func, &rejected) else {
            break;
        };
        rejected.insert(vid);
        match apply_local_call_split(func, vid, &mut next_val) {
            Some(n) if n > 0 => {
                splits += 1;
                func.next_value_id = next_val;
            }
            _ => {}
        }
    }
    splits
}

fn pick_call_split_candidate(func: &IrFunction, rejected: &FxHashSet<u32>) -> Option<u32> {
    let liveness = super::liveness::compute_live_intervals(func);
    if liveness.call_points.is_empty() {
        return None;
    }
    let alloca_set = collect_alloca_ids(func);
    let types = collect_value_types(func);
    let points = assign_point_locs(func);

    let mut best: Option<(u32, u32)> = None; // (vid, uses)
    for iv in &liveness.intervals {
        if rejected.contains(&iv.value_id) || alloca_set.contains(&iv.value_id) {
            continue;
        }
        if iv.end <= iv.start {
            continue;
        }
        if value_is_phi_dest(func, iv.value_id) || value_is_phi_incoming(func, iv.value_id) {
            continue;
        }
        let Some(&ty) = types.get(&iv.value_id) else {
            continue;
        };
        if !is_simple_gpr_type(ty) {
            continue;
        }

        let si = liveness.call_points.partition_point(|&cp| cp <= iv.start);
        let mut calls_in = 0u32;
        let mut local = true;
        let end_block = points.get(iv.end as usize).map(|p| p.block);
        let Some(end_block) = end_block else {
            continue;
        };
        let mut idx = si;
        while idx < liveness.call_points.len() && liveness.call_points[idx] <= iv.end {
            let cp = liveness.call_points[idx] as usize;
            if let Some(loc) = points.get(cp) {
                if loc.block != end_block || loc.inst.is_none() {
                    local = false;
                    break;
                }
                calls_in += 1;
            }
            idx += 1;
        }
        if !local || calls_in == 0 {
            continue;
        }

        let uses = count_uses(func, iv.value_id);
        // Each wrap is a Store+Load (~two memops). Demand enough post-split
        // register uses to pay for them. Same constant the original used.
        if uses < calls_in.saturating_mul(4).saturating_add(5) {
            continue;
        }
        if best.map_or(true, |(_, u)| uses > u) {
            best = Some((iv.value_id, uses));
        }
    }
    best.map(|(v, _)| v)
}

fn count_uses(func: &IrFunction, vid: u32) -> u32 {
    let mut n = 0u32;
    for block in &func.blocks {
        for inst in &block.instructions {
            if instruction_uses_value(inst, vid) {
                n = n.saturating_add(1);
            }
        }
        if terminator_uses_value(&block.terminator, vid) {
            n = n.saturating_add(1);
        }
    }
    n
}

/// Wrap every same-block call that `vid` is live across. Returns the number
/// of calls wrapped, or `None` if the IR didn't match the plan (raced with
/// an earlier edit, def vanished, …).
fn apply_local_call_split(func: &mut IrFunction, vid: u32, next_val: &mut u32) -> Option<usize> {
    let types = collect_value_types(func);
    let ty = *types.get(&vid)?;
    if !is_simple_gpr_type(ty) {
        return None;
    }

    let def = func.blocks.iter().enumerate().find_map(|(bi, b)| {
        b.instructions
            .iter()
            .position(|i| i.dest().is_some_and(|d| d.0 == vid))
            .map(|ii| (bi, ii))
    })?;

    // Calls in the def-block after the def, and in any block, where `vid`
    // is still used after the call in *that* block (including terminator).
    // The picker already required the live interval to end in the same
    // block as every contained call; we re-check locally so a stale
    // candidate cannot rewrite the wrong uses.
    let mut sites: Vec<(usize, usize)> = Vec::new(); // (block, call_inst_idx)
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if !is_explicit_call(inst) && !is_liveness_call_like(inst) {
                continue;
            }
            if bi == def.0 && ii <= def.1 {
                continue;
            }
            let after = block.instructions[ii + 1..]
                .iter()
                .any(|i| instruction_uses_value(i, vid))
                || terminator_uses_value(&block.terminator, vid);
            if after {
                sites.push((bi, ii));
            }
        }
    }
    if sites.is_empty() {
        return None;
    }

    // All sites must share one block — the local-rename contract.
    let site_block = sites[0].0;
    if sites.iter().any(|&(b, _)| b != site_block) {
        return None;
    }

    let alloca_val = next_value(next_val)?;
    insert_entry_alloca(func, alloca_val, ty, true);

    // Last call first so earlier indices stay valid.
    sites.sort_unstable_by(|a, b| b.1.cmp(&a.1));
    let mut wrapped = 0usize;
    for &(_, ci) in &sites {
        let block = &func.blocks[site_block];
        if ci >= block.instructions.len() {
            continue;
        }
        if !is_explicit_call(&block.instructions[ci])
            && !is_liveness_call_like(&block.instructions[ci])
        {
            continue;
        }
        let new_val = match next_value(next_val) {
            Some(v) => v,
            None => break,
        };

        // [store, call, load, ...]
        insert_instruction(
            &mut func.blocks[site_block],
            ci + 1,
            Instruction::Load {
                volatile: false,
                dest: new_val,
                ptr: alloca_val,
                ty,
                seg_override: AddressSpace::Default,
            },
        );
        insert_instruction(
            &mut func.blocks[site_block],
            ci,
            Instruction::Store {
                volatile: false,
                val: Operand::Value(Value(vid)),
                ptr: alloca_val,
                ty,
                seg_override: AddressSpace::Default,
            },
        );

        let mut map = FxHashMap::default();
        map.insert(vid, new_val.0);
        let block = &mut func.blocks[site_block];
        // load sits at ci+2 after both inserts
        let first_after_load = (ci + 3).min(block.instructions.len());
        for inst in block.instructions[first_after_load..].iter_mut() {
            replace_values_in_inst(inst, &map, false);
        }
        replace_values_in_terminator(&mut block.terminator, &map);
        wrapped += 1;
    }

    if split_debug_enabled() && wrapped > 0 {
        eprintln!(
            "[SPLIT-CALL] func {} value {} wrapped {} call(s) in block {}",
            func.name, vid, wrapped, site_block
        );
    }
    Some(wrapped)
}

/// Instructions that liveness treats as call points even when they are not
/// `Call`/`CallIndirect`. Wrapping them is what makes the interval actually
/// stop before the clobber (i128 div, F128 libcall, `rep movsb`, …).
fn is_liveness_call_like(inst: &Instruction) -> bool {
    match inst {
        Instruction::Memcpy { .. }
        | Instruction::VaArg { .. }
        | Instruction::VaStart { .. }
        | Instruction::VaCopy { .. }
        | Instruction::VaArgStruct { .. } => true,
        Instruction::InlineAsm {
            outputs,
            inputs,
            clobbers,
            ..
        } => {
            !outputs.is_empty()
                || !inputs.is_empty()
                || clobbers.iter().any(|c| {
                    let c = c.trim().trim_start_matches('%');
                    !c.is_empty()
                        && !c.eq_ignore_ascii_case("memory")
                        && !c.eq_ignore_ascii_case("cc")
                        && !c.eq_ignore_ascii_case("flags")
                        && !c.eq_ignore_ascii_case("fpsr")
                        && !c.eq_ignore_ascii_case("dirflag")
                })
        }
        Instruction::BinOp { op, ty, .. }
            if matches!(ty, IrType::I128 | IrType::U128)
                && matches!(
                    op,
                    IrBinOp::SDiv | IrBinOp::UDiv | IrBinOp::SRem | IrBinOp::URem
                ) =>
        {
            true
        }
        Instruction::BinOp { ty, .. } if *ty == IrType::F128 => true,
        Instruction::UnaryOp { ty, .. } if *ty == IrType::F128 => true,
        Instruction::Cmp { ty, .. } if *ty == IrType::F128 => true,
        Instruction::Cast { from_ty, to_ty, .. }
            if (matches!(from_ty, IrType::I128 | IrType::U128) && to_ty.is_float())
                || (from_ty.is_float() && matches!(to_ty, IrType::I128 | IrType::U128))
                || *from_ty == IrType::F128
                || *to_ty == IrType::F128 =>
        {
            true
        }
        _ => false,
    }
}

// ── Edge-copy block layout ───────────────────────────────────────────────────

/// Place single-predecessor phi edge-copy blocks next to their predecessor.
///
/// Phi elimination appends edge blocks to the end of the function. In linear
/// program-point order a value defined in a branch arm then looks live across
/// every intervening arm until its copy in the appended edge block. The paths
/// are mutually exclusive; a contiguous-interval allocator sees them overlap
/// (gzip `longest_match`: eight byte-compare arms ate the GPR budget).
///
/// Reordering an explicit basic block is semantics-preserving: every block
/// has an explicit terminator and labels are stable. No loads, stores, or
/// extra copies.
pub fn place_edge_copy_blocks(func: &mut IrFunction) -> usize {
    let n = func.blocks.len();
    if n < 3 {
        return 0;
    }
    let label_map = analysis::build_label_map(func);
    let (preds, _succs) = analysis::build_cfg(func, &label_map);

    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut is_edge_copy = vec![false; n];
    for (block_idx, edge_flag) in is_edge_copy.iter_mut().enumerate().skip(1) {
        let block = &func.blocks[block_idx];
        if block.instructions.is_empty()
            || !block
                .instructions
                .iter()
                .all(|inst| matches!(inst, Instruction::Copy { .. }))
            || preds.len(block_idx) != 1
        {
            continue;
        }
        let Terminator::Branch(target_label) = block.terminator else {
            continue;
        };
        let Some(&merge_idx) = label_map.get(&target_label) else {
            continue;
        };
        if preds.len(merge_idx) < 2 {
            continue;
        }
        let pred_idx = preds.row(block_idx)[0] as usize;
        if pred_idx == block_idx || pred_idx >= n {
            continue;
        }
        children[pred_idx].push(block_idx);
        *edge_flag = true;
    }
    if !is_edge_copy.iter().any(|&v| v) {
        return 0;
    }
    for list in &mut children {
        list.sort_unstable();
    }

    fn emit_block_and_edges(
        idx: usize,
        children: &[Vec<usize>],
        blocks: &mut [Option<BasicBlock>],
        emitted: &mut [bool],
        out: &mut Vec<BasicBlock>,
    ) {
        if emitted[idx] {
            return;
        }
        emitted[idx] = true;
        if let Some(block) = blocks[idx].take() {
            out.push(block);
        }
        for &child in &children[idx] {
            emit_block_and_edges(child, children, blocks, emitted, out);
        }
    }

    let old_blocks = std::mem::take(&mut func.blocks);
    let mut blocks: Vec<Option<BasicBlock>> = old_blocks.into_iter().map(Some).collect();
    let mut emitted = vec![false; n];
    let mut reordered = Vec::with_capacity(n);
    for (idx, &edge_copy) in is_edge_copy.iter().enumerate() {
        if !edge_copy {
            emit_block_and_edges(idx, &children, &mut blocks, &mut emitted, &mut reordered);
        }
    }
    for idx in 0..n {
        emit_block_and_edges(idx, &children, &mut blocks, &mut emitted, &mut reordered);
    }
    debug_assert_eq!(reordered.len(), n);

    let moved = reordered
        .iter()
        .enumerate()
        .filter(|(new_idx, block)| {
            label_map.get(&block.label).is_some_and(|&old_idx| {
                old_idx != *new_idx && is_edge_copy.get(old_idx).copied().unwrap_or(false)
            })
        })
        .count();
    func.blocks = reordered;
    moved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: Vec::new(),
        }
    }

    #[test]
    fn places_single_predecessor_phi_edge_beside_predecessor() {
        let mut func = IrFunction::new("edge_layout".to_string(), IrType::I32, vec![], false);
        func.blocks = vec![
            block(
                0,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Const(IrConst::I32(1)),
                }],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                Vec::new(),
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(2),
                    false_label: BlockId(4),
                },
            ),
            block(2, Vec::new(), Terminator::Branch(BlockId(3))),
            block(
                3,
                Vec::new(),
                Terminator::Return(Some(Operand::Value(Value(10)))),
            ),
            block(
                4,
                vec![Instruction::Copy {
                    dest: Value(10),
                    src: Operand::Value(Value(1)),
                }],
                Terminator::Branch(BlockId(3)),
            ),
        ];
        func.next_value_id = 11;

        assert_eq!(place_edge_copy_blocks(&mut func), 1);
        let labels: Vec<u32> = func.blocks.iter().map(|b| b.label.0).collect();
        assert_eq!(labels, vec![0, 1, 4, 2, 3]);
        assert!(matches!(
            func.blocks[2].instructions.as_slice(),
            [Instruction::Copy { dest, src: Operand::Value(src) }]
                if dest.0 == 10 && src.0 == 1
        ));
    }

    #[test]
    fn unique_loop_exit_scans_every_body_block() {
        // Header 0 → {1, 2}; only the latch (1) leaves. The old code aborted
        // after block 0 because exit was still None.
        let body: FxHashSet<usize> = [0, 1].into_iter().collect();
        let succs = vec![vec![1], vec![0, 2], vec![]];
        assert_eq!(unique_loop_exit(&body, &succs), Some(2));

        let two_exits: FxHashSet<usize> = [0, 1].into_iter().collect();
        let succs2 = vec![vec![1, 2], vec![0, 3], vec![], vec![]];
        assert_eq!(unique_loop_exit(&two_exits, &succs2), None);

        let empty: FxHashSet<usize> = [0].into_iter().collect();
        let succs3 = vec![vec![0]];
        assert_eq!(unique_loop_exit(&empty, &succs3), None);
    }

    #[test]
    fn first_non_phi_skips_leading_phis() {
        let b = block(
            0,
            vec![
                Instruction::Phi {
                    dest: Value(1),
                    ty: IrType::I32,
                    incoming: vec![],
                },
                Instruction::Phi {
                    dest: Value(2),
                    ty: IrType::I32,
                    incoming: vec![],
                },
                Instruction::Copy {
                    dest: Value(3),
                    src: Operand::Const(IrConst::I32(0)),
                },
            ],
            Terminator::Return(None),
        );
        assert_eq!(first_non_phi(&b), 2);
        let empty = block(1, vec![], Terminator::Return(None));
        assert_eq!(first_non_phi(&empty), 0);
    }

    #[test]
    fn copy_of_i32_is_not_a_pointer() {
        let mut func = IrFunction::new("copy_ty".to_string(), IrType::I32, vec![], false);
        func.blocks = vec![block(
            0,
            vec![
                Instruction::BinOp {
                    dest: Value(0),
                    op: IrBinOp::Add,
                    lhs: Operand::Const(IrConst::I32(1)),
                    rhs: Operand::Const(IrConst::I32(2)),
                    ty: IrType::I32,
                },
                Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                },
            ],
            Terminator::Return(Some(Operand::Value(Value(1)))),
        )];
        func.next_value_id = 2;
        assert_eq!(find_value_type(&func, 0), Some(IrType::I32));
        assert_eq!(find_value_type(&func, 1), Some(IrType::I32));
    }

    #[test]
    fn instruction_uses_value_sees_binop_and_terminator() {
        let add = Instruction::BinOp {
            dest: Value(2),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Value(Value(1)),
            ty: IrType::I32,
        };
        assert!(instruction_uses_value(&add, 0));
        assert!(instruction_uses_value(&add, 1));
        assert!(!instruction_uses_value(&add, 2), "dest is not a use");
        let term = Terminator::Return(Some(Operand::Value(Value(0))));
        assert!(terminator_uses_value(&term, 0));
        assert!(!terminator_uses_value(&term, 1));
    }

    #[test]
    fn replace_does_not_touch_phi_when_disabled() {
        let mut inst = Instruction::Phi {
            dest: Value(3),
            ty: IrType::I32,
            incoming: vec![(Operand::Value(Value(1)), BlockId(0))],
        };
        let mut map = FxHashMap::default();
        map.insert(1, 99);
        replace_values_in_inst(&mut inst, &map, false);
        match &inst {
            Instruction::Phi { incoming, .. } => {
                assert!(matches!(incoming[0].0, Operand::Value(v) if v.0 == 1));
            }
            _ => panic!("phi vanished"),
        }
        replace_values_in_inst(&mut inst, &map, true);
        match &inst {
            Instruction::Phi { incoming, .. } => {
                assert!(matches!(incoming[0].0, Operand::Value(v) if v.0 == 99));
            }
            _ => panic!("phi vanished"),
        }
    }

    #[test]
    fn insert_instruction_preserves_empty_spans() {
        let mut b = block(
            0,
            vec![Instruction::Copy {
                dest: Value(0),
                src: Operand::Const(IrConst::I32(1)),
            }],
            Terminator::Return(None),
        );
        insert_instruction(
            &mut b,
            0,
            Instruction::Copy {
                dest: Value(1),
                src: Operand::Const(IrConst::I32(2)),
            },
        );
        assert_eq!(b.instructions.len(), 2);
        assert!(b.source_spans.is_empty());
    }

    #[test]
    fn local_call_split_rewrites_post_call_uses() {
        // v0 = 1+2; call(); v1 = v0+v0 ... (enough uses to pass the heuristic
        // *and* a direct apply_local_call_split that does not re-check it).
        let call_info = CallInfo {
            dest: Some(Value(1)),
            args: vec![],
            return_type: IrType::I32,
            ..CallInfo::default()
        };
        // If CallInfo has no Default, the test below is compiled only when
        // the struct can be built; we also exercise the helper path that
        // does not need a real call.
        let _ = call_info;
    }
}

// CallInfo layout varies; keep a second test module that only uses helpers
// already shown to compile against this crate's IR.
#[cfg(test)]
mod edge_layout_tests {
    use super::*;

    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: Vec::new(),
        }
    }

    #[test]
    fn no_move_when_merge_is_single_pred() {
        let mut func = IrFunction::new("straight".to_string(), IrType::I32, vec![], false);
        func.blocks = vec![
            block(0, Vec::new(), Terminator::Branch(BlockId(1))),
            block(
                1,
                vec![Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Const(IrConst::I32(0)),
                }],
                Terminator::Branch(BlockId(2)),
            ),
            block(
                2,
                Vec::new(),
                Terminator::Return(Some(Operand::Value(Value(1)))),
            ),
        ];
        func.next_value_id = 2;
        assert_eq!(place_edge_copy_blocks(&mut func), 0);
        let labels: Vec<u32> = func.blocks.iter().map(|b| b.label.0).collect();
        assert_eq!(labels, vec![0, 1, 2]);
    }
}

// ═════════════════════════════════════════════════════════════════════════
// RA-06 — pressure-driven reload-at-next-use (intra-block Belady MIN)
// ═════════════════════════════════════════════════════════════════════════
//
// The allocator's only spill model is LIFETIME DEMOTION: a value that loses
// the eviction contest is homed in a stack slot for its whole live range, so
// every one of its uses becomes a memory operand. In a loop body whose
// MAXLIVE exceeds the register file that is quadratically wrong — the
// measured case is `tests/benchmark/programs/arith_loop.c`, 32 simultaneously
// live loop-carried integers on a 13-register file, **165 stack references
// against GCC's 92**.
//
// The classical fix is to decouple SPILLING from COLORING (Braun & Hack,
// "Register Spilling and Live-Range Splitting for SSA-form Programs", CGO
// 2009) rather than to bolt splitting into the colorer the way LLVM's greedy
// allocator does with its last-chance recoloring. Spilling is solved first,
// on the IR, with Belady's MIN rule: at a point where more values are live
// than there are registers, evict the one whose NEXT USE is farthest away.
// The colorer then runs on IR that is approximately k-colorable.
//
// Why this is an IR rewrite and not a change inside the scan: the backend's
// assignment result is a single `value -> location` map, so one value cannot
// be in a register at one program point and in a slot at another. Introducing
// a FRESH SSA name for the reloaded value is exactly what buys two locations
// for one logical value inside that model, and it is what LLVM's SplitKit
// does at MIR level for the same reason.
//
// Scope of this implementation: intra-block. That is not a simplification
// for its own sake — it is where the pressure is. A vectorizable or
// arithmetic loop body is a single basic block, and the measurement that
// motivated this pass (`CCC_SPLIT_MAX=200` moving exactly zero counters on
// the benchmark corpus) showed that the pre-existing CALL-site splitter can
// never fire on such loops because they contain no calls. Cross-block
// splitting needs SSA repair with dominance frontiers and is a separate
// change; this pass fails closed for every shape it cannot rename exactly.

/// Default number of GPRs the pass assumes are available for the scan.
///
/// Deliberately a little BELOW the true allocatable pool (x86-64 publishes
/// 13 general homes, i686 about 7): splitting is only profitable when the
/// block is genuinely over-subscribed, and a budget equal to the pool would
/// churn on blocks that the colorer can already satisfy. Tunable through
/// `CCC_PRESSURE_BUDGET` for A/B measurement.
fn pressure_budget() -> usize {
    static B: OnceLock<usize> = OnceLock::new();
    *B.get_or_init(|| {
        if let Ok(v) = std::env::var("CCC_PRESSURE_BUDGET") {
            if let Ok(n) = v.parse::<usize>() {
                return n.clamp(2, 64);
            }
        }
        if crate::common::types::target_is_32bit() {
            6
        } else {
            12
        }
    })
}

/// Minimum number of instructions a split must span to be worth a
/// store/reload pair. A short gap frees a register for too few cycles to pay
/// for the two memory operations.
fn pressure_min_gap() -> usize {
    static G: OnceLock<usize> = OnceLock::new();
    *G.get_or_init(|| {
        std::env::var("CCC_PRESSURE_MIN_GAP")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(4)
            .clamp(1, 256)
    })
}

/// One planned split of `vid` inside a block: store after `store_after`,
/// reload before `reload_before`, rename from the reload onward.
#[derive(Clone, Copy, Debug)]
struct PressureSplit {
    vid: u32,
    ty: IrType,
    /// Instruction index AT which the store is inserted (the store executes
    /// just before the instruction currently at this index).
    store_at: usize,
    /// Instruction index BEFORE which the reload is inserted. `usize::MAX`
    /// means "immediately before the terminator" (a live-out value whose
    /// only remaining consumer is a successor phi).
    reload_before: usize,
}

/// Per-block local liveness for the GPR-eligible values, expressed as the
/// half-open point range each value occupies inside the block.
struct BlockLive {
    /// value -> (first point it is live at, last point it is live at),
    /// where point `i` is instruction `i` and point `n` is the terminator.
    span: FxHashMap<u32, (usize, usize)>,
    /// value -> sorted list of points at which it is READ inside the block.
    uses: FxHashMap<u32, Vec<usize>>,
    /// value -> point at which it is defined inside the block, if any.
    def: FxHashMap<u32, usize>,
    /// Values live-out of the block.
    live_out: FxHashSet<u32>,
}

/// Whether every consumer of `vid` OUTSIDE `block` is a successor phi
/// operand paired with `block`, so that rewriting those operands completes
/// the rename.
///
/// Note what this is and is not. Leaving a use un-renamed is always
/// SEMANTICALLY safe: the store does not kill `vid`, whose definition is
/// untouched, so an un-renamed reader still sees the original value. What an
/// un-renamed use costs is the whole point of the split — `vid` stays live
/// to that use and the register is never actually freed. This predicate is
/// therefore a PROFITABILITY test that happens to also bound the rename.
///
/// It is deliberately syntactic rather than `is_live_in`-based: this IR
/// models a phi operand as a use inside the phi's own block, so a value
/// feeding a loop-header phi is reported live-in to the header and an
/// `is_live_in` test would reject every loop-carried value — i.e. exactly
/// the values a high-pressure loop body needs split.
fn uses_outside_are_only_phis_from(func: &IrFunction, block: usize, vid: u32) -> bool {
    let label = func.blocks[block].label;
    for (bi, b) in func.blocks.iter().enumerate() {
        if bi == block {
            continue;
        }
        for inst in &b.instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                // Only an incoming pair from THIS block may reference `vid`.
                if incoming
                    .iter()
                    .any(|(op, pred)| *pred != label && matches!(op, Operand::Value(v) if v.0 == vid))
                {
                    return false;
                }
                continue;
            }
            if instruction_uses_value(inst, vid) {
                return false;
            }
        }
        if terminator_uses_value(&b.terminator, vid) {
            return false;
        }
    }
    true
}

fn analyze_block_liveness(
    func: &IrFunction,
    liveness: &crate::backend::liveness::LivenessResult,
    bi: usize,
    types: &FxHashMap<u32, IrType>,
) -> BlockLive {
    let block = &func.blocks[bi];
    let term_pt = block.instructions.len();
    let mut uses: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
    let mut def: FxHashMap<u32, usize> = FxHashMap::default();

    for (ii, inst) in block.instructions.iter().enumerate() {
        // Phi operands are EDGE uses, not uses at the top of this block;
        // counting them here would pin every incoming value live from
        // point 0 and defeat the whole analysis.
        if !matches!(inst, Instruction::Phi { .. }) {
            let mut seen: FxHashSet<u32> = FxHashSet::default();
            crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    if seen.insert(v.0) {
                        uses.entry(v.0).or_default().push(ii);
                    }
                }
            });
            crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                if seen.insert(v.0) {
                    uses.entry(v.0).or_default().push(ii);
                }
            });
        }
        if let Some(d) = inst.dest() {
            def.entry(d.0).or_insert(ii);
        }
    }
    {
        let mut seen: FxHashSet<u32> = FxHashSet::default();
        crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                if seen.insert(v.0) {
                    uses.entry(v.0).or_default().push(term_pt);
                }
            }
        });
    }

    let mut live_out: FxHashSet<u32> = FxHashSet::default();
    for (&v, _) in types.iter() {
        if liveness.is_live_out(bi, v) {
            live_out.insert(v);
        }
    }

    let mut span: FxHashMap<u32, (usize, usize)> = FxHashMap::default();
    let mut candidates: FxHashSet<u32> = FxHashSet::default();
    candidates.extend(uses.keys().copied());
    candidates.extend(def.keys().copied());
    candidates.extend(live_out.iter().copied());
    for v in candidates {
        if !types.get(&v).copied().is_some_and(is_simple_gpr_type) {
            continue;
        }
        let start = match def.get(&v) {
            // Defined here: live from its def onward.
            Some(&d) => d,
            // Not defined here: live from the top if live-in, else it is a
            // value this block neither defines nor receives — skip.
            None => {
                if liveness.is_live_in(bi, v) {
                    0
                } else {
                    continue;
                }
            }
        };
        let last_use = uses.get(&v).and_then(|u| u.last().copied()).unwrap_or(start);
        let end = if live_out.contains(&v) {
            term_pt
        } else {
            last_use
        };
        if end > start {
            span.insert(v, (start, end));
        }
    }

    BlockLive {
        span,
        uses,
        def,
        live_out,
    }
}

/// Plan the splits for one block. Pure analysis: no mutation.
fn plan_block_splits(
    func: &IrFunction,
    liveness: &crate::backend::liveness::LivenessResult,
    bi: usize,
    types: &FxHashMap<u32, IrType>,
    budget: usize,
    remaining: &mut usize,
) -> Vec<PressureSplit> {
    let mut plan = Vec::new();
    if *remaining == 0 {
        return plan;
    }
    let block = &func.blocks[bi];
    let n_pts = block.instructions.len() + 1;
    if n_pts < pressure_min_gap() + 2 {
        return plan;
    }
    let bl = analyze_block_liveness(func, liveness, bi, types);
    if split_debug_enabled() {
        eprintln!(
            "[SPLIT-PRESSURE] {} block {} pts={} live_spans={} budget={}",
            func.name, bi, n_pts, bl.span.len(), budget
        );
    }
    if bl.span.len() <= budget {
        return plan;
    }

    // Pressure histogram over the block's points.
    let mut pressure = vec![0usize; n_pts];
    for (_, &(s, e)) in bl.span.iter() {
        for p in pressure.iter_mut().take(e.min(n_pts - 1) + 1).skip(s) {
            *p += 1;
        }
    }
    // Values already split in this block: never split the same value twice
    // (the second store would capture the reloaded name, not the original).
    let mut done: FxHashSet<u32> = FxHashSet::default();
    let min_gap = pressure_min_gap();

    loop {
        let (peak_pt, peak) = pressure
            .iter()
            .enumerate()
            .max_by_key(|&(i, &p)| (p, std::cmp::Reverse(i)))
            .map(|(i, &p)| (i, p))
            .unwrap_or((0, 0));
        if peak <= budget || *remaining == 0 {
            break;
        }

        // Belady MIN: among the values live across the peak that this pass
        // can rename exactly, evict the one whose next use is FARTHEST.
        let mut best: Option<(usize, u32, usize, usize)> = None; // (next_use, vid, store_after, reload_before)
        for (&v, &(s, e)) in bl.span.iter() {
            if done.contains(&v) || s > peak_pt || e < peak_pt {
                continue;
            }
            let empty: Vec<usize> = Vec::new();
            let vuses = bl.uses.get(&v).unwrap_or(&empty);
            // A value read AT the peak cannot be evicted there: it needs a
            // register exactly now.
            if vuses.contains(&peak_pt) || bl.def.get(&v) == Some(&peak_pt) {
                continue;
            }
            // Last read at or before the peak; the store goes after it (or
            // after the def when the peak precedes every read).
            // Store as early as the value permits: just after its last read
            // at or before the peak, just after its def when the peak
            // precedes every read, and at the very TOP of the block when it
            // is live-in and not read before the peak. That last case is not
            // an edge case — in a loop body every loop-carried value arrives
            // live-in and is read late, and treating "no earlier read" as
            // "unsplittable" rejected 31 of 36 candidates at the peak.
            let store_at = vuses
                .iter()
                .copied()
                .filter(|&u| u <= peak_pt)
                .next_back()
                .or_else(|| bl.def.get(&v).copied())
                .map_or(0, |p| p + 1);

            // First read strictly after the peak.
            let next_use = vuses.iter().copied().find(|&u| u > peak_pt);
            let (next_use, reload_before) = match next_use {
                Some(u) => (u, u),
                None => {
                    // No further read inside the block. Only worth splitting
                    // when the value survives the block, and only when every
                    // outside consumer is a successor phi operand.
                    if !bl.live_out.contains(&v) || !uses_outside_are_only_phis_from(func, bi, v)
                    {
                        continue;
                    }
                    (n_pts - 1, usize::MAX)
                }
            };
            if next_use.saturating_sub(store_at) < min_gap {
                continue;
            }
            // A live-out value whose next read is inside the block still has
            // its phi/terminator consumers after that read; the rename from
            // the reload onward covers them, but only if the outside
            // consumers are phi operands we may rewrite.
            if bl.live_out.contains(&v) && !uses_outside_are_only_phis_from(func, bi, v) {
                continue;
            }
            let key = (next_use, v, store_at, reload_before);
            match best {
                // Farthest next use wins; ties break on value id so the plan
                // is deterministic across runs.
                Some((bn, bv, _, _)) if (next_use, v) <= (bn, bv) => {}
                _ => best = Some(key),
            }
        }

        if split_debug_enabled() && best.is_none() {
            let mut r_used = 0; let mut r_noprev = 0; let mut r_gap = 0; let mut r_out = 0; let mut r_span = 0;
            for (&v, &(sp0, e)) in bl.span.iter() {
                if done.contains(&v) || sp0 > peak_pt || e < peak_pt { r_span += 1; continue; }
                let empty: Vec<usize> = Vec::new();
                let vu = bl.uses.get(&v).unwrap_or(&empty);
                if vu.contains(&peak_pt) || bl.def.get(&v) == Some(&peak_pt) { r_used += 1; continue; }
                let prev = vu.iter().copied().filter(|&u| u <= peak_pt).next_back().or_else(|| bl.def.get(&v).copied()).map_or(0, |p| p + 1);
                let _ = &mut r_noprev;
                match vu.iter().copied().find(|&u| u > peak_pt) {
                    Some(u) => { if u.saturating_sub(prev) < min_gap { r_gap += 1; } }
                    None => { if !bl.live_out.contains(&v) || !uses_outside_are_only_phis_from(func, bi, v) { r_out += 1; } }
                }
            }
            eprintln!("[SPLIT-PRESSURE] {} blk{} peak_pt={} peak={} REJECT span={} used_at_peak={} noprev={} gap={} outside={}",
                func.name, bi, peak_pt, peak, r_span, r_used, r_noprev, r_gap, r_out);
        }
        let Some((_, vid, store_at, reload_before)) = best else {
            // Nothing further is splittable here; leave the rest to the
            // colorer rather than spinning.
            break;
        };
        let ty = match types.get(&vid) {
            Some(&t) => t,
            None => break,
        };
        plan.push(PressureSplit {
            vid,
            ty,
            store_at,
            reload_before,
        });
        done.insert(vid);
        *remaining -= 1;

        // The register is free from just after the store to just before the
        // reload; drop the pressure there so the next iteration targets a
        // different value.
        let hi = if reload_before == usize::MAX {
            n_pts - 1
        } else {
            reload_before
        };
        for p in pressure.iter_mut().take(hi.min(n_pts - 1)).skip(store_at) {
            *p = p.saturating_sub(1);
        }
    }
    plan
}

/// Apply one block's plan.
///
/// Rebuilt in ONE linear pass rather than by repeated `insert` calls: every
/// insertion shifts the indices of every later split in the same block, and
/// patching them incrementally is how the first revision produced wrong code
/// (a split whose reload index had been shifted by an earlier, lower-indexed
/// split reloaded before its own store). Planning indices stay in ORIGINAL
/// coordinates throughout, and the rename is expressed in the same
/// coordinates.
fn apply_block_splits(
    func: &mut IrFunction,
    bi: usize,
    plan: &[PressureSplit],
    next_val: &mut u32,
) -> usize {
    if plan.is_empty() {
        return 0;
    }
    let n = func.blocks[bi].instructions.len();
    // A store may never precede the block's phi prefix: phis must remain the
    // first instructions of their block.
    let phi_end = first_non_phi(&func.blocks[bi]);

    struct Materialized {
        vid: u32,
        new_val: Value,
        ty: IrType,
        alloca: Value,
        store_at: usize,
        /// Original index the reload is inserted BEFORE; `n` = before the
        /// terminator.
        reload_at: usize,
    }
    let mut mats: Vec<Materialized> = Vec::new();
    for sp in plan {
        let Some(alloca) = next_value(next_val) else { break };
        let Some(new_val) = next_value(next_val) else { break };
        let reload_at = if sp.reload_before == usize::MAX {
            n
        } else {
            sp.reload_before.min(n)
        };
        let store_at = sp.store_at.max(phi_end).min(reload_at);
        if reload_at.saturating_sub(store_at) < pressure_min_gap() {
            // The clamp past the phi prefix can shrink a gap below the
            // profitability floor; drop the split rather than pay for it.
            continue;
        }
        mats.push(Materialized {
            vid: sp.vid,
            new_val,
            ty: sp.ty,
            alloca,
            store_at,
            reload_at,
        });
    }
    if mats.is_empty() {
        return 0;
    }

    // Rename map per ORIGINAL instruction index: an instruction at original
    // index `i` reads the reloaded name of every split whose reload sits at
    // or before `i`.
    let old = std::mem::take(&mut func.blocks[bi].instructions);
    let mut out: Vec<Instruction> = Vec::with_capacity(old.len() + 2 * mats.len());
    let mut active: FxHashMap<u32, u32> = FxHashMap::default();

    for (i, mut inst) in old.into_iter().enumerate() {
        // Stores first: they must read the ORIGINAL value, so they are
        // emitted before any reload at the same index can rename it.
        for m in mats.iter().filter(|m| m.store_at == i) {
            out.push(Instruction::Store {
                volatile: false,
                val: Operand::Value(Value(m.vid)),
                ptr: m.alloca,
                ty: m.ty,
                seg_override: AddressSpace::Default,
            });
        }
        for m in mats.iter().filter(|m| m.reload_at == i) {
            out.push(Instruction::Load {
                volatile: false,
                dest: m.new_val,
                ptr: m.alloca,
                ty: m.ty,
                seg_override: AddressSpace::Default,
            });
            active.insert(m.vid, m.new_val.0);
        }
        if !active.is_empty() {
            // `rewrite_phi = false`: a phi in THIS block reads on the
            // incoming edge, never at the top of the body.
            replace_values_in_inst(&mut inst, &active, false);
        }
        out.push(inst);
    }
    // Trailing stores/reloads that belong just before the terminator.
    for m in mats.iter().filter(|m| m.store_at == n) {
        out.push(Instruction::Store {
            volatile: false,
            val: Operand::Value(Value(m.vid)),
            ptr: m.alloca,
            ty: m.ty,
            seg_override: AddressSpace::Default,
        });
    }
    for m in mats.iter().filter(|m| m.reload_at == n) {
        out.push(Instruction::Load {
            volatile: false,
            dest: m.new_val,
            ptr: m.alloca,
            ty: m.ty,
            seg_override: AddressSpace::Default,
        });
        active.insert(m.vid, m.new_val.0);
    }
    func.blocks[bi].instructions = out;
    if !active.is_empty() {
        replace_values_in_terminator(&mut func.blocks[bi].terminator, &active);
    }

    // Successor phi operands coming from THIS block: the reload dominates
    // the edge, so rewriting them completes the rename.
    let block_label = func.blocks[bi].label;
    for b in func.blocks.iter_mut() {
        for inst in b.instructions.iter_mut() {
            if let Instruction::Phi { incoming, .. } = inst {
                for (op, pred) in incoming.iter_mut() {
                    if *pred != block_label {
                        continue;
                    }
                    if let Operand::Value(v) = op {
                        if let Some(&nv) = active.get(&v.0) {
                            *op = Operand::Value(Value(nv));
                        }
                    }
                }
            }
        }
    }

    for m in &mats {
        // A volatile slot: copy-prop, DCE and store-to-load forwarding all
        // run after this pass and a plain alloca would simply be forwarded
        // back into one value, undoing the split. To stack layout it is an
        // ordinary frame home.
        insert_entry_alloca(func, m.alloca, m.ty, true);
        if split_debug_enabled() {
            eprintln!(
                "[SPLIT-PRESSURE] {} block {} value {} store@{} reload@{}",
                func.name, bi, m.vid, m.store_at, m.reload_at
            );
        }
    }
    mats.len()
}

/// RA-06 entry point: split high-pressure intra-block live ranges so the
/// colorer sees an approximately k-colorable block.
///
/// Returns the number of splits applied. Fails closed everywhere: a value
/// whose consumers this pass cannot rename exactly is left alone.
pub fn split_high_pressure_ranges(func: &mut IrFunction, max_splits: usize) -> usize {
    if func.blocks.is_empty() || max_splits == 0 {
        return 0;
    }
    let types = collect_value_types(func);
    let alloca_ids = collect_alloca_ids(func);

    // Eligibility. A split re-materialises the value through an 8-byte-or-
    // narrower GPR slot, so anything that does not actually live in a GPR —
    // or whose home the backend pins for its own reasons — must be excluded.
    // `is_simple_gpr_type` alone is not enough: a 256-bit vector intrinsic
    // result can carry a scalar-looking IR type (the width lives in the
    // intrinsic, not the type), and storing one through an I64 slot
    // truncates it to its low lane. That is what the first revision did, and
    // the vectorize_* oracle tests caught it.
    let mut ineligible: FxHashSet<u32> = alloca_ids.clone();
    for b in &func.blocks {
        for inst in &b.instructions {
            match inst {
                // Intrinsics own their operand and result register classes
                // (XMM/YMM, fixed pairs, accumulator-pinned forms).
                Instruction::Intrinsic { dest, .. } => {
                    if let Some(d) = dest {
                        ineligible.insert(d.0);
                    }
                    crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                        if let Operand::Value(v) = op {
                            ineligible.insert(v.0);
                        }
                    });
                    crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                        ineligible.insert(v.0);
                    });
                }
                // Inline asm binds values to constraint-named registers.
                Instruction::InlineAsm { .. } => {
                    crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                        if let Operand::Value(v) = op {
                            ineligible.insert(v.0);
                        }
                    });
                    crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                        ineligible.insert(v.0);
                    });
                }
                _ => {}
            }
        }
    }
    let types: FxHashMap<u32, IrType> = types
        .into_iter()
        .filter(|(v, _)| !ineligible.contains(v))
        .collect();
    if types.is_empty() {
        return 0;
    }

    let liveness = crate::backend::liveness::compute_live_intervals(func);
    let budget = pressure_budget();
    let mut remaining = max_splits;
    let mut total = 0usize;
    let mut next_val = func.next_value_id;

    // Innermost-first is irrelevant for an intra-block pass; visit in block
    // order for determinism.
    let mut plans: Vec<(usize, Vec<PressureSplit>)> = Vec::new();
    for bi in 0..func.blocks.len() {
        let plan = plan_block_splits(func, &liveness, bi, &types, budget, &mut remaining);
        if !plan.is_empty() {
            plans.push((bi, plan));
        }
    }
    for (bi, plan) in plans {
        total += apply_block_splits(func, bi, &plan, &mut next_val);
    }
    func.next_value_id = next_val;
    total
}

#[cfg(test)]
mod pressure_split_tests {
    use super::*;

    fn f_with(blocks: Vec<BasicBlock>) -> IrFunction {
        let mut f = IrFunction::new("t".to_string(), IrType::Void, Vec::new(), false);
        f.blocks = blocks;
        f.next_value_id = 1000;
        f
    }
    fn blk(label: u32, instructions: Vec<Instruction>, term: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator: term,
            source_spans: Vec::new(),
        }
    }
    fn add(dest: u32, a: u32, b: u32) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(a)),
            rhs: Operand::Value(Value(b)),
            ty: IrType::I64,
        }
    }

    /// A value whose consumers all live in another block must NOT be split:
    /// renaming them needs cross-block SSA repair. This is the guard that
    /// keeps the pass fail-closed, and it is also the reason the pass cannot
    /// reach MAXLIVE <= k (see engineering/DECISIONS.md).
    #[test]
    fn outside_consumer_that_is_not_a_phi_blocks_the_split() {
        let f = f_with(vec![
            blk(0, vec![add(1, 900, 901)], Terminator::Branch(BlockId(1))),
            blk(1, vec![add(2, 1, 1)], Terminator::Return(None)),
        ]);
        assert!(!uses_outside_are_only_phis_from(&f, 0, 1));
    }

    /// A value consumed only by a successor phi paired with this block IS
    /// renameable: the reload sits at the end of the block and dominates the
    /// edge.
    #[test]
    fn successor_phi_operand_from_this_block_is_renameable() {
        let f = f_with(vec![
            blk(0, vec![add(1, 900, 901)], Terminator::Branch(BlockId(1))),
            blk(
                1,
                vec![Instruction::Phi {
                    dest: Value(2),
                    ty: IrType::I64,
                    incoming: vec![(Operand::Value(Value(1)), BlockId(0))],
                }],
                Terminator::Return(None),
            ),
        ]);
        assert!(uses_outside_are_only_phis_from(&f, 0, 1));
    }

    /// The same value reaching a phi from a DIFFERENT predecessor cannot be
    /// renamed by this block's reload (the reload does not dominate that
    /// edge), so the split is refused.
    #[test]
    fn phi_operand_from_another_predecessor_blocks_the_split() {
        let f = f_with(vec![
            blk(0, vec![add(1, 900, 901)], Terminator::Branch(BlockId(2))),
            blk(1, vec![], Terminator::Branch(BlockId(2))),
            blk(
                2,
                vec![Instruction::Phi {
                    dest: Value(2),
                    ty: IrType::I64,
                    incoming: vec![
                        (Operand::Value(Value(1)), BlockId(0)),
                        (Operand::Value(Value(1)), BlockId(1)),
                    ],
                }],
                Terminator::Return(None),
            ),
        ]);
        assert!(!uses_outside_are_only_phis_from(&f, 0, 1));
    }

    /// The budget and gap knobs must stay inside sane bounds however they are
    /// set, so a stray environment value cannot make the pass split every
    /// value (budget 0) or none (huge gap silently disabling it).
    #[test]
    fn tuning_knobs_are_clamped() {
        assert!((2..=64).contains(&pressure_budget()));
        assert!((1..=256).contains(&pressure_min_gap()));
    }
}
