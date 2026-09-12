//! Recognition of common integer bit-manipulation idioms.
//!
//! Source code often spells operations such as population count using portable
//! shifts and masks.  Recognizing the complete data-flow graph here lets every
//! backend select its native instruction without making codegen source-specific.

use crate::common::types::IrType;
use crate::ir::reexports::{
    Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, IrUnaryOp, Operand,
};

fn const_u64(op: Operand) -> Option<u64> {
    match op {
        Operand::Const(IrConst::I8(v)) => Some(v as u8 as u64),
        Operand::Const(IrConst::I16(v)) => Some(v as u16 as u64),
        Operand::Const(IrConst::I32(v)) => Some(v as u32 as u64),
        Operand::Const(IrConst::I64(v)) => Some(v as u64),
        Operand::Const(IrConst::Zero) => Some(0),
        _ => None,
    }
}

fn peel(mut op: Operand, defs: &[Option<Instruction>]) -> Operand {
    // Casts between the IR's integer carrier types are common after inlining.
    // The masks below constrain the recognized computation to 32 bits.
    for _ in 0..32 {
        let Operand::Value(v) = op else { break };
        match defs.get(v.0 as usize).and_then(Option::as_ref) {
            Some(Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            }) if from_ty.is_integer() && to_ty.is_integer() => op = *src,
            Some(Instruction::Copy { src, .. }) => op = *src,
            _ => break,
        }
    }
    op
}

/// Peel through `Copy` chains only.  The generic `peel` also crosses
/// integer casts, which is correct only for matchers whose operation is
/// cast-transparent.  A rotate is not: its two halves must consume the SAME
/// SSA value at the SAME width, and `zext(x)`/`sext(x)`/`trunc(x)` are all
/// different values even though they peel to the same root.  Matching a
/// `(zext(x) << n) | (sext(x) >> (W - n))` as a rotate of `x` silently drops
/// the extension half and miscompiles (observed: `(z << 16) | ((uint32_t)s
/// >> 16)` with `z = (uint32_t)(uint8_t)a`, `s = (int32_t)(int8_t)a` folded
/// to `rol a, 16`, losing the `0xffff` sign half).
fn peel_copies(mut op: Operand, defs: &[Option<Instruction>]) -> Operand {
    for _ in 0..32 {
        let Operand::Value(v) = op else { break };
        match defs.get(v.0 as usize).and_then(Option::as_ref) {
            Some(Instruction::Copy { src, .. }) => op = *src,
            _ => break,
        }
    }
    op
}

fn same(a: Operand, b: Operand, defs: &[Option<Instruction>]) -> bool {
    match (peel(a, defs), peel(b, defs)) {
        (Operand::Value(a), Operand::Value(b)) => a == b,
        (Operand::Const(a), Operand::Const(b)) => a.to_i128() == b.to_i128(),
        _ => false,
    }
}

/// Value identity: both operands must name the SAME SSA value, modulo
/// `Copy` chains.  Unlike `same`, this does NOT cross integer casts --
/// `zext(x)`, `sext(x)` and `trunc(x)` all peel to `x` but are different
/// values, and an idiom whose halves (or whose input and its uses) run
/// through different casts is not the idiom.  Every identity check in the
/// matchers below that guards a VALUE uses this; cast-peeling remains
/// correct only where constants or shift AMOUNTS are extracted (amounts are
/// taken modulo the width, so a cast of a small count is harmless).
fn same_value(a: Operand, b: Operand, defs: &[Option<Instruction>]) -> bool {
    match (peel_copies(a, defs), peel_copies(b, defs)) {
        (Operand::Value(a), Operand::Value(b)) => a == b,
        (Operand::Const(a), Operand::Const(b)) => a.to_i128() == b.to_i128(),
        _ => false,
    }
}

fn binop(
    opnd: Operand,
    wanted: IrBinOp,
    defs: &[Option<Instruction>],
) -> Option<(Operand, Operand, IrType)> {
    let Operand::Value(v) = peel(opnd, defs) else {
        return None;
    };
    match defs.get(v.0 as usize).and_then(Option::as_ref) {
        Some(Instruction::BinOp {
            op, lhs, rhs, ty, ..
        }) if *op == wanted => Some((*lhs, *rhs, *ty)),
        _ => None,
    }
}

fn commutative_const(
    opnd: Operand,
    wanted: IrBinOp,
    constant: u64,
    defs: &[Option<Instruction>],
) -> Option<Operand> {
    let (lhs, rhs, _) = binop(opnd, wanted, defs)?;
    if const_u64(rhs) == Some(constant) {
        Some(lhs)
    } else if const_u64(lhs) == Some(constant) {
        Some(rhs)
    } else {
        None
    }
}

fn shift(
    opnd: Operand,
    wanted: IrBinOp,
    amount: u64,
    defs: &[Option<Instruction>],
) -> Option<Operand> {
    let (lhs, rhs, _) = binop(opnd, wanted, defs)?;
    (const_u64(rhs) == Some(amount)).then_some(lhs)
}

/// One half of a rotate: `value` shifted by `amount` with the shift kind the
/// caller asked for.  Unlike `shift`, the amount is returned as an `Operand`
/// rather than matched against a constant, because the complementary half of a
/// run-time rotate is a live `W - n` subtraction, not a literal.
fn shift_any(
    opnd: Operand,
    wanted: IrBinOp,
    defs: &[Option<Instruction>],
) -> Option<(Operand, Operand, IrType)> {
    let (lhs, rhs, ty) = binop(opnd, wanted, defs)?;
    Some((lhs, rhs, ty))
}

/// Is `opnd` the expression `width - other`?  The complementary half of a
/// variable-count rotate is spelled exactly this way (`ROTL32(v, n)` expands
/// to `(v << n) | (v >> (32 - n))`), and recognizing it lets the rotate keep
/// `n` directly so the subtraction dies with its last use.
fn is_width_minus(opnd: Operand, other: Operand, width: u64, defs: &[Option<Instruction>]) -> bool {
    let Some((lhs, rhs, _)) = binop(opnd, IrBinOp::Sub, defs) else {
        return false;
    };
    const_u64(peel(lhs, defs)) == Some(width) && same(rhs, other, defs)
}

