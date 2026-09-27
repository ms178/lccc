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
//! the block and the copies commute, they are appended to the block itself and
//! the latch disappears. If any destination escapes, the trampoline is built
//! exactly as before, so the transform is strictly a refinement of the old
//! behaviour. Rotation makes this fire in practice: rotating lifts the exit
//! value into a distinct SSA value, so the accumulator the exit phi reads is
//! no longer a phi destination. Set `CCC_NO_PHI_SELFLOOP_HOIST=1` to disable.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IrBinOp, IrFunction, IrModule, Operand, Terminator, Value,
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
        eprintln!(
            "[PHI] Starting phi_eliminate with next_block_id = {} policy={:?}",
            next_block_id, policy
        );
    }

    for func in &mut module.functions {
        if func.is_declaration || func.blocks.is_empty() {
            continue;
        }
        if std::env::var("LCCC_DEBUG_LABELS").is_ok() {
            eprintln!(
                "[PHI] Processing function {}, current labels: {:?}",
                func.name,
                func.blocks.iter().map(|b| b.label.0).collect::<Vec<_>>()
            );
        }
        eliminate_phis_with_policy(func, &mut next_block_id, policy);
        if std::env::var("LCCC_DEBUG_LABELS").is_ok() {
            eprintln!(
                "[PHI] After processing {}, labels: {:?}, next_block_id now {}",
                func.name,
                func.blocks.iter().map(|b| b.label.0).collect::<Vec<_>>(),
                next_block_id
            );
        }
    }
}

/// Returns the number of distinct successor block IDs for a block.
/// Accounts for both terminator targets and InlineAsm goto_labels.
fn successor_count(block: &BasicBlock) -> usize {
    let mut seen: Vec<BlockId> = Vec::new();
    match &block.terminator {
        Terminator::Return(_) | Terminator::Unreachable => {}
        Terminator::Branch(label) => {
            seen.push(*label);
        }
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            seen.push(*true_label);
            if true_label != false_label {
                seen.push(*false_label);
            }
        }
        Terminator::IndirectBranch {
            possible_targets, ..
        } => {
            seen.extend_from_slice(possible_targets);
        }
        Terminator::Switch { cases, default, .. } => {
            seen.push(*default);
            for &(_, label) in cases {
                if !seen.contains(&label) {
                    seen.push(label);
                }
            }
        }
    }
    // InlineAsm goto_labels are implicit control flow edges.
    for inst in &block.instructions {
        if let Instruction::InlineAsm { goto_labels, .. } = inst {
            for (_, label) in goto_labels {
                if !seen.contains(label) {
                    seen.push(*label);
                }
            }
        }
    }
    seen.len()
}

