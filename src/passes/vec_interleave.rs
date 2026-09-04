//! Vector-reduction accumulator interleaving (ILP unroll of latency-bound
//! vector reduction loops).
//!
//! `vectorize.rs` turns `s += a[i] * b[i]` into a single-accumulator vector
//! loop:
//!
//! ```text
//! H:  acc = phi [zero, P], [acc', B];  iv = phi [0, P], [iv', B]
//!     if (iv < limit) goto B else goto E
//! B:  acc' = VecFmaF64x4(acc, a, iv64, b, iv64);  iv' = iv + 32;  goto H
//! ```
//!
//! One `vfmadd231pd` per iteration on ONE `%ymm` chain is latency-bound:
//! Raptor Lake FMA latency is 4 cycles at a throughput of 2/cycle, so the
//! chain runs at 1/8 of peak.  ICX/Clang emit FOUR independent accumulators
//! with displacement-folded loads (`vfmadd231pd 32(%rsi,%rax), %ymm1,
//! %ymm5`); GCC keeps one chain.  Rule 15 of `engineering/agent/RULES.md` —
//! "copy ICX's `vfmadd231pd` YMM accumulators" — is what this pass
//! implements, generically for every reduction shape the vectorizer
//! produces (F64/F32/I32/I64, AVX2 and SSE2 widths, Sum/Dot/Max, one or two
//! accumulator phis).
//!
//! ## Transformation
//!
//! ```text
//! P  -> NP:  hoisted limit math; extra accumulators = identity;
//!            limit_main = limit & ~(IF*step - 1)
//!       MH:  acc_k = phi [init_k, NP], [acc_k', MB]  (k = 0..IF)
//!            ivm   = phi [0, NP], [ivm', MB]
//!            if (ivm < limit_main) goto MB else goto C
//!       MB:  acc_0' = op(acc_0, load(base, ivm64 +  0))
//!            acc_1' = op(acc_1, load(base, ivm64 + 32))   // disp-folded
//!            ...
//!            ivm' = ivm + IF*step; goto MH
//!       C:   comb = acc_0 (+) acc_1 (+) ... ; goto H
//!       H:   acc = phi [comb, C], [acc', B]; iv = phi [ivm, C], [iv', B]
//!            ... original loop, now the vector EPILOGUE (< IF iterations)
//! ```
//!
//! The original loop is kept verbatim as the epilogue, so every downstream
//! consumer (the scalar remainder, `rewire_escaping_iv_uses`, the horizontal
//! reduce in E) keeps reading the same SSA names, and the value of the IV
//! at loop exit is unchanged (`limit_main` is exactly the multiple of
//! `IF*step` the original loop would have stopped at).  `limit_main` is the
//! largest multiple of `IF*step` that is `<= limit`, which makes the main
//! loop exact for ANY limit (including negative/zero: `0 < limit_main` is
//! then false and the epilogue takes over), and needs no divisibility
//! assumption between `limit` and `step`.
//!
//! ## Legality
//!
//! * Integer accumulation is associative modulo 2^n (wrap-around), so the
//!   split is always exact.  FP accumulation is only split when the caller
//!   passes `fp_reassoc` — the same contract under which the vectorizer
//!   created the FP reduction in the first place.
//! * `max` is associative, commutative and idempotent: every extra
//!   accumulator starts from the SAME seed (`max(seed, seed, x...) ==
//!   max(seed, x...)`).  Additive kinds start extra accumulators from the
//!   zero vector.  (Loops whose accumulators mix max with another kind, or
//!   carry more than one max accumulator, are conservatively skipped: the
//!   mixed-seed bookkeeping is not worth the extra state v1.)
//! * Displacement folding `Cast64(iv + c) -> Cast64(iv) + c` is exact
//!   because the IV is a non-negative element/byte counter bounded by
//!   `limit` (which fits the IV type), so neither form can wrap: inside the
//!   main loop `ivm <= limit_main - group`, hence `ivm + k*step < limit`.
//! * The body may contain only pure, side-effect-free instructions (casts,
//!   integer arithmetic, GEPs, vector loads/arithmetic).  A store, call,
//!   scalar load or unknown intrinsic rejects the loop.
//! * The pass is x86-64-only (gated in the pipeline, like
//!   `vec_load_sink`): the AArch64 emitter reads `args[0..2]` of vector
//!   loads and would silently DROP a folded displacement argument.
//!
//! Kill switch: `CCC_NO_VEC_INTERLEAVE=1`.  `CCC_VEC_INTERLEAVE=<n>` forces
//! the factor (2, 4 or 8) for A/B experiments.  Diagnostics under
//! `LCCC_DEBUG_VECTORIZE=1` / `LCCC_WHY_NOT_VECTORIZE=1`.
//!
//! Pass name for CCC_DISABLE_PASSES: "vec_interleave".
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::analysis::CfgAnalysis;
use crate::ir::instruction::{BasicBlock, BlockId, Instruction, Operand, Terminator, Value};
use crate::ir::intrinsics::IntrinsicOp;
use crate::ir::ops::{IrBinOp, IrCmpOp};
use crate::ir::reexports::{IrConst, IrFunction};
use crate::passes::loop_analysis;

/// How an accumulator intrinsic combines its running value with new lanes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AccKind {
    /// Additive: extra accumulators start at the zero vector, combine with add.
    Add,
    /// Signed max: extra accumulators start at the same seed, combine with max.
    Max,
}

