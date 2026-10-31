//! Bounded recursive self-inlining.
//!
//! A small, directly self-recursive function is inlined into its own
//! self-call sites for a few rounds, so one call runs several recursion
//! levels.  The win is not only the saved calls: each inlined level owns its
//! copy of the base-case branch, and the copy at level `j` only ever sees
//! recursion depths `d - j (mod levels)`.  For a tree walk, the base case
//! (`depth == 0`, `n->left == NULL`) then lands almost entirely on one copy
//! and every other copy is always-taken, where a single site sees the whole
//! depth-first outcome sequence.  `binary_trees`' `make`/`destroy` mispredict
//! 82,735/62,680 times from one site each against 2,948/1,503 for GCC 16.2,
//! which inlines them into themselves the same way (Callgrind branch
//! simulation, `TREE_DEPTH=10`; the 14700KF report shows 5.5M vs 1.09M
//! branch misses for the full run).
//!
//! Bounds (every one is required):
//! * a frame-neutral body: no referenced `Alloca` and no `DynAlloca`, so
//!   the stack used per source-level recursion level does not grow;
//! * no frame save points: no `setjmp`-family call, `__builtin_setjmp`,
//!   `__builtin_longjmp` or non-local goto;
//! * at most [`MAX_SELF_CALLS`] self-calls and [`MAX_BODY_INSTS`] IR
//!   instructions in the original body;
//! * at most [`MAX_COPIES`] body copies in total, [`MAX_ROUNDS`] rounds and
//!   [`MAX_GROWN_INSTS`] instructions after growth -- two rounds (three
//!   levels) for a two-call tree walk, three rounds for a one-call walk such
//!   as `check` after accumulator recursion elimination;
//! * everything the ordinary inliner requires of a callee
//!   ([`super::callee_data_for`]: no `noinline`, weak, inline asm, computed
//!   goto, VA operations, ...), the caller being the callee.
//!
//! Each round splices the *original* body into every self-call present at
//! the start of the round, so round `r` adds exactly one level.

use super::{CallArgMeta, InlineCallSite, callee_context, callee_data_for, inline_call_site};
use crate::backend::liveness::is_returns_twice_call;
use crate::common::fx_hash::FxHashSet;
use crate::ir::intrinsics::IntrinsicOp;
use crate::ir::reexports::{Instruction, IrFunction, IrModule};

/// Most self-calls the original body may make.
const MAX_SELF_CALLS: usize = 2;
/// Largest original body (IR instructions).
const MAX_BODY_INSTS: usize = 32;
/// Most body copies after growth, the original included.
const MAX_COPIES: usize = 8;
/// Most inlining rounds (added recursion levels).
const MAX_ROUNDS: usize = 3;
/// Largest grown body (IR instructions).
const MAX_GROWN_INSTS: usize = 160;

/// Does `f` use a stack slot?  Dead slots (the parameter homes mem2reg
/// leaves unreferenced) get no frame space and do not count.
fn uses_stack_slots(f: &IrFunction) -> bool {
    let mut used: FxHashSet<u32> = FxHashSet::default();
    for b in &f.blocks {
        for i in &b.instructions {
            i.for_each_used_value(|v| {
                used.insert(v);
            });
        }
        b.terminator.for_each_used_value(|v| {
            used.insert(v);
        });
    }
    f.blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .any(|i| match i {
            Instruction::Alloca { dest, .. } => used.contains(&dest.0),
            Instruction::DynAlloca { .. } => true,
            _ => false,
        })
}

/// Does `f` record its own frame for a later non-local return
/// (`setjmp`-family calls, `__builtin_setjmp`/`__builtin_longjmp`,
/// non-local goto)?  Self-inlining would put the save points of several
/// recursion levels into one frame.  The backend handles a single
/// returns_twice site per frame level correctly, but nothing tests the
/// shared-frame shape and such functions are never hot recursion, so the
/// pass stays closed for them.
fn saves_its_frame(f: &IrFunction) -> bool {
    f.blocks.iter().flat_map(|b| &b.instructions).any(|i| {
        is_returns_twice_call(i)
            || matches!(
                i,
                Instruction::NonlocalGotoSave { .. }
                    | Instruction::NonlocalGoto { .. }
                    | Instruction::Intrinsic {
                        op: IntrinsicOp::BuiltinSetjmp | IntrinsicOp::BuiltinLongjmp,
                        ..
                    }
            )
    })
}

