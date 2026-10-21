//! Exact byte provenance for portable 32/64-bit byte-swap networks.
//!
//! Each lane of the candidate word carries either zero or the 1-based index of
//! one unchanged byte of a single opaque SSA source.  Value-preserving rewrites
//! of that word keep the model exact, so it can be pushed through:
//!
//!   * `Copy`, and integer `Cast`s that are bit-identical no-ops — a cast
//!     between same-size non-floating types reinterprets the bit pattern and
//!     cannot move, drop or invent a byte;
//!   * `Shl`/`LShr` by a whole number of bytes (lanes move, zeroes enter);
//!   * `And` by a mask whose every byte is `0x00` or `0xff` (lanes drop or stay);
//!   * `Or`/`Xor` of disjoint lanes, plus `Or` of one byte with itself in the
//!     same lane, which is idempotent.
//!
//! Everything else is refused rather than approximated: arithmetic shifts,
//! sub-byte shifts, partial mask bytes, narrowing or widening casts, arithmetic
//! and memory operations are at best an opaque source and are never peeled, so
//! independently extended or truncated values stay distinct sources.  A
//! candidate is rewritten exactly when its final lane map is the reversed
//! permutation, which is the definition of `Bswap` at that width.  A map that is
//! the reversal on a *subset* of the lanes is the masked form of the same
//! operation: the bytes it permutes are exactly `Bswap(source) & mask`, and the
//! lanes it never writes are the zeroes the original network produced there.
//! Only the consuming instruction is replaced: producers, side effects and
//! shared users stay untouched, so the rewrite is safe even when the network's
//! inputs or intermediate values are read elsewhere.
//!
//! The walk is allocation-free, keeps one 8-byte lane vector per frame, and
//! charges one unit of a fixed budget per node visited, so neither pathological
//! DAG sharing nor adversarially deep chains can inflate compile time.
use super::*;

/// Lanes of the widest supported carrier (`I64`/`U64`).
const MAX_BYTES: usize = 8;
/// Nodes inspected per candidate.  A 32-bit network needs 5 nodes per term and
/// a staged 64-bit one about 40, so this leaves room for the `Copy` and `Cast`
/// chains that survive inlining while still bounding a hostile DAG.
const VISIT_BUDGET: u16 = 128;
/// Structural recursion cap.  Real networks are 3 to 15 nodes deep; this only
/// stops adversarially long `Copy`/`Cast` chains.
const MAX_DEPTH: u8 = 24;
/// Nodes one discharge may visit.  The same bound as a match: the walk follows
/// the same operand edges the matcher proved, plus the dead code feeding them.
pub(super) const DISCHARGE_BUDGET: u16 = VISIT_BUDGET;

/// A recognized byte-reversal network.
#[derive(Clone, Copy)]
pub(super) struct Match {
    /// The single opaque value whose bytes the network permutes.
    pub src: Operand,
    /// Lanes the network fills.  `None` when it fills every lane, so the value
    /// is exactly `Bswap(src)`; otherwise it is `Bswap(src) & mask`.
    pub mask: Option<u64>,
    /// Bit operations in the matched tree: its cost, and the rank that makes a
    /// merge chain resolve to its widest node.
    pub ops: u16,
}

/// The smallest network worth folding is a two-lane reversal, which needs five
/// recognised bit operations in the source.  Anything narrower is at most as
/// cheap as the tree it would replace, so `(x >> 24) & 0xff` stays a shift and
/// an `and` rather than becoming `bswap; and`.
const MIN_MASKED_OPS: u16 = 5;

struct Bytes<'a> {
    defs: &'a [Option<Instruction>],
    source: Option<Operand>,
    width: usize,
    visits: u16,
    /// Bit operations the walk interpreted: the source tree's instruction
    /// count, and therefore the cost the replacement must beat.
    ops: u16,
}

