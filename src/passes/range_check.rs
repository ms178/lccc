//! Range-check folding: `(x >= lo && x <= hi)` → `(unsigned)(x - lo) <= hi - lo`.
//!
//! C's `a && b` / `a || b` lower to short-circuit control flow, which mem2reg
//! and if-conversion collapse into `Select` chains. When both arms of the
//! short-circuit compare the SAME value against constant bounds, the whole
//! boolean dance (two `cmp`+`setcc`+`movzbl`, a `test`, a `cmov`, and a
//! narrowing cast) collapses into one subtract and one unsigned compare —
//! exactly what GCC emits for Expat's `xml_name_continue` / SQLite's varint
//! classifier hot paths.
//!
//! The transform is the classic unsigned-bias identity:
//!   x >= lo && x <= hi  ⇔  (unsigned)(x - lo) <= (hi - lo)      (lo <= hi)
//!   x <  lo || x >  hi  ⇔  (unsigned)(x - lo) >  (hi - lo)      (lo <= hi)
//! Wraparound subtraction makes out-of-range values wrap to huge unsigned
//! values, so a single unsigned compare classifies the whole range. Valid for
//! both signed and unsigned comparisons, in any integer width, provided
//! `hi - lo` is representable.
//!
//! Pass name for CCC_DISABLE_PASSES: "range_fold".

use crate::common::types::IrType;
use crate::ir::reexports::{
    Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Terminator, Value,
};

