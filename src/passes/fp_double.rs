//! Scalar binary32/binary64 doubling without a constant register.
//!
//! Multiplication by positive two and adding a value to itself have the same
//! exact real result, hence the same rounding, signed zero and FP exceptions.
//! This is not reassociation. Restrict the pipeline to x86-64 SSE semantics;
//! x87 excess precision and other backends are deliberately outside its scope.
//! Keep multiply/add contraction opportunities, including through copy chains.
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, IrFunction, Operand};

/// x*2 and x+x have identical IEEE binary32/binary64 results, signed zero,
/// rounding and exceptions. Leave potential multiply-add contractions alone.
pub(crate) fn run(func: &mut IrFunction) -> usize {
    let mut contraction_inputs = FxHashSet::default();
    for b in &func.blocks {
        for i in &b.instructions {
            if let Instruction::BinOp {
                op: IrBinOp::Add | IrBinOp::Sub,
                lhs,
                rhs,
                ty,
                ..
            } = i
                && matches!(ty, IrType::F32 | IrType::F64)
            {
                for op in [lhs, rhs] {
                    if let Operand::Value(v) = op {
                        contraction_inputs.insert(v.0);
                    }
                }
            }
        }
    }
    let copies: FxHashMap<_, _> = func
        .blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .filter_map(|i| match i {
            Instruction::Copy {
                dest,
                src: Operand::Value(v),
            } => Some((dest.0, v.0)),
            _ => None,
        })
        .collect();
    let mut work: Vec<_> = contraction_inputs.iter().copied().collect();
    while let Some(v) = work.pop() {
        if let Some(&src) = copies.get(&v)
            && contraction_inputs.insert(src)
        {
            work.push(src);
        }
    }
    let mut changes = 0;
    for b in &mut func.blocks {
        for i in &mut b.instructions {
            let Instruction::BinOp {
                dest,
                op: IrBinOp::Mul,
                lhs,
                rhs,
                ty,
            } = i
            else {
                continue;
            };
            if contraction_inputs.contains(&dest.0) {
                continue;
            }
            let two = |op: Operand| {
                matches!(
                    (op, *ty),
                    (Operand::Const(IrConst::F32(2.0)), IrType::F32)
                        | (Operand::Const(IrConst::F64(2.0)), IrType::F64)
                )
            };
            let value = if two(*lhs) {
                *rhs
            } else if two(*rhs) {
                *lhs
            } else {
                continue;
            };
            *i = Instruction::BinOp {
                dest: *dest,
                op: IrBinOp::Add,
                lhs: value,
                rhs: value,
                ty: *ty,
            };
            changes += 1;
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator, Value};
    fn v(n: u32) -> Operand {
        Operand::Value(Value(n))
    }
    fn fixture(ty: IrType, constant: IrConst, left: bool) -> IrFunction {
        let mut f = IrFunction::new("double".into(), ty, vec![], false);
        let (lhs, rhs) = if left {
            (Operand::Const(constant), v(1))
        } else {
            (v(1), Operand::Const(constant))
        };
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            source_spans: vec![],
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 0,
                    ty,
                },
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Mul,
                    lhs,
                    rhs,
                    ty,
                },
            ],
            terminator: Terminator::Return(Some(v(2))),
        });
        f
    }
    #[test]
    fn doubles_both_operand_orders_and_precisions_without_new_values() {
        for (ty, c) in [
            (IrType::F32, IrConst::F32(2.0)),
            (IrType::F64, IrConst::F64(2.0)),
        ] {
            for left in [false, true] {
                let mut f = fixture(ty, c, left);
                assert_eq!(run(&mut f), 1);
                assert!(matches!(f.blocks[0].instructions[1], Instruction::BinOp {
                    dest: Value(2), op: IrBinOp::Add, lhs, rhs, ty: t
                } if lhs == v(1) && rhs == v(1) && t == ty));
                assert_eq!(f.next_value_id, 0);
                assert_eq!(run(&mut f), 0);
            }
        }
    }
    #[test]
    fn preserves_contractions_through_arbitrarily_ordered_copy_chains() {
        for op in [IrBinOp::Add, IrBinOp::Sub] {
            for left in [false, true] {
                let mut f = fixture(IrType::F64, IrConst::F64(2.0), left);
                for n in 3..=35 {
                    f.blocks[0].instructions.push(Instruction::Copy {
                        dest: Value(n),
                        src: v(n - 1),
                    });
                }
                f.blocks[0].instructions.push(Instruction::BinOp {
                    dest: Value(36),
                    op,
                    lhs: v(35),
                    rhs: v(1),
                    ty: IrType::F64,
                });
                f.blocks[0].terminator = Terminator::Return(Some(v(36)));
                let before = format!("{f:?}");
                assert_eq!(run(&mut f), 0);
                assert_eq!(format!("{f:?}"), before);
            }
        }
    }
    #[test]
    fn refuses_non_two_wrong_carrier_width_integer_and_extended_precision() {
        for (ty, c) in [
            (IrType::F32, IrConst::F64(2.0)),
            (IrType::F64, IrConst::F32(2.0)),
            (IrType::F64, IrConst::F64(-2.0)),
            (IrType::F64, IrConst::F64(f64::NAN)),
            (IrType::F64, IrConst::F64(2.0000000000000004)),
            (IrType::I32, IrConst::I32(2)),
            (IrType::F128, IrConst::F64(2.0)),
        ] {
            let mut f = fixture(ty, c, false);
            let before = format!("{f:?}");
            assert_eq!(run(&mut f), 0);
            assert_eq!(format!("{f:?}"), before);
        }
    }
}
