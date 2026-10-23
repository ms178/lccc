//! Form 5 of `aggregate_sroa`: coalesce a whole-object copy whose source dies
//! at the copy, and rebuild a by-value aggregate image only where one is
//! actually needed.
//!
//! # The shape
//!
//! ```c
//! seq_t const sequence = decode(&state, ...);   /* struct-valued expression */
//! size_t n = exec(op, oend, sequence, ...);     /* struct passed by value    */
//! ```
//!
//! `exec` is inlined, so its parameter is *its own copy* of `sequence`: a
//! stack object `D` filled by a 24-byte `Memcpy` from the caller's object `S`,
//! and then read — and adjusted — field by field.  The copy exists only because
//! a by-value argument needs a private object, but it is placed before the
//! branch that decides whether a call happens at all, so its cost is paid on
//! every iteration.  `ZSTD_decompressSequences_default` — 87 % of the zstd
//! decoder's run time — has exactly this shape around `seq_t sequence`: 128
//! stack references in 709 instructions against GCC's 3, and 0.5x of GCC's
//! decode throughput.
//!
//! # Why the existing forms do not fire
//!
//! * Form 1 forwards loads through a `Memcpy`-written object, but only within a
//!   single block, only while the destination never escapes, and only for reads
//!   that no later store invalidates.  Here the destination is written *and*
//!   read in the inlined callee's blocks.
//! * Form 4 splits an object into per-field allocas, but it must refuse any
//!   object a `Memcpy` names, and it must refuse any object whose address is
//!   passed to a call — which is exactly what a by-value argument looks like.
//!
//! # The transform
//!
//! For `Memcpy { dest: D, src: S, size: N }` with `D` and `S` distinct,
//! exact-size, non-volatile allocas:
//!
//! 1. **`S` is private and dead after the copy.**  Its address never escapes, so
//!    the only observers are the `Load`s/`Store`s this pass can see, and *no*
//!    access to `S` can execute between the copy and a use of `D`.  The write to
//!    `S` performed by the copy is therefore its last observable effect: from
//!    that point `S` and `D` are interchangeable.  (This is the memory
//!    counterpart of a use-def chain, and it must be answered per program point:
//!    `[body writes] -> copy -> use` is fine even when the body blocks are
//!    reachable from the copy through the loop's back edge, because that path
//!    runs the copy again first.)
//! 2. **The copy dominates every use of `D`**, so every use observes bytes the
//!    copy wrote.
//! 3. Every use of `D` is a constant-offset scalar `Load`/`Store`, a
//!    constant-offset `GEP`, or the argument of a **by-value aggregate**
//!    parameter of a direct call (`struct_arg_sizes[i] == Some(N)`).  Anything
//!    else refuses the copy — not because the coalescing would be unsound, but
//!    because this form only pays off when the object ends up as independent
//!    scalar fields, which is what it needs to prove.
//!
//! Then `D` is **renamed to `S`** (one value substitution on the object's root,
//! so every derived pointer follows), the copy is deleted, and — if a by-value
//! call read `D` — a fresh image `I` is built at that call site with one
//! exact-width `Load`/`Store` pair per field.  `S` is now named by no `Memcpy`
//! and escapes to no call, so form 4 splits it and mem2reg promotes the fields
//! to SSA values: the aggregate's contents live in registers, and the
//! contiguous image is materialized only on the path that calls, which is
//! precisely GCC's code shape.
//!
//! # Soundness of the rename
//!
//! Replacing the root value `D` with `S` in every use makes the same machine
//! addresses be computed from `S`'s storage.  `S` and `D` have the same size and
//! the object is untyped, so every offset that was in bounds in `D` is in bounds
//! in `S`.  The only semantic difference is *which* storage the accesses reach,
//! and that is unobservable precisely when `S` has no observer after the copy:
//! the copy leaves the two objects byte-identical (1), and every use of `D` is
//! after the copy (2).  Aliasing is not an issue because both objects are
//! distinct allocas and `S` is not address-taken by anything the analysis cannot
//! see; a program that could tell `S` from `D` apart would have to read `S`
//! after the copy, which (1) rules out.
//!
//! The image `I` receives exactly the bytes the copy would have handed the
//! callee: the fields come from `S` at the same offsets, and the gap pieces
//! (bytes no access names) are copied rather than invented, so the callee's view
//! of the padding is the one it had before.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, EightbyteClass, IrType};
use crate::ir::reexports::{Instruction, IrConst, IrFunction, Operand, Value};

