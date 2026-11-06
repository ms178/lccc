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
    let bail = |why: &str| {
        if dbg {
            lccc::cli::stderr_line(format_args!("[IVSR-LFTR] header={header} keep: {why}"));
        }
        0
    };
    if crate::common::types::target_ptr_size() != 8 {
        return bail("not an LP64 target");
    }
    // Top-tested: the header holds the exit test, the one body block is the
    // latch and branches back unconditionally.  Bottom-tested: one block.
    let top_tested = match natural_loop.body.len() {
        1 if header == back_block => false,
        2 if header != back_block
            && natural_loop.body.contains(&back_block)
            && matches!(func.blocks[back_block].terminator,
                Terminator::Branch(t) if t == func.blocks[header].label) =>
        {
            true
        }
        _ => return bail("not a one- or two-block counted loop"),
    };
    let header_label = func.blocks[header].label;
    let latch_label = func.blocks[back_block].label;
    let preheader_label = func.blocks[preheader].label;
    let continue_label = if top_tested {
        latch_label
    } else {
        header_label
    };
    let test = &func.blocks[header];
    let Terminator::CondBranch {
        cond: Operand::Value(cond),
        true_label,
        false_label,
    } = &test.terminator
    else {
        return bail("the exit test is not a conditional branch");
    };
    let cond = *cond;
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
        return bail("the test does not separate the loop from its exit");
    };
    let Some((cmp_idx, op, lhs, rhs, cmp_ty)) = cmp_in(test, cond) else {
        return bail("the exit condition is not a compare in the header");
    };
    let op = if continue_on_true { op } else { op.inverse() };

    // The counter: a basic IV of step +-1, tested before (top) or after
    // (bottom) its increment.
    let phi_incoming = |phi: Value| {
        func.blocks[header]
            .instructions
            .iter()
            .find_map(|inst| match inst {
                Instruction::Phi { dest, incoming, .. } if *dest == phi => Some(incoming.clone()),
                _ => None,
            })
    };
    let mut found = None;
    for iv in basic_ivs {
        if !step_is_supported(iv.step) || iv.ty != cmp_ty || !iv.ty.is_integer() {
            continue;
        }
        let Some(incoming) = phi_incoming(iv.phi_dest) else {
            continue;
        };
        let Some(next) = incoming.iter().find_map(|(o, l)| match o {
            Operand::Value(v) if *l == latch_label => Some(*v),
            _ => None,
        }) else {
            continue;
        };
        let tested = if top_tested { iv.phi_dest } else { next };
        if let Some((rel, bound)) = relation_on(op, &lhs, &rhs, &Operand::Value(tested)) {
            found = Some((iv, next, rel, bound));
            break;
        }
    }
    let Some((iv, i_next, rel, bound)) = found else {
        return bail("no power-of-two-step counter in the exit test");
    };
    let up = iv.step > 0;
    let (inclusive, clampable) = match (up, rel) {
        (true, IrCmpOp::Slt | IrCmpOp::Ult) | (false, IrCmpOp::Sgt | IrCmpOp::Ugt) => (false, true),
        (true, IrCmpOp::Sle | IrCmpOp::Ule) | (false, IrCmpOp::Sge | IrCmpOp::Uge) => (true, true),
        // `!=` counts modulo the counter's width: exact only for a unit
        // step at 64 bits, where `T = D` needs no clamp (D = 0 exactly when
        // the test fails on entry).
        (_, IrCmpOp::Ne) if iv.ty.size() == 8 && iv.step.abs() == 1 => (false, false),
        _ => return bail("relation does not bound the counter"),
    };
    match bound {
        Operand::Const(_) => {}
        Operand::Value(v) if is_loop_invariant(v.0, &natural_loop.body, func) => {}
        _ => return bail("the bound is not loop-invariant"),
    }
    let latch = &func.blocks[back_block];
    let Some(inc_idx) = latch
        .instructions
        .iter()
        .position(|inst| inst.dest() == Some(i_next))
    else {
        return bail("the increment is not in the latch");
    };
    if !matches!(
        &latch.instructions[inc_idx],
        Instruction::BinOp {
            op: IrBinOp::Add | IrBinOp::Sub,
            ..
        }
    ) {
        return bail("the increment is not an add");
    }
    // First iteration: proven (no clamp) by the preheader guard or by
    // constants; otherwise the trip count is clamped below.
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

    // A dereferenced pointer recurrence q = phi(q0, q + k), k a constant.
    let mut derefs = dereferenced_roots(&func.blocks[header]);
    if top_tested {
        derefs.extend(dereferenced_roots(latch));
    }
    let mut pointer = None;
    for inst in &func.blocks[header].instructions {
        let Instruction::Phi {
            dest,
            ty: IrType::Ptr,
            incoming,
        } = inst
        else {
            continue;
        };
        if incoming.len() != 2 || !derefs.contains(&dest.0) {
            continue;
        }
        let init = incoming
            .iter()
            .find_map(|(o, l)| (*l == preheader_label).then_some(*o));
        let next = incoming.iter().find_map(|(o, l)| match o {
            Operand::Value(v) if *l == latch_label => Some(*v),
            _ => None,
        });
        let (Some(Operand::Value(q0)), Some(q_next)) = (init, next) else {
            continue;
        };
        let step = latch.instructions.iter().find_map(|i| match i {
            Instruction::GetElementPtr {
                dest: d,
                base,
                offset: Operand::Const(c),
                ..
            } if *d == q_next && *base == *dest => c.to_i64(),
            _ => None,
        });
        if let Some(k) = step.filter(|&k| k != 0) {
            pointer = Some((*dest, q0, q_next, k));
            break;
        }
    }
    let Some((q, q0, q_next, k)) = pointer else {
        return bail("no dereferenced pointer recurrence");
    };

    // The counter must have no other live use: deadness-aware use counts.
    // Deadness is only established inside the loop's blocks, where IVSR's
    // own rewrite leaves the dead address arithmetic; a use anywhere else
    // counts as live (fail closed), which also bounds this scan.
    let (users, in_terminator) = users(func);
    let mut dead: FxHashSet<(usize, usize)> = FxHashSet::default();
    let loop_blocks: Vec<usize> = if top_tested {
        vec![header, back_block]
    } else {
        vec![header]
    };
    loop {
        let mut grew = false;
        for &bi in &loop_blocks {
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
            break;
        }
    }
    let live_users = |v: Value| -> Vec<(usize, usize)> {
        let mut us: Vec<(usize, usize)> = users
            .get(&v.0)
            .map(|us| us.iter().copied().filter(|u| !dead.contains(u)).collect())
            .unwrap_or_default();
        us.sort_unstable();
        us
    };
    let Some(phi_idx) = func.blocks[header]
        .instructions
        .iter()
        .position(|inst| inst.dest() == Some(iv.phi_dest))
    else {
        return bail("the counter phi is missing");
    };
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
        return bail("the counter has other uses");
    }
    if dbg {
        lccc::cli::stderr_line(format_args!(
            "[IVSR-LFTR] header={header} pointer exit test (step {k} bytes, {}, {})",
            if top_tested {
                "top-tested"
            } else {
                "bottom-tested"
            },
            if proven { "guarded" } else { "clamped" }
        ));
    }

    // Preheader: T = (i0 REL n) ? D : floor, q_end = q0 + T * k.
    let mut next_id = func.next_value_id;
    let mut fresh = || {
        let v = Value(next_id);
        next_id += 1;
        v
    };
    let mut pre: Vec<Instruction> = Vec::new();
    let widen = |op: Operand, pre: &mut Vec<Instruction>, fresh: &mut dyn FnMut() -> Value| {
        if iv.ty.size() == 8 {
            return op;
        }
        let from_ty = if is_unsigned_rel(rel) {
            iv.ty.to_unsigned()
        } else {
            match iv.ty {
                IrType::U8 => IrType::I8,
                IrType::U16 => IrType::I16,
                IrType::U32 => IrType::I32,
                t => t,
            }
        };
        let d = fresh();
        pre.push(Instruction::Cast {
            dest: d,
            src: op,
            from_ty,
            to_ty: IrType::I64,
        });
        Operand::Value(d)
    };
    let (hi, lo) = if up {
        (bound, iv.init)
    } else {
        (iv.init, bound)
    };
    let hi = widen(hi, &mut pre, &mut fresh);
    let lo = widen(lo, &mut pre, &mut fresh);
    let mut trip = fresh();
    pre.push(Instruction::BinOp {
        dest: trip,
        op: IrBinOp::Sub,
        lhs: hi,
        rhs: lo,
        ty: IrType::I64,
    });
    // T = ceil(D / s) for an exclusive bound, floor(D / s) + 1 for an
    // inclusive one: (D + adj) >> log2(s) with adj = s - 1 or s (s = |step|
    // a power of two; D >= 0 whenever the clamp below keeps T).
    let s_abs = iv.step.unsigned_abs();
    let adj = (s_abs - 1 + u64::from(inclusive)) as i64;
    if adj != 0 {
        let t1 = fresh();
        pre.push(Instruction::BinOp {
            dest: t1,
            op: IrBinOp::Add,
            lhs: Operand::Value(trip),
            rhs: Operand::Const(IrConst::I64(adj)),
            ty: IrType::I64,
        });
        trip = t1;
    }
    if s_abs > 1 {
        let t1 = fresh();
        pre.push(Instruction::BinOp {
            dest: t1,
            op: IrBinOp::LShr,
            lhs: Operand::Value(trip),
            rhs: Operand::Const(IrConst::I64(i64::from(s_abs.trailing_zeros()))),
            ty: IrType::I64,
        });
        trip = t1;
    }
    if clampable && !proven {
        // Not entered (top) / entered once (bottom) when `i0 REL n` fails.
        let c = fresh();
        pre.push(Instruction::Cmp {
            dest: c,
            op: rel,
            lhs: iv.init,
            rhs: bound,
            ty: cmp_ty,
        });
        let t1 = fresh();
        pre.push(Instruction::Select {
            dest: t1,
            cond: Operand::Value(c),
            true_val: Operand::Value(trip),
            false_val: Operand::Const(IrConst::I64(i64::from(!top_tested))),
            ty: IrType::I64,
        });
        trip = t1;
    }
    let bytes = if k == 1 {
        trip
    } else {
        let b = fresh();
        let inst = if k > 0 && (k as u64).is_power_of_two() {
            Instruction::BinOp {
                dest: b,
                op: IrBinOp::Shl,
                lhs: Operand::Value(trip),
                rhs: Operand::Const(IrConst::I64(i64::from(k.trailing_zeros()))),
                ty: IrType::I64,
            }
        } else {
            Instruction::BinOp {
                dest: b,
                op: IrBinOp::Mul,
                lhs: Operand::Value(trip),
                rhs: Operand::Const(IrConst::I64(k)),
                ty: IrType::I64,
            }
        };
        pre.push(inst);
        b
    };
    let q_end = fresh();
    pre.push(Instruction::GetElementPtr {
        dest: q_end,
        base: q0,
        offset: Operand::Value(bytes),
        ty: IrType::I8,
    });

    // Doomed: the counter phi, its increment, the old compare, and every
    // dead pure instruction reading one of them (transitively).
    let mut doomed_vals: FxHashSet<u32> = [iv.phi_dest.0, i_next.0, cond.0].into_iter().collect();
    let mut doomed: FxHashSet<(usize, usize)> =
        [(header, phi_idx), (back_block, inc_idx), (header, cmp_idx)]
            .into_iter()
            .collect();
    loop {
        let mut grew = false;
        for &(bi, ii) in &dead {
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
            break;
        }
    }

    // Apply: delete doomed (highest index first per block), then add the
    // preheader code and the new test at the end of the header.
    let mut by_block: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
    for &(bi, ii) in &doomed {
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
    let ph = &mut func.blocks[preheader];
    let ph_spans = !ph.source_spans.is_empty();
    for inst in pre {
        ph.instructions.push(inst);
        if ph_spans {
            ph.source_spans.push(Span::dummy());
        }
    }
    let tested = if top_tested { q } else { q_next };
    let block = &mut func.blocks[header];
    block.instructions.push(Instruction::Cmp {
        dest: cond,
        op: if continue_on_true {
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
    func.next_value_id = next_id;
    1
}
