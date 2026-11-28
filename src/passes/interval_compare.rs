//! Interval predicates: two comparisons against one value become one unsigned
//! comparison against a wrapped difference.
//!
//! C spells a range test in at least four ways, and the frontend hands over
//! whichever one the source used:
//!
//! ```text
//! (x > 3) & (x < 10)                 bitwise conjunction of two compares
//! x > 3 && x < 10                    short-circuit, if-converted to a select
//! (x >= 4) && (x <= 9)               the other pair of bound spellings
//! (x < 4) | (x > 9)                  the complement, for rejection tests
//! ```
//!
//! All four are one machine comparison after a wrapped subtraction:
//!
//! ```text
//! (x >= L) & (x <= U)   ==   (uW)(x - L) <  U - L + 1
//! (x <  L) | (x >  U)   ==   (uW)(x - L) >  U - L
//! ```
//!
//! The subtraction wraps, which is exactly what makes the test work: a value
//! below the interval wraps to a huge unsigned number, so the single unsigned
//! compare covers both bounds.  GCC, Clang and ICX all perform this fold; LCCC
//! did not, and emitted two `setCC`/`movzx` pairs plus a logical combine where
//! one compare suffices (nine instructions against five on x86-64 for
//! `(x > 3) & (x < 10)`).
//!
//! # Admissibility
//!
//! * Both predicates must test the **same value** (after peeling copies) at the
//!   **same width and signedness**, and each must compare it against a
//!   constant.
//! * The interval must be non-empty and no wider than `2^31 - 1`.  A wider
//!   interval can put a genuine in-range value above the wrapped bound, and a
//!   signed `x - L` is only "huge" once its sign bit is the wrap: restricting
//!   the width keeps every displacement either entirely inside or entirely
//!   outside the unsigned window.
//! * Signed values are reinterpreted through an explicit same-width cast, so
//!   the subtraction is unsigned and the IR never claims that a signed
//!   operation wraps.
//! * The two source comparisons are left in place; they become dead and DCE
//!   retires them.  A shared comparison keeps its value, so code that also
//!   reads a bound directly is unaffected.
//!
//! Everything in this file is verified twice: by brute force over the 8- and
//! 16-bit rings in the unit tests, and by the behavioural gate, which compares
//! every emitted program against GCC 16.2 at `-O0` through `-O3`.

use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Value};

fn unsigned_of(ty: IrType) -> Option<IrType> {
    Some(match ty {
        IrType::I8 | IrType::U8 => IrType::U8,
        IrType::I16 | IrType::U16 => IrType::U16,
        IrType::I32 | IrType::U32 => IrType::U32,
        IrType::I64 | IrType::U64 => IrType::U64,
        _ => return None,
    })
}

fn bits_of(ty: IrType) -> u32 {
    // `IrType::size` is only implemented for the scalar types reachable here.
    match ty {
        IrType::I8 | IrType::U8 => 8,
        IrType::I16 | IrType::U16 => 16,
        IrType::I32 | IrType::U32 => 32,
        _ => 64,
    }
}

fn ring(value: i64, ty: IrType) -> u64 {
    let bits = bits_of(ty);
    if bits == 64 {
        value as u64
    } else {
        (value as u64) & ((1u64 << bits) - 1)
    }
}

fn signed_of(ty: IrType) -> bool {
    matches!(ty, IrType::I8 | IrType::I16 | IrType::I32 | IrType::I64)
}

/// Sign-extend a ring value of `bits` width to an `i64`.
fn extend_to_i64(value: u64, bits: u32) -> i64 {
    if bits == 64 {
        return value as i64;
    }
    let shift = 64 - bits;
    ((value << shift) as i64) >> shift
}

/// A comparison of one value against a constant, with the constant on the
/// right.
struct Predicate {
    op: IrCmpOp,
    value: Operand,
    constant: u64,
    ty: IrType,
}

fn peel(operand: Operand, defs: &[Option<Instruction>]) -> Operand {
    let mut current = operand;
    for _ in 0..8 {
        let Operand::Value(v) = current else {
            return current;
        };
        match defs.get(v.0 as usize).and_then(Option::as_ref) {
            Some(Instruction::Copy { src, .. }) => current = *src,
            _ => return current,
        }
    }
    current
}

fn predicate(operand: Operand, defs: &[Option<Instruction>]) -> Option<Predicate> {
    let Operand::Value(v) = peel(operand, defs) else {
        return None;
    };
    let Some(Instruction::Cmp {
        op, lhs, rhs, ty, ..
    }) = defs.get(v.0 as usize).and_then(Option::as_ref)
    else {
        return None;
    };
    if unsigned_of(*ty).is_none() {
        return None;
    }
    let (op, value, constant) = match (lhs, rhs) {
        (value, Operand::Const(c)) => (*op, *value, ring(c.to_i64()?, *ty)),
        (Operand::Const(c), value) => (op.swapped(), *value, ring(c.to_i64()?, *ty)),
        _ => return None,
    };
    Some(Predicate {
        op,
        value: peel(value, defs),
        constant,
        ty: *ty,
    })
}

fn same_value(a: &Operand, b: &Operand) -> bool {
    match (a, b) {
        (Operand::Value(x), Operand::Value(y)) => x == y,
        (Operand::Const(x), Operand::Const(y)) => x.to_i64() == y.to_i64(),
        _ => false,
    }
}

/// Salient bits of a width-limited interval test.
struct Interval {
    lower_bound: u64,
    /// Inclusive upper bound of the in-range window.
    upper: u64,
    /// `true` when the predicate is true *inside* `[lower, upper]`.
    inside: bool,
    /// Whether the bounds are ordered signed: the ordering of the two
    /// predicates the window was read from.  The opcode orders, not the
    /// type (`Ule` on an `I8` operand is what `range_fold` writes), and the
    /// two can differ.
    signed: bool,
    ty: IrType,
    value: Operand,
}

