//! Single-bit selection: `x | 1 << n`, `x & ~(1 << n)`, `x ^ 1 << n` and
//! `(x & 1 << n) != 0`.
//!
//! These four shapes are how portable C spells "set bit n", "clear bit n",
//! "toggle bit n" and "test bit n".  Spelled out they cost a materialized
//! constant, a variable
//! shift pinned to `%cl` and the logic op; every ISA this compiler targets has
//! a one- or two-instruction form of the *whole* idiom instead (x86
//! `bts`/`btr`/`btc`/`bt`, AArch64 `orr`/`bic`/`eor` with an `lsl`, RISC-V
//! `or`/`and`/`xor` against a shifted one).  The IR forms are
//! [`IrBinOp::BitSet`], [`IrBinOp::BitClear`], [`IrBinOp::BitToggle`] and the
//! already-existing [`IrBinOp::BitTest`], whose `index mod W` contract all
//! four share.
//!
//! Every rewrite here replaces only the instruction it matches and leaves the
//! producers (`1 << n`, and the `~` above it) to DCE, which is why each one
//! requires those producers to be single-use.  Nothing repurposes an earlier
//! instruction, so the fold needs no coherence bookkeeping: `defs` and
//! `use_counts` are read, never written.
//!
//! What is deliberately *not* matched, because the IR cannot express it more
//! cheaply than the shape already is:
//!
//! * `(x & 1 << n) == 0`, the equality direction of the test.  `BitTest` is
//!   defined as a 0/1 boolean, so the negated value needs a second instruction
//!   (`xor $1`) that the `setnc` GCC and Clang emit does not; folding the
//!   producer would still leave the compare.  The inequality direction folds
//!   because `BitTest`'s 0/1 result *is* the answer.
//! * `x ^ 1u << n` with the shifted operand used twice, and the seven other
//!   spellings the controls in `check_bit_select_fold.sh` pin.  The single-use
//!   rule is what keeps a toggle from duplicating the shift, exactly as it does
//!   for `BitSet`/`BitClear`.

use super::*;
use crate::ir::reexports::Terminator;

/// The width an IR bit operation may be created at.  The 128-bit pair type has
/// no lowering, and on a 32-bit target neither does a 64-bit operation (its
/// "native" width is a register pair the bit emitters do not handle) -- the
/// same legality rule `simplify`'s `BitTest` producer applies, kept here in one
/// place for the four shapes below.
///
/// The backends also lower 8/16-bit bit operations (the C frontend never asks
/// for one, because the integer promotions widen both operands to `int`, but
/// the IR contract allows them and the ARM/RISC-V narrow paths are tested
/// directly).  This recognizer only creates what its own shapes produce, so the
/// narrow widths are simply out of reach here.
fn native_bit_width(ty: IrType) -> bool {
    ty.is_integer()
        && !ty.is_128bit()
        && (ty.size() <= 4 || !crate::common::types::target_is_32bit())
}

/// Whether `ty` is an ordinary 8..64-bit integer type a cast can be read
/// through: the 128-bit pair types and pointers are never part of a mask.
fn plain_int(ty: IrType) -> bool {
    ty.is_integer() && !ty.is_128bit()
}

/// The index operand of a bit operation, read through copies and integer
/// casts: `index mod W` (W <= 64) only reads the index's low six bits, which
/// every integer cast preserves, so `BitTest(x, (int)n)` and `BitClear(x, n)`
/// name the same bit -- and only identical operands let the x86 backend fuse
/// the pair into one `btr`.  On a 32-bit target the walk stops before a
/// 64-bit producer: that value is a register pair, which no backend's bit
/// operation reads as an index.
pub(super) fn peel_index(op: Operand, defs: &[Option<Instruction>]) -> Operand {
    let widest = if crate::common::types::target_is_32bit() {
        4
    } else {
        8
    };
    let mut op = op;
    for _ in 0..32 {
        let Operand::Value(v) = op else {
            break;
        };
        match defs.get(v.0 as usize).and_then(Option::as_ref) {
            Some(Instruction::Copy { src, .. }) => op = *src,
            Some(Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            }) if plain_int(*from_ty) && plain_int(*to_ty) && from_ty.size() <= widest => {
                op = *src;
            }
            _ => break,
        }
    }
    op
}

/// A proven upper bound on an index operand, read from its producer: a
/// non-negative constant, `i & M`, or `i >> s` (logical).  Copies and integer
/// casts are crossed.
///
/// Every bound returned is below `2^(w-1)` for the producer's width `w`, i.e.
/// the value is provably non-negative there, so a sign-extending cast keeps it
/// exactly as a zero-extending one does; a narrowing cast keeps it only when
/// it still fits non-negatively in the destination width.  `None` means
/// "unknown", never "unbounded but valid".
pub(super) fn index_upper_bound(op: Operand, defs: &[Option<Instruction>]) -> Option<u64> {
    index_upper_bound_depth(op, defs, 0)
}

