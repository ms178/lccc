//! Dead Code Elimination (DCE) pass.
//!
//! Removes instructions that are not backward-reachable from any *root*:
//!
//! * instructions with side effects (stores, calls, atomics, …)
//! * values consumed by a terminator
//! * dest-less instructions (conservatively kept; see `is_dce_root`)
//!
//! This is classical mark-and-sweep DCE — *not* reference counting.
//!
//! # Why mark-and-sweep (and not use-counts)
//!
//! A use-count / refcount worklist cannot collect *cycles*. After SSA
//! construction, inlining, mem2reg and loop opts the IR is full of them:
//!
//! ```text
//!     %p = phi [0, entry], [%a, latch]
//!     %a = add %p, 1          ; unused induction variable
//! ```
//!
//! `%p` and `%a` each have use-count 1, so a refcount DCE never seeds them
//! and never deletes them. The leftover phis lower to copies that (a) burn
//! registers and (b) have caused real miscompiles: they clobbered inline-asm
//! output-pointer stack slots and crashed the kernel in
//! `init_scattered_cpuid_features`. A phi-self-reference exclusion only
//! patches the trivial `phi V: [0, V]` shape; the IV cycle above and mutual
//! phi SCCs stay uncollected.
//!
//! Mark-and-sweep starts from semantically live roots and walks the use-def
//! graph. A cycle with no path from a root is dead, regardless of internal
//! self- or mutual references. Complexity stays O(n): each instruction is
//! marked at most once and visited at most once.
//!
//! # THIS PASS ALSO RUNS ON NON-SSA IR — multi-def values are real
//!
//! The driver invokes DCE *after* `eliminate_phis` (post-phi cleanup).
//! Phi elimination lowers `%d = phi …` to `Copy { dest: %d, … }` in EVERY
//! predecessor — the same dest id is defined at several (block, inst)
//! sites. A single-slot def map would keep only the last-seen site, mark
//! only that copy live, and sweep the others: the phi value would be
//! garbage on every other inbound path. `def_loc` therefore records the
//! unique def site for single-def values and diverts multi-def ids to an
//! overflow table whose sites are marked live TOGETHER. (A refcount DCE
//! fails multi-def only conservatively; naive mark-and-sweep fails it
//! UNSAFELY. Both directions are covered by tests below.)
//!
//! # Seeding order matters
//!
//! Terminator uses are seeded in a SECOND full pass, after `def_loc` is
//! complete. Block indices are not dominance-ordered after CFG transforms
//! (merges, splits, relayout): a terminator may consume a value whose
//! defining block sits at a HIGHER index. Interleaving the def scan with
//! terminator seeding would look that id up while its slot still reads
//! NO_DEF and silently drop the use — deleting a live value.
//!
//! # Correctness constraints (do not "simplify" these away)
//!
//! * Ordinary `Alloca` is pure only for a proven single-block leaf whose
//!   ParamRefs execute in the leading prefix. Other functions retain positional
//!   parameter homes because late ParamRefs need the saved ABI value.
//! * `DynAlloca` / `StackRestore` adjust the runtime stack pointer.
//! * Every `Call` / `CallIndirect` is a root until we have trustworthy
//!   `pure`/`const` attributes. Intrinsics already carry purity and *are*
//!   deleted when unused.
//! * This pass does **not** delete unreachable blocks. Run unreachable-
//!   code elimination first so side-effecting insts in dead blocks do not
//!   pin otherwise-dead values.
//!
//! # What this is not
//!
//! Aggressive DCE (control-dependence / post-dom frontier), bit-tracking
//! DCE, and dead-*store* elimination are separate passes. They need CFG
//! analyses this file must not quietly reimplement.

use crate::common::fx_hash::FxHashMap;
use crate::ir::reexports::{CallInfo, Instruction, IrFunction};

/// Sentinel stored in `def_loc` for values that have no removable definition
/// in this function (parameters, globals, malformed IDs).
const NO_DEF: u32 = u32::MAX;

/// Sentinel block index marking an id as multi-def; its sites live in the
/// overflow table instead of the packed word.
const MULTI_DEF: u32 = u32::MAX - 1;