/// Recognize the portable rotate idiom; returns `(value, amount, is_left)`.
///
/// For a width-`W` integer `x` the two source spellings are
///
/// ```c
/// #define ROTL32(x, n) (((x) << (n)) | ((x) >> (32 - (n))))
/// #define ROTR32(x, n) (((x) >> (n)) | ((x) << (32 - (n))))
/// ```
///
/// They appear verbatim in ChaCha20/Salsa20, SHA-1/SHA-256/SHA-512, MD5,
/// BLAKE2, SipHash, RC5, Speck, in the Linux kernel's `rol32`/`ror32`/
/// `rol64`/`ror64`, and in zlib-ng's and zstd's hash/xxh paths.  Left as a
/// shift/or triple each rotate costs three instructions and two dependent ALU
/// results; x86 `rol`/`ror`, AArch64 `ror`/`extr` and RISC-V's funnel shift
/// all do the same work in one (RISC-V in four, but still without the
/// temporary the triple's `or` needs).
///
/// Both `Or` operand orders are matched, and the complementary amount may be a
/// literal the frontend already folded (`>> 16`) or a live `W - n`, so constant
/// and run-time rotation amounts are both recognized.
///
/// Two deliberate restrictions:
///
/// * The right-shift half must be *logical*.  With an arithmetic shift the sign
///   bit is replicated rather than wrapped around, so the expression is not a
///   rotate and folding it would be a miscompile.
/// * Only 32- and 64-bit types are recognized.  C's integer promotions already
///   run a narrower rotate at `int` width, and the x86 hardware count mask is
///   mod 32 for every operand size below 64 bits, so an 8- or 16-bit `rol`
///   would wrap at the wrong width.
///
/// Direction is chosen to match what the reference compilers emit, so oracle
/// diffs stay readable: for a run-time amount the direction whose count is the
/// *direct* value (not the `W - n` subtraction) wins, which also makes that
/// subtraction dead; for two constants the smaller amount wins, reproducing
/// GCC's `ror $7` for `(x >> 7) | (x << 25)` and its `rol $12` for
/// `(x << 12) | (x >> 20)`.
fn match_rotate(
    result: Operand,
    max_bits: u32,
    defs: &[Option<Instruction>],
) -> Option<(Operand, Operand, bool)> {
    let (a, b, ty) = binop(result, IrBinOp::Or, defs)?;
    let width = match ty {
        IrType::I32 | IrType::U32 => 32u64,
        IrType::I64 | IrType::U64 => 64u64,
        // Narrower and wider types are deliberately not recognized; see the
        // function comment for the count-mask and promotion reasons.
        _ => return None,
    };
    // Per-target capability: i686 routes I64/U64 arithmetic through its
    // paired-register path rather than the 32-bit ALU path that owns the
    // native `rol`/`ror` lowering, so it only advertises 32-bit rotates.
    if width > u64::from(max_bits) {
        return None;
    }

    // The two halves in either `Or` order.  Each shift must run at the
    // OR's own width: `binop` peels the OR's operands through casts before
    // matching the shift defs, and a shift found that way (e.g. a u32 shift
    // widened by `(uint64_t)(x << n)` for a u64 OR) computes at a different
    // width than the rotate would, so the amounts cannot be checked against
    // `width` at all.  Only same-width halves are a rotate.
    let halves = [(a, b), (b, a)];
    let mut matched = None;
    for (x, y) in halves {
        if let (Some(shl), Some(lshr)) = (
            shift_any(x, IrBinOp::Shl, defs),
            shift_any(y, IrBinOp::LShr, defs),
        ) {
            matched = Some((shl, lshr));
            break;
        }
    }
    let ((shl_val, shl_amt, shl_ty), (lshr_val, lshr_amt, lshr_ty)) = matched?;
    if shl_ty != ty || lshr_ty != ty {
        return None;
    }
    // The halves must read the SAME SSA value at that width.  Cast-peeling
    // is unsound here (`zext(x)` and `sext(x)` peel to one root but are
    // different values), so only `Copy` chains are transparent; the
    // returned rotate source is the value the shifts actually consume.
    let value = peel_copies(shl_val, defs);
    if value != peel_copies(lshr_val, defs) {
        return None;
    }

    // The halves must be complementary: either both amounts are constants
    // summing to the width, or one is literally `width - the_other`.
    let (amount, is_left) = match (
        const_u64(peel(shl_amt, defs)),
        const_u64(peel(lshr_amt, defs)),
    ) {
        (Some(left), Some(right)) => {
            if left.wrapping_add(right) != width || left == 0 || right == 0 {
                return None;
            }
            if left <= right {
                (shl_amt, true)
            } else {
                (lshr_amt, false)
            }
        }
        // Exactly one amount folded to a constant: the other must still be the
        // explicit `width - amount` subtraction (a frontend that folded one
        // side usually folds both, but an unfolded `32 - 16` does occur after
        // inlining), otherwise the halves are not complementary.
        (Some(_), None) => {
            if !is_width_minus(lshr_amt, shl_amt, width, defs) {
                return None;
            }
            (shl_amt, true)
        }
        (None, Some(_)) => {
            if !is_width_minus(shl_amt, lshr_amt, width, defs) {
                return None;
            }
            (lshr_amt, false)
        }
        (None, None) => {
            // Both amounts dynamic: accept either complement direction, but
            // only one of them needs to hold.
            if is_width_minus(lshr_amt, shl_amt, width, defs) {
                (shl_amt, true)
            } else if is_width_minus(shl_amt, lshr_amt, width, defs) {
                (lshr_amt, false)
            } else {
                return None;
            }
        }
    };

    Some((value, amount, is_left))
}

/// Width in bits of the sub-word integer types, or `None` for wider ones.
fn narrow_width_bits(ty: IrType) -> Option<u64> {
    match ty {
        IrType::I8 | IrType::U8 => Some(8),
        IrType::I16 | IrType::U16 => Some(16),
        _ => None,
    }
}

/// Prove that `opnd`'s bits at positions `[width, 32)` are zero.
///
/// The truncation-aware narrow-rotate pattern needs this twice: the two
/// promoted halves are exact at the narrow width only when the shared source
/// carries nothing above it.  `AShr` on such a value is `LShr` (the sign bit
/// it would replicate is provably zero), which is what makes the
/// C-promotion spelling of the idiom — `(uint16_t)((v << 8) | (v >> 8))`
/// where both shifts run on the *signed* `int` promotion of `v` — fold.
///
/// Only definitions that *construct* the zero-extension are accepted
/// (`peel_copies` first, so `Copy` webs are transparent):
///
/// * a widening cast from an unsigned type no wider than `width` — the
///   extension fills with zeros (`CastKind::IntWiden` zero-extends exactly
///   when `from_ty` is unsigned);
/// * `And` with the full low-`width` mask on either side;
/// * a constant below `2^width`.
///
/// Anything else — `sext` of a narrow value, a phi, a load — is refused:
/// an unprovable pattern stays unfolded, never miscompiles.
fn high_bits_zero(opnd: Operand, width: u64, defs: &[Option<Instruction>]) -> bool {
    let v = peel_copies(opnd, defs);
    if let Some(c) = const_u64(v) {
        if width >= 64 {
            return true; // any u64 < 2^64
        }
        return c < (1u64 << width);
    }
    let Operand::Value(id) = v else {
        return false;
    };
    match defs.get(id.0 as usize).and_then(Option::as_ref) {
        Some(Instruction::Cast { from_ty, to_ty, .. }) => {
            to_ty.size() > from_ty.size()
                && from_ty.is_unsigned()
                && (from_ty.size() as u64) * 8 <= width
        }
        Some(Instruction::BinOp {
            op: IrBinOp::And,
            lhs,
            rhs,
            ..
        }) => {
            let mask = if width >= 64 {
                u64::MAX
            } else {
                (1u64 << width) - 1
            };
            const_u64(peel(*lhs, defs)) == Some(mask) || const_u64(peel(*rhs, defs)) == Some(mask)
        }
        _ => false,
    }
}

/// Truncation-aware narrow rotate.
///
/// C runs all 8/16-bit arithmetic at `int` width, so a narrow rotate arrives
/// promoted: the complements meet only at the NARROW width, and the result is
/// consumed by a truncation back to it:
///
/// ```c
/// uint16_t rol16_sw(uint16_t v)  { return (uint16_t)((v << 8) | (v >> 8)); }
/// uint16_t rol16_var(uint16_t v, int c)
///                                 { return (uint16_t)((v << c) | (v >> (16 - c))); }
/// ```
///
/// The IR shape is `Trunc_W(Or_{32}(Shl_{32}(x, a), Shr_{32}(x, b)))` with
/// `a + b == W` and `x` provably zero above bit `W` (see [`high_bits_zero`]).
/// Both shift kinds are accepted on the right half: the promotion of an
/// unsigned narrow value shifts a signed `int`, arriving as `AShr`, but the
/// proof makes it exact.
///
/// Value identity follows the post-beb28b4c discipline: the halves must read
/// the SAME SSA value (Copy chains transparent, casts opaque) at the Or's own
/// width.
///
/// The rewrite is exact for every count the IR defines: constant counts in
/// `[1, W-1]`; run-time counts in `[0, W]` (0 and W both degenerate to the
/// identity — `Shl(x, 0)` contributes `x` and `Shr(x, W)` contributes zero
/// under the proof — matching `rol` by `c mod W`).  The x86 hardware count
/// for 8/16-bit rotates is `(count & 31) mod W` (measured: `rolw` by 16 is
/// the identity, by 17 a rotate of 1), i.e. exactly count-mod-width, so the
/// backend needs no masking.
///
/// Returns `(value, amount, is_left)` like [`match_rotate`], where `value`
/// is the *wide* source — the driver emits `Rol_W(Trunc(value), amount)`.
fn match_narrow_rotate(
    or_src: Operand,
    narrow: u64,
    defs: &[Option<Instruction>],
) -> Option<(Operand, Operand, bool)> {
    let (a, b, ty) = binop(or_src, IrBinOp::Or, defs)?;
    if !matches!(ty, IrType::I32 | IrType::U32) {
        return None;
    }
    // The two halves in either `Or` order, the right half as LShr or AShr.
    let mut matched = None;
    for (x, y) in [(a, b), (b, a)] {
        if let (Some(shl), Some(shr)) = (
            shift_any(x, IrBinOp::Shl, defs),
            shift_any(y, IrBinOp::LShr, defs).or_else(|| shift_any(y, IrBinOp::AShr, defs)),
        ) {
            matched = Some((shl, shr));
            break;
        }
    }
    let ((shl_val, shl_amt, shl_ty), (shr_val, shr_amt, shr_ty)) = matched?;
    if shl_ty != ty || shr_ty != ty {
        return None;
    }
    let value = peel_copies(shl_val, defs);
    if value != peel_copies(shr_val, defs) {
        return None;
    }
    if !high_bits_zero(value, narrow, defs) {
        return None;
    }
    // Complementary at the NARROW width, with the same constant / live
    // `W - n` spellings (and the same direction choice) as `match_rotate`.
    let (amount, is_left) = match (
        const_u64(peel(shl_amt, defs)),
        const_u64(peel(shr_amt, defs)),
    ) {
        (Some(left), Some(right)) => {
            if left + right != narrow || left == 0 || right == 0 {
                return None;
            }
            if left <= right {
                (shl_amt, true)
            } else {
                (shr_amt, false)
            }
        }
        (Some(_), None) => {
            if !is_width_minus(shr_amt, shl_amt, narrow, defs) {
                return None;
            }
            (shl_amt, true)
        }
        (None, Some(_)) => {
            if !is_width_minus(shl_amt, shr_amt, narrow, defs) {
                return None;
            }
            (shr_amt, false)
        }
        (None, None) => {
            if is_width_minus(shr_amt, shl_amt, narrow, defs) {
                (shl_amt, true)
            } else if is_width_minus(shl_amt, shr_amt, narrow, defs) {
                (shr_amt, false)
            } else {
                return None;
            }
        }
    };
    Some((value, amount, is_left))
}

