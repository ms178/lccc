//! The affine exit-compare fold (AFFOLD): `iv + C <op> N` becomes
//! `iv <op> N - C` wherever the loop's own induction sequence proves the
//! rewrite exact.
//!
//! Two producers share the proof machinery in this file:
//!
//! * [`fold_affine_exit_compares`], the standalone pass over every natural
//!   loop of a function (default-on at -O2+ and on the size pipelines);
//! * [`canonicalise_affine_exit_cmps`], applied by loop rotation to the copy
//!   of the exit test it places at the bottom of a rotated loop.
//!
//! `CCC_NO_AFFINE_EXIT_FOLD` turns both off and `CCC_DEBUG_AFFINE_FOLD`
//! reports both; the two knobs are resolved once in `run_passes` and handed
//! over through [`set_affine_fold_knobs`].
//!
//! The file was split out of `loop_rotate.rs`, which is why the paths other
//! passes use (`loop_rotate::integer_type_bounds`, ...) are re-exported from
//! the parent module unchanged.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::analysis::CfgAnalysis;
use crate::ir::reexports::{
    BlockId, Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Terminator, Value,
};
use crate::passes::loop_analysis::{NaturalLoop, find_natural_loops, merge_loops_by_header};

/// Returns true if `block_idx` is dominated by `dom_idx` — i.e. every path
/// from the function entry to `block_idx` passes through `dom_idx`. Walks the
/// `idom` chain: `dom_idx` must be an ancestor of `block_idx` in the dominator
/// tree. `block_idx == dom_idx` returns true (a block dominates itself). A
/// block no path reaches has no immediate dominator and is dominated by
/// nothing here.
///
/// Complexity: O(depth of dom tree) — bounded by `num_blocks`. The walk
/// terminates because `idom[entry] == entry` (the entry block is its own
/// immediate dominator), so the chain always reaches a fixed point.
fn is_dominated_by(block_idx: usize, dom_idx: usize, cfg: &CfgAnalysis) -> bool {
    if block_idx == dom_idx {
        return true;
    }
    let mut cur = block_idx;
    // Bounded by num_blocks: the idom chain is at most num_blocks deep (a
    // degenerate chain that visits every block). Defensive guard against a
    // pathological cycle (shouldn't happen — idom is a tree — but a corrupt
    // CfgAnalysis could loop forever without this).
    for _ in 0..cfg.num_blocks {
        match cfg.idom.get(cur).copied() {
            Some(p) if p == dom_idx => return true,
            Some(p) if p == cur => return false, // reached root without finding dom_idx
            Some(p) => cur = p,
            None => return false, // block_idx out of range (shouldn't happen)
        }
    }
    false // dom chain longer than num_blocks — corrupt CfgAnalysis, fail closed
}

fn signed_type_bounds(ty: IrType) -> Option<(i64, i64)> {
    match ty {
        IrType::I8 => Some((i8::MIN as i64, i8::MAX as i64)),
        IrType::I16 => Some((i16::MIN as i64, i16::MAX as i64)),
        IrType::I32 => Some((i32::MIN as i64, i32::MAX as i64)),
        IrType::I64 => Some((i64::MIN, i64::MAX)),
        _ => None,
    }
}

/// ZERO-ROT-AFFINE: canonicalise the CLONED exit comparison
/// `icmp slt (add iv, C), N` -> `icmp slt iv, (N - C)`.
///
/// Rotation re-tests the loop condition in the latch, on the values the latch
/// just produced (`i_next`), so the cloned condition is
/// `icmp slt (add i_next, 4), 2048` and the backend materialises the `+4` as a
/// fresh `leaq`-style temporary before every compare -- the shape the
/// FOLLOWUP-2026-09-30-affine-exit-compare blocker measured at 5 instructions
/// per iteration against GCC's 4.  Folding the constant into the bound removes
/// the temporary AND gives the compare-branch fusion a register compare it can
/// fuse on the back edge (the rotation's licence).
///
/// Scope and justification:
/// * only freshly cloned temporaries whose ONLY use is the cloned compare are
///   rewritten (`use_count` is computed over `cloned`); the header's original
///   `add`/`cmp` pair is untouched, so nothing outside the rotated latch
///   changes;
/// * signed comparisons only, and only the cloned guard whose identity the
///   caller supplies after proving the original loop's executed IV range and
///   actual guard nonwrapping. Signed source types alone prove nothing: the
///   IR has no `nsw` flag and also represents `-fwrapv` and casted unsigned
///   arithmetic. Missing proof or a different comparison means refusal;
/// * the folded bound must be REPRESENTABLE in the comparison's type
///   (`checked_sub` + `signed_type_bounds`), so no wrap is ever introduced;
/// * constants may appear on either side of the `add` and the affine operand
///   may be on either side of the compare (`N > i + C` folds too).  The
///   comparison OPERATOR is preserved, never mirrored: mirroring the operand
///   side requires negating the relation as well, and taking one without the
///   other is exactly the double-negation bug the orientation oracle pins
///   (`N op (i + C) == (N - C) op i`, operator unchanged).
pub(super) fn canonicalise_affine_exit_cmps(
    cloned: &mut [Instruction],
    proved_cmp: Option<Value>,
) -> usize {
    let Some(proved_cmp) = proved_cmp else {
        return 0;
    };
    // Uses of each cloned dest inside the cloned slice (fresh IDs cannot be
    // referenced anywhere else, so this is the function-wide count).
    let mut use_count: FxHashMap<u32, usize> = FxHashMap::default();
    for inst in cloned.iter() {
        inst.for_each_used_value(|v| *use_count.entry(v).or_default() += 1);
    }
    // Cloned `add iv, C` (or `add C, iv`) temporaries.
    let mut affine: FxHashMap<u32, (Operand, i64, IrType)> = FxHashMap::default();
    for inst in cloned.iter() {
        let Instruction::BinOp {
            dest,
            op: IrBinOp::Add,
            lhs,
            rhs,
            ty,
        } = inst
        else {
            continue;
        };
        let pair = match (lhs, rhs) {
            (Operand::Value(v), Operand::Const(c)) | (Operand::Const(c), Operand::Value(v)) => {
                c.to_i64().map(|c| (*v, c))
            }
            _ => None,
        };
        if let Some((v, c)) = pair {
            affine.insert(dest.0, (Operand::Value(v), c, *ty));
        }
    }

    let mut folded = 0usize;
    for inst in cloned.iter_mut() {
        let Instruction::Cmp {
            dest,
            lhs,
            op,
            rhs,
            ty,
        } = inst
        else {
            continue;
        };
        if *dest != proved_cmp {
            continue;
        }
        let (lhs_snapshot, rhs_snapshot) = (lhs.clone(), rhs.clone());
        let is_foldable = |v: Value| use_count.get(&v.0) == Some(&1) && affine.contains_key(&v.0);
        let (affine_val, flipped) = match (&lhs_snapshot, &rhs_snapshot) {
            (Operand::Value(v), _) if is_foldable(*v) => (*v, false),
            (_, Operand::Value(v)) if is_foldable(*v) => (*v, true),
            _ => continue,
        };
        // Signed order only -- see the doc comment.  The OPERATOR DOES NOT
        // CHANGE in either orientation: `iv + C op N` is `iv op N - C`, and
        // `N op (iv + C)` is `(N - C) op iv` -- subtracting the same constant
        // from both sides is what preserves the relation, and it is also why
        // the rewrite is exact rather than approximate.  (An earlier revision
        // mirrored the operator in the flipped case while ALSO swapping the
        // operands; two negations do not cancel, so `N > iv + C` came out as
        // `(N - C) < iv` -- for N=100, C=4, iv=0 that turned the true `100 > 4`
        // into the false `96 < 0`.  Pinned by `affine_fold_orientation_tests`.)
        let folded_op = match op {
            IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge => *op,
            _ => continue,
        };
        let Some((iv_op, c, add_ty)) = affine.get(&affine_val.0) else {
            continue;
        };
        if add_ty != ty {
            continue; // the add's width is what makes the constant affine
        }
        let Some((lo, hi)) = signed_type_bounds(*ty) else {
            continue;
        };
        let bound_op = if flipped {
            &lhs_snapshot
        } else {
            &rhs_snapshot
        };
        let Operand::Const(bound_c) = bound_op else {
            continue; // only an invariant constant bound is foldable here
        };
        let Some(bound) = bound_c.to_i64() else {
            continue;
        };
        let Some(new_bound) = bound.checked_sub(*c) else {
            continue; // would wrap: skip
        };
        if new_bound < lo || new_bound > hi {
            continue; // not representable in the comparison's type: skip
        }
        let new_bound_c = IrConst::from_i64(new_bound, *ty);
        if flipped {
            *lhs = Operand::Const(new_bound_c);
            *rhs = iv_op.clone();
        } else {
            *lhs = iv_op.clone();
            *rhs = Operand::Const(new_bound_c);
        }
        *op = folded_op;
        folded += 1;
    }
    folded
}

/// Mathematical domain for the sequence proof. U64's upper endpoint does
/// not fit in i64; use i128 until the final typed constant is encoded.
pub(crate) fn integer_type_bounds(ty: IrType) -> Option<(i128, i128)> {
    if !ty.is_integer() || !matches!(ty.size(), 1 | 2 | 4 | 8) {
        return None;
    }
    let bits = ty.size() * 8;
    Some(if ty.is_unsigned() {
        (0, (1_i128 << bits) - 1)
    } else {
        (-(1_i128 << (bits - 1)), (1_i128 << (bits - 1)) - 1)
    })
}

pub(crate) fn integer_constant(c: IrConst, ty: IrType) -> Option<i128> {
    integer_type_bounds(ty)?;
    let bits = ty.size() * 8;
    let value = c.to_i128()? & ((1_i128 << bits) - 1);
    Some(if !ty.is_unsigned() && value >= 1_i128 << (bits - 1) {
        value - (1_i128 << bits)
    } else {
        value
    })
}

fn comparison_matches_domain(op: IrCmpOp, ty: IrType) -> bool {
    if ty.is_unsigned() {
        matches!(
            op,
            IrCmpOp::Ult | IrCmpOp::Ule | IrCmpOp::Ugt | IrCmpOp::Uge
        )
    } else {
        matches!(
            op,
            IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge
        )
    }
}

