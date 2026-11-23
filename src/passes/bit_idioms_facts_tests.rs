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
    fn defs_and_uses(&self) -> (Vec<Option<Instruction>>, Vec<u32>) {
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
        (defs, uses)
    }
    fn context_view(&self, f: impl FnOnce(&Context<'_>, &Facts), facts: &Facts) {
        let (defs, uses) = self.defs_and_uses();
        let cx = Context {
            defs: &defs,
            uses: &uses,
            visits: std::cell::Cell::new(512),
            disable_logic: false,
            wrap_signed_overflow: true,
        };
        f(&cx, facts);
    }
    fn known(&self, op: Operand, ty: IrType, facts: &Facts) -> (u64, u64) {
        let mut out = (0, 0);
        self.context_view(|cx, facts| out = cx.known(op, ty, facts, 8), facts);
        out
    }
    fn with_const(&self, op: Operand, want: IrBinOp, ty: IrType) -> Option<(Operand, u64)> {
        let mut out = None;
        self.context_view(|cx, _| out = cx.with_const(op, want, ty), &Facts(vec![]));
        out
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
            source_inline_ids: Vec::new(),
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
        source_inline_ids: Vec::new(),
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
            if constant_bound {
                // Nothing is below the least value, so the upper guard is
                // decided and the pair is 0: a constant, not the unsigned
                // comparison the range rule would make of it.
                assert!(
                    matches!(
                        b.fold().as_deref(),
                        Some([Instruction::Copy { src, .. }])
                            if *src == Operand::Const(IrConst::I32(0))
                    ),
                    "{ty:?}"
                );
                b.verify(unary_samples(ty));
            } else {
                assert!(b.fold().is_none(), "{ty:?}");
            }
        }
    }
}

/// `cast_sign_range_fold` replaces a proven `U32 -> I32` Cast with an
/// untyped Copy. The producer keeps its U32 label while the consuming Cmp
/// keeps its I32 label. That mixed IR is bit-pattern-correct, but exact
/// `IrType` equality used to make the guarded-range proof blind.
#[test]
fn same_width_signedness_labels_do_not_hide_bit_facts() {
    assert!(same_width_scalar(IrType::I32, IrType::U32));
    assert!(same_width_scalar(IrType::U32, IrType::I32));
    assert!(!same_width_scalar(IrType::I32, IrType::I64));
    assert!(!same_width_scalar(IrType::I32, IrType::F32));
    assert!(!same_width_scalar(IrType::I128, IrType::U128));

    let mut b = Builder::new(IrType::U32);
    let masked = b.bin(IrBinOp::And, v(1), constant(15, IrType::U32));
    assert_eq!(
        b.with_const(masked, IrBinOp::And, IrType::I32),
        Some((v(1), 15)),
        "same-width signedness must not hide a masked producer"
    );
    assert!(b.with_const(masked, IrBinOp::And, IrType::I64).is_none());
    let facts = Facts(vec![]);
    let (zero, one) = b.known(masked, IrType::I32, &facts);
    assert_ne!(zero & (1u64 << 31), 0, "sign bit must be known zero");
    assert_eq!(one & !15, 0, "all high bits must be known zero");
    assert_eq!(
        b.known(masked, IrType::U32, &facts),
        (zero, one),
        "both signedness labels describe the same bit ring"
    );
    assert_eq!(
        b.known(masked, IrType::I64, &facts),
        (0, 0),
        "a different width must still refuse the fact"
    );
}

fn mixed_signedness_range(mask: u64, producer_ty: IrType) -> Builder {
    let mut b = Builder::new(IrType::U32);
    let masked = b.push(|dest| Instruction::BinOp {
        dest,
        op: IrBinOp::And,
        lhs: v(1),
        rhs: constant(mask, producer_ty),
        ty: producer_ty,
    });
    let lower = b.push(|dest| Instruction::Cmp {
        dest,
        op: IrCmpOp::Sge,
        lhs: v(0),
        rhs: constant(0, IrType::I32),
        ty: IrType::I32,
    });
    let upper = b.push(|dest| Instruction::Cmp {
        dest,
        op: IrCmpOp::Slt,
        lhs: v(0),
        rhs: masked,
        ty: IrType::I32,
    });
    let widened = b.push(|dest| Instruction::Cast {
        dest,
        src: upper,
        from_ty: IrType::U8,
        to_ty: IrType::I64,
    });
    b.push(|dest| Instruction::Select {
        dest,
        cond: lower,
        true_val: widened,
        false_val: constant(0, IrType::I64),
        ty: IrType::I64,
    });
    b
}

