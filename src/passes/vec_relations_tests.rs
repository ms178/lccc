use super::*;
use crate::ir::reexports::{BasicBlock, BlockId, IrConst, Terminator};
fn v(id: u32) -> Operand {
    Operand::Value(Value(id))
}
fn function(code: Vec<Instruction>) -> IrFunction {
    let mut f = IrFunction::new("packed_relations".into(), IrType::Void, vec![], false);
    f.next_value_id = 64;
    f.blocks = vec![BasicBlock {
        label: BlockId(0),
        instructions: code,
        source_spans: vec![],
        terminator: Terminator::Return(None),
    }];
    f
}
fn evaluate(
    code: &[Instruction],
    f: Family,
    bits: usize,
    input: &[Vec<u64>; 4],
    bit_masks: bool,
) -> Vec<u64> {
    let full = u64::MAX >> (64 - bits);
    let mut values = FxHashMap::default();
    for (i, value) in input.iter().enumerate() {
        values.insert(Value(i as u32), value.clone());
    }
    let mut last = Value(0);
    for inst in code {
        let dest = inst.dest().unwrap();
        last = dest;
        let data = match inst {
            Instruction::Copy {
                src: Operand::Value(src),
                ..
            } => values[src].clone(),
            Instruction::Intrinsic { op, args, .. } => {
                if *op == f.zero {
                    vec![0; input[0].len()]
                } else if *op == f.broadcast {
                    let Operand::Const(value) = args[0] else {
                        panic!("bad splat")
                    };
                    vec![value.to_i128().unwrap() as u64 & full; input[0].len()]
                } else {
                    let get = |index: usize| match args[index] {
                        Operand::Value(value) => values[&value].clone(),
                        Operand::Const(c) => {
                            vec![c.to_i128().unwrap() as u64 & full; input[0].len()]
                        }
                    };
                    let a = get(0);
                    let b = get(1);
                    if blend_shape(*op).is_some() {
                        let mask = get(2);
                        a.iter()
                            .zip(&b)
                            .zip(mask)
                            .map(|((&a, &b), m)| {
                                if bit_masks {
                                    (a & !m) | (b & m)
                                } else if m & (1 << (bits - 1)) != 0 {
                                    b
                                } else {
                                    a
                                }
                            })
                            .collect()
                    } else if ![f.and, f.or, f.xor, f.add, f.sub].contains(op) {
                        // The only other test instruction is an equality mask.
                        assert_eq!(*op, f.cmp);
                        let Operand::Const(predicate) = args[2] else {
                            panic!("bad predicate")
                        };
                        a.iter()
                            .zip(&b)
                            .map(|(a, b)| {
                                if match predicate.to_i128().unwrap() {
                                    5 => a < b,
                                    6 => a <= b,
                                    _ => a == b,
                                } {
                                    full
                                } else {
                                    0
                                }
                            })
                            .collect()
                    } else {
                        a.iter()
                            .zip(&b)
                            .map(|(&a, &b)| {
                                if *op == f.and {
                                    a & b
                                } else if *op == f.or {
                                    a | b
                                } else if *op == f.xor {
                                    a ^ b
                                } else if *op == f.add {
                                    a.wrapping_add(b) & full
                                } else {
                                    a.wrapping_sub(b) & full
                                }
                            })
                            .collect()
                    }
                }
            }
            _ => panic!("unexpected test instruction"),
        };
        values.insert(dest, data);
    }
    values.remove(&last).unwrap()
}
fn check(code: Vec<Instruction>, family: Family, bits: usize, bytes: usize) {
    let mut f = function(code.clone());
    assert!(run_function(&mut f, &families()) > 0, "{code:?}");
    let rewritten = &f.blocks[0].instructions;
    let mask = u64::MAX >> (64 - bits);
    let mut seed = 0x243f6a8885a308d3u64;
    for case in 0..256 {
        let input = std::array::from_fn(|arg| {
            (0..bytes * 8 / bits)
                .map(|lane| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    (if case < 8 {
                        [0, 1, mask, mask >> 1, 1 << (bits - 1), 0x55, 0xaa, 0xff]
                            [(case + arg + lane) % 8]
                    } else {
                        seed
                    }) & mask
                })
                .collect()
        });
        for bit_masks in [false, true] {
            assert_eq!(
                evaluate(&code, family, bits, &input, bit_masks),
                evaluate(rewritten, family, bits, &input, bit_masks),
                "case {case}: {code:?}"
            );
        }
    }
    // Every emitted temporary is unique, with the fresh cursor above it.
    let mut definitions = std::collections::BTreeSet::new();
    for inst in rewritten {
        let dest = inst.dest().unwrap();
        assert!(definitions.insert(dest.0));
        assert!(dest.0 < f.next_value_id);
    }
}
#[test]
fn wrapping_lane_cancellation_absorption_and_factoring_all_integer_families() {
    for (index, f) in families().into_iter().enumerate() {
        let bits = [8, 16, 32, 64][index / 2];
        let bytes = if index % 2 == 0 { 16 } else { 32 };
        for op in [f.and, f.or, f.xor, f.sub] {
            check(
                vec![intrinsic(Value(4), op, vec![v(0), v(0)])],
                f,
                bits,
                bytes,
            );
        }
        for swapped in [false, true] {
            let pair = if swapped {
                vec![v(1), v(0)]
            } else {
                vec![v(0), v(1)]
            };
            for (inner, outer) in [(f.xor, f.xor), (f.or, f.and), (f.and, f.or)] {
                check(
                    vec![
                        intrinsic(Value(4), inner, pair.clone()),
                        intrinsic(Value(5), outer, vec![v(4), v(0)]),
                    ],
                    f,
                    bits,
                    bytes,
                );
            }
            check(
                vec![
                    intrinsic(Value(4), f.add, pair),
                    intrinsic(Value(5), f.sub, vec![v(4), v(0)]),
                ],
                f,
                bits,
                bytes,
            );
            check(
                vec![
                    intrinsic(Value(4), f.sub, vec![v(0), v(1)]),
                    intrinsic(Value(5), f.add, vec![v(1), v(4)]),
                ],
                f,
                bits,
                bytes,
            );
            for (inner, outer) in [(f.and, f.or), (f.or, f.and), (f.xor, f.xor)] {
                let left = if swapped {
                    vec![v(1), v(0)]
                } else {
                    vec![v(0), v(1)]
                };
                check(
                    vec![
                        intrinsic(Value(4), inner, left),
                        intrinsic(Value(5), inner, vec![v(2), v(0)]),
                        intrinsic(Value(6), outer, vec![v(4), v(5)]),
                    ],
                    f,
                    bits,
                    bytes,
                );
            }
        }
    }
}
#[test]
fn correlated_blends_preserve_raw_masks_and_canonical_lane_arithmetic() {
    for (index, f) in families().into_iter().enumerate() {
        let Some(blend) = f.blend else {
            continue;
        };
        let bits = [8, 16, 32, 64][index / 2];
        let bytes = if index % 2 == 0 { 16 } else { 32 };
        for arm in [0, 1] {
            let mut outer = vec![v(0), v(1), v(3)];
            outer[arm] = v(4);
            check(
                vec![
                    intrinsic(Value(4), blend, vec![v(1), v(2), v(3)]),
                    intrinsic(Value(5), blend, outer),
                ],
                f,
                bits,
                bytes,
            );
        }
        for op in [f.and, f.or, f.xor] {
            check(
                vec![
                    intrinsic(Value(4), blend, vec![v(0), v(1), v(3)]),
                    intrinsic(Value(5), blend, vec![v(1), v(0), v(3)]),
                    intrinsic(Value(6), op, vec![v(4), v(5)]),
                ],
                f,
                bits,
                bytes,
            );
        }
        let cmp = match blend {
            IntrinsicOp::VecBlendvI8x16 => IntrinsicOp::VecCmpI8x16,
            IntrinsicOp::VecBlendvI8x32 => IntrinsicOp::VecCmpI8x32,
            IntrinsicOp::VecBlendvI16x8 => IntrinsicOp::VecCmpI16x8,
            IntrinsicOp::VecBlendvI16x16 => IntrinsicOp::VecCmpI16x16,
            IntrinsicOp::VecBlendvI32x4 => IntrinsicOp::VecCmpI32x4,
            _ => IntrinsicOp::VecCmpI32x8,
        };
        check(
            vec![
                intrinsic(
                    Value(4),
                    cmp,
                    vec![v(0), v(3), Operand::Const(IrConst::I32(0))],
                ),
                intrinsic(Value(5), blend, vec![v(0), v(1), v(4)]),
                intrinsic(Value(6), blend, vec![v(1), v(0), v(4)]),
                intrinsic(Value(7), f.add, vec![v(5), v(6)]),
            ],
            f,
            bits,
            bytes,
        );
    }
}
#[test]
fn refuses_noncanonical_add_masks_mixed_families_shared_factoring_and_fp_arithmetic() {
    let all = families();
    let f = all[4];
    let blend = f.blend.unwrap();
    let mut defs = FxHashMap::default();
    let mut uses = FxHashMap::default();
    for inst in [
        intrinsic(Value(4), blend, vec![v(0), v(1), v(3)]),
        intrinsic(Value(5), blend, vec![v(1), v(0), v(3)]),
    ] {
        defs.insert(inst.dest().unwrap(), inst);
    }
    assert!(
        fold(
            &intrinsic(Value(6), f.add, vec![v(4), v(5)]),
            &defs,
            &uses,
            &all,
            Value(64)
        )
        .is_none()
    );
    defs.insert(Value(4), intrinsic(Value(4), f.and, vec![v(0), v(1)]));
    defs.insert(Value(5), intrinsic(Value(5), f.and, vec![v(0), v(2)]));
    uses.insert(Value(4), 2);
    uses.insert(Value(5), 1);
    assert!(
        fold(
            &intrinsic(Value(6), f.or, vec![v(4), v(5)]),
            &defs,
            &uses,
            &all,
            Value(64)
        )
        .is_none()
    );
    uses.insert(Value(4), 1);
    defs.insert(Value(5), intrinsic(Value(5), all[5].and, vec![v(0), v(2)]));
    assert!(
        fold(
            &intrinsic(Value(6), f.or, vec![v(4), v(5)]),
            &defs,
            &uses,
            &all,
            Value(64)
        )
        .is_none()
    );
    for op in [
        IntrinsicOp::VecAddF32x4,
        IntrinsicOp::VecSubF32x4,
        IntrinsicOp::VecMulF32x4,
        IntrinsicOp::VecMinF32x4,
        IntrinsicOp::VecMaxF32x4,
    ] {
        assert!(
            fold(
                &intrinsic(Value(6), op, vec![v(0), v(0)]),
                &defs,
                &uses,
                &all,
                Value(64)
            )
            .is_none()
        );
    }
}
#[test]
fn fp_bitwise_xor_preserves_every_payload_bit_without_float_arithmetic() {
    for (xor, zero) in [
        (IntrinsicOp::VecXorF32x4, IntrinsicOp::VecZeroF32x4),
        (IntrinsicOp::VecXorF32x8, IntrinsicOp::VecZeroF32x8),
        (IntrinsicOp::VecXorF64x2, IntrinsicOp::VecZeroF64x2),
        (IntrinsicOp::VecXorF64x4, IntrinsicOp::VecZeroF64x4),
    ] {
        let mut defs = FxHashMap::default();
        defs.insert(Value(4), intrinsic(Value(4), xor, vec![v(0), v(1)]));
        assert!(
            matches!(&fold(&intrinsic(Value(5),xor,vec![v(4),v(0)]),&defs,&FxHashMap::default(),&families(),Value(64)).unwrap()[0],Instruction::Copy{src,..} if *src==v(1))
        );
        assert!(
            matches!(&fold(&intrinsic(Value(5),xor,vec![v(0),v(0)]),&defs,&FxHashMap::default(),&families(),Value(64)).unwrap()[0],Instruction::Intrinsic{op,..} if *op==zero)
        );
        for payload in [
            0u64,
            1,
            0x80000000,
            0x7fc01234,
            0x7f800001,
            0xff800000,
            0x8000000000000000,
            0x7ff0000000000001,
            0x7ff8123456789abc,
            u64::MAX,
        ] {
            assert_eq!((payload ^ 0xa5a5a5a5a5a5a5a5) ^ payload, 0xa5a5a5a5a5a5a5a5);
        }
    }
}

