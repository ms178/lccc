//! Exact guarded relations over wrapping integers; missing guards refuse.
use super::*;
fn cmp(op: Operand, cx: &Context<'_>) -> Option<(IrCmpOp, Operand, Operand, IrType)> {
    match cx.def(op)? {
        Instruction::Cmp {
            op, lhs, rhs, ty, ..
        } if scalar(*ty) => Some((*op, *lhs, *rhs, *ty)),
        _ => None,
    }
}
pub(super) fn fold(inst: &Instruction, cx: &Context<'_>) -> Option<Vec<Instruction>> {
    if let Instruction::BinOp {
        dest,
        op: IrBinOp::And,
        lhs,
        rhs,
        ..
    } = inst
    {
        for (a, b) in [(*lhs, *rhs), (*rhs, *lhs)] {
            let Some((pa, x, c, ty)) = cmp(a, cx) else {
                continue;
            };
            let Some((pb, y, z, other)) = cmp(b, cx) else {
                continue;
            };
            if other != ty {
                continue;
            }
            let width = ty.size() * 8;
            let full = u64::MAX >> (64 - width);
            // (x != MAX) & (x+1 <=u y) is exactly x <u y.
            if pa == IrCmpOp::Ne
                && const_in_ring(c, width) == Some(full)
                && pb == IrCmpOp::Ule
                && cx.with_const(y, IrBinOp::Add, ty) == Some((x, 1))
            {
                return Some(vec![Instruction::Cmp {
                    dest: *dest,
                    op: IrCmpOp::Ult,
                    lhs: x,
                    rhs: z,
                    ty,
                }]);
            }
            // Under x < 2^k, a contiguous mask with its top bit at k-1
            // is full iff x >= mask. Sparse masks deliberately fail.
            if pa == IrCmpOp::Ult
                && pb == IrCmpOp::Ne
                && let Some(bound) = const_in_ring(c, width)
                && bound.is_power_of_two()
                && let Some((base, mask)) = cx.with_const(y, IrBinOp::And, ty)
                && base == x
                && const_in_ring(z, width) == Some(mask)
                && mask != 0
                && mask.checked_add(1u64 << mask.trailing_zeros()) == Some(bound)
            {
                return Some(vec![Instruction::Cmp {
                    dest: *dest,
                    op: IrCmpOp::Ult,
                    lhs: x,
                    rhs: constant(mask, ty),
                    ty,
                }]);
            }
        }
    }
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
    if !scalar(*ty) {
        return None;
    }
    let width = ty.size() * 8;
    // Adding two canonical Boolean extensions cannot overflow any admitted
    // carrier. Do not reinterpret sext(i1) == -1 as zext(i1) == 1.
    if *op == IrCmpOp::Eq
        && const_in_ring(*rhs, width) == Some(1)
        && let Some((a, b)) = cx.binary(*lhs, IrBinOp::Add, *ty)
        && let (Some(a), Some(b)) = (cx.boolean(a), cx.boolean(b))
    {
        return Some(vec![bin(*dest, IrBinOp::Xor, a, b, IrType::I32)]);
    }
    // Two identical disjoint nonzero constant choices intersect exactly
    // when their predicates have equal truth values.
    if matches!(op, IrCmpOp::Eq | IrCmpOp::Ne)
        && const_in_ring(*rhs, width) == Some(0)
        && let Some((a, b)) = cx.binary(*lhs, IrBinOp::And, *ty)
    {
        let choice = |value| match cx.def(value)? {
            Instruction::Select {
                cond,
                true_val,
                false_val,
                ty: word,
                ..
            } if word == ty => Some((
                cx.boolean(*cond)?,
                const_in_ring(*true_val, width)?,
                const_in_ring(*false_val, width)?,
            )),
            _ => None,
        };
        if let (Some((ca, a, b)), Some((cb, c, d))) = (choice(a), choice(b))
            && a != 0
            && b != 0
            && a & b == 0
            && a == c
            && b == d
        {
            return Some(vec![Instruction::Cmp {
                dest: *dest,
                op: if *op == IrCmpOp::Eq {
                    IrCmpOp::Ne
                } else {
                    IrCmpOp::Eq
                },
                lhs: ca,
                rhs: cb,
                ty: IrType::I32,
            }]);
        }
    }
    // lshr(x,s) <=u x <=u (x|z). Count validity is proved explicitly;
    // neither source UB nor LLVM poison supplies a range fact.
    if matches!(op, IrCmpOp::Ult | IrCmpOp::Uge)
        && let Some((a, b)) = cx.binary(*lhs, IrBinOp::Or, *ty)
        && let Some((x, count)) = cx.binary(*rhs, IrBinOp::LShr, *ty)
        && (x == a || x == b)
        && (fixed_shift_count(count, width).is_some()
            || shift_observe::bounded_count(count, cx, width, 8).is_some())
    {
        return Some(copy(
            *dest,
            constant(u64::from(*op == IrCmpOp::Uge), IrType::I32),
        ));
    }
    None
}
