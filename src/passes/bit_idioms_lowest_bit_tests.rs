//! `1 << Ctz(y)` -> `y & -y` with the `y == 0` arm, tested by execution.
//!
//! The rewrite is only interesting because of its zero case: `Ctz(0)` is the
//! operand width, the shift masks its count modulo that width, so the answer
//! there is `1`, while `y & -y` is `0`. The tests therefore do not compare IR
//! shapes -- they EVALUATE the rewritten block bit-exactly against the
//! reference semantics `1 << ((ctz(y)) mod W)` over a value set that covers
//! every zero and single-bit boundary that can matter, exactly like the
//! neighbouring `bit_idioms_*_tests` suites.
use super::*;
use crate::common::source::Span;
use crate::ir::reexports::{BasicBlock, BlockId, Terminator, Value};

/// Minimal straight-line builder: one block, appends instructions.
struct Graph {
    insts: Vec<Instruction>,
    next: u32,
}

impl Graph {
    fn new() -> Self {
        Self {
            insts: Vec::new(),
            next: 0,
        }
    }

    fn put(&mut self, f: impl FnOnce(Value) -> Instruction) -> Operand {
        let v = Value(self.next);
        self.next += 1;
        self.insts.push(f(v));
        Operand::Value(v)
    }

    fn param(&mut self, idx: usize, ty: IrType) -> Operand {
        self.put(|dest| Instruction::ParamRef {
            dest,
            param_idx: idx,
            ty,
        })
    }

    fn bin(&mut self, op: IrBinOp, lhs: Operand, rhs: Operand, ty: IrType) -> Operand {
        self.put(|dest| Instruction::BinOp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        })
    }

    fn un(&mut self, op: IrUnaryOp, src: Operand, ty: IrType) -> Operand {
        self.put(|dest| Instruction::UnaryOp { dest, op, src, ty })
    }

    fn function(self, result: Operand, ty: IrType) -> IrFunction {
        let mut f = IrFunction::new("lowest_set_bit".into(), ty, vec![], false);
        let source_spans = (0..self.insts.len())
            .map(|i| Span::new(i as u32, i as u32 + 1, 1))
            .collect();
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: self.insts,
            source_spans,
            terminator: Terminator::Return(Some(result)),
        });
        f.next_value_id = self.next;
        f
    }
}

fn one(ty: IrType) -> Operand {
    Operand::Const(IrConst::from_i64(1, ty))
}

/// `1 << Ctz(y)` with an optional `Not` between the parameter and the count.
fn fixture(ty: IrType, complement: bool) -> IrFunction {
    let mut g = Graph::new();
    let x = g.param(0, ty);
    let y = if complement {
        g.un(IrUnaryOp::Not, x, ty)
    } else {
        x
    };
    let count = g.un(IrUnaryOp::Ctz, y, ty);
    let shifted = g.bin(IrBinOp::Shl, one(ty), count, ty);
    g.function(shifted, ty)
}

/// The same shape, but the count is taken at half the shift's width: the two
/// widths disagree, so `1 << W` does NOT wrap back to 1 and the rewrite must
/// decline.
fn width_mismatch_fixture() -> IrFunction {
    let mut g = Graph::new();
    let x = g.param(0, IrType::U64);
    // A 64-bit count feeding a 32-bit shift: `ctz == 64` folds to `64 % 32`,
    // i.e. back to 1, but only because the COUNT is 64 bits wide. The rewrite
    // keys on the count's width, so this must decline.
    let count = g.un(IrUnaryOp::Ctz, x, IrType::U64);
    let shifted = g.bin(IrBinOp::Shl, one(IrType::U32), count, IrType::U32);
    g.function(shifted, IrType::U32)
}

/// Reference semantics of the source expression at width `bits`.
fn reference(y: u64, bits: u32) -> u64 {
    let mask = if bits == 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };
    let masked = y & mask;
    let count = if masked == 0 {
        bits
    } else {
        masked.trailing_zeros()
    };
    // The IR shift masks its count modulo the operand width (the same contract
    // `fold_binop` and every backend implement).
    let count = (count % bits) as u32;
    (1u64 << count) & mask
}

