//! A count's width is the width of its operand's significant bits.
//!
//! Two facts the per-instruction simplifier cannot express, because `UnaryOp`
//! carries a single type and cannot allocate a second value:
//!
//!   * `popcountll(x) & 1` reaches the back ends as a 64-bit count masked by a
//!     32-bit `and` (the mask is the C `int` the frontend narrowed the literal
//!     to), so the "mask == 1 means parity" rule -- which insists on equal
//!     widths -- cannot fire, and the whole 64-bit scan plus `andl $1` is
//!     emitted.  The low bit of a count is the parity of the count's operand at
//!     *the operand's* width, whatever width the mask was narrowed to;
//!   * `__builtin_parity(uchar)` promotes its argument, so the count runs over
//!     `zext(x)` at 32 bits.  Popcount and parity are invariant under zero
//!     extension, so the count runs at the narrow width and is extended back.
//!
//! Both rewrites keep the value bit for bit and only ever replace a scan with a
//! cheaper one at a narrower width.  A shared count is left alone -- replacing
//! one of its two uses would put a second scan beside it -- and the extension
//! chain is only walked, never deleted.  Where a rewrite does run, the step back
//! to the consumer's type is a `Copy`: a count of an n-bit operand fits n bits,
//! so there is nothing to extend, and saying so is what lets every back end run
//! its narrow scan (see `scaled`).
use super::*;

/// The constant `op` denotes at `bits`, looking through the copies and constant
/// casts the frontend leaves behind (`& 1` arrives as a cast of an `i64`
/// literal to `i32`).
fn const_bits(cx: &Context<'_>, op: Operand, bits: usize) -> Option<u64> {
    let mask = if bits >= 64 {
        u64::MAX
    } else {
        u64::MAX >> (64 - bits)
    };
    let mut op = op;
    for _ in 0..8 {
        match op {
            Operand::Const(c) => return c.to_i128().map(|v| (v as u64) & mask),
            _ => match cx.def(op)? {
                Instruction::Copy { src, .. } => op = *src,
                Instruction::Cast {
                    src,
                    from_ty,
                    to_ty,
                    ..
                } if from_ty.is_integer() && to_ty.is_integer() => op = *src,
                _ => return None,
            },
        }
    }
    None
}

/// The narrowest source reachable from `op` along *zero* extensions, with its
/// type.  A cast from an unsigned type to a wider one is a zero extension in
/// this IR (the extension kind is the source's signedness), so every value on
/// the returned path holds the same bit pattern with leading zeros added --
/// which no count can see.
fn narrow_source(cx: &Context<'_>, op: Operand, ty: IrType) -> Option<(Operand, IrType)> {
    let mut best = None;
    let mut width = ty.size() * 8;
    let mut cur = op;
    for _ in 0..8 {
        let Some(Instruction::Cast {
            src,
            from_ty,
            to_ty,
            ..
        }) = cx.def(cur)
        else {
            break;
        };
        if !from_ty.is_unsigned() || !to_ty.is_integer() || from_ty.size() * 8 >= width {
            break;
        }
        width = from_ty.size() * 8;
        cur = *src;
        best = Some((cur, *from_ty));
    }
    best
}

/// Where narrowing a count actually buys instructions.
///
/// The rewrite is always sound; this gate is about instruction count.  The
/// result of a narrow count is a small value -- at most eight bits for a byte
/// count -- so extending it back to the consumer's width is free as long as
/// the extension is spelled as what it is, a copy (see [`scaled`]).  That
/// leaves the question of whether the narrow scan is itself shorter:
///
/// * `parity` of a byte or halfword is a shorter fold than the 32-bit one on
///   every target: two `eor`/`xor` stages less on Arm and Riscv64, and x86
///   reads the low byte's PF, so one `testb` settles a byte;
/// * `popcount` has a five-instruction multiplier form for a byte on x86, a
///   three-instruction `cnt` on Arm, and a lane-wise byte scan on Riscv64; a
///   16-bit count still runs a 32-bit scan on x86, so it buys nothing there
///   but costs nothing either;
/// * a 64-bit count halves when it runs at 32 bits.
///
/// `clz`/`ctz` are excluded: their narrow forms are the wide ones (the
/// hardware scans whatever width the register has), so narrowing would only
/// move work into the extension.
fn profitable(op: IrUnaryOp, width: usize, current: IrType) -> bool {
    let current = current.size() * 8;
    if width >= current {
        return false;
    }
    match op {
        IrUnaryOp::Parity => width <= 16,
        IrUnaryOp::Popcount => width <= 16 || (current == 64 && width <= 32),
        _ => false,
    }
}

