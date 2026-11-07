//! Return duplication into call blocks, so branch arms can end in a
//! sibling call (x86-64, AArch64, RISC-V).
//!
//! The sibling-call conversion (`generation::sibcall_block_hint`, TODO-3 /
//! §5b of the codegen-gaps ledger) recognises a call only when it is the
//! last instruction of a block whose own terminator returns the call's
//! result. A dispatcher, the most common tail-call shape in C,
//!
//! ```c
//! void dispatch(void) { if (G & 32) fast(); else slow(); }
//! int sel(int c, int x) { return c ? g1(x) : g2(x); }
//! ```
//!
//! does not lower that way: each arm calls and then branches to one shared
//! block `R` = `[phi]* ; Return`, so neither call qualified, and LCCC
//! emitted `subq $8, %rsp; ... call fast; jmp R; ... call slow; R: addq $8,
//! %rsp; ret`. GCC 16.2, Clang 23.1.4 and ICX 2026.2 all emit
//! `testb $32, G(%rip); je; jmp fast; jmp slow` (frameless, no `call`);
//! on AArch64 and RISC-V Clang 23.1.5 likewise emits `cbz w0, .L; mov w0,
//! w1; b g1; .L: mov w0, w1; b g2` and `beqz a0, .L; mv a0, a1; tail g1;
//! .L: mv a0, a1; tail g2`.
//!
//! The rewrite is LLVM CodeGenPrepare's `dupRetToEnableTailCallOpts`,
//! stated over this IR. For a block `R` whose instructions are all phis and
//! whose terminator is `Return(op)`, every predecessor `P` that reaches `R`
//! through an unconditional `Branch(R)` and whose last instruction is a
//! call `d = call ...` gets `R`'s return in place of the branch:
//!
//! * `Return(None)`: `P` returns nothing; the call's result, if any, must
//!   be dead (no use anywhere), exactly the backend's void-forwarder rule;
//! * `Return(Some(w))`, `w = phi [.., (d, P), ..]` in `R`: `P` returns `d`
//!   itself; `d`'s only use must be that phi entry, so after the rewrite
//!   its only use is `P`'s `Return`, the backend's single-use rule.
//!
//! `P`'s entries leave `R`'s phis. When `R` loses every predecessor it is
//! deleted; when exactly one remains, `R`'s phis become copies (a one-entry
//! phi is a copy, and the phi-elimination copy webs need no special case).
//! The duplicated `Return` is the same instruction executed on the same
//! path, so the transformation is semantics-preserving by construction.
//!
//! The duplication only pays when the backend then converts the call, so it
//! applies the backend's refusals that are visible in the IR: the function
//! guard (no live local alloca, no dynamic alloca, no inline asm), and per
//! site a returns-twice callee, a variadic/sret/fastcall/regparm callee, or
//! any argument that might not travel in a register. The argument test is a
//! strict subset of `call_abi::classify_call_args`' register classes for
//! the target ([`SiblingAbi`]: at most six INTEGER and eight SSE scalars on
//! x86-64, eight GP and eight FP scalars on AArch64 and RISC-V, no
//! aggregates, no x87 or 128-bit float), so a site this pass rewrites is
//! one the backend arms; a site it refuses keeps today's code. The cases
//! the backend can still refuse after a rewrite (the kernel return-thunk
//! build, `-fno-omit` style per-function state) cost one duplicated
//! epilogue instead of a branch, which is never slower.
//!
//! `-g` rows: `R`'s recorded return row moves with the return, so the
//! duplicated terminator keeps a line, and `-g` never changes the rewrite.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::{BlockId, CallInfo, Instruction, IrFunction, Operand, Terminator};

