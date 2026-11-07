//! Sign-magnitude lowering: absolute value and absolute difference become a
//! mask, a XOR and a subtract.
//!
//! `x < 0 ? -x : x` is what a C programmer writes, and what the frontend hands
//! over: a signed compare, a negation and a select.  On x86-64 that shape
//! lowers to a test, a second register copy and a conditional move — six
//! instructions where GCC and Clang both produce four — and on RISC-V, where
//! there is no conditional move, to a branch.  The branch-free arithmetic
//! identity costs no compare at all:
//!
//! ```text
//! s = x >>s (W-1)      // all ones when x is negative, zero otherwise
//! t = x ^ s            // magnitude of a negative value, x itself otherwise
//! r = t - s            // add one back for the negative case
//! ```
//!
//! Every step is exact in the ring, including at the type minimum: `Neg` of
//! `INT_MIN` wraps to `INT_MIN`, and the identity also yields `INT_MIN`, so no
//! undefined-behaviour argument is needed — the two forms agree on every input
//! the IR can represent.
//!
//! The same identity expresses an absolute difference.  `a > b ? a - b : b - a`
//! computes `|a - b|` in the ring once the difference is taken modulo `2^W`:
//!
//! ```text
//! d = a - b            // wraps; the sign bit tells which side is larger
//! s = d >>s (W-1)
//! r = (d ^ s) - s
//! ```
//!
//! The compare may test the sign with or without zero (`x < 0`, `x <= 0`,
//! `x > 0`, `x >= 0`, and their `-1`/`1` spellings): at zero both arms are 0,
//! so every one of them selects `|x|`.  GCC 16.2 and Clang 23.1.4 emit the
//! same `neg; cmovs` for all eight.
//!
//! Both rewrites are admitted only for the 32-bit and 64-bit signed integer
//! types the C frontend uses for `int` and `long`, only for a select whose
//! arms are exactly `Neg(x)` and `x` (or the two subtractions of the same
//! pair, in either order), and only when the compared value is the same value
//! the arms operate on -- or the narrower signed value they sign-extend.  The
//! latter is how C spells `|x|` for a `short` or `signed char` `x`: the
//! compare runs at the operand's width (`cmp.i16 x, 0`) and the arms at the
//! promoted width (`neg.i32 sext(x)`, `sext(x)`); a sign extension preserves
//! order, so the two compares agree on every input.  A narrow select is
//! refused: the frontend promotes arithmetic to `int`, so a narrow select is
//! a different program, and a 8-bit `AShr` mask is not the abstraction the
//! backend expects here.
//!
//! The identity is what GCC 16.2 and Clang 23.1.4 emit on RISC-V (`sraiw;
//! xor; subw`), and it stays the final shape on i686, whose baseline has no
//! `cmov`; on x86-64 the peephole `abs_cmov` turns it into the shorter
//! `mov; neg; cmovs` both oracles emit there.
//!
//! Nothing is deleted: the pattern's own compare and negation stay in place
//! for any other reader, and DCE retires them when this select was their last
//! consumer.  Only dead code is created (the intermediate mask), never a
//! duplicated live computation.

use crate::common::types::IrType;
use crate::ir::reexports::{
    Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, IrUnaryOp, Operand, Value,
};

/// `s = x >>s (W-1)` as a constant operand.
fn sign_mask_amount(ty: IrType) -> IrConst {
    IrConst::from_i64(i64::from(ty.size() as u32 * 8 - 1), ty)
}

fn is_neg_of(inst: Option<&Instruction>, source: Value) -> bool {
    matches!(
        inst,
        Some(Instruction::UnaryOp {
            op: IrUnaryOp::Neg,
            src: Operand::Value(v),
            ..
        }) if *v == source
    )
}

fn is_sub_of(inst: Option<&Instruction>, lhs: Value, rhs: Value, ty: IrType) -> bool {
    matches!(
        inst,
        Some(Instruction::BinOp {
            op: IrBinOp::Sub,
            lhs: Operand::Value(a),
            rhs: Operand::Value(b),
            ty: t,
            ..
        }) if *a == lhs && *b == rhs && *t == ty
    )
}

