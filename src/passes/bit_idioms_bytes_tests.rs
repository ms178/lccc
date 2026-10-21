//! Byte-provenance matcher: accepted shapes, refusals, and cost bounds.
//!
//! Fixtures are built in IR (not C) so the assertions are about the pass's
//! decisions rather than about what one backend happens to emit.  The negative
//! cases matter as much as the positive ones: a matcher that fires on a
//! non-permutation is a miscompile, so every refusal is pinned.
use super::*;
use crate::ir::reexports::{BasicBlock, BlockId, Terminator};

struct Graph {
    insts: Vec<Instruction>,
    next: u32,
}

impl Graph {
    fn new() -> Self {
        Self {
            insts: Vec::new(),
            next: 0,
        }
    }

    fn put(&mut self, f: impl FnOnce(Value) -> Instruction) -> Operand {
        let v = Value(self.next);
        self.next += 1;
        self.insts.push(f(v));
        Operand::Value(v)
    }

    fn param(&mut self, ty: IrType) -> Operand {
        self.put(|dest| Instruction::ParamRef {
            dest,
            param_idx: 0,
            ty,
        })
    }

    fn c(&self, value: u64, ty: IrType) -> Operand {
        Operand::Const(IrConst::from_i64(value as i64, ty))
    }

    fn bin(&mut self, op: IrBinOp, lhs: Operand, rhs: Operand, ty: IrType) -> Operand {
        self.put(|dest| Instruction::BinOp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        })
    }

    fn cast(&mut self, src: Operand, from_ty: IrType, to_ty: IrType) -> Operand {
        self.put(|dest| Instruction::Cast {
            dest,
            src,
            from_ty,
            to_ty,
        })
    }

    fn copy(&mut self, src: Operand) -> Operand {
        self.put(|dest| Instruction::Copy { dest, src })
    }

    fn function(self, ty: IrType, result: Operand) -> IrFunction {
        let mut f = IrFunction::new("bytes".into(), ty, vec![], false);
        f.next_value_id = self.next;
        f.next_label = 1;
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            source_spans: vec![crate::common::source::Span::dummy(); self.insts.len()],
            instructions: self.insts,
            terminator: Terminator::Return(Some(result)),
        });
        f
    }
}

/// Byte-reversal network: byte `i` of the source moves to lane `width-1-i`
/// through one masked shift term per lane, merged with `|` or `^`.
fn network(g: &mut Graph, x: Operand, ty: IrType, merge: IrBinOp) -> Operand {
    let width = ty.size();
    let mut result = g.c(0, ty);
    for i in 0..width {
        let masked = g.bin(IrBinOp::And, x, g.c(0xff << (8 * i), ty), ty);
        // Byte `i` lands in lane `width-1-i`: left by 8*(width-1-2i) when it
        // moves up, right by 8*(2i+1-width) when it moves down.
        let (op, amount) = if i < width / 2 {
            (IrBinOp::Shl, 8 * (width - 1 - 2 * i) as u64)
        } else {
            (IrBinOp::LShr, 8 * (2 * i + 1 - width) as u64)
        };
        let shifted = g.bin(op, masked, g.c(amount, ty), ty);
        result = g.bin(merge, result, shifted, ty);
    }
    result
}

/// Staged network: whole-byte swap stages, the shape glibc's `bswap_64` uses.
fn staged(g: &mut Graph, mut x: Operand, ty: IrType) -> Operand {
    let width = ty.size();
    let mut amount = 1usize;
    while amount < width {
        let mut mask = 0u64;
        let mut lane = 0usize;
        while lane + amount <= width {
            for k in lane..lane + amount {
                mask |= 0xffu64 << (8 * k);
            }
            lane += 2 * amount;
        }
        let shift = 8 * amount as u64;
        let right = g.bin(IrBinOp::LShr, x, g.c(shift, ty), ty);
        let right = g.bin(IrBinOp::And, right, g.c(mask, ty), ty);
        let left = g.bin(IrBinOp::And, x, g.c(mask, ty), ty);
        let left = g.bin(IrBinOp::Shl, left, g.c(shift, ty), ty);
        x = g.bin(IrBinOp::Or, right, left, ty);
        amount *= 2;
    }
    x
}

/// The single `Bswap` in `f`, if the rewrite fired exactly once.
fn sole_bswap(f: &IrFunction) -> Option<(Operand, IrType)> {
    let mut found = None;
    for block in &f.blocks {
        for inst in &block.instructions {
            if let Instruction::UnaryOp {
                op: IrUnaryOp::Bswap,
                src,
                ty,
                ..
            } = inst
            {
                assert!(found.is_none(), "more than one Bswap was introduced");
                found = Some((*src, *ty));
            }
        }
    }
    found
}