/// The register budget and IR-visible refusals of one target's sibling-call
/// conversion (`sibcall_abi_admits` in each backend).
#[derive(Clone, Copy)]
pub(crate) struct SiblingAbi {
    /// Integer argument registers (x86-64: rdi..r9; AArch64: x0-x7;
    /// RISC-V: a0-a7).
    max_gp: u32,
    /// FP argument registers (xmm0-7, v0-v7, fa0-fa7).
    max_fp: u32,
    /// GP registers a 128-bit integer argument takes, or `None` when such
    /// an argument is refused (AAPCS64 and the RISC-V psABI align the pair
    /// to an even register, which a plain count does not model).
    wide_int_gp: Option<u32>,
    /// The backend refuses every sibling call of a variadic function or of
    /// a function returning through a hidden buffer, and every callee
    /// returning a `_Float128` carrier (`ret_is_f128_sse`): its return
    /// sequence does work after the call.
    strict_frame: bool,
}

impl SiblingAbi {
    /// The budget of `target`, or `None` when the backend has no sibling
    /// calls (i686).
    pub(crate) fn for_target(target: crate::backend::Target) -> Option<Self> {
        use crate::backend::Target;
        match target {
            Target::X86_64 => Some(Self {
                max_gp: 6,
                max_fp: 8,
                wide_int_gp: Some(2),
                strict_frame: false,
            }),
            Target::Aarch64 | Target::Riscv64 => Some(Self {
                max_gp: 8,
                max_fp: 8,
                wide_int_gp: None,
                strict_frame: true,
            }),
            Target::I686 => None,
        }
    }
}

/// True when every argument of `info` is a scalar the target's call
/// sequence passes in a register (see [`SiblingAbi`]). Aggregates, `long
/// double` / `_Float128` and decimal floats are refused outright -- the
/// backend's classifier decides those, and a refusal here only keeps the
/// existing code.
fn register_only_args(info: &CallInfo, abi: SiblingAbi) -> bool {
    if info.struct_arg_sizes.iter().any(Option::is_some)
        || info.struct_arg_is_f128_sse.iter().any(|&f| f)
        || info.struct_arg_classes.iter().any(|c| !c.is_empty())
    {
        return false;
    }
    let (mut gpr, mut fpr) = (0u32, 0u32);
    for ty in &info.arg_types {
        match ty {
            IrType::I8
            | IrType::I16
            | IrType::I32
            | IrType::I64
            | IrType::U8
            | IrType::U16
            | IrType::U32
            | IrType::U64
            | IrType::Ptr => gpr += 1,
            IrType::I128 | IrType::U128 => match abi.wide_int_gp {
                Some(n) => gpr += n,
                None => return false,
            },
            IrType::F32 | IrType::F64 => fpr += 1,
            _ => return false,
        }
    }
    gpr <= abi.max_gp && fpr <= abi.max_fp && info.args.len() == info.arg_types.len()
}

/// The backend's per-site refusals that the IR can see (see the module
/// comment); `None` when the call at the end of a block cannot become a
/// sibling call.
fn tail_call_info(inst: &Instruction, abi: SiblingAbi) -> Option<&CallInfo> {
    if crate::backend::liveness::is_returns_twice_call(inst) {
        return None;
    }
    let info = match inst {
        Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => info,
        _ => return None,
    };
    if info.calling_convention != crate::common::types::CallingConvention::Default
        || info.is_variadic
        || info.is_sret
        || info.is_fastcall
        || info.regparm.is_some()
        || (abi.strict_frame && info.ret_is_f128_sse)
    {
        return None;
    }
    register_only_args(info, abi).then_some(info)
}

/// Whether `term` can transfer control to `label`.
fn targets(term: &Terminator, label: BlockId) -> bool {
    match term {
        Terminator::Branch(t) => *t == label,
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => *true_label == label || *false_label == label,
        Terminator::IndirectBranch {
            possible_targets, ..
        } => possible_targets.contains(&label),
        Terminator::Switch { cases, default, .. } => {
            *default == label || cases.iter().any(|(_, t)| *t == label)
        }
        Terminator::Return(_) | Terminator::Unreachable => false,
    }
}

fn use_counts(func: &IrFunction) -> FxHashMap<u32, u32> {
    let mut counts: FxHashMap<u32, u32> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            for id in inst.used_values() {
                *counts.entry(id).or_insert(0) += 1;
            }
        }
        for id in block.terminator.used_values() {
            *counts.entry(id).or_insert(0) += 1;
        }
    }
    counts
}

