//! Exhaustive and adversarial tests for the ISPC mask lattice.
//!
//! Every positive test is paired with a mutation that must REFUSE the fold;
//! a checker that cannot say no is decoration.  The 16-lane sweep covers all
//! 65,536 byte masks, including arbitrary non-sign bits, so the fold is
//! proven over the whole input domain rather than a handful of examples.
use super::*;
use crate::common::source::Span;
use crate::common::types::{AddressSpace, IrType};
use crate::ir::reexports::{BasicBlock, BlockId, IrUnaryOp, Terminator};

fn alloca(dest: Value, size: usize, volatile: bool, semantic_volatile: bool) -> Instruction {
    Instruction::Alloca {
        dest,
        ty: IrType::Ptr,
        size,
        align: 1,
        volatile,
        semantic_volatile,
    }
}

fn intrinsic(
    dest: Option<Value>,
    op: IntrinsicOp,
    dest_ptr: Option<Value>,
    args: Vec<Operand>,
) -> Instruction {
    Instruction::Intrinsic {
        dest,
        op,
        dest_ptr,
        args,
    }
}

fn copy(dest: Value, src: Operand) -> Instruction {
    Instruction::Copy { dest, src }
}

/// Test-side adapter: the proof engine's `ByteImage` is a fixed-capacity
/// value type, so tests build one from a byte vector instead of allocating a
/// second heap buffer inside every assertion.
fn bi(v: Vec<u8>) -> ByteImage {
    ByteImage::from_slice(&v).expect("test image fits MAX_IMAGE")
}

fn gep(dest: Value, base: Value, offset: Operand) -> Instruction {
    Instruction::GetElementPtr {
        dest,
        base,
        offset,
        ty: IrType::I8,
    }
}

fn byte_store(ptr: Value, byte: u8, volatile: bool) -> Instruction {
    Instruction::Store {
        val: Operand::Const(IrConst::I8(byte as i8)),
        ptr,
        ty: IrType::I8,
        seg_override: AddressSpace::Default,
        volatile,
    }
}

fn block(insts: Vec<Instruction>, terminator: Terminator, spans: bool) -> BasicBlock {
    let source_spans = if spans {
        (0..insts.len()).map(|_| Span::dummy()).collect()
    } else {
        vec![]
    };
    BasicBlock {
        label: BlockId(0),
        instructions: insts,
        terminator,
        source_spans,
        // These fixtures model concrete (non-inlined) code, so provenance
        // stays sparse-empty rather than a parallel array of sentinels.
        source_inline_ids: Vec::new(),
    }
}

/// Builder for the exact final-IR shape LCCC emits for
/// `__builtin_ia32_pmovmskb128` on a constant 16-byte vector: a source
/// alloca tiled by 16 scalar constant stores, a whole-register load, a
/// whole-register store into a private temp, and the movemask read.
struct Image {
    insts: Vec<Instruction>,
    next: u32,
    source: Value,
    temp: Value,
    vector: Value,
    result: Value,
}

impl Image {
    fn new(bytes: &[u8; 16]) -> Self {
        let mut b = Image {
            insts: Vec::new(),
            next: 2,
            source: Value(0),
            temp: Value(1),
            vector: Value(0),
            result: Value(0),
        };
        b.insts.push(alloca(b.source, 16, false, false));
        b.insts.push(alloca(b.temp, 16, false, false));
        for (i, byte) in bytes.iter().enumerate() {
            let p = b.v();
            b.insts
                .push(gep(p, b.source, Operand::Const(IrConst::I64(i as i64))));
            b.insts.push(byte_store(p, *byte, false));
        }
        b.vector = b.v();
        b.insts.push(intrinsic(
            Some(b.vector),
            IntrinsicOp::VecLoadI8x16,
            None,
            vec![Operand::Value(b.source), Operand::Const(IrConst::I64(0))],
        ));
        b.insts.push(intrinsic(
            None,
            IntrinsicOp::VecStoreI8x16,
            Some(b.temp),
            vec![
                Operand::Value(b.vector),
                Operand::Value(b.temp),
                Operand::Const(IrConst::I64(0)),
            ],
        ));
        b.result = b.v();
        b.insts.push(intrinsic(
            Some(b.result),
            IntrinsicOp::Pmovmskb128,
            None,
            vec![Operand::Value(b.temp)],
        ));
        b
    }

    fn v(&mut self) -> Value {
        let value = Value(self.next);
        self.next += 1;
        value
    }

    fn function(&self, spans: bool) -> IrFunction {
        let mut f = IrFunction::new("mask".into(), IrType::I32, vec![], false);
        f.blocks.push(block(
            self.insts.clone(),
            Terminator::Return(Some(Operand::Value(self.result))),
            spans,
        ));
        f.next_value_id = self.next;
        f
    }
}

/// Run the pass and report the MOVEMASK's folded constant.  `run_function`'s
/// edit count is NOT a sound oracle for these tests: the load-of-constant
/// consumer legitimately adds an edit of its own while leaving the mask
/// verdict unchanged.  Asserting the semantic result is both stricter and
/// immune to new consumers.
fn movemask_result_after(func: &mut IrFunction, result: Value) -> Option<i32> {
    run_function(func);
    result_const(func, result)
}

fn result_const(func: &IrFunction, result: Value) -> Option<i32> {
    func.blocks[0]
        .instructions
        .iter()
        .find_map(|inst| match inst {
            Instruction::Copy {
                dest,
                src: Operand::Const(IrConst::I32(v)),
            } if *dest == result => Some(*v),
            _ => None,
        })
}

/// `Instruction` has no `PartialEq` (it carries spans-free but structurally
/// rich payloads), so structural assertions destructure the variant.
fn assert_copy(inst: &Instruction, dest: Value, src: Operand, label: &str) {
    match inst {
        Instruction::Copy { dest: d, src: s } => {
            assert_eq!(*d, dest, "{label}: destination");
            assert_eq!(*s, src, "{label}: source");
        }
        other => panic!("{label}: expected Copy, got {other:?}"),
    }
}

#[test]
fn lattice_is_exhaustive_over_all_65_536_byte_masks() {
    for mask in 0u32..=0xFFFF {
        // Low seven bits are arbitrary data, never consulted by the lattice.
        let mut bytes = [0u8; 16];
        for (i, b) in bytes.iter_mut().enumerate() {
            let noise = ((mask as usize)
                .wrapping_mul(0x9E37_79B9)
                .wrapping_add(i * 37)
                & 0x7F) as u8;
            *b = noise | if mask & (1 << i) != 0 { 0x80 } else { 0 };
        }
        let expected = match mask {
            0 => MaskStatus::AllOff,
            0xFFFF => MaskStatus::AllOn,
            other => MaskStatus::Mixed(other),
        };
        assert_eq!(
            MaskStatus::from_lanes(&bytes, 8),
            expected,
            "mask {mask:#06x}"
        );
    }
}

#[test]
fn lane_widths_read_only_the_sign_bit_of_each_lane() {
    // Four F32 lanes: -0.0, +1.0, -1.0, +0.0 -> bits 0b0101.
    let mut ps = [0u8; 16];
    ps[3] = 0x80;
    ps[4..8].copy_from_slice(&1.0f32.to_bits().to_le_bytes());
    ps[11] = 0x80 | 0x3F;
    assert_eq!(MaskStatus::from_lanes(&ps, 32), MaskStatus::Mixed(0b0101));
    // Two F64 lanes: -0.0, +0.0 -> bit 0b01.
    let mut pd = [0u8; 16];
    pd[7] = 0x80;
    assert_eq!(MaskStatus::from_lanes(&pd, 64), MaskStatus::Mixed(0b01));
    // 32 lanes: all-on must not shift a u32 by 32.
    assert_eq!(MaskStatus::from_lanes(&[0x80u8; 32], 8), MaskStatus::AllOn);
    assert_eq!(MaskStatus::from_lanes(&[0u8; 32], 8), MaskStatus::AllOff);
    // Malformed inputs refuse rather than truncate.
    // Three 8-bit lanes is a well-formed (if unusual) three-lane mask.
    assert_eq!(MaskStatus::from_lanes(&[0u8; 3], 8), MaskStatus::AllOff);
    assert_eq!(MaskStatus::from_lanes(&[], 8), MaskStatus::Unknown);
    assert_eq!(MaskStatus::from_lanes(&[0u8; 17], 32), MaskStatus::Unknown);
    assert_eq!(MaskStatus::from_lanes(&[0u8; 16], 24), MaskStatus::Unknown);
    assert_eq!(MaskStatus::from_lanes(&[0u8; 40], 8), MaskStatus::Unknown);
}

#[test]
fn byte_classification_is_independent_of_packed_lane_width() {
    assert_eq!(MaskStatus::from_lanes(&[0x80; 16], 8), MaskStatus::AllOn);
    assert_eq!(MaskStatus::from_lanes(&[0x01; 16], 8), MaskStatus::AllOff);
    assert_eq!(MaskStatus::from_lanes(&[0xFF; 32], 8), MaskStatus::AllOn);
    let mut mixed = [0u8; 16];
    mixed[7] = 0x80;
    assert!(matches!(
        MaskStatus::from_lanes(&mixed, 8),
        MaskStatus::Mixed(_)
    ));
    assert_eq!(MaskStatus::from_lanes(&[], 8), MaskStatus::Unknown);
    // A mask whose *low* bits vary but whose sign bits agree is still exact.
    let mut low = [0u8; 16];
    for (i, b) in low.iter_mut().enumerate() {
        *b = 0x80 | i as u8;
    }
    assert_eq!(MaskStatus::from_lanes(&low, 8), MaskStatus::AllOn);
}

/// A well-formed dword/qword selector mask has DON'T-CARE bytes in the low
/// part of each lane.  `from_bytes` used to call such a mask `Unknown` and
/// miss the fold; `blend_shape` + `from_lanes` must see AllOn/AllOff.  The
/// byte-lane blends MUST still refuse the very same image, because
/// `vpblendvb` reads bit 7 of every byte.
#[test]
fn blend_selector_lanes_are_classified_exactly_not_bytewise() {
    // Build each selector image at the EXACT byte width of its blend, so
    // the 32-byte ops are not silently tested with a truncated 16-byte mask.
    let dword_image = |width: usize, on: bool| -> Vec<u8> {
        (0..width)
            .map(|i| {
                let high_byte = i % 4 == 3;
                let sign = if on { high_byte } else { !high_byte };
                if sign { 0x80 } else { 0 }
            })
            .collect()
    };
    let qword_image = |width: usize, on: bool| -> Vec<u8> {
        (0..width)
            .map(|i| {
                let high_byte = i % 8 == 7;
                let sign = if on { high_byte } else { !high_byte };
                if sign { 0x80 } else { 0 }
            })
            .collect()
    };

    for (op, width, lane_bits, qword) in [
        (IntrinsicOp::VecBlendvF32x4, 16usize, 32usize, false),
        (IntrinsicOp::VecBlendvI32x4, 16, 32, false),
        (IntrinsicOp::VecBlendvF32x8, 32, 32, false),
        (IntrinsicOp::VecBlendvI32x8, 32, 32, false),
        (IntrinsicOp::VecBlendvF64x2, 16, 64, true),
        (IntrinsicOp::VecBlendvF64x4, 32, 64, true),
    ] {
        let (shape_width, selector) = blend_shape(op).expect("blend op");
        assert_eq!(shape_width, width, "{op:?} mask width");
        assert_eq!(selector, lane_bits, "{op:?} selector width");
        let image = if qword { qword_image } else { dword_image };
        let on = image(width, true);
        let off = image(width, false);
        assert_eq!(on.len(), width, "{op:?} all-on image width");
        assert_eq!(off.len(), width, "{op:?} all-off image width");
        assert_eq!(
            MaskStatus::from_lanes(&on, selector),
            MaskStatus::AllOn,
            "{op:?} well-formed all-on selector"
        );
        assert_eq!(
            MaskStatus::from_lanes(&off, selector),
            MaskStatus::AllOff,
            "{op:?} well-formed all-off selector"
        );
        // The SAME image is NOT an all-on BYTE mask.
        assert!(
            !matches!(MaskStatus::from_lanes(&on, 8), MaskStatus::AllOn),
            "{op:?} byte-lane agreement must be stronger"
        );
    }
    // Byte/word blends must use the 8-bit selector, so their shape proves
    // that a dword-style mask cannot be misclassified as AllOn.
    for op in [
        IntrinsicOp::VecBlendvI8x16,
        IntrinsicOp::VecBlendvI16x8,
        IntrinsicOp::VecBlendvI8x32,
        IntrinsicOp::VecBlendvI16x16,
    ] {
        assert_eq!(blend_shape(op).expect("blend op").1, 8, "{op:?}");
    }
}

/// End-to-end fold of the exact GCC/Clang case: a dword all-on selector with
/// mixed payload bytes must select the true arm; the byte-lane blend with the
/// same mask must NOT fold.
#[test]
fn dword_blend_with_dont_care_payload_bytes_folds_and_byte_blend_refuses() {
    let f = Operand::Const(IrConst::I64(11));
    let t = Operand::Const(IrConst::I64(22));
    // I128 little-endian: bytes 0,1,2 = 0, byte 3 = 0x80, per dword.
    let dword_all_on = Operand::Const(IrConst::I128(i128::from_le_bytes([
        0, 0, 0, 0x80, 0, 0, 0, 0x80, 0, 0, 0, 0x80, 0, 0, 0, 0x80,
    ])));
    let mut d = blend_function(dword_all_on, f, t, IntrinsicOp::VecBlendvF32x4);
    assert_eq!(run_function(&mut d), 1, "dword all-on selector must fold");
    assert_copy(&d.blocks[0].instructions[0], Value(9), t, "dword blend");

    let mut b = blend_function(dword_all_on, f, t, IntrinsicOp::VecBlendvI8x16);
    assert_eq!(run_function(&mut b), 0, "byte-lane blend must refuse");

    // A qword selector: sign bit only in bytes 7 and 15.
    let qword_all_on = Operand::Const(IrConst::I128(i128::from_le_bytes([
        0, 0, 0, 0, 0, 0, 0, 0x80, 0, 0, 0, 0, 0, 0, 0, 0x80,
    ])));
    let mut q = blend_function(qword_all_on, f, t, IntrinsicOp::VecBlendvF64x2);
    assert_eq!(run_function(&mut q), 1, "qword all-on selector must fold");
    assert_copy(&q.blocks[0].instructions[0], Value(9), t, "qword blend");
    let mut qb = blend_function(qword_all_on, f, t, IntrinsicOp::VecBlendvI16x8);
    assert_eq!(run_function(&mut qb), 0, "word-lane blend must refuse");
}

/// The measured repro: GCC 16.2 and Clang 23.1.5 both emit
/// `movl $53069, %eax`; LCCC emitted 22 instructions before this pass.
#[test]
fn constant_byte_mask_folds_to_the_exact_gcc_clang_constant() {
    let bytes = [
        0xFF, 0x00, 0xFF, 0xFF, 0x00, 0x00, 0xFF, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0xFF,
        0xFF,
    ];
    let image = Image::new(&bytes);
    let result = image.result;
    let mut func = image.function(true);
    assert_eq!(run_function(&mut func), 1);
    assert_eq!(result_const(&func, result), Some(53069));
    assert_eq!(
        func.blocks[0].source_spans.len(),
        func.blocks[0].instructions.len(),
        "in-place rewrite must keep spans parallel"
    );
    assert_eq!(run_function(&mut func), 0, "the fold is idempotent");
}

#[test]
fn every_mixed_16_lane_image_folds_to_its_exact_bitmask() {
    for mask in [
        0u32, 1, 0x00FF, 0x0F0F, 0x5555, 0xAAAA, 0xCF4D, 0xFFFE, 0xFFFF,
    ] {
        let mut bytes = [0x01u8; 16];
        for (i, b) in bytes.iter_mut().enumerate() {
            if mask & (1 << i) != 0 {
                *b = 0x81;
            }
        }
        let image = Image::new(&bytes);
        let result = image.result;
        let mut func = image.function(false);
        assert_eq!(run_function(&mut func), 1, "mask {mask:#06x}");
        assert_eq!(
            result_const(&func, result),
            Some(mask as i32),
            "mask {mask:#06x}"
        );
    }
}

/// A negative helper: mutate the canonical image and insist the fold is
/// refused.  Each case also asserts the mutation actually changed the IR, so
/// every rejection is non-vacuous.
fn must_refuse(name: &str, mutate: impl FnOnce(&mut Image)) {
    let mut image = Image::new(&[0x80u8; 16]);
    let original = format!("{:?}", image.insts);
    mutate(&mut image);
    assert_ne!(
        format!("{:?}", image.insts),
        original,
        "negative case {name} did not mutate the IR"
    );
    let mut func = image.function(false);
    assert_eq!(run_function(&mut func), 0, "negative case {name} folded");
    assert!(
        func.blocks[0].instructions.iter().any(|i| {
            matches!(i, Instruction::Intrinsic { op, .. } if *op == IntrinsicOp::Pmovmskb128)
        }),
        "negative case {name} rewrote the intrinsic"
    );
}

/// Red-team round 8: overlapping constant writes used to be refused with the
/// claim "store order is not proven".  That was a MISSED OPTIMIZATION, not a
/// soundness requirement: `object_image` only accepts writes in the read's
/// own block at indices `<= read_index`, and the mention index is built in
/// program order.  The order is therefore fully proven and exact
/// last-write-wins is semantics-preserving.  This test pins the value, and a
/// companion test below proves the reverse order gives a DIFFERENT value.
#[test]
fn overlapping_writes_use_proven_program_order_last_write_wins() {
    // Base image: every byte has its sign bit set -> mask 0xFFFF.
    let mut im = Image::new(&[0x80u8; 16]);
    let result = im.result;
    // Insert a LATER overwrite of byte 0 with 0x07 (sign bit clear), just
    // before the VecLoad.  It must win over the earlier 0x80 store.
    let at = im
        .insts
        .iter()
        .position(
            |i| matches!(i, Instruction::Intrinsic { op, .. } if *op == IntrinsicOp::VecLoadI8x16),
        )
        .expect("fixture must contain the whole-vector load");
    let p = im.v();
    im.insts
        .insert(at, gep(p, im.source, Operand::Const(IrConst::I64(0))));
    im.insts.insert(at + 1, byte_store(p, 7, false));
    let mut f = im.function(true);
    assert!(run_function(&mut f) > 0, "the fold must fire");
    assert_eq!(
        result_const(&f, result),
        Some(0xFFFE),
        "last write (0x07) must clear bit 0"
    );
    assert_eq!(
        f.blocks[0].source_spans.len(),
        f.blocks[0].instructions.len(),
        "spans stay parallel"
    );

    // Reverse order: the late write is 0x87 (sign bit SET), so byte 0 goes
    // back to a set mask bit.  A checker that ignored write order could not
    // distinguish these two cases.
    let mut im2 = Image::new(&[0x07u8; 16]);
    let result2 = im2.result;
    let at2 = im2
        .insts
        .iter()
        .position(
            |i| matches!(i, Instruction::Intrinsic { op, .. } if *op == IntrinsicOp::VecLoadI8x16),
        )
        .unwrap();
    let p2 = im2.v();
    im2.insts
        .insert(at2, gep(p2, im2.source, Operand::Const(IrConst::I64(0))));
    im2.insts.insert(at2 + 1, byte_store(p2, 0x87, false));
    let mut f2 = im2.function(false);
    run_function(&mut f2);
    assert_eq!(
        result_const(&f2, result2),
        Some(0x0001),
        "last write (0x87) must set bit 0 only"
    );
}

#[test]
fn every_unsound_image_is_refused() {
    must_refuse("gap in the byte tiling", |im| {
        let victim = im.insts.iter().find_map(|inst| match inst {
            Instruction::GetElementPtr {
                dest,
                offset: Operand::Const(IrConst::I64(3)),
                ..
            } => Some(*dest),
            _ => None,
        });
        let victim = victim.expect("the fixture must contain byte 3's GEP");
        im.insts
            .retain(|inst| !matches!(inst, Instruction::Store { ptr, .. } if *ptr == victim));
    });
    // A write AFTER the read (or in another block) is the genuinely
    // unprovable ordering case: program order at the read point is unknown.
    must_refuse("write after the mask read", |im| {
        let p = im.v();
        im.insts
            .push(gep(p, im.source, Operand::Const(IrConst::I64(0))));
        im.insts.push(byte_store(p, 7, false));
    });
    must_refuse("non-constant store", |im| {
        for inst in &mut im.insts {
            if let Instruction::Store { val, .. } = inst {
                *val = Operand::Value(im.vector);
                break;
            }
        }
    });
    must_refuse("volatile store", |im| {
        for inst in &mut im.insts {
            if let Instruction::Store { volatile, .. } = inst {
                *volatile = true;
                break;
            }
        }
    });
    must_refuse("semantic-volatile alloca", |im| {
        for inst in &mut im.insts {
            if let Instruction::Alloca {
                dest,
                semantic_volatile,
                ..
            } = inst
            {
                if *dest == im.source {
                    *semantic_volatile = true;
                }
            }
        }
    });
    must_refuse("address used in arithmetic", |im| {
        let dest = im.v();
        im.insts.insert(
            2,
            Instruction::UnaryOp {
                dest,
                op: IrUnaryOp::Not,
                src: Operand::Value(im.source),
                ty: IrType::Ptr,
            },
        );
    });
    must_refuse("variable-offset GEP loses provenance", |im| {
        let dest = im.v();
        im.insts
            .insert(2, gep(dest, im.source, Operand::Value(im.vector)));
    });
    must_refuse("unknown intrinsic touches the address", |im| {
        im.insts.insert(
            im.insts.len() - 1,
            intrinsic(
                None,
                IntrinsicOp::Crc32_64,
                None,
                vec![Operand::Value(im.source)],
            ),
        );
    });
    must_refuse("object too small for the read", |im| {
        for inst in &mut im.insts {
            if let Instruction::Alloca { dest, size, .. } = inst {
                if *dest == im.temp {
                    *size = 8;
                }
            }
        }
    });
    must_refuse("write after the read", |im| {
        let p = im.v();
        im.insts
            .push(gep(p, im.temp, Operand::Const(IrConst::I64(0))));
        im.insts.push(byte_store(p, 0, false));
    });
}

#[test]
fn a_write_in_a_different_block_is_refused() {
    let image = Image::new(&[0x80u8; 16]);
    let mut f = image.function(false);
    let p = Value(image.next);
    f.blocks.push(BasicBlock {
        label: BlockId(1),
        instructions: vec![
            gep(p, image.temp, Operand::Const(IrConst::I64(0))),
            byte_store(p, 0, false),
        ],
        terminator: Terminator::Return(None),
        source_spans: vec![],
        source_inline_ids: Vec::new(),
    });
    f.next_value_id = image.next + 1;
    assert_eq!(run_function(&mut f), 0);
    // The same shape with the extra block removed DOES fold, so the
    // rejection is attributable to the cross-block write, not to the fixture.
    let mut positive = image.function(false);
    assert_eq!(run_function(&mut positive), 1);
}

#[test]
fn an_escaping_address_is_refused() {
    let image = Image::new(&[0x80u8; 16]);
    let mut func = image.function(false);
    func.blocks[0].terminator = Terminator::Return(Some(Operand::Value(image.source)));
    let facts = Facts::new(&func);
    let idx = func.blocks[0]
        .instructions
        .iter()
        .position(
            |i| matches!(i, Instruction::Intrinsic { op, .. } if *op == IntrinsicOp::Pmovmskb128),
        )
        .unwrap();
    assert!(fold(&facts, &func.blocks[0].instructions[idx], 0, idx).is_none());
}

#[test]
fn a_256_bit_byte_mask_folds_without_shift_overflow() {
    let mut f = IrFunction::new("wide".into(), IrType::I32, vec![], false);
    let mut insts = vec![alloca(Value(0), 32, false, false)];
    for i in 0..32 {
        let p = Value(100 + i);
        insts.push(gep(p, Value(0), Operand::Const(IrConst::I64(i as i64))));
        // Alternating sign bits: 0x5555_5555, proving Mixed (not AllOn).
        let byte = if i % 2 == 0 { 0x81 } else { 0x01 };
        insts.push(byte_store(p, byte, false));
    }
    insts.push(intrinsic(
        Some(Value(200)),
        IntrinsicOp::Pmovmskb256,
        None,
        vec![Operand::Value(Value(0))],
    ));
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Value(Value(200)))),
        false,
    ));
    f.next_value_id = 201;
    assert_eq!(run_function(&mut f), 1);
    assert_eq!(result_const(&f, Value(200)), Some(0x5555_5555u32 as i32));

    let mut all = IrFunction::new("wide_all".into(), IrType::I32, vec![], false);
    let mut insts = vec![alloca(Value(0), 32, false, false)];
    for i in 0..32 {
        let p = Value(100 + i);
        insts.push(gep(p, Value(0), Operand::Const(IrConst::I64(i as i64))));
        insts.push(byte_store(p, 0xFF, false));
    }
    insts.push(intrinsic(
        Some(Value(200)),
        IntrinsicOp::Pmovmskb256,
        None,
        vec![Operand::Value(Value(0))],
    ));
    all.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Value(Value(200)))),
        false,
    ));
    all.next_value_id = 201;
    assert_eq!(run_function(&mut all), 1);
    assert_eq!(result_const(&all, Value(200)), Some(-1));
}

#[test]
fn float_movemasks_fold_from_sign_bits_not_byte_masks() {
    for (op, lane_bytes, lanes, expected) in [
        (IntrinsicOp::MovemaskPs128, 4usize, 4usize, 0b1001i32),
        (IntrinsicOp::MovemaskPd128, 8usize, 2usize, 0b10i32),
    ] {
        let mut bytes = [0u8; 16];
        for lane in 0..lanes {
            if expected & (1 << lane) != 0 {
                bytes[lane * lane_bytes + lane_bytes - 1] = 0x80;
            }
        }
        // A low-bit byte MSB elsewhere would be a byte-mask, not a lane mask.
        bytes[0] = 0x01;
        let mut f = IrFunction::new("fp".into(), IrType::I32, vec![], false);
        let mut insts = vec![alloca(Value(0), 16, false, false)];
        for (i, byte) in bytes.iter().enumerate() {
            let p = Value(10 + i as u32);
            insts.push(gep(p, Value(0), Operand::Const(IrConst::I64(i as i64))));
            insts.push(byte_store(p, *byte, false));
        }
        insts.push(intrinsic(
            Some(Value(99)),
            op,
            None,
            vec![Operand::Value(Value(0))],
        ));
        f.blocks.push(block(
            insts,
            Terminator::Return(Some(Operand::Value(Value(99)))),
            false,
        ));
        f.next_value_id = 100;
        assert_eq!(run_function(&mut f), 1, "{op:?}");
        assert_eq!(result_const(&f, Value(99)), Some(expected), "{op:?}");
    }
}

fn blend_function(
    mask: Operand,
    false_arm: Operand,
    true_arm: Operand,
    op: IntrinsicOp,
) -> IrFunction {
    let mut f = IrFunction::new("blend".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        vec![intrinsic(
            Some(Value(9)),
            op,
            None,
            vec![false_arm, true_arm, mask],
        )],
        Terminator::Return(Some(Operand::Value(Value(9)))),
        false,
    ));
    f.next_value_id = 10;
    f
}

