//! Sparse Conditional Constant Propagation (Wegman & Zadeck, TOPLAS 1991).
//!
//! SCCP reasons about *control flow* and *data flow* simultaneously. It runs
//! two interacting worklists over an optimistic lattice:
//!
//! ```text
//!            ⊤  (Top)        not yet reached — assumed constant
//!         /  |  \
//!    C(0) C(1) C(2) ...      known to be exactly this constant
//!         \  |  /
//!            ⊥  (Bottom)     overdefined — may differ between executions
//! ```
//!
//! Blocks start *unreachable* and edges start *non-executable*; a conditional
//! branch on a value that is still `⊤` marks neither successor. Because a phi
//! only meets the operands arriving on executable edges, SCCP proves constants
//! that no amount of iterated local folding can reach:
//!
//! ```c
//! int f(int n) {
//!     int x;
//!     if (n > 0) x = 7; else x = 7;   // phi(7, 7)          -> 7
//!     if (x == 7) return 1;            // branch folds       -> return 1
//!     return g();                      // proven unreachable
//! }
//! ```
//!
//! and, crucially, it does so *without* first proving the branch dead — the two
//! facts are discovered together, which is exactly what makes the algorithm
//! strictly stronger than "constant fold, then simplify CFG, repeat".
//!
//! # Relationship to the rest of the pipeline
//!
//! * **Constant semantics** come from [`crate::passes::constant_fold`]'s shared
//!   oracle (`eval_binop_const` and friends). This pass owns *no* arithmetic of
//!   its own, so it can never disagree with the folder about what `1.0L/3.0L`
//!   or `(signed char)200` evaluates to. Even the algebraic absorption rules
//!   (`x * 0`, `x & 0`, `x | -1`; see [`fold_despite_unknown`]) obtain their
//!   typed results by asking the oracle to fold `v op v`, which normalises the
//!   constant representation to the `BinOp`'s type.
//! * **Materialisation** follows the codebase idiom: a value proven constant
//!   has its *defining instruction* rewritten to `Copy { dest, Const }`, and
//!   its uses substituted in place. `copy_prop`/`dce` then clean up.
//! * **CFG cleanup** is left to `cfg_simplify`, but the edges this pass kills
//!   are repaired here (see "Edge maintenance" below) so the IR is well-formed
//!   at every point in between.
//! * **Ordering.** SCCP runs in the post-inline cleanup phase, *before* the
//!   main pipeline's `bit_idioms`. Algebraic folds placed here therefore die
//!   one full pipeline stage earlier and can fold branches in the same SCCP
//!   run that `bit_idioms` would only have simplified after the main loop.
//!
//! # Soundness invariants
//!
//! These are the properties that make the pass safe. Each is enforced
//! mechanically, not by convention.
//!
//! 1. **No value may stay `⊤` unless it is genuinely unevaluated.** Every
//!    instruction that defines a value and is not explicitly modelled falls
//!    through to a closed default that lowers its destination to `⊥`
//!    ([`evaluate_instruction`]). A `⊤` that reaches a phi is *absorbed*
//!    (`⊤ ⊓ C = C`), so an unmodelled opcode silently left at `⊤` would make
//!    the phi report a constant it does not have. Adding an IR opcode can
//!    therefore never introduce a miscompile here, only a missed optimisation.
//!    `InlineAsm` is modelled explicitly: its `outputs` are definitions that
//!    `dest()` does not cover (an asm can define several), so they are lowered
//!    to `⊥` by construction — not left to the accident that the use-def table
//!    currently counts asm outputs as uses without a recorded definition.
//! 2. **Every control-flow edge is modelled, including the implicit ones.**
//!    `asm goto` (`InlineAsm { goto_labels }`) is real control flow that lives
//!    on an *instruction*, not on the terminator. Ignoring it would mark a
//!    block reachable only from an `asm goto` as dead and delete its phi
//!    operands — see the regression test `asm_goto_targets_stay_reachable`.
//! 3. **Edge maintenance.** Folding `CondBranch`/`Switch` to `Branch` removes
//!    CFG edges. Every phi in a block that loses a predecessor has the matching
//!    incoming operand removed in the same step, so the IR is never handed to
//!    the next pass with a phi operand for an edge that no longer exists.
//! 4. **Only pure definitions are replaced.** A value can only leave `⊤` for a
//!    constant through the pure opcodes (`BinOp`, `UnaryOp`, `Cmp`, `Cast`,
//!    `Copy`, `Select`, `Phi`); everything with a side effect is seeded `⊥`.
//!    The rewrite additionally asserts this before overwriting a definition, so
//!    a future lattice extension cannot delete a call or a volatile load.
//! 5. **No `⊤` survives the fixpoint inside an executable block.** In valid
//!    SSA this cannot happen (every operand's definition dominates its use and
//!    is therefore evaluated). It *does* happen on IR that is tolerated but not
//!    valid — a phi lacking an entry for the predecessor that actually reached
//!    it, an empty phi, a use not dominated by its def. A `CondBranch` on such
//!    a `⊤` would mark *neither* successor, the successor would be treated as
//!    dead, its out-edges would be pruned from live phis — yet the branch would
//!    not be folded (it is not a constant), so the "dead" block still runs at
//!    runtime with a phi copy missing on its edge. That is a miscompile. After
//!    every fixpoint, [`force_unresolved_to_bottom`] lowers any remaining `⊤`
//!    in an executable block to `⊥` (conservative — *not* to an arbitrary
//!    constant, because the cause may not be a genuine `undef`) and the solver
//!    re-runs until nothing is left. LLVM's `resolvedUndefsIn` loop exists for
//!    the same reason.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, IrUnaryOp, Operand,
    Terminator, Value,
};
use crate::passes::constant_fold;
use crate::passes::use_def::UseDefInfo;

/// Lattice element. Values only ever move *down*: `⊤ → C → ⊥`.
#[derive(Debug, Clone, Copy)]
enum LatticeVal {
    /// Not yet reached. The optimistic assumption that makes SCCP stronger
    /// than pessimistic dataflow — and the reason invariant (1) above matters.
    Top,
    /// Provably this exact constant on every execution that reaches the def.
    Constant(IrConst),
    /// Overdefined.
    Bottom,
}