// The fold's two switches, `CCC_NO_AFFINE_EXIT_FOLD` and
// `CCC_DEBUG_AFFINE_FOLD`: read from the environment once per translation
// unit by `run_passes` and handed over through `set_affine_fold_knobs`.
thread_local! {
    static AFFINE_FOLD_ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    static AFFINE_FOLD_DEBUG: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(crate) fn set_affine_fold_knobs(enabled: bool, debug: bool) {
    AFFINE_FOLD_ENABLED.with(|cell| cell.set(enabled));
    AFFINE_FOLD_DEBUG.with(|cell| cell.set(debug));
}

pub(crate) fn affine_fold_enabled() -> bool {
    AFFINE_FOLD_ENABLED.with(|cell| cell.get())
}

pub(crate) fn affine_fold_debug() -> bool {
    AFFINE_FOLD_DEBUG.with(|cell| cell.get())
}

// ── AFFOLD: the affine exit-compare fold ────────────────────────────────────

/// AFFOLD phase entry point: fold `iv + C <op> N` into `iv <op> N - C`
/// wherever the proof below holds, for every natural loop of `func`.
///
/// Why this is its own pass rather than a step of rotation: `canonicalise_
/// affine_exit_cmps` only ever sees the copy of the exit test that rotation
/// makes, and rotation takes a loop only where it pays (see `loop_rotate`) --
/// while every filter kernel's hot loop wants the fold.  The rewrite needs no
/// CFG surgery (it is a value equivalence), so it runs on every loop,
/// default-on at -O2+ and on the size pipelines, and reports through
/// `CCC_DEBUG_AFFINE_FOLD`; kill switch `CCC_NO_AFFINE_EXIT_FOLD`.
///
/// The rewrite trades an `add`-plus-`cmp` for the same `cmp` against a
/// pre-folded bound, which is what lets the x86 compare-branch fusion see a
/// register-vs-immediate compare on the back edge instead of a fresh
/// `leaq 4(...)` per iteration (measured: 4.914G Ir -> 4.095G, parity with GCC
/// 16.2, on the `i + 4 < 4096` shape inside a nest).
pub(crate) fn fold_affine_exit_compares(func: &mut IrFunction) -> usize {
    // The kill switch is checked FIRST: `CCC_DEBUG_AFFINE_FOLD=1` with the
    // switch on must report nothing at all (the gate's contract 2).
    if !affine_fold_enabled() {
        return 0;
    }
    // The value-based searches below are not a reaching-definitions analysis.
    // Fail closed on non-SSA webs rather than selecting a convenient writer.
    if !crate::passes::gvn::find_multi_def_values(func).is_empty() {
        return 0;
    }
    let cfg = CfgAnalysis::build(func);
    let raw = find_natural_loops(cfg.num_blocks, &cfg.preds, &cfg.succs, &cfg.idom);
    if raw.is_empty() {
        return 0;
    }
    let loops = merge_loops_by_header(raw);
    // Innermost first (smallest body), deterministic tie-break on the header
    // index: a nested loop's own fold must happen before its outer loop's
    // analysis can be affected by it, and two runs of the pass over the same
    // IR must produce the same order.
    let mut order: Vec<&NaturalLoop> = loops.iter().collect();
    order.sort_by_key(|lp| (lp.body.len(), lp.header));
    let mut total = 0;
    for lp in order {
        total += fold_loop(func, lp, &cfg);
        total += fold_loop_value_bound(func, lp, &cfg);
    }
    if total > 0 && affine_fold_debug() {
        lccc::cli::stderr_line(format_args!(
            "[AFFOLD] {}: affine exit-compare folds: {}",
            func.name, total
        ));
    }
    total
}

/// Whether the loop's own controlling test is proven to stay inside its
/// type's range: the licence [`canonicalise_affine_exit_cmps`] needs before it
/// may fold the copy rotation makes of that test.
///
/// Asked on the loop as it stands, before rotation changes it. Cloning is not
/// a no-overflow licence: the IR also represents `-fwrapv` and unsigned
/// arithmetic feeding signed casts. The copy observes the next induction
/// values, a subset of the original sequence through its failing test, so
/// the proof carries over to that one comparison and to no other.
pub(super) fn header_test_is_proven_affine(
    func: &IrFunction,
    lp: &NaturalLoop,
    cfg: &CfgAnalysis,
) -> bool {
    affine_fold_enabled()
        && loop_iv_shape(func, lp).iter().any(|iv| {
            signed_type_bounds(iv.ty).is_some() && loop_exit_threshold(func, lp, iv, cfg).is_some()
        })
}

/// The induction-variable shape the proof needs: which phi, its constant seed
/// and the constant step of its latch increment.
struct IvShape {
    iv: u32,
    /// Value of the phi on entry to the loop.
    start: i128,
    /// Constant added by the latch (`step > 0`).
    step: i128,
    ty: IrType,
}

/// Classify a loop's induction variable BY VALUE.
///
/// Classify incoming edges by loop membership, not their order: a unique
/// outside edge supplies a constant seed; a unique inside edge supplies the
/// matching typed phi + constant step. Neither fact alone is a certificate.
fn loop_iv_shape(func: &IrFunction, lp: &NaturalLoop) -> Vec<IvShape> {
    let mut out = Vec::new();
    let body_set: FxHashSet<usize> = lp.body.iter().copied().collect();
    // The latch increment lives in whatever block the back edge leaves from
    // (the header itself, for a do-while self-loop), so the step search looks at
    // the whole loop body rather than at the phi's block.
    let body_insts: Vec<&Instruction> = lp
        .body
        .iter()
        .filter(|&&bi| bi < func.blocks.len())
        .flat_map(|&bi| func.blocks[bi].instructions.iter())
        .collect();
    for &bi in &lp.body {
        if bi >= func.blocks.len() {
            continue;
        }
        for inst in &func.blocks[bi].instructions {
            let Instruction::Phi { dest, incoming, ty } = inst else {
                continue;
            };
            if integer_type_bounds(*ty).is_none() {
                continue; // unsupported widths/types have no domain certificate
            }
            if let Some((start, step)) =
                phi_iv_shape(func, &body_set, &body_insts, dest, *ty, incoming)
            {
                if step > 0 {
                    out.push(IvShape {
                        iv: dest.0,
                        start,
                        step,
                        ty: *ty,
                    });
                }
            }
        }
    }
    out
}

/// The counted-loop shape of ONE phi, classified BY EDGE SIDE.
///
/// The phi's incoming edges are not classified by predecessor: a do-while
/// self-loop has the header as its own entry predecessor, and the "entry"
/// incoming may be the second slot.  What decides the role of an incoming is
/// whether its source block is INSIDE the loop body:
///
/// * outside = the entry edge, and its value must be a loop-invariant constant
///   (written directly, or materialised by the frontend as a `Copy` of one
///   defined outside the body) -- that is the seed;
/// * inside = the back edge, and its value must be `phi + Const` -- that is the
///   step.
///
/// Classifying by operand kind instead ("a constant is the seed, a `phi + C` is
/// the step" -- either of them anywhere) accepts two shapes it must not: a
/// constant arriving on the BACK edge, i.e. a loop that restarts its IV from
/// that constant every iteration while the real entry value is an unrelated
/// value, and an entry edge fed by a `phi + C` of some *other* loop.  In both,
/// `start` would be a number the first iteration never takes, and the no-wrap
/// obligation would be checked over the wrong range.  Requiring exactly one
/// incoming per side (`incoming.len() == 2` plus the two roles) keeps the
/// refusal total: anything else yields no shape, hence no fold.
fn phi_iv_shape(
    func: &IrFunction,
    body_set: &FxHashSet<usize>,
    body_insts: &[&Instruction],
    dest: &Value,
    ty: IrType,
    incoming: &[(Operand, BlockId)],
) -> Option<(i128, i128)> {
    if incoming.len() != 2 {
        return None;
    }
    let mut seed: Option<i128> = None;
    let mut step: Option<i128> = None;
    for (val, label) in incoming.iter() {
        let from_body = func
            .blocks
            .iter()
            .position(|b| b.label == *label)
            .is_some_and(|i| body_set.contains(&i));
        match (from_body, val) {
            // Entry edge, constant written directly.
            (false, Operand::Const(c)) => seed = Some(integer_constant(*c, ty)?),
            // Entry edge, constant through a `Copy` defined outside the body.
            (false, Operand::Value(v)) => {
                seed = Some(const_through_copy_outside(func, v.0, body_set, ty)?)
            }
            // Back edge: `phi + Const`, either operand order.
            (true, Operand::Value(v)) => {
                step = Some(body_insts.iter().find_map(|cand| match cand {
                    Instruction::BinOp {
                        dest: d2,
                        op: IrBinOp::Add,
                        lhs,
                        rhs,
                        ty: add_ty,
                    } if d2 == v && *add_ty == ty => match (lhs, rhs) {
                        (Operand::Value(p), Operand::Const(c))
                        | (Operand::Const(c), Operand::Value(p))
                            if p.0 == dest.0 =>
                        {
                            integer_constant(*c, ty)
                        }
                        _ => None,
                    },
                    _ => None,
                })?)
            }
            // A constant on the back edge, or a non-constant on the entry edge.
            _ => return None,
        }
    }
    Some((seed?, step?))
}

/// Constant `C` when `value` is a `Copy` of a constant defined OUTSIDE
/// `body_set` -- the form a loop's seed takes before copy propagation runs.
/// A copy made INSIDE the body is not a seed: its value is the previous
/// iteration's, so the loop is not the counted shape this proof is about.
fn const_through_copy_outside(
    func: &IrFunction,
    value: u32,
    body_set: &FxHashSet<usize>,
    ty: IrType,
) -> Option<i128> {
    func.blocks.iter().enumerate().find_map(|(bi, b)| {
        if body_set.contains(&bi) {
            return None;
        }
        b.instructions.iter().find_map(|i| match i {
            Instruction::Copy { dest, src } if dest.0 == value => match src {
                Operand::Const(c) => integer_constant(*c, ty),
                _ => None,
            },
            _ => None,
        })
    })
}

/// Constant `C` when `value` is defined as `iv + C` (either operand order) in
/// `func`, for the induction variable `iv`.  `None` for anything else --
/// including `iv` itself, which callers handle as the `C == 0` case.
fn affine_offset_of(func: &IrFunction, iv: u32, mut value: u32, ty: IrType) -> Option<i128> {
    let mut offset = 0_i128;
    // Rotation composes (phi + step) + bias. Follow a bounded, same-width
    // additive chain; never infer a value through a narrowing/sign cast.
    for _ in 0..16 {
        if value == iv {
            return Some(offset);
        }
        let inst = func
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .find(|i| i.dest() == Some(Value(value)))?;
        match inst {
            Instruction::BinOp {
                op: IrBinOp::Add,
                lhs,
                rhs,
                ty: add_ty,
                ..
            } if *add_ty == ty => {
                let (v, k) = match (lhs, rhs) {
                    (Operand::Value(v), Operand::Const(k))
                    | (Operand::Const(k), Operand::Value(v)) => (v, k),
                    _ => return None,
                };
                offset = offset.checked_add(integer_constant(*k, ty)?)?;
                value = v.0;
            }
            Instruction::Copy {
                src: Operand::Value(v),
                ..
            } => value = v.0,
            _ => return None,
        }
    }
    None // cycles/deep chains prove nothing
}

/// The exclusive upper bound `T` every test of `iv` in this loop respects,
/// interpreted in the comparison's signed or unsigned mathematical domain.  `None` when the
/// loop's exit is not a single branch on a constant-bounded comparison of
/// `iv` -- a runtime bound (`refuse_rt_bound`), several exits, or an exit that
/// tests something else entirely gives the proof nothing to stand on.
fn loop_exit_threshold(
    func: &IrFunction,
    lp: &NaturalLoop,
    shape: &IvShape,
    cfg: &CfgAnalysis,
) -> Option<i128> {
    let iv = shape.iv;
    let iv_ty = shape.ty;
    let mut exiting: Vec<(usize, &Terminator)> = Vec::new();
    for &bi in &lp.body {
        if bi >= func.blocks.len() {
            continue;
        }
        let term = &func.blocks[bi].terminator;
        if let Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } = term
        {
            let tl = func.blocks.iter().position(|b| b.label == *true_label);
            let fl = func.blocks.iter().position(|b| b.label == *false_label);
            let inside_t = tl.is_some_and(|t| lp.body.contains(&t));
            let inside_f = fl.is_some_and(|f| lp.body.contains(&f));
            if inside_t != inside_f {
                exiting.push((bi, term));
            }
        }
    }
    if exiting.len() != 1 {
        return None;
    }
    let (test_block, term) = exiting[0];
    // An optional interior test is not an induction bound: a bypass could
    // advance the IV past the certified interval before testing it again.
    // Every loop-closing edge must pass through this actual exit test.
    let mut saw_backedge = false;
    for &pred in cfg.preds.row(lp.header) {
        let pred = pred as usize;
        if lp.body.contains(&pred) {
            saw_backedge = true;
            if !is_dominated_by(pred, test_block, cfg) {
                return None;
            }
        }
    }
    if !saw_backedge {
        return None;
    }
    let Terminator::CondBranch {
        cond,
        true_label,
        false_label,
        ..
    } = term
    else {
        return None;
    };
    // POLARITY: which successor stays in the loop decides whether the loop
    // CONTINUES on the test or on its negation.  `continue_on_true` below is
    // that answer, and it is the load-bearing premise of the whole proof: the
    // set of IV values for which the loop continues must be a PREFIX of the IV
    // sequence.  With `continue_on_true`, only `<`/`<=` give a prefix; with
    // `continue_on_false`, only `>`/`>=` do (their negations).  Taking the
    // threshold from the wrong polarity would size the trip count against the
    // complement of the real continuation set -- e.g. a loop that keeps going
    // while `iv` is ABOVE the bound would have been credited with a prefix trip
    // count, and the no-wrap obligation would then be checked over the wrong
    // range.  Refusing here is the only sound answer: the sequence argument
    // below says nothing about upper-interval continuation.
    let continues_inside = |label: &BlockId| {
        func.blocks
            .iter()
            .position(|b| b.label == *label)
            .is_some_and(|i| lp.body.contains(&i))
    };
    let continue_on_true = continues_inside(true_label);
    let _ = continues_inside(false_label);
    let Operand::Value(cond_v) = cond else {
        return None;
    };
    let cmp = func.blocks.iter().find_map(|b| {
        b.instructions.iter().find_map(|i| match i {
            Instruction::Cmp {
                dest,
                lhs,
                op,
                rhs,
                ty,
            } if dest.0 == cond_v.0 && *ty == iv_ty => Some((lhs, op, rhs)),
            _ => None,
        })
    })?;
    let (lhs, op, rhs) = cmp;
    if !comparison_matches_domain(*op, iv_ty) {
        return None;
    }
    // One side names the IV (directly, or through `iv + C`) and contributes a
    // constant offset; the other side must be the constant bound.  A side that
    // is neither -- a runtime bound (`refuse_rt_bound`), an unrelated value --
    // leaves the proof nothing to stand on.
    let iv_side = |operand: &Operand| -> Option<i128> {
        match operand {
            Operand::Value(v) if v.0 == iv => Some(0),
            Operand::Value(v) => affine_offset_of(func, iv, v.0, iv_ty),
            _ => None,
        }
    };
    let (bound_c, c, op) = match (lhs, rhs) {
        // `iv + C <op> N`
        (l, Operand::Const(bc)) if iv_side(l).is_some() => {
            (integer_constant(*bc, iv_ty)?, iv_side(l)?, *op)
        }
        // `N <op> iv + C`: mirror so the affine side is on the left.
        (Operand::Const(bc), r) if iv_side(r).is_some() => {
            let mirrored = match op {
                IrCmpOp::Eq | IrCmpOp::Ne => return None,
                _ => op.swapped(),
            };
            (integer_constant(*bc, iv_ty)?, iv_side(r)?, mirrored)
        }
        _ => return None,
    };
    // The two polarities that admit a prefix continuation set: `op` is the
    // CONTINUATION test.  Anything else is the upper-interval case the polarity
    // comment above refuses.
    if continue_on_true {
        if !matches!(
            op,
            IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Ult | IrCmpOp::Ule
        ) {
            return None;
        }
    } else if !matches!(
        op,
        IrCmpOp::Sgt | IrCmpOp::Sge | IrCmpOp::Ugt | IrCmpOp::Uge
    ) {
        return None;
    }
    // Failing test of `iv + c <op> bound`, in terms of `iv`: the exclusive
    // upper bound of the values for which the loop continues.
    let bound = bound_c;
    let t = match op {
        IrCmpOp::Slt | IrCmpOp::Ult => bound,
        IrCmpOp::Sle | IrCmpOp::Ule => bound + 1,
        IrCmpOp::Sgt | IrCmpOp::Ugt => bound + 1,
        IrCmpOp::Sge | IrCmpOp::Uge => bound,
        _ => return None,
    } - c;
    let (lo, hi) = integer_type_bounds(iv_ty)?;
    // `t` is the derived exclusive bound; `bound` and `c` are the raw
    // operands it came from. Their containment follows from integer_constant
    // and the offset-chain admission today, but the guards are kept as cheap
    // defense-in-depth against future matcher extensions.
    if !(lo..=hi).contains(&t) || !(lo..=hi).contains(&bound) || !(lo..=hi).contains(&c) {
        return None;
    }
    // Prove the CONTROLLING expression itself, independently of whichever
    // comparison a caller wants to fold: if `iv + c` wraps on an executed
    // test, the exit is postponed PAST `t`, the executed IV sequence is no
    // longer a prefix of `start, start+step, ..`, and every range
    // certificate derived from `t` is void. Concretely (-fwrapv, step
    // 2^30-1): `i + 1073741830 < 2147483644` wraps negative on the second
    // test and keeps looping; affine_control_proof.c returns 95 instead of
    // 97 without this obligation. The sequence is monotone (`step > 0`),
    // so the first and the first-failing sums bound every executed sum.
    let last = first_failing_iv(shape, t)?;
    let first_sum = shape.start.checked_add(c)?;
    let last_sum = last.checked_add(c)?;
    if !(lo..=hi).contains(&last)
        || !(lo..=hi).contains(&first_sum)
        || !(lo..=hi).contains(&last_sum)
    {
        return None;
    }
    Some(t)
}

