//! Tests for [`super::bit_test::fold_bit_tests`], the function-level
//! bit-test rewrites (`bit_idioms_bit_test.rs`).
//!
//! Every test drives the shipped entry point on a small IR function and
//! checks the exact instruction it leaves behind, plus -- for the rewrites
//! that change a value -- an exhaustive evaluation of the original and the
//! rewritten form over every index the operation can see.  The negative tests
//! pin the refusals that keep a rewrite from duplicating a producer, from
//! testing a bit outside the operation's width, or from running on a target
//! where it measured longer.

use super::bit_test::fold_bit_tests;
use super::*;
use crate::common::types::{
    set_target_elf_machine, set_target_ptr_size, target_elf_machine, target_ptr_size,
};
use crate::ir::reexports::{BasicBlock, BlockId, Terminator};

/// Restores the process-global target on drop.
struct TargetGuard {
    machine: u16,
    ptr: usize,
}

impl TargetGuard {
    fn set(machine: u16, ptr: usize) -> Self {
        let g = Self {
            machine: target_elf_machine(),
            ptr: target_ptr_size(),
        };
        set_target_elf_machine(machine);
        set_target_ptr_size(ptr);
        g
    }
    fn x86_64() -> Self {
        Self::set(62, 8)
    }
    fn i686() -> Self {
        Self::set(3, 4)
    }
    fn aarch64() -> Self {
        Self::set(183, 8)
    }
}

impl Drop for TargetGuard {
    fn drop(&mut self) {
        set_target_elf_machine(self.machine);
        set_target_ptr_size(self.ptr);
    }
}

fn v(n: u32) -> Operand {
    Operand::Value(Value(n))
}

fn c32(n: i64) -> Operand {
    Operand::Const(IrConst::from_i64(n, IrType::U32))
}

fn binop(dest: u32, op: IrBinOp, lhs: Operand, rhs: Operand) -> Instruction {
    Instruction::BinOp {
        dest: Value(dest),
        op,
        lhs,
        rhs,
        ty: IrType::U32,
    }
}

fn ne0(dest: u32, src: u32) -> Instruction {
    Instruction::Cmp {
        dest: Value(dest),
        op: IrCmpOp::Ne,
        lhs: v(src),
        rhs: c32(0),
        ty: IrType::U32,
    }
}

/// One block; Value(1) and Value(2) are parameters.
fn func(insts: Vec<Instruction>, ret: u32) -> IrFunction {
    let mut f = IrFunction::new("bt".to_string(), IrType::U32, vec![], false);
    let mut all = vec![
        Instruction::ParamRef {
            dest: Value(1),
            param_idx: 0,
            ty: IrType::U32,
        },
        Instruction::ParamRef {
            dest: Value(2),
            param_idx: 1,
            ty: IrType::U32,
        },
    ];
    all.extend(insts);
    f.blocks = vec![BasicBlock {
        label: BlockId(0),
        instructions: all,
        terminator: Terminator::Return(Some(v(ret))),
        source_spans: Vec::new(),
    }];
    f.next_value_id = 64;
    f
}

fn inst_defining(f: &IrFunction, dest: u32) -> &Instruction {
    f.blocks[0]
        .instructions
        .iter()
        .find(|i| i.dest().is_some_and(|d| d.0 == dest))
        .unwrap_or_else(|| panic!("no definition of v{dest}"))
}

fn op_of(f: &IrFunction, dest: u32) -> Option<IrBinOp> {
    match inst_defining(f, dest) {
        Instruction::BinOp { op, .. } => Some(*op),
        _ => None,
    }
}

/// The MSB-first byte map `x OP (0x80 >> (i & 7))` (FreeType
/// `pshrec.c:182`, `ttcmap.c:1868`): v3 = i & 7, v4 = 128 >> v3, v5 = x OP v4.
fn msb_first(op: IrBinOp, count_mask: i64) -> Vec<Instruction> {
    vec![
        binop(3, IrBinOp::And, v(2), c32(count_mask)),
        binop(4, IrBinOp::LShr, c32(128), v(3)),
        binop(5, op, v(1), v(4)),
    ]
}

