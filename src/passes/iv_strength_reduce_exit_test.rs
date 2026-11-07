//! Linear-function test replacement for IVSR's pointer recurrences.
//!
//! `for (long i = 0; i < n; i++) s += p[i];` walks a pointer
//! `q = phi(p, q + 4)` after IVSR, but keeps the counter `i = phi(0, i + 1)`
//! alive for the exit test `i + 1 < n` alone: an add per iteration and a
//! register for nothing.  GCC 16.2 and Clang 23.1.5 test the pointer against
//! its end value instead (AArch64 `ldr w9, [x0], #4; ...; cmp x0, x8; b.ne`,
//! RISC-V `addi a0, a0, 4; bne a0, a1`).  This step does the same for
//! single-block loops on the targets that ask for it (AArch64 and RISC-V:
//! x86 keeps its index form for SIB addressing, see `univsr`):
//!
//! ```text
//!   preheader:  T = n - i0   (64-bit; +1 for an inclusive bound; i0 - n for
//!                             a descending counter)
//!               q_end = q0 + T * k
//!   latch:      continue while q_next != q_end
//! ```
//!
//! Exactness.  The latch continues while `i_next REL n`, `i_next = i + s`,
//! with `s = +1` and REL one of `<`, `<=`, `!=` or `s = -1` and REL one of
//! `>`, `>=`, `!=` (signed or unsigned, `n` loop-invariant).  The loop is
//! entered only when `i0 REL n` holds -- the preheader's guard tests exactly
//! that, or `i0` and `n` are constants satisfying it.  Then the latch test
//! fails for the first time at iteration T >= 1, T as above computed in the
//! counter's own signedness and widened to 64 bits (exact: the difference of
//! two values of a domain no wider than 64 bits, taken modulo 2^64, is the
//! true non-negative distance).  The pointer test fails at the first `j >= 1`
//! with `j * k == T * k (mod 2^64)`.  The pointer is dereferenced on every
//! iteration (the loop is one block), so its T addresses lie in one object,
//! whose size is below 2^63 (C17 6.5.6p8): `0 < (T - j) * |k| < 2^64` for
//! every earlier `j`, and the first such `j` is T.
//!
//! The counter, its increment and the old compare are then dead -- the only
//! uses of the counter were the increment, the compare and dead pure
//! instructions IVSR's own rewrite left behind (the multiply that fed the old
//! address), all of which are deleted here so the IR stays well formed.

use super::loop_analysis::NaturalLoop;
use super::{BasicIV, is_loop_invariant};
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::source::Span;
use crate::common::types::IrType;
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Terminator,
    Value,
};

/// The relation `lhs OP rhs` read as `iv REL other` when `iv` is one side:
/// `Some(REL)`, mirrored when the counter is the right operand.
fn relation_on(
    op: IrCmpOp,
    lhs: &Operand,
    rhs: &Operand,
    iv: &Operand,
) -> Option<(IrCmpOp, Operand)> {
    if lhs == iv && rhs != iv {
        Some((op, *rhs))
    } else if rhs == iv && lhs != iv {
        Some((op.swapped(), *lhs))
    } else {
        None
    }
}

fn is_unsigned_rel(op: IrCmpOp) -> bool {
    matches!(
        op,
        IrCmpOp::Ult | IrCmpOp::Ule | IrCmpOp::Ugt | IrCmpOp::Uge
    )
}

/// The defining `Cmp` of `cond` in `block`: (index, op, lhs, rhs, ty).
fn cmp_in(block: &BasicBlock, cond: Value) -> Option<(usize, IrCmpOp, Operand, Operand, IrType)> {
    block
        .instructions
        .iter()
        .enumerate()
        .find_map(|(k, inst)| match inst {
            Instruction::Cmp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            } if *dest == cond => Some((k, *op, *lhs, *rhs, *ty)),
            _ => None,
        })
}

