//! LOOP-PREHEADER-1: give every loop a *dedicated* preheader.
//!
//! A dedicated preheader is a block whose only successor is the loop header,
//! reached exactly when the loop is entered from outside.  It is the shape
//! LICM needs before it may hoist a **derived-pointer** load (a load through a
//! call result, a parameter, or a GEP chain — anything that can fault):
//!
//! ```c
//! int f(const int *c, int n) {          // `addl (%rdi), %edx` in the loop,
//!     int t = 0;                        // every iteration, forever
//!     for (int i = 0; i < n; i++) t += c[0];
//!     return t;
//! }
//! ```
//!
//! ## Why the shape matters, and why it is a *soundness* requirement
//!
//! Hoisting a possibly-faulting load out of a loop executes it on paths that
//! never entered the loop.  The interesting case is not the loop body — it is
//! the guard that makes the dereference legal in the first place.  The
//! documented miscompile this gate exists to prevent (`licm.rs`):
//!
//! ```c
//! p = sqlite3_get_auxdata(ctx, JSON_CACHE_ID);
//! if (p == 0) return 0;
//! for (i = 0; i < p->nUsed; i++) ...    // hoisting `p->nUsed` segfaults
//! ```
//!
//! The `if (p == 0)` block is the loop's unique outside predecessor, so it *is*
//! the preheader — but it also branches to the early return.  Hoisting into it
//! dereferences NULL whenever the cache is empty.
//!
//! So LICM refuses any load whose preheader is not dedicated.  That gate is
//! correct.  **The defect was that LCCC never *creates* a dedicated
//! preheader**, so the gate refused essentially every derived-pointer load in
//! the corpus: the frontend lowers a counted loop as
//!
//! ```text
//!   entry:  t = 0; i = 0; cmp; CondBranch(body, exit)   <-- preheader, 2 succs
//!   body:   ...; i++; cmp; CondBranch(body, exit)
//!   exit:
//! ```
//!
//! and a preheader with two successors is by definition not dedicated.
//!
//! ## What this pass does
//!
//! Splice an empty block onto the single outside edge:
//!
//! ```text
//!   entry:  t = 0; i = 0; cmp; CondBranch(pre, exit)
//!   pre:    Branch(body)                                <-- dedicated
//!   body:   ...; i++; cmp; CondBranch(body, exit)
//!   exit:
//! ```
//!
//! `pre` is reached exactly when the loop is entered, and the header is part of
//! the loop body, so reaching `pre` implies the body runs at least once.  An
//! instruction that was already must-execute inside the loop (LICM requires the
//! hoisted load's block to dominate **every** loop block — `licm.rs`) is
//! therefore still must-execute in `pre`, and nothing executes on a path that
//! never entered the loop.  The SQLite shape is safe because the `p == 0`
//! branch now goes to `exit` *before* `pre` exists.
//!
//! ## Scope
//!
//! Only loops with a **single** outside predecessor are rewritten.  A loop with
//! several would need the header's phis to carry two entries for the same
//! incoming block, which SSA does not allow; LICM skips those loops anyway
//! (`find_preheader` returns `None`).  An `IndirectBranch` into the header
//! cannot be routed through a new block at all (the computed address is the
//! header's), so such loops are left alone.
//!
//! The inserted block carries no instructions, so an empty preheader that LICM
//! ends up not using costs a label and nothing else.

use super::loop_analysis::{self, NaturalLoop};
use crate::common::fx_hash::FxHashSet;
use crate::ir::analysis;
use crate::ir::reexports::{BasicBlock, BlockId, Instruction, IrFunction, Operand, Terminator};