#[test]
fn guarded_range_folds_when_signedness_cast_was_range_elided() {
    let b = mixed_signedness_range(15, IrType::U32);
    let plan = b
        .fold()
        .expect("same-width U32 mask must still certify a nonnegative I32 bound");
    assert_eq!(plan.len(), 2, "Boolean result must be widened once");
    assert!(matches!(
        &plan[0],
        Instruction::Cmp {
            op: IrCmpOp::Ult,
            lhs,
            rhs,
            ty: IrType::I32,
            ..
        } if *lhs == v(0) && *rhs == v(3)
    ));
    let xs = [
        i32::MIN,
        i32::MIN + 1,
        -256,
        -16,
        -15,
        -2,
        -1,
        0,
        1,
        2,
        14,
        15,
        16,
        255,
        i32::MAX - 1,
        i32::MAX,
    ];
    let ys = [
        0u32,
        1,
        2,
        14,
        15,
        16,
        255,
        0x4000_0000,
        0x7fff_fff0,
        0x8000_0000,
        0xffff_fff0,
        u32::MAX,
    ];
    b.verify(
        xs.iter()
            .flat_map(|&x| ys.iter().map(move |&y| [x as u32 as u64, u64::from(y), 0])),
    );
}

#[test]
fn guarded_range_still_refuses_negative_or_wrong_width_bounds() {
    for (mask, ty) in [
        (0x8000_000f, IrType::U32), // sign bit is not known clear
        (15, IrType::U64),          // same-width normalization is not widening
    ] {
        let b = mixed_signedness_range(mask, ty);
        assert!(
            b.fold().is_none(),
            "unsafe bound must keep both signed guards: mask={mask:#x} ty={ty:?}"
        );
    }
}

