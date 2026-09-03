//! Loop unrolling pass.
//!
//! Unrolls small inner loops using "unroll with intermediate IV steps and
//! early exits". Replicates the loop body K times per unrolled cycle, with
//! an exit-condition check inserted between each copy. This handles
//! non-multiple-K trip counts without a separate cleanup loop: whichever
//! intermediate check fires first terminates the partial cycle.
//!
//! Example — 4× unrolled loop:
//!
//! ```text
//! header:   %iv = Phi [init, %iv_next]
//!           %cond = Cmp %iv, limit
//!           CondBranch %cond, exit, body_entry
//!
//! [original body blocks]  →  exit_check_1
//!
//! exit_check_1:
//!   %iv_1  = Add %iv, step
//!   %cond_1 = Cmp %iv_1, limit
//!   CondBranch %cond_1, exit, body_copy_2_entry
//!
//! [body_copy_2]  →  exit_check_2
//!   ...
//! exit_check_3  →  [body_copy_4]  →  latch
//!
//! latch:  %iv_next = Add %iv_3, step   ← was Add %iv, step
//!         Branch header
//! ```

use super::loop_analysis;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::analysis::CfgAnalysis;
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Terminator,
    Value,
};

/// Maximum number of body-work blocks (body excluding header and latch) for
/// a loop to be eligible. Prevents excessive code size growth.
const MAX_UNROLL_BODY_BLOCKS: usize = 12; // increased for hot loops via PGO

/// Choose the unroll factor based on total instruction count in body-work blocks.
fn choose_unroll_factor(body_inst_count: usize) -> u32 {
    match body_inst_count {
        // Tiny bodies (e.g. a single FmaF64x4 after vectorize): aggressive
        // unroll exposes independent accumulators without code-size blow-up.
        0..=4 => 8,
        5..=8 => 4,
        9..=20 => 4,
        21..=60 => 2,
        _ => 1, // too large — skip
    }
}

/// All information needed to perform the unrolling transformation.
struct UnrollCandidate {
    /// Block index of the loop header (has the phi + condition check).
    header: usize,
    /// Block index of the single latch (has the IV increment + back-branch).
    latch: usize,
    /// Body blocks, excluding header and latch.
    body_work: Vec<usize>,
    /// Index into `body_work` whose label equals `body_entry`.
    body_entry_work_idx: usize,
    /// Index into `body_work` of the block that branches to the latch.
    pre_latch_work_idx: usize,
    /// Exit block label (outside the loop, target of the header's exit branch).
    exit_target: BlockId,
    /// First in-loop block label (target of the header's continue branch).
    body_entry: BlockId,
    /// The IV phi value defined in the header.
    iv_phi: Value,
    /// Type of the IV.
    iv_ty: IrType,
    /// Constant step added to IV per iteration.
    iv_step: i64,
    /// Comparison operator used in the exit condition.
    exit_cmp_op: IrCmpOp,
    /// Type of the exit comparison instruction.
    exit_cmp_ty: IrType,
    /// The loop-invariant operand of the exit comparison (the "limit").
    exit_limit: Operand,
    /// `true` if the IV is the left-hand operand of the exit Cmp.
    iv_is_lhs: bool,
    /// `true` if cond==true means exit (false means continue).
    exit_cond_positive: bool,
    /// Index of the `Add %iv, step` instruction inside the latch block.
    latch_iv_incr_idx: usize,
    /// Number of times to replicate the loop body (K). Always ≥ 2.
    unroll_factor: u32,
}

/// Run the loop-unrolling pass on one function. Returns the number of loops
/// that were successfully unrolled.
pub(crate) fn unroll_loops(func: &mut IrFunction) -> usize {
    if func.blocks.len() < 2 {
        return 0;
    }
    let cfg = CfgAnalysis::build(func);
    let raw = loop_analysis::find_natural_loops(cfg.num_blocks, &cfg.preds, &cfg.succs, &cfg.idom);
    if raw.is_empty() {
        return 0;
    }
    let loops = loop_analysis::merge_loops_by_header(raw);

    // Set of all loop-header block indices (used for nested-loop detection).
    let all_headers: FxHashSet<usize> = loops.iter().map(|l| l.header).collect();

    let mut count = 0;

    // Pass A: complete-unroll constant-trip loops. Two shapes:
    //   1. the 2–3 block linear form (flattened into one straight-line
    //      block per iteration), and
    //   2. the GENERAL multi-block form — including bodies that contain
    //      inner loops — which clones the body structure wholesale. The
    //      general form lets an outer loop unroll first (its body contains
    //      the inner loop), after which the inner loops' triangular IV
    //      inits (`j = i+1`) are per-clone constant expressions that
    //      `resolve_const_operand` evaluates, so the fixpoint cascades
    //      outer→inner within one call.
    // Rebuild CFG after each success so block indices stay valid.
    loop {
        let cfg = CfgAnalysis::build(func);
        let raw =
            loop_analysis::find_natural_loops(cfg.num_blocks, &cfg.preds, &cfg.succs, &cfg.idom);
        let loops_now = loop_analysis::merge_loops_by_header(raw);
        let mut did = false;
        let mut tiny: Vec<_> = loops_now
            .iter()
            .filter(|lp| matches!(lp.body.len(), 2 | 3))
            .cloned()
            .collect();
        tiny.sort_by_key(|lp| lp.header);
        for lp in &tiny {
            if try_complete_unroll_two_block(func, lp, &cfg) {
                count += 1;
                did = true;
                break;
            }
            // The linear flattener needs trip ≥ 2; a single-trip tiny loop
            // (the last row of a triangular nest once the outer loop is
            // straightened, or `for (i = n-1; i < n; i++)` after inlining)
            // is deleted by the general cloner instead of surviving as a
            // rolled loop with a compare, two phis and a back-edge.
            if try_complete_unroll_general(func, lp, &cfg, 1..=1) {
                count += 1;
                did = true;
                break;
            }
        }
        if !did {
            // General shape: prefer SMALL bodies (innermost-first), but any
            // constant-trip loop is fair game — failures (non-const trip,
            // budget, shape) just fall through to the next candidate.
            let mut general: Vec<_> = loops_now
                .iter()
                .filter(|lp| lp.body.len() > 3 && lp.body.len() <= 33)
                .cloned()
                .collect();
            general.sort_by_key(|lp: &loop_analysis::NaturalLoop| {
                lp.body
                    .iter()
                    .map(|&bi| func.blocks[bi].instructions.len())
                    .sum::<usize>()
            });
            for lp in &general {
                if try_complete_unroll_general(func, lp, &cfg, 1..=16) {
                    count += 1;
                    did = true;
                    break;
                }
            }
        }
        if !did || count > 96 {
            break;
        }
    }
    if count > 0 {
        return count;
    }

    // Collect and sort candidates by body size (smallest first = innermost first).
    let mut candidates: Vec<UnrollCandidate> = loops
        .iter()
        .filter_map(|lp| analyze_loop(func, lp, &cfg, &all_headers))
        .collect();
    candidates.sort_by_key(|c| c.body_work.len());
    let pgo_profile = crate::pgo::get_pgo_profile();
    for c in candidates {
        // PGO gating
        if let Some(profile) = pgo_profile {
            if let Some(should) = crate::pgo::unroll_pgo::should_unroll_loop(
                func,
                c.header,
                c.body_work.len(),
                Some(profile),
            ) {
                if !should {
                    continue;
                }
            }
        }
        if do_unroll(func, c) {
            count += 1;
        }
    }
    count
}

// ── Eligibility analysis ──────────────────────────────────────────────────────

fn analyze_loop(
    func: &IrFunction,
    lp: &loop_analysis::NaturalLoop,
    cfg: &CfgAnalysis,
    all_headers: &FxHashSet<usize>,
) -> Option<UnrollCandidate> {
    let header = lp.header;

    // 1. Size check: body (header + latch + work blocks) must be small.
    if lp.body.len() > MAX_UNROLL_BODY_BLOCKS + 2 {
        return None;
    }

    // 2. Single latch: exactly one block in body has a back-edge to header.
    let back_preds: Vec<usize> = cfg
        .preds
        .row(header)
        .iter()
        .map(|&p| p as usize)
        .filter(|p| lp.body.contains(p))
        .collect();
    if back_preds.len() != 1 {
        return None;
    }
    let latch = back_preds[0];

    // Latch must terminate with an unconditional Branch back to the header.
    let header_label = func.blocks[header].label;
    match &func.blocks[latch].terminator {
        Terminator::Branch(lbl) if *lbl == header_label => {}
        _ => return None,
    }

    // 3. A unique preheader must exist.
    loop_analysis::find_preheader(header, &lp.body, &cfg.preds)?;

    // 4. body_work = body \ {header, latch}; must be non-empty.
    let body_work: Vec<usize> = lp
        .body
        .iter()
        .copied()
        .filter(|&b| b != header && b != latch)
        .collect();
    if body_work.is_empty() {
        return None;
    }

    // 5. No nested loops: body_work blocks must not be headers of other loops.
    for &b in &body_work {
        if all_headers.contains(&b) {
            return None;
        }
    }

    // Cloning currently re-plumbs only the header exit. Reject body exits
    // until exit-phi remapping is implemented.
    {
        let label_to_idx: FxHashMap<BlockId, usize> = func
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.label, i))
            .collect();
        for &bi in &body_work {
            let succs: Vec<BlockId> = match &func.blocks[bi].terminator {
                Terminator::Branch(l) => vec![*l],
                Terminator::CondBranch {
                    true_label,
                    false_label,
                    ..
                } => vec![*true_label, *false_label],
                Terminator::Switch { default, cases, .. } => {
                    let mut v = vec![*default];
                    v.extend(cases.iter().map(|(_, l)| *l));
                    v
                }
                _ => vec![],
            };
            for s in succs {
                let in_loop = label_to_idx
                    .get(&s)
                    .map(|&idx| lp.body.contains(&idx))
                    .unwrap_or(false);
                if !in_loop {
                    return None;
                }
            }
        }
    }

    // 6. No disqualifying instructions in body_work.
    for &bi in &body_work {
        for inst in &func.blocks[bi].instructions {
            match inst {
                Instruction::Call { .. }
                | Instruction::CallIndirect { .. }
                | Instruction::InlineAsm { .. }
                | Instruction::AtomicRmw { .. }
                | Instruction::AtomicCmpxchg { .. }
                | Instruction::AtomicLoad { .. }
                | Instruction::AtomicStore { .. }
                | Instruction::DynAlloca { .. } => return None,
                _ => {}
            }
        }
    }

    // 7. Find basic IV: a phi in the header whose back-edge value is
    //    Add(%iv, const_step) in the latch.
    let latch_label = func.blocks[latch].label;
    let (iv_phi, iv_ty, iv_step, latch_iv_incr_idx) =
        find_iv_in_loop(func, header, latch, latch_label)?;

    // 8. Detect the exit condition from the header's CondBranch.
    let (
        exit_target,
        body_entry,
        exit_cmp_op,
        exit_cmp_ty,
        exit_limit,
        iv_is_lhs,
        exit_cond_positive,
    ) = find_exit_condition(func, header, &lp.body, iv_phi)?;

    // 9. Count body instructions and select the unroll factor.
    let body_inst_count: usize = body_work
        .iter()
        .map(|&bi| func.blocks[bi].instructions.len())
        .sum();
    let unroll_factor = choose_unroll_factor(body_inst_count);
    if unroll_factor <= 1 {
        return None;
    }

    // 10. Find body_entry_work_idx and ensure a unique pre-latch block.
    let body_entry_work_idx = body_work
        .iter()
        .position(|&bi| func.blocks[bi].label == body_entry)?;

    let mut pre_latch_work_idx: Option<usize> = None;
    for (j, &bi) in body_work.iter().enumerate() {
        if block_has_succ(&func.blocks[bi].terminator, latch_label) {
            if pre_latch_work_idx.is_some() {
                return None; // multiple blocks branch to latch — too complex
            }
            pre_latch_work_idx = Some(j);
        }
    }
    let pre_latch_work_idx = pre_latch_work_idx?;

    // 11. Exit-block phi eligibility: all incoming-from-header values must be
    //     loop-invariant (not defined in body_work), so each new exit edge can
    //     carry the same value without creating new definitions.
    if let Some(exit_bi) = func.blocks.iter().position(|b| b.label == exit_target) {
        for inst in &func.blocks[exit_bi].instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                for (op, src_label) in incoming {
                    if *src_label == header_label {
                        if let Operand::Value(v) = op {
                            if is_defined_in_body(v.0, &lp.body, func) {
                                return None;
                            }
                        }
                    }
                }
            }
        }
    }

    // Skip unrolling for I32/U32 IV types on 64-bit targets when the loop body
    // contains Cast(I32→I64) or GEP instructions that widen the IV. The unroller
    // creates intermediate IV values at the narrow type, and in complex functions
    // (like SQLite's 255K-line amalgamation) the widened values can interact
    // incorrectly with subsequent optimization passes.
    // Simple loops without IV widening (pure I32 arithmetic) are safe to unroll.
    if !crate::common::types::target_is_32bit() && iv_ty.size() < 8 && iv_ty.is_integer() {
        let has_iv_widening = body_work.iter().any(|&bi| {
            func.blocks[bi].instructions.iter().any(|inst| {
                match inst {
                    Instruction::Cast {
                        src: Operand::Value(v),
                        from_ty,
                        to_ty,
                        ..
                    } => {
                        v.0 == iv_phi.0
                            && matches!(from_ty, IrType::I32 | IrType::U32)
                            && matches!(to_ty, IrType::I64 | IrType::U64 | IrType::Ptr)
                    }
                    // Direct GEP uses of the IV are cloned through the
                    // per-clone value map in do_unroll, so they are legal. The
                    // historical blanket rejection made the core counted-array
                    // loop test permanently unreachable.
                    _ => false,
                }
            })
        });
        let small_const_trip = match &exit_limit {
            Operand::Const(IrConst::I32(n)) => *n > 0 && (*n as i64) <= 8,
            Operand::Const(IrConst::I64(n)) => *n > 0 && *n <= 8,
            _ => false,
        };
        if has_iv_widening && !small_const_trip {
            return None;
        }
    }

    Some(UnrollCandidate {
        header,
        latch,
        body_work,
        body_entry_work_idx,
        pre_latch_work_idx,
        exit_target,
        body_entry,
        iv_phi,
        iv_ty,
        iv_step,
        exit_cmp_op,
        exit_cmp_ty,
        exit_limit,
        iv_is_lhs,
        exit_cond_positive,
        latch_iv_incr_idx,
        unroll_factor,
    })
}

/// Find a basic induction variable in the loop header and its increment in
/// the latch. Returns `(phi_dest, ty, step, latch_incr_idx)`.

/// Complete-unroll a 2-block loop (header + latch) with a small constant trip
/// count. Chains loop-carried phis across linearized clones and substitutes the
/// IV with `init + k*step` (not always `0..trip`).

fn coalesce_linear_chain(func: &mut IrFunction, labels: &[BlockId]) {
    if labels.len() < 2 {
        return;
    }
    let indices: Vec<Option<usize>> = labels
        .iter()
        .map(|l| func.blocks.iter().position(|b| b.label == *l))
        .collect();
    if indices.iter().any(|i| i.is_none()) {
        return;
    }
    let indices: Vec<usize> = indices.into_iter().map(|i| i.unwrap()).collect();
    for i in 0..indices.len() - 1 {
        match &func.blocks[indices[i]].terminator {
            Terminator::Branch(tgt) if *tgt == labels[i + 1] => {}
            _ => return,
        }
    }
    let last_term = func.blocks[indices[indices.len() - 1]].terminator.clone();
    let mut merged = std::mem::take(&mut func.blocks[indices[0]].instructions);
    for &bi in &indices[1..] {
        let insts = std::mem::take(&mut func.blocks[bi].instructions);
        merged.extend(insts);
        func.blocks[bi].terminator = Terminator::Branch(labels[0]);
        func.blocks[bi].instructions.clear();
    }
    func.blocks[indices[0]].instructions = merged;
    func.blocks[indices[0]].terminator = last_term;
}

fn latch_is_bit_iteration(func: &IrFunction, latch: usize) -> bool {
    let mut saw_shift = false;
    let mut saw_and = false;
    let mut other_arith = false;
    for inst in &func.blocks[latch].instructions {
        match inst {
            Instruction::BinOp { op, .. } => {
                use crate::ir::reexports::IrBinOp::*;
                match op {
                    AShr | LShr | Shl | BitTest => saw_shift = true,
                    And | Or | Xor => saw_and = true,
                    Add | Sub | Mul | SDiv | UDiv | SRem | URem => other_arith = true,
                }
            }
            Instruction::Store { .. }
            | Instruction::Load { .. }
            | Instruction::GetElementPtr { .. } => {
                other_arith = true;
            }
            _ => {}
        }
    }
    saw_shift && saw_and && !other_arith
}