fn const_is(operand: &Operand, value: i64) -> bool {
    matches!(operand, Operand::Const(c) if c.to_i64() == Some(value))
}

/// Whether a signed compare of `v` at type `t` orders exactly like the same
/// compare of `x` at type `ty`: `v` is `x` itself, or `x` is the sign
/// extension `sext(v)` from the narrower signed type `t`.  A sign extension
/// is strictly monotone -- it maps every value of `t` to the same integer in
/// `ty` -- so `v < c` and `x < c` agree for every constant `c` that `t` holds
/// (the accepted thresholds are -1, 0 and 1, which every signed type holds),
/// and `a > b` agrees with `sext(a) > sext(b)`.  This is the shape C hands
/// over for a `short` or `signed char` operand: the compare is narrowed to
/// the operand's own width while the arithmetic is promoted to `int`.  A zero
/// extension is refused: it reorders the negative half of a signed `t`.
fn same_signed_order(
    defs: &[Option<Instruction>],
    v: Value,
    t: IrType,
    x: Value,
    ty: IrType,
) -> bool {
    if v == x {
        return t == ty;
    }
    matches!(
        defs.get(x.0 as usize).and_then(Option::as_ref),
        Some(Instruction::Cast {
            src: Operand::Value(n),
            from_ty,
            to_ty,
            ..
        }) if *n == v
            && *from_ty == t
            && *to_ty == ty
            && t.is_signed()
            && t.size() < ty.size()
    )
}

/// The compare selecting the `-x` arm: true for every negative `x`, false
/// for every positive one.  At `x == 0` the two arms agree (`-0 == 0`), so
/// the compare may go either way there: `x < 0` and `x <= -1` exclude zero,
/// `x <= 0` and `x < 1` include it, and all four select `|x|`.
fn proves_negative(
    defs: &[Option<Instruction>],
    inst: Option<&Instruction>,
    x: Value,
    ty: IrType,
) -> bool {
    match inst {
        Some(Instruction::Cmp {
            op,
            lhs: Operand::Value(v),
            rhs,
            ty: t,
            ..
        }) if same_signed_order(defs, *v, *t, x, ty) => match op {
            IrCmpOp::Slt => const_is(rhs, 0) || const_is(rhs, 1),
            IrCmpOp::Sle => const_is(rhs, -1) || const_is(rhs, 0),
            _ => false,
        },
        _ => false,
    }
}

/// The compare selecting the `x` arm: true for every positive `x`, false for
/// every negative one, either way at zero (`x >= 0`, `x > -1`, `x > 0`,
/// `x >= 1`) -- the mirror of [`proves_negative`].
fn proves_nonneg(
    defs: &[Option<Instruction>],
    inst: Option<&Instruction>,
    x: Value,
    ty: IrType,
) -> bool {
    match inst {
        Some(Instruction::Cmp {
            op,
            lhs: Operand::Value(v),
            rhs,
            ty: t,
            ..
        }) if same_signed_order(defs, *v, *t, x, ty) => match op {
            IrCmpOp::Sge => const_is(rhs, 0) || const_is(rhs, 1),
            IrCmpOp::Sgt => const_is(rhs, -1) || const_is(rhs, 0),
            _ => false,
        },
        _ => false,
    }
}

fn def_of(defs: &[Option<Instruction>], operand: Operand) -> Option<&Instruction> {
    let Operand::Value(v) = operand else {
        return None;
    };
    defs.get(v.0 as usize).and_then(Option::as_ref)
}

fn def_id(operand: Operand) -> Option<Value> {
    match operand {
        Operand::Value(v) => Some(v),
        Operand::Const(_) => None,
    }
}