/// `UnaryOp` at `width`, handed to `dest_ty` when the two differ.
///
/// The value is unchanged, and a count of a `width`-bit operand cannot exceed
/// `width`, so the result already fits the narrow type: the step back to the
/// consumer's width is a copy, not an extension.  Spelling it as a `Cast`
/// asked every back end to re-normalise an operand that was already clean --
/// `and w4, w19, #0xff` and a chain of `mv`s on Arm/Riscv64 -- for a value
/// that is at most 64.  A `Copy` states the same thing and costs nothing on
/// all four back ends.
fn scaled(
    dest: Value,
    fresh: Value,
    op: IrUnaryOp,
    src: Operand,
    width: IrType,
    dest_ty: IrType,
) -> Vec<Instruction> {
    if width == dest_ty {
        return vec![Instruction::UnaryOp {
            dest,
            op,
            src,
            ty: dest_ty,
        }];
    }
    vec![
        Instruction::UnaryOp {
            dest: fresh,
            op,
            src,
            ty: width,
        },
        Instruction::Copy {
            dest,
            src: Operand::Value(fresh),
        },
    ]
}

/// `and(popcount(v), 1)` is the parity of `v`.
fn masked_count(cx: &Context<'_>, inst: &Instruction, fresh: Value) -> Option<Vec<Instruction>> {
    let Instruction::BinOp {
        dest,
        op: IrBinOp::And,
        lhs,
        rhs,
        ty,
    } = inst
    else {
        return None;
    };
    if !scalar(*ty) || ty.is_float() {
        return None;
    }
    let bits = ty.size() * 8;
    let count = if const_bits(cx, *lhs, bits) == Some(1) {
        *rhs
    } else if const_bits(cx, *rhs, bits) == Some(1) {
        *lhs
    } else {
        return None;
    };
    let Operand::Value(id) = count else {
        return None;
    };
    if cx.uses.get(id.0 as usize) != Some(&1) {
        return None;
    }
    let Instruction::UnaryOp {
        op: IrUnaryOp::Popcount,
        src,
        ty: count_ty,
        ..
    } = cx.def(count)?
    else {
        return None;
    };
    if !count_ty.is_integer() {
        return None;
    }
    // The count runs at the narrowest width it can, when that width is one
    // the back end counts cheaply; otherwise it keeps its own operand.
    let narrow = narrow_source(cx, *src, *count_ty)
        .filter(|&(_, width)| profitable(IrUnaryOp::Parity, width.size() * 8, *count_ty));
    let (src, width) = narrow.unwrap_or((*src, *count_ty));
    // The simplifier's `fold_popcount_mask_to_parity` owns every same-width
    // shape (it compares sizes, not spellings: an `unsigned` count masked by an
    // `int` literal is still same-width).  Declining there keeps exactly one
    // owner per instruction; what is left for this rule is the cross-width
    // shape the simplifier cannot express.
    if width.size() == ty.size() {
        return None;
    }
    Some(scaled(*dest, fresh, IrUnaryOp::Parity, src, width, *ty))
}

/// A count of a zero-extended narrow value runs at the narrow width.
fn narrow_count(cx: &Context<'_>, inst: &Instruction, fresh: Value) -> Option<Vec<Instruction>> {
    let Instruction::UnaryOp { dest, op, src, ty } = inst else {
        return None;
    };
    if !matches!(op, IrUnaryOp::Popcount | IrUnaryOp::Parity) || !ty.is_integer() {
        return None;
    }
    let (src, width) = narrow_source(cx, *src, *ty)?;
    if !profitable(*op, width.size() * 8, *ty) {
        return None;
    }
    Some(scaled(*dest, fresh, *op, src, width, *ty))
}

pub(super) fn fold(inst: &Instruction, cx: &Context<'_>, fresh: Value) -> Option<Vec<Instruction>> {
    masked_count(cx, inst, fresh).or_else(|| narrow_count(cx, inst, fresh))
}

#[cfg(test)]
#[path = "bit_idioms_count_width_tests.rs"]
mod tests;
