//! Strength reduction for integer division and modulo by constants.
//!
//! Replaces slow `div`/`idiv` instructions (20-90 cycles on x86) with fast
//! multiply-and-shift sequences (3-5 cycles). This is one of the most impactful
//! single optimizations for integer-heavy code.
//!
//! Supported transformations:
//!
//! **Unsigned division by constant** (32-bit and 64-bit):
//!   `x /u C` => `mulhi(x, M) >> s` (with optional add-and-shift fixup)
//!   32-bit uses 64-bit intermediate; 64-bit uses 128-bit intermediate.
//!   Uses the "magic number" algorithm from Hacker's Delight by Henry S. Warren Jr.
//!
//! **Signed division by power-of-2**:
//!   `x /s 2^k` => `(x + ((x >> (N-1)) >>> (N - k))) >> k`
//!   Adds a bias for negative numbers to ensure correct rounding toward zero.
//!
//! **Signed division by constant** (32-bit and 64-bit):
//!   `x /s C` => multiply by magic number + shift + sign correction
//!   Uses the signed magic number algorithm from Hacker's Delight.
//!   32-bit arithmetic avoids add-fixup by using zero-overhead 64-bit products.
//!   64-bit arithmetic handles hardware `smulh`/`mulh`/`imulq` sign semantics correctly.
//!
//! **Modulo by constant**:
//!   `x % C` => `x - (x / C) * C`  (using the optimized division above)
//!   Includes full strength reduction for signed power-of-2 remainders.
//!
//! All transformations produce correct results for the full range of inputs,
//! including edge cases (0, 1, -1, INT_MIN, UINT_MAX, INT64_MIN, UINT64_MAX).
//!
//! ## Implementation
//!
//! 64-bit magic numbers require 128-bit intermediate products. We use the
//! compiler's existing I128/U128 types: cast both operands to 128-bit,
//! multiply, shift right by 64, and truncate back to 64-bit. All backends
//! (x86, i686, ARM64, RISC-V) have I128 multiply support that maps to efficient
//! hardware instructions (mulq/imulq on x86, umulh/smulh on ARM64,
//! mulhu/mulh on RISC-V).
//!
//! - Negative divisors are handled via identity: `x / -C == -(x / C)` and
//!   `x % -C == x % C` (in C, the sign of the remainder follows the dividend).

use crate::common::types::IrType;
use crate::ir::reexports::{
    Instruction,
    IrBinOp,
    IrConst,
    IrFunction,
    Operand,
    Value,
};