/// The two-instruction prologue shared by both identities.
fn magnitude_body(
    x: Operand,
    ty: IrType,
    final_dest: Value,
    first_fresh: Value,
) -> Option<Vec<Instruction>> {
    let mask = Value(first_fresh.0.checked_add(1)?);
    Some(vec![
        Instruction::BinOp {
            dest: first_fresh,
            op: IrBinOp::AShr,
            lhs: x,
            rhs: Operand::Const(sign_mask_amount(ty)),
            ty,
        },
        Instruction::BinOp {
            dest: mask,
            op: IrBinOp::Xor,
            lhs: x,
            rhs: Operand::Value(first_fresh),
            ty,
        },
        Instruction::BinOp {
            dest: final_dest,
            op: IrBinOp::Sub,
            lhs: Operand::Value(mask),
            rhs: Operand::Value(first_fresh),
            ty,
        },
    ])
}

/// Recognize `|x|` and `|a - b|` in one select.
fn replacement(
    inst: &Instruction,
    defs: &[Option<Instruction>],
    fresh: Value,
) -> Option<Vec<Instruction>> {
    let Instruction::Select {
        dest,
        cond,
        true_val,
        false_val,
        ty,
    } = inst
    else {
        return None;
    };
    if !matches!(ty, IrType::I32 | IrType::I64) {
        return None;
    }
    let cond_inst = def_of(defs, *cond);

    // Absolute value: one arm is `Neg(x)`, the other is `x` itself, and the
    // condition says which side is negative.
    for (arm, other, negative_when_true) in [
        (*true_val, *false_val, true),
        (*false_val, *true_val, false),
    ] {
        let (Some(source), Some(other_value)) = (def_id(other), def_id(other)) else {
            continue;
        };
        if !is_neg_of(def_of(defs, arm), source) {
            continue;
        }
        // The other arm must be the bare source value (after nothing else:
        // the frontend emits the parameter, and a `Copy` is peeled by the
        // select's own operand here).
        if other_value != source {
            continue;
        }
        let proven = if negative_when_true {
            proves_negative(defs, cond_inst, source, *ty)
        } else {
            proves_nonneg(defs, cond_inst, source, *ty)
        };
        if !proven {
            continue;
        }
        return magnitude_body(Operand::Value(source), *ty, *dest, fresh);
    }

    // Absolute difference: `a > b ? (a - b) : (b - a)`, in either arm order
    // and with either comparison direction.
    let arms = [(true_val, false_val, true), (false_val, true_val, false)];
    for (diff_arm, other_arm, greater_when_true) in arms {
        let (diff_lhs, diff_rhs) = match def_of(defs, *diff_arm) {
            Some(Instruction::BinOp {
                op: IrBinOp::Sub,
                lhs: Operand::Value(a),
                rhs: Operand::Value(b),
                ty: t,
                ..
            }) if *t == *ty => (*a, *b),
            _ => continue,
        };
        if !is_sub_of(def_of(defs, *other_arm), diff_rhs, diff_lhs, *ty) {
            continue;
        }
        let proven = match cond_inst {
            Some(Instruction::Cmp {
                op,
                lhs: Operand::Value(l),
                rhs: Operand::Value(r),
                ty: t,
                ..
            }) if same_signed_order(defs, *l, *t, diff_lhs, *ty)
                && same_signed_order(defs, *r, *t, diff_rhs, *ty) =>
            {
                match (op, greater_when_true) {
                    (IrCmpOp::Sgt, true) | (IrCmpOp::Sle, false) => true,
                    _ => false,
                }
            }
            _ => false,
        };
        if !proven {
            continue;
        }
        // `d = a - b` on the same type; the sign of `d` decides the arm.
        let d = Value(fresh.0.checked_add(2)?);
        let mut body = vec![Instruction::BinOp {
            dest: d,
            op: IrBinOp::Sub,
            lhs: Operand::Value(diff_lhs),
            rhs: Operand::Value(diff_rhs),
            ty: *ty,
        }];
        body.extend(magnitude_body(Operand::Value(d), *ty, *dest, fresh)?);
        return Some(body);
    }
    None
}

struct Expansion {
    block: usize,
    index: usize,
    dest: Value,
    replacement: Vec<Instruction>,
}

