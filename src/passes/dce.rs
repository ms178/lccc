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
//! unique def site for single-def values and diverts multi-def ids (tag bit
//! `MULTI_BIT`) to an overflow table whose sites are marked live TOGETHER.
//! (A refcount DCE fails multi-def only conservatively; naive mark-and-sweep
//! fails it UNSAFELY. Both directions are covered by tests below.)
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
//! # The id bound is computed from the IR, not from metadata
//!
//! `def_loc` is sized from the largest dest id actually present in the
//! function (Pass 0). It deliberately does NOT trust `max_value_id()`: that
//! helper returns the cached `next_value_id - 1` without scanning, so a pass
//! that minted a new value id without bumping the counter leaves the cached
//! bound stale and *low*. Sizing `def_loc` from the stale bound gives the
//! live definition no slot; its uses resolve to NO_DEF and the live
//! definition is swept — a silent miscompile (reproduced by
//! `stale_low_next_value_id_does_not_sweep_live_high_def`). Computing the
//! bound from the actual destinations is sound for every pass, including the
//! non-SSA post-phi case.
//!
//! Uses of ids beyond the bound have no def in this function and are
//! correctly treated as parameters/globals. The bound never needs to cover
//! *uses*: mark-and-sweep only ever needs to look a use up in `def_loc`, and
//! a use with no def is a parameter/global, which is exactly the NO_DEF /
//! out-of-bounds sentinel path.
//!
//! # Correctness constraints (do not "simplify" these away)
//!
//! * Ordinary `Alloca` is pure only for a proven single-block leaf whose
//!   ParamRefs execute in the leading prefix. Other functions retain positional
//!   parameter homes because late ParamRefs need the saved ABI value.
//!   Extending dead-alloca removal to multi-block functions requires proof
//!   that every backend's stack layout only relies on `param_alloca_values`
//!   (value-based) and never on positional slot identity; see the audit
//!   backlog. Until then it stays conservative.
//! * `DynAlloca` / `StackRestore` adjust the runtime stack pointer.
//! * Every `Call` / `CallIndirect` is a root until we have trustworthy
//!   `pure`/`const` attributes. Intrinsics already carry purity and *are*
//!   deleted when unused.
//! * This pass does **not** delete unreachable blocks. Run unreachable-
//!   code elimination first so side-effecting insts in dead blocks do not
//!   pin otherwise-dead values.
//!
//! # Data layout (compile-time budget)
//!
//! DCE runs several times per function per pipeline. Every table is flat and
//! indexed by a function-wide instruction number:
//!
//! ```text
//!   insts      : Vec<&Instruction>  flat idx → instruction
//!   block_off  : Vec<u32>           block b owns [block_off[b], block_off[b+1])
//!   live       : Vec<u8>            one byte per instruction
//!   def_loc    : Vec<u32>           value id → flat idx | NO_DEF | MULTI_BIT|k
//!   multi_defs : Vec<Vec<u32>>      k → every flat idx defining that id
//!   worklist   : Vec<u32>           flat indices, each pushed at most once
//! ```
//!
//! Four allocations regardless of block count (the previous per-block
//! `Vec<Vec<u8>>` cost `blocks + 1` allocations), a 4-byte def map (the
//! hottest random-access table), and no hashing on the multi-def path.
//!
//! # What this is not
//!
//! Aggressive DCE (control-dependence / post-dom frontier), bit-tracking
//! DCE, and dead-*store* elimination are separate passes. They need CFG
//! analyses this file must not quietly reimplement.

use crate::ir::reexports::{CallInfo, Instruction, IrFunction};

/// Sentinel stored in `def_loc` for values that have no removable definition
/// in this function (parameters, globals, malformed IDs).
const NO_DEF: u32 = u32::MAX;

/// Tag bit in `def_loc`: the low 31 bits index `multi_defs` instead of
/// naming a flat instruction. `NO_DEF` also has this bit set, so callers
/// must test `== NO_DEF` first. Flat instruction indices are guaranteed
/// `< MULTI_BIT` by the size guard in `eliminate_dead_code`.
const MULTI_BIT: u32 = 1 << 31;