/// `known` through an integer conversion. For every conversion among the 8-
/// and 16-bit types and every fact over six bits of the source (its low four
/// and its top two: each known zero, known one or unknown), every source the
/// fact allows converts to a value the answer allows. And the answer is not
/// merely cautious: a source known altogether converts to a value that is.
#[test]
fn known_bits_cross_integer_conversions() {
    let convert = |x: u64, from: IrType, to: IrType| -> u64 {
        let wide = match from {
            IrType::I8 => x as u8 as i8 as i64,
            IrType::U8 => x as u8 as i64,
            IrType::I16 => x as u16 as i16 as i64,
            IrType::U16 => x as u16 as i64,
            IrType::I32 => x as u32 as i32 as i64,
            IrType::U32 => x as u32 as i64,
            _ => x as i64,
        };
        wide as u64 & (u64::MAX >> (64 - to.size() * 8))
    };
    let cast_of_v0 = |from: IrType, to: IrType| {
        let mut b = Builder::new(to);
        let cast = b.push(|dest| Instruction::Cast {
            dest,
            src: v(0),
            from_ty: from,
            to_ty: to,
        });
        (b, cast)
    };
    let narrow = [IrType::I8, IrType::U8, IrType::I16, IrType::U16];
    for from in narrow {
        for to in narrow {
            let (b, cast) = cast_of_v0(from, to);
            let from_w = from.size() * 8;
            let from_mask = u64::MAX >> (64 - from_w);
            let positions = [0, 1, 2, 3, from_w - 2, from_w - 1];
            for code in 0..3u32.pow(6) {
                let (mut zero, mut one, mut rest) = (0u64, 0u64, code);
                for bit in positions {
                    match rest % 3 {
                        1 => zero |= 1 << bit,
                        2 => one |= 1 << bit,
                        _ => {}
                    }
                    rest /= 3;
                }
                let (z, o) = b.known(cast, to, &Facts(vec![(v(0), from_w, zero, one)]));
                assert_eq!(z & o, 0, "{from:?}->{to:?} zero {zero:#x} one {one:#x}");
                for x in (0..=from_mask).filter(|x| x & zero == 0 && x & one == one) {
                    let y = convert(x, from, to);
                    assert!(
                        y & z == 0 && y & o == o,
                        "{from:?}->{to:?} zero {zero:#x} one {one:#x}: {x:#x} converts to \
                         {y:#x}, answered zero {z:#x} one {o:#x}"
                    );
                }
            }
        }
    }
    let every = [
        IrType::I8,
        IrType::U8,
        IrType::I16,
        IrType::U16,
        IrType::I32,
        IrType::U32,
        IrType::I64,
        IrType::U64,
    ];
    for from in every {
        for to in every {
            let (b, cast) = cast_of_v0(from, to);
            let from_w = from.size() * 8;
            let from_mask = u64::MAX >> (64 - from_w);
            let to_mask = u64::MAX >> (64 - to.size() * 8);
            let top = 1u64 << (from_w - 1);
            for x in [0, 1, 0x5a, top - 1, top, top | 0x25, from_mask] {
                let facts = Facts(vec![(v(0), from_w, !x & from_mask, x)]);
                let y = convert(x, from, to);
                assert_eq!(
                    b.known(cast, to, &facts),
                    (!y & to_mask, y),
                    "{from:?}->{to:?} of {x:#x}"
                );
            }
            // Only the top bit known: a signed widening knows every bit it
            // adds, an unsigned one knows them to be zero, a narrowing that
            // cuts the bit off knows nothing.
            for (zero, one) in [(top, 0), (0, top)] {
                let (z, o) = b.known(cast, to, &Facts(vec![(v(0), from_w, zero, one)]));
                let added = to_mask & !from_mask;
                let want = if to.size() < from.size() {
                    (0, 0)
                } else if !from.is_signed() {
                    (zero | added, one)
                } else if one != 0 {
                    (0, one | added)
                } else {
                    (zero | added, 0)
                };
                assert_eq!(
                    (z, o),
                    want,
                    "{from:?}->{to:?} top bit ({zero:#x}, {one:#x})"
                );
            }
        }
    }
    // The value is looked at in the conversion's own width, or not at all;
    // and a conversion from a float moves no bits.
    let (b, cast) = cast_of_v0(IrType::U8, IrType::U32);
    let facts = Facts(vec![(v(0), 8, 0xfe, 1)]);
    assert_eq!(b.known(cast, IrType::U32, &facts), (0xffff_fffe, 1));
    assert_eq!(b.known(cast, IrType::I32, &facts), (0xffff_fffe, 1));
    assert_eq!(b.known(cast, IrType::U64, &facts), (0, 0));
    assert_eq!(b.known(cast, IrType::U16, &facts), (0, 0));
    let (b, cast) = cast_of_v0(IrType::F32, IrType::U32);
    assert_eq!(
        b.known(cast, IrType::U32, &Facts(vec![(v(0), 32, 0, 1)])),
        (0, 0)
    );
}

