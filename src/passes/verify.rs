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
use crate::ir::instruction::{BlockId, Terminator};
use crate::ir::reexports::{Instruction, IrFunction, IrModule};

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
    static MODE: std::sync::OnceLock<Mode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("CCC_VERIFY_IR") {
        Err(_) => Mode::Off,
        Ok(v) => match v.trim() {
            "" | "0" | "off" | "no" => Mode::Off,
            "abort" | "panic" | "2" => Mode::Abort,
            _ => Mode::Report,
        },
    })
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
    for (i, block) in func.blocks.iter().enumerate() {
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
