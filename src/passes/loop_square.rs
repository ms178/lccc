//! Reuse an already-computed square across a loop backedge.
//!
//! A header p = phi(0, next) and body p*p need not recompute next*next
//! when that exact product already dominates the latch. The initial square
//! is +0 (including an initial -0); no arithmetic or memory access is
//! speculated into a zero-trip preheader. One extra recurrence per innermost
//! loop bounds register pressure and avoids defeating both sides of an FMA.
//!
//! Floating-environment-changing and unmodelled instructions reject a loop.
//! This is value reuse, not reassociation or a reconstruction of builtin FMA.
use super::loop_analysis::{self, DominanceChecker};
use crate::common::types::IrType;
use crate::ir::analysis::CfgAnalysis;
use crate::ir::reexports::{BlockId, Instruction, IrBinOp, IrConst, IrFunction, Operand, Value};

fn square_zero(operand: Operand, ty: IrType) -> Option<Operand> {
    match (operand, ty) {
        (Operand::Const(IrConst::F32(x)), IrType::F32) if x == 0.0 => {
            Some(Operand::Const(IrConst::F32(0.0)))
        }
        (Operand::Const(IrConst::F64(x)), IrType::F64) if x == 0.0 => {
            Some(Operand::Const(IrConst::F64(0.0)))
        }
        _ => None,
    }
}

fn stable_fp_environment(inst: &Instruction) -> bool {
    // Default-closed: in particular calls (even pure calls), asm, intrinsics
    // such as LDMXCSR, atomics and nonlocal transfers cannot cross the proof.
    matches!(
        inst,
        Instruction::Phi { .. }
            | Instruction::BinOp { .. }
            | Instruction::UnaryOp { .. }
            | Instruction::Cmp { .. }
            | Instruction::Cast { .. }
            | Instruction::Copy { .. }
            | Instruction::GetElementPtr { .. }
            | Instruction::GlobalAddr { .. }
            | Instruction::Load {
                volatile: false,
                ..
            }
            | Instruction::Store {
                volatile: false,
                ..
            }
    )
}

struct SquarePlan {
    header: usize,
    pre: BlockId,
    latch: BlockId,
    block: usize,
    instruction: usize,
    dest: Value,
    replacement: Value,
    zero: Operand,
    ty: IrType,
}

fn has_fp_square(func: &IrFunction) -> bool {
    func.blocks.iter().flat_map(|b| &b.instructions).any(|i| {
        matches!(i, Instruction::BinOp { op: IrBinOp::Mul, lhs, rhs,
            ty: IrType::F32 | IrType::F64, .. } if lhs == rhs)
    })
}

