//! Loop rotation pass.
//!
//! Rotates "guard-at-top" loops into "test-at-bottom" form. The canonical
//! unrotated loop emits TWO branch instructions per iteration in the hot
//! path (the guard's conditional + the latch's unconditional backedge):
//!
//! ```text
//!   preheader → header
//!   header:  cmp; CondBranch(cond, body, exit)   // guard (continue = fall)
//!   body:    ...                                 // (cond not taken = continue)
//!   latch:   Branch(header)                      // unconditional backedge
//!   exit:    ...
//! ```
//!
//! After rotation the hot path has ONE conditional branch (the test, taken
//! when continuing) and the exit falls through:
//!
//! ```text
//!   preheader → header
//!   header:  cmp; CondBranch(cond, body, exit)   // guard: enter or skip
//!   body:    ...
//!   latch:   cmp'; CondBranch(cond', body, exit)  // test: continue or exit
//!   exit:    ...
//! ```
//!
//! The `cmp'` in the latch is a clone of the header's `cmp` with phi
//! references rewritten to the latch-edge incoming values (so the test sees
//! the post-increment IV, not the pre-increment phi). The header retains its
//! guard so the 0-trip case still skips the body.
//!
//! Safety: the transform is conservative — it bails on any loop whose header
//! guard is not a simple CondBranch, whose latch is not a pure backedge to
//! the header, or whose cond-setup closure touches memory or calls. Only
//! SSA-pure arithmetic/cmp instructions are cloned.
//!
//! Kill-switch: set `CCC_NO_LOOP_ROTATE=1` to disable the pass at runtime
//! (wins over opt-in). Opt-in: set `CCC_LOOP_ROTATE=1` to enable the pass
//! at -O2+ (empty / `0` / unset is a no-op).
//!
//! v17: REVERTED to opt-in. The v16 default-enable introduced 16
//! miscompiles (15 remaining after the v17 cross-phi self-loop-phi
//! latch-incoming rewrite fixed `fib`). The 9-worst-benchmark suite is
//! unaffected (rotation bails on multi-exit loops via Guard A/B), so
//! reverting loses no perf on the 9 worst while eliminating all 15
//! remaining v16 miscompiles. The v16/v17 hardening (Guard A exit-block
//! single-predecessor, Guard B dominance-checked external phi uses,
//! v17 cross-phi self-loop-phi latch-incoming rewrite) is KEPT — it makes
//! the pass safer when opt-in. (The v17 undo-on-bail is gone with what it
//! undid: Guards A and B now stand in front of the first change.)
//!
//! PF-17 (2026-09-01, PRs #325/#327): those 15 shapes MATCH GCC on the
//! 19-name A/B. Pred-label uses `(pre_op, header_label)` (never the
//! original preheader). Guards C/D/E, bepre `latest_dep`, univsr skip of
//! rotated self-loop pointer IVs, and complete-unroll Copy-INIT are in
//! tree. Guard F rejects header work outside the cloned condition closure.
//! Guard H rejects a loop in which anything but the header reads a value
//! the header computes: those readers kept the guard's value.
//!
//! Every refusal stands in front of the first change to the function, and
//! the types say so: `plan_rotation` reads the function through a shared
//! reference and is the only place that refuses, `apply_rotation` changes
//! it and returns nothing. A loop that is not rotated is left as it was,
//! value numbers included.
//! Rotation STAYS opt-in (`CCC_LOOP_ROTATE=1`) while default-enable needs
//! broader fuzz/performance evidence; kill-switch `CCC_NO_LOOP_ROTATE=1`
//! wins.
//!
//! v16: the pass was DEFAULT-ON at -O2+. The v14 hardening (exit-merge-phi
//! off-by-one fix, post-vectorize placement, conservative body guards)
//! plus the v16 stricter guards (exit-block single-predecessor check,
//! dominance-checked external phi uses) make the transform safe for the
//! canonical single-block-body counted-loop shape. The ~18 v15 miscompile
//! shapes (multi-exit, header-phi-escapes-through-non-Return-terminator,
//! missing-downstream-use) all bail under the stricter guards.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::analysis::CfgAnalysis;
use crate::ir::reexports::{
    BlockId, Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Terminator, Value,
    replace_operand_value, replace_terminator_value, replace_values_in_inst_map,
};
use crate::passes::loop_analysis::{NaturalLoop, find_natural_loops, merge_loops_by_header};
use crate::passes::loop_unroll::rename_inst_dest;

/// Per-function entry point for the dirty-tracking pipeline.
pub(crate) fn run_function(func: &mut IrFunction) -> usize {
    rotate_loops(func)
}

/// Maximum number of loops to rotate per function per fixpoint run. Each
/// successful rotation rebuilds the CFG, so this bounds quadratic worst cases
/// in pathological loop nests.
const MAX_ROTATIONS_PER_FUNC: usize = 256;

/// Conservative bound on the cond-setup closure size. Real guard conditions are
/// 1–3 instructions (`Cmp` + maybe `BinOp And`); anything deeper is likely
/// an already-inlined expression that is better left to GVN/LICM than to
/// duplicate.
const MAX_CLOSURE: usize = 8;

/// True when `name` is set to a truthy value (`1` / `true` / `yes` / `on`).
/// Empty, `0`, `false`, `no`, `off`, or unset => false. Used for
/// `CCC_LOOP_ROTATE` so `CCC_LOOP_ROTATE=` (empty) no longer silently
/// enables the pass.
fn env_flag_truthy(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => {
            let t = v.trim();
            t == "1"
                || t.eq_ignore_ascii_case("true")
                || t.eq_ignore_ascii_case("yes")
                || t.eq_ignore_ascii_case("on")
        }
        Err(_) => false,
    }
}

pub(crate) fn rotate_loops(func: &mut IrFunction) -> usize {
    // v17: REVERTED to OPT-IN (CCC_LOOP_ROTATE=1). The v16 default-enable
    // introduced 16 miscompiles (15 remaining after the v17 cross-phi
    // self-loop-phi latch-incoming rewrite fixed fib): vectorize_sse2_path,
    // vectorize_reduction_dyn, simd_crc_adler, simd_vecreg, backedge_pre_*,
    // bitops_builtins, adler_inline_tail, aggregate_dse_soundness,
    // alloca_bare_builtin, alu_peepholes, arm_vec_load_offset,
    // huft_build_crash, loop_promote_affine_alias, stmt_expr_asm_typeof,
    // vectorize_iv_dependent_base. The 9-worst-benchmark suite is UNAFFECTED
    // by default-enable (rotation bails on multi-exit loops via Guard A/B),
    // so reverting loses no perf on the 9 worst while eliminating all 15
    // remaining v16 miscompiles. The v16 hardening (Guard A exit-block
    // single-predecessor, Guard B dominance-checked external phi uses,
    // v17 cross-phi self-loop-phi latch-incoming rewrite) is KEPT — it
    // makes the pass safer when opt-in (the v17 undo-on-bail went when
    // Guards A and B moved in front of the first change). A future session
    // will root-cause the 15 remaining
    // miscompiles (likely the exit-merge-phi off-by-one for cross-phi
    // latch_ops used externally, plus the cloned-closure header-phi
    // reference collapse) before flipping the default again.
    //
    //
    // 2026-09-07 UPDATE — the 15 miscompiles above are no longer reproducible.
    // On main 2d1db59 (which includes PR #437 "Harden optimizer memory
    // barriers and x86 allocation safety") `CCC_LOOP_ROTATE=1` runs
    // scripts/run_regression_suite.sh at PASS=651 FAIL=0 with 0 AB-diff
    // failures, and so does `CCC_LOOP_ROTATE=1
    // CCC_LOOP_ROTATE_IGNORE_PRESSURE=1`, i.e. with Guard G disabled and every
    // high-pressure loop rotated. The list above is kept as the historical
    // record of what v16 hit; it is not a current blocker. What still blocks a
    // default flip is (a) the profitability question Guard G answers and (b)
    // the absence of a fuzz sweep on this revision -- the regression suite is
    // a fixed corpus, not a generator.
    //
    // Opt-in: `CCC_LOOP_ROTATE=1` (also true/yes/on). Empty, `0`, `false`,
    // `no`, `off`, or unset => the pass is a no-op. A previous `is_err()`
    // check treated `CCC_LOOP_ROTATE=` (empty) as enabled — a silent
    // A/B footgun. Kill-switch: `CCC_NO_LOOP_ROTATE` set (any value,
    // matching `CCC_NO_IVSR`) wins even when opt-in is on.
    if std::env::var("CCC_NO_LOOP_ROTATE").is_ok() {
        return 0;
    }
    if !env_flag_truthy("CCC_LOOP_ROTATE") {
        return 0;
    }
    if func.blocks.len() < 3 {
        return 0;
    }
    let mut total = 0;
    loop {
        let cfg = CfgAnalysis::build(func);
        let raw = find_natural_loops(cfg.num_blocks, &cfg.preds, &cfg.succs, &cfg.idom);
        if raw.is_empty() {
            break;
        }
        let loops = merge_loops_by_header(raw);
        // Process innermost loops first (smallest body) — their rotation is
        // least likely to disturb outer-loop assumptions, and nested
        // rotation can cascade (an outer loop becomes rotatable once an
        // inner latch becomes a conditional test).
        let mut sorted: Vec<&NaturalLoop> = loops.iter().collect();
        sorted.sort_by_key(|lp| lp.body.len());
        let all_headers: crate::common::fx_hash::FxHashSet<usize> =
            loops.iter().map(|lp| lp.header).collect();
        let cx = RotationContext::new(func, &cfg, &all_headers);
        if cx.debug {
            lccc::cli::stderr_line(format_args!("[ROT] found {} loops", loops.len()));
        }
        let mut did = false;
        for lp in sorted.into_iter() {
            // Guard E: do not rotate a loop nested inside another.
            // After inlining, `for (; i < sz; i++)` sits inside
            // `for (sz = 1; ...)`. Rotating the inner remainder is
            // locally SSA-legal, but GVN+LICM on the combined CFG freeze
            // the outer IV to its init (simd_crc_adler adler sz=2
            // returned the sz=1 result 00010001). Outermost loops and
            // sequential (non-nested) loops still rotate. A tighter
            // "cond uses outer-header phi" check is not enough: after
            // copy-prop `sz` is no longer the phi dest.
            let nested = loops
                .iter()
                .any(|outer| outer.header != lp.header && outer.body.contains(&lp.header));
            if nested {
                if cx.debug {
                    lccc::cli::stderr_line(format_args!(
                        "[ROT] nested loop header={} — bail (Guard E)",
                        lp.header
                    ));
                }
                continue;
            }
            if try_rotate_loop(func, lp, &cx) {
                total += 1;
                did = true;
                break; // CFG changed — rebuild before the next candidate.
            }
        }
        if !did || total >= MAX_ROTATIONS_PER_FUNC {
            break;
        }
    }
    total
}

/// GPRs the target can spare for loop-carried state, for Guard G.
///
/// Keyed on the target's ELF e_machine because that is what the backend's
/// register pools are built from. `ABI_RESERVED` covers the stack pointer,
/// the fixed scratch register the fixed-GPR model needs, and argument/return
/// staging; those are never available to hold a loop-carried value across the
/// body. Unknown targets take the most conservative pool so a new backend
/// fails closed rather than rotating into a spill storm.
fn rotation_pressure_budget() -> usize {
    const ABI_RESERVED: usize = 4;
    const EM_386: u16 = 3;
    const EM_X86_64: u16 = 62;
    const EM_AARCH64: u16 = 183;
    const EM_RISCV: u16 = 243;
    match crate::common::types::target_elf_machine() {
        EM_X86_64 => 16usize.saturating_sub(ABI_RESERVED),
        EM_386 => 8usize.saturating_sub(ABI_RESERVED),
        EM_AARCH64 => 31usize.saturating_sub(ABI_RESERVED),
        // RISC-V has 32 registers but x0 is hardwired to zero.
        EM_RISCV => 31usize.saturating_sub(ABI_RESERVED),
        _ => 8usize.saturating_sub(ABI_RESERVED),
    }
}

/// How many of the loop's header phis have a use outside the loop body.
///
/// Those are precisely the values that need an exit phi after rotation (see
/// Guard G). Terminator operands are enumerated as well as instruction
/// operands: a live-out IV is very often consumed by an outside block's
/// `CondBranch`, and ignoring terminators would under-count and let the
/// pressure guard miss exactly the IV-heavy shapes it exists for.
fn live_out_loop_carried_count(func: &IrFunction, lp: &NaturalLoop, phis: &[HeaderPhi]) -> usize {
    if phis.is_empty() {
        return 0;
    }
    let dests: FxHashSet<u32> = phis.iter().map(|phi| phi.dest).collect();
    let mut live_out: FxHashSet<u32> = FxHashSet::default();
    for (bi, blk) in func.blocks.iter().enumerate() {
        if lp.body.contains(&bi) {
            continue;
        }
        for inst in &blk.instructions {
            inst.for_each_used_value(|v| {
                if dests.contains(&v) {
                    live_out.insert(v);
                }
            });
        }
        blk.terminator.for_each_used_value(|v| {
            if dests.contains(&v) {
                live_out.insert(v);
            }
        });
    }
    live_out.len()
}

/// What every candidate of one pass over a function's loops shares.
struct RotationContext<'a> {
    cfg: &'a CfgAnalysis,
    /// The headers of all loops of the function (Guard E).
    all_headers: &'a FxHashSet<usize>,
    label_to_idx: FxHashMap<BlockId, usize>,
    /// `CCC_DEBUG_LOOP_ROTATE`: a `[ROT]` line for every decision.
    debug: bool,
}

impl<'a> RotationContext<'a> {
    fn new(func: &IrFunction, cfg: &'a CfgAnalysis, all_headers: &'a FxHashSet<usize>) -> Self {
        Self {
            cfg,
            all_headers,
            label_to_idx: func
                .blocks
                .iter()
                .enumerate()
                .map(|(i, b)| (b.label, i))
                .collect(),
            debug: std::env::var("CCC_DEBUG_LOOP_ROTATE").is_ok(),
        }
    }
}

/// One phi of the loop's header: what it holds when the loop is entered and
/// what the latch passes over the back edge.
#[derive(Clone, Copy)]
struct HeaderPhi {
    dest: u32,
    ty: IrType,
    /// The value from in front of the loop.
    init: Operand,
    /// The value over the back edge.
    next: Operand,
}

/// The loop as rotation takes it: `header -> latch -> header`, left through
/// the header's conditional branch.
struct LoopShape {
    latch_idx: usize,
    header_label: BlockId,
    latch_label: BlockId,
    continue_label: BlockId,
    exit_label: BlockId,
    /// The header continues into the loop when its condition is true.
    continue_on_true: bool,
    /// The header's branch condition.
    cond: Value,
}

/// Where the loop's values leave it.
struct ExitSide {
    exit_idx: usize,
    /// The phis the exit block has of its own, each with one incoming, from
    /// the header.
    own_phis: usize,
    /// The header phis read behind the loop: each gets a merge at the top of
    /// the exit block.
    read_behind: Vec<HeaderPhi>,
}

/// A rotation that has been decided: everything `apply_rotation` needs.
struct Rotation {
    shape: LoopShape,
    /// The header instructions that compute the condition, in header order.
    test: Vec<Instruction>,
    phis: Vec<HeaderPhi>,
    exit: ExitSide,
    /// The header's own test is proven to stay in its type's range (see
    /// `canonicalise_affine_exit_cmps`).
    affine_guard_proven: bool,
}

/// Try to rotate one natural loop. Returns true if the transform was applied.
///
/// Two halves, and their types keep them apart: `plan_rotation` reads the
/// function through a shared reference and is the only one that can refuse;
/// `apply_rotation` changes the function and returns nothing. A loop that is
/// refused is left exactly as it was, value numbers included, and a loop
/// that is planned is rotated.
fn try_rotate_loop(func: &mut IrFunction, lp: &NaturalLoop, cx: &RotationContext<'_>) -> bool {
    let Some(plan) = plan_rotation(func, lp, cx) else {
        return false;
    };
    apply_rotation(func, lp, cx.cfg, plan, cx.debug);
    true
}

/// Every question that can refuse the rotation, in the order the steps are
/// numbered, and what the rotation needs once none did.
fn plan_rotation(
    func: &IrFunction,
    lp: &NaturalLoop,
    cx: &RotationContext<'_>,
) -> Option<Rotation> {
    let shape = loop_shape(func, lp, cx)?;
    let header_insts = &func.blocks[lp.header].instructions;
    let (closure, def_idx) = test_closure(header_insts, shape.cond, cx.debug)?;
    if !header_values_stay_in_the_header(func, lp, header_insts, &closure, cx.debug)
        || !test_reads_no_other_loops_phi(
            func,
            lp,
            header_insts,
            &closure,
            &def_idx,
            shape.cond,
            cx,
        )
    {
        return None;
    }
    let phis = header_phis(header_insts, shape.latch_label, &def_idx, cx.debug)?;
    if !body_can_take_the_test(func, lp, &shape, cx.debug)
        || !pressure_allows(func, lp, &phis, cx.debug)
    {
        return None;
    }

    let exit = exit_side(func, lp, &shape, &phis, cx)?;

    // Prove the original controlling expression on the loop as it stands,
    // before `apply_rotation` moves any phi. Cloning is not a no-overflow
    // licence: the IR also represents -fwrapv and unsigned arithmetic
    // feeding signed casts. The cloned test observes the next IVs, a subset
    // of the original sequence through its failing test. Only that
    // particular comparison may consume this proof. (Not a refusal, and
    // asked last: a loop that is refused does not pay for it.)
    let affine_guard_proven = affine_fold_enabled()
        && loop_iv_shape(func, lp).iter().any(|iv| {
            signed_type_bounds(iv.ty).is_some()
                && loop_exit_threshold(func, lp, iv, cx.cfg).is_some()
        });

    Some(Rotation {
        test: closure.iter().map(|&i| header_insts[i].clone()).collect(),
        shape,
        phis,
        exit,
        affine_guard_proven,
    })
}

/// Steps 1 to 4: the loop as rotation takes it.
fn loop_shape(func: &IrFunction, lp: &NaturalLoop, cx: &RotationContext<'_>) -> Option<LoopShape> {
    let debug = cx.debug;
    // 1. Single latch that is NOT the header (a self-loop is already rotated).
    let Some(latch_idx) = lp.single_latch(&cx.cfg.preds) else {
        if debug {
            lccc::cli::stderr_line(format_args!(
                "[ROT] no single latch (header={}, body_len={})",
                lp.header,
                lp.body.len()
            ));
        }
        return None;
    };
    if latch_idx == lp.header {
        if debug {
            lccc::cli::stderr_line(format_args!(
                "[ROT] self-loop (latch==header={}, body_len={})",
                lp.header,
                lp.body.len()
            ));
        }
        return None;
    }
    if debug {
        lccc::cli::stderr_line(format_args!(
            "[ROT] candidate: header={}, latch={}, body_len={}",
            lp.header,
            latch_idx,
            lp.body.len()
        ));
    }

    // 2. Latch terminator must be a pure backedge `Branch(header)`.
    let header_label = func.blocks[lp.header].label;
    let latch_label = func.blocks[latch_idx].label;
    if !matches!(
        &func.blocks[latch_idx].terminator,
        Terminator::Branch(t) if *t == header_label
    ) {
        if debug {
            lccc::cli::stderr_line(format_args!("[ROT] latch not Branch(header)"));
        }
        return None;
    }

    // 3. Header terminator must be CondBranch with one in-loop and one
    //    out-of-loop target. The in-loop target is the "continue"; the
    //    out-of-loop target is the "exit".
    let (cond, continue_label, exit_label, continue_on_true) = match &func.blocks[lp.header]
        .terminator
    {
        Terminator::CondBranch {
            cond,
            true_label,
            false_label,
        } => {
            let t_in = is_in_loop(*true_label, &cx.label_to_idx, lp);
            let f_in = is_in_loop(*false_label, &cx.label_to_idx, lp);
            if t_in == f_in {
                if debug {
                    lccc::cli::stderr_line(format_args!("[ROT] CondBranch both-in or both-out"));
                }
                return None; // both in (infinite) or both out (no body)
            }
            let c = *cond;
            if t_in {
                (c, *true_label, *false_label, true)
            } else {
                (c, *false_label, *true_label, false)
            }
        }
        _ => {
            if debug {
                lccc::cli::stderr_line(format_args!(
                    "[ROT] header not CondBranch: {:?}",
                    func.blocks[lp.header].terminator
                ));
            }
            return None;
        }
    };
    if debug {
        lccc::cli::stderr_line(format_args!(
            "[ROT] CondBranch OK, continue={:?} exit={:?}",
            continue_label, exit_label
        ));
    }

    // 4. The guard cond must be a Value (not a constant — those are folded
    //    by cfg_simplify and would not reach here, but fail closed).
    let cond_val = match cond {
        Operand::Value(v) => v,
        Operand::Const(_) => return None,
    };
    Some(LoopShape {
        latch_idx,
        header_label,
        latch_label,
        continue_label,
        exit_label,
        continue_on_true,
        cond: cond_val,
    })
}

/// Step 5 and Guard F: the header instructions that compute the condition,
/// as indices in header order, and where each header value is defined.
fn test_closure(
    header_insts: &[Instruction],
    cond_val: Value,
    debug: bool,
) -> Option<(Vec<usize>, FxHashMap<u32, usize>)> {
    // 5. Collect the transitive closure of header-local instructions that
    //    feed `cond_val`. Only SSA-pure arithmetic/cmp/cast/copy/select
    //    instructions are cloned; anything with memory, calls, or atomics
    //    bails (we do not duplicate side effects).
    let mut def_idx: FxHashMap<u32, usize> = FxHashMap::default();
    for (i, inst) in header_insts.iter().enumerate() {
        if let Some(d) = inst.dest() {
            def_idx.insert(d.0, i);
        }
    }
    let mut closure: Vec<usize> = Vec::new();
    let mut visited: FxHashSet<u32> = FxHashSet::default();
    let mut worklist: Vec<Value> = vec![cond_val];
    while let Some(v) = worklist.pop() {
        if !visited.insert(v.0) {
            continue;
        }
        let Some(&idx) = def_idx.get(&v.0) else {
            continue; // defined outside header (loop-invariant) — keep as-is
        };
        let inst = &header_insts[idx];
        // Phis are NOT cloned — they are rewritten to latch-edge values
        // in step 7. Skip them here (do not add to closure, do not trace
        // their incoming operands — those are `HeaderPhi::next`).
        if matches!(inst, Instruction::Phi { .. }) {
            continue;
        }
        if !is_cloneable_pure(inst) {
            if debug {
                lccc::cli::stderr_line(format_args!(
                    "[ROT] closure: inst not cloneable-pure: idx={} {:?}",
                    idx, inst
                ));
            }
            return None; // cond setup touches memory/calls — bail
        }
        closure.push(idx);
        if closure.len() > MAX_CLOSURE {
            return None; // too deep — leave to other passes
        }
        // Add operands to the worklist.
        inst.for_each_used_value(|v_id| {
            worklist.push(Value(v_id));
        });
    }
    // Sort by header instruction index so the cloned instructions emit in
    // dependency order (a def before its uses).
    closure.sort_unstable();
    closure.dedup();
    if closure.is_empty() {
        return None; // cond is loop-invariant — wouldn't terminate, bail
    }

    // Guard F: every non-Phi header instruction must belong to the cloned
    // condition closure.  Rotation leaves the original header in place for
    // the first-trip guard, but later iterations execute only the cloned
    // closure at the latch.  A header-side-effect outside that closure would
    // therefore run once instead of once per guard evaluation.  For example:
    //
    //   for (i = 0; (trace[i] = i), i < n; ++i) { ... }
    //
    // The Store does not feed `i < n`, so the old transform cloned only the
    // Cmp; `trace[i]` was updated for i == 0 but not for later iterations.
    // Reordering or separately replaying arbitrary header instructions is
    // not sound here (they may touch memory, call, or observe sequencing), so
    // reject the whole loop.  Phi nodes are intentionally exempt: step 6.6
    // recreates their recurrence in the body/latch rather than cloning them.
    if let Some((idx, inst)) = header_insts
        .iter()
        .enumerate()
        .find(|(idx, inst)| !matches!(inst, Instruction::Phi { .. }) && !closure.contains(idx))
    {
        if debug {
            lccc::cli::stderr_line(format_args!(
                "[ROT] header non-Phi inst outside condition closure: idx={} {:?} — bail (Guard F, un-cloned header effect)",
                idx, inst
            ));
        }
        return None;
    }
    Some((closure, def_idx))
}

