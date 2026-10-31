//! Difference-compare canonicalization: `(a - b) <op> 0` becomes `a <op> b`.
//!
//! A subtraction whose only purpose is to be compared against zero is a
//! wasted instruction: the comparison can consume the two subtrahends
//! directly, and the subtraction then dies with the last reader that the
//! comparison was.  GCC and Clang both perform this at the tree/IR level, and
//! the shape is common in reduced library code — `if (a - b > 0)`,
//! `if (x - y == 0)`, and pointer-strides written as differences.
//!
//! # Why the rule is typed
//!
//! For a two's-complement ring, `(a - b) == 0` and `a == b` are the same
//! predicate at every width and every signedness: the subtraction wraps, and
//! it wraps to zero exactly when the operands are equal.  Nothing else about
//! modular subtraction can be compared for free:
//!
//! * **Unsigned.**  `(a - b) >u 0` is `a != b` (a difference wraps to a
//!   nonzero value exactly when the operands differ) and `(a - b) <=u 0` is
//!   `a == b`.  But `(a - b) <u c` for `c != 0` says nothing about `a < b`,
//!   because the wrap turns "below" into "far above"; those forms are refused.
//! * **Signed.**  Order commutes with subtraction only when the subtraction
//!   does not overflow, and C makes signed overflow undefined.  The rule is
//!   therefore admitted only when the module's pass policy says the frontend
//!   already hardened signed overflow (`wrap_signed_overflow == false`, the
//!   default for C) so the comparison of the difference and the comparison of
//!   the operands agree on every defined execution.  A module compiled with
//!   wrapping signed arithmetic keeps the subtraction.
//!
//! Only the comparison is rewritten.  A shared subtraction keeps its value:
//! the pass replaces one `Cmp` node and never deletes an instruction, so every
//! other reader sees exactly what it saw before.

use crate::common::fx_hash::FxHashMap;
use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Value};

/// Mirror a comparison so `0 <op> x` can be read as `x <op'> 0`.
fn mirror(op: IrCmpOp) -> IrCmpOp {
    match op {
        IrCmpOp::Eq => IrCmpOp::Eq,
        IrCmpOp::Ne => IrCmpOp::Ne,
        IrCmpOp::Slt => IrCmpOp::Sgt,
        IrCmpOp::Sle => IrCmpOp::Sge,
        IrCmpOp::Sgt => IrCmpOp::Slt,
        IrCmpOp::Sge => IrCmpOp::Sle,
        IrCmpOp::Ult => IrCmpOp::Ugt,
        IrCmpOp::Ule => IrCmpOp::Uge,
        IrCmpOp::Ugt => IrCmpOp::Ult,
        IrCmpOp::Uge => IrCmpOp::Ule,
    }
}

fn is_zero(operand: &Operand) -> bool {
    matches!(operand, Operand::Const(c) if c.to_i64() == Some(0))
}

/// The operator that compares the subtrahends directly, if one exists.
fn direct_op(op: IrCmpOp, signed: bool, may_wrap: bool) -> Option<IrCmpOp> {
    match op {
        // Equality never depends on the wrap or on the order.
        IrCmpOp::Eq | IrCmpOp::Ne => Some(op),
        // Unsigned: a wrapped difference is nonzero exactly when a != b.
        IrCmpOp::Ugt if !signed => Some(IrCmpOp::Ne),
        IrCmpOp::Ule if !signed => Some(IrCmpOp::Eq),
        // Signed: order is preserved only without overflow.
        IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge if signed && !may_wrap => {
            Some(op)
        }
        _ => None,
    }
}