/// What a guard `shift(x, n) == c` says about `x`, for every shift, count
/// and constant of a byte: every `x` that passes the guard satisfies the
/// facts; a guard something passes fixes exactly the bits the shift kept;
/// a guard nothing passes says nothing about `x`.
#[test]
fn equality_of_a_shifted_operand_fixes_the_bits_the_shift_kept() {
    let about_v0 = |b: &Builder, guard: Operand, truth: bool| {
        let mut out = Facts(vec![]);
        b.context_view(|cx, _| cx.facts(guard, truth, &mut out, 8), &Facts(vec![]));
        out.0
            .iter()
            .filter(|fact| fact.0 == v(0))
            .fold((0, 0), |(z, o), fact| {
                assert_eq!(fact.1, 8);
                (z | fact.2, o | fact.3)
            })
    };
    for ty in [IrType::U8, IrType::I8] {
        for shift in [IrBinOp::LShr, IrBinOp::AShr, IrBinOp::Shl] {
            for n in 0..8u32 {
                let shifted = |x: u64| match shift {
                    IrBinOp::LShr => x >> n,
                    IrBinOp::AShr => ((x as u8 as i8) >> n) as u8 as u64,
                    _ => (x << n) & 0xff,
                };
                let kept = if shift == IrBinOp::Shl {
                    0xffu64 >> n
                } else {
                    (0xffu64 << n) & 0xff
                };
                for c in 0..256u64 {
                    let mut b = Builder::new(ty);
                    let value = b.bin(shift, v(0), b.c(u64::from(n)));
                    let guard = b.cmp(IrCmpOp::Eq, value, b.c(c));
                    let (zero, one) = about_v0(&b, guard, true);
                    let passing: Vec<u64> = (0..256).filter(|&x| shifted(x) == c).collect();
                    for &x in &passing {
                        assert!(
                            x & zero == 0 && x & one == one,
                            "{ty:?} {shift:?} by {n} == {c:#x}: {x:#x} passes, facts zero \
                             {zero:#x} one {one:#x}"
                        );
                    }
                    assert_eq!(
                        zero | one,
                        if passing.is_empty() { 0 } else { kept },
                        "{ty:?} {shift:?} by {n} == {c:#x}"
                    );
                    // The other spelling of the same edge, and the edges that
                    // say only that the value differs.
                    let differs = b.cmp(IrCmpOp::Ne, value, b.c(c));
                    assert_eq!(about_v0(&b, differs, false), (zero, one));
                    assert_eq!(about_v0(&b, differs, true), (0, 0));
                    assert_eq!(about_v0(&b, guard, false), (0, 0));
                }
            }
        }
    }
    // A count that is no constant, or one as wide as the operand, fixes
    // nothing.
    let mut b = Builder::new(IrType::U8);
    let value = b.bin(IrBinOp::LShr, v(0), v(1));
    let guard = b.cmp(IrCmpOp::Eq, value, b.c(1));
    assert_eq!(about_v0(&b, guard, true), (0, 0));
    let mut b = Builder::new(IrType::U8);
    let value = b.bin(IrBinOp::LShr, v(0), b.c(8));
    let guard = b.cmp(IrCmpOp::Eq, value, b.c(0));
    assert_eq!(about_v0(&b, guard, true), (0, 0));
}

/// What the known bits of two operands say about each of the ten
/// comparisons, against every pair of values the bits allow: each of four
/// bits of the left operand and three of the right is known zero, known one
/// or unknown, the rest are fixed. The answer is exact both ways: `truth`
/// names an outcome when every pair has it, and only then.
#[test]
fn known_bits_decide_a_comparison_exactly() {
    let ops = [
        IrCmpOp::Eq,
        IrCmpOp::Ne,
        IrCmpOp::Ult,
        IrCmpOp::Ule,
        IrCmpOp::Ugt,
        IrCmpOp::Uge,
        IrCmpOp::Slt,
        IrCmpOp::Sle,
        IrCmpOp::Sgt,
        IrCmpOp::Sge,
    ];
    // (zero, one, the values they allow) for every state of `positions`.
    let shapes = |positions: &[u32], fixed: u64| -> Vec<(u64, u64, Vec<u64>)> {
        let free = positions.iter().fold(0u64, |m, bit| m | 1 << bit);
        (0..3u32.pow(positions.len() as u32))
            .map(|mut code| {
                let (mut zero, mut one) = (!fixed & !free & 0xff, fixed & !free);
                for bit in positions {
                    match code % 3 {
                        1 => zero |= 1 << bit,
                        2 => one |= 1 << bit,
                        _ => {}
                    }
                    code /= 3;
                }
                let allowed = (0..256).filter(|x| x & zero == 0 && x & one == one);
                (zero, one, allowed.collect())
            })
            .collect()
    };
    let mut checked = 0;
    for ty in [IrType::U8, IrType::I8] {
        let mut b = Builder::new(ty);
        let compares: Vec<Operand> = ops.iter().map(|&op| b.cmp(op, v(0), v(1))).collect();
        let left = shapes(&[0, 1, 6, 7], 0b0001_0100);
        // The right operand's middle bits equal the left's, exceed them and
        // fall short of them, so that the low bits, the middle and the top
        // each get to decide.
        for middle in [0b0001_0100, 0b0010_0100, 0b0000_1100] {
            let right = shapes(&[0, 6, 7], middle);
            for (az, ao, xs) in &left {
                for (bz, bo, ys) in &right {
                    let facts = Facts(vec![(v(0), 8, *az, *ao), (v(1), 8, *bz, *bo)]);
                    for (index, &compare) in compares.iter().enumerate() {
                        let (mut holds, mut fails) = (false, false);
                        for &x in xs {
                            for &y in ys {
                                let mut values = vec![0; b.next as usize + 1];
                                values[0] = x;
                                values[1] = y;
                                eval(&b.insts[index], &mut values);
                                if values[3 + index] != 0 {
                                    holds = true;
                                } else {
                                    fails = true;
                                }
                            }
                        }
                        let want = match (holds, fails) {
                            (true, false) => Some(true),
                            (false, true) => Some(false),
                            _ => None,
                        };
                        let mut got = None;
                        b.context_view(|cx, facts| got = cx.truth(compare, facts, 8), &facts);
                        assert_eq!(
                            got, want,
                            "{ty:?} {:?}: left ({az:#x}, {ao:#x}) right ({bz:#x}, {bo:#x})",
                            ops[index]
                        );
                        checked += usize::from(want.is_some());
                    }
                }
            }
        }
    }
    // 37,830 of the 65,610 rows of each type are decided.
    assert_eq!(checked, 75_660);
}