/// Complete-unroll a constant-trip loop of ANY block shape, including bodies
/// that contain inner loops (their headers/latches are cloned wholesale; the
/// inner IV phis' initial values — typically `i+1` of the outer IV — become
/// per-clone constants that the next fixpoint round can itself complete-unroll
/// once the outer substitution makes them constant-foldable).
///
/// Structural contract:
/// - single latch (unconditional Branch to header); the ONLY loop exit is the
///   header's CondBranch; every body block's successors stay inside
///   body ∪ {header}; body terminators are Branch/CondBranch/Switch only.
/// - constant IV init (after const-chain resolution) and constant limit,
///   trip inside `trip_range` (callers pass `1..=16`; a trip of exactly 1
///   deletes the loop — header phis become copies of their init values, the
///   latch falls through to the exit — with zero code growth, which is what
///   the last row of a triangular nest `for (j = i+1; j < n; …)` becomes
///   after the outer loop is straightened), and (header non-phi + body)
///   instructions × trip within the expansion budget.
/// - no calls / inline asm / dynamic allocas in body or header; intrinsics
///   only when pure (sqrt/FMA-class loop bodies stay eligible — the nbody
///   `advance`/`energy` shapes).
///
/// Transform (compare [`try_complete_unroll_two_block`], which flattens a
/// linear 2–3 block body into one straight-line block per iteration):
/// - iteration 0 reuses the ORIGINAL header (its phis receive the init values
///   from the preheader) and the ORIGINAL body blocks; the header terminator
///   becomes an unconditional Branch to the body entry (trip ≥ 2 ⇒ the first
///   iteration always executes).
/// - iterations 1..trip-1 get a full clone of [header non-phi instructions] +
///   [all body blocks] with per-clone fresh values/labels, the IV phi
///   substituted by the per-iteration constant, and the loop-carried phis
///   substituted by the previous iteration's values. Internal branches are
///   remapped through the clone's label map; the outer back-edge (latch →
///   header) is redirected to the NEXT clone's entry (or the exit target for
///   the last clone). Inner-loop phis' incoming labels that referenced the
///   outer header are relabeled to the clone's header-copy label — the
///   actual predecessor inside the clone.
/// - outside uses of the IV phi are replaced with the final constant; outside
///   uses of carried phis with the last clone's corresponding value; the exit
///   block's phi edges from the header label are relabeled to the last
///   clone's latch (the block that now branches to the exit).
/// Fixed-width integer types whose constants the unroller can reason about
/// exactly.  `Ptr`, the 128-bit types and non-integers are excluded: their
/// canonical `IrConst` representation is target- or width-dependent and the
/// closed-form trip arithmetic below would silently lose bits.
fn is_fixed_width_int(ty: IrType) -> bool {
    matches!(
        ty,
        IrType::I8
            | IrType::U8
            | IrType::I16
            | IrType::U16
            | IrType::I32
            | IrType::U32
            | IrType::I64
            | IrType::U64
    )
}

/// Bring an i64 into the canonical `IrConst::to_i64()` representation of a
/// value of type `ty` — sign-extended for signed types, zero-extended for
/// unsigned ones.  This is exactly the convention the frontend and constant
/// folder use (`IrConst::from_i64(v, ty).to_i64()`), so every value the
/// unroller derives by folding is bit-for-bit what the rest of the pipeline
/// would have produced.  Any other narrowing (e.g. `x as i32 as i64` for a
/// U32) turns `0xFFFFFFF9` into `-7`, and a trip count computed from `-7`
/// unrolls a loop that never executes.
fn normalize_to_type(v: i64, ty: IrType) -> Option<i64> {
    if !is_fixed_width_int(ty) {
        return None;
    }
    IrConst::from_i64(v, ty).to_i64()
}

/// Ordering comparisons that read their operands as unsigned.
fn cmp_is_unsigned(op: IrCmpOp) -> bool {
    matches!(op, IrCmpOp::Ult | IrCmpOp::Ule | IrCmpOp::Ugt | IrCmpOp::Uge)
}

/// `a OP b`  ⇔  `b MIRROR(OP) a`.
fn mirror_cmp(op: IrCmpOp) -> IrCmpOp {
    match op {
        IrCmpOp::Slt => IrCmpOp::Sgt,
        IrCmpOp::Sgt => IrCmpOp::Slt,
        IrCmpOp::Sle => IrCmpOp::Sge,
        IrCmpOp::Sge => IrCmpOp::Sle,
        IrCmpOp::Ult => IrCmpOp::Ugt,
        IrCmpOp::Ugt => IrCmpOp::Ult,
        IrCmpOp::Ule => IrCmpOp::Uge,
        IrCmpOp::Uge => IrCmpOp::Ule,
        IrCmpOp::Eq => IrCmpOp::Eq,
        IrCmpOp::Ne => IrCmpOp::Ne,
    }
}

/// `!(a OP b)`  ⇔  `a NEGATE(OP) b`  (total order on integers).
fn negate_cmp(op: IrCmpOp) -> IrCmpOp {
    match op {
        IrCmpOp::Slt => IrCmpOp::Sge,
        IrCmpOp::Sge => IrCmpOp::Slt,
        IrCmpOp::Sle => IrCmpOp::Sgt,
        IrCmpOp::Sgt => IrCmpOp::Sle,
        IrCmpOp::Ult => IrCmpOp::Uge,
        IrCmpOp::Uge => IrCmpOp::Ult,
        IrCmpOp::Ule => IrCmpOp::Ugt,
        IrCmpOp::Ugt => IrCmpOp::Ule,
        IrCmpOp::Eq => IrCmpOp::Ne,
        IrCmpOp::Ne => IrCmpOp::Eq,
    }
}

/// Rewrite the header's exit compare into the canonical "continue while
/// `iv OP limit`" form that the trip-count arithmetic assumes.
///
/// [`find_exit_condition`] reports the compare exactly as written: the IV may
/// be the RIGHT operand (`limit > i`) and the CondBranch may send `true` to
/// the EXIT (`for (;;) { if (i >= n) break; … }` or `!(i >= n)` guards).
/// Feeding the raw operator into the closed form in either case computes the
/// trip count of a different loop.  Both call sites of
/// [`complete_unroll_trip`] go through this normalisation.
fn canonical_continue_cmp(op: IrCmpOp, iv_is_lhs: bool, exit_cond_positive: bool) -> IrCmpOp {
    let op = if iv_is_lhs { op } else { mirror_cmp(op) };
    if exit_cond_positive {
        negate_cmp(op)
    } else {
        op
    }
}

