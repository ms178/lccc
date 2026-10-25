use super::*;
fn v(n: u32) -> Operand {
    Operand::Value(Value(n))
}
fn c(n: i32) -> Operand {
    Operand::Const(IrConst::I32(n))
}
fn shift(d: u32, src: u32, op: IntrinsicOp, n: i32) -> Instruction {
    intrinsic(Value(d), op, vec![v(src), c(n)])
}
fn ashr(x: u64, n: usize, bits: usize) -> u64 {
    let full = u64::MAX >> (64 - bits);
    if x & (1 << (bits - 1)) != 0 {
        (x >> n) | (full ^ (full >> n))
    } else {
        x >> n
    }
}
#[test]
fn fixed_shift_chains_all_counts_and_exhaustive_words() {
    for shape in shapes() {
        let full = u64::MAX >> (64 - shape.bits);
        for op in [Some(shape.shl), Some(shape.lshr), shape.ashr]
            .into_iter()
            .flatten()
        {
            for n in 1..shape.bits {
                for m in 1..shape.bits {
                    let defs = [(Value(1), shift(1, 0, op, m as i32))]
                        .into_iter()
                        .collect();
                    let uses = [(Value(1), 1)].into_iter().collect();
                    let out = fold(&shift(2, 1, op, n as i32), &defs, &uses, &families()).unwrap();
                    assert_eq!(out.len(), 1);
                    let apply = |x: u64, k: usize| {
                        if op == shape.shl {
                            x.wrapping_shl(k as u32) & full
                        } else if op == shape.lshr {
                            x >> k
                        } else {
                            ashr(x, k, shape.bits)
                        }
                    };
                    let run = |x| match &out[0] {
                        Instruction::Copy { .. } => apply(x, m),
                        Instruction::Intrinsic {
                            op: other, args, ..
                        } if *other == op => {
                            assert!(matches!(args[0], Operand::Value(Value(0))));
                            apply(x, amount(args[1], shape.bits).unwrap())
                        }
                        Instruction::Intrinsic {
                            op: other, args, ..
                        } => {
                            assert!(matches!(
                                other,
                                IntrinsicOp::VecZeroI32x4 | IntrinsicOp::VecZeroI32x8
                            ));
                            assert!(args.is_empty());
                            0
                        }
                        _ => panic!("bad shift replacement"),
                    };
                    let count = if shape.bits == 16 && shape.bytes == 16 {
                        65536
                    } else {
                        256
                    };
                    for i in 0..count {
                        let x = if shape.bits == 16 {
                            i
                        } else {
                            (i as u64).wrapping_mul(0x9e3779b97f4a7c15) & full
                        };
                        assert_eq!(run(x), apply(apply(x, m), n));
                    }
                    for x in [0, 1, full, full >> 1, 1 << (shape.bits - 1)] {
                        assert_eq!(run(x), apply(apply(x, m), n));
                    }
                }
            }
        }
    }
}
#[test]
fn canonical_deep_mask_and_granularity_boundaries() {
    for shape in shapes().into_iter().filter(|s| s.ashr.is_some()) {
        let f = families()
            .into_iter()
            .find(|f| f.bits == shape.bits && f.bytes == shape.bytes)
            .unwrap();
        let mut defs = FxHashMap::default();
        for id in 2..9 {
            defs.insert(
                Value(id),
                intrinsic(Value(id), f.cmp, vec![v(0), v(1), c(2)]),
            );
        }
        let mut previous = 2;
        for id in 9..15 {
            defs.insert(
                Value(id),
                intrinsic(
                    Value(id),
                    if id == 9 { f.or } else { f.and },
                    vec![v(previous), v(id - 6)],
                ),
            );
            previous = id;
        }
        assert!(canonical(
            v(previous),
            shape,
            &defs,
            &families(),
            16,
            &mut 256
        ));
        assert!(!canonical(
            v(previous),
            shape,
            &defs,
            &families(),
            2,
            &mut 256
        ));
        assert!(!canonical(
            v(previous),
            shape,
            &defs,
            &families(),
            16,
            &mut 0
        ));
        defs.insert(
            Value(20),
            shift(20, previous, shape.shl, (shape.bits - 1) as i32),
        );
        let uses = [(Value(20), 1)].into_iter().collect();
        let out = fold(
            &shift(21, 20, shape.ashr.unwrap(), (shape.bits - 1) as i32),
            &defs,
            &uses,
            &families(),
        )
        .unwrap();
        assert!(
            matches!(out.as_slice(),[Instruction::Copy{src:Operand::Value(x),..}] if *x==Value(previous))
        );
        let fine = families()
            .into_iter()
            .find(|f| f.bits == 8 && f.bytes == shape.bytes)
            .unwrap();
        defs.insert(
            Value(2),
            intrinsic(Value(2), fine.cmp, vec![v(0), v(1), c(2)]),
        );
        assert!(!canonical(
            v(previous),
            shape,
            &defs,
            &families(),
            16,
            &mut 256
        ));
    }
}
#[test]
fn refuses_invalid_counts_shared_producers_and_unknown_masks() {
    let shape = shapes()[0];
    let mut defs = [(Value(1), shift(1, 0, shape.shl, 7))]
        .into_iter()
        .collect();
    let mut uses = [(Value(1), 1)].into_iter().collect();
    assert!(
        fold(
            &shift(2, 1, shape.ashr.unwrap(), 7),
            &defs,
            &uses,
            &families()
        )
        .is_none()
    );
    uses.insert(Value(1), 2);
    assert!(fold(&shift(2, 1, shape.shl, 7), &defs, &uses, &families()).is_none());
    uses.insert(Value(1), 1);
    for bad in [-1, 0, 16, 17, i32::MAX] {
        assert!(fold(&shift(2, 1, shape.shl, bad), &defs, &uses, &families()).is_none());
        defs.insert(Value(1), shift(1, 0, shape.shl, bad));
        assert!(fold(&shift(2, 1, shape.shl, 7), &defs, &uses, &families()).is_none());
    }
    let mut inst = shift(2, 1, shape.shl, 7);
    if let Instruction::Intrinsic { args, .. } = &mut inst {
        args[1] = v(0);
    }
    assert!(fold(&inst, &defs, &uses, &families()).is_none());
}

#[test]
fn refuses_malformed_compare_predicates() {
    let shape = shapes()[0];
    for predicate in [c(-1), c(3), c(7), v(9)] {
        let defs = [(
            Value(1),
            intrinsic(
                Value(1),
                IntrinsicOp::VecCmpI16x8,
                vec![v(0), v(2), predicate],
            ),
        )]
        .into_iter()
        .collect();
        assert!(!canonical(v(1), shape, &defs, &families(), 16, &mut 256));
    }
}