/// Eliminate dead instructions in `func`.
///
/// Returns the number of instructions removed. The pass manager uses a
/// non-zero return as a "something changed" signal to iterate cooperating
/// passes (DCE → simplifycfg → DCE, …).
pub(crate) fn eliminate_dead_code(func: &mut IrFunction) -> usize {
    if func.blocks.is_empty() {
        return 0;
    }

    // ------------------------------------------------------------------
    // Pass 0: instruction count and the TRUE value-id bound (see module doc
    // "The id bound is computed from the IR, not from metadata").
    // ------------------------------------------------------------------
    let mut n_insts = 0usize;
    let mut max_dest: Option<u32> = None;
    for block in &func.blocks {
        n_insts += block.instructions.len();
        for inst in &block.instructions {
            if let Some(d) = inst.dest() {
                max_dest = Some(max_dest.map_or(d.0, |m| m.max(d.0)));
            }
        }
    }
    if n_insts == 0 {
        return 0;
    }
    // Flat indices must fit below the multi-def tag bit. Unreachable in
    // practice (2^31 instructions in one function); refusing to sweep is the
    // safe direction for fuzzers.
    if n_insts >= MULTI_BIT as usize {
        return 0;
    }

    // Constructed from the destinations actually present, so every def is in
    // bounds. saturating_add keeps the allocation well-defined even if the id
    // space is exhausted (impossible in practice; required for fuzz safety).
    let def_len = max_dest.map_or(0, |m| (m as usize).saturating_add(1));

    let mut def_loc: Vec<u32> = vec![NO_DEF; def_len];
    let mut multi_defs: Vec<Vec<u32>> = Vec::new();
    let mut insts: Vec<&Instruction> = Vec::with_capacity(n_insts);
    let mut block_off: Vec<u32> = Vec::with_capacity(func.blocks.len() + 1);
    // Per-instruction liveness flag. `u8` (not `bool`) so the flag can never
    // be mistaken for a semantic boolean; Rust's Vec<bool> is not bit-packed
    // anyway, so the footprint is identical.
    let mut live: Vec<u8> = vec![0u8; n_insts];
    // Every instruction is pushed at most once (guarded by its live flag),
    // so this capacity guarantees the worklist never reallocates.
    let mut worklist: Vec<u32> = Vec::with_capacity(n_insts);

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
    // Pass 1: build the flat instruction table, record every def site and
    // seed instruction roots. Terminator uses are deliberately NOT seeded
    // here (see module doc: def_loc must be complete before any lookup).
    // ------------------------------------------------------------------
    for block in &func.blocks {
        block_off.push(insts.len() as u32);
        for inst in &block.instructions {
            let fi = insts.len() as u32;
            insts.push(inst);
            if let Some(dest) = inst.dest() {
                // In bounds by construction: def_len covers every dest seen
                // in Pass 0 over the same, unmodified instruction lists.
                let slot = &mut def_loc[dest.0 as usize];
                if *slot == NO_DEF {
                    *slot = fi;
                } else if *slot & MULTI_BIT != 0 {
                    multi_defs[(*slot & !MULTI_BIT) as usize].push(fi);
                } else {
                    // Second def of the same id: post-phi copies. Divert
                    // BOTH sites to the overflow table; liveness of the
                    // value must pin every one of its defs.
                    let first = *slot;
                    *slot = MULTI_BIT | multi_defs.len() as u32;
                    multi_defs.push(vec![first, fi]);
                }
            }
            if is_dce_root(inst, !allow_dead_allocas) {
                live[fi as usize] = 1;
                worklist.push(fi);
            }
        }
    }
    block_off.push(insts.len() as u32);
    debug_assert_eq!(insts.len(), n_insts);

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
    // Propagate liveness along the use-def graph. This is a function rather
    // than an inline loop because it is also called below, after the
    // parameter-home pinning, to restore the invariant "the worklist is
    // empty when the sweep starts".
    //
    // A phi is live iff some *already-live* user (instruction or
    // terminator) reads it. Once live, *every* incoming operand becomes
    // live — including self- and cross-references. That is what makes
    // unused induction-variable cycles vanish: they are never seeded.
    // The already-live check in mark_site_live is what terminates walks
    // around *live* phi cycles.
    // ------------------------------------------------------------------
    propagate(&insts, &def_loc, &multi_defs, &mut live, &mut worklist);

    // Parameter homes are still identified by nth-Alloca position in every
    // backend. If parameter k's home survives, every earlier parameter-home
    // Alloca must survive too or k silently shifts to the wrong ABI argument.
    // Block 0 starts at flat index 0, so its instruction index IS its flat
    // index.
    if allow_dead_allocas {
        let param_allocas: Vec<u32> = func.blocks[0]
            .instructions
            .iter()
            .enumerate()
            .filter_map(|(ii, inst)| {
                matches!(inst, Instruction::Alloca { .. }).then_some(ii as u32)
            })
            .take(func.params.len())
            .collect();
        if let Some(last_live) = param_allocas
            .iter()
            .rposition(|&fi| live[fi as usize] != 0)
        {
            for &fi in &param_allocas[..=last_live] {
                mark_site_live(fi, &mut live, &mut worklist);
            }
            // Allocas have no operands today, but the invariant "the
            // worklist is empty when the sweep starts" must not depend on
            // that. Draining here is O(pinned) and keeps the pass correct
            // if a future home opcode ever carries operands.
            propagate(&insts, &def_loc, &multi_defs, &mut live, &mut worklist);
        }
    }
    debug_assert!(worklist.is_empty());
    // Release the immutable borrow of `func.blocks` before the sweep.
    drop(insts);

    if dce_debug_enabled() {
        dump_dead_instructions(func, &live, &block_off);
    }

    // ------------------------------------------------------------------
    // Sweep. Single pass, order of surviving instructions preserved.
    // ------------------------------------------------------------------
    let mut total = 0usize;
    for (bi, block) in func.blocks.iter_mut().enumerate() {
        let start = block_off[bi] as usize;
        let end = block_off[bi + 1] as usize;
        total += sweep_block(
            &mut block.instructions,
            &mut block.source_spans,
            &live[start..end],
        );
    }
    total
}

