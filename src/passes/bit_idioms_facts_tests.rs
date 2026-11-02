use super::*;

fn v(n: u32) -> Operand {
    Operand::Value(Value(n))
}
struct Builder {
    ty: IrType,
    insts: Vec<Instruction>,
    next: u32,
}
impl Builder {
    fn new(ty: IrType) -> Self {
        Self {
            ty,
            insts: vec![],
            next: 3,
        }
    }
    fn push(&mut self, make: impl FnOnce(Value) -> Instruction) -> Operand {
        let d = Value(self.next);
        self.next += 1;
        self.insts.push(make(d));
        Operand::Value(d)
    }
    fn bin(&mut self, op: IrBinOp, a: Operand, b: Operand) -> Operand {
        let ty = self.ty;
        self.push(|d| bin(d, op, a, b, ty))
    }
    fn and_bool(&mut self, a: Operand, b: Operand) -> Operand {
        self.push(|d| bin(d, IrBinOp::And, a, b, IrType::I32))
    }
    fn cmp(&mut self, op: IrCmpOp, a: Operand, b: Operand) -> Operand {
        let ty = self.ty;
        self.push(|dest| Instruction::Cmp {
            dest,
            op,
            lhs: a,
            rhs: b,
            ty,
        })
    }
    fn c(&self, n: u64) -> Operand {
        constant(n, self.ty)
    }
    fn fold(&self) -> Option<Vec<Instruction>> {
        let mut defs = vec![None; self.next as usize];
        let mut uses = vec![0; defs.len()];
        for i in &self.insts {
            defs[i.dest().unwrap().0 as usize] = Some(i.clone());
            crate::backend::liveness::for_each_operand_in_instruction(i, |o| {
                if let Operand::Value(v) = o {
                    uses[v.0 as usize] += 1;
                }
            });
        }
        fold(
            self.insts.last().unwrap(),
            &Context {
                defs: &defs,
                uses: &uses,
                visits: std::cell::Cell::new(512),
                disable_logic: false,
                wrap_signed_overflow: true,
            },
            &Facts(vec![]),
            Value(self.next),
        )
    }
    fn verify(&self, samples: impl IntoIterator<Item = [u64; 3]>) {
        let replacement = self.fold().expect("production matcher must fire");
        for args in samples {
            let mut old = vec![0; self.next as usize + 1];
            old[..3].copy_from_slice(&args);
            let mut new = old.clone();
            for i in &self.insts {
                eval(i, &mut old);
            }
            for i in &self.insts[..self.insts.len() - 1] {
                eval(i, &mut new);
            }
            for i in &replacement {
                eval(i, &mut new);
            }
            assert_eq!(
                old[self.next as usize - 1],
                new[self.next as usize - 1],
                "args={args:?} ty={:?}",
                self.ty
            );
        }
    }
}
fn eval(i: &Instruction, values: &mut [u64]) {
    let get = |o: Operand| match o {
        Operand::Value(v) => values[v.0 as usize],
        Operand::Const(c) => c.to_i128().unwrap() as u64,
    };
    let mask = |ty: IrType| u64::MAX >> (64 - ty.size() * 8);
    let n = match i {
        Instruction::Copy { src, .. } => get(*src),
        Instruction::Cast { src, to_ty, .. } => get(*src) & mask(*to_ty),
        Instruction::BinOp {
            op, lhs, rhs, ty, ..
        } => {
            let a = get(*lhs) & mask(*ty);
            let b = get(*rhs) & mask(*ty);
            let n = match op {
                IrBinOp::Add => a.wrapping_add(b),
                IrBinOp::Sub => a.wrapping_sub(b),
                IrBinOp::Mul => a.wrapping_mul(b),
                IrBinOp::UDiv => a / b,
                IrBinOp::URem => a % b,
                IrBinOp::And => a & b,
                IrBinOp::Or => a | b,
                IrBinOp::Xor => a ^ b,
                IrBinOp::LShr => a >> b,
                IrBinOp::Shl => a << b,
                _ => panic!("unsupported evaluator op"),
            };
            n & mask(*ty)
        }
        Instruction::Cmp {
            op, lhs, rhs, ty, ..
        } => {
            let a = get(*lhs) & mask(*ty);
            let b = get(*rhs) & mask(*ty);
            let w = ty.size() * 8;
            let s = |x: u64| ((x << (64 - w)) as i64) >> (64 - w);
            let value = match op {
                IrCmpOp::Eq => a == b,
                IrCmpOp::Ne => a != b,
                IrCmpOp::Ult => a < b,
                IrCmpOp::Ule => a <= b,
                IrCmpOp::Ugt => a > b,
                IrCmpOp::Uge => a >= b,
                IrCmpOp::Slt => s(a) < s(b),
                IrCmpOp::Sle => s(a) <= s(b),
                IrCmpOp::Sgt => s(a) > s(b),
                IrCmpOp::Sge => s(a) >= s(b),
            };
            u64::from(value)
        }
        Instruction::UnaryOp {
            op: IrUnaryOp::Neg,
            src,
            ty,
            ..
        } => get(*src).wrapping_neg() & mask(*ty),
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
        _ => panic!("unsupported evaluator instruction"),
    };
    values[i.dest().unwrap().0 as usize] = n;
}
fn unary_samples(ty: IrType) -> impl Iterator<Item = [u64; 3]> {
    let w = ty.size() * 8;
    let mask = u64::MAX >> (64 - w);
    (0..=u16::MAX as u64)
        .flat_map(move |x| [x, x | (1u64 << (w - 1)), mask.wrapping_sub(x) & mask])
        .map(|x| [x, 0, 0])
}

