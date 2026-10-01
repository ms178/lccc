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
//! v17 undo-on-bail for `next_value_id` consistency, v17 cross-phi
//! self-loop-phi latch-incoming rewrite) is KEPT — it makes the pass
//! safer when opt-in.
//!
//! PF-17 (2026-09-01, PRs #325/#327): those 15 shapes MATCH GCC on the
//! 19-name A/B. Pred-label uses `(pre_op, header_label)` (never the
//! original preheader). Guards C/D/E, bepre `latest_dep`, univsr skip of
//! rotated self-loop pointer IVs, and complete-unroll Copy-INIT are in
//! tree. Guard F rejects header work outside the cloned condition closure.
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
    // v17 undo-on-bail for next_value_id consistency, v17 cross-phi
    // self-loop-phi latch-incoming rewrite) is KEPT — it makes the pass
    // safer when opt-in. A future session will root-cause the 15 remaining
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
        if std::env::var("CCC_DEBUG_LOOP_ROTATE").is_ok() {
            eprintln!("[ROT] found {} loops", loops.len());
        }
        // Process innermost loops first (smallest body) — their rotation is
        // least likely to disturb outer-loop assumptions, and nested
        // rotation can cascade (an outer loop becomes rotatable once an
        // inner latch becomes a conditional test).
        let mut sorted: Vec<&NaturalLoop> = loops.iter().collect();
        sorted.sort_by_key(|lp| lp.body.len());
        let all_headers: crate::common::fx_hash::FxHashSet<usize> =
            loops.iter().map(|lp| lp.header).collect();
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
                if std::env::var("CCC_DEBUG_LOOP_ROTATE").is_ok() {
                    eprintln!("[ROT] nested loop header={} — bail (Guard E)", lp.header);
                }
                continue;
            }
            if try_rotate_loop(func, lp, &cfg, &all_headers) {
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
fn live_out_loop_carried_count(
    func: &IrFunction,
    lp: &NaturalLoop,
    phi_info: &[(u32, IrType, (BlockId, Operand), Operand)],
) -> usize {
    if phi_info.is_empty() {
        return 0;
    }
    let dests: FxHashSet<u32> = phi_info.iter().map(|&(d, _, _, _)| d).collect();
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

/// Try to rotate one natural loop. Returns true if the transform was applied.
fn try_rotate_loop(
    func: &mut IrFunction,
    lp: &NaturalLoop,
    cfg: &CfgAnalysis,
    all_headers: &FxHashSet<usize>,
) -> bool {
    let debug = std::env::var("CCC_DEBUG_LOOP_ROTATE").is_ok();
    // 1. Single latch that is NOT the header (a self-loop is already rotated).
    let Some(latch_idx) = lp.single_latch(&cfg.preds) else {
        if debug {
            eprintln!(
                "[ROT] no single latch (header={}, body_len={})",
                lp.header,
                lp.body.len()
            );
        }
        return false;
    };
    if latch_idx == lp.header {
        if debug {
            eprintln!(
                "[ROT] self-loop (latch==header={}, body_len={})",
                lp.header,
                lp.body.len()
            );
        }
        return false;
    }
    if debug {
        eprintln!(
            "[ROT] candidate: header={}, latch={}, body_len={}",
            lp.header,
            latch_idx,
            lp.body.len()
        );
    }

    // 2. Latch terminator must be a pure backedge `Branch(header)`.
    let header_label = func.blocks[lp.header].label;
    let latch_label = func.blocks[latch_idx].label;
    if !matches!(
        &func.blocks[latch_idx].terminator,
        Terminator::Branch(t) if *t == header_label
    ) {
        if debug {
            eprintln!("[ROT] latch not Branch(header)");
        }
        return false;
    }

    // 3. Header terminator must be CondBranch with one in-loop and one
    //    out-of-loop target. The in-loop target is the "continue"; the
    //    out-of-loop target is the "exit".
    let label_to_idx: FxHashMap<BlockId, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label, i))
        .collect();
    let (cond, continue_label, exit_label) = match &func.blocks[lp.header].terminator {
        Terminator::CondBranch {
            cond,
            true_label,
            false_label,
        } => {
            let t_in = is_in_loop(*true_label, &label_to_idx, lp);
            let f_in = is_in_loop(*false_label, &label_to_idx, lp);
            if t_in == f_in {
                if debug {
                    eprintln!("[ROT] CondBranch both-in or both-out");
                }
                return false; // both in (infinite) or both out (no body)
            }
            let c = *cond;
            if t_in {
                (c, *true_label, *false_label)
            } else {
                (c, *false_label, *true_label)
            }
        }
        _ => {
            if debug {
                eprintln!(
                    "[ROT] header not CondBranch: {:?}",
                    func.blocks[lp.header].terminator
                );
            }
            return false;
        }
    };
    if debug {
        eprintln!(
            "[ROT] CondBranch OK, continue={:?} exit={:?}",
            continue_label, exit_label
        );
    }

    // 4. The guard cond must be a Value (not a constant — those are folded
    //    by cfg_simplify and would not reach here, but fail closed).
    let cond_val = match cond {
        Operand::Value(v) => v,
        Operand::Const(_) => return false,
    };

    // 5. Collect the transitive closure of header-local instructions that
    //    feed `cond_val`. Only SSA-pure arithmetic/cmp/cast/copy/select
    //    instructions are cloned; anything with memory, calls, or atomics
    //    bails (we do not duplicate side effects).
    let header_insts = &func.blocks[lp.header].instructions;
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
        // their incoming operands — those are handled by phi_latch_val).
        if matches!(inst, Instruction::Phi { .. }) {
            continue;
        }
        if !is_cloneable_pure(inst) {
            if debug {
                eprintln!(
                    "[ROT] closure: inst not cloneable-pure: idx={} {:?}",
                    idx, inst
                );
            }
            return false; // cond setup touches memory/calls — bail
        }
        closure.push(idx);
        if closure.len() > MAX_CLOSURE {
            return false; // too deep — leave to other passes
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
        return false; // cond is loop-invariant — wouldn't terminate, bail
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
            eprintln!(
                "[ROT] header non-Phi inst outside condition closure: idx={} {:?} — bail (Guard F, un-cloned header effect)",
                idx, inst
            );
        }
        return false;
    }

    // Guard E: refuse to rotate when the cloned cond consumes a phi that
    // lives in a DIFFERENT loop header. After inlining, a remainder loop
    // `for (; i < sz; i++)` has `sz` as the IV of an outer counted loop.
    // Rotating the inner loop is locally SSA-legal, but GVN+LICM on the
    // combined CFG then freeze that outer IV to its init (simd_crc_adler
    // adler sz=2 returned the sz=1 result 00010001). Constant-trip inner
    // loops (`k < 8`) and loops whose limit is a param/invariant still
    // rotate. Nested loops whose trip count is an outer IV do not.
    {
        let mut used_outside: Vec<u32> = Vec::new();
        for &idx in &closure {
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
                if bi == lp.header || !all_headers.contains(&bi) {
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
            if debug {
                eprintln!(
                    "[ROT] cond uses foreign loop-header phi v{} — bail (Guard E, nested IV as trip limit)",
                    foreign_phi
                );
            }
            return false;
        }
    }

    // 5.5 Collect owned snapshots of the closure instructions and the header
    //     phi metadata so the immutable borrow of `func.blocks[header]` ends
    //     before we take mutable borrows of `func.blocks[latch]` below. The
    //     Rust borrow checker can't prove the header and latch are disjoint
    //     within `Vec<BasicBlock>`, so we copy out the needed data here.
    let closure_insts_owned: Vec<Instruction> =
        closure.iter().map(|&i| header_insts[i].clone()).collect();
    // (phi_dest, phi_ty, preheader_incoming, latch_incoming)
    let mut phi_info: Vec<(u32, IrType, (BlockId, Operand), Operand)> = Vec::new();
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
    let mut multi_pre_header = false;
    for inst in header_insts {
        if let Instruction::Phi { dest, incoming, ty } = inst {
            let mut pre: Option<(BlockId, Operand)> = None;
            let mut lat = None;
            for (op, lbl) in incoming {
                if *lbl == latch_label {
                    lat = Some(*op);
                } else if pre.is_some() && pre.as_ref().unwrap().0 != *lbl {
                    // Second DISTINCT outside predecessor: the single-pre
                    // self-loop phi shape cannot represent this header.
                    multi_pre_header = true;
                    break;
                } else {
                    pre = Some((*lbl, *op));
                }
            }
            if multi_pre_header {
                if debug {
                    eprintln!(
                        "[ROT] header phi v{} has >1 non-latch incoming — bail (Guard C)",
                        dest.0
                    );
                }
                return false;
            }
            if let (Some(pre), Some(lat)) = (pre, lat) {
                phi_info.push((dest.0, *ty, pre, lat));
            }
        }
    }
    // phi_latch_val and phi_pre_val — derived from phi_info, owned.
    let mut phi_latch_val: FxHashMap<u32, Operand> = FxHashMap::default();
    let mut phi_pre_val: FxHashMap<u32, (BlockId, Operand)> = FxHashMap::default();
    for &(phi_dest, _ty, pre, lat) in &phi_info {
        phi_latch_val.insert(phi_dest, lat);
        phi_pre_val.insert(phi_dest, pre);
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
    for &(phi_dest, _, _, latch_op) in &phi_info {
        let Operand::Value(v) = latch_op else {
            continue;
        };
        let Some(&idx) = def_idx.get(&v.0) else {
            continue; // defined outside the header (body / preheader)
        };
        if matches!(header_insts[idx], Instruction::Phi { .. }) {
            continue; // cross-phi latch incoming: v17 rewrite handles this
        }
        if debug {
            eprintln!(
                "[ROT] header phi v{} latch incoming v{} is defined in the header (not a phi) — bail (Guard D, while(--i) shape)",
                phi_dest, v.0
            );
        }
        return false;
    }
    // The immutable header borrow (`header_insts`) ends here under NLL —
    // its last use was the phi_info collection above. The mutable borrows
    // of `func.blocks[...]` below are disjoint from that borrow. (A prior
    // `drop(header_insts)` here was a no-op on a `&T` and tripped the
    // `dropping_references` lint under `-D warnings`.)

    // 6.5 Restrict to the single-block body+latch shape (header + one body
    //     block that is ALSO the latch). This is the canonical counted-loop
    //     form after mem2reg + cfg_simplify: `header → body_latch → header`.
    //     In this shape, rotating turns body_latch into a self-loop, so the
    //     IV phi must be MOVED from the header into body_latch (a fresh phi
    //     that receives the preheader value on entry and the computed next
    //     value on each self-loop iteration). Multi-block bodies need the new
    //     phi placed in the body ENTRY (not the latch) and body-wide use
    //     replacement — left to a future enhancement.
    let single_block_body = lp.body.len() == 2 && continue_label == latch_label;
    if !single_block_body {
        if debug {
            eprintln!(
                "[ROT] not single-block body (body_len={}, continue==latch={})",
                lp.body.len(),
                continue_label == latch_label
            );
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
    let body_block = &func.blocks[latch_idx];
    for inst in &body_block.instructions {
        match inst {
            Instruction::Call { .. } | Instruction::CallIndirect { .. } => {
                if debug {
                    eprintln!(
                        "[ROT] body has Call/CallIndirect — bail (call clobbers exit-merge values)"
                    );
                }
                return false;
            }
            Instruction::Load { volatile: true, .. }
            | Instruction::Store { volatile: true, .. } => {
                if debug {
                    eprintln!("[ROT] body has volatile mem op — bail (ordering)");
                }
                return false;
            }
            // Intrinsics (Vec*/SSE/AVX) are also rejected: the rotated
            // self-loop form's XMM phi handling doesn't match the backend's
            // vector-register home assignment, and the vectorizer has already
            // had a chance to run (rotation is post-vectorize).
            Instruction::Intrinsic { .. } => {
                if debug {
                    eprintln!(
                        "[ROT] body has Intrinsic — bail (XMM phi / vector-reg home mismatch)"
                    );
                }
                return false;
            }
            _ => {}
        }
    }

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
        let live_out = live_out_loop_carried_count(func, lp, &phi_info);
        let budget = rotation_pressure_budget();
        if live_out > budget {
            if debug {
                eprintln!(
                    "[ROT] {} loop-carried values live out of the loop, budget {} — bail (Guard G, register pressure)",
                    live_out, budget
                );
            }
            return false;
        }
    }

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
    let mut next_val = func.next_value_id;
    let mut new_loop_phis: FxHashMap<u32, u32> = FxHashMap::default(); // header_phi_dest → new_loop_phi_dest
    let mut new_phi_insts: Vec<Instruction> = Vec::with_capacity(phi_info.len());
    for &(phi_dest, phi_ty, (_pre_label, pre_op), latch_op) in &phi_info {
        let new_dest = Value(next_val);
        next_val += 1;
        new_loop_phis.insert(phi_dest, new_dest.0);
        new_phi_insts.push(Instruction::Phi {
            dest: new_dest,
            ty: phi_ty,
            // The init edge is labelled with the GUARD (`header_label`), not
            // with the original preheader (`_pre_label`). Steps 6/6.5 rewire
            // every original header predecessor onto the guard, so after
            // rotation the body's only entry edge is guard → body; the
            // preheader is no longer a predecessor of this block. Naming it
            // here produces malformed IR: phi elimination resolves the stale
            // label to the preheader's block index and places the init copy
            // on an edge that is not the live one, so the first iteration
            // reads an undefined register (SIGSEGV when the phi is an array
            // index — see tests/regression/loop_rotate_stale_phi_pred.c).
            // `pre_op` itself is unchanged and still dominates: it is defined
            // in the preheader, which dominates the guard and hence the body.
            // This matches the exit-phi construction below, which already
            // labels the guard-exit incoming with `header_label`.
            incoming: vec![(pre_op, header_label), (latch_op, latch_label)],
        });
    }
    // Insert the new phis at the TOP of body_latch (phis must precede all
    // other instructions in a block).
    let latch_block = &mut func.blocks[latch_idx];
    let mut new_body_insts: Vec<Instruction> = new_phi_insts;
    new_body_insts.extend(latch_block.instructions.drain(..));
    latch_block.instructions = new_body_insts;

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
    let latch_block = &mut func.blocks[latch_idx];
    let n_new = phi_info.len();
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
    let latch_block = &mut func.blocks[latch_idx];
    let n_new_phis = phi_info.len();
    for inst in latch_block.instructions.iter_mut().skip(n_new_phis) {
        for (&old_phi, &new_phi) in &new_loop_phis {
            let repl = Operand::Value(Value(new_phi));
            replace_operand_value(inst, Value(old_phi), repl.clone());
        }
    }

    // 7. Clone the closure instructions to the latch, allocating fresh dest
    //    IDs and rewriting: cloned refs → new dests, phi refs → latch values.
    //
    //    But FIRST (step 6.7): the header phis are used OUTSIDE the loop
    //    (e.g. the accumulator `s` is read after the loop to return the
    //    sum). After rotation, the header phi's latch incoming is gone
    //    (step 10), so the header phi collapses to the preheader value
    //    — losing the accumulated result. The real final value lives in
    //    the new self-loop phi `s_loop` (in body_latch), reached via the
    //    test-exit edge. So for each header phi with external uses, we
    //    create a merge phi in the exit block: `s_final = phi[header:
    //    v_pre, body_latch: s_loop]` and rewrite external uses to it.
    let exit_idx = label_to_idx.get(&exit_label).copied().unwrap_or(usize::MAX);
    // v16 Guard A: the exit block must have exactly ONE predecessor (the
    // header's guard-exit edge). After rotation the latch's test-exit edge
    // adds a SECOND predecessor, so the exit-merge-phi (which has exactly 2
    // incomings: (pre_op, header) and (latch_op, latch)) is correct ONLY when
    // no third block branches to exit. A third predecessor would leave the
    // merge-phi missing an incoming — the classic
    // "header-phi-escapes-through-non-Return-terminator" miscompile class,
    // where the phi's value is read on an edge the phi does not cover.
    // This also subsumes the "multi-exit" class: any loop whose body or
    // header has a second edge to exit (or to a block that branches to
    // exit) is rejected here.
    if exit_idx != usize::MAX {
        let exit_preds = cfg.preds.row(exit_idx);
        if exit_preds.len() != 1 || exit_preds[0] as usize != lp.header {
            if debug {
                eprintln!(
                    "[ROT] exit block has {} predecessors (expected 1 = header); bailing",
                    exit_preds.len()
                );
            }
            // v17 fix: undo step 6.6's state changes before bailing. Step
            // 6.6 added `n_new_phis` new self-loop phis at the top of
            // body_latch and rewrote body_latch's uses of the header phis
            // to the new self-loop phis. Without undoing, the new phi IDs
            // (allocated from `next_val`, which is bumped past
            // `func.next_value_id`) exceed the cached `next_value_id`
            // watermark, so the next pass (bit_idioms) sizes its defs vec
            // from `max_value_id() == next_value_id - 1` and panics with
            // index-out-of-bounds (sieve/expat at -O2). Undoing restores
            // the IR to its pre-6.6 state (no orphaned IDs referenced)
            // and keeps `next_value_id` consistent with the IR's actual
            // content. The undo is safe here because Guard A is NOT
            // inside any `func.blocks.iter()` loop (no borrow conflict).
            let latch_block = &mut func.blocks[latch_idx];
            latch_block.instructions.drain(..n_new_phis);
            let latch_block = &mut func.blocks[latch_idx];
            for inst in latch_block.instructions.iter_mut() {
                for (&old_phi, &new_phi) in &new_loop_phis {
                    let repl = Operand::Value(Value(old_phi));
                    replace_operand_value(inst, Value(new_phi), repl.clone());
                }
            }
            // Defensive: bump the watermark past the now-unused IDs so
            // any future pass that reads `next_value_id` sees a value
            // that bounds all live instructions (the undo removed all
            // references to the new IDs, so the original watermark is
            // also correct; this just guards against a missed reference).
            func.next_value_id = next_val;
            return false;
        }
        // Collect (header_phi_dest, new_loop_phi_dest, preheader_operand,
        // latch_operand) for phis that have at least one use outside the
        // loop body. `latch_operand` is the value the body computed THIS
        // iteration (the original header phi's latch incoming — e.g. the
        // post-add accumulator `s_new = s + a[i]`, or the post-increment
        // IV `i_next = i + 1`). On the test-exit edge the body has already
        // computed this value but it has NOT been written back to the new
        // self-loop phi yet (that only happens on the NEXT iteration's
        // entry via the backedge), so the exit-merge-phi must read
        // `latch_operand`, NOT the self-loop phi (which still holds the
        // start-of-iteration value).
        //
        // v16 Guard B: every external use of a header phi must be in a
        // block DOMINATED by the exit block. The exit-merge-phi is defined
        // at the top of the exit block; it dominates only exit and exit's
        // dominator-tree descendants. A use in a block NOT dominated by
        // exit (reachable via a path that bypasses exit) would read the
        // merge-phi before it is defined — use-before-def, the
        // "missing-downstream-use" miscompile class. Bail conservatively
        // rather than risk an unverified rewrite.
        let mut external_users: Vec<(u32, u32, Operand, Operand)> = Vec::new();
        for &(phi_dest, _ty, (_pre_lbl, pre_op), latch_op) in &phi_info {
            let new_loop = *new_loop_phis
                .get(&phi_dest)
                .expect("new_loop_phis has an entry for every header phi");
            // Scan all blocks outside the loop for uses of phi_dest.
            // Includes BOTH instructions and the terminator (the loop
            // phi is often returned, e.g. `Return(s)` reads the
            // accumulator phi — missing the terminator use would leave
            // the return reading the guard's preheader value (0) instead
            // of the accumulated result).
            let mut found = false;
            // v17 fix: Guard B previously did `return false;` directly
            // from inside the `for (bi, block) in func.blocks.iter()` loop
            // below, which (a) held an immutable borrow of `func.blocks`
            // for the loop's duration, blocking the mutable borrow needed
            // to undo step 6.6, and (b) left step 6.6's new self-loop phis
            // orphaned in body_latch (their IDs exceeded the cached
            // `next_value_id` watermark, causing bit_idioms to panic with
            // index-out-of-bounds). Now we collect the failing block
            // index into `guard_b_fail` and break the loop, then undo
            // step 6.6 AFTER the immutable borrow is released.
            let mut guard_b_fail: Option<usize> = None;
            'outer: for (bi, block) in func.blocks.iter().enumerate() {
                if bi == lp.header || lp.body.contains(&bi) {
                    continue;
                }
                let mut used_here = false;
                for inst in &block.instructions {
                    inst.for_each_used_value(|v| {
                        if v == phi_dest {
                            used_here = true;
                        }
                    });
                    if used_here {
                        break;
                    }
                }
                if !used_here {
                    // Check the terminator too (Return, CondBranch, Switch, etc.).
                    block.terminator.for_each_used_value(|v| {
                        if v == phi_dest {
                            used_here = true;
                        }
                    });
                }
                if used_here {
                    // v16 Guard B: external use must be dominated by exit.
                    if exit_idx != usize::MAX && !is_dominated_by(bi, exit_idx, cfg) {
                        if debug {
                            eprintln!(
                                "[ROT] header phi {} has external use in block {} \
                                 not dominated by exit {} — bailing",
                                phi_dest, bi, exit_idx
                            );
                        }
                        guard_b_fail = Some(bi);
                        break 'outer;
                    }
                    found = true;
                }
            }
            if let Some(bi) = guard_b_fail {
                // v17 fix: undo step 6.6's state changes before bailing.
                // See the Guard A path above for the full rationale.
                // The immutable borrow of `func.blocks` from the loop
                // above has been released (the loop ended via `break`),
                // so we can mutably borrow `func.blocks[latch_idx]` here.
                // `bi` is preserved for the debug eprintln above; we keep
                // it in scope via the `if let Some(bi)` pattern.
                let latch_block = &mut func.blocks[latch_idx];
                latch_block.instructions.drain(..n_new_phis);
                let latch_block = &mut func.blocks[latch_idx];
                for inst in latch_block.instructions.iter_mut() {
                    for (&old_phi, &new_phi) in &new_loop_phis {
                        let repl = Operand::Value(Value(old_phi));
                        replace_operand_value(inst, Value(new_phi), repl.clone());
                    }
                }
                func.next_value_id = next_val;
                let _ = bi; // bi was used in the debug eprintln above
                return false;
            }
            if found {
                external_users.push((phi_dest, new_loop, pre_op, latch_op));
            }
        }
        // Create the merge phis at the top of the exit block.
        let mut exit_merge_map: FxHashMap<u32, u32> = FxHashMap::default();
        let mut exit_phi_insts: Vec<Instruction> = Vec::with_capacity(external_users.len());
        for &(phi_dest, _new_loop, pre_op, latch_op) in &external_users {
            let nd = Value(next_val);
            next_val += 1;
            exit_merge_map.insert(phi_dest, nd.0);
            let ty = phi_info
                .iter()
                .find(|&&(pd, _, _, _)| pd == phi_dest)
                .map(|&(_, ty, _, _)| ty)
                .expect("phi_info has an entry for every header phi");
            // The test-exit incoming is `latch_op` (the value the body
            // computed this iteration, e.g. `s_new = s + a[i]`), NOT the
            // new self-loop phi `Value(new_loop)`. The self-loop phi holds
            // the START-of-iteration value at the CondBranch point (its
            // backedge writeback only fires on the next iteration's
            // entry); reading it on the exit edge would lose the final
            // iteration's contribution (off-by-one accumulator).
            //
            // Exception: when latch_op IS another header phi (cross-phi
            // swap: `a_next = b`), that header phi collapses to its
            // preheader value after step 10. The value we want on the
            // test-exit edge is the corresponding new self-loop phi
            // (start-of-this-iteration of the sibling), which is what
            // rewrite_header_phi_operand substitutes.
            let latch_for_exit = rewrite_header_phi_operand(latch_op, &new_loop_phis);
            exit_phi_insts.push(Instruction::Phi {
                dest: nd,
                ty,
                incoming: vec![
                    (pre_op, header_label),        // guard-exit path: preheader value (0-trip)
                    (latch_for_exit, latch_label), // test-exit path: post-iteration value
                ],
            });
        }
        if !exit_phi_insts.is_empty() {
            let exit_block = &mut func.blocks[exit_idx];
            let mut new_exit_insts: Vec<Instruction> = exit_phi_insts;
            new_exit_insts.extend(exit_block.instructions.drain(..));
            exit_block.instructions = new_exit_insts;
            // Rewrite external uses of the header phis to the merge phis.
            // We must skip the new merge phis themselves (they're at the top
            // of the exit block and reference v_pre / s_loop, NOT the header
            // phi dest). We also skip the loop body (uses there were already
            // rewritten to s_loop in step 6.6) and the header (the header
            // phi is still live there until step 10 strips its latch edge —
            // but the header's own cond uses will be cloned, not the original).
            let n_exit_phis = external_users.len();
            for (bi, block) in func.blocks.iter_mut().enumerate() {
                if bi == lp.header || lp.body.contains(&bi) || bi == exit_idx {
                    if bi == exit_idx {
                        // Rewrite exit-block instructions (skip the new phis at top).
                        for inst in block.instructions.iter_mut().skip(n_exit_phis) {
                            for (&old_phi, &new_phi) in &exit_merge_map {
                                let repl = Operand::Value(Value(new_phi));
                                replace_operand_value(inst, Value(old_phi), repl.clone());
                            }
                        }
                        // Also rewrite the exit block's terminator.
                        for (&old_phi, &new_phi) in &exit_merge_map {
                            let repl = Operand::Value(Value(new_phi));
                            replace_terminator_value(
                                &mut block.terminator,
                                Value(old_phi),
                                repl.clone(),
                            );
                        }
                    }
                    continue;
                }
                for inst in &mut block.instructions {
                    for (&old_phi, &new_phi) in &exit_merge_map {
                        let repl = Operand::Value(Value(new_phi));
                        replace_operand_value(inst, Value(old_phi), repl.clone());
                    }
                }
                for (&old_phi, &new_phi) in &exit_merge_map {
                    let repl = Operand::Value(Value(new_phi));
                    replace_terminator_value(&mut block.terminator, Value(old_phi), repl.clone());
                }
            }
        }
    }

    // 7. Clone the closure instructions to the latch, allocating fresh dest
    //    IDs and rewriting: cloned refs → new dests, phi refs → latch values.
    //    If a header phi's latch incoming is itself a header phi (cross-phi),
    //    rewrite it to the new self-loop phi — the old header phi is about
    //    to collapse to its preheader value in step 10.
    for latch_op in phi_latch_val.values_mut() {
        *latch_op = rewrite_header_phi_operand(*latch_op, &new_loop_phis);
    }
    let mut clone_map: FxHashMap<u32, u32> = FxHashMap::default();
    let mut cloned_insts: Vec<Instruction> = Vec::with_capacity(closure_insts_owned.len());
    for inst in &closure_insts_owned {
        let new_dest_opt = if let Some(d) = inst.dest() {
            let nd = Value(next_val);
            next_val += 1;
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
        for (&phi_id, latch_op) in &phi_latch_val {
            replace_operand_value(&mut cloned, Value(phi_id), (latch_op).clone());
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
        canonicalise_affine_exit_cmps(&mut cloned_insts)
    } else {
        0
    };
    if debug && affine_folds > 0 {
        eprintln!("[ROT] affine exit-compare folds: {affine_folds}");
    }

    // The cloned cond value (new ID) is the latch's new CondBranch cond.
    let new_cond = Operand::Value(Value(*clone_map.get(&cond_val.0).expect(
        "cond_val must be in clone_map (it was visited in the closure and has a dest)",
    )));

    // 8. Insert cloned instructions at the END of the latch (before the
    //    terminator, which we replace below). The latch's own instructions
    //    (e.g., the IV increment `i_next = i + 1`) must run BEFORE the test.
    let latch_block = &mut func.blocks[latch_idx];
    latch_block.instructions.extend(cloned_insts);

    // 9. Replace the latch's `Branch(header)` with a conditional test that
    //    branches to `continue_label` (the body) when the cloned cond is
    //    true, and to `exit_label` when false. Same polarity as the header
    //    guard: true → continue, false → exit.
    latch_block.terminator = Terminator::CondBranch {
        cond: new_cond,
        true_label: continue_label,
        false_label: exit_label,
    };

    // 10. The latch no longer branches to the header — its only successor is
    //     now `continue_label` (self or body) or `exit_label`. Remove the
    //     stale `(op, latch_label)` incoming from every phi in the header,
    //     because the latch→header backedge is gone. cfg_simplify then
    //     collapses the now-single-incoming phi to the preheader value.
    for inst in &mut func.blocks[lp.header].instructions {
        if let Instruction::Phi { incoming, .. } = inst {
            incoming.retain(|(_, lbl)| *lbl != latch_label);
        }
    }

    // 11. Advance the watermark so subsequent passes see the new IDs.
    func.next_value_id = next_val;

    if debug {
        eprintln!(
            "[ROT] SUCCESS: rotated header={} latch={}",
            lp.header, latch_idx
        );
    }
    true
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

/// Returns true if `block_idx` is dominated by `dom_idx` — i.e. every path
/// from the function entry to `block_idx` passes through `dom_idx`. Walks the
/// `idom` chain: `dom_idx` must be an ancestor of `block_idx` in the dominator
/// tree. `block_idx == dom_idx` returns true (a block dominates itself).
///
/// Used by the v16 Guard B in `try_rotate_loop`: an external use of a header
/// phi is only safe to rewrite to the exit-merge-phi if the use's block is
/// dominated by the exit block (where the merge-phi is defined). A use in a
/// block reachable via a path that bypasses exit would be a use-before-def.
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

/// Upper bound of a SIGNED integer type, or `None` for every type the affine
/// fold below must not touch (unsigned widths, pointers, floats).
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
/// * signed comparisons only, on signed integer types:
///   `Slt(iv + C, N) == Slt(iv, N - C)` holds only for the signed order (the
///   unsigned order has no such freedom), and only when neither side
///   overflows -- `iv + C` overflowing is signed-overflow UB in the C source
///   (the same licence GCC's own `cmpq %rsi, %rax` fold takes);
/// * the folded bound must be REPRESENTABLE in the comparison's type
///   (`checked_sub` + `signed_type_bounds`), so no wrap is ever introduced;
/// * constants may appear on either side of the `add` and the affine operand
///   may be on either side of the compare (`N > i + C` folds too).  The
///   comparison OPERATOR is preserved, never mirrored: mirroring the operand
///   side requires negating the relation as well, and taking one without the
///   other is exactly the double-negation bug the orientation oracle pins
///   (`N op (i + C) == (N - C) op i`, operator unchanged).
fn canonicalise_affine_exit_cmps(cloned: &mut [Instruction]) -> usize {
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
            lhs, op, rhs, ty, ..
        } = inst
        else {
            continue;
        };
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

/// Predicate: is this instruction safe to clone into the latch?
///
/// Safe = SSA-pure, no memory side effects, no calls, no atomics, no
/// intrinsics, no alloca. The allow-list is arithmetic/logic/cmp/cast/
/// copy/select/gep — the building blocks of loop guard conditions.
///
/// NOTE: Phi is deliberately EXCLUDED. Header phis are NOT cloned into
/// the latch — they are REWRITTEN to their latch-edge incoming values
/// (step 7's `replace_operand_value` via `phi_latch_val`). If a Phi
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

fn affine_fold_enabled() -> bool {
    AFFINE_FOLD_ENABLED.with(|cell| cell.get())
}

fn affine_fold_debug() -> bool {
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
        total += fold_loop(func, lp);
    }
    if total > 0 && affine_fold_debug() {
        eprintln!(
            "[AFFOLD] {}: affine exit-compare folds: {}",
            func.name, total
        );
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
/// The phi's incoming edges are not classified by predecessor: a do-while
/// self-loop has the header as its own entry predecessor, and the "entry"
/// incoming may be the second slot, so the seed is the incoming operand that is
/// a constant and the step is the incoming value that is a `phi + Const` add.
/// Requiring BOTH (rather than "the non-latch incoming is a constant") is what
/// makes `refuse_rt_start` and `refuse_rt_step` refusals instead of guesses.
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
            if signed_type_bounds(*ty).is_none() {
                continue; // unsigned IVs are refused by the fold itself
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
            (false, Operand::Const(c)) => seed = Some(c.to_i64()? as i128),
            // Entry edge, constant through a `Copy` defined outside the body.
            (false, Operand::Value(v)) => {
                seed = Some(const_through_copy_outside(func, v.0, body_set)?)
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
                            if p.0 == dest.0 && c.to_i64().is_some() =>
                        {
                            c.to_i64()
                        }
                        _ => None,
                    },
                    _ => None,
                })? as i128)
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
) -> Option<i128> {
    func.blocks.iter().enumerate().find_map(|(bi, b)| {
        if body_set.contains(&bi) {
            return None;
        }
        b.instructions.iter().find_map(|i| match i {
            Instruction::Copy { dest, src } if dest.0 == value => match src {
                Operand::Const(c) => c.to_i64().map(|v| v as i128),
                _ => None,
            },
            _ => None,
        })
    })
}

/// Constant `C` when `value` is defined as `iv + C` (either operand order) in
/// `func`, for the induction variable `iv`.  `None` for anything else --
/// including `iv` itself, which callers handle as the `C == 0` case.
fn affine_offset_of(func: &IrFunction, iv: u32, value: u32, ty: IrType) -> Option<i128> {
    func.blocks
        .iter()
        .find_map(|b| {
            b.instructions.iter().find_map(|i| match i {
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs,
                    rhs,
                    ty: add_ty,
                } if dest.0 == value && *add_ty == ty => match (lhs, rhs) {
                    (Operand::Value(p), Operand::Const(c)) if p.0 == iv => c.to_i64(),
                    (Operand::Const(c), Operand::Value(p)) if p.0 == iv => c.to_i64(),
                    _ => None,
                },
                _ => None,
            })
        })
        .map(|c| c as i128)
}

/// The exclusive upper bound `T` every test of `iv` in this loop respects:
/// the loop runs only while `iv <s T`, in the folded domain.  `None` when the
/// loop's exit is not a single branch on a constant-bounded comparison of
/// `iv` -- a runtime bound (`refuse_rt_bound`), several exits, or an exit that
/// tests something else entirely gives the proof nothing to stand on.
fn loop_exit_threshold(
    func: &IrFunction,
    lp: &NaturalLoop,
    iv: u32,
    iv_ty: IrType,
) -> Option<i128> {
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
    let (_, term) = exiting[0];
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
        (l, Operand::Const(bc)) if iv_side(l).is_some() => (bc.to_i64()?, iv_side(l)?, *op),
        // `N <op> iv + C`: mirror so the affine side is on the left.
        (Operand::Const(bc), r) if iv_side(r).is_some() => {
            let mirrored = match op {
                IrCmpOp::Slt => IrCmpOp::Sgt,
                IrCmpOp::Sle => IrCmpOp::Sge,
                IrCmpOp::Sgt => IrCmpOp::Slt,
                IrCmpOp::Sge => IrCmpOp::Sle,
                _ => return None,
            };
            (bc.to_i64()?, iv_side(r)?, mirrored)
        }
        _ => return None,
    };
    // The two polarities that admit a prefix continuation set: `op` is the
    // CONTINUATION test.  Anything else is the upper-interval case the polarity
    // comment above refuses.
    if continue_on_true {
        if !matches!(op, IrCmpOp::Slt | IrCmpOp::Sle) {
            return None;
        }
    } else if !matches!(op, IrCmpOp::Sgt | IrCmpOp::Sge) {
        return None;
    }
    // Failing test of `iv + c <op> bound`, in terms of `iv`: the exclusive
    // upper bound of the values for which the loop continues.
    let bound = bound_c as i128;
    let t = match op {
        IrCmpOp::Slt => bound,
        IrCmpOp::Sle => bound + 1,
        IrCmpOp::Sgt => bound + 1,
        IrCmpOp::Sge => bound,
        _ => return None,
    } - c;
    let (lo, hi) = signed_type_bounds(iv_ty)?;
    (t >= lo as i128 && t <= hi as i128).then_some(t)
}

