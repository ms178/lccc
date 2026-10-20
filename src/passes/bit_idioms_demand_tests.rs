use super::*;
use crate::ir::reexports::{BasicBlock, BlockId, IrParam, Terminator};

struct P {
    code: Vec<Instruction>,
    ty: IrType,
    next: u32,
}
impl P {
    fn new(ty: IrType) -> Self {
        Self {
            code: (0..2)
                .map(|n| Instruction::ParamRef {
                    dest: Value(n),
                    param_idx: n as usize,
                    ty,
                })
                .collect(),
            ty,
            next: 2,
        }
    }
    fn c(&self, n: u64) -> Operand {
        Operand::Const(IrConst::from_i64(n as i64, self.ty))
    }
    fn b(&mut self, op: IrBinOp, lhs: Operand, rhs: Operand) -> Operand {
        let dest = Value(self.next);
        self.next += 1;
        self.code.push(Instruction::BinOp {
            dest,
            op,
            lhs,
            rhs,
            ty: self.ty,
        });
        Operand::Value(dest)
    }
    fn u(&mut self, op: IrUnaryOp, src: Operand) -> Operand {
        let dest = Value(self.next);
        self.next += 1;
        self.code.push(Instruction::UnaryOp {
            dest,
            op,
            src,
            ty: self.ty,
        });
        Operand::Value(dest)
    }
    fn mask(&mut self, x: Operand, n: u64) -> Operand {
        self.b(IrBinOp::And, x, self.c(n))
    }
    fn finish(self, root: Operand) -> IrFunction {
        let param = || IrParam {
            ty: self.ty,
            noalias: false,
            struct_size: None,
            struct_align: None,
            param_align: None,
            struct_eightbyte_classes: vec![],
            is_f128_sse: false,
            riscv_float_class: None,
        };
        let mut f = IrFunction::new("demand".into(), self.ty, vec![param(), param()], false);
        f.next_value_id = self.next;
        f.next_label = 1;
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            source_spans: vec![crate::common::source::Span::dummy(); self.code.len()],
            instructions: self.code,
            terminator: Terminator::Return(Some(root)),
        });
        f
    }
}
fn fixture(case: usize, ty: IrType) -> IrFunction {
    use IrBinOp::*;
    use IrUnaryOp::*;
    let mut p = P::new(ty);
    let x = Operand::Value(Value(0));
    let y = Operand::Value(Value(1));
    let all = full(ty).unwrap();
    let root = match case {
        0..=2 | 10..=12 => {
            let a = p.mask(x, 255);
            let b = p.mask(y, 255);
            let op = match case {
                0 => Add,
                1 => Sub,
                2 => Mul,
                10 => Xor,
                11 => Or,
                _ => And,
            };
            let r = p.b(op, a, b);
            p.mask(r, 15)
        }
        3 => {
            let a = p.mask(x, 15);
            let r = p.b(Mul, a, p.c(1536));
            p.mask(r, 1536)
        }
        4 => {
            let a = p.mask(x, 255);
            let r = p.b(Shl, a, p.c(7));
            p.mask(r, 1536)
        }
        5 => {
            let a = p.mask(x, 0xff00);
            let r = p.b(LShr, a, p.c(8));
            p.mask(r, 15)
        }
        6 => {
            let a = p.mask(x, all & !127);
            let r = p.b(AShr, a, p.c(7));
            p.mask(r, 15)
        }
        7 => {
            let a = p.mask(x, 65535);
            let r = p.b(RotateLeft, a, p.c(8));
            p.mask(r, 0xf00)
        }
        8 | 9 => {
            let a = p.mask(x, 255);
            let r = p.u(if case == 8 { Not } else { Neg }, a);
            p.mask(r, 15)
        }
        13 => {
            let a = p.mask(x, 0xffu64 << (ty.size() * 8 - 8));
            let r = p.u(Bswap, a);
            p.mask(r, 15)
        }
        14 => {
            let a = p.b(URem, x, p.c(48));
            let r = p.b(Shl, a, p.c(9));
            p.mask(r, 1536)
        }
        15 => {
            let a = p.b(URem, x, p.c(768));
            let r = p.b(LShr, a, p.c(4));
            p.mask(r, 15)
        }
        16 => {
            let a = p.b(URem, x, p.c(48));
            let b = p.b(URem, y, p.c(80));
            let r = p.b(Add, a, b);
            p.mask(r, 15)
        }
        17 => {
            let a = p.b(URem, x, p.c(48));
            let r = p.b(Mul, a, p.c(1536));
            p.mask(r, 1536)
        }
        18 | 19 => {
            let factor = if case == 18 {
                p.c(3)
            } else {
                p.b(Or, y, p.c(1))
            };
            let a = p.b(Mul, x, factor);
            p.u(Ctz, a)
        }
        20..=22 => {
            let a = p.u(Neg, x);
            let r = if case == 20 {
                a
            } else {
                p.b(if case == 21 { And } else { Or }, x, a)
            };
            p.u(Ctz, r)
        }
        23..=30 => {
            let leading = (27..=29).contains(&case);
            let a = p.b(if leading { LShr } else { Shl }, x, p.c(7));
            let a = if case >= 29 { p.u(Not, a) } else { a };
            let op = match case {
                23 | 28 => Xor,
                24 | 27 => Or,
                25 => Add,
                26 => Sub,
                _ => And,
            };
            let r = p.b(op, x, a);
            p.u(if leading { Clz } else { Ctz }, r)
        }
        31 | 32 => {
            let a = if case == 31 {
                p.u(Bswap, x)
            } else {
                p.b(RotateRight, x, p.c(7))
            };
            p.u(Popcount, a)
        }
        33..=35 => {
            let a = p.u(Popcount, x);
            let b = p.u(Popcount, y);
            let r = p.b(
                match case {
                    33 => Add,
                    34 => Sub,
                    _ => Xor,
                },
                a,
                b,
            );
            p.mask(r, 1)
        }
        36 | 37 => {
            let a = if case == 36 {
                p.u(Not, x)
            } else {
                p.b(Xor, x, p.c(0x5a))
            };
            let r = p.u(Popcount, a);
            p.mask(r, 1)
        }
        38 => {
            let a = p.mask(x, 0x55555555);
            let b = p.mask(x, all ^ 0x55555555);
            let a = p.u(Popcount, a);
            let b = p.u(Popcount, b);
            p.b(Add, a, b)
        }
        39 => {
            let a = p.b(Or, x, y);
            let b = p.b(And, x, y);
            let a = p.u(Popcount, a);
            let b = p.u(Popcount, b);
            p.b(Sub, a, b)
        }
        40 => {
            let b = p.u(Not, x);
            let a = p.u(Popcount, x);
            let b = p.u(Popcount, b);
            p.b(Add, a, b)
        }
        41 => {
            let n = p.u(Not, y);
            let a = p.b(And, x, y);
            let b = p.b(And, x, n);
            let a = p.u(Popcount, a);
            let b = p.u(Popcount, b);
            p.b(Add, a, b)
        }
        _ => unreachable!(),
    };
    p.finish(root)
}
fn eval(f: &IrFunction, x: u64, y: u64) -> u64 {
    let mut values = vec![0; f.max_value_id() as usize + 1];
    for i in &f.blocks[0].instructions {
        let ty = match i {
            Instruction::BinOp { ty, .. }
            | Instruction::UnaryOp { ty, .. }
            | Instruction::ParamRef { ty, .. } => *ty,
            Instruction::Cast { to_ty, .. } => *to_ty,
            _ => f.return_type,
        };
        let width = ty.size() * 8;
        let mask = full(ty).unwrap();
        let get = |o| match o {
            Operand::Value(v) => values[v.0 as usize],
            _ => const_in_ring(o, width).unwrap(),
        };
        let n = match i {
            Instruction::ParamRef { param_idx, .. } => {
                if *param_idx == 0 {
                    x
                } else {
                    y
                }
            }
            Instruction::Copy { src, .. } => get(*src),
            Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            } => {
                let n = get(*src);
                if from_ty.is_signed() && to_ty.size() > from_ty.size() {
                    ((n << (64 - from_ty.size() * 8)) as i64 >> (64 - from_ty.size() * 8)) as u64
                } else {
                    n
                }
            }
            Instruction::BinOp { op, lhs, rhs, .. } => {
                let a = get(*lhs);
                let b = get(*rhs);
                match op {
                    IrBinOp::Add => a.wrapping_add(b),
                    IrBinOp::Sub => a.wrapping_sub(b),
                    IrBinOp::Mul => a.wrapping_mul(b),
                    IrBinOp::And => a & b,
                    IrBinOp::Or => a | b,
                    IrBinOp::Xor => a ^ b,
                    IrBinOp::Shl => a << b,
                    IrBinOp::LShr => a >> b,
                    IrBinOp::AShr => {
                        if width == 32 {
                            ((a as i32) >> b) as u32 as u64
                        } else {
                            ((a as i64) >> b) as u64
                        }
                    }
                    IrBinOp::RotateLeft => rotate(a, b as u32, ty, true),
                    IrBinOp::RotateRight => rotate(a, b as u32, ty, false),
                    IrBinOp::URem => a % b,
                    _ => panic!("{i:?}"),
                }
            }
            Instruction::UnaryOp { op, src, .. } => {
                let a = get(*src);
                match op {
                    IrUnaryOp::Not => !a,
                    IrUnaryOp::Neg => a.wrapping_neg(),
                    IrUnaryOp::Bswap => swap(a, ty),
                    IrUnaryOp::BitReverse => a.reverse_bits() >> (64 - width),
                    IrUnaryOp::Popcount => a.count_ones() as u64,
                    IrUnaryOp::Clz | IrUnaryOp::ClzNonZero => {
                        if width == 32 {
                            (a as u32).leading_zeros() as u64
                        } else {
                            a.leading_zeros() as u64
                        }
                    }
                    IrUnaryOp::Ctz | IrUnaryOp::CtzNonZero => {
                        if width == 32 {
                            (a as u32).trailing_zeros() as u64
                        } else {
                            a.trailing_zeros() as u64
                        }
                    }
                    _ => panic!("{i:?}"),
                }
            }
            _ => panic!("{i:?}"),
        };
        values[i.dest().unwrap().0 as usize] = n & mask;
    }
    let Terminator::Return(Some(root)) = f.blocks[0].terminator else {
        panic!()
    };
    match root {
        Operand::Value(v) => values[v.0 as usize],
        _ => const_in_ring(root, f.return_type.size() * 8).unwrap(),
    }
}
fn check(mut f: IrFunction, expect: bool) {
    let before = f.clone();
    let changed = run(&mut f);
    assert_eq!(
        changed > 0,
        expect,
        "{} {:#?}",
        f.name,
        before.blocks[0].instructions
    );
    assert_eq!(
        f.blocks[0].source_spans.len(),
        f.blocks[0].instructions.len()
    );
    let mut violations = vec![];
    crate::passes::verify::verify_function(&f, "batch3", &mut violations);
    assert!(violations.is_empty(), "{violations:?}");
    let ids = f.next_value_id;
    assert_eq!(run(&mut f), 0, "not idempotent: {before:#?}");
    assert_eq!(f.next_value_id, ids);
    for x in [
        0,
        1,
        2,
        7,
        127,
        128,
        255,
        256,
        0x7fffffff,
        0x80000000,
        u64::MAX,
        1 << 63,
    ] {
        for y in [0, 1, 3, 127, u64::MAX] {
            assert_eq!(
                eval(&before, x, y),
                eval(&f, x, y),
                "x={x} y={y} before={before:#?}"
            );
        }
    }
    let mut x = 178u64;
    for _ in 0..4096 {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
        let y = x.rotate_left(23) ^ 0x55aa55aa55aa55aa;
        assert_eq!(eval(&before, x, y), eval(&f, x, y));
    }
}
#[test]
fn all_42_witnesses_both_widths_signedness_verifier_and_idempotence() {
    for ty in [IrType::U32, IrType::U64, IrType::I32, IrType::I64] {
        for case in 0..42 {
            let mut f = fixture(case, ty);
            f.name = format!("case{case}{ty:?}");
            check(f, true);
        }
    }
}
#[test]
fn shared_masks_aliases_and_count_results_are_not_duplicated() {
    for alias in 0..3 {
        let mut p = P::new(IrType::U32);
        let x = Operand::Value(Value(0));
        let a = p.mask(x, 255);
        let a = if alias == 1 {
            let dest = Value(p.next);
            p.next += 1;
            p.code.push(Instruction::Copy { dest, src: a });
            Operand::Value(dest)
        } else if alias == 2 {
            let dest = Value(p.next);
            p.next += 1;
            p.code.push(Instruction::Cast {
                dest,
                src: a,
                from_ty: IrType::U32,
                to_ty: IrType::I32,
            });
            Operand::Value(dest)
        } else {
            a
        };
        let b = p.b(IrBinOp::Add, a, p.c(1));
        let b = p.mask(b, 15);
        let root = p.b(IrBinOp::Add, a, b);
        check(p.finish(root), false);
    }
    let mut p = P::new(IrType::U32);
    let a = p.u(IrUnaryOp::Popcount, Operand::Value(Value(0)));
    let b = p.u(IrUnaryOp::Popcount, Operand::Value(Value(1)));
    let s = p.b(IrBinOp::Add, a, b);
    let parity = p.mask(s, 1);
    let root = p.b(IrBinOp::Add, a, parity);
    check(p.finish(root), false);
}
#[test]
fn rejects_even_factors_zero_shifts_wrong_direction_and_nondivisible_remainders() {
    for case in [18, 23, 27, 14] {
        let mut f = fixture(case, IrType::U32);
        for i in &mut f.blocks[0].instructions {
            if let Instruction::BinOp { op, rhs, .. } = i {
                if case == 18 && *op == IrBinOp::Mul {
                    *rhs = Operand::Const(IrConst::I32(6));
                }
                if case == 23 && *op == IrBinOp::Shl {
                    *rhs = Operand::Const(IrConst::I32(0));
                }
                if case == 27 && *op == IrBinOp::LShr {
                    *op = IrBinOp::Shl;
                }
                if case == 14 && *op == IrBinOp::URem {
                    *rhs = Operand::Const(IrConst::I32(49));
                }
            }
        }
        check(f, false);
    }
}
#[test]
fn partial_signed_arithmetic_is_rebuilt_unsigned() {
    let mut f = fixture(0, IrType::I32);
    let old = f.next_value_id;
    assert!(run(&mut f) > 0);
    assert!(f.blocks[0].instructions.iter().any(
        |i| matches!(i,Instruction::BinOp{dest,op:IrBinOp::Add,ty:IrType::U32,..} if dest.0>=old)
    ));
}
#[test]
fn count_casts_use_word_width_and_preserve_narrow_result() {
    let mut p = P::new(IrType::U64);
    let x = Operand::Value(Value(0));
    let y = Operand::Value(Value(1));
    let a = p.u(IrUnaryOp::Popcount, x);
    let b = p.u(IrUnaryOp::Popcount, y);
    let mut cast = |src| {
        let dest = Value(p.next);
        p.next += 1;
        p.code.push(Instruction::Cast {
            dest,
            src,
            from_ty: IrType::U64,
            to_ty: IrType::I32,
        });
        Operand::Value(dest)
    };
    let a = cast(a);
    let b = cast(b);
    p.ty = IrType::I32;
    let sum = p.b(IrBinOp::Add, a, b);
    let root = p.mask(sum, 1);
    let mut f = p.finish(root);
    for i in &mut f.blocks[0].instructions {
        if let Instruction::ParamRef { ty, .. } = i {
            *ty = IrType::U64;
        }
    }
    for param in &mut f.params {
        param.ty = IrType::U64;
    }
    check(f, true);
}
#[test]
fn arithmetic_shift_sign_bit_is_demanded() {
    let mut p = P::new(IrType::U32);
    let x = Operand::Value(Value(0));
    let a = p.mask(x, 255);
    let a = p.b(IrBinOp::AShr, a, p.c(7));
    let root = p.mask(a, 1 << 31);
    check(p.finish(root), false);
}