/// Guard H.
fn header_values_stay_in_the_header(
    func: &IrFunction,
    lp: &NaturalLoop,
    header_insts: &[Instruction],
    closure: &[usize],
    debug: bool,
) -> bool {
    // Guard H: what the header computes is read by the header alone.
    //
    // The header stays behind as the guard and runs once. From the second
    // test on, the copy at the end of the latch computes these values (step
    // 7), under new names that only the copied test reads. A reader outside
    // the header is given neither a merge of the two nor the copy: it keeps
    // the guard's value for every iteration. Steps 6.6 and 6.7 build that
    // merge for the header's phis (a phi in the body, a phi in the exit);
    // nothing builds it for the header's other values, so
    //
    //   for (i = 2; i * i < n; i++) t += i * i;           added 4 each time
    //   for (i = 0; (t = i + 1) < n; i++) s += a[t];      read a[1] each time
    //   for (i = 0; (t = i * 3) < n; i++) ...; return t;  returned 0
    //
    // The first is plain C: value numbering gives the body's `i * i` the
    // header's name. With value numbering through casts (CCC_CAST_RENAME=1)
    // the widening of a narrow counter is such a value too, and every
    // `unsigned char i` loop that indexes with `i + 1` read one element
    // (tests/regression/loop_rotate_header_value_readers.c).
    //
    // A refusal, not the missing merges: over tests/regression at -O2
    // (1,490 files) the guard refuses 24 of 1,978 rotations -- the seven
    // loops of that file in each of the three tests that compile it, and
    // the loop of gvn_cross_block_mul_dot_product.c with its two copies in
    // `main` -- and every one of them computed a wrong result when rotated.
    // A header value that depends on no header phi is the same in every
    // iteration and could be read where it is; no loop of the corpus has
    // one left in its header, so the guard does not tell them apart.
    //
    // Asked here, in front of the first change to the function, so that a
    // refusal has nothing to undo. Readers inside the header are the other
    // instructions of the closure and the branch; a header phi that takes
    // such a value over the back edge is Guard D's.
    let computed: FxHashSet<u32> = closure
        .iter()
        .filter_map(|&idx| header_insts[idx].dest())
        .map(|dest| dest.0)
        .collect();
    for (block_idx, block) in func.blocks.iter().enumerate() {
        if block_idx == lp.header {
            continue;
        }
        let mut read: Option<u32> = None;
        let mut note = |value: u32| {
            if read.is_none() && computed.contains(&value) {
                read = Some(value);
            }
        };
        for inst in &block.instructions {
            inst.for_each_used_value(&mut note);
        }
        block.terminator.for_each_used_value(&mut note);
        if let Some(value) = read {
            if debug {
                lccc::cli::stderr_line(format_args!(
                    "[ROT] v{} is computed in the header and read in block {} — bail (Guard H, no merge of the guard's value and the latch's)",
                    value, block_idx
                ));
            }
            return false;
        }
    }
    true
}

/// Guard E.
fn test_reads_no_other_loops_phi(
    func: &IrFunction,
    lp: &NaturalLoop,
    header_insts: &[Instruction],
    closure: &[usize],
    def_idx: &FxHashMap<u32, usize>,
    cond_val: Value,
    cx: &RotationContext<'_>,
) -> bool {
    // Guard E: refuse to rotate when the cloned cond consumes a phi that
    // lives in a DIFFERENT loop header. After inlining, a remainder loop
    // `for (; i < sz; i++)` has `sz` as the IV of an outer counted loop.
    // Rotating the inner loop is locally SSA-legal, but GVN+LICM on the
    // combined CFG then freeze that outer IV to its init (simd_crc_adler
    // adler sz=2 returned the sz=1 result 00010001). Constant-trip inner
    // loops (`k < 8`) and loops whose limit is a param/invariant still
    // rotate. Nested loops whose trip count is an outer IV do not.
    let mut used_outside: Vec<u32> = Vec::new();
    for &idx in closure {
        header_insts[idx].for_each_used_value(|v| {
            if !def_idx.contains_key(&v) {
                used_outside.push(v);
            }
        });
    }
    // The cond value itself may be defined outside (rare); include it.
    if !def_idx.contains_key(&cond_val.0) {
        used_outside.push(cond_val.0);
    }
    let mut foreign_iv = false;
    let mut foreign_phi = 0u32;
    'scan: for vid in used_outside {
        for (bi, block) in func.blocks.iter().enumerate() {
            if bi == lp.header || !cx.all_headers.contains(&bi) {
                continue;
            }
            for inst in &block.instructions {
                if let Instruction::Phi { dest, .. } = inst {
                    if dest.0 == vid {
                        foreign_iv = true;
                        foreign_phi = vid;
                        break 'scan;
                    }
                }
            }
        }
    }
    if foreign_iv {
        if cx.debug {
            lccc::cli::stderr_line(format_args!(
                "[ROT] cond uses foreign loop-header phi v{} — bail (Guard E, nested IV as trip limit)",
                foreign_phi
            ));
        }
        return false;
    }
    true
}

/// Guards C and D: the header's phis, each with its value from in front of
/// the loop and its value over the back edge.
fn header_phis(
    header_insts: &[Instruction],
    latch_label: BlockId,
    def_idx: &FxHashMap<u32, usize>,
    debug: bool,
) -> Option<Vec<HeaderPhi>> {
    // v18 Guard C: a header phi may have MULTIPLE non-latch incomings when
    // the loop header has several outside predecessors (e.g. two exits of a
    // preceding loop both flowing into this loop's header, or a break edge
    // and a normal-exit edge merging at the header). Step 6.6 records
    // exactly ONE init incoming, labelled with the GUARD
    // (`(pre_op, header_label)` — never the original preheader). That is
    // enough for a single outside predecessor. Multiple distinct outside
    // preds cannot be represented by one init edge: after rotation the
    // body's only forward predecessor is the guard, and dropping the extra
    // incomings would lose an init value. Observed historically as
    // loop_rotate_default_enable.c shape 4 (`sum_with_call(50)` garbage)
    // when the init edge still named a dead preheader. Routing every
    // extra init through the guard's own phis is a future enhancement;
    // bailing keeps rotation sound (same policy as Guard A/B).
    let mut phis: Vec<HeaderPhi> = Vec::new();
    for inst in header_insts {
        let Instruction::Phi { dest, incoming, ty } = inst else {
            continue;
        };
        let mut init: Option<(BlockId, Operand)> = None;
        let mut next = None;
        for (op, label) in incoming {
            if *label == latch_label {
                next = Some(*op);
            } else if init.is_some_and(|(seen, _)| seen != *label) {
                // A second DISTINCT outside predecessor: the single-init
                // self-loop phi cannot represent this header.
                if debug {
                    lccc::cli::stderr_line(format_args!(
                        "[ROT] header phi v{} has >1 non-latch incoming — bail (Guard C)",
                        dest.0
                    ));
                }
                return None;
            } else {
                init = Some((*label, *op));
            }
        }
        if let (Some((_, init)), Some(next)) = (init, next) {
            phis.push(HeaderPhi {
                dest: dest.0,
                ty: *ty,
                init,
                next,
            });
        }
    }

    // Guard D: refuse to rotate when a header phi's latch incoming is
    // defined by a NON-PHI instruction in the header. That is the
    // `while (--i)` / header-decremented IV shape:
    //
    //   header: i = phi(g, i_next); i_next = i - 1; if i_next { body }
    //   body:   ...; goto header
    //
    // Step 7 rewrites cloned-closure phi uses to the latch incoming, so
    // the cloned `i_next' = i - 1` becomes `i_next' = i_next - 1` with
    // `i_next` the GUARD's already-computed `g-1`. After rotation that
    // value is loop-invariant, the backedge test is `(g-1)-1 != 0`
    // forever, and the body walks off the end of the array (SIGSEGV on
    // huft_build's `while (--i) { *xp++ = (j += *p++); }`, PF-17).
    // Canonical `for (i = 0; i < n; i++)` is unaffected: its latch
    // incoming is the body-defined `i + 1`.
    for phi in &phis {
        let Operand::Value(v) = phi.next else {
            continue;
        };
        let Some(&idx) = def_idx.get(&v.0) else {
            continue; // defined outside the header (body / preheader)
        };
        if matches!(header_insts[idx], Instruction::Phi { .. }) {
            continue; // cross-phi latch incoming: the self-loop phis take it
        }
        if debug {
            lccc::cli::stderr_line(format_args!(
                "[ROT] header phi v{} latch incoming v{} is defined in the header (not a phi) — bail (Guard D, while(--i) shape)",
                phi.dest, v.0
            ));
        }
        return None;
    }
    Some(phis)
}

/// Steps 6.5 and 6.55: one body block, and nothing in it the test may not follow.
fn body_can_take_the_test(
    func: &IrFunction,
    lp: &NaturalLoop,
    shape: &LoopShape,
    debug: bool,
) -> bool {
    // 6.5 Restrict to the single-block body+latch shape (header + one body
    //     block that is ALSO the latch). This is the canonical counted-loop
    //     form after mem2reg + cfg_simplify: `header → body_latch → header`.
    //     In this shape, rotating turns body_latch into a self-loop, so the
    //     IV phi must be MOVED from the header into body_latch (a fresh phi
    //     that receives the preheader value on entry and the computed next
    //     value on each self-loop iteration). Multi-block bodies need the new
    //     phi placed in the body ENTRY (not the latch) and body-wide use
    //     replacement — left to a future enhancement.
    let single_block_body = lp.body.len() == 2 && shape.continue_label == shape.latch_label;
    if !single_block_body {
        if debug {
            lccc::cli::stderr_line(format_args!(
                "[ROT] not single-block body (body_len={}, continue==latch={})",
                lp.body.len(),
                shape.continue_label == shape.latch_label
            ));
        }
        return false;
    }

    // 6.55 Conservative bail: reject bodies that contain a Call/CallIndirect
    //     or any volatile memory op. The transform's exit-merge-phi and
    //     latch-phi rewriting assume the body is straight-line SSA-pure
    //     arithmetic + non-volatile memory. A call in the body can clobber
    //     caller-saved values that the exit-merge-phi references across the
    //     call boundary, and the recursive-call CFG of `fib` is detected as
    //     a spurious loop by `find_natural_loops` (no C-level loop) — bailing
    //     here keeps recursion untouched. Volatile ops must not have their
    //     ordering relative to the rotated test perturbed either.
    let body_block = &func.blocks[shape.latch_idx];
    for inst in &body_block.instructions {
        match inst {
            Instruction::Call { .. } | Instruction::CallIndirect { .. } => {
                if debug {
                    lccc::cli::stderr_line(format_args!(
                        "[ROT] body has Call/CallIndirect — bail (call clobbers exit-merge values)"
                    ));
                }
                return false;
            }
            Instruction::Load { volatile: true, .. }
            | Instruction::Store { volatile: true, .. } => {
                if debug {
                    lccc::cli::stderr_line(format_args!(
                        "[ROT] body has volatile mem op — bail (ordering)"
                    ));
                }
                return false;
            }
            // Intrinsics (Vec*/SSE/AVX) are also rejected: the rotated
            // self-loop form's XMM phi handling doesn't match the backend's
            // vector-register home assignment, and the vectorizer has already
            // had a chance to run (rotation is post-vectorize).
            Instruction::Intrinsic { .. } => {
                if debug {
                    lccc::cli::stderr_line(format_args!(
                        "[ROT] body has Intrinsic — bail (XMM phi / vector-reg home mismatch)"
                    ));
                }
                return false;
            }
            _ => {}
        }
    }
    true
}

/// Guard G.
fn pressure_allows(func: &IrFunction, lp: &NaturalLoop, phis: &[HeaderPhi], debug: bool) -> bool {
    // Guard G: register-pressure profitability.
    //
    // Rotation is not free at the machine level.  After rotation the loop exit
    // is reachable from TWO edges -- the 0-trip guard and the latch -- so every
    // loop-carried value that is live OUT of the loop needs an exit phi
    // merging its preheader init with the body's last definition.  That keeps
    // the body's last definition live from its definition point to the exit,
    // i.e. rotation extends one live range across the whole loop body per
    // live-out value.  A value read only inside the loop pays nothing new (its
    // header phi already spanned the loop); the live-out set is exactly the
    // set whose backedge value previously died at the backedge.
    //
    // When that set exceeds the registers the target can spare, the allocator
    // pays for the extension with spills *inside the hot loop*, which costs
    // far more than the one compare rotation removes from the entry path.
    // Measured (x86-64, -O2, `scripts/perf_ab.py`, 5 interleaved reps):
    //   arith_loop        32 live-out ints -> hot loop 113 -> 149 insns,
    //                     stack traffic 22 -> 47 spill/reloads, 24.9% SLOWER
    //   sha256_transform  state in memory, few live-out -> 27.1% FASTER
    // A global on/off switch cannot express that trade; the live-out count
    // against the register budget can.  Without this guard the two effects
    // cancel to a 0.998 geomean over the corpus -- a wash that hides both a
    // 27% win and a 25% regression.
    //
    // A/B escape hatch for isolating the guard itself:
    // `CCC_LOOP_ROTATE_IGNORE_PRESSURE=1`.
    if !env_flag_truthy("CCC_LOOP_ROTATE_IGNORE_PRESSURE") {
        let live_out = live_out_loop_carried_count(func, lp, phis);
        let budget = rotation_pressure_budget();
        if live_out > budget {
            if debug {
                lccc::cli::stderr_line(format_args!(
                    "[ROT] {} loop-carried values live out of the loop, budget {} — bail (Guard G, register pressure)",
                    live_out, budget
                ));
            }
            return false;
        }
    }
    true
}

/// Guards A and B: where the loop's values leave it.
fn exit_side(
    func: &IrFunction,
    lp: &NaturalLoop,
    shape: &LoopShape,
    phis: &[HeaderPhi],
    cx: &RotationContext<'_>,
) -> Option<ExitSide> {
    let debug = cx.debug;
    // 6.7, decided: where the loop's values leave it.
    //
    // After rotation the exit block has two predecessors, the guard (no
    // iteration ran) and the latch (the copied test failed). Whatever
    // crosses needs the merge of the two: a header phi read behind the loop
    // gets a phi in the exit block, and a phi the exit block already has
    // gets a second incoming. Both are built by `merge_behind_the_loop`.
    //
    // Whether they can be built is decided here, with every other refusal:
    // the planner cannot change the function. (Guards A and B used to stand
    // behind step 6.6 and take its phis out again; the value numbers stayed
    // taken.)
    let &exit_idx = cx.label_to_idx.get(&shape.exit_label)?;
    // Guard A: the header is the exit block's only predecessor, so that the
    // exit block has exactly the guard and the latch in front of it once
    // rotated and two incomings say everything. A third predecessor (a
    // second exit of the loop, a jump around it) would be an edge on which
    // the merge has no value.
    let exit_preds = cx.cfg.preds.row(exit_idx);
    if exit_preds.len() != 1 || exit_preds[0] as usize != lp.header {
        if debug {
            lccc::cli::stderr_line(format_args!(
                "[ROT] exit block has {} predecessors (expected 1 = header); bailing",
                exit_preds.len()
            ));
        }
        return None;
    }
    // The exit block's own phis. With one predecessor each has one
    // incoming, from the header; anything else is a phi this function did
    // not keep up with its edges, and nothing is built on it.
    let exit_own_phis = func.blocks[exit_idx]
        .instructions
        .iter()
        .take_while(|inst| matches!(inst, Instruction::Phi { .. }))
        .count();
    let exit_phis_name_the_header = func.blocks[exit_idx].instructions[..exit_own_phis]
        .iter()
        .all(|inst| {
            matches!(inst, Instruction::Phi { incoming, .. }
                if incoming.len() == 1 && incoming[0].1 == shape.header_label)
        });
    if !exit_phis_name_the_header {
        if debug {
            lccc::cli::stderr_line(format_args!(
                "[ROT] a phi of the exit block does not name the header alone — bail"
            ));
        }
        return None;
    }
    // Guard B: the merge of a header phi is defined at the top of the exit
    // block, so it serves the readers the exit block dominates. A phi reads
    // its incoming at the end of the predecessor it names, so for a phi it
    // is that predecessor the exit block has to dominate, not the phi's own
    // block: `if (c) { loop } else { s = 0; }` merges the loop's sum in a
    // block the exit does not dominate, from an edge it does. (The guard
    // used to ask about the phi's block and refused that shape.)
    //
    // A block no path reaches is not asked: nothing in it runs, nothing
    // dominates it, and it is left reading the header's phi, which stays
    // defined. The unroller leaves the body it replaced behind as such a
    // block until the next CFG cleanup, and rotation runs before that.
    //
    // In valid SSA no reader that can run fails this: it is dominated by
    // the header, and every path from the header out of the loop starts
    // with the edge to the exit block (the body is one block that jumps
    // back). The guard is asked because its answer is what the rewrite in
    // `merge_behind_the_loop` relies on. Over tests/regression at -O2 it refused 227 times
    // while it asked about a phi's own block and about dead blocks (18 and
    // 209 of them) and does not refuse now.
    let mut merged: Vec<HeaderPhi> = Vec::new();
    for phi in phis {
        let phi_dest = phi.dest;
        let mut read_behind = false;
        for (bi, block) in func.blocks.iter().enumerate() {
            if bi == lp.header || lp.body.contains(&bi) || is_unreachable(bi, cx.cfg) {
                continue;
            }
            // The exit block's own phis read the header's value on the
            // guard's edge; they get their own second incoming.
            let own_phis = if bi == exit_idx { exit_own_phis } else { 0 };
            let block_is_served = is_dominated_by(bi, exit_idx, cx.cfg);
            let mut served = true;
            let mut read_here = false;
            for inst in &block.instructions[own_phis..] {
                if let Instruction::Phi { incoming, .. } = inst {
                    for (op, pred_label) in incoming {
                        if !matches!(op, Operand::Value(v) if v.0 == phi_dest) {
                            continue;
                        }
                        read_here = true;
                        served &= cx
                            .label_to_idx
                            .get(pred_label)
                            .is_some_and(|&pred| is_dominated_by(pred, exit_idx, cx.cfg));
                    }
                } else {
                    let mut reads = false;
                    inst.for_each_used_value(|v| reads |= v == phi_dest);
                    read_here |= reads;
                    served &= !reads || block_is_served;
                }
            }
            let mut reads = false;
            block
                .terminator
                .for_each_used_value(|v| reads |= v == phi_dest);
            read_here |= reads;
            served &= !reads || block_is_served;
            if !served {
                if debug {
                    lccc::cli::stderr_line(format_args!(
                        "[ROT] header phi {} has external use in block {} \
                         not dominated by exit {} — bailing",
                        phi_dest, bi, exit_idx
                    ));
                }
                return None;
            }
            read_behind |= read_here;
        }
        if read_behind {
            merged.push(*phi);
        }
    }
    Some(ExitSide {
        exit_idx,
        own_phis: exit_own_phis,
        read_behind: merged,
    })
}

/// Rotate a planned loop. Nothing here refuses.
fn apply_rotation(
    func: &mut IrFunction,
    lp: &NaturalLoop,
    cfg: &CfgAnalysis,
    plan: Rotation,
    debug: bool,
) {
    let mut next_val = func.next_value_id;
    let new_loop_phis = move_phis_into_the_latch(func, &plan, &mut next_val);
    merge_behind_the_loop(func, lp, cfg, &plan, &new_loop_phis, &mut next_val);
    put_the_test_at_the_latch(func, &plan, &new_loop_phis, &mut next_val, debug);

    // 10. The latch no longer branches to the header — its only successor is
    //     now `continue_label` (self or body) or `exit_label`. Remove the
    //     stale `(op, latch_label)` incoming from every phi in the header,
    //     because the latch→header backedge is gone. cfg_simplify then
    //     collapses the now-single-incoming phi to the preheader value.
    for inst in &mut func.blocks[lp.header].instructions {
        if let Instruction::Phi { incoming, .. } = inst {
            incoming.retain(|(_, lbl)| *lbl != plan.shape.latch_label);
        }
    }

    // 11. Advance the watermark so subsequent passes see the new IDs.
    func.next_value_id = next_val;

    if debug {
        lccc::cli::stderr_line(format_args!(
            "[ROT] SUCCESS: rotated header={} latch={}",
            lp.header, plan.shape.latch_idx
        ));
    }
}