fn select(
    opnd: Operand,
    defs: &[Option<Instruction>],
) -> Option<(Operand, Operand, Operand, IrType)> {
    let Operand::Value(v) = peel(opnd, defs) else {
        return None;
    };
    match defs.get(v.0 as usize).and_then(Option::as_ref) {
        Some(Instruction::Select {
            cond,
            true_val,
            false_val,
            ty,
            ..
        }) => Some((*cond, *true_val, *false_val, *ty)),
        _ => None,
    }
}

fn unsigned_le_const(
    opnd: Operand,
    constant: u64,
    defs: &[Option<Instruction>],
) -> Option<Operand> {
    let Operand::Value(v) = peel(opnd, defs) else {
        return None;
    };
    match defs.get(v.0 as usize).and_then(Option::as_ref) {
        Some(Instruction::Cmp {
            op: IrCmpOp::Ule,
            lhs,
            rhs,
            ..
        }) if const_u64(*rhs) == Some(constant) => Some(*lhs),
        _ => None,
    }
}

fn add_const(opnd: Operand, amount: u64, defs: &[Option<Instruction>]) -> Option<Operand> {
    commutative_const(opnd, IrBinOp::Add, amount, defs)
}

/// Match `((value & mask) == 0)` in either operand order and return `value`.
/// This is the condition form produced by lowering Linux's portable `__ffs`
/// tree. It is intentionally stricter than a generic equality simplifier: the
/// mask is part of the idiom's proof and each stage is checked independently.
fn equal_zero_mask(condition: Operand, mask: u64, defs: &[Option<Instruction>]) -> Option<Operand> {
    let Operand::Value(v) = peel(condition, defs) else {
        return None;
    };
    let Instruction::Cmp { op, lhs, rhs, .. } = defs.get(v.0 as usize).and_then(Option::as_ref)?
    else {
        return None;
    };
    if *op != IrCmpOp::Eq || const_u64(*rhs) != Some(0) {
        return None;
    }
    commutative_const(*lhs, IrBinOp::And, mask, defs)
}

fn is_incremented(
    true_val: Operand,
    false_val: Operand,
    amount: u64,
    defs: &[Option<Instruction>],
) -> bool {
    if add_const(true_val, amount, defs).is_some_and(|base| same_value(base, false_val, defs)) {
        return true;
    }
    match (
        const_u64(peel(true_val, defs)),
        const_u64(peel(false_val, defs)),
    ) {
        (Some(a), Some(b)) => a == b.wrapping_add(amount),
        _ => false,
    }
}

/// Match the canonical 32-bit parallel population-count expression:
///
/// `x -= (x >> 1) & 0x55555555; ...; (x * 0x01010101) >> 24`
fn match_popcount32(result: Operand, defs: &[Option<Instruction>]) -> Option<Operand> {
    let multiplied = shift(result, IrBinOp::LShr, 24, defs)?;
    let stage3 = commutative_const(multiplied, IrBinOp::Mul, 0x0101_0101, defs)?;
    let stage3_add = commutative_const(stage3, IrBinOp::And, 0x0f0f_0f0f, defs)?;
    let (a, b, _) = binop(stage3_add, IrBinOp::Add, defs)?;
    let stage2 = if shift(a, IrBinOp::LShr, 4, defs).is_some_and(|base| same(base, b, defs)) {
        b
    } else if shift(b, IrBinOp::LShr, 4, defs).is_some_and(|base| same(base, a, defs)) {
        a
    } else {
        return None;
    };

    let (a, b, _) = binop(stage2, IrBinOp::Add, defs)?;
    let a_base = commutative_const(a, IrBinOp::And, 0x3333_3333, defs)?;
    let b_base = commutative_const(b, IrBinOp::And, 0x3333_3333, defs)?;
    let stage1 = if shift(a_base, IrBinOp::LShr, 2, defs)
        .is_some_and(|base| same(base, b_base, defs))
    {
        b_base
    } else if shift(b_base, IrBinOp::LShr, 2, defs).is_some_and(|base| same(base, a_base, defs)) {
        a_base
    } else {
        return None;
    };

    let (original, subtracted, ty) = binop(stage1, IrBinOp::Sub, defs)?;
    if ty != IrType::U32 && ty != IrType::I32 {
        return None;
    }
    let shifted = commutative_const(subtracted, IrBinOp::And, 0x5555_5555, defs)?;
    let shifted_base = shift(shifted, IrBinOp::LShr, 1, defs)?;
    same_value(original, shifted_base, defs).then_some(peel_copies(original, defs))
}

/// Match the common binary-search implementation of 32-bit count-leading-zeros.
/// If-conversion turns each source-level `if` into paired selects: one shifts the
/// working value and the other increments the count.  Requiring both chains to
/// agree makes this substantially stricter than merely matching the constants.
fn match_clz32(result: Operand, defs: &[Option<Instruction>]) -> Option<Operand> {
    let (final_cond, final_true, final_false, final_ty) = select(result, defs)?;
    if final_ty != IrType::I32 && final_ty != IrType::U32 {
        return None;
    }
    let mut count = final_false;
    if !is_incremented(final_true, count, 1, defs) {
        return None;
    }
    let mut working = unsigned_le_const(final_cond, 0x7fff_ffff, defs)?;

    for (amount, threshold) in [
        (2, 0x3fff_ffff),
        (4, 0x0fff_ffff),
        (8, 0x00ff_ffff),
        (16, 0x0000_ffff),
    ] {
        let (count_cond, count_true, count_false, _) = select(count, defs)?;
        if !is_incremented(count_true, count_false, amount, defs) {
            return None;
        }
        let (value_cond, value_true, value_false, value_ty) = select(working, defs)?;
        if value_ty != IrType::U32 && value_ty != IrType::I32 {
            return None;
        }
        if !same_value(count_cond, value_cond, defs) {
            return None;
        }
        if !shift(value_true, IrBinOp::Shl, amount, defs)
            .is_some_and(|base| same_value(base, value_false, defs))
        {
            return None;
        }
        let compared = unsigned_le_const(value_cond, threshold, defs)?;
        if !same_value(compared, value_false, defs) {
            return None;
        }
        count = count_false;
        working = value_false;
    }
    (const_u64(peel(count, defs)) == Some(0)).then_some(peel_copies(working, defs))
}