impl Interval {
    /// Number of values in the window, ordered in the COMPARISON's domain.
    ///
    /// The bounds are stored masked into the operand ring (`ring`), so the
    /// width cannot be read off the raw bits: `[-7, 7]` at 32 bits is stored
    /// `[0xFFFF_FFF9, 7]`, and a 64-bit subtraction calls that ~2^64 values
    /// wide -- which is why `x > -8 && x < 8` used to keep its two compares
    /// while `x > 3 && x < 10` folded.
    ///
    /// The fix is NOT to count the span around the ring, because the same
    /// masked bounds mean different things in the two domains:
    ///
    /// ```text
    /// signed    x > -8 && x < 8        stored [0xFFFF_FFF9, 7]   15 values
    /// unsigned  x >= 0xFFFFFFF7&&x<=10 stored [0xFFFFFFF7, 10]    0 values
    /// ```
    ///
    /// A signed window that crosses zero is contiguous in the ring, so ordering
    /// the SIGN-EXTENDED bounds gives both the count and (through `emit`'s
    /// `(x - L) < width`) the right test. An unsigned window is ordered
    /// unsigned, where an inverted pair is empty -- folding it as a ring span
    /// would accept both `0..10` and `0xFFFFFFF7..0xFFFFFFFF`, the phantom
    /// window `range_fold_branch.c` A11 exists to catch.
    fn width(&self) -> u64 {
        let bits = bits_of(self.ty);
        if self.signed {
            let (low, high) = (
                extend_to_i64(self.lower_bound, bits),
                extend_to_i64(self.upper, bits),
            );
            if low > high {
                return 0;
            }
            return (high as u64).wrapping_sub(low as u64).wrapping_add(1);
        }
        if self.lower_bound > self.upper {
            return 0;
        }
        self.upper.wrapping_sub(self.lower_bound).wrapping_add(1)
    }
}

/// The ordering a predicate reads its operand in: signed, unsigned, or --
/// for the two that read the bits alone -- none.
fn ordering(op: IrCmpOp) -> Option<bool> {
    match op {
        IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge => Some(true),
        IrCmpOp::Ult | IrCmpOp::Ule | IrCmpOp::Ugt | IrCmpOp::Uge => Some(false),
        IrCmpOp::Eq | IrCmpOp::Ne => None,
    }
}

/// The all-ones value of `ty`'s ring.
fn ring_mask(ty: IrType) -> u64 {
    u64::MAX >> (64 - bits_of(ty))
}

/// Where a ring value stands in the ordering: sign-extended for a signed
/// one, as it is for an unsigned one.  `i128` holds both.
fn rank(value: u64, signed: bool, ty: IrType) -> i128 {
    if signed {
        i128::from(extend_to_i64(value, bits_of(ty)))
    } else {
        i128::from(value)
    }
}

/// Recognize the four shapes:
///
/// ```text
/// (x >= L) & (x <= U)      inside [L, U]
/// (x >  L) & (x <  U)      inside [L+1, U-1]
/// (x <  L) | (x >  U)      outside [L, U]
/// (x == A) | (x == A + 1)  inside [A, A+1]   (membership)
/// ```
///
/// The complement forms are rewritten as an inside test whose window is the
/// excluded one shifted outward, so both polarities share one emission.
///
/// What is returned computes what the two predicates compute, for every
/// value of the operand (`every_pair_taken_for_an_interval_computes_what_the_
/// pair_does` walks the 8-bit ring through the emitted instructions).  That
/// rests on four refusals, each of which was a wrong answer once the pair
/// reached this pass unsimplified, as it does behind late threading:
///
/// * a strict bound at the edge of its domain (`x > MAX`, `x < MIN`) holds
///   for no `x` and is no bound: stepping it wraps to the other edge and
///   bounds everything ([`lower_bounds`], [`upper_bounds`]);
/// * the two predicates order alike, and the window is ordered as they do
///   -- not as the operand's type would;
/// * the two half-lines of a disjunction leave a gap, measured in that
///   ordering: `x >= MIN || x <= 0` leaves none and holds everywhere;
/// * `x != A || x != A + 1` holds everywhere too and is no window: only the
///   pair of equalities is.
fn interval(logic: IrBinOp, lhs: Predicate, rhs: Predicate) -> Option<Interval> {
    if lhs.ty != rhs.ty || !same_value(&lhs.value, &rhs.value) {
        return None;
    }
    let ty = lhs.ty;
    let mask = ring_mask(ty);
    // `{A, A+1}` membership: two neighbours of the ring, which is a window
    // in unsigned order whatever the type.  `x != A || x != A + 1` holds for
    // every `x` (no value equals both) and a mixed pair is one of its two
    // tests: neither is an interval, both are the constant folders' to
    // answer.  The pair of `!=` used to be taken for "outside the window",
    // which is what `x != A && x != A + 1` computes: `k += either(s[i])`
    // with `either` built from two inlined helpers counted the characters
    // that are neither of two, where every character counts.
    if logic == IrBinOp::Or && lhs.op == IrCmpOp::Eq && rhs.op == IrCmpOp::Eq {
        let (low, high) = (
            lhs.constant.min(rhs.constant),
            lhs.constant.max(rhs.constant),
        );
        if low == mask || high != low + 1 {
            return None;
        }
        return Some(Interval {
            lower_bound: low,
            upper: high,
            inside: true,
            signed: false,
            ty,
            value: lhs.value,
        });
    }
    let signed = match (ordering(lhs.op), ordering(rhs.op)) {
        (Some(first), Some(second)) if first == second => first,
        _ => return None,
    };
    let (value, window) = match logic {
        IrBinOp::And => {
            let (lo_pred, hi_pred) = match (lower_bounds(&lhs), lower_bounds(&rhs)) {
                (Some(_), None) => (lhs, rhs),
                (None, Some(_)) => (rhs, lhs),
                _ => return None,
            };
            let lower = lower_bounds(&lo_pred)?;
            let upper = upper_bounds(&hi_pred)?;
            // No `lower > upper` test here: the bounds live in the operand
            // ring, where a window that legitimately starts below zero stores
            // its lower bound above its upper one. `Interval::width` decides
            // emptiness in the comparison's own domain, and `emit` refuses an
            // empty or too-wide window (0 or > 2^31-1 values, or the whole
            // ring), so both a signed crossing window (`x > -8 && x < 8`)
            // and an inverted one (`x >= 10 && x <= 5`, `x >= 0xFFFFFFF7u &&
            // x <= 10u`) are handled correctly.
            (lo_pred.value, (lower, upper, true))
        }
        IrBinOp::Or => {
            // The complement of a window: `x <= U || x >= L`.
            let (upper_pred, lower_pred) = match (upper_bounds(&lhs), lower_bounds(&rhs)) {
                (Some(_), Some(_)) => (lhs, rhs),
                _ => match (upper_bounds(&rhs), lower_bounds(&lhs)) {
                    (Some(_), Some(_)) => (rhs, lhs),
                    _ => return None,
                },
            };
            let upper = upper_bounds(&upper_pred)?;
            let lower = lower_bounds(&lower_pred)?;
            // The true region is everything outside the gap `[upper + 1,
            // lower - 1]`.  Without a gap -- the half-lines touch or
            // overlap, in the ordering the predicates read -- the
            // disjunction holds everywhere and is no window.  (The bounds
            // were compared as ring values here, which called `x >= MIN ||
            // x <= 0` a window from 1 to MAX and refused `x < -3 || x >
            // 3`.)
            if rank(lower, signed, ty) - rank(upper, signed, ty) < 2 {
                return None;
            }
            (
                upper_pred.value,
                (
                    upper.wrapping_add(1) & mask,
                    lower.wrapping_sub(1) & mask,
                    false,
                ),
            )
        }
        _ => return None,
    };
    let (lower_bound, upper, inside) = window;
    Some(Interval {
        lower_bound,
        upper,
        inside,
        signed,
        ty,
        value,
    })
}