/// Transform division/modulo by constants in a single function.
pub(crate) fn div_by_const_function(func: &mut IrFunction) -> usize {
    let mut changes = 0;
    let mut next_id = func.next_value_id.max(func.max_value_id() + 1);

    // Build sets of values known to fit in unsigned 32-bit or signed 32-bit ranges.
    //
    // Soundness definitions:
    //   - is_known_u32: The 64-bit representation is guaranteed to be in [0, 2^32 - 1]
    //     (upper 32 bits are 0). Safe for UDiv/URem strength reduction in I64.
    //   - is_known_i32: The 64-bit representation is guaranteed to be in [-2^31, 2^31 - 1]
    //     (upper 32 bits are the exact sign extension of bit 31). Safe for SDiv/SRem in I64.
    //
    // Note: An unsigned U32 in range [2^31, 2^32-1] does NOT fit in i32!
    // A signed negative I32 (e.g. -10) in 64 bits has bit 63 set, and does NOT fit in u32!
    let max_id = func.max_value_id() as usize;
    let mut is_known_u32: Vec<bool> = vec![false; max_id + 1];
    let mut is_known_i32: Vec<bool> = vec![false; max_id + 1];

    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::ParamRef { dest, ty, .. } => {
                    let id = dest.0 as usize;
                    if id <= max_id && ty.is_integer() {
                        if ty.is_unsigned() {
                            if ty.size() <= 4 {
                                is_known_u32[id] = true;
                            }
                            if ty.size() < 4 {
                                // u8 and u16 fit in positive signed i32 [0, 65535]
                                is_known_i32[id] = true;
                            }
                        } else if ty.size() <= 4 {
                            // Signed <= 32 bits fits in i32 range
                            is_known_i32[id] = true;
                        }
                    }
                }
                Instruction::Cast { dest, from_ty, to_ty, .. } => {
                    let id = dest.0 as usize;
                    if id <= max_id {
                        // Widening to 64-bit
                        if from_ty.is_integer() && from_ty.size() <= 4
                            && (*to_ty == IrType::I64 || *to_ty == IrType::U64)
                        {
                            if from_ty.is_unsigned() {
                                is_known_u32[id] = true;
                                if from_ty.size() < 4 {
                                    is_known_i32[id] = true;
                                }
                            } else {
                                is_known_i32[id] = true;
                            }
                        }
                        // Truncation/narrowing to <= 32-bit
                        if to_ty.is_integer() && to_ty.size() <= 4 {
                            if to_ty.is_unsigned() {
                                is_known_u32[id] = true;
                                if to_ty.size() < 4 {
                                    is_known_i32[id] = true;
                                }
                            } else {
                                is_known_i32[id] = true;
                            }
                        }
                    }
                }
                Instruction::Load { dest, ty, .. } => {
                    let id = dest.0 as usize;
                    if id <= max_id && ty.is_integer() {
                        if ty.is_unsigned() {
                            if ty.size() <= 4 {
                                is_known_u32[id] = true;
                            }
                            if ty.size() < 4 {
                                is_known_i32[id] = true;
                            }
                        } else if ty.size() <= 4 {
                            is_known_i32[id] = true;
                        }
                    }
                }
                Instruction::BinOp { dest, op, ty, .. } => {
                    let id = dest.0 as usize;
                    if id <= max_id && ty.is_integer() {
                        if ty.is_unsigned() {
                            if ty.size() <= 4 {
                                is_known_u32[id] = true;
                            }
                            if ty.size() < 4 {
                                is_known_i32[id] = true;
                            }
                        } else if ty.size() <= 4 {
                            if *op == IrBinOp::And || *op == IrBinOp::LShr {
                                is_known_u32[id] = true;
                            }
                            is_known_i32[id] = true;
                        }
                    }
                }
                Instruction::Cmp { dest, .. } => {
                    let id = dest.0 as usize;
                    if id <= max_id {
                        // Comparison results are always 0 or 1
                        is_known_u32[id] = true;
                        is_known_i32[id] = true;
                    }
                }
                _ => {}
            }
        }
    }

    let lhs_is_u32 = |op: &Operand| -> bool {
        match op {
            Operand::Value(v) => {
                let id = v.0 as usize;
                id <= max_id && is_known_u32[id]
            }
            Operand::Const(c) => {
                if let Some(v) = c.to_i64() {
                    v >= 0 && v <= u32::MAX as i64
                } else {
                    false
                }
            }
        }
    };

    let lhs_is_i32 = |op: &Operand| -> bool {
        match op {
            Operand::Value(v) => {
                let id = v.0 as usize;
                id <= max_id && is_known_i32[id]
            }
            Operand::Const(c) => {
                if let Some(v) = c.to_i64() {
                    v >= i32::MIN as i64 && v <= i32::MAX as i64
                } else {
                    false
                }
            }
        }
    };

    for block in &mut func.blocks {
        let mut new_insts: Vec<Instruction> = Vec::new();
        let has_spans = !block.source_spans.is_empty();
        let mut new_spans: Vec<crate::common::source::Span> = Vec::new();
        let old_spans = std::mem::take(&mut block.source_spans);

        for (inst_idx, inst) in block.instructions.drain(..).enumerate() {
            let span = if has_spans && inst_idx < old_spans.len() {
                Some(old_spans[inst_idx])
            } else {
                None
            };

            match &inst {
                Instruction::BinOp { dest, op, lhs, rhs, ty } => {
                    let const_val = match rhs {
                        Operand::Const(c) => c.to_i64(),
                        _ => None,
                    };

                    if let Some(divisor) = const_val {
                        let expanded = match op {
                            IrBinOp::UDiv => {
                                let udivisor = divisor as u64;
                                if udivisor >= 2 && udivisor <= u32::MAX as u64 {
                                    let d32 = udivisor as u32;
                                    match *ty {
                                        IrType::U32 => expand_udiv32(*dest, lhs, d32, *ty, &mut next_id),
                                        IrType::I64 | IrType::U64 if lhs_is_u32(lhs) => {
                                            expand_udiv32_in_i64(*dest, lhs, d32, &mut next_id).map(|(i, _)| i)
                                        }
                                        IrType::I64 | IrType::U64 => expand_udiv64(*dest, lhs, udivisor, *ty, &mut next_id),
                                        _ => None,
                                    }
                                } else if udivisor >= 2 {
                                    match *ty {
                                        IrType::I64 | IrType::U64 => expand_udiv64(*dest, lhs, udivisor, *ty, &mut next_id),
                                        _ => None,
                                    }
                                } else {
                                    None
                                }
                            }
                            IrBinOp::SDiv => {
                                if divisor > 1 && divisor <= i32::MAX as i64 {
                                    match *ty {
                                        IrType::I32 => expand_sdiv32(*dest, lhs, divisor as i32, *ty, &mut next_id),
                                        IrType::I64 if lhs_is_i32(lhs) => {
                                            expand_sdiv32_in_i64(*dest, lhs, divisor as i32, &mut next_id).map(|(i, _)| i)
                                        }
                                        IrType::I64 => expand_sdiv64(*dest, lhs, divisor, &mut next_id),
                                        _ => None,
                                    }
                                } else if divisor > i32::MAX as i64 {
                                    match *ty {
                                        IrType::I64 => expand_sdiv64(*dest, lhs, divisor, &mut next_id),
                                        _ => None,
                                    }
                                } else if divisor < -1 && divisor > i64::MIN {
                                    // Safe negation: divisor is strictly between (i64::MIN, -1)
                                    let pos_divisor = -divisor;
                                    if pos_divisor <= i32::MAX as i64 {
                                        let pd = pos_divisor as i32;
                                        match *ty {
                                            IrType::I32 => expand_sdiv_neg(*dest, lhs, pd, *ty, &lhs_is_i32, &mut next_id),
                                            IrType::I64 if lhs_is_i32(lhs) => {
                                                expand_sdiv_neg(*dest, lhs, pd, *ty, &lhs_is_i32, &mut next_id)
                                            }
                                            IrType::I64 => expand_sdiv64_neg(*dest, lhs, pos_divisor, &mut next_id),
                                            _ => None,
                                        }
                                    } else {
                                        match *ty {
                                            IrType::I64 => expand_sdiv64_neg(*dest, lhs, pos_divisor, &mut next_id),
                                            _ => None,
                                        }
                                    }
                                } else {
                                    None
                                }
                            }
                            IrBinOp::URem => {
                                let udivisor = divisor as u64;
                                if udivisor >= 2 && udivisor <= u32::MAX as u64 {
                                    let d32 = udivisor as u32;
                                    match *ty {
                                        IrType::U32 => expand_urem32(*dest, lhs, d32, *ty, &mut next_id),
                                        IrType::I64 | IrType::U64 if lhs_is_u32(lhs) => {
                                            expand_urem32_in_i64(*dest, lhs, d32, &mut next_id)
                                        }
                                        IrType::I64 | IrType::U64 => expand_urem64(*dest, lhs, udivisor, *ty, &mut next_id),
                                        _ => None,
                                    }
                                } else if udivisor >= 2 {
                                    match *ty {
                                        IrType::I64 | IrType::U64 => expand_urem64(*dest, lhs, udivisor, *ty, &mut next_id),
                                        _ => None,
                                    }
                                } else {
                                    None
                                }
                            }
                            IrBinOp::SRem => {
                                if divisor > 1 && divisor <= i32::MAX as i64 {
                                    let d32 = divisor as i32;
                                    match *ty {
                                        IrType::I32 => expand_srem32(*dest, lhs, d32, *ty, &mut next_id),
                                        IrType::I64 if lhs_is_i32(lhs) => expand_srem32_in_i64(*dest, lhs, d32, &mut next_id),
                                        IrType::I64 => expand_srem64(*dest, lhs, divisor, &mut next_id),
                                        _ => None,
                                    }
                                } else if divisor > i32::MAX as i64 {
                                    match *ty {
                                        IrType::I64 => expand_srem64(*dest, lhs, divisor, &mut next_id),
                                        _ => None,
                                    }
                                } else if divisor < -1 && divisor > i64::MIN {
                                    // In C: x % -C == x % C (sign follows dividend)
                                    let pos_divisor = -divisor;
                                    if pos_divisor <= i32::MAX as i64 {
                                        let pd = pos_divisor as i32;
                                        match *ty {
                                            IrType::I32 => expand_srem32(*dest, lhs, pd, *ty, &mut next_id),
                                            IrType::I64 if lhs_is_i32(lhs) => expand_srem32_in_i64(*dest, lhs, pd, &mut next_id),
                                            IrType::I64 => expand_srem64(*dest, lhs, pos_divisor, &mut next_id),
                                            _ => None,
                                        }
                                    } else {
                                        match *ty {
                                            IrType::I64 => expand_srem64(*dest, lhs, pos_divisor, &mut next_id),
                                            _ => None,
                                        }
                                    }
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        };

                        if let Some(exp) = expanded {
                            let count = exp.len();
                            new_insts.extend(exp);
                            if has_spans {
                                let s = span.unwrap_or(crate::common::source::Span::new(0, 0, 0));
                                for _ in 0..count {
                                    new_spans.push(s);
                                }
                            }
                            changes += 1;
                            continue;
                        }
                    }

                    new_insts.push(inst);
                    if has_spans {
                        new_spans.push(span.unwrap_or(crate::common::source::Span::new(0, 0, 0)));
                    }
                }
                _ => {
                    new_insts.push(inst);
                    if has_spans {
                        new_spans.push(span.unwrap_or(crate::common::source::Span::new(0, 0, 0)));
                    }
                }
            }
        }

        block.instructions = new_insts;
        if has_spans {
            block.source_spans = new_spans;
        }
    }

    if changes > 0 {
        func.next_value_id = next_id;
    }
    changes
}

#[inline(always)]
fn fresh_value(next_id: &mut u32) -> Value {
    let v = Value(*next_id);
    *next_id += 1;
    v
}

// ─── Unsigned division by constant (32-bit) ────────────────────────────────

fn compute_unsigned_magic_32(d: u32) -> Option<(u64, u32, bool)> {
    assert!(d >= 2);

    for p in 0u32..32 {
        let two_pow = 1u128 << (32 + p);
        let magic = two_pow.div_ceil(d as u128);

        let error = magic * (d as u128) - two_pow;
        if error <= (1u128 << p) {
            if magic <= u32::MAX as u128 {
                return Some((magic as u64, p, false));
            } else if magic <= u64::MAX as u128 {
                let m = magic - (1u128 << 32);
                return Some((m as u64, p, true));
            }
        }
    }

    None
}

fn expand_udiv32(
    dest: Value,
    x: &Operand,
    d: u32,
    ty: IrType,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    if d < 2 {
        return None;
    }

    if d.is_power_of_two() {
        let shift = d.trailing_zeros();
        return Some(vec![Instruction::BinOp {
            dest,
            op: IrBinOp::LShr,
            lhs: *x,
            rhs: Operand::Const(IrConst::from_i64(shift as i64, ty)),
            ty,
        }]);
    }

    let (magic, shift, needs_add) = compute_unsigned_magic_32(d)?;
    let mut insts = Vec::new();

    let x64 = fresh_value(next_id);
    insts.push(Instruction::Cast {
        dest: x64,
        src: *x,
        from_ty: IrType::U32,
        to_ty: IrType::U64,
    });

    let product = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: product,
        op: IrBinOp::Mul,
        lhs: Operand::Value(x64),
        rhs: Operand::Const(IrConst::I64(magic as i64)),
        ty: IrType::U64,
    });

    let hi = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: hi,
        op: IrBinOp::LShr,
        lhs: Operand::Value(product),
        rhs: Operand::Const(IrConst::I64(32)),
        ty: IrType::U64,
    });

    if !needs_add {
        if shift == 0 {
            insts.push(Instruction::Cast {
                dest,
                src: Operand::Value(hi),
                from_ty: IrType::U64,
                to_ty: IrType::U32,
            });
        } else {
            let shifted = fresh_value(next_id);
            insts.push(Instruction::BinOp {
                dest: shifted,
                op: IrBinOp::LShr,
                lhs: Operand::Value(hi),
                rhs: Operand::Const(IrConst::I64(shift as i64)),
                ty: IrType::U64,
            });
            insts.push(Instruction::Cast {
                dest,
                src: Operand::Value(shifted),
                from_ty: IrType::U64,
                to_ty: IrType::U32,
            });
        }
    } else {
        let diff = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: diff,
            op: IrBinOp::Sub,
            lhs: Operand::Value(x64),
            rhs: Operand::Value(hi),
            ty: IrType::U64,
        });

        let half = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: half,
            op: IrBinOp::LShr,
            lhs: Operand::Value(diff),
            rhs: Operand::Const(IrConst::I64(1)),
            ty: IrType::U64,
        });

        let sum = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: sum,
            op: IrBinOp::Add,
            lhs: Operand::Value(half),
            rhs: Operand::Value(hi),
            ty: IrType::U64,
        });

        if shift > 1 {
            let result64 = fresh_value(next_id);
            insts.push(Instruction::BinOp {
                dest: result64,
                op: IrBinOp::LShr,
                lhs: Operand::Value(sum),
                rhs: Operand::Const(IrConst::I64((shift - 1) as i64)),
                ty: IrType::U64,
            });
            insts.push(Instruction::Cast {
                dest,
                src: Operand::Value(result64),
                from_ty: IrType::U64,
                to_ty: IrType::U32,
            });
        } else {
            insts.push(Instruction::Cast {
                dest,
                src: Operand::Value(sum),
                from_ty: IrType::U64,
                to_ty: IrType::U32,
            });
        }
    }

    Some(insts)
}