/// Last IV tested, including the first failing guard evaluation.
fn first_failing_iv(shape: &IvShape, threshold: i128) -> Option<i128> {
    if shape.step <= 0 || shape.start >= threshold {
        return None;
    }
    let span = threshold.checked_sub(shape.start)?;
    let iterations = span.checked_add(shape.step.checked_sub(1)?)? / shape.step;
    shape.start.checked_add(iterations.checked_mul(shape.step)?)
}

/// Fold every eligible affine comparison in one loop.  Returns the number of
/// comparisons rewritten.
fn fold_loop(func: &mut IrFunction, lp: &NaturalLoop, cfg: &CfgAnalysis) -> usize {
    // Uses of every value in the function: the affine temporary must have
    // exactly one (the comparison), or folding it neither removes the add nor
    // is provably local (`refuse_two_uses`).
    let mut uses: FxHashMap<u32, usize> = FxHashMap::default();
    for block in func.blocks.iter() {
        for inst in &block.instructions {
            inst.for_each_used_value(|v| *uses.entry(v).or_default() += 1);
        }
        block
            .terminator
            .for_each_used_value(|v| *uses.entry(v).or_default() += 1);
    }

    let shapes = loop_iv_shape(func, lp);
    if shapes.is_empty() {
        return 0;
    }
    // The loop's exit test must bound one of those IVs; the proof is about the
    // tests the loop actually performs, so a loop with no such bound has
    // nothing to prove with.
    let mut bound_iv: Option<(IvShape, i128)> = None;
    for shape in shapes {
        if let Some(t) = loop_exit_threshold(func, lp, &shape, cfg) {
            bound_iv = Some((shape, t));
            break;
        }
    }
    let Some((iv_shape, threshold)) = bound_iv else {
        return 0;
    };
    let mut folded = 0;
    for &bi in &lp.body.clone() {
        if bi >= func.blocks.len() {
            continue;
        }
        let n = func.blocks[bi].instructions.len();
        for idx in 0..n {
            let ty = match &func.blocks[bi].instructions[idx] {
                Instruction::Cmp { ty, .. } => *ty,
                _ => continue,
            };
            if ty != iv_shape.ty {
                continue; // width mismatch: the add's width is what makes it affine
            }
            if let Some((new_op, new_lhs, new_rhs)) =
                plan_affine_fold(func, &iv_shape, threshold, bi, idx, &uses)
            {
                if let Instruction::Cmp { lhs, op, rhs, .. } =
                    &mut func.blocks[bi].instructions[idx]
                {
                    *lhs = new_lhs;
                    *op = new_op;
                    *rhs = new_rhs;
                    folded += 1;
                }
            }
        }
    }
    folded
}

/// AFFOLD-VB: fold a VALUE-bounded affine exit compare, `iv + C <strict> n`,
/// into `iv <strict> t` with `t = n - C` computed once in the guard block.
///
/// The constant-bound fold above certifies the whole IV range against a
/// compile-time threshold.  With a runtime bound `n` no such range exists, so
/// this fold rests on a different, narrower proof.  Every clause below is
/// load-bearing; the counterexamples are in `affine_value_bound_tests`.
///
/// Soundness (exact integer arithmetic, no UB licence -- the IR also carries
/// `-fwrapv` code and the proof must not care):
///
/// * GUARD: the loop's unique outside predecessor `P` ends in
///   `CondBranch(k <strict> n, header, exit)` with constant `k >= start + C`
///   (`start` the IV's constant seed).  Every execution of the folded compare
///   is therefore dominated by the established fact `n > start + C`, so
///   `n - C > start >= type_min`: the pre-folded bound `t` CANNOT underflow on
///   any path that reaches the compare.  Without the guard, `n` near the type
///   minimum wraps `t` to a huge value and the folded loop runs where the
///   original exits immediately (`vb_refuses_unguarded`).
/// * STRICT comparison only (`Slt`/`Ult`): with `<=` and `n == type_max` the
///   original wrapping `iv + C` eventually laps the bound and the ORIGINAL
///   loop never exits, while the folded one does -- a finite loop is not a
///   faithful image of an infinite one (`vb_refuses_sle`).
/// * STEP == 1: by induction every compare-time value `x` satisfies
///   `x + C <= n <= type_max` -- base case from the guard, inductive step
///   `x' = x + 1 <= n - C` from the previous compare being true -- so the
///   original add never wraps and both compares decide identically.  A step
///   `s > 1` overshoots the bound by up to `s - 1`, `x + C` can exceed
///   `type_max` when `n` is near it, and the wrapped sum compares differently
///   (`vb_refuses_step2`).
/// * The affine temporary and the compare result are SINGLE-USE, the bound
///   `n` is defined outside the loop (SSA: one def, loop-invariant), and the
///   compare is the loop's unique exit test with every backedge dominated by
///   it -- the induction needs "reaching the compare again means the previous
///   compare was true".
///
/// The rewrite anchors on the LAST link of the affine chain (`a = x + C_rel`,
/// usually the post-increment), not on the phi: `x` is already live at the
/// compare, so no live range is lengthened, and the single-use add dies.
fn fold_loop_value_bound(func: &mut IrFunction, lp: &NaturalLoop, cfg: &CfgAnalysis) -> usize {
    const MAX_C: i128 = i64::MAX as i128;
    let header_label = match func.blocks.get(lp.header) {
        Some(b) => b.label,
        None => return 0,
    };
    // Uses of every value: the affine temporary and the compare result must
    // each have exactly one (see the single-use clause above).
    let mut uses: FxHashMap<u32, usize> = FxHashMap::default();
    for block in func.blocks.iter() {
        for inst in &block.instructions {
            inst.for_each_used_value(|v| *uses.entry(v).or_default() += 1);
        }
        block
            .terminator
            .for_each_used_value(|v| *uses.entry(v).or_default() += 1);
    }
    let shapes: Vec<IvShape> = loop_iv_shape(func, lp)
        .into_iter()
        .filter(|s| s.step == 1)
        .collect();
    if shapes.is_empty() {
        return 0;
    }
    // Unique exit test; every backedge must pass through it (same structural
    // requirements as `loop_exit_threshold`, for the same inductive reason).
    let mut exiting: Vec<usize> = Vec::new();
    for &bi in &lp.body {
        if bi >= func.blocks.len() {
            continue;
        }
        if let Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } = &func.blocks[bi].terminator
        {
            let tl = func.blocks.iter().position(|b| b.label == *true_label);
            let fl = func.blocks.iter().position(|b| b.label == *false_label);
            let inside_t = tl.is_some_and(|t| lp.body.contains(&t));
            let inside_f = fl.is_some_and(|f| lp.body.contains(&f));
            if inside_t != inside_f {
                exiting.push(bi);
            }
        }
    }
    if exiting.len() != 1 {
        return 0;
    }
    let test_block = exiting[0];
    let mut saw_backedge = false;
    for &pred in cfg.preds.row(lp.header) {
        let pred = pred as usize;
        if lp.body.contains(&pred) {
            saw_backedge = true;
            if !is_dominated_by(pred, test_block, cfg) {
                return 0;
            }
        }
    }
    if !saw_backedge {
        return 0;
    }
    let Terminator::CondBranch {
        cond, true_label, ..
    } = &func.blocks[test_block].terminator
    else {
        return 0;
    };
    // CONTINUE must be the true edge: the strict-less forms below are the
    // continuation test.  Exit-on-true carries the negated operator and is
    // out of this proof's scope.
    let continue_inside = func
        .blocks
        .iter()
        .position(|b| b.label == *true_label)
        .is_some_and(|i| lp.body.contains(&i));
    if !continue_inside {
        return 0;
    }
    let Operand::Value(cond_v) = *cond else {
        return 0;
    };
    if uses.get(&cond_v.0).copied() != Some(1) {
        return 0;
    }
    // The unique outside predecessor is the guard block.
    let mut outside_preds = cfg
        .preds
        .row(lp.header)
        .iter()
        .map(|&p| p as usize)
        .filter(|p| !lp.body.contains(p));
    let (Some(pre), None) = (outside_preds.next(), outside_preds.next()) else {
        return 0;
    };
    // Locate the compare (inside the loop: it consumes the IV).
    let cmp_loc = lp.body.iter().find_map(|&bi| {
        func.blocks
            .get(bi)?
            .instructions
            .iter()
            .enumerate()
            .find_map(|(ii, inst)| match inst {
                Instruction::Cmp { dest, .. } if dest.0 == cond_v.0 => Some((bi, ii)),
                _ => None,
            })
    });
    let Some((cmp_bi, cmp_ii)) = cmp_loc else {
        return 0;
    };
    let Instruction::Cmp {
        lhs, op, rhs, ty, ..
    } = func.blocks[cmp_bi].instructions[cmp_ii].clone()
    else {
        return 0;
    };
    for shape in &shapes {
        if shape.ty != ty || !comparison_matches_domain(op, ty) {
            continue;
        }
        let Some((lo, hi)) = integer_type_bounds(ty) else {
            continue;
        };
        // Normalize to `affine <strict> bound`; mirrored orientation flips
        // the operator.  Non-strict forms are refused (see `vb_refuses_sle`).
        let (affine_op, bound_op, strict_op) = match op {
            IrCmpOp::Slt | IrCmpOp::Ult => (&lhs, &rhs, op),
            IrCmpOp::Sgt => (&rhs, &lhs, IrCmpOp::Slt),
            IrCmpOp::Ugt => (&rhs, &lhs, IrCmpOp::Ult),
            _ => continue,
        };
        let Operand::Value(av) = *affine_op else {
            continue; // constant affine sides are the other fold's business
        };
        let Operand::Value(nv) = *bound_op else {
            continue; // constant bounds are `fold_loop`'s business
        };
        if uses.get(&av.0).copied() != Some(1) {
            continue;
        }
        let Some(c_total) = affine_offset_of(func, shape.iv, av.0, ty) else {
            continue;
        };
        if !(1..=MAX_C).contains(&c_total) {
            continue;
        }
        // The bound is loop-invariant: its single SSA def is outside the body.
        let n_def_inside = lp.body.iter().any(|&bi| {
            func.blocks
                .get(bi)
                .is_some_and(|b| b.instructions.iter().any(|i| i.dest() == Some(nv)))
        });
        if n_def_inside {
            continue;
        }
        // Anchor on the last additive link: `av = anchor + c_rel`.
        let Some((anchor, c_rel)) = last_add_link(func, av.0, ty) else {
            continue;
        };
        if !(1..=MAX_C).contains(&c_rel) {
            continue;
        }
        // Prefer the latch's post-increment as the anchor when the chain was
        // reassociated onto the phi (`(phi + 1) + 4` folded to `phi + 5`):
        // comparing the PRE-increment phi forces the backend to keep both the
        // old and the new IV live across the bottom test (a `mov` + taken
        // `jmp` per iteration), while the post-increment is the value the
        // tight `add; cmp; jcc` latch already has in hand.  Exactness is the
        // same induction: `phi + step <= phi + c_total <= n <= type_max`.
        let (anchor, c_rel) = if anchor == shape.iv && c_rel == c_total && c_total > shape.step {
            post_increment_anchor(func, cfg, lp, shape, cmp_bi, cmp_ii)
                .map(|inc| (inc, c_total - shape.step))
                .unwrap_or((anchor, c_rel))
        } else {
            (anchor, c_rel)
        };
        let first = match shape.start.checked_add(c_total) {
            Some(f) if (lo..=hi).contains(&f) => f,
            _ => continue,
        };
        // GUARD: a conditional branch above the loop whose edge towards the
        // header establishes `n > first - 1` (equivalently `n >= first`).
        if !guard_proves_bound(func, cfg, pre, header_label, nv, strict_op, ty, first) {
            continue;
        }
        // All obligations hold: fold.
        let tdest = Value(func.next_value_id);
        func.next_value_id += 1;
        let sub = Instruction::BinOp {
            dest: tdest,
            op: IrBinOp::Sub,
            lhs: Operand::Value(nv),
            rhs: Operand::Const(IrConst::from_i64(c_rel as i64, ty)),
            ty,
        };
        let pre_block = &mut func.blocks[pre];
        let span = pre_block
            .source_spans
            .last()
            .copied()
            .unwrap_or_else(crate::common::source::Span::dummy);
        pre_block.push_inst(sub, span);
        if let Instruction::Cmp { lhs, op, rhs, .. } = &mut func.blocks[cmp_bi].instructions[cmp_ii]
        {
            *lhs = Operand::Value(Value(anchor));
            *op = strict_op;
            *rhs = Operand::Value(tdest);
        }
        if affine_fold_debug() {
            lccc::cli::stderr_line(format_args!(
                "[AFFOLD] {}: value-bound fold: iv v{} + {} <strict> v{} -> v{} <strict> (v{} - {})",
                func.name, shape.iv, c_total, nv.0, anchor, nv.0, c_rel
            ));
        }
        return 1;
    }
    0
}