/// A comparison, canonicalized to `value OP const` with an explicit role.
#[derive(Clone, Copy)]
struct Bound {
    /// The value being compared (always the same value on both arms).
    value: Value,
    /// The constant bound.
    bound: i64,
    /// The operand type of the comparison (e.g. I32 after promotion).
    ty: IrType,
    /// Inclusive/exclusive, lower/upper role.
    kind: BoundKind,
    /// Signedness of the original comparison (S* vs U*).
    signed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BoundKind {
    /// x >= c  (or c <= x)
    InclLower,
    /// x <= c  (or c >= x)
    InclUpper,
    /// x > c  (or c < x)
    ExclLower,
    /// x < c  (or c > x)
    ExclUpper,
}

/// Canonicalize `op lhs rhs` into a `Bound` when one side is a value and the
/// other an integer constant. Returns None for non-constant or non-integer
/// comparisons.
fn canonicalize(op: IrCmpOp, lhs: &Operand, rhs: &Operand, ty: IrType) -> Option<Bound> {
    if !ty.is_integer() {
        return None;
    }
    let signed = matches!(
        op,
        IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge
    );
    // (value, const) with the const on the RHS, swapping the operator when the
    // constant was on the LHS.
    let (value, bound, op) = match (lhs, rhs) {
        (Operand::Value(v), Operand::Const(c)) => (*v, c.to_i64()?, op),
        (Operand::Const(c), Operand::Value(v)) => (*v, c.to_i64()?, swap(op)),
        _ => return None,
    };
    let kind = match op {
        IrCmpOp::Sge | IrCmpOp::Uge => BoundKind::InclLower,
        IrCmpOp::Sle | IrCmpOp::Ule => BoundKind::InclUpper,
        IrCmpOp::Sgt | IrCmpOp::Ugt => BoundKind::ExclLower,
        IrCmpOp::Slt | IrCmpOp::Ult => BoundKind::ExclUpper,
        IrCmpOp::Eq | IrCmpOp::Ne => return None,
    };
    Some(Bound {
        value,
        bound,
        ty,
        kind,
        signed,
    })
}

fn swap(op: IrCmpOp) -> IrCmpOp {
    match op {
        IrCmpOp::Slt => IrCmpOp::Sgt,
        IrCmpOp::Sle => IrCmpOp::Sge,
        IrCmpOp::Sgt => IrCmpOp::Slt,
        IrCmpOp::Sge => IrCmpOp::Sle,
        IrCmpOp::Ult => IrCmpOp::Ugt,
        IrCmpOp::Ule => IrCmpOp::Uge,
        IrCmpOp::Ugt => IrCmpOp::Ult,
        IrCmpOp::Uge => IrCmpOp::Ule,
        other => other,
    }
}

/// Rebuild an integer constant of type `ty` from an i64 value. Returns None if
/// the value does not fit the type.
fn int_const(ty: IrType, v: i64) -> Option<IrConst> {
    match ty {
        IrType::I8 => i8::try_from(v).ok().map(IrConst::I8),
        IrType::U8 => u8::try_from(v).ok().map(|x| IrConst::I8(x as i8)),
        IrType::I16 => i16::try_from(v).ok().map(IrConst::I16),
        IrType::U16 => u16::try_from(v).ok().map(|x| IrConst::I16(x as i16)),
        IrType::I32 => i32::try_from(v).ok().map(IrConst::I32),
        IrType::U32 => u32::try_from(v).ok().map(|x| IrConst::I32(x as i32)),
        IrType::I64 => Some(IrConst::I64(v)),
        IrType::U64 => Some(IrConst::I64(v)),
        _ => None,
    }
}

/// The `lo`/`hi` bounds of a shared-value range, with lo <= hi in the value's
/// domain. `and_form` selects `&&` (inside) vs `||` (outside).
struct Range {
    lo: i64,
    hi: i64,
    value: Value,
    ty: IrType,
}

/// Follow a chain of Cast definitions back to the root value, recording the
/// (from_ty, to_ty) steps. Two operands are the same comparison operand iff
/// their root AND their full cast chain match (an i8→i32 zero-extension is
/// not the same operand as an i16→i32 sign-extension).
fn follow_casts(
    mut v: Value,
    cast_defs: &[Option<(Operand, IrType, IrType)>],
) -> (Value, Vec<(IrType, IrType)>) {
    let mut chain = Vec::with_capacity(16);
    loop {
        let idx = v.0 as usize;
        match cast_defs.get(idx).and_then(|x| x.as_ref()) {
            Some((Operand::Value(src), from_ty, to_ty)) => {
                chain.push((*from_ty, *to_ty));
                v = *src;
            }
            _ => break,
        }
    }
    (v, chain)
}

/// Resolve a boolean-producing value to its defining `Cmp`, looking through
/// the boolean-widening cast the frontend emits after every comparison
/// (`(i32)(u8)cmp`). Phi arms and Select arms carry that cast; without this
/// resolution the fold never sees the comparison at all (which is why the
/// Select form never fired on standard lowering). Returns the value id of
/// the underlying Cmp.
fn resolve_bool_cmp(
    v: u32,
    cmp_defs: &[Option<(IrCmpOp, Operand, Operand, IrType)>],
    cast_defs: &[Option<(Operand, IrType, IrType)>],
) -> Option<u32> {
    if cmp_defs.get(v as usize).is_some_and(|d| d.is_some()) {
        return Some(v);
    }
    let (src, from_ty, to_ty) = cast_defs.get(v as usize)?.as_ref()?;
    let Operand::Value(sv) = src else {
        return None;
    };
    // Only the boolean widening (U8/I8 → wider integer); anything else is a
    // semantic cast the fold must not see through.
    //
    // EVERY wider integer target must be listed. The set used to stop at
    // `I16 | I32 | U32`, which silently excluded the 64-bit widening the
    // frontend actually emits for `char c; return c >= 'a' && c <= 'z';`:
    //
    //     Cmp  v10 = Sle v1, I8(122)
    //     Cast v11 = v10 (U8 -> I64)        <-- not matched, fold gave up
    //     Select v14 = v6 ? v11 : Const(0)  (ty I64)
    //
    // so the pass never fired on the very idiom it was written for, and
    // `set_membership` (which consumes range_fold's output) was starved with
    // it -- lccc emitted an 11-deep compare/branch chain for Expat's
    // `xml_name_continue` classifier where GCC and ICX emit a handful of
    // branchless instructions.
    //
    // Widening is value-preserving here regardless of signedness: the source
    // is a `Cmp` result, verified below to be 0 or 1, and both zero- and
    // sign-extension of 0/1 yield 0/1 in every width. Narrowing is NOT in the
    // list -- that could discard bits of a non-boolean.
    if !matches!(
        (from_ty, to_ty),
        (
            IrType::I8 | IrType::U8,
            IrType::I16
                | IrType::U16
                | IrType::I32
                | IrType::U32
                | IrType::I64
                | IrType::U64
                | IrType::I128
                | IrType::U128
        )
    ) {
        return None;
    }
    let s = sv.0;
    if cmp_defs.get(s as usize).is_some_and(|d| d.is_some()) {
        Some(s)
    } else {
        None
    }
}

/// Match the two arms of a short-circuit select against the same value and
/// extract the constant range. `and_form`:
///   true  → `x >= lo && x <= hi` (inclusive both ends)
///   false → `x <  lo || x >  hi` (exclusive both ends, outside the range)
fn extract_range(
    a: &Bound,
    b: &Bound,
    and_form: bool,
    cast_defs: &[Option<(Operand, IrType, IrType)>],
) -> Option<Range> {
    if a.ty != b.ty || a.signed != b.signed {
        return None;
    }
    // The two comparison operands must be the same value. The lowering casts
    // the operand once per comparison (e.g. two separate `u8 -> i32` casts of
    // the same byte), so compare the cast-chain canonical form, not the raw
    // value id.
    if follow_casts(a.value, cast_defs) != follow_casts(b.value, cast_defs) {
        return None;
    }
    let (lo, hi) = if and_form {
        match (a.kind, b.kind) {
            (BoundKind::InclLower, BoundKind::InclUpper) => (a.bound, b.bound),
            (BoundKind::InclUpper, BoundKind::InclLower) => (b.bound, a.bound),
            _ => return None,
        }
    } else {
        match (a.kind, b.kind) {
            (BoundKind::ExclUpper, BoundKind::ExclLower) => (a.bound, b.bound),
            (BoundKind::ExclLower, BoundKind::ExclUpper) => (b.bound, a.bound),
            _ => return None,
        }
    };
    if lo > hi {
        return None;
    }
    Some(Range {
        lo,
        hi,
        value: a.value,
        ty: a.ty,
    })
}

/// Fold one `Select` into a range check when it matches. Returns the
/// replacement instruction sequence (1-3 instructions) and consumes a fresh
/// value id when needed.
fn try_fold_select(
    inst: &Instruction,
    cmp_defs: &[Option<(IrCmpOp, Operand, Operand, IrType)>],
    cast_defs: &[Option<(Operand, IrType, IrType)>],
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    let Instruction::Select {
        dest,
        cond,
        true_val,
        false_val,
        ty,
    } = inst
    else {
        return None;
    };
    if !ty.is_integer() {
        return None;
    }

    // Resolve the two arms. Both forms have exactly one constant arm and two
    // comparison arms. Arms may carry the boolean-widening cast — resolve
    // through it to the defining comparison.
    let (cond_bound, other_bound, and_form): (Bound, Bound, bool) = {
        let cond_id = match cond {
            Operand::Value(v) => v.0,
            _ => return None,
        };
        let cond_cmp_id = resolve_bool_cmp(cond_id, cmp_defs, cast_defs)?;
        let cond_cmp = cmp_defs.get(cond_cmp_id as usize)?.as_ref()?;
        let cond_bound = canonicalize(cond_cmp.0, &cond_cmp.1, &cond_cmp.2, cond_cmp.3)?;

        // `&&`: Select(cond, other, 0).  `||`: Select(cond, 1, other).
        if matches!(false_val, Operand::Const(c) if c.to_i64() == Some(0)) {
            let other_id = match true_val {
                Operand::Value(v) => v.0,
                _ => return None,
            };
            let other_cmp_id = resolve_bool_cmp(other_id, cmp_defs, cast_defs)?;
            let other_cmp = cmp_defs.get(other_cmp_id as usize)?.as_ref()?;
            let other_bound = canonicalize(other_cmp.0, &other_cmp.1, &other_cmp.2, other_cmp.3)?;
            (cond_bound, other_bound, true)
        } else if matches!(true_val, Operand::Const(c) if c.to_i64() == Some(1)) {
            let other_id = match false_val {
                Operand::Value(v) => v.0,
                _ => return None,
            };
            let other_cmp_id = resolve_bool_cmp(other_id, cmp_defs, cast_defs)?;
            let other_cmp = cmp_defs.get(other_cmp_id as usize)?.as_ref()?;
            let other_bound = canonicalize(other_cmp.0, &other_cmp.1, &other_cmp.2, other_cmp.3)?;
            (cond_bound, other_bound, false)
        } else {
            return None;
        }
    };

    let range = extract_range(&cond_bound, &other_bound, and_form, cast_defs)?;

    // `hi - lo` must be representable in the operand type.
    let span = range.hi - range.lo;
    int_const(range.ty, span)?;
    let lo_const = int_const(range.ty, range.lo)?;

    // Narrow the COMPARE to the operand's source width when the operand is a
    // widening cast of a byte/short: `(u8)(x - lo) <= span` reads the low
    // byte of the 32-bit sub and classifies the range identically (the wrap
    // in the narrow domain is what makes the unsigned-bias check work).
    // Matches GCC's `add edx,62; cmp dl,29` byte-compare shape. The sub stays
    // at the promoted width; only the compare narrows, so the emitter reuses
    // the v11 byte/word compare path.
    let mut cmp_ty = range.ty;
    if let Some(Some((_, from_ty, to_ty))) = cast_defs.get(range.value.0 as usize) {
        if *to_ty == range.ty
            && from_ty.is_integer()
            && from_ty.size() < range.ty.size()
            && int_const(*from_ty, span).is_some()
        {
            cmp_ty = *from_ty;
        }
    }
    let span_narrow_const = int_const(cmp_ty, span)?;

    let sub_dest = Value(*next_id);
    *next_id += 1;
    let mut out = Vec::with_capacity(3);
    out.push(Instruction::BinOp {
        dest: sub_dest,
        op: IrBinOp::Sub,
        lhs: Operand::Value(range.value),
        rhs: Operand::Const(lo_const),
        ty: range.ty,
    });
    let cmp_op = if and_form { IrCmpOp::Ule } else { IrCmpOp::Ugt };
    // The compare result is I8 (boolean). When the Select's result type is I8
    // we can use `dest` directly; otherwise widen the boolean to the Select's
    // type so every downstream use keeps its type.
    if *ty == IrType::I8 {
        out.push(Instruction::Cmp {
            dest: *dest,
            op: cmp_op,
            lhs: Operand::Value(sub_dest),
            rhs: Operand::Const(span_narrow_const),
            ty: cmp_ty,
        });
    } else {
        let cmp_dest = Value(*next_id);
        *next_id += 1;
        out.push(Instruction::Cmp {
            dest: cmp_dest,
            op: cmp_op,
            lhs: Operand::Value(sub_dest),
            rhs: Operand::Const(span_narrow_const),
            ty: cmp_ty,
        });
        out.push(Instruction::Cast {
            dest: *dest,
            src: Operand::Value(cmp_dest),
            from_ty: IrType::I8,
            to_ty: *ty,
        });
    }
    Some(out)
}

/// Fold the PHI form of a short-circuit `&&`/`||` range predicate.
///
/// if_convert is disabled on the m16 size profile (measured: it grows the
/// boot corpus), so a value-context `x >= lo && x <= hi` — an inlined
/// isdigit/isxdigit-style predicate — stays a branch diamond whose merge is
/// a Phi:
///
///   Bcond:  %c1 = Cmp(<bound1>, x, K1)
///           CondBranch(%c1, Bcheck, Bmerge)        // either orientation
///   Bcheck: %c2 = Cmp(<bound2>, x, K2)
///           [%w  = Cast %c2 -> T]                  // boolean widening
///           Branch(Bmerge)
///   Bmerge: %p = Phi([Const 0|1, Bcond], [%w, Bcheck])
///           ... uses of %p ...
///
/// The backend materializes that Phi as two setcc + slot-store arms plus a
/// reload (~25 bytes where GCC needs one sub+cmp). When both bounds compare
/// the same value, replace the whole diamond with the unsigned-bias form
/// computed in Bcond:
///
///   %d = Sub(x, K1); %u = Cmp(Ule, %d, K2-K1); [Cast %u -> T]
///   Branch(Bmerge)                               // unconditional
///
/// with every use of %p replaced by %u. Bcheck becomes unreachable and is
/// dropped. All structural requirements are checked fail-closed: the Phi has
/// exactly the two diamond arms, Bcheck has Bcond as its only predecessor
/// and branches straight to Bmerge, Bmerge has no other predecessors, the
/// second comparison (and its widening cast) has no uses besides the Phi
/// arm, and the cast chain of both comparison operands matches. Nested
/// diamonds (`(a&&b) || (c&&d)`) simply fail the match and keep their
/// existing shape — the inner folds still apply.
fn fold_phi_diamonds(
    func: &mut IrFunction,
    cmp_defs: &[Option<(IrCmpOp, Operand, Operand, IrType)>],
    cast_defs: &[Option<(Operand, IrType, IrType)>],
    next_id: &mut u32,
) -> usize {
    use crate::common::fx_hash::{FxHashMap, FxHashSet};

    if func.blocks.is_empty() {
        return 0;
    }
    let idx_of: FxHashMap<u32, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label.0, i))
        .collect();
    let label_of: Vec<u32> = func.blocks.iter().map(|b| b.label.0).collect();

    // Successor labels of a terminator (structural, no operand reading).
    fn succs(t: &Terminator) -> Vec<u32> {
        match t {
            Terminator::Branch(l) => vec![l.0],
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => vec![true_label.0, false_label.0],
            Terminator::Switch { cases, default, .. } => {
                let mut v: Vec<u32> = cases.iter().map(|(_, l)| l.0).collect();
                v.push(default.0);
                v
            }
            _ => Vec::new(),
        }
    }

    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); func.blocks.len()];
    for (i, b) in func.blocks.iter().enumerate() {
        for s in succs(&b.terminator) {
            if let Some(&j) = idx_of.get(&s) {
                preds[j].push(i);
            }
        }
    }

    // Candidate merges: a block whose leading phis include a 2-incoming Phi
    // with one constant arm and one value arm.
    struct Cand {
        merge_idx: usize,
        phi_pos: usize,
    }
    let mut cands: Vec<Cand> = Vec::new();
    for (mi, block) in func.blocks.iter().enumerate() {
        for (pi, inst) in block.instructions.iter().enumerate() {
            if let Instruction::Phi { incoming, .. } = inst {
                if incoming.len() == 2 {
                    cands.push(Cand {
                        merge_idx: mi,
                        phi_pos: pi,
                    });
                }
            } else {
                break; // phis lead the block
            }
        }
    }

    let mut changes = 0usize;
    let mut removed: FxHashSet<u32> = FxHashSet::default();
    for cand in cands {
        if removed.contains(&label_of[cand.merge_idx]) {
            continue;
        }
        let (phi_dest, phi_ty, const_arm, val_arm) = {
            let merge = &func.blocks[cand.merge_idx];
            let Instruction::Phi { dest, incoming, ty } = &merge.instructions[cand.phi_pos] else {
                continue;
            };
            let a = &incoming[0];
            let b = &incoming[1];
            let (const_arm, val_arm) = match (&a.0, &b.0) {
                (Operand::Const(_), Operand::Value(_)) => (a, b),
                (Operand::Value(_), Operand::Const(_)) => (b, a),
                _ => continue,
            };
            (
                *dest,
                *ty,
                (const_arm.0, const_arm.1),
                (val_arm.0, val_arm.1),
            )
        };
        // Constant arm must be 0 (&&) or 1 (||).
        let and_form = match const_arm.0 {
            Operand::Const(c) if c.to_i64() == Some(0) => true,
            Operand::Const(c) if c.to_i64() == Some(1) => false,
            _ => continue,
        };

        let bcond_idx = match idx_of.get(&const_arm.1 .0) {
            Some(&i) => i,
            None => continue,
        };
        let bcheck_idx = match idx_of.get(&val_arm.1 .0) {
            Some(&i) => i,
            None => continue,
        };
        if bcond_idx == cand.merge_idx || bcheck_idx == cand.merge_idx || bcond_idx == bcheck_idx {
            continue;
        }

        // Bcheck: single predecessor (Bcond), branches straight to Bmerge.
        if preds[cand.merge_idx].len() != 2
            || !preds[cand.merge_idx].contains(&bcond_idx)
            || !preds[cand.merge_idx].contains(&bcheck_idx)
        {
            continue;
        }
        if preds[bcheck_idx].len() != 1 || preds[bcheck_idx][0] != bcond_idx {
            continue;
        }
        let bcheck_label = label_of[bcheck_idx];
        let merge_label = label_of[cand.merge_idx];
        if !matches!(func.blocks[bcheck_idx].terminator, Terminator::Branch(l) if l.0 == merge_label)
        {
            continue;
        }

        // Bcond must end in a CondBranch whose two targets are exactly
        // {Bcheck, Bmerge}; remember which side Bmerge is on so the first
        // comparison can be normalized to its "contributes to the phi"
        // sense.
        let (cond_val, merge_on_true) = {
            let bcond = &func.blocks[bcond_idx];
            let Terminator::CondBranch {
                cond: Operand::Value(cv),
                true_label,
                false_label,
            } = &bcond.terminator
            else {
                continue;
            };
            let t_ok = true_label.0 == bcheck_label || true_label.0 == merge_label;
            let f_ok = false_label.0 == bcheck_label || false_label.0 == merge_label;
            if !(t_ok && f_ok) || true_label == false_label {
                continue;
            }
            (*cv, true_label.0 == merge_label)
        };

        // Resolve both comparisons (through boolean widenings).
        let Some(c1_id) = resolve_bool_cmp(cond_val.0, cmp_defs, cast_defs) else {
            continue;
        };
        let Some(c1) = cmp_defs.get(c1_id as usize).and_then(|d| d.as_ref()) else {
            continue;
        };
        // Normalize the first comparison to the sense that contributes to
        // the phi: the Bmerge edge carries the constant arm. For `&&` the
        // constant is 0 and the merge edge means "first test failed", so
        // the contributing sense is the branch edge to Bcheck; when Bmerge
        // is the TRUE target the comparison is inverted. For `||` (constant
        // 1 on the merge edge) it is the opposite.
        let negate = if and_form {
            merge_on_true
        } else {
            !merge_on_true
        };
        let c1_op = if negate {
            match c1.0 {
                IrCmpOp::Slt => IrCmpOp::Sge,
                IrCmpOp::Sle => IrCmpOp::Sgt,
                IrCmpOp::Sgt => IrCmpOp::Sle,
                IrCmpOp::Sge => IrCmpOp::Slt,
                IrCmpOp::Ult => IrCmpOp::Uge,
                IrCmpOp::Ule => IrCmpOp::Ugt,
                IrCmpOp::Ugt => IrCmpOp::Ule,
                IrCmpOp::Uge => IrCmpOp::Ult,
                IrCmpOp::Eq => IrCmpOp::Ne,
                IrCmpOp::Ne => IrCmpOp::Eq,
            }
        } else {
            c1.0
        };
        let val_id = match val_arm.0 {
            Operand::Value(v) => v.0,
            _ => continue,
        };
        let Some(c2_id) = resolve_bool_cmp(val_id, cmp_defs, cast_defs) else {
            continue;
        };
        let Some(c2) = cmp_defs.get(c2_id as usize).and_then(|d| d.as_ref()) else {
            continue;
        };
        let Some(bound1) = canonicalize(c1_op, &c1.1, &c1.2, c1.3) else {
            continue;
        };
        let Some(bound2) = canonicalize(c2.0, &c2.1, &c2.2, c2.3) else {
            continue;
        };

        let Some(range) = extract_range(&bound1, &bound2, and_form, cast_defs) else {
            continue;
        };
        let span = range.hi - range.lo;
        if int_const(range.ty, span).is_none() {
            continue;
        }
        let Some(lo_const) = int_const(range.ty, range.lo) else {
            continue;
        };

        // Narrow the compare to the operand's source width when the operand
        // is a widening cast of a byte/short (mirrors the Select path: GCC's
        // `add edx,62; cmp dl,29` byte-compare shape).
        let mut cmp_ty = range.ty;
        if let Some(Some((_, from_ty, to_ty))) = cast_defs.get(range.value.0 as usize) {
            if *to_ty == range.ty
                && from_ty.is_integer()
                && from_ty.size() < range.ty.size()
                && int_const(*from_ty, span).is_some()
            {
                cmp_ty = *from_ty;
            }
        }
        let Some(span_narrow_const) = int_const(cmp_ty, span) else {
            continue;
        };

        // The second comparison (and its widening cast, when the phi arm
        // carries one) must have no uses OUTSIDE Bcheck and the phi arm:
        // Bcheck dies with the fold, taking its Cmp/Cast definitions along,
        // so any other live consumer would reference a dead value. Uses
        // inside Bcheck itself are exactly the instructions being deleted.
        let mut arm_ok = true;
        'outer: for block in &func.blocks {
            if block.label.0 == bcheck_label {
                continue;
            }
            for inst in &block.instructions {
                if let Instruction::Phi { dest, incoming, .. } = inst {
                    if dest.0 == phi_dest.0 {
                        continue;
                    }
                    if incoming.iter().any(
                        |(op, _)| matches!(op, Operand::Value(v) if v.0 == val_id || v.0 == c2_id),
                    ) {
                        arm_ok = false;
                        break 'outer;
                    }
                }
                let mut bad = false;
                inst.for_each_used_value(|id| {
                    if id == val_id || id == c2_id {
                        bad = true;
                    }
                });
                if bad {
                    arm_ok = false;
                    break 'outer;
                }
            }
        }
        if !arm_ok {
            continue;
        }

        // Build the replacement: sub + unsigned compare (+ widening cast to
        // the phi's type when it is not the boolean type itself).
        let sub_dest = Value(*next_id);
        *next_id += 1;
        let cmp_dest = Value(*next_id);
        *next_id += 1;
        let mut new_insts = Vec::with_capacity(3);
        new_insts.push(Instruction::BinOp {
            dest: sub_dest,
            op: IrBinOp::Sub,
            lhs: Operand::Value(range.value),
            rhs: Operand::Const(lo_const),
            ty: range.ty,
        });
        let cmp_op = if and_form { IrCmpOp::Ule } else { IrCmpOp::Ugt };
        new_insts.push(Instruction::Cmp {
            dest: cmp_dest,
            op: cmp_op,
            lhs: Operand::Value(sub_dest),
            rhs: Operand::Const(span_narrow_const),
            ty: cmp_ty,
        });
        // Replacement value for the phi's uses.
        let replacement: Operand = if phi_ty == IrType::I8 || phi_ty == IrType::U8 {
            Operand::Value(cmp_dest)
        } else {
            let cast_dest = Value(*next_id);
            *next_id += 1;
            new_insts.push(Instruction::Cast {
                dest: cast_dest,
                src: Operand::Value(cmp_dest),
                from_ty: IrType::I8,
                to_ty: phi_ty,
            });
            Operand::Value(cast_dest)
        };

        // Apply: splice into Bcond, unconditional branch to Bmerge, replace
        // phi uses, delete the phi and Bcheck.
        {
            let bcond = &mut func.blocks[bcond_idx];
            bcond.instructions.extend(new_insts);
            bcond.terminator = Terminator::Branch(crate::ir::reexports::BlockId(merge_label));
        }
        // Replace every use of phi_dest with the replacement.
        for block in &mut func.blocks {
            for inst in &mut block.instructions {
                inst.for_each_operand_mut(|op: &mut Operand| {
                    if let Operand::Value(v) = op {
                        if v.0 == phi_dest.0 {
                            *op = replacement;
                        }
                    }
                });
                if let Some(d) = inst.dest() {
                    if d.0 == phi_dest.0 {
                        // The phi itself is handled below; other redefs of
                        // the same id cannot exist in SSA.
                    }
                }
            }
            block.terminator.for_each_operand_mut(|op: &mut Operand| {
                if let Operand::Value(v) = op {
                    if v.0 == phi_dest.0 {
                        *op = replacement;
                    }
                }
            });
        }
        // Drop the phi from Bmerge.
        let merge = &mut func.blocks[cand.merge_idx];
        merge.instructions.remove(cand.phi_pos);
        // Drop Bcheck (unreachable: its only predecessor now branches to
        // Bmerge). Deferred removal keeps earlier indices valid for this
        // loop; later candidates re-check `removed`.
        removed.insert(bcheck_label);
        changes += 1;
    }

    if !removed.is_empty() {
        func.blocks.retain(|b| !removed.contains(&b.label.0));
    }
    changes
}

