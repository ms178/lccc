//! Integer constant hoisting for loop bodies.
//!
//! Integer constants that do not fit the AArch64 add/cmp immediate forms
//! (imm12: 0..=4095, or cmn negative range) are otherwise materialized with
//! movz/movk inside the loop on every iteration — sieve's marking-loop bound
//! `cmp j, #10000000` cost two instructions per iteration. This pass
//! materializes each distinct large constant once in the loop preheader as a
//! `Copy`, making it a register-allocatable SSA value (the register steal
//! rebalances it if a hotter value needs the register more).
//!
//! Constants are collected from BinOp/Cmp operands (the forms whose immediate
//! encoding is range-limited); small constants and zero (`wzr`/`xzr`) are
//! already free. A hoisted value is reused by nested loops (the outer
//! preheader dominates them) but never across sibling loops.

use super::loop_analysis;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use std::cell::Cell;

thread_local! {
    /// Whether the current target is AArch64 (imm12 immediate model).
    static AARCH64: Cell<bool> = const { Cell::new(false) };
}

/// Record the current target for the immediate-encoding model. Called by
/// the driver before the pass runs.
pub(crate) fn set_target_aarch64(is_aarch64: bool) {
    AARCH64.with(|c| c.set(is_aarch64));
}
use crate::common::types::IrType;
use crate::ir::analysis;
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, IrFunction, Operand, Value};

/// Run integer-constant hoisting on a function. Returns the number of
/// distinct constants materialized in preheaders.
pub(crate) fn run(func: &mut IrFunction) -> usize {
    if func.blocks.len() < 2 {
        return 0;
    }
    let cfg = analysis::CfgAnalysis::build(func);
    let mut loops = loop_analysis::merge_loops_by_header(loop_analysis::find_natural_loops(
        cfg.num_blocks,
        &cfg.preds,
        &cfg.succs,
        &cfg.idom,
    ));
    if loops.is_empty() {
        return 0;
    }
    // Outermost first so constants used by nested loops hoist as far out as
    // possible and inner loops can reuse the dominating value.
    loops.sort_by_key(|l| l.body.len());
    loops.reverse();

    // const bits -> (hoisted value id, body of the loop whose preheader defines it)
    let mut hoisted: FxHashMap<u64, (u32, FxHashSet<usize>)> = FxHashMap::default();
    let mut count = 0;
    for lp in &loops {
        let Some(preheader) = loop_analysis::find_preheader(lp.header, &lp.body, &cfg.preds) else {
            continue;
        };
        // Distinct large constants used in this loop body.
        let mut consts: Vec<u64> = Vec::new();
        let mut seen: FxHashSet<u64> = FxHashSet::default();
        for &bi in &lp.body {
            for inst in &func.blocks[bi].instructions {
                for_each_int_operand(inst, &mut |op, needs_reg, imm32_exact| {
                    if let Some(bits) = large_int_const(op, needs_reg, imm32_exact) {
                        if seen.insert(bits) {
                            consts.push(bits);
                        }
                    }
                });
            }
        }

        for bits in consts {
            // Reuse a previously hoisted value when its definition dominates
            // this loop (i.e. this loop is nested inside the defining one).
            let reusable = hoisted
                .get(&bits)
                .filter(|(_, body)| lp.body.is_subset(body));
            let new_val = if let Some(&(vid, _)) = reusable {
                vid
            } else {
                let vid = func.next_value_id;
                func.next_value_id += 1;
                func.blocks[preheader].instructions.push(Instruction::Copy {
                    dest: Value(vid),
                    src: Operand::Const(IrConst::I64(bits as i64)),
                });
                hoisted.insert(bits, (vid, lp.body.clone()));
                count += 1;
                vid
            };
            // Rewrite uses within the loop body.
            for &bi in &lp.body {
                for inst in &mut func.blocks[bi].instructions {
                    rewrite_int_operands(inst, bits, new_val);
                }
            }
        }
    }
    count
}

