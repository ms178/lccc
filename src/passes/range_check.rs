//! Range-check folding: `(x >= lo && x <= hi)` → `(unsigned)(x - lo) <= hi - lo`.
//!
//! C's `a && b` / `a || b` lower to short-circuit control flow, which mem2reg
//! and if-conversion collapse into `Select` chains. When both arms of the
//! short-circuit compare the SAME value against constant bounds, the whole
//! boolean dance (two `cmp`+`setcc`+`movzbl`, a `test`, a `cmov`, and a
//! narrowing cast) collapses into one subtract and one unsigned compare —
//! exactly what GCC emits for Expat's `xml_name_continue` / SQLite's varint
//! classifier hot paths.
//!
//! The transform is the classic unsigned-bias identity:
//!   x >= lo && x <= hi  ⇔  (unsigned)(x - lo) <= (hi - lo)      (lo <= hi)
//!   x <  lo || x >  hi  ⇔  (unsigned)(x - lo) >  (hi - lo)      (lo <= hi)
//! Wraparound subtraction makes out-of-range values wrap to huge unsigned
//! values, so a single unsigned compare classifies the whole range. Valid for
//! both signed and unsigned comparisons, in any integer width up to 64 bits,
//! provided `hi - lo` is representable.
//!
//! # Domain algebra (why every bound is normalized through `i128`)
//!
//! The identity's `lo <= hi` precondition and its span are statements about
//! the comparison's MATHEMATICAL domain, not about the `i64` bit patterns the
//! constants arrive in. Comparing raw `i64`s is wrong in both directions:
//!
//! * **Unsigned wrapped ranges fold that are empty.** `x >= u64max-9 &&
//!   x <= 10` arrives as `lo = -9, hi = 10` (top-bit-set bit patterns are
//!   negative as `i64`), passes an `i64` `lo <= hi`, and miscompiles into a
//!   21-value wrapped window that accepts both `0..10` and `u64max-9..u64max`
//!   (reproduced live: 15 wrong values on one sweep). The same class hits U8
//!   (`x >= 246 && x <= 10`) and would hit U32 whenever both constants are
//!   spelled as sign-extended patterns.
//! * **Narrowing aliases values outside the source type.** Comparing
//!   `(u8)(x - 250) <= 10` reads the low byte only, so `x = 0..4` wrap to
//!   `6..10` and pass a test the source predicate fails (`x >= 250 && x <=
//!   260` is true for 250..255 only; reproduced live). Narrowing is therefore
//!   legal only when `lo` and `hi` BOTH fit the pre-promotion source domain.
//!
//! Every fold path normalizes its bounds through [`dom_value`] into `i128`,
//! orders them in the domain's own signedness, computes the span without any
//! possibility of `i64` overflow (`i64` full-range spans used to wrap or
//! panic), and only then decides representability. `I128`/`U128` comparisons
//! are rejected explicitly — no path silently guesses at their domains.
//!
//! # CFG discipline
//!
//! Predecessor structure comes from the canonical `ir::analysis::build_cfg`
//! — never from a local successor scan. A bespoke scan that stops at
//! `Branch`/`CondBranch`/`Switch` is blind to `IndirectBranch` targets and to
//! `InlineAsm` `goto_labels` edges; a check block reachable through either is
//! not single-predecessor, and deleting it would leave a dangling target.
//!
//! The two CFG-mutating folds (phi diamonds and branch chains) each apply AT
//! MOST ONE rewrite per call and [`run_function`] restarts them with freshly
//! built definition maps until neither fires. Analyses are therefore never
//! consumed after the IR they describe has changed: the previous single-pass
//! structure kept `cmp_defs`/`cast_defs` across operand rewrites (a stale
//! operand record could emit a `Sub` over a deleted phi) and indexed
//! path-facts by pre-removal block positions (facts could land on unrelated
//! blocks after a fold deleted one).
//!
//! Pass name for CCC_DISABLE_PASSES: "range_fold".

use crate::common::types::IrType;
use crate::ir::reexports::{
    Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Terminator, Value,
};

