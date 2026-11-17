//! Loop-invariant wide-constant hoisting.
//!
//! A 64-bit integer immediate that does not fit a sign-extended imm32 cannot
//! be encoded in an x86-64 ALU instruction: the backend materializes it with
//! a 10-byte `movabsq` into a scratch register AT EVERY USE.  When that use
//! sits inside a loop (FNV/xxhash-style `h = (h ^ b) * PRIME` recurrences:
//! sqlite_vdbe_serial, expat_xml_scan, fnv folds), the loop pays the
//! materialization once per iteration while GCC and LLVM keep the prime in a
//! register across the loop (LLVM: ConstantHoisting; GCC: RTL loop
//! invariant motion after expand).  Other targets pay the same shape in
//! different currency (AArch64: a mov/movk chain per use).
//!
//! The fix is IR-level and tiny: in a natural loop with a preheader, rewrite
//! `BinOp ... Const(K)` to use a value defined by a preheader `Copy K` so
//! register allocation can keep K live across the loop.  It must run LAST —
//! after every constant-folding, GVN, and copy-propagation round — because
//! every one of those passes would legitimately fold the copy right back
//! into the use.
//!
//! Admission is deliberately narrow:
//! - only `IrConst::I64` values outside the sign-extended imm32 range (a
//!   fitting immediate is free at the use site; hoisting it would only add
//!   register pressure);
//! - only commutative-ish ALU ops (`Add/Sub/Mul/And/Or/Xor`): division and
//!   remainder by a constant must keep the immediate visible for the
//!   backend's magic-number strength reduction, and shift amounts never
//!   exceed imm8;
//! - `Mul` by a power of two keeps its immediate: the backend turns it into
//!   a shift, which beats `imulq %reg`;
//! - only uses in blocks that DOMINATE every latch — blocks that execute on
//!   every iteration.  A wide constant on a conditional path (sqlite_varint's
//!   9-byte decode tail) pays its `movabsq` only when that path runs; pinning
//!   it in a register for the whole loop trades a rare 10-byte instruction
//!   for a callee-saved register plus push/pop in the prologue, a measured
//!   net loss;
//! - at most [`MAX_HOISTS_PER_LOOP`] distinct constants per loop (ordered by
//!   use count) so a pressured loop body cannot lose a register better spent
//!   on its own values.
//!
//! Outer loops are processed before inner ones, so a constant shared by a
//! nest is materialized once, outside the outermost loop that uses it.
//!
//! Kill switch: `CCC_DISABLE_PASSES=consthoist`.

use super::loop_analysis;
use crate::common::fx_hash::FxHashMap;
use crate::common::source::Span;
use crate::ir::analysis::CfgAnalysis;
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, IrFunction, Operand, Value};

/// Pressure cap: distinct wide constants promoted to registers per loop.
const MAX_HOISTS_PER_LOOP: usize = 2;

#[inline]
fn fits_imm32(v: i64) -> bool {
    v == (v as i32) as i64
}

/// True when the BinOp position may trade its immediate for a register
/// without defeating a backend strength reduction.
#[inline]
fn op_admits_register(op: IrBinOp, k: i64) -> bool {
    match op {
        IrBinOp::Add | IrBinOp::Sub | IrBinOp::And | IrBinOp::Or | IrBinOp::Xor => true,
        // `x * 2^n` lowers to a shift; `imulq %reg` would be a regression.
        IrBinOp::Mul => !(k as u64).is_power_of_two(),
        // Division/remainder magic-number lowering needs the immediate;
        // shifts never carry a wide amount; everything else is untouched.
        _ => false,
    }
}

#[inline]
fn candidate(op: IrBinOp, operand: &Operand) -> Option<i64> {
    match operand {
        Operand::Const(IrConst::I64(v)) if !fits_imm32(*v) && op_admits_register(op, *v) => {
            Some(*v)
        }
        _ => None,
    }
}