/// Step 6.6. Returns the self-loop phi of each header phi.
fn move_phis_into_the_latch(
    func: &mut IrFunction,
    plan: &Rotation,
    next_val: &mut u32,
) -> FxHashMap<u32, u32> {
    let shape = &plan.shape;
    // 6.6 Create a fresh self-loop phi in body_latch for each header phi.
    //     The new phi `i_loop = phi[header: v_pre, latch: v_latch]`
    //     becomes the IV for the rotated self-loop. The header's original
    //     phi is left in place (step 10 strips its latch incoming so
    //     cfg_simplify collapses it to the preheader value — which is what
    //     the guard now checks).
    //
    //     The init incoming MUST be labeled with the HEADER (the guard),
    //     not the original preheader. After rotation the body's only
    //     forward predecessor is the header's continue edge; the original
    //     preheader still branches to the header. Naming the preheader
    //     here records a dead edge: cfg_simplify then drops that incoming
    //     (jump-threading + unreachable-block sweep) and collapses the
    //     phi to a Copy of the latch operand, which is defined LATER in
    //     the same block — use-before-def, garbage IV, SIGSEGV. Observed
    //     on every function with a second sequential counted loop
    //     (alloca_bare_builtin, alu_peepholes, bitops_builtins, huft,
    //     arm_vec_load_offset, …): the first loop's preheader is the
    //     entry and accidentally becomes a predecessor after header-merge,
    //     so the bug hid there; the second loop's preheader is a
    //     now-dead jump block. PF-17.
    //
    //     CRITICAL: uses of the header phi inside body_latch (e.g.
    //     `load a[i]` or `i_next = i + 1`) must be rewritten to the new
    //     `i_loop` BEFORE the cloned cond is appended — otherwise the body
    //     would still reference the header phi, which after step 10 holds
    //     only the preheader value (so the IV would never advance and the
    //     loop would spin forever).
    let mut new_loop_phis: FxHashMap<u32, u32> = FxHashMap::default(); // header_phi_dest → new_loop_phi_dest
    let mut new_phi_insts: Vec<Instruction> = Vec::with_capacity(plan.phis.len());
    for phi in &plan.phis {
        let new_dest = Value(*next_val);
        *next_val += 1;
        new_loop_phis.insert(phi.dest, new_dest.0);
        new_phi_insts.push(Instruction::Phi {
            dest: new_dest,
            ty: phi.ty,
            // The init edge is labelled with the GUARD (`header_label`), not
            // with the original preheader the phi named. Steps 6/6.5 rewire
            // every original header predecessor onto the guard, so after
            // rotation the body's only entry edge is guard → body; the
            // preheader is no longer a predecessor of this block. Naming it
            // here produces malformed IR: phi elimination resolves the stale
            // label to the preheader's block index and places the init copy
            // on an edge that is not the live one, so the first iteration
            // reads an undefined register (SIGSEGV when the phi is an array
            // index — see tests/regression/loop_rotate_stale_phi_pred.c).
            // `phi.init` itself is unchanged and still dominates: it is defined
            // in the preheader, which dominates the guard and hence the body.
            // This matches the exit-phi construction of step 6.7, which
            // labels the guard-exit incoming with `header_label` too.
            incoming: vec![
                (phi.init, shape.header_label),
                (phi.next, shape.latch_label),
            ],
        });
    }
    // Insert the new phis at the TOP of body_latch (phis must precede all
    // other instructions in a block).
    let latch_block = &mut func.blocks[shape.latch_idx];
    latch_block.insert_insts(0, new_phi_insts, crate::common::source::Span::dummy());

    // v17 fix: rewrite the new self-loop phis' latch incomings to
    // reference the NEW self-loop phis (when the latch_op referenced a
    // header phi), NOT the OLD header phis. Without this rewrite, the
    // new self-loop phi's latch incoming references the OLD header phi,
    // which after step 10 (strip header phi's latch edge) collapses to
    // its preheader value (a constant), breaking the value rotation
    // in loops with cross-phi dependencies.
    //
    // Example (iterative Fibonacci, lccc's recursion-elimination output):
    //   Pre-rotation header:
    //     fib_a = phi(0, fib_b)        // fib_a_next = fib_b_old (cross-phi)
    //     fib_b = phi(1, fib_new)
    //   Body:
    //     fib_new = fib_a + fib_b
    //
    //   Without this fix (BUGGY, verified by IR dump + assembly diff):
    //     fib_a_loop = phi(0, fib_b)        // fib_b is OLD header phi
    //     fib_b_loop = phi(1, fib_new)
    //   After step 10, fib_b collapses to 1 (its preheader value),
    //   so fib_a_loop always reads 1 — fib(40) returns 39 (N-1) instead
    //   of 102334155.
    //
    //   With this fix (CORRECT):
    //     fib_a_loop = phi(0, fib_b_loop)   // fib_b_loop is NEW self-loop phi
    //     fib_b_loop = phi(1, fib_new)
    //   fib_a_loop correctly tracks the rotating fib_b value.
    //
    // The latch_op of a new phi is the pre-rotation header phi's latch
    // incoming. When that latch incoming is itself a header phi (the
    // cross-phi dependency), it must be rewritten to the corresponding
    // new self-loop phi. The `new_loop_phis` map (header_phi_dest ->
    // new_loop_phi_dest) provides the lookup. Only Operand::Value
    // variants can reference a header phi; Operand::Const and others
    // are left untouched.
    let latch_block = &mut func.blocks[shape.latch_idx];
    let n_new = plan.phis.len();
    for inst in latch_block.instructions.iter_mut().take(n_new) {
        if let Instruction::Phi { incoming, .. } = inst {
            for (op, _) in incoming.iter_mut() {
                if let Operand::Value(v) = op {
                    if let Some(&new_phi) = new_loop_phis.get(&v.0) {
                        *op = Operand::Value(Value(new_phi));
                    }
                }
            }
        }
    }

    // Rewrite uses of header phis in body_latch's (now-relocated) existing
    // instructions to the new loop phis. The new phis themselves are at the
    // top and were already processed by the v17 latch-incoming rewrite
    // above; `skip(n)` jumps past the n new phis so this loop only touches
    // the body's existing instructions.
    let latch_block = &mut func.blocks[shape.latch_idx];
    let n_new_phis = plan.phis.len();
    for inst in latch_block.instructions.iter_mut().skip(n_new_phis) {
        for (&old_phi, &new_phi) in &new_loop_phis {
            let repl = Operand::Value(Value(new_phi));
            replace_operand_value(inst, Value(old_phi), repl.clone());
        }
    }
    new_loop_phis
}

/// Step 6.7: what leaves the loop is merged at the top of the exit block.
fn merge_behind_the_loop(
    func: &mut IrFunction,
    lp: &NaturalLoop,
    cfg: &CfgAnalysis,
    plan: &Rotation,
    new_loop_phis: &FxHashMap<u32, u32>,
    next_val: &mut u32,
) {
    let shape = &plan.shape;
    let (exit_idx, exit_own_phis) = (plan.exit.exit_idx, plan.exit.own_phis);
    // 6.7 The merges in the exit block (decided by `exit_side`).
    //
    // A header phi read behind the loop (the accumulator a function
    // returns): after step 10 the header's phi is the value from in front of
    // the loop, and the value the loop computed arrives over the latch's
    // exit edge. `s_final = phi [(v_pre, guard), (v_latch, latch)]` at the
    // top of the exit block merges the two, and every reader behind the loop
    // reads it.
    //
    // `v_latch` is the header phi's latch incoming: what the body computed
    // in the iteration whose test failed (`s_new = s + a[i]`, `i_next = i +
    // 1`). Not the new self-loop phi, which still holds the value the
    // iteration began with (the last iteration's contribution would be
    // lost). When the latch incoming is itself a header phi (a swap,
    // `a_next = b`), that phi is the guard's after step 10 and its self-loop
    // phi is the value meant: `rewrite_header_phi_operand`.
    let mut exit_merge_map: FxHashMap<u32, u32> = FxHashMap::default();
    let mut exit_phi_insts: Vec<Instruction> = Vec::with_capacity(plan.exit.read_behind.len());
    for phi in &plan.exit.read_behind {
        let merge = Value(*next_val);
        *next_val += 1;
        exit_merge_map.insert(phi.dest, merge.0);
        exit_phi_insts.push(Instruction::Phi {
            dest: merge,
            ty: phi.ty,
            incoming: vec![
                (phi.init, shape.header_label), // the guard's edge: no iteration ran
                (
                    rewrite_header_phi_operand(phi.next, new_loop_phis),
                    shape.latch_label,
                ), // the latch's edge: the last iteration's value
            ],
        });
    }
    let merge_phis = exit_phi_insts.len();
    {
        let exit_block = &mut func.blocks[exit_idx];
        // The exit block's own phis: over the latch's edge each takes what
        // its incoming from the header would have been at the test that
        // failed. A header phi's value there is its latch incoming, as
        // above; anything else the header passed on is defined in front of
        // the loop (Guard H: the header's other values are read by the
        // header alone) and is the same on both edges.
        //
        // Without this the phi kept one incoming for two predecessors, and
        // the rewrite below gave that one incoming the merge defined beside
        // it: `v46 = phi [(v57, guard)]` with v57 a phi of the same block
        // (tests/regression/loop_rotate_exit_block_phis.c; seven tests of
        // the corpus did not compile with rotation on).
        for inst in exit_block.instructions.iter_mut().take(exit_own_phis) {
            let Instruction::Phi { incoming, .. } = inst else {
                continue;
            };
            let from_guard = incoming[0].0;
            let from_latch = match from_guard {
                Operand::Value(v) => plan
                    .phis
                    .iter()
                    .find(|phi| phi.dest == v.0)
                    .map_or(from_guard, |phi| {
                        rewrite_header_phi_operand(phi.next, new_loop_phis)
                    }),
                Operand::Const(_) => from_guard,
            };
            incoming.push((from_latch, shape.latch_label));
        }
        exit_block.insert_insts(0, exit_phi_insts, crate::common::source::Span::dummy());
    }
    if !exit_merge_map.is_empty() {
        // Every reader behind the loop reads the merge. The header and the
        // body are not behind the loop (the body's readers were given the
        // self-loop phis in step 6.6), and neither are the phis at the top
        // of the exit block: the merges themselves and the block's own.
        for (bi, block) in func.blocks.iter_mut().enumerate() {
            if bi == lp.header || lp.body.contains(&bi) || is_unreachable(bi, cfg) {
                continue;
            }
            let leading_phis = if bi == exit_idx {
                merge_phis + exit_own_phis
            } else {
                0
            };
            for inst in block.instructions.iter_mut().skip(leading_phis) {
                for (&old_phi, &merge) in &exit_merge_map {
                    replace_operand_value(inst, Value(old_phi), Operand::Value(Value(merge)));
                }
            }
            for (&old_phi, &merge) in &exit_merge_map {
                replace_terminator_value(
                    &mut block.terminator,
                    Value(old_phi),
                    Operand::Value(Value(merge)),
                );
            }
        }
    }
}

/// Steps 7 to 9: a copy of the test ends the latch.
fn put_the_test_at_the_latch(
    func: &mut IrFunction,
    plan: &Rotation,
    new_loop_phis: &FxHashMap<u32, u32>,
    next_val: &mut u32,
    debug: bool,
) {
    let shape = &plan.shape;
    // 7. Clone the closure instructions to the latch, allocating fresh dest
    //    IDs and rewriting: cloned refs → new dests, phi refs → latch values.
    //    If a header phi's latch incoming is itself a header phi (cross-phi),
    //    rewrite it to the new self-loop phi — the old header phi is about
    //    to collapse to its preheader value in step 10.
    let latch_values: Vec<(u32, Operand)> = plan
        .phis
        .iter()
        .map(|phi| {
            (
                phi.dest,
                rewrite_header_phi_operand(phi.next, new_loop_phis),
            )
        })
        .collect();
    let mut clone_map: FxHashMap<u32, u32> = FxHashMap::default();
    let mut cloned_insts: Vec<Instruction> = Vec::with_capacity(plan.test.len());
    for inst in &plan.test {
        let new_dest_opt = if let Some(d) = inst.dest() {
            let nd = Value(*next_val);
            *next_val += 1;
            clone_map.insert(d.0, nd.0);
            Some(nd)
        } else {
            None
        };
        let mut cloned = inst.clone();
        // First: rewrite Value operands that are cloned-instruction dests →
        // their fresh IDs. This handles references BETWEEN cloned
        // instructions (e.g. `BinOp And(c1, c2)` where both c1 and c2 are
        // cloned). `replace_values_in_inst_map` only touches reads, not dest.
        replace_values_in_inst_map(&mut cloned, &clone_map);
        // Second: rewrite phi references → latch-edge incoming operands.
        // Phi references are NOT in clone_map (phis are not cloned), so
        // `replace_values_in_inst_map` left them untouched.
        for &(phi_id, latch_op) in &latch_values {
            replace_operand_value(&mut cloned, Value(phi_id), latch_op);
        }
        // Third: rename the dest to the fresh ID.
        if new_dest_opt.is_some() {
            rename_inst_dest(&mut cloned, &clone_map);
        }
        cloned_insts.push(cloned);
    }
    // 7b. ZERO-ROT-AFFINE: fold the cloned exit comparison's affine operand
    //     into its constant bound (see `canonicalise_affine_exit_cmps`).
    //
    // `CCC_NO_AFFINE_EXIT_FOLD` must cover BOTH producers of the fold -- this
    // clone and the standalone pass.  The switch exists so a misbehaving fold
    // can be turned off without turning off rotation (which is a separate,
    // older transformation), and a switch that quietly leaves half the fold
    // running is worse than no switch: it makes the A/B that condemns the fold
    // the same A/B that hides it.
    let affine_folds = if affine_fold_enabled() {
        let proved_cmp = plan
            .affine_guard_proven
            .then(|| clone_map.get(&shape.cond.0).copied().map(Value))
            .flatten();
        canonicalise_affine_exit_cmps(&mut cloned_insts, proved_cmp)
    } else {
        0
    };
    if debug && affine_folds > 0 {
        lccc::cli::stderr_line(format_args!(
            "[ROT] affine exit-compare folds: {affine_folds}"
        ));
    }

    // The cloned cond value (new ID) is the latch's new CondBranch cond.
    let new_cond = Operand::Value(Value(*clone_map.get(&shape.cond.0).expect(
        "cond_val must be in clone_map (it was visited in the closure and has a dest)",
    )));

    // 8. Insert cloned instructions at the END of the latch (before the
    //    terminator, which we replace below). The latch's own instructions
    //    (e.g., the IV increment `i_next = i + 1`) must run BEFORE the test.
    let latch_block = &mut func.blocks[shape.latch_idx];
    latch_block.extend_insts(cloned_insts, crate::common::source::Span::dummy());

    // 9. Preserve the header's branch polarity. Classifying the targets as
    //    continue/exit does not negate the cloned condition: an exit-on-true
    //    header still needs an exit-on-true latch (e.g. !(n <= i)).
    latch_block.terminator = Terminator::CondBranch {
        cond: new_cond,
        true_label: if shape.continue_on_true {
            shape.continue_label
        } else {
            shape.exit_label
        },
        false_label: if shape.continue_on_true {
            shape.exit_label
        } else {
            shape.continue_label
        },
    };
}

/// If `op` is a header phi of the loop being rotated, rewrite it to the
/// corresponding new self-loop phi. Needed whenever a latch incoming or
/// cloned-closure operand still names the old header phi: after step 10
/// that phi collapses to the preheader value.
fn rewrite_header_phi_operand(op: Operand, new_loop_phis: &FxHashMap<u32, u32>) -> Operand {
    if let Operand::Value(v) = op {
        if let Some(&np) = new_loop_phis.get(&v.0) {
            return Operand::Value(Value(np));
        }
    }
    op
}

/// Check if a block label is inside the loop body (or is the header).
fn is_in_loop(label: BlockId, label_to_idx: &FxHashMap<BlockId, usize>, lp: &NaturalLoop) -> bool {
    label_to_idx
        .get(&label)
        .map(|&idx| idx == lp.header || lp.body.contains(&idx))
        .unwrap_or(false)
}

/// A block no path from the entry reaches: `compute_dominators` numbers the
/// blocks it walks to and leaves the immediate dominator of the others
/// unset. Nothing in such a block runs, and nothing dominates it.
fn is_unreachable(block_idx: usize, cfg: &CfgAnalysis) -> bool {
    cfg.idom
        .get(block_idx)
        .is_none_or(|&idom| idom == usize::MAX)
}

/// Returns true if `block_idx` is dominated by `dom_idx` — i.e. every path
/// from the function entry to `block_idx` passes through `dom_idx`. Walks the
/// `idom` chain: `dom_idx` must be an ancestor of `block_idx` in the dominator
/// tree. `block_idx == dom_idx` returns true (a block dominates itself).
///
/// Used by Guard B (`exit_side`): a reader of a header phi behind
/// the loop is rewritten to the merge phi of the exit block, which has to
/// dominate the reader's block, or for a phi reader the predecessor its
/// incoming names. A block no path reaches has no immediate dominator and
/// is dominated by nothing here; Guard B does not ask about those
/// (`is_unreachable`).
///
/// Complexity: O(depth of dom tree) — bounded by `num_blocks`. The walk
/// terminates because `idom[entry] == entry` (the entry block is its own
/// immediate dominator), so the chain always reaches a fixed point.
fn is_dominated_by(block_idx: usize, dom_idx: usize, cfg: &CfgAnalysis) -> bool {
    if block_idx == dom_idx {
        return true;
    }
    let mut cur = block_idx;
    // Bounded by num_blocks: the idom chain is at most num_blocks deep (a
    // degenerate chain that visits every block). Defensive guard against a
    // pathological cycle (shouldn't happen — idom is a tree — but a corrupt
    // CfgAnalysis could loop forever without this).
    for _ in 0..cfg.num_blocks {
        match cfg.idom.get(cur).copied() {
            Some(p) if p == dom_idx => return true,
            Some(p) if p == cur => return false, // reached root without finding dom_idx
            Some(p) => cur = p,
            None => return false, // block_idx out of range (shouldn't happen)
        }
    }
    false // dom chain longer than num_blocks — corrupt CfgAnalysis, fail closed
}

fn signed_type_bounds(ty: IrType) -> Option<(i64, i64)> {
    match ty {
        IrType::I8 => Some((i8::MIN as i64, i8::MAX as i64)),
        IrType::I16 => Some((i16::MIN as i64, i16::MAX as i64)),
        IrType::I32 => Some((i32::MIN as i64, i32::MAX as i64)),
        IrType::I64 => Some((i64::MIN, i64::MAX)),
        _ => None,
    }
}

/// ZERO-ROT-AFFINE: canonicalise the CLONED exit comparison
/// `icmp slt (add iv, C), N` -> `icmp slt iv, (N - C)`.
///
/// Rotation re-tests the loop condition in the latch, on the values the latch
/// just produced (`i_next`), so the cloned condition is
/// `icmp slt (add i_next, 4), 2048` and the backend materialises the `+4` as a
/// fresh `leaq`-style temporary before every compare -- the shape the
/// FOLLOWUP-2026-09-30-affine-exit-compare blocker measured at 5 instructions
/// per iteration against GCC's 4.  Folding the constant into the bound removes
/// the temporary AND gives the compare-branch fusion a register compare it can
/// fuse on the back edge (the rotation's licence).
///
/// Scope and justification:
/// * only freshly cloned temporaries whose ONLY use is the cloned compare are
///   rewritten (`use_count` is computed over `cloned`); the header's original
///   `add`/`cmp` pair is untouched, so nothing outside the rotated latch
///   changes;
/// * signed comparisons only, and only the cloned guard whose identity the
///   caller supplies after proving the original loop's executed IV range and
///   actual guard nonwrapping. Signed source types alone prove nothing: the
///   IR has no `nsw` flag and also represents `-fwrapv` and casted unsigned
///   arithmetic. Missing proof or a different comparison means refusal;
/// * the folded bound must be REPRESENTABLE in the comparison's type
///   (`checked_sub` + `signed_type_bounds`), so no wrap is ever introduced;
/// * constants may appear on either side of the `add` and the affine operand
///   may be on either side of the compare (`N > i + C` folds too).  The
///   comparison OPERATOR is preserved, never mirrored: mirroring the operand
///   side requires negating the relation as well, and taking one without the
///   other is exactly the double-negation bug the orientation oracle pins
///   (`N op (i + C) == (N - C) op i`, operator unchanged).
fn canonicalise_affine_exit_cmps(cloned: &mut [Instruction], proved_cmp: Option<Value>) -> usize {
    let Some(proved_cmp) = proved_cmp else {
        return 0;
    };
    // Uses of each cloned dest inside the cloned slice (fresh IDs cannot be
    // referenced anywhere else, so this is the function-wide count).
    let mut use_count: FxHashMap<u32, usize> = FxHashMap::default();
    for inst in cloned.iter() {
        inst.for_each_used_value(|v| *use_count.entry(v).or_default() += 1);
    }
    // Cloned `add iv, C` (or `add C, iv`) temporaries.
    let mut affine: FxHashMap<u32, (Operand, i64, IrType)> = FxHashMap::default();
    for inst in cloned.iter() {
        let Instruction::BinOp {
            dest,
            op: IrBinOp::Add,
            lhs,
            rhs,
            ty,
        } = inst
        else {
            continue;
        };
        let pair = match (lhs, rhs) {
            (Operand::Value(v), Operand::Const(c)) | (Operand::Const(c), Operand::Value(v)) => {
                c.to_i64().map(|c| (*v, c))
            }
            _ => None,
        };
        if let Some((v, c)) = pair {
            affine.insert(dest.0, (Operand::Value(v), c, *ty));
        }
    }

    let mut folded = 0usize;
    for inst in cloned.iter_mut() {
        let Instruction::Cmp {
            dest,
            lhs,
            op,
            rhs,
            ty,
        } = inst
        else {
            continue;
        };
        if *dest != proved_cmp {
            continue;
        }
        let (lhs_snapshot, rhs_snapshot) = (lhs.clone(), rhs.clone());
        let is_foldable = |v: Value| use_count.get(&v.0) == Some(&1) && affine.contains_key(&v.0);
        let (affine_val, flipped) = match (&lhs_snapshot, &rhs_snapshot) {
            (Operand::Value(v), _) if is_foldable(*v) => (*v, false),
            (_, Operand::Value(v)) if is_foldable(*v) => (*v, true),
            _ => continue,
        };
        // Signed order only -- see the doc comment.  The OPERATOR DOES NOT
        // CHANGE in either orientation: `iv + C op N` is `iv op N - C`, and
        // `N op (iv + C)` is `(N - C) op iv` -- subtracting the same constant
        // from both sides is what preserves the relation, and it is also why
        // the rewrite is exact rather than approximate.  (An earlier revision
        // mirrored the operator in the flipped case while ALSO swapping the
        // operands; two negations do not cancel, so `N > iv + C` came out as
        // `(N - C) < iv` -- for N=100, C=4, iv=0 that turned the true `100 > 4`
        // into the false `96 < 0`.  Pinned by `affine_fold_orientation_tests`.)
        let folded_op = match op {
            IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge => *op,
            _ => continue,
        };
        let Some((iv_op, c, add_ty)) = affine.get(&affine_val.0) else {
            continue;
        };
        if add_ty != ty {
            continue; // the add's width is what makes the constant affine
        }
        let Some((lo, hi)) = signed_type_bounds(*ty) else {
            continue;
        };
        let bound_op = if flipped {
            &lhs_snapshot
        } else {
            &rhs_snapshot
        };
        let Operand::Const(bound_c) = bound_op else {
            continue; // only an invariant constant bound is foldable here
        };
        let Some(bound) = bound_c.to_i64() else {
            continue;
        };
        let Some(new_bound) = bound.checked_sub(*c) else {
            continue; // would wrap: skip
        };
        if new_bound < lo || new_bound > hi {
            continue; // not representable in the comparison's type: skip
        }
        let new_bound_c = IrConst::from_i64(new_bound, *ty);
        if flipped {
            *lhs = Operand::Const(new_bound_c);
            *rhs = iv_op.clone();
        } else {
            *lhs = iv_op.clone();
            *rhs = Operand::Const(new_bound_c);
        }
        *op = folded_op;
        folded += 1;
    }
    folded
}

/// Mathematical domain for the sequence proof. U64's upper endpoint does
/// not fit in i64; use i128 until the final typed constant is encoded.
pub(crate) fn integer_type_bounds(ty: IrType) -> Option<(i128, i128)> {
    if !ty.is_integer() || !matches!(ty.size(), 1 | 2 | 4 | 8) {
        return None;
    }
    let bits = ty.size() * 8;
    Some(if ty.is_unsigned() {
        (0, (1_i128 << bits) - 1)
    } else {
        (-(1_i128 << (bits - 1)), (1_i128 << (bits - 1)) - 1)
    })
}

pub(crate) fn integer_constant(c: IrConst, ty: IrType) -> Option<i128> {
    integer_type_bounds(ty)?;
    let bits = ty.size() * 8;
    let value = c.to_i128()? & ((1_i128 << bits) - 1);
    Some(if !ty.is_unsigned() && value >= 1_i128 << (bits - 1) {
        value - (1_i128 << bits)
    } else {
        value
    })
}

fn comparison_matches_domain(op: IrCmpOp, ty: IrType) -> bool {
    if ty.is_unsigned() {
        matches!(
            op,
            IrCmpOp::Ult | IrCmpOp::Ule | IrCmpOp::Ugt | IrCmpOp::Uge
        )
    } else {
        matches!(
            op,
            IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge
        )
    }
}