#[test]
fn blends_fold_only_for_proven_trivial_masks_or_identical_arms() {
    let f = Operand::Const(IrConst::I64(11));
    let t = Operand::Const(IrConst::I64(22));
    let cases: [(IntrinsicOp, usize); 10] = [
        (IntrinsicOp::VecBlendvF32x4, 16),
        (IntrinsicOp::VecBlendvF64x2, 16),
        (IntrinsicOp::VecBlendvI32x4, 16),
        (IntrinsicOp::VecBlendvI8x16, 16),
        (IntrinsicOp::VecBlendvI16x8, 16),
        (IntrinsicOp::VecBlendvF32x8, 32),
        (IntrinsicOp::VecBlendvF64x4, 32),
        (IntrinsicOp::VecBlendvI32x8, 32),
        (IntrinsicOp::VecBlendvI8x32, 32),
        (IntrinsicOp::VecBlendvI16x16, 32),
    ];
    for (op, width) in cases {
        // A 16-byte image is only valid for a 16-byte blend; a 32-byte blend
        // needs a 32-byte producer.  Both are exercised below.
        if width == 16 {
            let all_off = Operand::Const(IrConst::I128(0));
            let mut func = blend_function(all_off, f, t, op);
            assert_eq!(run_function(&mut func), 1, "{op:?} all-off");
            assert_copy(&func.blocks[0].instructions[0], Value(9), f, "all-off");

            let all_on = Operand::Const(IrConst::I128(-1));
            let mut func = blend_function(all_on, f, t, op);
            assert_eq!(run_function(&mut func), 1, "{op:?} all-on");
            assert_copy(&func.blocks[0].instructions[0], Value(9), t, "all-on");

            let mixed = Operand::Const(IrConst::I128(0x8000_0000_0000_0000u64 as i128));
            let mut func = blend_function(mixed, f, t, op);
            assert_eq!(
                run_function(&mut func),
                0,
                "{op:?} mixed mask must not fold"
            );
        } else {
            // 32-byte all-off / all-on masks via a zero/broadcast producer.
            for (producer, extra, want) in [
                (IntrinsicOp::VecZeroI32x8, Vec::<Instruction>::new(), f),
                (IntrinsicOp::VecBroadcastI32x8, Vec::<Instruction>::new(), t),
            ] {
                let mut func = IrFunction::new("wide_blend".into(), IrType::I32, vec![], false);
                let mut insts = Vec::new();
                let args = if producer == IntrinsicOp::VecBroadcastI32x8 {
                    vec![Operand::Const(IrConst::I32(-1))]
                } else {
                    vec![]
                };
                insts.push(intrinsic(Some(Value(1)), producer, None, args));
                insts.extend(extra);
                insts.push(intrinsic(
                    Some(Value(9)),
                    op,
                    None,
                    vec![f, t, Operand::Value(Value(1))],
                ));
                func.blocks.push(block(
                    insts,
                    Terminator::Return(Some(Operand::Value(Value(9)))),
                    false,
                ));
                func.next_value_id = 10;
                assert_eq!(run_function(&mut func), 1, "{op:?} {producer:?}");
                assert_copy(
                    func.blocks[0].instructions.last().expect("blend present"),
                    Value(9),
                    want,
                    &format!("{op:?} {producer:?}"),
                );
            }
        }

        let unknown = Operand::Value(Value(77));
        let mut func = blend_function(unknown, f, f, op);
        assert_eq!(run_function(&mut func), 1, "{op:?} identical arms");
        assert_copy(
            &func.blocks[0].instructions[0],
            Value(9),
            f,
            "identical arms",
        );

        let mut func = blend_function(Operand::Value(Value(77)), f, t, op);
        assert_eq!(
            run_function(&mut func),
            0,
            "{op:?} unknown mask must not fold"
        );
    }
}

#[test]
fn zero_vector_producer_is_an_all_off_mask() {
    let mut f = IrFunction::new("zero".into(), IrType::I32, vec![], false);
    let insts = vec![
        intrinsic(Some(Value(1)), IntrinsicOp::VecZeroI32x8, None, vec![]),
        intrinsic(
            Some(Value(2)),
            IntrinsicOp::VecBlendvI32x8,
            None,
            vec![
                Operand::Const(IrConst::I64(3)),
                Operand::Const(IrConst::I64(4)),
                Operand::Value(Value(1)),
            ],
        ),
    ];
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Value(Value(2)))),
        false,
    ));
    f.next_value_id = 3;
    assert_eq!(run_function(&mut f), 1);
    assert_copy(
        &f.blocks[0].instructions[1],
        Value(2),
        Operand::Const(IrConst::I64(3)),
        "zero-vector all-off",
    );
}

#[test]
fn phi_of_one_known_image_is_known_and_a_disagreement_is_not() {
    let known = Operand::Const(IrConst::I128(-1));
    let mut f = IrFunction::new("phi".into(), IrType::I32, vec![], false);
    let insts = vec![
        Instruction::Phi {
            dest: Value(1),
            ty: IrType::I128,
            incoming: vec![(known, BlockId(0)), (known, BlockId(1))],
        },
        intrinsic(
            Some(Value(2)),
            IntrinsicOp::VecBlendvI32x4,
            None,
            vec![
                Operand::Const(IrConst::I64(3)),
                Operand::Const(IrConst::I64(4)),
                Operand::Value(Value(1)),
            ],
        ),
    ];
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Value(Value(2)))),
        false,
    ));
    f.next_value_id = 3;
    assert_eq!(run_function(&mut f), 1);

    let mut g = IrFunction::new("phi_disagree".into(), IrType::I32, vec![], false);
    let insts = vec![
        Instruction::Phi {
            dest: Value(1),
            ty: IrType::I128,
            incoming: vec![
                (Operand::Const(IrConst::I128(-1)), BlockId(0)),
                (Operand::Const(IrConst::I128(0)), BlockId(1)),
            ],
        },
        intrinsic(
            Some(Value(2)),
            IntrinsicOp::VecBlendvI32x4,
            None,
            vec![
                Operand::Const(IrConst::I64(3)),
                Operand::Const(IrConst::I64(4)),
                Operand::Value(Value(1)),
            ],
        ),
    ];
    g.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Value(Value(2)))),
        false,
    ));
    g.next_value_id = 3;
    assert_eq!(run_function(&mut g), 0);
}

#[test]
fn recursive_copy_cycles_are_refused() {
    let mut f = IrFunction::new("cycle".into(), IrType::I32, vec![], false);
    let insts = vec![
        copy(Value(1), Operand::Value(Value(2))),
        copy(Value(2), Operand::Value(Value(1))),
        intrinsic(
            Some(Value(3)),
            IntrinsicOp::VecBlendvI32x4,
            None,
            vec![
                Operand::Const(IrConst::I64(3)),
                Operand::Const(IrConst::I64(4)),
                Operand::Value(Value(1)),
            ],
        ),
    ];
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Value(Value(3)))),
        false,
    ));
    f.next_value_id = 4;
    assert_eq!(run_function(&mut f), 0);
}

// ---------------------------------------------------------------------------
// Red-team regressions for the second audit round.  Each test pins one
// specific defect class: width mismatches, silent lane extension, the
// half-wide pair store, provenance depth, and the dead-write reclaimer.
// ---------------------------------------------------------------------------

fn load(dest: Value, ptr: Value) -> Instruction {
    Instruction::Load {
        dest,
        ptr,
        ty: IrType::I8,
        volatile: false,
        seg_override: AddressSpace::Default,
    }
}

fn cast(dest: Value, src: Operand) -> Instruction {
    Instruction::Cast {
        dest,
        src,
        from_ty: IrType::Ptr,
        to_ty: IrType::I64,
    }
}

fn unary_not(dest: Value, src: Operand) -> Instruction {
    Instruction::UnaryOp {
        dest,
        op: IrUnaryOp::Not,
        src,
        ty: IrType::I64,
    }
}

/// A `SetEpi32`-style splat as a blend mask, with a controllable constant.
fn splat_blend(c: IrConst, op: IntrinsicOp) -> IrFunction {
    let mut f = IrFunction::new("splat".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        vec![
            intrinsic(
                Some(Value(1)),
                IntrinsicOp::SetEpi32,
                None,
                vec![Operand::Const(c)],
            ),
            intrinsic(
                Some(Value(2)),
                op,
                None,
                vec![
                    Operand::Const(IrConst::I64(3)),
                    Operand::Const(IrConst::I64(4)),
                    Operand::Value(Value(1)),
                ],
            ),
        ],
        Terminator::Return(Some(Operand::Value(Value(2)))),
        false,
    ));
    f.next_value_id = 3;
    f
}

#[test]
fn splat_requires_an_exact_width_constant_lane() {
    // I32(-1) -> four 0xFF bytes -> AllOn, exact.
    let mut f = splat_blend(IrConst::I32(-1), IntrinsicOp::VecBlendvI32x4);
    assert_eq!(run_function(&mut f), 1);
    assert_copy(
        &f.blocks[0].instructions[1],
        Value(2),
        Operand::Const(IrConst::I64(4)),
        "exact I32 lane",
    );

    // I8(-1) is ONE byte.  Zero-extending it to a four-byte lane would
    // produce 0x000000FF and a DIFFERENT sign bit than sign-extending it to
    // 0xFFFFFFFF.  There is no IR evidence of which the producer intended,
    // so the pass must refuse instead of guessing.
    let mut g = splat_blend(IrConst::I8(-1), IntrinsicOp::VecBlendvI32x4);
    assert_eq!(run_function(&mut g), 0, "silently extended lane folded");

    // F64 has eight bytes; a four-byte lane slot is not F64.
    let mut h = splat_blend(IrConst::F64(-0.0), IntrinsicOp::VecBlendvI32x4);
    assert_eq!(run_function(&mut h), 0, "over-wide lane folded");
}

#[test]
fn zero_vector_width_must_match_the_consumer() {
    // A 128-bit zero cannot satisfy a 256-bit mask: legacy SSE leaves the
    // upper half of the YMM register undefined, so claiming 32 zero bytes
    // is a miscompile, not a fold.
    let mut f = IrFunction::new("zw".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        vec![
            intrinsic(Some(Value(1)), IntrinsicOp::VecZeroI32x4, None, vec![]),
            intrinsic(
                Some(Value(2)),
                IntrinsicOp::VecBlendvI32x8,
                None,
                vec![
                    Operand::Const(IrConst::I64(3)),
                    Operand::Const(IrConst::I64(4)),
                    Operand::Value(Value(1)),
                ],
            ),
        ],
        Terminator::Return(Some(Operand::Value(Value(2)))),
        false,
    ));
    f.next_value_id = 3;
    assert_eq!(
        run_function(&mut f),
        0,
        "128-bit zero satisfied a 256-bit mask"
    );

    // Matching 256-bit zero still folds.
    let mut g = IrFunction::new("zw_ok".into(), IrType::I32, vec![], false);
    g.blocks.push(block(
        vec![
            intrinsic(Some(Value(1)), IntrinsicOp::VecZeroI32x8, None, vec![]),
            intrinsic(
                Some(Value(2)),
                IntrinsicOp::VecBlendvI32x8,
                None,
                vec![
                    Operand::Const(IrConst::I64(3)),
                    Operand::Const(IrConst::I64(4)),
                    Operand::Value(Value(1)),
                ],
            ),
        ],
        Terminator::Return(Some(Operand::Value(Value(2)))),
        false,
    ));
    g.next_value_id = 3;
    assert_eq!(run_function(&mut g), 1);
    assert_copy(
        &g.blocks[0].instructions[1],
        Value(2),
        Operand::Const(IrConst::I64(3)),
        "matching zero",
    );
}

/// Rebuild the canonical image with a different whole-register store op.
fn image_with_store(
    op: IntrinsicOp,
    result_op: IntrinsicOp,
    source_bytes: &[u8; 16],
) -> IrFunction {
    let mut f = IrFunction::new("store_shape".into(), IrType::I32, vec![], false);
    let mut insts = vec![
        alloca(Value(0), 16, false, false),
        alloca(Value(1), 16, false, false),
    ];
    for (i, byte) in source_bytes.iter().enumerate() {
        let p = Value(10 + i as u32);
        insts.push(gep(p, Value(0), Operand::Const(IrConst::I64(i as i64))));
        insts.push(byte_store(p, *byte, false));
    }
    insts.push(intrinsic(
        Some(Value(50)),
        IntrinsicOp::VecLoadI8x16,
        None,
        vec![Operand::Value(Value(0)), Operand::Const(IrConst::I64(0))],
    ));
    insts.push(intrinsic(
        None,
        op,
        Some(Value(1)),
        vec![
            Operand::Value(Value(50)),
            Operand::Value(Value(1)),
            Operand::Const(IrConst::I64(0)),
        ],
    ));
    insts.push(intrinsic(
        Some(Value(51)),
        result_op,
        None,
        vec![Operand::Value(Value(1))],
    ));
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Value(Value(51)))),
        false,
    ));
    f.next_value_id = 52;
    f
}

#[test]
fn half_wide_pair_store_is_not_a_whole_register_image() {
    // Positive control: a genuine 16-byte whole-register store proves the
    // image, so the harness is non-vacuous.
    let mut ok = image_with_store(
        IntrinsicOp::VecStoreI64x2,
        IntrinsicOp::Pmovmskb128,
        &[0x80u8; 16],
    );
    assert_eq!(run_function(&mut ok), 1, "positive control did not fold");
    assert_eq!(result_const(&ok, Value(51)), Some(0xFFFF));

    // `VecStoreI32x4Pair` writes only the LOW EIGHT bytes of a 16-byte XMM
    // source.  Treating its args[0] as an 8-byte vector image is not a proof,
    // so the object must be refused outright.
    let mut bad = image_with_store(
        IntrinsicOp::VecStoreI32x4Pair,
        IntrinsicOp::Pmovmskb128,
        &[0x80u8; 16],
    );
    assert_eq!(
        run_function(&mut bad),
        0,
        "half-wide pair store was trusted"
    );
    assert!(
        bad.blocks[0].instructions.iter().any(|i| {
            matches!(i, Instruction::Intrinsic { op, .. } if *op == IntrinsicOp::Pmovmskb128)
        }),
        "the movemask must survive",
    );
}

#[test]
fn scalar_loads_and_arithmetic_mentions_veto_the_image() {
    // A NON-VOLATILE scalar Load is a pure read: no IrConst can encode a
    // stack address, so a value loaded from a constant object cannot be
    // turned back into an aliasing pointer.  It must NOT veto the image.
    let mut im = Image::new(&[0x80u8; 16]);
    let p = im.v();
    im.insts.insert(34, load(p, im.source));
    let mut f = im.function(false);
    assert_eq!(
        movemask_result_after(&mut f, im.result),
        Some(0xFFFF),
        "a pure scalar read must not veto"
    );

    // A VOLATILE scalar Load is an observable read: refuse.
    let mut im = Image::new(&[0x80u8; 16]);
    let p = im.v();
    im.insts.insert(34, {
        let mut l = load(p, im.source);
        if let Instruction::Load { volatile, .. } = &mut l {
            *volatile = true;
        }
        l
    });
    let mut f = im.function(false);
    assert_eq!(run_function(&mut f), 0, "a volatile scalar read must veto");

    // A read whose window runs past the end of the object must refuse.
    let mut im = Image::new(&[0x80u8; 16]);
    let p = im.v();
    let gp = im.v();
    im.insts
        .insert(34, gep(gp, im.source, Operand::Const(IrConst::I64(15))));
    im.insts.insert(35, {
        let mut l = load(p, gp);
        if let Instruction::Load { ty, .. } = &mut l {
            *ty = IrType::I64;
        }
        l
    });
    let mut f = im.function(false);
    assert_eq!(
        run_function(&mut f),
        0,
        "an out-of-bounds scalar read must veto"
    );

    // A Cast of the address loses provenance (it could become an integer
    // that aliases the object).  Refuse.
    let mut im = Image::new(&[0x80u8; 16]);
    let p = im.v();
    im.insts.insert(34, cast(p, Operand::Value(im.source)));
    let mut f = im.function(false);
    assert_eq!(run_function(&mut f), 0, "cast mention folded");

    // Arithmetic on the address likewise.
    let mut im = Image::new(&[0x80u8; 16]);
    let p = im.v();
    im.insts.insert(34, unary_not(p, Operand::Value(im.source)));
    let mut f = im.function(false);
    assert_eq!(run_function(&mut f), 0, "arithmetic mention folded");
}

#[test]
fn fold_reclaims_the_entire_private_constant_image() {
    // The measured x86-64 repro: GCC 16.2 / Clang 23.1.5 emit exactly
    // `movl $53069, %eax; ret`.  The fold alone leaves 16 movb + movdqu
    // behind; the reclaimer must remove them too.
    let bytes = [
        0xFF, 0x00, 0xFF, 0xFF, 0x00, 0x00, 0xFF, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0xFF,
        0xFF,
    ];
    let image = Image::new(&bytes);
    let result = image.result;
    let mut func = image.function(true);
    assert_eq!(run_function(&mut func), 1);
    assert_eq!(result_const(&func, result), Some(53069));

    let insts = &func.blocks[0].instructions;
    assert!(
        !insts.iter().any(|i| matches!(i, Instruction::Store { .. })),
        "a scalar initialization store survived: {insts:?}"
    );
    assert!(
        !insts
            .iter()
            .any(|i| matches!(i, Instruction::GetElementPtr { .. })),
        "an orphaned address derivation survived: {insts:?}"
    );
    assert!(
        !insts.iter().any(|i| matches!(
            i,
            Instruction::Intrinsic { op, .. }
                if whole_store_width(*op).is_some() || whole_vector_load(*op)
        )),
        "a dead vector load/store survived: {insts:?}"
    );
    assert_eq!(
        func.blocks[0].source_spans.len(),
        insts.len(),
        "span bookkeeping must stay parallel across the reclaimer and DCE"
    );
    assert_eq!(run_function(&mut func), 0, "the whole pass is idempotent");
}

/// Directly audit one round of the dead-write decision without needing a
/// successful fold, so every veto can be exercised non-vacuously.
fn dead_writes_for(func: &IrFunction) -> Vec<(usize, usize)> {
    let facts = Facts::new(func);
    dead_private_writes(&facts)
}

fn write_only_object(extra: Vec<Instruction>, terminator: Terminator) -> IrFunction {
    let mut f = IrFunction::new("dw".into(), IrType::I32, vec![], false);
    let mut insts = vec![alloca(Value(0), 16, false, false)];
    let p = Value(20);
    insts.push(gep(p, Value(0), Operand::Const(IrConst::I64(0))));
    insts.push(byte_store(p, 0x80, false));
    insts.extend(extra);
    f.blocks.push(block(insts, terminator, false));
    f.next_value_id = 30;
    f
}

#[test]
fn dead_write_round_accepts_only_unobserved_private_writes() {
    // Positive: a lone constant store into a private object with no reader.
    let f = write_only_object(vec![], Terminator::Return(None));
    let dead = dead_writes_for(&f);
    assert_eq!(dead, vec![(0, 2)], "the lone write must be reclaimable");

    // A scalar Load is a reader: veto, even though its result is unused.
    let f = write_only_object(vec![load(Value(21), Value(0))], Terminator::Return(None));
    assert!(dead_writes_for(&f).is_empty(), "a scalar load must veto");

    // A returned address escapes: veto.
    let f = write_only_object(vec![], Terminator::Return(Some(Operand::Value(Value(0)))));
    assert!(
        dead_writes_for(&f).is_empty(),
        "a terminator escape must veto"
    );

    // A Cast of the address loses provenance: veto.
    let f = write_only_object(
        vec![cast(Value(21), Operand::Value(Value(0)))],
        Terminator::Return(None),
    );
    assert!(dead_writes_for(&f).is_empty(), "a cast must veto");

    // Arithmetic on the address: veto.
    let f = write_only_object(
        vec![unary_not(Value(21), Operand::Value(Value(0)))],
        Terminator::Return(None),
    );
    assert!(dead_writes_for(&f).is_empty(), "arithmetic must veto");

    // Volatile stores are observable writes, not reclaimable ones.
    let mut f = write_only_object(vec![], Terminator::Return(None));
    if let Instruction::Store { volatile, .. } = &mut f.blocks[0].instructions[2] {
        *volatile = true;
    }
    assert!(dead_writes_for(&f).is_empty(), "a volatile store must veto");
}

#[test]
fn deep_gep_chains_stay_attributed_to_their_root() {
    // `resolve_pointer` stops at MAX_DEPTH; `base_of_value` must NOT, or a
    // deep derivation could hide a reader from the non-escape audit.
    let mut f = IrFunction::new("deep".into(), IrType::I32, vec![], false);
    let mut insts = vec![alloca(Value(0), 16, false, false)];
    let mut cur = Value(0);
    for i in 0..40u32 {
        let next = Value(100 + i);
        insts.push(gep(next, cur, Operand::Const(IrConst::I64(0))));
        cur = next;
    }
    insts.push(byte_store(cur, 0x80, false));
    f.blocks
        .push(block(insts.clone(), Terminator::Return(None), false));
    f.next_value_id = 200;
    let facts = Facts::new(&f);
    let mut memo = FxHashMap::default();
    let mut cycle = FxHashSet::default();
    assert_eq!(
        facts.base_of_value(cur.0, &mut memo, &mut cycle),
        Some(0),
        "a 40-link derivation lost its root"
    );
    assert!(
        !dead_writes_for(&f).is_empty(),
        "deep write must be reclaimable"
    );

    // Add a Load through the deepest address: the root must be vetoed even
    // though `resolve_pointer` would have given up at link 16.
    let mut g = IrFunction::new("deep_read".into(), IrType::I32, vec![], false);
    insts.push(load(Value(199), cur));
    g.blocks.push(block(insts, Terminator::Return(None), false));
    g.next_value_id = 200;
    assert!(
        dead_writes_for(&g).is_empty(),
        "a reader 40 links away escaped the audit"
    );
}

#[test]
fn storing_the_address_into_another_object_is_an_escape() {
    // `Store { val: A, ptr: B }` writes A's ADDRESS into memory.  It mentions
    // A but is not a write TO A, so A must be vetoed.
    let mut f = IrFunction::new("addr_store".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        vec![
            alloca(Value(0), 16, false, false),
            alloca(Value(1), 16, false, false),
            byte_store(Value(0), 0x80, false),
            Instruction::Store {
                val: Operand::Value(Value(0)),
                ptr: Value(1),
                ty: IrType::Ptr,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
        ],
        Terminator::Return(None),
        false,
    ));
    f.next_value_id = 2;
    let dead = dead_writes_for(&f);
    assert!(
        !dead.contains(&(0, 2)),
        "A's own store was reclaimed despite the address escaping: {dead:?}"
    );
}

// ---------------------------------------------------------------------------
// Third audit round: shapes the real C front end emits but the first lattice
// port refused.  Found by differential probing against GCC 16.2 / Clang
// 23.1.5 with LCCC_DUMP_IR, then proved here at IR level.
// ---------------------------------------------------------------------------

fn typed_store(ptr: Value, c: IrConst, ty: IrType) -> Instruction {
    Instruction::Store {
        val: Operand::Const(c),
        ptr,
        ty,
        seg_override: AddressSpace::Default,
        volatile: false,
    }
}

#[test]
fn narrow_constant_stores_are_truncated_exactly() {
    // `Store { val: I64(255), ty: U8 }` writes ONE byte 0xFF, not eight.
    assert_eq!(
        const_store_bytes(IrConst::I64(255), 1),
        Some(bi(vec![0xFF]))
    );
    assert_eq!(const_store_bytes(IrConst::I64(-1), 1), Some(bi(vec![0xFF])));
    assert_eq!(const_store_bytes(IrConst::I64(0), 1), Some(bi(vec![0])));
    assert_eq!(
        const_store_bytes(IrConst::I32(-1), 2),
        Some(bi(vec![0xFF, 0xFF]))
    );
    assert_eq!(const_store_bytes(IrConst::Zero, 4), Some(bi(vec![0; 4])));
    // Exact-width float and integer constants are unchanged.
    assert_eq!(
        const_store_bytes(IrConst::F32(-0.0), 4),
        Some(bi(vec![0, 0, 0, 0x80]))
    );
    // A mixed tail has no single value at the narrowed width: refuse.
    assert_eq!(const_store_bytes(IrConst::I64(0x1FF), 1), None);
    assert_eq!(const_store_bytes(IrConst::I64(0x1_00_00), 1), None);
    // Widening and a float in a narrow integer slot are not proofs.
    assert_eq!(const_store_bytes(IrConst::I8(1), 4), None);
    assert_eq!(const_store_bytes(IrConst::F64(-0.0), 1), None);
    assert_eq!(const_store_bytes(IrConst::I64(1), 0), None);
}

/// The exact 32-lane shape LCCC lowers `_mm256_setr_epi8` into: a 32-byte
/// source object tiled by `Store { val: I64(..), ty: U8 }`, a `VecLoadI64x4`
/// and a `VecStoreI64x4` into the movemask's private slot.
fn narrow_256_image(bytes: &[u8; 32]) -> IrFunction {
    let mut f = IrFunction::new("narrow256".into(), IrType::I32, vec![], false);
    let mut insts = vec![
        alloca(Value(0), 32, false, false),
        alloca(Value(1), 32, false, false),
    ];
    for (i, byte) in bytes.iter().enumerate() {
        let p = Value(10 + i as u32);
        insts.push(gep(p, Value(0), Operand::Const(IrConst::I64(i as i64))));
        insts.push(typed_store(p, IrConst::I64(*byte as i64), IrType::U8));
    }
    insts.push(intrinsic(
        Some(Value(60)),
        IntrinsicOp::VecLoadI64x4,
        None,
        vec![Operand::Value(Value(0)), Operand::Const(IrConst::I64(0))],
    ));
    insts.push(intrinsic(
        None,
        IntrinsicOp::VecStoreI64x4,
        Some(Value(1)),
        vec![
            Operand::Value(Value(60)),
            Operand::Value(Value(1)),
            Operand::Const(IrConst::I64(0)),
        ],
    ));
    insts.push(intrinsic(
        Some(Value(61)),
        IntrinsicOp::Pmovmskb256,
        None,
        vec![Operand::Value(Value(1))],
    ));
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Value(Value(61)))),
        false,
    ));
    f.next_value_id = 62;
    f
}

#[test]
fn narrow_stores_prove_a_256_bit_movemask_image() {
    let mut bytes = [0u8; 32];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = if i % 2 == 0 { 255 } else { 0 };
    }
    let mut f = narrow_256_image(&bytes);
    assert_eq!(run_function(&mut f), 1);
    assert_eq!(result_const(&f, Value(61)), Some(0x5555_5555u32 as i32));
    assert!(
        !f.blocks[0]
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Store { .. })),
        "the 32 narrow stores must be reclaimed"
    );

    // All-on: every one of the 32 sign bits set -> u32::MAX as i32 == -1.
    let mut g = narrow_256_image(&[255u8; 32]);
    assert_eq!(run_function(&mut g), 1);
    assert_eq!(result_const(&g, Value(61)), Some(-1));

    // A value that cannot be narrowed unambiguously must refuse the whole
    // image, not silently truncate a different byte.
    let mut bad = narrow_256_image(&bytes);
    if let Instruction::Store { val, .. } = &mut bad.blocks[0].instructions[3] {
        *val = Operand::Const(IrConst::I64(0x1FF));
    }
    assert_eq!(run_function(&mut bad), 0, "ambiguous truncation folded");
}