#[test]
fn nested_count_sources_reach_a_fixed_point() {
    let mut p = P::new(IrType::U64);
    let a = p.u(IrUnaryOp::Neg, Operand::Value(Value(0)));
    let b = p.b(IrBinOp::Mul, a, p.c(3));
    let root = p.u(IrUnaryOp::Ctz, b);
    check(p.finish(root), true);
}
#[test]
fn nonzero_count_forms_and_bit_reverse_are_supported() {
    for op in [
        IrUnaryOp::CtzNonZero,
        IrUnaryOp::ClzNonZero,
        IrUnaryOp::Popcount,
    ] {
        let mut p = P::new(IrType::U64);
        let x = Operand::Value(Value(0));
        let a = match op {
            IrUnaryOp::CtzNonZero => p.u(IrUnaryOp::Neg, x),
            IrUnaryOp::ClzNonZero => {
                let s = p.b(IrBinOp::LShr, x, p.c(7));
                p.b(IrBinOp::Or, x, s)
            }
            _ => p.u(IrUnaryOp::BitReverse, x),
        };
        let root = p.u(op, a);
        check(p.finish(root), true);
    }
}
#[test]
fn rejects_zero_remainder_and_out_of_range_shifts_without_allocating() {
    for shift in [false, true] {
        let mut f = fixture(if shift { 4 } else { 14 }, IrType::U32);
        for i in &mut f.blocks[0].instructions {
            if let Instruction::BinOp { op, rhs, .. } = i {
                if shift && *op == IrBinOp::Shl {
                    *rhs = Operand::Const(IrConst::I32(32));
                }
                if !shift && *op == IrBinOp::URem {
                    *rhs = Operand::Const(IrConst::I32(0));
                }
            }
        }
        let ids = f.next_value_id;
        assert_eq!(run(&mut f), 0);
        assert_eq!(f.next_value_id, ids);
    }
}

