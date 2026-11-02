//! Late scalar unsigned-saturating-subtract canonicalization.
//!
//! This runs after the vectorizers.  Their packed U16 lowering therefore keeps
//! its original select/min-max input, while scalar remainders and targets
//! without the x86-64 vector-SSA backend (notably i686) can still use one
//! exact scalar intrinsic.  The rewrite is deliberately limited to a
//! single-use compare/subtract/select graph; unrelated readers keep the
//! original graph instead of paying for a second computation.

use crate::common::fx_hash::FxHashMap;
use crate::common::types::IrType;
use crate::ir::reexports::{
    Instruction, IntrinsicOp, IrBinOp, IrCmpOp, IrFunction, Operand, Value,
};

/// Prove the signed and unsigned 32-bit orders identical for one operand.
///
/// A zero extension from U8/U16 produces [0, 255] or [0, 65535].  Constants
/// are accepted only in the signed-nonnegative 32-bit domain.  In particular,
/// a same-width U32 -> I32 reinterpretation is NOT a proof: its high bit may
/// make the signed and unsigned orders disagree.
fn nonnegative_i32(
    func: &IrFunction,
    defs: &FxHashMap<u32, (usize, usize)>,
    operand: &Operand,
    depth: usize,
) -> bool {
    if depth > 8 {
        return false;
    }
    match operand {
        Operand::Const(c) => c
            .to_i64()
            .is_some_and(|v| (0..=i64::from(i32::MAX)).contains(&v)),
        Operand::Value(v) => {
            let Some(&(bi, ii)) = defs.get(&v.0) else {
                return false;
            };
            match &func.blocks[bi].instructions[ii] {
                Instruction::Cast {
                    from_ty: IrType::U8 | IrType::U16,
                    to_ty: IrType::I32 | IrType::U32,
                    ..
                } => true,
                Instruction::Copy { src, .. } => nonnegative_i32(func, defs, src, depth + 1),
                _ => false,
            }
        }
    }
}

fn zero(operand: &Operand) -> bool {
    matches!(operand, Operand::Const(c) if c.to_i64() == Some(0))
}

fn zero_extension_of(
    func: &IrFunction,
    defs: &FxHashMap<u32, (usize, usize)>,
    wide: &Operand,
    narrow: &Operand,
    narrow_ty: IrType,
    wide_ty: IrType,
) -> bool {
    let Operand::Value(v) = wide else {
        return false;
    };
    let Some(&(bi, ii)) = defs.get(&v.0) else {
        return false;
    };
    matches!(
        &func.blocks[bi].instructions[ii],
        Instruction::Cast { src, from_ty, to_ty, .. }
            if src == narrow && *from_ty == narrow_ty && *to_ty == wide_ty
    )
}