/// Whether the loop is entered only when `init REL bound` holds: the guard
/// of the preheader (or of the single-predecessor chain of unconditional
/// blocks above it) tests exactly that relation, or both are constants
/// satisfying it.
fn guard_proves(
    func: &IrFunction,
    preds: &crate::ir::analysis::FlatAdj,
    preheader: usize,
    header_label: BlockId,
    rel: IrCmpOp,
    cmp_ty: IrType,
    init: &Operand,
    bound: &Operand,
) -> bool {
    if let (Operand::Const(a), Operand::Const(b)) = (init, bound)
        && let (Some(a), Some(b)) = (a.to_i64(), b.to_i64())
    {
        let (a, b) = match cmp_ty.size() {
            4 if is_unsigned_rel(rel) => (a as u32 as i64, b as u32 as i64),
            4 => (a as i32 as i64, b as i32 as i64),
            _ => (a, b),
        };
        return rel.eval_i64(a, b);
    }
    let mut block = preheader;
    let mut target = header_label;
    for _ in 0..3 {
        let b = &func.blocks[block];
        match &b.terminator {
            Terminator::CondBranch {
                cond: Operand::Value(c),
                true_label,
                false_label,
            } => {
                let enter_on_true = *true_label == target && *false_label != target;
                let enter_on_false = *false_label == target && *true_label != target;
                if !(enter_on_true || enter_on_false) {
                    return false;
                }
                let Some((_, op, lhs, rhs, ty)) = cmp_in(b, *c) else {
                    return false;
                };
                if ty != cmp_ty {
                    return false;
                }
                let op = if enter_on_true { op } else { op.inverse() };
                return relation_on(op, &lhs, &rhs, init)
                    .is_some_and(|(grel, gbound)| grel == rel && gbound == *bound);
            }
            Terminator::Branch(t) if *t == target => {
                // An unconditional block between the guard and the loop.
                let row = preds.row(block);
                if row.len() != 1 {
                    return false;
                }
                target = b.label;
                block = row[0] as usize;
            }
            _ => return false,
        }
    }
    false
}

/// Values the loop dereferences: every `Load`/`Store` address in `block`,
/// resolved through `Copy` and constant-offset `GetElementPtr` to roots.
fn dereferenced_roots(block: &BasicBlock) -> FxHashSet<u32> {
    let mut def: FxHashMap<u32, u32> = FxHashMap::default();
    for inst in &block.instructions {
        match inst {
            Instruction::Copy {
                dest,
                src: Operand::Value(v),
            } => {
                def.insert(dest.0, v.0);
            }
            Instruction::GetElementPtr {
                dest,
                base,
                offset: Operand::Const(_),
                ..
            } => {
                def.insert(dest.0, base.0);
            }
            _ => {}
        }
    }
    let root = |mut v: u32| {
        for _ in 0..8 {
            match def.get(&v) {
                Some(&next) => v = next,
                None => break,
            }
        }
        v
    };
    let mut out = FxHashSet::default();
    for inst in &block.instructions {
        match inst {
            Instruction::Load { ptr, .. } | Instruction::Store { ptr, .. } => {
                out.insert(root(ptr.0));
            }
            _ => {}
        }
    }
    out
}

/// Counter steps the trip-count formula handles: a power of two in
/// magnitude, at most 64 (`T = (D + adj) >> log2 |s|`).
fn step_is_supported(step: i64) -> bool {
    let a = step.unsigned_abs();
    a.is_power_of_two() && a <= 64
}

