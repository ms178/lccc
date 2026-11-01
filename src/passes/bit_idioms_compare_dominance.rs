//! Dominance between two comparisons of one value joined by `&` or `|`.
//!
//! Comparing one value twice and joining the results is how portable code
//! spells "non-zero and large", "positive and above a floor", "below a and
//! below b":
//!
//! ```c
//! return (x != 0) & (x > 4);   /* the second test already excludes zero */
//! return (x > 0) & (x > 99);   /* the tighter floor decides both          */
//! return (x < 10) & (x < 5);   /* the tighter ceiling decides both        */
//! ```
//!
//! When one comparison's value set contains the other's, the join collapses to
//! a single operand of the join: `A & B = B` if `A` implies `B`, and
//! `A | B = A` under the same implication.  Every backend then emits one
//! compare instead of two compares, two materializations and the logic op.
//!
//! The replacement is a `Copy` of one of the join's OWN operands, so it cannot
//! change a type, a width or a boolean carrier: the fold only has to be right
//! about the implication, never about how the surviving value is represented.
//! That is also why the compared value may be read through casts here — the
//! casts stay where they are and are never dropped.
//!
//! Domain: a comparison denotes a set of bit patterns, ordered either signed or
//! unsigned.  Two ordering comparisons are only combined when they agree on the
//! ordering (`x < 5` and `x <u 5` share no ordering and are refused); `Eq`/`Ne`
//! are ordering-free and join either domain.  Sets are modelled over `i128`
//! with sentinel infinities, and the containment test is exact for the shapes a
//! single comparison can denote (interval, singleton, co-singleton), so the
//! fold never needs a value range analysis and never assumes a width's
//! extremes: a comparison that is empty in its own ring (`u32 x < 0`) simply
//! stays unmatched, which is a missed fold, never a wrong one.

use super::*;

/// A comparison of one value against a constant, read through copies and
/// integer casts, with the ordering normalized so the value is on the left
/// (`4 > x` is stored as `x < 4`).
struct AgainstConstant {
    op: IrCmpOp,
    value: Operand,
    constant: u64,
    ty: IrType,
}

/// Reverse a comparison's operands.
fn mirror_cmp(op: IrCmpOp) -> IrCmpOp {
    match op {
        IrCmpOp::Eq | IrCmpOp::Ne => op,
        IrCmpOp::Slt => IrCmpOp::Sgt,
        IrCmpOp::Sle => IrCmpOp::Sge,
        IrCmpOp::Sgt => IrCmpOp::Slt,
        IrCmpOp::Sge => IrCmpOp::Sle,
        IrCmpOp::Ult => IrCmpOp::Ugt,
        IrCmpOp::Ule => IrCmpOp::Uge,
        IrCmpOp::Ugt => IrCmpOp::Ult,
        IrCmpOp::Uge => IrCmpOp::Ule,
    }
}

fn against_constant(opnd: Operand, defs: &[Option<Instruction>]) -> Option<AgainstConstant> {
    let Operand::Value(v) = peel(opnd, defs) else {
        return None;
    };
    let Some(Instruction::Cmp {
        op, lhs, rhs, ty, ..
    }) = defs.get(v.0 as usize).and_then(Option::as_ref)
    else {
        return None;
    };
    if !ty.is_integer() {
        return None;
    }
    let bits = ty.size() * 8;
    if let Some(constant) = const_in_ring(*rhs, bits) {
        return Some(AgainstConstant {
            op: *op,
            value: *lhs,
            constant,
            ty: *ty,
        });
    }
    if let Some(constant) = const_in_ring(*lhs, bits) {
        return Some(AgainstConstant {
            op: mirror_cmp(*op),
            value: *rhs,
            constant,
            ty: *ty,
        });
    }
    None
}

/// The set of values a single comparison of a constant admits: an inclusive
/// interval (with sentinel infinities) that may exclude exactly one point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ValueSet {
    lo: i128,
    hi: i128,
    excluded: Option<i128>,
}

const NEG_INF: i128 = i128::MIN;
const POS_INF: i128 = i128::MAX;

/// The constant as a number in the comparison's own ordering.  `const_in_ring`
/// returns the bit pattern; a signed ordering reads that pattern sign-extended
/// from the compared width, an unsigned one zero-extended.
fn constant_value(pattern: u64, bits: usize, signed: bool) -> i128 {
    if !signed || bits == 0 || bits > 64 {
        return pattern as i128;
    }
    let shift = 64 - bits;
    (((pattern << shift) as i64) >> shift) as i128
}