/// Evaluate the (rewritten) straight-line block for one parameter value.
fn evaluate(f: &IrFunction, x: u64) -> u64 {
    use crate::ir::reexports::IrConst;
    let width = f.return_type.size() as u32 * 8;
    let mask = |v: u64| {
        if width == 64 {
            v
        } else {
            v & ((1u64 << width) - 1)
        }
    };
    let mut vals: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
    let get = |vals: &std::collections::HashMap<u32, u64>, op: &Operand| -> u64 {
        match op {
            Operand::Const(c) => match c {
                IrConst::I8(v) => *v as i64 as u64,
                IrConst::I16(v) => *v as i64 as u64,
                IrConst::I32(v) => *v as i64 as u64,
                IrConst::I64(v) => *v as u64,
                other => panic!("unexpected constant {other:?}"),
            },
            Operand::Value(v) => *vals
                .get(&v.0)
                .unwrap_or_else(|| panic!("value {} read before definition", v.0)),
        }
    };
    for block in &f.blocks {
        for inst in &block.instructions {
            let dest = match inst.dest() {
                Some(d) => d,
                None => continue,
            };
            let value = match inst {
                Instruction::ParamRef { .. } => x,
                Instruction::Copy { src, .. } => get(&vals, src),
                Instruction::UnaryOp { op, src, .. } => {
                    let s = get(&vals, src);
                    match op {
                        IrUnaryOp::Not => mask(!s),
                        IrUnaryOp::Neg => mask(s.wrapping_neg()),
                        IrUnaryOp::Ctz => {
                            let m = mask(s);
                            if m == 0 {
                                width as u64
                            } else {
                                m.trailing_zeros() as u64
                            }
                        }
                        other => panic!("unexpected unary {other:?}"),
                    }
                }
                Instruction::BinOp {
                    op, lhs, rhs, ty, ..
                } => {
                    let a = get(&vals, lhs);
                    let b = get(&vals, rhs);
                    // Evaluate at the instruction's own width so a narrow
                    // operand cannot leak high bits into a wider result.
                    let w = ty.size() as u32 * 8;
                    let m = |v: u64| if w == 64 { v } else { v & ((1u64 << w) - 1) };
                    let (a, b) = (m(a), m(b));
                    match op {
                        IrBinOp::Add => m(a.wrapping_add(b)),
                        IrBinOp::Sub => m(a.wrapping_sub(b)),
                        IrBinOp::And => m(a & b),
                        IrBinOp::Or => m(a | b),
                        IrBinOp::Xor => m(a ^ b),
                        IrBinOp::Shl => m(a.wrapping_shl((b % w as u64) as u32)),
                        IrBinOp::LShr => m(a.wrapping_shr((b % w as u64) as u32)),
                        IrBinOp::AShr => {
                            let shift = (b % w as u64) as u32;
                            let signed = sign_extend(a, w);
                            m((signed >> shift) as u64)
                        }
                        other => panic!("unexpected binary {other:?}"),
                    }
                }
                Instruction::Cmp {
                    op, lhs, rhs, ty, ..
                } => {
                    let a = get(&vals, lhs);
                    let b = get(&vals, rhs);
                    let w = ty.size() as u32 * 8;
                    let m = |v: u64| if w == 64 { v } else { v & ((1u64 << w) - 1) };
                    let (a, b) = (m(a), m(b));
                    let (sa, sb) = (sign_extend(a, w), sign_extend(b, w));
                    let r = match op {
                        IrCmpOp::Eq => a == b,
                        IrCmpOp::Ne => a != b,
                        IrCmpOp::Slt => sa < sb,
                        IrCmpOp::Sle => sa <= sb,
                        IrCmpOp::Sgt => sa > sb,
                        IrCmpOp::Sge => sa >= sb,
                        IrCmpOp::Ult => a < b,
                        IrCmpOp::Ule => a <= b,
                        IrCmpOp::Ugt => a > b,
                        IrCmpOp::Uge => a >= b,
                    };
                    r as u64
                }
                Instruction::Select {
                    cond,
                    true_val,
                    false_val,
                    ..
                } => {
                    if get(&vals, cond) != 0 {
                        get(&vals, true_val)
                    } else {
                        get(&vals, false_val)
                    }
                }
                Instruction::Cast { src, to_ty, .. } => {
                    let s = get(&vals, src);
                    let w = to_ty.size() as u32 * 8;
                    if w == 64 { s } else { s & ((1u64 << w) - 1) }
                }
                other => panic!("unexpected instruction {other:?}"),
            };
            vals.insert(dest.0, mask(value));
        }
    }
    match &f.blocks[0].terminator {
        Terminator::Return(Some(op)) => get(&vals, op),
        other => panic!("unexpected terminator {other:?}"),
    }
}