/// Whether the counter `iv` of a loop is used only by address arithmetic
/// (GEP offsets, directly or through one value-preserving widening cast),
/// its own increment and compares inside the loop: exactly the counters a
/// pointer recurrence plus the exit-test replacement can retire.  IVSR
/// asks this before turning a free stride-1 index into a pointer that the
/// counter would otherwise have to keep company with (AArch64 `ascii_prefix`
/// returns `i`: the pointer form paid a bump per iteration).
pub(super) fn counter_is_dedicated(
    func: &IrFunction,
    loop_body: &FxHashSet<usize>,
    iv: &BasicIV,
) -> bool {
    let mut next: Option<Value> = None;
    let mut cast_dests: FxHashSet<u32> = FxHashSet::default();
    let phi = iv.phi_dest;
    // Pass 1: the increment and the casts of the counter.
    for &bi in loop_body {
        let Some(block) = func.blocks.get(bi) else {
            continue;
        };
        for inst in &block.instructions {
            match inst {
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add | IrBinOp::Sub,
                    lhs,
                    rhs,
                    ..
                } if *lhs == Operand::Value(phi) || *rhs == Operand::Value(phi) => {
                    if next.is_some() {
                        return false;
                    }
                    next = Some(*dest);
                }
                Instruction::Cast {
                    dest,
                    src: Operand::Value(v),
                    from_ty,
                    to_ty,
                } if *v == phi && from_ty.cast_preserves_offset_value(*to_ty) => {
                    cast_dests.insert(dest.0);
                }
                _ => {}
            }
        }
    }
    let Some(next) = next else {
        return false;
    };
    // Pass 2: every use of the counter, its increment and its casts.
    for (bi, block) in func.blocks.iter().enumerate() {
        let inside = loop_body.contains(&bi);
        for inst in &block.instructions {
            let mut reads = [false; 3];
            inst.for_each_used_value(|v| {
                reads[0] |= v == phi.0;
                reads[1] |= v == next.0;
                reads[2] |= cast_dests.contains(&v);
            });
            if !(reads[0] || reads[1] || reads[2]) {
                continue;
            }
            if !inside {
                return false;
            }
            let ok = match inst {
                Instruction::Phi { dest, .. } => *dest == phi && !reads[0] && !reads[2],
                Instruction::BinOp { dest, .. } => *dest == next,
                Instruction::Cast { dest, .. } => cast_dests.contains(&dest.0),
                Instruction::Cmp { .. } => !reads[2],
                Instruction::GetElementPtr { offset, base, .. } => {
                    !reads_value(base, phi, next, &cast_dests)
                        && matches!(offset, Operand::Value(_))
                }
                _ => false,
            };
            if !ok {
                return false;
            }
        }
        let mut in_term = false;
        block.terminator.for_each_used_value(|v| {
            in_term |= v == phi.0 || v == next.0 || cast_dests.contains(&v);
        });
        if in_term {
            return false;
        }
    }
    true
}

fn reads_value(v: &Value, phi: Value, next: Value, casts: &FxHashSet<u32>) -> bool {
    *v == phi || *v == next || casts.contains(&v.0)
}

/// Side-effect-free instructions a dead result makes deletable.
fn is_pure(inst: &Instruction) -> bool {
    matches!(
        inst,
        Instruction::BinOp { .. }
            | Instruction::UnaryOp { .. }
            | Instruction::Cast { .. }
            | Instruction::Copy { .. }
            | Instruction::GetElementPtr { .. }
            | Instruction::Cmp { .. }
            | Instruction::Select { .. }
    )
}

/// Every instruction (block, index) reading each value, and whether any
/// terminator reads it.
fn users(func: &IrFunction) -> (FxHashMap<u32, Vec<(usize, usize)>>, FxHashSet<u32>) {
    let mut by_value: FxHashMap<u32, Vec<(usize, usize)>> = FxHashMap::default();
    let mut in_terminator = FxHashSet::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            inst.for_each_used_value(|v| by_value.entry(v).or_default().push((bi, ii)));
        }
        block.terminator.for_each_used_value(|v| {
            in_terminator.insert(v);
        });
    }
    (by_value, in_terminator)
}

/// Everything `match_exit_test` established about one loop: the exit
/// compare, the counter it tests, the pointer recurrence that replaces it
/// and the instructions the rewrite deletes.
struct ExitTestPlan {
    header: usize,
    preheader: usize,
    top_tested: bool,
    continue_on_true: bool,
    /// The old exit condition; the new compare reuses its value id.
    cond: Value,
    cmp_ty: IrType,
    counter_ty: IrType,
    counter_init: Operand,
    counter_step: i64,
    /// The counter relation, read as `counter REL bound` on continuation.
    rel: IrCmpOp,
    bound: Operand,
    inclusive: bool,
    clampable: bool,
    /// The preheader guard (or constants) proves the first test passes.
    proven: bool,
    /// `q = phi(q0, q_next)`, `q_next = q + k`.
    q: Value,
    q0: Value,
    q_next: Value,
    k: i64,
    /// Deleted instructions: the counter phi, its increment, the old
    /// compare and every dead pure instruction reading one of them.
    doomed: FxHashSet<(usize, usize)>,
}