/// Every `Bswap` destination in `f`, in program order.
fn bswap_dests(f: &IrFunction) -> Vec<Value> {
    let mut found = Vec::new();
    for block in &f.blocks {
        for inst in &block.instructions {
            if let Instruction::UnaryOp {
                dest,
                op: IrUnaryOp::Bswap,
                ..
            } = inst
            {
                found.push(*dest);
            }
        }
    }
    found
}

/// Every mask applied to a partial reversal, in program order.
fn masked_swap_masks(f: &IrFunction) -> Vec<u64> {
    let swapped = bswap_dests(f);
    let mut masks = Vec::new();
    for block in &f.blocks {
        for inst in &block.instructions {
            if let Instruction::BinOp {
                op: IrBinOp::And,
                lhs: Operand::Value(lhs),
                rhs: Operand::Const(c),
                ..
            } = inst
            {
                if swapped.contains(lhs) {
                    if let Some(mask) = c.to_u64() {
                        masks.push(mask);
                    }
                }
            }
        }
    }
    masks
}

/// The seven surviving terms of a 64-bit reversal whose source byte 6 is never
/// routed anywhere: lanes 0..2 and 4..7 are filled, lane 1 stays zero.
fn missing_byte_network(g: &mut Graph, x: Operand) -> Operand {
    let mut result = g.c(0, IrType::U64);
    for (mask, op, amount) in [
        (0x0000_0000_0000_00ffu64, IrBinOp::Shl, 56u64),
        (0x0000_0000_0000_ff00, IrBinOp::Shl, 40),
        (0x0000_0000_00ff_0000, IrBinOp::Shl, 24),
        (0x0000_0000_ff00_0000, IrBinOp::Shl, 8),
        (0x0000_00ff_0000_0000, IrBinOp::LShr, 8),
        (0x0000_ff00_0000_0000, IrBinOp::LShr, 24),
        (0xff00_0000_0000_0000, IrBinOp::LShr, 56),
    ] {
        let m = g.bin(IrBinOp::And, x, g.c(mask, IrType::U64), IrType::U64);
        let shifted = g.bin(op, m, g.c(amount, IrType::U64), IrType::U64);
        result = g.bin(IrBinOp::Or, result, shifted, IrType::U64);
    }
    result
}

fn recognize(f: &mut IrFunction) -> usize {
    // Bit reversal off (x86-64), rotate bounds as the pass sets them for the
    // same target, scalar ANDN unavailable, and a native byte swap.
    recognize_function(f, false, 64, 8, false, true)
}

#[test]
fn byte_terms_with_or_fold_to_bswap64() {
    let mut g = Graph::new();
    let x = g.param(IrType::U64);
    let root = network(&mut g, x, IrType::U64, IrBinOp::Or);
    let mut f = g.function(IrType::U64, root);
    assert_eq!(recognize(&mut f), 1);
    assert_eq!(sole_bswap(&f), Some((x, IrType::U64)));
}

#[test]
fn byte_terms_with_xor_fold_to_bswap64() {
    let mut g = Graph::new();
    let x = g.param(IrType::U64);
    let root = network(&mut g, x, IrType::U64, IrBinOp::Xor);
    let mut f = g.function(IrType::U64, root);
    assert_eq!(recognize(&mut f), 1);
    assert_eq!(sole_bswap(&f), Some((x, IrType::U64)));
}

#[test]
fn byte_terms_fold_to_bswap32() {
    let mut g = Graph::new();
    let x = g.param(IrType::U32);
    let root = network(&mut g, x, IrType::U32, IrBinOp::Or);
    let mut f = g.function(IrType::U32, root);
    assert_eq!(recognize(&mut f), 1);
    assert_eq!(sole_bswap(&f), Some((x, IrType::U32)));
}

#[test]
fn staged_masks_fold_to_bswap64() {
    // Three mask/or stages — no single instruction is a swap, so this pins that
    // the rewrite is about the resulting permutation, not about the shape.
    let mut g = Graph::new();
    let x = g.param(IrType::U64);
    let root = staged(&mut g, x, IrType::U64);
    let mut f = g.function(IrType::U64, root);
    assert_eq!(recognize(&mut f), 1);
    assert_eq!(sole_bswap(&f), Some((x, IrType::U64)));
}

