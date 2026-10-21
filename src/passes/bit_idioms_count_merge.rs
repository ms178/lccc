//! Compose scalar zero counts at a minimum consumer. Exact widths and use
//! counts keep shared scans intact; no variable shifts or vector costs enter.
use super::*;

fn count(cx: &Context<'_>, operand: Operand, ty: IrType) -> Option<(IrUnaryOp, Operand)> {
    match cx.def(operand)? {
        Instruction::UnaryOp { op, src, ty: t, .. }
            if *t == ty
                && matches!(
                    op,
                    IrUnaryOp::Clz | IrUnaryOp::ClzNonZero | IrUnaryOp::Ctz | IrUnaryOp::CtzNonZero
                ) =>
        {
            Some((*op, *src))
        }
        _ => None,
    }
}
fn uses(cx: &Context<'_>, operand: Operand, expected: u32) -> bool {
    matches!(operand, Operand::Value(v) if cx.uses.get(v.0 as usize) == Some(&expected))
}
fn nonzero(op: IrUnaryOp) -> Option<IrUnaryOp> {
    match op {
        IrUnaryOp::Clz | IrUnaryOp::ClzNonZero => Some(IrUnaryOp::ClzNonZero),
        IrUnaryOp::Ctz | IrUnaryOp::CtzNonZero => Some(IrUnaryOp::CtzNonZero),
        _ => None,
    }
}
pub(super) fn fold(
    inst: &Instruction,
    cx: &Context<'_>,
    facts: &Facts,
    fresh: Value,
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
    if !scalar(*ty) || !uses(cx, *cond, 1) {
        return None;
    }
    let Instruction::Cmp {
        op,
        lhs,
        rhs,
        ty: cmp_ty,
        ..
    } = cx.def(*cond)?
    else {
        return None;
    };
    if cmp_ty != ty {
        return None;
    }
    let (low, high) = match op {
        IrCmpOp::Ult | IrCmpOp::Ule | IrCmpOp::Slt | IrCmpOp::Sle => (*lhs, *rhs),
        IrCmpOp::Ugt | IrCmpOp::Uge | IrCmpOp::Sgt | IrCmpOp::Sge => (*rhs, *lhs),
        _ => return None,
    };
    if *true_val != low || *false_val != high {
        return None;
    }
    for (a, b) in [(low, high), (high, low)] {
        let Some((op, x)) = count(cx, a, *ty) else {
            continue;
        };
        let nonzero_op = nonzero(op)?;
        // Both occurrences must belong to this comparison/select network.
        // A shared scan or comparison would make a second scan extra work.
        if !uses(cx, a, 2) {
            continue;
        }
        let width = ty.size() * 8;
        let (other, merged_op) = if let Some((other_op, y)) = count(cx, b, *ty) {
            if op != other_op || !uses(cx, b, 2) {
                continue;
            }
            let proven_nonzero = op == nonzero_op
                || cx.known(x, *ty, facts, 8).1 != 0
                || cx.known(y, *ty, facts, 8).1 != 0;
            (y, if proven_nonzero { nonzero_op } else { op })
        } else if let Some(cap) = const_in_ring(b, width) {
            // In particular reject negative signed constants and width-sized
            // shifts. Leading counts need width-1-cap, NOT width-cap.
            if cap >= width as u64 {
                continue;
            }
            let bit = if matches!(op, IrUnaryOp::Clz | IrUnaryOp::ClzNonZero) {
                width as u64 - 1 - cap
            } else {
                cap
            };
            (constant(1u64 << bit, *ty), nonzero_op)
        } else {
            continue;
        };
        return Some(vec![
            bin(fresh, IrBinOp::Or, x, other, *ty),
            Instruction::UnaryOp {
                dest: *dest,
                op: merged_op,
                src: Operand::Value(fresh),
                ty: *ty,
            },
        ]);
    }
    None
}

#[cfg(test)]
#[path = "bit_idioms_count_merge_tests.rs"]
mod tests;
