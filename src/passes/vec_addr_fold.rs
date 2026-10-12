//! Fold constant GEP chains into register-vector memory displacements.
//!
//! SLP leaves some accesses as VecLoad(GEP(base, C), 0) while other accesses
//! already use VecLoad(base, index, displacement). Leaving the first form
//! through RA creates long-lived pointer homes and loop-time pointer reloads.
//! Canonicalize at IR level so liveness sees the base actually read by the
//! instruction; do not silently rematerialize a register after RA.
//!
//! The argument-address VecStore form's legacy dest_ptr is ignored by the
//! x86 emitter. Remove that redundant use only when canonicalizing its full
//! argument address; the intrinsic's args-writing effect classification must
//! remain authoritative. Gate this pass to x86-64 until other emitters have
//! been audited for the same contract.
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::{
    Instruction, IntrinsicOp, IrBinOp, IrConst, IrFunction, Operand, Value,
};

fn integer(op: Operand) -> Option<i64> {
    match op {
        Operand::Const(c) => c.to_i128().and_then(|n| i64::try_from(n).ok()),
        _ => None,
    }
}

fn peel(
    mut root: Value,
    defs: &FxHashMap<u32, &Instruction>,
    multi: &FxHashSet<u32>,
) -> Option<(Value, i64)> {
    let mut displacement = 0i64;
    let mut seen = FxHashSet::default();
    loop {
        // A materialized GEP captures the base at its definition. It cannot
        // be replaced by a later read of a mutable copy-web base.
        if multi.contains(&root.0) || !seen.insert(root.0) {
            return None;
        }
        let Some(Instruction::GetElementPtr { base, offset, .. }) = defs.get(&root.0).copied()
        else {
            break;
        };
        let Some(delta) = integer(*offset) else { break };
        displacement = displacement.checked_add(delta)?;
        root = *base;
    }
    Some((root, displacement))
}