#[test]
fn same_width_casts_are_transparent() {
    // `(uint32_t)x` in the source expression: the cast reinterprets the same
    // bytes, so the network must still fold — and the source must stay the
    // cast's result, whose type matches the `Bswap` it feeds.
    let mut g = Graph::new();
    let x = g.param(IrType::I32);
    let cast = g.cast(x, IrType::I32, IrType::U32);
    let root = network(&mut g, cast, IrType::U32, IrBinOp::Or);
    let mut f = g.function(IrType::U32, root);
    assert_eq!(recognize(&mut f), 1);
    assert_eq!(sole_bswap(&f), Some((cast, IrType::U32)));
}

#[test]
fn copy_chains_are_peeled() {
    let mut g = Graph::new();
    let x = g.param(IrType::U32);
    let a = g.copy(x);
    let b = g.copy(a);
    let c = g.copy(b);
    let root = network(&mut g, c, IrType::U32, IrBinOp::Or);
    let mut f = g.function(IrType::U32, root);
    assert_eq!(recognize(&mut f), 1);
    assert_eq!(sole_bswap(&f), Some((x, IrType::U32)));
}

#[test]
fn idempotent_or_lane_is_accepted() {
    // `(x >> 24) | (x >> 24)` writes the same byte into the same lane twice;
    // the merged lane is still exactly that byte of the source.
    let mut g = Graph::new();
    let x = g.param(IrType::U32);
    let mut root = g.c(0, IrType::U32);
    for (mask, op, amount) in [
        (0x0000_00ffu64, IrBinOp::Shl, 24u64),
        (0x0000_ff00, IrBinOp::Shl, 8),
        (0x00ff_0000, IrBinOp::LShr, 8),
    ] {
        let m = g.bin(IrBinOp::And, x, g.c(mask, IrType::U32), IrType::U32);
        let s = g.bin(op, m, g.c(amount, IrType::U32), IrType::U32);
        root = g.bin(IrBinOp::Or, root, s, IrType::U32);
    }
    let high = g.bin(IrBinOp::LShr, x, g.c(24, IrType::U32), IrType::U32);
    root = g.bin(IrBinOp::Or, root, high, IrType::U32);
    root = g.bin(IrBinOp::Or, root, high, IrType::U32);
    let mut f = g.function(IrType::U32, root);
    assert_eq!(recognize(&mut f), 1);
    assert_eq!(sole_bswap(&f), Some((x, IrType::U32)));
}

#[test]
fn xor_of_two_bytes_in_one_lane_is_refused() {
    // `(x & 0xff) ^ (x & 0xff00)` mixes two source bytes in one lane; it is
    // neither a lane permutation nor representable, so it must stay as written.
    let mut g = Graph::new();
    let x = g.param(IrType::U32);
    let a = g.bin(IrBinOp::And, x, g.c(0xff, IrType::U32), IrType::U32);
    let b = g.bin(IrBinOp::And, x, g.c(0xff00, IrType::U32), IrType::U32);
    let mixed = g.bin(IrBinOp::Xor, a, b, IrType::U32);
    let mut f = g.function(IrType::U32, mixed);
    recognize(&mut f);
    assert_eq!(sole_bswap(&f), None);
}

#[test]
fn partial_mask_byte_is_refused() {
    // A mask that keeps half a byte mixes two source bytes in one lane.
    let mut g = Graph::new();
    let x = g.param(IrType::U32);
    let masked = g.bin(IrBinOp::And, x, g.c(0x0f0f_0f0f, IrType::U32), IrType::U32);
    let shifted = g.bin(IrBinOp::Shl, masked, g.c(4, IrType::U32), IrType::U32);
    let root = g.bin(IrBinOp::Or, masked, shifted, IrType::U32);
    let mut f = g.function(IrType::U32, root);
    recognize(&mut f);
    assert_eq!(sole_bswap(&f), None);
}

#[test]
fn sub_byte_shift_is_refused() {
    let mut g = Graph::new();
    let x = g.param(IrType::U32);
    let masked = g.bin(IrBinOp::And, x, g.c(0xff, IrType::U32), IrType::U32);
    let shifted = g.bin(IrBinOp::Shl, masked, g.c(4, IrType::U32), IrType::U32);
    let mut f = g.function(IrType::U32, shifted);
    recognize(&mut f);
    assert_eq!(sole_bswap(&f), None);
}

