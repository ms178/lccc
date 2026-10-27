//! Byte-sum reduction (`ByteSumKind` × `ByteSumWidth`): executed differential
//! tests.
//!
//! Every positive case builds the scalar loop C produces for the shape,
//! runs the real analyzer and transform, and then EXECUTES both the original
//! and the rewritten IR -- packed body, horizontal exit, signed bias
//! correction and scalar remainder -- on a small two-world interpreter, over
//! trip counts that straddle every packed/remainder boundary and data that
//! hits both byte extremes.  Shape assertions alone cannot catch a wrong
//! bias, a dropped tail or a sign-extended remainder; executing the IR does.
//! The interpreter also asserts that no load reads outside the stream's
//! `n` live bytes, so a packed over-read fails the test outright.

use super::*;
use crate::common::source::Span;

const BASE_A: u64 = 0x1000_0000;
const BASE_B: u64 = 0x2000_0000;

/// The added-value shapes, each spelled the way C lowers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// `s += p[i]` (one direct widening cast, as lccc emits it).
    Plain,
    /// `s += p[i] & 0x0f` (promotion to int, ring op, widening).
    MaskLow,
    /// `s += p[i] ^ q[i]` (two streams).
    XorStreams,
    /// `s += p[i] > 100 ? p[i] : 0` (select over a lane mask).
    SelectGt,
    /// `s += (unsigned char)(p[i] + 3)` (truncation to the lane).
    TruncPlus3,
    /// `s += sp[i]` with `signed char *sp` (sign-extending widening).
    SignedPlain,
    /// `s += (signed char)p[i]` (truncation to a signed lane).
    SignedTrunc,
    /// `s += (signed char)p[i]` spelled as lccc lowers it: ONE same-size
    /// `Cast U8 -> I8` reinterpretation.  Its interval is [-128, 127], not
    /// the source's [0, 255] (the map-parser defect this pins).
    SignedReinterpret,
    /// `s += (p[i] == 7)` (the 0/1 predicate count).
    PredEq,
    /// `s += (p[i] != 7)`: counted as `consumed - count(==)`.
    PredNe,
    /// `m = (p[i] == 7); s += (m != 0)`: `!= 0` over a lane mask is the
    /// mask itself (what count normalisation leaves for `if (p[i] == 7)`).
    PredNeOfEq,
    /// `s += (p[i] == 7) | (q[i] == 9)`: an OR of lane masks is a mask.
    PredOrStreams,
    /// `s += 0 - (p[i] == 7)`: a compare used as a VALUE.  Its packed lane
    /// is 0xFF where the scalar setcc is 1; must be refused.
    ZeroMinusCmp,
    /// `s += p[i] - q[i]`: interval [-255, 255] fits neither byte domain.
    DiffStreams,
    /// `s += sp[i] + 1`: interval [-127, 128] fits neither byte domain.
    SignedPlusOne,
}