/// The function-level guard of `generation.rs` (`sibcall_fn_ok`): a live
/// local alloca's address could reach the tail callee, and dynamic allocas
/// and inline asm have unbounded stack expectations. The backend exempts
/// incoming by-value parameter homes; that fact is backend state, so this
/// pass refuses them too (a miss, never a wrong rewrite).
fn function_admits_sibcalls(func: &IrFunction, counts: &FxHashMap<u32, u32>) -> bool {
    !func.blocks.iter().any(|b| {
        b.instructions.iter().any(|inst| match inst {
            Instruction::Alloca { dest, .. } => counts.get(&dest.0).copied().unwrap_or(0) > 0,
            Instruction::DynAlloca { .. } | Instruction::InlineAsm { .. } => true,
            _ => false,
        })
    })
}

/// The `Return` a predecessor ending in `info`'s call takes over from the
/// shared return block, or `None` when the shape does not match.
fn duplicated_return(
    ret: &Option<Operand>,
    phis: &[Instruction],
    pred: BlockId,
    info: &CallInfo,
    counts: &FxHashMap<u32, u32>,
) -> Option<Terminator> {
    let uses = |id: u32| counts.get(&id).copied().unwrap_or(0);
    match ret {
        None => match info.dest {
            Some(d) if uses(d.0) != 0 => None,
            _ => Some(Terminator::Return(None)),
        },
        Some(Operand::Value(w)) => {
            let d = info.dest?;
            let incoming = phis.iter().find_map(|inst| match inst {
                Instruction::Phi { dest, incoming, .. } if dest == w => Some(incoming),
                _ => None,
            })?;
            let mut from_pred = incoming.iter().filter(|(_, label)| *label == pred);
            let (op, _) = from_pred.next()?;
            let single_entry = from_pred.next().is_none();
            let returns_call = matches!(op, Operand::Value(v) if *v == d);
            (single_entry && returns_call && uses(d.0) == 1)
                .then(|| Terminator::Return(Some(Operand::Value(d))))
        }
        Some(_) => None,
    }
}

