//! Instruction-local FP legality classification shared by optimizer passes.
use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrConst, IrFunction, IrUnaryOp, Operand};

/// Conservatively classify operations that may observe or affect FP state.
/// Unknown intrinsics remain observable. This is not a claim that loads or
/// all conversions trap on every target: declining that optimization is safe.
pub(super) fn observes_fp_environment(inst: &Instruction) -> bool {
    inst.result_type().is_some_and(|ty| ty.is_float())
        || matches!(inst, Instruction::Cmp { ty, .. } | Instruction::Store { ty, .. } if ty.is_float())
        || matches!(inst, Instruction::Cast { from_ty, .. } if from_ty.is_float())
        || matches!(inst, Instruction::Intrinsic { .. })
        || matches!(inst, Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. }
            if info.arg_types.iter().any(|ty| ty.is_float()))
}

/// Passes without instruction-local legality proofs retain the whole-function
/// fence. FENV-off functions incur no scan.
pub(super) fn fp_environment_blocks_reordering(func: &IrFunction) -> bool {
    func.pass_policy.fp_environment_observable
        && func
            .blocks
            .iter()
            .any(|b| b.instructions.iter().any(observes_fp_environment))
}

/// A positive whitelist, not an assumption that every non-FP result is safe:
/// comparisons/conversions can have integer results and still raise FP flags.
/// Selects are intentionally not admitted until their condition's type is
/// proved. Besides ordinary integer arithmetic/address folding, permit the
/// exact sign change of a literal binary32/64 zero: no rounding, invalid,
/// overflow, underflow or inexact event is possible, including on x87. This
/// exposes -0.0 recurrence seeds without admitting general FP evaluation.
pub(super) fn local_fold_is_fenv_safe(inst: &Instruction) -> bool {
    match inst {
        Instruction::UnaryOp {
            op: IrUnaryOp::Neg,
            src: Operand::Const(IrConst::F32(x)),
            ty: IrType::F32,
            ..
        } => x.to_bits() & 0x7fff_ffff == 0,
        Instruction::UnaryOp {
            op: IrUnaryOp::Neg,
            src: Operand::Const(IrConst::F64(x)),
            ty: IrType::F64,
            ..
        } => x.to_bits() & 0x7fff_ffff_ffff_ffff == 0,
        Instruction::BinOp { ty, .. }
        | Instruction::UnaryOp { ty, .. }
        | Instruction::Cmp { ty, .. } => ty.is_integer(),
        Instruction::Cast { from_ty, to_ty, .. } => !from_ty.is_float() && !to_ty.is_float(),
        Instruction::GetElementPtr { .. } => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{pass_policy::PassPolicy, types::IrType};
    use crate::ir::reexports::*;

    fn function(
        name: &str,
        fenv: bool,
        instructions: Vec<Instruction>,
        result: Option<Operand>,
    ) -> IrFunction {
        let mut f = IrFunction::new(name.into(), IrType::I32, vec![], false);
        f.is_static = true;
        f.pass_policy = PassPolicy::new(false, fenv);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            source_spans: vec![],
            terminator: Terminator::Return(result),
        });
        f.next_value_id = 32;
        f.next_label = 1;
        f
    }
    fn integer(dest: u32) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op: IrBinOp::Add,
            lhs: Operand::Const(IrConst::I32(2)),
            rhs: Operand::Const(IrConst::I32(3)),
            ty: IrType::I32,
        }
    }
    fn floating(dest: u32) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op: IrBinOp::SDiv,
            lhs: Operand::Const(IrConst::F64(1.0)),
            rhs: Operand::Const(IrConst::F64(0.0)),
            ty: IrType::F64,
        }
    }
    fn call(name: &str, dest: Option<Value>, return_type: IrType) -> Instruction {
        Instruction::Call {
            func: name.into(),
            info: CallInfo {
                dest,
                return_type,
                ..Default::default()
            },
        }
    }

    #[test]
    fn fenv_dce_roots_float_effects_not_unrelated_integer_work() {
        let mut f = function(
            "mixed",
            true,
            vec![
                integer(0),
                floating(1),
                Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::F64(1.0)),
                    ty: IrType::F64,
                },
                Instruction::Cast {
                    dest: Value(3),
                    src: Operand::Value(Value(1)),
                    from_ty: IrType::F64,
                    to_ty: IrType::I32,
                },
                integer(4),
                Instruction::Cast {
                    dest: Value(5),
                    src: Operand::Value(Value(4)),
                    from_ty: IrType::I32,
                    to_ty: IrType::F64,
                },
            ],
            None,
        );
        assert_eq!(crate::passes::dce::eliminate_dead_code(&mut f), 1);
        assert_eq!(
            f.blocks[0]
                .instructions
                .iter()
                .filter_map(Instruction::dest)
                .collect::<Vec<_>>(),
            (1..=5).map(Value).collect::<Vec<_>>()
        );
        f.pass_policy.fp_environment_observable = false;
        assert_eq!(crate::passes::dce::eliminate_dead_code(&mut f), 5);
    }

    #[test]
    fn fenv_folding_keeps_fp_comparisons_and_both_conversion_directions() {
        let mut f = function(
            "mixed",
            true,
            vec![
                integer(0),
                floating(1),
                Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Eq,
                    lhs: Operand::Const(IrConst::F64(0.0)),
                    rhs: Operand::Const(IrConst::F64(0.0)),
                    ty: IrType::F64,
                },
                Instruction::Cast {
                    dest: Value(3),
                    src: Operand::Const(IrConst::F64(1.5)),
                    from_ty: IrType::F64,
                    to_ty: IrType::I32,
                },
                Instruction::Cast {
                    dest: Value(4),
                    src: Operand::Const(IrConst::I64(9007199254740993)),
                    from_ty: IrType::I64,
                    to_ty: IrType::F64,
                },
            ],
            Some(Operand::Value(Value(0))),
        );
        let fp_before = format!("{:?}", &f.blocks[0].instructions[1..]);
        let mut sparse = f.clone();
        assert!(crate::passes::sccp::run_function(&mut sparse) > 0);
        assert_eq!(
            format!("{:?}", &sparse.blocks[0].instructions[1..]),
            fp_before
        );
        assert_eq!(crate::passes::constant_fold::fold_function(&mut f), 1);
        assert!(matches!(
            f.blocks[0].instructions[0],
            Instruction::Copy {
                src: Operand::Const(IrConst::I32(5)),
                ..
            }
        ));
        assert_eq!(format!("{:?}", &f.blocks[0].instructions[1..]), fp_before);
    }

    #[test]
    fn fenv_interprocedural_calls_are_not_empty_pure_or_constant() {
        for fenv in [false, true] {
            let mut dead_fp = function("dead_fp", fenv, vec![floating(0)], None);
            dead_fp.return_type = IrType::Void;
            let mut constant_fp = function(
                "constant_fp",
                fenv,
                vec![Instruction::BinOp {
                    dest: Value(0),
                    op: IrBinOp::Add,
                    lhs: Operand::Const(IrConst::F64(1.0)),
                    rhs: Operand::Const(IrConst::F64(1.1102230246251565e-16)),
                    ty: IrType::F64,
                }],
                Some(Operand::Value(Value(0))),
            );
            constant_fp.return_type = IrType::F64;
            let integer_fn = function(
                "integer",
                fenv,
                vec![integer(0)],
                Some(Operand::Value(Value(0))),
            );
            let caller = function(
                "caller",
                fenv,
                vec![
                    call("dead_fp", None, IrType::Void),
                    call("constant_fp", Some(Value(1)), IrType::F64),
                    call("integer", Some(Value(2)), IrType::I32),
                ],
                Some(Operand::Value(Value(2))),
            );
            let mut m = IrModule::new();
            m.functions = vec![dead_fp, constant_fp, integer_fn, caller];
            crate::passes::ip_purity::run(&mut m);
            for inst in &m.functions[3].blocks[0].instructions[..2] {
                let Instruction::Call { info, .. } = inst else {
                    panic!("missing call")
                };
                assert_eq!(info.is_pure, !fenv);
                assert_eq!(info.is_const, !fenv);
            }
            crate::passes::ipcp::run(&mut m);
            let calls = m.functions[3].blocks[0]
                .instructions
                .iter()
                .filter(|i| matches!(i, Instruction::Call { .. }))
                .count();
            assert_eq!(calls, if fenv { 2 } else { 0 });
            assert!(matches!(
                m.functions[3].blocks[0].instructions.last(),
                Some(Instruction::Copy {
                    src: Operand::Const(IrConst::I32(5)),
                    ..
                })
            ));
        }
    }

    #[test]
    fn fenv_literal_zero_negation_is_exact_but_general_float_folding_stays_closed() {
        for (c, ty) in [
            (IrConst::F32(0.0), IrType::F32),
            (IrConst::F32(-0.0), IrType::F32),
            (IrConst::F64(0.0), IrType::F64),
            (IrConst::F64(-0.0), IrType::F64),
        ] {
            let mut f = function(
                "zero_sign",
                true,
                vec![Instruction::UnaryOp {
                    dest: Value(0),
                    op: IrUnaryOp::Neg,
                    src: Operand::Const(c),
                    ty,
                }],
                Some(Operand::Value(Value(0))),
            );
            f.return_type = ty;
            assert_eq!(crate::passes::constant_fold::fold_function(&mut f), 1);
            let Instruction::Copy {
                src: Operand::Const(folded),
                ..
            } = f.blocks[0].instructions[0]
            else {
                panic!("zero did not fold")
            };
            match (c, folded) {
                (IrConst::F32(a), IrConst::F32(b)) => {
                    assert_eq!(b.to_bits(), a.to_bits() ^ (1 << 31))
                }
                (IrConst::F64(a), IrConst::F64(b)) => {
                    assert_eq!(b.to_bits(), a.to_bits() ^ (1 << 63))
                }
                _ => panic!("zero changed type"),
            }
        }
        // Stay conservative for subnormals, infinities and both NaN classes;
        // the admission test itself uses bits, never an FP comparison.
        for bits in [
            1,
            0x3ff0000000000000,
            0x7ff0000000000000,
            0x7ff0000000000001,
            0x7ff8000000000001,
        ] {
            let inst = Instruction::UnaryOp {
                dest: Value(0),
                op: IrUnaryOp::Neg,
                src: Operand::Const(IrConst::F64(f64::from_bits(bits))),
                ty: IrType::F64,
            };
            assert!(!local_fold_is_fenv_safe(&inst));
            let mut f = function("not_zero", true, vec![inst], Some(Operand::Value(Value(0))));
            f.return_type = IrType::F64;
            assert_eq!(crate::passes::constant_fold::fold_function(&mut f), 0);
        }
    }
}