fn index_upper_bound_depth(op: Operand, defs: &[Option<Instruction>], depth: u32) -> Option<u64> {
    if depth > 16 {
        return None;
    }
    let v = match op {
        Operand::Const(c) => {
            let n = c.to_i64()?;
            return (0..=i64::from(i32::MAX)).contains(&n).then_some(n as u64);
        }
        Operand::Value(v) => v,
    };
    match defs.get(v.0 as usize).and_then(Option::as_ref)? {
        Instruction::Copy { src, .. } => index_upper_bound_depth(*src, defs, depth + 1),
        Instruction::Cast {
            src,
            from_ty,
            to_ty,
            ..
        } if plain_int(*from_ty) && plain_int(*to_ty) => {
            let bound = index_upper_bound_depth(*src, defs, depth + 1)?;
            let to_bits = to_ty.size() * 8;
            (bound < 1u64 << (to_bits - 1)).then_some(bound)
        }
        Instruction::BinOp {
            op: IrBinOp::And,
            lhs,
            rhs,
            ty,
            ..
        } if plain_int(*ty) => {
            let bits = ty.size() * 8;
            let mask = const_in_ring(*rhs, bits).or_else(|| const_in_ring(*lhs, bits))?;
            (mask < 1u64 << (bits - 1)).then_some(mask)
        }
        Instruction::BinOp {
            op: IrBinOp::LShr,
            rhs,
            ty,
            ..
        } if plain_int(*ty) => {
            let bits = ty.size() * 8;
            let shift = const_in_ring(*rhs, bits)?;
            (1..bits as u64)
                .contains(&shift)
                .then(|| (1u64 << (bits as u64 - shift)) - 1)
        }
        _ => None,
    }
}

/// `op` read as a one-hot mask `1 << n` at `bits` width.  Returns the index
/// operand `n` (the shift count, read through integer casts because `index mod
/// W` survives all of them -- see below) and records every value on the path in
/// `chain`, so the caller can require each one to die with its consumer.
///
/// The path may cross copies and integer casts between the consumer and the
/// shift:
///
/// * a same-width cast (`I32 <-> U32`, the C frontend's `(unsigned)(1 << n)`)
///   reinterprets the same bit pattern and is always transparent;
/// * a widening cast keeps the single bit at the same position only when the
///   bit is below the source width -- and, for a sign extension, below the
///   source's sign bit, which would otherwise smear into every new bit (the
///   `unsigned long x | (1 << n)` control with an `int` shift must keep its
///   `cltq`);
/// * a narrowing cast keeps it only when the bit is below the destination
///   width (`(u8)(1 << (i & 7))`).
///
/// Each width-changing cast therefore demands a proof that the count is below
/// a limit; [`index_upper_bound`] supplies it or the match fails.  With a
/// proven count below every crossed width, `n mod W` is `n` at the shift's
/// width and at the consumer's, so the bit operation's own `index mod W`
/// contract reads the same bit.
pub(super) fn one_hot_at(
    op: Operand,
    bits: usize,
    defs: &[Option<Instruction>],
    chain: &mut Vec<u32>,
) -> Option<Operand> {
    let mut op = op;
    let mut bits = bits;
    // Exclusive upper bound the count must provably satisfy, set by the
    // tightest width-changing cast crossed so far.
    let mut limit: Option<u64> = None;
    for _ in 0..16 {
        let Operand::Value(v) = op else {
            return None;
        };
        chain.push(v.0);
        match defs.get(v.0 as usize).and_then(Option::as_ref)? {
            Instruction::Copy { src, .. } => op = *src,
            Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            } if plain_int(*from_ty) && plain_int(*to_ty) => {
                let from_bits = from_ty.size() * 8;
                if to_ty.size() * 8 != bits {
                    return None;
                }
                let need = if from_bits < bits {
                    // A sign extension copies bit `from_bits - 1` upwards.
                    Some(from_bits - usize::from(from_ty.is_signed()))
                } else if from_bits > bits {
                    Some(bits)
                } else {
                    None
                };
                if let Some(need) = need {
                    let need = need as u64;
                    limit = Some(limit.map_or(need, |l| l.min(need)));
                }
                bits = from_bits;
                op = *src;
            }
            Instruction::BinOp {
                op: IrBinOp::Shl,
                lhs,
                rhs,
                ty,
                ..
            } if plain_int(*ty) && ty.size() * 8 == bits => {
                if const_in_ring(*lhs, bits) != Some(1) {
                    return None;
                }
                if let Some(limit) = limit {
                    if index_upper_bound(*rhs, defs)? >= limit {
                        return None;
                    }
                }
                return Some(peel_index(*rhs, defs));
            }
            _ => return None,
        }
    }
    None
}