/// The index `k ^ 7` inserted in front of an MSB-first rewrite at `dest`.
fn msb_index_of(f: &IrFunction, dest: u32) -> Operand {
    let Instruction::BinOp { rhs: index, .. } = inst_defining(f, dest) else {
        panic!("v{dest} is not a binop");
    };
    let Operand::Value(ix) = index else {
        panic!("constant index");
    };
    match inst_defining(f, ix.0) {
        Instruction::BinOp {
            op: IrBinOp::Xor,
            lhs,
            rhs,
            ..
        } => {
            assert_eq!(*lhs, v(3), "the index reads the masked count");
            assert_eq!(*rhs, c32(7), "k ^ 7 == 7 - k for k <= 7");
        }
        other => panic!("index producer {other:?}"),
    }
    *index
}

#[test]
fn a_zero_tested_and_of_a_one_hot_becomes_a_bit_test_in_place() {
    let _g = TargetGuard::x86_64();
    let mut f = func(
        vec![
            binop(3, IrBinOp::Shl, c32(1), v(2)),
            binop(4, IrBinOp::And, v(1), v(3)),
            ne0(5, 4),
        ],
        5,
    );
    assert!(fold_bit_tests(&mut f) >= 1);
    assert_eq!(op_of(&f, 4), Some(IrBinOp::BitTest));
    // The `!= 0` reader is untouched: BitTest is zero exactly when the AND is.
    assert!(matches!(
        inst_defining(&f, 5),
        Instruction::Cmp {
            op: IrCmpOp::Ne,
            ..
        }
    ));
}

/// dav1d `thread_task.c:820` `~prog & (1u << k)`: the test reads `prog` and
/// every reader's polarity flips.
#[test]
fn a_complemented_base_tests_the_inner_value_with_flipped_readers() {
    let _g = TargetGuard::x86_64();
    let mut f = func(
        vec![
            Instruction::UnaryOp {
                dest: Value(3),
                op: IrUnaryOp::Not,
                src: v(1),
                ty: IrType::U32,
            },
            binop(4, IrBinOp::Shl, c32(1), v(2)),
            binop(5, IrBinOp::And, v(3), v(4)),
            ne0(6, 5),
        ],
        6,
    );
    assert!(fold_bit_tests(&mut f) >= 1);
    match inst_defining(&f, 5) {
        Instruction::BinOp {
            op: IrBinOp::BitTest,
            lhs,
            ..
        } => assert_eq!(*lhs, v(1), "the complement is peeled"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        inst_defining(&f, 6),
        Instruction::Cmp {
            op: IrCmpOp::Eq,
            ..
        }
    ));
}

#[test]
fn the_msb_first_zero_test_indexes_bit_7_minus_k_on_x86_only() {
    for (guard, rewrites) in [
        (TargetGuard::x86_64 as fn() -> TargetGuard, true),
        (TargetGuard::i686, true),
        (TargetGuard::aarch64, false),
    ] {
        let _g = guard();
        let mut insts = msb_first(IrBinOp::And, 7);
        insts.push(ne0(6, 5));
        let mut f = func(insts, 6);
        fold_bit_tests(&mut f);
        if rewrites {
            assert_eq!(op_of(&f, 5), Some(IrBinOp::BitTest));
            msb_index_of(&f, 5);
        } else {
            assert_eq!(op_of(&f, 5), Some(IrBinOp::And), "measured longer off x86");
        }
    }
}

#[test]
fn an_msb_first_count_that_can_exceed_the_top_bit_is_refused() {
    let _g = TargetGuard::x86_64();
    // `i & 15` may shift 0x80 to zero: no single bit is tested.
    let mut insts = msb_first(IrBinOp::And, 15);
    insts.push(ne0(6, 5));
    let mut f = func(insts, 6);
    fold_bit_tests(&mut f);
    assert_eq!(op_of(&f, 5), Some(IrBinOp::And));
    let mut f = func(msb_first(IrBinOp::Or, 15), 5);
    fold_bit_tests(&mut f);
    assert_eq!(op_of(&f, 5), Some(IrBinOp::Or));
}

