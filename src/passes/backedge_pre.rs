//! Backedge partial-redundancy elimination: carry a loop-bottom computation
//! of f(next_value) into the loop-top use of f(phi) through a new phi.
//!
//! In a recurrence loop like mandelbrot's
//!   p  = phi(init, v)              // header
//!   e  = fmul p, p                 // top use (iteration difference)
//!   v  = ...                       // next value, computed mid-body
//!   e2 = fmul v, v                 // bottom use (escape/magnitude test)
//!        ... backedge to header
//! e and e2 are the same expression one iteration apart: on iteration n,
//! e == e2 of iteration n-1. Rewriting e's uses to the new phi
//!   q  = phi(f(init) [preheader], e2 [latch])
//! removes e's computation from the loop entirely. GCC often reaches the same
//! shape via loop rotation + CSE.
//!
//! Profitability is fusion-aware, mirroring the backend's fmadd/fmsub/madd/
//! msub rules: the top expression e must be an instruction the emitter would
//! actually materialize (not a multiply absorbed into a fused op), and the
//! bottom expression e2 must not be fusion-eligible either — adding the phi
//! use to a fusion-captured multiply would break the fusion and cost back
//! the saved instruction. (This is exactly why GCC carries zr^2 across the
//! backedge but leaves zi^2 fused into the magnitude fmadd.)
//!
//! Soundness: q is defined in the loop header, which dominates every use of
//! e (e's block is in the loop body). e2 must dominate the latch so its
//! value is always available on the backedge. Only non-trapping BinOps are
//! considered, since f(init) is speculated in the preheader even for
//! zero-trip loops. Current production profitability enables integer PRE by
//! default and keeps FP PRE behind `CCC_BEPRE_FP=1`: x86 Mandelbrot testing
//! showed that a saved FP multiply can be outweighed by the added loop-carried
//! FP phi/copy until register allocation can coalesce that state. Disable with
//! `CCC_DISABLE_PASSES=bepre` or `CCC_NO_BEPRE=1`.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::analysis::CfgAnalysis;
use crate::common::types::IrType;
use crate::ir::reexports::{BlockId, Instruction, IrBinOp, IrConst, IrFunction, Operand, Value};
use super::loop_analysis;

/// A planned rewrite: replace uses of `e_dest` with a new header phi whose
/// latch incoming is `eprime_dest` and whose preheader incoming is a fresh
/// computation of the same expression over the phi's init operands.
struct Rewrite {
    header: usize,
    preheader: usize,
    latch_label: BlockId,
    preheader_label: BlockId,
    e_block: usize,
    eprime_block: usize,
    e_dest: Value,
    eprime_dest: Value,
    op: IrBinOp,
    ty: IrType,
    /// Operands for the preheader computation (phi dest replaced by init).
    init_lhs: Operand,
    init_rhs: Operand,
}