/// Exact static trip count of a constant-stride counted loop
/// `for (iv = init; iv OP limit; iv += step)` whose IV has type `iv_ty`, or
/// `None` whenever the count cannot be established EXACTLY.  Complete
/// unrolling has no cleanup loop, so an off-by-anything here is a miscompile,
/// and every arm is closed-form and checked:
///
///   * `iv <  limit`, step > 0: ceil((limit - init) / step)
///   * `iv <= limit`, step > 0: floor((limit - init) / step) + 1
///   * `iv >  limit`, step < 0: ceil((init - limit) / -step)
///   * `iv >= limit`, step < 0: floor((init - limit) / -step) + 1
///   * `iv != limit`          : (limit - init) / step when step divides the
///                              span exactly and the walk moves TOWARDS the
///                              limit; anything else wraps around the type
///                              first and is refused.
///
/// The arithmetic is done in i128 in the comparison's own value domain:
/// `init` and `limit` are first normalised to `iv_ty`'s canonical constant
/// representation and then read as unsigned (for `Ult`/`Ule`/`Ugt`/`Uge`) or
/// signed values.  A U64 constant such as `0xFFFF_FFFF_FFFF_FFF9` is stored as
/// a negative i64 by `IrConst`; read as signed it is "less than 4" and a
/// signed-only closed form unrolls ten iterations of a loop that never runs.
///
/// Finally every IV value the unrolled iterations will observe —
/// `init + k*step` for `k = 0..=trip`, including the value the last exit test
/// sees — must be representable in `iv_ty` without wrapping.  Narrow IVs
/// with strides > 1 (`unsigned char i = 250; i < 255; i += 3`) wrap past the
/// limit and keep looping; the closed form would report 2 trips for a loop
/// that executes 87.  A wrap is refused, not modelled: such loops are never
/// worth unrolling.
///
/// Mismatched signedness between the operator and the IV type is refused
/// outright (the frontend never emits it; anything that does is not a shape
/// this pass has evidence for).
fn complete_unroll_trip(
    iv_init: i64,
    limit: i64,
    cmp_op: IrCmpOp,
    iv_step: i64,
    iv_ty: IrType,
) -> Option<i64> {
    if iv_step == 0 {
        return None; // non-advancing IV: infinite or empty; not a trip count
    }
    let init = normalize_to_type(iv_init, iv_ty)?;
    let limit = normalize_to_type(limit, iv_ty)?;

    // Value domain of the comparison.
    let unsigned = match cmp_op {
        IrCmpOp::Eq | IrCmpOp::Ne => iv_ty.is_unsigned(),
        op => {
            let u = cmp_is_unsigned(op);
            if u != iv_ty.is_unsigned() {
                return None;
            }
            u
        }
    };
    let to_domain = |v: i64| -> i128 {
        if unsigned {
            (v as u64) as i128
        } else {
            v as i128
        }
    };
    let (init_d, limit_d) = (to_domain(init), to_domain(limit));
    let step = iv_step as i128;
    let ascending = step > 0;
    let step_abs = step.abs();

    let trip: i128 = match cmp_op {
        IrCmpOp::Slt | IrCmpOp::Ult => {
            if !ascending {
                return None;
            }
            let span = limit_d - init_d;
            if span <= 0 {
                return None;
            }
            (span - 1) / step_abs + 1 // ceil(span / step)
        }
        IrCmpOp::Sle | IrCmpOp::Ule => {
            if !ascending {
                return None;
            }
            let span = limit_d - init_d;
            if span < 0 {
                return None;
            }
            span / step_abs + 1
        }
        IrCmpOp::Sgt | IrCmpOp::Ugt => {
            if ascending {
                return None;
            }
            let span = init_d - limit_d;
            if span <= 0 {
                return None;
            }
            (span - 1) / step_abs + 1
        }
        IrCmpOp::Sge | IrCmpOp::Uge => {
            if ascending {
                return None;
            }
            let span = init_d - limit_d;
            if span < 0 {
                return None;
            }
            span / step_abs + 1
        }
        IrCmpOp::Ne => {
            let span = limit_d - init_d;
            if span == 0 || (span > 0) != ascending || span % step != 0 {
                return None;
            }
            span / step
        }
        IrCmpOp::Eq => return None,
    };

    // Every observed IV value — including the one the final exit test sees —
    // must stay inside `iv_ty` in this domain.
    let bits = (iv_ty.size() * 8) as u32;
    let (lo, hi): (i128, i128) = if unsigned {
        (0, (1i128 << bits) - 1)
    } else {
        (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
    };
    let final_iv = init_d + trip * step;
    if final_iv < lo || final_iv > hi {
        return None;
    }
    i64::try_from(trip).ok()
}

fn try_complete_unroll_general(
    func: &mut IrFunction,
    lp: &loop_analysis::NaturalLoop,
    cfg: &CfgAnalysis,
    trip_range: std::ops::RangeInclusive<i64>,
) -> bool {
    let header = lp.header;
    let header_label = func.blocks[header].label;

    // Single latch whose terminator is an unconditional Branch to the header.
    let back_preds: Vec<usize> = cfg
        .preds
        .row(header)
        .iter()
        .map(|&p| p as usize)
        .filter(|p| lp.body.contains(p))
        .collect();
    if back_preds.len() != 1 {
        return false;
    }
    let latch = back_preds[0];
    if latch == header {
        return false;
    }
    let latch_label = func.blocks[latch].label;
    match &func.blocks[latch].terminator {
        Terminator::Branch(lbl) if *lbl == header_label => {}
        _ => return false,
    }

    // Body blocks: everything except the header. The latch is among them.
    let body_blocks: Vec<usize> = lp.body.iter().copied().filter(|&b| b != header).collect();
    if body_blocks.is_empty() || body_blocks.len() > 32 {
        return false;
    }

    // All body successors stay inside body ∪ {header}; terminators are
    // Branch/CondBranch/Switch (a Ret/Unreachable inside the loop breaks the
    // "always executes exactly `trip` times" invariant).
    let in_loop_target = |lbl: BlockId| -> bool {
        func.blocks
            .iter()
            .position(|b| b.label == lbl)
            .is_some_and(|bi| lp.body.contains(&bi))
    };
    for &bi in &body_blocks {
        let succs: Vec<BlockId> = match &func.blocks[bi].terminator {
            Terminator::Branch(l) => vec![*l],
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => vec![*true_label, *false_label],
            Terminator::Switch { default, cases, .. } => {
                let mut v = vec![*default];
                v.extend(cases.iter().map(|(_, l)| *l));
                v
            }
            _ => return false,
        };
        for s in succs {
            if s != header_label && !in_loop_target(s) {
                return false;
            }
        }
    }

    // Unique preheader (the header's only non-latch predecessor).
    if loop_analysis::find_preheader(header, &lp.body, &cfg.preds).is_none() {
        return false;
    }

    // Basic IV: phi in header, Add(phi, step) in the latch. The stride may
    // be any non-zero constant; complete_unroll_trip rejects degenerate
    // stride/comparison combinations.
    let Some((iv_phi, iv_ty, iv_step, latch_iv_incr_idx)) =
        find_iv_in_loop(func, header, latch, latch_label)
    else {
        return false;
    };

    // Exit from the header's CondBranch.
    let Some((exit_target, body_entry, raw_cmp_op, cmp_ty, exit_limit, iv_is_lhs, exit_pos)) =
        find_exit_condition(func, header, &lp.body, iv_phi)
    else {
        return false;
    };
    // The compare must be in the IV's own type, and its operator is put into
    // the canonical "continue while iv OP limit" form before any arithmetic.
    if cmp_ty != iv_ty {
        return false;
    }
    let cmp_op = canonical_continue_cmp(raw_cmp_op, iv_is_lhs, exit_pos);

    // Constant init (through const chains) and constant limit.
    let mut iv_init_op: Option<Operand> = None;
    for inst in &func.blocks[header].instructions {
        if let Instruction::Phi { dest, incoming, .. } = inst {
            if dest.0 == iv_phi.0 {
                for (op, lbl) in incoming {
                    if *lbl != latch_label {
                        iv_init_op = Some(op.clone());
                    }
                }
            }
        }
    }
    let Some(iv_init_op) = iv_init_op else {
        return false;
    };
    let Some(iv_init) = resolve_const_operand(func, &iv_init_op, 0) else {
        return false;
    };
    let Some(limit_n) = resolve_const_operand(func, &exit_limit, 0) else {
        return false;
    };

    // Trip count (same arithmetic as the two-block unroller).
    let Some(trip) = complete_unroll_trip(iv_init, limit_n, cmp_op, iv_step, iv_ty) else {
        return false;
    };
    // The post-loop IV may exceed i64 even when the trip count is small
    // (e.g. init = 0, limit = i64::MAX, step = 2^62 -> trip = 2, final =
    // 2^63): refuse before mutating rather than substitute a wrapped
    // constant, or worse, return false after rewriting the CFG.
    let Some(final_iv_n) = iv_step
        .checked_mul(trip)
        .and_then(|d| iv_init.checked_add(d))
    else {
        return false;
    };
    if !trip_range.contains(&trip) {
        return false;
    }

    // Instruction budget: header non-phi + body, times trip. FP-heavy bodies
    // get HALF the budget: their unrolled copies create simultaneously-live
    // FP temps that the linear-scan XMM pool spills (nbody's advance: 1183
    // insns / 476 stack-refs vs 594/159 un-unrolled — the unroll is code
    // growth without runtime gain once the spills dominate). Integer bodies
    // keep the full 512 budget.
    let header_nonphi: Vec<usize> = func.blocks[header]
        .instructions
        .iter()
        .enumerate()
        .filter(|(_, inst)| !matches!(inst, Instruction::Phi { .. }))
        .map(|(i, _)| i)
        .collect();
    let total_insts: usize = header_nonphi.len()
        + body_blocks
            .iter()
            .map(|&bi| func.blocks[bi].instructions.len())
            .sum::<usize>();
    let has_fp = body_blocks.iter().any(|&bi| {
        func.blocks[bi]
            .instructions
            .iter()
            .any(|inst| matches!(inst, Instruction::BinOp { ty, .. } | Instruction::Load { ty, .. } | Instruction::Store { ty, .. } if ty.is_float()))
    }) || header_nonphi.iter().any(|&hi| {
        matches!(&func.blocks[header].instructions[hi], Instruction::BinOp { ty, .. } if ty.is_float())
    });
    // Tunable for A/B experiments (default 256 FP / 512 int).
    let budget: usize = if has_fp {
        std::env::var("CCC_CUNROLL_FP_BUDGET")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(256)
    } else {
        std::env::var("CCC_CUNROLL_INT_BUDGET")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(512)
    };
    if total_insts.saturating_mul(trip as usize) > budget {
        return false;
    }

    // Disqualifying instructions in body and header.
    let inst_ok = |inst: &Instruction| -> bool {
        match inst {
            Instruction::Call { .. }
            | Instruction::CallIndirect { .. }
            | Instruction::InlineAsm { .. }
            | Instruction::DynAlloca { .. } => false,
            Instruction::Intrinsic { op, .. } => op.is_pure(),
            Instruction::PgoCounterInc { .. } => false,
            // The general CFG cloner currently models carried state through
            // header phis.  Pointer-controlled diamonds can encode additional
            // mutable state through local pointer objects (often around a
            // nested loop); substituting the outer IV then made every clone
            // reuse iteration zero's pointer comparison.  Refuse this shape
            // until memory-SSA/object-state carrying is explicit.  Numeric and
            // FP nested loops—the performance target of the general form—are
            // unaffected.
            Instruction::Cmp { lhs, rhs, .. }
                if operand_has_pointer_origin(func, lhs)
                    || operand_has_pointer_origin(func, rhs) =>
            {
                false
            }
            _ => true,
        }
    };
    for &bi in &body_blocks {
        for inst in &func.blocks[bi].instructions {
            if !inst_ok(inst) {
                return false;
            }
        }
    }
    for &hi in &header_nonphi {
        if !inst_ok(&func.blocks[header].instructions[hi]) {
            return false;
        }
    }
    if latch_is_bit_iteration(func, latch) {
        return false;
    }

    // Loop-carried phis (skip the IV): init from the preheader edge, back
    // value defined somewhere in the body.
    let mut carried: Vec<(u32, Operand, u32)> = Vec::new();
    for inst in &func.blocks[header].instructions {
        let Instruction::Phi { dest, incoming, .. } = inst else {
            continue;
        };
        if dest.0 == iv_phi.0 {
            continue;
        }
        let mut init: Option<Operand> = None;
        let mut back: Option<u32> = None;
        for (op, lbl) in incoming {
            if *lbl == latch_label {
                if let Operand::Value(v) = op {
                    back = Some(v.0);
                }
            } else {
                init = Some(op.clone());
            }
        }
        if let (Some(init_op), Some(back_id)) = (init, back) {
            if is_defined_in_body(back_id, &lp.body, func) {
                carried.push((dest.0, init_op, back_id));
            }
        }
    }

    // ── Build clones for iterations 1..trip ─────────────────────────────────
    let mut next_label = func.blocks.iter().map(|b| b.label.0).max().unwrap_or(0) + 1;
    let mut next_val = func.next_value_id;
    let num_clones = (trip - 1) as usize;

    // Per-clone: header-copy label, per-body-block labels, vmap.
    struct ClonePlan {
        header_copy: BlockId,
        block_labels: Vec<BlockId>, // parallel to body_blocks
        vmap: FxHashMap<u32, u32>,
    }
    let mut plans: Vec<ClonePlan> = Vec::with_capacity(num_clones);
    for _ in 0..num_clones {
        let header_copy = BlockId(next_label);
        next_label += 1;
        let block_labels = body_blocks
            .iter()
            .map(|_| {
                let l = BlockId(next_label);
                next_label += 1;
                l
            })
            .collect();
        let mut vmap: FxHashMap<u32, u32> = FxHashMap::default();
        // Header non-phi defs.
        for &hi in &header_nonphi {
            if let Some(d) = func.blocks[header].instructions[hi].dest() {
                vmap.insert(d.0, next_val);
                next_val += 1;
            }
        }
        // Body defs (skip the latch IV increment — it is dead per iteration).
        for &bi in &body_blocks {
            for (idx, inst) in func.blocks[bi].instructions.iter().enumerate() {
                if bi == latch && idx == latch_iv_incr_idx {
                    continue;
                }
                if let Some(d) = inst.dest() {
                    vmap.insert(d.0, next_val);
                    next_val += 1;
                }
            }
        }
        plans.push(ClonePlan {
            header_copy,
            block_labels,
            vmap,
        });
    }

    let mut new_blocks: Vec<BasicBlock> = Vec::new();
    // prev_back[c]: value id delivering carried phi c entering this clone
    // (clone 1 uses the ORIGINAL body's back value; clone t>1 uses
    // clone t-1's renamed back value).
    let mut prev_back: Vec<u32> = carried.iter().map(|&(_, _, back)| back).collect();
    let mut final_carried: Vec<(u32, u32)> = Vec::new();

    for (ci, plan) in plans.iter().enumerate() {
        let t = (ci + 1) as i64; // iteration index this clone represents
                                 // Clone `t` observes the IV after `t` strides, not after `t` steps of
                                 // 1: with the constant-stride trip count in place the stride may be
                                 // any non-zero constant, so the substituted constant must be scaled
                                 // (mirrors the two-block cloner's `iv_init + t_idx * iv_step`).
        let iv_const = Operand::Const(IrConst::from_i64(iv_init + t * iv_step, iv_ty));

        // label_map: original label → clone label (header + body).
        let mut label_map: FxHashMap<BlockId, BlockId> = FxHashMap::default();
        label_map.insert(header_label, plan.header_copy);
        for (i, &bi) in body_blocks.iter().enumerate() {
            label_map.insert(func.blocks[bi].label, plan.block_labels[i]);
        }

        // Where this clone's latch back-edge goes: next clone's header-copy,
        // or the exit target for the last clone.
        let back_redirect = if ci + 1 < num_clones {
            plans[ci + 1].header_copy
        } else {
            exit_target
        };

        // Operand substitution: iv → const, carried → previous value.
        let substitute = |inst: &mut Instruction| {
            subst_value_with_operand(inst, iv_phi.0, &iv_const);
            for (c, &(phi_id, _, _)) in carried.iter().enumerate() {
                let repl = Operand::Value(Value(prev_back[c]));
                subst_value_with_operand(inst, phi_id, &repl);
            }
        };
        let substitute_term = |term: &mut Terminator| {
            subst_value_in_terminator(term, iv_phi.0, &iv_const);
            for (c, &(phi_id, _, _)) in carried.iter().enumerate() {
                let repl = Operand::Value(Value(prev_back[c]));
                subst_value_in_terminator(term, phi_id, &repl);
            }
        };

        // Header copy: non-phi instructions, unconditional Branch into the
        // body entry (trip ≥ 2 ⇒ every cloned iteration executes).
        {
            let mut insts: Vec<Instruction> = Vec::new();
            for &hi in &header_nonphi {
                let mut cloned = func.blocks[header].instructions[hi].clone();
                // ORDER: rename FIRST, substitute header-phi references
                // AFTER. Clone 1's carried back values are ORIGINAL body
                // ids that are also in this clone's vmap — substituting
                // first would let the rename rewrite them into this
                // clone's own definitions (self-referential phi).
                replace_values_in_inst(&mut cloned, &plan.vmap);
                rename_inst_dest(&mut cloned, &plan.vmap);
                substitute(&mut cloned);
                insts.push(cloned);
            }
            new_blocks.push(BasicBlock {
                label: plan.header_copy,
                instructions: insts,
                terminator: Terminator::Branch(
                    *label_map
                        .get(&body_entry)
                        .expect("body entry must be a body block"),
                ),
                source_spans: Vec::new(),
            });
        }

        // Body block copies.
        for (i, &bi) in body_blocks.iter().enumerate() {
            let orig = &func.blocks[bi];
            let mut insts: Vec<Instruction> = Vec::new();
            for (idx, inst) in orig.instructions.iter().enumerate() {
                if bi == latch && idx == latch_iv_incr_idx {
                    continue; // dead IV increment
                }
                let mut cloned = inst.clone();
                // Rename first, substitute after (see the header-copy note:
                // clone 1's carried values are original body ids present in
                // this clone's vmap).
                replace_values_in_inst(&mut cloned, &plan.vmap);
                rename_inst_dest(&mut cloned, &plan.vmap);
                substitute(&mut cloned);
                // Phi incoming labels: remap through the label map (inner
                // loop headers cloned inside this iteration). Labels equal
                // to the OUTER header label now arrive from this clone's
                // header copy — the actual predecessor.
                if let Instruction::Phi { incoming, .. } = &mut cloned {
                    for (op, lbl) in incoming.iter_mut() {
                        if *lbl == header_label {
                            *lbl = plan.header_copy;
                        } else if let Some(&nl) = label_map.get(lbl) {
                            *lbl = nl;
                        }
                    }
                }
                insts.push(cloned);
            }
            let mut term = orig.terminator.clone();
            // Rename value uses (e.g. an inner loop header's exit compare)
            // first — then substitute header-phi references (see the
            // header-copy ordering note).
            replace_values_in_terminator(&mut term, &plan.vmap);
            // Back-edge (latch → header) redirects to the next iteration.
            if bi == latch {
                if let Terminator::Branch(lbl) = &mut term {
                    if *lbl == header_label {
                        *lbl = back_redirect;
                    }
                }
            }
            // General label remap for internal branches.
            replace_block_ids(&mut term, &label_map);
            substitute_term(&mut term);
            new_blocks.push(BasicBlock {
                label: plan.block_labels[i],
                instructions: insts,
                terminator: term,
                source_spans: Vec::new(),
            });
        }

        // Update prev_back for the NEXT clone: this clone's renamed back
        // values. The LAST clone's values feed outside uses.
        for (c, &(_, _, back_id)) in carried.iter().enumerate() {
            if let Some(&new_id) = plan.vmap.get(&back_id) {
                prev_back[c] = new_id;
                if ci + 1 == num_clones {
                    final_carried.push((carried[c].0, new_id));
                }
            }
        }
    }

    func.next_value_id = next_val;

    // ── Mutate the originals ────────────────────────────────────────────────
    // Iteration 0 keeps the original header + body. The header's terminator
    // becomes an unconditional Branch into the body entry.
    func.blocks[header].terminator = Terminator::Branch(body_entry);
    // The original latch's back-edge flows into clone 1's header copy — or
    // straight to the exit when the loop runs exactly once (no clones).
    func.blocks[latch].terminator = Terminator::Branch(match plans.first() {
        Some(first) => first.header_copy,
        None => exit_target,
    });
    if num_clones == 0 {
        // No clone ever renamed the carried values: the original body's
        // back-edge definitions ARE the final values.
        final_carried = carried.iter().map(|&(phi_id, _, back)| (phi_id, back)).collect();
    }

    // Outside uses: IV → final constant; carried phis → final clone values.
    let final_iv = Operand::Const(IrConst::from_i64(final_iv_n, iv_ty));
    for (block_index, block) in func.blocks.iter_mut().enumerate() {
        if lp.body.contains(&block_index) {
            continue;
        }
        for instruction in &mut block.instructions {
            subst_value_with_operand(instruction, iv_phi.0, &final_iv);
            for &(phi_id, final_id) in &final_carried {
                let repl = Operand::Value(Value(final_id));
                subst_value_with_operand(instruction, phi_id, &repl);
            }
        }
        subst_value_in_terminator(&mut block.terminator, iv_phi.0, &final_iv);
        for &(phi_id, final_id) in &final_carried {
            let repl = Operand::Value(Value(final_id));
            subst_value_in_terminator(&mut block.terminator, phi_id, &repl);
        }
    }

    // Exit-block phi edges: the edge that arrived from the header now
    // arrives from the last clone's latch (the block that branches to the
    // exit target).
    if let Some(exit_bi) = func.blocks.iter().position(|b| b.label == exit_target) {
        if exit_bi != header && !lp.body.contains(&exit_bi) {
            let last_latch_label = match plans.last() {
                Some(last) => last.block_labels[body_blocks
                    .iter()
                    .position(|&b| b == latch)
                    .expect("latch is a body block")],
                None => latch_label,
            };
            for inst in &mut func.blocks[exit_bi].instructions {
                if let Instruction::Phi { incoming, .. } = inst {
                    for (_, lbl) in incoming.iter_mut() {
                        if *lbl == header_label {
                            *lbl = last_latch_label;
                        }
                    }
                }
            }
        }
    }

    // Replace the header's (now dead) phis with Copies of their INIT values.
    // The header still executes exactly once — entered from the preheader —
    // so the correct value is the NON-LATCH incoming. Taking
    // `incoming.first()` instead is wrong whenever the latch edge is listed
    // first: the phi would become a self-referential cycle with its own
    // back-edge value (e.g. `i = i + 1`), leaving iteration 0's IV garbage
    // and silently skipping its body.
    for inst in func.blocks[header].instructions.iter_mut() {
        if let Instruction::Phi { dest, incoming, .. } = inst {
            let init = incoming
                .iter()
                .find(|(_, lbl)| *lbl != latch_label)
                .map(|(op, _)| *op);
            if let Some(src) = init {
                *inst = Instruction::Copy { dest: *dest, src };
            }
        }
    }

    func.blocks.extend(new_blocks);
    true
}

fn try_complete_unroll_two_block(
    func: &mut IrFunction,
    lp: &loop_analysis::NaturalLoop,
    cfg: &CfgAnalysis,
) -> bool {
    if !matches!(lp.body.len(), 2 | 3) {
        return false;
    }
    let header = lp.header;
    let back_preds: Vec<usize> = cfg
        .preds
        .row(header)
        .iter()
        .map(|&p| p as usize)
        .filter(|p| lp.body.contains(p))
        .collect();
    if back_preds.len() != 1 {
        return false;
    }
    let latch = back_preds[0];
    if latch == header {
        return false;
    }
    let header_label = func.blocks[header].label;
    let latch_label = func.blocks[latch].label;
    match &func.blocks[latch].terminator {
        Terminator::Branch(lbl) if *lbl == header_label => {}
        _ => return false,
    }
    let Some((iv_phi, iv_ty, iv_step, latch_iv_incr_idx)) =
        find_iv_in_loop(func, header, latch, latch_label)
    else {
        return false;
    };
    let Some((
        exit_target,
        body_entry,
        raw_cmp_op,
        cmp_ty,
        exit_limit,
        iv_is_lhs,
        exit_cond_positive,
    )) = find_exit_condition(func, header, &lp.body, iv_phi)
    else {
        return false;
    };
    if cmp_ty != iv_ty {
        return false;
    }
    let cmp_op = canonical_continue_cmp(raw_cmp_op, iv_is_lhs, exit_cond_positive);
    // work_blocks: instructions to clone each iteration (excluding IV increment).
    // 2-block: work lives in the latch (body_entry == latch).
    // 3-block: header -> body_entry (one work block) -> latch (IV incr only) -> header.
    let work_blocks: Vec<usize> = if body_entry == latch_label {
        vec![latch]
    } else {
        let Some(bi) = func.blocks.iter().position(|b| b.label == body_entry) else {
            return false;
        };
        // body must be in the loop and branch to latch
        if !lp.body.contains(&bi) {
            return false;
        }
        match &func.blocks[bi].terminator {
            Terminator::Branch(t) if *t == latch_label => {}
            _ => return false,
        }
        // Only pure linear header->body->latch
        if lp.body.len() != 3 {
            return false;
        }
        vec![bi, latch]
    };
    // Constant IV init from the preheader edge.  Resolved through the same
    // const-chain evaluator as the general cloner so that a 2–3 block inner
    // loop whose init became `Add(const, const)` after an outer complete
    // unroll is handled in the SAME fixpoint round instead of waiting for a
    // later folding pass (or never, when unroll is the last loop pass).
    let mut iv_init_op: Option<Operand> = None;
    for inst in &func.blocks[header].instructions {
        if let Instruction::Phi { dest, incoming, .. } = inst {
            if dest.0 == iv_phi.0 {
                for (op, lbl) in incoming {
                    if *lbl != latch_label {
                        iv_init_op = Some(op.clone());
                    }
                }
            }
        }
    }
    let Some(iv_init_op) = iv_init_op else {
        return false;
    };
    let Some(iv_init) = resolve_const_operand(func, &iv_init_op, 0) else {
        return false;
    };
    let Some(limit_n) = resolve_const_operand(func, &exit_limit, 0) else {
        return false;
    };
    let Some(trip) = complete_unroll_trip(iv_init, limit_n, cmp_op, iv_step, iv_ty) else {
        return false;
    };
    // The post-loop IV may exceed i64 even when the trip count is small
    // (e.g. init = 0, limit = i64::MAX, step = 2^62 -> trip = 2, final =
    // 2^63): refuse before mutating rather than substitute a wrapped
    // constant, or worse, return false after rewriting the CFG.
    let Some(final_iv_n) = iv_step
        .checked_mul(trip)
        .and_then(|d| iv_init.checked_add(d))
    else {
        return false;
    };
    // Trip bound 16 with a 512-expanded-instruction budget (levkropp
    // 1b4bac8b's full-unroll limits, grafted onto our two-block complete
    // unroller): trips 9..16 are profitable for the same struct-return
    // temp-forwarding reasons as 4/8 once the expansion stays cache-tight.
    // The pow2 restriction below still applies to trips with F32-only
    // bodies (residual map-vectorizer interaction).
    if !(2..=16).contains(&trip) {
        return false;
    }
    {
        let body_insts: usize = work_blocks
            .iter()
            .map(|&wbi| func.blocks[wbi].instructions.len())
            .sum();
        if body_insts.saturating_mul(trip as usize) > 512 {
            return false;
        }
    }
    // Reject F32-only residual map loops (vectorizer remainder expects a loop).
    // F64 (nbody) and integer (struct_copy) non-pow2 trips are allowed.
    if !matches!(trip, 2 | 4 | 8) {
        let mut has_f32 = false;
        let mut has_f64 = false;
        for &wbi in &work_blocks {
            for inst in &func.blocks[wbi].instructions {
                match inst {
                    Instruction::BinOp {
                        ty: IrType::F32, ..
                    }
                    | Instruction::Load {
                        ty: IrType::F32, ..
                    }
                    | Instruction::Store {
                        ty: IrType::F32, ..
                    } => has_f32 = true,
                    Instruction::BinOp {
                        ty: IrType::F64, ..
                    }
                    | Instruction::Load {
                        ty: IrType::F64, ..
                    }
                    | Instruction::Store {
                        ty: IrType::F64, ..
                    } => has_f64 = true,
                    _ => {}
                }
            }
        }
        if has_f32 && !has_f64 {
            return false;
        }
    }
    for &wbi in &work_blocks {
        for inst in &func.blocks[wbi].instructions {
            match inst {
                Instruction::Call { .. }
                | Instruction::CallIndirect { .. }
                | Instruction::InlineAsm { .. }
                | Instruction::DynAlloca { .. } => return false,
                // Pure intrinsics (sqrt/FMA-class) are side-effect-free and
                // clone-safely; non-pure ones (stores via dest_ptr, fences,
                // atomics) stay rejected. This keeps FP loop bodies like
                // nbody's energy()/advance() eligible for complete unrolling.
                Instruction::Intrinsic { op, .. } => {
                    if !op.is_pure() {
                        return false;
                    }
                }
                _ => {}
            }
        }
    }
    if latch_is_bit_iteration(func, latch) {
        return false;
    }

    // Loop-carried phis (phi_id, init_op, back_value_id in latch). Skip IV phi.
    let mut carried: Vec<(u32, Operand, u32)> = Vec::new();
    for inst in &func.blocks[header].instructions {
        let Instruction::Phi { dest, incoming, .. } = inst else {
            continue;
        };
        if dest.0 == iv_phi.0 {
            continue;
        }
        let mut init: Option<Operand> = None;
        let mut back: Option<u32> = None;
        for (op, lbl) in incoming {
            if *lbl == latch_label {
                if let Operand::Value(v) = op {
                    back = Some(v.0);
                }
            } else {
                init = Some(op.clone());
            }
        }
        if let (Some(init_op), Some(back_id)) = (init, back) {
            let defined_in_latch = func.blocks[latch]
                .instructions
                .iter()
                .any(|li| li.dest().map(|d| d.0 == back_id).unwrap_or(false));
            if defined_in_latch {
                carried.push((dest.0, init_op, back_id));
            }
        }
    }

    let mut next_label = func.blocks.iter().map(|b| b.label.0).max().unwrap_or(0) + 1;
    let mut next_val = func.next_value_id;
    let labels: Vec<BlockId> = (0..trip)
        .map(|_| {
            let l = BlockId(next_label);
            next_label += 1;
            l
        })
        .collect();

    let mut prev_back: Vec<Option<u32>> = vec![None; carried.len()];
    let mut final_carried: Vec<(u32, u32)> = Vec::new();
    let mut new_blocks: Vec<BasicBlock> = Vec::with_capacity(trip as usize);

    for t_idx in 0..trip {
        let iv_const = Operand::Const(IrConst::from_i64(iv_init + t_idx * iv_step, iv_ty));
        let mut vmap: FxHashMap<u32, u32> = FxHashMap::default();
        for &wbi in &work_blocks {
            for (idx, inst) in func.blocks[wbi].instructions.iter().enumerate() {
                if wbi == latch && idx == latch_iv_incr_idx {
                    continue;
                }
                if let Some(d) = inst.dest() {
                    vmap.insert(d.0, next_val);
                    next_val += 1;
                }
            }
        }
        let mut new_insts = Vec::new();
        for &wbi in &work_blocks {
            for (idx, inst) in func.blocks[wbi].instructions.iter().enumerate() {
                if wbi == latch && idx == latch_iv_incr_idx {
                    continue;
                }
                let mut cloned = inst.clone();
                subst_value_with_operand(&mut cloned, iv_phi.0, &iv_const);
                for (ci, (phi_id, init_op, _)) in carried.iter().enumerate() {
                    match prev_back[ci] {
                        None => subst_value_with_operand(&mut cloned, *phi_id, init_op),
                        Some(prev_id) => subst_value_with_operand(
                            &mut cloned,
                            *phi_id,
                            &Operand::Value(Value(prev_id)),
                        ),
                    }
                }
                replace_values_in_inst(&mut cloned, &vmap);
                rename_inst_dest(&mut cloned, &vmap);
                new_insts.push(cloned);
            }
        }
        for (ci, (_, _, back_id)) in carried.iter().enumerate() {
            if let Some(&new_id) = vmap.get(back_id) {
                prev_back[ci] = Some(new_id);
                if t_idx + 1 == trip {
                    final_carried.push((carried[ci].0, new_id));
                }
            }
        }
        let term = if t_idx + 1 < trip {
            Terminator::Branch(labels[(t_idx + 1) as usize])
        } else {
            Terminator::Branch(exit_target)
        };
        new_blocks.push(BasicBlock {
            label: labels[t_idx as usize],
            instructions: new_insts,
            terminator: term,
            source_spans: Vec::new(),
        });
    }
    func.next_value_id = next_val;

    func.blocks[header].terminator = Terminator::Branch(labels[0]);
    // The latch is now UNREACHABLE: the header branches into the clone chain
    // and the chain's last block branches straight to `exit_target`, so no
    // edge reaches the latch any more.
    //
    // This used to be `Branch(exit_target)`, which fabricated a control-flow
    // edge latch->exit that the *program* does not have.  The latch was only
    // ever a predecessor of the header (its terminator is checked to be
    // `Branch(header_label)` above), never of the exit, so any phi in the
    // exit block acquired a predecessor it had no incoming for.  That is a
    // hard SSA violation, and it is exactly what the IR verifier reported on
    // linux-cachymod 6.18.47 `drivers/gpu/drm/i915/display/intel_sprite.c`:
    //
    //   after `loop_unroll_post_vec` in `vlv_sprite_update_arm`:
    //     phi v904 has an incoming from BlockId(94080), which is not a
    //     predecessor (real predecessors: [93725, 94081, 94090])
    //     phi v904 has no incoming for predecessor BlockId(94090)
    //
    // Downstream, v904 reached x86 codegen with no register home and no stack
    // slot, and `operand_to_rax`'s hard gate aborted the compile ("refusing to
    // fabricate a value").  Before that gate existed this same shape silently
    // emitted `xorl %eax,%eax` — i.e. a miscompile.  Marking the block
    // `Unreachable` states the truth; DCE/CFG-simplify then delete it.
    func.blocks[latch].terminator = Terminator::Unreachable;
    let _ = body_entry;

    // Repair the exit block's phis.  The `header -> exit_target` edge is gone
    // (the header now branches into the clone chain); the block that reaches
    // the exit is the LAST CLONE, `labels[trip - 1]`.  Every phi incoming
    // still labelled `header_label` must therefore be relabelled to that
    // clone, otherwise it names a non-predecessor and the real predecessor
    // has no incoming at all.
    //
    // Only the LABEL changes, never the operand: the substitution sweep below
    // visits every block outside `lp.body` -- the exit block included -- and
    // rewrites the IV to `final_iv` and each carried phi to its final clone
    // value, which are precisely the values live on that edge.
    //
    // A phi incoming labelled `latch_label` cannot exist here (the latch's
    // terminator was verified to be `Branch(header_label)`), but drop any such
    // entry defensively rather than leave a dangling predecessor behind.
    {
        let last_clone = labels[(trip - 1) as usize];
        if let Some(exit_bi) = func.blocks.iter().position(|b| b.label == exit_target) {
            for inst in func.blocks[exit_bi].instructions.iter_mut() {
                let Instruction::Phi { incoming, .. } = inst else {
                    continue;
                };
                incoming.retain(|(_, lbl)| *lbl != latch_label);
                for (_, lbl) in incoming.iter_mut() {
                    if *lbl == header_label {
                        *lbl = last_clone;
                    }
                }
            }
        }
    }

    // `final_iv_n` was already computed and overflow-checked above, before
    // any CFG mutation — a second check here would bail with the header and
    // latch already retargeted (the exact bug the hoisted check fixed).
    let final_iv = Operand::Const(IrConst::from_i64(final_iv_n, iv_ty));
    for (block_index, block) in func.blocks.iter_mut().enumerate() {
        if lp.body.contains(&block_index) {
            continue;
        }
        for instruction in &mut block.instructions {
            subst_value_with_operand(instruction, iv_phi.0, &final_iv);
            for &(phi_id, final_id) in &final_carried {
                subst_value_with_operand(instruction, phi_id, &Operand::Value(Value(final_id)));
            }
        }
        subst_value_in_terminator(&mut block.terminator, iv_phi.0, &final_iv);
        for &(phi_id, final_id) in &final_carried {
            subst_value_in_terminator(
                &mut block.terminator,
                phi_id,
                &Operand::Value(Value(final_id)),
            );
        }
    }

    func.blocks.extend(new_blocks);

    // Resolve EVERY phi in the header. The loop is now straight-line -- the
    // header's terminator is an unconditional branch into the clone chain and
    // the latch no longer branches back -- so the header has exactly one
    // predecessor (the preheader), executes exactly once, and a phi there is
    // both malformed and meaningless.
    //
    // The value to use is the NON-LATCH (init) incoming, for every phi
    // including the loop-carried ones. Two independent reasons, and BOTH have
    // been violated by real code in this function's history:
    //
    //  1. SSA ORDER. Copying the final clone's dest here is a copy-before-def:
    //     the clone that defines it runs AFTER this header, so the definition
    //     does not dominate the use. GVN/copy-prop then forwards an unordered
    //     value into inlined callers -- observed as simd_crc_adler returning
    //     00010001 instead of 00bd00bc for sz=2 under CCC_LOOP_ROTATE=1.
    //     Do NOT "fix" this by reaching for `final_carried` here.
    //
    //  2. IT IS UNNECESSARY. Outside uses of a carried phi were already
    //     rewritten to the final clone's dest by the substitution loop above,
    //     which visits every block NOT in `lp.body`. Nothing outside still
    //     reads the phi id, so the header Copy only has to be well-formed.
    //
    // Rewriting only *some* of the phis is what made this latent: the pass
    // used to convert the loop-carried ones and leave the induction variable's
    // phi behind, still naming the latch as a predecessor (a block that no
    // longer branches here) and -- because the carried phis ahead of it had
    // become Copies -- sitting after a non-phi instruction. DCE deletes that
    // dead phi before codegen, which is the only reason it never showed up in
    // output, but every pass scheduled between here and DCE consumed invalid
    // IR. `try_complete_unroll_general` always did this correctly; the two
    // paths had simply diverged.
    //
    // Regression coverage: tests/regression/unroll_header_phi_resolve.c (a
    // true negative control -- with this loop removed it reports both
    // STALE_PRED and PHI_ORDER against `loop_unroll`) and the unit test
    // `complete_unroll_leaves_structurally_valid_ir`.
    //
    // Scanning the header alone is sufficient AND cheaper than the historical
    // `carried` x every-block x every-instruction sweep: `carried` is built
    // exclusively from `func.blocks[header]` (see its construction above), so
    // no phi outside the header could ever have matched.
    for inst in func.blocks[header].instructions.iter_mut() {
        let Instruction::Phi { dest, incoming, .. } = inst else {
            continue;
        };
        let dest = *dest;
        let init = incoming
            .iter()
            .find(|(_, lbl)| *lbl != latch_label)
            .map(|(op, _)| *op);
        // Guard the copy-before-def hazard described above: a future edit that
        // reaches for the final clone's value here would reintroduce an SSA
        // violation that the corpus CANNOT catch (every test still prints the
        // right answer; only the IR is malformed). Fail loudly in debug builds
        // instead.
        debug_assert!(
            !matches!(init, Some(Operand::Value(v))
                if final_carried.iter().any(|&(_, final_id)| final_id == v.0)),
            "header phi v{} resolved to a clone value defined after the header \
             (copy-before-def); it must resolve to its non-latch incoming",
            dest.0
        );
        match init {
            Some(src) => *inst = Instruction::Copy { dest, src },
            None => {
                // Unreachable for a natural loop: the header is dominated by a
                // preheader, so every header phi has a non-latch incoming.
                // Leaving the phi in place would keep the exact structural
                // violation this loop exists to remove, so make the impossible
                // case loud in debug builds instead of silently malformed.
                debug_assert!(
                    false,
                    "header phi v{} has no non-latch incoming after complete unroll",
                    dest.0
                );
            }
        }
    }
    true
}

pub(crate) fn subst_value_with_operand(inst: &mut Instruction, old_id: u32, new_op: &Operand) {
    inst.for_each_operand_mut(|operand| {
        if matches!(operand, Operand::Value(value) if value.0 == old_id) {
            *operand = new_op.clone();
        }
    });
    if let Operand::Value(replacement) = new_op {
        inst.for_each_value_use_mut(|value| {
            if value.0 == old_id {
                *value = *replacement;
            }
        });
    }
}

pub(crate) fn subst_value_in_terminator(
    terminator: &mut Terminator,
    old_id: u32,
    new_op: &Operand,
) {
    terminator.for_each_operand_mut(|operand| {
        if matches!(operand, Operand::Value(value) if value.0 == old_id) {
            *operand = new_op.clone();
        }
    });
}

/// Evaluate an operand as an integer constant by looking through a bounded
/// chain of pure integer Copy/Cast/BinOp(Add/Sub/Mul) definitions.
///
/// Purpose: complete unrolling of an OUTER loop turns inner-loop IV initial
/// values (the triangular `for (j = i+1; ...)` shape) into constant
/// *expressions* (`Add(const, 1)`) that the pipeline's constant folder has
/// not run on yet (the unroller runs before constant folding in the same
/// pipeline iteration). Resolving the chain here lets the complete-unroll
/// fixpoint cascade outer→inner within a single `unroll_loops` call.
/// Whether an operand is transitively an object address. Pointer comparisons
/// are represented as I64 on x86, so checking only `Cmp.ty == Ptr` misses the
/// common `p == &local` form used by pointer-state loops.
fn operand_has_pointer_origin(func: &IrFunction, op: &Operand) -> bool {
    // Walk the def-web of `op` looking for a pointer-producing instruction.
    // A visited set (rather than a recursion-depth cap) terminates the walk:
    // induction-variable cycles (`phi <- Add(phi, step)` in the latch) are
    // ordinary in every counted loop, and a depth cap would fail closed on
    // them, rejecting the exact numeric loops this analysis is meant to
    // admit.  Each distinct value is expanded once, so the walk stays linear
    // in the size of the web; pointer definitions are found at any depth.
    let Operand::Value(root) = op else {
        return false;
    };
    let mut visited: FxHashSet<Value> = FxHashSet::default();
    let mut stack: Vec<Value> = vec![*root];
    while let Some(value) = stack.pop() {
        if !visited.insert(value) {
            continue;
        }
        let def = func
            .blocks
            .iter()
            .flat_map(|block| block.instructions.iter())
            .find(|inst| inst.dest() == Some(value));
        let mut push = |stack: &mut Vec<Value>, operand: &Operand| {
            if let Operand::Value(v) = operand {
                stack.push(*v);
            }
        };
        match def {
            Some(
                Instruction::Alloca { .. }
                | Instruction::DynAlloca { .. }
                | Instruction::GlobalAddr { .. }
                | Instruction::LabelAddr { .. }
                | Instruction::GetElementPtr { .. },
            ) => return true,
            Some(Instruction::Copy { src, .. } | Instruction::Cast { src, .. }) => {
                push(&mut stack, src);
            }
            Some(Instruction::Select {
                true_val,
                false_val,
                ..
            }) => {
                push(&mut stack, true_val);
                push(&mut stack, false_val);
            }
            Some(Instruction::Phi { incoming, .. }) => {
                for (incoming, _) in incoming {
                    push(&mut stack, incoming);
                }
            }
            Some(Instruction::BinOp { lhs, rhs, .. }) => {
                push(&mut stack, lhs);
                push(&mut stack, rhs);
            }
            _ => {}
        }
    }
    false
}

/// Evaluate `op` to a constant when it is a constant or a short chain of
/// `Copy` / integer `Cast` / `Add` / `Sub` / `Mul` over constants — the shape
/// an inner loop's `j = i + 1` init takes once the outer loop has been
/// completely unrolled and `i` substituted by a per-clone constant.
///
/// Every intermediate result is normalised to the canonical `IrConst`
/// representation of the instruction's result type via
/// [`normalize_to_type`], so unsigned arithmetic zero-extends and narrowing
/// casts truncate exactly as the constant folder would.  Values are only ever
/// compared by [`complete_unroll_trip`] after a second normalisation to the
/// IV type, which makes the whole chain representation-stable regardless of
/// which pass produced the constants.
fn resolve_const_operand(func: &IrFunction, op: &Operand, depth: usize) -> Option<i64> {
    if depth > 6 {
        return None;
    }
    match op {
        Operand::Const(c) => c.to_i64(),
        Operand::Value(v) => {
            let mut def: Option<&Instruction> = None;
            'outer: for block in &func.blocks {
                for inst in &block.instructions {
                    if let Some(d) = inst.dest() {
                        if d.0 == v.0 {
                            def = Some(inst);
                            break 'outer;
                        }
                    }
                }
            }
            let inst = def?;
            match inst {
                Instruction::Copy { src, .. } => resolve_const_operand(func, src, depth + 1),
                Instruction::Cast {
                    src,
                    from_ty,
                    to_ty,
                    ..
                } if is_fixed_width_int(*from_ty) && is_fixed_width_int(*to_ty) => {
                    // The source value is already in `from_ty`'s canonical
                    // form (sign- or zero-extended to i64 per its own
                    // signedness), so widening is the identity and narrowing
                    // is a truncation into `to_ty` — both are exactly
                    // `normalize_to_type`.
                    let v = resolve_const_operand(func, src, depth + 1)?;
                    let v = normalize_to_type(v, *from_ty)?;
                    normalize_to_type(v, *to_ty)
                }
                Instruction::BinOp {
                    op, lhs, rhs, ty, ..
                } if is_fixed_width_int(*ty) => {
                    let l = resolve_const_operand(func, lhs, depth + 1)?;
                    let r = resolve_const_operand(func, rhs, depth + 1)?;
                    let raw = match op {
                        IrBinOp::Add => l.wrapping_add(r),
                        IrBinOp::Sub => l.wrapping_sub(r),
                        IrBinOp::Mul => l.wrapping_mul(r),
                        _ => return None,
                    };
                    normalize_to_type(raw, *ty)
                }
                _ => None,
            }
        }
    }
}