/// Replace one occurrence of `old_target` with `new_target` in a block's
/// terminator or InlineAsm goto_labels.
fn retarget_block_edge_once(block: &mut BasicBlock, old_target: BlockId, new_target: BlockId) {
    match &mut block.terminator {
        Terminator::Branch(t) => {
            if *t == old_target {
                *t = new_target;
                return;
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
                return;
            } else if *false_label == old_target {
                *false_label = new_target;
                return;
            }
        }
        Terminator::IndirectBranch {
            possible_targets, ..
        } => {
            for t in possible_targets.iter_mut() {
                if *t == old_target {
                    *t = new_target;
                    return;
                }
            }
        }
        Terminator::Switch { cases, default, .. } => {
            if *default == old_target {
                *default = new_target;
                return;
            } else {
                for (_, t) in cases.iter_mut() {
                    if *t == old_target {
                        *t = new_target;
                        return;
                    }
                }
            }
        }
        _ => {}
    }
    // Check InlineAsm goto_labels for implicit control flow edges.
    for inst in &mut block.instructions {
        if let Instruction::InlineAsm { goto_labels, .. } = inst {
            for (_, label) in goto_labels.iter_mut() {
                if *label == old_target {
                    *label = new_target;
                    return;
                }
            }
        }
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
                eprintln!(
                    "[PHI] Creating trampoline block with BlockId({}), next will be {}",
                    label.0,
                    *next_block_id + 1
                );
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
            eprintln!(
                "[PHIELIM] block {} phis={:?} pred {} copies={:?}",
                target_block_id.0,
                phis.iter().map(|p| p.dest.0).collect::<Vec<_>>(),
                pred_label.0,
                edge_copies
            );
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
/// A fourth condition is about *profit*, not correctness. Absorbing appends the
/// copies immediately before the terminator. If the block's branch condition is
/// a `Cmp` in this block and one of the copies redefines an operand of it, the
/// copy lands between that `Cmp` and its consumer, so the x86 emitter's
/// compare-replay must decline (see the redefinition guard in
/// `x86/codegen/comparison.rs`) and emit the compare at its own position rather
/// than fusing it into the branch. That costs an operand reload on every
/// iteration -- more than the removed branch is worth -- so the absorption is
/// skipped in exactly that shape and kept wherever removing the latch pays.
fn self_loop_copies_can_hoist(
    block: &BasicBlock,
    copies: &[Instruction],
    uses: &BlockUses,
    block_idx: usize,
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

    for &dest in &dests {
        // Conditions 2 and 3: the destination must be invisible everywhere
        // except this block's own instructions.
        if !uses.no_terminator_reads(dest) || !uses.only_read_within(dest, block_idx) {
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

/// Resolve every buffered self-loop backedge: absorb the copies into the block
/// when that is provably safe, otherwise split the edge as before.
fn resolve_self_loop_copies(func: &IrFunction, ctx: &mut PhiElimCtx) {
    if ctx.self_loop_copies.iter().all(Vec::is_empty) {
        return;
    }
    let enabled = self_loop_hoist_enabled();
    let uses = BlockUses::from_blocks(&func.blocks);
    let buffered = std::mem::take(&mut ctx.self_loop_copies);
    let debug = std::env::var("LCCC_DEBUG_PHI_HOIST").is_ok();

    for (block_idx, copies) in buffered.into_iter().enumerate() {
        if copies.is_empty() {
            continue;
        }
        let target_block_id = func.blocks[block_idx].label;
        let hoist = enabled
            && self_loop_copies_can_hoist(&func.blocks[block_idx], &copies, &uses, block_idx);
        if debug {
            let dests: Vec<String> = copies
                .iter()
                .map(|c| match c {
                    Instruction::Copy { dest, src } => format!("v{}={:?}", dest.0, src),
                    _ => "?".to_string(),
                })
                .collect();
            eprintln!(
                "[PHI] self-loop block {}: {} ({} copies: {})",
                target_block_id.0,
                if hoist { "absorb" } else { "split" },
                copies.len(),
                dests.join(", ")
            );
        }
        if hoist {
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
}

/// Apply all phi elimination transformations to the function:
/// remove phis, insert copies, retarget terminators, add trampolines.
fn apply_phi_transformations(func: &mut IrFunction, ctx: &mut PhiElimCtx) {
    for (block_idx, block) in func.blocks.iter_mut().enumerate() {
        // Remove phi instructions (and their spans)
        if !block.source_spans.is_empty() {
            let mut span_idx = 0;
            block.source_spans.retain(|_| {
                let keep = !matches!(
                    block.instructions.get(span_idx),
                    Some(Instruction::Phi { .. })
                );
                span_idx += 1;
                keep
            });
        }
        block
            .instructions
            .retain(|inst| !matches!(inst, Instruction::Phi { .. }));

        // Prepend target copies (these go at the start, replacing the phis)
        if !ctx.target_copies[block_idx].is_empty() {
            let num_copies = ctx.target_copies[block_idx].len();
            let mut new_insts = ctx.target_copies[block_idx].clone();
            new_insts.append(&mut block.instructions);
            block.instructions = new_insts;
            if !block.source_spans.is_empty() {
                let mut new_spans = vec![crate::common::source::Span::dummy(); num_copies];
                new_spans.append(&mut block.source_spans);
                block.source_spans = new_spans;
            }
        }

        // Insert predecessor copies before terminator
        if let Some(copies) = ctx.pred_copies.remove(&block_idx) {
            let num_copies = copies.len();
            block.instructions.extend(copies);
            if !block.source_spans.is_empty() {
                block.source_spans.extend(std::iter::repeat_n(
                    crate::common::source::Span::dummy(),
                    num_copies,
                ));
            }
        }
    }

    // Retarget predecessors that need trampolines
    for trampoline in &ctx.trampolines {
        retarget_block_edge_once(
            &mut func.blocks[trampoline.pred_idx],
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
            terminator: Terminator::Branch(trampoline.branch_target),
        });
    }
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
                    'dom: for (bj, blkj) in func.blocks.iter().enumerate() {
                        if bj == bi {
                            continue;
                        }
                        // Strict dominance via the idom chain of `bj`
                        // (bounded walk; the usize::MAX sentinel marks
                        // unreachable blocks and terminates the loop).
                        let mut cur = bj;
                        let mut steps = 0usize;
                        while cur < idom.len() && steps <= num_blocks {
                            if cur == bi {
                                for inst in &blkj.instructions {
                                    if inst.dest().is_some_and(|Value(d)| d == dest.0) {
                                        pending_relay = true;
                                        break 'dom;
                                    }
                                    let mut touches = false;
                                    inst.for_each_used_value(|id| {
                                        if id == dest.0 {
                                            touches = true;
                                        }
                                    });
                                    if touches {
                                        pending_relay = true;
                                        break 'dom;
                                    }
                                }
                                break;
                            }
                            let next = idom[cur];
                            if next == cur || next >= idom.len() {
                                // entry self-parent or sentinel: stop
                                break;
                            }
                            cur = next;
                            steps += 1;
                        }
                    }
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
            let def_inst = block.instructions.remove(def_pos);
            if !block.source_spans.is_empty() {
                let span = block
                    .source_spans
                    .remove(def_pos.min(block.source_spans.len() - 1));
                let at = (copy_pos - 1).min(block.source_spans.len());
                block.source_spans.insert(at, span);
            }
            // Definition removal shifted the copy down one slot; the
            // definition lands directly before it.
            let copy_pos = copy_pos - 1;
            block.instructions.insert(copy_pos, def_inst);
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
        self_loop_copies_can_hoist(&blocks[block_idx], copies, &uses, block_idx)
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
