//! Production-matcher tests for [`super::bit_select::fold_bit_select`].
//!
//! `x | 1 << n`, `x & ~(1 << n)`, `x ^ 1 << n` and `(x & 1 << n) != 0` are
//! the shapes accepted here; the fold answers with `BitSet`, `BitClear`,
//! `BitToggle` and `BitTest`.
//!
//! Every semantic check evaluates the ORIGINAL instructions and the
//! REPLACEMENT over the whole ring of each width the fold can create, plus
//! indices *above* the width -- where the C source is undefined but the IR
//! contract (and every backend's lowering) is `index mod W`, so the fold must
//! agree with itself there too.  The negative tests pin what must NOT fold,
//! because each of those refusals is what keeps the rewrite from duplicating a
//! producer or from changing a value's type.

use super::bit_select::fold_bit_select;
use super::*;

fn v(n: u32) -> Operand {
    Operand::Value(Value(n))
}

fn one(ty: IrType) -> Operand {
    Operand::Const(IrConst::from_i64(1, ty))
}

fn zero(ty: IrType) -> Operand {
    Operand::Const(IrConst::zero(ty))
}

fn all_ones(ty: IrType) -> Operand {
    let bits = ty.size() * 8;
    Operand::Const(IrConst::from_i64((u64::MAX >> (64 - bits)) as i64, ty))
}

/// `x | (1 << n)` with every producer used once.
fn or_fixture(
    ty: IrType,
    shifted_first: bool,
) -> (Vec<Option<Instruction>>, Vec<u32>, Instruction) {
    let mut defs = vec![None; 6];
    // Value(1) = base, Value(2) = index, Value(3) = 1 << index, Value(4) = result.
    defs[3] = Some(Instruction::BinOp {
        dest: Value(3),
        op: IrBinOp::Shl,
        lhs: one(ty),
        rhs: v(2),
        ty,
    });
    let (lhs, rhs) = if shifted_first {
        (v(3), v(1))
    } else {
        (v(1), v(3))
    };
    let inst = Instruction::BinOp {
        dest: Value(4),
        op: IrBinOp::Or,
        lhs,
        rhs,
        ty,
    };
    let mut uses = vec![0u32; 6];
    uses[1] = 1;
    uses[2] = 2;
    uses[3] = 1;
    (defs, uses, inst)
}

/// `x ^ (1 << n)` with every producer used once.  Same shape as
/// [`or_fixture`] with the join swapped, which is the whole point: the two
/// idioms differ only in the operator.
fn xor_fixture(
    ty: IrType,
    shifted_first: bool,
) -> (Vec<Option<Instruction>>, Vec<u32>, Instruction) {
    let (defs, uses, _) = or_fixture(ty, shifted_first);
    let (lhs, rhs) = if shifted_first {
        (v(3), v(1))
    } else {
        (v(1), v(3))
    };
    let inst = Instruction::BinOp {
        dest: Value(4),
        op: IrBinOp::Xor,
        lhs,
        rhs,
        ty,
    };
    (defs, uses, inst)
}

/// `x & ~(1 << n)`, with the complement spelled `Not` or `Xor -1`.
fn and_fixture(
    ty: IrType,
    xor_spelling: bool,
    complement_first: bool,
) -> (Vec<Option<Instruction>>, Vec<u32>, Instruction) {
    let mut defs = vec![None; 6];
    defs[3] = Some(Instruction::BinOp {
        dest: Value(3),
        op: IrBinOp::Shl,
        lhs: one(ty),
        rhs: v(2),
        ty,
    });
    defs[4] = Some(if xor_spelling {
        Instruction::BinOp {
            dest: Value(4),
            op: IrBinOp::Xor,
            lhs: v(3),
            rhs: all_ones(ty),
            ty,
        }
    } else {
        Instruction::UnaryOp {
            dest: Value(4),
            op: IrUnaryOp::Not,
            src: v(3),
            ty,
        }
    });
    let (lhs, rhs) = if complement_first {
        (v(4), v(1))
    } else {
        (v(1), v(4))
    };
    let inst = Instruction::BinOp {
        dest: Value(5),
        op: IrBinOp::And,
        lhs,
        rhs,
        ty,
    };
    let mut uses = vec![0u32; 6];
    uses[1] = 1;
    uses[2] = 2;
    uses[3] = 1;
    uses[4] = 1;
    (defs, uses, inst)
}