/// `_mm_movemask_ps` shape: F32 constant stores -> VecLoadI64x2 ->
/// VecStoreF32x4 -> Memcpy into the movemask's own slot -> MovemaskPs128.
fn memcpy_ps_image(lane_signs: [bool; 4], mutate: Option<fn(&mut Vec<Instruction>)>) -> IrFunction {
    let mut insts = vec![
        alloca(Value(0), 16, false, false),
        alloca(Value(1), 16, false, false),
        alloca(Value(2), 16, false, false),
    ];
    for (i, sign) in lane_signs.iter().enumerate() {
        let c = if *sign {
            IrConst::F32(-0.0)
        } else {
            IrConst::F32(1.0)
        };
        if i == 0 {
            insts.push(typed_store(Value(0), c, IrType::F32));
        } else {
            let p = Value(10 + i as u32);
            insts.push(gep(
                p,
                Value(0),
                Operand::Const(IrConst::I64((i * 4) as i64)),
            ));
            insts.push(typed_store(p, c, IrType::F32));
        }
    }
    insts.push(intrinsic(
        Some(Value(20)),
        IntrinsicOp::VecLoadI64x2,
        None,
        vec![Operand::Value(Value(0)), Operand::Const(IrConst::I64(0))],
    ));
    insts.push(intrinsic(
        None,
        IntrinsicOp::VecStoreF32x4,
        Some(Value(1)),
        vec![
            Operand::Value(Value(20)),
            Operand::Value(Value(1)),
            Operand::Const(IrConst::I64(0)),
        ],
    ));
    insts.push(Instruction::Memcpy {
        dest: Value(2),
        src: Value(1),
        size: 16,
    });
    insts.push(intrinsic(
        Some(Value(21)),
        IntrinsicOp::MovemaskPs128,
        None,
        vec![Operand::Value(Value(2))],
    ));
    if let Some(m) = mutate {
        m(&mut insts);
    }
    let mut f = IrFunction::new("memcpy_ps".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Value(Value(21)))),
        true,
    ));
    f.next_value_id = 22;
    f
}

#[test]
fn memcpy_backed_constant_images_are_proven_not_guessed() {
    // Sign bits on lanes 0 and 2 -> 0b0101 == 5, the GCC/Clang oracle.
    let mut f = memcpy_ps_image([true, false, true, false], None);
    assert_eq!(run_function(&mut f), 1);
    assert_eq!(result_const(&f, Value(21)), Some(0b0101));
    assert_eq!(
        f.blocks[0].source_spans.len(),
        f.blocks[0].instructions.len(),
        "spans must stay parallel through the Memcpy fixpoint"
    );
    assert!(
        !f.blocks[0]
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Memcpy { .. })),
        "the dead Memcpy must be reclaimed"
    );
    assert!(
        !f.blocks[0]
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Store { .. })),
        "the source F32 stores must be reclaimed in a later fixpoint round"
    );
    assert_eq!(run_function(&mut f), 0, "idempotent");

    // All four all-on / all-off permutations.
    let mut on = memcpy_ps_image([true, true, true, true], None);
    assert_eq!(run_function(&mut on), 1);
    assert_eq!(result_const(&on, Value(21)), Some(0b1111));
    let mut off = memcpy_ps_image([false, false, false, false], None);
    assert_eq!(run_function(&mut off), 1);
    assert_eq!(result_const(&off, Value(21)), Some(0));
}

#[test]
fn memcpy_refuses_self_copies_unprovable_sources_and_out_of_bounds_reads() {
    fn refuse(name: &str, mutate: fn(&mut Vec<Instruction>)) {
        let mut f = memcpy_ps_image([true, false, true, false], Some(mutate));
        assert_eq!(
            run_function(&mut f),
            0,
            "negative Memcpy case {name} folded"
        );
        assert!(
            f.blocks[0].instructions.iter().any(|i| matches!(
                i,
                Instruction::Intrinsic { op, .. } if *op == IntrinsicOp::MovemaskPs128
            )),
            "negative Memcpy case {name} rewrote the intrinsic"
        );
    }

    refuse("self-copy", |insts| {
        // Keep the movemask reading the Memcpy's destination so the
        // `s_base == d_base == base` branch is actually exercised (rather
        // than merely leaving the destination object with no writes).
        if let Some(Instruction::Memcpy { dest, src, .. }) = insts
            .iter_mut()
            .find(|i| matches!(i, Instruction::Memcpy { .. }))
        {
            *dest = Value(2);
            *src = Value(2);
        }
    });
    refuse("unprovable source", |insts| {
        if let Some(Instruction::Memcpy { src, .. }) = insts
            .iter_mut()
            .find(|i| matches!(i, Instruction::Memcpy { .. }))
        {
            *src = Value(999);
        }
    });
    refuse("zero length", |insts| {
        if let Some(Instruction::Memcpy { size, .. }) = insts
            .iter_mut()
            .find(|i| matches!(i, Instruction::Memcpy { .. }))
        {
            *size = 0;
        }
    });
    refuse("oversized copy", |insts| {
        if let Some(Instruction::Memcpy { size, .. }) = insts
            .iter_mut()
            .find(|i| matches!(i, Instruction::Memcpy { .. }))
        {
            *size = 33;
        }
    });
    // A read past the end of the SOURCE object is refused while proving the
    // source image through a second Memcpy.
    refuse("out-of-bounds source read", |insts| {
        if let Some(Instruction::Memcpy { size, .. }) = insts
            .iter_mut()
            .find(|i| matches!(i, Instruction::Memcpy { .. }))
        {
            *size = 17;
        }
    });
}

#[test]
fn dead_write_round_treats_memcpy_as_a_write_and_a_read() {
    // dest=A, src=B: the copy is reclaimable for A, but B must be vetoed
    // (it is read).  This is the two-sided verdict the generic classifier
    // cannot express.
    let mut f = IrFunction::new("dw_memcpy".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        vec![
            alloca(Value(0), 16, false, false),
            alloca(Value(1), 16, false, false),
            typed_store(Value(1), IrConst::I64(0x80), IrType::U8),
            Instruction::Memcpy {
                dest: Value(0),
                src: Value(1),
                size: 16,
            },
        ],
        Terminator::Return(None),
        false,
    ));
    f.next_value_id = 2;
    let dead = dead_writes_for(&f);
    assert!(
        dead.contains(&(0, 3)),
        "the Memcpy write must be reclaimable: {dead:?}"
    );
    assert!(
        !dead.contains(&(0, 2)),
        "the source object's store must stay until the Memcpy is gone: {dead:?}"
    );

    // A self-copy is refused outright.
    let mut g = IrFunction::new("dw_self".into(), IrType::I32, vec![], false);
    g.blocks.push(block(
        vec![
            alloca(Value(0), 16, false, false),
            typed_store(Value(0), IrConst::I64(0x80), IrType::U8),
            Instruction::Memcpy {
                dest: Value(0),
                src: Value(0),
                size: 16,
            },
        ],
        Terminator::Return(None),
        false,
    ));
    g.next_value_id = 1;
    assert!(
        dead_writes_for(&g).is_empty(),
        "a self-copy must veto the object"
    );
}

// ---------------------------------------------------------------------------
// Fourth audit round: the i686 front end lowers a constant vector through an
// `I128 Load -> I128 Store` aggregate copy, not scalar byte stores.  Found by
// `LCCC_DUMP_IR=1` on `lccc-i686`; without this path i686 emitted the full
// SIMD sequence while x86-64 folded.  Each case has a non-vacuous control.
// ---------------------------------------------------------------------------

fn i686_aggregate_image(bytes: &[u8; 16], mutate: Option<fn(&mut Vec<Instruction>)>) -> IrFunction {
    let lo = i64::from_le_bytes(bytes[0..8].try_into().unwrap());
    let hi = i64::from_le_bytes(bytes[8..16].try_into().unwrap());
    let mut insts = vec![
        alloca(Value(0), 16, false, false),
        alloca(Value(1), 16, false, false),
        alloca(Value(2), 16, false, false),
        typed_store(Value(2), IrConst::I64(lo), IrType::I64),
        gep(Value(10), Value(2), Operand::Const(IrConst::I64(8))),
        typed_store(Value(10), IrConst::I64(hi), IrType::I64),
        Instruction::Load {
            dest: Value(11),
            ptr: Value(2),
            ty: IrType::I128,
            volatile: false,
            seg_override: AddressSpace::Default,
        },
        // The i686 front end inserts this Copy (measured `Copy v84 <- v214`);
        // the analysis must follow it, not demand a direct Load.
        copy(Value(14), Operand::Value(Value(11))),
        Instruction::Store {
            val: Operand::Value(Value(14)),
            ptr: Value(1),
            ty: IrType::I128,
            seg_override: AddressSpace::Default,
            volatile: false,
        },
        intrinsic(
            Some(Value(12)),
            IntrinsicOp::VecLoadI64x2,
            None,
            vec![Operand::Value(Value(1)), Operand::Const(IrConst::I64(0))],
        ),
        intrinsic(
            None,
            IntrinsicOp::VecStoreI64x2,
            Some(Value(0)),
            vec![
                Operand::Value(Value(12)),
                Operand::Value(Value(0)),
                Operand::Const(IrConst::I64(0)),
            ],
        ),
        intrinsic(
            Some(Value(13)),
            IntrinsicOp::Pmovmskb128,
            None,
            vec![Operand::Value(Value(0))],
        ),
    ];
    if let Some(m) = mutate {
        m(&mut insts);
    }
    let mut f = IrFunction::new("i686agg".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Value(Value(13)))),
        true,
    ));
    f.next_value_id = 14;
    f
}

#[test]
fn i686_i128_aggregate_copy_is_a_proven_constant_image() {
    let bytes = [
        0xFF, 0x00, 0xFF, 0xFF, 0x00, 0x00, 0xFF, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0xFF,
        0xFF,
    ];
    let mut f = i686_aggregate_image(&bytes, None);
    assert_eq!(movemask_result_after(&mut f, Value(13)), Some(53069));
    assert_eq!(
        f.blocks[0].source_spans.len(),
        f.blocks[0].instructions.len(),
        "spans must stay parallel across the aggregate-copy fixpoint"
    );
    assert!(
        !f.blocks[0]
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Load { .. } | Instruction::Store { .. })),
        "the whole I128 aggregate-copy chain must be reclaimed"
    );
    assert_eq!(run_function(&mut f), 0, "idempotent");

    // All-off / all-on boundaries through the same path.
    let mut off = i686_aggregate_image(&[0u8; 16], None);
    assert_eq!(movemask_result_after(&mut off, Value(13)), Some(0));
    let mut on = i686_aggregate_image(&[0x80u8; 16], None);
    assert_eq!(movemask_result_after(&mut on, Value(13)), Some(0xFFFF));
}

#[test]
fn i686_aggregate_copy_refuses_unproven_or_mismatched_loads() {
    fn refuse(name: &str, mutate: fn(&mut Vec<Instruction>)) {
        let bytes = [0x80u8; 16];
        let mut f = i686_aggregate_image(&bytes, Some(mutate));
        assert_eq!(
            movemask_result_after(&mut f, Value(13)),
            None,
            "negative I128 case {name} folded"
        );
        assert!(
            f.blocks[0].instructions.iter().any(|i| matches!(
                i,
                Instruction::Intrinsic { op, .. } if *op == IntrinsicOp::Pmovmskb128
            )),
            "negative I128 case {name} rewrote the intrinsic"
        );
    }

    refuse("volatile load", |insts| {
        for i in insts.iter_mut() {
            if let Instruction::Load { volatile, .. } = i {
                *volatile = true;
            }
        }
    });
    refuse("load width does not match the store", |insts| {
        for i in insts.iter_mut() {
            if let Instruction::Load { ty, .. } = i {
                *ty = IrType::I64;
            }
        }
    });
    refuse("source object has a non-constant store", |insts| {
        for i in insts.iter_mut() {
            if let Instruction::Store {
                ptr,
                val: Operand::Const(c),
                ..
            } = i
            {
                if *ptr == Value(10) {
                    *c = IrConst::I64(0);
                    // Replace the constant with an unknown SSA value.
                    *i = Instruction::Store {
                        val: Operand::Value(Value(999)),
                        ptr: Value(10),
                        ty: IrType::I64,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    };
                }
            }
        }
    });
    refuse("load is not from a proven alloca", |insts| {
        for i in insts.iter_mut() {
            if let Instruction::Load { ptr, .. } = i {
                *ptr = Value(999);
            }
        }
    });
    refuse("store value is not a Load at all", |insts| {
        for i in insts.iter_mut() {
            if let Instruction::Store {
                val: Operand::Value(v),
                ty: IrType::I128,
                ..
            } = i
            {
                *v = Value(999);
            }
        }
    });
}

#[test]
fn scalar_value_copy_chains_are_followed_and_cycles_refused() {
    let bytes = [0x80u8; 16];

    // Two Copy links still resolve to the Load (the i686 shape has one).
    let mut two = i686_aggregate_image(
        &bytes,
        Some(|insts| {
            insts.insert(8, copy(Value(15), Operand::Value(Value(14))));
            for i in insts.iter_mut() {
                if let Instruction::Store {
                    val: Operand::Value(v),
                    ty: IrType::I128,
                    ..
                } = i
                {
                    *v = Value(15);
                }
            }
        }),
    );
    assert_eq!(
        movemask_result_after(&mut two, Value(13)),
        Some(0xFFFF),
        "a two-link Copy chain must resolve"
    );

    // A Copy cycle cannot resolve to a Load: refuse.
    let mut cycle = i686_aggregate_image(
        &bytes,
        Some(|insts| {
            insts.insert(8, copy(Value(14), Operand::Value(Value(14))));
        }),
    );
    assert_eq!(
        movemask_result_after(&mut cycle, Value(13)),
        None,
        "a self Copy cycle must refuse"
    );

    // A Copy of a constant (not a Load) is not a proven object image.
    let mut konst = i686_aggregate_image(
        &bytes,
        Some(|insts| {
            for i in insts.iter_mut() {
                if let Instruction::Copy { dest, .. } = i {
                    if *dest == Value(14) {
                        *i = copy(Value(14), Operand::Const(IrConst::I128(-1)));
                    }
                }
            }
        }),
    );
    assert_eq!(
        movemask_result_after(&mut konst, Value(13)),
        None,
        "a constant Copy must refuse"
    );
}

// ---------------------------------------------------------------------------
// Vector constant propagation (ISPC `InstructionSimplify` transplant).
//
// `logic_image` reproduces the EXACT IR LCCC emits at the maskfacts hook for
// `_mm_movemask_epi8(_mm_and_si128(A, B))`: two private constant objects, a
// legacy intrinsic that writes through `dest_ptr`, a whole-register load of
// that result, a whole-register store into the movemask's temp, and the
// movemask itself.  Value IDs are pinned so a mutation can target one site.
// ---------------------------------------------------------------------------
const LG_A: u32 = 0;
const LG_B: u32 = 1;
const LG_D: u32 = 2;
const LG_M: u32 = 3;
const LG_SA: u32 = 4;
const LG_SB: u32 = 5;
const LG_RESULT: u32 = 54;

fn tiled_bytes(base: Value, bytes: &[u8; 16], start_id: u32) -> Vec<Instruction> {
    let mut out = Vec::with_capacity(32);
    for (i, byte) in bytes.iter().enumerate() {
        let p = Value(start_id + i as u32);
        out.push(gep(p, base, Operand::Const(IrConst::I64(i as i64))));
        out.push(byte_store(p, *byte, false));
    }
    out
}

fn logic_image(
    a: &[u8; 16],
    b: &[u8; 16],
    op: IntrinsicOp,
    mutate: Option<fn(&mut Vec<Instruction>)>,
) -> IrFunction {
    let mut insts = vec![
        alloca(Value(LG_A), 16, false, false),
        alloca(Value(LG_B), 16, false, false),
        alloca(Value(LG_D), 16, false, false),
        alloca(Value(LG_M), 16, false, false),
        alloca(Value(LG_SA), 16, false, false),
        alloca(Value(LG_SB), 16, false, false),
    ];
    insts.extend(tiled_bytes(Value(LG_SA), a, 10));
    insts.extend(tiled_bytes(Value(LG_SB), b, 30));
    insts.push(intrinsic(
        Some(Value(50)),
        IntrinsicOp::VecLoadI8x16,
        None,
        vec![
            Operand::Value(Value(LG_SA)),
            Operand::Const(IrConst::I64(0)),
        ],
    ));
    insts.push(intrinsic(
        None,
        IntrinsicOp::VecStoreI8x16,
        Some(Value(LG_A)),
        vec![
            Operand::Value(Value(50)),
            Operand::Value(Value(LG_A)),
            Operand::Const(IrConst::I64(0)),
        ],
    ));
    insts.push(intrinsic(
        Some(Value(51)),
        IntrinsicOp::VecLoadI8x16,
        None,
        vec![
            Operand::Value(Value(LG_SB)),
            Operand::Const(IrConst::I64(0)),
        ],
    ));
    insts.push(intrinsic(
        None,
        IntrinsicOp::VecStoreI8x16,
        Some(Value(LG_B)),
        vec![
            Operand::Value(Value(51)),
            Operand::Value(Value(LG_B)),
            Operand::Const(IrConst::I64(0)),
        ],
    ));
    insts.push(intrinsic(
        Some(Value(52)),
        op,
        Some(Value(LG_D)),
        vec![Operand::Value(Value(LG_A)), Operand::Value(Value(LG_B))],
    ));
    insts.push(intrinsic(
        Some(Value(53)),
        IntrinsicOp::VecLoadI8x16,
        None,
        vec![Operand::Value(Value(LG_D)), Operand::Const(IrConst::I64(0))],
    ));
    insts.push(intrinsic(
        None,
        IntrinsicOp::VecStoreI8x16,
        Some(Value(LG_M)),
        vec![
            Operand::Value(Value(53)),
            Operand::Value(Value(LG_M)),
            Operand::Const(IrConst::I64(0)),
        ],
    ));
    insts.push(intrinsic(
        Some(Value(LG_RESULT)),
        IntrinsicOp::Pmovmskb128,
        None,
        vec![Operand::Value(Value(LG_M))],
    ));
    if let Some(f) = mutate {
        f(&mut insts);
    }
    let mut f = IrFunction::new("logic".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Value(Value(LG_RESULT)))),
        true,
    ));
    f.next_value_id = 100;
    f
}

/// Independent oracle for the four byte-wise ops, derived from the SSE
/// instruction semantics rather than from the pass under test.
fn oracle_bits(a: &[u8; 16], b: &[u8; 16], op: IntrinsicOp) -> i32 {
    let mut bits = 0u32;
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        let byte = match op {
            IntrinsicOp::Pand128 => x & y,
            IntrinsicOp::Por128 => x | y,
            IntrinsicOp::Pxor128 => x ^ y,
            IntrinsicOp::Pcmpeqb128 => (x == y) as u8 * 0xFF,
            _ => unreachable!(),
        };
        if byte & 0x80 != 0 {
            bits |= 1u32 << i;
        }
    }
    bits as i32
}

#[test]
fn byte_logic_chains_fold_to_the_exact_movemask_constant() {
    // Deliberately non-trivial bytes (not just 0x80/0x00), so a wrong
    // byte function, a wrong lane width or a sign-bit slip is visible.
    let a = [
        0x81, 0x02, 0xFF, 0x7F, 0x00, 0xC3, 0x80, 0x55, 0xAA, 0x10, 0xFE, 0x01, 0x8F, 0x30, 0x00,
        0xFF,
    ];
    let b = [
        0x80, 0x80, 0x80, 0x80, 0x81, 0x41, 0x00, 0xD5, 0x0A, 0xF0, 0x7E, 0x81, 0x0F, 0x80, 0xFF,
        0xFF,
    ];
    for op in [
        IntrinsicOp::Pand128,
        IntrinsicOp::Por128,
        IntrinsicOp::Pxor128,
        IntrinsicOp::Pcmpeqb128,
    ] {
        let mut f = logic_image(&a, &b, op, None);
        let want = oracle_bits(&a, &b, op);
        assert_eq!(run_function(&mut f), 1, "{op:?}: fold must fire");
        assert_eq!(result_const(&f, Value(LG_RESULT)), Some(want), "{op:?}");
        // The dead-write reclaimer must also erase the whole computed image.
        let insts = &f.blocks[0].instructions;
        assert!(
            !insts.iter().any(|i| matches!(i, Instruction::Store { .. })),
            "{op:?}: scalar initialization store survived"
        );
        assert!(
            !insts.iter().any(|i| matches!(
                i,
                Instruction::Intrinsic { op: o, .. } if byte_logic_shape(*o).is_some()
            )),
            "{op:?}: dead legacy vector op survived"
        );
        assert_eq!(run_function(&mut f), 0, "{op:?}: pass must be idempotent");
    }
}

#[test]
fn byte_logic_refuses_every_unproven_shape() {
    let a = [0x80u8; 16];
    let b = [0xFFu8; 16];

    // Wrong arity (three operands / one operand).
    let mut three = logic_image(
        &a,
        &b,
        IntrinsicOp::Pand128,
        Some(|insts| {
            for i in insts.iter_mut() {
                if let Instruction::Intrinsic { op, args, .. } = i {
                    if *op == IntrinsicOp::Pand128 {
                        args.push(Operand::Value(Value(LG_A)));
                    }
                }
            }
        }),
    );
    assert_eq!(
        run_function(&mut three),
        0,
        "three-argument Pand128 must refuse"
    );

    // `dest_ptr: None` means no proven write to the result object.
    let mut no_dest = logic_image(
        &a,
        &b,
        IntrinsicOp::Pand128,
        Some(|insts| {
            for i in insts.iter_mut() {
                if let Instruction::Intrinsic { op, dest_ptr, .. } = i {
                    if *op == IntrinsicOp::Pand128 {
                        *dest_ptr = None;
                    }
                }
            }
        }),
    );
    assert_eq!(
        run_function(&mut no_dest),
        0,
        "Pand128 without dest_ptr must refuse"
    );

    // An unresolved source operand is not a provable image.
    let mut unknown = logic_image(
        &a,
        &b,
        IntrinsicOp::Pand128,
        Some(|insts| {
            for i in insts.iter_mut() {
                if let Instruction::Intrinsic { op, args, .. } = i {
                    if *op == IntrinsicOp::Pand128 {
                        args[1] = Operand::Value(Value(999));
                    }
                }
            }
        }),
    );
    assert_eq!(
        run_function(&mut unknown),
        0,
        "unresolved source must refuse"
    );

    // A source object whose image is not constant (a volatile store).
    let mut vol = logic_image(
        &a,
        &b,
        IntrinsicOp::Pand128,
        Some(|insts| {
            insts.push(byte_store(Value(LG_SA), 0x12, true));
        }),
    );
    assert_eq!(
        run_function(&mut vol),
        0,
        "volatile source store must refuse"
    );

    // A non-commutative compare is NOT in the table: unknown intrinsic.
    let mut gt = logic_image(&a, &b, IntrinsicOp::Pcmpgtb128, None);
    assert_eq!(
        run_function(&mut gt),
        0,
        "non-commutative compare must refuse"
    );

    // A width mismatch (256-bit op with a 16-byte consumer) must refuse.
    let mut wide = logic_image(&a, &b, IntrinsicOp::Pand256, None);
    assert_eq!(
        run_function(&mut wide),
        0,
        "256-bit write cannot prove a 16-byte image"
    );
}

/// `logic_image` with B supplied by a legacy `SetEpi*` splat instead of a
/// tiled scalar image.  Everything stays BEFORE the movemask so the
/// at-or-before-read proof is exercised honestly.
fn splat_logic_image(
    a: &[u8; 16],
    lane: Operand,
    splat: IntrinsicOp,
    op: IntrinsicOp,
) -> IrFunction {
    let mut insts = vec![
        alloca(Value(LG_A), 16, false, false),
        alloca(Value(LG_B), 16, false, false),
        alloca(Value(LG_D), 16, false, false),
        alloca(Value(LG_M), 16, false, false),
        alloca(Value(LG_SA), 16, false, false),
    ];
    insts.extend(tiled_bytes(Value(LG_SA), a, 10));
    insts.push(intrinsic(
        Some(Value(50)),
        IntrinsicOp::VecLoadI8x16,
        None,
        vec![
            Operand::Value(Value(LG_SA)),
            Operand::Const(IrConst::I64(0)),
        ],
    ));
    insts.push(intrinsic(
        None,
        IntrinsicOp::VecStoreI8x16,
        Some(Value(LG_A)),
        vec![
            Operand::Value(Value(50)),
            Operand::Value(Value(LG_A)),
            Operand::Const(IrConst::I64(0)),
        ],
    ));
    insts.push(intrinsic(
        Some(Value(60)),
        splat,
        Some(Value(LG_B)),
        vec![lane],
    ));
    insts.push(intrinsic(
        Some(Value(52)),
        op,
        Some(Value(LG_D)),
        vec![Operand::Value(Value(LG_A)), Operand::Value(Value(LG_B))],
    ));
    insts.push(intrinsic(
        Some(Value(53)),
        IntrinsicOp::VecLoadI8x16,
        None,
        vec![Operand::Value(Value(LG_D)), Operand::Const(IrConst::I64(0))],
    ));
    insts.push(intrinsic(
        None,
        IntrinsicOp::VecStoreI8x16,
        Some(Value(LG_M)),
        vec![
            Operand::Value(Value(53)),
            Operand::Value(Value(LG_M)),
            Operand::Const(IrConst::I64(0)),
        ],
    ));
    insts.push(intrinsic(
        Some(Value(LG_RESULT)),
        IntrinsicOp::Pmovmskb128,
        None,
        vec![Operand::Value(Value(LG_M))],
    ));
    let mut f = IrFunction::new("splat".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Value(Value(LG_RESULT)))),
        true,
    ));
    f.next_value_id = 100;
    f
}

#[test]
fn dest_ptr_splat_is_a_proven_image_and_refuses_bad_lanes() {
    let a = [
        0xFF, 0x00, 0xFF, 0xFF, 0x00, 0x00, 0xFF, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0xFF,
        0xFF,
    ];
    let mut f = splat_logic_image(
        &a,
        Operand::Const(IrConst::I8(-128)),
        IntrinsicOp::SetEpi8,
        IntrinsicOp::Pand128,
    );
    assert_eq!(run_function(&mut f), 1, "a proven dest_ptr splat must fold");
    assert_eq!(result_const(&f, Value(LG_RESULT)), Some(53069));

    // A widened lane (I64 constant into a 1-byte lane) is ambiguous: the
    // backend's truncation is not IR evidence, so it must refuse.
    let mut widened = splat_logic_image(
        &a,
        Operand::Const(IrConst::I64(-1)),
        IntrinsicOp::SetEpi8,
        IntrinsicOp::Pand128,
    );
    assert_eq!(
        run_function(&mut widened),
        0,
        "a widened splat lane must refuse"
    );

    // A splat whose operand is a runtime value is not a constant image.
    let mut runtime = splat_logic_image(
        &a,
        Operand::Value(Value(999)),
        IntrinsicOp::SetEpi8,
        IntrinsicOp::Pand128,
    );
    assert_eq!(run_function(&mut runtime), 0, "a runtime splat must refuse");

    // A 32-byte splat cannot tile a 16-byte object.
    let mut too_wide = splat_logic_image(
        &a,
        Operand::Const(IrConst::I8(-128)),
        IntrinsicOp::SetEpi8_256,
        IntrinsicOp::Pand128,
    );
    assert_eq!(
        run_function(&mut too_wide),
        0,
        "a 32-byte splat in a 16-byte object must refuse"
    );
}