use super::{Insert, Plan, Scan, any_escape, const_i64, resolve, ty_size};

/// Largest object this form coalesces.  The transform is about *when* an
/// aggregate image exists, not about huge objects; an unbounded size would let
/// one pathological struct explode the instruction count.
const MAX_BYTES: i64 = 512;

/// Widest scalar this form copies.  A wider access (`i128`, a vector) would have
/// to be decomposed into several, which changes the instructions the *program*
/// performs; this form only moves bytes.
const MAX_PIECE: i64 = 8;

/// Upper bound on the field count of one object, so a struct accessed at every
/// byte offset cannot blow up the plan.
const MAX_FIELDS: usize = 64;

/// One field of the image: byte offset, width, and the type used to move it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Piece {
    off: i64,
    width: i64,
    ty: IrType,
}

fn scalar_ty(width: i64) -> Option<IrType> {
    match width {
        1 => Some(IrType::U8),
        2 => Some(IrType::U16),
        4 => Some(IrType::U32),
        8 => Some(IrType::U64),
        _ => None,
    }
}

/// A scalar access of `D`: `(block, index, offset, type)`.
#[derive(Clone, Copy)]
struct Access {
    block: usize,
    index: usize,
    off: i64,
    ty: IrType,
}

impl Access {
    fn width(&self) -> i64 {
        ty_size(self.ty)
    }
}

/// Everything gathered about one copy in one walk of the function.
struct Uses {
    /// Scalar accesses of `D` (loads and stores alike).
    d_accesses: Vec<Access>,
    /// Every address use of `S`, in any shape: these are the observers that
    /// must not run after the copy.
    s_uses: Vec<(usize, usize)>,
    /// By-value aggregate call uses of `D`: (block, index, arg index).
    byval: Vec<(usize, usize, usize)>,
}

/// Is `v` argument `ai` of `info` a by-value aggregate of exactly `size` bytes,
/// passed whole (`off == 0`), on a call whose ABI this form models?
fn byval_arg(info: &crate::ir::reexports::CallInfo, v: u32, size: i64, off: i64) -> Option<usize> {
    // `is_sret`/`is_fastcall`/`regparm` change the caller-side lowering, and an
    // `assume_library` call is folded or expanded against a different
    // prototype.  Refusing all four keeps the proof local.
    if off != 0 || info.assume_library || info.is_sret || info.is_fastcall || info.regparm.is_some()
    {
        return None;
    }
    for (i, arg) in info.args.iter().enumerate() {
        let Operand::Value(av) = arg else { continue };
        if av.0 != v {
            continue;
        }
        let Some(sz) = info.struct_arg_sizes.get(i).copied().flatten() else {
            return None;
        };
        if sz as i64 != size {
            return None;
        }
        // An empty class list (objects over 16 bytes) and `[Memory]` are the
        // plain memory image this form models.  A register-classified aggregate
        // is read eightbyte by eightbyte; still a read of the caller's object,
        // but it needs its own proof, so it refuses.
        if let Some(classes) = info.struct_arg_classes.get(i) {
            if !classes.is_empty() && !matches!(classes.as_slice(), [EightbyteClass::Memory]) {
                return None;
            }
        }
        return Some(i);
    }
    None
}