pub fn run(func: &mut IrFunction) -> usize {
    if func.blocks.len() < 2 {
        return 0;
    }
    let cfg = CfgAnalysis::build(func);
    let mut loops = loop_analysis::merge_loops_by_header(loop_analysis::find_natural_loops(
        cfg.num_blocks,
        &cfg.preds,
        &cfg.succs,
        &cfg.idom,
    ));
    if loops.is_empty() {
        return 0;
    }
    // Outermost first: a constant a nest shares is hoisted past every loop
    // that uses it, and the inner pass finds its operands already rewritten.
    loops.sort_by(|a, b| b.body.len().cmp(&a.body.len()));

    let mut next_id = func.max_value_id() + 1;
    let mut hoisted = 0usize;

    for lp in &loops {
        let Some(preheader) = loop_analysis::find_preheader(lp.header, &lp.body, &cfg.preds) else {
            continue;
        };
        if preheader == lp.header || preheader >= func.blocks.len() {
            continue;
        }

        // Only blocks that run on EVERY iteration participate: a block
        // dominating every latch is on every header->latch path.  A use on a
        // conditional path pays its materialization only when taken, while
        // the hoisted value would occupy a register for the whole loop.
        let latches: Vec<usize> = cfg
            .preds
            .row(lp.header)
            .iter()
            .map(|&v| v as usize)
            .filter(|b| lp.body.contains(b))
            .collect();
        let every_iteration: Vec<usize> = lp
            .body
            .iter()
            .copied()
            .filter(|&b| {
                latches
                    .iter()
                    .all(|&l| crate::ir::analysis::dominates_idom(&cfg.idom, b, l))
            })
            .collect();

        // Census: distinct wide constants and their per-iteration use counts.
        let mut counts: FxHashMap<i64, usize> = FxHashMap::default();
        for &bi in &every_iteration {
            for inst in &func.blocks[bi].instructions {
                if let Instruction::BinOp { op, lhs, rhs, .. } = inst {
                    for operand in [lhs, rhs] {
                        if let Some(k) = candidate(*op, operand) {
                            *counts.entry(k).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
        if counts.is_empty() {
            continue;
        }
        // Deterministic selection: most-used first, value as tie-break.
        let mut ranked: Vec<(i64, usize)> = counts.into_iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        ranked.truncate(MAX_HOISTS_PER_LOOP);

        let mut reg_for: FxHashMap<i64, Value> = FxHashMap::default();
        for &(k, _) in &ranked {
            let dest = Value(next_id);
            next_id += 1;
            let at = func.blocks[preheader].instructions.len();
            func.blocks[preheader].insert_inst(
                at,
                Instruction::Copy {
                    dest,
                    src: Operand::Const(IrConst::I64(k)),
                },
                Span::dummy(),
            );
            reg_for.insert(k, dest);
            hoisted += 1;
        }

        for &bi in &every_iteration {
            for inst in &mut func.blocks[bi].instructions {
                if let Instruction::BinOp { op, lhs, rhs, .. } = inst {
                    let op = *op;
                    for operand in [&mut *lhs, &mut *rhs] {
                        if let Some(k) = candidate(op, operand) {
                            if let Some(&v) = reg_for.get(&k) {
                                *operand = Operand::Value(v);
                            }
                        }
                    }
                }
            }
        }
    }

    if hoisted > 0 {
        func.next_value_id = next_id;
    }
    hoisted
}

pub fn run_module(module: &mut crate::ir::reexports::IrModule) -> usize {
    let mut total = 0;
    for func in &mut module.functions {
        if func.is_declaration {
            continue;
        }
        total += run(func);
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imm32_boundary() {
        assert!(fits_imm32(i32::MAX as i64));
        assert!(fits_imm32(i32::MIN as i64));
        assert!(!fits_imm32(i32::MAX as i64 + 1));
        assert!(!fits_imm32(i32::MIN as i64 - 1));
        assert!(!fits_imm32(1099511628211)); // FNV-1a 64 prime
    }

    #[test]
    fn mul_power_of_two_keeps_immediate() {
        assert!(!op_admits_register(IrBinOp::Mul, 1i64 << 40));
        assert!(op_admits_register(IrBinOp::Mul, 1099511628211));
        assert!(op_admits_register(IrBinOp::Xor, 1i64 << 40));
    }

    #[test]
    fn division_never_admits() {
        for op in [
            IrBinOp::SDiv,
            IrBinOp::UDiv,
            IrBinOp::SRem,
            IrBinOp::URem,
            IrBinOp::Shl,
            IrBinOp::AShr,
            IrBinOp::LShr,
        ] {
            assert!(!op_admits_register(op, 1099511628211));
        }
    }

    #[test]
    fn candidate_rejects_fitting_and_nonwide_consts() {
        assert_eq!(
            candidate(IrBinOp::Add, &Operand::Const(IrConst::I64(42))),
            None
        );
        assert_eq!(
            candidate(IrBinOp::Add, &Operand::Const(IrConst::I32(42))),
            None
        );
        assert_eq!(
            candidate(IrBinOp::Mul, &Operand::Const(IrConst::I64(1099511628211))),
            Some(1099511628211)
        );
        assert_eq!(candidate(IrBinOp::Mul, &Operand::Value(Value(3))), None);
    }
}
