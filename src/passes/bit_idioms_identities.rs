//! Ring identities that hold for every bit pattern of the operand type, so no
//! UB, no-wrap, range or dominance premise is needed:
//!
//! * `x % C` compared equal/different against `x` collapses to one wrapping
//!   add and one unsigned compare, because `x % C == x` exactly when `x / C`
//!   is zero.
//! * A min/max select saturated at the extreme value of its own signedness is
//!   the identity on the surviving operand, and a select whose condition an
//!   arm forces true keeps that arm whenever the other arm is zero on its
//!   zero. The or-chain (`(a | b | c) ? a : 0`), the zero test (`x ? x : 0`)
//!   and the nested boolean select (`(a != 0 || b) ? a : 0`) are all instances
//!   of the same two facts.
//! * `~(0 - x)` is `x - 1` and `0 - ~x` is `x + 1` in two's complement.
//! * `((x ^ m) >>u n) ^ m` with `m` the sign broadcast of `x` is `x >>s n`,
//!   the two's complement identity `x >>s n == ~(~x >>u n)`.
//! * A product by a value that is only ever `0` or `1` is `x` when that
//!   boolean is forced to `1` by a non-zero `x`, and the select `b ? x : 0`
//!   otherwise, which removes the multiplier and the `setcc` that fed it.
//!
//! Each edit rewrites one consumer and leaves producers, memory and shared
//! users untouched. Nothing here inspects a loop, a block order or a dominator
//! tree, and nothing assumes a value is poison-free or in range: every rewrite
//! is an equality in the ring of `2^w` residues, which is exactly the
//! semantics the IR gives an integer type.
use super::*;
use std::cell::Cell;

/// Same authority contract as the other fold families: the whole sub-pass can
/// be switched off for A/B measurement and for bisecting a miscompile.
pub(super) fn disabled() -> bool {
    static FLAG: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var_os("CCC_NO_RING_IDENTITIES").is_some());
    *FLAG
}

fn ring_mask(ty: IrType) -> u64 {
    u64::MAX >> (64 - ty.size() * 8)
}

/// Inclusive ring bounds `[1 - |C|, |C| - 1]` of `{x : x / C == 0}`.
///
/// `|C|` is taken as a signed magnitude so that `C = INT_MIN`, whose negation
/// is not representable, still yields the window `[INT_MIN + 1, INT_MAX]`.
/// Both bounds stay inside the ring and the window never wraps the signed
/// range, so its unsigned span `2 * |C| - 2` is a plain signed distance.
fn quotient_zero_window(divisor: u64, ty: IrType) -> Option<(u64, u64)> {
    let w = ty.size() * 8;
    let mask = ring_mask(ty);
    let shift = 64 - w;
    let signed = ((divisor << shift) as i64) >> shift;
    // A zero divisor is division by zero and never reaches this fold.
    let magnitude = signed.unsigned_abs();
    if magnitude == 0 {
        return None;
    }
    debug_assert!(magnitude <= 1u64 << (w - 1));
    Some((
        1u64.wrapping_sub(magnitude) & mask,
        magnitude.wrapping_sub(1) & mask,
    ))
}

