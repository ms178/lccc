use super::*;
use crate::ir::reexports::{BasicBlock, BlockId, Terminator};

fn v(n: u32) -> Operand {
    Operand::Value(Value(n))
}
fn fixture(
    ty: IrType,
    subtract: bool,
    swap: bool,
    op: IrCmpOp,
    zero_left: bool,
) -> (Vec<Option<Instruction>>, Instruction) {
    let mut defs = vec![None; 8];
    defs[2] = Some(if subtract {
        Instruction::BinOp {
            dest: Value(2),
            op: IrBinOp::Sub,
            lhs: Operand::Const(IrConst::from_i64(0, ty)),
            rhs: v(0),
            ty,
        }
    } else {
        Instruction::UnaryOp {
            dest: Value(2),
            op: IrUnaryOp::Neg,
            src: v(0),
            ty,
        }
    });
    defs[3] = Some(Instruction::Select {
        dest: Value(3),
        cond: v(1),
        true_val: if swap { v(0) } else { v(2) },
        false_val: if swap { v(2) } else { v(0) },
        ty,
    });
    let zero = Operand::Const(IrConst::from_i64(0, ty));
    let cmp = Instruction::Cmp {
        dest: Value(4),
        op,
        lhs: if zero_left { zero } else { v(3) },
        rhs: if zero_left { v(3) } else { zero },
        ty,
    };
    (defs, cmp)
}

#[test]
fn actual_fold_exhaustive_low_words_high_bits_both_polarities() {
    for ty in [IrType::I32, IrType::U32, IrType::I64, IrType::U64] {
        let width = ty.size() * 8;
        let mask = if width == 64 {
            u64::MAX
        } else {
            u32::MAX as u64
        };
        for sub in [false, true] {
            for swap in [false, true] {
                for zero_left in [false, true] {
                    for op in [IrCmpOp::Eq, IrCmpOp::Ne] {
                        let (defs, cmp) = fixture(ty, sub, swap, op, zero_left);
                        let result =
                            fold_sign_choice_zero(&cmp, &defs).expect("missing production fold");
                        assert!(
                            matches!(result,Instruction::Cmp { lhs:Operand::Value(Value(0)), op:o, ty:t, .. } if o==op && t==ty)
                        );
                        for low in 0..=u16::MAX as u64 {
                            for high in [0, 1u64 << (width - 1)] {
                                let x = low | high;
                                for c in [false, true] {
                                    let original =
                                        if c ^ swap { x.wrapping_neg() & mask } else { x };
                                    let before = if op == IrCmpOp::Eq {
                                        original == 0
                                    } else {
                                        original != 0
                                    };
                                    let after = if op == IrCmpOp::Eq { x == 0 } else { x != 0 };
                                    assert_eq!(before, after, "{ty:?} x={x} c={c}");
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn rejects_non_equalities_float_mismatched_types_roots_and_casts() {
    for variant in 0..8 {
        let (mut defs, mut cmp) = fixture(IrType::U64, false, false, IrCmpOp::Eq, false);
        match variant {
            0 => {
                if let Instruction::Cmp { op, .. } = &mut cmp {
                    *op = IrCmpOp::Ugt
                }
            }
            1 => {
                if let Instruction::Cmp { ty, .. } = &mut cmp {
                    *ty = IrType::F64
                }
            }
            2 => {
                if let Some(Instruction::UnaryOp { ty, .. }) = &mut defs[2] {
                    *ty = IrType::U32
                }
            }
            3 => {
                if let Some(Instruction::Select { ty, .. }) = &mut defs[3] {
                    *ty = IrType::U32
                }
            }
            4 => {
                if let Some(Instruction::Select { false_val, .. }) = &mut defs[3] {
                    *false_val = v(7)
                }
            }
            5 => {
                if let Some(Instruction::UnaryOp { op, .. }) = &mut defs[2] {
                    *op = IrUnaryOp::Not
                }
            }
            6 => {
                if let Instruction::Cmp { rhs, .. } = &mut cmp {
                    *rhs = Operand::Const(IrConst::I64(1))
                }
            }
            7 => {
                defs[6] = Some(Instruction::Cast {
                    dest: Value(6),
                    src: v(0),
                    from_ty: IrType::U64,
                    to_ty: IrType::U32,
                });
                if let Some(Instruction::Select { false_val, .. }) = &mut defs[3] {
                    *false_val = v(6)
                }
            }
            _ => unreachable!(),
        }
        assert!(
            fold_sign_choice_zero(&cmp, &defs).is_none(),
            "variant {variant}"
        );
    }
}

#[test]
fn copy_chains_are_transparent_without_new_ssa_or_producer_edits() {
    let (mut defs, cmp) = fixture(IrType::U64, false, false, IrCmpOp::Ne, false);
    defs[5] = Some(Instruction::Copy {
        dest: Value(5),
        src: v(0),
    });
    if let Some(Instruction::Select { false_val, .. }) = &mut defs[3] {
        *false_val = v(5)
    }
    assert!(fold_sign_choice_zero(&cmp, &defs).is_some());
    let mut f = IrFunction::new("choice".into(), IrType::I32, vec![], false);
    let mut insts = vec![
        Instruction::Copy {
            dest: Value(0),
            src: Operand::Const(IrConst::I64(7)),
        },
        Instruction::Copy {
            dest: Value(1),
            src: Operand::Const(IrConst::I32(1)),
        },
    ];
    insts.extend([
        defs[2].clone().unwrap(),
        defs[5].clone().unwrap(),
        defs[3].clone().unwrap(),
        cmp,
    ]);
    let original = insts.clone();
    f.blocks.push(BasicBlock {
        label: BlockId(0),
        instructions: insts,
        source_spans: vec![],
        terminator: Terminator::Return(Some(v(4))),
    });
    f.next_value_id = 8;
    f.next_label = 1;
    assert_eq!(recognize_function(&mut f, false, 0, 0, false, true), 1);
    assert_eq!(
        format!("{:?}", &f.blocks[0].instructions[..5]),
        format!("{:?}", &original[..5])
    );
    assert_eq!(f.next_value_id, 8);
    assert_eq!(recognize_function(&mut f, false, 0, 0, false, true), 0);
}