#[test]
fn masked_guard_implication_and_range_mask_have_exhaustive_small_domain_proofs() {
    for ty in [IrType::U8, IrType::U16, IrType::U32, IrType::U64] {
        let mut b = Builder::new(ty);
        let different = b.cmp(IrCmpOp::Ne, v(0), b.c(13));
        let null = b.cmp(IrCmpOp::Eq, v(1), b.c(0));
        let other = b.push(|d| bin(d, IrBinOp::Or, different, null, IrType::I32));
        let m = b.bin(IrBinOp::And, v(0), b.c(u64::MAX - 1));
        let wanted = b.cmp(IrCmpOp::Eq, m, b.c(42));
        b.and_bool(other, wanted);
        b.verify((0..=255).flat_map(|x| [[x, 0, 0], [x, 1, 0]]));
        let mut b = Builder::new(ty);
        let plus = b.bin(IrBinOp::Add, v(0), b.c(1));
        let guard = b.cmp(IrCmpOp::Ult, plus, b.c(3));
        let m = (1u64 << (ty.size() * 8 - 1)) | 1;
        let bits = b.bin(IrBinOp::And, v(0), b.c(m));
        let wanted = b.cmp(IrCmpOp::Eq, bits, b.c(m));
        b.and_bool(guard, wanted);
        b.verify(unary_samples(ty));
    }
}

#[test]
fn guarded_constant_substitution_keeps_shared_producers_and_other_comparison_uses() {
    for ty in [IrType::U8, IrType::U32, IrType::U64] {
        let mut b = Builder::new(ty);
        let zero = b.cmp(IrCmpOp::Eq, v(0), b.c(0));
        let sum = b.bin(IrBinOp::Add, v(0), v(1));
        let eq = b.cmp(IrCmpOp::Eq, sum, v(2));
        b.and_bool(zero, eq);
        let plan = b.fold().unwrap();
        assert_eq!(plan.len(), 2);
        assert!(matches!(&plan[0],Instruction::Cmp{lhs,rhs,..}if *lhs==v(1)&&*rhs==v(2)));
        b.verify(
            (0..256)
                .flat_map(|x| (0..256).flat_map(move |y| [[x, y, y], [x, y, y + 1], [x, y, 0]])),
        );
    }
}

