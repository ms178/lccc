//! Consumer-local conditional substitution. Never mutate a shared producer,
//! clone an operation, speculate memory, or assume LLVM poison/no-wrap flags.
use super::*;

// Compare predicates only over identical scalar operands and carrier types.
// Equality/non-equality and signed/unsigned complements remain distinct.
fn relation(a: Operand, b: Operand, cx: &Context<'_>) -> Option<bool> {
    if a == b {
        return Some(true);
    }
    let (
        Some(Instruction::Cmp {
            op: x,
            lhs: al,
            rhs: ar,
            ty: at,
            ..
        }),
        Some(Instruction::Cmp {
            op: y,
            lhs: bl,
            rhs: br,
            ty: bt,
            ..
        }),
    ) = (cx.def(a), cx.def(b))
    else {
        return None;
    };
    if !scalar(*at) || at != bt {
        return None;
    }
    let y = if al == bl && ar == br {
        *y
    } else if al == br && ar == bl {
        match y {
            IrCmpOp::Eq => IrCmpOp::Eq,
            IrCmpOp::Ne => IrCmpOp::Ne,
            IrCmpOp::Ult => IrCmpOp::Ugt,
            IrCmpOp::Ule => IrCmpOp::Uge,
            IrCmpOp::Ugt => IrCmpOp::Ult,
            IrCmpOp::Uge => IrCmpOp::Ule,
            IrCmpOp::Slt => IrCmpOp::Sgt,
            IrCmpOp::Sle => IrCmpOp::Sge,
            IrCmpOp::Sgt => IrCmpOp::Slt,
            IrCmpOp::Sge => IrCmpOp::Sle,
        }
    } else {
        return None;
    };
    if *x == y {
        return Some(true);
    }
    let inverse = match x {
        IrCmpOp::Eq => IrCmpOp::Ne,
        IrCmpOp::Ne => IrCmpOp::Eq,
        IrCmpOp::Ult => IrCmpOp::Uge,
        IrCmpOp::Ule => IrCmpOp::Ugt,
        IrCmpOp::Ugt => IrCmpOp::Ule,
        IrCmpOp::Uge => IrCmpOp::Ult,
        IrCmpOp::Slt => IrCmpOp::Sge,
        IrCmpOp::Sle => IrCmpOp::Sgt,
        IrCmpOp::Sgt => IrCmpOp::Sle,
        IrCmpOp::Sge => IrCmpOp::Slt,
    };
    (inverse == y).then_some(false)
}
fn arm(
    op: Operand,
    ty: IrType,
    cond: Operand,
    yes: bool,
    facts: &Facts,
    cx: &Context<'_>,
    depth: u8,
) -> Operand {
    if depth == 0 {
        return op;
    }
    if cx.boolean(op).is_some()
        && let Some(same) = relation(op, cond, cx)
    {
        return constant(u64::from(yes == same), ty);
    }
    match cx.def(op) {
        Some(Instruction::Copy { src, .. }) => arm(*src, ty, cond, yes, facts, cx, depth - 1),
        Some(Instruction::Cast {
            src,
            from_ty,
            to_ty,
            ..
        }) if *to_ty == ty && scalar(*from_ty) && cx.boolean(*src).is_some() => {
            if let Some(n) = const_in_ring(
                arm(*src, *from_ty, cond, yes, facts, cx, depth - 1),
                from_ty.size() * 8,
            ) {
                return constant(n, ty);
            }
            op
        }
        Some(Instruction::Select {
            cond: inner,
            true_val,
            false_val,
            ty: word,
            ..
        }) if *word == ty => {
            if let Some(same) = relation(*inner, cond, cx) {
                arm(
                    if same == yes { *true_val } else { *false_val },
                    ty,
                    cond,
                    yes,
                    facts,
                    cx,
                    depth - 1,
                )
            } else {
                op
            }
        }
        Some(Instruction::BinOp {
            op: IrBinOp::And,
            lhs,
            rhs,
            ty: word,
            ..
        }) if *word == ty => {
            let mask = u64::MAX >> (64 - ty.size() * 8);
            for (base, c) in [(*lhs, *rhs), (*rhs, *lhs)] {
                if let Some(c) = const_in_ring(c, ty.size() * 8) {
                    let (zero, _) = cx.known(base, ty, facts, 8);
                    if zero & (!c & mask) == !c & mask {
                        return base;
                    }
                }
            }
            cx.simplify_operand(op, ty, facts)
        }
        _ => cx.simplify_operand(op, ty, facts),
    }
}
fn selected(op: Operand, ty: IrType, cx: &Context<'_>) -> Option<(Operand, Operand, Operand)> {
    match cx.def(op)? {
        Instruction::Select {
            cond,
            true_val,
            false_val,
            ty: word,
            ..
        } if *word == ty => Some((*cond, *true_val, *false_val)),
        _ => None,
    }
}
pub(super) fn fold(
    inst: &Instruction,
    cx: &Context<'_>,
    facts: &Facts,
    fresh: Value,
) -> Option<Vec<Instruction>> {
    match inst {
        Instruction::Select {
            dest,
            cond,
            true_val,
            false_val,
            ty,
        } if scalar(*ty) => {
            // A select of opposite private selects is a Boolean parity
            // choice. Normalize both predicates to proven I32 Booleans;
            // arbitrary nonzero words cannot be XORed as truth values.
            if let (Some((ca, a, b)), Some((cb, c, d))) =
                (selected(*true_val, *ty, cx), selected(*false_val, *ty, cx))
                && relation(ca, cb, cx) == Some(true)
                && a == d
                && b == c
                && matches!(true_val,Operand::Value(v) if cx.uses.get(v.0 as usize)==Some(&1))
                && matches!(false_val,Operand::Value(v) if cx.uses.get(v.0 as usize)==Some(&1))
                && let (Some(ca), Some(outer)) = (cx.boolean(ca), cx.boolean(*cond))
            {
                return Some(vec![
                    bin(fresh, IrBinOp::Xor, ca, outer, IrType::I32),
                    Instruction::Select {
                        dest: *dest,
                        cond: Operand::Value(fresh),
                        true_val: b,
                        false_val: a,
                        ty: *ty,
                    },
                ]);
            }
            let mut yes = Facts(facts.0.clone());
            let mut no = Facts(facts.0.clone());
            cx.facts(*cond, true, &mut yes, 8);
            cx.facts(*cond, false, &mut no, 8);
            let a = arm(*true_val, *ty, *cond, true, &yes, cx, 8);
            let b = arm(*false_val, *ty, *cond, false, &no, cx, 8);
            if a == *true_val && b == *false_val {
                return None;
            }
            if a == b {
                return Some(copy(*dest, a));
            }
            Some(vec![Instruction::Select {
                dest: *dest,
                cond: *cond,
                true_val: a,
                false_val: b,
                ty: *ty,
            }])
        }
        Instruction::BinOp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        } if scalar(*ty) => {
            if *op == IrBinOp::Sub
                && let Some(c) = const_in_ring(*lhs, ty.size() * 8)
            {
                let mask = u64::MAX >> (64 - ty.size() * 8);
                if c != 0
                    && c != mask
                    && c & c.wrapping_add(1) == 0
                    && cx.known(*rhs, *ty, facts, 8).0 & (!c & mask) == !c & mask
                {
                    return Some(vec![bin(*dest, IrBinOp::Xor, *rhs, *lhs, *ty)]);
                }
            }
            if !matches!(
                op,
                IrBinOp::Add | IrBinOp::Mul | IrBinOp::And | IrBinOp::Or | IrBinOp::Xor
            ) {
                return None;
            }
            // Fold a binop into a constant-choice select only when BOTH
            // outcomes are constants. No variable operation is cloned.
            if let Some((cond, _, _)) = selected(*lhs, *ty, cx).or_else(|| selected(*rhs, *ty, cx))
            {
                let outcomes = [true, false].map(|yes| {
                    let local = Facts(vec![]);
                    let a = const_in_ring(arm(*lhs, *ty, cond, yes, &local, cx, 8), ty.size() * 8)?;
                    let b = const_in_ring(arm(*rhs, *ty, cond, yes, &local, cx, 8), ty.size() * 8)?;
                    Some(match op {
                        IrBinOp::Add => a.wrapping_add(b),
                        IrBinOp::Mul => a.wrapping_mul(b),
                        IrBinOp::And => a & b,
                        IrBinOp::Or => a | b,
                        IrBinOp::Xor => a ^ b,
                        _ => return None,
                    })
                });
                if let [Some(a), Some(b)] = outcomes {
                    if a == b {
                        return Some(copy(*dest, constant(a, *ty)));
                    }
                    return Some(vec![Instruction::Select {
                        dest: *dest,
                        cond,
                        true_val: constant(a, *ty),
                        false_val: constant(b, *ty),
                        ty: *ty,
                    }]);
                }
            }
            let (ca, a, b) = selected(*lhs, *ty, cx)?;
            let (cb, c, d) = selected(*rhs, *ty, cx)?;
            let (c, d) = if relation(ca, cb, cx)? {
                (c, d)
            } else {
                (d, c)
            };
            if a == d && b == c {
                return Some(vec![bin(*dest, *op, a, b, *ty)]);
            }
            None
        }
        Instruction::Cmp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        } if scalar(*ty) => {
            // Contiguous masks, not arbitrary sparse masks: replace one bit
            // operation and a comparison with a single unsigned comparison.
            if matches!(op, IrCmpOp::Eq | IrCmpOp::Ne) {
                for (masked, base) in [(*lhs, *rhs), (*rhs, *lhs)] {
                    for kind in [IrBinOp::Or, IrBinOp::And] {
                        if let Some((x, c)) = cx.with_const(masked, kind, *ty)
                            && x == base
                        {
                            let full = u64::MAX >> (64 - ty.size() * 8);
                            let low = if kind == IrBinOp::Or { !c & full } else { c };
                            if low != full && low & low.wrapping_add(1) == 0 {
                                let pred = match (kind, op) {
                                    (IrBinOp::Or, IrCmpOp::Eq) => IrCmpOp::Uge,
                                    (IrBinOp::Or, _) => IrCmpOp::Ult,
                                    (_, IrCmpOp::Eq) => IrCmpOp::Ule,
                                    _ => IrCmpOp::Ugt,
                                };
                                return Some(vec![Instruction::Cmp {
                                    dest: *dest,
                                    op: pred,
                                    lhs: base,
                                    rhs: constant(c, *ty),
                                    ty: *ty,
                                }]);
                            }
                        }
                    }
                }
            }
            // Comparing correlated selects does not require private arms:
            // shared values remain in place, and no producer is cloned.
            let (ca, a, b) = selected(*lhs, *ty, cx)?;
            let (cb, c, d) = selected(*rhs, *ty, cx)?;
            let (c, d) = if relation(ca, cb, cx)? {
                (c, d)
            } else {
                (d, c)
            };
            if matches!(op, IrCmpOp::Eq | IrCmpOp::Ne) && a == d && b == c {
                return Some(vec![Instruction::Cmp {
                    dest: *dest,
                    op: *op,
                    lhs: a,
                    rhs: b,
                    ty: *ty,
                }]);
            }
            let evaluate = |a, b| {
                if a == b {
                    return Some(compare(*op, 0, 0, ty.size() * 8));
                }
                Some(compare(
                    *op,
                    const_in_ring(a, ty.size() * 8)?,
                    const_in_ring(b, ty.size() * 8)?,
                    ty.size() * 8,
                ))
            };
            let yes = evaluate(a, c)?;
            let no = evaluate(b, d)?;
            if yes == no {
                return Some(copy(*dest, constant(u64::from(yes), IrType::I32)));
            }
            Some(vec![Instruction::Select {
                dest: *dest,
                cond: ca,
                true_val: constant(u64::from(yes), IrType::I32),
                false_val: constant(u64::from(no), IrType::I32),
                ty: IrType::I32,
            }])
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "bit_idioms_select_facts_tests.rs"]
mod tests;