/// Rewrite every edge `old -> new` in a terminator.  Returns true if any edge
/// was rewritten.
///
/// `IndirectBranch` is deliberately NOT handled: its `possible_targets` list is
/// advisory metadata about a computed address, not an edge that can be
/// rerouted, so callers must refuse those blocks instead.
fn retarget_edges(term: &mut Terminator, old: BlockId, new: BlockId) -> bool {
    let mut hit = false;
    let mut fix = |l: &mut BlockId| {
        if *l == old {
            *l = new;
            hit = true;
        }
    };
    match term {
        Terminator::Branch(l) => fix(l),
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            fix(true_label);
            fix(false_label);
        }
        Terminator::Switch { cases, default, .. } => {
            for (_, l) in cases.iter_mut() {
                fix(l);
            }
            fix(default);
        }
        // `Return`, `Unreachable` and `IndirectBranch`: no reroutable edge.
        _ => {}
    }
    hit
}

/// True when `term` is an unconditional branch to `label` — the dedicated
/// preheader shape.
fn is_dedicated_to(term: &Terminator, label: BlockId) -> bool {
    matches!(term, Terminator::Branch(l) if *l == label)
}

/// Emit a pass diagnostic when `CCC_DEBUG_LOOP_PREHEADER` is set.
#[inline]
fn debug(f: impl FnOnce()) {
    if std::env::var("CCC_DEBUG_LOOP_PREHEADER").is_ok() {
        f();
    }
}

/// One planned preheader insertion, in labels so it survives other insertions.
struct Plan {
    header_label: BlockId,
    pred_label: BlockId,
    new_label: BlockId,
}

/// True when inserting a dedicated preheader could actually let LICM hoist
/// something out of this loop.
///
/// A dedicated preheader unlocks exactly one class of hoist: a **derived
/// pointer** load (a parameter, a call result, a GEP chain — anything that can
/// fault).  Loads through an alloca or a direct `GlobalAddr` are already
/// hoistable without one, so a loop with none of the former gains nothing and
/// would only carry an empty block.
///
/// The load must also sit in a block LICM's must-execute rule accepts: LICM
/// requires the hoisted load's block to dominate **every** loop block, which
/// in a natural loop means the block must be the header.  In the guard-at-top
/// shape the header is the guard block and holds no loads — which is exactly
/// why the ungated version of this pass measured a small *regression*
/// corpus-wide (27 inserted preheaders, +7 instructions): it paid for blocks
/// that could not unlock anything.  This check mirrors LICM's rule rather than
/// hard-coding "the header", so it stays correct if that rule is relaxed.
///
/// It is a cheap syntactic pre-filter, not a re-implementation of LICM's alias
/// analysis: if the load proves unhoistable for some other reason (a store
/// that may alias it), the cost is one empty block, and that case is rare and
/// bounded.
fn preheader_would_unlock_a_hoist(
    func: &IrFunction,
    natural_loop: &NaturalLoop,
    idom: &[usize],
    alloca_values: &FxHashSet<u32>,
    global_derived: &FxHashSet<u32>,
) -> bool {
    for &block_idx in &natural_loop.body {
        if block_idx >= func.blocks.len() {
            continue;
        }
        if !natural_loop
            .body
            .iter()
            .all(|&b| dominates_block(idom, block_idx, b))
        {
            continue; // not must-execute: LICM refuses the hoist anyway
        }
        for inst in &func.blocks[block_idx].instructions {
            if let Instruction::Load { ptr, volatile, .. } = inst {
                if *volatile || alloca_values.contains(&ptr.0) {
                    continue;
                }
                if global_derived.contains(&ptr.0) {
                    continue; // LICM's global path does not need this shape
                }
                return true;
            }
        }
    }
    false
}

/// `true` when `a` dominates `b` in the immediate-dominator tree.
fn dominates_block(idom: &[usize], a: usize, mut b: usize) -> bool {
    if a == b {
        return true;
    }
    let mut guard = 0usize;
    while b < idom.len() && idom[b] != b && guard <= idom.len() {
        b = idom[b];
        if a == b {
            return true;
        }
        guard += 1;
    }
    false
}

