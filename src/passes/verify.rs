//! IR structural verifier.
//!
//! lccc has a register-allocator verifier (`CCC_VERIFY_REGALLOC`) but no
//! equivalent for the mid-level IR, so a pass that emits *structurally
//! malformed* IR can only be caught by the wrong answer it eventually
//! produces — if it produces one at all. That gap is not hypothetical: it hid
//! a real `loop_rotate` defect for months.
//!
//! # The defect this module was built for
//!
//! When `loop_rotate` rewrites a guard-at-top loop into test-at-bottom form it
//! creates a guard block and rewires every original header predecessor onto
//! it. The rotated body's induction-variable phi was still labelling its init
//! incoming with the *original preheader*, which is no longer a predecessor of
//! that block:
//!
//! ```text
//! block .LBB6:  term: Branch(.LBB7)
//! block .LBB7:  Phi v22 = [(0, .LBB6)]                 // guard
//! block .LBB8:  Phi v72 = [(0, .LBB6), (v16, .LBB8)]   // <-- .LBB6 is not a pred
//! ```
//!
//! Nothing complained, because phi elimination resolves a phi operand's label
//! to a block *index* and emits the init copy there. `.LBB6` still dominated
//! `.LBB8`, so the copy landed somewhere that happened to execute first and
//! the program worked by accident. Any consumer that instead trusts the
//! predecessor list — SCCP pruning operands on provably-dead edges — deletes
//! the initialisation outright, and the loop indexes an array with an
//! uninitialised register.
//!
//! The lesson is that "the tests pass" is not evidence of well-formed IR when
//! the only consumers are forgiving. This verifier makes the invariant
//! explicit and checks it after every pass.
//!
//! # Usage
//!
//! Off by default (it is O(blocks + edges) per pass, but allocates). Enable
//! with `CCC_VERIFY_IR=1` to print every violation to stderr, or
//! `CCC_VERIFY_IR=abort` to panic on the first one, which gives a backtrace
//! pointing at the pass that produced the bad IR:
//!
//! ```text
//! CCC_VERIFY_IR=abort CCC_LOOP_ROTATE=1 lccc -O2 foo.c -o foo
//! ```
//!
//! Because the pass loop verifies after each pass, the reported `stage` names
//! the pass that broke the invariant, not the one that tripped over it later.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::instruction::{BlockId, Operand, Terminator};
use crate::ir::reexports::{Instruction, IrFunction, IrModule, Value};

/// A single structural violation, with enough context to identify the culprit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// Pass name (or other stage label) that produced the IR.
    pub stage: String,
    /// Function the violation was found in.
    pub function: String,
    /// Human-readable description.
    pub detail: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[ir-verify] after `{}` in `{}`: {}",
            self.stage, self.function, self.detail
        )
    }
}

/// What `CCC_VERIFY_IR` asks us to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Off,
    Report,
    Abort,
}

/// The env var is read exactly once per process. `verify_after_pass` is called
/// after every pass of every module, so a fresh `std::env::var` (which
/// allocates a `String`) on the disabled path would be pure waste in the
/// overwhelmingly common case where verification is off.
fn mode() -> Mode {
    static MODE: std::sync::LazyLock<Mode> =
        std::sync::LazyLock::new(|| match std::env::var("CCC_VERIFY_IR") {
            Err(_) => Mode::Off,
            Ok(v) => match v.trim() {
                "" | "0" | "off" | "no" => Mode::Off,
                "abort" | "panic" | "2" => Mode::Abort,
                _ => Mode::Report,
            },
        });
    *MODE
}

/// Visit every CFG successor label of a terminator.
///
/// Kept private and exhaustive (no wildcard arm) so that adding a terminator
/// variant is a compile error here rather than a silently unchecked edge.
fn for_each_target(term: &Terminator, mut f: impl FnMut(BlockId)) {
    match term {
        Terminator::Branch(t) => f(*t),
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            f(*true_label);
            f(*false_label);
        }
        Terminator::Switch { cases, default, .. } => {
            for (_, label) in cases {
                f(*label);
            }
            f(*default);
        }
        Terminator::IndirectBranch {
            possible_targets, ..
        } => {
            for t in possible_targets {
                f(*t);
            }
        }
        Terminator::Return(_) | Terminator::Unreachable => {}
    }
}