pub(crate) fn run(func: &mut IrFunction) -> usize {
    if !has_fp_square(func) {
        return 0;
    }
    if func.blocks.is_empty() || !super::gvn::find_multi_def_values(func).is_empty() {
        return 0;
    }
    let cfg = CfgAnalysis::build(func);
    let dom = DominanceChecker::new(func.blocks.len(), &cfg.idom);
    let loops = loop_analysis::find_merged_natural_loops(
        func.blocks.len(),
        &cfg.preds,
        &cfg.succs,
        &cfg.idom,
    );
    // All plans are from one immutable SSA snapshot; distinct innermost
    // loops have disjoint bodies. Applying them changes no CFG edge.
    let mut plans = Vec::new();
    for lp in &loops {
        if loops
            .iter()
            .any(|other| other.header != lp.header && lp.contains(other.header))
        {
            continue;
        }
        let Some(pre) = lp.find_dedicated_preheader(&cfg.preds, &cfg.succs) else {
            continue;
        };
        let Some(latch) = lp.single_latch(&cfg.preds) else {
            continue;
        };
        // Reserve space for temporaries/invariants in the 16-register x86-64
        // SIMD file. This is a conservative profitability heuristic, not a
        // proof that the allocator cannot spill. Add at most one recurrence.
        if func.blocks[lp.header]
            .instructions
            .iter()
            .filter(|i| {
                matches!(
                    i,
                    Instruction::Phi {
                        ty: IrType::F32 | IrType::F64,
                        ..
                    }
                )
            })
            .count()
            >= 6
        {
            continue;
        }
        if lp.body.iter().any(|&b| {
            !dom.dominates(lp.header, b)
                || func.blocks[b]
                    .instructions
                    .iter()
                    .any(|i| !stable_fp_environment(i))
        }) {
            continue;
        }
        let pre_label = func.blocks[pre].label;
        let latch_label = func.blocks[latch].label;
        let mut body: Vec<usize> = lp.body.iter().copied().collect();
        body.sort_unstable();
        'one_per_loop: for &b in &body {
            // Save work on every backedge, not just a conditional side path.
            if !dom.dominates(b, latch) {
                continue;
            }
            for (ii, inst) in func.blocks[b].instructions.iter().enumerate() {
                let Instruction::BinOp {
                    dest,
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(p),
                    rhs: Operand::Value(q),
                    ty,
                } = inst
                else {
                    continue;
                };
                if p != q || !matches!(ty, IrType::F32 | IrType::F64) {
                    continue;
                }
                let Some(Instruction::Phi {
                    incoming,
                    ty: phi_ty,
                    ..
                }) = func.blocks[lp.header]
                    .instructions
                    .iter()
                    .find(|i| matches!(i, Instruction::Phi { dest, .. } if dest == p))
                else {
                    continue;
                };
                if phi_ty != ty || incoming.len() != 2 {
                    continue;
                }
                let Some((initial, _)) = incoming.iter().find(|(_, from)| *from == pre_label)
                else {
                    continue;
                };
                let Some((next, _)) = incoming.iter().find(|(_, from)| *from == latch_label) else {
                    continue;
                };
                let Some(zero) = square_zero(*initial, *ty) else {
                    continue;
                };
                let Operand::Value(next) = next else { continue };
                for &nb in &body {
                    if !dom.dominates(nb, latch) {
                        continue;
                    }
                    let replacement = func.blocks[nb].instructions.iter().find_map(|i| match i {
                        Instruction::BinOp {
                            dest: d,
                            op: IrBinOp::Mul,
                            lhs: Operand::Value(a),
                            rhs: Operand::Value(c),
                            ty: t,
                        } if a == next && c == next && t == ty && d != dest => Some(*d),
                        _ => None,
                    });
                    if let Some(replacement) = replacement {
                        plans.push(SquarePlan {
                            header: lp.header,
                            pre: pre_label,
                            latch: latch_label,
                            block: b,
                            instruction: ii,
                            dest: *dest,
                            replacement,
                            zero,
                            ty: *ty,
                        });
                        break 'one_per_loop;
                    }
                }
            }
        }
    }
    if plans.is_empty() {
        return 0;
    }
    func.next_value_id = func.sound_next_value_id();
    // Rewrite the old instruction in place to a Copy before inserting any
    // phis, so header indices never become stale. Preserve original spans.
    let mut additions = Vec::new();
    for SquarePlan {
        header,
        pre,
        latch,
        block: b,
        instruction: ii,
        dest,
        replacement,
        zero,
        ty,
    } in &plans
    {
        let carried = Value(func.next_value_id);
        func.next_value_id += 1;
        func.blocks[*b].instructions[*ii] = Instruction::Copy {
            dest: *dest,
            src: Operand::Value(carried),
        };
        additions.push((
            *header,
            Instruction::Phi {
                dest: carried,
                ty: *ty,
                incoming: vec![(*zero, *pre), (Operand::Value(*replacement), *latch)],
            },
        ));
    }
    for (header, phi) in additions {
        let b = &mut func.blocks[header];
        let tracked = !b.source_spans.is_empty();
        b.instructions.insert(0, phi);
        if tracked {
            b.source_spans
                .insert(0, crate::common::source::Span::dummy());
        }
    }
    plans.len()
}

#[cfg(test)]
#[path = "loop_square_tests.rs"]
mod tests;
