//! Canonicalize unsigned counted-loop guards `(iv + k) <[=] limit`.
//!
//! Subtracting k from the bound is NOT a modular identity. We prove a finite
//! increasing trajectory, including the final failing header test, in u128.
//! Both the IV and its affine sum must remain in their original type domain.
//! No signed-overflow assumption, memory speculation, CFG change, new value,
//! or metadata rewrite is involved. This exposes existing unroll machinery.
use super::loop_analysis;
use crate::common::types::IrType;
use crate::ir::analysis::CfgAnalysis;
use crate::ir::reexports::{
    Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Terminator, Value,
};

fn adjusted_limit(
    init: u64,
    step: u64,
    offset: u64,
    limit: u64,
    bits: u32,
    inclusive: bool,
) -> Option<u64> {
    if !(1..=64).contains(&bits) || step == 0 || offset == 0 {
        return None;
    }
    let max = (1u128 << bits) - 1;
    let (init, step, offset, limit) = (
        u128::from(init),
        u128::from(step),
        u128::from(offset),
        u128::from(limit),
    );
    if init > max || step > max || offset > max || limit > max || init + offset > max {
        return None;
    }
    let bound = limit.checked_sub(offset)?;
    let span = bound.checked_sub(init)?;
    let trips = if inclusive {
        span / step + 1
    } else {
        span.checked_sub(1)? / step + 1
    };
    let final_iv = init + trips * step;
    // Include the exit observation: allowing just the body interval admits
    // a wrapping header sum, which can turn a terminating loop into a cycle.
    (final_iv + offset <= max).then_some(bound as u64)
}

fn unsigned_constant(op: Operand, ty: IrType) -> Option<u64> {
    let Operand::Const(c) = op else {
        return None;
    };
    let raw = c.to_i64()? as u64;
    Some(if ty.size() == 8 {
        raw
    } else {
        raw & ((1u64 << (ty.size() * 8)) - 1)
    })
}

fn value_plus_const(inst: &Instruction, ty: IrType) -> Option<(Value, u64)> {
    let Instruction::BinOp {
        op: IrBinOp::Add,
        lhs,
        rhs,
        ty: add_ty,
        ..
    } = *inst
    else {
        return None;
    };
    if add_ty != ty {
        return None;
    }
    match (lhs, rhs) {
        (Operand::Value(v), c @ Operand::Const(_)) | (c @ Operand::Const(_), Operand::Value(v)) => {
            Some((v, unsigned_constant(c, ty)?))
        }
        _ => None,
    }
}

struct Plan {
    header: usize,
    compare: usize,
    iv: Value,
    bound: IrConst,
    op: IrCmpOp,
}