impl LatticeVal {
    /// Lattice meet (greatest lower bound).
    #[inline]
    fn meet(self, other: LatticeVal) -> LatticeVal {
        match (self, other) {
            (LatticeVal::Top, x) | (x, LatticeVal::Top) => x,
            (LatticeVal::Bottom, _) | (_, LatticeVal::Bottom) => LatticeVal::Bottom,
            (LatticeVal::Constant(a), LatticeVal::Constant(b)) => {
                // Bit-pattern equality: `to_hash_key` distinguishes +0.0 from
                // -0.0 and keeps NaN payloads apart, which plain `==` on f64
                // would not (0.0 == -0.0, NaN != NaN).
                if a.to_hash_key() == b.to_hash_key() {
                    LatticeVal::Constant(a)
                } else {
                    LatticeVal::Bottom
                }
            }
        }
    }

    #[inline]
    fn is_top(self) -> bool {
        matches!(self, LatticeVal::Top)
    }

    #[inline]
    fn is_bottom(self) -> bool {
        matches!(self, LatticeVal::Bottom)
    }

    #[inline]
    fn as_const(self) -> Option<IrConst> {
        match self {
            LatticeVal::Constant(c) => Some(c),
            _ => None,
        }
    }
}

/// Pack a CFG edge into one `u64` so the executable-edge set hashes a single
/// integer instead of a tuple. `FxHasher` reduces that to one multiply.
#[inline]
fn edge_key(from: u32, to: u32) -> u64 {
    ((from as u64) << 32) | (to as u64)
}

struct SccpState {
    /// Lattice value per SSA value, indexed by `Value.0`.
    lattice: Vec<LatticeVal>,
    /// Whether each block (by position) has been proven reachable.
    block_executable: Vec<bool>,
    /// Executable CFG edges, keyed by [`edge_key`].
    executable_edges: FxHashSet<u64>,
    /// Blocks to (re)visit.
    cfg_worklist: Vec<u32>,
    /// Values whose lattice element moved down.
    ssa_worklist: Vec<u32>,
    /// `BlockId` → position in `func.blocks`.
    label_to_idx: FxHashMap<BlockId, u32>,
    /// `to_ty` of the `Cast` defining each value, if any. Mirrors the
    /// `ConstMapEntry::cast_to_ty` that `constant_fold` threads into the C11
    /// sub-int promotion rule, so unary folding agrees between the two passes.
    cast_to_ty: Vec<Option<IrType>>,
}

/// Result of an SCCP run, for pipeline bookkeeping and tests.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SccpStats {
    /// Definitions rewritten to `Copy { dest, Const }`.
    pub defs_materialised: usize,
    /// Operand occurrences replaced by a constant.
    pub operands_substituted: usize,
    /// `CondBranch`/`Switch` terminators folded to `Branch`.
    pub branches_folded: usize,
    /// Phi incoming operands dropped because their edge is not executable.
    pub phi_edges_pruned: usize,
    /// Blocks proven unreachable (left for `cfg_simplify` to delete).
    pub unreachable_blocks: usize,
}

impl SccpStats {
    /// Total IR changes; used as the pass's "did something" signal.
    #[inline]
    pub fn total(self) -> usize {
        self.defs_materialised
            + self.operands_substituted
            + self.branches_folded
            + self.phi_edges_pruned
    }
}

/// Run SCCP over every function in the module. Returns the number of changes.
pub fn run(module: &mut crate::ir::reexports::IrModule) -> usize {
    let mut total = 0;
    for func in module.functions.iter_mut() {
        total += run_function(func);
    }
    total
}

/// Run SCCP on one function, building the use-def info it needs.
pub fn run_function(func: &mut IrFunction) -> usize {
    let usedef = UseDefInfo::build(func);
    run_with_usedef(func, &usedef).total()
}

/// Run SCCP on one function with pre-built use-def info.
///
/// `usedef` must describe `func` as it is *now*: SCCP indexes instructions
/// positionally through the use-chains. The rewrite phase runs only after the
/// fixpoint is reached, so it never invalidates the info it is still reading.
pub fn run_with_usedef(func: &mut IrFunction, usedef: &UseDefInfo) -> SccpStats {
    if func.blocks.is_empty() {
        return SccpStats::default();
    }
    debug_assert!(
        !usedef.is_stale_for(func),
        "SCCP was handed use-def info built for a different IR shape"
    );

    let num_blocks = func.blocks.len();

    // The lattice must have a slot for every value *defined* in this function.
    // `max_value_id()` is metadata and may be stale; the use-def table may be
    // sized differently. Trust neither alone: take the maximum of both and of
    // the largest dest id actually present in the IR. Uses of ids beyond the
    // bound resolve to ⊥ (see `resolve`), which is the safe direction; the
    // `get`/`get_mut` accesses throughout make a size disagreement a missed
    // optimisation instead of a panic in release builds.
    let mut max_dest: usize = 0;
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(d) = inst.dest() {
                max_dest = max_dest.max(d.0 as usize);
            }
        }
    }
    let num_values = usedef
        .len()
        .max((func.max_value_id() as usize).saturating_add(1))
        .max(max_dest.saturating_add(1));

    let mut label_to_idx: FxHashMap<BlockId, u32> =
        FxHashMap::with_capacity_and_hasher(num_blocks, Default::default());
    for (i, block) in func.blocks.iter().enumerate() {
        // A duplicated label is malformed IR; keep the first, which is what
        // every other lccc pass that builds this map does.
        label_to_idx.entry(block.label).or_insert(i as u32);
    }

    let mut state = SccpState {
        lattice: vec![LatticeVal::Top; num_values],
        block_executable: vec![false; num_blocks],
        executable_edges: FxHashSet::default(),
        cfg_worklist: Vec::with_capacity(num_blocks),
        ssa_worklist: Vec::new(),
        label_to_idx,
        cast_to_ty: vec![None; num_values],
    };

    seed(func, usedef, &mut state, num_values);

    // ── fixpoint ────────────────────────────────────────────────────────────
    // `solve` terminates because both worklists are only fed by strictly
    // downward lattice moves (`update_lattice`) and by first-time edge
    // insertions (`mark_edge_executable`). Both are finite and monotone.
    //
    // The outer loop is soundness invariant (5): any ⊤ that survived inside an
    // executable block is forced to ⊥ and the solver re-runs. Each round lowers
    // at least one value, so it too terminates; on well-formed SSA it costs
    // exactly one linear scan that forces nothing.
    state.cfg_worklist.push(0);
    loop {
        solve(func, usedef, &mut state);
        if !force_unresolved_to_bottom(func, &mut state) {
            break;
        }
    }

    rewrite(func, &state)
}