/// Verify one function, appending any violations to `out`.
///
/// Checks, in order:
///
/// 1. **Unique block labels.** Everything else keys off label→index.
/// 2. **Targets exist.** Every terminator and `asm goto` label names a block.
/// 3. **Phi contiguity.** Phis form an unbroken prefix of their block
///    (`loop_rotate` and `mem2reg` both rely on this).
/// 4. **Phi predecessors are real.** Every label in a phi's incoming list is
///    an actual CFG predecessor of the phi's block. *This is the check that
///    catches the `loop_rotate` stale-guard-label defect.*
/// 5. **No duplicate phi predecessors.** One incoming per edge, otherwise phi
///    elimination emits two conflicting copies on the same edge.
/// 6. **Phi coverage.** Every real predecessor appears in the incoming list,
///    so no live edge reaches the phi without a value.
/// 7. **SSA single definition.** Every `Value` is defined by at most one
///    instruction. A second definition of the same id is malformed even when
///    both sites happen to dominate their uses: consumers index def sites
///    by value id (the def map here, GVN's leader table, regalloc's
///    live-range construction) and silently pick one.
/// 8. **Def dominates use.** For every value use in a reachable block the
///    defining instruction's block dominates the use site. Ordinary operands
///    and terminator operands are uses *in* their block (a same-block def
///    must also precede the use textually); a phi incoming on edge `P -> B`
///    is a use *at the end of P* — the value must be defined on entry to
///    that edge, i.e. its def must dominate `P`. Uses of values with no
///    definition anywhere, and uses of values defined only in unreachable
///    blocks, are reported here too: the backend would read a register that
///    no executed path ever wrote.
pub fn verify_function(func: &IrFunction, stage: &str, out: &mut Vec<Violation>) {
    if func.blocks.is_empty() {
        return;
    }
    let push = |out: &mut Vec<Violation>, detail: String| {
        out.push(Violation {
            stage: stage.to_string(),
            function: func.name.clone(),
            detail,
        });
    };

    // 1. label → index, detecting duplicates.
    let mut label_to_idx: FxHashMap<BlockId, usize> =
        FxHashMap::with_capacity_and_hasher(func.blocks.len(), Default::default());
    let mut max_label: u32 = 0;
    for (i, block) in func.blocks.iter().enumerate() {
        max_label = max_label.max(block.label.0);
        if let Some(&prev) = label_to_idx.get(&block.label) {
            push(
                out,
                format!(
                    "duplicate block label {:?}: blocks #{} and #{}",
                    block.label, prev, i
                ),
            );
        } else {
            label_to_idx.insert(block.label, i);
        }
    }

    // 1b. label-counter health: the documented `IrFunction::next_label`
    // invariant is that every live label is < next_label. A pass that mints
    // blocks without writing the counter back (loop_unroll pre-fix) leaves a
    // stale counter that hands a LATER pass colliding labels — the duplicate
    // above then misattributes blame to the later pass. Naming the stale
    // counter here points at the pass that actually broke the invariant.
    if func.next_label <= max_label {
        push(
            out,
            format!(
                "stale label counter: next_label={} but live max label is {}",
                func.next_label, max_label
            ),
        );
    }

    // 2. + build the real predecessor sets.
    let mut preds: Vec<FxHashSet<BlockId>> = vec![FxHashSet::default(); func.blocks.len()];
    for block in func.blocks.iter() {
        let from = block.label;
        let mut edge =
            |label: BlockId, kind: &str, out: &mut Vec<Violation>| match label_to_idx.get(&label) {
                Some(&to) => {
                    preds[to].insert(from);
                }
                None => push(
                    out,
                    format!("{} in {:?} targets unknown block {:?}", kind, from, label),
                ),
            };
        for_each_target(&block.terminator, |l| edge(l, "terminator", out));
        for inst in &block.instructions {
            if let Instruction::InlineAsm { goto_labels, .. } = inst {
                for (name, label) in goto_labels {
                    edge(*label, &format!("`asm goto` label `{}`", name), out);
                }
            }
        }
    }

    let reachable = reachable_blocks(func, &label_to_idx);

    for (bi, block) in func.blocks.iter().enumerate() {
        // 3. phis must be a contiguous prefix.
        let mut seen_non_phi = false;
        for inst in &block.instructions {
            let is_phi = matches!(inst, Instruction::Phi { .. });
            if is_phi && seen_non_phi {
                push(
                    out,
                    format!(
                        "block #{} ({:?}): phi appears after a non-phi instruction",
                        bi, block.label
                    ),
                );
                break;
            }
            seen_non_phi |= !is_phi;
        }

        // Edge-set checks are meaningful only for blocks that can execute.
        if !reachable[bi] {
            continue;
        }

        // The entry block has no predecessors by construction; a phi there is
        // already covered by checks 4/6 producing an empty expected set.
        let real = &preds[bi];
        for inst in &block.instructions {
            let Instruction::Phi { dest, incoming, .. } = inst else {
                continue;
            };
            let mut listed: FxHashSet<BlockId> = FxHashSet::default();
            for (_, from) in incoming {
                // 5. duplicates.
                if !listed.insert(*from) {
                    push(
                        out,
                        format!(
                            "block #{} ({:?}): phi v{} lists predecessor {:?} more than once",
                            bi, block.label, dest.0, from
                        ),
                    );
                }
                // 4. the stale-label check.
                if !real.contains(from) {
                    push(
                        out,
                        format!(
                            "block #{} ({:?}): phi v{} has an incoming from {:?}, \
                             which is not a predecessor (real predecessors: {:?})",
                            bi,
                            block.label,
                            dest.0,
                            from,
                            sorted(real)
                        ),
                    );
                }
            }
            // 6. coverage -- reachable predecessors only (see `reachable_blocks`).
            for from in real.iter() {
                let from_reachable = label_to_idx
                    .get(from)
                    .map(|&i| reachable[i])
                    .unwrap_or(false);
                if from_reachable && !listed.contains(from) {
                    push(
                        out,
                        format!(
                            "block #{} ({:?}): phi v{} has no incoming for predecessor {:?}",
                            bi, block.label, dest.0, from
                        ),
                    );
                }
            }
        }
    }

    verify_dominance(func, stage, &label_to_idx, &reachable, &preds, out);
}

