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
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::source::Span;
use crate::common::types::IrType;
use crate::ir::analysis::CfgAnalysis;
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Terminator,
    Value,
};

const LANES: usize = 4;
const MAX_BLOCKS_BUDGET: usize = 200;
const MAX_VOLATILE_LOOPS_BUDGET: usize = 32;

// The mask, balanced combine and register-pressure policy are deliberately
// fixed-four, not a generic unrolling knob.
const _: () = assert!(LANES == 4);

struct ReductionSpans {
    iv: Span,
    accumulator: Span,
    compare: Span,
    address: Span,
    load: Span,
    reduction: Span,
    increment: Span,
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
    spans: ReductionSpans,
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
    costly: &CostlyAddresses,
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
    if costly.contains(ptr.0) {
        return None;
    }
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
    let span = |block: &BasicBlock, index| {
        block
            .source_spans
            .get(index)
            .copied()
            .unwrap_or_else(Span::dummy)
    };
    let iv_index =
        usize::from(matches!(h.instructions[1], Instruction::Phi { dest, .. } if dest == iv));
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
        spans: ReductionSpans {
            iv: span(h, iv_index),
            accumulator: span(h, 1 - iv_index),
            compare: span(h, 2),
            address: span(b, 0),
            load: span(b, load_idx),
            reduction: span(b, load_idx + 1),
            increment: span(b, load_idx + 2),
        },
    })
}

fn rewrite(func: &mut IrFunction, c: Candidate) -> Option<()> {
    // Revalidate the instruction variants before any mutation. Carry the full
    // load in Candidate so newly added memory attributes cannot be lost silently.
    if !matches!(c.load, Instruction::Load { volatile: true, .. }) {
        return None;
    }
    let address = match &c.address {
        None => None,
        Some(Instruction::GetElementPtr {
            base,
            offset: offset @ Operand::Const(_),
            ty: IrType::Ptr,
            ..
        }) => Some((*base, *offset)),
        _ => return None,
    };
    // Four control values, two accumulator
    // values per lane, one load and optionally one address per lane, then
    // LANES-1 combine results.
    let required_values = 4 + 3 * LANES + usize::from(address.is_some()) * LANES + (LANES - 1);
    let mut value_id = func.sound_next_value_id();
    value_id.checked_add(required_values as u32)?;
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
    let mut h_spans = vec![c.spans.iv];
    let neutral = identity(c.reduction, c.acc_ty)?;
    let mut b_spans = Vec::with_capacity((2 + usize::from(address.is_some())) * LANES + 1);
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
        h_spans.push(c.spans.accumulator);
        let mut load = c.load.clone();
        if let Some((base, offset)) = address {
            let address = fresh();
            b_insts.push(Instruction::GetElementPtr {
                dest: address,
                base,
                offset,
                ty: IrType::Ptr,
            });
            b_spans.push(c.spans.address);
            let Instruction::Load { ptr, .. } = &mut load else {
                return None;
            };
            *ptr = address;
        }
        let loaded = fresh();
        // Only SSA names change: the address expression, segment, width and
        // volatility are preserved. Emit reads in original execution order.
        let Instruction::Load { dest, .. } = &mut load else {
            return None;
        };
        *dest = loaded;
        b_insts.push(load);
        b_spans.extend([c.spans.load, c.spans.reduction]);
        b_insts.push(Instruction::BinOp {
            dest: updates[lane],
            op: c.reduction,
            lhs: Operand::Value(accs[lane]),
            rhs: Operand::Value(loaded),
            ty: c.acc_ty,
        });
    }
    h_spans.push(c.spans.compare);
    b_spans.push(c.spans.increment);
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
    // Empty input span vectors mean "unknown". Fill only those holes, never
    // erase locations of untouched preheader instructions.
    let preheader = &mut func.blocks[c.preheader];
    preheader
        .source_spans
        .resize(preheader.instructions.len(), Span::dummy());
    preheader.source_spans.push(c.spans.compare);
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
            for (operand, predecessor) in incoming {
                if *predecessor == pre_label {
                    *operand = Operand::Value(if *dest == c.iv { iv } else { result });
                    *predecessor = combine;
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
            source_spans: h_spans,
        },
        BasicBlock {
            label: mb,
            instructions: b_insts,
            terminator: Terminator::Branch(mh),
            source_spans: b_spans,
        },
        BasicBlock {
            label: combine,
            instructions: combine_insts,
            terminator: Terminator::Branch(old_header),
            source_spans: vec![c.spans.reduction; LANES - 1],
        },
    ]);
    func.next_value_id = value_id;
    func.next_label = next_label;
    Some(())
}