/// Read a step constant of IV type `ty` as a SIGNED stride.  Unsigned IR
/// constants are stored zero-extended (`IrConst::I64(0xFFFFFFFE)` for a U32
/// `-2`); sign-extending them at the IV's width recovers the arithmetic
/// stride so that `for (unsigned i = n; i != 0; i += -2)` and `i -= 2` are
/// recognised as the same countdown.
fn signed_step(c: IrConst, ty: IrType) -> Option<i64> {
    let raw = c.to_i64()?;
    match ty {
        IrType::I8 | IrType::U8 => Some(raw as u8 as i8 as i64),
        IrType::I16 | IrType::U16 => Some(raw as u16 as i16 as i64),
        IrType::I32 | IrType::U32 => Some(raw as u32 as i32 as i64),
        IrType::I64 | IrType::U64 => Some(raw),
        _ => None,
    }
}

fn find_iv_in_loop(
    func: &IrFunction,
    header: usize,
    latch: usize,
    latch_label: BlockId,
) -> Option<(Value, IrType, i64, usize)> {
    for inst in &func.blocks[header].instructions {
        let (phi_dest, ty, incoming) = match inst {
            Instruction::Phi { dest, ty, incoming } if ty.is_integer() => (dest, ty, incoming),
            _ => continue,
        };

        // Value flowing into the header from the latch (the back-edge value).
        let back_val = incoming
            .iter()
            .find(|(_, lbl)| *lbl == latch_label)
            .and_then(|(op, _)| {
                if let Operand::Value(v) = op {
                    Some(*v)
                } else {
                    None
                }
            });
        let back_val = back_val?;

        // Look for `Add(phi_dest, const_step)` / `Add(const_step, phi_dest)`
        // or `Sub(phi_dest, const_step)` in the latch that produces
        // `back_val`.  `i -= k` lowers to a `Sub` — the countdown-loop
        // idiom of every decompressor and hash routine — and is the same
        // basic IV with step `-k`.  The step is expressed as a signed i64 of
        // the IV's width: `Sub(phi, 2)` on a U32 is the additive step
        // `0xFFFFFFFE`, i.e. `-2`, and every consumer (the closed-form trip
        // count, the per-clone `init + t*step` constants and the partial
        // unroller's `Add(iv, step)` checks) re-materialises it through
        // `IrConst::from_i64(step, iv_ty)`, so the two-complement identity
        // `x - k == x + (-k)` holds bit-exactly for every fixed-width type.
        let phi_id = phi_dest.0;
        for (idx, latch_inst) in func.blocks[latch].instructions.iter().enumerate() {
            let Instruction::BinOp {
                dest, op, lhs, rhs, ..
            } = latch_inst
            else {
                continue;
            };
            if *dest != back_val {
                continue;
            }
            let step = match (op, lhs, rhs) {
                (IrBinOp::Add, Operand::Value(v), Operand::Const(c)) if v.0 == phi_id => {
                    signed_step(*c, *ty)
                }
                (IrBinOp::Add, Operand::Const(c), Operand::Value(v)) if v.0 == phi_id => {
                    signed_step(*c, *ty)
                }
                (IrBinOp::Sub, Operand::Value(v), Operand::Const(c)) if v.0 == phi_id => {
                    signed_step(*c, *ty).and_then(i64::checked_neg)
                }
                _ => None,
            };
            if let Some(step) = step {
                return Some((*phi_dest, *ty, step, idx));
            }
        }
    }
    None
}