/// Checks 7 and 8: SSA single definition and def-dominates-use.
///
/// Dominance is computed with the Cooper-Harvey-Kennedy iterative algorithm
/// over a reverse postorder of the *reachable* subgraph, then answered in
/// O(1) via preorder intervals of the dominator tree
/// (`a` dominates `b` iff `tin[a] <= tin[b] <= tout[a]`).
///
/// Gated on reachability for the same reason as the phi edge-set checks:
/// dominance is undefined for unreachable blocks, and passes legitimately
/// leave dead blocks for `cfg_simplify`. What survives the gate is the case
/// that miscompiles: a *reachable* use whose definition no executed path
/// guarantees.
fn verify_dominance(
    func: &IrFunction,
    stage: &str,
    label_to_idx: &FxHashMap<BlockId, usize>,
    reachable: &[bool],
    real_preds: &[FxHashSet<BlockId>],
    out: &mut Vec<Violation>,
) {
    let push = |out: &mut Vec<Violation>, detail: String| {
        out.push(Violation {
            stage: stage.to_string(),
            function: func.name.clone(),
            detail,
        });
    };

    // ---- def map (check 7) -------------------------------------------------
    // value id -> (block idx, instruction idx). A second definition of the
    // same value id is an SSA violation on its own: consumers index def
    // sites by value id (GVN's leader table, regalloc live-range
    // construction) and would silently pick one.
    //
    // `Instruction::dest()` returns None for `InlineAsm`, yet an asm output
    // slot can BE an SSA definition later instructions read directly
    // (verified: `__cpuid`'s "=a".."=d" outputs feed Copies/Casts). The
    // dual shape also exists: a read-write ("+") or memory ("=m") output
    // names the stack home the asm writes THROUGH -- an alloca/GEP whose
    // definition is the pointer everyone shares (`asm_alternative_length_
    // template`'s "+a" names the Alloca itself, with a later Load reading
    // the result back). Model: in program order, an asm output slot whose
    // value is already defined is a MEMORY HOME (a pointer use, checked as
    // a use below); a slot with no prior definition is the SSA definition.
    let mut defs: FxHashMap<u32, (usize, usize)> =
        FxHashMap::with_capacity_and_hasher(func.blocks.len() * 4, Default::default());
    let mut asm_home_slots: FxHashSet<u32> = FxHashSet::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Instruction::InlineAsm { outputs, .. } = inst {
                for (_, dest, _) in outputs {
                    if defs.contains_key(&dest.0) {
                        // Memory home: the asm writes through this pointer.
                        asm_home_slots.insert(dest.0);
                    } else {
                        record_def(&mut defs, out, func, stage, *dest, (bi, ii));
                    }
                }
            } else if let Some(dest) = inst.dest() {
                record_def(&mut defs, out, func, stage, dest, (bi, ii));
            }
        }
    }

    // ---- dominators (CHK over RPO of the reachable subgraph) ---------------
    let dom = Dominance::compute(func, label_to_idx, reachable);

    // ---- check 8: every use gets its def along every executed path --------
    for (bi, block) in func.blocks.iter().enumerate() {
        if !reachable[bi] {
            continue;
        }
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Instruction::Phi { incoming, .. } = inst {
                // A phi incoming is a use at the END of the predecessor it
                // labels. Only real, reachable predecessors are checked:
                // a stale label is check 4's defect (already reported), and
                // a dead edge never consumes its value.
                for (op, from) in incoming {
                    let Operand::Value(v) = op else { continue };
                    let Some(&pred) = label_to_idx.get(from) else {
                        continue;
                    };
                    if !reachable[pred] || !real_preds[bi].contains(from) {
                        continue;
                    }
                    let site = format!(
                        "the incoming from block #{} ({:?}) of the phi in \
                         block #{} ({:?})",
                        pred, func.blocks[pred].label, bi, block.label
                    );
                    check_value_use(
                        func, stage, &defs, reachable, &dom, v.0, pred, None, &site, out,
                    );
                }
            } else if let Instruction::InlineAsm {
                outputs, inputs, ..
            } = inst
            {
                // Inputs are always uses. An output slot that is a memory
                // home (recorded in `asm_home_slots`) is a pointer USE --
                // the asm writes through it; a fresh output slot is the
                // definition itself and no use at all.
                let mut check_op = |out: &mut Vec<Violation>, op: &Operand, what: &str| {
                    let Operand::Value(v) = op else { return };
                    let site = format!(
                        "block #{} ({:?}) at instruction #{} ({})",
                        bi, block.label, ii, what
                    );
                    check_value_use(
                        func,
                        stage,
                        &defs,
                        reachable,
                        &dom,
                        v.0,
                        bi,
                        Some(ii),
                        &site,
                        out,
                    );
                };
                for (_, op, _) in inputs {
                    check_op(out, op, "asm input");
                }
                for (_, slot, _) in outputs {
                    if asm_home_slots.contains(&slot.0) {
                        check_op(out, &Operand::Value(*slot), "asm output home");
                    }
                }
            } else {
                inst.for_each_used_value(|v| {
                    let site = format!("block #{} ({:?}) at instruction #{}", bi, block.label, ii);
                    check_value_use(
                        func,
                        stage,
                        &defs,
                        reachable,
                        &dom,
                        v,
                        bi,
                        Some(ii),
                        &site,
                        out,
                    );
                });
            }
        }
        block.terminator.for_each_used_value(|v| {
            let site = format!("the terminator of block #{} ({:?})", bi, block.label);
            // Every instruction of the block precedes the terminator, so a
            // same-block def needs no ordering test here.
            check_value_use(func, stage, &defs, reachable, &dom, v, bi, None, &site, out);
        });
    }
}