/// An integer constant that is NOT encodable as a target immediate and so
/// would be materialized into a register inside the loop:
///
/// * AArch64: anything outside imm12 (0..=4095) / cmn (-4095..=-1) pays
///   movz/movk per iteration. The i32 test below does NOT apply here — imm12
///   is the limit, so `10000000` genuinely needs hoisting.
/// * x86-64: 64-bit instructions sign-extend imm32, so only constants inside
///   *signed* i32 are free; anything else pays a `movabsq`. The dominant
///   source is div_by_const's magic multipliers (e.g. 2454267027 is out of the
///   signed range, so the sign-extended I64 sequence uses movabsq).
///   32-bit instructions are different: they take the imm32 bit pattern
///   *verbatim*, so the unsigned half `[2^31, 2^32)` is free too and must not
///   be hoisted. This covers essentially every hash multiplier in existence
///   (0x9E3779B1, 0x85EBCA77, 0xCC9E2D51, 0x1B873593).
///
/// `imm32_exact` says the consuming instruction encodes a *verbatim* imm32 bit
/// pattern (x86-64 32-bit ALU/imul/cmp forms) rather than sign-extending it.
///
/// Returns the value as u64 bits.
fn large_int_const(op: &Operand, needs_reg: bool, imm32_exact: bool) -> Option<u64> {
    let v: i64 = match op {
        Operand::Const(IrConst::I8(v)) => *v as i64,
        Operand::Const(IrConst::I16(v)) => *v as i64,
        Operand::Const(IrConst::I32(v)) => *v as i64,
        Operand::Const(IrConst::I64(v)) => *v,
        _ => return None,
    };
    // Operand positions with no immediate encoding at all (div/rem on every
    // target; mul on AArch64) pay a register materialization even for small
    // constants — except that on x86-64 a div-by-constant is expanded by
    // div_by_const into imul+shift whose immediates ARE encodable, so by the
    // time this pass sees the loop, small divisors are gone. Constants that
    // survive here are the real per-iteration movabs/movz materializations.
    if !needs_reg {
        if AARCH64.with(|c| c.get()) {
            // AArch64 add/sub/cmp take imm12 (0..=4095, optionally shifted
            // left by 12) and cmn covers the small negative range; everything
            // else pays movz/movk *per iteration*.
            //
            // The signed-i32 test below must NOT apply here: it used to, which
            // silently disabled this pass for every constant in [-2^31, 2^31) —
            // including the sieve marking bound `cmp j, #10000000` that
            // motivated the pass in the first place.
            if (-4095..=4095).contains(&v) {
                return None; // imm12 / cmn encodable — free already
            }
        } else {
            // x86-64: a 32-bit instruction (`imull`, `andl`, `cmpl`, ...) uses
            // the imm32 bit pattern verbatim, so all of [0, u32::MAX] is a free
            // immediate — not just the signed half. Mirrors the backend's
            // `const_as_imm32_typed(op, is_32bit_op)`.
            //
            // Hoisting such a constant is a pessimization, not a win: it turns
            // a self-contained 3-operand `imull $0x9e3779b1, %edi, %eax` into
            // `movl $imm, %eax; movq %rax, slot` in the preheader plus a
            // memory-operand `imull slot, %eax` on every iteration — one extra
            // load per iteration, one extra live register across the loop, and
            // more code. This is what lz4's hash multiplier degenerated into.
            if imm32_exact && (0..=u32::MAX as i64).contains(&v) {
                return None;
            }
            // 64-bit instructions sign-extend imm32, so only the signed range
            // is representable there.
            if v >= i32::MIN as i64 && v <= i32::MAX as i64 {
                return None; // x86-64 imm32 encodable
            }
        }
    } else if AARCH64.with(|c| !c.get()) && v >= i32::MIN as i64 && v <= i32::MAX as i64 {
        // x86-64 div/rem: only out-of-i32 constants force movabsq; an imm32
        // constant costs one movl — cheaper than a hoisted register's
        // prologue pressure in short loops.
        return None;
    }
    Some(v as u64)
}

/// Visit operands of the instruction forms with range-limited immediate encodings.
/// `needs_reg` is true when the operand position has NO immediate encoding at
/// all (div/rem on both targets; mul on AArch64), so even a small constant
/// pays a materialization there (Lev Kropp's 558e3ed9 insight, generalized
/// per target).
fn for_each_int_operand(inst: &Instruction, f: &mut dyn FnMut(&Operand, bool, bool)) {
    match inst {
        Instruction::BinOp {
            op, lhs, rhs, ty, ..
        } => {
            let needs_reg = match op {
                IrBinOp::SDiv | IrBinOp::UDiv | IrBinOp::SRem | IrBinOp::URem => true,
                IrBinOp::Mul => AARCH64.with(|c| c.get()),
                _ => false,
            };
            // Shift counts are a separate encoding (imm8 / %cl) and never take
            // a 32-bit immediate, so they get no bit-pattern relaxation.
            let imm32_exact = !matches!(
                op,
                IrBinOp::Shl
                    | IrBinOp::LShr
                    | IrBinOp::AShr
                    | IrBinOp::RotateLeft
                    | IrBinOp::RotateRight
            ) && matches!(ty, IrType::I32 | IrType::U32);
            f(lhs, needs_reg, imm32_exact);
            f(rhs, needs_reg, imm32_exact);
        }
        Instruction::Cmp { lhs, rhs, ty, .. } => {
            // `cmpl $imm32` takes the bit pattern verbatim (comparison.rs
            // computes `use_32bit` from the same type). Narrower compares use
            // imm8/imm16 and get no relaxation.
            let imm32_exact = matches!(ty, IrType::I32 | IrType::U32);
            f(lhs, false, imm32_exact);
            f(rhs, false, imm32_exact);
        }
        _ => {}
    }
}