fn bounded_source(
    f: Family,
    blend: IntrinsicOp,
    predicate: i32,
    bound: u64,
    mask: u64,
) -> Vec<Instruction> {
    vec![
        intrinsic(
            Value(4),
            f.broadcast,
            vec![Operand::Const(IrConst::I64(bound as i64))],
        ),
        intrinsic(
            Value(5),
            f.broadcast,
            vec![Operand::Const(IrConst::I64(mask as i64))],
        ),
        intrinsic(
            Value(6),
            f.cmp,
            vec![v(0), v(4), Operand::Const(IrConst::I32(predicate))],
        ),
        intrinsic(Value(7), f.and, vec![v(0), v(5)]),
        intrinsic(Value(8), blend, vec![v(1), v(7), v(6)]),
    ]
}
#[test]
fn unsigned_contiguous_range_removes_mask_only_in_selected_vector_arm() {
    for f in families() {
        let byte_blend = if f.bytes == 16 {
            IntrinsicOp::VecBlendvI8x16
        } else {
            IntrinsicOp::VecBlendvI8x32
        };
        for (predicate, bound) in [(5, 8), (6, 7)] {
            check(
                bounded_source(f, byte_blend, predicate, bound, 7),
                f,
                f.bits,
                f.bytes,
            );
            if let Some(blend) = f.blend {
                check(
                    bounded_source(f, blend, predicate, bound, 7),
                    f,
                    f.bits,
                    f.bytes,
                );
            }
        }
        let mut source = bounded_source(f, byte_blend, 5, 8, 7);
        source[3] = intrinsic(Value(7), f.and, vec![v(5), v(0)]);
        check(source, f, f.bits, f.bytes);
    }
}
#[test]
fn vector_range_refuses_signed_bounds_noncontiguous_masks_and_coarser_blends() {
    for f in families() {
        let blend = if f.bytes == 16 {
            IntrinsicOp::VecBlendvI8x16
        } else {
            IntrinsicOp::VecBlendvI8x32
        };
        for (predicate, bound, mask) in [(1, 8, 7), (5, 9, 7), (5, 8, 5), (6, 8, 7)] {
            let source = bounded_source(f, blend, predicate, bound, mask);
            let mut defs = FxHashMap::default();
            for inst in &source[..4] {
                defs.insert(inst.dest().unwrap(), inst.clone());
            }
            assert!(
                fold(
                    source.last().unwrap(),
                    &defs,
                    &FxHashMap::default(),
                    &families(),
                    Value(64)
                )
                .is_none()
            );
        }
        if f.bits < 64 {
            let coarse = if f.bytes == 16 {
                IntrinsicOp::VecBlendvF64x2
            } else {
                IntrinsicOp::VecBlendvF64x4
            };
            let source = bounded_source(f, coarse, 5, 8, 7);
            let defs = source[..4]
                .iter()
                .map(|i| (i.dest().unwrap(), i.clone()))
                .collect();
            assert!(
                fold(
                    source.last().unwrap(),
                    &defs,
                    &FxHashMap::default(),
                    &families(),
                    Value(64)
                )
                .is_none()
            );
        }
    }
}