// ─── Signed division by constant (32-bit) ──────────────────────────────────

/// Compute the unsigned 32-bit magic number for signed division.
/// Returns (magic as u64 in [0, 2^32-1], shift).
/// When multiplied by signed 32-bit x in 64 bits, x * magic never overflows signed 64-bit,
/// eliminating the need for an additional add fixup instruction.
fn compute_signed_magic_32(d: i32) -> (u64, u32) {
    assert!(d >= 2);
    let ad = d as u32;
    let t = 0x80000000u32;
    let anc = t - 1 - (t % ad);
    let mut p: u32 = 31;
    let mut q1 = 0x80000000u64 / anc as u64;
    let mut r1 = 0x80000000u64 - q1 * anc as u64;
    let mut q2 = 0x80000000u64 / ad as u64;
    let mut r2 = 0x80000000u64 - q2 * ad as u64;

    loop {
        p += 1;
        q1 *= 2;
        r1 *= 2;
        if r1 >= anc as u64 {
            q1 += 1;
            r1 -= anc as u64;
        }
        q2 *= 2;
        r2 *= 2;
        if r2 >= ad as u64 {
            q2 += 1;
            r2 -= ad as u64;
        }
        let delta = ad as u64 - r2;
        if q1 < delta || (q1 == delta && r1 == 0) {
            continue;
        }
        break;
    }

    let magic32 = (q2 + 1) as u32;
    let shift = p - 32;
    (magic32 as u64, shift)
}

