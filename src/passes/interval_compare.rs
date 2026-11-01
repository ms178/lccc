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
        (Operand::Const(c), value) => (mirror(*op), *value, ring(c.to_i64()?, *ty)),
        _ => return None,
    };
    Some(Predicate {
        op,
        value: peel(value, defs),
        constant,
        ty: *ty,
    })
}

fn mirror(op: IrCmpOp) -> IrCmpOp {
    match op {
        IrCmpOp::Eq => IrCmpOp::Eq,
        IrCmpOp::Ne => IrCmpOp::Ne,
        IrCmpOp::Slt => IrCmpOp::Sgt,
        IrCmpOp::Sle => IrCmpOp::Sge,
        IrCmpOp::Sgt => IrCmpOp::Slt,
        IrCmpOp::Sge => IrCmpOp::Sle,
        IrCmpOp::Ult => IrCmpOp::Ugt,
        IrCmpOp::Ule => IrCmpOp::Uge,
        IrCmpOp::Ugt => IrCmpOp::Ult,
        IrCmpOp::Uge => IrCmpOp::Ule,
    }
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
        if signed_of(self.ty) {
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
fn interval(logic: IrBinOp, lhs: Predicate, rhs: Predicate) -> Option<Interval> {
    if lhs.ty != rhs.ty || !same_value(&lhs.value, &rhs.value) {
        return None;
    }
    let ty = lhs.ty;
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
            // empty or too-wide window (0 or > 2^31-1 values), so both a
            // signed crossing window (`x > -8 && x < 8`) and an inverted one
            // (`x >= 10 && x <= 5`, `x >= 0xFFFFFFF7u && x <= 10u`) are
            // handled correctly.
            (lo_pred.value, (lower, upper, true))
        }
        IrBinOp::Or => {
            // Either the complement of a window, or `{A, A+1}` membership.
            if let (Some((ca, eq_a)), Some((cb, eq_b))) = (membership(&lhs), membership(&rhs)) {
                // Mixed `==`/`!=` pairs are a different function.
                if eq_a != eq_b {
                    return None;
                }
                let (low, high) = (ca.min(cb), ca.max(cb));
                if high != low.wrapping_add(1) {
                    return None;
                }
                // `==` is true inside the two-element window, `!=` outside it.
                (lhs.value, (low, high, eq_a))
            } else {
                let (upper_pred, lower_pred) = match (upper_bounds(&lhs), lower_bounds(&rhs)) {
                    (Some(_), Some(_)) => (lhs, rhs),
                    _ => match (upper_bounds(&rhs), lower_bounds(&lhs)) {
                        (Some(_), Some(_)) => (rhs, lhs),
                        _ => return None,
                    },
                };
                let upper = upper_bounds(&upper_pred)?;
                let lower = lower_bounds(&lower_pred)?;
                // The true region is everything outside `[lower, upper]`; the
                // equivalent "huge unsigned" test starts at `upper + 1` and
                // ends one below `lower`.
                if lower <= upper.wrapping_add(1) {
                    return None;
                }
                (
                    upper_pred.value,
                    (upper.wrapping_add(1), lower.wrapping_sub(1), false),
                )
            }
        }
        _ => return None,
    };
    let (lower_bound, upper, inside) = window;
    Some(Interval {
        lower_bound,
        upper,
        inside,
        ty,
        value,
    })
}

/// `x == A` or `x != A`, for the membership forms: the constant and whether
/// the predicate is the equality.
fn membership(pred: &Predicate) -> Option<(u64, bool)> {
    match pred.op {
        IrCmpOp::Eq => Some((pred.constant, true)),
        IrCmpOp::Ne => Some((pred.constant, false)),
        _ => None,
    }
}

fn lower_bounds(pred: &Predicate) -> Option<u64> {
    match pred.op {
        IrCmpOp::Sge | IrCmpOp::Uge => Some(pred.constant),
        IrCmpOp::Sgt | IrCmpOp::Ugt => Some(pred.constant.wrapping_add(1)),
        _ => None,
    }
}

fn upper_bounds(pred: &Predicate) -> Option<u64> {
    match pred.op {
        IrCmpOp::Sle | IrCmpOp::Ule => Some(pred.constant),
        IrCmpOp::Slt | IrCmpOp::Ult => Some(pred.constant.wrapping_sub(1)),
        _ => None,
    }
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
    // A window of 2^31 or more lets an in-range value wrap past the bound; an
    // empty window is not a range test at all.
    if width == 0 || width > (1u64 << 31) - 1 {
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
            interval.upper.wrapping_sub(interval.lower_bound),
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
    // constant 1 or 0 that makes the select a boolean operator.
    let (arm, neutral) = if is_boolean_constant(false_val) {
        (true_val, 0i64)
    } else if is_boolean_constant(true_val) {
        (false_val, 1i64)
    } else {
        return None;
    };
    let inner = zero_extended_predicate(*arm, defs)?;
    let outer = predicate(*cond, defs)?;
    let logic = if neutral == 0 {
        IrBinOp::And
    } else {
        IrBinOp::Or
    };
    Some((logic, outer, inner, *ty))
}

fn is_boolean_constant(operand: &Operand) -> bool {
    matches!(operand, Operand::Const(c) if matches!(c.to_i64(), Some(0) | Some(1)))
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
        if !block.source_spans.is_empty() {
            let span = block
                .source_spans
                .get(ii)
                .copied()
                .unwrap_or_else(crate::common::source::Span::dummy);
            block.source_spans.resize(block.instructions.len(), span);
            block
                .source_spans
                .splice(ii..=ii, std::iter::repeat_n(span, replacement.len()));
        }
        block.instructions.splice(ii..=ii, replacement);
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
            d > interval.upper.wrapping_sub(interval.lower_bound)
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
