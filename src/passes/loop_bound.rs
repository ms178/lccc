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

/// Target-independent legality, deliberately x86-64/O2+ non-size profitability
/// rollout: only that downstream unroll/vector pipeline has corpus evidence.
/// Run before its first unroll decision.
/// CCC_DISABLE_PASSES=loop_bound remains the dedicated ablation switch.
pub(super) fn scheduled(
    iteration: usize,
    opt_level: u32,
    optimize_for_size: bool,
    target: crate::backend::Target,
    disabled: &str,
) -> bool {
    iteration == 0
        && opt_level >= 2
        && !optimize_for_size
        && target == crate::backend::Target::X86_64
        && !super::pass_disabled(disabled, "loop_bound")
        // `CCC_NO_AFFINE_EXIT_FOLD` must cover EVERY producer of the
        // affine exit-compare fold: a switch that quietly leaves one
        // producer running turns the A/B that condemns the fold into the
        // A/B that hides it. For finer attribution this early unsigned
        // producer alone is disabled with `CCC_DISABLE_PASSES=loop_bound`
        // (the late fold then takes over where its weaker proof allows);
        // the full matrix is pinned by check_unsigned_affine_loop.sh.
        && super::loop_rotate::affine_fold_enabled()
}

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
    // All operands are <= max < 2^64 and trips*step = (span/step)*step + step
    // <= span + step < 2^65, so the u128 sums below cannot overflow; the
    // checked forms make that bound a compiler-verified fact rather than a
    // comment-only proof.
    let final_iv = trips.checked_mul(step).and_then(|p| init.checked_add(p))?;
    // Include the exit observation: allowing just the body interval admits
    // a wrapping header sum, which can turn a terminating loop into a cycle.
    (final_iv.checked_add(offset)? <= max).then_some(bound as u64)
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

    // Execute the actual CFG, typed arithmetic, phi edges, comparisons and
    // terminators; this does not reuse adjusted_limit or the matcher algebra.
    fn interpret(f: &IrFunction) -> (Option<u64>, Vec<(u64, bool)>) {
        let mut values = [0u64; 8];
        let mut block = 0;
        let mut previous = f.blocks[0].label;
        let mut trace = Vec::new();
        let operand = |op: Operand, values: &[u64; 8]| match op {
            Operand::Const(c) => c.to_i64().unwrap() as u64,
            Operand::Value(v) => values[v.0 as usize],
        };
        for _ in 0..1024 {
            let b = &f.blocks[block];
            for inst in &b.instructions {
                let (dest, result) = match *inst {
                    Instruction::Phi {
                        dest,
                        ref incoming,
                        ty,
                    } => {
                        let op = incoming
                            .iter()
                            .find(|(_, label)| *label == previous)
                            .unwrap()
                            .0;
                        let mask = u64::MAX >> (64 - ty.size() * 8);
                        (dest, operand(op, &values) & mask)
                    }
                    Instruction::BinOp {
                        dest,
                        op: IrBinOp::Add,
                        lhs,
                        rhs,
                        ty,
                    } => {
                        let mask = u64::MAX >> (64 - ty.size() * 8);
                        (
                            dest,
                            operand(lhs, &values).wrapping_add(operand(rhs, &values)) & mask,
                        )
                    }
                    Instruction::Cmp {
                        dest,
                        op,
                        lhs,
                        rhs,
                        ty,
                    } => {
                        let mask = u64::MAX >> (64 - ty.size() * 8);
                        let (l, r) = (operand(lhs, &values) & mask, operand(rhs, &values) & mask);
                        let yes = match op {
                            IrCmpOp::Ult => l < r,
                            IrCmpOp::Ule => l <= r,
                            IrCmpOp::Ugt => l > r,
                            IrCmpOp::Uge => l >= r,
                            _ => panic!("unsupported test comparison"),
                        };
                        (dest, u64::from(yes))
                    }
                    _ => panic!("unsupported test instruction"),
                };
                values[dest.0 as usize] = result;
            }
            let next = match b.terminator {
                Terminator::Return(Some(op)) => return (Some(operand(op, &values)), trace),
                Terminator::Branch(label) => label,
                Terminator::CondBranch {
                    cond,
                    true_label,
                    false_label,
                } => {
                    let yes = operand(cond, &values) != 0;
                    trace.push((values[1], yes));
                    if yes { true_label } else { false_label }
                }
                _ => panic!("unsupported test terminator"),
            };
            previous = b.label;
            block = f.blocks.iter().position(|b| b.label == next).unwrap();
        }
        (None, trace) // Bounded observation of intentionally wrapping/cyclic refusals.
    }

    #[test]
    fn interpreted_all_compare_spellings_preserve_body_and_final_header_trajectories() {
        use crate::ir::reexports::BlockId;
        let mut accepted = 0;
        for ty in [IrType::U8, IrType::U16, IrType::U32, IrType::U64] {
            let max = u64::MAX >> (64 - ty.size() * 8);
            for (init, step, offset, limit) in [
                (0, 1, 1, 8),
                (2, 3, 4, 32),
                (5, 8, 2, 32),
                (30, 3, 1, 31),
                (8, 1, 2, 4),
                (0, 0, 1, 8),
                (max - 5, 2, 2, max - 1),
                (max - 20, 3, 2, max - 4),
                (max - 1, 1, 2, max),
                (max - 7, 3, 2, max),
            ] {
                for inclusive in [false, true] {
                    for swapped in [false, true] {
                        for reversed in [false, true] {
                            for wraps in [false, true] {
                                let mut f = fixture(
                                    ty,
                                    init as i64,
                                    step as i64,
                                    offset as i64,
                                    limit as i64,
                                );
                                f.pass_policy.wrap_signed_overflow = wraps;
                                if let Instruction::Cmp { lhs, rhs, op, .. } =
                                    &mut f.blocks[1].instructions[2]
                                {
                                    if swapped {
                                        std::mem::swap(lhs, rhs);
                                    }
                                    *op = match (inclusive, swapped, reversed) {
                                        (false, false, false) | (true, true, true) => IrCmpOp::Ult,
                                        (false, true, false) | (true, false, true) => IrCmpOp::Ugt,
                                        (false, false, true) | (true, true, false) => IrCmpOp::Uge,
                                        (false, true, true) | (true, false, false) => IrCmpOp::Ule,
                                    };
                                }
                                if reversed {
                                    if let Terminator::CondBranch {
                                        true_label,
                                        false_label,
                                        ..
                                    } = &mut f.blocks[1].terminator
                                    {
                                        std::mem::swap(true_label, false_label);
                                    }
                                }
                                // Exercise label/index separation rather than identity-labelled CFGs.
                                let relabel = |l: BlockId| BlockId(10 + l.0 * 20);
                                for b in &mut f.blocks {
                                    b.label = relabel(b.label);
                                    for i in &mut b.instructions {
                                        if let Instruction::Phi { incoming, .. } = i {
                                            for (_, l) in incoming {
                                                *l = relabel(*l);
                                            }
                                        }
                                    }
                                    match &mut b.terminator {
                                        Terminator::Branch(l) => *l = relabel(*l),
                                        Terminator::CondBranch {
                                            true_label,
                                            false_label,
                                            ..
                                        } => {
                                            *true_label = relabel(*true_label);
                                            *false_label = relabel(*false_label);
                                        }
                                        _ => {}
                                    }
                                }
                                f.next_label = 100;
                                let before = interpret(&f);
                                let original = format!("{f:?}");
                                let n = run(&mut f);
                                if init == 0 && step == 1 && offset == 1 && limit == 8 {
                                    assert_eq!(
                                        n, 1,
                                        "every positive compare spelling must rewrite"
                                    );
                                }
                                if n == 0 {
                                    assert_eq!(format!("{f:?}"), original);
                                } else {
                                    accepted += 1;
                                    assert!(before.0.is_some());
                                }
                                assert_eq!(
                                    interpret(&f),
                                    before,
                                    "{ty:?} {init} {step} {offset} {limit} {inclusive} {swapped} {reversed}"
                                );
                            }
                        }
                    }
                }
            }
        }
        assert!(accepted > 100);
    }

    #[test]
    fn scheduling_honors_the_global_affine_kill_switch() {
        // `CCC_NO_AFFINE_EXIT_FOLD` owns EVERY affine exit-compare producer,
        // including this early canonicalizer; `CCC_DISABLE_PASSES=loop_bound`
        // remains the early-only ablation handle. Pinned end-to-end by
        // check_unsigned_affine_loop.sh.
        use crate::backend::Target;
        struct Restore(bool, bool);
        impl Drop for Restore {
            fn drop(&mut self) {
                super::super::loop_rotate::set_affine_fold_knobs(self.0, self.1);
            }
        }
        let _restore = Restore(
            super::super::loop_rotate::affine_fold_enabled(),
            super::super::loop_rotate::affine_fold_debug(),
        );
        for affine in [false, true] {
            super::super::loop_rotate::set_affine_fold_knobs(affine, false);
            for level in 0..=5 {
                assert_eq!(
                    scheduled(0, level, level >= 4, Target::X86_64, ""),
                    affine && (2..=3).contains(&level)
                );
            }
            // `affine_fold` in CCC_DISABLE_PASSES only ablates the late fold;
            // the early producer follows the global switch alone.
            assert_eq!(
                scheduled(0, 2, false, Target::X86_64, "affine_fold"),
                affine
            );
            assert!(!scheduled(0, 2, false, Target::X86_64, "loop_bound"));
            assert!(!scheduled(1, 2, false, Target::X86_64, ""));
            assert!(!scheduled(0, 2, false, Target::Aarch64, ""));
        }
    }
}