/// `x % C <op> x` for `Eq`/`Ne` becomes `(x + 1 - |C|) <=u 2 * |C| - 2` plus
/// its strict complement. Shifting the window down by its lower bound maps it
/// onto `0 ..= high - low`, and every remaining ring value lands above it, so
/// a single unsigned compare decides membership.
fn remainder_identity(
    dest: Value,
    op: IrCmpOp,
    lhs: Operand,
    rhs: Operand,
    ty: IrType,
    cx: &Context<'_>,
    fresh: Value,
) -> Option<Vec<Instruction>> {
    // `SRem` is signed arithmetic by definition, and lccc agrees with itself on
    // that only for signed types: the x86 backend lowers `SRem` to `idiv` for
    // every type, while the constant folder evaluates the operands as stored
    // (zero-extended for `U32`/`U64`). The front end never emits an unsigned
    // `SRem`, so requiring a signed type costs nothing and keeps this fold away
    // from the one shape where the two disagree.
    if !matches!(ty, IrType::I8 | IrType::I16 | IrType::I32 | IrType::I64) {
        return None;
    }
    let private = |x| matches!(x, Operand::Value(v) if cx.uses.get(v.0 as usize) == Some(&1));
    let strict = match op {
        IrCmpOp::Eq => false,
        IrCmpOp::Ne => true,
        _ => return None,
    };
    for (value, other) in [(lhs, rhs), (rhs, lhs)] {
        let Some(Instruction::BinOp {
            op: IrBinOp::SRem,
            lhs: dividend,
            rhs: divisor,
            ty: word,
            ..
        }) = cx.def(value)
        else {
            // A constant or a parameter has no definition: try the other
            // operand order instead of abandoning the fold.
            continue;
        };
        if *word != ty || *dividend != other || !private(value) {
            continue;
        }
        let Operand::Const(c) = *divisor else {
            continue;
        };
        let (low, high) = quotient_zero_window(c.to_i128()? as u64, ty)?;
        let span = high.wrapping_sub(low) & ring_mask(ty);
        return Some(vec![
            bin(
                fresh,
                IrBinOp::Add,
                other,
                constant(low.wrapping_neg(), ty),
                ty,
            ),
            Instruction::Cmp {
                dest,
                op: if strict { IrCmpOp::Ugt } else { IrCmpOp::Ule },
                lhs: Operand::Value(fresh),
                rhs: constant(span, ty),
                ty,
            },
        ]);
    }
    None
}

/// The truth value the condition takes whenever `value` is non-zero.
///
/// `Some(true)` means `value != 0` forces the condition true, `Some(false)`
/// means it forces it false, `None` means the structure proves neither. An
/// or-tree over the value, or over `value != 0` tests, is forced true; a
/// zero test of the value, or of an or-tree over it, is forced false. The walk
/// is depth-capped so a shared DAG cannot be re-walked.
fn implied_condition_sense(
    node: Operand,
    value: Operand,
    ty: IrType,
    cx: &Context<'_>,
    budget: &Cell<u32>,
    depth: u8,
) -> Option<bool> {
    if depth == 0 || budget.get() == 0 {
        return None;
    }
    budget.set(budget.get() - 1);
    let zero = |op: Operand| matches!(op, Operand::Const(c) if c.to_i128() == Some(0));
    let holds = |op: Operand, depth: u8| {
        op == value || implied_condition_sense(op, value, ty, cx, budget, depth).is_some()
    };
    match cx.def(node) {
        // An or-tree is non-zero as soon as one side is.
        Some(Instruction::BinOp {
            op: IrBinOp::Or,
            lhs,
            rhs,
            ty: word,
            ..
        }) if *word == ty => {
            for side in [*lhs, *rhs] {
                match implied_condition_sense(side, value, ty, cx, budget, depth - 1) {
                    Some(true) => return Some(true),
                    // An or is non-zero whenever any side is, so a side forced
                    // false says nothing about the whole.
                    Some(false) | None => {}
                }
            }
            None
        }
        Some(Instruction::Cmp {
            op,
            lhs,
            rhs,
            ty: word,
            ..
        }) if *word == ty && matches!(op, IrCmpOp::Eq | IrCmpOp::Ne) => {
            for (probe, other) in [(*lhs, *rhs), (*rhs, *lhs)] {
                if !zero(other) {
                    continue;
                }
                if probe == value || holds(probe, depth - 1) {
                    return Some(*op == IrCmpOp::Ne);
                }
            }
            None
        }
        // A select on a condition the value forces takes the arm that
        // condition picks, so its sense is that arm's own sense: `x ? 1 : rest`
        // is non-zero whenever `x` is.
        Some(Instruction::Select {
            cond,
            true_val,
            false_val,
            ..
        }) => {
            let arm = match implied_condition_sense(*cond, value, ty, cx, budget, depth - 1)? {
                true => *true_val,
                false => *false_val,
            };
            if arm == value || holds(arm, depth - 1) {
                Some(true)
            } else if matches!(arm, Operand::Const(c) if c.to_i128() != Some(0)) {
                Some(true)
            } else if matches!(arm, Operand::Const(c) if c.to_i128() == Some(0)) {
                Some(false)
            } else {
                None
            }
        }
        // A bare integer condition is a non-zero test of itself.
        _ if node == value => Some(true),
        _ => None,
    }
}