/// Top-tested (`header: test; latch: branch back`, two blocks) or
/// bottom-tested (one block); anything else is not a counted loop here.
fn loop_shape(
    func: &IrFunction,
    natural_loop: &NaturalLoop,
    back_block: usize,
) -> Result<bool, &'static str> {
    let header = natural_loop.header;
    match natural_loop.body.len() {
        1 if header == back_block => Ok(false),
        2 if header != back_block
            && natural_loop.body.contains(&back_block)
            && matches!(func.blocks[back_block].terminator,
                Terminator::Branch(t) if t == func.blocks[header].label) =>
        {
            Ok(true)
        }
        _ => Err("not a one- or two-block counted loop"),
    }
}

/// The header's exit compare, its relation oriented to hold on
/// continuation.
struct ExitCompare {
    cond: Value,
    continue_on_true: bool,
    cmp_idx: usize,
    op: IrCmpOp,
    lhs: Operand,
    rhs: Operand,
    ty: IrType,
}

fn exit_compare(
    func: &IrFunction,
    natural_loop: &NaturalLoop,
    continue_label: BlockId,
) -> Result<ExitCompare, &'static str> {
    let test = &func.blocks[natural_loop.header];
    let Terminator::CondBranch {
        cond: Operand::Value(cond),
        true_label,
        false_label,
    } = &test.terminator
    else {
        return Err("the exit test is not a conditional branch");
    };
    let leaves = |l: &BlockId| {
        func.blocks
            .iter()
            .position(|b| b.label == *l)
            .is_some_and(|bi| !natural_loop.body.contains(&bi))
    };
    let continue_on_true = if *true_label == continue_label && leaves(false_label) {
        true
    } else if *false_label == continue_label && leaves(true_label) {
        false
    } else {
        return Err("the test does not separate the loop from its exit");
    };
    let Some((cmp_idx, op, lhs, rhs, ty)) = cmp_in(test, *cond) else {
        return Err("the exit condition is not a compare in the header");
    };
    Ok(ExitCompare {
        cond: *cond,
        continue_on_true,
        cmp_idx,
        op: if continue_on_true { op } else { op.inverse() },
        lhs,
        rhs,
        ty,
    })
}

/// The counter: a basic IV of a supported step, tested before (top) or
/// after (bottom) its increment.  Returns it with its latch value, the
/// relation and the bound.
fn find_counter<'a>(
    func: &IrFunction,
    header: usize,
    latch_label: BlockId,
    top_tested: bool,
    basic_ivs: &'a [BasicIV],
    compare: (IrCmpOp, &Operand, &Operand, IrType),
) -> Option<(&'a BasicIV, Value, IrCmpOp, Operand)> {
    let (op, lhs, rhs, cmp_ty) = compare;
    basic_ivs.iter().find_map(|iv| {
        if !step_is_supported(iv.step) || iv.ty != cmp_ty || !iv.ty.is_integer() {
            return None;
        }
        let next = func.blocks[header]
            .instructions
            .iter()
            .find_map(|inst| match inst {
                Instruction::Phi { dest, incoming, .. } if *dest == iv.phi_dest => {
                    incoming.iter().find_map(|(o, l)| match o {
                        Operand::Value(v) if *l == latch_label => Some(*v),
                        _ => None,
                    })
                }
                _ => None,
            })?;
        let tested = if top_tested { iv.phi_dest } else { next };
        let (rel, bound) = relation_on(op, lhs, rhs, &Operand::Value(tested))?;
        Some((iv, next, rel, bound))
    })
}

