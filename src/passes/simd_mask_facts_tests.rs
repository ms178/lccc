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
    // A NON-VOLATILE scalar Load is a pure read: no IrConst can encode a
    // stack address, so a value loaded from a constant object cannot be
    // turned back into an aliasing pointer.  It must NOT veto the image.
    let mut im = Image::new(&[0x80u8; 16]);
    let p = im.v();
    im.insts.insert(34, load(p, im.source));
    let mut f = im.function(false);
    assert_eq!(run_function(&mut f), 1, "a pure scalar read must not veto");

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
    assert_eq!(const_store_bytes(IrConst::I64(255), 1), Some(vec![0xFF]));
    assert_eq!(const_store_bytes(IrConst::I64(-1), 1), Some(vec![0xFF]));
    assert_eq!(const_store_bytes(IrConst::I64(0), 1), Some(vec![0]));
    assert_eq!(
        const_store_bytes(IrConst::I32(-1), 2),
        Some(vec![0xFF, 0xFF])
    );
    assert_eq!(const_store_bytes(IrConst::Zero, 4), Some(vec![0; 4]));
    // Exact-width float and integer constants are unchanged.
    assert_eq!(
        const_store_bytes(IrConst::F32(-0.0), 4),
        Some(vec![0, 0, 0, 0x80])
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
    assert_eq!(run_function(&mut f), 1);
    assert_eq!(result_const(&f, Value(13)), Some(53069));
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
    assert_eq!(run_function(&mut off), 1);
    assert_eq!(result_const(&off, Value(13)), Some(0));
    let mut on = i686_aggregate_image(&[0x80u8; 16], None);
    assert_eq!(run_function(&mut on), 1);
    assert_eq!(result_const(&on, Value(13)), Some(0xFFFF));
}

#[test]
fn i686_aggregate_copy_refuses_unproven_or_mismatched_loads() {
    fn refuse(name: &str, mutate: fn(&mut Vec<Instruction>)) {
        let bytes = [0x80u8; 16];
        let mut f = i686_aggregate_image(&bytes, Some(mutate));
        assert_eq!(run_function(&mut f), 0, "negative I128 case {name} folded");
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
        run_function(&mut two),
        1,
        "a two-link Copy chain must resolve"
    );
    assert_eq!(result_const(&two, Value(13)), Some(0xFFFF));

    // A Copy cycle cannot resolve to a Load: refuse.
    let mut cycle = i686_aggregate_image(
        &bytes,
        Some(|insts| {
            insts.insert(8, copy(Value(14), Operand::Value(Value(14))));
        }),
    );
    assert_eq!(run_function(&mut cycle), 0, "a self Copy cycle must refuse");

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
    assert_eq!(run_function(&mut konst), 0, "a constant Copy must refuse");
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
