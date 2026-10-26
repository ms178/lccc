use super::*;

fn pattern(ty: IrType, op: IrBinOp, c: u64) -> Vec<Instruction> {
    vec![
        bin(
            Value(2),
            IrBinOp::Xor,
            Operand::Value(Value(0)),
            constant(c, ty),
            ty,
        ),
        bin(
            Value(3),
            IrBinOp::Add,
            Operand::Value(Value(2)),
            constant(1, ty),
            ty,
        ),
        bin(
            Value(4),
            op,
            Operand::Value(Value(1)),
            Operand::Value(Value(3)),
            ty,
        ),
    ]
}
fn rewrite(source: &[Instruction], shared: Option<Value>) -> Option<Vec<Instruction>> {
    let mut defs = vec![None; 8];
    let mut uses = vec![0; 8];
    for inst in source {
        defs[inst.dest()?.0 as usize] = Some(inst.clone());
        crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
            if let Operand::Value(v) = op {
                uses[v.0 as usize] += 1;
            }
        });
    }
    if let Some(v) = shared {
        uses[v.0 as usize] += 1;
    }
    fold(
        source.last()?,
        &Context {
            defs: &defs,
            uses: &uses,
            visits: std::cell::Cell::new(512),
            disable_logic: false,
            wrap_signed_overflow: true,
        },
        Value(6),
    )
}
#[test]
fn complement_arithmetic_exhausts_byte_ring_and_all_signednesses() {
    for ty in [
        IrType::U8,
        IrType::I8,
        IrType::U16,
        IrType::I16,
        IrType::U32,
        IrType::I32,
        IrType::U64,
        IrType::I64,
    ] {
        let mask = u64::MAX >> (64 - ty.size() * 8);
        for op in [IrBinOp::Add, IrBinOp::Sub] {
            for c in [0, 1, 42, 128, 255, 0x8000000000000000, u64::MAX] {
                let source = pattern(ty, op, c);
                let replacement = rewrite(&source, None).unwrap();
                assert_eq!(replacement.len(), 2);
                let Instruction::BinOp { rhs, .. } = replacement[0] else {
                    panic!("xor expected")
                };
                let inverted = const_in_ring(rhs, ty.size() * 8).unwrap();
                for x in 0..256u64 {
                    for y in 0..256u64 {
                        let original = (x ^ (c & mask)).wrapping_add(1) & mask;
                        let expected = if op == IrBinOp::Add {
                            y.wrapping_add(original)
                        } else {
                            y.wrapping_sub(original)
                        } & mask;
                        let transformed = if op == IrBinOp::Add {
                            y.wrapping_sub(x ^ inverted)
                        } else {
                            y.wrapping_add(x ^ inverted)
                        } & mask;
                        assert_eq!(transformed, expected);
                    }
                }
            }
        }
    }
}
#[test]
fn shared_arithmetic_and_nonunit_offsets_are_refused() {
    let source = pattern(IrType::U32, IrBinOp::Add, 42);
    for shared in [Value(2), Value(3)] {
        assert!(rewrite(&source, Some(shared)).is_none());
    }
    let mut wrong = source;
    if let Instruction::BinOp { rhs, .. } = &mut wrong[1] {
        *rhs = constant(2, IrType::U32);
    }
    assert!(rewrite(&wrong, None).is_none());
}

#[test]
fn complemented_comparisons_reverse_order_without_changing_signedness() {
    for ty in [
        IrType::U8,
        IrType::I8,
        IrType::U16,
        IrType::I16,
        IrType::U32,
        IrType::I32,
        IrType::U64,
        IrType::I64,
    ] {
        let width = ty.size() * 8;
        let mask = u64::MAX >> (64 - width);
        let sign = 1u64 << (width - 1);
        for op in [
            IrCmpOp::Eq,
            IrCmpOp::Ne,
            IrCmpOp::Ult,
            IrCmpOp::Ule,
            IrCmpOp::Ugt,
            IrCmpOp::Uge,
            IrCmpOp::Slt,
            IrCmpOp::Sle,
            IrCmpOp::Sgt,
            IrCmpOp::Sge,
        ] {
            let source = vec![
                Instruction::UnaryOp {
                    dest: Value(2),
                    op: IrUnaryOp::Not,
                    src: Operand::Value(Value(0)),
                    ty,
                },
                Instruction::UnaryOp {
                    dest: Value(3),
                    op: IrUnaryOp::Not,
                    src: Operand::Value(Value(1)),
                    ty,
                },
                Instruction::Cmp {
                    dest: Value(4),
                    op,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Value(Value(3)),
                    ty,
                },
            ];
            let replacement = rewrite(&source, None).unwrap();
            assert!(matches!(
                replacement.as_slice(),
                [Instruction::Cmp {
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Value(Value(0)),
                    ..
                }]
            ));
            for x in [0, 1, mask, mask - 1, sign, sign - 1] {
                for y in [0, 1, mask, mask - 1, sign, sign - 1] {
                    assert_eq!(
                        compare(op, !x & mask, !y & mask, width),
                        compare(op, y, x, width)
                    );
                }
            }
            assert!(rewrite(&source, Some(Value(2))).is_none());
        }
    }
}
