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
    // comparison arms.
    let (cond_bound, other_bound, and_form): (Bound, Bound, bool) = {
        let cond_id = match cond {
            Operand::Value(v) => v.0,
            _ => return None,
        };
        let cond_cmp = cmp_defs.get(cond_id as usize)?.as_ref()?;
        let cond_bound = canonicalize(cond_cmp.0, &cond_cmp.1, &cond_cmp.2, cond_cmp.3)?;

        // `&&`: Select(cond, other, 0).  `||`: Select(cond, 1, other).
        if matches!(false_val, Operand::Const(c) if c.to_i64() == Some(0)) {
            let other_id = match true_val {
                Operand::Value(v) => v.0,
                _ => return None,
            };
            let other_cmp = cmp_defs.get(other_id as usize)?.as_ref()?;
            let other_bound = canonicalize(other_cmp.0, &other_cmp.1, &other_cmp.2, other_cmp.3)?;
            (cond_bound, other_bound, true)
        } else if matches!(true_val, Operand::Const(c) if c.to_i64() == Some(1)) {
            let other_id = match false_val {
                Operand::Value(v) => v.0,
                _ => return None,
            };
            let other_cmp = cmp_defs.get(other_id as usize)?.as_ref()?;
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

    let mut changes = 0usize;
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