fn value_set(op: IrCmpOp, constant: i128) -> ValueSet {
    match op {
        IrCmpOp::Eq => ValueSet {
            lo: constant,
            hi: constant,
            excluded: None,
        },
        IrCmpOp::Ne => ValueSet {
            lo: NEG_INF,
            hi: POS_INF,
            excluded: Some(constant),
        },
        IrCmpOp::Slt | IrCmpOp::Ult => ValueSet {
            lo: NEG_INF,
            hi: constant - 1,
            excluded: None,
        },
        IrCmpOp::Sle | IrCmpOp::Ule => ValueSet {
            lo: NEG_INF,
            hi: constant,
            excluded: None,
        },
        IrCmpOp::Sgt | IrCmpOp::Ugt => ValueSet {
            lo: constant + 1,
            hi: POS_INF,
            excluded: None,
        },
        IrCmpOp::Sge | IrCmpOp::Uge => ValueSet {
            lo: constant,
            hi: POS_INF,
            excluded: None,
        },
    }
}

/// `inner ⊆ outer`, exactly, for the interval/singleton/co-singleton shapes
/// [`value_set`] produces.  The interval bound check is necessary and the
/// exclusion check then decides the one point the interval check can hide.
fn is_subset(inner: ValueSet, outer: ValueSet) -> bool {
    if inner.lo < outer.lo || inner.hi > outer.hi {
        return false;
    }
    match outer.excluded {
        None => true,
        Some(point) => point < inner.lo || point > inner.hi || inner.excluded == Some(point),
    }
}

/// The ordering both comparisons share, or `None` when they disagree.  `Eq`/`Ne`
/// admit either ordering, so an unconstrained sibling decides.
fn shared_ordering(a: IrCmpOp, b: IrCmpOp) -> Option<bool> {
    let signed = |op| match op {
        IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge => Some(true),
        IrCmpOp::Ult | IrCmpOp::Ule | IrCmpOp::Ugt | IrCmpOp::Uge => Some(false),
        IrCmpOp::Eq | IrCmpOp::Ne => None,
    };
    match (signed(a), signed(b)) {
        (Some(a), Some(b)) if a != b => None,
        (Some(a), _) => Some(a),
        (_, Some(b)) => Some(b),
        // Two equality-only tests: the ordering is irrelevant to the answer.
        _ => Some(false),
    }
}

/// `(a != 0) & (a > 4)` is `a > 4`; `(a > 0) & (a > 99)` is `a > 99`;
/// `(a < 10) & (a < 5)` is `a < 5`; `(a != 0) | (a > 4)` is `a != 0`.
///
/// Requires both join operands to be comparisons of the same SSA value at the
/// same integer type, so both sides are 0/1 and `&`/`|` are logical.  The
/// replacement copies the surviving operand, which keeps the join's own type
/// and boolean carrier.
pub(super) fn fold_compare_dominance(
    inst: &Instruction,
    defs: &[Option<Instruction>],
) -> Option<Instruction> {
    let Instruction::BinOp {
        dest,
        op: logic @ (IrBinOp::And | IrBinOp::Or),
        lhs,
        rhs,
        ty: _,
    } = inst
    else {
        return None;
    };
    let a = against_constant(*lhs, defs)?;
    let b = against_constant(*rhs, defs)?;
    if a.ty != b.ty || !same_value(a.value, b.value, defs) {
        return None;
    }
    let signed = shared_ordering(a.op, b.op)?;
    let bits = a.ty.size() * 8;
    let set_a = value_set(a.op, constant_value(a.constant, bits, signed));
    let set_b = value_set(b.op, constant_value(b.constant, bits, signed));
    // A conjunction is its stronger conjunct and a disjunction its weaker one:
    // `a & b` is `a` when `a` implies `b`, and `b` when `b` implies `a`; the
    // disjunction keeps the opposite operand under each implication.  (The
    // first draft had both backwards, which is why the matcher tests below
    // evaluate both directions against the original pair instead of asserting
    // the operand they expect.)
    let keep = if *logic == IrBinOp::And {
        if is_subset(set_a, set_b) {
            *lhs
        } else if is_subset(set_b, set_a) {
            *rhs
        } else {
            return None;
        }
    } else if is_subset(set_b, set_a) {
        *lhs
    } else if is_subset(set_a, set_b) {
        *rhs
    } else {
        return None;
    };
    Some(Instruction::Copy {
        dest: *dest,
        src: keep,
    })
}
