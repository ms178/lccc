//! Observe scalar counts under bounded, consumer-local bit facts. Count
//! outputs lie in 0..=64, so explicit scalar integer casts preserve them.
//! No producer is mutated, moved, cloned or made nonzero by assumption.
use super::*;

fn scan(
    cx: &Context<'_>,
    value: Operand,
    result_width: usize,
    depth: u8,
) -> Option<(IrUnaryOp, Operand, IrType)> {
    if depth == 0 || !matches!(result_width, 8 | 16 | 32 | 64) {
        return None;
    }
    match cx.def(value)? {
        Instruction::Copy { src, .. } => scan(cx, *src, result_width, depth - 1),
        Instruction::Cast {
            src,
            from_ty,
            to_ty,
            ..
        } if scalar(*to_ty) && to_ty.size() * 8 == result_width && scalar(*from_ty) => {
            scan(cx, *src, from_ty.size() * 8, depth - 1)
        }
        Instruction::UnaryOp { op, src, ty, .. }
            if scalar(*ty)
                && matches!(
                    op,
                    IrUnaryOp::Popcount
                        | IrUnaryOp::Clz
                        | IrUnaryOp::Ctz
                        | IrUnaryOp::ClzNonZero
                        | IrUnaryOp::CtzNonZero
                ) =>
        {
            Some((*op, *src, *ty))
        }
        _ => None,
    }
}

fn bounds(cx: &Context<'_>, value: Operand, ty: IrType, facts: &Facts) -> Option<(u64, u64)> {
    if !scalar(ty) {
        return None;
    }
    let (op, src, word) = scan(cx, value, ty.size() * 8, 8)?;
    let width = word.size() * 8;
    let mask = u64::MAX >> (64 - width);
    let (zero, one) = cx.known(src, word, facts, 8);
    if zero & one != 0 {
        return None; // Contradictory facts are not a license to pick a value.
    }
    // Do not derive a nonzero scan's range from its undefined-at-zero
    // contract. Demand an actual known-one bit in this observation's scope.
    if matches!(op, IrUnaryOp::ClzNonZero | IrUnaryOp::CtzNonZero) && one == 0 {
        return None;
    }
    let possible = mask & !zero;
    let leading = |n: u64| u64::from(n.leading_zeros()) - (64 - width) as u64;
    let trailing = |n: u64| u64::from(n.trailing_zeros()).min(width as u64);
    Some(match op {
        IrUnaryOp::Popcount => (
            u64::from(one.count_ones()),
            u64::from(possible.count_ones()),
        ),
        IrUnaryOp::Clz | IrUnaryOp::ClzNonZero => (leading(possible), leading(one)),
        IrUnaryOp::Ctz | IrUnaryOp::CtzNonZero => (trailing(possible), trailing(one)),
        _ => return None,
    })
}

fn reverse(op: IrCmpOp) -> IrCmpOp {
    match op {
        IrCmpOp::Eq | IrCmpOp::Ne => op,
        IrCmpOp::Ult => IrCmpOp::Ugt,
        IrCmpOp::Ule => IrCmpOp::Uge,
        IrCmpOp::Ugt => IrCmpOp::Ult,
        IrCmpOp::Uge => IrCmpOp::Ule,
        IrCmpOp::Slt => IrCmpOp::Sgt,
        IrCmpOp::Sle => IrCmpOp::Sge,
        IrCmpOp::Sgt => IrCmpOp::Slt,
        IrCmpOp::Sge => IrCmpOp::Sle,
    }
}

// Unary count `ty` describes the input word. Frontend C builtins may
// consume its result as I32 even for U64 inputs, without an explicit cast.
// All admitted carriers preserve 0..=64; keep source width separate.
fn source_zero(cx: &Context<'_>, src: Operand, ty: IrType, facts: &Facts) -> Option<bool> {
    for &(value, width, zero, one) in &facts.0 {
        let Some((IrUnaryOp::Popcount, input, word)) = scan(cx, value, width, 8) else {
            continue;
        };
        if input != src || word != ty || zero & one != 0 {
            continue;
        }
        if one != 0 {
            return Some(false);
        }
        if zero == u64::MAX >> (64 - width) {
            return Some(true);
        }
    }
    None
}