/// `~(1 << n)` at `bits` width, as [`one_hot_at`] reads the mask underneath.
/// Both spellings of the complement are accepted: `UnaryOp::Not` (what the
/// frontend emits for `~`) and `Xor` with an all-ones constant (what
/// reassociation can leave).  Copies and same-width casts above the complement
/// are crossed too (`(unsigned)~(1 << n)`).
pub(super) fn complement_of_one_hot(
    op: Operand,
    bits: usize,
    defs: &[Option<Instruction>],
    chain: &mut Vec<u32>,
) -> Option<Operand> {
    let mut op = op;
    for _ in 0..16 {
        let Operand::Value(v) = op else {
            return None;
        };
        chain.push(v.0);
        match defs.get(v.0 as usize).and_then(Option::as_ref)? {
            Instruction::Copy { src, .. } => op = *src,
            Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            } if plain_int(*from_ty)
                && plain_int(*to_ty)
                && from_ty.size() == to_ty.size()
                && to_ty.size() * 8 == bits =>
            {
                op = *src;
            }
            Instruction::UnaryOp {
                op: IrUnaryOp::Not,
                src,
                ty,
                ..
            } if plain_int(*ty) && ty.size() * 8 == bits => {
                return one_hot_at(*src, bits, defs, chain);
            }
            Instruction::BinOp {
                op: IrBinOp::Xor,
                lhs,
                rhs,
                ty,
                ..
            } if plain_int(*ty) && ty.size() * 8 == bits => {
                let all_ones = u64::MAX >> (64 - bits);
                let inner = if const_in_ring(*rhs, bits) == Some(all_ones) {
                    *lhs
                } else if const_in_ring(*lhs, bits) == Some(all_ones) {
                    *rhs
                } else {
                    return None;
                };
                return one_hot_at(inner, bits, defs, chain);
            }
            _ => return None,
        }
    }
    None
}

/// Every value on a matched path is used exactly once, i.e. the whole path
/// dies when the consumer stops reading it.  `uses` holds the running counts
/// of the scan, so a use by an instruction already rewritten is gone.
fn chain_dies(chain: &[u32], uses: &[u32]) -> bool {
    !chain.is_empty() && chain.iter().all(|id| uses.get(*id as usize) == Some(&1))
}

/// Pick the one-hot side of a two-operand instruction at `bits` width,
/// returning the other operand (`base`) and the index.  Both operand orders
/// are accepted; a shape where *both* sides are one-hot is left alone, because
/// either rewrite would then have to duplicate the other shift.  The whole mask
/// path must die with the consumer.
fn split_one_hot(
    lhs: Operand,
    rhs: Operand,
    bits: usize,
    defs: &[Option<Instruction>],
    uses: &[u32],
) -> Option<(Operand, Operand)> {
    let mut lhs_chain = Vec::new();
    let mut rhs_chain = Vec::new();
    let (base, index, chain) = match (
        one_hot_at(lhs, bits, defs, &mut lhs_chain),
        one_hot_at(rhs, bits, defs, &mut rhs_chain),
    ) {
        (Some(index), None) => (rhs, index, lhs_chain),
        (None, Some(index)) => (lhs, index, rhs_chain),
        _ => return None,
    };
    chain_dies(&chain, uses).then_some((base, index))
}

/// The complement twin of [`split_one_hot`]: `x & ~(1 << n)` in either order.
fn split_complement(
    lhs: Operand,
    rhs: Operand,
    bits: usize,
    defs: &[Option<Instruction>],
    uses: &[u32],
) -> Option<(Operand, Operand)> {
    let mut lhs_chain = Vec::new();
    let mut rhs_chain = Vec::new();
    let (base, index, chain) = match (
        complement_of_one_hot(lhs, bits, defs, &mut lhs_chain),
        complement_of_one_hot(rhs, bits, defs, &mut rhs_chain),
    ) {
        (Some(index), None) => (rhs, index, lhs_chain),
        (None, Some(index)) => (lhs, index, rhs_chain),
        _ => return None,
    };
    chain_dies(&chain, uses).then_some((base, index))
}

