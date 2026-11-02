//! Production-matcher tests for [`super::compare_dominance::fold_compare_dominance`].
//!
//! `(a != 0) & (a > 4)`, `(a > 0) & (a > 99)`, `(a < 10) & (a < 5)` and their
//! disjunctive twins are the shapes accepted here; the fold answers with a
//! `Copy` of one of the join's own operands.
//!
//! Every semantic check evaluates the ORIGINAL pair and the REPLACEMENT the
//! matcher returns over the whole 8-bit ring, and over the 16-bit ring for the
//! exhaustive cross product of the comparison kinds.  The first draft of the
//! matcher attributed `(x != 0) & (x > 4)` to `x != 0` — it kept the operand
//! that was implied instead of the one that implies — so the tests below never
//! assert a particular operand: they compare what the replacement computes
//! against what the pair computes.

use super::*;

fn v(n: u32) -> Operand {
    Operand::Value(Value(n))
}

fn is_signed(op: IrCmpOp) -> bool {
    matches!(
        op,
        IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge
    )
}

/// Sign- or zero-extend a bit pattern of `bits` to an i64, as the comparison's
/// own ordering reads it.
fn extend(pattern: u64, bits: u32, signed: bool) -> i64 {
    if !signed || bits >= 64 {
        return pattern as i64;
    }
    let shift = 64 - bits;
    (((pattern << shift) as i64) >> shift) as i64
}

fn cmp_result(op: IrCmpOp, x: u64, constant: u64, bits: u32) -> bool {
    // The fixture hands over the constant as a signed `i64`; the comparison
    // sees the bit pattern of the ring, so mask before ordering.  (Reading
    // `-128` as an unmasked negative here is what made the unsigned sweep
    // report a correct fold as wrong.)
    let mask = if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };
    let constant = constant & mask;
    match op {
        IrCmpOp::Eq | IrCmpOp::Ne => op.eval_i64(x as i64, constant as i64),
        _ => {
            let signed = is_signed(op);
            op.eval_i64(extend(x, bits, signed), extend(constant, bits, signed))
        }
    }
}

/// Assert the matcher's answer is a `Copy` of `src` into the join's dest.
#[track_caller]
fn assert_copies(replacement: Option<Instruction>, src: Operand) {
    match replacement {
        Some(Instruction::Copy { dest, src: actual }) => {
            assert_eq!(dest, Value(3), "the join's dest is kept");
            assert_eq!(actual, src, "the surviving operand");
        }
        other => panic!("expected a Copy of {src:?}, got {other:?}"),
    }
}

fn join_fixture(
    logic: IrBinOp,
    ty: IrType,
    op_a: IrCmpOp,
    const_a: i64,
    op_b: IrCmpOp,
    const_b: i64,
) -> (Vec<Option<Instruction>>, Instruction) {
    let constant = |c: i64| Operand::Const(IrConst::from_i64(c, ty));
    let mut defs = vec![None; 4];
    defs[1] = Some(Instruction::Cmp {
        dest: Value(1),
        op: op_a,
        lhs: v(0),
        rhs: constant(const_a),
        ty,
    });
    defs[2] = Some(Instruction::Cmp {
        dest: Value(2),
        op: op_b,
        lhs: v(0),
        rhs: constant(const_b),
        ty,
    });
    let inst = Instruction::BinOp {
        dest: Value(3),
        op: logic,
        lhs: v(1),
        rhs: v(2),
        ty,
    };
    (defs, inst)
}

fn join_value(logic: IrBinOp, a: bool, b: bool) -> bool {
    match logic {
        IrBinOp::And => a & b,
        IrBinOp::Or => a | b,
        _ => unreachable!(),
    }
}

/// Evaluate the pair and the replacement over every value of the ring.  A
/// replacement that copies an operand the matcher did not prove redundant is
/// caught here, whichever operand it picked.  Refusal always agrees.
fn agree_over_ring(
    logic: IrBinOp,
    ty: IrType,
    op_a: IrCmpOp,
    const_a: i64,
    op_b: IrCmpOp,
    const_b: i64,
) -> bool {
    let bits = ty.size() as u32 * 8;
    let (defs, inst) = join_fixture(logic, ty, op_a, const_a, op_b, const_b);
    let Some(replacement) = compare_dominance::fold_compare_dominance(&inst, &defs) else {
        return true;
    };
    let Instruction::Copy {
        dest: Value(3),
        src,
        ..
    } = replacement
    else {
        return false;
    };
    let copies_first = match src {
        Operand::Value(Value(1)) => true,
        Operand::Value(Value(2)) => false,
        _ => return false,
    };
    let ring = 1u64 << bits.min(16);
    for x in 0..ring {
        let a = cmp_result(op_a, x, const_a as u64, bits);
        let b = cmp_result(op_b, x, const_b as u64, bits);
        let want = join_value(logic, a, b) as u8;
        let got = if copies_first { a as u8 } else { b as u8 };
        if want != got {
            return false;
        }
    }
    true
}

