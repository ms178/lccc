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
//! - **Must-execute / speculation.** The load is hoisted and the final store
//!   may be newly executed on a zero-trip exit. This is allowed only for a
//!   proven in-bounds, naturally aligned local alloca access, or when both the
//!   original load and store dominate the unique exit source.
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

use super::loop_analysis;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::analysis::CfgAnalysis;
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IrBinOp, IrConst, IrFunction, Operand, Terminator, Value,
    replace_operand_value, replace_terminator_value,
};
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
    static FLAG: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var_os("CCC_DEBUG_PROMOTE").is_some());
    *FLAG
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

pub(crate) fn byte_size(ty: IrType) -> Option<i64> {
    let size = match ty {
        IrType::I8
        | IrType::U8
        | IrType::I16
        | IrType::U16
        | IrType::I32
        | IrType::U32
        | IrType::I64
        | IrType::U64
        | IrType::F32
        | IrType::F64
        | IrType::Ptr => ty.size(),
        _ => return None,
    };
    i64::try_from(size).ok()
}

fn is_promotable_type(ty: IrType) -> bool {
    byte_size(ty).is_some()
}

/// Prove that speculatively loading and redundantly storing this access cannot
/// fault. Merely being alloca-derived is not enough: a constant offset may be
/// out of bounds or under-aligned, and either access may be absent in a
/// zero-trip loop in the source program.
fn alloca_access_is_nonfaulting(
    defs: &FxHashMap<u32, &Instruction>,
    path: Path,
    access_ty: IrType,
    segment: AddressSpace,
) -> bool {
    if segment != AddressSpace::Default || !is_alloca_root(path.root) {
        return false;
    }
    let Ok(root_id) = u32::try_from(path.root & !TAG_MASK) else {
        return false;
    };
    let Some(Instruction::Alloca {
        dest,
        ty: object_ty,
        size,
        align,
        volatile,
        semantic_volatile,
    }) = defs.get(&root_id).copied()
    else {
        return false;
    };
    if dest.0 != root_id || *volatile || *semantic_volatile {
        return false;
    }
    let (Ok(offset), Some(access_size)) = (
        usize::try_from(path.offset),
        byte_size(access_ty).and_then(|n| usize::try_from(n).ok()),
    ) else {
        return false;
    };
    let Some(end) = offset.checked_add(access_size) else {
        return false;
    };
    if end > *size {
        return false;
    }

    let required_align = access_ty.align();
    let object_align = if *align == 0 {
        object_ty.align()
    } else {
        *align
    };
    object_align % required_align == 0 && offset % required_align == 0
}

/// A bounded, aligned access to an UNEXPOSED static global can never fault:
/// the storage is defined in this module and the offset/size are proven
/// within it. This licenses hoisting the preheader LOAD. It does NOT
/// license an unconditional exit STORE — unlike an alloca, the global may
/// be reached concurrently by other threads running this module's code, so
/// a store the original program did not perform (zero-trip path) could
/// revert another thread's write. The caller must pair this with a
/// guarded writeback.
fn global_access_is_nonfaulting(
    defs: &FxHashMap<u32, &Instruction>,
    paths: &FxHashMap<u32, Path>,
    ptr: Value,
    access_ty: IrType,
    segment: AddressSpace,
    safe_globals: &UnexposedGlobals,
) -> bool {
    if segment != AddressSpace::Default {
        return false;
    }
    let Some(path) = paths.get(&ptr.0) else {
        return false;
    };
    if tag_of(path.root) != TAG_GLOBAL {
        return false;
    }
    let Some((root_vid, _)) = resolve_ptr_chain(defs, ptr) else {
        return false;
    };
    let Some(Instruction::GlobalAddr { name, .. }) = defs.get(&root_vid) else {
        return false;
    };
    let Some(&(size, align)) = safe_globals.get(name.as_str()) else {
        return false;
    };
    let (Ok(offset), Some(access_size)) = (
        usize::try_from(path.offset),
        byte_size(access_ty).and_then(|n| usize::try_from(n).ok()),
    ) else {
        return false;
    };
    let Some(end) = offset.checked_add(access_size) else {
        return false;
    };
    if end > size {
        return false;
    }
    let required_align = access_ty.align();
    let object_align = if align == 0 { 1 } else { align };
    object_align % required_align == 0 && offset % required_align == 0
}