/// Predicate: is this instruction safe to clone into the latch?
///
/// Safe = SSA-pure, no memory side effects, no calls, no atomics, no
/// intrinsics, no alloca. The allow-list is arithmetic/logic/cmp/cast/
/// copy/select/gep — the building blocks of loop guard conditions.
///
/// NOTE: Phi is deliberately EXCLUDED. Header phis are NOT cloned into
/// the latch — they are REWRITTEN to their latch-edge incoming values
/// (step 7's `replace_operand_value` with `HeaderPhi::next`). If a Phi
/// were cloned, the cloned Cmp would reference the cloned Phi (a
/// duplicate self-loop phi) instead of the post-increment `i_next`,
/// producing an off-by-one (the test reads the phi's stale value).
fn is_cloneable_pure(inst: &Instruction) -> bool {
    matches!(
        inst,
        Instruction::BinOp { .. }
            | Instruction::UnaryOp { .. }
            | Instruction::Cmp { .. }
            | Instruction::Cast { .. }
            | Instruction::Copy { .. }
            | Instruction::Select { .. }
            | Instruction::GetElementPtr { .. }
    )
}

// ── AFFOLD knobs ────────────────────────────────────────────────────────────
//
// `CCC_NO_AFFINE_EXIT_FOLD` and `CCC_DEBUG_AFFINE_FOLD` are resolved ONCE in
// `run_passes` (`src/passes/mod.rs`) and handed here, never read at a call
// site: the project policy is that the pass pipeline's environment reads are
// ratcheted in `check_env_test_hygiene.sh`, and a `thread_local` (not a
// `LazyLock`) because `run_passes` re-resolves it per translation unit while
// worker threads compile several on one process -- the same shape as the
// byte-compare arm's `LCCC_NO_BYTECMP_VEC`.
thread_local! {
    static AFFINE_FOLD_ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    static AFFINE_FOLD_DEBUG: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(crate) fn set_affine_fold_knobs(enabled: bool, debug: bool) {
    AFFINE_FOLD_ENABLED.with(|cell| cell.set(enabled));
    AFFINE_FOLD_DEBUG.with(|cell| cell.set(debug));
}

pub(crate) fn affine_fold_enabled() -> bool {
    AFFINE_FOLD_ENABLED.with(|cell| cell.get())
}

pub(crate) fn affine_fold_debug() -> bool {
    AFFINE_FOLD_DEBUG.with(|cell| cell.get())
}

// ── AFFOLD: the affine exit-compare fold ────────────────────────────────────

/// AFFOLD phase entry point: fold `iv + C <op> N` into `iv <op> N - C`
/// wherever the proof below holds, for every natural loop of `func`.
///
/// Why this is its own pass rather than a step of rotation: `canonicalise_
/// affine_exit_cmps` only ever saw the CLONE rotation emits, and rotation is
/// opt-in and refuses nested loops (Guard E) -- while every filter kernel's hot
/// loop is the inner loop of a nest.  The rewrite needs no CFG surgery (it is a
/// value equivalence), so it runs on every loop, default-on at -O2+ and on the
/// size pipelines, and reports through `CCC_DEBUG_AFFINE_FOLD`; kill switch
/// `CCC_NO_AFFINE_EXIT_FOLD`.
///
/// The rewrite trades an `add`-plus-`cmp` for the same `cmp` against a
/// pre-folded bound, which is what lets the x86 compare-branch fusion see a
/// register-vs-immediate compare on the back edge instead of a fresh
/// `leaq 4(...)` per iteration (measured: 4.914G Ir -> 4.095G, parity with GCC
/// 16.2, on the `i + 4 < 4096` shape inside a nest).
pub(crate) fn fold_affine_exit_compares(func: &mut IrFunction) -> usize {
    // The kill switch is checked FIRST: `CCC_DEBUG_AFFINE_FOLD=1` with the
    // switch on must report nothing at all (the gate's contract 2).
    if !affine_fold_enabled() {
        return 0;
    }
    // The value-based searches below are not a reaching-definitions analysis.
    // Fail closed on non-SSA webs rather than selecting a convenient writer.
    if !super::gvn::find_multi_def_values(func).is_empty() {
        return 0;
    }
    let cfg = CfgAnalysis::build(func);
    let raw = find_natural_loops(cfg.num_blocks, &cfg.preds, &cfg.succs, &cfg.idom);
    if raw.is_empty() {
        return 0;
    }
    let loops = merge_loops_by_header(raw);
    // Innermost first (smallest body), deterministic tie-break on the header
    // index: a nested loop's own fold must happen before its outer loop's
    // analysis can be affected by it, and two runs of the pass over the same
    // IR must produce the same order.
    let mut order: Vec<&NaturalLoop> = loops.iter().collect();
    order.sort_by_key(|lp| (lp.body.len(), lp.header));
    let mut total = 0;
    for lp in order {
        total += fold_loop(func, lp, &cfg);
        total += fold_loop_value_bound(func, lp, &cfg);
    }
    if total > 0 && affine_fold_debug() {
        lccc::cli::stderr_line(format_args!(
            "[AFFOLD] {}: affine exit-compare folds: {}",
            func.name, total
        ));
    }
    total
}

/// The induction-variable shape the proof needs: which phi, its constant seed
/// and the constant step of its latch increment.
struct IvShape {
    iv: u32,
    /// Value of the phi on entry to the loop.
    start: i128,
    /// Constant added by the latch (`step > 0`).
    step: i128,
    ty: IrType,
}

/// Classify a loop's induction variable BY VALUE.
///
/// Classify incoming edges by loop membership, not their order: a unique
/// outside edge supplies a constant seed; a unique inside edge supplies the
/// matching typed phi + constant step. Neither fact alone is a certificate.
fn loop_iv_shape(func: &IrFunction, lp: &NaturalLoop) -> Vec<IvShape> {
    let mut out = Vec::new();
    let body_set: FxHashSet<usize> = lp.body.iter().copied().collect();
    // The latch increment lives in whatever block the back edge leaves from
    // (the header itself, for a do-while self-loop), so the step search looks at
    // the whole loop body rather than at the phi's block.
    let body_insts: Vec<&Instruction> = lp
        .body
        .iter()
        .filter(|&&bi| bi < func.blocks.len())
        .flat_map(|&bi| func.blocks[bi].instructions.iter())
        .collect();
    for &bi in &lp.body {
        if bi >= func.blocks.len() {
            continue;
        }
        for inst in &func.blocks[bi].instructions {
            let Instruction::Phi { dest, incoming, ty } = inst else {
                continue;
            };
            if integer_type_bounds(*ty).is_none() {
                continue; // unsupported widths/types have no domain certificate
            }
            if let Some((start, step)) =
                phi_iv_shape(func, &body_set, &body_insts, dest, *ty, incoming)
            {
                if step > 0 {
                    out.push(IvShape {
                        iv: dest.0,
                        start,
                        step,
                        ty: *ty,
                    });
                }
            }
        }
    }
    out
}

/// The counted-loop shape of ONE phi, classified BY EDGE SIDE.
///
/// The phi's incoming edges are not classified by predecessor: a do-while
/// self-loop has the header as its own entry predecessor, and the "entry"
/// incoming may be the second slot.  What decides the role of an incoming is
/// whether its source block is INSIDE the loop body:
///
/// * outside = the entry edge, and its value must be a loop-invariant constant
///   (written directly, or materialised by the frontend as a `Copy` of one
///   defined outside the body) -- that is the seed;
/// * inside = the back edge, and its value must be `phi + Const` -- that is the
///   step.
///
/// Classifying by operand kind instead ("a constant is the seed, a `phi + C` is
/// the step" -- either of them anywhere) accepts two shapes it must not: a
/// constant arriving on the BACK edge, i.e. a loop that restarts its IV from
/// that constant every iteration while the real entry value is an unrelated
/// value, and an entry edge fed by a `phi + C` of some *other* loop.  In both,
/// `start` would be a number the first iteration never takes, and the no-wrap
/// obligation would be checked over the wrong range.  Requiring exactly one
/// incoming per side (`incoming.len() == 2` plus the two roles) keeps the
/// refusal total: anything else yields no shape, hence no fold.
fn phi_iv_shape(
    func: &IrFunction,
    body_set: &FxHashSet<usize>,
    body_insts: &[&Instruction],
    dest: &Value,
    ty: IrType,
    incoming: &[(Operand, BlockId)],
) -> Option<(i128, i128)> {
    if incoming.len() != 2 {
        return None;
    }
    let mut seed: Option<i128> = None;
    let mut step: Option<i128> = None;
    for (val, label) in incoming.iter() {
        let from_body = func
            .blocks
            .iter()
            .position(|b| b.label == *label)
            .is_some_and(|i| body_set.contains(&i));
        match (from_body, val) {
            // Entry edge, constant written directly.
            (false, Operand::Const(c)) => seed = Some(integer_constant(*c, ty)?),
            // Entry edge, constant through a `Copy` defined outside the body.
            (false, Operand::Value(v)) => {
                seed = Some(const_through_copy_outside(func, v.0, body_set, ty)?)
            }
            // Back edge: `phi + Const`, either operand order.
            (true, Operand::Value(v)) => {
                step = Some(body_insts.iter().find_map(|cand| match cand {
                    Instruction::BinOp {
                        dest: d2,
                        op: IrBinOp::Add,
                        lhs,
                        rhs,
                        ty: add_ty,
                    } if d2 == v && *add_ty == ty => match (lhs, rhs) {
                        (Operand::Value(p), Operand::Const(c))
                        | (Operand::Const(c), Operand::Value(p))
                            if p.0 == dest.0 =>
                        {
                            integer_constant(*c, ty)
                        }
                        _ => None,
                    },
                    _ => None,
                })?)
            }
            // A constant on the back edge, or a non-constant on the entry edge.
            _ => return None,
        }
    }
    Some((seed?, step?))
}

/// Constant `C` when `value` is a `Copy` of a constant defined OUTSIDE
/// `body_set` -- the form a loop's seed takes before copy propagation runs.
/// A copy made INSIDE the body is not a seed: its value is the previous
/// iteration's, so the loop is not the counted shape this proof is about.
fn const_through_copy_outside(
    func: &IrFunction,
    value: u32,
    body_set: &FxHashSet<usize>,
    ty: IrType,
) -> Option<i128> {
    func.blocks.iter().enumerate().find_map(|(bi, b)| {
        if body_set.contains(&bi) {
            return None;
        }
        b.instructions.iter().find_map(|i| match i {
            Instruction::Copy { dest, src } if dest.0 == value => match src {
                Operand::Const(c) => integer_constant(*c, ty),
                _ => None,
            },
            _ => None,
        })
    })
}

/// Constant `C` when `value` is defined as `iv + C` (either operand order) in
/// `func`, for the induction variable `iv`.  `None` for anything else --
/// including `iv` itself, which callers handle as the `C == 0` case.
fn affine_offset_of(func: &IrFunction, iv: u32, mut value: u32, ty: IrType) -> Option<i128> {
    let mut offset = 0_i128;
    // Rotation composes (phi + step) + bias. Follow a bounded, same-width
    // additive chain; never infer a value through a narrowing/sign cast.
    for _ in 0..16 {
        if value == iv {
            return Some(offset);
        }
        let inst = func
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .find(|i| i.dest() == Some(Value(value)))?;
        match inst {
            Instruction::BinOp {
                op: IrBinOp::Add,
                lhs,
                rhs,
                ty: add_ty,
                ..
            } if *add_ty == ty => {
                let (v, k) = match (lhs, rhs) {
                    (Operand::Value(v), Operand::Const(k))
                    | (Operand::Const(k), Operand::Value(v)) => (v, k),
                    _ => return None,
                };
                offset = offset.checked_add(integer_constant(*k, ty)?)?;
                value = v.0;
            }
            Instruction::Copy {
                src: Operand::Value(v),
                ..
            } => value = v.0,
            _ => return None,
        }
    }
    None // cycles/deep chains prove nothing
}

/// The exclusive upper bound `T` every test of `iv` in this loop respects,
/// interpreted in the comparison's signed or unsigned mathematical domain.  `None` when the
/// loop's exit is not a single branch on a constant-bounded comparison of
/// `iv` -- a runtime bound (`refuse_rt_bound`), several exits, or an exit that
/// tests something else entirely gives the proof nothing to stand on.
fn loop_exit_threshold(
    func: &IrFunction,
    lp: &NaturalLoop,
    shape: &IvShape,
    cfg: &CfgAnalysis,
) -> Option<i128> {
    let iv = shape.iv;
    let iv_ty = shape.ty;
    let mut exiting: Vec<(usize, &Terminator)> = Vec::new();
    for &bi in &lp.body {
        if bi >= func.blocks.len() {
            continue;
        }
        let term = &func.blocks[bi].terminator;
        if let Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } = term
        {
            let tl = func.blocks.iter().position(|b| b.label == *true_label);
            let fl = func.blocks.iter().position(|b| b.label == *false_label);
            let inside_t = tl.is_some_and(|t| lp.body.contains(&t));
            let inside_f = fl.is_some_and(|f| lp.body.contains(&f));
            if inside_t != inside_f {
                exiting.push((bi, term));
            }
        }
    }
    if exiting.len() != 1 {
        return None;
    }
    let (test_block, term) = exiting[0];
    // An optional interior test is not an induction bound: a bypass could
    // advance the IV past the certified interval before testing it again.
    // Every loop-closing edge must pass through this actual exit test.
    let mut saw_backedge = false;
    for &pred in cfg.preds.row(lp.header) {
        let pred = pred as usize;
        if lp.body.contains(&pred) {
            saw_backedge = true;
            if !is_dominated_by(pred, test_block, cfg) {
                return None;
            }
        }
    }
    if !saw_backedge {
        return None;
    }
    let Terminator::CondBranch {
        cond,
        true_label,
        false_label,
        ..
    } = term
    else {
        return None;
    };
    // POLARITY: which successor stays in the loop decides whether the loop
    // CONTINUES on the test or on its negation.  `continue_on_true` below is
    // that answer, and it is the load-bearing premise of the whole proof: the
    // set of IV values for which the loop continues must be a PREFIX of the IV
    // sequence.  With `continue_on_true`, only `<`/`<=` give a prefix; with
    // `continue_on_false`, only `>`/`>=` do (their negations).  Taking the
    // threshold from the wrong polarity would size the trip count against the
    // complement of the real continuation set -- e.g. a loop that keeps going
    // while `iv` is ABOVE the bound would have been credited with a prefix trip
    // count, and the no-wrap obligation would then be checked over the wrong
    // range.  Refusing here is the only sound answer: the sequence argument
    // below says nothing about upper-interval continuation.
    let continues_inside = |label: &BlockId| {
        func.blocks
            .iter()
            .position(|b| b.label == *label)
            .is_some_and(|i| lp.body.contains(&i))
    };
    let continue_on_true = continues_inside(true_label);
    let _ = continues_inside(false_label);
    let Operand::Value(cond_v) = cond else {
        return None;
    };
    let cmp = func.blocks.iter().find_map(|b| {
        b.instructions.iter().find_map(|i| match i {
            Instruction::Cmp {
                dest,
                lhs,
                op,
                rhs,
                ty,
            } if dest.0 == cond_v.0 && *ty == iv_ty => Some((lhs, op, rhs)),
            _ => None,
        })
    })?;
    let (lhs, op, rhs) = cmp;
    if !comparison_matches_domain(*op, iv_ty) {
        return None;
    }
    // One side names the IV (directly, or through `iv + C`) and contributes a
    // constant offset; the other side must be the constant bound.  A side that
    // is neither -- a runtime bound (`refuse_rt_bound`), an unrelated value --
    // leaves the proof nothing to stand on.
    let iv_side = |operand: &Operand| -> Option<i128> {
        match operand {
            Operand::Value(v) if v.0 == iv => Some(0),
            Operand::Value(v) => affine_offset_of(func, iv, v.0, iv_ty),
            _ => None,
        }
    };
    let (bound_c, c, op) = match (lhs, rhs) {
        // `iv + C <op> N`
        (l, Operand::Const(bc)) if iv_side(l).is_some() => {
            (integer_constant(*bc, iv_ty)?, iv_side(l)?, *op)
        }
        // `N <op> iv + C`: mirror so the affine side is on the left.
        (Operand::Const(bc), r) if iv_side(r).is_some() => {
            let mirrored = match op {
                IrCmpOp::Eq | IrCmpOp::Ne => return None,
                _ => op.swapped(),
            };
            (integer_constant(*bc, iv_ty)?, iv_side(r)?, mirrored)
        }
        _ => return None,
    };
    // The two polarities that admit a prefix continuation set: `op` is the
    // CONTINUATION test.  Anything else is the upper-interval case the polarity
    // comment above refuses.
    if continue_on_true {
        if !matches!(
            op,
            IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Ult | IrCmpOp::Ule
        ) {
            return None;
        }
    } else if !matches!(
        op,
        IrCmpOp::Sgt | IrCmpOp::Sge | IrCmpOp::Ugt | IrCmpOp::Uge
    ) {
        return None;
    }
    // Failing test of `iv + c <op> bound`, in terms of `iv`: the exclusive
    // upper bound of the values for which the loop continues.
    let bound = bound_c;
    let t = match op {
        IrCmpOp::Slt | IrCmpOp::Ult => bound,
        IrCmpOp::Sle | IrCmpOp::Ule => bound + 1,
        IrCmpOp::Sgt | IrCmpOp::Ugt => bound + 1,
        IrCmpOp::Sge | IrCmpOp::Uge => bound,
        _ => return None,
    } - c;
    let (lo, hi) = integer_type_bounds(iv_ty)?;
    // `t` is the derived exclusive bound; `bound` and `c` are the raw
    // operands it came from. Their containment follows from integer_constant
    // and the offset-chain admission today, but the guards are kept as cheap
    // defense-in-depth against future matcher extensions.
    if !(lo..=hi).contains(&t) || !(lo..=hi).contains(&bound) || !(lo..=hi).contains(&c) {
        return None;
    }
    // Prove the CONTROLLING expression itself, independently of whichever
    // comparison a caller wants to fold: if `iv + c` wraps on an executed
    // test, the exit is postponed PAST `t`, the executed IV sequence is no
    // longer a prefix of `start, start+step, ..`, and every range
    // certificate derived from `t` is void. Concretely (-fwrapv, step
    // 2^30-1): `i + 1073741830 < 2147483644` wraps negative on the second
    // test and keeps looping; affine_control_proof.c returns 95 instead of
    // 97 without this obligation. The sequence is monotone (`step > 0`),
    // so the first and the first-failing sums bound every executed sum.
    let last = first_failing_iv(shape, t)?;
    let first_sum = shape.start.checked_add(c)?;
    let last_sum = last.checked_add(c)?;
    if !(lo..=hi).contains(&last)
        || !(lo..=hi).contains(&first_sum)
        || !(lo..=hi).contains(&last_sum)
    {
        return None;
    }
    Some(t)
}

/// Last IV tested, including the first failing guard evaluation.
fn first_failing_iv(shape: &IvShape, threshold: i128) -> Option<i128> {
    if shape.step <= 0 || shape.start >= threshold {
        return None;
    }
    let span = threshold.checked_sub(shape.start)?;
    let iterations = span.checked_add(shape.step.checked_sub(1)?)? / shape.step;
    shape.start.checked_add(iterations.checked_mul(shape.step)?)
}

/// Fold every eligible affine comparison in one loop.  Returns the number of
/// comparisons rewritten.
fn fold_loop(func: &mut IrFunction, lp: &NaturalLoop, cfg: &CfgAnalysis) -> usize {
    // Uses of every value in the function: the affine temporary must have
    // exactly one (the comparison), or folding it neither removes the add nor
    // is provably local (`refuse_two_uses`).
    let mut uses: FxHashMap<u32, usize> = FxHashMap::default();
    for block in func.blocks.iter() {
        for inst in &block.instructions {
            inst.for_each_used_value(|v| *uses.entry(v).or_default() += 1);
        }
        block
            .terminator
            .for_each_used_value(|v| *uses.entry(v).or_default() += 1);
    }

    let shapes = loop_iv_shape(func, lp);
    if shapes.is_empty() {
        return 0;
    }
    // The loop's exit test must bound one of those IVs; the proof is about the
    // tests the loop actually performs, so a loop with no such bound has
    // nothing to prove with.
    let mut bound_iv: Option<(IvShape, i128)> = None;
    for shape in shapes {
        if let Some(t) = loop_exit_threshold(func, lp, &shape, cfg) {
            bound_iv = Some((shape, t));
            break;
        }
    }
    let Some((iv_shape, threshold)) = bound_iv else {
        return 0;
    };
    let mut folded = 0;
    for &bi in &lp.body.clone() {
        if bi >= func.blocks.len() {
            continue;
        }
        let n = func.blocks[bi].instructions.len();
        for idx in 0..n {
            let ty = match &func.blocks[bi].instructions[idx] {
                Instruction::Cmp { ty, .. } => *ty,
                _ => continue,
            };
            if ty != iv_shape.ty {
                continue; // width mismatch: the add's width is what makes it affine
            }
            if let Some((new_op, new_lhs, new_rhs)) =
                plan_affine_fold(func, &iv_shape, threshold, bi, idx, &uses)
            {
                if let Instruction::Cmp { lhs, op, rhs, .. } =
                    &mut func.blocks[bi].instructions[idx]
                {
                    *lhs = new_lhs;
                    *op = new_op;
                    *rhs = new_rhs;
                    folded += 1;
                }
            }
        }
    }
    folded
}

