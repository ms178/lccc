//! Interprocedural no-return discovery for internal functions.
//!
//! A `static` function none of whose blocks returns cannot hand control back
//! to a direct caller, whatever its declaration says: every path through it
//! ends in a call that does not return, a trap, or a loop without an exit.
//! GCC finds this in `ipa-pure-const` ("Function found to be noreturn") and
//! drops whatever follows each direct call.
//!
//! objtool applies the same rule to local symbols -- a function without a
//! return instruction is a dead end -- and reports the caller's surviving
//! continuation as
//!
//! ```text
//! io_wq_worker+0x47a: io_worker_exit() missing __noreturn in .c/.h or
//! NORETURN() in noreturns.h
//! ```
//!
//! Linux 6.18.63 has three such sites in an x86-64 build: `io_worker_exit`
//! (io_uring/io-wq.c, ends in `do_exit`) and the two C callers of
//! `__sysvec_reboot` (arch/x86/kernel/smp.c, ends in `stop_this_cpu`). GCC 16.2
//! is silent on all three; neither function is annotated in the source.
//!
//! This pass ends the calling block after such a call, so the backend emits
//! the same `call; ud2` it emits for a declared `noreturn` callee.
//!
//! # What is inferred
//!
//! Only a function this translation unit defines exactly once with internal
//! linkage, so that a direct call by name can reach no other body. A function
//! with external linkage is left alone even where it could not be interposed:
//! objtool's own inference stops at local symbols, and that is the contract
//! this pass exists to meet.
//!
//! "No block returns" is asked of every block, reachable or not. A stale
//! unreachable `Return` therefore only costs the optimization, and the answer
//! cannot depend on an edge this pass failed to model. Inline assembly is
//! taken to fall through, as GCC takes it; a `naked` body, which is nothing
//! but assembly and returns on its own, is excluded.
//!
//! # What is cut
//!
//! A call already known to be `pure` or `const` is not touched: dead-code
//! elimination may delete such a call, and what it leaves behind must not be
//! a block that ends without one.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::{BlockId, Instruction, IntrinsicOp, IrFunction, IrModule, Terminator};

/// End the calling block after each direct call to an internal function that
/// cannot return. Returns the number of call sites changed.
pub(crate) fn run(module: &mut IrModule) -> usize {
    let never_return = functions_that_never_return(module);
    if never_return.is_empty() {
        return 0;
    }
    let mut total = 0;
    for func in &mut module.functions {
        if func.is_declaration {
            continue;
        }
        let cut = end_blocks_after_calls(func, &never_return);
        if cut > 0 {
            // The removed edges can strand whole blocks and leave phis with a
            // single input. Clean up here rather than rely on a later
            // iteration: this may be the last one.
            super::cfg_simplify::run_function(func);
            total += cut;
        }
    }
    total
}

/// Names of the internal functions that no path returns from.
fn functions_that_never_return(module: &IrModule) -> FxHashSet<String> {
    let mut definitions: FxHashMap<&str, u32> = FxHashMap::default();
    for func in module.functions.iter().filter(|f| !f.is_declaration) {
        *definitions.entry(func.name.as_str()).or_default() += 1;
    }
    module
        .functions
        .iter()
        .filter(|func| {
            !func.is_declaration
                && func.is_static
                && !func.is_weak
                && !func.is_naked
                && !func.is_gnu_inline_def
                && definitions.get(func.name.as_str()) == Some(&1)
                && never_returns(func)
        })
        .map(|func| func.name.clone())
        .collect()
}

fn never_returns(func: &IrFunction) -> bool {
    !func.blocks.is_empty()
        && func.blocks.iter().all(|block| {
            !matches!(block.terminator, Terminator::Return(_))
                && !block.instructions.iter().any(leaves_outside_the_cfg)
        })
}

/// Transfers of control that are not edges of this function's graph. A nested
/// function's `goto` into its parent resumes the *caller's* frame somewhere
/// other than after the call; the setjmp/apply builtins re-enter or leave a
/// frame by saved state. None of them is worth reasoning about here.
fn leaves_outside_the_cfg(inst: &Instruction) -> bool {
    match inst {
        Instruction::NonlocalGoto { .. } | Instruction::NonlocalGotoSave { .. } => true,
        Instruction::Intrinsic { op, .. } => matches!(
            op,
            IntrinsicOp::BuiltinSetjmp
                | IntrinsicOp::BuiltinLongjmp
                | IntrinsicOp::SaveApplyArgs
                | IntrinsicOp::DoBuiltinApply
                | IntrinsicOp::RestoreApplyResult
        ),
        _ => false,
    }
}