#[test]
fn parity_normalizes_nested_permutations_and_not_in_one_run() {
    let mut p = P::new(IrType::U64);
    let mut a = Operand::Value(Value(0));
    for _ in 0..9 {
        a = p.u(IrUnaryOp::Not, a);
    }
    a = p.u(IrUnaryOp::Bswap, a);
    a = p.u(IrUnaryOp::BitReverse, a);
    let count = p.u(IrUnaryOp::Popcount, a);
    let root = p.mask(count, 1);
    check(p.finish(root), true);
}
#[test]
fn carries_and_walk_limits_fail_closed() {
    let mut f = fixture(0, IrType::U32);
    if let Some(Instruction::BinOp { rhs, .. }) = f.blocks[0].instructions.last_mut() {
        *rhs = Operand::Const(IrConst::I32(256));
    }
    check(f, false);
    let mut p = P::new(IrType::U32);
    let mut a = Operand::Value(Value(0));
    for _ in 0..40 {
        a = p.u(IrUnaryOp::Not, a);
    }
    let count = p.u(IrUnaryOp::Popcount, a);
    let root = p.mask(count, 1);
    check(p.finish(root), false);
}

#[test]
fn duplicate_consumer_definitions_fail_closed_without_allocating() {
    let mut f = fixture(0, IrType::U32);
    let dest = f.blocks[0].instructions.last().unwrap().dest().unwrap();
    f.blocks[0].instructions.push(Instruction::ParamRef {
        dest,
        param_idx: 0,
        ty: IrType::U32,
    });
    f.blocks[0]
        .source_spans
        .push(crate::common::source::Span::dummy());
    let ids = f.next_value_id;
    assert_eq!(run(&mut f), 0);
    assert_eq!(f.next_value_id, ids);
}
#[test]
fn cardinality_synthesis_all_two_term_boolean_truth_tables() {
    for ty in [IrType::U32, IrType::U64] {
        for a in 0..16u8 {
            for b in 0..16u8 {
                for op in [IrBinOp::Add, IrBinOp::Sub] {
                    let mut p = P::new(ty);
                    let mut build = |table| {
                        let root = if table == 0 {
                            p.c(0)
                        } else if table == 15 {
                            p.c(full(ty).unwrap())
                        } else {
                            let mut bases =
                                vec![Operand::Value(Value(0)), Operand::Value(Value(1))];
                            // Use a tiny local table decoder independent of the synthesizer.
                            let x = bases.remove(0);
                            let y = bases.remove(0);
                            let mut terms = vec![];
                            for i in 0..4 {
                                if table & (1 << i) != 0 {
                                    let x = if i & 1 == 0 {
                                        p.u(IrUnaryOp::Not, x)
                                    } else {
                                        x
                                    };
                                    let y = if i & 2 == 0 {
                                        p.u(IrUnaryOp::Not, y)
                                    } else {
                                        y
                                    };
                                    terms.push(p.b(IrBinOp::And, x, y));
                                }
                            }
                            let mut root = terms[0];
                            for next in terms.into_iter().skip(1) {
                                root = p.b(IrBinOp::Or, root, next);
                            }
                            root
                        };
                        p.u(IrUnaryOp::Popcount, root)
                    };
                    let lhs = build(a);
                    let rhs = build(b);
                    let root = p.b(op, lhs, rhs);
                    let mut f = p.finish(root);
                    let before = f.clone();
                    run(&mut f);
                    for x in [0, 1, 7, 255, u64::MAX, 1 << 63] {
                        for y in [0, 1, 127, u64::MAX] {
                            assert_eq!(
                                eval(&before, x, y),
                                eval(&f, x, y),
                                "a={a} b={b} op={op:?}"
                            );
                        }
                    }
                    // Idempotence includes budget rejection for large minterm encodings.
                    assert_eq!(run(&mut f), 0, "a={a} b={b} op={op:?}");
                }
            }
        }
    }
}

