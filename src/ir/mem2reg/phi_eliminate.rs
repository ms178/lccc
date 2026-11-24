//! Phi elimination: lower SSA phi nodes to copies in predecessor blocks.
//!
//! This pass runs after all SSA optimizations and before backend codegen.
//! It converts each Phi instruction into Copy instructions placed at the end
//! of each predecessor block (before the terminator).
//!
//! Smart temporary allocation:
//! When a block has multiple phis, we analyze the copy graph per-predecessor
//! to determine which copies actually need temporaries (only those involved
//! in cycles, e.g., swap patterns) and which can be direct copies. This
//! dramatically reduces the number of temporaries and copy instructions,
//! especially for large switch statements where most phis pass through
//! values unchanged.
//!
//! "Involved in a cycle" is decided exactly, by decomposing the edge's
//! precedence graph with Kahn's algorithm (see [`plan_edge_copies`]): a copy
//! needs a temporary only when it is mutually blocked with another copy, i.e.
//! when the parallel assignment genuinely contains a cycle. An acyclic chain
//! needs none and is emitted in the order that makes the sequence equivalent
//! to the simultaneous assignment. The earlier rule approximated a cycle by
//! "my source is somebody's destination", which is true of every copy in a
//! rotation, so rotations paid a temporary and an unconditional relay copy per
//! phi per iteration — and, because that doubles the loop-carried webs, the
//! register allocator went over budget and staged the rotation through the
//! stack.
//!
//! # Resolution policy: cycle-accurate is the default (2026-09-12)
//!
//! The cycle-accurate resolver is **on by default**; `CCC_PHI_ACYCLIC_ORDER=0`
//! (alias `CCC_NO_PHI_ACYCLIC_ORDER=1`) restores the legacy two-phase policy.
//! It lowered `sha256_transform`'s stack references 53 -> 21 and instruction
//! count 195 -> 170 at -O2, and the runtime delta flipped from a 1-7 % loss to
//! a 3-169 % win at every optimizing tier (biggest at -O1/-Os where the legacy
//! temps exploded the live-range population). See `copy_order_policy` for the
//! measurements and the 6-of-807-TU blast-radius screen.
//!
//! The original ~5-7% penalty was misdiagnosed as "eliminating redundant copies
//! is not monotone in code quality". It is not that. The real cause was an
//! allocator bug the resolver merely *exposed*:
//!
//! * With the two-phase temps the loop-carried word `a` exists as two short
//!   complementary ranges — the phi input (latch -> header) and the body value
//!   (header -> latch) — and the register allocator assigns both.
//! * Resolving the rotation acyclically merges them into one range spanning the
//!   back edge, and that range was then left stack-homed: 14 memops/iteration
//!   concentrated on 2 slots read 7x each, against 25 memops spread over 11
//!   slots in the legacy loop.
//! * The demotion never came from the range being long.
//!   `live_range::mark_loop_spanning` computed the web-wide in-loop-use flag
//!   from `ranges.iter()`, and a coalesced member is merged into its leader's
//!   interval and owns no `LiveRange`, so the aggregation documented as carrying
//!   the web-wide flag was a silent no-op for exactly those webs. The recurrence
//!   words are led by their cold preheader loads (`leader=v166
//!   members=[166,389]`, `leader=v182 members=[182,392]`) with every own use
//!   point outside the loop, so `span_has_in_loop_use` came out false and the
//!   in-loop-USELESS-span demotion rule fired on the hottest values in the loop.
//!
//! That is fixed — see `mark_loop_spanning`, kill switch
//! `CCC_NO_WEB_INLOOP_USE=1` — and it is worth +3.6..+4.3% on this kernel on its
//! own. With the bug gone the resolver's penalty shrank from ~5% to
//! -0.7..-2.0%: still reproducible in sign across two replicates, no longer
//! catastrophic. It stays opt-in because the allocator fix *without* it is
//! strictly better — +1.9% combined vs +4.0% for the fix alone.
//!
//! What remains is spill traffic, not range length and not loop structure. The
//! resolver cuts this function's frame-relative stack references 62 -> 29 and its
//! instruction count 198 -> 178, yet `gcc -O2` emits **8** references in **142**
//! instructions and is still 44% faster at runtime — while both compilers emit
//! the same number of loops here (2 backward jumps each), so no fusion or
//! splitting difference explains it. What LCCC emits extra is 54 frame-relative
//! references, ~40 more `mov`s, 9 more labels and 6 more compares. RA-06
//! "location pieces" is that open item, and it is independent of this resolver.
//! Measured record, including the detector caveat that makes per-loop counts
//! untrustworthy in LCCC's output here:
//! `engineering/evidence/ra-web-inloop-use-2026-09-11/`.
//!
//! Falsified alternatives (do not retry): the store-to-load width mismatch —
//! the rotation emits 64-bit `movq` stores feeding 32-bit `movl` loads — is
//! *not* the cause; rewriting the round loop's moves to 32-bit moves runtime by
//! 1.8%, i.e. noise. `CCC_EVICT_MODE`, `CCC_NO_TIER2_GRAPH` and
//! `CCC_NO_LEAF_CALLER_HOME` do not change the victim set either, and widening
//! the round-loop budget would mean forcing `MachInst` onto a large loop,
//! already measured negative (gzip -3%).
//!
//! For non-conflicting phis (the common case), we emit direct copies:
//!   pred_block:
//!     %phi1_dest = copy src1
//!     %phi2_dest = copy src2
//!     `<terminator>`
//!
//! For conflicting phis (cycles), we use shared temporaries and a two-phase
//! copy sequence to avoid the lost-copy problem:
//!   pred_block:
//!     %tmp1 = copy src1  // save source before it's overwritten
//!     `<terminator>`
//!   target_block:
//!     %phi1_dest = copy %tmp1  // restore from temporary
//!     ... rest of block ...
//!
//! Critical edge splitting:
//! When a predecessor block has multiple successors (e.g., a CondBranch) and
//! the target block has phis, placing copies at the end of the predecessor
//! would execute them on ALL outgoing paths, not just the edge to the phi's
//! block. This corrupts values used on other paths. To fix this, we split
//! the critical edge by inserting a new trampoline block that contains only
//! the phi copies and branches unconditionally to the target.
//!
//! Self-loop latch absorption:
//! The critical-edge rule above is over-broad for a *backedge into the block
//! that holds the phis*. Such a block is its own predecessor, so it always has
//! at least two successors (itself plus the loop exits) and the rule always
//! fires, even though the copies it protects belong to the loop's own
//! induction values. The result is a copy-only latch block and one extra
//! unconditional branch per iteration -- precisely the cost loop rotation is
//! meant to remove, reintroduced one pass later.
//!
//! Splitting is only *needed* when a copy destination is observable on one of
//! the block's other outgoing edges. So instead of splitting unconditionally,
//! the backedge copies are buffered and then validated as a parallel copy
//! (`self_loop_copies_can_hoist`). If every destination is invisible outside
//! the block and the copies commute, they are moved into the block itself
//! (ahead of the compare feeding its branch, see `place_pred_copies`) and the
//! latch disappears. If any destination escapes, the trampoline is built
//! exactly as before, so the transform is strictly a refinement of the old
//! behaviour. Rotation makes this fire in practice: rotating lifts the exit
//! value into a distinct SSA value, so the accumulator the exit phi reads is
//! no longer a phi destination. Set `CCC_NO_PHI_SELFLOOP_HOIST=1` to disable.
//!
//! Phi-web coalescing:
//! Before lowering, non-interfering phi webs are renamed to one variable
//! (`phi_webs`), so the copies between them vanish. `CCC_NO_PHI_WEB_COALESCE=1`
//! disables it.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, IrModule, Operand,
    Terminator, Value,
};

/// Eliminate all phi nodes in the module by lowering them to copies.
///
/// `narrow_gprs` selects the target default parallel-copy policy. Targets
/// with the i686 register economy (6 free GPRs) default to the legacy
/// resolver, because the cycle-accurate resolver merges each loop-carried
/// value's two short ranges into one range spanning the back edge, which on
/// a 6-GPR target leaves exactly the recurrence words stack-homed: measured
/// -7.4 % on `sha256_transform` (tight paired CI, i686 -O2) even though the
/// static counts shrink. Wide-GPR targets (x86-64's 15 GPRs, AArch64's 31,
/// RISC-V 64) default to the cycle-accurate resolver, where the same change
/// is +3-169 %. The environment always wins (see [`copy_order_policy`]).
pub fn eliminate_phis(module: &mut IrModule, narrow_gprs: bool) {
    let policy = copy_order_policy(narrow_gprs);
    // Compute the global max block ID across ALL functions to avoid label collisions
    // when creating trampoline blocks. Labels are module-wide (.LBB0, .LBB1, ...).
    let mut next_block_id = 0u32;
    for func in &module.functions {
        for block in &func.blocks {
            if block.label.0 >= next_block_id {
                next_block_id = block.label.0 + 1;
            }
        }
    }
    if std::env::var("LCCC_DEBUG_LABELS").is_ok() {
        lccc::cli::stderr_line(format_args!(
            "[PHI] Starting phi_eliminate with next_block_id = {} policy={:?}",
            next_block_id, policy
        ));
    }

    for func in &mut module.functions {
        if func.is_declaration || func.blocks.is_empty() {
            continue;
        }
        if std::env::var("LCCC_DEBUG_LABELS").is_ok() {
            lccc::cli::stderr_line(format_args!(
                "[PHI] Processing function {}, current labels: {:?}",
                func.name,
                func.blocks.iter().map(|b| b.label.0).collect::<Vec<_>>()
            ));
        }
        eliminate_phis_with_policy(func, &mut next_block_id, policy);
        if std::env::var("LCCC_DEBUG_LABELS").is_ok() {
            lccc::cli::stderr_line(format_args!(
                "[PHI] After processing {}, labels: {:?}, next_block_id now {}",
                func.name,
                func.blocks.iter().map(|b| b.label.0).collect::<Vec<_>>(),
                next_block_id
            ));
        }
    }
}

/// Returns the number of distinct successor block IDs for a block.
/// Accounts for both terminator targets and InlineAsm goto_labels.
/// Number of DISTINCT blocks `block` can transfer control to, counting the
/// terminator's edges and every asm-goto label.
///
/// The four terminator kinds name at most an `IndirectBranch`'s target list or
/// a `Switch`'s case list, and those lists are not deduplicated: C lowers
/// `switch (x) { case 0: case 1: ... }` to a case list that repeats targets,
/// which on machine-generated input is tens of thousands of repeats.  The
/// previous implementation deduplicated with `seen.contains()`, so one such
/// block cost O(labels^2) -- 8.2% of a whole compile on a 25,000-arm switch
/// (callgrind), on top of the three sibling CFG builders that had the same
/// loop.  `SuccSet` deduplicates in O(1) per label and stops counting as soon
/// as the block is known to have more than one successor, which is all the
/// caller asks.
fn successor_count(block: &BasicBlock) -> usize {
    let mut set = crate::common::succ_set::SuccSet::new();
    // `BlockId` values are sparse (they name IR labels, not positions), so
    // size the stamp from the widest id this terminator can name instead of
    // from a block count this function has no access to.  `insert` grows the
    // stamp if a target still exceeds it, so this is an allocation hint, not
    // a precondition.
    set.begin(widest_target_id(block).saturating_add(1));
    match &block.terminator {
        Terminator::Return(_) | Terminator::Unreachable => {}
        Terminator::Branch(label) => {
            set.insert(label.0 as usize);
        }
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            set.insert(true_label.0 as usize);
            set.insert(false_label.0 as usize);
        }
        Terminator::IndirectBranch {
            possible_targets, ..
        } => {
            for label in possible_targets {
                set.insert(label.0 as usize);
            }
        }
        Terminator::Switch { cases, default, .. } => {
            set.insert(default.0 as usize);
            for &(_, label) in cases {
                set.insert(label.0 as usize);
            }
        }
    }
    for inst in &block.instructions {
        if let Instruction::InlineAsm { goto_labels, .. } = inst {
            for (_, label) in goto_labels {
                set.insert(label.0 as usize);
            }
        }
    }
    set.len()
}

/// One past the widest `BlockId` the block's terminator or asm-goto labels can
/// name; 0 when the block names nothing.
fn widest_target_id(block: &BasicBlock) -> usize {
    let mut widest = 0u32;
    let mut bump = |id: u32| widest = widest.max(id);
    match &block.terminator {
        Terminator::Return(_) | Terminator::Unreachable => {}
        Terminator::Branch(label) => bump(label.0),
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            bump(true_label.0);
            bump(false_label.0);
        }
        Terminator::IndirectBranch {
            possible_targets, ..
        } => {
            for label in possible_targets {
                bump(label.0);
            }
        }
        Terminator::Switch { cases, default, .. } => {
            bump(default.0);
            for &(_, label) in cases {
                bump(label.0);
            }
        }
    }
    for inst in &block.instructions {
        if let Instruction::InlineAsm { goto_labels, .. } = inst {
            for (_, label) in goto_labels {
                bump(label.0);
            }
        }
    }
    widest as usize + 1
}

/// Send the edges from `block` to `old_target` to `new_target` instead: the
/// terminator's (see [`retarget_terminator`]), and every asm goto label.
///
/// The asm gotos are not an alternative to the terminator.  A block may reach
/// the target both ways, the phi has one entry for the block either way, and
/// the copies of that entry have to run on each of the edges.
fn retarget_block_edges(block: &mut BasicBlock, old_target: BlockId, new_target: BlockId) {
    retarget_terminator(&mut block.terminator, old_target, new_target);
    for inst in &mut block.instructions {
        if let Instruction::InlineAsm { goto_labels, .. } = inst {
            for (_, label) in goto_labels.iter_mut() {
                if *label == old_target {
                    *label = new_target;
                }
            }
        }
    }
}

/// Replace `old_target` with `new_target` in a terminator: one arm of a
/// conditional branch, and EVERY case of a switch that names it.
///
/// A phi has one entry per predecessor block, so the copies of that entry
/// belong on each edge the block has into the target. A `Switch` can have
/// several (`case 4: case 6: v = 30; break;` once the empty case blocks are
/// threaded): moving only the first left the other cases jumping straight to
/// the target, past the copies, with whatever the phi's home held.
fn retarget_terminator(terminator: &mut Terminator, old_target: BlockId, new_target: BlockId) {
    match terminator {
        Terminator::Branch(t) => {
            if *t == old_target {
                *t = new_target;
            }
        }
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            // Only retarget one edge to avoid changing both sides of a diamond
            if *true_label == old_target {
                *true_label = new_target;
            } else if *false_label == old_target {
                *false_label = new_target;
            }
        }
        Terminator::IndirectBranch {
            possible_targets, ..
        } => {
            if let Some(t) = possible_targets.iter_mut().find(|t| **t == old_target) {
                *t = new_target;
            }
        }
        Terminator::Switch { cases, default, .. } => {
            if *default == old_target {
                *default = new_target;
            }
            for (_, target) in cases.iter_mut() {
                if *target == old_target {
                    *target = new_target;
                }
            }
        }
        _ => {}
    }
}

struct TrampolineBlock {
    label: BlockId,
    copies: Vec<Instruction>,
    branch_target: BlockId,
    pred_idx: usize,
    old_target: BlockId,
}

/// Get or create a trampoline block for a (pred, target) critical edge.
fn get_or_create_trampoline(
    trampoline_map: &mut FxHashMap<(usize, BlockId), usize>,
    trampolines: &mut Vec<TrampolineBlock>,
    pred_idx: usize,
    target_block_id: BlockId,
    next_block_id: &mut u32,
) -> usize {
    *trampoline_map
        .entry((pred_idx, target_block_id))
        .or_insert_with(|| {
            let idx = trampolines.len();
            let label = BlockId(*next_block_id);
            if std::env::var("LCCC_DEBUG_LABELS").is_ok() {
                lccc::cli::stderr_line(format_args!(
                    "[PHI] Creating trampoline block with BlockId({}), next will be {}",
                    label.0,
                    *next_block_id + 1
                ));
            }
            *next_block_id += 1;
            trampolines.push(TrampolineBlock {
                label,
                copies: Vec::new(),
                branch_target: target_block_id,
                pred_idx,
                old_target: target_block_id,
            });
            idx
        })
}

/// How one predecessor edge's parallel copies are resolved.
///
/// Carried in [`PhiElimCtx`] so the environment is read once per function
/// instead of once per (block, edge) — and so both arms are reachable from the
/// unit tests without mutating the process environment, which would race with
/// the parallel test threads (the hazard `MS-04` exists for).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CopyOrderPolicy {
    /// The historical over-approximation: every copy whose source is another
    /// copy's destination is routed through a shared temporary. This is the
    /// production default — see "Blocker status" in the module header.
    Legacy,
    /// Cycle-accurate: only copies in a genuine dependency cycle get a
    /// temporary, and acyclic chains are resolved by ordering alone. Opt-in via
    /// `CCC_PHI_ACYCLIC_ORDER=1`.
    Acyclic,
}

/// Select the parallel-copy resolution policy.
///
/// **Default: [`CopyOrderPolicy::Acyclic`]** (cycle-accurate Kahn
/// decomposition). The legacy over-approximation (route every copy whose
/// source is another copy's destination through a shared temporary) is kept
/// behind the kill switch `CCC_PHI_ACYCLIC_ORDER=0` (alias
/// `CCC_NO_PHI_ACYCLIC_ORDER=1`) for bisection and rollback.
///
/// History / why the default flipped on 2026-09-12. The cycle-accurate
/// resolver originally shipped opt-in because it *measured* 1-7 % slower on
/// `sha256_transform` while being a large static win. That runtime loss was
/// never in the resolver: `live_range::mark_loop_spanning` derived the
/// web-wide in-loop-use flag from `ranges` alone, so coalesced members
/// (which own no range) were invisible and the hottest recurrence words were
/// demoted to the stack. Once that supply bug was fixed (Session 16/17), the
/// static gains still did not translate to speed, so it stayed gated.
/// Re-measured on the full stack (post PR #501/#502), paired interleaved
/// A/B with `scripts/run_benchmarks.py` (41 rounds,
/// PASSES=8/BLOCK_COUNT=131072 ≈ 420-470 ms/arm, digest-identical outputs):
///
/// | flags | default/resolver paired median | CI 95 |
/// |---|---|---|
/// | -O1 | 2.69 × faster | [2.657, 2.701] |
/// | -O2 | 1.028 × faster | [1.027, 1.030] |
/// | -O3 | 1.029 × faster | [1.027, 1.031] |
/// | -Os | 1.136 × faster | [1.135, 1.138] |
///
/// Corpus blast radius was screened assembly-by-assembly: of 807 translation
/// units under tests/ only 6 change codegen at -O2 (sha256_transform, fib and
/// four regression tests), all with equal or lower instruction/stack-ref
/// counts; at -O0 output is byte-identical. The exhaustive 18,240-copy-graph
/// oracle in the unit tests proves BOTH policies implement simultaneous
/// assignment.
fn copy_order_policy(narrow_gprs: bool) -> CopyOrderPolicy {
    // Explicit kill: "0" on the positive switch, or the dedicated NO_ switch.
    // Explicit enable: "1"/"true"/"yes" (used by tests, gates, and i686
    // experiments). Unset/empty/other -> target default: acyclic on wide-GPR
    // targets, legacy where the register economy cannot hold the merged
    // back-edge ranges (i686, measured -7% sha256_transform).
    match std::env::var("CCC_PHI_ACYCLIC_ORDER")
        .as_deref()
        .unwrap_or("")
    {
        "0" => return CopyOrderPolicy::Legacy,
        "1" | "true" | "yes" => return CopyOrderPolicy::Acyclic,
        _ => {}
    }
    if std::env::var("CCC_NO_PHI_ACYCLIC_ORDER").as_deref() == Ok("1") {
        return CopyOrderPolicy::Legacy;
    }
    if narrow_gprs {
        CopyOrderPolicy::Legacy
    } else {
        CopyOrderPolicy::Acyclic
    }
}

/// How one predecessor edge's parallel copies are resolved.
///
/// `eliminate_phis` lowers a block's phis to a *simultaneous* assignment
/// `dest_i <- src_i`. Executing that as a plain instruction sequence is only
/// correct when every copy reads its source before some other copy overwrites
/// it. The classical solution (parallel-move resolution) decomposes the copy
/// graph into cycles, which need one shared temporary each, and chains, which
/// need none and are simply executed in reverse-topological order.
///
/// The decomposition this used to perform was much coarser: it treated "my
/// source is somebody's destination" as a cycle. A rotation such as
/// `b <- a; c <- b; d <- c` is an acyclic chain, yet every copy in it reads a
/// destination, so all of them were routed through the two-phase temporary
/// scheme. The cost is not one redundant instruction but a redundant *web*:
/// each temporary is a fresh value with its own live range, so the register
/// allocator sees twice as many loop-carried values and twice as many
/// unconditional copies per iteration. SHA-256's eight-word state rotation is
/// the extreme case — 16 webs and 8 header copies per round where the rotation
/// needs 8 webs and 6 moves — and the allocator, over budget, staged the whole
/// rotation through the stack (7 stores + 6 loads per round against GCC's 6
/// `movl`).
#[derive(Default, Debug)]
struct EdgeCopyPlan {
    /// Indices of copies that participate in a dependency *cycle*. These
    /// cannot be executed sequentially and are served by a shared temporary.
    cyclic: FxHashSet<usize>,
    /// Every other index, in a safe execution order: each copy appears after
    /// all copies whose destination it reads.
    ordered: Vec<usize>,
}

/// Resolve one predecessor edge's parallel copies under the selected policy.
///
/// Both arms are separate functions so the unit tests can pin each policy
/// directly instead of mutating the process environment.
///
/// `guards[i]` lists IR value ids that copy `i`'s *computed* source reads
/// (see [`plan_edge_copies_guards`]); register-to-register copies carry an
/// empty set. Only the acyclic arm consults them.
fn plan_edge_copies(copies: &[(u32, Option<u32>)], policy: CopyOrderPolicy) -> EdgeCopyPlan {
    let empty: Vec<crate::common::fx_hash::FxHashSet<u32>> = vec![Default::default(); copies.len()];
    plan_edge_copies_guards(copies, &empty, policy)
}

/// Decompose one edge's parallel copies into cyclic and orderable parts.
///
/// Builds the precedence graph `i -> j` meaning "copy `i` must execute before
/// copy `j`", which holds exactly when `i` reads the destination `j` writes.
/// Kahn's algorithm then emits every copy whose prerequisites are satisfied;
/// whatever is left when the queue drains is blocked by a cycle and needs a
/// temporary. Self-copies (`dest == src`) write nothing observable and are
/// excluded from both sets — the emitter already skips them.
///
/// The order is deterministic: each round scans `remaining` in ascending index
/// order, so the result is the lexicographically smallest topological order by
/// index. No hash-map iteration reaches the output.
///
/// `guards[i]` are value ids read by copy `i`'s computed (non-copy) source
/// definition — an ALU instruction in the predecessor, not another copy. After
/// copy `i` executes, its destination may be clobbered freely, but while it is
/// pending the values its computation reads must stay intact. Copies whose
/// destination is such a guard are therefore deferred: they may run, but only
/// once every guarded computation that still waits for them has run. This is
/// what lets `fold_computed_phi_incomings` sink a computed source to its copy
/// slot instead of materializing it through a temporary relay (the `rot()`
/// relay that cost one instruction per iteration in
/// `tests/regression/check_phi_acyclic_order.sh`). Deferral never deadlocks:
/// a ready copy is deferred only by another *ready* copy's guard, and a full
/// round that emits nothing falls back to ignoring deferral, so progress is
/// guaranteed and the result stays a valid topological order of the same
/// precedence graph (only the tie-break changed).
fn plan_edge_copies_guards(
    copies: &[(u32, Option<u32>)],
    guards: &[crate::common::fx_hash::FxHashSet<u32>],
    policy: CopyOrderPolicy,
) -> EdgeCopyPlan {
    match policy {
        CopyOrderPolicy::Acyclic => plan_edge_copies_acyclic_guards(copies, guards),
        CopyOrderPolicy::Legacy => plan_edge_copies_legacy(copies),
    }
}