/// `(inclusive, clampable)` for a counter of step direction `up` bounded by
/// `rel`, or `None` when the relation does not bound it.
fn bound_kind(up: bool, rel: IrCmpOp, ty: IrType, step: i64) -> Option<(bool, bool)> {
    match (up, rel) {
        (true, IrCmpOp::Slt | IrCmpOp::Ult) | (false, IrCmpOp::Sgt | IrCmpOp::Ugt) => {
            Some((false, true))
        }
        (true, IrCmpOp::Sle | IrCmpOp::Ule) | (false, IrCmpOp::Sge | IrCmpOp::Uge) => {
            Some((true, true))
        }
        // `!=` counts modulo the counter's width: exact only for a unit
        // step at 64 bits, where `T = D` needs no clamp (D = 0 exactly when
        // the test fails on entry).
        (_, IrCmpOp::Ne) if ty.size() == 8 && step.abs() == 1 => Some((false, false)),
        _ => None,
    }
}

/// A dereferenced pointer recurrence `q = phi(q0, q + k)`, `k` a nonzero
/// constant: `(q, q0, q_next, k)`.
fn find_pointer_recurrence(
    func: &IrFunction,
    header: usize,
    back_block: usize,
    preheader: usize,
    top_tested: bool,
) -> Option<(Value, Value, Value, i64)> {
    let latch = &func.blocks[back_block];
    let latch_label = latch.label;
    let preheader_label = func.blocks[preheader].label;
    let mut derefs = dereferenced_roots(&func.blocks[header]);
    if top_tested {
        derefs.extend(dereferenced_roots(latch));
    }
    func.blocks[header].instructions.iter().find_map(|inst| {
        let Instruction::Phi {
            dest,
            ty: IrType::Ptr,
            incoming,
        } = inst
        else {
            return None;
        };
        if incoming.len() != 2 || !derefs.contains(&dest.0) {
            return None;
        }
        let init = incoming
            .iter()
            .find_map(|(o, l)| (*l == preheader_label).then_some(*o));
        let next = incoming.iter().find_map(|(o, l)| match o {
            Operand::Value(v) if *l == latch_label => Some(*v),
            _ => None,
        });
        let (Some(Operand::Value(q0)), Some(q_next)) = (init, next) else {
            return None;
        };
        let k = latch.instructions.iter().find_map(|i| match i {
            Instruction::GetElementPtr {
                dest: d,
                base,
                offset: Operand::Const(c),
                ..
            } if *d == q_next && *base == *dest => c.to_i64(),
            _ => None,
        })?;
        (k != 0).then_some((*dest, q0, q_next, k))
    })
}

/// Pure instructions of `loop_blocks` whose results only dead
/// instructions read (transitively).  Deadness is only established inside
/// the loop's blocks, where IVSR's own rewrite leaves the dead address
/// arithmetic; a use anywhere else counts as live (fail closed), which also
/// bounds this scan.
fn dead_loop_instructions(
    func: &IrFunction,
    loop_blocks: &[usize],
    users: &FxHashMap<u32, Vec<(usize, usize)>>,
    in_terminator: &FxHashSet<u32>,
) -> FxHashSet<(usize, usize)> {
    let mut dead: FxHashSet<(usize, usize)> = FxHashSet::default();
    loop {
        let mut grew = false;
        for &bi in loop_blocks {
            for (ii, inst) in func.blocks[bi].instructions.iter().enumerate() {
                if dead.contains(&(bi, ii)) || !is_pure(inst) {
                    continue;
                }
                let Some(d) = inst.dest() else { continue };
                if in_terminator.contains(&d.0) {
                    continue;
                }
                let all_dead = users
                    .get(&d.0)
                    .is_none_or(|us| us.iter().all(|u| dead.contains(u)));
                if all_dead {
                    dead.insert((bi, ii));
                    grew = true;
                }
            }
        }
        if !grew {
            return dead;
        }
    }
}

/// The seeds plus every instruction of `dead` reading a doomed value
/// (transitively).
fn doomed_instructions(
    func: &IrFunction,
    dead: &FxHashSet<(usize, usize)>,
    seeds: [(usize, usize); 3],
    seed_values: [u32; 3],
) -> FxHashSet<(usize, usize)> {
    let mut doomed_vals: FxHashSet<u32> = seed_values.into_iter().collect();
    let mut doomed: FxHashSet<(usize, usize)> = seeds.into_iter().collect();
    loop {
        let mut grew = false;
        for &(bi, ii) in dead {
            if doomed.contains(&(bi, ii)) {
                continue;
            }
            let inst = &func.blocks[bi].instructions[ii];
            let mut reads_doomed = false;
            inst.for_each_used_value(|v| reads_doomed |= doomed_vals.contains(&v));
            if reads_doomed {
                doomed.insert((bi, ii));
                if let Some(d) = inst.dest() {
                    doomed_vals.insert(d.0);
                }
                grew = true;
            }
        }
        if !grew {
            return doomed;
        }
    }
}

