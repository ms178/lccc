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
use crate::common::types::IrType;
use crate::ir::analysis;
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, IrFunction, Operand, Value};
use std::cell::Cell;

/// Target immediate-encoding model that drives the hoist decision. One
/// constructor per backend the pass runs on (see the driver gate in
/// `passes/mod.rs`); `i686` is deliberately excluded — nothing this pass
/// sees pays a 64-bit materialization there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ImmModel {
    /// x86-64: 64-bit instructions sign-extend imm32 (only the signed range
    /// is free at 64-bit width); 32-bit instructions take the imm32 bit
    /// pattern verbatim (the whole u32 range is free).
    X86_64,
    /// AArch64: add/sub/cmp take imm12 (0..=4095) / cmn (-4095..=-1);
    /// mul has no immediate form at all.
    Aarch64,
    /// RISC-V (RV64): no ALU immediate reaches this pass's operand forms
    /// beyond imm12 — every integer constant materializes through `li`
    /// (register-form-only emitters), and `li` itself is one `addi` inside
    /// [-2048, 2047] and a `lui`+`addi` pair outside it.
    Riscv,
}

thread_local! {
    /// Current target immediate-encoding model.
    static MODEL: Cell<ImmModel> = const { Cell::new(ImmModel::X86_64) };
}

/// Record the current target for the immediate-encoding model. Called by
/// the driver before the pass runs.
pub(crate) fn set_target_model(model: ImmModel) {
    MODEL.with(|c| c.set(model));
}

/// The immediate model of the targets this pass runs on (`None`: i686,
/// where nothing it sees pays a 64-bit materialization).
pub(crate) fn model_for(target: crate::backend::Target) -> Option<ImmModel> {
    match target {
        crate::backend::Target::Aarch64 => Some(ImmModel::Aarch64),
        crate::backend::Target::Riscv64 => Some(ImmModel::Riscv),
        crate::backend::Target::X86_64 => Some(ImmModel::X86_64),
        _ => None,
    }
}

/// Whether `model` materializes `c` into a register with ONE instruction
/// that reads no register -- the case where rematerializing at a use never
/// costs more than a spill reload, so a register held for it through a hot
/// loop that does not use it can only lose (see `run`'s leaf-loop rule):
///
/// * x86-64: `movl $imm32` (zero-extends; every u32 and every 32-bit-typed
///   constant) or `movq $simm32`. A 64-bit `movabsq` is excluded: ten bytes,
///   the shape whose per-use cost the hoist exists to remove.
/// * AArch64: one `movz` (at most one non-zero halfword) or one `movn` (at
///   most one halfword other than 0xffff), in the constant's own width.
///   Bitmask (`orr`) immediates are not modelled and count as costly -- the
///   conservative side (such a constant keeps the unconditional hoist).
/// * RISC-V: one `addi` (imm12) or one `lui` (a sign-extended 32-bit value
///   with the low 12 bits clear).
pub(crate) fn single_insn_materialization(model: ImmModel, c: &IrConst) -> bool {
    let (v, wide) = match *c {
        IrConst::I8(v) => (i64::from(v), false),
        IrConst::I16(v) => (i64::from(v), false),
        IrConst::I32(v) => (i64::from(v), false),
        IrConst::I64(v) => (v, true),
        _ => return false,
    };
    match model {
        ImmModel::X86_64 => !wide || i32::try_from(v).is_ok() || u32::try_from(v).is_ok(),
        ImmModel::Aarch64 => {
            let (bits, halfwords) = if wide {
                (v as u64, 4)
            } else {
                (u64::from(v as u32), 2)
            };
            let halfword = |i: u32| (bits >> (16 * i)) & 0xffff;
            (0..halfwords).filter(|&i| halfword(i) != 0).count() <= 1
                || (0..halfwords).filter(|&i| halfword(i) != 0xffff).count() <= 1
        }
        ImmModel::Riscv => {
            (-2048..=2047).contains(&v) || (i32::try_from(v).is_ok() && v & 0xfff == 0)
        }
    }
}