fn plan_edge_copies_acyclic_guards(
    copies: &[(u32, Option<u32>)],
    guards: &[crate::common::fx_hash::FxHashSet<u32>],
) -> EdgeCopyPlan {
    let n = copies.len();
    // value -> index of the copy that writes it. Phi destinations are unique
    // within a block, so this map is a function; `entry().or_insert()` keeps
    // the first writer if a malformed block ever repeats one, which is the
    // conservative choice (it can only add a precedence edge).
    let mut writer: FxHashMap<u32, usize> = FxHashMap::default();
    for (i, &(dest, _)) in copies.iter().enumerate() {
        writer.entry(dest).or_insert(i);
    }

    // preds[j] = the copies that must run before j.
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut skippable = vec![false; n];
    for (i, &(dest_i, src_i)) in copies.iter().enumerate() {
        // A constant source reads no register, so it has no prerequisite.
        let Some(src) = src_i else { continue };
        if src == dest_i {
            // `dest <- dest` is a no-op the emitter drops; giving it a
            // precedence edge would manufacture a self-cycle.
            skippable[i] = true;
            continue;
        }
        if let Some(&j) = writer.get(&src) {
            if j != i {
                // i reads what j writes, so i must come first.
                preds[j].push(i);
            }
        }
    }

    let mut ordered: Vec<usize> = Vec::new();
    let mut emitted = vec![false; n];
    let mut remaining: Vec<usize> = (0..n).filter(|&i| !skippable[i]).collect();
    loop {
        let mut progressed = false;
        let mut blocked: Vec<usize> = Vec::new();
        // Guard deferral: a ready copy is held back while some still-unemitted
        // copy's computed source reads its destination. Scanned in ascending
        // index order, then unblocked wholesale if it starved a round.
        for &i in &remaining {
            let ready = preds[i].iter().all(|&p| emitted[p] || skippable[p]);
            let deferred = ready
                && remaining.iter().any(|&k| {
                    k != i && !emitted[k] && !skippable[k] && guards[k].contains(&copies[i].0)
                });
            if ready && !deferred {
                ordered.push(i);
                emitted[i] = true;
                progressed = true;
            } else {
                blocked.push(i);
            }
        }
        if !progressed {
            // Everything left is either genuinely blocked (a cycle — drained
            // below) or mutually guard-deferred; emit ready copies ignoring
            // deferral so the topological drain always advances.
            for &i in &blocked {
                if preds[i].iter().all(|&p| emitted[p] || skippable[p]) {
                    ordered.push(i);
                    emitted[i] = true;
                    progressed = true;
                }
            }
        }
        remaining = blocked.iter().copied().filter(|&i| !emitted[i]).collect();
        if !progressed {
            break;
        }
    }

    // Whatever never drained is mutually blocked, i.e. on a dependency cycle.
    // Copies merely *downstream* of a cycle also land here, which is safe:
    // a cyclic copy is served by a temporary and therefore never writes its
    // destination on this edge, so the constraint that blocked them does not
    // actually exist. They are still routed through a temporary, which is the
    // conservative (historical) treatment and never changes the result.
    let cyclic: FxHashSet<usize> = remaining.into_iter().collect();
    EdgeCopyPlan { cyclic, ordered }
}

/// The historical over-approximation — the production default.
///
/// It marks more copies cyclic than necessary (see the module header), but its
/// two-phase temps incidentally split loop-carried ranges at the back edge,
/// which the current allocator depends on.
///
/// Marks every copy whose source is another copy's destination, then closes
/// the set transitively over destinations read by an already-marked copy. It
/// never misorders anything — every marked copy is served by a temporary — but
/// it marks acyclic chains too, which is exactly the redundancy
/// [`plan_edge_copies_acyclic`] removes.
fn plan_edge_copies_legacy(copies: &[(u32, Option<u32>)]) -> EdgeCopyPlan {
    let dest_set: FxHashSet<u32> = copies.iter().map(|(d, _)| *d).collect();
    let mut needs_temp: FxHashSet<usize> = FxHashSet::default();
    for (i, &(dest_i, src_i_opt)) in copies.iter().enumerate() {
        if let Some(src_i) = src_i_opt {
            if src_i != dest_i && dest_set.contains(&src_i) {
                needs_temp.insert(i);
            }
        }
    }
    if !needs_temp.is_empty() {
        let conflicting_sources: FxHashSet<u32> =
            needs_temp.iter().filter_map(|&i| copies[i].1).collect();
        for (j, &(dest_j, _)) in copies.iter().enumerate() {
            if conflicting_sources.contains(&dest_j) {
                needs_temp.insert(j);
            }
        }
    }
    // The legacy path emits Pass 2 in phi order, so `ordered` is the identity
    // order with the marked copies removed.
    let ordered = (0..copies.len())
        .filter(|i| !needs_temp.contains(i))
        .collect();
    EdgeCopyPlan {
        cyclic: needs_temp,
        ordered,
    }
}

/// Phi information extracted from a block.
struct PhiInfo {
    dest: Value,
    incoming: Vec<(Operand, BlockId)>,
}

/// Context shared across phi elimination for a single function.
struct PhiElimCtx<'a> {
    label_to_idx: FxHashMap<BlockId, usize>,
    multi_succ: Vec<bool>,
    is_indirect_branch: Vec<bool>,
    /// The `(block, label)` pairs joined by an asm-goto edge.
    asm_goto_edges: FxHashSet<(usize, BlockId)>,
    /// The blocks an asm goto can be left through.
    has_asm_goto: Vec<bool>,
    pred_copies: FxHashMap<usize, Vec<Instruction>>,
    /// Backedge copies for blocks that are their own phi target, buffered so
    /// they can be validated as a whole before deciding between absorbing them
    /// into the block and splitting the edge. Indexed by block.
    self_loop_copies: Vec<Vec<Instruction>>,
    target_copies: Vec<Vec<Instruction>>,
    trampolines: Vec<TrampolineBlock>,
    trampoline_map: FxHashMap<(usize, BlockId), usize>,
    next_block_id: &'a mut u32,
    next_value: u32,
    /// Which copy resolver this function was planned with; selected once per
    /// module by [`eliminate_phis`] (target default + environment override).
    copy_order: CopyOrderPolicy,
}

/// `LCCC_DEBUG_PHI_HOIST`: one line on stderr per loop test retargeted and
/// per set of self-loop copies absorbed or split.
fn phi_hoist_trace() -> bool {
    std::env::var("LCCC_DEBUG_PHI_HOIST").is_ok()
}

/// Where each value of a function is defined, for [`retarget_loop_tests`].
struct DefSites {
    /// Indexed by value number: `(block, instruction)`, [`Self::NONE`] for a
    /// value no instruction defines (a parameter), [`Self::MANY`] for one
    /// with more than one definition.
    sites: Vec<(u32, u32)>,
}

impl DefSites {
    const NONE: (u32, u32) = (u32::MAX, u32::MAX);
    const MANY: (u32, u32) = (u32::MAX, u32::MAX - 1);

    fn of(func: &IrFunction) -> Self {
        let mut this = Self { sites: Vec::new() };
        for (b, block) in func.blocks.iter().enumerate() {
            for (i, inst) in block.instructions.iter().enumerate() {
                if let Some(dest) = inst.dest() {
                    this.define(dest, b, i);
                }
                // An inline-asm output is written by the statement and is
                // no `dest()`: a value named there is defined there too.
                if let Instruction::InlineAsm { outputs, .. } = inst {
                    for (_, output, _) in outputs {
                        this.define(*output, b, i);
                    }
                }
            }
        }
        this
    }

    fn define(&mut self, value: Value, block: usize, inst: usize) {
        let value = value.0 as usize;
        if value >= self.sites.len() {
            self.sites.resize(value + 1, Self::NONE);
        }
        self.sites[value] = if self.sites[value] == Self::NONE {
            // Block and instruction counts are far below `u32::MAX` (labels
            // and value numbers are `u32` themselves), where the two marks
            // above live; a count that is not, were there one, reads as
            // `MANY` and the value is left alone.
            match (u32::try_from(block), u32::try_from(inst)) {
                (Ok(block), Ok(inst)) if block != u32::MAX => (block, inst),
                _ => Self::MANY,
            }
        } else {
            Self::MANY
        };
    }

    /// The one definition of `value`, as `(block, instruction)`.
    fn one(&self, value: Value) -> Option<(usize, usize)> {
        match self.sites.get(value.0 as usize) {
            Some(&site) if site != Self::NONE && site != Self::MANY => {
                Some((site.0 as usize, site.1 as usize))
            }
            _ => None,
        }
    }
}

/// Most links [`offset_from`] follows from a value down to the phi: a sum
/// and a cast on either side of it are three.
const MAX_STEP_LINKS: usize = 4;

/// `value - phi` modulo two to the power of `8 * width`, when `value` is
/// `phi` and constants added and subtracted, with casts in between.
///
/// Each link has to keep the low `width` bytes of what it reads, which is
/// what makes the sum of the constants the answer whatever the wider types
/// do above them: an addition or subtraction in an integer type of at least
/// that width, a cast between two such types (to the narrower one it cuts,
/// to the wider one it makes up bits above the ones that count). A cast
/// *from* a narrower type reads fewer bits than the phi has and is no link.
/// Also returns whether a cast was among them, for the trace.
fn offset_from(
    func: &IrFunction,
    defs: &DefSites,
    value: Value,
    phi: Value,
    width: usize,
) -> Option<(i64, bool)> {
    let keeps_the_bits = |ty: IrType| ty.is_integer() && (width..=8).contains(&ty.size());
    let mut value = value;
    let mut offset = 0i64;
    let mut cast = false;
    for _ in 0..=MAX_STEP_LINKS {
        if value == phi {
            return Some((offset, cast));
        }
        let (block, at) = defs.one(value)?;
        value = match &func.blocks[block].instructions[at] {
            Instruction::BinOp {
                op: op @ (IrBinOp::Add | IrBinOp::Sub),
                lhs,
                rhs,
                ty,
                ..
            } if keeps_the_bits(*ty) => {
                let (from, by) = match (op, lhs, rhs) {
                    (_, Operand::Value(from), Operand::Const(by)) => (*from, by.to_i64()?),
                    // `k + x`; `k - x` is no step of `x`.
                    (IrBinOp::Add, Operand::Const(by), Operand::Value(from)) => {
                        (*from, by.to_i64()?)
                    }
                    _ => return None,
                };
                offset = match op {
                    IrBinOp::Add => offset.wrapping_add(by),
                    _ => offset.wrapping_sub(by),
                };
                from
            }
            Instruction::Cast {
                src: Operand::Value(from),
                from_ty,
                to_ty,
                ..
            } if keeps_the_bits(*from_ty) && keeps_the_bits(*to_ty) => {
                cast = true;
                *from
            }
            _ => return None,
        };
    }
    None
}

/// Let a loop's test read the value the loop goes round with. Returns the
/// number of tests rewritten.
///
/// `do { … } while (--n);` arrives here as
///
/// ```text
///   H:  n  = phi [n0, entry], [n1, L]
///       …
///   L:  n1 = n - 1
///       c  = n != 1          ; the mid-level form: a zero test of a
///       br c, H, X           ;   difference compares its operands
/// ```
///
/// and everything from here on has to keep two values where the loop has
/// one. The test reads `n` after `n1` exists, so the two are alive at once
/// and cannot share a register; the copy `n = n1` the phi becomes therefore
/// stays a copy, and it belongs on the back edge, which leaves a block with
/// two ways out and is split for it. The result is `lea -1(n), n1; cmp $1,
/// n; je X; mov n1, n; jmp H`: five instructions and two branches a trip for
/// what is `sub $1, n; jne H`.
///
/// Adding one constant to both sides of an equality changes nothing in a
/// ring, so `n != 1` is `n1 != 0` at every width, signed or unsigned,
/// wrapping or not, and the test can read `n1` instead. Then `n` dies where
/// `n1` is made, the two share a register, and the copy goes, together with
/// the block that held it. No value lives longer for it: `next` has to
/// reach the phi, so it is alive at the test either way.
///
/// It is done here, as late as it can be, and not by keeping the mid-level
/// passes from comparing the operands: they find a counted loop by the
/// compare of its counter, and a test written on the old value in the source
/// (`while (n-- != 1)`) is the same case and has no other place to be caught.
///
/// The test comes in two forms:
///
/// * `c = phi ==/!= K`, a compare in the block that branches on it. An
///   ordered test does not commute with a wrapping addition.
/// * The branch reads `phi` itself, which asks `phi != 0`: the test of
///   `while (n--)` is the old value and nothing else. A compare of `next` is
///   appended for it and takes the place of the test the branch made.
///
/// And the step is `phi` plus and minus constants, with casts on the way
/// ([`offset_from`]): `next = phi ± k`; `next = (T)(phi ± k)`, the sum made
/// in `int`, where C computes `unsigned char n; n--`, and cut back; `next =
/// (T)phi ± k`, as the narrowing passes leave that. Every link is an integer
/// that has the phi's bits and no more than 64. Cutting a value to its low
/// bits commutes with addition, so each of them is `phi ± k` modulo the
/// phi's width.
///
/// What is asked beyond that, each for a reason:
///
/// * `next` is what the loop hands the phi (the identity does not need
///   that; it is what makes the old value die and the rewrite worth doing),
///   and it has been computed, on this trip, when the test is. There are
///   two ways to know that it has. This block computes it ahead of the
///   test, wherever it is handed on from: the header of `while (n--)`, step
///   and test at the top and the back edge from the bottom. Or this block
///   is the one that hands it on, wherever it was computed: then the step
///   dominates the end of this block, and from the phi's block there is no
///   way to the test that misses the step, because such a way, behind any
///   way to the phi's block that misses the step (there is one, the phi's
///   block dominates the step and not the reverse), would reach the end of
///   this block without it. Either way the `next` the test reads was made
///   from the `phi` it used to read. The second is the loop whose counter
///   steps at the top of a body of several blocks:
///
///   ```text
///     H:  n  = phi [n0, entry], [n1, L]
///         n1 = n - 1
///         br …, A, B
///     …
///     L:  c  = n != 1
///         br c, H, X
///   ```
/// * each value read on the way (`c`, `phi`, `next`, the links between the
///   two) has one definition. Single assignment says so of every value; it
///   is looked up, not assumed, and a value with two definitions is left
///   alone.
/// * test, phi and `next` are integers of one width of at most 64 bits, so
///   the three agree on which bits there are.
/// * the new constant is no harder to load than the old one.
pub(crate) fn retarget_loop_tests(func: &mut IrFunction) -> usize {
    // A function is looked at closely only if one of its branches tests a
    // phi: directly, or through an equality compare in the branch's own
    // block. Most do not, and are done with here, before the definitions of
    // every value are looked up.
    let mut phis: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            let Instruction::Phi { dest, .. } = inst else {
                break;
            };
            phis.insert(dest.0);
        }
    }
    let tests_a_phi = !phis.is_empty()
        && func.blocks.iter().any(|block| {
            let Terminator::CondBranch {
                cond: Operand::Value(cond),
                ..
            } = &block.terminator
            else {
                return false;
            };
            phis.contains(&cond.0)
                || block.instructions.iter().any(|inst| {
                    matches!(
                        inst,
                        Instruction::Cmp {
                            dest,
                            op: IrCmpOp::Eq | IrCmpOp::Ne,
                            lhs,
                            rhs,
                            ..
                        } if dest == cond
                            && [lhs, rhs].iter().any(
                                |side| matches!(side, Operand::Value(v) if phis.contains(&v.0)),
                            )
                    )
                })
        });
    if !tests_a_phi {
        return 0;
    }
    let defs = DefSites::of(func);

    /// How a block's branch reads the value it tests.
    #[derive(Clone, Copy)]
    enum Reads {
        /// Through the compare at this index of the block.
        Compare(usize),
        /// Directly: the branch asks whether the value is zero.
        Itself,
    }
    /// One test to rewrite: it is to read `next` against `bound`.
    struct Retarget {
        block: usize,
        reads: Reads,
        ty: IrType,
        next: Value,
        bound: IrConst,
        /// For the trace: where the step stands, and whether a cast is
        /// among its links.
        step_in_block: bool,
        cut_back: bool,
    }
    let mut rewrites: Vec<Retarget> = Vec::new();
    for (b, block) in func.blocks.iter().enumerate() {
        let Terminator::CondBranch {
            cond: Operand::Value(cond),
            ..
        } = &block.terminator
        else {
            continue;
        };
        let Some((cond_block, cond_at)) = defs.one(*cond) else {
            continue;
        };
        // The value tested, the constant it is tested against, and how.
        let (phi, bound, reads, test_ty) = match &func.blocks[cond_block].instructions[cond_at] {
            // A compare, in this block: what it is to read has to be there
            // to be read.
            Instruction::Cmp {
                op: IrCmpOp::Eq | IrCmpOp::Ne,
                lhs,
                rhs,
                ty,
                ..
            } if cond_block == b => match (lhs, rhs) {
                (Operand::Value(v), Operand::Const(k)) | (Operand::Const(k), Operand::Value(v)) => {
                    (*v, k.to_i64(), Reads::Compare(cond_at), *ty)
                }
                _ => continue,
            },
            Instruction::Phi { ty, .. } => (*cond, Some(0), Reads::Itself, *ty),
            _ => continue,
        };
        let Some((phi_block, phi_inst)) = defs.one(phi) else {
            continue;
        };
        let Instruction::Phi {
            ty: phi_ty,
            incoming,
            ..
        } = &func.blocks[phi_block].instructions[phi_inst]
        else {
            continue;
        };
        // The value the loop goes round with: what this block hands the
        // phi, or what it has computed ahead of the test for another block
        // to hand on. One value, or there is no telling which.
        let ahead_of_test = |at: usize| match reads {
            Reads::Compare(test) => at < test,
            // The compare to come goes behind everything the block holds.
            Reads::Itself => true,
        };
        let mut next: Option<Value> = None;
        let mut one_value = true;
        for (operand, from) in incoming {
            let handed = *from == block.label;
            let candidate = match operand {
                Operand::Value(v)
                    if handed
                        || defs
                            .one(*v)
                            .is_some_and(|(block, at)| block == b && ahead_of_test(at)) =>
                {
                    *v
                }
                // This block hands the phi a constant: no step.
                _ if handed => {
                    one_value = false;
                    break;
                }
                _ => continue,
            };
            if next.is_some_and(|other| other != candidate) {
                one_value = false;
                break;
            }
            next = Some(candidate);
        }
        let (Some(next), true) = (next, one_value) else {
            continue;
        };
        let Some((next_block, next_at)) = defs.one(next) else {
            continue;
        };
        // Handed on from here and computed here: ahead of the test, then.
        // (Computed in another block it is ahead of it by the argument
        // above.)
        if next_block == b && !ahead_of_test(next_at) {
            continue;
        }
        // The step, and the three that have to agree on which bits there
        // are: the test, the phi, and `next` as its definition types it.
        let width = test_ty.size();
        let next_ty = match &func.blocks[next_block].instructions[next_at] {
            Instruction::BinOp { ty, .. } => *ty,
            Instruction::Cast { to_ty, .. } => *to_ty,
            _ => continue,
        };
        if ![*phi_ty, next_ty, test_ty]
            .iter()
            .all(|t| t.is_integer() && t.size() == width)
            || width > 8
        {
            continue;
        }
        let Some((by, cut_back)) = offset_from(func, &defs, next, phi, width) else {
            continue;
        };
        let Some(bound) = bound else {
            continue;
        };
        // `phi == K` is `phi + k == K + k`, modulo the width; `truncate_i64`
        // keeps the bits the type has.
        let moved = test_ty.truncate_i64(bound.wrapping_add(by));
        // A 64-bit compare takes a sign-extended 32-bit immediate; a
        // constant beyond that costs a register and an instruction.
        let fits_imm32 = |k: i64| i32::try_from(k).is_ok();
        if width == 8 && !fits_imm32(moved) && fits_imm32(bound) {
            continue;
        }
        rewrites.push(Retarget {
            block: b,
            reads,
            ty: test_ty,
            next,
            bound: IrConst::from_i64(moved, test_ty),
            step_in_block: next_block == b,
            cut_back,
        });
    }
    if rewrites.is_empty() {
        return 0;
    }

    let trace = phi_hoist_trace();
    // Only a branch that read its value directly needs a new name.
    let mut fresh = if rewrites
        .iter()
        .any(|rewrite| matches!(rewrite.reads, Reads::Itself))
    {
        func.sound_next_value_id()
    } else {
        func.next_value_id
    };
    for rewrite in &rewrites {
        let block = &mut func.blocks[rewrite.block];
        let was = match rewrite.reads {
            Reads::Compare(at) => {
                let Instruction::Cmp { lhs, rhs, .. } = &mut block.instructions[at] else {
                    continue;
                };
                let (value, constant) = if matches!(lhs, Operand::Value(_)) {
                    (lhs, rhs)
                } else {
                    (rhs, lhs)
                };
                let was = trace.then(|| format!("{value:?} against {constant:?}"));
                *value = Operand::Value(rewrite.next);
                *constant = Operand::Const(rewrite.bound);
                was
            }
            Reads::Itself => {
                let Terminator::CondBranch { cond, .. } = &mut block.terminator else {
                    continue;
                };
                let was = trace.then(|| format!("{cond:?} itself"));
                let dest = Value(fresh);
                fresh += 1;
                *cond = Operand::Value(dest);
                // The compare belongs to the statement the branch does.
                let span = block
                    .source_spans
                    .last()
                    .copied()
                    .unwrap_or_else(crate::common::source::Span::dummy);
                block.push_inst(
                    Instruction::Cmp {
                        dest,
                        op: IrCmpOp::Ne,
                        lhs: Operand::Value(rewrite.next),
                        rhs: Operand::Const(rewrite.bound),
                        ty: rewrite.ty,
                    },
                    span,
                );
                was
            }
        };
        if let Some(was) = was {
            lccc::cli::stderr_line(format_args!(
                "[PHI] loop test of block {}: {} becomes v{} against {:?} (step in {}{})",
                block.label.0,
                was,
                rewrite.next.0,
                rewrite.bound,
                if rewrite.step_in_block {
                    "the block"
                } else {
                    "a dominating block"
                },
                if rewrite.cut_back {
                    ", through a cast"
                } else {
                    ""
                },
            ));
        }
    }
    func.next_value_id = func.next_value_id.max(fresh);
    rewrites.len()
}

/// Module entry [`eliminate_phis`] with an explicit [`CopyOrderPolicy`].
///
/// The production wrapper reads the opt-in gate; the unit tests pass a policy
/// directly so the end-to-end rotation pin can exercise the cycle-accurate
/// resolver without touching the environment.
fn eliminate_phis_with_policy(
    func: &mut IrFunction,
    next_block_id: &mut u32,
    policy: CopyOrderPolicy,
) {
    retarget_loop_tests(func);
    // Phi-web coalescing (see `phi_webs`) on the targets that run the
    // cycle-accurate resolver; narrow-GPR targets keep separate webs for the
    // same register-economy reason they keep the legacy resolver.
    // `CCC_NO_PHI_WEB_COALESCE=1` disables it.
    if policy == CopyOrderPolicy::Acyclic
        && std::env::var("CCC_NO_PHI_WEB_COALESCE").as_deref() != Ok("1")
    {
        super::phi_webs::coalesce_phi_webs(func);
    }
    let mut ctx = PhiElimCtx {
        label_to_idx: func
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.label, i))
            .collect(),
        multi_succ: func.blocks.iter().map(|b| successor_count(b) > 1).collect(),
        is_indirect_branch: func
            .blocks
            .iter()
            .map(|b| matches!(&b.terminator, Terminator::IndirectBranch { .. }))
            .collect(),
        asm_goto_edges: func
            .blocks
            .iter()
            .enumerate()
            .flat_map(|(idx, block)| {
                crate::ir::analysis::asm_goto_successors(block).map(move |label| (idx, label))
            })
            .collect(),
        has_asm_goto: func
            .blocks
            .iter()
            .map(|block| {
                crate::ir::analysis::asm_goto_successors(block)
                    .next()
                    .is_some()
            })
            .collect(),
        pred_copies: FxHashMap::default(),
        self_loop_copies: vec![Vec::new(); func.blocks.len()],
        target_copies: vec![Vec::new(); func.blocks.len()],
        trampolines: Vec::new(),
        trampoline_map: FxHashMap::default(),
        next_block_id,
        next_value: if func.next_value_id > 0 {
            func.next_value_id
        } else {
            func.max_value_id() + 1
        },
        copy_order: policy,
    };

    let block_phis = collect_block_phis(func);

    for (block_idx, phis) in block_phis.iter().enumerate() {
        if phis.is_empty() {
            continue;
        }
        let target_block_id = func.blocks[block_idx].label;
        if phis.len() == 1 {
            emit_single_phi_copies(&phis[0], target_block_id, &mut ctx);
        } else {
            emit_multi_phi_copies(phis, block_idx, target_block_id, func, &mut ctx);
        }
    }

    resolve_self_loop_copies(func, &mut ctx);
    hoist_uniform_edge_copies(func, &mut ctx);
    apply_phi_transformations(func, &mut ctx);
    if ctx.copy_order == CopyOrderPolicy::Acyclic {
        fold_computed_phi_incomings(func);
    }
    func.next_value_id = ctx.next_value;
}

/// Fold computed phi incomings into the copy sequence ("sink to slot").
///
/// After elimination, an edge's copy for a phi whose incoming was *computed*
/// in the predecessor reads a scratch value `v` defined by an ALU instruction
/// in the same block: `v = d + t1` ... `e := v`. Materializing `v` into its
/// own home and then copying costs one extra register move per execution —
/// the "relay" that `tests/regression/check_phi_acyclic_order.sh` pins for
/// `rot()` (SHA-256's rotation shape), and one that kept resurfacing because
/// ordinary coalescing cannot remove it: at the ALU instruction's original
/// position every phi home is still occupied by its old value, so the
/// computed value and the phi destination formally interfere.
///
/// The acyclic planner guarantees readers of a destination precede its copy
/// and (via guard deferral) delays copies that would clobber the
/// computation's operands, so the defining instruction can instead execute
/// directly at its copy's slot, writing the phi destination's home:
/// `f := e` ... `e = d + t1` ... with no relay at all. This performs the
/// move only when an explicit hazard scan proves it legal:
///
/// * the ALU definition is pure and non-trapping ([`is_pure_sinking_op`]),
/// * `v` has exactly one use in the whole function (this copy),
/// * nothing strictly between the definition and the copy writes any of its
///   operands or reads `v`,
/// * nothing after the copy's position in the run reads the phi destination
///   (every reader must consume the *old* value before the fold redefines
///   it — the planner already guarantees this; re-checked here so the fold
///   can never depend on planner internals for its correctness).
///
/// The copy itself becomes a self-move and is dropped. The transformation is
/// policy-gated to the acyclic arm: the legacy arm must keep reproducing the
/// historical shape bit-for-bit for the gate's A/B contract.
/// Collect PhiInfo from all blocks.
fn collect_block_phis(func: &IrFunction) -> Vec<Vec<PhiInfo>> {
    func.blocks
        .iter()
        .map(|block| {
            block
                .instructions
                .iter()
                .filter_map(|inst| {
                    if let Instruction::Phi { dest, incoming, .. } = inst {
                        Some(PhiInfo {
                            dest: *dest,
                            incoming: incoming.clone(),
                        })
                    } else {
                        None
                    }
                })
                .collect()
        })
        .collect()
}

