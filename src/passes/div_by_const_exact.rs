//! Exact signed power-of-two division: omit rounding bias only when the
//! dividend's low bits are provably zero. Facts describe modular word bits,
//! so they remain valid under -fwrapv, for negative inputs and INT_MIN.
use super::*;

fn width(ty: IrType) -> Option<u32> {
    match ty {
        IrType::I32 | IrType::U32 => Some(32),
        IrType::I64 | IrType::U64 => Some(64),
        _ => None,
    }
}

struct Facts<'a> {
    defs: Vec<Option<&'a Instruction>>,
    bits: u32,
    budget: u32,
}
impl Facts<'_> {
    fn view(&self, mut operand: Operand) -> Operand {
        for _ in 0..16 {
            let Operand::Value(v) = operand else { break };
            match self.defs.get(v.0 as usize).copied().flatten() {
                Some(Instruction::Copy { src, .. }) => operand = *src,
                Some(Instruction::Cast {
                    src,
                    from_ty,
                    to_ty,
                    ..
                }) if width(*from_ty) == Some(self.bits) && width(*to_ty) == Some(self.bits) => {
                    operand = *src
                }
                _ => break,
            }
        }
        operand
    }

    // Inlining precedes GVN in the first loop iteration. Identical coefficient
    // subexpressions can therefore have distinct SSA IDs. Compare only a
    // bounded, pure, equal-width modular DAG; never equate loads or calls.
    fn same(&self, a: Operand, b: Operand, budget: &mut u32) -> bool {
        if *budget == 0 {
            return false;
        }
        *budget -= 1;
        let (a, b) = (self.view(a), self.view(b));
        if a == b {
            return true;
        }
        let (Operand::Value(a), Operand::Value(b)) = (a, b) else {
            return false;
        };
        match (
            self.defs.get(a.0 as usize).copied().flatten(),
            self.defs.get(b.0 as usize).copied().flatten(),
        ) {
            (
                Some(Instruction::BinOp {
                    op: ao,
                    lhs: al,
                    rhs: ar,
                    ty: at,
                    ..
                }),
                Some(Instruction::BinOp {
                    op: bo,
                    lhs: bl,
                    rhs: br,
                    ty: bt,
                    ..
                }),
            ) if ao == bo
                && width(*at) == Some(self.bits)
                && width(*bt) == Some(self.bits)
                && matches!(
                    ao,
                    IrBinOp::Add
                        | IrBinOp::Sub
                        | IrBinOp::Mul
                        | IrBinOp::And
                        | IrBinOp::Or
                        | IrBinOp::Xor
                ) =>
            {
                (self.same(*al, *bl, budget) && self.same(*ar, *br, budget))
                    || (*ao != IrBinOp::Sub
                        && self.same(*al, *br, budget)
                        && self.same(*ar, *bl, budget))
            }
            _ => false,
        }
    }

    // Opposite-parity factors have an even product, including wrapping
    // products. This does NOT assume a signed product is nonnegative.
    fn opposite_parity(&self, a: Operand, b: Operand) -> bool {
        let a = self.view(a);
        let Operand::Value(b) = self.view(b) else {
            return false;
        };
        match self.defs.get(b.0 as usize).copied().flatten() {
            Some(Instruction::BinOp {
                op, lhs, rhs, ty, ..
            }) if width(*ty) == Some(self.bits) => {
                let odd = |v: Operand| matches!(v, Operand::Const(c) if c.to_i64().is_some_and(|n| n & 1 != 0));
                match op {
                    IrBinOp::Add | IrBinOp::Xor => {
                        (self.same(*lhs, a, &mut 32) && odd(*rhs))
                            || (self.same(*rhs, a, &mut 32) && odd(*lhs))
                    }
                    IrBinOp::Sub => self.same(*lhs, a, &mut 32) && odd(*rhs),
                    _ => false,
                }
            }
            Some(Instruction::UnaryOp {
                op: crate::ir::ops::IrUnaryOp::Not,
                src,
                ty,
                ..
            }) if width(*ty) == Some(self.bits) => self.same(*src, a, &mut 32),
            _ => false,
        }
    }

    fn zeros(&mut self, operand: Operand) -> u32 {
        if self.budget == 0 {
            return 0;
        }
        self.budget -= 1;
        let operand = self.view(operand);
        let Operand::Value(v) = operand else {
            return match operand {
                Operand::Const(c) => c
                    .to_i64()
                    .map_or(0, |n| (n as u64).trailing_zeros().min(self.bits)),
                _ => 0,
            };
        };
        let Some(inst) = self.defs.get(v.0 as usize).copied().flatten() else {
            return 0;
        };
        match inst {
            Instruction::UnaryOp {
                op: crate::ir::ops::IrUnaryOp::Neg,
                src,
                ty,
                ..
            } if width(*ty) == Some(self.bits) => return self.zeros(*src),
            Instruction::Select {
                true_val,
                false_val,
                ty,
                ..
            } if width(*ty) == Some(self.bits) => {
                let (tv, fv) = (*true_val, *false_val);
                return self.zeros(tv).min(self.zeros(fv));
            }
            _ => {}
        }
        let Instruction::BinOp {
            op, lhs, rhs, ty, ..
        } = inst
        else {
            return 0;
        };
        if width(*ty) != Some(self.bits) {
            return 0;
        }
        let (op, lhs, rhs) = (*op, *lhs, *rhs);
        if matches!(op, IrBinOp::Shl | IrBinOp::LShr | IrBinOp::AShr) {
            let Operand::Const(c) = rhs else { return 0 };
            let Some(n) = c.to_i64().filter(|&n| n >= 0 && n < i64::from(self.bits)) else {
                return 0;
            };
            let z = self.zeros(lhs);
            return if op == IrBinOp::Shl {
                (z + n as u32).min(self.bits)
            } else {
                z.saturating_sub(n as u32)
            };
        }
        if !matches!(
            op,
            IrBinOp::Add | IrBinOp::Sub | IrBinOp::Mul | IrBinOp::And | IrBinOp::Or | IrBinOp::Xor
        ) {
            return 0;
        }
        let a = self.zeros(lhs);
        let b = self.zeros(rhs);
        match op {
            IrBinOp::And => a.max(b),
            IrBinOp::Mul => (a + b).min(self.bits).max(u32::from(
                self.opposite_parity(lhs, rhs) || self.opposite_parity(rhs, lhs),
            )),
            _ => a.min(b),
        }
    }
}

