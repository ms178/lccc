//! Bool-phi merge-diamond branch threading.
//!
//! A value-context short-circuit (`int ok = a && b;`), an explicit
//! `if (a) ok = 1; else ok = cmp;`, and the C99 `_Bool` merges of
//! vsprintf/printf all lower to the same IR shape:
//!
//!   Pi:  ...                  ; Branch(Bmerge)     // unconditional pred
//!   Pj:  ...                  ; Branch(Bmerge)
//!   Bmerge: %p = phi([c_i, Pi], [v_j, Pj], ...)
//!           CondBranch(%p, Bt, Bf)
//!
//! With if-conversion disabled on the -m16 size profile this diamond survives
//! the whole pipeline, and phi elimination materializes it as one arm store
//! per predecessor plus a reload-and-test in the merge:
//!
//!   Pi:  movl $1, 44(%esp)    ; jmp .Lmerge        // 11 bytes
//!   Pj:  movl %eax, 44(%esp)  ; jmp .Lmerge
//!   .Lmerge: cmpl $0, 44(%esp); je .Lf             // 7 bytes
//!
//! GCC never emits that form because its jump threader folds the branch into
//! every predecessor while the diamond is still SSA: each predecessor branches
//! on its own incoming value, and the phi, the arm stores, the merge test,
//! and (in the all-predecessors-threaded case) the merge block itself die.
//! This pass performs exactly that transform — on IR, for every target, at
//! every optimization tier — instead of trying to patch it up in a backend
//! peephole: a peephole fusion of `cmpl $0, slot; je` would break the other
//! predecessor of a multi-predecessor slot merge; the merge is only removable
//! when ALL incoming edges are rewritten together, which is a CFG-level
//! decision.
//!
//! Soundness contract (every rule is fail-closed; a candidate that fails any
//! check keeps its existing shape):
//!
//! 1. The merge block contains phis and nothing else, and ends in
//!    `CondBranch(%p, Bt, Bf)` where `%p` is one of its own phis. Nothing is
//!    ever duplicated, so threading cannot grow code: a threaded predecessor
//!    only swaps its `jmp` for a (possibly conditional) branch whose
//!    condition is a value it already computed or a constant.
//! 2. Bt and Bf differ from the merge block (self-referencing branches are
//!    left alone) and from each other (a same-target CondBranch is
//!    cfg_simplify's job).
//! 3. Every phi of the merge block may be referenced in exactly two
//!    positions: as this block's CondBranch condition (only `%p`), or as an
//!    incoming arm of a phi in Bt/Bf on the edge from this block. Any other
//!    use — in a regular instruction, in a terminator, on any other phi
//!    edge, in any other block — rejects the candidate. This is the
//!    domination argument made mechanical: a value `v` arriving on the
//!    merge→Bt edge is available at the end of every predecessor (its def
//!    dominates the merge, so it lies on every entry→Pi→merge path, and the
//!    only instructions between Pi and the merge are none — edges carry no
//!    code; a value *defined by the merge* exists only on paths that ran the
//!    merge, so it may only be consumed where this transform rewrites the
//!    consumption per edge (Bt/Bf phi arms) or deletes the consumption
//!    (the CondBranch)).
//! 4. A predecessor is threadable only when its terminator is exactly
//!    `Branch(Bmerge)` (unconditional — no critical-edge splitting) and it
//!    contains no InlineAsm with goto labels (those carry implicit CFG edges
//!    that the terminator does not describe). NonlocalGoto is outside CFG
//!    discipline in this IR — like every other CFG pass (cfg_simplify,
//!    phi elimination, liveness), this pass does not model it as an edge.
//! 5. A predecessor whose incoming arm for ANY merge phi is itself a merge
//!    phi (self- or mutual reference — the loop-carried shape) is skipped:
//!    the arm value is not provably available at the predecessor's end.
//!    Rule 3 already rejects such candidates wholesale (the self/mutual arm
//!    sits in a merge phi of the merge block itself, which is not a target
//!    phi); this check is kept as an independent second lock — the two rules
//!    guard the same hazard through different scans, so a future relaxation
//!    of either one stays sound.
//! 6. Every merge phi carries exactly one arm per predecessor edge, with no
//!    labels outside the visible predecessor set (an arm for an invisible
//!    edge — e.g. an asm-goto target — would be silently dropped by the
//!    rewrite; reject instead). Conversely, every phi of Bt/Bf carries
//!    exactly one arm on the merge edge (malformed otherwise — reject).
//!
//! Threading a predecessor Pi:
//!   - `%p`'s arm `v_i` selects the new terminator: constant → the matching
//!     unconditional branch, value → `CondBranch(v_i, Bt, Bf)` (phi arms
//!     carry the phi's type, so the branch tests the identical value the
//!     merge would have tested).
//!   - Each phi of Bt/Bf whose merge-edge arm references a merge phi `%q`
//!     gains an arm for Pi carrying `%q`'s Pi-arm; arms holding constants or
//!     values defined outside the merge are duplicated verbatim (available
//!     at Pi's end by the domination argument of rule 3).
//!   - Pi's arm is removed from every merge phi.
//!
//! If all predecessors thread, the merge block keeps no phi arms and no
//! predecessors; it is left in place for cfg_simplify's unreachable-block
//! removal (which also cleans up phi/asm-goto references to dead blocks).
//! Predecessors that fail rules 4/5 keep their edge; the merge, its phis and
//!    its test survive for them (partial threading — still a strict win:
//! each threaded predecessor drops its arm store and bypasses the merge test).
//!
//! Each analysis round rewrites at most ONE candidate, then restarts the
//! analysis: threading mutates phi arms and terminators of arbitrary blocks,
//! which invalidates every cached table. Loop-free CFGs converge (edges into
//! merges strictly decrease); cyclic merge graphs are capped by the round
//! bound (see `MAX_ROUNDS`). Kill switches: `CCC_NO_BOOL_THREAD=1`,
//! `CCC_DISABLE_PASSES=boolthread`.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::{
    BlockId, Instruction, IrCmpOp, IrConst, IrFunction, Operand, Terminator, Value,
};