/// A comparison its operands' own bits decide folds with no guard in front
/// of it, at every width; one a single value away from decided stays.
#[test]
fn a_comparison_the_operands_bits_decide_folds_without_a_guard() {
    for ty in [IrType::U8, IrType::U16, IrType::U32, IrType::U64] {
        let w = ty.size() as u64 * 8;
        let top = 1u64 << (w - 1);
        let mask = u64::MAX >> (64 - w);
        type Shape = (IrBinOp, u64, IrCmpOp, u64);
        let decided: [Shape; 12] = [
            (IrBinOp::Or, 1, IrCmpOp::Eq, 0),
            (IrBinOp::Or, 1, IrCmpOp::Ne, 0),
            (IrBinOp::And, 6, IrCmpOp::Eq, 1),
            (IrBinOp::And, 7, IrCmpOp::Ult, 8),
            (IrBinOp::And, 7, IrCmpOp::Ugt, 7),
            (IrBinOp::Or, 8, IrCmpOp::Uge, 8),
            (IrBinOp::Or, 16, IrCmpOp::Ule, 15),
            (IrBinOp::Or, top, IrCmpOp::Slt, 0),
            (IrBinOp::And, top - 1, IrCmpOp::Sge, 0),
            (IrBinOp::And, top - 1, IrCmpOp::Sgt, mask),
            (IrBinOp::LShr, 4, IrCmpOp::Ult, top >> 3),
            (IrBinOp::Shl, 2, IrCmpOp::Eq, 5),
        ];
        let is_a_constant = |b: &Builder| {
            matches!(
                b.fold().as_deref(),
                Some([Instruction::Copy {
                    src: Operand::Const(_),
                    ..
                }])
            )
        };
        for (shape, operand, op, bound) in decided {
            let mut b = Builder::new(ty);
            let value = b.bin(shape, v(0), b.c(operand));
            b.cmp(op, value, b.c(bound));
            assert!(
                is_a_constant(&b),
                "{ty:?} {shape:?} {operand:#x} {op:?} {bound:#x}"
            );
            b.verify(unary_samples(ty));
            // The same comparison written the other way round.
            let swapped = match op {
                IrCmpOp::Ult => IrCmpOp::Ugt,
                IrCmpOp::Ugt => IrCmpOp::Ult,
                IrCmpOp::Ule => IrCmpOp::Uge,
                IrCmpOp::Uge => IrCmpOp::Ule,
                IrCmpOp::Slt => IrCmpOp::Sgt,
                IrCmpOp::Sgt => IrCmpOp::Slt,
                IrCmpOp::Sle => IrCmpOp::Sge,
                IrCmpOp::Sge => IrCmpOp::Sle,
                same => same,
            };
            let mut b = Builder::new(ty);
            let value = b.bin(shape, v(0), b.c(operand));
            b.cmp(swapped, b.c(bound), value);
            assert!(
                is_a_constant(&b),
                "{ty:?} {bound:#x} {swapped:?} {shape:?} {operand:#x}"
            );
            b.verify(unary_samples(ty));
        }
        let open: [Shape; 7] = [
            (IrBinOp::Or, 1, IrCmpOp::Eq, 1),
            (IrBinOp::And, 6, IrCmpOp::Eq, 2),
            (IrBinOp::And, 7, IrCmpOp::Ult, 7),
            (IrBinOp::Or, 8, IrCmpOp::Ugt, 8),
            (IrBinOp::Or, top >> 1, IrCmpOp::Slt, 0),
            (IrBinOp::And, top - 1, IrCmpOp::Sgt, 0),
            (IrBinOp::LShr, 4, IrCmpOp::Ult, (top >> 3) - 1),
        ];
        for (shape, operand, op, bound) in open {
            let mut b = Builder::new(ty);
            let value = b.bin(shape, v(0), b.c(operand));
            b.cmp(op, value, b.c(bound));
            assert!(
                !is_a_constant(&b),
                "{ty:?} {shape:?} {operand:#x} {op:?} {bound:#x}"
            );
        }
    }
    // Through the promotion C puts in front of a narrow operand:
    // `(x | 0x80) < 0x80` for an `unsigned char x` compares two ints.
    let mut b = Builder::new(IrType::I32);
    let wide = b.push(|dest| Instruction::Cast {
        dest,
        src: v(0),
        from_ty: IrType::U8,
        to_ty: IrType::I32,
    });
    let value = b.bin(IrBinOp::Or, wide, b.c(0x80));
    b.cmp(IrCmpOp::Slt, value, b.c(0x80));
    assert!(matches!(
        b.fold().as_deref(),
        Some([Instruction::Copy { src, .. }]) if *src == constant(0, IrType::I32)
    ));
    b.verify((0..=255).map(|x| [x, 0, 0]));
}