/// Drain `worklist`, marking the defs of every operand of every popped
/// instruction. Each instruction enters the worklist at most once (its live
/// flag is set on push), so the total work is O(sum of operand counts).
fn propagate(
    insts: &[&Instruction],
    def_loc: &[u32],
    multi_defs: &[Vec<u32>],
    live: &mut [u8],
    worklist: &mut Vec<u32>,
) {
    while let Some(fi) = worklist.pop() {
        let inst = insts[fi as usize];
        inst.for_each_used_value(|id| {
            mark_def_live(id, def_loc, multi_defs, live, worklist);
        });
        // `dest_ptr` is a pointer *operand* of a store-shaped intrinsic.
        // `for_each_used_value` already visits it (see `Instruction`'s
        // `Intrinsic` arm), so the explicit pin below is a no-op today. It is
        // kept as defense-in-depth: a regression in the canonical visitor
        // must never leave a store intrinsic's pointer producer dead and let
        // the sweep write through a dangling slot.
        if let Instruction::Intrinsic {
            dest_ptr: Some(p), ..
        } = inst
        {
            mark_def_live(p.0, def_loc, multi_defs, live, worklist);
        }
    }
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
    def_loc: &[u32],
    multi_defs: &[Vec<u32>],
    live: &mut [u8],
    worklist: &mut Vec<u32>,
) {
    let d = match def_loc.get(id as usize) {
        Some(&d) => d,
        None => return, // no def in this function (parameter / global)
    };
    if d == NO_DEF {
        return;
    }
    if d & MULTI_BIT != 0 {
        // Index is in bounds by construction (assigned from multi_defs.len()
        // right before the push); `get` keeps release builds panic-free.
        if let Some(sites) = multi_defs.get((d & !MULTI_BIT) as usize) {
            for &fi in sites {
                mark_site_live(fi, live, worklist);
            }
        }
        return;
    }
    mark_site_live(d, live, worklist);
}

