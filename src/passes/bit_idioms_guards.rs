//! Follow-up scalar contracts sharing the bounded bit-idiom observation context.
//! These are consumer rewrites, not speculation or CFG motion. In particular,
//! an arbitrary nonzero integer is NOT a Boolean mask, and a signed upper bound
//! must be proven nonnegative before discarding the lower range guard.
use super::*;

pub(super) fn fold(
    inst: &Instruction,
    cx: &Context<'_>,
    facts: &Facts,
    fresh: Value,
) -> Option<Vec<Instruction>> {
    if let Instruction::BinOp {
        dest,
        op: IrBinOp::Add,
        lhs,
        rhs,
        ty,
    } = inst
        && scalar(*ty)
    {
        let mask = u64::MAX >> (64 - ty.size() * 8);
        for (xor, boolean) in [(*lhs, *rhs), (*rhs, *lhs)] {
            // A comparison (possibly copied/cast) or known bits must certify
            // exactly {0,1}; truthiness alone does not prove the identity.
            if cx.boolean(boolean).is_none()
                && cx.known(boolean, *ty, facts, 8).0 & (mask & !1) != mask & !1
            {
                continue;
            }
            let Some((a, b)) = cx.binary(xor, IrBinOp::Xor, *ty) else {
                continue;
            };
            for (x, neg) in [(a, b), (b, a)] {
                if !matches!(cx.def(neg), Some(Instruction::UnaryOp {
                    op: IrUnaryOp::Neg, src, ty: t, ..
                }) if *src == boolean && *t == *ty)
                    && !matches!(cx.def(neg), Some(Instruction::BinOp {
                        op: IrBinOp::Sub, lhs: zero, rhs: src, ty: t, ..
                    }) if *src == boolean && *t == *ty && const_in_ring(*zero, ty.size()*8) == Some(0))
                {
                    continue;
                }
                // Do not create a select while retaining a shared XOR/mask
                // computation: the dedicated three-op form is then cheaper.
                let single_use = |o| match o {
                    Operand::Value(v) => cx.uses.get(v.0 as usize) == Some(&1),
                    _ => false,
                };
                if !single_use(xor) || !single_use(neg) {
                    continue;
                }
                return Some(vec![
                    Instruction::UnaryOp {
                        dest: fresh,
                        op: IrUnaryOp::Neg,
                        src: x,
                        ty: *ty,
                    },
                    Instruction::Select {
                        dest: *dest,
                        cond: cx.boolean(boolean).unwrap_or(boolean),
                        true_val: Operand::Value(fresh),
                        false_val: x,
                        ty: *ty,
                    },
                ]);
            }
        }
    }
    if let Instruction::Select {
        dest,
        cond,
        true_val,
        false_val,
        ty: result_ty,
    } = inst
        && scalar(*result_ty)
    {
        let (guard, result) = match cx.def(cx.boolean(*cond)?)? {
            Instruction::Cmp {
                op: IrCmpOp::Sge,
                lhs,
                rhs,
                ty,
                ..
            } if scalar(*ty)
                && const_in_ring(*rhs, ty.size() * 8) == Some(0)
                && const_in_ring(*false_val, result_ty.size() * 8) == Some(0) =>
            {
                ((*lhs, *ty), *true_val)
            }
            Instruction::Cmp {
                op: IrCmpOp::Slt,
                lhs,
                rhs,
                ty,
                ..
            } if scalar(*ty)
                && const_in_ring(*rhs, ty.size() * 8) == Some(0)
                && const_in_ring(*true_val, result_ty.size() * 8) == Some(0) =>
            {
                ((*lhs, *ty), *false_val)
            }
            _ => return None,
        };
        if let Some(Instruction::Cmp {
            op: IrCmpOp::Slt,
            lhs,
            rhs,
            ty,
            ..
        }) = cx.boolean(result).and_then(|b| cx.def(b))
            && (*lhs, *ty) == guard
            && sign_bit_zero(cx, *rhs, *ty, facts, 8)
        {
            // 0 <=s x <s y, with 0 <=s y, iff x <u y. The matched SSA
            // operand and width must agree in BOTH comparisons; no peeling
            // of truncations or sign/zero extensions is justified here.
            let compare = Instruction::Cmp {
                dest: if *result_ty == IrType::I8 {
                    *dest
                } else {
                    fresh
                },
                op: IrCmpOp::Ult,
                lhs: *lhs,
                rhs: *rhs,
                ty: *ty,
            };
            return Some(if *result_ty == IrType::I8 {
                vec![compare]
            } else {
                vec![
                    compare,
                    Instruction::Cast {
                        dest: *dest,
                        src: Operand::Value(fresh),
                        from_ty: IrType::I8,
                        to_ty: *result_ty,
                    },
                ]
            });
        }
    }
    None
}

