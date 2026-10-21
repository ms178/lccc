//! Wrapping complement arithmetic: (x ^ C) + 1 + y = y - (x ^ ~C).
//! Require the two replaced producers to be private and exactly same-width;
//! no no-wrap, range, signed-overflow or source-level UB inference is needed.
//! Complemented comparisons reverse both operand orders, including signed ones.
use super::*;

pub(super) fn fold(inst: &Instruction, cx: &Context<'_>, fresh: Value) -> Option<Vec<Instruction>> {
    if let Instruction::Cmp {
        dest,
        op,
        lhs,
        rhs,
        ty,
    } = inst
    {
        if !scalar(*ty) {
            return None;
        }
        let inverted = |operand| match cx.def(operand)? {
            Instruction::UnaryOp {
                op: IrUnaryOp::Not,
                src,
                ty: word,
                ..
            } if word == ty
                && matches!(operand, Operand::Value(v) if cx.uses.get(v.0 as usize) == Some(&1)) =>
            {
                Some(*src)
            }
            _ => None,
        };
        return Some(vec![Instruction::Cmp {
            dest: *dest,
            op: *op,
            lhs: inverted(*rhs)?,
            rhs: inverted(*lhs)?,
            ty: *ty,
        }]);
    }
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
    if !scalar(*ty) || !matches!(op, IrBinOp::Add | IrBinOp::Sub) {
        return None;
    }
    let private = |x| matches!(x, Operand::Value(v) if cx.uses.get(v.0 as usize) == Some(&1));
    for (offset, other) in [(*rhs, *lhs), (*lhs, *rhs)] {
        if *op == IrBinOp::Sub && offset != *rhs {
            continue;
        }
        let Some((xor, 1)) = cx.with_const(offset, IrBinOp::Add, *ty) else {
            continue;
        };
        let Some((x, c)) = cx.with_const(xor, IrBinOp::Xor, *ty) else {
            continue;
        };
        if !private(offset) || !private(xor) {
            continue;
        }
        return Some(vec![
            bin(fresh, IrBinOp::Xor, x, constant(!c, *ty), *ty),
            bin(
                *dest,
                if *op == IrBinOp::Add {
                    IrBinOp::Sub
                } else {
                    IrBinOp::Add
                },
                other,
                Operand::Value(fresh),
                *ty,
            ),
        ]);
    }
    None
}

#[cfg(test)]
#[path = "bit_idioms_complements_tests.rs"]
mod tests;
