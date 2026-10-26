use super::*;

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
        Value(6),
    )
}

fn mask(ty: IrType) -> u64 {
    u64::MAX >> (64 - ty.size() * 8)
}

fn signed_value(ring: u64, ty: IrType) -> i128 {
    let w = ty.size() * 8;
    (((ring << (64 - w)) as i64) >> (64 - w)) as i128
}

fn reference_remainder(x: u64, c: u64, ty: IrType) -> u64 {
    let dividend = signed_value(x, ty);
    let divisor = signed_value(c, ty);
    assert_ne!(divisor, 0, "division by zero is undefined at source level");
    // Truncating division, matching the C99 semantics of `%`.
    let remainder = if divisor == -1 { 0 } else { dividend % divisor };
    remainder as u64 & mask(ty)
}

/// Ring evaluation of the emitted replacement: `add` then unsigned compare.
fn evaluate(replacement: &[Instruction], x: u64, ty: IrType) -> bool {
    let [
        Instruction::BinOp {
            dest: shifted,
            op: IrBinOp::Add,
            lhs,
            rhs,
            ..
        },
        Instruction::Cmp {
            op,
            lhs: probe,
            rhs: limit,
            ..
        },
    ] = replacement
    else {
        panic!("unexpected replacement shape: {replacement:?}")
    };
    assert_eq!(lhs, &Operand::Value(Value(0)));
    let Operand::Const(addend) = rhs else {
        panic!("non-constant addend")
    };
    assert_eq!(probe, &Operand::Value(*shifted));
    let Operand::Const(limit) = limit else {
        panic!("non-constant limit")
    };
    let shifted = x.wrapping_add(c_value(*addend) as u64) & mask(ty);
    let limit = c_value(*limit) & mask(ty);
    compare(*op, shifted, limit, ty.size() * 8)
}

fn c_value(c: IrConst) -> u64 {
    c.to_i128().unwrap() as u64
}

fn remainder_pattern(ty: IrType, cmp: IrCmpOp, divisor: u64, swapped: bool) -> Vec<Instruction> {
    let (lhs, rhs) = if swapped {
        (Operand::Value(Value(0)), Operand::Value(Value(2)))
    } else {
        (Operand::Value(Value(2)), Operand::Value(Value(0)))
    };
    vec![
        bin(
            Value(2),
            IrBinOp::SRem,
            Operand::Value(Value(0)),
            constant(divisor, ty),
            ty,
        ),
        Instruction::Cmp {
            dest: Value(3),
            op: cmp,
            lhs,
            rhs,
            ty,
        },
    ]
}