/// Detect the exit condition from the header's CondBranch terminator.
///
/// Returns `(exit_target, body_entry, cmp_op, cmp_ty, limit, iv_is_lhs, exit_cond_positive)`.
/// `exit_cond_positive` is `true` when the condition evaluating to `true` means "exit".
fn find_exit_condition(
    func: &IrFunction,
    header: usize,
    loop_body: &FxHashSet<usize>,
    iv_phi: Value,
) -> Option<(BlockId, BlockId, IrCmpOp, IrType, Operand, bool, bool)> {
    let header_block = &func.blocks[header];

    let (cond_op, true_label, false_label) = match &header_block.terminator {
        Terminator::CondBranch {
            cond,
            true_label,
            false_label,
        } => (*cond, *true_label, *false_label),
        _ => return None,
    };

    // Map labels to block indices for in-loop membership check.
    let label_to_idx: FxHashMap<BlockId, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label, i))
        .collect();

    let true_in_loop = label_to_idx
        .get(&true_label)
        .map(|&bi| loop_body.contains(&bi))
        .unwrap_or(false);
    let false_in_loop = label_to_idx
        .get(&false_label)
        .map(|&bi| loop_body.contains(&bi))
        .unwrap_or(false);

    // Exactly one branch must be in-loop, the other is the exit.
    if true_in_loop == false_in_loop {
        return None;
    }

    let (exit_target, body_entry, exit_cond_positive) = if !true_in_loop {
        (true_label, false_label, true)
    } else {
        (false_label, true_label, false)
    };

    // Trace the condition value to a Cmp instruction (through at most one Cast).
    let cond_id = match cond_op {
        Operand::Value(v) => v.0,
        _ => return None,
    };

    // Build a map of value-id → instruction for the header.
    let mut hdr_defs: FxHashMap<u32, &Instruction> = FxHashMap::default();
    for inst in &header_block.instructions {
        if let Some(dest) = inst.dest() {
            hdr_defs.insert(dest.0, inst);
        }
    }

    // Look through one Cast.
    let cmp_id = match hdr_defs.get(&cond_id) {
        Some(Instruction::Cast {
            src: Operand::Value(v),
            ..
        }) => v.0,
        _ => cond_id,
    };

    let (cmp_op, cmp_lhs, cmp_rhs, cmp_ty) = match hdr_defs.get(&cmp_id) {
        Some(Instruction::Cmp {
            op, lhs, rhs, ty, ..
        }) => (*op, *lhs, *rhs, *ty),
        _ => return None,
    };

    let iv_id = iv_phi.0;

    // One Cmp operand must be exactly the IV phi; the other must be loop-invariant.
    let (iv_is_lhs, limit_op) = if matches!(cmp_lhs, Operand::Value(v) if v.0 == iv_id)
        && is_loop_invariant_op(cmp_rhs, loop_body, func)
    {
        (true, cmp_rhs)
    } else if matches!(cmp_rhs, Operand::Value(v) if v.0 == iv_id)
        && is_loop_invariant_op(cmp_lhs, loop_body, func)
    {
        (false, cmp_lhs)
    } else {
        return None;
    };

    Some((
        exit_target,
        body_entry,
        cmp_op,
        cmp_ty,
        limit_op,
        iv_is_lhs,
        exit_cond_positive,
    ))
}

// ── CFG helpers ───────────────────────────────────────────────────────────────

fn is_loop_invariant_op(op: Operand, loop_body: &FxHashSet<usize>, func: &IrFunction) -> bool {
    match op {
        Operand::Const(_) => true,
        Operand::Value(v) => !is_defined_in_body(v.0, loop_body, func),
    }
}

