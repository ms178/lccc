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
//! chain is only walked, never deleted.
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
/// The rewrite is always sound, but a narrower count only pays when the
/// extension back to the consumer's width is free and the narrow scan is
/// genuinely shorter:
///
/// * x86-64 and i686 keep integers in a 32-bit (or 64-bit) register, so a
///   value produced at `u8`/`u16` already has clean high bits and the `Cast`
///   costs nothing; ARM and Riscv64 emit a mask plus copies for it, and their
///   narrow count paths still run the full-width sequence, so a narrower scan
///   there would pay for the extension and save nothing.  (Teaching those two
///   back ends narrow counts is what would lift this gate; until then they keep
///   the promoted form, which is what the frontend already hands them.)
/// * `parity` of a narrower value is a shorter fold at any narrower width --
///   x86 reads the low byte's PF, so one `testb` settles a byte;
/// * `popcount` has a five-instruction multiplier form for a byte, and a
///   64-bit count halves when it runs at 32 bits; a 16-bit count over an
///   already-promoted value would still run the 32-bit scan, so it is left
///   alone (the extension would be pure cost).
fn profitable(op: IrUnaryOp, width: usize, current: IrType) -> bool {
    let current = current.size() * 8;
    if width >= current {
        return false;
    }
    match op {
        IrUnaryOp::Parity => true,
        IrUnaryOp::Popcount => width == 8 || (current == 64 && width <= 32),
        _ => false,
    }
}

/// Whether the back ends of the current target widen a narrow result for free.
fn free_extension() -> bool {
    use crate::common::types::target_arch_name;

    matches!(target_arch_name(), "x86-64" | "i686")
}

/// `UnaryOp` at `width`, extended to `dest_ty` when the two differ.  The
/// extension keeps the IR type-correct after the operation has been narrowed:
/// the value is unchanged, but its declared type is the width it ran at.
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
        Instruction::Cast {
            dest,
            src: Operand::Value(fresh),
            from_ty: width,
            to_ty: dest_ty,
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
    let narrow = narrow_source(cx, *src, *count_ty).filter(|&(_, width)| {
        free_extension() && profitable(IrUnaryOp::Parity, width.size() * 8, *count_ty)
    });
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
    if !free_extension() || !profitable(*op, width.size() * 8, *ty) {
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