/// Drain both worklists to a fixpoint.
fn solve(func: &IrFunction, usedef: &UseDefInfo, state: &mut SccpState) {
    let num_blocks = func.blocks.len();
    while !state.cfg_worklist.is_empty() || !state.ssa_worklist.is_empty() {
        while let Some(block_idx) = state.cfg_worklist.pop() {
            let bi = block_idx as usize;
            if bi >= num_blocks {
                continue;
            }
            if !state.block_executable[bi] {
                state.block_executable[bi] = true;
                visit_block(func, block_idx, state);
            } else {
                // Already visited; a *new* incoming edge can only change phis
                // (and, through them, the terminator).
                visit_phis(func, block_idx, state);
            }
        }

        while let Some(value_id) = state.ssa_worklist.pop() {
            // Re-evaluate each consumer once, even if it reads the value twice
            // (`%d = add %v, %v` yields two adjacent identical chain entries).
            for loc in usedef.use_insts_of(value_id) {
                let bi = loc.block_idx as usize;
                if bi >= num_blocks || !state.block_executable[bi] {
                    continue;
                }
                if loc.is_terminator() {
                    evaluate_terminator(&func.blocks[bi].terminator, loc.block_idx, state);
                } else if let Some(inst) = func.blocks[bi].instructions.get(loc.inst_idx as usize) {
                    evaluate_instruction(inst, loc.block_idx, state);
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Seeding
// ═══════════════════════════════════════════════════════════════════════════

fn seed(func: &IrFunction, usedef: &UseDefInfo, state: &mut SccpState, num_values: usize) {
    for block in func.blocks.iter() {
        for inst in &block.instructions {
            match inst {
                // Parameters are unknown on entry. (IPCP is the pass that knows
                // better; it runs separately and rewrites ParamRef to a Copy.)
                Instruction::ParamRef { dest, .. } => {
                    if let Some(slot) = state.lattice.get_mut(dest.0 as usize) {
                        *slot = LatticeVal::Bottom;
                    }
                }
                Instruction::Cast { dest, to_ty, .. } => {
                    if let Some(slot) = state.cast_to_ty.get_mut(dest.0 as usize) {
                        *slot = Some(*to_ty);
                    }
                }
                // Invariant (1): `InlineAsm` outputs are definitions the
                // `dest()` accessor does not cover. Lower them here so their
                // opacity never depends on how the use-def table happens to
                // record them.
                Instruction::InlineAsm { outputs, .. } => {
                    for (_, v, _) in outputs {
                        if let Some(slot) = state.lattice.get_mut(v.0 as usize) {
                            *slot = LatticeVal::Bottom;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // A value that is read but never defined is a dangling reference left by an
    // earlier pass. It must be ⊥: leaving it ⊤ would let it be absorbed by a
    // phi meet and turn an unknown into a fabricated constant. Bounded by every
    // table involved so a size disagreement can never index out of range.
    let scan = num_values.min(usedef.len()).min(state.lattice.len());
    for v in 0..scan {
        let used = usedef.use_count.get(v).map_or(false, |&n| n > 0);
        if used && usedef.def_of(v as u32).is_none() {
            state.lattice[v] = LatticeVal::Bottom;
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Transfer functions
// ═══════════════════════════════════════════════════════════════════════════

fn visit_block(func: &IrFunction, block_idx: u32, state: &mut SccpState) {
    let block = &func.blocks[block_idx as usize];
    for inst in &block.instructions {
        evaluate_instruction(inst, block_idx, state);
        // Invariant (2): `asm goto` is control flow carried by an instruction.
        if let Instruction::InlineAsm { goto_labels, .. } = inst {
            for (_, label) in goto_labels {
                if let Some(&to) = state.label_to_idx.get(label) {
                    mark_edge_executable(block_idx, to, state);
                }
            }
        }
    }
    evaluate_terminator(&block.terminator, block_idx, state);
}

fn visit_phis(func: &IrFunction, block_idx: u32, state: &mut SccpState) {
    let block = &func.blocks[block_idx as usize];
    for inst in &block.instructions {
        if matches!(inst, Instruction::Phi { .. }) {
            evaluate_instruction(inst, block_idx, state);
        }
    }
    // The terminator may test a phi result that just moved.
    evaluate_terminator(&block.terminator, block_idx, state);
}

#[inline]
fn resolve(op: &Operand, state: &SccpState) -> LatticeVal {
    match op {
        Operand::Const(c) => LatticeVal::Constant(*c),
        Operand::Value(v) => state
            .lattice
            .get(v.0 as usize)
            .copied()
            .unwrap_or(LatticeVal::Bottom),
    }
}

/// The constant `v` is proven to be, if any.
#[inline]
fn lattice_const(state: &SccpState, v: Value) -> Option<IrConst> {
    state
        .lattice
        .get(v.0 as usize)
        .copied()
        .and_then(LatticeVal::as_const)
}

/// Lower `value_id` to `old ⊓ new`, enqueuing it when it actually moved.
///
/// Meeting with the previous element (rather than assigning) keeps the pass
/// monotone even for transfer functions that are not themselves monotone —
/// `Select` with a condition that goes `C → ⊥` is the practical case: its
/// result can jump between two unrelated constants, and the meet collapses that
/// to `⊥` instead of oscillating forever.
#[inline]
fn update_lattice(value_id: u32, new_val: LatticeVal, state: &mut SccpState) {
    let Some(slot) = state.lattice.get_mut(value_id as usize) else {
        return;
    };
    let old = *slot;
    let merged = old.meet(new_val);

    let changed = match (old, merged) {
        (LatticeVal::Bottom, _) => false,
        (LatticeVal::Top, LatticeVal::Top) => false,
        (LatticeVal::Top, _) => true,
        (LatticeVal::Constant(a), LatticeVal::Constant(b)) => a.to_hash_key() != b.to_hash_key(),
        (LatticeVal::Constant(_), LatticeVal::Bottom) => true,
        // Unreachable: meet never moves up. Listed for exhaustiveness.
        (LatticeVal::Constant(_), LatticeVal::Top) => false,
    };

    if changed {
        *slot = merged;
        state.ssa_worklist.push(value_id);
    }
}

/// Integer zero in every width this IR can store. `Zero` is the IR's
/// type-agnostic zero; it may only be treated as an *integer* zero because
/// [`fold_despite_unknown`] guards every rule on `ty.is_integer()` — the same
/// `Zero` can appear in float contexts, where multiply-by-zero is not
/// absorbing (`NaN * 0.0` is NaN, `-x * 0.0` is `-0.0`).
#[inline]
fn is_integer_zero(c: IrConst) -> bool {
    matches!(
        c,
        IrConst::I8(0)
            | IrConst::I16(0)
            | IrConst::I32(0)
            | IrConst::I64(0)
            | IrConst::I128(0)
            | IrConst::Zero
    )
}

/// All-ones in the constant's own width. Only exact-width negative
/// constructors match: `IrConst::I128(0x0000_0000_0000_0000_FFFF_FFFF_FFFF_FFFF)`
/// would truncate to `-1` in `to_i64()` yet is *not* all-ones in 128 bits, so
/// the test is by constructor, never by conversion. Narrower `BinOp` types
/// truncate correctly inside the shared oracle.
#[inline]
fn is_all_ones(c: IrConst) -> bool {
    matches!(
        c,
        IrConst::I8(-1)
            | IrConst::I16(-1)
            | IrConst::I32(-1)
            | IrConst::I64(-1)
            | IrConst::I128(-1)
    )
}

/// Results that are constant although an operand is unknown.
///
/// Three rule classes, all guarded on `ty.is_integer()`:
///
/// * **Same-value identities.** The two operands are the *same* SSA value, so
///   `x ^ x == 0` and `x - x == 0` in every integer width (wrapping), whatever
///   `x` may be. This is a syntactic fact about SSA identity, independent of
///   the lattice, hence monotone.
/// * **Absorbing zero.** `x * 0 == 0` and `x & 0 == 0` for any `x` — the one
///   place where SCCP is strictly more precise than "both operands constant".
///   GCC's CCP (through its bit-mask tracking) and LLVM's SCCP (through
///   `simplifyInstruction`) both perform it, and it matters in practice: a
///   Kconfig mask folded to `0`, or an element count proven `0` by IPCP, turns
///   `flags & MASK` / `n * size` into `0`, which is what lets the guarding
///   branch fold and the whole path die in the *same* SCCP run instead of two
///   pipeline iterations later.
/// * **Absorbing all-ones.** `x | -1 == -1` for any `x`, the mirror image of
///   the zero rule for `Or`.
///
/// Soundness: signed-overflow UB cannot arise from `0`/`-1` seeds; float
/// multiply and float subtraction are excluded wholesale because `NaN * 0`,
/// `-x * 0` and `NaN - NaN` are not `0`. The typed results are produced by the
/// shared oracle (`v op v`), so this pass still owns no arithmetic. Monotone:
/// each result is the same constant regardless of what the other operand later
/// becomes. If the *seed* operand itself later moves down (a `Constant` can
/// still reach `⊥`), the rule stops firing and the meet drops the result to
/// `⊥` — sound, and still monotone.
#[inline]
fn fold_despite_unknown(
    op: IrBinOp,
    lhs: &Operand,
    rhs: &Operand,
    lv: LatticeVal,
    rv: LatticeVal,
    ty: IrType,
) -> Option<IrConst> {
    if !ty.is_integer() {
        return None;
    }

    // Same-value identities: syntactic, checked before any lattice reasoning.
    if matches!(op, IrBinOp::Xor | IrBinOp::Sub) {
        if let (Operand::Value(a), Operand::Value(b)) = (lhs, rhs) {
            if a.0 == b.0 {
                return constant_fold::eval_binop_const(op, IrConst::I32(0), IrConst::I32(0), ty);
            }
        }
    }

    // Absorbing elements: one operand is a known 0 (Mul/And) or -1 (Or).
    let seed = match (lv, rv) {
        (LatticeVal::Constant(c), _) if is_integer_zero(c) || is_all_ones(c) => c,
        (_, LatticeVal::Constant(c)) if is_integer_zero(c) || is_all_ones(c) => c,
        _ => return None,
    };
    match op {
        IrBinOp::Mul | IrBinOp::And if is_integer_zero(seed) => {
            constant_fold::eval_binop_const(op, seed, seed, ty)
        }
        IrBinOp::Or if is_all_ones(seed) => constant_fold::eval_binop_const(op, seed, seed, ty),
        _ => None,
    }
}

fn evaluate_instruction(inst: &Instruction, block_idx: u32, state: &mut SccpState) {
    match inst {
        Instruction::Phi { dest, incoming, .. } => {
            let mut result = LatticeVal::Top;
            for (op, from_label) in incoming {
                match state.label_to_idx.get(from_label) {
                    Some(&from_idx) => {
                        if !state
                            .executable_edges
                            .contains(&edge_key(from_idx, block_idx))
                        {
                            continue;
                        }
                        // A self-reference contributes nothing new: on the
                        // iteration that carries it, the phi already holds the
                        // value being merged.
                        if let Operand::Value(v) = op {
                            if v.0 == dest.0 {
                                continue;
                            }
                        }
                        result = result.meet(resolve(op, state));
                    }
                    // The phi names a block that no longer exists. We cannot
                    // tell whether that edge is live, so assume the worst.
                    None => result = LatticeVal::Bottom,
                }
                if result.is_bottom() {
                    break;
                }
            }
            update_lattice(dest.0, result, state);
        }

        Instruction::Copy { dest, src } => {
            update_lattice(dest.0, resolve(src, state), state);
        }

        Instruction::BinOp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        } => {
            let (lv, rv) = (resolve(lhs, state), resolve(rhs, state));
            let r = match (lv, rv) {
                (LatticeVal::Constant(l), LatticeVal::Constant(r)) => {
                    lift(constant_fold::eval_binop_const(*op, l, r, *ty))
                }
                _ => match fold_despite_unknown(*op, lhs, rhs, lv, rv, *ty) {
                    Some(c) => LatticeVal::Constant(c),
                    // ⊥ dominates ⊤: once an operand is overdefined the result
                    // can never become a constant, so there is nothing to wait
                    // for. (Both orders are sound; this one converges faster.)
                    None if lv.is_bottom() || rv.is_bottom() => LatticeVal::Bottom,
                    None => LatticeVal::Top,
                },
            };
            update_lattice(dest.0, r, state);
        }

        Instruction::UnaryOp { dest, op, src, ty } => {
            let sv = resolve(src, state);
            if *op == IrUnaryOp::IsConstant {
                // `__builtin_constant_p` is phase-dependent, not value-
                // dependent. Answering "1" once the operand is proven constant
                // is monotone and always correct. Answering "0" here would be
                // premature: inlining and IPCP run later and can still turn the
                // operand into a constant, and the kernel gates whole code
                // paths on this. `constant_fold::resolve_remaining_is_constant`
                // settles the negative case at the end of the pipeline.
                let r = match sv {
                    LatticeVal::Constant(_) => LatticeVal::Constant(IrConst::I32(1)),
                    LatticeVal::Top => LatticeVal::Top,
                    LatticeVal::Bottom => LatticeVal::Bottom,
                };
                update_lattice(dest.0, r, state);
                return;
            }
            let cast_ty = match src {
                Operand::Value(v) => state.cast_to_ty.get(v.0 as usize).copied().flatten(),
                Operand::Const(_) => None,
            };
            let r = match sv {
                LatticeVal::Top => LatticeVal::Top,
                LatticeVal::Bottom => LatticeVal::Bottom,
                LatticeVal::Constant(c) => {
                    lift(constant_fold::eval_unaryop_const(*op, c, cast_ty, *ty))
                }
            };
            update_lattice(dest.0, r, state);
        }

        Instruction::Cmp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        } => {
            // Reflexive comparison: the two operands are the *same* SSA value,
            // so for integers every predicate has a fixed truth value
            // (Eq/Sle/Sge/Ule/Uge are true of `x cmp x`, the rest false),
            // whatever `x` may be. Floats are excluded: `x == x` is false
            // exactly when `x` is NaN. Like the same-value BinOp identities
            // this is a syntactic fact about SSA identity — the answer is the
            // IR's canonical boolean (`IrConst::I32(0 | 1)`, the same output
            // domain `eval_cmp_const` documents) and it is monotone.
            let r = if ty.is_integer()
                && matches!((lhs, rhs), (Operand::Value(a), Operand::Value(b)) if a.0 == b.0)
            {
                LatticeVal::Constant(IrConst::I32(matches!(
                    op,
                    IrCmpOp::Eq | IrCmpOp::Sle | IrCmpOp::Sge | IrCmpOp::Ule | IrCmpOp::Uge
                ) as i32))
            } else {
                match (resolve(lhs, state), resolve(rhs, state)) {
                    (LatticeVal::Bottom, _) | (_, LatticeVal::Bottom) => LatticeVal::Bottom,
                    (LatticeVal::Top, _) | (_, LatticeVal::Top) => LatticeVal::Top,
                    (LatticeVal::Constant(l), LatticeVal::Constant(r)) => {
                        lift(constant_fold::eval_cmp_const(*op, l, r, *ty))
                    }
                }
            };
            update_lattice(dest.0, r, state);
        }

        Instruction::Cast {
            dest,
            src,
            from_ty,
            to_ty,
        } => {
            let r = match resolve(src, state) {
                LatticeVal::Top => LatticeVal::Top,
                LatticeVal::Bottom => LatticeVal::Bottom,
                LatticeVal::Constant(c) => {
                    lift(constant_fold::eval_cast_const(c, *from_ty, *to_ty))
                }
            };
            update_lattice(dest.0, r, state);
        }

        Instruction::Select {
            dest,
            cond,
            true_val,
            false_val,
            ..
        } => {
            let r = match resolve(cond, state) {
                LatticeVal::Top => LatticeVal::Top,
                LatticeVal::Constant(c) => {
                    if c.is_nonzero() {
                        resolve(true_val, state)
                    } else {
                        resolve(false_val, state)
                    }
                }
                // Unknown condition: the result is whatever both arms agree on.
                LatticeVal::Bottom => resolve(true_val, state).meet(resolve(false_val, state)),
            };
            update_lattice(dest.0, r, state);
        }

        // ── Invariant (1) for `InlineAsm` ────────────────────────────────────
        // The outputs are definitions `dest()` does not cover (an asm can
        // define several), and they are opaque runtime values. Lowering them
        // here makes their opacity true by construction; the seed pass lowers
        // them a second time so ordering can never resurrect a `⊤`.
        Instruction::InlineAsm { outputs, .. } => {
            for (_, v, _) in outputs {
                update_lattice(v.0, LatticeVal::Bottom, state);
            }
        }

        // ── closed default (soundness invariant 1) ──────────────────────────
        // Everything else that defines a value is overdefined. This arm is
        // deliberately a catch-all keyed on `dest()` rather than an explicit
        // opcode list: a new IR opcode then defaults to ⊥ (a missed
        // optimisation) instead of staying ⊤ (a miscompile).
        other => {
            if let Some(dest) = other.dest() {
                update_lattice(dest.0, LatticeVal::Bottom, state);
            }
        }
    }
}

/// `Some(c)` folded to a constant; `None` means "not foldable" → overdefined.
#[inline]
fn lift(folded: Option<IrConst>) -> LatticeVal {
    match folded {
        Some(c) => LatticeVal::Constant(c),
        None => LatticeVal::Bottom,
    }
}

fn evaluate_terminator(term: &Terminator, block_idx: u32, state: &mut SccpState) {
    match term {
        Terminator::Branch(target) => {
            if let Some(&to) = state.label_to_idx.get(target) {
                mark_edge_executable(block_idx, to, state);
            }
        }

        Terminator::CondBranch {
            cond,
            true_label,
            false_label,
        } => match resolve(cond, state) {
            // Optimistic: an unevaluated condition marks neither successor.
            // Invariant (5) guarantees this cannot be the final state.
            LatticeVal::Top => {}
            LatticeVal::Constant(c) => {
                let taken = if c.is_nonzero() {
                    true_label
                } else {
                    false_label
                };
                if let Some(&to) = state.label_to_idx.get(taken) {
                    mark_edge_executable(block_idx, to, state);
                }
            }
            LatticeVal::Bottom => {
                for label in [true_label, false_label] {
                    if let Some(&to) = state.label_to_idx.get(label) {
                        mark_edge_executable(block_idx, to, state);
                    }
                }
            }
        },

        Terminator::Switch {
            val,
            cases,
            default,
            ..
        } => match resolve(val, state) {
            LatticeVal::Top => {}
            LatticeVal::Constant(c) => {
                let target = match c.to_i64() {
                    Some(v) => cases
                        .iter()
                        .find(|(case, _)| *case == v)
                        .map_or(default, |(_, label)| label),
                    // Non-integer switch value: malformed IR. Stay conservative
                    // rather than guessing the default arm.
                    None => {
                        mark_all_switch_targets(cases, default, block_idx, state);
                        return;
                    }
                };
                if let Some(&to) = state.label_to_idx.get(target) {
                    mark_edge_executable(block_idx, to, state);
                }
            }
            LatticeVal::Bottom => mark_all_switch_targets(cases, default, block_idx, state),
        },

        Terminator::IndirectBranch {
            possible_targets, ..
        } => {
            for label in possible_targets {
                if let Some(&to) = state.label_to_idx.get(label) {
                    mark_edge_executable(block_idx, to, state);
                }
            }
        }

        Terminator::Return(_) | Terminator::Unreachable => {}
    }
}

fn mark_all_switch_targets(
    cases: &[(i64, BlockId)],
    default: &BlockId,
    block_idx: u32,
    state: &mut SccpState,
) {
    for (_, label) in cases {
        if let Some(&to) = state.label_to_idx.get(label) {
            mark_edge_executable(block_idx, to, state);
        }
    }
    if let Some(&to) = state.label_to_idx.get(default) {
        mark_edge_executable(block_idx, to, state);
    }
}

#[inline]
fn mark_edge_executable(from: u32, to: u32, state: &mut SccpState) {
    if state.executable_edges.insert(edge_key(from, to)) {
        state.cfg_worklist.push(to);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Undef resolution (soundness invariant 5)
// ═══════════════════════════════════════════════════════════════════════════

/// Force `id` from `⊤` to `⊥`. Returns whether it moved.
#[inline]
fn force_bottom(id: u32, state: &mut SccpState) -> bool {
    match state.lattice.get(id as usize) {
        Some(v) if v.is_top() => {
            update_lattice(id, LatticeVal::Bottom, state);
            true
        }
        _ => false,
    }
}

/// Lower every `⊤` that is still visible inside an executable block to `⊥`.
///
/// Covers the destinations of all instructions, the operands of all non-phi
/// instructions and terminators, and phi operands arriving on *executable*
/// edges. Phi operands on non-executable edges are deliberately left at `⊤`:
/// they are not observed, and if a later round makes their edge executable
/// their definition will be evaluated normally then, with full precision.
///
/// Returns whether anything moved (i.e. whether the solver must run again).
/// Terminators whose condition was forced are re-queued explicitly so the
/// re-evaluation does not depend on the use-def table being complete.
fn force_unresolved_to_bottom(func: &IrFunction, state: &mut SccpState) -> bool {
    let mut forced = false;
    for (bi, block) in func.blocks.iter().enumerate() {
        if !state.block_executable[bi] {
            continue;
        }
        let bi32 = bi as u32;
        for inst in &block.instructions {
            if let Instruction::Phi { dest, incoming, .. } = inst {
                for (op, from_label) in incoming {
                    let Operand::Value(v) = op else { continue };
                    let Some(&from_idx) = state.label_to_idx.get(from_label) else {
                        continue; // unknown pred: the phi already evaluates to ⊥
                    };
                    if state.executable_edges.contains(&edge_key(from_idx, bi32)) {
                        forced |= force_bottom(v.0, state);
                    }
                }
                forced |= force_bottom(dest.0, state);
                continue;
            }
            inst.for_each_used_value(|id| {
                forced |= force_bottom(id, state);
            });
            if let Some(d) = inst.dest() {
                forced |= force_bottom(d.0, state);
            }
        }
        let mut term_forced = false;
        block.terminator.for_each_used_value(|id| {
            term_forced |= force_bottom(id, state);
        });
        if term_forced {
            forced = true;
            // Block is already executable, so this re-runs `visit_phis`, which
            // re-evaluates the terminator with the now-⊥ condition.
            state.cfg_worklist.push(bi32);
        }
    }
    forced
}

// ═══════════════════════════════════════════════════════════════════════════
// Rewrite
// ═══════════════════════════════════════════════════════════════════════════

/// Per-action diagnostic gates. SCCP performs independent rewrites (edge
/// pruning, definition materialisation, operand substitution, terminator
/// folding); when a workload miscompiles, bisecting them is far faster than
/// bisecting the whole pass. Documented in `engineering/SCCP_ADOPTION_AUDIT.md`.
///
/// ```text
///   CCC_SCCP_NO_PRUNE     keep phi operands for non-executable edges
///   CCC_SCCP_NO_DEFS      do not rewrite definitions to Copy{dest, Const}
///   CCC_SCCP_NO_SUBST     do not substitute constants into operands
///   CCC_SCCP_NO_FOLD      do not fold CondBranch/Switch to Branch
///   CCC_SCCP_TRACE_PRUNE  print every pruned phi operand
/// ```
///
/// Read once per process: five environment lookups per function per pass
/// invocation (lock + hash + `OsString` allocation each) is a measurable
/// self-host compile-time tax for zero benefit.
struct RewriteGates {
    no_prune: bool,
    no_defs: bool,
    no_subst: bool,
    no_fold: bool,
    trace_prune: bool,
}

fn gates() -> &'static RewriteGates {
    static GATES: std::sync::OnceLock<RewriteGates> = std::sync::OnceLock::new();
    GATES.get_or_init(|| {
        let on = |k: &str| std::env::var_os(k).is_some();
        RewriteGates {
            no_prune: on("CCC_SCCP_NO_PRUNE"),
            no_defs: on("CCC_SCCP_NO_DEFS"),
            no_subst: on("CCC_SCCP_NO_SUBST"),
            no_fold: on("CCC_SCCP_NO_FOLD"),
            trace_prune: on("CCC_SCCP_TRACE_PRUNE"),
        }
    })
}

/// Opcodes whose destination the rewrite is allowed to overwrite with a
/// `Copy`. Enforcing this explicitly (rather than trusting that only pure
/// opcodes can reach `Constant`) means a future lattice extension that starts
/// folding, say, a pure intrinsic cannot accidentally delete a call.
#[inline]
fn is_pure_replaceable(inst: &Instruction) -> bool {
    matches!(
        inst,
        Instruction::BinOp { .. }
            | Instruction::UnaryOp { .. }
            | Instruction::Cmp { .. }
            | Instruction::Cast { .. }
            | Instruction::Copy { .. }
            | Instruction::Select { .. }
            | Instruction::Phi { .. }
    )
}

fn rewrite(func: &mut IrFunction, state: &SccpState) -> SccpStats {
    let mut stats = SccpStats::default();
    let g = gates();

    // ── 1. decide the surviving successor set of every executable block ─────
    // This must happen before any mutation, because step 2 prunes phi operands
    // against it and step 3 installs the folded terminators.
    let mut folded_term: Vec<Option<Terminator>> = vec![None; func.blocks.len()];
    let mut live_edges: FxHashSet<u64> = FxHashSet::default();
    // Every edge that actually exists in the CFG as written, independent of
    // reachability. Used to distinguish "this edge is dead" (prunable) from
    // "this label was never a predecessor at all" (malformed IR, not ours to
    // repair) -- see the pruning loop below.
    let mut real_edges: FxHashSet<u64> = FxHashSet::default();

    for (bi, block) in func.blocks.iter().enumerate() {
        let bi32 = bi as u32;
        let executable = state.block_executable[bi];
        if !executable {
            stats.unreachable_blocks += 1;
        }

        // `asm goto` edges are real, and when the block executes they always
        // survive: they are not conditional on any value this pass can reason
        // about.
        for inst in &block.instructions {
            if let Instruction::InlineAsm { goto_labels, .. } = inst {
                for (_, label) in goto_labels {
                    if let Some(&to) = state.label_to_idx.get(label) {
                        let k = edge_key(bi32, to);
                        real_edges.insert(k);
                        if executable {
                            live_edges.insert(k);
                        }
                    }
                }
            }
        }

        for_each_target(&block.terminator, |label| {
            if let Some(&to) = state.label_to_idx.get(&label) {
                real_edges.insert(edge_key(bi32, to));
            }
        });
        if !executable {
            continue;
        }

        let new_term = if g.no_fold {
            None
        } else {
            fold_terminator(&block.terminator, state)
        };
        let effective = new_term.as_ref().unwrap_or(&block.terminator);
        for_each_target(effective, |label| {
            if let Some(&to) = state.label_to_idx.get(&label) {
                live_edges.insert(edge_key(bi32, to));
            }
        });
        if new_term.is_some() {
            stats.branches_folded += 1;
        }
        folded_term[bi] = new_term;
    }

    // ── 2. prune phi operands whose edge is gone ────────────────────────────
    //
    // Two sources of dead edges are handled together:
    //   * a predecessor block SCCP proved unreachable, and
    //   * a surviving predecessor whose branch we are about to fold away.
    //
    // Leaving either behind is a real miscompile, not a cosmetic issue: phi
    // elimination materialises one copy per listed predecessor edge, so a
    // stale entry writes the wrong value into the phi's register on a path
    // that is still live. `cfg_simplify::fold_constant_cond_branches` performs
    // the same repair for its own folds and documents the same hazard.
    //
    // Note on `CCC_SCCP_NO_FOLD`: an SCCP-unreachable block stays CFG-reachable
    // when its predecessor's branch is not folded, but it is still *dynamically*
    // unreachable (the branch condition is a proven constant), so pruning its
    // out-edges from live phis remains semantically correct.
    if !g.no_prune {
        let func_name_dbg = if g.trace_prune {
            func.name.clone()
        } else {
            String::new()
        };
        for bi in 0..func.blocks.len() {
            if !state.block_executable[bi] {
                continue;
            }
            let bi32 = bi as u32;
            // Keep an incoming entry iff its edge is live, or it is not a real
            // CFG edge at all. Prune only edges SCCP *proved* dead: the edge
            // must really exist in the CFG and have been shown non-executable.
            // A label that names a block which is not a predecessor of this
            // block at all is malformed IR (loop_rotate produced exactly this
            // shape before the stale-guard-label fix). Dropping such an entry
            // silently deletes an initialisation, so keep it and let
            // cfg_simplify's stale-label cleanup deal with it. An unknown label
            // is likewise not ours to judge.
            let keep = |from_label: &BlockId| -> bool {
                match state.label_to_idx.get(from_label) {
                    Some(&from) => {
                        let k = edge_key(from, bi32);
                        live_edges.contains(&k) || !real_edges.contains(&k)
                    }
                    None => true,
                }
            };
            for inst in func.blocks[bi].instructions.iter_mut() {
                let Instruction::Phi { incoming, .. } = inst else {
                    continue;
                };
                let before = incoming.len();
                let survivors = incoming.iter().filter(|(_, l)| keep(l)).count();

                // Defensive: an executable block always has at least one live
                // incoming edge, so an empty survivor set means our edge model
                // and the phi disagree. Keeping the phi unchanged is always
                // safe; emitting a zero-operand phi would not be.
                if survivors == 0 || survivors == before {
                    debug_assert!(
                        survivors != 0 || bi == 0,
                        "phi in executable block {} lost every incoming edge",
                        bi
                    );
                    continue;
                }
                incoming.retain(|(op, from_label)| {
                    let k = keep(from_label);
                    if !k && g.trace_prune {
                        let fi = state.label_to_idx.get(from_label).copied();
                        eprintln!(
                            "[sccp] fn {:?} prune: block#{bi} drops incoming ({:?} from {:?}) predidx={:?} pred_exec={:?}",
                            func_name_dbg,
                            op,
                            from_label,
                            fi,
                            fi.map(|i| state.block_executable[i as usize])
                        );
                    }
                    k
                });
                stats.phi_edges_pruned += before - incoming.len();
            }
        }
    }

    // ── 3. materialise constants and install folded terminators ─────────────
    for bi in 0..func.blocks.len() {
        if !state.block_executable[bi] {
            // Unreachable: leave the body alone. Its values are ⊤ and are never
            // materialised, and cfg_simplify removes the block once the folded
            // branches above make it CFG-unreachable.
            continue;
        }

        if !g.no_defs {
            materialise_constant_defs(&mut func.blocks[bi], state, &mut stats);
        }

        // Substitute constants into every operand position. `for_each_operand_mut`
        // is the canonical exhaustive visitor and only ever yields `Operand`
        // fields, never the bare `Value` fields (Load.ptr, GEP.base, ...) that
        // cannot legally hold a constant — so this cannot construct invalid IR.
        if !g.no_subst {
            for inst in func.blocks[bi].instructions.iter_mut() {
                inst.for_each_operand_mut(|op| stats.operands_substituted += substitute(op, state));
            }
            func.blocks[bi]
                .terminator
                .for_each_operand_mut(|op| stats.operands_substituted += substitute(op, state));
        }

        if let Some(term) = folded_term[bi].take() {
            func.blocks[bi].terminator = term;
        }
    }

    stats
}

/// Replace each definition proven constant with `Copy { dest, Const }`.
///
/// Phis are handled separately to preserve the "all phis first" block layout
/// that `loop_rotate` and the backend rely on: the replacement `Copy` is
/// spliced in immediately after the phi prefix instead of in place.
fn materialise_constant_defs(block: &mut BasicBlock, state: &SccpState, stats: &mut SccpStats) {
    // Phase A: non-phi definitions, replaced in place.
    for inst in block.instructions.iter_mut() {
        if matches!(inst, Instruction::Phi { .. }) {
            continue;
        }
        let Some(dest) = inst.dest() else { continue };
        let Some(c) = lattice_const(state, dest) else {
            continue;
        };
        if !is_pure_replaceable(inst) {
            // Soundness invariant (4). Only reachable if a transfer function
            // starts proving a side-effecting opcode constant without teaching
            // the rewrite how to preserve its effect.
            debug_assert!(
                false,
                "SCCP proved a side-effecting instruction constant: {inst:?}"
            );
            continue;
        }
        if matches!(inst, Instruction::Copy { src: Operand::Const(existing), .. } if existing.to_hash_key() == c.to_hash_key())
        {
            continue; // already materialised; do not report a phantom change
        }
        *inst = Instruction::Copy {
            dest,
            src: Operand::Const(c),
        };
        stats.defs_materialised += 1;
    }

    // Phase B: constant phis become Copies placed after the phi prefix.
    // One stable `retain` over the prefix plus one `splice`: O(n) regardless of
    // how many phis fold (the previous remove-in-a-loop was O(phis²)).
    let phi_prefix = block
        .instructions
        .iter()
        .position(|i| !matches!(i, Instruction::Phi { .. }))
        .unwrap_or(block.instructions.len());
    if phi_prefix == 0 {
        return;
    }

    let mut promoted: Vec<Instruction> = Vec::new();
    let mut idx = 0usize;
    block.instructions.retain(|inst| {
        let i = idx;
        idx += 1;
        if i >= phi_prefix {
            return true;
        }
        let Instruction::Phi { dest, .. } = inst else {
            return true;
        };
        match lattice_const(state, *dest) {
            Some(c) => {
                promoted.push(Instruction::Copy {
                    dest: *dest,
                    src: Operand::Const(c),
                });
                false
            }
            None => true,
        }
    });
    if !promoted.is_empty() {
        stats.defs_materialised += promoted.len();
        // The surviving phis now occupy exactly [0, phi_prefix - promoted.len()).
        let insert_at = phi_prefix - promoted.len();
        block.instructions.splice(insert_at..insert_at, promoted);
    }
}

#[inline]
fn substitute(op: &mut Operand, state: &SccpState) -> usize {
    let Operand::Value(v) = op else { return 0 };
    match lattice_const(state, *v) {
        Some(c) => {
            *op = Operand::Const(c);
            1
        }
        None => 0,
    }
}

/// The folded form of `term`, or `None` when it cannot be folded.
fn fold_terminator(term: &Terminator, state: &SccpState) -> Option<Terminator> {
    match term {
        Terminator::CondBranch {
            cond,
            true_label,
            false_label,
        } => {
            let c = const_of_operand(cond, state)?;
            Some(Terminator::Branch(if c.is_nonzero() {
                *true_label
            } else {
                *false_label
            }))
        }
        Terminator::Switch {
            val,
            cases,
            default,
            ..
        } => {
            let v = const_of_operand(val, state)?.to_i64()?;
            let target = cases
                .iter()
                .find(|(case, _)| *case == v)
                .map_or(*default, |(_, label)| *label);
            Some(Terminator::Branch(target))
        }
        _ => None,
    }
}

#[inline]
fn const_of_operand(op: &Operand, state: &SccpState) -> Option<IrConst> {
    match op {
        Operand::Const(c) => Some(*c),
        Operand::Value(v) => lattice_const(state, *v),
    }
}

/// Every block label a terminator can transfer control to.
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

#[cfg(test)]
mod tests;