/// AFFOLD-VB: fold a VALUE-bounded affine exit compare, `iv + C <strict> n`,
/// into `iv <strict> t` with `t = n - C` computed once in the guard block.
///
/// The constant-bound fold above certifies the whole IV range against a
/// compile-time threshold.  With a runtime bound `n` no such range exists, so
/// this fold rests on a different, narrower proof.  Every clause below is
/// load-bearing; the counterexamples are in `affine_value_bound_tests`.
///
/// Soundness (exact integer arithmetic, no UB licence -- the IR also carries
/// `-fwrapv` code and the proof must not care):
///
/// * GUARD: the loop's unique outside predecessor `P` ends in
///   `CondBranch(k <strict> n, header, exit)` with constant `k >= start + C`
///   (`start` the IV's constant seed).  Every execution of the folded compare
///   is therefore dominated by the established fact `n > start + C`, so
///   `n - C > start >= type_min`: the pre-folded bound `t` CANNOT underflow on
///   any path that reaches the compare.  Without the guard, `n` near the type
///   minimum wraps `t` to a huge value and the folded loop runs where the
///   original exits immediately (`vb_refuses_unguarded`).
/// * STRICT comparison only (`Slt`/`Ult`): with `<=` and `n == type_max` the
///   original wrapping `iv + C` eventually laps the bound and the ORIGINAL
///   loop never exits, while the folded one does -- a finite loop is not a
///   faithful image of an infinite one (`vb_refuses_sle`).
/// * STEP == 1: by induction every compare-time value `x` satisfies
///   `x + C <= n <= type_max` -- base case from the guard, inductive step
///   `x' = x + 1 <= n - C` from the previous compare being true -- so the
///   original add never wraps and both compares decide identically.  A step
///   `s > 1` overshoots the bound by up to `s - 1`, `x + C` can exceed
///   `type_max` when `n` is near it, and the wrapped sum compares differently
///   (`vb_refuses_step2`).
/// * The affine temporary and the compare result are SINGLE-USE, the bound
///   `n` is defined outside the loop (SSA: one def, loop-invariant), and the
///   compare is the loop's unique exit test with every backedge dominated by
///   it -- the induction needs "reaching the compare again means the previous
///   compare was true".
///
/// The rewrite anchors on the LAST link of the affine chain (`a = x + C_rel`,
/// usually the post-increment), not on the phi: `x` is already live at the
/// compare, so no live range is lengthened, and the single-use add dies.
fn fold_loop_value_bound(func: &mut IrFunction, lp: &NaturalLoop, cfg: &CfgAnalysis) -> usize {
    const MAX_C: i128 = i64::MAX as i128;
    let header_label = match func.blocks.get(lp.header) {
        Some(b) => b.label,
        None => return 0,
    };
    // Uses of every value: the affine temporary and the compare result must
    // each have exactly one (see the single-use clause above).
    let mut uses: FxHashMap<u32, usize> = FxHashMap::default();
    for block in func.blocks.iter() {
        for inst in &block.instructions {
            inst.for_each_used_value(|v| *uses.entry(v).or_default() += 1);
        }
        block
            .terminator
            .for_each_used_value(|v| *uses.entry(v).or_default() += 1);
    }
    let shapes: Vec<IvShape> = loop_iv_shape(func, lp)
        .into_iter()
        .filter(|s| s.step == 1)
        .collect();
    if shapes.is_empty() {
        return 0;
    }
    // Unique exit test; every backedge must pass through it (same structural
    // requirements as `loop_exit_threshold`, for the same inductive reason).
    let mut exiting: Vec<usize> = Vec::new();
    for &bi in &lp.body {
        if bi >= func.blocks.len() {
            continue;
        }
        if let Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } = &func.blocks[bi].terminator
        {
            let tl = func.blocks.iter().position(|b| b.label == *true_label);
            let fl = func.blocks.iter().position(|b| b.label == *false_label);
            let inside_t = tl.is_some_and(|t| lp.body.contains(&t));
            let inside_f = fl.is_some_and(|f| lp.body.contains(&f));
            if inside_t != inside_f {
                exiting.push(bi);
            }
        }
    }
    if exiting.len() != 1 {
        return 0;
    }
    let test_block = exiting[0];
    let mut saw_backedge = false;
    for &pred in cfg.preds.row(lp.header) {
        let pred = pred as usize;
        if lp.body.contains(&pred) {
            saw_backedge = true;
            if !is_dominated_by(pred, test_block, cfg) {
                return 0;
            }
        }
    }
    if !saw_backedge {
        return 0;
    }
    let Terminator::CondBranch {
        cond, true_label, ..
    } = &func.blocks[test_block].terminator
    else {
        return 0;
    };
    // CONTINUE must be the true edge: the strict-less forms below are the
    // continuation test.  Exit-on-true carries the negated operator and is
    // out of this proof's scope.
    let continue_inside = func
        .blocks
        .iter()
        .position(|b| b.label == *true_label)
        .is_some_and(|i| lp.body.contains(&i));
    if !continue_inside {
        return 0;
    }
    let Operand::Value(cond_v) = *cond else {
        return 0;
    };
    if uses.get(&cond_v.0).copied() != Some(1) {
        return 0;
    }
    // The unique outside predecessor is the guard block.
    let mut outside_preds = cfg
        .preds
        .row(lp.header)
        .iter()
        .map(|&p| p as usize)
        .filter(|p| !lp.body.contains(p));
    let (Some(pre), None) = (outside_preds.next(), outside_preds.next()) else {
        return 0;
    };
    // Locate the compare (inside the loop: it consumes the IV).
    let cmp_loc = lp.body.iter().find_map(|&bi| {
        func.blocks
            .get(bi)?
            .instructions
            .iter()
            .enumerate()
            .find_map(|(ii, inst)| match inst {
                Instruction::Cmp { dest, .. } if dest.0 == cond_v.0 => Some((bi, ii)),
                _ => None,
            })
    });
    let Some((cmp_bi, cmp_ii)) = cmp_loc else {
        return 0;
    };
    let Instruction::Cmp {
        lhs, op, rhs, ty, ..
    } = func.blocks[cmp_bi].instructions[cmp_ii].clone()
    else {
        return 0;
    };
    for shape in &shapes {
        if shape.ty != ty || !comparison_matches_domain(op, ty) {
            continue;
        }
        let Some((lo, hi)) = integer_type_bounds(ty) else {
            continue;
        };
        // Normalize to `affine <strict> bound`; mirrored orientation flips
        // the operator.  Non-strict forms are refused (see `vb_refuses_sle`).
        let (affine_op, bound_op, strict_op) = match op {
            IrCmpOp::Slt | IrCmpOp::Ult => (&lhs, &rhs, op),
            IrCmpOp::Sgt => (&rhs, &lhs, IrCmpOp::Slt),
            IrCmpOp::Ugt => (&rhs, &lhs, IrCmpOp::Ult),
            _ => continue,
        };
        let Operand::Value(av) = *affine_op else {
            continue; // constant affine sides are the other fold's business
        };
        let Operand::Value(nv) = *bound_op else {
            continue; // constant bounds are `fold_loop`'s business
        };
        if uses.get(&av.0).copied() != Some(1) {
            continue;
        }
        let Some(c_total) = affine_offset_of(func, shape.iv, av.0, ty) else {
            continue;
        };
        if !(1..=MAX_C).contains(&c_total) {
            continue;
        }
        // The bound is loop-invariant: its single SSA def is outside the body.
        let n_def_inside = lp.body.iter().any(|&bi| {
            func.blocks
                .get(bi)
                .is_some_and(|b| b.instructions.iter().any(|i| i.dest() == Some(nv)))
        });
        if n_def_inside {
            continue;
        }
        // Anchor on the last additive link: `av = anchor + c_rel`.
        let Some((anchor, c_rel)) = last_add_link(func, av.0, ty) else {
            continue;
        };
        if !(1..=MAX_C).contains(&c_rel) {
            continue;
        }
        // Prefer the latch's post-increment as the anchor when the chain was
        // reassociated onto the phi (`(phi + 1) + 4` folded to `phi + 5`):
        // comparing the PRE-increment phi forces the backend to keep both the
        // old and the new IV live across the bottom test (a `mov` + taken
        // `jmp` per iteration), while the post-increment is the value the
        // tight `add; cmp; jcc` latch already has in hand.  Exactness is the
        // same induction: `phi + step <= phi + c_total <= n <= type_max`.
        let (anchor, c_rel) = if anchor == shape.iv && c_rel == c_total && c_total > shape.step {
            post_increment_anchor(func, cfg, lp, shape, cmp_bi, cmp_ii)
                .map(|inc| (inc, c_total - shape.step))
                .unwrap_or((anchor, c_rel))
        } else {
            (anchor, c_rel)
        };
        let first = match shape.start.checked_add(c_total) {
            Some(f) if (lo..=hi).contains(&f) => f,
            _ => continue,
        };
        // GUARD: a conditional branch above the loop whose edge towards the
        // header establishes `n > first - 1` (equivalently `n >= first`).
        if !guard_proves_bound(func, cfg, pre, header_label, nv, strict_op, ty, first) {
            continue;
        }
        // All obligations hold: fold.
        let tdest = Value(func.next_value_id);
        func.next_value_id += 1;
        let sub = Instruction::BinOp {
            dest: tdest,
            op: IrBinOp::Sub,
            lhs: Operand::Value(nv),
            rhs: Operand::Const(IrConst::from_i64(c_rel as i64, ty)),
            ty,
        };
        let pre_block = &mut func.blocks[pre];
        let span = pre_block
            .source_spans
            .last()
            .copied()
            .unwrap_or_else(crate::common::source::Span::dummy);
        pre_block.push_inst(sub, span);
        if let Instruction::Cmp { lhs, op, rhs, .. } = &mut func.blocks[cmp_bi].instructions[cmp_ii]
        {
            *lhs = Operand::Value(Value(anchor));
            *op = strict_op;
            *rhs = Operand::Value(tdest);
        }
        if affine_fold_debug() {
            lccc::cli::stderr_line(format_args!(
                "[AFFOLD] {}: value-bound fold: iv v{} + {} <strict> v{} -> v{} <strict> (v{} - {})",
                func.name, shape.iv, c_total, nv.0, anchor, nv.0, c_rel
            ));
        }
        return 1;
    }
    0
}

/// The loop's post-increment of `shape`'s phi — `inc = phi + step`, in the
/// fold's width — provided its definition dominates the comparison at
/// `(cmp_bi, cmp_ii)` so the rewritten compare may read it.
fn post_increment_anchor(
    func: &IrFunction,
    cfg: &CfgAnalysis,
    lp: &NaturalLoop,
    shape: &IvShape,
    cmp_bi: usize,
    cmp_ii: usize,
) -> Option<u32> {
    for &bi in &lp.body {
        let Some(block) = func.blocks.get(bi) else {
            continue;
        };
        for (ii, inst) in block.instructions.iter().enumerate() {
            let Instruction::BinOp {
                dest,
                op: IrBinOp::Add,
                lhs,
                rhs,
                ty,
            } = inst
            else {
                continue;
            };
            if *ty != shape.ty {
                continue;
            }
            let matches_inc = match (lhs, rhs) {
                (Operand::Value(v), Operand::Const(k)) | (Operand::Const(k), Operand::Value(v)) => {
                    v.0 == shape.iv && integer_constant(*k, shape.ty) == Some(shape.step)
                }
                _ => false,
            };
            if !matches_inc {
                continue;
            }
            let dominates = if bi == cmp_bi {
                ii < cmp_ii
            } else {
                is_dominated_by(cmp_bi, bi, cfg)
            };
            if dominates {
                return Some(dest.0);
            }
        }
    }
    None
}

/// The last link of an affine chain: `value = x + K` (through `Copy`s), with
/// the add in the comparison's own width.  Returns `(x, K)`.
fn last_add_link(func: &IrFunction, mut value: u32, ty: IrType) -> Option<(u32, i128)> {
    for _ in 0..16 {
        let inst = func
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .find(|i| i.dest() == Some(Value(value)))?;
        match inst {
            Instruction::BinOp {
                op: IrBinOp::Add,
                lhs,
                rhs,
                ty: add_ty,
                ..
            } if *add_ty == ty => {
                let (v, k) = match (lhs, rhs) {
                    (Operand::Value(v), Operand::Const(k))
                    | (Operand::Const(k), Operand::Value(v)) => (v, k),
                    _ => return None,
                };
                return Some((v.0, integer_constant(*k, ty)?));
            }
            Instruction::Copy {
                src: Operand::Value(v),
                ..
            } => value = v.0,
            _ => return None,
        }
    }
    None
}

/// Is `n >= first` established on every path into the loop?
///
/// The climb: starting at the loop's unique outside predecessor, walk up
/// through single-predecessor relay blocks (`Branch`-terminated preheaders
/// full of phi-elim copies are the normal shape) until a `CondBranch` is
/// found.  Exactly ONE of its edges may lead down the climbed chain; the
/// condition on that edge must pin `n` against a constant:
///
/// * edge-taken-when-TRUE:  `K < n`, `n > K`  (fact `n > K`),
///   `K <= n`, `n >= K`  (fact `n >= K`);
/// * edge-taken-when-FALSE: the negations — `n <= K`, `K >= n` (fact
///   `n > K`), `n < K`, `K > n` (fact `n >= K`).
///
/// A strict fact needs `K >= first - 1`, a non-strict one `K >= first`.  The
/// comparison domain must match the fold's (signed guard for a signed fold:
/// an unsigned fact says nothing about signed order).  `n` is an SSA value,
/// so the fact established once holds at every later execution point.
fn guard_proves_bound(
    func: &IrFunction,
    cfg: &CfgAnalysis,
    pre: usize,
    header_label: BlockId,
    nv: Value,
    strict_op: IrCmpOp,
    ty: IrType,
    first: i128,
) -> bool {
    let mut child_label = header_label;
    let mut cur = pre;
    for _ in 0..8 {
        let Some(cur_block) = func.blocks.get(cur) else {
            return false;
        };
        match &cur_block.terminator {
            Terminator::Branch(l) if *l == child_label => {
                // Relay block: climb to its unique predecessor.
                let mut preds = cfg.preds.row(cur).iter().map(|&p| p as usize);
                let (Some(p), None) = (preds.next(), preds.next()) else {
                    return false;
                };
                child_label = cur_block.label;
                cur = p;
            }
            Terminator::CondBranch {
                cond,
                true_label,
                false_label,
            } => {
                let enters_true = *true_label == child_label;
                let enters_false = *false_label == child_label;
                if enters_true == enters_false {
                    return false; // both or neither: no fact on the edge
                }
                let Operand::Value(gv) = *cond else {
                    return false;
                };
                let gcmp = func.blocks.iter().find_map(|b| {
                    b.instructions.iter().find_map(|i| match i {
                        Instruction::Cmp {
                            dest,
                            lhs,
                            op,
                            rhs,
                            ty: cmp_ty,
                        } if dest.0 == gv.0 && *cmp_ty == ty => Some((*lhs, *op, *rhs)),
                        _ => None,
                    })
                });
                let Some((glhs, gop, grhs)) = gcmp else {
                    return false;
                };
                // Normalize to a fact about `n` vs a constant `K` on the
                // entering edge.  `strict` is whether the fact is `n > K`
                // (versus `n >= K`).
                let signed = matches!(strict_op, IrCmpOp::Slt);
                let fact = guard_fact(glhs, gop, grhs, nv, ty, signed, enters_true);
                let Some((k, strict)) = fact else {
                    return false;
                };
                let need = if strict {
                    first.checked_sub(1)
                } else {
                    Some(first)
                };
                return need.is_some_and(|need| k >= need);
            }
            _ => return false,
        }
    }
    false
}

/// The fact `n > K` (strict) or `n >= K` (non-strict) carried by `cmp` on the
/// edge taken when the condition is `taken_on_true`, or `None` if the
/// comparison does not pin `nv` from below in the required domain.
fn guard_fact(
    lhs: Operand,
    op: IrCmpOp,
    rhs: Operand,
    nv: Value,
    ty: IrType,
    signed: bool,
    taken_on_true: bool,
) -> Option<(i128, bool)> {
    let k_lhs = |k: IrConst| integer_constant(k, ty);
    // (const K relates-to n) or (n relates-to const K), as (K, op-with-n-on-
    // the-RIGHT): mirror the second form.
    let (k, op) = match (lhs, rhs) {
        (Operand::Const(k), Operand::Value(v)) if v == nv => (k_lhs(k)?, op),
        (Operand::Value(v), Operand::Const(k)) if v == nv => {
            let mirrored = match op {
                IrCmpOp::Eq | IrCmpOp::Ne => return None,
                _ => op.swapped(),
            };
            (k_lhs(k)?, mirrored)
        }
        _ => return None,
    };
    // `op` now reads `K <op> n`.  The edge's fact is `op` itself when taken
    // on true, its negation when taken on false.
    let effective = if taken_on_true {
        op
    } else {
        match op {
            IrCmpOp::Eq | IrCmpOp::Ne => return None,
            _ => op.inverse(),
        }
    };
    // Keep only lower bounds on `n`, in the matching domain.
    match (signed, effective) {
        (true, IrCmpOp::Slt) => Some((k, true)),   // K < n
        (true, IrCmpOp::Sle) => Some((k, false)),  // K <= n
        (false, IrCmpOp::Ult) => Some((k, true)),  // K < n (unsigned)
        (false, IrCmpOp::Ule) => Some((k, false)), // K <= n (unsigned)
        _ => None,
    }
}