#[test]
fn observable_fp_environment_keeps_producers_and_register_image_consumers() {
    let f = families()[4];
    let mut func = function(vec![intrinsic(Value(4), f.xor, vec![v(0), v(0)])]);
    func.pass_policy.fp_environment_observable = true;
    let before = func.blocks[0].instructions.clone();
    assert_eq!(run_function(&mut func, &families()), 0);
    assert_eq!(
        format!("{:?}", func.blocks[0].instructions),
        format!("{before:?}")
    );
}

#[test]
fn unsigned_contiguous_range_proves_bitwise_selected_arms_too() {
    for f in families() {
        for (predicate, bound) in [(5, 8), (6, 7)] {
            for swapped in [false, true] {
                let mut source =
                    bounded_source(f, IntrinsicOp::VecBlendvI8x16, predicate, bound, 7);
                source[4] = intrinsic(
                    Value(8),
                    f.and,
                    if swapped {
                        vec![v(7), v(6)]
                    } else {
                        vec![v(6), v(7)]
                    },
                );
                check(source, f, f.bits, f.bytes);
            }
        }
        for (predicate, bound, mask) in [(1, 8, 7), (5, 9, 7), (5, 8, 5), (6, 8, 7)] {
            let mut source = bounded_source(f, IntrinsicOp::VecBlendvI8x16, predicate, bound, mask);
            source[4] = intrinsic(Value(8), f.and, vec![v(6), v(7)]);
            let defs = source[..4]
                .iter()
                .map(|i| (i.dest().unwrap(), i.clone()))
                .collect();
            assert!(
                fold(
                    &source[4],
                    &defs,
                    &FxHashMap::default(),
                    &families(),
                    Value(64)
                )
                .is_none()
            );
        }
    }
}