fn expand_sdiv32(
    dest: Value,
    x: &Operand,
    d: i32,
    _ty: IrType,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    if d < 2 {
        return None;
    }

    if d > 0 && (d as u32).is_power_of_two() {
        let k = d.trailing_zeros();
        let mut insts = Vec::new();

        let sign = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: sign,
            op: IrBinOp::AShr,
            lhs: *x,
            rhs: Operand::Const(IrConst::I32(31)),
            ty: IrType::I32,
        });

        let bias = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: bias,
            op: IrBinOp::LShr,
            lhs: Operand::Value(sign),
            rhs: Operand::Const(IrConst::I32(32 - k as i32)),
            ty: IrType::I32,
        });

        let biased = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: biased,
            op: IrBinOp::Add,
            lhs: *x,
            rhs: Operand::Value(bias),
            ty: IrType::I32,
        });

        insts.push(Instruction::BinOp {
            dest,
            op: IrBinOp::AShr,
            lhs: Operand::Value(biased),
            rhs: Operand::Const(IrConst::I32(k as i32)),
            ty: IrType::I32,
        });

        return Some(insts);
    }

    let (magic, shift) = compute_signed_magic_32(d);
    let mut insts = Vec::new();

    let x64 = fresh_value(next_id);
    insts.push(Instruction::Cast {
        dest: x64,
        src: *x,
        from_ty: IrType::I32,
        to_ty: IrType::I64,
    });

    // product = x64 * magic (fits in 64 bits signed without overflow)
    let product = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: product,
        op: IrBinOp::Mul,
        lhs: Operand::Value(x64),
        rhs: Operand::Const(IrConst::I64(magic as i64)),
        ty: IrType::I64,
    });

    let hi = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: hi,
        op: IrBinOp::AShr,
        lhs: Operand::Value(product),
        rhs: Operand::Const(IrConst::I64(32)),
        ty: IrType::I64,
    });

    let shifted = if shift > 0 {
        let s = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: s,
            op: IrBinOp::AShr,
            lhs: Operand::Value(hi),
            rhs: Operand::Const(IrConst::I64(shift as i64)),
            ty: IrType::I64,
        });
        s
    } else {
        hi
    };

    let sign_bit = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: sign_bit,
        op: IrBinOp::LShr,
        lhs: Operand::Value(shifted),
        rhs: Operand::Const(IrConst::I64(63)),
        ty: IrType::I64,
    });

    let corrected = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: corrected,
        op: IrBinOp::Add,
        lhs: Operand::Value(shifted),
        rhs: Operand::Value(sign_bit),
        ty: IrType::I64,
    });

    insts.push(Instruction::Cast {
        dest,
        src: Operand::Value(corrected),
        from_ty: IrType::I64,
        to_ty: IrType::I32,
    });

    Some(insts)
}

// ─── Unsigned modulo by constant (32-bit) ──────────────────────────────────

fn expand_urem32(
    dest: Value,
    x: &Operand,
    d: u32,
    ty: IrType,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    if d < 2 || d.is_power_of_two() {
        return None;
    }

    let quotient = fresh_value(next_id);
    let mut insts = expand_udiv32(quotient, x, d, ty, next_id)?;

    let prod = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: prod,
        op: IrBinOp::Mul,
        lhs: Operand::Value(quotient),
        rhs: Operand::Const(IrConst::from_i64(d as i64, ty)),
        ty,
    });

    insts.push(Instruction::BinOp {
        dest,
        op: IrBinOp::Sub,
        lhs: *x,
        rhs: Operand::Value(prod),
        ty,
    });

    Some(insts)
}

// ─── Signed modulo by constant (32-bit) ────────────────────────────────────

fn expand_srem32(
    dest: Value,
    x: &Operand,
    d: i32,
    ty: IrType,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    if d < 2 {
        return None;
    }

    let quotient = fresh_value(next_id);
    let mut insts = expand_sdiv32(quotient, x, d, ty, next_id)?;

    let prod = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: prod,
        op: IrBinOp::Mul,
        lhs: Operand::Value(quotient),
        rhs: Operand::Const(IrConst::I32(d)),
        ty,
    });

    insts.push(Instruction::BinOp {
        dest,
        op: IrBinOp::Sub,
        lhs: *x,
        rhs: Operand::Value(prod),
        ty,
    });

    Some(insts)
}

// ─── Unsigned division by constant (64-bit) ────────────────────────────────

fn compute_unsigned_magic_64(d: u64) -> Option<(u128, u32, bool)> {
    assert!(d >= 2);

    for p in 0u32..64 {
        let two_pow: u128 = 1u128 << (64 + p);
        let magic = two_pow.div_ceil(d as u128);

        let error = magic * (d as u128) - two_pow;
        if error <= (1u128 << p) {
            if magic <= u64::MAX as u128 {
                return Some((magic, p, false));
            } else {
                let m = magic - (1u128 << 64);
                if m <= u64::MAX as u128 {
                    return Some((m, p, true));
                }
            }
        }
    }

    None
}

fn expand_udiv64(
    dest: Value,
    x: &Operand,
    d: u64,
    ty: IrType,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    if d < 2 {
        return None;
    }

    if d.is_power_of_two() {
        let shift = d.trailing_zeros();
        return Some(vec![Instruction::BinOp {
            dest,
            op: IrBinOp::LShr,
            lhs: *x,
            rhs: Operand::Const(IrConst::I64(shift as i64)),
            ty,
        }]);
    }

    let (magic, shift, needs_add) = compute_unsigned_magic_64(d)?;
    let mut insts = Vec::new();

    let x_u64 = if ty == IrType::U64 {
        *x
    } else {
        let v = fresh_value(next_id);
        insts.push(Instruction::Cast {
            dest: v,
            src: *x,
            from_ty: ty,
            to_ty: IrType::U64,
        });
        Operand::Value(v)
    };

    let x128 = fresh_value(next_id);
    insts.push(Instruction::Cast {
        dest: x128,
        src: x_u64,
        from_ty: IrType::U64,
        to_ty: IrType::U128,
    });

    let product = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: product,
        op: IrBinOp::Mul,
        lhs: Operand::Value(x128),
        rhs: Operand::Const(IrConst::I128(magic as i128)),
        ty: IrType::U128,
    });

    let hi128 = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: hi128,
        op: IrBinOp::LShr,
        lhs: Operand::Value(product),
        rhs: Operand::Const(IrConst::I128(64)),
        ty: IrType::U128,
    });

    let hi_u64 = fresh_value(next_id);
    insts.push(Instruction::Cast {
        dest: hi_u64,
        src: Operand::Value(hi128),
        from_ty: IrType::U128,
        to_ty: IrType::U64,
    });

    let hi = if ty == IrType::U64 {
        hi_u64
    } else {
        let v = fresh_value(next_id);
        insts.push(Instruction::Cast {
            dest: v,
            src: Operand::Value(hi_u64),
            from_ty: IrType::U64,
            to_ty: ty,
        });
        v
    };

    if !needs_add {
        if shift == 0 {
            insts.push(Instruction::Copy { dest, src: Operand::Value(hi) });
        } else {
            insts.push(Instruction::BinOp {
                dest,
                op: IrBinOp::LShr,
                lhs: Operand::Value(hi),
                rhs: Operand::Const(IrConst::I64(shift as i64)),
                ty,
            });
        }
    } else {
        let diff = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: diff,
            op: IrBinOp::Sub,
            lhs: *x,
            rhs: Operand::Value(hi),
            ty,
        });

        let half = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: half,
            op: IrBinOp::LShr,
            lhs: Operand::Value(diff),
            rhs: Operand::Const(IrConst::I64(1)),
            ty,
        });

        let sum = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: sum,
            op: IrBinOp::Add,
            lhs: Operand::Value(half),
            rhs: Operand::Value(hi),
            ty,
        });

        if shift > 1 {
            insts.push(Instruction::BinOp {
                dest,
                op: IrBinOp::LShr,
                lhs: Operand::Value(sum),
                rhs: Operand::Const(IrConst::I64((shift - 1) as i64)),
                ty,
            });
        } else {
            insts.push(Instruction::Copy { dest, src: Operand::Value(sum) });
        }
    }

    Some(insts)
}

