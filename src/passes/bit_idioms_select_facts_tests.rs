use super::*;
fn v(n: u32) -> Operand {
    Operand::Value(Value(n))
}
fn cmp(d: u32, op: IrCmpOp, a: Operand, b: Operand, ty: IrType) -> Instruction {
    Instruction::Cmp {
        dest: Value(d),
        op,
        lhs: a,
        rhs: b,
        ty,
    }
}
fn sel(d: u32, c: Operand, a: Operand, b: Operand, ty: IrType) -> Instruction {
    Instruction::Select {
        dest: Value(d),
        cond: c,
        true_val: a,
        false_val: b,
        ty,
    }
}
fn context(source: &[Instruction], test: impl FnOnce(&Context<'_>)) {
    let mut defs = vec![None; 128];
    let mut uses = vec![0; 128];
    for inst in source {
        defs[inst.dest().unwrap().0 as usize] = Some(inst.clone());
        crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
            if let Operand::Value(v) = op {
                uses[v.0 as usize] += 1;
            }
        });
    }
    test(&Context {
        defs: &defs,
        uses: &uses,
        visits: std::cell::Cell::new(512),
        disable_logic: false,
    });
}
fn rewrite(source: &[Instruction]) -> Option<Vec<Instruction>> {
    let mut result = None;
    context(source, |cx| {
        result = fold(source.last().unwrap(), cx, &Facts(vec![]))
    });
    result
}
fn eval(source: &[Instruction], inputs: [u64; 3], result: u32) -> u64 {
    let mut values = [0u64; 128];
    values[..3].copy_from_slice(&inputs);
    let get = |op: Operand, w: usize, values: &[u64]| match op {
        Operand::Value(v) => values[v.0 as usize] & (u64::MAX >> (64 - w)),
        _ => const_in_ring(op, w).unwrap(),
    };
    for inst in source {
        let (d, n) = match inst {
            Instruction::Copy { dest, src } => (*dest, get(*src, 64, &values)),
            Instruction::Cast {
                dest, src, to_ty, ..
            } => (*dest, get(*src, to_ty.size() * 8, &values)),
            Instruction::Cmp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            } => (
                *dest,
                u64::from(compare(
                    *op,
                    get(*lhs, ty.size() * 8, &values),
                    get(*rhs, ty.size() * 8, &values),
                    ty.size() * 8,
                )),
            ),
            Instruction::Select {
                dest,
                cond,
                true_val,
                false_val,
                ty,
            } => (
                *dest,
                get(
                    if get(*cond, 64, &values) != 0 {
                        *true_val
                    } else {
                        *false_val
                    },
                    ty.size() * 8,
                    &values,
                ),
            ),
            Instruction::BinOp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            } => {
                let w = ty.size() * 8;
                let mask = u64::MAX >> (64 - w);
                let a = get(*lhs, w, &values);
                let b = get(*rhs, w, &values);
                (
                    *dest,
                    match op {
                        IrBinOp::Add => a.wrapping_add(b),
                        IrBinOp::Sub => a.wrapping_sub(b),
                        IrBinOp::Mul => a.wrapping_mul(b),
                        IrBinOp::And => a & b,
                        IrBinOp::Or => a | b,
                        IrBinOp::Xor => a ^ b,
                        IrBinOp::LShr => a >> b,
                        IrBinOp::Shl => a << b,
                        _ => panic!("unsupported evaluator operation"),
                    } & mask,
                )
            }
            _ => panic!("unsupported evaluator instruction"),
        };
        values[d.0 as usize] = n;
    }
    values[result as usize]
}
fn types() -> [IrType; 8] {
    [
        IrType::U8,
        IrType::I8,
        IrType::U16,
        IrType::I16,
        IrType::U32,
        IrType::I32,
        IrType::U64,
        IrType::I64,
    ]
}
fn check(source: &[Instruction], inputs: [u64; 3]) {
    let replacement = rewrite(source).expect("expected consumer rewrite");
    let mut rewritten = source[..source.len() - 1].to_vec();
    rewritten.extend(replacement);
    let dest = source.last().unwrap().dest().unwrap().0;
    assert_eq!(
        eval(source, inputs, dest),
        eval(&rewritten, inputs, dest),
        "{source:?}, {inputs:?}"
    );
}
#[test]
fn nested_same_and_inverse_conditions_preserve_every_scalar_width() {
    for ty in types() {
        for inverse in [false, true] {
            for truth in [0, 1, 128, 255] {
                let source = vec![
                    cmp(3, IrCmpOp::Ne, v(0), constant(0, ty), ty),
                    cmp(
                        4,
                        if inverse { IrCmpOp::Eq } else { IrCmpOp::Ne },
                        v(0),
                        constant(0, ty),
                        ty,
                    ),
                    sel(5, v(4), v(1), v(2), ty),
                    sel(6, v(3), v(5), v(2), ty),
                ];
                check(&source, [truth, 0x123456789abcdef0, 0xfedcba9876543210]);
            }
        }
    }
}
#[test]
fn symmetric_select_operations_and_equality_are_exact_with_shared_producers() {
    for ty in types() {
        for inverse in [false, true] {
            for op in [
                IrBinOp::Add,
                IrBinOp::Mul,
                IrBinOp::And,
                IrBinOp::Or,
                IrBinOp::Xor,
            ] {
                let mut source = vec![
                    cmp(3, IrCmpOp::Ne, v(0), constant(0, ty), ty),
                    cmp(
                        4,
                        if inverse { IrCmpOp::Eq } else { IrCmpOp::Ne },
                        v(0),
                        constant(0, ty),
                        ty,
                    ),
                    sel(5, v(3), v(1), v(2), ty),
                    sel(
                        6,
                        v(4),
                        if inverse { v(1) } else { v(2) },
                        if inverse { v(2) } else { v(1) },
                        ty,
                    ),
                ];
                source.push(bin(Value(7), op, v(5), v(6), ty));
                for x in [0, 1, 127, 128, 255, u64::MAX] {
                    for y in [0, 1, 128, u64::MAX] {
                        check(&source, [x, x, y]);
                    }
                }
                source.pop();
                for pred in [IrCmpOp::Eq, IrCmpOp::Ne] {
                    let mut pair = source.clone();
                    pair.push(cmp(7, pred, v(5), v(6), ty));
                    for x in 0..256 {
                        check(&pair, [x % 2, x, 255 - x]);
                    }
                }
            }
        }
    }
}
#[test]
fn constant_select_comparisons_keep_signedness_and_boolean_result_type() {
    for ty in types() {
        let mask = u64::MAX >> (64 - ty.size() * 8);
        for pred in [
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
                cmp(3, IrCmpOp::Ne, v(0), constant(0, ty), ty),
                sel(4, v(3), constant(mask, ty), constant(0, ty), ty),
                sel(5, v(3), constant(1, ty), constant(mask, ty), ty),
                cmp(6, pred, v(4), v(5), ty),
            ];
            for c in [0, 1, 255, u64::MAX] {
                check(&source, [c, 0, 0]);
            }
        }
    }
}
#[test]
fn contiguous_mask_observations_exhaust_byte_ring_and_wide_boundaries() {
    for ty in types() {
        let width = ty.size() * 8;
        let full = u64::MAX >> (64 - width);
        for k in 0..width {
            let low = if k == 0 { 0 } else { (1u64 << k) - 1 };
            for kind in [IrBinOp::And, IrBinOp::Or] {
                let mask = if kind == IrBinOp::And {
                    low
                } else {
                    !low & full
                };
                for pred in [IrCmpOp::Eq, IrCmpOp::Ne] {
                    let source = vec![
                        bin(Value(3), kind, v(0), constant(mask, ty), ty),
                        cmp(4, pred, v(3), v(0), ty),
                    ];
                    for x in 0..256 {
                        check(&source, [x, 0, 0]);
                    }
                    for x in [
                        full,
                        full >> 1,
                        1u64 << (width - 1),
                        mask,
                        mask.wrapping_sub(1) & full,
                        mask.wrapping_add(1) & full,
                    ] {
                        check(&source, [x, 0, 0]);
                    }
                }
            }
        }
    }
}
#[test]
fn bounded_complement_subtraction_refuses_unknown_high_bits() {
    for ty in types() {
        let source = vec![
            bin(Value(3), IrBinOp::And, v(0), constant(15, ty), ty),
            bin(Value(4), IrBinOp::Sub, constant(15, ty), v(3), ty),
        ];
        for x in 0..256 {
            check(&source, [x, 0, 0]);
        }
        assert!(rewrite(&[bin(Value(3), IrBinOp::Sub, constant(15, ty), v(0), ty)]).is_none());
    }
}
#[test]
fn nonzero_single_bit_facts_do_not_generalize_to_sparse_masks() {
    for mask in [1, 2, 3, 5, 128] {
        let source = vec![
            bin(
                Value(3),
                IrBinOp::And,
                v(0),
                constant(mask, IrType::U8),
                IrType::U8,
            ),
            cmp(4, IrCmpOp::Eq, v(3), constant(0, IrType::U8), IrType::U8),
        ];
        context(&source, |cx| {
            let mut facts = Facts(vec![]);
            cx.facts(v(4), false, &mut facts, 8);
            let (zero, one) = cx.known(v(0), IrType::U8, &facts, 8);
            assert_eq!(zero, 0);
            assert_eq!(one, if mask.is_power_of_two() { mask } else { 0 });
        });
    }
}
#[test]
fn shift_known_bits_are_sound_for_every_byte_value_and_valid_count() {
    for op in [IrBinOp::LShr, IrBinOp::Shl] {
        for amount in 0..8 {
            for input in 0..256 {
                let source = vec![
                    bin(
                        Value(3),
                        IrBinOp::And,
                        v(0),
                        constant(0x5a, IrType::U8),
                        IrType::U8,
                    ),
                    bin(Value(4), op, v(3), constant(amount, IrType::U8), IrType::U8),
                ];
                context(&source, |cx| {
                    let (zero, one) = cx.known(v(4), IrType::U8, &Facts(vec![]), 8);
                    let actual = eval(&source, [input, 0, 0], 4);
                    assert_eq!(actual & zero, 0);
                    assert_eq!(actual & one, one);
                });
            }
        }
    }
}
#[test]
fn unrelated_conditions_noncommutative_ops_sparse_masks_and_floats_refuse() {
    let ty = IrType::U32;
    let mut source = vec![
        sel(3, v(0), v(1), v(2), ty),
        sel(4, v(1), v(2), v(1), ty),
        bin(Value(5), IrBinOp::Mul, v(3), v(4), ty),
    ];
    assert!(rewrite(&source).is_none());
    source[1] = sel(4, v(0), v(2), v(1), ty);
    source[2] = bin(Value(5), IrBinOp::Sub, v(3), v(4), ty);
    assert!(rewrite(&source).is_none());
    assert!(
        rewrite(&[
            bin(Value(3), IrBinOp::Or, v(0), constant(0xa5, ty), ty),
            cmp(4, IrCmpOp::Eq, v(3), v(0), ty)
        ])
        .is_none()
    );
    assert!(rewrite(&[sel(3, v(0), v(1), v(2), IrType::F64)]).is_none());
}