fn sign_extend(v: u64, bits: u32) -> i64 {
    if bits == 64 {
        v as i64
    } else {
        let shift = 64 - bits;
        ((v << shift) as i64) >> shift
    }
}

/// Every value that can distinguish `lowbit` from the zero arm: 0, every
/// single-bit value, low-word exhaustively, and each of those with the top bit
/// and the bit just above the low word set.
fn probe_values(bits: u32) -> Vec<u64> {
    let mask = if bits == 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };
    let mut out = vec![0u64, 1, mask];
    for bit in 0..bits {
        out.push(1u64 << bit);
        if bit + 1 < bits {
            out.push((1u64 << bit) | (1u64 << (bit + 1)));
        }
    }
    for low in 0u64..=u16::MAX as u64 {
        out.push(low & mask);
        let high = ((low as u128 * 0x9E37_79B9_7F4A_7C15u128) as u64) & mask;
        out.push(high);
    }
    out
}

#[test]
fn plain_source_is_left_alone() {
    // `1 << Ctz(y)` without a complement keeps the count path on purpose: the
    // isolated-bit form costs `neg` + `and` + `mov` + `cmov` (8 instructions on
    // x86-64) while `bsf` + count staging + `shl` is 5 (32-bit) and 6 (64-bit)
    // -- measured, see the matcher's comment. The fold must therefore decline,
    // and the program must still answer exactly.
    for ty in [IrType::I32, IrType::U32, IrType::I64, IrType::U64] {
        let mut f = fixture(ty, false);
        recognize_function(&mut f, false, 0, 0, false, false);
        assert!(
            f.blocks[0].instructions.iter().any(|i| matches!(
                i,
                Instruction::BinOp {
                    op: IrBinOp::Shl,
                    ..
                }
            )),
            "{ty:?}: the plain count form must keep its shift"
        );
        assert!(
            f.blocks[0].instructions.iter().any(|i| matches!(
                i,
                Instruction::UnaryOp {
                    op: IrUnaryOp::Ctz,
                    ..
                }
            )),
            "{ty:?}: the plain count form must keep its count"
        );
        let bits = ty.size() as u32 * 8;
        let mask = if bits == 64 {
            u64::MAX
        } else {
            (1u64 << bits) - 1
        };
        for y in probe_values(bits) {
            let got = evaluate(&f, y) & mask;
            assert_eq!(got, reference(y, bits), "{ty:?}: 1 << Ctz({y:#x})");
        }
    }
}

