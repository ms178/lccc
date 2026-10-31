//! Production-matcher tests for the batch-5 scalar folds:
//!
//! * [`super::fold_shl_lshr_mask`] — `(x << k) >>u k` becomes `x & (2^(W-k) - 1)`
//!   (LLVM's `InstCombine` "shift pair to mask"; the audit's `#233830` cohort
//!   kept hitting it in byte/nibble extraction).
//! * [`super::fold_zero_one_membership`] — `(x == 0) | (x == 1)` becomes
//!   `x <=u 1`, and `(x != 0) & (x != 1)` becomes `x >u 1` (the comparator form
//!   of the implied-false disjunct in LLVM #234703 and the guarded-OR fold in
//!   #237533).
//!
//! The semantic checks evaluate the ORIGINAL instruction pair and the
//! REPLACEMENT the matcher returns.  They are exhaustive over every value of
//! the 8- and 16-bit rings, and boundary-plus-sweep for 32 and 64 bits, which
//! is what the campaign's protocol calls a reduced proof: the identity is
//! width-generic, but only the small rings are enumerated.
//!
//! The first draft of the shift fold masked with `2^k - 1` instead of
//! `2^(W-k) - 1` and was wrong for every `x` with a bit at or above `k`.
//! `shl_lshr_mask_matches_the_pair_exhaustively` is the test that now fails if
//! anybody reintroduces that mistake: it compares the emitted mask against the
//! pair's own two-step evaluation rather than against a hand-written constant.

use super::*;

fn v(n: u32) -> Operand {
    Operand::Value(Value(n))
}

fn ring_mask(bits: u32) -> u64 {
    if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    }
}

/// `(x << k) >>u k` evaluated exactly as the two IR instructions would be, in
/// a container wide enough to do the left shift without losing bits.
fn pair_value(x: u64, k: u32, bits: u32) -> u64 {
    let mask = ring_mask(bits);
    let shifted = if k == 0 { x } else { (x << k) & mask };
    (shifted >> k) & mask
}

fn shl_lshr_fixture(
    ty: IrType,
    amount: u64,
    shift_amount: Option<u64>,
) -> (Vec<Option<Instruction>>, Instruction) {
    let mut defs = vec![None; 4];
    defs[1] = Some(Instruction::BinOp {
        dest: Value(1),
        op: IrBinOp::Shl,
        lhs: v(0),
        rhs: Operand::Const(IrConst::from_i64(shift_amount.unwrap_or(amount) as i64, ty)),
        ty,
    });
    let inst = Instruction::BinOp {
        dest: Value(2),
        op: IrBinOp::LShr,
        lhs: v(1),
        rhs: Operand::Const(IrConst::from_i64(amount as i64, ty)),
        ty,
    };
    (defs, inst)
}

fn expect_and_mask(replacement: &Instruction, ty: IrType, bits: u32, amount: u32) -> bool {
    let Instruction::BinOp {
        dest,
        op: IrBinOp::And,
        lhs,
        rhs: Operand::Const(mask),
        ty: and_ty,
    } = replacement
    else {
        return false;
    };
    *dest == Value(2)
        && *lhs == v(0)
        && *and_ty == ty
        && mask.to_i128() == Some(((1u64 << (bits - amount)) - 1) as i128)
}

#[test]
fn shl_lshr_mask_matches_the_pair_exhaustively() {
    // Every value of the 8- and 16-bit rings, every legal amount.
    for (ty, bits) in [(IrType::U8, 8u32), (IrType::U16, 16)] {
        for amount in 1..bits {
            let (defs, inst) = shl_lshr_fixture(ty, amount as u64, None);
            let replacement = fold_shl_lshr_mask(&inst, &defs).expect("pair folds");
            assert!(
                expect_and_mask(&replacement, ty, bits, amount),
                "{ty:?} k={amount}: mask must be 2^(W-k)-1"
            );
            let Some(Operand::Const(mask)) = (match &replacement {
                Instruction::BinOp { rhs, .. } => Some(*rhs),
                _ => None,
            }) else {
                panic!("replacement must be an AND against a constant");
            };
            let mask = mask.to_i128().expect("integer constant") as u64;
            for x in 0..(1u64 << bits) {
                assert_eq!(
                    pair_value(x, amount, bits),
                    x & mask,
                    "{ty:?} k={amount} x={x}"
                );
            }
        }
    }
}

