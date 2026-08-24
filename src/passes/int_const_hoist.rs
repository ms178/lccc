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
                for_each_int_operand(inst, &mut |op, needs_reg| {
                    if let Some(bits) = large_int_const(op, needs_reg) {
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
///   movz/movk per iteration.
/// * x86-64: `cmp/add/sub` (and `imul r,r,i`) take a SIGNED imm32 directly,
///   so only constants outside i32 pay a `movabsq`. The dominant source is
///   div_by_const's magic multipliers (e.g. 2454267027 is encodable as
///   imm32 for imul's 3-operand form, but the sign-extended I64 sequence
///   uses movabsq — and the pass conservatively hoists any i64-out-of-range
///   constant).
///
/// Returns the value as u64 bits.
fn large_int_const(op: &Operand, needs_reg: bool) -> Option<u64> {
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
        if (-4095..=4095).contains(&v) && AARCH64.with(|c| c.get()) {
            return None; // AArch64 imm12 / cmn encodable — free already
        }
        if v >= i32::MIN as i64 && v <= i32::MAX as i64 {
            return None; // x86-64 imm32 encodable
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
fn for_each_int_operand(inst: &Instruction, f: &mut dyn FnMut(&Operand, bool)) {
    match inst {
        Instruction::BinOp { op, lhs, rhs, .. } => {
            let needs_reg = match op {
                IrBinOp::SDiv | IrBinOp::UDiv | IrBinOp::SRem | IrBinOp::URem => true,
                IrBinOp::Mul => AARCH64.with(|c| c.get()),
                _ => false,
            };
            f(lhs, needs_reg);
            f(rhs, needs_reg);
        }
        Instruction::Cmp { lhs, rhs, .. } => {
            f(lhs, false);
            f(rhs, false);
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