/// Eliminate dead instructions in `func`.
///
/// Returns the number of instructions removed. The pass manager uses a
/// non-zero return as a "something changed" signal to iterate cooperating
/// passes (DCE → simplifycfg → DCE, …).
pub(crate) fn eliminate_dead_code(func: &mut IrFunction) -> usize {
    if func.blocks.is_empty() {
        return 0;
    }
    let mut n_insts = 0usize;
    for block in &func.blocks {
        n_insts += block.instructions.len();
    }
    if n_insts == 0 {
        return 0;
    }

    // `max_value_id` can be sparse after earlier deletions. saturating_add
    // keeps the allocation well-defined even if the id space is exhausted
    // (impossible in practice; required for fuzz robustness).
    let def_len = (func.max_value_id() as usize).saturating_add(1);

    // def_loc[v] = packed (block_idx << 32 | inst_idx) of the def when it is
    // unique. Parameters / undeclared ids stay at NO_DEF; ids defined at
    // several sites (post-phi copies) are diverted to `multi_defs`.
    let mut def_loc: Vec<u64> = vec![pack(NO_DEF, NO_DEF); def_len];
    let mut multi_defs: FxHashMap<u32, Vec<(u32, u32)>> = FxHashMap::default();

    // Per-instruction liveness flags. NOTE (honesty): Rust's `Vec<bool>` is
    // one byte per element — it is NOT bit-packed like C++'s vector<bool>,
    // so bool vs u8 is a wash here. `u8` is kept only so the flag can never
    // be mistaken for a semantic boolean the optimizer may re-pack.
    let mut live: Vec<Vec<u8>> = func
        .blocks
        .iter()
        .map(|b| vec![0u8; b.instructions.len()])
        .collect();

    // Worst case every instruction is a root (pure side-effect soup).
    let mut worklist: Vec<(u32, u32)> = Vec::with_capacity(n_insts);

    // Dead parameter homes are safe only in a single-block leaf whose every
    // ParamRef executes in the leading declaration/parameter prefix. Then no
    // call or earlier generated instruction can clobber an incoming ABI
    // register before ParamRef consumes it.
    let allow_dead_allocas = func.blocks.len() == 1 && {
        let mut seen_code = false;
        func.blocks[0].instructions.iter().all(|inst| match inst {
            Instruction::Alloca { .. } if !seen_code => true,
            Instruction::ParamRef { .. } if !seen_code => true,
            Instruction::ParamRef { .. } => false,
            Instruction::Call { .. }
            | Instruction::CallIndirect { .. }
            | Instruction::InlineAsm { .. } => false,
            _ => {
                seen_code = true;
                true
            }
        })
    };

    // ------------------------------------------------------------------
    // Pass 1: record every def site and seed instruction roots.
    // Terminator uses are deliberately NOT seeded here (see module doc:
    // def_loc must be complete before any lookup).
    // ------------------------------------------------------------------
    for (bi, block) in func.blocks.iter().enumerate() {
        let bi_u = bi as u32;
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Some(dest) = inst.dest() {
                let id = dest.0 as usize;
                if id < def_len {
                    let (prev_b, prev_i) = unpack(def_loc[id]);
                    if prev_b == NO_DEF {
                        def_loc[id] = pack(bi_u, ii as u32);
                    } else if prev_b == MULTI_DEF {
                        multi_defs
                            .get_mut(&dest.0)
                            .expect("MULTI_DEF marker without overflow entry")
                            .push((bi_u, ii as u32));
                    } else {
                        // Second def of the same id: post-phi copies. Divert
                        // BOTH sites to the overflow table; liveness of the
                        // value must pin every one of its defs.
                        multi_defs.insert(dest.0, vec![(prev_b, prev_i), (bi_u, ii as u32)]);
                        def_loc[id] = pack(MULTI_DEF, 0);
                    }
                }
            }
            if is_dce_root(inst, !allow_dead_allocas) {
                live[bi][ii] = 1;
                worklist.push((bi_u, ii as u32));
            }
        }
    }

    // ------------------------------------------------------------------
    // Pass 2: seed terminator uses. A value that feeds a branch condition
    // or a return is live even if no instruction reads it. def_loc is now
    // complete, so later-indexed defining blocks resolve correctly.
    // ------------------------------------------------------------------
    for block in &func.blocks {
        block.terminator.for_each_used_value(|id| {
            mark_def_live(id, &def_loc, &multi_defs, &mut live, &mut worklist);
        });
    }

    // ------------------------------------------------------------------
    // Propagate liveness along the use-def graph.
    //
    // A phi is live iff some *already-live* user (instruction or
    // terminator) reads it. Once live, *every* incoming operand becomes
    // live — including self- and cross-references. That is what makes
    // unused induction-variable cycles vanish: they are never seeded.
    // The already-live check in mark_def_live is what terminates walks
    // around *live* phi cycles.
    // ------------------------------------------------------------------
    while let Some((bi, ii)) = worklist.pop() {
        let inst = &func.blocks[bi as usize].instructions[ii as usize];
        inst.for_each_used_value(|id| {
            mark_def_live(id, &def_loc, &multi_defs, &mut live, &mut worklist);
        });
    }

    // Parameter homes are still identified by nth-Alloca position in every
    // backend. If parameter k's home survives, every earlier parameter-home
    // Alloca must survive too or k silently shifts to the wrong ABI argument.
    if allow_dead_allocas {
        let param_allocas: Vec<usize> = func.blocks[0]
            .instructions
            .iter()
            .enumerate()
            .filter_map(|(ii, inst)| matches!(inst, Instruction::Alloca { .. }).then_some(ii))
            .take(func.params.len())
            .collect();
        if let Some(last_live) = param_allocas.iter().rposition(|&ii| live[0][ii] != 0) {
            for &ii in &param_allocas[..=last_live] {
                mark_site_live(0, ii as u32, &mut live, &mut worklist);
            }
        }
    }

    if dce_debug_enabled() {
        dump_dead_instructions(func, &live);
    }

    // ------------------------------------------------------------------
    // Sweep. Single pass, order of surviving instructions preserved.
    // ------------------------------------------------------------------
    let mut total = 0usize;
    for (bi, block) in func.blocks.iter_mut().enumerate() {
        total += sweep_block(&mut block.instructions, &mut block.source_spans, &live[bi]);
    }
    total
}