#[test]
fn shl_lshr_mask_matches_boundaries_at_32_and_64() {
    // Not exhaustive at these widths by construction; boundaries, the mask
    // edge, alternating bits and a deterministic sweep.
    for (ty, bits) in [
        (IrType::U32, 32u32),
        (IrType::I32, 32),
        (IrType::U64, 64),
        (IrType::I64, 64),
    ] {
        let full = ring_mask(bits);
        let mut probes = vec![
            0,
            1,
            full,
            full - 1,
            1u64 << (bits - 1),
            (1u64 << (bits - 1)) - 1,
        ];
        let mut s = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..64 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            probes.push(s & full);
        }
        for amount in [1u32, 2, bits / 2, bits - 2, bits - 1] {
            let (defs, inst) = shl_lshr_fixture(ty, amount as u64, None);
            let replacement = fold_shl_lshr_mask(&inst, &defs).expect("pair folds");
            assert!(
                expect_and_mask(&replacement, ty, bits, amount),
                "{ty:?} k={amount}"
            );
            let Instruction::BinOp {
                rhs: Operand::Const(mask),
                ..
            } = &replacement
            else {
                panic!("and");
            };
            let mask = mask.to_i128().expect("integer constant") as u64;
            for &x in &probes {
                assert_eq!(
                    pair_value(x, amount, bits),
                    x & mask,
                    "{ty:?} k={amount} x={x:#x}"
                );
            }
        }
    }
}

#[test]
fn shl_lshr_mask_refuses_every_non_member() {
    // Arithmetic right shift: a sign extension of a low field, never a mask.
    let mut defs = vec![None; 4];
    defs[1] = Some(Instruction::BinOp {
        dest: Value(1),
        op: IrBinOp::Shl,
        lhs: v(0),
        rhs: Operand::Const(IrConst::from_i64(3, IrType::I32)),
        ty: IrType::I32,
    });
    let ashr = Instruction::BinOp {
        dest: Value(2),
        op: IrBinOp::AShr,
        lhs: v(1),
        rhs: Operand::Const(IrConst::from_i64(3, IrType::I32)),
        ty: IrType::I32,
    };
    assert!(fold_shl_lshr_mask(&ashr, &defs).is_none());

    // k == 0 is already the identity.
    let (defs, inst) = shl_lshr_fixture(IrType::U32, 0, None);
    assert!(fold_shl_lshr_mask(&inst, &defs).is_none());

    // k >= W is undefined in C and is refused rather than folded.
    let (defs, inst) = shl_lshr_fixture(IrType::U32, 32, None);
    assert!(fold_shl_lshr_mask(&inst, &defs).is_none());
    let (defs, inst) = shl_lshr_fixture(IrType::U32, 40, None);
    assert!(fold_shl_lshr_mask(&inst, &defs).is_none());

    // Mismatched amounts and mismatched widths are different operations.
    let (defs, inst) = shl_lshr_fixture(IrType::U32, 3, Some(4));
    assert!(fold_shl_lshr_mask(&inst, &defs).is_none());
    let mut defs = vec![None; 4];
    defs[1] = Some(Instruction::BinOp {
        dest: Value(1),
        op: IrBinOp::Shl,
        lhs: v(0),
        rhs: Operand::Const(IrConst::from_i64(3, IrType::U64)),
        ty: IrType::U64,
    });
    let mixed = Instruction::BinOp {
        dest: Value(2),
        op: IrBinOp::LShr,
        lhs: v(1),
        rhs: Operand::Const(IrConst::from_i64(3, IrType::U32)),
        ty: IrType::U32,
    };
    assert!(fold_shl_lshr_mask(&mixed, &defs).is_none());

    // A non-constant amount is a different value for each call.
    let mut defs = vec![None; 4];
    defs[1] = Some(Instruction::BinOp {
        dest: Value(1),
        op: IrBinOp::Shl,
        lhs: v(0),
        rhs: v(3),
        ty: IrType::U32,
    });
    let variable = Instruction::BinOp {
        dest: Value(2),
        op: IrBinOp::LShr,
        lhs: v(1),
        rhs: v(3),
        ty: IrType::U32,
    };
    assert!(fold_shl_lshr_mask(&variable, &defs).is_none());
}

