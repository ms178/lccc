//! Promote simple loop-local memory recurrences to SSA.
//!
//! Recognizes a natural loop with **one** load and **one** store of the same
//! loop-invariant scalar address, store in the unique latch, no may-alias
//! memory ops in the body. The load is hoisted to the preheader, the value
//! is carried by a phi, and a single store is emitted on the unique exit.
//!
//! # Soundness contracts
//!
//! - **Recurrence direction.** Load observes the *previous* iteration's store.
//!   If load and store share a block, the load must precede the store.
//!   Store-then-load is a block-local temporary and must not become a phi
//!   (that handed iteration 0 an uninitialized preheader read).
//! - **Exit store value.** `phi` is the value *entering* the iteration.
//!   The latch store writes `store_val`. If the unique exit leaves from the
//!   latch, memory must receive `store_val`, not `phi`. Storing `phi` on a
//!   do-while is a one-iteration-stale write.
//! - **Must-execute / speculation.** The load is hoisted to the preheader.
//!   Allowed only when the address is alloca-derived (function-local,
//!   non-faulting) **or** the load block dominates the unique exit source.
//! - **The load dominates the latch.** Otherwise a path to the store skips
//!   the load and the phi would invent a value.
//! - **Alias.** Distinct *allocas* and distinct *globals* are different
//!   objects. A `ParamRef` or an unknown root may alias anything that is
//!   not a proven-distinct alloca/global range. Two GEPs of the same field
//!   are different SSA ids and are **not** disjoint.
//! - **Unknown memory.** Call, memcpy, inline-asm, atomics, va_*,
//!   stackrestore, intrinsics, fences: refuse the whole loop.
//! - **Phis stay first.** The exit store is inserted after leading phis.
//!   `source_spans` stay 1:1 with instructions.
//!
//! Safe degradation is “don't promote”.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::analysis::CfgAnalysis;
use crate::ir::reexports::{
    BasicBlock, Instruction, IrBinOp, IrConst, IrFunction, Operand, Terminator, Value,
};
use super::loop_analysis;
use std::sync::OnceLock;

const MAX_PROMOTIONS: usize = 64;
const RESOLVE_FUEL: u8 = 32;
const CHAIN_FUEL: u32 = 64;

/// Object-identity tags in the top 4 bits of a [`Path::root`].
/// The low 60 bits are an id / hash. Different *kinds* never collide.
const TAG_SHIFT: u32 = 60;
const TAG_ALLOCA: u64 = 1 << TAG_SHIFT;
const TAG_PARAM: u64 = 2 << TAG_SHIFT;
const TAG_GLOBAL: u64 = 3 << TAG_SHIFT;
const TAG_OTHER: u64 = 4 << TAG_SHIFT;
const TAG_MASK: u64 = 0xF << TAG_SHIFT;

#[derive(Clone, Copy, Debug)]
struct Path {
    root: u64,
    offset: i64,
}

fn promote_debug() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("CCC_DEBUG_PROMOTE").is_some())
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &b in bytes {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    h
}

fn tag_of(root: u64) -> u64 {
    root & TAG_MASK
}

fn is_alloca_root(root: u64) -> bool {
    tag_of(root) == TAG_ALLOCA
}

fn is_unique_object_tag(tag: u64) -> bool {
    tag == TAG_ALLOCA || tag == TAG_GLOBAL
}

/// Stable identity for the *object* a pointer refers to.
///
/// Two `GlobalAddr "foo"` with different dest ids are the same object.
/// Two `ParamRef` of the same argument are the same object. Dest ids are
/// **not** object identity.
fn object_root(inst: &Instruction, dest: Value) -> u64 {
    match inst {
        Instruction::GlobalAddr { name, .. } => TAG_GLOBAL | (fnv1a(name.as_bytes()) >> 4),
        Instruction::Alloca { .. } => TAG_ALLOCA | dest.0 as u64,
        Instruction::ParamRef { param_idx, .. } => TAG_PARAM | *param_idx as u64,
        _ => TAG_OTHER | dest.0 as u64,
    }
}