/// Every value of this expression is `0` or `1`, so a non-zero boolean is `1`.
///
/// The property is width-independent: a compare yields `0` or `1` in its own
/// type, and `copy`, any cast, and bitwise `and`/`or`/`xor` of two such values
/// preserve it for every width. A `not` does not — `~1` is all-ones — so it is
/// deliberately absent here; the front end's `!(c)` reaches this pass as a
/// compare with the opposite predicate.
fn zero_or_one(node: Operand, cx: &Context<'_>, budget: &Cell<u32>, depth: u8) -> bool {
    if let Operand::Const(c) = node {
        return matches!(c.to_i128(), Some(0) | Some(1));
    }
    if depth == 0 || budget.get() == 0 {
        return false;
    }
    budget.set(budget.get() - 1);
    match cx.def(node) {
        Some(Instruction::Cmp { .. }) => true,
        Some(Instruction::Copy { src, .. }) => zero_or_one(*src, cx, budget, depth - 1),
        Some(Instruction::Cast { src, .. }) => zero_or_one(*src, cx, budget, depth - 1),
        Some(Instruction::BinOp {
            op: IrBinOp::And | IrBinOp::Or | IrBinOp::Xor,
            lhs,
            rhs,
            ..
        }) => zero_or_one(*lhs, cx, budget, depth - 1) && zero_or_one(*rhs, cx, budget, depth - 1),
        Some(Instruction::Select {
            true_val,
            false_val,
            ..
        }) => {
            zero_or_one(*true_val, cx, budget, depth - 1)
                && zero_or_one(*false_val, cx, budget, depth - 1)
        }
        _ => false,
    }
}

/// `node` is zero whenever `value` is zero.
///
/// Only structural witnesses are admitted: the constant zero, the value
/// itself, and producers whose every zero input forces a zero output — a copy,
/// any cast, a select over zero-forcing arms, an `and` or a `mul` with one
/// zero-forcing side, and an `or`/`xor` with two.
fn zero_when_zero(
    node: Operand,
    value: Operand,
    cx: &Context<'_>,
    budget: &Cell<u32>,
    depth: u8,
) -> bool {
    if node == value || matches!(node, Operand::Const(c) if c.to_i128() == Some(0)) {
        return true;
    }
    if depth == 0 || budget.get() == 0 {
        return false;
    }
    budget.set(budget.get() - 1);
    let side = |op: Operand| zero_when_zero(op, value, cx, budget, depth - 1);
    match cx.def(node) {
        Some(Instruction::Copy { src, .. }) => side(*src),
        Some(Instruction::Cast { src, .. }) => side(*src),
        Some(Instruction::Select {
            true_val,
            false_val,
            ..
        }) => side(*true_val) && side(*false_val),
        Some(Instruction::BinOp {
            op: IrBinOp::And | IrBinOp::Mul,
            lhs,
            rhs,
            ..
        }) => side(*lhs) || side(*rhs),
        Some(Instruction::BinOp {
            op: IrBinOp::Or | IrBinOp::Xor,
            lhs,
            rhs,
            ..
        }) => side(*lhs) && side(*rhs),
        _ => false,
    }
}

/// Extreme ring value of a signedness that can never clamp a min or a max:
/// the maximum for a minimum and the minimum for a maximum. Selecting it means
/// the other arm always survives, for every bit pattern of the type.
fn saturating_extremum(min: bool, signed: bool, ty: IrType) -> u64 {
    let w = ty.size() * 8;
    let sign = 1u64 << (w - 1);
    match (min, signed) {
        (true, true) => sign - 1,
        (true, false) => ring_mask(ty),
        (false, true) => sign,
        (false, false) => 0,
    }
}