/// The least value `pred` admits, when it bounds its operand from below.
///
/// `x > MAX` admits none: it is no bound, and `MAX + 1` is the other edge
/// of the domain, which would admit all.
fn lower_bounds(pred: &Predicate) -> Option<u64> {
    let mask = ring_mask(pred.ty);
    let top = match pred.op {
        IrCmpOp::Sge | IrCmpOp::Uge => return Some(pred.constant),
        IrCmpOp::Sgt => mask >> 1,
        IrCmpOp::Ugt => mask,
        _ => return None,
    };
    (pred.constant != top).then(|| pred.constant.wrapping_add(1) & mask)
}

/// The greatest value `pred` admits, when it bounds its operand from above.
///
/// `x < MIN` admits none: it is no bound, and `MIN - 1` is the other edge
/// of the domain, which would admit all.
fn upper_bounds(pred: &Predicate) -> Option<u64> {
    let mask = ring_mask(pred.ty);
    let bottom = match pred.op {
        IrCmpOp::Sle | IrCmpOp::Ule => return Some(pred.constant),
        IrCmpOp::Slt => (mask >> 1) + 1,
        IrCmpOp::Ult => 0,
        _ => return None,
    };
    (pred.constant != bottom).then(|| pred.constant.wrapping_sub(1) & mask)
}

struct Expansion {
    block: usize,
    index: usize,
    dest: Value,
    replacement: Vec<Instruction>,
}

/// Emit the folded predicate: `(uW)(x - L) <op> bound`.
fn emit(
    interval: &Interval,
    final_dest: Value,
    final_ty: Option<IrType>,
    fresh: Value,
) -> Option<(Vec<Instruction>, u32)> {
    let unsigned_ty = unsigned_of(interval.ty)?;
    let width = interval.width();
    let mask = ring_mask(interval.ty);
    // A window of 2^31 or more lets an in-range value wrap past the bound; an
    // empty window is not a range test at all; and a window that is the
    // whole ring (`x >= MIN && x <= MAX` at 8 or 16 bits) holds everywhere,
    // with a count of values that the ring cannot spell: as a bound it is 0,
    // and `(x - L) < 0` holds nowhere.
    if width == 0 || width > (1u64 << 31) - 1 || width > mask {
        return None;
    }
    let mut body = Vec::new();
    let mut next = fresh;
    let value = if signed_of(interval.ty) {
        let cast = next;
        next = Value(next.0.checked_add(1)?);
        body.push(Instruction::Cast {
            dest: cast,
            src: interval.value,
            from_ty: interval.ty,
            to_ty: unsigned_ty,
        });
        Operand::Value(cast)
    } else {
        interval.value
    };
    // A window that starts at zero needs no subtraction at all; emitting
    // `x - 0` survived to the backend as a real `subl $0` and cost one
    // instruction per folded test.
    let difference = if interval.lower_bound == 0 {
        match value {
            Operand::Value(v) => v,
            Operand::Const(_) => return None,
        }
    } else {
        let dest = next;
        next = Value(next.0.checked_add(1)?);
        body.push(Instruction::BinOp {
            dest,
            op: IrBinOp::Sub,
            lhs: value,
            rhs: Operand::Const(IrConst::from_i64(interval.lower_bound as i64, unsigned_ty)),
            ty: unsigned_ty,
        });
        dest
    };
    let (op, bound) = if interval.inside {
        (IrCmpOp::Ult, width)
    } else {
        (
            IrCmpOp::Ugt,
            interval.upper.wrapping_sub(interval.lower_bound) & mask,
        )
    };
    let compared = next;
    body.push(Instruction::Cmp {
        dest: if final_ty.is_some() {
            compared
        } else {
            final_dest
        },
        op,
        lhs: Operand::Value(difference),
        rhs: Operand::Const(IrConst::from_i64(bound as i64, unsigned_ty)),
        ty: unsigned_ty,
    });
    if let Some(ty) = final_ty {
        next = Value(next.0.checked_add(1)?);
        body.push(Instruction::Cast {
            dest: final_dest,
            src: Operand::Value(compared),
            from_ty: IrType::U8,
            to_ty: ty,
        });
    }
    let used = next.0.saturating_sub(fresh.0);
    Some((body, used))
}

/// The two predicates of a value-producing boolean select, if it is one.
fn select_predicates(
    inst: &Instruction,
    defs: &[Option<Instruction>],
) -> Option<(IrBinOp, Predicate, Predicate, IrType)> {
    let Instruction::Select {
        cond,
        true_val,
        false_val,
        ty,
        ..
    } = inst
    else {
        return None;
    };
    if !matches!(ty, IrType::I8 | IrType::I16 | IrType::I32 | IrType::I64) {
        return None;
    }
    // One arm must be a zero-extended boolean (`zext(cmp)`), the other the
    // constant that makes the select a boolean operator: `c ? p : 0` is `c
    // && p` and `c ? 1 : p` is `c || p`.  The other two placements negate
    // the condition (`c ? p : 1` is `!c || p`, `c ? 0 : p` is `!c && p`) and
    // are neither.
    let (arm, logic) = if is_constant(false_val, 0) {
        (true_val, IrBinOp::And)
    } else if is_constant(true_val, 1) {
        (false_val, IrBinOp::Or)
    } else {
        return None;
    };
    let inner = zero_extended_predicate(*arm, defs)?;
    let outer = predicate(*cond, defs)?;
    Some((logic, outer, inner, *ty))
}