/// Prove the fold for ONE comparison and return the rewritten form, or `None`.
///
/// The rewrite `iv + C <op> N  ==  iv <op> N - C` is a pure integer identity
/// when `iv + C` is computed without wrapping.  Everything below exists to
/// prove that, exactly, for the values the loop can observe:
///
/// * the comparison must be `(iv + C) <op> N` or `N <op2> (iv + C)` with a
///   CONSTANT `C >= 0`, and the add's type must equal the comparison's type
///   (`refuse_width_mismatch` refuses a narrow compare of a wide add);
/// * the folded bound `N - C` must be representable in that type (the
///   `near_high` boundary: `N - C == i64::MAX - 4`);
/// * the values `iv` can take inside the loop are `start, start + step, ...`
///   up to and including the test that fails, i.e. indices `0..=L` where `L`
///   is the first index at or past the loop's exit threshold `T`.  The loop
///   must actually perform at least one continuing test (`L >= 1`); a shape
///   whose entry test already fails folds nothing and is refused
///   (`top_end_start`), which also keeps the accepted set inside what the
///   fixtures exercise;
/// * `start + C` and `t_L + C` must both be representable: the sequence is
///   monotone in `step > 0`, so those two checked endpoints cover every test
///   (`near_high`/`near_low` are the accepted boundaries, where the failing
///   test computes exactly the type maximum/minimum);
/// * all obligation arithmetic runs in `i128` -- a proof that itself wraps
///   proves nothing, and the failure mode of a wrapped proof is silent
///   acceptance of an unsound fold.
fn plan_affine_fold(
    func: &IrFunction,
    iv_shape: &IvShape,
    threshold: i128,
    block_idx: usize,
    inst_idx: usize,
    uses: &FxHashMap<u32, usize>,
) -> Option<(IrCmpOp, Operand, Operand)> {
    let (lo, hi) = integer_type_bounds(iv_shape.ty)?;
    let Instruction::Cmp {
        lhs, op, rhs, ty, ..
    } = &func.blocks[block_idx].instructions[inst_idx]
    else {
        return None;
    };
    if *ty != iv_shape.ty || !comparison_matches_domain(*op, *ty) {
        return None;
    }
    // The affine temporary on one side: a single-use `Add(iv, Const c)` of the
    // same type.
    let affine_of = |operand: &Operand| -> Option<i128> {
        let Operand::Value(v) = operand else {
            return None;
        };
        // One use only: otherwise the add survives the fold and the rewrite is
        // no longer local (`refuse_two_uses`).
        if uses.get(&v.0).copied() != Some(1) {
            return None;
        }
        affine_offset_of(func, iv_shape.iv, v.0, iv_shape.ty)
    };
    let affine_l = affine_of(lhs);
    let affine_r = affine_of(rhs);
    // Orientation: normalize to `(iv + C) <op> N` and pin the operator; a
    // comparison whose affine side is on the right is mirrored, and a
    // comparison that would run away as `iv` grows (`iv + C > N` and friends)
    // is refused -- the sequence argument below covers the terminating forms
    // only.
    let (c, bound, op) = match (affine_l, &lhs, &rhs, op) {
        (Some(c), _, Operand::Const(b), o)
            if matches!(o, IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Ult | IrCmpOp::Ule) =>
        {
            (c, integer_constant(*b, *ty)?, *o)
        }
        (None, Operand::Const(b), _, o) if affine_r.is_some() => {
            let mirrored = match o {
                IrCmpOp::Sgt => IrCmpOp::Slt,
                IrCmpOp::Sge => IrCmpOp::Sle,
                IrCmpOp::Ugt => IrCmpOp::Ult,
                IrCmpOp::Uge => IrCmpOp::Ule,
                _ => return None,
            };
            (affine_r?, integer_constant(*b, *ty)?, mirrored)
        }
        _ => return None,
    };
    if c < 0 {
        return None; // the accepted set is the ascending, non-negative-addend shape
    }
    let new_bound = bound.checked_sub(c)?;
    if new_bound < lo || new_bound > hi {
        return None;
    }
    // Sequence obligations (see the doc comment).
    let first = iv_shape.start.checked_add(c)?;
    if first < lo || first > hi {
        return None;
    }
    if iv_shape.start >= threshold {
        return None; // entry test already fails: L == 0
    }
    let span = threshold - iv_shape.start;
    let step = iv_shape.step;
    let l = (span + step - 1) / step; // ceil, step > 0
    if l < 1 {
        return None;
    }
    let last = iv_shape.start.checked_add(l.checked_mul(step)?)?;
    let last_plus_c = last.checked_add(c)?;
    if last_plus_c < lo || last_plus_c > hi {
        return None;
    }
    // Prefer an already-shared additive source, especially the latch's
    // post-increment value. Folding all the way back to the phi would keep
    // its old value live across the update and lose the original one-compare
    // loop. Both the anchor and the final sum need representable endpoints.
    let candidate = if affine_l.is_some() { lhs } else { rhs };
    let anchor = if let Operand::Value(sum) = candidate {
        func.blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .find_map(|i| {
                let Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs,
                    rhs,
                    ty: add_ty,
                } = i
                else {
                    return None;
                };
                if dest != sum || add_ty != ty {
                    return None;
                }
                let (v, k) = match (lhs, rhs) {
                    (Operand::Value(v), Operand::Const(k))
                    | (Operand::Const(k), Operand::Value(v)) => (v, k),
                    _ => return None,
                };
                if uses.get(&v.0).copied().unwrap_or(0) < 2 {
                    return None;
                }
                let bias = integer_constant(*k, *ty)?;
                let prefix = affine_offset_of(func, iv_shape.iv, v.0, *ty)?;
                let adjusted = bound.checked_sub(bias)?;
                if bias < 0
                    || !(lo..=hi).contains(&adjusted)
                    || !(lo..=hi).contains(&iv_shape.start.checked_add(prefix)?)
                    || !(lo..=hi).contains(&last.checked_add(prefix)?)
                {
                    return None;
                }
                Some((*v, adjusted))
            })
    } else {
        None
    };
    let (anchor, new_bound) = anchor.unwrap_or((Value(iv_shape.iv), new_bound));
    let new_bound_c = IrConst::from_i64(new_bound as i64, iv_shape.ty);
    Some((op, Operand::Value(anchor), Operand::Const(new_bound_c)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType as Ty;
    use crate::ir::reexports::BasicBlock;

    fn blk(label: u32, insts: Vec<Instruction>, term: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions: insts,
            terminator: term,
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        }
    }

    /// The canonical counted loop the fold is written for:
    ///
    /// ```text
    ///   0 entry:  <seed>            -> 1
    ///   1 header: phi i = [seed, 0], [i_next, 2]
    ///             cmp  iv_ty: (i + C) OP bound
    ///             CondBranch(cond, body, exit)
    ///   2 body:   branch -> 3
    ///   3 latch:  i_next = i + STEP ; branch -> 1
    ///   4 exit:   ret
    /// ```
    ///
    /// `op` and `bound` are parameters so the refused orientations can be
    /// built; `bound_const` says whether the bound is a constant (the
    /// `refuse_rt_bound` shape uses a live-in value instead).
    fn counted_loop(
        ty: Ty,
        start: i64,
        c: i64,
        op: IrCmpOp,
        bound: i64,
        bound_const: bool,
        step: i64,
        start_const: bool,
        step_const: bool,
    ) -> IrFunction {
        let mut f = IrFunction::new("affold".into(), Ty::I32, vec![], false);
        // Values: 1 = seed, 2 = phi, 3 = affine temp, 4 = cond, 5 = step const,
        // 6 = i_next, 7 = runtime bound, 8 = runtime step/start.
        let seed_insts = if start_const {
            vec![Instruction::Copy {
                dest: Value(1),
                src: Operand::Const(IrConst::from_i64(start, ty)),
            }]
        } else {
            vec![Instruction::ParamRef {
                dest: Value(1),
                param_idx: 0,
                ty,
            }]
        };
        let mut header = vec![
            Instruction::Phi {
                dest: Value(2),
                ty,
                incoming: vec![
                    (Operand::Value(Value(1)), BlockId(0)),
                    (Operand::Value(Value(6)), BlockId(3)),
                ],
            },
            Instruction::BinOp {
                dest: Value(3),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Const(IrConst::from_i64(c, ty)),
                ty,
            },
        ];
        let bound_op = if bound_const {
            Operand::Const(IrConst::from_i64(bound, ty))
        } else {
            header.push(Instruction::ParamRef {
                dest: Value(7),
                param_idx: 1,
                ty,
            });
            Operand::Value(Value(7))
        };
        header.push(Instruction::Cmp {
            dest: Value(4),
            lhs: Operand::Value(Value(3)),
            op,
            rhs: bound_op,
            ty,
        });
        let step_inst = if step_const {
            Instruction::BinOp {
                dest: Value(6),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Const(IrConst::from_i64(step, ty)),
                ty,
            }
        } else {
            Instruction::BinOp {
                dest: Value(6),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Value(Value(8)),
                ty,
            }
        };
        let mut latch = vec![step_inst];
        if !step_const {
            latch.insert(
                0,
                Instruction::ParamRef {
                    dest: Value(8),
                    param_idx: 2,
                    ty,
                },
            );
        }
        f.blocks
            .push(blk(0, seed_insts, Terminator::Branch(BlockId(1))));
        f.blocks.push(blk(
            1,
            header,
            Terminator::CondBranch {
                cond: Operand::Value(Value(4)),
                true_label: BlockId(2),
                false_label: BlockId(4),
            },
        ));
        f.blocks
            .push(blk(2, vec![], Terminator::Branch(BlockId(3))));
        f.blocks.push(blk(3, latch, Terminator::Branch(BlockId(1))));
        f.blocks.push(blk(4, vec![], Terminator::Return(None)));
        f
    }

    /// Bounded interpreter for the integer-only rotation fixture. Phi reads
    /// are simultaneous, and an inverted backedge cannot hang the test suite.
    fn interpret_rotation_fixture(f: &IrFunction) -> i64 {
        fn read(op: Operand, values: &FxHashMap<u32, i64>) -> i64 {
            match op {
                Operand::Const(c) => c.to_i64().unwrap(),
                Operand::Value(v) => values[&v.0],
            }
        }
        let mut values = FxHashMap::default();
        let mut at = f.blocks[0].label;
        let mut previous = at;
        for _ in 0..64 {
            let block = f.blocks.iter().find(|b| b.label == at).unwrap();
            let phis: Vec<_> = block
                .instructions
                .iter()
                .filter_map(|inst| {
                    if let Instruction::Phi { dest, incoming, .. } = inst {
                        let op = incoming
                            .iter()
                            .find(|(_, label)| *label == previous)
                            .unwrap()
                            .0;
                        Some((dest.0, read(op, &values)))
                    } else {
                        None
                    }
                })
                .collect();
            values.extend(phis);
            for inst in &block.instructions {
                let result = match inst {
                    Instruction::Phi { .. } => continue,
                    Instruction::Copy { dest, src } => (dest.0, read(*src, &values)),
                    Instruction::BinOp {
                        dest,
                        op: IrBinOp::Add,
                        lhs,
                        rhs,
                        ..
                    } => (dest.0, read(*lhs, &values) + read(*rhs, &values)),
                    Instruction::Cmp {
                        dest, op, lhs, rhs, ..
                    } => {
                        let (l, r) = (read(*lhs, &values), read(*rhs, &values));
                        let truth = match op {
                            IrCmpOp::Slt => l < r,
                            IrCmpOp::Sge => l >= r,
                            IrCmpOp::Sgt => l > r,
                            IrCmpOp::Sle => l <= r,
                            _ => panic!("unexpected fixture comparison"),
                        };
                        (dest.0, i64::from(truth))
                    }
                    _ => panic!("unexpected fixture instruction"),
                };
                values.insert(result.0, result.1);
            }
            previous = at;
            at = match block.terminator {
                Terminator::Branch(next) => next,
                Terminator::CondBranch {
                    cond,
                    true_label,
                    false_label,
                } => {
                    if read(cond, &values) != 0 {
                        true_label
                    } else {
                        false_label
                    }
                }
                Terminator::Return(Some(op)) => return read(op, &values),
                _ => panic!("unexpected fixture terminator"),
            };
        }
        panic!("rotated fixture did not terminate within its bound");
    }

    #[test]
    fn rotation_preserves_both_guard_polarities_and_operand_orders() {
        for seed in [-3, 0, 2, 3, 4] {
            for continue_on_true in [false, true] {
                for swapped in [false, true] {
                    let op = match (continue_on_true, swapped) {
                        (true, false) => IrCmpOp::Slt,
                        (true, true) => IrCmpOp::Sgt,
                        (false, false) => IrCmpOp::Sge,
                        (false, true) => IrCmpOp::Sle,
                    };
                    let mut f = counted_loop(Ty::I32, seed, 0, op, 3, true, 1, true, true);
                    // Single-block body; labels deliberately differ from indices.
                    f.blocks.remove(2);
                    f.blocks[1].terminator = Terminator::CondBranch {
                        cond: Operand::Value(Value(4)),
                        true_label: BlockId(if continue_on_true { 3 } else { 4 }),
                        false_label: BlockId(if continue_on_true { 4 } else { 3 }),
                    };
                    if swapped {
                        if let Instruction::Cmp { lhs, rhs, .. } = &mut f.blocks[1].instructions[2]
                        {
                            std::mem::swap(lhs, rhs);
                        }
                    }
                    f.blocks[3].terminator = Terminator::Return(Some(Operand::Value(Value(2))));
                    f.next_value_id = 20;
                    f.next_label = 5;
                    let expected = interpret_rotation_fixture(&f);
                    assert_eq!(expected, seed.max(3));
                    let cfg = CfgAnalysis::build(&f);
                    let loops = merge_loops_by_header(find_natural_loops(
                        cfg.num_blocks,
                        &cfg.preds,
                        &cfg.succs,
                        &cfg.idom,
                    ));
                    let headers = loops.iter().map(|lp| lp.header).collect();
                    assert_eq!(loops.len(), 1);
                    let cx = RotationContext::new(&f, &cfg, &headers);
                    assert!(try_rotate_loop(&mut f, &loops[0], &cx));
                    let mut violations = Vec::new();
                    crate::passes::verify::verify_function(
                        &f,
                        "rotation_polarity",
                        &mut violations,
                    );
                    assert!(violations.is_empty(), "{violations:?}");
                    assert_eq!(interpret_rotation_fixture(&f), expected);
                }
            }
        }
    }

    /// Who, outside the header, reads the value the header computes.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Reader {
        Nobody,
        Body,
        Exit,
        Both,
    }

    /// `for (i = seed, s = 0; (t = i + c) < bound; i++) s += x; return s + y;`
    /// with `x` the header's `t` or the counter, `y` the header's `t` or 0:
    ///
    /// ```text
    ///   0 entry:  v1 = seed                       -> 1
    ///   1 header: v2 = phi i [v1, 0], [v6, 3]
    ///             v5 = phi s [0, 0],  [v7, 3]
    ///             v3 = v2 + c                     (t)
    ///             v4 = v3 < bound
    ///             CondBranch(v4, 3, 4)
    ///   3 body:   v7 = v5 + (v3 | v2)
    ///             v6 = v2 + 1                     -> 1
    ///   4 exit:   v8 = v5 + (v3 | 0)
    ///             ret v8
    /// ```
    fn loop_with_a_header_value(seed: i64, c: i64, bound: i64, reader: Reader) -> IrFunction {
        let ty = Ty::I32;
        let constant = |value: i64| Operand::Const(IrConst::from_i64(value, ty));
        let value = |id: u32| Operand::Value(Value(id));
        let add = |dest: u32, lhs: Operand, rhs: Operand| Instruction::BinOp {
            dest: Value(dest),
            op: IrBinOp::Add,
            lhs,
            rhs,
            ty,
        };
        let in_body = matches!(reader, Reader::Body | Reader::Both);
        let in_exit = matches!(reader, Reader::Exit | Reader::Both);
        let mut f = IrFunction::new("header_value".into(), ty, vec![], false);
        f.blocks.push(blk(
            0,
            vec![Instruction::Copy {
                dest: Value(1),
                src: constant(seed),
            }],
            Terminator::Branch(BlockId(1)),
        ));
        f.blocks.push(blk(
            1,
            vec![
                Instruction::Phi {
                    dest: Value(2),
                    ty,
                    incoming: vec![(value(1), BlockId(0)), (value(6), BlockId(3))],
                },
                Instruction::Phi {
                    dest: Value(5),
                    ty,
                    incoming: vec![(constant(0), BlockId(0)), (value(7), BlockId(3))],
                },
                add(3, value(2), constant(c)),
                Instruction::Cmp {
                    dest: Value(4),
                    op: IrCmpOp::Slt,
                    lhs: value(3),
                    rhs: constant(bound),
                    ty,
                },
            ],
            Terminator::CondBranch {
                cond: value(4),
                true_label: BlockId(3),
                false_label: BlockId(4),
            },
        ));
        f.blocks.push(blk(
            3,
            vec![
                add(7, value(5), value(if in_body { 3 } else { 2 })),
                add(6, value(2), constant(1)),
            ],
            Terminator::Branch(BlockId(1)),
        ));
        f.blocks.push(blk(
            4,
            vec![add(
                8,
                value(5),
                if in_exit { value(3) } else { constant(0) },
            )],
            Terminator::Return(Some(value(8))),
        ));
        f.next_value_id = 20;
        f.next_label = 5;
        f
    }

    /// What the loop above returns, computed from its C form.
    fn header_value_loop_result(seed: i64, c: i64, bound: i64, reader: Reader) -> i64 {
        let (mut i, mut s) = (seed, 0);
        while i + c < bound {
            s += if matches!(reader, Reader::Body | Reader::Both) {
                i + c
            } else {
                i
            };
            i += 1;
        }
        s + if matches!(reader, Reader::Exit | Reader::Both) {
            i + c
        } else {
            0
        }
    }

    fn rotate_the_only_loop(f: &mut IrFunction) -> bool {
        let cfg = CfgAnalysis::build(f);
        let loops = merge_loops_by_header(find_natural_loops(
            cfg.num_blocks,
            &cfg.preds,
            &cfg.succs,
            &cfg.idom,
        ));
        let headers = loops.iter().map(|lp| lp.header).collect();
        assert_eq!(loops.len(), 1);
        let cx = RotationContext::new(f, &cfg, &headers);
        try_rotate_loop(f, &loops[0], &cx)
    }

    fn written(f: &IrFunction) -> Vec<String> {
        f.blocks
            .iter()
            .map(|block| format!("{:?} {:?}", block.instructions, block.terminator))
            .collect()
    }

    /// The rotated loop computes what the loop computed, whoever reads the
    /// header's value. Held as a property of the function, so it stands for a
    /// rotation that builds the missing merges as it stands for a refusal: no
    /// seed, addend or reader may change the result.
    ///
    /// Without Guard H the body adds the guard's `t` on every iteration and
    /// the code behind the loop reads the guard's `t`: 50 of the 96 loops
    /// with a reader compute another result -- 14 of 32 with the reader in
    /// the body (every loop of two or more iterations), 18 of 32 with it
    /// behind the loop and 18 of 32 with both (every loop that iterates).
    #[test]
    fn rotation_keeps_every_reader_of_a_header_value_right() {
        let mut rotated = 0;
        let mut loops = 0;
        let mut wrong = Vec::new();
        for reader in [Reader::Nobody, Reader::Body, Reader::Exit, Reader::Both] {
            for seed in [-3, 0, 1, 2, 3, 4, 5, 9] {
                for c in [0, 1, 3, -2] {
                    let bound = 4;
                    let mut f = loop_with_a_header_value(seed, c, bound, reader);
                    let expected = header_value_loop_result(seed, c, bound, reader);
                    assert_eq!(
                        interpret_rotation_fixture(&f),
                        expected,
                        "the fixture is not the loop it is said to be: {reader:?} {seed} {c}"
                    );
                    loops += 1;
                    rotated += usize::from(rotate_the_only_loop(&mut f));
                    let mut violations = Vec::new();
                    crate::passes::verify::verify_function(&f, "header_value", &mut violations);
                    assert!(
                        violations.is_empty(),
                        "{reader:?} {seed} {c}: {violations:?}"
                    );
                    let computed = interpret_rotation_fixture(&f);
                    if computed != expected {
                        wrong.push(format!(
                            "{reader:?}, seed {seed}, addend {c}: {computed} for {expected}"
                        ));
                    }
                }
            }
        }
        assert!(
            wrong.is_empty(),
            "{} of {loops} loops compute another result once rotated:\n{}",
            wrong.len(),
            wrong.join("\n")
        );
        // The loops nobody else reads are rotated: the property above is not
        // held by rotating nothing.
        assert_eq!(rotated, 32);
    }

    /// Guard H as it stands: a reader outside the header is a refusal, and a
    /// refusal leaves the function as it was -- nothing renamed, no phi
    /// added, no value number taken.
    #[test]
    fn a_header_value_read_outside_the_header_is_refused_untouched() {
        for reader in [Reader::Body, Reader::Exit, Reader::Both] {
            let mut f = loop_with_a_header_value(0, 1, 4, reader);
            let before = written(&f);
            assert!(!rotate_the_only_loop(&mut f), "{reader:?}");
            assert_eq!(written(&f), before, "{reader:?}");
            assert_eq!(f.next_value_id, 20, "{reader:?}");
        }
        let mut f = loop_with_a_header_value(0, 1, 4, Reader::Nobody);
        let before = written(&f);
        assert!(rotate_the_only_loop(&mut f));
        assert_ne!(written(&f), before);
    }

    /// How the code behind `for (i = seed, s = 0; i < bound; i++) s += i;`
    /// reads the loop.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Behind {
        /// The exit block has phis of its own for the sum, the counter, a
        /// constant and a value from in front of the loop (what the header
        /// of a following loop looks like once that loop is rotated), and
        /// reads the sum directly as well.
        ExitPhis,
        /// `if (seed < 100) { loop } else { s = 77; i = seed; }`: the sum
        /// and the counter are merged in a block the exit block does not
        /// dominate, over an edge it does.
        MergeBehind,
        /// The entry jumps around the loop to the exit block: two
        /// predecessors (Guard A).
        JumpAround,
        /// An exit phi with an incoming for an edge that does not exist.
        StaleExitPhi,
        /// A block nothing branches to reads the sum and the counter (what
        /// the unroller leaves behind until the next CFG cleanup).
        DeadReader,
    }

    /// ```text
    ///   0 entry:  v1 = seed                              -> 1 (or see below)
    ///   1 header: v2 = phi i [v1, 0], [v6, 3]
    ///             v5 = phi s [0, 0],  [v7, 3]
    ///             v4 = v2 < bound
    ///             CondBranch(v4, 3, 4)
    ///   3 body:   v7 = v5 + v2 ; v6 = v2 + 1             -> 1
    ///   4 exit:   ...
    /// ```
    fn loop_read_behind(seed: i64, bound: i64, behind: Behind) -> IrFunction {
        let ty = Ty::I32;
        let constant = |value: i64| Operand::Const(IrConst::from_i64(value, ty));
        let value = |id: u32| Operand::Value(Value(id));
        let add = |dest: u32, lhs: Operand, rhs: Operand| Instruction::BinOp {
            dest: Value(dest),
            op: IrBinOp::Add,
            lhs,
            rhs,
            ty,
        };
        let phi = |dest: u32, incoming: Vec<(Operand, u32)>| Instruction::Phi {
            dest: Value(dest),
            ty,
            incoming: incoming
                .into_iter()
                .map(|(op, label)| (op, BlockId(label)))
                .collect(),
        };
        let mut f = IrFunction::new("read_behind".into(), ty, vec![], false);
        let mut entry = vec![Instruction::Copy {
            dest: Value(1),
            src: constant(seed),
        }];
        let entry_term = match behind {
            Behind::ExitPhis | Behind::StaleExitPhi | Behind::DeadReader => {
                Terminator::Branch(BlockId(1))
            }
            Behind::MergeBehind | Behind::JumpAround => {
                entry.push(Instruction::Cmp {
                    dest: Value(20),
                    op: IrCmpOp::Slt,
                    lhs: value(1),
                    rhs: constant(100),
                    ty,
                });
                Terminator::CondBranch {
                    cond: value(20),
                    true_label: BlockId(1),
                    false_label: BlockId(if behind == Behind::MergeBehind { 5 } else { 4 }),
                }
            }
        };
        f.blocks.push(blk(0, entry, entry_term));
        f.blocks.push(blk(
            1,
            vec![
                phi(2, vec![(value(1), 0), (value(6), 3)]),
                phi(5, vec![(constant(0), 0), (value(7), 3)]),
                Instruction::Cmp {
                    dest: Value(4),
                    op: IrCmpOp::Slt,
                    lhs: value(2),
                    rhs: constant(bound),
                    ty,
                },
            ],
            Terminator::CondBranch {
                cond: value(4),
                true_label: BlockId(3),
                false_label: BlockId(4),
            },
        ));
        f.blocks.push(blk(
            3,
            vec![add(7, value(5), value(2)), add(6, value(2), constant(1))],
            Terminator::Branch(BlockId(1)),
        ));
        match behind {
            Behind::ExitPhis | Behind::StaleExitPhi => {
                let mut sum = vec![(value(5), 1)];
                if behind == Behind::StaleExitPhi {
                    sum.push((constant(9), 0));
                }
                f.blocks.push(blk(
                    4,
                    vec![
                        phi(8, sum),
                        phi(9, vec![(value(2), 1)]),
                        phi(10, vec![(constant(100), 1)]),
                        phi(11, vec![(value(1), 1)]),
                        add(12, value(8), value(9)),
                        add(13, value(12), value(10)),
                        add(14, value(13), value(11)),
                        add(15, value(14), value(5)),
                    ],
                    Terminator::Return(Some(value(15))),
                ));
            }
            Behind::MergeBehind => {
                f.blocks
                    .push(blk(4, vec![], Terminator::Branch(BlockId(6))));
                f.blocks.push(blk(
                    5,
                    vec![Instruction::Copy {
                        dest: Value(21),
                        src: constant(77),
                    }],
                    Terminator::Branch(BlockId(6)),
                ));
                f.blocks.push(blk(
                    6,
                    vec![
                        phi(8, vec![(value(5), 4), (value(21), 5)]),
                        phi(9, vec![(value(2), 4), (value(1), 5)]),
                        add(10, value(8), value(9)),
                    ],
                    Terminator::Return(Some(value(10))),
                ));
            }
            Behind::JumpAround => {
                f.blocks.push(blk(
                    4,
                    vec![
                        phi(8, vec![(value(5), 1), (constant(77), 0)]),
                        phi(9, vec![(value(2), 1), (value(1), 0)]),
                        add(10, value(8), value(9)),
                    ],
                    Terminator::Return(Some(value(10))),
                ));
            }
            Behind::DeadReader => {
                f.blocks.push(blk(
                    4,
                    vec![add(10, value(5), value(2))],
                    Terminator::Return(Some(value(10))),
                ));
                f.blocks.push(blk(
                    5,
                    vec![add(21, value(5), value(2))],
                    Terminator::Unreachable,
                ));
            }
        }
        f.next_value_id = 30;
        f.next_label = 7;
        f
    }

    /// What the function above returns, computed from its C form.
    fn read_behind_result(seed: i64, bound: i64, behind: Behind) -> i64 {
        let (mut i, mut s) = (seed, 0);
        let looped = !matches!(behind, Behind::MergeBehind | Behind::JumpAround) || seed < 100;
        if looped {
            while i < bound {
                s += i;
                i += 1;
            }
        }
        match behind {
            Behind::ExitPhis | Behind::StaleExitPhi => s + i + 100 + seed + s,
            Behind::DeadReader => s + i,
            Behind::MergeBehind | Behind::JumpAround if looped => s + i,
            Behind::MergeBehind | Behind::JumpAround => 77 + seed,
        }
    }

    fn rotated_is_valid_and_computes(f: &mut IrFunction, expected: i64, what: &str) {
        assert_eq!(
            interpret_rotation_fixture(f),
            expected,
            "{what}: the fixture"
        );
        assert!(rotate_the_only_loop(f), "{what}: not rotated");
        let mut violations = Vec::new();
        crate::passes::verify::verify_function(f, "read_behind", &mut violations);
        assert!(violations.is_empty(), "{what}: {violations:?}");
        assert_eq!(interpret_rotation_fixture(f), expected, "{what}: rotated");
    }

    /// The phis the exit block already has get an incoming for the latch's
    /// edge. Before: one incoming for two predecessors, and for a phi that
    /// read a header phi that one incoming was rewritten to the merge phi
    /// defined beside it -- the verifier's "phi arity" and "phi dominance"
    /// violations with which seven tests of tests/regression failed to
    /// compile under CCC_LOOP_ROTATE=1.
    #[test]
    fn rotation_gives_the_exit_blocks_own_phis_the_latch_edge() {
        for seed in [-3, 0, 2, 3, 4, 7] {
            let mut f = loop_read_behind(seed, 4, Behind::ExitPhis);
            let expected = read_behind_result(seed, 4, Behind::ExitPhis);
            rotated_is_valid_and_computes(&mut f, expected, &format!("seed {seed}"));
            let exit = f.blocks.iter().find(|b| b.label == BlockId(4)).unwrap();
            for inst in &exit.instructions {
                let Instruction::Phi { dest, incoming, .. } = inst else {
                    continue;
                };
                let mut labels: Vec<u32> = incoming.iter().map(|(_, label)| label.0).collect();
                labels.sort_unstable();
                assert_eq!(labels, [1, 3], "v{}: one incoming per predecessor", dest.0);
            }
        }
    }

    /// `x, y = y + j, x` with an exit block whose own phis read both. Over
    /// the latch's edge `y` is what `x` was during the last iteration, its
    /// self-loop phi; the header's `x` is the guard's value once rotated.
    #[test]
    fn an_exit_phi_of_a_swapped_value_takes_the_last_iterations() {
        let ty = Ty::I32;
        let constant = |value: i64| Operand::Const(IrConst::from_i64(value, ty));
        let value = |id: u32| Operand::Value(Value(id));
        let add = |dest: u32, lhs: Operand, rhs: Operand| Instruction::BinOp {
            dest: Value(dest),
            op: IrBinOp::Add,
            lhs,
            rhs,
            ty,
        };
        let phi = |dest: u32, incoming: Vec<(Operand, u32)>| Instruction::Phi {
            dest: Value(dest),
            ty,
            incoming: incoming
                .into_iter()
                .map(|(op, label)| (op, BlockId(label)))
                .collect(),
        };
        for bound in 0..6 {
            let mut f = IrFunction::new("swap".into(), ty, vec![], false);
            f.blocks
                .push(blk(0, vec![], Terminator::Branch(BlockId(1))));
            f.blocks.push(blk(
                1,
                vec![
                    phi(2, vec![(constant(0), 0), (value(6), 3)]),
                    phi(3, vec![(constant(1), 0), (value(7), 3)]),
                    phi(5, vec![(constant(2), 0), (value(3), 3)]),
                    Instruction::Cmp {
                        dest: Value(4),
                        op: IrCmpOp::Slt,
                        lhs: value(2),
                        rhs: constant(bound),
                        ty,
                    },
                ],
                Terminator::CondBranch {
                    cond: value(4),
                    true_label: BlockId(3),
                    false_label: BlockId(4),
                },
            ));
            f.blocks.push(blk(
                3,
                vec![add(7, value(5), value(2)), add(6, value(2), constant(1))],
                Terminator::Branch(BlockId(1)),
            ));
            f.blocks.push(blk(
                4,
                vec![
                    phi(8, vec![(value(3), 1)]),
                    phi(9, vec![(value(5), 1)]),
                    add(10, value(8), value(9)),
                    add(11, value(10), value(9)),
                ],
                Terminator::Return(Some(value(11))),
            ));
            f.next_value_id = 20;
            f.next_label = 5;
            let (mut x, mut y) = (1, 2);
            for j in 0..bound {
                (x, y) = (y + j, x);
            }
            rotated_is_valid_and_computes(&mut f, x + 2 * y, &format!("bound {bound}"));
        }
    }

    /// `if (c) { loop } else { ... }` merges the loop's values in a block the
    /// exit block does not dominate, over an edge it does. A phi reads at
    /// the end of the predecessor it names, so the merge phi in the exit
    /// block serves it. (Guard B asked about the phi's own block and
    /// refused this loop.)
    #[test]
    fn rotation_serves_a_merge_behind_the_exit_block() {
        for seed in [-3, 0, 2, 3, 4, 7, 100, 150] {
            let mut f = loop_read_behind(seed, 4, Behind::MergeBehind);
            let expected = read_behind_result(seed, 4, Behind::MergeBehind);
            rotated_is_valid_and_computes(&mut f, expected, &format!("seed {seed}"));
        }
    }

    /// A block nothing reaches is no reader: the unroller leaves the body it
    /// replaced behind until the next CFG cleanup, and rotation runs before
    /// that. The loop rotates, and the dead block is left as it was (it
    /// keeps naming the header's phis, which stay defined).
    #[test]
    fn rotation_is_not_held_up_by_a_dead_reader() {
        for seed in [-3, 0, 2, 3, 4, 7] {
            let mut f = loop_read_behind(seed, 4, Behind::DeadReader);
            let dead = format!("{:?}", f.blocks[4].instructions);
            let expected = read_behind_result(seed, 4, Behind::DeadReader);
            rotated_is_valid_and_computes(&mut f, expected, &format!("seed {seed}"));
            let block = f.blocks.iter().find(|b| b.label == BlockId(5)).unwrap();
            assert_eq!(format!("{:?}", block.instructions), dead, "seed {seed}");
        }
    }

    /// Guard A and the stale exit phi are refusals, and a refusal leaves the
    /// function as it was: no phi added to the body and taken out again, no
    /// value number taken (the two guards used to stand behind step 6.6 and
    /// left `next_value_id` advanced).
    #[test]
    fn a_refusal_behind_the_loop_leaves_the_function_untouched() {
        for behind in [Behind::JumpAround, Behind::StaleExitPhi] {
            let mut f = loop_read_behind(0, 4, behind);
            let before = written(&f);
            assert!(!rotate_the_only_loop(&mut f), "{behind:?}");
            assert_eq!(written(&f), before, "{behind:?}");
            assert_eq!(f.next_value_id, 30, "{behind:?}");
        }
        // The jump around the loop is valid IR and still computes its result.
        for seed in [-3, 0, 5, 100, 150] {
            let f = loop_read_behind(seed, 4, Behind::JumpAround);
            assert_eq!(
                interpret_rotation_fixture(&f),
                read_behind_result(seed, 4, Behind::JumpAround)
            );
        }
    }

    /// The same counted loop with EXIT-ON-TRUE polarity: the CondBranch's true
    /// successor leaves the loop and the loop continues while the test is
    /// FALSE.  With an `<`/`<=` test that makes the continuation set an upper
    /// interval of the IV -- the shape the polarity guard must refuse.
    fn counted_loop_exit_on_true(ty: Ty, c: i64, op: IrCmpOp, bound: i64) -> IrFunction {
        let mut f = counted_loop(ty, 0, c, op, bound, true, 1, true, true);
        f.blocks[1].terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(4)),
            true_label: BlockId(4),
            false_label: BlockId(2),
        };
        f
    }

    /// A counted loop whose phi takes a CONSTANT on the back edge -- the loop
    /// restarts its IV from that constant every iteration, so the value the
    /// first iteration sees is the entry operand, not that constant.
    fn counted_loop_reset_on_latch(entry: i64, latch: i64) -> IrFunction {
        let mut f = counted_loop(Ty::I64, entry, 4, IrCmpOp::Slt, 4096, true, 1, true, true);
        f.blocks[1].instructions[0] = Instruction::Phi {
            dest: Value(2),
            ty: Ty::I64,
            incoming: vec![
                (
                    Operand::Const(IrConst::from_i64(entry, Ty::I64)),
                    BlockId(0),
                ),
                (
                    Operand::Const(IrConst::from_i64(latch, Ty::I64)),
                    BlockId(3),
                ),
            ],
        };
        f
    }

    /// A counted loop whose entry operand is an `Add` of two unrelated values:
    /// the seed is not a constant at all.
    fn counted_loop_entry_add(ty: Ty) -> IrFunction {
        let mut f = counted_loop(ty, 0, 4, IrCmpOp::Slt, 4096, true, 1, true, true);
        f.blocks[0].instructions.push(Instruction::BinOp {
            dest: Value(9),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(10)),
            rhs: Operand::Value(Value(11)),
            ty,
        });
        f.blocks[1].instructions[0] = Instruction::Phi {
            dest: Value(2),
            ty,
            incoming: vec![
                (Operand::Value(Value(9)), BlockId(0)),
                (Operand::Value(Value(6)), BlockId(3)),
            ],
        };
        f
    }

    /// The comparison's printed form after the pass, or `None` when the fold
    /// did not fire.
    fn folded_cmp(f: &IrFunction) -> Option<(IrCmpOp, u32, i64)> {
        let mut found = None;
        for b in f.blocks.iter() {
            for i in &b.instructions {
                if let Instruction::Cmp { lhs, op, rhs, .. } = i {
                    let (Operand::Value(iv), Operand::Const(c)) = (lhs, rhs) else {
                        return found;
                    };
                    // The folded form compares the IV phi against a literal; the
                    // unfolded form compares the affine temporary.
                    if iv.0 == 2 {
                        found = Some((*op, iv.0, c.to_i64()?));
                    }
                }
            }
        }
        found
    }

    #[test]
    fn accepts_the_canonical_shape() {
        let mut f = counted_loop(Ty::I64, 0, 4, IrCmpOp::Slt, 4096, true, 1, true, true);
        assert_eq!(fold_affine_exit_compares(&mut f), 1);
        assert_eq!(folded_cmp(&f), Some((IrCmpOp::Slt, 2, 4092)));
    }

    #[test]
    fn accepts_le_and_mirrors_the_bound() {
        // `i + 4 <= 64` folds to `i <= 60`.
        let mut f = counted_loop(Ty::I64, 0, 4, IrCmpOp::Sle, 64, true, 1, true, true);
        assert_eq!(fold_affine_exit_compares(&mut f), 1);
        assert_eq!(folded_cmp(&f), Some((IrCmpOp::Sle, 2, 60)));
    }

    #[test]
    fn accepts_a_nonzero_start_and_step() {
        let mut f = counted_loop(Ty::I64, 1, 8, IrCmpOp::Slt, 4097, true, 5, true, true);
        assert_eq!(fold_affine_exit_compares(&mut f), 1);
        assert_eq!(folded_cmp(&f), Some((IrCmpOp::Slt, 2, 4089)));
    }

    #[test]
    fn refuses_a_negative_addend() {
        let mut f = counted_loop(Ty::I64, 0, -4, IrCmpOp::Slt, 64, true, 1, true, true);
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }

    #[test]
    fn refuses_a_runtime_bound() {
        let mut f = counted_loop(Ty::I64, 0, 4, IrCmpOp::Slt, 0, false, 1, true, true);
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }

    #[test]
    fn refuses_a_runtime_seed() {
        let mut f = counted_loop(Ty::I64, 0, 4, IrCmpOp::Slt, 64, true, 1, false, true);
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }

    #[test]
    fn refuses_a_runtime_step() {
        let mut f = counted_loop(Ty::I64, 0, 4, IrCmpOp::Slt, 64, true, 0, true, false);
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }

    #[test]
    fn accepts_a_proven_unsigned_comparison() {
        let mut f = counted_loop(Ty::U32, 0, 4, IrCmpOp::Ult, 64, true, 1, true, true);
        assert_eq!(fold_affine_exit_compares(&mut f), 1);
        assert_eq!(folded_cmp(&f), Some((IrCmpOp::Ult, 2, 60)));
    }

    /// The controlling expression's OWN wrap postpones the exit: with
    /// `-fwrapv`, `i + 1073741830 < 2147483644` and `step = 2^30 - 1`,
    /// the second test computes `1073741823 + 1073741830`, which wraps
    /// negative and keeps looping past the threshold. Every certificate
    /// derived from that threshold is void, so the fold must refuse
    /// (affine_control_proof.c returned 95 instead of 97 without this).
    #[test]
    fn refuses_a_wrapping_controlling_expression() {
        let mut f = counted_loop(
            Ty::I32,
            0,
            1_073_741_830,
            IrCmpOp::Slt,
            2_147_483_644,
            true,
            1_073_741_823,
            true,
            true,
        );
        let original = format!("{f:?}");
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
        assert_eq!(format!("{f:?}"), original);
        // Control: the same shape with a small step exits before the sum
        // can wrap, and the fold still runs.
        let mut ok = counted_loop(
            Ty::I32,
            0,
            1_073_741_830,
            IrCmpOp::Slt,
            2_147_483_644,
            true,
            1,
            true,
            true,
        );
        assert_eq!(fold_affine_exit_compares(&mut ok), 1);
    }

    #[test]
    fn refuses_a_runaway_orientation() {
        // `i + 4 > 64` stays true as `i` grows: the loop does not terminate
        // through this test, so the sequence argument does not apply.
        let mut f = counted_loop(Ty::I64, 0, 4, IrCmpOp::Sgt, 64, true, 1, true, true);
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }

    #[test]
    fn refuses_a_type_top_zero_trip_shape() {
        // `top_end_start`: start = MAX - 4, `i + 4 < MAX` -- the entry test
        // already fails, so the loop never continues (`L == 0`).
        let mut f = counted_loop(
            Ty::I64,
            i64::MAX - 4,
            4,
            IrCmpOp::Slt,
            i64::MAX,
            true,
            1,
            true,
            true,
        );
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }

    #[test]
    fn accepts_the_type_top_boundary() {
        // `near_high`: start = MAX - 20, `i + 4 < MAX`; the failing test computes
        // exactly MAX, and the folded bound is MAX - 4.
        let mut f = counted_loop(
            Ty::I64,
            i64::MAX - 20,
            4,
            IrCmpOp::Slt,
            i64::MAX,
            true,
            1,
            true,
            true,
        );
        assert_eq!(fold_affine_exit_compares(&mut f), 1);
        assert_eq!(folded_cmp(&f), Some((IrCmpOp::Slt, 2, i64::MAX - 4)));
    }

    #[test]
    fn refuses_an_unrepresentable_folded_bound() {
        // start = MIN, C = 4, bound = MIN + 2: `N - C` underflows the type.
        let mut f = counted_loop(
            Ty::I64,
            i64::MIN,
            8,
            IrCmpOp::Slt,
            i64::MIN + 2,
            true,
            1,
            true,
            true,
        );
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }

    #[test]
    fn refuses_exit_on_true_with_a_less_than_test() {
        // Continuation is `!(iv + 4 < 4096)`: an upper interval, not a prefix.
        // The trip-count obligations say nothing about it, so the fold must not
        // fire however plausible the bound looks.
        let mut f = counted_loop_exit_on_true(Ty::I64, 4, IrCmpOp::Slt, 4096);
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }

    #[test]
    fn refuses_exit_on_true_with_a_greater_than_test_too() {
        // The mirror image WOULD have a prefix continuation set (`!(iv + 4 >
        // 4092)` is `iv + 4 <= 4092`), so the polarity guard lets the threshold
        // through -- and then the rewrite is still refused, because `>` with the
        // affine operand on the left is the runaway orientation the fold never
        // produces.  Two independent refusals, both pinned here.
        let mut f = counted_loop_exit_on_true(Ty::I64, 4, IrCmpOp::Sgt, 4092);
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
        // ... while the ascending form of the same loop does fold, which is what
        // makes the refusal above a choice about orientation and not about
        // polarity.
        let mut ok = counted_loop(Ty::I64, 0, 4, IrCmpOp::Sle, 4092, true, 1, true, true);
        assert_eq!(fold_affine_exit_compares(&mut ok), 1);
    }

    #[test]
    fn refuses_a_constant_on_the_back_edge() {
        let mut f = counted_loop_reset_on_latch(0, 5);
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }

    #[test]
    fn refuses_a_non_constant_entry_operand() {
        let mut f = counted_loop_entry_add(Ty::I64);
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }

    #[test]
    fn refuses_a_second_use_of_the_temporary() {
        let mut f = counted_loop(Ty::I64, 0, 4, IrCmpOp::Slt, 64, true, 1, true, true);
        // A second reader of the affine temporary: folding would leave the add
        // alive, and the rewrite is no longer local.
        f.blocks[2].instructions.push(Instruction::Copy {
            dest: Value(20),
            src: Operand::Value(Value(3)),
        });
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }
    #[test]
    fn unsigned_byte_sequences_and_mirrored_bounds_are_exact() {
        let mut checked = 0;
        for start in [0, 1, 127, 254, 255] {
            for bound in 0..=255 {
                for step in [1, 2, 7, 128] {
                    for offset in [0, 1, 8, 127, 255] {
                        for (op, mirror) in [
                            (IrCmpOp::Ult, false),
                            (IrCmpOp::Ule, false),
                            (IrCmpOp::Ugt, true),
                            (IrCmpOp::Uge, true),
                        ] {
                            let mut f = counted_loop(
                                Ty::U8,
                                start,
                                offset,
                                if mirror {
                                    if op == IrCmpOp::Ugt {
                                        IrCmpOp::Ult
                                    } else {
                                        IrCmpOp::Ule
                                    }
                                } else {
                                    op
                                },
                                bound,
                                true,
                                step,
                                true,
                                true,
                            );
                            if mirror {
                                for inst in &mut f.blocks[1].instructions {
                                    if let Instruction::Cmp {
                                        lhs, rhs, op: o, ..
                                    } = inst
                                    {
                                        std::mem::swap(lhs, rhs);
                                        *o = op;
                                    }
                                }
                            }
                            if fold_affine_exit_compares(&mut f) == 0 {
                                continue;
                            }
                            let (folded_op, _, raw) = folded_cmp(&f).unwrap();
                            let cutoff = integer_constant(IrConst::I64(raw), Ty::U8).unwrap();
                            let inclusive = matches!(op, IrCmpOp::Ule | IrCmpOp::Uge);
                            assert_eq!(
                                folded_op,
                                if inclusive {
                                    IrCmpOp::Ule
                                } else {
                                    IrCmpOp::Ult
                                }
                            );
                            let mut v = start;
                            let mut exited = false;
                            for _ in 0..=256 {
                                let sum = (v + offset) & 255; // independent modulo-256 model
                                let old = if inclusive { sum <= bound } else { sum < bound };
                                let new = if inclusive {
                                    i128::from(v) <= cutoff
                                } else {
                                    i128::from(v) < cutoff
                                };
                                assert_eq!(old, new, "{start}/{step}/{offset}/{bound} at {v}");
                                if !old {
                                    exited = true;
                                    break;
                                }
                                v = (v + step) & 255;
                            }
                            assert!(exited);
                            checked += 1;
                        }
                    }
                }
            }
        }
        assert!(checked > 1000, "nonvacuous certificate sweep: {checked}");
    }

    #[test]
    fn unsigned_full_width_bounds_are_not_signed_raw_patterns() {
        for ty in [Ty::U8, Ty::U16, Ty::U32, Ty::U64] {
            let max = (1_i128 << (ty.size() * 8)) - 1;
            let mut good = counted_loop(
                ty,
                (max - 20) as i64,
                4,
                IrCmpOp::Ult,
                (max - 1) as i64,
                true,
                1,
                true,
                true,
            );
            assert_eq!(fold_affine_exit_compares(&mut good), 1);
            let (_, _, raw) = folded_cmp(&good).unwrap();
            assert_eq!(integer_constant(IrConst::I64(raw), ty), Some(max - 5));
            let mut bad = counted_loop(
                ty,
                (max - 15) as i64,
                8,
                IrCmpOp::Ule,
                (max - 2) as i64,
                true,
                8,
                true,
                true,
            );
            let before = format!("{bad:?}");
            assert_eq!(fold_affine_exit_compares(&mut bad), 0);
            assert_eq!(format!("{bad:?}"), before);
        }
    }

    #[test]
    fn affine_fold_rejects_signedness_mismatch_and_multi_def_webs() {
        for (ty, op) in [(Ty::U32, IrCmpOp::Slt), (Ty::I32, IrCmpOp::Ult)] {
            let mut f = counted_loop(ty, 0, 8, op, 256, true, 8, true, true);
            let before = format!("{f:?}");
            assert_eq!(fold_affine_exit_compares(&mut f), 0);
            assert_eq!(format!("{f:?}"), before);
        }
        let mut f = counted_loop(Ty::U64, 0, 8, IrCmpOp::Ule, 256, true, 8, true, true);
        f.blocks[1].instructions.insert(
            2,
            Instruction::Copy {
                dest: Value(3),
                src: Operand::Const(IrConst::I64(1000)),
            },
        );
        let before = format!("{f:?}");
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
        assert_eq!(format!("{f:?}"), before);
    }

    #[test]
    fn unsigned_folding_respects_the_existing_kill_switch() {
        let mut f = counted_loop(Ty::U32, 0, 8, IrCmpOp::Ule, 256, true, 8, true, true);
        let before = format!("{f:?}");
        let old = affine_fold_enabled();
        AFFINE_FOLD_ENABLED.with(|v| v.set(false));
        let result = fold_affine_exit_compares(&mut f);
        AFFINE_FOLD_ENABLED.with(|v| v.set(old));
        assert_eq!(result, 0);
        assert_eq!(format!("{f:?}"), before);
        assert_eq!(fold_affine_exit_compares(&mut f), 1);
    }

    #[test]
    fn signed_mirrored_comparisons_retain_the_original_orientation() {
        for (ty, op) in [(Ty::I32, IrCmpOp::Slt), (Ty::I64, IrCmpOp::Sle)] {
            let mut f = counted_loop(ty, 0, 4, op, 100, true, 1, true, true);
            for inst in &mut f.blocks[1].instructions {
                if let Instruction::Cmp {
                    lhs, rhs, op: o, ..
                } = inst
                {
                    std::mem::swap(lhs, rhs);
                    *o = if op == IrCmpOp::Slt {
                        IrCmpOp::Sgt
                    } else {
                        IrCmpOp::Sge
                    };
                }
            }
            assert_eq!(fold_affine_exit_compares(&mut f), 1);
            assert_eq!(folded_cmp(&f), Some((op, 2, 96)));
            for v in -32..=100 {
                assert_eq!(op.eval_i64(v + 4, 100), op.eval_i64(v, 96));
            }
        }
    }
    #[test]
    fn optional_exit_test_does_not_bound_the_induction_sequence() {
        let mut f = counted_loop(Ty::U32, 0, 4, IrCmpOp::Ult, 64, true, 1, true, true);
        f.blocks[0].instructions.push(Instruction::ParamRef {
            dest: Value(9),
            param_idx: 0,
            ty: Ty::I32,
        });
        f.blocks[1].terminator = Terminator::Branch(BlockId(2));
        f.blocks[2].terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(9)),
            true_label: BlockId(5),
            false_label: BlockId(3),
        };
        f.blocks.push(blk(
            5,
            vec![],
            Terminator::CondBranch {
                cond: Operand::Value(Value(4)),
                true_label: BlockId(3),
                false_label: BlockId(4),
            },
        ));
        let before = format!("{f:?}");
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
        assert_eq!(format!("{f:?}"), before);
    }

    #[test]
    fn a_shared_affine_source_keeps_its_existing_value_and_bound() {
        let mut f = counted_loop(Ty::I64, 0, 4, IrCmpOp::Slt, 64, true, 1, true, true);
        f.blocks[1].instructions.insert(
            1,
            Instruction::BinOp {
                dest: Value(9),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Const(IrConst::I64(3)),
                ty: Ty::I64,
            },
        );
        if let Instruction::BinOp { lhs, .. } = &mut f.blocks[1].instructions[2] {
            *lhs = Operand::Value(Value(9));
        }
        f.blocks[2].instructions.push(Instruction::Copy {
            dest: Value(10),
            src: Operand::Value(Value(9)),
        });
        assert_eq!(fold_affine_exit_compares(&mut f), 1);
        assert!(matches!(
            f.blocks[1].instructions[3],
            Instruction::Cmp {
                op: IrCmpOp::Slt,
                lhs: Operand::Value(Value(9)),
                rhs: Operand::Const(IrConst::I64(60)),
                ..
            }
        ));
    }
    #[test]
    fn folds_proven_unsigned_header_comparisons() {
        for ty in [Ty::U8, Ty::U16, Ty::U32, Ty::U64] {
            let mut f = counted_loop(ty, 0, 4, IrCmpOp::Ult, 64, true, 1, true, true);
            assert_eq!(fold_affine_exit_compares(&mut f), 1);
            assert_eq!(folded_cmp(&f), Some((IrCmpOp::Ult, 2, 60)));
        }
        let mut f = counted_loop(Ty::U64, 0, 8, IrCmpOp::Ule, 4096, true, 8, true, true);
        assert_eq!(fold_affine_exit_compares(&mut f), 1);
        assert_eq!(folded_cmp(&f), Some((IrCmpOp::Ule, 2, 4088)));
    }

    #[test]
    fn unsigned_fold_refuses_wrap_and_unproven_entry() {
        for (start, add, bound, step) in [(-1, 4, 64, 1), (0, 100, 127, 250), (0, 4, 2, 1)] {
            let mut f = counted_loop(
                Ty::U8,
                start,
                add,
                IrCmpOp::Ult,
                bound,
                true,
                step,
                true,
                true,
            );
            assert_eq!(fold_affine_exit_compares(&mut f), 0);
        }
        let mut runtime = counted_loop(Ty::U64, 0, 8, IrCmpOp::Ule, 4096, true, 8, false, true);
        assert_eq!(fold_affine_exit_compares(&mut runtime), 0);
        let mut mixed = counted_loop(Ty::U32, 0, 4, IrCmpOp::Slt, 64, true, 1, true, true);
        assert_eq!(fold_affine_exit_compares(&mut mixed), 0);
    }

    #[test]
    fn unsigned_fold_proves_the_failing_test_does_not_wrap() {
        let mut f = counted_loop(Ty::U8, 0, 100, IrCmpOp::Ult, 127, true, 1, true, true);
        if let Instruction::BinOp { rhs, .. } = &mut f.blocks[3].instructions[0] {
            *rhs = Operand::Const(IrConst::I64(250));
        }
        // 0+100 < 127, then 250+100 wraps to 94 < 127: subtracting
        // 100 from the bound would incorrectly stop at the second test.
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }

    #[test]
    fn unsigned_fold_uses_a_dominating_nonwrapping_guard() {
        let mut f = counted_loop(Ty::U32, 0, 4, IrCmpOp::Ult, 64, true, 1, true, true);
        f.blocks[1].instructions.push(Instruction::BinOp {
            dest: Value(8),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(2)),
            rhs: Operand::Const(IrConst::I32(1)),
            ty: Ty::U32,
        });
        f.blocks[1].instructions.push(Instruction::Cmp {
            dest: Value(9),
            op: IrCmpOp::Ult,
            lhs: Operand::Value(Value(8)),
            rhs: Operand::Const(IrConst::I32(100)),
            ty: Ty::U32,
        });
        assert_eq!(fold_affine_exit_compares(&mut f), 2);
        assert!(matches!(
            &f.blocks[1].instructions[4],
            Instruction::Cmp {
                lhs: Operand::Value(Value(2)),
                ..
            }
        ));

        let mut optional = counted_loop(Ty::U32, 0, 4, IrCmpOp::Ult, 64, true, 1, true, true);
        let guard = optional.blocks[1].instructions.split_off(1);
        optional.blocks[2].instructions = guard;
        optional.blocks[1].terminator = Terminator::Branch(BlockId(2));
        optional.blocks[2].terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(4)),
            true_label: BlockId(3),
            false_label: BlockId(4),
        };
        // This body test is mandatory and dominates every backedge.
        // Upstream's stronger dominance certificate safely admits it.
        assert_eq!(fold_affine_exit_compares(&mut optional), 1);
    }

    #[test]
    fn signed_fold_requires_an_executed_nonwrapping_guard() {
        let mut optional = counted_loop(
            Ty::I32,
            0,
            2,
            IrCmpOp::Slt,
            100,
            true,
            1073741823,
            true,
            true,
        );
        let guard = optional.blocks[1].instructions.split_off(1);
        optional.blocks[2].instructions = guard;
        optional.blocks[1].instructions.push(Instruction::Cmp {
            dest: Value(10),
            op: IrCmpOp::Ne,
            lhs: Operand::Value(Value(2)),
            rhs: Operand::Const(IrConst::I32(0)),
            ty: Ty::I32,
        });
        optional.blocks[1].terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(10)),
            true_label: BlockId(2),
            false_label: BlockId(3),
        };
        optional.blocks[2].terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(4)),
            true_label: BlockId(3),
            false_label: BlockId(4),
        };
        assert_eq!(fold_affine_exit_compares(&mut optional), 0);

        // Both an independent refusal and a positive witness: retaining safe
        // body comparisons does not require weakening the controlling proof.
        for (guard_c, bound, step, expected) in
            [(1073741830, 2147483644, 1073741823, 0), (4, 64, 1, 2)]
        {
            let mut f = counted_loop(
                Ty::I32,
                0,
                guard_c,
                IrCmpOp::Slt,
                bound,
                true,
                step,
                true,
                true,
            );
            f.blocks[2].instructions.push(Instruction::BinOp {
                dest: Value(8),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Const(IrConst::I32(100)),
                ty: Ty::I32,
            });
            f.blocks[2].instructions.push(Instruction::Cmp {
                dest: Value(9),
                op: IrCmpOp::Slt,
                lhs: Operand::Value(Value(8)),
                rhs: Operand::Const(IrConst::I32(1000)),
                ty: Ty::I32,
            });
            assert_eq!(fold_affine_exit_compares(&mut f), expected);
        }
    }
}