#[test]
fn ssa_form_byte_logic_propagates_without_memory_images() {
    // Modern SSA intrinsics (`VecAndI32x4`) take vector VALUES, not
    // pointers.  No object image is needed at all: two pure constant
    // producers feed the op, and the movemask reads the SSA result.
    let mut insts = vec![
        intrinsic(Some(Value(1)), IntrinsicOp::VecZeroI32x4, None, vec![]),
        intrinsic(
            Some(Value(2)),
            IntrinsicOp::VecBroadcastI32x4,
            None,
            vec![Operand::Const(IrConst::I32(i32::MIN))],
        ),
        intrinsic(
            Some(Value(3)),
            IntrinsicOp::VecAndI32x4,
            None,
            vec![Operand::Value(Value(1)), Operand::Value(Value(2))],
        ),
        intrinsic(
            Some(Value(4)),
            IntrinsicOp::Pmovmskb128,
            None,
            vec![Operand::Value(Value(3))],
        ),
    ];
    let mut f = IrFunction::new("ssa".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        insts.clone(),
        Terminator::Return(Some(Operand::Value(Value(4)))),
        true,
    ));
    f.next_value_id = 10;
    // zeros AND 0x80000000 (little-endian: 00 00 00 80 per dword) = zero
    assert_eq!(run_function(&mut f), 1, "SSA VecAndI32x4 must fold");
    assert_eq!(result_const(&f, Value(4)), Some(0));

    // XOR of zeros with the sign-bit splat -> every byte lane's high bit
    // is set only in the last byte of each 4-byte group.
    insts[2] = intrinsic(
        Some(Value(3)),
        IntrinsicOp::VecXorI32x4,
        None,
        vec![Operand::Value(Value(1)), Operand::Value(Value(2))],
    );
    let mut g = IrFunction::new("ssa".into(), IrType::I32, vec![], false);
    g.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Value(Value(4)))),
        true,
    ));
    g.next_value_id = 10;
    assert_eq!(run_function(&mut g), 1, "SSA VecXorI32x4 must fold");
    assert_eq!(result_const(&g, Value(4)), Some(0x8888));
}

#[test]
fn a_rejected_proof_never_poisons_a_later_independent_proof() {
    // This is the invariant behind the ObjectMention refactor: when an early
    // proof refuses, its cycle marker MUST be popped.  Put one unprovable
    // movemask BEFORE the image's write (so `index > read_index` rejects it)
    // and one provable movemask after it, both reading the SAME object.  If
    // the first proof leaked `visiting`, the second would see
    // `visiting.contains(base)` and refuse too.
    let bytes = [
        0xFF, 0x00, 0xFF, 0xFF, 0x00, 0x00, 0xFF, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0xFF,
        0xFF,
    ];
    let mut image = Image::new(&bytes);
    let early = intrinsic(
        Some(Value(9000)),
        IntrinsicOp::Pmovmskb128,
        None,
        vec![Operand::Value(image.temp)],
    );
    image.insts.insert(1, early);
    let result = image.result;
    let mut f = IrFunction::new("poison".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        image.insts.clone(),
        Terminator::Return(Some(Operand::Value(result))),
        true,
    ));
    f.next_value_id = 9001;
    assert_eq!(
        run_function(&mut f),
        1,
        "a refused early proof must not veto the later valid proof"
    );
    assert_eq!(result_const(&f, result), Some(53069));
    assert_eq!(
        result_const(&f, Value(9000)),
        None,
        "the unprovable early movemask must not be folded"
    );
}

#[test]
fn a_live_legacy_intrinsic_result_is_never_reclaimed() {
    // `Pand128` carries BOTH `dest_ptr` and an SSA `dest`.  If that result
    // has a real user, deleting the intrinsic would leave an undefined
    // value.  Keep the user live through the terminator so DCE cannot remove
    // it between reclaimer rounds, then insist the intrinsic survives.
    let a = [0x80u8; 16];
    let b = [0xFFu8; 16];
    let mut f = logic_image(
        &a,
        &b,
        IntrinsicOp::Pand128,
        Some(|insts| {
            insts.push(copy(Value(70), Operand::Value(Value(52))));
        }),
    );
    f.blocks[0].terminator = Terminator::Return(Some(Operand::Value(Value(70))));
    assert_eq!(run_function(&mut f), 1, "the movemask fold is still sound");
    assert!(
        f.blocks[0].instructions.iter().any(|i| matches!(
            i,
            Instruction::Intrinsic { op, .. } if *op == IntrinsicOp::Pand128
        )),
        "the reclaimer deleted an intrinsic whose SSA result is still used"
    );
    assert!(
        f.blocks[0].instructions.iter().any(|i| matches!(
            i,
            Instruction::Copy {
                dest: Value(70),
                ..
            }
        )),
        "the live user of the intrinsic result disappeared"
    );
}

// ---------------------------------------------------------------------------
// Legacy `Blendv*128/256` intrinsics (the shape the C `_mm_blendv_ps/pd`
// family actually emits): `args = [mask, false_arm, true_arm]`, result
// written through `dest_ptr`.  Both the Memcpy rewrite and the object-image
// proof are exercised, plus the SSA-liveness guard and mixed-mask refusal.
// ---------------------------------------------------------------------------
const LB_F: u32 = 201;
const LB_T: u32 = 202;
const LB_D: u32 = 203;
const LB_R: u32 = 204;
const LB_RESULT: u32 = 250;

fn legacy_blend_image(
    mask: Operand,
    false_bytes: &[u8; 16],
    true_bytes: &[u8; 16],
    live_dest: bool,
) -> IrFunction {
    let mut insts = vec![
        alloca(Value(LB_F), 16, false, false),
        alloca(Value(LB_T), 16, false, false),
        alloca(Value(LB_D), 16, false, false),
        alloca(Value(LB_R), 16, false, false),
    ];
    insts.extend(tiled_bytes(Value(LB_F), false_bytes, 210));
    insts.extend(tiled_bytes(Value(LB_T), true_bytes, 230));
    insts.push(intrinsic(
        Some(Value(90)),
        IntrinsicOp::BlendvPs128,
        Some(Value(LB_D)),
        vec![
            mask,
            Operand::Value(Value(LB_F)),
            Operand::Value(Value(LB_T)),
        ],
    ));
    insts.push(intrinsic(
        Some(Value(91)),
        IntrinsicOp::VecLoadI8x16,
        None,
        vec![Operand::Value(Value(LB_D)), Operand::Const(IrConst::I64(0))],
    ));
    insts.push(intrinsic(
        None,
        IntrinsicOp::VecStoreI8x16,
        Some(Value(LB_R)),
        vec![
            Operand::Value(Value(91)),
            Operand::Value(Value(LB_R)),
            Operand::Const(IrConst::I64(0)),
        ],
    ));
    insts.push(intrinsic(
        Some(Value(LB_RESULT)),
        IntrinsicOp::MovemaskPs128,
        None,
        vec![Operand::Value(Value(LB_R))],
    ));
    if live_dest {
        insts.push(copy(Value(92), Operand::Value(Value(90))));
    }
    let ret = if live_dest {
        Value(92)
    } else {
        Value(LB_RESULT)
    };
    let mut f = IrFunction::new("legacy_blend".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Value(ret))),
        true,
    ));
    f.next_value_id = 300;
    f
}

fn dword_all_on_mask() -> Operand {
    Operand::Const(IrConst::I128(i128::from_le_bytes([
        0, 0, 0, 0x80, 0, 0, 0, 0x80, 0, 0, 0, 0x80, 0, 0, 0, 0x80,
    ])))
}

#[test]
fn legacy_blend_rewrites_to_memcpy_and_proves_its_image() {
    // True arm has sign bits in dword lanes 0 and 3 -> movmskps == 0b1001.
    let mut arm = [0u8; 16];
    arm[3] = 0x80;
    arm[15] = 0x80;
    let zeros = [0u8; 16];

    for (mask, want, label) in [
        (dword_all_on_mask(), 9, "all-on"),
        (Operand::Const(IrConst::I128(0)), 0, "all-off"),
    ] {
        let func = legacy_blend_image(mask, &zeros, &arm, false);
        let facts = Facts::new(&func);
        let (blend_index, blend_inst) = func.blocks[0]
            .instructions
            .iter()
            .enumerate()
            .find(|(_, i)| {
                matches!(i, Instruction::Intrinsic { op, .. } if *op == IntrinsicOp::BlendvPs128)
            })
            .expect("blend present");
        match fold(&facts, blend_inst, 0, blend_index).expect("blend must rewrite") {
            Instruction::Memcpy { size, .. } => assert_eq!(size, 16, "{label}: copy width"),
            other => panic!("{label}: expected Memcpy, got {other:?}"),
        }

        let mut f2 = legacy_blend_image(
            if want == 9 {
                dword_all_on_mask()
            } else {
                Operand::Const(IrConst::I128(0))
            },
            &zeros,
            &arm,
            false,
        );
        assert_eq!(run_function(&mut f2), 2, "{label}: rewrite + movemask fold");
        assert_eq!(result_const(&f2, Value(LB_RESULT)), Some(want), "{label}");
        assert!(
            !f2.blocks[0].instructions.iter().any(|i| matches!(
                i,
                Instruction::Intrinsic { op, .. } if *op == IntrinsicOp::BlendvPs128
            )),
            "{label}: the blendvps intrinsic survived"
        );
    }
}

#[test]
fn legacy_blend_refuses_mixed_masks_and_live_ssa_results() {
    let mut arm = [0u8; 16];
    arm[3] = 0x80;
    arm[15] = 0x80;
    let zeros = [0u8; 16];

    // Only dword 0 has its sign bit: Mixed -> neither arm is provable.
    let mixed = Operand::Const(IrConst::I128(i128::from_le_bytes([
        0, 0, 0, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ])));
    let mut f = legacy_blend_image(mixed, &zeros, &arm, false);
    assert_eq!(run_function(&mut f), 0, "a mixed selector must refuse");

    // The SSA `dest` of the blend is live: the rewrite must NOT delete or
    // replace it (that would leave an undefined value).  The movemask may
    // still fold through the object-image proof.
    let mut live = legacy_blend_image(dword_all_on_mask(), &zeros, &arm, true);
    let facts = Facts::new(&live);
    let (idx, inst) = live.blocks[0]
        .instructions
        .iter()
        .enumerate()
        .find(|(_, i)| {
            matches!(i, Instruction::Intrinsic { op, .. } if *op == IntrinsicOp::BlendvPs128)
        })
        .expect("blend present");
    assert!(
        fold(&facts, inst, 0, idx).is_none(),
        "a blend whose SSA result is used must not be rewritten"
    );
    // Exactly ONE edit is expected: the movemask fold.  The blend rewrite
    // is refused (asserted above), and no other candidate exists.  The
    // folded constant is then dead because the terminator returns the live
    // blend result instead, so DCE removes it -- assert the edit count, not
    // the surviving Copy.
    assert_eq!(run_function(&mut live), 1, "movemask fold is still sound");
    assert!(
        live.blocks[0].instructions.iter().any(|i| matches!(
            i,
            Instruction::Intrinsic { op, .. } if *op == IntrinsicOp::BlendvPs128
        )),
        "the live blend must survive the reclaimer"
    );
}

#[test]
fn legacy_blend_shapes_are_exact_and_pblendvb_is_excluded() {
    assert_eq!(legacy_blend_shape(IntrinsicOp::BlendvPs128), Some((16, 32)));
    assert_eq!(legacy_blend_shape(IntrinsicOp::BlendvPd128), Some((16, 64)));
    assert_eq!(legacy_blend_shape(IntrinsicOp::BlendvPs256), Some((32, 32)));
    assert_eq!(legacy_blend_shape(IntrinsicOp::BlendvPd256), Some((32, 64)));
    // Different argument order ([a, b, mask]) -> deliberately not included.
    assert_eq!(legacy_blend_shape(IntrinsicOp::Pblendvb128), None);
    assert_eq!(legacy_blend_shape(IntrinsicOp::Pand128), None);
}

/// Two movemasks sharing ONE constant source object, with the second
/// computed-vector op AFTER the first movemask in program order.  The second
/// op is a PROVEN pure read of the source, so it cannot change the image the
/// first movemask sees.  Before the write-order fix the first fold was
/// wrongly refused (`index > read_index`), so only one of the two folded.
#[test]
fn a_pure_read_after_the_movemask_does_not_invalidate_an_earlier_image() {
    let bytes = [
        0xFF, 0x00, 0xFF, 0xFF, 0x00, 0x00, 0xFF, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0xFF,
        0xFF,
    ];
    let src = Value(300);
    let d1 = Value(301);
    let d2 = Value(302);
    let mut insts = vec![
        alloca(src, 16, false, false),
        alloca(d1, 16, false, false),
        alloca(d2, 16, false, false),
    ];
    insts.extend(tiled_bytes(src, &bytes, 310));
    // First computed op + first movemask.
    insts.push(intrinsic(
        Some(Value(90)),
        IntrinsicOp::Pand128,
        Some(d1),
        vec![Operand::Value(src), Operand::Value(src)],
    ));
    insts.push(intrinsic(
        Some(Value(91)),
        IntrinsicOp::Pmovmskb128,
        None,
        vec![Operand::Value(d1)],
    ));
    // SECOND computed op reads the SAME source, AFTER the first movemask.
    insts.push(intrinsic(
        Some(Value(92)),
        IntrinsicOp::Pcmpeqb128,
        Some(d2),
        vec![Operand::Value(src), Operand::Value(src)],
    ));
    insts.push(intrinsic(
        Some(Value(93)),
        IntrinsicOp::Pmovmskb128,
        None,
        vec![Operand::Value(d2)],
    ));
    insts.push(Instruction::BinOp {
        dest: Value(94),
        op: crate::ir::reexports::IrBinOp::Add,
        lhs: Operand::Value(Value(91)),
        rhs: Operand::Value(Value(93)),
        ty: IrType::I32,
    });
    let mut f = IrFunction::new("two_masks".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Value(Value(94)))),
        true,
    ));
    f.next_value_id = 400;
    assert_eq!(run_function(&mut f), 2, "both movemasks must fold");
    // AND(x,x) == x -> 53069; CMPEQ(x,x) is all-ones -> 0xFFFF.
    assert_eq!(result_const(&f, Value(91)), Some(53069));
    assert_eq!(result_const(&f, Value(93)), Some(0xFFFF));
}

// ---------------------------------------------------------------------------
// Load-of-proven-constant (ISPC `ImproveMemoryOps`) + mention-index audit.
// Every positive is paired with a refusal mutation; the byte-exactness of
// the constant spelling is checked by round-tripping through
// `const_store_bytes`, the very routine the image proof already trusts.
// ---------------------------------------------------------------------------

fn folded_const(func: &IrFunction, dest: Value) -> Option<IrConst> {
    func.blocks[0].instructions.iter().find_map(|i| match i {
        Instruction::Copy {
            dest: d,
            src: Operand::Const(c),
        } if *d == dest => Some(*c),
        _ => None,
    })
}

fn scalar_load(
    dest: Value,
    ptr: Value,
    ty: IrType,
    volatile: bool,
    seg: AddressSpace,
) -> Instruction {
    Instruction::Load {
        dest,
        ptr,
        ty,
        seg_override: seg,
        volatile,
    }
}

fn load_image(mutate: Option<fn(&mut Vec<Instruction>)>) -> IrFunction {
    let bytes = [
        0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x80, 0x01, 0x02, 0x03, 0x04, 0xF8, 0xFF, 0xFF,
        0xFF,
    ];
    let mut insts = vec![alloca(Value(0), 16, false, false)];
    insts.extend(tiled_bytes(Value(0), &bytes, 100));
    insts.push(scalar_load(
        Value(1),
        Value(0),
        IrType::U32,
        false,
        AddressSpace::Default,
    ));
    insts.push(scalar_load(
        Value(2),
        Value(0),
        IrType::U8,
        false,
        AddressSpace::Default,
    ));
    insts.push(gep(Value(10), Value(0), Operand::Const(IrConst::I64(4))));
    insts.push(scalar_load(
        Value(3),
        Value(10),
        IrType::F32,
        false,
        AddressSpace::Default,
    ));
    insts.push(gep(Value(11), Value(0), Operand::Const(IrConst::I64(8))));
    insts.push(scalar_load(
        Value(4),
        Value(11),
        IrType::U64,
        false,
        AddressSpace::Default,
    ));
    // Keep ALL four load results live: `prune_dead_private_writes` runs DCE
    // after any successful fold, so an unused load is (correctly) removed and
    // would make `folded_const` return None for the wrong reason.
    for (i, rhs) in [(20u32, 2u32), (21, 3), (22, 4)] {
        let lhs = if i == 20 { Value(1) } else { Value(i - 1) };
        insts.push(Instruction::BinOp {
            dest: Value(i),
            op: crate::ir::reexports::IrBinOp::Add,
            lhs: Operand::Value(lhs),
            rhs: Operand::Value(Value(rhs)),
            ty: IrType::I32,
        });
    }
    if let Some(m) = mutate {
        m(&mut insts);
    }
    let mut f = IrFunction::new("loads".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Value(Value(22)))),
        false,
    ));
    f.next_value_id = 300;
    f
}

#[test]
fn const_from_bytes_is_bit_exact_for_every_supported_scalar_type() {
    let cases: &[(IrType, Vec<u8>)] = &[
        (IrType::I8, vec![0x80]),
        (IrType::U8, vec![0xFF]),
        (IrType::I16, vec![0x01, 0x80]),
        (IrType::U16, vec![0xFF, 0xFF]),
        (IrType::I32, vec![0xFF, 0xFF, 0xFF, 0x7F]),
        (IrType::U32, vec![0xFF, 0xFF, 0xFF, 0xFF]),
        (IrType::I64, vec![0; 7].into_iter().chain([0x80]).collect()),
        (IrType::U64, vec![0xFF; 8]),
        // -0.0 and a NaN with a NON-CANONICAL payload: bit preservation, not
        // numeric equality, is the property under test.
        (IrType::F32, vec![0x00, 0x00, 0x00, 0x80]),
        (IrType::F32, vec![0x01, 0x00, 0xC0, 0x7F]),
        (
            IrType::F64,
            vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80],
        ),
        (
            IrType::F64,
            vec![0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF8, 0x7F],
        ),
    ];
    for (ty, bytes) in cases {
        let c = const_from_bytes(*ty, bytes)
            .unwrap_or_else(|| panic!("{ty:?} must be constructible from {bytes:?}"));
        assert_eq!(
            const_store_bytes(c, ty.size()),
            Some(bi(bytes.clone())),
            "{ty:?}: round-trip changed the bit image"
        );
    }
    // Exhaustive unsigned byte domain: U8/U16/U32 are spelled as
    // zero-extended I64, so a sign-extending mistake shows up immediately.
    for b in 0u16..=255 {
        let c = const_from_bytes(IrType::U8, &[b as u8]).unwrap();
        assert_eq!(const_store_bytes(c, 1), Some(bi(vec![b as u8])));
    }
    for v in [0u32, 1, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFF] {
        let bytes = v.to_le_bytes().to_vec();
        let c = const_from_bytes(IrType::U32, &bytes).unwrap();
        assert_eq!(
            const_store_bytes(c, 4),
            Some(bi(bytes.clone())),
            "U32 {v:#x}"
        );
        assert_eq!(c.to_le_bytes(), IrConst::I64(v as i64).to_le_bytes());
    }
}

#[test]
fn const_from_bytes_refuses_unprovable_or_target_dependent_types() {
    assert!(const_from_bytes(IrType::Ptr, &[0u8; 8]).is_none());
    assert!(const_from_bytes(IrType::F128, &[0u8; 16]).is_none());
    // BID decimal containers are deliberately OUT of scope: no proven
    // `Copy { Const(D*) }` lowering for a load-folded value.
    assert!(const_from_bytes(IrType::D32, &[0u8; 4]).is_none());
    assert!(const_from_bytes(IrType::D64, &[0u8; 8]).is_none());
    assert!(const_from_bytes(IrType::Void, &[]).is_none());
    assert!(const_from_bytes(IrType::I32, &[0u8; 3]).is_none());
    assert!(const_from_bytes(IrType::I32, &[0u8; 17]).is_none());
    assert!(const_from_bytes(IrType::I32, &[]).is_none());
    // Wide loads are OUT of scope: a 128-bit constant propagated into an
    // InlineAsm "x" input is materialised as ZERO by the x86 backend
    // (measured CI regression sse34_encoding_emission, rc=3).
    assert!(const_from_bytes(IrType::I128, &[0u8; 16]).is_none());
    assert!(const_from_bytes(IrType::U128, &[0u8; 16]).is_none());
}

/// The exact CI-caught shape, at IR level: a proven constant object read by
/// a 128-bit `Load` that feeds an inline asm.  The wide load must NOT become
/// a constant, so the asm keeps its SSA operand.
#[test]
fn wide_loads_are_not_folded_and_inline_asm_functions_are_not_refolded() {
    let mut f = load_image(None);
    // Widen the U32 load to I128 to model the `sse34_encoding_emission` IR.
    for i in &mut f.blocks[0].instructions {
        if let Instruction::Load {
            dest: Value(1), ty, ..
        } = i
        {
            *ty = IrType::I128;
        }
    }
    run_function(&mut f);
    assert!(
        folded_const(&f, Value(1)).is_none(),
        "a 128-bit load must not become a Copy{{ Const }} that the backend mis-materialises"
    );
    assert!(
        f.blocks[0]
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Load { dest: Value(1), .. })),
        "the wide Load must survive"
    );
}

#[test]
fn load_of_a_proven_constant_object_folds_to_typed_constants() {
    let mut f = load_image(None);
    run_function(&mut f);
    // U32 is a zero-extended I64 spelling, NOT I32(-1).
    assert_eq!(
        folded_const(&f, Value(1)).map(|c| c.to_le_bytes()),
        Some(IrConst::I64(0xFFFF_FFFFu32 as i64).to_le_bytes()),
        "U32 load must preserve unsigned semantics"
    );
    assert_eq!(
        folded_const(&f, Value(2)).map(|c| c.to_le_bytes()),
        Some(IrConst::I64(255).to_le_bytes()),
        "U8 load"
    );
    // -0.0 must keep its sign bit; numeric equality would miss that.
    assert_eq!(
        folded_const(&f, Value(3)).map(|c| c.to_le_bytes()),
        Some(vec![0x00, 0x00, 0x00, 0x80]),
        "F32 -0.0 bit pattern"
    );
    assert_eq!(
        folded_const(&f, Value(4)).map(|c| c.to_le_bytes()),
        // bytes [8..16) are 01 02 03 04 F8 FF FF FF, so the full 64-bit
        // pattern (high bits set) is what must survive -- NOT a truncated
        // 0x04030201.  This is the zero/sign-extension boundary.
        Some(0xFFFF_FFF8_0403_0201u64.to_le_bytes().to_vec()),
        "U64 load"
    );
}

#[test]
fn load_fold_refuses_every_unproven_shape() {
    fn refuse(name: &str, m: fn(&mut Vec<Instruction>)) {
        let mut f = load_image(Some(m));
        run_function(&mut f);
        assert!(
            folded_const(&f, Value(1)).is_none(),
            "{name}: U32 load was folded from an unproven object"
        );
    }
    refuse("volatile load", |insts| {
        for i in insts.iter_mut() {
            if let Instruction::Load { volatile, .. } = i {
                *volatile = true;
            }
        }
    });
    refuse("non-default address space", |insts| {
        for i in insts.iter_mut() {
            if let Instruction::Load { seg_override, .. } = i {
                *seg_override = AddressSpace::SegFs;
            }
        }
    });
    refuse("runtime store into the object", |insts| {
        insts.push(Instruction::Store {
            val: Operand::Value(Value(999)),
            ptr: Value(0),
            ty: IrType::U8,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
    });
    refuse("volatile alloca", |insts| {
        for i in insts.iter_mut() {
            if let Instruction::Alloca { volatile, .. } = i {
                *volatile = true;
            }
        }
    });
    refuse("out-of-bounds GEP derivation", |insts| {
        insts.push(gep(Value(12), Value(0), Operand::Const(IrConst::I64(15))));
        insts.push(scalar_load(
            Value(13),
            Value(12),
            IrType::U64,
            false,
            AddressSpace::Default,
        ));
    });
    // The terminator case needs its own function (the helper fixes Return);
    // a stray `Copy` alone does NOT constitute an escape, so pretending it
    // does would be a vacuous test.
    let mut esc = load_image(None);
    esc.blocks[0]
        .instructions
        .push(copy(Value(14), Operand::Value(Value(0))));
    esc.blocks[0].terminator = Terminator::Return(Some(Operand::Value(Value(14))));
    run_function(&mut esc);
    assert!(
        folded_const(&esc, Value(1)).is_none(),
        "an escaping address must veto the load fold"
    );
}

#[test]
fn load_fold_proves_only_the_requested_window() {
    // `object_image` proves a WINDOW, not the whole object.  A gap at byte 9
    // is irrelevant to a 4-byte read at byte 0, so the fold must still fire.
    let mut outside = load_image(None);
    outside.blocks[0].instructions.retain(|i| {
        !matches!(
            i,
            Instruction::GetElementPtr {
                dest: Value(109),
                ..
            }
        ) && !matches!(
            i,
            Instruction::Store {
                ptr: Value(109),
                ..
            }
        )
    });
    run_function(&mut outside);
    assert!(folded_const(&outside, Value(1)).is_some());

    // A gap INSIDE the U32 window must refuse: the image is a value, not a
    // guess, and one unproven byte makes the whole read unproven.
    let mut inside = load_image(None);
    inside.blocks[0].instructions.retain(|i| {
        let d = match i {
            Instruction::GetElementPtr { dest, .. } => dest.0,
            Instruction::Store { ptr, .. } => ptr.0,
            _ => return true,
        };
        !(100..=103).contains(&d)
    });
    run_function(&mut inside);
    assert!(
        folded_const(&inside, Value(1)).is_none(),
        "an unproven byte inside the read window must veto the fold"
    );
}

/// THE soundness regression for the legacy-blend `Memcpy` rewrite: the
/// SELECTED ARM may be a live runtime value in an object SMALLER than 16
/// bytes.  Emitting an unchecked 16-byte copy would read past the object.
#[test]
fn legacy_blend_memcpy_refuses_an_out_of_bounds_selected_arm() {
    let all_on = Operand::Const(IrConst::I128(-1));
    let mut f = legacy_blend_image(all_on, &[0u8; 16], &[0x80u8; 16], false);
    for i in &mut f.blocks[0].instructions {
        if let Instruction::Alloca { dest, size, .. } = i {
            if *dest == Value(LB_T) {
                *size = 8;
            }
        }
    }
    run_function(&mut f);
    assert!(
        !f.blocks[0]
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Memcpy { size: 16, .. })),
        "a 16-byte Memcpy from an 8-byte object is a buffer over-read"
    );
}