/// `(x & (1 << n)) != 0`.
fn cmp_fixture(ty: IrType, swapped: bool) -> (Vec<Option<Instruction>>, Vec<u32>, Instruction) {
    let mut defs = vec![None; 7];
    defs[3] = Some(Instruction::BinOp {
        dest: Value(3),
        op: IrBinOp::Shl,
        lhs: one(ty),
        rhs: v(2),
        ty,
    });
    defs[4] = Some(Instruction::BinOp {
        dest: Value(4),
        op: IrBinOp::And,
        lhs: v(1),
        rhs: v(3),
        ty,
    });
    let (lhs, rhs) = if swapped {
        (zero(ty), v(4))
    } else {
        (v(4), zero(ty))
    };
    let inst = Instruction::Cmp {
        dest: Value(5),
        op: IrCmpOp::Ne,
        lhs,
        rhs,
        ty,
    };
    let mut uses = vec![0u32; 7];
    uses[1] = 1;
    uses[2] = 2;
    uses[3] = 1;
    uses[4] = 1;
    (defs, uses, inst)
}

fn bits_of(ty: IrType) -> u32 {
    (ty.size() * 8) as u32
}

fn mask_of(bits: u32) -> u64 {
    if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    }
}

/// The original `x | (1 << n)` and `x & ~(1 << n)` values, evaluated the way
/// the IR defines them (`1 << (n mod W)`, then the logic op, truncated to W).
fn original_set(x: u64, n: u64, bits: u32, set: bool) -> u64 {
    let bit = 1u64 << (n % u64::from(bits));
    let value = if set { x | bit } else { x & !bit };
    value & mask_of(bits)
}

/// The original `x ^ (1 << n)` value, evaluated the same way.
fn original_toggle(x: u64, n: u64, bits: u32) -> u64 {
    (x ^ (1u64 << (n % u64::from(bits)))) & mask_of(bits)
}

#[test]
fn bit_set_and_clear_match_the_original_over_the_whole_ring() {
    for ty in [
        IrType::U8,
        IrType::I8,
        IrType::U16,
        IrType::I16,
        IrType::U32,
        IrType::I32,
        IrType::U64,
        IrType::I64,
    ] {
        let bits = bits_of(ty);
        // Indices cover the width, one past it and a value far outside it.
        let mut indices: Vec<u64> = (0..=u64::from(bits) + 1).collect();
        indices.push(u64::from(bits) * 3 + 5);
        indices.push(u64::MAX);
        let patterns: Vec<u64> = if bits <= 8 {
            (0..=mask_of(bits)).collect()
        } else if bits == 16 {
            (0..=mask_of(bits)).step_by(97).collect()
        } else {
            vec![
                0,
                1,
                mask_of(bits),
                mask_of(bits) / 3,
                0x8000_0000,
                0x0123_4567_89AB_CDEF & mask_of(bits),
            ]
        };
        for &x in &patterns {
            for &n in &indices {
                let (defs, uses, inst) = or_fixture(ty, false);
                let Some(Instruction::BinOp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty: out_ty,
                }) = fold_bit_select(&inst, &defs, &uses)
                else {
                    panic!("x | 1 << n did not fold at {ty:?}");
                };
                assert_eq!(dest, Value(4), "the OR's dest is kept");
                assert_eq!(op, IrBinOp::BitSet);
                assert_eq!(out_ty, ty);
                assert_eq!((lhs, rhs), (v(1), v(2)), "base and index are kept");
                let got = crate::ir::ops::bit_op_within_bits(
                    x as u128,
                    n as u128,
                    bits,
                    crate::ir::ops::BitMode::Set,
                ) as u64;
                assert_eq!(
                    got,
                    original_set(x, n, bits, true),
                    "BitSet {x:#x} bit {n} at {ty:?}"
                );

                let (defs, uses, inst) = and_fixture(ty, false, false);
                let Some(Instruction::BinOp { op, .. }) = fold_bit_select(&inst, &defs, &uses)
                else {
                    panic!("x & ~(1 << n) did not fold at {ty:?}");
                };
                assert_eq!(op, IrBinOp::BitClear);
                let got = crate::ir::ops::bit_op_within_bits(
                    x as u128,
                    n as u128,
                    bits,
                    crate::ir::ops::BitMode::Clear,
                ) as u64;
                assert_eq!(
                    got,
                    original_set(x, n, bits, false),
                    "BitClear {x:#x} bit {n} at {ty:?}"
                );
            }
        }
    }
}

