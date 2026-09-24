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

thread_local! {
    /// Whether Pass B (two-block guard-free unrolling) runs at all — the
    /// documented `CCC_NO_TWO_BLOCK_UNROLL` kill switch that every A/B
    /// differential in the harness speaks.
    ///
    /// Resolved ONCE by the driver and then read from per-thread state, the
    /// same contract the vectorizer's ISA gates use (`set_x86_simd_isa`).  It
    /// used to be read out of the process environment inside `unroll_loops`;
    /// because `std::env::set_var` is process-global, the kill-switch test
    /// toggled it underneath every test running in parallel on the other
    /// threads of the same test binary, and those tests observed the wrong
    /// value.  Measured on the `two_block_unroll` group: 11 failing runs in
    /// 150 (~7%), the symptom being
    /// `two_block_unroll_profitability_positive_control` seeing the switch ON
    /// and the unroller declining a loop it must unroll.  This is the same
    /// defect `LoopInvertConfig` was introduced to remove from `loop_invert`;
    /// configuration a caller can supply belongs in a parameter or in
    /// per-thread state, never in ambient global state.
    ///
    /// Defaults to enabled, which is exactly the environment-unset default, so
    /// an entry point that never went through `run_passes` (unit tests) behaves
    /// as it did before the switch existed.
    static TWO_BLOCK_UNROLL_ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Record whether Pass B may run.  Called by `run_passes` before any unroll
/// entry point runs on this thread.
pub(crate) fn set_two_block_unroll_enabled(enabled: bool) {
    TWO_BLOCK_UNROLL_ENABLED.with(|cell| cell.set(enabled));
}

#[inline]
fn two_block_unroll_enabled() -> bool {
    TWO_BLOCK_UNROLL_ENABLED.with(|cell| cell.get())
}

thread_local! {
    /// Whether the persisting-inner-loop gate uses the pre-S20 lenient
    /// behavior (skip unrecognized nested inners instead of refusing the
    /// outer unroll) — the `CCC_UNROLL_LEGACY_PERSIST_GATE` kill switch for
    /// A/B and emergency revert.  Resolved once by the driver into
    /// per-thread state for the same reason as
    /// `TWO_BLOCK_UNROLL_ENABLED` above: an ambient `std::env` read here
    /// would race parallel tests that toggle it.  Defaults to false (the
    /// fail-closed gate), so unit tests exercise the shipped behavior.
    static PERSIST_GATE_LEGACY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Record whether the legacy lenient persist gate is active.  Called by
/// `run_passes` before any unroll entry point runs on this thread.
pub(crate) fn set_persist_gate_legacy(enabled: bool) {
    PERSIST_GATE_LEGACY.with(|cell| cell.set(enabled));
}

thread_local! {
    /// Whether veto-path tracing is active (`CCC_UNROLL_GATE_TRACE`).  Like
    /// `PERSIST_GATE_LEGACY` above, resolved once by the driver into
    /// per-thread state: the pass pipeline must not read the process
    /// environment per call (`check_env_test_hygiene.sh` ratchets the
    /// remaining sites).  Defaults to false.
    static PERSIST_GATE_TRACE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Record whether persist-gate veto tracing is active.  Called by
/// `run_passes` before any unroll entry point runs on this thread.
pub(crate) fn set_persist_gate_trace(enabled: bool) {
    PERSIST_GATE_TRACE.with(|cell| cell.set(enabled));
}

#[inline]
fn persist_gate_trace() -> bool {
    PERSIST_GATE_TRACE.with(|cell| cell.get())
}

#[inline]
fn persist_gate_legacy() -> bool {
    PERSIST_GATE_LEGACY.with(|cell| cell.get())
}

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

/// Which pipeline slot `unroll_loops` was invoked from.
///
/// The two invocation points have different safety envelopes around the
/// SAME loop population:
///
/// * `Early` (Phase 2b, before the main loop vectorizer): a two-block
///   partial unroll here would rewrite unit-stride elementwise store loops
///   (`a[i] = f(a[i])` shapes) into step-`k` store pairs BEFORE the
///   vectorizer ever sees them; its map detector then correctly declines
///   the non-unit stride and the loop stays SCALAR ×k — a large regression
///   versus the 4/8-wide packed form it would have produced. The complete
///   unroller does not have this hazard (it removes the loop entirely and
///   the SLP/unrolled pair machinery handles the flattened stores), and
///   `do_unroll`'s guarded form only takes 3+-block chains, so only the
///   NEW partial two-block path needs the gate.
/// * `PostVec` (after `vectorize`, before BB-SLP): every loop that remains
///   rolled here was DECLINED by the loop vectorizer — the two-block
///   partial unroll's feedstock. The k concatenated bodies land in one
///   basic block, exactly the shape `run_bb_slp` packs (adjacent stores
///   seed, isomorphic trees pack, `PackKind::MemLoad` fuses consecutive
///   loads).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnrollPhase {
    Early,
    PostVec,
}

pub(crate) fn unroll_loops(func: &mut IrFunction, phase: UnrollPhase) -> usize {
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

    // ARX-vectorized loops (passes::vec_arx) keep their ROLLED vector body
    // deliberately: one lane-parallel 128-bit body per iteration is the
    // ICX-beating shape for block-cipher rounds (instruction count AND
    // I-cache). Scalar unroll economics do not apply — the body already
    // saturates the machine's lane parallelism — and unrolling a trip-10
    // ARX loop would multiply .text tenfold for zero ILP gain. The marker
    // intrinsics (VecRotlI32x4/VecShufdI32x4) are only ever produced by
    // that pass, so this filter cannot fire on any other loop.
    let arx_marker_labels: FxHashSet<u32> = {
        let mut hit: FxHashSet<u32> = FxHashSet::default();
        for block in &func.blocks {
            let has_marker = block.instructions.iter().any(|inst| {
                matches!(inst, crate::ir::reexports::Instruction::Intrinsic { op, .. }
                    if matches!(op,
                        crate::ir::reexports::IntrinsicOp::VecRotlI32x4
                        | crate::ir::reexports::IntrinsicOp::VecShufdI32x4))
            });
            if has_marker {
                hit.insert(block.label.0);
            }
        }
        hit
    };
    // (Free function so the candidate filters below can call it while
    // `func` is otherwise mutably borrowed by the unroll attempts.)
    let loop_is_arx = |lp: &loop_analysis::NaturalLoop, f: &IrFunction| {
        !arx_marker_labels.is_empty()
            && lp
                .body
                .iter()
                .any(|&b| arx_marker_labels.contains(&f.blocks[b].label.0))
    };

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
            .filter(|lp| matches!(lp.body.len(), 2 | 3) && !loop_is_arx(lp, func))
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
                .filter(|lp| lp.body.len() > 3 && lp.body.len() <= 33 && !loop_is_arx(lp, func))
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

    // Pass B: partial GUARD-FREE unrolling of two-block counted loops
    // (post-vectorize phase only — see `UnrollPhase`). Rebuild the CFG
    // after each success: one new block is appended and the latch is
    // disconnected, so block indices captured by earlier analyses go
    // stale. The pass is idempotent-safe by construction: the unrolled
    // loop is itself a two-block loop, so the fixpoint may take it again
    // (trip/k, work×k) until either the divisibility, the code-size
    // budget, or the `count` cap stops it — every step is a legal
    // guard-free unroll of a smaller constant trip.
    // CCC_NO_TWO_BLOCK_UNROLL is the standard per-transform kill switch
    // (every A/B differential in the harness speaks it).  It is resolved once
    // by the driver into per-thread state — see `TWO_BLOCK_UNROLL_ENABLED` —
    // so this hot path never reads the process environment.
    if phase == UnrollPhase::PostVec && two_block_unroll_enabled() {
        loop {
            let cfg = CfgAnalysis::build(func);
            let raw = loop_analysis::find_natural_loops(
                cfg.num_blocks,
                &cfg.preds,
                &cfg.succs,
                &cfg.idom,
            );
            let loops_now = loop_analysis::merge_loops_by_header(raw);
            let mut did = false;
            let mut two_block: Vec<_> = loops_now
                .iter()
                .filter(|lp| lp.body.len() == 2 && !loop_is_arx(lp, func))
                .cloned()
                .collect();
            two_block.sort_by_key(|lp| lp.header);
            for lp in &two_block {
                if try_partial_unroll_two_block(func, lp, &cfg) {
                    count += 1;
                    did = true;
                    break;
                }
            }
            if !did || count > 96 {
                break;
            }
        }
        if count > 0 {
            return count;
        }
    }

    // Collect and sort candidates by body size (smallest first = innermost first).
    let mut candidates: Vec<UnrollCandidate> = loops
        .iter()
        .filter(|lp| !loop_is_arx(lp, func))
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

/// Would executing this instruction fewer times than the source program
/// specifies change observable behavior? This is execution-count semantics,
/// stricter than DCE's "unused ⇒ droppable": a Store of a dead value is
/// still a store, and a `pure` call whose result is unused still executes
/// (and may not return). The unrolled latch runs once per unroll factor,
/// so ANY of these disqualify the loop from partial unrolling.
fn latch_inst_is_side_effecting(inst: &Instruction) -> bool {
    match inst {
        // Every store counts: its memory write is observable regardless of
        // whether the stored value is otherwise live.
        Instruction::Store { .. } => true,
        Instruction::Load { volatile, .. } => *volatile,
        Instruction::Call { .. }
        | Instruction::CallIndirect { .. }
        | Instruction::InlineAsm { .. }
        | Instruction::AtomicRmw { .. }
        | Instruction::AtomicCmpxchg { .. }
        | Instruction::AtomicLoad { .. }
        | Instruction::AtomicStore { .. }
        | Instruction::DynAlloca { .. }
        | Instruction::Alloca { .. } => true,
        _ => false,
    }
}

/// Is `val` — defined in block `latch_idx` — used by any instruction,
/// terminator, or phi in a DIFFERENT block? After partial unrolling the
/// latch executes once per unroll factor, so such a use would observe a
/// value that is k-1 iterations stale.
fn latch_value_escapes(func: &IrFunction, latch_idx: usize, val: u32) -> bool {
    for (bi, block) in func.blocks.iter().enumerate() {
        if bi == latch_idx {
            continue;
        }
        let mut escapes = false;
        for inst in &block.instructions {
            inst.for_each_used_value(|id| {
                if id == val {
                    escapes = true;
                }
            });
        }
        block.terminator.for_each_used_value(|id| {
            if id == val {
                escapes = true;
            }
        });
        if escapes {
            return true;
        }
    }
    false
}

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
    let preheader = loop_analysis::find_preheader(header, &lp.body, &cfg.preds)?;

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

    // 5b. Body connectivity: every body block must be reachable from the
    // header WITHOUT taking the back edge.  This is an inherent property of
    // a natural loop, so a violation means the "body" contains a detached
    // cycle — e.g. the clone chain a complete unroller left behind when it
    // rewrote an inner loop and the CFG simplifier has not run yet.  Such a
    // cycle has no entry from the preheader, evades the nested-loop check
    // (unreachable blocks are not merged into `all_headers`), and do_unroll
    // would clone it into a garbage 4x structure whose exit guards fall
    // through to the wrong accumulator.  Rejecting here is purely
    // conservative: the loop simply stays rolled.
    // NOTE: this guard does NOT trigger on red-team cfg 1765/3169 (their
    // bodies are reachable and connected); those two were fixed elsewhere
    // (the strict Add-only partial-IV split and the do_unroll Step 5b
    // terminator rewrite below).
    {
        let mut seen: FxHashSet<usize> = FxHashSet::default();
        let mut stack: Vec<usize> = vec![header];
        while let Some(bi) = stack.pop() {
            if !seen.insert(bi) {
                continue;
            }
            let label = func.blocks[bi].label;
            let mut succs: Vec<BlockId> = match &func.blocks[bi].terminator {
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
            succs.retain(|l| *l != header_label);
            for l in succs {
                if let Some(next) = func.blocks.iter().position(|b| b.label == l) {
                    if lp.body.contains(&next) {
                        stack.push(next);
                    }
                }
            }
        }
        if seen.len() != lp.body.len() {
            return None;
        }
    }

    // 5c. body_work must be a simple linear chain header -> work -> latch.
    // `do_unroll` emits, per clone, ONE guard that decides whether to enter
    // the next clone or branch to the exit, and it threads the loop-carried
    // value through the clones under the assumption that control always
    // flows work[0] -> work[1] -> ... -> latch.  A body_work with branches,
    // merges or sub-cycles violates that: the clone-local guard exits jump
    // to the exit block with whatever the value carried from the PREVIOUS
    // iteration, doubling the accumulator.  Requiring a linear chain is the
    // exact precondition the guard arithmetic was derived for; anything else
    // simply stays rolled (fail-closed).
    // NOTE: unlike the general shape above, the chain in red-team cfg 1765
    // IS linear and passes this check — 1765 was fixed by the Step 5b
    // terminator rewrite in do_unroll, not by this guard.  The guard covers
    // the wider class of non-linear bodies; those shapes are exercised
    // exhaustively by scripts/unroll_stress.py.
    {
        let mut chain_ok = true;
        let mut seen_work: FxHashSet<usize> = FxHashSet::default();
        let mut entry_preds = 0usize;
        for &bi in &body_work {
            let inner_preds: Vec<usize> = cfg
                .preds
                .row(bi)
                .iter()
                .map(|&p| p as usize)
                .filter(|p| lp.body.contains(p))
                .collect();
            // Exactly one predecessor inside the loop; it must be the header
            // (the body entry) or another body_work block.
            if inner_preds.len() != 1 {
                chain_ok = false;
                break;
            }
            let p = inner_preds[0];
            if !(p == header || body_work.contains(&p)) {
                chain_ok = false;
                break;
            }
            if p == header {
                entry_preds += 1;
            }
            if !seen_work.insert(p) {
                // two work blocks share a predecessor -> merge, not a chain
                chain_ok = false;
                break;
            }
        }
        if !chain_ok || entry_preds != 1 {
            return None;
        }
        // Exactly one work block must terminate by branching to the latch
        // (the chain tail); every other work block's successor lies in
        // body_work (checked below via the chain walk).
        let latch_label = func.blocks[latch].label;
        let mut tails = 0usize;
        for &bi in &body_work {
            if let Terminator::Branch(l) = &func.blocks[bi].terminator {
                if *l == latch_label {
                    tails += 1;
                }
            }
        }
        if tails != 1 {
            return None;
        }
        // And every work block must branch only to body_work or the latch
        // (no jump out of the loop, no CondBranch in the chain).
        for &bi in &body_work {
            let succ_labels: Vec<BlockId> = match &func.blocks[bi].terminator {
                Terminator::Branch(l) => vec![*l],
                Terminator::CondBranch { .. } => vec![],
                _ => vec![],
            };
            for l in succ_labels {
                if l != latch_label {
                    let idx = func.blocks.iter().position(|b| b.label == l);
                    if let Some(idx) = idx {
                        if !body_work.contains(&idx) {
                            return None;
                        }
                    }
                }
            }
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

    // 7b. The latch must be pure IV bookkeeping. `do_unroll` NEVER clones
    //     the latch: after unrolling by k it executes once per k source
    //     iterations (the clones jump straight back to it). A side-effecting
    //     instruction in the latch would run 1/k as often as the source
    //     semantics require, and a latch-defined value escaping the latch
    //     (other than the IV increment itself, which Step 4 retargets in
    //     place) would be the stale once-per-k copy at every use. Both
    //     silently miscompile: a perfect-nest row-sum whose result store
    //     (`out[i] = s`) sits in the outer latch computed k-1 of every k
    //     rows and stored only row i's sum — odd rows kept their previous
    //     contents (tests/regression/outer_loop_shapes.c, kernel s4).
    for (idx, inst) in func.blocks[latch].instructions.iter().enumerate() {
        if idx == latch_iv_incr_idx {
            continue;
        }
        if latch_inst_is_side_effecting(inst) {
            return None;
        }
        if let Some(dest) = inst.dest() {
            if latch_value_escapes(func, latch, dest.0) {
                return None;
            }
        }
    }

    // 8. Detect the exit condition from the header's CondBranch.
    let (
        exit_target,
        body_entry,
        exit_cmp_op,
        exit_cmp_ty,
        exit_limit,
        iv_is_lhs,
        exit_cond_positive,
    ) = find_exit_condition(func, header, &lp.body, iv_phi, true)?;

    // 8b. The IV phi may only be referenced by a phi OUTSIDE the loop when
    //     that phi lives in the DIRECT exit block — Step 5 threads exactly
    //     those with per-exit-check edge values.  A phi in any LATER block
    //     (an exit block that an intermediate pass SPLIT, a downstream
    //     join) keeps its stale header-edge incoming while the new
    //     exit-check edges bypass it entirely: the reader sees the
    //     once-per-k IV (⌊n/k⌋·k instead of n — found on the counting
    //     epic's remainder, whose exit merge sat past a split).  Fail
    //     closed on the shape; the loop keeps its rolled form.
    {
        let exit_bi = func.blocks.iter().position(|b| b.label == exit_target);
        for (bi, block) in func.blocks.iter().enumerate() {
            if lp.body.contains(&bi) {
                continue;
            }
            if Some(bi) == exit_bi {
                continue; // the direct exit block: Step 5 threads it
            }
            for inst in &block.instructions {
                if let Instruction::Phi { incoming, .. } = inst {
                    for (op, _) in incoming {
                        if let Operand::Value(v) = op {
                            if v.0 == iv_phi.0 {
                                return None;
                            }
                        }
                    }
                }
            }
        }
    }

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

    // 11b. Foreign-edge soundness: `do_unroll` synthesizes exit phis for
    //     loop-carried values, and every FOREIGN predecessor of the exit
    //     (any exit pred other than the header — usually a zero-trip
    //     bypass, but possibly a jump from anywhere above the loop)
    //     receives the header phi's ENTRY operand: the value each carried
    //     value holds before the loop runs. The entry operand is used at
    //     the end of the preheader, so its definition dominates the
    //     preheader; requiring the preheader to dominate every foreign
    //     predecessor closes the chain to the exit edge transitively. A
    //     foreign edge the preheader does not dominate (jump threading,
    //     Duff's-device dispatch, a `goto` into the merge) would read a
    //     value no path defined — that loop simply stays rolled.
    if let Some(exit_bi) = func.blocks.iter().position(|b| b.label == exit_target) {
        let dom = loop_analysis::DominanceChecker::new(cfg.num_blocks, &cfg.idom);
        for &p in cfg.preds.row(exit_bi) {
            let p = p as usize;
            if p != header && !dom.dominates(preheader, p) {
                return None;
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
                    // Rotates belong to the shift family for this test: a
                    // bit-iteration latch that rotates its induction value is
                    // still a pure bit loop, and classifying it as `other_arith`
                    // would veto a complete unroll that is profitable.
                    AShr | LShr | Shl | BitTest | RotateLeft | RotateRight => saw_shift = true,
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
///   trip inside `trip_range` (callers pass `1..=1` for the tiny-loop
///   deletion pass — trip 1 folds every header phi to its init value and the
///   latch falls straight through to the exit, which is what the last row of
///   a triangular nest `for (j = i+1; j < n; …)` becomes after the outer loop
///   is straightened — and `1..=16` for the general complete unroll), and
///   (header non-phi + body) instructions × trip within the expansion
///   budget.
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
    matches!(
        op,
        IrCmpOp::Ult | IrCmpOp::Ule | IrCmpOp::Ugt | IrCmpOp::Uge
    )
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
/// True when `op` denotes a value that depends only on integer constants and
/// (possibly) `iv`.  Complete unrolling substitutes the OUTER IV with a
/// per-clone constant, so once such an operand is used in an inner loop's own
/// IV/bound, that inner loop becomes constant-trip after the outer unroll.
/// Intended for the persisting-inner-loop gate; a dependency on anything else
/// (a function parameter, a memory load, a non-IV-dependent phi) means the
/// inner loop's trip stays dynamic.
fn depends_only_on_const_and_iv(func: &IrFunction, op: &Operand, iv: Value, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    // `Operand` is closed over {Const, Value}; both handled above.  The
    // `Value` arm falls through to a def-chain walk; a def that is a memory /
    // opaque instruction returns false (never constant after substitution).
    let value = match op {
        Operand::Const(_) => return true,
        Operand::Value(v) => v,
    };
    if value.0 == iv.0 {
        return true;
    }
    let mut def: Option<&Instruction> = None;
    for block in &func.blocks {
        if let Some(inst) = block.instructions.iter().find(|i| i.dest() == Some(*value)) {
            def = Some(inst);
            break;
        }
    }
    let Some(inst) = def else { return false };
    match inst {
        Instruction::Copy { src, .. } | Instruction::Cast { src, .. } => {
            depends_only_on_const_and_iv(func, src, iv, depth + 1)
        }
        Instruction::BinOp { lhs, rhs, .. } => {
            depends_only_on_const_and_iv(func, lhs, iv, depth + 1)
                && depends_only_on_const_and_iv(func, rhs, iv, depth + 1)
        }
        Instruction::Phi { incoming, .. } => incoming
            .iter()
            .all(|(op, _)| depends_only_on_const_and_iv(func, op, iv, depth + 1)),
        _ => false,
    }
}

/// Observability for the persist gate: with `CCC_UNROLL_GATE_TRACE=1`, every
/// veto prints the function, the outer/inner headers, and the arm that fired.
/// Veto-path only (never on allow): the flag read runs solely when the gate is
/// about to refuse, so steady-state compilation pays nothing for it.  The
/// flag itself is resolved once by the driver (see `PERSIST_GATE_TRACE`).
fn trace_persist_veto(func: &IrFunction, outer_header: usize, inner_header: usize, arm: &str) {
    if persist_gate_trace() {
        eprintln!(
            "[UNROLL-GATE] veto fn={} outer={} inner={} arm={}",
            func.name, outer_header, inner_header, arm
        );
    }
}

/// True when completely unrolling the outer loop `outer` would leave an inner
/// natural loop alive as a runtime loop in every clone.  That happens when a
/// properly-nested inner loop's IV/bound depends on a runtime value (anything
/// other than constants and the outer loop's IV).  The clones then become
/// sibling runtime reductions sharing one accumulator, a shape the downstream
/// late vectorizer miscompiles (reproducer: `for (i = 0; i < 4; i++)
/// for (j = 0; j < n; j++) s += a[i*n+j]` — the outer trip-4 is unrolled, the
/// runtime-`n` inner survives as four chained reductions, and lccc sums the
/// wrong elements while GCC returns the correct value).  A CONSTANT-trip inner
/// loop is fine: the fixpoint fully unrolls it too.  Refusing keeps the outer
/// loop rolled, which is correct and matches GCC.
///
/// Fail-closed: the ONLY nested inner loop that does not persist is a clean
/// counted loop (single latch with an unconditional Branch to the header, an
/// Add/Sub-IV, a header exit on that IV) whose init and limit depend solely
/// on constants and the outer IV — the triangular-cascade shape, which the
/// fixpoint unrolls in the next round.  Every other nested inner loop
/// (multi-latch, non-Branch latch, no Add/Sub-IV, no exit on the found IV,
/// no recognizable init) is one the complete cloner cannot cascade by
/// construction, so it persists in every clone and the outer unroll is
/// refused.  This is a correctness AND a codegen rule: cloning a surviving
/// runtime loop K times is pure growth (csv_field_sum's 6-trip column loop
/// over two data-dependent digit while-loops: 19→89 blocks, +345 IR insns,
/// +156% .text, +8% runtime; the digit extractor's exit reads the
/// magic-divided value rather than its Add counter, and the digit emitter
/// carries an Add counter but exits on its Sub countdown — an exit/IV
/// mismatch in both cases, invisible to the old skip-on-unrecognized gate).
/// Refusing reproduces GCC's rolled shape for these nests.
///
/// The exit match is deliberately VALUE-based, not syntactic: the limit is
/// accepted whenever it depends only on constants and the outer IV, even
/// when it is recomputed in the inner header rather than hoisted (g3's
/// `i + 8`, invariant in value but defined in-body).  Substitution plus the
/// fold/copyprop between fixpoint rounds makes such a limit per-clone
/// constant, so the fixpoint still cascades — requiring syntactic
/// loop-invariance here would veto unrolls the fixpoint completes.  The gate
/// therefore predicts EXACTLY what the fixpoint can cascade: first-IV-only
/// (matching the fixpoint's own finder — an exit/IV mismatch vetoes because
/// the fixpoint would fail to unroll the inner too, so it would persist),
/// with init/limit constability decided by value.
///
/// Detached cycles need no special handling: a natural loop's header
/// dominates its body, so a block that is unreachable from the outer header
/// can never sit in `outer.body` — the membership test below already excludes
/// every dead mid-fixpoint artifact (`find_natural_loops` itself is
/// dominance-rooted and cannot even report one).
///
/// Known conservative case: an inner loop carrying two IVs where the exit
/// reads the SECOND one (csv's digit emitter: an Add o-counter plus a Sub
/// t-countdown with the exit on the countdown). The IV finder returns the
/// first IV, the exit match fails, and the gate refuses — even if the
/// exit's own IV were cascade-able. Trying every IV until one matches an
/// exit would recover that rare shape, but the added search in a
/// correctness-critical gate is not worth a missed unroll the corpus has
/// never shown; revisit with data if one appears.
///
/// `CCC_UNROLL_LEGACY_PERSIST_GATE=1` restores the pre-S20 lenient gate (skip
/// unrecognized inners instead of refusing) for A/B and emergency revert.
fn body_contains_persisting_inner_loop(
    func: &IrFunction,
    cfg: &CfgAnalysis,
    outer: &loop_analysis::NaturalLoop,
    outer_iv: Value,
) -> bool {
    // Fail-closed (see the doc comment): every nested inner loop the complete
    // cloner cannot cascade persists in every clone, so every unrecognized
    // shape refuses the outer unroll.  The legacy knob restores the old
    // skip-on-unrecognized behavior for A/B and emergency revert.
    let legacy = persist_gate_legacy();
    let all = loop_analysis::find_natural_loops(cfg.num_blocks, &cfg.preds, &cfg.succs, &cfg.idom);
    for inner in &all {
        if inner.header == outer.header {
            continue;
        }
        if !outer.body.contains(&inner.header) {
            continue;
        }
        let latches = inner.latches(&cfg.preds);
        if latches.len() != 1 {
            // Multi-latch inner loops can never complete-unroll (the cloner
            // requires a single latch), so they always persist.
            if legacy {
                continue;
            }
            trace_persist_veto(func, outer.header, inner.header, "multi-latch");
            return true;
        }
        let latch = latches[0];
        let latch_label = func.blocks[latch].label;
        if !matches!(
            &func.blocks[latch].terminator,
            Terminator::Branch(l) if *l == func.blocks[inner.header].label
        ) {
            // Without a Branch latch the cloner cannot cascade this inner
            // loop — it persists.
            if legacy {
                continue;
            }
            trace_persist_veto(func, outer.header, inner.header, "latch-shape");
            return true;
        }
        // Clean counted inner loop: IV, exit condition, and its initial value.
        // Anything else (a div/mul-driven while whose exit reads no
        // Add/Sub-IV, an exit/IV mismatch where the exit reads a different
        // phi than the finder's first IV, a loop with no recognizable exit
        // or init) persists: the fixpoint's own first-IV finder would fail
        // on it too.
        let Some((iv_phi, _ty, _step, _)) =
            find_iv_in_loop_ext(func, inner.header, latch, latch_label)
        else {
            if legacy {
                continue;
            }
            trace_persist_veto(func, outer.header, inner.header, "no-iv");
            return true;
        };
        // Value-based exit: no syntactic-invariance requirement (see
        // `find_exit_condition`).  A limit recomputed in the inner header
        // from the outer IV still cascades — substitution plus the
        // fold/copyprop between fixpoint rounds makes it per-clone
        // constant — so invariance is decided by `depends_only...` below.
        // Legacy mode keeps the SYNTACTIC exit (`true`): the lenient gate
        // must skip exactly the nests it skipped pre-S20, and a value exit
        // would send unrecognized nests down the refuse path instead of
        // the skip path (fse's main: 552 -> 405 in legacy mode).
        let Some((_et, _be, _cmp, _cty, limit, _islhs, _pos)) =
            find_exit_condition(func, inner.header, &inner.body, iv_phi, legacy)
        else {
            if legacy {
                continue;
            }
            trace_persist_veto(func, outer.header, inner.header, "no-exit");
            return true;
        };
        // Every non-latch incoming of the IV phi is an entry value the inner
        // loop can start from.  A multi-entry inner header (irreducible CFG)
        // is per-clone constant only if ALL of them are — testing just one
        // would bless a bound that varies by entry, and the fixpoint's
        // `resolve_const_operand` cannot see through multi-entry phis
        // anyway, so the inner would persist.  (Legacy mode keeps the old
        // last-incoming-only test for exact A/B restoration.)
        let mut init_ops = Vec::new();
        for inst in &func.blocks[inner.header].instructions {
            if let Instruction::Phi { dest, incoming, .. } = inst {
                if dest.0 == iv_phi.0 {
                    for (op, lbl) in incoming {
                        if *lbl != latch_label {
                            init_ops.push(op.clone());
                        }
                    }
                }
            }
        }
        if init_ops.is_empty() {
            if legacy {
                continue;
            }
            trace_persist_veto(func, outer.header, inner.header, "no-init");
            return true;
        }
        let inits_to_test: &[Operand] = if legacy {
            &init_ops[init_ops.len() - 1..]
        } else {
            &init_ops
        };
        // If the inner IV's initial value or its bound depends on a runtime
        // value (not a constant and not the OUTER IV), the inner loop persists
        // after the outer unroll -> unsafe to unroll the outer loop.  No
        // legacy skip here (unlike the unrecognized-shape arms above): a
        // recognized-but-dynamic bound persists under EITHER gate.
        if inits_to_test
            .iter()
            .any(|op| !depends_only_on_const_and_iv(func, op, outer_iv, 0))
            || !depends_only_on_const_and_iv(func, &limit, outer_iv, 0)
        {
            trace_persist_veto(func, outer.header, inner.header, "dynamic-bound");
            return true;
        }
    }
    false
}

// ── Header-phi iteration model ───────────────────────────────────────────────
/// One header phi of a natural loop with a unique preheader and a single
/// latch: `dest = phi [init, preheader], [back, latch]`.
#[derive(Clone, Debug)]
struct HeaderPhi {
    id: u32,
    ty: IrType,
    init: Operand,
    back: Operand,
}

/// Collect EVERY phi of the loop header (IV included).  `None` when a phi
/// is not in the two-incoming preheader/latch form the cloners rely on.
fn collect_header_phis(
    func: &IrFunction,
    header: usize,
    latch_label: BlockId,
) -> Option<Vec<HeaderPhi>> {
    let mut out = Vec::new();
    for inst in &func.blocks[header].instructions {
        let Instruction::Phi { dest, ty, incoming } = inst else {
            continue;
        };
        if incoming.len() != 2 {
            return None;
        }
        let back: Operand = incoming.iter().find(|(_, l)| *l == latch_label)?.0.clone();
        let init: Operand = incoming.iter().find(|(_, l)| *l != latch_label)?.0.clone();
        out.push(HeaderPhi {
            id: dest.0,
            ty: *ty,
            init,
            back,
        });
    }
    Some(out)
}

/// The value every header phi holds at the entry of a given iteration.
///
/// Both complete unrollers used to thread only the phis whose back-edge value
/// was defined in the cloned region and substituted nothing for the rest; the
/// remaining shapes silently read the phi's INITIAL value in every iteration:
///   * `first = 0` at the end of the body (constant back value);
///   * `cur = param` (loop-invariant back value);
///   * `x = y; y = t` (the back value is ANOTHER header phi).
/// This model closes all of them: the value of phi `P` at the entry of
/// iteration `t` (t ≥ 1) is `P.back` evaluated in iteration `t-1`:
///   * a constant is itself;
///   * the latch IV increment is the IV's value in iteration `t`;
///   * another header phi `Q` is `Q`'s value at iteration `t-1`;
///   * a value defined inside the loop is iteration `t-1`'s copy of it;
///   * anything else is loop-invariant and used verbatim.
/// Iteration 0 sees the preheader `init` operands.  Because the results are
/// never header-phi ids themselves, substituting one phi at a time is
/// order-independent.
struct LoopPhiModel {
    phis: Vec<HeaderPhi>,
    iv_id: u32,
    iv_incr_dest: Option<u32>,
    iv_ty: IrType,
    iv_init: i64,
    iv_step: i64,
    /// Every SSA id defined by an instruction inside the loop (header non-phi
    /// instructions and all body blocks).
    loop_defs: FxHashSet<u32>,
}

impl LoopPhiModel {
    fn iv_const(&self, t: i64) -> Operand {
        Operand::Const(IrConst::from_i64(
            self.iv_init.wrapping_add(self.iv_step.wrapping_mul(t)),
            self.iv_ty,
        ))
    }

    fn is_header_phi(&self, id: u32) -> bool {
        self.phis.iter().any(|p| p.id == id)
    }

    /// Environment of iteration 0: every phi holds its preheader value; the
    /// IV holds its (already constant-resolved) init.
    fn env_entry(&self) -> FxHashMap<u32, Operand> {
        let mut env = FxHashMap::default();
        for p in &self.phis {
            let v = if p.id == self.iv_id {
                self.iv_const(0)
            } else {
                p.init
            };
            env.insert(p.id, v);
        }
        env
    }

    /// Environment of iteration `t ≥ 1`, given iteration `t-1`'s environment
    /// and the rename map of iteration `t-1`'s copy of the loop (an absent
    /// entry means the original, un-renamed instruction executes there).
    fn env_next(
        &self,
        t: i64,
        env_prev: &FxHashMap<u32, Operand>,
        rename_prev: &FxHashMap<u32, u32>,
    ) -> FxHashMap<u32, Operand> {
        let mut env = FxHashMap::default();
        for p in &self.phis {
            let v = if p.id == self.iv_id {
                self.iv_const(t)
            } else {
                self.resolve_back(&p.back, t, env_prev, rename_prev)
            };
            env.insert(p.id, v);
        }
        env
    }

    fn resolve_back(
        &self,
        back: &Operand,
        t: i64,
        env_prev: &FxHashMap<u32, Operand>,
        rename_prev: &FxHashMap<u32, u32>,
    ) -> Operand {
        match back {
            Operand::Const(_) => *back,
            Operand::Value(v) => {
                if Some(v.0) == self.iv_incr_dest {
                    self.iv_const(t)
                } else if self.is_header_phi(v.0) {
                    *env_prev
                        .get(&v.0)
                        .expect("iteration environment covers every header phi")
                } else if self.loop_defs.contains(&v.0) {
                    Operand::Value(Value(*rename_prev.get(&v.0).unwrap_or(&v.0)))
                } else {
                    *back
                }
            }
        }
    }
}

/// Substitute every header phi of `env` in one instruction / terminator.
fn apply_env_inst(inst: &mut Instruction, env: &FxHashMap<u32, Operand>) {
    for (&id, repl) in env {
        subst_value_with_operand(inst, id, repl);
    }
}

fn apply_env_term(term: &mut Terminator, env: &FxHashMap<u32, Operand>) {
    for (&id, repl) in env {
        subst_value_in_terminator(term, id, repl);
    }
}

/// Header non-phi instructions that are NOT part of the exit-test chain (the
/// Cmp and an optional Cast feeding the CondBranch).  These carry the loop
/// condition's side effects and extra values (`for (...; cnt++, i < 4; ...)`),
/// execute once per guard evaluation — `trip + 1` times — and must therefore
/// be replicated per iteration AND once more after the last one.
fn header_extra_indices(func: &IrFunction, header: usize) -> Vec<usize> {
    let block = &func.blocks[header];
    let mut chain: FxHashSet<u32> = FxHashSet::default();
    if let Terminator::CondBranch {
        cond: Operand::Value(c),
        ..
    } = &block.terminator
    {
        chain.insert(c.0);
        if let Some(Instruction::Cast {
            src: Operand::Value(s),
            ..
        }) = block.instructions.iter().find(|i| i.dest() == Some(*c))
        {
            chain.insert(s.0);
        }
    }
    block
        .instructions
        .iter()
        .enumerate()
        .filter(|(_, inst)| {
            !matches!(inst, Instruction::Phi { .. })
                && !inst.dest().is_some_and(|d| chain.contains(&d.0))
        })
        .map(|(i, _)| i)
        .collect()
}

/// Rewrite every use of `env`'s phis (and the `extra` header values) in the
/// blocks OUTSIDE the loop to their post-loop values.
fn rewrite_outside_uses(
    func: &mut IrFunction,
    loop_body: &FxHashSet<usize>,
    final_env: &FxHashMap<u32, Operand>,
    extra_final: &FxHashMap<u32, u32>,
) {
    for (bi, block) in func.blocks.iter_mut().enumerate() {
        if loop_body.contains(&bi) {
            continue;
        }
        for inst in &mut block.instructions {
            apply_env_inst(inst, final_env);
            for (&old, &new) in extra_final {
                subst_value_with_operand(inst, old, &Operand::Value(Value(new)));
            }
        }
        apply_env_term(&mut block.terminator, final_env);
        for (&old, &new) in extra_final {
            subst_value_in_terminator(&mut block.terminator, old, &Operand::Value(Value(new)));
        }
    }
}

/// Relabel the exit block's phi edges that named `old_pred` (the header,
/// which no longer branches to the exit) to `new_pred`.
fn relabel_exit_phis(
    func: &mut IrFunction,
    exit_target: BlockId,
    old_pred: BlockId,
    new_pred: BlockId,
) {
    if let Some(exit_bi) = func.blocks.iter().position(|b| b.label == exit_target) {
        for inst in &mut func.blocks[exit_bi].instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                for (_, lbl) in incoming.iter_mut() {
                    if *lbl == old_pred {
                        *lbl = new_pred;
                    }
                }
            }
        }
    }
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
        find_iv_in_loop_ext(func, header, latch, latch_label)
    else {
        return false;
    };

    // Fail closed (correctness AND codegen, see
    // `body_contains_persisting_inner_loop`): refuse to unroll when the body
    // contains a nested inner loop that would survive as a runtime loop in
    // every clone — either its trip count depends on a runtime value, or it
    // is a shape the complete cloner cannot cascade (no Add-IV, no clean
    // latch/exit).  The surviving sibling runtime reductions reuse one
    // accumulator, which the downstream late vectorizer miscompiles; staying
    // rolled is correct and matches GCC for these shapes.
    if body_contains_persisting_inner_loop(func, cfg, lp, iv_phi) {
        return false;
    }

    // Exit from the header's CondBranch.
    let Some((exit_target, body_entry, raw_cmp_op, cmp_ty, exit_limit, iv_is_lhs, exit_pos)) =
        find_exit_condition(func, header, &lp.body, iv_phi, true)
    else {
        return false;
    };
    // The compare must be in the IV's own type, and its operator is put into
    // the canonical "continue while iv OP limit" form before any arithmetic
    // (the IV may be the RIGHT operand and the CondBranch may send `true` to
    // the EXIT; feeding the raw operator into the closed form computes the
    // trip count of a different loop).
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

    // ── Per-iteration SSA environment (header-phi model) ────────────────────
    let Some(phis) = collect_header_phis(func, header, latch_label) else {
        return false;
    };
    let iv_id = iv_phi.0;
    let iv_incr_dest = phis
        .iter()
        .find(|p| p.id == iv_id)
        .and_then(|p| match p.back {
            Operand::Value(v) => Some(v.0),
            _ => None,
        });
    let mut loop_defs: FxHashSet<u32> = FxHashSet::default();
    for &hi in &header_nonphi {
        if let Some(d) = func.blocks[header].instructions[hi].dest() {
            loop_defs.insert(d.0);
        }
    }
    for &bi in &body_blocks {
        for inst in &func.blocks[bi].instructions {
            if let Some(d) = inst.dest() {
                loop_defs.insert(d.0);
            }
        }
    }
    let model = LoopPhiModel {
        phis,
        iv_id,
        iv_incr_dest,
        iv_ty,
        iv_init,
        iv_step,
        loop_defs,
    };
    // Header instructions beyond the (pure) exit-test chain: they execute on
    // every evaluation of the loop condition, i.e. `trip + 1` times. The last
    // evaluation is the failing one, which must run exactly once more after
    // the final cloned iteration.
    let header_extra: Vec<usize> = header_extra_indices(func, header);
    let has_header_extra = !header_extra.is_empty();
    // A header instruction with a real observable side effect (volatile read,
    // store, call, asm) that we do not model: leave the loop rolled rather
    // than drop the extra failing evaluation. Pure arithmetic (a `cnt++`
    // counter, an address computation) is safe to model.
    if has_header_extra
        && header_extra.iter().any(|&hi| {
            matches!(
                &func.blocks[header].instructions[hi],
                Instruction::Store { .. }
                    | Instruction::Call { .. }
                    | Instruction::CallIndirect { .. }
                    | Instruction::InlineAsm { .. }
                    | Instruction::Load { volatile: true, .. }
            )
        })
    {
        return false;
    }

    // ── Build clones for iterations 1..trip ─────────────────────────────────
    let mut next_label = func.blocks.iter().map(|b| b.label.0).max().unwrap_or(0) + 1;
    let mut next_val = func.next_value_id;
    let num_clones = (trip - 1) as usize;

    struct ClonePlan {
        header_copy: BlockId,
        block_labels: Vec<BlockId>, // parallel to body_blocks
        vmap: FxHashMap<u32, u32>,
        env: FxHashMap<u32, Operand>,
    }
    let mut plans: Vec<ClonePlan> = Vec::with_capacity(num_clones);

    // Iteration 0's environment is the preheader init; iteration 0 uses the
    // ORIGINAL (un-renamed) body so its "rename map" is empty.
    let mut env_prev = model.env_entry();
    let mut rename_prev: FxHashMap<u32, u32> = FxHashMap::default();

    for ci in 0..num_clones {
        let t = (ci + 1) as i64; // clone `t` = iteration index after `t` strides
        let env = model.env_next(t, &env_prev, &rename_prev);
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
        // Body defs (skip the dead latch IV increment).
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
        env_prev = env.clone();
        rename_prev = vmap.clone();
        plans.push(ClonePlan {
            header_copy,
            block_labels,
            vmap,
            env,
        });
    }

    let mut new_blocks: Vec<BasicBlock> = Vec::new();

    // ── Final guard (the failing trip-th evaluation) ────────────────────────
    // Runs the header_extra instructions once more with the last clone's
    // values, then falls to the exit. Its fresh outputs are the post-loop
    // live-out values of those header instructions (e.g. `cnt` after a
    // `for (i=0; cnt++, i<N; i++)` is N+1, not the value at the last clone's
    // entry).
    let mut extra_final: FxHashMap<u32, u32> = FxHashMap::default();
    let mut final_guard: Option<BlockId> = None;
    // Failing (trip-th) evaluation environment: derived from the LAST
    // executed iteration — the last clone when trip >= 2, iteration 0 (the
    // original, un-renamed body) when the loop runs exactly once.
    let empty_vmap: FxHashMap<u32, u32> = FxHashMap::default();
    let (env_fg, last_vmap) = match plans.last() {
        Some(last) => (model.env_next(trip, &last.env, &last.vmap), &last.vmap),
        None => (
            model.env_next(trip, &model.env_entry(), &empty_vmap),
            &empty_vmap,
        ),
    };
    if has_header_extra {
        let fg_label = BlockId(next_label);
        next_label += 1;
        let mut fg_vmap: FxHashMap<u32, u32> = FxHashMap::default();
        for &hi in &header_extra {
            if let Some(d) = func.blocks[header].instructions[hi].dest() {
                fg_vmap.insert(d.0, next_val);
                next_val += 1;
            }
        }
        let mut fg_insts: Vec<Instruction> = Vec::new();
        for &hi in &header_extra {
            let mut cloned = func.blocks[header].instructions[hi].clone();
            // header_extra → this guard's own fresh outputs first, then the
            // last clone's values, then substitute the header phis with the
            // failing-entry environment.
            replace_values_in_inst(&mut cloned, &fg_vmap);
            replace_values_in_inst(&mut cloned, last_vmap);
            rename_inst_dest(&mut cloned, &fg_vmap);
            apply_env_inst(&mut cloned, &env_fg);
            fg_insts.push(cloned);
        }
        for &hi in &header_extra {
            if let Some(d) = func.blocks[header].instructions[hi].dest() {
                extra_final.insert(d.0, fg_vmap[&d.0]);
            }
        }
        new_blocks.push(BasicBlock {
            label: fg_label,
            instructions: fg_insts,
            terminator: Terminator::Branch(exit_target),
            source_spans: Vec::new(),
        });
        final_guard = Some(fg_label);
    }

    // ── Emit the clone blocks ───────────────────────────────────────────────
    for (ci, plan) in plans.iter().enumerate() {
        // Original label → clone label (header + body).
        let mut label_map: FxHashMap<BlockId, BlockId> = FxHashMap::default();
        label_map.insert(header_label, plan.header_copy);
        for (i, &bi) in body_blocks.iter().enumerate() {
            label_map.insert(func.blocks[bi].label, plan.block_labels[i]);
        }
        let back_redirect = if ci + 1 < num_clones {
            plans[ci + 1].header_copy
        } else {
            final_guard.unwrap_or(exit_target)
        };

        // Header copy: non-phi instructions, unconditional Branch into the
        // body entry (trip ≥ 2 ⇒ every cloned iteration executes). Pure
        // exit-test chain instructions are cloned too and DCE'd away.
        {
            let mut insts: Vec<Instruction> = Vec::new();
            for &hi in &header_nonphi {
                let mut cloned = func.blocks[header].instructions[hi].clone();
                // ORDER: rename first, substitute header-phi references after.
                // Clone 1's env values are ORIGINAL body ids also present in
                // this clone's vmap; substituting first would let the rename
                // rewrite them into this clone's own definitions.
                replace_values_in_inst(&mut cloned, &plan.vmap);
                rename_inst_dest(&mut cloned, &plan.vmap);
                apply_env_inst(&mut cloned, &plan.env);
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
                replace_values_in_inst(&mut cloned, &plan.vmap);
                rename_inst_dest(&mut cloned, &plan.vmap);
                apply_env_inst(&mut cloned, &plan.env);
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
            replace_values_in_terminator(&mut term, &plan.vmap);
            if bi == latch {
                if let Terminator::Branch(lbl) = &mut term {
                    if *lbl == header_label {
                        *lbl = back_redirect;
                    }
                }
            }
            replace_block_ids(&mut term, &label_map);
            apply_env_term(&mut term, &plan.env);
            new_blocks.push(BasicBlock {
                label: plan.block_labels[i],
                instructions: insts,
                terminator: term,
                source_spans: Vec::new(),
            });
        }
    }

    func.next_value_id = next_val;
    // Keep the label counter ahead of the clone labels minted above: every
    // `BlockId(next_label)` handed out here must stay unique for the rest of
    // the pipeline (a stale counter gave loop_memset colliding labels and
    // detached its guard chain on strcmp-1). `.max()` never moves a healthy
    // (already-ahead) counter backwards.
    func.next_label = next_label.max(func.next_label);

    // ── Mutate the originals ────────────────────────────────────────────────
    // Iteration 0 keeps the original header + body. The header's terminator
    // becomes an unconditional Branch into the body entry.
    func.blocks[header].terminator = Terminator::Branch(body_entry);
    // The original latch's back-edge flows into clone 1's header copy — or
    // straight to the exit (final guard when the header carries extra
    // instructions) when the loop runs exactly once and no clone exists.
    func.blocks[latch].terminator = Terminator::Branch(match plans.first() {
        Some(first) => first.header_copy,
        None => final_guard.unwrap_or(exit_target),
    });

    // ── Post-loop substitution for blocks outside the loop ──────────────────
    // The failing-entry environment holds the live-out value of every ordinary
    // carried value (an accumulator `sum` is its value after the last body).
    // A header-extra-backed phi (e.g. `cnt` whose back value is `cnt + 1` in
    // the header) instead takes the final guard's fresh output: that captures
    // the one-extra failing evaluation.
    let extra_out_ids: FxHashSet<u32> = extra_final.keys().copied().collect();
    let mut final_subst: FxHashMap<u32, Operand> = FxHashMap::default();
    final_subst.insert(iv_id, Operand::Const(IrConst::from_i64(final_iv_n, iv_ty)));
    for (&e, &fg) in &extra_final {
        final_subst.insert(e, Operand::Value(Value(fg)));
    }
    for p in &model.phis {
        if p.id == iv_id {
            continue;
        }
        if let Operand::Value(v) = &p.back {
            if extra_out_ids.contains(&v.0) {
                final_subst.insert(p.id, final_subst[&v.0]);
                continue;
            }
        }
        final_subst.insert(p.id, *env_fg.get(&p.id).expect("env covers every phi"));
    }
    for (block_index, block) in func.blocks.iter_mut().enumerate() {
        if lp.body.contains(&block_index) {
            continue;
        }
        for instruction in &mut block.instructions {
            apply_env_inst(instruction, &final_subst);
        }
        apply_env_term(&mut block.terminator, &final_subst);
    }

    // Exit-block phi edges: the edge that arrived from the header now arrives
    // from the last clone's latch (or the final guard, when the guard is the
    // block that branches to the exit).
    if let Some(exit_bi) = func.blocks.iter().position(|b| b.label == exit_target) {
        if exit_bi != header && !lp.body.contains(&exit_bi) {
            let new_pred = final_guard.unwrap_or_else(|| match plans.last() {
                Some(last) => {
                    last.block_labels[body_blocks
                        .iter()
                        .position(|&b| b == latch)
                        .expect("latch is a body block")]
                }
                // trip == 1: no clone; the latch itself branches to the exit.
                None => latch_label,
            });
            relabel_exit_phis(func, exit_target, header_label, new_pred);
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
        find_iv_in_loop_ext(func, header, latch, latch_label)
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
    )) = find_exit_condition(func, header, &lp.body, iv_phi, true)
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

    // ── Header-phi environment model ─────────────────────────────────────────
    // The final value of every header phi (a counter, a "last" element, a
    // loop-invariant-then-set value, or a cross-phi swap) depends on where its
    // back value is computed. The old code carried back values defined in the
    // latch and silently left everything else at its INIT value, which is why
    // `cur = p` observed `cur == 1` forever and `x = y; y = t` observed the
    // initial `x`. Model every header phi explicitly.
    let Some(phis) = collect_header_phis(func, header, latch_label) else {
        return false;
    };
    let iv_id = iv_phi.0;
    let iv_incr_dest = phis
        .iter()
        .find(|p| p.id == iv_id)
        .and_then(|p| match p.back {
            Operand::Value(v) => Some(v.0),
            _ => None,
        });
    let mut loop_defs: FxHashSet<u32> = FxHashSet::default();
    for &wbi in &work_blocks {
        for inst in &func.blocks[wbi].instructions {
            if let Some(d) = inst.dest() {
                loop_defs.insert(d.0);
            }
        }
    }
    for inst in &func.blocks[header].instructions {
        if !matches!(inst, Instruction::Phi { .. }) {
            if let Some(d) = inst.dest() {
                loop_defs.insert(d.0);
            }
        }
    }
    let model = LoopPhiModel {
        phis,
        iv_id,
        iv_incr_dest,
        iv_ty,
        iv_init,
        iv_step,
        loop_defs,
    };
    // A header instruction beyond the (pure) exit-test chain must run on every
    // evaluation of the condition, i.e. `trip + 1` times (the failing
    // evaluation included). The two-block cloner clones only the work blocks,
    // never the header, so it cannot reproduce that; leaving the loop rolled
    // is correct.
    if !header_extra_indices(func, header).is_empty() {
        return false;
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

    let mut new_blocks: Vec<BasicBlock> = Vec::with_capacity(trip as usize);

    // Per-iteration environments. env[0] = preheader init; env[t] for t ≥ 1 is
    // derived from the previous clone's environment and rename map. The last
    // clone's env + rename map feed the failing (`trip`-th) evaluation used
    // for live-out substitution.
    let mut env_prev = model.env_entry();
    let mut rename_prev: FxHashMap<u32, u32> = FxHashMap::default();
    let mut final_carried: Vec<(u32, u32)> = Vec::new();

    for t_idx in 0..trip {
        let t = t_idx as i64;
        let env = if t == 0 {
            model.env_entry()
        } else {
            model.env_next(t, &env_prev, &rename_prev)
        };
        let iv_const = Operand::Const(IrConst::from_i64(iv_init + t * iv_step, iv_ty));
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
                // Rename first, then substitute the header-phi environment.
                // The env values are consts / invariants / previous-clone ids,
                // none of which are keys in this clone's rename map.
                replace_values_in_inst(&mut cloned, &vmap);
                rename_inst_dest(&mut cloned, &vmap);
                subst_value_with_operand(&mut cloned, iv_phi.0, &iv_const);
                apply_env_inst(&mut cloned, &env);
                new_insts.push(cloned);
            }
        }
        if t_idx + 1 == trip {
            // Clone-defined back values (for the copy-before-def guard below).
            for p in &model.phis {
                if p.id == iv_id {
                    continue;
                }
                if let Operand::Value(v) = &p.back {
                    if let Some(&new_id) = vmap.get(&v.0) {
                        final_carried.push((p.id, new_id));
                    }
                }
            }
        }
        env_prev = env;
        rename_prev = vmap;
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
    // Label-counter writeback (see the clone-label note in
    // try_complete_unroll_general): the clone labels minted from the local
    // counter must stay unique pipeline-wide.
    func.next_label = next_label.max(func.next_label);

    func.blocks[header].terminator = Terminator::Branch(labels[0]);
    // The latch is now UNREACHABLE: the header branches into the clone chain
    // and the chain's last block branches straight to `exit_target`, so no
    // edge reaches the latch any more.  (This used to be
    // `Branch(exit_target)`, which fabricated a control-flow edge latch->exit
    // the program does not have: the latch was only ever a predecessor of the
    // header, never of the exit, so any phi in the exit block acquired a
    // predecessor it had no incoming for — a hard SSA violation the verifier
    // reported on linux-cachymod drivers/gpu/drm/i915/display/intel_sprite.c
    // and which silently emitted `xorl %eax,%eax` before the hard gate
    // existed.  Marking the block Unreachable states the truth; DCE/CFG-
    // simplify then delete it.)
    func.blocks[latch].terminator = Terminator::Unreachable;
    let _ = body_entry;

    // Repair the exit block's phis (the exit may be a JOIN carrying phis:
    // `if (cond) { for (...) ... }` merges the loop exit with the skip path).
    // The `header -> exit_target` edge is gone; the block that reaches the
    // exit is the LAST CLONE.  Every phi incoming still labelled
    // `header_label` must be relabelled to that clone (label only — the
    // operands are rewritten to the final values by the substitution sweep
    // below).  A phi incoming labelled `latch_label` cannot exist here (the
    // latch's terminator was verified to be `Branch(header_label)`), but drop
    // any such entry defensively rather than leave a dangling predecessor.
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

    // Live-out substitution. The failing (`trip`-th) environment holds the
    // value each carried phi has after the final cloned iteration: a body
    // accumulator's computed total, a constant back value, a loop-invariant
    // operand, or a cross-phi value — all bound to the last clone's defs.
    let final_iv = Operand::Const(IrConst::from_i64(final_iv_n, iv_ty));
    let env_fg = model.env_next(trip, &env_prev, &rename_prev);
    let mut final_subst: FxHashMap<u32, Operand> = FxHashMap::default();
    final_subst.insert(iv_id, final_iv);
    for p in &model.phis {
        if p.id == iv_id {
            continue;
        }
        final_subst.insert(p.id, *env_fg.get(&p.id).expect("env covers every phi"));
    }
    for (block_index, block) in func.blocks.iter_mut().enumerate() {
        if lp.body.contains(&block_index) {
            continue;
        }
        for instruction in &mut block.instructions {
            apply_env_inst(instruction, &final_subst);
        }
        apply_env_term(&mut block.terminator, &final_subst);
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

// ── Pass B: partial guard-free two-block unrolling ───────────────────────────

/// Partially unroll a two-block counted loop — header (phis + exit test)
/// and a work-carrying latch — by a factor `k` that DIVIDES the known
/// constant trip count, emitting all `k` iteration bodies concatenated in
/// ONE straight-line block between the header and the back edge with NO
/// inter-iteration guards.
///
/// This is the missing middle ground between the two existing unrollers:
/// the complete unroller flattens trips ≤ 16 (≤ 512 expanded instructions)
/// entirely, and `do_unroll` partially unrolls only 3+-block chains
/// (`analyze_loop` requires non-empty `body_work`); the tight two-block
/// counted loop — by far the most common profitable shape (SHA-256's
/// message schedule and round loops, stream transforms, table walks) —
/// was taken by NEITHER and always stayed rolled.
///
/// # Why guard-free is sound
///
/// `trip % k == 0` partitions the source iterations into `trip / k` exact
/// groups. The header guard still runs once per group: control enters the
/// body only while the IV is below the limit, and then the group executes
/// exactly `k` source iterations — the (t)-th iteration is in-bounds for
/// every `t < trip` by the definition of `trip`
/// (`complete_unroll_trip`), and the group containing iterations
/// `g·k .. g·k+k-1` starts at IV `init + g·k·step`, which is below the
/// limit for every `g < trip/k` (the largest is `init + (trip-k)·step`,
/// still the (trip-k)-th iteration's IV — in-bounds). The first group
/// whose start IV reaches the limit exits at the header, exactly as the
/// rolled loop would at iteration `trip`. No mid-group exit can exist.
///
/// # Why one straight-line block is sound (and precious)
///
/// Concatenating the `k` renamed copies of consecutive iterations in
/// program order IS `k` consecutive iterations — every cross-iteration
/// memory dependence (store of iteration `t` feeding load of iteration
/// `t+2`, etc.) is preserved verbatim because the loads/stores keep their
/// relative order. SSA is maintained by renaming: each clone defines
/// fresh values; header-phi references are substituted with the previous
/// clone's definitions (the threading below); the IV advances through
/// explicit `Add` instructions.
///
/// The single-block form is what makes BB-SLP able to pack the group:
/// `run_bb_slp` works per basic block, so the two adjacent stores
/// `m[i]`, `m[i+1]` seed one pack, the isomorphic per-lane trees pack,
/// and `PackKind::MemLoad` fuses the consecutive `m[i-2]/m[i-1]` loads
/// into one vector load — GCC's 2-wide `vpsrld/vpxor` schedule form. The
/// SLP side's own legality rules stay in charge: rule (e) rejects any
/// seed whose lanes would read bytes a seed store writes (the
/// store→load-forwarding shape `m[i] = f(m[i-1])`), so the transform is
/// beneficial exactly where it is safe and falls back to scalar where it
/// is not.
///
/// # Live-out exactness
///
/// The IV phi's back-edge value becomes `Add(iv_{k-1}, step)`, so after
/// the final group the phi reads `init + trip·step` — the same value the
/// rolled loop leaves (each group advances the IV by `k·step`, and
/// `trip/k` groups run). Carried phis thread through the clones and end
/// at the last clone's copy of their back value — the value iteration
/// `trip` computed in the rolled form. Values defined in the latch can
/// never be live out of the loop through the exit edge: the exit is
/// reachable from the header WITHOUT executing the latch (the very first
/// guard evaluation), so SSA dominance already forbids such uses.
///
/// # Fail-closed shape requirements
///
/// * exactly two blocks in the natural loop, single back-edge predecessor
///   (the latch), `Branch(header)` latch terminator;
/// * a basic IV (`find_iv_in_loop_ext`) whose latch increment is the LAST
///   latch instruction (nothing may follow it — a trailing instruction
///   would need clone-local placement semantics this pass does not model);
/// * the exit comparison in the header against a loop-invariant operand
///   (`find_exit_condition`), same type as the IV;
/// * constant IV init (preheader edge) and constant limit → exact trip
///   via `complete_unroll_trip`;
/// * no header instructions beyond the phi + exit-test chain
///   (`header_extra_indices`);
/// * every header phi in the two-incoming preheader/latch form
///   (`collect_header_phis`);
/// * clone-safe latch work: no calls, indirect calls, inline asm, dynamic
///   allocas, or atomics; intrinsics must be pure;
/// * `trip ≥ 2k` (at least two groups — one group is complete-unroll
///   territory), `k ≤ 4`, and `work × k ≤ 512` expanded instructions.
///
/// Returns `true` when the loop was rewritten.
fn try_partial_unroll_two_block(
    func: &mut IrFunction,
    lp: &loop_analysis::NaturalLoop,
    cfg: &CfgAnalysis,
) -> bool {
    if lp.body.len() != 2 {
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
        find_iv_in_loop_ext(func, header, latch, latch_label)
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
    )) = find_exit_condition(func, header, &lp.body, iv_phi, true)
    else {
        return false;
    };
    if cmp_ty != iv_ty {
        return false;
    }
    // 2-block form only: the work lives in the latch itself.
    if body_entry != latch_label {
        return false;
    }
    // WO-2 (red-team audit, 2026-09-18): `body_entry` is, by construction
    // in `find_exit_condition`, one of the header CondBranch's two arm
    // labels — so the check above is exactly the invariant the CFG rewire
    // at the bottom of this function relies on (one arm of the header's
    // terminator IS `latch_label`). Pin it: nothing between here and the
    // rewire may change the header terminator, and the rewire declines
    // (rather than silently corrupting the CFG) if the invariant ever
    // breaks.
    debug_assert!(
        matches!(
            &func.blocks[header].terminator,
            Terminator::CondBranch { true_label, false_label, .. }
                if *true_label == latch_label || *false_label == latch_label
        ),
        "two-block unroller: body_entry == latch_label implies a CondBranch arm is the latch"
    );
    // The IV increment must be the final latch instruction.
    if latch_iv_incr_idx + 1 != func.blocks[latch].instructions.len() {
        return false;
    }

    // Constant IV init from the preheader edge (resolved through the same
    // const-chain evaluator as the complete unrollers).
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
    let cmp_op = canonical_continue_cmp(raw_cmp_op, iv_is_lhs, exit_cond_positive);
    let Some(trip) = complete_unroll_trip(iv_init, limit_n, cmp_op, iv_step, iv_ty) else {
        return false;
    };

    let work_len = latch_iv_incr_idx;
    // An empty work latch (pure counting loop) has nothing to unroll.
    if work_len == 0 {
        return false;
    }

    // PROFITABILITY: the pass exists to make the k concatenated bodies
    // BB-SLP feedstock — `run_bb_slp` is STORE-SEEDED and its lane inputs
    // are LOAD packs, so a latch lacking either can never pack anything
    // and the unroll is pure code-size bloat with zero runtime gain.
    // Measured (2026-09-18, the codegen-quality gate's tolerance breach):
    // gzip_crc32's table walk (loads + the crc rotation only, no store)
    // +30% insns at neutral runtime; glibc_memcmp's compare loops +20.7%
    // at neutral runtime; crc32's inlined fill_data LCG loop (store, no
    // load — a serial dependence chain) +21 lines at neutral runtime.
    // Both must be plain non-volatile accesses through any pointer (the
    // seed rules vet the address shapes later); volatile accesses never
    // participate in packs.
    {
        let work = &func.blocks[latch].instructions[..work_len];
        let has_store = work.iter().any(|inst| {
            matches!(
                inst,
                Instruction::Store {
                    volatile: false,
                    ..
                }
            )
        });
        let has_load = work.iter().any(|inst| {
            matches!(
                inst,
                Instruction::Load {
                    volatile: false,
                    ..
                }
            )
        });
        if !has_store || !has_load {
            return false;
        }
    }

    // Clone-safety scan of the latch work.
    for inst in &func.blocks[latch].instructions[..work_len] {
        match inst {
            Instruction::Call { .. }
            | Instruction::CallIndirect { .. }
            | Instruction::InlineAsm { .. }
            | Instruction::DynAlloca { .. }
            | Instruction::AtomicRmw { .. }
            | Instruction::AtomicCmpxchg { .. }
            | Instruction::AtomicLoad { .. }
            | Instruction::AtomicStore { .. } => return false,
            Instruction::Intrinsic { op, .. } => {
                if !op.is_pure() {
                    return false;
                }
                // VECTORIZED BODIES STAY ROLLED: the loop vectorizer's
                // rolled body is the deliberate final shape (the ARX
                // marker philosophy — `do_unroll`'s guarded form already
                // serves the multi-accumulator FMA exposure case, and the
                // RA's accumulator homing is tuned for the rolled phi
                // web). Unrolling a Vec*-intrinsic chain here both
                // fights that design and demonstrably breaks the
                // accumulator web: the chained `VecWidenAdd`s of an
                // unrolled widening reduction left the phi's register
                // stale (arm_vec_load_offset summed to 0). This pass
                // exists for the SCALAR two-block feedstock the
                // vectorizer declined — the SLP pack takes it from here.
                if op.produces_vector_value() || op.writes_memory_via_args() || op.may_read_memory()
                {
                    return false;
                }
            }
            _ => {}
        }
    }

    // The header must be phis + exit-test chain only.
    if !header_extra_indices(func, header).is_empty() {
        return false;
    }
    let Some(phis) = collect_header_phis(func, header, latch_label) else {
        return false;
    };

    // ROTATION-DOMINATED loops stay rolled: count carried phis whose
    // back-edge value is ANOTHER header phi (a rotation edge — `h = g;
    // g = f; f = e; ...`). A k-shift register of ≥ 3 values is a SERIAL
    // dependence chain (each round consumes the previous round's outputs
    // through t1/t2-class temporaries): unrolling cannot shorten the
    // chain, it only duplicates the transient temporaries on top of the
    // already-high carried-state pressure — the sha256 round loop
    // unrolled ×2 spilled its whole working set through the stack (the
    // measured 1.50× gap became 1.94×, every MAJ term store-forwarding
    // through `%rsp`). The SCHEDULE loop (distance-2 memory recurrence,
    // ZERO rotation edges) is the shape this unroll exists for.
    {
        let phi_ids: FxHashSet<u32> = phis.iter().map(|p| p.id).collect();
        let rotation_edges = phis
            .iter()
            .filter(|p| {
                p.id != iv_phi.0 && matches!(&p.back, Operand::Value(v) if phi_ids.contains(&v.0))
            })
            .count();
        if rotation_edges >= 3 {
            return false;
        }
    }

    // Factor selection: the size-based preference, walked down through
    // powers of two until one divides the trip. k is capped at 4 (a
    // 2-4× two-block body is the SLP/code-size sweet spot; larger
    // factors belong to `do_unroll`'s guarded form).
    //
    // NOTE (routing, verified empirically): the `trip >= 2k` guard below
    // can only bind for trips < 8 — and every trip <= 16 loop whose
    // budget the complete unroller accepts (work*trip <= 512, which any
    // Pass-B-eligible work satisfies) is already complete-unrolled by
    // Pass A before Pass B ever sees it. The guard is therefore a
    // fail-closed INVARIANT for the states that reach this pass today,
    // not a reachable decline — "exactly two groups" is complete-unroll
    // territory by construction (pinned by
    // `two_block_small_trips_are_complete_unrolled_not_partial`).
    let k_pref = choose_unroll_factor(work_len).min(4) as usize;
    let mut k: usize = 1;
    let mut cand_k = k_pref;
    while cand_k >= 2 {
        if trip % (cand_k as i64) == 0 {
            k = cand_k;
            break;
        }
        cand_k /= 2;
    }
    if k < 2 {
        return false;
    }
    if trip < 2 * k as i64 {
        return false;
    }
    if work_len.saturating_mul(k) > 512 {
        return false;
    }

    // ── Mint fresh values and the clone label ─────────────────────────────
    let mut next_val = func.next_value_id;
    let mut fresh = || {
        let v = Value(next_val);
        next_val += 1;
        v
    };
    let body_label = {
        let l = BlockId(func.next_label);
        func.next_label += 1;
        l
    };

    // Per-clone rename maps for the latch work definitions.
    let mut vmaps: Vec<FxHashMap<u32, u32>> = Vec::with_capacity(k);
    for _ in 0..k {
        let mut vmap: FxHashMap<u32, u32> = FxHashMap::default();
        for inst in &func.blocks[latch].instructions[..work_len] {
            if let Some(d) = inst.dest() {
                vmap.insert(d.0, fresh().0);
            }
        }
        vmaps.push(vmap);
    }

    // The IV chain ids, minted BEFORE the phi threading: a carried phi's
    // back edge may reference the IV itself ("x = i" — the phi) or the
    // canonical increment (a GVN-merged "x = i + 1"), and both must thread
    // through the per-clone IV values. iv_vals[j] is the IV clone j runs
    // with (iv_vals[0] = the phi); iv_vals[k] is the post-group value that
    // becomes the phi's new back-edge incoming.
    let mut iv_vals: Vec<Value> = Vec::with_capacity(k + 1);
    iv_vals.push(iv_phi);
    for _ in 1..=k {
        iv_vals.push(fresh());
    }
    // The canonical increment's dest: references to it evaluate to the
    // NEXT clone's IV (iv_vals[c+1] in clone c's context).
    let iv_incr_dest_id: Option<u32> = func.blocks[latch].instructions[latch_iv_incr_idx]
        .dest()
        .map(|d| d.0);

    // Threaded phi operands: thread[j][P] is the operand clone j sees for
    // header phi P. thread[0][P] = P itself; thread[j][P] (j ≥ 1) is P's
    // back value evaluated in clone j-1:
    //   * a latch-defined value → clone j-1's renamed copy;
    //   * the IV phi → iv_vals[j-1] (the IV clone j-1 ran with);
    //   * the canonical IV increment → iv_vals[j] (the post-increment IV
    //     of clone j-1 — a GVN-merged "x = i + 1");
    //   * another header phi Q → thread[j-1][Q] (cross-phi threading);
    //   * anything else (constant, loop-invariant, dominating def) →
    //     verbatim.
    // The IV phi itself is NOT a thread-map key (clone j's references to
    // it are substituted with iv_vals[j] by the combined map below); the
    // subst_back IV arms exist precisely for OTHER phis' back edges that
    // reference the IV — the "x = i" hole: left verbatim, clone j ≥ 2
    // reads the GROUP-START IV and the loop's live-out x becomes the
    // group start instead of the last iteration (measured: trip 64, k 4
    // → x = 60 instead of 63).
    let phi_ids: FxHashSet<u32> = phis.iter().map(|p| p.id).collect();
    let subst_back = |op: &Operand,
                      clone_ctx: usize,
                      vmap: &FxHashMap<u32, u32>,
                      thread_prev: &FxHashMap<u32, Operand>|
     -> Operand {
        match op {
            Operand::Value(v) => {
                if v.0 == iv_phi.0 {
                    Operand::Value(iv_vals[clone_ctx])
                } else if Some(v.0) == iv_incr_dest_id {
                    Operand::Value(iv_vals[clone_ctx + 1])
                } else if let Some(&nv) = vmap.get(&v.0) {
                    Operand::Value(Value(nv))
                } else if phi_ids.contains(&v.0) {
                    thread_prev.get(&v.0).cloned().unwrap_or_else(|| op.clone())
                } else {
                    op.clone()
                }
            }
            _ => op.clone(),
        }
    };
    let mut thread: Vec<FxHashMap<u32, Operand>> = Vec::with_capacity(k);
    {
        let mut t0: FxHashMap<u32, Operand> = FxHashMap::default();
        for p in &phis {
            if p.id != iv_phi.0 {
                t0.insert(p.id, Operand::Value(Value(p.id)));
            }
        }
        thread.push(t0);
    }
    for j in 1..k {
        let mut tj: FxHashMap<u32, Operand> = FxHashMap::default();
        for p in &phis {
            if p.id == iv_phi.0 {
                continue;
            }
            let threaded = subst_back(&p.back, j - 1, &vmaps[j - 1], &thread[j - 1]);
            tj.insert(p.id, threaded);
        }
        thread.push(tj);
    }
    // The phi's new back-edge incoming: the back value evaluated in the
    // LAST clone (k-1).
    let mut back_new: FxHashMap<u32, Operand> = FxHashMap::default();
    for p in &phis {
        if p.id == iv_phi.0 {
            continue;
        }
        back_new.insert(
            p.id,
            subst_back(&p.back, k - 1, &vmaps[k - 1], &thread[k - 1]),
        );
    }

    // ── Build the merged body block ───────────────────────────────────────
    let mut body_insts: Vec<Instruction> = Vec::with_capacity(work_len * k + k);
    // iv chain: iv_vals[0] = the phi; iv_vals[j] = Add(iv_vals[j-1], step)
    // for j in 1..=k (the ids were minted before the phi threading because
    // the threading's IV arms reference them; iv_vals[k] doubles as the
    // phi's new back-edge value).
    for j in 1..=k {
        body_insts.push(Instruction::BinOp {
            dest: iv_vals[j],
            op: IrBinOp::Add,
            lhs: Operand::Value(iv_vals[j - 1]),
            rhs: Operand::Const(IrConst::from_i64(iv_step, iv_ty)),
            ty: iv_ty,
        });
    }
    // Const-backed carried phis (back value a Const — `x = 0` at the end
    // of the body): clones j ≥ 1 read the constant. Materialized ONCE as
    // a Copy at the body top (the const is j-independent), then mapped
    // like every other phi. Without this the u32→u32 rename below could
    // not express the substitution.
    let mut const_temp: FxHashMap<u32, Value> = FxHashMap::default();
    for p in &phis {
        if p.id == iv_phi.0 {
            continue;
        }
        if thread[1..].iter().all(|tj| {
            tj.get(&p.id)
                .is_some_and(|op| matches!(op, Operand::Const(_)))
        }) && thread
            .get(1)
            .and_then(|t1| t1.get(&p.id))
            .is_some_and(|op| matches!(op, Operand::Const(_)))
        {
            let cv = fresh();
            if let Some(Operand::Const(c)) = thread[1].get(&p.id).cloned() {
                body_insts.push(Instruction::Copy {
                    dest: cv,
                    src: Operand::Const(c),
                });
                const_temp.insert(p.id, cv);
            }
        }
    }

    for j in 0..k {
        for inst in &func.blocks[latch].instructions[..work_len] {
            let mut cloned = inst.clone();
            // Every clone — including the first — is renamed to FRESH
            // value ids: the original latch block still physically exists
            // (unreachable, deleted by the next DCE sweep), so keeping any
            // original definition id would double-define it and hand every
            // pass between here and DCE malformed SSA.
            //
            // The phi/IV substitution and the definition rename share ONE
            // combined u32→u32 map applied by `replace_values_in_inst` —
            // a SINGLE-PASS walker over every operand position INCLUDING
            // the bare-Value positions (`Store.ptr`, `GEP.base`, `Load.
            // ptr`, ...) that `for_each_operand_mut` skips. Two hard-won
            // properties:
            //
            // * COMPLETENESS: the fill-loop regression
            //   (`memset_loop_iv_escape`) catches exactly the bare-position
            //   class — a `Store { ptr: phi }` left unsubstituted made
            //   every unrolled clone store through the SAME phi pointer
            //   (buf[0] four times instead of buf[0..3]).
            // * ATOMICITY: the threading maps phi→phi (clone 1's h-slot
            //   is phi g, its g-slot is phi f, ...); applying each phi's
            //   substitution in its own pass would rewrite the values a
            //   previous pass just inserted (the sha256 round loop read
            //   phi f one rotation step too far). One pass over the
            //   combined map looks each position up exactly once.
            if j > 0 {
                let mut combined: FxHashMap<u32, u32> = FxHashMap::default();
                for p in &phis {
                    if p.id == iv_phi.0 {
                        continue;
                    }
                    match thread[j].get(&p.id) {
                        Some(Operand::Value(tv)) => {
                            combined.insert(p.id, tv.0);
                        }
                        Some(Operand::Const(_)) => {
                            if let Some(&cv) = const_temp.get(&p.id) {
                                combined.insert(p.id, cv.0);
                            }
                        }
                        _ => {}
                    }
                }
                combined.insert(iv_phi.0, iv_vals[j].0);
                for (&old, &new) in &vmaps[j] {
                    combined.insert(old, new);
                }
                replace_values_in_inst(&mut cloned, &combined);
            } else {
                replace_values_in_inst(&mut cloned, &vmaps[j]);
            }
            rename_inst_dest(&mut cloned, &vmaps[j]);
            body_insts.push(cloned);
        }
    }
    func.next_value_id = next_val;

    // ── Rewire the CFG ────────────────────────────────────────────────────
    // Header: the continue target becomes the merged body.
    {
        let term = &mut func.blocks[header].terminator;
        match term {
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => {
                if *true_label == latch_label {
                    *true_label = body_label;
                } else if *false_label == latch_label {
                    *false_label = body_label;
                } else {
                    // WO-2 (red-team audit, 2026-09-18): unreachable under
                    // the checked invariant (`body_entry == latch_label` is
                    // verified before any cloning), but this is the one
                    // place in the pass where an invariant violation would
                    // yield silent IR corruption instead of a decline:
                    // relabelling nothing, marking the latch Unreachable,
                    // and appending an unreachable body would strand the
                    // loop with no exit. Decline instead. The only mutation
                    // that precedes this point is the monotonic
                    // `next_value_id` bump — the orphaned clone ids are
                    // simply never issued again.
                    return false;
                }
            }
            _ => return false, // find_exit_condition guaranteed this shape
        }
    }
    // Header phis: relabel the back edge to the body block and substitute
    // the threaded back values.
    for inst in func.blocks[header].instructions.iter_mut() {
        let Instruction::Phi { dest, incoming, .. } = inst else {
            continue;
        };
        let is_iv = dest.0 == iv_phi.0;
        for (op, lbl) in incoming.iter_mut() {
            if *lbl == latch_label {
                *lbl = body_label;
                if is_iv {
                    *op = Operand::Value(iv_vals[k]);
                } else if let Some(new_op) = back_new.get(&dest.0) {
                    *op = new_op.clone();
                }
                // A phi whose back operand is neither latch-defined nor a
                // cross-phi (constant / loop-invariant) keeps it verbatim —
                // exactly the rolled semantics.
            }
        }
    }
    // The latch is unreachable: the header now enters the body and the
    // body branches straight back to the header.
    func.blocks[latch].terminator = Terminator::Unreachable;

    func.blocks.push(BasicBlock {
        label: body_label,
        instructions: body_insts,
        terminator: Terminator::Branch(header_label),
        source_spans: Vec::new(),
    });

    let _ = exit_target;
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

/// STRICT induction-variable detector for the partial-unroll path.
///
/// `do_unroll` was written for Add-form induction; its guard arithmetic and
/// back-edge offset assume `step` is the additive constant of an `Add` in the
/// latch.  The extended detector used by the COMPLETE unrollers also accepts
/// `Sub(phi, k)` countdowns and re-materials unsigned constants through
/// `IrConst::from_i64`, which can turn a U32 `0xFFFFFFFE` step into `-2`.
/// Feeding either to `do_unroll` produces guards the unroller cannot express
/// and silently drops/adds iterations (red-team cfg 3169: `lim != i; i -= 1`
/// with a nested body lost exactly half the accumulator).  Keeping this
/// Add-only detector here restores the baseline acceptance set for the
/// partial path; the extended detector stays confined to the complete
/// unrollers, whose closed-form trip count re-verifies every step.
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
        // Same contract as `find_iv_in_loop_ext`: a non-Value back edge
        // (constant reset, malformed incoming) disqualifies THIS phi as
        // the IV but must not abort the search — header phi order is
        // arbitrary and the real IV can be minted after a const-backed
        // phi.
        let Some(back_val) = back_val else {
            continue;
        };

        // Look for `Add(phi_dest, const_step)` or `Add(const_step, phi_dest)`
        // in the latch that produces `back_val`.
        let phi_id = phi_dest.0;
        for (idx, latch_inst) in func.blocks[latch].instructions.iter().enumerate() {
            if let Instruction::BinOp {
                dest,
                op: IrBinOp::Add,
                lhs,
                rhs,
                ..
            } = latch_inst
            {
                if *dest != back_val {
                    continue;
                }
                let step = match (lhs, rhs) {
                    (Operand::Value(v), Operand::Const(c)) if v.0 == phi_id => c.to_i64(),
                    (Operand::Const(c), Operand::Value(v)) if v.0 == phi_id => c.to_i64(),
                    _ => None,
                };
                if let Some(step) = step {
                    return Some((*phi_dest, *ty, step, idx));
                }
            }
        }
    }
    None
}

fn find_iv_in_loop_ext(
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
        // A phi whose back edge is not a Value (a CONSTANT back edge — the
        // `c = 3;` reset idiom, or a missing latch incoming on a malformed
        // phi) cannot be this loop's IV — but it must not abort the SEARCH
        // either. Header phi order is arbitrary (mem2reg mints phis in
        // discovery order, not IV-first), so a const-backed phi emitted
        // before the real IV used to `?`-bail the whole finder and silently
        // decline the loop (measured: the `s += c*(i+1); c = 3;` battery
        // shape never unrolled while the identical loop with `c = i & 3`
        // did). Skip the phi and keep looking.
        let Some(back_val) = back_val else {
            continue;
        };

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
///
/// `require_loop_invariant_limit` is true at every trip-computing caller:
/// the closed form is only valid for a limit that is fixed for the whole
/// loop.  The persisting-inner-loop gate passes false: it predicts
/// post-substitution cascade-ability by VALUE (`depends_only_on_const_and_iv`
/// on the limit), and a value-invariant limit recomputed in the inner header
/// (g3's `i + 8`: invariant in value, defined in-body) still folds to a
/// per-clone constant once the outer IV is substituted and fold/copyprop run
/// before the next fixpoint round.  Requiring syntactic invariance there
/// vetoes cascades the fixpoint would complete.
fn find_exit_condition(
    func: &IrFunction,
    header: usize,
    loop_body: &FxHashSet<usize>,
    iv_phi: Value,
    require_loop_invariant_limit: bool,
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

    // One Cmp operand must be exactly the IV phi; the other is the limit.
    // Trip-computing callers additionally require a syntactically
    // loop-invariant limit (the closed form is only valid for a fixed
    // limit); the persist gate passes false and decides invariance by
    // VALUE instead (see the doc comment above).
    let (iv_is_lhs, limit_op) = if matches!(cmp_lhs, Operand::Value(v) if v.0 == iv_id)
        && (!require_loop_invariant_limit || is_loop_invariant_op(cmp_rhs, loop_body, func))
    {
        (true, cmp_rhs)
    } else if matches!(cmp_rhs, Operand::Value(v) if v.0 == iv_id)
        && (!require_loop_invariant_limit || is_loop_invariant_op(cmp_lhs, loop_body, func))
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
                func.next_label = next_label.max(func.next_label);
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
        // On those edges the loop never ran, so the live value is the one
        // the header phi carries at loop ENTRY: its non-latch incoming
        // (the preheader operand).  The header phi itself is NOT sound
        // here — it is defined in the header, which no foreign path
        // executes, so an incoming naming it makes the join read a
        // register no path defined (the def-dominates-use verifier
        // rejects exactly this shape).  The entry operand is defined at
        // the end of the preheader, which dominates every foreign
        // predecessor of the exit.
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

        // Entry-edge (non-latch) operand of every header phi in `need`:
        // the value each carried value holds before the loop runs.  A
        // single-entry loop — the only shape this unroller admits — has
        // exactly one; anything else has no sound foreign-edge operand
        // and skips synthesis rather than guessing.
        let mut entry_ops: FxHashMap<u32, Operand> = FxHashMap::default();
        for inst in &func.blocks[c.header].instructions {
            if let Instruction::Phi { dest, incoming, .. } = inst {
                let mut non_latch = incoming.iter().filter(|(_, l)| *l != latch_label);
                if let (Some((op, _)), None) = (non_latch.next(), non_latch.next()) {
                    entry_ops.insert(dest.0, op.clone());
                }
            }
        }

        for (dest, edges, pty) in &need {
            if existing.contains_key(&dest.0) {
                continue; // Step 5 already threaded this phi's edges
            }
            let Some(entry_op) = entry_ops.get(&dest.0) else {
                continue; // no unique entry operand; no sound foreign edge
            };
            let new_phi = Value(next_val);
            next_val += 1;
            let mut incoming: Vec<(Operand, BlockId)> =
                Vec::with_capacity(num_new + 1 + foreign_preds.len());
            incoming.push((Operand::Value(*dest), header_label));
            for (j, ev) in edges.iter().enumerate() {
                incoming.push((ev.clone(), ec_labels[j]));
            }
            for fp in &foreign_preds {
                incoming.push((entry_op.clone(), *fp));
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
                    // The terminator can also USE the carried value: a
                    // `Return(H)` after a counted loop is the canonical
                    // late-exit reader, and its operand is not covered by
                    // the instruction loop above (red-team cfg 1765:
                    // `i <= lim; i += 7` with an inlined chain body returned
                    // the phi's PREHEADER value — 0 instead of 30 — because
                    // every early exit-check edge read the un-updated
                    // header phi).
                    subst_value_in_terminator(
                        &mut func.blocks[bi].terminator,
                        dest.0,
                        &Operand::Value(new_phi),
                    );
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
    // Label-counter writeback: the exit-check/clone labels minted from the
    // local counter must stay unique pipeline-wide (stale counters hand
    // later passes colliding labels).
    func.next_label = next_label.max(func.next_label);
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

        // B0: preheader — init i = 0; materialize the array base the
        // body GEPs use. The base is loop-invariant and defined outside,
        // so it must have a defining instruction here or the verifier's
        // def-dominates-use check (rightly) reports every cloned GEP as
        // reading an undefined value.
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I32(0)),
                },
                Instruction::GlobalAddr {
                    dest: Value(10),
                    name: "arr".to_string(),
                },
            ],
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
        func.next_label = 5; // labels 0–4 used
        func
    }

    /// Two-block counted loop (header + work-carrying latch) with a
    /// carried phi X whose back edge references `back` — used to pin the
    /// Pass B phi-threading contract for the two IV-reference shapes:
    /// `back == iv_phi` (the C `x = i;`) and `back == the latch's Add
    /// dest` (the GVN-merged `x = i + 1`).
    ///
    /// The latch carries REAL SLP feedstock (`arr[iv] = X + src[iv]` — a
    /// non-volatile load AND store): the profitability gate declines
    /// latch work without both, and these fixtures exist to exercise the
    /// threading of the transform WHEN IT FIRES, so they must model the
    /// shapes it fires on.
    ///
    /// Shape: B0 preheader (iv init, X init, arr/src bases) → B1 header
    /// (iv phi %1, X phi %2, exit cmp %3 < trip) → B2 latch (GEP+GEP+
    /// load src[iv], Add X+loaded, store arr[iv], iv Add %5 LAST) → B1;
    /// B4 exit uses X (live-out).
    fn make_two_block_iv_back_loop(trip: i32, x_back_is_iv_phi: bool) -> IrFunction {
        let mut func = IrFunction::new("two_block_test".to_string(), IrType::Void, vec![], false);
        let x_back = if x_back_is_iv_phi {
            Operand::Value(Value(1)) // the IV phi itself
        } else {
            Operand::Value(Value(5)) // the latch's canonical increment
        };
        // B0: preheader
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I32(0)),
                },
                Instruction::Copy {
                    dest: Value(11),
                    src: Operand::Const(IrConst::I32(7)),
                },
                Instruction::GlobalAddr {
                    dest: Value(10),
                    name: "arr".to_string(),
                },
                Instruction::GlobalAddr {
                    dest: Value(12),
                    name: "src".to_string(),
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        // B1: header — iv phi, X phi (back edge = the chosen IV shape), cmp
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(1),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(5)), BlockId(2)),
                    ],
                },
                Instruction::Phi {
                    dest: Value(2),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(11)), BlockId(0)),
                        (x_back, BlockId(2)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(3),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(trip)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(3)),
                true_label: BlockId(2),
                false_label: BlockId(4),
            },
            source_spans: Vec::new(),
        });
        // B2: latch — load src[iv], store arr[iv] = X + loaded, iv Add LAST
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::GetElementPtr {
                    dest: Value(4),
                    base: Value(10),
                    offset: Operand::Value(Value(1)),
                    ty: IrType::I32,
                },
                Instruction::GetElementPtr {
                    dest: Value(13),
                    base: Value(12),
                    offset: Operand::Value(Value(1)),
                    ty: IrType::I32,
                },
                Instruction::Load {
                    dest: Value(14),
                    ptr: Value(13),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                Instruction::BinOp {
                    dest: Value(15),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Value(Value(14)),
                    ty: IrType::I32,
                },
                Instruction::Store {
                    volatile: false,
                    val: Operand::Value(Value(15)),
                    ptr: Value(4),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        // B4: exit — X is live out (its exactness is the contract)
        func.blocks.push(BasicBlock {
            label: BlockId(4),
            instructions: vec![
                Instruction::GetElementPtr {
                    dest: Value(6),
                    base: Value(10),
                    offset: Operand::Value(Value(2)),
                    ty: IrType::I32,
                },
                Instruction::Store {
                    volatile: false,
                    val: Operand::Value(Value(2)),
                    ptr: Value(6),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        func.next_value_id = 16; // 0–15 used (10/12 = arr/src placeholders)
        func.next_label = 5;
        func
    }

    /// Shared Pass B contract check: after the PostVec unroll of the
    /// two-block loop, the carried phi X (Value(2)) must NOT keep an
    /// IV-shaped verbatim back edge — its back incoming must be a value
    /// defined by an Add inside the merged body block (the threaded
    /// per-clone IV chain), and the live body must not reference the
    /// dead original latch's definitions.
    fn assert_two_block_iv_threading(func: &IrFunction, unrolled: usize) {
        assert_eq!(unrolled, 1, "the two-block loop must unroll");
        // The merged body block: the header's non-exit CondBranch
        // successor (the preheader also branches to the header, and the
        // original latch is now Unreachable — neither is the body).
        let body_label = {
            let header = func
                .blocks
                .iter()
                .find(|b| b.label == BlockId(1))
                .expect("header survives");
            match &header.terminator {
                Terminator::CondBranch {
                    true_label,
                    false_label,
                    ..
                } => {
                    if *true_label != BlockId(4) {
                        *true_label
                    } else {
                        *false_label
                    }
                }
                other => panic!("header lost its exit branch: {other:?}"),
            }
        };
        let body = func
            .blocks
            .iter()
            .find(|b| b.label == body_label)
            .expect("merged body block exists");
        // The adds defined in the body (the iv chain) — legal threading
        // targets for X's back edge.
        let body_adds: Vec<u32> = body
            .instructions
            .iter()
            .filter_map(|i| match i {
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    ..
                } => Some(dest.0),
                _ => None,
            })
            .collect();
        assert!(
            body_adds.len() >= 2,
            "the iv chain must materialize (found {} adds)",
            body_adds.len()
        );
        // X's phi: the back-edge incoming (from the body label) must be one
        // of the body's Adds — NOT the iv phi (Value(1), the group-start
        // miscompile) and NOT the dead latch's Value(5).
        let x_phi = func
            .blocks
            .iter()
            .find(|b| b.label == BlockId(1))
            .unwrap()
            .instructions
            .iter()
            .find_map(|i| match i {
                Instruction::Phi { dest, incoming, .. } if dest.0 == 2 => Some(incoming.clone()),
                _ => None,
            })
            .expect("X phi survives");
        let back = x_phi
            .iter()
            .find(|(_, lbl)| *lbl == body.label)
            .map(|(op, _)| op.clone())
            .expect("X has a back-edge incoming from the body");
        match back {
            Operand::Value(v) => {
                assert!(
                    body_adds.contains(&v.0),
                    "X's back edge must thread to the body's iv chain (got Value({}), adds {:?})",
                    v.0,
                    body_adds
                );
                assert_ne!(
                    v.0, 1,
                    "X's back edge must not be the iv phi (group-start IV)"
                );
                assert_ne!(v.0, 5, "X's back edge must not be the dead latch increment");
            }
            other => panic!("X's back edge must be a value, got {other:?}"),
        }
        // The live body must not reference the dead latch's definitions
        // (Value(5)) anywhere.
        for inst in &body.instructions {
            let mut probe = inst.clone();
            let mut bad = false;
            probe.for_each_operand_mut(|op| {
                if matches!(op, Operand::Value(v) if v.0 == 5) {
                    bad = true;
                }
            });
            assert!(!bad, "live body references the dead latch increment");
        }
    }

    #[test]
    fn two_block_unroll_threads_iv_referencing_phi() {
        // The C `x = i;` — the carried phi's back edge IS the iv phi.
        // Before the threading fix this left the group-start IV in every
        // clone >= 2 and in the live-out (measured: x = 60, not 63).
        let mut func = make_two_block_iv_back_loop(64, true);
        let n = unroll_loops(&mut func, UnrollPhase::PostVec);
        assert_two_block_iv_threading(&func, n);
    }

    #[test]
    fn two_block_unroll_threads_increment_referencing_phi() {
        // The GVN-merged `x = i + 1;` — the carried phi's back edge is the
        // latch's canonical Add. Verbatim threading would dangle on the
        // dead latch; the fix threads to the per-clone post-increment IVs.
        let mut func = make_two_block_iv_back_loop(64, false);
        let n = unroll_loops(&mut func, UnrollPhase::PostVec);
        assert_two_block_iv_threading(&func, n);
    }

    #[test]
    fn two_block_unroll_declines_when_killed() {
        // CCC_NO_TWO_BLOCK_UNROLL must leave the loop rolled.  The switch is
        // per-thread state rather than the process environment, so this test
        // cannot race the sibling `two_block_unroll_*` tests that run on the
        // other threads of the same test binary and expect Pass B enabled.
        set_two_block_unroll_enabled(false);
        let mut func = make_two_block_iv_back_loop(64, true);
        let n = unroll_loops(&mut func, UnrollPhase::PostVec);
        set_two_block_unroll_enabled(true);
        assert_eq!(n, 0, "kill switch must disable Pass B");
        // The loop is intact: latch still branches to the header.
        assert!(matches!(
            func.blocks.iter().find(|b| b.label == BlockId(2)).unwrap().terminator,
            Terminator::Branch(l) if l == BlockId(1)
        ));
    }

    /// Latch memory profile for the profitability-gate fixtures: the pass
    /// exists to manufacture BB-SLP feedstock (store-seeded packs with
    /// load-pack lane inputs), so the latch's non-volatile access census
    /// decides profitability.
    #[derive(Clone, Copy, PartialEq)]
    enum LatchFeed {
        /// Plain load + store — the packable feedstock: FIRES.
        LoadStore,
        /// Store only (fill loop): pure code-size bloat at neutral runtime
        /// (measured: crc32's fill_data LCG +21 lines) — DECLINES.
        StoreOnly,
        /// Load only (table walk / compare loop): bloat (measured:
        /// gzip_crc32 +30% insns, glibc_memcmp +20.7%) — DECLINES.
        LoadOnly,
        /// Volatile load + store only: volatile accesses never join packs,
        /// so there is no feedstock either — DECLINES.
        VolatileOnly,
        /// Plain load + store PLUS a volatile store: fires, and the
        /// volatile side effect must be cloned per iteration exactly.
        MixedVolatile,
    }

    /// Two-block counted loop with an accumulator phi and a configurable
    /// latch memory profile — the profitability-gate fixture family.
    ///
    /// Shape: B0 preheader → B1 header (iv phi %1, acc phi %2, cmp %3) →
    /// B2 latch (profile-dependent work, iv Add %5 LAST) → B1; B4 exit
    /// stores acc (live-out exactness).
    fn make_two_block_feed_loop(trip: i32, feed: LatchFeed) -> IrFunction {
        let mut func = IrFunction::new("feed_loop_test".to_string(), IrType::Void, vec![], false);
        let has_load = matches!(
            feed,
            LatchFeed::LoadStore
                | LatchFeed::LoadOnly
                | LatchFeed::VolatileOnly
                | LatchFeed::MixedVolatile
        );
        let has_store = !matches!(feed, LatchFeed::LoadOnly);
        let vol = matches!(feed, LatchFeed::VolatileOnly);
        // B0: preheader
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I32(0)),
                },
                Instruction::Copy {
                    dest: Value(11),
                    src: Operand::Const(IrConst::I32(7)),
                },
                Instruction::GlobalAddr {
                    dest: Value(10),
                    name: "arr".to_string(),
                },
                Instruction::GlobalAddr {
                    dest: Value(12),
                    name: "src".to_string(),
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        // B1: header — iv phi, acc phi (back edge = the latch's work Add),
        // exit cmp
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(1),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(5)), BlockId(2)),
                    ],
                },
                Instruction::Phi {
                    dest: Value(2),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(11)), BlockId(0)),
                        (Operand::Value(Value(15)), BlockId(2)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(3),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(trip)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(3)),
                true_label: BlockId(2),
                false_label: BlockId(4),
            },
            source_spans: Vec::new(),
        });
        // B2: latch — the profile-dependent work, iv Add LAST
        let mut work: Vec<Instruction> = Vec::new();
        work.push(Instruction::GetElementPtr {
            dest: Value(4),
            base: Value(10),
            offset: Operand::Value(Value(1)),
            ty: IrType::I32,
        });
        if has_load {
            work.push(Instruction::GetElementPtr {
                dest: Value(13),
                base: Value(12),
                offset: Operand::Value(Value(1)),
                ty: IrType::I32,
            });
            work.push(Instruction::Load {
                dest: Value(14),
                ptr: Value(13),
                ty: IrType::I32,
                seg_override: AddressSpace::Default,
                volatile: vol,
            });
            work.push(Instruction::BinOp {
                dest: Value(15),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Value(Value(14)),
                ty: IrType::I32,
            });
        } else {
            work.push(Instruction::BinOp {
                dest: Value(15),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            });
        }
        if has_store {
            work.push(Instruction::Store {
                volatile: vol,
                val: Operand::Value(Value(15)),
                ptr: Value(4),
                ty: IrType::I32,
                seg_override: AddressSpace::Default,
            });
        }
        if feed == LatchFeed::MixedVolatile {
            // The observable side effect riding along the packable work:
            // cloned per iteration, never packed.
            work.push(Instruction::Store {
                volatile: true,
                val: Operand::Value(Value(2)),
                ptr: Value(4),
                ty: IrType::I32,
                seg_override: AddressSpace::Default,
            });
        }
        work.push(Instruction::BinOp {
            dest: Value(5),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(1)),
            ty: IrType::I32,
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: work,
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        // B4: exit — acc is live out
        func.blocks.push(BasicBlock {
            label: BlockId(4),
            instructions: vec![
                Instruction::GetElementPtr {
                    dest: Value(6),
                    base: Value(10),
                    offset: Operand::Const(IrConst::I32(0)),
                    ty: IrType::I32,
                },
                Instruction::Store {
                    volatile: false,
                    val: Operand::Value(Value(2)),
                    ptr: Value(6),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        func.next_value_id = 16; // 0–15 used
        func.next_label = 5;
        func
    }

    /// The merged body block of an unrolled two-block loop: the header's
    /// non-exit CondBranch successor (the preheader also branches to the
    /// header and the original latch is now Unreachable — neither is the
    /// body).
    fn merged_body_of(func: &IrFunction) -> &BasicBlock {
        let header = func
            .blocks
            .iter()
            .find(|b| b.label == BlockId(1))
            .expect("header survives");
        let body_label = match &header.terminator {
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => {
                if *true_label != BlockId(4) {
                    *true_label
                } else {
                    *false_label
                }
            }
            other => panic!("header lost its exit branch: {other:?}"),
        };
        func.blocks
            .iter()
            .find(|b| b.label == body_label)
            .expect("merged body block exists")
    }

    #[test]
    fn two_block_unroll_profitability_positive_control() {
        // Plain load + store feedstock: the unroll must FIRE.
        let mut func = make_two_block_feed_loop(64, LatchFeed::LoadStore);
        let n = unroll_loops(&mut func, UnrollPhase::PostVec);
        assert_eq!(n, 1, "load+store latch must unroll");
    }

    #[test]
    fn two_block_unroll_declines_store_only_latch() {
        // A store-only latch (fill loop) has no load packs to feed the
        // store seeds: the unroll is pure bloat and must decline.
        let mut func = make_two_block_feed_loop(64, LatchFeed::StoreOnly);
        let n = unroll_loops(&mut func, UnrollPhase::PostVec);
        assert_eq!(n, 0, "store-only latch must decline (fill-loop bloat)");
        assert!(matches!(
            func.blocks.iter().find(|b| b.label == BlockId(2)).unwrap().terminator,
            Terminator::Branch(l) if l == BlockId(1)
        ));
    }

    #[test]
    fn two_block_unroll_declines_load_only_latch() {
        // A load-only latch (table walk / compare loop) has no store seeds
        // at all: must decline.
        let mut func = make_two_block_feed_loop(64, LatchFeed::LoadOnly);
        let n = unroll_loops(&mut func, UnrollPhase::PostVec);
        assert_eq!(n, 0, "load-only latch must decline (no store seeds)");
        assert!(matches!(
            func.blocks.iter().find(|b| b.label == BlockId(2)).unwrap().terminator,
            Terminator::Branch(l) if l == BlockId(1)
        ));
    }

    #[test]
    fn two_block_unroll_declines_volatile_only_latch() {
        // Volatile accesses never join packs: a volatile-only latch has no
        // feedstock and must decline (the same bloat class, plus volatile
        // loops are I/O-shaped where branch overhead is noise).
        let mut func = make_two_block_feed_loop(64, LatchFeed::VolatileOnly);
        let n = unroll_loops(&mut func, UnrollPhase::PostVec);
        assert_eq!(n, 0, "volatile-only latch must decline");
        assert!(matches!(
            func.blocks.iter().find(|b| b.label == BlockId(2)).unwrap().terminator,
            Terminator::Branch(l) if l == BlockId(1)
        ));
    }

    #[test]
    fn two_block_unroll_clones_volatile_riding_packable_work() {
        // Mixed latch: plain load + store (feedstock) PLUS a volatile
        // store. The unroll fires, and the volatile side effect must be
        // cloned into EVERY iteration of the merged body — k plain loads,
        // k plain stores, k volatile stores, no more, no fewer.
        let mut func = make_two_block_feed_loop(64, LatchFeed::MixedVolatile);
        let n = unroll_loops(&mut func, UnrollPhase::PostVec);
        assert_eq!(
            n, 1,
            "mixed latch (plain load+store + volatile) must unroll"
        );
        let body = merged_body_of(&func);
        let plain_loads = body
            .instructions
            .iter()
            .filter(|i| {
                matches!(
                    i,
                    Instruction::Load {
                        volatile: false,
                        ..
                    }
                )
            })
            .count();
        let plain_stores = body
            .instructions
            .iter()
            .filter(|i| {
                matches!(
                    i,
                    Instruction::Store {
                        volatile: false,
                        ..
                    }
                )
            })
            .count();
        let vol_stores = body
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::Store { volatile: true, .. }))
            .count();
        assert!(
            plain_loads >= 2 && plain_stores >= 2,
            "the body must be a k-fold clone (loads={plain_loads}, stores={plain_stores})"
        );
        assert_eq!(
            plain_loads, plain_stores,
            "each clone carries one plain load and one plain store"
        );
        assert_eq!(
            vol_stores, plain_stores,
            "the volatile store must be cloned exactly once per clone"
        );
    }

    /// Two-block counted loop whose header mints a CONST-BACKED carried
    /// phi (%2, back edge `Const(3)`) BEFORE the IV phi (%1) — the phi
    /// order that used to make `find_iv_in_loop_ext`'s `?`-bail abort the
    /// IV search and silently decline the whole loop.
    fn make_two_block_const_phi_before_iv(trip: i32) -> IrFunction {
        let mut func = IrFunction::new("const_phi_test".to_string(), IrType::Void, vec![], false);
        // B0: preheader
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I32(0)),
                },
                Instruction::Copy {
                    dest: Value(11),
                    src: Operand::Const(IrConst::I32(7)),
                },
                Instruction::GlobalAddr {
                    dest: Value(10),
                    name: "arr".to_string(),
                },
                Instruction::GlobalAddr {
                    dest: Value(12),
                    name: "src".to_string(),
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        // B1: header — the CONST-BACKED phi comes FIRST (the discovery
        // order mem2reg would mint for `int c = 7; ... c = 3;` declared
        // before the loop's IV), then the iv phi, then the exit cmp.
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(2),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(11)), BlockId(0)),
                        (Operand::Const(IrConst::I32(3)), BlockId(2)),
                    ],
                },
                Instruction::Phi {
                    dest: Value(1),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(5)), BlockId(2)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(3),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(trip)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(3)),
                true_label: BlockId(2),
                false_label: BlockId(4),
            },
            source_spans: Vec::new(),
        });
        // B2: latch — load src[iv], store arr[iv] = C + loaded, iv Add LAST
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::GetElementPtr {
                    dest: Value(4),
                    base: Value(10),
                    offset: Operand::Value(Value(1)),
                    ty: IrType::I32,
                },
                Instruction::GetElementPtr {
                    dest: Value(13),
                    base: Value(12),
                    offset: Operand::Value(Value(1)),
                    ty: IrType::I32,
                },
                Instruction::Load {
                    dest: Value(14),
                    ptr: Value(13),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                Instruction::BinOp {
                    dest: Value(15),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Value(Value(14)),
                    ty: IrType::I32,
                },
                Instruction::Store {
                    volatile: false,
                    val: Operand::Value(Value(15)),
                    ptr: Value(4),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        // B4: exit — C is live out (its post-loop value must be the const)
        func.blocks.push(BasicBlock {
            label: BlockId(4),
            instructions: vec![
                Instruction::GetElementPtr {
                    dest: Value(6),
                    base: Value(10),
                    offset: Operand::Const(IrConst::I32(0)),
                    ty: IrType::I32,
                },
                Instruction::Store {
                    volatile: false,
                    val: Operand::Value(Value(2)),
                    ptr: Value(6),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        func.next_value_id = 16;
        func.next_label = 5;
        func
    }

    #[test]
    fn two_block_unroll_threads_const_phi_minted_before_iv() {
        // THE IV-FINDER ORDER BUG: a const-backed phi minted before the IV
        // used to `?`-bail the whole IV search (the loop silently stayed
        // rolled while the identical loop with the phis swapped unrolled).
        // The finder must SKIP unqualified phis and keep searching.
        let mut func = make_two_block_const_phi_before_iv(64);
        let n = unroll_loops(&mut func, UnrollPhase::PostVec);
        assert_eq!(n, 1, "const-back phi before the IV must not hide the IV");
        let body = merged_body_of(&func);
        // The const_temp materialization: one Copy of Const(3) at the body
        // top feeding clones >= 1.
        let const_temps: Vec<u32> = body
            .instructions
            .iter()
            .filter_map(|i| match i {
                Instruction::Copy {
                    dest,
                    src: Operand::Const(IrConst::I32(3)),
                } => Some(dest.0),
                _ => None,
            })
            .collect();
        assert_eq!(
            const_temps.len(),
            1,
            "exactly one const_temp Copy of the reset constant"
        );
        // Clone 0 reads the C phi itself; every later clone reads the
        // const_temp — never the phi again.
        let phi_reads = body
            .instructions
            .iter()
            .filter(|i| {
                let mut probe = (*i).clone();
                let mut reads = false;
                probe.for_each_operand_mut(|op| {
                    if matches!(op, Operand::Value(v) if v.0 == 2) {
                        reads = true;
                    }
                });
                reads
            })
            .count();
        assert_eq!(
            phi_reads, 1,
            "only clone 0 may read the const-backed phi (got {phi_reads})"
        );
        let temp_reads = body
            .instructions
            .iter()
            .filter(|i| {
                let mut probe = (*i).clone();
                let mut reads = 0;
                probe.for_each_operand_mut(|op| {
                    if matches!(op, Operand::Value(v) if v.0 == const_temps[0]) {
                        reads += 1;
                    }
                });
                reads > 0
            })
            .count();
        assert!(temp_reads >= 1, "clones >= 1 must read the const_temp copy");
        // The phi's back edge stays the constant, relabelled to the body.
        let c_phi = func
            .blocks
            .iter()
            .find(|b| b.label == BlockId(1))
            .unwrap()
            .instructions
            .iter()
            .find_map(|i| match i {
                Instruction::Phi { dest, incoming, .. } if dest.0 == 2 => Some(incoming.clone()),
                _ => None,
            })
            .expect("C phi survives");
        let (back, lbl) = c_phi
            .iter()
            .find(|(_, l)| *l != BlockId(0))
            .expect("back edge exists");
        assert!(
            matches!(back, Operand::Const(IrConst::I32(3))),
            "the reset constant stays the back edge"
        );
        assert_eq!(*lbl, merged_body_of(&func).label, "relabelled to the body");
    }

    /// Two-block counted loop with STEP-2 IV and trip 4 (i = 0, 2, 4, 6;
    /// limit 8) with load+store feedstock — the SMALL-TRIP routing
    /// fixture: trips <= 16 whose budget the complete unroller accepts
    /// are Pass A territory by construction, so this loop must be
    /// FLATTENED, never partially unrolled.
    fn make_small_trip_feed_loop() -> IrFunction {
        let mut func = IrFunction::new("small_trip_test".to_string(), IrType::Void, vec![], false);
        // B0: preheader
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Copy {
                    dest: Value(0),
                    src: Operand::Const(IrConst::I32(0)),
                },
                Instruction::Copy {
                    dest: Value(11),
                    src: Operand::Const(IrConst::I32(7)),
                },
                Instruction::GlobalAddr {
                    dest: Value(10),
                    name: "arr".to_string(),
                },
                Instruction::GlobalAddr {
                    dest: Value(12),
                    name: "src".to_string(),
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        // B1: header — iv phi (STEP 2), acc phi (back edge = the work
        // Add below), exit cmp against the limit 8 (trip 4: 0,2,4,6)
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(1),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(5)), BlockId(2)),
                    ],
                },
                Instruction::Phi {
                    dest: Value(2),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(11)), BlockId(0)),
                        (Operand::Value(Value(15)), BlockId(2)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(3),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(8)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(3)),
                true_label: BlockId(2),
                false_label: BlockId(4),
            },
            source_spans: Vec::new(),
        });
        // B2: latch — load src[iv], store arr[iv] = acc + loaded, iv
        // Add (STEP 2) LAST
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::GetElementPtr {
                    dest: Value(4),
                    base: Value(10),
                    offset: Operand::Value(Value(1)),
                    ty: IrType::I32,
                },
                Instruction::GetElementPtr {
                    dest: Value(13),
                    base: Value(12),
                    offset: Operand::Value(Value(1)),
                    ty: IrType::I32,
                },
                Instruction::Load {
                    dest: Value(14),
                    ptr: Value(13),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                Instruction::BinOp {
                    dest: Value(15),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Value(Value(14)),
                    ty: IrType::I32,
                },
                Instruction::Store {
                    volatile: false,
                    val: Operand::Value(Value(15)),
                    ptr: Value(4),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(2)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        // B4: exit — acc is live out
        func.blocks.push(BasicBlock {
            label: BlockId(4),
            instructions: vec![
                Instruction::GetElementPtr {
                    dest: Value(6),
                    base: Value(10),
                    offset: Operand::Const(IrConst::I32(0)),
                    ty: IrType::I32,
                },
                Instruction::Store {
                    volatile: false,
                    val: Operand::Value(Value(2)),
                    ptr: Value(6),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        func.next_value_id = 16; // 0–15 used
        func.next_label = 5;
        func
    }

    #[test]
    fn two_block_small_trips_are_complete_unrolled_not_partial() {
        // Step-2 IV, trip 4, feedstock: the complete unroller owns every
        // trip <= 16 whose budget it accepts (work*trip <= 512 holds for
        // any Pass-B-eligible work), so "exactly two groups" (trip == 2k,
        // k <= 4) is complete-unroll territory BY CONSTRUCTION. Pin the
        // routing: the loop must be flattened, not partially unrolled —
        // and the flattened body must carry all four iterations' traffic.
        let mut func = make_small_trip_feed_loop();
        let n = unroll_loops(&mut func, UnrollPhase::PostVec);
        assert!(n >= 1, "the small-trip loop must be unrolled by SOMETHING");
        // No rolled two-block loop remains: no block both ends in a
        // conditional and is the target of an unconditional back-branch.
        let has_rolled_two_block = func.blocks.iter().any(|b| {
            matches!(&b.terminator, Terminator::CondBranch { .. })
                && func
                    .blocks
                    .iter()
                    .any(|l| matches!(&l.terminator, Terminator::Branch(t) if *t == b.label))
        });
        assert!(
            !has_rolled_two_block,
            "trip-4 loops are flattened by the complete unroller, not partially unrolled"
        );
        // The feedstock survives the flatten: 4 flattened loads and
        // stores (offsets 0, 2, 4, 6), the exit block's live-out store,
        // and the ORIGINAL latch's copy — the unreachable block still
        // physically exists until the next DCE sweep (same contract as
        // Pass B's dead latch).
        let stores = func
            .blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .filter(|i| {
                matches!(
                    i,
                    Instruction::Store {
                        volatile: false,
                        ..
                    }
                )
            })
            .count();
        let loads = func
            .blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .filter(|i| {
                matches!(
                    i,
                    Instruction::Load {
                        volatile: false,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(
            (loads, stores),
            (5, 6),
            "all four iterations' traffic in the flattened body"
        );
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
            unroll_loops(&mut func, UnrollPhase::Early);

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

    /// Regression: a loop whose LATCH carries a side effect must never be
    /// partially unrolled.
    ///
    /// `do_unroll` clones only `body_work`; the latch executes once per
    /// unroll factor, shared by all clones. The perfect-nest row-sum
    ///
    /// ```c
    /// for (i = 0; i < n; i++) { int s = 0;
    ///     for (j = 0; j < 8; j++) s += a[i*8+j];
    ///     out[i] = s; }        // ← store lands in the OUTER latch
    /// ```
    ///
    /// after inner-loop full unrolling has exactly that shape, and partial
    /// unrolling of the outer loop made it compute k-1 of every k row sums
    /// while storing only row i's — odd rows kept their previous contents
    /// (tests/regression/outer_loop_shapes.c, kernel s4; 20/20 deterministic).
    #[test]
    fn side_effecting_latch_blocks_partial_unroll() {
        // Positive control: the same loop with the store in the BODY is a
        // normal partial-unroll candidate.
        let mut plain = make_counting_loop(40);
        let blocks_before = plain.blocks.len();
        unroll_loops(&mut plain, UnrollPhase::Early);
        assert!(
            plain.blocks.len() > blocks_before,
            "plain counting loop (store in body) should partially unroll"
        );

        // Move the store from the body into the latch — the miscompiled
        // shape. The loop must be left untouched.
        let mut func = make_counting_loop(40);
        let store = func.blocks[2]
            .instructions
            .pop()
            .expect("body ends with the store");
        assert!(matches!(store, Instruction::Store { .. }));
        func.blocks[3].instructions.insert(0, store);

        let blocks_before = func.blocks.len();
        let header_term_before = format!("{:?}", func.blocks[1].terminator);
        let latch_before = format!("{:?}", func.blocks[3].instructions);
        unroll_loops(&mut func, UnrollPhase::Early);

        assert_eq!(
            func.blocks.len(),
            blocks_before,
            "side-effecting latch must disqualify partial unrolling"
        );
        assert_eq!(
            format!("{:?}", func.blocks[3].instructions),
            latch_before,
            "latch must be untouched"
        );
        assert_eq!(
            format!("{:?}", func.blocks[1].terminator),
            header_term_before,
            "header must be untouched"
        );
        let mut violations = Vec::new();
        crate::passes::verify::verify_function(&func, "unroll_loops", &mut violations);
        assert!(violations.is_empty());
    }

    /// A latch-defined value that ESCAPES the latch would be read
    /// once-per-unroll-factor stale after `do_unroll` rethreads the back
    /// edge (`do_unroll` never clones the latch and only retargets the IV
    /// itself). Such loops must be rejected too.
    ///
    /// The escape is spelled in LCSSA form — the latch def feeds a header
    /// phi on the back edge, and the exit returns the phi — because a
    /// while-shape latch def used directly outside the loop is inherently
    /// non-dominated (the header's exit edge bypasses the latch) and
    /// therefore malformed SSA that the def-dominates-use check rightly
    /// rejects. The phi-edge spelling is well-formed (a phi incoming's def
    /// need only dominate its own predecessor) while escaping exactly the
    /// same way: the direct `latch_value_escapes` assertion below pins the
    /// gate predicate itself, so the rejection assertion cannot pass
    /// vacuously.
    #[test]
    fn escaping_latch_def_blocks_partial_unroll() {
        let mut func = make_counting_loop(40);
        // Latch: %20 = Copy %5 (the incremented IV).
        func.blocks[3].instructions.push(Instruction::Copy {
            dest: Value(20),
            src: Operand::Value(Value(5)),
        });
        // Header: %21 = phi(const 0 [preheader], %20 [latch]); the exit
        // returns %21, so the latch def escapes through the phi.
        func.blocks[1].instructions.insert(
            1,
            Instruction::Phi {
                dest: Value(21),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Const(IrConst::I32(0)), BlockId(0)),
                    (Operand::Value(Value(20)), BlockId(3)),
                ],
            },
        );
        func.blocks[4].terminator = Terminator::Return(Some(Operand::Value(Value(21))));
        func.next_value_id = 22;

        // The gate predicate itself fires for the LCSSA escape.
        assert!(
            latch_value_escapes(&func, 3, 20),
            "latch def reaching the header phi must count as escaping"
        );
        let blocks_before = func.blocks.len();
        unroll_loops(&mut func, UnrollPhase::Early);
        assert_eq!(
            func.blocks.len(),
            blocks_before,
            "escaping latch-defined value must disqualify partial unrolling"
        );
        let mut violations = Vec::new();
        crate::passes::verify::verify_function(&func, "unroll_loops", &mut violations);
        assert!(violations.is_empty());
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
            unroll_loops(&mut func, UnrollPhase::Early);

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
        let n = unroll_loops(&mut func, UnrollPhase::Early);
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
        let n = unroll_loops(&mut func, UnrollPhase::Early);
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
        let n = unroll_loops(&mut func, UnrollPhase::Early);
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
                regparm: None,
                is_pure: false,
                is_const: false,
                ret_eightbyte_classes: vec![],
                ret_is_f128_sse: false,
            },
        });
        let n = unroll_loops(&mut func, UnrollPhase::Early);
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
        let n = unroll_loops(&mut func, UnrollPhase::Early);
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
        let n = unroll_loops(&mut func, UnrollPhase::Early);
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

        let n = unroll_loops(&mut func, UnrollPhase::Early);

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

    // ── Persisting-inner-loop gate (fail-closed) ──────────────────────────────
    //
    // `body_contains_persisting_inner_loop` refuses an outer complete unroll
    // unless every nested inner loop is a clean counted loop whose bound and
    // init the outer IV substitution makes per-clone constant (the
    // triangular-cascade shape).  These tests pin both directions of that
    // predicate directly at the gate, plus end-to-end through `unroll_loops`.

    /// Verdict of the persist gate for a hand-declared outer loop.
    fn persist_gate_verdict(func: &IrFunction, outer_body: &[usize], outer_iv: Value) -> bool {
        let cfg = CfgAnalysis::build(func);
        let outer = loop_analysis::NaturalLoop {
            header: 1,
            body: outer_body.iter().copied().collect(),
        };
        body_contains_persisting_inner_loop(func, &cfg, &outer, outer_iv)
    }

    /// Push B0 (preheader) + B1 (`for (i = 0; i < limit; i++)` header, body
    /// at B2, outer latch `latch`, exit `exit`) of a nest fixture.
    fn persist_gate_push_outer_prefix(
        func: &mut IrFunction,
        limit: i32,
        latch: BlockId,
        exit: BlockId,
    ) {
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Copy {
                dest: Value(0),
                src: Operand::Const(IrConst::I32(0)),
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(1),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(8)), latch),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(10),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(limit)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(10)),
                true_label: BlockId(2),
                false_label: exit,
            },
            source_spans: Vec::new(),
        });
    }

    /// Push the outer latch (`%8 = %1 + 1`, back to B1) + the returning exit.
    fn persist_gate_push_outer_suffix(func: &mut IrFunction, latch: u32, exit: u32) {
        func.blocks.push(BasicBlock {
            label: BlockId(latch),
            instructions: vec![Instruction::BinOp {
                dest: Value(8),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(exit),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        func.next_value_id = 32;
    }

    /// Triangular nest (`for i` / `for j = i+1; j < 4`): the cascade shape —
    /// the inner init depends only on the outer IV, so the inner becomes
    /// per-clone constant-trip and the fixpoint unrolls it next. Allowed.
    fn persist_gate_triangular() -> IrFunction {
        let mut func = IrFunction::new("tri".to_string(), IrType::Void, vec![], false);
        persist_gate_push_outer_prefix(&mut func, 4, BlockId(5), BlockId(6));
        // B1 also computes the triangular init j = i + 1.
        func.blocks[1].instructions.push(Instruction::BinOp {
            dest: Value(2),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I32(1)),
            ty: IrType::I32,
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(3),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(2)), BlockId(1)),
                        (Operand::Value(Value(7)), BlockId(4)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(11),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(3)),
                    rhs: Operand::Const(IrConst::I32(4)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(11)),
                true_label: BlockId(3),
                false_label: BlockId(5),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::Copy {
                dest: Value(9),
                src: Operand::Value(Value(3)),
            }],
            terminator: Terminator::Branch(BlockId(4)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(4),
            instructions: vec![Instruction::BinOp {
                dest: Value(7),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(3)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            }],
            terminator: Terminator::Branch(BlockId(2)),
            source_spans: Vec::new(),
        });
        persist_gate_push_outer_suffix(&mut func, 5, 6);
        func
    }

    #[test]
    fn persist_gate_triangular_inner_allowed() {
        let func = persist_gate_triangular();
        assert!(
            !persist_gate_verdict(&func, &[1, 2, 3, 4, 5], Value(1)),
            "triangular inner (init on outer IV) must not veto the outer unroll"
        );
    }

    #[test]
    fn persist_gate_rectangular_inner_allowed() {
        // Const-init/const-bound inner: per-clone constant-trip. Allowed.
        let mut func = persist_gate_triangular();
        // Rectangularize: inner init becomes const 0 (drop the i+1 use).
        if let Instruction::Phi { incoming, .. } = &mut func.blocks[2].instructions[0] {
            incoming[0].0 = Operand::Const(IrConst::I32(0));
        }
        assert!(
            !persist_gate_verdict(&func, &[1, 2, 3, 4, 5], Value(1)),
            "rectangular const inner must not veto the outer unroll"
        );
    }

    #[test]
    fn persist_gate_clean_countdown_inner_allowed() {
        // Single Sub-IV with the exit on it and const init/limit: a clean
        // countdown, which the complete cloner (negative steps) cascades.
        // Allowed — countdown unrolling is pinned by
        // `unroll_countdown_chain_body.c` and must keep working nested.
        let mut func = persist_gate_triangular();
        func.blocks[2].instructions = vec![
            Instruction::Phi {
                dest: Value(3),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Const(IrConst::I32(4)), BlockId(1)),
                    (Operand::Value(Value(7)), BlockId(4)),
                ],
            },
            Instruction::Cmp {
                dest: Value(11),
                op: IrCmpOp::Sgt,
                lhs: Operand::Value(Value(3)),
                rhs: Operand::Const(IrConst::I32(0)),
                ty: IrType::I32,
            },
        ];
        func.blocks[4].instructions = vec![Instruction::BinOp {
            dest: Value(7),
            op: IrBinOp::Sub,
            lhs: Operand::Value(Value(3)),
            rhs: Operand::Const(IrConst::I32(1)),
            ty: IrType::I32,
        }];
        assert!(
            !persist_gate_verdict(&func, &[1, 2, 3, 4, 5], Value(1)),
            "clean countdown inner must not veto the outer unroll"
        );
    }

    /// csv_field_sum's digit EXTRACTOR shape: the inner loop carries an Add
    /// counter, but the exit reads the magic-divided value — no exit on any
    /// Add-IV. The inner persists in every clone. Must refuse.
    fn persist_gate_div_driven() -> IrFunction {
        let mut func = IrFunction::new("divinner".to_string(), IrType::Void, vec![], false);
        persist_gate_push_outer_prefix(&mut func, 6, BlockId(5), BlockId(6));
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(4),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Const(IrConst::I32(0)), BlockId(1)),
                        (Operand::Value(Value(6)), BlockId(4)),
                    ],
                },
                Instruction::Phi {
                    dest: Value(3),
                    ty: IrType::I32,
                    incoming: vec![
                        // Const init ON PURPOSE: the refusal comes from the
                        // exit/IV mismatch (exit reads the div-driven phi),
                        // not from the bound.
                        (Operand::Const(IrConst::I32(7)), BlockId(1)),
                        (Operand::Value(Value(5)), BlockId(4)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(11),
                    op: IrCmpOp::Sgt,
                    lhs: Operand::Value(Value(3)),
                    rhs: Operand::Const(IrConst::I32(0)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(11)),
                true_label: BlockId(3),
                false_label: BlockId(5),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::Copy {
                dest: Value(9),
                src: Operand::Value(Value(3)),
            }],
            terminator: Terminator::Branch(BlockId(4)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(4),
            instructions: vec![
                Instruction::BinOp {
                    dest: Value(6),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(4)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
                // Non-Add update of the exit phi (stands in for the
                // magic-div chain): no Add-IV explains the exit.
                Instruction::BinOp {
                    dest: Value(5),
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(Value(3)),
                    rhs: Operand::Value(Value(3)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Branch(BlockId(2)),
            source_spans: Vec::new(),
        });
        persist_gate_push_outer_suffix(&mut func, 5, 6);
        func
    }

    #[test]
    fn persist_gate_div_driven_inner_refused() {
        let func = persist_gate_div_driven();
        assert!(
            persist_gate_verdict(&func, &[1, 2, 3, 4, 5], Value(1)),
            "div-driven inner (exit on non-IV phi) must veto the outer unroll"
        );
    }

    /// csv_field_sum's digit EMITTER shape: an Add o-counter listed FIRST
    /// plus a Sub t-countdown, with the exit on the countdown. The IV
    /// finder returns the Add-IV, the exit match fails, and the loop
    /// persists. Must refuse.
    fn persist_gate_two_iv_exit_on_second() -> IrFunction {
        let mut func = persist_gate_div_driven();
        // Turn the div update into a Sub countdown; keep the Add-phi first.
        func.blocks[4].instructions[1] = Instruction::BinOp {
            dest: Value(5),
            op: IrBinOp::Sub,
            lhs: Operand::Value(Value(3)),
            rhs: Operand::Const(IrConst::I32(1)),
            ty: IrType::I32,
        };
        func
    }

    #[test]
    fn persist_gate_exit_on_second_iv_refused() {
        let func = persist_gate_two_iv_exit_on_second();
        assert!(
            persist_gate_verdict(&func, &[1, 2, 3, 4, 5], Value(1)),
            "inner with the exit on its second IV must veto the outer unroll"
        );
    }

    /// Multi-latch inner: B3 is the Branch latch, B4 re-enters the header
    /// conditionally. The unmerged loop with tail B3 has two latches, which
    /// the complete cloner can never cascade. Must refuse. (Tail order only
    /// selects WHICH arm fires first — B3 < B4 puts the two-latch loop
    /// first — but the verdict is order-independent: the other unmerged
    /// loop is latch-malformed and refuses on its own.)
    fn persist_gate_multi_latch() -> IrFunction {
        let mut func = IrFunction::new("multilatch".to_string(), IrType::Void, vec![], false);
        persist_gate_push_outer_prefix(&mut func, 4, BlockId(5), BlockId(6));
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(3),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(7)), BlockId(4)),
                        (Operand::Value(Value(6)), BlockId(3)),
                        (Operand::Value(Value(0)), BlockId(1)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(11),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(3)),
                    rhs: Operand::Const(IrConst::I32(10)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(11)),
                true_label: BlockId(4),
                false_label: BlockId(5),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::BinOp {
                dest: Value(6),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(3)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            }],
            terminator: Terminator::Branch(BlockId(2)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(4),
            instructions: vec![
                Instruction::BinOp {
                    dest: Value(7),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(3)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
                Instruction::Cmp {
                    dest: Value(12),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(3)),
                    rhs: Operand::Const(IrConst::I32(5)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(12)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
            source_spans: Vec::new(),
        });
        persist_gate_push_outer_suffix(&mut func, 5, 6);
        func
    }

    #[test]
    fn persist_gate_multi_latch_inner_refused() {
        let func = persist_gate_multi_latch();
        assert!(
            persist_gate_verdict(&func, &[1, 2, 3, 4, 5], Value(1)),
            "multi-latch inner must veto the outer unroll"
        );
    }

    #[test]
    fn persist_gate_detached_cycle_no_veto() {
        // A disconnected B5↔B6 cycle next to a clean unrollable outer loop:
        // dead code must never veto optimization. The membership test
        // excludes it even if loop detection ever learns to see detached
        // cycles (a natural header dominates its body, so an unreachable
        // block can never sit in `outer.body`); the second assertion pins
        // today's invisibility as the tripwire for that analysis change.
        let mut func = IrFunction::new("detached".to_string(), IrType::Void, vec![], false);
        persist_gate_push_outer_prefix(&mut func, 4, BlockId(3), BlockId(4));
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![Instruction::Copy {
                dest: Value(9),
                src: Operand::Value(Value(1)),
            }],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
        });
        persist_gate_push_outer_suffix(&mut func, 3, 4);
        func.blocks.push(BasicBlock {
            label: BlockId(5),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(6)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(6),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(5)),
            source_spans: Vec::new(),
        });
        assert!(
            !persist_gate_verdict(&func, &[1, 2, 3], Value(1)),
            "detached cycle must not veto the outer unroll"
        );
        let cfg = CfgAnalysis::build(&func);
        let all =
            loop_analysis::find_natural_loops(cfg.num_blocks, &cfg.preds, &cfg.succs, &cfg.idom);
        assert!(
            all.iter().all(|lp| lp.header != 5 && lp.header != 6),
            "tripwire: loop detection learned to see detached cycles — \
             re-examine the persist gate's membership argument"
        );
    }

    #[test]
    fn persist_gate_legacy_restores_lenient() {
        // Differential pin: under the legacy knob the four refusing shapes
        // above are allowed again (the pre-S20 skip-on-unrecognized gate),
        // while the three cascade shapes stay allowed in both modes.
        let prev = persist_gate_legacy();
        set_persist_gate_legacy(true);
        assert!(!persist_gate_verdict(
            &persist_gate_triangular(),
            &[1, 2, 3, 4, 5],
            Value(1)
        ));
        let mut rect = persist_gate_triangular();
        if let Instruction::Phi { incoming, .. } = &mut rect.blocks[2].instructions[0] {
            incoming[0].0 = Operand::Const(IrConst::I32(0));
        }
        assert!(!persist_gate_verdict(&rect, &[1, 2, 3, 4, 5], Value(1)));
        assert!(!persist_gate_verdict(
            &persist_gate_affine_limit_in_header(),
            &[1, 2, 3, 4, 5],
            Value(1)
        ));
        assert!(!persist_gate_verdict(
            &persist_gate_div_driven(),
            &[1, 2, 3, 4, 5],
            Value(1)
        ));
        assert!(!persist_gate_verdict(
            &persist_gate_two_iv_exit_on_second(),
            &[1, 2, 3, 4, 5],
            Value(1)
        ));
        assert!(!persist_gate_verdict(
            &persist_gate_multi_latch(),
            &[1, 2, 3, 4, 5],
            Value(1)
        ));
        assert!(!persist_gate_verdict(
            &persist_gate_multi_entry_inner(),
            &[1, 2, 3, 4, 5],
            Value(1)
        ));
        set_persist_gate_legacy(prev);
        // Restore verified: the refusing shapes veto again.
        assert!(persist_gate_verdict(
            &persist_gate_div_driven(),
            &[1, 2, 3, 4, 5],
            Value(1)
        ));
        assert!(persist_gate_verdict(
            &persist_gate_two_iv_exit_on_second(),
            &[1, 2, 3, 4, 5],
            Value(1)
        ));
        assert!(persist_gate_verdict(
            &persist_gate_multi_latch(),
            &[1, 2, 3, 4, 5],
            Value(1)
        ));
        assert!(persist_gate_verdict(
            &persist_gate_multi_entry_inner(),
            &[1, 2, 3, 4, 5],
            Value(1)
        ));
    }

    /// g3's affine-limit shape (`for j = 2; j < i + 8; j++` with the limit
    /// recomputed in the INNER header from the outer IV): invariant in value
    /// but not hoisted, so no SYNTACTIC exit exists — yet substitution plus
    /// the fold/copyprop between fixpoint rounds makes it per-clone
    /// constant, and the fixpoint cascades. Must allow. (S20's syntactic
    /// gate vetoed this: +20 insns on g3 plus a lost ipcp fold in main.
    /// Legacy allows it too, but for the wrong reason — skipping the
    /// unrecognized exit — while S20b allows it by value prediction.)
    fn persist_gate_affine_limit_in_header() -> IrFunction {
        let mut func = IrFunction::new("afflim".to_string(), IrType::Void, vec![], false);
        persist_gate_push_outer_prefix(&mut func, 4, BlockId(5), BlockId(6));
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(3),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Const(IrConst::I32(2)), BlockId(1)),
                        (Operand::Value(Value(7)), BlockId(4)),
                    ],
                },
                // The limit, computed HERE (in-body, hence not
                // syntactically invariant) from the OUTER iv %1.
                Instruction::BinOp {
                    dest: Value(12),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(8)),
                    ty: IrType::I32,
                },
                Instruction::Cmp {
                    dest: Value(11),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(3)),
                    rhs: Operand::Value(Value(12)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(11)),
                true_label: BlockId(3),
                false_label: BlockId(5),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::Copy {
                dest: Value(9),
                src: Operand::Value(Value(3)),
            }],
            terminator: Terminator::Branch(BlockId(4)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(4),
            instructions: vec![Instruction::BinOp {
                dest: Value(7),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(3)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            }],
            terminator: Terminator::Branch(BlockId(2)),
            source_spans: Vec::new(),
        });
        persist_gate_push_outer_suffix(&mut func, 5, 6);
        func
    }

    #[test]
    fn persist_gate_affine_limit_in_header_allowed() {
        let func = persist_gate_affine_limit_in_header();
        assert!(
            !persist_gate_verdict(&func, &[1, 2, 3, 4, 5], Value(1)),
            "value-invariant limit recomputed in the inner header must not veto"
        );
    }

    /// Multi-entry inner header (irreducible CFG): the IV phi has TWO
    /// non-latch incomings, one const (B1) and one opaque-dynamic (B7, a
    /// syntactic-only predecessor — Value(99) is deliberately undefined).
    /// Per-clone constancy needs ALL entries constable. Must refuse. (The
    /// const incoming is listed LAST so a last-incoming-wins test would
    /// bless this nest — the S20/legacy false-negative this pins shut.)
    fn persist_gate_multi_entry_inner() -> IrFunction {
        let mut func = IrFunction::new("multientry".to_string(), IrType::Void, vec![], false);
        persist_gate_push_outer_prefix(&mut func, 4, BlockId(5), BlockId(6));
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(3),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(99)), BlockId(7)),
                        (Operand::Const(IrConst::I32(0)), BlockId(1)),
                        (Operand::Value(Value(7)), BlockId(4)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(11),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(3)),
                    rhs: Operand::Const(IrConst::I32(4)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(11)),
                true_label: BlockId(3),
                false_label: BlockId(5),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::Copy {
                dest: Value(9),
                src: Operand::Value(Value(3)),
            }],
            terminator: Terminator::Branch(BlockId(4)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(4),
            instructions: vec![Instruction::BinOp {
                dest: Value(7),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(3)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            }],
            terminator: Terminator::Branch(BlockId(2)),
            source_spans: Vec::new(),
        });
        persist_gate_push_outer_suffix(&mut func, 5, 6);
        // Syntactic-only second entry into the inner header.
        func.blocks.push(BasicBlock {
            label: BlockId(7),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(2)),
            source_spans: Vec::new(),
        });
        func
    }

    #[test]
    fn persist_gate_multi_entry_inner_refused() {
        let func = persist_gate_multi_entry_inner();
        assert!(
            persist_gate_verdict(&func, &[1, 2, 3, 4, 5], Value(1)),
            "multi-entry inner with a dynamic entry value must veto the outer unroll"
        );
    }

    /// fse's shape: a value-only exit (affine limit recomputed in the inner
    /// header) PLUS a dynamic init (Value(99) is deliberately undefined, an
    /// opaque runtime value). The new gate finds the value exit, then
    /// refuses on the unconstable init; the legacy gate must SKIP the nest
    /// (no SYNTACTIC exit) exactly as pre-S20. Pins the legacy-fidelity fix:
    /// with a value exit in legacy mode this nest would refuse (fse's main
    /// 552 -> 405 with the knob set) instead of restoring the lenient gate.
    fn persist_gate_dynamic_init_affine_limit() -> IrFunction {
        let mut func = IrFunction::new("dyninit".to_string(), IrType::Void, vec![], false);
        persist_gate_push_outer_prefix(&mut func, 4, BlockId(5), BlockId(6));
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(3),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(Value(99)), BlockId(1)),
                        (Operand::Value(Value(7)), BlockId(4)),
                    ],
                },
                Instruction::BinOp {
                    dest: Value(12),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(8)),
                    ty: IrType::I32,
                },
                Instruction::Cmp {
                    dest: Value(11),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(3)),
                    rhs: Operand::Value(Value(12)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(11)),
                true_label: BlockId(3),
                false_label: BlockId(5),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![Instruction::Copy {
                dest: Value(9),
                src: Operand::Value(Value(3)),
            }],
            terminator: Terminator::Branch(BlockId(4)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(4),
            instructions: vec![Instruction::BinOp {
                dest: Value(7),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(3)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            }],
            terminator: Terminator::Branch(BlockId(2)),
            source_spans: Vec::new(),
        });
        persist_gate_push_outer_suffix(&mut func, 5, 6);
        func
    }

    #[test]
    fn persist_gate_dynamic_init_affine_limit_vetoes_unless_legacy() {
        let func = persist_gate_dynamic_init_affine_limit();
        assert!(
            persist_gate_verdict(&func, &[1, 2, 3, 4, 5], Value(1)),
            "value exit with a dynamic init must veto the outer unroll"
        );
        let prev = persist_gate_legacy();
        set_persist_gate_legacy(true);
        assert!(
            !persist_gate_verdict(&func, &[1, 2, 3, 4, 5], Value(1)),
            "legacy gate must skip the unrecognized nest, not refuse it"
        );
        set_persist_gate_legacy(prev);
        assert!(
            persist_gate_verdict(&func, &[1, 2, 3, 4, 5], Value(1)),
            "restored gate must veto again"
        );
    }

    #[test]
    fn e2e_persist_gate_csv_nest_rolled_unless_legacy() {
        // End-to-end through `unroll_loops`: the csv-shaped nest (const-6
        // outer, div-driven inner) keeps its outer back-edge under the
        // fail-closed gate, and unrolls it under the legacy knob — the
        // committed differential proving this test discriminates the fix.
        let mut func = persist_gate_div_driven();
        let n = unroll_loops(&mut func, UnrollPhase::Early);
        let latch = func.blocks.iter().find(|b| b.label == BlockId(5)).unwrap();
        assert!(
            matches!(&latch.terminator, Terminator::Branch(lbl) if *lbl == BlockId(1)),
            "csv-shaped outer must stay rolled (n={n})"
        );
        assert!(
            matches!(&func.blocks[1].terminator, Terminator::CondBranch { .. }),
            "csv-shaped outer header must keep its loop exit"
        );

        let prev = persist_gate_legacy();
        set_persist_gate_legacy(true);
        let mut legacy_func = persist_gate_div_driven();
        let legacy_n = unroll_loops(&mut legacy_func, UnrollPhase::Early);
        let legacy_latch = legacy_func
            .blocks
            .iter()
            .find(|b| b.label == BlockId(5))
            .unwrap();
        assert!(
            !matches!(&legacy_latch.terminator, Terminator::Branch(lbl) if *lbl == BlockId(1)),
            "legacy gate must still unroll the csv-shaped outer (n={legacy_n})"
        );
        assert!(legacy_n >= 1, "legacy gate must report the unroll");
        set_persist_gate_legacy(prev);
    }

    #[test]
    fn e2e_persist_gate_triangular_cascade_allowed() {
        // The triangular nest still complete-unrolls outer→inner through
        // `unroll_loops`: the gate's allow-path end-to-end.
        let mut func = persist_gate_triangular();
        let n = unroll_loops(&mut func, UnrollPhase::Early);
        assert!(n >= 2, "triangular nest must cascade outer+inner (n={n})");
        let latch = func.blocks.iter().find(|b| b.label == BlockId(5)).unwrap();
        assert!(
            !matches!(&latch.terminator, Terminator::Branch(lbl) if *lbl == BlockId(1)),
            "triangular outer latch must feed the clone chain"
        );
        assert!(
            func.blocks
                .iter()
                .any(|b| matches!(b.terminator, Terminator::Return(None))),
            "triangular exit must still exist and return"
        );
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
        unroll_loops(&mut func, UnrollPhase::Early);

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
            unroll_loops(&mut func, UnrollPhase::Early);

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
        // `-iv_step` used to overflow for iv_step = i64::MIN (checked_abs
        // refused).  The typed closed form computes |step| in i128, so the
        // true answer emerges: `for (i = 0; i > MIN; i += MIN)` runs exactly
        // ONCE (0 > MIN, then i = MIN, MIN > MIN is false) and the final IV
        // stays in range — Some(1).  trip == 1 is below the unroll gate, so
        // the loop stays rolled; the count only feeds the caller's gate.
        assert_eq!(trip_i64(0, i64::MIN, Sgt, i64::MIN), Some(1));
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
        assert_eq!(trip_i64(i64::MIN, i64::MIN + 8, Slt, 2), Some(4));
        assert_eq!(trip_i64(i64::MAX, i64::MAX - 9, Sgt, -3), Some(3));
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

        let n = unroll_loops(&mut func, UnrollPhase::Early);

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