impl Shape {
    fn expected(self) -> Option<ByteSumKind> {
        match self {
            Shape::Plain
            | Shape::MaskLow
            | Shape::XorStreams
            | Shape::SelectGt
            | Shape::TruncPlus3 => Some(ByteSumKind::Unsigned),
            Shape::SignedPlain | Shape::SignedTrunc | Shape::SignedReinterpret => {
                Some(ByteSumKind::Signed)
            }
            Shape::PredEq | Shape::PredNe | Shape::PredNeOfEq | Shape::PredOrStreams => {
                Some(ByteSumKind::Predicate)
            }
            Shape::ZeroMinusCmp | Shape::DiffStreams | Shape::SignedPlusOne => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Limit {
    Const(i64),
    /// A loop-invariant value defined in the preheader (`Copy` of the
    /// constant), so the transform must divide it at run time.
    Dynamic(i64),
}

impl Limit {
    fn n(self) -> i64 {
        match self {
            Limit::Const(n) | Limit::Dynamic(n) => n,
        }
    }
}

fn konst(v: i64, ty: IrType) -> Operand {
    Operand::Const(IrConst::from_i64(v, ty))
}

/// Build `for (iv = 0; iv < limit; iv++) acc += shape(p[iv], q[iv]);`.
fn fixture(shape: Shape, acc_ty: IrType, iv_ty: IrType, limit: Limit, seed: i64) -> IrFunction {
    let v = |id: u32| Operand::Value(Value(id));
    let next = std::cell::Cell::new(200u32);
    let fresh = || {
        next.set(next.get() + 1);
        Value(next.get())
    };
    let (iv, acc, cmp, iv_next, acc_next) = (Value(1), Value(2), Value(3), Value(4), Value(5));
    let (base_a, base_b, limit_val) = (Value(100), Value(101), Value(102));

    let mut pre = vec![
        Instruction::GlobalAddr {
            dest: base_a,
            name: "a".into(),
        },
        Instruction::GlobalAddr {
            dest: base_b,
            name: "b".into(),
        },
    ];
    let limit_op = match limit {
        Limit::Const(n) => konst(n, iv_ty),
        Limit::Dynamic(n) => {
            pre.push(Instruction::Copy {
                dest: limit_val,
                src: konst(n, iv_ty),
            });
            v(limit_val.0)
        }
    };

    let mut body = Vec::new();
    let off = if matches!(iv_ty, IrType::I64 | IrType::U64) {
        iv
    } else {
        let o = fresh();
        body.push(Instruction::Cast {
            dest: o,
            src: v(iv.0),
            from_ty: iv_ty,
            to_ty: IrType::I64,
        });
        o
    };
    let load = |body: &mut Vec<Instruction>, base: Value, ty: IrType| {
        let g = fresh();
        let l = fresh();
        body.push(Instruction::GetElementPtr {
            dest: g,
            base,
            offset: Operand::Value(off),
            ty: IrType::Ptr,
        });
        body.push(Instruction::Load {
            dest: l,
            ptr: g,
            ty,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
        l
    };
    let stream_ty = match shape {
        Shape::SignedPlain | Shape::SignedPlusOne => IrType::I8,
        _ => IrType::U8,
    };
    let la = load(&mut body, base_a, stream_ty);
    let lb = if matches!(
        shape,
        Shape::XorStreams | Shape::DiffStreams | Shape::PredOrStreams
    ) {
        Some(load(&mut body, base_b, IrType::U8))
    } else {
        None
    };
    let cast = |body: &mut Vec<Instruction>, src: Value, from: IrType, to: IrType| {
        let d = fresh();
        body.push(Instruction::Cast {
            dest: d,
            src: Operand::Value(src),
            from_ty: from,
            to_ty: to,
        });
        d
    };
    let bin = |body: &mut Vec<Instruction>, op: IrBinOp, l: Operand, r: Operand| {
        let d = fresh();
        body.push(Instruction::BinOp {
            dest: d,
            op,
            lhs: l,
            rhs: r,
            ty: IrType::I32,
        });
        d
    };
    let i32c = |n: i64| konst(n, IrType::I32);
    // Each arm leaves the I32 (or narrower) value and its type to widen.
    let (val, val_ty) = match shape {
        Shape::Plain => (la, IrType::U8),
        Shape::SignedPlain => (la, IrType::I8),
        Shape::MaskLow => {
            let c = cast(&mut body, la, IrType::U8, IrType::I32);
            (
                bin(&mut body, IrBinOp::And, v(c.0), i32c(0x0f)),
                IrType::I32,
            )
        }
        Shape::XorStreams | Shape::DiffStreams => {
            let ca = cast(&mut body, la, IrType::U8, IrType::I32);
            let cb = cast(&mut body, lb.unwrap(), IrType::U8, IrType::I32);
            let op = if shape == Shape::XorStreams {
                IrBinOp::Xor
            } else {
                IrBinOp::Sub
            };
            (bin(&mut body, op, v(ca.0), v(cb.0)), IrType::I32)
        }
        Shape::SelectGt => {
            let c = cast(&mut body, la, IrType::U8, IrType::I32);
            let m = fresh();
            body.push(Instruction::Cmp {
                dest: m,
                op: IrCmpOp::Sgt,
                lhs: v(c.0),
                rhs: i32c(100),
                ty: IrType::I32,
            });
            let s = fresh();
            body.push(Instruction::Select {
                dest: s,
                cond: v(m.0),
                true_val: v(c.0),
                false_val: i32c(0),
                ty: IrType::I32,
            });
            (s, IrType::I32)
        }
        Shape::TruncPlus3 => {
            let c = cast(&mut body, la, IrType::U8, IrType::I32);
            let a = bin(&mut body, IrBinOp::Add, v(c.0), i32c(3));
            (cast(&mut body, a, IrType::I32, IrType::U8), IrType::U8)
        }
        Shape::SignedTrunc => {
            let c = cast(&mut body, la, IrType::U8, IrType::I32);
            (cast(&mut body, c, IrType::I32, IrType::I8), IrType::I8)
        }
        Shape::SignedReinterpret => (cast(&mut body, la, IrType::U8, IrType::I8), IrType::I8),
        Shape::SignedPlusOne => {
            let c = cast(&mut body, la, IrType::I8, IrType::I32);
            (bin(&mut body, IrBinOp::Add, v(c.0), i32c(1)), IrType::I32)
        }
        Shape::PredNe | Shape::PredNeOfEq | Shape::PredOrStreams => {
            let cmp = |body: &mut Vec<Instruction>, op: IrCmpOp, l: Operand, r: Operand| {
                let d = fresh();
                body.push(Instruction::Cmp {
                    dest: d,
                    op,
                    lhs: l,
                    rhs: r,
                    ty: IrType::I32,
                });
                d
            };
            let c = cast(&mut body, la, IrType::U8, IrType::I32);
            let m = match shape {
                Shape::PredNe => cmp(&mut body, IrCmpOp::Ne, v(c.0), i32c(7)),
                Shape::PredNeOfEq => {
                    let e = cmp(&mut body, IrCmpOp::Eq, v(c.0), i32c(7));
                    cmp(&mut body, IrCmpOp::Ne, v(e.0), i32c(0))
                }
                _ => {
                    let e = cmp(&mut body, IrCmpOp::Eq, v(c.0), i32c(7));
                    let cb = cast(&mut body, lb.unwrap(), IrType::U8, IrType::I32);
                    let f = cmp(&mut body, IrCmpOp::Eq, v(cb.0), i32c(9));
                    bin(&mut body, IrBinOp::Or, v(e.0), v(f.0))
                }
            };
            (m, IrType::I32)
        }
        Shape::PredEq | Shape::ZeroMinusCmp => {
            let c = cast(&mut body, la, IrType::U8, IrType::I32);
            let m = fresh();
            body.push(Instruction::Cmp {
                dest: m,
                op: IrCmpOp::Eq,
                lhs: v(c.0),
                rhs: i32c(7),
                ty: IrType::I32,
            });
            if shape == Shape::PredEq {
                (m, IrType::I32)
            } else {
                (bin(&mut body, IrBinOp::Sub, i32c(0), v(m.0)), IrType::I32)
            }
        }
    };
    let added = if val_ty == acc_ty {
        val
    } else {
        cast(&mut body, val, val_ty, acc_ty)
    };
    body.push(Instruction::BinOp {
        dest: acc_next,
        op: IrBinOp::Add,
        lhs: v(acc.0),
        rhs: v(added.0),
        ty: acc_ty,
    });
    body.push(Instruction::BinOp {
        dest: iv_next,
        op: IrBinOp::Add,
        lhs: v(iv.0),
        rhs: konst(1, iv_ty),
        ty: iv_ty,
    });

    let signed_iv = matches!(iv_ty, IrType::I32 | IrType::I64);
    let header = vec![
        Instruction::Phi {
            dest: iv,
            ty: iv_ty,
            incoming: vec![(konst(0, iv_ty), BlockId(0)), (v(iv_next.0), BlockId(2))],
        },
        Instruction::Phi {
            dest: acc,
            ty: acc_ty,
            incoming: vec![
                (konst(seed, acc_ty), BlockId(0)),
                (v(acc_next.0), BlockId(2)),
            ],
        },
        Instruction::Cmp {
            dest: cmp,
            op: if signed_iv {
                IrCmpOp::Slt
            } else {
                IrCmpOp::Ult
            },
            lhs: v(iv.0),
            rhs: limit_op,
            ty: iv_ty,
        },
    ];
    let block = |id: u32, instructions: Vec<Instruction>, terminator| BasicBlock {
        label: BlockId(id),
        source_spans: vec![Span::dummy(); instructions.len()],
        instructions,
        terminator,
    };
    let mut f = IrFunction::new("byte_sum".into(), acc_ty, vec![], false);
    f.blocks = vec![
        block(0, pre, Terminator::Branch(BlockId(1))),
        block(
            1,
            header,
            Terminator::CondBranch {
                cond: v(cmp.0),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
        ),
        block(2, body, Terminator::Branch(BlockId(1))),
        block(3, vec![], Terminator::Return(Some(v(acc.0)))),
    ];
    f.next_value_id = 400;
    f.next_label = 4;
    f
}

fn the_loop(f: &IrFunction) -> (loop_analysis::NaturalLoop, CfgAnalysis) {
    let cfg = CfgAnalysis::build(f);
    let loops =
        loop_analysis::find_natural_loops(cfg.num_blocks, &cfg.preds, &cfg.succs, &cfg.idom);
    assert_eq!(loops.len(), 1, "fixture must have exactly one loop");
    (loops.into_iter().next().unwrap(), cfg)
}

/// x86-64 ISA for this test thread: SSE2 baseline, optionally AVX2.
fn set_isa(avx2: bool) {
    set_x86_simd_isa(true, avx2, avx2, false);
    set_x86_64_vector_intrinsics_available(true);
}

// ── the two-world interpreter ───────────────────────────────────────────

#[derive(Clone, Debug)]
enum Val {
    S(u64),
    V(Vec<u8>),
}

fn signed(ty: IrType) -> bool {
    matches!(ty, IrType::I8 | IrType::I16 | IrType::I32 | IrType::I64)
}

/// The signed integer type of `ty`'s width.
fn sized_signed(ty: IrType) -> IrType {
    match ty.size() {
        1 => IrType::I8,
        2 => IrType::I16,
        4 => IrType::I32,
        _ => IrType::I64,
    }
}

fn mask(x: u64, ty: IrType) -> u64 {
    match ty.size() {
        8 => x,
        n => x & ((1u64 << (8 * n)) - 1),
    }
}

/// Sign- or zero-extend a `ty`-typed bit pattern to 64 bits.
fn extend(x: u64, ty: IrType) -> u64 {
    let n = ty.size();
    if n == 8 {
        return x;
    }
    let x = mask(x, ty);
    if signed(ty) && (x >> (8 * n - 1)) & 1 == 1 {
        x | !((1u64 << (8 * n)) - 1)
    } else {
        x
    }
}

struct Machine<'a> {
    a: &'a [u8],
    b: &'a [u8],
    /// Live bytes per stream: any byte read at index >= n is an over-read.
    n: usize,
    values: FxHashMap<Value, Val>,
}

impl Machine<'_> {
    fn s(&self, op: &Operand) -> u64 {
        match op {
            Operand::Const(c) => c.to_i64().expect("integer constant") as u64,
            Operand::Value(v) => match &self.values[v] {
                Val::S(x) => *x,
                Val::V(_) => panic!("vector value v{} read as scalar", v.0),
            },
        }
    }
    fn vec(&self, op: &Operand) -> Vec<u8> {
        match op {
            Operand::Value(v) => match &self.values[v] {
                Val::V(x) => x.clone(),
                Val::S(_) => panic!("scalar value v{} read as vector", v.0),
            },
            Operand::Const(_) => panic!("constant used as a vector operand"),
        }
    }
    fn byte(&self, addr: u64) -> u8 {
        let (stream, base) = if addr >= BASE_B {
            (self.b, BASE_B)
        } else {
            (self.a, BASE_A)
        };
        let idx = (addr - base) as usize;
        assert!(
            idx < self.n,
            "load at stream index {idx} reads past the {} live bytes",
            self.n
        );
        stream[idx]
    }
    fn exec(&mut self, inst: &Instruction) {
        let (dest, val) = match inst {
            Instruction::GlobalAddr { dest, name } => {
                (*dest, Val::S(if name == "a" { BASE_A } else { BASE_B }))
            }
            Instruction::Copy { dest, src } => (*dest, Val::S(self.s(src))),
            Instruction::GetElementPtr {
                dest, base, offset, ..
            } => (
                *dest,
                Val::S(self.s(&Operand::Value(*base)).wrapping_add(self.s(offset))),
            ),
            Instruction::Load { dest, ptr, ty, .. } => {
                assert_eq!(ty.size(), 1, "byte-sum fixtures only load bytes");
                (
                    *dest,
                    Val::S(u64::from(self.byte(self.s(&Operand::Value(*ptr))))),
                )
            }
            Instruction::Cast {
                dest,
                src,
                from_ty,
                to_ty,
            } => (*dest, Val::S(mask(extend(self.s(src), *from_ty), *to_ty))),
            Instruction::BinOp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            } => {
                let (l, r) = (extend(self.s(lhs), *ty), extend(self.s(rhs), *ty));
                let x = match op {
                    IrBinOp::Add => l.wrapping_add(r),
                    IrBinOp::Sub => l.wrapping_sub(r),
                    IrBinOp::Mul => l.wrapping_mul(r),
                    IrBinOp::And => l & r,
                    IrBinOp::Or => l | r,
                    IrBinOp::Xor => l ^ r,
                    IrBinOp::Shl => l.wrapping_shl(r as u32),
                    IrBinOp::LShr => mask(l, *ty) >> r,
                    IrBinOp::SDiv => ((l as i64) / (r as i64)) as u64,
                    IrBinOp::UDiv => mask(l, *ty) / mask(r, *ty),
                    other => panic!("interpreter has no {other:?}"),
                };
                (*dest, Val::S(mask(x, *ty)))
            }
            Instruction::Cmp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            } => {
                // The PREDICATE carries the signedness; the type only the
                // width (the mask canonicalisation emits signed compares at
                // U8, exactly as the backend's `cmpb` + `setl` reads them).
                let (l, r) = (self.s(lhs), self.s(rhs));
                let sext = |x: u64| extend(x, sized_signed(*ty)) as i64;
                let (sl, sr) = (sext(l), sext(r));
                let (ul, ur) = (mask(l, *ty), mask(r, *ty));
                let t = match op {
                    IrCmpOp::Eq => ul == ur,
                    IrCmpOp::Ne => ul != ur,
                    IrCmpOp::Slt => sl < sr,
                    IrCmpOp::Sle => sl <= sr,
                    IrCmpOp::Sgt => sl > sr,
                    IrCmpOp::Sge => sl >= sr,
                    IrCmpOp::Ult => ul < ur,
                    IrCmpOp::Ule => ul <= ur,
                    IrCmpOp::Ugt => ul > ur,
                    IrCmpOp::Uge => ul >= ur,
                };
                (*dest, Val::S(u64::from(t)))
            }
            Instruction::Select {
                dest,
                cond,
                true_val,
                false_val,
                ty,
            } => {
                let pick = if self.s(cond) != 0 {
                    true_val
                } else {
                    false_val
                };
                (*dest, Val::S(mask(self.s(pick), *ty)))
            }
            Instruction::Intrinsic {
                dest: Some(dest),
                op,
                args,
                ..
            } => (*dest, self.intrinsic(op, args)),
            other => panic!("interpreter has no {other:?}"),
        };
        self.values.insert(dest, val);
    }
    fn intrinsic(&self, op: &IntrinsicOp, args: &[Operand]) -> Val {
        use IntrinsicOp as O;
        let lanes = |w: usize| -> Vec<u8> { vec![0; w] };
        let bytewise = |f: &dyn Fn(u8, u8) -> u8| {
            let (x, y) = (self.vec(&args[0]), self.vec(&args[1]));
            Val::V(x.iter().zip(&y).map(|(a, b)| f(*a, *b)).collect())
        };
        let qwords = |x: &[u8]| -> Vec<u64> {
            x.chunks(8)
                .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
                .collect()
        };
        let from_qwords = |q: Vec<u64>| Val::V(q.iter().flat_map(|x| x.to_le_bytes()).collect());
        match op {
            O::VecLoadI32x4 | O::VecLoadI32x8 => {
                let w = if *op == O::VecLoadI32x4 { 16 } else { 32 };
                let mut addr = self.s(&args[0]).wrapping_add(self.s(&args[1]));
                if let Some(d) = args.get(2) {
                    addr = addr.wrapping_add(self.s(d));
                }
                Val::V((0..w).map(|i| self.byte(addr + i as u64)).collect())
            }
            O::VecZeroI64x2 => Val::V(lanes(16)),
            O::VecZeroI64x4 => Val::V(lanes(32)),
            O::VecBroadcastI32x4 | O::VecBroadcastI32x8 => {
                let w = if *op == O::VecBroadcastI32x4 { 16 } else { 32 };
                let d = (self.s(&args[0]) as u32).to_le_bytes();
                Val::V((0..w).map(|i| d[i % 4]).collect())
            }
            O::VecBroadcastI8x16 => Val::V(vec![self.s(&args[0]) as u8; 16]),
            O::VecBroadcastI8x32 => Val::V(vec![self.s(&args[0]) as u8; 32]),
            O::VecAndI32x4 | O::VecAndI32x8 => bytewise(&|a, b| a & b),
            O::VecOrI32x4 | O::VecOrI32x8 => bytewise(&|a, b| a | b),
            O::VecXorI32x4 | O::VecXorI32x8 => bytewise(&|a, b| a ^ b),
            O::VecAddI8x16 | O::VecAddI8x32 => bytewise(&|a, b| a.wrapping_add(b)),
            O::VecSubI8x16 | O::VecSubI8x32 => bytewise(&|a, b| a.wrapping_sub(b)),
            O::VecMinU8x16 | O::VecMinU8x32 => bytewise(&|a, b| a.min(b)),
            O::VecMaxU8x16 | O::VecMaxU8x32 => bytewise(&|a, b| a.max(b)),
            O::VecCmpI8x16 | O::VecCmpI8x32 => {
                let imm = self.s(&args[2]);
                let f = move |a: u8, b: u8| -> u8 {
                    let (sa, sb) = (a as i8, b as i8);
                    let t = match imm {
                        0 => a == b,
                        1 => sa < sb,
                        2 => sa <= sb,
                        4 => a != b,
                        5 => a < b,
                        6 => a <= b,
                        other => panic!("byte compare predicate {other}"),
                    };
                    if t { 0xFF } else { 0 }
                };
                bytewise(&f)
            }
            O::VecBlendvI8x16 | O::VecBlendvI8x32 => {
                // args = [false, true, mask]; the mask byte's sign selects.
                let (f, t, m) = (self.vec(&args[0]), self.vec(&args[1]), self.vec(&args[2]));
                Val::V(
                    (0..f.len())
                        .map(|i| if m[i] & 0x80 != 0 { t[i] } else { f[i] })
                        .collect(),
                )
            }
            O::VecSadbwU8x16 | O::VecSadbwU8x32 => {
                let (x, y) = (self.vec(&args[0]), self.vec(&args[1]));
                from_qwords(
                    x.chunks(8)
                        .zip(y.chunks(8))
                        .map(|(p, q)| {
                            p.iter()
                                .zip(q)
                                .map(|(a, b)| u64::from(a.abs_diff(*b)))
                                .sum()
                        })
                        .collect(),
                )
            }
            O::VecAddI64x2 | O::VecAddI64x4 => {
                let (x, y) = (qwords(&self.vec(&args[0])), qwords(&self.vec(&args[1])));
                from_qwords(x.iter().zip(&y).map(|(a, b)| a.wrapping_add(*b)).collect())
            }
            O::VecHorizontalAddI64x2 | O::VecHorizontalAddI64x4 => Val::S(
                qwords(&self.vec(&args[0]))
                    .iter()
                    .fold(0u64, |s, x| s.wrapping_add(*x)),
            ),
            other => panic!("interpreter has no intrinsic {other:?}"),
        }
    }
}

/// Run `f` on streams `a`/`b` with `n` live bytes; return the masked result.
fn run(f: &IrFunction, a: &[u8], b: &[u8], n: usize) -> u64 {
    let mut m = Machine {
        a,
        b,
        n,
        values: FxHashMap::default(),
    };
    let mut current = BlockId(0);
    let mut previous = BlockId(u32::MAX);
    for _ in 0..100_000 {
        let blk = f.blocks.iter().find(|b| b.label == current).expect("block");
        // Phis read the values of the PREVIOUS block, all at once.
        let phis: Vec<(Value, Val)> = blk
            .instructions
            .iter()
            .filter_map(|i| match i {
                Instruction::Phi { dest, incoming, ty } => {
                    let (op, _) = incoming
                        .iter()
                        .find(|(_, l)| *l == previous)
                        .expect("phi edge");
                    Some((
                        *dest,
                        match op {
                            Operand::Value(v) => match &m.values[v] {
                                Val::S(x) => Val::S(mask(*x, *ty)),
                                vector => vector.clone(),
                            },
                            Operand::Const(c) => Val::S(mask(c.to_i64().unwrap() as u64, *ty)),
                        },
                    ))
                }
                _ => None,
            })
            .collect();
        m.values.extend(phis);
        for inst in &blk.instructions {
            if !matches!(inst, Instruction::Phi { .. }) {
                m.exec(inst);
            }
        }
        previous = current;
        current = match &blk.terminator {
            Terminator::Branch(l) => *l,
            Terminator::CondBranch {
                cond,
                true_label,
                false_label,
            } => {
                if m.s(cond) != 0 {
                    *true_label
                } else {
                    *false_label
                }
            }
            Terminator::Return(Some(v)) => return mask(m.s(v), f.return_type),
            other => panic!("unexpected terminator {other:?}"),
        };
    }
    panic!("byte-sum fixture did not terminate")
}

fn data_sets(len: usize) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut lcg = 0x2545_F491_u32;
    let mut rnd = || {
        lcg = lcg.wrapping_mul(1_103_515_245).wrapping_add(12345);
        (lcg >> 16) as u8
    };
    let random: Vec<u8> = (0..len).map(|_| rnd()).collect();
    let random_b: Vec<u8> = (0..len).map(|_| rnd()).collect();
    vec![
        (vec![0; len], vec![255; len]),
        (vec![255; len], vec![0; len]),
        (vec![0x80; len], vec![0x7f; len]),
        (
            (0..len).map(|i| i as u8).collect(),
            (0..len).map(|i| !(i as u8)).collect(),
        ),
        // Byte 7 (the predicate's needle) on a stride that is coprime to both
        // packed widths, so it lands in every lane position and the tail.
        (
            (0..len).map(|i| if i % 5 == 0 { 7 } else { 200 }).collect(),
            random_b.clone(),
        ),
        (random, random_b),
    ]
}

/// The core differential: analyze, transform, then execute both versions.
fn check(shape: Shape, acc_ty: IrType, iv_ty: IrType, limit: Limit, seed: i64, avx2: bool) {
    set_isa(avx2);
    let original = fixture(shape, acc_ty, iv_ty, limit, seed);
    let (lp, cfg) = the_loop(&original);
    let pattern = analyze_byte_count_loop(&original, &lp, &cfg);
    let ctx = format!("{shape:?} acc={acc_ty:?} iv={iv_ty:?} {limit:?} seed={seed} avx2={avx2}");
    let Some(pattern) = pattern else {
        assert_eq!(
            shape.expected(),
            None,
            "{ctx}: refused a shape with an exact packed form"
        );
        return;
    };
    assert_eq!(
        Some(pattern.kind),
        shape.expected(),
        "{ctx}: wrong byte-sum kind"
    );
    let want_width = if avx2 {
        ByteSumWidth::Ymm32
    } else {
        ByteSumWidth::Xmm16
    };
    assert_eq!(pattern.width, want_width, "{ctx}: wrong width");
    let mut rewritten = original.clone();
    assert!(
        transform_byte_count_reduction(&mut rewritten, &pattern) > 0,
        "{ctx}: no rewrite"
    );
    // Codegen reads line info by instruction index: every pre-existing
    // block's span table stays in step; new blocks carry none or a full one.
    for (bi, b) in rewritten.blocks.iter().enumerate() {
        assert!(
            b.source_spans.len() == b.instructions.len()
                || (bi >= original.blocks.len() && b.source_spans.is_empty()),
            "{ctx}: block {bi} span table out of step ({} spans, {} instructions)",
            b.source_spans.len(),
            b.instructions.len()
        );
    }
    let sad = if avx2 {
        IntrinsicOp::VecSadbwU8x32
    } else {
        IntrinsicOp::VecSadbwU8x16
    };
    let ops: Vec<IntrinsicOp> = rewritten
        .blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .filter_map(|i| match i {
            Instruction::Intrinsic { op, .. } => Some(*op),
            _ => None,
        })
        .collect();
    assert!(ops.contains(&sad), "{ctx}: no {sad:?} in {ops:?}");
    let n = limit.n() as usize;
    for (a, b) in data_sets(n.max(1)) {
        assert_eq!(
            run(&original, &a, &b, n),
            run(&rewritten, &a, &b, n),
            "{ctx}: packed result differs (a[0..4]={:?})",
            &a[..a.len().min(4)]
        );
    }
}

#[test]
fn interpreted_byte_sums_match_the_scalar_loop_at_both_widths() {
    let shapes = [
        Shape::Plain,
        Shape::MaskLow,
        Shape::XorStreams,
        Shape::SelectGt,
        Shape::TruncPlus3,
        Shape::SignedPlain,
        Shape::SignedTrunc,
        Shape::SignedReinterpret,
        Shape::PredEq,
        Shape::PredNe,
        Shape::PredNeOfEq,
        Shape::PredOrStreams,
    ];
    for avx2 in [false, true] {
        // Trip counts straddle one and two packed iterations of each width
        // plus a remainder of every parity; 64 is the smallest trip either
        // width accepts as a constant.
        for n in [64, 65, 79, 95, 96, 127, 128, 131, 200] {
            for shape in shapes {
                check(shape, IrType::I64, IrType::I64, Limit::Const(n), 0, avx2);
                check(shape, IrType::I32, IrType::I32, Limit::Dynamic(n), -5, avx2);
            }
        }
        // Accumulator / IV type matrix, with seeds right below the wrap
        // point of the accumulator so the exit's truncation is exercised.
        for shape in shapes {
            for (acc_ty, seed) in [
                (IrType::U32, 0xFFFF_FF00),
                (IrType::I32, i64::from(i32::MAX) - 100),
                (IrType::U64, -200),
                (IrType::I64, i64::MIN + 5),
            ] {
                for iv_ty in [IrType::I32, IrType::U32, IrType::I64, IrType::U64] {
                    check(shape, acc_ty, iv_ty, Limit::Dynamic(97), seed, avx2);
                }
            }
        }
    }
}

#[test]
fn dynamic_trip_counts_below_one_packed_iteration_run_only_the_remainder() {
    // A runtime limit is admitted at any value; 0..W-1 must execute zero
    // packed iterations and leave everything to the remainder (and 0 must
    // return the seed untouched).
    for avx2 in [false, true] {
        for n in [0, 1, 2, 15, 16, 17, 31, 32, 33] {
            for shape in [
                Shape::Plain,
                Shape::SignedPlain,
                Shape::PredEq,
                Shape::PredNe,
            ] {
                check(shape, IrType::I64, IrType::I64, Limit::Dynamic(n), 11, avx2);
            }
        }
    }
}

/// The packed body of a predicate count is the bare lane mask: a `!=`
/// packs as ONE equality compare (the exit complements it), `!= 0` over a
/// mask folds away, and no vector ever pays the `& 1`.
#[test]
fn predicate_counts_sum_the_raw_mask() {
    for avx2 in [false, true] {
        set_isa(avx2);
        for (shape, want_cmps, want_ors) in [
            (Shape::PredEq, vec![0], 0),
            (Shape::PredNe, vec![0], 0),
            (Shape::PredNeOfEq, vec![0], 0),
            (Shape::PredOrStreams, vec![0, 0], 1),
        ] {
            let mut f = fixture(shape, IrType::I64, IrType::I64, Limit::Dynamic(200), 0);
            let (lp, cfg) = the_loop(&f);
            let p = analyze_byte_count_loop(&f, &lp, &cfg).expect("admitted");
            assert!(transform_byte_count_reduction(&mut f, &p) > 0);
            let (mut cmps, mut ors, mut ands) = (vec![], 0, 0);
            for inst in f.blocks.iter().flat_map(|b| &b.instructions) {
                if let Instruction::Intrinsic { op, args, .. } = inst {
                    use IntrinsicOp as O;
                    match op {
                        O::VecCmpI8x16 | O::VecCmpI8x32 => match &args[2] {
                            Operand::Const(c) => cmps.push(c.to_i64().unwrap()),
                            other => panic!("non-constant predicate {other:?}"),
                        },
                        O::VecOrI32x4 | O::VecOrI32x8 => ors += 1,
                        O::VecAndI32x4 | O::VecAndI32x8 => ands += 1,
                        _ => {}
                    }
                }
            }
            let ctx = format!("{shape:?} avx2={avx2}");
            assert_eq!(cmps, want_cmps, "{ctx}: compare predicates");
            assert_eq!(ors, want_ors, "{ctx}: mask ORs");
            assert_eq!(ands, 0, "{ctx}: a packed `& 1` survived");
            // The mask sum is scaled out by an exact division: one multiply
            // by 255's inverse mod 2^64, never a real division.
            let insts = || f.blocks.iter().flat_map(|b| &b.instructions);
            assert!(
                insts().any(|i| matches!(i, Instruction::BinOp {
                    op: IrBinOp::Mul, rhs: Operand::Const(IrConst::I64(k)), ..
                } if *k as u64 == 0xFEFE_FEFE_FEFE_FEFF)),
                "{ctx}: no exact-division multiply"
            );
            // (The signed vector-end `SDiv` by the lane count is the trip
            // computation, not the mask scaling.)
            assert!(
                !insts().any(|i| matches!(
                    i,
                    Instruction::BinOp {
                        op: IrBinOp::UDiv | IrBinOp::SDiv,
                        rhs: Operand::Const(c),
                        ..
                    } if c.to_i64() == Some(255)
                )),
                "{ctx}: a division by 255 survived"
            );
        }
    }
    set_isa(false);
}

/// Every byte value through every predicate count, across lane positions
/// and a remainder (293 = 256 + 37: all values, then a ragged tail).
#[test]
fn predicate_counts_are_exact_over_every_byte_value() {
    for avx2 in [false, true] {
        for shape in [
            Shape::PredEq,
            Shape::PredNe,
            Shape::PredNeOfEq,
            Shape::PredOrStreams,
        ] {
            for (acc_ty, seed) in [(IrType::I64, 3), (IrType::U32, 0xFFFF_FFF0)] {
                check(shape, acc_ty, IrType::I64, Limit::Dynamic(293), seed, avx2);
                check(shape, acc_ty, IrType::I32, Limit::Const(293), seed, avx2);
            }
        }
    }
}

#[test]
fn byte_sum_refuses_shapes_without_an_exact_packed_form() {
    for avx2 in [false, true] {
        set_isa(avx2);
        for shape in [
            Shape::ZeroMinusCmp,
            Shape::DiffStreams,
            Shape::SignedPlusOne,
        ] {
            let f = fixture(shape, IrType::I64, IrType::I64, Limit::Const(200), 0);
            let (lp, cfg) = the_loop(&f);
            assert!(
                analyze_byte_count_loop(&f, &lp, &cfg).is_none(),
                "{shape:?} must stay scalar (avx2={avx2})"
            );
        }
    }
}

/// A constant trip count decides both loops at compile time: the packed
/// loop becomes a guard-free do-while (its header no longer tests, the
/// latch does), the remainder starts at the constant packed byte count
/// (no truncate/extend of the byte IV), and it is elided entirely when the
/// limit is a multiple of the width.  Every remainder 0..=W past the
/// two-vector admission floor, for every IV type and sum kind, is also
/// interpreted against the scalar loop.
#[test]
fn constant_trip_counts_bottom_test_both_loops() {
    let shapes = [
        Shape::Plain,
        Shape::TruncPlus3,
        Shape::SignedPlain,
        Shape::PredEq,
        Shape::PredNe,
        Shape::PredNeOfEq,
    ];
    for (avx2, lanes) in [(false, 16i64), (true, 32)] {
        for iv_ty in [IrType::I32, IrType::U32, IrType::I64, IrType::U64] {
            for n in (2 * lanes)..=(3 * lanes) {
                for shape in shapes {
                    check(shape, IrType::I32, iv_ty, Limit::Const(n), 7, avx2);
                    set_isa(avx2);
                    let f = fixture(shape, IrType::I32, iv_ty, Limit::Const(n), 7);
                    let (lp, cfg) = the_loop(&f);
                    let ctx = format!("{shape:?} iv={iv_ty:?} n={n} avx2={avx2}");
                    let p = analyze_byte_count_loop(&f, &lp, &cfg)
                        .unwrap_or_else(|| panic!("{ctx}: refused"));
                    let mut g = f.clone();
                    assert!(transform_byte_count_reduction(&mut g, &p) > 0, "{ctx}");
                    assert!(
                        matches!(g.blocks[p.header_idx].terminator, Terminator::Branch(_)),
                        "{ctx}: the packed header still tests the trip count"
                    );
                    assert!(
                        matches!(
                            g.blocks[p.latch_idx].terminator,
                            Terminator::CondBranch { .. }
                        ),
                        "{ctx}: the packed latch does not test the trip count"
                    );
                    let added = g.blocks.len() - f.blocks.len();
                    let want = if n % lanes == 0 { 1 } else { 4 };
                    assert_eq!(added, want, "{ctx}: remainder blocks");
                    let packed = n / lanes * lanes;
                    assert!(
                        g.blocks
                            .iter()
                            .flat_map(|b| &b.instructions)
                            .any(|i| matches!(
                                i,
                                Instruction::Copy { src: Operand::Const(c), .. }
                                    if c.to_i64() == Some(packed)
                            )),
                        "{ctx}: the remainder does not start at the constant {packed}"
                    );
                }
            }
        }
    }
}

#[test]
fn constant_trip_counts_below_two_packed_iterations_stay_scalar() {
    for (avx2, lanes) in [(false, 16i64), (true, 32)] {
        set_isa(avx2);
        for (n, admitted) in [(2 * lanes - 1, false), (2 * lanes, true)] {
            let f = fixture(Shape::Plain, IrType::I64, IrType::I64, Limit::Const(n), 0);
            let (lp, cfg) = the_loop(&f);
            assert_eq!(
                analyze_byte_count_loop(&f, &lp, &cfg).is_some(),
                admitted,
                "n={n} avx2={avx2}"
            );
        }
    }
}

#[test]
fn byte_sum_width_follows_the_target() {
    // x86-64 with AVX2: 32 lanes, unless SSE2 is forced.
    set_isa(true);
    assert_eq!(byte_sum_width(false), Some(ByteSumWidth::Ymm32));
    assert_eq!(byte_sum_width(true), Some(ByteSumWidth::Xmm16));
    // x86-64 baseline: 16 lanes.
    set_isa(false);
    assert_eq!(byte_sum_width(false), Some(ByteSumWidth::Xmm16));
    // i686 (SSE2 not in the baseline; the exit needs a 64-bit GPR): none.
    set_x86_64_vector_intrinsics_available(false);
    assert_eq!(byte_sum_width(false), None);
    // -mno-sse / -mgeneral-regs-only: none.
    set_x86_simd_isa(false, false, false, false);
    set_x86_64_vector_intrinsics_available(true);
    assert_eq!(byte_sum_width(false), None);
    set_isa(false);
}

#[test]
fn classify_byte_sum_intervals_and_value_positions() {
    let load = || MapExpr::Load(0);
    let cmp = || {
        MapExpr::Cmp(
            IrCmpOp::Eq,
            Box::new(load()),
            Box::new(MapExpr::Invariant(konst(7, IrType::I32))),
        )
    };
    // Interval edges.
    assert_eq!(
        classify_byte_sum(&load(), Some((0, 255))),
        Some(ByteSumKind::Unsigned)
    );
    assert_eq!(
        classify_byte_sum(&load(), Some((0, 127))),
        Some(ByteSumKind::Unsigned)
    );
    assert_eq!(
        classify_byte_sum(&load(), Some((-128, 127))),
        Some(ByteSumKind::Signed)
    );
    assert_eq!(classify_byte_sum(&load(), Some((0, 256))), None);
    assert_eq!(classify_byte_sum(&load(), Some((-129, 0))), None);
    assert_eq!(classify_byte_sum(&load(), Some((-1, 255))), None);
    assert_eq!(classify_byte_sum(&load(), None), None);
    // A bare compare is a predicate count; under arithmetic it is a VALUE
    // and must be refused even though its (packed) interval looks harmless.
    assert_eq!(
        classify_byte_sum(&cmp(), Some((-1, 0))),
        Some(ByteSumKind::Predicate)
    );
    let zero_minus = MapExpr::BinOp(
        IrBinOp::Sub,
        Box::new(MapExpr::Invariant(konst(0, IrType::I32))),
        Box::new(cmp()),
    );
    assert_eq!(classify_byte_sum(&zero_minus, Some((0, 1))), None);
    // A mask is fine as a select CONDITION.
    let select = MapExpr::Select(
        Box::new(cmp()),
        Box::new(load()),
        Box::new(MapExpr::Invariant(konst(0, IrType::I32))),
    );
    assert_eq!(
        classify_byte_sum(&select, Some((0, 255))),
        Some(ByteSumKind::Unsigned)
    );
    // min/max has no scalar mirror: refused before any mutation.
    let minmax = MapExpr::MinMax {
        is_max: false,
        l: Box::new(load()),
        r: Box::new(MapExpr::Invariant(konst(9, IrType::I32))),
    };
    assert_eq!(classify_byte_sum(&minmax, Some((0, 9))), None);
}