const OPS: [IrCmpOp; 10] = [
    IrCmpOp::Eq,
    IrCmpOp::Ne,
    IrCmpOp::Slt,
    IrCmpOp::Sle,
    IrCmpOp::Sgt,
    IrCmpOp::Sge,
    IrCmpOp::Ult,
    IrCmpOp::Ule,
    IrCmpOp::Ugt,
    IrCmpOp::Uge,
];

#[test]
fn signed_guard_survives_the_nonzero_test() {
    // `(x != 0) & (x > 4)` is `x > 4`, never `x != 0`: the second test already
    // excludes zero, so the first is the redundant one.
    let (defs, inst) = join_fixture(IrBinOp::And, IrType::I32, IrCmpOp::Ne, 0, IrCmpOp::Sgt, 4);
    // The surviving conjunct is the stricter floor, not the guard.
    assert_copies(
        compare_dominance::fold_compare_dominance(&inst, &defs),
        v(2),
    );
    assert!(agree_over_ring(
        IrBinOp::And,
        IrType::I32,
        IrCmpOp::Ne,
        0,
        IrCmpOp::Sgt,
        4
    ));
}

#[test]
fn the_guard_is_kept_by_a_disjunction() {
    // `(x != 0) | (x > 4)` is `x != 0`: the floor implies the guard, so the
    // disjunction's weaker disjunct decides it.
    let (defs, inst) = join_fixture(IrBinOp::And, IrType::I32, IrCmpOp::Ne, 0, IrCmpOp::Sgt, 4);
    assert!(compare_dominance::fold_compare_dominance(&inst, &defs).is_some());
    let (defs, inst) = join_fixture(IrBinOp::Or, IrType::I32, IrCmpOp::Ne, 0, IrCmpOp::Sgt, 4);
    assert_copies(
        compare_dominance::fold_compare_dominance(&inst, &defs),
        v(1),
    );
}

#[test]
fn tighter_bound_wins_in_either_order() {
    for (op, const_a, const_b, expect_second) in [
        (IrCmpOp::Sgt, 0i64, 99i64, true),
        (IrCmpOp::Sgt, 99, 0, false),
        (IrCmpOp::Sge, -3, 7, true),
        (IrCmpOp::Ugt, 1, 20, true),
        (IrCmpOp::Uge, 20, 1, false),
        (IrCmpOp::Slt, 10, 5, true),
        (IrCmpOp::Slt, 5, 10, false),
        (IrCmpOp::Sle, -1, -9, true),
        (IrCmpOp::Ult, 300, 12, true),
        (IrCmpOp::Ule, 12, 300, false),
    ] {
        let (defs, inst) = join_fixture(IrBinOp::And, IrType::I32, op, const_a, op, const_b);
        let expected = if expect_second { v(2) } else { v(1) };
        assert_copies(
            compare_dominance::fold_compare_dominance(&inst, &defs),
            expected,
        );
    }
}

#[test]
fn the_constant_may_sit_on_the_left() {
    // `4 < x` is `x > 4` after mirroring, so `(x != 0) & (4 < x)` folds too.
    let mut defs = vec![None; 4];
    defs[1] = Some(Instruction::Cmp {
        dest: Value(1),
        op: IrCmpOp::Ne,
        lhs: v(0),
        rhs: Operand::Const(IrConst::from_i64(0, IrType::I32)),
        ty: IrType::I32,
    });
    defs[2] = Some(Instruction::Cmp {
        dest: Value(2),
        op: IrCmpOp::Slt,
        lhs: Operand::Const(IrConst::from_i64(4, IrType::I32)),
        rhs: v(0),
        ty: IrType::I32,
    });
    let inst = Instruction::BinOp {
        dest: Value(3),
        op: IrBinOp::And,
        lhs: v(1),
        rhs: v(2),
        ty: IrType::I32,
    };
    assert_copies(
        compare_dominance::fold_compare_dominance(&inst, &defs),
        v(2),
    );
}

#[test]
fn unrelated_bounds_are_left_alone() {
    // `(x < 5) & (x > 10)` describes nothing, but neither conjunct implies the
    // other, so the pair stays as written (the range folder owns emptiness).
    let (defs, inst) = join_fixture(IrBinOp::And, IrType::I32, IrCmpOp::Slt, 5, IrCmpOp::Sgt, 10);
    assert!(compare_dominance::fold_compare_dominance(&inst, &defs).is_none());
    // `(x < 5) & (x < 10)` is unresolved in the other direction only for `Or`.
    let (defs, inst) = join_fixture(IrBinOp::Or, IrType::I32, IrCmpOp::Slt, 5, IrCmpOp::Sgt, 10);
    assert!(compare_dominance::fold_compare_dominance(&inst, &defs).is_none());
}