/// Safety bound on the per-function fixpoint. In loop-free CFGs the edge
/// count into merges strictly decreases and functions converge in a few
/// rounds; in cyclic merge graphs (a merge whose branch target is itself a
/// merge whose target flows back) re-opened candidates are possible, and
/// this bound — not a structural argument — is the termination guarantee.
/// 512 rounds × O(function) is the worst-case compile-time cost; every
/// individual threading is sound regardless of round order, so a bound hit
/// only leaves optimization opportunities unclaimed.
const MAX_ROUNDS: usize = 512;

/// Thread bool-phi merge diamonds. Returns the number of threaded
/// predecessor edges.
pub(crate) fn run(func: &mut IrFunction) -> usize {
    let mut total = 0usize;
    for _ in 0..MAX_ROUNDS {
        let n = thread_round(func);
        total += n;
        if n == 0 {
            break;
        }
    }
    total
}

/// Structural successors of a terminator (labels only, no operand reads).
fn succs(t: &Terminator) -> Vec<BlockId> {
    match t {
        Terminator::Branch(l) => vec![*l],
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => vec![*true_label, *false_label],
        Terminator::Switch { cases, default, .. } => {
            let mut v: Vec<BlockId> = cases.iter().map(|(_, l)| *l).collect();
            v.push(*default);
            v
        }
        Terminator::IndirectBranch {
            possible_targets, ..
        } => possible_targets.clone(),
        _ => Vec::new(),
    }
}

