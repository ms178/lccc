//! Exact per-bit synthesis of small, private scalar Boolean networks.
//!
//! Three opaque word operands have eight per-bit valuations. AND/OR/XOR/NOT
//! operate independently on every bit, so equality of those tables proves the
//! rewrite for every supported word width, including arbitrary constant masks.
//! Only closed producer DAGs pay for replacements; external users refuse a fold.
//! Recipes have at most three operations and must strictly reduce the private
//! operation count. No loads, shifts, arithmetic, casts or comparisons are moved.
use super::*;

#[derive(Clone, Copy)]
enum Expr {
    Leaf(usize),
    Constant(bool),
    Not(u8),
    Binary(IrBinOp, u8, u8),
}
#[derive(Clone, Copy)]
struct Recipe {
    cost: u8,
    expr: Expr,
}
fn apply(op: IrBinOp, a: u8, b: u8) -> Option<u8> {
    match op {
        IrBinOp::And => Some(a & b),
        IrBinOp::Or => Some(a | b),
        IrBinOp::Xor => Some(a ^ b),
        _ => None,
    }
}
fn recipes() -> &'static [Option<Recipe>; 256] {
    static TABLE: std::sync::OnceLock<[Option<Recipe>; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [None; 256];
        for (bits, expr) in [
            (0, Expr::Constant(false)),
            (255, Expr::Constant(true)),
            (170, Expr::Leaf(0)),
            (204, Expr::Leaf(1)),
            (240, Expr::Leaf(2)),
        ] {
            table[bits] = Some(Recipe { cost: 0, expr });
        }
        for cost in 1..=3 {
            // Children always cost strictly less than their parent. Snapshot
            // each layer so iteration order cannot affect the search space.
            let previous = table;
            for (a, left) in previous.iter().enumerate() {
                let Some(left) = left else { continue };
                if left.cost + 1 == cost && table[255 ^ a].is_none() {
                    table[255 ^ a] = Some(Recipe {
                        cost,
                        expr: Expr::Not(a as u8),
                    });
                }
                for (b, right) in previous.iter().enumerate().skip(a) {
                    let Some(right) = right else { continue };
                    if left.cost + right.cost + 1 != cost {
                        continue;
                    }
                    for op in [IrBinOp::And, IrBinOp::Or, IrBinOp::Xor] {
                        let Some(bits) = apply(op, a as u8, b as u8) else {
                            continue;
                        };
                        if table[bits as usize].is_none() {
                            table[bits as usize] = Some(Recipe {
                                cost,
                                expr: Expr::Binary(op, a as u8, b as u8),
                            });
                        }
                    }
                }
            }
        }
        table
    })
}
// The established mux/distributive matcher owns these shapes. In particular,
// CH can fuse to ANDN and a distributive prefix can belong to a majority tree.
// Treat it as an opaque value, not a cheaper target-neutral Boolean network.
fn specialized_and_pair(inst: &Instruction, cx: &Context<'_>, ty: IrType) -> bool {
    let Instruction::BinOp {
        op: IrBinOp::Or | IrBinOp::Xor,
        lhs,
        rhs,
        ..
    } = inst
    else {
        return false;
    };
    let Some((a, b)) = cx.binary(*lhs, IrBinOp::And, ty) else {
        return false;
    };
    let Some((c, d)) = cx.binary(*rhs, IrBinOp::And, ty) else {
        return false;
    };
    for x in [a, b] {
        for y in [c, d] {
            if x == y {
                return true;
            }
            for (inverted, plain) in [(x, y), (y, x)] {
                if matches!(cx.def(inverted), Some(Instruction::UnaryOp { op: IrUnaryOp::Not, src, ty: word, .. }) if *word == ty && *src == plain)
                {
                    return true;
                }
            }
        }
    }
    false
}