/// Drop a redundant `& M` from a bit operation's index.  The four operations
/// read `index mod W`, which only depends on the low `log2(W)` bits, so an
/// index of `i & M` with `M` covering `W - 1` reads the same bit as `i` itself
/// (brotli's `1u << (i & 31)`, libsrtp's `(bit) & 31`, `bitNum % 64`).  Copies
/// and integer casts between the mask and the operation are crossed for the
/// same reason the count casts are: none of them changes the low six bits.
///
/// Only a mask that dies here is dropped -- a mask with other readers stays
/// live anyway, and reading its input instead would just lengthen that input's
/// live range for nothing.
pub(super) fn strip_index_mask(
    index: Operand,
    bits: usize,
    defs: &[Option<Instruction>],
    uses: &[u32],
) -> Operand {
    let low = (bits - 1) as u64;
    let mut op = index;
    for _ in 0..8 {
        let Operand::Value(v) = op else {
            return index;
        };
        if uses.get(v.0 as usize) != Some(&1) {
            return index;
        }
        match defs.get(v.0 as usize).and_then(Option::as_ref) {
            Some(Instruction::Copy { src, .. }) => op = *src,
            Some(Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            }) if plain_int(*from_ty) && plain_int(*to_ty) => op = *src,
            Some(Instruction::BinOp {
                op: IrBinOp::And,
                lhs,
                rhs,
                ty,
                ..
            }) if plain_int(*ty) => {
                let width = ty.size() * 8;
                let (other, mask) = match (const_in_ring(*rhs, width), const_in_ring(*lhs, width)) {
                    (Some(mask), _) => (*lhs, mask),
                    (None, Some(mask)) => (*rhs, mask),
                    _ => return index,
                };
                // The mask must keep every index bit the operation reads.  Its
                // own width never matters: the narrowest integer has eight
                // bits, which already holds the six an index can need.
                return if mask & low == low {
                    peel_index(other, defs)
                } else {
                    index
                };
            }
            _ => return index,
        }
    }
    index
}

/// Whether every bit `op` can have at `bits` width lies inside `mask`: a
/// zero-extension from a narrower unsigned type, an `& C'` with `C'` inside
/// `mask`, or such a constant, read through copies and same-width casts.
fn bits_within(op: Operand, mask: u64, bits: usize, defs: &[Option<Instruction>]) -> bool {
    let mut op = op;
    for _ in 0..16 {
        match op {
            Operand::Const(_) => {
                return const_in_ring(op, bits).is_some_and(|c| c & !mask == 0);
            }
            Operand::Value(v) => match defs.get(v.0 as usize).and_then(Option::as_ref) {
                Some(Instruction::Copy { src, .. }) => op = *src,
                Some(Instruction::Cast {
                    src,
                    from_ty,
                    to_ty,
                    ..
                }) if plain_int(*from_ty) && plain_int(*to_ty) && to_ty.size() * 8 == bits => {
                    let from_bits = from_ty.size() * 8;
                    if from_bits == bits {
                        op = *src;
                    } else if from_bits < bits && from_ty.is_unsigned() {
                        let possible = u64::MAX >> (64 - from_bits);
                        return possible & !mask == 0;
                    } else {
                        return false;
                    }
                }
                Some(Instruction::BinOp {
                    op: IrBinOp::And,
                    lhs,
                    rhs,
                    ty,
                    ..
                }) if plain_int(*ty) && ty.size() * 8 == bits => {
                    return const_in_ring(*rhs, bits)
                        .or_else(|| const_in_ring(*lhs, bits))
                        .is_some_and(|c| c & !mask == 0);
                }
                _ => return false,
            },
        }
    }
    false
}

/// `x & BitClear(C, k)` -> `BitClear(x, k)` when every bit `x` can have is
/// already inside `C`: then `x & C` is `x`, and clearing bit `k` of the mask
/// clears it in the result.  usrsctp's `arry[gap >> 3] &= (~(1 << (gap & 7)))
/// & 0xff` reaches the IR as `zext(byte) & BitClear(255, k)` once the inner
/// clear has folded, which materialized `255`, cleared a bit in it and ANDed
/// -- three instructions for one `btr`.
fn fold_and_of_constant_clear(
    dest: Value,
    lhs: Operand,
    rhs: Operand,
    ty: IrType,
    defs: &[Option<Instruction>],
    uses: &[u32],
) -> Option<Instruction> {
    let bits = ty.size() * 8;
    for (clear, other) in [(lhs, rhs), (rhs, lhs)] {
        let Operand::Value(cv) = clear else {
            continue;
        };
        let Some(Instruction::BinOp {
            op: IrBinOp::BitClear,
            lhs: constant,
            rhs: index,
            ty: clear_ty,
            ..
        }) = defs.get(cv.0 as usize).and_then(Option::as_ref)
        else {
            continue;
        };
        if clear_ty.size() != ty.size() || uses.get(cv.0 as usize) != Some(&1) {
            continue;
        }
        let Some(constant) = const_in_ring(*constant, bits) else {
            continue;
        };
        if matches!(other, Operand::Const(_)) || !bits_within(other, constant, bits, defs) {
            continue;
        }
        return Some(Instruction::BinOp {
            dest,
            op: IrBinOp::BitClear,
            lhs: other,
            rhs: *index,
            ty,
        });
    }
    None
}

