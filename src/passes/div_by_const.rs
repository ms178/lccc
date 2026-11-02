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
//! 64-bit magic numbers need the high half of a 128-bit product: one
//! `IrBinOp::UMulHigh`/`SMulHigh` (x86-64 `mulq`/`imulq`, AArch64
//! `umulh`/`smulh`, RISC-V `mulhu`/`mulh`).  Spelling it as an I128 multiply
//! instead put I128 values in the function, and on x86-64 any I128 value costs
//! the whole function five registers of its allocation pool.  Unsigned
//! divisors >= 2^63 become `x >= d`.  The pass runs only on 64-bit targets
//! (`passes::mod` gates it off for i686).
//!
//! - Negative divisors are handled via identity: `x / -C == -(x / C)` and
//!   `x % -C == x % C` (in C, the sign of the remainder follows the dividend).

use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Value};

#[path = "div_by_const_exact.rs"]
mod exact;

thread_local! {
    static EXACT_ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

pub(crate) fn set_exact_enabled(enabled: bool) {
    EXACT_ENABLED.with(|cell| cell.set(enabled));
}

/// Transform division/modulo by constants in a single function.
pub(crate) fn div_by_const_function(func: &mut IrFunction) -> usize {
    let mut changes =
        EXACT_ENABLED.with(|enabled| if enabled.get() { exact::run(func) } else { 0 });
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
    let bounded = BoundedDivAnalysis::compute(func, max_id);
    let mut is_known_u32: Vec<bool> = vec![false; max_id + 1];
    let mut is_known_i32: Vec<bool> = vec![false; max_id + 1];

    for (id, &ub) in bounded.max_u64.iter().enumerate() {
        if ub <= u32::MAX as u64 {
            is_known_u32[id] = true;
        }
        if ub <= i32::MAX as u64 {
            is_known_i32[id] = true;
        }
    }

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
                Instruction::Cast {
                    dest,
                    from_ty,
                    to_ty,
                    ..
                } => {
                    let id = dest.0 as usize;
                    if id <= max_id {
                        // Widening to 64-bit
                        if from_ty.is_integer()
                            && from_ty.size() <= 4
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

    for (block_idx, block) in func.blocks.iter_mut().enumerate() {
        // Chained remainder decomposition: `(x % a) % b` is `x % b` whenever
        // `b` divides `a` — both are truncating remainders of the same sign
        // domain, and a value that differs from `x` by a multiple of `b` has
        // the same truncating remainder modulo `b` (the results live in the
        // same strict `(-|b|, |b|)` window, so equal congruence is equal
        // truncation, negatives included).  The inner remainder goes dead
        // when the outer was its only reader and the text-level dead sweep
        // retires the orphaned magic.  Pre-pass: operand retarget only, the
        // expansion loop below still sees and expands the outer remainder.
        {
            let mut last_def: std::collections::HashMap<
                Value,
                (usize, IrBinOp, Operand, i64, IrType),
            > = std::collections::HashMap::new();
            let mut rewrites: Vec<(usize, Operand)> = Vec::new();
            for (idx, inst) in block.instructions.iter().enumerate() {
                if let Instruction::BinOp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } = inst
                {
                    let outer_const = match rhs {
                        Operand::Const(c) => c.to_i64(),
                        _ => None,
                    };
                    if matches!(op, IrBinOp::SRem | IrBinOp::URem) {
                        if let (Some(b), Operand::Value(inner)) = (outer_const, lhs) {
                            if let Some((_, inner_op, inner_lhs, a, inner_ty)) =
                                last_def.get(&inner)
                            {
                                if *inner_op == *op
                                    && *inner_ty == *ty
                                    && matches!(
                                        inner_ty,
                                        IrType::I32 | IrType::U32 | IrType::I64 | IrType::U64
                                    )
                                    && *a > 0
                                    && b > 0
                                    && *a % b == 0
                                {
                                    rewrites.push((idx, inner_lhs.clone()));
                                }
                            }
                        }
                    }
                    if matches!(op, IrBinOp::SRem | IrBinOp::URem) {
                        if let (Some(a), Operand::Const(_)) = (outer_const, rhs) {
                            last_def.insert(*dest, (idx, *op, lhs.clone(), a, *ty));
                        }
                    }
                }
            }
            for (idx, new_lhs) in rewrites {
                if let Instruction::BinOp { lhs, .. } = &mut block.instructions[idx] {
                    *lhs = new_lhs;
                }
            }
        }
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
                Instruction::BinOp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => {
                    let const_val = match rhs {
                        Operand::Const(c) => c.to_i64(),
                        _ => None,
                    };

                    if let Some(divisor) = const_val {
                        let expanded = match op {
                            IrBinOp::SDiv | IrBinOp::SRem if is_signed_type_min(divisor, *ty) => {
                                Some(expand_by_signed_type_min(
                                    *dest,
                                    lhs,
                                    *op == IrBinOp::SRem,
                                    *ty,
                                    &mut next_id,
                                ))
                            }
                            IrBinOp::UDiv => {
                                let udivisor = divisor as u64;
                                if udivisor >= 2 {
                                    if let Some(exp) = bounded.try_expand_udiv(
                                        block_idx,
                                        *dest,
                                        lhs,
                                        udivisor,
                                        *ty,
                                        &mut next_id,
                                    ) {
                                        Some(exp)
                                    } else if udivisor <= u32::MAX as u64 {
                                        let d32 = udivisor as u32;
                                        match *ty {
                                            IrType::U32 => {
                                                expand_udiv32(*dest, lhs, d32, *ty, &mut next_id)
                                            }
                                            IrType::I64 | IrType::U64 if lhs_is_u32(lhs) => {
                                                expand_udiv32_in_i64(*dest, lhs, d32, &mut next_id)
                                                    .map(|(i, _)| i)
                                            }
                                            IrType::I64 | IrType::U64 => expand_udiv64(
                                                *dest,
                                                lhs,
                                                udivisor,
                                                *ty,
                                                &mut next_id,
                                            ),
                                            _ => None,
                                        }
                                    } else {
                                        match *ty {
                                            IrType::I64 | IrType::U64 => expand_udiv64(
                                                *dest,
                                                lhs,
                                                udivisor,
                                                *ty,
                                                &mut next_id,
                                            ),
                                            _ => None,
                                        }
                                    }
                                } else {
                                    None
                                }
                            }
                            IrBinOp::SDiv => {
                                if divisor > 1
                                    && matches!(ty, IrType::I32 | IrType::I64)
                                    && bounded.is_nonneg_operand(block_idx, lhs, *ty)
                                {
                                    bounded.expand_nonneg_sdiv(
                                        block_idx,
                                        *dest,
                                        lhs,
                                        divisor as u64,
                                        *ty,
                                        &mut next_id,
                                    )
                                } else if divisor < -1
                                    && divisor > i64::MIN
                                    && matches!(ty, IrType::I32 | IrType::I64)
                                    && bounded.is_nonneg_operand(block_idx, lhs, *ty)
                                {
                                    let pos_d = (-divisor) as u64;
                                    let q_pos = fresh_value(&mut next_id);
                                    bounded
                                        .expand_nonneg_sdiv(
                                            block_idx,
                                            q_pos,
                                            lhs,
                                            pos_d,
                                            *ty,
                                            &mut next_id,
                                        )
                                        .map(|mut insts| {
                                            insts.push(Instruction::BinOp {
                                                dest: *dest,
                                                op: IrBinOp::Sub,
                                                lhs: Operand::Const(IrConst::from_i64(0, *ty)),
                                                rhs: Operand::Value(q_pos),
                                                ty: *ty,
                                            });
                                            insts
                                        })
                                } else if divisor > 1 && divisor <= i32::MAX as i64 {
                                    match *ty {
                                        IrType::I32 => expand_sdiv32(
                                            *dest,
                                            lhs,
                                            divisor as i32,
                                            *ty,
                                            &mut next_id,
                                        ),
                                        IrType::I64 if lhs_is_i32(lhs) => expand_sdiv32_in_i64(
                                            *dest,
                                            lhs,
                                            divisor as i32,
                                            &mut next_id,
                                        )
                                        .map(|(i, _)| i),
                                        IrType::I64 => {
                                            expand_sdiv64(*dest, lhs, divisor, &mut next_id)
                                        }
                                        _ => None,
                                    }
                                } else if divisor > i32::MAX as i64 {
                                    match *ty {
                                        IrType::I64 => {
                                            expand_sdiv64(*dest, lhs, divisor, &mut next_id)
                                        }
                                        _ => None,
                                    }
                                } else if divisor < -1 && divisor > i64::MIN {
                                    // Safe negation: divisor is strictly between (i64::MIN, -1)
                                    let pos_divisor = -divisor;
                                    if pos_divisor <= i32::MAX as i64 {
                                        let pd = pos_divisor as i32;
                                        match *ty {
                                            IrType::I32 => expand_sdiv_neg(
                                                *dest,
                                                lhs,
                                                pd,
                                                *ty,
                                                &lhs_is_i32,
                                                &mut next_id,
                                            ),
                                            IrType::I64 if lhs_is_i32(lhs) => expand_sdiv_neg(
                                                *dest,
                                                lhs,
                                                pd,
                                                *ty,
                                                &lhs_is_i32,
                                                &mut next_id,
                                            ),
                                            IrType::I64 => expand_sdiv64_neg(
                                                *dest,
                                                lhs,
                                                pos_divisor,
                                                &mut next_id,
                                            ),
                                            _ => None,
                                        }
                                    } else {
                                        match *ty {
                                            IrType::I64 => expand_sdiv64_neg(
                                                *dest,
                                                lhs,
                                                pos_divisor,
                                                &mut next_id,
                                            ),
                                            _ => None,
                                        }
                                    }
                                } else {
                                    None
                                }
                            }
                            IrBinOp::URem => {
                                let udivisor = divisor as u64;
                                if udivisor >= 2 {
                                    if let Some(exp) = bounded.try_expand_urem(
                                        block_idx,
                                        *dest,
                                        lhs,
                                        udivisor,
                                        *ty,
                                        &mut next_id,
                                    ) {
                                        Some(exp)
                                    } else if udivisor <= u32::MAX as u64 {
                                        let d32 = udivisor as u32;
                                        match *ty {
                                            IrType::U32 => {
                                                expand_urem32(*dest, lhs, d32, *ty, &mut next_id)
                                            }
                                            IrType::I64 | IrType::U64 if lhs_is_u32(lhs) => {
                                                expand_urem32_in_i64(*dest, lhs, d32, &mut next_id)
                                            }
                                            IrType::I64 | IrType::U64 => expand_urem64(
                                                *dest,
                                                lhs,
                                                udivisor,
                                                *ty,
                                                &mut next_id,
                                            ),
                                            _ => None,
                                        }
                                    } else {
                                        match *ty {
                                            IrType::I64 | IrType::U64 => expand_urem64(
                                                *dest,
                                                lhs,
                                                udivisor,
                                                *ty,
                                                &mut next_id,
                                            ),
                                            _ => None,
                                        }
                                    }
                                } else {
                                    None
                                }
                            }
                            IrBinOp::SRem => {
                                let abs_d = if divisor > 1 {
                                    Some(divisor as u64)
                                } else if divisor < -1 && divisor > i64::MIN {
                                    Some((-divisor) as u64)
                                } else {
                                    None
                                };
                                if let Some(pos_d) = abs_d.filter(|_| {
                                    matches!(ty, IrType::I32 | IrType::I64)
                                        && bounded.is_nonneg_operand(block_idx, lhs, *ty)
                                }) {
                                    bounded.expand_nonneg_srem(
                                        block_idx,
                                        *dest,
                                        lhs,
                                        pos_d,
                                        *ty,
                                        &mut next_id,
                                    )
                                } else if divisor > 1 && divisor <= i32::MAX as i64 {
                                    let d32 = divisor as i32;
                                    match *ty {
                                        IrType::I32 => {
                                            expand_srem32(*dest, lhs, d32, *ty, &mut next_id)
                                        }
                                        IrType::I64 if lhs_is_i32(lhs) => {
                                            expand_srem32_in_i64(*dest, lhs, d32, &mut next_id)
                                        }
                                        IrType::I64 => {
                                            expand_srem64(*dest, lhs, divisor, &mut next_id)
                                        }
                                        _ => None,
                                    }
                                } else if divisor > i32::MAX as i64 {
                                    match *ty {
                                        IrType::I64 => {
                                            expand_srem64(*dest, lhs, divisor, &mut next_id)
                                        }
                                        _ => None,
                                    }
                                } else if divisor < -1 && divisor > i64::MIN {
                                    // In C: x % -C == x % C (sign follows dividend)
                                    let pos_divisor = -divisor;
                                    if pos_divisor <= i32::MAX as i64 {
                                        let pd = pos_divisor as i32;
                                        match *ty {
                                            IrType::I32 => {
                                                expand_srem32(*dest, lhs, pd, *ty, &mut next_id)
                                            }
                                            IrType::I64 if lhs_is_i32(lhs) => {
                                                expand_srem32_in_i64(*dest, lhs, pd, &mut next_id)
                                            }
                                            IrType::I64 => {
                                                expand_srem64(*dest, lhs, pos_divisor, &mut next_id)
                                            }
                                            _ => None,
                                        }
                                    } else {
                                        match *ty {
                                            IrType::I64 => {
                                                expand_srem64(*dest, lhs, pos_divisor, &mut next_id)
                                            }
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

// ─── Bounded & Dominating-Sign Analysis ────────────────────────────────────

struct BoundedDivAnalysis {
    /// Sound non-negative upper bound for each SSA `Value(id)` (`u64::MAX` = unknown or possibly negative).
    max_u64: Vec<u64>,
    /// Immediate dominator per block (`usize::MAX` if unreachable, `idom[0] == 0`).
    idom: Vec<usize>,
    /// Per-block single-predecessor edge facts proving `Value(id) >= 0`.
    dom_nonneg: Vec<Vec<u32>>,
    /// Whether `block_idx` belongs to a natural loop.
    in_loop: Vec<bool>,
    /// Whether `block_idx` is a load-free byte-store loop body (`generator_map` candidate).
    is_gen_map_candidate: Vec<bool>,
}

#[inline]
fn type_umax(ty: IrType) -> u64 {
    match ty {
        IrType::U8 => u8::MAX as u64,
        IrType::U16 => u16::MAX as u64,
        IrType::U32 => u32::MAX as u64,
        IrType::U64 => u64::MAX,
        IrType::I8 => u8::MAX as u64,
        IrType::I16 => u16::MAX as u64,
        IrType::I32 => u32::MAX as u64,
        IrType::I64 => u64::MAX,
        _ => u64::MAX,
    }
}

#[inline]
fn type_smax(ty: IrType) -> u64 {
    match ty {
        IrType::I8 => i8::MAX as u64,
        IrType::I16 => i16::MAX as u64,
        IrType::I32 => i32::MAX as u64,
        IrType::I64 => i64::MAX as u64,
        _ => 0,
    }
}

#[inline]
fn block_dominates(a: usize, mut b: usize, idom: &[usize]) -> bool {
    loop {
        if b == a {
            return true;
        }
        let next = *idom.get(b).unwrap_or(&usize::MAX);
        if next == usize::MAX || next == b {
            return false;
        }
        b = next;
    }
}

impl BoundedDivAnalysis {
    fn compute(func: &IrFunction, max_id: usize) -> Self {
        use crate::common::fx_hash::FxHashMap;
        use crate::ir::analysis::{build_cfg, build_label_map, compute_dominators};
        use crate::ir::reexports::Terminator;

        let num_blocks = func.blocks.len();
        let mut def_count = vec![0u32; max_id + 1];
        let mut cmp_defs: FxHashMap<u32, (IrCmpOp, Operand, Operand, IrType)> =
            FxHashMap::default();
        let mut cast_parents: FxHashMap<u32, u32> = FxHashMap::default();

        for block in &func.blocks {
            for inst in &block.instructions {
                if let Some(d) = inst.dest() {
                    let id = d.0 as usize;
                    if id <= max_id {
                        def_count[id] = def_count[id].saturating_add(1);
                    }
                }
                match inst {
                    Instruction::Cmp {
                        dest,
                        op,
                        lhs,
                        rhs,
                        ty,
                    } => {
                        cmp_defs.insert(dest.0, (*op, *lhs, *rhs, *ty));
                    }
                    Instruction::Cast {
                        dest,
                        src: Operand::Value(s),
                        from_ty,
                        to_ty,
                    } if from_ty.is_integer()
                        && to_ty.is_integer()
                        && !from_ty.is_unsigned()
                        && !to_ty.is_unsigned()
                        && to_ty.size() >= from_ty.size() =>
                    {
                        cast_parents.insert(dest.0, s.0);
                    }
                    _ => {}
                }
            }
        }

        let label_map = build_label_map(func);
        let (preds, succs) = build_cfg(func, &label_map);
        let idom = compute_dominators(num_blocks, &preds, &succs);

        // Identify natural-loop blocks via back-edges `tail -> header` where `header` dominates `tail`.
        let mut in_loop = vec![false; num_blocks];
        let mut loop_work: Vec<usize> = Vec::new();
        for tail in 0..num_blocks {
            if idom.get(tail).copied().unwrap_or(usize::MAX) == usize::MAX {
                continue;
            }
            for &succ in succs.row(tail) {
                let header = succ as usize;
                if block_dominates(header, tail, &idom) {
                    in_loop[header] = true;
                    if !in_loop[tail] {
                        in_loop[tail] = true;
                        loop_work.push(tail);
                    }
                    while let Some(cur) = loop_work.pop() {
                        if cur == header {
                            continue;
                        }
                        for &p in preds.row(cur) {
                            let pi = p as usize;
                            if !in_loop[pi]
                                && idom.get(pi).copied().unwrap_or(usize::MAX) != usize::MAX
                            {
                                in_loop[pi] = true;
                                loop_work.push(pi);
                            }
                        }
                    }
                }
            }
        }

        let mut is_gen_map_candidate = vec![false; num_blocks];
        for (bi, block) in func.blocks.iter().enumerate() {
            if !in_loop[bi] {
                continue;
            }
            let mut has_load_or_call = false;
            let mut byte_stores = 0usize;
            let mut other_stores = false;
            for inst in &block.instructions {
                match inst {
                    Instruction::Load { .. }
                    | Instruction::Call { .. }
                    | Instruction::CallIndirect { .. } => {
                        has_load_or_call = true;
                        break;
                    }
                    Instruction::Store { ty, .. } => {
                        if matches!(ty, IrType::U8 | IrType::I8) {
                            byte_stores += 1;
                        } else {
                            other_stores = true;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if !has_load_or_call && !other_stores && byte_stores == 1 {
                is_gen_map_candidate[bi] = true;
            }
        }

        // Gather single-predecessor edge facts proving `v >= 0`.
        let mut dom_nonneg: Vec<Vec<u32>> = vec![Vec::new(); num_blocks];
        for (pi, block) in func.blocks.iter().enumerate() {
            let Terminator::CondBranch {
                cond: Operand::Value(cv),
                true_label,
                false_label,
            } = block.terminator
            else {
                continue;
            };
            if true_label == false_label {
                continue;
            }
            if def_count.get(cv.0 as usize).copied().unwrap_or(2) > 1 {
                continue;
            }
            let Some(&(raw_op, lhs, rhs, cmp_ty)) = cmp_defs.get(&cv.0) else {
                continue;
            };
            if !cmp_ty.is_integer() || cmp_ty.is_unsigned() {
                continue;
            }
            let canon: Option<(IrCmpOp, Value, i64)> = match (lhs, rhs) {
                (Operand::Value(v), Operand::Const(c)) => c.to_i64().map(|ci| (raw_op, v, ci)),
                (Operand::Const(c), Operand::Value(v)) => {
                    let flipped = match raw_op {
                        IrCmpOp::Slt => Some(IrCmpOp::Sgt),
                        IrCmpOp::Sle => Some(IrCmpOp::Sge),
                        IrCmpOp::Sgt => Some(IrCmpOp::Slt),
                        IrCmpOp::Sge => Some(IrCmpOp::Sle),
                        _ => None,
                    };
                    flipped.and_then(|op| c.to_i64().map(|ci| (op, v, ci)))
                }
                _ => None,
            };
            let Some((op, val, c)) = canon else {
                continue;
            };
            if def_count.get(val.0 as usize).copied().unwrap_or(2) > 1 {
                continue;
            }
            let true_nonneg = (op == IrCmpOp::Sge && c >= 0) || (op == IrCmpOp::Sgt && c >= -1);
            let false_nonneg = (op == IrCmpOp::Slt && c >= 0) || (op == IrCmpOp::Sle && c >= -1);
            if true_nonneg {
                if let Some(&ti) = label_map.get(&true_label) {
                    let ti = ti as usize;
                    if preds.row(ti) == [pi as u32] && !dom_nonneg[ti].contains(&val.0) {
                        dom_nonneg[ti].push(val.0);
                    }
                }
            }
            if false_nonneg {
                if let Some(&fi) = label_map.get(&false_label) {
                    let fi = fi as usize;
                    if preds.row(fi) == [pi as u32] && !dom_nonneg[fi].contains(&val.0) {
                        dom_nonneg[fi].push(val.0);
                    }
                }
            }
        }

        let is_dom_nonneg = |b_idx: usize, mut vid: u32| -> bool {
            loop {
                let mut cur = b_idx;
                loop {
                    if let Some(list) = dom_nonneg.get(cur) {
                        if list.contains(&vid) {
                            return true;
                        }
                    }
                    let next = *idom.get(cur).unwrap_or(&usize::MAX);
                    if next == usize::MAX || next == cur {
                        break;
                    }
                    cur = next;
                }
                if let Some(&src_id) = cast_parents.get(&vid) {
                    if def_count.get(vid as usize).copied().unwrap_or(2) <= 1
                        && def_count.get(src_id as usize).copied().unwrap_or(2) <= 1
                    {
                        vid = src_id;
                        continue;
                    }
                }
                return false;
            }
        };

        // Monotone pessimistic greatest-fixpoint (`u64::MAX` top): every pass only
        // tightens sound upper bounds, so stopping after 4 passes is sound by induction.
        let mut max_u64 = vec![u64::MAX; max_id + 1];
        for _pass in 0..4 {
            let mut changed = false;
            for (bi, block) in func.blocks.iter().enumerate() {
                for inst in &block.instructions {
                    let Some(dest) = inst.dest() else {
                        continue;
                    };
                    let id = dest.0 as usize;
                    if id > max_id || def_count[id] != 1 {
                        continue;
                    }
                    let cand = Self::eval_inst_bound(inst, bi, &max_u64, &is_dom_nonneg);
                    if cand < max_u64[id] {
                        max_u64[id] = cand;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }

        // Propagate single-def signed widening casts (`I32 -> I64`) into `dom_nonneg`
        // for any child whose source is known non-negative.
        for (&child, &parent) in &cast_parents {
            if def_count.get(child as usize).copied().unwrap_or(2) == 1
                && def_count.get(parent as usize).copied().unwrap_or(2) <= 1
            {
                for list in &mut dom_nonneg {
                    if list.contains(&parent) && !list.contains(&child) {
                        list.push(child);
                    }
                }
            }
        }

        Self {
            max_u64,
            idom,
            dom_nonneg,
            in_loop,
            is_gen_map_candidate,
        }
    }

    fn op_bound_in(op: &Operand, ty: IrType, bounds: &[u64]) -> u64 {
        match op {
            Operand::Value(v) => bounds.get(v.0 as usize).copied().unwrap_or(u64::MAX),
            Operand::Const(c) => match c {
                IrConst::I128(v) => {
                    if *v >= 0 && *v <= u64::MAX as i128 {
                        *v as u64
                    } else {
                        u64::MAX
                    }
                }
                _ => {
                    let Some(ci) = c.to_i64() else {
                        return u64::MAX;
                    };
                    if ty.is_unsigned() {
                        match ty {
                            IrType::U8 => (ci as u8) as u64,
                            IrType::U16 => (ci as u16) as u64,
                            IrType::U32 => (ci as u32) as u64,
                            IrType::U64 => ci as u64,
                            _ => u64::MAX,
                        }
                    } else if ci >= 0 && (ci as u64) <= type_smax(ty) {
                        ci as u64
                    } else {
                        u64::MAX
                    }
                }
            },
        }
    }

    fn eval_inst_bound(
        inst: &Instruction,
        block_idx: usize,
        bounds: &[u64],
        is_dom_nonneg: &dyn Fn(usize, u32) -> bool,
    ) -> u64 {
        match inst {
            Instruction::ParamRef { ty, .. } | Instruction::Load { ty, .. } => match ty {
                IrType::U8 => u8::MAX as u64,
                IrType::U16 => u16::MAX as u64,
                IrType::U32 => u32::MAX as u64,
                _ => u64::MAX,
            },
            Instruction::Cmp { .. } => 1,
            Instruction::Copy { src, .. } => match src {
                Operand::Value(v) => bounds.get(v.0 as usize).copied().unwrap_or(u64::MAX),
                Operand::Const(c) => match c.to_i64() {
                    Some(v) if v >= 0 => v as u64,
                    _ => u64::MAX,
                },
            },
            Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            } if from_ty.is_integer() && to_ty.is_integer() && to_ty.size() <= 8 => {
                let b = Self::op_bound_in(src, *from_ty, bounds);
                let from_nonneg = if from_ty.is_unsigned() {
                    b < u64::MAX || from_ty.size() <= 4
                } else {
                    b <= type_smax(*from_ty)
                        || matches!(src, Operand::Value(v) if is_dom_nonneg(block_idx, v.0))
                };
                let b_eff = if from_ty.is_unsigned() && from_ty.size() <= 4 {
                    b.min(type_umax(*from_ty))
                } else if !from_ty.is_unsigned() && b == u64::MAX && from_nonneg {
                    type_smax(*from_ty)
                } else {
                    b
                };
                if from_nonneg {
                    if to_ty.size() >= from_ty.size() {
                        if to_ty.is_unsigned() || b_eff <= type_smax(*to_ty) {
                            b_eff
                        } else {
                            u64::MAX
                        }
                    } else if b_eff <= type_umax(*to_ty) {
                        if to_ty.is_unsigned() || b_eff <= type_smax(*to_ty) {
                            b_eff
                        } else {
                            u64::MAX
                        }
                    } else if to_ty.is_unsigned() {
                        type_umax(*to_ty)
                    } else {
                        u64::MAX
                    }
                } else if to_ty.is_unsigned() && to_ty.size() <= 4 {
                    type_umax(*to_ty)
                } else {
                    u64::MAX
                }
            }
            Instruction::Phi { incoming, ty, .. }
                if ty.is_integer() && ty.size() <= 8 && !incoming.is_empty() =>
            {
                let mut acc = 0u64;
                for (op, _) in incoming {
                    acc = acc.max(Self::op_bound_in(op, *ty, bounds));
                    if acc == u64::MAX {
                        break;
                    }
                }
                if ty.is_unsigned() {
                    if ty.size() <= 4 {
                        acc.min(type_umax(*ty))
                    } else {
                        acc
                    }
                } else if acc <= type_smax(*ty) {
                    acc
                } else {
                    u64::MAX
                }
            }
            Instruction::Select {
                true_val,
                false_val,
                ty,
                ..
            } if ty.is_integer() && ty.size() <= 8 => {
                let m = Self::op_bound_in(true_val, *ty, bounds)
                    .max(Self::op_bound_in(false_val, *ty, bounds));
                if ty.is_unsigned() {
                    if ty.size() <= 4 {
                        m.min(type_umax(*ty))
                    } else {
                        m
                    }
                } else if m <= type_smax(*ty) {
                    m
                } else {
                    u64::MAX
                }
            }
            Instruction::BinOp {
                op, lhs, rhs, ty, ..
            } if ty.is_integer() && ty.size() <= 8 => {
                let lim = if ty.is_unsigned() {
                    type_umax(*ty)
                } else {
                    type_smax(*ty)
                };
                let type_floor = if ty.is_unsigned() && ty.size() <= 4 {
                    type_umax(*ty)
                } else {
                    u64::MAX
                };
                let a = Self::op_bound_in(lhs, *ty, bounds);
                let b = Self::op_bound_in(rhs, *ty, bounds);
                let rhs_const = match rhs {
                    Operand::Const(c) => c.to_i64(),
                    _ => None,
                };
                match op {
                    IrBinOp::URem if ty.is_unsigned() => {
                        let d_opt = rhs_const.map(|c| {
                            if ty.size() == 4 {
                                (c as u32) as u64
                            } else if ty.size() == 2 {
                                (c as u16) as u64
                            } else if ty.size() == 1 {
                                (c as u8) as u64
                            } else {
                                c as u64
                            }
                        });
                        if let Some(d) = d_opt.filter(|&d| d >= 1) {
                            (d - 1).min(a).min(type_floor)
                        } else {
                            a.min(type_floor)
                        }
                    }
                    IrBinOp::SRem if !ty.is_unsigned() => {
                        let lhs_nn = a <= lim
                            || matches!(lhs, Operand::Value(v) if is_dom_nonneg(block_idx, v.0));
                        let abs_d = rhs_const.and_then(|c| {
                            if c > 0 {
                                Some(c as u64)
                            } else if c < 0 && c > i64::MIN {
                                Some((-c) as u64)
                            } else {
                                None
                            }
                        });
                        if let Some(d) = abs_d.filter(|&d| lhs_nn && d >= 1 && d - 1 <= lim) {
                            let a_eff = if a <= lim { a } else { lim };
                            (d - 1).min(a_eff)
                        } else {
                            u64::MAX
                        }
                    }
                    IrBinOp::UDiv if ty.is_unsigned() => {
                        let d_opt = rhs_const.map(|c| {
                            if ty.size() == 4 {
                                (c as u32) as u64
                            } else {
                                c as u64
                            }
                        });
                        if let Some(d) = d_opt.filter(|&d| d >= 1 && a <= lim) {
                            (a / d).min(type_floor)
                        } else {
                            type_floor
                        }
                    }
                    IrBinOp::SDiv if !ty.is_unsigned() => {
                        let lhs_nn = a <= lim
                            || matches!(lhs, Operand::Value(v) if is_dom_nonneg(block_idx, v.0));
                        if let Some(d) = rhs_const
                            .filter(|&c| c > 0)
                            .map(|c| c as u64)
                            .filter(|_| lhs_nn)
                        {
                            let a_eff = if a <= lim { a } else { lim };
                            a_eff / d
                        } else {
                            u64::MAX
                        }
                    }
                    IrBinOp::LShr => {
                        let bits = (ty.size() * 8) as i64;
                        if let Some(s) = rhs_const.filter(|&s| s >= 0 && s < bits) {
                            if ty.is_unsigned() {
                                let a_eff = a.min(type_umax(*ty));
                                a_eff >> (s as u32)
                            } else if s > 0 {
                                let a_eff = a.min(type_umax(*ty));
                                let shifted = a_eff >> (s as u32);
                                if shifted <= lim { shifted } else { u64::MAX }
                            } else if a <= lim {
                                a
                            } else {
                                u64::MAX
                            }
                        } else {
                            type_floor
                        }
                    }
                    IrBinOp::AShr if !ty.is_unsigned() => {
                        let bits = (ty.size() * 8) as i64;
                        let lhs_nn = a <= lim
                            || matches!(lhs, Operand::Value(v) if is_dom_nonneg(block_idx, v.0));
                        if let Some(s) = rhs_const.filter(|&s| lhs_nn && s >= 0 && s < bits) {
                            let a_eff = if a <= lim { a } else { lim };
                            a_eff >> (s as u32)
                        } else {
                            u64::MAX
                        }
                    }
                    IrBinOp::And => {
                        if a <= lim && b <= lim {
                            a.min(b)
                        } else if a <= lim {
                            a
                        } else if b <= lim {
                            b
                        } else {
                            type_floor
                        }
                    }
                    IrBinOp::Or | IrBinOp::Xor => {
                        if a <= lim && b <= lim {
                            let u = a | b;
                            let mask = if u == 0 {
                                0
                            } else {
                                u64::MAX >> u.leading_zeros()
                            };
                            if mask <= lim { mask } else { type_floor }
                        } else {
                            type_floor
                        }
                    }
                    IrBinOp::Add => {
                        if a <= lim && b <= lim {
                            a.checked_add(b).filter(|&s| s <= lim).unwrap_or(type_floor)
                        } else {
                            type_floor
                        }
                    }
                    IrBinOp::Mul => {
                        if a <= lim && b <= lim {
                            a.checked_mul(b).filter(|&p| p <= lim).unwrap_or(type_floor)
                        } else {
                            type_floor
                        }
                    }
                    IrBinOp::Shl => {
                        let bits = (ty.size() * 8) as i64;
                        if let Some(s) = rhs_const.filter(|&s| a <= lim && s >= 0 && s < bits) {
                            let su = s as u32;
                            a.checked_shl(su)
                                .filter(|&p| p <= lim && (su == 0 || (p >> su) == a))
                                .unwrap_or(type_floor)
                        } else {
                            type_floor
                        }
                    }
                    _ => type_floor,
                }
            }
            _ => u64::MAX,
        }
    }

    fn op_unsigned_bound(&self, op: &Operand, ty: IrType) -> u64 {
        match op {
            Operand::Value(v) => self.max_u64.get(v.0 as usize).copied().unwrap_or(u64::MAX),
            Operand::Const(c) => {
                let Some(ci) = c.to_i64() else {
                    return u64::MAX;
                };
                match ty.size() {
                    1 => (ci as u8) as u64,
                    2 => (ci as u16) as u64,
                    4 => (ci as u32) as u64,
                    _ => ci as u64,
                }
            }
        }
    }

    fn is_nonneg_in_block(&self, block_idx: usize, vid: u32) -> bool {
        let mut cur = block_idx;
        loop {
            if let Some(list) = self.dom_nonneg.get(cur) {
                if list.contains(&vid) {
                    return true;
                }
            }
            let next = *self.idom.get(cur).unwrap_or(&usize::MAX);
            if next == usize::MAX || next == cur {
                return false;
            }
            cur = next;
        }
    }

    fn is_nonneg_operand(&self, block_idx: usize, op: &Operand, ty: IrType) -> bool {
        let smax = type_smax(ty);
        match op {
            Operand::Value(v) => {
                let ub = self.max_u64.get(v.0 as usize).copied().unwrap_or(u64::MAX);
                ub <= smax || self.is_nonneg_in_block(block_idx, v.0)
            }
            Operand::Const(c) => c.to_i64().is_some_and(|ci| ci >= 0 && (ci as u64) <= smax),
        }
    }

    fn try_expand_udiv(
        &self,
        block_idx: usize,
        dest: Value,
        lhs: &Operand,
        udivisor: u64,
        ty: IrType,
        next_id: &mut u32,
    ) -> Option<Vec<Instruction>> {
        if !matches!(ty, IrType::U32 | IrType::U64 | IrType::I64) || udivisor < 2 {
            return None;
        }
        let max_x = self.op_unsigned_bound(lhs, ty);
        if max_x < udivisor {
            return Some(vec![Instruction::Copy {
                dest,
                src: Operand::Const(IrConst::from_i64(0, ty)),
            }]);
        }
        if udivisor.is_power_of_two() {
            return None;
        }
        let gen_map = self
            .is_gen_map_candidate
            .get(block_idx)
            .copied()
            .unwrap_or(false);
        let in_loop = self.in_loop.get(block_idx).copied().unwrap_or(false);
        if ty == IrType::U32 && udivisor <= 65_535 && max_x <= 65_535 {
            if gen_map {
                return expand_udiv32_u16_pure(
                    dest,
                    lhs,
                    udivisor as u32,
                    max_x as u32,
                    true,
                    next_id,
                );
            }
            if max_x < udivisor.saturating_mul(2) {
                return Some(emit_uge_quotient(dest, lhs, udivisor, ty, next_id));
            }
            return expand_udiv32_u16_pure(
                dest,
                lhs,
                udivisor as u32,
                max_x as u32,
                in_loop,
                next_id,
            );
        }
        if max_x < udivisor.saturating_mul(2) {
            return Some(emit_uge_quotient(dest, lhs, udivisor, ty, next_id));
        }
        None
    }

    fn try_expand_urem(
        &self,
        block_idx: usize,
        dest: Value,
        lhs: &Operand,
        udivisor: u64,
        ty: IrType,
        next_id: &mut u32,
    ) -> Option<Vec<Instruction>> {
        if !matches!(ty, IrType::U32 | IrType::U64 | IrType::I64) || udivisor < 2 {
            return None;
        }
        let max_x = self.op_unsigned_bound(lhs, ty);
        if max_x < udivisor {
            return Some(vec![Instruction::Copy { dest, src: *lhs }]);
        }
        if udivisor.is_power_of_two() {
            return None;
        }
        let gen_map = self
            .is_gen_map_candidate
            .get(block_idx)
            .copied()
            .unwrap_or(false);
        let in_loop = self.in_loop.get(block_idx).copied().unwrap_or(false);
        if ty == IrType::U32 && udivisor <= 65_535 && max_x <= 65_535 {
            if gen_map {
                return expand_urem32_u16_pure(
                    dest,
                    lhs,
                    udivisor as u32,
                    max_x as u32,
                    true,
                    next_id,
                );
            }
            if max_x < udivisor.saturating_mul(2) {
                return Some(emit_uge_remainder(dest, lhs, udivisor, ty, next_id));
            }
            return expand_urem32_u16_pure(
                dest,
                lhs,
                udivisor as u32,
                max_x as u32,
                in_loop,
                next_id,
            );
        }
        if max_x < udivisor.saturating_mul(2) {
            return Some(emit_uge_remainder(dest, lhs, udivisor, ty, next_id));
        }
        None
    }

    fn expand_nonneg_sdiv(
        &self,
        block_idx: usize,
        dest: Value,
        lhs: &Operand,
        udivisor: u64,
        ty: IrType,
        next_id: &mut u32,
    ) -> Option<Vec<Instruction>> {
        if udivisor < 2 {
            return None;
        }
        let smax = type_smax(ty);
        let max_x = self.op_unsigned_bound(lhs, ty).min(smax);
        let gen_map = self
            .is_gen_map_candidate
            .get(block_idx)
            .copied()
            .unwrap_or(false);
        let in_loop = self.in_loop.get(block_idx).copied().unwrap_or(false);

        if max_x < udivisor {
            return Some(vec![Instruction::Copy {
                dest,
                src: Operand::Const(IrConst::from_i64(0, ty)),
            }]);
        }
        if udivisor.is_power_of_two() {
            let k = udivisor.trailing_zeros();
            if ty == IrType::I32 && gen_map {
                let u_in = fresh_value(next_id);
                let u_out = fresh_value(next_id);
                return Some(vec![
                    Instruction::Cast {
                        dest: u_in,
                        src: *lhs,
                        from_ty: IrType::I32,
                        to_ty: IrType::U32,
                    },
                    Instruction::BinOp {
                        dest: u_out,
                        op: IrBinOp::LShr,
                        lhs: Operand::Value(u_in),
                        rhs: Operand::Const(IrConst::I32(k as i32)),
                        ty: IrType::U32,
                    },
                    Instruction::Cast {
                        dest,
                        src: Operand::Value(u_out),
                        from_ty: IrType::U32,
                        to_ty: IrType::I32,
                    },
                ]);
            }
            return Some(vec![Instruction::BinOp {
                dest,
                op: IrBinOp::AShr,
                lhs: *lhs,
                rhs: Operand::Const(IrConst::from_i64(k as i64, ty)),
                ty,
            }]);
        }

        match ty {
            IrType::I32 if udivisor <= i32::MAX as u64 => {
                if !gen_map && max_x < udivisor.saturating_mul(2) {
                    let ge = fresh_value(next_id);
                    return Some(vec![
                        Instruction::Cmp {
                            dest: ge,
                            op: IrCmpOp::Sge,
                            lhs: *lhs,
                            rhs: Operand::Const(IrConst::I32(udivisor as i32)),
                            ty: IrType::I32,
                        },
                        Instruction::Cast {
                            dest,
                            src: Operand::Value(ge),
                            from_ty: IrType::I8,
                            to_ty: IrType::I32,
                        },
                    ]);
                }
                let u_in = fresh_value(next_id);
                let u_out = fresh_value(next_id);
                let mut insts = vec![Instruction::Cast {
                    dest: u_in,
                    src: *lhs,
                    from_ty: IrType::I32,
                    to_ty: IrType::U32,
                }];
                let u_op = Operand::Value(u_in);
                let body = if udivisor <= 65_535 && max_x <= 65_535 {
                    expand_udiv32_u16_pure(
                        u_out,
                        &u_op,
                        udivisor as u32,
                        max_x as u32,
                        gen_map || in_loop,
                        next_id,
                    )
                    .or_else(|| {
                        expand_udiv32(u_out, &u_op, udivisor as u32, IrType::U32, next_id)
                    })?
                } else {
                    expand_udiv32(u_out, &u_op, udivisor as u32, IrType::U32, next_id)?
                };
                insts.extend(body);
                insts.push(Instruction::Cast {
                    dest,
                    src: Operand::Value(u_out),
                    from_ty: IrType::U32,
                    to_ty: IrType::I32,
                });
                Some(insts)
            }
            IrType::I64 => {
                if max_x < udivisor.saturating_mul(2) && udivisor <= i64::MAX as u64 {
                    let ge = fresh_value(next_id);
                    return Some(vec![
                        Instruction::Cmp {
                            dest: ge,
                            op: IrCmpOp::Sge,
                            lhs: *lhs,
                            rhs: Operand::Const(IrConst::I64(udivisor as i64)),
                            ty: IrType::I64,
                        },
                        Instruction::Cast {
                            dest,
                            src: Operand::Value(ge),
                            from_ty: IrType::I8,
                            to_ty: IrType::I64,
                        },
                    ]);
                }
                if max_x <= u32::MAX as u64 && udivisor <= u32::MAX as u64 {
                    expand_udiv32_in_i64(dest, lhs, udivisor as u32, next_id).map(|(i, _)| i)
                } else {
                    expand_udiv64(dest, lhs, udivisor, IrType::I64, next_id)
                }
            }
            _ => None,
        }
    }

    fn expand_nonneg_srem(
        &self,
        block_idx: usize,
        dest: Value,
        lhs: &Operand,
        udivisor: u64,
        ty: IrType,
        next_id: &mut u32,
    ) -> Option<Vec<Instruction>> {
        if udivisor < 2 {
            return None;
        }
        let smax = type_smax(ty);
        let max_x = self.op_unsigned_bound(lhs, ty).min(smax);
        let gen_map = self
            .is_gen_map_candidate
            .get(block_idx)
            .copied()
            .unwrap_or(false);
        let in_loop = self.in_loop.get(block_idx).copied().unwrap_or(false);

        if max_x < udivisor {
            return Some(vec![Instruction::Copy { dest, src: *lhs }]);
        }
        if udivisor.is_power_of_two() {
            return Some(vec![Instruction::BinOp {
                dest,
                op: IrBinOp::And,
                lhs: *lhs,
                rhs: Operand::Const(IrConst::from_i64((udivisor - 1) as i64, ty)),
                ty,
            }]);
        }

        match ty {
            IrType::I32 if udivisor <= i32::MAX as u64 => {
                if !gen_map && max_x < udivisor.saturating_mul(2) {
                    let ge = fresh_value(next_id);
                    let diff = fresh_value(next_id);
                    return Some(vec![
                        Instruction::Cmp {
                            dest: ge,
                            op: IrCmpOp::Sge,
                            lhs: *lhs,
                            rhs: Operand::Const(IrConst::I32(udivisor as i32)),
                            ty: IrType::I32,
                        },
                        Instruction::BinOp {
                            dest: diff,
                            op: IrBinOp::Sub,
                            lhs: *lhs,
                            rhs: Operand::Const(IrConst::I32(udivisor as i32)),
                            ty: IrType::I32,
                        },
                        Instruction::Select {
                            dest,
                            cond: Operand::Value(ge),
                            true_val: Operand::Value(diff),
                            false_val: *lhs,
                            ty: IrType::I32,
                        },
                    ]);
                }
                let u_in = fresh_value(next_id);
                let u_out = fresh_value(next_id);
                let mut insts = vec![Instruction::Cast {
                    dest: u_in,
                    src: *lhs,
                    from_ty: IrType::I32,
                    to_ty: IrType::U32,
                }];
                let u_op = Operand::Value(u_in);
                let body = if udivisor <= 65_535 && max_x <= 65_535 {
                    expand_urem32_u16_pure(
                        u_out,
                        &u_op,
                        udivisor as u32,
                        max_x as u32,
                        gen_map || in_loop,
                        next_id,
                    )
                    .or_else(|| {
                        expand_urem32(u_out, &u_op, udivisor as u32, IrType::U32, next_id)
                    })?
                } else {
                    expand_urem32(u_out, &u_op, udivisor as u32, IrType::U32, next_id)?
                };
                insts.extend(body);
                insts.push(Instruction::Cast {
                    dest,
                    src: Operand::Value(u_out),
                    from_ty: IrType::U32,
                    to_ty: IrType::I32,
                });
                Some(insts)
            }
            IrType::I64 => {
                if max_x < udivisor.saturating_mul(2) && udivisor <= i64::MAX as u64 {
                    let ge = fresh_value(next_id);
                    let diff = fresh_value(next_id);
                    return Some(vec![
                        Instruction::Cmp {
                            dest: ge,
                            op: IrCmpOp::Sge,
                            lhs: *lhs,
                            rhs: Operand::Const(IrConst::I64(udivisor as i64)),
                            ty: IrType::I64,
                        },
                        Instruction::BinOp {
                            dest: diff,
                            op: IrBinOp::Sub,
                            lhs: *lhs,
                            rhs: Operand::Const(IrConst::I64(udivisor as i64)),
                            ty: IrType::I64,
                        },
                        Instruction::Select {
                            dest,
                            cond: Operand::Value(ge),
                            true_val: Operand::Value(diff),
                            false_val: *lhs,
                            ty: IrType::I64,
                        },
                    ]);
                }
                if max_x <= u32::MAX as u64 && udivisor <= u32::MAX as u64 {
                    expand_urem32_in_i64(dest, lhs, udivisor as u32, next_id)
                } else {
                    expand_urem64(dest, lhs, udivisor, IrType::I64, next_id)
                }
            }
            _ => None,
        }
    }
}

fn emit_uge_quotient(
    dest: Value,
    x: &Operand,
    d: u64,
    ty: IrType,
    next_id: &mut u32,
) -> Vec<Instruction> {
    let ge = fresh_value(next_id);
    vec![
        Instruction::Cmp {
            dest: ge,
            op: IrCmpOp::Uge,
            lhs: *x,
            rhs: Operand::Const(IrConst::from_i64(d as i64, ty)),
            ty,
        },
        Instruction::Cast {
            dest,
            src: Operand::Value(ge),
            from_ty: IrType::I8,
            to_ty: ty,
        },
    ]
}

fn emit_uge_remainder(
    dest: Value,
    x: &Operand,
    d: u64,
    ty: IrType,
    next_id: &mut u32,
) -> Vec<Instruction> {
    let ge = fresh_value(next_id);
    let diff = fresh_value(next_id);
    vec![
        Instruction::Cmp {
            dest: ge,
            op: IrCmpOp::Uge,
            lhs: *x,
            rhs: Operand::Const(IrConst::from_i64(d as i64, ty)),
            ty,
        },
        Instruction::BinOp {
            dest: diff,
            op: IrBinOp::Sub,
            lhs: *x,
            rhs: Operand::Const(IrConst::from_i64(d as i64, ty)),
            ty,
        },
        Instruction::Select {
            dest,
            cond: Operand::Value(ge),
            true_val: Operand::Value(diff),
            false_val: *x,
            ty,
        },
    ]
}

/// Pure-`U32` division by constant `d` (`2 <= d <= 65535`, non-power-of-2)
/// when `x <= max_x <= 65535`.
///
/// Avoids widening to `U64` so that:
/// 1. 52,099 / 65,519 divisors use a 2-op 32-bit `Mul` + `LShr` (cheaper even in scalar code);
/// 2. Inside loops (`allow_add16`), the remaining 13,420 divisors use Hacker's Delight
///    16-bit add-fixup (`(x * m_lo) >> 16` + half-step), keeping every instruction in
///    `IrType::U32` so `vectorize_generator_map` can vectorize 32 lanes per iteration.
fn expand_udiv32_u16_pure(
    dest: Value,
    x: &Operand,
    d: u32,
    max_x: u32,
    allow_add16: bool,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    if !(2..=65_535).contains(&d) || d.is_power_of_two() || max_x > 65_535 {
        return None;
    }
    let d64 = d as u64;
    let mx64 = max_x as u64;
    let x1 = if mx64 >= d64 - 1 {
        ((mx64 - (d64 - 1)) / d64) * d64 + (d64 - 1)
    } else {
        mx64
    };
    let x2 = mx64;

    for s in 1u32..32 {
        let two_s = 1u64 << s;
        let m = two_s.div_ceil(d64);
        if mx64 * m <= u32::MAX as u64 && ((x1 * m) >> s) == x1 / d64 && ((x2 * m) >> s) == x2 / d64
        {
            let prod = fresh_value(next_id);
            return Some(vec![
                Instruction::BinOp {
                    dest: prod,
                    op: IrBinOp::Mul,
                    lhs: *x,
                    rhs: Operand::Const(IrConst::I32(m as u32 as i32)),
                    ty: IrType::U32,
                },
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::LShr,
                    lhs: Operand::Value(prod),
                    rhs: Operand::Const(IrConst::I32(s as i32)),
                    ty: IrType::U32,
                },
            ]);
        }
    }

    if !allow_add16 {
        return None;
    }

    for p in 1u32..=16 {
        let two_pow = 1u64 << (16 + p);
        let magic = two_pow.div_ceil(d64);
        let error = magic * d64 - two_pow;
        if error <= (1u64 << p) && magic > 0x1_0000 && magic <= 0x2_0000 {
            let m_lo = (magic - 0x1_0000) as u32;
            let prod = fresh_value(next_id);
            let t1 = fresh_value(next_id);
            let diff = fresh_value(next_id);
            let half = fresh_value(next_id);
            let mut insts = Vec::with_capacity(6);
            insts.push(Instruction::BinOp {
                dest: prod,
                op: IrBinOp::Mul,
                lhs: *x,
                rhs: Operand::Const(IrConst::I32(m_lo as i32)),
                ty: IrType::U32,
            });
            insts.push(Instruction::BinOp {
                dest: t1,
                op: IrBinOp::LShr,
                lhs: Operand::Value(prod),
                rhs: Operand::Const(IrConst::I32(16)),
                ty: IrType::U32,
            });
            insts.push(Instruction::BinOp {
                dest: diff,
                op: IrBinOp::Sub,
                lhs: *x,
                rhs: Operand::Value(t1),
                ty: IrType::U32,
            });
            insts.push(Instruction::BinOp {
                dest: half,
                op: IrBinOp::LShr,
                lhs: Operand::Value(diff),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::U32,
            });
            if p > 1 {
                let sum = fresh_value(next_id);
                insts.push(Instruction::BinOp {
                    dest: sum,
                    op: IrBinOp::Add,
                    lhs: Operand::Value(half),
                    rhs: Operand::Value(t1),
                    ty: IrType::U32,
                });
                insts.push(Instruction::BinOp {
                    dest,
                    op: IrBinOp::LShr,
                    lhs: Operand::Value(sum),
                    rhs: Operand::Const(IrConst::I32((p - 1) as i32)),
                    ty: IrType::U32,
                });
            } else {
                insts.push(Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs: Operand::Value(half),
                    rhs: Operand::Value(t1),
                    ty: IrType::U32,
                });
            }
            return Some(insts);
        }
    }

    None
}

fn expand_urem32_u16_pure(
    dest: Value,
    x: &Operand,
    d: u32,
    max_x: u32,
    allow_add16: bool,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    let q = fresh_value(next_id);
    let mut insts = expand_udiv32_u16_pure(q, x, d, max_x, allow_add16, next_id)?;
    let prod = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: prod,
        op: IrBinOp::Mul,
        lhs: Operand::Value(q),
        rhs: Operand::Const(IrConst::I32(d as i32)),
        ty: IrType::U32,
    });
    insts.push(Instruction::BinOp {
        dest,
        op: IrBinOp::Sub,
        lhs: *x,
        rhs: Operand::Value(prod),
        ty: IrType::U32,
    });
    Some(insts)
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
    let mut insts = Vec::with_capacity(16);

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
        let mut insts = Vec::with_capacity(16);

        // Truncating division floors towards zero, so a negative dividend needs
        // `2^k - 1` added before the arithmetic shift.  That is the sign
        // broadcast narrowed to k bits, and the sign has to be materialised
        // first: the top k bits of `x` are not the broadcast (`0x80000001` has
        // top bits `1000`, and rounding it needs `1111`).
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
    let mut insts = Vec::with_capacity(16);

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

    if (d as u32).is_power_of_two() {
        // `x % 2^k` truncating: add the top-`k`-bits bias, mask the low bits and
        // take the bias back out.  `t` is 0 for a non-negative `x` and `2^k - 1`
        // for a negative one, so the masked sum is `x mod 2^k` shifted by the
        // bias, and subtracting it restores the truncating sign.  Four
        // instructions, no quotient and no magic constant.
        let k = (d as u32).trailing_zeros();
        let mask = (1u32 << k) - 1;
        let sign = fresh_value(next_id);
        let bias = fresh_value(next_id);
        let biased = fresh_value(next_id);
        let masked = fresh_value(next_id);
        return Some(vec![
            Instruction::BinOp {
                dest: sign,
                op: IrBinOp::AShr,
                lhs: *x,
                rhs: Operand::Const(IrConst::I32(31)),
                ty,
            },
            Instruction::BinOp {
                dest: bias,
                op: IrBinOp::And,
                lhs: Operand::Value(sign),
                rhs: Operand::Const(IrConst::I32(mask as i32)),
                ty,
            },
            Instruction::BinOp {
                dest: biased,
                op: IrBinOp::Add,
                lhs: *x,
                rhs: Operand::Value(bias),
                ty,
            },
            Instruction::BinOp {
                dest: masked,
                op: IrBinOp::And,
                lhs: Operand::Value(biased),
                rhs: Operand::Const(IrConst::I32(mask as i32)),
                ty,
            },
            Instruction::BinOp {
                dest,
                op: IrBinOp::Sub,
                lhs: Operand::Value(masked),
                rhs: Operand::Value(bias),
                ty,
            },
        ]);
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

    // d >= 2^63: the quotient is 0 or 1, i.e. `x >= d` -- one compare and a
    // setcc (GCC and LLVM emit the same), cheaper than any magic sequence,
    // and the only expansion for the ~0.5% of these divisors whose magic
    // needs a 65th shift bit (`compute_unsigned_magic_64` gives up on them,
    // e.g. u64::MAX - 1, which then kept a `divq`).
    if d > i64::MAX as u64 {
        let ge = fresh_value(next_id);
        return Some(vec![
            Instruction::Cmp {
                dest: ge,
                op: IrCmpOp::Uge,
                lhs: *x,
                rhs: Operand::Const(IrConst::I64(d as i64)),
                ty,
            },
            Instruction::Cast {
                dest,
                src: Operand::Value(ge),
                from_ty: IrType::I8,
                to_ty: ty,
            },
        ]);
    }

    let (magic, shift, needs_add) = compute_unsigned_magic_64(d)?;
    let mut insts = Vec::with_capacity(16);

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

    // `compute_unsigned_magic_64` returns magic <= u64::MAX (the 65-bit
    // case comes back reduced, with `needs_add`), so the 64-bit high
    // multiply is exact.
    let hi_u64 = mul_high_64(x_u64, magic as u64 as i64, false, &mut insts, next_id);

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
            insts.push(Instruction::Copy {
                dest,
                src: Operand::Value(hi),
            });
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
            insts.push(Instruction::Copy {
                dest,
                src: Operand::Value(sum),
            });
        }
    }

    Some(insts)
}

/// `(x * magic) >> 64` -- high half of the 128-bit product, unsigned or
/// signed -- as one `UMulHigh` (typed U64) or `SMulHigh` (typed I64),
/// appended to `insts`.  The pass runs only on 64-bit targets (the
/// `!target.is_32bit()` gate in `passes::mod`), where every backend has the
/// instruction.
fn mul_high_64(
    x: Operand,
    magic: i64,
    signed: bool,
    insts: &mut Vec<Instruction>,
    next_id: &mut u32,
) -> Value {
    debug_assert!(!crate::common::types::target_is_32bit());
    let hi = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: hi,
        op: if signed {
            IrBinOp::SMulHigh
        } else {
            IrBinOp::UMulHigh
        },
        lhs: x,
        rhs: Operand::Const(IrConst::I64(magic)),
        ty: if signed { IrType::I64 } else { IrType::U64 },
    });
    hi
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

fn expand_sdiv64(dest: Value, x: &Operand, d: i64, next_id: &mut u32) -> Option<Vec<Instruction>> {
    if d < 2 {
        return None;
    }

    if d > 0 && (d as u64).is_power_of_two() {
        let k = d.trailing_zeros();
        let mut insts = Vec::with_capacity(16);

        // Same bias argument as the 32-bit path: the rounding correction is the
        // sign broadcast narrowed to k bits, which needs the sign first.
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
    let mut insts = Vec::with_capacity(16);

    // `magic` is the sign extension of the 64-bit magic, so `magic as i64`
    // is exact and the signed high multiply sees the same operand.
    let hi = mul_high_64(*x, magic as i64, true, &mut insts, next_id);

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

    // d >= 2^63: the quotient is 0 or 1, so the remainder is
    // `x >= d ? x - d : x` -- a compare and a cmov, not `(x >= d) * d`.
    if d > i64::MAX as u64 {
        let ge = fresh_value(next_id);
        let diff = fresh_value(next_id);
        return Some(vec![
            Instruction::Cmp {
                dest: ge,
                op: IrCmpOp::Uge,
                lhs: *x,
                rhs: Operand::Const(IrConst::I64(d as i64)),
                ty,
            },
            Instruction::BinOp {
                dest: diff,
                op: IrBinOp::Sub,
                lhs: *x,
                rhs: Operand::Const(IrConst::I64(d as i64)),
                ty,
            },
            Instruction::Select {
                dest,
                cond: Operand::Value(ge),
                true_val: Operand::Value(diff),
                false_val: *x,
                ty,
            },
        ]);
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

fn expand_srem64(dest: Value, x: &Operand, d: i64, next_id: &mut u32) -> Option<Vec<Instruction>> {
    if d < 2 {
        return None;
    }

    if (d as u64).is_power_of_two() {
        // Same four-instruction form as the 32-bit path, at 64 bits.
        let k = (d as u64).trailing_zeros();
        let mask = (1u64 << k) - 1;
        let sign = fresh_value(next_id);
        let bias = fresh_value(next_id);
        let biased = fresh_value(next_id);
        let masked = fresh_value(next_id);
        return Some(vec![
            Instruction::BinOp {
                dest: sign,
                op: IrBinOp::AShr,
                lhs: *x,
                rhs: Operand::Const(IrConst::I64(63)),
                ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: bias,
                op: IrBinOp::And,
                lhs: Operand::Value(sign),
                rhs: Operand::Const(IrConst::I64(mask as i64)),
                ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: biased,
                op: IrBinOp::Add,
                lhs: *x,
                rhs: Operand::Value(bias),
                ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: masked,
                op: IrBinOp::And,
                lhs: Operand::Value(biased),
                rhs: Operand::Const(IrConst::I64(mask as i64)),
                ty: IrType::I64,
            },
            Instruction::BinOp {
                dest,
                op: IrBinOp::Sub,
                lhs: Operand::Value(masked),
                rhs: Operand::Value(bias),
                ty: IrType::I64,
            },
        ]);
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

    let mut insts = Vec::with_capacity(16);

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
            insts.push(Instruction::Copy {
                dest,
                src: Operand::Value(hi),
            });
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
            insts.push(Instruction::Copy {
                dest,
                src: Operand::Value(sum),
            });
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

    let mut insts = Vec::with_capacity(16);

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

/// Whether `divisor` is the most negative value of the signed word `ty`
/// (`INT_MIN` for I32, `LLONG_MIN` for I64).
fn is_signed_type_min(divisor: i64, ty: IrType) -> bool {
    match ty {
        IrType::I32 => divisor == i64::from(i32::MIN),
        IrType::I64 => divisor == i64::MIN,
        _ => false,
    }
}

/// `x / MIN` and `x % MIN` for the word's most negative value.  Every
/// dividend but `MIN` itself has `|x| < |MIN|`, so the truncating quotient is
/// `x == MIN` and the remainder is `x`, except `0` at `MIN`.  One compare
/// (plus a `cmov` for the remainder) replaces an `idiv`, which costs 10-18
/// cycles on Golden Cove and Zen 4 and is not pipelined; GCC 16.2 emits the
/// same compare.  Neither result can trap: `MIN / MIN` is 1 and `MIN % MIN`
/// is 0, both representable.
///
/// The compare yields a C `int`, so the I64 quotient widens it exactly as the
/// front end widens `(long)(x == y)`.
fn expand_by_signed_type_min(
    dest: Value,
    x: &Operand,
    remainder: bool,
    ty: IrType,
    next_id: &mut u32,
) -> Vec<Instruction> {
    let min = match ty {
        IrType::I32 => IrConst::I32(i32::MIN),
        _ => IrConst::I64(i64::MIN),
    };
    let is_min = |dest| Instruction::Cmp {
        dest,
        op: IrCmpOp::Eq,
        lhs: *x,
        rhs: Operand::Const(min),
        ty,
    };
    if remainder {
        let hit = fresh_value(next_id);
        return vec![
            is_min(hit),
            Instruction::Select {
                dest,
                cond: Operand::Value(hit),
                true_val: Operand::Const(IrConst::from_i64(0, ty)),
                false_val: *x,
                ty,
            },
        ];
    }
    if ty == IrType::I32 {
        return vec![is_min(dest)];
    }
    let hit = fresh_value(next_id);
    vec![
        is_min(hit),
        Instruction::Cast {
            dest,
            src: Operand::Value(hit),
            from_ty: IrType::I32,
            to_ty: IrType::I64,
        },
    ]
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
    use crate::common::fx_hash::FxHashMap;

    #[inline]
    fn c_div_i32(a: i32, b: i32) -> i32 {
        a / b
    }

    #[inline]
    fn c_div_i64(a: i64, b: i64) -> i64 {
        a / b
    }

    /// Evaluate an expansion: BinOp through `IrBinOp::eval_i64`, Cmp through
    /// `IrCmpOp::eval_i64`, Cast as two's-complement extend/truncate, Copy.
    /// Values are held as i128 so a U64 is visibly zero- and an I64
    /// sign-extended.
    fn run_expansion(insts: &[Instruction], x: (Value, i64), dest: Value) -> i64 {
        let fit = |v: i128, ty: IrType| -> i128 {
            match ty {
                IrType::U64 => i128::from(v as u64),
                IrType::I8 => i128::from(v as i8),
                _ => i128::from(v as i64),
            }
        };
        let mut env: FxHashMap<u32, i128> = FxHashMap::default();
        env.insert(x.0.0, i128::from(x.1));
        let read = |env: &FxHashMap<u32, i128>, op: &Operand| match op {
            Operand::Value(v) => env[&v.0],
            Operand::Const(c) => c.to_i128().unwrap(),
        };
        for inst in insts {
            let (dest, value) = match inst {
                Instruction::BinOp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => {
                    assert!(
                        matches!(ty, IrType::I64 | IrType::U64),
                        "wide op in {insts:?}"
                    );
                    let r = op.eval_i64(read(&env, lhs) as i64, read(&env, rhs) as i64);
                    (dest, fit(i128::from(r.unwrap()), *ty))
                }
                Instruction::Cmp {
                    dest, op, lhs, rhs, ..
                } => {
                    let r = op.eval_i64(read(&env, lhs) as i64, read(&env, rhs) as i64);
                    (dest, i128::from(r))
                }
                Instruction::Cast {
                    dest,
                    src,
                    from_ty,
                    to_ty,
                } => (dest, fit(fit(read(&env, src), *from_ty), *to_ty)),
                Instruction::Copy { dest, src } => (dest, read(&env, src)),
                Instruction::Select {
                    dest,
                    cond,
                    true_val,
                    false_val,
                    ..
                } => {
                    let pick = if read(&env, cond) != 0 {
                        true_val
                    } else {
                        false_val
                    };
                    (dest, read(&env, pick))
                }
                other => panic!("unexpected expansion instruction {other:?}"),
            };
            env.insert(dest.0, value);
        }
        env[&dest.0] as i64
    }

    /// The 64-bit expansions are exact for every divisor class -- plain
    /// magic, 65-bit magic with the add fixup, negative signed magic, and
    /// unsigned divisors >= 2^63 (the `x >= d` compare; u64::MAX - 1 is one
    /// whose magic would need a 65th shift bit) -- and the magic forms use
    /// one `UMulHigh`/`SMulHigh` with no I128 value anywhere, which is what
    /// keeps the x86-64 register pool whole.
    #[test]
    fn div64_expansions_exact_with_mul_high() {
        let udivs = [
            3u64,
            5,
            7,
            10,
            11,
            25,
            641,
            1_000_000_007,
            1_000_000_000,
            i64::MAX as u64,
            1 << 63,
            1 << 63 | 1,
            u64::MAX - 1,
            u64::MAX,
        ];
        let sdivs = [
            3i64,
            5,
            6,
            7,
            10,
            11,
            125,
            1_000_000_000,
            i64::MAX,
            i64::MAX - 1,
        ];
        let uxs = [
            0u64,
            1,
            2,
            4,
            5,
            6,
            9,
            99,
            1000,
            1 << 32,
            (1 << 63) - 1,
            1 << 63,
            u64::MAX - 1,
            u64::MAX,
            0xDEAD_BEEF_CAFE_F00D,
        ];
        let sxs = [
            0i64,
            1,
            -1,
            2,
            -2,
            6,
            -6,
            7,
            -7,
            1000,
            -1000,
            i64::MAX,
            i64::MIN,
            i64::MIN + 1,
            -0x1234_5678_9ABC_DEF0,
        ];
        let (x, q) = (Value(1), Value(2));
        let has_op = |insts: &[Instruction], want: IrBinOp| {
            insts
                .iter()
                .any(|i| matches!(i, Instruction::BinOp { op, .. } if *op == want))
        };
        for &d in &udivs {
            let mut next = 10;
            let div = expand_udiv64(q, &Operand::Value(x), d, IrType::U64, &mut next).unwrap();
            let compare = d > i64::MAX as u64;
            let pow2 = d.is_power_of_two();
            assert_eq!(
                matches!(
                    div[0],
                    Instruction::Cmp {
                        op: IrCmpOp::Uge,
                        ..
                    }
                ),
                compare && !pow2,
                "udiv {d}: {div:?}"
            );
            assert_eq!(
                has_op(&div, IrBinOp::UMulHigh),
                !compare && !pow2,
                "udiv {d}: {div:?}"
            );
            for &v in &uxs {
                assert_eq!(
                    run_expansion(&div, (x, v as i64), q) as u64,
                    v / d,
                    "{v} / {d}"
                );
            }
            if pow2 {
                continue;
            }
            let mut next = 10;
            let rem = expand_urem64(q, &Operand::Value(x), d, IrType::U64, &mut next).unwrap();
            for &v in &uxs {
                assert_eq!(
                    run_expansion(&rem, (x, v as i64), q) as u64,
                    v % d,
                    "{v} % {d}"
                );
            }
        }
        for &d in &sdivs {
            let mut next = 10;
            let div = expand_sdiv64(q, &Operand::Value(x), d, &mut next).unwrap();
            assert!(has_op(&div, IrBinOp::SMulHigh), "sdiv {d}: {div:?}");
            for &v in &sxs {
                assert_eq!(
                    run_expansion(&div, (x, v), q),
                    v.wrapping_div(d),
                    "{v} / {d}"
                );
            }
            let mut next = 10;
            let rem = expand_srem64(q, &Operand::Value(x), d, &mut next).unwrap();
            for &v in &sxs {
                assert_eq!(
                    run_expansion(&rem, (x, v), q),
                    v.wrapping_rem(d),
                    "{v} % {d}"
                );
            }
        }
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
                assert_eq!(
                    result,
                    c_div_i32(x, d),
                    "Failed signed 32-bit for x={} d={}",
                    x,
                    d
                );
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
                .chain(vec![
                    d - 1,
                    d,
                    d + 1,
                    2 * d,
                    0x7FFFFFFFFFFFFFFF,
                    0x8000000000000000,
                ])
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
                0,
                1,
                -1,
                2,
                -2,
                d - 1,
                d,
                d + 1,
                -d - 1,
                -d,
                -d + 1,
                1000,
                -1000,
                0x7FFFFFFFFFFFFFFF,
                -0x8000000000000000,
                -0x7FFFFFFFFFFFFFFF,
            ];
            for &x in &test_vals {
                let x128 = x as i128;
                let product = x128 * magic;
                let hi = (product >> 64) as i64;
                let adjusted_hi = if magic < 0 { hi + x } else { hi };
                let shifted = adjusted_hi >> shift;
                let sign_bit = (shifted as u64) >> 63;
                let result = shifted + sign_bit as i64;
                assert_eq!(
                    result,
                    c_div_i64(x, d),
                    "Failed signed 64-bit for x={} d={}",
                    x,
                    d
                );
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
                assert_eq!(
                    result,
                    c_div_i64(x, d),
                    "Failed signed 64-bit for x={} d={}",
                    x,
                    d
                );
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
        let large_divisors: Vec<i64> = vec![i32::MAX as i64 + 1, 1_000_000_000_000, i64::MAX / 3];
        for d in large_divisors {
            if (d as u64).is_power_of_two() {
                continue;
            }
            let (magic, shift) = compute_signed_magic_64(d);
            let test_vals: Vec<i64> = vec![
                0,
                1,
                -1,
                d - 1,
                d,
                d + 1,
                -d + 1,
                -d,
                -d - 1,
                i64::MAX,
                i64::MIN + 1,
                -1000,
                1000,
            ];
            for x in test_vals {
                let x128 = x as i128;
                let product = x128 * magic;
                let hi = (product >> 64) as i64;
                let adjusted_hi = if magic < 0 { hi + x } else { hi };
                let shifted = adjusted_hi >> shift;
                let sign_bit = (shifted as u64) >> 63;
                let result = shifted + sign_bit as i64;
                assert_eq!(
                    result,
                    c_div_i64(x, d),
                    "Failed signed large for x={} d={}",
                    x,
                    d
                );
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

                assert_eq!(
                    result,
                    c_div_i32(x, neg_d),
                    "SDiv negative failed for x={} d={}",
                    x,
                    neg_d
                );
                assert_eq!(
                    x % pos_d,
                    x % neg_d,
                    "SRem negative identity failed for x={} d={}",
                    x,
                    neg_d
                );
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
            let test_vals: Vec<i64> = vec![0, 1, -1, 100, -100, i64::MAX, i64::MIN + 1];
            for x in test_vals {
                let x128 = x as i128;
                let product = x128 * magic;
                let hi = (product >> 64) as i64;
                let adjusted_hi = if magic < 0 { hi + x } else { hi };
                let shifted = adjusted_hi >> shift;
                let sign_bit = (shifted as u64) >> 63;
                let pos_result = shifted + sign_bit as i64;
                let result = -pos_result;
                assert_eq!(
                    result,
                    c_div_i64(x, -pos_d),
                    "Failed sdiv64 neg for x={} d={}",
                    x,
                    -pos_d
                );
            }
        }
    }

    #[test]
    fn test_signed_pow2_division_and_modulo() {
        for k in 1..=30 {
            let d = 1i32 << k;
            for &x in &[
                -1000,
                -d - 1,
                -d,
                -d + 1,
                -1,
                0,
                1,
                d - 1,
                d,
                d + 1,
                1000,
                i32::MAX,
                i32::MIN,
            ] {
                // Bias and shift
                let sign = x >> 31;
                let bias = ((sign as u32) >> (32 - k)) as i32;
                let quotient = (x.wrapping_add(bias)) >> k;
                let remainder = x.wrapping_sub(quotient.wrapping_mul(d));

                assert_eq!(
                    quotient,
                    c_div_i32(x, d),
                    "Signed pow2 div failed for x={} k={}",
                    x,
                    k
                );
                assert_eq!(
                    remainder,
                    x % d,
                    "Signed pow2 mod failed for x={} k={}",
                    x,
                    k
                );
            }
        }
    }

    /// Evaluate a 32-bit expansion directly, since `run_expansion` only models
    /// the wide sequences.
    fn run_expansion32(insts: &[Instruction], x: (Value, i32), dest: Value) -> i32 {
        let mut env: FxHashMap<u32, i32> = FxHashMap::default();
        env.insert(x.0.0, x.1);
        let read = |env: &FxHashMap<u32, i32>, op: &Operand| match op {
            Operand::Value(v) => env[&v.0],
            Operand::Const(c) => c.to_i128().unwrap() as i32,
        };
        for inst in insts {
            let (d, v) = match inst {
                Instruction::BinOp {
                    dest, op, lhs, rhs, ..
                } => {
                    let (a, b) = (read(&env, lhs), read(&env, rhs));
                    let value = match op {
                        IrBinOp::Add => a.wrapping_add(b),
                        IrBinOp::Sub => a.wrapping_sub(b),
                        IrBinOp::Mul => a.wrapping_mul(b),
                        IrBinOp::And => a & b,
                        IrBinOp::Or => a | b,
                        IrBinOp::Xor => a ^ b,
                        IrBinOp::Shl => ((a as u32) << (b as u32 & 31)) as i32,
                        IrBinOp::LShr => ((a as u32) >> (b as u32 & 31)) as i32,
                        IrBinOp::AShr => a >> (b as u32 & 31),
                        other => panic!("unexpected 32-bit operator {other:?} in {insts:?}"),
                    };
                    (*dest, value)
                }
                Instruction::Cmp {
                    dest, op, lhs, rhs, ..
                } => {
                    let (a, b) = (read(&env, lhs), read(&env, rhs));
                    let truth = match op {
                        IrCmpOp::Eq => a == b,
                        IrCmpOp::Ne => a != b,
                        IrCmpOp::Ult => (a as u32) < (b as u32),
                        IrCmpOp::Ule => (a as u32) <= (b as u32),
                        IrCmpOp::Ugt => (a as u32) > (b as u32),
                        IrCmpOp::Uge => (a as u32) >= (b as u32),
                        IrCmpOp::Slt => a < b,
                        IrCmpOp::Sle => a <= b,
                        IrCmpOp::Sgt => a > b,
                        IrCmpOp::Sge => a >= b,
                    };
                    (*dest, i32::from(truth))
                }
                Instruction::Copy { dest, src } => (*dest, read(&env, src)),
                Instruction::Select {
                    dest,
                    cond,
                    true_val,
                    false_val,
                    ..
                } => {
                    let pick = if read(&env, cond) != 0 {
                        true_val
                    } else {
                        false_val
                    };
                    (*dest, read(&env, pick))
                }
                other => panic!("unexpected instruction in a 32-bit expansion: {other:?}"),
            };
            env.insert(d.0, v);
        }
        env[&dest.0]
    }

    #[test]
    fn pow2_signed_expansions_match_truncating_semantics() {
        let x = Value(1);
        let q = Value(0);
        for k in 1..=30 {
            let d = 1i32 << k;
            let mut cases = vec![
                i32::MIN,
                i32::MIN + 1,
                i32::MAX,
                -1,
                0,
                1,
                -d - 1,
                -d,
                -d + 1,
                d - 1,
                d,
                d + 1,
                -12345,
                12345,
            ];
            cases.sort_unstable();
            cases.dedup();
            let mut next = 10;
            let rem = expand_srem32(q, &Operand::Value(x), d, IrType::I32, &mut next).unwrap();
            let mut next = 10;
            let div = expand_sdiv32(q, &Operand::Value(x), d, IrType::I32, &mut next).unwrap();
            assert_eq!(rem.len(), 5, "k={k}: {rem:?}");
            assert_eq!(div.len(), 4, "k={k}: {div:?}");
            // The bias is built by materialising the sign and then narrowing
            // it: the top k bits of the dividend are not the rounding mask.
            assert!(
                matches!(
                    rem[0],
                    Instruction::BinOp {
                        op: IrBinOp::AShr,
                        rhs: Operand::Const(_),
                        ..
                    }
                ),
                "k={k}: {rem:?}"
            );
            for &v in &cases {
                assert_eq!(
                    run_expansion32(&rem, (x, v), q),
                    v % d,
                    "srem32 {v} % {d}: {rem:?}"
                );
                assert_eq!(
                    run_expansion32(&div, (x, v), q),
                    v.wrapping_div(d),
                    "sdiv32 {v} / {d}: {div:?}"
                );
            }
        }
        for k in 1..=62 {
            let d = 1i64 << k;
            let mut cases = vec![
                i64::MIN,
                i64::MIN + 1,
                i64::MAX,
                -1,
                0,
                1,
                -d - 1,
                -d,
                -d + 1,
                d - 1,
                d,
                d + 1,
                -1234567890123,
                1234567890123,
            ];
            if d > 2 {
                cases.push(-(d / 2));
                cases.push(d / 2);
                cases.push(d / 2 - 1);
            }
            cases.sort_unstable();
            cases.dedup();
            let mut next = 10;
            let rem = expand_srem64(q, &Operand::Value(x), d, &mut next).unwrap();
            let mut next = 10;
            let div = expand_sdiv64(q, &Operand::Value(x), d, &mut next).unwrap();
            assert_eq!(rem.len(), 5, "k={k}: {rem:?}");
            assert_eq!(div.len(), 4, "k={k}: {div:?}");
            // The bias is built by materialising the sign and then narrowing
            // it: the top k bits of the dividend are not the rounding mask.
            assert!(
                matches!(
                    rem[0],
                    Instruction::BinOp {
                        op: IrBinOp::AShr,
                        rhs: Operand::Const(_),
                        ..
                    }
                ),
                "k={k}: {rem:?}"
            );
            for &v in &cases {
                assert_eq!(
                    run_expansion(&rem, (x, v), q),
                    v % d,
                    "srem64 {v} % {d}: {rem:?}"
                );
                assert_eq!(
                    run_expansion(&div, (x, v), q),
                    v.wrapping_div(d),
                    "sdiv64 {v} / {d}: {div:?}"
                );
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

                assert_eq!(
                    result,
                    c_div_i32(x32, d),
                    "Dirty bits test failed for x={} d={}",
                    x32,
                    d
                );
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

                assert_eq!(
                    result,
                    x32 / d,
                    "Dirty bits udiv failed for x={} d={}",
                    x32,
                    d
                );
            }
        }
    }

    /// `x / MIN` and `x % MIN` match C's truncating semantics at the
    /// boundaries, are a compare rather than an `idiv`, and the dispatcher
    /// routes both widths there.
    #[test]
    fn signed_type_min_divisor_is_a_compare() {
        let (x, q) = (Value(1), Value(0));
        let cases32 = [
            i32::MIN,
            i32::MIN + 1,
            -2,
            -1,
            0,
            1,
            2,
            i32::MAX - 1,
            i32::MAX,
        ];
        for remainder in [false, true] {
            let mut next = 2;
            let insts =
                expand_by_signed_type_min(q, &Operand::Value(x), remainder, IrType::I32, &mut next);
            assert!(insts.len() <= 2, "{insts:?}");
            for &v in &cases32 {
                let want = if remainder {
                    v.wrapping_rem(i32::MIN)
                } else {
                    v.wrapping_div(i32::MIN)
                };
                assert_eq!(
                    run_expansion32(&insts, (x, v), q),
                    want,
                    "x={v} rem={remainder}"
                );
            }
        }
        let cases64 = [
            i64::MIN,
            i64::MIN + 1,
            i64::from(i32::MIN),
            -1,
            0,
            1,
            i64::MAX,
        ];
        for remainder in [false, true] {
            let mut next = 2;
            let insts =
                expand_by_signed_type_min(q, &Operand::Value(x), remainder, IrType::I64, &mut next);
            assert!(insts.len() <= 2, "{insts:?}");
            for &v in &cases64 {
                let want = if remainder {
                    v.wrapping_rem(i64::MIN)
                } else {
                    v.wrapping_div(i64::MIN)
                };
                assert_eq!(
                    run_expansion(&insts, (x, v), q),
                    want,
                    "x={v} rem={remainder}"
                );
            }
        }
        assert!(is_signed_type_min(i64::from(i32::MIN), IrType::I32));
        assert!(is_signed_type_min(i64::MIN, IrType::I64));
        // An I64 division by INT_MIN is an ordinary power of two, not MIN.
        assert!(!is_signed_type_min(i64::from(i32::MIN), IrType::I64));
        assert!(!is_signed_type_min(i64::MIN, IrType::U64));
    }
}

#[cfg(test)]
mod nested_remainder_tests {
    use super::*;
    use crate::common::fx_hash::FxHashMap;
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};

    fn func_with(insts: Vec<Instruction>) -> IrFunction {
        let mut f = IrFunction::new("t".to_string(), IrType::Void, Vec::new(), false);
        f.params = vec![crate::ir::reexports::IrParam {
            ty: IrType::I32,
            noalias: false,
            struct_size: None,
            struct_align: None,
            param_align: None,
            struct_eightbyte_classes: Vec::new(),
            is_f128_sse: false,
            riscv_float_class: None,
        }];
        f.next_value_id = 256;
        f.blocks = vec![BasicBlock {
            label: BlockId(0),
            instructions: insts,
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        }];
        f
    }

    /// Interpret a post-pass instruction list: 32-bit ops in wrapping `i32`
    /// arithmetic, wider ops through `eval_i64`, `Cast` as two's-complement
    /// extend/truncate — the union both expansion families emit.
    fn run_mixed(insts: &[Instruction], seed: (Value, i32), dest: Value) -> i32 {
        let mut env: FxHashMap<u32, i64> = FxHashMap::default();
        env.insert(seed.0.0, i64::from(seed.1));
        let fit = |v: i64, ty: IrType| -> i64 {
            match ty {
                IrType::I32 | IrType::U32 => v as i32 as i64,
                IrType::I64 | IrType::U64 => v,
                other => panic!("unexpected type {other:?}"),
            }
        };
        let read = |env: &FxHashMap<u32, i64>, op: &Operand| match op {
            Operand::Value(v) => env[&v.0],
            Operand::Const(c) => c.to_i64().unwrap_or(0),
        };
        for inst in insts {
            let (d, v) = match inst {
                Instruction::BinOp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => {
                    let (a, b) = (read(&env, lhs), read(&env, rhs));
                    let raw = match ty {
                        IrType::I32 | IrType::U32 => {
                            let a32 = a as i32;
                            let b32 = b as i32;
                            let r = match op {
                                IrBinOp::Add => a32.wrapping_add(b32),
                                IrBinOp::Sub => a32.wrapping_sub(b32),
                                IrBinOp::Mul => a32.wrapping_mul(b32),
                                IrBinOp::And => a32 & b32,
                                IrBinOp::Or => a32 | b32,
                                IrBinOp::Xor => a32 ^ b32,
                                IrBinOp::Shl => a32.wrapping_shl(b32 as u32 & 31),
                                IrBinOp::LShr => ((a32 as u32) >> (b32 as u32 & 31)) as i32,
                                IrBinOp::AShr => a32.wrapping_shr(b32 as u32 & 31),
                                other => panic!("unexpected i32 op {other:?} in {insts:?}"),
                            };
                            i64::from(r)
                        }
                        _ => op.eval_i64(a, b).unwrap_or(0),
                    };
                    (*dest, fit(raw, *ty))
                }
                Instruction::Cast {
                    dest, src, to_ty, ..
                } => (*dest, fit(read(&env, src), *to_ty)),
                Instruction::Copy { dest, src } => (*dest, read(&env, src)),
                other => panic!("unexpected instruction in mixed expansion: {other:?}"),
            };
            env.insert(d.0, v);
        }
        env[&dest.0] as i32
    }

    #[test]
    fn the_nested_remainder_identity_holds_over_the_truncating_domain() {
        // (x % a) % b == x % b whenever b | a — truncating remainders on
        // both sides, negatives included.
        for a in [5i32, 10, 12, 100, 1000] {
            for b in 1..=a {
                if a % b != 0 {
                    continue;
                }
                for x in -2500..=2500 {
                    assert_eq!((x % a) % b, x % b, "a={a} b={b} x={x}");
                }
            }
        }
    }

    #[test]
    fn nested_remainder_retarges_the_outer_rem_to_the_dividend() {
        // (x % 10) % 5  →  x % 5 : the pass retargets the outer remainder
        // and the expansion interprets to the collapsed value.
        let x = Value(1);
        let inner = Value(2);
        let outer = Value(3);
        let mut f = func_with(vec![
            Instruction::BinOp {
                dest: inner,
                op: IrBinOp::SRem,
                lhs: Operand::Value(x),
                rhs: Operand::Const(IrConst::I32(10)),
                ty: IrType::I32,
            },
            Instruction::BinOp {
                dest: outer,
                op: IrBinOp::SRem,
                lhs: Operand::Value(inner),
                rhs: Operand::Const(IrConst::I32(5)),
                ty: IrType::I32,
            },
        ]);
        div_by_const_function(&mut f);
        let insts = &f.blocks[0].instructions;
        for &v in &[-1000i32, -101, -99, -10, -5, -1, 0, 1, 5, 10, 99, 101, 1000] {
            assert_eq!(
                run_mixed(insts, (x, v), outer),
                v % 5,
                "(x % 10) % 5 collapsed for x={v}: {insts:?}"
            );
            // The orphaned inner magic still computes its value; it is the
            // text-level dead sweep that removes it after lowering.
            assert_eq!(run_mixed(insts, (x, v), inner), v % 10, "x={v}");
        }
    }

    #[test]
    fn a_non_divisible_pair_is_left_alone() {
        // (x % 7) % 3 keeps its nesting: 3 does not divide 7.
        let x = Value(1);
        let inner = Value(2);
        let outer = Value(3);
        let mut f = func_with(vec![
            Instruction::BinOp {
                dest: inner,
                op: IrBinOp::SRem,
                lhs: Operand::Value(x),
                rhs: Operand::Const(IrConst::I32(7)),
                ty: IrType::I32,
            },
            Instruction::BinOp {
                dest: outer,
                op: IrBinOp::SRem,
                lhs: Operand::Value(inner),
                rhs: Operand::Const(IrConst::I32(3)),
                ty: IrType::I32,
            },
        ]);
        div_by_const_function(&mut f);
        let insts = &f.blocks[0].instructions;
        for &v in &[-500i32, -7, -3, -1, 0, 1, 3, 7, 500] {
            assert_eq!(
                run_mixed(insts, (x, v), outer),
                (v % 7) % 3,
                "nesting kept for x={v}: {insts:?}"
            );
        }
    }

    #[test]
    fn u16_pure_u32_div_and_rem_are_exact_across_full_u16_domain() {
        let x = Value(1);
        let out = Value(2);
        // Covers both Direct (3, 5, 10, 95, 255, 1000, 65521) and Add16 (7, 14, 19, 21, 31, 65519).
        for &d in &[
            3u32, 5, 7, 10, 14, 19, 21, 31, 95, 255, 1000, 65_519, 65_521,
        ] {
            let mut next = 10;
            let div_insts =
                expand_udiv32_u16_pure(out, &Operand::Value(x), d, 65_535, true, &mut next)
                    .expect("u16 pure u32 udiv must succeed");
            assert!(
                div_insts.iter().all(|i| matches!(
                    i,
                    Instruction::BinOp {
                        ty: IrType::U32,
                        ..
                    }
                )),
                "expected pure U32 BinOps for d={d}: {div_insts:?}"
            );
            let mut next_r = 10;
            let rem_insts =
                expand_urem32_u16_pure(out, &Operand::Value(x), d, 65_535, true, &mut next_r)
                    .expect("u16 pure u32 urem must succeed");
            for xv in 0u32..=65_535 {
                let q = run_mixed(&div_insts, (x, xv as i32), out) as u32;
                let r = run_mixed(&rem_insts, (x, xv as i32), out) as u32;
                assert_eq!(q, xv / d, "udiv mismatch for x={xv} d={d}");
                assert_eq!(r, xv % d, "urem mismatch for x={xv} d={d}");
            }
        }
    }

    #[test]
    fn dominating_nonneg_branch_promotes_srem64_to_unsigned() {
        // Block 0: if (len <s 0) goto B1 else goto B2
        // Block 2: r = SRem(len, 65521, I64) -> must use UMulHigh (not SMulHigh).
        let len = Value(1);
        let cmp = Value(2);
        let rem = Value(3);
        let mut f = IrFunction::new(
            "zng_combine_like".to_string(),
            IrType::I64,
            Vec::new(),
            false,
        );
        f.next_value_id = 32;
        f.blocks = vec![
            BasicBlock {
                label: BlockId(0),
                instructions: vec![
                    Instruction::ParamRef {
                        dest: len,
                        param_idx: 0,
                        ty: IrType::I64,
                    },
                    Instruction::Cmp {
                        dest: cmp,
                        op: IrCmpOp::Slt,
                        lhs: Operand::Value(len),
                        rhs: Operand::Const(IrConst::I64(0)),
                        ty: IrType::I64,
                    },
                ],
                terminator: Terminator::CondBranch {
                    cond: Operand::Value(cmp),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
                source_spans: Vec::new(),
            },
            BasicBlock {
                label: BlockId(1),
                instructions: Vec::new(),
                terminator: Terminator::Return(Some(Operand::Const(IrConst::I64(0)))),
                source_spans: Vec::new(),
            },
            BasicBlock {
                label: BlockId(2),
                instructions: vec![Instruction::BinOp {
                    dest: rem,
                    op: IrBinOp::SRem,
                    lhs: Operand::Value(len),
                    rhs: Operand::Const(IrConst::I64(65_521)),
                    ty: IrType::I64,
                }],
                terminator: Terminator::Return(Some(Operand::Value(rem))),
                source_spans: Vec::new(),
            },
        ];
        div_by_const_function(&mut f);
        let b2 = &f.blocks[2].instructions;
        assert!(
            b2.iter().any(|i| matches!(
                i,
                Instruction::BinOp {
                    op: IrBinOp::UMulHigh,
                    ..
                }
            )),
            "expected UMulHigh in nonneg branch block: {b2:?}"
        );
        assert!(
            !b2.iter().any(|i| matches!(
                i,
                Instruction::BinOp {
                    op: IrBinOp::SMulHigh,
                    ..
                }
            )),
            "unexpected signed SMulHigh in nonneg branch block: {b2:?}"
        );
    }

    #[test]
    fn loop_phi_upper_bound_reduces_adler_urem_to_select() {
        // Header B1: s1 = Phi((1, B0), (s1_next, B1)); s2 = Phi((0, B0), (s2_next, B1))
        //            b = Load(U8); bz = Cast(b, U8 -> U32)
        //            a1 = Add(s1, bz, U32); s1_next = URem(a1, 65521, U32)
        //            a2 = Add(s2, s1_next, U32); s2_next = URem(a2, 65521, U32)
        let mut f = IrFunction::new("adler_ref".to_string(), IrType::U32, Vec::new(), false);
        f.next_value_id = 64;
        f.blocks = vec![
            BasicBlock {
                label: BlockId(0),
                instructions: vec![Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 0,
                    ty: IrType::Ptr,
                }],
                terminator: Terminator::Branch(BlockId(1)),
                source_spans: Vec::new(),
            },
            BasicBlock {
                label: BlockId(1),
                instructions: vec![
                    Instruction::Phi {
                        dest: Value(2),
                        ty: IrType::U32,
                        incoming: vec![
                            (Operand::Const(IrConst::I32(1)), BlockId(0)),
                            (Operand::Value(Value(6)), BlockId(1)),
                        ],
                    },
                    Instruction::Phi {
                        dest: Value(3),
                        ty: IrType::U32,
                        incoming: vec![
                            (Operand::Const(IrConst::I32(0)), BlockId(0)),
                            (Operand::Value(Value(8)), BlockId(1)),
                        ],
                    },
                    Instruction::Load {
                        dest: Value(4),
                        ptr: Value(1),
                        ty: IrType::U8,
                        seg_override: crate::common::types::AddressSpace::Default,
                        volatile: false,
                    },
                    Instruction::Cast {
                        dest: Value(5),
                        src: Operand::Value(Value(4)),
                        from_ty: IrType::U8,
                        to_ty: IrType::U32,
                    },
                    Instruction::BinOp {
                        dest: Value(10),
                        op: IrBinOp::Add,
                        lhs: Operand::Value(Value(2)),
                        rhs: Operand::Value(Value(5)),
                        ty: IrType::U32,
                    },
                    Instruction::BinOp {
                        dest: Value(6),
                        op: IrBinOp::URem,
                        lhs: Operand::Value(Value(10)),
                        rhs: Operand::Const(IrConst::I32(65_521)),
                        ty: IrType::U32,
                    },
                    Instruction::BinOp {
                        dest: Value(11),
                        op: IrBinOp::Add,
                        lhs: Operand::Value(Value(3)),
                        rhs: Operand::Value(Value(6)),
                        ty: IrType::U32,
                    },
                    Instruction::BinOp {
                        dest: Value(8),
                        op: IrBinOp::URem,
                        lhs: Operand::Value(Value(11)),
                        rhs: Operand::Const(IrConst::I32(65_521)),
                        ty: IrType::U32,
                    },
                ],
                terminator: Terminator::Branch(BlockId(1)),
                source_spans: Vec::new(),
            },
        ];
        div_by_const_function(&mut f);
        let b1 = &f.blocks[1].instructions;
        let selects = b1
            .iter()
            .filter(|i| matches!(i, Instruction::Select { .. }))
            .count();
        let muls = b1
            .iter()
            .filter(|i| {
                matches!(
                    i,
                    Instruction::BinOp {
                        op: IrBinOp::Mul,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(
            selects, 2,
            "both adler URem ops must lower to Select: {b1:?}"
        );
        assert_eq!(
            muls, 0,
            "no multiply-by-magic should remain in loop: {b1:?}"
        );
    }
}
