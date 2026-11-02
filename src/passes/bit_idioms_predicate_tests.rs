//! Tests for [`super::predicate::fold_predicate_select`].
//!
//! The fold answers `select(c, 1, 0)` with `c != 0` and `select(c, 0, 1)`
//! with `c == 0`.  Every semantic check evaluates the ORIGINAL instruction
//! stream and the REPLACEMENT over the whole input domain of the narrow
//! widths (and over an edge set for 32/64 bits), because the one thing that
//! must never drift is which arm a given condition selects: the arms are the
//! only place the boolean value is spelled out.
//!
//! The negative tests pin the refusals.  Each of them exists for a different
//! reason -- unequal/equal arms are not comparisons at all, a narrower
//! condition would need a widening value, a float result has no `setcc`, and
//! an inverted comparison may only be created when the original comparison
//! has no other reader -- so a later relaxation of one must not silently
//! widen another.

use super::*;

fn v(n: u32) -> Operand {
    Operand::Value(Value(n))
}

/// `Instruction` has no `PartialEq`; the debug form names every field, which
/// is exactly what a shape assertion has to pin.
fn shape(found: Option<Instruction>) -> Option<String> {
    found.map(|inst| format!("{inst:?}"))
}

fn expect_shape(found: Option<Instruction>, want: Instruction) -> bool {
    shape(found).as_deref() == Some(format!("{want:?}").as_str())
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

fn signed(x: u64, w: usize) -> i64 {
    if w == 64 {
        x as i64
    } else {
        let shift = 64 - w;
        ((x << shift) as i64) >> shift
    }
}

fn compare_bits(op: IrCmpOp, a: u64, b: u64, w: usize) -> bool {
    match op {
        IrCmpOp::Eq => a == b,
        IrCmpOp::Ne => a != b,
        IrCmpOp::Ult => a < b,
        IrCmpOp::Ule => a <= b,
        IrCmpOp::Ugt => a > b,
        IrCmpOp::Uge => a >= b,
        IrCmpOp::Slt => signed(a, w) < signed(b, w),
        IrCmpOp::Sle => signed(a, w) <= signed(b, w),
        IrCmpOp::Sgt => signed(a, w) > signed(b, w),
        IrCmpOp::Sge => signed(a, w) >= signed(b, w),
    }
}

/// Evaluate a straight-line stream.  `Value(n)` for `n < inputs.len()` is the
/// `n`-th input; `Select` tests its condition for zero-ness the way the IR
/// contract does (a condition is a truthiness, not a `0`/`1` carrier).
fn evaluate(code: &[Instruction], inputs: &[u64], result: Value) -> u64 {
    let mut values = [0u64; 32];
    values[..inputs.len()].copy_from_slice(inputs);
    let get = |op: Operand, w: usize, values: &[u64; 32]| match op {
        Operand::Value(value) => {
            let mask = if w == 64 { u64::MAX } else { (1u64 << w) - 1 };
            values[value.0 as usize] & mask
        }
        Operand::Const(_) => const_in_ring(op, w).expect("constant in ring"),
    };
    for inst in code {
        let (dest, value) = match inst {
            Instruction::ParamRef {
                dest,
                param_idx,
                ty,
            } => {
                let mask = if ty.size() * 8 == 64 {
                    u64::MAX
                } else {
                    (1u64 << (ty.size() * 8)) - 1
                };
                (*dest, inputs[*param_idx] & mask)
            }
            Instruction::Copy { dest, src } => (*dest, get(*src, 64, &values)),
            Instruction::Cmp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            } => {
                let w = ty.size() * 8;
                (
                    *dest,
                    u64::from(compare_bits(
                        *op,
                        get(*lhs, w, &values),
                        get(*rhs, w, &values),
                        w,
                    )),
                )
            }
            Instruction::BinOp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            } => {
                let w = ty.size() * 8;
                let mask = if w == 64 { u64::MAX } else { (1u64 << w) - 1 };
                let a = get(*lhs, w, &values);
                let b = get(*rhs, w, &values);
                let value = match op {
                    IrBinOp::Or => a | b,
                    IrBinOp::And => a & b,
                    _ => panic!("unsupported evaluator operation: {op:?}"),
                };
                (*dest, value & mask)
            }
            Instruction::Select {
                dest,
                cond,
                true_val,
                false_val,
                ty,
            } => {
                let w = ty.size() * 8;
                let chosen = if get(*cond, w, &values) != 0 {
                    *true_val
                } else {
                    *false_val
                };
                (*dest, get(chosen, w, &values))
            }
            _ => panic!("unsupported evaluator instruction: {inst:?}"),
        };
        values[dest.0 as usize] = value;
    }
    values[result.0 as usize]
}