/// The truth of a bitwise `And` and `Or` of two words, against every pair
/// of values their known bits allow (three bits of each known zero, known
/// one or unknown). Two words that are each non-zero need not share a bit.
#[test]
fn the_truth_of_a_bitwise_and_is_a_shared_bit() {
    let positions = [0u32, 1, 7];
    let states: Vec<(u64, u64)> = (0..27)
        .map(|mut code| {
            let (mut zero, mut one) = (0b0111_1100, 0);
            for bit in positions {
                match code % 3 {
                    1 => zero |= 1 << bit,
                    2 => one |= 1 << bit,
                    _ => {}
                }
                code /= 3;
            }
            (zero, one)
        })
        .collect();
    let allowed =
        |(zero, one): (u64, u64)| (0..256u64).filter(move |x| x & zero == 0 && x & one == one);
    let mut b = Builder::new(IrType::U8);
    let and = b.bin(IrBinOp::And, v(0), v(1));
    let or = b.bin(IrBinOp::Or, v(0), v(1));
    let mut decided = [0, 0];
    for &left in &states {
        for &right in &states {
            let facts = Facts(vec![(v(0), 8, left.0, left.1), (v(1), 8, right.0, right.1)]);
            for (index, (word, combine)) in [
                (and, (|x, y| x & y) as fn(u64, u64) -> u64),
                (or, |x, y| x | y),
            ]
            .into_iter()
            .enumerate()
            {
                let (mut holds, mut fails) = (false, false);
                for x in allowed(left) {
                    for y in allowed(right) {
                        if combine(x, y) != 0 {
                            holds = true;
                        } else {
                            fails = true;
                        }
                    }
                }
                let want = match (holds, fails) {
                    (true, false) => Some(true),
                    (false, true) => Some(false),
                    _ => None,
                };
                let mut got = None;
                b.context_view(|cx, facts| got = cx.truth(word, facts, 8), &facts);
                assert_eq!(got, want, "word {index}: left {left:x?} right {right:x?}");
                decided[index] += usize::from(want.is_some());
            }
        }
    }
    // Of the 729 pairs: an `And` is decided by a shared one or by a zero in
    // every position, an `Or` by a one on either side or zeros throughout.
    assert_eq!(decided, [342, 666]);

    // The shape that was miscompiled: `((x | 1) & (y | 2)) ? 4 : 8` is 8
    // for `x == y == 0`, so its comparison with 4 is no constant ...
    let nonzero_words = |right_bits: u64| {
        let mut b = Builder::new(IrType::U32);
        let left = b.bin(IrBinOp::Or, v(0), b.c(1));
        let right = b.bin(IrBinOp::Or, v(1), b.c(right_bits));
        let both = b.bin(IrBinOp::And, left, right);
        let (four, eight) = (b.c(4), b.c(8));
        let chosen = b.push(|dest| Instruction::Select {
            dest,
            cond: both,
            true_val: four,
            false_val: eight,
            ty: IrType::U32,
        });
        b.cmp(IrCmpOp::Eq, chosen, four);
        b
    };
    let pairs = || (0..64u64).flat_map(|x| (0..64u64).map(move |y| [x, y, 0]));
    let b = nonzero_words(2);
    let folded = b.fold();
    assert!(!matches!(
        folded.as_deref(),
        Some([Instruction::Copy {
            src: Operand::Const(_),
            ..
        }])
    ));
    if folded.is_some() {
        b.verify(pairs());
    }
    // ... and with a bit in common it is one.
    let b = nonzero_words(3);
    assert!(matches!(
        b.fold().as_deref(),
        Some([Instruction::Copy { src, .. }]) if *src == constant(1, IrType::I32)
    ));
    b.verify(pairs());

    // Two comparisons that both hold are both 1, and their `And` is.
    let mut b = Builder::new(IrType::U32);
    let left = b.bin(IrBinOp::Or, v(0), b.c(1));
    let right = b.bin(IrBinOp::Or, v(1), b.c(2));
    let left = b.cmp(IrCmpOp::Ne, left, b.c(0));
    let right = b.cmp(IrCmpOp::Ne, right, b.c(0));
    let both = b.and_bool(left, right);
    let mut truth = None;
    b.context_view(|cx, facts| truth = cx.truth(both, facts, 8), &Facts(vec![]));
    assert_eq!(truth, Some(true));
}