struct Network<'a> {
    cx: &'a Context<'a>,
    leaves: Vec<Operand>,
    ty: IrType,
    remaining: u8,
    operations: u8,
    nodes: Vec<(Operand, u8, u32)>,
}
impl Network<'_> {
    fn table(&mut self, operand: Operand) -> Option<u8> {
        self.remaining = self.remaining.checked_sub(1)?;
        if let Some(n) = const_in_ring(operand, self.ty.size() * 8) {
            if n == 0 {
                return Some(0);
            }
            if n == u64::MAX >> (64 - self.ty.size() * 8) {
                return Some(255);
            }
        }
        if let Some((_, bits, seen)) = self.nodes.iter_mut().find(|(node, _, _)| *node == operand) {
            *seen += 1;
            return Some(*bits);
        }
        if let Some(def) = self
            .cx
            .def(operand)
            .filter(|inst| !specialized_and_pair(inst, self.cx, self.ty))
        {
            match def {
                Instruction::BinOp {
                    op, lhs, rhs, ty, ..
                } if *ty == self.ty && matches!(op, IrBinOp::And | IrBinOp::Or | IrBinOp::Xor) => {
                    let (op, lhs, rhs) = (*op, *lhs, *rhs);
                    self.operations += 1;
                    let a = self.table(lhs)?;
                    let b = self.table(rhs)?;
                    let bits = apply(op, a, b)?;
                    self.nodes.push((operand, bits, 1));
                    return Some(bits);
                }
                Instruction::UnaryOp {
                    op: IrUnaryOp::Not,
                    src,
                    ty,
                    ..
                } if *ty == self.ty => {
                    let src = *src;
                    self.operations += 1;
                    let bits = !self.table(src)?;
                    self.nodes.push((operand, bits, 1));
                    return Some(bits);
                }
                _ => {}
            }
        }
        for (i, leaf) in self.leaves.iter().enumerate() {
            let table = [170u8, 204, 240].get(i).copied()?;
            if *leaf == operand {
                return Some(table);
            }
            if let (Some(a), Some(b)) = (
                const_in_ring(*leaf, self.ty.size() * 8),
                const_in_ring(operand, self.ty.size() * 8),
            ) && a ^ b == u64::MAX >> (64 - self.ty.size() * 8)
            {
                return Some(!table);
            }
        }
        let table = [170, 204, 240].get(self.leaves.len()).copied()?;
        self.leaves.push(operand);
        Some(table)
    }
}
fn emit(
    bits: u8,
    leaves: &[Operand],
    ty: IrType,
    fresh: Value,
    out: &mut Vec<Instruction>,
) -> Option<Operand> {
    let recipe = recipes()[bits as usize]?;
    let (op, lhs, rhs) = match recipe.expr {
        Expr::Leaf(i) => return leaves.get(i).copied(),
        Expr::Constant(set) => return Some(constant(if set { u64::MAX } else { 0 }, ty)),
        Expr::Not(a) => (None, emit(a, leaves, ty, fresh, out)?, None),
        Expr::Binary(op, a, b) => (
            Some(op),
            emit(a, leaves, ty, fresh, out)?,
            Some(emit(b, leaves, ty, fresh, out)?),
        ),
    };
    let dest = Value(fresh.0.checked_add(out.len() as u32)?);
    out.push(match (op, rhs) {
        (Some(op), Some(rhs)) => bin(dest, op, lhs, rhs, ty),
        _ => Instruction::UnaryOp {
            dest,
            op: IrUnaryOp::Not,
            src: lhs,
            ty,
        },
    });
    Some(Operand::Value(dest))
}
pub(super) fn fold(inst: &Instruction, cx: &Context<'_>, fresh: Value) -> Option<Vec<Instruction>> {
    if cx.disable_logic {
        return None;
    }
    let (dest, ty) = match inst {
        Instruction::BinOp {
            dest,
            ty,
            op: IrBinOp::And | IrBinOp::Or | IrBinOp::Xor,
            ..
        }
        | Instruction::UnaryOp {
            dest,
            ty,
            op: IrUnaryOp::Not,
            ..
        } if scalar(*ty) => (*dest, *ty),
        _ => return None,
    };
    let mut network = Network {
        cx,
        leaves: Vec::new(),
        ty,
        remaining: 32,
        operations: 0,
        nodes: Vec::new(),
    };
    let bits = network.table(Operand::Value(dest))?;
    for &(node, _, internal_uses) in &network.nodes {
        if node != Operand::Value(dest)
            && !matches!(node, Operand::Value(v) if cx.uses.get(v.0 as usize) == Some(&internal_uses))
        {
            return None;
        }
    }
    let recipe = recipes()[bits as usize]?;
    if recipe.cost >= network.operations {
        return None;
    }
    let mut out = Vec::new();
    let result = emit(bits, &network.leaves, ty, fresh, &mut out)?;
    match out.last_mut() {
        Some(Instruction::BinOp { dest: last, .. } | Instruction::UnaryOp { dest: last, .. }) => {
            *last = dest
        }
        _ => return Some(copy(dest, result)),
    }
    Some(out)
}

#[cfg(test)]
#[path = "bit_idioms_logic_tests.rs"]
mod tests;
