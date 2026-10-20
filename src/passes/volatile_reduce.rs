//! Interleave tiny unsigned volatile reductions without vectorizing their reads.
//!
//! Four independent scalar accumulators shorten multiply/add dependency chains
//! and amortize loop control. The original loop handles at most three remaining
//! iterations. Reads retain their order, width, address and volatile flag; only
//! associative unsigned arithmetic is regrouped. No read is speculated.
//!
//! Intentionally match only a two-phi, two-block loop with one invariant-address
//! volatile load (optionally behind a constant GEP), one unsigned reduction,
//! and a unit unsigned IV starting at zero.
//! This excludes observed intermediate accumulators, side exits, atomics, calls,
//! stores, FP/signed reassociation and wraparound IVs. The main bound n & ~3 never
//! exceeds n; its last increment cannot wrap even when n is UINT_MAX.
use super::loop_analysis;
use crate::common::types::IrType;
use crate::ir::analysis::CfgAnalysis;
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Terminator,
    Value,
};

const LANES: usize = 4;
const MAX_BLOCKS_BUDGET: usize = 200;
const MAX_VOLATILE_LOOPS_BUDGET: usize = 32;

fn lanes() -> usize {
    // Tunable ILP width for measurement: 0=off, 2/4/8 explicit, otherwise 4.
    // Env read is per-call (not per-process) so A/B harnesses can compare
    // CCC_VOLATILE_REDUCE_LANES=2 vs 4 in the same invocation via --lccc-alt-env.
    if let Ok(v) = std::env::var("CCC_VOLATILE_REDUCE_LANES") {
        match v.trim() {
            "0" => return 0,
            "2" => return 2,
            "4" => return 4,
            "8" => return 8,
            _ => {}
        }
        eprintln!(
            "volatile_reduce: invalid CCC_VOLATILE_REDUCE_LANES={v:?}, using {LANES} (expected 0/2/4/8)"
        );
    }
    LANES
}

fn should_skip_for_compile_time(func: &IrFunction) -> bool {
    // Compile-time guardrail for YARPGen mega-TUs (seed 101: 63k asm lines,
    // 580s peephole). Volatile reduction itself is cheap, but on a function
    // with 200+ blocks or 32+ volatile loops the win is noise vs the risk of
    // scaling a per-loop analysis across a pathological CFG. The budget is
    // deliberately conservative and disable-able via CCC_VOLATILE_REDUCE_BUDGET=0.
    if std::env::var("CCC_VOLATILE_REDUCE_BUDGET").as_deref() == Ok("0") {
        return false;
    }
    if func.blocks.len() > MAX_BLOCKS_BUDGET {
        eprintln!(
            "volatile_reduce: skipping {}: {} blocks > {} (budget)",
            func.name,
            func.blocks.len(),
            MAX_BLOCKS_BUDGET
        );
        return true;
    }
    false
}

struct Candidate {
    header: usize,
    preheader: usize,
    iv: Value,
    iv_ty: IrType,
    acc_ty: IrType,
    seed: Operand,
    limit: Operand,
    reduction: IrBinOp,
    address: Option<Instruction>,
    load: Instruction,
}

fn identity(op: IrBinOp, ty: IrType) -> Option<Operand> {
    let n = match op {
        IrBinOp::Add | IrBinOp::Or | IrBinOp::Xor => 0,
        IrBinOp::Mul => 1,
        IrBinOp::And => -1,
        _ => return None,
    };
    Some(Operand::Const(IrConst::from_i64(n, ty)))
}