/// One analysis + rewrite round: finds the FIRST threadable candidate and
/// rewrites it. Returns the number of threaded predecessor edges (0 = no
/// candidate remains).
fn thread_round(func: &mut IrFunction) -> usize {
    if func.blocks.len() < 2 {
        return 0;
    }

    let idx_of: FxHashMap<BlockId, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label, i))
        .collect();

    // Visible predecessors per block (deduped; a CondBranch with both arms
    // into the same target yields one entry — it is never threadable, and
    // the exact-arm-count check rejects whatever phi shape it implies).
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); func.blocks.len()];
    for (i, b) in func.blocks.iter().enumerate() {
        let mut seen = FxHashSet::default();
        for s in succs(&b.terminator) {
            if let Some(&j) = idx_of.get(&s) {
                if seen.insert(j) {
                    preds[j].push(i);
                }
            }
        }
    }

    // Immediate-dominator tree for the value-RHS admissibility check
    // (G6). Computed lazily on the first value-RHS candidate — the CFG is
    // stable during candidate classification (rewrites happen only on the
    // return path, after which this round is over).
    let mut idom: Vec<usize> = Vec::new();

    for (mi, _) in func.blocks.iter().enumerate() {
        if preds[mi].len() < 2 {
            continue; // single-pred phis are cfg_simplify's trivial-phi case
        }
        let ml = func.blocks[mi].label;

        // ---- Rule 1/2: phis-only block (bool shape) or phis + exactly one
        // merge-local compare (int shape), CondBranch on a phi or on that
        // compare's dest. ----
        let (cond_val, bt, bf) = match &func.blocks[mi].terminator {
            Terminator::CondBranch {
                cond: Operand::Value(v),
                true_label,
                false_label,
            } => (*v, *true_label, *false_label),
            _ => continue,
        };
        if bt == ml || bf == ml || bt == bf {
            continue;
        }
        // The int-cmp generalization: vsprintf/printf's `--precision >= 0`
        // merges test an int phi through a merge-local compare. Threading
        // re-materializes `Cmp(op, v_i, rhs)` on each predecessor's own
        // incoming value. Because this shape DOES duplicate an instruction,
        // it threads only when EVERY predecessor is threadable (the merge
        // then dies: N added compares strictly lose against N removed arm
        // stores plus the merge's reload-compare-branch); partial threading
        // would let a threaded predecessor pay the compare without the
        // merge's death, so it is rejected wholesale.
        let mut int_cmp: Option<(IrCmpOp, Operand, IrType, usize)> = None; // (op, rhs, ty, p_pos)
        let mut phi_dests: Vec<Value> = Vec::new();
        {
            let merge = &func.blocks[mi];
            let mut dests: Vec<Value> = Vec::new();
            let mut non_phi: Option<(&Value, &IrCmpOp, &Operand, &Operand, &IrType)> = None;
            let mut only = true;
            for inst in &merge.instructions {
                match inst {
                    Instruction::Phi { dest, .. } => dests.push(*dest),
                    Instruction::Cmp {
                        dest,
                        op,
                        lhs,
                        rhs,
                        ty,
                    } => {
                        if non_phi.is_some() {
                            only = false;
                            break;
                        }
                        non_phi = Some((dest, op, lhs, rhs, ty));
                    }
                    _ => {
                        only = false;
                        break;
                    }
                }
            }
            if !only {
                continue;
            }
            if dests.contains(&cond_val) {
                // Bool shape: the branch tests a phi directly. A tolerated
                // merge-local Cmp's dest must be provably dead — a use
                // ANYWHERE outside the defining Cmp vetoes threading, even
                // partial threading: a threaded predecessor branches
                // directly to Bt/Bf, BYPASSING the merge where the Cmp is
                // defined, so every downstream use (including Bt/Bf phi
                // arms, whose per-predecessor substitution would append a
                // non-dominating (q, pred) arm) reads an undefined value
                // on shortcut paths. (The tempting "partial threading
                // keeps the merge alive" refinement is unsound for exactly
                // this dominance reason.)
                if let Some((cdest, _, _, _, _)) = non_phi {
                    if cmp_dest_used_elsewhere(func, mi, *cdest) {
                        continue;
                    }
                }
                phi_dests = dests;
            } else if let Some((cdest, op, lhs, rhs, ty)) = non_phi {
                // Int shape: the branch tests the merge-local compare...
                if *cdest != cond_val {
                    continue;
                }
                // ... whose LHS must be a merge phi and whose RHS an integer
                // constant (a constant dominates every predecessor, so the
                // re-materialized compare is always well-formed) — or, as
                // the value-RHS extension (G6), an SSA value whose
                // definition dominates EVERY predecessor's end and survives
                // the merge's death (a merge-local def — e.g. another merge
                // phi, which passes the dominance test for loop-header
                // merges — would dangle once the full thread kills the
                // merge, so merge-local defs are rejected outright).
                let p = match lhs {
                    Operand::Value(v) if dests.contains(v) => *v,
                    _ => continue,
                };
                let rhs_op = match rhs {
                    Operand::Const(c) => Operand::Const(c.clone()),
                    Operand::Value(rv) => {
                        if idom.is_empty() {
                            let (p_adj, s_adj) = crate::ir::analysis::build_cfg(func, &idx_of);
                            idom = crate::ir::analysis::compute_dominators(
                                func.blocks.len(),
                                &p_adj,
                                &s_adj,
                            );
                        }
                        let def_block = def_block_of(func, *rv);
                        let admissible = match def_block {
                            Some(db) if db != mi => (0..preds[mi].len())
                                .all(|pos| dominates(db, preds[mi][pos], &idom, func.blocks.len())),
                            _ => false,
                        };
                        if !admissible {
                            continue;
                        }
                        Operand::Value(*rv)
                    }
                };
                let p_pos = match dests.iter().position(|d| *d == p) {
                    Some(x) => x,
                    None => continue,
                };
                // Rule 7: the compare's type equals the tested phi's type —
                // the per-predecessor re-materialization compares the
                // identical value (phi arms carry the phi's type).
                let phi_ty = func.blocks[mi].instructions.iter().find_map(|i| match i {
                    Instruction::Phi { dest, ty, .. } if *dest == p => Some(*ty),
                    _ => None,
                });
                if phi_ty != Some(*ty) {
                    continue;
                }
                // The compare's dest may be referenced ONLY as this block's
                // CondBranch condition (rule 3): a use anywhere else —
                // another instruction, any terminator, any phi arm — would
                // dangle after the merge's death.
                if cmp_dest_used_elsewhere(func, mi, *cdest) {
                    continue;
                }
                int_cmp = Some((*op, rhs_op, *ty, p_pos));
                phi_dests = dests;
            } else {
                continue;
            }
        }
        if phi_dests.is_empty() {
            continue;
        }

        // ---- Rule 3: use classification for every merge phi. The one
        // merge-local compare of the int shape is an allowed LHS user. ----
        if !phi_uses_are_threadable(func, &phi_dests, mi, bt, bf, ml) {
            continue;
        }

        // ---- Rule 6 (merge side): exact per-pred arm tables. ----
        // arms[phi_pos][pred_pos] = incoming operand.
        let pred_labels: Vec<BlockId> = preds[mi].iter().map(|&pi| func.blocks[pi].label).collect();
        let mut arms: Vec<Vec<Option<Operand>>> =
            vec![vec![None; preds[mi].len()]; phi_dests.len()];
        let mut arms_ok = true;
        for (pp, inst) in func.blocks[mi].instructions.iter().enumerate() {
            let incoming = match inst {
                Instruction::Phi { incoming, .. } => incoming,
                _ => break,
            };
            let mut counts: FxHashMap<BlockId, usize> = FxHashMap::default();
            for (op, pred_label) in incoming {
                *counts.entry(*pred_label).or_insert(0) += 1;
                if let Some(&pi) = idx_of.get(pred_label) {
                    if let Some(pos) = preds[mi].iter().position(|&x| x == pi) {
                        if arms[pp][pos].is_none() {
                            arms[pp][pos] = Some(*op);
                        }
                    }
                }
            }
            // Exactly one arm per incoming label, the label set matches the
            // visible predecessor set exactly (invisible edges — asm-goto
            // targets, stale labels — reject), every visible pred covered.
            if counts.values().any(|&c| c != 1)
                || counts.len() != pred_labels.len()
                || arms[pp].iter().any(|a| a.is_none())
            {
                arms_ok = false;
                break;
            }
        }
        if !arms_ok || phi_dests.is_empty() {
            continue;
        }

        // ---- Rule 6 (target side): every phi of Bt/Bf has exactly one
        // merge-edge arm. Pre-validated so the rewrite below cannot fail. ----
        let bti = match idx_of.get(&bt) {
            Some(&x) => x,
            None => continue,
        };
        let bfi = match idx_of.get(&bf) {
            Some(&x) => x,
            None => continue,
        };
        let targets_ok = [bti, bfi].iter().all(|&ti| {
            for inst in &func.blocks[ti].instructions {
                match inst {
                    Instruction::Phi { incoming, .. } => {
                        let n = incoming.iter().filter(|(_, l)| *l == ml).count();
                        if n != 1 {
                            return false;
                        }
                    }
                    _ => break, // phis lead the block
                }
            }
            true
        });
        if !targets_ok {
            continue;
        }

        // ---- Rules 4/5: per-predecessor threadability. ----
        let mut threadable: Vec<bool> = vec![false; preds[mi].len()];
        for (pos, &pi) in preds[mi].iter().enumerate() {
            let pred = &func.blocks[pi];
            let clean_term = matches!(pred.terminator, Terminator::Branch(l) if l == ml);
            let has_asm_goto = pred.instructions.iter().any(|i| {
                matches!(i, Instruction::InlineAsm { goto_labels, .. } if !goto_labels.is_empty())
            });
            if !clean_term || has_asm_goto {
                continue;
            }
            let self_ref = arms.iter().any(|phi_arms| {
                phi_arms[pos]
                    .map(|op| matches!(op, Operand::Value(v) if phi_dests.contains(&v)))
                    .unwrap_or(true)
            });
            if self_ref {
                continue;
            }
            // A predecessor that IS Bt or Bf must not be threaded: its new
            // terminator would be a Branch/CondBranch back to itself
            // (Bt==Pi => Branch(Bt) is an unconditional self-loop), an
            // infinite loop no downstream pass can detect.
            if pi == bti || pi == bfi {
                continue;
            }
            threadable[pos] = true;
        }
        let threaded_count = threadable.iter().filter(|t| **t).count();
        if threaded_count == 0 {
            continue;
        }
        // The int-cmp shape requires the merge to die (every predecessor
        // threaded): the re-materialized compares must be paid for by the
        // merge's death, not by an individual predecessor.
        if int_cmp.is_some() && threaded_count != preds[mi].len() {
            continue;
        }
        // Pre-validate every constant-arm constant-fold BEFORE the rewrite:
        // an un-evaluable pair must reject the candidate wholesale, never
        // leave it half-threaded.
        if let Some((op, Operand::Const(rc), ty, ipos)) = &int_cmp {
            let un_evaluable = (0..preds[mi].len()).any(|pos| {
                !threadable[pos]
                    || matches!(&arms[*ipos][pos], Some(Operand::Const(c))
                        if eval_const_cmp(*op, c, rc, ty).is_none())
            });
            if un_evaluable {
                continue;
            }
        }

        // ---- Rewrite (indices stay valid: no block is added or removed). ----
        // Order matters: (1) threaded predecessors' terminators, (2) Bt/Bf
        // phi arms — appended for every threaded pred in one pass per phi,
        // with the merge-edge arm dropped last so every substitution still
        // reads it —, (3) the merge phis' threaded arms.
        let merge_dies = threaded_count == preds[mi].len();

        // (1) New terminators from %p's arm per threaded predecessor. The
        // bool shape branches on the arm itself; the int shape
        // re-materializes the compare on the arm (constant arms fold the
        // comparison at compile time — no instruction is emitted; pairs that
        // cannot fold were pre-validated away above).
        //
        // Each threaded predecessor also records which of Bt/Bf its new
        // terminator can actually reach: a CONSTANT arm folds to exactly one
        // target, a value arm reaches both. Step (2) must deliver phi arms
        // only along edges that exist after the rewrite — an arm delivered
        // from a predecessor that branches elsewhere is a use of a value on
        // a path where the phi it feeds was never entered with that arm,
        // which corrupts the receiving phi (seen as the zstd FSE tail-loop
        // counter over-count: the loop header kept a stale arm from the
        // predecessor threaded to the overflow exit).
        #[derive(Clone, Copy, PartialEq)]
        enum PredReach {
            Bt,
            Bf,
            Both,
        }
        let mut reach = vec![PredReach::Both; preds[mi].len()];
        let p_pos = match &int_cmp {
            Some((_, _, _, ipos)) => *ipos,
            None => match phi_dests.iter().position(|d| *d == cond_val) {
                Some(x) => x,
                None => continue,
            },
        };
        for (pos, &pi) in preds[mi].iter().enumerate() {
            if !threadable[pos] {
                continue;
            }
            let new_term = match &int_cmp {
                None => match arms[p_pos][pos].unwrap() {
                    Operand::Const(c) => {
                        if const_is_truthy(&c) {
                            reach[pos] = PredReach::Bt;
                            Terminator::Branch(bt)
                        } else {
                            reach[pos] = PredReach::Bf;
                            Terminator::Branch(bf)
                        }
                    }
                    cond => {
                        reach[pos] = PredReach::Both;
                        Terminator::CondBranch {
                            cond,
                            true_label: bt,
                            false_label: bf,
                        }
                    }
                },
                Some((op, rhs, ty, _)) => {
                    let arm = arms[p_pos][pos].unwrap();
                    match (&arm, rhs) {
                        (Operand::Const(c), Operand::Const(rc)) => {
                            match eval_const_cmp(*op, c, rc, ty) {
                                Some(true) => {
                                    reach[pos] = PredReach::Bt;
                                    Terminator::Branch(bt)
                                }
                                Some(false) => {
                                    reach[pos] = PredReach::Bf;
                                    Terminator::Branch(bf)
                                }
                                None => unreachable!("pre-validated constant pair"),
                            }
                        }
                        _ => {
                            reach[pos] = PredReach::Both;
                            // Re-materialize the compare on this
                            // predecessor's own incoming value. A constant
                            // arm against the value-RHS extension is
                            // normalized to the canonical (value LHS, const
                            // RHS) form via the swap mirror, so backends see
                            // exactly the operand shapes the original merge
                            // compare could produce.
                            let fresh = fresh_value(func);
                            let (op_e, lhs_e, rhs_e) = match (&arm, rhs) {
                                (Operand::Const(_), Operand::Value(_)) => {
                                    (mirror_cmp_op(*op), rhs.clone(), arm.clone())
                                }
                                _ => (*op, arm.clone(), rhs.clone()),
                            };
                            func.blocks[pi].instructions.push(Instruction::Cmp {
                                dest: fresh,
                                op: op_e,
                                lhs: lhs_e,
                                rhs: rhs_e,
                                ty: *ty,
                            });
                            Terminator::CondBranch {
                                cond: Operand::Value(fresh),
                                true_label: bt,
                                false_label: bf,
                            }
                        }
                    }
                }
            };
            func.blocks[pi].terminator = new_term;
        }

        // (2) Phi arms in Bt/Bf on the merge edge: merge-phi references are
        // substituted with the corresponding Pi-arm; constants and values
        // defined outside the merge are duplicated verbatim (available at
        // every Pi's end by rule 3's domination argument). The merge-edge
        // arm is dropped only when the merge dies — unthreaded predecessors
        // still deliver through it.
        for &ti in &[bti, bfi] {
            let target_is_bt = ti == bti;
            for inst in &mut func.blocks[ti].instructions {
                let incoming = match inst {
                    Instruction::Phi { incoming, .. } => incoming,
                    _ => break, // phis lead the block
                };
                let k = incoming
                    .iter()
                    .position(|(_, l)| *l == ml)
                    .expect("pre-validated: exactly one merge-edge arm");
                let merge_edge_op = incoming[k].0;
                for (pos, threaded) in threadable.iter().enumerate() {
                    if !*threaded {
                        continue;
                    }
                    // Deliver the arm only along an edge the rewritten CFG
                    // actually has: predecessors folded to one target must
                    // not feed the other target's phis (a stale arm from a
                    // non-predecessor corrupts the phi's slot accounting in
                    // phi elimination).
                    match reach[pos] {
                        PredReach::Both => {}
                        PredReach::Bt if !target_is_bt => continue,
                        PredReach::Bf if target_is_bt => continue,
                        _ => {}
                    }
                    let repl = match merge_edge_op {
                        Operand::Value(v) => match phi_dests.iter().position(|d| *d == v) {
                            Some(qp) => arms[qp][pos].unwrap(),
                            None => merge_edge_op,
                        },
                        Operand::Const(_) => merge_edge_op,
                    };
                    incoming.push((repl, pred_labels[pos]));
                }
                if merge_dies {
                    incoming.remove(k);
                }
            }
        }

        // (3) Drop the threaded predecessors' arms from the merge phis.
        // With unthreaded predecessors remaining, every phi still holds
        // their arms (rule 6 guarantees one arm per visible pred), so no phi
        // can go arm-less here. With all threaded, the phis go arm-less in a
        // now-unreachable block — cfg_simplify collects it.
        let live_labels: FxHashSet<BlockId> = pred_labels
            .iter()
            .zip(threadable.iter())
            .filter(|(_, t)| !**t)
            .map(|(l, _)| *l)
            .collect();
        for inst in &mut func.blocks[mi].instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                incoming.retain(|(_, l)| live_labels.contains(l));
            } else {
                break;
            }
        }

        return threaded_count;
    }

    0
}

