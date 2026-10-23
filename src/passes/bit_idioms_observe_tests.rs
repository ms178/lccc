use super::*;
use crate::ir::reexports::{BasicBlock, BlockId, Terminator};

struct Program {
    instructions: Vec<Instruction>,
    ty: IrType,
    next: u32,
}
impl Program {
    fn new(ty: IrType) -> Self {
        Self {
            instructions: vec![],
            ty,
            next: 3,
        }
    }
    fn c(&self, n: u64) -> Operand {
        Operand::Const(IrConst::from_i64(n as i64, self.ty))
    }
    fn unary(&mut self, op: IrUnaryOp, src: Operand) -> Operand {
        let dest = Value(self.next);
        self.next += 1;
        self.instructions.push(Instruction::UnaryOp {
            dest,
            op,
            src,
            ty: self.ty,
        });
        Operand::Value(dest)
    }
    fn bin(&mut self, op: IrBinOp, lhs: Operand, rhs: Operand) -> Operand {
        let dest = Value(self.next);
        self.next += 1;
        self.instructions.push(Instruction::BinOp {
            dest,
            op,
            lhs,
            rhs,
            ty: self.ty,
        });
        Operand::Value(dest)
    }
    fn choice(&mut self, a: Operand, b: Operand) -> Operand {
        let dest = Value(self.next);
        self.next += 1;
        self.instructions.push(Instruction::Select {
            dest,
            cond: Operand::Value(Value(2)),
            true_val: a,
            false_val: b,
            ty: self.ty,
        });
        Operand::Value(dest)
    }
    fn compare(&mut self, a: Operand, b: Operand, op: IrCmpOp) -> Operand {
        let dest = Value(self.next);
        self.next += 1;
        self.instructions.push(Instruction::Cmp {
            dest,
            op,
            lhs: a,
            rhs: b,
            ty: self.ty,
        });
        Operand::Value(dest)
    }
    fn finish(self, result: Operand) -> IrFunction {
        let mut f = IrFunction::new("observe".into(), self.ty, vec![], false);
        f.next_value_id = self.next;
        f.next_label = 1;
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            source_spans: vec![crate::common::source::Span::dummy(); self.instructions.len()],
            instructions: self.instructions,
            terminator: Terminator::Return(Some(result)),
        });
        f
    }
}
fn fixture(case: usize, ty: IrType, ne: bool) -> IrFunction {
    let mut p = Program::new(ty);
    let x = Operand::Value(Value(0));
    let y = Operand::Value(Value(1));
    let c = Operand::Value(Value(2));
    let z = p.c(0);
    let seven = p.c(7);
    let all = full(ty).unwrap();
    let width = ty.size() * 8;
    let (a, b) = match case {
        0..=12 => {
            let branch = match case {
                0 => {
                    let k = p.c(3);
                    p.bin(IrBinOp::Mul, x, k)
                }
                1 | 2 => {
                    let n = p.unary(IrUnaryOp::Neg, x);
                    p.bin(if case == 1 { IrBinOp::Or } else { IrBinOp::And }, x, n)
                }
                3..=9 => {
                    let s = p.bin(
                        if matches!(case, 4 | 6) {
                            IrBinOp::LShr
                        } else {
                            IrBinOp::Shl
                        },
                        x,
                        seven,
                    );
                    let op = match case {
                        3 | 4 => IrBinOp::Or,
                        5 | 6 => IrBinOp::Xor,
                        7 => IrBinOp::Add,
                        _ => IrBinOp::Sub,
                    };
                    if case == 9 {
                        p.bin(op, s, x)
                    } else {
                        p.bin(op, x, s)
                    }
                }
                10 => p.unary(IrUnaryOp::Bswap, x),
                11 => p.bin(IrBinOp::RotateLeft, x, seven),
                _ => p.unary(IrUnaryOp::Popcount, x),
            };
            (p.choice(branch, x), z)
        }
        13..=15 => {
            let five = p.c(5);
            let negfive = p.c(5u64.wrapping_neg());
            let a = match case {
                13 => p.bin(IrBinOp::Add, x, five),
                14 => p.bin(IrBinOp::Sub, five, x),
                _ => p.bin(IrBinOp::Sub, x, five),
            };
            let target = if case == 15 { negfive } else { five };
            let b = p.bin(IrBinOp::Xor, x, target);
            (p.choice(a, b), target)
        }
        16 => {
            let three = p.c(3);
            let k = p.c(21);
            (p.bin(IrBinOp::Mul, x, three), k)
        }
        17 => {
            let k = p.c(width as u64);
            (p.unary(IrUnaryOp::Popcount, x), k)
        }
        18 => (p.unary(IrUnaryOp::Clz, x), z),
        19 => (p.unary(IrUnaryOp::Ctz, x), z),
        20 | 21 => {
            let k = p.c(if case == 20 { 0 } else { all >> 7 });
            (p.bin(IrBinOp::LShr, x, seven), k)
        }
        22 | 23 => {
            let k = p.c(if case == 22 { 0 } else { all });
            (p.bin(IrBinOp::AShr, x, seven), k)
        }
        24 => (p.bin(IrBinOp::Shl, x, seven), z),
        25 => {
            let m = p.c(0x5a5a);
            (p.bin(IrBinOp::Or, x, m), m)
        }
        26 => {
            let n = p.unary(IrUnaryOp::Not, x);
            let m = p.c(0x5a5a);
            (p.bin(IrBinOp::And, n, m), z)
        }
        27 => {
            let d = p.c(37);
            (p.bin(IrBinOp::UDiv, x, d), z)
        }
        28 => {
            let n = p.unary(IrUnaryOp::Popcount, x);
            let r = p.compare(n, x, IrCmpOp::Ule);
            return p.finish(r);
        }
        29..=31 => {
            let (a, b) = if case == 31 {
                (p.bin(IrBinOp::Sub, c, x), p.bin(IrBinOp::Sub, c, y))
            } else {
                let op = if case == 29 {
                    IrBinOp::Add
                } else {
                    IrBinOp::Sub
                };
                (p.bin(op, x, c), p.bin(op, y, c))
            };
            (a, b)
        }
        32 => (p.bin(IrBinOp::Xor, x, c), p.bin(IrBinOp::Xor, y, c)),
        33..=35 => {
            let op = match case {
                33 => IrUnaryOp::Neg,
                34 => IrUnaryOp::Not,
                _ => IrUnaryOp::Bswap,
            };
            (p.unary(op, x), p.unary(op, y))
        }
        36 => (
            p.bin(IrBinOp::RotateLeft, x, seven),
            p.bin(IrBinOp::RotateLeft, y, seven),
        ),
        37..=43 => {
            let a = p.unary(IrUnaryOp::Bswap, x);
            let b = p.unary(IrUnaryOp::Bswap, y);
            let inner = match case {
                37..=39 => p.bin(
                    match case {
                        37 => IrBinOp::And,
                        38 => IrBinOp::Or,
                        _ => IrBinOp::Xor,
                    },
                    a,
                    b,
                ),
                40 => p.unary(IrUnaryOp::Not, a),
                41 | 42 => {
                    let eight = p.c(8);
                    p.bin(
                        if case == 41 {
                            IrBinOp::Shl
                        } else {
                            IrBinOp::LShr
                        },
                        a,
                        eight,
                    )
                }
                _ => p.choice(a, b),
            };
            let r = p.unary(IrUnaryOp::Bswap, inner);
            return p.finish(r);
        }
        44 => {
            let d = p.c(48);
            let m = p.c(15);
            let r = p.bin(IrBinOp::URem, x, d);
            let r = p.bin(IrBinOp::And, r, m);
            return p.finish(r);
        }
        45 => {
            // Case 28 with the operands swapped: `x >= ctpop(x)` / `x < ctpop(x)`.
            let n = p.unary(IrUnaryOp::Popcount, x);
            let r = p.compare(x, n, if ne { IrCmpOp::Ult } else { IrCmpOp::Uge });
            return p.finish(r);
        }
        _ => unreachable!(),
    };
    let result = p.compare(a, b, if ne { IrCmpOp::Ne } else { IrCmpOp::Eq });
    p.finish(result)
}
fn definitions(f: &IrFunction) -> Vec<Option<Instruction>> {
    let mut defs = vec![None; f.max_value_id() as usize + 1];
    for i in &f.blocks[0].instructions {
        defs[i.dest().unwrap().0 as usize] = Some(i.clone());
    }
    defs
}
fn evaluate(f: &IrFunction, x: u64, y: u64, c: u64) -> u64 {
    let ty = f.return_type;
    let mask = full(ty).unwrap();
    let width = ty.size() * 8;
    let mut values = vec![0; f.max_value_id() as usize + 1];
    values[0] = x & mask;
    values[1] = y & mask;
    values[2] = c & mask;
    for i in &f.blocks[0].instructions {
        let get = |o: Operand| match o {
            Operand::Value(v) => values[v.0 as usize],
            _ => const_in_ring(o, width).unwrap(),
        };
        let n = match i {
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
                    IrBinOp::UDiv => a / b,
                    IrBinOp::URem => a % b,
                    _ => panic!("unsupported {op:?}"),
                }
            }
            Instruction::UnaryOp { op, src, .. } => {
                let a = get(*src);
                match op {
                    IrUnaryOp::Neg => a.wrapping_neg(),
                    IrUnaryOp::Not => !a,
                    IrUnaryOp::Bswap => swap(a, ty),
                    IrUnaryOp::BitReverse => a.reverse_bits() >> (64 - width),
                    IrUnaryOp::Popcount => a.count_ones() as u64,
                    IrUnaryOp::Clz => {
                        if width == 32 {
                            (a as u32).leading_zeros() as u64
                        } else {
                            a.leading_zeros() as u64
                        }
                    }
                    IrUnaryOp::Ctz => {
                        if width == 32 {
                            (a as u32).trailing_zeros() as u64
                        } else {
                            a.trailing_zeros() as u64
                        }
                    }
                    _ => panic!("unsupported {op:?}"),
                }
            }
            Instruction::Cmp { op, lhs, rhs, .. } => {
                let a = get(*lhs);
                let b = get(*rhs);
                match op {
                    IrCmpOp::Eq => (a == b) as u64,
                    IrCmpOp::Ne => (a != b) as u64,
                    IrCmpOp::Ult => (a < b) as u64,
                    IrCmpOp::Uge => (a >= b) as u64,
                    IrCmpOp::Ule => (a <= b) as u64,
                    _ => panic!("unsupported {op:?}"),
                }
            }
            Instruction::Select {
                cond,
                true_val,
                false_val,
                ..
            } => get(if get(*cond) != 0 {
                *true_val
            } else {
                *false_val
            }),
            Instruction::Copy { src, .. } => get(*src),
            _ => panic!("unsupported instruction"),
        };
        values[i.dest().unwrap().0 as usize] = n & mask;
    }
    let Terminator::Return(Some(Operand::Value(v))) = f.blocks[0].terminator else {
        panic!()
    };
    values[v.0 as usize]
}
#[test]
fn production_46_families_both_widths_polarities_and_high_bits() {
    for ty in [IrType::U32, IrType::U64, IrType::I32, IrType::I64] {
        for case in 0..46 {
            for ne in [false, true] {
                let before = fixture(case, ty, ne);
                let mut after = before.clone();
                let defs = definitions(&after);
                assert!(run(&mut after, &defs) > 0, "case={case} ty={ty:?}");
                assert_eq!(
                    after.blocks[0].source_spans.len(),
                    after.blocks[0].instructions.len()
                );
                let defs = definitions(&after);
                let ids = after.next_value_id;
                assert_eq!(
                    run(&mut after, &defs),
                    0,
                    "not idempotent case={case} ty={ty:?}"
                );
                assert_eq!(after.next_value_id, ids);
                for low in 0..1024u64 {
                    for high in [0, 1u64 << (ty.size() * 8 - 1), full(ty).unwrap() & !1023] {
                        let x = high | low;
                        let y = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                        for c in [0, 1, 3] {
                            assert_eq!(
                                evaluate(&before, x, y, c),
                                evaluate(&after, x, y, c),
                                "case={case} ty={ty:?} ne={ne} x={x} y={y} c={c}"
                            );
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn rejects_even_multipliers_zero_shifts_width_changes_and_nonbyte_swaps() {
    for variant in 0..6 {
        let mut f = fixture(
            if variant == 0 {
                0
            } else if variant == 1 {
                5
            } else if variant == 4 {
                44
            } else {
                41
            },
            IrType::U32,
            false,
        );
        match variant {
            0 => {
                if let Instruction::BinOp { rhs, .. } = &mut f.blocks[0].instructions[0] {
                    *rhs = Operand::Const(IrConst::I32(6));
                }
            }
            1 => {
                if let Instruction::BinOp { rhs, .. } = &mut f.blocks[0].instructions[0] {
                    *rhs = Operand::Const(IrConst::I32(0));
                }
            }
            2 => {
                if let Instruction::UnaryOp { ty, .. } = &mut f.blocks[0].instructions[0] {
                    *ty = IrType::U64;
                }
            }
            3 => {
                if let Instruction::BinOp { rhs, .. } = &mut f.blocks[0].instructions[2] {
                    *rhs = Operand::Const(IrConst::I32(7));
                }
            }
            4 => {
                if let Instruction::BinOp { rhs, .. } = &mut f.blocks[0].instructions[0] {
                    *rhs = Operand::Const(IrConst::I32(47));
                }
            }
            5 => {
                if let Instruction::BinOp { op, .. } = &mut f.blocks[0].instructions[2] {
                    *op = IrBinOp::AShr;
                }
            }
            _ => unreachable!(),
        }
        let defs = definitions(&f);
        assert_eq!(run(&mut f, &defs), 0, "variant={variant}");
    }
}
#[test]
fn odd_inverse_exhaustive_low_words_and_64_bit_boundaries() {
    for n in (1..65536u64)
        .step_by(2)
        .chain([u64::MAX, u64::MAX - 2, 0x8000000000000001])
    {
        assert_eq!(n.wrapping_mul(inverse_odd(n)), 1);
    }
}

#[test]
fn defined_count_width_boundaries_and_bit_reversal_observations() {
    for ty in [IrType::U32, IrType::U64] {
        for case in [10, 18, 19, 35] {
            let mut before = fixture(case, ty, false);
            for i in &mut before.blocks[0].instructions {
                match i {
                    Instruction::UnaryOp {
                        op: IrUnaryOp::Bswap,
                        ..
                    } => {
                        if let Instruction::UnaryOp { op, .. } = i {
                            *op = IrUnaryOp::BitReverse;
                        }
                    }
                    Instruction::Cmp { rhs, .. } if matches!(case, 18 | 19) => {
                        *rhs = Operand::Const(IrConst::from_i64((ty.size() * 8) as i64, ty));
                    }
                    _ => (),
                }
            }
            let mut after = before.clone();
            let defs = definitions(&after);
            assert!(run(&mut after, &defs) > 0);
            for x in [0, 1, 2, 0x80000000, u32::MAX as u64, 1u64 << 63, u64::MAX] {
                for y in [x, x ^ 1] {
                    assert_eq!(evaluate(&before, x, y, 1), evaluate(&after, x, y, 1));
                }
            }
        }
    }
}

// Cost rule: peeling a producer that outlives the compare must pay for itself.
fn cmp_operands(f: &IrFunction) -> Vec<(IrCmpOp, Operand, Operand)> {
    f.blocks[0]
        .instructions
        .iter()
        .filter_map(|i| match i {
            Instruction::Cmp { op, lhs, rhs, .. } => Some((*op, *lhs, *rhs)),
            _ => None,
        })
        .collect()
}

fn run_and_check(before: &IrFunction) -> (usize, IrFunction) {
    let mut after = before.clone();
    let defs = definitions(&after);
    let n = run(&mut after, &defs);
    for x in [
        0,
        1,
        2,
        3,
        9,
        31,
        0x80000000,
        u32::MAX as u64,
        1u64 << 63,
        u64::MAX,
    ] {
        for y in [0, 1, 5, x ^ 4] {
            assert_eq!(
                evaluate(before, x, y, 7),
                evaluate(&after, x, y, 7),
                "x={x:#x} y={y:#x}"
            );
        }
    }
    (n, after)
}

#[test]
fn shared_single_cycle_producer_keeps_its_compare() {
    // hb = x ^ 31 has a second user, so `hb == 0 -> x == 31` would keep the
    // XOR, save nothing, and stretch x across the compare (the `31 - clz`
    // highbit shape whose x86 XOR pair can then no longer cancel).
    for ty in [IrType::U32, IrType::U64] {
        let mut p = Program::new(ty);
        let x = Operand::Value(Value(0));
        let k = p.c(31);
        let hb = p.bin(IrBinOp::Xor, x, k);
        let zero = p.c(0);
        let is_zero = p.compare(hb, zero, IrCmpOp::Eq);
        let five = p.c(5);
        let other = p.bin(IrBinOp::Add, hb, five);
        let result = p.bin(IrBinOp::Add, is_zero, other);
        let before = p.finish(result);
        let (_, after) = run_and_check(&before);
        assert_eq!(
            cmp_operands(&after),
            vec![(IrCmpOp::Eq, hb, zero)],
            "{ty:?}"
        );
    }
}

#[test]
fn dying_single_cycle_producer_still_peels() {
    for ty in [IrType::U32, IrType::U64] {
        let mut p = Program::new(ty);
        let x = Operand::Value(Value(0));
        let k = p.c(31);
        let hb = p.bin(IrBinOp::Xor, x, k);
        let zero = p.c(0);
        let result = p.compare(hb, zero, IrCmpOp::Eq);
        let before = p.finish(result);
        let (n, after) = run_and_check(&before);
        assert!(n > 0, "{ty:?}");
        let cmps = cmp_operands(&after);
        assert_eq!(cmps.len(), 1);
        assert_eq!((cmps[0].0, cmps[0].1), (IrCmpOp::Eq, x), "{ty:?}");
        assert_eq!(const_in_ring(cmps[0].2, ty.size() * 8), Some(31));
    }
}

#[test]
fn shared_multi_cycle_producer_still_peels() {
    // An odd multiply keeps running for its other user, but the compare
    // leaves its 3-cycle latency chain: the peel still pays.
    for ty in [IrType::U32, IrType::U64] {
        let mut p = Program::new(ty);
        let x = Operand::Value(Value(0));
        let three = p.c(3);
        let m = p.bin(IrBinOp::Mul, x, three);
        let nine = p.c(9);
        let is_nine = p.compare(m, nine, IrCmpOp::Eq);
        let result = p.bin(IrBinOp::Add, is_nine, m);
        let before = p.finish(result);
        let (n, after) = run_and_check(&before);
        assert!(n > 0, "{ty:?}");
        let cmps = cmp_operands(&after);
        assert_eq!((cmps[0].0, cmps[0].1), (IrCmpOp::Eq, x), "{ty:?}");
        assert_eq!(const_in_ring(cmps[0].2, ty.size() * 8), Some(3));
    }
}

#[test]
fn shared_producer_still_allows_direct_bit_materialization() {
    // ctz(x ^ 2) == 0 observes bit 0 of x through a shared XOR. Replacing
    // cmp+setcc+movzx by one AND is cheaper than the compare regardless.
    let mut p = Program::new(IrType::U32);
    let x = Operand::Value(Value(0));
    let two = p.c(2);
    let t = p.bin(IrBinOp::Xor, x, two);
    let tz = p.unary(IrUnaryOp::Ctz, t);
    let zero = p.c(0);
    let odd = p.compare(tz, zero, IrCmpOp::Eq);
    let result = p.bin(IrBinOp::Add, odd, t);
    let before = p.finish(result);
    let (n, after) = run_and_check(&before);
    assert!(n > 0);
    assert!(
        cmp_operands(&after).is_empty(),
        "{:?}",
        after.blocks[0].instructions
    );
}

#[test]
fn cancel_requires_one_side_to_pay() {
    // (x ^ 7) == (y ^ 7) -> x == y only if at least one XOR dies with it.
    for shared_sides in 0..=2usize {
        let mut p = Program::new(IrType::U64);
        let (x, y) = (Operand::Value(Value(0)), Operand::Value(Value(1)));
        let k = p.c(7);
        let a = p.bin(IrBinOp::Xor, x, k);
        let b = p.bin(IrBinOp::Xor, y, k);
        let eq = p.compare(a, b, IrCmpOp::Eq);
        let mut result = eq;
        for side in [a, b].into_iter().take(shared_sides) {
            result = p.bin(IrBinOp::Add, result, side);
        }
        let before = p.finish(result);
        let (_, after) = run_and_check(&before);
        let cmps = cmp_operands(&after);
        if shared_sides == 2 {
            assert_eq!(cmps, vec![(IrCmpOp::Eq, a, b)]);
        } else {
            assert_eq!(
                cmps,
                vec![(IrCmpOp::Eq, x, y)],
                "shared_sides={shared_sides}"
            );
        }
    }
}