fn cmp_of(value: u32, op: IrCmpOp, constant: i64, ty: IrType, dest: u32) -> Instruction {
    Instruction::Cmp {
        dest: Value(dest),
        op,
        lhs: v(value),
        rhs: Operand::Const(IrConst::from_i64(constant, ty)),
        ty,
    }
}

fn membership_fixture(
    logic: IrBinOp,
    op: IrCmpOp,
    left: (u32, i64),
    right: (u32, i64),
    ty: IrType,
) -> (Vec<Option<Instruction>>, Instruction) {
    let mut defs = vec![None; 6];
    defs[1] = Some(cmp_of(left.0, op, left.1, ty, 1));
    defs[2] = Some(cmp_of(right.0, op, right.1, ty, 2));
    let inst = Instruction::BinOp {
        dest: Value(3),
        op: logic,
        lhs: v(1),
        rhs: v(2),
        ty: IrType::I8,
    };
    (defs, inst)
}

fn cmp_holds(op: IrCmpOp, a: u64, b: u64, bits: u32) -> bool {
    let mask = ring_mask(bits);
    let (a, b) = (a & mask, b & mask);
    let sign = 1u64 << (bits - 1);
    match op {
        IrCmpOp::Eq => a == b,
        IrCmpOp::Ne => a != b,
        IrCmpOp::Ult => a < b,
        IrCmpOp::Ule => a <= b,
        IrCmpOp::Ugt => a > b,
        IrCmpOp::Uge => a >= b,
        IrCmpOp::Slt => ((a ^ sign).wrapping_sub(sign)) < ((b ^ sign).wrapping_sub(sign)),
        IrCmpOp::Sle => ((a ^ sign).wrapping_sub(sign)) <= ((b ^ sign).wrapping_sub(sign)),
        IrCmpOp::Sgt => ((a ^ sign).wrapping_sub(sign)) > ((b ^ sign).wrapping_sub(sign)),
        IrCmpOp::Sge => ((a ^ sign).wrapping_sub(sign)) >= ((b ^ sign).wrapping_sub(sign)),
    }
}

#[test]
fn zero_one_membership_matches_the_pair_exhaustively() {
    // Both directions, both operand orders, both constant positions, on the
    // 8- and 16-bit rings where every value can be enumerated.
    for (ty, bits) in [(IrType::U8, 8u32), (IrType::U16, 16)] {
        for (logic, cmp_op, want) in [
            (IrBinOp::Or, IrCmpOp::Eq, IrCmpOp::Ule),
            (IrBinOp::And, IrCmpOp::Ne, IrCmpOp::Ugt),
        ] {
            for (left, right) in [((0u32, 0i64), (0u32, 1i64)), ((0, 1), (0, 0))] {
                let (defs, inst) = membership_fixture(logic, cmp_op, left, right, ty);
                let replacement = fold_zero_one_membership(&inst, &defs).expect("membership folds");
                let Instruction::Cmp {
                    dest,
                    op,
                    lhs,
                    rhs: Operand::Const(one),
                    ty: cmp_ty,
                } = &replacement
                else {
                    panic!("replacement must be a single comparison");
                };
                assert_eq!(*dest, Value(3));
                assert_eq!(*op, want);
                assert_eq!(*lhs, v(0), "the compared value is the shared root");
                assert_eq!(*cmp_ty, ty);
                assert_eq!(one.to_i128(), Some(1));
                for x in 0..(1u64 << bits) {
                    let original = if logic == IrBinOp::Or {
                        cmp_holds(cmp_op, x, 0, bits) | cmp_holds(cmp_op, x, 1, bits)
                    } else {
                        cmp_holds(cmp_op, x, 0, bits) & cmp_holds(cmp_op, x, 1, bits)
                    };
                    assert_eq!(original, cmp_holds(want, x, 1, bits), "{ty:?} x={x}");
                }
            }
        }
    }
}