fn is_constant(operand: &Operand, value: i64) -> bool {
    matches!(operand, Operand::Const(c) if c.to_i64() == Some(value))
}

fn zero_extended_predicate(operand: Operand, defs: &[Option<Instruction>]) -> Option<Predicate> {
    // `zext i8 -> ty` of a compare, spelled as a cast the frontend produced.
    let Operand::Value(v) = peel(operand, defs) else {
        return None;
    };
    match defs.get(v.0 as usize).and_then(Option::as_ref) {
        Some(Instruction::Cast {
            src,
            from_ty: IrType::I8 | IrType::U8,
            ..
        }) => predicate(*src, defs),
        _ => None,
    }
}

pub(crate) fn run(func: &mut IrFunction) -> usize {
    let mut defs: Vec<Option<Instruction>> = vec![None; func.max_value_id() as usize + 1];
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                if let Some(slot) = defs.get_mut(dest.0 as usize) {
                    *slot = Some(inst.clone());
                }
            }
        }
    }
    let mut fresh = func.max_value_id().saturating_add(1);
    let mut expansions = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            let Some(dest) = inst.dest() else {
                continue;
            };
            // Shape 1: a bitwise combine of two compares.
            let mut candidate = None;
            if let Instruction::BinOp {
                op: logic @ (IrBinOp::And | IrBinOp::Or),
                lhs,
                rhs,
                ..
            } = inst
            {
                if let (Some(a), Some(b)) = (predicate(*lhs, &defs), predicate(*rhs, &defs)) {
                    if let Some(interval) = interval(*logic, a, b) {
                        candidate = emit(&interval, dest, None, Value(fresh));
                    }
                }
            }
            // Shape 2: the if-converted boolean select of the `&&`/`||` form.
            if candidate.is_none() {
                if let Some((logic, a, b, ty)) = select_predicates(inst, &defs) {
                    if let Some(interval) = interval(logic, a, b) {
                        candidate = emit(&interval, dest, Some(ty), Value(fresh));
                    }
                }
            }
            let Some((body, used)) = candidate else {
                continue;
            };
            let Some(next) = fresh.checked_add(used) else {
                continue;
            };
            fresh = next;
            expansions.push(Expansion {
                block: bi,
                index: ii,
                dest,
                replacement: body,
            });
        }
    }
    expansions.sort_by(|a, b| (b.block, b.index).cmp(&(a.block, a.index)));
    let changed = expansions.len();
    for Expansion {
        block: bi,
        index: ii,
        dest,
        replacement,
    } in expansions
    {
        let block = &mut func.blocks[bi];
        if block.instructions.get(ii).and_then(Instruction::dest) != Some(dest) {
            continue;
        }
        block.splice_inst(ii, replacement);
    }
    if fresh > func.next_value_id {
        func.next_value_id = fresh;
    }
    changed
}