/// Record one definition, reporting a second definition of the same value
/// id as an SSA violation (consumers index def sites by id and would
/// silently pick one).
fn record_def(
    defs: &mut FxHashMap<u32, (usize, usize)>,
    out: &mut Vec<Violation>,
    func: &IrFunction,
    stage: &str,
    v: Value,
    site: (usize, usize),
) {
    match defs.entry(v.0) {
        std::collections::hash_map::Entry::Occupied(prev) => {
            let (pb, pi) = *prev.get();
            out.push(Violation {
                stage: stage.to_string(),
                function: func.name.clone(),
                detail: format!(
                    "v{} defined more than once: block #{} instruction #{} \
                     and block #{} instruction #{}",
                    v.0, pb, pi, site.0, site.1
                ),
            });
        }
        std::collections::hash_map::Entry::Vacant(slot) => {
            slot.insert(site);
        }
    }
}

/// Check one value use: the def must exist, be reachable, and dominate the
/// use. `use_idx` is the instruction index for in-block uses (`None` for
/// terminators and phi-edge uses, where any same-block def precedes the
/// consumption point by construction).
fn check_value_use(
    func: &IrFunction,
    stage: &str,
    defs: &FxHashMap<u32, (usize, usize)>,
    reachable: &[bool],
    dom: &Dominance,
    v: u32,
    use_block: usize,
    use_idx: Option<usize>,
    use_site: &str,
    out: &mut Vec<Violation>,
) {
    let detail = |d: String| Violation {
        stage: stage.to_string(),
        function: func.name.clone(),
        detail: d,
    };
    let Some(&(db, di)) = defs.get(&v) else {
        out.push(detail(format!(
            "use of undefined value v{} in {}",
            v, use_site
        )));
        return;
    };
    if !reachable[db] {
        out.push(detail(format!(
            "use of v{} in {}, but v{} is defined in unreachable block #{} ({:?})",
            v, use_site, v, db, func.blocks[db].label
        )));
        return;
    }
    let ok = dom.dominates(db, use_block) && (db != use_block || use_idx.is_none_or(|ui| di < ui));
    if !ok {
        out.push(detail(format!(
            "v{} (defined in block #{} ({:?}) at instruction #{}) does not \
             dominate its use in {}",
            v, db, func.blocks[db].label, di, use_site
        )));
    }
}