#[cfg(test)]
mod affine_fold_orientation_tests {
    //! `canonicalise_affine_exit_cmps` folds the rotated latch's cloned exit
    //! comparison.  Its algebra is the whole content of the pass, and it had a
    //! wrong half: with the affine operand on the RIGHT it swapped the operands
    //! *and* mirrored the operator, and two negations do not cancel, so
    //! `N > iv + C` became `(N - C) < iv` -- for N=100, C=4, iv=0 the true
    //! `100 > 4` came out as the false `96 < 0`.
    //!
    //! These tests do not check the SHAPE of the rewrite; they check that it is
    //! an EQUIVALENCE, by evaluating the source predicate and the rewritten
    //! comparison over a sweep of induction-variable values, for every signed
    //! relation in both orientations.  A shape assertion would have passed on
    //! the broken version -- it produced a comparison of exactly the expected
    //! shape -- which is precisely why the algebra needs an oracle.
    use super::*;

    /// The IV value both sides are evaluated at.
    const IV: u32 = 1;
    /// The cloned `iv + C` temporary.
    const TEMP: u32 = 100;

    fn build(flipped: bool, op: IrCmpOp, c: i64, n: i64) -> Vec<Instruction> {
        let add = Instruction::BinOp {
            dest: Value(TEMP),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(IV)),
            rhs: Operand::Const(IrConst::from_i64(c, IrType::I64)),
            ty: IrType::I64,
        };
        let n_konst = Operand::Const(IrConst::from_i64(n, IrType::I64));
        let cmp = Instruction::Cmp {
            dest: Value(101),
            lhs: if flipped {
                n_konst.clone()
            } else {
                Operand::Value(Value(TEMP))
            },
            op,
            rhs: if flipped {
                Operand::Value(Value(TEMP))
            } else {
                n_konst
            },
            ty: IrType::I64,
        };
        vec![add, cmp]
    }

    /// Evaluate the (possibly rewritten) comparison at `iv`.  Only the IV and
    /// constants may appear once the fold has run; the temporary appearing means
    /// the fold did not fire.
    fn eval_folded(cmp: &Instruction, iv: i64) -> Option<bool> {
        let Instruction::Cmp { lhs, op, rhs, .. } = cmp else {
            return None;
        };
        let value = |o: &Operand| -> Option<i64> {
            match o {
                Operand::Const(c) => c.to_i64(),
                Operand::Value(v) if v.0 == IV => Some(iv),
                _ => None,
            }
        };
        Some(op.eval_i64(value(lhs)?, value(rhs)?))
    }

    /// The source predicate, evaluated directly from the C-level relation.
    fn eval_source(flipped: bool, op: IrCmpOp, c: i64, n: i64, iv: i64) -> bool {
        let temp = iv + c;
        if flipped {
            op.eval_i64(n, temp)
        } else {
            op.eval_i64(temp, n)
        }
    }

    #[test]
    fn all_signed_relations_in_both_orientations_are_equivalences() {
        let relations = [IrCmpOp::Slt, IrCmpOp::Sle, IrCmpOp::Sgt, IrCmpOp::Sge];
        let mut checked = 0usize;
        for flipped in [false, true] {
            for op in relations {
                for &c in &[0i64, 1, 4, 7] {
                    for &n in &[0i64, 1, 5, 16, 4096] {
                        for iv in [-33i64, -1, 0, 1, 2, 5, 17, 100, 4095] {
                            let mut insts = build(flipped, op, c, n);
                            let folded =
                                canonicalise_affine_exit_cmps(&mut insts, Some(Value(101)));
                            // This algebra-only oracle supplies an explicit
                            // proof token; evaluate only its nonwrapping domain.
                            // Admission of that token is tested by real loops.
                            if iv.checked_add(c).is_none() {
                                continue;
                            }
                            // The fold is allowed to refuse when `N - C` is not
                            // representable; when it fires it must be exact.
                            if folded == 0 {
                                continue;
                            }
                            let Some(after) = eval_folded(&insts[1], iv) else {
                                // Still refers to the temporary: refused.
                                continue;
                            };
                            let before = eval_source(flipped, op, c, n, iv);
                            assert_eq!(
                                before, after,
                                "fold changed the predicate: flipped={flipped} op={op:?} c={c} \
                                 n={n} iv={iv} ({before} -> {after})"
                            );
                            checked += 1;
                        }
                    }
                }
            }
        }
        assert!(
            checked > 400,
            "expected a broad sweep, checked only {checked}"
        );
    }

    #[test]
    fn a_clone_needs_a_proof_for_its_own_comparison() {
        let original = build(false, IrCmpOp::Slt, 4, 100);
        let mut missing = original.clone();
        assert_eq!(canonicalise_affine_exit_cmps(&mut missing, None), 0);
        assert_eq!(format!("{missing:?}"), format!("{original:?}"));
        let mut unrelated = original.clone();
        assert_eq!(
            canonicalise_affine_exit_cmps(&mut unrelated, Some(Value(999))),
            0
        );
        assert_eq!(format!("{unrelated:?}"), format!("{original:?}"));
    }

    #[test]
    fn the_flipped_oracle_catches_the_regression_it_was_written_for() {
        // The exact shape the audit reported: `100 > (iv + 4)` at `iv = 0` is
        // true.  The broken rewrite produced `96 < 0` (false).  Pin the value
        // pair so the oracle above cannot silently stop covering it.
        let mut insts = build(true, IrCmpOp::Sgt, 4, 100);
        assert_eq!(
            canonicalise_affine_exit_cmps(&mut insts, Some(Value(101))),
            1
        );
        let after = eval_folded(&insts[1], 0).expect("fold must leave a constant bound");
        assert!(after, "100 > (0 + 4) must stay true after folding");
        assert_eq!(eval_source(true, IrCmpOp::Sgt, 4, 100, 0), after);
    }
}