/// Rule 3: every use of every merge phi is either the merge block's own
/// CondBranch condition, the LHS of the merge block's own (single) compare —
/// the int-cmp shape —, or an incoming arm of a phi in Bt/Bf on the edge
/// from the merge block. `mi` is the merge block index; `ml` its label.
fn phi_uses_are_threadable(
    func: &IrFunction,
    phi_dests: &[Value],
    mi: usize,
    bt: BlockId,
    bf: BlockId,
    ml: BlockId,
) -> bool {
    let is_phi_dest = |id: u32| phi_dests.iter().any(|d| d.0 == id);
    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                let is_target_phi = block.label == bt || block.label == bf;
                for (op, pred_label) in incoming {
                    if let Operand::Value(v) = op {
                        if is_phi_dest(v.0) && !(is_target_phi && *pred_label == ml) {
                            return false;
                        }
                    }
                }
                continue;
            }
            // The merge block's own compare (the int-cmp candidate's compare)
            // may use a merge phi as its LHS; any other regular instruction
            // using a merge phi rejects the candidate.
            if bi == mi {
                if let Instruction::Cmp { lhs, .. } = inst {
                    if let Operand::Value(v) = lhs {
                        if is_phi_dest(v.0) {
                            continue;
                        }
                    }
                }
            }
            let mut bad = false;
            inst.for_each_used_value(|id| {
                if is_phi_dest(id) {
                    bad = true;
                }
            });
            if bad {
                return false;
            }
        }
        if bi != mi {
            let mut bad = false;
            block.terminator.for_each_used_value(|id| {
                if is_phi_dest(id) {
                    bad = true;
                }
            });
            if bad {
                return false;
            }
        }
    }
    true
}