/// A select whose condition is decided is the arm it chooses, whether the
/// condition is a comparison or a word; one that is not decided stays.
#[test]
fn a_select_on_a_decided_condition_is_its_arm() {
    for ty in [IrType::U8, IrType::U16, IrType::U32, IrType::U64] {
        let select = |b: &mut Builder, cond: Operand| {
            b.push(|dest| Instruction::Select {
                dest,
                cond,
                true_val: v(1),
                false_val: v(2),
                ty,
            })
        };
        let arm = |b: &Builder| match b.fold().as_deref() {
            Some([Instruction::Copy { src, .. }]) => Some(*src),
            _ => None,
        };
        let samples = || {
            unary_samples(ty)
                .step_by(97)
                .map(|[x, ..]| [x, x.wrapping_mul(3) ^ 5, x.wrapping_add(11)])
        };
        // A word with a bit set chooses the first arm.
        let mut b = Builder::new(ty);
        let word = b.bin(IrBinOp::Or, v(0), b.c(4));
        select(&mut b, word);
        assert_eq!(arm(&b), Some(v(1)), "{ty:?}");
        b.verify(samples());
        // A comparison that cannot hold chooses the second.
        let mut b = Builder::new(ty);
        let masked = b.bin(IrBinOp::And, v(0), b.c(6));
        let never = b.cmp(IrCmpOp::Eq, masked, b.c(1));
        select(&mut b, never);
        assert_eq!(arm(&b), Some(v(2)), "{ty:?}");
        b.verify(samples());
        // A word that is zero altogether chooses the second.
        let mut b = Builder::new(ty);
        let low = b.bin(IrBinOp::And, v(0), b.c(3));
        let none = b.bin(IrBinOp::And, low, b.c(12));
        select(&mut b, none);
        assert_eq!(arm(&b), Some(v(2)), "{ty:?}");
        b.verify(samples());
        // An open word and an open comparison choose nothing.
        let mut b = Builder::new(ty);
        let word = b.bin(IrBinOp::And, v(0), b.c(4));
        select(&mut b, word);
        assert_eq!(arm(&b), None, "{ty:?}");
        let mut b = Builder::new(ty);
        let masked = b.bin(IrBinOp::And, v(0), b.c(6));
        let open = b.cmp(IrCmpOp::Eq, masked, b.c(2));
        select(&mut b, open);
        assert_eq!(arm(&b), None, "{ty:?}");
    }
}