/// Description of one interleavable accumulator op.
#[derive(Clone, Copy, Debug)]
struct AccOp {
    kind: AccKind,
    /// Intrinsic that combines two partial accumulators of this type.
    combine: IntrinsicOp,
    /// Intrinsic producing the additive identity (None for Max: seed reuse).
    zero: Option<IntrinsicOp>,
    /// Whether the element type is floating point (needs `fp_reassoc`).
    is_fp: bool,
    /// Preferred number of independent chains for this op's latency class.
    preferred_if: u32,
}

fn acc_op_info(op: IntrinsicOp) -> Option<AccOp> {
    use IntrinsicOp as O;
    // Note: `VecAddI64x4` is deliberately absent — the x86 emitter has no
    // lowering for it (unimplemented no-op), so nothing may ever select it.
    let (kind, combine, zero, is_fp, preferred_if) = match op {
        O::VecFmaF64x4 | O::VecAddF64x4 => {
            (AccKind::Add, O::VecAddF64x4, Some(O::VecZeroF64x4), true, 4)
        }
        O::VecFmaF32x8 | O::VecAddF32x8 => {
            (AccKind::Add, O::VecAddF32x8, Some(O::VecZeroF32x8), true, 4)
        }
        O::VecAddF64x2 => (AccKind::Add, O::VecAddF64x2, Some(O::VecZeroF64x2), true, 4),
        O::VecAddF32x4 => (AccKind::Add, O::VecAddF32x4, Some(O::VecZeroF32x4), true, 4),
        O::VecAddI32x8 => (AccKind::Add, O::VecAddI32x8, Some(O::VecZeroI32x8), false, 4),
        O::VecAddI32x4 => (AccKind::Add, O::VecAddI32x4, Some(O::VecZeroI32x4), false, 4),
        O::VecAddI64x2 | O::VecWidenAddI32x4ToI64x2 => {
            (AccKind::Add, O::VecAddI64x2, Some(O::VecZeroI64x2), false, 4)
        }
        O::VecMaxI32x8 => (AccKind::Max, O::VecMaxI32x8, None, false, 2),
        _ => return None,
    };
    Some(AccOp {
        kind,
        combine,
        zero,
        is_fp,
        preferred_if,
    })
}

/// Which argument slot of an accumulator intrinsic carries the running value.
/// FMA forms fix it at 0; the binary forms accept either side.
fn acc_arg_index(op: IntrinsicOp, args: &[Operand], acc: Value) -> Option<usize> {
    use IntrinsicOp as O;
    let is_acc = |o: &Operand| matches!(o, Operand::Value(v) if *v == acc);
    match op {
        O::VecFmaF64x4 | O::VecFmaF32x8 => (args.len() >= 5 && is_acc(&args[0])).then_some(0),
        O::VecWidenAddI32x4ToI64x2 => (0..args.len()).find(|&i| is_acc(&args[i])),
        _ => {
            if args.len() != 2 {
                return None;
            }
            (0..2).find(|&i| is_acc(&args[i]))
        }
    }
}

/// Pure vector intrinsics that may be duplicated per interleave slice.
fn is_pure_vector_intrinsic(op: IntrinsicOp) -> bool {
    use IntrinsicOp as O;
    matches!(
        op,
        O::VecLoadF64x4
            | O::VecLoadF64x2
            | O::VecLoadF32x8
            | O::VecLoadF32x4
            | O::VecLoadI32x8
            | O::VecLoadI32x4
            | O::VecLoadI64x2
            | O::VecMulF64x4
            | O::VecMulF64x2
            | O::VecMulF32x8
            | O::VecMulF32x4
            | O::VecMulI32x8
            | O::VecMulI32x4
            | O::VecMulI64x2
            | O::VecSubF64x4
            | O::VecSubF64x2
            | O::VecSubF32x8
            | O::VecSubF32x4
            | O::VecBroadcastF64x4
            | O::VecBroadcastF64x2
            | O::VecBroadcastF32x8
            | O::VecBroadcastF32x4
            | O::VecBroadcastI32x8
            | O::VecBroadcastI32x4
            | O::VecBroadcastI64x2
    )
}

/// Vector memory intrinsics whose `(base, offset)` pair accepts a trailing
/// constant displacement in the x86 emitter (`disp(%base,%index)`):
/// `(offset_arg_index, disp_arg_index)` pairs.  Only intrinsics with an
/// actual emitter that reads the displacement may appear here — a folded
/// displacement silently dropped by the backend is a miscompile, not a
/// missed optimization.
fn disp_slots(op: IntrinsicOp) -> Option<&'static [(usize, usize)]> {
    use IntrinsicOp as O;
    const LOAD: &[(usize, usize)] = &[(1, 2)];
    const FMA: &[(usize, usize)] = &[(2, 5), (4, 6)];
    match op {
        O::VecLoadF64x4
        | O::VecLoadF64x2
        | O::VecLoadF32x8
        | O::VecLoadF32x4
        | O::VecLoadI32x8
        | O::VecLoadI32x4
        | O::VecLoadI64x2 => Some(LOAD),
        O::VecFmaF64x4 | O::VecFmaF32x8 => Some(FMA),
        O::VecWidenAddI32x4ToI64x2 => Some(&[(2, 3)]),
        _ => None,
    }
}

fn int_const(ty: IrType, v: i64) -> Option<IrConst> {
    match ty {
        IrType::I32 | IrType::U32 => Some(IrConst::I32(v as i32)),
        IrType::I64 | IrType::U64 => Some(IrConst::I64(v)),
        _ => None,
    }
}

fn why_not() -> bool {
    std::env::var_os("LCCC_DEBUG_VECTORIZE").is_some()
        || std::env::var_os("LCCC_WHY_NOT_VECTORIZE").is_some()
}

/// One accumulator phi of a candidate loop.
struct Acc {
    phi: Value,
    ty: IrType,
    init: Operand,
    /// Index (in the body) of the intrinsic that updates it.
    op_idx: usize,
    info: AccOp,
}