/// Emit copies for a block with a single phi (no temporaries needed).
fn emit_single_phi_copies(phi: &PhiInfo, target_block_id: BlockId, ctx: &mut PhiElimCtx) {
    for (src, pred_label) in &phi.incoming {
        let pred_idx = match ctx.label_to_idx.get(pred_label) {
            Some(&idx) => idx,
            None => continue,
        };
        // Skip self-copies
        if let Operand::Value(v) = src {
            if v.0 == phi.dest.0 {
                continue;
            }
        }
        let copy_inst = Instruction::Copy {
            dest: phi.dest,
            src: *src,
        };
        place_copy(ctx, pred_idx, target_block_id, copy_inst);
    }
}

/// Emit copies for a block with multiple phis, using smart temporary allocation.
/// Shared temporaries are only allocated for phis involved in copy cycles.
fn emit_multi_phi_copies(
    phis: &[PhiInfo],
    block_idx: usize,
    target_block_id: BlockId,
    func: &IrFunction,
    ctx: &mut PhiElimCtx,
) {
    // Collect unique predecessor labels.
    let mut pred_label_set: FxHashSet<BlockId> = FxHashSet::default();
    let mut pred_labels: Vec<BlockId> = Vec::new();
    for phi in phis {
        for (_, pred_label) in &phi.incoming {
            if pred_label_set.insert(*pred_label) {
                pred_labels.push(*pred_label);
            }
        }
    }

    // Precompute per-phi source lookup tables.
    let phi_src_maps: Vec<FxHashMap<BlockId, &Operand>> = phis
        .iter()
        .map(|phi| phi.incoming.iter().map(|(src, pl)| (*pl, src)).collect())
        .collect();

    // Resolve every predecessor edge's parallel copies once. The cycle members
    // need a shared temporary; everything else is emitted directly, in the
    // order the decomposition proved safe.
    let edge_plans = plan_all_edges(
        phis,
        &pred_labels,
        &phi_src_maps,
        &ctx.label_to_idx,
        &func.blocks,
        ctx.copy_order,
    );
    let mut globally_needs_temp: FxHashSet<usize> = FxHashSet::default();
    for (_, plan) in &edge_plans {
        for &i in &plan.cyclic {
            globally_needs_temp.insert(i);
        }
    }

    // Allocate shared temporaries.
    let mut phi_temps: Vec<Option<Value>> = vec![None; phis.len()];
    for &i in &globally_needs_temp {
        phi_temps[i] = Some(Value(ctx.next_value));
        ctx.next_value += 1;
    }

    // Emit target block copies for conflicting phis (temp -> dest).
    for (i, phi) in phis.iter().enumerate() {
        if let Some(tmp) = phi_temps[i] {
            ctx.target_copies[block_idx].push(Instruction::Copy {
                dest: phi.dest,
                src: Operand::Value(tmp),
            });
        }
    }

    // Emit copies for each predecessor edge, in the resolved execution order.
    for (pred_label, plan) in &edge_plans {
        let pred_idx = match ctx.label_to_idx.get(pred_label) {
            Some(&idx) => idx,
            None => continue,
        };
        let edge_copies =
            build_edge_copies(phis, &phi_temps, &phi_src_maps, pred_label, &plan.ordered);
        if std::env::var_os("CCC_DEBUG_PHIELIM").is_some() {
            lccc::cli::stderr_line(format_args!(
                "[PHIELIM] block {} phis={:?} pred {} copies={:?}",
                target_block_id.0,
                phis.iter().map(|p| p.dest.0).collect::<Vec<_>>(),
                pred_label.0,
                edge_copies
            ));
        }
        place_copies(ctx, pred_idx, target_block_id, edge_copies);
    }
}

/// This edge's `(phi dest, source value id)` pairs.
///
/// `None` marks a non-`Value` (constant) operand: it reads no register, so it
/// imposes no ordering constraint and is never part of a cycle.
fn edge_copies_info(
    phis: &[PhiInfo],
    phi_src_maps: &[FxHashMap<BlockId, &Operand>],
    pred_label: &BlockId,
) -> Vec<(u32, Option<u32>)> {
    phis.iter()
        .enumerate()
        .map(|(i, phi)| {
            let src_val_id = phi_src_maps[i].get(pred_label).and_then(|s| {
                if let Operand::Value(v) = *s {
                    Some(v.0)
                } else {
                    None
                }
            });
            (phi.dest.0, src_val_id)
        })
        .collect()
}

/// Resolve the parallel copies of every predecessor edge of one phi block.
///
/// Edges whose predecessor is not in this function are dropped, matching the
/// emission loop's own `label_to_idx` lookup. The plans are computed once and
/// used twice: their union of `cyclic` sets decides which phis get a shared
/// temporary, and each plan's `ordered` list decides the emission sequence on
/// that edge.
fn plan_all_edges(
    phis: &[PhiInfo],
    pred_labels: &[BlockId],
    phi_src_maps: &[FxHashMap<BlockId, &Operand>],
    label_to_idx: &FxHashMap<BlockId, usize>,
    blocks: &[BasicBlock],
    policy: CopyOrderPolicy,
) -> Vec<(BlockId, EdgeCopyPlan)> {
    let mut plans = Vec::new();
    for pred_label in pred_labels {
        let Some(&pred_idx) = label_to_idx.get(pred_label) else {
            continue;
        };
        let copies_info = edge_copies_info(phis, phi_src_maps, pred_label);
        // Computed incomings carry their computation's operand ids as guards
        // so the planner defers copies that would clobber them (see
        // [`plan_edge_copies_guards`]); the sink pass can then fold the
        // definition onto its copy slot instead of through a relay.
        let guards: Vec<FxHashSet<u32>> = copies_info
            .iter()
            .map(|&(_, src)| {
                src.and_then(|src| computed_source_operands(&blocks[pred_idx], src))
                    .unwrap_or_default()
            })
            .collect();
        plans.push((
            *pred_label,
            plan_edge_copies_guards(&copies_info, &guards, policy),
        ));
    }
    plans
}

/// If `v` is defined in `block` by a pure, non-trapping ALU instruction
/// (no memory access, no division — sinking must not move a faulting or
/// observing operation), return that instruction's value operands.
fn computed_source_operands(block: &BasicBlock, v: u32) -> Option<FxHashSet<u32>> {
    block.instructions.iter().find_map(|inst| {
        if let Instruction::BinOp {
            dest: Value(dv),
            op,
            lhs,
            rhs,
            ..
        } = inst
        {
            if *dv == v && is_pure_sinking_op(*op) {
                let mut ops = FxHashSet::default();
                if let Operand::Value(Value(x)) = lhs {
                    ops.insert(*x);
                }
                if let Operand::Value(Value(x)) = rhs {
                    ops.insert(*x);
                }
                return Some(ops);
            }
        }
        None
    })
}

/// Operations whose definition may be sunk across the phi copy sequence:
/// pure register arithmetic that can neither fault nor observe memory.
/// Division and remainder are excluded because `x / 0` and `x % 0` trap;
/// reordering them across copies would move the fault point.
fn is_pure_sinking_op(op: IrBinOp) -> bool {
    matches!(
        op,
        IrBinOp::Add
            | IrBinOp::Sub
            | IrBinOp::Mul
            | IrBinOp::And
            | IrBinOp::Or
            | IrBinOp::Xor
            | IrBinOp::Shl
            | IrBinOp::AShr
            | IrBinOp::LShr
            | IrBinOp::BitTest
            | IrBinOp::RotateLeft
            | IrBinOp::RotateRight
    )
}

/// Build the ordered copy instructions for a single predecessor edge:
/// Pass 1 (temp saves for the cyclic phis), then Pass 2 (direct copies in the
/// execution order [`plan_edge_copies`] proved safe).
fn build_edge_copies(
    phis: &[PhiInfo],
    phi_temps: &[Option<Value>],
    phi_src_maps: &[FxHashMap<BlockId, &Operand>],
    pred_label: &BlockId,
    order: &[usize],
) -> Vec<Instruction> {
    let mut copies = Vec::new();

    // Pass 1: Emit temporary saves for cyclic phis (must come first). Each
    // writes a fresh temporary and reads its source as it is on entry to the
    // edge, so these have to precede every Pass 2 copy — Pass 2 may overwrite
    // those sources. Among themselves they are order-independent.
    for (i, _phi) in phis.iter().enumerate() {
        if let Some(tmp) = phi_temps[i] {
            if let Some(src) = phi_src_maps[i].get(pred_label) {
                copies.push(Instruction::Copy {
                    dest: tmp,
                    src: *(*src),
                });
            }
        }
    }

    // Pass 2: Emit direct copies in the resolved order. A copy whose
    // destination another copy reads is emitted first, which is what makes the
    // sequence equivalent to the simultaneous assignment without a temporary.
    // Indices this edge resolved as cyclic are skipped: they were served in
    // Pass 1 (a phi is temporary-backed on *every* edge once any edge needs
    // it, so `phi_temps` is edge-independent).
    for &i in order {
        let phi = &phis[i];
        if phi_temps[i].is_some() {
            continue;
        }
        let Some(src) = phi_src_maps[i].get(pred_label) else {
            continue;
        };
        if let Operand::Value(v) = *src {
            if v.0 == phi.dest.0 {
                continue; // skip self-copy
            }
        }
        copies.push(Instruction::Copy {
            dest: phi.dest,
            src: *(*src),
        });
    }

    copies
}

/// Where copies travelling along the edge `pred_idx -> target_block_id` land.
enum CopyPlacement {
    /// Appended to the predecessor, before its terminator. Safe when the
    /// predecessor has a single successor: the copies cannot run on any path
    /// other than the one to the phi's block.
    Predecessor,
    /// Buffered for self-loop latch absorption. Decided later, once every phi
    /// contributing to the same backedge has been seen.
    SelfLoop,
    /// A freshly split block carrying only these copies.
    Trampoline,
}

fn classify_placement(
    ctx: &PhiElimCtx,
    pred_idx: usize,
    target_block_id: BlockId,
) -> CopyPlacement {
    if ctx.asm_goto_edges.contains(&(pred_idx, target_block_id))
        || (ctx.has_asm_goto[pred_idx] && !ctx.is_indirect_branch[pred_idx])
    {
        // Nothing goes behind an asm goto.  It ends its block
        // (`lower_inline_asm_stmt`), and the block is left through it ahead
        // of anything put at the block's end: copies placed there ran only
        // when the block fell through, so `asm goto(... : out); f(); out:`
        // reached `out` by the asm goto with whatever the phi homes held.
        // Every edge out of such a block gets a block of its own: the label
        // edges, the terminator's (shared with a label edge to the same
        // target -- the phi has one entry for the block), and the edge of a
        // block that is its own target, whose copies would otherwise be
        // absorbed into it.  (The targets of an indirect branch cannot be
        // rerouted; its edges keep their copies in the block as before.)
        return CopyPlacement::Trampoline;
    }
    if !ctx.multi_succ[pred_idx] || ctx.is_indirect_branch[pred_idx] {
        // No critical edge to split: the predecessor reaches the target and
        // nothing else. (An indirect branch is left alone because its target
        // set is not a single edge we can split.)
        return CopyPlacement::Predecessor;
    }
    match ctx.label_to_idx.get(&target_block_id) {
        Some(&target_idx) if target_idx == pred_idx => CopyPlacement::SelfLoop,
        _ => CopyPlacement::Trampoline,
    }
}

/// Place a single copy instruction, using trampolines for critical edges.
fn place_copy(
    ctx: &mut PhiElimCtx,
    pred_idx: usize,
    target_block_id: BlockId,
    copy_inst: Instruction,
) {
    match classify_placement(ctx, pred_idx, target_block_id) {
        CopyPlacement::Predecessor => {
            ctx.pred_copies.entry(pred_idx).or_default().push(copy_inst);
        }
        CopyPlacement::SelfLoop => ctx.self_loop_copies[pred_idx].push(copy_inst),
        CopyPlacement::Trampoline => {
            let tramp_idx = get_or_create_trampoline(
                &mut ctx.trampoline_map,
                &mut ctx.trampolines,
                pred_idx,
                target_block_id,
                ctx.next_block_id,
            );
            ctx.trampolines[tramp_idx].copies.push(copy_inst);
        }
    }
}

/// Place multiple copy instructions, using trampolines for critical edges.
fn place_copies(
    ctx: &mut PhiElimCtx,
    pred_idx: usize,
    target_block_id: BlockId,
    copies: Vec<Instruction>,
) {
    if copies.is_empty() {
        return;
    }
    match classify_placement(ctx, pred_idx, target_block_id) {
        CopyPlacement::Predecessor => {
            ctx.pred_copies.entry(pred_idx).or_default().extend(copies);
        }
        CopyPlacement::SelfLoop => ctx.self_loop_copies[pred_idx].extend(copies),
        CopyPlacement::Trampoline => {
            let tramp_idx = get_or_create_trampoline(
                &mut ctx.trampoline_map,
                &mut ctx.trampolines,
                pred_idx,
                target_block_id,
                ctx.next_block_id,
            );
            ctx.trampolines[tramp_idx].copies.extend(copies);
        }
    }
}

/// Whether self-loop latch absorption is enabled.
fn self_loop_hoist_enabled() -> bool {
    std::env::var("CCC_NO_PHI_SELFLOOP_HOIST").as_deref() != Ok("1")
}

/// Reads performed by each block, split into instruction reads and terminator
/// reads. The split matters because absorbed copies are appended *after* the
/// last instruction but *before* the terminator.
struct BlockUses {
    /// For each value, the blocks whose *instructions* read it.
    ///
    /// A phi's incoming operands are attributed to the block holding the phi
    /// rather than to the predecessor the edge comes from. That is
    /// deliberately conservative: it can only turn a hoist down, never let an
    /// unsound one through, and it avoids modelling per-edge liveness.
    instr_readers: FxHashMap<u32, FxHashSet<usize>>,
    /// For each value, the blocks whose *terminator* reads it.
    ///
    /// Kept separate from `instr_readers` because absorbed copies are appended
    /// after the last instruction but before the terminator, so the two have
    /// different consequences. Terminator reads must not be folded into
    /// `instr_readers`: a value consumed straight from a predecessor by another
    /// block's terminator (typically `Return(v)`) would then look like it never
    /// left the self-loop block.
    term_readers: FxHashMap<u32, FxHashSet<usize>>,
}

impl BlockUses {
    fn from_blocks(blocks: &[BasicBlock]) -> Self {
        let mut instr_readers: FxHashMap<u32, FxHashSet<usize>> = FxHashMap::default();
        let mut term_readers: FxHashMap<u32, FxHashSet<usize>> = FxHashMap::default();
        for (idx, block) in blocks.iter().enumerate() {
            for inst in &block.instructions {
                inst.for_each_used_value(|v| {
                    instr_readers.entry(v).or_default().insert(idx);
                });
            }
            block.terminator.for_each_used_value(|v| {
                term_readers.entry(v).or_default().insert(idx);
            });
        }
        Self {
            instr_readers,
            term_readers,
        }
    }

    /// True when `value` is read only by `block_idx`'s own instructions (or not
    /// at all), i.e. it is invisible to every other block.
    fn only_read_within(&self, value: u32, block_idx: usize) -> bool {
        match self.instr_readers.get(&value) {
            None => true,
            Some(blocks) => blocks.len() == 1 && blocks.contains(&block_idx),
        }
    }

    /// True when no terminator anywhere reads `value`.
    fn no_terminator_reads(&self, value: u32) -> bool {
        !self.term_readers.contains_key(&value)
    }
}

/// Decide, for one self-loop backedge, whether its phi copies can be appended
/// to the block itself instead of being moved into a split latch block.
///
/// The copies run at the end of `block` on *every* path out of it, so they are
/// safe exactly when overwriting their destinations is unobservable everywhere
/// except the backedge. Three conditions are checked, and all three are
/// necessary:
///
/// 1. The backedge is still present in the terminator. Without it the block is
///    not a self-loop and the buffered copies would simply be wrong.
/// 2. No destination is read by any terminator. For the block's own terminator
///    this is an ordering constraint -- the copies are inserted before it, so a
///    terminator reading a destination would branch on the new value. For every
///    other block's terminator it is the same escape condition as (3), and it
///    has to be checked separately because a value can travel straight from
///    this block into another block's `Return` or branch condition without any
///    instruction in between reading it.
/// 3. No destination is read by any other block's instructions. Reads inside the
///    block are fine because they all precede the appended copies; reads
///    elsewhere are reached through a non-backedge edge where the copies also
///    executed.
///
/// Separately, the copies must form a *commuting* parallel copy: no destination
/// may also be a source. Sequentialising a non-commuting set (a swap, or a
/// shift chain) changes its meaning, and the existing temporary machinery in
/// `emit_multi_phi_copies` is what handles those.
///
/// (A shift chain in reader-first order, `a = b; b = t`, would be sound to
/// absorb as well, and the check was once relaxed to take it. Over the 1,493
/// files of the regression corpus and the 88 benchmark programs that changed
/// none of 835 decisions: where a loop rotates values, something behind the
/// loop reads them, and conditions 2 and 3 keep the back edge split whatever
/// this one says. A weaker check that nothing exercises is not kept.)
///
/// A fifth condition lifts (2) and (3) for one kind of copy: one the block's
/// exit edge makes as well (`copy_repeats_on_exit`). A bottom-tested loop
/// whose result is read behind it has the same `s = s'` on its backedge and
/// on its exit edge once the two phis share a name (`phi_webs`, exit twins),
/// and a copy that runs on every way out of the block runs as well once
/// ahead of the branch: the destination is written before anything outside
/// can read it either way. What is left to rule out lies between the two
/// places -- the block's own terminator and the exit edge's other copies.
/// The exit edge then drops its share, and is not split when that was all it
/// carried.
///
/// A fourth condition is about *profit*, not correctness. Absorbed copies are
/// placed ahead of the compare feeding the branch (`place_pred_copies`), which
/// keeps the compare fusable on every backend. That is impossible when the
/// compare reads a copy destination (it must see the old value), and placing
/// the copies after it separates the compare from its branch: the compare is
/// then materialized and re-tested (x86-64's compare replay declines on the
/// redefined operand, see `x86/codegen/comparison.rs`). That costs more per
/// iteration than the removed latch branch, so the absorption is skipped in
/// exactly that shape and kept wherever removing the latch pays.
fn self_loop_copies_can_hoist(
    block: &BasicBlock,
    copies: &[Instruction],
    uses: &BlockUses,
    block_idx: usize,
    exit_copies: &[Instruction],
) -> bool {
    if copies.is_empty() {
        return false;
    }

    // Condition 1.
    let self_label = block.label;
    let has_self_edge = match &block.terminator {
        Terminator::Branch(t) => *t == self_label,
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => *true_label == self_label || *false_label == self_label,
        // A switch can carry the backedge too, in a case arm or as the default.
        // Conditions 2 and 3 below are terminator-shape independent, and a
        // destination that no other block reads is unobservable on however many
        // edges leave the block, so a switch needs no special handling beyond
        // finding the self edge.
        Terminator::Switch { cases, default, .. } => {
            *default == self_label || cases.iter().any(|&(_, t)| t == self_label)
        }
        Terminator::Return(_) | Terminator::Unreachable | Terminator::IndirectBranch { .. } => {
            false
        }
    };
    if !has_self_edge {
        return false;
    }

    let mut dests: FxHashSet<u32> = FxHashSet::default();
    let mut srcs: FxHashSet<u32> = FxHashSet::default();
    for copy in copies {
        let Instruction::Copy { dest, src } = copy else {
            return false;
        };
        // A repeated destination means the set is not a parallel copy.
        if !dests.insert(dest.0) {
            return false;
        }
        if let Operand::Value(v) = src {
            srcs.insert(v.0);
        }
    }

    // Commuting check.
    if dests.iter().any(|d| srcs.contains(d)) {
        return false;
    }

    let mut read_by_own_terminator: FxHashSet<u32> = FxHashSet::default();
    block.terminator.for_each_used_value(|v| {
        read_by_own_terminator.insert(v);
    });
    for copy in copies {
        let Instruction::Copy { dest, .. } = copy else {
            return false;
        };
        // Condition 5: the block's other edge makes the same copy, so the
        // destination is written on every way out and only what lies between
        // the copy's new place and its old ones can tell the difference.
        if copy_repeats_on_exit(copy, exit_copies) {
            if read_by_own_terminator.contains(&dest.0) {
                return false;
            }
            continue;
        }
        // Conditions 2 and 3: the destination must be invisible everywhere
        // except this block's own instructions.
        if !uses.no_terminator_reads(dest.0) || !uses.only_read_within(dest.0, block_idx) {
            return false;
        }
    }

    // Condition 4 (profit): keep the branch condition fusable.
    if let Terminator::CondBranch {
        cond: Operand::Value(cond),
        ..
    } = &block.terminator
    {
        for inst in &block.instructions {
            let Instruction::Cmp { dest, lhs, rhs, .. } = inst else {
                continue;
            };
            if dest.0 != cond.0 {
                continue;
            }
            let touched = [lhs, rhs].iter().any(|op| match op {
                Operand::Value(v) => dests.contains(&v.0),
                Operand::Const(_) => false,
            });
            if touched {
                return false;
            }
        }
    }

    true
}

/// Whether `copy` (one of a self-loop's backedge copies) is made by the
/// block's exit edge as well, in a way that lets one execution ahead of the
/// branch stand for both.
///
/// `exit_copies` is the exit edge's sequence as it will run. Moving its
/// `d = s` to the front of that sequence (and from there above the branch)
/// changes nothing when no other copy of the sequence reads or writes `d`
/// and none writes `s`.
fn copy_repeats_on_exit(copy: &Instruction, exit_copies: &[Instruction]) -> bool {
    let Instruction::Copy { dest, src } = copy else {
        return false;
    };
    let mut repeated = 0usize;
    for other in exit_copies {
        let Instruction::Copy { dest: d, src: s } = other else {
            return false;
        };
        if d == dest && s == src {
            repeated += 1;
            continue;
        }
        let reads_dest = matches!(s, Operand::Value(v) if v == dest);
        let writes_src = matches!(src, Operand::Value(v) if v == d);
        if d == dest || reads_dest || writes_src {
            return false;
        }
    }
    repeated == 1
}

/// The trampoline of the one edge that leaves the self-loop block
/// `block_idx`, when the block ends in a two-way branch and that edge was
/// split for copies of its own.
fn exit_edge_trampoline(block: &BasicBlock, block_idx: usize, ctx: &PhiElimCtx) -> Option<usize> {
    let Terminator::CondBranch {
        true_label,
        false_label,
        ..
    } = &block.terminator
    else {
        return None;
    };
    let exit = match (*true_label == block.label, *false_label == block.label) {
        (true, false) => *false_label,
        (false, true) => *true_label,
        _ => return None,
    };
    if ctx.asm_goto_edges.contains(&(block_idx, exit)) {
        return None;
    }
    ctx.trampoline_map.get(&(block_idx, exit)).copied()
}

/// Resolve every buffered self-loop backedge: absorb the copies into the block
/// when that is provably safe, otherwise split the edge as before.
fn resolve_self_loop_copies(func: &IrFunction, ctx: &mut PhiElimCtx) {
    if ctx.self_loop_copies.iter().all(Vec::is_empty) {
        return;
    }
    let enabled = self_loop_hoist_enabled();
    let uses = BlockUses::from_blocks(&func.blocks);
    let buffered = std::mem::take(&mut ctx.self_loop_copies);
    let debug = phi_hoist_trace();
    // Exit trampolines whose every copy was absorbed with the backedge's.
    let mut emptied: FxHashSet<usize> = FxHashSet::default();

    for (block_idx, copies) in buffered.into_iter().enumerate() {
        if copies.is_empty() {
            continue;
        }
        let target_block_id = func.blocks[block_idx].label;
        let exit = exit_edge_trampoline(&func.blocks[block_idx], block_idx, ctx);
        let hoist = enabled
            && self_loop_copies_can_hoist(
                &func.blocks[block_idx],
                &copies,
                &uses,
                block_idx,
                exit.map_or(&[], |t| &ctx.trampolines[t].copies),
            );
        if debug {
            let dests: Vec<String> = copies
                .iter()
                .map(|c| match c {
                    Instruction::Copy { dest, src } => format!("v{}={:?}", dest.0, src),
                    _ => "?".to_string(),
                })
                .collect();
            lccc::cli::stderr_line(format_args!(
                "[PHI] self-loop block {}: {} ({} copies: {})",
                target_block_id.0,
                if hoist { "absorb" } else { "split" },
                copies.len(),
                dests.join(", ")
            ));
        }
        if hoist {
            // The exit edge's share of the absorbed copies has run by the
            // time the edge is taken; an edge left with nothing to copy is
            // not split at all.
            if let Some(t) = exit {
                let on_exit = &mut ctx.trampolines[t].copies;
                on_exit.retain(|other| {
                    !copies.iter().any(|own| match (own, other) {
                        (
                            Instruction::Copy { dest: d1, src: s1 },
                            Instruction::Copy { dest: d2, src: s2 },
                        ) => d1 == d2 && s1 == s2,
                        _ => false,
                    })
                });
                if on_exit.is_empty() {
                    emptied.insert(t);
                }
            }
            ctx.pred_copies.entry(block_idx).or_default().extend(copies);
        } else {
            let tramp_idx = get_or_create_trampoline(
                &mut ctx.trampoline_map,
                &mut ctx.trampolines,
                block_idx,
                target_block_id,
                ctx.next_block_id,
            );
            ctx.trampolines[tramp_idx].copies.extend(copies);
        }
    }

    if emptied.is_empty() {
        return;
    }
    let mut index = 0usize;
    ctx.trampolines.retain(|_| {
        let keep = !emptied.contains(&index);
        index += 1;
        keep
    });
    ctx.trampoline_map = ctx
        .trampolines
        .iter()
        .enumerate()
        .map(|(i, t)| ((t.pred_idx, t.old_target), i))
        .collect();
}

