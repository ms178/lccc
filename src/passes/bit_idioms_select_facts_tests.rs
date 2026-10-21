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
        result = super::super::relational_guards::fold(source.last().unwrap(), cx)
            .or_else(|| fold(source.last().unwrap(), cx, &Facts(vec![]), Value(80)))
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

#[test]
fn guarded_increment_and_contiguous_bound_exhaust_all_byte_pairs() {
    let ty = IrType::U8;
    let source = vec![
        bin(Value(3), IrBinOp::Add, v(0), constant(1, ty), ty),
        cmp(4, IrCmpOp::Ne, v(0), constant(255, ty), ty),
        cmp(5, IrCmpOp::Ule, v(3), v(1), ty),
        bin(Value(6), IrBinOp::And, v(4), v(5), IrType::I32),
    ];
    for x in 0..256 {
        for y in 0..256 {
            check(&source, [x, y, 0]);
        }
    }
    for bit in 1..8 {
        let bound = 1u64 << bit;
        for trailing in 0..bit {
            let mask = bound - (1u64 << trailing);
            let source = vec![
                cmp(3, IrCmpOp::Ult, v(0), constant(bound, ty), ty),
                bin(Value(4), IrBinOp::And, v(0), constant(mask, ty), ty),
                cmp(5, IrCmpOp::Ne, v(4), constant(mask, ty), ty),
                bin(Value(6), IrBinOp::And, v(3), v(5), IrType::I32),
            ];
            for x in 0..256 {
                check(&source, [x, 0, 0]);
            }
        }
    }
}
#[test]
fn disjoint_constant_choices_and_boolean_cardinality_preserve_truth_values() {
    let ty = IrType::U32;
    let predicates = vec![
        cmp(3, IrCmpOp::Ne, v(0), constant(0, ty), ty),
        cmp(4, IrCmpOp::Ne, v(1), constant(0, ty), ty),
    ];
    let mut cardinality = predicates.clone();
    cardinality.extend([
        bin(Value(5), IrBinOp::Add, v(3), v(4), ty),
        cmp(6, IrCmpOp::Eq, v(5), constant(1, ty), ty),
    ]);
    let mut choices = predicates;
    choices.extend([
        sel(5, v(3), constant(4, ty), constant(1, ty), ty),
        sel(6, v(4), constant(4, ty), constant(1, ty), ty),
        bin(Value(7), IrBinOp::And, v(5), v(6), ty),
        cmp(8, IrCmpOp::Eq, v(7), constant(0, ty), ty),
    ]);
    for x in 0..16 {
        for y in 0..16 {
            check(&cardinality, [x, y, 0]);
            check(&choices, [x, y, 0]);
        }
    }
}
#[test]
fn or_shift_order_requires_an_explicit_count_bound() {
    let ty = IrType::U8;
    let source = vec![
        bin(Value(3), IrBinOp::And, v(2), constant(7, ty), ty),
        bin(Value(4), IrBinOp::LShr, v(0), v(3), ty),
        bin(Value(5), IrBinOp::Or, v(0), v(1), ty),
        cmp(6, IrCmpOp::Ult, v(5), v(4), ty),
    ];
    for x in 0..256 {
        for count in 0..8 {
            check(&source, [x, 255 - x, count]);
        }
    }
    let unbounded = vec![
        bin(Value(3), IrBinOp::LShr, v(0), v(2), ty),
        bin(Value(4), IrBinOp::Or, v(0), v(1), ty),
        cmp(5, IrCmpOp::Ult, v(4), v(3), ty),
    ];
    assert!(rewrite(&unbounded).is_none());
}

#[test]
fn constant_choice_binops_fold_both_arms_without_cloning_variables() {
    let ty = IrType::U32;
    for op in [
        IrBinOp::Add,
        IrBinOp::Mul,
        IrBinOp::And,
        IrBinOp::Or,
        IrBinOp::Xor,
    ] {
        let source = vec![
            cmp(3, IrCmpOp::Ne, v(0), constant(0, ty), ty),
            sel(4, v(3), constant(7, ty), constant(0, ty), ty),
            bin(Value(5), op, v(4), v(3), ty),
        ];
        for x in 0..256 {
            check(&source, [x, 0, 0]);
        }
    }
}

#[test]
fn select_of_private_symmetric_selects_uses_canonical_predicate_parity() {
    let ty = IrType::U32;
    let source = vec![
        cmp(3, IrCmpOp::Ne, v(0), constant(0, ty), ty),
        cmp(4, IrCmpOp::Ne, v(1), constant(0, ty), ty),
        sel(5, v(3), v(0), v(1), ty),
        sel(6, v(3), v(1), v(0), ty),
        sel(7, v(4), v(5), v(6), ty),
    ];
    for x in 0..256 {
        for y in [0, 1, 127, 255, u32::MAX as u64] {
            check(&source, [x, y, 0]);
        }
    }
}

#[test]
fn relational_guards_refuse_missing_bounds_sparse_masks_and_word_truthiness() {
    let ty = IrType::U8;
    for (pred, limit) in [(IrCmpOp::Eq, 255), (IrCmpOp::Ne, 254)] {
        let source = vec![
            bin(Value(3), IrBinOp::Add, v(0), constant(1, ty), ty),
            cmp(4, pred, v(0), constant(limit, ty), ty),
            cmp(5, IrCmpOp::Ule, v(3), v(1), ty),
            bin(Value(6), IrBinOp::And, v(4), v(5), IrType::I32),
        ];
        assert!(rewrite(&source).is_none());
    }
    for mask in [0, 3, 5, 10, 20] {
        let source = vec![
            cmp(3, IrCmpOp::Ult, v(0), constant(16, ty), ty),
            bin(Value(4), IrBinOp::And, v(0), constant(mask, ty), ty),
            cmp(5, IrCmpOp::Ne, v(4), constant(mask, ty), ty),
            bin(Value(6), IrBinOp::And, v(3), v(5), IrType::I32),
        ];
        assert!(rewrite(&source).is_none());
    }
    let cardinality = vec![
        bin(Value(3), IrBinOp::Add, v(0), v(1), ty),
        cmp(4, IrCmpOp::Eq, v(3), constant(1, ty), ty),
    ];
    assert!(rewrite(&cardinality).is_none());
    for (a, b) in [(4, 5), (0, 255)] {
        let source = vec![
            cmp(3, IrCmpOp::Ne, v(0), constant(0, ty), ty),
            cmp(4, IrCmpOp::Ne, v(1), constant(0, ty), ty),
            sel(5, v(3), constant(a, ty), constant(b, ty), ty),
            sel(6, v(4), constant(a, ty), constant(b, ty), ty),
            bin(Value(7), IrBinOp::And, v(5), v(6), ty),
            cmp(8, IrCmpOp::Eq, v(7), constant(0, ty), ty),
        ];
        assert!(rewrite(&source).is_none());
    }
}