#[test]
fn legacy_blend_memcpy_refuses_a_self_overlapping_copy() {
    let all_on = Operand::Const(IrConst::I128(-1));
    let mut f = legacy_blend_image(all_on, &[0u8; 16], &[0x80u8; 16], false);
    for i in &mut f.blocks[0].instructions {
        if let Instruction::Intrinsic {
            op: IntrinsicOp::BlendvPs128,
            dest_ptr,
            args,
            ..
        } = i
        {
            *dest_ptr = Some(Value(LB_T));
            args[1] = Operand::Value(Value(LB_F));
            args[2] = Operand::Value(Value(LB_T));
        }
    }
    run_function(&mut f);
    assert!(
        !f.blocks[0].instructions.iter().any(|i| matches!(
            i,
            Instruction::Memcpy {
                dest: Value(LB_T),
                src: Value(LB_T),
                ..
            }
        )),
        "memcpy with identical dest/src is not a value proof"
    );
}

/// The precomputed mention index must see a mention in a LATER BLOCK, not
/// just the read's block.  A write in block 1 invalidates a block-0 read.
#[test]
fn mention_index_sees_cross_block_writes() {
    let bytes = [0x80u8; 16];
    let mut f = load_image(None);
    // Give the function a second block that writes to the same object.
    let mut second: Vec<Instruction> = vec![Instruction::Store {
        val: Operand::Const(IrConst::I8(0)),
        ptr: Value(0),
        ty: IrType::I8,
        seg_override: AddressSpace::Default,
        volatile: false,
    }];
    second.push(Instruction::Alloca {
        dest: Value(400),
        ty: IrType::Ptr,
        size: 16,
        align: 1,
        volatile: false,
        semantic_volatile: false,
    });
    f.blocks.push(block(
        second,
        Terminator::Return(Some(Operand::Value(Value(1)))),
        false,
    ));
    let _ = bytes;
    run_function(&mut f);
    assert!(
        folded_const(&f, Value(1)).is_none(),
        "a write in a later block must be visible to the mention index"
    );
}

// ===========================================================================
// Round 5: algebraic mask identities, legacy zero producers and constant
// packed-integer compares.  Every positive rule has a paired mutation that
// MUST refuse; every oracle is computed independently of the pass.
// ===========================================================================

const ALG_A: u32 = 200;
const ALG_Z: u32 = 201;
const ALG_D: u32 = 202;
const ALG_RESULT: u32 = 203;
const ALG_VEC: u32 = 204;
const ALG_ZVEC: u32 = 205;

/// `movemask(op(dest_ptr = ALG_D, args))` with three private objects.  `ALG_A`
/// is deliberately left UNWRITTEN: it has no provable constant image, so any
/// fold can only come from an algebraic identity or an absorbing constant.
fn algebraic_function(
    op: IntrinsicOp,
    args: Vec<Operand>,
    prologue: Vec<Instruction>,
    a_volatile: bool,
    object_size: usize,
    movemask: IntrinsicOp,
) -> IrFunction {
    let mut insts = vec![
        alloca(Value(ALG_A), object_size, a_volatile, false),
        alloca(Value(ALG_Z), object_size, false, false),
        alloca(Value(ALG_D), object_size, false, false),
    ];
    insts.extend(prologue);
    insts.push(intrinsic(
        Some(Value(ALG_VEC)),
        op,
        Some(Value(ALG_D)),
        args,
    ));
    insts.push(intrinsic(
        Some(Value(ALG_RESULT)),
        movemask,
        None,
        vec![Operand::Value(Value(ALG_D))],
    ));
    let mut f = IrFunction::new("algebraic".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Value(Value(ALG_RESULT)))),
        true,
    ));
    f.next_value_id = 300;
    f
}

fn zero_producer(op: IntrinsicOp) -> Vec<Instruction> {
    vec![intrinsic(
        Some(Value(ALG_ZVEC)),
        op,
        Some(Value(ALG_Z)),
        vec![],
    )]
}

fn splat_producer(op: IntrinsicOp, byte: i8) -> Vec<Instruction> {
    vec![intrinsic(
        Some(Value(ALG_ZVEC)),
        op,
        Some(Value(ALG_Z)),
        vec![Operand::Const(IrConst::I8(byte))],
    )]
}

/// Independent oracle for `combine_byte_logic`'s element-wise rule.
fn oracle_byte(op: ByteOp, x: u8, y: u8) -> u8 {
    match op {
        ByteOp::And => x & y,
        ByteOp::Or => x | y,
        ByteOp::Xor => x ^ y,
        ByteOp::Eq => u8::from(x == y) * 0xFF,
        ByteOp::Avg => ((x as u16 + y as u16 + 1) / 2) as u8,
    }
}

fn pseudo_bytes(width: usize, seed: u32) -> Vec<u8> {
    (0..width)
        .map(|i| {
            let h = seed
                .wrapping_mul(0x9E37_79B9)
                .wrapping_add((i as u32).wrapping_mul(0x85EB_CA6B));
            (h >> 24) as u8
        })
        .collect()
}

#[test]
fn algebraic_lattice_is_exact_for_every_rule_and_every_byte_op() {
    let ops = [
        ByteOp::And,
        ByteOp::Or,
        ByteOp::Xor,
        ByteOp::Eq,
        ByteOp::Avg,
    ];
    // Width 64 is now provable too (one 512-bit register); the fixed-capacity
    // image type must stay exact at the widest supported width.
    for &width in &[16usize, 32usize, 64usize] {
        for &kind in &ops {
            let a = bi(pseudo_bytes(width, 0x1234_5678 ^ width as u32));
            let b = bi(pseudo_bytes(width, 0xDEAD_BEEF ^ width as u32));
            // Rule 1: both images known -> exact element-wise function.
            let expected = bi(a
                .iter()
                .copied()
                .zip(b.iter().copied())
                .map(|(x, y)| oracle_byte(kind, x, y))
                .collect());
            assert_eq!(
                combine_byte_logic(kind, width, Some(a), Some(b), false),
                Some(expected),
                "rule 1 (both known), {kind:?}, width {width}"
            );
            // `same` with both known must still be exact (no shortcut lies).
            let same_expected = bi(a
                .iter()
                .copied()
                .zip(a.iter().copied())
                .map(|(x, y)| oracle_byte(kind, x, y))
                .collect());
            assert_eq!(
                combine_byte_logic(kind, width, Some(a), Some(a), true),
                Some(same_expected),
                "rule 1 wins over rule 2, {kind:?}, width {width}"
            );
            // Rule 2: one PURE operand used twice, no image at all.
            let none = combine_byte_logic(kind, width, None, None, true);
            match kind {
                ByteOp::Xor => assert_eq!(none, ByteImage::zeros(width)),
                ByteOp::Eq => assert_eq!(none, ByteImage::ones(width)),
                ByteOp::And | ByteOp::Or | ByteOp::Avg => {
                    assert_eq!(none, None, "idempotent ops need one proven image")
                }
            }
            // `x&x` and `x|x` return the single proven image unchanged.
            let one = combine_byte_logic(kind, width, Some(a), None, true);
            match kind {
                ByteOp::Xor => assert_eq!(one, ByteImage::zeros(width)),
                ByteOp::Eq => assert_eq!(one, ByteImage::ones(width)),
                ByteOp::And | ByteOp::Or | ByteOp::Avg => assert_eq!(one, Some(a)),
            }
            // Rule 3: absorbing constants, one image known.
            let zeros = ByteImage::zeros(width).unwrap();
            let ones = ByteImage::ones(width).unwrap();
            let and_zero = combine_byte_logic(ByteOp::And, width, Some(zeros), None, false);
            let or_one = combine_byte_logic(ByteOp::Or, width, None, Some(ones), false);
            assert_eq!(and_zero, Some(zeros), "x&0 = 0");
            assert_eq!(or_one, Some(ones), "x|-1 = -1");
            // Non-absorbing combinations MUST refuse: an absorbing rule that
            // fires on the wrong byte value would be a miscompile.
            assert_eq!(
                combine_byte_logic(ByteOp::And, width, Some(ones), None, false),
                None,
                "x&-1 is x, not a constant"
            );
            assert_eq!(
                combine_byte_logic(ByteOp::Or, width, Some(zeros), None, false),
                None,
                "x|0 is x, not a constant"
            );
            assert_eq!(
                combine_byte_logic(ByteOp::Xor, width, Some(a), None, false),
                None,
                "x^a needs a"
            );
            assert_eq!(
                combine_byte_logic(ByteOp::Eq, width, Some(ones), None, false),
                None,
                "x==-1 needs x"
            );
            assert_eq!(
                combine_byte_logic(kind, width, None, None, false),
                None,
                "no evidence, no fold, {kind:?}"
            );
        }
    }
    // Width and length guards: no out-of-bounds indexing, no silent truncate.
    assert_eq!(combine_byte_logic(ByteOp::Xor, 0, None, None, true), None);
    assert_eq!(
        combine_byte_logic(ByteOp::Xor, MAX_IMAGE + 1, None, None, true),
        None
    );
    assert_eq!(
        combine_byte_logic(
            ByteOp::And,
            16,
            Some(bi(vec![0; 8])),
            Some(bi(vec![0; 8])),
            false
        ),
        None
    );
    assert_eq!(
        combine_byte_logic(ByteOp::And, 16, Some(bi(vec![0; 8])), None, false),
        None
    );
}

#[test]
fn self_identities_fold_runtime_vectors_without_any_constant_image() {
    // `x^x = 0` and `x==x = -1` need NO operand image.  Measured before this
    // change: LCCC kept a `pmovmskb` for each, GCC 16.2 / Clang 23.1.5 folded
    // all four to immediates.
    let cases: [(IntrinsicOp, usize, IntrinsicOp, i32); 4] = [
        (IntrinsicOp::Pxor128, 16, IntrinsicOp::Pmovmskb128, 0),
        (
            IntrinsicOp::Pcmpeqb128,
            16,
            IntrinsicOp::Pmovmskb128,
            0xFFFF,
        ),
        (IntrinsicOp::Pxor256, 32, IntrinsicOp::Pmovmskb256, 0),
        (
            IntrinsicOp::Pcmpeqb256,
            32,
            IntrinsicOp::Pmovmskb256,
            u32::MAX as i32,
        ),
    ];
    for (op, width, movemask, expected) in cases {
        let args = vec![Operand::Value(Value(ALG_A)), Operand::Value(Value(ALG_A))];
        let mut f = algebraic_function(op, args, vec![], false, width, movemask);
        assert_eq!(
            movemask_result_after(&mut f, Value(ALG_RESULT)),
            Some(expected),
            "{op:?} self-identity must fold to {expected}"
        );
        assert!(
            !f.blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Intrinsic { op: m, .. } if *m == movemask)),
            "{op:?} must remove the runtime movemask"
        );
    }
}

#[test]
fn self_identities_refuse_volatile_and_unproven_operands() {
    // A volatile object can change between the intrinsic's two reads, so
    // `x^x` is NOT provably zero.  This is the soundness guard, not a nicety.
    let mut vol = algebraic_function(
        IntrinsicOp::Pxor128,
        vec![Operand::Value(Value(ALG_A)), Operand::Value(Value(ALG_A))],
        vec![],
        true,
        16,
        IntrinsicOp::Pmovmskb128,
    );
    assert_eq!(
        movemask_result_after(&mut vol, Value(ALG_RESULT)),
        None,
        "volatile source must refuse the identity"
    );

    // An unresolved SSA value has no definition at all: neither an identity
    // nor an image proof can be built from it.
    let mut unresolved = algebraic_function(
        IntrinsicOp::Pxor128,
        vec![Operand::Value(Value(999)), Operand::Value(Value(999))],
        vec![],
        false,
        16,
        IntrinsicOp::Pmovmskb128,
    );
    assert_eq!(
        movemask_result_after(&mut unresolved, Value(ALG_RESULT)),
        None,
        "unresolved value must refuse the identity"
    );

    // Different operands are not an identity: XOR has no absorbing rule.
    let mut different = algebraic_function(
        IntrinsicOp::Pxor128,
        vec![Operand::Value(Value(ALG_A)), Operand::Value(Value(ALG_Z))],
        vec![],
        false,
        16,
        IntrinsicOp::Pmovmskb128,
    );
    assert_eq!(
        movemask_result_after(&mut different, Value(ALG_RESULT)),
        None,
        "x^y with no evidence must refuse"
    );
}

#[test]
fn absorbing_constants_fold_runtime_vectors_and_refuse_the_complement() {
    let positives: [(IntrinsicOp, Vec<Instruction>, i32); 2] = [
        (
            IntrinsicOp::Pand128,
            zero_producer(IntrinsicOp::Setzero128),
            0,
        ),
        (
            IntrinsicOp::Por128,
            splat_producer(IntrinsicOp::SetEpi8, -1),
            0xFFFF,
        ),
    ];
    for (op, prologue, expected) in positives {
        let args = vec![Operand::Value(Value(ALG_A)), Operand::Value(Value(ALG_Z))];
        let mut f = algebraic_function(op, args, prologue, false, 16, IntrinsicOp::Pmovmskb128);
        assert_eq!(
            movemask_result_after(&mut f, Value(ALG_RESULT)),
            Some(expected),
            "{op:?} absorbing constant must fold to {expected}"
        );
    }

    // The complementary constants are IDENTITY elements, not absorbing ones.
    let negatives: [(IntrinsicOp, Vec<Instruction>); 3] = [
        (
            IntrinsicOp::Pand128,
            splat_producer(IntrinsicOp::SetEpi8, -1),
        ),
        (IntrinsicOp::Por128, splat_producer(IntrinsicOp::SetEpi8, 0)),
        (IntrinsicOp::Pxor128, zero_producer(IntrinsicOp::Setzero128)),
    ];
    for (op, prologue) in negatives {
        let args = vec![Operand::Value(Value(ALG_A)), Operand::Value(Value(ALG_Z))];
        let mut f = algebraic_function(op, args, prologue, false, 16, IntrinsicOp::Pmovmskb128);
        assert_eq!(
            movemask_result_after(&mut f, Value(ALG_RESULT)),
            None,
            "{op:?} with an identity element must refuse"
        );
    }
}

#[test]
fn legacy_zero_producers_respect_width_and_object_bounds() {
    // `SetzeroPs128` is the FP spelling of the same 16-byte zero store.
    let mut ps = algebraic_function(
        IntrinsicOp::Pand128,
        vec![Operand::Value(Value(ALG_A)), Operand::Value(Value(ALG_Z))],
        zero_producer(IntrinsicOp::SetzeroPs128),
        false,
        16,
        IntrinsicOp::Pmovmskb128,
    );
    assert_eq!(movemask_result_after(&mut ps, Value(ALG_RESULT)), Some(0));

    // A 32-byte zero producer into a 16-byte object is a buffer overrun, not
    // a value proof.  `tile_image` must reject rather than truncate.
    let mut overrun = algebraic_function(
        IntrinsicOp::Pand128,
        vec![Operand::Value(Value(ALG_A)), Operand::Value(Value(ALG_Z))],
        zero_producer(IntrinsicOp::Setzero256),
        false,
        16,
        IntrinsicOp::Pmovmskb128,
    );
    assert_eq!(
        movemask_result_after(&mut overrun, Value(ALG_RESULT)),
        None,
        "32-byte write into a 16-byte object must refuse"
    );

    // A 16-byte zero producer leaves half of a 32-bit image unproven: the
    // exact-tiling gap must refuse (no silent zero fill).
    let mut gap = algebraic_function(
        IntrinsicOp::Pand256,
        vec![Operand::Value(Value(ALG_A)), Operand::Value(Value(ALG_Z))],
        zero_producer(IntrinsicOp::Setzero128),
        false,
        32,
        IntrinsicOp::Pmovmskb256,
    );
    assert_eq!(
        movemask_result_after(&mut gap, Value(ALG_RESULT)),
        None,
        "a partial image must refuse"
    );

    // The matching 32-bit pair does fold.
    let mut exact = algebraic_function(
        IntrinsicOp::Pand256,
        vec![Operand::Value(Value(ALG_A)), Operand::Value(Value(ALG_Z))],
        zero_producer(IntrinsicOp::Setzero256),
        false,
        32,
        IntrinsicOp::Pmovmskb256,
    );
    assert_eq!(
        movemask_result_after(&mut exact, Value(ALG_RESULT)),
        Some(0)
    );
}

#[test]
fn ssa_zero_producers_are_all_off_images_with_an_exact_width() {
    for (op, width, movemask) in [
        (IntrinsicOp::Setzero128, 16, IntrinsicOp::Pmovmskb128),
        (IntrinsicOp::SetzeroPs128, 16, IntrinsicOp::Pmovmskb128),
        (IntrinsicOp::Setzero256, 32, IntrinsicOp::Pmovmskb256),
    ] {
        let insts = vec![
            intrinsic(Some(Value(1)), op, None, vec![]),
            intrinsic(
                Some(Value(2)),
                movemask,
                None,
                vec![Operand::Value(Value(1))],
            ),
        ];
        let mut f = IrFunction::new("ssa_zero".into(), IrType::I32, vec![], false);
        f.blocks.push(block(
            insts,
            Terminator::Return(Some(Operand::Value(Value(2)))),
            true,
        ));
        f.next_value_id = 10;
        assert_eq!(
            movemask_result_after(&mut f, Value(2)),
            Some(0),
            "SSA {op:?} (width {width}) must be all-off"
        );
    }

    // Width mismatch: a 16-byte zero cannot answer a 32-lane movemask.
    let insts = vec![
        intrinsic(Some(Value(1)), IntrinsicOp::Setzero128, None, vec![]),
        intrinsic(
            Some(Value(2)),
            IntrinsicOp::Pmovmskb256,
            None,
            vec![Operand::Value(Value(1))],
        ),
    ];
    let mut f = IrFunction::new("ssa_zero".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Value(Value(2)))),
        true,
    ));
    f.next_value_id = 10;
    assert_eq!(movemask_result_after(&mut f, Value(2)), None);
}

/// `movemask(VecCmp*(a, b, pred))` with SSA vector operands.  `splat_a` and
/// `splat_b` may also be raw 16-byte `IrConst::I128` operands for a lane-wise
/// mixed oracle (the pass treats a full-width constant as a vector image).
fn compare_function(
    cmp: IntrinsicOp,
    a: Operand,
    b: Operand,
    pred: Operand,
    movemask: IntrinsicOp,
) -> IrFunction {
    let insts = vec![
        intrinsic(Some(Value(1)), cmp, None, vec![a, b, pred]),
        intrinsic(
            Some(Value(2)),
            movemask,
            None,
            vec![Operand::Value(Value(1))],
        ),
    ];
    let mut f = IrFunction::new("cmp".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Value(Value(2)))),
        true,
    ));
    f.next_value_id = 10;
    f
}

fn i128_const(dwords: [i32; 4]) -> Operand {
    let mut bytes = [0u8; 16];
    for (i, d) in dwords.iter().enumerate() {
        bytes[i * 4..i * 4 + 4].copy_from_slice(&d.to_le_bytes());
    }
    Operand::Const(IrConst::I128(i128::from_le_bytes(bytes)))
}

#[test]
fn constant_integer_compares_fold_to_exact_lane_masks() {
    // Equal / unequal byte splats, EQ and NE, at both vector widths.
    let cases: [(IntrinsicOp, IntrinsicOp, i8, i8, i32, i32); 4] = [
        (
            IntrinsicOp::VecCmpI8x16,
            IntrinsicOp::Pmovmskb128,
            -128,
            -128,
            0xFFFF,
            0,
        ),
        (
            IntrinsicOp::VecCmpI8x16,
            IntrinsicOp::Pmovmskb128,
            -128,
            127,
            0,
            0xFFFF,
        ),
        (
            IntrinsicOp::VecCmpI8x32,
            IntrinsicOp::Pmovmskb256,
            1,
            1,
            u32::MAX as i32,
            0,
        ),
        (
            IntrinsicOp::VecCmpI8x32,
            IntrinsicOp::Pmovmskb256,
            1,
            2,
            0,
            u32::MAX as i32,
        ),
    ];
    for (cmp, mm, x, y, eq, ne) in cases {
        let broadcast = if cmp == IntrinsicOp::VecCmpI8x32 {
            (
                IntrinsicOp::VecBroadcastI8x32,
                IntrinsicOp::VecBroadcastI8x32,
            )
        } else {
            (
                IntrinsicOp::VecBroadcastI8x16,
                IntrinsicOp::VecBroadcastI8x16,
            )
        };
        // Wrap the two constant splats as SSA values, exactly as `vectorize`
        // emits them, then feed the compare.
        let build = |pred: i32| -> IrFunction {
            let insts = vec![
                intrinsic(
                    Some(Value(1)),
                    broadcast.0,
                    None,
                    vec![Operand::Const(IrConst::I8(x))],
                ),
                intrinsic(
                    Some(Value(2)),
                    broadcast.1,
                    None,
                    vec![Operand::Const(IrConst::I8(y))],
                ),
                intrinsic(
                    Some(Value(3)),
                    cmp,
                    None,
                    vec![
                        Operand::Value(Value(1)),
                        Operand::Value(Value(2)),
                        Operand::Const(IrConst::I32(pred)),
                    ],
                ),
                intrinsic(Some(Value(4)), mm, None, vec![Operand::Value(Value(3))]),
            ];
            let mut f = IrFunction::new("cmp".into(), IrType::I32, vec![], false);
            f.blocks.push(block(
                insts,
                Terminator::Return(Some(Operand::Value(Value(4)))),
                true,
            ));
            f.next_value_id = 10;
            f
        };
        let mut f_eq = build(0);
        assert_eq!(
            movemask_result_after(&mut f_eq, Value(4)),
            Some(eq),
            "{cmp:?} eq"
        );
        let mut f_ne = build(4);
        assert_eq!(
            movemask_result_after(&mut f_ne, Value(4)),
            Some(ne),
            "{cmp:?} ne"
        );
    }

    // Lane-wise mixed dword oracle: EQ true in lanes 0 and 2 -> 0x0F0F,
    // NE true in lanes 1 and 3 -> 0xF0F0.
    let a = i128_const([1, 2, 3, 4]);
    let b = i128_const([1, 9, 3, 8]);
    let mut eq = compare_function(
        IntrinsicOp::VecCmpI32x4,
        a.clone(),
        b.clone(),
        Operand::Const(IrConst::I32(0)),
        IntrinsicOp::Pmovmskb128,
    );
    assert_eq!(movemask_result_after(&mut eq, Value(2)), Some(0x0F0F));
    let mut ne = compare_function(
        IntrinsicOp::VecCmpI32x4,
        a,
        b,
        Operand::Const(IrConst::I32(4)),
        IntrinsicOp::Pmovmskb128,
    );
    assert_eq!(movemask_result_after(&mut ne, Value(2)), Some(0xF0F0));

    // The other integer lane widths use the very same lane equality proof.
    for (cmp, a, b) in [
        (
            IntrinsicOp::VecCmpI16x8,
            i128_const([1, 2, 3, 4]),
            i128_const([1, 2, 9, 9]),
        ),
        (
            IntrinsicOp::VecCmpI64x2,
            i128_const([1, 2, 3, 4]),
            i128_const([1, 2, 3, 9]),
        ),
    ] {
        let mut f = compare_function(
            cmp,
            a,
            b,
            Operand::Const(IrConst::I32(0)),
            IntrinsicOp::Pmovmskb128,
        );
        assert!(
            movemask_result_after(&mut f, Value(2)).is_some(),
            "{cmp:?} must fold a mixed constant compare"
        );
    }
}

#[test]
fn compare_shape_and_predicate_guards_are_fail_closed() {
    let a = Operand::Const(IrConst::I128(0x0102_0304_0506_0708_090A_0B0C_0D0E_0F10));
    let b = a.clone();
    // Only eq(0) and ne(4) are sign-independent; every other predicate
    // needs signedness/ordering the pass has not proved.
    for pred in [1i32, 2, 5, 6, 7, -1] {
        let mut f = compare_function(
            IntrinsicOp::VecCmpI32x4,
            a.clone(),
            b.clone(),
            Operand::Const(IrConst::I32(pred)),
            IntrinsicOp::Pmovmskb128,
        );
        assert_eq!(
            movemask_result_after(&mut f, Value(2)),
            None,
            "predicate {pred} must refuse"
        );
    }
    // A runtime predicate operand is not a proof.
    let mut runtime_pred = compare_function(
        IntrinsicOp::VecCmpI32x4,
        a.clone(),
        b.clone(),
        Operand::Value(Value(9)),
        IntrinsicOp::Pmovmskb128,
    );
    assert_eq!(movemask_result_after(&mut runtime_pred, Value(2)), None);
    // A 16-byte compare cannot answer a 32-lane movemask.
    let mut width = compare_function(
        IntrinsicOp::VecCmpI32x4,
        a.clone(),
        b.clone(),
        Operand::Const(IrConst::I32(0)),
        IntrinsicOp::Pmovmskb256,
    );
    assert_eq!(movemask_result_after(&mut width, Value(2)), None);
    // FP compares are deliberately out of scope (NaN / signed-zero rules).
    let mut fp = compare_function(
        IntrinsicOp::VecCmpF32x4,
        a.clone(),
        b.clone(),
        Operand::Const(IrConst::I32(0)),
        IntrinsicOp::Pmovmskb128,
    );
    assert_eq!(movemask_result_after(&mut fp, Value(2)), None);
    // Wrong arity.
    let insts = vec![
        intrinsic(
            Some(Value(1)),
            IntrinsicOp::VecCmpI32x4,
            None,
            vec![a.clone(), b.clone()],
        ),
        intrinsic(
            Some(Value(2)),
            IntrinsicOp::Pmovmskb128,
            None,
            vec![Operand::Value(Value(1))],
        ),
    ];
    let mut arity = IrFunction::new("cmp".into(), IrType::I32, vec![], false);
    arity.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Value(Value(2)))),
        true,
    ));
    arity.next_value_id = 10;
    assert_eq!(movemask_result_after(&mut arity, Value(2)), None);
}

#[test]
fn compare_self_identity_folds_runtime_ssa_values() {
    // `VecLoadI8x16` from an unwritten object produces a runtime SSA vector
    // value.  Using it twice is bit-identical, so eq is all-on and ne all-off.
    for (pred, expected) in [(0i32, 0xFFFF), (4, 0)] {
        let insts = vec![
            alloca(Value(0), 16, false, false),
            intrinsic(
                Some(Value(1)),
                IntrinsicOp::VecLoadI8x16,
                None,
                vec![Operand::Value(Value(0)), Operand::Const(IrConst::I64(0))],
            ),
            intrinsic(
                Some(Value(2)),
                IntrinsicOp::VecCmpI8x16,
                None,
                vec![
                    Operand::Value(Value(1)),
                    Operand::Value(Value(1)),
                    Operand::Const(IrConst::I32(pred)),
                ],
            ),
            intrinsic(
                Some(Value(3)),
                IntrinsicOp::Pmovmskb128,
                None,
                vec![Operand::Value(Value(2))],
            ),
        ];
        let mut f = IrFunction::new("cmp_self".into(), IrType::I32, vec![], false);
        f.blocks.push(block(
            insts,
            Terminator::Return(Some(Operand::Value(Value(3)))),
            true,
        ));
        f.next_value_id = 10;
        assert_eq!(
            movemask_result_after(&mut f, Value(3)),
            Some(expected),
            "self-compare predicate {pred}"
        );
    }
}