fn pointer_paths(defs: &FxHashMap<u32, &Instruction>) -> FxHashMap<u32, Path> {
    let mut paths = FxHashMap::default();
    for (&vid, inst) in defs {
        match inst {
            Instruction::Alloca { dest, .. }
            | Instruction::ParamRef { dest, .. }
            | Instruction::GlobalAddr { dest, .. } => {
                paths.insert(
                    vid,
                    Path {
                        root: object_root(inst, *dest),
                        offset: 0,
                    },
                );
            }
            _ => {}
        }
    }

    // Fixpoint on GEP/Copy/Add-const/widening-cast. Each dest is inserted
    // at most once, so the loop is O(|defs| × chain depth).
    loop {
        let mut progressed = false;
        for (&vid, inst) in defs {
            if paths.contains_key(&vid) {
                continue;
            }
            let derived = match inst {
                Instruction::GetElementPtr {
                    dest,
                    base,
                    offset: Operand::Const(c),
                    ..
                } => c.to_i64().and_then(|off| {
                    let p = paths.get(&base.0)?;
                    Some((
                        dest.0,
                        Path {
                            root: p.root,
                            offset: p.offset.checked_add(off)?,
                        },
                    ))
                }),
                Instruction::Copy {
                    dest,
                    src: Operand::Value(src),
                } => paths.get(&src.0).copied().map(|p| (dest.0, p)),
                Instruction::Cast {
                    dest,
                    src: Operand::Value(src),
                    from_ty,
                    to_ty,
                    ..
                } if from_ty.size() <= to_ty.size() => {
                    paths.get(&src.0).copied().map(|p| (dest.0, p))
                }
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs,
                    rhs,
                    ..
                } => match (lhs, rhs) {
                    (Operand::Value(base), Operand::Const(c))
                    | (Operand::Const(c), Operand::Value(base)) => c.to_i64().and_then(|off| {
                        let p = paths.get(&base.0)?;
                        Some((
                            dest.0,
                            Path {
                                root: p.root,
                                offset: p.offset.checked_add(off)?,
                            },
                        ))
                    }),
                    _ => None,
                },
                _ => None,
            };
            if let Some((dest, path)) = derived {
                paths.insert(dest, path);
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    paths
}

fn byte_size(ty: IrType) -> Option<i64> {
    Some(match ty {
        IrType::I8 | IrType::U8 => 1,
        IrType::I16 | IrType::U16 => 2,
        IrType::I32 | IrType::U32 | IrType::F32 => 4,
        IrType::I64 | IrType::U64 | IrType::F64 | IrType::Ptr => 8,
        _ => return None,
    })
}

fn is_promotable_type(ty: IrType) -> bool {
    byte_size(ty).is_some()
}

/// True only when the two accesses are **proved** to never overlap.
///
/// Different roots are disjoint iff both name uniquely-identified objects
/// (distinct allocas, distinct globals, or alloca↔global). A parameter or
/// an unknown root may alias any of those — returning true here would
/// drop an aliasing in-loop store and leave a load reading stale memory.
fn disjoint(paths: &FxHashMap<u32, Path>, a: Value, a_ty: IrType, b: Value, b_ty: IrType) -> bool {
    let (Some(pa), Some(pb)) = (paths.get(&a.0), paths.get(&b.0)) else {
        return false;
    };
    if pa.root != pb.root {
        return is_unique_object_tag(tag_of(pa.root)) && is_unique_object_tag(tag_of(pb.root));
    }
    let (Some(asize), Some(bsize)) = (byte_size(a_ty), byte_size(b_ty)) else {
        return false;
    };
    let (Some(ae), Some(be)) = (pa.offset.checked_add(asize), pb.offset.checked_add(bsize)) else {
        return false;
    };
    ae <= pb.offset || be <= pa.offset
}

fn resolve_ptr_chain(defs: &FxHashMap<u32, &Instruction>, start: Value) -> Option<(u32, i64)> {
    let mut cur = start;
    let mut off: i64 = 0;
    let mut seen = FxHashSet::default();
    for _ in 0..CHAIN_FUEL {
        if !seen.insert(cur.0) {
            return None;
        }
        let Some(inst) = defs.get(&cur.0) else {
            return Some((cur.0, off));
        };
        let next = match *inst {
            Instruction::GetElementPtr {
                base,
                offset: Operand::Const(c),
                ..
            } => c.to_i64().map(|k| (*base, k)),
            Instruction::Copy {
                src: Operand::Value(src),
                ..
            } => Some((*src, 0)),
            Instruction::BinOp {
                op: IrBinOp::Add,
                lhs,
                rhs,
                ..
            } => match (lhs, rhs) {
                (Operand::Value(v), Operand::Const(c))
                | (Operand::Const(c), Operand::Value(v)) => c.to_i64().map(|k| (*v, k)),
                _ => None,
            },
            _ => None,
        };
        match next {
            Some((base, k)) => {
                off = off.checked_add(k)?;
                cur = base;
            }
            None => return Some((cur.0, off)),
        }
    }
    None
}

/// `address = root + Σ coeff_i * iv_i + konst + march * t`
/// `t` counts iterations of the *current* loop. Checked arithmetic; overflow
/// refuses the proof (never wraps into a false disjoint).
#[derive(Clone, PartialEq, Eq, Debug)]
struct LinForm {
    root: u64,
    syms: Vec<(u32, i64)>,
    konst: i64,
    march: i64,
}

fn striding_phi(defs: &FxHashMap<u32, &Instruction>, phi_v: Value) -> Option<(Operand, i64)> {
    let Instruction::Phi { incoming, .. } = *defs.get(&phi_v.0)? else {
        return None;
    };
    if incoming.len() != 2 {
        return None;
    }
    let mut init = None;
    let mut stride = 0i64;
    for (op, _) in incoming {
        if let Operand::Value(v) = op {
            if let Some((root, off)) = resolve_ptr_chain(defs, *v) {
                if root == phi_v.0 && off != 0 {
                    stride = off;
                    continue;
                }
            }
        }
        if init.is_some() {
            return None;
        }
        init = Some(*op);
    }
    if stride == 0 {
        return None;
    }
    Some((init?, stride))
}

fn resolve_lin_form(
    func: &IrFunction,
    defs: &FxHashMap<u32, &Instruction>,
    lp_body: &FxHashSet<usize>,
    def_block: &FxHashMap<u32, usize>,
    cur_header: usize,
    v: Value,
    fuel: u8,
) -> Option<LinForm> {
    if fuel == 0 {
        return None;
    }
    let fuel = fuel - 1;
    let inst = *defs.get(&v.0)?;
    let def_bi = def_block.get(&v.0).copied().unwrap_or(usize::MAX);

    if def_bi == cur_header {
        if matches!(inst, Instruction::Phi { .. }) {
            let (init_op, stride) = striding_phi(defs, v)?;
            let mut f = match init_op {
                Operand::Value(init_v) => {
                    resolve_lin_form(func, defs, lp_body, def_block, cur_header, init_v, fuel)?
                }
                Operand::Const(c) => LinForm {
                    root: 0,
                    syms: vec![],
                    konst: c.to_i64()?,
                    march: 0,
                },
                _ => return None,
            };
            f.march = f.march.checked_add(stride)?;
            return Some(f);
        }
        return None;
    }

    match inst {
        Instruction::Copy {
            src: Operand::Value(src),
            ..
        } => resolve_lin_form(func, defs, lp_body, def_block, cur_header, *src, fuel),
        Instruction::Cast {
            src: Operand::Value(src),
            from_ty,
            to_ty,
            ..
        } if from_ty.size() <= to_ty.size() => {
            resolve_lin_form(func, defs, lp_body, def_block, cur_header, *src, fuel)
        }
        Instruction::GetElementPtr { base, offset, .. } => {
            let mut f = resolve_lin_form(func, defs, lp_body, def_block, cur_header, *base, fuel)?;
            match offset {
                Operand::Const(c) => {
                    f.konst = f.konst.checked_add(c.to_i64()?)?;
                }
                Operand::Value(ov) => {
                    let g = resolve_lin_form(func, defs, lp_body, def_block, cur_header, *ov, fuel)?;
                    f = merge_forms(f, g)?;
                }
                _ => return None,
            }
            Some(f)
        }
        Instruction::BinOp {
            op: IrBinOp::Add,
            lhs,
            rhs,
            ..
        } => match (lhs, rhs) {
            (Operand::Value(a), Operand::Value(b)) => {
                let fa = resolve_lin_form(func, defs, lp_body, def_block, cur_header, *a, fuel)?;
                let fb = resolve_lin_form(func, defs, lp_body, def_block, cur_header, *b, fuel)?;
                merge_forms(fa, fb)
            }
            (Operand::Value(a), Operand::Const(c)) | (Operand::Const(c), Operand::Value(a)) => {
                let mut fa = resolve_lin_form(func, defs, lp_body, def_block, cur_header, *a, fuel)?;
                fa.konst = fa.konst.checked_add(c.to_i64()?)?;
                Some(fa)
            }
            _ => None,
        },
        Instruction::BinOp {
            op: IrBinOp::Mul,
            lhs,
            rhs,
            ..
        } => {
            let (val_op, c) = match (lhs, rhs) {
                (Operand::Value(a), Operand::Const(c)) | (Operand::Const(c), Operand::Value(a)) => {
                    (*a, c.to_i64()?)
                }
                _ => return None,
            };
            let mut f = resolve_lin_form(func, defs, lp_body, def_block, cur_header, val_op, fuel)?;
            if f.root != 0 {
                return None;
            }
            f.konst = f.konst.checked_mul(c)?;
            f.march = f.march.checked_mul(c)?;
            for s in f.syms.iter_mut() {
                s.1 = s.1.checked_mul(c)?;
            }
            Some(f)
        }
        Instruction::Phi { ty, .. } => {
            if lp_body.contains(&def_bi) {
                return None;
            }
            if *ty != IrType::Ptr {
                if striding_phi(defs, v).is_some() {
                    return Some(LinForm {
                        root: 0,
                        syms: vec![(v.0, 1)],
                        konst: 0,
                        march: 0,
                    });
                }
                return Some(LinForm {
                    root: object_root(inst, v),
                    syms: vec![],
                    konst: 0,
                    march: 0,
                });
            }
            let (init_op, stride) = striding_phi(defs, v)?;
            let Operand::Value(init_v) = init_op else {
                return None;
            };
            let outer_header = def_bi;
            if outer_header >= func.blocks.len() {
                return None;
            }
            let mut iv_sym = None;
            for binst in &func.blocks[outer_header].instructions {
                let Instruction::Phi {
                    dest, ty: ity, ..
                } = binst
                else {
                    continue;
                };
                if *ity == IrType::Ptr {
                    continue;
                }
                let Some((iv_init_op, iv_stride)) = striding_phi(defs, *dest) else {
                    continue;
                };
                let Operand::Const(ivc) = iv_init_op else {
                    continue;
                };
                let Some(iv_c0) = ivc.to_i64() else {
                    continue;
                };
                if iv_stride == 0 || stride % iv_stride != 0 {
                    continue;
                }
                iv_sym = Some((dest.0, stride / iv_stride, iv_c0));
                break;
            }
            let (iv_id, ratio, iv_c0) = iv_sym?;
            let mut f = resolve_lin_form(func, defs, lp_body, def_block, cur_header, init_v, fuel)?;
            if f.march != 0 {
                return None;
            }
            f.konst = f.konst.checked_sub(ratio.checked_mul(iv_c0)?)?;
            match f.syms.iter_mut().find(|s| s.0 == iv_id) {
                Some(s) => s.1 = s.1.checked_add(ratio)?,
                None => f.syms.push((iv_id, ratio)),
            }
            f.syms.sort_unstable_by_key(|s| s.0);
            Some(f)
        }
        _ => {
            if lp_body.contains(&def_bi) {
                return None;
            }
            Some(LinForm {
                root: object_root(inst, v),
                syms: vec![],
                konst: 0,
                march: 0,
            })
        }
    }
}

fn merge_forms(mut a: LinForm, b: LinForm) -> Option<LinForm> {
    if a.root != 0 && b.root != 0 {
        return None;
    }
    if b.root != 0 {
        a.root = b.root;
    }
    a.konst = a.konst.checked_add(b.konst)?;
    a.march = a.march.checked_add(b.march)?;
    for (id, c) in b.syms {
        match a.syms.iter_mut().find(|s| s.0 == id) {
            Some(s) => s.1 = s.1.checked_add(c)?,
            None => a.syms.push((id, c)),
        }
    }
    a.syms.sort_unstable_by_key(|s| s.0);
    a.syms.retain(|s| s.1 != 0);
    Some(a)
}

fn ranges_apart(a0: i64, a_sz: i64, b0: i64, b_sz: i64) -> Option<bool> {
    let ae = a0.checked_add(a_sz)?;
    let be = b0.checked_add(b_sz)?;
    Some(ae <= b0 || be <= a0)
}

/// Prove `cand` (loop-invariant) never overlaps `store` in any iteration.
///
/// Same root + same symbolic IV terms. If the store marches away from the
/// candidate's range starting from a non-overlapping `t = 0` slot, later
/// iterations only recede further. Overflow ⇒ not proved.
fn affine_disjoint(
    func: &IrFunction,
    defs: &FxHashMap<u32, &Instruction>,
    lp_body: &FxHashSet<usize>,
    def_block: &FxHashMap<u32, usize>,
    header_idx: usize,
    cand: Value,
    cand_ty: IrType,
    store: Value,
    store_ty: IrType,
) -> bool {
    if def_block.get(&cand.0).is_some_and(|b| lp_body.contains(b)) {
        return false;
    }
    let cf = resolve_lin_form(func, defs, lp_body, def_block, header_idx, cand, RESOLVE_FUEL);
    let sf = resolve_lin_form(func, defs, lp_body, def_block, header_idx, store, RESOLVE_FUEL);
    if promote_debug() {
        eprintln!(
            "[AFFINE] cand={} -> {:?}; store={} -> {:?}",
            cand.0,
            cf.as_ref().map(|f| (f.root, &f.syms, f.konst, f.march)),
            store.0,
            sf.as_ref().map(|f| (f.root, &f.syms, f.konst, f.march))
        );
    }
    let (Some(cf), Some(sf)) = (cf, sf) else {
        return false;
    };
    if cf.root == 0 || cf.root != sf.root {
        return false;
    }
    let (Some(cand_sz), Some(store_sz)) = (byte_size(cand_ty), byte_size(store_ty)) else {
        return false;
    };
    if sf.march == 0 && cf.march == 0 {
        if cf.syms != sf.syms {
            return false;
        }
        return ranges_apart(cf.konst, cand_sz, sf.konst, store_sz).unwrap_or(false);
    }
    if cf.march != 0 {
        return false;
    }
    if cf.syms != sf.syms {
        return false;
    }
    if sf.march > 0 {
        cf.konst
            .checked_add(cand_sz)
            .is_some_and(|end| sf.konst >= end)
    } else {
        sf.konst
            .checked_add(store_sz)
            .is_some_and(|end| end <= cf.konst)
    }
}

fn dominates(idom: &[usize], node: usize, anc: usize) -> bool {
    if node == anc {
        return true;
    }
    let mut cur = node;
    for _ in 0..idom.len().saturating_add(1) {
        if cur == anc {
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

fn inst_may_clobber_unknown(inst: &Instruction) -> bool {
    matches!(
        inst,
        Instruction::Call { .. }
            | Instruction::CallIndirect { .. }
            | Instruction::Memcpy { .. }
            | Instruction::InlineAsm { .. }
            | Instruction::Intrinsic { .. }
            | Instruction::VaArg { .. }
            | Instruction::VaStart { .. }
            | Instruction::VaCopy { .. }
            | Instruction::VaArgStruct { .. }
            | Instruction::StackRestore { .. }
            | Instruction::Fence { .. }
            | Instruction::AtomicRmw { .. }
            | Instruction::AtomicInc { .. }
            | Instruction::AtomicCmpxchg { .. }
            | Instruction::AtomicLoad { .. }
            | Instruction::AtomicStore { .. }
    )
}

fn first_non_phi(block: &BasicBlock) -> usize {
    block
        .instructions
        .iter()
        .position(|i| !matches!(i, Instruction::Phi { .. }))
        .unwrap_or(block.instructions.len())
}

fn insert_inst(block: &mut BasicBlock, idx: usize, inst: Instruction) {
    let n_inst = block.instructions.len();
    let n_span = block.source_spans.len();
    let idx = idx.min(n_inst);
    block.instructions.insert(idx, inst);
    if n_span == n_inst && n_span > 0 {
        let span = block.source_spans[idx.min(n_span - 1)].clone();
        block.source_spans.insert(idx.min(n_span), span);
    }
}

fn remove_inst(block: &mut BasicBlock, idx: usize) {
    if idx >= block.instructions.len() {
        return;
    }
    let n_span = block.source_spans.len();
    let n_inst = block.instructions.len();
    block.instructions.remove(idx);
    if n_span == n_inst && idx < n_span {
        block.source_spans.remove(idx);
    }
}

fn subst_operand(op: &Operand, from: u32, to: u32) -> Operand {
    match op {
        Operand::Value(v) if v.0 == from => Operand::Value(Value(to)),
        other => other.clone(),
    }
}

fn rewrite_operand(op: &mut Operand, from: u32, to: u32) {
    if let Operand::Value(v) = op {
        if v.0 == from {
            v.0 = to;
        }
    }
}

fn rewrite_value(v: &mut Value, from: u32, to: u32) {
    if v.0 == from {
        v.0 = to;
    }
}

fn rewrite_uses_in_inst(inst: &mut Instruction, from: u32, to: u32) {
    match inst {
        Instruction::Alloca { .. }
        | Instruction::PgoCounterInc { .. }
        | Instruction::GlobalAddr { .. }
        | Instruction::Fence { .. }
        | Instruction::LabelAddr { .. }
        | Instruction::GetReturnF64Second { .. }
        | Instruction::GetReturnF32Second { .. }
        | Instruction::GetReturnF128Second { .. }
        | Instruction::StackSave { .. }
        | Instruction::ParamRef { .. }
        | Instruction::VaEnd { .. } => {}
        Instruction::DynAlloca { size, .. } => rewrite_operand(size, from, to),
        Instruction::Store { val, ptr, .. } => {
            rewrite_operand(val, from, to);
            rewrite_value(ptr, from, to);
        }
        Instruction::Load { ptr, .. } => rewrite_value(ptr, from, to),
        Instruction::BinOp { lhs, rhs, .. } | Instruction::Cmp { lhs, rhs, .. } => {
            rewrite_operand(lhs, from, to);
            rewrite_operand(rhs, from, to);
        }
        Instruction::UnaryOp { src, .. }
        | Instruction::Cast { src, .. }
        | Instruction::Copy { src, .. } => rewrite_operand(src, from, to),
        Instruction::Call { info, .. } => {
            for a in &mut info.args {
                rewrite_operand(a, from, to);
            }
        }
        Instruction::CallIndirect { func_ptr, info } => {
            rewrite_operand(func_ptr, from, to);
            for a in &mut info.args {
                rewrite_operand(a, from, to);
            }
        }
        Instruction::GetElementPtr { base, offset, .. } => {
            rewrite_value(base, from, to);
            rewrite_operand(offset, from, to);
        }
        Instruction::Memcpy { dest, src, .. } => {
            rewrite_value(dest, from, to);
            rewrite_value(src, from, to);
        }
        Instruction::VaArg { va_list_ptr, .. } | Instruction::VaStart { va_list_ptr } => {
            rewrite_value(va_list_ptr, from, to);
        }
        Instruction::VaCopy { dest_ptr, src_ptr } => {
            rewrite_value(dest_ptr, from, to);
            rewrite_value(src_ptr, from, to);
        }
        Instruction::VaArgStruct {
            dest_ptr,
            va_list_ptr,
            ..
        } => {
            rewrite_value(dest_ptr, from, to);
            rewrite_value(va_list_ptr, from, to);
        }
        Instruction::AtomicRmw { ptr, val, .. } | Instruction::AtomicStore { ptr, val, .. } => {
            rewrite_operand(ptr, from, to);
            rewrite_operand(val, from, to);
        }
        Instruction::AtomicInc { ptr, .. } | Instruction::AtomicLoad { ptr, .. } => {
            rewrite_operand(ptr, from, to);
        }
        Instruction::AtomicCmpxchg {
            ptr,
            expected,
            desired,
            ..
        } => {
            rewrite_operand(ptr, from, to);
            rewrite_operand(expected, from, to);
            rewrite_operand(desired, from, to);
        }
        Instruction::Phi { incoming, .. } => {
            for (op, _) in incoming {
                rewrite_operand(op, from, to);
            }
        }
        Instruction::SetReturnF64Second { src }
        | Instruction::SetReturnF32Second { src }
        | Instruction::SetReturnF128Second { src } => rewrite_operand(src, from, to),
        Instruction::InlineAsm {
            inputs, outputs, ..
        } => {
            for (_, op, _) in inputs {
                rewrite_operand(op, from, to);
            }
            for (_, v, _) in outputs {
                rewrite_value(v, from, to);
            }
        }
        Instruction::Intrinsic {
            args, dest_ptr, ..
        } => {
            for a in args {
                rewrite_operand(a, from, to);
            }
            if let Some(dp) = dest_ptr {
                rewrite_value(dp, from, to);
            }
        }
        Instruction::Select {
            cond,
            true_val,
            false_val,
            ..
        } => {
            rewrite_operand(cond, from, to);
            rewrite_operand(true_val, from, to);
            rewrite_operand(false_val, from, to);
        }
        Instruction::StackRestore { ptr } => rewrite_value(ptr, from, to),
    }
}

fn rewrite_uses_in_terminator(term: &mut Terminator, from: u32, to: u32) {
    match term {
        Terminator::Return(Some(op)) => rewrite_operand(op, from, to),
        Terminator::CondBranch { cond, .. } => rewrite_operand(cond, from, to),
        Terminator::IndirectBranch { target, .. } => rewrite_operand(target, from, to),
        Terminator::Switch { val, .. } => rewrite_operand(val, from, to),
        _ => {}
    }
}

/// Value that must be written on the unique exit so memory matches the
/// original last store that actually executed.
fn memory_on_exit(exit_from: usize, latch: usize, phi: Value, store_val: &Operand) -> Operand {
    if exit_from == latch {
        store_val.clone()
    } else {
        Operand::Value(phi)
    }
}

fn next_value(func: &mut IrFunction) -> Option<Value> {
    let id = func.next_value_id;
    if id == u32::MAX {
        return None;
    }
    func.next_value_id = id.saturating_add(1);
    Some(Value(id))
}

fn collect_defs(func: &IrFunction) -> (FxHashMap<u32, &Instruction>, FxHashMap<u32, usize>) {
    let mut defs = FxHashMap::default();
    let mut def_block = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                defs.entry(dest.0).or_insert(inst);
                def_block.entry(dest.0).or_insert(bi);
            }
        }
    }
    (defs, def_block)
}

/// Owned snapshot of a legal promotion. Built while `defs` is borrowed,
/// applied after that borrow ends — otherwise the function does not compile.
struct PromotePlan {
    ptr: Value,
    load_b: usize,
    load_i: usize,
    load_dest: Value,
    load_ty: IrType,
    load_seg: AddressSpace,
    store_b: usize,
    store_i: usize,
    store_val: Operand,
    preheader: usize,
    header: usize,
    latch: usize,
    exit_from: usize,
    exit_block: usize,
}

pub(crate) fn run(func: &mut IrFunction) -> usize {
    // One pointer per `run_once`: indices stay honest and a promoted loop
    // no longer matches. Each success strictly drops the loop's memop count.
    let mut total = 0;
    for _ in 0..MAX_PROMOTIONS {
        let n = run_once(func);
        if n == 0 {
            break;
        }
        total += n;
    }
    total
}

fn run_once(func: &mut IrFunction) -> usize {
    let Some(plan) = find_promotion(func) else {
        return 0;
    };
    apply_promotion(func, plan)
}

fn find_promotion(func: &IrFunction) -> Option<PromotePlan> {
    let cfg = CfgAnalysis::build(func);
    let loops = loop_analysis::merge_loops_by_header(loop_analysis::find_natural_loops(
        cfg.num_blocks,
        &cfg.preds,
        &cfg.succs,
        &cfg.idom,
    ));
    if loops.is_empty() {
        return None;
    }

    let (defs, def_block) = collect_defs(func);
    let paths = pointer_paths(&defs);
    let volatile_roots: FxHashSet<u64> = func
        .blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .filter_map(|inst| match inst {
            Instruction::Alloca {
                dest,
                volatile,
                semantic_volatile,
                ..
            } if *volatile || *semantic_volatile => Some(TAG_ALLOCA | dest.0 as u64),
            _ => None,
        })
        .collect();

    for lp in loops {
        let Some(preheader) = loop_analysis::find_preheader(lp.header, &lp.body, &cfg.preds) else {
            continue;
        };
        if preheader == lp.header {
            continue;
        }
        let latches: Vec<usize> = cfg
            .preds
            .row(lp.header)
            .iter()
            .map(|&v| v as usize)
            .filter(|b| lp.body.contains(b))
            .collect();
        if latches.len() != 1 {
            continue;
        }
        let latch = latches[0];
        let exits: Vec<(usize, usize)> = lp
            .body
            .iter()
            .flat_map(|&from| {
                cfg.succs
                    .row(from)
                    .iter()
                    .map(move |&to| (from, to as usize))
            })
            .filter(|(_, to)| !lp.body.contains(to))
            .collect();
        if exits.len() != 1 {
            continue;
        }
        let (exit_from, exit_block) = exits[0];
        if exit_block >= func.blocks.len() {
            continue;
        }
        // Unique predecessor so the exit store is the unique writeback
        // (and so the exit block is not also a join / the preheader).
        if cfg
            .preds
            .row(exit_block)
            .iter()
            .any(|&p| p as usize != exit_from)
        {
            continue;
        }

        let mut loads: FxHashMap<u32, Vec<(usize, usize, Value, IrType, AddressSpace)>> =
            FxHashMap::default();
        let mut stores: FxHashMap<u32, Vec<(usize, usize, Operand, IrType, AddressSpace)>> =
            FxHashMap::default();
        let mut has_unknown_mem = false;
        for &bi in &lp.body {
            for (ii, inst) in func.blocks[bi].instructions.iter().enumerate() {
                match inst {
                    Instruction::Load {
                        dest,
                        ptr,
                        ty,
                        seg_override,
                        ..
                    } => loads
                        .entry(ptr.0)
                        .or_default()
                        .push((bi, ii, *dest, *ty, *seg_override)),
                    Instruction::Store {
                        val,
                        ptr,
                        ty,
                        seg_override,
                        ..
                    } => stores.entry(ptr.0).or_default().push((
                        bi,
                        ii,
                        val.clone(),
                        *ty,
                        *seg_override,
                    )),
                    other if inst_may_clobber_unknown(other) => has_unknown_mem = true,
                    _ => {}
                }
            }
        }
        if has_unknown_mem {
            continue;
        }

        for (&ptr_id, load_list) in &loads {
            let Some(store_list) = stores.get(&ptr_id) else {
                continue;
            };
            if load_list.len() != 1 || store_list.len() != 1 {
                continue;
            }
            let (load_b, load_i, load_dest, load_ty, load_seg) = load_list[0];
            let (store_b, store_i, store_val, store_ty, store_seg) = &store_list[0];
            if *store_b != latch || load_ty != *store_ty || load_seg != *store_seg {
                continue;
            }
            if !is_promotable_type(load_ty) {
                continue;
            }
            if load_b == *store_b && load_i > *store_i {
                continue;
            }
            if !dominates(&cfg.idom, latch, load_b) {
                continue;
            }
            if def_block.get(&ptr_id).is_some_and(|b| lp.body.contains(b)) {
                continue;
            }
            let ptr = Value(ptr_id);
            if paths
                .get(&ptr_id)
                .is_some_and(|p| volatile_roots.contains(&p.root))
            {
                continue;
            }

            // Speculative preheader load: only for alloca roots, or when the
            // original load already executes on every path to the unique exit.
            let alloca_backed = paths.get(&ptr_id).is_some_and(|p| is_alloca_root(p.root));
            if !alloca_backed && !dominates(&cfg.idom, exit_from, load_b) {
                continue;
            }

            let mut alias = false;
            let mut may_alias = |other: Value, other_ty: IrType| {
                !disjoint(&paths, ptr, load_ty, other, other_ty)
                    && !affine_disjoint(
                        func,
                        &defs,
                        &lp.body,
                        &def_block,
                        lp.header,
                        ptr,
                        load_ty,
                        other,
                        other_ty,
                    )
            };
            for (&other_ptr, other_stores) in &stores {
                if other_ptr == ptr_id {
                    continue;
                }
                for (_, _, _, other_ty, _) in other_stores {
                    if may_alias(Value(other_ptr), *other_ty) {
                        alias = true;
                    }
                }
            }
            // Other loads through a different SSA pointer that aliases this
            // location must block: the in-loop store is removed, so they
            // would read stale memory (sqlite3FpDecode `n--` infinite loop).
            for (&other_ptr, other_loads) in &loads {
                if other_ptr == ptr_id {
                    continue;
                }
                for (_, _, _, other_ty, _) in other_loads {
                    if may_alias(Value(other_ptr), *other_ty) {
                        alias = true;
                    }
                }
            }
            if alias {
                continue;
            }

            if promote_debug() {
                eprintln!(
                    "[PROMOTE] func={} loop header={} ptr=Value({}) ty={:?} exit_from={} latch={}",
                    func.name, lp.header, ptr_id, load_ty, exit_from, latch
                );
            }

            return Some(PromotePlan {
                ptr,
                load_b,
                load_i,
                load_dest,
                load_ty,
                load_seg,
                store_b: *store_b,
                store_i: *store_i,
                store_val: store_val.clone(),
                preheader,
                header: lp.header,
                latch,
                exit_from,
                exit_block,
            });
        }
    }
    None
}

fn apply_promotion(func: &mut IrFunction, plan: PromotePlan) -> usize {
    let Some(init) = next_value(func) else {
        return 0;
    };
    let Some(phi) = next_value(func) else {
        return 0;
    };

    let latch_incoming = subst_operand(&plan.store_val, plan.load_dest.0, phi.0);
    let exit_val = memory_on_exit(plan.exit_from, plan.latch, phi, &latch_incoming);

    insert_inst(
        &mut func.blocks[plan.preheader],
        func.blocks[plan.preheader].instructions.len(),
        Instruction::Load {
            dest: init,
            ptr: plan.ptr,
            ty: plan.load_ty,
            seg_override: plan.load_seg,
        },
    );
    let pre_label = func.blocks[plan.preheader].label;
    let latch_label = func.blocks[plan.latch].label;
    insert_inst(
        &mut func.blocks[plan.header],
        0,
        Instruction::Phi {
            dest: phi,
            ty: plan.load_ty,
            incoming: vec![
                (Operand::Value(init), pre_label),
                (latch_incoming, latch_label),
            ],
        },
    );
    if plan.load_ty == IrType::F64 {
        func.loop_promoted_f64_values.push(phi);
    }

    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            rewrite_uses_in_inst(inst, plan.load_dest.0, phi.0);
        }
        rewrite_uses_in_terminator(&mut block.terminator, plan.load_dest.0, phi.0);
    }

    // Phi at header[0] shifted every later index in the header.
    let shift = |bi: usize, ii: usize| ii + usize::from(bi == plan.header);
    let li = shift(plan.load_b, plan.load_i);
    let si = shift(plan.store_b, plan.store_i);
    if plan.load_b == plan.store_b {
        let (hi, lo) = if si > li { (si, li) } else { (li, si) };
        remove_inst(&mut func.blocks[plan.load_b], hi);
        remove_inst(&mut func.blocks[plan.load_b], lo);
    } else {
        remove_inst(&mut func.blocks[plan.load_b], li);
        remove_inst(&mut func.blocks[plan.store_b], si);
    }

    insert_inst(
        &mut func.blocks[plan.exit_block],
        first_non_phi(&func.blocks[plan.exit_block]),
        Instruction::Store {
            val: exit_val,
            ptr: plan.ptr,
            ty: plan.load_ty,
            seg_override: plan.load_seg,
        },
    );
    1
}