/// Replace operands equal to the constant `bits` with the hoisted value.
/// Position-agnostic by value: a constant rewritten at one operand position
/// (e.g. a div magic multiplier) is identical everywhere it appears.
fn rewrite_int_operands(inst: &mut Instruction, bits: u64, new_val: u32) {
    let sub = |op: &mut Operand| {
        let matches = match op {
            Operand::Const(IrConst::I8(v)) => (*v as i64) as u64 == bits,
            Operand::Const(IrConst::I16(v)) => (*v as i64) as u64 == bits,
            Operand::Const(IrConst::I32(v)) => (*v as i64) as u64 == bits,
            Operand::Const(IrConst::I64(v)) => (*v as u64) == bits,
            _ => false,
        };
        if matches {
            *op = Operand::Value(Value(new_val));
        }
    };
    match inst {
        Instruction::BinOp { lhs, rhs, .. } | Instruction::Cmp { lhs, rhs, .. } => {
            sub(lhs);
            sub(rhs);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, IrCmpOp, Terminator};

    /// Run `f` with the AArch64 immediate model enabled, then restore the
    /// previous setting (the flag is a thread-local shared with other passes).
    fn with_aarch64<R>(f: impl FnOnce() -> R) -> R {
        let prev = AARCH64.with(|c| c.replace(true));
        let r = f();
        AARCH64.with(|c| c.set(prev));
        r
    }

    /// preheader -> header -> body -> header, header -> exit, where the body
    /// holds a single `acc = acc <op> K` in type `ty`.
    fn make_op_loop(op: IrBinOp, ty: IrType, k: IrConst) -> IrFunction {
        let mut func = IrFunction::new("mul_loop".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I32(0)),
                },
                Instruction::Copy {
                    dest: Value(1),
                    src: Operand::Const(IrConst::I32(64)),
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(2),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(5)), BlockId(2)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(3),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Value(Value(1)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(3)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::BinOp {
                    dest: Value(4),
                    op,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(k),
                    ty,
                },
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Value(Value(4)))),
            source_spans: Vec::new(),
        });
        func.next_value_id = 6;
        func
    }

    /// The rhs operand of the body's operation after `run`.
    fn mul_rhs(func: &IrFunction) -> Operand {
        match &func.blocks[2].instructions[0] {
            Instruction::BinOp { rhs, .. } => rhs.clone(),
            other => panic!("expected BinOp, got {other:?}"),
        }
    }

    // ---- x86-64: the hash-multiplier regression -------------------------

    /// 0x9E3779B1 (lz4/xxHash/zstd) is outside *signed* i32 but is a verbatim
    /// imm32 bit pattern for a 32-bit `imull`, so it must stay an immediate.
    /// Hoisting it produced `imull 0x28(%rsp), %eax` — a load per iteration
    /// plus a preheader materialization, where GCC emits `imull $imm, %r, %r`.
    #[test]
    fn x86_hash_multiplier_stays_an_immediate_for_32bit_mul() {
        let mut func = make_op_loop(IrBinOp::Mul, IrType::U32, IrConst::I64(0x9E37_79B1));
        assert_eq!(run(&mut func), 0);
        assert!(
            matches!(mul_rhs(&func), Operand::Const(IrConst::I64(0x9E37_79B1))),
            "32-bit hash multiplier was hoisted: {:?}",
            mul_rhs(&func)
        );
    }

    /// Same constant in a 64-bit multiply DOES need hoisting: `imulq $imm32`
    /// sign-extends, so 0x9E3779B1 would become 0xFFFFFFFF9E3779B1.
    #[test]
    fn x86_hash_multiplier_is_hoisted_for_64bit_mul() {
        let mut func = make_op_loop(IrBinOp::Mul, IrType::U64, IrConst::I64(0x9E37_79B1));
        assert_eq!(run(&mut func), 1);
        assert!(
            matches!(mul_rhs(&func), Operand::Value(_)),
            "64-bit multiplier must be materialized once, got {:?}",
            mul_rhs(&func)
        );
    }

    /// A signed-i32 constant is a free imm32 at either width — never hoist.
    #[test]
    fn x86_signed_i32_constant_is_never_hoisted() {
        let mut func = make_op_loop(IrBinOp::Mul, IrType::I64, IrConst::I64(1000));
        assert_eq!(run(&mut func), 0);
        assert!(matches!(mul_rhs(&func), Operand::Const(_)));
    }

    /// Above u32 there is no imm32 encoding at any width.
    #[test]
    fn x86_constant_above_u32_is_hoisted_even_for_32bit_mul() {
        let mut func = make_op_loop(IrBinOp::Mul, IrType::U32, IrConst::I64(0x1_0000_0000));
        assert_eq!(run(&mut func), 1);
        assert!(matches!(mul_rhs(&func), Operand::Value(_)));
    }

    /// Shift counts have no 32-bit immediate form (imm8/%cl), so the
    /// bit-pattern relaxation must not apply to them.
    #[test]
    fn shift_operands_get_no_bitpattern_relaxation() {
        let inst = Instruction::BinOp {
            dest: Value(0),
            op: IrBinOp::Shl,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I64(0x9E37_79B1)),
            ty: IrType::U32,
        };
        let mut seen = Vec::new();
        for_each_int_operand(&inst, &mut |op, _needs_reg, imm32_exact| {
            if matches!(op, Operand::Const(_)) {
                seen.push(imm32_exact);
            }
        });
        assert_eq!(seen, vec![false]);
    }

    /// 32-bit compares take the bit pattern verbatim, so an unsigned-half
    /// bound must not be hoisted either.
    #[test]
    fn cmp_operands_get_the_bitpattern_relaxation() {
        let inst = Instruction::Cmp {
            dest: Value(0),
            op: IrCmpOp::Eq,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I64(0xFFFF_FFF0)),
            ty: IrType::U32,
        };
        let mut hoisted = Vec::new();
        for_each_int_operand(&inst, &mut |op, needs_reg, imm32_exact| {
            hoisted.push(large_int_const(op, needs_reg, imm32_exact));
        });
        assert_eq!(hoisted, vec![None, None]);
    }

    // ---- AArch64: the pass used to be a no-op here ----------------------

    /// imm12 tops out at 4095, so the sieve marking bound `cmp j, #10000000`
    /// — the pass's own motivating example — sits inside signed i32 yet still
    /// pays movz/movk every iteration. The i32 shortcut must not suppress it.
    /// Driven through `Add`: on AArch64 `Mul` is a needs_reg position and would
    /// hoist unconditionally, masking the gate under test.
    #[test]
    fn aarch64_hoists_i32_range_immediate_operand() {
        with_aarch64(|| {
            let mut func = make_op_loop(IrBinOp::Add, IrType::I64, IrConst::I64(10_000_000));
            assert_eq!(run(&mut func), 1);
            assert!(matches!(mul_rhs(&func), Operand::Value(_)));
        });
    }

    /// Anything imm12/cmn can encode is already free on AArch64.
    #[test]
    fn aarch64_leaves_imm12_range_alone() {
        with_aarch64(|| {
            for k in [0i64, 1, 1000, 4095, -1, -4095] {
                let mut func = make_op_loop(IrBinOp::Add, IrType::I64, IrConst::I64(k));
                assert_eq!(run(&mut func), 0, "imm12 constant {k} was hoisted");
                assert!(matches!(mul_rhs(&func), Operand::Const(_)));
            }
        });
    }

    /// 4096 is the first AArch64 value that needs materializing.
    #[test]
    fn aarch64_hoists_just_past_imm12() {
        with_aarch64(|| {
            let mut func = make_op_loop(IrBinOp::Add, IrType::I64, IrConst::I64(4096));
            assert_eq!(run(&mut func), 1);
        });
    }

    /// The x86 bit-pattern relaxation must not leak into AArch64: 0x9E3779B1
    /// has no AArch64 immediate form and must be hoisted.
    #[test]
    fn aarch64_still_hoists_the_x86_bitpattern_constant() {
        with_aarch64(|| {
            let mut func = make_op_loop(IrBinOp::Add, IrType::U32, IrConst::I64(0x9E37_79B1));
            assert_eq!(run(&mut func), 1);
            assert!(matches!(mul_rhs(&func), Operand::Value(_)));
        });
    }

    /// AArch64 `mul` has no immediate form at all, so even a small constant
    /// must be hoisted out of the loop there.
    #[test]
    fn aarch64_hoists_mul_operand_regardless_of_size() {
        with_aarch64(|| {
            let mut func = make_op_loop(IrBinOp::Mul, IrType::U32, IrConst::I64(3));
            assert_eq!(run(&mut func), 1);
        });
    }
}