fn candidate(i: &Instruction) -> Option<(Value, IrBinOp, Operand, IrType, u32)> {
    let Instruction::BinOp {
        dest,
        op,
        lhs,
        rhs: Operand::Const(c),
        ty,
        ..
    } = i
    else {
        return None;
    };
    if !matches!(ty, IrType::I32 | IrType::I64) || !matches!(op, IrBinOp::SDiv | IrBinOp::SRem) {
        return None;
    }
    let n = c.to_i64()?;
    if n <= 1 || !(n as u64).is_power_of_two() {
        return None;
    }
    let shift = n.trailing_zeros();
    (shift < width(*ty)? - 1).then_some((*dest, *op, *lhs, *ty, shift))
}

pub(super) fn run(func: &mut IrFunction) -> usize {
    if !func
        .blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .any(|i| candidate(i).is_some())
    {
        return 0;
    }
    let mut facts = Facts {
        defs: vec![None; func.max_value_id() as usize + 1],
        bits: 32,
        budget: 0,
    };
    for i in func.blocks.iter().flat_map(|b| &b.instructions) {
        if let Some(d) = i.dest() {
            if facts.defs[d.0 as usize].replace(i).is_some() {
                return 0;
            }
        }
    }
    let mut plans = Vec::new();
    for (bi, b) in func.blocks.iter().enumerate() {
        for (ii, i) in b.instructions.iter().enumerate() {
            let Some((dest, op, lhs, ty, shift)) = candidate(i) else {
                continue;
            };
            let Some(bits) = width(ty) else {
                continue;
            };
            facts.bits = bits;
            facts.budget = 64;
            if facts.zeros(lhs) < shift {
                continue;
            }
            let replacement = if op == IrBinOp::SDiv {
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::AShr,
                    lhs,
                    rhs: Operand::Const(IrConst::from_i64(i64::from(shift), ty)),
                    ty,
                }
            } else {
                Instruction::Copy {
                    dest,
                    src: Operand::Const(IrConst::from_i64(0, ty)),
                }
            };
            plans.push((bi, ii, replacement));
        }
    }
    let count = plans.len();
    for (bi, ii, i) in plans {
        func.blocks[bi].instructions[ii] = i;
    }
    count
}

#[cfg(test)]
#[path = "div_by_const_exact_tests.rs"]
mod tests;
