//! Production-matcher tests for [`super::diff_branch::rewrite_diff_zero_branches`].
//!
//! `if (a ^ b)`, `if (a - b)` and `(a ^ b) != 0` are the accepted shapes; the
//! rewrite answers with `a != b` — appended as a fresh `Cmp Ne` for a direct
//! `CondBranch` on the difference, or edited into the existing `Cmp` in
//! place. The difference's own definition is never touched (its mismatch-arm
//! consumers keep their value; `expr_sink` decides where it lives).
//!
//! The semantic ground truth is checked exhaustively over the 8-bit ring:
//! `(a ^ b) != 0` and `(a - b) != 0` are `a != b` on every input pair, so
//! the branch rewrite can never change a taken edge.

use super::diff_branch::rewrite_diff_zero_branches;
use super::*;
use crate::ir::reexports::{BasicBlock, BlockId, Terminator};

fn mk(id: u32, insts: Vec<Instruction>, term: Terminator) -> BasicBlock {
    BasicBlock {
        label: BlockId(id),
        instructions: insts,
        terminator: term,
        source_spans: Vec::new(),
        source_inline_ids: Vec::new(),
    }
}

fn v(n: u32) -> Operand {
    Operand::Value(Value(n))
}

fn diff_binop(op: IrBinOp, dest: u32, ty: IrType) -> Instruction {
    Instruction::BinOp {
        dest: Value(dest),
        op,
        lhs: v(1),
        rhs: v(2),
        ty,
    }
}

