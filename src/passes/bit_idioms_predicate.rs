//! Predicate materialization: `c ? 1 : 0` and `c ? 0 : 1` are the comparison
//! that the select is hiding.
//!
//! C source spells a boolean result in two different ways.  `return a != b;`
//! lowers to the comparison itself, whose value is already the `0`/`1` carrier
//! and which every backend emits as one `setcc` off the flags.  The equivalent
//! `return a != b ? 1 : 0;` (LLVM and GCC canonicalize both to the same IR)
//! instead lowers to a `Select` over the two constants, and a `Select` needs
//! its own arms in registers: x86 materializes `mov $1,%ecx; mov $0,%eax;
//! cmovne` where the comparison alone would be `xor %eax,%eax; setne %al`.
//!
//! The rewrite is a pure substitution of ONE instruction, and it never needs a
//! new value:
//!
//! * `select(c, 1, 0)` is `(c != 0)` and `select(c, 0, 1)` is `(c == 0)`, so a
//!   condition that is already a comparison of the right type is copied
//!   (`1, 0`) or replaced by that comparison with the inverted predicate
//!   (`0, 1`, single-use condition only, so no second compare is emitted).
//! * Any other integer condition of the select's width gets the zero-test it
//!   was hiding: `Cmp { Ne | Eq, c, 0 }`.  The select's arm constants are
//!   already `0`/`1` *in the select's ring* (the frontend may emit them at a
//!   wider `IrConst` type than `ty`; see `const_in_ring`), so the comparison's
//!   `0`/`1` carrier is the same value the select produced.
//!
//! Why the arms must be `1, 0` and not merely "zero or one": `select(c, 1, 1)`
//! and `select(c, 0, 0)` are constant folds, not comparisons, and a `Select`
//! whose arms are equal is handled where it belongs (`simplify`).  Both arms
//! are checked against the select's ring width, so a `1` that only looks like
//! one after truncation is not accepted.
//!
//! The rewrite is deliberately not extended to non-boolean arm pairs
//! (`select(c, 5, 3)`), to float or vector selects, or to conditions of a
//! different width than the result: the first has no cheaper form, `setcc`
//! does not exist for the last two, and a narrower condition would have to be
//! widened first (that widening is `simplify`'s job and would need a value of
//! its own).

use super::*;

/// The comparison behind an operand, through `Copy` chains only.
///
/// An integer `Cast` is NOT transparent: a cast away from `0`/`1` keeps the
/// zero-ness but changes the carrier width, and the replacement below reuses
/// the comparison's own operands.
fn comparison(
    op: Operand,
    defs: &[Option<Instruction>],
) -> Option<(Value, IrCmpOp, Operand, Operand, IrType)> {
    let Operand::Value(v) = peel_copies(op, defs) else {
        return None;
    };
    match defs.get(v.0 as usize).and_then(Option::as_ref) {
        Some(Instruction::Cmp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        }) => Some((*dest, *op, *lhs, *rhs, *ty)),
        _ => None,
    }
}

/// Is this operand a `0`/`1` value, seen through `Copy`s and integer casts?
///
/// A comparison yields `0`/`1`, and materializing an integer of any width from
/// a `0`/`1` value keeps it `0`/`1` (widening cannot introduce a bit, and
/// truncating a value that has at most one low bit cannot drop one).  So the
/// chain ends at a `Cmp` from ANY integer width, unlike [`comparison`], which
/// answers only when the comparison itself is usable in place of the select.
fn boolean_operand(mut op: Operand, defs: &[Option<Instruction>]) -> bool {
    for _ in 0..32 {
        let Operand::Value(v) = op else { return false };
        match defs.get(v.0 as usize).and_then(Option::as_ref) {
            Some(Instruction::Cmp { .. }) => return true,
            Some(Instruction::Copy { src, .. }) => op = *src,
            Some(Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            }) if from_ty.is_integer() && to_ty.is_integer() => op = *src,
            _ => return false,
        }
    }
    false
}

