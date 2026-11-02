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
//! the arms operate on.  A narrow type is refused: the frontend promotes
//! arithmetic to `int`, so a narrow select is a different program, and a
//! 8-bit `AShr` mask is not the abstraction the backend expects here.
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

/// The compare selecting the `-x` arm: true for every negative `x`, false
/// for every positive one.  At `x == 0` the two arms agree (`-0 == 0`), so
/// the compare may go either way there: `x < 0` and `x <= -1` exclude zero,
/// `x <= 0` and `x < 1` include it, and all four select `|x|`.
fn proves_negative(inst: Option<&Instruction>, x: Value, ty: IrType) -> bool {
    match inst {
        Some(Instruction::Cmp {
            op,
            lhs: Operand::Value(v),
            rhs,
            ty: t,
            ..
        }) if *v == x && *t == ty => match op {
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
fn proves_nonneg(inst: Option<&Instruction>, x: Value, ty: IrType) -> bool {
    match inst {
        Some(Instruction::Cmp {
            op,
            lhs: Operand::Value(v),
            rhs,
            ty: t,
            ..
        }) if *v == x && *t == ty => match op {
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
            proves_negative(cond_inst, source, *ty)
        } else {
            proves_nonneg(cond_inst, source, *ty)
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
            }) if *t == *ty && *l == diff_lhs && *r == diff_rhs => match (op, greater_when_true) {
                (IrCmpOp::Sgt, true) | (IrCmpOp::Sle, false) => true,
                _ => false,
            },
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
}