/// Apply to every function body of a module; returns the number of rewrites.
pub(crate) fn run_on_module(module: &mut crate::ir::module::IrModule) -> usize {
    let mut total = 0;
    for func in module.functions.iter_mut() {
        if !func.is_declaration {
            total += run(func);
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};

    /// Reference implementation of the source predicate, for brute force.
    fn evaluate(op: IrCmpOp, x: i64, c: i64) -> bool {
        match op {
            IrCmpOp::Eq => x == c,
            IrCmpOp::Ne => x != c,
            IrCmpOp::Slt => x < c,
            IrCmpOp::Sle => x <= c,
            IrCmpOp::Sgt => x > c,
            IrCmpOp::Sge => x >= c,
            IrCmpOp::Ult => (x as u64) < (c as u64),
            IrCmpOp::Ule => (x as u64) <= (c as u64),
            IrCmpOp::Ugt => (x as u64) > (c as u64),
            IrCmpOp::Uge => (x as u64) >= (c as u64),
        }
    }

    /// Evaluate the emitted test on a signed value of the given width.
    fn folded(interval: &Interval, x: i64) -> bool {
        let unsigned_ty = unsigned_of(interval.ty).unwrap();
        let bits = bits_of(unsigned_ty);
        let mask = if bits == 64 {
            u64::MAX
        } else {
            (1u64 << bits) - 1
        };
        let d = (x as u64).wrapping_sub(interval.lower_bound) & mask;
        let width = interval.width();
        if interval.inside {
            d < width
        } else {
            d > interval.upper.wrapping_sub(interval.lower_bound) & mask
        }
    }

    #[test]
    fn every_signed_width_matches_brute_force() {
        for ty in [IrType::I8, IrType::I16] {
            let bits = bits_of(ty);
            let mask = if bits == 64 {
                u64::MAX
            } else {
                (1u64 << bits) - 1
            };
            let span = 1i64 << (bits / 2);
            // Sample lower bounds and widths, including the wrap points.
            for lower in [-span, -3, 0, 3, span - 1] {
                for width in [1i64, 2, 5, span] {
                    let interval = Interval {
                        lower_bound: (lower as u64) & mask,
                        upper: ((lower + width - 1) as u64) & mask,
                        inside: true,
                        signed: signed_of(ty),
                        ty,
                        value: Operand::Value(Value(0)),
                    };
                    if interval.width() == 0 || interval.width() > (1u64 << 31) - 1 {
                        continue;
                    }
                    let upper = lower + width - 1;
                    for x in (lower - 3)..=(upper + 3) {
                        let want = x >= lower && x <= upper;
                        assert_eq!(
                            folded(&interval, x),
                            want,
                            "{ty:?} [{lower}, {upper}] x={x}"
                        );
                    }
                }
            }
        }
    }

    /// What `body` (the instructions `emit` wrote) leaves in `result` when
    /// value 0 holds `x`: each instruction in the ring of its own type, a
    /// constant read as that type reads it.
    fn run_emitted(body: &[Instruction], result: Value, x: u64) -> u64 {
        let mut env: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
        env.insert(0, x);
        let read = |env: &std::collections::HashMap<u32, u64>, op: &Operand, ty: IrType| match op {
            Operand::Value(v) => env[&v.0] & ring_mask(ty),
            Operand::Const(c) => c.to_i64().expect("an integer") as u64 & ring_mask(ty),
        };
        for inst in body {
            let (dest, value) = match inst {
                Instruction::Cast {
                    dest,
                    src,
                    from_ty,
                    to_ty,
                } => {
                    let bits = read(&env, src, *from_ty);
                    let wide = if signed_of(*from_ty) {
                        extend_to_i64(bits, bits_of(*from_ty)) as u64
                    } else {
                        bits
                    };
                    (dest, wide & ring_mask(*to_ty))
                }
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Sub,
                    lhs,
                    rhs,
                    ty,
                } => (
                    dest,
                    read(&env, lhs, *ty).wrapping_sub(read(&env, rhs, *ty)) & ring_mask(*ty),
                ),
                Instruction::Cmp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => {
                    let (l, r) = (read(&env, lhs, *ty), read(&env, rhs, *ty));
                    let holds = match op {
                        IrCmpOp::Ult => l < r,
                        IrCmpOp::Ugt => l > r,
                        other => panic!("emit wrote a {other:?}"),
                    };
                    (dest, u64::from(holds))
                }
                other => panic!("emit wrote {other:?}"),
            };
            env.insert(dest.0, value);
        }
        env[&result.0]
    }

    /// What the two source predicates compute for `x`, in the ring of
    /// `bits` bits: the opcode orders, the width is the type's.
    fn source(
        logic: IrBinOp,
        first: (IrCmpOp, i64),
        second: (IrCmpOp, i64),
        x: u64,
        bits: u32,
    ) -> bool {
        let mask = u64::MAX >> (64 - bits);
        let holds = |(op, c): (IrCmpOp, i64)| {
            let (xu, cu) = (x & mask, (c as u64) & mask);
            let (xs, cs) = (extend_to_i64(xu, bits), extend_to_i64(cu, bits));
            match op {
                IrCmpOp::Eq => xu == cu,
                IrCmpOp::Ne => xu != cu,
                IrCmpOp::Slt => xs < cs,
                IrCmpOp::Sle => xs <= cs,
                IrCmpOp::Sgt => xs > cs,
                IrCmpOp::Sge => xs >= cs,
                IrCmpOp::Ult => xu < cu,
                IrCmpOp::Ule => xu <= cu,
                IrCmpOp::Ugt => xu > cu,
                IrCmpOp::Uge => xu >= cu,
            }
        };
        if logic == IrBinOp::And {
            holds(first) && holds(second)
        } else {
            holds(first) || holds(second)
        }
    }

    const ALL_PREDICATES: [IrCmpOp; 10] = [
        IrCmpOp::Eq,
        IrCmpOp::Ne,
        IrCmpOp::Slt,
        IrCmpOp::Sle,
        IrCmpOp::Sgt,
        IrCmpOp::Sge,
        IrCmpOp::Ult,
        IrCmpOp::Ule,
        IrCmpOp::Ugt,
        IrCmpOp::Uge,
    ];

    /// The recognizer and the emitter against the source: for every pair of
    /// the ten predicates, under both connectives, on a signed and on an
    /// unsigned 8-bit operand (so an opcode that orders against its type
    /// too), with constants at and next to every edge -- whatever is taken
    /// for an interval, the instructions emitted for it compute for each of
    /// the 256 values what the two predicates compute.  (The tests above
    /// check an interval against its own bounds; this one checks that the
    /// bounds are the source's, and that the emitted constants are the
    /// ring's.)
    #[test]
    fn every_pair_taken_for_an_interval_computes_what_the_pair_does() {
        let constants = [
            -128i64, -127, -126, -100, -3, -2, -1, 0, 1, 2, 3, 44, 45, 46, 100, 125, 126, 127,
        ];
        let mut taken = 0usize;
        let mut wrong = Vec::new();
        for ty in [IrType::I8, IrType::U8] {
            for logic in [IrBinOp::And, IrBinOp::Or] {
                for op1 in ALL_PREDICATES {
                    for op2 in ALL_PREDICATES {
                        for c1 in constants {
                            for c2 in constants {
                                let pred = |op, c| Predicate {
                                    op,
                                    value: Operand::Value(Value(0)),
                                    constant: ring(c, ty),
                                    ty,
                                };
                                let Some(found) = interval(logic, pred(op1, c1), pred(op2, c2))
                                else {
                                    continue;
                                };
                                let Some((body, _)) = emit(&found, Value(9), None, Value(10))
                                else {
                                    continue;
                                };
                                taken += 1;
                                let differs = (0..256u64).find(|&x| {
                                    (run_emitted(&body, Value(9), x) != 0)
                                        != source(logic, (op1, c1), (op2, c2), x, 8)
                                });
                                if let Some(x) = differs {
                                    wrong.push(format!(
                                        "{ty:?}: ({op1:?} {c1}) {logic:?} ({op2:?} {c2}) at x = {x}"
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(
            wrong.is_empty(),
            "{} of {taken} pairs are folded to another function, the first: {:?}",
            wrong.len(),
            &wrong[..wrong.len().min(8)]
        );
        assert!(taken > 4000, "{taken} pairs taken: the test reads nothing");
    }

    /// The same at 16 bits, where the whole ring is a count of values the
    /// width limit lets through: every window over a few bounds, the edges
    /// of both domains among them, against the two predicates at each
    /// value near a bound.
    #[test]
    fn sixteen_bit_windows_compute_what_their_pair_does() {
        let constants = [-32768i64, -32767, -2, -1, 0, 1, 2, 300, 32766, 32767];
        let probes: Vec<u64> = constants
            .iter()
            .flat_map(|&c| [c - 2, c - 1, c, c + 1, c + 2])
            .map(|c| c as u64 & 0xffff)
            .collect();
        let mut taken = 0usize;
        for ty in [IrType::I16, IrType::U16] {
            for logic in [IrBinOp::And, IrBinOp::Or] {
                for op1 in ALL_PREDICATES {
                    for op2 in ALL_PREDICATES {
                        for c1 in constants {
                            for c2 in constants {
                                let pred = |op, c| Predicate {
                                    op,
                                    value: Operand::Value(Value(0)),
                                    constant: ring(c, ty),
                                    ty,
                                };
                                let Some(found) = interval(logic, pred(op1, c1), pred(op2, c2))
                                else {
                                    continue;
                                };
                                let Some((body, _)) = emit(&found, Value(9), None, Value(10))
                                else {
                                    continue;
                                };
                                taken += 1;
                                for &x in &probes {
                                    assert_eq!(
                                        run_emitted(&body, Value(9), x) != 0,
                                        source(logic, (op1, c1), (op2, c2), x, 16),
                                        "{ty:?}: ({op1:?} {c1}) {logic:?} ({op2:?} {c2}) at x = {x}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(taken > 1000, "{taken} pairs taken: the test reads nothing");
    }

    /// What used to be taken for a window and is none, each by its own
    /// refusal; and what is one and used to be refused.
    #[test]
    fn what_is_no_window_is_refused_and_what_is_one_is_taken() {
        use IrCmpOp::{Sge, Sgt, Sle, Slt, Uge, Ule, Ult};
        let pred = |op, c: i64, ty| Predicate {
            op,
            value: Operand::Value(Value(0)),
            constant: ring(c, ty),
            ty,
        };
        let taken = |logic, a: (IrCmpOp, i64), b: (IrCmpOp, i64), ty| {
            interval(logic, pred(a.0, a.1, ty), pred(b.0, b.1, ty))
                .and_then(|found| emit(&found, Value(9), None, Value(10)))
                .is_some()
        };
        let i8 = IrType::I8;
        // A strict bound at the edge of its domain holds nowhere.
        assert!(!taken(IrBinOp::And, (Sge, 5), (Slt, -128), i8));
        assert!(!taken(IrBinOp::And, (Sgt, 127), (Sle, 5), i8));
        assert!(!taken(IrBinOp::And, (Uge, 5), (Ult, 0), IrType::U8));
        assert!(!taken(IrBinOp::Or, (Slt, -128), (Sgt, 3), i8));
        assert!(!taken(IrBinOp::Or, (Sle, 3), (Sgt, 127), i8));
        // Half-lines that touch or overlap leave no gap.
        assert!(!taken(IrBinOp::Or, (Sge, -128), (Sle, 0), i8));
        assert!(!taken(IrBinOp::Or, (Sle, 4), (Sge, 5), i8));
        assert!(!taken(IrBinOp::Or, (Sle, 9), (Sge, 4), i8));
        // The whole ring is no window.
        assert!(!taken(IrBinOp::And, (Sge, -128), (Sle, 127), i8));
        assert!(!taken(IrBinOp::And, (Uge, 0), (Ule, 255), IrType::U8));
        assert!(!taken(
            IrBinOp::And,
            (Sge, -32768),
            (Sle, 32767),
            IrType::I16
        ));
        // Predicates that order differently bound different things.
        assert!(!taken(IrBinOp::And, (Uge, 4), (Sle, 9), i8));
        assert!(!taken(IrBinOp::Or, (Ult, 4), (Sgt, 9), i8));
        // An unsigned window on a signed operand is ordered unsigned: 200
        // to 250 is one, 200 to 100 is empty.
        assert!(taken(IrBinOp::And, (Uge, -56), (Ule, -6), i8));
        assert!(!taken(IrBinOp::And, (Uge, -56), (Ule, 100), i8));
        // A gap that crosses zero is a gap.
        assert!(taken(IrBinOp::Or, (Slt, -3), (Sgt, 3), i8));
        // Neighbours across the sign change are neighbours of the ring.
        assert!(taken(
            IrBinOp::Or,
            (IrCmpOp::Eq, 127),
            (IrCmpOp::Eq, -128),
            i8
        ));
        // ... and the two ends of the ring are not.
        assert!(!taken(
            IrBinOp::Or,
            (IrCmpOp::Eq, -1),
            (IrCmpOp::Eq, 0),
            IrType::U8
        ));
    }

    /// A select is a connective by where its constant stands: `c ? p : 0`
    /// is `c && p`, `c ? 1 : p` is `c || p`, and the other two placements
    /// negate the condition and are left alone.
    #[test]
    fn a_select_is_the_connective_its_constant_makes_it() {
        // `x > 3` and `x < 10` over an `int`, combined by the select.
        let select = |true_val: Operand, false_val: Operand| {
            let mut func = IrFunction::new("sel".into(), IrType::I64, vec![], false);
            let v = Operand::Value;
            func.blocks.push(BasicBlock {
                label: BlockId(0),
                instructions: vec![
                    Instruction::ParamRef {
                        dest: Value(0),
                        param_idx: 0,
                        ty: IrType::I32,
                    },
                    Instruction::Cmp {
                        dest: Value(1),
                        op: IrCmpOp::Sgt,
                        lhs: v(Value(0)),
                        rhs: Operand::Const(IrConst::I32(3)),
                        ty: IrType::I32,
                    },
                    Instruction::Cmp {
                        dest: Value(2),
                        op: IrCmpOp::Slt,
                        lhs: v(Value(0)),
                        rhs: Operand::Const(IrConst::I32(10)),
                        ty: IrType::I32,
                    },
                    Instruction::Cast {
                        dest: Value(3),
                        src: v(Value(2)),
                        from_ty: IrType::U8,
                        to_ty: IrType::I64,
                    },
                    Instruction::Select {
                        dest: Value(4),
                        cond: v(Value(1)),
                        true_val,
                        false_val,
                        ty: IrType::I64,
                    },
                ],
                terminator: Terminator::Return(Some(v(Value(4)))),
                source_spans: vec![],
                source_inline_ids: Vec::new(),
            });
            func.next_value_id = 5;
            func
        };
        let arm = Operand::Value(Value(3));
        let k = |x: i64| Operand::Const(IrConst::I64(x));
        // `c ? p : 0`: the conjunction, a window.
        assert_eq!(run(&mut select(arm, k(0))), 1);
        // `c ? p : 1` is `!c || p`: every `x` below 10.
        let mut negated = select(arm, k(1));
        let before = format!("{negated:?}");
        assert_eq!(run(&mut negated), 0);
        assert_eq!(format!("{negated:?}"), before);
        // `c ? 0 : p` is `!c && p`: every `x` up to 3.
        assert_eq!(run(&mut select(k(0), arm)), 0);
        // `c ? 1 : p` is the disjunction, which holds everywhere here (the
        // half-lines overlap): no window.
        assert_eq!(run(&mut select(k(1), arm)), 0);
        // A constant that is no truth value.
        assert_eq!(run(&mut select(arm, k(2))), 0);
        assert_eq!(run(&mut select(k(2), arm)), 0);
    }

    /// `x != 44 || x != 45` holds for every `x`: it is no interval, in
    /// either spelling, and neither is a mixed pair.  `x == 44 || x == 45`
    /// is the two-element window.
    #[test]
    fn a_disjunction_of_inequalities_is_not_a_window() {
        use IrCmpOp::{Eq, Ne};
        let pred = |op, c: i64| Predicate {
            op,
            value: Operand::Value(Value(0)),
            constant: ring(c, IrType::I8),
            ty: IrType::I8,
        };
        assert!(interval(IrBinOp::Or, pred(Ne, 44), pred(Ne, 45)).is_none());
        assert!(interval(IrBinOp::Or, pred(Ne, 45), pred(Ne, 44)).is_none());
        assert!(interval(IrBinOp::Or, pred(Eq, 44), pred(Ne, 45)).is_none());
        assert!(interval(IrBinOp::Or, pred(Ne, 44), pred(Eq, 45)).is_none());
        let window = interval(IrBinOp::Or, pred(Eq, 45), pred(Eq, 44)).expect("a window");
        assert!(window.inside);
        assert_eq!((window.lower_bound, window.upper), (44, 45));
        // Two apart is no window.
        assert!(interval(IrBinOp::Or, pred(Eq, 44), pred(Eq, 46)).is_none());

        // The select `if_convert` makes of `c != 44 || c != 45`, as the
        // late if-conversion left it behind two inlined helpers:
        // `t = (c != 44) ? 1 : zext(c != 45)`.
        let mut func = IrFunction::new("either".into(), IrType::I64, vec![], false);
        let v = Operand::Value;
        let ne = |dest: u32, k: i32| Instruction::Cmp {
            dest: Value(dest),
            op: Ne,
            lhs: v(Value(0)),
            rhs: Operand::Const(IrConst::I8(k as i8)),
            ty: IrType::I8,
        };
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::I8,
                },
                ne(1, 44),
                ne(2, 45),
                Instruction::Cast {
                    dest: Value(3),
                    src: v(Value(2)),
                    from_ty: IrType::I8,
                    to_ty: IrType::I64,
                },
                Instruction::Select {
                    dest: Value(4),
                    cond: v(Value(1)),
                    true_val: Operand::Const(IrConst::I64(1)),
                    false_val: v(Value(3)),
                    ty: IrType::I64,
                },
            ],
            terminator: Terminator::Return(Some(v(Value(4)))),
            source_spans: vec![],
            source_inline_ids: Vec::new(),
        });
        func.next_value_id = 5;
        let before = format!("{func:?}");
        assert_eq!(run(&mut func), 0);
        assert_eq!(format!("{func:?}"), before);
        // The bitwise spelling of the same.
        func.blocks[0].instructions[4] = Instruction::BinOp {
            dest: Value(4),
            op: IrBinOp::Or,
            lhs: v(Value(1)),
            rhs: v(Value(2)),
            ty: IrType::I8,
        };
        assert_eq!(run(&mut func), 0);
    }

    #[test]
    fn complement_interval_matches_brute_force() {
        for ty in [IrType::I16, IrType::U16] {
            let bits = bits_of(ty);
            let mask = (1u64 << bits) - 1;
            let (lower, upper) = (4u64 & mask, 9u64 & mask);
            let interval = Interval {
                lower_bound: lower,
                upper,
                inside: false,
                signed: signed_of(ty),
                ty,
                value: Operand::Value(Value(0)),
            };
            for x in -8i64..24 {
                let want = !(x >= 4 && x <= 9);
                assert_eq!(folded(&interval, x), want, "{ty:?} x={x}");
            }
        }
    }

    fn and_fixture(ty: IrType) -> IrFunction {
        let mut func = IrFunction::new("interval_test".into(), IrType::I32, vec![], false);
        let v = Operand::Value;
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty,
                },
                Instruction::Cmp {
                    dest: Value(1),
                    op: IrCmpOp::Sgt,
                    lhs: v(Value(0)),
                    rhs: Operand::Const(IrConst::from_i64(3, ty)),
                    ty,
                },
                Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Slt,
                    lhs: v(Value(0)),
                    rhs: Operand::Const(IrConst::from_i64(10, ty)),
                    ty,
                },
                Instruction::BinOp {
                    dest: Value(3),
                    op: IrBinOp::And,
                    lhs: v(Value(1)),
                    rhs: v(Value(2)),
                    ty,
                },
            ],
            terminator: Terminator::Return(Some(v(Value(3)))),
            source_spans: vec![],
            source_inline_ids: Vec::new(),
        });
        func.next_value_id = 4;
        func
    }

    #[test]
    fn signed_conjunction_emits_one_unsigned_compare() {
        let mut func = and_fixture(IrType::I32);
        assert_eq!(run(&mut func), 1);
        let ninstr = func.blocks[0].instructions.len();
        // The conjunction is replaced by cast, sub, cmp.
        assert_eq!(ninstr, 4 - 1 + 3);
        assert!(matches!(
            func.blocks[0].instructions[ninstr - 1],
            Instruction::Cmp {
                op: IrCmpOp::Ult,
                rhs: Operand::Const(c),
                dest: Value(3),
                ..
            } if c.to_i64() == Some(6)
        ));
    }

    #[test]
    fn unsigned_conjunction_needs_no_reinterpretation() {
        let mut func = and_fixture(IrType::U32);
        assert_eq!(run(&mut func), 1);
        let ninstr = func.blocks[0].instructions.len();
        assert_eq!(ninstr, 4 - 1 + 2, "no cast for an already-unsigned value");
        assert!(matches!(
            func.blocks[0].instructions[ninstr - 2],
            Instruction::BinOp {
                op: IrBinOp::Sub,
                rhs: Operand::Const(c),
                ..
            } if c.to_i64() == Some(4)
        ));
    }

    #[test]
    fn window_that_crosses_zero_folds_like_any_other() {
        // `x > -8 && x < 8` is the window `[-7, 7]`: 15 values whose lower
        // bound, in the operand ring, sits ABOVE the upper one. The width must
        // be counted in that ring (15, not ~2^64), or the fold declines and
        // the shape keeps two compares plus a select -- which is exactly what
        // `x > 3 && x < 10` (a window that does not cross zero) never did.
        for (ty, lower, width_const) in [(IrType::I32, -8i64, 8i64), (IrType::I64, -8, 8)] {
            let mut func = and_fixture(ty);
            func.blocks[0].instructions[1] = Instruction::Cmp {
                dest: Value(1),
                op: IrCmpOp::Sgt,
                lhs: Operand::Value(Value(0)),
                rhs: Operand::Const(IrConst::from_i64(lower, ty)),
                ty,
            };
            func.blocks[0].instructions[2] = Instruction::Cmp {
                dest: Value(2),
                op: IrCmpOp::Slt,
                lhs: Operand::Value(Value(0)),
                rhs: Operand::Const(IrConst::from_i64(width_const, ty)),
                ty,
            };
            assert_eq!(run(&mut func), 1, "{ty:?}: crossing window not folded");
            let unsigned = unsigned_of(ty).unwrap();
            let bits = bits_of(ty);
            let mask = if bits == 64 {
                u64::MAX
            } else {
                (1u64 << bits) - 1
            };
            let placed = Interval {
                lower_bound: (-7i64 as u64) & mask,
                upper: 7u64 & mask,
                inside: true,
                signed: signed_of(ty),
                ty,
                value: Operand::Value(Value(0)),
            };
            assert_eq!(placed.width(), 15, "{ty:?}: ring-crossing width");
            // The emitted compare is the biased one: `(unsigned)(x + 7) < 15`.
            let last = func.blocks[0].instructions.last().unwrap();
            assert!(
                matches!(
                    last,
                    Instruction::Cmp {
                        op: IrCmpOp::Ult,
                        rhs: Operand::Const(c),
                        ty: t,
                        ..
                    } if c.to_i64() == Some(15) && *t == unsigned
                ),
                "{ty:?}: expected `(unsigned)(x - L) < 15`, got {last:?}"
            );
        }
    }

    #[test]
    fn unsigned_inverted_window_stays_empty() {
        // `x >= 0xFFFFFFF7u && x <= 10u` is EMPTY in unsigned order, and its
        // stored bounds are the same pair that a signed crossing window has
        // (`[-9, 10]` is `[0xFFFFFFF7, 10]`). Ordering the mask around the
        // ring instead of in the compare's domain fabricates a 20-value window
        // that accepts both `0..10` and `0xFFFFFFF7..0xFFFFFFFF` -- the
        // phantom window `range_fold_branch.c` A11 reproduces live, at u32 and
        // u64 width.
        for (ty, constant) in [
            (IrType::U32, IrConst::from_i64(0xFFFF_FFF7, IrType::U32)),
            (IrType::U64, IrConst::from_i64(-9, IrType::U64)),
        ] {
            let mut wrap = and_fixture(ty);
            wrap.blocks[0].instructions[1] = Instruction::Cmp {
                dest: Value(1),
                op: IrCmpOp::Uge,
                lhs: Operand::Value(Value(0)),
                rhs: Operand::Const(constant),
                ty,
            };
            wrap.blocks[0].instructions[2] = Instruction::Cmp {
                dest: Value(2),
                op: IrCmpOp::Ule,
                lhs: Operand::Value(Value(0)),
                rhs: Operand::Const(IrConst::from_i64(10, ty)),
                ty,
            };
            assert_eq!(
                run(&mut wrap),
                0,
                "{ty:?}: a wrapping unsigned window is empty, not a range test"
            );
        }
    }

    #[test]
    fn empty_and_oversized_windows_are_refused() {
        // x > 3 & x < 3 is empty.
        let mut empty = and_fixture(IrType::I32);
        empty.blocks[0].instructions[2] = Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Slt,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Const(IrConst::I32(3)),
            ty: IrType::I32,
        };
        assert_eq!(run(&mut empty), 0);

        // An inverted window whose LOWER bound is negative first: the ring
        // makes `[-5, -9]` look like a 2^64-wide window, and the ring-correct
        // width must still reject it as empty.
        let mut inverted = and_fixture(IrType::I32);
        inverted.blocks[0].instructions[1] = Instruction::Cmp {
            dest: Value(1),
            op: IrCmpOp::Sge,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Const(IrConst::I32(-5)),
            ty: IrType::I32,
        };
        inverted.blocks[0].instructions[2] = Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Sle,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Const(IrConst::I32(-9)),
            ty: IrType::I32,
        };
        assert_eq!(run(&mut inverted), 0, "an inverted window must be refused");

        // A window spanning the whole ring is not a range test.
        let mut wide = and_fixture(IrType::I32);
        wide.blocks[0].instructions[1] = Instruction::Cmp {
            dest: Value(1),
            op: IrCmpOp::Sge,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Const(IrConst::I32(i32::MIN)),
            ty: IrType::I32,
        };
        wide.blocks[0].instructions[2] = Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Sle,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Const(IrConst::I32(i32::MAX)),
            ty: IrType::I32,
        };
        assert_eq!(run(&mut wide), 0);
    }

    #[test]
    fn different_values_are_not_an_interval() {
        let mut func = and_fixture(IrType::I32);
        func.blocks[0].instructions[2] = Instruction::Cmp {
            dest: Value(2),
            op: IrCmpOp::Slt,
            lhs: Operand::Value(Value(9)),
            rhs: Operand::Const(IrConst::I32(10)),
            ty: IrType::I32,
        };
        assert_eq!(run(&mut func), 0);
    }

    #[test]
    fn select_form_keeps_the_boolean_extension() {
        let mut func = IrFunction::new("sel".into(), IrType::I64, vec![], false);
        let v = Operand::Value;
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::I32,
                },
                Instruction::Cmp {
                    dest: Value(1),
                    op: IrCmpOp::Sgt,
                    lhs: v(Value(0)),
                    rhs: Operand::Const(IrConst::I32(3)),
                    ty: IrType::I32,
                },
                Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Slt,
                    lhs: v(Value(0)),
                    rhs: Operand::Const(IrConst::I32(10)),
                    ty: IrType::I32,
                },
                Instruction::Cast {
                    dest: Value(3),
                    src: v(Value(2)),
                    from_ty: IrType::U8,
                    to_ty: IrType::I64,
                },
                Instruction::Select {
                    dest: Value(4),
                    cond: v(Value(1)),
                    true_val: v(Value(3)),
                    false_val: Operand::Const(IrConst::I64(0)),
                    ty: IrType::I64,
                },
            ],
            terminator: Terminator::Return(Some(v(Value(4)))),
            source_spans: vec![],
            source_inline_ids: Vec::new(),
        });
        func.next_value_id = 5;
        assert_eq!(run(&mut func), 1);
        let last = func.blocks[0].instructions.last().unwrap();
        assert!(
            matches!(
                last,
                Instruction::Cast {
                    dest: Value(4),
                    from_ty: IrType::U8,
                    to_ty: IrType::I64,
                    ..
                }
            ),
            "{last:?}"
        );
    }
}
