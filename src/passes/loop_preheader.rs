//! LOOP-PREHEADER-1: give every loop a *dedicated* preheader.
//!
//! A dedicated preheader is a block whose only successor is the loop header,
//! reached exactly when the loop is entered from outside.  It is the shape
//! LICM needs before it may hoist a **derived-pointer** load (a load through a
//! call result, a parameter, or a GEP chain — anything that can fault):
//!
//! ```c
//! int f(const int *c, int n) {          // `addl (%rdi), %edx` in the loop,
//!     if (n <= 0) return 0;             // every iteration, forever
//!     int i = 0, t = 0;
//!     do { t += c[0]; i++; } while (i < n);
//!     return t;
//! }
//! ```
//!
//! The guard being *outside* the loop is load-bearing, not decoration: it puts
//! the load in the loop header, which dominates every loop block, so LICM's
//! must-execute rule is satisfied and only the dedicated-preheader rule is
//! missing.  The guard-at-top spelling of the same body —
//! `for (int i = 0; i < n; i++) t += c[0];` — lowers with the guard *as* the
//! header, so the load sits in a block that does not dominate the loop and this
//! pass correctly declines it.  The two spellings are therefore not
//! interchangeable -- one is the target, the other is a refusal -- and both are
//! pinned in `tests/regression/check_loop_preheader.sh` (contracts 1 and 2) so
//! the distinction cannot rot.
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
///
/// The variable is read once per process. `std::env::var` is a real call
/// that walks `environ`, and `check_env_test_hygiene.sh` budgets env reads
/// per pass, so a `OnceLock<bool>` is both the cheaper and the
/// policy-consistent answer.
fn debug_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("CCC_DEBUG_LOOP_PREHEADER").is_ok())
}

#[inline]
fn debug(f: impl FnOnce()) {
    if debug_enabled() {
        f();
    }
}

/// Why a natural loop did not get a dedicated preheader, and how many did.
///
/// OBS-4: this pass is 500+ lines of default-on CFG surgery whose only
/// observable was one line per INSERTION.  "inserted nothing" and "inserted
/// nothing because the profitability gate rejected every loop" are entirely
/// different findings, and only the second says the gate is mis-tuned.  A pass
/// that never fires is either useless or blind and you cannot tell which
/// without a census, so the counters live here and are reported per function
/// under `CCC_DEBUG_LOOP_PREHEADER`.
///
/// NOT a `thread_local!`. It is a plain `&mut Census` threaded through the
/// call graph and owned by `run_function`. A thread-local was wrong twice
/// over: `run_function` is called once per function on the same worker
/// thread, so nothing ever reset it and every function's report printed
/// the running total of all previous ones under its own name; and the
/// `bump()` read-modify-WRITE-back dance existed only to work around
/// `Cell`. A stack local in `run_function` is per-function by
/// construction and is race-free without any thread-local reasoning.
#[derive(Default, Clone, Copy)]
struct Census {
    /// Merged natural loops in the FINAL round -- the steady state.
    loops: usize,
    no_profit: usize,
    no_single_pred: usize,
    already_dedicated: usize,
    indirect_pred: usize,
    /// A header or preheader index outside `func.blocks`: a malformed CFG
    /// the pass must not act on. It used to be added to `loops` and to no
    /// bucket at all, which is why the buckets did not sum to `loops`.
    out_of_range: usize,
    /// Loops this round planned an insertion for. Non-zero only in a round
    /// that inserted; the final round plans nothing, which is why it stops
    /// the fixpoint.
    planned: usize,
    /// Running total over ALL rounds. The only field that accumulates: it is
    /// the answer to "what did this pass actually do", as opposed to the
    /// rejection buckets, which describe the code as it now stands.
    inserted: usize,
}