// ─── Signed division by constant (64-bit) ──────────────────────────────────

fn compute_signed_magic_64(d: i64) -> (i128, u32) {
    assert!(d >= 2);
    let ad = d as u64;
    let t: u64 = 0x8000000000000000u64;
    let anc = t - 1 - (t % ad);
    let mut p: u32 = 63;
    let mut q1: u128 = (1u128 << 63) / anc as u128;
    let mut r1: u128 = (1u128 << 63) - q1 * anc as u128;
    let mut q2: u128 = (1u128 << 63) / ad as u128;
    let mut r2: u128 = (1u128 << 63) - q2 * ad as u128;

    loop {
        p += 1;
        q1 *= 2;
        r1 *= 2;
        if r1 >= anc as u128 {
            q1 += 1;
            r1 -= anc as u128;
        }
        q2 *= 2;
        r2 *= 2;
        if r2 >= ad as u128 {
            q2 += 1;
            r2 -= ad as u128;
        }
        let delta = ad as u128 - r2;
        if q1 < delta || (q1 == delta && r1 == 0) {
            continue;
        }
        break;
    }

    // Sign-extend the 64-bit magic to 128-bit signed
    let magic64 = (q2 + 1) as u64;
    let magic = magic64 as i64 as i128;
    let shift = p - 64;
    (magic, shift)
}

fn expand_sdiv64(
    dest: Value,
    x: &Operand,
    d: i64,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    if d < 2 {
        return None;
    }

    if d > 0 && (d as u64).is_power_of_two() {
        let k = d.trailing_zeros();
        let mut insts = Vec::new();

        let sign = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: sign,
            op: IrBinOp::AShr,
            lhs: *x,
            rhs: Operand::Const(IrConst::I64(63)),
            ty: IrType::I64,
        });

        let bias = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: bias,
            op: IrBinOp::LShr,
            lhs: Operand::Value(sign),
            rhs: Operand::Const(IrConst::I64(64 - k as i64)),
            ty: IrType::I64,
        });

        let biased = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: biased,
            op: IrBinOp::Add,
            lhs: *x,
            rhs: Operand::Value(bias),
            ty: IrType::I64,
        });

        insts.push(Instruction::BinOp {
            dest,
            op: IrBinOp::AShr,
            lhs: Operand::Value(biased),
            rhs: Operand::Const(IrConst::I64(k as i64)),
            ty: IrType::I64,
        });

        return Some(insts);
    }

    let (magic, shift) = compute_signed_magic_64(d);
    let mut insts = Vec::new();

    let x128 = fresh_value(next_id);
    insts.push(Instruction::Cast {
        dest: x128,
        src: *x,
        from_ty: IrType::I64,
        to_ty: IrType::I128,
    });

    let product = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: product,
        op: IrBinOp::Mul,
        lhs: Operand::Value(x128),
        rhs: Operand::Const(IrConst::I128(magic)),
        ty: IrType::I128,
    });

    let hi128 = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: hi128,
        op: IrBinOp::AShr,
        lhs: Operand::Value(product),
        rhs: Operand::Const(IrConst::I128(64)),
        ty: IrType::I128,
    });

    let hi = fresh_value(next_id);
    insts.push(Instruction::Cast {
        dest: hi,
        src: Operand::Value(hi128),
        from_ty: IrType::I128,
        to_ty: IrType::I64,
    });

    // Hacker's Delight: If M < 0 (bit 63 set), add the dividend to the high part
    let adjusted_hi = if magic < 0 {
        let s = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: s,
            op: IrBinOp::Add,
            lhs: Operand::Value(hi),
            rhs: *x,
            ty: IrType::I64,
        });
        s
    } else {
        hi
    };

    let shifted = if shift > 0 {
        let s = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: s,
            op: IrBinOp::AShr,
            lhs: Operand::Value(adjusted_hi),
            rhs: Operand::Const(IrConst::I64(shift as i64)),
            ty: IrType::I64,
        });
        s
    } else {
        adjusted_hi
    };

    let sign_bit = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: sign_bit,
        op: IrBinOp::LShr,
        lhs: Operand::Value(shifted),
        rhs: Operand::Const(IrConst::I64(63)),
        ty: IrType::I64,
    });

    insts.push(Instruction::BinOp {
        dest,
        op: IrBinOp::Add,
        lhs: Operand::Value(shifted),
        rhs: Operand::Value(sign_bit),
        ty: IrType::I64,
    });

    Some(insts)
}

// ─── 64-bit modulo by constant ─────────────────────────────────────────────

fn expand_urem64(
    dest: Value,
    x: &Operand,
    d: u64,
    ty: IrType,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    if d < 2 || d.is_power_of_two() {
        return None;
    }

    let quotient = fresh_value(next_id);
    let mut insts = expand_udiv64(quotient, x, d, ty, next_id)?;

    let prod = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: prod,
        op: IrBinOp::Mul,
        lhs: Operand::Value(quotient),
        rhs: Operand::Const(IrConst::I64(d as i64)),
        ty,
    });

    insts.push(Instruction::BinOp {
        dest,
        op: IrBinOp::Sub,
        lhs: *x,
        rhs: Operand::Value(prod),
        ty,
    });

    Some(insts)
}

fn expand_srem64(
    dest: Value,
    x: &Operand,
    d: i64,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    if d < 2 {
        return None;
    }

    let quotient = fresh_value(next_id);
    let mut insts = expand_sdiv64(quotient, x, d, next_id)?;

    let prod = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: prod,
        op: IrBinOp::Mul,
        lhs: Operand::Value(quotient),
        rhs: Operand::Const(IrConst::I64(d)),
        ty: IrType::I64,
    });

    insts.push(Instruction::BinOp {
        dest,
        op: IrBinOp::Sub,
        lhs: *x,
        rhs: Operand::Value(prod),
        ty: IrType::I64,
    });

    Some(insts)
}

fn expand_sdiv64_neg(
    dest: Value,
    x: &Operand,
    pos_d: i64,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    let quotient = fresh_value(next_id);
    let mut insts = expand_sdiv64(quotient, x, pos_d, next_id)?;

    insts.push(Instruction::BinOp {
        dest,
        op: IrBinOp::Sub,
        lhs: Operand::Const(IrConst::I64(0)),
        rhs: Operand::Value(quotient),
        ty: IrType::I64,
    });

    Some(insts)
}