impl Bytes<'_> {
    /// Lane map of `value`, or `None` if `value` is not provably a byte
    /// permutation of a single source or the visit/depth budget runs out.
    ///
    /// `typed` is the outermost bit-identical cast the walk crossed to reach
    /// here.  When the walk ends at an opaque value it records that cast's
    /// destination as the source instead, because a source operand feeds a
    /// `Bswap` of the ring's type: peeling to the pre-cast value would produce
    /// an operand of a different IR type with the same bit pattern.
    fn walk(
        &mut self,
        value: Operand,
        depth: u8,
        typed: Option<Operand>,
    ) -> Option<[u8; MAX_BYTES]> {
        self.visits = self.visits.checked_sub(1)?;
        if depth == 0 {
            return None;
        }
        if let Some(c) = const_in_ring(value, self.width * 8) {
            return (c == 0).then_some([0; MAX_BYTES]);
        }
        let Operand::Value(v) = value else {
            return None;
        };
        let definition = self.defs.get(v.0 as usize)?.as_ref();
        match definition {
            Some(Instruction::Copy { src, .. }) => return self.walk(*src, depth - 1, typed),
            // A bit-identical cast reinterprets the same bytes; anything that
            // changes the size (or crosses the integer/float boundary) is not a
            // reinterpretation and falls through to the opaque-source arm.
            Some(Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            }) if IrType::cast_is_bitidentical_nop(*from_ty, *to_ty) => {
                return self.walk(*src, depth - 1, typed.or(Some(Operand::Value(v))));
            }
            _ => {}
        }
        if let Some(Instruction::BinOp {
            op, lhs, rhs, ty, ..
        }) = definition
        {
            // Never read a different ring as this word's byte network.
            if !ty.is_integer() || ty.size() != self.width {
                return None;
            }
            let (op, lhs, rhs) = (*op, *lhs, *rhs);
            if matches!(
                op,
                IrBinOp::Shl | IrBinOp::LShr | IrBinOp::And | IrBinOp::Or | IrBinOp::Xor
            ) {
                self.ops = self.ops.saturating_add(1);
            }
            match op {
                IrBinOp::Shl | IrBinOp::LShr => {
                    let Operand::Const(count) = rhs else {
                        return None;
                    };
                    let shift = count.to_i128()?;
                    if !(0..(self.width * 8) as i128).contains(&shift) || shift % 8 != 0 {
                        return None;
                    }
                    let input = self.walk(lhs, depth - 1, None)?;
                    let amount = shift as usize / 8;
                    let mut output = [0; MAX_BYTES];
                    if op == IrBinOp::Shl {
                        output[amount..self.width].copy_from_slice(&input[..self.width - amount]);
                    } else {
                        output[..self.width - amount].copy_from_slice(&input[amount..self.width]);
                    }
                    return Some(output);
                }
                IrBinOp::And => {
                    // A whole-byte mask keeps or drops lanes; a partial byte
                    // mixes two source bytes and is not representable.
                    let (mask, input) = match (const_in_ring(lhs, self.width * 8), rhs) {
                        (Some(mask), _) => (mask, rhs),
                        (None, rhs_const) => (const_in_ring(rhs_const, self.width * 8)?, lhs),
                    };
                    let input = self.walk(input, depth - 1, None)?;
                    let mut output = [0; MAX_BYTES];
                    for i in 0..self.width {
                        if mask >> (8 * i) & 0xff == 0xff {
                            output[i] = input[i];
                        } else if mask >> (8 * i) & 0xff != 0 {
                            return None;
                        }
                    }
                    return Some(output);
                }
                IrBinOp::Or | IrBinOp::Xor => {
                    let lhs = self.walk(lhs, depth - 1, None)?;
                    let rhs = self.walk(rhs, depth - 1, None)?;
                    let mut output = [0; MAX_BYTES];
                    for i in 0..self.width {
                        output[i] = match (lhs[i], rhs[i]) {
                            (a, 0) => a,
                            (0, b) => b,
                            // `Or` of one byte with itself writes that byte;
                            // `Xor` would clear the lane, and two different
                            // bytes in one lane are not a permutation at all.
                            (a, b) if a == b && op == IrBinOp::Or => a,
                            _ => return None,
                        };
                    }
                    return Some(output);
                }
                _ => {}
            }
        }
        // A load, an arithmetic result, a narrowing/widening cast or an
        // intrinsic can be the opaque input, but is never peeled: independently
        // extended or truncated values are distinct sources.  Exactly one such
        // value may feed a candidate.
        // The recorded source is the typed operand, not the raw leaf: a second
        // path to the same leaf through the same cast must compare equal.
        let source = typed.unwrap_or(value);
        match self.source {
            None => self.source = Some(source),
            Some(existing) if existing == source => {}
            _ => return None,
        }
        let mut identity = [0; MAX_BYTES];
        for (i, lane) in identity[..self.width].iter_mut().enumerate() {
            *lane = i as u8 + 1;
        }
        Some(identity)
    }
}