pub(super) fn comparison(
    cx: &Context<'_>,
    op: IrCmpOp,
    lhs: Operand,
    rhs: Operand,
    ty: IrType,
    facts: &Facts,
) -> Option<bool> {
    if !scalar(ty) {
        return None;
    }
    for (value, constant_operand, op) in [(lhs, rhs, op), (rhs, lhs, reverse(op))] {
        let Some(k) = const_in_ring(constant_operand, ty.size() * 8) else {
            continue;
        };
        if k == 0
            && matches!(op, IrCmpOp::Eq | IrCmpOp::Ne)
            && let Some(is_zero) = source_zero(cx, value, ty, facts)
        {
            return Some(is_zero == (op == IrCmpOp::Eq));
        }
        let Some((lo, hi)) = bounds(cx, value, ty, facts) else {
            continue;
        };
        if matches!(op, IrCmpOp::Eq | IrCmpOp::Ne) {
            if lo == hi {
                return Some(compare(op, lo, k, ty.size() * 8));
            }
            if k < lo || k > hi {
                return Some(op == IrCmpOp::Ne);
            }
        } else {
            // Every ordered predicate is monotone on this nonnegative,
            // nonwrapping interval, even for a negative signed constant.
            let a = compare(op, lo, k, ty.size() * 8);
            let b = compare(op, hi, k, ty.size() * 8);
            if a == b {
                return Some(a);
            }
        }
    }
    None
}

// Fold only a terminal observation certified by this block's unique incoming
// edge. Return a constant condition, not a new CFG edge: normal CFG cleanup
// owns successor/PHI maintenance after the frozen-definition sweep.
pub(super) fn branch(cond: Operand, cx: &Context<'_>, facts: &Facts, depth: u8) -> Option<bool> {
    if depth == 0 {
        return None;
    }
    let inst = cx.def(cond)?;
    match inst {
        Instruction::Copy { src, .. } => branch(*src, cx, facts, depth - 1),
        Instruction::Cmp {
            op, lhs, rhs, ty, ..
        } => comparison(cx, *op, *lhs, *rhs, *ty, facts),
        _ => {
            let ty = inst.result_type()?;
            if !scalar(ty) {
                return None;
            }
            source_zero(cx, cond, ty, facts).map(|zero| !zero)
        }
    }
}

// Bridge the frontend's input-word / I32-result distinction for exact
// zero/full observations. The existing same-width observation normalizer
// cannot express this type-changing comparison. Only private scan/cast/copy
// chains qualify: keep shared producers and their register lifetimes intact.
pub(super) fn fold(inst: &Instruction, cx: &Context<'_>) -> Option<Vec<Instruction>> {
    let Instruction::Cmp {
        dest,
        op,
        lhs,
        rhs,
        ty,
    } = inst
    else {
        return None;
    };
    if !scalar(*ty) || !matches!(op, IrCmpOp::Eq | IrCmpOp::Ne) {
        return None;
    }
    for (value, k) in [(*lhs, *rhs), (*rhs, *lhs)] {
        let Some(k) = const_in_ring(k, ty.size() * 8) else {
            continue;
        };
        let Some((kind, src, word)) = scan(cx, value, ty.size() * 8, 8) else {
            continue;
        };
        let width = word.size() * 8;
        let wanted = match kind {
            IrUnaryOp::Popcount if k == 0 => 0,
            IrUnaryOp::Popcount if k == width as u64 => u64::MAX >> (64 - width),
            IrUnaryOp::Clz | IrUnaryOp::Ctz if k == width as u64 => 0,
            _ => continue,
        };
        let mut cursor = value;
        let mut private = false;
        for _ in 0..8 {
            if !matches!(cursor, Operand::Value(v) if cx.uses.get(v.0 as usize)==Some(&1)) {
                break;
            }
            match cx.def(cursor) {
                Some(Instruction::Copy { src, .. }) | Some(Instruction::Cast { src, .. }) => {
                    cursor = *src
                }
                Some(Instruction::UnaryOp { .. }) => {
                    private = true;
                    break;
                }
                _ => break,
            }
        }
        if private {
            return Some(vec![Instruction::Cmp {
                dest: *dest,
                op: *op,
                lhs: src,
                rhs: constant(wanted, word),
                ty: word,
            }]);
        }
    }
    None
}

#[cfg(test)]
#[path = "bit_idioms_count_observe_tests.rs"]
mod tests;