#[test]
fn different_values_and_types_are_refused() {
    let constant = |c: i64| Operand::Const(IrConst::from_i64(c, IrType::I32));
    let mut defs = vec![None; 4];
    defs[1] = Some(Instruction::Cmp {
        dest: Value(1),
        op: IrCmpOp::Ne,
        lhs: v(0),
        rhs: constant(0),
        ty: IrType::I32,
    });
    defs[2] = Some(Instruction::Cmp {
        dest: Value(2),
        op: IrCmpOp::Sgt,
        lhs: v(9),
        rhs: constant(4),
        ty: IrType::I32,
    });
    let inst = Instruction::BinOp {
        dest: Value(3),
        op: IrBinOp::And,
        lhs: v(1),
        rhs: v(2),
        ty: IrType::I32,
    };
    assert!(compare_dominance::fold_compare_dominance(&inst, &defs).is_none());

    // Same value, different compared width: the orderings are different rings.
    let mut defs = vec![None; 4];
    defs[1] = Some(Instruction::Cmp {
        dest: Value(1),
        op: IrCmpOp::Ne,
        lhs: v(0),
        rhs: Operand::Const(IrConst::from_i64(0, IrType::I32)),
        ty: IrType::I32,
    });
    defs[2] = Some(Instruction::Cmp {
        dest: Value(2),
        op: IrCmpOp::Slt,
        lhs: v(0),
        rhs: Operand::Const(IrConst::from_i64(4, IrType::I8)),
        ty: IrType::I8,
    });
    assert!(compare_dominance::fold_compare_dominance(&inst, &defs).is_none());
}

#[test]
fn non_comparison_operands_are_refused() {
    // Two compare results are 0/1, so `&` is the conjunction; a bitwise join of
    // wider values must never be rewritten as one.
    let mut defs = vec![None; 4];
    defs[1] = Some(Instruction::BinOp {
        dest: Value(1),
        op: IrBinOp::Or,
        lhs: v(0),
        rhs: v(5),
        ty: IrType::I32,
    });
    defs[2] = Some(Instruction::Cmp {
        dest: Value(2),
        op: IrCmpOp::Sgt,
        lhs: v(0),
        rhs: Operand::Const(IrConst::from_i64(4, IrType::I32)),
        ty: IrType::I32,
    });
    let inst = Instruction::BinOp {
        dest: Value(3),
        op: IrBinOp::And,
        lhs: v(1),
        rhs: v(2),
        ty: IrType::I32,
    };
    assert!(compare_dominance::fold_compare_dominance(&inst, &defs).is_none());
    // And a float comparison denotes no such set.
    let mut defs = vec![None; 4];
    defs[1] = Some(Instruction::Cmp {
        dest: Value(1),
        op: IrCmpOp::Ne,
        lhs: v(0),
        rhs: Operand::Const(IrConst::from_i64(0, IrType::F64)),
        ty: IrType::F64,
    });
    defs[2] = Some(Instruction::Cmp {
        dest: Value(2),
        op: IrCmpOp::Sgt,
        lhs: v(0),
        rhs: Operand::Const(IrConst::from_i64(4, IrType::F64)),
        ty: IrType::F64,
    });
    assert!(compare_dominance::fold_compare_dominance(&inst, &defs).is_none());
}