/// `b0: d = a <op> b; br d, b1, b2` with returns in both arms — the
/// compare256 loop-exit shape reduced to its skeleton.
fn branch_on_difference(op: IrBinOp, ty: IrType) -> IrFunction {
    let mut f = IrFunction::new("diff".to_string(), IrType::U64, vec![], false);
    f.blocks = vec![
        mk(
            0,
            vec![diff_binop(op, 3, ty)],
            Terminator::CondBranch {
                cond: v(3),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
        ),
        mk(1, vec![], Terminator::Return(Some(v(3)))),
        mk(
            2,
            vec![],
            Terminator::Return(Some(Operand::Const(IrConst::I64(0)))),
        ),
    ];
    f.next_value_id = 4;
    f
}

#[test]
fn branch_on_xor_becomes_branch_on_disequality() {
    let mut f = branch_on_difference(IrBinOp::Xor, IrType::U64);
    assert_eq!(rewrite_diff_zero_branches(&mut f), 1);
    // A fresh `Cmp Ne(a, b)` is appended and branched on; the XOR stays —
    // the mismatch arm still returns it.
    assert!(matches!(
        f.blocks[0].instructions[..],
        [
            Instruction::BinOp {
                op: IrBinOp::Xor,
                ..
            },
            Instruction::Cmp {
                dest: Value(4),
                op: IrCmpOp::Ne,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Value(Value(2)),
                ty: IrType::U64,
            },
        ]
    ));
    assert!(matches!(
        f.blocks[0].terminator,
        Terminator::CondBranch {
            cond: Operand::Value(Value(4)),
            ..
        }
    ));
    // The fresh id is registered: a later pass minting from next_value_id
    // must not collide with the appended compare.
    assert_eq!(f.next_value_id, 5);
    // Idempotent: the new condition is a Cmp, not a difference.
    assert_eq!(rewrite_diff_zero_branches(&mut f), 0);
}

#[test]
fn branch_on_sub_becomes_branch_on_disequality() {
    let mut f = branch_on_difference(IrBinOp::Sub, IrType::I32);
    assert_eq!(rewrite_diff_zero_branches(&mut f), 1);
    assert!(matches!(
        f.blocks[0].instructions[1],
        Instruction::Cmp {
            op: IrCmpOp::Ne,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Value(Value(2)),
            ty: IrType::I32,
            ..
        }
    ));
}

#[test]
fn explicit_zero_compare_reads_the_operands() {
    // `(a ^ b) != 0` and `0 == (a ^ b)`: both sides of the zero accepted,
    // the predicate kept, the compare edited in place (same dest).
    for (lhs, rhs, op) in [
        (v(3), Operand::Const(IrConst::I64(0)), IrCmpOp::Ne),
        (Operand::Const(IrConst::I64(0)), v(3), IrCmpOp::Eq),
    ] {
        let mut f = IrFunction::new("cmp".to_string(), IrType::U32, vec![], false);
        f.blocks = vec![mk(
            0,
            vec![
                diff_binop(IrBinOp::Xor, 3, IrType::U64),
                Instruction::Cmp {
                    dest: Value(4),
                    op,
                    lhs,
                    rhs,
                    ty: IrType::U64,
                },
            ],
            Terminator::Return(Some(v(4))),
        )];
        f.next_value_id = 5;
        assert_eq!(rewrite_diff_zero_branches(&mut f), 1);
        assert!(matches!(
            f.blocks[0].instructions[1],
            Instruction::Cmp {
                dest: Value(4),
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Value(Value(2)),
                ty: IrType::U64,
                ..
            }
        ));
        if let Instruction::Cmp { op: kept, .. } = f.blocks[0].instructions[1] {
            assert_eq!(kept, op, "the predicate must survive the edit");
        }
        // No fresh value was needed for the in-place edit.
        assert_eq!(f.next_value_id, 5);
    }
}

#[test]
fn refused_shapes_stay_untouched() {
    // An ordered compare against zero is NOT a disequality of the operands.
    let ordered = |op| {
        let mut f = IrFunction::new("no".to_string(), IrType::U32, vec![], false);
        f.blocks = vec![mk(
            0,
            vec![
                diff_binop(IrBinOp::Sub, 3, IrType::I64),
                Instruction::Cmp {
                    dest: Value(4),
                    op,
                    lhs: v(3),
                    rhs: Operand::Const(IrConst::I64(0)),
                    ty: IrType::I64,
                },
            ],
            Terminator::Return(Some(v(4))),
        )];
        f.next_value_id = 5;
        f
    };
    for op in [IrCmpOp::Slt, IrCmpOp::Sgt, IrCmpOp::Ule] {
        let mut f = ordered(op);
        assert_eq!(rewrite_diff_zero_branches(&mut f), 0);
    }

    // A non-zero constant is a different question entirely.
    let mut f = ordered(IrCmpOp::Ne);
    if let Instruction::Cmp { rhs, .. } = &mut f.blocks[0].instructions[1] {
        *rhs = Operand::Const(IrConst::I64(5));
    }
    assert_eq!(rewrite_diff_zero_branches(&mut f), 0);

    // A compare narrower than the difference tests only the low bits:
    // `(u32)(a ^_64 b) != 0` is false for a, b differing above bit 31.
    let mut f = ordered(IrCmpOp::Ne);
    if let Instruction::Cmp { ty, .. } = &mut f.blocks[0].instructions[1] {
        *ty = IrType::U32;
    }
    assert_eq!(rewrite_diff_zero_branches(&mut f), 0);

    // A branch on a non-difference producer is not this pattern.
    let mut f = branch_on_difference(IrBinOp::And, IrType::U64);
    assert_eq!(rewrite_diff_zero_branches(&mut f), 0);

    // A degenerate two-same-targets branch is cfg_simplify's business.
    let mut f = branch_on_difference(IrBinOp::Xor, IrType::U64);
    f.blocks[0].terminator = Terminator::CondBranch {
        cond: v(3),
        true_label: BlockId(1),
        false_label: BlockId(1),
    };
    assert_eq!(rewrite_diff_zero_branches(&mut f), 0);

    // A 128-bit ring is refused: the branch would expand to a multi-word
    // compare, not the single fusible `cmp` the rewrite is priced on.
    let mut f = branch_on_difference(IrBinOp::Xor, IrType::U128);
    assert_eq!(rewrite_diff_zero_branches(&mut f), 0);
}

#[test]
fn the_wired_pass_rewrites_the_compare256_skeleton() {
    // Through `recognize_function` itself, so the dispatcher wiring (and its
    // placement BEFORE the def/use snapshot) is what is under test.
    let mut f = branch_on_difference(IrBinOp::Xor, IrType::U64);
    assert!(recognize_function(&mut f, false, 64, 8, false, true, true) >= 1);
    assert!(matches!(
        f.blocks[0].terminator,
        Terminator::CondBranch {
            cond: Operand::Value(Value(4)),
            ..
        }
    ));
}

#[test]
fn eight_bit_ring_ground_truth() {
    // The fold's entire claim: over every pair of the 8-bit ring, the
    // difference is zero exactly on the diagonal, for XOR and wrapping SUB.
    for a in 0u8..=255 {
        for b in 0u8..=255 {
            assert_eq!((a ^ b) != 0, a != b);
            assert_eq!(a.wrapping_sub(b) != 0, a != b);
        }
    }
}