/// A comparison, canonicalized to `value OP const` with an explicit role.
#[derive(Clone, Copy)]
struct Bound {
    /// The value being compared (always the same value on both arms).
    value: Value,
    /// The constant bound, as the raw `i64` bit pattern of the IR constant.
    bound: i64,
    /// The operand type of the comparison (e.g. I32 after promotion).
    ty: IrType,
    /// Inclusive/exclusive, lower/upper role.
    kind: BoundKind,
    /// Signedness of the original comparison (S* vs U*).
    signed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BoundKind {
    /// x >= c  (or c <= x)
    InclLower,
    /// x <= c  (or c >= x)
    InclUpper,
    /// x > c  (or c < x)
    ExclLower,
    /// x < c  (or c > x)
    ExclUpper,
}

/// Canonicalize `op lhs rhs` into a `Bound` when one side is a value and the
/// other an integer constant. Returns None for non-constant or non-integer
/// comparisons.
fn canonicalize(op: IrCmpOp, lhs: &Operand, rhs: &Operand, ty: IrType) -> Option<Bound> {
    if !ty.is_integer() {
        return None;
    }
    let signed = matches!(
        op,
        IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge
    );
    // (value, const) with the const on the RHS, swapping the operator when the
    // constant was on the LHS.
    let (value, bound, op) = match (lhs, rhs) {
        (Operand::Value(v), Operand::Const(c)) => (*v, c.to_i64()?, op),
        (Operand::Const(c), Operand::Value(v)) => (*v, c.to_i64()?, swap(op)),
        _ => return None,
    };
    let kind = match op {
        IrCmpOp::Sge | IrCmpOp::Uge => BoundKind::InclLower,
        IrCmpOp::Sle | IrCmpOp::Ule => BoundKind::InclUpper,
        IrCmpOp::Sgt | IrCmpOp::Ugt => BoundKind::ExclLower,
        IrCmpOp::Slt | IrCmpOp::Ult => BoundKind::ExclUpper,
        IrCmpOp::Eq | IrCmpOp::Ne => return None,
    };
    Some(Bound {
        value,
        bound,
        ty,
        kind,
        signed,
    })
}

fn swap(op: IrCmpOp) -> IrCmpOp {
    match op {
        IrCmpOp::Slt => IrCmpOp::Sgt,
        IrCmpOp::Sle => IrCmpOp::Sge,
        IrCmpOp::Sgt => IrCmpOp::Slt,
        IrCmpOp::Sge => IrCmpOp::Sle,
        IrCmpOp::Ult => IrCmpOp::Ugt,
        IrCmpOp::Ule => IrCmpOp::Uge,
        IrCmpOp::Ugt => IrCmpOp::Ult,
        IrCmpOp::Uge => IrCmpOp::Ule,
        other => other,
    }
}

/// The comparison domain of an integer type: `(bit width, signedness)`.
/// 128-bit domains are rejected explicitly — the fold's `i128` arithmetic
/// cannot order them, and silently truncating them once was how wrapped
/// ranges miscompiled.
fn domain_of(ty: IrType) -> Option<(u32, bool)> {
    Some(match ty {
        IrType::I8 => (8, true),
        IrType::U8 => (8, false),
        IrType::I16 => (16, true),
        IrType::U16 => (16, false),
        IrType::I32 => (32, true),
        IrType::U32 => (32, false),
        IrType::I64 => (64, true),
        IrType::U64 => (64, false),
        _ => return None, // I128/U128/FP/aggregate: out of scope, fail closed
    })
}

/// Interpret a raw `i64` bit pattern as a mathematical value of the domain
/// `(bits, signed)`: sign patterns are already sign-extended by `to_i64`;
/// unsigned patterns are masked to their width, which makes the result
/// independent of whether the frontend spelled `60000u` as `I16(-5536)` or
/// `I64(60000)`.
fn dom_value(bits: i64, bits_w: u32, signed: bool) -> i128 {
    if signed {
        bits as i128
    } else {
        let mask: u128 = if bits_w >= 128 {
            u128::MAX
        } else {
            (1u128 << bits_w) - 1
        };
        ((bits as u64) as u128 & mask) as i128
    }
}

/// Does the mathematical value `v` lie inside `ty`'s own domain? Narrowing a
/// compare to a byte/word source type requires this of BOTH bounds: the
/// unsigned-bias identity in the narrow domain classifies exactly
/// `[lo, hi] ∩ domain`, so a bound outside the domain (e.g. `260` for `u8`)
/// would let wrapped values alias into the window.
fn fits_domain(v: i128, ty: IrType) -> bool {
    let Some((w, signed)) = domain_of(ty) else {
        return false;
    };
    if signed {
        let lo = -(1i128 << (w - 1));
        let hi = (1i128 << (w - 1)) - 1;
        lo <= v && v <= hi
    } else {
        (0..=(1i128 << w) - 1).contains(&v)
    }
}

/// Rebuild an integer constant of type `ty` from a mathematical value.
/// Returns None if the value does not fit the type's signed range.
fn int_const(ty: IrType, v: i64) -> Option<IrConst> {
    match ty {
        IrType::I8 => i8::try_from(v).ok().map(IrConst::I8),
        IrType::U8 => u8::try_from(v).ok().map(|x| IrConst::I8(x as i8)),
        IrType::I16 => i16::try_from(v).ok().map(IrConst::I16),
        IrType::U16 => u16::try_from(v).ok().map(|x| IrConst::I16(x as i16)),
        IrType::I32 => i32::try_from(v).ok().map(IrConst::I32),
        IrType::U32 => u32::try_from(v).ok().map(|x| IrConst::I32(x as i32)),
        IrType::I64 => Some(IrConst::I64(v)),
        IrType::U64 => Some(IrConst::I64(v)),
        _ => None,
    }
}

/// Rebuild an integer constant of type `ty` from a raw two's-complement bit
/// pattern. Unlike [`int_const`] this never rejects: the caller has already
/// proven the value is representable in the domain, and the bias constant of
/// the `Sub` is the bit pattern itself (unsigned upper-half constants arrive
/// as "negative" `i64`s and must keep those exact bits).
fn int_const_bits(ty: IrType, bits: i64) -> Option<IrConst> {
    Some(match ty {
        IrType::I8 | IrType::U8 => IrConst::I8(bits as i8),
        IrType::I16 | IrType::U16 => IrConst::I16(bits as i16),
        IrType::I32 | IrType::U32 => IrConst::I32(bits as i32),
        IrType::I64 | IrType::U64 => IrConst::I64(bits),
        _ => return None,
    })
}

/// The `lo`/`hi` bounds of a shared-value range, as raw bit patterns, with
/// the comparison's type. `and_form` selects `&&` (inside) vs `||` (outside).
struct Range {
    lo: i64,
    hi: i64,
    value: Value,
    ty: IrType,
}

/// A validated fold plan shared by every emission path (Select, Boolean op,
/// phi diamond, branch chain). Building it performs the whole domain
/// argument once, so the four paths cannot diverge on the algebra:
///
/// * `lo <= hi` in the comparison domain's own order — this rejects empty
///   and WRAPPED ranges (`x >= u64max-9 && x <= 10` is empty, not a
///   21-value window);
/// * the span `hi - lo` computed in `i128` (an `i64` subtraction of
///   full-range bounds overflows — a panic in checked builds, a wrapped
///   "always-true" span in release);
/// * the span must be mathematically representable in `ty` (the status-quo
///   threshold: `int_const` rejects `I32`'s full-range span 2^32-1, keeping
///   the too-wide-range shapes unfolded) — EXCEPT for the one span that
///   needs no compare at all: a range covering the compare type's ENTIRE
///   domain makes the membership test a constant;
/// * the compare may narrow to the operand's pre-promotion source type only
///   when BOTH bounds fit that source's domain (and a range covering that
///   narrowed domain entirely is likewise a constant).
enum RangePlan {
    /// The unsigned-bias test: `Sub(value, lo)` at `ty`, compared
    /// `Ule|Ugt` against the span at `cmp_ty`.
    Test {
        value: Value,
        /// The `Sub`'s (and the compare's fallback) width — the compare type the
        /// two source comparisons ran at.
        ty: IrType,
        /// The compare's width — `ty`, or the narrower source domain when the
        /// bounds provably fit it.
        cmp_ty: IrType,
        /// The bias constant for the `Sub`, bit-exact for `ty`.
        lo_const: IrConst,
        /// The span constant for the compare, bit-exact for `cmp_ty`.
        span_const: IrConst,
    },
    /// The range covers the compare domain's every value: the INSIDE form
    /// (`x >= lo && x <= hi`) is constant TRUE and the OUTSIDE form
    /// (`x < lo || x > hi`) is constant FALSE. No `Sub`, no compare — the
    /// emission paths materialize the constant directly. (GCC folds this
    /// family too; the historical span-representability rejection kept the
    /// SIGNED full domain `[INT_MIN, INT_MAX]` unfolded, and the unsigned
    /// full domain paid a dead `Sub`+`Cmp` for a compare that always
    /// answered the same.)
    FullDomain,
}

fn plan_range(range: &Range, cast_defs: &[Option<(Operand, IrType, IrType)>]) -> Option<RangePlan> {
    let (w, signed) = domain_of(range.ty)?;
    let lo = dom_value(range.lo, w, signed);
    let hi = dom_value(range.hi, w, signed);
    if lo > hi {
        // Empty or wrapped: `x >= lo && x <= hi` is false for every x (and
        // the `||` complement is a tautology). Not this pass's fold.
        return None;
    }
    // i128 span: cannot overflow for any <=64-bit domain.
    let span: i128 = hi - lo;
    // Full domain at the COMPARE type: every value the operand can hold at
    // this width lies inside the range, so the inside test is constant TRUE
    // (and the outside form constant FALSE). Detected BEFORE the span-cap
    // rejection — the signed full domain `[INT_MIN, INT_MAX]` spans 2^w-1,
    // which exceeds the signed cap but needs no span constant at all.
    // (No narrowing can apply here: the bounds of a full `ty` domain never
    // fit a narrower source domain, so `cmp_ty` would be `ty` anyway.)
    let ty_dom_min: i128 = if signed { -(1i128 << (w - 1)) } else { 0 };
    let ty_dom_max: i128 = if signed {
        (1i128 << (w - 1)) - 1
    } else {
        (1i128 << w) - 1
    };
    if lo == ty_dom_min && hi == ty_dom_max {
        return Some(RangePlan::FullDomain);
    }
    // Representability threshold, identical to the historical `int_const`
    // gate: signed types must fit their positive half, unsigned types their
    // full width.
    let span_cap: i128 = if signed {
        (1i128 << (w - 1)) - 1
    } else {
        (1i128 << w) - 1
    };
    if span > span_cap {
        return None;
    }
    // Bias constant: the ORIGINAL bit pattern, re-dressed at `ty`. Unsigned
    // upper-half constants keep their "negative" `i64` spelling here — the
    // two's-complement bits are what the wrapping `Sub` consumes.
    let lo_const = int_const_bits(range.ty, range.lo)?;
    // Narrow the compare to the operand's pre-promotion source width when
    // the operand is a widening cast of a byte/short AND both bounds fit
    // that source's own domain: `(u8)(x - lo) <= span` reads the low byte
    // of the wide sub and classifies `[lo, hi]` exactly — but ONLY when no
    // bound lies outside the source domain, where wrapped low-byte values
    // would alias into the window (`[250, 260]` on `u8` also accepts 0..4).
    let mut cmp_ty = range.ty;
    if let Some(Some((_, from_ty, to_ty))) = cast_defs.get(range.value.0 as usize) {
        if *to_ty == range.ty
            && from_ty.is_integer()
            && from_ty.size() < range.ty.size()
            && fits_domain(lo, *from_ty)
            && fits_domain(hi, *from_ty)
        {
            cmp_ty = *from_ty;
        }
    }
    // Full domain at the NARROWED compare type: both bounds fit the source
    // domain and together they cover it — e.g. `(unsigned char)x >= 0 &&
    // (unsigned char)x <= 255`, where the u8 compare is the tautology. The
    // operand's domain at this compare is exactly `cmp_ty` (it is the
    // promoted source value), so coverage of `cmp_ty` is coverage of every
    // reachable value.
    if let Some((cw, csigned)) = domain_of(cmp_ty) {
        let cmin: i128 = if csigned { -(1i128 << (cw - 1)) } else { 0 };
        let cmax: i128 = if csigned {
            (1i128 << (cw - 1)) - 1
        } else {
            (1i128 << cw) - 1
        };
        if lo == cmin && hi == cmax {
            return Some(RangePlan::FullDomain);
        }
    }
    // The span fits `cmp_ty` by construction: both bounds fit `cmp_ty`'s
    // domain (either it IS `ty` and the cap above held, or it is the source
    // domain both bounds fit), so their difference is < 2^width. Only the
    // unsigned 64-bit full-domain span (2^64-1) exceeds `i64::MAX`; its
    // two's-complement bit pattern (all ones) is exactly what the unsigned
    // compare needs, so the wrapping re-dress is the correct constant.
    let span_bits = if span > i64::MAX as i128 {
        u64::try_from(span).ok()? as i64
    } else {
        span as i64
    };
    let span_const = int_const_bits(cmp_ty, span_bits)?;
    Some(RangePlan::Test {
        value: range.value,
        ty: range.ty,
        cmp_ty,
        lo_const,
        span_const,
    })
}

/// Build the unsigned-bias test core: `Sub(value, lo)` at `ty` and
/// `Cmp(Ule|Ugt, sub, span)` at `cmp_ty`. Returns the instructions plus the
/// compare's result value; callers adapt the boolean to their own result
/// type (widening cast) or terminator.
fn range_test_core(
    plan: &RangePlan,
    and_form: bool,
    next_id: &mut u32,
) -> (Vec<Instruction>, Value) {
    let RangePlan::Test {
        value,
        ty,
        cmp_ty,
        lo_const,
        span_const,
    } = plan
    else {
        // FullDomain plans carry no test: callers materialize the constant
        // before ever reaching the core. Arriving here means a caller lost
        // the distinction — an empty core would silently delete the test
        // and miscompile, so fail loudly instead.
        panic!("range_test_core: FullDomain plan has no test to build");
    };
    let sub_dest = Value(*next_id);
    *next_id += 1;
    let cmp_dest = Value(*next_id);
    *next_id += 1;
    let cmp_op = if and_form { IrCmpOp::Ule } else { IrCmpOp::Ugt };
    (
        vec![
            Instruction::BinOp {
                dest: sub_dest,
                op: IrBinOp::Sub,
                lhs: Operand::Value(*value),
                rhs: Operand::Const(lo_const.clone()),
                ty: *ty,
            },
            Instruction::Cmp {
                dest: cmp_dest,
                op: cmp_op,
                lhs: Operand::Value(sub_dest),
                rhs: Operand::Const(span_const.clone()),
                ty: *cmp_ty,
            },
        ],
        cmp_dest,
    )
}

/// Follow a chain of Cast definitions back to the root value, recording the
/// (from_ty, to_ty) steps. Two operands are the same comparison operand iff
/// their root AND their full cast chain match (an i8→i32 zero-extension is
/// not the same operand as an i16→i32 sign-extension).
fn follow_casts(
    mut v: Value,
    cast_defs: &[Option<(Operand, IrType, IrType)>],
) -> (Value, Vec<(IrType, IrType)>) {
    let mut chain = Vec::with_capacity(16);
    loop {
        let idx = v.0 as usize;
        match cast_defs.get(idx).and_then(|x| x.as_ref()) {
            Some((Operand::Value(src), from_ty, to_ty)) => {
                chain.push((*from_ty, *to_ty));
                v = *src;
            }
            _ => break,
        }
    }
    (v, chain)
}

/// Resolve a boolean-producing value to its defining `Cmp`, looking through
/// the boolean-widening cast the frontend emits after every comparison
/// (`(i32)(u8)cmp`). Phi arms and Select arms carry that cast; without this
/// resolution the fold never sees the comparison at all (which is why the
/// Select form never fired on standard lowering). Returns the value id of
/// the underlying Cmp.
fn resolve_bool_cmp(
    v: u32,
    cmp_defs: &[Option<(IrCmpOp, Operand, Operand, IrType)>],
    cast_defs: &[Option<(Operand, IrType, IrType)>],
) -> Option<u32> {
    if cmp_defs.get(v as usize).is_some_and(|d| d.is_some()) {
        return Some(v);
    }
    let (src, from_ty, to_ty) = cast_defs.get(v as usize)?.as_ref()?;
    let Operand::Value(sv) = src else {
        return None;
    };
    // Only the boolean widening (U8/I8 → wider integer); anything else is a
    // semantic cast the fold must not see through.
    //
    // EVERY wider integer target must be listed. The set used to stop at
    // `I16 | I32 | U32`, which silently excluded the 64-bit widening the
    // frontend actually emits for `char c; return c >= 'a' && c <= 'z';`:
    //
    //     Cmp  v10 = Sle v1, I8(122)
    //     Cast v11 = v10 (U8 -> I64)        <-- not matched, fold gave up
    //     Select v14 = v6 ? v11 : Const(0)  (ty I64)
    //
    // so the pass never fired on the very idiom it was written for, and
    // `set_membership` (which consumes range_fold's output) was starved with
    // it -- lccc emitted an 11-deep compare/branch chain for Expat's
    // `xml_name_continue` classifier where GCC and ICX emit a handful of
    // branchless instructions.
    //
    // Widening is value-preserving here regardless of signedness: the source
    // is a `Cmp` result, verified below to be 0 or 1, and both zero- and
    // sign-extension of 0/1 yield 0/1 in every width. Narrowing is NOT in the
    // list -- that could discard bits of a non-boolean.
    if !matches!(
        (from_ty, to_ty),
        (
            IrType::I8 | IrType::U8,
            IrType::I16
                | IrType::U16
                | IrType::I32
                | IrType::U32
                | IrType::I64
                | IrType::U64
                | IrType::I128
                | IrType::U128
        )
    ) {
        return None;
    }
    let s = sv.0;
    if cmp_defs.get(s as usize).is_some_and(|d| d.is_some()) {
        Some(s)
    } else {
        None
    }
}

/// Match the two arms of a short-circuit select against the same value and
/// extract the constant range (STRUCTURE only — all arithmetic lives in
/// [`plan_range`]). `and_form`:
///   true  → `x >= lo && x <= hi` (inclusive both ends)
///   false → `x <  lo || x >  hi` (exclusive both ends, outside the range)
fn extract_range(
    a: &Bound,
    b: &Bound,
    and_form: bool,
    cast_defs: &[Option<(Operand, IrType, IrType)>],
) -> Option<Range> {
    if a.ty != b.ty || a.signed != b.signed {
        return None;
    }
    // The two comparison operands must be the same value. The lowering casts
    // the operand once per comparison (e.g. two separate `u8 -> i32` casts of
    // the same byte), so compare the cast-chain canonical form, not the raw
    // value id.
    if follow_casts(a.value, cast_defs) != follow_casts(b.value, cast_defs) {
        return None;
    }
    let (lo, hi) = if and_form {
        match (a.kind, b.kind) {
            (BoundKind::InclLower, BoundKind::InclUpper) => (a.bound, b.bound),
            (BoundKind::InclUpper, BoundKind::InclLower) => (b.bound, a.bound),
            _ => return None,
        }
    } else {
        match (a.kind, b.kind) {
            (BoundKind::ExclUpper, BoundKind::ExclLower) => (a.bound, b.bound),
            (BoundKind::ExclLower, BoundKind::ExclUpper) => (b.bound, a.bound),
            _ => return None,
        }
    };
    // NOTE: deliberately NO `lo > hi` rejection here — that judgement
    // belongs to the domain-aware `plan_range` (an `i64` comparison of raw
    // bit patterns is wrong for every unsigned domain).
    Some(Range {
        lo,
        hi,
        value: a.value,
        ty: a.ty,
    })
}

/// Fold one `Select` into a range check when it matches. Returns the
/// replacement instruction sequence (1-3 instructions) and consumes a fresh
/// value id when needed.
fn try_fold_select(
    inst: &Instruction,
    cmp_defs: &[Option<(IrCmpOp, Operand, Operand, IrType)>],
    cast_defs: &[Option<(Operand, IrType, IrType)>],
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    let Instruction::Select {
        dest,
        cond,
        true_val,
        false_val,
        ty,
    } = inst
    else {
        return None;
    };
    if !ty.is_integer() {
        return None;
    }

    // Resolve the two arms. Both forms have exactly one constant arm and two
    // comparison arms. Arms may carry the boolean-widening cast — resolve
    // through it to the defining comparison.
    let (cond_bound, other_bound, and_form): (Bound, Bound, bool) = {
        let cond_id = match cond {
            Operand::Value(v) => v.0,
            _ => return None,
        };
        let cond_cmp_id = resolve_bool_cmp(cond_id, cmp_defs, cast_defs)?;
        let cond_cmp = cmp_defs.get(cond_cmp_id as usize)?.as_ref()?;
        let cond_bound = canonicalize(cond_cmp.0, &cond_cmp.1, &cond_cmp.2, cond_cmp.3)?;

        // `&&`: Select(cond, other, 0).  `||`: Select(cond, 1, other).
        if matches!(false_val, Operand::Const(c) if c.to_i64() == Some(0)) {
            let other_id = match true_val {
                Operand::Value(v) => v.0,
                _ => return None,
            };
            let other_cmp_id = resolve_bool_cmp(other_id, cmp_defs, cast_defs)?;
            let other_cmp = cmp_defs.get(other_cmp_id as usize)?.as_ref()?;
            let other_bound = canonicalize(other_cmp.0, &other_cmp.1, &other_cmp.2, other_cmp.3)?;
            (cond_bound, other_bound, true)
        } else if matches!(true_val, Operand::Const(c) if c.to_i64() == Some(1)) {
            let other_id = match false_val {
                Operand::Value(v) => v.0,
                _ => return None,
            };
            let other_cmp_id = resolve_bool_cmp(other_id, cmp_defs, cast_defs)?;
            let other_cmp = cmp_defs.get(other_cmp_id as usize)?.as_ref()?;
            let other_bound = canonicalize(other_cmp.0, &other_cmp.1, &other_cmp.2, other_cmp.3)?;
            (cond_bound, other_bound, false)
        } else {
            return None;
        }
    };

    let range = extract_range(&cond_bound, &other_bound, and_form, cast_defs)?;
    let plan = plan_range(&range, cast_defs)?;
    if matches!(plan, RangePlan::FullDomain) {
        // The range covers the compare domain entirely: the inside form is
        // constant TRUE and the outside form constant FALSE. The Select's
        // arms (both comparison booleans, or the constant 0/1 partner) are
        // left for DCE — this was their only consumer.
        let v: i64 = if and_form { 1 } else { 0 };
        let c = int_const(*ty, v)?;
        return Some(vec![Instruction::Copy {
            dest: *dest,
            src: Operand::Const(c),
        }]);
    }
    let (core, cmp_dest) = range_test_core(&plan, and_form, next_id);
    let mut out = core;
    // The compare result is I8 (boolean). When the Select's result type is
    // I8 we can use `dest` directly; otherwise widen the boolean to the
    // Select's type so every downstream use keeps its type.
    if *ty == IrType::I8 {
        if let Some(Instruction::Cmp { dest: d, .. }) = out.last_mut() {
            *d = *dest;
        }
    } else {
        out.push(Instruction::Cast {
            dest: *dest,
            src: Operand::Value(cmp_dest),
            from_ty: IrType::I8,
            to_ty: *ty,
        });
    }
    Some(out)
}

/// Fold the BITWISE form of a two-sided range predicate:
/// `And(Cmp(lo-bound, x), Cmp(hi-bound, x))` (and the `Or` complement).
///
/// When if-conversion HAS collapsed the short-circuit control flow — the
/// normal `-O2` path for small guarded bodies like `if (in_range) r++;` —
/// the frontend/if-convert pair lowers `x >= lo && x <= hi` to
///
///   %c1 = Cmp(Sge, x, lo)
///   %c2 = Cmp(Sle, x, hi)
///   %a  = And %c1, %c2          (ty I32; operands are the 0/1 compares)
///   Select %a ? r+1 : r
///
/// The Select/Phi/branch folds never see a range: the And is not a
/// one-constant-armed Select, and the compares are branchless. GCC folds
/// exactly this shape in match.pd (`(X >= A) & (X <= B)`), because both
/// operands are provably 0/1 — bitwise AND *is* logical AND there.
///
/// Replace the And/Or with the unsigned-bias test; keep the compares for
/// any other uses (DCE retires them when this was the last one):
///
///   %d = Sub(x, lo); %t = Cmp(Ule, %d, hi - lo); [Cast %t -> and_ty]
///
/// Soundness: only Cmp-defined (or boolean-widening-cast-of-Cmp) operands
/// match, so both sides are exactly 0/1 and the bitwise/logical equivalence
/// holds. The same `extract_range` structure match feeds the shared
/// [`plan_range`] domain algebra as every other form. The result keeps the
/// And/Or's own type via the widening cast when it is wider than the
/// boolean.
fn try_fold_bool_op(
    inst: &Instruction,
    cmp_defs: &[Option<(IrCmpOp, Operand, Operand, IrType)>],
    cast_defs: &[Option<(Operand, IrType, IrType)>],
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    let Instruction::BinOp {
        dest,
        op,
        lhs,
        rhs,
        ty,
    } = inst
    else {
        return None;
    };
    let and_form = match op {
        IrBinOp::And => true,
        IrBinOp::Or => false,
        _ => return None,
    };
    if !ty.is_integer() {
        return None;
    }
    let lhs_id = match lhs {
        Operand::Value(v) => v.0,
        _ => return None,
    };
    let rhs_id = match rhs {
        Operand::Value(v) => v.0,
        _ => return None,
    };
    let lhs_cmp_id = resolve_bool_cmp(lhs_id, cmp_defs, cast_defs)?;
    let rhs_cmp_id = resolve_bool_cmp(rhs_id, cmp_defs, cast_defs)?;
    // Tautology guard: And(x, x) / Or(x, x) of one compare is not a range.
    if lhs_cmp_id == rhs_cmp_id {
        return None;
    }
    let (Some(&Some((op1, l1, r1, ty1))), Some(&Some((op2, l2, r2, ty2)))) = (
        cmp_defs.get(lhs_cmp_id as usize),
        cmp_defs.get(rhs_cmp_id as usize),
    ) else {
        return None;
    };
    let bound1 = canonicalize(op1, &l1, &r1, ty1)?;
    let bound2 = canonicalize(op2, &l2, &r2, ty2)?;
    let range = extract_range(&bound1, &bound2, and_form, cast_defs)?;
    let plan = plan_range(&range, cast_defs)?;
    if matches!(plan, RangePlan::FullDomain) {
        // Constant TRUE for the And (inside) form, FALSE for the Or
        // (outside) complement — the operand compares are left for DCE.
        let v: i64 = if and_form { 1 } else { 0 };
        let c = int_const(*ty, v)?;
        return Some(vec![Instruction::Copy {
            dest: *dest,
            src: Operand::Const(c),
        }]);
    }
    let (core, cmp_dest) = range_test_core(&plan, and_form, next_id);
    let mut out = core;
    if *ty == IrType::I8 || *ty == IrType::U8 {
        if let Some(Instruction::Cmp { dest: d, .. }) = out.last_mut() {
            *d = *dest;
        }
    } else {
        out.push(Instruction::Cast {
            dest: *dest,
            src: Operand::Value(cmp_dest),
            from_ty: IrType::I8,
            to_ty: *ty,
        });
    }
    Some(out)
}

/// Fold the PHI form of a short-circuit `&&`/`||` range predicate.
///
/// if_convert is disabled on the m16 size profile (measured: it grows the
/// boot corpus), so a value-context `x >= lo && x <= hi` — an inlined
/// isdigit/isxdigit-style predicate — stays a branch diamond whose merge is
/// a Phi:
///
///   Bcond:  %c1 = Cmp(<bound1>, x, K1)
///           CondBranch(%c1, Bcheck, Bmerge)        // either orientation
///   Bcheck: %c2 = Cmp(<bound2>, x, K2)
///           [%w  = Cast %c2 -> T]                  // boolean widening
///           Branch(Bmerge)
///   Bmerge: %p = Phi([Const 0|1, Bcond], [%w, Bcheck])
///           ... uses of %p ...
///
/// The backend materializes that Phi as two setcc + slot-store arms plus a
/// reload (~25 bytes where GCC needs one sub+cmp). When both bounds compare
/// the same value, replace the whole diamond with the unsigned-bias form
/// computed in Bcond:
///
///   %d = Sub(x, K1); %u = Cmp(Ule, %d, K2-K1); [Cast %u -> T]
///   Branch(Bmerge)                               // unconditional
///
/// with every use of %p replaced by %u. Bcheck becomes unreachable and is
/// dropped. All structural requirements are checked fail-closed: the Phi has
/// exactly the two diamond arms, Bcheck has Bcond as its only predecessor
/// (canonical CFG: computed-goto and asm-goto edges count) and branches
/// straight to Bmerge, Bmerge has no other predecessors, the second
/// comparison (and its widening cast) has no uses besides the Phi arm, the
/// cast chain of both comparison operands matches, and every OTHER leading
/// phi of Bmerge carries the SAME value on its Bcond and Bcheck arms (they
/// collapse onto one edge; differing values would need a merge that no
/// longer exists — reject). At most ONE diamond is folded per call;
/// [`run_function`] re-invokes with fresh analyses.
fn fold_phi_diamonds(
    func: &mut IrFunction,
    cmp_defs: &[Option<(IrCmpOp, Operand, Operand, IrType)>],
    cast_defs: &[Option<(Operand, IrType, IrType)>],
    next_id: &mut u32,
) -> usize {
    use crate::common::fx_hash::{FxHashMap, FxHashSet};

    if func.blocks.is_empty() {
        return 0;
    }
    // Canonical CFG ONLY: build_cfg counts IndirectBranch targets and
    // InlineAsm goto_labels edges, so a check block reachable through either
    // is not single-predecessor here. (A bespoke Branch/CondBranch/Switch
    // successor scan would happily delete a computed-goto target.)
    let idx_of: FxHashMap<u32, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label.0, i))
        .collect();
    let (preds, _succs) = {
        let label_map = crate::ir::analysis::build_label_map(func);
        crate::ir::analysis::build_cfg(func, &label_map)
    };

    let mut changes = 0usize;
    // The approved candidate's apply-time descriptor (owned values only —
    // the scan's immutable borrows end before the apply phase begins).
    struct ApprovedDiamond {
        bcond_idx: usize,
        jm: usize,
        pi: usize,
        new_insts: Vec<Instruction>,
        replacement: DiamondReplacement,
        phi_dest: Value,
        merge_label: u32,
        bcheck_label: u32,
    }
    /// What replaces the folded phi's value.
    enum DiamondReplacement {
        /// The range-test value (rewrites every use; the phi is deleted).
        Value(Operand),
        /// A full-domain constant: the phi itself becomes `Copy phi, const`
        /// (uses keep their reference; no rewrite, no dangling position).
        Const(IrConst),
    }
    let mut approved: Option<ApprovedDiamond> = None;
    'cands: for (jm, merge) in func.blocks.iter().enumerate() {
        for (pi, inst) in merge.instructions.iter().enumerate() {
            let Instruction::Phi {
                dest: phi_dest,
                incoming,
                ty: phi_ty,
            } = inst
            else {
                break; // phis lead the block
            };
            if incoming.len() != 2 {
                break;
            }
            let a = &incoming[0];
            let b = &incoming[1];
            let (const_arm, val_arm) = match (&a.0, &b.0) {
                (Operand::Const(_), Operand::Value(_)) => (a, b),
                (Operand::Value(_), Operand::Const(_)) => (b, a),
                _ => break,
            };
            // Constant arm must be 0 (&&) or 1 (||).
            let and_form = match const_arm.0 {
                Operand::Const(c) if c.to_i64() == Some(0) => true,
                Operand::Const(c) if c.to_i64() == Some(1) => false,
                _ => break,
            };

            let bcond_idx = match idx_of.get(&const_arm.1.0) {
                Some(&i) => i,
                None => continue,
            };
            let bcheck_idx = match idx_of.get(&val_arm.1.0) {
                Some(&i) => i,
                None => continue,
            };
            if bcond_idx == jm || bcheck_idx == jm || bcond_idx == bcheck_idx {
                continue;
            }

            // Bcheck: single predecessor (Bcond), branches straight to Bmerge.
            let bcond_label = func.blocks[bcond_idx].label.0;
            let bcheck_label = func.blocks[bcheck_idx].label.0;
            let merge_label = func.blocks[jm].label.0;
            if preds.len(jm) != 2
                || !preds.row(jm).contains(&(bcond_idx as u32))
                || !preds.row(jm).contains(&(bcheck_idx as u32))
            {
                continue;
            }
            if preds.len(bcheck_idx) != 1 || preds.row(bcheck_idx)[0] as usize != bcond_idx {
                continue;
            }
            if !matches!(func.blocks[bcheck_idx].terminator, Terminator::Branch(l) if l.0 == merge_label)
            {
                continue;
            }

            // Bcond must end in a CondBranch whose two targets are exactly
            // {Bcheck, Bmerge}; remember which side Bmerge is on so the first
            // comparison can be normalized to its "contributes to the phi"
            // sense.
            let (cond_val, merge_on_true) = {
                let bcond = &func.blocks[bcond_idx];
                let Terminator::CondBranch {
                    cond: Operand::Value(cv),
                    true_label,
                    false_label,
                } = &bcond.terminator
                else {
                    continue;
                };
                let t_ok = true_label.0 == bcheck_label || true_label.0 == merge_label;
                let f_ok = false_label.0 == bcheck_label || false_label.0 == merge_label;
                if !(t_ok && f_ok) || true_label == false_label {
                    continue;
                }
                (*cv, true_label.0 == merge_label)
            };

            // Resolve both comparisons (through boolean widenings).
            let Some(c1_id) = resolve_bool_cmp(cond_val.0, cmp_defs, cast_defs) else {
                continue;
            };
            let Some(c1) = cmp_defs.get(c1_id as usize).and_then(|d| d.as_ref()) else {
                continue;
            };
            // Normalize the first comparison to the sense that contributes to
            // the phi: the Bmerge edge carries the constant arm. For `&&` the
            // constant is 0 and the merge edge means "first test failed", so
            // the contributing sense is the branch edge to Bcheck; when Bmerge
            // is the TRUE target the comparison is inverted. For `||` (constant
            // 1 on the merge edge) it is the opposite.
            let negate = if and_form {
                merge_on_true
            } else {
                !merge_on_true
            };
            let c1_op = if negate {
                match c1.0 {
                    IrCmpOp::Slt => IrCmpOp::Sge,
                    IrCmpOp::Sle => IrCmpOp::Sgt,
                    IrCmpOp::Sgt => IrCmpOp::Sle,
                    IrCmpOp::Sge => IrCmpOp::Slt,
                    IrCmpOp::Ult => IrCmpOp::Uge,
                    IrCmpOp::Ule => IrCmpOp::Ugt,
                    IrCmpOp::Ugt => IrCmpOp::Ule,
                    IrCmpOp::Uge => IrCmpOp::Ult,
                    IrCmpOp::Eq => IrCmpOp::Ne,
                    IrCmpOp::Ne => IrCmpOp::Eq,
                }
            } else {
                c1.0
            };
            let val_id = match val_arm.0 {
                Operand::Value(v) => v.0,
                _ => continue,
            };
            let Some(c2_id) = resolve_bool_cmp(val_id, cmp_defs, cast_defs) else {
                continue;
            };
            let Some(c2) = cmp_defs.get(c2_id as usize).and_then(|d| d.as_ref()) else {
                continue;
            };
            let Some(bound1) = canonicalize(c1_op, &c1.1, &c1.2, c1.3) else {
                continue;
            };
            let Some(bound2) = canonicalize(c2.0, &c2.1, &c2.2, c2.3) else {
                continue;
            };

            let Some(range) = extract_range(&bound1, &bound2, and_form, cast_defs) else {
                continue;
            };
            let Some(plan) = plan_range(&range, cast_defs) else {
                continue;
            };

            // The second comparison (and its widening cast, when the phi arm
            // carries one) must have no uses OUTSIDE Bcheck and the phi arm:
            // Bcheck dies with the fold, taking its Cmp/Cast definitions along,
            // so any other live consumer would reference a dead value. Uses
            // inside Bcheck itself are exactly the instructions being deleted.
            // (Phi arms count as uses — verified against
            // `Instruction::for_each_used_value`.)
            let mut arm_ok = true;
            'outer: for block in &func.blocks {
                if block.label.0 == bcheck_label {
                    continue;
                }
                for inst in &block.instructions {
                    if let Instruction::Phi { dest, incoming, .. } = inst {
                        if dest.0 == phi_dest.0 {
                            continue;
                        }
                        if incoming.iter().any(
                            |(op, _)| matches!(op, Operand::Value(v) if v.0 == val_id || v.0 == c2_id),
                        ) {
                            arm_ok = false;
                            break 'outer;
                        }
                    }
                    let mut bad = false;
                    inst.for_each_used_value(|id| {
                        if id == val_id || id == c2_id {
                            bad = true;
                        }
                    });
                    if bad {
                        arm_ok = false;
                        break 'outer;
                    }
                }
            }
            if !arm_ok {
                continue;
            }

            // EVERY other leading phi of Bmerge must carry the same value on
            // its Bcond and Bcheck arms: the fold collapses the two edges
            // into one Bcond edge, so a differing pair would silently select
            // the Bcond value on the old-Bcheck path. Equal arms collapse to
            // the single Bcond arm (the Bcheck arm is dropped below).
            for (opi, other) in merge.instructions.iter().enumerate() {
                if opi == pi {
                    continue;
                }
                let Instruction::Phi { incoming, .. } = other else {
                    break; // phis lead the block
                };
                let Some(bc_arm) = incoming.iter().find(|(_, f)| f.0 == bcond_label) else {
                    continue 'cands; // malformed: missing pred arm — reject
                };
                let Some(bk_arm) = incoming.iter().find(|(_, f)| f.0 == bcheck_label) else {
                    continue 'cands; // malformed: missing pred arm — reject
                };
                if bc_arm.0 != bk_arm.0 {
                    continue 'cands; // differing merge values — reject
                }
            }

            // Build the replacement: sub + unsigned compare (+ widening cast
            // to the phi's type when it is not the boolean type itself) — or,
            // for a full-domain range, the constant itself.
            let (mut new_insts, replacement) = if matches!(plan, RangePlan::FullDomain) {
                // The inside form is constant TRUE (outside: FALSE). The
                // phi is rewritten IN PLACE into a Copy of the constant —
                // every use keeps referencing phi_dest (Operand and
                // bare-Value positions alike), so no use-rewrite is needed
                // and no dangling reference is possible.
                let Some(c) = int_const(*phi_ty, if and_form { 1 } else { 0 }) else {
                    continue 'cands;
                };
                (Vec::new(), DiamondReplacement::Const(c))
            } else {
                let (mut new_insts, cmp_dest) = range_test_core(&plan, and_form, next_id);
                // Replacement value for the phi's uses.
                let replacement: Operand = if *phi_ty == IrType::I8 || *phi_ty == IrType::U8 {
                    Operand::Value(cmp_dest)
                } else {
                    let cast_dest = Value(*next_id);
                    *next_id += 1;
                    new_insts.push(Instruction::Cast {
                        dest: cast_dest,
                        src: Operand::Value(cmp_dest),
                        from_ty: IrType::I8,
                        to_ty: *phi_ty,
                    });
                    Operand::Value(cast_dest)
                };
                (new_insts, DiamondReplacement::Value(replacement))
            };

            // Approved. Everything below mutates, so the descriptor is
            // handed to the apply phase AFTER the immutable scan ends.
            approved = Some(ApprovedDiamond {
                bcond_idx,
                jm,
                pi,
                new_insts,
                replacement,
                phi_dest: *phi_dest,
                merge_label,
                bcheck_label,
            });
            break 'cands;
        }
    }
    let Some(a) = approved else { return changes };

    // Apply: splice into Bcond, unconditional branch to Bmerge,
    // replace phi uses (or rewrite the phi to its full-domain constant),
    // delete the phi (and the now-dead Bcheck arm of every other merge
    // phi), drop Bcheck.
    {
        let ApprovedDiamond {
            bcond_idx,
            jm,
            pi,
            new_insts,
            replacement,
            phi_dest,
            merge_label,
            bcheck_label,
        } = a;
        let _ = merge_label;
        {
            let bcond = &mut func.blocks[bcond_idx];
            bcond.instructions.extend(new_insts);
            bcond.terminator = Terminator::Branch(crate::ir::reexports::BlockId(merge_label));
        }
        match replacement {
            DiamondReplacement::Value(replacement) => {
                // Replace every use of phi_dest with the replacement.
                for block in &mut func.blocks {
                    for inst in &mut block.instructions {
                        inst.for_each_operand_mut(|op: &mut Operand| {
                            if let Operand::Value(v) = op {
                                if v.0 == phi_dest.0 {
                                    *op = replacement;
                                }
                            }
                        });
                        // Bare-Value positions (Store ptr, GEP base, ...) are
                        // not Operands: the phi is deleted below, so any
                        // naming it here must be rewritten too (same two-walk
                        // contract as loop_memset; the replacement is always
                        // a Value).
                        if let Operand::Value(replacement_val) = replacement {
                            inst.for_each_value_use_mut(|v: &mut Value| {
                                if v.0 == phi_dest.0 {
                                    *v = replacement_val;
                                }
                            });
                        }
                    }
                    block.terminator.for_each_operand_mut(|op: &mut Operand| {
                        if let Operand::Value(v) = op {
                            if v.0 == phi_dest.0 {
                                *op = replacement;
                            }
                        }
                    });
                }
                // Drop the phi from Bmerge; collapse every other phi's
                // duplicate Bcheck arm (proven equal above).
                {
                    let merge = &mut func.blocks[jm];
                    merge.instructions.remove(pi);
                    for inst in &mut merge.instructions {
                        let Instruction::Phi { incoming, .. } = inst else {
                            break;
                        };
                        incoming.retain(|(_, f)| f.0 != bcheck_label);
                    }
                }
            }
            DiamondReplacement::Const(c) => {
                // Full domain: the phi IS the constant on every edge.
                // Rewrite it in place (after the block's remaining leading
                // phis, which must stay block-leading) so every use —
                // Operand or bare-Value — keeps its reference; no rewrite
                // walk, no dangling position.
                let merge = &mut func.blocks[jm];
                merge.instructions.remove(pi);
                let mut insert_at = 0;
                for (i, inst) in merge.instructions.iter().enumerate() {
                    if matches!(inst, Instruction::Phi { .. }) {
                        insert_at = i + 1;
                    } else {
                        break;
                    }
                }
                merge.instructions.insert(
                    insert_at,
                    Instruction::Copy {
                        dest: phi_dest,
                        src: Operand::Const(c),
                    },
                );
                for inst in &mut merge.instructions {
                    let Instruction::Phi { incoming, .. } = inst else {
                        break;
                    };
                    incoming.retain(|(_, f)| f.0 != bcheck_label);
                }
            }
        }
        func.blocks.retain(|b| b.label.0 != bcheck_label);
        changes += 1;
        return changes; // ONE fold per call — analyses restart in run_function
    }
}