fn asm_goto_labels(insts: &[Instruction]) -> impl Iterator<Item = BlockId> + '_ {
    insts
        .iter()
        .flat_map(|inst| match inst {
            Instruction::InlineAsm { goto_labels, .. } => goto_labels.as_slice(),
            _ => &[],
        })
        .map(|(_, label)| *label)
}

fn terminator_targets(term: &Terminator) -> Vec<BlockId> {
    match term {
        Terminator::Branch(target) => vec![*target],
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => vec![*true_label, *false_label],
        Terminator::IndirectBranch {
            possible_targets, ..
        } => possible_targets.clone(),
        Terminator::Switch { cases, default, .. } => std::iter::once(*default)
            .chain(cases.iter().map(|&(_, label)| label))
            .collect(),
        Terminator::Return(_) | Terminator::Unreachable => Vec::new(),
    }
}

/// Cut each block of `func` after its first call into `never_return`.
///
/// The successors lose this block as a predecessor, so their phi inputs from
/// it go too -- except where an `asm goto` *before* the call still jumps
/// there: that edge is taken before the call runs and survives the cut.
fn end_blocks_after_calls(func: &mut IrFunction, never_return: &FxHashSet<String>) -> usize {
    let mut cut = 0;
    for index in 0..func.blocks.len() {
        let block = &func.blocks[index];
        let Some(call) = block.instructions.iter().position(|inst| {
            matches!(inst, Instruction::Call { func: callee, info }
                if !info.is_pure && !info.is_const && never_return.contains(callee.as_str()))
        }) else {
            continue;
        };
        let after = call + 1;
        if after == block.instructions.len() && matches!(block.terminator, Terminator::Unreachable)
        {
            continue;
        }
        let label = block.label;
        let kept: FxHashSet<BlockId> = asm_goto_labels(&block.instructions[..call]).collect();
        let lost: FxHashSet<BlockId> = terminator_targets(&block.terminator)
            .into_iter()
            .chain(asm_goto_labels(&block.instructions[after..]))
            .filter(|target| !kept.contains(target))
            .collect();

        let block = &mut func.blocks[index];
        let len = block.instructions.len();
        block.remove_insts(after..len);
        block.terminator = Terminator::Unreachable;
        for successor in func.blocks.iter_mut().filter(|b| lost.contains(&b.label)) {
            for inst in &mut successor.instructions {
                if let Instruction::Phi { incoming, .. } = inst {
                    incoming.retain(|(_, from)| *from != label);
                }
            }
        }
        cut += 1;
    }
    cut
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::reexports::{BasicBlock, CallInfo, IrConst, Operand, Value};

    fn block(id: u32, insts: Vec<Instruction>, term: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(id),
            instructions: insts,
            terminator: term,
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        }
    }

    fn call(name: &str) -> Instruction {
        Instruction::Call {
            func: name.into(),
            info: CallInfo {
                return_type: IrType::Void,
                ..Default::default()
            },
        }
    }

    fn function(name: &str, is_static: bool, blocks: Vec<BasicBlock>) -> IrFunction {
        let mut f = IrFunction::new(name.into(), IrType::I32, vec![], false);
        f.is_static = is_static;
        f.next_label = blocks.iter().map(|b| b.label.0 + 1).max().unwrap_or(0);
        f.next_value_id = 16;
        f.blocks = blocks;
        f
    }

    /// `static void die(void) { abort(); }`: one block, no return.
    fn die(is_static: bool) -> IrFunction {
        function(
            "die",
            is_static,
            vec![block(0, vec![call("abort")], Terminator::Unreachable)],
        )
    }

    fn ret(value: i32) -> Terminator {
        Terminator::Return(Some(Operand::Const(IrConst::I32(value))))
    }

    fn module(functions: Vec<IrFunction>) -> IrModule {
        let mut module = IrModule::new();
        module.functions = functions;
        module
    }

    fn caller(module: &IrModule) -> &IrFunction {
        module.functions.iter().find(|f| f.name == "run").unwrap()
    }

    /// `die(); return 7;` -- the shape objtool reports as `die() missing
    /// __noreturn`: the return after the call can never run.
    #[test]
    fn a_call_to_a_static_function_without_a_return_ends_the_block() {
        let run_fn = function("run", false, vec![block(0, vec![call("die")], ret(7))]);
        let mut m = module(vec![die(true), run_fn]);
        assert_eq!(run(&mut m), 1);
        let f = caller(&m);
        assert_eq!(f.blocks.len(), 1);
        assert_eq!(f.blocks[0].instructions.len(), 1);
        assert!(matches!(f.blocks[0].terminator, Terminator::Unreachable));
        // The caller no longer returns either, but it has external linkage:
        // nothing may be concluded for its own callers. A second run finds
        // the block already cut and changes nothing.
        assert_eq!(run(&mut m), 0);
    }

    /// The property is transitive for internal callers, one run per link.
    #[test]
    fn an_internal_caller_that_only_dies_is_found_on_the_next_run() {
        let relay = function("relay", true, vec![block(0, vec![call("die")], ret(1))]);
        let run_fn = function("run", false, vec![block(0, vec![call("relay")], ret(7))]);
        let mut m = module(vec![die(true), relay, run_fn]);
        assert_eq!(run(&mut m), 1);
        assert!(matches!(
            caller(&m).blocks[0].terminator,
            Terminator::Return(_)
        ));
        assert_eq!(run(&mut m), 1);
        assert!(matches!(
            caller(&m).blocks[0].terminator,
            Terminator::Unreachable
        ));
    }

    #[test]
    fn a_function_that_may_be_another_body_or_may_return_is_left_alone() {
        let run_fn = || function("run", false, vec![block(0, vec![call("die")], ret(7))]);
        // External linkage: objtool does not infer it, and neither do we.
        let mut m = module(vec![die(false), run_fn()]);
        assert_eq!(run(&mut m), 0);

        let mut weak = die(true);
        weak.is_weak = true;
        let mut m = module(vec![weak, run_fn()]);
        assert_eq!(run(&mut m), 0);

        // A naked body is assembly that returns by itself.
        let mut naked = die(true);
        naked.is_naked = true;
        let mut m = module(vec![naked, run_fn()]);
        assert_eq!(run(&mut m), 0);

        // One returning path is enough.
        let mut returns = die(true);
        returns.blocks.push(block(1, vec![], ret(0)));
        let mut m = module(vec![returns, run_fn()]);
        assert_eq!(run(&mut m), 0);

        // Two bodies under one name: a call cannot be attributed to either.
        let mut m = module(vec![die(true), die(true), run_fn()]);
        assert_eq!(run(&mut m), 0);

        // A prototype without a body says nothing.
        let mut declared = die(true);
        declared.is_declaration = true;
        declared.blocks.clear();
        let mut m = module(vec![declared, run_fn()]);
        assert_eq!(run(&mut m), 0);
        assert!(matches!(
            caller(&m).blocks[0].terminator,
            Terminator::Return(_)
        ));
    }

    /// A nested function's `goto` into its parent resumes the caller's frame.
    #[test]
    fn a_function_that_leaves_by_nonlocal_goto_is_left_alone() {
        let mut escapes = die(true);
        escapes.blocks[0]
            .instructions
            .push(Instruction::NonlocalGoto {
                chain: Operand::Const(IrConst::I64(0)),
                up: 1,
                rbp_off: 0,
                rsp_off: 0,
                label: "out".into(),
            });
        let run_fn = function("run", false, vec![block(0, vec![call("die")], ret(7))]);
        let mut m = module(vec![escapes, run_fn]);
        assert_eq!(run(&mut m), 0);
    }

    /// DCE may delete a pure call; the block must not then end in nothing.
    #[test]
    fn a_pure_or_const_call_site_is_not_cut() {
        for (is_pure, is_const) in [(true, false), (false, true)] {
            let pure_call = Instruction::Call {
                func: "die".into(),
                info: CallInfo {
                    return_type: IrType::Void,
                    is_pure,
                    is_const,
                    ..Default::default()
                },
            };
            let run_fn = function("run", false, vec![block(0, vec![pure_call], ret(7))]);
            let mut m = module(vec![die(true), run_fn]);
            assert_eq!(run(&mut m), 0);
            assert!(matches!(
                caller(&m).blocks[0].terminator,
                Terminator::Return(_)
            ));
        }
    }

    fn phi(dest: u32, incoming: &[(i32, u32)]) -> Instruction {
        Instruction::Phi {
            dest: Value(dest),
            ty: IrType::I32,
            incoming: incoming
                .iter()
                .map(|&(v, from)| (Operand::Const(IrConst::I32(v)), BlockId(from)))
                .collect(),
        }
    }

    fn phi_sources(block: &BasicBlock) -> Vec<u32> {
        match &block.instructions[0] {
            Instruction::Phi { incoming, .. } => incoming.iter().map(|(_, b)| b.0).collect(),
            other => panic!("expected a phi, found {other:?}"),
        }
    }

    fn never_return() -> FxHashSet<String> {
        std::iter::once("die".to_string()).collect()
    }

    /// `if (c) die(); return c ? 7 : 8;` -- the join keeps the arm that still
    /// reaches it and loses the input of the arm that died.
    #[test]
    fn a_join_drops_only_the_input_of_the_arm_that_died() {
        let branch = Terminator::CondBranch {
            cond: Operand::Value(Value(0)),
            true_label: BlockId(1),
            false_label: BlockId(2),
        };
        let mut f = function(
            "run",
            false,
            vec![
                block(0, vec![], branch),
                block(
                    1,
                    vec![call("die"), call("after")],
                    Terminator::Branch(BlockId(3)),
                ),
                block(2, vec![], Terminator::Branch(BlockId(3))),
                block(
                    3,
                    vec![phi(1, &[(7, 1), (8, 2)])],
                    Terminator::Return(Some(Operand::Value(Value(1)))),
                ),
            ],
        );
        assert_eq!(end_blocks_after_calls(&mut f, &never_return()), 1);
        assert_eq!(
            f.blocks[1].instructions.len(),
            1,
            "the call stays, `after` goes"
        );
        assert!(matches!(f.blocks[1].terminator, Terminator::Unreachable));
        assert_eq!(phi_sources(&f.blocks[3]), [2]);
        assert_eq!(end_blocks_after_calls(&mut f, &never_return()), 0);
    }

    /// An `asm goto` ahead of the call is taken before the call runs, so its
    /// edge -- and the phi input that describes it -- outlives the cut. One
    /// that comes after the call dies with the rest of the block.
    #[test]
    fn an_asm_goto_before_the_call_keeps_its_edge_and_one_after_loses_it() {
        let asm_goto = |target: u32| Instruction::InlineAsm {
            template: "jmp %l[out]".into(),
            outputs: vec![],
            inputs: vec![],
            clobbers: vec![],
            operand_types: vec![],
            goto_labels: vec![("out".into(), BlockId(target))],
            input_symbols: vec![],
            seg_overrides: vec![],
        };
        let mut f = function(
            "run",
            false,
            vec![
                block(
                    0,
                    vec![asm_goto(1), call("die"), asm_goto(2)],
                    Terminator::Branch(BlockId(1)),
                ),
                block(1, vec![phi(1, &[(7, 0)])], ret(1)),
                block(2, vec![phi(2, &[(8, 0)])], ret(2)),
            ],
        );
        assert_eq!(end_blocks_after_calls(&mut f, &never_return()), 1);
        assert_eq!(f.blocks[0].instructions.len(), 2);
        assert!(matches!(f.blocks[0].terminator, Terminator::Unreachable));
        assert_eq!(
            phi_sources(&f.blocks[1]),
            [0],
            "still entered by the first asm goto"
        );
        assert!(
            phi_sources(&f.blocks[2]).is_empty(),
            "only the dead asm goto led here"
        );
    }

    /// A block that loops to itself is its own successor.
    #[test]
    fn a_self_loop_loses_its_own_backedge_input() {
        let mut f = function(
            "run",
            false,
            vec![
                block(0, vec![], Terminator::Branch(BlockId(1))),
                block(
                    1,
                    vec![phi(1, &[(0, 0), (1, 1)]), call("die")],
                    Terminator::Branch(BlockId(1)),
                ),
            ],
        );
        assert_eq!(end_blocks_after_calls(&mut f, &never_return()), 1);
        assert_eq!(phi_sources(&f.blocks[1]), [0]);
    }
}