/// Fold every eligible affine comparison in one loop.  Returns the number of
/// comparisons rewritten.
fn fold_loop(func: &mut IrFunction, lp: &NaturalLoop) -> usize {
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
        if let Some(t) = loop_exit_threshold(func, lp, shape.iv, shape.ty) {
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
    let (lo, hi) = signed_type_bounds(iv_shape.ty)?;
    let (lo, hi) = (lo as i128, hi as i128);
    let Instruction::Cmp {
        lhs, op, rhs, ty, ..
    } = &func.blocks[block_idx].instructions[inst_idx]
    else {
        return None;
    };
    if *ty != iv_shape.ty {
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
        (Some(c), _, Operand::Const(b), o) if matches!(o, IrCmpOp::Slt | IrCmpOp::Sle) => {
            (c, b.to_i64()? as i128, *o)
        }
        (None, Operand::Const(b), _, o) if affine_r.is_some() => {
            let mirrored = match o {
                IrCmpOp::Sgt => IrCmpOp::Slt,
                IrCmpOp::Sge => IrCmpOp::Sle,
                _ => return None,
            };
            (affine_r?, b.to_i64()? as i128, mirrored)
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
    let new_bound_c = IrConst::from_i64(new_bound as i64, iv_shape.ty);
    Some((
        op,
        Operand::Value(Value(iv_shape.iv)),
        Operand::Const(new_bound_c),
    ))
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
    fn refuses_an_unsigned_comparison() {
        let mut f = counted_loop(Ty::U32, 0, 4, IrCmpOp::Ult, 64, true, 1, true, true);
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
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
                            let folded = canonicalise_affine_exit_cmps(&mut insts);
                            // `iv + C` must not overflow for the source to be
                            // defined; skip the wrapping cases.
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
    fn the_flipped_oracle_catches_the_regression_it_was_written_for() {
        // The exact shape the audit reported: `100 > (iv + 4)` at `iv = 0` is
        // true.  The broken rewrite produced `96 < 0` (false).  Pin the value
        // pair so the oracle above cannot silently stop covering it.
        let mut insts = build(true, IrCmpOp::Sgt, 4, 100);
        assert_eq!(canonicalise_affine_exit_cmps(&mut insts), 1);
        let after = eval_folded(&insts[1], 0).expect("fold must leave a constant bound");
        assert!(after, "100 > (0 + 4) must stay true after folding");
        assert_eq!(eval_source(true, IrCmpOp::Sgt, 4, 100, 0), after);
    }
}