#[test]
fn select_uses_joint_facts_but_never_equates_distinct_nonboolean_values() {
    let mut b = Builder::new(IrType::U32);
    let x = b.cmp(IrCmpOp::Eq, v(0), b.c(1));
    let y = b.cmp(IrCmpOp::Eq, v(1), b.c(0));
    let guard = b.and_bool(x, y);
    let yes = b.cmp(IrCmpOp::Ne, v(1), v(0));
    let no = b.cmp(IrCmpOp::Ugt, v(0), v(1));
    b.push(|dest| Instruction::Select {
        dest,
        cond: guard,
        true_val: yes,
        false_val: no,
        ty: IrType::I32,
    });
    b.verify((0..256).flat_map(|x| (0..256).map(move |y| [x, y, 0])));
    let mut bad = Builder::new(IrType::U32);
    let guard = bad.cmp(IrCmpOp::Eq, v(0), bad.c(1));
    bad.push(|dest| Instruction::Select {
        dest,
        cond: guard,
        true_val: constant(2, IrType::U32),
        false_val: constant(3, IrType::U32),
        ty: IrType::U32,
    });
    assert!(bad.fold().is_none());
}

#[test]
fn masked_trip_proof_covers_wrap_and_declines_visible_carry_and_bad_shift() {
    for ty in [IrType::U32, IrType::U64] {
        for a in [12, 28, u64::MAX - 3] {
            let mut b = Builder::new(ty);
            let x = b.bin(IrBinOp::Add, v(0), b.c(a));
            let q = b.bin(IrBinOp::LShr, x, b.c(2));
            let n = b.bin(IrBinOp::Add, q, b.c(1));
            b.bin(IrBinOp::And, n, b.c(3));
            b.verify(unary_samples(ty));
        }
        for (a, s, m) in [(11, 2, 3), (12, 2, 7), (12, 0, 3), (12, 64, 3)] {
            let mut b = Builder::new(ty);
            let x = b.bin(IrBinOp::Add, v(0), b.c(a));
            let q = b.bin(IrBinOp::LShr, x, b.c(s));
            let n = b.bin(IrBinOp::Add, q, b.c(1));
            b.bin(IrBinOp::And, n, b.c(m));
            assert!(b.fold().is_none());
        }
    }
}

#[test]
fn shared_and_masked_divrem_recombination_proofs_and_near_misses() {
    for ty in [IrType::U32, IrType::U64] {
        let mut b = Builder::new(ty);
        let q = b.bin(IrBinOp::UDiv, v(0), b.c(6));
        let r = b.bin(IrBinOp::URem, v(0), b.c(6));
        let r = b.bin(IrBinOp::Shl, r, b.c(1));
        b.bin(IrBinOp::Xor, r, b.c(1));
        let q = b.bin(IrBinOp::Mul, q, b.c(12));
        b.bin(IrBinOp::Add, q, r);
        b.verify(unary_samples(ty));
        for (qs, mask, should_fold) in [
            (24576, 30720, true),
            (24577, 30720, false),
            (24576, 28672, true),
            (24576, 26624, false),
        ] {
            let mut b = Builder::new(ty);
            let q = b.bin(IrBinOp::LShr, v(0), b.c(4));
            let r = b.bin(IrBinOp::And, v(0), b.c(15));
            let q = b.bin(IrBinOp::Mul, q, b.c(qs));
            let r = b.bin(IrBinOp::Mul, r, b.c(1536));
            let r = b.bin(IrBinOp::And, r, b.c(mask));
            b.bin(IrBinOp::Add, q, r);
            if should_fold {
                b.verify(unary_samples(ty));
            } else {
                assert!(b.fold().is_none());
            }
        }
    }
}