/// Blocks reachable from the entry block, following terminator and `asm goto`
/// edges.
///
/// Reachability gates the three checks that concern a phi's *edge set*
/// (stale / duplicate / missing predecessors) and nothing else. Those checks
/// describe which value arrives along which edge, so they are vacuous when the
/// edge or the block cannot execute:
///
/// * a missing incoming for an unreachable predecessor needs no copy, because
///   phi elimination emits one copy per *listed* edge and that edge is never
///   taken; and
/// * any phi inside an unreachable block is dead by construction. Passes
///   legitimately leave such blocks alone -- SCCP explicitly does, documenting
///   that `cfg_simplify` will delete them -- so flagging them would punish
///   correct behaviour.
///
/// What survives the gate is the case that actually miscompiles: a *reachable*
/// block whose phi names a predecessor that is not one. Phi elimination
/// resolves that label to a block index and emits the copy there, so the value
/// either lands on the wrong edge or never lands at all.
///
/// Phi *contiguity* is deliberately NOT gated: passes index the phi prefix
/// arithmetically (`loop_rotate` scans instructions at block start) without
/// first checking reachability, so the invariant must hold everywhere.
fn reachable_blocks(func: &IrFunction, label_to_idx: &FxHashMap<BlockId, usize>) -> Vec<bool> {
    let mut seen = vec![false; func.blocks.len()];
    if func.blocks.is_empty() {
        return seen;
    }
    let mut stack = vec![0usize];
    seen[0] = true;
    while let Some(bi) = stack.pop() {
        let block = &func.blocks[bi];
        let mut go = |label: BlockId, stack: &mut Vec<usize>| {
            if let Some(&to) = label_to_idx.get(&label) {
                if !seen[to] {
                    seen[to] = true;
                    stack.push(to);
                }
            }
        };
        for_each_target(&block.terminator, |l| go(l, &mut stack));
        for inst in &block.instructions {
            if let Instruction::InlineAsm { goto_labels, .. } = inst {
                for (_, label) in goto_labels {
                    go(*label, &mut stack);
                }
            }
        }
    }
    seen
}

/// Stable ordering for diagnostics (hash-set iteration order is not stable).
fn sorted(s: &FxHashSet<BlockId>) -> Vec<u32> {
    let mut v: Vec<u32> = s.iter().map(|b| b.0).collect();
    v.sort_unstable();
    v
}