struct Candidate {
    header: usize,
    body: usize,
    preheader: usize,
    iv: Value,
    iv_ty: IrType,
    step: i64,
    iv_inc_idx: usize,
    cmp_op: IrCmpOp,
    limit: Operand,
    accs: Vec<Acc>,
    /// Header instruction indices (non-phi, non-cmp) to hoist into the new
    /// preheader.
    hoist: Vec<usize>,
}

fn operand_mentions(inst: &Instruction, pred: &dyn Fn(u32) -> bool) -> bool {
    let mut hit = false;
    let mut probe = inst.clone();
    probe.for_each_operand_mut(|o| {
        if let Operand::Value(v) = o {
            if pred(v.0) {
                hit = true;
            }
        }
    });
    probe.for_each_value_use_mut(|v| {
        if pred(v.0) {
            hit = true;
        }
    });
    hit
}

fn analyze(
    func: &IrFunction,
    cfg: &CfgAnalysis,
    lp: &loop_analysis::NaturalLoop,
    fp_reassoc: bool,
    debug: bool,
) -> Option<Candidate> {
    let reject = |why: &str| -> Option<Candidate> {
        if debug {
            eprintln!("[VEC-IL] loop@{} not interleaved: {}", lp.header, why);
        }
        None
    };
    if lp.body.len() != 2 || !lp.body.contains(&lp.header) {
        return None; // only the vectorizer's 2-block loops qualify
    }
    let header = lp.header;
    let body = *lp.body.iter().find(|&&b| b != header)?;
    let hb = &func.blocks[header];
    let bb = &func.blocks[body];
    // Shape: H --cond--> {B, E}, B --> H.
    let cond = match &hb.terminator {
        Terminator::CondBranch {
            cond,
            true_label,
            false_label,
        } if *true_label == bb.label && *false_label != hb.label && *false_label != bb.label => {
            cond.clone()
        }
        _ => return reject("header is not `if (iv < limit) body else exit`"),
    };
    if !matches!(&bb.terminator, Terminator::Branch(l) if *l == hb.label) {
        return reject("body does not branch straight back to the header");
    }
    let preds = cfg.preds.row(header);
    if preds.len() != 2 {
        return reject("header does not have exactly one preheader + one latch");
    }
    let preheader = preds.iter().map(|&p| p as usize).find(|&p| p != body)?;
    if !matches!(&func.blocks[preheader].terminator, Terminator::Branch(l) if *l == hb.label) {
        return reject("preheader edge is conditional");
    }
    let pre_label = func.blocks[preheader].label;
    // Exit compare `iv < limit`.
    let Operand::Value(cond_v) = cond else {
        return reject("constant loop condition");
    };
    let cmp_idx = hb
        .instructions
        .iter()
        .position(|i| matches!(i, Instruction::Cmp { dest, .. } if *dest == cond_v))?;
    let (cmp_op, cmp_lhs, limit) = match &hb.instructions[cmp_idx] {
        Instruction::Cmp { op, lhs, rhs, .. } => (*op, lhs.clone(), rhs.clone()),
        _ => unreachable!(),
    };
    if !matches!(cmp_op, IrCmpOp::Slt | IrCmpOp::Ult) {
        return reject("exit compare is not `iv < limit`");
    }
    let Operand::Value(iv) = cmp_lhs else {
        return reject("exit compare lhs is not the IV");
    };
    // Phis: exactly one IV (the compare lhs) + >= 1 accumulators.  Any other
    // loop-carried phi rejects the loop (the transformation rewires every
    // preheader-edge phi incoming, and values it cannot classify must not be
    // silently carried across the main loop).
    let mut iv_info = None;
    let mut accs = Vec::new();
    let defined_in_body: FxHashMap<u32, usize> = bb
        .instructions
        .iter()
        .enumerate()
        .filter_map(|(i, ins)| ins.dest().map(|d| (d.0, i)))
        .collect();
    for inst in &hb.instructions {
        let Instruction::Phi { dest, ty, incoming } = inst else {
            continue;
        };
        if incoming.len() != 2 {
            return reject("phi with != 2 incoming edges");
        }
        let init = incoming
            .iter()
            .find(|(_, l)| *l == pre_label)
            .map(|(o, _)| o.clone());
        let latch = incoming
            .iter()
            .find(|(_, l)| *l == bb.label)
            .map(|(o, _)| o.clone());
        let (Some(init), Some(Operand::Value(latch_v))) = (init, latch) else {
            return reject("phi incoming edges are not {preheader, latch}");
        };
        let Some(&def_idx) = defined_in_body.get(&latch_v.0) else {
            return reject("phi latch value is not defined in the body");
        };
        if *dest == iv {
            if !matches!(init, Operand::Const(c) if c.to_i64() == Some(0)) {
                return reject("IV does not start at 0");
            }
            let step = match &bb.instructions[def_idx] {
                Instruction::BinOp {
                    op: IrBinOp::Add,
                    lhs: Operand::Value(l),
                    rhs: Operand::Const(c),
                    ..
                } if *l == iv => c.to_i64(),
                _ => None,
            };
            let Some(step) = step.filter(|s| *s > 0 && (*s & (*s - 1)) == 0) else {
                return reject("IV step is not a positive power of two");
            };
            if !matches!(ty, IrType::I32 | IrType::I64 | IrType::U32 | IrType::U64) {
                return reject("IV is not a 32/64-bit integer");
            }
            iv_info = Some((*ty, step, def_idx));
        } else {
            let Instruction::Intrinsic { op, args, .. } = &bb.instructions[def_idx] else {
                return reject("accumulator latch value is not a vector intrinsic");
            };
            let Some(info) = acc_op_info(*op) else {
                return reject("accumulator intrinsic is not interleavable");
            };
            if acc_arg_index(*op, args, *dest).is_none() {
                return reject("accumulator phi is not an operand of its update");
            }
            if info.is_fp && !fp_reassoc {
                return reject("FP accumulator without -fassociative-math");
            }
            accs.push(Acc {
                phi: *dest,
                ty: *ty,
                init,
                op_idx: def_idx,
                info,
            });
        }
    }
    let Some((iv_ty, step, iv_inc_idx)) = iv_info else {
        return reject("no IV phi feeding the exit compare");
    };
    if accs.is_empty() {
        return reject("no vector accumulator phi");
    }
    // Conservative: any Max accumulator + more than one accumulator in
    // total.  (Seed reuse makes multiple max chains exact, and the combine
    // trees are built per accumulator, so mixed kinds would also be exact —
    // but the seed/zero bookkeeping per extra accumulator has no test
    // coverage yet; keep the rejection until a real workload needs it.)
    if accs.iter().any(|a| a.info.kind == AccKind::Max) && accs.len() > 1 {
        return reject("max accumulator combined with another accumulator");
    }
    if accs.iter().any(|a| a.op_idx == iv_inc_idx) {
        return reject("IV increment aliases an accumulator update");
    }
    // Header: every other non-phi instruction must be loop-invariant and pure
    // (the vectorizer leaves `limit = (n / w) * bytes` inside the header).
    let loop_defs: FxHashSet<u32> = hb
        .instructions
        .iter()
        .chain(bb.instructions.iter())
        .filter_map(|i| i.dest().map(|d| d.0))
        .collect();
    let mut hoist: Vec<usize> = Vec::new();
    for (idx, inst) in hb.instructions.iter().enumerate() {
        if matches!(inst, Instruction::Phi { .. }) || idx == cmp_idx {
            continue;
        }
        if !matches!(
            inst,
            Instruction::BinOp { .. } | Instruction::Cast { .. } | Instruction::Copy { .. }
        ) {
            return reject("header contains a non-pure non-phi instruction");
        }
        let hoisted_so_far: FxHashSet<u32> = hoist
            .iter()
            .filter_map(|&i| hb.instructions[i].dest().map(|d| d.0))
            .collect();
        if operand_mentions(inst, &|id| loop_defs.contains(&id) && !hoisted_so_far.contains(&id)) {
            return reject("header instruction depends on a loop-carried value");
        }
        hoist.push(idx);
    }
    if let Operand::Value(lv) = &limit {
        let hoisted: FxHashSet<u32> = hoist
            .iter()
            .filter_map(|&i| hb.instructions[i].dest().map(|d| d.0))
            .collect();
        if loop_defs.contains(&lv.0) && !hoisted.contains(&lv.0) {
            return reject("limit is loop-variant");
        }
    }
    // Nothing in the header may read the compare result except the branch.
    if operand_mentions_any(&hb.instructions, cond_v.0) {
        return reject("exit compare result is reused inside the header");
    }
    // Body: every instruction is pure (clonable).
    for (idx, inst) in bb.instructions.iter().enumerate() {
        if idx == iv_inc_idx || accs.iter().any(|a| a.op_idx == idx) {
            continue;
        }
        let pure = match inst {
            Instruction::BinOp { .. }
            | Instruction::Cast { .. }
            | Instruction::Copy { .. }
            | Instruction::GetElementPtr { .. } => true,
            Instruction::Intrinsic { op, dest_ptr, .. } => {
                dest_ptr.is_none() && is_pure_vector_intrinsic(*op)
            }
            _ => false,
        };
        if !pure {
            return reject("body contains a non-clonable instruction");
        }
    }
    // Accumulator phis may be read ONLY by their own update inside the loop.
    for a in &accs {
        let phi_id = a.phi.0;
        for (idx, inst) in bb.instructions.iter().enumerate() {
            if idx != a.op_idx && operand_mentions(inst, &|id| id == phi_id) {
                return reject("accumulator phi has a second in-loop reader");
            }
        }
        for inst in &hb.instructions {
            if !matches!(inst, Instruction::Phi { .. }) && operand_mentions(inst, &|id| id == phi_id)
            {
                return reject("accumulator phi is read by the header");
            }
        }
    }
    // Body definitions must not be used outside the loop (they become the
    // epilogue's private names; the exit reads header phis only).
    let body_defs: FxHashSet<u32> = defined_in_body.keys().copied().collect();
    for (bi, blk) in func.blocks.iter().enumerate() {
        if bi == header || bi == body {
            continue;
        }
        for inst in &blk.instructions {
            if operand_mentions(inst, &|id| body_defs.contains(&id)) {
                return reject("a body value is live outside the loop");
            }
        }
        let mut leak = false;
        let mut term = blk.terminator.clone();
        term.for_each_operand_mut(|o| {
            if matches!(o, Operand::Value(v) if body_defs.contains(&v.0)) {
                leak = true;
            }
        });
        if leak {
            return reject("a body value is live outside the loop");
        }
    }
    Some(Candidate {
        header,
        body,
        preheader,
        iv,
        iv_ty,
        step,
        iv_inc_idx,
        cmp_op,
        limit,
        accs,
        hoist,
    })
}