/// Classify every use of the two objects.  `Err` refuses the copy.
fn classify(
    func: &IrFunction,
    scan: &Scan,
    d_root: u32,
    s_root: u32,
    size: i64,
) -> Result<Uses, &'static str> {
    let mut u = Uses {
        d_accesses: Vec::new(),
        s_uses: Vec::new(),
        byval: Vec::new(),
    };
    let dbg = crate::passes::debug_agg_byval();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            // A variable-offset derivation of `D` has no fixed root, so a use
            // through it would be invisible to the allowlist below.  `S` may be
            // derived however the program likes: every use still resolves to
            // `S` for the "is it dead after the copy" question only if the root
            // is known, so those are rejected too.
            if let Instruction::GetElementPtr { base, offset, .. } = inst {
                let (root, _) = resolve(&scan.gep, base.0);
                if (root == d_root || root == s_root) && const_i64(offset).is_none() {
                    if dbg {
                        eprintln!("[byval]   variable-offset GEP of v{root}: {inst:?}");
                    }
                    return Err("variable-offset GEP");
                }
            }
            for v in inst.used_values() {
                let (root, off) = resolve(&scan.gep, v);
                if root == s_root {
                    // Any address use of S is an observer that must be dead
                    // after the copy; its shape does not matter.
                    u.s_uses.push((bi, ii));
                }
                if root != d_root {
                    continue;
                }
                // A pointer into D at a runtime offset names no field. The
                // GEP that made it is refused above, but blocks are visited
                // in layout order, so its uses can be seen first.
                let Some(off) = off else {
                    return Err("variable-offset GEP");
                };
                match inst {
                    Instruction::GetElementPtr { .. } => {}
                    Instruction::Load {
                        ptr,
                        ty,
                        volatile,
                        seg_override,
                        ..
                    }
                    | Instruction::Store {
                        ptr,
                        ty,
                        volatile,
                        seg_override,
                        ..
                    } => {
                        if ptr.0 != v || *volatile || !matches!(seg_override, AddressSpace::Default)
                        {
                            return Err("unsupported access");
                        }
                        let w = ty_size(*ty);
                        if w <= 0 || w > MAX_PIECE || off < 0 || off + w > size {
                            return Err("unsupported access width");
                        }
                        u.d_accesses.push(Access {
                            block: bi,
                            index: ii,
                            off,
                            ty: *ty,
                        });
                    }
                    Instruction::Memcpy { dest, .. } => {
                        // Only the copy under analysis may name D.
                        if dest.0 != v || off != 0 {
                            return Err("second copy names D");
                        }
                    }
                    Instruction::Call { info, .. } => {
                        let Some(ai) = byval_arg(info, v, size, off) else {
                            return Err("D escapes to a call");
                        };
                        u.byval.push((bi, ii, ai));
                    }
                    other => {
                        if dbg {
                            eprintln!("[byval]   unmodelled use of D: {other:?}");
                        }
                        return Err("unmodelled use of D");
                    }
                }
            }
        }
        // A terminator naming D would make the rename an escape rewrite; a
        // terminator naming S would be an observer this analysis cannot place.
        for v in block.terminator.used_values() {
            let (root, _) = resolve(&scan.gep, v);
            if root == d_root {
                return Err("terminator names D");
            }
            if root == s_root {
                // A pointer to the object reaching control flow is an observer
                // whose position this form cannot bound (and `any_escape` does
                // not see terminators).
                return Err("terminator names S");
            }
        }
    }
    Ok(u)
}

/// Turn `D`'s access set into a complete tiling of `[0, size)`.
fn tile(uses: &[Access], size: i64, max_width: i64) -> Option<Vec<Piece>> {
    // Overlap detection, and the access type per offset: the field's own type
    // then matches what the program uses there.
    let mut seen: FxHashMap<i64, (i64, IrType)> = FxHashMap::default();
    for a in uses {
        let w = a.width();
        match seen.get(&a.off) {
            Some(&(pw, _)) if pw != w => return None,
            Some(_) => {}
            None => {
                seen.insert(a.off, (w, a.ty));
            }
        }
    }
    let mut spans: Vec<(i64, i64, IrType)> =
        seen.iter().map(|(&off, &(w, ty))| (off, w, ty)).collect();
    spans.sort_unstable_by_key(|&(off, _, _)| off);
    // Equal offsets were merged above, so neighbour comparison rules out every
    // overlap between two named fields; the gap fill below fills the rest.
    if spans.windows(2).any(|w| w[0].0 + w[0].1 > w[1].0) {
        return None;
    }
    let mut pieces: Vec<Piece> = Vec::with_capacity(spans.len() + 4);
    let mut pos = 0i64;
    let fill_to = |pieces: &mut Vec<Piece>, target: i64, pos: &mut i64| -> Option<()> {
        while *pos < target {
            let mut w = max_width.min(target - *pos);
            while w > 1 && (*pos % w != 0 || scalar_ty(w).is_none()) {
                w -= 1;
            }
            pieces.push(Piece {
                off: *pos,
                width: w,
                ty: scalar_ty(w)?,
            });
            *pos += w;
        }
        Some(())
    };
    for (off, width, ty) in spans {
        fill_to(&mut pieces, off, &mut pos)?;
        pieces.push(Piece { off, width, ty });
        pos = off + width;
    }
    fill_to(&mut pieces, size, &mut pos)?;
    if pieces.len() > MAX_FIELDS {
        return None;
    }
    Some(pieces)
}