/// Whether `cdest` — the int-cmp candidate's compare dest, defined in block
/// `mi` — is referenced anywhere other than block `mi`'s own CondBranch
/// condition. A stray use (instruction, terminator, phi arm — including in
/// the merge block itself) would dangle once the merge dies.
fn cmp_dest_used_elsewhere(func: &IrFunction, mi: usize, cdest: Value) -> bool {
    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            if bi == mi {
                // The defining compare itself does not count as a use.
                if let Instruction::Cmp { dest, .. } = inst {
                    if *dest == cdest {
                        continue;
                    }
                }
            }
            let mut bad = false;
            inst.for_each_used_value(|id| {
                if id == cdest.0 {
                    bad = true;
                }
            });
            if bad {
                return true;
            }
        }
        let mut bad = false;
        block.terminator.for_each_used_value(|id| {
            if id == cdest.0 && !(bi == mi) {
                bad = true;
            }
        });
        if bad {
            return true;
        }
    }
    false
}

/// Whether block `a` dominates block `b` under the immediate-dominator
/// tree `idom` (idom[entry] == entry; unreachable blocks stay usize::MAX
/// and dominate nothing). Bounded by the block count against corrupt trees.
fn dominates(a: usize, b: usize, idom: &[usize], n: usize) -> bool {
    let mut cur = b;
    for _ in 0..n {
        if cur == a {
            return true;
        }
        match idom.get(cur) {
            Some(&p) if p != usize::MAX && p != cur => cur = p,
            _ => return false,
        }
    }
    false
}

/// Block index whose instructions define `v` (a ParamRef def counts for
/// its containing block, the entry by construction). None when the value
/// has no definition in this function — an undefined operand, which any
/// conservative consumer must reject.
fn def_block_of(func: &IrFunction, v: Value) -> Option<usize> {
    for (bi, b) in func.blocks.iter().enumerate() {
        for inst in &b.instructions {
            if inst.dest() == Some(v) {
                return Some(bi);
            }
        }
    }
    None
}

/// Swap-operand mirror of a comparison: `a op b == b mirror(op) a`.
/// Used to normalize re-materialized compares back to the canonical
/// (value LHS, const RHS) operand form the original merge compare had.
fn mirror_cmp_op(op: IrCmpOp) -> IrCmpOp {
    match op {
        IrCmpOp::Eq => IrCmpOp::Eq,
        IrCmpOp::Ne => IrCmpOp::Ne,
        IrCmpOp::Slt => IrCmpOp::Sgt,
        IrCmpOp::Sle => IrCmpOp::Sge,
        IrCmpOp::Sgt => IrCmpOp::Slt,
        IrCmpOp::Sge => IrCmpOp::Sle,
        IrCmpOp::Ult => IrCmpOp::Ugt,
        IrCmpOp::Ule => IrCmpOp::Uge,
        IrCmpOp::Ugt => IrCmpOp::Ult,
        IrCmpOp::Uge => IrCmpOp::Ule,
    }
}

/// Mint a fresh Value id (defensive against a stale `next_value_id` cache:
/// never collide with an existing id).
fn fresh_value(func: &mut IrFunction) -> Value {
    let nid = func
        .next_value_id
        .max(func.max_value_id().saturating_add(1));
    func.next_value_id = nid + 1;
    Value(nid)
}