#[cfg(test)]
mod affine_value_bound_tests {
    //! `fold_loop_value_bound` is an EQUIVALENCE under a guard, and every
    //! clause of its proof has a counterexample when dropped.  The oracle is
    //! therefore behavioural: a wrapping interpreter runs the loop before and
    //! after the fold and the TRIP COUNTS must agree for every bound in a
    //! sweep that includes the type extremes — the exact values where the
    //! unsound variants (no guard, non-strict compare, step 2) diverge.
    use super::*;
    use crate::ir::reexports::BasicBlock;

    const N_PARAM: u32 = 1;
    const GUARD_CMP: u32 = 2;
    const PHI: u32 = 3;
    const INC: u32 = 4;
    const AFFINE: u32 = 5;
    const EXIT_CMP: u32 = 6;
    const RELAY_BLOCK: u32 = 9;

    struct Shape {
        ty: IrType,
        start: i64,
        step: i64,
        /// `Some(c)` chains the affine add onto the post-increment
        /// (`(phi + step) + c`); `None` uses the reassociated `phi + c_total`.
        chain_c: Option<i64>,
        c_total: i64,
        cmp_op: IrCmpOp,
        /// Mirror the exit compare to `n <op'> affine`.
        cmp_mirrored: bool,
        guard: Guard,
        relay: bool,
    }

    enum Guard {
        /// `CondBranch(K <op> n, loop, exit)`.
        TrueEdge(i64, IrCmpOp),
        /// `CondBranch(n <op> K, exit, loop)` — the loop is the FALSE edge.
        FalseEdgeMirrored(i64, IrCmpOp),
        /// Unconditional entry: nothing proven.
        None,
    }

    fn blk(label: u32, insts: Vec<Instruction>, term: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions: insts,
            terminator: term,
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        }
    }

    /// ```text
    ///   0 entry:  n = param0 ; [guard cmp] ; guard-branch (or Branch)
    ///   9 relay:  (optional) Branch -> 1
    ///   1 loop:   phi = [start, pre], [inc, 1]
    ///             inc = phi + step
    ///             affine = (inc + c) | (phi + c_total)
    ///             cmp ; CondBranch(cmp, 1, 2)   (mirrored: operands swapped)
    ///   2 exit:   Return
    /// ```
    fn fixture(s: &Shape) -> IrFunction {
        let ty = s.ty;
        let mut f = IrFunction::new("affold_vb".into(), IrType::I32, vec![], false);
        f.next_value_id = 50;
        let entry_label = if s.relay {
            BlockId(RELAY_BLOCK)
        } else {
            BlockId(0)
        };
        let loop_target = if s.relay {
            BlockId(RELAY_BLOCK)
        } else {
            BlockId(1)
        };
        let mut entry = vec![Instruction::ParamRef {
            dest: Value(N_PARAM),
            param_idx: 0,
            ty,
        }];
        let entry_term = match s.guard {
            Guard::TrueEdge(k, op) => {
                entry.push(Instruction::Cmp {
                    dest: Value(GUARD_CMP),
                    lhs: Operand::Const(IrConst::from_i64(k, ty)),
                    op,
                    rhs: Operand::Value(Value(N_PARAM)),
                    ty,
                });
                Terminator::CondBranch {
                    cond: Operand::Value(Value(GUARD_CMP)),
                    true_label: loop_target,
                    false_label: BlockId(2),
                }
            }
            Guard::FalseEdgeMirrored(k, op) => {
                entry.push(Instruction::Cmp {
                    dest: Value(GUARD_CMP),
                    lhs: Operand::Value(Value(N_PARAM)),
                    op,
                    rhs: Operand::Const(IrConst::from_i64(k, ty)),
                    ty,
                });
                Terminator::CondBranch {
                    cond: Operand::Value(Value(GUARD_CMP)),
                    true_label: BlockId(2),
                    false_label: loop_target,
                }
            }
            Guard::None => Terminator::Branch(loop_target),
        };
        let mut body = vec![
            Instruction::Phi {
                dest: Value(PHI),
                ty,
                incoming: vec![
                    (Operand::Const(IrConst::from_i64(s.start, ty)), entry_label),
                    (Operand::Value(Value(INC)), BlockId(1)),
                ],
            },
            Instruction::BinOp {
                dest: Value(INC),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(PHI)),
                rhs: Operand::Const(IrConst::from_i64(s.step, ty)),
                ty,
            },
        ];
        match s.chain_c {
            Some(c) => body.push(Instruction::BinOp {
                dest: Value(AFFINE),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(INC)),
                rhs: Operand::Const(IrConst::from_i64(c, ty)),
                ty,
            }),
            None => body.push(Instruction::BinOp {
                dest: Value(AFFINE),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(PHI)),
                rhs: Operand::Const(IrConst::from_i64(s.c_total, ty)),
                ty,
            }),
        }
        let (cmp_lhs, cmp_rhs) = if s.cmp_mirrored {
            (
                Operand::Value(Value(N_PARAM)),
                Operand::Value(Value(AFFINE)),
            )
        } else {
            (
                Operand::Value(Value(AFFINE)),
                Operand::Value(Value(N_PARAM)),
            )
        };
        body.push(Instruction::Cmp {
            dest: Value(EXIT_CMP),
            lhs: cmp_lhs,
            op: s.cmp_op,
            rhs: cmp_rhs,
            ty,
        });
        f.blocks.push(blk(0, entry, entry_term));
        f.blocks.push(blk(
            1,
            body,
            Terminator::CondBranch {
                cond: Operand::Value(Value(EXIT_CMP)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
        ));
        f.blocks.push(blk(2, vec![], Terminator::Return(None)));
        if s.relay {
            f.blocks
                .push(blk(RELAY_BLOCK, vec![], Terminator::Branch(BlockId(1))));
        }
        f
    }

    /// Wrapping interpreter.  Returns the number of loop-block executions,
    /// saturated at the bail-out bound so an (expected) non-terminating
    /// variant still yields a comparable observation.
    fn trips(f: &IrFunction, n: i128) -> u32 {
        const BAIL: u32 = 4096;
        let wrap = |v: i128, ty: IrType| -> i128 {
            let (lo, hi) = integer_type_bounds(ty).unwrap();
            let span = hi - lo + 1;
            lo + (v - lo).rem_euclid(span)
        };
        let read = |op: Operand, vals: &FxHashMap<u32, i128>| -> i128 {
            match op {
                Operand::Const(c) => c.to_i64().map(i128::from).unwrap(),
                Operand::Value(v) => vals[&v.0],
            }
        };
        let mut vals: FxHashMap<u32, i128> = FxHashMap::default();
        let mut at = f.blocks[0].label;
        let mut previous = at;
        let mut count = 0u32;
        loop {
            let block = f.blocks.iter().find(|b| b.label == at).unwrap();
            if at == BlockId(1) {
                count += 1;
                if count >= BAIL {
                    return BAIL;
                }
            }
            let phis: Vec<(u32, i128)> = block
                .instructions
                .iter()
                .filter_map(|inst| match inst {
                    Instruction::Phi { dest, incoming, .. } => {
                        let op = incoming.iter().find(|(_, l)| *l == previous).unwrap().0;
                        Some((dest.0, read(op, &vals)))
                    }
                    _ => None,
                })
                .collect();
            vals.extend(phis);
            for inst in &block.instructions {
                match inst {
                    Instruction::Phi { .. } => {}
                    Instruction::ParamRef { dest, ty, .. } => {
                        vals.insert(dest.0, wrap(n, *ty));
                    }
                    Instruction::Copy { dest, src } => {
                        let v = read(*src, &vals);
                        vals.insert(dest.0, v);
                    }
                    Instruction::BinOp {
                        dest,
                        op,
                        lhs,
                        rhs,
                        ty,
                    } => {
                        let (l, r) = (read(*lhs, &vals), read(*rhs, &vals));
                        let raw = match op {
                            IrBinOp::Add => l + r,
                            IrBinOp::Sub => l - r,
                            other => panic!("fixture does not use {other:?}"),
                        };
                        vals.insert(dest.0, wrap(raw, *ty));
                    }
                    Instruction::Cmp {
                        dest,
                        op,
                        lhs,
                        rhs,
                        ty,
                    } => {
                        // Signed relations read the wrapped (lo..=hi) image
                        // directly; unsigned types already wrap into 0..=hi.
                        let (l, r) = (wrap(read(*lhs, &vals), *ty), wrap(read(*rhs, &vals), *ty));
                        let t = match op {
                            IrCmpOp::Slt | IrCmpOp::Ult => l < r,
                            IrCmpOp::Sle | IrCmpOp::Ule => l <= r,
                            IrCmpOp::Sgt | IrCmpOp::Ugt => l > r,
                            IrCmpOp::Sge | IrCmpOp::Uge => l >= r,
                            other => panic!("fixture does not use {other:?}"),
                        };
                        vals.insert(dest.0, i128::from(t));
                    }
                    other => panic!("fixture does not use {other:?}"),
                }
            }
            previous = at;
            at = match block.terminator {
                Terminator::Branch(next) => next,
                Terminator::CondBranch {
                    cond,
                    true_label,
                    false_label,
                } => {
                    if read(cond, &vals) != 0 {
                        true_label
                    } else {
                        false_label
                    }
                }
                Terminator::Return(_) => return count,
                _ => panic!("unexpected fixture terminator"),
            };
        }
    }

    fn sweep(ty: IrType) -> Vec<i128> {
        let (lo, hi) = integer_type_bounds(ty).unwrap();
        vec![
            lo,
            lo + 3,
            lo + 4,
            lo + 5,
            lo + 6,
            -1,
            0,
            1,
            3,
            4,
            5,
            6,
            7,
            20,
            hi - 5,
            hi - 1,
            hi,
        ]
        .into_iter()
        .filter(|&v| v >= lo && v <= hi)
        .collect()
    }

    /// Fold, assert the expected fire count, and when it fired assert trip
    /// equivalence against the unfolded original over the whole sweep.
    fn check(s: &Shape, expect_fold: usize) -> IrFunction {
        let original = fixture(s);
        let mut folded = fixture(s);
        assert_eq!(
            fold_affine_exit_compares(&mut folded),
            expect_fold,
            "unexpected fold count for {:?} c_total={} step={}",
            s.cmp_op,
            s.c_total,
            s.step
        );
        for &n in &sweep(s.ty) {
            assert_eq!(
                trips(&original, n),
                trips(&folded, n),
                "trip divergence at n={n} for {:?} c_total={} step={}",
                s.cmp_op,
                s.c_total,
                s.step
            );
        }
        folded
    }

    fn base() -> Shape {
        Shape {
            ty: IrType::I64,
            start: 0,
            step: 1,
            chain_c: Some(4),
            c_total: 5,
            cmp_op: IrCmpOp::Slt,
            cmp_mirrored: false,
            guard: Guard::TrueEdge(4, IrCmpOp::Slt),
            relay: false,
        }
    }

    #[test]
    fn vb_folds_signed_postinc_chain() {
        let folded = check(&base(), 1);
        // The guard block gained the hoisted `n - 4`, and the compare now
        // reads the post-increment against it.
        let sub = folded.blocks[0]
            .instructions
            .iter()
            .find_map(|i| match i {
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Sub,
                    lhs: Operand::Value(nv),
                    rhs: Operand::Const(k),
                    ..
                } if nv.0 == N_PARAM => Some((dest.0, k.to_i64().unwrap())),
                _ => None,
            })
            .expect("hoisted sub in the guard block");
        assert_eq!(sub.1, 4, "the chain anchors on the post-increment");
        let cmp = folded.blocks[1].instructions.iter().find_map(|i| match i {
            Instruction::Cmp { lhs, op, rhs, .. } => Some((*lhs, *op, *rhs)),
            _ => None,
        });
        assert_eq!(
            cmp,
            Some((
                Operand::Value(Value(INC)),
                IrCmpOp::Slt,
                Operand::Value(Value(sub.0))
            ))
        );
    }

    #[test]
    fn vb_reassociated_chain_prefers_postinc_anchor() {
        // `phi + 5` (the shape instcombine leaves) must still anchor on the
        // post-increment: comparing the stale phi would keep two IV values
        // live across the bottom test.
        let s = Shape {
            chain_c: None,
            ..base()
        };
        let folded = check(&s, 1);
        let cmp = folded.blocks[1].instructions.iter().find_map(|i| match i {
            Instruction::Cmp { lhs, .. } => Some(*lhs),
            _ => None,
        });
        assert_eq!(cmp, Some(Operand::Value(Value(INC))));
    }

    #[test]
    fn vb_folds_unsigned() {
        let s = Shape {
            ty: IrType::U64,
            cmp_op: IrCmpOp::Ult,
            guard: Guard::TrueEdge(4, IrCmpOp::Ult),
            ..base()
        };
        check(&s, 1);
    }

    #[test]
    fn vb_folds_mirrored_compare_and_false_edge_guard() {
        // `n > affine` with entry `CondBranch(n <= 4, exit, loop)`: the fact
        // on the false edge is the negation `n > 4`.
        let s = Shape {
            cmp_op: IrCmpOp::Sgt,
            cmp_mirrored: true,
            guard: Guard::FalseEdgeMirrored(4, IrCmpOp::Sle),
            ..base()
        };
        check(&s, 1);
    }

    #[test]
    fn vb_folds_through_relay_preheader() {
        let s = Shape {
            relay: true,
            ..base()
        };
        check(&s, 1);
    }

    #[test]
    fn vb_refuses_unguarded() {
        // Near the signed minimum, `n - c` underflows to a huge positive
        // bound: the sweep (which includes `lo + 3`) would catch a fold here
        // as a trip divergence.  There must be no fold at all.
        let s = Shape {
            guard: Guard::None,
            ..base()
        };
        check(&s, 0);
    }

    #[test]
    fn vb_refuses_non_strict_compare() {
        // With `<=` and `n == type_max`, the original wrapping loop never
        // exits while the folded one would.
        let s = Shape {
            cmp_op: IrCmpOp::Sle,
            ..base()
        };
        check(&s, 0);
    }

    #[test]
    fn vb_refuses_step_two() {
        // A step of 2 overshoots: near `type_max` the original `iv + c`
        // wraps and the compares diverge.
        let s = Shape { step: 2, ..base() };
        check(&s, 0);
    }

    #[test]
    fn vb_guard_strength_boundary() {
        // `first = start + c_total = 5`.  A strict guard needs `K >= 4`:
        // `K = 4` proves `n >= 5` and folds, `K = 3` only proves `n >= 4`
        // and must refuse.
        let strong = Shape {
            guard: Guard::TrueEdge(4, IrCmpOp::Slt),
            ..base()
        };
        check(&strong, 1);
        let weak = Shape {
            guard: Guard::TrueEdge(3, IrCmpOp::Slt),
            ..base()
        };
        check(&weak, 0);
        // Non-strict facts shift by one: `4 <= n` is too weak, `5 <= n` folds.
        let weak_nonstrict = Shape {
            guard: Guard::TrueEdge(4, IrCmpOp::Sle),
            ..base()
        };
        check(&weak_nonstrict, 0);
        let strong_nonstrict = Shape {
            guard: Guard::TrueEdge(5, IrCmpOp::Sle),
            ..base()
        };
        check(&strong_nonstrict, 1);
    }

    #[test]
    fn vb_refuses_domain_mismatch() {
        // An UNSIGNED guard fact says nothing about SIGNED order: a negative
        // `n` is enormous unsigned, so `4 <u n` holds while `n - 4` still
        // underflows the signed compare.
        let s = Shape {
            guard: Guard::TrueEdge(4, IrCmpOp::Ult),
            ..base()
        };
        check(&s, 0);
    }

    #[test]
    fn vb_kill_switch_disables() {
        set_affine_fold_knobs(false, false);
        let mut f = fixture(&base());
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
        set_affine_fold_knobs(true, false);
    }
}