/// Dominator information over the reachable subgraph.
///
/// Computed with the Cooper-Harvey-Kennedy iterative dominator construction
/// ("A Simple, Fast Dominance Algorithm", 2001): immediate dominators are
/// refined over blocks in reverse postorder until a fixed point, with the
/// `intersect` walk choosing the highest common ancestor in the dominator
/// tree by RPO number. Queries then run in O(1) through preorder intervals
/// of the dominator tree: `a` dominates `b` iff `a`'s interval contains
/// `b`'s preorder number.
///
/// Only reachable blocks take part (indexing is by block index; unreachable
/// entries hold `u32::MAX` and are never queried — check 8 skips them by
/// construction, and check 8's def-side gate reports defs there instead).
struct Dominance {
    /// Preorder entry time in the dominator-tree DFS, `u32::MAX` if
    /// unreachable.
    tin: Vec<u32>,
    /// Preorder exit time: the largest `tin` inside `a`'s subtree.
    tout: Vec<u32>,
}

impl Dominance {
    fn compute(
        func: &IrFunction,
        label_to_idx: &FxHashMap<BlockId, usize>,
        reachable: &[bool],
    ) -> Dominance {
        let n = func.blocks.len();
        let unreachable = u32::MAX;
        let mut dom = Dominance {
            tin: vec![unreachable; n],
            tout: vec![unreachable; n],
        };
        if n == 0 || !reachable[0] {
            return dom;
        }

        // Successor lists (terminator + `asm goto` edges), reachable blocks
        // only, deduplicated, in first-seen order.
        let succs: Vec<Vec<usize>> = func
            .blocks
            .iter()
            .enumerate()
            .map(|(bi, block)| {
                let mut list: Vec<usize> = Vec::new();
                let mut add = |label: BlockId, list: &mut Vec<usize>| {
                    if let Some(&to) = label_to_idx.get(&label) {
                        if reachable[to] && !list.contains(&to) {
                            list.push(to);
                        }
                    }
                };
                if !reachable[bi] {
                    return list;
                }
                for_each_target(&block.terminator, |l| add(l, &mut list));
                for inst in &block.instructions {
                    if let Instruction::InlineAsm { goto_labels, .. } = inst {
                        for (_, label) in goto_labels {
                            add(*label, &mut list);
                        }
                    }
                }
                list
            })
            .collect();

        // Postorder DFS from the entry block; RPO is its reverse.
        let mut postorder: Vec<usize> = Vec::with_capacity(n);
        let mut state: Vec<u8> = vec![0; n]; // 0 = unvisited, 1 = on stack
        let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
        state[0] = 1;
        while let Some(&mut (bi, ref mut next)) = stack.last_mut() {
            if *next < succs[bi].len() {
                let to = succs[bi][*next];
                *next += 1;
                if state[to] == 0 {
                    state[to] = 1;
                    stack.push((to, 0));
                }
            } else {
                postorder.push(bi);
                stack.pop();
            }
        }
        let rpo: Vec<usize> = postorder.iter().rev().copied().collect();
        let mut rpo_num = vec![unreachable; n];
        for (i, &b) in rpo.iter().enumerate() {
            rpo_num[b] = i as u32;
        }

        // Predecessor lists over the reachable subgraph.
        let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
        for bi in &rpo {
            for &to in &succs[*bi] {
                if !preds[to].contains(bi) {
                    preds[to].push(*bi);
                }
            }
        }

        // CHK: idom[b] = intersect of already-processed reachable preds.
        let entry = rpo[0];
        let mut idom = vec![usize::MAX; n];
        idom[entry] = entry;
        let intersect = |mut a: usize, mut b: usize, idom: &[usize], rpo_num: &[u32]| -> usize {
            while a != b {
                while rpo_num[a] > rpo_num[b] {
                    a = idom[a];
                }
                while rpo_num[b] > rpo_num[a] {
                    b = idom[b];
                }
            }
            a
        };
        // Iterate to a fixed point: a single RPO pass is exact only for
        // reducible CFGs. Irreducible control flow (jump threading, computed
        // `goto`, `asm goto` joins) can leave `idom[]` over-approximated
        // after one pass, and an over-approximate dominator tree makes
        // check 8 ACCEPT def-use pairs the program does not dominate — a
        // verifier hole exactly where miscompiles hide. Reducible graphs
        // converge in two passes; each pass is linear.
        let mut changed = true;
        while changed {
            changed = false;
            for &b in rpo.iter().skip(1) {
                let mut new_idom = usize::MAX;
                for &p in &preds[b] {
                    if idom[p] == usize::MAX {
                        continue; // not yet processed (cannot happen in RPO, cheap guard)
                    }
                    new_idom = if new_idom == usize::MAX {
                        p
                    } else {
                        intersect(p, new_idom, &idom, &rpo_num)
                    };
                }
                // A reachable non-entry block has at least one reachable pred,
                // and in RPO at least one pred was processed before it.
                debug_assert!(
                    new_idom != usize::MAX,
                    "RPO order invariant broken: block {b} has no processed predecessor"
                );
                if new_idom != usize::MAX && idom[b] != new_idom {
                    idom[b] = new_idom;
                    changed = true;
                }
            }
        }

        // Dominator-tree preorder intervals for O(1) dominates(), assigned
        // iteratively (enter/leave marks): the tree can be as deep as the
        // CFG, and a verifier must never blow the stack.
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (b, &i) in idom.iter().enumerate() {
            if i != usize::MAX && b != entry {
                children[i].push(b);
            }
        }
        let mut clock = 0u32;
        let mut stack: Vec<(usize, bool)> = vec![(entry, false)];
        while let Some((b, leaving)) = stack.pop() {
            if leaving {
                dom.tout[b] = clock - 1;
                continue;
            }
            dom.tin[b] = clock;
            clock += 1;
            stack.push((b, true));
            for &c in children[b].iter().rev() {
                stack.push((c, false));
            }
        }

        dom
    }