/// The loop's post-increment of `shape`'s phi — `inc = phi + step`, in the
/// fold's width — provided its definition dominates the comparison at
/// `(cmp_bi, cmp_ii)` so the rewritten compare may read it.
fn post_increment_anchor(
    func: &IrFunction,
    cfg: &CfgAnalysis,
    lp: &NaturalLoop,
    shape: &IvShape,
    cmp_bi: usize,
    cmp_ii: usize,
) -> Option<u32> {
    for &bi in &lp.body {
        let Some(block) = func.blocks.get(bi) else {
            continue;
        };
        for (ii, inst) in block.instructions.iter().enumerate() {
            let Instruction::BinOp {
                dest,
                op: IrBinOp::Add,
                lhs,
                rhs,
                ty,
            } = inst
            else {
                continue;
            };
            if *ty != shape.ty {
                continue;
            }
            let matches_inc = match (lhs, rhs) {
                (Operand::Value(v), Operand::Const(k)) | (Operand::Const(k), Operand::Value(v)) => {
                    v.0 == shape.iv && integer_constant(*k, shape.ty) == Some(shape.step)
                }
                _ => false,
            };
            if !matches_inc {
                continue;
            }
            let dominates = if bi == cmp_bi {
                ii < cmp_ii
            } else {
                is_dominated_by(cmp_bi, bi, cfg)
            };
            if dominates {
                return Some(dest.0);
            }
        }
    }
    None
}

/// The last link of an affine chain: `value = x + K` (through `Copy`s), with
/// the add in the comparison's own width.  Returns `(x, K)`.
fn last_add_link(func: &IrFunction, mut value: u32, ty: IrType) -> Option<(u32, i128)> {
    for _ in 0..16 {
        let inst = func
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .find(|i| i.dest() == Some(Value(value)))?;
        match inst {
            Instruction::BinOp {
                op: IrBinOp::Add,
                lhs,
                rhs,
                ty: add_ty,
                ..
            } if *add_ty == ty => {
                let (v, k) = match (lhs, rhs) {
                    (Operand::Value(v), Operand::Const(k))
                    | (Operand::Const(k), Operand::Value(v)) => (v, k),
                    _ => return None,
                };
                return Some((v.0, integer_constant(*k, ty)?));
            }
            Instruction::Copy {
                src: Operand::Value(v),
                ..
            } => value = v.0,
            _ => return None,
        }
    }
    None
}

/// Is `n >= first` established on every path into the loop?
///
/// The climb: starting at the loop's unique outside predecessor, walk up
/// through single-predecessor relay blocks (`Branch`-terminated preheaders
/// full of phi-elim copies are the normal shape) until a `CondBranch` is
/// found.  Exactly ONE of its edges may lead down the climbed chain; the
/// condition on that edge must pin `n` against a constant:
///
/// * edge-taken-when-TRUE:  `K < n`, `n > K`  (fact `n > K`),
///   `K <= n`, `n >= K`  (fact `n >= K`);
/// * edge-taken-when-FALSE: the negations — `n <= K`, `K >= n` (fact
///   `n > K`), `n < K`, `K > n` (fact `n >= K`).
///
/// A strict fact needs `K >= first - 1`, a non-strict one `K >= first`.  The
/// comparison domain must match the fold's (signed guard for a signed fold:
/// an unsigned fact says nothing about signed order).  `n` is an SSA value,
/// so the fact established once holds at every later execution point.
fn guard_proves_bound(
    func: &IrFunction,
    cfg: &CfgAnalysis,
    pre: usize,
    header_label: BlockId,
    nv: Value,
    strict_op: IrCmpOp,
    ty: IrType,
    first: i128,
) -> bool {
    let mut child_label = header_label;
    let mut cur = pre;
    for _ in 0..8 {
        let Some(cur_block) = func.blocks.get(cur) else {
            return false;
        };
        match &cur_block.terminator {
            Terminator::Branch(l) if *l == child_label => {
                // Relay block: climb to its unique predecessor.
                let mut preds = cfg.preds.row(cur).iter().map(|&p| p as usize);
                let (Some(p), None) = (preds.next(), preds.next()) else {
                    return false;
                };
                child_label = cur_block.label;
                cur = p;
            }
            Terminator::CondBranch {
                cond,
                true_label,
                false_label,
            } => {
                let enters_true = *true_label == child_label;
                let enters_false = *false_label == child_label;
                if enters_true == enters_false {
                    return false; // both or neither: no fact on the edge
                }
                let Operand::Value(gv) = *cond else {
                    return false;
                };
                let gcmp = func.blocks.iter().find_map(|b| {
                    b.instructions.iter().find_map(|i| match i {
                        Instruction::Cmp {
                            dest,
                            lhs,
                            op,
                            rhs,
                            ty: cmp_ty,
                        } if dest.0 == gv.0 && *cmp_ty == ty => Some((*lhs, *op, *rhs)),
                        _ => None,
                    })
                });
                let Some((glhs, gop, grhs)) = gcmp else {
                    return false;
                };
                // Normalize to a fact about `n` vs a constant `K` on the
                // entering edge.  `strict` is whether the fact is `n > K`
                // (versus `n >= K`).
                let signed = matches!(strict_op, IrCmpOp::Slt);
                let fact = guard_fact(glhs, gop, grhs, nv, ty, signed, enters_true);
                let Some((k, strict)) = fact else {
                    return false;
                };
                let need = if strict {
                    first.checked_sub(1)
                } else {
                    Some(first)
                };
                return need.is_some_and(|need| k >= need);
            }
            _ => return false,
        }
    }
    false
}

/// The fact `n > K` (strict) or `n >= K` (non-strict) carried by `cmp` on the
/// edge taken when the condition is `taken_on_true`, or `None` if the
/// comparison does not pin `nv` from below in the required domain.
fn guard_fact(
    lhs: Operand,
    op: IrCmpOp,
    rhs: Operand,
    nv: Value,
    ty: IrType,
    signed: bool,
    taken_on_true: bool,
) -> Option<(i128, bool)> {
    let k_lhs = |k: IrConst| integer_constant(k, ty);
    // (const K relates-to n) or (n relates-to const K), as (K, op-with-n-on-
    // the-RIGHT): mirror the second form.
    let (k, op) = match (lhs, rhs) {
        (Operand::Const(k), Operand::Value(v)) if v == nv => (k_lhs(k)?, op),
        (Operand::Value(v), Operand::Const(k)) if v == nv => {
            let mirrored = match op {
                IrCmpOp::Eq | IrCmpOp::Ne => return None,
                _ => op.swapped(),
            };
            (k_lhs(k)?, mirrored)
        }
        _ => return None,
    };
    // `op` now reads `K <op> n`.  The edge's fact is `op` itself when taken
    // on true, its negation when taken on false.
    let effective = if taken_on_true {
        op
    } else {
        match op {
            IrCmpOp::Eq | IrCmpOp::Ne => return None,
            _ => op.inverse(),
        }
    };
    // Keep only lower bounds on `n`, in the matching domain.
    match (signed, effective) {
        (true, IrCmpOp::Slt) => Some((k, true)),   // K < n
        (true, IrCmpOp::Sle) => Some((k, false)),  // K <= n
        (false, IrCmpOp::Ult) => Some((k, true)),  // K < n (unsigned)
        (false, IrCmpOp::Ule) => Some((k, false)), // K <= n (unsigned)
        _ => None,
    }
}