/// Pack a (`block`, `inst`) pair into one word. Layout-wise `Vec<u64>` and
/// `Vec<(u32, u32)>` are identical (8 bytes/elem); the packed form is kept
/// because a single sentinel compare (`== NO_DEF`) reads one lane instead
/// of two fields, and it makes accidental partial updates impossible.
#[inline]
fn pack(block: u32, inst: u32) -> u64 {
    ((block as u64) << 32) | inst as u64
}

#[inline]
fn unpack(word: u64) -> (u32, u32) {
    ((word >> 32) as u32, word as u32)
}

/// Mark the defining instruction(s) of `id` live and enqueue them.
///
/// No-ops for parameters (no def in this function), out-of-range ids and
/// already-live defs. Multi-def ids (post-phi copies) mark ALL their def
/// sites — keeping only one would leave the value undefined on the other
/// inbound paths.
#[inline]
fn mark_def_live(
    id: u32,
    def_loc: &[u64],
    multi_defs: &FxHashMap<u32, Vec<(u32, u32)>>,
    live: &mut [Vec<u8>],
    worklist: &mut Vec<(u32, u32)>,
) {
    let idx = id as usize;
    if idx >= def_loc.len() {
        return;
    }
    let (dbi, dii) = unpack(def_loc[idx]);
    if dbi == NO_DEF {
        return;
    }
    if dbi == MULTI_DEF {
        if let Some(sites) = multi_defs.get(&id) {
            for &(b, i) in sites {
                mark_site_live(b, i, live, worklist);
            }
        }
        return;
    }
    mark_site_live(dbi, dii, live, worklist);
}

#[inline]
fn mark_site_live(dbi: u32, dii: u32, live: &mut [Vec<u8>], worklist: &mut Vec<(u32, u32)>) {
    let b = dbi as usize;
    let i = dii as usize;
    if b >= live.len() || i >= live[b].len() {
        return;
    }
    if live[b][i] != 0 {
        return;
    }
    live[b][i] = 1;
    worklist.push((dbi, dii));
}

