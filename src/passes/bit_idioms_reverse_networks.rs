//! Conjugate a private bitwise network through bit reversal. Reversal is
//! a permutation of a fixed-width ring, not a homomorphism of addition.
use super::*;
struct Builder<'a, 'b> {
    cx: &'a Context<'b>,
    ty: IrType,
    next: u32,
    code: Vec<Instruction>,
    budget: u8,
}
impl Builder<'_, '_> {
    fn emit(&mut self, op: IrBinOp, a: Operand, b: Operand) -> Option<Operand> {
        let dest = Value(self.next);
        self.next = self.next.checked_add(1)?;
        self.code.push(bin(dest, op, a, b, self.ty));
        Some(Operand::Value(dest))
    }
    fn visit(&mut self, value: Operand) -> Option<Operand> {
        self.budget = self.budget.checked_sub(1)?;
        let width = self.ty.size() * 8;
        if let Some(n) = const_in_ring(value, width) {
            return Some(constant(n.reverse_bits() >> (64 - width), self.ty));
        }
        let inst = self.cx.def(value)?.clone();
        if let Instruction::UnaryOp {
            op: IrUnaryOp::BitReverse,
            src,
            ty,
            ..
        } = inst
            && ty == self.ty
        {
            return Some(src);
        }
        let Operand::Value(id) = value else {
            return None;
        };
        if self.cx.uses.get(id.0 as usize) != Some(&1) {
            return None;
        }
        match inst {
            Instruction::Copy { src, .. } => self.visit(src),
            Instruction::UnaryOp {
                op: IrUnaryOp::Not,
                src,
                ty,
                ..
            } if ty == self.ty => {
                let src = self.visit(src)?;
                let dest = Value(self.next);
                self.next = self.next.checked_add(1)?;
                self.code.push(Instruction::UnaryOp {
                    dest,
                    op: IrUnaryOp::Not,
                    src,
                    ty,
                });
                Some(Operand::Value(dest))
            }
            Instruction::BinOp {
                op: op @ (IrBinOp::And | IrBinOp::Or | IrBinOp::Xor),
                lhs,
                rhs,
                ty,
                ..
            } if ty == self.ty => {
                let a = self.visit(lhs)?;
                let b = self.visit(rhs)?;
                self.emit(op, a, b)
            }
            Instruction::BinOp {
                op, lhs, rhs, ty, ..
            } if ty == self.ty && matches!(op, IrBinOp::Shl | IrBinOp::LShr) => {
                fixed_shift_count(rhs, width)?;
                let a = self.visit(lhs)?;
                self.emit(
                    if op == IrBinOp::Shl {
                        IrBinOp::LShr
                    } else {
                        IrBinOp::Shl
                    },
                    a,
                    rhs,
                )
            }
            _ => None,
        }
    }
}
pub(super) fn fold(inst: &Instruction, cx: &Context<'_>, fresh: Value) -> Option<Vec<Instruction>> {
    let Instruction::UnaryOp {
        dest,
        op: IrUnaryOp::BitReverse,
        src,
        ty,
    } = inst
    else {
        return None;
    };
    if !scalar(*ty) {
        return None;
    }
    let mut build = Builder {
        cx,
        ty: *ty,
        next: fresh.0,
        code: vec![],
        budget: 24,
    };
    let result = build.visit(*src)?;
    if let Some(last) = build.code.last_mut() {
        match last {
            Instruction::BinOp { dest: last, .. } | Instruction::UnaryOp { dest: last, .. }
                if result == Operand::Value(*last) =>
            {
                *last = *dest
            }
            _ => return None,
        }
        Some(build.code)
    } else {
        Some(copy(*dest, result))
    }
}
#[cfg(test)]
#[path = "bit_idioms_reverse_networks_tests.rs"]
mod tests;