/// The integer constant homes of `func` under `model`: every single-def
/// `Copy dest = Const(c)` that feeds at least one operand position where
/// this pass's own criterion ([`large_int_const_in`] with that position's
/// `needs_reg`/`imm32_exact`) says `c` costs a materialization. Late passes
/// that forward constant copies must spare these -- forwarding one re-sinks
/// the `movabsq`/`movz`+`movk`/`lui`+`addi` (or AArch64's `mov` for a `mul`
/// or `sdiv` operand, which has no immediate form at all) into every use.
/// Position-exact on purpose: the pass hoists a small constant only for a
/// register-only position, so a test on the constant alone cannot tell
/// such a home from an ordinary copy.
pub(crate) fn const_homes(func: &IrFunction, model: ImmModel) -> FxHashSet<u32> {
    let mut consts: FxHashMap<u32, Option<IrConst>> = FxHashMap::default();
    for inst in func.blocks.iter().flat_map(|b| &b.instructions) {
        if let Instruction::Copy {
            dest,
            src: Operand::Const(c),
        } = inst
        {
            // A second Copy definition poisons the entry (not single-def).
            consts
                .entry(dest.0)
                .and_modify(|e| *e = None)
                .or_insert(Some(*c));
        }
    }
    let mut homes = FxHashSet::default();
    if consts.is_empty() {
        return homes;
    }
    for inst in func.blocks.iter().flat_map(|b| &b.instructions) {
        for_each_int_operand_in(model, inst, &mut |op, needs_reg, imm32_exact| {
            if let Operand::Value(v) = op {
                if let Some(Some(c)) = consts.get(&v.0) {
                    let op = Operand::Const(*c);
                    if large_int_const_in(model, &op, needs_reg, imm32_exact).is_some() {
                        homes.insert(v.0);
                    }
                }
            }
        });
    }
    homes
}

