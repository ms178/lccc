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
//!    `Branch(Bmerge)` (unconditional — no critical-edge splitting).
//!    NonlocalGoto is outside CFG discipline in this IR — like every other
//!    CFG pass (cfg_simplify, phi elimination, liveness), this pass does not
//!    model it as an edge.
//!
//!    A predecessor that ends in an asm goto has edges the terminator does
//!    not describe: its labels.  The arm it holds in the merge phis belongs
//!    to the terminator's edge, and that edge threads like any other -- it
//!    is how the kernel's static keys reach here, `asm goto(... : l_yes);
//!    return false; l_yes: return true;` inlined into `if (key)`, where the
//!    asm goto's own block brings the constant 0 -- under two conditions:
//!    * no label is Bmerge, Bt or Bf.  The block would then be one
//!      predecessor of that block along two edges with one arm, and the
//!      rewrite removes or adds arms per predecessor;
//!    * the rewrite adds no instruction to it (rule 1's "possibly
//!      conditional branch" does not; the int shape's re-materialized
//!      compare of a non-constant arm does).  Nothing may follow an asm
//!      goto in its block: its labels' edges would skip it.
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
    BasicBlock, BlockId, Instruction, IrCmpOp, IrConst, IrFunction, Operand, Terminator, Value,
};
use crate::passes::round_cap;

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
/// predecessor edges plus removed truth-preserving merge casts and folded
/// tests of constant phis.
pub(crate) fn run(func: &mut IrFunction) -> usize {
    let mut total = fold_tests_of_constant_phis(func) + remove_boolean_phi_branch_casts(func);
    let mut converged = false;
    for _ in 0..MAX_ROUNDS {
        let n = thread_round(func);
        total += n;
        if n == 0 {
            converged = true;
            break;
        }
    }
    if !converged {
        round_cap::note_cap_hit(round_cap::Site::BoolThread);
    }
    total
}

/// One link between a phi of constants and the branch that tests it.
#[derive(Clone, Copy)]
enum TestLink {
    Cast(IrType, IrType),
    /// `phi op constant`; a constant on the left is stored swapped.
    Cmp(IrCmpOp, IrConst, IrType),
    Copy,
}

/// The test of a phi of constants, folded: what replaces the phi and which
/// instructions go.
struct FoldedTest {
    phi_at: usize,
    /// Positions of the cast/compare/copy instructions between the phi and
    /// the branch.
    links_at: Vec<usize>,
    /// The phi's value and the links' results except the last: each is read
    /// by the next link and must be read by nothing else.
    intermediates: Vec<Value>,
    dest: Value,
    ty: IrType,
    incoming: Vec<(Operand, BlockId)>,
}