#[test]
fn lowbit_observations_and_boolean_or_are_width_exact() {
    for ty in [IrType::U8, IrType::U16, IrType::U32, IrType::U64] {
        for k in [0, 1] {
            for op in [IrCmpOp::Eq, IrCmpOp::Ne] {
                let mut b = Builder::new(ty);
                let neg = b.bin(IrBinOp::Sub, b.c(0), v(0));
                let low = b.bin(IrBinOp::And, v(0), neg);
                b.cmp(op, low, b.c(k));
                b.verify(unary_samples(ty));
            }
        }
        let mut b = Builder::new(ty);
        let cmp = b.cmp(IrCmpOp::Ne, v(0), b.c(0));
        let cast = b.push(|dest| Instruction::Cast {
            dest,
            src: cmp,
            from_ty: IrType::I32,
            to_ty: ty,
        });
        let x = b.bin(IrBinOp::Or, v(0), cast);
        b.cmp(IrCmpOp::Ne, x, b.c(0));
        b.verify(unary_samples(ty));
    }
}

#[test]
fn conflicting_widths_signed_division_and_casted_lowbits_are_not_transparent() {
    let mut b = Builder::new(IrType::I32);
    let q = b.bin(IrBinOp::UDiv, v(0), b.c(6));
    let r = b.bin(IrBinOp::URem, v(0), b.c(6));
    let q = b.bin(IrBinOp::Mul, q, b.c(12));
    let r = b.bin(IrBinOp::Mul, r, b.c(2));
    b.bin(IrBinOp::Add, q, r);
    assert!(b.fold().is_none());
    let mut b = Builder::new(IrType::U64);
    let neg = b.bin(IrBinOp::Sub, b.c(0), v(0));
    let narrow = b.push(|dest| Instruction::Cast {
        dest,
        src: neg,
        from_ty: IrType::U64,
        to_ty: IrType::U32,
    });
    let low = b.bin(IrBinOp::And, v(0), narrow);
    b.cmp(IrCmpOp::Eq, low, b.c(1));
    assert!(b.fold().is_none());
}

#[test]
fn edge_facts_require_one_real_conditional_edge_and_preserve_spans() {
    use crate::common::source::Span;
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};
    for variant in 0..4 {
        let make = |label, instructions: Vec<Instruction>, terminator| BasicBlock {
            label: BlockId(label),
            source_spans: vec![Span::dummy(); instructions.len()],
            instructions,
            terminator,
        };
        let mut f = IrFunction::new("edge_facts".into(), IrType::I32, vec![], false);
        f.blocks = vec![
            make(
                0,
                vec![Instruction::Cmp {
                    dest: Value(3),
                    op: IrCmpOp::Ult,
                    lhs: v(0),
                    rhs: constant(13, IrType::U32),
                    ty: IrType::U32,
                }],
                Terminator::CondBranch {
                    cond: v(3),
                    true_label: BlockId(1),
                    false_label: BlockId(if variant == 3 { 1 } else { 2 }),
                },
            ),
            make(
                1,
                vec![
                    bin(
                        Value(4),
                        IrBinOp::And,
                        v(0),
                        constant(16, IrType::U32),
                        IrType::U32,
                    ),
                    Instruction::Cmp {
                        dest: Value(5),
                        op: IrCmpOp::Eq,
                        lhs: v(4),
                        rhs: constant(0, IrType::U32),
                        ty: IrType::U32,
                    },
                ],
                Terminator::Return(Some(v(5))),
            ),
            make(
                2,
                vec![],
                Terminator::Return(Some(constant(0, IrType::I32))),
            ),
        ];
        if variant == 1 {
            f.blocks.push(make(
                3,
                vec![],
                Terminator::Switch {
                    val: v(1),
                    cases: vec![],
                    default: BlockId(1),
                    ty: IrType::U32,
                },
            ));
        }
        if variant == 2 {
            f.blocks.push(make(
                3,
                vec![],
                Terminator::IndirectBranch {
                    target: v(1),
                    possible_targets: vec![BlockId(1)],
                },
            ));
        }
        f.next_value_id = 6;
        run(&mut f);
        assert_eq!(
            matches!(
                &f.blocks[1].instructions[1],
                Instruction::Copy {
                    src: Operand::Const(IrConst::I32(1)),
                    ..
                }
            ),
            variant == 0
        );
        for b in &f.blocks {
            assert_eq!(b.source_spans.len(), b.instructions.len());
        }
    }
}