impl Census {
    /// Every merged loop lands in exactly one bucket, so the buckets always
    /// sum to `loops`. The round loop `debug_assert!`s this, which is the
    /// point: a new `continue` that forgets to classify becomes a test
    /// failure instead of a number that quietly stops adding up. That is
    /// the whole difference between a census you can trust and one that
    /// only looks like data.
    fn accounted(&self) -> usize {
        self.no_profit
            + self.no_single_pred
            + self.already_dedicated
            + self.indirect_pred
            + self.out_of_range
            + self.planned
    }
}

fn report_census(func_name: &str, c: Census) {
    // `inserted` is the pass's total work; the rest is the steady state the
    // last round settled on. `already_dedicated` therefore counts loops this
    // pass itself dedicated on an earlier round -- which is the healthy
    // outcome, not a rejection.
    //
    // `planned` MUST be reported. It is one of the six buckets `accounted()`
    // sums, and the gate in tests/regression/check_loop_preheader.sh checks
    // the REPORTED numbers rather than trusting the in-process assert (which
    // a build with debug-assertions off would not run). Leaving this field
    // out made the gate's equation `loops == five buckets` while the code's
    // was `loops == six`, so the two disagreed by construction and the gate
    // was only passing because the reported round is the fixpoint, where
    // nothing is planned. Emitting it is what makes the two the same
    // equation.
    debug(|| {
        eprintln!(
            "[LOOP-PREHEADER-CENSUS] {func_name}: loops={loops} inserted={inserted} \
             planned={planned} \
             rejected: profitability={no_profit} no_single_outside_pred={no_single_pred} \
             already_dedicated={already_dedicated} indirect_pred={indirect_pred} \
             out_of_range={out_of_range}",
            func_name = func_name,
            loops = c.loops,
            inserted = c.inserted,
            planned = c.planned,
            no_profit = c.no_profit,
            no_single_pred = c.no_single_pred,
            already_dedicated = c.already_dedicated,
            indirect_pred = c.indirect_pred,
            out_of_range = c.out_of_range,
        );
    });
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
/// requires the hoisted load's block to dominate **every** loop block.  In the
/// guard-at-top shape the header is the guard block and holds no loads — which
/// is exactly why the ungated version of this pass measured a small
/// *regression* corpus-wide (27 inserted preheaders, +7 instructions): it paid
/// for blocks that could not unlock anything.
///
/// This check DOES hard-code the header, and that is deliberate. It is not a
/// re-derivation of LICM's rule that would track it automatically; it is the
/// conclusion of the dominance proof in the body below, which shows that no
/// other block can satisfy the rule *under LICM's rule as it stands today*.
/// The coupling is therefore one-directional and needs a human to revisit it:
/// if LICM is ever changed to hoist out of a block that does not dominate the
/// whole loop, this check will silently under-report, and the pass will
/// insert fewer preheaders than LICM could have used. The proof's PREMISE,
/// not just its result, is the thing that has to keep holding.
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
    // ONLY THE HEADER CAN QUALIFY -- proved, not assumed.
    //
    //   Let B be a loop block that dominates every loop block.  The header H is
    //   a loop block, so B dom H.  B is in the natural loop, so by definition of
    //   a natural loop H dom B.  Dominance is antisymmetric, therefore B = H.
    //
    // Cost.  The previous version walked every block of the body and asked
    // whether *it* dominated the body: |body| x |body| dominance queries,
    // each O(depth) up the idom chain, so O(|body|^2 * depth).  For a
    // 5000-block loop at depth 1 that is 2.5e7 steps to recompute one
    // yes/no.  That figure is arithmetic, not a measurement, and it is a
    // WORST case -- real loops are small and shallow, so the typical cost
    // was never near it.  It is kept because it is what motivated the
    // change, and stated as what it is so nobody reads it as a profile.
    // What is actually measured is the outcome: the whole corpus is 20
    // census reports plus the gate's two effect contracts.  The
    // `dominates_block` call is kept rather than assumed away, so a
    // malformed idom degrades to "do not insert" rather than "insert anyway".
    //
    // UPSTREAM ADDED A `debug_assert!` HERE INSTEAD, and this comment records
    // why it was rejected rather than kept. It is worse on both axes.
    //
    // 1. It cannot fail, so it guards nothing. It asserts that no non-header
    //    body block dominates the header. A natural loop is DEFINED so that
    //    the header dominates every block in its body, so if some b != header
    //    also dominated the header, then (header dom b) and (b dom header)
    //    both hold and dominance antisymmetry forces b == header -- a
    //    contradiction. The assert is therefore true by construction of
    //    `NaturalLoop`, and holds or falls with the natural-loop builder, not
    //    with anything this pass decides. It verifies a theorem about the
    //    input data, and would have to be deleted if the natural-loop code
    //    changed, but it is not evidence for the decision below.
    //
    // 2. It does not detect the failure its own comment names. The comment
    //    claimed it would catch "LICM's must-execute rule being relaxed, so
    //    the header-only shortcut silently stops being equivalent". Relaxing
    //    LICM changes what LICM hoists; it changes nothing about dominance
    //    among loop blocks. The assert would stay green through exactly the
    //    regression it was written to catch.
    //
    // 3. It costs the same as the check it replaced. The assert walks the body
    //    with one dominance query per block -- O(|body| * depth) -- which is
    //    exactly the `all(dominates_block(idom, header, b))` below. So the
    //    change bought nothing in debug and, in release, swapped an always-on
    //    fail-closed guard for one that is compiled out. `fastbuild` inherits
    //    `release`, so the shipping compiler would have had no check at all.
    //
    // The `all(...)` below is kept because it is the real invariant and it is
    // cheap enough to always run: it is the same dominance fact LICM itself
    // consults, it is what makes "header-only" an inference rather than a
    // guess, and on a malformed idom it returns false -- do not insert --
    // rather than inserting a preheader LICM would not have used.
    let header = natural_loop.header;
    if header >= func.blocks.len() {
        return false;
    }
    if !natural_loop
        .body
        .iter()
        .all(|&b| dominates_block(idom, header, b))
    {
        return false; // not must-execute: LICM refuses the hoist anyway
    }
    func.blocks[header]
        .instructions
        .iter()
        .any(|inst| match inst {
            Instruction::Load { ptr, volatile, .. } => {
                !*volatile && !alloca_values.contains(&ptr.0) && !global_derived.contains(&ptr.0)
            }
            _ => false,
        })
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
    census: &mut Census,
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
            census.out_of_range += 1;
            continue;
        }
        let header_label = func.blocks[header].label;
        let Some(pred) = loop_analysis::find_preheader(header, &natural_loop.body, &cfg.preds)
        else {
            census.no_single_pred += 1;
            continue; // zero or several outside predecessors: LICM skips it too
        };
        if pred >= func.blocks.len() {
            census.out_of_range += 1;
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
            census.already_dedicated += 1;
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
            census.indirect_pred += 1;
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
        census.planned += 1;
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
        // Block 0 is the function entry.  Inserting before it would make the
        // new block the entry, silently reparenting every parameter and alloca
        // in the function.  It is unreachable today -- an entry-block header
        // has no predecessor outside the loop, so `find_preheader` returns
        // `None` and no plan is ever made for it -- but the failure mode is
        // catastrophic and the guard is free, so it is a refusal rather than
        // an assertion: a future analysis change must lose the optimisation
        // here, not the function.
        //
        // A `debug_assert!` would be the wrong instrument for this invariant: it
        // panics in debug builds and is compiled OUT of release, which inverts
        // the intent for a guard whose entire purpose is to contain a
        // catastrophic, otherwise-silent outcome.  Assertions document
        // invariants you believe cannot be violated; refusals enforce the ones
        // you are not willing to bet the function on.
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
        //
        //    Inserting AT the entry block would silently replace the function
        //    prologue target.  That is unreachable today -- an entry block
        //    has no outside predecessor, so `find_preheader` returns None
        //    first -- but "unreachable" is exactly the kind of invariant that
        //    stops holding when someone widens the admission test.  Assert it
        //    at the point of the dangerous operation.
        debug_assert!(
            header_idx != 0,
            "loop header is the entry block: splicing a preheader would replace \
             the function's entry"
        );
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
    // Per-function census. `steady` is rebuilt from scratch every round and
    // only the LAST one is reported: the fixpoint re-walks every loop on
    // every round, so accumulating into one census counted a single loop
    // once per round -- one guarded loop reported `loops=2 inserted=1`,
    // and a loop the pass had just dedicated was counted in the rejection
    // buckets of a later round as if it had refused it. The rejection
    // buckets now describe the code as it finally stands, and `inserted`
    // is the one field that accumulates.
    let mut steady = Census::default();
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
        steady = Census::default();
        if loops.is_empty() {
            break;
        }
        let loops = loop_analysis::merge_loops_by_header(loops);
        // EVERY merged loop, profitable or not. The profitability filter
        // below used to be the only place `loops` was counted, so the
        // headline number silently excluded exactly the loops the pass had
        // rejected.
        steady.loops = loops.len();
        // Profitability gate: only loops that could actually unlock a hoist.
        let paying: Vec<&NaturalLoop> = loops
            .iter()
            .filter(|l| {
                let ok = preheader_would_unlock_a_hoist(func, l, &cfg.idom, &allocas, &globals);
                if !ok {
                    steady.no_profit += 1;
                }
                ok
            })
            .collect();
        let plans = plan_insertions(func, &paying, &cfg, &mut steady);
        // A hard assert, not `debug_assert!`: `fastbuild` inherits `release`,
        // which has `debug-assertions = false`, so a debug assert here is
        // compiled out of the binary AND out of `cargo test` -- it would
        // enforce nothing at all. The check is one add and one compare per
        // round (a function settles in ~2 rounds), so it is free next to the
        // CFG rebuild that precedes it, and it is the only thing standing
        // between a future unclassified `continue` and a census that quietly
        // stops adding up -- which is the exact failure this whole change
        // exists to remove.
        assert_eq!(
            steady.loops,
            steady.accounted(),
            "every merged loop must be classified exactly once, or the census \
             is not the sum of its parts"
        );
        if plans.is_empty() {
            break;
        }
        let n = apply_insertions(func, &plans);
        if n == 0 {
            break;
        }
        total += n;
    }
    // `inserted` is assigned AFTER the loop, not inside it: `steady` is
    // rebuilt from scratch every round, so writing it per round would be
    // wiped by the next round's reset.
    steady.inserted = total;
    report_census(&func.name, steady);
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
    //! The *transformation* is covered end-to-end by
    //! `tests/regression/check_loop_preheader.sh` and
    //! `tests/regression/loop_preheader_shapes.c`, which assert on the
    //! emitted assembly: a preheader insertion that is structurally right but
    //! hoists nothing is invisible to a Rust-side test, and the property that
    //! matters is the code LCCC finally emits.  The gate asserts both
    //! directions -- the guarded shape whose derived-pointer load must be
    //! hoisted, and the shapes that must come out byte-identical with the
    //! pass disabled.
    use super::*;
    use crate::common::types::{AddressSpace, IrType};
    use crate::ir::reexports::{IrConst, Operand, Value};

    fn cond(t: u32, f: u32) -> Terminator {
        Terminator::CondBranch {
            cond: Operand::Const(IrConst::I32(1)),
            true_label: BlockId(t),
            false_label: BlockId(f),
        }
    }

    /// The header-only scan is sound only while the header dominates EVERY
    /// block of the loop body -- that is LICM's must-execute rule, and it is
    /// what makes "only the header can qualify" an inference rather than a
    /// guess. `NaturalLoop` does not enforce it: `header` and `body` are
    /// public fields with no validation, so a caller can hand this pass a
    /// loop whose body contains a block the header does not dominate.
    ///
    /// This test calls `preheader_would_unlock_a_hoist` ITSELF on exactly that
    /// input, because a test of a guard has to reach the code under test. An
    /// earlier version re-derived the predicate locally and passed with the
    /// guard deleted -- the same "green for the wrong reason" defect this
    /// series exists to remove.
    ///
    /// It also pins why the `debug_assert!` upstream added in the function's
    /// place was rejected. That assert asks a different question ("does some
    /// non-header body block dominate the header?"), which holds for every
    /// well-formed natural loop and so cannot fail; it stays green through
    /// the very input below, which is the regression its comment promised to
    /// catch.
    #[test]
    fn header_only_scan_fails_closed_when_the_header_does_not_dominate_the_body() {
        // 0 (entry) -> 1 (header) -> 2      the header dominates 2
        // 0 (entry) -> 3                    the header does NOT dominate 3
        let idom: Vec<usize> = vec![0, 0, 1, 0];
        let mut blocks: Vec<BasicBlock> = (0..4)
            .map(|i| BasicBlock {
                label: BlockId(i),
                instructions: Vec::new(),
                terminator: Terminator::Unreachable,
                source_spans: Vec::new(),
            })
            .collect();
        // The header holds a hoistable load, so the dominance check is the
        // ONLY thing that can turn this answer to `false`.
        blocks[1].instructions.push(Instruction::Load {
            dest: Value(0),
            ptr: Value(7),
            ty: IrType::I64,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
        let mut func = IrFunction::new("t".into(), IrType::I32, Vec::new(), false);
        func.blocks = blocks;

        let mut malformed = NaturalLoop {
            header: 1,
            body: FxHashSet::default(),
        };
        malformed.body.insert(1);
        malformed.body.insert(2);
        malformed.body.insert(3); // sibling of the header: NOT dominated by it

        let empty: FxHashSet<u32> = FxHashSet::default();
        assert!(
            !preheader_would_unlock_a_hoist(&func, &malformed, &idom, &empty, &empty),
            "a body the header does not fully dominate must NOT unlock a hoist"
        );

        // Same CFG, body pruned to what the header really dominates. Without
        // this the assertion above could be satisfied by a function that
        // always answers `false`.
        let mut well_formed = NaturalLoop {
            header: 1,
            body: FxHashSet::default(),
        };
        well_formed.body.insert(1);
        well_formed.body.insert(2);
        assert!(
            preheader_would_unlock_a_hoist(&func, &well_formed, &idom, &empty, &empty),
            "with the body pruned to the header's dominance the load must be found"
        );

        // The assert upstream added, on the input that breaks the invariant:
        // it PASSES, so it was guarding nothing.
        let discarded_assert_holds = malformed
            .body
            .iter()
            .filter(|&&b| b != malformed.header)
            .all(|&b| !dominates_block(&idom, b, malformed.header));
        assert!(
            discarded_assert_holds,
            "the discarded debug_assert passes on the very input that breaks \
             the invariant it was offered as a guard for"
        );
    }

    /// The census is only useful if its headline number is the sum of its
    /// reasons. `accounted()` is what the round loop asserts against
    /// `loops`, and `out_of_range` is the bucket that used to be missing:
    /// an index outside `func.blocks` was counted in `loops` and in no
    /// bucket at all, so the parts could not add up.
    #[test]
    fn census_buckets_partition_the_loops() {
        let mut c = Census {
            loops: 6,
            no_profit: 1,
            no_single_pred: 1,
            already_dedicated: 2,
            indirect_pred: 1,
            out_of_range: 1,
            planned: 0,
            inserted: 2,
        };
        assert_eq!(c.loops, c.accounted(), "buckets must sum to loops");

        // A loop the pass refused without a reason is exactly the drift this
        // catches: it is in `loops` and in no bucket.
        c.no_profit -= 1;
        assert_ne!(c.loops, c.accounted());

        // A planned loop is its own bucket, not a rejection.
        let mut p = Census {
            loops: 1,
            planned: 1,
            ..Census::default()
        };
        assert_eq!(p.loops, p.accounted());
        p.inserted = 1;
        assert_eq!(p.loops, p.accounted(), "inserted is a total, not a bucket");
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