#[test]
fn remainder_identity_exhausts_every_byte_ring_divisor() {
    for ty in [IrType::I8, IrType::U8] {
        let ring = mask(ty);
        for divisor in 0..=ring {
            if signed_value(divisor, ty) == 0 {
                continue;
            }
            for cmp in [IrCmpOp::Eq, IrCmpOp::Ne] {
                for swapped in [false, true] {
                    let replacement =
                        rewrite(&remainder_pattern(ty, cmp, divisor, swapped), None).unwrap();
                    for x in 0..=ring {
                        let expected =
                            compare(cmp, reference_remainder(x, divisor, ty), x, ty.size() * 8);
                        assert_eq!(
                            evaluate(&replacement, x, ty),
                            expected,
                            "ty={ty:?} c={divisor} x={x} op={cmp:?} swapped={swapped}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn remainder_identity_holds_at_extremal_divisors_and_widths() {
    for ty in [
        IrType::I8,
        IrType::I16,
        IrType::I32,
        IrType::I64,
        IrType::U32,
    ] {
        let ring = mask(ty);
        let w = ty.size() * 8;
        let sign = 1u64 << (w - 1);
        let divisors = [1, 2, 7, 250, sign - 1, sign, sign + 1, ring];
        let mut probes = vec![0, 1, 2, sign - 2, sign - 1, sign, sign + 1, ring - 1, ring];
        probes.extend([3, 249, 251, 999_983, 0x0123_4567_89ab_cdef & ring]);
        for divisor in divisors {
            if signed_value(divisor, ty) == 0 {
                continue;
            }
            for cmp in [IrCmpOp::Eq, IrCmpOp::Ne] {
                let replacement = rewrite(&remainder_pattern(ty, cmp, divisor, false), None)
                    .unwrap_or_else(|| panic!("ty={ty:?} c={divisor} not folded"));
                for x in probes.iter().copied().chain(
                    // Deterministic sweep of the neighbourhoods where the
                    // quotient crosses zero.
                    (0..64).map(|k| divisor.wrapping_add(k * 2 + 1)),
                ) {
                    let x = x & ring;
                    let expected = compare(cmp, reference_remainder(x, divisor, ty), x, w);
                    assert_eq!(
                        evaluate(&replacement, x, ty),
                        expected,
                        "ty={ty:?} c={divisor} x={x} op={cmp:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn remainder_identity_refuses_unproven_shapes() {
    let ty = IrType::I32;
    // A shared remainder keeps its producer, so the rewrite is not free.
    let source = remainder_pattern(ty, IrCmpOp::Eq, 250, false);
    assert!(rewrite(&source, Some(Value(2))).is_none());
    // Ordering compares are not membership tests.
    for cmp in [IrCmpOp::Ult, IrCmpOp::Slt, IrCmpOp::Uge, IrCmpOp::Sgt] {
        assert!(rewrite(&remainder_pattern(ty, cmp, 250, false), None).is_none());
    }
    // The compared value must be the dividend itself, not a rescaled copy.
    let mut other = remainder_pattern(ty, IrCmpOp::Eq, 250, false);
    if let Instruction::Cmp { rhs, .. } = &mut other[1] {
        *rhs = Operand::Value(Value(1));
    }
    assert!(rewrite(&other, None).is_none());
    // Width mismatches are left to the type legaliser.
    let mut wide = remainder_pattern(ty, IrCmpOp::Eq, 250, false);
    if let Instruction::Cmp { ty: t, .. } = &mut wide[1] {
        *t = IrType::I64;
    }
    assert!(rewrite(&wide, None).is_none());
    // A symbolic divisor has no static window.
    let mut symbolic = remainder_pattern(ty, IrCmpOp::Eq, 250, false);
    if let Instruction::BinOp { rhs, .. } = &mut symbolic[0] {
        *rhs = Operand::Value(Value(1));
    }
    assert!(rewrite(&symbolic, None).is_none());
}

fn bound_select(ty: IrType, op: IrCmpOp, bound: u64, x_first: bool) -> Vec<Instruction> {
    let x = Operand::Value(Value(0));
    let b = constant(bound, ty);
    let (lhs, rhs) = (x, b);
    let (true_val, false_val) = if x_first { (lhs, rhs) } else { (rhs, lhs) };
    vec![
        Instruction::Cmp {
            dest: Value(1),
            op,
            lhs,
            rhs,
            ty,
        },
        Instruction::Select {
            dest: Value(2),
            cond: Operand::Value(Value(1)),
            true_val,
            false_val,
            ty,
        },
    ]
}

#[test]
fn saturated_min_max_selects_are_identities_on_the_other_arm() {
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
        let ring = mask(ty);
        let w = ty.size() * 8;
        let sign = 1u64 << (w - 1);
        let cases = [
            (IrCmpOp::Slt, sign - 1),
            (IrCmpOp::Sle, sign - 1),
            (IrCmpOp::Sgt, sign),
            (IrCmpOp::Sge, sign),
            (IrCmpOp::Ult, ring),
            (IrCmpOp::Ule, ring),
            (IrCmpOp::Ugt, 0),
            (IrCmpOp::Uge, 0),
        ];
        for (op, bound) in cases {
            // Mirroring the arms swaps which direction is a minimum, so the
            // dual predicate tests the same min/max with the arms reversed.
            let dual = match op {
                IrCmpOp::Slt => IrCmpOp::Sgt,
                IrCmpOp::Sle => IrCmpOp::Sge,
                IrCmpOp::Sgt => IrCmpOp::Slt,
                IrCmpOp::Sge => IrCmpOp::Sle,
                IrCmpOp::Ult => IrCmpOp::Ugt,
                IrCmpOp::Ule => IrCmpOp::Uge,
                IrCmpOp::Ugt => IrCmpOp::Ult,
                IrCmpOp::Uge => IrCmpOp::Ule,
                IrCmpOp::Eq | IrCmpOp::Ne => unreachable!(),
            };
            for (op, x_first) in [(op, true), (dual, false)] {
                let replacement = rewrite(&bound_select(ty, op, bound, x_first), None)
                    .unwrap_or_else(|| panic!("{ty:?} {op:?} bound={bound} x_first={x_first}"));
                assert!(matches!(
                    replacement.as_slice(),
                    [Instruction::Copy {
                        src: Operand::Value(Value(0)),
                        ..
                    }]
                ));
            }
        }
        // One step inside the ring is a real clamp and must survive.
        assert!(rewrite(&bound_select(ty, IrCmpOp::Slt, sign - 2, true), None).is_none());
        assert!(rewrite(&bound_select(ty, IrCmpOp::Sgt, sign + 1, true), None).is_none());
        assert!(rewrite(&bound_select(ty, IrCmpOp::Ult, ring - 1, false), None).is_none());
        assert!(rewrite(&bound_select(ty, IrCmpOp::Ugt, 1, false), None).is_none());
    }
}

fn complement_of_negation(ty: IrType, outer: IrUnaryOp, inner: IrUnaryOp) -> Vec<Instruction> {
    vec![
        Instruction::UnaryOp {
            dest: Value(1),
            op: inner,
            src: Operand::Value(Value(0)),
            ty,
        },
        Instruction::UnaryOp {
            dest: Value(2),
            op: outer,
            src: Operand::Value(Value(1)),
            ty,
        },
    ]
}

#[test]
fn complement_and_negation_commute_into_a_unit_offset() {
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
        let ring = mask(ty);
        let w = ty.size() * 8;
        for (outer, inner, offset) in [
            (IrUnaryOp::Not, IrUnaryOp::Neg, 1u64.wrapping_neg()),
            (IrUnaryOp::Neg, IrUnaryOp::Not, 1),
        ] {
            let source = complement_of_negation(ty, outer, inner);
            let replacement = rewrite(&source, None).unwrap();
            let [
                Instruction::BinOp {
                    op: IrBinOp::Add,
                    lhs,
                    rhs,
                    ..
                },
            ] = replacement.as_slice()
            else {
                panic!("expected a single add")
            };
            assert_eq!(lhs, &Operand::Value(Value(0)));
            let Operand::Const(c) = rhs else {
                panic!("expected a constant addend")
            };
            assert_eq!(c_value(*c) & ring, offset & ring);
            assert!(rewrite(&source, Some(Value(1))).is_none());
            for x in [0, 1, 2, sign_bit(ty) - 1, sign_bit(ty), ring - 1, ring] {
                let x = x & ring;
                let expected = match (outer, inner) {
                    // `~(-x)` is `x - 1` and `-(~x)` is `x + 1`.
                    (IrUnaryOp::Not, IrUnaryOp::Neg) => x.wrapping_sub(1) & ring,
                    _ => x.wrapping_add(1) & ring,
                };
                let transformed = x.wrapping_add(offset) & ring;
                assert_eq!(transformed, expected, "{ty:?} x={x} w={w}");
            }
        }
        // Two negations and two complements are not unit offsets.
        assert!(
            rewrite(
                &complement_of_negation(ty, IrUnaryOp::Neg, IrUnaryOp::Neg),
                None
            )
            .is_none()
        );
        assert!(
            rewrite(
                &complement_of_negation(ty, IrUnaryOp::Not, IrUnaryOp::Not),
                None
            )
            .is_none()
        );
    }
}

fn sign_bit(ty: IrType) -> u64 {
    1u64 << (ty.size() * 8 - 1)
}

fn masked_shift(ty: IrType, mask_form: bool, count: Operand, swapped: bool) -> Vec<Instruction> {
    let sign_shift = bin(
        Value(1),
        IrBinOp::LShr,
        Operand::Value(Value(0)),
        constant(ty.size() as u64 * 8 - 1, ty),
        ty,
    );
    let mask = if mask_form {
        Instruction::UnaryOp {
            dest: Value(2),
            op: IrUnaryOp::Neg,
            src: Operand::Value(Value(1)),
            ty,
        }
    } else {
        bin(
            Value(2),
            IrBinOp::AShr,
            Operand::Value(Value(0)),
            constant(ty.size() as u64 * 8 - 1, ty),
            ty,
        )
    };
    let (lhs, rhs) = if swapped {
        (Operand::Value(Value(2)), Operand::Value(Value(0)))
    } else {
        (Operand::Value(Value(0)), Operand::Value(Value(2)))
    };
    vec![
        sign_shift,
        mask,
        crate::ir::instruction::Instruction::BinOp {
            dest: Value(3),
            op: IrBinOp::Xor,
            lhs,
            rhs,
            ty,
        },
        bin(Value(4), IrBinOp::LShr, Operand::Value(Value(3)), count, ty),
        bin(
            Value(5),
            IrBinOp::Xor,
            Operand::Value(Value(4)),
            Operand::Value(Value(2)),
            ty,
        ),
    ]
}

fn ashr_reference(x: u64, count: u64, ty: IrType) -> u64 {
    let w = ty.size() * 8;
    let signed = signed_value(x, ty) as i64;
    ((signed >> (count as u32 % w as u32)) as u64) & mask(ty)
}

#[test]
fn masked_shift_folds_to_arithmetic_shift_for_every_count() {
    for ty in [
        IrType::I8,
        IrType::U8,
        IrType::I16,
        IrType::U16,
        IrType::I32,
    ] {
        let ring = mask(ty);
        let w = ty.size() * 8;
        for mask_form in [true, false] {
            for swapped in [false, true] {
                // Counts beyond the width prove the masked-count agreement
                // between the two shifts, not just the in-range values.
                for count in 0..=2 * w as u64 + 1 {
                    let source = masked_shift(ty, mask_form, constant(count, ty), swapped);
                    let replacement = rewrite(&source, None).unwrap_or_else(|| {
                        panic!("{ty:?} mask_form={mask_form} n={count} swapped={swapped}")
                    });
                    let [
                        Instruction::BinOp {
                            op: IrBinOp::AShr,
                            lhs,
                            rhs,
                            ty: result_ty,
                            ..
                        },
                    ] = replacement.as_slice()
                    else {
                        panic!("expected one arithmetic shift: {replacement:?}")
                    };
                    assert_eq!(*lhs, Operand::Value(Value(0)));
                    assert_eq!(*result_ty, ty);
                    let Operand::Const(shift) = rhs else {
                        panic!("non-constant count")
                    };
                    let shift = c_value(*shift) & ring;
                    assert_eq!(shift, count & ring);
                    for x in [
                        0,
                        1,
                        2,
                        ring / 2,
                        sign_bit(ty),
                        sign_bit(ty) + 1,
                        ring - 1,
                        ring,
                    ] {
                        let x = x & ring;
                        assert_eq!(ashr_reference(x, count, ty), ashr_reference(x, shift, ty));
                    }
                }
            }
        }
    }
}

#[test]
fn masked_shift_accepts_a_runtime_count_and_refuses_flat_masks() {
    let ty = IrType::I32;
    let runtime = Operand::Value(Value(6));
    assert!(rewrite(&masked_shift(ty, true, runtime, false), None).is_some());
    // A mask that is not this value's sign broadcast is not covered.
    let mut foreign = masked_shift(ty, true, runtime, false);
    if let Instruction::BinOp { lhs, .. } = &mut foreign[2] {
        *lhs = Operand::Value(Value(7));
    }
    assert!(rewrite(&foreign, None).is_none());
    // A shared mask has a second consumer and must survive.
    assert!(rewrite(&masked_shift(ty, true, runtime, false), Some(Value(2))).is_none());
    // Width mismatches between xor and shift are left alone.
    let mut wide = masked_shift(ty, true, runtime, false);
    if let Instruction::BinOp { ty: t, .. } = &mut wide[3] {
        *t = IrType::I64;
    }
    assert!(rewrite(&wide, None).is_none());
}

fn zero_test_select(ty: IrType, op: IrCmpOp, zero_is_true: bool) -> Vec<Instruction> {
    let x = Operand::Value(Value(0));
    let zero = constant(0, ty);
    let (true_val, false_val) = if zero_is_true { (zero, x) } else { (x, zero) };
    vec![
        Instruction::Cmp {
            dest: Value(1),
            op,
            lhs: x,
            rhs: zero,
            ty,
        },
        Instruction::Select {
            dest: Value(2),
            cond: Operand::Value(Value(1)),
            true_val,
            false_val,
            ty,
        },
    ]
}

#[test]
fn zero_test_selects_keep_the_value_only_in_the_selecting_order() {
    for ty in [
        IrType::I8,
        IrType::U8,
        IrType::I32,
        IrType::U32,
        IrType::I64,
        IrType::U64,
    ] {
        // `x != 0 ? x : 0` and `x == 0 ? 0 : x` are both `x`.
        for (op, zero_is_true) in [(IrCmpOp::Ne, false), (IrCmpOp::Eq, true)] {
            let source = zero_test_select(ty, op, zero_is_true);
            let replacement = rewrite(&source, None)
                .unwrap_or_else(|| panic!("{ty:?} {op:?} zero_is_true={zero_is_true}"));
            assert!(matches!(
                replacement.as_slice(),
                [Instruction::Copy {
                    src: Operand::Value(Value(0)),
                    ..
                }]
            ));
            // Exhaustive over the byte ring and the sign boundaries of wider
            // types: the folded value equals the original select everywhere.
            for x in [
                0,
                1,
                2,
                mask(ty) - 1,
                mask(ty),
                sign_bit(ty),
                sign_bit(ty) - 1,
            ] {
                let x = x & mask(ty);
                let condition = match op {
                    IrCmpOp::Ne => x != 0,
                    _ => x == 0,
                };
                let selected = if condition {
                    if zero_is_true { 0 } else { x }
                } else if zero_is_true {
                    x
                } else {
                    0
                };
                assert_eq!(selected, x, "{ty:?} {op:?} x={x}");
            }
        }
        // The reversed orders are real zeroing choices, not identities.
        for (op, zero_is_true) in [(IrCmpOp::Ne, true), (IrCmpOp::Eq, false)] {
            assert!(
                rewrite(&zero_test_select(ty, op, zero_is_true), None).is_none(),
                "{ty:?} {op:?} zero_is_true={zero_is_true}"
            );
        }
    }
}