#[test]
fn arithmetic_shift_is_refused() {
    // `>>` on a signed type fills from the sign bit, so lanes are not a
    // permutation of the source's bytes.
    let mut g = Graph::new();
    let x = g.param(IrType::I32);
    let masked = g.bin(IrBinOp::And, x, g.c(0xff00_0000, IrType::I32), IrType::I32);
    let shifted = g.bin(IrBinOp::AShr, masked, g.c(24, IrType::I32), IrType::I32);
    let mut f = g.function(IrType::I32, shifted);
    recognize(&mut f);
    assert_eq!(sole_bswap(&f), None);
}

#[test]
fn two_sources_are_refused() {
    // Lanes that come from different values are not a byte reversal of either.
    let mut g = Graph::new();
    let x = g.param(IrType::U32);
    let y = g.param(IrType::U32);
    let lo = g.bin(IrBinOp::And, x, g.c(0xff, IrType::U32), IrType::U32);
    let lo = g.bin(IrBinOp::Shl, lo, g.c(24, IrType::U32), IrType::U32);
    let hi = g.bin(IrBinOp::And, y, g.c(0xff00_0000, IrType::U32), IrType::U32);
    let hi = g.bin(IrBinOp::LShr, hi, g.c(24, IrType::U32), IrType::U32);
    let mid = g.bin(IrBinOp::And, x, g.c(0x00ff_0000, IrType::U32), IrType::U32);
    let mid = g.bin(IrBinOp::LShr, mid, g.c(8, IrType::U32), IrType::U32);
    let low = g.bin(IrBinOp::And, x, g.c(0x0000_ff00, IrType::U32), IrType::U32);
    let low = g.bin(IrBinOp::Shl, low, g.c(8, IrType::U32), IrType::U32);
    let root = g.bin(IrBinOp::Or, lo, hi, IrType::U32);
    let root = g.bin(IrBinOp::Or, root, mid, IrType::U32);
    let root = g.bin(IrBinOp::Or, root, low, IrType::U32);
    let mut f = g.function(IrType::U32, root);
    recognize(&mut f);
    assert_eq!(sole_bswap(&f), None);
}

#[test]
fn rotate_is_not_a_byte_swap() {
    // `(x << 8) | (x >> 24)` is a rotate: it must never become a `Bswap`.
    let mut g = Graph::new();
    let x = g.param(IrType::U32);
    let l = g.bin(IrBinOp::Shl, x, g.c(8, IrType::U32), IrType::U32);
    let r = g.bin(IrBinOp::LShr, x, g.c(24, IrType::U32), IrType::U32);
    let root = g.bin(IrBinOp::Or, l, r, IrType::U32);
    let mut f = g.function(IrType::U32, root);
    recognize(&mut f);
    assert_eq!(sole_bswap(&f), None);
}

#[test]
fn deep_copy_chain_is_bounded() {
    // The walk has a fixed depth; a chain longer than the cap must be refused
    // rather than recursed through (compile time stays bounded, no overflow).
    let mut g = Graph::new();
    let mut x = g.param(IrType::U32);
    for _ in 0..64 {
        x = g.copy(x);
    }
    let root = network(&mut g, x, IrType::U32, IrBinOp::Or);
    let mut f = g.function(IrType::U32, root);
    recognize(&mut f);
    assert_eq!(sole_bswap(&f), None);
}

#[test]
fn only_the_consuming_instruction_is_replaced() {
    // The rewrite is consumer-side: every producer instruction survives, so a
    // network input that other code shares keeps its definition.
    let mut g = Graph::new();
    let x = g.param(IrType::U32);
    let root = network(&mut g, x, IrType::U32, IrBinOp::Or);
    let mut f = g.function(IrType::U32, root);
    let before = f.blocks[0].instructions.len();
    assert_eq!(recognize(&mut f), 1);
    assert_eq!(f.blocks[0].instructions.len(), before);
}

#[test]
fn missing_byte_becomes_a_masked_swap() {
    // One source byte is never routed anywhere, so the value is the reversal on
    // seven lanes: `Bswap(x) & !(0xff << 8)`, not a full swap and not the
    // 130-byte network it is written as.
    let mut g = Graph::new();
    let x = g.param(IrType::U64);
    let root = missing_byte_network(&mut g, x);
    let mut f = g.function(IrType::U64, root);
    assert_eq!(recognize(&mut f), 1);
    assert_eq!(sole_bswap(&f), Some((x, IrType::U64)));
    assert_eq!(masked_swap_masks(&f), vec![!0xff00u64]);
}

#[test]
fn missing_byte_is_kept_without_a_native_swap() {
    // Where `Bswap` expands to the same shift/mask tree the network is written
    // in, the portable form is already the cheaper one and must be left alone.
    let mut g = Graph::new();
    let x = g.param(IrType::U64);
    let root = missing_byte_network(&mut g, x);
    let mut f = g.function(IrType::U64, root);
    assert_eq!(recognize_function(&mut f, false, 64, 8, false, false), 0);
    assert_eq!(sole_bswap(&f), None);
}

