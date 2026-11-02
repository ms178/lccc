use super::*;
fn v(i: u32) -> Operand {
    Operand::Value(Value(i))
}
fn sh(d: u32, s: u32, m: i32) -> Instruction {
    intrinsic(
        Value(d),
        IntrinsicOp::VecShufdI32x4,
        vec![v(s), Operand::Const(IrConst::I32(m))],
    )
}
fn setup(
    a: i32,
    b: i32,
) -> (
    FxHashMap<Value, Instruction>,
    FxHashMap<Value, u32>,
    Instruction,
) {
    let defs = [(Value(2), sh(2, 0, a)), (Value(3), sh(3, 1, b))]
        .into_iter()
        .collect();
    let uses = [(Value(2), 1), (Value(3), 1)].into_iter().collect();
    (
        defs,
        uses,
        intrinsic(Value(4), IntrinsicOp::VecSubI32x4, vec![v(2), v(3)]),
    )
}
fn apply(x: [u32; 4], mask: u8) -> [u32; 4] {
    std::array::from_fn(|l| x[((mask >> (2 * l)) & 3) as usize])
}
#[test]
fn all_65536_compositions_match_independent_lane_interpreter() {
    let image = [0x7f800001, 0xffc01234, 0, 0xffffffff];
    for inner in 0..=255u8 {
        for outer in 0..=255u8 {
            assert_eq!(
                apply(image, compose(inner, outer)),
                apply(apply(image, inner), outer)
            );
            let (defs, uses, _) = setup(inner.into(), 0);
            let result = fold(
                &sh(4, 2, outer.into()),
                &defs,
                &uses,
                &families(),
                Value(10),
            )
            .unwrap();
            assert_eq!(result.len(), 1);
            match &result[0] {
                Instruction::Copy {
                    src: Operand::Value(src),
                    ..
                } => {
                    let actual = if *src == Value(2) {
                        apply(image, inner)
                    } else {
                        assert_eq!(*src, Value(0));
                        image
                    };
                    assert_eq!(actual, apply(apply(image, inner), outer));
                }
                Instruction::Intrinsic { op, args, .. } => {
                    assert_eq!(*op, IntrinsicOp::VecShufdI32x4);
                    assert!(matches!(args[0], Operand::Value(Value(0))));
                    assert_eq!(
                        apply(image, immediate(args[1]).unwrap()),
                        apply(apply(image, inner), outer)
                    );
                }
                _ => panic!("unexpected composition"),
            }
        }
    }
}
#[test]
fn factor_all_masks_and_modular_lane_families() {
    for f in families().into_iter().filter(|f| f.bytes == 16) {
        for op in [f.and, f.or, f.xor, f.add, f.sub] {
            for mask in 0..=255 {
                let (defs, uses, _) = setup(mask, mask);
                let root = intrinsic(Value(4), op, vec![v(2), v(3)]);
                let result = fold(&root, &defs, &uses, &families(), Value(10));
                if f.bits == 64 && [f.add, f.sub].contains(&op) {
                    assert!(result.is_none());
                    continue;
                }
                let result = result.unwrap();
                assert!(
                    matches!(&result[0],Instruction::Intrinsic{op:other,args,..} if *other==op && matches!(args.as_slice(),[Operand::Value(Value(0)),Operand::Value(Value(1))]))
                );
                // Check every sublane, including carry/borrow boundaries. Byte
                // interpretation is independent of the matcher's family table.
                let a = std::array::from_fn(|i| 0x8fff00ffu32.wrapping_mul(i as u32 + 1));
                let b = std::array::from_fn(|i| 0x7fffff01u32.wrapping_mul(i as u32 + 3));
                let calc = |a: [u32; 4], b: [u32; 4]| {
                    let mut out = [0u32; 4];
                    let bits = f.bits.min(32);
                    let full = u32::MAX >> (32 - bits);
                    for i in 0..4 {
                        for lane in 0..32 / bits {
                            let x = (a[i] >> (lane * bits)) & full;
                            let y = (b[i] >> (lane * bits)) & full;
                            let z = if op == f.add {
                                x.wrapping_add(y)
                            } else if op == f.sub {
                                x.wrapping_sub(y)
                            } else if op == f.and {
                                x & y
                            } else if op == f.or {
                                x | y
                            } else {
                                x ^ y
                            };
                            out[i] |= (z & full) << (lane * bits);
                        }
                    }
                    out
                };
                assert_eq!(
                    calc(apply(a, mask as u8), apply(b, mask as u8)),
                    apply(calc(a, b), mask as u8)
                );
            }
        }
    }
}
#[test]
fn rejected_shared_mismatched_float_wide_copies_and_bad_immediates() {
    let (mut defs, mut uses, root) = setup(27, 27);
    uses.insert(Value(2), 2);
    assert!(fold(&root, &defs, &uses, &families(), Value(10)).is_none());
    assert!(fold(&sh(4, 2, 27), &defs, &uses, &families(), Value(10)).is_none());
    uses.insert(Value(2), 1);
    defs.insert(Value(3), sh(3, 1, 57));
    assert!(fold(&root, &defs, &uses, &families(), Value(10)).is_none());
    defs.insert(Value(3), sh(3, 1, 27));
    for op in [
        IntrinsicOp::VecAddF32x4,
        IntrinsicOp::VecSubF32x4,
        IntrinsicOp::VecMulF32x4,
        IntrinsicOp::VecAddI64x2,
        IntrinsicOp::VecSubI64x2,
        IntrinsicOp::VecAddI32x8,
    ] {
        assert!(
            fold(
                &intrinsic(Value(4), op, vec![v(2), v(3)]),
                &defs,
                &uses,
                &families(),
                Value(10)
            )
            .is_none()
        );
    }
    for bad in [-1, 256, i32::MAX] {
        assert!(fold(&sh(4, 2, bad), &defs, &uses, &families(), Value(10)).is_none());
        defs.insert(Value(2), sh(2, 0, bad));
        assert!(fold(&sh(4, 2, 27), &defs, &uses, &families(), Value(10)).is_none());
    }
    defs.insert(
        Value(2),
        Instruction::Copy {
            dest: Value(2),
            src: v(0),
        },
    );
    assert!(fold(&sh(4, 2, 27), &defs, &uses, &families(), Value(10)).is_none());
    assert!(fold(&sh(4, 2, 228), &defs, &uses, &families(), Value(10)).is_some());
}