/// Prove the fold for ONE comparison and return the rewritten form, or `None`.
///
/// The rewrite `iv + C <op> N  ==  iv <op> N - C` is a pure integer identity
/// when `iv + C` is computed without wrapping.  Everything below exists to
/// prove that, exactly, for the values the loop can observe:
///
/// * the comparison must be `(iv + C) <op> N` or `N <op2> (iv + C)` with a
///   CONSTANT `C >= 0`, and the add's type must equal the comparison's type
///   (`refuse_width_mismatch` refuses a narrow compare of a wide add);
/// * the folded bound `N - C` must be representable in that type (the
///   `near_high` boundary: `N - C == i64::MAX - 4`);
/// * the values `iv` can take inside the loop are `start, start + step, ...`
///   up to and including the test that fails, i.e. indices `0..=L` where `L`
///   is the first index at or past the loop's exit threshold `T`.  The loop
///   must actually perform at least one continuing test (`L >= 1`); a shape
///   whose entry test already fails folds nothing and is refused
///   (`top_end_start`), which also keeps the accepted set inside what the
///   fixtures exercise;
/// * `start + C` and `t_L + C` must both be representable: the sequence is
///   monotone in `step > 0`, so those two checked endpoints cover every test
///   (`near_high`/`near_low` are the accepted boundaries, where the failing
///   test computes exactly the type maximum/minimum);
/// * all obligation arithmetic runs in `i128` -- a proof that itself wraps
///   proves nothing, and the failure mode of a wrapped proof is silent
///   acceptance of an unsound fold.
fn plan_affine_fold(
    func: &IrFunction,
    iv_shape: &IvShape,
    threshold: i128,
    block_idx: usize,
    inst_idx: usize,
    uses: &FxHashMap<u32, usize>,
) -> Option<(IrCmpOp, Operand, Operand)> {
    let (lo, hi) = integer_type_bounds(iv_shape.ty)?;
    let Instruction::Cmp {
        lhs, op, rhs, ty, ..
    } = &func.blocks[block_idx].instructions[inst_idx]
    else {
        return None;
    };
    if *ty != iv_shape.ty || !comparison_matches_domain(*op, *ty) {
        return None;
    }
    // The affine temporary on one side: a single-use `Add(iv, Const c)` of the
    // same type.
    let affine_of = |operand: &Operand| -> Option<i128> {
        let Operand::Value(v) = operand else {
            return None;
        };
        // One use only: otherwise the add survives the fold and the rewrite is
        // no longer local (`refuse_two_uses`).
        if uses.get(&v.0).copied() != Some(1) {
            return None;
        }
        affine_offset_of(func, iv_shape.iv, v.0, iv_shape.ty)
    };
    let affine_l = affine_of(lhs);
    let affine_r = affine_of(rhs);
    // Orientation: normalize to `(iv + C) <op> N` and pin the operator; a
    // comparison whose affine side is on the right is mirrored, and a
    // comparison that would run away as `iv` grows (`iv + C > N` and friends)
    // is refused -- the sequence argument below covers the terminating forms
    // only.
    let (c, bound, op) = match (affine_l, &lhs, &rhs, op) {
        (Some(c), _, Operand::Const(b), o)
            if matches!(o, IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Ult | IrCmpOp::Ule) =>
        {
            (c, integer_constant(*b, *ty)?, *o)
        }
        (None, Operand::Const(b), _, o) if affine_r.is_some() => {
            let mirrored = match o {
                IrCmpOp::Sgt => IrCmpOp::Slt,
                IrCmpOp::Sge => IrCmpOp::Sle,
                IrCmpOp::Ugt => IrCmpOp::Ult,
                IrCmpOp::Uge => IrCmpOp::Ule,
                _ => return None,
            };
            (affine_r?, integer_constant(*b, *ty)?, mirrored)
        }
        _ => return None,
    };
    if c < 0 {
        return None; // the accepted set is the ascending, non-negative-addend shape
    }
    let new_bound = bound.checked_sub(c)?;
    if new_bound < lo || new_bound > hi {
        return None;
    }
    // Sequence obligations (see the doc comment).
    let first = iv_shape.start.checked_add(c)?;
    if first < lo || first > hi {
        return None;
    }
    if iv_shape.start >= threshold {
        return None; // entry test already fails: L == 0
    }
    let span = threshold - iv_shape.start;
    let step = iv_shape.step;
    let l = (span + step - 1) / step; // ceil, step > 0
    if l < 1 {
        return None;
    }
    let last = iv_shape.start.checked_add(l.checked_mul(step)?)?;
    let last_plus_c = last.checked_add(c)?;
    if last_plus_c < lo || last_plus_c > hi {
        return None;
    }
    // Prefer an already-shared additive source, especially the latch's
    // post-increment value. Folding all the way back to the phi would keep
    // its old value live across the update and lose the original one-compare
    // loop. Both the anchor and the final sum need representable endpoints.
    let candidate = if affine_l.is_some() { lhs } else { rhs };
    let anchor = if let Operand::Value(sum) = candidate {
        func.blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .find_map(|i| {
                let Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs,
                    rhs,
                    ty: add_ty,
                } = i
                else {
                    return None;
                };
                if dest != sum || add_ty != ty {
                    return None;
                }
                let (v, k) = match (lhs, rhs) {
                    (Operand::Value(v), Operand::Const(k))
                    | (Operand::Const(k), Operand::Value(v)) => (v, k),
                    _ => return None,
                };
                if uses.get(&v.0).copied().unwrap_or(0) < 2 {
                    return None;
                }
                let bias = integer_constant(*k, *ty)?;
                let prefix = affine_offset_of(func, iv_shape.iv, v.0, *ty)?;
                let adjusted = bound.checked_sub(bias)?;
                if bias < 0
                    || !(lo..=hi).contains(&adjusted)
                    || !(lo..=hi).contains(&iv_shape.start.checked_add(prefix)?)
                    || !(lo..=hi).contains(&last.checked_add(prefix)?)
                {
                    return None;
                }
                Some((*v, adjusted))
            })
    } else {
        None
    };
    let (anchor, new_bound) = anchor.unwrap_or((Value(iv_shape.iv), new_bound));
    let new_bound_c = IrConst::from_i64(new_bound as i64, iv_shape.ty);
    Some((op, Operand::Value(anchor), Operand::Const(new_bound_c)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType as Ty;
    use crate::ir::reexports::BasicBlock;

    fn blk(label: u32, insts: Vec<Instruction>, term: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions: insts,
            terminator: term,
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        }
    }

    /// The canonical counted loop the fold is written for:
    ///
    /// ```text
    ///   0 entry:  <seed>            -> 1
    ///   1 header: phi i = [seed, 0], [i_next, 2]
    ///             cmp  iv_ty: (i + C) OP bound
    ///             CondBranch(cond, body, exit)
    ///   2 body:   branch -> 3
    ///   3 latch:  i_next = i + STEP ; branch -> 1
    ///   4 exit:   ret
    /// ```
    ///
    /// `op` and `bound` are parameters so the refused orientations can be
    /// built; `bound_const` says whether the bound is a constant (the
    /// `refuse_rt_bound` shape uses a live-in value instead).
    fn counted_loop(
        ty: Ty,
        start: i64,
        c: i64,
        op: IrCmpOp,
        bound: i64,
        bound_const: bool,
        step: i64,
        start_const: bool,
        step_const: bool,
    ) -> IrFunction {
        let mut f = IrFunction::new("affold".into(), Ty::I32, vec![], false);
        // Values: 1 = seed, 2 = phi, 3 = affine temp, 4 = cond, 5 = step const,
        // 6 = i_next, 7 = runtime bound, 8 = runtime step/start.
        let seed_insts = if start_const {
            vec![Instruction::Copy {
                dest: Value(1),
                src: Operand::Const(IrConst::from_i64(start, ty)),
            }]
        } else {
            vec![Instruction::ParamRef {
                dest: Value(1),
                param_idx: 0,
                ty,
            }]
        };
        let mut header = vec![
            Instruction::Phi {
                dest: Value(2),
                ty,
                incoming: vec![
                    (Operand::Value(Value(1)), BlockId(0)),
                    (Operand::Value(Value(6)), BlockId(3)),
                ],
            },
            Instruction::BinOp {
                dest: Value(3),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Const(IrConst::from_i64(c, ty)),
                ty,
            },
        ];
        let bound_op = if bound_const {
            Operand::Const(IrConst::from_i64(bound, ty))
        } else {
            header.push(Instruction::ParamRef {
                dest: Value(7),
                param_idx: 1,
                ty,
            });
            Operand::Value(Value(7))
        };
        header.push(Instruction::Cmp {
            dest: Value(4),
            lhs: Operand::Value(Value(3)),
            op,
            rhs: bound_op,
            ty,
        });
        let step_inst = if step_const {
            Instruction::BinOp {
                dest: Value(6),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Const(IrConst::from_i64(step, ty)),
                ty,
            }
        } else {
            Instruction::BinOp {
                dest: Value(6),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Value(Value(8)),
                ty,
            }
        };
        let mut latch = vec![step_inst];
        if !step_const {
            latch.insert(
                0,
                Instruction::ParamRef {
                    dest: Value(8),
                    param_idx: 2,
                    ty,
                },
            );
        }
        f.blocks
            .push(blk(0, seed_insts, Terminator::Branch(BlockId(1))));
        f.blocks.push(blk(
            1,
            header,
            Terminator::CondBranch {
                cond: Operand::Value(Value(4)),
                true_label: BlockId(2),
                false_label: BlockId(4),
            },
        ));
        f.blocks
            .push(blk(2, vec![], Terminator::Branch(BlockId(3))));
        f.blocks.push(blk(3, latch, Terminator::Branch(BlockId(1))));
        f.blocks.push(blk(4, vec![], Terminator::Return(None)));
        f
    }

    /// The same counted loop with EXIT-ON-TRUE polarity: the CondBranch's true
    /// successor leaves the loop and the loop continues while the test is
    /// FALSE.  With an `<`/`<=` test that makes the continuation set an upper
    /// interval of the IV -- the shape the polarity guard must refuse.
    fn counted_loop_exit_on_true(ty: Ty, c: i64, op: IrCmpOp, bound: i64) -> IrFunction {
        let mut f = counted_loop(ty, 0, c, op, bound, true, 1, true, true);
        f.blocks[1].terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(4)),
            true_label: BlockId(4),
            false_label: BlockId(2),
        };
        f
    }

    /// A counted loop whose phi takes a CONSTANT on the back edge -- the loop
    /// restarts its IV from that constant every iteration, so the value the
    /// first iteration sees is the entry operand, not that constant.
    fn counted_loop_reset_on_latch(entry: i64, latch: i64) -> IrFunction {
        let mut f = counted_loop(Ty::I64, entry, 4, IrCmpOp::Slt, 4096, true, 1, true, true);
        f.blocks[1].instructions[0] = Instruction::Phi {
            dest: Value(2),
            ty: Ty::I64,
            incoming: vec![
                (
                    Operand::Const(IrConst::from_i64(entry, Ty::I64)),
                    BlockId(0),
                ),
                (
                    Operand::Const(IrConst::from_i64(latch, Ty::I64)),
                    BlockId(3),
                ),
            ],
        };
        f
    }

    /// A counted loop whose entry operand is an `Add` of two unrelated values:
    /// the seed is not a constant at all.
    fn counted_loop_entry_add(ty: Ty) -> IrFunction {
        let mut f = counted_loop(ty, 0, 4, IrCmpOp::Slt, 4096, true, 1, true, true);
        f.blocks[0].instructions.push(Instruction::BinOp {
            dest: Value(9),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(10)),
            rhs: Operand::Value(Value(11)),
            ty,
        });
        f.blocks[1].instructions[0] = Instruction::Phi {
            dest: Value(2),
            ty,
            incoming: vec![
                (Operand::Value(Value(9)), BlockId(0)),
                (Operand::Value(Value(6)), BlockId(3)),
            ],
        };
        f
    }

    /// The comparison's printed form after the pass, or `None` when the fold
    /// did not fire.
    fn folded_cmp(f: &IrFunction) -> Option<(IrCmpOp, u32, i64)> {
        let mut found = None;
        for b in f.blocks.iter() {
            for i in &b.instructions {
                if let Instruction::Cmp { lhs, op, rhs, .. } = i {
                    let (Operand::Value(iv), Operand::Const(c)) = (lhs, rhs) else {
                        return found;
                    };
                    // The folded form compares the IV phi against a literal; the
                    // unfolded form compares the affine temporary.
                    if iv.0 == 2 {
                        found = Some((*op, iv.0, c.to_i64()?));
                    }
                }
            }
        }
        found
    }

    #[test]
    fn accepts_the_canonical_shape() {
        let mut f = counted_loop(Ty::I64, 0, 4, IrCmpOp::Slt, 4096, true, 1, true, true);
        assert_eq!(fold_affine_exit_compares(&mut f), 1);
        assert_eq!(folded_cmp(&f), Some((IrCmpOp::Slt, 2, 4092)));
    }

    #[test]
    fn accepts_le_and_mirrors_the_bound() {
        // `i + 4 <= 64` folds to `i <= 60`.
        let mut f = counted_loop(Ty::I64, 0, 4, IrCmpOp::Sle, 64, true, 1, true, true);
        assert_eq!(fold_affine_exit_compares(&mut f), 1);
        assert_eq!(folded_cmp(&f), Some((IrCmpOp::Sle, 2, 60)));
    }

    #[test]
    fn accepts_a_nonzero_start_and_step() {
        let mut f = counted_loop(Ty::I64, 1, 8, IrCmpOp::Slt, 4097, true, 5, true, true);
        assert_eq!(fold_affine_exit_compares(&mut f), 1);
        assert_eq!(folded_cmp(&f), Some((IrCmpOp::Slt, 2, 4089)));
    }

    #[test]
    fn refuses_a_negative_addend() {
        let mut f = counted_loop(Ty::I64, 0, -4, IrCmpOp::Slt, 64, true, 1, true, true);
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }

    #[test]
    fn refuses_a_runtime_bound() {
        let mut f = counted_loop(Ty::I64, 0, 4, IrCmpOp::Slt, 0, false, 1, true, true);
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }

    #[test]
    fn refuses_a_runtime_seed() {
        let mut f = counted_loop(Ty::I64, 0, 4, IrCmpOp::Slt, 64, true, 1, false, true);
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }

    #[test]
    fn refuses_a_runtime_step() {
        let mut f = counted_loop(Ty::I64, 0, 4, IrCmpOp::Slt, 64, true, 0, true, false);
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }

    #[test]
    fn accepts_a_proven_unsigned_comparison() {
        let mut f = counted_loop(Ty::U32, 0, 4, IrCmpOp::Ult, 64, true, 1, true, true);
        assert_eq!(fold_affine_exit_compares(&mut f), 1);
        assert_eq!(folded_cmp(&f), Some((IrCmpOp::Ult, 2, 60)));
    }

    /// The controlling expression's OWN wrap postpones the exit: with
    /// `-fwrapv`, `i + 1073741830 < 2147483644` and `step = 2^30 - 1`,
    /// the second test computes `1073741823 + 1073741830`, which wraps
    /// negative and keeps looping past the threshold. Every certificate
    /// derived from that threshold is void, so the fold must refuse
    /// (affine_control_proof.c returned 95 instead of 97 without this).
    #[test]
    fn refuses_a_wrapping_controlling_expression() {
        let mut f = counted_loop(
            Ty::I32,
            0,
            1_073_741_830,
            IrCmpOp::Slt,
            2_147_483_644,
            true,
            1_073_741_823,
            true,
            true,
        );
        let original = format!("{f:?}");
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
        assert_eq!(format!("{f:?}"), original);
        // Control: the same shape with a small step exits before the sum
        // can wrap, and the fold still runs.
        let mut ok = counted_loop(
            Ty::I32,
            0,
            1_073_741_830,
            IrCmpOp::Slt,
            2_147_483_644,
            true,
            1,
            true,
            true,
        );
        assert_eq!(fold_affine_exit_compares(&mut ok), 1);
    }

    #[test]
    fn refuses_a_runaway_orientation() {
        // `i + 4 > 64` stays true as `i` grows: the loop does not terminate
        // through this test, so the sequence argument does not apply.
        let mut f = counted_loop(Ty::I64, 0, 4, IrCmpOp::Sgt, 64, true, 1, true, true);
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }

    #[test]
    fn refuses_a_type_top_zero_trip_shape() {
        // `top_end_start`: start = MAX - 4, `i + 4 < MAX` -- the entry test
        // already fails, so the loop never continues (`L == 0`).
        let mut f = counted_loop(
            Ty::I64,
            i64::MAX - 4,
            4,
            IrCmpOp::Slt,
            i64::MAX,
            true,
            1,
            true,
            true,
        );
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }

    #[test]
    fn accepts_the_type_top_boundary() {
        // `near_high`: start = MAX - 20, `i + 4 < MAX`; the failing test computes
        // exactly MAX, and the folded bound is MAX - 4.
        let mut f = counted_loop(
            Ty::I64,
            i64::MAX - 20,
            4,
            IrCmpOp::Slt,
            i64::MAX,
            true,
            1,
            true,
            true,
        );
        assert_eq!(fold_affine_exit_compares(&mut f), 1);
        assert_eq!(folded_cmp(&f), Some((IrCmpOp::Slt, 2, i64::MAX - 4)));
    }

    #[test]
    fn refuses_an_unrepresentable_folded_bound() {
        // start = MIN, C = 4, bound = MIN + 2: `N - C` underflows the type.
        let mut f = counted_loop(
            Ty::I64,
            i64::MIN,
            8,
            IrCmpOp::Slt,
            i64::MIN + 2,
            true,
            1,
            true,
            true,
        );
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }

    #[test]
    fn refuses_exit_on_true_with_a_less_than_test() {
        // Continuation is `!(iv + 4 < 4096)`: an upper interval, not a prefix.
        // The trip-count obligations say nothing about it, so the fold must not
        // fire however plausible the bound looks.
        let mut f = counted_loop_exit_on_true(Ty::I64, 4, IrCmpOp::Slt, 4096);
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }

    #[test]
    fn refuses_exit_on_true_with_a_greater_than_test_too() {
        // The mirror image WOULD have a prefix continuation set (`!(iv + 4 >
        // 4092)` is `iv + 4 <= 4092`), so the polarity guard lets the threshold
        // through -- and then the rewrite is still refused, because `>` with the
        // affine operand on the left is the runaway orientation the fold never
        // produces.  Two independent refusals, both pinned here.
        let mut f = counted_loop_exit_on_true(Ty::I64, 4, IrCmpOp::Sgt, 4092);
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
        // ... while the ascending form of the same loop does fold, which is what
        // makes the refusal above a choice about orientation and not about
        // polarity.
        let mut ok = counted_loop(Ty::I64, 0, 4, IrCmpOp::Sle, 4092, true, 1, true, true);
        assert_eq!(fold_affine_exit_compares(&mut ok), 1);
    }

    #[test]
    fn refuses_a_constant_on_the_back_edge() {
        let mut f = counted_loop_reset_on_latch(0, 5);
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }

    #[test]
    fn refuses_a_non_constant_entry_operand() {
        let mut f = counted_loop_entry_add(Ty::I64);
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }

    #[test]
    fn refuses_a_second_use_of_the_temporary() {
        let mut f = counted_loop(Ty::I64, 0, 4, IrCmpOp::Slt, 64, true, 1, true, true);
        // A second reader of the affine temporary: folding would leave the add
        // alive, and the rewrite is no longer local.
        f.blocks[2].instructions.push(Instruction::Copy {
            dest: Value(20),
            src: Operand::Value(Value(3)),
        });
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }
    #[test]
    fn unsigned_byte_sequences_and_mirrored_bounds_are_exact() {
        let mut checked = 0;
        for start in [0, 1, 127, 254, 255] {
            for bound in 0..=255 {
                for step in [1, 2, 7, 128] {
                    for offset in [0, 1, 8, 127, 255] {
                        for (op, mirror) in [
                            (IrCmpOp::Ult, false),
                            (IrCmpOp::Ule, false),
                            (IrCmpOp::Ugt, true),
                            (IrCmpOp::Uge, true),
                        ] {
                            let mut f = counted_loop(
                                Ty::U8,
                                start,
                                offset,
                                if mirror {
                                    if op == IrCmpOp::Ugt {
                                        IrCmpOp::Ult
                                    } else {
                                        IrCmpOp::Ule
                                    }
                                } else {
                                    op
                                },
                                bound,
                                true,
                                step,
                                true,
                                true,
                            );
                            if mirror {
                                for inst in &mut f.blocks[1].instructions {
                                    if let Instruction::Cmp {
                                        lhs, rhs, op: o, ..
                                    } = inst
                                    {
                                        std::mem::swap(lhs, rhs);
                                        *o = op;
                                    }
                                }
                            }
                            if fold_affine_exit_compares(&mut f) == 0 {
                                continue;
                            }
                            let (folded_op, _, raw) = folded_cmp(&f).unwrap();
                            let cutoff = integer_constant(IrConst::I64(raw), Ty::U8).unwrap();
                            let inclusive = matches!(op, IrCmpOp::Ule | IrCmpOp::Uge);
                            assert_eq!(
                                folded_op,
                                if inclusive {
                                    IrCmpOp::Ule
                                } else {
                                    IrCmpOp::Ult
                                }
                            );
                            let mut v = start;
                            let mut exited = false;
                            for _ in 0..=256 {
                                let sum = (v + offset) & 255; // independent modulo-256 model
                                let old = if inclusive { sum <= bound } else { sum < bound };
                                let new = if inclusive {
                                    i128::from(v) <= cutoff
                                } else {
                                    i128::from(v) < cutoff
                                };
                                assert_eq!(old, new, "{start}/{step}/{offset}/{bound} at {v}");
                                if !old {
                                    exited = true;
                                    break;
                                }
                                v = (v + step) & 255;
                            }
                            assert!(exited);
                            checked += 1;
                        }
                    }
                }
            }
        }
        assert!(checked > 1000, "nonvacuous certificate sweep: {checked}");
    }

    #[test]
    fn unsigned_full_width_bounds_are_not_signed_raw_patterns() {
        for ty in [Ty::U8, Ty::U16, Ty::U32, Ty::U64] {
            let max = (1_i128 << (ty.size() * 8)) - 1;
            let mut good = counted_loop(
                ty,
                (max - 20) as i64,
                4,
                IrCmpOp::Ult,
                (max - 1) as i64,
                true,
                1,
                true,
                true,
            );
            assert_eq!(fold_affine_exit_compares(&mut good), 1);
            let (_, _, raw) = folded_cmp(&good).unwrap();
            assert_eq!(integer_constant(IrConst::I64(raw), ty), Some(max - 5));
            let mut bad = counted_loop(
                ty,
                (max - 15) as i64,
                8,
                IrCmpOp::Ule,
                (max - 2) as i64,
                true,
                8,
                true,
                true,
            );
            let before = format!("{bad:?}");
            assert_eq!(fold_affine_exit_compares(&mut bad), 0);
            assert_eq!(format!("{bad:?}"), before);
        }
    }

    #[test]
    fn affine_fold_rejects_signedness_mismatch_and_multi_def_webs() {
        for (ty, op) in [(Ty::U32, IrCmpOp::Slt), (Ty::I32, IrCmpOp::Ult)] {
            let mut f = counted_loop(ty, 0, 8, op, 256, true, 8, true, true);
            let before = format!("{f:?}");
            assert_eq!(fold_affine_exit_compares(&mut f), 0);
            assert_eq!(format!("{f:?}"), before);
        }
        let mut f = counted_loop(Ty::U64, 0, 8, IrCmpOp::Ule, 256, true, 8, true, true);
        f.blocks[1].instructions.insert(
            2,
            Instruction::Copy {
                dest: Value(3),
                src: Operand::Const(IrConst::I64(1000)),
            },
        );
        let before = format!("{f:?}");
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
        assert_eq!(format!("{f:?}"), before);
    }

    #[test]
    fn unsigned_folding_respects_the_existing_kill_switch() {
        let mut f = counted_loop(Ty::U32, 0, 8, IrCmpOp::Ule, 256, true, 8, true, true);
        let before = format!("{f:?}");
        let old = affine_fold_enabled();
        AFFINE_FOLD_ENABLED.with(|v| v.set(false));
        let result = fold_affine_exit_compares(&mut f);
        AFFINE_FOLD_ENABLED.with(|v| v.set(old));
        assert_eq!(result, 0);
        assert_eq!(format!("{f:?}"), before);
        assert_eq!(fold_affine_exit_compares(&mut f), 1);
    }

    #[test]
    fn signed_mirrored_comparisons_retain_the_original_orientation() {
        for (ty, op) in [(Ty::I32, IrCmpOp::Slt), (Ty::I64, IrCmpOp::Sle)] {
            let mut f = counted_loop(ty, 0, 4, op, 100, true, 1, true, true);
            for inst in &mut f.blocks[1].instructions {
                if let Instruction::Cmp {
                    lhs, rhs, op: o, ..
                } = inst
                {
                    std::mem::swap(lhs, rhs);
                    *o = if op == IrCmpOp::Slt {
                        IrCmpOp::Sgt
                    } else {
                        IrCmpOp::Sge
                    };
                }
            }
            assert_eq!(fold_affine_exit_compares(&mut f), 1);
            assert_eq!(folded_cmp(&f), Some((op, 2, 96)));
            for v in -32..=100 {
                assert_eq!(op.eval_i64(v + 4, 100), op.eval_i64(v, 96));
            }
        }
    }
    #[test]
    fn optional_exit_test_does_not_bound_the_induction_sequence() {
        let mut f = counted_loop(Ty::U32, 0, 4, IrCmpOp::Ult, 64, true, 1, true, true);
        f.blocks[0].instructions.push(Instruction::ParamRef {
            dest: Value(9),
            param_idx: 0,
            ty: Ty::I32,
        });
        f.blocks[1].terminator = Terminator::Branch(BlockId(2));
        f.blocks[2].terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(9)),
            true_label: BlockId(5),
            false_label: BlockId(3),
        };
        f.blocks.push(blk(
            5,
            vec![],
            Terminator::CondBranch {
                cond: Operand::Value(Value(4)),
                true_label: BlockId(3),
                false_label: BlockId(4),
            },
        ));
        let before = format!("{f:?}");
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
        assert_eq!(format!("{f:?}"), before);
    }

    #[test]
    fn a_shared_affine_source_keeps_its_existing_value_and_bound() {
        let mut f = counted_loop(Ty::I64, 0, 4, IrCmpOp::Slt, 64, true, 1, true, true);
        f.blocks[1].instructions.insert(
            1,
            Instruction::BinOp {
                dest: Value(9),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Const(IrConst::I64(3)),
                ty: Ty::I64,
            },
        );
        if let Instruction::BinOp { lhs, .. } = &mut f.blocks[1].instructions[2] {
            *lhs = Operand::Value(Value(9));
        }
        f.blocks[2].instructions.push(Instruction::Copy {
            dest: Value(10),
            src: Operand::Value(Value(9)),
        });
        assert_eq!(fold_affine_exit_compares(&mut f), 1);
        assert!(matches!(
            f.blocks[1].instructions[3],
            Instruction::Cmp {
                op: IrCmpOp::Slt,
                lhs: Operand::Value(Value(9)),
                rhs: Operand::Const(IrConst::I64(60)),
                ..
            }
        ));
    }
    #[test]
    fn folds_proven_unsigned_header_comparisons() {
        for ty in [Ty::U8, Ty::U16, Ty::U32, Ty::U64] {
            let mut f = counted_loop(ty, 0, 4, IrCmpOp::Ult, 64, true, 1, true, true);
            assert_eq!(fold_affine_exit_compares(&mut f), 1);
            assert_eq!(folded_cmp(&f), Some((IrCmpOp::Ult, 2, 60)));
        }
        let mut f = counted_loop(Ty::U64, 0, 8, IrCmpOp::Ule, 4096, true, 8, true, true);
        assert_eq!(fold_affine_exit_compares(&mut f), 1);
        assert_eq!(folded_cmp(&f), Some((IrCmpOp::Ule, 2, 4088)));
    }

    #[test]
    fn unsigned_fold_refuses_wrap_and_unproven_entry() {
        for (start, add, bound, step) in [(-1, 4, 64, 1), (0, 100, 127, 250), (0, 4, 2, 1)] {
            let mut f = counted_loop(
                Ty::U8,
                start,
                add,
                IrCmpOp::Ult,
                bound,
                true,
                step,
                true,
                true,
            );
            assert_eq!(fold_affine_exit_compares(&mut f), 0);
        }
        let mut runtime = counted_loop(Ty::U64, 0, 8, IrCmpOp::Ule, 4096, true, 8, false, true);
        assert_eq!(fold_affine_exit_compares(&mut runtime), 0);
        let mut mixed = counted_loop(Ty::U32, 0, 4, IrCmpOp::Slt, 64, true, 1, true, true);
        assert_eq!(fold_affine_exit_compares(&mut mixed), 0);
    }

    #[test]
    fn unsigned_fold_proves_the_failing_test_does_not_wrap() {
        let mut f = counted_loop(Ty::U8, 0, 100, IrCmpOp::Ult, 127, true, 1, true, true);
        if let Instruction::BinOp { rhs, .. } = &mut f.blocks[3].instructions[0] {
            *rhs = Operand::Const(IrConst::I64(250));
        }
        // 0+100 < 127, then 250+100 wraps to 94 < 127: subtracting
        // 100 from the bound would incorrectly stop at the second test.
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
    }

    #[test]
    fn unsigned_fold_uses_a_dominating_nonwrapping_guard() {
        let mut f = counted_loop(Ty::U32, 0, 4, IrCmpOp::Ult, 64, true, 1, true, true);
        f.blocks[1].instructions.push(Instruction::BinOp {
            dest: Value(8),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(2)),
            rhs: Operand::Const(IrConst::I32(1)),
            ty: Ty::U32,
        });
        f.blocks[1].instructions.push(Instruction::Cmp {
            dest: Value(9),
            op: IrCmpOp::Ult,
            lhs: Operand::Value(Value(8)),
            rhs: Operand::Const(IrConst::I32(100)),
            ty: Ty::U32,
        });
        assert_eq!(fold_affine_exit_compares(&mut f), 2);
        assert!(matches!(
            &f.blocks[1].instructions[4],
            Instruction::Cmp {
                lhs: Operand::Value(Value(2)),
                ..
            }
        ));

        let mut optional = counted_loop(Ty::U32, 0, 4, IrCmpOp::Ult, 64, true, 1, true, true);
        let guard = optional.blocks[1].instructions.split_off(1);
        optional.blocks[2].instructions = guard;
        optional.blocks[1].terminator = Terminator::Branch(BlockId(2));
        optional.blocks[2].terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(4)),
            true_label: BlockId(3),
            false_label: BlockId(4),
        };
        // This body test is mandatory and dominates every backedge.
        // Upstream's stronger dominance certificate safely admits it.
        assert_eq!(fold_affine_exit_compares(&mut optional), 1);
    }

    #[test]
    fn signed_fold_requires_an_executed_nonwrapping_guard() {
        let mut optional = counted_loop(
            Ty::I32,
            0,
            2,
            IrCmpOp::Slt,
            100,
            true,
            1073741823,
            true,
            true,
        );
        let guard = optional.blocks[1].instructions.split_off(1);
        optional.blocks[2].instructions = guard;
        optional.blocks[1].instructions.push(Instruction::Cmp {
            dest: Value(10),
            op: IrCmpOp::Ne,
            lhs: Operand::Value(Value(2)),
            rhs: Operand::Const(IrConst::I32(0)),
            ty: Ty::I32,
        });
        optional.blocks[1].terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(10)),
            true_label: BlockId(2),
            false_label: BlockId(3),
        };
        optional.blocks[2].terminator = Terminator::CondBranch {
            cond: Operand::Value(Value(4)),
            true_label: BlockId(3),
            false_label: BlockId(4),
        };
        assert_eq!(fold_affine_exit_compares(&mut optional), 0);

        // Both an independent refusal and a positive witness: retaining safe
        // body comparisons does not require weakening the controlling proof.
        for (guard_c, bound, step, expected) in
            [(1073741830, 2147483644, 1073741823, 0), (4, 64, 1, 2)]
        {
            let mut f = counted_loop(
                Ty::I32,
                0,
                guard_c,
                IrCmpOp::Slt,
                bound,
                true,
                step,
                true,
                true,
            );
            f.blocks[2].instructions.push(Instruction::BinOp {
                dest: Value(8),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Const(IrConst::I32(100)),
                ty: Ty::I32,
            });
            f.blocks[2].instructions.push(Instruction::Cmp {
                dest: Value(9),
                op: IrCmpOp::Slt,
                lhs: Operand::Value(Value(8)),
                rhs: Operand::Const(IrConst::I32(1000)),
                ty: Ty::I32,
            });
            assert_eq!(fold_affine_exit_compares(&mut f), expected);
        }
    }
}