/// `p = phi [0, A], [1, B]; c = (u8)p; t = c == 0; branch t` as
/// `t = phi [1, A], [0, B]; branch t`, when every arm is an integer constant
/// and the links are integer casts, copies and compares against constants.
///
/// What the branch tests is then a phi again, which is the shape this pass
/// threads; the links stood between the two.  They differ by target and by
/// source spelling -- a `_Bool` promoted to `int`, `!x`, `x != 0` -- and are
/// evaluated here with the constant folder's own evaluators, so each arm gets
/// exactly the value the folder would compute behind it.
fn folded_test(block: &BasicBlock, cond: Value) -> Option<FoldedTest> {
    let mut links: Vec<(usize, TestLink)> = Vec::new();
    let mut value = cond;
    // A cast, a compare and a cast again is as long as a test gets; the bound
    // keeps a malformed cycle from spinning.
    let (phi_at, phi_ty, arms) = loop {
        if links.len() > 6 {
            return None;
        }
        let (at, def) = block
            .instructions
            .iter()
            .enumerate()
            .find(|(_, inst)| inst.dest() == Some(value))?;
        value = match def {
            Instruction::Phi { ty, incoming, .. } => break (at, *ty, incoming),
            Instruction::Cast {
                src: Operand::Value(src),
                from_ty,
                to_ty,
                ..
            } if from_ty.is_integer() && to_ty.is_integer() => {
                links.push((at, TestLink::Cast(*from_ty, *to_ty)));
                *src
            }
            Instruction::Copy {
                src: Operand::Value(src),
                ..
            } => {
                links.push((at, TestLink::Copy));
                *src
            }
            Instruction::Cmp {
                op,
                lhs: Operand::Value(src),
                rhs: Operand::Const(rhs),
                ty,
                ..
            } if ty.is_integer() => {
                links.push((at, TestLink::Cmp(*op, *rhs, *ty)));
                *src
            }
            Instruction::Cmp {
                op,
                lhs: Operand::Const(lhs),
                rhs: Operand::Value(src),
                ty,
                ..
            } if ty.is_integer() => {
                links.push((at, TestLink::Cmp(op.swapped(), *lhs, *ty)));
                *src
            }
            _ => return None,
        };
    };
    if links.is_empty() || arms.len() < 2 || !phi_ty.is_integer() {
        return None;
    }
    // From the phi outwards.
    links.reverse();
    let mut ty = phi_ty;
    for (_, link) in &links {
        ty = match link {
            TestLink::Cast(_, to_ty) => *to_ty,
            TestLink::Cmp(..) => IrType::I32,
            TestLink::Copy => ty,
        };
    }
    let incoming = arms
        .iter()
        .map(|(arm, from)| {
            let Operand::Const(mut constant) = *arm else {
                return None;
            };
            for (_, link) in &links {
                constant = match *link {
                    TestLink::Cast(from_ty, to_ty) => {
                        crate::passes::constant_fold::eval_cast_const(constant, from_ty, to_ty)?
                    }
                    TestLink::Cmp(op, rhs, cmp_ty) => {
                        crate::passes::constant_fold::eval_cmp_const(op, constant, rhs, cmp_ty)?
                    }
                    TestLink::Copy => constant,
                };
            }
            Some((Operand::Const(constant), *from))
        })
        .collect::<Option<Vec<_>>>()?;
    let phi_value = block.instructions[phi_at].dest()?;
    let intermediates = std::iter::once(phi_value)
        .chain(
            links[..links.len() - 1]
                .iter()
                .filter_map(|(at, _)| block.instructions[*at].dest()),
        )
        .collect();
    Some(FoldedTest {
        phi_at,
        links_at: links.iter().map(|(at, _)| *at).collect(),
        intermediates,
        dest: cond,
        ty,
        incoming,
    })
}

/// Fold every such test ([`folded_test`]) whose links nothing else reads.
/// Returns the number of branches that test a phi now.
fn fold_tests_of_constant_phis(func: &mut IrFunction) -> usize {
    let candidates: Vec<(usize, FoldedTest)> = func
        .blocks
        .iter()
        .enumerate()
        .filter_map(|(bi, block)| match &block.terminator {
            Terminator::CondBranch {
                cond: Operand::Value(cond),
                ..
            } => folded_test(block, *cond).map(|fold| (bi, fold)),
            _ => None,
        })
        .collect();
    if candidates.is_empty() {
        return 0;
    }
    let mut uses: FxHashMap<u32, u8> = FxHashMap::default();
    for block in &func.blocks {
        let mut count = |id| {
            let n = uses.entry(id).or_default();
            *n = (*n + 1).min(2);
        };
        for inst in &block.instructions {
            inst.for_each_used_value(&mut count);
        }
        block.terminator.for_each_used_value(&mut count);
    }
    let mut folded = 0;
    for (bi, mut fold) in candidates {
        if fold
            .intermediates
            .iter()
            .any(|value| uses.get(&value.0) != Some(&1))
        {
            continue;
        }
        let block = &mut func.blocks[bi];
        block.instructions[fold.phi_at] = Instruction::Phi {
            dest: fold.dest,
            ty: fold.ty,
            incoming: fold.incoming,
        };
        // Highest position first: the others, and the phi ahead of them
        // all, keep theirs.
        fold.links_at.sort_unstable_by(|a, b| b.cmp(a));
        for at in fold.links_at {
            block.remove_inst(at);
        }
        folded += 1;
    }
    folded
}