/// Declaration attributes override TU policy; full PIC defaults to a dynamic
/// model. Both dynamic models currently issue a resolver for each folded load,
/// so interleaving adds register/setup cost without removing that bottleneck.
pub(crate) fn costly_tls_symbols(
    module: &crate::ir::reexports::IrModule,
    pic: bool,
    model: Option<crate::ir::reexports::TlsModel>,
) -> FxHashSet<String> {
    module
        .globals
        .iter()
        .filter(|g| g.is_thread_local && dynamic_tls(g.tls_model, model, pic))
        .map(|g| g.name.clone())
        .collect()
}

fn dynamic_tls(
    declaration: Option<crate::ir::reexports::TlsModel>,
    command_line: Option<crate::ir::reexports::TlsModel>,
    pic: bool,
) -> bool {
    use crate::ir::reexports::TlsModel;
    matches!(
        declaration.or(command_line).unwrap_or(if pic {
            TlsModel::GlobalDynamic
        } else {
            TlsModel::LocalExec
        }),
        TlsModel::LocalDynamic | TlsModel::GlobalDynamic
    )
}

/// Build once per function, before rewriting. Original pointer definitions do
/// not change. Follow only the same bounded constant-address/Copy chains as the
/// backend's TLS fold; other pointers remain eligible, without guessing aliases.
struct CostlyAddresses {
    roots: FxHashSet<u32>,
    bases: FxHashMap<u32, u32>,
}

impl CostlyAddresses {
    fn new(func: &IrFunction, symbols: &FxHashSet<String>) -> Self {
        let mut result = Self {
            roots: FxHashSet::default(),
            bases: FxHashMap::default(),
        };
        if symbols.is_empty() {
            return result;
        }
        result.roots = func
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .filter_map(|inst| match inst {
                Instruction::GlobalAddr { dest, name } if symbols.contains(name) => Some(dest.0),
                _ => None,
            })
            .collect();
        if result.roots.is_empty() {
            return result;
        }
        for inst in func.blocks.iter().flat_map(|b| &b.instructions) {
            match inst {
                Instruction::GetElementPtr {
                    dest,
                    base,
                    offset: Operand::Const(_),
                    ..
                }
                | Instruction::Copy {
                    dest,
                    src: Operand::Value(base),
                } => {
                    result.bases.insert(dest.0, base.0);
                }
                _ => (),
            }
        }
        result
    }

    fn contains(&self, mut ptr: u32) -> bool {
        for _ in 0..8 {
            if self.roots.contains(&ptr) {
                return true;
            }
            let Some(&base) = self.bases.get(&ptr) else {
                return false;
            };
            ptr = base;
        }
        false
    }
}

/// Pipeline caller resolves environment policy once per translation unit.
/// Keep block/loop budgets explicit; the matcher reads no process-global state.
pub(crate) fn run(
    func: &mut IrFunction,
    costly_symbols: &FxHashSet<String>,
    enforce_block_budget: bool,
) -> usize {
    if enforce_block_budget && func.blocks.len() > MAX_BLOCKS_BUDGET {
        eprintln!(
            "volatile_reduce: skipping {}: {} blocks > {} (budget)",
            func.name,
            func.blocks.len(),
            MAX_BLOCKS_BUDGET
        );
        return 0;
    }
    if !func.blocks.iter().any(|b| {
        b.instructions
            .iter()
            .any(|i| matches!(i, Instruction::Load { volatile: true, .. }))
    }) {
        return 0;
    }
    let costly = CostlyAddresses::new(func, costly_symbols);
    let mut count = 0;
    // Rebuild after each rewrite; original indices remain stable but CFG edges
    // change. Main loops have five phis; remainder IVs no longer start at zero,
    // so neither can be interleaved again (including on a subsequent pass run).
    loop {
        let cfg = CfgAnalysis::build(func);
        let loops = loop_analysis::merge_loops_by_header(loop_analysis::find_natural_loops(
            cfg.num_blocks,
            &cfg.preds,
            &cfg.succs,
            &cfg.idom,
        ));
        let candidate = loops.iter().find_map(|lp| analyze(func, &cfg, lp, &costly));
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