/// Blocks reachable from `start`, expanding each block once.  Re-entering
/// `start` therefore does not extend the set: a path that comes back to the
/// copy re-executes it, which starts a fresh "whose bytes are these" question.
fn reach_forward(succs: &crate::ir::analysis::FlatAdj, start: usize, n: usize) -> Vec<bool> {
    let mut seen = vec![false; n];
    let mut stack = Vec::with_capacity(n);
    seen[start] = true;
    stack.push(start);
    while let Some(b) = stack.pop() {
        for &s in succs.row(b) {
            let s = s as usize;
            if s < n && !seen[s] {
                seen[s] = true;
                stack.push(s);
            }
        }
    }
    seen
}

/// Blocks that reach `target`, without expanding past `cut`.
///
/// A path `w -> ... -> mb(copy) -> ... -> target` runs the copy after `w`, so
/// `w` is not "between the copy and the target" on that path; stopping the
/// backward walk at `mb` is what keeps an ordinary loop body from looking like
/// it interferes with itself on the next iteration.
fn reach_backward(
    preds: &crate::ir::analysis::FlatAdj,
    target: usize,
    cut: usize,
    n: usize,
) -> Vec<bool> {
    let mut seen = vec![false; n];
    let mut stack = Vec::with_capacity(n);
    seen[target] = true;
    stack.push(target);
    while let Some(b) = stack.pop() {
        if b == cut {
            continue;
        }
        for &p in preds.row(b) {
            let p = p as usize;
            if p < n && !seen[p] {
                seen[p] = true;
                stack.push(p);
            }
        }
    }
    seen
}

/// Prove that no access to `S` can execute between the copy at `(mb, mi)` and
/// any use of `D`.
///
/// This is the memory counterpart of a use-def chain: the rename is only sound
/// while `S` has no observer after the copy.  It is answered per program point,
/// not per block, because the copy's own block may sit in a loop:
/// `[body writes] -> copy -> use` is fine even though the body blocks can reach
/// the uses on the *next* iteration, since that iteration runs the copy first.
/// Returns the offending access.
fn find_interference(
    s_uses: &[(usize, usize)],
    uses_of_d: &[(usize, usize)],
    mb: usize,
    mi: usize,
    succs: &crate::ir::analysis::FlatAdj,
    preds: &crate::ir::analysis::FlatAdj,
    n: usize,
) -> Option<(usize, usize)> {
    if s_uses.is_empty() || uses_of_d.is_empty() {
        return None;
    }
    let fwd = reach_forward(succs, mb, n);
    let mut back: FxHashMap<usize, Vec<bool>> = FxHashMap::default();
    for &(wb, wi) in s_uses {
        // Accesses in the copy's own block that precede it are outside the
        // window by construction.
        if wb == mb {
            if wi > mi && wi != usize::MAX {
                return Some((wb, wi));
            }
            continue;
        }
        if !fwd[wb] {
            // The copy can never reach this access, so it cannot run in between.
            continue;
        }
        for &(ub, ui) in uses_of_d {
            if ub == wb {
                // Same block: only accesses in front of the use are in between.
                if wi < ui {
                    return Some((wb, wi));
                }
                continue;
            }
            let reach = back
                .entry(ub)
                .or_insert_with(|| reach_backward(preds, ub, mb, n));
            if reach[wb] {
                return Some((wb, wi));
            }
        }
    }
    None
}