/// Match `value` as a byte-reversal network over one source.
///
/// Widths are the ones the IR defines `Bswap` for and every backend lowers to a
/// single instruction or a fixed short sequence: `I32`/`U32` and `I64`/`U64`.
/// `I16`/`U16` is deliberately excluded even though `Bswap` exists there: C
/// promotes every 16-bit swap expression to `int`, so a 16-bit ring never
/// reaches the pass from source, and the backends disagree about the upper
/// bits of an I16 carrier (x86 leaves them stale, i686 re-extends), which is
/// not a contract this rewrite should start depending on for no gain.
///
/// `native_bswap` is the target's capability flag: where the swap is a single
/// instruction, a partial reversal folds to `Bswap` plus a mask; where the
/// backend has to expand the swap into the very shift/mask operations the
/// network is written in (RISC-V without `rev8`), the portable tree is kept
/// because replacing it would grow the code.
pub(super) fn bswap(
    value: Operand,
    ty: IrType,
    defs: &[Option<Instruction>],
    native_bswap: bool,
) -> Option<Match> {
    if !matches!(ty, IrType::I32 | IrType::U32 | IrType::I64 | IrType::U64) {
        return None;
    }
    let mut bytes = Bytes {
        defs,
        source: None,
        width: ty.size(),
        visits: VISIT_BUDGET,
        ops: 0,
    };
    let permutation = bytes.walk(value, MAX_DEPTH, None)?;
    let width = bytes.width;
    let src = bytes.source?;
    // Lane `i` of a reversed word carries source byte `width - i`; a zero lane
    // is one the original network never writes.
    let mut mask = 0u64;
    for (i, &lane) in permutation[..width].iter().enumerate() {
        if lane == 0 {
            continue;
        }
        if lane as usize != width - i {
            return None;
        }
        mask |= 0xffu64 << (8 * i);
    }
    let all = if width == MAX_BYTES {
        u64::MAX
    } else {
        (1u64 << (8 * width)) - 1
    };
    if mask == all {
        return Some(Match {
            src,
            mask: None,
            ops: bytes.ops,
        });
    }
    if mask == 0 || !native_bswap || bytes.ops < MIN_MASKED_OPS {
        return None;
    }
    Some(Match {
        src,
        mask: Some(mask),
        ops: bytes.ops,
    })
}

/// Account for replacing `old` with `new`, then drop the values that no longer
/// have any reader.
///
/// `live_uses` is the pass's reference count.  Decrementing `old`'s operands
/// can take a value to zero, and a value with no reader is dead: its defining
/// instruction is dead code too, so *its* operands lose a reader, transitively.
/// This is what keeps one `Or`-merge chain from emitting one masked reversal
/// per prefix node — accepting the widest candidate kills every prefix, so they
/// are skipped — while a prefix that still feeds live code keeps a positive
/// count and is folded on its own.  Marking a whole subtree as "seen" instead
/// is both too strong (it suppresses that second, profitable fold) and too weak
/// to prove deadness.
///
/// The walk stops at instructions that are not dead just because their result
/// is unused — stores, volatile accesses, impure calls — so those reads stay
/// counted.
///
/// `budget` bounds the walk: exhausting it leaves counts conservatively high,
/// which can only forgo an optimization, never break one.
pub(super) fn discharge(
    old: &Instruction,
    new: &Instruction,
    defs: &[Option<Instruction>],
    live_uses: &mut [u32],
    budget: &mut u16,
) {
    fn dec(counts: &mut [u32], v: Value) -> bool {
        match counts.get_mut(v.0 as usize) {
            Some(c) => {
                *c = c.saturating_sub(1);
                *c == 0
            }
            None => false,
        }
    }
    fn inc(counts: &mut [u32], v: Value) {
        if let Some(c) = counts.get_mut(v.0 as usize) {
            *c = c.saturating_add(1);
        }
    }
    fn operand_values(inst: &Instruction, f: &mut impl FnMut(Value)) {
        crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
            if let Operand::Value(v) = op {
                f(*v);
            }
        });
        crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| f(*v));
    }

    let mut dead: Vec<Value> = Vec::new();
    {
        let mut killed = |v: Value| {
            if dec(live_uses, v) {
                dead.push(v);
            }
        };
        operand_values(old, &mut killed);
    }
    operand_values(new, &mut |v: Value| inc(live_uses, v));
    while let Some(value) = dead.pop() {
        let Some(left) = budget.checked_sub(1) else {
            return;
        };
        *budget = left;
        let Some(Some(definition)) = defs.get(value.0 as usize) else {
            continue;
        };
        // Only a droppable definition loses its reads when its result dies.
        // A store, a volatile load or an impure call keeps reading its operands
        // even with an unused result, so its counts must stay where they are.
        if crate::passes::dce::has_side_effects(definition) {
            continue;
        }
        let mut killed = |v: Value| {
            if dec(live_uses, v) {
                dead.push(v);
            }
        };
        operand_values(definition, &mut killed);
    }
}

#[cfg(test)]
#[path = "bit_idioms_bytes_tests.rs"]
mod tests;