/// The inner base of `(x ^ m) ^ m`, i.e. `x` when `base` is a `BitToggle` (or
/// the pre-fold `Xor`) of the same index operand.  Used to keep the toggle's
/// involution from being *worse* than the shape it replaces: the two masks in
/// `x ^ (1u << n) ^ (1u << n)` are the same CSE'd value, so the outer XOR here
/// is `BitToggle(BitToggle(x, n), n)` and its answer is `x`.
fn toggle_involution(
    base: Operand,
    index: Operand,
    defs: &[Option<Instruction>],
) -> Option<Operand> {
    let Operand::Value(v) = peel_copies(base, defs) else {
        return None;
    };
    let (inner_base, inner_index) = match defs.get(v.0 as usize).and_then(Option::as_ref)? {
        Instruction::BinOp {
            op: IrBinOp::BitToggle,
            lhs,
            rhs,
            ..
        } => (*lhs, *rhs),
        _ => return None,
    };
    (peel_copies(inner_index, defs) == peel_copies(index, defs)).then_some(inner_base)
}

fn bit_op(dest: Value, op: IrBinOp, lhs: Operand, rhs: Operand, ty: IrType) -> Instruction {
    Instruction::BinOp {
        dest,
        op,
        lhs,
        rhs,
        ty,
    }
}

pub(super) fn fold_bit_select(
    inst: &Instruction,
    defs: &[Option<Instruction>],
    uses: &[u32],
) -> Option<Instruction> {
    match inst {
        // `x | (1 << n)` -> `BitSet(x, n)`
        Instruction::BinOp {
            dest,
            op: IrBinOp::Or,
            lhs,
            rhs,
            ty,
        } if native_bit_width(*ty) => {
            let bits = ty.size() * 8;
            let (base, index) = split_one_hot(*lhs, *rhs, bits, defs, uses)?;
            let index = strip_index_mask(index, bits, defs, uses);
            Some(bit_op(*dest, IrBinOp::BitSet, base, index, *ty))
        }
        // `x & ~(1 << n)` -> `BitClear(x, n)`, and `x & BitClear(C, k)` ->
        // `BitClear(x, k)` when `x` has no bit outside `C`.
        Instruction::BinOp {
            dest,
            op: IrBinOp::And,
            lhs,
            rhs,
            ty,
        } if native_bit_width(*ty) => {
            let bits = ty.size() * 8;
            if let Some((base, index)) = split_complement(*lhs, *rhs, bits, defs, uses) {
                let index = strip_index_mask(index, bits, defs, uses);
                return Some(bit_op(*dest, IrBinOp::BitClear, base, index, *ty));
            }
            fold_and_of_constant_clear(*dest, *lhs, *rhs, *ty, defs, uses)
        }
        // `x ^ (1 << n)` -> `BitToggle(x, n)`
        //
        // The third member of the family: `btc` on x86, `eor` behind an `lsl`
        // elsewhere.  Accepted in both operand orders like `BitSet`, and the
        // shift must die here too.
        Instruction::BinOp {
            dest,
            op: IrBinOp::Xor,
            lhs,
            rhs,
            ty,
        } if native_bit_width(*ty) => {
            let bits = ty.size() * 8;
            let (base, index) = split_one_hot(*lhs, *rhs, bits, defs, uses)?;
            // `(x ^ m) ^ m` is `x`, and the toggle is its own inverse.  The
            // earlier XOR of that pair has usually been folded to a `BitToggle`
            // by the time this one is reached, and the pass that would cancel
            // `a ^ a` sees two different operations afterwards -- so folding
            // BOTH would turn a 2-instruction identity into two `btc`s
            // (`x ^ (1u << n) ^ (1u << n)` measured 2 before this fold and 5
            // after).  Catch it here instead: the result is a plain copy of the
            // inner base, which DCE turns into the identity.
            if let Some(inner_base) = toggle_involution(base, index, defs) {
                return Some(Instruction::Copy {
                    dest: *dest,
                    src: inner_base,
                });
            }
            let index = strip_index_mask(index, bits, defs, uses);
            Some(bit_op(*dest, IrBinOp::BitToggle, base, index, *ty))
        }
        // `(x & (1 << n)) != 0` -> `BitTest(x, n)`
        //
        // The whole comparison is replaced: `BitTest`'s value is the 0/1 the
        // comparison computes, so nothing downstream changes type or carrier.
        // (`fold_bit_tests` rewrites the AND itself whenever every reader is a
        // zero test; this arm keeps covering a comparison some other rewrite of
        // this same scan exposes.)
        Instruction::Cmp {
            dest,
            op: IrCmpOp::Ne,
            lhs,
            rhs,
            ty,
        } if native_bit_width(*ty) => {
            let bits = ty.size() * 8;
            let tested = if const_in_ring(*rhs, bits) == Some(0) {
                *lhs
            } else if const_in_ring(*lhs, bits) == Some(0) {
                *rhs
            } else {
                return None;
            };
            let mut and_chain = Vec::new();
            let v = peel_copies_recording(tested, defs, &mut and_chain)?;
            let Some(Instruction::BinOp {
                op: IrBinOp::And,
                lhs: and_lhs,
                rhs: and_rhs,
                ty: and_ty,
                ..
            }) = defs.get(v.0 as usize).and_then(Option::as_ref)
            else {
                return None;
            };
            if and_ty.size() != ty.size() || !chain_dies(&and_chain, uses) {
                return None;
            }
            let (base, index) = split_one_hot(*and_lhs, *and_rhs, bits, defs, uses)?;
            let index = strip_index_mask(index, bits, defs, uses);
            Some(bit_op(*dest, IrBinOp::BitTest, base, index, *ty))
        }
        // An existing bit operation (built by this fold earlier, by the
        // shared-mask pass, or by `simplify`'s `(x >> i) & 1`) whose index
        // still carries a redundant `& (W - 1)`.
        Instruction::BinOp {
            dest,
            op: op @ (IrBinOp::BitSet | IrBinOp::BitClear | IrBinOp::BitToggle | IrBinOp::BitTest),
            lhs,
            rhs,
            ty,
        } if native_bit_width(*ty) => {
            let index = peel_index(strip_index_mask(*rhs, ty.size() * 8, defs, uses), defs);
            (index != *rhs).then(|| bit_op(*dest, *op, *lhs, index, *ty))
        }
        _ => None,
    }
}