#[test]
fn duplicate_ssa_and_exhausted_id_space_decline_without_mutation() {
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};
    let mut b = Builder::new(IrType::U32);
    let neg = b.bin(IrBinOp::Sub, b.c(0), v(0));
    let low = b.bin(IrBinOp::And, v(0), neg);
    b.cmp(IrCmpOp::Eq, low, b.c(1));
    let mut f = IrFunction::new("bad_ssa".into(), IrType::I32, vec![], false);
    f.blocks = vec![BasicBlock {
        label: BlockId(0),
        instructions: b.insts.clone(),
        source_spans: vec![],
        terminator: Terminator::Return(Some(v(b.next - 1))),
    }];
    f.next_value_id = u32::MAX;
    let old = f.blocks[0].instructions.clone();
    assert_eq!(run(&mut f), 0);
    assert_eq!(
        format!("{old:?}"),
        format!("{:?}", f.blocks[0].instructions)
    );
    f.next_value_id = b.next;
    f.blocks[0].instructions.push(b.insts[0].clone());
    let old = f.blocks[0].instructions.clone();
    assert_eq!(run(&mut f), 0);
    assert_eq!(
        format!("{old:?}"),
        format!("{:?}", f.blocks[0].instructions)
    );
}

#[test]
fn secondary_upstream_examples_even_heap_and_signed_zero_select() {
    let mut b = Builder::new(IrType::U64);
    let m = b.bin(IrBinOp::And, v(0), b.c(1));
    let even = b.cmp(IrCmpOp::Eq, m, b.c(0));
    let sub = b.bin(IrBinOp::Sub, v(0), b.c(2));
    let two = b.cmp(IrCmpOp::Eq, sub, b.c(0));
    b.and_bool(even, two);
    b.verify(unary_samples(IrType::U64));
    let mut b = Builder::new(IrType::I32);
    let sign = b.cmp(IrCmpOp::Slt, v(0), b.c(0));
    let zero = b.cmp(IrCmpOp::Eq, v(0), b.c(0));
    b.push(|dest| Instruction::Select {
        dest,
        cond: sign,
        true_val: constant(0, IrType::I32),
        false_val: zero,
        ty: IrType::I32,
    });
    b.verify(unary_samples(IrType::I32));
}

#[test]
fn already_ifconverted_nested_range_test_and_false_edge_facts() {
    let mut b = Builder::new(IrType::U32);
    let guard = b.cmp(IrCmpOp::Ult, v(0), b.c(13));
    let mask = b.bin(IrBinOp::And, v(0), b.c(16));
    let zero = b.cmp(IrCmpOp::Eq, mask, b.c(0));
    let inner = b.push(|dest| Instruction::Select {
        dest,
        cond: zero,
        true_val: constant(1, IrType::I32),
        false_val: constant(2, IrType::I32),
        ty: IrType::I32,
    });
    b.push(|dest| Instruction::Select {
        dest,
        cond: guard,
        true_val: inner,
        false_val: constant(0, IrType::I32),
        ty: IrType::I32,
    });
    b.verify(unary_samples(IrType::U32));
}

#[test]
fn canonical_doubled_shared_remainder_is_not_lost_before_division_lowering() {
    let mut b = Builder::new(IrType::U32);
    let r = b.bin(IrBinOp::URem, v(0), b.c(6));
    let r = b.bin(IrBinOp::Add, r, r);
    b.bin(IrBinOp::Xor, r, b.c(1));
    let q = b.bin(IrBinOp::UDiv, v(0), b.c(6));
    let q = b.bin(IrBinOp::Mul, q, b.c(12));
    b.bin(IrBinOp::Add, q, r);
    b.verify(unary_samples(IrType::U32));
}

