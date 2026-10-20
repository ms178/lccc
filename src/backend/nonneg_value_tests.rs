use super::*;

fn v(n: u32) -> Operand {
    Operand::Value(Value(n))
}
fn c(n: i32) -> Operand {
    Operand::Const(IrConst::I32(n))
}
fn binary(dest: u32, op: IrBinOp, lhs: Operand, rhs: Operand, ty: IrType) -> Instruction {
    Instruction::BinOp {
        dest: Value(dest),
        op,
        lhs,
        rhs,
        ty,
    }
}
fn fixture(mut insts: Vec<Instruction>) -> IrFunction {
    insts.insert(
        0,
        Instruction::ParamRef {
            dest: Value(0),
            param_idx: 0,
            ty: IrType::I32,
        },
    );
    let mut f = IrFunction::new("nonneg_contract".into(), IrType::I64, vec![], false);
    f.blocks.push(BasicBlock {
        label: crate::ir::reexports::BlockId(0),
        instructions: insts,
        terminator: Terminator::Return(None),
        source_spans: vec![],
    });
    f
}

#[test]
fn bitwise_and_shift_transfer_rules_fail_closed_at_sign_boundaries() {
    let mut insts = vec![binary(1, IrBinOp::And, v(0), c(255), IrType::I32)];
    let mut expected = vec![1];
    let mut next = 2;
    for op in [
        IrBinOp::And,
        IrBinOp::Or,
        IrBinOp::Xor,
        IrBinOp::AShr,
        IrBinOp::UDiv,
        IrBinOp::URem,
    ] {
        let rhs = if matches!(op, IrBinOp::AShr | IrBinOp::UDiv | IrBinOp::URem) {
            c(1)
        } else {
            v(1)
        };
        insts.push(binary(next, op, v(1), rhs, IrType::I32));
        expected.push(next);
        next += 1;
    }
    for amount in [-1, 0, 1, 31, 32, 33] {
        insts.push(binary(next, IrBinOp::LShr, v(0), c(amount), IrType::I32));
        if amount == 1 || amount == 31 {
            expected.push(next);
        }
        next += 1;
    }
    insts.extend([
        binary(next, IrBinOp::Or, v(0), v(1), IrType::I32),
        binary(next + 1, IrBinOp::Xor, v(0), v(1), IrType::I32),
        binary(next + 2, IrBinOp::And, v(0), c(i32::MIN), IrType::I32),
        binary(next + 3, IrBinOp::And, v(0), c(255), IrType::I64),
        binary(next + 4, IrBinOp::LShr, v(1), c(0), IrType::I32),
        binary(next + 5, IrBinOp::And, v(0), c(i32::MAX), IrType::I32),
    ]);
    expected.extend([next + 4, next + 5]);
    let facts = collect_bitop_nonneg_values(&fixture(insts));
    for id in 0..=next + 5 {
        assert_eq!(facts.contains(&id), expected.contains(&id), "value {id}");
    }
}

#[test]
fn casts_selects_and_ambiguous_copies_obey_their_producer_contracts() {
    let f = fixture(vec![
        Instruction::Cast {
            dest: Value(1),
            src: v(0),
            from_ty: IrType::U8,
            to_ty: IrType::I32,
        },
        Instruction::Cast {
            dest: Value(2),
            src: v(0),
            from_ty: IrType::I8,
            to_ty: IrType::I32,
        },
        Instruction::Copy {
            dest: Value(3),
            src: v(1),
        },
        Instruction::Select {
            dest: Value(4),
            cond: v(0),
            true_val: v(3),
            false_val: c(255),
            ty: IrType::I32,
        },
        Instruction::Select {
            dest: Value(5),
            cond: v(0),
            true_val: v(3),
            false_val: c(-1),
            ty: IrType::I32,
        },
        Instruction::Copy {
            dest: Value(6),
            src: v(1),
        },
        Instruction::Copy {
            dest: Value(6),
            src: v(0),
        },
        Instruction::Copy {
            dest: Value(7),
            src: v(6),
        },
    ]);
    let facts = collect_bitop_nonneg_values(&f);
    for id in 0..8 {
        assert_eq!(facts.contains(&id), [1, 3, 4].contains(&id), "value {id}");
    }
}

#[test]
fn reverse_order_chains_reach_fixpoint_without_an_iteration_cap() {
    let mut insts = Vec::new();
    for id in (2..1000).rev() {
        insts.push(Instruction::Copy {
            dest: Value(id),
            src: v(id - 1),
        });
    }
    insts.push(binary(1, IrBinOp::And, v(0), c(255), IrType::I32));
    insts.push(Instruction::Copy {
        dest: Value(1000),
        src: v(1001),
    });
    insts.push(Instruction::Copy {
        dest: Value(1001),
        src: v(1000),
    });
    let facts = collect_bitop_nonneg_values(&fixture(insts));
    assert_eq!(facts.len(), 999);
    for id in 1..1000 {
        assert!(facts.contains(&id));
    }
    assert!(!facts.contains(&1000));
    assert!(!facts.contains(&1001));
}