#[test]
fn weighted_cardinality_handles_doubled_counts_but_not_external_uses() {
    for shared in [false, true] {
        let mut p = P::new(IrType::U64);
        let x = Operand::Value(Value(0));
        let y = Operand::Value(Value(1));
        let a = p.u(IrUnaryOp::Popcount, x);
        let b = p.u(IrUnaryOp::Popcount, y);
        let sum = p.b(IrBinOp::Add, a, b);
        let both = p.b(IrBinOp::And, x, y);
        let both = p.u(IrUnaryOp::Popcount, both);
        let twice = p.b(IrBinOp::Add, both, both);
        let root = p.b(IrBinOp::Sub, sum, twice);
        let root = if shared {
            p.b(IrBinOp::Add, root, both)
        } else {
            root
        };
        check(p.finish(root), !shared);
    }
}
#[test]
fn redefined_count_inputs_do_not_supply_stable_observations() {
    let mut p = P::new(IrType::U32);
    let x = Operand::Value(Value(0));
    let a = p.u(IrUnaryOp::Popcount, x);
    p.code.push(Instruction::ParamRef {
        dest: Value(0),
        param_idx: 1,
        ty: IrType::U32,
    });
    let n = p.u(IrUnaryOp::Not, x);
    let b = p.u(IrUnaryOp::Popcount, n);
    let root = p.b(IrBinOp::Add, a, b);
    let mut f = p.finish(root);
    let before = format!("{:?}", f.blocks);
    assert_eq!(run(&mut f), 0);
    assert_eq!(format!("{:?}", f.blocks), before);
}