#[test]
fn new_producers_are_reclaimed_and_leave_no_dangling_ssa() {
    // The zero producer and the computed write must both be removable once
    // the movemask is folded; `dce` must not leave an undefined SSA value.
    let mut f = algebraic_function(
        IntrinsicOp::Pxor128,
        vec![Operand::Value(Value(ALG_A)), Operand::Value(Value(ALG_A))],
        zero_producer(IntrinsicOp::Setzero128),
        false,
        16,
        IntrinsicOp::Pmovmskb128,
    );
    assert_eq!(movemask_result_after(&mut f, Value(ALG_RESULT)), Some(0));
    let defs: Vec<u32> = f.blocks[0]
        .instructions
        .iter()
        .filter_map(|i| i.dest().map(|v| v.0))
        .collect();
    let uses: Vec<u32> = f.blocks[0]
        .instructions
        .iter()
        .flat_map(|i| i.used_values())
        .collect();
    for u in &uses {
        assert!(
            defs.contains(u),
            "dangling SSA reference to Value({u}) after reclamation"
        );
    }
}

// ---------------------------------------------------------------------------
// Round 6: repeated-operand purity is a POSITIVE proof, not a fallback.
// ---------------------------------------------------------------------------

/// Build `Pxor128 { args: [v, v] } -> movemask` with one extra definition
/// instruction in the prologue, and return the folded movemask constant.
fn identity_with_definition(defs: Vec<Instruction>, v: Value) -> Option<i32> {
    let mut f = algebraic_function(
        IntrinsicOp::Pxor128,
        vec![Operand::Value(v), Operand::Value(v)],
        defs,
        false,
        16,
        IntrinsicOp::Pmovmskb128,
    );
    movemask_result_after(&mut f, Value(ALG_RESULT))
}

fn purity_function(insts: Vec<Instruction>) -> crate::ir::reexports::IrFunction {
    let mut f = crate::ir::reexports::IrFunction::new("purity".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
        true,
    ));
    f.next_value_id = 1000;
    f
}

#[test]
fn repeated_operand_refuses_every_pointer_and_observable_definition() {
    const V: Value = Value(999);
    // A pointer materialised by a cast, a global address, a label address,
    // a phi, a volatile load, a pointer-typed load, a write-through home,
    // an atomic load, and an external value with no definition at all.
    let definitions: Vec<(&str, Vec<Instruction>)> = vec![
        (
            "cast to pointer",
            vec![Instruction::Cast {
                dest: V,
                src: Operand::Const(IrConst::I64(0)),
                from_ty: IrType::I64,
                to_ty: IrType::Ptr,
            }],
        ),
        (
            "global address",
            vec![Instruction::GlobalAddr {
                dest: V,
                name: "extern_object".into(),
            }],
        ),
        (
            "label address",
            vec![Instruction::LabelAddr {
                dest: V,
                label: BlockId(0),
            }],
        ),
        (
            "phi",
            vec![Instruction::Phi {
                dest: V,
                ty: IrType::I128,
                incoming: vec![(Operand::Const(IrConst::I64(0)), BlockId(0))],
            }],
        ),
        (
            "volatile load",
            vec![Instruction::Load {
                dest: V,
                ptr: Value(ALG_A),
                ty: IrType::I64,
                seg_override: AddressSpace::Default,
                volatile: true,
            }],
        ),
        (
            "pointer-typed load",
            vec![Instruction::Load {
                dest: V,
                ptr: Value(ALG_A),
                ty: IrType::Ptr,
                seg_override: AddressSpace::Default,
                volatile: false,
            }],
        ),
        (
            "write-through intrinsic home",
            vec![intrinsic(
                Some(V),
                IntrinsicOp::Pxor128,
                Some(Value(ALG_Z)),
                vec![],
            )],
        ),
        (
            "atomic load",
            vec![Instruction::AtomicLoad {
                dest: V,
                ptr: Operand::Value(Value(ALG_A)),
                ty: IrType::I64,
                ordering: crate::ir::ops::AtomicOrdering::SeqCst,
            }],
        ),
        ("undefined external value", vec![]),
        (
            "copy cycle",
            vec![
                copy(V, Operand::Value(Value(998))),
                copy(Value(998), Operand::Value(V)),
            ],
        ),
        (
            "copy to undefined value",
            vec![copy(V, Operand::Value(Value(997)))],
        ),
    ];
    for (label, defs) in definitions {
        assert_eq!(
            identity_with_definition(defs, V),
            None,
            "repeated operand defined by {label} must NOT be treated as pure"
        );
    }
    // Control: a plain non-volatile stack home IS the legacy form and must
    // still fold.  This proves the refusals above are selective, not a
    // blanket disable of the identity.
    assert_eq!(
        identity_with_definition(vec![], Value(ALG_A)),
        Some(0),
        "a proven plain stack home must still fold"
    );
}

#[test]
fn repeated_operand_accepts_a_copy_chain_ending_in_a_pure_ssa_producer() {
    const V: Value = Value(999);
    let defs = vec![
        intrinsic(
            Some(Value(998)),
            IntrinsicOp::VecBroadcastI8x16,
            None,
            vec![Operand::Const(IrConst::I8(-1))],
        ),
        copy(V, Operand::Value(Value(998))),
    ];
    assert_eq!(
        identity_with_definition(defs, V),
        Some(0),
        "an SSA vector value reached through Copy is immutable"
    );
    // A whole-vector load is also an immutable SSA value after the load.
    let loaded = vec![intrinsic(
        Some(V),
        IntrinsicOp::VecLoadI8x16,
        None,
        vec![
            Operand::Value(Value(ALG_A)),
            Operand::Const(IrConst::I64(0)),
        ],
    )];
    assert_eq!(identity_with_definition(loaded, V), Some(0));
}

#[test]
fn purity_refuses_on_budget_exhaustion_and_cycles() {
    // Direct probe of the decision procedure: budget 0 must refuse even for
    // the most obviously pure shape, and a Copy cycle must refuse regardless
    // of the starting budget.  Pinning both prevents an unbounded walk from
    // being reintroduced under a new identity rule.
    let plain = purity_function(vec![
        alloca(Value(ALG_A), 16, false, false),
        copy(Value(999), Operand::Value(Value(ALG_A))),
    ]);
    let facts = Facts::new(&plain);
    let mut zero_budget = 0usize;
    assert!(!facts.operand_is_pure(Operand::Value(Value(999)), &mut zero_budget));
    assert_eq!(zero_budget, 0);
    let mut full_budget = MAX_WORK;
    assert!(facts.operand_is_pure(Operand::Value(Value(ALG_A)), &mut full_budget));
    assert!(
        full_budget < MAX_WORK,
        "the proof must charge the shared budget"
    );

    let cyclic = purity_function(vec![
        copy(Value(999), Operand::Value(Value(998))),
        copy(Value(998), Operand::Value(Value(999))),
    ]);
    let facts = Facts::new(&cyclic);
    let mut budget = MAX_WORK;
    assert!(!facts.operand_is_pure(Operand::Value(Value(999)), &mut budget));
    assert!(
        budget > 0,
        "the cycle must terminate by depth, not by draining all work"
    );
}

#[test]
fn ssa_intrinsic_without_a_write_through_home_refuses_object_mentions() {
    // `mention_of` must stay fail-closed for an SSA byte-logic intrinsic
    // whose args happen to name an object: with `dest_ptr: None` there is no
    // proven memory home, so classifying it as a WRITE or as harmless would
    // both be guesses.  `Reject` is the only defensible verdict.
    let inst = intrinsic(
        Some(Value(1)),
        IntrinsicOp::VecXorI32x4,
        None,
        vec![Operand::Value(Value(ALG_A)), Operand::Value(Value(ALG_A))],
    );
    let f = purity_function(vec![alloca(Value(ALG_A), 16, false, false), inst.clone()]);
    let facts = Facts::new(&f);
    let w = ImageWindow {
        base: Value(ALG_A),
        size: 16,
    };
    let mut budget = MAX_WORK;
    let mut visiting = Vec::new();
    assert!(matches!(
        facts.mention_of(&inst, 0, 0, w, &mut budget, &mut visiting),
        ObjectMention::Reject
    ));
}

// ---------------------------------------------------------------------------
// Round 7: ISPC `IntrinsicsOpt` scalar `__mmaskN` transplants.
// ---------------------------------------------------------------------------

const MC_A: u32 = 100;
const MC_B: u32 = 200;
const MC_RESULT: u32 = 1;

/// A proven-constant stack image: one plain Alloca tiled by byte stores.
fn mc_image(base: Value, bytes: &[u8], gep_start: u32) -> Vec<Instruction> {
    let mut v = vec![alloca(base, bytes.len(), false, false)];
    for (i, b) in bytes.iter().enumerate() {
        let p = Value(gep_start + i as u32);
        v.push(gep(p, base, Operand::Const(IrConst::I64(i as i64))));
        v.push(byte_store(p, *b, false));
    }
    v
}

fn mc_function(cmp: IntrinsicOp, pred: Operand, a: &[u8], b: &[u8]) -> IrFunction {
    let mut insts = mc_image(Value(MC_A), a, 400);
    insts.extend(mc_image(Value(MC_B), b, 600));
    insts.push(intrinsic(
        Some(Value(MC_RESULT)),
        cmp,
        None,
        vec![
            Operand::Value(Value(MC_A)),
            Operand::Value(Value(MC_B)),
            pred,
        ],
    ));
    let mut f = IrFunction::new("mcmp".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Value(Value(MC_RESULT)))),
        true,
    ));
    f.next_value_id = 1000;
    f
}

/// Independent oracle: bit i of the scalar mask is the equality of lane i.
fn mc_oracle(a: &[u8], b: &[u8], lane_bytes: usize, lanes: u32, ne: bool) -> u64 {
    let mut m = 0u64;
    for i in 0..lanes as usize {
        let x = &a[i * lane_bytes..(i + 1) * lane_bytes];
        let y = &b[i * lane_bytes..(i + 1) * lane_bytes];
        if (x == y) ^ ne {
            m |= 1u64 << i;
        }
    }
    m
}

fn mc_folded(func: &IrFunction) -> Option<i64> {
    let facts = Facts::new(func);
    // Find by DEST, not by shape: the negative cases deliberately use
    // opcodes outside `mask_compare_shape` (FP compares), which must be
    // refused rather than skipped by the test harness.
    let (index, inst) = func.blocks[0]
        .instructions
        .iter()
        .enumerate()
        .find(|(_, i)| matches!(i, Instruction::Intrinsic { dest: Some(d), .. } if *d == Value(MC_RESULT)))
        .expect("mask compare present");
    match fold(&facts, inst, 0, index)? {
        Instruction::Copy {
            src: Operand::Const(c),
            ..
        } => c.to_i64(),
        _ => None,
    }
}

#[test]
fn scalar_mask_lattice_is_exact_over_all_16_bit_values() {
    let f = purity_function(vec![]);
    let facts = Facts::new(&f);
    for k in 0u64..=0xFFFF {
        let got = facts.scalar_mask(Operand::Const(IrConst::I64(k as i64)), 16);
        let want = match k {
            0 => ScalarMask::AllOff,
            0xFFFF => ScalarMask::AllOn,
            other => ScalarMask::Mixed(other),
        };
        assert_eq!(got, want, "16-lane scalar mask {k:#06x}");
    }
    // Signed -1 and over-wide constants are masked to the lane width.
    assert_eq!(
        facts.scalar_mask(Operand::Const(IrConst::I64(-1)), 16),
        ScalarMask::AllOn
    );
    assert_eq!(
        facts.scalar_mask(Operand::Const(IrConst::I64(0x1234_FFFF)), 16),
        ScalarMask::AllOn
    );
    assert_eq!(
        facts.scalar_mask(Operand::Const(IrConst::I64(0x1_0000)), 16),
        ScalarMask::AllOff
    );
    // 64 lanes use the full u64 domain; 0 lanes / 65 lanes refuse.
    assert_eq!(
        facts.scalar_mask(Operand::Const(IrConst::I64(-1)), 64),
        ScalarMask::AllOn
    );
    assert_eq!(
        facts.scalar_mask(Operand::Const(IrConst::I64(0)), 0),
        ScalarMask::Unknown
    );
    assert_eq!(
        facts.scalar_mask(Operand::Const(IrConst::I64(0)), 65),
        ScalarMask::Unknown
    );
    // A float constant is not a mask; neither is an undefined value.
    assert_eq!(
        facts.scalar_mask(Operand::Const(IrConst::F64(1.0)), 16),
        ScalarMask::Unknown
    );
    assert_eq!(
        facts.scalar_mask(Operand::Value(Value(999)), 16),
        ScalarMask::Unknown
    );
    // Bounded Copy chains, cycles and non-Copy definitions.
    let chained = purity_function(vec![
        copy(Value(10), Operand::Const(IrConst::I64(0))),
        copy(Value(11), Operand::Value(Value(10))),
    ]);
    assert_eq!(
        Facts::new(&chained).scalar_mask(Operand::Value(Value(11)), 16),
        ScalarMask::AllOff
    );
    let cyclic = purity_function(vec![
        copy(Value(10), Operand::Value(Value(11))),
        copy(Value(11), Operand::Value(Value(10))),
    ]);
    assert_eq!(
        Facts::new(&cyclic).scalar_mask(Operand::Value(Value(10)), 16),
        ScalarMask::Unknown
    );
    let noncopy = purity_function(vec![alloca(Value(10), 16, false, false)]);
    assert_eq!(
        Facts::new(&noncopy).scalar_mask(Operand::Value(Value(10)), 16),
        ScalarMask::Unknown
    );
}

#[test]
fn constant_mask_compares_fold_to_exact_scalar_masks() {
    // Four lane widths, two vector widths, two sign-independent predicates.
    let cases: [(IntrinsicOp, usize, usize, u32, usize); 8] = [
        (IntrinsicOp::CmpeqEpu8Mask128, 16, 1, 16, 16),
        (IntrinsicOp::CmpEpi8Mask128, 16, 1, 16, 16),
        (IntrinsicOp::CmpEpi16Mask128, 16, 2, 8, 16),
        (IntrinsicOp::CmpEpi32Mask128, 16, 4, 4, 16),
        (IntrinsicOp::CmpEpi64Mask128, 16, 8, 2, 16),
        (IntrinsicOp::CmpeqEpu8Mask256, 32, 1, 32, 32),
        (IntrinsicOp::CmpEpi32Mask256, 32, 4, 8, 32),
        (IntrinsicOp::CmpEpu64Mask256, 32, 8, 4, 32),
    ];
    for (cmp, width, lane_bytes, lanes, _) in cases {
        let a: Vec<u8> = (0..width)
            .map(|i| ((i as u32 * 37 + 11) & 0xFF) as u8)
            .collect();
        let mut b = a.clone();
        // Flip one byte in lanes 1 and 3 (and leave lanes 0, 2 equal).
        b[lane_bytes] ^= 0x40;
        if 3 * lane_bytes < width {
            b[3 * lane_bytes] ^= 0x80;
        }
        for (pred, ne) in [(0i32, false), (4, true)] {
            let f = mc_function(cmp, Operand::Const(IrConst::I32(pred)), &a, &b);
            let want = mc_oracle(&a, &b, lane_bytes, lanes, ne);
            assert_eq!(
                mc_folded(&f),
                Some(want as i64),
                "{cmp:?} pred={pred} must fold to {want:#x}"
            );
        }
    }
    // All-equal and all-different images exercise the two extreme masks.
    let a = vec![0x5Au8; 16];
    let b = vec![0xA5u8; 16];
    let eq = mc_function(
        IntrinsicOp::CmpEpi8Mask128,
        Operand::Const(IrConst::I32(0)),
        &a,
        &b,
    );
    assert_eq!(mc_folded(&eq), Some(0));
    let same = mc_function(
        IntrinsicOp::CmpEpi8Mask128,
        Operand::Const(IrConst::I32(0)),
        &a,
        &a,
    );
    assert_eq!(mc_folded(&same), Some(0xFFFF));
}

/// Round 8: `MAX_IMAGE = 64` unlocks the WHOLE AVX-512 scalar-mask compare
/// domain.  Every 512-bit compare has a 64-byte image; the byte-lane form
/// has 64 lanes, which is exactly the `u64` mask width (`kmovq %k1, %rax`).
/// The independent `mc_oracle` recomputes each bit from the byte arrays, so
/// this is a proof, not a restatement of the fold.
#[test]
fn avx512_constant_compares_fold_to_exact_64_bit_scalar_masks() {
    let cases: [(IntrinsicOp, usize, u32); 12] = [
        (IntrinsicOp::CmpeqEpu8Mask512, 1, 64),
        (IntrinsicOp::CmpEpi8Mask512, 1, 64),
        (IntrinsicOp::CmpEpu8Mask512, 1, 64),
        (IntrinsicOp::CmpeqEpu16Mask512, 2, 32),
        (IntrinsicOp::CmpEpi16Mask512, 2, 32),
        (IntrinsicOp::CmpEpu16Mask512, 2, 32),
        (IntrinsicOp::CmpeqEpu32Mask512, 4, 16),
        (IntrinsicOp::CmpEpi32Mask512, 4, 16),
        (IntrinsicOp::CmpEpu32Mask512, 4, 16),
        (IntrinsicOp::CmpeqEpu64Mask512, 8, 8),
        (IntrinsicOp::CmpEpi64Mask512, 8, 8),
        (IntrinsicOp::CmpEpu64Mask512, 8, 8),
    ];
    for (cmp, lane_bytes, lanes) in cases {
        let a: Vec<u8> = (0..64)
            .map(|i| ((i as u32 * 37 + 11) & 0xFF) as u8)
            .collect();
        let mut b = a.clone();
        b[lane_bytes] ^= 0x40; // lane 1 differs
        // The TOP lane differs too: this exercises `1u64 << 63` for the
        // 64-lane byte form, the case the old `lane_count > 63` guard
        // silently refused.
        b[(lanes as usize - 1) * lane_bytes] ^= 0x80;
        for (pred, ne) in [(0i32, false), (4, true)] {
            let f = mc_function(cmp, Operand::Const(IrConst::I32(pred)), &a, &b);
            let want = mc_oracle(&a, &b, lane_bytes, lanes, ne);
            assert_eq!(
                mc_folded(&f),
                Some(want as i64),
                "{cmp:?} pred={pred} must fold to {want:#018x}"
            );
        }
    }

    // Extremes: all 64 lanes equal / none equal, for both predicates.
    let a = vec![0x5Au8; 64];
    let same = mc_function(
        IntrinsicOp::CmpeqEpu8Mask512,
        Operand::Const(IrConst::I32(0)),
        &a,
        &a,
    );
    assert_eq!(mc_folded(&same), Some(-1), "all 64 lanes equal => u64::MAX");
    // A stack array (not `vec!`): the image engine is allocation-free and
    // the test must not allocate a heap buffer just to name 64 bytes.
    let other = [0xA5u8; 64];
    let diff = mc_function(
        IntrinsicOp::CmpeqEpu8Mask512,
        Operand::Const(IrConst::I32(0)),
        &a,
        &other,
    );
    assert_eq!(mc_folded(&diff), Some(0));
    let ne = mc_function(
        IntrinsicOp::CmpEpi8Mask512,
        Operand::Const(IrConst::I32(4)),
        &a,
        &other,
    );
    assert_eq!(
        mc_folded(&ne),
        Some(-1),
        "ne with no equal lane => u64::MAX"
    );

    // Exactly ONE equal lane: the top lane (bit 63). `1u64 << 63` is the
    // boundary shift; a `> 63` lane-count guard would have refused it.
    let mut a = vec![0x11u8; 64];
    let mut b = vec![0x22u8; 64];
    b[63] = 0x11;
    let top = mc_function(
        IntrinsicOp::CmpeqEpu8Mask512,
        Operand::Const(IrConst::I32(0)),
        &a,
        &b,
    );
    assert_eq!(mc_folded(&top), Some(i64::MIN), "only bit 63 set");
    a[63] = 0x22;
    let none = mc_function(
        IntrinsicOp::CmpeqEpu8Mask512,
        Operand::Const(IrConst::I32(0)),
        &a,
        &b,
    );
    assert_eq!(mc_folded(&none), Some(0));

    // Refusal: an ordering predicate has no lane-signedness proof even at
    // 512 bits, and a 32-byte image cannot prove a 64-byte compare.
    for pred in [1i32, 2, 3, 5, 6, 7] {
        let f = mc_function(
            IntrinsicOp::CmpEpi8Mask512,
            Operand::Const(IrConst::I32(pred)),
            &a,
            &b,
        );
        assert_eq!(mc_folded(&f), None, "predicate {pred} must refuse");
    }
}

#[test]
fn mask_compare_guards_are_fail_closed() {
    let a = vec![0x11u8; 16];
    let b = vec![0x22u8; 16];
    // Signed/unsigned ordering predicates have no lane-signedness proof.
    for pred in [1i32, 2, 3, 5, 6, 7, -1] {
        let f = mc_function(
            IntrinsicOp::CmpEpi8Mask128,
            Operand::Const(IrConst::I32(pred)),
            &a,
            &b,
        );
        assert_eq!(mc_folded(&f), None, "predicate {pred} must refuse");
    }
    // A runtime predicate is not a proof.
    let runtime_pred = mc_function(
        IntrinsicOp::CmpEpi8Mask128,
        Operand::Value(Value(999)),
        &a,
        &b,
    );
    assert_eq!(mc_folded(&runtime_pred), None);
    // A 512-bit compare with only 16 provable bytes must still refuse:
    // the image proof is width-exact, never a partial prefix.
    let wide = mc_function(
        IntrinsicOp::CmpeqEpu8Mask512,
        Operand::Const(IrConst::I32(0)),
        &a,
        &b,
    );
    assert_eq!(mc_folded(&wide), None);
    // An FP compare opcode is not in the integer mask-compare table.
    let fp = mc_function(
        IntrinsicOp::VecCmpF32x4,
        Operand::Const(IrConst::I32(0)),
        &a,
        &b,
    );
    assert_eq!(mc_folded(&fp), None);
    // Wrong arity: no predicate operand.
    let mut f = mc_function(
        IntrinsicOp::CmpEpi8Mask128,
        Operand::Const(IrConst::I32(0)),
        &a,
        &b,
    );
    if let Instruction::Intrinsic { args, .. } = &mut f.blocks[0]
        .instructions
        .iter_mut()
        .find(
            |i| matches!(i, Instruction::Intrinsic { op, .. } if mask_compare_shape(*op).is_some()),
        )
        .expect("compare")
    {
        args.pop();
    }
    assert_eq!(mc_folded(&f), None);
    // An unproven (volatile) operand image cannot be folded.
    let mut vol = mc_function(
        IntrinsicOp::CmpEpi8Mask128,
        Operand::Const(IrConst::I32(0)),
        &a,
        &b,
    );
    for inst in &mut vol.blocks[0].instructions {
        if let Instruction::Alloca { dest, volatile, .. } = inst {
            if *dest == Value(MC_B) {
                *volatile = true;
            }
        }
    }
    assert_eq!(mc_folded(&vol), None);
}

#[test]
fn mask_compare_self_identity_needs_no_operand_image() {
    for (pred, want) in [(0i32, 0xFFFFi64), (4, 0)] {
        let insts = vec![
            alloca(Value(MC_A), 16, false, false),
            intrinsic(
                Some(Value(MC_RESULT)),
                IntrinsicOp::CmpEpi8Mask128,
                None,
                vec![
                    Operand::Value(Value(MC_A)),
                    Operand::Value(Value(MC_A)),
                    Operand::Const(IrConst::I32(pred)),
                ],
            ),
        ];
        let mut f = IrFunction::new("mcself".into(), IrType::I32, vec![], false);
        f.blocks.push(block(
            insts,
            Terminator::Return(Some(Operand::Value(Value(MC_RESULT)))),
            true,
        ));
        f.next_value_id = 1000;
        assert_eq!(mc_folded(&f), Some(want), "self-compare pred {pred}");
    }
}

#[test]
fn mask_compare_is_a_pure_read_for_object_images() {
    // `mention_of` must classify a mask compare as NoWrite (a read cannot
    // change an image) but refuse an unresolved pointer operand.
    let inst = intrinsic(
        Some(Value(MC_RESULT)),
        IntrinsicOp::CmpEpi8Mask128,
        None,
        vec![
            Operand::Value(Value(MC_A)),
            Operand::Value(Value(MC_B)),
            Operand::Const(IrConst::I32(0)),
        ],
    );
    let f = purity_function(vec![
        alloca(Value(MC_A), 16, false, false),
        alloca(Value(MC_B), 16, false, false),
        inst.clone(),
    ]);
    let facts = Facts::new(&f);
    let w = ImageWindow {
        base: Value(MC_A),
        size: 16,
    };
    let mut budget = MAX_WORK;
    let mut visiting = Vec::new();
    assert!(matches!(
        facts.mention_of(&inst, 0, 0, w, &mut budget, &mut visiting),
        ObjectMention::NoWrite
    ));
    let unresolved = intrinsic(
        Some(Value(MC_RESULT)),
        IntrinsicOp::CmpEpi8Mask128,
        None,
        vec![
            Operand::Value(Value(MC_A)),
            Operand::Value(Value(999)),
            Operand::Const(IrConst::I32(0)),
        ],
    );
    assert!(matches!(
        facts.mention_of(&unresolved, 0, 0, w, &mut budget, &mut visiting),
        ObjectMention::Reject
    ));
}

fn maskz_function(
    op: IntrinsicOp,
    mask: Operand,
    size: usize,
    dest_used: bool,
    volatile: bool,
) -> IrFunction {
    let mut insts = vec![alloca(Value(MC_A), size, volatile, false)];
    insts.push(intrinsic(
        Some(Value(MC_RESULT)),
        op,
        Some(Value(MC_A)),
        vec![mask, Operand::Value(Value(999))],
    ));
    if dest_used {
        insts.push(copy(Value(2), Operand::Value(Value(MC_RESULT))));
    }
    let mut f = IrFunction::new("maskz".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
        true,
    ));
    f.next_value_id = 1000;
    f
}

fn maskz_folded(func: &IrFunction) -> Option<IntrinsicOp> {
    let facts = Facts::new(func);
    let (index, inst) = func.blocks[0]
        .instructions
        .iter()
        .enumerate()
        .find(|(_, i)| matches!(i, Instruction::Intrinsic { op, .. } if matches!(*op, IntrinsicOp::MaskzLoaduEpi8_128 | IntrinsicOp::MaskzLoaduEpi8_256 | IntrinsicOp::MaskzLoaduEpi8_512)))
        .expect("maskz load present");
    match fold(&facts, inst, 0, index)? {
        Instruction::Intrinsic {
            op,
            dest: None,
            dest_ptr: Some(_),
            args,
        } if args.is_empty() => Some(op),
        _ => None,
    }
}