/// Values that are `GlobalAddr` results or GEPs transitively derived from one.
fn global_derived_values(func: &IrFunction) -> FxHashSet<u32> {
    let mut out: FxHashSet<u32> = FxHashSet::default();
    // Repeat until stable: a GEP chain can be built in any block order.
    loop {
        let before = out.len();
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::GlobalAddr { dest, .. } => {
                        out.insert(dest.0);
                    }
                    Instruction::GetElementPtr { dest, base, .. } => {
                        if out.contains(&base.0) {
                            out.insert(dest.0);
                        }
                    }
                    Instruction::Copy { dest, src, .. } => {
                        if let Operand::Value(s) = src {
                            if out.contains(&s.0) {
                                out.insert(dest.0);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if out.len() == before {
            break;
        }
    }
    out
}

/// Values produced by an `alloca` — never in need of a dedicated preheader.
fn alloca_values(func: &IrFunction) -> FxHashSet<u32> {
    let mut out: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca { dest, .. } | Instruction::DynAlloca { dest, .. } = inst {
                out.insert(dest.0);
            }
        }
    }
    out
}

/// Plan the insertions for one CFG snapshot.
fn plan_insertions(
    func: &IrFunction,
    loops: &[&NaturalLoop],
    cfg: &analysis::CfgAnalysis,
) -> Vec<Plan> {
    let mut plans = Vec::new();
    let mut next_label = func.next_label.max(
        func.blocks
            .iter()
            .map(|b| b.label.0)
            .max()
            .map(|m| m + 1)
            .unwrap_or(1),
    );
    for natural_loop in loops {
        let header = natural_loop.header;
        if header >= func.blocks.len() {
            continue;
        }
        let header_label = func.blocks[header].label;
        let Some(pred) = loop_analysis::find_preheader(header, &natural_loop.body, &cfg.preds)
        else {
            continue; // zero or several outside predecessors: LICM skips it too
        };
        if pred >= func.blocks.len() {
            continue;
        }
        let pred_block = &func.blocks[pred];
        if is_dedicated_to(&pred_block.terminator, header_label) {
            debug(|| {
                eprintln!(
                    "[LOOP-PREHEADER] loop header {} already has a dedicated \
                     preheader {}",
                    header_label.0, pred_block.label.0
                )
            });
            continue; // already dedicated
        }
        if matches!(pred_block.terminator, Terminator::IndirectBranch { .. }) {
            debug(|| {
                eprintln!(
                    "[LOOP-PREHEADER] loop header {} reached by IndirectBranch \
                     from {}: cannot reroute",
                    header_label.0, pred_block.label.0
                )
            });
            continue; // a computed goto into the header cannot be rerouted
        }
        debug(|| {
            eprintln!(
                "[LOOP-PREHEADER] header {} preheader {} is not dedicated: \
                 inserting {}",
                header_label.0, pred_block.label.0, next_label
            )
        });
        let new_label = BlockId(next_label);
        next_label = next_label.saturating_add(1);
        plans.push(Plan {
            header_label,
            pred_label: pred_block.label,
            new_label,
        });
    }
    plans
}

/// Insert the planned preheaders.
///
/// Applied from the highest header index downwards so an insertion never
/// invalidates the index of a header it has not reached yet; even so, every
/// block is located by label immediately before use, so the pass is correct
/// regardless of ordering.
fn apply_insertions(func: &mut IrFunction, plans: &[Plan]) -> usize {
    let mut inserted = 0usize;
    let mut max_label = func.next_label;
    for plan in plans {
        let Some(pred_idx) = func.blocks.iter().position(|b| b.label == plan.pred_label) else {
            continue;
        };
        let Some(header_idx) = func
            .blocks
            .iter()
            .position(|b| b.label == plan.header_label)
        else {
            continue;
        };
        if pred_idx == header_idx {
            continue; // a self-loop header: not a shape LICM hoists from
        }
        // Block 0 is the function entry. Inserting before it would make the
        // new block the entry, silently reparenting every parameter and
        // alloca in the function. It is unreachable today -- an entry-block
        // header has no predecessor outside the loop, so `find_preheader`
        // returns `None` and no plan is ever made for it -- but the failure
        // mode is catastrophic and the guard is free, so it is a refusal
        // rather than an assertion: a future analysis change must lose the
        // optimisation here, not the function.
        if header_idx == 0 {
            continue;
        }
        // 1. Reroute the outside edge.  Only `pred`'s terminator is rewritten:
        //    backedges from inside the loop still target the header directly.
        if !retarget_edges(
            &mut func.blocks[pred_idx].terminator,
            plan.header_label,
            plan.new_label,
        ) {
            continue; // pred does not actually branch to the header
        }
        // 2. The header's phis name the incoming edge by predecessor label.
        for inst in func.blocks[header_idx].instructions.iter_mut() {
            if let Instruction::Phi { incoming, .. } = inst {
                for (_, lbl) in incoming.iter_mut() {
                    if *lbl == plan.pred_label {
                        *lbl = plan.new_label;
                    }
                }
            }
        }
        // 3. Splice the empty block in immediately before the header, so the
        //    backend can fall through from it instead of emitting a jump.
        func.blocks.insert(
            header_idx,
            BasicBlock {
                label: plan.new_label,
                instructions: Vec::new(),
                terminator: Terminator::Branch(plan.header_label),
                source_spans: Vec::new(),
            },
        );
        max_label = max_label.max(plan.new_label.0 + 1);
        inserted += 1;
    }
    func.next_label = max_label;
    inserted
}

/// Give every eligible loop in `func` a dedicated preheader.
///
/// Returns the number of blocks inserted.  Re-planning after each round is
/// what keeps the CFG snapshot honest: a single snapshot cannot describe a CFG
/// the pass is about to change, and the cost is one `CfgAnalysis::build` per
/// round — bounded, because each round strictly reduces the number of
/// non-dedicated loops.
pub fn run_function(func: &mut IrFunction) -> usize {
    if func.is_declaration || func.blocks.len() < 2 {
        return 0;
    }
    let allocas = alloca_values(func);
    let globals = global_derived_values(func);
    let mut total = 0usize;
    // Bound the fixpoint: at most one insertion per loop, plus one final round
    // that finds nothing.  A pathological CFG must not turn into an
    // unbounded rebuild loop.
    let max_rounds = func.blocks.len() + 1;
    for _ in 0..max_rounds {
        let cfg = analysis::CfgAnalysis::build(func);
        let loops =
            loop_analysis::find_natural_loops(cfg.num_blocks, &cfg.preds, &cfg.succs, &cfg.idom);
        if loops.is_empty() {
            break;
        }
        let loops = loop_analysis::merge_loops_by_header(loops);
        // Profitability gate: only loops that could actually unlock a hoist.
        let paying: Vec<&NaturalLoop> = loops
            .iter()
            .filter(|l| preheader_would_unlock_a_hoist(func, l, &cfg.idom, &allocas, &globals))
            .collect();
        let plans = plan_insertions(func, &paying, &cfg);
        if plans.is_empty() {
            break;
        }
        let n = apply_insertions(func, &plans);
        if n == 0 {
            break;
        }
        total += n;
    }
    total
}

/// True when the pass is enabled for this run.
///
/// The kill switch is the generic `CCC_DISABLE_PASSES=loop_preheader`, which
/// `pass_disabled` matches as an exact token.  There is deliberately no
/// dedicated `CCC_NO_LOOP_PREHEADER`: it would be a second name for the same
/// switch, and `tests/regression/check_env_test_hygiene.sh` ratchets the pass
/// pipeline's environment reads downward (157 call sites, "never raise it"),
/// so a redundant knob costs a real invariant rather than nothing.
pub fn enabled(disabled: impl AsRef<str>) -> bool {
    !crate::passes::pass_disabled(disabled, "loop_preheader")
}

#[cfg(test)]
mod tests {
    //! Unit tests for the pieces that are pure functions of a terminator.
    //!
    //! COVERAGE GAP, STATED PLAINLY: the *transformation* has no dedicated
    //! end-to-end gate yet. What exists is indirect and worth knowing about,
    //! because it is what would catch a mis-spliced edge today:
    //!
    //!   * `tests/regression/licm_no_speculative_load_nondedicated_preheader.c`
    //!     drives the SQLite NULL-guard shape this pass rewrites, and
    //!     segfaults deterministically if the inserted block ever lands on
    //!     the wrong edge.
    //!   * `verify::verify_after_pass(module, "loop_preheader")` runs on
    //!     every insertion, so a broken phi or a dangling label fails loudly.
    //!
    //! Neither asserts the pass *fires*, so a silent regression to "inserts
    //! nothing" would pass both. A real gate must assert on emitted assembly
    //! (a structurally correct preheader that hoists nothing is invisible to
    //! a Rust-side test) and must use `CCC_DISABLE_PASSES=loop_preheader` as
    //! its negative control, or it proves nothing. Tracked in `backlog.md`.
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::reexports::{IrConst, Operand};

    fn cond(t: u32, f: u32) -> Terminator {
        Terminator::CondBranch {
            cond: Operand::Const(IrConst::I32(1)),
            true_label: BlockId(t),
            false_label: BlockId(f),
        }
    }

    #[test]
    fn branch_is_dedicated_only_when_it_targets_the_header() {
        assert!(is_dedicated_to(&Terminator::Branch(BlockId(7)), BlockId(7)));
        assert!(!is_dedicated_to(
            &Terminator::Branch(BlockId(8)),
            BlockId(7)
        ));
        assert!(!is_dedicated_to(&cond(7, 9), BlockId(7)));
        assert!(!is_dedicated_to(&Terminator::Return(None), BlockId(7)));
    }

    #[test]
    fn retarget_rewrites_every_kind_of_reroutable_edge() {
        // Unconditional.
        let mut t = Terminator::Branch(BlockId(1));
        assert!(retarget_edges(&mut t, BlockId(1), BlockId(5)));
        assert!(matches!(t, Terminator::Branch(l) if l == BlockId(5)));

        // Conditional: both arms, but only the matching one.
        let mut t = cond(1, 2);
        assert!(retarget_edges(&mut t, BlockId(1), BlockId(5)));
        match t {
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => {
                assert_eq!(true_label, BlockId(5));
                assert_eq!(false_label, BlockId(2), "the other arm is untouched");
            }
            _ => unreachable!(),
        }

        // Switch: every case plus the default.
        let mut t = Terminator::Switch {
            val: Operand::Const(IrConst::I32(0)),
            cases: vec![(0, BlockId(1)), (1, BlockId(2)), (2, BlockId(1))],
            default: BlockId(1),
            ty: IrType::I32,
        };
        assert!(retarget_edges(&mut t, BlockId(1), BlockId(5)));
        match t {
            Terminator::Switch { cases, default, .. } => {
                assert_eq!(cases[0].1, BlockId(5));
                assert_eq!(cases[1].1, BlockId(2), "non-matching case untouched");
                assert_eq!(cases[2].1, BlockId(5));
                assert_eq!(default, BlockId(5));
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn retarget_refuses_when_there_is_nothing_to_reroute() {
        let mut t = cond(1, 2);
        assert!(!retarget_edges(&mut t, BlockId(9), BlockId(5)));

        // An IndirectBranch is never rerouted: its target list describes a
        // computed address, so a caller must refuse the loop instead.
        let mut t = Terminator::IndirectBranch {
            target: Operand::Const(IrConst::I64(0)),
            possible_targets: vec![BlockId(1)],
        };
        assert!(!retarget_edges(&mut t, BlockId(1), BlockId(5)));
    }
}