fn operand_mentions_any(insts: &[Instruction], id: u32) -> bool {
    insts.iter().any(|i| operand_mentions(i, &|x| x == id))
}

/// Interleave factor: forced by `CCC_VEC_INTERLEAVE`, else the latency class
/// of the accumulator ops, halved for two-accumulator loops (register
/// pressure: 2×4 YMM accumulators plus load temporaries would spill).
fn choose_factor(c: &Candidate) -> u32 {
    if let Some(forced) = std::env::var("CCC_VEC_INTERLEAVE")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|f| matches!(f, 2 | 4 | 8))
    {
        return forced;
    }
    let base = c
        .accs
        .iter()
        .map(|a| a.info.preferred_if)
        .min()
        .unwrap_or(2);
    match c.accs.len() {
        1 => base,
        2 => base.min(2),
        _ => 1,
    }
}

fn set_dest(inst: &mut Instruction, new: Value) {
    match inst {
        Instruction::BinOp { dest, .. }
        | Instruction::Cast { dest, .. }
        | Instruction::Copy { dest, .. }
        | Instruction::GetElementPtr { dest, .. } => *dest = new,
        Instruction::Intrinsic { dest, .. } => *dest = Some(new),
        _ => {}
    }
}

fn transform(func: &mut IrFunction, c: &Candidate, factor: u32, debug: bool) -> bool {
    let group = factor as i64 * c.step;
    let (Some(mask_const), Some(zero_const), Some(group_const)) = (
        int_const(c.iv_ty, !(group - 1)),
        int_const(c.iv_ty, 0),
        int_const(c.iv_ty, group),
    ) else {
        return false;
    };
    // Fresh IDs are seeded from the sound bound (max used + 1), never from
    // the cached `next_value_id` hint, which is documented to be 0 ("not yet
    // computed") until lowering/maintenance passes set it.
    let mut next_val = func.max_value_id().saturating_add(1);
    let mut next_label = func.blocks.iter().map(|b| b.label.0).max().unwrap_or(0) + 1;
    let mut fresh = || {
        let v = Value(next_val);
        next_val += 1;
        v
    };
    let h_label = func.blocks[c.header].label;
    let pre_label = func.blocks[c.preheader].label;
    let np_label = BlockId(next_label);
    let mh_label = BlockId(next_label + 1);
    let mb_label = BlockId(next_label + 2);
    let cb_label = BlockId(next_label + 3);
    next_label += 4;
    // ---- NP: hoisted header invariants, identity inits, masked limit -----
    let mut np: Vec<Instruction> = c
        .hoist
        .iter()
        .map(|&idx| func.blocks[c.header].instructions[idx].clone())
        .collect();
    let limit_main = fresh();
    np.push(Instruction::BinOp {
        dest: limit_main,
        op: IrBinOp::And,
        lhs: c.limit.clone(),
        rhs: Operand::Const(mask_const),
        ty: c.iv_ty,
    });
    let mut acc_init: Vec<Vec<Operand>> = Vec::new();
    for a in &c.accs {
        let mut inits = vec![a.init.clone()];
        for _ in 1..factor {
            match (a.info.kind, a.info.zero) {
                (AccKind::Add, Some(zero_op)) => {
                    let z = fresh();
                    np.push(Instruction::Intrinsic {
                        dest: Some(z),
                        op: zero_op,
                        dest_ptr: None,
                        args: vec![],
                    });
                    inits.push(Operand::Value(z));
                }
                _ => inits.push(a.init.clone()),
            }
        }
        acc_init.push(inits);
    }
    // ---- MH: phis + exit compare ---------------------------------------
    let ivm = fresh();
    let ivm_next = fresh();
    let acc_phis: Vec<Vec<Value>> = c
        .accs
        .iter()
        .map(|_| (0..factor).map(|_| fresh()).collect())
        .collect();
    let acc_nexts: Vec<Vec<Value>> = c
        .accs
        .iter()
        .map(|_| (0..factor).map(|_| fresh()).collect())
        .collect();
    let mut mh = Vec::new();
    for (ai, a) in c.accs.iter().enumerate() {
        for k in 0..factor as usize {
            mh.push(Instruction::Phi {
                dest: acc_phis[ai][k],
                ty: a.ty,
                incoming: vec![
                    (acc_init[ai][k].clone(), np_label),
                    (Operand::Value(acc_nexts[ai][k]), mb_label),
                ],
            });
        }
    }
    mh.push(Instruction::Phi {
        dest: ivm,
        ty: c.iv_ty,
        incoming: vec![
            (Operand::Const(zero_const), np_label),
            (Operand::Value(ivm_next), mb_label),
        ],
    });
    let cmp_m = fresh();
    mh.push(Instruction::Cmp {
        dest: cmp_m,
        op: c.cmp_op,
        lhs: Operand::Value(ivm),
        rhs: Operand::Value(limit_main),
        ty: c.iv_ty,
    });
    // ---- MB: IF slices of the body -------------------------------------
    let body_insts = func.blocks[c.body].instructions.clone();
    let mut mb: Vec<Instruction> = Vec::new();
    // Slice 0 materialises the 64-bit cast of the IV; later slices read
    // `cast0 + k*step` so the constant folds into the memory operand
    // displacement of the loads/FMA.
    let mut iv_cast64: Option<Value> = None;
    for k in 0..factor as usize {
        let mut rename: FxHashMap<u32, Value> = FxHashMap::default();
        rename.insert(c.iv.0, ivm);
        for (ai, a) in c.accs.iter().enumerate() {
            rename.insert(a.phi.0, acc_phis[ai][k]);
        }
        // Slice k > 0 sees the IV at `ivm + k*step`; materialised lazily on
        // first use (both as an Operand and as a direct value use).
        let mut iv_k: Option<Value> = None;
        for (idx, inst) in body_insts.iter().enumerate() {
            if idx == c.iv_inc_idx {
                continue;
            }
            if let Instruction::Cast {
                dest,
                src: Operand::Value(sv),
                from_ty,
                to_ty,
            } = inst
            {
                if *sv == c.iv && matches!(to_ty, IrType::I64 | IrType::U64) {
                    let nd = fresh();
                    rename.insert(dest.0, nd);
                    if k == 0 {
                        mb.push(Instruction::Cast {
                            dest: nd,
                            src: Operand::Value(ivm),
                            from_ty: *from_ty,
                            to_ty: *to_ty,
                        });
                        iv_cast64 = Some(nd);
                    } else if let Some(c0) = iv_cast64 {
                        mb.push(Instruction::BinOp {
                            dest: nd,
                            op: IrBinOp::Add,
                            lhs: Operand::Value(c0),
                            rhs: Operand::Const(IrConst::I64(k as i64 * c.step)),
                            ty: *to_ty,
                        });
                    } else {
                        // Unreachable: every slice clones the same body, so a
                        // cast present in slice k > 0 was present in slice 0.
                        return false;
                    }
                    continue;
                }
            }
            let mut ni = inst.clone();
            let mut needs_iv_k = false;
            let mut probe = inst.clone();
            probe.for_each_operand_mut(|o| {
                if matches!(o, Operand::Value(v) if v.0 == c.iv.0) && k > 0 {
                    needs_iv_k = true;
                }
            });
            probe.for_each_value_use_mut(|v| {
                if v.0 == c.iv.0 && k > 0 {
                    needs_iv_k = true;
                }
            });
            if needs_iv_k && iv_k.is_none() {
                let v = fresh();
                mb.push(Instruction::BinOp {
                    dest: v,
                    op: IrBinOp::Add,
                    lhs: Operand::Value(ivm),
                    rhs: Operand::Const(int_const(c.iv_ty, k as i64 * c.step).unwrap()),
                    ty: c.iv_ty,
                });
                iv_k = Some(v);
                // Direct value uses (GEP bases, dest_ptr) must see the
                // shifted IV too, not just Operand uses.
                rename.insert(c.iv.0, v);
            }
            ni.for_each_operand_mut(|o| {
                if let Operand::Value(v) = o {
                    if v.0 == c.iv.0 && k > 0 {
                        *v = iv_k.unwrap();
                    } else if let Some(nv) = rename.get(&v.0) {
                        *v = *nv;
                    }
                }
            });
            ni.for_each_value_use_mut(|v| {
                if let Some(nv) = rename.get(&v.0) {
                    *v = *nv;
                }
            });
            if let Some(d) = inst.dest() {
                let nd = match c.accs.iter().position(|a| a.op_idx == idx) {
                    Some(ai) => acc_nexts[ai][k],
                    None => fresh(),
                };
                rename.insert(d.0, nd);
                set_dest(&mut ni, nd);
            }
            mb.push(ni);
        }
    }
    mb.push(Instruction::BinOp {
        dest: ivm_next,
        op: IrBinOp::Add,
        lhs: Operand::Value(ivm),
        rhs: Operand::Const(group_const),
        ty: c.iv_ty,
    });
    // Displacement folding: `VecLoad(base, add(x, C))` -> `VecLoad(base, x, C)`.
    {
        let adds: FxHashMap<u32, (Value, i64)> = mb
            .iter()
            .filter_map(|i| match i {
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs: Operand::Value(x),
                    rhs: Operand::Const(cst),
                    ty: IrType::I64 | IrType::U64,
                } => cst.to_i64().map(|cv| (dest.0, (*x, cv))),
                _ => None,
            })
            .collect();
        for inst in mb.iter_mut() {
            let Instruction::Intrinsic { op, args, .. } = inst else {
                continue;
            };
            let Some(slots) = disp_slots(*op) else {
                continue;
            };
            for &(off_i, disp_i) in slots {
                let Some(Operand::Value(ov)) = args.get(off_i).cloned() else {
                    continue;
                };
                let Some(&(x, cv)) = adds.get(&ov.0) else {
                    continue;
                };
                if cv.abs() > i32::MAX as i64 {
                    continue;
                }
                while args.len() <= disp_i {
                    args.push(Operand::Const(IrConst::I64(0)));
                }
                let prev = match &args[disp_i] {
                    Operand::Const(cst) => cst.to_i64().unwrap_or(0),
                    _ => continue,
                };
                if (prev + cv).abs() > i32::MAX as i64 {
                    continue;
                }
                args[off_i] = Operand::Value(x);
                args[disp_i] = Operand::Const(IrConst::I64(prev + cv));
            }
        }
    }
    // ---- C: tree-combine the partial accumulators ----------------------
    let mut cb = Vec::new();
    let mut combined: Vec<Value> = Vec::new();
    for (ai, a) in c.accs.iter().enumerate() {
        let mut level: Vec<Value> = acc_phis[ai].clone();
        while level.len() > 1 {
            let mut nxt = Vec::new();
            for pair in level.chunks(2) {
                if pair.len() == 2 {
                    let d = fresh();
                    cb.push(Instruction::Intrinsic {
                        dest: Some(d),
                        op: a.info.combine,
                        dest_ptr: None,
                        args: vec![Operand::Value(pair[0]), Operand::Value(pair[1])],
                    });
                    nxt.push(d);
                } else {
                    nxt.push(pair[0]);
                }
            }
            level = nxt;
        }
        combined.push(level[0]);
    }
    // ---- Rewire ----------------------------------------------------------
    func.blocks[c.preheader].terminator = Terminator::Branch(np_label);
    {
        let hb = &mut func.blocks[c.header];
        let hoist_set: FxHashSet<usize> = c.hoist.iter().copied().collect();
        let old = std::mem::take(&mut hb.instructions);
        let had_spans = old.len() == hb.source_spans.len() && !hb.source_spans.is_empty();
        let old_spans = std::mem::take(&mut hb.source_spans);
        for (idx, inst) in old.into_iter().enumerate() {
            if hoist_set.contains(&idx) {
                continue;
            }
            hb.instructions.push(inst);
            if had_spans {
                hb.source_spans.push(old_spans[idx]);
            }
        }
        for inst in hb.instructions.iter_mut() {
            let Instruction::Phi { dest, incoming, .. } = inst else {
                continue;
            };
            for (val, lbl) in incoming.iter_mut() {
                if *lbl != pre_label {
                    continue;
                }
                *lbl = cb_label;
                if *dest == c.iv {
                    *val = Operand::Value(ivm);
                } else if let Some(ai) = c.accs.iter().position(|a| a.phi == *dest) {
                    *val = Operand::Value(combined[ai]);
                }
            }
        }
    }
    func.blocks.push(BasicBlock {
        label: np_label,
        instructions: np,
        terminator: Terminator::Branch(mh_label),
        source_spans: Vec::new(),
    });
    func.blocks.push(BasicBlock {
        label: mh_label,
        instructions: mh,
        terminator: Terminator::CondBranch {
            cond: Operand::Value(cmp_m),
            true_label: mb_label,
            false_label: cb_label,
        },
        source_spans: Vec::new(),
    });
    func.blocks.push(BasicBlock {
        label: mb_label,
        instructions: mb,
        terminator: Terminator::Branch(mh_label),
        source_spans: Vec::new(),
    });
    func.blocks.push(BasicBlock {
        label: cb_label,
        instructions: cb,
        terminator: Terminator::Branch(h_label),
        source_spans: Vec::new(),
    });
    func.next_value_id = next_val.max(func.next_value_id);
    func.next_label = next_label.max(func.next_label);
    if debug {
        eprintln!(
            "[VEC-IL] {}: loop@{} interleaved x{} ({} accumulator(s), step {})",
            func.name,
            c.header,
            factor,
            c.accs.len(),
            c.step
        );
    }
    true
}