// ─── I64-promoted variants ─────────────────────────────────────────────────

fn expand_udiv32_in_i64(
    dest: Value,
    x: &Operand,
    d: u32,
    next_id: &mut u32,
) -> Option<(Vec<Instruction>, Operand)> {
    if d < 2 {
        return None;
    }

    let mut insts = Vec::new();

    // Explicitly zero-extend low 32 bits to protect against dirty upper bits from ABI register passing
    let x_masked = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: x_masked,
        op: IrBinOp::And,
        lhs: *x,
        rhs: Operand::Const(IrConst::I64(0xFFFFFFFF)),
        ty: IrType::I64,
    });
    let x_safe = Operand::Value(x_masked);

    if d.is_power_of_two() {
        let shift = d.trailing_zeros();
        insts.push(Instruction::BinOp {
            dest,
            op: IrBinOp::LShr,
            lhs: x_safe,
            rhs: Operand::Const(IrConst::I64(shift as i64)),
            ty: IrType::I64,
        });
        return Some((insts, x_safe));
    }

    let (magic, shift, needs_add) = compute_unsigned_magic_32(d)?;

    let product = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: product,
        op: IrBinOp::Mul,
        lhs: x_safe,
        rhs: Operand::Const(IrConst::I64(magic as i64)),
        ty: IrType::I64,
    });

    let hi = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: hi,
        op: IrBinOp::LShr,
        lhs: Operand::Value(product),
        rhs: Operand::Const(IrConst::I64(32)),
        ty: IrType::I64,
    });

    if !needs_add {
        if shift == 0 {
            insts.push(Instruction::Copy { dest, src: Operand::Value(hi) });
        } else {
            insts.push(Instruction::BinOp {
                dest,
                op: IrBinOp::LShr,
                lhs: Operand::Value(hi),
                rhs: Operand::Const(IrConst::I64(shift as i64)),
                ty: IrType::I64,
            });
        }
    } else {
        let diff = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: diff,
            op: IrBinOp::Sub,
            lhs: x_safe,
            rhs: Operand::Value(hi),
            ty: IrType::I64,
        });

        let half = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: half,
            op: IrBinOp::LShr,
            lhs: Operand::Value(diff),
            rhs: Operand::Const(IrConst::I64(1)),
            ty: IrType::I64,
        });

        let sum = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: sum,
            op: IrBinOp::Add,
            lhs: Operand::Value(half),
            rhs: Operand::Value(hi),
            ty: IrType::I64,
        });

        if shift > 1 {
            insts.push(Instruction::BinOp {
                dest,
                op: IrBinOp::LShr,
                lhs: Operand::Value(sum),
                rhs: Operand::Const(IrConst::I64((shift - 1) as i64)),
                ty: IrType::I64,
            });
        } else {
            insts.push(Instruction::Copy { dest, src: Operand::Value(sum) });
        }
    }

    Some((insts, x_safe))
}

fn expand_sdiv32_in_i64(
    dest: Value,
    x: &Operand,
    d: i32,
    next_id: &mut u32,
) -> Option<(Vec<Instruction>, Operand)> {
    if d < 2 {
        return None;
    }

    let mut insts = Vec::new();

    // Explicitly sign-extend low 32 bits into 64-bit signed integer
    let x_shl = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: x_shl,
        op: IrBinOp::Shl,
        lhs: *x,
        rhs: Operand::Const(IrConst::I64(32)),
        ty: IrType::I64,
    });
    let x_sext = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: x_sext,
        op: IrBinOp::AShr,
        lhs: Operand::Value(x_shl),
        rhs: Operand::Const(IrConst::I64(32)),
        ty: IrType::I64,
    });
    let x_safe = Operand::Value(x_sext);

    if d > 0 && (d as u32).is_power_of_two() {
        let k = d.trailing_zeros();

        let sign = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: sign,
            op: IrBinOp::AShr,
            lhs: x_safe,
            rhs: Operand::Const(IrConst::I64(63)),
            ty: IrType::I64,
        });

        let bias = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: bias,
            op: IrBinOp::LShr,
            lhs: Operand::Value(sign),
            rhs: Operand::Const(IrConst::I64(64 - k as i64)),
            ty: IrType::I64,
        });

        let biased = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: biased,
            op: IrBinOp::Add,
            lhs: x_safe,
            rhs: Operand::Value(bias),
            ty: IrType::I64,
        });

        insts.push(Instruction::BinOp {
            dest,
            op: IrBinOp::AShr,
            lhs: Operand::Value(biased),
            rhs: Operand::Const(IrConst::I64(k as i64)),
            ty: IrType::I64,
        });

        return Some((insts, x_safe));
    }

    let (magic, shift) = compute_signed_magic_32(d);

    let product = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: product,
        op: IrBinOp::Mul,
        lhs: x_safe,
        rhs: Operand::Const(IrConst::I64(magic as i64)),
        ty: IrType::I64,
    });

    let hi = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: hi,
        op: IrBinOp::AShr,
        lhs: Operand::Value(product),
        rhs: Operand::Const(IrConst::I64(32)),
        ty: IrType::I64,
    });

    let shifted = if shift > 0 {
        let s = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: s,
            op: IrBinOp::AShr,
            lhs: Operand::Value(hi),
            rhs: Operand::Const(IrConst::I64(shift as i64)),
            ty: IrType::I64,
        });
        s
    } else {
        hi
    };

    let sign_bit = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: sign_bit,
        op: IrBinOp::LShr,
        lhs: Operand::Value(shifted),
        rhs: Operand::Const(IrConst::I64(63)),
        ty: IrType::I64,
    });

    insts.push(Instruction::BinOp {
        dest,
        op: IrBinOp::Add,
        lhs: Operand::Value(shifted),
        rhs: Operand::Value(sign_bit),
        ty: IrType::I64,
    });

    Some((insts, x_safe))
}

fn expand_sdiv_neg(
    dest: Value,
    x: &Operand,
    pos_d: i32,
    ty: IrType,
    lhs_is_i32: &dyn Fn(&Operand) -> bool,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    let quotient = fresh_value(next_id);
    let mut insts = match ty {
        IrType::I32 => expand_sdiv32(quotient, x, pos_d, ty, next_id)?,
        IrType::I64 if lhs_is_i32(x) => expand_sdiv32_in_i64(quotient, x, pos_d, next_id)?.0,
        _ => return None,
    };

    insts.push(Instruction::BinOp {
        dest,
        op: IrBinOp::Sub,
        lhs: Operand::Const(IrConst::from_i64(0, ty)),
        rhs: Operand::Value(quotient),
        ty,
    });

    Some(insts)
}