/// Match the six-stage portable 64-bit `__ffs` tree used by Linux:
///
/// ```text
/// if ((x & 0xffffffff) == 0) { n += 32; x >>= 32; }
/// if ((x & 0xffff) == 0)     { n += 16; x >>= 16; }
/// ...
/// ```
///
/// If-conversion produces a select chain for `x` and a parallel select chain
/// for `n`. Requiring every condition, shift, mask and increment to agree is
/// what makes this safe. The zero case is preserved explicitly: the Linux tree
/// returns 63 for zero, while native `tzcnt` returns the operand width. The
/// rewrite materializes a nonzero Ctz operand and selects 63 for zero.
fn match_ctz64(result: Operand, defs: &[Option<Instruction>]) -> Option<Operand> {
    let (final_cond, final_true, final_false, final_ty) = select(result, defs)?;
    if final_ty != IrType::I32 && final_ty != IrType::U32 {
        return None;
    }
    if !is_incremented(final_true, final_false, 1, defs) {
        return None;
    }

    // The final one-bit test selects only the count; its working-value input
    // is the operand of the final mask test.
    let mut working = equal_zero_mask(final_cond, 1, defs)?;
    let mut count = final_false;

    for (amount, mask) in [(2, 3), (4, 15), (8, 255), (16, 65535), (32, 0xffff_ffff)] {
        let (count_cond, count_true, count_false, count_ty) = select(count, defs)?;
        if count_ty != IrType::I32 && count_ty != IrType::U32 {
            return None;
        }
        if !is_incremented(count_true, count_false, amount, defs) {
            return None;
        }

        let (value_cond, value_true, value_false, value_ty) = select(working, defs)?;
        if value_ty != IrType::I64 && value_ty != IrType::U64 {
            return None;
        }
        if !same_value(count_cond, value_cond, defs) {
            return None;
        }
        if !shift(value_true, IrBinOp::LShr, amount, defs)
            .is_some_and(|base| same_value(base, value_false, defs))
        {
            return None;
        }
        if !equal_zero_mask(count_cond, mask, defs)
            .is_some_and(|base| same_value(base, value_false, defs))
        {
            return None;
        }
        working = value_false;
        count = count_false;
    }

    (const_u64(peel(count, defs)) == Some(0)).then_some(peel_copies(working, defs))
}

fn match_shift_pair(opnd: Operand, amount: u64, defs: &[Option<Instruction>]) -> Option<Operand> {
    let (a, b, _) = binop(opnd, IrBinOp::Or, defs)?;
    if let (Some(x), Some(y)) = (
        shift(a, IrBinOp::LShr, amount, defs),
        shift(b, IrBinOp::Shl, amount, defs),
    ) {
        if same_value(x, y, defs) {
            return Some(x);
        }
    }
    if let (Some(x), Some(y)) = (
        shift(b, IrBinOp::LShr, amount, defs),
        shift(a, IrBinOp::Shl, amount, defs),
    ) {
        if same_value(x, y, defs) {
            return Some(x);
        }
    }
    None
}

/// Match the final byte-swap portion of a mask-and-shift bit reversal:
///
/// `y = ((x >> 8) & 0x00ff00ff) | ((x & 0x00ff00ff) << 8);`
/// `result = (y >> 16) | (y << 16);`
fn match_bswap32_network(result: Operand, defs: &[Option<Instruction>]) -> Option<Operand> {
    let byte_swapped = match_shift_pair(result, 16, defs)?;
    let (a, b, ty) = binop(byte_swapped, IrBinOp::Or, defs)?;
    if ty != IrType::U32 && ty != IrType::I32 {
        return None;
    }

    let match_halves = |right: Operand, left: Operand| -> Option<Operand> {
        let right_shifted = commutative_const(right, IrBinOp::And, 0x00ff_00ff, defs)?;
        let original = shift(right_shifted, IrBinOp::LShr, 8, defs)?;
        let left_masked = shift(left, IrBinOp::Shl, 8, defs)?;
        let left_original = commutative_const(left_masked, IrBinOp::And, 0x00ff_00ff, defs)?;
        same_value(original, left_original, defs).then_some(peel_copies(original, defs))
    };
    match_halves(a, b).or_else(|| match_halves(b, a))
}

fn match_masked_swap_stage(
    result: Operand,
    amount: u64,
    mask: u64,
    defs: &[Option<Instruction>],
) -> Option<Operand> {
    let (a, b, ty) = binop(result, IrBinOp::Or, defs)?;
    if ty != IrType::U32 && ty != IrType::I32 {
        return None;
    }
    let match_halves = |right: Operand, left: Operand| -> Option<Operand> {
        let shifted = commutative_const(right, IrBinOp::And, mask, defs)?;
        let original = shift(shifted, IrBinOp::LShr, amount, defs)?;
        let masked = shift(left, IrBinOp::Shl, amount, defs)?;
        let left_original = commutative_const(masked, IrBinOp::And, mask, defs)?;
        same_value(original, left_original, defs).then_some(peel_copies(original, defs))
    };
    match_halves(a, b).or_else(|| match_halves(b, a))
}

fn match_bit_reverse32(result: Operand, defs: &[Option<Instruction>]) -> Option<Operand> {
    let mut value = match_bswap32_network(result, defs)?;
    for (amount, mask) in [(4, 0x0f0f_0f0f), (2, 0x3333_3333), (1, 0x5555_5555)] {
        value = match_masked_swap_stage(value, amount, mask, defs)?;
    }
    Some(peel_copies(value, defs))
}

/// `enable_bit_reverse` gates the idiom whose native lowering is target
/// specific (AArch64 `rbit`).
///
/// `max_rotate_bits` is the widest rotate the target can lower natively, or 0
/// to disable recognition entirely: x86-64 `rol`/`ror`, AArch64 `ror`/`extr`
/// and the RISC-V funnel shift all handle 64 bits, while i686's paired-register
/// route for I64 arithmetic means only its 32-bit ALU path is wired.  Passing
/// the capability instead of a boolean keeps a backend from ever receiving a
/// node it cannot lower.
///
/// `min_rotate_bits` is the NARROWEST rotate the target can lower natively:
/// x86 `rolb`/`rolw` rotate at the operand width (hardware count is
/// `(count & 31) mod width`), but AArch64 and RISC-V only rotate at the full
/// register width, so their sub-word idioms must stay in shift form.  It gates
/// the truncation-aware narrow-rotate pattern.
/// True when `op`'s value provably fits in `bits` bits (zero-extended into
/// the carrying type), so `op & (2^bits - 1) == op`.
///
/// Provable shapes: an unsigned narrow constant, a widening `Cast` whose
/// SOURCE type is an unsigned narrow integer (a zero-extension of a value in
/// `[0, 2^bits)`), and a direct `Load` of an unsigned narrow type. A cast
/// from a SIGNED narrow type sign-extends and can be negative in the wide
/// domain — refused.
fn fits_low_bits(op: Operand, bits: u32, defs: &[Option<Instruction>]) -> bool {
    let mask = (1u64 << bits) - 1;
    match op {
        Operand::Const(_) => const_u64(op).is_some_and(|v| v <= mask),
        Operand::Value(v) => match defs.get(v.0 as usize).and_then(Option::as_ref) {
            Some(Instruction::Cast { src, from_ty, .. }) => {
                let from_ok = matches!((from_ty, bits), (IrType::U8, 8) | (IrType::U16, 16));
                from_ok && fits_low_bits(*src, bits, defs)
            }
            Some(Instruction::Load { ty, .. }) => {
                matches!((ty, bits), (IrType::U8, 8) | (IrType::U16, 16))
            }
            _ => false,
        },
    }
}