#[cfg(test)]
mod affine_fold_orientation_tests {
    //! `canonicalise_affine_exit_cmps` folds the rotated latch's cloned exit
    //! comparison.  Its algebra is the whole content of the pass, and it had a
    //! wrong half: with the affine operand on the RIGHT it swapped the operands
    //! *and* mirrored the operator, and two negations do not cancel, so
    //! `N > iv + C` became `(N - C) < iv` -- for N=100, C=4, iv=0 the true
    //! `100 > 4` came out as the false `96 < 0`.
    //!
    //! These tests do not check the SHAPE of the rewrite; they check that it is
    //! an EQUIVALENCE, by evaluating the source predicate and the rewritten
    //! comparison over a sweep of induction-variable values, for every signed
    //! relation in both orientations.  A shape assertion would have passed on
    //! the broken version -- it produced a comparison of exactly the expected
    //! shape -- which is precisely why the algebra needs an oracle.
    use super::*;

    /// The IV value both sides are evaluated at.
    const IV: u32 = 1;
    /// The cloned `iv + C` temporary.
    const TEMP: u32 = 100;

    fn build(flipped: bool, op: IrCmpOp, c: i64, n: i64) -> Vec<Instruction> {
        let add = Instruction::BinOp {
            dest: Value(TEMP),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(IV)),
            rhs: Operand::Const(IrConst::from_i64(c, IrType::I64)),
            ty: IrType::I64,
        };
        let n_konst = Operand::Const(IrConst::from_i64(n, IrType::I64));
        let cmp = Instruction::Cmp {
            dest: Value(101),
            lhs: if flipped {
                n_konst.clone()
            } else {
                Operand::Value(Value(TEMP))
            },
            op,
            rhs: if flipped {
                Operand::Value(Value(TEMP))
            } else {
                n_konst
            },
            ty: IrType::I64,
        };
        vec![add, cmp]
    }

    /// Evaluate the (possibly rewritten) comparison at `iv`.  Only the IV and
    /// constants may appear once the fold has run; the temporary appearing means
    /// the fold did not fire.
    fn eval_folded(cmp: &Instruction, iv: i64) -> Option<bool> {
        let Instruction::Cmp { lhs, op, rhs, .. } = cmp else {
            return None;
        };
        let value = |o: &Operand| -> Option<i64> {
            match o {
                Operand::Const(c) => c.to_i64(),
                Operand::Value(v) if v.0 == IV => Some(iv),
                _ => None,
            }
        };
        Some(op.eval_i64(value(lhs)?, value(rhs)?))
    }

    /// The source predicate, evaluated directly from the C-level relation.
    fn eval_source(flipped: bool, op: IrCmpOp, c: i64, n: i64, iv: i64) -> bool {
        let temp = iv + c;
        if flipped {
            op.eval_i64(n, temp)
        } else {
            op.eval_i64(temp, n)
        }
    }

    #[test]
    fn all_signed_relations_in_both_orientations_are_equivalences() {
        let relations = [IrCmpOp::Slt, IrCmpOp::Sle, IrCmpOp::Sgt, IrCmpOp::Sge];
        let mut checked = 0usize;
        for flipped in [false, true] {
            for op in relations {
                for &c in &[0i64, 1, 4, 7] {
                    for &n in &[0i64, 1, 5, 16, 4096] {
                        for iv in [-33i64, -1, 0, 1, 2, 5, 17, 100, 4095] {
                            let mut insts = build(flipped, op, c, n);
                            let folded =
                                canonicalise_affine_exit_cmps(&mut insts, Some(Value(101)));
                            // This algebra-only oracle supplies an explicit
                            // proof token; evaluate only its nonwrapping domain.
                            // Admission of that token is tested by real loops.
                            if iv.checked_add(c).is_none() {
                                continue;
                            }
                            // The fold is allowed to refuse when `N - C` is not
                            // representable; when it fires it must be exact.
                            if folded == 0 {
                                continue;
                            }
                            let Some(after) = eval_folded(&insts[1], iv) else {
                                // Still refers to the temporary: refused.
                                continue;
                            };
                            let before = eval_source(flipped, op, c, n, iv);
                            assert_eq!(
                                before, after,
                                "fold changed the predicate: flipped={flipped} op={op:?} c={c} \
                                 n={n} iv={iv} ({before} -> {after})"
                            );
                            checked += 1;
                        }
                    }
                }
            }
        }
        assert!(
            checked > 400,
            "expected a broad sweep, checked only {checked}"
        );
    }

    #[test]
    fn a_clone_needs_a_proof_for_its_own_comparison() {
        let original = build(false, IrCmpOp::Slt, 4, 100);
        let mut missing = original.clone();
        assert_eq!(canonicalise_affine_exit_cmps(&mut missing, None), 0);
        assert_eq!(format!("{missing:?}"), format!("{original:?}"));
        let mut unrelated = original.clone();
        assert_eq!(
            canonicalise_affine_exit_cmps(&mut unrelated, Some(Value(999))),
            0
        );
        assert_eq!(format!("{unrelated:?}"), format!("{original:?}"));
    }

    #[test]
    fn the_flipped_oracle_catches_the_regression_it_was_written_for() {
        // The exact shape the audit reported: `100 > (iv + 4)` at `iv = 0` is
        // true.  The broken rewrite produced `96 < 0` (false).  Pin the value
        // pair so the oracle above cannot silently stop covering it.
        let mut insts = build(true, IrCmpOp::Sgt, 4, 100);
        assert_eq!(
            canonicalise_affine_exit_cmps(&mut insts, Some(Value(101))),
            1
        );
        let after = eval_folded(&insts[1], 0).expect("fold must leave a constant bound");
        assert!(after, "100 > (0 + 4) must stay true after folding");
        assert_eq!(eval_source(true, IrCmpOp::Sgt, 4, 100, 0), after);
    }
}