/// Interleave every canonical vector reduction loop in `func`.  Returns the
/// number of loops transformed.
pub(crate) fn run(func: &mut IrFunction, fp_reassoc: bool) -> usize {
    if func.blocks.is_empty() || std::env::var_os("CCC_NO_VEC_INTERLEAVE").is_some() {
        return 0;
    }
    let has_vec_acc = func.blocks.iter().any(|b| {
        b.instructions
            .iter()
            .any(|i| matches!(i, Instruction::Intrinsic { op, .. } if acc_op_info(*op).is_some()))
    });
    if !has_vec_acc {
        return 0;
    }
    let debug = why_not();
    let cfg = CfgAnalysis::build(func);
    let loops =
        loop_analysis::find_natural_loops(func.blocks.len(), &cfg.preds, &cfg.succs, &cfg.idom);
    let mut cands = Vec::new();
    for lp in &loops {
        if let Some(c) = analyze(func, &cfg, lp, fp_reassoc, debug) {
            cands.push(c);
        }
    }
    // Candidates are disjoint 2-block loops; transforming one only appends
    // blocks and rewires its own preheader, so the others stay valid.
    let mut n = 0;
    for c in cands {
        let factor = choose_factor(&c);
        if factor >= 2 && transform(func, &c, factor, debug) {
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the canonical post-vectorizer dot-product loop (4×F64 FMA,
    /// byte IV step 32) as a synthetic IrFunction: 4 blocks (entry, header,
    /// body, exit).  The limit math sits in the header (as the vectorizer
    /// leaves it) so the hoisting path is exercised too.
    fn build_dot_func() -> (IrFunction, Vec<Value>) {
        // Values: 0..7 params/consts are referenced by ids below.
        // 1 = a base ptr, 2 = b base ptr, 3 = n (I32), 4 = zero vec,
        // 5 = acc phi, 6 = iv phi, 7 = limit, 8 = cast64, 9 = fma, 10 = iv+32.
        let mut f = IrFunction::new("dot".to_string(), IrType::F64, vec![], false);
        let e = BlockId(0);
        let h = BlockId(1);
        let b = BlockId(2);
        let x = BlockId(3);
        f.blocks.push(BasicBlock {
            label: e,
            instructions: vec![Instruction::Intrinsic {
                dest: Some(Value(4)),
                op: IntrinsicOp::VecZeroF64x4,
                dest_ptr: None,
                args: vec![],
            }],
            terminator: Terminator::Branch(h),
            source_spans: Vec::new(),
        });
        f.blocks.push(BasicBlock {
            label: h,
            instructions: vec![
                Instruction::Phi {
                    dest: Value(5),
                    ty: IrType::F64,
                    incoming: vec![
                        (Operand::Value(Value(4)), e),
                        (Operand::Value(Value(9)), b),
                    ],
                },
                Instruction::Phi {
                    dest: Value(6),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Const(IrConst::I32(0)), e),
                        (Operand::Value(Value(10)), b),
                    ],
                },
                Instruction::BinOp {
                    dest: Value(7),
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(Value(3)),
                    rhs: Operand::Const(IrConst::I32(32)),
                    ty: IrType::I32,
                },
                Instruction::Cmp {
                    dest: Value(11),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(6)),
                    rhs: Operand::Value(Value(7)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(11)),
                true_label: b,
                false_label: x,
            },
            source_spans: Vec::new(),
        });
        f.blocks.push(BasicBlock {
            label: b,
            instructions: vec![
                Instruction::Cast {
                    dest: Value(8),
                    src: Operand::Value(Value(6)),
                    from_ty: IrType::I32,
                    to_ty: IrType::I64,
                },
                Instruction::Intrinsic {
                    dest: Some(Value(9)),
                    op: IntrinsicOp::VecFmaF64x4,
                    dest_ptr: None,
                    args: vec![
                        Operand::Value(Value(5)),
                        Operand::Value(Value(1)),
                        Operand::Value(Value(8)),
                        Operand::Value(Value(2)),
                        Operand::Value(Value(8)),
                    ],
                },
                Instruction::BinOp {
                    dest: Value(10),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(6)),
                    rhs: Operand::Const(IrConst::I32(32)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Branch(h),
            source_spans: Vec::new(),
        });
        f.blocks.push(BasicBlock {
            label: x,
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        (f, vec![Value(1), Value(2), Value(3)])
    }
    #[test]
    fn acc_op_table_is_consistent() {
        for op in [
            IntrinsicOp::VecFmaF64x4,
            IntrinsicOp::VecAddF32x8,
            IntrinsicOp::VecAddI32x8,
            IntrinsicOp::VecAddI64x2,
            IntrinsicOp::VecWidenAddI32x4ToI64x2,
        ] {
            let info = acc_op_info(op).unwrap();
            assert_eq!(info.kind, AccKind::Add);
            assert!(info.zero.is_some());
        }
        let mx = acc_op_info(IntrinsicOp::VecMaxI32x8).unwrap();
        assert_eq!(mx.kind, AccKind::Max);
        assert!(mx.zero.is_none());
        assert!(acc_op_info(IntrinsicOp::VecMulF64x4).is_none());
        // VecAddI64x4 must stay out: the x86 emitter has no lowering for it.
        assert!(acc_op_info(IntrinsicOp::VecAddI64x4).is_none());
    }

    #[test]
    fn fma_accumulator_slot_is_zero_only() {
        let acc = Value(7);
        let args = vec![
            Operand::Value(acc),
            Operand::Value(Value(1)),
            Operand::Value(Value(2)),
            Operand::Value(Value(3)),
            Operand::Value(Value(2)),
        ];
        assert_eq!(acc_arg_index(IntrinsicOp::VecFmaF64x4, &args, acc), Some(0));
        let swapped = vec![Operand::Value(Value(1)), Operand::Value(acc)];
        assert_eq!(acc_arg_index(IntrinsicOp::VecAddI32x8, &swapped, acc), Some(1));
        assert_eq!(acc_arg_index(IntrinsicOp::VecAddI32x8, &args, acc), None);
    }

    #[test]
    fn group_mask_is_exact_for_all_limits() {
        for group in [4i64, 32, 64, 128] {
            for limit in [-130i64, -1, 0, 1, 31, 32, 33, 127, 128, 129, 1000] {
                let m = limit & !(group - 1);
                assert!(m <= limit);
                assert_eq!(m % group, 0);
                assert!(limit - m < group);
            }
        }
    }

    #[test]
    fn interleave_dot_f64_transforms_and_verifies() {
        let (mut f, _base) = build_dot_func();
        let n = run(&mut f, true);
        assert_eq!(n, 1, "the canonical dot loop must interleave");
        // 4 new blocks: NP, MH, MB, C.
        assert_eq!(f.blocks.len(), 8);
        // The transformed IR must satisfy the structural verifier.
        let mut viol = Vec::new();
        crate::passes::verify::verify_function(&f, "vec_interleave", &mut viol);
        assert!(
            viol.is_empty(),
            "verifier violations after interleave: {:?}",
            viol
        );
        // The original header must now be the epilogue: its preheader edge
        // comes from the combine block, with the IV seeded from `ivm`.
        let hb = &f.blocks[1];
        let iv_phi = hb
            .instructions
            .iter()
            .find_map(|i| match i {
                Instruction::Phi { dest, incoming, .. } if *dest == Value(6) => Some(incoming),
                _ => None,
            })
            .unwrap();
        assert_eq!(iv_phi.len(), 2);
        let (init, lbl) = &iv_phi[0];
        assert_ne!(*lbl, f.blocks[0].label, "preheader edge must be rewired");
        assert!(matches!(init, Operand::Value(_)), "IV init must be ivm");
        // Slice displacement folding: the FMA arguments of slices k > 0 must
        // carry constant displacements k*32 in their new slots.
        let mut fma_disps = Vec::new();
        for blk in f.blocks.iter().skip(4) {
            for inst in &blk.instructions {
                if let Instruction::Intrinsic {
                    op: IntrinsicOp::VecFmaF64x4,
                    args,
                    ..
                } = inst
                {
                    let d5 = match args.get(5) {
                        Some(Operand::Const(c)) => c.to_i64().unwrap_or(-1),
                        _ => -1,
                    };
                    let d6 = match args.get(6) {
                        Some(Operand::Const(c)) => c.to_i64().unwrap_or(-1),
                        _ => -1,
                    };
                    fma_disps.push((d5, d6));
                }
            }
        }
        assert_eq!(
            fma_disps,
            vec![(-1, -1), (32, 32), (64, 64), (96, 96)],
            "slice 0 keeps the 5-arg form; slices 1..3 fold k*step displacements"
        );
    }

    #[test]
    fn fp_interleave_requires_reassoc() {
        let (mut f, _base) = build_dot_func();
        let n = run(&mut f, false);
        assert_eq!(n, 0, "FP interleave without -fassociative-math must bail");
    }

    #[test]
    fn kill_switch_disables_the_pass() {
        let (mut f, _base) = build_dot_func();
        std::env::set_var("CCC_NO_VEC_INTERLEAVE", "1");
        let n = run(&mut f, true);
        std::env::remove_var("CCC_NO_VEC_INTERLEAVE");
        assert_eq!(n, 0);
    }
}