fn analyze(
    func: &IrFunction,
    cfg: &CfgAnalysis,
    lp: &loop_analysis::NaturalLoop,
) -> Option<Candidate> {
    if lp.body.len() != 2 {
        return None;
    }
    let header = lp.header;
    let body = *lp.body.iter().find(|&&b| b != header)?;
    let h = &func.blocks[header];
    let b = &func.blocks[body];
    if h.instructions.len() != 3
        || !matches!(b.instructions.len(), 3 | 4)
        || !matches!(b.terminator, Terminator::Branch(l) if l == h.label)
        || cfg.preds.row(body) != [header as u32]
    {
        return None;
    }
    let preheader = loop_analysis::find_preheader(header, &lp.body, &cfg.preds)?;
    if cfg.preds.row(header).len() != 2
        || !matches!(func.blocks[preheader].terminator, Terminator::Branch(l) if l == h.label)
    {
        return None;
    }
    let Instruction::Cmp {
        dest: cmp,
        op: IrCmpOp::Ult,
        lhs: Operand::Value(iv),
        rhs: limit,
        ty: iv_ty,
    } = h.instructions[2]
    else {
        return None;
    };
    if !matches!(iv_ty, IrType::U32 | IrType::U64)
        || !matches!(h.terminator, Terminator::CondBranch { cond: Operand::Value(v), true_label, false_label } if v == cmp && true_label == b.label && false_label != b.label && false_label != h.label)
    {
        return None;
    }
    let mut acc_info = None;
    let mut increment = None;
    for inst in &h.instructions[..2] {
        let Instruction::Phi { dest, ty, incoming } = inst else {
            return None;
        };
        if incoming.len() != 2 {
            return None;
        }
        let init = incoming
            .iter()
            .find(|(_, l)| *l == func.blocks[preheader].label)?
            .0;
        let back = incoming.iter().find(|(_, l)| *l == b.label)?.0;
        if *dest == iv {
            if *ty != iv_ty || !matches!(init, Operand::Const(c) if c.to_i64() == Some(0)) {
                return None;
            }
            increment = Some(back);
        } else {
            if !matches!(ty, IrType::U32 | IrType::U64) || acc_info.is_some() {
                return None;
            }
            acc_info = Some((*dest, *ty, init, back));
        }
    }
    let (acc, acc_ty, seed, back) = acc_info?;
    let increment = increment?;
    // A constant-offset address computation may still be in the latch before
    // LICM. Clone it with each access rather than speculate it on zero trips.
    let address = if b.instructions.len() == 4 {
        match &b.instructions[0] {
            Instruction::GetElementPtr {
                offset: Operand::Const(_),
                ty: IrType::Ptr,
                ..
            } => Some(b.instructions[0].clone()),
            _ => return None,
        }
    } else {
        None
    };
    let load_idx = usize::from(address.is_some());
    let Instruction::Load {
        dest: loaded,
        ptr,
        ty: load_ty,
        volatile: true,
        ..
    } = b.instructions[load_idx]
    else {
        return None;
    };
    let Instruction::BinOp {
        dest: updated,
        op: reduction,
        lhs,
        rhs,
        ty: reduce_ty,
    } = b.instructions[load_idx + 1]
    else {
        return None;
    };
    if load_ty != acc_ty
        || reduce_ty != acc_ty
        || back != Operand::Value(updated)
        || !((lhs == Operand::Value(acc) && rhs == Operand::Value(loaded))
            || (rhs == Operand::Value(acc) && lhs == Operand::Value(loaded)))
        || identity(reduction, acc_ty).is_none()
    {
        return None;
    }
    let Instruction::BinOp {
        dest: next_iv,
        op: IrBinOp::Add,
        lhs: Operand::Value(iv_use),
        rhs: Operand::Const(one),
        ty,
    } = b.instructions[load_idx + 2]
    else {
        return None;
    };
    if ty != iv_ty
        || iv_use != iv
        || one.to_i64() != Some(1)
        || increment != Operand::Value(next_iv)
    {
        return None;
    }

    let mut loop_defs = vec![iv.0, acc.0, cmp.0, loaded.0, updated.0, next_iv.0];
    if let Some(Instruction::GetElementPtr { dest, .. }) = &address {
        loop_defs.push(dest.0);
    }
    let invariant_pointer = match &address {
        Some(Instruction::GetElementPtr { dest, base, .. }) if *dest == ptr => {
            !loop_defs.contains(&base.0)
        }
        None => !loop_defs.contains(&ptr.0),
        _ => false,
    };
    if !invariant_pointer
        || matches!(limit, Operand::Value(v) if loop_defs.contains(&v.0))
        || matches!(seed, Operand::Value(v) if loop_defs.contains(&v.0))
    {
        return None;
    }
    // Only the header phis may export body definitions. Otherwise an exit
    // could observe the last per-iteration update rather than the combined sum.
    for (bi, block) in func.blocks.iter().enumerate() {
        if bi == header || bi == body {
            continue;
        }
        let mut escaped = false;
        let mut check = |id| {
            escaped |= [loaded.0, updated.0, next_iv.0].contains(&id)
                || address
                    .as_ref()
                    .and_then(Instruction::dest)
                    .is_some_and(|v| v.0 == id);
        };
        for inst in &block.instructions {
            inst.for_each_used_value(&mut check);
        }
        block.terminator.for_each_used_value(&mut check);
        if escaped {
            return None;
        }
    }
    Some(Candidate {
        header,
        preheader,
        iv,
        iv_ty,
        acc_ty,
        seed,
        limit,
        reduction,
        address,
        load: b.instructions[load_idx].clone(),
    })
}