#[test]
fn conditional_negate_boolean_mask_is_width_exact_and_commutative() {
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
        for commuted in [false, true] {
            let mut b = Builder::new(ty);
            let boolean = b.bin(IrBinOp::And, v(1), b.c(1));
            let neg = b.bin(IrBinOp::Sub, b.c(0), boolean);
            let xor = if commuted {
                b.bin(IrBinOp::Xor, neg, v(0))
            } else {
                b.bin(IrBinOp::Xor, v(0), neg)
            };
            if commuted {
                b.bin(IrBinOp::Add, boolean, xor);
            } else {
                b.bin(IrBinOp::Add, xor, boolean);
            }
            let w = ty.size() * 8;
            let mask = u64::MAX >> (64 - w);
            b.verify(
                (0..=255)
                    .flat_map(|x| [x, x | 1u64 << (w - 1), mask - x])
                    .flat_map(|x| [[x, 0, 0], [x, 1, 0], [x, 2, 0], [x, 3, 0]]),
            );
        }
    }
}

#[test]
fn conditional_negate_does_not_accept_truthiness_or_shared_masks() {
    for ty in [IrType::U8, IrType::U16, IrType::U32, IrType::U64] {
        let mut b = Builder::new(ty);
        let neg = b.bin(IrBinOp::Sub, b.c(0), v(1));
        let xor = b.bin(IrBinOp::Xor, v(0), neg);
        b.bin(IrBinOp::Add, xor, v(1));
        assert!(
            b.fold().is_none(),
            "arbitrary nonzero is not Boolean: {ty:?}"
        );
        let mut b = Builder::new(ty);
        let boolean = b.bin(IrBinOp::And, v(1), b.c(1));
        let neg = b.bin(IrBinOp::Sub, b.c(0), boolean);
        let xor = b.bin(IrBinOp::Xor, v(0), neg);
        b.bin(IrBinOp::Or, xor, v(2));
        b.bin(IrBinOp::Add, xor, boolean);
        assert!(
            b.fold().is_none(),
            "shared XOR must not be duplicated: {ty:?}"
        );
    }
}

#[test]
fn nonnegative_upper_range_guard_reduces_to_unsigned_compare() {
    for ty in [IrType::I8, IrType::I16, IrType::I32, IrType::I64] {
        for inverted in [false, true] {
            let mut b = Builder::new(ty);
            let bound = b.bin(IrBinOp::And, v(1), b.c(15));
            let lower = b.cmp(
                if inverted { IrCmpOp::Slt } else { IrCmpOp::Sge },
                v(0),
                b.c(0),
            );
            let upper = b.cmp(IrCmpOp::Slt, v(0), bound);
            let zero = Operand::Const(IrConst::I32(0));
            b.push(|dest| Instruction::Select {
                dest,
                cond: lower,
                true_val: if inverted { zero } else { upper },
                false_val: if inverted { upper } else { zero },
                ty: IrType::I32,
            });
            let w = ty.size() * 8;
            let mask = u64::MAX >> (64 - w);
            b.verify(
                (0..=255)
                    .flat_map(|x| [x, x | 1u64 << (w - 1), mask - x])
                    .flat_map(|x| (0..=31).map(move |y| [x, y, 0])),
            );
            assert!(matches!(
                b.fold().unwrap().first(),
                Some(Instruction::Cmp {
                    op: IrCmpOp::Ult,
                    ..
                })
            ));
        }
    }
}

#[test]
fn range_does_not_drop_sign_guard_for_unknown_or_negative_bounds() {
    for ty in [IrType::I8, IrType::I16, IrType::I32, IrType::I64] {
        for constant_bound in [false, true] {
            let mut b = Builder::new(ty);
            let lower = b.cmp(IrCmpOp::Sge, v(0), b.c(0));
            let upper = b.cmp(
                IrCmpOp::Slt,
                v(0),
                if constant_bound {
                    b.c(1u64 << (ty.size() * 8 - 1))
                } else {
                    v(1)
                },
            );
            b.push(|dest| Instruction::Select {
                dest,
                cond: lower,
                true_val: upper,
                false_val: Operand::Const(IrConst::I32(0)),
                ty: IrType::I32,
            });
            assert!(b.fold().is_none(), "{ty:?}");
        }
    }
}