/// The value-substitution step is the whole rename, so the analysis insists on
/// plain, non-volatile, exact-size storage on both sides.  Returns the object's
/// effective alignment (a declared `0` means "natural", which for the sizes this
/// form accepts is eight bytes): the image is rebuilt from narrower fields, so no
/// field may be wider than the storage is aligned for.
fn plain_alloca(func: &IrFunction, root: u32, size: i64) -> Option<i64> {
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca {
                dest,
                size: asz,
                align,
                volatile,
                semantic_volatile,
                ..
            } = inst
            {
                if dest.0 == root {
                    if *volatile || *semantic_volatile || *asz as i64 != size {
                        return None;
                    }
                    let a = if *align == 0 {
                        MAX_PIECE
                    } else {
                        *align as i64
                    };
                    return Some(a.min(MAX_PIECE).max(1));
                }
            }
        }
    }
    None
}

pub(super) fn run_function(
    func: &IrFunction,
    scan: &Scan,
    plan: &mut Plan,
    next: &mut u32,
) -> usize {
    if func.blocks.is_empty() || scan.memcpy.is_empty() {
        return 0;
    }
    let dbg = crate::passes::debug_agg_byval();
    let n = func.blocks.len();
    let label_map = crate::ir::analysis::build_label_map(func);
    let (preds, succs) = crate::ir::analysis::build_cfg(func, &label_map);
    let idom = crate::ir::analysis::compute_dominators(n, &preds, &succs);
    let dom = |a: usize, b: usize| crate::ir::analysis::dominates_idom(&idom, a, b);
    let before = |a: usize, ai: usize, b: usize, bi: usize| dom(a, b) && (a != b || ai < bi);

    let param_homes: FxHashSet<u32> = func.param_alloca_values.iter().map(|v| v.0).collect();
    let mut changed = 0usize;
    // A root claimed by one copy is never touched again: the plan is built
    // against one snapshot of the function, so two copies sharing an object
    // would otherwise rewrite the same instruction twice.
    let mut claimed: FxHashSet<u32> = FxHashSet::default();
    for &(mb, mi, dest, src, size) in &scan.memcpy {
        if plan.remove.contains(&(mb, mi)) || size <= 0 || size > MAX_BYTES {
            continue;
        }
        let (d_root, d_off) = resolve(&scan.gep, dest);
        let (s_root, s_off) = resolve(&scan.gep, src);
        if d_off != Some(0) || s_off != Some(0) || d_root == s_root {
            continue;
        }
        if claimed.contains(&d_root) || claimed.contains(&s_root) {
            continue;
        }
        let Some(d_align) = plain_alloca(func, d_root, size) else {
            continue;
        };
        let Some(s_align) = plain_alloca(func, s_root, size) else {
            continue;
        };
        // Parameter homes are initialized by backend ABI lowering rather than by
        // an IR-visible store, and for a by-value aggregate the home *is* the
        // incoming argument area.  Their storage identity is therefore not fully
        // described by the IR, which is the same reason form 4 never splits one:
        // renaming a local onto a parameter home made a 48-byte struct parameter
        // read from the incoming argument area at an 8-byte-wrong frame offset
        // (tests/regression/agg_copy_fwd_partial_rewrite.c).  Both sides are
        // refused, because the rename moves uses onto `S` and writes into `D`'s
        // old storage.
        if param_homes.contains(&d_root) || param_homes.contains(&s_root) {
            if dbg {
                eprintln!("[byval] copy b{mb}/{mi}: a parameter home is involved");
            }
            continue;
        }
        // S must be private: then the only observers of its bytes are the
        // instructions this pass enumerated.
        if any_escape(&scan.escapes, &scan.gep, s_root) {
            if dbg {
                eprintln!("[byval] copy b{mb}/{mi} S=v{s_root}: source escapes");
            }
            continue;
        }
        let u = match classify(func, scan, d_root, s_root, size) {
            Ok(u) => u,
            Err(why) => {
                if dbg {
                    eprintln!("[byval] copy b{mb}/{mi} D=v{d_root} S=v{s_root} size={size}: {why}");
                }
                continue;
            }
        };
        // Every use of D must be dominated by the copy, so that what it sees is
        // the bytes the copy wrote.
        let uses_of_d: Vec<(usize, usize)> = u
            .d_accesses
            .iter()
            .map(|a| (a.block, a.index))
            .chain(u.byval.iter().map(|&(b, i, _)| (b, i)))
            .collect();
        if !uses_of_d.iter().all(|&(b, i)| before(mb, mi, b, i)) {
            if dbg {
                eprintln!("[byval] copy b{mb}/{mi}: a use of D is not dominated by the copy");
            }
            continue;
        }
        // ...and S must have no observer after the copy.
        if let Some(w) = find_interference(&u.s_uses, &uses_of_d, mb, mi, &succs, &preds, n) {
            if dbg {
                eprintln!(
                    "[byval] copy b{mb}/{mi}: S is still used at b{}/{}",
                    w.0, w.1
                );
            }
            continue;
        }
        // More than one by-value use would need one image per call site; the
        // shape this form targets has exactly one, so anything else is left to
        // the simpler forms.
        if u.byval.len() > 1 {
            continue;
        }
        // No field may be wider than either object is aligned for: the image is
        // rebuilt by reinterpreting the bytes, and a wider scalar than the
        // storage guarantees would be an over-aligned access on some targets.
        let max_width = d_align.min(s_align);
        let Some(pieces) = tile(&u.d_accesses, size, max_width) else {
            if dbg {
                eprintln!("[byval] copy b{mb}/{mi}: field tiling failed");
            }
            continue;
        };

        // The one by-value call gets a private image, rebuilt at its own call
        // site from the (now SSA) fields.
        if let Some(&(cb, ci, ai)) = u.byval.first() {
            let img = *next;
            *next += 1;
            plan.new_allocas.push(Instruction::Alloca {
                dest: Value(img),
                ty: IrType::Ptr,
                size: size as usize,
                // The image is written one field at a time, so it needs exactly
                // as much alignment as the widest field.
                align: max_width as usize,
                volatile: false,
                semantic_volatile: false,
            });
            for p in &pieces {
                let sp = ptr_at(plan, next, (cb, ci), s_root, p.off);
                let dp = ptr_at(plan, next, (cb, ci), img, p.off);
                let t = *next;
                *next += 1;
                plan.insert.push(Insert {
                    block: cb,
                    at: ci,
                    inst: Instruction::Load {
                        dest: Value(t),
                        ptr: Value(sp),
                        ty: p.ty,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    },
                });
                plan.insert.push(Insert {
                    block: cb,
                    at: ci,
                    inst: Instruction::Store {
                        val: Operand::Value(Value(t)),
                        ptr: Value(dp),
                        ty: p.ty,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    },
                });
            }
            plan.call_arg.push((cb, ci, ai, img));
        }

        // The rename itself, and the copy goes away.
        plan.replace.push((d_root, s_root));
        plan.remove.push((mb, mi));
        plan.drop_allocas.insert(d_root);
        claimed.insert(d_root);
        claimed.insert(s_root);
        changed += 1;
        if dbg {
            eprintln!(
                "[byval] fn {}: coalesced copy b{mb}/{mi} D=v{d_root} -> S=v{s_root} size={size} fields={} byval={}",
                func.name,
                pieces.len(),
                u.byval.len()
            );
        }
    }
    changed
}

/// `base + off`, inserting a `GEP` at `sink` when the offset is non-zero.
fn ptr_at(plan: &mut Plan, next: &mut u32, sink: (usize, usize), base: u32, off: i64) -> u32 {
    if off == 0 {
        return base;
    }
    let g = *next;
    *next += 1;
    plan.insert.push(Insert {
        block: sink.0,
        at: sink.1,
        inst: Instruction::GetElementPtr {
            dest: Value(g),
            base: Value(base),
            offset: Operand::Const(IrConst::ptr_int(off)),
            ty: IrType::Ptr,
        },
    });
    g
}