/// Integer constant → i64 carrier (sign-extended). Returns None for
/// non-integer constants.
fn const_to_i64(c: &IrConst) -> Option<i64> {
    match c {
        IrConst::I8(v) => Some(*v as i64),
        IrConst::I16(v) => Some(*v as i64),
        IrConst::I32(v) => Some(*v as i64),
        IrConst::I64(v) => Some(*v),
        IrConst::Zero => Some(0),
        _ => None,
    }
}

/// Compile-time evaluation of `lhs op rhs` for integer constants of type
/// `ty`. Unsigned comparisons mask both carriers to the type width first
/// (constants are stored sign-extended on their signed carrier).
fn eval_const_cmp(op: IrCmpOp, lhs: &IrConst, rhs: &IrConst, ty: &IrType) -> Option<bool> {
    if ty.size() > 8 {
        return None;
    }
    let l = const_to_i64(lhs)?;
    let r = const_to_i64(rhs)?;
    let bits = ty.size() as u32 * 8;
    let mask = if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };
    let (lu, ru) = ((l as u64) & mask, (r as u64) & mask);
    Some(match op {
        IrCmpOp::Eq => l == r,
        IrCmpOp::Ne => l != r,
        IrCmpOp::Slt => l < r,
        IrCmpOp::Sle => l <= r,
        IrCmpOp::Sgt => l > r,
        IrCmpOp::Sge => l >= r,
        IrCmpOp::Ult => lu < ru,
        IrCmpOp::Ule => lu <= ru,
        IrCmpOp::Ugt => lu > ru,
        IrCmpOp::Uge => lu >= ru,
    })
}

/// Truthiness of an IR constant for branch purposes (`!= 0`).
fn const_is_truthy(c: &IrConst) -> bool {
    match c {
        IrConst::I8(v) => *v != 0,
        IrConst::I16(v) => *v != 0,
        IrConst::I32(v) => *v != 0,
        IrConst::I64(v) => *v != 0,
        IrConst::I128(v) => *v != 0,
        IrConst::F32(v) => *v != 0.0,
        IrConst::F64(v) => *v != 0.0,
        IrConst::D32(v) => *v != 0,
        IrConst::D64(v) => *v != 0,
        IrConst::LongDouble(v, _) => *v != 0.0,
        IrConst::Zero => false,
    }
}

#[cfg(test)]
mod tests {
    //! IR-level harness for the threading rules. Constructing the blocks by
    //! hand is the only reliable way to reach the vulnerable candidate
    //! shapes: the earlier pipeline folds source-level diamonds into
    //! Selects, so text-format repros cannot drive `thread_round` into the
    //! phi-merge paths these rules guard.

    use super::*;
    use crate::ir::reexports::{BasicBlock, IrParam};