/// `peel_copies`, recording every value it visits (the operand's own value
/// included) so the caller can require the whole copy path to die.
pub(super) fn peel_copies_recording(
    op: Operand,
    defs: &[Option<Instruction>],
    chain: &mut Vec<u32>,
) -> Option<Value> {
    let mut op = op;
    for _ in 0..32 {
        let Operand::Value(v) = op else {
            return None;
        };
        chain.push(v.0);
        match defs.get(v.0 as usize).and_then(Option::as_ref) {
            Some(Instruction::Copy { src, .. }) => op = *src,
            _ => return Some(v),
        }
    }
    None
}

/// Target-aware shared-mask selection.
///
/// The single-use matcher above deliberately refuses a one-hot mask that has
/// several readers: rewriting just one reader can leave the mask live and turn
/// a cheap logic op into a bit instruction without a net win.  On x86, however,
/// `bts`/`btr`/`btc`/`bt` consume the index directly.  If *every* reader of the
/// mask (through copies, complements, and test-ands) is a member of this family,
/// the whole group can be rewritten atomically: the mask and its internal
/// producers die, and no reader is left paying for a materialized mask.
///
/// This is intentionally not done on AArch64/RISC-V: their lowerings rebuild a
/// one-hot mask for each bit operation, so sharing the mask is the better shape.
/// The pass is all-or-nothing per mask.  One escaping store, call, phi, inline
/// asm, or non-bit reader refuses the entire group rather than duplicating work.
pub(super) fn fold_shared_bit_select(func: &mut IrFunction) -> usize {
    use crate::common::types::target_arch_name;

    if !matches!(target_arch_name(), "x86-64" | "i686") {
        return 0;
    }
    let max_id = func.max_value_id();
    if func.next_value_id == u32::MAX || max_id > 1_000_000 {
        return 0;
    }
    let len = max_id as usize + 1;
    let mut def_loc = vec![None; len];
    let mut ambiguous = vec![false; len];
    let mut uses = vec![Vec::new(); len];
    let mut candidates = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            let loc = SharedLoc {
                block: bi,
                inst: Some(ii),
            };
            if let Some(dest) = inst.dest() {
                let id = dest.0 as usize;
                if def_loc[id].is_some() {
                    ambiguous[id] = true;
                } else {
                    def_loc[id] = Some(loc);
                }
            }
            crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    uses[v.0 as usize].push(loc);
                }
            });
            crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                uses[v.0 as usize].push(loc);
            });
            if let Instruction::BinOp {
                dest,
                op: IrBinOp::Shl,
                lhs,
                rhs,
                ty,
            } = inst
            {
                if native_bit_width(*ty)
                    && const_in_ring(*lhs, ty.size() * 8) == Some(1)
                    && !ambiguous[dest.0 as usize]
                {
                    let index = forward_shared_index(*rhs, func, &def_loc);
                    candidates.push(SharedCandidate {
                        value: dest.0,
                        index,
                        ty: *ty,
                    });
                }
            }
        }
        crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                uses[v.0 as usize].push(SharedLoc {
                    block: bi,
                    inst: None,
                });
            }
        });
    }

    let mut changes = 0;
    for cand in candidates {
        if ambiguous[cand.value as usize] {
            continue;
        }
        let Some(rewrites) = analyze_shared_candidate(func, &uses, &ambiguous, &cand) else {
            continue;
        };
        for (loc, inst) in rewrites {
            let Some(ii) = loc.inst else { continue };
            func.blocks[loc.block].instructions[ii] = inst;
            changes += 1;
        }
    }
    changes
}

/// Where a value is read: an instruction of a block, or (`inst == None`) the
/// block's terminator.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) struct SharedLoc {
    pub(super) block: usize,
    pub(super) inst: Option<usize>,
}