fn sign_bit_zero(cx: &Context<'_>, op: Operand, ty: IrType, facts: &Facts, depth: u8) -> bool {
    if depth == 0 {
        return false;
    }
    if cx.known(op, ty, facts, depth).0 & (1u64 << (ty.size() * 8 - 1)) != 0 {
        return true;
    }
    match cx.def(op) {
        Some(Instruction::Copy { src, .. }) => sign_bit_zero(cx, *src, ty, facts, depth - 1),
        Some(Instruction::Cast {
            src,
            from_ty,
            to_ty,
            ..
        }) if *to_ty == ty && scalar(*from_ty) && from_ty.size() == ty.size() => {
            sign_bit_zero(cx, *src, *from_ty, facts, depth - 1)
        }
        _ => false,
    }
}

/// Distribute the specific null-observation/Boolean-PHI join from #233785.
/// One incoming pointer is null (the result is false regardless of its Boolean
/// partner); the other Boolean incoming is exactly one. Move ONLY a pure
/// comparison/cast to that predecessor and produce a Boolean result PHI.
/// No loads/calls are moved or deleted here. Normal DCE decides whether an
/// orphaned nonvolatile load is removable. One rewrite per sweep bounds growth.
pub(super) fn cfg(func: &mut IrFunction) -> usize {
    use crate::ir::analysis::{build_cfg, build_label_map, compute_dominators, dominates_idom};
    if !func.blocks.iter().any(|b| {
        b.instructions
            .iter()
            .any(|i| matches!(i, Instruction::Phi { .. }))
    }) || !func.blocks.iter().any(|b| {
        b.instructions
            .iter()
            .any(|i| matches!(i, Instruction::Select { .. }))
    }) {
        return 0;
    }
    let max = func.max_value_id();
    if max > 1_000_000 || func.next_value_id == u32::MAX {
        return 0;
    }
    let next = func.next_value_id.max(max.saturating_add(1));
    let Some(after) = next.checked_add(2) else {
        return 0;
    };
    if after > 1_000_000 {
        return 0;
    }
    let mut defs = vec![None; max as usize + 1];
    for (bi, b) in func.blocks.iter().enumerate() {
        if !b.source_spans.is_empty() && b.source_spans.len() != b.instructions.len() {
            return 0;
        }
        for (ii, i) in b.instructions.iter().enumerate() {
            if let Some(d) = i.dest() {
                if defs[d.0 as usize].replace((bi, ii, i.clone())).is_some() {
                    return 0;
                }
            }
        }
    }
    let definition = |o: Operand| match o {
        Operand::Value(v) => defs.get(v.0 as usize)?.as_ref(),
        _ => None,
    };
    let peel = |mut o: Operand, casts: bool| {
        for _ in 0..16 {
            match definition(o) {
                Some((_, _, Instruction::Copy { src, .. })) => o = *src,
                Some((
                    _,
                    _,
                    Instruction::Cast {
                        src,
                        from_ty,
                        to_ty,
                        ..
                    },
                )) if casts && scalar(*from_ty) && scalar(*to_ty) => o = *src,
                _ => break,
            }
        }
        o
    };
    let labels = build_label_map(func);
    if labels.len() != func.blocks.len() {
        return 0;
    }
    let (preds, succs) = build_cfg(func, &labels);
    let idom = compute_dominators(func.blocks.len(), &preds, &succs);
    for (bi, b) in func.blocks.iter().enumerate() {
        if bi == 0 || preds.len(bi) != 2 {
            continue;
        }
        let prefix = b
            .instructions
            .iter()
            .take_while(|i| matches!(i, Instruction::Phi { .. }))
            .count();
        for (ii, i) in b.instructions.iter().enumerate() {
            let Instruction::Select {
                dest,
                cond,
                true_val,
                false_val,
                ty,
            } = i
            else {
                continue;
            };
            if !scalar(*ty) || const_in_ring(*false_val, ty.size() * 8) != Some(0) {
                continue;
            }
            let Some((
                cb,
                ci,
                Instruction::Phi {
                    incoming: booleans,
                    ty: bt,
                    ..
                },
            )) = definition(peel(*cond, true))
            else {
                continue;
            };
            if *cb != bi || *ci >= prefix || booleans.len() != 2 || !scalar(*bt) {
                continue;
            }
            let mut cmp = peel(*true_val, true);
            let Some((
                qb,
                qi,
                Instruction::Cmp {
                    op: IrCmpOp::Ne,
                    lhs,
                    rhs,
                    ty: ct,
                    ..
                },
            )) = definition(cmp)
            else {
                continue;
            };
            if *qb != bi || *qi >= ii {
                continue;
            }
            if !scalar(*ct) && *ct != IrType::Ptr {
                continue;
            }
            cmp = if const_in_ring(*rhs, ct.size() * 8) == Some(0) {
                *lhs
            } else if const_in_ring(*lhs, ct.size() * 8) == Some(0) {
                *rhs
            } else {
                continue;
            };
            let Some((
                pb,
                pi,
                Instruction::Phi {
                    incoming: pointers,
                    ty: pt,
                    ..
                },
            )) = definition(peel(cmp, false))
            else {
                continue;
            };
            if *pb != bi
                || *pi >= prefix
                || !(*pt == *ct || *pt == IrType::Ptr && scalar(*ct) && pt.size() == ct.size())
                || pointers.len() != 2
            {
                continue;
            }
            for side in 0..2 {
                let (null, from_null) = pointers[side];
                let (pointer, from_pointer) = pointers[1 - side];
                if from_null == from_pointer || const_in_ring(null, ct.size() * 8) != Some(0) {
                    continue;
                }
                let Some(&pred) = labels.get(&from_pointer) else {
                    continue;
                };
                let Some(&null_pred) = labels.get(&from_null) else {
                    continue;
                };
                if !preds.row(bi).contains(&(pred as u32))
                    || !preds.row(bi).contains(&(null_pred as u32))
                {
                    continue;
                }
                if booleans.iter().filter(|(_, l)| *l == from_null).count() != 1
                    || booleans.iter().filter(|(_, l)| *l == from_pointer).count() != 1
                {
                    continue;
                }
                let Some((enabled, _)) = booleans.iter().find(|(_, l)| *l == from_pointer) else {
                    continue;
                };
                if const_in_ring(*enabled, bt.size() * 8) != Some(1) {
                    continue;
                }
                // The new operand must already be available at the predecessor
                // terminator, not merely at the join. Fail closed on unreachable
                // or malformed incoming definitions as well.
                if let Operand::Value(_) = pointer {
                    let Some((db, _, _)) = definition(pointer) else {
                        continue;
                    };
                    if !dominates_idom(&idom, *db, pred) {
                        continue;
                    }
                }
                let (dest, ty, ct) = (*dest, *ty, *ct);
                let span = b.source_spans.get(ii).copied();
                let compare = Instruction::Cmp {
                    dest: Value(next),
                    op: IrCmpOp::Ne,
                    lhs: pointer,
                    rhs: constant(0, ct),
                    ty: ct,
                };
                let cast = Instruction::Cast {
                    dest: Value(next + 1),
                    src: Operand::Value(Value(next)),
                    from_ty: IrType::I8,
                    to_ty: ty,
                };
                let pred_block = &mut func.blocks[pred];
                if (pred_block.instructions.is_empty() || !pred_block.source_spans.is_empty())
                    && let Some(placed) = span.or_else(|| pred_block.source_spans.last().copied())
                {
                    pred_block.source_spans.extend([placed; 2]);
                }
                pred_block.instructions.extend([compare, cast]);
                let join = &mut func.blocks[bi];
                join.instructions.remove(ii);
                if !join.source_spans.is_empty() {
                    join.source_spans.remove(ii);
                }
                join.instructions.insert(
                    prefix,
                    Instruction::Phi {
                        dest,
                        ty,
                        incoming: vec![
                            (constant(0, ty), from_null),
                            (Operand::Value(Value(next + 1)), from_pointer),
                        ],
                    },
                );
                if let Some(span) = span {
                    join.source_spans.insert(prefix, span);
                }
                func.next_value_id = after;
                return 1;
            }
        }
    }
    0
}

