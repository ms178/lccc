//! Small, exact Boolean cardinality synthesis, not an SMT solver in the compiler.
//!
//! For at most two bit-vector leaves, each Boolean expression has four per-bit
//! valuations. A private arithmetic tree of population counts is a linear sum
//! of those tables. If the resulting table is Boolean (or twice Boolean), one
//! population count suffices. The same identity holds after summing over every
//! bit. Reject negative/mixed coefficients at the output, mixed word widths,
//! shared counts/arithmetic, oversized coefficients, and exhausted walk budgets.
use super::*;

pub(super) enum Plan {
    Constant(i64),
    Count {
        input: Expr,
        word: IrType,
        factor: u32,
    },
}
struct Tables<'a> {
    defs: &'a [Option<Instruction>],
    uses: &'a [u32],
    bases: Vec<Operand>,
    word: Option<IrType>,
    terms: u32,
    arithmetic_budget: u32,
    bit_budget: u32,
}
impl Tables<'_> {
    fn bits(&mut self, v: Operand, ty: IrType) -> Option<u8> {
        self.bit_budget = self.bit_budget.checked_sub(1)?;
        if let Some(c) = const_in_ring(v, ty.size() * 8) {
            if c == 0 {
                return Some(0);
            }
            if c == full(ty)? {
                return Some(15);
            }
        }
        match definition(v, self.defs) {
            Some(Instruction::UnaryOp {
                op: IrUnaryOp::Not,
                src,
                ty: t,
                ..
            }) if compatible(*t, ty) => {
                let src = *src;
                Some(self.bits(src, ty)? ^ 15)
            }
            Some(Instruction::BinOp {
                op,
                lhs,
                rhs,
                ty: t,
                ..
            }) if compatible(*t, ty) && matches!(op, IrBinOp::And | IrBinOp::Or | IrBinOp::Xor) => {
                let (op, lhs, rhs) = (*op, *lhs, *rhs);
                let a = self.bits(lhs, ty)?;
                let b = self.bits(rhs, ty)?;
                Some(match op {
                    IrBinOp::And => a & b,
                    IrBinOp::Or => a | b,
                    _ => a ^ b,
                })
            }
            _ => {
                let index =
                    if let Some(i) = self.bases.iter().position(|x| same_bits(*x, v, self.defs)) {
                        i
                    } else {
                        if self.bases.len() == 2 {
                            return None;
                        }
                        self.bases.push(v);
                        self.bases.len() - 1
                    };
                Some(if index == 0 { 10 } else { 12 })
            }
        }
    }
    fn count(&mut self, v: Operand, coefficient: i32, expected: u32) -> Option<[i32; 4]> {
        if coefficient.abs() > 16 {
            return None;
        }
        if let Some((input, word)) = count_input(v, self.defs) {
            if !private_count_with_uses(v, self.defs, self.uses, expected) {
                return None;
            }
            if self.word.is_some_and(|ty| !compatible(ty, word)) {
                return None;
            }
            self.word = Some(word);
            self.terms += 1;
            let table = self.bits(input, word)?;
            return Some(std::array::from_fn(|i| {
                if table & (1 << i) != 0 {
                    coefficient
                } else {
                    0
                }
            }));
        }
        None
    }
    fn arithmetic(&mut self, v: Operand, coefficient: i32, result: IrType) -> Option<[i32; 4]> {
        self.arithmetic_budget = self.arithmetic_budget.checked_sub(1)?;
        if coefficient.abs() > 16 {
            return None;
        }
        if let Some(table) = self.count(v, coefficient, 1) {
            return Some(table);
        }
        exclusive(v, self.defs, self.uses)?;
        match definition(v, self.defs) {
            Some(Instruction::BinOp {
                op, lhs, rhs, ty, ..
            }) if compatible(*ty, result) => {
                let (op, lhs, rhs) = (*op, *lhs, *rhs);
                match op {
                    IrBinOp::Add | IrBinOp::Sub => {
                        // Strength reduction turns 2*count into count+count.
                        // These two edges must be ALL uses of the count/cast;
                        // consume it once with coefficient two, not as two
                        // apparently-private producers.
                        if op == IrBinOp::Add && lhs == rhs {
                            return self.count(lhs, coefficient * 2, 2);
                        }
                        let a = self.arithmetic(lhs, coefficient, result)?;
                        let b = self.arithmetic(
                            rhs,
                            if op == IrBinOp::Add {
                                coefficient
                            } else {
                                -coefficient
                            },
                            result,
                        )?;
                        Some(std::array::from_fn(|i| a[i] + b[i]))
                    }
                    IrBinOp::Mul => {
                        for (input, c) in [(lhs, rhs), (rhs, lhs)] {
                            if let Some(factor @ (2 | 4)) = const_in_ring(c, result.size() * 8) {
                                return self.arithmetic(input, coefficient * factor as i32, result);
                            }
                        }
                        None
                    }
                    IrBinOp::Shl if const_in_ring(rhs, result.size() * 8) == Some(1) => {
                        self.arithmetic(lhs, coefficient * 2, result)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }
}
fn input(table: u8, bases: &[Operand]) -> Option<Expr> {
    let x = Expr::Leaf(*bases.first()?);
    let y = Expr::Leaf(bases.get(1).copied().unwrap_or(bases[0]));
    let not = |a| Expr::Unary(IrUnaryOp::Not, Box::new(a));
    let bin = |op, a, b| Expr::Bin(op, Box::new(a), Box::new(b));
    Some(match table {
        1 => not(bin(IrBinOp::Or, x, y)),
        2 => bin(IrBinOp::And, x, not(y)),
        3 => not(y),
        4 => bin(IrBinOp::And, not(x), y),
        5 => not(x),
        6 => bin(IrBinOp::Xor, x, y),
        7 => not(bin(IrBinOp::And, x, y)),
        8 => bin(IrBinOp::And, x, y),
        9 => not(bin(IrBinOp::Xor, x, y)),
        10 => x,
        11 => bin(IrBinOp::Or, x, not(y)),
        12 => y,
        13 => bin(IrBinOp::Or, not(x), y),
        14 => bin(IrBinOp::Or, x, y),
        _ => return None,
    })
}
pub(super) fn plan(
    op: IrBinOp,
    lhs: Operand,
    rhs: Operand,
    result: IrType,
    defs: &[Option<Instruction>],
    uses: &[u32],
) -> Option<Plan> {
    let mut state = Tables {
        defs,
        uses,
        bases: vec![],
        word: None,
        terms: 0,
        arithmetic_budget: 32,
        bit_budget: 64,
    };
    let a = state.arithmetic(lhs, 1, result)?;
    let b = state.arithmetic(rhs, if op == IrBinOp::Add { 1 } else { -1 }, result)?;
    if state.terms < 2 {
        return None;
    } // Never trade one count for one count plus arithmetic.
    let values: [i32; 4] = std::array::from_fn(|i| a[i] + b[i]);
    let word = state.word?;
    if values.iter().all(|x| *x == values[0]) {
        return Some(Plan::Constant(values[0] as i64 * word.size() as i64 * 8));
    }
    let factor = *values.iter().find(|x| **x > 0)?;
    if !matches!(factor, 1 | 2) || values.iter().any(|x| *x != 0 && *x != factor) {
        return None;
    }
    let table = values
        .iter()
        .enumerate()
        .fold(0, |m, (i, x)| m | if *x == factor { 1 << i } else { 0 });
    Some(Plan::Count {
        input: input(table, &state.bases)?,
        word,
        factor: factor as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_fourteen_nonconstant_boolean_tables_synthesize_exactly() {
        let bases = [Operand::Value(Value(0)), Operand::Value(Value(1))];
        for table in 1..15 {
            let expr = input(table, &bases).unwrap();
            let mut next = 2;
            let mut defs = vec![None, None];
            let mut code = vec![];
            let root = emit(expr, IrType::U64, &mut next, &mut code);
            defs.resize(next as usize, None);
            for i in code {
                let d = i.dest().unwrap();
                defs[d.0 as usize] = Some(i);
            }
            let mut state = Tables {
                defs: &defs,
                uses: &[],
                bases: vec![],
                word: None,
                terms: 0,
                arithmetic_budget: 32,
                bit_budget: 64,
            };
            // Synthesis can encounter y before x; pin the external basis order.
            state.bases = bases.to_vec();
            assert_eq!(state.bits(root, IrType::U64), Some(table));
        }
    }
}