pub(crate) fn run(func: &mut IrFunction) -> usize {
    if !func.blocks.iter().any(|b| {
        b.instructions.iter().any(|i| {
            matches!(
                i,
                Instruction::Phi {
                    ty: IrType::U8 | IrType::U16 | IrType::U32 | IrType::U64,
                    ..
                }
            )
        })
    }) {
        return 0;
    }
    if !super::gvn::find_multi_def_values(func).is_empty() {
        return 0;
    }
    let cfg = CfgAnalysis::build(func);
    let loops = loop_analysis::find_merged_natural_loops(
        func.blocks.len(),
        &cfg.preds,
        &cfg.succs,
        &cfg.idom,
    );
    let mut plans = Vec::new();
    for lp in &loops {
        let Some(pre) = lp.find_dedicated_preheader(&cfg.preds, &cfg.succs) else {
            continue;
        };
        let Some(latch) = lp.single_latch(&cfg.preds) else {
            continue;
        };
        if latch == lp.header
            || !matches!(func.blocks[latch].terminator, Terminator::Branch(label) if label == func.blocks[lp.header].label)
        {
            continue;
        }
        let header = &func.blocks[lp.header];
        let Terminator::CondBranch {
            cond: Operand::Value(cond),
            true_label,
            false_label,
        } = header.terminator
        else {
            continue;
        };
        let inside = |label| {
            func.blocks
                .iter()
                .position(|b| b.label == label)
                .is_some_and(|b| lp.contains(b))
        };
        let true_inside = inside(true_label);
        if true_inside == inside(false_label) {
            continue;
        }
        let Some((
            ci,
            Instruction::Cmp {
                lhs, rhs, op, ty, ..
            },
        )) = header
            .instructions
            .iter()
            .enumerate()
            .find(|(_, i)| i.dest() == Some(cond))
        else {
            continue;
        };
        if !matches!(ty, IrType::U8 | IrType::U16 | IrType::U32 | IrType::U64) {
            continue;
        }
        let (sum, limit, op) = match (*lhs, *rhs, *op) {
            (Operand::Value(v), c @ Operand::Const(_), op) => (v, c, op),
            (c @ Operand::Const(_), Operand::Value(v), IrCmpOp::Ugt) => (v, c, IrCmpOp::Ult),
            (c @ Operand::Const(_), Operand::Value(v), IrCmpOp::Uge) => (v, c, IrCmpOp::Ule),
            (c @ Operand::Const(_), Operand::Value(v), IrCmpOp::Ult) => (v, c, IrCmpOp::Ugt),
            (c @ Operand::Const(_), Operand::Value(v), IrCmpOp::Ule) => (v, c, IrCmpOp::Uge),
            _ => continue,
        };
        let inclusive = match (true_inside, op) {
            (true, IrCmpOp::Ult) | (false, IrCmpOp::Uge) => false,
            (true, IrCmpOp::Ule) | (false, IrCmpOp::Ugt) => true,
            _ => continue,
        };
        let Some(sum_def) = header.instructions[..ci]
            .iter()
            .find(|i| i.dest() == Some(sum))
        else {
            continue;
        };
        let Some((iv, offset)) = value_plus_const(sum_def, *ty) else {
            continue;
        };
        let Some(Instruction::Phi {
            ty: phi_ty,
            incoming,
            ..
        }) = header.instructions.iter().find(|i| i.dest() == Some(iv))
        else {
            continue;
        };
        if phi_ty != ty || incoming.len() != 2 {
            continue;
        }
        let Some((init, _)) = incoming.iter().find(|(_, l)| *l == func.blocks[pre].label) else {
            continue;
        };
        let Some((Operand::Value(next), _)) = incoming
            .iter()
            .find(|(_, l)| *l == func.blocks[latch].label)
        else {
            continue;
        };
        let Some(next_def) = func.blocks[latch]
            .instructions
            .iter()
            .find(|i| i.dest() == Some(*next))
        else {
            continue;
        };
        let Some((updated, step)) = value_plus_const(next_def, *ty) else {
            continue;
        };
        if updated != iv {
            continue;
        }
        let (Some(init), Some(limit)) =
            (unsigned_constant(*init, *ty), unsigned_constant(limit, *ty))
        else {
            continue;
        };
        let Some(bound) =
            adjusted_limit(init, step, offset, limit, (ty.size() * 8) as u32, inclusive)
        else {
            continue;
        };
        plans.push(Plan {
            header: lp.header,
            compare: ci,
            iv,
            bound: IrConst::from_i64(bound as i64, *ty),
            op,
        });
    }
    let count = plans.len();
    for p in plans {
        if let Instruction::Cmp { lhs, rhs, op, .. } =
            &mut func.blocks[p.header].instructions[p.compare]
        {
            *lhs = Operand::Value(p.iv);
            *rhs = Operand::Const(p.bound);
            *op = p.op;
        }
    }
    if count > 0 && super::loop_rotate::affine_fold_debug() {
        eprintln!(
            "[AFFOLD] {}: affine exit-compare folds: {} (early unsigned)",
            func.name, count
        );
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(ty: IrType, init: i64, step: i64, offset: i64, limit: i64) -> IrFunction {
        use crate::ir::reexports::{BasicBlock, BlockId};
        let c = |n| Operand::Const(IrConst::from_i64(n, ty));
        let v = |n| Operand::Value(Value(n));
        let mut f = IrFunction::new("affine_bound".into(), ty, vec![], false);
        f.blocks = vec![
            BasicBlock {
                label: BlockId(0),
                instructions: vec![],
                source_spans: vec![],
                terminator: Terminator::Branch(BlockId(1)),
            },
            BasicBlock {
                label: BlockId(1),
                instructions: vec![
                    Instruction::Phi {
                        dest: Value(1),
                        ty,
                        incoming: vec![(c(init), BlockId(0)), (v(4), BlockId(2))],
                    },
                    Instruction::BinOp {
                        dest: Value(2),
                        op: IrBinOp::Add,
                        lhs: v(1),
                        rhs: c(offset),
                        ty,
                    },
                    Instruction::Cmp {
                        dest: Value(3),
                        op: IrCmpOp::Ule,
                        lhs: v(2),
                        rhs: c(limit),
                        ty,
                    },
                ],
                source_spans: vec![crate::common::source::Span::dummy(); 3],
                terminator: Terminator::CondBranch {
                    cond: v(3),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            },
            BasicBlock {
                label: BlockId(2),
                instructions: vec![Instruction::BinOp {
                    dest: Value(4),
                    op: IrBinOp::Add,
                    lhs: v(1),
                    rhs: c(step),
                    ty,
                }],
                source_spans: vec![crate::common::source::Span::dummy()],
                terminator: Terminator::Branch(BlockId(1)),
            },
            BasicBlock {
                label: BlockId(3),
                instructions: vec![],
                source_spans: vec![],
                terminator: Terminator::Return(Some(v(1))),
            },
        ];
        f.next_value_id = 5;
        f.next_label = 4;
        f
    }

    #[test]
    fn affine_bound_matcher_preserves_ssa_metadata_and_is_idempotent() {
        for ty in [IrType::U8, IrType::U16, IrType::U32, IrType::U64] {
            for swapped in [false, true] {
                for reversed_branch in [false, true] {
                    let mut f = fixture(ty, 0, 8, 8, 128);
                    if let Instruction::Cmp { lhs, rhs, op, .. } = &mut f.blocks[1].instructions[2]
                    {
                        if swapped {
                            std::mem::swap(lhs, rhs);
                        }
                        *op = match (swapped, reversed_branch) {
                            (false, false) => IrCmpOp::Ule,
                            (true, false) => IrCmpOp::Uge,
                            (false, true) => IrCmpOp::Ugt,
                            (true, true) => IrCmpOp::Ult,
                        };
                    }
                    if reversed_branch {
                        if let Terminator::CondBranch {
                            true_label,
                            false_label,
                            ..
                        } = &mut f.blocks[1].terminator
                        {
                            std::mem::swap(true_label, false_label);
                        }
                    }
                    let spans = format!(
                        "{:?}",
                        f.blocks.iter().map(|b| &b.source_spans).collect::<Vec<_>>()
                    );
                    assert_eq!(run(&mut f), 1);
                    assert_eq!(run(&mut f), 0);
                    assert_eq!(f.next_value_id, 5);
                    assert_eq!(f.next_label, 4);
                    assert_eq!(
                        spans,
                        format!(
                            "{:?}",
                            f.blocks.iter().map(|b| &b.source_spans).collect::<Vec<_>>()
                        )
                    );
                    let mut errors = Vec::new();
                    super::super::verify::verify_function(&f, "affine_bound", &mut errors);
                    assert!(errors.is_empty(), "{errors:?}");
                }
            }
        }
    }

    #[test]
    fn affine_bound_refusals_are_transactional() {
        let mut cases = vec![
            fixture(IrType::U8, 0, 5, 6, 253),
            fixture(IrType::U8, 254, 1, 2, 255),
            fixture(IrType::I32, 0, 8, 8, 128),
            fixture(IrType::U64, 0, 0, 8, 128),
            fixture(IrType::U64, 200, 8, 8, 128),
        ];
        let mut dynamic = fixture(IrType::U64, 0, 8, 8, 128);
        if let Instruction::Phi { incoming, .. } = &mut dynamic.blocks[1].instructions[0] {
            incoming[0].0 = Operand::Value(Value(90));
        }
        cases.push(dynamic);
        let mut wrong_step = fixture(IrType::U64, 0, 8, 8, 128);
        if let Instruction::BinOp { ty, .. } = &mut wrong_step.blocks[2].instructions[0] {
            *ty = IrType::I32;
        }
        cases.push(wrong_step);
        for mut f in cases {
            let before = format!("{f:?}");
            assert_eq!(run(&mut f), 0);
            assert_eq!(format!("{f:?}"), before);
        }
    }

    #[test]
    fn affine_bound_exhaustive_four_bit_modular_trajectories() {
        let mut accepted = 0;
        for init in 0..16 {
            for step in 1..16 {
                for offset in 1..16 {
                    for limit in 0..16 {
                        for inclusive in [false, true] {
                            let Some(bound) =
                                adjusted_limit(init, step, offset, limit, 4, inclusive)
                            else {
                                continue;
                            };
                            accepted += 1;
                            let mut iv = init;
                            let mut exited = false;
                            for _ in 0..=16 {
                                let sum = (iv + offset) & 15;
                                let original = if inclusive { sum <= limit } else { sum < limit };
                                let replacement = if inclusive { iv <= bound } else { iv < bound };
                                assert_eq!(
                                    original, replacement,
                                    "{init} {step} {offset} {limit} {inclusive}"
                                );
                                if !original {
                                    exited = true;
                                    break;
                                }
                                iv = (iv + step) & 15;
                            }
                            assert!(exited);
                        }
                    }
                }
            }
        }
        assert!(accepted > 1000);
    }
    #[test]
    fn affine_bound_exit_overflow_and_u64_carriers() {
        assert_eq!(adjusted_limit(0, 8, 8, 4096, 64, true), Some(4088));
        assert_eq!(adjusted_limit(0, 5, 6, 253, 8, true), None);
        assert_eq!(adjusted_limit(254, 1, 2, 255, 8, true), None);
        assert_eq!(adjusted_limit(0, 1, 8, 4, 64, true), None);
        assert_eq!(
            adjusted_limit(u64::MAX - 20, 1, 2, u64::MAX - 4, 64, true),
            Some(u64::MAX - 6)
        );
        assert_eq!(adjusted_limit(0, 1, 1, u64::MAX, 64, true), None);
    }
}
