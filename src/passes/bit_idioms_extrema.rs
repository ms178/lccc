//! Observe existing scalar min/max select structure before offset arithmetic
//! can obscure its order. No LLVM assume/no-wrap metadata is borrowed.
use super::*;
#[derive(Clone, Copy)]
struct Extremum {
    min: bool,
    signed: bool,
    a: Operand,
    b: Operand,
}
fn extremum(value: Operand, ty: IrType, cx: &Context<'_>) -> Option<Extremum> {
    let Instruction::Select {
        cond,
        true_val,
        false_val,
        ty: word,
        ..
    } = cx.def(value)?
    else {
        return None;
    };
    if *word != ty {
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
    if *word != ty {
        return None;
    }
    let (less, signed) = match op {
        IrCmpOp::Ult | IrCmpOp::Ule => (true, false),
        IrCmpOp::Ugt | IrCmpOp::Uge => (false, false),
        IrCmpOp::Slt | IrCmpOp::Sle => (true, true),
        IrCmpOp::Sgt | IrCmpOp::Sge => (false, true),
        _ => return None,
    };
    let min = if true_val == lhs && false_val == rhs {
        less
    } else if true_val == rhs && false_val == lhs {
        !less
    } else {
        return None;
    };
    Some(Extremum {
        min,
        signed,
        a: *lhs,
        b: *rhs,
    })
}
pub(super) fn fold(inst: &Instruction, cx: &Context<'_>) -> Option<Vec<Instruction>> {
    match inst {
        Instruction::Cmp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        } if scalar(*ty) => {
            for (value, member, swapped) in [(*lhs, *rhs, false), (*rhs, *lhs, true)] {
                let Some(e) = extremum(value, *ty, cx) else {
                    continue;
                };
                if member != e.a && member != e.b {
                    continue;
                }
                let (less, equal, signed) = match op {
                    IrCmpOp::Ult => (true, false, false),
                    IrCmpOp::Ule => (true, true, false),
                    IrCmpOp::Ugt => (false, false, false),
                    IrCmpOp::Uge => (false, true, false),
                    IrCmpOp::Slt => (true, false, true),
                    IrCmpOp::Sle => (true, true, true),
                    IrCmpOp::Sgt => (false, false, true),
                    IrCmpOp::Sge => (false, true, true),
                    _ => continue,
                };
                if signed != e.signed {
                    continue;
                }
                let less = less != swapped;
                // min <= either operand; max >= either operand. Strict
                // comparisons in the opposite direction are false.
                let value = if less == e.min && equal {
                    true
                } else if less != e.min && !equal {
                    false
                } else {
                    continue;
                };
                return Some(copy(*dest, constant(u64::from(value), IrType::I32)));
            }
            None
        }
        Instruction::Select {
            dest,
            cond,
            true_val,
            false_val,
            ty,
        } if scalar(*ty) => {
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
            if *word != *ty || !matches!(op, IrCmpOp::Eq | IrCmpOp::Ne) {
                return None;
            }
            let width = ty.size() * 8;
            let full = u64::MAX >> (64 - width);
            let x = if const_in_ring(*rhs, width) == Some(full) {
                *lhs
            } else if const_in_ring(*lhs, width) == Some(full) {
                *rhs
            } else {
                return None;
            };
            let (bound, min) = if *op == IrCmpOp::Eq {
                (*true_val, *false_val)
            } else {
                (*false_val, *true_val)
            };
            let e = extremum(min, *ty, cx)?;
            if e.min && !e.signed && ((e.a == x && e.b == bound) || (e.b == x && e.a == bound)) {
                Some(copy(*dest, min))
            } else {
                None
            }
        }
        _ => None,
    }
}
#[cfg(test)]
#[path = "bit_idioms_extrema_tests.rs"]
mod tests;
