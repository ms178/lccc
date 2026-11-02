use super::*;
fn v(n: u32) -> Operand {
    Operand::Value(Value(n))
}
fn unary(dest: u32, op: IrUnaryOp, src: Operand, ty: IrType) -> Instruction {
    Instruction::UnaryOp {
        dest: Value(dest),
        op,
        src,
        ty,
    }
}
fn context(code: &[Instruction], test: impl FnOnce(&Context<'_>)) {
    let mut defs = vec![None; 128];
    let mut uses = vec![0; 128];
    for inst in code {
        defs[inst.dest().unwrap().0 as usize] = Some(inst.clone());
        for value in inst.used_values() {
            uses[value as usize] += 1;
        }
    }
    test(&Context {
        defs: &defs,
        uses: &uses,
        visits: std::cell::Cell::new(512),
        disable_logic: false,
        wrap_signed_overflow: true,
    });
}
fn eval(code: &[Instruction], x: u64, ty: IrType) -> u64 {
    let width = ty.size() * 8;
    let mask = u64::MAX >> (64 - width);
    let mut values = [0; 128];
    values[0] = x & mask;
    let mut last = 0;
    let get = |op: Operand, values: &[u64]| match op {
        Operand::Value(v) => values[v.0 as usize],
        _ => const_in_ring(op, width).unwrap(),
    };
    for inst in code {
        let dest = inst.dest().unwrap().0 as usize;
        last = dest;
        values[dest] = match inst {
            Instruction::Copy { src, .. } => get(*src, &values),
            Instruction::UnaryOp { op, src, .. } => {
                let n = get(*src, &values);
                match op {
                    IrUnaryOp::Not => !n,
                    IrUnaryOp::BitReverse => n.reverse_bits() >> (64 - width),
                    _ => panic!("bad unary"),
                }
            }
            Instruction::BinOp { op, lhs, rhs, .. } => {
                let a = get(*lhs, &values);
                let b = get(*rhs, &values);
                match op {
                    IrBinOp::And => a & b,
                    IrBinOp::Or => a | b,
                    IrBinOp::Xor => a ^ b,
                    IrBinOp::Shl => a << b,
                    IrBinOp::LShr => a >> b,
                    _ => panic!("bad binary"),
                }
            }
            _ => panic!("bad node"),
        } & mask;
    }
    values[last]
}
#[test]
fn reverse_conjugation_matches_all_six_upstream_shapes_and_constant_masks() {
    for ty in [
        IrType::I8,
        IrType::U8,
        IrType::I16,
        IrType::U16,
        IrType::I32,
        IrType::U32,
        IrType::I64,
        IrType::U64,
    ] {
        let width = ty.size() * 8;
        for count in [0, 1, width as u64 - 1] {
            for shape in 0..8 {
                let mut code = vec![
                    unary(3, IrUnaryOp::BitReverse, v(0), ty),
                    bin(
                        Value(4),
                        if shape == 0 {
                            IrBinOp::Shl
                        } else {
                            IrBinOp::LShr
                        },
                        v(3),
                        constant(count, ty),
                        ty,
                    ),
                ];
                match shape {
                    0 | 1 => {}
                    2 => code.push(unary(5, IrUnaryOp::Not, v(4), ty)),
                    3 | 4 => code.push(bin(
                        Value(5),
                        if shape == 3 {
                            IrBinOp::Or
                        } else {
                            IrBinOp::And
                        },
                        v(3),
                        v(4),
                        ty,
                    )),
                    5 => {
                        code.push(unary(5, IrUnaryOp::Not, v(4), ty));
                        code.push(bin(Value(6), IrBinOp::And, v(3), v(5), ty));
                    }
                    6 | 7 => code.push(bin(
                        Value(5),
                        if shape == 6 {
                            IrBinOp::Xor
                        } else {
                            IrBinOp::And
                        },
                        v(4),
                        constant(0x35, ty),
                        ty,
                    )),
                    _ => unreachable!(),
                }
                let source = Operand::Value(code.last().unwrap().dest().unwrap());
                code.push(unary(7, IrUnaryOp::BitReverse, source, ty));
                context(&code, |cx| {
                    let rewritten =
                        fold(code.last().unwrap(), cx, Value(64)).expect("exact private network");
                    assert!(!rewritten.iter().any(|i| matches!(
                        i,
                        Instruction::UnaryOp {
                            op: IrUnaryOp::BitReverse,
                            ..
                        }
                    )));
                    let mut all = code[..code.len() - 1].to_vec();
                    all.extend(rewritten);
                    for low in 0..256 {
                        for high in [0, u64::MAX, 1u64 << (width - 1)] {
                            let x = high ^ low;
                            assert_eq!(eval(&code, x, ty), eval(&all, x, ty));
                        }
                    }
                });
            }
        }
    }
}
#[test]
fn reverse_networks_refuse_arithmetic_oversized_counts_casts_and_shared_internals() {
    let ty = IrType::U32;
    for op in [IrBinOp::Add, IrBinOp::Mul, IrBinOp::AShr] {
        let code = vec![
            unary(3, IrUnaryOp::BitReverse, v(0), ty),
            bin(Value(4), op, v(3), constant(1, ty), ty),
            unary(5, IrUnaryOp::BitReverse, v(4), ty),
        ];
        context(&code, |cx| {
            assert!(fold(code.last().unwrap(), cx, Value(64)).is_none())
        });
    }
    for count in [32i128, 33, -1, 1i128 << 64] {
        let code = vec![
            unary(3, IrUnaryOp::BitReverse, v(0), ty),
            bin(
                Value(4),
                IrBinOp::Shl,
                v(3),
                Operand::Const(IrConst::I128(count)),
                ty,
            ),
            unary(5, IrUnaryOp::BitReverse, v(4), ty),
        ];
        context(&code, |cx| {
            assert!(fold(code.last().unwrap(), cx, Value(64)).is_none())
        });
    }
    let code = vec![
        unary(3, IrUnaryOp::BitReverse, v(0), ty),
        bin(Value(4), IrBinOp::Shl, v(3), constant(1, ty), ty),
        bin(Value(5), IrBinOp::Add, v(4), v(0), ty),
        unary(6, IrUnaryOp::BitReverse, v(4), ty),
    ];
    context(&code, |cx| {
        assert!(fold(code.last().unwrap(), cx, Value(64)).is_none())
    });
    let code = vec![
        unary(3, IrUnaryOp::BitReverse, v(0), ty),
        Instruction::Cast {
            dest: Value(4),
            src: v(3),
            from_ty: ty,
            to_ty: IrType::U64,
        },
        unary(5, IrUnaryOp::BitReverse, v(4), IrType::U64),
    ];
    context(&code, |cx| {
        assert!(fold(code.last().unwrap(), cx, Value(64)).is_none())
    });
}

#[test]
fn early_portable_ladder_recognition_preserves_exact_masks() {
    use crate::ir::reexports::{BasicBlock, BlockId, IrFunction, Terminator};
    for wrong in [false, true] {
        let ty = IrType::U32;
        let mut code = Vec::new();
        let mut input = v(0);
        let mut next = 1;
        for (amount, mask) in [
            (1, 0x55555555u64),
            (2, 0x33333333),
            (4, 0x0f0f0f0f),
            (8, 0x00ff00ff),
        ] {
            let mut emit = |op, a, b| {
                let dest = Value(next);
                next += 1;
                code.push(bin(dest, op, a, b, ty));
                Operand::Value(dest)
            };
            let count = constant(amount, ty);
            let mask = constant(mask ^ u64::from(wrong && amount == 1), ty);
            let right_shift = emit(IrBinOp::LShr, input, count);
            let right = emit(IrBinOp::And, right_shift, mask);
            let left_mask = emit(IrBinOp::And, input, mask);
            let left = emit(IrBinOp::Shl, left_mask, count);
            input = emit(IrBinOp::Or, right, left);
        }
        code.push(bin(Value(next), IrBinOp::LShr, input, constant(16, ty), ty));
        code.push(bin(
            Value(next + 1),
            IrBinOp::Shl,
            input,
            constant(16, ty),
            ty,
        ));
        code.push(bin(Value(next + 2), IrBinOp::Or, v(next), v(next + 1), ty));
        let mut f = IrFunction::new("portable_reverse".into(), ty, vec![], false);
        // A real inline helper retains its own parameter home. Two
        // separate loads must be proved equal by the analysis projection,
        // not by pretending arbitrary pointer loads have equal contents.
        let home = Value(next + 3);
        let load_a = Value(next + 4);
        let load_b = Value(next + 5);
        if let Instruction::BinOp { lhs, .. } = &mut code[0] {
            *lhs = Operand::Value(load_a);
        }
        if let Instruction::BinOp { lhs, .. } = &mut code[2] {
            *lhs = Operand::Value(load_b);
        }
        code.insert(
            2,
            Instruction::Load {
                dest: load_b,
                ptr: home,
                ty,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
        );
        code.insert(
            0,
            Instruction::Load {
                dest: load_a,
                ptr: home,
                ty,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
        );
        code.insert(
            0,
            Instruction::Store {
                val: v(0),
                ptr: home,
                ty,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
        );
        code.insert(
            0,
            Instruction::Alloca {
                dest: home,
                ty,
                size: 4,
                align: 4,
                volatile: false,
                semantic_volatile: false,
            },
        );
        f.next_value_id = next + 6;
        f.blocks = vec![BasicBlock {
            label: BlockId(0),
            instructions: code,
            source_spans: vec![],
            terminator: Terminator::Return(Some(v(next + 2))),
        }];
        assert_eq!(
            crate::passes::bit_idioms::recognize_reverse_ladders(&mut f),
            usize::from(!wrong)
        );
        if !wrong {
            assert!(
                matches!(f.blocks[0].instructions.last(), Some(Instruction::UnaryOp { op: IrUnaryOp::BitReverse, src, .. }) if *src == v(0))
            );
        }
    }
}