fn operand_key(op: &Operand) -> Option<OperandKey> {
    match op {
        Operand::Value(v) => Some(OperandKey::Val(v.0)),
        Operand::Const(c) => const_key(c).map(OperandKey::Con),
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
enum OperandKey {
    Val(u32),
    Con(ConstKey),
}

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
enum ConstKey {
    I(i128),
    F32(u32),
    F64(u64),
    Zero,
}

fn const_key(c: &IrConst) -> Option<ConstKey> {
    Some(match c {
        IrConst::I8(v) => ConstKey::I(*v as i128),
        IrConst::I16(v) => ConstKey::I(*v as i128),
        IrConst::I32(v) => ConstKey::I(*v as i128),
        IrConst::I64(v) => ConstKey::I(*v as i128),
        IrConst::I128(v) => ConstKey::I(*v),
        IrConst::F32(v) => ConstKey::F32(v.to_bits()),
        IrConst::F64(v) => ConstKey::F64(v.to_bits()),
        IrConst::Zero => ConstKey::Zero,
        // LongDouble carries a lossy f64 plus exact bytes; skip rather than
        // risk equating distinct constants.
        IrConst::LongDouble(..) => return None,
    })
}

fn fold_preheader_binop(op: IrBinOp, lhs: &Operand, rhs: &Operand, ty: IrType) -> Option<IrConst> {
    match (op, lhs, rhs, ty) {
        (IrBinOp::Mul, Operand::Const(IrConst::F64(a)), Operand::Const(IrConst::F64(b)), IrType::F64) => {
            // `0.0 * 0.0` is the hot Mandelbrot PRE init. Folding it avoids
            // adding a useless preheader multiply and, more importantly, avoids
            // lengthening the loop-carried FP live set before the first iteration.
            // Keep NaNs/Infs out of this local fold; the pass is a profitability
            // transform, not a general FP folder.
            if a.is_finite() && b.is_finite() {
                Some(IrConst::F64(a * b))
            } else {
                None
            }
        }
        (IrBinOp::Add, Operand::Const(IrConst::F64(a)), Operand::Const(IrConst::F64(b)), IrType::F64) => {
            if a.is_finite() && b.is_finite() { Some(IrConst::F64(a + b)) } else { None }
        }
        (IrBinOp::Sub, Operand::Const(IrConst::F64(a)), Operand::Const(IrConst::F64(b)), IrType::F64) => {
            if a.is_finite() && b.is_finite() { Some(IrConst::F64(a - b)) } else { None }
        }
        (IrBinOp::Mul, Operand::Const(IrConst::F32(a)), Operand::Const(IrConst::F32(b)), IrType::F32) => {
            if a.is_finite() && b.is_finite() { Some(IrConst::F32(a * b)) } else { None }
        }
        (IrBinOp::Add, Operand::Const(IrConst::F32(a)), Operand::Const(IrConst::F32(b)), IrType::F32) => {
            if a.is_finite() && b.is_finite() { Some(IrConst::F32(a + b)) } else { None }
        }
        (IrBinOp::Sub, Operand::Const(IrConst::F32(a)), Operand::Const(IrConst::F32(b)), IrType::F32) => {
            if a.is_finite() && b.is_finite() { Some(IrConst::F32(a - b)) } else { None }
        }
        (IrBinOp::Mul, Operand::Const(a), Operand::Const(b), ty) if ty.is_integer() => {
            let (Some(av), Some(bv)) = (a.to_i64(), b.to_i64()) else { return None; };
            let v = av.wrapping_mul(bv);
            match ty {
                IrType::I8 | IrType::U8 => Some(IrConst::I8(v as i8)),
                IrType::I16 | IrType::U16 => Some(IrConst::I16(v as i16)),
                IrType::I32 | IrType::U32 => Some(IrConst::I32(v as i32)),
                IrType::I64 | IrType::U64 | IrType::Ptr => Some(IrConst::I64(v)),
                _ => None,
            }
        }
        (IrBinOp::Add, Operand::Const(a), Operand::Const(b), ty) if ty.is_integer() => {
            let (Some(av), Some(bv)) = (a.to_i64(), b.to_i64()) else { return None; };
            let v = av.wrapping_add(bv);
            match ty {
                IrType::I8 | IrType::U8 => Some(IrConst::I8(v as i8)),
                IrType::I16 | IrType::U16 => Some(IrConst::I16(v as i16)),
                IrType::I32 | IrType::U32 => Some(IrConst::I32(v as i32)),
                IrType::I64 | IrType::U64 | IrType::Ptr => Some(IrConst::I64(v)),
                _ => None,
            }
        }
        (IrBinOp::Sub, Operand::Const(a), Operand::Const(b), ty) if ty.is_integer() => {
            let (Some(av), Some(bv)) = (a.to_i64(), b.to_i64()) else { return None; };
            let v = av.wrapping_sub(bv);
            match ty {
                IrType::I8 | IrType::U8 => Some(IrConst::I8(v as i8)),
                IrType::I16 | IrType::U16 => Some(IrConst::I16(v as i16)),
                IrType::I32 | IrType::U32 => Some(IrConst::I32(v as i32)),
                IrType::I64 | IrType::U64 | IrType::Ptr => Some(IrConst::I64(v)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Approximate the backend's multiply-add fusion rules: would the multiply
/// defined at (block, idx) be absorbed into a fused fmadd/fmsub/madd/msub
/// (adjacent or short-gap pattern) instead of being emitted on its own?
/// Conservative both ways — a wrong answer only costs performance, never
/// correctness — so the gap rules are simplified to "only Loads/GEPs
/// between".
fn fusion_eligible(
    func: &IrFunction,
    use_counts: &FxHashMap<u32, u32>,
    def_site: &FxHashMap<u32, (usize, usize)>,
    value: u32,
) -> bool {
    if use_counts.get(&value).copied().unwrap_or(0) != 1 {
        return false;
    }
    let Some(&(b, i)) = def_site.get(&value) else {
        return false;
    };
    let insts = &func.blocks[b].instructions;
    let (mul_ty, is_float) = match &insts[i] {
        Instruction::BinOp { op: IrBinOp::Mul, ty, .. } => (ty, ty.is_float()),
        _ => return false,
    };
    if matches!(mul_ty, IrType::F128 | IrType::I128 | IrType::U128) {
        return false;
    }
    // The consumer must follow within a short gap (Loads/GEPs only between).
    for j in (i + 1)..usize::min(i + 4, insts.len()) {
        match &insts[j] {
            Instruction::Load { .. } | Instruction::GetElementPtr { .. } => continue,
            Instruction::BinOp { op, lhs, rhs, ty, .. }
                if matches!(op, IrBinOp::Add | IrBinOp::Sub) && ty == mul_ty =>
            {
                let lhs_is = matches!(lhs, Operand::Value(v) if v.0 == value);
                let rhs_is = matches!(rhs, Operand::Value(v) if v.0 == value);
                if !lhs_is && !rhs_is {
                    return false;
                }
                return match op {
                    IrBinOp::Add => true,
                    // a - b*c (rhs mul) fuses for int and float; b*c - a
                    // (lhs mul) fuses as fnmsub for floats only.
                    IrBinOp::Sub => rhs_is || is_float,
                    _ => false,
                };
            }
            _ => return false,
        }
    }
    false
}

/// Replace uses of `old` with `new` everywhere: all instruction operand
/// positions (including phi incomings) and terminators.
fn replace_value_everywhere(func: &mut IrFunction, old: u32, new: Value) {
    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            match inst {
                Instruction::Phi { incoming, .. } => {
                    for (op, _) in incoming.iter_mut() {
                        if matches!(op, Operand::Value(v) if v.0 == old) {
                            *op = Operand::Value(new);
                        }
                    }
                }
                _ => {
                    replace_use_in_inst(inst, old, new);
                }
            }
        }
        match &mut block.terminator {
            crate::ir::reexports::Terminator::CondBranch { cond, .. } => {
                if matches!(cond, Operand::Value(v) if v.0 == old) {
                    *cond = Operand::Value(new);
                }
            }
            crate::ir::reexports::Terminator::Return(Some(o)) => {
                if matches!(o, Operand::Value(v) if v.0 == old) {
                    *o = Operand::Value(new);
                }
            }
            crate::ir::reexports::Terminator::Switch { val, .. } => {
                if matches!(val, Operand::Value(v) if v.0 == old) {
                    *val = Operand::Value(new);
                }
            }
            _ => {}
        }
    }
}

fn inst_is_safe_to_cross(inst: &Instruction) -> bool {
    match inst {
        Instruction::BinOp { op, .. } => !op.can_trap(),
        Instruction::UnaryOp { .. }
        | Instruction::Cmp { .. }
        | Instruction::Cast { .. }
        | Instruction::Copy { .. }
        | Instruction::GetElementPtr { .. }
        | Instruction::Select { .. }
        | Instruction::GlobalAddr { .. }
        | Instruction::LabelAddr { .. } => true,
        _ => false,
    }
}

fn schedule_eprime_early(
    func: &mut IrFunction,
    rw: &Rewrite,
    def_site: &FxHashMap<u32, (usize, usize)>,
) {
    let block_idx = rw.eprime_block;
    let Some(block) = func.blocks.get_mut(block_idx) else { return };
    let Some(cur_idx) = block.instructions.iter().position(|inst| {
        matches!(inst, Instruction::BinOp { dest, .. } if dest.0 == rw.eprime_dest.0)
    }) else { return };
    let Instruction::BinOp { lhs, rhs, .. } = &block.instructions[cur_idx] else { return };
    let mut latest_dep: Option<usize> = None;
    for op in [lhs, rhs] {
        if let Operand::Value(v) = op {
            if let Some(&(db, di)) = def_site.get(&v.0) {
                if db == block_idx {
                    latest_dep = Some(latest_dep.map_or(di, |old| old.max(di)));
                }
            }
        }
    }
    let insert_idx = latest_dep.map_or(0, |i| i.saturating_add(1));
    if insert_idx >= cur_idx {
        return;
    }
    if !block.instructions[insert_idx..cur_idx]
        .iter()
        .all(inst_is_safe_to_cross)
    {
        return;
    }
    let inst = block.instructions.remove(cur_idx);
    block.instructions.insert(insert_idx, inst);
    if !block.source_spans.is_empty() {
        block.source_spans.clear();
    }
}

fn replace_use_in_inst(inst: &mut Instruction, old: u32, new: Value) {
    let mut op = |o: &mut Operand| {
        if matches!(o, Operand::Value(v) if v.0 == old) {
            *o = Operand::Value(new);
        }
    };
    let mut val = |v: &mut Value| {
        if v.0 == old {
            *v = new;
        }
    };
    match inst {
        Instruction::BinOp { lhs, rhs, .. } => {
            op(lhs);
            op(rhs);
        }
        Instruction::UnaryOp { src, .. } | Instruction::Cast { src, .. } => op(src),
        Instruction::Copy { src, .. } => op(src),
        Instruction::Cmp { lhs, rhs, .. } => {
            op(lhs);
            op(rhs);
        }
        Instruction::Select { cond, true_val, false_val, .. } => {
            op(cond);
            op(true_val);
            op(false_val);
        }
        Instruction::Store { val: v, ptr, .. } => {
            op(v);
            val(ptr);
        }
        Instruction::Load { ptr, .. } => val(ptr),
        Instruction::GetElementPtr { base, offset, .. } => {
            val(base);
            op(offset);
        }
        Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
            for a in &mut info.args {
                op(a);
            }
        }
        Instruction::SetReturnF64Second { src } | Instruction::SetReturnF32Second { src } => op(src),
        _ => {}
    }
}

pub(crate) fn run(func: &mut IrFunction) -> usize {
    if std::env::var("CCC_NO_BEPRE").is_ok() {
        return 0;
    }
    let cfg = CfgAnalysis::build(func);
    let loops = loop_analysis::merge_loops_by_header(loop_analysis::find_natural_loops(
        func.blocks.len(),
        &cfg.preds,
        &cfg.succs,
        &cfg.idom,
    ));
    if loops.is_empty() {
        return 0;
    }

    let dominates = |a: usize, mut b: usize| -> bool {
        loop {
            if b == a {
                return true;
            }
            if b == cfg.idom[b] || cfg.idom[b] == usize::MAX {
                return false;
            }
            b = cfg.idom[b];
        }
    };

    // Definition sites and use counts over the whole function.
    let mut def_site: FxHashMap<u32, (usize, usize)> = FxHashMap::default();
    let mut multi_def: FxHashSet<u32> = FxHashSet::default();
    let mut use_counts: FxHashMap<u32, u32> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Some(d) = inst.dest() {
                if def_site.insert(d.0, (bi, ii)).is_some() {
                    multi_def.insert(d.0);
                }
            }
            inst.for_each_used_value(|u| *use_counts.entry(u).or_insert(0) += 1);
        }
        block
            .terminator
            .for_each_used_value(|u| *use_counts.entry(u).or_insert(0) += 1);
    }

    // BlockId label -> block index (phi incomings are keyed by label).
    let mut idx_of_label: FxHashMap<BlockId, usize> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        idx_of_label.insert(block.label, bi);
    }

    let mut rewrites: Vec<Rewrite> = Vec::new();

    for lp in &loops {
        let header = lp.header;
        let Some(preheader) = loop_analysis::find_preheader(header, &lp.body, &cfg.preds) else {
            continue;
        };
        let preheader_label = func.blocks[preheader].label;

        // Header phis with exactly two incomings: (init, preheader) and
        // (next_value, latch). Two incomings imply a single latch.
        let phis: Vec<(Value, IrType, Vec<(Operand, BlockId)>)> = func.blocks[header]
            .instructions
            .iter()
            .filter_map(|inst| match inst {
                Instruction::Phi { dest, ty, incoming } => Some((*dest, *ty, incoming.clone())),
                _ => None,
            })
            .collect();

        for (phi_dest, _phi_ty, incoming) in phis {
            if incoming.len() != 2 {
                continue;
            }
            let (init_op, latch_op, latch_label) = match (&incoming[0], &incoming[1]) {
                ((a, from_a), (b, from_b)) if *from_a == preheader_label => (a, b, *from_b),
                ((a, from_a), (b, from_b)) if *from_b == preheader_label => (b, a, *from_a),
                _ => continue,
            };
            let Operand::Value(next_val) = latch_op else { continue };
            let Some(&latch) = idx_of_label.get(&latch_label) else { continue };
            if !lp.body.contains(&latch) || multi_def.contains(&next_val.0) {
                continue;
            }
            // The init operand must be available at the end of the preheader:
            // a constant, or a value defined outside the loop whose def block
            // dominates the preheader.
            let init_ok = match init_op {
                Operand::Const(_) => true,
                Operand::Value(v) => match def_site.get(&v.0) {
                    Some(&(db, _)) => !lp.body.contains(&db) && dominates(db, preheader),
                    None => false,
                },
            };
            if !init_ok {
                continue;
            }

            // Scan loop body for candidate top expressions f(phi, inv).
            for &eb in lp.body.iter() {
                for inst in func.blocks[eb].instructions.iter() {
                    let Instruction::BinOp { dest, op, lhs, rhs, ty } = inst else {
                        continue;
                    };
                    if op.can_trap() {
                        continue; // speculated in the preheader
                    }
                    let e_dest = *dest;
                    // Current x86-64 FP codegen already fuses Mandelbrot-like
                    // square/add chains aggressively. Carrying a singly-used FP
                    // square through an extra header phi removed one multiply but
                    // lengthened the loop-carried dependency and regressed runtime.
                    // Default-enable FP PRE only when the removed top expression
                    // has multiple direct uses (one saved FP op per use chain in
                    // practice); `CCC_BEPRE_FP=1` keeps the broader research path.
                    if ty.is_float()
                        && std::env::var_os("CCC_BEPRE_FP").is_none()
                        && use_counts.get(&e_dest.0).copied().unwrap_or(0) < 2
                    {
                        continue;
                    }
                    if multi_def.contains(&e_dest.0) {
                        continue;
                    }
                    // At least one operand must be the phi value; the other
                    // must be a constant or a value defined outside the loop
                    // whose def dominates the preheader (so the preheader
                    // recomputation can read it).
                    let subst = |o: &Operand| -> Option<(bool, Operand)> {
                        match o {
                            Operand::Value(v) if v.0 == phi_dest.0 => {
                                Some((true, Operand::Value(*next_val)))
                            }
                            Operand::Const(_) => Some((false, o.clone())),
                            Operand::Value(v) => match def_site.get(&v.0) {
                                Some(&(db, _))
                                    if !lp.body.contains(&db) && dominates(db, preheader) =>
                                {
                                    Some((false, o.clone()))
                                }
                                _ => None,
                            },
                        }
                    };
                    let Some((lhs_is_phi, tgt_lhs)) = subst(lhs) else { continue };
                    let Some((rhs_is_phi, tgt_rhs)) = subst(rhs) else { continue };
                    if !lhs_is_phi && !rhs_is_phi {
                        continue;
                    }
                    let (Some(kl), Some(kr)) = (operand_key(&tgt_lhs), operand_key(&tgt_rhs))
                    else {
                        continue;
                    };

                    // Find the bottom expression e2: same op/ty with the
                    // substituted operands, dominating the latch.
                    let commutative = op.is_commutative();
                    let mut eprime: Option<(usize, Value)> = None;
                    for &bb in lp.body.iter() {
                        if !(bb == latch || dominates(bb, latch)) {
                            continue;
                        }
                        for inst2 in &func.blocks[bb].instructions {
                            let Instruction::BinOp {
                                dest: d2,
                                op: op2,
                                lhs: l2,
                                rhs: r2,
                                ty: ty2,
                            } = inst2
                            else {
                                continue;
                            };
                            if op2 != op || ty2 != ty || d2.0 == e_dest.0 {
                                continue;
                            }
                            let (Some(l2k), Some(r2k)) = (operand_key(l2), operand_key(r2))
                            else {
                                continue;
                            };
                            let direct = l2k == kl && r2k == kr;
                            let swapped = commutative && l2k == kr && r2k == kl;
                            if direct || swapped {
                                eprime = Some((bb, *d2));
                                break;
                            }
                        }
                        if eprime.is_some() {
                            break;
                        }
                    }
                    let Some((eprime_block, eprime_dest)) = eprime else { continue };

                    // Fusion-aware profitability: the top expression must be
                    // a really-emitted instruction, and the bottom one must
                    // not lose a fusion by gaining the phi use.
                    if fusion_eligible(func, &use_counts, &def_site, e_dest.0) {
                        continue;
                    }
                    if fusion_eligible(func, &use_counts, &def_site, eprime_dest.0) {
                        continue;
                    }

                    // Preheader operands: phi replaced by the init operand.
                    let init_lhs = if lhs_is_phi { init_op.clone() } else { tgt_lhs.clone() };
                    let init_rhs = if rhs_is_phi { init_op.clone() } else { tgt_rhs.clone() };

                    rewrites.push(Rewrite {
                        header,
                        preheader,
                        latch_label,
                        preheader_label,
                        e_block: eb,
                        eprime_block,
                        e_dest,
                        eprime_dest,
                        op: *op,
                        ty: *ty,
                        init_lhs,
                        init_rhs,
                    });
                }
            }
        }
    }

    // A bottom expression reused as e2 by one rewrite must not be removed as
    // the top expression e of another.
    let removed: FxHashSet<u32> = rewrites.iter().map(|r| r.e_dest.0).collect();
    rewrites.retain(|r| !removed.contains(&r.eprime_dest.0));
    // One rewrite per top expression.
    rewrites.sort_by_key(|r| r.e_dest.0);
    rewrites.dedup_by_key(|r| r.e_dest.0);
    if rewrites.is_empty() {
        return 0;
    }

    let debug = std::env::var("CCC_DEBUG_BEPRE").is_ok();
    let mut applied = 0;
    for rw in &rewrites {
        // Preheader recomputation of f(init). If the init expression folds to
        // a constant, feed that constant directly into the header phi instead
        // of materializing a preheader temp. This is critical for recurrence
        // loops that start from zero (Mandelbrot): the naive PRE inserted a
        // useless `0.0 * 0.0` and increased FP pressure enough to spill an
        // outer-loop constant on x86-64.
        let preheader_incoming = if let Some(c) =
            fold_preheader_binop(rw.op, &rw.init_lhs, &rw.init_rhs, rw.ty)
        {
            Operand::Const(c)
        } else {
            let pv = Value(func.next_value_id);
            func.next_value_id += 1;
            let preh = &mut func.blocks[rw.preheader];
            preh.instructions.push(Instruction::BinOp {
                dest: pv,
                op: rw.op,
                lhs: rw.init_lhs.clone(),
                rhs: rw.init_rhs.clone(),
                ty: rw.ty,
            });
            if !preh.source_spans.is_empty() {
                preh.source_spans.clear();
            }
            Operand::Value(pv)
        };
        let q = Value(func.next_value_id);
        func.next_value_id += 1;

        // Header phi q = phi(f(init) [preheader], e2 [latch]), inserted after the
        // existing phis.
        let hdr = &mut func.blocks[rw.header];
        let phi_pos = hdr
            .instructions
            .iter()
            .position(|i| !matches!(i, Instruction::Phi { .. }))
            .unwrap_or(hdr.instructions.len());
        hdr.instructions.insert(
            phi_pos,
            Instruction::Phi {
                dest: q,
                ty: rw.ty,
                incoming: vec![
                    (preheader_incoming.clone(), rw.preheader_label),
                    (Operand::Value(rw.eprime_dest), rw.latch_label),
                ],
            },
        );
        if !hdr.source_spans.is_empty() {
            hdr.source_spans.clear();
        }

        // Pull the latch/bottom expression up to its earliest safe point after
        // its operands are available. For FP recurrences this shortens the
        // loop-carried PRE value's critical path; without it Mandelbrot saved
        // a multiply but lost to the later backedge copy/phi dependency.
        schedule_eprime_early(func, rw, &def_site);

        // Replace all uses of the top expression, then remove it. Values
        // are single-def (checked above), so locate the def by its dest.
        replace_value_everywhere(func, rw.e_dest.0, q);
        let blk = &mut func.blocks[rw.e_block];
        if let Some(idx) = blk.instructions.iter().position(|i| {
            matches!(i, Instruction::BinOp { dest, .. } if dest.0 == rw.e_dest.0)
        }) {
            blk.instructions.remove(idx);
            if !blk.source_spans.is_empty() {
                blk.source_spans.clear();
            }
        }
        if debug {
            eprintln!(
                "[BEPRE] func={} v{} -> phi({:?}, v{}) in header block {}",
                func.name, rw.e_dest.0, preheader_incoming, rw.eprime_dest.0, rw.header
            );
        }
        applied += 1;
    }
    applied
}
