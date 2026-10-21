use super::*;

fn v(n: u32) -> Operand {
    Operand::Value(Value(n))
}
fn pattern(ty: IrType, op: IrUnaryOp, other: Option<u64>, cmp: IrCmpOp) -> Vec<Instruction> {
    let b = other.map_or(v(3), |k| constant(k, ty));
    let mut out = vec![Instruction::UnaryOp {
        dest: Value(2),
        op,
        src: v(0),
        ty,
    }];
    if other.is_none() {
        out.push(Instruction::UnaryOp {
            dest: Value(3),
            op,
            src: v(1),
            ty,
        });
    }
    out.push(Instruction::Cmp {
        dest: Value(4),
        op: cmp,
        lhs: v(2),
        rhs: b,
        ty,
    });
    let reverse = matches!(
        cmp,
        IrCmpOp::Ugt | IrCmpOp::Uge | IrCmpOp::Sgt | IrCmpOp::Sge
    );
    out.push(Instruction::Select {
        dest: Value(5),
        cond: v(4),
        true_val: if reverse { b } else { v(2) },
        false_val: if reverse { v(2) } else { b },
        ty,
    });
    out
}
fn rewrite(insts: &[Instruction], extra: Option<Operand>) -> Option<Vec<Instruction>> {
    let mut defs = vec![None; 9];
    let mut counts = vec![0; 9];
    for i in insts {
        defs[i.dest().unwrap().0 as usize] = Some(i.clone());
        crate::backend::liveness::for_each_operand_in_instruction(i, |o| {
            if let Operand::Value(v) = o {
                counts[v.0 as usize] += 1
            }
        });
    }
    if let Some(Operand::Value(v)) = extra {
        counts[v.0 as usize] += 1;
    }
    fold(
        insts.last().unwrap(),
        &Context {
            defs: &defs,
            uses: &counts,
            visits: std::cell::Cell::new(512),
        },
        &Facts(vec![]),
        Value(8),
    )
}
fn count_value(op: IrUnaryOp, x: u64, w: usize) -> u64 {
    let x = x & (u64::MAX >> (64 - w));
    if matches!(op, IrUnaryOp::Clz | IrUnaryOp::ClzNonZero) {
        u64::from(x.leading_zeros()) - (64 - w) as u64
    } else if x == 0 {
        w as u64
    } else {
        u64::from(x.trailing_zeros())
    }
}
fn execute(insts: &[Instruction], x: u64, y: u64, w: usize) -> u64 {
    let mut values = [0u64; 9];
    values[0] = x;
    values[1] = y;
    for i in insts {
        let get = |o| match o {
            Operand::Value(v) => values[v.0 as usize],
            _ => const_in_ring(o, w).unwrap(),
        };
        let value = match i {
            Instruction::BinOp {
                op: IrBinOp::Or,
                lhs,
                rhs,
                ..
            } => get(*lhs) | get(*rhs),
            Instruction::UnaryOp { op, src, .. } => count_value(*op, get(*src), w),
            _ => panic!("unexpected replacement"),
        };
        values[i.dest().unwrap().0 as usize] = value;
    }
    values[5]
}
#[test]
fn count_union_is_exhaustive_at_byte_width() {
    for op in [IrUnaryOp::Clz, IrUnaryOp::Ctz] {
        let replacement = rewrite(&pattern(IrType::U8, op, None, IrCmpOp::Ult), None).unwrap();
        for x in 0..256 {
            for y in 0..256 {
                assert_eq!(
                    execute(&replacement, x, y, 8),
                    count_value(op, x, 8).min(count_value(op, y, 8))
                );
            }
        }
    }
}
#[test]
fn count_caps_include_zero_and_every_byte_cap() {
    for op in [IrUnaryOp::Clz, IrUnaryOp::Ctz] {
        for cap in 0..8 {
            let replacement =
                rewrite(&pattern(IrType::U8, op, Some(cap), IrCmpOp::Ult), None).unwrap();
            for x in 0..256 {
                assert_eq!(
                    execute(&replacement, x, 0, 8),
                    count_value(op, x, 8).min(cap)
                );
            }
        }
    }
}
#[test]
fn count_merges_preserve_width_predicate_and_zero_policy() {
    for ty in [
        IrType::I8,
        IrType::U8,
        IrType::I16,
        IrType::U16,
        IrType::I32,
        IrType::U32,
        IrType::I64,
        IrType::U64,
    ] {
        let w = ty.size() * 8;
        for op in [
            IrUnaryOp::Clz,
            IrUnaryOp::Ctz,
            IrUnaryOp::ClzNonZero,
            IrUnaryOp::CtzNonZero,
        ] {
            for cmp in [
                IrCmpOp::Ult,
                IrCmpOp::Ule,
                IrCmpOp::Ugt,
                IrCmpOp::Uge,
                IrCmpOp::Slt,
                IrCmpOp::Sle,
                IrCmpOp::Sgt,
                IrCmpOp::Sge,
            ] {
                let result = rewrite(&pattern(ty, op, None, cmp), None).unwrap();
                for x in [1, 2, 3, 1u64 << (w - 1), u64::MAX] {
                    for y in [1, 2, 3, u64::MAX] {
                        assert_eq!(
                            execute(&result, x, y, w),
                            count_value(op, x, w).min(count_value(op, y, w))
                        );
                    }
                }
                let capped = rewrite(&pattern(ty, op, Some((w - 1) as u64), cmp), None).unwrap();
                assert_eq!(execute(&capped, 0, 0, w), (w - 1) as u64);
            }
        }
    }
}
#[test]
fn shared_mixed_maximum_and_invalid_caps_are_refused() {
    let original = pattern(IrType::U32, IrUnaryOp::Ctz, None, IrCmpOp::Ult);
    for user in [v(2), v(3), v(4)] {
        assert!(rewrite(&original, Some(user)).is_none());
    }
    let mut mixed = original.clone();
    if let Instruction::UnaryOp { op, .. } = &mut mixed[1] {
        *op = IrUnaryOp::Clz;
    }
    assert!(rewrite(&mixed, None).is_none());
    let mut maximum = original.clone();
    if let Instruction::Select {
        true_val,
        false_val,
        ..
    } = maximum.last_mut().unwrap()
    {
        std::mem::swap(true_val, false_val);
    }
    assert!(rewrite(&maximum, None).is_none());
    for cap in [32, 33, u64::MAX] {
        assert!(
            rewrite(
                &pattern(IrType::I32, IrUnaryOp::Clz, Some(cap), IrCmpOp::Slt),
                None
            )
            .is_none()
        );
    }
    let mut width = original.clone();
    if let Instruction::UnaryOp { ty, .. } = &mut width[1] {
        *ty = IrType::U64;
    }
    assert!(rewrite(&width, None).is_none());
}

#[test]
fn nonzero_refinement_refuses_other_unary_operators() {
    for (input, output) in [
        (IrUnaryOp::Clz, IrUnaryOp::ClzNonZero),
        (IrUnaryOp::ClzNonZero, IrUnaryOp::ClzNonZero),
        (IrUnaryOp::Ctz, IrUnaryOp::CtzNonZero),
        (IrUnaryOp::CtzNonZero, IrUnaryOp::CtzNonZero),
    ] {
        assert_eq!(nonzero(input), Some(output));
    }
    for op in [IrUnaryOp::Neg, IrUnaryOp::Not, IrUnaryOp::Popcount] {
        assert_eq!(nonzero(op), None);
    }
}