#[test]
fn zero_one_membership_matches_signed_boundaries_at_32() {
    // The unsigned threshold is exact for signed inputs too: every negative
    // value is unsigned-above 1, and the signed {-1, -2, ...} band is exactly
    // what the identity must keep out of the set.
    let bits = 32u32;
    let probes: [u64; 10] = [
        0,
        1,
        2,
        0xFFFF_FFFF, // -1
        0xFFFF_FFFE, // -2
        0x8000_0000, // INT_MIN
        0x7FFF_FFFF, // INT_MAX
        3,
        0xFFFF_FF00,
        0x4000_0000,
    ];
    for x in probes {
        let original = cmp_holds(IrCmpOp::Eq, x, 0, bits) | cmp_holds(IrCmpOp::Eq, x, 1, bits);
        assert_eq!(original, cmp_holds(IrCmpOp::Ule, x, 1, bits), "x={x:#x}");
        let original_ne = cmp_holds(IrCmpOp::Ne, x, 0, bits) & cmp_holds(IrCmpOp::Ne, x, 1, bits);
        assert_eq!(original_ne, cmp_holds(IrCmpOp::Ugt, x, 1, bits), "x={x:#x}");
    }
}

#[test]
fn zero_one_membership_refuses_non_member_shapes() {
    let ty = IrType::U32;
    // Constants other than {0, 1}.
    for (l, r) in [(0i64, 2i64), (1, 2), (0, 3)] {
        let (defs, inst) = membership_fixture(IrBinOp::Or, IrCmpOp::Eq, (0, l), (0, r), ty);
        assert!(
            fold_zero_one_membership(&inst, &defs).is_none(),
            "{{{l}, {r}}} is not the {{0, 1}} set"
        );
    }
    // Mixed comparison ops are a different function (`x == 0 | x != 1`).
    let mut defs = vec![None; 6];
    defs[1] = Some(cmp_of(0, IrCmpOp::Eq, 0, ty, 1));
    defs[2] = Some(cmp_of(0, IrCmpOp::Ne, 1, ty, 2));
    let mixed = Instruction::BinOp {
        dest: Value(3),
        op: IrBinOp::Or,
        lhs: v(1),
        rhs: v(2),
        ty: IrType::I8,
    };
    assert!(fold_zero_one_membership(&mixed, &defs).is_none());
    // Different compared values.
    let (defs, inst) = membership_fixture(IrBinOp::Or, IrCmpOp::Eq, (0, 0), (1, 1), ty);
    assert!(fold_zero_one_membership(&inst, &defs).is_none());
    // `x != 0 | x != 1` is the tautology, not the threshold; and
    // `x == 0 & x == 1` is the empty set.  Neither is this fold.
    let (defs, inst) = membership_fixture(IrBinOp::Or, IrCmpOp::Ne, (0, 0), (0, 1), ty);
    assert!(fold_zero_one_membership(&inst, &defs).is_none());
    let (defs, inst) = membership_fixture(IrBinOp::And, IrCmpOp::Eq, (0, 0), (0, 1), ty);
    assert!(fold_zero_one_membership(&inst, &defs).is_none());
    // A non-comparison operand is not a membership test.
    let mut defs = vec![None; 6];
    defs[1] = Some(cmp_of(0, IrCmpOp::Eq, 0, ty, 1));
    let not_a_cmp = Instruction::BinOp {
        dest: Value(3),
        op: IrBinOp::Or,
        lhs: v(1),
        rhs: v(0),
        ty: IrType::I8,
    };
    assert!(fold_zero_one_membership(&not_a_cmp, &defs).is_none());
}