    /// `a` dominates `b` (both reachable). Reflexive.
    fn dominates(&self, a: usize, b: usize) -> bool {
        a == b || (self.tin[a] <= self.tin[b] && self.tin[b] <= self.tout[a])
    }
}

/// Verify every defined function in `module`. Returns the violations found.
pub fn verify_module(module: &IrModule, stage: &str) -> Vec<Violation> {
    let mut out = Vec::new();
    for func in module.functions.iter() {
        // Declarations have no body to check.
        if func.blocks.is_empty() {
            continue;
        }
        verify_function(func, stage, &mut out);
    }
    out
}

/// Per-function pass hook, for passes that run inside a shared-analysis loop
/// rather than through `timed_pass!` (gvn / licm / ivsr / univsr /
/// load_forward). Without these, a defect from one of them is first reported
/// by whichever *wrapped* pass runs next, which points the blame at an
/// innocent pass -- exactly the mis-attribution this module exists to prevent.
pub fn verify_after_func_pass(func: &IrFunction, stage: &str) {
    let m = mode();
    if m == Mode::Off || func.blocks.is_empty() {
        return;
    }
    let mut violations = Vec::new();
    verify_function(func, stage, &mut violations);
    if violations.is_empty() {
        return;
    }
    for v in &violations {
        eprintln!("{}", v);
    }
    if m == Mode::Abort {
        panic!(
            "IR verification failed after `{}`: {} violation(s); \
             the pass named here produced malformed IR",
            stage,
            violations.len()
        );
    }
}

/// Pass-loop hook: a no-op unless `CCC_VERIFY_IR` is set.
///
/// Deliberately checks the env var *before* touching the module so the
/// disabled path costs one `getenv` per pass and allocates nothing.
pub fn verify_after_pass(module: &IrModule, stage: &str) {
    let m = mode();
    if m == Mode::Off {
        return;
    }
    let violations = verify_module(module, stage);
    if violations.is_empty() {
        return;
    }
    for v in &violations {
        eprintln!("{}", v);
    }
    if m == Mode::Abort {
        panic!(
            "IR verification failed after `{}`: {} violation(s); \
             the pass named here produced malformed IR",
            stage,
            violations.len()
        );
    }
}

#[cfg(test)]
mod tests;