fn inst_count(f: &IrFunction) -> usize {
    f.blocks.iter().map(|b| b.instructions.len()).sum()
}

/// Positions `(block, instruction)` of the self-calls, last first.
fn self_call_sites(f: &IrFunction) -> Vec<(usize, usize)> {
    let mut sites: Vec<(usize, usize)> = f
        .blocks
        .iter()
        .enumerate()
        .flat_map(|(bi, b)| {
            b.instructions
                .iter()
                .enumerate()
                .filter(|(_, i)| matches!(i, Instruction::Call { func, .. } if *func == f.name))
                .map(move |(ii, _)| (bi, ii))
        })
        .collect();
    sites.sort_unstable_by(|a, b| b.cmp(a));
    sites
}

/// Rounds of self-inlining for a body of `insts` instructions making
/// `calls` self-calls: each round multiplies the open call sites by
/// `calls` and adds one copy per site.
fn planned_rounds(insts: usize, calls: usize) -> usize {
    let (mut copies, mut open, mut rounds) = (1usize, calls, 0usize);
    while rounds < MAX_ROUNDS {
        let next = copies + open;
        if next > MAX_COPIES || next * insts > MAX_GROWN_INSTS {
            break;
        }
        copies = next;
        open *= calls;
        rounds += 1;
    }
    rounds
}

/// Inline small self-recursive functions into themselves (see the module
/// docs).  Returns the number of call sites inlined.
pub(crate) fn inline_self_recursion(module: &mut IrModule) -> usize {
    let ctx = callee_context(module);
    let mut global_max_block_id = module
        .functions
        .iter()
        .flat_map(|f| f.blocks.iter().map(|b| b.label.0))
        .max()
        .unwrap_or(0);
    let mut total = 0;
    for fi in 0..module.functions.len() {
        let f = &module.functions[fi];
        if f.is_declaration || f.is_variadic || f.blocks.is_empty() {
            continue;
        }
        let calls = self_call_sites(f).len();
        let insts = inst_count(f);
        let frame_neutral = !uses_stack_slots(f);
        if calls == 0
            || calls > MAX_SELF_CALLS
            || insts > MAX_BODY_INSTS
            || !frame_neutral
            || saves_its_frame(f)
        {
            continue;
        }
        let rounds = planned_rounds(insts, calls);
        if rounds == 0 {
            continue;
        }
        let Some(body) = callee_data_for(f, module, &ctx) else {
            continue;
        };
        let before = total;
        'rounds: for _ in 0..rounds {
            let func = &mut module.functions[fi];
            for (bi, ii) in self_call_sites(func) {
                let Instruction::Call { func: name, info } = &func.blocks[bi].instructions[ii]
                else {
                    break 'rounds;
                };
                let site = InlineCallSite {
                    block_idx: bi,
                    inst_idx: ii,
                    callee_name: name.clone(),
                    dest: info.dest,
                    args: info.args.clone(),
                    pgo_force: false,
                    arg_meta: CallArgMeta::from_call_info(info),
                };
                if !inline_call_site(func, &site, &body, &mut global_max_block_id) {
                    break 'rounds;
                }
                func.has_inlined_calls = true;
                total += 1;
            }
        }
        if total > before {
            super::note_self_inlined(&module.functions[fi].name);
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_follow_the_copy_and_size_budgets() {
        // Two-call tree walk: 1 -> 3 -> 7 copies (15 would exceed 8).
        assert_eq!(planned_rounds(10, 2), 2);
        // One-call walk: 1 -> 2 -> 3 -> 4 copies, capped by MAX_ROUNDS.
        assert_eq!(planned_rounds(8, 1), 3);
        // Size: 3 copies of 32 fit in 160, 7 copies do not.
        assert_eq!(planned_rounds(32, 2), 1);
        // A body too big to copy even once.
        assert_eq!(planned_rounds(90, 1), 0);
    }
}
