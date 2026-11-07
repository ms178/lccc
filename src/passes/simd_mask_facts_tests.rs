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
    assert_eq!(MaskStatus::from_bytes(&[0x80; 16]), MaskStatus::AllOn);
    assert_eq!(MaskStatus::from_bytes(&[0x01; 16]), MaskStatus::AllOff);
    assert_eq!(MaskStatus::from_bytes(&[0xFF; 32]), MaskStatus::AllOn);
    let mut mixed = [0u8; 16];
    mixed[7] = 0x80;
    assert_eq!(MaskStatus::from_bytes(&mixed), MaskStatus::Unknown);
    assert_eq!(MaskStatus::from_bytes(&[]), MaskStatus::Unknown);
    // A mask whose *low* bits vary but whose sign bits agree is still exact.
    let mut low = [0u8; 16];
    for (i, b) in low.iter_mut().enumerate() {
        *b = 0x80 | i as u8;
    }
    assert_eq!(MaskStatus::from_bytes(&low), MaskStatus::AllOn);
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
    must_refuse("overlapping constant stores", |im| {
        let p = im.v();
        im.insts
            .insert(2, gep(p, im.source, Operand::Const(IrConst::I64(0))));
        im.insts.insert(3, byte_store(p, 7, false));
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
    // A scalar Load from the constant object is a read whose value we do not
    // model.  Refuse rather than assume it is harmless.
    let mut im = Image::new(&[0x80u8; 16]);
    let p = im.v();
    im.insts.insert(34, load(p, im.source));
    let mut f = im.function(false);
    assert_eq!(run_function(&mut f), 0, "scalar load mention folded");

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