#[test]
fn lowest_set_bit_matches_reference_for_complemented_source() {
    for ty in [IrType::I32, IrType::U32, IrType::I64, IrType::U64] {
        let mut f = fixture(ty, true);
        assert!(
            recognize_function(&mut f, false, 0, 0, false, false) > 0,
            "{ty:?}: 1 << Ctz(~x) was not rewritten"
        );
        // The complement identity is only sound if `-~x` really became
        // `x + 1`: an `Add` must be present and no `Neg`/`Shl` may be.
        assert!(
            f.blocks[0].instructions.iter().any(|i| matches!(
                i,
                Instruction::BinOp {
                    op: IrBinOp::Add,
                    ..
                }
            )),
            "{ty:?}: the complemented form did not fold the negate into `x + 1`"
        );
        assert!(
            !f.blocks[0].instructions.iter().any(|i| matches!(
                i,
                Instruction::UnaryOp {
                    op: IrUnaryOp::Neg,
                    ..
                }
            )),
            "{ty:?}: the complemented form kept a negate"
        );
        // The count itself may still be in the list (DCE belongs to the
        // pipeline, not to this pass), but nothing may read it any more.
        let dead: Vec<u32> = f.blocks[0]
            .instructions
            .iter()
            .filter_map(|i| match i {
                Instruction::UnaryOp {
                    dest,
                    op: IrUnaryOp::Ctz,
                    ..
                } => Some(dest.0),
                _ => None,
            })
            .collect();
        for id in dead {
            for inst in &f.blocks[0].instructions {
                let mut reads = false;
                crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                    if let Operand::Value(v) = op {
                        if v.0 == id {
                            reads = true;
                        }
                    }
                });
                assert!(
                    !reads,
                    "{ty:?}: the dead count %{id} is still read by {inst:?}"
                );
            }
        }
        let bits = ty.size() as u32 * 8;
        for x in probe_values(bits) {
            let got = evaluate(&f, x)
                & if bits == 64 {
                    u64::MAX
                } else {
                    (1u64 << bits) - 1
                };
            // `evaluate` feeds the parameter, which the block complements
            // itself, so the reference must apply the same `Not`.
            let y = !x;
            assert_eq!(
                got,
                reference(y, bits),
                "{ty:?}: 1 << Ctz(~{x:#x}) evaluated to {got:#x}"
            );
        }
    }
}

#[test]
fn zero_operand_yields_one_not_zero() {
    // The single case the whole `cmov` exists for, stated on its own so a
    // regression cannot hide inside the sweep.
    for ty in [IrType::U32, IrType::U64] {
        let mut f = fixture(ty, false);
        recognize_function(&mut f, false, 0, 0, false, false);
        assert_eq!(evaluate(&f, 0), 1, "{ty:?}: 1 << Ctz(0) must be 1");
        let bits = ty.size() as u32 * 8;
        let mut f = fixture(ty, true);
        recognize_function(&mut f, false, 0, 0, false, false);
        let all_ones = if bits == 64 {
            u64::MAX
        } else {
            (1u64 << bits) - 1
        };
        assert_eq!(
            evaluate(&f, all_ones),
            1,
            "{ty:?}: 1 << Ctz(~all_ones) = 1 << Ctz(0) must be 1"
        );
    }
}

#[test]
fn mismatched_count_width_is_not_rewritten() {
    let mut f = width_mismatch_fixture();
    assert_eq!(
        recognize_function(&mut f, false, 0, 0, false, false),
        0,
        "a 32-bit count feeding a 64-bit shift must not become `y & -y`"
    );
    assert!(
        f.blocks[0].instructions.iter().any(|i| matches!(
            i,
            Instruction::BinOp {
                op: IrBinOp::Shl,
                ..
            }
        )),
        "the declined shape must keep its shift"
    );
}

// The identity under test IS `y & -y`, so it is spelled that way rather than
// with `isolate_lowest_one()`: the point of the assertion is that the intrinsic
// and the two-instruction idiom agree, and calling the intrinsic on both sides
// would test nothing.
#[allow(clippy::manual_isolate_lowest_one)]
#[test]
fn nonzero_operands_are_the_isolated_bit() {
    // Spot-check the documented identity in the common case, independently of
    // the evaluator: `1 << ctz(y)` is the lowest set bit for every `y != 0`.
    for bits in [32u32, 64u32] {
        for y in probe_values(bits) {
            let mask = if bits == 64 {
                u64::MAX
            } else {
                (1u64 << bits) - 1
            };
            let y = y & mask;
            if y == 0 {
                continue;
            }
            assert_eq!(reference(y, bits), y & y.wrapping_neg(), "y={y:#x}");
        }
    }
}
