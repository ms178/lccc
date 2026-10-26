//! Ring identities that hold for every bit pattern of the operand type, so no
//! UB, no-wrap, range or dominance premise is needed:
//!
//! * `x % C` compared equal/different against `x` collapses to one wrapping
//!   add and one unsigned compare, because `x % C == x` exactly when `x / C`
//!   is zero.
//! * A min/max select saturated at the extreme value of its own signedness is
//!   the identity on the surviving operand.
//! * `~(0 - x)` is `x - 1` and `0 - ~x` is `x + 1` in two's complement.
//! * `((x ^ m) >>u n) ^ m` with `m` the sign broadcast of `x` is `x >>s n`,
//!   the two's complement identity `x >>s n == ~(~x >>u n)`.
//!
//! Each edit rewrites one consumer and leaves producers, memory and shared
//! users untouched.
use super::*;

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

fn saturated_bound(inst: &Instruction, cx: &Context<'_>) -> Option<Vec<Instruction>> {
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
    // A zero test is the canonical form a clamp against zero reaches after
    // boolean canonicalisation: `x != 0 ? x : 0` and `x == 0 ? 0 : x` are both
    // `x`, and the two reversed arm orders are real zeroing choices that must
    // keep their compare.
    if matches!(op, IrCmpOp::Eq | IrCmpOp::Ne) {
        let zero_arm = |arm: Operand| matches!(arm, Operand::Const(c) if c.to_i128() == Some(0));
        for (value, tested) in [(*lhs, *rhs), (*rhs, *lhs)] {
            if !zero_arm(tested) {
                continue;
            }
            let identity = match (op, *true_val == value, *false_val == value) {
                (IrCmpOp::Ne, true, false) => zero_arm(*false_val),
                (IrCmpOp::Eq, false, true) => zero_arm(*true_val),
                _ => false,
            };
            if identity {
                return Some(copy(*dest, value));
            }
        }
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

pub(super) fn fold(inst: &Instruction, cx: &Context<'_>, fresh: Value) -> Option<Vec<Instruction>> {
    if disabled() {
        return None;
    }
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
        Instruction::BinOp { .. } => signed_shift_through_mask(inst, cx),
        _ => saturated_bound(inst, cx),
    }
}

#[cfg(test)]
#[path = "bit_idioms_identities_tests.rs"]
mod tests;