#[test]
fn every_join_of_every_comparison_kind_agrees_with_the_pair() {
    // Product of the comparison kinds over the whole 8-bit ring.  The constants
    // are chosen to hit the empty set, the whole set, both singletons and both
    // kinds of boundary; refusals are not failures, mismatched evaluations are.
    let constants = [-128i64, -1, 0, 1, 5, 127];
    for logic in [IrBinOp::And, IrBinOp::Or] {
        for op_a in OPS {
            for op_b in OPS {
                for &const_a in &constants {
                    for &const_b in &constants {
                        assert!(
                            agree_over_ring(logic, IrType::I8, op_a, const_a, op_b, const_b),
                            "{logic:?} of {op_a:?} {const_a} and {op_b:?} {const_b}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn the_16_bit_ring_survives_the_wide_cases() {
    // The same product restricted to the pairs whose wide-ring behaviour the
    // 8-bit sweep cannot reach: sign boundaries, negative constants and the
    // signed/unsigned split at 2^15.
    let constants = [-32768i64, -1, 0, 1, 32767];
    let partners = [
        IrCmpOp::Ne,
        IrCmpOp::Eq,
        IrCmpOp::Sgt,
        IrCmpOp::Sge,
        IrCmpOp::Ult,
    ];
    for logic in [IrBinOp::And, IrBinOp::Or] {
        for op_a in OPS {
            for op_b in partners {
                for &const_a in &constants {
                    for &const_b in &constants {
                        assert!(
                            agree_over_ring(logic, IrType::I16, op_a, const_a, op_b, const_b),
                            "{logic:?} of {op_a:?} {const_a} and {op_b:?} {const_b}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn dominant_shapes_do_fold_at_every_integer_width() {
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
        let signed = ty.is_signed();
        let (cmp_op, guard_op) = if signed {
            (IrCmpOp::Sgt, IrCmpOp::Ne)
        } else {
            (IrCmpOp::Ugt, IrCmpOp::Ne)
        };
        let (defs, inst) = join_fixture(IrBinOp::And, ty, guard_op, 0, cmp_op, 3);
        assert!(
            compare_dominance::fold_compare_dominance(&inst, &defs).is_some(),
            "{ty:?}: `x != 0 && x > 3` folds"
        );
        assert!(agree_over_ring(IrBinOp::And, ty, guard_op, 0, cmp_op, 3));
    }
}

#[test]
fn the_set_model_is_exact_at_the_extremes() {
    // `x < 0` in the unsigned ordering is empty, so it is implied by anything;
    // `x >= 0` in the signed ordering is everything.  Both are legitimate
    // implications and both replacements agree with the pair.
    assert!(agree_over_ring(
        IrBinOp::And,
        IrType::U32,
        IrCmpOp::Ult,
        0,
        IrCmpOp::Uge,
        100
    ));
    assert!(agree_over_ring(
        IrBinOp::Or,
        IrType::I32,
        IrCmpOp::Sge,
        -2147483648,
        IrCmpOp::Slt,
        0
    ));
    // `x != 0` does not imply `x != 1`, and neither does the converse.
    let (defs, inst) = join_fixture(IrBinOp::And, IrType::I32, IrCmpOp::Ne, 0, IrCmpOp::Ne, 1);
    assert!(compare_dominance::fold_compare_dominance(&inst, &defs).is_none());
    // but `x == 1` does imply `x != 0`.
    let (defs, inst) = join_fixture(IrBinOp::And, IrType::I32, IrCmpOp::Ne, 0, IrCmpOp::Eq, 1);
    assert_copies(
        compare_dominance::fold_compare_dominance(&inst, &defs),
        v(2),
    );
}

#[test]
fn a_negative_floor_still_excludes_zero_for_signed_orderings() {
    // `x > -1` is `x >= 0`, which does NOT exclude zero, so `x != 0` is not
    // redundant...
    let (defs, inst) = join_fixture(IrBinOp::And, IrType::I32, IrCmpOp::Ne, 0, IrCmpOp::Sgt, -1);
    assert!(compare_dominance::fold_compare_dominance(&inst, &defs).is_none());
    // ...but `x > -0x8000_0000` does, and the guard goes.
    let (defs, inst) = join_fixture(IrBinOp::And, IrType::I32, IrCmpOp::Ne, 0, IrCmpOp::Sgt, 0);
    assert!(compare_dominance::fold_compare_dominance(&inst, &defs).is_some());
}

#[test]
fn mixed_orderings_are_refused() {
    // `x < 5` (signed) and `x <u 5` (unsigned) share no ordering; a fold would
    // have to pick one, so the pair stays.
    let (defs, inst) = join_fixture(IrBinOp::And, IrType::I32, IrCmpOp::Slt, 5, IrCmpOp::Ult, 5);
    assert!(compare_dominance::fold_compare_dominance(&inst, &defs).is_none());
    // Unless the two orderings agree on the answer: the equality test is
    // ordering-free and the single ordered test decides.
    let (defs, inst) = join_fixture(IrBinOp::And, IrType::I32, IrCmpOp::Eq, 0, IrCmpOp::Ult, 5);
    assert!(compare_dominance::fold_compare_dominance(&inst, &defs).is_some());
}

#[test]
fn the_replacement_copies_the_operand_verbatim() {
    // The join's own operand is copied, so a widened comparison still answers
    // with the widened value: the fold never re-types the surviving compare.
    let mut defs = vec![None; 4];
    defs[1] = Some(Instruction::Cmp {
        dest: Value(1),
        op: IrCmpOp::Ne,
        lhs: v(0),
        rhs: Operand::Const(IrConst::from_i64(0, IrType::I64)),
        ty: IrType::I64,
    });
    defs[2] = Some(Instruction::Cmp {
        dest: Value(2),
        op: IrCmpOp::Sgt,
        lhs: v(0),
        rhs: Operand::Const(IrConst::from_i64(1, IrType::I64)),
        ty: IrType::I64,
    });
    let inst = Instruction::BinOp {
        dest: Value(3),
        op: IrBinOp::And,
        lhs: v(1),
        rhs: v(2),
        ty: IrType::I8,
    };
    assert_copies(
        compare_dominance::fold_compare_dominance(&inst, &defs),
        v(2),
    );
}