/// Branch-condition range fusion — the short-circuit `&&`/`||` CFG form the
/// Select and Phi-diamond folds never see.
///
/// When the guarded body is too large for if-conversion (or the target is a
/// control-flow merge if-conversion declines to touch), C's
/// `if (x >= lo && x <= hi) {body} else {exit}` lowers to a two-block branch
/// chain that survives the whole pipeline:
///
///   Bcond:  ... c1 = Cmp(op1, x, K1) ...
///           CondBranch(c1, Bcheck, Bexit)        // `&&`: first half true → check second
///   Bcheck: [casts of x,] c2 = Cmp(op2, x, K2)
///           CondBranch(c2, Bbody, Bexit)         // both "no" edges → Bexit
///
/// The backend emits two compare-and-branch pairs per character where GCC's
/// jump threader fuses the pair into the unsigned-bias form on the FIRST
/// branch (csv_field_sum's digit test, Expat's classifiers, every
/// `c >= 'a' && c <= 'z'` loop guard). Fold:
///
///   Bcond:  ... %d = Sub(x, lo); %t = Cmp(Ule, %d, hi - lo)
///           CondBranch(%t, Bbody, Bexit)
///
/// The `||` form (`if (x < lo || x > hi) {exit} else {body}`) has the roles
/// swapped on both branches — Bcond's TRUE edge is the shared exit, Bcheck's
/// TRUE edge is the exit and its FALSE edge the body — and fuses to
/// `Cmp(Ugt, %d, hi - lo)` with the same target structure:
///
///   Bcond:  CondBranch(c1, Bexit, Bcheck)
///   Bcheck: CondBranch(c2, Bexit, Bbody)
///     →     CondBranch(%t, Bexit, Bbody)   // %t = Ugt(...) — out-of-range true
///
/// Soundness contract (fail-closed; a candidate failing any check keeps its
/// shape):
///
/// 1. `Bcheck` has exactly one predecessor, `Bcond` (canonical CFG:
///    computed-goto targets and asm-goto label edges count as predecessors),
///    and is a pure check block: its instructions are one `Cmp` plus casts
///    whose results are used only inside that same block. Nothing else.
/// 2. `c1` (the Bcond compare) is used ONLY by Bcond's terminator, and `c2`
///    only by Bcheck's. A value-context use (`int ok = a && b;`) belongs to
///    the Select/Phi forms and must not be stolen by this fold.
/// 3. The two compares canonicalize to the same root value (full cast-chain
///    identity, reusing `extract_range`) and pass the shared [`plan_range`]
///    domain algebra — non-empty in the comparison's own domain, span
///    representable, narrowing only between fitting bounds.
/// 4. Phi-edge rewrite discipline for the two retargeted successors:
///    - `Bbody` loses the Bcheck→Bbody edge and gains Bcond→Bbody. Every
///      phi arm keyed to Bcheck is retargeted to Bcond. This is sound
///      because any value arriving on that arm is either defined before
///      Bcond (it dominated Bcheck, whose only predecessor is Bcond, hence
///      it dominates Bcond's end too) or is `c2`/a Bcheck-local cast —
///      both excluded from having uses outside Bcheck by check 2. After
///      retargeting, a phi must not carry two arms keyed Bcond: if the
///      old Bcond arm and the retargeted arm differ, the fold is rejected
///      (the values would need a merge that no longer exists).
///    - `Bexit` keeps its Bcond arm and drops the Bcheck arm. The edge is
///      dead as control flow, but its PHI SELECTION is not: paths that used
///      to arrive through Bcheck still arrive (through the fused branch),
///      so the dropped arm's value must equal the surviving Bcond arm's
///      value for every leading phi of Bexit. A missing arm (malformed) or
///      a differing pair rejects the fold.
/// 5. `Bbody != Bexit` (a same-target chain is cfg_simplify's job, not a
///    range), and neither equals `Bcheck`.
/// 6. Bcheck's terminator is a `CondBranch` (not Switch/Return): the
///    second comparison must actually branch.
///
/// At most ONE chain is folded per call; [`run_function`] re-invokes with
/// freshly built analyses until no candidate fires.
fn fold_branch_chains(
    func: &mut IrFunction,
    cmp_defs: &[Option<(IrCmpOp, Operand, Operand, IrType)>],
    cast_defs: &[Option<(Operand, IrType, IrType)>],
    next_id: &mut u32,
) -> usize {
    use crate::common::fx_hash::{FxHashMap, FxHashSet};

    if func.blocks.is_empty() {
        return 0;
    }
    // Canonical CFG ONLY (see fold_phi_diamonds).
    let idx_of: FxHashMap<u32, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label.0, i))
        .collect();
    let (preds, _succs) = {
        let label_map = crate::ir::analysis::build_label_map(func);
        crate::ir::analysis::build_cfg(func, &label_map)
    };

    // Total use count of every value id across instructions, terminators and
    // phi arms — the "only use" checks need the whole function, not a block.
    let max_id = func.max_value_id() as usize;
    let mut uses: Vec<u32> = vec![0; max_id + 2];
    for block in &func.blocks {
        for inst in &block.instructions {
            inst.for_each_used_value(|id| {
                if (id as usize) < uses.len() {
                    uses[id as usize] += 1;
                }
            });
        }
        block.terminator.for_each_used_value(|id| {
            if (id as usize) < uses.len() {
                uses[id as usize] += 1;
            }
        });
    }

    // A pure check block: exactly one Cmp (the terminator's condition) plus
    // Casts whose results are used only inside this block. Returns the
    // compare's value id. The single `uses` count per def is split into
    // local instruction uses and terminator uses; any remainder means the
    // value escapes the block.
    fn check_block_cmp(block: &crate::ir::reexports::BasicBlock, uses: &[u32]) -> Option<u32> {
        let mut cmp_id = None;
        let mut local_defs: Vec<u32> = Vec::new();
        for inst in &block.instructions {
            match inst {
                Instruction::Cmp { dest, .. } => {
                    if cmp_id.is_some() {
                        return None; // more than one compare
                    }
                    cmp_id = Some(dest.0);
                    local_defs.push(dest.0);
                }
                Instruction::Cast { dest, .. } => local_defs.push(dest.0),
                _ => return None,
            }
        }
        let cmp_id = cmp_id?;
        let mut local_use_count: FxHashMap<u32, u32> = FxHashMap::default();
        for inst in &block.instructions {
            inst.for_each_used_value(|id| {
                if local_defs.contains(&id) {
                    *local_use_count.entry(id).or_insert(0) += 1;
                }
            });
        }
        let mut terminator_uses: Vec<u32> = Vec::new();
        block
            .terminator
            .for_each_used_value(|id| terminator_uses.push(id));
        for &d in &local_defs {
            let total = uses.get(d as usize).copied().unwrap_or(0);
            let local = local_use_count.get(&d).copied().unwrap_or(0);
            let term = u32::try_from(terminator_uses.iter().filter(|&&u| u == d).count())
                .unwrap_or(u32::MAX);
            if total != local + term {
                return None; // escapes the block
            }
            if d != cmp_id && term != 0 {
                return None; // only the compare may reach the terminator
            }
        }
        match &block.terminator {
            Terminator::CondBranch {
                cond: Operand::Value(v),
                ..
            } if v.0 == cmp_id => Some(cmp_id),
            _ => None,
        }
    }

    // Candidate scan (immutable borrow): (bcond index, check label,
    // shared-exit label, and_form) in block order. The first candidate
    // whose CURRENT-IR re-validation succeeds folds; the fold itself
    // re-derives every structural fact, so a candidate touched by an
    // earlier fold in this same call cannot fold against stale structure.
    let mut scan: Vec<(usize, u32, u32, bool)> = Vec::new();
    for (bcond_idx, bcond) in func.blocks.iter().enumerate() {
        let Terminator::CondBranch {
            cond: Operand::Value(c1),
            true_label,
            false_label,
        } = &bcond.terminator
        else {
            continue;
        };
        let c1 = c1.0;
        // c1 must be a compare used ONLY by this terminator.
        if uses.get(c1 as usize).copied().unwrap_or(0) != 1 {
            continue;
        }
        // `&&`: CondBranch(c1, Bcheck, Bexit); `||`: CondBranch(c1, Bexit, Bcheck).
        for (check_label, exit_label, and_form) in [
            (true_label.0, false_label.0, true),
            (false_label.0, true_label.0, false),
        ] {
            let Some(&bi) = idx_of.get(&check_label) else {
                continue;
            };
            if preds.row(bi) != [bcond_idx as u32] {
                continue; // not a dedicated single-pred check block
            }
            if check_block_cmp(&func.blocks[bi], &uses).is_none() {
                continue;
            }
            scan.push((bcond_idx, check_label, exit_label, and_form));
        }
    }
    // Apply (mutable borrow): first candidate that validates on the CURRENT
    // IR wins; one fold per call.
    for (bcond_idx, check_label, exit_label, and_form) in scan {
        let Some(&bi) = idx_of.get(&check_label) else {
            continue;
        };
        if preds.row(bi) != [bcond_idx as u32] {
            continue; // re-validate: still a dedicated single-pred block
        }
        if try_fold_one_branch_chain(
            func, &idx_of, &preds, cmp_defs, cast_defs, next_id, bcond_idx, bi, exit_label,
            and_form,
        )
        .is_some()
        {
            return 1; // ONE fold per call — analyses restart in run_function
        }
    }
    0
}