/// Apply all phi elimination transformations to the function:
/// remove phis, insert copies, retarget terminators, add trampolines.
/// Uniform edge copies: when every outgoing edge of a branching block was
/// split for the *same* copy sequence, run that sequence once before the
/// branch instead.
///
/// A tail-duplicated merge loop's compare block ends `t = a; a = next` on both
/// the edge back to its own arm and the edge into the other arm; splitting
/// both edges costs two trampolines and an unconditional jump per iteration
/// (GCC emits `mov; mov; cmp; jnb .L7`).  Executing an identical parallel copy
/// on every outgoing edge equals executing it at the end of the block, as long
/// as the terminator does not read a destination.  Refused when the block
/// already carries copies of its own, ends in an indirect branch, has
/// `asm goto` edges, or any edge is not split for the identical sequence.
fn hoist_uniform_edge_copies(func: &IrFunction, ctx: &mut PhiElimCtx) {
    let mut by_pred: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
    for (ti, t) in ctx.trampolines.iter().enumerate() {
        by_pred.entry(t.pred_idx).or_default().push(ti);
    }
    let mut dropped: FxHashSet<usize> = FxHashSet::default();
    for (pred, tramps) in by_pred {
        let block = &func.blocks[pred];
        if tramps.len() < 2
            || ctx.is_indirect_branch[pred]
            || ctx.pred_copies.contains_key(&pred)
            || block
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::InlineAsm { .. }))
        {
            continue;
        }
        let mut succs: Vec<BlockId> = Vec::new();
        match &block.terminator {
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => succs.extend([*true_label, *false_label]),
            Terminator::Switch { cases, default, .. } => {
                succs.push(*default);
                succs.extend(cases.iter().map(|&(_, l)| l));
            }
            _ => continue,
        }
        succs.sort_by_key(|b| b.0);
        succs.dedup();
        let mut targets: Vec<BlockId> = tramps
            .iter()
            .map(|&t| ctx.trampolines[t].old_target)
            .collect();
        targets.sort_by_key(|b| b.0);
        targets.dedup();
        let first = &ctx.trampolines[tramps[0]].copies;
        let uniform = targets == succs
            && first.iter().all(|c| matches!(c, Instruction::Copy { .. }))
            && tramps
                .iter()
                .all(|&t| copies_equal(&ctx.trampolines[t].copies, first));
        if !uniform {
            continue;
        }
        let mut read_by_term = false;
        block.terminator.for_each_used_value(|v| {
            read_by_term |= first.iter().any(|c| c.dest().is_some_and(|d| d.0 == v));
        });
        if read_by_term {
            continue;
        }
        ctx.pred_copies.insert(pred, first.clone());
        dropped.extend(tramps);
    }
    if dropped.is_empty() {
        return;
    }
    let mut index = 0usize;
    ctx.trampolines.retain(|_| {
        let keep = !dropped.contains(&index);
        index += 1;
        keep
    });
    // `trampoline_map` indices are stale after the retain and the map is not
    // read past this point; clear it rather than renumber.
    ctx.trampoline_map.clear();
}

/// Two copy sequences are the same parallel assignment, element by element.
fn copies_equal(a: &[Instruction], b: &[Instruction]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| match (x, y) {
            (Instruction::Copy { dest: d1, src: s1 }, Instruction::Copy { dest: d2, src: s2 }) => {
                d1 == d2 && s1 == s2
            }
            _ => false,
        })
}

fn apply_phi_transformations(func: &mut IrFunction, ctx: &mut PhiElimCtx) {
    for (block_idx, block) in func.blocks.iter_mut().enumerate() {
        // Remove phi instructions (and their spans/provenance) in one pass.
        block.retain_insts(|inst| !matches!(inst, Instruction::Phi { .. }));

        // Prepend target copies (these go at the start, replacing the phis)
        if !ctx.target_copies[block_idx].is_empty() {
            let copies = ctx.target_copies[block_idx].clone();
            block.insert_insts(0, copies, crate::common::source::Span::dummy());
        }

        // Insert predecessor copies before the terminator, ahead of the
        // compare that feeds it (`place_pred_copies`).
        if let Some(copies) = ctx.pred_copies.remove(&block_idx) {
            place_pred_copies(block, copies);
        }
    }

    // Retarget predecessors that need trampolines
    for trampoline in &ctx.trampolines {
        retarget_block_edges(
            &mut func.blocks[trampoline.pred_idx],
            trampoline.old_target,
            trampoline.label,
        );
        func.terminator_spans.retarget(
            func.blocks[trampoline.pred_idx].label,
            trampoline.old_target,
            trampoline.label,
        );
    }

    // Append trampoline blocks to the function
    for trampoline in std::mem::take(&mut ctx.trampolines) {
        let num_copies = trampoline.copies.len();
        func.blocks.push(BasicBlock {
            label: trampoline.label,
            instructions: trampoline.copies,
            source_spans: vec![crate::common::source::Span::dummy(); num_copies],
            source_inline_ids: Vec::new(),
            terminator: Terminator::Branch(trampoline.branch_target),
        });
    }
}

/// Insert a block's predecessor copies (its outgoing phi copies: absorbed
/// self-loop backedges, uniform edge copies, single-successor edges) before
/// its terminator -- and, when the terminator is a `CondBranch` fed by a
/// compare, sink that compare's chain below them.
///
/// Every backend fuses `Cmp` -> `CondBranch` (`cmp; jcc`, `cmp; b.cond`,
/// `bcc`) only when the compare is adjacent to the branch
/// (`detect_cmp_branch_fusion`). Copies appended after the compare break the
/// adjacency, and so does an instruction scheduled between them (a pointer
/// induction step `p += 4` after the exit test): the compare is then
/// materialized as a boolean and re-tested -- on i686, where phi webs are not
/// coalesced (see `eliminate_phis`), `setcc; movzbl; test` in every rotated
/// loop latch, and `cset`/`slt` plus a register branch on AArch64 and RISC-V.
/// x86-64 recovered most of these with its compare replay, which re-reads the
/// operands at the branch.
///
/// The block becomes `[other instructions] [copies] [compare chain]` when
/// `flag_tail` proves that order equivalent; otherwise the copies are appended
/// as before. Chain reads of a copy source are rewritten to that copy's
/// destination (same value after the copy), so the source still dies at the
/// copy and the two remain coalescable: the move adds no interference.
/// Source spans are permuted with their instructions; a block whose spans are
/// not index-aligned keeps the append order (its spans cannot be permuted).
fn place_pred_copies(block: &mut BasicBlock, copies: Vec<Instruction>) {
    use crate::common::source::Span;
    let num_copies = copies.len();
    // The chain is only moved when every instruction's span moves with it.
    let spans_aligned = block.source_spans.is_empty() || block.spans_parallel();
    let chain = if spans_aligned {
        flag_tail(block, &copies)
    } else {
        None
    };
    let Some(chain) = chain else {
        block.extend_insts(copies, Span::dummy());
        return;
    };
    // value -> the destination of the (unique) copy reading it.
    let mut carried: FxHashMap<u32, Value> = FxHashMap::default();
    for copy in &copies {
        if let Instruction::Copy {
            dest,
            src: Operand::Value(v),
        } = copy
        {
            carried.entry(v.0).or_insert(*dest);
        }
    }
    let in_chain = |i: usize| chain.binary_search(&i).is_ok();
    let (spans, ids) = block.take_parallel_debug();
    let tracked = !spans.is_empty();
    let track_ids = !ids.is_empty();
    let inline_at = |i: usize| {
        ids.get(i)
            .copied()
            .unwrap_or(crate::ir::instruction::NO_INLINE_INSTANCE)
    };
    let old = std::mem::take(&mut block.instructions);
    let mut insts = Vec::with_capacity(old.len() + num_copies);
    let mut new_spans = Vec::with_capacity(if tracked { insts.capacity() } else { 0 });
    let mut new_ids = Vec::with_capacity(if tracked { insts.capacity() } else { 0 });
    let mut tail = Vec::with_capacity(chain.len());
    let mut tail_spans = Vec::with_capacity(if tracked { chain.len() } else { 0 });
    let mut tail_ids = Vec::with_capacity(if tracked { chain.len() } else { 0 });
    for (i, mut inst) in old.into_iter().enumerate() {
        let span = spans.get(i).copied();
        if !in_chain(i) {
            insts.push(inst);
            new_spans.extend(span);
            if track_ids {
                new_ids.push(inline_at(i));
            }
            continue;
        }
        inst.for_each_operand_mut(|op| {
            if let Operand::Value(v) = op
                && let Some(d) = carried.get(&v.0)
            {
                *v = *d;
            }
        });
        inst.for_each_value_use_mut(|v| {
            if let Some(d) = carried.get(&v.0) {
                *v = *d;
            }
        });
        tail.push(inst);
        tail_spans.extend(span);
        if track_ids {
            tail_ids.push(inline_at(i));
        }
    }
    insts.extend(copies);
    if tracked {
        new_spans.extend(std::iter::repeat_n(Span::dummy(), num_copies));
    }
    if track_ids {
        new_ids.extend(std::iter::repeat_n(
            crate::ir::instruction::NO_INLINE_INSTANCE,
            num_copies,
        ));
    }
    insts.extend(tail);
    new_spans.extend(tail_spans);
    if track_ids {
        new_ids.extend(tail_ids);
    } else {
        // Span-only blocks do not track inline provenance; handing
        // replace_insts_inline a nonempty id list would turn tracking on for a
        // non-debug build and drift the array on the next span-only edit.
        new_ids.clear();
    }
    block.replace_insts_inline(insts, new_spans, new_ids);
}

/// Ascending indices of the compare chain feeding `block`'s `CondBranch` when
/// `[others] [copies] [chain]` is equivalent to `[block] [copies]`, else
/// `None` (the copies go last, as before).
///
/// The chain is the `Cmp` (or `BitTest`) defining the condition plus the
/// `Copy`/integer-`Cast` links from it to the condition -- exactly what the
/// backends fuse into the branch. Found walking back from the end of the
/// block; an instruction defining the value being traced that is not such a
/// link means the condition is not a compare chain.
///
/// Members may be interleaved with other instructions; sinking a member `c`
/// past a later non-member `x` (copies included) is sound when
/// * `x` does not read `c`'s result (no use before its new definition),
/// * `x` defines neither `c`'s result nor a value `c` reads -- explicit even
///   though SSA implies it, because this IR already carries phi copies --,
/// * `x` is not a call or inline asm: the chain is pure, so it may pass any
///   memory operation, but moving a compare below a call would stretch its
///   operands across the call for no fusion the call does not already break.
///
/// For the copies this amounts to the group conditions: no chain instruction
/// reads or defines a copy destination (the chain keeps seeing the values it
/// saw before the copies ran) or defines a copy source (the copies keep
/// reading the values they read at the end of the block). Each destination is
/// written once (checked), so after the group every destination equals its
/// source whenever that source is not itself a destination; chain reads of
/// such a source may use the destination instead. That holds for sequential
/// groups with cycle temporaries too: a value that is a destination is never
/// rewritten, because a chain reading it refuses.
fn flag_tail(block: &BasicBlock, copies: &[Instruction]) -> Option<Vec<usize>> {
    let Terminator::CondBranch {
        cond: Operand::Value(cond),
        ..
    } = &block.terminator
    else {
        return None;
    };
    let insts = &block.instructions;
    let mut wanted = cond.0;
    let mut chain = Vec::new();
    let mut at = insts.len();
    loop {
        at = at.checked_sub(1)?;
        let inst = &insts[at];
        match inst {
            Instruction::Cmp { dest, .. }
            | Instruction::BinOp {
                dest,
                op: IrBinOp::BitTest,
                ..
            } if dest.0 == wanted => {
                chain.push(at);
                break;
            }
            Instruction::Copy {
                dest,
                src: Operand::Value(src),
            } if dest.0 == wanted => {
                chain.push(at);
                wanted = src.0;
            }
            Instruction::Cast {
                dest,
                src: Operand::Value(src),
                from_ty,
                to_ty,
            } if dest.0 == wanted && from_ty.is_integer() && to_ty.is_integer() => {
                chain.push(at);
                wanted = src.0;
            }
            _ if inst.dest().is_some_and(|d| d.0 == wanted) => return None,
            Instruction::Call { .. }
            | Instruction::CallIndirect { .. }
            | Instruction::InlineAsm { .. } => return None,
            _ => {}
        }
    }
    chain.reverse();
    let mut dests: FxHashSet<u32> = FxHashSet::default();
    let mut srcs: FxHashSet<u32> = FxHashSet::default();
    for copy in copies {
        let Instruction::Copy { dest, src } = copy else {
            return None;
        };
        if !dests.insert(dest.0) {
            return None;
        }
        if let Operand::Value(v) = src {
            srcs.insert(v.0);
        }
    }
    for &c in &chain {
        let member = &insts[c];
        let result = member.dest()?.0;
        let reads = member.used_values();
        // The copies, which end up between every member's old and new place.
        if dests.contains(&result) || srcs.contains(&result) {
            return None;
        }
        if reads.iter().any(|v| dests.contains(v)) {
            return None;
        }
        // Later non-members this member sinks past.
        for (j, x) in insts.iter().enumerate().skip(c + 1) {
            if chain.binary_search(&j).is_ok() {
                continue;
            }
            if x.used_values().contains(&result) {
                return None;
            }
            if x.dest()
                .is_some_and(|d| d.0 == result || reads.contains(&d.0))
            {
                return None;
            }
        }
    }
    Some(chain)
}