/// A local integer cast of a {0,1} phi preserves the result of testing
/// that value for zero, even when it narrows. Both threaders use the same
/// proof; an arbitrary narrow cast cannot be peeled (e.g. 256 -> u8 is 0).
pub(super) fn boolean_phi_cast_source(block: &BasicBlock, cast: Value) -> Option<(usize, Value)> {
    let (position, source) =
        block
            .instructions
            .iter()
            .enumerate()
            .find_map(|(i, inst)| match inst {
                Instruction::Cast {
                    dest,
                    src: Operand::Value(src),
                    from_ty,
                    to_ty,
                } if *dest == cast && from_ty.is_integer() && to_ty.is_integer() => Some((i, *src)),
                _ => None,
            })?;
    let boolean_phi = block.instructions.iter().any(|inst| {
        matches!(inst, Instruction::Phi { dest, incoming, ty }
        if ty.is_integer() && *dest == source && !incoming.is_empty() && incoming.iter().all(|(op, _)| {
            match op {
                Operand::Const(c) => matches!(const_to_i64(c), Some(0 | 1))
                    || matches!(c, IrConst::I128(0 | 1)),
                Operand::Value(_) => false,
            }
        }))
    });
    boolean_phi.then_some((position, source))
}

/// Remove branch-only casts to expose the no-duplication threader. Count
/// uses once for the function, rather than rescanning it per candidate.
fn remove_boolean_phi_branch_casts(func: &mut IrFunction) -> usize {
    let mut candidates = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        let Terminator::CondBranch {
            cond: Operand::Value(cond),
            ..
        } = &block.terminator
        else {
            continue;
        };
        if let Some((position, source)) = boolean_phi_cast_source(block, *cond) {
            candidates.push((bi, position, source, *cond));
        }
    }
    if candidates.is_empty() {
        return 0;
    }
    let mut uses: FxHashMap<u32, u8> = FxHashMap::default();
    for block in &func.blocks {
        let mut count = |id| {
            let n = uses.entry(id).or_default();
            *n = (*n + 1).min(2);
        };
        for inst in &block.instructions {
            inst.for_each_used_value(&mut count);
        }
        block.terminator.for_each_used_value(&mut count);
    }
    let mut removed = 0;
    for (bi, position, source, cond) in candidates {
        if uses.get(&cond.0) != Some(&1) {
            continue;
        }
        let block = &mut func.blocks[bi];
        if let Terminator::CondBranch { cond, .. } = &mut block.terminator {
            *cond = Operand::Value(source);
        }
        block.remove_inst(position);
        removed += 1;
    }
    removed
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

/// Visible predecessors per block (deduped; a CondBranch with both arms
/// into the same target yields one entry — it is never threadable, and
/// the exact-arm-count check rejects whatever phi shape it implies).
fn visible_predecessors(func: &IrFunction, idx_of: &FxHashMap<BlockId, usize>) -> Vec<Vec<usize>> {
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
    preds
}