/// FreeType `ps_mask_set_bit`: `p | (0x80 >> (i & 7))` -> `BitSet(p, k ^ 7)`,
/// likewise for the toggle and (through the complement) the clear.
#[test]
fn msb_first_modifies_become_bit_operations_on_x86_64() {
    let _g = TargetGuard::x86_64();
    for (op, want) in [
        (IrBinOp::Or, IrBinOp::BitSet),
        (IrBinOp::Xor, IrBinOp::BitToggle),
    ] {
        let mut f = func(msb_first(op, 7), 5);
        assert!(fold_bit_tests(&mut f) >= 1, "{op:?}");
        assert_eq!(op_of(&f, 5), Some(want), "{op:?}");
        msb_index_of(&f, 5);
        // The constant shift is no longer read by the operation.
        let Instruction::BinOp { lhs, .. } = inst_defining(&f, 5) else {
            unreachable!()
        };
        assert_eq!(*lhs, v(1));
    }
    // `p & ~(0x80 >> k)`.
    let mut insts = vec![
        binop(3, IrBinOp::And, v(2), c32(7)),
        binop(4, IrBinOp::LShr, c32(128), v(3)),
        Instruction::UnaryOp {
            dest: Value(7),
            op: IrUnaryOp::Not,
            src: v(4),
            ty: IrType::U32,
        },
    ];
    insts.push(binop(5, IrBinOp::And, v(1), v(7)));
    let mut f = func(insts, 5);
    assert!(fold_bit_tests(&mut f) >= 1);
    assert_eq!(op_of(&f, 5), Some(IrBinOp::BitClear));
    msb_index_of(&f, 5);
}

#[test]
fn msb_first_modifies_stay_off_x86_64_and_keep_live_masks() {
    // i686 stages index and base through %eax: one instruction longer.
    for guard in [
        TargetGuard::i686 as fn() -> TargetGuard,
        TargetGuard::aarch64,
    ] {
        let _g = guard();
        let mut f = func(msb_first(IrBinOp::Or, 7), 5);
        fold_bit_tests(&mut f);
        assert_eq!(op_of(&f, 5), Some(IrBinOp::Or));
    }
    // A mask with a second reader stays materialised.
    let _g = TargetGuard::x86_64();
    let mut insts = msb_first(IrBinOp::Or, 7);
    insts.push(binop(6, IrBinOp::Add, v(5), v(4)));
    let mut f = func(insts, 6);
    fold_bit_tests(&mut f);
    assert_eq!(op_of(&f, 5), Some(IrBinOp::Or));
}

/// The index arithmetic itself: `(x OP (0x80 >> (i & 7)))` equals the bit
/// operation at index `(i & 7) ^ 7` for every byte value and every `i`.
#[test]
fn the_msb_first_index_is_exact_over_every_byte_and_count() {
    for x in 0u32..=255 {
        for i in 0u32..64 {
            let k = i & 7;
            let mask = 0x80u32 >> k;
            let bit = 1u32 << (k ^ 7);
            assert_eq!(mask, bit, "x={x} i={i}");
            assert_eq!(x | mask, x | bit);
            assert_eq!(x ^ mask, x ^ bit);
            assert_eq!(x & !mask, x & !bit);
            assert_eq!(x & mask != 0, (x >> (k ^ 7)) & 1 == 1);
        }
    }
}