fn replacement(
    func: &IrFunction,
    defs: &FxHashMap<u32, (usize, usize)>,
    uses: &FxHashMap<u32, usize>,
    inst: &Instruction,
) -> Option<Instruction> {
    let Instruction::Select {
        dest,
        cond,
        true_val,
        false_val,
        ty,
    } = inst
    else {
        return None;
    };
    if !matches!(ty, IrType::I32 | IrType::U32) {
        return None;
    }
    let Operand::Value(cond_value) = cond else {
        return None;
    };
    if uses.get(&cond_value.0) != Some(&1) {
        return None;
    }
    let &(cmp_bi, cmp_ii) = defs.get(&cond_value.0)?;
    let Instruction::Cmp {
        op,
        lhs: cmp_lhs,
        rhs: cmp_rhs,
        ty: cmp_ty,
        ..
    } = &func.blocks[cmp_bi].instructions[cmp_ii]
    else {
        return None;
    };
    if !matches!(cmp_ty, IrType::I32 | IrType::U32) {
        return None;
    }

    // GE/GT with the subtraction in the true arm, or LE/LT with it in the
    // false arm.  Strictness does not matter at equality because a-a is the
    // same zero as the fallback.
    let (sub_operand, forward) = if zero(false_val) {
        (true_val, true)
    } else if zero(true_val) {
        (false_val, false)
    } else {
        return None;
    };
    let Operand::Value(result_value) = sub_operand else {
        return None;
    };
    if uses.get(&result_value.0) != Some(&1) {
        return None;
    }
    let &(result_bi, result_ii) = defs.get(&result_value.0)?;
    let (sub_value, narrow_result) = match &func.blocks[result_bi].instructions[result_ii] {
        Instruction::BinOp {
            op: IrBinOp::Sub, ..
        } => (*result_value, false),
        Instruction::Cast {
            src: Operand::Value(v),
            from_ty: IrType::U8 | IrType::U16,
            to_ty,
            ..
        } if to_ty == ty => (*v, true),
        _ => return None,
    };
    if narrow_result && uses.get(&sub_value.0) != Some(&1) {
        return None;
    }
    let &(sub_bi, sub_ii) = defs.get(&sub_value.0)?;
    let Instruction::BinOp {
        op: IrBinOp::Sub,
        lhs: sub_lhs,
        rhs: sub_rhs,
        ty: sub_ty,
        ..
    } = &func.blocks[sub_bi].instructions[sub_ii]
    else {
        return None;
    };

    // Direct form: compare and subtract consume the exact same 32-bit
    // operands.  Narrow form: zext(sub_uN(...)) is selected, and every
    // compare operand is the proven zext of its corresponding narrow
    // operand.  The intrinsic ALWAYS receives the 32-bit compare operands;
    // a backend never has to guess whether a narrow stack home is 32-bit
    // readable.  Both the subtrahend and minuend are checked, so a merely
    // similar clamp cannot become saturation.
    let operands_match = if narrow_result {
        matches!(sub_ty, IrType::U8 | IrType::U16)
            && zero_extension_of(func, defs, cmp_lhs, sub_lhs, *sub_ty, *cmp_ty)
            && zero_extension_of(func, defs, cmp_rhs, sub_rhs, *sub_ty, *cmp_ty)
    } else {
        matches!(sub_ty, IrType::I32 | IrType::U32) && sub_lhs == cmp_lhs && sub_rhs == cmp_rhs
    };
    if !operands_match {
        return None;
    }

    let signed = if forward {
        match op {
            IrCmpOp::Sgt | IrCmpOp::Sge => true,
            IrCmpOp::Ugt | IrCmpOp::Uge => false,
            _ => return None,
        }
    } else {
        match op {
            IrCmpOp::Slt | IrCmpOp::Sle => true,
            IrCmpOp::Ult | IrCmpOp::Ule => false,
            _ => return None,
        }
    };

    // Full-domain U32 needs no range proof when both arithmetic and result
    // are unsigned.  Every signed compare, signed subtraction/result, and
    // narrow-roundtrip form requires the common nonnegative-domain proof:
    // it makes signed/unsigned order identical and excludes I32 overflow.
    let needs_proof = narrow_result || signed || *sub_ty == IrType::I32 || *ty == IrType::I32;
    if needs_proof
        && !(nonnegative_i32(func, defs, cmp_lhs, 0) && nonnegative_i32(func, defs, cmp_rhs, 0))
    {
        return None;
    }

    Some(Instruction::Intrinsic {
        dest: Some(*dest),
        op: IntrinsicOp::UsubSat32,
        dest_ptr: None,
        args: vec![cmp_lhs.clone(), cmp_rhs.clone()],
    })
}