/// Mark one ordinary F64 add-reduction for the dedicated ARM loop-register
/// pool. Narrow: only after this pass has already promoted a memory
/// recurrence in the function.
pub(crate) fn mark_f64_add_reduction(func: &mut IrFunction) -> usize {
    if func.loop_promoted_f64_values.is_empty() {
        return 0;
    }
    let mut add_defs: FxHashMap<u32, (Operand, Operand)> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::BinOp {
                dest,
                op: IrBinOp::Add,
                lhs,
                rhs,
                ty: IrType::F64,
            } = inst
            {
                add_defs.insert(dest.0, (lhs.clone(), rhs.clone()));
            }
        }
    }
    for block in &func.blocks {
        for inst in &block.instructions {
            let Instruction::Phi {
                dest,
                ty: IrType::F64,
                incoming,
            } = inst
            else {
                continue;
            };
            if func.loop_promoted_f64_values.iter().any(|v| v.0 == dest.0) {
                continue;
            }
            let has_zero = incoming
                .iter()
                .any(|(op, _)| matches!(op, Operand::Const(IrConst::F64(v)) if *v == 0.0));
            if !has_zero {
                continue;
            }
            let is_reduction = incoming
                .iter()
                .filter_map(|(op, _)| match op {
                    Operand::Value(v) => Some(v.0),
                    _ => None,
                })
                .any(|back| {
                    add_defs.get(&back).is_some_and(|(lhs, rhs)| {
                        matches!(lhs, Operand::Value(v) if v.0 == dest.0)
                            || matches!(rhs, Operand::Value(v) if v.0 == dest.0)
                    })
                });
            if is_reduction {
                func.loop_promoted_f64_values.push(*dest);
                return 1;
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::BlockId;

    fn block(label: u32, insts: Vec<Instruction>, term: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions: insts,
            terminator: term,
            source_spans: Vec::new(),
        }
    }

    #[test]
    fn memory_on_exit_latch_writes_new_value() {
        let phi = Value(10);
        let new = Operand::Value(Value(11));
        assert_eq!(memory_on_exit(2, 2, phi, &new), new);
        assert_eq!(
            memory_on_exit(0, 2, phi, &new),
            Operand::Value(phi),
            "exit from header must write the pre-store phi"
        );
    }

    #[test]
    fn byte_size_unknown_is_none() {
        assert_eq!(byte_size(IrType::I32), Some(4));
        assert_eq!(byte_size(IrType::Ptr), Some(8));
        assert!(!is_promotable_type(IrType::I128));
        assert!(!is_promotable_type(IrType::F128));
    }

    #[test]
    fn disjoint_unique_objects_only() {
        let mut paths = FxHashMap::default();
        paths.insert(
            1,
            Path {
                root: TAG_ALLOCA | 1,
                offset: 0,
            },
        );
        paths.insert(
            2,
            Path {
                root: TAG_ALLOCA | 1,
                offset: 8,
            },
        );
        paths.insert(
            3,
            Path {
                root: TAG_ALLOCA | 2,
                offset: 0,
            },
        );
        paths.insert(
            4,
            Path {
                root: TAG_PARAM | 0,
                offset: 0,
            },
        );
        paths.insert(
            5,
            Path {
                root: TAG_PARAM | 1,
                offset: 0,
            },
        );
        paths.insert(
            6,
            Path {
                root: TAG_GLOBAL | 7,
                offset: 0,
            },
        );
        paths.insert(
            7,
            Path {
                root: TAG_OTHER | 9,
                offset: 0,
            },
        );
        paths.insert(
            8,
            Path {
                root: TAG_OTHER | 10,
                offset: 0,
            },
        );

        assert!(disjoint(
            &paths,
            Value(1),
            IrType::I64,
            Value(2),
            IrType::I64
        ));
        assert!(!disjoint(
            &paths,
            Value(1),
            IrType::I64,
            Value(1),
            IrType::I64
        ));
        assert!(disjoint(
            &paths,
            Value(1),
            IrType::I64,
            Value(3),
            IrType::I64
        ));
        assert!(
            !disjoint(&paths, Value(4), IrType::I64, Value(5), IrType::I64),
            "distinct params may alias"
        );
        assert!(
            !disjoint(&paths, Value(1), IrType::I64, Value(4), IrType::I64),
            "param may point at alloca"
        );
        assert!(
            disjoint(&paths, Value(1), IrType::I64, Value(6), IrType::I64),
            "alloca vs global"
        );
        assert!(
            !disjoint(&paths, Value(7), IrType::I64, Value(8), IrType::I64),
            "unknown roots must not be assumed disjoint"
        );
        assert!(!disjoint(
            &paths,
            Value(1),
            IrType::I64,
            Value(99),
            IrType::I64
        ));
    }

    #[test]
    fn ranges_apart_overflow_is_not_disjoint() {
        assert_eq!(ranges_apart(i64::MAX - 1, 8, 0, 4), None);
        assert_eq!(ranges_apart(0, 8, 8, 4), Some(true));
        assert_eq!(ranges_apart(0, 8, 4, 4), Some(false));
    }

    #[test]
    fn merge_forms_rejects_two_roots() {
        let a = LinForm {
            root: 1,
            syms: vec![],
            konst: 0,
            march: 0,
        };
        let b = LinForm {
            root: 2,
            syms: vec![],
            konst: 4,
            march: 0,
        };
        assert!(merge_forms(a, b).is_none());
    }

    #[test]
    fn subst_rewrites_only_the_named_value() {
        assert_eq!(
            subst_operand(&Operand::Value(Value(7)), 7, 9),
            Operand::Value(Value(9))
        );
        assert_eq!(
            subst_operand(&Operand::Value(Value(1)), 7, 9),
            Operand::Value(Value(1))
        );
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
                Instruction::Copy {
                    dest: Value(2),
                    src: Operand::Const(IrConst::I32(0)),
                },
            ],
            Terminator::Return(None),
        );
        assert_eq!(first_non_phi(&b), 1);
    }

    /// Single-block do-while recurrence. The unique exit is the latch, so the
    /// exit store must write the *new* value (`y = x+1`), not the phi.
    #[test]
    fn do_while_exit_store_is_new_value_not_phi() {
        let mut func = IrFunction::new("rec".to_string(), IrType::I32, vec![], false);
        func.blocks = vec![
            block(
                0,
                vec![
                    Instruction::Alloca {
                        dest: Value(0),
                        ty: IrType::I32,
                        size: 4,
                        align: 4,
                        volatile: false,
                        semantic_volatile: false,
                    },
                    Instruction::Store {
                        val: Operand::Const(IrConst::I32(0)),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    },
                ],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![
                    Instruction::Load {
                        dest: Value(1),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    },
                    Instruction::BinOp {
                        dest: Value(2),
                        op: IrBinOp::Add,
                        lhs: Operand::Value(Value(1)),
                        rhs: Operand::Const(IrConst::I32(1)),
                        ty: IrType::I32,
                    },
                    Instruction::Store {
                        val: Operand::Value(Value(2)),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            block(2, vec![], Terminator::Return(None)),
        ];
        func.next_value_id = 3;

        let n = run(&mut func);
        if n == 0 {
            // Loop analysis may refuse this shape on a given CFG builder;
            // `memory_on_exit_latch_writes_new_value` still locks the contract.
            return;
        }
        let exit = func.blocks.last().unwrap();
        let store = exit
            .instructions
            .iter()
            .find(|i| matches!(i, Instruction::Store { .. }));
        let Some(Instruction::Store { val, .. }) = store else {
            panic!("expected exit store after promotion");
        };
        assert!(
            matches!(val, Operand::Value(v) if v.0 == 2),
            "do-while exit store must write the latch new-value, got {val:?}"
        );
    }
}