/// Run the rewrite on one function for a target with the sibling-call
/// budget `abi`; returns the number of predecessors that took over a return.
pub(crate) fn run(func: &mut IrFunction, abi: SiblingAbi) -> usize {
    if func.is_declaration
        || func.blocks.len() < 2
        || (abi.strict_frame && (func.is_variadic || func.uses_sret))
    {
        return 0;
    }
    let counts = use_counts(func);
    if !function_admits_sibcalls(func, &counts) {
        return 0;
    }
    let entry = func.blocks[0].label;
    // A block whose address escapes (`&&label`) stays even with no
    // predecessor edge left: a computed goto may still reach it.
    let address_taken: FxHashSet<BlockId> = func
        .blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .filter_map(|inst| match inst {
            Instruction::LabelAddr { label, .. } => Some(*label),
            _ => None,
        })
        .collect();
    let mut rewritten = 0usize;
    let mut emptied: FxHashSet<BlockId> = FxHashSet::default();
    for r in 0..func.blocks.len() {
        let r_label = func.blocks[r].label;
        let Terminator::Return(ret) = &func.blocks[r].terminator else {
            continue;
        };
        if !func.blocks[r]
            .instructions
            .iter()
            .all(|inst| matches!(inst, Instruction::Phi { .. }))
        {
            continue;
        }
        let ret = ret.clone();
        let return_row = func
            .terminator_spans
            .span_of(r_label, &func.blocks[r].terminator);
        let mut moved: Vec<(usize, Terminator)> = Vec::new();
        for (p, block) in func.blocks.iter().enumerate() {
            if p == r || !matches!(block.terminator, Terminator::Branch(t) if t == r_label) {
                continue;
            }
            let Some(info) = block
                .instructions
                .last()
                .and_then(|inst| tail_call_info(inst, abi))
            else {
                continue;
            };
            if let Some(term) = duplicated_return(
                &ret,
                &func.blocks[r].instructions,
                block.label,
                info,
                &counts,
            ) {
                moved.push((p, term));
            }
        }
        if moved.is_empty() {
            continue;
        }
        let moved_labels: FxHashSet<BlockId> =
            moved.iter().map(|&(p, _)| func.blocks[p].label).collect();
        for (p, term) in moved {
            let label = func.blocks[p].label;
            if let Some(span) = return_row {
                func.terminator_spans.record(label, span, &term);
            }
            func.blocks[p].terminator = term;
            rewritten += 1;
        }
        for inst in &mut func.blocks[r].instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                incoming.retain(|(_, label)| !moved_labels.contains(label));
            }
        }
        let remaining_preds = func
            .blocks
            .iter()
            .filter(|b| targets(&b.terminator, r_label))
            .count();
        if remaining_preds == 0 && r_label != entry && !address_taken.contains(&r_label) {
            emptied.insert(r_label);
        } else {
            for inst in &mut func.blocks[r].instructions {
                if let Instruction::Phi { dest, incoming, .. } = inst {
                    if let [(src, _)] = incoming.as_slice() {
                        *inst = Instruction::Copy {
                            dest: *dest,
                            src: src.clone(),
                        };
                    }
                }
            }
        }
    }
    if !emptied.is_empty() {
        func.blocks.retain(|b| !emptied.contains(&b.label));
    }
    rewritten
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Target;
    use crate::ir::reexports::{BasicBlock, IrConst, Value};

    fn call(dest: u32, callee: &str, arg_types: &[IrType], ret: IrType) -> Instruction {
        Instruction::Call {
            func: callee.into(),
            info: CallInfo {
                dest: (ret != IrType::Void).then_some(Value(dest)),
                args: (0..arg_types.len())
                    .map(|i| Operand::Const(IrConst::I64(i as i64)))
                    .collect(),
                arg_types: arg_types.to_vec(),
                return_type: ret,
                num_fixed_args: arg_types.len(),
                ..Default::default()
            },
        }
    }

    /// `int sel(int c) { return c ? g1(..) : g2(..); }`: two call arms
    /// branching to `R = phi; ret`.
    fn dispatcher(arg_types: &[IrType], ret: IrType) -> IrFunction {
        let mut f = IrFunction::new("sel".into(), ret, vec![], false);
        f.next_value_id = 20;
        let block =
            |label: u32, instructions: Vec<Instruction>, terminator: Terminator| BasicBlock {
                label: BlockId(label),
                instructions,
                terminator,
                source_spans: vec![],
            };
        f.blocks.push(block(
            0,
            vec![],
            Terminator::CondBranch {
                cond: Operand::Value(Value(1)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
        ));
        f.blocks.push(block(
            1,
            vec![call(10, "g1", arg_types, ret)],
            Terminator::Branch(BlockId(3)),
        ));
        f.blocks.push(block(
            2,
            vec![call(11, "g2", arg_types, ret)],
            Terminator::Branch(BlockId(3)),
        ));
        let (instructions, terminator) = if ret == IrType::Void {
            (vec![], Terminator::Return(None))
        } else {
            (
                vec![Instruction::Phi {
                    dest: Value(12),
                    ty: ret,
                    incoming: vec![
                        (Operand::Value(Value(10)), BlockId(1)),
                        (Operand::Value(Value(11)), BlockId(2)),
                    ],
                }],
                Terminator::Return(Some(Operand::Value(Value(12)))),
            )
        };
        f.blocks.push(block(3, instructions, terminator));
        f
    }

    fn abi(target: Target) -> SiblingAbi {
        SiblingAbi::for_target(target).expect("a target with sibling calls")
    }

    const SIBLING_TARGETS: [Target; 3] = [Target::X86_64, Target::Aarch64, Target::Riscv64];

    /// Both arms take the return on every target with sibling calls, and
    /// the emptied return block goes; i686 has no budget at all.
    #[test]
    fn dispatcher_arms_take_the_return() {
        for target in SIBLING_TARGETS {
            for ret in [IrType::I32, IrType::Void] {
                let mut f = dispatcher(&[IrType::I32, IrType::Ptr], ret);
                assert_eq!(run(&mut f, abi(target)), 2, "{target:?} {ret:?}");
                assert_eq!(f.blocks.len(), 3);
                let want = |d: u32| {
                    Terminator::Return((ret != IrType::Void).then_some(Operand::Value(Value(d))))
                };
                assert_eq!(
                    format!("{:?}", f.blocks[1].terminator),
                    format!("{:?}", want(10))
                );
                assert_eq!(
                    format!("{:?}", f.blocks[2].terminator),
                    format!("{:?}", want(11))
                );
            }
        }
        assert!(SiblingAbi::for_target(Target::I686).is_none());
    }

    /// The register budgets: seven integer arguments fit AArch64 and RISC-V
    /// (x0-x7, a0-a7) but not x86-64 (six), nine fit none; a 128-bit
    /// integer argument is two x86-64 registers and refused elsewhere (an
    /// even-aligned pair); nine doubles fit none.
    #[test]
    fn register_budgets_follow_each_abi() {
        let cases: [(Vec<IrType>, [bool; 3]); 4] = [
            (vec![IrType::I64; 7], [false, true, true]),
            (vec![IrType::I64; 9], [false, false, false]),
            (vec![IrType::I128, IrType::I32], [true, false, false]),
            (vec![IrType::F64; 9], [false, false, false]),
        ];
        for (args, admitted) in cases {
            for (target, admit) in SIBLING_TARGETS.into_iter().zip(admitted) {
                let mut f = dispatcher(&args, IrType::I64);
                let expect = if admit { 2 } else { 0 };
                assert_eq!(run(&mut f, abi(target)), expect, "{target:?} {args:?}");
            }
        }
    }

    /// AArch64 and RISC-V refuse every sibling call of a variadic function
    /// or of one returning through a hidden buffer, and a callee returning a
    /// `_Float128` carrier; x86-64 keeps its own rules for all three.
    #[test]
    fn strict_frames_refuse_what_the_backends_refuse() {
        for target in SIBLING_TARGETS {
            let strict = target != Target::X86_64;
            let mut variadic = dispatcher(&[IrType::I32], IrType::I32);
            variadic.is_variadic = true;
            let mut sret = dispatcher(&[IrType::I32], IrType::I32);
            sret.uses_sret = true;
            let mut f128 = dispatcher(&[IrType::I32], IrType::I32);
            for block in &mut f128.blocks {
                for inst in &mut block.instructions {
                    if let Instruction::Call { info, .. } = inst {
                        info.ret_is_f128_sse = true;
                    }
                }
            }
            for (name, mut f) in [("variadic", variadic), ("sret", sret), ("f128", f128)] {
                let expect = if strict { 0 } else { 2 };
                assert_eq!(run(&mut f, abi(target)), expect, "{target:?} {name}");
            }
        }
    }

    /// A call result read anywhere but the phi, a live result under a void
    /// return, and an arm that is not a call keep their branches; the
    /// other arm still converts, and the return block's last phi becomes a
    /// copy.
    #[test]
    fn shapes_that_keep_their_branch() {
        let mut used_twice = dispatcher(&[IrType::I32], IrType::I32);
        used_twice.blocks[0].instructions.push(Instruction::Copy {
            dest: Value(13),
            src: Operand::Value(Value(10)),
        });
        assert_eq!(run(&mut used_twice, abi(Target::Aarch64)), 1);
        assert!(matches!(
            used_twice.blocks[1].terminator,
            Terminator::Branch(_)
        ));
        let r = used_twice
            .blocks
            .iter()
            .find(|b| b.label == BlockId(3))
            .expect("kept R");
        assert!(matches!(r.instructions[0], Instruction::Copy { .. }));

        let mut live_void = dispatcher(&[IrType::I32], IrType::Void);
        if let Instruction::Call { info, .. } = &mut live_void.blocks[1].instructions[0] {
            info.dest = Some(Value(10));
        }
        live_void.blocks[0].instructions.push(Instruction::Copy {
            dest: Value(13),
            src: Operand::Value(Value(10)),
        });
        assert_eq!(run(&mut live_void, abi(Target::Riscv64)), 1);
        assert!(matches!(
            live_void.blocks[1].terminator,
            Terminator::Branch(_)
        ));
    }
}