/// Run the range-check fold over one function. Returns the number of folds.
pub(crate) fn run_function(func: &mut IrFunction) -> usize {
    let mut next_id = func.next_value_id;
    if next_id == 0 {
        next_id = func.max_value_id() + 1;
    }

    // Def maps: value → its defining comparison / cast / integer binop.
    let max_id = func.max_value_id() as usize;
    let mut cmp_defs: Vec<Option<(IrCmpOp, Operand, Operand, IrType)>> = vec![None; max_id + 1];
    let mut cast_defs: Vec<Option<(Operand, IrType, IrType)>> = vec![None; max_id + 1];
    let mut binop_defs: Vec<Option<(IrBinOp, Operand, Operand, IrType)>> = vec![None; max_id + 1];
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Cmp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => {
                    let idx = dest.0 as usize;
                    if idx < cmp_defs.len() {
                        cmp_defs[idx] = Some((*op, *lhs, *rhs, *ty));
                    }
                }
                Instruction::Cast {
                    dest,
                    src,
                    from_ty,
                    to_ty,
                } => {
                    let idx = dest.0 as usize;
                    if idx < cast_defs.len() {
                        cast_defs[idx] = Some((*src, *from_ty, *to_ty));
                    }
                }
                Instruction::BinOp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => {
                    let idx = dest.0 as usize;
                    if idx < binop_defs.len() {
                        binop_defs[idx] = Some((*op, *lhs, *rhs, *ty));
                    }
                }
                _ => {}
            }
        }
    }

    // One-block path-sensitive truth for the canonical short-circuit shape:
    //
    //   if (x <= 0) goto join; else goto check;
    // check:
    //   if ((unsigned)(x - 1) < UINT_MAX) goto join; else goto fail;
    //
    // On the false edge of `x <= 0`, signed 32-bit `x` is in [1, INT_MAX], so
    // `(u32)(x - 1)` is in [0, INT_MAX-1] and is necessarily below UINT_MAX.
    // GCC torture 20041114-1 uses this exact overflow-sensitive idiom to make
    // the `link_failure` edge unreachable.  This is deliberately local and
    // typed: no global range lattice is invented here, but the proof is strong
    // enough to remove the dead edge without weakening C signed-overflow rules.
    let mut block_known_pos_i32: Vec<Option<Value>> = vec![None; func.blocks.len()];
    for (pred_idx, pred) in func.blocks.iter().enumerate() {
        let Terminator::CondBranch {
            cond: Operand::Value(cond_v),
            true_label,
            false_label,
        } = pred.terminator
        else {
            continue;
        };
        let Some((op, lhs, rhs, ty)) = cmp_defs.get(cond_v.0 as usize).and_then(|x| *x) else {
            continue;
        };
        if ty != IrType::I32 {
            continue;
        }
        let known = match (op, lhs, rhs) {
            (IrCmpOp::Sle, Operand::Value(x), Operand::Const(c)) if c.to_i64() == Some(0) => {
                Some((false_label, x))
            }
            (IrCmpOp::Sgt, Operand::Value(x), Operand::Const(c)) if c.to_i64() == Some(0) => {
                Some((true_label, x))
            }
            (IrCmpOp::Sge, Operand::Const(c), Operand::Value(x)) if c.to_i64() == Some(0) => {
                Some((false_label, x))
            }
            (IrCmpOp::Slt, Operand::Const(c), Operand::Value(x)) if c.to_i64() == Some(0) => {
                Some((true_label, x))
            }
            _ => None,
        };
        if let Some((label, x)) = known {
            if let Some((idx, _)) = func
                .blocks
                .iter()
                .enumerate()
                .find(|(_, b)| b.label == label)
            {
                // Keep it single-predecessor: joining different path facts
                // requires a real range lattice, not this local edge fact.
                if func
                    .blocks
                    .iter()
                    .filter(|b| match &b.terminator {
                        Terminator::Branch(l) => *l == label,
                        Terminator::CondBranch {
                            true_label,
                            false_label,
                            ..
                        } => *true_label == label || *false_label == label,
                        _ => false,
                    })
                    .count()
                    == 1
                {
                    let _ = pred_idx;
                    block_known_pos_i32[idx] = Some(x);
                }
            }
        }
    }

    // Phi-diamond form (if-convert off, e.g. the m16 size profile): fold
    // first, while the def maps still describe the pre-fold structure.
    let mut changes = fold_phi_diamonds(func, &cmp_defs, &cast_defs, &mut next_id);
    for (block_idx, block) in func.blocks.iter_mut().enumerate() {
        let mut new_insts: Vec<Instruction> = Vec::with_capacity(block.instructions.len());
        let known_pos = block_known_pos_i32[block_idx];
        for inst in block.instructions.drain(..) {
            let path_fold = if let (
                Some(x),
                Instruction::Cmp {
                    dest,
                    op: IrCmpOp::Ult,
                    lhs: Operand::Value(cast_v),
                    rhs: Operand::Const(limit),
                    ty: IrType::U32,
                },
            ) = (known_pos, &inst)
            {
                if limit.to_i64() == Some(u32::MAX as i64) || limit.to_i64() == Some(-1) {
                    let cast_src = cast_defs
                        .get(cast_v.0 as usize)
                        .and_then(|d| d.as_ref())
                        .and_then(|(src, _from_ty, to_ty)| {
                            if *to_ty == IrType::U32 {
                                if let Operand::Value(v) = src {
                                    Some(*v)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        });
                    let is_x_minus_one = cast_src
                        .and_then(|sub_v| binop_defs.get(sub_v.0 as usize).and_then(|d| d.as_ref()))
                        .is_some_and(|(op, lhs, rhs, ty)| {
                            *op == IrBinOp::Sub
                                && *ty == IrType::I32
                                && matches!(lhs, Operand::Value(v) if *v == x)
                                && matches!(rhs, Operand::Const(c) if c.to_i64() == Some(1))
                        });
                    if is_x_minus_one {
                        Some(Instruction::Copy {
                            dest: *dest,
                            src: Operand::Const(IrConst::I8(1)),
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(replacement) = path_fold {
                changes += 1;
                new_insts.push(replacement);
            } else if let Some(replacements) =
                try_fold_select(&inst, &cmp_defs, &cast_defs, &mut next_id)
            {
                changes += 1;
                new_insts.extend(replacements);
            } else {
                new_insts.push(inst);
            }
        }
        block.instructions = new_insts;
    }
    func.next_value_id = next_id;
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `resolve_bool_cmp` must see through the boolean widening cast at EVERY
    /// width. The allow-list used to stop at `I16 | I32 | U32`, silently
    /// excluding the `U8 -> I64` widening the frontend actually emits for
    /// `char c; c >= 'a' && c <= 'z'` -- so the pass never fired on its own
    /// headline idiom, and `set_membership` downstream was starved with it.
    #[test]
    fn a_boolean_widening_cast_is_transparent_at_every_width() {
        let mut cmp_defs: Vec<Option<(IrCmpOp, Operand, Operand, IrType)>> = vec![None; 4];
        cmp_defs[1] = Some((
            IrCmpOp::Sle,
            Operand::Value(Value(0)),
            Operand::Const(IrConst::I8(122)),
            IrType::I8,
        ));

        for to_ty in [
            IrType::I16,
            IrType::U16,
            IrType::I32,
            IrType::U32,
            IrType::I64,
            IrType::U64,
            IrType::I128,
            IrType::U128,
        ] {
            let mut cast_defs: Vec<Option<(Operand, IrType, IrType)>> = vec![None; 4];
            cast_defs[2] = Some((Operand::Value(Value(1)), IrType::U8, to_ty));
            assert_eq!(
                resolve_bool_cmp(2, &cmp_defs, &cast_defs),
                Some(1),
                "widening a Cmp result to {:?} must stay transparent",
                to_ty
            );
        }
    }

    /// SOUNDNESS: only WIDENING is transparent. A narrowing cast can discard
    /// bits of a value that is not actually a boolean, so the fold must not
    /// see through it.
    #[test]
    fn a_narrowing_cast_is_not_transparent() {
        let mut cmp_defs: Vec<Option<(IrCmpOp, Operand, Operand, IrType)>> = vec![None; 4];
        cmp_defs[1] = Some((
            IrCmpOp::Sle,
            Operand::Value(Value(0)),
            Operand::Const(IrConst::I32(122)),
            IrType::I32,
        ));
        let mut cast_defs: Vec<Option<(Operand, IrType, IrType)>> = vec![None; 4];
        cast_defs[2] = Some((Operand::Value(Value(1)), IrType::I32, IrType::I8));
        assert_eq!(resolve_bool_cmp(2, &cmp_defs, &cast_defs), None);
    }

    #[test]
    fn canonicalize_lower_upper() {
        let lo = Operand::Const(IrConst::I32(97));
        let hi = Operand::Const(IrConst::I32(122));
        let x = Operand::Value(Value(1));

        let b1 = canonicalize(IrCmpOp::Sge, &x, &lo, IrType::I32).unwrap();
        assert_eq!(b1.kind, BoundKind::InclLower);
        assert_eq!(b1.bound, 97);

        let b2 = canonicalize(IrCmpOp::Sle, &x, &hi, IrType::I32).unwrap();
        assert_eq!(b2.kind, BoundKind::InclUpper);
        assert_eq!(b2.bound, 122);

        // Constant on the LHS: `97 <= x` is the same lower-inclusive bound.
        let b3 = canonicalize(IrCmpOp::Sle, &lo, &x, IrType::I32).unwrap();
        assert_eq!(b3.kind, BoundKind::InclLower);
        assert_eq!(b3.bound, 97);
    }

    #[test]
    fn extract_and_range() {
        let x = Value(1);
        let no_casts: Vec<Option<(Operand, IrType, IrType)>> = vec![None; 8];
        let lo = Bound {
            value: x,
            bound: 97,
            ty: IrType::I32,
            kind: BoundKind::InclLower,
            signed: true,
        };
        let hi = Bound {
            value: x,
            bound: 122,
            ty: IrType::I32,
            kind: BoundKind::InclUpper,
            signed: true,
        };
        let r = extract_range(&lo, &hi, true, &no_casts).unwrap();
        assert_eq!((r.lo, r.hi), (97, 122));

        // Swapped order still matches.
        let r2 = extract_range(&hi, &lo, true, &no_casts).unwrap();
        assert_eq!((r2.lo, r2.hi), (97, 122));

        // Reversed range (lo > hi) is not a valid inclusive range:
        // x >= 122 && x <= 97 is empty, so it must not fold.
        let bad = Bound {
            value: x,
            bound: 122,
            ty: IrType::I32,
            kind: BoundKind::InclLower,
            signed: true,
        };
        let low_hi = Bound {
            value: x,
            bound: 97,
            ty: IrType::I32,
            kind: BoundKind::InclUpper,
            signed: true,
        };
        assert!(extract_range(&bad, &low_hi, true, &no_casts).is_none());

        // A collapsed range (x >= 122 && x <= 122) IS foldable (== 122).
        let both = Bound {
            value: x,
            bound: 122,
            ty: IrType::I32,
            kind: BoundKind::InclUpper,
            signed: true,
        };
        let r3 = extract_range(&bad, &both, true, &no_casts).unwrap();
        assert_eq!((r3.lo, r3.hi), (122, 122));
    }

    #[test]
    fn follow_casts_matches_identical_chains() {
        // Two separate u8->i32 casts of the same root are the same operand.
        let x = Value(1);
        let mut cast_defs: Vec<Option<(Operand, IrType, IrType)>> = vec![None; 16];
        cast_defs[4] = Some((Operand::Value(x), IrType::U8, IrType::I32));
        cast_defs[8] = Some((Operand::Value(x), IrType::U8, IrType::I32));
        assert_eq!(
            follow_casts(Value(4), &cast_defs),
            follow_casts(Value(8), &cast_defs)
        );

        // A different cast chain (i16->i32) is a different operand.
        cast_defs[8] = Some((Operand::Value(x), IrType::I16, IrType::I32));
        assert_ne!(
            follow_casts(Value(4), &cast_defs),
            follow_casts(Value(8), &cast_defs)
        );
    }
}
