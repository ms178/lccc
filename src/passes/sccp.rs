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
//!   or `(signed char)200` evaluates to.
//! * **Materialisation** follows the codebase idiom: a value proven constant
//!   has its *defining instruction* rewritten to `Copy { dest, Const }`, and
//!   its uses substituted in place. `copy_prop`/`dce` then clean up.
//! * **CFG cleanup** is left to `cfg_simplify`, but the edges this pass kills
//!   are repaired here (see "Edge maintenance" below) so the IR is well-formed
//!   at every point in between.
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

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IrConst, IrFunction, IrUnaryOp, Operand, Terminator, Value,
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
    let num_values = usedef.len().max(func.max_value_id() as usize + 1);

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
    // Terminates because both worklists are only fed by strictly downward
    // lattice moves (`update_lattice`) and by first-time edge insertions
    // (`mark_edge_executable`). Both are finite and monotone.
    state.cfg_worklist.push(0);
    while !state.cfg_worklist.is_empty() || !state.ssa_worklist.is_empty() {
        while let Some(block_idx) = state.cfg_worklist.pop() {
            let bi = block_idx as usize;
            if bi >= num_blocks {
                continue;
            }
            if !state.block_executable[bi] {
                state.block_executable[bi] = true;
                visit_block(func, block_idx, &mut state);
            } else {
                // Already visited; a *new* incoming edge can only change phis
                // (and, through them, the terminator).
                visit_phis(func, block_idx, &mut state);
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
                    evaluate_terminator(&func.blocks[bi].terminator, loc.block_idx, &mut state);
                } else if let Some(inst) = func.blocks[bi].instructions.get(loc.inst_idx as usize) {
                    evaluate_instruction(inst, loc.block_idx, &mut state);
                }
            }
        }
    }

    rewrite(func, &state)
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
                    state.lattice[dest.0 as usize] = LatticeVal::Bottom;
                }
                Instruction::Cast { dest, to_ty, .. } => {
                    state.cast_to_ty[dest.0 as usize] = Some(*to_ty);
                }
                _ => {}
            }
        }
    }

    // A value that is read but never defined is a dangling reference left by an
    // earlier pass. It must be ⊥: leaving it ⊤ would let it be absorbed by a
    // phi meet and turn an unknown into a fabricated constant.
    for v in 0..num_values {
        if usedef.use_count[v] > 0 && usedef.def_of(v as u32).is_none() {
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
            let r = match (resolve(lhs, state), resolve(rhs, state)) {
                (LatticeVal::Bottom, _) | (_, LatticeVal::Bottom) => LatticeVal::Bottom,
                (LatticeVal::Top, _) | (_, LatticeVal::Top) => LatticeVal::Top,
                (LatticeVal::Constant(l), LatticeVal::Constant(r)) => {
                    lift(constant_fold::eval_binop_const(*op, l, r, *ty))
                }
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
            let r = match (resolve(lhs, state), resolve(rhs, state)) {
                (LatticeVal::Bottom, _) | (_, LatticeVal::Bottom) => LatticeVal::Bottom,
                (LatticeVal::Top, _) | (_, LatticeVal::Top) => LatticeVal::Top,
                (LatticeVal::Constant(l), LatticeVal::Constant(r)) => {
                    lift(constant_fold::eval_cmp_const(*op, l, r, *ty))
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
// Rewrite
// ═══════════════════════════════════════════════════════════════════════════

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

    // Per-action diagnostic gates. SCCP performs three independent rewrites
    // (edge pruning, definition materialisation, terminator folding); when a
    // workload miscompiles, bisecting them is far faster than bisecting the
    // whole pass. Documented in `engineering/SCCP_ADOPTION_AUDIT.md`.
    //
    //   CCC_SCCP_NO_PRUNE   keep phi operands for non-executable edges
    //   CCC_SCCP_NO_DEFS    do not rewrite definitions to Copy{dest, Const}
    //   CCC_SCCP_NO_SUBST   do not substitute constants into operands
    //   CCC_SCCP_NO_FOLD    do not fold CondBranch/Switch to Branch
    let no_prune = std::env::var_os("CCC_SCCP_NO_PRUNE").is_some();
    let no_defs = std::env::var_os("CCC_SCCP_NO_DEFS").is_some();
    let no_subst = std::env::var_os("CCC_SCCP_NO_SUBST").is_some();
    let no_fold = std::env::var_os("CCC_SCCP_NO_FOLD").is_some();

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
        for_each_target(&block.terminator, |label| {
            if let Some(&to) = state.label_to_idx.get(&label) {
                real_edges.insert(edge_key(bi32, to));
            }
        });
        for inst in &block.instructions {
            if let Instruction::InlineAsm { goto_labels, .. } = inst {
                for (_, label) in goto_labels {
                    if let Some(&to) = state.label_to_idx.get(label) {
                        real_edges.insert(edge_key(bi32, to));
                    }
                }
            }
        }
    }

    for (bi, block) in func.blocks.iter().enumerate() {
        if !state.block_executable[bi] {
            stats.unreachable_blocks += 1;
            continue;
        }
        let bi32 = bi as u32;

        // `asm goto` edges always survive: they are not conditional on any
        // value this pass can reason about.
        for inst in &block.instructions {
            if let Instruction::InlineAsm { goto_labels, .. } = inst {
                for (_, label) in goto_labels {
                    if let Some(&to) = state.label_to_idx.get(label) {
                        live_edges.insert(edge_key(bi32, to));
                    }
                }
            }
        }

        let new_term = if no_fold {
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
    let trace_prune = std::env::var_os("CCC_SCCP_TRACE_PRUNE").is_some();
    let func_name_dbg = if trace_prune {
        func.name.clone()
    } else {
        String::new()
    };
    for bi in 0..func.blocks.len() {
        if no_prune || !state.block_executable[bi] {
            continue;
        }
        let bi32 = bi as u32;
        for inst in func.blocks[bi].instructions.iter_mut() {
            let Instruction::Phi { incoming, .. } = inst else {
                continue;
            };
            let before = incoming.len();
            let survivors: Vec<_> = incoming
                .iter()
                .filter(|(_, from_label)| match state.label_to_idx.get(from_label) {
                    Some(&from) => {
                        let k = edge_key(from, bi32);
                        // Prune only edges SCCP *proved* dead: the edge must
                        // really exist in the CFG and have been shown
                        // non-executable. A label that names a block which is
                        // not a predecessor of this block at all is malformed
                        // IR (loop_rotate produced exactly this shape before
                        // the stale-guard-label fix). Dropping such an entry
                        // silently deletes an initialisation, so keep it and
                        // let cfg_simplify's stale-label cleanup deal with it.
                        live_edges.contains(&k) || !real_edges.contains(&k)
                    }
                    // Unknown predecessor label: not ours to judge.
                    None => true,
                })
                .cloned()
                .collect();

            // Defensive: an executable block always has at least one live
            // incoming edge, so an empty survivor set means our edge model and
            // the phi disagree. Keeping the phi unchanged is always safe;
            // emitting a zero-operand phi would not be.
            if survivors.is_empty() || survivors.len() == before {
                debug_assert!(
                    !survivors.is_empty() || bi == 0,
                    "phi in executable block {} lost every incoming edge",
                    bi
                );
                continue;
            }
            if trace_prune {
                for (op, fl) in incoming.iter() {
                    let keep = survivors.iter().any(|(_, l)| l == fl);
                    if !keep {
                        let fi = state.label_to_idx.get(fl).copied();
                        eprintln!(
                            "[sccp] fn prune: block#{bi} label={:?} drops incoming ({:?} from {:?}) predidx={:?} pred_exec={:?}",
                            func_name_dbg, op, fl, fi,
                            fi.map(|i| state.block_executable[i as usize])
                        );
                    }
                }
            }
            stats.phi_edges_pruned += before - survivors.len();
            *incoming = survivors;
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

        if !no_defs {
            materialise_constant_defs(&mut func.blocks[bi], state, &mut stats);
        }

        // Substitute constants into every operand position. `for_each_operand_mut`
        // is the canonical exhaustive visitor and only ever yields `Operand`
        // fields, never the bare `Value` fields (Load.ptr, GEP.base, ...) that
        // cannot legally hold a constant — so this cannot construct invalid IR.
        if !no_subst {
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
        let Some(c) = state
            .lattice
            .get(dest.0 as usize)
            .copied()
            .and_then(LatticeVal::as_const)
        else {
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
    let phi_prefix = block
        .instructions
        .iter()
        .position(|i| !matches!(i, Instruction::Phi { .. }))
        .unwrap_or(block.instructions.len());

    let mut promoted: Vec<Instruction> = Vec::new();
    let mut idx = 0;
    while idx < block.instructions.len().min(phi_prefix) {
        let Instruction::Phi { dest, .. } = block.instructions[idx] else {
            idx += 1;
            continue;
        };
        match state
            .lattice
            .get(dest.0 as usize)
            .copied()
            .and_then(LatticeVal::as_const)
        {
            Some(c) => {
                block.instructions.remove(idx);
                promoted.push(Instruction::Copy {
                    dest,
                    src: Operand::Const(c),
                });
                stats.defs_materialised += 1;
                // `idx` now indexes the next instruction; do not advance.
            }
            None => idx += 1,
        }
    }
    if !promoted.is_empty() {
        let insert_at = block
            .instructions
            .iter()
            .position(|i| !matches!(i, Instruction::Phi { .. }))
            .unwrap_or(block.instructions.len());
        block.instructions.splice(insert_at..insert_at, promoted);
    }
}

#[inline]
fn substitute(op: &mut Operand, state: &SccpState) -> usize {
    let Operand::Value(v) = op else { return 0 };
    match state
        .lattice
        .get(v.0 as usize)
        .copied()
        .and_then(LatticeVal::as_const)
    {
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
        Operand::Value(v) => state
            .lattice
            .get(v.0 as usize)
            .copied()
            .and_then(LatticeVal::as_const),
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

/// Silence the unused-import warning for `Value` in builds without tests while
/// keeping the import available to the test module's IR constructors.
#[allow(dead_code)]
fn _value_type_is_used(v: Value) -> u32 {
    v.0
}

#[cfg(test)]
mod tests;