fn fold_computed_phi_incomings(func: &mut IrFunction) {
    use crate::ir::analysis;
    use Instruction as I;

    // Sink computed phi-incoming definitions to their copy slots.
    //
    // After elimination an edge copy for a phi whose incoming was *computed*
    // in the predecessor reads a scratch value `v` defined by an ALU
    // instruction in the same block: `v = d + t1` ... `e := v`. The add runs
    // where it was first scheduled, so `v`'s live interval spans every phi
    // home still holding its old value; the allocator must give `v` its own
    // home and the copy becomes a real register move per execution — the
    // "relay" that `tests/regression/check_phi_acyclic_order.sh` pins for
    // `rot()` (SHA-256's rotation shape).
    //
    // The sink is only profitable — and only attempted — when a *pending
    // relay into the same phi home* sits after the definition (same block,
    // later position, or a block the defining block strictly dominates).
    // Such a relay is why ordinary coalescing must refuse to seat `v` in
    // the phi home: the def would clobber the home before the earlier
    // incoming's copy drained it. When no other incoming copy to the home
    // follows the definition (the preheader-only shape that a simple loop
    // accumulator produces), the allocator already merges `v` into the home
    // across the disjoint interval, the relay was never real, and sinking
    // would only move the computation away from its operands — measured
    // `spectral_norm` lost `vfmadd231sd` (mul and add separated across the
    // body/latch boundary) for zero copies saved.
    //
    // Moving the definition (a pure, non-trapping ALU op) to directly before
    // its copy is semantics-preserving when nothing between the two positions
    // reads `v` or writes an operand of the computation, and it collapses
    // `v`'s interval to the two instructions before the copy. The register
    // allocator's copy coalescer unions `Copy dest <- Value src` pairs with
    // disjoint intervals, so `v` then shares the phi home, the edge copy
    // becomes a same-home no-op at emission, and the relay disappears —
    // without breaking the post-elimination def discipline (one `Copy` per
    // edge) that the SSA validator enforces: the IR def structure is
    // untouched; only the position of a pure computation moved.
    //
    // Candidates are processed BOTTOM-UP (farthest slot first): chained
    // computations (`a = t1 + t2` reading `t1` that `e = d + t1` also
    // relays) move their reader below the upstream relay's slot first, which
    // empties the upstream hazard window and lets both fold. Each fold moves
    // the parallel `source_spans` entry with its instruction.
    //
    // The pending-relay test needs the CFG's dominator tree; the fold never
    // changes edges, so it is computed once per function.
    let num_blocks = func.blocks.len();
    let label_to_idx = analysis::build_label_map(func);
    let (preds, succs) = analysis::build_cfg(func, &label_to_idx);
    let idom = analysis::compute_dominators(num_blocks, &preds, &succs);
    // O(1) strict dominance (dominator-tree Euler intervals; unreachable
    // blocks are their own roots, so nothing else dominates them).
    let dom = analysis::DominanceChecker::new(num_blocks, &idom);
    // value id -> blocks whose instructions read or write it, ascending and
    // deduplicated.  The fold only moves an instruction within its own
    // block, so this stays exact for the whole sweep.  The dominated-region
    // test below then visits only the few blocks touching the phi home,
    // instead of every block with an idom-chain walk each (cubic in a long
    // sequence of loops).
    let mut touch_blocks: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    for (bj, blk) in func.blocks.iter().enumerate() {
        let bj = bj as u32;
        let mut note = |id: u32| {
            let v = touch_blocks.entry(id).or_default();
            if v.last() != Some(&bj) {
                v.push(bj);
            }
        };
        for inst in &blk.instructions {
            if let Some(Value(d)) = inst.dest() {
                note(d);
            }
            inst.for_each_used_value(&mut note);
        }
    }
    for bi in 0..func.blocks.len() {
        // Candidates: every Copy in the block whose source is defined by a
        // pure ALU instruction in the same block. The whole block is scanned
        // (not just the trailing run) because a previously sunk definition
        // splits the trailing Copy run.
        let mut candidates: Vec<(Value, Value)> = Vec::new();
        {
            let block = &func.blocks[bi];
            for inst in &block.instructions {
                if let I::Copy {
                    dest,
                    src: Operand::Value(v),
                } = inst
                {
                    if v.0 != dest.0 {
                        candidates.push((*dest, *v));
                    }
                }
            }
        }
        for (dest, v) in candidates.iter().rev() {
            // ── decision phase (immutable) ──────────────────────────────
            let decision: Option<(usize, usize)> = (|| {
                let block = &func.blocks[bi];
                let copy_pos = block.instructions.iter().position(|inst| {
                    matches!(inst, I::Copy { dest: d, src: Operand::Value(s) }
                        if *d == *dest && *s == *v)
                })?;
                // The computed definition must live in this block, above the
                // copy.
                let def_pos = block.instructions[..copy_pos].iter().position(|inst| {
                    matches!(inst, I::BinOp { dest: Value(d), op, .. }
                        if *d == v.0 && is_pure_sinking_op(*op))
                })?;
                // Operand ids of the computation (constants guard nothing).
                let operands: Vec<u32> = match &block.instructions[def_pos] {
                    I::BinOp { lhs, rhs, .. } => [&lhs, &rhs]
                        .into_iter()
                        .filter_map(|o| match o {
                            Operand::Value(Value(x)) => Some(*x),
                            _ => None,
                        })
                        .collect(),
                    _ => unreachable!("def_pos was matched as BinOp"),
                };
                // PROFITABILITY: sink only if the phi home `dest` is still
                // *live across the window* — some instruction between the
                // definition and the relay copy (same block), or anywhere in
                // a block the defining block strictly dominates, reads or
                // writes the home. That pending access is exactly what
                // forces `v` into a scratch home: seating `v` directly in
                // the home at its definition would clobber the old value
                // before its reader drains it (the `25 <- 24` rotation copy
                // pinned by check_phi_acyclic_order) or be clobbered by the
                // home's next writer. When the home is dead across the
                // window (a simple loop accumulator whose only other
                // incoming copy lives in the preheader), the allocator's
                // coalescer already seats `v` in the home across the
                // disjoint interval, the relay was never a real move, and
                // sinking would only distance the computation from its
                // operands (measured: `spectral_norm` decomposed
                // vfmadd231sd into mul+add for zero copies saved).
                let mut pending_relay =
                    block.instructions[def_pos + 1..copy_pos]
                        .iter()
                        .any(|inst| {
                            inst.dest().is_some_and(|Value(d)| d == dest.0) || {
                                let mut reads = false;
                                inst.for_each_used_value(|id| {
                                    if id == dest.0 {
                                        reads = true;
                                    }
                                });
                                reads
                            }
                        });
                if !pending_relay {
                    // Conservative dominated-region sweep: a rotation relay
                    // that the planner parked in a trampoline reads the home
                    // after this block ends. C1 itself lives in this block,
                    // so the sweep cannot match the relay we are folding.
                    pending_relay = touch_blocks.get(&dest.0).is_some_and(|blocks| {
                        blocks
                            .iter()
                            .any(|&bj| dom.strictly_dominates(bi, bj as usize))
                    });
                }
                if !pending_relay {
                    return None;
                }
                // Hazard scan over everything strictly between the definition
                // and the copy:
                // * a reader of `v` there would read past its (moved)
                //   definition — use before def;
                // * a writer of an operand would change what the computation
                //   sees after the move;
                // * a second definition of `v` in the window would reorder
                //   two defs of one value.
                // `dest()` is the exhaustive defined-value helper (calls
                // included), so no writing variant can slip through.
                let hazard = block.instructions[def_pos + 1..copy_pos]
                    .iter()
                    .any(|inst| {
                        let mut bad = inst
                            .dest()
                            .is_some_and(|Value(d)| d == v.0 || operands.contains(&d));
                        inst.for_each_used_value(|id| {
                            if id == v.0 {
                                bad = true;
                            }
                        });
                        bad
                    });
                if hazard {
                    return None;
                }
                Some((copy_pos, def_pos))
            })();
            let Some((copy_pos, def_pos)) = decision else {
                continue;
            };

            // ── mutation phase ───────────────────────────────────────────
            let block = &mut func.blocks[bi];
            // Position-only mutation: the definition (and the IR def set)
            // is unchanged; the sink merely relocates it.
            let span = block.span_at(def_pos);
            let inline_id = block.inline_id_at(def_pos);
            let def_inst = block.remove_inst(def_pos);
            // Definition removal shifted the copy down one slot; the
            // definition lands directly before it.
            let copy_pos = copy_pos - 1;
            block.insert_inst_inline(copy_pos, def_inst, span, inline_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blk(label: u32, insts: Vec<Instruction>, term: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions: insts,
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
            terminator: term,
        }
    }

    fn cp(d: u32, s: u32) -> Instruction {
        Instruction::Copy {
            dest: Value(d),
            src: Operand::Value(Value(s)),
        }
    }

    fn cmp9(lhs: u32) -> Instruction {
        Instruction::Cmp {
            dest: Value(9),
            op: crate::ir::reexports::IrCmpOp::Ne,
            lhs: Operand::Value(Value(lhs)),
            rhs: Operand::Const(crate::ir::reexports::IrConst::I64(0)),
            ty: crate::common::types::IrType::I32,
        }
    }

    fn cond(cond: u32, t: u32, f: u32) -> Terminator {
        Terminator::CondBranch {
            cond: Operand::Value(Value(cond)),
            true_label: BlockId(t),
            false_label: BlockId(f),
        }
    }

    /// The shape latch absorption exists for: a rotated loop whose
    /// next-iteration values are distinct SSA values computed last.
    fn rotated_loop() -> Vec<BasicBlock> {
        vec![
            blk(1, vec![cp(13, 10), cp(15, 11), cp(7, 13)], cond(9, 1, 2)),
            blk(2, vec![cp(8, 7)], Terminator::Return(None)),
        ]
    }

    fn hoists(blocks: &[BasicBlock], block_idx: usize, copies: &[Instruction]) -> bool {
        let uses = BlockUses::from_blocks(blocks);
        self_loop_copies_can_hoist(&blocks[block_idx], copies, &uses, block_idx, &[])
    }

    fn hoists_with_exit(
        blocks: &[BasicBlock],
        block_idx: usize,
        copies: &[Instruction],
        exit_copies: &[Instruction],
    ) -> bool {
        let uses = BlockUses::from_blocks(blocks);
        self_loop_copies_can_hoist(&blocks[block_idx], copies, &uses, block_idx, exit_copies)
    }

    /// `case 4: case 6: v = 30; break;` with the empty case blocks threaded:
    /// the switch block reaches the join twice (and once more by default) and
    /// owns ONE phi entry there. Each of those edges has to pass the copy.
    /// Only the first used to be sent through the trampoline; the others
    /// reached the join with whatever the phi's home held (`queue()` in
    /// tests/regression/switch_break_cases_covered_default.c returned 6 for
    /// 36).
    #[test]
    fn every_switch_edge_into_a_phi_target_passes_the_copies() {
        for policy in [CopyOrderPolicy::Acyclic, CopyOrderPolicy::Legacy] {
            let mut func = IrFunction::new(
                "multi_edge".to_string(),
                crate::common::types::IrType::I32,
                Vec::new(),
                false,
            );
            let konst = |v: i32| Operand::Const(crate::ir::reexports::IrConst::I32(v));
            func.blocks = vec![
                blk(
                    0,
                    vec![cmp9(1)],
                    Terminator::Switch {
                        val: Operand::Value(Value(9)),
                        cases: vec![
                            (4, BlockId(2)),
                            (5, BlockId(1)),
                            (6, BlockId(2)),
                            (7, BlockId(2)),
                        ],
                        default: BlockId(2),
                        ty: crate::common::types::IrType::I32,
                    },
                ),
                blk(1, Vec::new(), Terminator::Branch(BlockId(2))),
                blk(
                    2,
                    vec![Instruction::Phi {
                        dest: Value(20),
                        ty: crate::common::types::IrType::I32,
                        incoming: vec![(konst(30), BlockId(0)), (konst(40), BlockId(1))],
                    }],
                    Terminator::Return(Some(Operand::Value(Value(20)))),
                ),
            ];
            func.next_value_id = 64;
            func.next_label = 3;
            let mut next_block_id = 3;
            eliminate_phis_with_policy(&mut func, &mut next_block_id, policy);

            let Terminator::Switch { cases, default, .. } = &func.blocks[0].terminator else {
                panic!("{policy:?}: the switch stays a switch");
            };
            let edge = *default;
            assert_ne!(edge, BlockId(2), "{policy:?}: the default passes the copy");
            for &(value, target) in cases {
                if value == 5 {
                    assert_eq!(target, BlockId(1), "{policy:?}: case 5 keeps its block");
                } else {
                    assert_eq!(target, edge, "{policy:?}: case {value} passes the copy");
                }
            }
            let trampoline = func
                .blocks
                .iter()
                .find(|b| b.label == edge)
                .unwrap_or_else(|| panic!("{policy:?}: the edge block exists"));
            assert!(
                matches!(trampoline.terminator, Terminator::Branch(BlockId(2))),
                "{policy:?}"
            );
            assert!(
                trampoline.instructions.iter().any(|i| matches!(
                    i,
                    Instruction::Copy { src, .. } if *src == konst(30)
                )),
                "{policy:?}: {:?}",
                trampoline.instructions
            );
        }
    }

    /// `do { s += n; } while (--n);` as the mid-level passes leave it, the
    /// test on the *old* counter:
    ///
    /// ```text
    ///   b0:  br b1
    ///   b1:  n  = phi [v1, b0], [n1, b1]        (v10, v12)
    ///        s  = phi [0,  b0], [s1, b1]        (v11, v13)
    ///        s1 = s + n
    ///        n1 = n <step> k
    ///        c  = n <op> bound                  (v14)
    ///        br c, b1, b2
    ///   b2:  ret s1
    /// ```
    fn countdown(ty: IrType, step: IrBinOp, by: i64, op: IrCmpOp, bound: i64) -> IrFunction {
        let v = |n: u32| Operand::Value(Value(n));
        let k = |c: i64| Operand::Const(IrConst::I64(c));
        let phi = |dest: u32, first: Operand, next: u32| Instruction::Phi {
            dest: Value(dest),
            ty,
            incoming: vec![(first, BlockId(0)), (v(next), BlockId(1))],
        };
        let mut func = IrFunction::new("f".to_string(), ty, vec![], false);
        func.blocks = vec![
            blk(0, Vec::new(), Terminator::Branch(BlockId(1))),
            blk(
                1,
                vec![
                    phi(10, v(1), 12),
                    phi(11, k(0), 13),
                    Instruction::BinOp {
                        dest: Value(13),
                        op: IrBinOp::Add,
                        lhs: v(11),
                        rhs: v(10),
                        ty,
                    },
                    Instruction::BinOp {
                        dest: Value(12),
                        op: step,
                        lhs: v(10),
                        rhs: k(by),
                        ty,
                    },
                    Instruction::Cmp {
                        dest: Value(14),
                        op,
                        lhs: v(10),
                        rhs: k(bound),
                        ty,
                    },
                ],
                cond(14, 1, 2),
            ),
            blk(2, Vec::new(), Terminator::Return(Some(v(13)))),
        ];
        func.next_value_id = 64;
        func.next_label = 3;
        func
    }

    /// The loop's test: `(compared value, constant)`.
    fn test_of(func: &IrFunction) -> (Operand, i64) {
        let test = func.blocks[1]
            .instructions
            .iter()
            .find(|inst| inst.dest() == Some(Value(14)));
        let Some(Instruction::Cmp { lhs, rhs, .. }) = test else {
            panic!("the test is a compare: {test:?}");
        };
        match (lhs, rhs) {
            (value, Operand::Const(c)) | (Operand::Const(c), value) => {
                (*value, c.to_i64().expect("an integer bound"))
            }
            other => panic!("one side is a constant: {other:?}"),
        }
    }

    #[test]
    fn a_latch_test_reads_the_value_it_hands_on() {
        // `n != 1` with `n1 = n - 1`: `n1 != 0`.
        let mut func = countdown(IrType::U64, IrBinOp::Sub, 1, IrCmpOp::Ne, 1);
        assert_eq!(retarget_loop_tests(&mut func), 1);
        assert_eq!(test_of(&func), (Operand::Value(Value(12)), 0));

        // `n == 1`, the same test with the labels the other way round.
        let mut func = countdown(IrType::U64, IrBinOp::Sub, 1, IrCmpOp::Eq, 1);
        assert_eq!(retarget_loop_tests(&mut func), 1);
        assert_eq!(test_of(&func), (Operand::Value(Value(12)), 0));

        // Counting up to a bound: `i != 99` with `i1 = i + 1` is `i1 != 100`.
        let mut func = countdown(IrType::I32, IrBinOp::Add, 1, IrCmpOp::Ne, 99);
        assert_eq!(retarget_loop_tests(&mut func), 1);
        assert_eq!(test_of(&func), (Operand::Value(Value(12)), 100));

        // The constant on the left of the compare and of the addition.
        let mut func = countdown(IrType::I32, IrBinOp::Add, 4, IrCmpOp::Ne, 96);
        for inst in &mut func.blocks[1].instructions {
            match inst {
                Instruction::Cmp { lhs, rhs, .. } => std::mem::swap(lhs, rhs),
                Instruction::BinOp {
                    dest: Value(12),
                    lhs,
                    rhs,
                    ..
                } => std::mem::swap(lhs, rhs),
                _ => {}
            }
        }
        assert_eq!(retarget_loop_tests(&mut func), 1);
        assert_eq!(test_of(&func), (Operand::Value(Value(12)), 100));

        // Once rewritten there is nothing left to rewrite.
        assert_eq!(retarget_loop_tests(&mut func), 0);
    }

    #[test]
    fn the_moved_bound_wraps_as_the_type_does() {
        // `n != 0` with `n1 = n - 1` in 8 unsigned bits: `n1 != 255`.
        let mut func = countdown(IrType::U8, IrBinOp::Sub, 1, IrCmpOp::Ne, 0);
        assert_eq!(retarget_loop_tests(&mut func), 1);
        assert_eq!(test_of(&func), (Operand::Value(Value(12)), 255));

        // The same in 8 signed bits is -1, and at the top of the range the
        // sum comes round: `n != 127` with `n1 = n + 1` is `n1 != -128`.
        let mut func = countdown(IrType::I8, IrBinOp::Sub, 1, IrCmpOp::Ne, 0);
        assert_eq!(retarget_loop_tests(&mut func), 1);
        assert_eq!(test_of(&func), (Operand::Value(Value(12)), -1));
        let mut func = countdown(IrType::I8, IrBinOp::Add, 1, IrCmpOp::Ne, 127);
        assert_eq!(retarget_loop_tests(&mut func), 1);
        assert_eq!(test_of(&func), (Operand::Value(Value(12)), -128));

        // Every pair of an 8-bit ring: the two tests agree on every value.
        for bound in 0u8..=255 {
            for by in [1u8, 2, 3, 127, 128, 255] {
                for n in 0u8..=255 {
                    assert_eq!(n != bound, n.wrapping_sub(by) != bound.wrapping_sub(by));
                    assert_eq!(n != bound, n.wrapping_add(by) != bound.wrapping_add(by));
                }
            }
        }
    }

    #[test]
    fn a_latch_test_that_cannot_move_is_left_alone() {
        let untouched = |func: &mut IrFunction, why: &str| {
            let before = test_of(func);
            assert_eq!(retarget_loop_tests(func), 0, "{why}");
            assert_eq!(test_of(func), before, "{why}");
        };

        // An ordered test does not commute with an addition that wraps:
        // `n <u 1` is not `n - 1 <u 0`.
        for op in [IrCmpOp::Ult, IrCmpOp::Slt, IrCmpOp::Uge, IrCmpOp::Sgt] {
            let mut func = countdown(IrType::U64, IrBinOp::Sub, 1, op, 1);
            untouched(&mut func, "an ordered test");
        }

        // The step is not a constant step of the compared value.
        let mut func = countdown(IrType::U64, IrBinOp::Sub, 1, IrCmpOp::Ne, 1);
        if let Instruction::BinOp { op, .. } = &mut func.blocks[1].instructions[3] {
            *op = IrBinOp::Mul;
        }
        untouched(&mut func, "a multiplication");
        let mut func = countdown(IrType::U64, IrBinOp::Sub, 1, IrCmpOp::Ne, 1);
        if let Instruction::BinOp { lhs, rhs, .. } = &mut func.blocks[1].instructions[3] {
            // `1 - n` is not `n - 1`.
            std::mem::swap(lhs, rhs);
        }
        untouched(&mut func, "the constant minus the counter");

        // The step stands behind the test: the test cannot read it.
        let mut func = countdown(IrType::U64, IrBinOp::Sub, 1, IrCmpOp::Ne, 1);
        func.blocks[1].instructions.swap(3, 4);
        untouched(&mut func, "the step behind the test");

        // Not single assignment: with the counter written again between the
        // step and the test, the test reads another value than the step did.
        let mut func = countdown(IrType::U64, IrBinOp::Sub, 1, IrCmpOp::Ne, 1);
        func.blocks[1].instructions.insert(4, cp(10, 11));
        untouched(&mut func, "the counter written twice");
        let mut func = countdown(IrType::U64, IrBinOp::Sub, 1, IrCmpOp::Ne, 1);
        func.blocks[1].instructions.insert(4, cp(12, 11));
        untouched(&mut func, "the step written twice");
        // ... or in another block, or the test itself.
        let mut func = countdown(IrType::U64, IrBinOp::Sub, 1, IrCmpOp::Ne, 1);
        func.blocks[2].instructions.push(cp(12, 11));
        untouched(&mut func, "the step written again behind the loop");
        let mut func = countdown(IrType::U64, IrBinOp::Sub, 1, IrCmpOp::Ne, 1);
        func.blocks[0].instructions.push(cp(14, 1));
        untouched(&mut func, "the test written twice");

        // The compare is of another width than the counter. (The low half
        // of `n - 1` is the low half of `n`, less one, so the identity
        // would hold there too; what a compare of one width reads of a value
        // of another is not this function's to say.)
        let mut func = countdown(IrType::U64, IrBinOp::Sub, 1, IrCmpOp::Ne, 1);
        if let Instruction::Cmp { ty, .. } = &mut func.blocks[1].instructions[4] {
            *ty = IrType::U32;
        }
        untouched(&mut func, "a narrower compare");

        // The compared value is not the phi the step is handed to.
        let mut func = countdown(IrType::U64, IrBinOp::Sub, 1, IrCmpOp::Ne, 1);
        if let Instruction::Cmp { lhs, .. } = &mut func.blocks[1].instructions[4] {
            *lhs = Operand::Value(Value(11));
        }
        untouched(&mut func, "another phi");

        // The new bound would need a register where the old one did not:
        // `n != 0x7fffffff` with `n1 = n + 1` in 64 bits.
        let mut func = countdown(
            IrType::I64,
            IrBinOp::Add,
            1,
            IrCmpOp::Ne,
            i64::from(i32::MAX),
        );
        untouched(&mut func, "a bound past 32 bits");
        // In 32 bits every bound is an immediate.
        let mut func = countdown(
            IrType::U32,
            IrBinOp::Add,
            1,
            IrCmpOp::Ne,
            i64::from(i32::MAX),
        );
        assert_eq!(retarget_loop_tests(&mut func), 1);
        assert_eq!(
            test_of(&func),
            (Operand::Value(Value(12)), i64::from(i32::MAX) + 1)
        );

        // A float has no ring to do this in.
        let mut func = countdown(IrType::F64, IrBinOp::Sub, 1, IrCmpOp::Ne, 1);
        untouched(&mut func, "a float");
    }

    /// A countdown over a body of several blocks, the step in block
    /// `step_in` (1 the header, 2 or 3 an arm, 4 the latch) and the test in
    /// the latch:
    ///
    /// ```text
    ///   B0: br B1
    ///   B1: n = phi [v1, B0], [n1, B4]      (10, 12)
    ///       s = phi [0, B0], [s1, B4]       (11, 13)
    ///       odd = n & 1                     (15)
    ///       br odd, B2, B3
    ///   B2: br B4
    ///   B3: br B4
    ///   B4: s1 = s + n
    ///       c = n != 1                      (14)
    ///       br c, B1, B5
    ///   B5: ret s1
    /// ```
    fn countdown_over_arms(step_in: usize) -> IrFunction {
        let ty = IrType::U64;
        let v = |n: u32| Operand::Value(Value(n));
        let k = |c: i64| Operand::Const(IrConst::I64(c));
        let phi = |dest: u32, first: Operand, next: u32| Instruction::Phi {
            dest: Value(dest),
            ty,
            incoming: vec![(first, BlockId(0)), (v(next), BlockId(4))],
        };
        let binop = |dest: u32, op: IrBinOp, lhs: Operand, rhs: Operand| Instruction::BinOp {
            dest: Value(dest),
            op,
            lhs,
            rhs,
            ty,
        };
        let mut func = IrFunction::new("f".to_string(), ty, vec![], false);
        func.blocks = vec![
            blk(0, Vec::new(), Terminator::Branch(BlockId(1))),
            blk(
                1,
                vec![
                    phi(10, v(1), 12),
                    phi(11, k(0), 13),
                    binop(15, IrBinOp::And, v(10), k(1)),
                ],
                cond(15, 2, 3),
            ),
            blk(2, Vec::new(), Terminator::Branch(BlockId(4))),
            blk(3, Vec::new(), Terminator::Branch(BlockId(4))),
            blk(
                4,
                vec![
                    binop(13, IrBinOp::Add, v(11), v(10)),
                    Instruction::Cmp {
                        dest: Value(14),
                        op: IrCmpOp::Ne,
                        lhs: v(10),
                        rhs: k(1),
                        ty,
                    },
                ],
                cond(14, 1, 5),
            ),
            blk(5, Vec::new(), Terminator::Return(Some(v(13)))),
        ];
        // The step goes in front of whatever its block holds behind the phis.
        let step = binop(12, IrBinOp::Sub, v(10), k(1));
        let at = if step_in == 1 { 2 } else { 0 };
        func.blocks[step_in].instructions.insert(at, step);
        func.next_value_id = 64;
        func.next_label = 6;
        func
    }

    /// The test of [`countdown_over_arms`]: `(compared value, constant)`.
    fn latch_test_over_arms(func: &IrFunction) -> (Operand, i64) {
        let test = func.blocks[4]
            .instructions
            .iter()
            .find(|inst| inst.dest() == Some(Value(14)));
        let Some(Instruction::Cmp { lhs, rhs, .. }) = test else {
            panic!("the test is a compare: {test:?}");
        };
        match (lhs, rhs) {
            (value, Operand::Const(c)) | (Operand::Const(c), value) => {
                (*value, c.to_i64().expect("an integer bound"))
            }
            other => panic!("one side is a constant: {other:?}"),
        }
    }

    #[test]
    fn a_latch_test_reads_a_step_made_in_a_dominating_block() {
        // The step at the top of the body, the test at its bottom: the
        // header dominates the latch, the step has run when the test does.
        let mut func = countdown_over_arms(1);
        assert_eq!(retarget_loop_tests(&mut func), 1);
        assert_eq!(latch_test_over_arms(&func), (Operand::Value(Value(12)), 0));
        // In the latch itself, ahead of the test.
        let mut func = countdown_over_arms(4);
        assert_eq!(retarget_loop_tests(&mut func), 1);
        assert_eq!(latch_test_over_arms(&func), (Operand::Value(Value(12)), 0));
        // Behind the test it has not run yet.
        let mut func = countdown_over_arms(4);
        let step = func.blocks[4].instructions.remove(0);
        func.blocks[4].instructions.push(step);
        assert_eq!(retarget_loop_tests(&mut func), 0);
        assert_eq!(latch_test_over_arms(&func), (Operand::Value(Value(10)), 1));
    }

    #[test]
    fn a_latch_test_is_not_moved_onto_a_step_the_latch_does_not_hand_on() {
        // The step stands in one arm only, so it does not dominate the
        // latch and cannot be what the latch hands the phi: a function in
        // single-assignment form hands on a merge of the two arms. With the
        // merge in place the test has no step to read.
        let mut func = countdown_over_arms(2);
        func.blocks[4].instructions.insert(
            0,
            Instruction::Phi {
                dest: Value(16),
                ty: IrType::U64,
                incoming: vec![
                    (Operand::Value(Value(12)), BlockId(2)),
                    (Operand::Value(Value(10)), BlockId(3)),
                ],
            },
        );
        if let Instruction::Phi { incoming, .. } = &mut func.blocks[1].instructions[0] {
            incoming[1].0 = Operand::Value(Value(16));
        }
        assert_eq!(retarget_loop_tests(&mut func), 0);
        assert_eq!(latch_test_over_arms(&func), (Operand::Value(Value(10)), 1));

        // The test stands in another block than the branch that reads it:
        // nothing says the step has run there.
        let mut func = countdown_over_arms(4);
        let test = func.blocks[4].instructions.pop().expect("the test");
        func.blocks[1].instructions.push(test);
        assert_eq!(retarget_loop_tests(&mut func), 0);
    }

    /// The branch of [`countdown`]'s loop: the value it reads, and the
    /// compare that defines that value if one does.
    fn branch_of(func: &IrFunction) -> (Value, Option<&Instruction>) {
        let Terminator::CondBranch {
            cond: Operand::Value(cond),
            ..
        } = &func.blocks[1].terminator
        else {
            panic!("the loop ends in a branch on a value");
        };
        let test = func.blocks[1]
            .instructions
            .iter()
            .find(|inst| inst.dest() == Some(*cond) && matches!(inst, Instruction::Cmp { .. }));
        (*cond, test)
    }

    /// [`countdown`] with the branch on the counter itself, `while (n--)`
    /// as a `do` loop: no compare, the branch asks whether `n` is zero.
    fn countdown_on_itself(ty: IrType, step: IrBinOp, by: i64) -> IrFunction {
        let mut func = countdown(ty, step, by, IrCmpOp::Ne, 0);
        let test = func.blocks[1].instructions.pop();
        assert!(matches!(test, Some(Instruction::Cmp { .. })));
        func.blocks[1].terminator = cond(10, 1, 2);
        func
    }

    #[test]
    fn a_branch_on_the_counter_itself_gets_a_compare_of_the_stepped_one() {
        // `n != 0` with `n1 = n - 1`: `n1 != -1`, in a compare that did not
        // exist, at the end of the block, under a name nothing else has.
        let mut func = countdown_on_itself(IrType::U64, IrBinOp::Sub, 1);
        let names_before = func.next_value_id;
        assert_eq!(retarget_loop_tests(&mut func), 1);
        let (branch, test) = branch_of(&func);
        assert!(branch.0 >= names_before && func.next_value_id > branch.0);
        assert!(
            matches!(
                test,
                Some(Instruction::Cmp {
                    op: IrCmpOp::Ne,
                    lhs: Operand::Value(Value(12)),
                    rhs: Operand::Const(k),
                    ty: IrType::U64,
                    ..
                }) if k.to_i64() == Some(-1)
            ),
            "{test:?}"
        );
        assert!(matches!(
            func.blocks[1].instructions.last(),
            Some(Instruction::Cmp { dest, .. }) if *dest == branch
        ));
        // The branch keeps its two ways out: nonzero stays in the loop.
        assert!(matches!(
            func.blocks[1].terminator,
            Terminator::CondBranch {
                true_label: BlockId(1),
                false_label: BlockId(2),
                ..
            }
        ));
        // The compare reads the stepped counter now: nothing more to do.
        assert_eq!(retarget_loop_tests(&mut func), 0);

        // Counting up by three in 8 unsigned bits: `n != 0` is `n1 != 3`;
        // down by one it is `n1 != 255`.
        let mut func = countdown_on_itself(IrType::U8, IrBinOp::Add, 3);
        assert_eq!(retarget_loop_tests(&mut func), 1);
        let (_, test) = branch_of(&func);
        assert!(
            matches!(test, Some(Instruction::Cmp { rhs: Operand::Const(k), .. }) if k.to_i64() == Some(3)),
            "{test:?}"
        );
        let mut func = countdown_on_itself(IrType::U8, IrBinOp::Sub, 1);
        assert_eq!(retarget_loop_tests(&mut func), 1);
        let (_, test) = branch_of(&func);
        assert!(
            matches!(test, Some(Instruction::Cmp { rhs: Operand::Const(k), .. }) if k.to_i64() == Some(255)),
            "{test:?}"
        );

        // A step no 32-bit immediate holds, where the test was free.
        let mut func = countdown_on_itself(IrType::U64, IrBinOp::Sub, 1 << 40);
        assert_eq!(retarget_loop_tests(&mut func), 0);
        assert!(matches!(branch_of(&func), (Value(10), None)));
        // A float is zero in two ways and steps in no ring.
        let mut func = countdown_on_itself(IrType::F64, IrBinOp::Sub, 1);
        assert_eq!(retarget_loop_tests(&mut func), 0);
        // The branch reads a value that is no phi: not a loop's counter.
        let mut func = countdown_on_itself(IrType::U64, IrBinOp::Sub, 1);
        func.blocks[1].terminator = cond(13, 1, 2);
        assert_eq!(retarget_loop_tests(&mut func), 0);
        assert!(matches!(branch_of(&func), (Value(13), None)));
    }

    /// [`countdown`] over a counter of type `narrow` whose step is made in
    /// `wide` and cast back, as C's promotion leaves `n--`:
    ///
    /// ```text
    ///   w  = n - by       (v15, in `wide`)
    ///   n1 = (narrow)w    (v12)
    /// ```
    fn countdown_through_cast(narrow: IrType, wide: IrType, by: i64, bound: i64) -> IrFunction {
        let mut func = countdown(narrow, IrBinOp::Sub, by, IrCmpOp::Ne, bound);
        let insts = &mut func.blocks[1].instructions;
        match &mut insts[3] {
            Instruction::BinOp { dest, ty, .. } => {
                *dest = Value(15);
                *ty = wide;
            }
            other => panic!("the step: {other:?}"),
        }
        insts.insert(
            4,
            Instruction::Cast {
                dest: Value(12),
                src: Operand::Value(Value(15)),
                from_ty: wide,
                to_ty: narrow,
            },
        );
        func
    }

    #[test]
    fn a_step_made_in_a_wider_type_and_cut_back_is_the_same_step() {
        // `unsigned char n; do … while (n-- != 1);`
        let mut func = countdown_through_cast(IrType::U8, IrType::I32, 1, 1);
        assert_eq!(retarget_loop_tests(&mut func), 1);
        assert_eq!(test_of(&func), (Operand::Value(Value(12)), 0));
        // The bound wraps in the counter's width, not in the sum's:
        // `n != 0` with a step of 3 is `n1 != 253`.
        let mut func = countdown_through_cast(IrType::U8, IrType::I32, 3, 0);
        assert_eq!(retarget_loop_tests(&mut func), 1);
        assert_eq!(test_of(&func), (Operand::Value(Value(12)), 253));
        let mut func = countdown_through_cast(IrType::I16, IrType::I32, 1, -32768);
        assert_eq!(retarget_loop_tests(&mut func), 1);
        assert_eq!(test_of(&func), (Operand::Value(Value(12)), 32767));
        // The same width under the other sign is all the bits.
        let mut func = countdown_through_cast(IrType::U32, IrType::I32, 1, 1);
        assert_eq!(retarget_loop_tests(&mut func), 1);
        assert_eq!(test_of(&func), (Operand::Value(Value(12)), 0));
        // The branch on the counter itself, through the cast.
        let mut func = countdown_through_cast(IrType::U16, IrType::I32, 1, 0);
        let test = func.blocks[1].instructions.pop();
        assert!(matches!(test, Some(Instruction::Cmp { .. })));
        func.blocks[1].terminator = cond(10, 1, 2);
        assert_eq!(retarget_loop_tests(&mut func), 1);
        let (_, test) = branch_of(&func);
        assert!(
            matches!(
                test,
                Some(Instruction::Cmp {
                    lhs: Operand::Value(Value(12)),
                    rhs: Operand::Const(k),
                    ty: IrType::U16,
                    ..
                }) if k.to_i64() == Some(65535)
            ),
            "{test:?}"
        );

        // Every counter of 8 bits, every step and bound: cutting the wide
        // sum back gives the value the test is moved onto.
        for n in 0u8..=255 {
            for by in [1i32, 3, 7, 200, 256, 1000] {
                let next = (i32::from(n) - by) as u8;
                for bound in [0u8, 1, 5, 254, 255] {
                    let moved = (i64::from(bound) - i64::from(by)) as u8;
                    assert_eq!(n != bound, next != moved, "{n} - {by} against {bound}");
                }
            }
        }

        let untouched = |func: &mut IrFunction, why: &str| {
            let before = test_of(func);
            assert_eq!(retarget_loop_tests(func), 0, "{why}");
            assert_eq!(test_of(func), before, "{why}");
        };
        // A cast to a wider type makes up the high bits: `(u32)(u8)(n - 1)`
        // is not `n - 1` in 32 bits.
        let mut func = countdown_through_cast(IrType::U32, IrType::U8, 1, 1);
        untouched(&mut func, "a sum narrower than the counter");
        // A cast that reads fewer bits of the sum than the counter has:
        // `(u32)(u8)w` again, spelled in the cast alone.
        let mut func = countdown_through_cast(IrType::U32, IrType::I64, 1, 1);
        if let Instruction::Cast { from_ty, .. } = &mut func.blocks[1].instructions[4] {
            *from_ty = IrType::U8;
        }
        untouched(&mut func, "a cast that reads a byte of the sum");
        // One that reads more than it keeps is the same cut: the sum in
        // 32 bits, read as 16, kept as 8.
        let mut func = countdown_through_cast(IrType::U8, IrType::I32, 1, 1);
        if let Instruction::Cast { from_ty, .. } = &mut func.blocks[1].instructions[4] {
            *from_ty = IrType::I16;
        }
        assert_eq!(retarget_loop_tests(&mut func), 1);
        assert_eq!(test_of(&func), (Operand::Value(Value(12)), 0));
        // The cast is to another width than the counter's.
        let mut func = countdown_through_cast(IrType::U8, IrType::I32, 1, 1);
        if let Instruction::Cast { to_ty, .. } = &mut func.blocks[1].instructions[4] {
            *to_ty = IrType::U16;
        }
        untouched(&mut func, "a cast to another width than the counter's");
        // A sum of 128 bits is beyond the constants here.
        let mut func = countdown_through_cast(IrType::U64, IrType::I128, 1, 1);
        untouched(&mut func, "a sum of 128 bits");
        // The sum behind the cast is of something else.
        let mut func = countdown_through_cast(IrType::U8, IrType::I32, 1, 1);
        if let Instruction::BinOp { lhs, .. } = &mut func.blocks[1].instructions[3] {
            *lhs = Operand::Value(Value(11));
        }
        untouched(&mut func, "a sum of another value");
        // The sum has two definitions.
        let mut func = countdown_through_cast(IrType::U8, IrType::I32, 1, 1);
        func.blocks[2].instructions.push(cp(15, 11));
        untouched(&mut func, "the sum written twice");
    }

    /// [`countdown`] over a counter of type `narrow` that is cast first and
    /// stepped then, as the narrowing passes leave `n--`:
    ///
    /// ```text
    ///   t  = (to)n        (v15, the cast reading `n` as `from`)
    ///   n1 = t - by       (v12, in `sum`)
    /// ```
    fn countdown_cast_first(
        narrow: IrType,
        (from, to): (IrType, IrType),
        sum: IrType,
        by: i64,
        bound: i64,
    ) -> IrFunction {
        let mut func = countdown(narrow, IrBinOp::Sub, by, IrCmpOp::Ne, bound);
        let insts = &mut func.blocks[1].instructions;
        match &mut insts[3] {
            Instruction::BinOp { lhs, ty, .. } => {
                *lhs = Operand::Value(Value(15));
                *ty = sum;
            }
            other => panic!("the step: {other:?}"),
        }
        insts.insert(
            3,
            Instruction::Cast {
                dest: Value(15),
                src: Operand::Value(Value(10)),
                from_ty: from,
                to_ty: to,
            },
        );
        func
    }

    #[test]
    fn a_counter_cast_first_and_stepped_then_is_the_same_step() {
        // `(u16)n - 1`, the cast reading `n` as the `int` it was promoted
        // to: the shape of `unsigned short n; n-- != 1` at -O2.
        let mut func =
            countdown_cast_first(IrType::U16, (IrType::I32, IrType::U16), IrType::U16, 1, 1);
        assert_eq!(retarget_loop_tests(&mut func), 1);
        assert_eq!(test_of(&func), (Operand::Value(Value(12)), 0));
        // Widened, stepped in the wide type, and handed back by a cast.
        let mut func =
            countdown_cast_first(IrType::U8, (IrType::U8, IrType::I32), IrType::I32, 3, 0);
        match &mut func.blocks[1].instructions[4] {
            Instruction::BinOp { dest, .. } => *dest = Value(16),
            other => panic!("the step: {other:?}"),
        }
        func.blocks[1].instructions.insert(
            5,
            Instruction::Cast {
                dest: Value(12),
                src: Operand::Value(Value(16)),
                from_ty: IrType::I32,
                to_ty: IrType::U8,
            },
        );
        assert_eq!(retarget_loop_tests(&mut func), 1);
        assert_eq!(test_of(&func), (Operand::Value(Value(12)), 253));

        let untouched = |func: &mut IrFunction, why: &str| {
            let before = test_of(func);
            assert_eq!(retarget_loop_tests(func), 0, "{why}");
            assert_eq!(test_of(func), before, "{why}");
        };
        // `(u32)(u8)n - 1`: the cast reads one byte of a counter of four.
        let mut func =
            countdown_cast_first(IrType::U32, (IrType::U8, IrType::U32), IrType::U32, 1, 1);
        untouched(&mut func, "a cast that reads a byte of the counter");
        // The step is made in fewer bits than the counter has.
        let mut func =
            countdown_cast_first(IrType::U32, (IrType::U32, IrType::U16), IrType::U16, 1, 1);
        untouched(&mut func, "a step in half the counter's width");
        // More links than are followed.
        let mut func = countdown(IrType::U64, IrBinOp::Sub, 1, IrCmpOp::Ne, 1);
        let insts = &mut func.blocks[1].instructions;
        let Instruction::BinOp { lhs, .. } = &mut insts[3] else {
            panic!("the step");
        };
        *lhs = Operand::Value(Value(40 + MAX_STEP_LINKS as u32));
        for link in 0..=MAX_STEP_LINKS as u32 {
            let from = if link == 0 { 10 } else { 40 + link };
            insts.insert(
                3,
                Instruction::BinOp {
                    dest: Value(41 + link),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(from)),
                    rhs: Operand::Const(IrConst::I64(1)),
                    ty: IrType::U64,
                },
            );
        }
        untouched(&mut func, "a chain longer than is followed");
    }

    /// `while (n--) …;` before rotation: step and test at the top, the back
    /// edge from the bottom.
    ///
    /// ```text
    ///   B0: br B1
    ///   B1: n  = phi [v1, B0], [n1, B2]     (10, 12)
    ///       n1 = n - 1
    ///       c  = n != 0                     (14)
    ///       br c, B2, B3
    ///   B2: br B1
    ///   B3: ret n1
    /// ```
    fn while_post_decrement() -> IrFunction {
        let ty = IrType::U64;
        let v = |n: u32| Operand::Value(Value(n));
        let k = |c: i64| Operand::Const(IrConst::I64(c));
        let mut func = IrFunction::new("f".to_string(), ty, vec![], false);
        func.blocks = vec![
            blk(0, Vec::new(), Terminator::Branch(BlockId(1))),
            blk(
                1,
                vec![
                    Instruction::Phi {
                        dest: Value(10),
                        ty,
                        incoming: vec![(v(1), BlockId(0)), (v(12), BlockId(2))],
                    },
                    Instruction::BinOp {
                        dest: Value(12),
                        op: IrBinOp::Sub,
                        lhs: v(10),
                        rhs: k(1),
                        ty,
                    },
                    Instruction::Cmp {
                        dest: Value(14),
                        op: IrCmpOp::Ne,
                        lhs: v(10),
                        rhs: k(0),
                        ty,
                    },
                ],
                cond(14, 2, 3),
            ),
            blk(2, Vec::new(), Terminator::Branch(BlockId(1))),
            blk(3, Vec::new(), Terminator::Return(Some(v(12)))),
        ];
        func.next_value_id = 64;
        func.next_label = 4;
        func
    }

    #[test]
    fn a_header_test_reads_the_step_made_ahead_of_it() {
        // The header hands the phi nothing; it has computed what the latch
        // will, ahead of its test.
        let mut func = while_post_decrement();
        assert_eq!(retarget_loop_tests(&mut func), 1);
        assert_eq!(test_of(&func), (Operand::Value(Value(12)), -1));

        // The same with the branch on the counter itself.
        let mut func = while_post_decrement();
        func.blocks[1].instructions.pop();
        func.blocks[1].terminator = cond(10, 2, 3);
        assert_eq!(retarget_loop_tests(&mut func), 1);
        let (cond_value, test) = branch_of(&func);
        assert!(cond_value != Value(10));
        assert!(
            matches!(
                test,
                Some(Instruction::Cmp {
                    op: IrCmpOp::Ne,
                    lhs: Operand::Value(Value(12)),
                    rhs: Operand::Const(k),
                    ..
                }) if k.to_i64() == Some(-1)
            ),
            "{test:?}"
        );

        // The step behind the test, in a block that does not hand it on:
        // it has not run when the test does.
        let mut func = while_post_decrement();
        func.blocks[1].instructions.swap(1, 2);
        assert_eq!(retarget_loop_tests(&mut func), 0);
        assert_eq!(test_of(&func), (Operand::Value(Value(10)), 0));

        // The step in the latch, which the header neither is nor follows.
        let mut func = while_post_decrement();
        let step = func.blocks[1].instructions.remove(1);
        func.blocks[2].instructions.push(step);
        if let Terminator::Return(value) = &mut func.blocks[3].terminator {
            *value = Some(Operand::Value(Value(10)));
        }
        assert_eq!(retarget_loop_tests(&mut func), 0);
        assert_eq!(test_of(&func), (Operand::Value(Value(10)), 0));

        // Two values on their way to the phi, both made ahead of the test:
        // no telling which one the test is about.
        let mut func = while_post_decrement();
        func.blocks[1].instructions.insert(
            2,
            Instruction::BinOp {
                dest: Value(16),
                op: IrBinOp::Sub,
                lhs: Operand::Value(Value(10)),
                rhs: Operand::Const(IrConst::I64(2)),
                ty: IrType::U64,
            },
        );
        func.blocks
            .push(blk(4, Vec::new(), Terminator::Branch(BlockId(1))));
        if let Instruction::Phi { incoming, .. } = &mut func.blocks[1].instructions[0] {
            incoming.push((Operand::Value(Value(16)), BlockId(4)));
        }
        assert_eq!(retarget_loop_tests(&mut func), 0);
        assert_eq!(test_of(&func), (Operand::Value(Value(10)), 0));

        // The block hands the phi a constant: that is no step.
        let mut func = countdown(IrType::U64, IrBinOp::Sub, 1, IrCmpOp::Ne, 1);
        if let Instruction::Phi { incoming, .. } = &mut func.blocks[1].instructions[0] {
            incoming[1].0 = Operand::Const(IrConst::I64(7));
        }
        assert_eq!(retarget_loop_tests(&mut func), 0);
        assert_eq!(test_of(&func), (Operand::Value(Value(10)), 1));
    }

    #[test]
    fn a_value_an_inline_asm_statement_writes_has_that_definition_too() {
        // `next` is also the output of an asm statement between the step
        // and the test: it is not `n - 1` when the test runs.
        let mut func = countdown(IrType::U64, IrBinOp::Sub, 1, IrCmpOp::Ne, 1);
        func.blocks[1].instructions.insert(
            4,
            Instruction::InlineAsm {
                template: String::new(),
                outputs: vec![("=r".to_string(), Value(12), None)],
                inputs: Vec::new(),
                clobbers: Vec::new(),
                operand_types: vec![IrType::U64],
                goto_labels: Vec::new(),
                input_symbols: Vec::new(),
                seg_overrides: Vec::new(),
            },
        );
        assert_eq!(retarget_loop_tests(&mut func), 0);
        assert_eq!(test_of(&func), (Operand::Value(Value(10)), 1));
    }

    /// The point of it: with the test on the handed-on value the phi's copy
    /// goes into the block and the back edge is not split. `do s += n; while
    /// (--n);` lowers to one block that branches to itself.
    #[test]
    fn a_countdown_loop_keeps_no_block_on_its_back_edge() {
        for policy in [CopyOrderPolicy::Acyclic, CopyOrderPolicy::Legacy] {
            let mut func = countdown(IrType::U64, IrBinOp::Sub, 1, IrCmpOp::Ne, 1);
            let mut next_block_id = 3;
            eliminate_phis_with_policy(&mut func, &mut next_block_id, policy);
            assert_eq!(func.blocks.len(), 3, "{policy:?}: no block was added");
            assert!(
                matches!(
                    func.blocks[1].terminator,
                    Terminator::CondBranch {
                        true_label: BlockId(1),
                        false_label: BlockId(2),
                        ..
                    }
                ),
                "{policy:?}: {:?}",
                func.blocks[1].terminator
            );
        }
    }

    #[test]
    fn absorbs_block_local_commuting_copies() {
        let blocks = rotated_loop();
        assert!(hoists(&blocks, 0, &[cp(10, 13), cp(11, 15)]));
    }

    #[test]
    fn rejects_when_a_destination_is_read_by_another_block() {
        let mut blocks = rotated_loop();
        // v10 (the induction value) escapes to the exit block.
        blocks[1].instructions.push(cp(8, 10));
        assert!(!hoists(&blocks, 0, &[cp(10, 13), cp(11, 15)]));
    }

    #[test]
    fn rejects_when_the_own_terminator_reads_a_destination() {
        let mut blocks = rotated_loop();
        blocks[0].terminator = cond(10, 1, 2);
        assert!(!hoists(&blocks, 0, &[cp(10, 13), cp(11, 15)]));
    }

    /// The loop's result is read behind it, under the name the loop itself
    /// uses (`phi_webs` gave the exit phi and the loop phi one name): `v10 =
    /// v13` is wanted on the backedge and on the exit edge alike.
    fn loop_with_a_live_out() -> Vec<BasicBlock> {
        let mut blocks = rotated_loop();
        blocks[1].instructions.push(cp(8, 10));
        blocks
    }

    #[test]
    fn absorbs_a_copy_the_exit_edge_makes_as_well() {
        let blocks = loop_with_a_live_out();
        let backedge = [cp(10, 13), cp(11, 15)];
        // Alone, the read behind the loop keeps the copy on its edge.
        assert!(!hoists(&blocks, 0, &backedge));
        // With the exit edge making it too, once ahead of the branch is the
        // same program.
        assert!(hoists_with_exit(&blocks, 0, &backedge, &[cp(10, 13)]));
        // The exit edge may carry copies of its own beside it.
        assert!(hoists_with_exit(
            &blocks,
            0,
            &backedge,
            &[cp(10, 13), cp(30, 31)]
        ));
    }

    #[test]
    fn rejects_a_repeated_copy_the_exit_edge_would_see_too_early() {
        let blocks = loop_with_a_live_out();
        let backedge = [cp(10, 13), cp(11, 15)];
        // Another exit copy reads the destination: it wants the old value.
        assert!(!hoists_with_exit(
            &blocks,
            0,
            &backedge,
            &[cp(10, 13), cp(30, 10)]
        ));
        assert!(!hoists_with_exit(
            &blocks,
            0,
            &backedge,
            &[cp(30, 10), cp(10, 13)]
        ));
        // Another exit copy writes the source ahead of it.
        assert!(!hoists_with_exit(
            &blocks,
            0,
            &backedge,
            &[cp(13, 31), cp(10, 13)]
        ));
        // The exit edge gives the destination a different value.
        assert!(!hoists_with_exit(&blocks, 0, &backedge, &[cp(10, 14)]));
        // Twice on the exit edge is not a parallel copy.
        assert!(!hoists_with_exit(
            &blocks,
            0,
            &backedge,
            &[cp(10, 13), cp(10, 13)]
        ));
    }

    #[test]
    fn rejects_a_repeated_copy_the_own_terminator_reads() {
        let mut blocks = loop_with_a_live_out();
        blocks[0].terminator = cond(10, 1, 2);
        assert!(!hoists_with_exit(
            &blocks,
            0,
            &[cp(10, 13), cp(11, 15)],
            &[cp(10, 13)]
        ));
    }

    /// A copy only the backedge makes still has to be invisible outside the
    /// block, whatever the exit edge repeats.
    #[test]
    fn a_private_backedge_copy_keeps_its_rule_beside_a_repeated_one() {
        let mut blocks = loop_with_a_live_out();
        blocks[1].instructions.push(cp(6, 11));
        assert!(!hoists_with_exit(
            &blocks,
            0,
            &[cp(10, 13), cp(11, 15)],
            &[cp(10, 13)]
        ));
    }

    /// End to end: a bottom-tested loop whose accumulator is returned.
    ///
    /// ```text
    ///   0: -> 1
    ///   1: s = phi [v10, 0] [s', 1] ; i = phi [v11, 0] [i', 1]
    ///      s' = ... ; i' = ... ; c = ...      -> 1 | 2
    ///   2: r = phi [v10, 5] [s', 1] ; ret r
    ///   5: -> 2                                (the zero-trip path)
    /// ```
    ///
    /// The exit phi and the loop phi are twins: one name, one copy ahead of
    /// the branch, and neither edge of the loop block split.
    #[test]
    fn a_live_out_of_a_bottom_tested_loop_costs_no_edge_block() {
        let ty = crate::common::types::IrType::I64;
        let phi = |dest: u32, arms: &[(u32, u32)]| Instruction::Phi {
            dest: Value(dest),
            ty,
            incoming: arms
                .iter()
                .map(|&(v, from)| (Operand::Value(Value(v)), BlockId(from)))
                .collect(),
        };
        let add = |dest: u32, lhs: u32| Instruction::BinOp {
            dest: Value(dest),
            op: crate::ir::reexports::IrBinOp::Add,
            lhs: Operand::Value(Value(lhs)),
            rhs: Operand::Const(crate::ir::reexports::IrConst::I64(1)),
            ty,
        };
        let mut func = IrFunction::new("live_out".to_string(), ty, Vec::new(), false);
        func.blocks = vec![
            blk(0, vec![cmp9(11)], cond(9, 4, 5)),
            blk(4, Vec::new(), Terminator::Branch(BlockId(1))),
            blk(
                1,
                vec![
                    phi(20, &[(10, 4), (22, 1)]),
                    phi(21, &[(11, 4), (23, 1)]),
                    add(22, 20),
                    add(23, 21),
                    Instruction::Cmp {
                        dest: Value(24),
                        op: crate::ir::reexports::IrCmpOp::Ne,
                        lhs: Operand::Value(Value(23)),
                        rhs: Operand::Value(Value(12)),
                        ty,
                    },
                ],
                cond(24, 1, 2),
            ),
            blk(5, Vec::new(), Terminator::Branch(BlockId(2))),
            blk(
                2,
                vec![phi(30, &[(10, 5), (22, 1)])],
                Terminator::Return(Some(Operand::Value(Value(30)))),
            ),
        ];
        func.next_value_id = 64;
        func.next_label = 6;
        let mut next_block_id = 6;
        eliminate_phis_with_policy(&mut func, &mut next_block_id, CopyOrderPolicy::Acyclic);

        assert_eq!(
            func.blocks.len(),
            5,
            "an edge of the loop block was split: {func:#?}"
        );
        let body = func.blocks.iter().find(|b| b.label.0 == 1).unwrap();
        assert!(
            matches!(
                body.terminator,
                Terminator::CondBranch { true_label, false_label, .. }
                    if true_label == BlockId(1) && false_label == BlockId(2)
            ),
            "{:?}",
            body.terminator
        );
        // The exit block reads the loop's own name and copies nothing: the
        // loop block writes that name (through a copy ahead of its branch, or
        // with the copy folded into the addition itself), and so does the
        // zero-trip path.
        let exit = func.blocks.iter().find(|b| b.label.0 == 2).unwrap();
        assert!(exit.instructions.is_empty(), "{exit:#?}");
        let Terminator::Return(Some(Operand::Value(returned))) = exit.terminator else {
            panic!("the exit block returns a value");
        };
        let defines = |label: u32| {
            func.blocks
                .iter()
                .find(|b| b.label.0 == label)
                .unwrap()
                .instructions
                .iter()
                .any(|i| i.dest() == Some(returned))
        };
        assert!(
            defines(1),
            "the loop block writes the returned name: {body:#?}"
        );
        assert!(defines(5), "the zero-trip path writes the returned name");
    }

    /// A value can flow straight into another block's terminator with no
    /// instruction reading it in between; folding terminator reads into the
    /// instruction-read map hides exactly this and miscompiles.
    #[test]
    fn rejects_when_another_blocks_terminator_reads_a_destination() {
        let mut blocks = rotated_loop();
        blocks[1].terminator = Terminator::Return(Some(Operand::Value(Value(10))));
        assert!(!hoists(&blocks, 0, &[cp(10, 13), cp(11, 15)]));
    }

    #[test]
    fn rejects_non_commuting_copies() {
        let blocks = rotated_loop();
        // A swap: each destination is also a source, so sequentialising changes
        // the meaning.
        assert!(!hoists(&blocks, 0, &[cp(10, 11), cp(11, 10)]));
        // A shift chain, in either order: sound reader-first, and refused
        // all the same (see `self_loop_copies_can_hoist`).
        assert!(!hoists(&blocks, 0, &[cp(10, 11), cp(11, 13)]));
        assert!(!hoists(&blocks, 0, &[cp(11, 13), cp(10, 11)]));
    }

    /// A destination read that follows its source's definition is still safe to
    /// absorb: the copies are appended after every instruction, so the read
    /// sees the old value. What makes it safe *downstream* is the compare-replay
    /// redefinition guard in `x86/codegen/comparison.rs`, which refuses to
    /// re-emit a compare past a redefinition of its operand. Absorbing here is
    /// what exposed that latent backend bug (ra09_selfop_xor at -O3).
    #[test]
    fn absorbs_when_a_destination_is_read_after_its_source_is_defined() {
        let blocks = vec![
            blk(1, vec![cp(13, 10), cp(7, 10)], cond(9, 1, 2)),
            blk(2, Vec::new(), Terminator::Return(None)),
        ];
        assert!(hoists(&blocks, 0, &[cp(10, 13)]));
    }

    /// Reads at or before the source's own definition are inputs to it, so they
    /// precede the source and cannot overlap its live range.
    #[test]
    fn allows_a_destination_read_by_its_sources_own_definition() {
        let blocks = rotated_loop();
        assert!(hoists(&blocks, 0, &[cp(10, 13)]));
    }

    #[test]
    fn rejects_a_repeated_destination() {
        let blocks = rotated_loop();
        assert!(!hoists(&blocks, 0, &[cp(10, 13), cp(10, 15)]));
    }

    #[test]
    fn rejects_without_a_self_edge() {
        let mut blocks = rotated_loop();
        blocks[0].terminator = Terminator::Branch(BlockId(2));
        assert!(!hoists(&blocks, 0, &[cp(10, 13)]));
    }

    #[test]
    fn absorbs_a_source_defined_outside_the_block() {
        let blocks = rotated_loop();
        // A loop-invariant source is the normal case for a bound or step value;
        // the copies still execute only at the block tail, so the destinations
        // are exactly as observable as before.
        assert!(hoists(&blocks, 0, &[cp(10, 99)]));
    }

    /// Absorbing between a `Cmp` and the branch that consumes it forfeits
    /// compare-replay fusion when a copy redefines one of its operands, which
    /// costs an operand reload per iteration.
    #[test]
    fn rejects_when_a_copy_would_split_a_compare_from_its_branch() {
        let blocks = vec![
            blk(1, vec![cp(13, 10), cmp9(10)], cond(9, 1, 2)),
            blk(2, Vec::new(), Terminator::Return(None)),
        ];
        assert!(!hoists(&blocks, 0, &[cp(10, 13)]));
    }

    /// A compare whose operands the copies do not touch keeps its fusion, so
    /// the absorption still pays.
    #[test]
    fn absorbs_when_the_compare_operands_are_untouched() {
        let blocks = vec![
            blk(1, vec![cp(13, 10), cmp9(13)], cond(9, 1, 2)),
            blk(2, Vec::new(), Terminator::Return(None)),
        ];
        assert!(hoists(&blocks, 0, &[cp(10, 13)]));
    }

    #[test]
    fn absorbs_a_constant_source() {
        use crate::ir::reexports::IrConst;
        let blocks = vec![
            blk(1, vec![cp(7, 10)], cond(9, 1, 2)),
            blk(2, Vec::new(), Terminator::Return(None)),
        ];
        let copies = [Instruction::Copy {
            dest: Value(10),
            src: Operand::Const(IrConst::I32(0)),
        }];
        assert!(hoists(&blocks, 0, &copies));
    }

    #[test]
    fn rejects_an_empty_copy_set() {
        let blocks = rotated_loop();
        assert!(!hoists(&blocks, 0, &[]));
    }

    #[test]
    fn finds_a_self_edge_in_a_switch_arm() {
        let blocks = vec![
            blk(
                1,
                vec![cp(13, 10)],
                Terminator::Switch {
                    val: Operand::Value(Value(9)),
                    cases: vec![(0, BlockId(1)), (1, BlockId(2))],
                    default: BlockId(2),
                    ty: crate::common::types::IrType::I32,
                },
            ),
            blk(2, Vec::new(), Terminator::Return(None)),
        ];
        assert!(hoists(&blocks, 0, &[cp(10, 13)]));
    }

    /// Invariants of *any* sound parallel-move resolver are checked under both
    /// arms, so the suite keeps covering whichever one is not the default.
    const BOTH_POLICIES: [CopyOrderPolicy; 2] = [CopyOrderPolicy::Legacy, CopyOrderPolicy::Acyclic];

    #[test]
    fn test_no_conflicts_independent_phis() {
        // a = 1, b = 2 — no overlap between dests and sources
        let copies = vec![(10, Some(1)), (20, Some(2))];
        for policy in BOTH_POLICIES {
            let result = plan_edge_copies(&copies, policy).cyclic;
            assert!(
                result.is_empty(),
                "Independent phis should have no conflicts under {policy:?}"
            );
        }
    }

    #[test]
    fn test_swap_pattern() {
        // a = b, b = a — classic swap cycle
        let copies = vec![(10, Some(20)), (20, Some(10))];
        for policy in BOTH_POLICIES {
            let result = plan_edge_copies(&copies, policy).cyclic;
            assert!(
                result.contains(&0),
                "First phi in swap should be conflicting under {policy:?}"
            );
            assert!(
                result.contains(&1),
                "Second phi in swap should be conflicting under {policy:?}"
            );
        }
    }

    #[test]
    fn test_three_way_cycle() {
        // a = b, b = c, c = a — three-way rotation
        let copies = vec![(10, Some(20)), (20, Some(30)), (30, Some(10))];
        for policy in BOTH_POLICIES {
            let result = plan_edge_copies(&copies, policy).cyclic;
            assert_eq!(
                result.len(),
                3,
                "All three phis in a 3-way cycle should be conflicting under {policy:?}"
            );
        }
    }

    #[test]
    fn test_mixed_conflicting_and_non_conflicting() {
        // a = b, b = a (conflict), c = 99 (independent)
        let copies = vec![(10, Some(20)), (20, Some(10)), (30, Some(99))];
        for policy in BOTH_POLICIES {
            let result = plan_edge_copies(&copies, policy).cyclic;
            assert!(result.contains(&0), "swap member 0 under {policy:?}");
            assert!(result.contains(&1), "swap member 1 under {policy:?}");
            assert!(
                !result.contains(&2),
                "Independent phi should not be marked conflicting under {policy:?}"
            );
        }
    }

    #[test]
    fn test_self_copy_not_conflicting() {
        // a = a — self-copy, not a conflict
        let copies = vec![(10, Some(10)), (20, Some(30))];
        for policy in BOTH_POLICIES {
            let result = plan_edge_copies(&copies, policy).cyclic;
            assert!(
                result.is_empty(),
                "Self-copy should not be conflicting under {policy:?}"
            );
        }
    }

    #[test]
    fn test_constant_source_not_conflicting() {
        // a = <const>, b = <const> — None sources (constants)
        let copies = vec![(10, None), (20, None)];
        for policy in BOTH_POLICIES {
            let result = plan_edge_copies(&copies, policy).cyclic;
            assert!(
                result.is_empty(),
                "Constant sources should have no conflicts under {policy:?}"
            );
        }
    }

    /// A chain `a = b, b = c` is ACYCLIC: it needs no temporary, only the
    /// right execution order (read `b` before `b` is overwritten).
    ///
    /// This replaces the former `test_chain_pattern_conservative`, which
    /// pinned the old over-approximation ("source is somebody's destination"
    /// => temporary). That reading is what doubled SHA-256's rotation webs;
    /// the chain is the shape the fix exists to resolve directly.
    #[test]
    fn chain_needs_no_temporary_and_orders_the_reader_first() {
        // a = b, b = c  (values: a=10, b=20, c=30)
        let copies = vec![(10, Some(20)), (20, Some(30))];
        let plan = plan_edge_copies(&copies, CopyOrderPolicy::Acyclic);
        assert!(
            plan.cyclic.is_empty(),
            "an acyclic chain must not need a temporary, got {plan:?}"
        );
        // `a <- b` must run before `b <- c` overwrites b.
        assert_eq!(plan.ordered, vec![0, 1]);
    }

    /// The SHA-256 state rotation as a copy graph: two chains terminating in
    /// freshly computed values (`a <- t1+t2`, `e <- d+t1`), no cycle anywhere.
    ///
    /// Eight phis, eight direct copies, zero temporaries — and the order must
    /// walk each chain from its far end back to the fresh definition, or the
    /// rotation loses a word.
    #[test]
    fn sha_style_rotation_forest_needs_no_temporary() {
        // dest <- src, mirroring  h=g; g=f; f=e; e=d+t1; d=c; c=b; b=a; a=t1+t2
        // with a=1, b=2, c=3, d=4, e=5, f=6, g=7, h=8, t1t2=90, dt1=91.
        let copies = vec![
            (1, Some(90)), // a <- t1+t2   (fresh)
            (2, Some(1)),  // b <- a
            (3, Some(2)),  // c <- b
            (4, Some(3)),  // d <- c
            (5, Some(91)), // e <- d+t1    (fresh)
            (6, Some(5)),  // f <- e
            (7, Some(6)),  // g <- f
            (8, Some(7)),  // h <- g
        ];
        let plan = plan_edge_copies(&copies, CopyOrderPolicy::Acyclic);
        assert!(
            plan.cyclic.is_empty(),
            "the rotation is a forest of two chains, not a cycle: {plan:?}"
        );
        assert_eq!(plan.ordered.len(), copies.len());

        // The order must place every reader before the writer it reads.
        let pos: FxHashMap<usize, usize> = plan
            .ordered
            .iter()
            .enumerate()
            .map(|(p, &i)| (i, p))
            .collect();
        for &i in &plan.ordered {
            let (dest_i, src_i) = copies[i];
            if let Some(src) = src_i {
                for &j in &plan.ordered {
                    if copies[j].0 == src && copies[j].0 != dest_i {
                        assert!(
                            pos[&i] < pos[&j],
                            "copy {i} reads v{src} written by copy {j}, so it must come first"
                        );
                    }
                }
            }
        }
    }

    /// Near miss that must NOT fire: a genuine swap is a cycle, and a cycle
    /// cannot be executed sequentially — both copies still need temporaries.
    #[test]
    fn swap_cycle_still_needs_both_temporaries() {
        let copies = vec![(10, Some(20)), (20, Some(10))];
        for policy in BOTH_POLICIES {
            let plan = plan_edge_copies(&copies, policy);
            assert_eq!(
                plan.cyclic.len(),
                2,
                "a swap must keep both temporaries under {policy:?}, got {plan:?}"
            );
            assert!(plan.ordered.is_empty(), "{policy:?} ordered a cycle");
        }
    }

    /// Near miss that must NOT fire: a longer true cycle. Every copy in it is
    /// mutually blocked, so all four keep a temporary.
    #[test]
    fn four_way_cycle_is_fully_cyclic() {
        let copies = vec![
            (10, Some(20)),
            (20, Some(30)),
            (30, Some(40)),
            (40, Some(10)),
        ];
        for policy in BOTH_POLICIES {
            let result = plan_edge_copies(&copies, policy).cyclic;
            assert_eq!(
                result.len(),
                4,
                "every copy in a 4-cycle needs a temp under {policy:?}"
            );
        }
    }

    /// A chain hanging off a cycle: the cycle members need temporaries, and
    /// the copies that merely read them must still be emitted — in an order
    /// that is sound once the cyclic destinations stop being written on this
    /// edge.
    #[test]
    fn chain_beside_a_cycle_keeps_its_own_order() {
        // 0: x <- y, 1: y <- x  (swap); 2: z <- x (reads a cyclic dest)
        let copies = vec![(10, Some(20)), (20, Some(10)), (30, Some(10))];
        let plan = plan_edge_copies(&copies, CopyOrderPolicy::Acyclic);
        assert!(plan.cyclic.contains(&0) && plan.cyclic.contains(&1));
        assert!(
            !plan.cyclic.contains(&2),
            "z <- x is not part of the cycle and must stay a direct copy"
        );
        assert_eq!(plan.ordered, vec![2]);
    }

    /// A self-copy must not manufacture a self-cycle: `dest <- dest` writes
    /// nothing observable and the emitter drops it.
    #[test]
    fn self_copy_does_not_create_a_spurious_cycle() {
        let copies = vec![(10, Some(10)), (20, Some(30))];
        let plan = plan_edge_copies(&copies, CopyOrderPolicy::Acyclic);
        assert!(plan.cyclic.is_empty(), "got {plan:?}");
        // The self-copy is in neither set; only the real copy is ordered.
        assert_eq!(plan.ordered, vec![1]);
    }

    /// A constant source reads no register, so it neither joins a cycle nor
    /// blocks the copies that must precede it.
    #[test]
    fn constant_source_imposes_no_precedence() {
        // 0: a <- const, 1: b <- a   => b must be copied before a is overwritten.
        let copies = vec![(10, None), (20, Some(10))];
        let plan = plan_edge_copies(&copies, CopyOrderPolicy::Acyclic);
        assert!(plan.cyclic.is_empty(), "got {plan:?}");
        assert_eq!(plan.ordered, vec![1, 0]);
    }

    /// The plan must be deterministic: repeated runs over the same input give
    /// the same order. Codegen is compared byte-for-byte across builds, so a
    /// hash-iteration-order leak here would show up as nondeterministic asm.
    #[test]
    fn plan_is_deterministic_across_repeated_runs() {
        let copies = vec![
            (1, Some(90)),
            (2, Some(1)),
            (3, Some(2)),
            (4, Some(3)),
            (5, Some(91)),
            (6, Some(5)),
            (7, Some(6)),
            (8, Some(7)),
        ];
        for policy in BOTH_POLICIES {
            let first = plan_edge_copies(&copies, policy);
            for _ in 0..32 {
                let again = plan_edge_copies(&copies, policy);
                assert_eq!(
                    again.ordered, first.ordered,
                    "{policy:?} is not deterministic"
                );
                assert_eq!(again.cyclic.len(), first.cyclic.len());
            }
        }
    }

    /// Opt-in gate: the default selects [`plan_edge_copies_legacy`], which must
    /// still reproduce the historical over-approximation exactly, and
    /// `CCC_PHI_ACYCLIC_ORDER=1` selects the cycle-accurate resolver, so a
    /// suspected regression can be attributed without a rebuild.
    ///
    /// Both arms are called directly rather than through the environment: the
    /// test threads run in parallel, and an env mutation here would leak into
    /// every other test in the process (the hazard `MS-04` exists for). The
    /// env-to-policy wiring itself is pinned by
    /// `tests/regression/check_phi_acyclic_order.sh`.
    #[test]
    fn legacy_switch_policy_restores_the_over_approximation() {
        let chain = vec![(10, Some(20)), (20, Some(30))];
        let legacy = plan_edge_copies_legacy(&chain);
        let current = plan_edge_copies(&chain, CopyOrderPolicy::Acyclic);

        assert_eq!(
            legacy.cyclic.len(),
            2,
            "the legacy path marks the whole chain conflicting"
        );
        assert!(
            current.cyclic.is_empty(),
            "the cycle-accurate path does not"
        );

        // The legacy arm must remain sound, not merely conservative: it is
        // still a valid plan for the same exhaustive oracle.
        let state: Vec<u32> = vec![100, 107, 114];
        let copies = vec![(0, Some(1)), (1, Some(2))];
        let lp = plan_edge_copies_legacy(&copies);
        assert_eq!(
            simultaneous_result(&copies, &state),
            sequential_result(&copies, &lp, &state),
            "the legacy fallback must also be semantically equivalent"
        );
    }

    const CONST_TOKEN: u32 = u32::MAX;

    /// Reference semantics of a phi block: every destination receives the
    /// value its source held BEFORE any copy on this edge ran.
    fn simultaneous_result(copies: &[(u32, Option<u32>)], state: &[u32]) -> Vec<u32> {
        let mut out = state.to_vec();
        for &(dest, src) in copies {
            out[dest as usize] = match src {
                Some(s) => state[s as usize],
                None => CONST_TOKEN,
            };
        }
        out
    }

    /// What the emitter actually computes: Pass 1 saves the cyclic sources
    /// into temporaries, Pass 2 runs the direct copies in `plan.ordered`, and
    /// the target block then restores each cyclic destination from its
    /// temporary.
    fn sequential_result(
        copies: &[(u32, Option<u32>)],
        plan: &EdgeCopyPlan,
        state: &[u32],
    ) -> Vec<u32> {
        let mut cur = state.to_vec();
        let read = |cur: &[u32], src: Option<u32>| match src {
            Some(s) => cur[s as usize],
            None => CONST_TOKEN,
        };
        let mut temps: Vec<Option<u32>> = vec![None; copies.len()];
        for (i, &(_, src)) in copies.iter().enumerate() {
            if plan.cyclic.contains(&i) {
                temps[i] = Some(read(&cur, src));
            }
        }
        for &i in &plan.ordered {
            if plan.cyclic.contains(&i) {
                continue;
            }
            let (dest, src) = copies[i];
            let v = read(&cur, src);
            cur[dest as usize] = v;
        }
        for (i, &(dest, _)) in copies.iter().enumerate() {
            if let Some(v) = temps[i] {
                cur[dest as usize] = v;
            }
        }
        cur
    }

    /// Exhaustive oracle: for EVERY parallel-copy graph over `n` values, the
    /// emitted sequence must be indistinguishable from the simultaneous
    /// assignment the phis denote.
    ///
    /// This is the property the whole transform rests on, and enumeration is
    /// the only way to cover the shapes a hand-written test never thinks of —
    /// chains feeding cycles, cycles feeding chains, duplicated sources,
    /// self-copies, constants mixed in. Each destination independently takes
    /// one of `n` value sources, a constant, or no phi at all.
    ///
    /// The suite also guards against being vacuously green: a resolver that
    /// marked everything cyclic would pass the equivalence check while
    /// emitting a temporary for every copy, so each `n` additionally asserts
    /// that the majority of small graphs are recognised as acyclic.
    #[test]
    fn resolved_order_is_semantically_equivalent_exhaustively() {
        for &n in &[2usize, 3, 4, 5] {
            let choices = n + 2; // n value sources + constant + absent
            let total = choices.pow(n as u32);
            let mut checked = 0usize;
            let mut acyclic = 0usize;
            for code in 0..total {
                let mut copies: Vec<(u32, Option<u32>)> = Vec::new();
                let mut rest = code;
                for dest in 0..n as u32 {
                    let choice = rest % choices;
                    rest /= choices;
                    if choice == n {
                        continue; // this destination has no phi on this edge
                    }
                    let src = if choice == n + 1 {
                        None
                    } else {
                        Some(choice as u32)
                    };
                    copies.push((dest, src));
                }
                if copies.is_empty() {
                    continue;
                }
                let plan = plan_edge_copies(&copies, CopyOrderPolicy::Acyclic);

                // The two sets must partition every copy the emitter emits.
                let mut seen: FxHashSet<usize> = FxHashSet::default();
                for &i in &plan.ordered {
                    assert!(seen.insert(i), "index {i} repeated in ordered: {copies:?}");
                }
                for &i in &plan.cyclic {
                    assert!(
                        seen.insert(i),
                        "index {i} is both ordered and cyclic: {copies:?}"
                    );
                }
                for (i, &(d, s)) in copies.iter().enumerate() {
                    if s == Some(d) {
                        continue; // self-copy: dropped by the emitter
                    }
                    assert!(seen.contains(&i), "index {i} missing from plan: {copies:?}");
                }

                let state: Vec<u32> = (0..n as u32).map(|v| 100 + v * 7).collect();
                let want = simultaneous_result(&copies, &state);
                let got = sequential_result(&copies, &plan, &state);
                assert_eq!(
                    want, got,
                    "n={n}: emission is not equivalent to the parallel assignment\ncopies={copies:?}\nplan={plan:?}"
                );
                checked += 1;
                if plan.cyclic.is_empty() {
                    acyclic += 1;
                }
            }
            assert!(checked > 0, "n={n}: nothing was enumerated");
            assert!(
                acyclic * 2 > checked,
                "n={n}: only {acyclic}/{checked} graphs resolved without a temporary — \
                 the resolver is degenerating towards the legacy over-approximation"
            );
        }
    }

    /// The mechanism end to end: a loop-header phi block whose incoming copies
    /// form an acyclic rotation must leave `eliminate_phis` with NO temporary
    /// values and NO copies in the header — the rotation becomes ordered
    /// direct copies on the latch edge.
    ///
    /// This is the pin for the SHA-256 state rotation. Before the fix the
    /// header carried one unconditional copy per phi plus one fresh temporary
    /// web each, so the allocator saw twice the loop-carried values and staged
    /// the rotation through the stack.
    #[test]
    fn rotation_loop_emits_ordered_direct_copies_without_temporaries() {
        //  block0 (preheader) -> block1 (header, 3 phis) -> block2 (latch) -> block1
        //                                            \-----> block3 (exit)
        let phi = |dest: u32, pre: u32, latch: u32| Instruction::Phi {
            dest: Value(dest),
            ty: crate::common::types::IrType::I32,
            incoming: vec![
                (Operand::Value(Value(pre)), BlockId(0)),
                (Operand::Value(Value(latch)), BlockId(2)),
            ],
        };
        // a=v20 <- {v10, t1}; b=v21 <- {v11, a}; c=v22 <- {v12, b}
        let header = blk(
            1,
            vec![phi(20, 10, 19), phi(21, 11, 20), phi(22, 12, 21), cmp9(22)],
            cond(9, 2, 3),
        );
        let mut func = IrFunction::new(
            "rot".to_string(),
            crate::common::types::IrType::Void,
            Vec::new(),
            false,
        );
        func.blocks = vec![
            blk(0, Vec::new(), Terminator::Branch(BlockId(1))),
            header,
            blk(2, Vec::new(), Terminator::Branch(BlockId(1))),
            blk(3, Vec::new(), Terminator::Return(None)),
        ];
        func.next_value_id = 64;
        func.next_label = 4;

        let mut next_block_id = 4;
        // Explicit policy, not the environment: this pin exists to prove the
        // cycle-accurate resolver lowers the rotation without temporaries, and
        // it must keep proving that while `Legacy` is the shipping default.
        eliminate_phis_with_policy(&mut func, &mut next_block_id, CopyOrderPolicy::Acyclic);

        let by_label = |l: u32| -> &BasicBlock {
            func.blocks
                .iter()
                .find(|b| b.label.0 == l)
                .unwrap_or_else(|| panic!("block {l} disappeared"))
        };

        // No temporary was allocated: the value-id space did not grow past the
        // ids already in play.
        assert!(
            func.next_value_id <= 64,
            "a temporary was allocated (next_value_id {}); the rotation is acyclic",
            func.next_value_id
        );

        // The header carries no copies at all now.
        let header_copies: Vec<_> = by_label(1)
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::Copy { .. }))
            .collect();
        assert!(
            header_copies.is_empty(),
            "header still relays {} copies through temporaries",
            header_copies.len()
        );

        // The latch edge holds the three rotation copies, ordered so that each
        // reads its source before that source is overwritten: c<-b, b<-a, a<-t1.
        let latch: Vec<(u32, u32)> = by_label(2)
            .instructions
            .iter()
            .filter_map(|i| match i {
                Instruction::Copy {
                    dest,
                    src: Operand::Value(Value(s)),
                } => Some((dest.0, *s)),
                _ => None,
            })
            .collect();
        assert_eq!(
            latch,
            vec![(22, 21), (21, 20), (20, 19)],
            "rotation copies are missing or misordered"
        );
    }

    // ── predecessor-copy placement ahead of the branch compare chain ──────

    fn add(d: u32, a: u32) -> Instruction {
        Instruction::BinOp {
            dest: Value(d),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(a)),
            rhs: Operand::Const(crate::ir::reexports::IrConst::I64(1)),
            ty: crate::common::types::IrType::I64,
        }
    }

    fn widen(d: u32, s: u32) -> Instruction {
        Instruction::Cast {
            dest: Value(d),
            src: Operand::Value(Value(s)),
            from_ty: crate::common::types::IrType::I32,
            to_ty: crate::common::types::IrType::I64,
        }
    }

    /// `Instruction` has no `PartialEq`; its `Debug` form is structural.
    fn same(a: &[Instruction], b: &[Instruction]) -> bool {
        format!("{a:?}") == format!("{b:?}")
    }

    #[track_caller]
    fn assert_same(got: &[Instruction], want: &[Instruction]) {
        assert!(same(got, want), "got {got:?}\nwant {want:?}");
    }

    fn placed(
        insts: Vec<Instruction>,
        term: Terminator,
        copies: Vec<Instruction>,
    ) -> Vec<Instruction> {
        let mut b = blk(1, insts, term);
        place_pred_copies(&mut b, copies);
        b.instructions
    }

    /// The rotated-latch shape: `i' = i + 1; c = i' != 0; br c` with the
    /// backedge copy `i = i'`. The copy goes ahead of the compare, which
    /// then reads the copy's destination, so `i'` still dies at the copy.
    #[test]
    fn latch_copies_precede_the_compare_and_carry_its_operand() {
        let got = placed(vec![add(13, 10), cmp9(13)], cond(9, 1, 2), vec![cp(10, 13)]);
        assert_same(&got, &[add(13, 10), cp(10, 13), cmp9(10)]);
    }

    /// `Copy`/integer `Cast` links between the compare and the branch stay
    /// with the compare (the backends fuse through them).
    #[test]
    fn the_condition_chain_moves_with_its_compare() {
        let got = placed(
            vec![add(13, 10), cmp9(13), widen(20, 9), cp(21, 20)],
            cond(21, 1, 2),
            vec![cp(10, 13)],
        );
        assert_same(
            &got,
            &[add(13, 10), cp(10, 13), cmp9(10), widen(20, 9), cp(21, 20)],
        );
    }

    /// A compare that reads a copy destination must see the old value: the
    /// copies stay last.
    #[test]
    fn a_compare_reading_a_destination_keeps_the_copies_last() {
        let got = placed(vec![add(13, 10), cmp9(10)], cond(9, 1, 2), vec![cp(10, 13)]);
        assert_same(&got, &[add(13, 10), cmp9(10), cp(10, 13)]);
    }

    /// A copy reading a value the tail defines (a carried boolean, or a
    /// chain link) cannot run before that definition.
    #[test]
    fn a_copy_of_a_tail_value_keeps_the_copies_last() {
        let got = placed(vec![cmp9(13)], cond(9, 1, 2), vec![cp(10, 9)]);
        assert_same(&got, &[cmp9(13), cp(10, 9)]);
        let got = placed(
            vec![cmp9(13), widen(20, 9)],
            cond(20, 1, 2),
            vec![cp(10, 20)],
        );
        assert_same(&got, &[cmp9(13), widen(20, 9), cp(10, 20)]);
    }

    /// The pointer-induction latch: `i' = i + 1; c = i' < n; p' = p + 4; br c`.
    /// The compare sinks past the step and below the copies, reading the
    /// copy destination of `i'`.
    #[test]
    fn the_chain_sinks_past_an_independent_instruction() {
        let got = placed(
            vec![add(13, 10), cmp9(13), add(14, 11)],
            cond(9, 1, 2),
            vec![cp(10, 13), cp(11, 14)],
        );
        assert_same(
            &got,
            &[add(13, 10), add(14, 11), cp(10, 13), cp(11, 14), cmp9(10)],
        );
        // Members interleaved with other instructions keep their order.
        let got = placed(
            vec![cmp9(13), add(14, 11), widen(20, 9), add(15, 12)],
            cond(20, 1, 2),
            vec![cp(11, 14)],
        );
        assert_same(
            &got,
            &[add(14, 11), add(15, 12), cp(11, 14), cmp9(13), widen(20, 9)],
        );
    }

    /// Each sinking condition refuses on its own: a later instruction reading
    /// the compare's result, one redefining a value the compare reads, a call
    /// in between, a condition defined by something that is not a chain
    /// link, a terminator that is not a conditional branch, no copies.
    #[test]
    fn a_chain_that_cannot_sink_keeps_the_copies_last() {
        let got = placed(vec![cmp9(13), add(14, 9)], cond(9, 1, 2), vec![cp(10, 14)]);
        assert_same(&got, &[cmp9(13), add(14, 9), cp(10, 14)]);
        let got = placed(vec![cmp9(13), add(13, 12)], cond(9, 1, 2), vec![cp(10, 12)]);
        assert_same(&got, &[cmp9(13), add(13, 12), cp(10, 12)]);
        let call = Instruction::Call {
            func: "f".to_string(),
            info: crate::ir::reexports::CallInfo::default(),
        };
        let got = placed(
            vec![cmp9(13), call.clone()],
            cond(9, 1, 2),
            vec![cp(10, 12)],
        );
        assert_same(&got, &[cmp9(13), call, cp(10, 12)]);
        let got = placed(vec![cmp9(13), add(20, 9)], cond(20, 1, 2), vec![cp(10, 12)]);
        assert_same(&got, &[cmp9(13), add(20, 9), cp(10, 12)]);
        let got = placed(vec![cmp9(13)], cond(13, 1, 2), vec![cp(10, 12)]);
        assert_same(&got, &[cmp9(13), cp(10, 12)]);
        let got = placed(
            vec![cmp9(13)],
            Terminator::Branch(BlockId(1)),
            vec![cp(10, 13)],
        );
        assert_same(&got, &[cmp9(13), cp(10, 13)]);
        let got = placed(vec![cp(10, 13)], cond(9, 1, 2), Vec::new());
        assert_same(&got, &[cp(10, 13)]);
    }

    /// Source spans stay index-parallel with the instructions.
    #[test]
    fn spans_are_inserted_at_the_copy_position() {
        let mut b = blk(1, vec![add(13, 10), cmp9(13)], cond(9, 1, 2));
        let real = crate::common::source::Span::new(4, 9, 0);
        b.source_spans = vec![real, real];
        place_pred_copies(&mut b, vec![cp(10, 13), cp(11, 12)]);
        assert_eq!(b.instructions.len(), b.source_spans.len());
        let dummy = crate::common::source::Span::dummy();
        assert_eq!(b.source_spans, vec![real, dummy, dummy, real]);

        // A sunk chain takes its spans along.
        let (s1, s2, s3) = (
            crate::common::source::Span::new(1, 2, 0),
            crate::common::source::Span::new(3, 4, 0),
            crate::common::source::Span::new(5, 6, 0),
        );
        let mut b = blk(1, vec![add(13, 10), cmp9(13), add(14, 11)], cond(9, 1, 2));
        b.source_spans = vec![s1, s2, s3];
        place_pred_copies(&mut b, vec![cp(10, 13)]);
        assert_eq!(b.source_spans, vec![s1, s3, dummy, s2]);

        // Spans that are not index-aligned (a drift some earlier pass left,
        // reported by the verifier against that pass) cannot be permuted:
        // the copies are appended, nothing is spliced out of range, and the
        // drifted list is left exactly as found -- the `BasicBlock` span
        // primitives' contract (`span_edit_tests`).
        let mut b = blk(1, vec![add(13, 10), cmp9(13)], cond(9, 1, 2));
        b.source_spans = vec![real];
        place_pred_copies(&mut b, vec![cp(10, 13)]);
        assert_same(&b.instructions, &[add(13, 10), cmp9(13), cp(10, 13)]);
        assert_eq!(b.source_spans, vec![real]);
    }

    /// Exhaustive-by-sampling equivalence: for random blocks (compare chains
    /// interleaved with foreign instructions, some reading the chain, some
    /// redefining its operands) and random copy groups (parallel ones and
    /// sequential ones with cycle temporaries), running the block as placed by
    /// `place_pred_copies` must leave every value exactly as the reference
    /// placement (copies last) does.
    #[test]
    fn placement_preserves_the_branch_and_every_copy_destination() {
        use crate::ir::reexports::{IrCmpOp, IrConst};
        fn run(insts: &[Instruction], init: &FxHashMap<u32, i64>) -> FxHashMap<u32, i64> {
            let mut env = init.clone();
            let val = |env: &FxHashMap<u32, i64>, op: &Operand| match op {
                Operand::Value(v) => env[&v.0],
                Operand::Const(IrConst::I64(c)) => *c,
                Operand::Const(_) => unreachable!(),
            };
            for inst in insts {
                let (d, x) = match inst {
                    Instruction::Copy { dest, src } => (dest.0, val(&env, src)),
                    Instruction::Cast { dest, src, .. } => (dest.0, val(&env, src)),
                    Instruction::BinOp { dest, lhs, rhs, .. } => {
                        (dest.0, val(&env, lhs).wrapping_add(val(&env, rhs)))
                    }
                    Instruction::Cmp { dest, lhs, rhs, .. } => {
                        (dest.0, i64::from(val(&env, lhs) < val(&env, rhs)))
                    }
                    _ => unreachable!(),
                };
                env.insert(d, x);
            }
            env
        }
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = |n: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % u64::from(n)) as u32
        };
        // Values 1..=6 are live-in, 7 and 8 are computed in the block, 9 is
        // the compare, 20/21 are condition links, 30.. are temporaries.
        let mut moved = 0;
        for _ in 0..20_000 {
            let mut insts = vec![add(7, 1 + next(6)), add(8, 1 + next(7))];
            let pick = |r: u32| if r < 8 { 1 + r } else { 9 };
            insts.push(Instruction::Cmp {
                dest: Value(9),
                op: IrCmpOp::Slt,
                lhs: Operand::Value(Value(1 + next(8))),
                rhs: Operand::Value(Value(1 + next(8))),
                ty: crate::common::types::IrType::I64,
            });
            let mut cond_v = 9;
            if next(2) == 0 {
                insts.push(widen(20, 9));
                cond_v = 20;
                if next(2) == 0 {
                    insts.push(cp(21, 20));
                    cond_v = 21;
                }
            }
            // Foreign instructions anywhere after the compare: fresh values
            // (40..) read from anywhere -- the chain included -- and, now and
            // then, a redefinition of a live-in the compare may read.
            for k in 0..next(3) {
                let at = 3 + next(insts.len() as u32 - 2) as usize;
                let src = match next(6) {
                    0 => [9, 20, 21][next(3) as usize],
                    _ => 1 + next(8),
                };
                let dest = if next(8) == 0 { 1 + next(8) } else { 40 + k };
                insts.insert(at, add(dest, src));
            }
            // A copy group: distinct destinations among 1..=8 / 30..,
            // sources anywhere (including the compare and its links).
            let mut copies = Vec::new();
            let mut used = FxHashSet::default();
            for k in 0..(1 + next(3)) {
                let d = if next(4) == 0 { 30 + k } else { 1 + next(8) };
                if !used.insert(d) {
                    continue;
                }
                let s = match next(5) {
                    0 => cond_v,
                    1 if d != 30 + k => 30 + next(3),
                    _ => pick(next(9)),
                };
                copies.push(cp(d, s));
            }
            let term = cond(cond_v, 1, 2);
            let mut reference = insts.clone();
            reference.extend(copies.iter().cloned());
            let got = placed(insts.clone(), term, copies.clone());
            if !same(&got, &reference) {
                moved += 1;
            }
            let mut init: FxHashMap<u32, i64> = FxHashMap::default();
            for v in (1..=9).chain(20..=21).chain(30..=33).chain(40..=42) {
                init.insert(v, i64::from(next(5)) - 2);
            }
            // Every value -- the branch condition, each copy destination,
            // each chain link and foreign result -- ends up identical.
            let a = run(&reference, &init);
            let b = run(&got, &init);
            assert_eq!(a, b, "state differs: {reference:?} vs {got:?}");
        }
        // The property must have been exercised on moved placements, not
        // only on refusals.
        assert!(moved > 1000, "only {moved} placements moved");
    }

    // ── uniform edge copies ────────────────────────────────────────────────

    /// Block 1 branches to 2 and 3; each target's phi (already sharing the
    /// name `dest`, as after phi-web coalescing) takes `src` from block 1.
    fn uniform_edges(dest: u32, src2: u32, src3: u32) -> IrFunction {
        let phi = |d: u32, s: u32| Instruction::Phi {
            dest: Value(d),
            ty: crate::common::types::IrType::I64,
            incoming: vec![(Operand::Value(Value(s)), BlockId(1))],
        };
        let mut func = IrFunction::new(
            "u".to_string(),
            crate::common::types::IrType::I64,
            Vec::new(),
            false,
        );
        func.blocks = vec![
            blk(0, Vec::new(), Terminator::Branch(BlockId(1))),
            blk(1, vec![cmp9(5)], cond(9, 2, 3)),
            blk(
                2,
                vec![phi(dest, src2)],
                Terminator::Return(Some(Operand::Value(Value(dest)))),
            ),
            blk(
                3,
                vec![phi(dest, src3)],
                Terminator::Return(Some(Operand::Value(Value(dest)))),
            ),
        ];
        func.next_value_id = 64;
        func.next_label = 4;
        func
    }

    fn lower(mut func: IrFunction) -> IrFunction {
        let mut next_block_id = 4;
        eliminate_phis_with_policy(&mut func, &mut next_block_id, CopyOrderPolicy::Acyclic);
        func
    }

    #[test]
    fn identical_copies_on_every_edge_run_once_before_the_branch() {
        let func = lower(uniform_edges(30, 5, 5));
        assert_eq!(func.blocks.len(), 4, "no trampoline is needed");
        let b1: Vec<_> = func.blocks[1]
            .instructions
            .iter()
            .filter_map(|i| match i {
                Instruction::Copy {
                    dest,
                    src: Operand::Value(Value(s)),
                } => Some((dest.0, *s)),
                _ => None,
            })
            .collect();
        assert_eq!(b1, vec![(30, 5)]);
        assert!(matches!(
            func.blocks[1].terminator,
            Terminator::CondBranch {
                true_label: BlockId(2),
                false_label: BlockId(3),
                ..
            }
        ));
    }

    /// A loop of one block: `v30 = phi(v5, v31); v31 = v30 + 1;`, left by
    /// `exit` and repeated by its terminator.  Nothing outside reads `v30`,
    /// so the copy `v30 = v31` of the back edge may be absorbed into the
    /// block's end instead of costing a latch block.
    fn one_block_loop(exit: Option<Instruction>) -> IrFunction {
        use crate::common::types::IrType;
        let carried = Instruction::Phi {
            dest: Value(30),
            ty: IrType::I64,
            incoming: vec![
                (Operand::Value(Value(5)), BlockId(0)),
                (Operand::Value(Value(31)), BlockId(1)),
            ],
        };
        let step = Instruction::BinOp {
            dest: Value(31),
            op: crate::ir::reexports::IrBinOp::Add,
            lhs: Operand::Value(Value(30)),
            rhs: Operand::Const(crate::ir::reexports::IrConst::I64(1)),
            ty: IrType::I64,
        };
        let mut body = vec![carried, step];
        let leaves_by_asm_goto = exit.is_some();
        body.extend(exit);
        let mut func = IrFunction::new("l".to_string(), IrType::I64, Vec::new(), false);
        func.blocks = vec![
            blk(0, vec![cp(5, 4), cp(8, 4)], Terminator::Branch(BlockId(1))),
            blk(
                1,
                body,
                if leaves_by_asm_goto {
                    Terminator::Branch(BlockId(1))
                } else {
                    cond(8, 1, 2)
                },
            ),
            blk(
                2,
                Vec::new(),
                Terminator::Return(Some(Operand::Value(Value(8)))),
            ),
        ];
        func.next_value_id = 64;
        func.next_label = 3;
        func
    }

    #[test]
    fn nothing_is_put_behind_an_asm_goto_not_even_a_self_loop_copy() {
        // Left by a branch, the block absorbs the copy: three blocks stay
        // three, and the copy is the block's last instruction.
        let plain = lower(one_block_loop(None));
        assert_eq!(plain.blocks.len(), 3);
        assert!(matches!(
            plain.blocks[1].instructions.last(),
            Some(Instruction::Copy {
                dest: Value(30),
                ..
            })
        ));

        // Left by an asm goto, the block ends at the asm goto.  The copy
        // gets a latch block of its own; at the block's end it would stand
        // behind the asm goto, where liveness and every later reader of the
        // block take it to have run on the way to the label too.
        let asm_goto = Instruction::InlineAsm {
            template: "jmp %l0".to_string(),
            outputs: vec![],
            inputs: vec![],
            clobbers: vec![],
            operand_types: vec![],
            goto_labels: vec![("out".to_string(), BlockId(2))],
            input_symbols: vec![],
            seg_overrides: vec![],
        };
        let func = lower(one_block_loop(Some(asm_goto)));
        assert!(matches!(
            func.blocks[1].instructions.last(),
            Some(Instruction::InlineAsm { .. })
        ));
        let Terminator::Branch(latch) = func.blocks[1].terminator else {
            panic!("the loop block still ends in a plain branch");
        };
        assert_ne!(
            latch,
            BlockId(1),
            "the back edge goes through a latch block"
        );
        let latch_block = func
            .blocks
            .iter()
            .find(|block| block.label == latch)
            .expect("the latch block exists");
        assert!(matches!(
            latch_block.instructions.as_slice(),
            [Instruction::Copy {
                dest: Value(30),
                src: Operand::Value(Value(31)),
            }]
        ));
        assert!(matches!(
            latch_block.terminator,
            Terminator::Branch(BlockId(1))
        ));
    }

    #[test]
    fn differing_edges_or_a_destination_the_branch_reads_keep_trampolines() {
        // Different sources per edge.
        assert_eq!(lower(uniform_edges(30, 5, 6)).blocks.len(), 6);
        // The copy writes the branch condition itself.
        assert_eq!(lower(uniform_edges(9, 5, 5)).blocks.len(), 6);
    }

    // ── guard deferral + computed-incoming folding ─────────────────────────

    use crate::common::fx_hash::FxHashSet as TestFxHashSet;

    /// A ready copy whose destination is read by another copy's *computed*
    /// source must wait for that computation to be folded first, so the sink
    /// can place the definition at its slot without crossing a clobber.
    #[test]
    fn guard_deferral_holds_back_clobbers_of_computed_sources() {
        let copies = vec![(10, Some(11)), (12, Some(13))];
        // No guards: plain Kahn ascending order.
        let plain = plan_edge_copies(&copies, CopyOrderPolicy::Acyclic);
        assert_eq!(plain.ordered, vec![0, 1]);
        // guards[1] = {10}: copy 1's computed source reads v10, which copy 0
        // writes. Copy 0 must be deferred until copy 1 has run.
        let mut guards: Vec<TestFxHashSet<u32>> = vec![TestFxHashSet::default(); 2];
        guards[1].insert(10);
        let planned = plan_edge_copies_guards(&copies, &guards, CopyOrderPolicy::Acyclic);
        assert_eq!(
            planned.ordered,
            vec![1, 0],
            "the clobbering copy was not deferred past the guarded computation"
        );
        // The plan must remain a valid sequential execution of the parallel
        // assignment. (State covers every id the copies name.)
        let state: Vec<u32> = (0..14).map(|i| 100 + i * 7).collect();
        assert_eq!(
            simultaneous_result(&copies, &state),
            sequential_result(&copies, &planned, &state),
            "guard deferral changed the parallel-copy semantics"
        );
    }

    /// End to end, the SHA-256 rotation shape: phis a..h where the latch
    /// incoming of `e` is `d + t1` and that of `a` is `t1 + t2` (both computed
    /// in the latch). The guard-aware planner must schedule `d <- c` behind
    /// `e`'s slot and everything reading old `a` behind `a`'s slot, and the
    /// sink must seat each defining BinOp DIRECTLY before its relay copy —
    /// collapsing the relay value's live interval to zero so the register
    /// allocator's copy coalescer can union it with the phi home. The IR def
    /// set is untouched (the SSA validator's post-elimination contract).
    #[test]
    fn computed_incoming_folds_to_its_copy_slot() {
        let mut func = IrFunction::new(
            "folds".to_string(),
            crate::common::types::IrType::Void,
            Vec::new(),
            false,
        );
        let phi = |dest: u32, pre: u32, latch: u32| Instruction::Phi {
            dest: Value(dest),
            ty: crate::common::types::IrType::I32,
            incoming: vec![
                (Operand::Value(Value(pre)), BlockId(0)),
                (Operand::Value(Value(latch)), BlockId(2)),
            ],
        };
        let add = |dest: u32, lhs: u32, rhs: u32| Instruction::BinOp {
            dest: Value(dest),
            op: crate::ir::ops::IrBinOp::Add,
            lhs: Operand::Value(Value(lhs)),
            rhs: Operand::Value(Value(rhs)),
            ty: crate::common::types::IrType::I32,
        };
        func.blocks = vec![
            blk(0, Vec::new(), Terminator::Branch(BlockId(1))),
            blk(
                1,
                vec![
                    phi(20, 10, 41),
                    phi(21, 11, 20),
                    phi(22, 12, 21),
                    phi(23, 13, 22),
                    phi(24, 14, 40),
                    phi(25, 15, 24),
                    phi(26, 16, 25),
                    phi(27, 17, 26),
                    cmp9(27),
                ],
                cond(9, 2, 3),
            ),
            blk(
                2,
                vec![add(40, 23, 14), add(41, 40, 15)],
                Terminator::Branch(BlockId(1)),
            ),
            blk(3, Vec::new(), Terminator::Return(None)),
        ];
        func.next_value_id = 64;
        func.next_label = 4;

        let mut next_block_id = 4;
        eliminate_phis_with_policy(&mut func, &mut next_block_id, CopyOrderPolicy::Acyclic);

        let latch_block = func
            .blocks
            .iter()
            .find(|b| b.label.0 == 2)
            .expect("latch disappeared");
        // No temporary was minted (the rotation is acyclic) and the def set
        // is untouched: both computations still define their own values.
        assert!(
            func.next_value_id <= 64,
            "a temporary was allocated for an acyclic computed incoming"
        );
        for computed in [40u32, 41u32] {
            assert!(
                latch_block.instructions.iter().any(|i| matches!(i,
                    Instruction::BinOp { dest: Value(d), .. } if *d == computed)),
                "v{computed} lost its definition"
            );
        }
        // Each relay copy is immediately preceded by the BinOp defining its
        // source (the sink), and the guard-aware order holds: e's slot sits
        // before `d <- c` (the operand clobber), a's slot is last.
        let seq: Vec<u32> = latch_block
            .instructions
            .iter()
            .filter_map(|i| match i {
                Instruction::BinOp { dest: Value(d), .. } => Some(*d),
                Instruction::Copy {
                    dest: Value(d),
                    src: Operand::Value(Value(s)),
                } => Some(1_000_000 + 100 * d + s), // encode copy (d, s)
                _ => None,
            })
            .collect();
        // copies: h<-g=27/26 g<-f=26/25 f<-e=25/24 e<-t1=24/40
        //         d<-c=23/22 c<-b=22/21 b<-a=21/20 a<-t2=20/41
        let expected: Vec<u32> = [
            1_000_000 + 100 * 27 + 26,
            1_000_000 + 100 * 26 + 25,
            1_000_000 + 100 * 25 + 24,
            40, // e = d + t1, sunk directly before its copy
            1_000_000 + 100 * 24 + 40,
            1_000_000 + 100 * 23 + 22,
            1_000_000 + 100 * 22 + 21,
            1_000_000 + 100 * 21 + 20,
            41, // a = t1 + t2, sunk directly before its copy
            1_000_000 + 100 * 20 + 41,
        ]
        .to_vec();
        assert_eq!(
            seq, expected,
            "sink did not seat the definitions at their copy slots"
        );
    }

    /// Sinking is position-only: when the home has a pending reader (the
    /// `21 <- 20` copy) the sink fires, and a downstream reader of the
    /// relayed VALUE (here: the terminator's condition) stays untouched —
    /// the definition still dominates it because it sits directly before
    /// its copy, still above the terminator.
    #[test]
    fn fold_sinks_definition_without_touching_readers() {
        let mut func = IrFunction::new(
            "multiuse".to_string(),
            crate::common::types::IrType::Void,
            Vec::new(),
            false,
        );
        let latch_add = Instruction::BinOp {
            dest: Value(40),
            op: crate::ir::ops::IrBinOp::Add,
            lhs: Operand::Value(Value(21)),
            rhs: Operand::Const(crate::ir::reexports::IrConst::I32(1)),
            ty: crate::common::types::IrType::I32,
        };
        func.blocks = vec![
            blk(
                0,
                vec![latch_add],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(40)),
                    true_label: BlockId(1),
                    false_label: BlockId(1),
                },
            ),
            blk(1, Vec::new(), Terminator::Return(None)),
        ];
        func.next_value_id = 64;
        func.next_label = 2;
        // A pending reader of the phi home 20 keeps the sink profitable:
        // seating v40 in home 20 at the add's original position would
        // clobber the value this copy drains.
        func.blocks[0].instructions.push(Instruction::Copy {
            dest: Value(22),
            src: Operand::Value(Value(20)),
        });
        // The trailing copy run the pass scans:
        func.blocks[0].instructions.push(Instruction::Copy {
            dest: Value(20),
            src: Operand::Value(Value(40)),
        });
        fold_computed_phi_incomings(&mut func);
        let b = &func.blocks[0];
        // The home-reading copy keeps its position; the definition moved to
        // directly before the relay copy; the terminator still reads v40.
        assert_eq!(
            b.instructions.len(),
            3,
            "instruction count changed: {:?}",
            b.instructions
        );
        assert!(
            matches!(
                b.instructions.first(),
                Some(Instruction::Copy {
                    dest: Value(22),
                    src: Operand::Value(Value(20))
                })
            ),
            "the pending home reader moved: {:?}",
            b.instructions
        );
        assert!(
            matches!(
                b.instructions[1],
                Instruction::BinOp {
                    dest: Value(40),
                    ..
                }
            ),
            "definition did not sink to directly before its copy: {:?}",
            b.instructions
        );
        assert!(
            matches!(
                b.instructions[2],
                Instruction::Copy {
                    dest: Value(20),
                    src: Operand::Value(Value(40))
                }
            ),
            "relay copy was disturbed: {:?}",
            b.instructions
        );
        assert!(
            matches!(
                b.terminator,
                Terminator::CondBranch {
                    cond: Operand::Value(Value(40)),
                    ..
                }
            ),
            "terminator reader was modified"
        );
    }

    /// The profitability predicate must REFUSE the sink when the phi home has
    /// no pending reader across the window and none in the dominated region
    /// (a single-incoming accumulator shape): the allocator coalesces the
    /// relay across the disjoint intervals anyway, and moving the
    /// computation measured a real regression (spectral_norm decomposed
    /// `vfmadd231sd` into mul+add for zero copies saved). The definition
    /// starts AWAY from its copy, so a sink is observable if it wrongly
    /// fires.
    #[test]
    fn dead_home_computed_incoming_is_not_sunk() {
        let mut func = IrFunction::new(
            "deadhome".to_string(),
            crate::common::types::IrType::Void,
            Vec::new(),
            false,
        );
        let add = |dest: u32, lhs: u32, rhs: u32| Instruction::BinOp {
            dest: Value(dest),
            op: crate::ir::ops::IrBinOp::Add,
            lhs: Operand::Value(Value(lhs)),
            rhs: Operand::Value(Value(rhs)),
            ty: crate::common::types::IrType::I32,
        };
        func.blocks = vec![
            blk(
                0,
                vec![
                    add(40, 1, 2),
                    // Neutral filler: touches neither v40 nor the home 20,
                    // so only the profitability predicate can refuse.
                    Instruction::Copy {
                        dest: Value(50),
                        src: Operand::Value(Value(51)),
                    },
                    Instruction::Copy {
                        dest: Value(20),
                        src: Operand::Value(Value(40)),
                    },
                ],
                Terminator::Branch(BlockId(1)),
            ),
            blk(1, Vec::new(), Terminator::Return(None)),
        ];
        func.next_value_id = 64;
        func.next_label = 2;
        fold_computed_phi_incomings(&mut func);
        let b = &func.blocks[0];
        let seq: Vec<u32> = b
            .instructions
            .iter()
            .map(|inst| match inst {
                Instruction::BinOp { dest, .. } | Instruction::Copy { dest, .. } => dest.0,
                _ => u32::MAX,
            })
            .collect();
        assert_eq!(
            seq,
            vec![40, 50, 20],
            "definition moved even though the home had no pending reader"
        );
    }

    /// A use in a DIFFERENT block refuses the fold: that block reads a value
    /// this edge does not define, so the rename would strand it.
    #[test]
    fn fold_refuses_cross_block_readers() {
        let mut func = IrFunction::new(
            "xblock".to_string(),
            crate::common::types::IrType::Void,
            Vec::new(),
            false,
        );
        let latch_add = Instruction::BinOp {
            dest: Value(40),
            op: crate::ir::ops::IrBinOp::Add,
            lhs: Operand::Value(Value(21)),
            rhs: Operand::Const(crate::ir::reexports::IrConst::I32(1)),
            ty: crate::common::types::IrType::I32,
        };
        func.blocks = vec![
            blk(
                0,
                vec![latch_add],
                Terminator::CondBranch {
                    cond: Operand::Const(crate::ir::reexports::IrConst::I32(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            blk(1, Vec::new(), Terminator::Return(None)),
            blk(
                2,
                vec![Instruction::Copy {
                    dest: Value(50),
                    src: Operand::Value(Value(40)),
                }],
                Terminator::Return(None),
            ),
        ];
        func.next_value_id = 64;
        func.next_label = 3;
        func.blocks[0].instructions.push(Instruction::Copy {
            dest: Value(20),
            src: Operand::Value(Value(40)),
        });
        fold_computed_phi_incomings(&mut func);
        assert!(
            func.blocks[0].instructions.iter().any(|i| matches!(
                i,
                Instruction::Copy {
                    dest: Value(20),
                    src: Operand::Value(Value(40))
                }
            )),
            "fold ignored a cross-block reader of the source"
        );
    }

    /// The fold must refuse when a crossed instruction clobbers an operand of
    /// the computation: sinking past it would read the new value.
    #[test]
    fn fold_refuses_operand_clobber_in_between() {
        let mut func = IrFunction::new(
            "clobber".to_string(),
            crate::common::types::IrType::Void,
            Vec::new(),
            false,
        );
        let latch_add = Instruction::BinOp {
            dest: Value(40),
            op: crate::ir::ops::IrBinOp::Add,
            lhs: Operand::Value(Value(21)),
            rhs: Operand::Const(crate::ir::reexports::IrConst::I32(1)),
            ty: crate::common::types::IrType::I32,
        };
        func.blocks = vec![blk(
            0,
            vec![
                latch_add,
                // Clobbers v21 — an operand of the computation above — after
                // it and before the relay copy.
                cp(21, 12),
                Instruction::Copy {
                    dest: Value(20),
                    src: Operand::Value(Value(40)),
                },
            ],
            Terminator::Return(None),
        )];
        func.next_value_id = 64;
        func.next_label = 1;
        fold_computed_phi_incomings(&mut func);
        assert!(
            func.blocks[0].instructions.iter().any(|i| matches!(
                i,
                Instruction::Copy {
                    dest: Value(20),
                    src: Operand::Value(Value(40))
                }
            )),
            "fold crossed an operand clobber"
        );
    }
}
