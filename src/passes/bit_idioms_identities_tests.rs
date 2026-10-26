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
    // The identity is admitted for signed types only: the backend lowers
    // `SRem` to `idiv` for every type while the constant folder evaluates
    // `U32`/`U64` operands zero-extended, so an unsigned `SRem` is the one
    // shape where the two disagree. The refusal is asserted below.
    for ty in [IrType::I8] {
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
fn remainder_identity_is_refused_for_unsigned_word_types() {
    for ty in [IrType::U8, IrType::U16, IrType::U32, IrType::U64] {
        let ring = mask(ty);
        for divisor in [1, 2, 3, 7, 250, ring] {
            for cmp in [IrCmpOp::Eq, IrCmpOp::Ne] {
                assert!(
                    rewrite(&remainder_pattern(ty, cmp, divisor, false), None).is_none(),
                    "ty={ty:?} c={divisor} op={cmp:?} must stay with the divider"
                );
            }
        }
    }
}

#[test]
fn remainder_identity_holds_at_extremal_divisors_and_widths() {
    for ty in [IrType::I8, IrType::I16, IrType::I32, IrType::I64] {
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
        // The reversed orders select a zero arm on both paths — `x != 0` fills
        // the true arm with zero and `x == 0` leaves the false arm at zero — so
        // they fold to the constant zero rather than being refused.
        for (op, zero_is_true) in [(IrCmpOp::Ne, true), (IrCmpOp::Eq, false)] {
            let replacement = rewrite(&zero_test_select(ty, op, zero_is_true), None)
                .unwrap_or_else(|| panic!("{ty:?} {op:?} zero_is_true={zero_is_true}"));
            assert!(matches!(
                replacement.as_slice(),
                [Instruction::Copy {
                    src: Operand::Const(c),
                    ..
                }] if c.to_i128() == Some(0)
            ));
        }
    }
}

fn or_chain_select(ty: IrType, true_is_value: bool, chain_len: u8) -> Vec<Instruction> {
    // v = Value(0); chain = v | Value(1) | Value(2) ...
    let mut source = Vec::new();
    let mut node = Value(0);
    for step in 0..chain_len {
        let dest = Value(3 + step as u32);
        source.push(bin(
            dest,
            IrBinOp::Or,
            Operand::Value(node),
            Operand::Value(Value(1 + step as u32 % 2)),
            ty,
        ));
        node = dest;
    }
    let zero = constant(0, ty);
    let (true_val, false_val) = if true_is_value {
        (Operand::Value(Value(0)), zero)
    } else {
        (zero, Operand::Value(Value(0)))
    };
    source.push(Instruction::Select {
        dest: Value(7),
        cond: Operand::Value(node),
        true_val,
        false_val,
        ty,
    });
    source
}

#[test]
fn or_chain_selects_keep_the_implied_arm_and_zero_the_other() {
    for ty in [
        IrType::I8,
        IrType::U8,
        IrType::I32,
        IrType::U32,
        IrType::I64,
    ] {
        let ring = mask(ty);
        for chain_len in 1..=3u8 {
            // `(v | ...) ? v : 0` is `v`.
            let source = or_chain_select(ty, true, chain_len);
            let replacement = rewrite(&source, None)
                .unwrap_or_else(|| panic!("{ty:?} chain={chain_len} true-arm"));
            assert!(matches!(
                replacement.as_slice(),
                [Instruction::Copy {
                    src: Operand::Value(Value(0)),
                    ..
                }]
            ));
            // `(v | ...) ? 0 : v` is the constant zero.
            let source = or_chain_select(ty, false, chain_len);
            let replacement = rewrite(&source, None)
                .unwrap_or_else(|| panic!("{ty:?} chain={chain_len} false-arm"));
            assert!(matches!(
                replacement.as_slice(),
                [Instruction::Copy {
                    src: Operand::Const(c),
                    ..
                }] if c.to_i128() == Some(0)
            ));
        }
        // Exhaustive over the byte ring for the shortest chain: the two spellings
        // agree with their folded forms for every pair of operands.
        for v in 0..=255u64 {
            for b in 0..=255u64 {
                let v = v & ring;
                let b = b & ring;
                let cond = v | b;
                let select_true = if cond != 0 { v } else { 0 };
                let select_false = if cond != 0 { 0 } else { v };
                assert_eq!(select_true, v, "{ty:?} v={v} b={b}");
                assert_eq!(select_false, 0, "{ty:?} v={v} b={b}");
            }
        }
    }
}

#[test]
fn or_chain_selects_refuse_an_unrelated_condition() {
    let ty = IrType::I32;
    // The condition no longer mentions the surviving arm.
    let mut source = or_chain_select(ty, true, 2);
    let last = source.len() - 1;
    if let Instruction::Select { cond, .. } = &mut source[last] {
        *cond = Operand::Value(Value(2));
    }
    assert!(rewrite(&source, None).is_none());
    // An `and` condition is not an implication: `v & b != 0` does not follow
    // from `v != 0`.
    let mut source = or_chain_select(ty, true, 1);
    if let Instruction::BinOp { op, .. } = &mut source[0] {
        *op = IrBinOp::And;
    }
    assert!(rewrite(&source, None).is_none());
    // A different type on the condition chain is not the same ring.
    let mut source = or_chain_select(ty, true, 1);
    if let Instruction::BinOp { ty: t, .. } = &mut source[0] {
        *t = IrType::I64;
    }
    assert!(rewrite(&source, None).is_none());
}

fn boolean_product(ty: IrType, factor_shape: u8) -> Vec<Instruction> {
    // x = Value(0); b = a comparison or an or of comparisons; product = x * b.
    let x = Operand::Value(Value(0));
    let cmp_a = Instruction::Cmp {
        dest: Value(1),
        op: IrCmpOp::Ne,
        lhs: x,
        rhs: constant(0, ty),
        ty,
    };
    let cmp_b = Instruction::Cmp {
        dest: Value(2),
        op: IrCmpOp::Ne,
        lhs: Operand::Value(Value(1)),
        rhs: constant(0, ty),
        ty,
    };
    let factor = match factor_shape {
        0 => Operand::Value(Value(1)),
        _ => {
            return vec![
                cmp_a,
                cmp_b,
                bin(
                    Value(3),
                    IrBinOp::Or,
                    Operand::Value(Value(1)),
                    Operand::Value(Value(2)),
                    ty,
                ),
                bin(Value(4), IrBinOp::Mul, x, Operand::Value(Value(3)), ty),
            ];
        }
    };
    vec![cmp_a, bin(Value(4), IrBinOp::Mul, x, factor, ty)]
}

#[test]
fn multiply_by_an_implied_boolean_is_the_value() {
    for ty in [IrType::I8, IrType::I32, IrType::U32] {
        for shape in 0..2u8 {
            let source = boolean_product(ty, shape);
            let replacement =
                rewrite(&source, None).unwrap_or_else(|| panic!("{ty:?} shape={shape}"));
            assert!(matches!(
                replacement.as_slice(),
                [Instruction::Copy {
                    src: Operand::Value(Value(0)),
                    ..
                }]
            ));
        }
        // Exhaustive byte ring: `x * (x != 0)` is `x`, `x * (x != 0 | b != 0)`
        // is `x`, for every pair.
        for x in 0..=255u64 {
            for b in 0..=255u64 {
                let x = x & mask(ty);
                let b = b & mask(ty);
                let plain = (x != 0) as u64;
                let chained = ((x != 0) | (b != 0)) as u64;
                assert_eq!(x.wrapping_mul(plain), x, "{ty:?} x={x}");
                assert_eq!(x.wrapping_mul(chained), x, "{ty:?} x={x} b={b}");
            }
        }
    }
}

#[test]
fn multiply_by_a_non_boolean_or_unrelated_factor_is_refused() {
    let ty = IrType::I32;
    // `x * (x + 1)` is not a boolean product.
    let source = vec![
        bin(
            Value(1),
            IrBinOp::Add,
            Operand::Value(Value(0)),
            constant(1, ty),
            ty,
        ),
        bin(
            Value(2),
            IrBinOp::Mul,
            Operand::Value(Value(0)),
            Operand::Value(Value(1)),
            ty,
        ),
    ];
    assert!(rewrite(&source, None).is_none());
    // A comparison against something else says nothing about `x`, but it does
    // prove the factor is a boolean, so the product becomes a select.
    let source = vec![
        Instruction::Cmp {
            dest: Value(1),
            op: IrCmpOp::Ne,
            lhs: Operand::Value(Value(2)),
            rhs: constant(0, ty),
            ty,
        },
        bin(
            Value(3),
            IrBinOp::Mul,
            Operand::Value(Value(0)),
            Operand::Value(Value(1)),
            ty,
        ),
    ];
    let replacement = rewrite(&source, None).expect("a boolean factor is a select");
    assert!(matches!(
        replacement.as_slice(),
        [Instruction::Select {
            dest: Value(3),
            cond: Operand::Value(Value(1)),
            true_val: Operand::Value(Value(0)),
            false_val: Operand::Const(_),
            ty: IrType::I32,
        }]
    ));
    // The zero arm is what makes the select equal the product for both
    // boolean values of the factor.
    let Instruction::Select { false_val, .. } = &replacement[0] else {
        unreachable!()
    };
    assert!(matches!(false_val, Operand::Const(c) if c.to_i128() == Some(0)));
}

// ── Select/boolean lattice ───────────────────────────────────────────────────
//
// `implied_condition_sense`, `zero_when_zero` and `zero_or_one` decide the
// select and product folds, so the tests below drive whole straight-line
// programs through a small interpreter and compare against the source-level
// value of the same expression.

/// Straight-line interpreter over the instruction subset the tests build.
/// `params` maps a `Value` with no definition to its concrete value.
fn interpret(program: &[Instruction], params: &[(u32, u64)], ty: IrType) -> u64 {
    use std::collections::HashMap;
    let ring = mask(ty);
    let mut env: HashMap<u32, u64> = params.iter().map(|&(v, x)| (v, x & ring)).collect();
    let pick = |env: &HashMap<u32, u64>, op: Operand| match op {
        Operand::Value(v) => *env.get(&v.0).expect("defined before use"),
        Operand::Const(c) => c.to_i128().expect("integer constant") as u64 & ring,
    };
    for inst in program {
        let value = match inst {
            Instruction::Copy { src, .. } => pick(&env, *src),
            Instruction::Cast { src, .. } => pick(&env, *src),
            Instruction::Cmp { op, lhs, rhs, .. } => {
                let (a, b) = (pick(&env, *lhs), pick(&env, *rhs));
                let signed = signed_value(a, ty);
                let signed_b = signed_value(b, ty);
                let held = match op {
                    IrCmpOp::Eq => a == b,
                    IrCmpOp::Ne => a != b,
                    IrCmpOp::Ult => a < b,
                    IrCmpOp::Ule => a <= b,
                    IrCmpOp::Ugt => a > b,
                    IrCmpOp::Uge => a >= b,
                    IrCmpOp::Slt => signed < signed_b,
                    IrCmpOp::Sle => signed <= signed_b,
                    IrCmpOp::Sgt => signed > signed_b,
                    IrCmpOp::Sge => signed >= signed_b,
                };
                u64::from(held)
            }
            Instruction::Select {
                cond,
                true_val,
                false_val,
                ..
            } => {
                if pick(&env, *cond) != 0 {
                    pick(&env, *true_val)
                } else {
                    pick(&env, *false_val)
                }
            }
            Instruction::BinOp { op, lhs, rhs, .. } => {
                let (a, b) = (pick(&env, *lhs), pick(&env, *rhs));
                let raw = match op {
                    IrBinOp::And => a & b,
                    IrBinOp::Or => a | b,
                    IrBinOp::Xor => a ^ b,
                    IrBinOp::Add => a.wrapping_add(b),
                    IrBinOp::Sub => a.wrapping_sub(b),
                    IrBinOp::Mul => a.wrapping_mul(b),
                    _ => panic!("operator outside the tested subset"),
                };
                raw & ring
            }
            Instruction::UnaryOp { op, src, .. } => {
                let a = pick(&env, *src);
                match op {
                    IrUnaryOp::Not => !a & ring,
                    IrUnaryOp::Neg => a.wrapping_neg() & ring,
                    _ => panic!("unary operator outside the tested subset"),
                }
            }
            _ => panic!("instruction outside the tested subset"),
        };
        env.insert(inst.dest().expect("destination").0, value);
    }
    env[&program.last().expect("non-empty").dest().unwrap().0]
}

#[test]
fn nested_boolean_selects_collapse_to_the_surviving_value() {
    // `(a != 0 || b) ? a : 0`, whose `b && ...` arm the front end materialises
    // as a select: the value arm wins whenever the condition is forced, and the
    // other arm is zero whenever the value is.
    let ty = IrType::I32;
    let source = vec![
        Instruction::Cmp {
            dest: Value(1),
            op: IrCmpOp::Ne,
            lhs: Operand::Value(Value(0)),
            rhs: constant(0, ty),
            ty,
        },
        Instruction::Select {
            dest: Value(2),
            cond: Operand::Value(Value(4)),
            true_val: Operand::Value(Value(0)),
            false_val: constant(0, ty),
            ty,
        },
        Instruction::Select {
            dest: Value(3),
            cond: Operand::Value(Value(1)),
            true_val: Operand::Value(Value(0)),
            false_val: Operand::Value(Value(2)),
            ty,
        },
    ];
    let replacement = rewrite(&source, None).expect("the nested select is the value");
    assert!(matches!(
        replacement.as_slice(),
        [Instruction::Copy {
            dest: Value(3),
            src: Operand::Value(Value(0)),
            ..
        }]
    ));
    for a in [0u64, 1, 2, 7, mask(ty)] {
        for b in [0u64, 1, mask(ty)] {
            assert_eq!(
                interpret(&source, &[(0, a), (4, b)], ty),
                interpret(&replacement, &[(0, a), (4, b)], ty),
                "a={a} b={b}"
            );
        }
    }
}

#[test]
fn zero_arm_select_with_a_non_zero_other_arm_is_not_a_constant() {
    // `x ? x : y` keeps `y` on zero `x`: only a zero other arm makes the select
    // the value, so this shape must not fold.
    let ty = IrType::I32;
    let source = vec![Instruction::Select {
        dest: Value(1),
        cond: Operand::Value(Value(0)),
        true_val: Operand::Value(Value(0)),
        false_val: Operand::Value(Value(4)),
        ty,
    }];
    assert!(rewrite(&source, None).is_none());
}

#[test]
fn boolean_multiply_beats_the_multiplier_for_both_factor_shapes() {
    let ty = IrType::I32;
    // `x * ((x != 0) | (b && c))` in the shape the front end emits: the
    // or/and layers arrive as selects over compares.
    let source = vec![
        Instruction::Cmp {
            dest: Value(1),
            op: IrCmpOp::Ne,
            lhs: Operand::Value(Value(4)),
            rhs: constant(0, ty),
            ty,
        },
        Instruction::Select {
            dest: Value(2),
            cond: Operand::Value(Value(5)),
            true_val: Operand::Value(Value(1)),
            false_val: constant(0, ty),
            ty,
        },
        Instruction::Select {
            dest: Value(3),
            cond: Operand::Value(Value(0)),
            true_val: constant(1, ty),
            false_val: Operand::Value(Value(2)),
            ty,
        },
        bin(
            Value(7),
            IrBinOp::Mul,
            Operand::Value(Value(0)),
            Operand::Value(Value(3)),
            ty,
        ),
    ];
    let replacement = rewrite(&source, None).expect("the product is the value");
    assert!(matches!(
        replacement.as_slice(),
        [Instruction::Copy {
            dest: Value(7),
            src: Operand::Value(Value(0)),
            ..
        }]
    ));
    let folded = [source[..source.len() - 1].to_vec(), replacement.clone()].concat();
    for x in [0u64, 1, 3, mask(ty)] {
        for b in [0u64, 1] {
            for c in [0u64, 1] {
                assert_eq!(
                    interpret(&source, &[(0, x), (4, b), (5, c)], ty),
                    interpret(&folded, &[(0, x), (4, b), (5, c)], ty),
                    "x={x} b={b} c={c}"
                );
            }
        }
    }

    // `(x < y) * x` is not the identity, but the factor is a boolean: the
    // product becomes the select and the multiplier disappears.
    let source = vec![
        Instruction::Cmp {
            dest: Value(1),
            op: IrCmpOp::Ult,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Value(Value(4)),
            ty,
        },
        bin(
            Value(2),
            IrBinOp::Mul,
            Operand::Value(Value(1)),
            Operand::Value(Value(0)),
            ty,
        ),
    ];
    let replacement = rewrite(&source, None).expect("a boolean factor is a select");
    assert!(matches!(
        replacement.as_slice(),
        [Instruction::Select {
            dest: Value(2),
            cond: Operand::Value(Value(1)),
            ..
        }]
    ));
    let folded = [source[..source.len() - 1].to_vec(), replacement.clone()].concat();
    for x in [0u64, 1, 5, mask(ty)] {
        for y in [0u64, 1, 5, mask(ty)] {
            assert_eq!(
                interpret(&source, &[(0, x), (4, y)], ty),
                interpret(&folded, &[(0, x), (4, y)], ty),
                "x={x} y={y}"
            );
        }
    }
}

#[test]
fn a_complement_of_a_compare_is_not_a_boolean() {
    // `~1` is not `0` or `1`, so `x * ((x != 0) | ~(y < z))` must not shrink to
    // `x`: the bitwise-or of the test and the complement is all ones for a
    // non-zero `x`, and the product is then `-x`.
    let ty = IrType::I32;
    for factor_producer in [true] {
        let mut source = vec![
            Instruction::Cmp {
                dest: Value(1),
                op: IrCmpOp::Slt,
                lhs: Operand::Value(Value(4)),
                rhs: Operand::Value(Value(5)),
                ty,
            },
            Instruction::Cmp {
                dest: Value(2),
                op: IrCmpOp::Ne,
                lhs: Operand::Value(Value(0)),
                rhs: constant(0, ty),
                ty,
            },
        ];
        let complemented = if factor_producer { Value(3) } else { Value(2) };
        source.extend([
            Instruction::UnaryOp {
                dest: Value(3),
                op: IrUnaryOp::Not,
                src: Operand::Value(Value(1)),
                ty,
            },
            bin(
                Value(6),
                IrBinOp::Or,
                Operand::Value(Value(2)),
                Operand::Value(complemented),
                ty,
            ),
            bin(
                Value(7),
                IrBinOp::Mul,
                Operand::Value(Value(0)),
                Operand::Value(Value(6)),
                ty,
            ),
        ]);
        assert!(
            rewrite(&source, None).is_none(),
            "a complement is not a boolean factor"
        );
        // The source-level product must be `x * ((x != 0) | ~(y < z))`.
        for x in [0u64, 1, 3, mask(ty)] {
            for y in [0u64, 1] {
                for z in [0u64, 2] {
                    let params = [(0, x), (4, y), (5, z)];
                    let complement = !interpret(&source[..1], &params, ty) & mask(ty);
                    let test = u64::from(x != 0);
                    let factor = (test | complement) & mask(ty);
                    assert_eq!(
                        interpret(&source, &params, ty),
                        x.wrapping_mul(factor) & mask(ty)
                    );
                }
            }
        }
    }
}