#[cfg(test)]
mod cfg_tests {
    use super::*;
    use crate::common::source::Span;
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};
    fn v(n: u32) -> Operand {
        Operand::Value(Value(n))
    }
    fn make(pointer: u64, enabled: u64, edge: u64) -> IrFunction {
        let block = |label, instructions: Vec<Instruction>, terminator| BasicBlock {
            label: BlockId(label),
            source_spans: vec![Span::dummy(); instructions.len()],
            instructions,
            terminator,
        };
        let mut f = IrFunction::new("phi_observation".into(), IrType::I64, vec![], false);
        f.blocks = vec![
            block(
                0,
                vec![
                    Instruction::Copy {
                        dest: Value(0),
                        src: constant(pointer, IrType::I64),
                    },
                    Instruction::Copy {
                        dest: Value(1),
                        src: constant(enabled, IrType::I64),
                    },
                    Instruction::Copy {
                        dest: Value(2),
                        src: constant(edge, IrType::I32),
                    },
                ],
                Terminator::CondBranch {
                    cond: v(2),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            block(1, vec![], Terminator::Branch(BlockId(3))),
            block(2, vec![], Terminator::Branch(BlockId(3))),
            block(
                3,
                vec![
                    Instruction::Phi {
                        dest: Value(4),
                        ty: IrType::I64,
                        incoming: vec![(constant(0, IrType::I64), BlockId(1)), (v(0), BlockId(2))],
                    },
                    Instruction::Phi {
                        dest: Value(5),
                        ty: IrType::I64,
                        incoming: vec![(v(1), BlockId(1)), (constant(1, IrType::I64), BlockId(2))],
                    },
                    Instruction::Cmp {
                        dest: Value(6),
                        op: IrCmpOp::Ne,
                        lhs: v(4),
                        rhs: constant(0, IrType::I64),
                        ty: IrType::I64,
                    },
                    Instruction::Cast {
                        dest: Value(7),
                        src: v(6),
                        from_ty: IrType::I8,
                        to_ty: IrType::I64,
                    },
                    Instruction::Select {
                        dest: Value(8),
                        cond: v(5),
                        true_val: v(7),
                        false_val: constant(0, IrType::I64),
                        ty: IrType::I64,
                    },
                ],
                Terminator::Return(Some(v(8))),
            ),
        ];
        f.next_value_id = 9;
        f
    }
    fn evaluate(f: &IrFunction) -> u64 {
        let mut values = vec![0u64; f.next_value_id as usize + 1];
        let get = |o: Operand, vs: &[u64]| match o {
            Operand::Value(v) => vs[v.0 as usize],
            Operand::Const(c) => c.to_i128().unwrap() as u64,
        };
        let mut at = 0;
        let mut previous = BlockId(0);
        loop {
            let b = &f.blocks[at];
            for i in &b.instructions {
                let value = match i {
                    Instruction::Copy { src, .. } | Instruction::Cast { src, .. } => {
                        get(*src, &values)
                    }
                    Instruction::Phi { incoming, .. } => get(
                        incoming.iter().find(|(_, l)| *l == previous).unwrap().0,
                        &values,
                    ),
                    Instruction::Cmp {
                        op: IrCmpOp::Ne,
                        lhs,
                        rhs,
                        ..
                    } => u64::from(get(*lhs, &values) != get(*rhs, &values)),
                    Instruction::Select {
                        cond,
                        true_val,
                        false_val,
                        ..
                    } => get(
                        if get(*cond, &values) != 0 {
                            *true_val
                        } else {
                            *false_val
                        },
                        &values,
                    ),
                    _ => panic!("unsupported fixture instruction"),
                };
                values[i.dest().unwrap().0 as usize] = value;
            }
            let label = match b.terminator {
                Terminator::Return(Some(o)) => return get(o, &values),
                Terminator::Branch(l) => l,
                Terminator::CondBranch {
                    cond,
                    true_label,
                    false_label,
                } => {
                    if get(cond, &values) != 0 {
                        true_label
                    } else {
                        false_label
                    }
                }
                _ => panic!("unsupported fixture edge"),
            };
            previous = b.label;
            at = f.blocks.iter().position(|b| b.label == label).unwrap();
        }
    }
    #[test]
    fn null_and_enabled_phi_observation_preserves_both_paths_and_spans() {
        for pointer in [0, 1, 0x1234, 1u64 << 63, u64::MAX] {
            for enabled in [0, 1, 2, u64::MAX] {
                for edge in [0, 1] {
                    let mut f = make(pointer, enabled, edge);
                    let expected = evaluate(&f);
                    assert_eq!(cfg(&mut f), 1);
                    assert_eq!(evaluate(&f), expected);
                    for b in &f.blocks {
                        assert_eq!(b.source_spans.len(), b.instructions.len());
                    }
                    assert!(
                        f.blocks[3]
                            .instructions
                            .iter()
                            .take(3)
                            .all(|i| matches!(i, Instruction::Phi { .. }))
                    );
                }
            }
        }
    }
    #[test]
    fn phi_observation_rejects_mismatched_edges_widths_unknown_defs_and_truthy_constants() {
        for variant in 0..6 {
            let mut f = make(17, 1, 0);
            match variant {
                0 => {
                    if let Instruction::Phi { incoming, .. } = &mut f.blocks[3].instructions[1] {
                        incoming[1].0 = constant(256, IrType::I64);
                    }
                }
                1 => {
                    if let Instruction::Phi { incoming, .. } = &mut f.blocks[3].instructions[1] {
                        incoming[1].1 = BlockId(1);
                    }
                }
                2 => {
                    if let Instruction::Phi { ty, .. } = &mut f.blocks[3].instructions[0] {
                        *ty = IrType::I32;
                    }
                }
                3 => {
                    if let Instruction::Phi { incoming, .. } = &mut f.blocks[3].instructions[0] {
                        incoming[1].0 = v(99);
                    }
                }
                4 => {
                    if let Instruction::Cmp { op, .. } = &mut f.blocks[3].instructions[2] {
                        *op = IrCmpOp::Eq;
                    }
                }
                _ => {
                    f.blocks[3].source_spans.pop();
                }
            }
            let before = format!("{f:?}");
            assert_eq!(cfg(&mut f), 0, "{variant}");
            assert_eq!(format!("{f:?}"), before);
        }
    }
}