#[test]
fn all_off_maskz_load_becomes_an_exact_width_zero_producer() {
    for (op, size, want) in [
        (IntrinsicOp::MaskzLoaduEpi8_128, 16, IntrinsicOp::Setzero128),
        (IntrinsicOp::MaskzLoaduEpi8_256, 32, IntrinsicOp::Setzero256),
    ] {
        let f = maskz_function(op, Operand::Const(IrConst::I64(0)), size, false, false);
        assert_eq!(maskz_folded(&f), Some(want), "{op:?}");
        // Signed -1 in the mask slot is NOT all-off.
        let on = maskz_function(op, Operand::Const(IrConst::I64(-1)), size, false, false);
        assert_eq!(maskz_folded(&on), None);
        // A mixed mask is not all-off.
        let mixed = maskz_function(op, Operand::Const(IrConst::I64(1)), size, false, false);
        assert_eq!(maskz_folded(&mixed), None);
        // A runtime mask is not a proof.
        let runtime = maskz_function(op, Operand::Value(Value(999)), size, false, false);
        assert_eq!(maskz_folded(&runtime), None);
        // A used SSA dest cannot be dropped.
        let used = maskz_function(op, Operand::Const(IrConst::I64(0)), size, true, false);
        assert_eq!(maskz_folded(&used), None);
        // A volatile destination is refused.
        let vol = maskz_function(op, Operand::Const(IrConst::I64(0)), size, false, true);
        assert_eq!(maskz_folded(&vol), None);
        // A too-small alloca would be a stack overwrite; refuse.
        let small = maskz_function(op, Operand::Const(IrConst::I64(0)), size / 2, false, false);
        assert_eq!(maskz_folded(&small), None);
        // Round 8: `Setzero512` now provides the EXACT 64-byte zero
        // producer, so the 512-bit all-off load folds too (previously this
        // was a deliberate refusal).
        let wide = maskz_function(
            IntrinsicOp::MaskzLoaduEpi8_512,
            Operand::Const(IrConst::I64(0)),
            64,
            false,
            false,
        );
        assert_eq!(maskz_folded(&wide), Some(IntrinsicOp::Setzero512));
    }
}

fn mask_store_function(op: IntrinsicOp, mask: Operand) -> IrFunction {
    let insts = vec![
        alloca(Value(MC_A), 16, false, false),
        intrinsic(
            None,
            op,
            Some(Value(MC_A)),
            vec![mask, Operand::Const(IrConst::I64(7))],
        ),
    ];
    let mut f = IrFunction::new("mstore".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
        true,
    ));
    f.next_value_id = 1000;
    f
}

#[test]
fn all_on_128_bit_maskz_load_uses_the_proven_unmasked_load() {
    // `Loaddqu` with a raw pointer is exactly what `_mm_loadu_si128` lowers
    // to (measured in the x86 backend).  With every lane active the `{k1}{z}`
    // decorator and its `kmov` are pure overhead.
    let f = maskz_function(
        IntrinsicOp::MaskzLoaduEpi8_128,
        Operand::Const(IrConst::I64(0xFFFF)),
        16,
        false,
        false,
    );
    let facts = Facts::new(&f);
    let inst = &f.blocks[0].instructions[1];
    match fold(&facts, inst, 0, 1).expect("all-on 128-bit maskz load must fold") {
        Instruction::Intrinsic {
            op,
            dest: None,
            dest_ptr: Some(dp),
            args,
        } => {
            assert_eq!(op, IntrinsicOp::Loaddqu);
            assert_eq!(dp, Value(MC_A));
            assert_eq!(args.len(), 1, "the memory pointer must be preserved");
            assert!(matches!(args[0], Operand::Value(_)));
        }
        other => panic!("expected Loaddqu, got {other:?}"),
    }
    // Round 8: the 256-bit all-on form folds to `Loadu256Raw`, a DEDICATED
    // RAW-POINTER load.  `maskz_folded` only recognises zero producers
    // (empty args), so it reports None here; assert the real rewrite and pin
    // that the home-based `Loadu256` is NEVER substituted.
    let wide = maskz_function(
        IntrinsicOp::MaskzLoaduEpi8_256,
        Operand::Const(IrConst::I64(0xFFFF_FFFF)),
        32,
        false,
        false,
    );
    assert_eq!(
        maskz_folded(&wide),
        None,
        "a load rewrite is not a zero producer"
    );
    let facts = Facts::new(&wide);
    match fold(&facts, &wide.blocks[0].instructions[1], 0, 1)
        .expect("256-bit all-on maskz load must fold")
    {
        Instruction::Intrinsic {
            op,
            dest: None,
            dest_ptr: Some(_),
            args,
        } => {
            assert_eq!(op, IntrinsicOp::Loadu256Raw);
            assert_ne!(op, IntrinsicOp::Loadu256, "Loadu256 reads a vector home");
            assert_eq!(args.len(), 1, "the raw pointer must be preserved");
        }
        other => panic!("expected Loadu256Raw, got {other:?}"),
    }
    // A 16-bit all-on constant with junk in the ignored upper bits is still
    // all-on (the hardware masks to 16 lanes).
    let masked = maskz_function(
        IntrinsicOp::MaskzLoaduEpi8_128,
        Operand::Const(IrConst::I64(0x1234_FFFF)),
        16,
        false,
        false,
    );
    let facts = Facts::new(&masked);
    assert!(fold(&facts, &masked.blocks[0].instructions[1], 0, 1).is_some());
}

#[test]
fn all_off_masked_stores_are_erased_span_safely() {
    for op in [
        IntrinsicOp::MaskStoreuEpi8_128,
        IntrinsicOp::MaskStoreuEpi8_256,
        IntrinsicOp::MaskStoreuEpi8_512,
    ] {
        let mut f = mask_store_function(op, Operand::Const(IrConst::I64(0)));
        let edits = run_function(&mut f);
        assert!(edits >= 1, "{op:?} all-off store must be edited");
        assert!(
            !f.blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Intrinsic { op: o, .. } if *o == op)),
            "{op:?} all-off store must be erased"
        );
        // `remove_instructions` must keep source_spans parallel.
        let spans = f.blocks[0].source_spans.len();
        let insts = f.blocks[0].instructions.len();
        assert!(spans == 0 || spans == insts, "span drift after erasure");
    }
    // All-on, mixed, runtime masks and the value-form are NOT erasable.
    for mask in [
        Operand::Const(IrConst::I64(-1)),
        Operand::Const(IrConst::I64(1)),
        Operand::Value(Value(999)),
    ] {
        let mut f = mask_store_function(IntrinsicOp::MaskStoreuEpi8_128, mask);
        run_function(&mut f);
        assert!(
            f.blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Intrinsic { op, .. } if *op == IntrinsicOp::MaskStoreuEpi8_128)),
            "non-all-off store must survive"
        );
    }
    // A wrong-arity or dest-form intrinsic is not the canonical PtrStore.
    let mut wrong = mask_store_function(
        IntrinsicOp::MaskStoreuEpi8_128,
        Operand::Const(IrConst::I64(0)),
    );
    for inst in &mut wrong.blocks[0].instructions {
        if let Instruction::Intrinsic { args, .. } = inst {
            args.push(Operand::Const(IrConst::I64(0)));
        }
    }
    let facts = Facts::new(&wrong);
    assert!(!fold_removal(&facts, &wrong.blocks[0].instructions[1]));
}

#[test]
fn masked_candidate_gate_pays_nothing_for_unsupported_ops() {
    // `has_candidate` must accept exactly the shapes with a consumer.  A
    // function containing only an unsupported masked LOAD (the dword/qword
    // 512-bit siblings, which have no proven unmasked form in this pass)
    // must not even build `Facts` (edit count 0), while a supported one must
    // fold.  Round 8 made the BYTE 512-bit form supported, so the negative
    // case now uses `MaskzLoaduEpi32_512`.
    let unsupported = maskz_function(
        IntrinsicOp::MaskzLoaduEpi32_512,
        Operand::Const(IrConst::I64(0)),
        64,
        false,
        false,
    );
    let mut u = unsupported;
    assert_eq!(
        run_function(&mut u),
        0,
        "unsupported op must not trigger the pass"
    );
    let mut supported = maskz_function(
        IntrinsicOp::MaskzLoaduEpi8_128,
        Operand::Const(IrConst::I64(0)),
        16,
        false,
        false,
    );
    assert!(run_function(&mut supported) >= 1, "supported op must fold");
}

// =========================================================================
// Round 8, stage 1 — red-team tests for the allocation-free image engine and
// the exact integer-decoding soundness fix.  Every positive assertion is
// paired with a mutation that must refuse.
// =========================================================================

#[test]
fn byte_image_is_canonical_exact_and_allocation_free() {
    // Canonical tail: two images with the same visible bytes but different
    // hidden tail bytes cannot exist, because every constructor zero-fills.
    let a = ByteImage::from_slice(&[1, 2, 3]).unwrap();
    let b = ByteImage::from_slice(&[1, 2, 3]).unwrap();
    assert_eq!(a, b, "ByteImage equality is over [0..len) only");
    assert_eq!(a.len(), 3);
    assert_eq!(&*a, &[1, 2, 3]);
    assert_eq!(&a.bytes[3..], &[0u8; 61], "tail must be canonical zero");

    // Exact capacity: 64 bytes is ONE 512-bit register, 65 is out of domain.
    assert_eq!(ByteImage::zeros(MAX_IMAGE).unwrap().len(), MAX_IMAGE);
    assert!(ByteImage::zeros(MAX_IMAGE + 1).is_none());
    assert!(ByteImage::from_slice(&[0u8; MAX_IMAGE + 1]).is_none());
    assert_eq!(ByteImage::ones(64).unwrap().as_slice(), &[0xFFu8; 64]);

    // `filled` must not leak the fill byte into the canonical tail.
    let f = ByteImage::filled(3, 0x7E).unwrap();
    assert_eq!(f.as_slice(), &[0x7E, 0x7E, 0x7E]);
    assert_eq!(&f.bytes[3..], &[0u8; 61]);

    // `zip_map` must refuse a length mismatch instead of truncating the zip.
    let short = ByteImage::zeros(8).unwrap();
    assert!(
        ByteImage::zeros(16)
            .unwrap()
            .zip_map(&short, |a, b| a ^ b)
            .is_none()
    );
    let x = ByteImage::from_slice(&[0x0F, 0xF0]).unwrap();
    let y = ByteImage::from_slice(&[0xFF, 0x00]).unwrap();
    assert_eq!(
        x.zip_map(&y, |a, b| a & b).unwrap().as_slice(),
        &[0x0F, 0x00]
    );

    // `splat` refuses a lane that does not tile the width EXACTLY, and never
    // writes past the requested width (which would poison the canonical tail).
    assert_eq!(
        ByteImage::splat(&[0xAA, 0xBB], 6).unwrap().as_slice(),
        &[0xAA, 0xBB, 0xAA, 0xBB, 0xAA, 0xBB]
    );
    assert!(ByteImage::splat(&[1, 2, 3], 16).is_none(), "16 % 3 != 0");
    assert!(ByteImage::splat(&[], 16).is_none(), "zero-width lane");
    assert!(ByteImage::splat(&[1], MAX_IMAGE + 1).is_none());
    let s = ByteImage::splat(&[0x80], 3).unwrap();
    assert_eq!(s.as_slice(), &[0x80, 0x80, 0x80]);
    assert_eq!(&s.bytes[3..], &[0u8; 61]);

    // Exhaustive 64-byte splat/zip_map agreement with a scalar oracle.
    for width in [1usize, 2, 3, 4, 7, 8, 16, 31, 32, 63, 64] {
        let v: Vec<u8> = (0..width)
            .map(|i| (i as u8).wrapping_mul(37).wrapping_add(11))
            .collect();
        let img = ByteImage::from_slice(&v).unwrap();
        let want: Vec<u8> = v.iter().map(|b| !b).collect();
        let got = img.zip_map(&img, |a, b| !(a & b)).unwrap();
        assert_eq!(got.as_slice(), &want[..], "zip_map width {width}");
    }
}

#[test]
fn exact_i64_refuses_every_truncating_or_non_integer_constant() {
    // The pre-existing `IrConst::to_i64()` truncates I128 to its low 64 bits.
    // These are the EXACT values that used to be misread as 0 / 4 / -1.
    let truncating = [
        1i128 << 64,            // low 64 bits = 0  -> AllOff
        (1i128 << 64) | 4,      // low 64 bits = 4  -> "ne" predicate
        (1i128 << 64) | 0xFFFF, // low 64 bits = 0xFFFF -> AllOn
        -(1i128 << 64),         // low 64 bits = 0
        i128::from(i64::MAX) + 1,
        i128::from(i64::MIN) - 1,
        i128::MAX,
        i128::MIN,
    ];
    for v in truncating {
        assert_eq!(exact_i64(IrConst::I128(v)), None, "I128({v}) must refuse");
    }
    // In-range I128 values are still accepted EXACTLY (no regression).
    for v in [0i128, 1, -1, 4, 0xFFFF, i64::MAX as i128, i64::MIN as i128] {
        assert_eq!(exact_i64(IrConst::I128(v)), Some(v as i64), "I128({v})");
    }
    assert_eq!(exact_i64(IrConst::I8(-1)), Some(-1));
    assert_eq!(exact_i64(IrConst::I16(-300)), Some(-300));
    assert_eq!(exact_i64(IrConst::I32(i32::MIN)), Some(i32::MIN as i64));
    assert_eq!(exact_i64(IrConst::I64(i64::MAX)), Some(i64::MAX));
    assert_eq!(exact_i64(IrConst::Zero), Some(0));
    assert_eq!(exact_i64(IrConst::F32(-2.5)), None);
    assert_eq!(exact_i64(IrConst::F64(f64::INFINITY)), None);
    assert_eq!(exact_i64(IrConst::D32(7)), None);
    assert_eq!(exact_i64(IrConst::D64(7)), None);
    assert_eq!(exact_i64(IrConst::LongDouble(1.0, [0u8; 16])), None);
}

#[test]
fn ir_const_bytes_is_bit_exact_for_every_spelling() {
    assert_eq!(ir_const_bytes(IrConst::Zero).unwrap().as_slice(), &[0]);
    assert_eq!(ir_const_bytes(IrConst::I8(-1)).unwrap().as_slice(), &[0xFF]);
    assert_eq!(
        ir_const_bytes(IrConst::I16(-2)).unwrap().as_slice(),
        &[0xFE, 0xFF]
    );
    assert_eq!(
        ir_const_bytes(IrConst::I32(0x1234_5678))
            .unwrap()
            .as_slice(),
        &0x1234_5678i32.to_le_bytes()
    );
    assert_eq!(
        ir_const_bytes(IrConst::I64(-2)).unwrap().as_slice(),
        &(-2i64).to_le_bytes()
    );
    assert_eq!(
        ir_const_bytes(IrConst::I128(0x0102_0304_0506_0708_090A_0B0C_0D0E_0F10))
            .unwrap()
            .as_slice(),
        &0x0102_0304_0506_0708_090A_0B0C_0D0E_0F10i128.to_le_bytes()
    );
    // NaN payloads and -0.0 sign bits must survive bit-for-bit.
    assert_eq!(
        ir_const_bytes(IrConst::F32(-0.0)).unwrap().as_slice(),
        &(-0.0f32).to_bits().to_le_bytes()
    );
    assert_eq!(
        ir_const_bytes(IrConst::F64(f64::from_bits(0x7FF8_0000_1234_5678)))
            .unwrap()
            .as_slice(),
        &0x7FF8_0000_1234_5678u64.to_le_bytes()
    );
    assert_eq!(
        ir_const_bytes(IrConst::D32(0xDEAD_BEEF))
            .unwrap()
            .as_slice(),
        &0xDEAD_BEEFu32.to_le_bytes()
    );
    assert_eq!(
        ir_const_bytes(IrConst::D64(0x0123_4567_89AB_CDEF))
            .unwrap()
            .as_slice(),
        &0x0123_4567_89AB_CDEFu64.to_le_bytes()
    );
    let ld = [0x5Au8; 16];
    assert_eq!(
        ir_const_bytes(IrConst::LongDouble(1.5, ld))
            .unwrap()
            .as_slice(),
        &ld
    );
    // Agreement with the pre-existing encoder on the whole integer domain.
    for v in [0i64, 1, -1, 255, -128, i32::MAX as i64, i64::MIN, i64::MAX] {
        assert_eq!(
            ir_const_bytes(IrConst::I64(v)).unwrap().as_slice(),
            &IrConst::I64(v).to_le_bytes()[..],
            "I64({v})"
        );
    }
}

#[test]
fn scalar_mask_refuses_i128_constants_that_would_truncate_to_all_off() {
    let f = purity_function(vec![]);
    let facts = Facts::new(&f);
    // 1 << 64 has low 64 bits == 0.  The old `to_i64()` reported 0, i.e.
    // `AllOff`, which would have ERASED a masked store that actually has an
    // active lane at bit 64.  Refusing is the only sound verdict.
    for lanes in [16u32, 32, 64] {
        assert_eq!(
            facts.scalar_mask(Operand::Const(IrConst::I128(1i128 << 64)), lanes),
            ScalarMask::Unknown,
            "lanes {lanes}"
        );
        assert_eq!(
            facts.scalar_mask(Operand::Const(IrConst::I128((1i128 << 64) | 4)), lanes),
            ScalarMask::Unknown,
            "lanes {lanes}"
        );
        // An in-range I128 mask still classifies exactly.
        assert_eq!(
            facts.scalar_mask(Operand::Const(IrConst::I128(-1)), lanes),
            ScalarMask::AllOn
        );
        assert_eq!(
            facts.scalar_mask(Operand::Const(IrConst::I128(0)), lanes),
            ScalarMask::AllOff
        );
    }
    assert_eq!(
        facts.scalar_mask(Operand::Const(IrConst::F32(0.0)), 16),
        ScalarMask::Unknown
    );
    assert_eq!(
        facts.scalar_mask(Operand::Const(IrConst::I64(0)), 0),
        ScalarMask::Unknown
    );
    assert_eq!(
        facts.scalar_mask(Operand::Const(IrConst::I64(0)), 65),
        ScalarMask::Unknown
    );
}

#[test]
fn const_offset_refuses_i128_offsets_that_would_truncate_into_bounds() {
    // A GEP with offset 1<<64 could previously decode as offset 0 and point
    // the image proof at the WRONG bytes of the object: a miscompile.
    assert_eq!(
        Facts::const_offset(&Operand::Const(IrConst::I128(1i128 << 64))),
        None
    );
    assert_eq!(
        Facts::const_offset(&Operand::Const(IrConst::I128((1i128 << 64) + 8))),
        None
    );
    assert_eq!(
        Facts::const_offset(&Operand::Const(IrConst::I128(8))),
        Some(8)
    );
    assert_eq!(
        Facts::const_offset(&Operand::Const(IrConst::I64(-8))),
        Some(-8)
    );
    assert_eq!(
        Facts::const_offset(&Operand::Const(IrConst::F64(0.0))),
        None
    );
    assert_eq!(Facts::const_offset(&Operand::Value(Value(7))), None);
}

#[test]
fn compare_predicate_refuses_i128_whose_low_bits_look_like_ne() {
    let a = [0x81u8; 16];
    let mut b = a;
    b[3] = 0x01;
    // Low 64 bits == 4 would have been decoded as predicate `ne` by the old
    // truncating `to_i64()`; the real immediate is out of range.
    let bad = Operand::Const(IrConst::I128((1i128 << 64) | 4));
    let f = mc_function(IntrinsicOp::CmpEpi8Mask128, bad, &a, &b);
    assert_eq!(mc_folded(&f), None, "out-of-range predicate must not fold");

    // Positive control: the SAME function with an exact I64 predicate folds.
    let good = Operand::Const(IrConst::I64(4));
    let f2 = mc_function(IntrinsicOp::CmpEpi8Mask128, good, &a, &b);
    let want = mc_oracle(&a, &b, 1, 16, true);
    assert_eq!(
        mc_folded(&f2),
        Some(want as i64),
        "in-range must still fold"
    );

    // FP constants are never predicates.
    let fp = mc_function(
        IntrinsicOp::CmpEpi8Mask128,
        Operand::Const(IrConst::F32(0.0)),
        &a,
        &b,
    );
    assert_eq!(mc_folded(&fp), None);
}

#[test]
fn image_proof_supports_a_full_64_byte_window() {
    // The old engine refused `len > 32`, so NO 512-bit constant image could
    // ever be proven.  Build a 64-byte tiled object and prove the whole thing.
    let bytes: Vec<u8> = (0..64)
        .map(|i| (i as u8).wrapping_mul(7).wrapping_add(3))
        .collect();
    let base = Value(0);
    let mut insts = vec![alloca(base, 64, false, false)];
    for (i, b) in bytes.iter().enumerate() {
        let p = Value(100 + i as u32);
        insts.push(gep(p, base, Operand::Const(IrConst::I64(i as i64))));
        insts.push(byte_store(p, *b, false));
    }
    let read_index = insts.len();
    let mut f = IrFunction::new("wide".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
        true,
    ));
    f.next_value_id = 500;
    let facts = Facts::new(&f);
    let got = facts
        .image_of(Operand::Value(base), 64, 0, read_index)
        .expect("64-byte window must be provable");
    assert_eq!(got.len(), 64);
    assert_eq!(got.as_slice(), &bytes[..], "bit-exact 512-bit image");
    // A 65-byte window exceeds the fixed capacity: refuse, never truncate.
    assert!(
        facts
            .image_of(Operand::Value(base), 65, 0, read_index)
            .is_none()
    );
    // A 32-byte sub-window at a nonzero offset is still exact.
    let sub = facts
        .image_of(Operand::Value(base), 32, 0, read_index)
        .expect("32-byte prefix");
    assert_eq!(sub.as_slice(), &bytes[..32]);
}

#[test]
fn overlapping_writes_with_a_gap_still_refuse() {
    // Round 8's last-write-wins is sound ONLY because every byte in the
    // window must still be covered.  Remove one store -> gap -> refuse.
    let mut im = Image::new(&[0x80u8; 16]);
    let victim = im
        .insts
        .iter()
        .find_map(|inst| match inst {
            Instruction::GetElementPtr {
                dest,
                offset: Operand::Const(IrConst::I64(5)),
                ..
            } => Some(*dest),
            _ => None,
        })
        .expect("byte 5 GEP present");
    im.insts
        .retain(|i| !matches!(i, Instruction::Store { ptr, .. } if *ptr == victim));
    let mut f = im.function(false);
    assert_eq!(run_function(&mut f), 0, "a tiling gap must still refuse");
}

// =========================================================================
// Round 8, stage 1 — REGRESSION for the anchored-nested-proof defect.
//
//   src = [0x80..0x8F]        (16 constant byte stores)
//   dst = src                 (Memcpy, a 16-byte snapshot)
//   src[0] = 0x7F             (a write AFTER the copy, BEFORE the read)
//   return pmovmskb(dst)      (must be 0xFFFF, never 0xFFFE)
//
// The pre-existing `memcpy_mention` proved the source image at the OUTER
// read's program position (`w.read_index`), so the post-copy store was
// admitted into the snapshot.  The merged engine hid the defect by refusing
// any two writes to one byte; round 8's exact last-write-wins made it
// observable as a real miscompile (caught by the full regression corpus as
// `sret_copy_source_write_after`).  The fix anchors every nested proof at
// the audited instruction's OWN (block, index).
// =========================================================================

fn anchored_memcpy_function(post_copy: bool) -> (IrFunction, Value) {
    let src = Value(0);
    let dst = Value(1);
    let result = Value(500);
    let mut insts = vec![alloca(src, 16, false, false), alloca(dst, 16, false, false)];
    let mut next = 2u32;
    for i in 0..16u32 {
        let p = Value(next);
        next += 1;
        insts.push(gep(p, src, Operand::Const(IrConst::I64(i as i64))));
        insts.push(byte_store(p, 0x80u8 | i as u8, false));
    }
    insts.push(Instruction::Memcpy {
        dest: dst,
        src,
        size: 16,
    });
    if post_copy {
        // AFTER the copy but BEFORE the outer read.  This is the write that
        // the pre-existing unanchored nested proof wrongly admitted into the
        // copied snapshot.
        let p = Value(next);
        next += 1;
        insts.push(gep(p, src, Operand::Const(IrConst::I64(0))));
        insts.push(byte_store(p, 0x7F, false));
    }
    insts.push(intrinsic(
        Some(result),
        IntrinsicOp::Pmovmskb128,
        None,
        vec![Operand::Value(dst)],
    ));
    let mut f = IrFunction::new("anchored_memcpy".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Value(result))),
        true,
    ));
    f.next_value_id = next.max(501);
    (f, result)
}

#[test]
fn memcpy_snapshot_is_anchored_at_the_copy_not_the_outer_read() {
    // Positive control: without the later write the copy IS provable, so a
    // blanket refusal would fail this assertion.  This keeps "just refuse
    // everything" from masquerading as a fix.
    let (mut plain, r) = anchored_memcpy_function(false);
    assert_eq!(
        movemask_result_after(&mut plain, r),
        Some(0xFFFF),
        "all 16 bytes have their MSB set; the plain copy must fold"
    );

    // The stale-write case must NEVER fold to the post-copy value.  Sound
    // choices are `Some(0xFFFF)` (precise proof) or `None` (conservative
    // refusal); `Some(0xFFFE)` is the miscompile.
    let (mut stale, r2) = anchored_memcpy_function(true);
    let got = movemask_result_after(&mut stale, r2);
    assert_ne!(
        got,
        Some(0xFFFE),
        "a write to src AFTER the copy changed the copied snapshot"
    );
    assert!(
        got == Some(0xFFFF) || got.is_none(),
        "fold must be exact or refused, got {got:?}"
    );

    // Stronger than the result oracle: no instruction may have been rewritten
    // to the wrong constant at all, including the load-of-constant consumer.
    for inst in &stale.blocks[0].instructions {
        if let Instruction::Copy {
            src: Operand::Const(IrConst::I32(v)),
            ..
        } = inst
        {
            assert_ne!(*v, 0xFFFE, "wrong constant materialized in the IR");
        }
    }
}

#[test]
fn memcpy_snapshot_includes_writes_that_precede_the_copy() {
    // Ordering positive control: move the 0x7F store BEFORE the copy.  Then
    // the snapshot legitimately contains it and the exact last-write-wins
    // proof must produce 0xFFFE (not refuse, not 0xFFFF).
    let (mut f, r) = anchored_memcpy_function(false);
    let src = Value(0);
    let p = Value(480);
    let pre = vec![
        gep(p, src, Operand::Const(IrConst::I64(0))),
        byte_store(p, 0x7F, false),
    ];
    // Insert immediately before the Memcpy (the last-but-one instruction).
    let at = f.blocks[0]
        .instructions
        .iter()
        .position(|i| matches!(i, Instruction::Memcpy { .. }))
        .expect("memcpy present");
    f.blocks[0].instructions.splice(at..at, pre);
    // Keep spans parallel: `fold` and `prune_dead_private_writes` assert this.
    while f.blocks[0].source_spans.len() < f.blocks[0].instructions.len() {
        f.blocks[0].source_spans.push(Span::dummy());
    }
    assert_eq!(
        movemask_result_after(&mut f, r),
        Some(0xFFFE),
        "a write BEFORE the copy is part of the snapshot"
    );
}

// =========================================================================
// Round 8, stage 2 — ISPC `ReplaceMaskedMemOps` / `IntrinsicsOpt` / the
// generalised masked-family transplant.  Exhaustive positive sweeps plus a
// refusal mutation for EVERY rule.
// =========================================================================