fn replacement(
    defs: &FxHashMap<u32, (usize, usize)>,
    uses: &FxHashMap<u32, usize>,
    func: &IrFunction,
    inst: &Instruction,
    may_wrap: bool,
) -> Option<Instruction> {
    let Instruction::Cmp {
        dest,
        op,
        lhs,
        rhs,
        ty,
    } = inst
    else {
        return None;
    };
    if !matches!(
        ty,
        IrType::I8
            | IrType::I16
            | IrType::I32
            | IrType::I64
            | IrType::U8
            | IrType::U16
            | IrType::U32
            | IrType::U64
    ) {
        return None;
    }
    // Exactly one side is the literal zero; the other is the difference.
    let (value, op) = if is_zero(rhs) {
        (*lhs, *op)
    } else if is_zero(lhs) {
        // `0 <op> (a - b)` is `(a - b) mirror(op) 0`.
        (*rhs, mirror(*op))
    } else {
        return None;
    };
    let Operand::Value(v) = value else {
        return None;
    };
    let &(bi, ii) = defs.get(&v.0)?;
    let Instruction::BinOp {
        op: IrBinOp::Sub,
        lhs: minuend,
        rhs: subtrahend,
        ty: sub_ty,
        ..
    } = &func.blocks[bi].instructions[ii]
    else {
        return None;
    };
    if *sub_ty != *ty {
        return None;
    }
    // Only when the subtraction dies.  Its flags are free in the original
    // shape (`subl` sets them, `setg` reads them), so folding a *shared*
    // subtraction adds a comparison where the backend had none: measured as
    // +13 instructions in div_mod_by_const_exhaustive.c and +5 in
    // sign_test_cmov.c before this guard.
    if uses.get(&v.0).copied().unwrap_or(0) != 1 {
        return None;
    }
    let signed = matches!(ty, IrType::I8 | IrType::I16 | IrType::I32 | IrType::I64);
    let op = direct_op(op, signed, may_wrap)?;
    // `(0 - b) <op> 0` would rewrite to `0 <op> b`, which is not a difference
    // comparison any more; rewriting it is still correct, but it is a
    // subtraction-by-constant fold and belongs to the constant folder.  Keep
    // this pass to two live subtrahends.
    if is_zero(minuend) {
        return None;
    }
    Some(Instruction::Cmp {
        dest: *dest,
        op,
        lhs: *minuend,
        rhs: *subtrahend,
        ty: *ty,
    })
}

pub(crate) fn run(func: &mut IrFunction) -> usize {
    let mut defs: FxHashMap<u32, (usize, usize)> = FxHashMap::default();
    let mut uses: FxHashMap<u32, usize> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Some(Value(v)) = inst.dest() {
                defs.insert(v, (bi, ii));
            }
            inst.for_each_used_value(|v| *uses.entry(v).or_insert(0) += 1);
        }
        block
            .terminator
            .for_each_used_value(|v| *uses.entry(v).or_insert(0) += 1);
    }
    let may_wrap = func.pass_policy.wrap_signed_overflow;
    let mut replacements = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Some(new_inst) = replacement(&defs, &uses, func, inst, may_wrap) {
                replacements.push((bi, ii, new_inst));
            }
        }
    }
    let changed = replacements.len();
    for (bi, ii, inst) in replacements {
        func.blocks[bi].instructions[ii] = inst;
    }
    changed
}

/// The constant a difference is compared against is the ring's zero.
#[cfg(test)]
fn zero(ty: IrType) -> Operand {
    Operand::Const(IrConst::from_i64(0, ty))
}