fn saturated_bound(
    inst: &Instruction,
    cx: &Context<'_>,
    budget: &Cell<u32>,
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
    if !scalar(*ty) {
        return None;
    }
    // The zero-arm identities below hold for any condition that a non-zero arm
    // forces true, so they are decided before the condition is required to be a
    // compare.
    for (arm, other) in [(*true_val, *false_val), (*false_val, *true_val)] {
        if matches!(arm, Operand::Const(c) if c.to_i128() == Some(0)) {
            continue;
        }
        let Some(sense) = implied_condition_sense(*cond, arm, *ty, cx, budget, 8) else {
            continue;
        };
        // The value selects itself only when the condition is the one a
        // non-zero value forces it to take. The other arm then has to be zero
        // whenever this one is, and the select is that value.
        if sense == (arm == *true_val) {
            if zero_when_zero(other, arm, cx, budget, 8) {
                return Some(copy(*dest, arm));
            }
        } else if matches!(other, Operand::Const(c) if c.to_i128() == Some(0)) {
            // The opposite sense sends every non-zero value to a zero arm, and
            // a zero value selects zero, so the whole select is constant zero.
            return Some(copy(*dest, constant(0, *ty)));
        }
    }

    let Instruction::Cmp {
        op,
        lhs,
        rhs,
        ty: word,
        ..
    } = cx.def(*cond)?
    else {
        return None;
    };
    if *word != *ty {
        return None;
    }
    // A zero test with a non-zero other arm is a real clamp (`x > 0 ? x : 0`),
    // not an identity, so it falls through to the range rules below.
    if matches!(op, IrCmpOp::Eq | IrCmpOp::Ne) {
        return None;
    }

    let (less, signed) = match op {
        IrCmpOp::Ult | IrCmpOp::Ule => (true, false),
        IrCmpOp::Ugt | IrCmpOp::Uge => (false, false),
        IrCmpOp::Slt | IrCmpOp::Sle => (true, true),
        IrCmpOp::Sgt | IrCmpOp::Sge => (false, true),
        IrCmpOp::Eq | IrCmpOp::Ne => unreachable!(),
    };
    let min = if *true_val == *lhs && *false_val == *rhs {
        less
    } else if *true_val == *rhs && *false_val == *lhs {
        !less
    } else {
        return None;
    };
    let extremum = saturating_extremum(min, signed, *ty);
    for (bound, survivor) in [(*true_val, *false_val), (*false_val, *true_val)] {
        let Operand::Const(c) = bound else {
            continue;
        };
        if c.to_i128()? as u64 & ring_mask(*ty) == extremum {
            return Some(copy(*dest, survivor));
        }
    }
    None
}

/// Operand that is the all-ones broadcast of `x`'s sign bit, built either as
/// `0 - (x >>u w-1)` or as `x >>s w-1`. Both are `0` or `-1`, which is all the
/// shift identity below needs.
fn sign_broadcast(mask: Operand, x: Operand, ty: IrType, cx: &Context<'_>) -> bool {
    let width = ty.size() * 8;
    match cx.def(mask) {
        Some(Instruction::UnaryOp {
            op: IrUnaryOp::Neg,
            src: shifted,
            ty: word,
            ..
        }) if *word == ty => matches!(
            cx.def(*shifted),
            Some(Instruction::BinOp { op: IrBinOp::LShr, lhs, rhs, ty: word, .. })
                if *word == ty
                    && *lhs == x
                    && fixed_shift_count(*rhs, width) == Some(width as u32 - 1)
        ),
        Some(Instruction::BinOp {
            op: IrBinOp::AShr,
            lhs,
            rhs,
            ty: word,
            ..
        }) if *word == ty => *lhs == x && fixed_shift_count(*rhs, width) == Some(width as u32 - 1),
        _ => false,
    }
}

/// `((x ^ m) >>u n) ^ m` folds to `x >>s n` when `m` is `x`'s sign broadcast:
/// for a non-negative `x` the broadcast is zero and both shifts agree, for a
/// negative `x` it is all ones and the pair of xors is exactly the definition
/// of an arithmetic shift. Every step is bit-exact, so no signed-overflow or
/// poison premise is involved.
fn signed_shift_through_mask(inst: &Instruction, cx: &Context<'_>) -> Option<Vec<Instruction>> {
    let Instruction::BinOp {
        dest,
        op: IrBinOp::Xor,
        lhs,
        rhs,
        ty,
    } = inst
    else {
        return None;
    };
    if !scalar(*ty) {
        return None;
    }
    let uses = |x| matches!(x, Operand::Value(v) if cx.uses.get(v.0 as usize) == Some(&1));
    for (masked, mask) in [(lhs, rhs), (rhs, lhs)] {
        // The mask must vanish with its two consumers, the two xors.
        if !matches!(mask, Operand::Value(v) if cx.uses.get(v.0 as usize) == Some(&2)) {
            continue;
        }
        let Some(Instruction::BinOp {
            op: IrBinOp::LShr,
            lhs: inner,
            rhs: count,
            ty: word,
            ..
        }) = cx.def(*masked)
        else {
            continue;
        };
        if *word != *ty || !uses(*masked) {
            continue;
        }
        let Some(Instruction::BinOp {
            op: IrBinOp::Xor,
            lhs: a,
            rhs: b,
            ty: word,
            ..
        }) = cx.def(*inner)
        else {
            continue;
        };
        if *word != *ty || !uses(*inner) {
            continue;
        }
        for (x, shared) in [(a, b), (b, a)] {
            if shared != mask || !sign_broadcast(*mask, *x, *ty, cx) {
                continue;
            }
            return Some(vec![bin(*dest, IrBinOp::AShr, *x, *count, *ty)]);
        }
    }
    None
}