/// Match one loop against the rewrite's preconditions (module docs).
fn match_exit_test(
    func: &IrFunction,
    natural_loop: &NaturalLoop,
    preds: &crate::ir::analysis::FlatAdj,
    preheader: usize,
    back_block: usize,
    basic_ivs: &[BasicIV],
) -> Result<ExitTestPlan, &'static str> {
    let header = natural_loop.header;
    let top_tested = loop_shape(func, natural_loop, back_block)?;
    let header_label = func.blocks[header].label;
    let latch_label = func.blocks[back_block].label;
    let continue_label = if top_tested {
        latch_label
    } else {
        header_label
    };
    let ExitCompare {
        cond,
        continue_on_true,
        cmp_idx,
        op,
        lhs,
        rhs,
        ty: cmp_ty,
    } = exit_compare(func, natural_loop, continue_label)?;
    let (iv, i_next, rel, bound) = find_counter(
        func,
        header,
        latch_label,
        top_tested,
        basic_ivs,
        (op, &lhs, &rhs, cmp_ty),
    )
    .ok_or("no power-of-two-step counter in the exit test")?;
    let (inclusive, clampable) = bound_kind(iv.step > 0, rel, iv.ty, iv.step)
        .ok_or("relation does not bound the counter")?;
    match bound {
        Operand::Const(_) => {}
        Operand::Value(v) if is_loop_invariant(v.0, &natural_loop.body, func) => {}
        _ => return Err("the bound is not loop-invariant"),
    }
    let latch = &func.blocks[back_block];
    let inc_idx = latch
        .instructions
        .iter()
        .position(|inst| inst.dest() == Some(i_next))
        .ok_or("the increment is not in the latch")?;
    if !matches!(
        &latch.instructions[inc_idx],
        Instruction::BinOp {
            op: IrBinOp::Add | IrBinOp::Sub,
            ..
        }
    ) {
        return Err("the increment is not an add");
    }
    // First iteration: proven (no clamp) by the preheader guard or by
    // constants; otherwise the trip count is clamped.
    let proven = guard_proves(
        func,
        preds,
        preheader,
        header_label,
        rel,
        cmp_ty,
        &iv.init,
        &bound,
    );
    let (q, q0, q_next, k) =
        find_pointer_recurrence(func, header, back_block, preheader, top_tested)
            .ok_or("no dereferenced pointer recurrence")?;

    // The counter must have no other live use: deadness-aware use counts.
    let (users, in_terminator) = users(func);
    let loop_blocks: Vec<usize> = if top_tested {
        vec![header, back_block]
    } else {
        vec![header]
    };
    let dead = dead_loop_instructions(func, &loop_blocks, &users, &in_terminator);
    let live_users = |v: Value| -> Vec<(usize, usize)> {
        let mut us: Vec<(usize, usize)> = users
            .get(&v.0)
            .map(|us| us.iter().copied().filter(|u| !dead.contains(u)).collect())
            .unwrap_or_default();
        us.sort_unstable();
        us
    };
    let phi_idx = func.blocks[header]
        .instructions
        .iter()
        .position(|inst| inst.dest() == Some(iv.phi_dest))
        .ok_or("the counter phi is missing")?;
    let mut want_phi = vec![(back_block, inc_idx)];
    let mut want_next = vec![(header, phi_idx)];
    if top_tested {
        want_phi.push((header, cmp_idx));
    } else {
        want_next.push((header, cmp_idx));
    }
    want_phi.sort_unstable();
    want_next.sort_unstable();
    if in_terminator.contains(&iv.phi_dest.0)
        || in_terminator.contains(&i_next.0)
        || live_users(iv.phi_dest) != want_phi
        || live_users(i_next) != want_next
        || !live_users(cond).is_empty()
    {
        return Err("the counter has other uses");
    }
    let doomed = doomed_instructions(
        func,
        &dead,
        [(header, phi_idx), (back_block, inc_idx), (header, cmp_idx)],
        [iv.phi_dest.0, i_next.0, cond.0],
    );
    Ok(ExitTestPlan {
        header,
        preheader,
        top_tested,
        continue_on_true,
        cond,
        cmp_ty,
        counter_ty: iv.ty,
        counter_init: iv.init,
        counter_step: iv.step,
        rel,
        bound,
        inclusive,
        clampable,
        proven,
        q,
        q0,
        q_next,
        k,
        doomed,
    })
}