/// Roots are never deleted, even if their result (if any) is unread.
///
/// Dest-less non-side-effecting instructions are treated as roots so that
/// an unknown future IR opcode without a destination cannot be silently
/// discarded. True no-ops should be given a dest or a dedicated fold.
#[inline]
fn is_dce_root(inst: &Instruction, preserve_allocas: bool) -> bool {
    (preserve_allocas && matches!(inst, Instruction::Alloca { .. }))
        || has_side_effects(inst)
        || inst.dest().is_none()
}

/// Compact `instructions` (and `source_spans`, when they are a 1:1 map)
/// so that every slot with `live[i] == 0` disappears.
///
/// Returns the number of deleted instructions.
fn sweep_block<S>(
    instructions: &mut Vec<Instruction>,
    source_spans: &mut Vec<S>,
    live: &[u8],
) -> usize {
    let original_len = instructions.len();
    if original_len == 0 {
        return 0;
    }
    // A length mismatch here is a compiler bug, not malformed input.
    // Fail fast in debug builds; in release, refuse to sweep (leaving dead
    // code is safe, dropping a live instruction is not).
    debug_assert!(
        live.len() == original_len,
        "DCE live-flag vector desynchronized from instruction list"
    );
    if live.len() != original_len {
        return 0;
    }

    let dead_count = live.iter().filter(|&&l| l == 0).count();
    if dead_count == 0 {
        return 0;
    }

    let has_spans = source_spans.len() == original_len && !source_spans.is_empty();
    if has_spans {
        let mut i = 0usize;
        instructions.retain(|_| {
            let keep = live[i] != 0;
            i += 1;
            keep
        });
        let mut i = 0usize;
        source_spans.retain(|_| {
            let keep = live[i] != 0;
            i += 1;
            keep
        });
    } else {
        // Restore the documented invariant: empty spans == "no debug info
        // for this block". A stale non-empty, wrong-length vector would
        // desynchronize later debug emission.
        if !source_spans.is_empty() && source_spans.len() != original_len {
            source_spans.clear();
        }
        let mut i = 0usize;
        instructions.retain(|_| {
            let keep = live[i] != 0;
            i += 1;
            keep
        });
    }
    dead_count
}

/// `CCC_DEBUG_DCE` is a developer dump, not a user-facing flag. Reading
/// the environment on every function is a self-host compile-time tax
/// (lock + hash lookup + OsString alloc × every function × every DCE
/// invocation). Cache it for the process lifetime.
fn dce_debug_enabled() -> bool {
    static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("CCC_DEBUG_DCE").is_some())
}

fn dump_dead_instructions(func: &IrFunction, live: &[Vec<u8>]) {
    for (bi, block) in func.blocks.iter().enumerate() {
        if bi >= live.len() {
            break;
        }
        for (ii, inst) in block.instructions.iter().enumerate() {
            if ii < live[bi].len() && live[bi][ii] == 0 {
                if let Some(d) = inst.dest() {
                    eprintln!("[DCE] Removing Value({}) in block {}: {:?}", d.0, bi, inst);
                } else {
                    eprintln!("[DCE] Removing dest-less inst in block {}: {:?}", bi, inst);
                }
            }
        }
    }
}