/// `x * b` with a `b` that is only ever `0` or `1`.
///
/// Two answers, in order of value:
///
/// * `b` is forced to `1` whenever `x` is non-zero (`x * (x || c)`, `x * !!x`).
///   A zero `x` gives zero whatever `b` is, so the product is `x`.
/// * otherwise the product is the select `b ? x : 0`, which the backend lowers
///   to a compare and a conditional move and never needs a multiplier.
///
/// Both need the boolean producer to disappear, so the factor must be private;
/// a factor with other consumers would keep its `setcc` and only add the
/// select.
fn boolean_multiply(
    inst: &Instruction,
    cx: &Context<'_>,
    budget: &Cell<u32>,
) -> Option<Vec<Instruction>> {
    let Instruction::BinOp {
        dest,
        op: IrBinOp::Mul,
        lhs,
        rhs,
        ty,
    } = inst
    else {
        return None;
    };
    if !scalar(*ty) {
        return None;
    }
    let private = |x| matches!(x, Operand::Value(v) if cx.uses.get(v.0 as usize) == Some(&1));
    for (factor, value) in [(*lhs, *rhs), (*rhs, *lhs)] {
        if !matches!(value, Operand::Value(_)) || !zero_or_one(factor, cx, budget, 8) {
            continue;
        }
        if implied_condition_sense(factor, value, *ty, cx, budget, 8) == Some(true) {
            return Some(copy(*dest, value));
        }
        if private(factor) {
            return Some(vec![Instruction::Select {
                dest: *dest,
                cond: factor,
                true_val: value,
                false_val: constant(0, *ty),
                ty: *ty,
            }]);
        }
    }
    None
}

pub(super) fn fold(inst: &Instruction, cx: &Context<'_>, fresh: Value) -> Option<Vec<Instruction>> {
    if disabled() {
        return None;
    }
    // A private budget: the walks below are the only consumers, so a sibling
    // family that exhausted the shared visit counter cannot silently disable a
    // fold that a reader of this file expects to fire.
    let budget = Cell::new(4096u32);
    match inst {
        Instruction::Cmp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        } if scalar(*ty) => remainder_identity(*dest, *op, *lhs, *rhs, *ty, cx, fresh),
        Instruction::UnaryOp { dest, op, src, ty } if scalar(*ty) => {
            let Instruction::UnaryOp {
                op: inner,
                src: operand,
                ty: word,
                ..
            } = cx.def(*src)?
            else {
                return None;
            };
            if *word != *ty
                || !matches!(src, Operand::Value(v) if cx.uses.get(v.0 as usize) == Some(&1))
            {
                return None;
            }
            let offset = match (op, inner) {
                (IrUnaryOp::Not, IrUnaryOp::Neg) => 1u64.wrapping_neg(),
                (IrUnaryOp::Neg, IrUnaryOp::Not) => 1,
                _ => return None,
            };
            Some(vec![bin(
                *dest,
                IrBinOp::Add,
                *operand,
                constant(offset, *ty),
                *ty,
            )])
        }
        Instruction::BinOp { .. } => {
            signed_shift_through_mask(inst, cx).or_else(|| boolean_multiply(inst, cx, &budget))
        }
        _ => saturated_bound(inst, cx, &budget),
    }
}

#[cfg(test)]
#[path = "bit_idioms_identities_tests.rs"]
mod tests;