/// Appends preheader instructions, allocating their value ids.
struct PreheaderCode<'a> {
    code: Vec<Instruction>,
    next_id: &'a mut u32,
}

impl PreheaderCode<'_> {
    fn push(&mut self, make: impl FnOnce(Value) -> Instruction) -> Value {
        let dest = Value(*self.next_id);
        *self.next_id += 1;
        self.code.push(make(dest));
        dest
    }

    fn binop(&mut self, op: IrBinOp, lhs: Operand, rhs: Operand) -> Value {
        self.push(|dest| Instruction::BinOp {
            dest,
            op,
            lhs,
            rhs,
            ty: IrType::I64,
        })
    }
}

/// The preheader code `T = (i0 REL n) ? D : floor; q_end = q0 + T * k`
/// (module docs), allocating value ids from `next_id`.  Returns the code
/// and `q_end`.
fn trip_end_code(plan: &ExitTestPlan, next_id: &mut u32) -> (Vec<Instruction>, Value) {
    let mut out = PreheaderCode {
        code: Vec::new(),
        next_id,
    };
    let ty = plan.counter_ty;
    let from_ty = if is_unsigned_rel(plan.rel) {
        ty.to_unsigned()
    } else {
        match ty {
            IrType::U8 => IrType::I8,
            IrType::U16 => IrType::I16,
            IrType::U32 => IrType::I32,
            t => t,
        }
    };
    let widen = |out: &mut PreheaderCode, src: Operand| {
        if ty.size() == 8 {
            return src;
        }
        Operand::Value(out.push(|dest| Instruction::Cast {
            dest,
            src,
            from_ty,
            to_ty: IrType::I64,
        }))
    };
    let (hi, lo) = if plan.counter_step > 0 {
        (plan.bound, plan.counter_init)
    } else {
        (plan.counter_init, plan.bound)
    };
    let hi = widen(&mut out, hi);
    let lo = widen(&mut out, lo);
    let mut trip = out.binop(IrBinOp::Sub, hi, lo);
    // T = ceil(D / s) for an exclusive bound, floor(D / s) + 1 for an
    // inclusive one: (D + adj) >> log2(s) with adj = s - 1 or s (s = |step|
    // a power of two; D >= 0 whenever the clamp below keeps T).
    let s_abs = plan.counter_step.unsigned_abs();
    let adj = (s_abs - 1 + u64::from(plan.inclusive)) as i64;
    if adj != 0 {
        let rhs = Operand::Const(IrConst::I64(adj));
        trip = out.binop(IrBinOp::Add, Operand::Value(trip), rhs);
    }
    if s_abs > 1 {
        let rhs = Operand::Const(IrConst::I64(i64::from(s_abs.trailing_zeros())));
        trip = out.binop(IrBinOp::LShr, Operand::Value(trip), rhs);
    }
    if plan.clampable && !plan.proven {
        // Not entered (top) / entered once (bottom) when `i0 REL n` fails.
        let c = out.push(|dest| Instruction::Cmp {
            dest,
            op: plan.rel,
            lhs: plan.counter_init,
            rhs: plan.bound,
            ty: plan.cmp_ty,
        });
        let floor = Operand::Const(IrConst::I64(i64::from(!plan.top_tested)));
        trip = out.push(|dest| Instruction::Select {
            dest,
            cond: Operand::Value(c),
            true_val: Operand::Value(trip),
            false_val: floor,
            ty: IrType::I64,
        });
    }
    let k = plan.k;
    let bytes = if k == 1 {
        trip
    } else if k > 0 && (k as u64).is_power_of_two() {
        let rhs = Operand::Const(IrConst::I64(i64::from(k.trailing_zeros())));
        out.binop(IrBinOp::Shl, Operand::Value(trip), rhs)
    } else {
        out.binop(
            IrBinOp::Mul,
            Operand::Value(trip),
            Operand::Const(IrConst::I64(k)),
        )
    };
    let q_end = out.push(|dest| Instruction::GetElementPtr {
        dest,
        base: plan.q0,
        offset: Operand::Value(bytes),
        ty: IrType::I8,
    });
    (out.code, q_end)
}