#[test]
fn cardinality_and_parity_compositions_use_fresh_definitions_and_edges() {
    for variant in 0..3 {
        let mut p = P::new(IrType::U64);
        let x = Operand::Value(Value(0));
        let y = Operand::Value(Value(1));
        let a = p.b(IrBinOp::And, x, y);
        let a = p.u(IrUnaryOp::Popcount, a);
        let b = p.b(IrBinOp::Xor, x, y);
        let b = p.u(IrUnaryOp::Popcount, b);
        let sum = p.b(IrBinOp::Add, a, b);
        let root = if variant == 0 {
            p.mask(sum, 1)
        } else {
            let n = p.u(IrUnaryOp::Not, y);
            let c = p.b(IrBinOp::And, x, n);
            let c = p.u(IrUnaryOp::Popcount, c);
            let r = p.b(IrBinOp::Sub, sum, c);
            if variant == 1 { r } else { p.mask(r, 1) }
        };
        let mut f = p.finish(root);
        let before = f.clone();
        assert!(run(&mut f) > 0);
        assert_eq!(run(&mut f), 0);
        // DCE is intentionally separate; prove the return's reachable graph
        // contains exactly one count, not revived obsolete count operands.
        let mut seen = std::collections::HashSet::new();
        let mut pending = vec![];
        f.blocks[0]
            .terminator
            .for_each_used_value(|id| pending.push(id));
        while let Some(id) = pending.pop() {
            if seen.insert(id) {
                if let Some(i) = f.blocks[0]
                    .instructions
                    .iter()
                    .find(|i| i.dest() == Some(Value(id)))
                {
                    i.for_each_used_value(|next| pending.push(next));
                }
            }
        }
        assert_eq!(f.blocks[0].instructions.iter().filter(|i|matches!(i,Instruction::UnaryOp{dest,op:IrUnaryOp::Popcount,..} if seen.contains(&dest.0))).count(),1);
        for x in [0, 1, 127, u64::MAX, 1 << 63] {
            for y in [0, 1, 255, u64::MAX] {
                assert_eq!(eval(&before, x, y), eval(&f, x, y));
            }
        }
    }
}