/// Every out-edge label of a block: terminator targets (including
/// `IndirectBranch::possible_targets` and `Switch` cases) plus any
/// `InlineAsm` goto labels in its instructions — the same edge census the
/// canonical `build_cfg` performs.
fn block_successor_labels(block: &crate::ir::reexports::BasicBlock) -> Vec<u32> {
    let mut out = Vec::new();
    match &block.terminator {
        Terminator::Branch(b) => out.push(b.0),
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            out.push(true_label.0);
            out.push(false_label.0);
        }
        Terminator::Switch { cases, default, .. } => {
            out.extend(cases.iter().map(|(_, x)| x.0));
            out.push(default.0);
        }
        Terminator::IndirectBranch {
            possible_targets, ..
        } => out.extend(possible_targets.iter().map(|x| x.0)),
        _ => {}
    }
    for inst in &block.instructions {
        if let Instruction::InlineAsm { goto_labels, .. } = inst {
            out.extend(goto_labels.iter().map(|(_, b)| b.0));
        }
    }
    out
}

/// Audit a block for deletion after its last in-edges died: no value it
/// defines may be used anywhere else (phi arms count as uses), and no
/// successor may lose its last remaining predecessor besides blocks that
/// die in the same fold. Fail-closed: any unknown returns false and the
/// caller keeps the block (declining the fold when the block would
/// otherwise be left predecessor-less).
fn dead_block_is_droppable(
    func: &IrFunction,
    idx_of: &crate::common::fx_hash::FxHashMap<u32, usize>,
    preds: &crate::ir::analysis::FlatAdj,
    dead_label: u32,
    dying: &[u32],
) -> bool {
    let Some(&di) = idx_of.get(&dead_label) else {
        return false;
    };
    let dead = &func.blocks[di];
    // (a) Every definition of the dead block must be dead outside it.
    let defs: crate::common::fx_hash::FxHashSet<u32> = dead
        .instructions
        .iter()
        .filter_map(|i| i.dest().map(|d| d.0))
        .collect();
    if !defs.is_empty() {
        for (bi, block) in func.blocks.iter().enumerate() {
            if bi == di {
                continue;
            }
            let mut used = false;
            for inst in &block.instructions {
                inst.for_each_used_value(|id| {
                    if defs.contains(&id) {
                        used = true;
                    }
                });
            }
            block.terminator.for_each_used_value(|id| {
                if defs.contains(&id) {
                    used = true;
                }
            });
            if used {
                return false;
            }
        }
    }
    // (b) Every surviving successor keeps a predecessor outside the dying
    // set (checked on the canonical CFG, so computed-goto and asm-goto
    // edges count).
    for succ in block_successor_labels(dead) {
        if succ == dead_label || dying.contains(&succ) {
            continue;
        }
        let Some(&si) = idx_of.get(&succ) else {
            return false;
        };
        let keeps = preds.row(si).iter().any(|&p| {
            let pl = func.blocks[p as usize].label.0;
            pl != dead_label && !dying.contains(&pl)
        });
        if !keeps {
            return false;
        }
    }
    true
}

/// Delete a dead block: first drop its keyed arms from every surviving
/// successor's leading phis (the block's terminator still names them),
/// then remove the block.
fn drop_dead_block(func: &mut IrFunction, dead_label: u32) {
    let Some(di) = func.blocks.iter().position(|b| b.label.0 == dead_label) else {
        return;
    };
    let targets = block_successor_labels(&func.blocks[di]);
    for t in targets {
        if t == dead_label {
            continue;
        }
        if let Some(s) = func.blocks.iter_mut().find(|b| b.label.0 == t) {
            for inst in &mut s.instructions {
                let Instruction::Phi { incoming, .. } = inst else {
                    break;
                };
                incoming.retain(|(_, f)| f.0 != dead_label);
            }
        }
    }
    func.blocks.retain(|b| b.label.0 != dead_label);
}

