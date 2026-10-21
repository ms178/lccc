//! Observe bounded single-bit shifts directly through a mask comparison.
//! The shift count must have an explicit scalar AND bound below the word
//! width. Casts are followed only to prove that this bound survives; the
//! observation keeps the original count operand and its actual type. Do not infer a count range backwards from C undefined behavior or
//! LLVM poison, and do not narrow or mask the count as part of this rewrite.
use super::*;

fn bounded_count(op: Operand, cx: &Context<'_>, width: usize, depth: u8) -> Option<IrType> {
    if depth == 0 {
        return None;
    }
    match cx.def(op)? {
        Instruction::Copy { src, .. } => bounded_count(*src, cx, width, depth - 1),
        Instruction::Cast {
            src,
            from_ty,
            to_ty,
            ..
        } if scalar(*from_ty) && scalar(*to_ty) => {
            let source = bounded_count(*src, cx, width, depth - 1)?;
            // The bound is at most 63 and fits every admitted signed/unsigned
            // destination. No truncation, sign extension or bit reinterpretation
            // can change such a nonnegative value.
            (source == *from_ty).then_some(*to_ty)
        }
        Instruction::BinOp {
            op: IrBinOp::And,
            ty,
            ..
        } if scalar(*ty) => {
            let (_, limit) = cx.with_const(op, IrBinOp::And, *ty)?;
            (limit < width as u64).then_some(*ty)
        }
        _ => None,
    }
}

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
    let width = ty.size() * 8;
    let value = if const_in_ring(*rhs, width) == Some(0) {
        *lhs
    } else if const_in_ring(*lhs, width) == Some(0) {
        *rhs
    } else {
        return None;
    };
    let (shift, mask) = cx.with_const(value, IrBinOp::And, *ty)?;
    let Instruction::BinOp {
        op: direction,
        lhs: base,
        rhs: count,
        ty: word,
        ..
    } = cx.def(shift)?
    else {
        return None;
    };
    if *word != *ty || !matches!(direction, IrBinOp::Shl | IrBinOp::LShr) {
        return None;
    }
    let power = const_in_ring(*base, width)?;
    if !power.is_power_of_two() {
        return None;
    }
    let count_ty = bounded_count(*count, cx, width, 8)?;
    for node in [value, shift] {
        if !matches!(node, Operand::Value(v) if cx.uses.get(v.0 as usize) == Some(&1)) {
            return None;
        }
    }
    let position = i64::from(power.trailing_zeros());
    let nonzero = *op == IrCmpOp::Ne;
    let (predicate, bound) = if mask.is_power_of_two() {
        let bit = i64::from(mask.trailing_zeros());
        let target = if *direction == IrBinOp::Shl {
            bit - position
        } else {
            position - bit
        };
        if target < 0 {
            return Some(copy(*dest, constant(u64::from(!nonzero), IrType::I32)));
        }
        (
            if nonzero { IrCmpOp::Eq } else { IrCmpOp::Ne },
            target as u64,
        )
    } else if *direction == IrBinOp::Shl && mask != u64::MAX && (mask + 1).is_power_of_two() {
        let bits = i64::from(mask.count_ones());
        if position >= bits {
            return Some(copy(*dest, constant(u64::from(!nonzero), IrType::I32)));
        }
        (
            if nonzero { IrCmpOp::Ult } else { IrCmpOp::Uge },
            (bits - position) as u64,
        )
    } else {
        return None;
    };
    Some(vec![Instruction::Cmp {
        dest: *dest,
        op: predicate,
        lhs: *count,
        rhs: constant(bound, count_ty),
        ty: count_ty,
    }])
}

#[cfg(test)]
#[path = "bit_idioms_shift_observe_tests.rs"]
mod tests;