fn is_defined_in_body(val_id: u32, loop_body: &FxHashSet<usize>, func: &IrFunction) -> bool {
    for &bi in loop_body {
        if bi < func.blocks.len() {
            for inst in &func.blocks[bi].instructions {
                if let Some(dest) = inst.dest() {
                    if dest.0 == val_id {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn block_has_succ(term: &Terminator, target: BlockId) -> bool {
    match term {
        Terminator::Branch(lbl) => *lbl == target,
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => *true_label == target || *false_label == target,
        _ => false,
    }
}

/// Replace `old` with `new` in one specific block-label slot of a terminator.
fn redirect_label(term: &mut Terminator, old: BlockId, new: BlockId) {
    match term {
        Terminator::Branch(lbl) if *lbl == old => *lbl = new,
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            if *true_label == old {
                *true_label = new;
            }
            if *false_label == old {
                *false_label = new;
            }
        }
        _ => {}
    }
}

/// Apply a block-label rename map to all branch targets in a terminator.
fn replace_block_ids(term: &mut Terminator, map: &FxHashMap<BlockId, BlockId>) {
    match term {
        Terminator::Branch(lbl) => {
            if let Some(&new) = map.get(lbl) {
                *lbl = new;
            }
        }
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            if let Some(&new) = map.get(true_label) {
                *true_label = new;
            }
            if let Some(&new) = map.get(false_label) {
                *false_label = new;
            }
        }
        Terminator::Switch { cases, default, .. } => {
            if let Some(&new) = map.get(default) {
                *default = new;
            }
            for (_, lbl) in cases {
                if let Some(&new) = map.get(lbl) {
                    *lbl = new;
                }
            }
        }
        _ => {}
    }
}

// ── Transformation ────────────────────────────────────────────────────────────

fn do_unroll(func: &mut IrFunction, c: UnrollCandidate) -> bool {
    let k = c.unroll_factor as usize; // total copies (1 original + k-1 clones)
    let num_new = k - 1; // number of clones = number of exit-check blocks
    if num_new == 0 {
        return false;
    }

    let header_label = func.blocks[c.header].label;
    let latch_label = func.blocks[c.latch].label;

    // ── Pre-allocate all new BlockIds and Values ──────────────────────────────
    let max_label = func.blocks.iter().map(|b| b.label.0).max().unwrap_or(0);
    let mut next_label = max_label + 1;
    let mut next_val = func.next_value_id;

    // iv_vals[j]    = %iv_{j+1}    (used in exit_check_{j+1} and clone[j])
    // cond_vals[j]  = %cond_{j+1}  (used in exit_check_{j+1})
    // ec_labels[j]  = label of exit_check_{j+1}
    // cl_labels[j]  = labels of clone[j]'s body_work blocks (parallel to body_work)
    let iv_vals: Vec<Value> = (0..num_new)
        .map(|_| {
            let v = Value(next_val);
            next_val += 1;
            v
        })
        .collect();
    let cond_vals: Vec<Value> = (0..num_new)
        .map(|_| {
            let v = Value(next_val);
            next_val += 1;
            v
        })
        .collect();
    let ec_labels: Vec<BlockId> = (0..num_new)
        .map(|_| {
            let l = BlockId(next_label);
            next_label += 1;
            l
        })
        .collect();
    let cl_labels: Vec<Vec<BlockId>> = (0..num_new)
        .map(|_| {
            (0..c.body_work.len())
                .map(|_| {
                    let l = BlockId(next_label);
                    next_label += 1;
                    l
                })
                .collect()
        })
        .collect();

    // Build value-rename maps for each clone.
    // clone_vmaps[j]: old_value_id → fresh_value_id, seeded with iv_phi → iv_vals[j].
    let mut clone_vmaps: Vec<FxHashMap<u32, u32>> = Vec::with_capacity(num_new);
    for j in 0..num_new {
        let mut vmap: FxHashMap<u32, u32> = FxHashMap::default();
        vmap.insert(c.iv_phi.0, iv_vals[j].0);
        for &bi in &c.body_work {
            for inst in &func.blocks[bi].instructions {
                if let Some(dest) = inst.dest() {
                    vmap.entry(dest.0).or_insert_with(|| {
                        let v = next_val;
                        next_val += 1;
                        v
                    });
                }
            }
        }
        clone_vmaps.push(vmap);
    }
    func.next_value_id = next_val;

    // ── Build new blocks (read-only access to func.blocks) ───────────────────
    //
    // Loop-carried phi threading: the IV phi is rethreaded through the
    // exit-check chain by construction (iv_vals), but ANY OTHER header phi
    // — a reduction accumulator, a marching pointer — must also advance one
    // step per unrolled iteration. Without threading, every clone reads the
    // phi's CURRENT value (the pre-iteration one), the clone's update
    // becomes dead, DCE deletes it, and the loop silently skips k-1 of
    // every k elements (lea_sib_fold: 45 18 instead of 45 25). Thread:
    //   clone j reads prev_j(H), where prev_0(H) = H's latch-incoming
    //   operand L0 (defined by the original body — the original body's
    //   update), and prev_{j+1}(H) = clone_j's renamed copy of L0; and the
    //   header phi's latch incoming becomes the last clone's copy.
    let mut carried: Vec<(u32, Operand, IrType)> = Vec::new(); // (phi id, L0, ty)
    for inst in &func.blocks[c.header].instructions {
        if let Instruction::Phi { dest, incoming, ty } = inst {
            if dest.0 == c.iv_phi.0 {
                continue;
            }
            if let Some((op, lbl)) = incoming.iter().find(|(_, l)| *l == latch_label) {
                if let Operand::Value(_) = op {
                    carried.push((dest.0, op.clone(), ty.clone()));
                }
            }
        }
    }

    let mut new_blocks: Vec<BasicBlock> = Vec::new();

    for j in 0..num_new {
        // The IV value feeding into this exit check:
        //   j=0: prev_iv = %iv_phi (the header phi)
        //   j>0: prev_iv = iv_vals[j-1]
        let prev_iv: Operand = if j == 0 {
            Operand::Value(c.iv_phi)
        } else {
            Operand::Value(iv_vals[j - 1])
        };

        let iv_j = iv_vals[j];
        let cond_j = cond_vals[j];

        // Entry of clone[j] (the block exit_check_{j+1} jumps into on "continue").
        let clone_entry = cl_labels[j][c.body_entry_work_idx];

        // ── Build exit_check_{j+1} ────────────────────────────────────────
        let cmp_lhs = if c.iv_is_lhs {
            Operand::Value(iv_j)
        } else {
            c.exit_limit
        };
        let cmp_rhs = if c.iv_is_lhs {
            c.exit_limit
        } else {
            Operand::Value(iv_j)
        };
        let (ec_true, ec_false) = if c.exit_cond_positive {
            (c.exit_target, clone_entry)
        } else {
            (clone_entry, c.exit_target)
        };

        new_blocks.push(BasicBlock {
            label: ec_labels[j],
            instructions: vec![
                Instruction::BinOp {
                    dest: iv_j,
                    op: IrBinOp::Add,
                    lhs: prev_iv,
                    rhs: Operand::Const(IrConst::from_i64(c.iv_step, c.iv_ty)),
                    ty: c.iv_ty,
                },
                Instruction::Cmp {
                    dest: cond_j,
                    op: c.exit_cmp_op,
                    lhs: cmp_lhs,
                    rhs: cmp_rhs,
                    ty: c.exit_cmp_ty,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(cond_j),
                true_label: ec_true,
                false_label: ec_false,
            },
            source_spans: Vec::new(),
        });

        // ── Build clone[j] (cloned body_work blocks) ──────────────────────
        // Block-label rename map for internal branches within this clone.
        let mut blk_map: FxHashMap<BlockId, BlockId> = FxHashMap::default();
        for (i, &bi) in c.body_work.iter().enumerate() {
            blk_map.insert(func.blocks[bi].label, cl_labels[j][i]);
        }

        // Where does clone[j]'s pre-latch block redirect after "latch"?
        //   j < num_new-1: → exit_check_{j+2}  (= ec_labels[j+1])
        //   j = num_new-1: → original latch     (no redirect)
        let post_latch_redirect: Option<BlockId> = if j + 1 < num_new {
            Some(ec_labels[j + 1])
        } else {
            None // last clone keeps going to original latch
        };

        let vmap = &clone_vmaps[j];
        for (i, &bi) in c.body_work.iter().enumerate() {
            let orig = &func.blocks[bi];

            // Reads of carried phis in this clone see the previous
            // iteration's update: L0 for clone 0 (the original body's
            // result), clone j-1's renamed copy of L0 for clone j > 0.
            let prev_carried: FxHashMap<u32, Operand> = carried
                .iter()
                .map(|(hid, l0, _ty)| {
                    let prev = if j == 0 {
                        l0.clone()
                    } else {
                        match l0 {
                            Operand::Value(lv) => match clone_vmaps[j - 1].get(&lv.0) {
                                Some(nv) => Operand::Value(Value(*nv)),
                                None => l0.clone(),
                            },
                            other => other.clone(),
                        }
                    };
                    (*hid, prev)
                })
                .collect();

            // Rename order matters: the clone's own vmap first (it maps
            // ORIGINAL body values, including L0, to this clone's copies —
            // substituting the threaded prev value before that would let
            // the vmap re-rename the injected value onto this clone's own
            // id, producing self-referencing adds and killing the original
            // body). The threaded prev values are final names from other
            // blocks; substitute them after, untouched by this clone's map.
            let new_insts: Vec<Instruction> = orig
                .instructions
                .iter()
                .map(|inst| {
                    let mut cloned = inst.clone();
                    replace_values_in_inst(&mut cloned, vmap);
                    for (hid, prev) in &prev_carried {
                        subst_value_with_operand(&mut cloned, *hid, prev);
                    }
                    rename_inst_dest(&mut cloned, vmap);
                    cloned
                })
                .collect();

            let mut new_term = orig.terminator.clone();
            replace_values_in_terminator(&mut new_term, vmap);
            replace_block_ids(&mut new_term, &blk_map);

            // Redirect latch edge from pre-latch block.
            if i == c.pre_latch_work_idx {
                if let Some(redirect_to) = post_latch_redirect {
                    redirect_label(&mut new_term, latch_label, redirect_to);
                }
                // else: last clone's pre-latch block stays pointing at original latch.
            }

            new_blocks.push(BasicBlock {
                label: cl_labels[j][i],
                instructions: new_insts,
                terminator: new_term,
                source_spans: Vec::new(),
            });
        }
    }

    // ── Mutate existing blocks ────────────────────────────────────────────────

    // Step 3: Redirect original body's pre-latch block from latch → exit_check_1.
    redirect_label(
        &mut func.blocks[c.body_work[c.pre_latch_work_idx]].terminator,
        latch_label,
        ec_labels[0],
    );

    // Step 4: Update latch's IV increment: swap iv_phi → iv_{K-1} (= iv_vals[num_new-1]).
    let last_iv = iv_vals[num_new - 1];
    if let Instruction::BinOp {
        op: IrBinOp::Add,
        lhs,
        rhs,
        ..
    } = &mut func.blocks[c.latch].instructions[c.latch_iv_incr_idx]
    {
        if matches!(lhs, Operand::Value(v) if v.0 == c.iv_phi.0) {
            *lhs = Operand::Value(last_iv);
        } else if matches!(rhs, Operand::Value(v) if v.0 == c.iv_phi.0) {
            *rhs = Operand::Value(last_iv);
        }
    }

    // Step 4b: retarget every carried header phi's latch incoming to the
    // last clone's copy of the original body's update (the latch now runs
    // after clone num_new-1, not after the original body).
    for (hid, l0, _ty) in &carried {
        if let Operand::Value(lv) = l0 {
            if let Some(nv) = clone_vmaps[num_new - 1].get(&lv.0) {
                for inst in &mut func.blocks[c.header].instructions {
                    if let Instruction::Phi { dest, incoming, .. } = inst {
                        if dest.0 == *hid {
                            for (op, lbl) in incoming.iter_mut() {
                                if *lbl == latch_label {
                                    *op = Operand::Value(Value(*nv));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Step 5: For any phi in the exit block that has an incoming from
    // header, add the value that phi would hold on each new exit-check
    // edge. Classification (this is NOT simply the header edge's operand —
    // on the path preheader→header→body→exit_check→exit the header phi took
    // its PREHEADER value, because its update rides the latch edge, which
    // the early exit never takes; using the header operand here silently
    // dropped the last executed body's contribution — lea_sib_fold printed
    // 18 instead of 25 for the final remainder element):
    //   * `Value(c.iv_phi)`: the check block itself computes iv_vals[j]
    //     (prev + step) BEFORE the exit compare, so that value is live.
    //   * `Value(v)` naming another header phi H (loop-carried
    //     accumulator): the would-be update is H's latch-incoming operand;
    //     on exit_check_j's edge the original body and clones 1..=j-1 have
    //     executed, so the live copy is that operand renamed by clone
    //     j-1's value map (unrenamed for j = 0 — the original body).
    //   * anything else (constants, loop-invariant values): identical on
    //     every path; reused as-is.
    if let Some(exit_bi) = func.blocks.iter().position(|b| b.label == c.exit_target) {
        // The header phis' latch-incoming operands, by value id.
        let latch_incoming: FxHashMap<u32, Operand> = func.blocks[c.header]
            .instructions
            .iter()
            .filter_map(|inst| {
                if let Instruction::Phi { dest, incoming, .. } = inst {
                    incoming
                        .iter()
                        .find(|(_, lbl)| *lbl == latch_label)
                        .map(|(op, _)| (dest.0, op.clone()))
                } else {
                    None
                }
            })
            .collect();

        // Collect (phi_index, header-incoming value) pairs.
        let phi_header_vals: Vec<(usize, Operand)> = func.blocks[exit_bi]
            .instructions
            .iter()
            .enumerate()
            .filter_map(|(phi_idx, inst)| {
                if let Instruction::Phi { incoming, .. } = inst {
                    incoming
                        .iter()
                        .find(|(_, lbl)| *lbl == header_label)
                        .map(|(op, _)| (phi_idx, *op))
                } else {
                    None
                }
            })
            .collect();

        for (phi_idx, op) in phi_header_vals {
            // Per-edge incoming for this exit phi.
            let edge_val = |j: usize| -> Operand {
                match op {
                    Operand::Value(v) if v == c.iv_phi => Operand::Value(iv_vals[j]),
                    Operand::Value(v) => {
                        match latch_incoming.get(&v.0) {
                            Some(l) if j == 0 => l.clone(),
                            Some(l) => {
                                // Rename through the last executed clone's map.
                                let mut renamed = l.clone();
                                if let Operand::Value(lv) = renamed {
                                    if let Some(nv) = clone_vmaps[j - 1].get(&lv.0) {
                                        renamed = Operand::Value(Value(*nv));
                                    }
                                }
                                renamed
                            }
                            // Not a header phi: loop-invariant on this path.
                            None => op.clone(),
                        }
                    }
                    other => other,
                }
            };
            for j in 0..num_new {
                let val = edge_val(j);
                if let Instruction::Phi { incoming, .. } =
                    &mut func.blocks[exit_bi].instructions[phi_idx]
                {
                    incoming.push((val, ec_labels[j]));
                }
            }
        }
    }

    // Step 5b: If the exit block has NO phi fed by the header for a
    // carried value (or the IV), readers after the loop still use the
    // header phi's dest SSA name directly. On the NEW exit-check edges
    // that name holds its PREHEADER value (the phi's update rides the
    // latch edge, which an early exit never takes), so every contribution
    // of the last executed body was silently dropped (lea_sib_fold printed
    // 45 18 instead of 45 25). Insert a proper exit phi — dest fresh, with
    // the header phi itself on the header edge and the threaded edge values
    // on the new check edges — and rewrite the post-exit readers to it.
    // (Earlier passes in the pipeline may already have lowered loop phis to
    // explicit edge copies, which is exactly the case this step heals.)
    {
        let loop_labels: FxHashSet<BlockId> = c
            .body_work
            .iter()
            .map(|&bi| func.blocks[bi].label)
            .chain(std::iter::once(header_label))
            .chain(std::iter::once(latch_label))
            .collect();
        let exit_idx = match func.blocks.iter().position(|b| b.label == c.exit_target) {
            Some(i) => i,
            None => {
                func.blocks.extend(new_blocks);
                return true;
            }
        };

        // Values needing an exit phi: the IV (edge value = the check
        // block's prev+step result) and every carried phi (edge value = the
        // latch incoming renamed by the last executed clone's map).
        let mut need: Vec<(Value, Vec<Operand>, IrType)> = Vec::new();
        {
            let mut iv_edges = Vec::with_capacity(num_new);
            for j in 0..num_new {
                iv_edges.push(Operand::Value(iv_vals[j]));
            }
            let iv_ty = func.blocks[c.header]
                .instructions
                .iter()
                .find_map(|inst| match inst {
                    Instruction::Phi { dest, ty, .. } if dest.0 == c.iv_phi.0 => Some(ty.clone()),
                    _ => None,
                })
                .unwrap_or(c.iv_ty.clone());
            need.push((c.iv_phi, iv_edges, iv_ty));
        }
        for (hid, l0, hty) in &carried {
            let mut edges = Vec::with_capacity(num_new);
            for j in 0..num_new {
                let ev = match l0 {
                    Operand::Value(lv) if j > 0 => match clone_vmaps[j - 1].get(&lv.0) {
                        Some(nv) => Operand::Value(Value(*nv)),
                        None => l0.clone(),
                    },
                    _ => l0.clone(),
                };
                edges.push(ev);
            }
            need.push((Value(*hid), edges, hty.clone()));
        }

        // Existing phis in the exit block that already take the header as a
        // predecessor, keyed by the header-edge operand's value id.
        let mut existing: FxHashMap<u32, ()> = FxHashMap::default();
        for inst in &func.blocks[exit_idx].instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                if let Some((Operand::Value(v), lbl)) =
                    incoming.iter().find(|(_, l)| *l == header_label)
                {
                    existing.insert(v.0, ());
                }
            }
        }

        // Predecessors the exit block already had that are NOT the header and
        // NOT one of the new exit-check blocks.  An exit block is very often a
        // control-flow JOIN — `if (cond) { for (...) ... }` merges the loop's
        // exit with the skip path — and such a block has predecessors that the
        // unroll knows nothing about.  A phi inserted here must still supply an
        // incoming for every one of them or it is malformed (the verifier
        // reports "phi vN has no incoming for predecessor BlockId(M)").
        //
        // On those edges the loop never ran, so the live value is exactly the
        // one that reached the exit before the unroll: `dest` itself (the
        // header phi's SSA name, which on a non-loop path still holds its
        // pre-loop definition).  That is the same operand used for the header
        // edge, so reuse it.
        let foreign_preds: Vec<BlockId> = {
            let exit_label = func.blocks[exit_idx].label;
            let ec_set: FxHashSet<BlockId> = ec_labels.iter().copied().collect();
            func.blocks
                .iter()
                .filter(|b| {
                    b.label != header_label
                        && !ec_set.contains(&b.label)
                        && crate::pgo::branch_prob::successors(&b.terminator).contains(&exit_label)
                })
                .map(|b| b.label)
                .collect()
        };

        for (dest, edges, pty) in &need {
            if existing.contains_key(&dest.0) {
                continue; // Step 5 already threaded this phi's edges
            }
            let new_phi = Value(next_val);
            next_val += 1;
            let mut incoming: Vec<(Operand, BlockId)> =
                Vec::with_capacity(num_new + 1 + foreign_preds.len());
            incoming.push((Operand::Value(*dest), header_label));
            for (j, ev) in edges.iter().enumerate() {
                incoming.push((ev.clone(), ec_labels[j]));
            }
            for fp in &foreign_preds {
                incoming.push((Operand::Value(*dest), *fp));
            }
            func.blocks[exit_idx].instructions.insert(
                0,
                Instruction::Phi {
                    dest: new_phi,
                    incoming,
                    ty: pty.clone(),
                },
            );

            // Rewrite readers of `dest` in the post-exit region: every block
            // reachable from the exit block without re-entering the loop.
            let mut stack: Vec<usize> = vec![exit_idx];
            let mut visited: FxHashSet<usize> = FxHashSet::from_iter([exit_idx]);
            while let Some(bi) = stack.pop() {
                let in_loop = loop_labels.contains(&func.blocks[bi].label);
                if !in_loop {
                    for inst in &mut func.blocks[bi].instructions {
                        if matches!(inst, Instruction::Phi { .. }) {
                            continue; // other blocks' phis select per-edge
                        }
                        subst_value_with_operand(inst, dest.0, &Operand::Value(new_phi));
                    }
                }
                let succs: Vec<BlockId> = match &func.blocks[bi].terminator {
                    Terminator::Branch(l) => vec![*l],
                    Terminator::CondBranch {
                        true_label,
                        false_label,
                        ..
                    } => {
                        vec![*true_label, *false_label]
                    }
                    _ => vec![],
                };
                for l in succs {
                    if loop_labels.contains(&l) {
                        continue;
                    }
                    if let Some(si) = func.blocks.iter().position(|b| b.label == l) {
                        if visited.insert(si) {
                            stack.push(si);
                        }
                    }
                }
            }
        }
        func.next_value_id = next_val;
    }

    // Step 6: Append all new blocks.
    func.blocks.extend(new_blocks);

    true
}

// ── Value-replacement helpers (adapted from tail_call_elim.rs) ────────────────

/// Rename the SSA *definition* site (dest) of an instruction using `map`.
/// Only variants that produce an SSA value are affected; others are a no-op.
pub(crate) fn rename_inst_dest(inst: &mut Instruction, map: &FxHashMap<u32, u32>) {
    match inst {
        Instruction::PgoCounterInc { .. } => {}
        // Nested-function support: GetStaticChain defines a dest; the others
        // define nothing.
        Instruction::SetStaticChain { .. }
        | Instruction::InitTrampoline { .. }
        | Instruction::NonlocalGotoSave { .. }
        | Instruction::NonlocalGoto { .. } => {}
        Instruction::Alloca { dest, .. }
        | Instruction::DynAlloca { dest, .. }
        | Instruction::Load { dest, .. }
        | Instruction::BinOp { dest, .. }
        | Instruction::UnaryOp { dest, .. }
        | Instruction::Cmp { dest, .. }
        | Instruction::GetElementPtr { dest, .. }
        | Instruction::Cast { dest, .. }
        | Instruction::Copy { dest, .. }
        | Instruction::GlobalAddr { dest, .. }
        | Instruction::VaArg { dest, .. }
        | Instruction::AtomicRmw { dest, .. }
        | Instruction::AtomicCmpxchg { dest, .. }
        | Instruction::AtomicLoad { dest, .. }
        | Instruction::Phi { dest, .. }
        | Instruction::LabelAddr { dest, .. }
        | Instruction::GetReturnF64Second { dest }
        | Instruction::GetReturnF32Second { dest }
        | Instruction::GetReturnF128Second { dest }
        | Instruction::Select { dest, .. }
        | Instruction::StackSave { dest }
        | Instruction::GetStaticChain { dest }
        | Instruction::ParamRef { dest, .. } => replace_val(dest, map),

        Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
            if let Some(dest) = &mut info.dest {
                replace_val(dest, map);
            }
        }

        Instruction::Intrinsic { dest, .. } => {
            if let Some(dest) = dest {
                replace_val(dest, map);
            }
        }

        // No SSA destination.
        Instruction::Store { .. }
        | Instruction::Memcpy { .. }
        | Instruction::VaArgStruct { .. }
        | Instruction::VaStart { .. }
        | Instruction::VaEnd { .. }
        | Instruction::VaCopy { .. }
        | Instruction::AtomicStore { .. }
        | Instruction::AtomicInc { .. }
        | Instruction::Fence { .. }
        | Instruction::SetReturnF64Second { .. }
        | Instruction::SetReturnF32Second { .. }
        | Instruction::SetReturnF128Second { .. }
        | Instruction::InlineAsm { .. }
        | Instruction::StackRestore { .. } => {}
    }
}

#[inline]
fn replace_val(v: &mut Value, map: &FxHashMap<u32, u32>) {
    if let Some(&new_id) = map.get(&v.0) {
        *v = Value(new_id);
    }
}

#[inline]
fn replace_op(op: &mut Operand, map: &FxHashMap<u32, u32>) {
    if let Operand::Value(v) = op {
        replace_val(v, map);
    }
}

fn replace_values_in_inst(inst: &mut Instruction, map: &FxHashMap<u32, u32>) {
    inst.for_each_operand_mut(|operand| replace_op(operand, map));
    inst.for_each_value_use_mut(|value| replace_val(value, map));
}

fn replace_values_in_terminator(term: &mut Terminator, map: &FxHashMap<u32, u32>) {
    term.for_each_operand_mut(|operand| replace_op(operand, map));
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::{AddressSpace, IrType};
    use crate::ir::reexports::{AtomicOrdering, AtomicRmwOp, BasicBlock, BlockId, IrConst, Value};

    /// Historical truth table entry point: the closed form on a signed
    /// 64-bit IV (the domain the pre-typed arithmetic implicitly assumed).
    fn trip_i64(init: i64, limit: i64, op: IrCmpOp, step: i64) -> Option<i64> {
        complete_unroll_trip(init, limit, op, step, IrType::I64)
    }

    /// Build a simple counting loop:
    ///   preheader → header → body → latch → (back to header) / exit
    ///
    /// ```
    /// preheader (B0):
    ///   %0 = Copy 0i32
    ///   Branch B1
    ///
    /// header (B1):
    ///   %1 = Phi [(%0, B0), (%5, B3)]   // i
    ///   %3 = Cmp Slt %1, const(n_val)   // limit is a compile-time constant
    ///   CondBranch %3, B2(body), B4(exit)
    ///
    /// body (B2):
    ///   %4 = GEP(arr, %1)
    ///   Store(0, %4)
    ///   Branch B3
    ///
    /// latch (B3):
    ///   %5 = Add %1, 1
    ///   Branch B1
    ///
    /// exit (B4):
    ///   Return void
    /// ```
    ///
    /// The limit is a constant so it is loop-invariant (not defined in loop.body).
    fn make_counting_loop(n_val: i32) -> IrFunction {
        let mut func = IrFunction::new("loop_test".to_string(), IrType::Void, vec![], false);

        // B0: preheader — init i = 0
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Copy {
                dest: Value(0),
                src: Operand::Const(IrConst::I32(0)),
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });

        // B1: header — %1 = phi(0, %5); %3 = cmp %1 < const(n_val)
        // Limit is a constant → loop-invariant → eligible for unrolling.
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(1),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(5)), BlockId(3)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(3),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(n_val)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(3)),
                true_label: BlockId(2),  // continue (body)
                false_label: BlockId(4), // exit
            },
            source_spans: Vec::new(),
        });

        // B2: body — GEP + store. The GEP uses a constant offset (not the IV)
        // so the loop stays eligible under the IV-widening guard, which only
        // rejects GEPs that index directly by the narrow IV.
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::GetElementPtr {
                    dest: Value(4),
                    base: Value(10), // arr (loop-invariant, defined outside)
                    offset: Operand::Const(IrConst::I32(0)),
                    ty: IrType::I32,
                },
                Instruction::Store {
                    volatile: false,
                    val: Operand::Const(IrConst::I32(0)),
                    ptr: Value(4),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
            ],
            terminator: Terminator::Branch(BlockId(3)), // → latch
            source_spans: Vec::new(),
        });

        // B3: latch — %5 = %1 + 1; Branch B1
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::BinOp {
                dest: Value(5),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });

        // B4: exit
        func.blocks.push(BasicBlock {
            label: BlockId(4),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });

        func.next_value_id = 11; // 0–10 used (10 = arr placeholder)
        func
    }

    /// `make_counting_loop`, but the loop is entered CONDITIONALLY and the
    /// exit block is a join carrying a phi.
    ///
    /// This is the shape the plain `make_counting_loop` cannot express and
    /// therefore could not test: with an unconditional loop the exit block has
    /// a single predecessor and no phi at all, so nothing in the exit block
    /// records which block reaches it.  Once the exit block DOES carry a phi,
    /// the complete unroller's CFG rewrite must keep that phi's predecessor
    /// labels in sync:
    ///
    ///   * the `header -> exit` edge disappears (the header now branches into
    ///     the clone chain), so the header-labelled incoming must be
    ///     relabelled to the LAST CLONE, which is the block that actually
    ///     reaches the exit; and
    ///   * the latch must NOT be given a fabricated `latch -> exit` edge — its
    ///     only successor was the header, so after the rewrite it is
    ///     unreachable.
    ///
    /// Layout:
    ///   B0 preheader -> cond ? B1 (loop header) : B5 (skip)
    ///   B1 header -> B2 body -> B3 latch -> B1,  header exits to B4
    ///   B5 skip   -> B4
    ///   B4 exit: phi [ Value(20) from B1 (header), Value(21) from B5 ]
    fn make_conditional_counting_loop_with_exit_phi(n_val: i32) -> IrFunction {
        make_conditional_counting_loop_with_exit_phi_impl(n_val, true)
    }

    fn make_conditional_counting_loop_with_exit_phi_impl(
        n_val: i32,
        const_iv_init: bool,
    ) -> IrFunction {
        let mut func = make_counting_loop(n_val);

        // `try_complete_unroll_two_block` requires a LITERAL constant IV init
        // on the non-latch incoming (it constant-folds the trip count before
        // it will touch the CFG).  `make_counting_loop` feeds the phi
        // `Value(0)` -- a Copy of 0 in the preheader -- which routes the
        // fixture to the partial unroller instead.  Use the constant directly
        // so BOTH complete-unroll and partial-unroll paths are exercised.
        if const_iv_init {
        if let Some(Instruction::Phi { incoming, .. }) = func.blocks[1]
            .instructions
            .iter_mut()
            .find(|i| matches!(i, Instruction::Phi { dest, .. } if dest.0 == 1))
        {
            for (op, lbl) in incoming.iter_mut() {
                if *lbl == BlockId(0) {
                    *op = Operand::Const(IrConst::I32(0));
                }
            }
        }
        }

        // B0 becomes a conditional dispatch into the loop or around it.
        func.blocks[0].instructions.push(Instruction::Cmp {
            dest: Value(11),
            op: IrCmpOp::Slt,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Const(IrConst::I32(1)),
            ty: IrType::I32,
        });
        func.blocks[0].terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(11)),
            true_label: BlockId(1),
            false_label: BlockId(5),
        };

        // Two values that reach the join from the two paths.  v20 is defined
        // in the PREHEADER, not the header: a header's phis must remain the
        // first instructions in the block, so a Copy inserted ahead of them
        // would itself be a (different) structural violation.
        func.blocks[0].instructions.push(Instruction::Copy {
            dest: Value(20),
            src: Operand::Const(IrConst::I32(7)),
        });

        // B5: the skip path.
        func.blocks.push(BasicBlock {
            label: BlockId(5),
            instructions: vec![Instruction::Copy {
                dest: Value(21),
                src: Operand::Const(IrConst::I32(9)),
            }],
            terminator: Terminator::Branch(BlockId(4)),
            source_spans: Vec::new(),
        });

        // B4 (exit) gains the join phi.
        let exit_bi = func
            .blocks
            .iter()
            .position(|b| b.label == BlockId(4))
            .expect("exit block");
        func.blocks[exit_bi].instructions.insert(
            0,
            Instruction::Phi {
                dest: Value(22),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Value(Value(20)), BlockId(1)),
                    (Operand::Value(Value(21)), BlockId(5)),
                ],
            },
        );
        func.blocks[exit_bi].terminator = Terminator::Return(Some(Operand::Value(Value(22))));

        func.next_value_id = 23;
        func
    }

    /// Sibling of `complete_unroll_repairs_exit_block_phi_labels` for the
    /// PARTIAL unroller (`do_unroll`).
    ///
    /// Routing: `try_complete_unroll_two_block` only fires when the IV's
    /// non-latch incoming is a literal `Const` (it folds the trip count before
    /// touching the CFG).  Leaving the fixture's `Copy v0 = 0` preheader init
    /// in place therefore sends the loop down the partial-unroll path instead,
    /// which is a genuinely different rewrite: it builds a chain of
    /// exit-check blocks, each of which becomes a NEW predecessor of the exit
    /// block, and synthesises fresh exit phis for the IV and every carried
    /// value (Step 5b).
    ///
    /// Those synthesised phis used to list only the header edge and the new
    /// exit-check edges.  When the exit block is a JOIN — `if (cond) { for
    /// (...) ... }`, the overwhelmingly common shape — it also has
    /// predecessors from outside the loop entirely, and the new phi had no
    /// incoming for any of them:
    ///
    ///   phi v44 has no incoming for predecessor BlockId(5)
    ///
    /// On such an edge the loop never executed, so the correct value is the
    /// one that reached the exit before the unroll: the header phi's own SSA
    /// name.
    #[test]
    fn partial_unroll_exit_phi_covers_preexisting_predecessors() {
        // Trip counts large enough that complete unrolling would be refused
        // anyway (>16), so this exercises `do_unroll` even if the routing
        // condition above is ever relaxed.
        for trip in [40i32, 100, 257] {
            let mut func = make_conditional_counting_loop_with_exit_phi_impl(trip, false);
            unroll_loops(&mut func);

            let mut violations = Vec::new();
            crate::passes::verify::verify_function(&func, "unroll_loops", &mut violations);
            assert!(
                violations.is_empty(),
                "trip={trip} left malformed IR after partial unrolling:\n{}",
                violations
                    .iter()
                    .map(|v| format!("  {v}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
    }

    /// Regression: complete unrolling must repair the EXIT block's phi
    /// predecessor labels.
    ///
    /// Found building linux-cachymod 6.18.47 with lccc:
    /// `drivers/gpu/drm/i915/display/intel_sprite.c`'s `vlv_sprite_update_gamma`
    /// (`for (i = 1; i < 8 - 1; i++)`) inlined into `vlv_sprite_update_arm`.
    /// `CCC_VERIFY_IR=1` reported the exit phi naming the header — no longer a
    /// predecessor — while the real predecessor had no incoming.  The value
    /// then reached x86 ISel with no register home and no stack slot and
    /// `operand_to_rax`'s hard gate aborted the build.  Before that gate
    /// existed the same shape silently emitted `xorl %eax,%eax`: a miscompile.
    ///
    /// The trip counts below all take the 2-or-3-block complete-unroll path
    /// (`try_complete_unroll_two_block`); 6 is the i915 loop's own trip count.
    #[test]
    fn complete_unroll_repairs_exit_block_phi_labels() {
        for trip in [2i32, 3, 4, 6, 8] {
            let mut func = make_conditional_counting_loop_with_exit_phi(trip);
            unroll_loops(&mut func);

            let mut violations = Vec::new();
            crate::passes::verify::verify_function(&func, "unroll_loops", &mut violations);
            assert!(
                violations.is_empty(),
                "trip={trip} left malformed IR after unrolling:\n{}",
                violations
                    .iter()
                    .map(|v| format!("  {v}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );

            // Directly assert the two facts the verifier encodes, so a future
            // weakening of the verifier cannot silently un-cover this bug.
            let succs: Vec<(BlockId, Vec<BlockId>)> = func
                .blocks
                .iter()
                .map(|b| (b.label, crate::pgo::branch_prob::successors(&b.terminator)))
                .collect();
            let exit = func
                .blocks
                .iter()
                .find(|b| b.label == BlockId(4))
                .expect("exit block survives");
            let real_preds: Vec<BlockId> = succs
                .iter()
                .filter(|(_, ss)| ss.contains(&BlockId(4)))
                .map(|(l, _)| *l)
                .collect();
            for inst in &exit.instructions {
                let Instruction::Phi { dest, incoming, .. } = inst else {
                    continue;
                };
                for (_, lbl) in incoming {
                    assert!(
                        real_preds.contains(lbl),
                        "trip={trip}: exit phi v{} names {:?}, not a predecessor \
                         (real predecessors: {:?})",
                        dest.0,
                        lbl,
                        real_preds
                    );
                }
                for pred in &real_preds {
                    assert!(
                        incoming.iter().any(|(_, l)| l == pred),
                        "trip={trip}: exit phi v{} has no incoming for predecessor {:?}",
                        dest.0,
                        pred
                    );
                }
            }
        }
    }

    #[test]
    fn test_basic_unroll_8x() {
        let mut func = make_counting_loop(100);
        let n = unroll_loops(&mut func);
        assert_eq!(n, 1, "should unroll exactly one loop");

        // Original 5 blocks + 7 exit_check blocks + 7 body_work clones = 19.
        assert_eq!(
            func.blocks.len(),
            19,
            "expected 5 original + 7 exit_checks + 7 clones = 19 blocks"
        );

        // The latch's Add should now use one of the new IV values (not Value(1)).
        let latch = func.blocks.iter().find(|b| b.label == BlockId(3)).unwrap();
        let iv_incr = latch
            .instructions
            .iter()
            .find(|i| {
                matches!(
                    i,
                    Instruction::BinOp {
                        op: IrBinOp::Add,
                        ..
                    }
                )
            })
            .unwrap();
        if let Instruction::BinOp { lhs, .. } = iv_incr {
            assert!(
                !matches!(lhs, Operand::Value(v) if v.0 == 1),
                "latch IV increment should use iv_7 (not original iv_phi Value(1))"
            );
        }
    }

    #[test]
    fn test_unroll_iv_indexed_gep_is_legal() {
        // A body that GEPs directly by the narrow (I32) IV IS unrollable:
        // do_unroll clones the GEP through the per-clone value map, so each
        // clone indexes by its own IV copy. (The historical blanket rejection
        // was removed after differential validation against GCC; only
        // Cast(I32->I64/Ptr) widening of the IV remains a hazard, covered by
        // test_no_unroll_iv_widening_cast below.)
        let mut func = make_counting_loop(100);
        for inst in &mut func.blocks[2].instructions {
            if let Instruction::GetElementPtr { offset, .. } = inst {
                *offset = Operand::Value(Value(1)); // index by the IV
            }
        }
        let n = unroll_loops(&mut func);
        assert_eq!(
            n, 1,
            "IV-indexed GEP loop should be unrolled (per-clone remap)"
        );
    }

    #[test]
    fn test_no_unroll_iv_widening_cast() {
        // A body that widens the narrow IV via Cast(I32->I64) must NOT be
        // unrolled on 64-bit targets: the unroller's intermediate IV values
        // stay narrow and the widened uses can interact incorrectly with
        // later passes (observed in the SQLite amalgamation).
        let mut func = make_counting_loop(100);
        func.blocks[2].instructions.insert(
            0,
            Instruction::Cast {
                dest: Value(90),
                src: Operand::Value(Value(1)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
        );
        let n = unroll_loops(&mut func);
        if crate::common::types::target_is_32bit() {
            assert_eq!(n, 1, "32-bit targets have no widening hazard");
        } else {
            assert_eq!(n, 0, "IV-widening-cast loop should not be unrolled");
        }
    }

    #[test]
    fn test_no_unroll_call_in_body() {
        let mut func = make_counting_loop(100);
        // Insert a Call instruction into the body (B2).
        func.blocks[2].instructions.push(Instruction::Call {
            func: "some_func".to_string(),
            info: crate::ir::reexports::CallInfo {
                dest: None,
                args: vec![],
                arg_types: vec![],
                return_type: IrType::Void,
                is_variadic: false,
                num_fixed_args: 0,
                struct_arg_sizes: vec![],
                struct_arg_aligns: vec![],
                struct_arg_classes: vec![],
                struct_arg_riscv_float_classes: vec![],
                struct_arg_is_f128_sse: Vec::new(),
                is_sret: false,
                is_fastcall: false,
                is_pure: false,
                is_const: false,
                ret_eightbyte_classes: vec![],
                ret_is_f128_sse: false,
            },
        });
        let n = unroll_loops(&mut func);
        assert_eq!(n, 0, "loop with call should not be unrolled");
        assert_eq!(func.blocks.len(), 5, "block count should be unchanged");
    }

    #[test]
    fn test_no_unroll_large_body() {
        // Build a loop whose body has > 60 instructions → factor = 1 → no unroll.
        let mut func = make_counting_loop(100);
        // Pad body (B2) with NOPs (Copy %0 = %0) until > 60 instructions.
        for _ in 0..65 {
            func.blocks[2].instructions.push(Instruction::Copy {
                dest: Value(0),
                src: Operand::Value(Value(0)),
            });
        }
        let n = unroll_loops(&mut func);
        assert_eq!(
            n, 0,
            "loop with > 60 body instructions should not be unrolled"
        );
    }

    #[test]
    fn test_no_unroll_no_preheader() {
        // Make the header have two entry predecessors (no unique preheader).
        let mut func = make_counting_loop(100);
        // Add a second predecessor to the header (B1) from B4 (exit).
        func.blocks[4].terminator = Terminator::Branch(BlockId(1));
        // Also extend B1's phi to include B4.
        if let Instruction::Phi { incoming, .. } = &mut func.blocks[1].instructions[0] {
            incoming.push((Operand::Value(Value(0)), BlockId(4)));
        }
        let n = unroll_loops(&mut func);
        assert_eq!(n, 0, "loop without unique preheader should not be unrolled");
    }

    #[test]
    fn test_no_unroll_nested_loop_outer() {
        // The outer loop's body_work contains the inner loop's header —
        // the outer loop must NOT be unrolled, but the inner loop IS unrolled.
        //
        // Structure:
        //   B0 (outer preheader) → B1 (outer header)
        //   B1: %i = phi, cmp i < 10 → B2(inner hdr) or B6(outer exit)
        //   B2 (inner header): %j = phi, cmp j < 10 → B2b(inner body) or B5(outer latch)
        //   B2b (inner body): a Copy instruction → B3(inner latch)
        //   B3 (inner latch): %j_next = j+1 → B2 (back-edge)
        //   B5 (outer latch): %i_next = i+1 → B1 (back-edge)
        //   B6 (outer exit): Return
        //
        // Inner loop: {B2, B2b, B3}, body_work={B2b}, header=B2, latch=B3 → can unroll.
        // Outer loop: {B1, B2, B2b, B3, B5}, body_work={B2, B2b, B3}, header=B1, latch=B5
        //   → body_work contains B2 which is a loop header → outer NOT unrolled.
        let mut func = IrFunction::new("nested".to_string(), IrType::Void, vec![], false);

        // B0: outer preheader
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Copy {
                dest: Value(0),
                src: Operand::Const(IrConst::I32(0)),
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });

        // B1: outer header — %1 = phi(%0, %10); cmp %1 < 10
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(1),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(10)), BlockId(5)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(10)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(2)),
                true_label: BlockId(2),  // inner header
                false_label: BlockId(6), // outer exit
            },
            source_spans: Vec::new(),
        });

        // B2: inner header — %3 = phi(%1, %7); cmp %3 < 10
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(3),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(1)), BlockId(1)),
                        (Operand::Value(Value(7)), BlockId(3)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(4),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(3)),
                    rhs: Operand::Const(IrConst::I32(10)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(4)),
                true_label: BlockId(20), // inner body (B2b)
                false_label: BlockId(5), // outer latch (inner exit)
            },
            source_spans: Vec::new(),
        });

        // B2b (BlockId 20): inner body — a single Copy; branches to inner latch
        func.blocks.push(BasicBlock {
            label: BlockId(20),
            instructions: vec![Instruction::Copy {
                dest: Value(20),
                src: Operand::Const(IrConst::I32(0)),
            }],
            terminator: Terminator::Branch(BlockId(3)), // → inner latch
            source_spans: Vec::new(),
        });

        // B3: inner latch — %7 = %3+1; back to inner header
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::BinOp {
                dest: Value(7),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(3)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            }],
            terminator: Terminator::Branch(BlockId(2)), // back to inner header
            source_spans: Vec::new(),
        });

        // B5: outer latch — %10 = %1+1; back to outer header
        func.blocks.push(BasicBlock {
            label: BlockId(5),
            instructions: vec![Instruction::BinOp {
                dest: Value(10),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            }],
            terminator: Terminator::Branch(BlockId(1)), // back to outer header
            source_spans: Vec::new(),
        });

        // B6: outer exit
        func.blocks.push(BasicBlock {
            label: BlockId(6),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });

        func.next_value_id = 21;

        let n = unroll_loops(&mut func);

        // The GENERAL complete unroller may now also fully unroll this outer
        // loop (constant trip 10, tiny body, inner loop cloned wholesale —
        // exactly the triangular-cascade shape it exists for). The pass
        // contract that MUST hold either way:
        //  1. the inner loop is unrolled (n >= 1), and
        //  2. if the outer was complete-unrolled, its original latch now
        //     branches to the first clone (NOT back to the outer header —
        //     a surviving back-edge would be an infinite loop), and the
        //     exit block is still reachable and still Returns.
        assert!(n >= 1, "the inner loop must be unrolled");
        let outer_latch = func.blocks.iter().find(|b| b.label == BlockId(5)).unwrap();
        match &outer_latch.terminator {
            Terminator::Branch(lbl) if *lbl == BlockId(1) => {
                // Outer NOT unrolled (pre-general-unroller behaviour).
            }
            Terminator::Branch(_) => {
                // Outer complete-unrolled: latch must feed the clone chain,
                // and the exit block must remain a returning block.
                let exit = func
                    .blocks
                    .iter()
                    .find(|b| matches!(b.terminator, Terminator::Return(None)))
                    .expect("outer exit must still exist and return");
                let _ = exit;
            }
            other => panic!("outer latch terminator corrupted: {:?}", other),
        }
    }

    #[test]
    fn substitution_covers_twenty_five_non_algebraic_use_positions() {
        let mut instructions = vec![
            Instruction::Memcpy {
                dest: Value(1),
                src: Value(2),
                size: 8,
            },
            Instruction::AtomicCmpxchg {
                dest: Value(100),
                ptr: Operand::Value(Value(3)),
                expected: Operand::Value(Value(4)),
                desired: Operand::Value(Value(5)),
                ty: IrType::I32,
                success_ordering: AtomicOrdering::SeqCst,
                failure_ordering: AtomicOrdering::SeqCst,
                returns_bool: true,
            },
            Instruction::VaArgStruct {
                dest_ptr: Value(6),
                va_list_ptr: Value(7),
                size: 8,
                align: 8,
                eightbyte_classes: vec![],
            },
            Instruction::VaCopy {
                dest_ptr: Value(8),
                src_ptr: Value(9),
            },
            Instruction::AtomicRmw {
                dest: Value(101),
                op: AtomicRmwOp::Add,
                ptr: Operand::Value(Value(10)),
                val: Operand::Value(Value(11)),
                ty: IrType::I32,
                ordering: AtomicOrdering::SeqCst,
            },
            Instruction::AtomicStore {
                ptr: Operand::Value(Value(12)),
                val: Operand::Value(Value(13)),
                ty: IrType::I32,
                ordering: AtomicOrdering::SeqCst,
            },
            Instruction::Phi {
                dest: Value(102),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Value(Value(14)), BlockId(1)),
                    (Operand::Value(Value(15)), BlockId(2)),
                ],
            },
            Instruction::VaArg {
                dest: Value(103),
                va_list_ptr: Value(16),
                result_ty: IrType::I32,
            },
            Instruction::VaStart {
                va_list_ptr: Value(17),
            },
            Instruction::VaEnd {
                va_list_ptr: Value(18),
            },
            Instruction::SetReturnF64Second {
                src: Operand::Value(Value(19)),
            },
            Instruction::SetReturnF32Second {
                src: Operand::Value(Value(20)),
            },
            Instruction::SetReturnF128Second {
                src: Operand::Value(Value(21)),
            },
            Instruction::AtomicInc {
                ptr: Operand::Value(Value(22)),
                offset: 0,
                ty: IrType::I32,
                ordering: AtomicOrdering::SeqCst,
            },
            Instruction::AtomicLoad {
                dest: Value(104),
                ptr: Operand::Value(Value(23)),
                ty: IrType::I32,
                ordering: AtomicOrdering::SeqCst,
            },
        ];
        for instruction in &mut instructions {
            for old in 1..=23 {
                subst_value_with_operand(instruction, old, &Operand::Value(Value(old + 1000)));
            }
        }
        let mut uses: Vec<u32> = instructions
            .iter()
            .flat_map(Instruction::used_values)
            .collect();
        uses.sort_unstable();
        assert_eq!(uses, (1001..=1023).collect::<Vec<_>>());

        let mut terminators = [
            Terminator::Return(Some(Operand::Value(Value(24)))),
            Terminator::CondBranch {
                cond: Operand::Value(Value(25)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
        ];
        subst_value_in_terminator(&mut terminators[0], 24, &Operand::Value(Value(1024)));
        subst_value_in_terminator(&mut terminators[1], 25, &Operand::Value(Value(1025)));
        assert_eq!(terminators[0].used_values(), vec![1024]);
        assert_eq!(terminators[1].used_values(), vec![1025]);
    }

    #[test]
    fn test_value_ids_unique_after_unroll() {
        // After unrolling, all Value IDs must be distinct (no duplicates in all
        // block instructions). This catches the "reuse old val IDs" bug.
        let mut func = make_counting_loop(16);
        unroll_loops(&mut func);

        let mut seen: FxHashSet<u32> = FxHashSet::default();
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Some(dest) = inst.dest() {
                    assert!(
                        seen.insert(dest.0),
                        "duplicate Value({}) after unrolling",
                        dest.0
                    );
                }
            }
        }
    }

    /// Unrolling must leave STRUCTURALLY VALID IR, not merely IR that happens
    /// to produce the right answer once DCE has swept up after it.
    ///
    /// `try_complete_unroll_two_block` used to rewrite only the loop-carried
    /// phis (those in `final_map`) into Copies and leave the induction
    /// variable's phi untouched. Once the loop is straight-line that phi is
    /// malformed twice over: it still names the latch as a predecessor, and
    /// because the carried phis ahead of it have become Copies it now sits
    /// after a non-phi instruction. `try_complete_unroll_general` always got
    /// this right; the two paths had diverged.
    ///
    /// This is asserted with the real IR verifier rather than a hand-rolled
    /// check so any future structural invariant is enforced here for free.
    #[test]
    fn complete_unroll_leaves_structurally_valid_ir() {
        for trip in [2i32, 4, 8, 16] {
            let mut func = make_counting_loop(trip);
            unroll_loops(&mut func);

            let mut violations = Vec::new();
            crate::passes::verify::verify_function(&func, "unroll_loops", &mut violations);
            assert!(
                violations.is_empty(),
                "trip={} left malformed IR after unrolling:\n{}",
                trip,
                violations
                    .iter()
                    .map(|v| format!("  {}", v))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
    }

    // ── complete_unroll_trip truth table ─────────────────────────────────
    // Exhaustive closed-form coverage of the constant-stride trip count:
    // the historical step-1 arithmetic must reduce verbatim, and every
    // stride>1 / negative-stride arm must match a reference loop simulation.
    #[test]
    fn unroll_trip_step_one_matches_historical_arithmetic() {
        use crate::ir::reexports::IrCmpOp::*;
        // i < 8, step 1  → 8 trips (classic 0..8).
        assert_eq!(trip_i64(0, 8, Slt, 1), Some(8));
        // i <= 7, step 1 → 8 trips.
        assert_eq!(trip_i64(0, 7, Sle, 1), Some(8));
        // Non-zero init.
        assert_eq!(trip_i64(3, 8, Slt, 1), Some(5));
        // Empty / inverted spans stay rejected (pre-existing guards).
        assert_eq!(trip_i64(8, 8, Slt, 1), None);
        assert_eq!(trip_i64(9, 8, Slt, 1), None);
        assert_eq!(trip_i64(8, 7, Sle, 1), None);
    }

    #[test]
    fn unroll_trip_positive_strides() {
        use crate::ir::reexports::IrCmpOp::*;
        // dot8's shape: i < 8, i += 4 → exactly 2 trips.
        assert_eq!(trip_i64(0, 8, Slt, 4), Some(2));
        // Non-divisible Slt: ceil. i < 10, i += 3 → 0,3,6,9 → 4 trips.
        assert_eq!(trip_i64(0, 10, Slt, 3), Some(4));
        // Odd limit: i < 7, i += 2 → 0,2,4,6 → 4 trips.
        assert_eq!(trip_i64(0, 7, Slt, 2), Some(4));
        // Sle floor+1: i <= 9, i += 3 → 0,3,6,9 → 4 trips.
        assert_eq!(trip_i64(0, 9, Sle, 3), Some(4));
        // Sle non-divisible: i <= 10, i += 3 → 0,3,6,9 → 4 trips.
        assert_eq!(trip_i64(0, 10, Sle, 3), Some(4));
        // Single iteration: i < 5, i += 5 → 1 (callers reject trip < 2).
        assert_eq!(trip_i64(0, 5, Slt, 5), Some(1));
        // Non-zero init with stride: i = 2; i < 11; i += 3 → 2,5,8 → 3.
        assert_eq!(trip_i64(2, 11, Slt, 3), Some(3));
    }

    #[test]
    fn unroll_trip_negative_strides_countdown() {
        use crate::ir::reexports::IrCmpOp::*;
        // for (i = 10; i > 0; i -= 2) → 10,8,6,4,2 → 5 trips.
        assert_eq!(trip_i64(10, 0, Sgt, -2), Some(5));
        // Non-divisible countdown: i > 0, i -= 3 → 10,7,4,1 → 4 (ceil).
        assert_eq!(trip_i64(10, 0, Sgt, -3), Some(4));
        // Sge floor+1: i >= 0, i -= 3 → 9,6,3,0 → 4 trips.
        assert_eq!(trip_i64(9, 0, Sge, -3), Some(4));
        // Empty countdown.
        assert_eq!(trip_i64(0, 0, Sgt, -1), None);
    }

    #[test]
    fn unroll_trip_rejects_degenerate_shapes() {
        use crate::ir::reexports::IrCmpOp::*;
        // Zero stride: infinite loop, no trip count.
        assert_eq!(trip_i64(0, 8, Slt, 0), None);
        // Sign/stride contradictions: a negative stride against an
        // ascending comparison diverges (or wraps); must be refused.
        assert_eq!(trip_i64(0, 8, Slt, -1), None);
        assert_eq!(trip_i64(0, 8, Sle, -4), None);
        assert_eq!(trip_i64(8, 0, Sgt, 1), None);
        assert_eq!(trip_i64(8, 0, Sge, 2), None);
        // Equality-style comparisons carry no ordering: refused.
        assert_eq!(trip_i64(0, 8, Eq, 1), None);
    }

    #[test]
    fn unroll_trip_extremes_never_wrap() {
        use crate::ir::reexports::IrCmpOp::*;
        // `limit - iv_init` overflows i64 (MAX - (-1)); the checked span
        // must refuse rather than produce a wrapped trip count.
        assert_eq!(trip_i64(-1, i64::MAX, Slt, 1), None);
        assert_eq!(trip_i64(i64::MIN, 0, Sgt, -1), None);
        // `-iv_step` overflows for iv_step = i64::MIN; checked_abs refuses.
        assert_eq!(trip_i64(0, i64::MIN, Sgt, i64::MIN), None);
        assert_eq!(trip_i64(0, i64::MAX, Slt, i64::MIN), None);
        // Huge stride: 0, 2^62 are < MAX (2 body executions) but the exit
        // test would then observe 2^63, which is not an i64.  The typed
        // closed form refuses up front — previously the cloner's checked
        // final-IV guard was the only thing standing between this shape and
        // a wrapped substituted constant.
        assert_eq!(trip_i64(0, i64::MAX, Slt, 1 << 62), None);
        // Sle at the top of the range: span/step + 1 would be MAX + 1.
        assert_eq!(trip_i64(0, i64::MAX, Sle, 1), None);
        // Extremes that stay in range must still unroll.
        assert_eq!(
            trip_i64(i64::MIN, i64::MIN + 8, Slt, 2),
            Some(4)
        );
        assert_eq!(
            trip_i64(i64::MAX, i64::MAX - 9, Sgt, -3),
            Some(3)
        );
        // `i <= MAX` with stride 3 from MAX-6 executes for MAX-6, MAX-3,
        // MAX and then increments PAST the type: the exit test never sees
        // an in-range value, so the loop has no static trip (UB in C, an
        // infinite loop on hardware).  Refused.
        assert_eq!(trip_i64(i64::MAX - 6, i64::MAX, Sle, 3), None);
        // One stride shorter it is a well-defined 2-trip loop again.
        assert_eq!(trip_i64(i64::MAX - 6, i64::MAX - 3, Sle, 3), Some(2));
    }

    // The trip count above is representable (2), but the post-loop IV
    // (0 + 2 * 2^62 = 2^63) is not.  The general cloner must refuse BEFORE
    // mutating anything — with the checked bail placed after the header/
    // latch retargeting, the CFG would be left half-rewritten.  Diamond
    // body so the loop routes to the general (multi-block) cloner:
    //   B0 preheader -> B1 header(phi, cmp < MAX) -> B2 split -> B3|B4 arms
    //   -> B5 latch(+2^62) -> B1;   exit B6 Return.
    #[test]
    fn general_cloner_overflow_bail_leaves_cfg_untouched() {
        let mut func = IrFunction::new("overflow_bail".to_string(), IrType::Void, vec![], false);

        // B0: preheader — %0 = 0
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Copy {
                dest: Value(0),
                src: Operand::Const(IrConst::I64(0)),
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });

        // B1: header — %1 = phi(%0, %5); %2 = %1 < i64::MAX
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(1),
                    ty: IrType::I64,
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(5)), BlockId(5)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I64(i64::MAX)),
                    ty: IrType::I64,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(2)),
                true_label: BlockId(2),
                false_label: BlockId(6),
            },
            source_spans: Vec::new(),
        });

        // B2: split — %3 = %1 >= 0; diamond to B3/B4
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![Instruction::Cmp {
                dest: Value(3),
                op: IrCmpOp::Sge,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I64(0)),
                ty: IrType::I64,
            }],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(3)),
                true_label: BlockId(3),
                false_label: BlockId(4),
            },
            source_spans: Vec::new(),
        });

        // B3/B4: arms
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::Copy {
                dest: Value(4),
                src: Operand::Value(Value(1)),
            }],
            terminator: Terminator::Branch(BlockId(5)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(4),
            instructions: vec![Instruction::Copy {
                dest: Value(6),
                src: Operand::Value(Value(1)),
            }],
            terminator: Terminator::Branch(BlockId(5)),
            source_spans: Vec::new(),
        });

        // B5: latch — %5 = %1 + 2^62; back to header
        func.blocks.push(BasicBlock {
            label: BlockId(5),
            instructions: vec![Instruction::BinOp {
                dest: Value(5),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I64(1 << 62)),
                ty: IrType::I64,
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });

        // B6: exit
        func.blocks.push(BasicBlock {
            label: BlockId(6),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });

        func.next_value_id = 7;

        let n = unroll_loops(&mut func);

        // trip = 2 passes the 2..=16 gate, so only the checked final-IV
        // computation stands between this loop and a clone: it must bail
        // with the CFG byte-for-byte intact.
        assert_eq!(n, 0, "overflowing final IV must refuse the unroll");
        assert_eq!(func.blocks.len(), 7, "no clone blocks may be appended");
        let latch = func.blocks.iter().find(|b| b.label == BlockId(5)).unwrap();
        assert!(
            matches!(latch.terminator, Terminator::Branch(lbl) if lbl == BlockId(1)),
            "latch back-edge must be untouched, got {:?}",
            latch.terminator
        );
        let header = func.blocks.iter().find(|b| b.label == BlockId(1)).unwrap();
        assert!(
            matches!(
                header.terminator,
                Terminator::CondBranch { true_label, false_label, .. }
                    if true_label == BlockId(2) && false_label == BlockId(6)
            ),
            "header exit branch must be untouched, got {:?}",
            header.terminator
        );
    }

    #[test]
    fn unroll_trip_matches_reference_simulation() {
        // Cross-check the closed forms against a direct loop simulation
        // over a dense grid of (init, limit, step) — the helper is the
        // single source of truth for complete unrolling, so a mismatch
        // here is a miscompile waiting to happen.
        use crate::ir::reexports::IrCmpOp::*;
        for init in -3i64..=6 {
            for limit in -3i64..=12 {
                for step in 1i64..=5 {
                    for &(op, desc) in &[(Slt, "lt"), (Sle, "le")] {
                        let simulated = (0..)
                            .scan(init, |i, _| {
                                let cont = match op {
                                    Slt => *i < limit,
                                    _ => *i <= limit,
                                };
                                if !cont {
                                    return None;
                                }
                                let old = *i;
                                *i += step;
                                Some(old)
                            })
                            .count() as i64;
                        let closed = trip_i64(init, limit, op, step);
                        if simulated == 0 {
                            assert!(
                                closed.is_none(),
                                "{desc}: init={init} limit={limit} step={step} empty loop must reject"
                            );
                        } else {
                            assert_eq!(
                                closed,
                                Some(simulated),
                                "{desc}: init={init} limit={limit} step={step}"
                            );
                        }
                    }
                }
                for step in -5i64..=-1 {
                    for &(op, desc) in &[(Sgt, "gt"), (Sge, "ge")] {
                        let simulated = (0..)
                            .scan(init, |i, _| {
                                let cont = match op {
                                    Sgt => *i > limit,
                                    _ => *i >= limit,
                                };
                                if !cont {
                                    return None;
                                }
                                let old = *i;
                                *i += step;
                                Some(old)
                            })
                            .count() as i64;
                        let closed = trip_i64(init, limit, op, step);
                        if simulated == 0 {
                            assert!(
                                closed.is_none(),
                                "{desc}: init={init} limit={limit} step={step} empty loop must reject"
                            );
                        } else {
                            assert_eq!(
                                closed,
                                Some(simulated),
                                "{desc}: init={init} limit={limit} step={step}"
                            );
                        }
                    }
                }
            }
        }
    }
}