/// Validate and apply one branch-chain candidate. Every structural fact is
/// re-derived from the CURRENT IR (never from the scan's snapshot), so a
/// candidate whose blocks were touched by an earlier fold in the same pass
/// run cannot be folded against stale structure.
#[allow(clippy::too_many_arguments)]
fn try_fold_one_branch_chain(
    func: &mut IrFunction,
    idx_of: &crate::common::fx_hash::FxHashMap<u32, usize>,
    preds: &crate::ir::analysis::FlatAdj,
    cmp_defs: &[Option<(IrCmpOp, Operand, Operand, IrType)>],
    cast_defs: &[Option<(Operand, IrType, IrType)>],
    next_id: &mut u32,
    bcond_idx: usize,
    bcheck_idx: usize,
    bexit_label: u32,
    and_form: bool,
) -> Option<()> {
    let bcond_label = func.blocks[bcond_idx].label.0;
    let bcheck_label = func.blocks[bcheck_idx].label.0;

    // --- structural read-out of the pair ---------------------------------
    let (c1_id, c2_id, bbody_label) = {
        let bcond = &func.blocks[bcond_idx];
        let bcheck = &func.blocks[bcheck_idx];
        let (
            Terminator::CondBranch {
                cond: Operand::Value(c1),
                true_label: t1,
                false_label: f1,
            },
            Terminator::CondBranch {
                cond: Operand::Value(c2),
                true_label: t2,
                false_label: f2,
            },
        ) = (&bcond.terminator, &bcheck.terminator)
        else {
            return None;
        };
        let (c1, c2) = (c1.0, c2.0);
        // Re-derive the orientation from the actual targets.
        let (check_of_bcond, exit_of_bcond) = if and_form { (t1.0, f1.0) } else { (f1.0, t1.0) };
        if check_of_bcond != bcheck_label || exit_of_bcond != bexit_label {
            return None;
        }
        let (body_of_check, exit_of_check) = if and_form { (t2.0, f2.0) } else { (f2.0, t2.0) };
        if exit_of_check != bexit_label {
            return None; // the "no" edges must share one exit
        }
        if body_of_check == bexit_label
            || body_of_check == bcheck_label
            || bexit_label == bcheck_label
        {
            return None;
        }
        (c1, c2, body_of_check)
    };

    // --- the two bounds ---------------------------------------------------
    let (Some(&Some((op1, l1, r1, ty1))), Some(&Some((op2, l2, r2, ty2)))) =
        (cmp_defs.get(c1_id as usize), cmp_defs.get(c2_id as usize))
    else {
        return None;
    };
    let (Some(bound1), Some(bound2)) = (
        canonicalize(op1, &l1, &r1, ty1),
        canonicalize(op2, &l2, &r2, ty2),
    ) else {
        return None;
    };
    let range = extract_range(&bound1, &bound2, and_form, cast_defs)?;
    let plan = plan_range(&range, cast_defs)?;
    let full = matches!(plan, RangePlan::FullDomain);

    // --- full-domain structural guards -------------------------------------
    // The dead side (Bexit, in both forms — the inside test is constant
    // TRUE, so the outside continuation is never taken) is rewritten below;
    // a dead side that IS the block being rewritten (a self-referential
    // chain) needs surgery this fold does not model, and Bbody being
    // Bcond itself would make the fold's live target a self-branch.
    // Pathological, and not worth the audit surface.
    if full && (bexit_label == bcond_label || bbody_label == bcond_label) {
        return None;
    }

    // --- phi-edge discipline on Bbody and Bexit ---------------------------
    // Non-full: Bbody retargets its Bcheck arms to Bcond (reject on a
    // conflicting existing Bcond arm); Bexit keeps the fused branch's false
    // paths alive, so every leading phi must carry the SAME value on its
    // Bcond and Bcheck arms.
    // Full-domain (BOTH forms): the constant condition is TRUE in the
    // "value flows to Bbody" sense — for `&&` the inside test is constant
    // TRUE (Bcond branches to Bbody), and for `||` the outside test is
    // constant FALSE (Bcond branches to Bbody, the outside test's FALSE
    // continuation). Bexit loses BOTH chain edges in either form — its
    // arms are simply dropped (no arm-equality needed), and when the chain
    // was its only predecessor it is deleted outright (audited). Bbody is
    // the fold's LIVE target in both forms; its Bcheck arms retarget to
    // the new Bcond edge (value-safe: Bcheck's dedicated-block discipline
    // forbids any use of its defs outside it, so a phi arm from Bcheck
    // references a value defined outside Bcheck, which dominates Bcond).
    let mut drop_bexit = false;
    if full {
        let Some(&bexit_idx) = idx_of.get(&bexit_label) else {
            return None;
        };
        let chain_only = preds.row(bexit_idx).iter().all(|&p| {
            let l = func.blocks[p as usize].label.0;
            l == bcond_label || l == bcheck_label
        });
        if chain_only {
            if !dead_block_is_droppable(func, idx_of, preds, bexit_label, &[bcheck_label]) {
                return None;
            }
            drop_bexit = true;
        }
    } else {
        let bbody_idx = match idx_of.get(&bbody_label) {
            Some(&i) => i,
            None => return None,
        };
        let bexit_idx = match idx_of.get(&bexit_label) {
            Some(&i) => i,
            None => return None,
        };
        let mut bbody_ok = true;
        let mut bexit_ok = true;
        for (bi, ok) in [(bbody_idx, &mut bbody_ok), (bexit_idx, &mut bexit_ok)] {
            for inst in &func.blocks[bi].instructions {
                let Instruction::Phi { incoming, .. } = inst else {
                    break; // phis lead the block
                };
                for (arm_val, from) in incoming {
                    if from.0 == bcheck_label {
                        if bi == bbody_idx {
                            if incoming
                                .iter()
                                .any(|(v2, f2)| f2.0 == bcond_label && *v2 != *arm_val)
                            {
                                *ok = false;
                            }
                        } else if bi == bexit_idx {
                            // Dead-edge removal with live phi selection: the
                            // Bcheck arm must equal the surviving Bcond arm.
                            match incoming.iter().find(|(_, f2)| f2.0 == bcond_label) {
                                Some((v2, _)) if *v2 != *arm_val => *ok = false,
                                Some(_) => {}
                                None => *ok = false, // malformed: no Bcond arm
                            }
                        }
                    }
                }
                if !*ok {
                    break;
                }
            }
        }
        if !bbody_ok || !bexit_ok {
            return None;
        }
    }

    // --- build the replacement terminator -----------------------------------
    let (new_insts, new_term) = if full {
        // Constant condition: control flows one way, unconditionally. A
        // full-domain range makes the INSIDE test true for every operand
        // value — the `&&` form's conjunction holds and the `||` form's
        // disjunction fails — so Bcond branches to Bbody in BOTH forms.
        // (The outside test being constant FALSE selects its FALSE
        // continuation, which for the `||` chain is Bbody: c1 false → Bcheck,
        // c2 false → Bbody.)
        (
            Vec::new(),
            Terminator::Branch(crate::ir::reexports::BlockId(bbody_label)),
        )
    } else {
        let (new_insts, cmp_dest) = range_test_core(&plan, and_form, next_id);
        let (new_true, new_false) = if and_form {
            (
                crate::ir::reexports::BlockId(bbody_label),
                crate::ir::reexports::BlockId(bexit_label),
            )
        } else {
            (
                crate::ir::reexports::BlockId(bexit_label),
                crate::ir::reexports::BlockId(bbody_label),
            )
        };
        (
            new_insts,
            Terminator::CondBranch {
                cond: Operand::Value(cmp_dest),
                true_label: new_true,
                false_label: new_false,
            },
        )
    };

    // --- apply --------------------------------------------------------------
    // Order matters: every `idx_of` lookup happens BEFORE any block is
    // dropped (a drop shifts block indices and stales the map). The
    // dead-block drops run last, by label.
    {
        let bcond = &mut func.blocks[bcond_idx];
        bcond.instructions.extend(new_insts);
        bcond.terminator = new_term;
    }
    // Bbody phis: EVERY form gains the bcond→bbody edge, so Bcheck arms
    // retarget to Bcond (a duplicate Bcond arm with the same value
    // collapses onto it). The full-domain `||` form is no exception: its
    // target is Bbody exactly as the `&&` form's is.
    if let Some(&bbody_idx) = idx_of.get(&bbody_label) {
        let bbody = &mut func.blocks[bbody_idx];
        let mut arm_rewrite: Vec<(usize, Option<usize>)> = Vec::new();
        for (pi, inst) in bbody.instructions.iter().enumerate() {
            let Instruction::Phi { incoming, .. } = inst else {
                break;
            };
            for (ai, (_, from)) in incoming.iter().enumerate() {
                if from.0 == bcheck_label {
                    let dup = incoming
                        .iter()
                        .position(|(v2, f2)| f2.0 == bcond_label && *v2 == incoming[ai].0);
                    arm_rewrite.push((pi, dup));
                }
            }
        }
        for (pi, dup) in arm_rewrite.into_iter().rev() {
            if let Instruction::Phi { incoming, .. } = &mut bbody.instructions[pi] {
                let pos = incoming
                    .iter()
                    .position(|(_, f)| f.0 == bcheck_label)
                    .expect("arm existed in the scan");
                if let Some(_dup_with_same_value) = dup {
                    // Duplicate with identical value: drop the Bcheck arm,
                    // keeping the existing Bcond arm.
                    incoming.remove(pos);
                } else {
                    incoming[pos].1 = crate::ir::reexports::BlockId(bcond_label);
                }
            }
        }
    }
    // Bexit phis: full-domain forms lose BOTH chain arms (Bcond no longer
    // branches there and Bcheck is deleted); every other case keeps the
    // Bcond arm and drops Bcheck (proven value-equal above in the non-full
    // form).
    if full {
        if let Some(&bexit_idx) = idx_of.get(&bexit_label) {
            let bexit = &mut func.blocks[bexit_idx];
            for inst in &mut bexit.instructions {
                let Instruction::Phi { incoming, .. } = inst else {
                    break;
                };
                incoming.retain(|(_, f)| f.0 != bcond_label && f.0 != bcheck_label);
            }
        }
    } else if let Some(&bexit_idx) = idx_of.get(&bexit_label) {
        let bexit = &mut func.blocks[bexit_idx];
        for inst in &mut bexit.instructions {
            let Instruction::Phi { incoming, .. } = inst else {
                break;
            };
            incoming.retain(|(_, f)| f.0 != bcheck_label);
        }
    }
    // Dead-block drops, by label, after every index-based surgery. Bexit is
    // the dead side in both full-domain forms; Bbody is the live target and
    // is never dropped.
    if drop_bexit {
        drop_dead_block(func, bexit_label);
    }
    func.blocks.retain(|b| b.label.0 != bcheck_label);
    Some(())
}

/// Definition maps for one snapshot of the function. Rebuilt after EVERY
/// CFG-mutating fold — the folds rewrite operands (phi-use replacement) and
/// delete instructions, so a map from an earlier snapshot can hand a later
/// fold a deleted value (a `Sub` over a removed phi) or a stale operand.
struct DefMaps {
    cmp_defs: Vec<Option<(IrCmpOp, Operand, Operand, IrType)>>,
    cast_defs: Vec<Option<(Operand, IrType, IrType)>>,
    binop_defs: Vec<Option<(IrBinOp, Operand, Operand, IrType)>>,
}

fn build_def_maps(func: &IrFunction) -> DefMaps {
    let max_id = func.max_value_id() as usize;
    let mut maps = DefMaps {
        cmp_defs: vec![None; max_id + 1],
        cast_defs: vec![None; max_id + 1],
        binop_defs: vec![None; max_id + 1],
    };
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Cmp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => {
                    let idx = dest.0 as usize;
                    if idx < maps.cmp_defs.len() {
                        maps.cmp_defs[idx] = Some((*op, *lhs, *rhs, *ty));
                    }
                }
                Instruction::Cast {
                    dest,
                    src,
                    from_ty,
                    to_ty,
                } => {
                    let idx = dest.0 as usize;
                    if idx < maps.cast_defs.len() {
                        maps.cast_defs[idx] = Some((*src, *from_ty, *to_ty));
                    }
                }
                Instruction::BinOp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => {
                    let idx = dest.0 as usize;
                    if idx < maps.binop_defs.len() {
                        maps.binop_defs[idx] = Some((*op, *lhs, *rhs, *ty));
                    }
                }
                _ => {}
            }
        }
    }
    maps
}