/// Delete the doomed instructions (highest index first per block), append
/// the preheader code and the new pointer test at the end of the header.
fn apply_exit_test(
    func: &mut IrFunction,
    plan: &ExitTestPlan,
    pre: Vec<Instruction>,
    q_end: Value,
) {
    let mut by_block: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
    for &(bi, ii) in &plan.doomed {
        by_block.entry(bi).or_default().push(ii);
    }
    for (bi, mut idxs) in by_block {
        idxs.sort_unstable_by(|a, b| b.cmp(a));
        let block = &mut func.blocks[bi];
        for ii in idxs {
            block.instructions.remove(ii);
            if !block.source_spans.is_empty() && ii < block.source_spans.len() {
                block.source_spans.remove(ii);
            }
        }
    }
    let ph = &mut func.blocks[plan.preheader];
    let ph_spans = !ph.source_spans.is_empty();
    for inst in pre {
        ph.instructions.push(inst);
        if ph_spans {
            ph.source_spans.push(Span::dummy());
        }
    }
    let tested = if plan.top_tested { plan.q } else { plan.q_next };
    let block = &mut func.blocks[plan.header];
    block.instructions.push(Instruction::Cmp {
        dest: plan.cond,
        op: if plan.continue_on_true {
            IrCmpOp::Ne
        } else {
            IrCmpOp::Eq
        },
        lhs: Operand::Value(tested),
        rhs: Operand::Value(q_end),
        ty: IrType::I64,
    });
    if !block.source_spans.is_empty() {
        block.source_spans.push(Span::dummy());
    }
}

/// Replace the counter-based exit test of a loop by a test of a dereferenced
/// pointer recurrence (see the module docs).  Two shapes: the top-tested
/// loop the frontend builds (`header: test i; latch: i + s, branch back`,
/// two blocks) and the bottom-tested single block (`do ... while (i + s
/// REL n)`).  Returns the number of rewrites (0 or 1).
pub(super) fn replace_counter_exit_test(
    func: &mut IrFunction,
    natural_loop: &NaturalLoop,
    preds: &crate::ir::analysis::FlatAdj,
    preheader: usize,
    back_block: usize,
    basic_ivs: &[BasicIV],
    dbg: bool,
) -> usize {
    let header = natural_loop.header;
    let plan = if crate::common::types::target_ptr_size() != 8 {
        Err("not an LP64 target")
    } else {
        match_exit_test(func, natural_loop, preds, preheader, back_block, basic_ivs)
    };
    let plan = match plan {
        Ok(plan) => plan,
        Err(why) => {
            if dbg {
                lccc::cli::stderr_line(format_args!("[IVSR-LFTR] header={header} keep: {why}"));
            }
            return 0;
        }
    };
    if dbg {
        lccc::cli::stderr_line(format_args!(
            "[IVSR-LFTR] header={header} pointer exit test (step {} bytes, {}, {})",
            plan.k,
            if plan.top_tested {
                "top-tested"
            } else {
                "bottom-tested"
            },
            if plan.proven { "guarded" } else { "clamped" }
        ));
    }
    let mut next_id = func.next_value_id;
    let (pre, q_end) = trip_end_code(&plan, &mut next_id);
    apply_exit_test(func, &plan, pre, q_end);
    func.next_value_id = next_id;
    1
}