#[derive(Clone, Copy)]
struct SharedCandidate {
    value: u32,
    index: Operand,
    ty: IrType,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SharedKind {
    Mask,
    Complement,
}

/// [`peel_index`] over the group pass's location tables (the same walk and
/// the same 32-bit-target stop before a 64-bit pair producer).
fn forward_shared_index(
    mut op: Operand,
    func: &IrFunction,
    def_loc: &[Option<SharedLoc>],
) -> Operand {
    let widest = if crate::common::types::target_is_32bit() {
        4
    } else {
        8
    };
    for _ in 0..32 {
        let Operand::Value(v) = op else { break };
        let next = def_loc.get(v.0 as usize).and_then(|o| *o).and_then(|l| {
            let ii = l.inst?;
            match &func.blocks[l.block].instructions[ii] {
                Instruction::Cast {
                    src,
                    from_ty,
                    to_ty,
                    ..
                } if plain_int(*from_ty) && plain_int(*to_ty) && from_ty.size() <= widest => {
                    Some(*src)
                }
                Instruction::Copy { src, .. } => Some(*src),
                _ => None,
            }
        });
        match next {
            Some(src) => op = src,
            None => break,
        }
    }
    op
}

fn shared_group_operand(op: Operand, kind: &[Option<SharedKind>]) -> bool {
    match op {
        Operand::Value(v) => kind.get(v.0 as usize).and_then(|k| *k).is_some(),
        Operand::Const(_) => false,
    }
}

fn names(op: &Operand, v: u32) -> bool {
    matches!(op, Operand::Value(x) if x.0 == v)
}

/// Whether every reader of `v` -- through copies -- only asks whether it is
/// zero: `Cmp Eq/Ne` against a zero constant at `v`'s own width, a
/// `CondBranch` on it, or a `Select` that branches on it without also
/// returning it.  Each of those readers gets the same answer from ANY value
/// that is zero exactly when `v` is, which is what lets `x & (1 << n)` become
/// `BitTest(x, n)` in place without touching one reader.
///
/// A compare at another width is refused: a narrower one reads only the low
/// part of `v`, where a high one-hot bit is invisible but the 0/1 `BitTest`
/// is not.
pub(super) fn only_zero_tested(
    v: u32,
    bits: usize,
    func: &IrFunction,
    uses: &[Vec<SharedLoc>],
) -> bool {
    let mut stack = vec![v];
    let mut seen = 0usize;
    while let Some(cur) = stack.pop() {
        seen += 1;
        if seen > 64 {
            return false;
        }
        let Some(readers) = uses.get(cur as usize) else {
            return false;
        };
        for loc in readers {
            let Some(block) = func.blocks.get(loc.block) else {
                return false;
            };
            let Some(ii) = loc.inst else {
                match &block.terminator {
                    Terminator::CondBranch { cond, .. } if names(cond, cur) => continue,
                    _ => return false,
                }
            };
            match &block.instructions[ii] {
                Instruction::Copy { dest, src } if names(src, cur) => stack.push(dest.0),
                Instruction::Cmp {
                    op: IrCmpOp::Eq | IrCmpOp::Ne,
                    lhs,
                    rhs,
                    ty,
                    ..
                } if plain_int(*ty) && ty.size() * 8 == bits => {
                    let zero_other = if names(lhs, cur) && !names(rhs, cur) {
                        const_in_ring(*rhs, bits) == Some(0)
                    } else if names(rhs, cur) && !names(lhs, cur) {
                        const_in_ring(*lhs, bits) == Some(0)
                    } else {
                        false
                    };
                    if !zero_other {
                        return false;
                    }
                }
                Instruction::Select {
                    cond,
                    true_val,
                    false_val,
                    ..
                } if names(cond, cur) && !names(true_val, cur) && !names(false_val, cur) => {}
                _ => return false,
            }
        }
    }
    true
}

/// Prove that every reader of `cand` is a bit-family consumer, and return the
/// consumer rewrites.  Internal copies, same-width casts and complements are
/// closed over; any escaping or non-bit reader returns `None` for the whole
/// group.
///
/// A test-and (`x & mask`) whose readers only compare it with zero is
/// rewritten IN PLACE to `BitTest(x, n)`: it is zero exactly when the bit is
/// clear, so its `!= 0` / `== 0` / branch / select readers stay valid as they
/// are (`simplify` then retires the `!= 0`, and the backend fuses `bt` into
/// the branch, `setnc` or `cmov`).
fn analyze_shared_candidate(
    func: &IrFunction,
    uses: &[Vec<SharedLoc>],
    ambiguous: &[bool],
    cand: &SharedCandidate,
) -> Option<Vec<(SharedLoc, Instruction)>> {
    use crate::common::fx_hash::FxHashMap;

    let len = ambiguous.len();
    let width = cand.ty.size();
    let bits = width * 8;
    let all_ones = u64::MAX >> (64 - bits);
    let mut kind = vec![None; len];
    let mut queue = vec![cand.value];
    kind[cand.value as usize] = Some(SharedKind::Mask);
    let mut rewrites: FxHashMap<SharedLoc, Instruction> = FxHashMap::default();

    while let Some(v) = queue.pop() {
        let k = kind[v as usize]?;
        for &loc in &uses[v as usize] {
            let ii = loc.inst?;
            let inst = &func.blocks[loc.block].instructions[ii];
            if rewrites.contains_key(&loc) {
                return None;
            }
            macro_rules! enqueue {
                ($dest:expr, $new_kind:expr) => {{
                    let dest: Value = $dest;
                    let new_kind: SharedKind = $new_kind;
                    let id = dest.0 as usize;
                    if ambiguous[id] {
                        return None;
                    }
                    match kind[id] {
                        Some(existing) if existing != new_kind => return None,
                        Some(_) => {}
                        None => {
                            kind[id] = Some(new_kind);
                            queue.push(dest.0);
                        }
                    }
                }};
            }
            // The other operand of a two-operand reader of `v`; a reader that
            // names `v` twice (`m | m`) is not a bit-family consumer.
            let other_of = |lhs: &Operand, rhs: &Operand| -> Option<Operand> {
                match (names(lhs, v), names(rhs, v)) {
                    (true, false) => Some(*rhs),
                    (false, true) => Some(*lhs),
                    _ => None,
                }
            };
            match (k, inst) {
                (_, Instruction::Copy { dest, src }) if names(src, v) => enqueue!(*dest, k),
                // `(unsigned)(1 << n)`: the same bit pattern under another
                // signedness is the same mask (or complement).
                (
                    _,
                    Instruction::Cast {
                        dest,
                        src,
                        from_ty,
                        to_ty,
                    },
                ) if names(src, v)
                    && plain_int(*from_ty)
                    && plain_int(*to_ty)
                    && from_ty.size() == width
                    && to_ty.size() == width =>
                {
                    enqueue!(*dest, k)
                }
                (
                    SharedKind::Mask,
                    Instruction::UnaryOp {
                        dest,
                        op: IrUnaryOp::Not,
                        src,
                        ty,
                    },
                ) if plain_int(*ty) && ty.size() == width && names(src, v) => {
                    enqueue!(*dest, SharedKind::Complement);
                }
                (
                    SharedKind::Mask,
                    Instruction::BinOp {
                        dest,
                        op: IrBinOp::Xor,
                        lhs,
                        rhs,
                        ty,
                    },
                ) if plain_int(*ty) && ty.size() == width => {
                    let other = other_of(lhs, rhs)?;
                    if const_in_ring(other, bits) == Some(all_ones) {
                        enqueue!(*dest, SharedKind::Complement);
                    } else if !shared_group_operand(other, &kind) {
                        rewrites.insert(
                            loc,
                            bit_op(*dest, IrBinOp::BitToggle, other, cand.index, *ty),
                        );
                    } else {
                        return None;
                    }
                }
                (
                    SharedKind::Mask,
                    Instruction::BinOp {
                        dest,
                        op: IrBinOp::And,
                        lhs,
                        rhs,
                        ty,
                    },
                ) if plain_int(*ty) && ty.size() == width => {
                    let other = other_of(lhs, rhs)?;
                    if shared_group_operand(other, &kind)
                        || !only_zero_tested(dest.0, bits, func, uses)
                    {
                        return None;
                    }
                    rewrites.insert(loc, bit_op(*dest, IrBinOp::BitTest, other, cand.index, *ty));
                }
                (
                    SharedKind::Mask,
                    Instruction::BinOp {
                        dest,
                        op: IrBinOp::Or,
                        lhs,
                        rhs,
                        ty,
                    },
                ) if plain_int(*ty) && ty.size() == width => {
                    let other = other_of(lhs, rhs)?;
                    if shared_group_operand(other, &kind) {
                        return None;
                    }
                    rewrites.insert(loc, bit_op(*dest, IrBinOp::BitSet, other, cand.index, *ty));
                }
                (
                    SharedKind::Complement,
                    Instruction::BinOp {
                        dest,
                        op: IrBinOp::And,
                        lhs,
                        rhs,
                        ty,
                    },
                ) if plain_int(*ty) && ty.size() == width => {
                    let other = other_of(lhs, rhs)?;
                    if shared_group_operand(other, &kind) {
                        return None;
                    }
                    rewrites.insert(
                        loc,
                        bit_op(*dest, IrBinOp::BitClear, other, cand.index, *ty),
                    );
                }
                _ => return None,
            }
        }
    }
    if rewrites.len() < 2 {
        return None;
    }
    Some(rewrites.into_iter().collect())
}