/// Run the range-check fold over one function. Returns the number of folds.
pub(crate) fn run_function(func: &mut IrFunction) -> usize {
    let mut next_id = func.next_value_id;
    if next_id == 0 {
        next_id = func.max_value_id() + 1;
    }
    let mut total = 0usize;

    // CFG-mutating folds: ONE application per iteration, with definition
    // maps and CFG rebuilt from the post-fold IR every time. Terminates
    // because each successful iteration deletes one block.
    loop {
        let maps = build_def_maps(func);
        let n = fold_phi_diamonds(func, &maps.cmp_defs, &maps.cast_defs, &mut next_id);
        if n > 0 {
            total += n;
            continue;
        }
        let m = fold_branch_chains(func, &maps.cmp_defs, &maps.cast_defs, &mut next_id);
        if m > 0 {
            total += m;
            continue;
        }
        break;
    }

    // Path facts are computed ONLY after every block removal: indexing them
    // before the folds let a removal shift later blocks onto earlier facts,
    // attaching "x is positive here" to an unrelated block and folding a
    // compare that was never proven (the gcc torture 20041114-1 idiom
    // misfiring onto its neighbour). Single-predecessor-ness of the fact
    // target is decided by the canonical CFG (switch, computed-goto and
    // asm-goto edges all count).
    //
    // One-block path-sensitive truth for the canonical short-circuit shape:
    //
    //   if (x <= 0) goto join; else goto check;
    // check:
    //   if ((unsigned)(x - 1) < UINT_MAX) goto join; else goto fail;
    //
    // On the false edge of `x <= 0`, signed 32-bit `x` is in [1, INT_MAX], so
    // `(u32)(x - 1)` is in [0, INT_MAX-1] and is necessarily below UINT_MAX.
    // GCC torture 20041114-1 uses this exact overflow-sensitive idiom to make
    // the `link_failure` edge unreachable.  This is deliberately local and
    // typed: no global range lattice is invented here, but the proof is strong
    // enough to remove the dead edge without weakening C signed-overflow rules.
    let mut block_known_pos_i32: Vec<Option<Value>> = vec![None; func.blocks.len()];
    {
        let maps = build_def_maps(func);
        let idx_of: crate::common::fx_hash::FxHashMap<u32, usize> = func
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.label.0, i))
            .collect();
        let (preds, _succs) = {
            let label_map = crate::ir::analysis::build_label_map(func);
            crate::ir::analysis::build_cfg(func, &label_map)
        };
        for pred in &func.blocks {
            let Terminator::CondBranch {
                cond: Operand::Value(cond_v),
                true_label,
                false_label,
            } = pred.terminator
            else {
                continue;
            };
            let Some((op, lhs, rhs, ty)) = maps.cmp_defs.get(cond_v.0 as usize).and_then(|x| *x)
            else {
                continue;
            };
            if ty != IrType::I32 {
                continue;
            }
            let known = match (op, lhs, rhs) {
                (IrCmpOp::Sle, Operand::Value(x), Operand::Const(c)) if c.to_i64() == Some(0) => {
                    Some((false_label, x))
                }
                (IrCmpOp::Sgt, Operand::Value(x), Operand::Const(c)) if c.to_i64() == Some(0) => {
                    Some((true_label, x))
                }
                (IrCmpOp::Sge, Operand::Const(c), Operand::Value(x)) if c.to_i64() == Some(0) => {
                    Some((false_label, x))
                }
                (IrCmpOp::Slt, Operand::Const(c), Operand::Value(x)) if c.to_i64() == Some(0) => {
                    Some((true_label, x))
                }
                _ => None,
            };
            if let Some((label, x)) = known {
                if let Some(&idx) = idx_of.get(&label.0) {
                    // Keep it single-predecessor: joining different path facts
                    // requires a real range lattice, not this local edge fact.
                    // (Canonical preds: every edge kind counts.)
                    if preds.len(idx) == 1 {
                        block_known_pos_i32[idx] = Some(x);
                    }
                }
            }
        }
    }

    // Instruction-level folds (Select / Boolean op / path fact): these do
    // not mutate the CFG, so ONE sweep over freshly built maps is sound.
    // (Phi-use rewrites from the CFG folds above are reflected here because
    // the maps were rebuilt after them.)
    let maps = build_def_maps(func);
    for (block_idx, block) in func.blocks.iter_mut().enumerate() {
        let mut new_insts: Vec<Instruction> = Vec::with_capacity(block.instructions.len());
        let known_pos = block_known_pos_i32[block_idx];
        for inst in block.instructions.drain(..) {
            let path_fold = if let (
                Some(x),
                Instruction::Cmp {
                    dest,
                    op: IrCmpOp::Ult,
                    lhs: Operand::Value(cast_v),
                    rhs: Operand::Const(limit),
                    ty: IrType::U32,
                },
            ) = (known_pos, &inst)
            {
                if limit.to_i64() == Some(u32::MAX as i64) || limit.to_i64() == Some(-1) {
                    let cast_src = maps
                        .cast_defs
                        .get(cast_v.0 as usize)
                        .and_then(|d| d.as_ref())
                        .and_then(|(src, _from_ty, to_ty)| {
                            if *to_ty == IrType::U32 {
                                if let Operand::Value(v) = src {
                                    Some(*v)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        });
                    let is_x_minus_one = cast_src
                        .and_then(|sub_v| {
                            maps.binop_defs
                                .get(sub_v.0 as usize)
                                .and_then(|d| d.as_ref())
                        })
                        .is_some_and(|(op, lhs, rhs, ty)| {
                            *op == IrBinOp::Sub
                                && *ty == IrType::I32
                                && matches!(lhs, Operand::Value(v) if *v == x)
                                && matches!(rhs, Operand::Const(c) if c.to_i64() == Some(1))
                        });
                    if is_x_minus_one {
                        Some(Instruction::Copy {
                            dest: *dest,
                            src: Operand::Const(IrConst::I8(1)),
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(replacement) = path_fold {
                total += 1;
                new_insts.push(replacement);
            } else if let Some(replacements) =
                try_fold_select(&inst, &maps.cmp_defs, &maps.cast_defs, &mut next_id).or_else(
                    || try_fold_bool_op(&inst, &maps.cmp_defs, &maps.cast_defs, &mut next_id),
                )
            {
                total += 1;
                new_insts.extend(replacements);
            } else {
                new_insts.push(inst);
            }
        }
        block.instructions = new_insts;
    }
    func.next_value_id = next_id;
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fx_hash::FxHashSet;
    use crate::ir::reexports::BlockId;

    fn block(
        label: u32,
        insts: Vec<Instruction>,
        term: Terminator,
    ) -> crate::ir::reexports::BasicBlock {
        crate::ir::reexports::BasicBlock {
            label: BlockId(label),
            instructions: insts,
            terminator: term,
            source_spans: Vec::new(),
        }
    }

    /// The repository SSA/CFG validator, run on every transformed test IR:
    /// phi arms must match predecessors exactly, dominators must hold, and
    /// no dangling value may survive the CFG surgery.
    fn assert_verified(f: &IrFunction, stage: &str) {
        crate::passes::verify::verify_after_func_pass(f, stage);
    }

    /// `resolve_bool_cmp` must see through the boolean widening cast at EVERY
    /// width. The allow-list used to stop at `I16 | I32 | U32`, silently
    /// excluding the `U8 -> I64` widening the frontend actually emits for
    /// `char c; c >= 'a' && c <= 'z'` -- so the pass never fired on its own
    /// headline idiom, and `set_membership` downstream was starved with it.
    #[test]
    fn a_boolean_widening_cast_is_transparent_at_every_width() {
        let mut cmp_defs: Vec<Option<(IrCmpOp, Operand, Operand, IrType)>> = vec![None; 4];
        cmp_defs[1] = Some((
            IrCmpOp::Sle,
            Operand::Value(Value(0)),
            Operand::Const(IrConst::I8(122)),
            IrType::I8,
        ));

        for to_ty in [
            IrType::I16,
            IrType::U16,
            IrType::I32,
            IrType::U32,
            IrType::I64,
            IrType::U64,
            IrType::I128,
            IrType::U128,
        ] {
            let mut cast_defs: Vec<Option<(Operand, IrType, IrType)>> = vec![None; 4];
            cast_defs[2] = Some((Operand::Value(Value(1)), IrType::U8, to_ty));
            assert_eq!(
                resolve_bool_cmp(2, &cmp_defs, &cast_defs),
                Some(1),
                "widening a Cmp result to {:?} must stay transparent",
                to_ty
            );
        }
    }

    /// SOUNDNESS: only WIDENING is transparent. A narrowing cast can discard
    /// bits of a value that is not actually a boolean, so the fold must not
    /// see through it.
    #[test]
    fn a_narrowing_cast_is_not_transparent() {
        let mut cmp_defs: Vec<Option<(IrCmpOp, Operand, Operand, IrType)>> = vec![None; 4];
        cmp_defs[1] = Some((
            IrCmpOp::Sle,
            Operand::Value(Value(0)),
            Operand::Const(IrConst::I32(122)),
            IrType::I32,
        ));
        let mut cast_defs: Vec<Option<(Operand, IrType, IrType)>> = vec![None; 4];
        cast_defs[2] = Some((Operand::Value(Value(1)), IrType::I32, IrType::I8));
        assert_eq!(resolve_bool_cmp(2, &cmp_defs, &cast_defs), None);
    }

    #[test]
    fn canonicalize_lower_upper() {
        let lo = Operand::Const(IrConst::I32(97));
        let hi = Operand::Const(IrConst::I32(122));
        let x = Operand::Value(Value(1));

        let b1 = canonicalize(IrCmpOp::Sge, &x, &lo, IrType::I32).unwrap();
        assert_eq!(b1.kind, BoundKind::InclLower);
        assert_eq!(b1.bound, 97);

        let b2 = canonicalize(IrCmpOp::Sle, &x, &hi, IrType::I32).unwrap();
        assert_eq!(b2.kind, BoundKind::InclUpper);
        assert_eq!(b2.bound, 122);

        // Constant on the LHS: `97 <= x` is the same lower-inclusive bound.
        let b3 = canonicalize(IrCmpOp::Sle, &lo, &x, IrType::I32).unwrap();
        assert_eq!(b3.kind, BoundKind::InclLower);
        assert_eq!(b3.bound, 97);
    }

    #[test]
    fn extract_and_range() {
        let x = Value(1);
        let no_casts: Vec<Option<(Operand, IrType, IrType)>> = vec![None; 8];
        let lo = Bound {
            value: x,
            bound: 97,
            ty: IrType::I32,
            kind: BoundKind::InclLower,
            signed: true,
        };
        let hi = Bound {
            value: x,
            bound: 122,
            ty: IrType::I32,
            kind: BoundKind::InclUpper,
            signed: true,
        };
        let r = extract_range(&lo, &hi, true, &no_casts).unwrap();
        assert_eq!((r.lo, r.hi), (97, 122));

        // Swapped order still matches.
        let r2 = extract_range(&hi, &lo, true, &no_casts).unwrap();
        assert_eq!((r2.lo, r2.hi), (97, 122));

        // A collapsed range (x >= 122 && x <= 122) IS foldable (== 122).
        let bad = Bound {
            value: x,
            bound: 122,
            ty: IrType::I32,
            kind: BoundKind::InclLower,
            signed: true,
        };
        let both = Bound {
            value: x,
            bound: 122,
            ty: IrType::I32,
            kind: BoundKind::InclUpper,
            signed: true,
        };
        let r3 = extract_range(&bad, &both, true, &no_casts).unwrap();
        assert_eq!((r3.lo, r3.hi), (122, 122));
        // And plan_range confirms the domain algebra: empty ranges reject.
        assert!(plan_for(IrType::I32, 122, 97).is_none());
    }

    #[test]
    fn follow_casts_matches_identical_chains() {
        // Two separate u8->i32 casts of the same root are the same operand.
        let x = Value(1);
        let mut cast_defs: Vec<Option<(Operand, IrType, IrType)>> = vec![None; 16];
        cast_defs[4] = Some((Operand::Value(x), IrType::U8, IrType::I32));
        cast_defs[8] = Some((Operand::Value(x), IrType::U8, IrType::I32));
        assert_eq!(
            follow_casts(Value(4), &cast_defs),
            follow_casts(Value(8), &cast_defs)
        );

        // A different cast chain (i16->i32) is a different operand.
        cast_defs[8] = Some((Operand::Value(x), IrType::I16, IrType::I32));
        assert_ne!(
            follow_casts(Value(4), &cast_defs),
            follow_casts(Value(8), &cast_defs)
        );
    }

    // ------------------------------------------------------------------
    // Domain algebra (plan_range): the audit's live miscompile classes.
    // ------------------------------------------------------------------

    fn plan_for(ty: IrType, lo: i64, hi: i64) -> Option<RangePlan> {
        let range = Range {
            lo,
            hi,
            value: Value(0),
            ty,
        };
        let no_casts: Vec<Option<(Operand, IrType, IrType)>> = vec![None; 8];
        plan_range(&range, &no_casts)
    }

    /// `x >= 250 && x <= 260` on an unsigned char: the bounds exceed the
    /// source domain, so the compare must NOT narrow to the byte domain
    /// (a narrowed `(u8)(x-250) <= 10` also accepts x = 0..4). The plan
    /// still folds at the promoted width, where the identity is exact.
    #[test]
    fn u8_bounds_outside_source_domain_stay_wide() {
        // ty I32 (promoted), value = cast U8->I32 at id 1.
        let range = Range {
            lo: 250,
            hi: 260,
            value: Value(1),
            ty: IrType::I32,
        };
        let mut cast_defs: Vec<Option<(Operand, IrType, IrType)>> = vec![None; 4];
        cast_defs[1] = Some((Operand::Value(Value(0)), IrType::U8, IrType::I32));
        let plan = plan_range(&range, &cast_defs).expect("wide fold is legal");
        match plan {
            RangePlan::Test { cmp_ty, .. } => {
                assert_eq!(cmp_ty, IrType::I32, "must not narrow to U8");
            }
            RangePlan::FullDomain => panic!("[250, 260] is not the U8 domain"),
        }

        // The in-domain range [250, 255] still narrows.
        let range = Range {
            lo: 250,
            hi: 255,
            value: Value(1),
            ty: IrType::I32,
        };
        let plan = plan_range(&range, &cast_defs).expect("in-domain fold");
        match plan {
            RangePlan::Test { cmp_ty, .. } => {
                assert_eq!(cmp_ty, IrType::U8, "in-domain bounds narrow");
            }
            RangePlan::FullDomain => panic!("[250, 255] is not the U8 domain"),
        }
    }

    /// A U64 range that wraps ([u64max-9, 10]) is EMPTY, not a 21-value
    /// window: the domain-order check must reject it. (Reproduced live as
    /// 15 wrong values before the i128 normalization.)
    #[test]
    fn u64_wrapped_empty_range_rejects() {
        assert!(plan_for(IrType::U64, -9, 10).is_none());
        // And the same class one width down.
        assert!(plan_for(IrType::U32, -10, 10).is_none());
        assert!(plan_for(IrType::U8, -10, 10).is_none());
    }

    /// U64 upper-half ranges are VALID and must keep folding: the raw bound
    /// `lo` is "negative" as i64 but the domain order says lo <= hi.
    #[test]
    fn u64_upper_half_range_folds() {
        let plan = plan_for(IrType::U64, -100, -91).expect("upper-half range folds");
        match plan {
            RangePlan::Test {
                lo_const,
                span_const,
                ..
            } => {
                // Bias keeps its bit pattern; span is the domain difference 9.
                assert!(matches!(lo_const, IrConst::I64(-100)));
                assert!(matches!(span_const, IrConst::I64(9)));
            }
            RangePlan::FullDomain => panic!("[u64max-99, u64max-90] is not the domain"),
        }
    }

    /// I64 full-range bounds cannot overflow the span computation (i128).
    /// The FULL domain now folds to the constant form (it needs no span at
    /// all); every other extreme range still rejects on the cap — no panic
    /// in checked builds, no wrapped always-true span in release.
    #[test]
    fn i64_extreme_bounds_fold_or_reject_without_panic() {
        assert!(matches!(
            plan_for(IrType::I64, i64::MIN, i64::MAX),
            Some(RangePlan::FullDomain)
        ));
        assert!(plan_for(IrType::I64, i64::MIN, i64::MAX - 1).is_none());
        // A representable-span I64 range still folds.
        assert!(plan_for(IrType::I64, i64::MIN, i64::MIN + 5).is_some());
    }

    /// Every full domain at every width classifies: signed [INT_MIN,
    /// INT_MAX], unsigned [0, MAX], and a narrowed byte domain covering its
    /// source completely. The inside form is constant TRUE; the OUTSIDE
    /// form is the emission paths' constant FALSE.
    #[test]
    fn full_domain_ranges_classify() {
        assert!(matches!(
            plan_for(IrType::I32, i32::MIN as i64, i32::MAX as i64),
            Some(RangePlan::FullDomain)
        ));
        assert!(matches!(
            plan_for(IrType::U32, 0, u32::MAX as i64),
            Some(RangePlan::FullDomain)
        ));
        assert!(matches!(
            plan_for(IrType::U8, 0, 255),
            Some(RangePlan::FullDomain)
        ));
        assert!(matches!(
            plan_for(IrType::I8, -128, 127),
            Some(RangePlan::FullDomain)
        ));
        // One-off bounds are NOT the domain.
        assert!(!matches!(
            plan_for(IrType::I32, i32::MIN as i64, i32::MAX as i64 - 1),
            Some(RangePlan::FullDomain)
        ));
        assert!(!matches!(
            plan_for(IrType::U32, 1, u32::MAX as i64),
            Some(RangePlan::FullDomain)
        ));
        // The NARROWED domain: `(unsigned char)x >= 0 && (unsigned char)x
        // <= 255` compares at I32 but the u8 operand covers its own domain.
        let range = Range {
            lo: 0,
            hi: 255,
            value: Value(1),
            ty: IrType::I32,
        };
        let mut cast_defs: Vec<Option<(Operand, IrType, IrType)>> = vec![None; 4];
        cast_defs[1] = Some((Operand::Value(Value(0)), IrType::U8, IrType::I32));
        assert!(matches!(
            plan_range(&range, &cast_defs),
            Some(RangePlan::FullDomain)
        ));
        // Same shape at the byte's signed spelling.
        let range = Range {
            lo: -128,
            hi: 127,
            value: Value(1),
            ty: IrType::I32,
        };
        let mut cast_defs: Vec<Option<(Operand, IrType, IrType)>> = vec![None; 4];
        cast_defs[1] = Some((Operand::Value(Value(0)), IrType::I8, IrType::I32));
        assert!(matches!(
            plan_range(&range, &cast_defs),
            Some(RangePlan::FullDomain)
        ));
        // A narrowing cast whose bounds do NOT cover the source domain
        // stays a Test plan.
        let range = Range {
            lo: 0,
            hi: 254,
            value: Value(1),
            ty: IrType::I32,
        };
        let mut cast_defs: Vec<Option<(Operand, IrType, IrType)>> = vec![None; 4];
        cast_defs[1] = Some((Operand::Value(Value(0)), IrType::U8, IrType::I32));
        assert!(matches!(
            plan_range(&range, &cast_defs),
            Some(RangePlan::Test { .. })
        ));
    }

    /// 128-bit comparisons are rejected explicitly — no silent guessing.
    #[test]
    fn i128_domains_reject_explicitly() {
        assert!(plan_for(IrType::I128, 0, 10).is_none());
        assert!(plan_for(IrType::U128, 0, 10).is_none());
    }

    /// Reversed signed ranges are empty and reject (historical behaviour).
    #[test]
    fn signed_reversed_range_rejects() {
        assert!(plan_for(IrType::I32, 122, 97).is_none());
    }

    // ------------------------------------------------------------------
    // CFG surgery: direct IR tests for the branch-chain and phi-diamond
    // folds (the audit's demand for transformation-level coverage).
    // ------------------------------------------------------------------

    /// The canonical `&&` branch chain folds: one Sub + one Cmp in Bcond,
    /// Bcheck gone, Bexit phi arms collapsed.
    #[test]
    fn branch_chain_and_form_folds() {
        let mut f = IrFunction::new("andchain".to_string(), IrType::I32, vec![], false);
        f.blocks = vec![
            block(
                0,
                vec![Instruction::Cmp {
                    dest: Value(1),
                    op: IrCmpOp::Sge,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(10)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(3),
                },
            ),
            block(
                1,
                vec![Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Sle,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(20)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(
                2,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I32(1)))),
            ),
            block(
                3,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            ),
        ];
        f.next_value_id = 3;
        let n = run_function(&mut f);
        assert_verified(&f, "range_fold:andchain");
        assert_eq!(n, 1, "one chain folds");
        assert_eq!(f.blocks.len(), 3, "Bcheck removed");
        // Bcond now ends in CondBranch on the fused compare (a fresh id).
        let Terminator::CondBranch {
            cond: Operand::Value(fused),
            true_label,
            false_label,
        } = &f.blocks[0].terminator
        else {
            panic!("Bcond must stay a conditional branch");
        };
        assert_eq!(true_label.0, 2);
        assert_eq!(false_label.0, 3);
        // The fused test is Cmp(Ule, Sub(x, 10), 10) — the terminator's
        // condition is the CMP's result, whose lhs is the fresh Sub.
        let cmp = f.blocks[0].instructions.iter().find_map(|i| match i {
            Instruction::Cmp {
                dest,
                op: IrCmpOp::Ule,
                lhs: Operand::Value(sub_src),
                rhs: Operand::Const(IrConst::I32(10)),
                ..
            } if dest == fused => Some(*sub_src),
            _ => None,
        });
        let sub_src = cmp.expect("fused compare is Ule(Sub(x,10), 10)");
        let sub = f.blocks[0].instructions.iter().find_map(|i| match i {
            Instruction::BinOp {
                dest,
                op: IrBinOp::Sub,
                lhs: Operand::Value(Value(0)),
                rhs: Operand::Const(IrConst::I32(10)),
                ..
            } if *dest == sub_src => Some(()),
            _ => None,
        });
        assert!(sub.is_some(), "the compare reads the fresh Sub of x");
    }

    /// Bexit phis with DIFFERING Bcond/Bcheck arms must reject the fold:
    /// both failure paths survive through the fused branch, and the
    /// surviving Bcond arm would misselect for the old-Bcheck paths.
    /// (Unreachable through today's frontend — Bcheck purity blocks any
    /// between-tests definition — but the fold must be sound for every
    /// valid IR, including what future passes produce.)
    #[test]
    fn branch_chain_rejects_differing_exit_phi_arms() {
        let mut f = IrFunction::new("exitphi".to_string(), IrType::I32, vec![], false);
        // Bexit carries a phi selecting 4 on the Bcond edge and 13 on the
        // Bcheck edge — the below/above distinction.
        f.blocks = vec![
            block(
                0,
                vec![Instruction::Cmp {
                    dest: Value(1),
                    op: IrCmpOp::Sge,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(10)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(3),
                },
            ),
            block(
                1,
                vec![Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Sle,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(20)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(
                2,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I32(1)))),
            ),
            block(
                3,
                vec![Instruction::Phi {
                    dest: Value(5),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Const(IrConst::I32(4)), BlockId(0)),
                        (Operand::Const(IrConst::I32(13)), BlockId(1)),
                    ],
                }],
                Terminator::Return(Some(Operand::Value(Value(5)))),
            ),
        ];
        f.next_value_id = 6;
        let n = run_function(&mut f);
        assert_verified(&f, "range_fold:exitphi-reject");
        assert_eq!(n, 0, "differing exit-phi arms reject the fold");
        assert_eq!(f.blocks.len(), 4, "nothing removed");
        // Equal arms, by contrast, fold and collapse the duplicate.
        let mut g = IrFunction::new("exitphi2".to_string(), IrType::I32, vec![], false);
        g.blocks = f.blocks.clone();
        if let Instruction::Phi { incoming, .. } = &mut g.blocks[3].instructions[0] {
            incoming[1].0 = Operand::Const(IrConst::I32(4));
        }
        g.next_value_id = 6;
        let m = run_function(&mut g);
        assert_verified(&g, "range_fold:exitphi-fold");
        assert_eq!(m, 1, "equal exit-phi arms fold");
        assert_eq!(g.blocks.len(), 3);
        let phi_arms = g.blocks[2].instructions.iter().find_map(|i| match i {
            Instruction::Phi { incoming, .. } => Some(incoming.len()),
            _ => None,
        });
        assert_eq!(
            phi_arms,
            Some(1),
            "the Bcheck arm is dropped, one arm stays"
        );
    }

    /// A check block also targeted by a computed goto (IndirectBranch) is
    /// NOT single-predecessor: the canonical CFG must see the edge and the
    /// fold must reject (deleting Bcheck would dangle the goto target).
    #[test]
    fn branch_chain_rejects_indirect_branch_predecessor() {
        let mut f = IrFunction::new("gototarget".to_string(), IrType::I32, vec![], false);
        f.blocks = vec![
            block(
                0,
                vec![
                    Instruction::Cmp {
                        dest: Value(1),
                        op: IrCmpOp::Sge,
                        lhs: Operand::Value(Value(0)),
                        rhs: Operand::Const(IrConst::I32(10)),
                        ty: IrType::I32,
                    },
                    // Address of Bcheck taken and jumped through indirectly.
                    Instruction::LabelAddr {
                        dest: Value(6),
                        label: BlockId(1),
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(3),
                },
            ),
            block(
                1,
                vec![Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Sle,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(20)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(
                2,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I32(1)))),
            ),
            block(
                3,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            ),
            // A separate block reaching Bcheck through a computed goto.
            block(
                4,
                vec![],
                Terminator::IndirectBranch {
                    target: Operand::Value(Value(6)),
                    possible_targets: vec![BlockId(1)],
                },
            ),
        ];
        f.next_value_id = 7;
        let n = run_function(&mut f);
        assert_verified(&f, "range_fold:indirectbr-reject");
        assert_eq!(n, 0, "indirect-branch predecessor must reject the fold");
        assert_eq!(f.blocks.len(), 5, "nothing removed");
    }

    /// A check block also targeted by asm-goto (InlineAsm goto_labels) is
    /// not single-predecessor either: the canonical CFG counts the implicit
    /// edge.
    #[test]
    fn branch_chain_rejects_asm_goto_predecessor() {
        let mut f = IrFunction::new("asmgototarget".to_string(), IrType::I32, vec![], false);
        f.blocks = vec![
            block(
                0,
                vec![
                    Instruction::Cmp {
                        dest: Value(1),
                        op: IrCmpOp::Sge,
                        lhs: Operand::Value(Value(0)),
                        rhs: Operand::Const(IrConst::I32(10)),
                        ty: IrType::I32,
                    },
                    // asm with a goto label targeting Bcheck (label 1).
                    Instruction::InlineAsm {
                        template: "nop".to_string(),
                        outputs: vec![],
                        inputs: vec![],
                        clobbers: vec![],
                        operand_types: vec![],
                        goto_labels: vec![("Lcheck".to_string(), BlockId(1))],
                        input_symbols: vec![],
                        seg_overrides: vec![],
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(3),
                },
            ),
            block(
                1,
                vec![Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Sle,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(20)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(
                2,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I32(1)))),
            ),
            block(
                3,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            ),
        ];
        f.next_value_id = 3;
        let n = run_function(&mut f);
        assert_verified(&f, "range_fold:asmgoto-reject");
        assert_eq!(n, 0, "asm-goto predecessor must reject the fold");
        assert_eq!(f.blocks.len(), 4, "nothing removed");
    }

    /// Nested chains: the inner fold must not leave stale structure for the
    /// outer candidate — analyses rebuild between folds, and every label
    /// stays reachable.
    #[test]
    fn nested_chains_leave_no_dangling_labels() {
        // if (x >= 10 && x <= 20) { if (x >= 12 && x <= 18) return 1; return 2; }
        // return 3;
        let mut f = IrFunction::new("nested".to_string(), IrType::I32, vec![], false);
        let cmp = |dest: u32, op: IrCmpOp, k: i32| Instruction::Cmp {
            dest: Value(dest),
            op,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Const(IrConst::I32(k)),
            ty: IrType::I32,
        };
        // if (x >= 10 && x <= 20) { if (x >= 12 && x <= 18) return 1; return 2; }
        // return 3;
        f.blocks = vec![
            // 0: outer Bcond: x >= 10 → check, else exit(6)
            block(
                0,
                vec![cmp(1, IrCmpOp::Sge, 10)],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(6),
                },
            ),
            // 1: outer Bcheck (PURE: one Cmp): x <= 20 → body(2), else exit(6)
            block(
                1,
                vec![cmp(2, IrCmpOp::Sle, 20)],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(2),
                    false_label: BlockId(6),
                },
            ),
            // 2: outer body = inner Bcond: x >= 12 → check, else inner-exit(4)
            block(
                2,
                vec![cmp(3, IrCmpOp::Sge, 12)],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(3)),
                    true_label: BlockId(3),
                    false_label: BlockId(4),
                },
            ),
            // 3: inner Bcheck (PURE): x <= 18 → inner-body(5), else exit(4)
            block(
                3,
                vec![cmp(4, IrCmpOp::Sle, 18)],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(4)),
                    true_label: BlockId(5),
                    false_label: BlockId(4),
                },
            ),
            block(
                4,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I32(2)))),
            ),
            block(
                5,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I32(1)))),
            ),
            block(
                6,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I32(3)))),
            ),
        ];
        f.next_value_id = 5;
        let n = run_function(&mut f);
        assert_verified(&f, "range_fold:nested");
        assert_eq!(n, 2, "both chains fold (outer may see the inner first)");
        // Every block's terminator targets must still exist.
        let live: FxHashSet<u32> = f.blocks.iter().map(|b| b.label.0).collect();
        for b in &f.blocks {
            let mut targets = Vec::new();
            match &b.terminator {
                Terminator::Branch(l) => targets.push(l.0),
                Terminator::CondBranch {
                    true_label,
                    false_label,
                    ..
                } => {
                    targets.push(true_label.0);
                    targets.push(false_label.0);
                }
                _ => {}
            }
            for t in targets {
                assert!(live.contains(&t), "dangling target .LBB{}", t);
            }
        }
    }

    /// The phi-diamond fold must reject a merge whose OTHER leading phi
    /// carries differing Bcond/Bcheck arms — collapsing the two edges would
    /// misselect for one path.
    #[test]
    fn phi_diamond_rejects_differing_sibling_phi_arms() {
        let mut f = IrFunction::new("diamond".to_string(), IrType::I32, vec![], false);
        // Bmerge: %p = Phi(0, Bcond; w, Bcheck)  [the boolean]
        //         %q = Phi(4, Bcond; 13, Bcheck) [a sibling with differing arms]
        f.blocks = vec![
            block(
                0,
                vec![Instruction::Cmp {
                    dest: Value(1),
                    op: IrCmpOp::Sge,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(10)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            block(
                1,
                vec![
                    Instruction::Cmp {
                        dest: Value(2),
                        op: IrCmpOp::Sle,
                        lhs: Operand::Value(Value(0)),
                        rhs: Operand::Const(IrConst::I32(20)),
                        ty: IrType::I32,
                    },
                    Instruction::Cast {
                        dest: Value(3),
                        src: Operand::Value(Value(2)),
                        from_ty: IrType::I8,
                        to_ty: IrType::I32,
                    },
                ],
                Terminator::Branch(BlockId(2)),
            ),
            block(
                2,
                vec![
                    Instruction::Phi {
                        dest: Value(4),
                        ty: IrType::I32,
                        incoming: vec![
                            (Operand::Const(IrConst::I32(0)), BlockId(0)),
                            (Operand::Value(Value(3)), BlockId(1)),
                        ],
                    },
                    Instruction::Phi {
                        dest: Value(5),
                        ty: IrType::I32,
                        incoming: vec![
                            (Operand::Const(IrConst::I32(4)), BlockId(0)),
                            (Operand::Const(IrConst::I32(13)), BlockId(1)),
                        ],
                    },
                ],
                Terminator::Return(Some(Operand::Value(Value(4)))),
            ),
        ];
        f.next_value_id = 6;
        let n = run_function(&mut f);
        assert_verified(&f, "range_fold:sibling-reject");
        assert_eq!(n, 0, "differing sibling phi arms reject the fold");
        assert_eq!(f.blocks.len(), 3, "nothing removed");

        // Equal sibling arms fold, and the sibling's Bcheck arm collapses.
        let mut g = IrFunction::new("diamond2".to_string(), IrType::I32, vec![], false);
        g.blocks = f.blocks.clone();
        if let Instruction::Phi { incoming, .. } = &mut g.blocks[2].instructions[1] {
            incoming[1].0 = Operand::Const(IrConst::I32(4));
        }
        g.next_value_id = 6;
        let m = run_function(&mut g);
        assert_verified(&g, "range_fold:sibling-fold");
        assert_eq!(m, 1, "equal sibling arms fold");
        assert_eq!(g.blocks.len(), 2, "Bcheck removed");
        let sibling_arms = g.blocks[1].instructions.iter().find_map(|i| match i {
            Instruction::Phi { incoming, .. } => Some(incoming.len()),
            _ => None,
        });
        assert_eq!(sibling_arms, Some(1), "sibling phi collapses to one arm");
    }

    /// A phi folded by the diamond must not leave a STALE definition map
    /// entry that a later fold consumes: the boolean phi's replacement
    /// value is a fresh Cmp, and a subsequent Select feeding on the OLD
    /// phi value must be rewritten (not left referencing a deleted value).
    #[test]
    fn phi_diamond_rewrites_select_consumers_of_the_boolean() {
        // Bmerge's boolean phi %p feeds a Select %r = %p ? 7 : 9 in a later
        // block. After the diamond fold, the Select's condition must be the
        // replacement value (or the Select itself folds); it must NOT still
        // reference the deleted phi id.
        let mut f = IrFunction::new("diamondsel".to_string(), IrType::I32, vec![], false);
        f.blocks = vec![
            block(
                0,
                vec![Instruction::Cmp {
                    dest: Value(1),
                    op: IrCmpOp::Sge,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(10)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            block(
                1,
                vec![
                    Instruction::Cmp {
                        dest: Value(2),
                        op: IrCmpOp::Sle,
                        lhs: Operand::Value(Value(0)),
                        rhs: Operand::Const(IrConst::I32(20)),
                        ty: IrType::I32,
                    },
                    Instruction::Cast {
                        dest: Value(3),
                        src: Operand::Value(Value(2)),
                        from_ty: IrType::I8,
                        to_ty: IrType::I32,
                    },
                ],
                Terminator::Branch(BlockId(2)),
            ),
            block(
                2,
                vec![Instruction::Phi {
                    dest: Value(4),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Const(IrConst::I32(0)), BlockId(0)),
                        (Operand::Value(Value(3)), BlockId(1)),
                    ],
                }],
                Terminator::Branch(BlockId(3)),
            ),
            block(
                3,
                vec![Instruction::Select {
                    dest: Value(5),
                    cond: Operand::Value(Value(4)),
                    true_val: Operand::Const(IrConst::I32(7)),
                    false_val: Operand::Const(IrConst::I32(9)),
                    ty: IrType::I32,
                }],
                Terminator::Return(Some(Operand::Value(Value(5)))),
            ),
        ];
        f.next_value_id = 6;
        let n = run_function(&mut f);
        assert_verified(&f, "range_fold:diamondsel");
        assert_eq!(n, 1, "the diamond folds");
        // No instruction may reference the deleted phi id 4.
        for b in &f.blocks {
            for inst in &b.instructions {
                inst.for_each_used_value(|id| {
                    assert_ne!(id, 4, "deleted phi still referenced");
                });
            }
        }
    }

    /// Path facts attach AFTER block removals: a fold that deletes an early
    /// block must not shift the `x > 0` fact onto an unrelated block. The
    /// victim block's `Ult` test on an UNPROVEN value must survive.
    #[test]
    fn path_facts_survive_block_removal_index_shift() {
        // Blocks: 0 (chain Bcond), 1 (chain Bcheck, removed by the fold),
        // 2 (chain Bbody), 3 (the x<=0 branch attaching the fact for x at
        // block 4), 4 (the victim: Ult on y-1, NOT provable), 5 (fail).
        let mut f = IrFunction::new("pathshift".to_string(), IrType::I32, vec![], false);
        let cmp =
            |dest: u32, op: IrCmpOp, lhs: Operand, rhs: Operand, ty: IrType| Instruction::Cmp {
                dest: Value(dest),
                op,
                lhs,
                rhs,
                ty,
            };
        f.blocks = vec![
            // The chain: B0 (c-range Bcond) → B1 (pure check) → body B2 /
            // shared exit B6. Removing B1 shifts blocks 2..6 down one index.
            block(
                0,
                vec![cmp(
                    1,
                    IrCmpOp::Sge,
                    Operand::Value(Value(2)),
                    Operand::Const(IrConst::I32(10)),
                    IrType::I32,
                )],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(6),
                },
            ),
            block(
                1,
                vec![cmp(
                    3,
                    IrCmpOp::Sle,
                    Operand::Value(Value(2)),
                    Operand::Const(IrConst::I32(20)),
                    IrType::I32,
                )],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(3)),
                    true_label: BlockId(2),
                    false_label: BlockId(6),
                },
            ),
            block(2, vec![], Terminator::Branch(BlockId(3))),
            // 3: the fact-attaching branch on x (value 0).
            block(
                3,
                vec![cmp(
                    4,
                    IrCmpOp::Sle,
                    Operand::Value(Value(0)),
                    Operand::Const(IrConst::I32(0)),
                    IrType::I32,
                )],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(4)),
                    true_label: BlockId(5),
                    false_label: BlockId(4),
                },
            ),
            // 4: the victim — (u32)(y - 1) < UINT_MAX with y NOT proven > 0.
            block(
                4,
                vec![
                    Instruction::BinOp {
                        dest: Value(6),
                        op: IrBinOp::Sub,
                        lhs: Operand::Value(Value(9)),
                        rhs: Operand::Const(IrConst::I32(1)),
                        ty: IrType::I32,
                    },
                    Instruction::Cast {
                        dest: Value(7),
                        src: Operand::Value(Value(6)),
                        from_ty: IrType::I32,
                        to_ty: IrType::U32,
                    },
                    cmp(
                        8,
                        IrCmpOp::Ult,
                        Operand::Value(Value(7)),
                        Operand::Const(IrConst::I32(-1)),
                        IrType::U32,
                    ),
                ],
                Terminator::Return(Some(Operand::Value(Value(8)))),
            ),
            block(
                5,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            ),
            block(
                6,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            ),
        ];
        f.next_value_id = 10;
        let n = run_function(&mut f);
        assert_verified(&f, "range_fold:pathshift");
        // The chain fold (block 1 removed) fires; the victim's Ult must NOT
        // be folded to constant 1 (y is unproven — the fact was for x, and
        // the index shift must not move it).
        assert_eq!(n, 1, "exactly the chain folds");
        let victim = f
            .blocks
            .iter()
            .find(|b| b.label.0 == 4)
            .expect("victim block");
        let still_ult = victim.instructions.iter().any(|i| {
            matches!(
                i,
                Instruction::Cmp {
                    op: IrCmpOp::Ult,
                    ..
                }
            )
        });
        assert!(still_ult, "the unproven Ult must survive (no stale fact)");
        // And the positive control: the SAME Ult on x (value 0, proven > 0
        // on the false edge of x <= 0) DOES fold to 1.
        let mut g = IrFunction::new("pathshift2".to_string(), IrType::I32, vec![], false);
        g.blocks = f.blocks.clone();
        // Re-point the victim's Sub at x (value 0).
        for b in &mut g.blocks {
            for inst in &mut b.instructions {
                if let Instruction::BinOp {
                    dest: Value(6),
                    lhs,
                    ..
                } = inst
                {
                    *lhs = Operand::Value(Value(0));
                }
            }
        }
        g.next_value_id = 10;
        let m = run_function(&mut g);
        assert!(m >= 1);
        let victim = g
            .blocks
            .iter()
            .find(|b| b.label.0 == 4)
            .expect("victim block");
        let folded = victim.instructions.iter().any(|i| {
            matches!(
                i,
                Instruction::Copy {
                    src: Operand::Const(IrConst::I8(1)),
                    ..
                }
            )
        });
        assert!(folded, "the PROVEN Ult on x must fold to constant 1");
    }

    /// End-to-end domain check through the public entry: the U8 [250, 260]
    /// Select form must not miscompile (the value-context narrowing bug).
    #[test]
    fn select_form_u8_domain_violation_does_not_fold_narrow() {
        // Select(cond = Cmp(Sge, cast(u8->i32 x), 250) ? Cast(Cmp(Sle, ..., 260)) : 0)
        let mut f = IrFunction::new("u8dom".to_string(), IrType::I32, vec![], false);
        f.blocks = vec![block(
            0,
            vec![
                Instruction::Cast {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                    from_ty: IrType::U8,
                    to_ty: IrType::I32,
                },
                Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Sge,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(250)),
                    ty: IrType::I32,
                },
                Instruction::Cmp {
                    dest: Value(3),
                    op: IrCmpOp::Sle,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I32(260)),
                    ty: IrType::I32,
                },
                Instruction::Select {
                    dest: Value(4),
                    cond: Operand::Value(Value(2)),
                    true_val: Operand::Value(Value(3)),
                    false_val: Operand::Const(IrConst::I32(0)),
                    ty: IrType::I32,
                },
            ],
            Terminator::Return(Some(Operand::Value(Value(4)))),
        )];
        f.next_value_id = 5;
        let n = run_function(&mut f);
        assert_eq!(n, 1, "the Select folds at the promoted width");
        let narrow = f.blocks[0].instructions.iter().any(|i| {
            matches!(i,
            Instruction::Cmp { ty, .. } if *ty == IrType::U8)
        });
        assert!(!narrow, "the compare must stay at the I32 width");
        let wide = f.blocks[0].instructions.iter().any(|i| {
            matches!(i,
            Instruction::Cmp { op: IrCmpOp::Ule, ty, .. } if *ty == IrType::I32)
        });
        assert!(wide, "the unsigned-bias compare runs at I32");
    }

    /// Full-domain value forms fold to constants. The bitwise And of the
    /// two boolean-widened compares over the SIGNED full domain becomes
    /// `Copy 1`; the Or complement (exclusive bounds: `x < lo || x > hi`)
    /// becomes `Copy 0`.
    #[test]
    fn full_domain_bool_op_folds_to_constant() {
        let build = |op: IrBinOp| {
            // Inside spelling for And, outside spelling for Or.
            let (op1, op2) = if matches!(op, IrBinOp::And) {
                (IrCmpOp::Sge, IrCmpOp::Sle)
            } else {
                (IrCmpOp::Slt, IrCmpOp::Sgt)
            };
            let mut f = IrFunction::new("fullbool".to_string(), IrType::I32, vec![], false);
            f.blocks = vec![block(
                0,
                vec![
                    Instruction::Cmp {
                        dest: Value(1),
                        op: op1,
                        lhs: Operand::Value(Value(0)),
                        rhs: Operand::Const(IrConst::I32(i32::MIN)),
                        ty: IrType::I32,
                    },
                    Instruction::Cmp {
                        dest: Value(2),
                        op: op2,
                        lhs: Operand::Value(Value(0)),
                        rhs: Operand::Const(IrConst::I32(i32::MAX)),
                        ty: IrType::I32,
                    },
                    Instruction::Cast {
                        dest: Value(3),
                        src: Operand::Value(Value(1)),
                        from_ty: IrType::I8,
                        to_ty: IrType::I32,
                    },
                    Instruction::Cast {
                        dest: Value(4),
                        src: Operand::Value(Value(2)),
                        from_ty: IrType::I8,
                        to_ty: IrType::I32,
                    },
                    Instruction::BinOp {
                        dest: Value(5),
                        op,
                        lhs: Operand::Value(Value(3)),
                        rhs: Operand::Value(Value(4)),
                        ty: IrType::I32,
                    },
                ],
                Terminator::Return(Some(Operand::Value(Value(5)))),
            )];
            f.next_value_id = 6;
            f
        };
        let mut f = build(IrBinOp::And);
        assert_eq!(run_function(&mut f), 1, "the And folds");
        assert_verified(&f, "range_fold:full-and");
        match &f.blocks[0].instructions[4] {
            Instruction::Copy {
                dest: Value(5),
                src: Operand::Const(IrConst::I32(1)),
            } => {}
            other => panic!("And must become Copy 1, got {other:?}"),
        }
        let mut g = build(IrBinOp::Or);
        assert_eq!(run_function(&mut g), 1, "the Or folds");
        assert_verified(&g, "range_fold:full-or");
        match &g.blocks[0].instructions[4] {
            Instruction::Copy {
                dest: Value(5),
                src: Operand::Const(IrConst::I32(0)),
            } => {}
            other => panic!("Or must become Copy 0, got {other:?}"),
        }
    }

    /// Full-domain phi diamond: the boolean phi becomes a Copy of the
    /// constant IN the merge block (uses keep their reference), the check
    /// block dies, and Bcond branches unconditionally to the merge.
    #[test]
    fn full_domain_phi_diamond_folds_to_constant() {
        let mut f = IrFunction::new("fulldiamond".to_string(), IrType::I32, vec![], false);
        f.blocks = vec![
            block(
                0,
                vec![Instruction::Cmp {
                    dest: Value(1),
                    op: IrCmpOp::Sge,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(i32::MIN)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            block(
                1,
                vec![
                    Instruction::Cmp {
                        dest: Value(2),
                        op: IrCmpOp::Sle,
                        lhs: Operand::Value(Value(0)),
                        rhs: Operand::Const(IrConst::I32(i32::MAX)),
                        ty: IrType::I32,
                    },
                    Instruction::Cast {
                        dest: Value(3),
                        src: Operand::Value(Value(2)),
                        from_ty: IrType::I8,
                        to_ty: IrType::I32,
                    },
                ],
                Terminator::Branch(BlockId(2)),
            ),
            block(
                2,
                vec![Instruction::Phi {
                    dest: Value(4),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Const(IrConst::I32(0)), BlockId(0)),
                        (Operand::Value(Value(3)), BlockId(1)),
                    ],
                }],
                Terminator::Return(Some(Operand::Value(Value(4)))),
            ),
        ];
        f.next_value_id = 5;
        assert_eq!(run_function(&mut f), 1, "the diamond folds");
        assert_verified(&f, "range_fold:full-diamond");
        assert_eq!(f.blocks.len(), 2, "Bcheck removed");
        assert!(matches!(
            &f.blocks[0].terminator,
            Terminator::Branch(BlockId(2))
        ));
        // The phi's id survives as a Copy of the constant.
        match &f.blocks[1].instructions[0] {
            Instruction::Copy {
                dest: Value(4),
                src: Operand::Const(IrConst::I32(1)),
            } => {}
            other => panic!("phi must become Copy 1, got {other:?}"),
        }
    }

    /// Full-domain branch chain, inside form: `x >= INT_MIN && x <=
    /// INT_MAX` guarding a body — the guard is constant TRUE, so Bcond
    /// branches to the body unconditionally and the never-taken Bexit is
    /// deleted (its only predecessors were the chain, and nothing used its
    /// definitions).
    #[test]
    fn full_domain_branch_chain_inside_folds_to_unconditional() {
        let mut f = IrFunction::new("fullchain".to_string(), IrType::I32, vec![], false);
        f.blocks = vec![
            block(
                0,
                vec![Instruction::Cmp {
                    dest: Value(1),
                    op: IrCmpOp::Sge,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(i32::MIN)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(3),
                },
            ),
            block(
                1,
                vec![Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Sle,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(i32::MAX)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(
                2,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I32(1)))),
            ),
            block(
                3,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            ),
        ];
        f.next_value_id = 3;
        assert_eq!(run_function(&mut f), 1, "the chain folds");
        assert_verified(&f, "range_fold:full-chain-and");
        assert_eq!(f.blocks.len(), 2, "Bcheck and Bexit removed");
        assert!(matches!(
            &f.blocks[0].terminator,
            Terminator::Branch(BlockId(2))
        ));
    }

    /// Full-domain branch chain, outside form: `x < INT_MIN || x >
    /// INT_MAX` guarding the exit — the OUTSIDE test is constant FALSE, so
    /// control always takes the inside continuation: Bcond branches to
    /// Bbody unconditionally and the never-reached Bexit is deleted.
    /// (Bexit is the outside test's TRUE continuation: c1 true → Bexit,
    /// c2 true → Bexit. A constant-FALSE test must select its FALSE
    /// continuation — Bbody — NOT the exit.)
    #[test]
    fn full_domain_branch_chain_outside_folds_to_unconditional() {
        let mut f = IrFunction::new("fullchainor".to_string(), IrType::I32, vec![], false);
        f.blocks = vec![
            block(
                0,
                vec![Instruction::Cmp {
                    dest: Value(1),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(i32::MIN)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(3),
                    false_label: BlockId(1),
                },
            ),
            block(
                1,
                vec![Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Sgt,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(i32::MAX)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(3),
                    false_label: BlockId(2),
                },
            ),
            block(
                2,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I32(1)))),
            ),
            block(
                3,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            ),
        ];
        f.next_value_id = 3;
        assert_eq!(run_function(&mut f), 1, "the chain folds");
        assert_verified(&f, "range_fold:full-chain-or");
        assert_eq!(f.blocks.len(), 2, "Bcheck and Bexit removed");
        assert!(matches!(
            &f.blocks[0].terminator,
            Terminator::Branch(BlockId(2))
        ));
    }

    /// Full-domain `||` chain with a distinguishable phi on the LIVE target:
    /// Bbody's Bcheck arm must be RETARGETED to the new Bcond edge (the
    /// same value flows — Bcheck is no longer on the path), and Bbody must
    /// keep its other predecessor's arm untouched.
    #[test]
    fn full_domain_or_retargets_bbody_phi_arms() {
        let mut f = IrFunction::new("fullorphi".to_string(), IrType::I32, vec![], false);
        // Block 2 (Bbody) merges a phi from Bcheck (77) and block 4 (13).
        f.blocks = vec![
            block(
                0,
                vec![Instruction::Cmp {
                    dest: Value(1),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(i32::MIN)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(3),
                    false_label: BlockId(1),
                },
            ),
            block(
                1,
                vec![Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Sgt,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(i32::MAX)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(3),
                    false_label: BlockId(2),
                },
            ),
            block(
                2,
                vec![Instruction::Phi {
                    dest: Value(5),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Const(IrConst::I32(77)), BlockId(1)),
                        (Operand::Const(IrConst::I32(13)), BlockId(4)),
                    ],
                }],
                Terminator::Return(Some(Operand::Value(Value(5)))),
            ),
            block(
                3,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            ),
            block(4, vec![], Terminator::Branch(BlockId(2))),
        ];
        f.next_value_id = 6;
        assert_eq!(run_function(&mut f), 1, "the chain folds");
        assert_verified(&f, "range_fold:full-or-phi");
        assert!(matches!(
            &f.blocks[0].terminator,
            Terminator::Branch(BlockId(2))
        ));
        let bbody = f.blocks.iter().find(|b| b.label.0 == 2).unwrap();
        match &bbody.instructions[0] {
            Instruction::Phi { incoming, .. } => {
                // The Bcheck arm became the Bcond arm; block 4's arm is
                // untouched. Exactly one arm per surviving predecessor.
                assert_eq!(incoming.len(), 2, "both arms survive (retargeted + other)");
                assert!(incoming.iter().any(
                    |(v, from)| matches!(v, Operand::Const(c) if c.to_i64() == Some(77))
                        && from.0 == 0
                ));
                assert!(incoming.iter().any(
                    |(v, from)| matches!(v, Operand::Const(c) if c.to_i64() == Some(13))
                        && from.0 == 4
                ));
            }
            other => panic!("phi must survive in Bbody, got {other:?}"),
        }
    }

    /// Full-domain `||` chain whose dead side (Bexit) has an EXTRA
    /// predecessor: Bexit survives with that predecessor's arm only — both
    /// chain arms are dropped, and the extra pred's control flow into it is
    /// untouched.
    #[test]
    fn full_domain_or_keeps_bexit_extra_predecessor() {
        let mut f = IrFunction::new("fullorxpred".to_string(), IrType::I32, vec![], false);
        // Block 3 (Bexit) also receives an edge from block 4.
        f.blocks = vec![
            block(
                0,
                vec![Instruction::Cmp {
                    dest: Value(1),
                    op: IrCmpOp::Slt,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(i32::MIN)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(3),
                    false_label: BlockId(1),
                },
            ),
            block(
                1,
                vec![Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Sgt,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(i32::MAX)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(3),
                    false_label: BlockId(2),
                },
            ),
            block(
                2,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I32(1)))),
            ),
            block(
                3,
                vec![Instruction::Phi {
                    dest: Value(5),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Const(IrConst::I32(4)), BlockId(0)),
                        (Operand::Const(IrConst::I32(13)), BlockId(1)),
                        (Operand::Const(IrConst::I32(42)), BlockId(4)),
                    ],
                }],
                Terminator::Return(Some(Operand::Value(Value(5)))),
            ),
            block(4, vec![], Terminator::Branch(BlockId(3))),
        ];
        f.next_value_id = 6;
        assert_eq!(run_function(&mut f), 1, "the chain folds");
        assert_verified(&f, "range_fold:full-or-xpred");
        // Bcheck gone; Bexit kept (block 4 still enters it) with exactly
        // the block-4 arm remaining.
        assert_eq!(f.blocks.len(), 4, "Bcheck removed, Bexit kept");
        let bexit = f.blocks.iter().find(|b| b.label.0 == 3).unwrap();
        match &bexit.instructions[0] {
            Instruction::Phi { incoming, .. } => {
                assert_eq!(incoming.len(), 1, "both chain arms dropped");
                assert!(matches!(
                    (&incoming[0].0, incoming[0].1),
                    (Operand::Const(c), BlockId(4)) if c.to_i64() == Some(42)
                ));
            }
            other => panic!("phi must survive in Bexit, got {other:?}"),
        }
        assert!(matches!(
            &f.blocks[0].terminator,
            Terminator::Branch(BlockId(2))
        ));
    }

    /// Full-domain chain whose dead side is NOT deletable (its definitions
    /// are used elsewhere): the fold still applies — the block survives
    /// with both chain arms dropped from its phis.
    #[test]
    fn full_domain_chain_keeps_used_dead_side_with_arms_dropped() {
        let mut f = IrFunction::new("fullkeep".to_string(), IrType::I32, vec![], false);
        // Bexit defines v5, which a LATER block consumes: not droppable.
        f.blocks = vec![
            block(
                0,
                vec![Instruction::Cmp {
                    dest: Value(1),
                    op: IrCmpOp::Sge,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(i32::MIN)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(3),
                },
            ),
            block(
                1,
                vec![Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Sle,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I32(i32::MAX)),
                    ty: IrType::I32,
                }],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(
                2,
                vec![],
                Terminator::Return(Some(Operand::Const(IrConst::I32(1)))),
            ),
            block(
                3,
                vec![Instruction::Phi {
                    dest: Value(5),
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Const(IrConst::I32(4)), BlockId(0)),
                        (Operand::Const(IrConst::I32(13)), BlockId(1)),
                        (Operand::Const(IrConst::I32(42)), BlockId(4)),
                    ],
                }],
                Terminator::Return(Some(Operand::Value(Value(5)))),
            ),
            block(4, vec![], Terminator::Branch(BlockId(3))),
        ];
        f.next_value_id = 6;
        assert_eq!(run_function(&mut f), 1, "the chain folds");
        assert_verified(&f, "range_fold:full-chain-keep");
        // Bcheck gone; Bexit kept (block 4 still enters it) with exactly
        // the block-4 arm remaining.
        assert_eq!(f.blocks.len(), 4, "Bcheck removed, Bexit kept");
        let bexit = f.blocks.iter().find(|b| b.label.0 == 3).unwrap();
        match &bexit.instructions[0] {
            Instruction::Phi { incoming, .. } => {
                assert_eq!(incoming.len(), 1, "both chain arms dropped");
                assert_eq!(incoming[0].1, BlockId(4));
            }
            other => panic!("expected the exit phi, got {other:?}"),
        }
    }
}