#[test]
fn lane_immediates_are_splats_not_scalar_copies_or_src1_constants() {
    for f in families() {
        let seven = Operand::Const(IrConst::I64(7));
        let mut source = bounded_source(f, IntrinsicOp::VecBlendvI8x16, 5, 8, 7);
        source[3] = intrinsic(Value(7), f.and, vec![v(0), seven]);
        source[4] = intrinsic(Value(8), f.and, vec![v(6), v(7)]);
        check(source, f, f.bits, f.bytes);
        for op in [f.xor, f.add] {
            let cancel = if op == f.xor { f.xor } else { f.sub };
            check(
                vec![
                    intrinsic(Value(4), op, vec![v(0), seven]),
                    intrinsic(Value(5), cancel, vec![v(4), v(0)]),
                ],
                f,
                f.bits,
                f.bytes,
            );
        }
        check(
            vec![
                intrinsic(Value(4), f.and, vec![v(0), seven]),
                intrinsic(Value(5), f.and, vec![v(0), v(1)]),
                intrinsic(Value(6), f.or, vec![v(4), v(5)]),
            ],
            f,
            f.bits,
            f.bytes,
        );
        check(
            vec![
                intrinsic(Value(4), f.and, vec![v(0), seven]),
                intrinsic(Value(5), f.and, vec![v(1), seven]),
                intrinsic(Value(6), f.or, vec![v(4), v(5)]),
            ],
            f,
            f.bits,
            f.bytes,
        );
    }
}

#[test]
fn fp_literal_results_and_two_literal_factoring_remain_conservative() {
    let mut f = function(vec![
        intrinsic(
            Value(4),
            IntrinsicOp::VecXorF32x4,
            vec![v(0), Operand::Const(IrConst::F32(-0.0))],
        ),
        intrinsic(Value(5), IntrinsicOp::VecXorF32x4, vec![v(4), v(0)]),
    ]);
    assert_eq!(run_function(&mut f, &families()), 0);
    for family in families() {
        let mut f = function(vec![
            intrinsic(
                Value(4),
                family.and,
                vec![v(0), Operand::Const(IrConst::I64(7))],
            ),
            intrinsic(
                Value(5),
                family.and,
                vec![v(0), Operand::Const(IrConst::I64(3))],
            ),
            intrinsic(Value(6), family.or, vec![v(4), v(5)]),
        ]);
        assert_eq!(run_function(&mut f, &families()), 0);
    }
}