/// Apply to every function body of a module; returns the number of rewrites.
pub(crate) fn run_on_module(module: &mut crate::ir::module::IrModule) -> usize {
    let mut total = 0;
    for func in module.functions.iter_mut() {
        if !func.is_declaration {
            total += run(func);
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, IrBinOp, Terminator};

    fn function(ty: IrType, op: IrCmpOp, zero_on_left: bool, sub_ty: IrType) -> IrFunction {
        let mut func = IrFunction::new("diffcmp".into(), ty, vec![], false);
        let php = Operand::Value;
        let mut insts = vec![
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
            Instruction::BinOp {
                dest: Value(2),
                op: IrBinOp::Sub,
                lhs: php(Value(0)),
                rhs: php(Value(1)),
                ty: sub_ty,
            },
        ];
        let (lhs, rhs) = if zero_on_left {
            (zero(ty), php(Value(2)))
        } else {
            (php(Value(2)), zero(ty))
        };
        insts.push(Instruction::Cmp {
            dest: Value(3),
            op,
            lhs,
            rhs,
            ty,
        });
        func.next_value_id = 4;
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: insts,
            terminator: Terminator::Return(Some(php(Value(3)))),
            source_spans: vec![],
        });
        func
    }

    fn folded(func: &mut IrFunction) -> Option<(IrCmpOp, Operand, Operand)> {
        let before = func.blocks[0].instructions[3].clone();
        let n = run(func);
        if n == 0 {
            return None;
        }
        match (&before, &func.blocks[0].instructions[3]) {
            (
                Instruction::Cmp { dest: d0, .. },
                Instruction::Cmp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                },
            ) if d0 == dest => {
                assert_eq!(*ty, func.return_type, "the comparison keeps its type");
                Some((*op, *lhs, *rhs))
            }
            _ => panic!("the comparison node must stay a comparison"),
        }
    }

    #[test]
    fn signed_order_moves_to_the_operands() {
        for op in [
            IrCmpOp::Slt,
            IrCmpOp::Sle,
            IrCmpOp::Sgt,
            IrCmpOp::Sge,
            IrCmpOp::Eq,
            IrCmpOp::Ne,
        ] {
            let mut func = function(IrType::I32, op, false, IrType::I32);
            let (new_op, lhs, rhs) = folded(&mut func).expect("signed difference compare folds");
            assert_eq!(new_op, op);
            assert_eq!(
                (lhs, rhs),
                (Operand::Value(Value(0)), Operand::Value(Value(1)))
            );
        }
    }

    #[test]
    fn zero_on_the_left_mirrors_the_operator() {
        let mut func = function(IrType::I32, IrCmpOp::Slt, true, IrType::I32);
        // `0 < (a - b)` is `a > b`.
        let (op, lhs, rhs) = folded(&mut func).expect("mirrored form folds");
        assert_eq!(op, IrCmpOp::Sgt);
        assert_eq!(
            (lhs, rhs),
            (Operand::Value(Value(0)), Operand::Value(Value(1)))
        );
    }

    #[test]
    fn unsigned_order_is_refused_but_equality_and_nonzero_are_admitted() {
        assert!(folded(&mut function(IrType::U32, IrCmpOp::Ult, false, IrType::U32)).is_none());
        assert!(folded(&mut function(IrType::U32, IrCmpOp::Ule, false, IrType::U32)).is_some());
        assert!(folded(&mut function(IrType::U32, IrCmpOp::Ugt, false, IrType::U32)).is_some());
        let mut eq = function(IrType::U64, IrCmpOp::Ne, false, IrType::U64);
        let (op, _, _) = folded(&mut eq).expect("unsigned equality folds at 64 bits");
        assert_eq!(op, IrCmpOp::Ne);
    }

    #[test]
    fn wrapping_signed_modules_keep_the_subtraction() {
        let mut func = function(IrType::I32, IrCmpOp::Sgt, false, IrType::I32);
        func.pass_policy.wrap_signed_overflow = true;
        assert_eq!(run(&mut func), 0);
        // Equality is still exact under wrapping.
        let mut eq = function(IrType::I32, IrCmpOp::Eq, false, IrType::I32);
        eq.pass_policy.wrap_signed_overflow = true;
        assert_eq!(run(&mut eq), 1);
    }

    #[test]
    fn mismatched_subtraction_width_is_refused() {
        let mut func = function(IrType::I32, IrCmpOp::Sgt, false, IrType::U16);
        assert_eq!(run(&mut func), 0);
    }

    #[test]
    fn a_nonzero_comparison_constant_is_not_a_difference_compare() {
        let mut func = function(IrType::I32, IrCmpOp::Sgt, false, IrType::I32);
        func.blocks[0].instructions[3] = Instruction::Cmp {
            dest: Value(3),
            op: IrCmpOp::Sgt,
            lhs: Operand::Value(Value(2)),
            rhs: Operand::Const(IrConst::I32(7)),
            ty: IrType::I32,
        };
        assert_eq!(run(&mut func), 0);
    }

    #[test]
    fn shared_subtraction_is_left_alone() {
        // The subtraction's flags are free in the original shape, so folding a
        // shared difference would ADD a comparison.  Neither node may move.
        let mut func = function(IrType::I32, IrCmpOp::Sgt, false, IrType::I32);
        func.blocks[0].instructions.push(Instruction::Cmp {
            dest: Value(4),
            op: IrCmpOp::Slt,
            lhs: Operand::Value(Value(2)),
            rhs: zero(IrType::I32),
            ty: IrType::I32,
        });
        func.next_value_id = 5;
        assert_eq!(run(&mut func), 0);
    }
}