/// Run the fold against a definition table derived from `code`.
fn fold_last(code: &[Instruction], uses: &[(u32, u32)]) -> Option<Instruction> {
    let mut defs = vec![None; 32];
    let mut counted = vec![0u32; 32];
    for inst in code {
        if let Some(dest) = inst.dest() {
            defs[dest.0 as usize] = Some(inst.clone());
        }
        crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
            if let Operand::Value(value) = op {
                counted[value.0 as usize] += 1;
            }
        });
    }
    // Callers override the reader count of the comparison they want to guard.
    for (value, count) in uses {
        counted[*value as usize] = *count;
    }
    super::predicate::fold_predicate_select(code.last().unwrap(), &defs, &counted)
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

/// Values worth testing for a width: the whole ring at 8 bits (where every
/// condition/operand pair is enumerable) and an edge set everywhere else.
fn inputs(w: usize) -> Vec<u64> {
    if w <= 8 {
        return (0..(1u64 << w)).collect();
    }
    let mut set = vec![
        0,
        1,
        2,
        0x7f,
        0x80,
        0xff,
        0x100,
        0x7fff,
        0x8000,
        0xffff,
        0x10000,
        0x7fff_ffff,
        0x8000_0000,
        0xffff_ffff,
    ];
    if w == 64 {
        set.extend([
            0x1_0000_0000,
            0x7fff_ffff_ffff_ffff,
            0x8000_0000_0000_0000,
            0xffff_ffff_ffff_ffff,
        ]);
    }
    set.into_iter()
        .map(|x| if w == 64 { x } else { x & ((1u64 << w) - 1) })
        .collect()
}

/// Shapes of a condition worth folding, as (code prefix, condition operand).
fn condition_shapes(ty: IrType) -> Vec<(Vec<Instruction>, Operand)> {
    let zero = Operand::Const(IrConst::from_i64(0, ty));
    let params = vec![
        Instruction::ParamRef {
            dest: Value(0),
            param_idx: 0,
            ty,
        },
        Instruction::ParamRef {
            dest: Value(1),
            param_idx: 1,
            ty,
        },
    ];
    let mut shapes = vec![(params.clone(), v(1))];
    let mut compared = params.clone();
    compared.push(cmp(2, IrCmpOp::Slt, v(1), zero, ty));
    shapes.push((compared, v(2)));
    let mut copied = params;
    copied.push(cmp(2, IrCmpOp::Ne, v(1), v(0), ty));
    copied.push(Instruction::Copy {
        dest: Value(3),
        src: v(2),
    });
    shapes.push((copied, v(3)));
    shapes
}

#[test]
fn materialization_agrees_with_the_select_it_replaces() {
    for ty in types() {
        let w = ty.size() * 8;
        let cases = inputs(w);
        for positive in [true, false] {
            let (yes, no) = if positive { (1, 0) } else { (0, 1) };
            for (prefix, cond) in condition_shapes(ty) {
                let mut code = prefix.clone();
                code.push(sel(
                    9,
                    cond,
                    Operand::Const(IrConst::from_i64(yes, ty)),
                    Operand::Const(IrConst::from_i64(no, ty)),
                    ty,
                ));
                let replacement = fold_last(&code, &[]).unwrap_or_else(|| {
                    panic!("no fold for {ty:?} positive={positive} cond={cond:?}")
                });
                let mut folded_code = prefix.clone();
                folded_code.push(replacement.clone());
                for &x in &cases {
                    for &y in &cases {
                        let original = evaluate(&code, &[x, y], Value(9));
                        let folded = evaluate(&folded_code, &[x, y], Value(9));
                        assert_eq!(
                            original, folded,
                            "{ty:?} positive={positive} cond={cond:?} x={x:#x} y={y:#x}"
                        );
                    }
                }
            }
        }
    }
}

/// Run the merge fold against a definition table derived from `code`, with
/// explicit reader counts for the two comparisons.
fn fold_merge(
    code: &[Instruction],
    uses: &[(u32, u32)],
    fresh: u32,
) -> Option<(Vec<Instruction>, Instruction)> {
    let mut defs = vec![None; 32];
    let mut counted = vec![0u32; 32];
    for inst in code {
        if let Some(dest) = inst.dest() {
            defs[dest.0 as usize] = Some(inst.clone());
        }
        crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
            if let Operand::Value(value) = op {
                counted[value.0 as usize] += 1;
            }
        });
    }
    for (value, count) in uses {
        counted[*value as usize] = *count;
    }
    super::predicate::fold_predicate_merge(code.last().unwrap(), &defs, &counted, Value(fresh))
}