#[test]
fn single_lane_extraction_is_refused() {
    // `bswap(x) & 0xff` would be two instructions for a one-instruction shift.
    let mut g = Graph::new();
    let x = g.param(IrType::U32);
    let shifted = g.bin(IrBinOp::LShr, x, g.c(24, IrType::U32), IrType::U32);
    let masked = g.bin(IrBinOp::And, shifted, g.c(0xff, IrType::U32), IrType::U32);
    let mut f = g.function(IrType::U32, masked);
    assert_eq!(recognize(&mut f), 0);
    assert_eq!(sole_bswap(&f), None);
}

#[test]
fn a_merge_chain_folds_to_exactly_one_masked_swap() {
    // Every `Or` of the chain is itself a partial reversal, so the ranked phase
    // sees seven candidates for one network.  Accepting the widest must
    // discharge the rest: one `Bswap`, one mask, and it is the widest mask.
    let mut g = Graph::new();
    let x = g.param(IrType::U64);
    let root = missing_byte_network(&mut g, x);
    let mut f = g.function(IrType::U64, root);
    assert_eq!(recognize(&mut f), 1);
    assert_eq!(bswap_dests(&f).len(), 1);
    assert_eq!(masked_swap_masks(&f), vec![!0xff00u64]);
}

#[test]
fn a_narrower_lane_map_keeps_its_own_mask() {
    // A 32-bit network over the high half only: the mask must describe exactly
    // the lanes the network writes.
    let mut g = Graph::new();
    let x = g.param(IrType::U32);
    let a = g.bin(IrBinOp::And, x, g.c(0x0000_00ff, IrType::U32), IrType::U32);
    let a = g.bin(IrBinOp::Shl, a, g.c(24, IrType::U32), IrType::U32);
    let b = g.bin(IrBinOp::And, x, g.c(0x0000_ff00, IrType::U32), IrType::U32);
    let b = g.bin(IrBinOp::Shl, b, g.c(8, IrType::U32), IrType::U32);
    let root = g.bin(IrBinOp::Or, a, b, IrType::U32);
    let mut f = g.function(IrType::U32, root);
    assert_eq!(recognize(&mut f), 1);
    assert_eq!(masked_swap_masks(&f), vec![0xffff_0000u64]);
}

#[test]
fn a_live_prefix_keeps_its_own_masked_swap() {
    // The widest candidate consumes the whole merge chain, but the node that
    // reverses bytes 0..3 also feeds a consumer outside that tree. Reversing
    // only the widest would leave the prefix's twelve-instruction expansion
    // alive for the sake of one `Add`; the ranked pass has to fold the prefix
    // as well, and the widest mask must still be the superset.
    let mut g = Graph::new();
    let x = g.param(IrType::U64);
    let mut result = g.c(0, IrType::U64);
    let mut prefix = None;
    for (lane, (mask, op, amount)) in [
        (0x0000_0000_0000_00ffu64, IrBinOp::Shl, 56u64),
        (0x0000_0000_0000_ff00, IrBinOp::Shl, 40),
        (0x0000_0000_00ff_0000, IrBinOp::Shl, 24),
        (0x0000_0000_ff00_0000, IrBinOp::Shl, 8),
        (0x0000_00ff_0000_0000, IrBinOp::LShr, 8),
        (0x0000_ff00_0000_0000, IrBinOp::LShr, 24),
        (0xff00_0000_0000_0000, IrBinOp::LShr, 56),
    ]
    .into_iter()
    .enumerate()
    {
        let m = g.bin(IrBinOp::And, x, g.c(mask, IrType::U64), IrType::U64);
        let shifted = g.bin(op, m, g.c(amount, IrType::U64), IrType::U64);
        result = g.bin(IrBinOp::Or, result, shifted, IrType::U64);
        if lane == 3 {
            prefix = Some(result);
        }
    }
    let live = g.bin(
        IrBinOp::Add,
        result,
        prefix.expect("prefix recorded after four terms"),
        IrType::U64,
    );
    let mut f = g.function(IrType::U64, live);
    assert_eq!(recognize(&mut f), 2);
    let masks = masked_swap_masks(&f);
    assert_eq!(masks.len(), 2);
    assert!(masks.contains(&!0xff00u64), "{masks:x?}");
    assert!(masks.contains(&0xffff_ffff_0000_0000u64), "{masks:x?}");
}