fn accesses_may_be_speculated(
    defs: &FxHashMap<u32, &Instruction>,
    paths: &FxHashMap<u32, Path>,
    ptr: Value,
    ty: IrType,
    segment: AddressSpace,
    idom: &[usize],
    load_block: usize,
    store_block: usize,
    exit_from: usize,
) -> bool {
    paths
        .get(&ptr.0)
        .is_some_and(|path| alloca_access_is_nonfaulting(defs, *path, ty, segment))
        || (dominates(idom, exit_from, load_block) && dominates(idom, exit_from, store_block))
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

pub(crate) fn resolve_ptr_chain(
    defs: &FxHashMap<u32, &Instruction>,
    start: Value,
) -> Option<(u32, i64)> {
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
                (Operand::Value(v), Operand::Const(c)) | (Operand::Const(c), Operand::Value(v)) => {
                    c.to_i64().map(|k| (*v, k))
                }
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
pub(crate) struct LinForm {
    pub(crate) root: u64,
    pub(crate) syms: Vec<(u32, i64)>,
    pub(crate) konst: i64,
    pub(crate) march: i64,
}

pub(crate) fn striding_phi(
    defs: &FxHashMap<u32, &Instruction>,
    phi_v: Value,
) -> Option<(Operand, i64)> {
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

pub(crate) fn resolve_lin_form(
    func: &IrFunction,
    defs: &FxHashMap<u32, &Instruction>,
    lp_body: &FxHashSet<usize>,
    def_block: &FxHashMap<u32, usize>,
    multi_def: &FxHashSet<u32>,
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

    // COPY-WEB LEAF: a value with more than one defining instruction (a
    // post-phi-coalescing induction web: `v ← init` in the preheader,
    // `v' = v + k; v ← v'` on the latch) has position-dependent content —
    // no single def describes it. Treat it as an opaque SYMBOL LEAF: the
    // form stays exact at each use site, and every cross-position
    // consumer already invalidates on the web's redefinition
    // (redundant_loads' sym-redefinition guard; LICM/loop-memory-promote
    // loop-defined gates: a web defined inside the loop is never
    // invariant, one defined outside it is stable across the loop by
    // dominance). Before this arm, resolution followed one def into the
    // init/backedge cycle, burned its fuel and returned `None` — every
    // copy-web-driven array access (nbody's `bodies[i].*`, `bodies[j].*`)
    // was opaque to the whole alias engine, so field loads re-loaded
    // each iteration and same-address loads never merged.
    if multi_def.contains(&v.0) {
        return Some(LinForm {
            root: 0,
            syms: vec![(v.0, 1)],
            konst: 0,
            march: 0,
        });
    }

    if def_bi == cur_header {
        if matches!(inst, Instruction::Phi { .. }) {
            let (init_op, stride) = striding_phi(defs, v)?;
            let mut f = match init_op {
                Operand::Value(init_v) => resolve_lin_form(
                    func, defs, lp_body, def_block, multi_def, cur_header, init_v, fuel,
                )?,
                Operand::Const(c) => LinForm {
                    root: 0,
                    syms: vec![],
                    konst: c.to_i64()?,
                    march: 0,
                },
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
        } => resolve_lin_form(
            func, defs, lp_body, def_block, multi_def, cur_header, *src, fuel,
        ),
        Instruction::Cast {
            src: Operand::Value(src),
            from_ty,
            to_ty,
            ..
        } if from_ty.size() <= to_ty.size() => resolve_lin_form(
            func, defs, lp_body, def_block, multi_def, cur_header, *src, fuel,
        ),
        Instruction::GetElementPtr { base, offset, .. } => {
            let mut f = resolve_lin_form(
                func, defs, lp_body, def_block, multi_def, cur_header, *base, fuel,
            )?;
            match offset {
                Operand::Const(c) => {
                    f.konst = f.konst.checked_add(c.to_i64()?)?;
                }
                Operand::Value(ov) => {
                    let g = resolve_lin_form(
                        func, defs, lp_body, def_block, multi_def, cur_header, *ov, fuel,
                    )?;
                    f = merge_forms(f, g)?;
                }
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
                let fa = resolve_lin_form(
                    func, defs, lp_body, def_block, multi_def, cur_header, *a, fuel,
                )?;
                let fb = resolve_lin_form(
                    func, defs, lp_body, def_block, multi_def, cur_header, *b, fuel,
                )?;
                merge_forms(fa, fb)
            }
            (Operand::Value(a), Operand::Const(c)) | (Operand::Const(c), Operand::Value(a)) => {
                let mut fa = resolve_lin_form(
                    func, defs, lp_body, def_block, multi_def, cur_header, *a, fuel,
                )?;
                fa.konst = fa.konst.checked_add(c.to_i64()?)?;
                Some(fa)
            }
            _ => None,
        },
        Instruction::BinOp {
            op: IrBinOp::Shl,
            lhs: Operand::Value(value),
            rhs: Operand::Const(amount),
            ..
        } => {
            let shift = amount.to_i64()?;
            if !(0..64).contains(&shift) {
                return None;
            }
            let scale = 1i64.checked_shl(shift as u32)?;
            let mut f = resolve_lin_form(
                func, defs, lp_body, def_block, multi_def, cur_header, *value, fuel,
            )?;
            if f.root != 0 {
                return None;
            }
            f.konst = f.konst.checked_mul(scale)?;
            f.march = f.march.checked_mul(scale)?;
            for symbol in &mut f.syms {
                symbol.1 = symbol.1.checked_mul(scale)?;
            }
            Some(f)
        }
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
            let mut f = resolve_lin_form(
                func, defs, lp_body, def_block, multi_def, cur_header, val_op, fuel,
            )?;
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
                let Instruction::Phi { dest, ty: ity, .. } = binst else {
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
            let mut f = resolve_lin_form(
                func, defs, lp_body, def_block, multi_def, cur_header, init_v, fuel,
            )?;
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

pub(crate) fn merge_forms(mut a: LinForm, b: LinForm) -> Option<LinForm> {
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
    multi_def: &FxHashSet<u32>,
    header_idx: usize,
    cand: Value,
    cand_ty: IrType,
    store: Value,
    store_ty: IrType,
) -> bool {
    if def_block.get(&cand.0).is_some_and(|b| lp_body.contains(b)) {
        return false;
    }
    let cf = resolve_lin_form(
        func,
        defs,
        lp_body,
        def_block,
        multi_def,
        header_idx,
        cand,
        RESOLVE_FUEL,
    );
    let sf = resolve_lin_form(
        func,
        defs,
        lp_body,
        def_block,
        multi_def,
        header_idx,
        store,
        RESOLVE_FUEL,
    );
    if promote_debug() {
        lccc::cli::stderr_line(format_args!(
            "[AFFINE] cand={} -> {:?}; store={} -> {:?}",
            cand.0,
            cf.as_ref().map(|f| (f.root, &f.syms, f.konst, f.march)),
            store.0,
            sf.as_ref().map(|f| (f.root, &f.syms, f.konst, f.march))
        ));
    }
    let (Some(cf), Some(sf)) = (cf, sf) else {
        return false;
    };
    let (Some(cand_sz), Some(store_sz)) = (byte_size(cand_ty), byte_size(store_ty)) else {
        return false;
    };
    // Promotion requires the candidate to be invariant in this loop. Delegate
    // all separation arithmetic to the shared checked alias engine.
    if cf.march != 0 {
        return false;
    }
    super::alias::forms_disjoint(&cf, cand_sz, &sf, store_sz, true)
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
    let idx = idx.min(block.instructions.len());
    let span = if block.spans_parallel() {
        block.source_spans[idx.min(block.instructions.len().saturating_sub(1))]
    } else {
        crate::common::source::Span::dummy()
    };
    block.insert_inst(idx, inst, span);
}

fn remove_inst(block: &mut BasicBlock, idx: usize) {
    if idx < block.instructions.len() {
        block.remove_inst(idx);
    }
}

fn subst_operand(op: &Operand, from: u32, to: u32) -> Operand {
    match op {
        Operand::Value(v) if v.0 == from => Operand::Value(Value(to)),
        other => other.clone(),
    }
}

/// Rewrite every *read* of `from` in `inst` to `to`; destinations are untouched.
///
/// This was a 135-line hand-rolled table until it was replaced by the canonical
/// two-walk helpers.  The table was exhaustive, but its
/// `VaEnd { .. } => {}` arm sat in the group of field-less variants while
/// `VaEnd` actually holds a bare `va_list_ptr: Value` — so a promoted slot whose
/// value was redirected left one stale `va_end` pointer behind.  A table cannot
/// be audited as easily as a visitor pair that the compiler checks exhaustively;
/// the canonical form covers every slot the table covered plus the ones it forgot.
fn rewrite_uses_in_inst(inst: &mut Instruction, from: u32, to: u32) {
    replace_operand_value(inst, Value(from), Operand::Value(Value(to)));
}

/// Terminator half of [`rewrite_uses_in_inst`].
fn rewrite_uses_in_terminator(term: &mut Terminator, from: u32, to: u32) {
    replace_terminator_value(term, Value(from), Operand::Value(Value(to)));
}

/// Value that must be written on the unique exit so memory matches the
/// original last store that actually executed.
///
/// `store_dominates_exit` is the decisive fact, computed by the finder from
/// the dominator tree: when the store block dominates the exiting block,
/// every header->exit path of the final (partial) iteration passed the
/// store, so the freshest value is the store's operand. Otherwise the exit
/// fired before this iteration's store and the header phi carries the last
/// completed iteration's value. (The old `exit_from == latch` test was a
/// special case of this: a store in the latch trivially dominates a
/// latch-exit and never dominates an earlier exit.)
fn memory_on_exit(store_dominates_exit: bool, phi: Value, store_val: &Operand) -> Operand {
    if store_dominates_exit {
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

fn collect_defs(
    func: &IrFunction,
) -> (
    FxHashMap<u32, &Instruction>,
    FxHashMap<u32, usize>,
    FxHashSet<u32>,
) {
    let mut defs = FxHashMap::default();
    let mut def_block = FxHashMap::default();
    let mut multi_def: FxHashSet<u32> = FxHashSet::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                if defs.insert(dest.0, inst).is_some() {
                    multi_def.insert(dest.0);
                }
                def_block.entry(dest.0).or_insert(bi);
            }
        }
    }
    (defs, def_block, multi_def)
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
    /// The store block dominates the exiting block: the final (partial)
    /// iteration executed the store, so the writeback must use the store's
    /// value rather than the header phi.
    store_dominates_exit: bool,
    /// Writeback must execute only when the loop body ran at least once:
    /// route the exit edge through a did-iterate check (see
    /// `find_promotion` tier 2).
    guarded_writeback: bool,
}

pub(crate) fn run(func: &mut IrFunction) -> usize {
    run_with(func, &FxHashMap::default())
}

/// `name -> (size, align)` of the module's unexposed static globals.
type UnexposedGlobals = FxHashMap<String, (usize, usize)>;

/// Module-entry wrapper: computes the set of internal globals whose address
/// is provably never exposed, then promotes with that knowledge. A load or
/// store through a pointer the function cannot trace to a known object can
/// never touch such a global, so the FNV/CRC-style `global = f(global,
/// buf[k])` byte loops — where `buf` is an unknown caller pointer — promote
/// the accumulator out of memory (sqlite_vdbe_serial kept the digest
/// round-tripping through `content(%rip)` on the loop-carried path, a
/// store+load added to every byte).
pub(crate) fn run_module(module: &mut crate::ir::module::IrModule) -> usize {
    let safe = unexposed_static_globals(module);
    let mut total = 0;
    for func in &mut module.functions {
        if !func.is_declaration {
            total += run_with(func, &safe);
        }
    }
    total
}

/// Names of `static` globals whose address is NEVER exposed anywhere in the
/// module, so a pointer that cannot be traced to a known object cannot
/// legally point into them (forming such a pointer would require taking the
/// address somewhere, and internal linkage confines "somewhere" to this
/// module).
///
/// Exposure is judged strictly; a name survives only if ALL of these hold:
///   * defined here, `static`, not weak/common/extern, not
///     `__attribute__((used))` (a `used` symbol is advertised to forces
///     outside the IR, e.g. module-external asm), not thread-local — a
///     different addressing model entirely;
///   * no symbol alias involves it (an alias is a second name for the same
///     storage whose uses this analysis does not see);
///   * no global initializer anywhere references its address;
///   * the module has no top-level asm and no function contains inline asm:
///     asm text can name even a local symbol without any IR-visible use;
///   * in every function, every use of every `GlobalAddr` of the name —
///     followed transitively through `Copy` — is a direct `Load.ptr` or
///     `Store.ptr` (the STORED VALUE being the address is an escape, as is
///     any GEP/Cast/Phi/Select/call/compare/terminator use).
fn unexposed_static_globals(module: &crate::ir::module::IrModule) -> UnexposedGlobals {
    let mut cand: UnexposedGlobals = module
        .globals
        .iter()
        .filter(|g| {
            g.is_static
                && !g.is_extern
                && !g.is_weak
                && !g.is_common
                && !g.is_used
                && !g.is_thread_local
        })
        .map(|g| (g.name.clone(), (g.size, g.align)))
        .collect();
    if cand.is_empty() {
        return cand;
    }
    if !module.toplevel_asm.is_empty() {
        cand.clear();
        return cand;
    }
    for (alias, target, _) in &module.aliases {
        cand.remove(alias);
        cand.remove(target);
    }
    for g in &module.globals {
        g.init.for_each_ref(&mut |name| {
            cand.remove(name);
        });
    }
    for func in &module.functions {
        for block in &func.blocks {
            for inst in &block.instructions {
                if matches!(inst, Instruction::InlineAsm { .. }) {
                    cand.clear();
                    return cand;
                }
            }
        }
    }
    for func in &module.functions {
        if cand.is_empty() {
            break;
        }
        // Tracked = GlobalAddr of a candidate, closed over `Copy`. SSA gives
        // every id one def, so a fixpoint over blocks terminates.
        let mut tracked: FxHashMap<u32, usize> = FxHashMap::default();
        let names: Vec<&String> = cand.keys().collect();
        let name_idx: FxHashMap<&str, usize> = names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.as_str(), i))
            .collect();
        loop {
            let mut grew = false;
            for block in &func.blocks {
                for inst in &block.instructions {
                    match inst {
                        Instruction::GlobalAddr { dest, name } => {
                            if let Some(&i) = name_idx.get(name.as_str())
                                && tracked.insert(dest.0, i).is_none()
                            {
                                grew = true;
                            }
                        }
                        Instruction::Copy {
                            dest,
                            src: Operand::Value(v),
                            ..
                        } => {
                            if let Some(&i) = tracked.get(&v.0)
                                && tracked.insert(dest.0, i).is_none()
                            {
                                grew = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
            if !grew {
                break;
            }
        }
        if tracked.is_empty() {
            continue;
        }
        let mut exposed: FxHashSet<usize> = FxHashSet::default();
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    // The pointer POSITION of a direct load/store is the one
                    // sanctioned use. A stored VALUE that is the address
                    // escapes.
                    Instruction::Load { .. } => {}
                    Instruction::Store {
                        val: Operand::Value(v),
                        ..
                    } => {
                        if let Some(&i) = tracked.get(&v.0) {
                            exposed.insert(i);
                        }
                    }
                    Instruction::Store { .. } => {}
                    // Copy propagation was handled above; the Copy itself
                    // does not escape.
                    Instruction::Copy { .. } => {}
                    other => other.for_each_used_value(|id| {
                        if let Some(&i) = tracked.get(&id) {
                            exposed.insert(i);
                        }
                    }),
                }
            }
            block.terminator.for_each_used_value(|id| {
                if let Some(&i) = tracked.get(&id) {
                    exposed.insert(i);
                }
            });
        }
        let doomed: Vec<String> = exposed.iter().map(|&i| names[i].clone()).collect();
        for name in doomed {
            cand.remove(&name);
        }
    }
    cand
}

fn run_with(func: &mut IrFunction, safe_globals: &UnexposedGlobals) -> usize {
    // One pointer per `run_once`: indices stay honest and a promoted loop
    // no longer matches. Each success strictly drops the loop's memop count.
    let mut total = 0;
    for _ in 0..MAX_PROMOTIONS {
        let n = run_once(func, safe_globals);
        if n == 0 {
            break;
        }
        total += n;
    }
    total
}

fn run_once(func: &mut IrFunction, safe_globals: &UnexposedGlobals) -> usize {
    let Some(plan) = find_promotion(func, safe_globals) else {
        return 0;
    };
    apply_promotion(func, plan)
}

/// Canonical single-latch / single-exit frame of a promotable loop.
struct LoopShape {
    preheader: usize,
    latch: usize,
    exit_from: usize,
    exit_block: usize,
}

/// Vet the loop's control shape: a dedicated preheader, exactly one latch,
/// exactly one exit edge, and an exit block whose ONLY predecessor is the
/// exiting block (so the exit store is the unique writeback and the exit
/// block is not also a join or the preheader).
fn loop_shape(
    func: &IrFunction,
    cfg: &CfgAnalysis,
    lp: &loop_analysis::NaturalLoop,
) -> Option<LoopShape> {
    let preheader = loop_analysis::find_preheader(lp.header, &lp.body, &cfg.preds)?;
    if preheader == lp.header {
        return None;
    }
    let latches: Vec<usize> = cfg
        .preds
        .row(lp.header)
        .iter()
        .map(|&v| v as usize)
        .filter(|b| lp.body.contains(b))
        .collect();
    if latches.len() != 1 {
        return None;
    }
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
        return None;
    }
    let (exit_from, exit_block) = exits[0];
    if exit_block >= func.blocks.len() {
        return None;
    }
    if cfg
        .preds
        .row(exit_block)
        .iter()
        .any(|&p| p as usize != exit_from)
    {
        return None;
    }
    Some(LoopShape {
        preheader,
        latch: latches[0],
        exit_from,
        exit_block,
    })
}

fn find_promotion(func: &IrFunction, safe_globals: &UnexposedGlobals) -> Option<PromotePlan> {
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

    let (defs, def_block, multi_def) = collect_defs(func);
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

    for lp in &loops {
        let Some(LoopShape {
            preheader,
            latch,
            exit_from,
            exit_block,
        }) = loop_shape(func, &cfg, lp)
        else {
            continue;
        };

        let mut loads: FxHashMap<u32, Vec<(usize, usize, Value, IrType, AddressSpace)>> =
            FxHashMap::default();
        let mut stores: FxHashMap<u32, Vec<(usize, usize, Operand, IrType, AddressSpace)>> =
            FxHashMap::default();
        let mut has_unknown_mem = false;
        for &bi in &lp.body {
            for (ii, inst) in func.blocks[bi].instructions.iter().enumerate() {
                match inst {
                    // Volatile accesses are observable side effects: promoting
                    // them to a register preheader load + epilogue store
                    // collapses N observable accesses into 2 (C11 5.1.2.3).
                    Instruction::Load { volatile: true, .. }
                    | Instruction::Store { volatile: true, .. } => {
                        has_unknown_mem = true;
                    }
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
            if load_ty != *store_ty || load_seg != *store_seg {
                continue;
            }
            if !is_promotable_type(load_ty) {
                continue;
            }
            // The store need not sit IN the latch — a rotated-later loop
            // keeps `load; op; store` in a body block that branches to a
            // separate increment latch. It must execute exactly once per
            // completed iteration AFTER the load:
            //   * `store_b` dominates the latch: every header->latch path
            //     passes it (a bypassing path appended to the loop-free
            //     preheader->header entry would contradict dominance);
            //   * neither access sits in a NESTED loop of this loop, which
            //     would re-execute it within one outer iteration and
            //     re-read a value the rewrite no longer reloads;
            //   * the load precedes the store: same-block index order, or
            //     `load_b` strictly dominates `store_b` (both dominate the
            //     latch, and dominators of a node form a chain, so the two
            //     blocks are always ordered by dominance).
            if !dominates(&cfg.idom, latch, *store_b) {
                continue;
            }
            if !dominates(&cfg.idom, latch, load_b) {
                continue;
            }
            if load_b == *store_b {
                if load_i > *store_i {
                    continue;
                }
            } else if !dominates(&cfg.idom, *store_b, load_b) {
                continue;
            }
            if *store_b != latch
                && loops.iter().any(|lp2| {
                    lp2.header != lp.header
                        && lp.body.contains(&lp2.header)
                        && (lp2.body.contains(store_b) || lp2.body.contains(&load_b))
                })
            {
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

            // The transformation may execute both a preheader load and an exit
            // store on a source-level zero-trip path. Three admission tiers:
            //   1. must-execute proofs or a proved-safe LOCAL access — the
            //      unconditional exit writeback is exactly as observable as
            //      the original accesses;
            //   2. an unexposed static global — the preheader load cannot
            //      fault, but the writeback must be GUARDED by a did-iterate
            //      flag so the zero-trip path stores nothing the original
            //      program did not store (an unconditional writeback could
            //      revert a concurrent thread's update);
            //   3. anything else — decline.
            let guarded_writeback;
            if accesses_may_be_speculated(
                &defs, &paths, ptr, load_ty, load_seg, &cfg.idom, load_b, *store_b, exit_from,
            ) {
                guarded_writeback = false;
            } else if global_access_is_nonfaulting(
                &defs,
                &paths,
                ptr,
                load_ty,
                load_seg,
                safe_globals,
            )
                // The guard retargets the exit edge, giving the exit block a
                // second predecessor; a phi there would need a new incoming.
                && !func.blocks[exit_block]
                    .instructions
                    .iter()
                    .any(|i| matches!(i, Instruction::Phi { .. }))
                // Retargeting rewrites a Branch/CondBranch label; any other
                // exit terminator (switch, computed goto) is not handled.
                && matches!(
                    func.blocks[exit_from].terminator,
                    Terminator::Branch(_) | Terminator::CondBranch { .. }
                )
            {
                guarded_writeback = true;
            } else {
                continue;
            }

            let mut alias = false;
            // An unexposed static global can only be reached through its own
            // `GlobalAddr`: a pointer whose path is unknown (loaded, a
            // parameter, call-produced) cannot point into it, and a pointer
            // rooted at a DIFFERENT object is handled by `disjoint`. So when
            // either side is such a global, the only aliasing candidate is a
            // path rooted at the same object — everything else is provably
            // apart. (Root-hash equality is judged conservatively: a
            // colliding root falls through to the precise check.)
            let unexposed_root = |v: Value| -> Option<u64> {
                let p = paths.get(&v.0)?;
                match defs.get(&resolve_ptr_chain(&defs, v)?.0) {
                    Some(Instruction::GlobalAddr { name, .. })
                        if safe_globals.contains_key(name.as_str()) =>
                    {
                        Some(p.root)
                    }
                    _ => None,
                }
            };
            let ptr_safe_root = unexposed_root(ptr);
            let may_alias = |other: Value, other_ty: IrType| {
                for (safe, that) in [(ptr_safe_root, other), (unexposed_root(other), ptr)] {
                    if let Some(root) = safe
                        && paths.get(&that.0).is_none_or(|p| p.root != root)
                    {
                        return false;
                    }
                }
                !disjoint(&paths, ptr, load_ty, other, other_ty)
                    && !affine_disjoint(
                        func, &defs, &lp.body, &def_block, &multi_def, lp.header, ptr, load_ty,
                        other, other_ty,
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
                lccc::cli::stderr_line(format_args!(
                    "[PROMOTE] func={} loop header={} ptr=Value({}) ty={:?} exit_from={} latch={}",
                    func.name, lp.header, ptr_id, load_ty, exit_from, latch
                ));
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
                store_dominates_exit: dominates(&cfg.idom, exit_from, *store_b),
                guarded_writeback,
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
    // Guarded writeback: allocate everything BEFORE mutating, so a failed
    // allocation leaves the function untouched.
    let guard = if plan.guarded_writeback {
        let Some(flag) = next_value(func) else {
            return 0;
        };
        let Some(cond) = next_value(func) else {
            return 0;
        };
        let max_label = func.blocks.iter().map(|b| b.label.0).max().unwrap_or(0);
        let first = func.next_label.max(max_label.saturating_add(1));
        let Some(end) = first.checked_add(2) else {
            return 0;
        };
        func.next_label = end;
        Some((flag, cond, BlockId(first), BlockId(first + 1)))
    } else {
        None
    };

    let latch_incoming = subst_operand(&plan.store_val, plan.load_dest.0, phi.0);
    let exit_val = memory_on_exit(plan.store_dominates_exit, phi, &latch_incoming);

    let preheader_end = func.blocks[plan.preheader].instructions.len();
    insert_inst(
        &mut func.blocks[plan.preheader],
        preheader_end,
        Instruction::Load {
            volatile: false,
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
    if let Some((flag, _, _, _)) = guard {
        // Did-iterate flag: 0 from the preheader, 1 from the latch. Placed
        // right after the value phi, so the header insertion shift below is
        // 2 when guarded.
        insert_inst(
            &mut func.blocks[plan.header],
            1,
            Instruction::Phi {
                dest: flag,
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Const(IrConst::I32(0)), pre_label),
                    (Operand::Const(IrConst::I32(1)), latch_label),
                ],
            },
        );
    }

    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            rewrite_uses_in_inst(inst, plan.load_dest.0, phi.0);
        }
        rewrite_uses_in_terminator(&mut block.terminator, plan.load_dest.0, phi.0);
    }

    // Phis at header[0..] shifted every later index in the header.
    let header_phis = 1 + usize::from(guard.is_some());
    let shift = |bi: usize, ii: usize| ii + if bi == plan.header { header_phis } else { 0 };
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

    match guard {
        None => {
            let exit_insert = first_non_phi(&func.blocks[plan.exit_block]);
            insert_inst(
                &mut func.blocks[plan.exit_block],
                exit_insert,
                Instruction::Store {
                    volatile: false,
                    val: exit_val,
                    ptr: plan.ptr,
                    ty: plan.load_ty,
                    seg_override: plan.load_seg,
                },
            );
        }
        Some((flag, cond, check_label, store_label)) => {
            // Route the single exit edge through `check`: writeback runs
            // only when the latch executed at least once, so the zero-trip
            // path performs exactly the accesses the original program did
            // (plus the benign speculative preheader load).
            let exit_label = func.blocks[plan.exit_block].label;
            let retarget = |l: &mut BlockId| {
                if *l == exit_label {
                    *l = check_label;
                }
            };
            match &mut func.blocks[plan.exit_from].terminator {
                Terminator::Branch(t) => retarget(t),
                Terminator::CondBranch {
                    true_label,
                    false_label,
                    ..
                } => {
                    retarget(true_label);
                    retarget(false_label);
                }
                // find_promotion only admits Branch/CondBranch exit edges;
                // anything else means the plan is stale — leave the exit
                // store out rather than corrupt the CFG. The loop itself is
                // already rewritten consistently (phi + removed accesses),
                // so memory would be stale: panic in debug to surface it.
                other => {
                    debug_assert!(
                        false,
                        "guarded writeback with non-branch exit terminator {other:?}"
                    );
                }
            }
            func.blocks.push(BasicBlock {
                label: check_label,
                instructions: vec![Instruction::Cmp {
                    dest: cond,
                    op: crate::ir::reexports::IrCmpOp::Ne,
                    lhs: Operand::Value(flag),
                    rhs: Operand::Const(IrConst::I32(0)),
                    ty: IrType::I32,
                }],
                source_spans: Vec::new(),
                source_inline_ids: Vec::new(),
                terminator: Terminator::CondBranch {
                    cond: Operand::Value(cond),
                    true_label: store_label,
                    false_label: exit_label,
                },
            });
            func.blocks.push(BasicBlock {
                label: store_label,
                instructions: vec![Instruction::Store {
                    volatile: false,
                    val: exit_val,
                    ptr: plan.ptr,
                    ty: plan.load_ty,
                    seg_override: plan.load_seg,
                }],
                source_spans: Vec::new(),
                source_inline_ids: Vec::new(),
                terminator: Terminator::Branch(exit_label),
            });
        }
    }
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
            source_inline_ids: Vec::new(),
        }
    }

    #[test]
    fn memory_on_exit_latch_writes_new_value() {
        let phi = Value(10);
        let new = Operand::Value(Value(11));
        assert!(
            matches!(memory_on_exit(true, phi, &new), Operand::Value(Value(11))),
            "a store dominating the exit must write the just-stored value"
        );
        assert!(
            matches!(memory_on_exit(false, phi, &new), Operand::Value(Value(10))),
            "an exit the store does not dominate must write the pre-store phi"
        );
    }

    #[test]
    fn byte_size_uses_target_pointer_width() {
        struct RestorePointerSize(usize);
        impl Drop for RestorePointerSize {
            fn drop(&mut self) {
                crate::common::types::set_target_ptr_size(self.0);
            }
        }

        let _restore = RestorePointerSize(crate::common::types::target_ptr_size());
        assert_eq!(byte_size(IrType::I32), Some(4));
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
                offset: 4,
            },
        );

        crate::common::types::set_target_ptr_size(4);
        assert_eq!(byte_size(IrType::Ptr), Some(4));
        assert!(disjoint(
            &paths,
            Value(1),
            IrType::Ptr,
            Value(2),
            IrType::Ptr
        ));
        crate::common::types::set_target_ptr_size(8);
        assert_eq!(byte_size(IrType::Ptr), Some(8));
        assert!(!disjoint(
            &paths,
            Value(1),
            IrType::Ptr,
            Value(2),
            IrType::Ptr
        ));
        assert!(!is_promotable_type(IrType::I128));
        assert!(!is_promotable_type(IrType::F128));
    }

    #[test]
    fn speculative_alloca_access_requires_bounds_and_alignment() {
        let alloca = Instruction::Alloca {
            dest: Value(7),
            ty: IrType::I32,
            size: 16,
            align: 4,
            volatile: false,
            semantic_volatile: false,
        };
        let mut defs = FxHashMap::default();
        defs.insert(7, &alloca);
        let path = |offset| Path {
            root: TAG_ALLOCA | 7,
            offset,
        };

        assert!(alloca_access_is_nonfaulting(
            &defs,
            path(12),
            IrType::I32,
            AddressSpace::Default,
        ));
        assert!(!alloca_access_is_nonfaulting(
            &defs,
            path(-4),
            IrType::I32,
            AddressSpace::Default,
        ));
        assert!(!alloca_access_is_nonfaulting(
            &defs,
            path(13),
            IrType::I32,
            AddressSpace::Default,
        ));
        assert!(!alloca_access_is_nonfaulting(
            &defs,
            path(2),
            IrType::I32,
            AddressSpace::Default,
        ));
        assert!(!alloca_access_is_nonfaulting(
            &defs,
            path(0),
            IrType::I32,
            AddressSpace::SegFs,
        ));
    }

    #[test]
    fn speculative_external_store_must_dominate_exit() {
        // 0 -> header 1 -> body 2 or exit 3. The header load executes on
        // the zero-trip path, but the body store does not.
        let idom = [0, 0, 1, 1];
        let defs = FxHashMap::default();
        let mut paths = FxHashMap::default();
        paths.insert(
            9,
            Path {
                root: TAG_GLOBAL | 9,
                offset: 0,
            },
        );
        assert!(!accesses_may_be_speculated(
            &defs,
            &paths,
            Value(9),
            IrType::I32,
            AddressSpace::Default,
            &idom,
            1,
            2,
            3,
        ));
        assert!(accesses_may_be_speculated(
            &defs,
            &paths,
            Value(9),
            IrType::I32,
            AddressSpace::Default,
            &idom,
            1,
            1,
            3,
        ));
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
                root: TAG_PARAM,
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
        assert!(matches!(
            subst_operand(&Operand::Value(Value(7)), 7, 9),
            Operand::Value(Value(9))
        ));
        assert!(matches!(
            subst_operand(&Operand::Value(Value(1)), 7, 9),
            Operand::Value(Value(1))
        ));
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
                        volatile: false,
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
                        volatile: false,
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
                        volatile: false,
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