    fn empty_block(label: u32, term: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions: Vec::new(),
            terminator: term,
            source_spans: Vec::new(),
        }
    }

    /// b0: CondBranch(c, b1, b2)
    /// b1: p = a; Branch(b3)   b2: p = b; Branch(b3)
    /// b3 (merge): Phi p; Cmp q = p < 5; CondBranch(p, b4, b5)
    /// b4: Branch(b5)          b5: ret = q*1000 (+ phi r); Return
    /// `live_cmp`: when true, q is USED in b5 (the dangling-use hazard);
    /// when false, q is dead (the tolerated-dead-Cmp bool shape).
    fn build_bool_merge(live_cmp: bool) -> IrFunction {
        let mut f = IrFunction::new(
            "t".to_string(),
            IrType::I32,
            vec![
                IrParam {
                    ty: IrType::I32,
                    noalias: false,
                    struct_size: None,
                    struct_align: None,
                    struct_eightbyte_classes: vec![],
                    is_f128_sse: false,
                    riscv_float_class: None,
                },
                IrParam {
                    ty: IrType::I32,
                    noalias: false,
                    struct_size: None,
                    struct_align: None,
                    struct_eightbyte_classes: vec![],
                    is_f128_sse: false,
                    riscv_float_class: None,
                },
                IrParam {
                    ty: IrType::I32,
                    noalias: false,
                    struct_size: None,
                    struct_align: None,
                    struct_eightbyte_classes: vec![],
                    is_f128_sse: false,
                    riscv_float_class: None,
                },
            ],
            false,
        );
        f.next_value_id = 6;
        let (va, vb, vc) = (Value(0), Value(1), Value(2));
        let (vp, vq) = (Value(6), Value(7));
        let vr = Value(8); // join phi result
                           // Mirror the real front-end shape: entry block opens with one
                           // ParamRef per parameter defining its value id.
        let b0 = BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::ParamRef {
                    dest: va,
                    param_idx: 0,
                    ty: IrType::I32,
                },
                Instruction::ParamRef {
                    dest: vb,
                    param_idx: 1,
                    ty: IrType::I32,
                },
                Instruction::ParamRef {
                    dest: vc,
                    param_idx: 2,
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(vc),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
        };
        let mut arm = |label: u32, src: Value, target: BlockId| BasicBlock {
            label: BlockId(label),
            instructions: vec![Instruction::Copy {
                dest: vp,
                src: Operand::Value(src),
            }],
            terminator: Terminator::Branch(target),
            source_spans: Vec::new(),
        };
        let b1 = arm(1, va, BlockId(3));
        let b2 = arm(2, vb, BlockId(3));
        let b3 = BasicBlock {
            label: BlockId(3),
            instructions: vec![
                Instruction::Phi {
                    dest: vp,
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(va), BlockId(1)),
                        (Operand::Value(vb), BlockId(2)),
                    ],
                },
                Instruction::Cmp {
                    dest: vq,
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(vp),
                    rhs: Operand::Const(IrConst::I32(5)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(vp),
                true_label: BlockId(4),
                false_label: BlockId(5),
            },
            source_spans: Vec::new(),
        };
        let b4 = empty_block(4, Terminator::Branch(BlockId(5)));
        let b5 = BasicBlock {
            label: BlockId(5),
            instructions: vec![Instruction::Phi {
                dest: vr,
                ty: IrType::I32,
                incoming: vec![
                    // false edge: b3 falls through into b5 directly
                    (Operand::Const(IrConst::I32(100)), BlockId(4)),
                    (Operand::Const(IrConst::I32(200)), BlockId(3)),
                ],
            }],
            terminator: if live_cmp {
                // return q*1000 + r — q crosses the merge boundary
                Terminator::Return(Some(Operand::Value(vq)))
            } else {
                Terminator::Return(Some(Operand::Value(vr)))
            },
            source_spans: Vec::new(),
        };
        f.blocks = vec![b0, b1, b2, b3, b4, b5];
        f
    }

    /// Every used value id must have a definition (ParamRef/def in some
    /// block). The dangling-use invariant after any threading rewrite.
    fn is_branch_to(t: &Terminator, id: u32) -> bool {
        matches!(t, Terminator::Branch(b) if b.0 == id)
    }

    fn all_uses_resolved(f: &IrFunction) -> bool {
        let mut defs: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for b in &f.blocks {
            for i in &b.instructions {
                if let Some(d) = i.dest() {
                    defs.insert(d.0);
                }
            }
        }
        let mut ok = true;
        for b in &f.blocks {
            for i in &b.instructions {
                i.for_each_used_value(|id| {
                    if !defs.contains(&id) {
                        ok = false;
                    }
                });
            }
            b.terminator.for_each_used_value(|id| {
                if !defs.contains(&id) {
                    ok = false;
                }
            });
        }
        ok
    }

    #[test]
    fn bool_shape_with_live_merge_cmp_is_rejected() {
        // The soundness hole: without the dead-check on a FULL thread, the
        // merge dies and b5's use of the Cmp dest dangles. With it, the
        // candidate (every predecessor threadable) must be rejected
        // outright — the arms still branch to the merge and the Cmp
        // survives with every use resolved.
        let mut f = build_bool_merge(true);
        super::run(&mut f);
        let b3 = &f.blocks[3];
        let has_cmp = b3
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Cmp { dest, .. } if *dest == Value(7)));
        let preds_still_branch_to_merge =
            is_branch_to(&f.blocks[1].terminator, 3) && is_branch_to(&f.blocks[2].terminator, 3);
        assert!(
            has_cmp && preds_still_branch_to_merge,
            "the live-Cmp bool candidate must be rejected outright"
        );
        assert!(all_uses_resolved(&f));
    }

    #[test]
    fn bool_shape_with_dead_merge_cmp_still_threads() {
        // q dead: nothing dangles; the candidate keeps the cannot-grow-code
        // property (threading removes arm stores + the merge branch).
        let mut f = build_bool_merge(false);
        super::run(&mut f);
        assert!(all_uses_resolved(&f));
        // The merge must not be reachable from the entry anymore.
        let arms_leave =
            !is_branch_to(&f.blocks[1].terminator, 3) || !is_branch_to(&f.blocks[2].terminator, 3);
        assert!(arms_leave, "dead-Cmp bool candidate is expected to thread");
    }

    #[test]
    fn int_shape_threads_through_merge_local_compare() {
        // Same skeleton, but the merge branches on the Cmp DEST (the
        // '--precision >= 0' shape) and q has no other use.
        let mut f = build_bool_merge(true);
        // Redirect the merge branch to the compare dest; drop the join's
        // use of q so the candidate is admissible.
        f.blocks[3].terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(7)),
            true_label: BlockId(4),
            false_label: BlockId(5),
        };
        if let Terminator::Return(v) = &mut f.blocks[5].terminator {
            *v = Some(Operand::Value(Value(8)));
        }
        super::run(&mut f);
        assert!(all_uses_resolved(&f));
        let arms_leave =
            !is_branch_to(&f.blocks[1].terminator, 3) && !is_branch_to(&f.blocks[2].terminator, 3);
        assert!(
            arms_leave,
            "int-shape candidate (every pred threadable) must thread"
        );
    }

    #[test]
    fn int_shape_value_rhs_dominating_threads() {
        // G6: the merge-local compare tests the phi against a VALUE whose
        // definition (an entry-block ParamRef) dominates both predecessors'
        // ends — admissible. Both predecessors re-materialize the compare
        // against that value on their own arm; the merge dies.
        let mut f = build_bool_merge(true);
        f.blocks[3].terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(7)),
            true_label: BlockId(4),
            false_label: BlockId(5),
        };
        if let Terminator::Return(v) = &mut f.blocks[5].terminator {
            *v = Some(Operand::Value(Value(8)));
        }
        // rhs: constant 5 -> the entry param vc (Value(2)), dominating.
        if let Instruction::Cmp { rhs, .. } = &mut f.blocks[3].instructions[1] {
            *rhs = Operand::Value(Value(2));
        }
        super::run(&mut f);
        assert!(all_uses_resolved(&f));
        let arms_leave =
            !is_branch_to(&f.blocks[1].terminator, 3) && !is_branch_to(&f.blocks[2].terminator, 3);
        assert!(
            arms_leave,
            "value-RHS candidate with dominating def must thread"
        );
        // Both predecessors must carry a re-materialized compare whose RHS
        // is the dominating value (the swap normalization keeps value LHS).
        for pi in [1usize, 2] {
            let has = f.blocks[pi].instructions.iter().any(|i| {
                matches!(
                    i,
                    Instruction::Cmp { lhs: Operand::Value(_), rhs: Operand::Value(rv), .. }
                        if *rv == Value(2)
                )
            });
            assert!(has, "pred b{pi} must re-materialize Cmp(arm, vc)");
        }
    }

    #[test]
    fn int_shape_const_arm_value_rhs_swaps_operands() {
        // G6 normalization: a CONSTANT phi arm against a value RHS would
        // re-materialize as Cmp(const, value); the canonical form is the
        // mirrored Cmp(value, const) — exactly the operand shape the
        // original merge compare could produce. The threaded predecessors
        // must carry the normalized form.
        let mut f = build_bool_merge(true);
        f.blocks[3].terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(7)),
            true_label: BlockId(4),
            false_label: BlockId(5),
        };
        if let Terminator::Return(v) = &mut f.blocks[5].terminator {
            *v = Some(Operand::Value(Value(8)));
        }
        if let Instruction::Cmp { rhs, .. } = &mut f.blocks[3].instructions[1] {
            *rhs = Operand::Value(Value(2));
        }
        // Constant arms: p = 10 on the b1 edge, 20 on the b2 edge.
        if let Instruction::Copy { src, .. } = &mut f.blocks[1].instructions[0] {
            *src = Operand::Const(IrConst::I32(10));
        }
        if let Instruction::Phi { incoming, .. } = &mut f.blocks[3].instructions[0] {
            incoming[0].0 = Operand::Const(IrConst::I32(10));
            incoming[1].0 = Operand::Const(IrConst::I32(20));
        }
        super::run(&mut f);
        assert!(all_uses_resolved(&f));
        let arms_leave =
            !is_branch_to(&f.blocks[1].terminator, 3) && !is_branch_to(&f.blocks[2].terminator, 3);
        assert!(arms_leave, "const-arm value-RHS candidate must thread");
        // Normalized form: Cmp(Gt(v ? : vc-const pair), lhs=Value(2), rhs=Const).
        for pi in [1usize, 2] {
            let has = f.blocks[pi].instructions.iter().any(|i| {
                matches!(
                    i,
                    Instruction::Cmp { op: IrCmpOp::Sgt, lhs: Operand::Value(l), rhs: Operand::Const(_), .. }
                        if *l == Value(2)
                )
            });
            assert!(
                has,
                "pred b{pi} must carry the swap-normalized Cmp(Sgt, vc, const)"
            );
        }
    }

    #[test]
    fn int_shape_value_rhs_non_dominating_is_rejected() {
        // G6 soundness: an RHS defined in ONE predecessor does not dominate
        // the other predecessor's end — the re-materialized compare would
        // read an undefined value there. The candidate must be rejected.
        let mut f = build_bool_merge(true);
        f.blocks[3].terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(7)),
            true_label: BlockId(4),
            false_label: BlockId(5),
        };
        if let Terminator::Return(v) = &mut f.blocks[5].terminator {
            *v = Some(Operand::Value(Value(8)));
        }
        // rhs: a value defined inside b1 (Value(9), fresh id below).
        if let Instruction::Cmp { rhs, .. } = &mut f.blocks[3].instructions[1] {
            *rhs = Operand::Value(Value(9));
        }
        f.blocks[1].instructions.push(Instruction::Copy {
            dest: Value(9),
            src: Operand::Const(IrConst::I32(7)),
        });
        f.next_value_id = f.next_value_id.max(10);
        super::run(&mut f);
        assert!(all_uses_resolved(&f));
        let arms_stay =
            is_branch_to(&f.blocks[1].terminator, 3) && is_branch_to(&f.blocks[2].terminator, 3);
        assert!(
            arms_stay,
            "non-dominating value RHS must reject the candidate"
        );
    }

    #[test]
    fn int_shape_value_rhs_merge_local_is_rejected() {
        // G6 soundness: an RHS defined IN the merge (here: another merge
        // phi) passes dominance for loop-header merges but the full thread
        // KILLS the merge — the re-materialized compares would dangle.
        // Merge-local defs are rejected outright.
        let mut f = build_bool_merge(true);
        f.blocks[3].terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(7)),
            true_label: BlockId(4),
            false_label: BlockId(5),
        };
        if let Terminator::Return(v) = &mut f.blocks[5].terminator {
            *v = Some(Operand::Value(Value(8)));
        }
        // rhs: a second merge phi (Value(10)), defined in the merge block
        // itself — merge-local by construction.
        if let Instruction::Cmp { rhs, .. } = &mut f.blocks[3].instructions[1] {
            *rhs = Operand::Value(Value(10));
        }
        f.blocks[3].instructions.insert(
            1,
            Instruction::Phi {
                dest: Value(10),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Const(IrConst::I32(1)), BlockId(1)),
                    (Operand::Const(IrConst::I32(2)), BlockId(2)),
                ],
            },
        );
        f.next_value_id = f.next_value_id.max(11);
        super::run(&mut f);
        assert!(all_uses_resolved(&f));
        let arms_stay =
            is_branch_to(&f.blocks[1].terminator, 3) && is_branch_to(&f.blocks[2].terminator, 3);
        assert!(arms_stay, "merge-local value RHS must reject the candidate");
    }

    #[test]
    fn bool_shape_live_cmp_partial_thread_is_rejected_too() {
        // The dominance control for the tempting-but-unsound refinement:
        // even when only SOME predecessors would thread (b2 ends in a
        // CondBranch, so it is not threadable), a live merge-local Cmp
        // STILL vetoes threading. A threaded predecessor branches directly
        // to Bt/Bf, bypassing the merge where the Cmp is defined — every
        // downstream use of q (including Bt/Bf phi arms, whose
        // per-predecessor substitution would append a non-dominating
        // (q, pred) arm) reads an undefined value on shortcut paths. The
        // candidate must be rejected outright: NO predecessor threads.
        let mut f = build_bool_merge(true);
        // b2: p = b; CondBranch(vc, merge, b5) — a conditional terminator
        // is not threadable; b5 gains a direct b2 edge that its join phi
        // must cover (arm 300 below).
        f.blocks[2].terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(2)),
            true_label: BlockId(3),
            false_label: BlockId(5),
        };
        if let Instruction::Phi { incoming, .. } = &mut f.blocks[5].instructions[0] {
            if !incoming.iter().any(|(_, l)| *l == BlockId(2)) {
                incoming.push((Operand::Const(IrConst::I32(300)), BlockId(2)));
            }
        }
        super::run(&mut f);
        assert!(
            all_uses_resolved(&f),
            "a rejected candidate must leave the function intact"
        );
        // No predecessor may thread: both arms still branch to the merge.
        assert!(
            is_branch_to(&f.blocks[1].terminator, 3),
            "the live-Cmp candidate must be rejected even when threading would be partial"
        );
        assert!(
            is_branch_to(&f.blocks[2].terminator, 3)
                || matches!(&f.blocks[2].terminator,
                    Terminator::CondBranch { true_label, .. } if *true_label == BlockId(3))
        );
        // The merge block (with its live Cmp) survives untouched.
        let has_cmp = f.blocks[3]
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Cmp { dest, .. } if *dest == Value(7)));
        assert!(has_cmp, "the merge-local Cmp must survive the rejection");
    }
}