fn expand_urem32_in_i64(
    dest: Value,
    x: &Operand,
    d: u32,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    if d < 2 || d.is_power_of_two() {
        return None;
    }

    let quotient = fresh_value(next_id);
    let (mut insts, x_safe) = expand_udiv32_in_i64(quotient, x, d, next_id)?;

    let prod = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: prod,
        op: IrBinOp::Mul,
        lhs: Operand::Value(quotient),
        rhs: Operand::Const(IrConst::I64(d as i64)),
        ty: IrType::I64,
    });

    insts.push(Instruction::BinOp {
        dest,
        op: IrBinOp::Sub,
        lhs: x_safe,
        rhs: Operand::Value(prod),
        ty: IrType::I64,
    });

    Some(insts)
}

fn expand_srem32_in_i64(
    dest: Value,
    x: &Operand,
    d: i32,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    if d < 2 {
        return None;
    }

    let quotient = fresh_value(next_id);
    let (mut insts, x_safe) = expand_sdiv32_in_i64(quotient, x, d, next_id)?;

    let prod = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: prod,
        op: IrBinOp::Mul,
        lhs: Operand::Value(quotient),
        rhs: Operand::Const(IrConst::I64(d as i64)),
        ty: IrType::I64,
    });

    insts.push(Instruction::BinOp {
        dest,
        op: IrBinOp::Sub,
        lhs: x_safe,
        rhs: Operand::Value(prod),
        ty: IrType::I64,
    });

    Some(insts)
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[inline]
    fn c_div_i32(a: i32, b: i32) -> i32 {
        a / b
    }

    #[inline]
    fn c_div_i64(a: i64, b: i64) -> i64 {
        a / b
    }

    #[test]
    fn test_unsigned_magic_exhaustive_small() {
        for d in 2u32..=200 {
            if d.is_power_of_two() {
                continue;
            }
            let (magic, shift, needs_add) = compute_unsigned_magic_32(d).unwrap();
            let test_vals: Vec<u32> = (0..1000u32)
                .chain(u32::MAX - 1000..=u32::MAX)
                .chain(vec![d - 1, d, d + 1, 2 * d, 0x7FFFFFFF, 0x80000000])
                .collect();
            for x in test_vals {
                let result = if !needs_add {
                    ((x as u64 * magic) >> 32 >> shift) as u32
                } else {
                    let hi = (x as u64 * magic) >> 32;
                    let diff = x as u64 - hi;
                    ((diff >> 1).wrapping_add(hi) >> (shift - 1)) as u32
                };
                assert_eq!(result, x / d, "Failed unsigned 32-bit for x={} d={}", x, d);
            }
        }
    }

    #[test]
    fn test_signed_magic_32_exhaustive_small() {
        for d in 2i32..=200 {
            if (d as u32).is_power_of_two() {
                continue;
            }
            let (magic, shift) = compute_signed_magic_32(d);
            let test_vals: Vec<i32> = (-1000..1000)
                .chain(std::iter::once(i32::MAX))
                .chain(std::iter::once(i32::MIN))
                .chain(std::iter::once(i32::MIN + 1))
                .chain(vec![-d, d, -d - 1, d + 1, -2 * d, 2 * d])
                .collect();
            for &x in &test_vals {
                let x64 = x as i64;
                let product = x64 * magic as i64;
                let hi = product >> 32;
                let shifted = hi >> shift;
                let sign_bit = (shifted as u64) >> 63;
                let result = (shifted + sign_bit as i64) as i32;
                assert_eq!(result, c_div_i32(x, d), "Failed signed 32-bit for x={} d={}", x, d);
            }
        }
    }

    #[test]
    fn test_unsigned_magic_64_exhaustive_small_divisors() {
        for d in 2u64..=100 {
            if d.is_power_of_two() {
                continue;
            }
            let (magic, shift, needs_add) = compute_unsigned_magic_64(d).unwrap();
            let test_vals: Vec<u64> = (0..1000u64)
                .chain(u64::MAX - 1000..=u64::MAX)
                .chain(std::iter::once(u32::MAX as u64))
                .chain(std::iter::once(u32::MAX as u64 + 1))
                .chain(vec![d - 1, d, d + 1, 2 * d, 0x7FFFFFFFFFFFFFFF, 0x8000000000000000])
                .collect();
            for x in test_vals {
                let result = if !needs_add {
                    ((x as u128 * magic) >> 64 >> shift) as u64
                } else {
                    let hi = ((x as u128 * magic) >> 64) as u64;
                    let diff = x - hi;
                    (diff >> 1).wrapping_add(hi) >> (shift - 1)
                };
                assert_eq!(result, x / d, "Failed unsigned 64-bit for x={} d={}", x, d);
            }
        }
    }

    #[test]
    fn test_signed_magic_64_critical_cases() {
        // Critical divisors with bit 63 set in magic number (e.g. 15, 21, 23, 25, 31)
        for &d in &[3i64, 7, 10, 15, 21, 23, 25, 30, 31, 100, 1_000_000_000_000] {
            let (magic, shift) = compute_signed_magic_64(d);
            let test_vals: Vec<i64> = vec![
                0, 1, -1, 2, -2, d - 1, d, d + 1, -d - 1, -d, -d + 1,
                1000, -1000, 0x7FFFFFFFFFFFFFFF, -0x8000000000000000, -0x7FFFFFFFFFFFFFFF,
            ];
            for &x in &test_vals {
                let x128 = x as i128;
                let product = x128 * magic;
                let hi = (product >> 64) as i64;
                let adjusted_hi = if magic < 0 { hi + x } else { hi };
                let shifted = adjusted_hi >> shift;
                let sign_bit = (shifted as u64) >> 63;
                let result = shifted + sign_bit as i64;
                assert_eq!(result, c_div_i64(x, d), "Failed signed 64-bit for x={} d={}", x, d);
            }
        }
    }

    #[test]
    fn test_signed_magic_64_exhaustive_small_divisors() {
        for d in 2i64..=100 {
            if (d as u64).is_power_of_two() {
                continue;
            }
            let (magic, shift) = compute_signed_magic_64(d);
            let test_vals: Vec<i64> = (-500..500i64)
                .chain(std::iter::once(i64::MAX))
                .chain(std::iter::once(i64::MIN))
                .chain(std::iter::once(i64::MIN + 1))
                .collect();
            for x in test_vals {
                let x128 = x as i128;
                let product = x128 * magic;
                let hi = (product >> 64) as i64;
                let adjusted_hi = if magic < 0 { hi + x } else { hi };
                let shifted = adjusted_hi >> shift;
                let sign_bit = (shifted as u64) >> 63;
                let result = shifted + sign_bit as i64;
                assert_eq!(result, c_div_i64(x, d), "Failed signed 64-bit for x={} d={}", x, d);
            }
        }
    }

    #[test]
    fn test_unsigned_magic_64_large_divisors() {
        let large_divisors: Vec<u64> = vec![
            u32::MAX as u64 + 1,
            0x100000001,
            0x123456789ABCDEF0,
            u64::MAX / 2,
            u64::MAX / 3,
            1_000_000_000_000,
        ];
        for d in large_divisors {
            if (d & (d - 1)) == 0 {
                continue;
            }
            if let Some((magic, shift, needs_add)) = compute_unsigned_magic_64(d) {
                for &x in &[0u64, 1, d - 1, d, d + 1, d * 2, u64::MAX, u64::MAX - 1] {
                    let result = if !needs_add {
                        ((x as u128 * magic) >> 64 >> shift) as u64
                    } else {
                        let hi = ((x as u128 * magic) >> 64) as u64;
                        let diff = x - hi;
                        (diff >> 1).wrapping_add(hi) >> (shift - 1)
                    };
                    assert_eq!(result, x / d, "Failed unsigned large for x={} d={}", x, d);
                }
            }
        }
    }

    #[test]
    fn test_signed_magic_64_large_divisors() {
        let large_divisors: Vec<i64> = vec![
            i32::MAX as i64 + 1,
            1_000_000_000_000,
            i64::MAX / 3,
        ];
        for d in large_divisors {
            if (d as u64).is_power_of_two() {
                continue;
            }
            let (magic, shift) = compute_signed_magic_64(d);
            let test_vals: Vec<i64> = vec![
                0, 1, -1, d - 1, d, d + 1, -d + 1, -d, -d - 1,
                i64::MAX, i64::MIN + 1, -1000, 1000,
            ];
            for x in test_vals {
                let x128 = x as i128;
                let product = x128 * magic;
                let hi = (product >> 64) as i64;
                let adjusted_hi = if magic < 0 { hi + x } else { hi };
                let shifted = adjusted_hi >> shift;
                let sign_bit = (shifted as u64) >> 63;
                let result = shifted + sign_bit as i64;
                assert_eq!(result, c_div_i64(x, d), "Failed signed large for x={} d={}", x, d);
            }
        }
    }

    #[test]
    fn test_negative_divisor_sdiv_and_srem() {
        for pos_d in 2i32..=50 {
            let neg_d = -pos_d;
            let (magic, shift) = compute_signed_magic_32(pos_d);
            let test_vals: Vec<i32> = (-500..500)
                .chain(std::iter::once(i32::MAX))
                .chain(std::iter::once(i32::MIN + 1))
                .collect();
            for &x in &test_vals {
                let x64 = x as i64;
                let product = x64 * magic as i64;
                let hi = product >> 32;
                let shifted = hi >> shift;
                let sign_bit = (shifted as u64) >> 63;
                let pos_result = (shifted + sign_bit as i64) as i32;
                let result = -pos_result;

                assert_eq!(result, c_div_i32(x, neg_d), "SDiv negative failed for x={} d={}", x, neg_d);
                assert_eq!(x % pos_d, x % neg_d, "SRem negative identity failed for x={} d={}", x, neg_d);
            }
        }
    }

    #[test]
    fn test_negative_divisor_sdiv64() {
        for pos_d in [2i64, 3, 7, 10, 15, 21, 25, 100, 1_000_000_000_000] {
            if (pos_d as u64).is_power_of_two() {
                continue;
            }
            let (magic, shift) = compute_signed_magic_64(pos_d);
            let test_vals: Vec<i64> = vec![
                0, 1, -1, 100, -100, i64::MAX, i64::MIN + 1,
            ];
            for x in test_vals {
                let x128 = x as i128;
                let product = x128 * magic;
                let hi = (product >> 64) as i64;
                let adjusted_hi = if magic < 0 { hi + x } else { hi };
                let shifted = adjusted_hi >> shift;
                let sign_bit = (shifted as u64) >> 63;
                let pos_result = shifted + sign_bit as i64;
                let result = -pos_result;
                assert_eq!(result, c_div_i64(x, -pos_d), "Failed sdiv64 neg for x={} d={}", x, -pos_d);
            }
        }
    }

    #[test]
    fn test_signed_pow2_division_and_modulo() {
        for k in 1..=30 {
            let d = 1i32 << k;
            for &x in &[-1000, -d - 1, -d, -d + 1, -1, 0, 1, d - 1, d, d + 1, 1000, i32::MAX, i32::MIN] {
                // Bias and shift
                let sign = x >> 31;
                let bias = ((sign as u32) >> (32 - k)) as i32;
                let quotient = (x.wrapping_add(bias)) >> k;
                let remainder = x.wrapping_sub(quotient.wrapping_mul(d));

                assert_eq!(quotient, c_div_i32(x, d), "Signed pow2 div failed for x={} k={}", x, k);
                assert_eq!(remainder, x % d, "Signed pow2 mod failed for x={} k={}", x, k);
            }
        }
    }

    #[test]
    fn test_sdiv32_in_i64_with_dirty_upper_bits() {
        for d in 2i32..=50 {
            if (d as u32).is_power_of_two() {
                continue;
            }
            let (magic, shift) = compute_signed_magic_32(d);
            for &x32 in &[-100, -7, -1, 0, 1, 7, 100, i32::MAX, i32::MIN] {
                // Inject upper 32-bit garbage
                let dirty_x64 = (0xDEADBEEFu64 << 32) | (x32 as u32 as u64);
                // Clean via sign-extension (x << 32 >> 32)
                let x_safe = ((dirty_x64 as i64) << 32) >> 32;

                let product = x_safe * magic as i64;
                let hi = product >> 32;
                let shifted = hi >> shift;
                let sign_bit = (shifted as u64) >> 63;
                let result = (shifted + sign_bit as i64) as i32;

                assert_eq!(result, c_div_i32(x32, d), "Dirty bits test failed for x={} d={}", x32, d);
            }
        }
    }

    #[test]
    fn test_udiv32_in_i64_with_dirty_upper_bits() {
        for d in 2u32..=50 {
            if d.is_power_of_two() {
                continue;
            }
            let (magic, shift, needs_add) = compute_unsigned_magic_32(d).unwrap();
            for &x32 in &[0u32, 1, 10, 100, u32::MAX - 1, u32::MAX] {
                let dirty_x64 = (0xCAFEBABEu64 << 32) | (x32 as u64);
                // Clean via mask (x & 0xFFFFFFFF)
                let x_safe = dirty_x64 & 0xFFFFFFFF;

                let product = x_safe * magic;
                let hi = (product >> 32) as u64;
                let result = if !needs_add {
                    (hi >> shift) as u32
                } else {
                    let diff = x_safe - hi;
                    (((diff >> 1) + hi) >> (shift - 1)) as u32
                };

                assert_eq!(result, x32 / d, "Dirty bits udiv failed for x={} d={}", x32, d);
            }
        }
    }
}