#[cfg(test)]
mod affine_value_bound_tests {
    //! `fold_loop_value_bound` is an EQUIVALENCE under a guard, and every
    //! clause of its proof has a counterexample when dropped.  The oracle is
    //! therefore behavioural: a wrapping interpreter runs the loop before and
    //! after the fold and the TRIP COUNTS must agree for every bound in a
    //! sweep that includes the type extremes — the exact values where the
    //! unsound variants (no guard, non-strict compare, step 2) diverge.
    use super::*;
    use crate::ir::reexports::BasicBlock;

    const N_PARAM: u32 = 1;
    const GUARD_CMP: u32 = 2;
    const PHI: u32 = 3;
    const INC: u32 = 4;
    const AFFINE: u32 = 5;
    const EXIT_CMP: u32 = 6;
    const RELAY_BLOCK: u32 = 9;

    struct Shape {
        ty: IrType,
        start: i64,
        step: i64,
        /// `Some(c)` chains the affine add onto the post-increment
        /// (`(phi + step) + c`); `None` uses the reassociated `phi + c_total`.
        chain_c: Option<i64>,
        c_total: i64,
        cmp_op: IrCmpOp,
        /// Mirror the exit compare to `n <op'> affine`.
        cmp_mirrored: bool,
        guard: Guard,
        relay: bool,
    }

    enum Guard {
        /// `CondBranch(K <op> n, loop, exit)`.
        TrueEdge(i64, IrCmpOp),
        /// `CondBranch(n <op> K, exit, loop)` — the loop is the FALSE edge.
        FalseEdgeMirrored(i64, IrCmpOp),
        /// Unconditional entry: nothing proven.
        None,
    }