pub(crate) fn run(func: &mut IrFunction) -> usize {
    let mut defs: Vec<Option<Instruction>> = vec![None; func.max_value_id() as usize + 1];
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                if let Some(slot) = defs.get_mut(dest.0 as usize) {
                    *slot = Some(inst.clone());
                }
            }
        }
    }
    let mut fresh = func.max_value_id().saturating_add(1);
    let mut expansions = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            let Some(dest) = inst.dest() else {
                continue;
            };
            let Some(body) = replacement(inst, &defs, Value(fresh)) else {
                continue;
            };
            let Some(next) = fresh.checked_add(body.len() as u32) else {
                continue;
            };
            fresh = next;
            expansions.push(Expansion {
                block: bi,
                index: ii,
                dest,
                replacement: body,
            });
        }
    }
    // Splices insert instructions, so apply from the highest coordinate down.
    expansions.sort_by(|a, b| (b.block, b.index).cmp(&(a.block, a.index)));
    let changed = expansions.len();
    for Expansion {
        block: bi,
        index: ii,
        dest,
        replacement,
    } in expansions
    {
        let block = &mut func.blocks[bi];
        if block.instructions.get(ii).and_then(Instruction::dest) != Some(dest) {
            continue;
        }
        if !block.source_spans.is_empty() {
            let span = block
                .source_spans
                .get(ii)
                .copied()
                .unwrap_or_else(crate::common::source::Span::dummy);
            block.source_spans.resize(block.instructions.len(), span);
            block
                .source_spans
                .splice(ii..=ii, std::iter::repeat_n(span, replacement.len()));
        }
        block.instructions.splice(ii..=ii, replacement);
    }
    if fresh > func.next_value_id {
        func.next_value_id = fresh;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};

    fn abs_function(cond_op: IrCmpOp, cond_const: i64, neg_arm_true: bool) -> IrFunction {
        let mut func = IrFunction::new("abs_test".into(), IrType::I32, vec![], false);
        let v = Operand::Value;
        let (t, f) = if neg_arm_true {
            (v(Value(2)), v(Value(0)))
        } else {
            (v(Value(0)), v(Value(2)))
        };
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::I32,
                },
                Instruction::Cmp {
                    dest: Value(1),
                    op: cond_op,
                    lhs: v(Value(0)),
                    rhs: Operand::Const(IrConst::I32(cond_const as i32)),
                    ty: IrType::I32,
                },
                Instruction::UnaryOp {
                    dest: Value(2),
                    op: IrUnaryOp::Neg,
                    src: v(Value(0)),
                    ty: IrType::I32,
                },
                Instruction::Select {
                    dest: Value(3),
                    cond: v(Value(1)),
                    true_val: t,
                    false_val: f,
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Return(Some(v(Value(3)))),
            source_spans: vec![],
            source_inline_ids: Vec::new(),
        });
        func.next_value_id = 4;
        func
    }

    #[test]
    fn negative_test_abs_becomes_mask_xor_subtract() {
        for (op, c, neg_true) in [
            (IrCmpOp::Slt, 0, true),
            (IrCmpOp::Sle, -1, true),
            (IrCmpOp::Sge, 0, false),
            (IrCmpOp::Sgt, -1, false),
            // Zero-inclusive spellings: at `x == 0` both arms are 0.
            (IrCmpOp::Sle, 0, true),
            (IrCmpOp::Slt, 1, true),
            (IrCmpOp::Sgt, 0, false),
            (IrCmpOp::Sge, 1, false),
        ] {
            let mut func = abs_function(op, c, neg_true);
            assert_eq!(run(&mut func), 1, "{op:?} {c}");
            let body: Vec<_> = func.blocks[0]
                .instructions
                .iter()
                .filter(|i| matches!(i, Instruction::BinOp { .. }))
                .collect();
            assert_eq!(body.len(), 3, "{op:?}");
            assert!(matches!(
                body[0],
                Instruction::BinOp {
                    op: IrBinOp::AShr,
                    rhs: Operand::Const(IrConst::I32(31)),
                    ..
                }
            ));
            assert!(matches!(
                body[1],
                Instruction::BinOp {
                    op: IrBinOp::Xor,
                    ..
                }
            ));
            assert!(matches!(
                body[2],
                Instruction::BinOp {
                    op: IrBinOp::Sub,
                    dest: Value(3),
                    ..
                }
            ));
        }
    }

    #[test]
    fn a_threshold_off_zero_or_a_swapped_arm_is_not_an_absolute_value() {
        // One step past each accepted constant the arms disagree on a nonzero
        // input (`x < 2 ? -x : x` is -1 at 1), and the mirrored arm order
        // computes -|x|; none of them may fold.
        for (op, c, neg_true) in [
            (IrCmpOp::Slt, 2, true),
            (IrCmpOp::Slt, -1, true),
            (IrCmpOp::Sle, 1, true),
            (IrCmpOp::Sle, -2, true),
            (IrCmpOp::Sgt, 1, false),
            (IrCmpOp::Sgt, -2, false),
            (IrCmpOp::Sge, 2, false),
            (IrCmpOp::Sge, -1, false),
            (IrCmpOp::Slt, 0, false),
            (IrCmpOp::Sle, 0, false),
            (IrCmpOp::Sgt, 0, true),
            (IrCmpOp::Sge, 0, true),
            (IrCmpOp::Ult, 1, true),
            (IrCmpOp::Eq, 0, true),
        ] {
            let mut func = abs_function(op, c, neg_true);
            assert_eq!(run(&mut func), 0, "{op:?} {c} neg_true={neg_true}");
        }
    }

    #[test]
    fn every_accepted_spelling_is_the_magnitude_on_every_boundary_input() {
        // The rule behind the accepted list, checked by evaluation: the
        // select and the identity agree on the extrema, around zero and at
        // the accepted thresholds themselves.
        let accepted = [
            (IrCmpOp::Slt, 0, true),
            (IrCmpOp::Sle, -1, true),
            (IrCmpOp::Sle, 0, true),
            (IrCmpOp::Slt, 1, true),
            (IrCmpOp::Sge, 0, false),
            (IrCmpOp::Sgt, -1, false),
            (IrCmpOp::Sgt, 0, false),
            (IrCmpOp::Sge, 1, false),
        ];
        let identity = |x: i32| (x ^ (x >> 31)).wrapping_sub(x >> 31);
        for (op, c, neg_true) in accepted {
            for x in [i32::MIN, i32::MIN + 1, -2, -1, 0, 1, 2, i32::MAX] {
                let cond = match op {
                    IrCmpOp::Slt => x < c,
                    IrCmpOp::Sle => x <= c,
                    IrCmpOp::Sgt => x > c,
                    IrCmpOp::Sge => x >= c,
                    _ => unreachable!(),
                };
                let (t, f) = if neg_true {
                    (x.wrapping_neg(), x)
                } else {
                    (x, x.wrapping_neg())
                };
                let select = if cond { t } else { f };
                assert_eq!(select, identity(x), "{op:?} {c} at {x}");
            }
        }
    }

    #[test]
    fn unrelated_select_arms_are_not_an_absolute_value() {
        let mut func = abs_function(IrCmpOp::Slt, 0, true);
        // Negate a *different* value: the arms no longer agree on `x`.
        func.blocks[0].instructions[2] = Instruction::UnaryOp {
            dest: Value(2),
            op: IrUnaryOp::Neg,
            src: Operand::Value(Value(7)),
            ty: IrType::I32,
        };
        assert_eq!(run(&mut func), 0);
    }

    #[test]
    fn narrow_and_wide_types_are_gated() {
        let mut narrow = abs_function(IrCmpOp::Slt, 0, true);
        for inst in &mut narrow.blocks[0].instructions {
            match inst {
                Instruction::Cmp { ty, .. } => *ty = IrType::I16,
                Instruction::UnaryOp { ty, .. } => *ty = IrType::I16,
                Instruction::Select { ty, .. } => *ty = IrType::I16,
                Instruction::ParamRef { ty, .. } => *ty = IrType::I16,
                _ => {}
            }
        }
        narrow.return_type = IrType::I16;
        assert_eq!(run(&mut narrow), 0);

        let mut wide = abs_function(IrCmpOp::Slt, 0, true);
        for inst in &mut wide.blocks[0].instructions {
            match inst {
                Instruction::Cmp {
                    ty,
                    rhs: Operand::Const(c),
                    ..
                } => {
                    *ty = IrType::I64;
                    *c = IrConst::I64(0);
                }
                Instruction::Cmp { ty, .. } => *ty = IrType::I64,
                Instruction::UnaryOp { ty, .. } => *ty = IrType::I64,
                Instruction::Select { ty, .. } => *ty = IrType::I64,
                Instruction::ParamRef { ty, .. } => *ty = IrType::I64,
                _ => {}
            }
        }
        wide.return_type = IrType::I64;
        assert_eq!(run(&mut wide), 1);
        assert!(wide.blocks[0].instructions.iter().any(|i| matches!(
            i,
            Instruction::BinOp {
                op: IrBinOp::AShr,
                rhs: Operand::Const(IrConst::I64(63)),
                ..
            }
        )));
    }

    fn absdiff_function() -> IrFunction {
        let mut func = IrFunction::new("absdiff_test".into(), IrType::I32, vec![], false);
        let v = Operand::Value;
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::I32,
                },
                Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 1,
                    ty: IrType::I32,
                },
                Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Sgt,
                    lhs: v(Value(0)),
                    rhs: v(Value(1)),
                    ty: IrType::I32,
                },
                Instruction::BinOp {
                    dest: Value(3),
                    op: IrBinOp::Sub,
                    lhs: v(Value(0)),
                    rhs: v(Value(1)),
                    ty: IrType::I32,
                },
                Instruction::BinOp {
                    dest: Value(4),
                    op: IrBinOp::Sub,
                    lhs: v(Value(1)),
                    rhs: v(Value(0)),
                    ty: IrType::I32,
                },
                Instruction::Select {
                    dest: Value(5),
                    cond: v(Value(2)),
                    true_val: v(Value(3)),
                    false_val: v(Value(4)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Return(Some(v(Value(5)))),
            source_spans: vec![],
            source_inline_ids: Vec::new(),
        });
        func.next_value_id = 6;
        func
    }

    #[test]
    fn absolute_difference_uses_one_subtraction() {
        let mut func = absdiff_function();
        assert_eq!(run(&mut func), 1);
        let subs: Vec<_> = func.blocks[0]
            .instructions
            .iter()
            .filter(|i| {
                matches!(
                    i,
                    Instruction::BinOp {
                        op: IrBinOp::Sub,
                        ..
                    }
                )
            })
            .collect();
        // The original two subtractions remain (nothing is deleted); the
        // rewrite adds its own difference and the identity's final subtract.
        assert_eq!(subs.len(), 4);
        assert!(matches!(
            func.blocks[0].instructions.last(),
            Some(Instruction::BinOp {
                op: IrBinOp::Sub,
                dest: Value(5),
                ..
            })
        ));
    }

    #[test]
    fn a_difference_select_with_reversed_compare_is_refused() {
        let mut func = absdiff_function();
        func.blocks[0].instructions[2] = Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Slt,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Value(Value(1)),
            ty: IrType::I32,
        };
        assert_eq!(run(&mut func), 0);
    }

    /// `x: narrow` compared at its own width, the arms on `sext(x): wide`:
    /// the IR the C frontend emits for `short x; return x < 0 ? -x : x;`.
    fn narrow_abs_function(
        narrow: IrType,
        cast_from: IrType,
        wide: IrType,
        cond_op: IrCmpOp,
        cond_const: i64,
        neg_arm_true: bool,
    ) -> IrFunction {
        let mut func = IrFunction::new("narrow_abs_test".into(), wide, vec![], false);
        let v = Operand::Value;
        let (t, f) = if neg_arm_true {
            (v(Value(3)), v(Value(2)))
        } else {
            (v(Value(2)), v(Value(3)))
        };
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: narrow,
                },
                Instruction::Cmp {
                    dest: Value(1),
                    op: cond_op,
                    lhs: v(Value(0)),
                    rhs: Operand::Const(IrConst::from_i64(cond_const, narrow)),
                    ty: narrow,
                },
                Instruction::Cast {
                    dest: Value(2),
                    src: v(Value(0)),
                    from_ty: cast_from,
                    to_ty: wide,
                },
                Instruction::UnaryOp {
                    dest: Value(3),
                    op: IrUnaryOp::Neg,
                    src: v(Value(2)),
                    ty: wide,
                },
                Instruction::Select {
                    dest: Value(4),
                    cond: v(Value(1)),
                    true_val: t,
                    false_val: f,
                    ty: wide,
                },
            ],
            terminator: Terminator::Return(Some(v(Value(4)))),
            source_spans: vec![],
            source_inline_ids: Vec::new(),
        });
        func.next_value_id = 5;
        func
    }

    const ACCEPTED_SPELLINGS: [(IrCmpOp, i64, bool); 8] = [
        (IrCmpOp::Slt, 0, true),
        (IrCmpOp::Sle, -1, true),
        (IrCmpOp::Sle, 0, true),
        (IrCmpOp::Slt, 1, true),
        (IrCmpOp::Sge, 0, false),
        (IrCmpOp::Sgt, -1, false),
        (IrCmpOp::Sgt, 0, false),
        (IrCmpOp::Sge, 1, false),
    ];

    #[test]
    fn a_narrow_compare_of_the_extended_operand_is_an_absolute_value() {
        for (narrow, wide) in [
            (IrType::I8, IrType::I32),
            (IrType::I16, IrType::I32),
            (IrType::I8, IrType::I64),
            (IrType::I16, IrType::I64),
            (IrType::I32, IrType::I64),
        ] {
            for (op, c, neg_true) in ACCEPTED_SPELLINGS {
                let mut func = narrow_abs_function(narrow, narrow, wide, op, c, neg_true);
                assert_eq!(run(&mut func), 1, "{narrow:?}->{wide:?} {op:?} {c}");
                let insts = &func.blocks[0].instructions;
                let width = i64::from(wide.size() as u32 * 8 - 1);
                // The identity runs at the promoted width on `sext(x)`.
                assert!(
                    insts.iter().any(|i| matches!(
                        i,
                        Instruction::BinOp {
                            op: IrBinOp::AShr,
                            lhs: Operand::Value(Value(2)),
                            rhs: Operand::Const(k),
                            ty,
                            ..
                        } if *ty == wide && k.to_i64() == Some(width)
                    )),
                    "{narrow:?}->{wide:?} {op:?} {c}: {insts:?}"
                );
                assert!(matches!(
                    insts.last(),
                    Some(Instruction::BinOp {
                        op: IrBinOp::Sub,
                        dest: Value(4),
                        ty,
                        ..
                    }) if *ty == wide
                ));
            }
        }
    }

    #[test]
    fn a_zero_extension_or_a_truncation_is_not_an_order_preserving_witness() {
        // `zext(u16)` reorders the negative half of the 16-bit pattern: a
        // signed compare of the narrow bits says nothing about the sign of
        // the widened value.
        let mut zext =
            narrow_abs_function(IrType::U16, IrType::U16, IrType::I32, IrCmpOp::Slt, 0, true);
        assert_eq!(run(&mut zext), 0);
        // A compare typed I16 on a value the cast reads as U16: the types
        // disagree, so the witness is refused.
        let mut mismatched =
            narrow_abs_function(IrType::I16, IrType::U16, IrType::I32, IrCmpOp::Slt, 0, true);
        assert_eq!(run(&mut mismatched), 0);
        // A truncation `i64 -> i32` discards the sign bit the compare saw.
        let mut trunc =
            narrow_abs_function(IrType::I64, IrType::I64, IrType::I32, IrCmpOp::Slt, 0, true);
        assert_eq!(run(&mut trunc), 0);
        // A threshold the identity does not express stays a select.
        let mut off =
            narrow_abs_function(IrType::I16, IrType::I16, IrType::I32, IrCmpOp::Slt, 2, true);
        assert_eq!(run(&mut off), 0);
    }

    #[test]
    fn every_narrow_input_agrees_with_the_promoted_identity() {
        // Exhaustive over every 8-bit and 16-bit value, every accepted
        // spelling: the select the frontend wrote (narrow compare, promoted
        // arms) equals the identity computed on the sign-extended value.
        let identity = |x: i32| (x ^ (x >> 31)).wrapping_sub(x >> 31);
        let narrow_inputs =
            (i32::from(i16::MIN)..=i32::from(i16::MAX)).map(|n| (n, n as i16 as i32));
        for (op, c, neg_true) in ACCEPTED_SPELLINGS {
            for (n, wide) in narrow_inputs.clone() {
                let cond = match op {
                    IrCmpOp::Slt => n < c as i32,
                    IrCmpOp::Sle => n <= c as i32,
                    IrCmpOp::Sgt => n > c as i32,
                    IrCmpOp::Sge => n >= c as i32,
                    _ => unreachable!(),
                };
                let (t, f) = if neg_true {
                    (wide.wrapping_neg(), wide)
                } else {
                    (wide, wide.wrapping_neg())
                };
                assert_eq!(
                    if cond { t } else { f },
                    identity(wide),
                    "{op:?} {c} at {n}"
                );
            }
        }
    }

    fn narrow_absdiff_function(lhs_from: IrType, rhs_from: IrType) -> IrFunction {
        let mut func = IrFunction::new("narrow_absdiff_test".into(), IrType::I32, vec![], false);
        let v = Operand::Value;
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::I16,
                },
                Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 1,
                    ty: IrType::I16,
                },
                Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Sgt,
                    lhs: v(Value(0)),
                    rhs: v(Value(1)),
                    ty: IrType::I16,
                },
                Instruction::Cast {
                    dest: Value(3),
                    src: v(Value(0)),
                    from_ty: lhs_from,
                    to_ty: IrType::I32,
                },
                Instruction::Cast {
                    dest: Value(4),
                    src: v(Value(1)),
                    from_ty: rhs_from,
                    to_ty: IrType::I32,
                },
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Sub,
                    lhs: v(Value(3)),
                    rhs: v(Value(4)),
                    ty: IrType::I32,
                },
                Instruction::BinOp {
                    dest: Value(6),
                    op: IrBinOp::Sub,
                    lhs: v(Value(4)),
                    rhs: v(Value(3)),
                    ty: IrType::I32,
                },
                Instruction::Select {
                    dest: Value(7),
                    cond: v(Value(2)),
                    true_val: v(Value(5)),
                    false_val: v(Value(6)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Return(Some(v(Value(7)))),
            source_spans: vec![],
            source_inline_ids: Vec::new(),
        });
        func.next_value_id = 8;
        func
    }

    #[test]
    fn a_narrow_difference_compare_is_an_absolute_difference_only_when_both_sides_extend() {
        let mut both = narrow_absdiff_function(IrType::I16, IrType::I16);
        assert_eq!(run(&mut both), 1);
        assert!(matches!(
            both.blocks[0].instructions.last(),
            Some(Instruction::BinOp {
                op: IrBinOp::Sub,
                dest: Value(7),
                ty: IrType::I32,
                ..
            })
        ));
        // One side zero-extended: `a > b` at 16 bits no longer orders the
        // promoted pair.
        let mut mixed = narrow_absdiff_function(IrType::I16, IrType::U16);
        assert_eq!(run(&mut mixed), 0);
        // Exhaustive agreement on a 16-bit grid that covers both extrema
        // and every sign combination: the promoted difference of two
        // sign-extended shorts never wraps, so the identity is `|a - b|`.
        let identity = |x: i32| (x ^ (x >> 31)).wrapping_sub(x >> 31);
        let grid = [
            i16::MIN,
            i16::MIN + 1,
            -300,
            -2,
            -1,
            0,
            1,
            2,
            300,
            i16::MAX - 1,
            i16::MAX,
        ];
        for a in grid {
            for b in grid {
                let (wa, wb) = (i32::from(a), i32::from(b));
                let select = if a > b { wa - wb } else { wb - wa };
                assert_eq!(select, identity(wa.wrapping_sub(wb)), "{a} {b}");
            }
        }
    }
}