/// Side-effecting instructions are DCE roots.
///
/// New `Instruction` variants default to *pure* under this `matches!`.
/// That is the dangerous direction: adding a side-effecting opcode and
/// forgetting this list is a miscompile. Extend the match in the same
/// commit that introduces the opcode. (Dest-less opcodes get a second
/// safety net via `is_dce_root`, which pins anything without a dest —
/// that is why the historically missing `PgoCounterInc` never caused a
/// deletion; it is now listed anyway so the predicate is truthful.)
fn has_side_effects(inst: &Instruction) -> bool {
    // Calls first: a pure/const call is droppable when its result is unused —
    // EXCEPT for sret returns. The observable result of an sret call is the
    // memory write through the hidden sret pointer, not the (always unused)
    // dest value. On i686 every struct/_Complex return is sret, so dropping
    // a pure struct-returning call left the result buffer uninitialized
    // (gcc.c-torture 20070614-1: `pure _Complex double` callee).
    if let Instruction::Call {
        info: CallInfo { is_sret, is_pure, is_const, .. },
        ..
    }
    | Instruction::CallIndirect {
        info: CallInfo { is_sret, is_pure, is_const, .. },
        ..
    } = inst
    {
        return (!is_pure && !is_const) || *is_sret;
    }
    // A volatile load is an observable side effect (C11 5.1.2.3) even when
    // its result is unused: it must never be dead-code eliminated.
    matches!(
        inst,
        Instruction::Load { volatile: true, .. } |
        // Ordinary Alloca is conditionally rooted by eliminate_dead_code.
        // DynAlloca modifies the stack pointer at runtime.
        Instruction::DynAlloca { .. } |
        Instruction::Store { .. } |
        Instruction::Memcpy { .. } |
        Instruction::VaStart { .. } |
        Instruction::VaEnd { .. } |
        Instruction::VaCopy { .. } |
        Instruction::VaArg { .. } |
        Instruction::VaArgStruct { .. } |
        Instruction::AtomicRmw { .. } |
        Instruction::AtomicInc { .. } |
        Instruction::AtomicCmpxchg { .. } |
        Instruction::AtomicLoad { .. } |
        Instruction::AtomicStore { .. } |
        Instruction::Fence { .. } |
        // Profile counters update memory; deleting one silently corrupts
        // PGO data even though the instruction has no dest.
        Instruction::PgoCounterInc { .. } |
        // GetReturn* reads the second half of a wide ABI return. They are
        // pinned because they are ordered relative to the producing call
        // (a later call would clobber the hardwired register). Unused
        // results still have to occupy that program point.
        Instruction::GetReturnF64Second { .. } |
        Instruction::GetReturnF32Second { .. } |
        Instruction::GetReturnF128Second { .. } |
        Instruction::SetReturnF64Second { .. } |
        Instruction::SetReturnF32Second { .. } |
        Instruction::SetReturnF128Second { .. } |
        Instruction::InlineAsm { .. } |
        // StackRestore modifies the stack pointer at runtime. StackSave is
        // kept alive by its use in StackRestore (normal DCE liveness) and
        // is *not* a root of its own — an unused StackSave is a dead read
        // of RSP and should disappear.
        Instruction::StackRestore { .. }
    ) || matches!(
        inst,
        Instruction::Intrinsic { op, dest_ptr, .. } if !op.is_pure() || dest_ptr.is_some()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::{AddressSpace, IrType};
    use crate::ir::reexports::{
        BasicBlock, BlockId, CallInfo, IrBinOp, IrConst, IrParam, Operand, Terminator, Value,
    };

    fn make_simple_func() -> IrFunction {
        // Function with: %0 = alloca i32, %1 = add 3, 4 (dead), store 42 to %0, load from %0
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Alloca {
                    dest: Value(0),
                    ty: IrType::I32,
                    size: 4,
                    align: 0,
                    volatile: false,
                    semantic_volatile: false,
                },
                // Dead instruction: result %1 is never used
                Instruction::BinOp {
                    dest: Value(1),
                    op: IrBinOp::Add,
                    lhs: Operand::Const(IrConst::I32(3)),
                    rhs: Operand::Const(IrConst::I32(4)),
                    ty: IrType::I32,
                },
                Instruction::Store {
                    volatile: false,
                    val: Operand::Const(IrConst::I32(42)),
                    ptr: Value(0),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
                Instruction::Load {
                    volatile: false,
                    dest: Value(2),
                    ptr: Value(0),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
            source_spans: Vec::new(),
        });
        func
    }

    #[test]
    fn test_eliminate_dead_binop() {
        let mut func = make_simple_func();
        let removed = eliminate_dead_code(&mut func);
        assert_eq!(removed, 1);
        assert_eq!(func.blocks[0].instructions.len(), 3); // alloca, store, load
    }

    #[test]
    fn test_side_effects_preserved() {
        // Calls should never be removed even if result is unused
        let mut func = IrFunction::new("test".to_string(), IrType::Void, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Call {
                func: "printf".to_string(),
                info: CallInfo {
                    dest: Some(Value(0)),
                    ..CallInfo::default()
                },
            }],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let removed = eliminate_dead_code(&mut func);
        assert_eq!(removed, 0);
    }

    #[test]
    fn test_transitive_dead_chain() {
        // %1 -> %2 -> %3, none used: all three go in one pass.
        let mut func = IrFunction::new("test".to_string(), IrType::Void, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Alloca {
                    dest: Value(0),
                    ty: IrType::I32,
                    size: 4,
                    align: 0,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::BinOp {
                    dest: Value(1),
                    op: IrBinOp::Add,
                    lhs: Operand::Const(IrConst::I32(1)),
                    rhs: Operand::Const(IrConst::I32(2)),
                    ty: IrType::I32,
                },
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(3)),
                    ty: IrType::I32,
                },
                Instruction::BinOp {
                    dest: Value(3),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(2)),
                    rhs: Operand::Const(IrConst::I32(4)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });

        let removed = eliminate_dead_code(&mut func);
        assert_eq!(removed, 4);
        assert!(func.blocks[0].instructions.is_empty())
    }

    #[test]
    fn live_later_parameter_home_pins_positional_prefix() {
        let param = || IrParam {
            ty: IrType::Ptr,
            noalias: false,
            struct_size: None,
            struct_align: None,
            struct_eightbyte_classes: vec![],
            is_f128_sse: false,
            riscv_float_class: None,
        };
        let mut func = IrFunction::new(
            "param_prefix".to_string(),
            IrType::Void,
            vec![param(), param()],
            false,
        );
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Alloca {
                    dest: Value(0),
                    ty: IrType::Ptr,
                    size: 8,
                    align: 8,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::Alloca {
                    dest: Value(1),
                    ty: IrType::Ptr,
                    size: 8,
                    align: 8,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::Store {
                    volatile: false,
                    val: Operand::Const(IrConst::I64(7)),
                    ptr: Value(1),
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: vec![],
        });
        assert_eq!(eliminate_dead_code(&mut func), 0);
        assert!(matches!(
            func.blocks[0].instructions[0],
            Instruction::Alloca { dest: Value(0), .. }
        ));
        assert!(matches!(
            func.blocks[0].instructions[1],
            Instruction::Alloca { dest: Value(1), .. }
        ));
    }

    #[test]
    fn test_self_referencing_phi_removed() {
        // phi V: [0, V] with no external user. Mark-and-sweep needs no
        // special case: V is never seeded, so it is swept.
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![Instruction::Phi {
                dest: Value(0),
                ty: IrType::I64,
                incoming: vec![
                    (Operand::Const(IrConst::I64(0)), BlockId(0)),
                    (Operand::Value(Value(0)), BlockId(2)),
                ],
            }],
            terminator: Terminator::CondBranch {
                cond: Operand::Const(IrConst::I32(1)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            source_spans: Vec::new(),
        });

        let removed = eliminate_dead_code(&mut func);
        assert_eq!(removed, 1, "Self-referencing dead phi should be removed");
        assert!(func.blocks[1].instructions.is_empty());
    }

    #[test]
    fn test_dead_induction_variable_cycle() {
        // The motivating case for mark-and-sweep:
        //   header:  %p = phi [0, entry], [%a, latch]
        //   latch:   %a = add %p, 1
        // No external user. Refcount DCE keeps both (use_count 1 each);
        // mark-and-sweep must delete both.
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![Instruction::Phi {
                dest: Value(0),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Const(IrConst::I32(0)), BlockId(0)),
                    (Operand::Value(Value(1)), BlockId(2)),
                ],
            }],
            terminator: Terminator::Branch(BlockId(2)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![Instruction::BinOp {
                dest: Value(1),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(0)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            }],
            terminator: Terminator::CondBranch {
                cond: Operand::Const(IrConst::I32(1)),
                true_label: BlockId(1),
                false_label: BlockId(3),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            source_spans: Vec::new(),
        });

        let removed = eliminate_dead_code(&mut func);
        assert_eq!(removed, 2, "Dead IV phi+add cycle must be removed");
        assert!(func.blocks[1].instructions.is_empty(), "dead phi survived");
        assert!(func.blocks[2].instructions.is_empty(), "dead add survived");
    }

    #[test]
    fn test_live_induction_variable_kept() {
        // Same shape, but the header branches on %p and the exit returns %p:
        // both phi and add are live. Guards against "delete all phis".
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![Instruction::Phi {
                dest: Value(0),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Const(IrConst::I32(0)), BlockId(0)),
                    (Operand::Value(Value(1)), BlockId(2)),
                ],
            }],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(0)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![Instruction::BinOp {
                dest: Value(1),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(0)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Value(Value(0)))),
            source_spans: Vec::new(),
        });

        let removed = eliminate_dead_code(&mut func);
        assert_eq!(removed, 0, "Live IV must not be touched");
        assert_eq!(func.blocks[1].instructions.len(), 1);
        assert_eq!(func.blocks[2].instructions.len(), 1);
    }

    #[test]
    fn test_mutual_dead_phis() {
        // %0 = phi [0, B0], [%1, B2]; %1 = phi [1, B0], [%0, B1].
        // Neither escapes: the SCC must be collected whole.
        let mut func = IrFunction::new("test".to_string(), IrType::Void, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::CondBranch {
                cond: Operand::Const(IrConst::I32(1)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![Instruction::Phi {
                dest: Value(0),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Const(IrConst::I32(0)), BlockId(0)),
                    (Operand::Value(Value(1)), BlockId(2)),
                ],
            }],
            terminator: Terminator::Branch(BlockId(2)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![Instruction::Phi {
                dest: Value(1),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Const(IrConst::I32(1)), BlockId(0)),
                    (Operand::Value(Value(0)), BlockId(1)),
                ],
            }],
            terminator: Terminator::CondBranch {
                cond: Operand::Const(IrConst::I32(1)),
                true_label: BlockId(1),
                false_label: BlockId(3),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });

        let removed = eliminate_dead_code(&mut func);
        assert_eq!(removed, 2, "Mutually recursive dead phis must both go");
        assert!(func.blocks[1].instructions.is_empty());
        assert!(func.blocks[2].instructions.is_empty());
    }

    #[test]
    fn test_multi_def_post_phi_copies_all_kept() {
        // NON-SSA input, exactly what eliminate_phis produces: the same
        // dest id %5 is written by a Copy in BOTH predecessors, and the
        // join block returns %5. A single-slot def map marks only the
        // last-seen copy live and sweeps the first — the returned value
        // would be garbage on the other path. Both copies must survive.
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::CondBranch {
                cond: Operand::Const(IrConst::I32(1)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![Instruction::Copy {
                dest: Value(5),
                src: Operand::Const(IrConst::I32(10)),
            }],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![Instruction::Copy {
                dest: Value(5),
                src: Operand::Const(IrConst::I32(20)),
            }],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Value(Value(5)))),
            source_spans: Vec::new(),
        });

        let removed = eliminate_dead_code(&mut func);
        assert_eq!(
            removed, 0,
            "both defs of a live multi-def value must be kept"
        );
        assert_eq!(func.blocks[1].instructions.len(), 1, "first copy deleted");
        assert_eq!(func.blocks[2].instructions.len(), 1, "second copy deleted");
    }

    #[test]
    fn test_multi_def_dead_copies_all_removed() {
        // Same multi-def shape but NOTHING reads %5: every copy is dead
        // and all of them must go (the conservative direction).
        let mut func = IrFunction::new("test".to_string(), IrType::Void, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::CondBranch {
                cond: Operand::Const(IrConst::I32(1)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![Instruction::Copy {
                dest: Value(5),
                src: Operand::Const(IrConst::I32(10)),
            }],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![Instruction::Copy {
                dest: Value(5),
                src: Operand::Const(IrConst::I32(20)),
            }],
            terminator: Terminator::Branch(BlockId(3)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });

        let removed = eliminate_dead_code(&mut func);
        assert_eq!(removed, 2, "all defs of a dead multi-def value must go");
    }

    #[test]
    fn test_terminator_use_of_later_block_def() {
        // Block order is NOT dominance order after CFG transforms. Here
        // block 0's terminator branches on %7, which is DEFINED in block 1
        // (a higher index; block 1 is block 0's dominator via the entry
        // arrangement below — the shape is artificial but legal layout-
        // wise). Seeding terminator uses during the def scan would look up
        // %7 while def_loc still reads NO_DEF and delete the live Cmp.
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);
        // Entry is block 1 by convention of this test: block 0 is reached
        // from block 1 and consumes a value defined there.
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(7)), // defined in block index 1!
                true_label: BlockId(2),
                false_label: BlockId(2),
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![Instruction::Cmp {
                dest: Value(7),
                op: crate::ir::reexports::IrCmpOp::Eq,
                lhs: Operand::Const(IrConst::I32(1)),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            }],
            terminator: Terminator::Branch(BlockId(0)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            source_spans: Vec::new(),
        });

        let removed = eliminate_dead_code(&mut func);
        assert_eq!(
            removed, 0,
            "Cmp consumed by an earlier-indexed terminator must stay"
        );
        assert_eq!(func.blocks[1].instructions.len(), 1);
    }

    #[test]
    fn test_pgo_counter_kept() {
        let mut func = IrFunction::new("test".to_string(), IrType::Void, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::PgoCounterInc {
                name: "cnt".to_string(),
                offset: 0,
                atomic: false,
            }],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let removed = eliminate_dead_code(&mut func);
        assert_eq!(removed, 0, "profile counter must never be deleted");
    }

    #[test]
    fn test_empty_function_is_a_noop() {
        let mut func = IrFunction::new("empty".to_string(), IrType::Void, vec![], false);
        let removed = eliminate_dead_code(&mut func);
        assert_eq!(removed, 0);
    }

    #[test]
    fn test_source_spans_stay_aligned() {
        // source_spans must be compacted in lock-step with instructions,
        // otherwise later debug emission indexes the wrong span.
        let mut func = make_simple_func();
        let sp = |n: u32| crate::common::source::Span::new(n, n + 1, 0);
        func.blocks[0].source_spans = vec![sp(10), sp(20), sp(30), sp(40)];
        let removed = eliminate_dead_code(&mut func);
        assert_eq!(removed, 1);
        assert_eq!(func.blocks[0].instructions.len(), 3);
        // The dead BinOp was slot 1: its span (20) must vanish, order kept.
        assert_eq!(func.blocks[0].source_spans, vec![sp(10), sp(30), sp(40)]);
    }

    #[test]
    fn test_idempotent() {
        let mut func = make_simple_func();
        let first = eliminate_dead_code(&mut func);
        let second = eliminate_dead_code(&mut func);
        assert_eq!(first, 1);
        assert_eq!(second, 0);
    }

    #[test]
    fn dead_pure_vector_intrinsics_are_removed() {
        // Regression: an orphaned VecBroadcastF64x4 (vectorizer bailout over
        // a global-array map loop) used to be DCE-rooted as "side-effecting"
        // because IntrinsicOp::is_pure() missed the modern Vec* families. It
        // reached codegen with no register or slot home and ICEd the
        // intrinsic emitter. Pure vector value producers with an unused
        // dest must be swept.
        let mut func = IrFunction::new("vb".to_string(), IrType::Void, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Intrinsic {
                dest: Some(Value(10)),
                op: crate::ir::intrinsics::IntrinsicOp::VecBroadcastF64x4,
                dest_ptr: None,
                args: vec![Operand::Const(IrConst::F64(2.0))],
            }],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let removed = eliminate_dead_code(&mut func);
        assert_eq!(removed, 1);
        assert!(func.blocks[0].instructions.is_empty());
    }

    #[test]
    fn vector_store_intrinsics_are_never_removed_by_purity() {
        // The mirror image: a store-shaped vector intrinsic must survive DCE
        // even when its (nonexistent) result is unused -- it writes memory.
        let mut func = IrFunction::new("vs".to_string(), IrType::Void, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Intrinsic {
                dest: None,
                op: crate::ir::intrinsics::IntrinsicOp::VecStoreF64x4,
                dest_ptr: Some(Value(3)),
                args: vec![Operand::Value(Value(9))],
            }],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        let removed = eliminate_dead_code(&mut func);
        assert_eq!(removed, 0);
        assert_eq!(func.blocks[0].instructions.len(), 1);
    }
}