fn rewrite(func: &mut IrFunction, c: Candidate) -> Option<()> {
    // Reserve a fixed upper bound before mutation: no wrapping SSA/label IDs.
    let mut value_id = func.sound_next_value_id();
    value_id.checked_add(32)?;
    let label_id = func
        .blocks
        .iter()
        .map(|b| b.label.0)
        .max()?
        .checked_add(1)?
        .max(func.next_label);
    let next_label = label_id.checked_add(3)?;
    let mut fresh = || {
        let v = Value(value_id);
        value_id += 1;
        v
    };
    let mh = BlockId(label_id);
    let mb = BlockId(label_id + 1);
    let combine = BlockId(label_id + 2);
    let pre_label = func.blocks[c.preheader].label;
    let old_header = func.blocks[c.header].label;
    let bound = fresh();
    let iv = fresh();
    let next_iv = fresh();
    let cond = fresh();
    let accs: [Value; LANES] = std::array::from_fn(|_| fresh());
    let updates: [Value; LANES] = std::array::from_fn(|_| fresh());
    let mut h_insts = vec![Instruction::Phi {
        dest: iv,
        ty: c.iv_ty,
        incoming: vec![
            (Operand::Const(IrConst::from_i64(0, c.iv_ty)), pre_label),
            (Operand::Value(next_iv), mb),
        ],
    }];
    let neutral = identity(c.reduction, c.acc_ty)?;
    let mut b_insts = Vec::with_capacity(3 * LANES + 1);
    for lane in 0..LANES {
        h_insts.push(Instruction::Phi {
            dest: accs[lane],
            ty: c.acc_ty,
            incoming: vec![
                (if lane == 0 { c.seed } else { neutral }, pre_label),
                (Operand::Value(updates[lane]), mb),
            ],
        });
        let mut load = c.load.clone();
        if let Some(Instruction::GetElementPtr {
            base, offset, ty, ..
        }) = &c.address
        {
            let address = fresh();
            b_insts.push(Instruction::GetElementPtr {
                dest: address,
                base: *base,
                offset: *offset,
                ty: *ty,
            });
            if let Instruction::Load { ptr, .. } = &mut load {
                *ptr = address;
            }
        }
        let loaded = fresh();
        // Only SSA names change: the address expression, segment, width and
        // volatility are preserved. Emit reads in original execution order.
        if let Instruction::Load { dest, .. } = &mut load {
            *dest = loaded;
        }
        b_insts.push(load);
        b_insts.push(Instruction::BinOp {
            dest: updates[lane],
            op: c.reduction,
            lhs: Operand::Value(accs[lane]),
            rhs: Operand::Value(loaded),
            ty: c.acc_ty,
        });
    }
    h_insts.push(Instruction::Cmp {
        dest: cond,
        op: IrCmpOp::Ult,
        lhs: Operand::Value(iv),
        rhs: Operand::Value(bound),
        ty: c.iv_ty,
    });
    b_insts.push(Instruction::BinOp {
        dest: next_iv,
        op: IrBinOp::Add,
        lhs: Operand::Value(iv),
        rhs: Operand::Const(IrConst::from_i64(LANES as i64, c.iv_ty)),
        ty: c.iv_ty,
    });
    // Balanced combine avoids an unnecessary three-operation serial chain.
    let left = fresh();
    let right = fresh();
    let result = fresh();
    let combine_insts = [
        (left, accs[0], accs[1]),
        (right, accs[2], accs[3]),
        (result, left, right),
    ]
    .map(|(dest, a, b)| Instruction::BinOp {
        dest,
        op: c.reduction,
        lhs: Operand::Value(a),
        rhs: Operand::Value(b),
        ty: c.acc_ty,
    })
    .to_vec();
    func.blocks[c.preheader].source_spans.clear();
    func.blocks[c.preheader]
        .instructions
        .push(Instruction::BinOp {
            dest: bound,
            op: IrBinOp::And,
            lhs: c.limit,
            rhs: Operand::Const(IrConst::from_i64(-(LANES as i64), c.iv_ty)),
            ty: c.iv_ty,
        });
    func.blocks[c.preheader].terminator = Terminator::Branch(mh);
    for inst in &mut func.blocks[c.header].instructions {
        if let Instruction::Phi { dest, incoming, .. } = inst {
            for (op, label) in incoming {
                if *label == pre_label {
                    *label = combine;
                    *op = Operand::Value(if *dest == c.iv { iv } else { result });
                }
            }
        }
    }
    func.blocks.extend([
        BasicBlock {
            label: mh,
            instructions: h_insts,
            terminator: Terminator::CondBranch {
                cond: Operand::Value(cond),
                true_label: mb,
                false_label: combine,
            },
            source_spans: Vec::new(),
        },
        BasicBlock {
            label: mb,
            instructions: b_insts,
            terminator: Terminator::Branch(mh),
            source_spans: Vec::new(),
        },
        BasicBlock {
            label: combine,
            instructions: combine_insts,
            terminator: Terminator::Branch(old_header),
            source_spans: Vec::new(),
        },
    ]);
    func.next_value_id = value_id;
    func.next_label = next_label;
    Some(())
}