#[inline]
fn mark_site_live(fi: u32, live: &mut [u8], worklist: &mut Vec<u32>) {
    if let Some(slot) = live.get_mut(fi as usize) {
        if *slot == 0 {
            *slot = 1;
            worklist.push(fi);
        }
    }
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
        "DCE live-flag slice desynchronized from instruction list"
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
    debug_assert_eq!(instructions.len(), original_len - dead_count);
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

fn dump_dead_instructions(func: &IrFunction, live: &[u8], block_off: &[u32]) {
    for (bi, block) in func.blocks.iter().enumerate() {
        let Some(&start) = block_off.get(bi) else { break };
        let start = start as usize;
        for (ii, inst) in block.instructions.iter().enumerate() {
            match live.get(start + ii) {
                Some(0) => {}
                _ => continue,
            }
            if let Some(d) = inst.dest() {
                eprintln!("[DCE] Removing Value({}) in block {}: {:?}", d.0, bi, inst);
            } else {
                eprintln!("[DCE] Removing dest-less inst in block {}: {:?}", bi, inst);
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
#[inline]
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
        return (!*is_pure && !*is_const) || *is_sret;
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
        // Atomics are kept regardless of ordering: even an unused relaxed
        // load participates in the modification order LLVM/GCC also refuse
        // to reason away here. Cheap to keep, unsafe to get wrong.
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
        // Every asm statement is a root today. BACKLOG: a non-volatile asm
        // whose outputs are all unused is deletable (GCC/Clang do so); that
        // needs the volatile flag of the InlineAsm payload, not visible here.
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

    fn alloca(dest: u32, ty: IrType, size: usize, align: usize) -> Instruction {
        Instruction::Alloca {
            dest: Value(dest),
            ty,
            size,
            align,
            volatile: false,
            semantic_volatile: false,
        }
    }

    fn make_simple_func() -> IrFunction {
        // Function with: %0 = alloca i32, %1 = add 3, 4 (dead), store 42 to %0, load from %0
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                alloca(0, IrType::I32, 4, 0),
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
    fn pure_call_with_unused_result_is_removed() {
        // A pure call whose value nobody reads is dead (C attribute semantics
        // as in GCC/Clang). This is the direction that buys code quality.
        let mut func = IrFunction::new("test".to_string(), IrType::Void, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Call {
                func: "strlen".to_string(),
                info: CallInfo {
                    dest: Some(Value(0)),
                    is_pure: true,
                    ..CallInfo::default()
                },
            }],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        assert_eq!(eliminate_dead_code(&mut func), 1);
        assert!(func.blocks[0].instructions.is_empty());
    }

    #[test]
    fn pure_sret_call_is_kept() {
        // gcc.c-torture 20070614-1: the observable effect of an sret call is
        // the write through the hidden pointer, not the unused dest value.
        let mut func = IrFunction::new("test".to_string(), IrType::Void, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Call {
                func: "mk_complex".to_string(),
                info: CallInfo {
                    dest: Some(Value(0)),
                    is_pure: true,
                    is_sret: true,
                    ..CallInfo::default()
                },
            }],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        assert_eq!(eliminate_dead_code(&mut func), 0);
    }

    #[test]
    fn volatile_load_with_unused_result_is_kept() {
        // C11 5.1.2.3: a volatile access is an observable side effect.
        // Single-block leaf: the alloca is NOT a root on its own, so this
        // also checks that the volatile load pins its pointer operand.
        let mut func = IrFunction::new("test".to_string(), IrType::Void, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                alloca(0, IrType::I32, 4, 4),
                Instruction::Load {
                    volatile: true,
                    dest: Value(1),
                    ptr: Value(0),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        assert_eq!(eliminate_dead_code(&mut func), 0);
        assert_eq!(func.blocks[0].instructions.len(), 2);
    }

    #[test]
    fn test_transitive_dead_chain() {
        // %1 -> %2 -> %3, none used: all three go in one pass.
        let mut func = IrFunction::new("test".to_string(), IrType::Void, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                alloca(0, IrType::I32, 4, 0),
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
                alloca(0, IrType::Ptr, 8, 8),
                alloca(1, IrType::Ptr, 8, 8),
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
    fn dead_non_parameter_alloca_removed_in_single_block_leaf() {
        // No parameters → no positional homes → a dead slot is just wasted
        // frame space and must go.
        let mut func = IrFunction::new("leaf".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![alloca(0, IrType::I64, 8, 8)],
            terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            source_spans: vec![],
        });
        assert_eq!(eliminate_dead_code(&mut func), 1);
        assert!(func.blocks[0].instructions.is_empty());
    }

    #[test]
    fn dead_alloca_kept_in_multi_block_function() {
        // Documents the deliberate conservatism (see module doc): outside a
        // proven single-block leaf, allocas are roots. If this test starts
        // failing because the policy was relaxed, the positional-home proof
        // for every backend must accompany that change.
        let mut func = IrFunction::new("multi".to_string(), IrType::Void, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![alloca(0, IrType::I64, 8, 8)],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: vec![],
        });
        assert_eq!(eliminate_dead_code(&mut func), 0);
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

    /// Build the classic post-`eliminate_phis` diamond/fan-in: `n_preds`
    /// predecessors each `Copy %5 = const`, all branching to a join block.
    fn multi_def_fan_in(n_preds: u32, ret: Terminator) -> IrFunction {
        let ret_ty = if matches!(ret, Terminator::Return(Some(_))) {
            IrType::I32
        } else {
            IrType::Void
        };
        let mut func = IrFunction::new("test".to_string(), ret_ty, vec![], false);
        let join = BlockId(n_preds + 1);
        // Entry fans out to predecessors 1..=n_preds via a chain of branches
        // (the exact CFG shape is irrelevant to DCE; only def sites matter).
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::CondBranch {
                cond: Operand::Const(IrConst::I32(1)),
                true_label: BlockId(1),
                false_label: BlockId(n_preds),
            },
            source_spans: Vec::new(),
        });
        for p in 1..=n_preds {
            func.blocks.push(BasicBlock {
                label: BlockId(p),
                instructions: vec![Instruction::Copy {
                    dest: Value(5),
                    src: Operand::Const(IrConst::I32(10 * p as i32)),
                }],
                terminator: Terminator::Branch(join),
                source_spans: Vec::new(),
            });
        }
        func.blocks.push(BasicBlock {
            label: join,
            instructions: vec![],
            terminator: ret,
            source_spans: Vec::new(),
        });
        func
    }

    #[test]
    fn test_multi_def_post_phi_copies_all_kept() {
        // NON-SSA input, exactly what eliminate_phis produces: the same
        // dest id %5 is written by a Copy in BOTH predecessors, and the
        // join block returns %5. A single-slot def map marks only the
        // last-seen copy live and sweeps the first — the returned value
        // would be garbage on the other path. Both copies must survive.
        let mut func =
            multi_def_fan_in(2, Terminator::Return(Some(Operand::Value(Value(5)))));
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
        let mut func = multi_def_fan_in(2, Terminator::Return(None));
        let removed = eliminate_dead_code(&mut func);
        assert_eq!(removed, 2, "all defs of a dead multi-def value must go");
    }

    #[test]
    fn multi_def_three_sites_exercise_overflow_append() {
        // Third def of the same id takes the "already multi-def → append"
        // path rather than the "first collision → create entry" path.
        let mut live_func =
            multi_def_fan_in(3, Terminator::Return(Some(Operand::Value(Value(5)))));
        assert_eq!(eliminate_dead_code(&mut live_func), 0);
        for p in 1..=3 {
            assert_eq!(live_func.blocks[p].instructions.len(), 1, "copy {p} deleted");
        }

        let mut dead_func = multi_def_fan_in(3, Terminator::Return(None));
        assert_eq!(eliminate_dead_code(&mut dead_func), 3);
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
    fn sparse_high_value_ids_and_undefined_uses_are_handled() {
        // The def map is sized from the ids actually present, independent
        // of any function-level counter. A live def with a large sparse id
        // must be tracked (not silently swept), and uses of ids with no def
        // in this function (parameters/globals, here plainly undefined)
        // must be no-ops rather than panics.
        const HIGH: u32 = 100_003;
        let mut func = IrFunction::new("sparse".to_string(), IrType::I32, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::BinOp {
                    dest: Value(HIGH),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(50_000)), // no def anywhere
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
                Instruction::BinOp {
                    dest: Value(HIGH - 1), // dead
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(HIGH)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(HIGH)))),
            source_spans: Vec::new(),
        });
        assert_eq!(eliminate_dead_code(&mut func), 1);
        assert_eq!(func.blocks[0].instructions.len(), 1);
        assert!(matches!(
            func.blocks[0].instructions[0],
            Instruction::BinOp { dest: Value(HIGH), .. }
        ));
    }

    #[test]
    fn stale_low_next_value_id_does_not_sweep_live_high_def() {
        // RED-TEAM regression for the "bound computed from the IR" fix.
        // A pass minted Value id 100_003 but left next_value_id stale at 5.
        // max_value_id() then reports 4, so the OLD implementation sized
        // def_loc to 5, gave the def at 100_003 NO slot, and swept the LIVE
        // returned value (a silent miscompile). The new implementation sizes
        // from the actual destinations and must keep the BinOp.
        let mut func = IrFunction::new("stale".to_string(), IrType::I32, vec![], false);
        func.next_value_id = 5; // stale/cached bound under-reports the def
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::BinOp {
                dest: Value(100_003),
                op: IrBinOp::Add,
                lhs: Operand::Const(IrConst::I32(1)),
                rhs: Operand::Const(IrConst::I32(2)),
                ty: IrType::I32,
            }],
            terminator: Terminator::Return(Some(Operand::Value(Value(100_003)))),
            source_spans: Vec::new(),
        });
        assert_eq!(eliminate_dead_code(&mut func), 0);
        assert_eq!(func.blocks[0].instructions.len(), 1);
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
    fn blocks_without_instructions_are_a_noop() {
        let mut func = IrFunction::new("empty_blocks".to_string(), IrType::Void, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        assert_eq!(eliminate_dead_code(&mut func), 0);
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
    fn stale_wrong_length_source_spans_are_cleared() {
        // A span vector that is neither empty nor 1:1 is a stale artifact;
        // the sweep must restore the "empty == no debug info" invariant
        // instead of leaving a desynchronized table behind.
        let mut func = make_simple_func();
        let sp = |n: u32| crate::common::source::Span::new(n, n + 1, 0);
        func.blocks[0].source_spans = vec![sp(10), sp(20)];
        assert_eq!(eliminate_dead_code(&mut func), 1);
        assert!(func.blocks[0].source_spans.is_empty());
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

    #[test]
    fn vector_store_dest_ptr_pins_pointer_producer() {
        // Single-block leaf: the alloca is NOT a root by itself. It must
        // survive purely because the store intrinsic's `dest_ptr` reads it.
        // Sweeping it would make the store write through a dangling slot.
        let mut func = IrFunction::new("vsp".to_string(), IrType::Void, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                alloca(3, IrType::F64, 32, 32),
                Instruction::Intrinsic {
                    dest: None,
                    op: crate::ir::intrinsics::IntrinsicOp::VecStoreF64x4,
                    dest_ptr: Some(Value(3)),
                    args: vec![Operand::Value(Value(9))],
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        assert_eq!(eliminate_dead_code(&mut func), 0);
        assert!(matches!(
            func.blocks[0].instructions[0],
            Instruction::Alloca { dest: Value(3), .. }
        ));
    }
}