/// Rule 4 for a predecessor that ends in an asm goto: none of its labels is
/// one of `in_the_way` (the merge and its two targets), and threading puts no
/// instruction behind the asm goto.  A predecessor without an asm goto always
/// passes.
fn asm_goto_allows_threading(
    pred: &BasicBlock,
    in_the_way: [BlockId; 3],
    adds_no_instruction: bool,
) -> bool {
    let mut labels = crate::ir::analysis::asm_goto_successors(pred).peekable();
    labels.peek().is_none() || (adds_no_instruction && !labels.any(|l| in_the_way.contains(&l)))
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

    let preds = visible_predecessors(func, &idx_of);

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
        let phi_dests: Vec<Value>;
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
            if !clean_term {
                continue;
            }
            let adds_no_instruction = match &int_cmp {
                None => true,
                Some((_, rhs, _, ipos)) => matches!(
                    (&arms[*ipos][pos], rhs),
                    (Some(Operand::Const(_)), Operand::Const(_))
                ),
            };
            if !asm_goto_allows_threading(pred, [ml, bt, bf], adds_no_instruction) {
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
                                    (op.swapped(), rhs.clone(), arm.clone())
                                }
                                _ => (*op, arm.clone(), rhs.clone()),
                            };
                            func.blocks[pi].push_inst(
                                Instruction::Cmp {
                                    dest: fresh,
                                    op: op_e,
                                    lhs: lhs_e,
                                    rhs: rhs_e,
                                    ty: *ty,
                                },
                                crate::common::source::Span::dummy(),
                            );
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
                let Some(k) = incoming.iter().position(|(_, l)| *l == ml) else {
                    // Fail-closed: if the merge edge is missing, the CFG
                    // changed under us (or validation missed it). Keep the
                    // phi as-is rather than panicking — the thread for this
                    // merge will be incomplete but the IR stays well-formed.
                    continue;
                };
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
            source_inline_ids: Vec::new(),
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
                    param_align: None,
                    struct_eightbyte_classes: vec![],
                    is_f128_sse: false,
                    fp_agg_class: None,
                },
                IrParam {
                    ty: IrType::I32,
                    noalias: false,
                    struct_size: None,
                    struct_align: None,
                    param_align: None,
                    struct_eightbyte_classes: vec![],
                    is_f128_sse: false,
                    fp_agg_class: None,
                },
                IrParam {
                    ty: IrType::I32,
                    noalias: false,
                    struct_size: None,
                    struct_align: None,
                    param_align: None,
                    struct_eightbyte_classes: vec![],
                    is_f128_sse: false,
                    fp_agg_class: None,
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
            source_inline_ids: Vec::new(),
        };
        let arm = |label: u32, src: Value, target: BlockId| BasicBlock {
            label: BlockId(label),
            instructions: vec![Instruction::Copy {
                dest: vp,
                src: Operand::Value(src),
            }],
            terminator: Terminator::Branch(target),
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
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
            source_inline_ids: Vec::new(),
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
            source_inline_ids: Vec::new(),
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

    fn cast_boolean_merge(from_ty: IrType, to_ty: IrType) -> IrFunction {
        let mut f = IrFunction::new("cast_phi".into(), IrType::I32, vec![], false);
        f.next_value_id = 10;
        let entry = empty_block(
            0,
            Terminator::CondBranch {
                cond: Operand::Const(IrConst::I32(1)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
        );
        let mut merge = empty_block(
            3,
            Terminator::CondBranch {
                cond: Operand::Value(Value(9)),
                true_label: BlockId(4),
                false_label: BlockId(5),
            },
        );
        merge.instructions = vec![
            Instruction::Phi {
                dest: Value(6),
                ty: from_ty,
                incoming: vec![
                    (Operand::Const(IrConst::I64(0)), BlockId(1)),
                    (Operand::Const(IrConst::I64(1)), BlockId(2)),
                ],
            },
            Instruction::Cast {
                dest: Value(9),
                src: Operand::Value(Value(6)),
                from_ty,
                to_ty,
            },
        ];
        merge.source_spans = vec![
            crate::common::source::Span::new(1, 2, 0),
            crate::common::source::Span::new(2, 3, 0),
        ];
        f.blocks = vec![
            entry,
            empty_block(1, Terminator::Branch(BlockId(3))),
            empty_block(2, Terminator::Branch(BlockId(3))),
            merge,
            empty_block(4, Terminator::Return(Some(Operand::Const(IrConst::I32(1))))),
            empty_block(5, Terminator::Return(Some(Operand::Const(IrConst::I32(0))))),
        ];
        f
    }

    #[test]
    fn exact_boolean_phi_casts_preserve_truth_for_every_integer_width() {
        let types = [
            IrType::I8,
            IrType::U8,
            IrType::I16,
            IrType::U16,
            IrType::I32,
            IrType::U32,
            IrType::I64,
            IrType::U64,
            IrType::I128,
            IrType::U128,
        ];
        for from in types {
            for to in types {
                let mut f = cast_boolean_merge(from, to);
                assert_eq!(
                    remove_boolean_phi_branch_casts(&mut f),
                    1,
                    "{from:?} -> {to:?}"
                );
                assert_eq!(f.blocks[3].instructions.len(), 1);
                assert_eq!(
                    f.blocks[3].source_spans,
                    [crate::common::source::Span::new(1, 2, 0)]
                );
                assert!(matches!(
                    f.blocks[3].terminator,
                    Terminator::CondBranch {
                        cond: Operand::Value(Value(6)),
                        ..
                    }
                ));
                assert!(all_uses_resolved(&f));
            }
        }
    }

    #[test]
    fn promoted_cast_metadata_still_requires_an_integer_phi() {
        // Real promoted return-value shape: an u8 phi feeds an i32->u8 cast.
        let mut f = cast_boolean_merge(IrType::I32, IrType::U8);
        if let Instruction::Phi { ty, .. } = &mut f.blocks[3].instructions[0] {
            *ty = IrType::U8;
        }
        assert_eq!(remove_boolean_phi_branch_casts(&mut f), 1);
        // Fail closed on contradictory floating-point phi metadata.
        let mut f = cast_boolean_merge(IrType::I32, IrType::U8);
        if let Instruction::Phi { ty, .. } = &mut f.blocks[3].instructions[0] {
            *ty = IrType::F64;
        }
        assert_eq!(remove_boolean_phi_branch_casts(&mut f), 0);
    }

    #[test]
    fn boolean_cast_removal_exposes_both_constant_thread_edges() {
        let mut f = cast_boolean_merge(IrType::I32, IrType::U8);
        assert_eq!(run(&mut f), 3); // cast + both incoming branches
        assert!(is_branch_to(&f.blocks[1].terminator, 5));
        assert!(is_branch_to(&f.blocks[2].terminator, 4));
        assert!(all_uses_resolved(&f));
    }

    #[test]
    fn narrowing_non_booleans_and_unknown_phi_arms_are_not_rewritten() {
        for value in [
            Operand::Const(IrConst::I64(256)),
            Operand::Const(IrConst::I64(-256)),
            Operand::Const(IrConst::I64(2)),
            Operand::Const(IrConst::I128(1i128 << 64)),
            Operand::Value(Value(0)),
        ] {
            let from = if matches!(value, Operand::Const(IrConst::I128(_))) {
                IrType::I128
            } else {
                IrType::I64
            };
            let mut f = cast_boolean_merge(from, IrType::U8);
            f.blocks[0].instructions.push(Instruction::Copy {
                dest: Value(0),
                src: Operand::Const(IrConst::I64(256)),
            });
            if let Instruction::Phi { incoming, .. } = &mut f.blocks[3].instructions[0] {
                incoming[1].0 = value;
            }
            assert_eq!(remove_boolean_phi_branch_casts(&mut f), 0);
            assert_eq!(f.blocks[3].instructions.len(), 2);
        }
    }

    #[test]
    fn live_cast_results_and_noninteger_casts_are_preserved() {
        let mut f = cast_boolean_merge(IrType::I32, IrType::U8);
        f.blocks[5].terminator = Terminator::Return(Some(Operand::Value(Value(9))));
        assert_eq!(remove_boolean_phi_branch_casts(&mut f), 0);
        for (from, to) in [
            (IrType::I32, IrType::F32),
            (IrType::F64, IrType::U8),
            (IrType::Ptr, IrType::U8),
        ] {
            let mut f = cast_boolean_merge(from, to);
            assert_eq!(remove_boolean_phi_branch_casts(&mut f), 0);
        }
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

    // ---- A predecessor that ends in an asm goto (rule 4). ----

    fn block(label: u32, instructions: Vec<Instruction>, term: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator: term,
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        }
    }
    fn asm_goto_to(label: u32) -> Instruction {
        Instruction::InlineAsm {
            template: "jmp %l0".into(),
            outputs: vec![],
            inputs: vec![],
            clobbers: vec![],
            operand_types: vec![],
            goto_labels: vec![("l_yes".into(), BlockId(label))],
            input_symbols: vec![],
            seg_overrides: vec![],
        }
    }
    fn jump(label: u32) -> Terminator {
        Terminator::Branch(BlockId(label))
    }
    fn branch_on(value: u32, yes: u32, no: u32) -> Terminator {
        Terminator::CondBranch {
            cond: Operand::Value(Value(value)),
            true_label: BlockId(yes),
            false_label: BlockId(no),
        }
    }
    fn phi(dest: u32, arms: Vec<(Operand, u32)>) -> Instruction {
        Instruction::Phi {
            dest: Value(dest),
            ty: IrType::I32,
            incoming: arms.into_iter().map(|(op, b)| (op, BlockId(b))).collect(),
        }
    }
    fn int(value: i32) -> Operand {
        Operand::Const(IrConst::I32(value))
    }
    fn ret() -> Terminator {
        Terminator::Return(Some(int(0)))
    }
    fn function(blocks: Vec<BasicBlock>) -> IrFunction {
        let mut f = IrFunction::new("t".to_string(), IrType::I32, vec![], false);
        f.next_value_id = 100;
        f.blocks = blocks;
        f
    }
    fn ends_in_its_asm_goto(block: &BasicBlock) -> bool {
        matches!(
            block.instructions.last(),
            Some(Instruction::InlineAsm { .. })
        ) && block
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::InlineAsm { .. }))
            .count()
            == 1
    }

    /// `if (static_branch_unlikely(&key)) slow();` once the helper is
    /// inlined: the asm goto's own block brings 0 (it fell through the
    /// `nop`), the block of its label brings 1.
    ///
    /// b0: asm goto -> b1; Branch(b2)
    /// b1: Branch(b2)
    /// b2: p = phi [0, b0], [1, b1]; CondBranch(p, b3, b4)
    /// b3 (slow): Branch(b4)        b4: Return
    fn static_key(label: u32) -> IrFunction {
        function(vec![
            block(0, vec![asm_goto_to(label)], jump(2)),
            block(1, vec![], jump(2)),
            block(
                2,
                vec![phi(10, vec![(int(0), 0), (int(1), 1)])],
                branch_on(10, 3, 4),
            ),
            block(3, vec![], jump(4)),
            block(4, vec![], ret()),
        ])
    }

    #[test]
    fn an_asm_goto_predecessor_threads_along_its_terminator() {
        let mut f = static_key(1);
        assert_eq!(super::run(&mut f), 2);
        // The asm falls through to the code behind the `if`; its label is
        // the call.  No value is left to test.
        assert!(is_branch_to(&f.blocks[0].terminator, 4));
        assert!(is_branch_to(&f.blocks[1].terminator, 3));
        assert!(ends_in_its_asm_goto(&f.blocks[0]));
        assert!(all_uses_resolved(&f));
        let Instruction::Phi { incoming, .. } = &f.blocks[2].instructions[0] else {
            panic!("the merge keeps its (dead) phi");
        };
        assert!(incoming.is_empty(), "no way into the merge is left");
    }

    /// The int shape with constant arms folds its compare per predecessor
    /// and adds nothing behind the asm goto: `if (!static_branch(...))`.
    #[test]
    fn an_asm_goto_predecessor_threads_through_a_folded_compare() {
        let mut f = static_key(1);
        f.blocks[2].instructions.push(Instruction::Cmp {
            dest: Value(11),
            op: IrCmpOp::Eq,
            lhs: Operand::Value(Value(10)),
            rhs: int(0),
            ty: IrType::I32,
        });
        f.blocks[2].terminator = branch_on(11, 3, 4);
        // One folded test (the compare becomes the phi's arms), then the
        // two ways in.
        assert_eq!(super::run(&mut f), 3);
        assert!(is_branch_to(&f.blocks[0].terminator, 3), "0 == 0");
        assert!(is_branch_to(&f.blocks[1].terminator, 4), "1 != 0");
        assert!(ends_in_its_asm_goto(&f.blocks[0]));
        assert!(all_uses_resolved(&f));
    }

    /// A label that is the merge or one of its two targets: the asm goto's
    /// block reaches that block along two edges and has one arm there.
    #[test]
    fn an_asm_goto_whose_label_is_in_the_way_keeps_its_predecessor() {
        for label in [2u32, 3, 4] {
            let mut f = static_key(label);
            // Block 1 is no longer the label; keep it reachable.
            f.blocks.insert(0, block(9, vec![], branch_on(50, 0, 1)));
            f.blocks[0].instructions.push(Instruction::ParamRef {
                dest: Value(50),
                param_idx: 0,
                ty: IrType::I32,
            });
            super::run(&mut f);
            assert!(
                is_branch_to(&f.blocks[1].terminator, 2),
                "label {label}: the asm goto's block still jumps to the merge"
            );
            assert!(ends_in_its_asm_goto(&f.blocks[1]), "label {label}");
            assert!(all_uses_resolved(&f), "label {label}");
            // The other predecessor threads on its own (bool shape).
            assert!(is_branch_to(&f.blocks[2].terminator, 3), "label {label}");
        }
    }

    /// A value arm: the bool shape branches on it and adds no instruction;
    /// the int shape would put its compare behind the asm goto and is
    /// refused, for every predecessor (the merge has to die as a whole).
    #[test]
    fn an_asm_goto_predecessor_never_gets_an_instruction_behind_the_asm_goto() {
        let with_value_arm = |int_shape: bool| {
            let mut f = static_key(1);
            f.blocks.insert(
                0,
                block(
                    9,
                    vec![Instruction::ParamRef {
                        dest: Value(50),
                        param_idx: 0,
                        ty: IrType::I32,
                    }],
                    jump(0),
                ),
            );
            f.blocks[3].instructions[0] =
                phi(10, vec![(Operand::Value(Value(50)), 0), (int(1), 1)]);
            if int_shape {
                f.blocks[3].instructions.push(Instruction::Cmp {
                    dest: Value(11),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(10)),
                    rhs: int(5),
                    ty: IrType::I32,
                });
                f.blocks[3].terminator = branch_on(11, 3, 4);
            }
            f
        };
        let mut f = with_value_arm(false);
        assert_eq!(super::run(&mut f), 2);
        assert!(matches!(
            &f.blocks[1].terminator,
            Terminator::CondBranch { cond: Operand::Value(Value(50)), true_label, false_label }
                if *true_label == BlockId(3) && *false_label == BlockId(4)
        ));
        assert!(ends_in_its_asm_goto(&f.blocks[1]));

        let mut f = with_value_arm(true);
        let before = format!("{f:?}");
        assert_eq!(super::run(&mut f), 0);
        assert_eq!(format!("{f:?}"), before);
    }

    // ---- The test of a phi of constants, folded into the phi. ----

    fn cast(dest: u32, src: u32, from_ty: IrType, to_ty: IrType) -> Instruction {
        Instruction::Cast {
            dest: Value(dest),
            src: Operand::Value(Value(src)),
            from_ty,
            to_ty,
        }
    }
    fn cmp(dest: u32, op: IrCmpOp, lhs: u32, rhs: Operand, ty: IrType) -> Instruction {
        Instruction::Cmp {
            dest: Value(dest),
            op,
            lhs: Operand::Value(Value(lhs)),
            rhs,
            ty,
        }
    }

    /// `if (static_branch(&key))` on i686 at -O1: the `_Bool` the helper
    /// returns is narrowed, promoted and compared with zero before the
    /// branch.  Each way in brings a constant; the branch folds per way in.
    #[test]
    fn a_test_through_casts_and_a_compare_folds_into_the_phi_and_threads() {
        for (op, to_b0, to_b1) in [(IrCmpOp::Ne, 4, 3), (IrCmpOp::Eq, 3, 4)] {
            let mut f = static_key(1);
            f.blocks[2].instructions.extend([
                cast(11, 10, IrType::I32, IrType::U8),
                cast(12, 11, IrType::U8, IrType::I32),
                cmp(13, op, 12, int(0), IrType::I32),
            ]);
            f.blocks[2].terminator = branch_on(13, 3, 4);
            // One fold, two threaded edges.
            assert_eq!(super::run(&mut f), 3, "{op:?}");
            assert!(is_branch_to(&f.blocks[0].terminator, to_b0), "{op:?}");
            assert!(is_branch_to(&f.blocks[1].terminator, to_b1), "{op:?}");
            assert!(
                matches!(
                    f.blocks[2].instructions.as_slice(),
                    [Instruction::Phi {
                        dest: Value(13),
                        ..
                    }]
                ),
                "{op:?}: the phi is what the branch tests, got {:?}",
                f.blocks[2].instructions
            );
            assert!(all_uses_resolved(&f), "{op:?}");
        }
    }

    /// The links are evaluated, not assumed to keep the truth: 256 narrowed
    /// to a byte is 0, and a constant on the left of the compare is read
    /// the right way round.
    #[test]
    fn folding_a_test_computes_each_arm() {
        let folded = |arms: Vec<(Operand, u32)>, links: Vec<Instruction>, tested: u32| {
            let mut instructions = vec![phi(10, arms)];
            instructions.extend(links);
            let merge = block(2, instructions, branch_on(tested, 3, 4));
            super::folded_test(&merge, Value(tested)).map(|fold| {
                fold.incoming
                    .iter()
                    .map(|(op, _)| match op {
                        Operand::Const(c) => const_to_i64(c).expect("an integer"),
                        Operand::Value(_) => panic!("a value arm"),
                    })
                    .collect::<Vec<i64>>()
            })
        };
        assert_eq!(
            folded(
                vec![(int(256), 0), (int(1), 1)],
                vec![cast(11, 10, IrType::I32, IrType::U8)],
                11
            ),
            Some(vec![0, 1])
        );
        // `5 < p`, the constant on the left.
        let five_less_than = Instruction::Cmp {
            dest: Value(11),
            op: IrCmpOp::Slt,
            lhs: int(5),
            rhs: Operand::Value(Value(10)),
            ty: IrType::I32,
        };
        assert_eq!(
            folded(vec![(int(3), 0), (int(9), 1)], vec![five_less_than], 11),
            Some(vec![0, 1])
        );
        // Unsigned: -1 is the largest value.
        assert_eq!(
            folded(
                vec![(int(-1), 0), (int(1), 1)],
                vec![cmp(11, IrCmpOp::Ugt, 10, int(2), IrType::U32)],
                11
            ),
            Some(vec![1, 0])
        );
        // Nothing between the phi and the branch: nothing to fold.
        assert_eq!(folded(vec![(int(0), 0), (int(1), 1)], vec![], 10), None);
        // An arm that is a value, and a compare with one.
        assert_eq!(
            folded(
                vec![(int(0), 0), (Operand::Value(Value(5)), 1)],
                vec![cast(11, 10, IrType::I32, IrType::U8)],
                11
            ),
            None
        );
        assert_eq!(
            folded(
                vec![(int(0), 0), (int(1), 1)],
                vec![cmp(
                    11,
                    IrCmpOp::Eq,
                    10,
                    Operand::Value(Value(5)),
                    IrType::I32
                )],
                11
            ),
            None
        );
    }

    /// A link something else reads stays, and so does the test.
    #[test]
    fn a_test_whose_links_are_read_elsewhere_is_not_folded() {
        let mut f = static_key(1);
        f.blocks[2].instructions.extend([
            cast(11, 10, IrType::I32, IrType::U8),
            cmp(13, IrCmpOp::Ne, 11, int(0), IrType::U8),
        ]);
        f.blocks[2].terminator = branch_on(13, 3, 4);
        // The slow block reads the narrowed value.
        f.blocks[3].instructions.push(Instruction::Copy {
            dest: Value(40),
            src: Operand::Value(Value(11)),
        });
        let before = format!("{f:?}");
        assert_eq!(super::fold_tests_of_constant_phis(&mut f), 0);
        assert_eq!(format!("{f:?}"), before);
    }
}