/// Pipeline caller owns target/optimization policy and the ordinary pass-disable
/// switch. Tunable lanes via CCC_VOLATILE_REDUCE_LANES (0/2/4/8) and budget
/// via CCC_VOLATILE_REDUCE_BUDGET are read here so A/B harnesses can vary them
/// without recompiling; the core matcher stays pure.
pub(crate) fn run(func: &mut IrFunction) -> usize {
    let lanes = lanes();
    if lanes == 0 {
        return 0;
    }
    if lanes != LANES {
        eprintln!(
            "volatile_reduce: non-default lanes={lanes} requested but only {LANES} is validated; using {LANES}"
        );
    }
    if should_skip_for_compile_time(func) {
        return 0;
    }
    if !func.blocks.iter().any(|b| {
        b.instructions
            .iter()
            .any(|i| matches!(i, Instruction::Load { volatile: true, .. }))
    }) {
        return 0;
    }
    let mut count = 0;
    // Rebuild after each rewrite; original indices remain stable but CFG edges
    // change. Main loops have five phis, remainders have a nonconstant IV init,
    // so neither can be interleaved again (including on a subsequent pass run).
    loop {
        let cfg = CfgAnalysis::build(func);
        let loops = loop_analysis::merge_loops_by_header(loop_analysis::find_natural_loops(
            cfg.num_blocks,
            &cfg.preds,
            &cfg.succs,
            &cfg.idom,
        ));
        let candidate = loops.iter().find_map(|lp| analyze(func, &cfg, lp));
        let Some(c) = candidate else {
            break;
        };
        if rewrite(func, c).is_none() {
            break;
        }
        count += 1;
        if count >= MAX_VOLATILE_LOOPS_BUDGET {
            eprintln!(
                "volatile_reduce: budget {} loops reached in {}, stopping",
                MAX_VOLATILE_LOOPS_BUDGET, func.name
            );
            break;
        }
    }
    count
}

#[cfg(test)]
#[path = "volatile_reduce_tests.rs"]
mod tests;
