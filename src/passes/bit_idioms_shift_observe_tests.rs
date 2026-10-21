use super::*;

fn pattern(
    ty: IrType,
    direction: IrBinOp,
    power: u64,
    mask: u64,
    cmp: IrCmpOp,
    bound: bool,
) -> Vec<Instruction> {
    vec![
        bin(
            Value(1),
            IrBinOp::And,
            Operand::Value(Value(0)),
            constant(
                if bound {
                    (ty.size() * 8 - 1) as u64
                } else {
                    u64::MAX
                },
                ty,
            ),
            ty,
        ),
        bin(
            Value(2),
            direction,
            constant(power, ty),
            Operand::Value(Value(1)),
            ty,
        ),
        bin(
            Value(3),
            IrBinOp::And,
            Operand::Value(Value(2)),
            constant(mask, ty),
            ty,
        ),
        Instruction::Cmp {
            dest: Value(4),
            op: cmp,
            lhs: Operand::Value(Value(3)),
            rhs: constant(0, ty),
            ty,
        },
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
        },
    )
}
fn evaluate(replacement: &[Instruction], count: u64, width: usize) -> bool {
    match replacement[0] {
        Instruction::Copy { src, .. } => const_in_ring(src, 32).unwrap() != 0,
        Instruction::Cmp { op, rhs, .. } => {
            compare(op, count, const_in_ring(rhs, width).unwrap(), width)
        }
        _ => panic!("comparison or constant expected"),
    }
}
#[test]
fn bounded_shift_observations_exhaust_every_position_and_count() {
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
        for direction in [IrBinOp::Shl, IrBinOp::LShr] {
            for bit in 0..width {
                for observed in 0..width {
                    for cmp in [IrCmpOp::Eq, IrCmpOp::Ne] {
                        let source =
                            pattern(ty, direction, 1u64 << bit, 1u64 << observed, cmp, true);
                        let replacement = rewrite(&source, None).unwrap();
                        for count in 0..width {
                            let shifted = if direction == IrBinOp::Shl {
                                (1u64 << bit) << count
                            } else {
                                (1u64 << bit) >> count
                            };
                            let set = shifted & (1u64 << observed) != 0;
                            assert_eq!(
                                evaluate(&replacement, count as u64, width),
                                if cmp == IrCmpOp::Ne { set } else { !set }
                            );
                        }
                    }
                }
                for bits in 2..width {
                    let mask = (1u64 << bits) - 1;
                    let source = pattern(ty, IrBinOp::Shl, 1u64 << bit, mask, IrCmpOp::Ne, true);
                    let replacement = rewrite(&source, None).unwrap();
                    for count in 0..width {
                        assert_eq!(
                            evaluate(&replacement, count as u64, width),
                            ((1u64 << bit) << count) & mask != 0
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn unknown_counts_nonpowers_shared_uses_and_arithmetic_shifts_are_refused() {
    let ty = IrType::U32;
    assert!(rewrite(&pattern(ty, IrBinOp::Shl, 2, 16, IrCmpOp::Ne, false), None).is_none());
    assert!(rewrite(&pattern(ty, IrBinOp::Shl, 3, 16, IrCmpOp::Ne, true), None).is_none());
    assert!(rewrite(&pattern(ty, IrBinOp::Shl, 2, 21, IrCmpOp::Ne, true), None).is_none());
    assert!(rewrite(&pattern(ty, IrBinOp::AShr, 2, 16, IrCmpOp::Ne, true), None).is_none());
    for node in [Value(2), Value(3)] {
        assert!(
            rewrite(
                &pattern(ty, IrBinOp::Shl, 2, 16, IrCmpOp::Ne, true),
                Some(node)
            )
            .is_none()
        );
    }
}

#[test]
fn bounded_count_casts_keep_the_original_operand_and_actual_compare_type() {
    for to_ty in [
        IrType::U8,
        IrType::I8,
        IrType::U16,
        IrType::I16,
        IrType::U32,
        IrType::I32,
        IrType::U64,
        IrType::I64,
    ] {
        let mut source = pattern(IrType::U64, IrBinOp::Shl, 2, 16, IrCmpOp::Ne, true);
        source.insert(
            1,
            Instruction::Cast {
                dest: Value(5),
                src: Operand::Value(Value(1)),
                from_ty: IrType::U64,
                to_ty,
            },
        );
        if let Instruction::BinOp { rhs, .. } = &mut source[2] {
            *rhs = Operand::Value(Value(5));
        }
        let replacement = rewrite(&source, None).unwrap();
        assert!(
            matches!(replacement[0], Instruction::Cmp { lhs: Operand::Value(Value(5)), ty, .. } if ty == to_ty)
        );
        for count in 0..64 {
            assert_eq!(evaluate(&replacement, count, to_ty.size() * 8), count == 3);
        }
    }
    let mut unbounded = pattern(IrType::U64, IrBinOp::Shl, 2, 16, IrCmpOp::Ne, false);
    unbounded.insert(
        1,
        Instruction::Cast {
            dest: Value(5),
            src: Operand::Value(Value(1)),
            from_ty: IrType::U64,
            to_ty: IrType::I32,
        },
    );
    if let Instruction::BinOp { rhs, .. } = &mut unbounded[2] {
        *rhs = Operand::Value(Value(5));
    }
    assert!(rewrite(&unbounded, None).is_none());
}