    fn blk(label: u32, insts: Vec<Instruction>, term: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions: insts,
            terminator: term,
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        }
    }

    /// ```text
    ///   0 entry:  n = param0 ; [guard cmp] ; guard-branch (or Branch)
    ///   9 relay:  (optional) Branch -> 1
    ///   1 loop:   phi = [start, pre], [inc, 1]
    ///             inc = phi + step
    ///             affine = (inc + c) | (phi + c_total)
    ///             cmp ; CondBranch(cmp, 1, 2)   (mirrored: operands swapped)
    ///   2 exit:   Return
    /// ```
    fn fixture(s: &Shape) -> IrFunction {
        let ty = s.ty;
        let mut f = IrFunction::new("affold_vb".into(), IrType::I32, vec![], false);
        f.next_value_id = 50;
        let entry_label = if s.relay {
            BlockId(RELAY_BLOCK)
        } else {
            BlockId(0)
        };
        let loop_target = if s.relay {
            BlockId(RELAY_BLOCK)
        } else {
            BlockId(1)
        };
        let mut entry = vec![Instruction::ParamRef {
            dest: Value(N_PARAM),
            param_idx: 0,
            ty,
        }];
        let entry_term = match s.guard {
            Guard::TrueEdge(k, op) => {
                entry.push(Instruction::Cmp {
                    dest: Value(GUARD_CMP),
                    lhs: Operand::Const(IrConst::from_i64(k, ty)),
                    op,
                    rhs: Operand::Value(Value(N_PARAM)),
                    ty,
                });
                Terminator::CondBranch {
                    cond: Operand::Value(Value(GUARD_CMP)),
                    true_label: loop_target,
                    false_label: BlockId(2),
                }
            }
            Guard::FalseEdgeMirrored(k, op) => {
                entry.push(Instruction::Cmp {
                    dest: Value(GUARD_CMP),
                    lhs: Operand::Value(Value(N_PARAM)),
                    op,
                    rhs: Operand::Const(IrConst::from_i64(k, ty)),
                    ty,
                });
                Terminator::CondBranch {
                    cond: Operand::Value(Value(GUARD_CMP)),
                    true_label: BlockId(2),
                    false_label: loop_target,
                }
            }
            Guard::None => Terminator::Branch(loop_target),
        };
        let mut body = vec![
            Instruction::Phi {
                dest: Value(PHI),
                ty,
                incoming: vec![
                    (Operand::Const(IrConst::from_i64(s.start, ty)), entry_label),
                    (Operand::Value(Value(INC)), BlockId(1)),
                ],
            },
            Instruction::BinOp {
                dest: Value(INC),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(PHI)),
                rhs: Operand::Const(IrConst::from_i64(s.step, ty)),
                ty,
            },
        ];
        match s.chain_c {
            Some(c) => body.push(Instruction::BinOp {
                dest: Value(AFFINE),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(INC)),
                rhs: Operand::Const(IrConst::from_i64(c, ty)),
                ty,
            }),
            None => body.push(Instruction::BinOp {
                dest: Value(AFFINE),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(PHI)),
                rhs: Operand::Const(IrConst::from_i64(s.c_total, ty)),
                ty,
            }),
        }
        let (cmp_lhs, cmp_rhs) = if s.cmp_mirrored {
            (
                Operand::Value(Value(N_PARAM)),
                Operand::Value(Value(AFFINE)),
            )
        } else {
            (
                Operand::Value(Value(AFFINE)),
                Operand::Value(Value(N_PARAM)),
            )
        };
        body.push(Instruction::Cmp {
            dest: Value(EXIT_CMP),
            lhs: cmp_lhs,
            op: s.cmp_op,
            rhs: cmp_rhs,
            ty,
        });
        f.blocks.push(blk(0, entry, entry_term));
        f.blocks.push(blk(
            1,
            body,
            Terminator::CondBranch {
                cond: Operand::Value(Value(EXIT_CMP)),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
        ));
        f.blocks.push(blk(2, vec![], Terminator::Return(None)));
        if s.relay {
            f.blocks
                .push(blk(RELAY_BLOCK, vec![], Terminator::Branch(BlockId(1))));
        }
        f
    }

    /// Wrapping interpreter.  Returns the number of loop-block executions,
    /// saturated at the bail-out bound so an (expected) non-terminating
    /// variant still yields a comparable observation.
    fn trips(f: &IrFunction, n: i128) -> u32 {
        const BAIL: u32 = 4096;
        let wrap = |v: i128, ty: IrType| -> i128 {
            let (lo, hi) = integer_type_bounds(ty).unwrap();
            let span = hi - lo + 1;
            lo + (v - lo).rem_euclid(span)
        };
        let read = |op: Operand, vals: &FxHashMap<u32, i128>| -> i128 {
            match op {
                Operand::Const(c) => c.to_i64().map(i128::from).unwrap(),
                Operand::Value(v) => vals[&v.0],
            }
        };
        let mut vals: FxHashMap<u32, i128> = FxHashMap::default();
        let mut at = f.blocks[0].label;
        let mut previous = at;
        let mut count = 0u32;
        loop {
            let block = f.blocks.iter().find(|b| b.label == at).unwrap();
            if at == BlockId(1) {
                count += 1;
                if count >= BAIL {
                    return BAIL;
                }
            }
            let phis: Vec<(u32, i128)> = block
                .instructions
                .iter()
                .filter_map(|inst| match inst {
                    Instruction::Phi { dest, incoming, .. } => {
                        let op = incoming.iter().find(|(_, l)| *l == previous).unwrap().0;
                        Some((dest.0, read(op, &vals)))
                    }
                    _ => None,
                })
                .collect();
            vals.extend(phis);
            for inst in &block.instructions {
                match inst {
                    Instruction::Phi { .. } => {}
                    Instruction::ParamRef { dest, ty, .. } => {
                        vals.insert(dest.0, wrap(n, *ty));
                    }
                    Instruction::Copy { dest, src } => {
                        let v = read(*src, &vals);
                        vals.insert(dest.0, v);
                    }
                    Instruction::BinOp {
                        dest,
                        op,
                        lhs,
                        rhs,
                        ty,
                    } => {
                        let (l, r) = (read(*lhs, &vals), read(*rhs, &vals));
                        let raw = match op {
                            IrBinOp::Add => l + r,
                            IrBinOp::Sub => l - r,
                            other => panic!("fixture does not use {other:?}"),
                        };
                        vals.insert(dest.0, wrap(raw, *ty));
                    }
                    Instruction::Cmp {
                        dest,
                        op,
                        lhs,
                        rhs,
                        ty,
                    } => {
                        // Signed relations read the wrapped (lo..=hi) image
                        // directly; unsigned types already wrap into 0..=hi.
                        let (l, r) = (wrap(read(*lhs, &vals), *ty), wrap(read(*rhs, &vals), *ty));
                        let t = match op {
                            IrCmpOp::Slt | IrCmpOp::Ult => l < r,
                            IrCmpOp::Sle | IrCmpOp::Ule => l <= r,
                            IrCmpOp::Sgt | IrCmpOp::Ugt => l > r,
                            IrCmpOp::Sge | IrCmpOp::Uge => l >= r,
                            other => panic!("fixture does not use {other:?}"),
                        };
                        vals.insert(dest.0, i128::from(t));
                    }
                    other => panic!("fixture does not use {other:?}"),
                }
            }
            previous = at;
            at = match block.terminator {
                Terminator::Branch(next) => next,
                Terminator::CondBranch {
                    cond,
                    true_label,
                    false_label,
                } => {
                    if read(cond, &vals) != 0 {
                        true_label
                    } else {
                        false_label
                    }
                }
                Terminator::Return(_) => return count,
                _ => panic!("unexpected fixture terminator"),
            };
        }
    }

    fn sweep(ty: IrType) -> Vec<i128> {
        let (lo, hi) = integer_type_bounds(ty).unwrap();
        vec![
            lo,
            lo + 3,
            lo + 4,
            lo + 5,
            lo + 6,
            -1,
            0,
            1,
            3,
            4,
            5,
            6,
            7,
            20,
            hi - 5,
            hi - 1,
            hi,
        ]
        .into_iter()
        .filter(|&v| v >= lo && v <= hi)
        .collect()
    }

    /// Fold, assert the expected fire count, and when it fired assert trip
    /// equivalence against the unfolded original over the whole sweep.
    fn check(s: &Shape, expect_fold: usize) -> IrFunction {
        let original = fixture(s);
        let mut folded = fixture(s);
        assert_eq!(
            fold_affine_exit_compares(&mut folded),
            expect_fold,
            "unexpected fold count for {:?} c_total={} step={}",
            s.cmp_op,
            s.c_total,
            s.step
        );
        for &n in &sweep(s.ty) {
            assert_eq!(
                trips(&original, n),
                trips(&folded, n),
                "trip divergence at n={n} for {:?} c_total={} step={}",
                s.cmp_op,
                s.c_total,
                s.step
            );
        }
        folded
    }

    fn base() -> Shape {
        Shape {
            ty: IrType::I64,
            start: 0,
            step: 1,
            chain_c: Some(4),
            c_total: 5,
            cmp_op: IrCmpOp::Slt,
            cmp_mirrored: false,
            guard: Guard::TrueEdge(4, IrCmpOp::Slt),
            relay: false,
        }
    }

    #[test]
    fn vb_folds_signed_postinc_chain() {
        let folded = check(&base(), 1);
        // The guard block gained the hoisted `n - 4`, and the compare now
        // reads the post-increment against it.
        let sub = folded.blocks[0]
            .instructions
            .iter()
            .find_map(|i| match i {
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Sub,
                    lhs: Operand::Value(nv),
                    rhs: Operand::Const(k),
                    ..
                } if nv.0 == N_PARAM => Some((dest.0, k.to_i64().unwrap())),
                _ => None,
            })
            .expect("hoisted sub in the guard block");
        assert_eq!(sub.1, 4, "the chain anchors on the post-increment");
        let cmp = folded.blocks[1].instructions.iter().find_map(|i| match i {
            Instruction::Cmp { lhs, op, rhs, .. } => Some((*lhs, *op, *rhs)),
            _ => None,
        });
        assert_eq!(
            cmp,
            Some((
                Operand::Value(Value(INC)),
                IrCmpOp::Slt,
                Operand::Value(Value(sub.0))
            ))
        );
    }

    #[test]
    fn vb_reassociated_chain_prefers_postinc_anchor() {
        // `phi + 5` (the shape instcombine leaves) must still anchor on the
        // post-increment: comparing the stale phi would keep two IV values
        // live across the bottom test.
        let s = Shape {
            chain_c: None,
            ..base()
        };
        let folded = check(&s, 1);
        let cmp = folded.blocks[1].instructions.iter().find_map(|i| match i {
            Instruction::Cmp { lhs, .. } => Some(*lhs),
            _ => None,
        });
        assert_eq!(cmp, Some(Operand::Value(Value(INC))));
    }

    #[test]
    fn vb_folds_unsigned() {
        let s = Shape {
            ty: IrType::U64,
            cmp_op: IrCmpOp::Ult,
            guard: Guard::TrueEdge(4, IrCmpOp::Ult),
            ..base()
        };
        check(&s, 1);
    }

    #[test]
    fn vb_folds_mirrored_compare_and_false_edge_guard() {
        // `n > affine` with entry `CondBranch(n <= 4, exit, loop)`: the fact
        // on the false edge is the negation `n > 4`.
        let s = Shape {
            cmp_op: IrCmpOp::Sgt,
            cmp_mirrored: true,
            guard: Guard::FalseEdgeMirrored(4, IrCmpOp::Sle),
            ..base()
        };
        check(&s, 1);
    }

    #[test]
    fn vb_folds_through_relay_preheader() {
        let s = Shape {
            relay: true,
            ..base()
        };
        check(&s, 1);
    }

    #[test]
    fn vb_refuses_unguarded() {
        // Near the signed minimum, `n - c` underflows to a huge positive
        // bound: the sweep (which includes `lo + 3`) would catch a fold here
        // as a trip divergence.  There must be no fold at all.
        let s = Shape {
            guard: Guard::None,
            ..base()
        };
        check(&s, 0);
    }

    #[test]
    fn vb_refuses_non_strict_compare() {
        // With `<=` and `n == type_max`, the original wrapping loop never
        // exits while the folded one would.
        let s = Shape {
            cmp_op: IrCmpOp::Sle,
            ..base()
        };
        check(&s, 0);
    }

    #[test]
    fn vb_refuses_step_two() {
        // A step of 2 overshoots: near `type_max` the original `iv + c`
        // wraps and the compares diverge.
        let s = Shape { step: 2, ..base() };
        check(&s, 0);
    }

    #[test]
    fn vb_guard_strength_boundary() {
        // `first = start + c_total = 5`.  A strict guard needs `K >= 4`:
        // `K = 4` proves `n >= 5` and folds, `K = 3` only proves `n >= 4`
        // and must refuse.
        let strong = Shape {
            guard: Guard::TrueEdge(4, IrCmpOp::Slt),
            ..base()
        };
        check(&strong, 1);
        let weak = Shape {
            guard: Guard::TrueEdge(3, IrCmpOp::Slt),
            ..base()
        };
        check(&weak, 0);
        // Non-strict facts shift by one: `4 <= n` is too weak, `5 <= n` folds.
        let weak_nonstrict = Shape {
            guard: Guard::TrueEdge(4, IrCmpOp::Sle),
            ..base()
        };
        check(&weak_nonstrict, 0);
        let strong_nonstrict = Shape {
            guard: Guard::TrueEdge(5, IrCmpOp::Sle),
            ..base()
        };
        check(&strong_nonstrict, 1);
    }

    #[test]
    fn vb_refuses_domain_mismatch() {
        // An UNSIGNED guard fact says nothing about SIGNED order: a negative
        // `n` is enormous unsigned, so `4 <u n` holds while `n - 4` still
        // underflows the signed compare.
        let s = Shape {
            guard: Guard::TrueEdge(4, IrCmpOp::Ult),
            ..base()
        };
        check(&s, 0);
    }

    #[test]
    fn vb_kill_switch_disables() {
        set_affine_fold_knobs(false, false);
        let mut f = fixture(&base());
        assert_eq!(fold_affine_exit_compares(&mut f), 0);
        set_affine_fold_knobs(true, false);
    }
}