/// The integer type a value is carried at.
///
/// Only producers whose type is part of the instruction shape are covered;
/// anything else (addresses, aggregate results, calls) fails closed.  The fold
/// uses this to prove the condition is exactly as wide as the select's result,
/// which is what makes a single-width `test` the whole computation.
fn integer_carrier(v: Value, defs: &[Option<Instruction>]) -> Option<IrType> {
    let ty = match defs.get(v.0 as usize).and_then(Option::as_ref)? {
        Instruction::ParamRef { ty, .. }
        | Instruction::Load { ty, .. }
        | Instruction::Cmp { ty, .. }
        | Instruction::BinOp { ty, .. }
        | Instruction::UnaryOp { ty, .. }
        | Instruction::Select { ty, .. }
        | Instruction::Cast { to_ty: ty, .. } => *ty,
        _ => return None,
    };
    ty.is_integer().then_some(ty)
}

/// Invert a comparison predicate.
fn invert(op: IrCmpOp) -> IrCmpOp {
    match op {
        IrCmpOp::Eq => IrCmpOp::Ne,
        IrCmpOp::Ne => IrCmpOp::Eq,
        IrCmpOp::Slt => IrCmpOp::Sge,
        IrCmpOp::Sle => IrCmpOp::Sgt,
        IrCmpOp::Sgt => IrCmpOp::Sle,
        IrCmpOp::Sge => IrCmpOp::Slt,
        IrCmpOp::Ult => IrCmpOp::Uge,
        IrCmpOp::Ule => IrCmpOp::Ugt,
        IrCmpOp::Ugt => IrCmpOp::Ule,
        IrCmpOp::Uge => IrCmpOp::Ult,
    }
}

/// `select(c, 1, 0)` -> `c != 0`, `select(c, 0, 1)` -> `c == 0`.
///
/// `use_counts` guards the one rewrite that would otherwise clone a
/// comparison: replacing `select(c, 0, 1)` by the INVERTED comparison is only
/// a saving while the original comparison has no other reader.
pub(crate) fn fold_predicate_select(
    inst: &Instruction,
    defs: &[Option<Instruction>],
    use_counts: &[u32],
) -> Option<Instruction> {
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
    let bits = ty.size() * 8;
    // Which arm is the `1`, if this is a boolean materialization at all.
    let positive = match (
        const_in_ring(*true_val, bits),
        const_in_ring(*false_val, bits),
    ) {
        (Some(1), Some(0)) => true,
        (Some(0), Some(1)) => false,
        _ => return None,
    };
    // The condition may already BE the `0`/`1` value the select re-creates.
    // `comparison` follows copies only, so a hit means the comparison's own
    // operands can stand in for it -- which is what makes the rewrites below
    // free of extra instructions.
    let boolean = boolean_operand(*cond, defs);
    let carrier = match peel_copies(*cond, defs) {
        Operand::Value(v) => integer_carrier(v, defs),
        Operand::Const(_) => None,
    };
    let comparison = comparison(*cond, defs);
    // The select's carrier and the comparison's may differ in signedness
    // alone (`u32 <= u32 ? 1 : 0` returns an `int`); every rewrite here is
    // about the `0`/`1` VALUE, which is bit-identical at the same width, so
    // the select's own type wins and the operands keep theirs.
    let same_size =
        comparison.is_some_and(|c| c.4.is_integer() && ty.is_integer() && c.4.size() == ty.size());
    if let (true, Some(carrier)) = (positive, carrier)
        && boolean
    {
        // Already the boolean -- only the carrier has to be restated.
        return Some(if carrier == *ty {
            Instruction::Copy {
                dest: *dest,
                src: *cond,
            }
        } else {
            Instruction::Cast {
                dest: *dest,
                src: *cond,
                from_ty: carrier,
                to_ty: *ty,
            }
        });
    }
    if let (false, true, Some((cmp_dest, cmp_op, cmp_lhs, cmp_rhs, _))) =
        (positive, same_size, comparison)
        && use_counts.get(cmp_dest.0 as usize).copied() == Some(1)
    {
        // `c ? 0 : 1` is `!c`, and the only reader of the comparison is this
        // select, so the inverted predicate replaces both.
        return Some(Instruction::Cmp {
            dest: *dest,
            op: invert(cmp_op),
            lhs: cmp_lhs,
            rhs: cmp_rhs,
            ty: *ty,
        });
    }
    if boolean {
        // A `0`/`1` condition that the rewrites above could not take (a shared
        // comparison in the negative polarity) keeps the select: testing a
        // value that is already a boolean costs a `setcc` on top of the
        // `setcc` that made it.
        return None;
    }
    // Materialize the zero-test the select was hiding.  The condition may be
    // narrower than the result only if it would have to be widened first,
    // which is not free and is not this fold's job.
    let peeled = peel_copies(*cond, defs);
    let Operand::Value(cond_value) = peeled else {
        return None;
    };
    let cond_ty = integer_carrier(cond_value, defs)?;
    if cond_ty.size() != ty.size() {
        return None;
    }
    Some(Instruction::Cmp {
        dest: *dest,
        op: if positive { IrCmpOp::Ne } else { IrCmpOp::Eq },
        lhs: *cond,
        rhs: Operand::Const(IrConst::from_i64(0, *ty)),
        ty: *ty,
    })
}

