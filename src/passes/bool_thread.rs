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
use crate::ir::reexports::{BlockId, Instruction, IrConst, IrFunction, Operand, Terminator, Value};

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

    for (mi, _) in func.blocks.iter().enumerate() {
        if preds[mi].len() < 2 {
            continue; // single-pred phis are cfg_simplify's trivial-phi case
        }
        let ml = func.blocks[mi].label;

        // ---- Rule 1/2: phis-only block, CondBranch on one of its phis. ----
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
        let (phi_dests, phis_only) = {
            let merge = &func.blocks[mi];
            let mut dests: Vec<Value> = Vec::new();
            let mut only = true;
            for inst in &merge.instructions {
                match inst {
                    Instruction::Phi { dest, .. } => dests.push(*dest),
                    _ => {
                        only = false;
                        break;
                    }
                }
            }
            (dests, only)
        };
        if !phis_only || !phi_dests.contains(&cond_val) {
            continue;
        }

        // ---- Rule 3: use classification for every merge phi. ----
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
            threadable[pos] = true;
        }
        let threaded_count = threadable.iter().filter(|t| **t).count();
        if threaded_count == 0 {
            continue;
        }

        // ---- Rewrite (indices stay valid: no block is added or removed). ----
        // Order matters: (1) threaded predecessors' terminators, (2) Bt/Bf
        // phi arms — appended for every threaded pred in one pass per phi,
        // with the merge-edge arm dropped last so every substitution still
        // reads it —, (3) the merge phis' threaded arms.
        let merge_dies = threaded_count == preds[mi].len();
        let p_pos = phi_dests.iter().position(|d| *d == cond_val).unwrap();

        // (1) New terminators from %p's arm per threaded predecessor.
        for (pos, &pi) in preds[mi].iter().enumerate() {
            if !threadable[pos] {
                continue;
            }
            let new_term = match arms[p_pos][pos].unwrap() {
                Operand::Const(c) => {
                    if const_is_truthy(&c) {
                        Terminator::Branch(bt)
                    } else {
                        Terminator::Branch(bf)
                    }
                }
                cond => Terminator::CondBranch {
                    cond,
                    true_label: bt,
                    false_label: bf,
                },
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
/// CondBranch condition or an incoming arm of a phi in Bt/Bf on the edge
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
        IrConst::LongDouble(v, _) => *v != 0.0,
        IrConst::Zero => false,
    }
}