/// Run integer-constant hoisting on a function. Returns the number of
/// distinct constants materialized in preheaders.
pub(crate) fn run(func: &mut IrFunction) -> usize {
    if func.blocks.len() < 2 {
        return 0;
    }
    // Never hoist into a sibling loop's header (see
    // `loop_preheader::dedicate_foreign_preheaders`).
    super::loop_preheader::dedicate_foreign_preheaders(func, loop_has_candidate);
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
    // The candidate constants of every loop body, taken before any rewrite
    // (an outer hoist turns the inner loops' operands into values).
    let consts_in: Vec<FxHashSet<u64>> = loops
        .iter()
        .map(|lp| {
            let mut set = FxHashSet::default();
            for &bi in &lp.body {
                for inst in &func.blocks[bi].instructions {
                    for_each_int_operand(inst, &mut |op, needs_reg, imm32_exact| {
                        if let Some(bits) = large_int_const(op, needs_reg, imm32_exact) {
                            set.insert(bits);
                        }
                    });
                }
            }
            set
        })
        .collect();
    let leaves_in = leaf_subloops(&loops);
    let model = MODEL.with(|c| c.get());

    // const bits -> (hoisted value id, body of the loop whose preheader defines it)
    let mut hoisted: FxHashMap<u64, (u32, FxHashSet<usize>)> = FxHashMap::default();
    let mut count = 0;
    for (li, lp) in loops.iter().enumerate() {
        let Some(preheader) = loop_analysis::find_preheader(lp.header, &lp.body, &cfg.preds) else {
            continue;
        };
        // Distinct large constants used in this loop body, in program order.
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
        // A constant one register-free instruction rematerializes leaves
        // this loop only if every leaf loop nested in it uses the constant:
        // otherwise the hoisted register stays live through a hot leaf loop
        // that gains nothing, to save one instruction per execution of a
        // colder block -- and under pressure costs a spill there instead
        // (measured: zlib_ng_adler32 held its `% 65521` reciprocal in %r15
        // across the byte loop and spilled the block bound, +2 stack
        // operands and a callee-saved push/pop). A leaf loop that uses it
        // still hoists it to its own preheader when its turn comes.
        consts.retain(|&bits| {
            !single_insn_materialization(model, &IrConst::I64(bits as i64))
                || leaves_in[li]
                    .iter()
                    .all(|&leaf| consts_in[leaf].contains(&bits))
        });

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
                func.blocks[preheader].push_inst(
                    Instruction::Copy {
                        dest: Value(vid),
                        src: Operand::Const(IrConst::I64(bits as i64)),
                    },
                    crate::common::source::Span::dummy(),
                );
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

/// For each loop (by index), the leaf loops strictly nested in it: loops
/// whose body is a proper subset of its own and that contain no other loop.
/// Loops merged by header are distinct bodies, so subset is nesting. A leaf
/// loop has no leaves of its own. O(n^2) set tests over a function's natural
/// loops, which stay few.
fn leaf_subloops(loops: &[loop_analysis::NaturalLoop]) -> Vec<Vec<usize>> {
    let inside = |inner: usize, outer: usize| {
        inner != outer
            && loops[inner].body.len() < loops[outer].body.len()
            && loops[inner].body.is_subset(&loops[outer].body)
    };
    let is_leaf: Vec<bool> = (0..loops.len())
        .map(|i| !(0..loops.len()).any(|j| inside(j, i)))
        .collect();
    (0..loops.len())
        .map(|outer| {
            (0..loops.len())
                .filter(|&inner| is_leaf[inner] && inside(inner, outer))
                .collect()
        })
        .collect()
}

/// Whether `run` would hoist anything for this loop.
fn loop_has_candidate(func: &IrFunction, lp: &loop_analysis::NaturalLoop) -> bool {
    let mut found = false;
    for &bi in &lp.body {
        for inst in &func.blocks[bi].instructions {
            for_each_int_operand(inst, &mut |op, needs_reg, imm32_exact| {
                found |= large_int_const(op, needs_reg, imm32_exact).is_some();
            });
            if found {
                return true;
            }
        }
    }
    false
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
/// * RISC-V (RV64): the ALU emitters are register-form-only, so every
///   constant pays a `li` materialization per iteration; inside imm12
///   ([-2048, 2047]) `li` is a single `addi` (free in place), outside it a
///   `lui`+`addi` pair (hoist — the RISC-V analogue of AArch64's under-hoist
///   before fix B: RISC-V previously never ran this pass at all).
///
/// `imm32_exact` says the consuming instruction encodes a *verbatim* imm32 bit
/// pattern (x86-64 32-bit ALU/imul/cmp forms) rather than sign-extending it.
///
/// Returns the value as u64 bits.
fn large_int_const(op: &Operand, needs_reg: bool, imm32_exact: bool) -> Option<u64> {
    large_int_const_in(MODEL.with(|c| c.get()), op, needs_reg, imm32_exact)
}

/// [`large_int_const`] under an explicit model.
fn large_int_const_in(
    model: ImmModel,
    op: &Operand,
    needs_reg: bool,
    imm32_exact: bool,
) -> Option<u64> {
    let v: i64 = match op {
        Operand::Const(IrConst::I8(v)) => *v as i64,
        Operand::Const(IrConst::I16(v)) => *v as i64,
        Operand::Const(IrConst::I32(v)) => *v as i64,
        Operand::Const(IrConst::I64(v)) => *v,
        _ => return None,
    };
    if model == ImmModel::Riscv {
        // RISC-V: the backend stages EVERY integer constant through `li`
        // (the ALU emitters are register-form only — there is no imm32
        // bit-pattern relaxation and no position without materialization),
        // so the only cost axis is `li`'s own expansion: imm12 is a single
        // `addi` (free in place — hoisting such a constant trades a free
        // one-instruction materialization for a register live across the
        // whole loop, the exact x86 "one movl is cheaper" rule), while
        // anything outside imm12 costs `lui`+`addi` on EVERY iteration —
        // the movz/movk-class per-iteration waste this pass exists to
        // remove. The `needs_reg`/`imm32_exact` axes are x86/AArch64
        // distinctions that do not exist here: the rule is uniform across
        // operand positions.
        //
        // PREMISE: "one addi is free" holds relative to the CURRENT
        // register-form-only emitters. If the RISC-V backend ever grows
        // `addi`/`andi`/`slti` immediate folds (like x86's imm32 forms), an
        // in-place imm12 becomes strictly cheaper than a hoisted register
        // even in short loops — revisit this arm then.
        if (-2048..=2047).contains(&v) {
            return None; // `li` = one addi — free in place
        }
        return Some(v as u64);
    }
    // Operand positions with no immediate encoding at all (div/rem on every
    // target; mul on AArch64) pay a register materialization even for small
    // constants — except that on x86-64 a div-by-constant is expanded by
    // div_by_const into imul+shift whose immediates ARE encodable, so by the
    // time this pass sees the loop, small divisors are gone. Constants that
    // survive here are the real per-iteration movabs/movz materializations.
    if !needs_reg {
        if model == ImmModel::Aarch64 {
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
    } else if model == ImmModel::X86_64 && v >= i32::MIN as i64 && v <= i32::MAX as i64 {
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
    for_each_int_operand_in(MODEL.with(|c| c.get()), inst, f);
}

/// [`for_each_int_operand`] under an explicit immediate model.
fn for_each_int_operand_in(
    model: ImmModel,
    inst: &Instruction,
    f: &mut dyn FnMut(&Operand, bool, bool),
) {
    match inst {
        Instruction::BinOp {
            op, lhs, rhs, ty, ..
        } => {
            let needs_reg = match op {
                IrBinOp::SDiv | IrBinOp::UDiv | IrBinOp::SRem | IrBinOp::URem => true,
                // AArch64 `mul` has no immediate form while its neighbours
                // take imm12, so a small constant is a materialization ONLY
                // at that position. RISC-V has no immediate forms anywhere
                // (uniform `li` staging), so `mul` is not special there.
                IrBinOp::Mul => model == ImmModel::Aarch64,
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
    /// previous setting (the model is a thread-local shared with other passes).
    fn with_aarch64<R>(f: impl FnOnce() -> R) -> R {
        let prev = MODEL.with(|c| c.replace(ImmModel::Aarch64));
        let r = f();
        MODEL.with(|c| c.set(prev));
        r
    }

    /// Run `f` with the RISC-V immediate model enabled, then restore the
    /// previous setting.
    fn with_riscv<R>(f: impl FnOnce() -> R) -> R {
        let prev = MODEL.with(|c| c.replace(ImmModel::Riscv));
        let r = f();
        MODEL.with(|c| c.set(prev));
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

    // ---- RISC-V: the third immediate model --------------------------------

    /// imm12 is `li` = one `addi` — as cheap as the position can get at any
    /// operand form, so it must stay in place (the x86 "one movl is cheaper"
    /// rule transposed).
    #[test]
    fn riscv_leaves_imm12_range_alone() {
        with_riscv(|| {
            for k in [0i64, 1, 100, 2047, -1, -2048] {
                let mut func = make_op_loop(IrBinOp::Add, IrType::I64, IrConst::I64(k));
                assert_eq!(run(&mut func), 0, "imm12 constant {k} was hoisted");
                assert!(matches!(mul_rhs(&func), Operand::Const(_)));
            }
        });
    }

    /// 2048 is the first value whose `li` expands to lui+addi — the per-iteration
    /// waste this pass exists to remove. Same constant, negative side.
    #[test]
    fn riscv_hoists_just_past_imm12() {
        with_riscv(|| {
            for k in [2048i64, -2049, 10_000_000] {
                let mut func = make_op_loop(IrBinOp::Add, IrType::I64, IrConst::I64(k));
                assert_eq!(run(&mut func), 1, "out-of-imm12 constant {k} not hoisted");
                assert!(matches!(mul_rhs(&func), Operand::Value(_)));
            }
        });
    }

    /// The x86-64 imm32 bit-pattern relaxation must NOT leak into RISC-V:
    /// 0x9E3779B1 has no single-`addi` encoding and must hoist there even
    /// from a 32-bit multiply.
    #[test]
    fn riscv_hoists_the_x86_bitpattern_constant() {
        with_riscv(|| {
            let mut func = make_op_loop(IrBinOp::Mul, IrType::U32, IrConst::I64(0x9E37_79B1));
            assert_eq!(run(&mut func), 1);
            assert!(matches!(mul_rhs(&func), Operand::Value(_)));
        });
    }

    /// `mul` is NOT a needs_reg position on RISC-V (no immediate forms exist
    /// anywhere, so a small constant at a mul position costs the same one
    /// `addi` as at an add position) — an imm12 multiplier stays in place.
    #[test]
    fn riscv_mul_imm12_operand_stays_an_immediate() {
        with_riscv(|| {
            let mut func = make_op_loop(IrBinOp::Mul, IrType::U32, IrConst::I64(3));
            assert_eq!(run(&mut func), 0);
            assert!(matches!(mul_rhs(&func), Operand::Const(_)));
        });
    }

    /// The model is uniform across operand positions: an out-of-imm12 constant
    /// hoists from a div/rem (needs_reg) position exactly as from an add.
    #[test]
    fn riscv_hoists_out_of_imm12_divisor() {
        with_riscv(|| {
            let mut func = make_op_loop(IrBinOp::UDiv, IrType::I64, IrConst::I64(100_000));
            assert_eq!(run(&mut func), 1);
            assert!(matches!(mul_rhs(&func), Operand::Value(_)));
        });
    }

    /// One block: `Copy %1 = Const(c)`, then `%3 = %2 <op> %1` in type `ty`
    /// (and a second Copy of %1 when `twice`), returning %3.
    fn home_func(c: IrConst, op: IrBinOp, ty: IrType, twice: bool) -> IrFunction {
        let mut func = IrFunction::new("homes".to_string(), IrType::I64, vec![], false);
        let mut instructions = vec![Instruction::Copy {
            dest: Value(1),
            src: Operand::Const(c),
        }];
        if twice {
            instructions.push(Instruction::Copy {
                dest: Value(1),
                src: Operand::Const(c),
            });
        }
        instructions.push(Instruction::Copy {
            dest: Value(2),
            src: Operand::Const(IrConst::I64(3)),
        });
        instructions.push(Instruction::BinOp {
            dest: Value(3),
            op,
            lhs: Operand::Value(Value(2)),
            rhs: Operand::Value(Value(1)),
            ty,
        });
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
            source_spans: Vec::new(),
        });
        func.next_value_id = 4;
        func
    }

    fn is_home(model: ImmModel, c: IrConst, op: IrBinOp, ty: IrType) -> bool {
        const_homes(&home_func(c, op, ty, false), model).contains(&1)
    }

    /// The homes are exactly the copies this pass would have created for
    /// their use positions: AArch64 hoists a small constant only for `mul`
    /// / `sdiv` (no immediate form), so `#7` is a home there and not under
    /// `add`; x86-64 64-bit forms sign-extend imm32 while 32-bit forms take
    /// the bit pattern verbatim; RISC-V's boundary is imm12. %2 (a small
    /// constant in the left position) is never a home, and a multiply
    /// defined copy never is either.
    #[test]
    fn const_homes_follow_the_hoisting_criterion_per_position() {
        use IrBinOp::{Add, Mul, SDiv};
        use IrType::{I32, I64};
        let a64 = ImmModel::Aarch64;
        assert!(is_home(a64, IrConst::I64(7), Mul, I64));
        assert!(is_home(a64, IrConst::I64(7), SDiv, I64));
        assert!(!is_home(a64, IrConst::I64(7), Add, I64));
        assert!(!is_home(a64, IrConst::I64(4095), Add, I64));
        assert!(is_home(a64, IrConst::I64(4096), Add, I64));

        let x86 = ImmModel::X86_64;
        assert!(!is_home(x86, IrConst::I64(7), Mul, I64));
        assert!(!is_home(x86, IrConst::I64(i32::MAX as i64), Add, I64));
        assert!(is_home(x86, IrConst::I64(1 << 32), Add, I64));
        assert!(is_home(x86, IrConst::I64(0x9e37_79b1), Mul, I64));
        assert!(!is_home(x86, IrConst::I64(0x9e37_79b1), Mul, I32));
        assert!(is_home(x86, IrConst::I64(1 << 40), SDiv, I64));

        let rv = ImmModel::Riscv;
        assert!(!is_home(rv, IrConst::I64(2047), Add, I64));
        assert!(!is_home(rv, IrConst::I64(-2048), Mul, I64));
        assert!(is_home(rv, IrConst::I64(2048), Add, I64));

        for model in [a64, x86, rv] {
            let f = home_func(IrConst::I64(1 << 40), Add, I64, false);
            assert!(!const_homes(&f, model).contains(&2), "{model:?}");
            let f = home_func(IrConst::I64(1 << 40), Add, I64, true);
            assert!(const_homes(&f, model).is_empty(), "{model:?}");
        }
        assert_eq!(model_for(crate::backend::Target::I686), None);
    }

    // ---- frequency: never hoist into a sibling loop's header -------------

    fn blk(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: Vec::new(),
        }
    }

    fn counter_loop(
        header: u32,
        body: u32,
        exit: u32,
        entry: (Operand, u32),
        iv: u32,
        body_insts: Vec<Instruction>,
    ) -> [BasicBlock; 2] {
        let (next, cmp) = (iv + 1, iv + 2);
        let mut insts = body_insts;
        insts.push(Instruction::BinOp {
            dest: Value(next),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(iv)),
            rhs: Operand::Const(IrConst::I32(1)),
            ty: IrType::I32,
        });
        [
            blk(
                header,
                vec![
                    Instruction::Phi {
                        dest: Value(iv),
                        ty: IrType::I32,
                        incoming: vec![
                            (entry.0, BlockId(entry.1)),
                            (Operand::Value(Value(next)), BlockId(body)),
                        ],
                    },
                    Instruction::Cmp {
                        dest: Value(cmp),
                        op: IrCmpOp::Slt,
                        lhs: Operand::Value(Value(iv)),
                        rhs: Operand::Const(IrConst::I32(64)),
                        ty: IrType::I32,
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(cmp)),
                    true_label: BlockId(body),
                    false_label: BlockId(exit),
                },
            ),
            blk(body, insts, Terminator::Branch(BlockId(header))),
        ]
    }

    /// Loop B's body multiplies by a constant that needs a register.
    fn wide_mul(dest: u32, src: u32) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op: IrBinOp::Mul,
            lhs: Operand::Value(Value(src)),
            rhs: Operand::Const(IrConst::I64(0x1_0000_0000)),
            ty: IrType::U64,
        }
    }

    fn block_defining_copy_of(func: &IrFunction, bits: i64) -> usize {
        func.blocks
            .iter()
            .position(|b| {
                b.instructions.iter().any(|i| {
                    matches!(i, Instruction::Copy { src: Operand::Const(IrConst::I64(v)), .. }
                        if *v == bits)
                })
            })
            .expect("hoisted copy")
    }

    /// zlib_ng_adler32's shape: loop A's header exits straight into loop B's
    /// header, so A's header is B's unique outside predecessor. Appending
    /// the hoisted copy there ran it once per A iteration (2,097,153 times,
    /// and it cost A its bottom-test rotation). The copy must land in a new
    /// dedicated block on the A-exit edge instead.
    #[test]
    fn hoist_never_lands_in_a_sibling_loop_header() {
        let mut func = IrFunction::new("sibling".to_string(), IrType::I32, vec![], false);
        func.blocks.push(blk(
            0,
            vec![Instruction::Copy {
                dest: Value(0),
                src: Operand::Const(IrConst::I32(0)),
            }],
            Terminator::Branch(BlockId(1)),
        ));
        let a = counter_loop(1, 2, 3, (Operand::Value(Value(0)), 0), 10, vec![]);
        let b = counter_loop(
            3,
            4,
            5,
            (Operand::Value(Value(10)), 1),
            20,
            vec![wide_mul(30, 20)],
        );
        func.blocks.extend(a);
        func.blocks.extend(b);
        func.blocks.push(blk(
            5,
            vec![],
            Terminator::Return(Some(Operand::Value(Value(20)))),
        ));
        func.next_value_id = 31;
        func.next_label = 6;
        let a_header_len = func.blocks[1].instructions.len();

        assert_eq!(run(&mut func), 1);

        let home = block_defining_copy_of(&func, 0x1_0000_0000);
        let home_label = func.blocks[home].label;
        assert!(
            home_label.0 >= 6,
            "copy must live in a new block, got {home_label:?}"
        );
        assert!(matches!(
            func.blocks[home].terminator,
            Terminator::Branch(BlockId(3))
        ));
        let a_header = func.blocks.iter().find(|b| b.label == BlockId(1)).unwrap();
        assert_eq!(
            a_header.instructions.len(),
            a_header_len,
            "loop A's header gained code"
        );
        assert!(matches!(&a_header.terminator,
            Terminator::CondBranch { false_label, .. } if *false_label == home_label));
        let b_header = func.blocks.iter().find(|b| b.label == BlockId(3)).unwrap();
        let Instruction::Phi { incoming, .. } = &b_header.instructions[0] else {
            panic!("expected B's phi");
        };
        assert!(incoming.iter().any(|(_, l)| *l == home_label));
        assert!(incoming.iter().all(|(_, l)| *l != BlockId(1)));
    }

    /// A guard block outside every loop runs at most once per entry: no
    /// block is spent on it and the copy goes to the guard as before.
    #[test]
    fn guard_preheader_is_not_split() {
        let mut func = IrFunction::new("guarded".to_string(), IrType::I32, vec![], false);
        func.blocks.push(blk(
            0,
            vec![Instruction::Copy {
                dest: Value(0),
                src: Operand::Const(IrConst::I32(0)),
            }],
            Terminator::CondBranch {
                cond: Operand::Value(Value(0)),
                true_label: BlockId(1),
                false_label: BlockId(3),
            },
        ));
        func.blocks.extend(counter_loop(
            1,
            2,
            3,
            (Operand::Value(Value(0)), 0),
            10,
            vec![wide_mul(30, 10)],
        ));
        func.blocks.push(blk(
            3,
            vec![],
            Terminator::Return(Some(Operand::Value(Value(0)))),
        ));
        func.next_value_id = 31;
        func.next_label = 4;

        assert_eq!(run(&mut func), 1);

        assert_eq!(func.blocks.len(), 4, "no block for a guard preheader");
        assert_eq!(
            func.blocks[block_defining_copy_of(&func, 0x1_0000_0000)].label,
            BlockId(0)
        );
    }

    /// entry -> outer header -> inner header <-> inner body; inner header ->
    /// outer latch -> outer header; outer header -> exit. One `%10 = %9 * K`
    /// (64-bit) sits in the inner body (`in_leaf`) or in the outer latch.
    fn nested_loops(k: IrConst, in_leaf: bool) -> IrFunction {
        let mut func = IrFunction::new("nested".to_string(), IrType::I64, vec![], false);
        let use_k = Instruction::BinOp {
            dest: Value(10),
            op: IrBinOp::Mul,
            lhs: Operand::Value(Value(9)),
            rhs: Operand::Const(k),
            ty: IrType::I64,
        };
        let (inner_body, latch) = if in_leaf {
            (vec![use_k], vec![])
        } else {
            (vec![], vec![use_k])
        };
        let cond = |t: u32, f: u32| Terminator::CondBranch {
            cond: Operand::Value(Value(0)),
            true_label: BlockId(t),
            false_label: BlockId(f),
        };
        let blocks = [
            (
                vec![
                    Instruction::Copy {
                        dest: Value(0),
                        src: Operand::Const(IrConst::I64(1)),
                    },
                    Instruction::Copy {
                        dest: Value(9),
                        src: Operand::Const(IrConst::I64(3)),
                    },
                ],
                Terminator::Branch(BlockId(1)),
            ),
            (vec![], cond(2, 5)),
            (vec![], cond(3, 4)),
            (inner_body, Terminator::Branch(BlockId(2))),
            (latch, Terminator::Branch(BlockId(1))),
            (vec![], Terminator::Return(Some(Operand::Value(Value(0))))),
        ];
        for (id, (instructions, terminator)) in blocks.into_iter().enumerate() {
            func.blocks.push(BasicBlock {
                label: BlockId(id as u32),
                instructions,
                terminator,
                source_spans: Vec::new(),
            });
        }
        func.next_value_id = 11;
        func
    }

    /// The multiplier operand of `%10`, wherever the pass left it.
    fn use_rhs(func: &IrFunction) -> Operand {
        func.blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .find_map(|inst| match inst {
                Instruction::BinOp {
                    dest: Value(10),
                    rhs,
                    ..
                } => Some(rhs.clone()),
                _ => None,
            })
            .expect("the multiply survives")
    }

    /// x86-64 leaf-loop rule. adler32's `% 65521` reciprocal (one `movl`)
    /// used only in the outer loop's own block stays at its use: hoisting it
    /// would hold a register through the leaf loop for nothing. Used inside
    /// the leaf loop it is hoisted. A `movabsq` constant is hoisted either
    /// way -- its per-use cost is what the pass exists to remove.
    #[test]
    fn single_insn_constants_hoist_only_into_loops_whose_leaves_use_them() {
        let recip = IrConst::I64(0x8007_8071);
        let mut f = nested_loops(recip, false);
        run(&mut f);
        assert_eq!(use_rhs(&f), Operand::Const(recip));

        let mut f = nested_loops(recip, true);
        assert!(run(&mut f) > 0);
        assert!(matches!(use_rhs(&f), Operand::Value(_)));

        let wide = IrConst::I64(1 << 40);
        let mut f = nested_loops(wide, false);
        assert!(run(&mut f) > 0);
        assert!(matches!(use_rhs(&f), Operand::Value(_)));
    }

    #[test]
    fn leaf_subloops_lists_strictly_nested_leaves() {
        let body = |b: &[usize]| loop_analysis::NaturalLoop {
            header: b[0],
            body: b.iter().copied().collect(),
        };
        // outer {1..=6} holds leaves {2,3} and {5,6}; {2,3} has none.
        let loops = vec![body(&[1, 2, 3, 4, 5, 6]), body(&[2, 3]), body(&[5, 6])];
        let leaves = leaf_subloops(&loops);
        assert_eq!(leaves[0], vec![1, 2]);
        assert!(leaves[1].is_empty() && leaves[2].is_empty());
    }

    /// The single-instruction set is exactly what each backend encodes in
    /// one register-free instruction (see [`single_insn_materialization`]).
    #[test]
    fn single_insn_materialization_matches_the_encodings() {
        let x86 = ImmModel::X86_64;
        // adler32's `% 65521` reciprocal: one `movl` (zero-extending).
        assert!(single_insn_materialization(x86, &IrConst::I64(0x8007_8071)));
        assert!(single_insn_materialization(
            x86,
            &IrConst::I64(u32::MAX as i64)
        ));
        assert!(single_insn_materialization(
            x86,
            &IrConst::I64(i32::MIN as i64)
        ));
        assert!(single_insn_materialization(x86, &IrConst::I32(-1)));
        // `movabsq` territory stays costly.
        assert!(!single_insn_materialization(x86, &IrConst::I64(1 << 32)));
        assert!(!single_insn_materialization(
            x86,
            &IrConst::I64(i32::MIN as i64 - 1)
        ));
        assert!(!single_insn_materialization(
            x86,
            &IrConst::I64(0x9e37_79b9_7f4a_7c15_u64 as i64)
        ));

        let a64 = ImmModel::Aarch64;
        assert!(single_insn_materialization(a64, &IrConst::I64(7)));
        assert!(single_insn_materialization(
            a64,
            &IrConst::I64(0xffff << 48)
        ));
        assert!(single_insn_materialization(a64, &IrConst::I64(-2))); // movn
        assert!(single_insn_materialization(a64, &IrConst::I32(-65536))); // movn w
        assert!(!single_insn_materialization(
            a64,
            &IrConst::I64(0x8007_8071)
        ));
        assert!(!single_insn_materialization(
            a64,
            &IrConst::I32(0x1234_5678)
        ));
        assert!(!single_insn_materialization(
            a64,
            &IrConst::I64(0xffff_0000_ffff)
        ));

        let rv = ImmModel::Riscv;
        assert!(single_insn_materialization(rv, &IrConst::I64(2047)));
        assert!(single_insn_materialization(rv, &IrConst::I64(-2048)));
        assert!(single_insn_materialization(rv, &IrConst::I64(0x7fff_f000)));
        assert!(single_insn_materialization(rv, &IrConst::I64(-0x8000_0000)));
        assert!(!single_insn_materialization(rv, &IrConst::I64(2048)));
        assert!(!single_insn_materialization(rv, &IrConst::I64(0x8000_0000)));
        assert!(!single_insn_materialization(rv, &IrConst::I64(0x1001)));

        for model in [x86, a64, rv] {
            assert!(!single_insn_materialization(model, &IrConst::F64(1.0)));
        }
    }
}