pub(crate) fn run(func: &mut IrFunction) -> usize {
    let mut defs = FxHashMap::default();
    let mut uses = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Some(Value(v)) = inst.dest() {
                defs.insert(v, (bi, ii));
            }
            inst.for_each_used_value(|v| *uses.entry(v).or_insert(0) += 1);
        }
        block
            .terminator
            .for_each_used_value(|v| *uses.entry(v).or_insert(0) += 1);
    }

    let mut replacements = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Some(new_inst) = replacement(func, &defs, &uses, inst) {
                replacements.push((bi, ii, new_inst));
            }
        }
    }
    let changed = replacements.len();
    for (bi, ii, inst) in replacements {
        func.blocks[bi].instructions[ii] = inst;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, IrConst, Terminator};

    fn function(
        value_ty: IrType,
        cmp_op: IrCmpOp,
        narrow_zext: bool,
        shared_compare: bool,
    ) -> IrFunction {
        let mut func = IrFunction::new("sat_test".into(), value_ty, vec![], false);
        let mut insts = vec![
            Instruction::ParamRef {
                dest: Value(0),
                param_idx: 0,
                ty: if narrow_zext { IrType::U16 } else { value_ty },
            },
            Instruction::ParamRef {
                dest: Value(1),
                param_idx: 1,
                ty: if narrow_zext { IrType::U16 } else { value_ty },
            },
        ];
        if narrow_zext {
            insts.push(Instruction::Cast {
                dest: Value(2),
                src: Operand::Value(Value(0)),
                from_ty: IrType::U16,
                to_ty: value_ty,
            });
            insts.push(Instruction::Cast {
                dest: Value(3),
                src: Operand::Value(Value(1)),
                from_ty: IrType::U16,
                to_ty: value_ty,
            });
        } else {
            insts.push(Instruction::Copy {
                dest: Value(2),
                src: Operand::Value(Value(0)),
            });
            insts.push(Instruction::Copy {
                dest: Value(3),
                src: Operand::Value(Value(1)),
            });
        }
        insts.extend([
            Instruction::Cmp {
                dest: Value(4),
                op: cmp_op,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Value(Value(3)),
                ty: value_ty,
            },
            Instruction::BinOp {
                dest: Value(5),
                op: IrBinOp::Sub,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Value(Value(3)),
                ty: value_ty,
            },
            Instruction::Select {
                dest: Value(6),
                cond: Operand::Value(Value(4)),
                true_val: Operand::Value(Value(5)),
                false_val: Operand::Const(IrConst::I32(0)),
                ty: value_ty,
            },
        ]);
        if shared_compare {
            insts.push(Instruction::Copy {
                dest: Value(7),
                src: Operand::Value(Value(4)),
            });
        }
        func.next_value_id = 8;
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: insts,
            terminator: Terminator::Return(Some(Operand::Value(Value(6)))),
            source_spans: vec![],
        });
        func
    }

    fn is_intrinsic(func: &IrFunction) -> bool {
        func.blocks[0].instructions.iter().any(|inst| {
            matches!(
                inst,
                Instruction::Intrinsic {
                    op: IntrinsicOp::UsubSat32,
                    ..
                }
            )
        })
    }

    #[test]
    fn unsigned_full_word_and_proven_narrow_promotion_match() {
        let mut full = function(IrType::U32, IrCmpOp::Uge, false, false);
        assert_eq!(run(&mut full), 1);
        assert!(is_intrinsic(&full));
        for op in [IrCmpOp::Sgt, IrCmpOp::Sge, IrCmpOp::Ugt, IrCmpOp::Uge] {
            let mut narrow = function(IrType::I32, op, true, false);
            assert_eq!(run(&mut narrow), 1, "{op:?}");
            assert!(is_intrinsic(&narrow));
        }
    }

    #[test]
    fn narrow_subtract_roundtrip_uses_the_proven_wide_compare_operands() {
        let mut func = function(IrType::I32, IrCmpOp::Sge, true, false);
        func.return_type = IrType::U32;
        func.blocks[0].instructions[5] = Instruction::BinOp {
            dest: Value(5),
            op: IrBinOp::Sub,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Value(Value(1)),
            ty: IrType::U16,
        };
        func.blocks[0].instructions[6] = Instruction::Cast {
            dest: Value(7),
            src: Operand::Value(Value(5)),
            from_ty: IrType::U16,
            to_ty: IrType::U32,
        };
        func.blocks[0].instructions.insert(
            7,
            Instruction::Select {
                dest: Value(6),
                cond: Operand::Value(Value(4)),
                true_val: Operand::Value(Value(7)),
                false_val: Operand::Const(IrConst::I32(0)),
                ty: IrType::U32,
            },
        );
        assert_eq!(run(&mut func), 1);
        assert!(func.blocks[0].instructions.iter().any(|inst| matches!(
            inst,
            Instruction::Intrinsic {
                op: IntrinsicOp::UsubSat32,
                args,
                ..
            } if args == &[Operand::Value(Value(2)), Operand::Value(Value(3))]
        )));

        // The same narrow subtraction without 32-bit compare operands is
        // deliberately rejected: the backend must never read a narrow home
        // as though it were a 32-bit value.
        let mut direct_narrow = function(IrType::I32, IrCmpOp::Sge, true, false);
        direct_narrow.blocks[0].instructions[4] = Instruction::Cmp {
            dest: Value(4),
            op: IrCmpOp::Uge,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Value(Value(1)),
            ty: IrType::U16,
        };
        assert_eq!(run(&mut direct_narrow), 0);
    }

    #[test]
    fn unproven_signed_order_and_shared_compare_do_not_match() {
        for op in [IrCmpOp::Sge, IrCmpOp::Uge] {
            let mut unproven = function(IrType::I32, op, false, false);
            assert_eq!(run(&mut unproven), 0, "{op:?}");
            assert!(!is_intrinsic(&unproven));
        }
        let mut shared = function(IrType::U32, IrCmpOp::Uge, false, true);
        assert_eq!(run(&mut shared), 0);
        assert!(!is_intrinsic(&shared));
    }
}