pub(crate) fn run(func: &mut IrFunction) -> usize {
    let multi = super::gvn::find_multi_def_values(func);
    let mut plans = Vec::new();
    let mut fma_plans = Vec::new();
    {
        let defs: FxHashMap<u32, &Instruction> = func
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .filter_map(|i| i.dest().map(|v| (v.0, i)))
            .collect();
        for (bi, block) in func.blocks.iter().enumerate() {
            for (ii, inst) in block.instructions.iter().enumerate() {
                let Instruction::Intrinsic { op, args, .. } = inst else {
                    continue;
                };
                // Make an offset Add's real inputs visible to liveness.
                // A narrow Add wraps BEFORE address extension and cannot be
                // replaced with a 64-bit address sum. Mutable webs cannot be
                // substituted position-independently either.
                if *op == IntrinsicOp::FmaF64x4HoistedSIB {
                    if args.len() == 3
                        && let Operand::Value(offset) = args[2]
                        && !multi.contains(&offset.0)
                        && let Some(Instruction::BinOp {
                            op: IrBinOp::Add,
                            lhs,
                            rhs,
                            ty: IrType::I64 | IrType::U64,
                            ..
                        }) = defs.get(&offset.0).copied()
                    {
                        let pair = match (lhs, rhs) {
                            (Operand::Value(v), c @ Operand::Const(_))
                            | (c @ Operand::Const(_), Operand::Value(v)) => Some((*v, *c)),
                            _ => None,
                        };
                        if let Some((base, constant)) = pair
                            && !multi.contains(&base.0)
                            // The intrinsic reuses a full-width home directly;
                            // do not lose an implicit narrow operand extension.
                            && matches!(defs.get(&base.0).and_then(|i| i.result_type()), Some(IrType::I64 | IrType::U64 | IrType::Ptr))
                            && let Some(delta) = integer(constant)
                            && i32::try_from(delta).is_ok()
                        {
                            fma_plans.push((bi, ii, base, delta));
                        }
                    }
                    continue;
                }
                let Some((store, _, base, offset, _)) =
                    super::alias::vec_intrinsic_access(op, args)
                else {
                    continue;
                };
                let bi_arg = usize::from(store);
                let di_arg = bi_arg + 2;
                if !(args.len() == di_arg || args.len() == di_arg + 1) {
                    continue;
                }
                let Operand::Value(old_base) = base else {
                    continue;
                };
                let Some((root, delta)) = peel(*old_base, &defs, &multi) else {
                    continue;
                };
                if root == *old_base {
                    continue;
                }
                let old_disp = if args.len() == di_arg {
                    Some(0)
                } else {
                    integer(args[di_arg])
                };
                let Some(mut displacement) = old_disp.and_then(|d| d.checked_add(delta)) else {
                    continue;
                };
                let mut new_offset = *offset;
                // Never generate an out-of-range x86 disp32, including the
                // emitter's constant-index addition. Preserve unusual forms.
                if matches!(offset, Operand::Const(_)) {
                    let Some(sum) = integer(*offset).and_then(|v| displacement.checked_add(v))
                    else {
                        continue;
                    };
                    displacement = sum;
                    new_offset = Operand::Const(IrConst::I64(0));
                }
                if i32::try_from(displacement).is_err() {
                    continue;
                }
                if store && (op.is_pure() || !op.writes_memory_via_args()) {
                    continue;
                }
                plans.push((
                    bi,
                    ii,
                    bi_arg,
                    di_arg,
                    store,
                    root,
                    new_offset,
                    displacement,
                ));
            }
        }
    }
    for &(bi, ii, base_index, disp_index, store, root, offset, displacement) in &plans {
        let Instruction::Intrinsic { args, dest_ptr, .. } = &mut func.blocks[bi].instructions[ii]
        else {
            unreachable!()
        };
        args[base_index] = Operand::Value(root);
        args[base_index + 1] = offset;
        if args.len() == disp_index {
            args.push(Operand::Const(IrConst::I64(displacement)));
        } else {
            args[disp_index] = Operand::Const(IrConst::I64(displacement));
        }
        if store {
            *dest_ptr = None;
        }
    }
    for &(bi, ii, base, delta) in &fma_plans {
        let Instruction::Intrinsic { args, .. } = &mut func.blocks[bi].instructions[ii] else {
            unreachable!()
        };
        args[2] = Operand::Value(base);
        args.push(Operand::Const(IrConst::I64(delta)));
    }
    plans.len() + fma_plans.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::reexports::{BasicBlock, BlockId, IntrinsicOp, Terminator};

    fn gep(dest: u32, base: u32, offset: IrConst) -> Instruction {
        Instruction::GetElementPtr {
            dest: Value(dest),
            base: Value(base),
            offset: Operand::Const(offset),
            ty: IrType::F64,
        }
    }
    fn load(base: u32, index: Operand, disp: IrConst) -> Instruction {
        Instruction::Intrinsic {
            dest: Some(Value(90)),
            op: IntrinsicOp::VecLoadF64x2,
            dest_ptr: None,
            args: vec![Operand::Value(Value(base)), index, Operand::Const(disp)],
        }
    }
    fn fixture(instructions: Vec<Instruction>) -> IrFunction {
        let mut f = IrFunction::new("addr".into(), IrType::Void, vec![], false);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        f
    }
    fn arguments(f: &IrFunction) -> &[Operand] {
        let Instruction::Intrinsic { args, .. } = f.blocks[0].instructions.last().unwrap() else {
            panic!()
        };
        args
    }
    #[test]
    fn hoisted_fma_offsets_are_normalized_before_allocation_only_at_pointer_width() {
        for ty in [IrType::I32, IrType::U32, IrType::I64, IrType::U64] {
            for delta in [i64::from(i32::MIN), 0, 32, i64::from(i32::MAX), i64::MAX] {
                for mutable in [false, true] {
                    let mut instructions = vec![Instruction::Cast {
                        dest: Value(0),
                        src: Operand::Const(IrConst::I32(0)),
                        from_ty: IrType::I32,
                        to_ty: IrType::I64,
                    }];
                    if mutable {
                        instructions.push(Instruction::Copy {
                            dest: Value(0),
                            src: Operand::Const(IrConst::I64(1)),
                        });
                    }
                    instructions.push(Instruction::BinOp {
                        dest: Value(1),
                        op: IrBinOp::Add,
                        lhs: Operand::Value(Value(0)),
                        rhs: Operand::Const(IrConst::I64(delta)),
                        ty,
                    });
                    instructions.push(Instruction::Intrinsic {
                        dest: None,
                        dest_ptr: None,
                        op: IntrinsicOp::FmaF64x4HoistedSIB,
                        args: vec![
                            Operand::Value(Value(10)),
                            Operand::Value(Value(11)),
                            Operand::Value(Value(1)),
                        ],
                    });
                    let mut f = fixture(instructions);
                    let allowed = matches!(ty, IrType::I64 | IrType::U64)
                        && i32::try_from(delta).is_ok()
                        && !mutable;
                    assert_eq!(run(&mut f), usize::from(allowed));
                    assert_eq!(
                        arguments(&f)[2],
                        Operand::Value(Value(if allowed { 0 } else { 1 }))
                    );
                    assert_eq!(arguments(&f).len(), if allowed { 4 } else { 3 });
                    assert_eq!(run(&mut f), 0);
                }
            }
        }
    }

    #[test]
    fn chained_offsets_and_constant_index_share_one_displacement() {
        let mut f = fixture(vec![
            gep(1, 0, IrConst::I64(56)),
            gep(2, 1, IrConst::I64(-8)),
            load(2, Operand::Const(IrConst::I32(16)), IrConst::I64(8)),
        ]);
        assert_eq!(run(&mut f), 1);
        assert_eq!(
            arguments(&f),
            &[
                Operand::Value(Value(0)),
                Operand::Const(IrConst::I64(0)),
                Operand::Const(IrConst::I64(72))
            ]
        );
        assert_eq!(run(&mut f), 0, "canonicalization must be idempotent");
    }
    #[test]
    fn variable_index_and_negative_disp32_boundary_survive() {
        let mut f = fixture(vec![
            gep(1, 0, IrConst::I64(i32::MIN as i64)),
            load(1, Operand::Value(Value(50)), IrConst::I64(0)),
        ]);
        assert_eq!(run(&mut f), 1);
        assert_eq!(arguments(&f)[1], Operand::Value(Value(50)));
        assert_eq!(
            arguments(&f)[2],
            Operand::Const(IrConst::I64(i32::MIN as i64))
        );
    }
    #[test]
    fn unencodable_or_unrepresentable_displacements_are_rejected() {
        for offset in [
            IrConst::I64(i32::MAX as i64 + 1),
            IrConst::I64(i32::MIN as i64 - 1),
            IrConst::I128(1i128 << 80),
        ] {
            let mut f = fixture(vec![
                gep(1, 0, offset),
                load(1, Operand::Const(IrConst::I64(0)), IrConst::I64(0)),
            ]);
            assert_eq!(run(&mut f), 0);
        }
        for (index, displacement) in [
            (IrConst::I128(1i128 << 80), IrConst::I64(0)),
            (IrConst::I64(0), IrConst::I128(1i128 << 80)),
            (IrConst::I64(1), IrConst::I64(i32::MAX as i64)),
        ] {
            let mut f = fixture(vec![
                gep(1, 0, IrConst::I64(8)),
                load(1, Operand::Const(index), displacement),
            ]);
            assert_eq!(run(&mut f), 0);
        }
    }
    #[test]
    fn checked_chain_addition_rejects_overflow() {
        let mut f = fixture(vec![
            gep(1, 0, IrConst::I64(i64::MAX)),
            gep(2, 1, IrConst::I64(1)),
            load(2, Operand::Const(IrConst::I64(0)), IrConst::I64(0)),
        ]);
        assert_eq!(run(&mut f), 0);
    }
    #[test]
    fn cyclic_and_multidef_roots_cannot_be_rematerialized() {
        for instructions in [
            vec![gep(1, 2, IrConst::I64(8)), gep(2, 1, IrConst::I64(8))],
            vec![gep(1, 0, IrConst::I64(8)), gep(1, 0, IrConst::I64(16))],
            vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I64(1)),
                },
                gep(1, 0, IrConst::I64(8)),
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I64(2)),
                },
            ],
        ] {
            let mut f = fixture(instructions);
            f.blocks[0].instructions.push(load(
                1,
                Operand::Const(IrConst::I64(0)),
                IrConst::I64(0),
            ));
            assert_eq!(run(&mut f), 0);
        }
    }
    #[test]
    fn argument_store_remains_effectful_after_legacy_pointer_is_removed() {
        let mut f = fixture(vec![
            gep(1, 0, IrConst::I64(56)),
            Instruction::Intrinsic {
                dest: None,
                op: IntrinsicOp::VecStoreF64x2,
                dest_ptr: Some(Value(1)),
                args: vec![
                    Operand::Value(Value(50)),
                    Operand::Value(Value(1)),
                    Operand::Const(IrConst::I64(0)),
                ],
            },
        ]);
        assert_eq!(run(&mut f), 1);
        assert!(matches!(
            f.blocks[0].instructions[1],
            Instruction::Intrinsic { dest_ptr: None, .. }
        ));
        super::super::dce::eliminate_dead_code(&mut f);
        assert_eq!(f.blocks[0].instructions.len(), 1);
        assert!(matches!(
            f.blocks[0].instructions[0],
            Instruction::Intrinsic {
                op: IntrinsicOp::VecStoreF64x2,
                ..
            }
        ));
        assert_eq!(arguments(&f)[1], Operand::Value(Value(0)));
    }
    #[test]
    fn malformed_arity_and_variable_displacement_fail_closed() {
        for extra in [Operand::Value(Value(20)), Operand::Const(IrConst::I64(0))] {
            let mut f = fixture(vec![
                gep(1, 0, IrConst::I64(8)),
                load(1, Operand::Const(IrConst::I64(0)), IrConst::I64(0)),
            ]);
            let Instruction::Intrinsic { args, .. } = &mut f.blocks[0].instructions[1] else {
                unreachable!()
            };
            if matches!(extra, Operand::Value(_)) {
                args[2] = extra;
            } else {
                args.push(extra);
            }
            assert_eq!(run(&mut f), 0);
        }
    }
}