pub(crate) fn recognize_function(
    func: &mut IrFunction,
    enable_bit_reverse: bool,
    max_rotate_bits: u32,
    min_rotate_bits: u32,
) -> usize {
    let mut defs = vec![None; func.max_value_id() as usize + 1];
    // (block index, instruction index) of each value's defining instruction,
    // for in-place rewrites of the PRODUCER while visiting the consumer.
    let mut def_loc = vec![None; func.max_value_id() as usize + 1];
    // Total read counts (operands + address uses + terminator operands), for
    // single-use proofs when repurposing a producer's operation.
    let mut use_counts = vec![0u32; func.max_value_id() as usize + 1];
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Some(dest) = inst.dest() {
                defs[dest.0 as usize] = Some(inst.clone());
                def_loc[dest.0 as usize] = Some((bi, ii));
            }
            crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    use_counts[v.0 as usize] += 1;
                }
            });
            crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                use_counts[v.0 as usize] += 1;
            });
        }
        crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                use_counts[v.0 as usize] += 1;
            }
        });
    }

    let mut changes = 0;
    let mut next_value_id = func.max_value_id().saturating_add(1);
    // Deferred cross-block rewrites (mask distribution): the producer and
    // consumer may live in different blocks; both mutations are applied
    // after the scan ends so no &mut borrow of `func.blocks` overlaps the
    // loop's. Entries: (producer block, producer index, new producer
    // instruction, consumer block, consumer index, new consumer
    // instruction).
    let mut pending_masks: Vec<(usize, usize, Instruction, usize, usize, Instruction)> = Vec::new();
    for (block_bi, block) in func.blocks.iter_mut().enumerate() {
        let mut index = 0;
        while index < block.instructions.len() {
            let mut consumed = 1;

            // Shift/rotate count canonicalization: forward integer casts and
            // Copies on the COUNT operand. Every backend consumes a variable
            // count masked at the shift's own width (x86 `%cl` with
            // `(count & 31) mod width`; AArch64/RISC-V register shifts mask
            // at the operand width), and every integer cast preserves the
            // count's low bits, so `Rol(x, (i64)c)` is exactly `Rol(x, c)`.
            // Forwarding deletes the cast's last use (DCE retires it) and
            // lets the count stage with one 32-bit move instead of a 64-bit
            // extension dance (`movslq %esi,%rdx; mov %edx,%rcx`). GCC's
            // equivalent is folding the extension into the count's single
            // use.
            //
            // Runs before the match so the idiom arms below see the
            // canonicalized amounts (a `32 - (i64)n` complement matches
            // against a plain `n` on both halves after forwarding).
            if let Instruction::BinOp { op, rhs, .. } = &block.instructions[index] {
                if matches!(
                    op,
                    IrBinOp::Shl
                        | IrBinOp::LShr
                        | IrBinOp::AShr
                        | IrBinOp::RotateLeft
                        | IrBinOp::RotateRight
                ) {
                    let start = *rhs;
                    let mut forwarded = start;
                    let mut hops = 0;
                    let mut changed_here = false;
                    while hops < 8 {
                        hops += 1;
                        let Operand::Value(v) = forwarded else { break };
                        let next = match defs.get(v.0 as usize).and_then(Option::as_ref) {
                            Some(Instruction::Cast {
                                src,
                                from_ty,
                                to_ty,
                                ..
                            }) if from_ty.is_integer() && to_ty.is_integer() => Some(*src),
                            Some(Instruction::Copy { src, .. }) => Some(*src),
                            _ => None,
                        };
                        match next {
                            Some(next_src) => {
                                forwarded = next_src;
                                changed_here = true;
                            }
                            None => break,
                        }
                    }
                    if changed_here {
                        if let Instruction::BinOp { rhs, .. } = &mut block.instructions[index] {
                            *rhs = forwarded;
                        }
                        changes += 1;
                    }
                }
            }

            // Mask distribution over a zero-extended operand:
            // `(a ^ b) & 0xff` -> `(a & 0xff) ^ b` when b is a zero-extended
            // byte (same for `|`, and for 0xffff/`movzwl`). See
            // `fits_low_bits` for the range proof. The rewrite moves the mask
            // onto the recurrence operand — where x86 lowers it to a
            // rename-eliminable cross-register movz that only READS the
            // recurrence register — and off the xor result, where the mask
            // always executes on the loop-carried chain (`and $255` or a
            // same-register movz, both uneliminable).
            if let Instruction::BinOp {
                dest,
                op: IrBinOp::And,
                lhs: Operand::Value(x),
                rhs: mask_op @ Operand::Const(_),
                ty,
                ..
            } = &block.instructions[index]
            {
                let dest = *dest;
                let x = *x;
                let mask_op = *mask_op;
                let ty = *ty;
                let bits = match const_u64(mask_op) {
                    Some(0xff) => 8,
                    Some(0xffff) => 16,
                    _ => 0,
                };
                if bits != 0
                    && matches!(ty, IrType::U32 | IrType::I32 | IrType::U64 | IrType::I64)
                    && use_counts.get(x.0 as usize).copied() == Some(1)
                {
                    let xor = defs.get(x.0 as usize).and_then(Option::as_ref).cloned();
                    if let Some(Instruction::BinOp {
                        op: op @ (IrBinOp::Xor | IrBinOp::Or),
                        lhs: Operand::Value(a),
                        rhs: Operand::Value(b),
                        ty: x_ty,
                        ..
                    }) = xor
                    {
                        let a = a;
                        let b = b;
                        let op = op;
                        let x_ty = x_ty;
                        if x_ty == ty {
                            let a_fits = fits_low_bits(Operand::Value(a), bits, &defs);
                            let b_fits = fits_low_bits(Operand::Value(b), bits, &defs);
                            // Exactly one side carries the narrow value; the
                            // other receives the mask. Both fitting is
                            // already optimal as-is (both and-free), and
                            // masking either side would only add work.
                            if a_fits != b_fits {
                                let (masked, narrow) = if b_fits { (a, b) } else { (b, a) };
                                // Rewrite the producer: xor/or -> and-with-mask;
                                // rewrite the consumer: and -> xor/or.
                                if let Some((pbi, pii)) =
                                    def_loc.get(x.0 as usize).copied().flatten()
                                {
                                    if matches!(
                                        defs.get(x.0 as usize),
                                        Some(Some(Instruction::BinOp { dest: d, .. })) if *d == x
                                    ) {
                                        pending_masks.push((
                                            pbi,
                                            pii,
                                            Instruction::BinOp {
                                                dest: x,
                                                op: IrBinOp::And,
                                                lhs: Operand::Value(masked),
                                                rhs: mask_op,
                                                ty,
                                            },
                                            block_bi,
                                            index,
                                            Instruction::BinOp {
                                                dest,
                                                op,
                                                lhs: Operand::Value(x),
                                                rhs: Operand::Value(narrow),
                                                ty,
                                            },
                                        ));
                                        changes += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            match &block.instructions[index] {
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::LShr,
                    ty,
                    ..
                } if *ty == IrType::U32 || *ty == IrType::I32 => {
                    let dest = *dest;
                    if let Some(src) = match_popcount32(Operand::Value(dest), &defs) {
                        block.instructions[index] = Instruction::UnaryOp {
                            dest,
                            op: IrUnaryOp::Popcount,
                            src,
                            ty: IrType::U32,
                        };
                        changes += 1;
                    }
                }
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Or,
                    ty,
                    ..
                } if matches!(*ty, IrType::U32 | IrType::I32 | IrType::U64 | IrType::I64) => {
                    let dest = *dest;
                    let ty = *ty;
                    let is_32bit = ty == IrType::U32 || ty == IrType::I32;
                    // Bit reversal and byte swap are strictly MORE specific than
                    // a rotate: bswap32's final stage is literally
                    // `(y >> 16) | (y << 16)`, which is a rotate by 16.  Trying
                    // the rotate first would strip that stage out from under
                    // `match_bswap32_network` and turn a one-instruction `bswap`
                    // back into the five-instruction network, so both specific
                    // matchers run first and bail out of this iteration when
                    // either fires.
                    if is_32bit {
                        if enable_bit_reverse
                            && let Some(src) = match_bit_reverse32(Operand::Value(dest), &defs)
                        {
                            block.instructions[index] = Instruction::UnaryOp {
                                dest,
                                op: IrUnaryOp::BitReverse,
                                src,
                                ty: IrType::U32,
                            };
                            changes += 1;
                            index += consumed;
                            continue;
                        }
                        if let Some(src) = match_bswap32_network(Operand::Value(dest), &defs) {
                            block.instructions[index] = Instruction::UnaryOp {
                                dest,
                                op: IrUnaryOp::Bswap,
                                src,
                                ty: IrType::U32,
                            };
                            changes += 1;
                            index += consumed;
                            continue;
                        }
                    }
                    if max_rotate_bits > 0
                        && let Some((value, amount, is_left)) =
                            match_rotate(Operand::Value(dest), max_rotate_bits, &defs)
                    {
                        block.instructions[index] = Instruction::BinOp {
                            dest,
                            op: if is_left {
                                IrBinOp::RotateLeft
                            } else {
                                IrBinOp::RotateRight
                            },
                            lhs: value,
                            rhs: amount,
                            ty,
                        };
                        changes += 1;
                    }
                }
                Instruction::Select { dest, ty, .. }
                    if *ty == IrType::U32 || *ty == IrType::I32 || *ty == IrType::U64 =>
                {
                    let dest = *dest;
                    let ty = *ty;
                    if let Some(src) = match_ctz64(Operand::Value(dest), &defs) {
                        if ty == IrType::U64 {
                            block.instructions[index] = Instruction::UnaryOp {
                                dest,
                                op: IrUnaryOp::Ctz,
                                src,
                                ty: IrType::U64,
                            };
                        } else {
                            // The source tree returns 63 for zero (it is
                            // normally called only after a nonzero guard, but
                            // the standalone C function is still defined).
                            // Make the Ctz operand nonzero before evaluating
                            // it, then select the exact zero result. This is
                            // required on targets where the native Ctz
                            // instruction is undefined for zero.
                            let zero = crate::ir::reexports::Value(next_value_id);
                            let safe_src = crate::ir::reexports::Value(next_value_id + 1);
                            let ctz = crate::ir::reexports::Value(next_value_id + 2);
                            let narrowed = crate::ir::reexports::Value(next_value_id + 3);
                            next_value_id = next_value_id.saturating_add(4);
                            block.instructions[index] = Instruction::Cmp {
                                dest: zero,
                                op: IrCmpOp::Eq,
                                lhs: src,
                                rhs: Operand::Const(IrConst::I64(0)),
                                ty: IrType::U64,
                            };
                            block.instructions.insert(
                                index + 1,
                                Instruction::Select {
                                    dest: safe_src,
                                    cond: Operand::Value(zero),
                                    true_val: Operand::Const(IrConst::I64(1)),
                                    false_val: src,
                                    ty: IrType::U64,
                                },
                            );
                            block.instructions.insert(
                                index + 2,
                                Instruction::UnaryOp {
                                    dest: ctz,
                                    op: IrUnaryOp::Ctz,
                                    src: Operand::Value(safe_src),
                                    ty: IrType::U64,
                                },
                            );
                            block.instructions.insert(
                                index + 3,
                                Instruction::Cast {
                                    dest: narrowed,
                                    src: Operand::Value(ctz),
                                    from_ty: IrType::U64,
                                    to_ty: ty,
                                },
                            );
                            block.instructions.insert(
                                index + 4,
                                Instruction::Select {
                                    dest,
                                    cond: Operand::Value(zero),
                                    true_val: Operand::Const(IrConst::I64(63)),
                                    false_val: Operand::Value(narrowed),
                                    ty,
                                },
                            );
                            consumed = 5;
                        }
                        changes += 1;
                    } else if (ty == IrType::U32 || ty == IrType::I32)
                        && match_clz32(Operand::Value(dest), &defs).is_some()
                    {
                        let src = match_clz32(Operand::Value(dest), &defs).unwrap();
                        block.instructions[index] = Instruction::UnaryOp {
                            dest,
                            op: IrUnaryOp::Clz,
                            src,
                            ty: IrType::U32,
                        };
                        changes += 1;
                    }
                }
                // Truncation-aware narrow rotate: the promoted halves'
                // complements meet at the truncation's width, not the Or's.
                // Targets without sub-word rotates (`min_rotate_bits` above
                // the narrow width) never take this arm, so their backends
                // cannot receive a node they cannot lower.
                Instruction::Cast {
                    dest,
                    src,
                    from_ty,
                    to_ty,
                } if matches!(from_ty, IrType::I32 | IrType::U32)
                    && narrow_width_bits(*to_ty).is_some_and(|w| {
                        w >= u64::from(min_rotate_bits) && min_rotate_bits > 0
                    }) =>
                {
                    let dest = *dest;
                    let src = *src;
                    let from_ty = *from_ty;
                    let to_ty = *to_ty;
                    let narrow = narrow_width_bits(to_ty).unwrap();
                    if let Some((value, amount, is_left)) = match_narrow_rotate(src, narrow, &defs)
                    {
                        // Trunc the shared source down to the rotate's own
                        // width, then rotate there.  The Or/Shl/Shr chain
                        // loses this consumer; DCE retires it when nothing
                        // else reads it (multiple consumers just keep the
                        // chain — the rewrite never touches it).
                        let trunc = crate::ir::reexports::Value(next_value_id);
                        next_value_id = next_value_id.saturating_add(1);
                        block.instructions[index] = Instruction::Cast {
                            dest: trunc,
                            src: value,
                            from_ty,
                            to_ty,
                        };
                        block.instructions.insert(
                            index + 1,
                            Instruction::BinOp {
                                dest,
                                op: if is_left {
                                    IrBinOp::RotateLeft
                                } else {
                                    IrBinOp::RotateRight
                                },
                                lhs: Operand::Value(trunc),
                                rhs: amount,
                                ty: to_ty,
                            },
                        );
                        changes += 1;
                        consumed = 2;
                    }
                }
                _ => {}
            }
            index += consumed;
        }
    }
    func.next_value_id = next_value_id.max(func.next_value_id);
    for (pbi, pii, new_prod, cbi, cii, new_cons) in pending_masks {
        let x_dest = match &new_prod {
            Instruction::BinOp { dest, .. } => *dest,
            _ => unreachable!("mask distribution producer is always a BinOp"),
        };
        let c_dest = match &new_cons {
            Instruction::BinOp { dest, .. } => *dest,
            _ => unreachable!("mask distribution consumer is always a BinOp"),
        };
        if let Some(pb) = func.blocks.get_mut(pbi) {
            if matches!(
                pb.instructions.get(pii),
                Some(Instruction::BinOp { dest: d, .. }) if *d == x_dest
            ) {
                pb.instructions[pii] = new_prod;
            }
        }
        if let Some(cb) = func.blocks.get_mut(cbi) {
            if matches!(
                cb.instructions.get(cii),
                Some(Instruction::BinOp { dest: d, .. }) if *d == c_dest
            ) {
                cb.instructions[cii] = new_cons;
            }
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::Value;

    fn swar_defs(second_mask: i64) -> Vec<Option<Instruction>> {
        let value = |id| Operand::Value(Value(id));
        let constant = |n| Operand::Const(IrConst::I64(n));
        let mut defs = vec![None; 13];
        let mut put = |id, op, lhs, rhs| {
            defs[id as usize] = Some(Instruction::BinOp {
                dest: Value(id),
                op,
                lhs,
                rhs,
                ty: IrType::U32,
            });
        };
        put(1, IrBinOp::LShr, value(0), constant(1));
        put(2, IrBinOp::And, value(1), constant(0x5555_5555));
        put(3, IrBinOp::Sub, value(0), value(2));
        put(4, IrBinOp::And, value(3), constant(0x3333_3333));
        put(5, IrBinOp::LShr, value(3), constant(2));
        put(6, IrBinOp::And, value(5), constant(second_mask));
        put(7, IrBinOp::Add, value(4), value(6));
        put(8, IrBinOp::LShr, value(7), constant(4));
        put(9, IrBinOp::Add, value(7), value(8));
        put(10, IrBinOp::And, value(9), constant(0x0f0f_0f0f));
        put(11, IrBinOp::Mul, value(10), constant(0x0101_0101));
        put(12, IrBinOp::LShr, value(11), constant(24));
        defs
    }

    #[test]
    fn recognizes_canonical_swar_popcount32() {
        let defs = swar_defs(0x3333_3333);
        assert!(same(
            match_popcount32(Operand::Value(Value(12)), &defs).unwrap(),
            Operand::Value(Value(0)),
            &defs
        ));
    }

    #[test]
    fn rejects_near_miss_swar_popcount32() {
        let defs = swar_defs(0x3333_3331);
        assert!(match_popcount32(Operand::Value(Value(12)), &defs).is_none());
    }

    /// Build `Or(Shl(shl_src, c1), LShr(lshr_src, c2))` at `ty`; ids 1..4.
    fn rotate_defs(
        ty: IrType,
        shl_src: Operand,
        lshr_src: Operand,
        c1: i64,
        c2: i64,
    ) -> Vec<Option<Instruction>> {
        let value = |id| Operand::Value(Value(id));
        let constant = |n| Operand::Const(IrConst::I64(n));
        let mut defs = vec![None; 5];
        defs[1] = Some(Instruction::BinOp {
            dest: Value(1),
            op: IrBinOp::Shl,
            lhs: shl_src,
            rhs: constant(c1),
            ty,
        });
        defs[2] = Some(Instruction::BinOp {
            dest: Value(2),
            op: IrBinOp::LShr,
            lhs: lshr_src,
            rhs: constant(c2),
            ty,
        });
        defs[3] = Some(Instruction::BinOp {
            dest: Value(3),
            op: IrBinOp::Or,
            lhs: value(1),
            rhs: value(2),
            ty,
        });
        defs
    }

    #[test]
    fn recognizes_rotate_both_operand_orders() {
        let v = |id| Operand::Value(Value(id));
        // Or(Shl(x, 12), LShr(x, 20)) — shl on the lhs, smaller count wins.
        let defs = rotate_defs(IrType::U32, v(0), v(0), 12, 20);
        let (value, amount, is_left) = match_rotate(Operand::Value(Value(3)), 64, &defs).unwrap();
        assert_eq!(value, v(0));
        assert_eq!(amount, Operand::Const(IrConst::I64(12)));
        assert!(is_left);

        // Swapped OR operands: still the rotate by 12.
        let mut defs = rotate_defs(IrType::U32, v(0), v(0), 12, 20);
        if let Some(Instruction::BinOp { lhs, rhs, .. }) = defs[3].as_mut() {
            std::mem::swap(lhs, rhs);
        }
        let (value, _, _) = match_rotate(Operand::Value(Value(3)), 64, &defs).unwrap();
        assert_eq!(value, v(0));

        // Larger shl count: direction flips to the rotate-right view.
        let defs = rotate_defs(IrType::U32, v(0), v(0), 25, 7);
        let (_, amount, is_left) = match_rotate(Operand::Value(Value(3)), 64, &defs).unwrap();
        assert_eq!(amount, Operand::Const(IrConst::I64(7)));
        assert!(!is_left);
    }

    #[test]
    fn recognizes_rotate_64_bit_and_extreme_counts() {
        let v = |id| Operand::Value(Value(id));
        for c in [1u64, 63, 33, 40] {
            let defs = rotate_defs(IrType::U64, v(0), v(0), c as i64, (64 - c) as i64);
            let (value, amount, is_left) =
                match_rotate(Operand::Value(Value(3)), 64, &defs).unwrap();
            assert_eq!(value, v(0));
            // The smaller count wins, matching the reference compilers'
            // spelling choice: rol $1, not ror $63.
            let small = c.min(64 - c);
            assert_eq!(amount, Operand::Const(IrConst::I64(small as i64)));
            assert_eq!(is_left, c < 64 - c);
        }
        // i686 advertises 32-bit rotates only.
        let defs = rotate_defs(IrType::U64, v(0), v(0), 33, 31);
        assert!(match_rotate(Operand::Value(Value(3)), 32, &defs).is_none());
    }

    #[test]
    fn recognizes_variable_amount_rotate() {
        let v = |id| Operand::Value(Value(id));
        let constant = |n| Operand::Const(IrConst::I64(n));
        // Or(Shl(x, n), LShr(x, Sub(32, n)))
        let mut defs = vec![None; 6];
        defs[1] = Some(Instruction::BinOp {
            dest: Value(1),
            op: IrBinOp::Shl,
            lhs: v(0),
            rhs: v(4),
            ty: IrType::U32,
        });
        defs[2] = Some(Instruction::BinOp {
            dest: Value(2),
            op: IrBinOp::Sub,
            lhs: constant(32),
            rhs: v(4),
            ty: IrType::U32,
        });
        defs[3] = Some(Instruction::BinOp {
            dest: Value(3),
            op: IrBinOp::LShr,
            lhs: v(0),
            rhs: v(2),
            ty: IrType::U32,
        });
        defs[4] = Some(Instruction::BinOp {
            dest: Value(4),
            op: IrBinOp::Or,
            lhs: v(1),
            rhs: v(3),
            ty: IrType::U32,
        });
        let (value, amount, is_left) = match_rotate(Operand::Value(Value(4)), 64, &defs).unwrap();
        assert_eq!(value, v(0));
        assert_eq!(amount, v(4));
        assert!(is_left);
    }

    #[test]
    fn rejects_non_complementary_rotate_counts() {
        let v = |id| Operand::Value(Value(id));
        // 5 + 20 != 32.
        let defs = rotate_defs(IrType::U32, v(0), v(0), 5, 20);
        assert!(match_rotate(Operand::Value(Value(3)), 64, &defs).is_none());
        // Degenerate full-width shift (C1 == W) with C2 == 0.
        let defs = rotate_defs(IrType::U32, v(0), v(0), 32, 0);
        assert!(match_rotate(Operand::Value(Value(3)), 64, &defs).is_none());
        // Arithmetic right shift half: not a rotate.
        let mut defs = rotate_defs(IrType::U32, v(0), v(0), 12, 20);
        if let Some(Instruction::BinOp { op, .. }) = defs[2].as_mut() {
            *op = IrBinOp::AShr;
        }
        assert!(match_rotate(Operand::Value(Value(3)), 64, &defs).is_none());
    }

    #[test]
    fn rejects_rotate_with_differing_sources() {
        let v = |id| Operand::Value(Value(id));
        let defs = rotate_defs(IrType::U32, v(0), v(9), 5, 27);
        assert!(match_rotate(Operand::Value(Value(3)), 64, &defs).is_none());
    }

    #[test]
    fn rejects_rotate_across_width_mismatch() {
        let v = |id| Operand::Value(Value(id));
        // The shifts run at u32 under a u64 OR: the amounts cannot be
        // checked against the OR's width, so this must not fold.
        let mut defs = rotate_defs(IrType::U64, v(0), v(0), 12, 20);
        if let Some(Instruction::BinOp { ty, .. }) = defs[1].as_mut() {
            *ty = IrType::U32;
        }
        if let Some(Instruction::BinOp { ty, .. }) = defs[2].as_mut() {
            *ty = IrType::U32;
        }
        assert!(match_rotate(Operand::Value(Value(3)), 64, &defs).is_none());
    }

    #[test]
    fn rejects_rotate_halves_through_different_casts_of_one_root() {
        // Or(Shl(zext(a), 16), LShr(sext(a), 16)): both halves peel to `a`
        // but zext/sext are different 32-bit values.  Folding this to a
        // rotate of anything silently drops the sign half — this is the
        // regression test for the cast-peeling soundness hole.
        let v = |id| Operand::Value(Value(id));
        let mut defs = vec![None; 7];
        defs[1] = Some(Instruction::Cast {
            dest: Value(1),
            src: v(0),
            from_ty: IrType::I8,
            to_ty: IrType::U32,
        });
        defs[2] = Some(Instruction::Cast {
            dest: Value(2),
            src: v(0),
            from_ty: IrType::I8,
            to_ty: IrType::I32,
        });
        defs[3] = Some(Instruction::BinOp {
            dest: Value(3),
            op: IrBinOp::Shl,
            lhs: v(1),
            rhs: Operand::Const(IrConst::I64(16)),
            ty: IrType::U32,
        });
        defs[4] = Some(Instruction::BinOp {
            dest: Value(4),
            op: IrBinOp::LShr,
            lhs: v(2),
            rhs: Operand::Const(IrConst::I64(16)),
            ty: IrType::U32,
        });
        defs[5] = Some(Instruction::BinOp {
            dest: Value(5),
            op: IrBinOp::Or,
            lhs: v(3),
            rhs: v(4),
            ty: IrType::U32,
        });
        assert!(match_rotate(Operand::Value(Value(5)), 64, &defs).is_none());
    }

    #[test]
    fn recognizes_rotate_through_one_shared_cast() {
        // Both halves read the SAME cast value: a rotate of the extended
        // value.  The fold must fire and keep the cast as the source (the
        // rotate runs at the cast's width), not peel to the narrow root.
        let v = |id| Operand::Value(Value(id));
        let mut defs = vec![None; 7];
        defs[1] = Some(Instruction::Cast {
            dest: Value(1),
            src: v(0),
            from_ty: IrType::U16,
            to_ty: IrType::U32,
        });
        defs[3] = Some(Instruction::BinOp {
            dest: Value(3),
            op: IrBinOp::Shl,
            lhs: v(1),
            rhs: Operand::Const(IrConst::I64(8)),
            ty: IrType::U32,
        });
        defs[4] = Some(Instruction::BinOp {
            dest: Value(4),
            op: IrBinOp::LShr,
            lhs: v(1),
            rhs: Operand::Const(IrConst::I64(24)),
            ty: IrType::U32,
        });
        defs[5] = Some(Instruction::BinOp {
            dest: Value(5),
            op: IrBinOp::Or,
            lhs: v(3),
            rhs: v(4),
            ty: IrType::U32,
        });
        let (value, _, is_left) = match_rotate(Operand::Value(Value(5)), 64, &defs).unwrap();
        assert_eq!(value, v(1)); // the cast, not the u16 root
        assert!(is_left);
    }

    /// IR shape of `(uint16_t)((v << 8) | (v >> 8))` after integer
    /// promotion: both halves shift the SAME zext of the u16 value at i32,
    /// the right half arrives as AShr (the promoted type is signed), and a
    /// truncation consumes the Or.
    fn narrow_rotate_defs(
        right_op: IrBinOp,
        shr_amt: Operand,
        value: Operand,
    ) -> Vec<Option<Instruction>> {
        let mut defs = vec![None; 7];
        defs[1] = Some(Instruction::BinOp {
            dest: Value(1),
            op: IrBinOp::Shl,
            lhs: value,
            rhs: Operand::Const(IrConst::I64(8)),
            ty: IrType::I32,
        });
        defs[2] = Some(Instruction::BinOp {
            dest: Value(2),
            op: right_op,
            lhs: value,
            rhs: shr_amt,
            ty: IrType::I32,
        });
        defs[3] = Some(Instruction::BinOp {
            dest: Value(3),
            op: IrBinOp::Or,
            lhs: v(1),
            rhs: v(2),
            ty: IrType::I32,
        });
        defs
    }

    fn v(id: u32) -> Operand {
        Operand::Value(Value(id))
    }

    fn zext_u16(id: u32) -> Instruction {
        Instruction::Cast {
            dest: Value(id),
            src: v(id - 1),
            from_ty: IrType::U16,
            to_ty: IrType::I32,
        }
    }

    #[test]
    fn recognizes_narrow_rotate_constant_ashr() {
        // `(uint16_t)((v << 8) | (v >> 8))`: complements meet only at 16;
        // the promoted right shift is AShr but the zext proof makes it exact.
        let mut defs = narrow_rotate_defs(IrBinOp::AShr, Operand::Const(IrConst::I64(8)), v(5));
        defs[5] = Some(zext_u16(5));
        let (value, amount, is_left) = match_narrow_rotate(v(3), 16, &defs).unwrap();
        assert_eq!(value, v(5)); // the zext, not the u16 root
        assert_eq!(const_u64(peel(amount, &defs)), Some(8));
        assert!(is_left);
    }

    #[test]
    fn recognizes_narrow_rotate_variable_count() {
        // `(uint16_t)((v << c) | (v >> (16 - c)))`: the complement is the
        // live `16 - c` subtraction, matched at the NARROW width.
        let mut defs = vec![None; 8];
        defs[1] = Some(Instruction::BinOp {
            dest: Value(1),
            op: IrBinOp::Sub,
            lhs: Operand::Const(IrConst::I64(16)),
            rhs: v(4),
            ty: IrType::I32,
        });
        defs[2] = Some(Instruction::BinOp {
            dest: Value(2),
            op: IrBinOp::Shl,
            lhs: v(5),
            rhs: v(4),
            ty: IrType::I32,
        });
        defs[3] = Some(Instruction::BinOp {
            dest: Value(3),
            op: IrBinOp::LShr,
            lhs: v(5),
            rhs: v(1),
            ty: IrType::I32,
        });
        defs[6] = Some(Instruction::BinOp {
            dest: Value(6),
            op: IrBinOp::Or,
            lhs: v(2),
            rhs: v(3),
            ty: IrType::I32,
        });
        defs[5] = Some(zext_u16(5));
        let (value, amount, is_left) = match_narrow_rotate(v(6), 16, &defs).unwrap();
        assert_eq!(value, v(5));
        assert_eq!(amount, v(4)); // the direct count, not the subtraction
        assert!(is_left);
    }

    #[test]
    fn rejects_narrow_rotate_without_high_bits_proof() {
        // Same shape but the source is a SEXT of an i16: bits above 16 are
        // the sign replication, not zero, so the narrow pattern must not
        // fire (the AShr half is not an LShr of the low word).
        let mut defs = narrow_rotate_defs(IrBinOp::AShr, Operand::Const(IrConst::I64(8)), v(5));
        defs[5] = Some(Instruction::Cast {
            dest: Value(5),
            src: v(4),
            from_ty: IrType::I16,
            to_ty: IrType::I32,
        });
        assert!(match_narrow_rotate(v(3), 16, &defs).is_none());
    }

    #[test]
    fn rejects_narrow_rotate_wrong_complement() {
        // 8 + 12 != 16: not complementary at the narrow width either.
        let mut defs = narrow_rotate_defs(IrBinOp::LShr, Operand::Const(IrConst::I64(12)), v(5));
        defs[5] = Some(zext_u16(5));
        assert!(match_narrow_rotate(v(3), 16, &defs).is_none());
        // 8 + 24 complements at 32, not 16: that is the WIDE rotate's shape
        // (already handled by `match_rotate`), never a narrow one.
        let mut defs = narrow_rotate_defs(IrBinOp::LShr, Operand::Const(IrConst::I64(24)), v(5));
        defs[5] = Some(zext_u16(5));
        assert!(match_narrow_rotate(v(3), 16, &defs).is_none());
    }

    #[test]
    fn rejects_narrow_rotate_differing_sources() {
        // The halves read different zexts of one root: different values, no
        // fold (the post-beb28b4c identity discipline).
        let mut defs = narrow_rotate_defs(IrBinOp::LShr, Operand::Const(IrConst::I64(8)), v(5));
        defs[5] = Some(zext_u16(5));
        defs[6] = Some(zext_u16(6));
        if let Some(Instruction::BinOp { lhs, .. }) = defs[2].as_mut() {
            *lhs = v(6);
        }
        assert!(match_narrow_rotate(v(3), 16, &defs).is_none());
    }

    #[test]
    fn high_bits_zero_accepts_mask_and_rejects_sext() {
        let mut defs = vec![None; 4];
        defs[1] = Some(Instruction::BinOp {
            dest: Value(1),
            op: IrBinOp::And,
            lhs: v(0),
            rhs: Operand::Const(IrConst::I64(0xffff)),
            ty: IrType::I32,
        });
        assert!(high_bits_zero(v(1), 16, &defs));
        assert!(!high_bits_zero(v(1), 8, &defs)); // mask covers 16, not 8
        defs[2] = Some(Instruction::Cast {
            dest: Value(2),
            src: v(0),
            from_ty: IrType::I16,
            to_ty: IrType::I32,
        });
        assert!(!high_bits_zero(v(2), 16, &defs)); // sext: not zeros above 16
        defs[3] = Some(Instruction::Cast {
            dest: Value(3),
            src: v(0),
            from_ty: IrType::U8,
            to_ty: IrType::I32,
        });
        assert!(high_bits_zero(v(3), 16, &defs)); // zext of u8: zeros above 8 ⊇ above 16
        assert!(high_bits_zero(
            Operand::Const(IrConst::I64(0x1234)),
            16,
            &defs
        ));
        assert!(!high_bits_zero(
            Operand::Const(IrConst::I64(0x12345)),
            16,
            &defs
        ));
    }
}