/// One masked write-through intrinsic with a bounds-proven destination and a
/// separate bounds-proven source/merge object.
fn masked_family_function(
    op: IntrinsicOp,
    args: Vec<Operand>,
    size: usize,
    src_size: usize,
    used_dest: bool,
    volatile: bool,
) -> IrFunction {
    let mut insts = vec![
        alloca(Value(MC_A), size, volatile, false),
        alloca(Value(MC_B), src_size, false, false),
        intrinsic(Some(Value(MC_RESULT)), op, Some(Value(MC_A)), args),
    ];
    if used_dest {
        insts.push(copy(Value(2), Operand::Value(Value(MC_RESULT))));
    }
    let mut f = IrFunction::new("masked_family".into(), IrType::I32, vec![], false);
    f.blocks.push(block(
        insts,
        Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
        true,
    ));
    f.next_value_id = 1000;
    f
}

fn fold_masked(f: &IrFunction) -> Option<Instruction> {
    let facts = Facts::new(f);
    fold(&facts, &f.blocks[0].instructions[2], 0, 2)
}

/// `mask` + a raw pointer + an optional merge/value object.
fn two_arg(mask: Operand) -> Vec<Operand> {
    vec![mask, Operand::Value(Value(MC_B))]
}

#[test]
fn true_prefix_len_is_exact_over_the_whole_16_bit_domain() {
    // Exhaustive: all 65,536 16-bit masks.  `Some(n)` iff bits are exactly
    // `0..n`, i.e. `k == 2^n - 1` with 1 <= n <= 15 (n == 16 is AllOn).
    for k in 0u64..=0xFFFF {
        let expect = (1..16u32)
            .find(|n| k == (1u64 << n) - 1)
            .map(|n| n as usize);
        assert_eq!(
            true_prefix_len(k, 16, 0),
            expect,
            "prefix classification of {k:#06x}"
        );
        // ISPC's bounded-gap coalescing is NOT applicable when the mask is
        // dropped: any nonzero gap budget must be refused, for every k.
        assert_eq!(
            true_prefix_len(k, 16, 1),
            None,
            "gap budget must refuse {k:#06x}"
        );
    }
    // 32- and 64-lane domains: every prefix plus adversarial non-prefixes.
    for lanes in [8u32, 32, 64] {
        for n in 1..lanes {
            let k = (1u64 << n) - 1;
            assert_eq!(
                true_prefix_len(k, lanes, 0),
                Some(n as usize),
                "lanes {lanes} n {n}"
            );
        }
        for hole in 0..lanes.min(20) {
            // A hole in the TOP lane is itself a shorter true prefix, which
            // is a legitimate (and tested) narrowing; skip that one case.
            if hole == lanes - 1 {
                continue;
            }
            let holed = lane_mask(lanes) & !(1u64 << hole);
            assert_eq!(
                true_prefix_len(holed, lanes, 0),
                None,
                "hole at {hole} must refuse (lanes {lanes})"
            );
        }
    }
    assert_eq!(true_prefix_len(0, 16, 0), None, "all-off is not a prefix");
    assert_eq!(
        true_prefix_len(u32::MAX as u64, 16, 0),
        None,
        "truncation guard"
    );
    assert_eq!(
        true_prefix_len(0xFFFF, 16, 0),
        None,
        "all-on is not a prefix"
    );
    assert_eq!(true_prefix_len(1, 0, 0), None, "zero lanes");
    assert_eq!(true_prefix_len(1, 65, 0), None, "over-wide lanes");
}

#[test]
fn masked_form_table_and_zero_producers_are_consistent() {
    assert_eq!(zero_producer_op(16), Some(IntrinsicOp::Setzero128));
    assert_eq!(zero_producer_op(32), Some(IntrinsicOp::Setzero256));
    assert_eq!(zero_producer_op(64), Some(IntrinsicOp::Setzero512));
    for bad in [0usize, 1, 8, 15, 17, 31, 33, 63, 65, 128] {
        assert_eq!(zero_producer_op(bad), None, "width {bad}");
    }
    // Every form's result width must have an exact zero producer, and its
    // lane count must be a real AVX-512 mask width (8/16/32/64).
    use IntrinsicOp as O;
    for op in [
        O::MaskzLoaduEpi8_128,
        O::MaskzLoaduEpi8_256,
        O::MaskzLoaduEpi8_512,
        O::MaskLoaduEpi8_128,
        O::MaskLoaduEpi8_256,
        O::MaskLoaduEpi8_512,
        O::MaskStoreuEpi8_128,
        O::MaskStoreuEpi8_256,
        O::MaskStoreuEpi8_512,
        O::MaskzSet1Epi16_512,
        O::MaskzSet1Epi32_512,
        O::MaskzSet1Epi64x512,
        O::MaskzMaddubsEpi16_512,
        O::MaskzShuffleEpi8_128,
        O::MaskShuffleEpi8_128,
        O::MaskzInsertI64x2,
        O::MaskzInsertI32x4,
        O::MaskzExtractI32x4,
        O::MaskzExtractI64x4,
        O::MaskzExtractI64x2,
    ] {
        let (mask_arg, arity, lanes, width, _) =
            masked_form(op).unwrap_or_else(|| panic!("{op:?} must be in the form table"));
        assert_eq!(mask_arg, 0, "{op:?} mask slot");
        assert!(arity >= 2 && arity <= 4, "{op:?} arity");
        assert!(matches!(lanes, 8 | 16 | 32 | 64), "{op:?} lanes");
        assert!(
            zero_producer_op(width).is_some(),
            "{op:?} result width {width}"
        );
        assert!(masked_fold_candidate(op), "{op:?} gate");
    }
    for op in [
        O::MaskzLoaduEpi32_512,
        O::MaskzLoaduEpi64_512,
        O::Pmovmskb128,
        O::Setzero128,
    ] {
        assert!(!masked_fold_candidate(op), "{op:?} must not enter the pass");
    }
    // dest_ptr-zero producer widths are exact on the 512-bit form.
    assert_eq!(dest_ptr_zero_width(O::Setzero512), Some(64));
    assert_eq!(zero_vector_width(O::Setzero512), Some(64));
}

#[test]
fn all_off_zeroing_forms_emit_an_exact_width_zero_producer_op() {
    use IntrinsicOp as O;
    // (op, arity-built args, result width)
    let cases: Vec<(O, Vec<Operand>, usize)> = vec![
        (
            O::MaskzLoaduEpi8_512,
            two_arg(Operand::Const(IrConst::I64(0))),
            64,
        ),
        (
            O::MaskzSet1Epi16_512,
            two_arg(Operand::Const(IrConst::I64(0))),
            64,
        ),
        (
            O::MaskzSet1Epi32_512,
            two_arg(Operand::Const(IrConst::I64(0))),
            64,
        ),
        (
            O::MaskzSet1Epi64x512,
            two_arg(Operand::Const(IrConst::I64(0))),
            64,
        ),
        (
            O::MaskzMaddubsEpi16_512,
            vec![
                Operand::Const(IrConst::I64(0)),
                Operand::Value(Value(MC_B)),
                Operand::Value(Value(MC_B)),
            ],
            64,
        ),
        (
            O::MaskzShuffleEpi8_128,
            vec![
                Operand::Const(IrConst::I64(0)),
                Operand::Value(Value(MC_B)),
                Operand::Value(Value(MC_B)),
            ],
            16,
        ),
        (
            O::MaskzInsertI64x2,
            vec![
                Operand::Const(IrConst::I64(0)),
                Operand::Value(Value(MC_B)),
                Operand::Value(Value(MC_B)),
                Operand::Const(IrConst::I64(0)),
            ],
            64,
        ),
        (
            O::MaskzExtractI32x4,
            vec![
                Operand::Const(IrConst::I64(0)),
                Operand::Value(Value(MC_B)),
                Operand::Const(IrConst::I64(0)),
            ],
            16,
        ),
        (
            O::MaskzExtractI64x4,
            vec![
                Operand::Const(IrConst::I64(0)),
                Operand::Value(Value(MC_B)),
                Operand::Const(IrConst::I64(0)),
            ],
            32,
        ),
    ];
    let want = |w: usize| match w {
        16 => O::Setzero128,
        32 => O::Setzero256,
        64 => O::Setzero512,
        _ => unreachable!(),
    };
    for (op, args, width) in cases {
        let f = masked_family_function(op, args.clone(), width, 64, false, false);
        match fold_masked(&f).unwrap_or_else(|| panic!("{op:?} all-off must fold")) {
            Instruction::Intrinsic {
                dest: None,
                op: got,
                dest_ptr: Some(dp),
                args: out,
            } => {
                assert_eq!(got, want(width), "{op:?} zero producer width");
                assert_eq!(dp, Value(MC_A));
                assert!(out.is_empty(), "{op:?} zero producer takes no operands");
            }
            other => panic!("{op:?} unexpected rewrite {other:?}"),
        }
        // Mutation: a too-small destination must refuse.
        let small = masked_family_function(op, args.clone(), width / 2, 64, false, false);
        assert!(fold_masked(&small).is_none(), "{op:?} too-small dest");
        // Mutation: a volatile destination must refuse.
        let vol = masked_family_function(op, args, width, 64, false, true);
        assert!(fold_masked(&vol).is_none(), "{op:?} volatile dest");
    }
}

#[test]
fn merge_forms_copy_the_old_vector_when_all_off() {
    use IntrinsicOp as O;
    let cases: Vec<(O, Vec<Operand>, usize)> = vec![
        (
            O::MaskLoaduEpi8_128,
            vec![
                Operand::Const(IrConst::I64(0)),
                Operand::Value(Value(MC_B)), // raw pointer
                Operand::Value(Value(MC_B)), // old
            ],
            16,
        ),
        (
            O::MaskLoaduEpi8_256,
            vec![
                Operand::Const(IrConst::I64(0)),
                Operand::Value(Value(MC_B)),
                Operand::Value(Value(MC_B)),
            ],
            32,
        ),
        (
            O::MaskLoaduEpi8_512,
            vec![
                Operand::Const(IrConst::I64(0)),
                Operand::Value(Value(MC_B)),
                Operand::Value(Value(MC_B)),
            ],
            64,
        ),
        (
            O::MaskShuffleEpi8_128,
            vec![
                Operand::Const(IrConst::I64(0)),
                Operand::Value(Value(MC_B)), // merge source src
                Operand::Value(Value(MC_B)), // a
                Operand::Value(Value(MC_B)), // b
            ],
            16,
        ),
    ];
    for (op, args, width) in cases {
        let f = masked_family_function(op, args.clone(), width, 64, false, false);
        match fold_masked(&f).unwrap_or_else(|| panic!("{op:?} all-off merge must fold")) {
            Instruction::Memcpy { dest, src, size } => {
                assert_eq!(dest, Value(MC_A), "{op:?} dest");
                assert_eq!(src, Value(MC_B), "{op:?} merge source");
                assert_eq!(size, width, "{op:?} exact copy width");
            }
            other => panic!("{op:?} unexpected rewrite {other:?}"),
        }
        // Mutation: an under-sized merge source cannot supply `width` bytes.
        let short = masked_family_function(op, args.clone(), width, width / 2, false, false);
        assert!(fold_masked(&short).is_none(), "{op:?} short merge source");
        // Mutation: a used SSA dest cannot be dropped.
        let used = masked_family_function(op, args, width, 64, true, false);
        assert!(fold_masked(&used).is_none(), "{op:?} used SSA dest");
    }
    // Same-base `memcpy` is UB; the pass must refuse it.
    let alias = masked_family_function(
        O::MaskLoaduEpi8_128,
        vec![
            Operand::Const(IrConst::I64(0)),
            Operand::Value(Value(MC_A)),
            Operand::Value(Value(MC_A)),
        ],
        16,
        64,
        false,
        false,
    );
    assert!(
        fold_masked(&alias).is_none(),
        "self-overlapping copy must refuse"
    );
}

#[test]
fn all_on_loads_drop_the_mask_only_where_a_raw_pointer_load_is_proven() {
    use IntrinsicOp as O;
    for (op, width, want) in [
        (O::MaskzLoaduEpi8_128, 16usize, O::Loaddqu),
        (O::MaskLoaduEpi8_128, 16, O::Loaddqu),
        (O::MaskzLoaduEpi8_512, 64, O::Loadu512),
        (O::MaskLoaduEpi8_512, 64, O::Loadu512),
    ] {
        let lanes = width as u32;
        let mask = Operand::Const(IrConst::I64(-1));
        let args = if width == 16 || width == 64 {
            match op {
                O::MaskLoaduEpi8_128 | O::MaskLoaduEpi8_512 => vec![
                    mask,
                    Operand::Value(Value(MC_B)),
                    Operand::Value(Value(MC_B)),
                ],
                _ => vec![mask, Operand::Value(Value(MC_B))],
            }
        } else {
            unreachable!()
        };
        let f = masked_family_function(op, args, width, 64, false, false);
        match fold_masked(&f).unwrap_or_else(|| panic!("{op:?} all-on must fold (lanes {lanes})")) {
            Instruction::Intrinsic {
                dest: None,
                op: got,
                dest_ptr: Some(dp),
                args: out,
            } => {
                assert_eq!(got, want, "{op:?} unmasked load");
                assert_eq!(dp, Value(MC_A));
                assert_eq!(out.len(), 1, "{op:?} keeps only the raw pointer");
                assert!(matches!(out[0], Operand::Value(_)));
            }
            other => panic!("{op:?} unexpected rewrite {other:?}"),
        }
    }
    // 256-bit all-on folds to the DEDICATED raw-pointer load.  `Loadu256`
    // reads a vector HOME and is never used: that was the round-7 blocker,
    // resolved by an emitter that dereferences `(ptr)` directly.
    for op in [O::MaskzLoaduEpi8_256, O::MaskLoaduEpi8_256] {
        let args = if op == O::MaskLoaduEpi8_256 {
            vec![
                Operand::Const(IrConst::I64(-1)),
                Operand::Value(Value(MC_B)),
                Operand::Value(Value(MC_B)),
            ]
        } else {
            vec![
                Operand::Const(IrConst::I64(-1)),
                Operand::Value(Value(MC_B)),
            ]
        };
        let f = masked_family_function(op, args, 32, 64, false, false);
        match fold_masked(&f).unwrap_or_else(|| panic!("{op:?} 256 all-on must fold")) {
            Instruction::Intrinsic { op: got, args, .. } => {
                assert_eq!(got, O::Loadu256Raw, "{op:?}");
                assert_ne!(got, O::Loadu256, "{op:?} must not use the home-based load");
                assert_eq!(args.len(), 1, "{op:?} keeps only the raw pointer");
            }
            other => panic!("{op:?} unexpected rewrite {other:?}"),
        }
    }
}

#[test]
fn all_on_masked_arithmetic_maps_to_the_exact_unmasked_op() {
    use IntrinsicOp as O;
    let b = Operand::Value(Value(MC_B));
    let one = Operand::Const(IrConst::I64(-1));
    let imm = Operand::Const(IrConst::I64(0));
    for (op, args, want, want_args) in [
        (
            O::MaskzSet1Epi16_512,
            vec![one, b],
            O::SetEpi16_512,
            vec![b],
        ),
        (
            O::MaskzSet1Epi32_512,
            vec![one, b],
            O::SetEpi32_512,
            vec![b],
        ),
        (
            O::MaskzSet1Epi64x512,
            vec![one, b],
            O::SetEpi64x512,
            vec![b],
        ),
        (
            O::MaskzMaddubsEpi16_512,
            vec![one, b, b],
            O::Pmaddubsw512,
            vec![b, b],
        ),
        (
            O::MaskzShuffleEpi8_128,
            vec![one, b, b],
            O::Pshufb128,
            vec![b, b],
        ),
        (
            O::MaskShuffleEpi8_128,
            vec![one, b, b, b],
            O::Pshufb128,
            vec![b, b],
        ),
    ] {
        let f = masked_family_function(op, args, 64, 64, false, false);
        match fold_masked(&f).unwrap_or_else(|| panic!("{op:?} all-on must fold")) {
            Instruction::Intrinsic {
                dest: None,
                op: got,
                dest_ptr: Some(dp),
                args: out,
            } => {
                assert_eq!(got, want, "{op:?} unmasked counterpart");
                assert_eq!(dp, Value(MC_A));
                assert_eq!(out, want_args, "{op:?} operand order");
            }
            other => panic!("{op:?} unexpected rewrite {other:?}"),
        }
    }
    // `ZeroOnly` insert/extract families deliberately refuse all-on: their
    // mask lane count is not their result lane count, and the immediate
    // selects a lane rather than being droppable.
    for (op, args, width) in [
        (O::MaskzInsertI64x2, vec![one, b, b, imm], 64usize),
        (O::MaskzInsertI32x4, vec![one, b, b, imm], 64),
        (O::MaskzExtractI32x4, vec![one, b, imm], 16),
        (O::MaskzExtractI64x4, vec![one, b, imm], 32),
        (O::MaskzExtractI64x2, vec![one, b, imm], 16),
    ] {
        let f = masked_family_function(op, args, width, 64, false, false);
        assert!(fold_masked(&f).is_none(), "{op:?} all-on must refuse");
    }
}

#[test]
fn constant_true_prefix_masked_stores_narrow_to_an_unmasked_copy() {
    use IntrinsicOp as O;
    for (op, width) in [
        (O::MaskStoreuEpi8_128, 16usize),
        (O::MaskStoreuEpi8_256, 32),
        (O::MaskStoreuEpi8_512, 64),
    ] {
        // Every strict prefix 1..width-1 becomes an exact n-byte Memcpy.
        for n in 1..width {
            let k = (1u64 << n) - 1;
            let f = masked_family_function(
                op,
                vec![
                    Operand::Const(IrConst::I64(k as i64)),
                    Operand::Value(Value(MC_B)),
                ],
                width,
                64,
                false,
                false,
            );
            match fold_masked(&f).unwrap_or_else(|| panic!("{op:?} prefix {n} must fold")) {
                Instruction::Memcpy { dest, src, size } => {
                    assert_eq!(dest, Value(MC_A));
                    assert_eq!(src, Value(MC_B));
                    assert_eq!(size, n, "{op:?} narrowed copy width");
                }
                other => panic!("{op:?} prefix {n} unexpected {other:?}"),
            }
        }
        // All-on becomes a FULL-width unmasked copy.
        let on = masked_family_function(
            op,
            vec![
                Operand::Const(IrConst::I64(-1)),
                Operand::Value(Value(MC_B)),
            ],
            width,
            64,
            false,
            false,
        );
        assert!(matches!(
            fold_masked(&on),
            Some(Instruction::Memcpy { size, .. }) if size == width
        ));
        // Non-prefix mixed masks refuse (the hardware leaves those bytes
        // untouched, so an unmasked copy would clobber them).
        for hole in [1u32, 3, 7] {
            // Use the production helper rather than re-deriving the mask:
            // `1u64 << width` overflows at width == 64. With overflow-checks it
            // panics; in release it wraps to `1 << 0`, so the mask became 0 and
            // the "must refuse" assertion below passed for the wrong reason.
            let k = lane_mask(width as u32) & !(1u64 << hole);
            let f = masked_family_function(
                op,
                vec![
                    Operand::Const(IrConst::I64(k as i64)),
                    Operand::Value(Value(MC_B)),
                ],
                width,
                64,
                false,
                false,
            );
            assert!(
                fold_masked(&f).is_none(),
                "{op:?} mask with hole {hole} must refuse"
            );
        }
        // A too-narrow VALUE object cannot supply n bytes.
        let short = masked_family_function(
            op,
            vec![Operand::Const(IrConst::I64(7)), Operand::Value(Value(MC_B))],
            width,
            2,
            false,
            false,
        );
        assert!(fold_masked(&short).is_none(), "{op:?} short value object");
    }
}

#[test]
fn masked_folds_refuse_every_unproven_mask_and_shape() {
    use IntrinsicOp as O;
    let good = vec![Operand::Const(IrConst::I64(0)), Operand::Value(Value(MC_B))];
    let table = [
        (O::MaskzLoaduEpi8_128, 16usize),
        (O::MaskzLoaduEpi8_256, 32),
        (O::MaskzLoaduEpi8_512, 64),
        (O::MaskStoreuEpi8_128, 16),
        (O::MaskStoreuEpi8_256, 32),
        (O::MaskStoreuEpi8_512, 64),
    ];
    let bad_masks = [
        Operand::Value(Value(999)),                 // runtime
        Operand::Const(IrConst::I128(1i128 << 64)), // truncating I128
        Operand::Const(IrConst::F32(0.0)),          // FP
        Operand::Const(IrConst::D64(0)),            // decimal bit container
    ];
    for (op, width) in table {
        for m in bad_masks {
            let f = masked_family_function(
                op,
                vec![m, Operand::Value(Value(MC_B))],
                width,
                64,
                false,
                false,
            );
            assert!(fold_masked(&f).is_none(), "{op:?} must refuse {m:?}");
        }
        // Wrong arity.
        let mut args = good.clone();
        args.push(Operand::Const(IrConst::I64(0)));
        let wrong = masked_family_function(op, args, width, 64, false, false);
        assert!(fold_masked(&wrong).is_none(), "{op:?} wrong arity");
        // Used SSA dest.
        let used = masked_family_function(op, good.clone(), width, 64, true, false);
        assert!(fold_masked(&used).is_none(), "{op:?} used SSA dest");
        // Too-small destination.
        let small = masked_family_function(op, good.clone(), width / 2, 64, false, false);
        assert!(fold_masked(&small).is_none(), "{op:?} too-small dest");
        // Volatile destination.
        let vol = masked_family_function(op, good.clone(), width, 64, false, true);
        assert!(fold_masked(&vol).is_none(), "{op:?} volatile dest");
    }
}

#[test]
fn mask_lane_truncation_is_the_safety_critical_property() {
    // `scalar_mask` truncates the raw constant to `lanes` bits, matching the
    // hardware `k` register.  A lane count that is TOO LARGE would treat a
    // mask with an active high lane as `Mixed` (safe) but a lane count that
    // is TOO SMALL could turn it into `AllOff` and ERASE a store.  Verify the
    // exact boundary for every supported mask width.
    let f = purity_function(vec![]);
    let facts = Facts::new(&f);
    for lanes in [8u32, 16, 32, 64] {
        let full = lane_mask(lanes);
        assert_eq!(
            facts.scalar_mask(Operand::Const(IrConst::I64(-1)), lanes),
            ScalarMask::AllOn,
            "lanes {lanes}"
        );
        assert_eq!(
            facts.scalar_mask(Operand::Const(IrConst::I64(0)), lanes),
            ScalarMask::AllOff,
            "lanes {lanes}"
        );
        // The first bit ABOVE the mask width is ignored by hardware.
        if lanes < 64 {
            let above = 1i128 << lanes;
            assert_eq!(
                facts.scalar_mask(Operand::Const(IrConst::I128(above)), lanes),
                ScalarMask::AllOff,
                "bit {lanes} is outside the k register"
            );
            assert_eq!(
                facts.scalar_mask(Operand::Const(IrConst::I128(above | 1)), lanes),
                ScalarMask::Mixed(1),
                "bit {lanes} must not hide lane 0"
            );
        }
        assert_eq!(full.count_ones(), lanes);
    }
}

// =========================================================================
// Round 8, stage 3 — "widen-then-average" (`pavgb`/`vpavgb`) constant-image
// transplant.  Exhaustive over all 65,536 byte pairs, plus refusal probes.
// =========================================================================

#[test]
fn widen_then_average_is_exact_for_every_byte_pair() {
    // Independent scalar oracle: widen to u16, add, round half UP, narrow.
    for a in 0u16..=255 {
        for b in 0u16..=255 {
            let want = ((a + b + 1) / 2) as u8;
            assert_eq!(ByteOp::Avg.apply(a as u8, b as u8), want, "avg({a},{b})");
            // Commutativity is a precondition of `combine_byte_logic`.
            assert_eq!(ByteOp::Avg.apply(b as u8, a as u8), want);
        }
    }
    // The widening is what makes 0xFF+0xFF+1 safe: a u8 add would wrap to
    // 0xFF+0xFF = 0xFE, +1 = 0xFF, /2 = 0x7F instead of 0xFF.
    assert_eq!(ByteOp::Avg.apply(0xFF, 0xFF), 0xFF);
    assert_eq!(ByteOp::Avg.apply(0, 0), 0);
    assert_eq!(ByteOp::Avg.apply(0, 1), 1, "round half UP");
    assert_eq!(ByteOp::Avg.apply(0xFE, 0xFF), 0xFF);

    let width = 16;
    let a: Vec<u8> = (0..width).map(|i| (i * 17 + 3) as u8).collect();
    let b: Vec<u8> = (0..width).map(|i| (i * 5 + 200) as u8).collect();
    let ia = bi(a.clone());
    let ib = bi(b.clone());
    let want: Vec<u8> = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| ((*x as u16 + *y as u16 + 1) / 2) as u8)
        .collect();
    assert_eq!(
        combine_byte_logic(ByteOp::Avg, width, Some(ia), Some(ib), false)
            .expect("two constant images must combine")
            .as_slice(),
        &want[..]
    );
    // `x avg x == x` is the algebraic identity; refuse when no image exists.
    assert_eq!(
        combine_byte_logic(ByteOp::Avg, width, Some(ia), None, true),
        Some(ia)
    );
    assert_eq!(
        combine_byte_logic(ByteOp::Avg, width, None, None, true),
        None
    );
    // A single known image with the other operand runtime-valued is NOT an
    // absorbing constant for Avg: there is no byte c with `x avg c` constant.
    for known in [bi(vec![0u8; width]), bi(vec![0xFFu8; width]), ia] {
        assert_eq!(
            combine_byte_logic(ByteOp::Avg, width, Some(known), None, false),
            None
        );
        assert_eq!(
            combine_byte_logic(ByteOp::Avg, width, None, Some(known), false),
            None
        );
    }
    // Width mismatch refuses rather than truncating.
    assert_eq!(
        combine_byte_logic(
            ByteOp::Avg,
            width,
            Some(bi(vec![1u8; 8])),
            Some(bi(vec![2u8; 8])),
            false
        ),
        None
    );
    // The 64-byte AVX-512 form is inside MAX_IMAGE and exact.
    let a64: Vec<u8> = (0..64).map(|i| (i * 7) as u8).collect();
    let b64: Vec<u8> = (0..64).map(|i| (255 - i * 3) as u8).collect();
    let want64: Vec<u8> = a64
        .iter()
        .zip(b64.iter())
        .map(|(x, y)| ((*x as u16 + *y as u16 + 1) / 2) as u8)
        .collect();
    assert_eq!(
        combine_byte_logic(ByteOp::Avg, 64, Some(bi(a64)), Some(bi(b64)), false)
            .expect("64-byte avg")
            .as_slice(),
        &want64[..]
    );
}

#[test]
fn average_shape_table_is_byte_local_only() {
    assert_eq!(
        byte_logic_shape(IntrinsicOp::Pavgb128),
        Some((16, ByteOp::Avg))
    );
    assert_eq!(
        byte_logic_shape(IntrinsicOp::Pavgb512),
        Some((64, ByteOp::Avg))
    );
    // WORD-wise averages carry between bytes and are deliberately refused.
    assert_eq!(byte_logic_shape(IntrinsicOp::Pavgw128), None);
    assert_eq!(byte_logic_shape(IntrinsicOp::Pavgw512), None);
    // Non-commutative / saturating / lane-width-dependent ops stay refused.
    for op in [
        IntrinsicOp::Pcmpgtb128,
        IntrinsicOp::Paddsb128,
        IntrinsicOp::Paddusb128,
        IntrinsicOp::Pmaddubsw128,
    ] {
        assert_eq!(byte_logic_shape(op), None, "{op:?} must not be byte-local");
    }
}
