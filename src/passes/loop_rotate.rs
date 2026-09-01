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
//! tree. Rotation STAYS opt-in (`CCC_LOOP_ROTATE=1`) until the full 474
//! corpus is green. Kill-switch `CCC_NO_LOOP_ROTATE=1` wins.
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
use crate::ir::reexports::{BlockId, Instruction, IrConst, IrFunction, Operand, Terminator, Value};
use crate::ir::analysis::CfgAnalysis;
use crate::passes::loop_analysis::{find_natural_loops, merge_loops_by_header, NaturalLoop};
use crate::passes::loop_unroll::{rename_inst_dest, subst_value_in_terminator, subst_value_with_operand};
use crate::passes::tail_call_elim::replace_values_in_inst;

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
            let nested = loops.iter().any(|outer| {
                outer.header != lp.header && outer.body.contains(&lp.header)
            });
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
        if debug { eprintln!("[ROT] no single latch (header={}, body_len={})", lp.header, lp.body.len()); }
        return false;
    };
    if latch_idx == lp.header {
        if debug { eprintln!("[ROT] self-loop (latch==header={}, body_len={})", lp.header, lp.body.len()); }
        return false;
    }
    if debug { eprintln!("[ROT] candidate: header={}, latch={}, body_len={}", lp.header, latch_idx, lp.body.len()); }

    // 2. Latch terminator must be a pure backedge `Branch(header)`.
    let header_label = func.blocks[lp.header].label;
    let latch_label = func.blocks[latch_idx].label;
    if !matches!(
        &func.blocks[latch_idx].terminator,
        Terminator::Branch(t) if *t == header_label
    ) {
        if debug { eprintln!("[ROT] latch not Branch(header)"); }
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
                if debug { eprintln!("[ROT] CondBranch both-in or both-out"); }
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
            if debug { eprintln!("[ROT] header not CondBranch: {:?}", func.blocks[lp.header].terminator); }
            return false;
        }
    };
    if debug { eprintln!("[ROT] CondBranch OK, continue={:?} exit={:?}", continue_label, exit_label); }

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
            if debug { eprintln!("[ROT] closure: inst not cloneable-pure: idx={} {:?}", idx, inst); }
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
        if let Instruction::Phi {
            dest,
            incoming,
            ty,
        } = inst
        {
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
        if debug { eprintln!("[ROT] not single-block body (body_len={}, continue==latch={})", lp.body.len(), continue_label == latch_label); }
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
                if debug { eprintln!("[ROT] body has Call/CallIndirect — bail (call clobbers exit-merge values)"); }
                return false;
            }
            Instruction::Load { volatile: true, .. } | Instruction::Store { volatile: true, .. } => {
                if debug { eprintln!("[ROT] body has volatile mem op — bail (ordering)"); }
                return false;
            }
            // Intrinsics (Vec*/SSE/AVX) are also rejected: the rotated
            // self-loop form's XMM phi handling doesn't match the backend's
            // vector-register home assignment, and the vectorizer has already
            // had a chance to run (rotation is post-vectorize).
            Instruction::Intrinsic { .. } => {
                if debug { eprintln!("[ROT] body has Intrinsic — bail (XMM phi / vector-reg home mismatch)"); }
                return false;
            }
            _ => {}
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
            incoming: vec![
                (pre_op, header_label),
                (latch_op, latch_label),
            ],
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
            subst_value_with_operand(inst, old_phi, &repl);
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
    let exit_idx = label_to_idx
        .get(&exit_label)
        .copied()
        .unwrap_or(usize::MAX);
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
                    subst_value_with_operand(inst, new_phi, &repl);
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
                    if exit_idx != usize::MAX
                        && !is_dominated_by(bi, exit_idx, cfg)
                    {
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
                        subst_value_with_operand(inst, new_phi, &repl);
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
                    (pre_op, header_label),          // guard-exit path: preheader value (0-trip)
                    (latch_for_exit, latch_label),   // test-exit path: post-iteration value
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
                                subst_value_with_operand(inst, old_phi, &repl);
                            }
                        }
                        // Also rewrite the exit block's terminator.
                        for (&old_phi, &new_phi) in &exit_merge_map {
                            let repl = Operand::Value(Value(new_phi));
                            subst_value_in_terminator(&mut block.terminator, old_phi, &repl);
                        }
                    }
                    continue;
                }
                for inst in &mut block.instructions {
                    for (&old_phi, &new_phi) in &exit_merge_map {
                        let repl = Operand::Value(Value(new_phi));
                        subst_value_with_operand(inst, old_phi, &repl);
                    }
                }
                for (&old_phi, &new_phi) in &exit_merge_map {
                    let repl = Operand::Value(Value(new_phi));
                    subst_value_in_terminator(&mut block.terminator, old_phi, &repl);
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
        // cloned). `replace_values_in_inst` only touches operands, not dest.
        replace_values_in_inst(&mut cloned, &clone_map);
        // Second: rewrite phi references → latch-edge incoming operands.
        // Phi references are NOT in clone_map (phis are not cloned), so
        // `replace_values_in_inst` left them untouched.
        for (&phi_id, latch_op) in &phi_latch_val {
            subst_value_with_operand(&mut cloned, phi_id, latch_op);
        }
        // Third: rename the dest to the fresh ID.
        if new_dest_opt.is_some() {
            rename_inst_dest(&mut cloned, &clone_map);
        }
        cloned_insts.push(cloned);
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

    if debug { eprintln!("[ROT] SUCCESS: rotated header={} latch={}", lp.header, latch_idx); }
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

/// Predicate: is this instruction safe to clone into the latch?
///
/// Safe = SSA-pure, no memory side effects, no calls, no atomics, no
/// intrinsics, no alloca. The allow-list is arithmetic/logic/cmp/cast/
/// copy/select/gep — the building blocks of loop guard conditions.
///
/// NOTE: Phi is deliberately EXCLUDED. Header phis are NOT cloned into
/// the latch — they are REWRITTEN to their latch-edge incoming values
/// (step 7's `subst_value_with_operand` via `phi_latch_val`). If a Phi
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