/// A same-width cast under a bit test is a reinterpretation: the test reads
/// the cast's source (dav1d `thread_task.c:649`), and a 32 -> 64 extension
/// does too when the index is provably below 32 (brotli `decode.c:1251`).
#[test]
fn bit_test_bases_read_through_bit_preserving_casts() {
    let _g = TargetGuard::x86_64();
    let mut f = func(
        vec![
            Instruction::Cast {
                dest: Value(3),
                src: v(1),
                from_ty: IrType::U32,
                to_ty: IrType::I32,
            },
            Instruction::BinOp {
                dest: Value(4),
                op: IrBinOp::BitTest,
                lhs: v(3),
                rhs: v(2),
                ty: IrType::I32,
            },
        ],
        4,
    );
    assert!(fold_bit_tests(&mut f) >= 1);
    let Instruction::BinOp { lhs, .. } = inst_defining(&f, 4) else {
        unreachable!()
    };
    assert_eq!(*lhs, v(1));

    // 32 -> 64 zero extension with `i & 31`: tested at 32 bits, mask gone.
    let mut f = func(
        vec![
            Instruction::Cast {
                dest: Value(3),
                src: v(1),
                from_ty: IrType::U32,
                to_ty: IrType::U64,
            },
            Instruction::BinOp {
                dest: Value(5),
                op: IrBinOp::And,
                lhs: v(2),
                rhs: Operand::Const(IrConst::from_i64(31, IrType::U64)),
                ty: IrType::U64,
            },
            Instruction::BinOp {
                dest: Value(4),
                op: IrBinOp::BitTest,
                lhs: v(3),
                rhs: v(5),
                ty: IrType::U64,
            },
        ],
        4,
    );
    assert!(fold_bit_tests(&mut f) >= 1);
    match inst_defining(&f, 4) {
        Instruction::BinOp { lhs, rhs, ty, .. } => {
            assert_eq!(*lhs, v(1));
            assert_eq!(*ty, IrType::U32);
            assert_eq!(*rhs, v(2), "bt reduces the index mod 32 itself");
        }
        other => panic!("{other:?}"),
    }

    // Without the bound, bit 40 of the extension is not a bit of the source.
    let mut f = func(
        vec![
            Instruction::Cast {
                dest: Value(3),
                src: v(1),
                from_ty: IrType::U32,
                to_ty: IrType::U64,
            },
            Instruction::BinOp {
                dest: Value(4),
                op: IrBinOp::BitTest,
                lhs: v(3),
                rhs: v(2),
                ty: IrType::U64,
            },
        ],
        4,
    );
    fold_bit_tests(&mut f);
    let Instruction::BinOp { lhs, ty, .. } = inst_defining(&f, 4) else {
        unreachable!()
    };
    assert_eq!((*lhs, *ty), (v(3), IrType::U64));
}

/// The libxml control (`valid.c:309`): `long & (1 << d)` sign-extends the int
/// mask, so bit 31 selects bits 31..63 -- never a one-bit test.
#[test]
fn a_sign_extended_one_hot_is_not_a_bit_test() {
    let _g = TargetGuard::x86_64();
    let mut f = IrFunction::new("sext".to_string(), IrType::I64, vec![], false);
    f.blocks = vec![BasicBlock {
        label: BlockId(0),
        instructions: vec![
            Instruction::ParamRef {
                dest: Value(1),
                param_idx: 0,
                ty: IrType::I64,
            },
            Instruction::ParamRef {
                dest: Value(2),
                param_idx: 1,
                ty: IrType::I32,
            },
            Instruction::BinOp {
                dest: Value(3),
                op: IrBinOp::Shl,
                lhs: Operand::Const(IrConst::I32(1)),
                rhs: v(2),
                ty: IrType::I32,
            },
            Instruction::Cast {
                dest: Value(4),
                src: v(3),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: Value(5),
                op: IrBinOp::And,
                lhs: v(1),
                rhs: v(4),
                ty: IrType::I64,
            },
            Instruction::Cmp {
                dest: Value(6),
                op: IrCmpOp::Eq,
                lhs: v(5),
                rhs: Operand::Const(IrConst::I64(0)),
                ty: IrType::I64,
            },
        ],
        terminator: Terminator::Return(Some(v(6))),
        source_spans: Vec::new(),
    }];
    f.next_value_id = 64;
    fold_bit_tests(&mut f);
    assert_eq!(op_of(&f, 5), Some(IrBinOp::And));
}