/// The two-comparison merge fixture: `(a op 0) <binop> (b op 0)`.
fn merge_fixture(ty: IrType, cmp_op: IrCmpOp, binop: IrBinOp) -> Vec<Instruction> {
    let zero = Operand::Const(IrConst::from_i64(0, ty));
    vec![
        Instruction::ParamRef {
            dest: Value(0),
            param_idx: 0,
            ty,
        },
        Instruction::ParamRef {
            dest: Value(1),
            param_idx: 1,
            ty,
        },
        cmp(2, cmp_op, v(0), zero, ty),
        cmp(3, cmp_op, v(1), zero, ty),
        Instruction::BinOp {
            dest: Value(4),
            op: binop,
            lhs: v(2),
            rhs: v(3),
            ty,
        },
    ]
}

#[test]
fn disjunction_merge_agrees_with_the_selector_it_replaces() {
    for ty in types() {
        let w = ty.size() * 8;
        for (cmp_op, binop) in [(IrCmpOp::Ne, IrBinOp::Or), (IrCmpOp::Eq, IrBinOp::And)] {
            let code = merge_fixture(ty, cmp_op, binop);
            let (prelude, result) = fold_merge(&code, &[(2, 1), (3, 1)], 20)
                .unwrap_or_else(|| panic!("no merge for {ty:?} {cmp_op:?} {binop:?}"));
            let mut folded = code[..2].to_vec();
            folded.extend(prelude);
            folded.push(result);
            for &x in &inputs(w) {
                for &y in &inputs(w) {
                    let original = evaluate(&code, &[x, y], Value(4));
                    let merged = evaluate(&folded, &[x, y], Value(4));
                    assert_eq!(
                        original, merged,
                        "{ty:?} {cmp_op:?} {binop:?} x={x:#x} y={y:#x}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_two_mixed_and_conjunctive_forms_are_refused() {
    // `(a != 0) & (b != 0)` is NOT `(a & b) != 0` (a = 1, b = 2 is the
    // counter-example), and `(a == 0) | (b == 0)` is not a zero test of any
    // conjunction either.  Both must be refused, not approximated.
    for (cmp_op, binop) in [
        (IrCmpOp::Ne, IrBinOp::And),
        (IrCmpOp::Eq, IrBinOp::Or),
        (IrCmpOp::Ne, IrBinOp::Xor),
        (IrCmpOp::Eq, IrBinOp::Xor),
    ] {
        let code = merge_fixture(IrType::I32, cmp_op, binop);
        assert!(
            fold_merge(&code, &[(2, 1), (3, 1)], 20).is_none(),
            "{cmp_op:?} {binop:?}"
        );
    }
}

#[test]
fn shared_or_nonzero_comparisons_are_refused() {
    let code = merge_fixture(IrType::I32, IrCmpOp::Ne, IrBinOp::Or);
    for uses in [(2, 1), (1, 2), (2, 2)] {
        assert!(
            fold_merge(&code, &[(2, uses.0), (3, uses.1)], 20).is_none(),
            "uses {uses:?}"
        );
    }
}

#[test]
fn non_zero_comparisons_are_refused() {
    // `(a != 4) | (b != 0)` carries no zero test for the left half.
    let ty = IrType::I32;
    let code = vec![
        Instruction::ParamRef {
            dest: Value(0),
            param_idx: 0,
            ty,
        },
        Instruction::ParamRef {
            dest: Value(1),
            param_idx: 1,
            ty,
        },
        cmp(
            2,
            IrCmpOp::Ne,
            v(0),
            Operand::Const(IrConst::from_i64(4, ty)),
            ty,
        ),
        cmp(
            3,
            IrCmpOp::Ne,
            v(1),
            Operand::Const(IrConst::from_i64(0, ty)),
            ty,
        ),
        Instruction::BinOp {
            dest: Value(4),
            op: IrBinOp::Or,
            lhs: v(2),
            rhs: v(3),
            ty,
        },
    ];
    assert!(fold_merge(&code, &[(2, 1), (3, 1)], 20).is_none());
}

#[test]
fn selector_width_must_match_the_disjunction() {
    // 8-bit comparisons feeding a 32-bit OR: the merge would change the
    // width of the disjunction and the meaning of "zero" with it.
    let code = vec![
        Instruction::ParamRef {
            dest: Value(0),
            param_idx: 0,
            ty: IrType::I8,
        },
        Instruction::ParamRef {
            dest: Value(1),
            param_idx: 1,
            ty: IrType::I8,
        },
        cmp(
            2,
            IrCmpOp::Ne,
            v(0),
            Operand::Const(IrConst::from_i64(0, IrType::I8)),
            IrType::I8,
        ),
        cmp(
            3,
            IrCmpOp::Ne,
            v(1),
            Operand::Const(IrConst::from_i64(0, IrType::I8)),
            IrType::I8,
        ),
        Instruction::BinOp {
            dest: Value(4),
            op: IrBinOp::Or,
            lhs: v(2),
            rhs: v(3),
            ty: IrType::U32,
        },
    ];
    assert!(fold_merge(&code, &[(2, 1), (3, 1)], 20).is_none());
}

#[test]
fn positive_select_copies_the_comparison() {
    let ty = IrType::I32;
    let zero = Operand::Const(IrConst::from_i64(0, ty));
    let code = vec![
        cmp(2, IrCmpOp::Sgt, v(0), v(1), ty),
        sel(
            3,
            v(2),
            Operand::Const(IrConst::from_i64(1, ty)),
            Operand::Const(IrConst::from_i64(0, ty)),
            ty,
        ),
    ];
    let _ = zero;
    assert!(expect_shape(
        fold_last(&code, &[]),
        Instruction::Copy {
            dest: Value(3),
            src: v(2)
        }
    ));
}

#[test]
fn single_use_negative_select_inverts_the_comparison() {
    let ty = IrType::I32;
    let code = vec![
        cmp(2, IrCmpOp::Slt, v(0), v(1), ty),
        sel(
            3,
            v(2),
            Operand::Const(IrConst::from_i64(0, ty)),
            Operand::Const(IrConst::from_i64(1, ty)),
            ty,
        ),
    ];
    assert!(expect_shape(
        fold_last(&code, &[(2, 1)]),
        Instruction::Cmp {
            dest: Value(3),
            op: IrCmpOp::Sge,
            lhs: v(0),
            rhs: v(1),
            ty,
        }
    ));
}

#[test]
fn shared_negative_select_keeps_the_select() {
    let ty = IrType::I32;
    let code = vec![
        cmp(2, IrCmpOp::Slt, v(0), v(1), ty),
        sel(
            3,
            v(2),
            Operand::Const(IrConst::from_i64(0, ty)),
            Operand::Const(IrConst::from_i64(1, ty)),
            ty,
        ),
    ];
    // Two readers: the comparison stays live for the other one, so inverting
    // it here would emit a second compare.  The condition is already a
    // boolean, so a zero test on top of it would pay for a `setcc` it does
    // not need either -- the select's `cmov` form is kept.
    assert!(fold_last(&code, &[(2, 2)]).is_none());
}

#[test]
fn foreign_constant_carriers_are_read_in_the_select_ring() {
    // `c ? 1u : 0u` lowers with I64 arm constants on a U32 select; the fold
    // must read them in the select's ring, not refuse the mismatch.
    let ty = IrType::U32;
    let code = vec![
        Instruction::ParamRef {
            dest: Value(1),
            param_idx: 0,
            ty: IrType::I32,
        },
        sel(
            3,
            v(1),
            Operand::Const(IrConst::I64(1)),
            Operand::Const(IrConst::I64(0)),
            ty,
        ),
    ];
    assert!(expect_shape(
        fold_last(&code, &[]),
        Instruction::Cmp {
            dest: Value(3),
            op: IrCmpOp::Ne,
            lhs: v(1),
            rhs: Operand::Const(IrConst::from_i64(0, ty)),
            ty,
        }
    ));
}

#[test]
fn cast_carried_boolean_is_restated_not_retested() {
    // `(int)t` for a `_Bool`-shaped `t`: the condition is already `0`/`1`, so
    // the positive polarity only restates its carrier and the negative one
    // keeps the select (inverting would materialize a second boolean).
    let narrow = IrType::I8;
    let ty = IrType::I32;
    let mut prefix = vec![
        Instruction::ParamRef {
            dest: Value(0),
            param_idx: 0,
            ty,
        },
        cmp(
            1,
            IrCmpOp::Slt,
            v(0),
            Operand::Const(IrConst::from_i64(0, ty)),
            ty,
        ),
        Instruction::Cast {
            dest: Value(2),
            src: v(1),
            from_ty: narrow,
            to_ty: IrType::U8,
        },
        Instruction::Copy {
            dest: Value(3),
            src: v(2),
        },
    ];
    let mut positive = prefix.clone();
    positive.push(sel(
        4,
        v(3),
        Operand::Const(IrConst::from_i64(1, ty)),
        Operand::Const(IrConst::from_i64(0, ty)),
        ty,
    ));
    assert!(expect_shape(
        fold_last(&positive, &[]),
        Instruction::Cast {
            dest: Value(4),
            src: v(3),
            from_ty: IrType::U8,
            to_ty: ty,
        }
    ));
    prefix.push(sel(
        4,
        v(3),
        Operand::Const(IrConst::from_i64(0, ty)),
        Operand::Const(IrConst::from_i64(1, ty)),
        ty,
    ));
    assert!(fold_last(&prefix, &[]).is_none());
}

#[test]
fn same_width_signedness_alone_does_not_block_the_inversion() {
    // `a <=u b ? 0 : 1` returns an `int` from a `u32` comparison: the `0`/`1`
    // value is bit-identical at the same width, so the inverted comparison is
    // still the exact answer.
    let code = vec![
        cmp(2, IrCmpOp::Ule, v(0), v(1), IrType::U32),
        sel(
            3,
            v(2),
            Operand::Const(IrConst::from_i64(0, IrType::I32)),
            Operand::Const(IrConst::from_i64(1, IrType::I32)),
            IrType::I32,
        ),
    ];
    assert!(expect_shape(
        fold_last(&code, &[(2, 1)]),
        Instruction::Cmp {
            dest: Value(3),
            op: IrCmpOp::Ugt,
            lhs: v(0),
            rhs: v(1),
            ty: IrType::I32,
        }
    ));
}

#[test]
fn equal_arms_are_not_a_comparison() {
    for arms in [(1, 1), (0, 0)] {
        let ty = IrType::I32;
        let code = vec![sel(
            3,
            v(1),
            Operand::Const(IrConst::from_i64(arms.0, ty)),
            Operand::Const(IrConst::from_i64(arms.1, ty)),
            ty,
        )];
        assert!(fold_last(&code, &[]).is_none(), "arms {arms:?}");
    }
}

#[test]
fn non_boolean_arms_are_refused() {
    let ty = IrType::I32;
    for arms in [(5, 3), (1, 2), (2, 1), (-1, 0), (1, -1)] {
        let code = vec![sel(
            3,
            v(1),
            Operand::Const(IrConst::from_i64(arms.0, ty)),
            Operand::Const(IrConst::from_i64(arms.1, ty)),
            ty,
        )];
        assert!(fold_last(&code, &[]).is_none(), "arms {arms:?}");
    }
}

#[test]
fn narrow_condition_is_refused() {
    // A condition narrower than the result would have to be widened first;
    // the fold has no value to widen into.
    let code = vec![
        Instruction::ParamRef {
            dest: Value(1),
            param_idx: 0,
            ty: IrType::I8,
        },
        sel(
            3,
            v(1),
            Operand::Const(IrConst::from_i64(1, IrType::I32)),
            Operand::Const(IrConst::from_i64(0, IrType::I32)),
            IrType::I32,
        ),
    ];
    assert!(fold_last(&code, &[]).is_none());
}

#[test]
fn float_result_is_refused() {
    for ty in [IrType::F32, IrType::F64] {
        let code = vec![sel(
            3,
            v(1),
            Operand::Const(IrConst::I64(1)),
            Operand::Const(IrConst::I64(0)),
            ty,
        )];
        assert!(fold_last(&code, &[]).is_none(), "{ty:?}");
    }
}

#[test]
fn constant_condition_is_refused() {
    let ty = IrType::I32;
    let code = vec![sel(
        3,
        Operand::Const(IrConst::from_i64(7, ty)),
        Operand::Const(IrConst::from_i64(1, ty)),
        Operand::Const(IrConst::from_i64(0, ty)),
        ty,
    )];
    // A constant condition is a constant fold, not a comparison.
    assert!(fold_last(&code, &[]).is_none());
}

#[test]
fn unknown_producers_fail_closed() {
    // The condition is produced by something whose carrier type the fold
    // cannot see; it must refuse rather than guess a width.
    let ty = IrType::I32;
    let code = vec![
        Instruction::GlobalAddr {
            dest: Value(1),
            name: "g".into(),
        },
        sel(
            3,
            v(1),
            Operand::Const(IrConst::from_i64(1, ty)),
            Operand::Const(IrConst::from_i64(0, ty)),
            ty,
        ),
    ];
    assert!(fold_last(&code, &[]).is_none());
}