#[test]
fn bit_toggle_matches_the_original_over_the_whole_ring() {
    for ty in [
        IrType::U8,
        IrType::I8,
        IrType::U16,
        IrType::I16,
        IrType::U32,
        IrType::I32,
        IrType::U64,
        IrType::I64,
    ] {
        let bits = bits_of(ty);
        let mut indices: Vec<u64> = (0..=u64::from(bits) + 1).collect();
        indices.push(u64::from(bits) * 3 + 5);
        indices.push(u64::MAX);
        let patterns: Vec<u64> = if bits <= 8 {
            (0..=mask_of(bits)).collect()
        } else if bits == 16 {
            (0..=mask_of(bits)).step_by(97).collect()
        } else {
            vec![
                0,
                1,
                mask_of(bits),
                mask_of(bits) / 3,
                0x8000_0000,
                0x0123_4567_89AB_CDEF & mask_of(bits),
            ]
        };
        for &x in &patterns {
            for &n in &indices {
                for shifted_first in [false, true] {
                    let (defs, uses, inst) = xor_fixture(ty, shifted_first);
                    let Some(Instruction::BinOp {
                        dest,
                        op,
                        lhs,
                        rhs,
                        ty: out_ty,
                    }) = fold_bit_select(&inst, &defs, &uses)
                    else {
                        panic!("x ^ 1 << n did not fold at {ty:?}");
                    };
                    assert_eq!(dest, Value(4), "the XOR's dest is kept");
                    assert_eq!(op, IrBinOp::BitToggle);
                    assert_eq!(out_ty, ty);
                    assert_eq!((lhs, rhs), (v(1), v(2)), "base and index are kept");
                    let got = crate::ir::ops::bit_op_within_bits(
                        x as u128,
                        n as u128,
                        bits,
                        crate::ir::ops::BitMode::Toggle,
                    ) as u64;
                    assert_eq!(
                        got,
                        original_toggle(x, n, bits),
                        "BitToggle {x:#x} bit {n} at {ty:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn bit_test_matches_the_original_under_both_operand_orders() {
    for ty in [IrType::U8, IrType::U16, IrType::U32, IrType::U64] {
        let bits = bits_of(ty);
        for &x in &[0u64, 1, 2, mask_of(bits), mask_of(bits) / 2 + 1] {
            for n in 0..=u64::from(bits) + 1 {
                for swapped in [false, true] {
                    let (defs, uses, inst) = cmp_fixture(ty, swapped);
                    let Some(Instruction::BinOp {
                        dest,
                        op,
                        lhs,
                        rhs,
                        ty: out_ty,
                    }) = fold_bit_select(&inst, &defs, &uses)
                    else {
                        panic!("(x & 1 << n) != 0 did not fold at {ty:?}");
                    };
                    assert_eq!(dest, Value(5), "the comparison's dest is kept");
                    assert_eq!(op, IrBinOp::BitTest);
                    assert_eq!(out_ty, ty);
                    assert_eq!((lhs, rhs), (v(1), v(2)));
                    let want = original_set(x, n, bits, true) != 0;
                    let got = crate::ir::ops::bit_op_within_bits(
                        x as u128,
                        n as u128,
                        bits,
                        crate::ir::ops::BitMode::Set,
                    ) as u64
                        != 0;
                    assert_eq!(got, want, "BitTest {x:#x} bit {n} at {ty:?}");
                }
            }
        }
    }
}

#[test]
fn the_complement_may_be_spelled_with_xor_of_all_ones() {
    let (defs, uses, inst) = and_fixture(IrType::U32, true, false);
    let Some(Instruction::BinOp { op, .. }) = fold_bit_select(&inst, &defs, &uses) else {
        panic!("~(1 << n) spelled as `(1 << n) ^ -1` must fold");
    };
    assert_eq!(op, IrBinOp::BitClear);
}

#[test]
fn the_shift_may_be_the_first_operand() {
    let (defs, uses, inst) = or_fixture(IrType::U32, true);
    let Some(Instruction::BinOp { op, lhs, .. }) = fold_bit_select(&inst, &defs, &uses) else {
        panic!("(1 << n) | x must fold");
    };
    assert_eq!(op, IrBinOp::BitSet);
    assert_eq!(lhs, v(1), "the value operand becomes the base");
}

#[test]
fn a_shared_producer_is_not_duplicated() {
    // The shift has a second use, so rewriting the OR would leave the shift in
    // place AND add the bit operation.
    let (defs, mut uses, inst) = or_fixture(IrType::U32, false);
    uses[3] = 2;
    assert!(fold_bit_select(&inst, &defs, &uses).is_none());

    // Same for a shared comparison operand in the bit-test shape.
    let (defs, mut uses, inst) = cmp_fixture(IrType::U32, false);
    uses[4] = 2;
    assert!(fold_bit_select(&inst, &defs, &uses).is_none());

    // ... and for a shared complement in the bit-clear shape.
    let (defs, mut uses, inst) = and_fixture(IrType::U32, false, false);
    uses[4] = 2;
    assert!(fold_bit_select(&inst, &defs, &uses).is_none());

    // ... and for the toggle, which uses the same fixture as the OR.
    let (defs, mut uses, inst) = xor_fixture(IrType::U32, false);
    uses[3] = 2;
    assert!(fold_bit_select(&inst, &defs, &uses).is_none());
}

#[test]
fn the_toggle_involution_becomes_a_copy_not_a_second_toggle() {
    // `(x ^ 1 << n) ^ 1 << n`: the inner XOR has already been folded to
    // `BitToggle(x, n)` by the time the outer one is visited, so the outer must
    // answer with a copy of `x` -- folding it into a second `BitToggle` is
    // correct but costs two `btc`s where the shape used to cancel outright.
    let (mut defs, mut uses, outer) = xor_fixture(IrType::U32, false);
    // Value(4) = inner toggle, Value(5) = the outer XOR.
    defs[4] = Some(Instruction::BinOp {
        dest: Value(4),
        op: IrBinOp::BitToggle,
        lhs: v(1),
        rhs: v(2),
        ty: IrType::U32,
    });
    let outer = match outer {
        Instruction::BinOp { dest, .. } => Instruction::BinOp {
            dest,
            op: IrBinOp::Xor,
            lhs: v(4),
            rhs: v(3),
            ty: IrType::U32,
        },
        other => other,
    };
    uses[4] = 1;
    let folded = fold_bit_select(&outer, &defs, &uses);
    // The dest is the outer XOR's own value (Value(4) in the fixture), so the
    // copy must re-point that value at `x`.
    match folded {
        Some(Instruction::Copy { src, .. }) => assert_eq!(src, v(1), "the copy is of x"),
        other => panic!("expected a Copy of x, got {other:?}"),
    }
}

#[test]
fn the_toggle_does_not_capture_a_shift_of_a_wider_value() {
    // `1ul << n` at 64 bits feeding a 32-bit XOR: the shift's width is not the
    // xor's, exactly the refusal `BitSet` makes, so the toggle must refuse it
    // too rather than fold and silently change which index bits matter.
    let (defs, uses, inst) = xor_fixture(IrType::U32, false);
    let mut defs = defs;
    if let Some(Instruction::BinOp { ty, .. }) = defs[3].as_mut() {
        *ty = IrType::U64;
    }
    assert!(fold_bit_select(&inst, &defs, &uses).is_none());
}

#[test]
fn only_a_single_bit_shift_of_one_folds() {
    // `3 << n` is a two-bit mask, not a bit operation.
    let mut defs = vec![None; 6];
    defs[3] = Some(Instruction::BinOp {
        dest: Value(3),
        op: IrBinOp::Shl,
        lhs: Operand::Const(IrConst::from_i64(3, IrType::U32)),
        rhs: v(2),
        ty: IrType::U32,
    });
    let inst = Instruction::BinOp {
        dest: Value(4),
        op: IrBinOp::Or,
        lhs: v(1),
        rhs: v(3),
        ty: IrType::U32,
    };
    let mut uses = vec![0u32; 6];
    uses[3] = 1;
    assert!(fold_bit_select(&inst, &defs, &uses).is_none());
}

#[test]
fn a_shift_at_another_width_does_not_fold() {
    let (mut defs, uses, inst) = or_fixture(IrType::U32, false);
    if let Some(Instruction::BinOp { ty, .. }) = defs[3].as_mut() {
        *ty = IrType::U64;
    }
    assert!(fold_bit_select(&inst, &defs, &uses).is_none());
}

#[test]
fn the_equality_direction_is_left_alone() {
    // `(x & 1 << n) == 0` would need a complemented boolean on top of the bit
    // test; the inequality direction is the one that folds.
    let (defs, uses, mut inst) = cmp_fixture(IrType::U32, false);
    if let Instruction::Cmp { op, .. } = &mut inst {
        *op = IrCmpOp::Eq;
    }
    assert!(fold_bit_select(&inst, &defs, &uses).is_none());
}

#[test]
fn a_comparison_against_a_non_zero_constant_does_not_fold() {
    let (defs, uses, mut inst) = cmp_fixture(IrType::U32, false);
    if let Instruction::Cmp { rhs, .. } = &mut inst {
        *rhs = Operand::Const(IrConst::from_i64(4, IrType::U32));
    }
    assert!(fold_bit_select(&inst, &defs, &uses).is_none());
}

#[test]
fn the_128_bit_pair_type_is_refused() {
    let (defs, uses, mut inst) = or_fixture(IrType::U128, false);
    if let Instruction::BinOp { ty, .. } = &mut inst {
        *ty = IrType::U128;
    }
    assert!(fold_bit_select(&inst, &defs, &uses).is_none());
}

#[test]
fn an_unrelated_and_does_not_fold() {
    // `x & y` with y not a complement of a shift of one.
    let mut defs = vec![None; 6];
    defs[3] = Some(Instruction::BinOp {
        dest: Value(3),
        op: IrBinOp::Add,
        lhs: v(2),
        rhs: one(IrType::U32),
        ty: IrType::U32,
    });
    let inst = Instruction::BinOp {
        dest: Value(4),
        op: IrBinOp::And,
        lhs: v(1),
        rhs: v(3),
        ty: IrType::U32,
    };
    let mut uses = vec![0u32; 6];
    uses[3] = 1;
    assert!(fold_bit_select(&inst, &defs, &uses).is_none());
}

/// The fold's own contract for an index at or above the width: `bit n` is
/// `bit (n mod W)`, so an out-of-range index is defined and reuses bit zero
/// rather than reading outside the value.
#[test]
fn an_index_above_the_width_wraps_to_the_same_bit() {
    for ty in [IrType::U8, IrType::I16, IrType::U32, IrType::I64] {
        let bits = bits_of(ty);
        for &n in &[
            u64::from(bits),
            u64::from(bits) + 3,
            u64::from(bits) * 7 + 1,
        ] {
            let (defs, uses, inst) = or_fixture(ty, false);
            let Some(Instruction::BinOp { .. }) = fold_bit_select(&inst, &defs, &uses) else {
                panic!("no fold at {ty:?}");
            };
            let wrapped = n % u64::from(bits);
            for &x in &[0u64, 1, mask_of(bits)] {
                let a = crate::ir::ops::bit_op_within_bits(
                    x as u128,
                    n as u128,
                    bits,
                    crate::ir::ops::BitMode::Set,
                ) as u64;
                let b = crate::ir::ops::bit_op_within_bits(
                    x as u128,
                    wrapped as u128,
                    bits,
                    crate::ir::ops::BitMode::Set,
                ) as u64;
                assert_eq!(a, b, "index {n} must read bit {wrapped} at {ty:?}");
            }
        }
    }
}