/// A comparison against zero, as `(comparison value, predicate, tested value)`.
///
/// `Eq`/`Ne` only: the merge below turns a *zero test of a disjunction* into a
/// boolean combine, and an ordering predicate has no such law.
fn zero_test(
    op: Operand,
    ty: IrType,
    defs: &[Option<Instruction>],
) -> Option<(Value, IrCmpOp, Operand)> {
    let Operand::Value(v) = peel_copies(op, defs) else {
        return None;
    };
    let Some(Instruction::Cmp {
        dest,
        op: cmp_op @ (IrCmpOp::Eq | IrCmpOp::Ne),
        lhs,
        rhs,
        ty: cmp_ty,
    }) = defs.get(v.0 as usize).and_then(Option::as_ref)
    else {
        return None;
    };
    if *cmp_ty != ty {
        return None;
    }
    let bits = ty.size() * 8;
    let value = if const_in_ring(*rhs, bits) == Some(0) {
        *lhs
    } else if const_in_ring(*lhs, bits) == Some(0) {
        *rhs
    } else {
        return None;
    };
    // A constant operand folds in `simplify`; here it would only move the
    // same constant into a new disjunction.
    matches!(value, Operand::Value(_)).then_some((*dest, *cmp_op, value))
}

/// `(a != 0) | (b != 0)` -> `(a | b) != 0`
/// and `(a == 0) & (b == 0)` -> `(a | b) == 0`.
///
/// Both are the same fact about bitwise disjunction: a disjunction is zero
/// exactly when both inputs are zero.  The `Ne`/`Or` form is its contrapositive
/// and the `Eq`/`And` form is its De Morgan dual; the two MIXED forms are not
/// rewrites at all (`(a == 0) | (b == 0)` is about two *separate* zeros), and
/// neither is `(a != 0) & (b != 0)` — that one is false for every pair that
/// shares no set bit, so it is refused rather than approximated.
///
/// The answer is a spliced pair: a fresh disjunction and the comparison that
/// keeps `dest`.  Both comparisons must have exactly one reader (this merge),
/// otherwise the selector would stay live and the splice would ADD an
/// instruction instead of removing two.
pub(crate) fn fold_predicate_merge(
    inst: &Instruction,
    defs: &[Option<Instruction>],
    use_counts: &[u32],
    fresh: Value,
) -> Option<(Vec<Instruction>, Instruction)> {
    let Instruction::BinOp {
        dest,
        op,
        lhs,
        rhs,
        ty,
    } = inst
    else {
        return None;
    };
    if !ty.is_integer() {
        return None;
    }
    let (left, right) = (zero_test(*lhs, *ty, defs)?, zero_test(*rhs, *ty, defs)?);
    let result_op = match (op, left.1, right.1) {
        (IrBinOp::Or, IrCmpOp::Ne, IrCmpOp::Ne) => IrCmpOp::Ne,
        (IrBinOp::And, IrCmpOp::Eq, IrCmpOp::Eq) => IrCmpOp::Eq,
        _ => return None,
    };
    for comparison in [left.0, right.0] {
        if use_counts.get(comparison.0 as usize).copied() != Some(1) {
            return None;
        }
    }
    let merged = Instruction::BinOp {
        dest: fresh,
        op: IrBinOp::Or,
        lhs: left.2,
        rhs: right.2,
        ty: *ty,
    };
    let result = Instruction::Cmp {
        dest: *dest,
        op: result_op,
        lhs: Operand::Value(fresh),
        rhs: Operand::Const(IrConst::from_i64(0, *ty)),
        ty: *ty,
    };
    Some((vec![merged], result))
}
