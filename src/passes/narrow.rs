//! Integer narrowing optimization pass.
//!
//! The C compiler's lowering always promotes sub-64-bit operations to I64
//! (via C's integer promotion rules), then narrows the result back. This
//! produces a widen-operate-narrow pattern that generates unnecessary
//! sign-extension instructions and wastes registers.
//!
//! This pass detects the pattern:
//!   %w = Cast %x, T -> I64       (widening)
//!   %r = BinOp op %w, rhs, I64   (operation in I64)
//!   %n = Cast %r, I64 -> T       (narrowing)
//!
//! And converts it to:
//!   %r = BinOp op %x, narrow(rhs), T  (direct operation in T)
//!   (the narrowing Cast %n becomes dead, removed by DCE)
//!
//! Phase 4 (with explicit narrowing Cast) is safe for arithmetic ops
//! (Add, Sub, Mul, And, Or, Xor, Shl) because the Cast truncates the
//! result, and the low bits are identical regardless of operation width.
//! Right shifts (AShr, LShr) are also safe when the extension matches
//! the shift type: AShr with sign-extended LHS, LShr with zero-extended
//! LHS. This is because the extension bits are exactly what the shift
//! brings in, so the narrow-width shift produces the same low bits.
//! Phase 5 (no Cast) is restricted to bitwise ops (And, Or, Xor) since
//! arithmetic ops can produce different upper bits due to carries.
//!
//! Similarly, comparisons (Cmp) where both operands are widened from
//! the same type can be narrowed, since sign/zero extension preserves
//! the ordering of values.

use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand};

/// Information about a Cast instruction (widening).
#[derive(Clone)]
struct CastInfo {
    src: Operand,
    from_ty: IrType,
}

/// Information about a BinOp instruction.
#[derive(Clone)]
struct BinOpDef {
    op: IrBinOp,
    lhs: Operand,
    rhs: Operand,
}

/// Narrow operations in a single function.
pub(crate) fn narrow_function(func: &mut IrFunction) -> usize {
    // Early exit: the pass can only do work if the function contains I64/U64
    // BinOps or Cmps (which are the targets of narrowing). Skip the expensive
    // multi-pass analysis if none exist.
    let has_narrowable = func.blocks.iter().any(|block| {
        block.instructions.iter().any(|inst| match inst {
            Instruction::BinOp { ty, .. } => {
                matches!(ty, IrType::I64 | IrType::U64)
            }
            Instruction::Cmp { ty, .. } => {
                matches!(ty, IrType::I64 | IrType::U64)
            }
            _ => false,
        })
    });
    if !has_narrowable {
        return 0;
    }

    let max_id = func.max_value_id() as usize;
    let mut changes = 0;

    // Phase 1: Build a map of Value -> CastInfo for widening casts.
    // We only care about casts from a smaller integer type to I64/U64.
    let mut widen_map: Vec<Option<CastInfo>> = vec![None; max_id + 1];
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Cast {
                dest,
                src,
                from_ty,
                to_ty,
            } = inst
            {
                let is_widen = from_ty.is_integer()
                    && (*to_ty == IrType::I64 || *to_ty == IrType::U64)
                    && from_ty.size() < to_ty.size();
                if is_widen {
                    let id = dest.0 as usize;
                    if id <= max_id {
                        widen_map[id] = Some(CastInfo {
                            src: *src,
                            from_ty: *from_ty,
                        });
                    }
                }
            }
        }
    }

    // Phase 2: Build a map of Value -> defining BinOp instruction info.
    // We only care about BinOps in I64 that use widened operands.
    let mut binop_map: Vec<Option<BinOpDef>> = vec![None; max_id + 1];
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::BinOp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            } = inst
            {
                if *ty == IrType::I64 || *ty == IrType::U64 {
                    let id = dest.0 as usize;
                    if id <= max_id {
                        binop_map[id] = Some(BinOpDef {
                            op: *op,
                            lhs: *lhs,
                            rhs: *rhs,
                        });
                    }
                }
            }
        }
    }

    // Phase 3: Find narrowing casts whose source is a BinOp that uses
    // widened operands, and narrow the BinOp.
    //
    // We need to be careful: the BinOp result might have other uses
    // besides the narrowing cast. In that case, we cannot change the BinOp's
    // type. Instead, we only narrow when the BinOp result is ONLY used
    // by narrowing casts back to the original type.
    //
    // For simplicity, we use a two-step approach:
    // 1. Count uses of each value
    // 2. Only narrow when the BinOp has exactly one use (the narrowing cast)

    // Count uses of each value
    let mut use_counts: Vec<u32> = vec![0; max_id + 1];
    for block in &func.blocks {
        for inst in &block.instructions {
            inst.for_each_used_value(|id| {
                let idx = id as usize;
                if idx < use_counts.len() {
                    use_counts[idx] = use_counts[idx].saturating_add(1);
                }
            });
        }
        block.terminator.for_each_used_value(|id| {
            let idx = id as usize;
            if idx < use_counts.len() {
                use_counts[idx] = use_counts[idx].saturating_add(1);
            }
        });
    }

    // Values consumed by (or produced by) an `asm` operand must keep the
    // width the front end recorded in `InlineAsm::operand_types`.
    //
    // The backend allocates each asm operand's home from that declared type
    // and emits the corresponding move width.  Narrowing the PRODUCER of an
    // asm input changes only the store, never the asm's own load, so the two
    // disagree: the value is stored with `movl` (4 bytes) and read back with
    // `movq` (8 bytes), pulling four bytes of unrelated stack into the
    // operand register.
    //
    // Found building linux-cachymod 6.18.47:
    // `arch/x86/boot/compressed/string.c`'s `____memcpy`
    //
    //     asm volatile("rep movsq\n\t movq %4,%%rcx\n\t rep movsb"
    //                  : "=&c"(d0), "=&D"(d1), "=&S"(d2)
    //                  : "0"(n >> 3), "g"(n & 7), "1"(dest), "2"(src)
    //                  : "memory");
    //
    // Phase 5 narrowed `n & 7` (a single-use 64-bit AND) to 32 bits, so the
    // generated code stored `movl %eax,28(%rsp)` but the asm read
    // `movq 28(%rsp),%rdx`.  The garbage upper half became `%rcx` for the
    // trailing `rep movsb`, which then copied ~2^32 bytes.  In userspace that
    // is an immediate SIGSEGV; in the kernel's decompression stub it silently
    // corrupted the output buffer and the boot died with "ZSTD-compressed
    // data is corrupt" (`decompress_kernel -> zstd_decompress_dctx ->
    // handle_zstd_error -> error`).
    //
    // The narrowing itself is a legitimate optimization; what is illegal is
    // doing it to a value whose width is part of an opaque ABI contract with
    // hand-written assembly.  `asm` operands are exactly that, so pin them.
    let mut asm_pinned: Vec<bool> = vec![false; max_id + 1];
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::InlineAsm {
                outputs, inputs, ..
            } = inst
            {
                for (_, value, _) in outputs {
                    let idx = value.0 as usize;
                    if idx < asm_pinned.len() {
                        asm_pinned[idx] = true;
                    }
                }
                for (_, operand, _) in inputs {
                    if let Operand::Value(value) = operand {
                        let idx = value.0 as usize;
                        if idx < asm_pinned.len() {
                            asm_pinned[idx] = true;
                        }
                    }
                }
            }
        }
    }

    let mut narrowed_map: Vec<Option<IrType>> = vec![None; max_id + 1];

    changes += narrow_binops_with_cast(
        func,
        &binop_map,
        &use_counts,
        &widen_map,
        &mut narrowed_map,
        &asm_pinned,
    );
    changes += narrow_binops_without_cast(
        func,
        &use_counts,
        &widen_map,
        &mut narrowed_map,
        &asm_pinned,
    );
    changes += narrow_through_stores(func, &widen_map, &mut narrowed_map);
    changes += narrow_cmps(func, &widen_map);

    changes
}

/// Phase 6: Narrow I64/U64 BinOp chains whose results are consumed only by
/// truncating stores or truncating casts.
///
/// Lowering widens C `int` arithmetic to I64 on x86-64, but the low bits of
/// Add/Sub/Mul/And/Or/Xor/Shl are width-independent: computing them in a
/// 32-bit type T and storing to a T-sized slot is bit-identical. Removing
/// the widening lets the backend emit 32-bit ops (addl/xorl/andl) instead
/// of 64-bit ones and keeps hot loops in fewer registers — a big win in
/// hash/checksum loops (e.g. zlib-ng insert_string_roll, where
/// `(ins_h << 5) ^ val` and `& mask` were emitted as xorq/andq on
/// zero-extended 64-bit values).
///
/// A value v is narrowable-to-T when:
///   - it is a widening Cast{X→I64} with X == T (size 4), or a Load of type T;
///   - it is a BinOp in I64/U64 whose op is Add/Sub/Mul/And/Or/Xor (or Shl
///     with a constant count < T.width), whose operands are narrowable-to-T
///     (or constants representable in T);
/// and every use of it is a Store{ty: T}, a truncating Cast, or the operand
/// of another narrowable-to-T BinOp. Any other use (Cmp, Select, Call,
/// address arithmetic, terminator) disqualifies the value.
///
/// Restricting the target type to I32/U32 matches the backend's supported
/// operand widths (sub-int arithmetic is not codegen-able).
fn narrow_through_stores(
    func: &mut IrFunction,
    widen_map: &[Option<CastInfo>],
    narrowed_map: &mut [Option<IrType>],
) -> usize {
    if crate::common::types::target_is_32bit() {
        return 0;
    }
    let max_id = narrowed_map.len() - 1;

    // Value -> narrow type for Load instructions.
    let mut load_type_map: Vec<Option<IrType>> = vec![None; max_id + 1];
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Load { dest, ty, .. } = inst {
                let id = dest.0 as usize;
                if id <= max_id && ty.is_integer() && ty.size() == 4 {
                    load_type_map[id] = Some(*ty);
                }
            }
        }
    }

    // ntype[v]: the type to which v can be narrowed (def-side fixpoint).
    let mut ntype: Vec<Option<IrType>> = vec![None; max_id + 1];
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Cast {
                    dest,
                    from_ty,
                    to_ty,
                    ..
                } => {
                    if (*to_ty == IrType::I64 || *to_ty == IrType::U64)
                        && from_ty.is_integer()
                        && from_ty.size() == 4
                    {
                        let id = dest.0 as usize;
                        if id <= max_id {
                            ntype[id] = Some(*from_ty);
                        }
                    }
                }
                Instruction::Load { dest, ty, .. } => {
                    let id = dest.0 as usize;
                    if id <= max_id {
                        if let Some(t) = load_type_map[id] {
                            ntype[id] = Some(t);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    loop {
        let mut changed = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::BinOp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } = inst
                {
                    if !(*ty == IrType::I64 || *ty == IrType::U64) {
                        continue;
                    }
                    let id = dest.0 as usize;
                    if id > max_id || ntype[id].is_some() {
                        continue;
                    }
                    if let Some(t) = store_narrow_target(op, lhs, rhs, &ntype, &load_type_map) {
                        ntype[id] = Some(t);
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    // Use-side validation: every use of a narrowable value must truncate.
    // Disqualification is monotone; iterate until stable.
    loop {
        let mut disqualified = false;
        let mut mark_bad =
            |v: &Operand, ok: bool, ntype: &mut Vec<Option<IrType>>, max_id: usize| {
                if let Operand::Value(vv) = v {
                    let id = vv.0 as usize;
                    if id <= max_id && ntype[id].is_some() && !ok {
                        ntype[id] = None;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            };
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::Store { val, ptr, ty, .. } => {
                        let val_ok = match val {
                            Operand::Value(v) => {
                                let id = v.0 as usize;
                                if id <= max_id {
                                    match ntype[id] {
                                        Some(t) => t.size() == ty.size(),
                                        None => true,
                                    }
                                } else {
                                    true
                                }
                            }
                            _ => true,
                        };
                        if mark_bad(val, val_ok, &mut ntype, max_id) {
                            disqualified = true;
                        }
                        if mark_bad(&Operand::Value(*ptr), false, &mut ntype, max_id) {
                            disqualified = true;
                        }
                    }
                    Instruction::Cast {
                        src,
                        from_ty,
                        to_ty,
                        ..
                    } => {
                        let src_ok = match src {
                            Operand::Value(v) => {
                                let id = v.0 as usize;
                                if id <= max_id {
                                    match ntype[id] {
                                        Some(t) => {
                                            // Only a real truncation to T or a
                                            // real widening from T can consume
                                            // the low bits alone.  A same-width
                                            // I64<->U64 cast observes every bit:
                                            // accepting it used to narrow a
                                            // 64-bit multiply before `>> 32`
                                            // into `imull` + sign extension,
                                            // destroying the high product.
                                            let truncates_to_t = from_ty.size() > to_ty.size()
                                                && to_ty.size() == t.size();
                                            let widens_from_t = from_ty.size() == t.size()
                                                && to_ty.size() > from_ty.size();
                                            truncates_to_t || widens_from_t
                                        }
                                        None => true,
                                    }
                                } else {
                                    true
                                }
                            }
                            _ => true,
                        };
                        if mark_bad(src, src_ok, &mut ntype, max_id) {
                            disqualified = true;
                        }
                    }
                    Instruction::BinOp { dest, lhs, rhs, .. } => {
                        let did = dest.0 as usize;
                        let dest_narrow = if did <= max_id { ntype[did] } else { None };
                        for opnd in [lhs, rhs] {
                            let ok = match opnd {
                                Operand::Value(v) => {
                                    let id = v.0 as usize;
                                    if id <= max_id {
                                        match ntype[id] {
                                            Some(t) => {
                                                dest_narrow.is_some_and(|dt| dt.size() == t.size())
                                            }
                                            None => true,
                                        }
                                    } else {
                                        true
                                    }
                                }
                                _ => true,
                            };
                            if mark_bad(opnd, ok, &mut ntype, max_id) {
                                disqualified = true;
                            }
                        }
                    }
                    _ => {
                        let mut uses: Vec<u32> = Vec::new();
                        inst.for_each_used_value(|id| uses.push(id));
                        for id in uses {
                            let idx = id as usize;
                            if idx <= max_id && ntype[idx].is_some() {
                                ntype[idx] = None;
                                disqualified = true;
                            }
                        }
                    }
                }
            }
            let mut uses: Vec<u32> = Vec::new();
            block.terminator.for_each_used_value(|id| uses.push(id));
            for id in uses {
                let idx = id as usize;
                if idx <= max_id && ntype[idx].is_some() {
                    ntype[idx] = None;
                    disqualified = true;
                }
            }
        }
        if !disqualified {
            break;
        }
    }

    // Rewrite narrowable BinOps to T and forward stored narrowable values
    // through their widening casts.
    let mut changes = 0;
    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            match inst {
                Instruction::BinOp {
                    dest, lhs, rhs, ty, ..
                } => {
                    let id = dest.0 as usize;
                    if id <= max_id {
                        if let Some(t) = ntype[id] {
                            let nl = try_narrow_operand(
                                lhs,
                                t,
                                Some(&load_type_map),
                                widen_map,
                                narrowed_map,
                            );
                            let nr = try_narrow_operand(
                                rhs,
                                t,
                                Some(&load_type_map),
                                widen_map,
                                narrowed_map,
                            );
                            if let (Some(a), Some(b)) = (nl, nr) {
                                *lhs = a;
                                *rhs = b;
                                *ty = t;
                                narrowed_map[id] = Some(t);
                                changes += 1;
                            }
                        }
                    }
                }
                Instruction::Store { val, ty, .. } => {
                    if let Operand::Value(v) = val {
                        let id = v.0 as usize;
                        if id <= max_id {
                            if let Some(t) = ntype[id] {
                                if t.size() == ty.size() {
                                    if let Some(new_op) = try_narrow_operand(
                                        val,
                                        t,
                                        Some(&load_type_map),
                                        widen_map,
                                        narrowed_map,
                                    ) {
                                        *val = new_op;
                                        changes += 1;
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    changes
}

/// Compute the narrow target type of a BinOp's operands, or None.
fn store_narrow_target(
    op: &IrBinOp,
    lhs: &Operand,
    rhs: &Operand,
    ntype: &[Option<IrType>],
    load_type_map: &[Option<IrType>],
) -> Option<IrType> {
    let operand_t = |op: &Operand| -> Option<IrType> {
        match op {
            Operand::Value(v) => {
                let id = v.0 as usize;
                if id < ntype.len() {
                    if let Some(t) = ntype[id] {
                        return Some(t);
                    }
                }
                if id < load_type_map.len() {
                    if let Some(t) = load_type_map[id] {
                        return Some(t);
                    }
                }
                None
            }
            Operand::Const(_) => None,
        }
    };
    match op {
        IrBinOp::Add | IrBinOp::Sub | IrBinOp::Mul | IrBinOp::And | IrBinOp::Or | IrBinOp::Xor => {
            match (operand_t(lhs), operand_t(rhs)) {
                (Some(a), Some(b)) if a == b => Some(a),
                (Some(a), Some(b)) if a.size() == b.size() => Some(a),
                (Some(t), None) => {
                    if try_narrow_const_operand(rhs, t).is_some() {
                        Some(t)
                    } else {
                        None
                    }
                }
                (None, Some(t)) => {
                    if try_narrow_const_operand(lhs, t).is_some() {
                        Some(t)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }
        IrBinOp::Shl => {
            // Narrow only when the shift count is a constant < T.width bits;
            // 64-bit vs 32-bit shifts with larger/variable counts differ.
            let t = operand_t(lhs)?;
            let width_bits = (t.size() * 8) as i64;
            if let Operand::Const(c) = rhs {
                let val = match c {
                    IrConst::I64(v) => *v,
                    IrConst::I32(v) => *v as i64,
                    IrConst::I16(v) => *v as i64,
                    IrConst::I8(v) => *v as i64,
                    _ => return None,
                };
                if (0..width_bits).contains(&val) {
                    Some(t)
                } else {
                    None
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Phase 4: Narrow BinOps that have an explicit narrowing Cast consumer.
/// Finds `Cast(BinOp(widen(x), widen(y), I64), I64->T)` and replaces with
/// `BinOp(x, y, T)`. Safe for Add/Sub/Mul/And/Or/Xor/Shl because the
/// narrowing Cast truncates the result (low bits are width-independent).
/// Also safe for AShr when LHS was sign-extended from a signed target type,
/// and LShr when LHS was zero-extended from an unsigned target type.
fn narrow_binops_with_cast(
    func: &mut IrFunction,
    binop_map: &[Option<BinOpDef>],
    use_counts: &[u32],
    widen_map: &[Option<CastInfo>],
    narrowed_map: &mut [Option<IrType>],
    asm_pinned: &[bool],
) -> usize {
    let max_id = narrowed_map.len() - 1;
    let mut changes = 0;

    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            if let Instruction::Cast {
                dest,
                src: Operand::Value(src_val),
                from_ty,
                to_ty,
            } = inst
            {
                // Must be a narrowing cast from I64 to a smaller integer type
                if !((*from_ty == IrType::I64 || *from_ty == IrType::U64)
                    && to_ty.is_integer()
                    && to_ty.size() < from_ty.size())
                {
                    continue;
                }

                let src_id = src_val.0 as usize;
                if src_id > max_id {
                    continue;
                }

                let binop_info = match &binop_map[src_id] {
                    Some(info) => info.clone(),
                    None => continue,
                };

                // Shl is safe (shifting left only affects higher bits).
                // AShr/LShr are conditionally safe: right shifts bring in bits
                // from above, but if the LHS was widened from the target type
                // via matching extension (sign-ext for AShr, zero-ext for LShr),
                // the upper bits are just copies of the sign/zero bit, and the
                // lower bits of the result are identical to doing the shift in
                // the narrow type.
                let is_safe_op = matches!(
                    binop_info.op,
                    IrBinOp::Add
                        | IrBinOp::Sub
                        | IrBinOp::Mul
                        | IrBinOp::And
                        | IrBinOp::Or
                        | IrBinOp::Xor
                        | IrBinOp::Shl
                );
                let is_safe_shift = if binop_info.op == IrBinOp::AShr {
                    // AShr narrowing is safe when the LHS was sign-extended
                    // from a signed type of the same size as the target.
                    // sext(x, I32->I64) >> k has the same low 32 bits as
                    // x >> k (I32 arithmetic right shift).
                    to_ty.is_signed()
                        && is_widened_from_matching_type(&binop_info.lhs, *to_ty, widen_map)
                } else if binop_info.op == IrBinOp::LShr {
                    // LShr narrowing is safe when the LHS was zero-extended
                    // from an unsigned type of the same size as the target.
                    // zext(x, U32->U64) >> k has the same low 32 bits as
                    // x >> k (U32 logical right shift).
                    to_ty.is_unsigned()
                        && is_widened_from_matching_type(&binop_info.lhs, *to_ty, widen_map)
                } else {
                    false
                };
                if !is_safe_op && !is_safe_shift {
                    continue;
                }

                // Shift counts are the one operand whose *value* (not just its
                // low bits) decides whether narrowing is exact.  The width
                // argument above ("same low T bits") holds only for counts in
                // [0, T.width): `zext(x:U32):U64 >> 40` is 0 for every x, but
                // the narrowed `x >> 40` is out of range in U32 (x86 masks the
                // count to 8).  Likewise `Shl` by >= T.width has all-zero low
                // T bits.  Decide per count:
                //   * constant in range         -> narrow as usual
                //   * constant >= width (< 64)  -> fold: LShr/Shl to 0,
                //                                  AShr to `x >> (width-1)`
                //   * variable count            -> narrow only when the count
                //                                  is provably masked below
                //                                  the width (`n & c`, c<width)
                let mut shift_rhs_override: Option<Operand> = None;
                let mut fold_to_zero = false;
                if matches!(binop_info.op, IrBinOp::Shl | IrBinOp::LShr | IrBinOp::AShr) {
                    let width_bits = (to_ty.size() * 8) as i64;
                    match shift_count_class(&binop_info.rhs, width_bits, binop_map) {
                        ShiftCount::InRange => {}
                        ShiftCount::ConstOutOfRange => match binop_info.op {
                            IrBinOp::AShr => {
                                shift_rhs_override = Some(Operand::Const(IrConst::from_i64(
                                    width_bits - 1,
                                    *to_ty,
                                )));
                            }
                            _ => fold_to_zero = true,
                        },
                        ShiftCount::Unknown => continue,
                    }
                }

                // BinOp result must only be used by this narrowing cast
                if use_counts[src_id] != 1 {
                    continue;
                }

                // Never rewrite a value that an `asm` operand names, nor
                // produce a narrowed result into one: the operand's width is
                // fixed by `InlineAsm::operand_types` and the backend sizes
                // its home accordingly (see `asm_pinned` in `narrow_function`).
                if (src_id < asm_pinned.len() && asm_pinned[src_id])
                    || ((dest.0 as usize) < asm_pinned.len() && asm_pinned[dest.0 as usize])
                {
                    continue;
                }

                if fold_to_zero {
                    // The 64-bit shift produced a value whose low T bits are
                    // all zero; the truncating cast is therefore a constant.
                    // (The original BinOp becomes dead and DCE removes it.)
                    let narrow_dest = *dest;
                    let narrow_ty = *to_ty;
                    *inst = Instruction::Copy {
                        dest: narrow_dest,
                        src: Operand::Const(IrConst::zero(narrow_ty)),
                    };
                    changes += 1;
                    let dest_id = narrow_dest.0 as usize;
                    if dest_id <= max_id {
                        narrowed_map[dest_id] = Some(narrow_ty);
                    }
                    continue;
                }

                let narrow_lhs =
                    try_narrow_operand(&binop_info.lhs, *to_ty, None, widen_map, narrowed_map);
                let narrow_rhs = match &shift_rhs_override {
                    Some(o) => Some(o.clone()),
                    None => try_narrow_operand(&binop_info.rhs, *to_ty, None, widen_map, narrowed_map),
                };

                if let (Some(new_lhs), Some(new_rhs)) = (narrow_lhs, narrow_rhs) {
                    let narrow_dest = *dest;
                    let narrow_ty = *to_ty;

                    *inst = Instruction::BinOp {
                        dest: narrow_dest,
                        op: binop_info.op,
                        lhs: new_lhs,
                        rhs: new_rhs,
                        ty: narrow_ty,
                    };
                    changes += 1;

                    let dest_id = narrow_dest.0 as usize;
                    if dest_id <= max_id {
                        narrowed_map[dest_id] = Some(narrow_ty);
                    }
                }
            }
        }
    }

    changes
}

/// Phase 5: Narrow I64 BinOps whose operands are all sub-64-bit values,
/// even without an explicit narrowing Cast. Only safe for bitwise ops
/// (And/Or/Xor) and only on 64-bit targets (32-bit register ops don't
/// implicitly zero-extend to 64-bit on i686).
fn narrow_binops_without_cast(
    func: &mut IrFunction,
    use_counts: &[u32],
    widen_map: &[Option<CastInfo>],
    narrowed_map: &mut [Option<IrType>],
    asm_pinned: &[bool],
) -> usize {
    if crate::common::types::target_is_32bit() {
        return 0;
    }

    let max_id = narrowed_map.len() - 1;
    let mut changes = 0;

    // Build load_type_map: Value -> type for Load instructions
    let mut load_type_map: Vec<Option<IrType>> = vec![None; max_id + 1];
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Load { dest, ty, .. } = inst {
                let id = dest.0 as usize;
                if id <= max_id && ty.is_integer() && ty.size() < 8 {
                    load_type_map[id] = Some(*ty);
                }
            }
        }
    }

    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            if let Instruction::BinOp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            } = inst
            {
                if !(*ty == IrType::I64 || *ty == IrType::U64) {
                    continue;
                }
                let dest_id = dest.0 as usize;
                if dest_id >= use_counts.len() || use_counts[dest_id] != 1 {
                    continue;
                }
                // An `asm` operand's width is an ABI contract with hand-written
                // assembly (see `asm_pinned` in `narrow_function`): the backend
                // sizes the operand's home from `InlineAsm::operand_types`, so
                // narrowing its producer makes the store and the asm's load
                // disagree.  (kernel `____memcpy`: `movl` store vs `movq` load.)
                if dest_id < asm_pinned.len() && asm_pinned[dest_id] {
                    continue;
                }
                // Only bitwise ops are safe without an explicit narrowing Cast.
                if !matches!(op, IrBinOp::And | IrBinOp::Or | IrBinOp::Xor) {
                    continue;
                }

                let lhs_narrow_ty =
                    operand_narrow_type(lhs, &load_type_map, widen_map, narrowed_map);
                let rhs_narrow_ty =
                    operand_narrow_type(rhs, &load_type_map, widen_map, narrowed_map);
                // Keep the narrowed operand's signedness aligned with the
                // original 64-bit BinOp type. Picking the "other" 32-bit
                // signedness can change the value when widened back to 64 bits
                // (sign-extend vs zero-extend) even when low bits match
                // (Regehr yarpgen fix 4d9913e7).
                let op_narrow_ty = if *ty == IrType::I64 {
                    IrType::I32
                } else {
                    IrType::U32
                };

                let target_ty = match (lhs_narrow_ty, rhs_narrow_ty) {
                    (Some(lt), Some(rt)) if lt == rt && lt == op_narrow_ty => lt,
                    (Some(t), None) => {
                        if t == op_narrow_ty && try_narrow_const_operand(rhs, t).is_some() {
                            t
                        } else {
                            continue;
                        }
                    }
                    (None, Some(t)) => {
                        if t == op_narrow_ty && try_narrow_const_operand(lhs, t).is_some() {
                            t
                        } else {
                            continue;
                        }
                    }
                    _ => continue,
                };

                // Only narrow to I32/U32 (sub-int types are unsafe without
                // an explicit Cast; see Phase 4 for sub-int handling).
                if target_ty.size() < 4 {
                    continue;
                }

                // For AND, don't narrow when the constant operand would become
                // all-ones for the target type. Such an AND acts as a zero-extension
                // mask (e.g., `uint32_val & 0xFFFFFFFFUL` zero-extends to 64 bits).
                // Narrowing would make the AND a no-op (x & all_ones = x) which the
                // simplify pass removes, losing the zero-extension. On RISC-V, this
                // causes sign-extended 32-bit values to leak into 64-bit results.
                if *op == IrBinOp::And {
                    let const_becomes_all_ones = |operand: &Operand| -> bool {
                        if let Some(narrowed_const) = try_narrow_const_operand(operand, target_ty) {
                            let val = match narrowed_const {
                                IrConst::I64(v) => v,
                                IrConst::I32(v) => v as i64,
                                IrConst::I16(v) => v as i64,
                                IrConst::I8(v) => v as i64,
                                _ => return false,
                            };
                            target_ty.truncate_i64(val) == target_ty.truncate_i64(-1)
                        } else {
                            false
                        }
                    };
                    if const_becomes_all_ones(lhs) || const_becomes_all_ones(rhs) {
                        continue;
                    }
                }

                let new_lhs = try_narrow_operand(
                    lhs,
                    target_ty,
                    Some(&load_type_map),
                    widen_map,
                    narrowed_map,
                );
                let new_rhs = try_narrow_operand(
                    rhs,
                    target_ty,
                    Some(&load_type_map),
                    widen_map,
                    narrowed_map,
                );
                if let (Some(nl), Some(nr)) = (new_lhs, new_rhs) {
                    *lhs = nl;
                    *rhs = nr;
                    *ty = target_ty;
                    narrowed_map[dest_id] = Some(target_ty);
                    changes += 1;
                }
            }
        }
    }

    changes
}

/// Phase 6: Narrow Cmp instructions where both operands are widened from
/// the same type. Only safe when the extension kind matches the comparison
/// kind (signed cmp needs signed extension, unsigned cmp needs unsigned).
/// Eq/Ne are safe with either extension kind.
fn narrow_cmps(func: &mut IrFunction, widen_map: &[Option<CastInfo>]) -> usize {
    let max_id = widen_map.len() - 1;
    let mut changes = 0;

    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            if let Instruction::Cmp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            } = inst
            {
                if !(*ty == IrType::I64 || *ty == IrType::U64) {
                    continue;
                }

                let lhs_cast = match lhs {
                    Operand::Value(v) => {
                        let id = v.0 as usize;
                        if id <= max_id {
                            widen_map[id].as_ref()
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                let rhs_cast = match rhs {
                    Operand::Value(v) => {
                        let id = v.0 as usize;
                        if id <= max_id {
                            widen_map[id].as_ref()
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                let narrow_ty = if let Some(lhs_info) = lhs_cast {
                    lhs_info.from_ty
                } else {
                    continue;
                };

                let is_signed_cmp = matches!(
                    op,
                    IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge
                );
                let is_unsigned_cmp = matches!(
                    op,
                    IrCmpOp::Ult | IrCmpOp::Ule | IrCmpOp::Ugt | IrCmpOp::Uge
                );

                if is_signed_cmp && narrow_ty.is_unsigned() {
                    continue;
                }
                if is_unsigned_cmp && narrow_ty.is_signed() {
                    continue;
                }

                let new_lhs = if let Some(info) = lhs_cast {
                    if info.from_ty == narrow_ty {
                        info.src
                    } else {
                        continue;
                    }
                } else {
                    continue;
                };

                let new_rhs = if let Some(info) = rhs_cast {
                    if info.from_ty == narrow_ty {
                        info.src
                    } else {
                        continue;
                    }
                } else if let Operand::Const(c) = rhs {
                    if let Some(narrow_c) = try_narrow_const_for_cmp(c, narrow_ty) {
                        Operand::Const(narrow_c)
                    } else {
                        continue;
                    }
                } else {
                    continue;
                };

                *lhs = new_lhs;
                *rhs = new_rhs;
                *ty = narrow_ty;
                let _ = dest;
                changes += 1;
            }
        }
    }

    changes
}

/// Check whether an operand was widened from a type matching the target_ty
/// (same size and same signedness). Used to validate that right-shift narrowing
/// is safe: AShr requires the LHS was sign-extended (signed type), and LShr
/// requires the LHS was zero-extended (unsigned type).
fn is_widened_from_matching_type(
    op: &Operand,
    target_ty: IrType,
    widen_map: &[Option<CastInfo>],
) -> bool {
    if let Operand::Value(v) = op {
        let id = v.0 as usize;
        if id < widen_map.len() {
            if let Some(info) = &widen_map[id] {
                return info.from_ty == target_ty
                    || (info.from_ty.size() == target_ty.size()
                        && info.from_ty.is_unsigned() == target_ty.is_unsigned());
            }
        }
    }
    false
}

/// Classification of a shift count against a narrow width.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ShiftCount {
    /// Provably in `[0, width)`: the narrow shift is exact.
    InRange,
    /// A constant in `[width, 64)`: the wide shift is a known fill
    /// (zero for Shl/LShr, sign for AShr), so the result can be folded.
    ConstOutOfRange,
    /// Anything else (negative/huge constants, unbounded variables):
    /// narrowing would change the value; leave the wide shift alone.
    Unknown,
}

/// Classify a shift count operand for narrowing to `width_bits`.
///
/// A variable count is `InRange` only when its defining instruction is
/// `And(_, c)` / `And(c, _)` with a non-negative constant `c < width_bits`
/// (the C idiom `x >> (n & 31)`); every other variable count is `Unknown`
/// because the wide shift and the narrow shift disagree for counts in
/// `[width, 64)`.
fn shift_count_class(
    rhs: &Operand,
    width_bits: i64,
    binop_map: &[Option<BinOpDef>],
) -> ShiftCount {
    let const_val = |c: &IrConst| -> Option<i64> {
        match c {
            IrConst::I64(v) => Some(*v),
            IrConst::I32(v) => Some(*v as i64),
            IrConst::I16(v) => Some(*v as i64),
            IrConst::I8(v) => Some(*v as i64),
            IrConst::Zero => Some(0),
            _ => None,
        }
    };
    match rhs {
        Operand::Const(c) => match const_val(c) {
            Some(v) if (0..width_bits).contains(&v) => ShiftCount::InRange,
            Some(v) if (width_bits..64).contains(&v) => ShiftCount::ConstOutOfRange,
            _ => ShiftCount::Unknown,
        },
        Operand::Value(v) => {
            let id = v.0 as usize;
            let Some(Some(def)) = binop_map.get(id) else {
                return ShiftCount::Unknown;
            };
            if def.op != IrBinOp::And {
                return ShiftCount::Unknown;
            }
            let mask = match (&def.lhs, &def.rhs) {
                (_, Operand::Const(c)) | (Operand::Const(c), _) => const_val(c),
                _ => None,
            };
            match mask {
                Some(m) if (0..width_bits).contains(&m) => ShiftCount::InRange,
                _ => ShiftCount::Unknown,
            }
        }
    }
}

/// Try to narrow an operand from I64 to a target type.
/// Returns the narrowed operand if possible, None otherwise.
///
/// Checks (in order):
/// (a) If load_type_map provided: value is a Load of the target type → return as-is
/// (b) The value is a widening cast from the target type → return the original
/// (c) The value was already narrowed to the target type by Phase 4 → return as-is
/// (d) The operand is a constant that fits in the target type → return narrowed const
fn try_narrow_operand(
    op: &Operand,
    target_ty: IrType,
    load_type_map: Option<&[Option<IrType>]>,
    widen_map: &[Option<CastInfo>],
    narrowed_map: &[Option<IrType>],
) -> Option<Operand> {
    match op {
        Operand::Value(v) => {
            let id = v.0 as usize;
            // (a) Check if it's a Load of the target type
            if let Some(ltm) = load_type_map {
                if id < ltm.len() {
                    if let Some(ty) = &ltm[id] {
                        if *ty == target_ty
                            || (ty.size() == target_ty.size()
                                && ty.is_unsigned() == target_ty.is_unsigned())
                        {
                            return Some(Operand::Value(*v));
                        }
                    }
                }
            }
            // (b) Check if it's a widening cast from the target type
            if id < widen_map.len() {
                if let Some(info) = &widen_map[id] {
                    if info.from_ty == target_ty
                        || (info.from_ty.size() == target_ty.size()
                            && info.from_ty.is_unsigned() == target_ty.is_unsigned())
                    {
                        return Some(info.src);
                    }
                }
            }
            // (c) Check if it was already narrowed to the target type
            if id < narrowed_map.len() {
                if let Some(nt) = &narrowed_map[id] {
                    if *nt == target_ty
                        || (nt.size() == target_ty.size()
                            && nt.is_unsigned() == target_ty.is_unsigned())
                    {
                        return Some(Operand::Value(*v));
                    }
                }
            }
            None
        }
        Operand::Const(c) => try_narrow_const(c, target_ty).map(Operand::Const),
    }
}

/// Determine the narrow type of an operand (from loads, widen_map, or narrowed_map).
/// Returns None for constants or if the operand's narrow type can't be determined.
fn operand_narrow_type(
    op: &Operand,
    load_type_map: &[Option<IrType>],
    widen_map: &[Option<CastInfo>],
    narrowed_map: &[Option<IrType>],
) -> Option<IrType> {
    match op {
        Operand::Value(v) => {
            let id = v.0 as usize;
            if id < load_type_map.len() {
                if let Some(ty) = &load_type_map[id] {
                    return Some(*ty);
                }
            }
            if id < widen_map.len() {
                if let Some(info) = &widen_map[id] {
                    return Some(info.from_ty);
                }
            }
            if id < narrowed_map.len() {
                if let Some(ty) = &narrowed_map[id] {
                    return Some(*ty);
                }
            }
            None
        }
        Operand::Const(_) => None,
    }
}

/// Check if an operand is a constant that can be narrowed to target_ty.
fn try_narrow_const_operand(op: &Operand, target_ty: IrType) -> Option<IrConst> {
    match op {
        Operand::Const(c) => try_narrow_const(c, target_ty),
        _ => None,
    }
}

/// Try to narrow a constant to a smaller type.
/// When `require_roundtrip` is false (bit-preserving ops like Add/And/Or/Xor),
/// U32 accepts any value in [i32::MIN, u32::MAX] because only the low 32 bits
/// matter. When `require_roundtrip` is true (comparisons), U32 requires
/// val >= 0 so the value survives zero-extension back to 64 bits.
fn try_narrow_const_core(
    c: &IrConst,
    target_ty: IrType,
    require_roundtrip: bool,
) -> Option<IrConst> {
    let val = match c {
        IrConst::I64(v) => *v,
        IrConst::I32(v) => *v as i64,
        IrConst::I16(v) => *v as i64,
        IrConst::I8(v) => *v as i64,
        IrConst::I128(v) => *v as i64,
        IrConst::Zero => 0,
        _ => return None,
    };

    match target_ty {
        IrType::I32 => {
            if val >= i32::MIN as i64 && val <= i32::MAX as i64 {
                Some(IrConst::I32(val as i32))
            } else {
                None
            }
        }
        IrType::U32 => {
            // For bit-preserving ops, accept [i32::MIN, u32::MAX] (low 32 bits matter).
            // For comparisons, require [0, u32::MAX] (value must round-trip through zext).
            let lo = if require_roundtrip {
                0
            } else {
                i32::MIN as i64
            };
            if val >= lo && val <= u32::MAX as i64 {
                Some(IrConst::from_i64(val, IrType::U32))
            } else {
                None
            }
        }
        IrType::I16 => {
            if val >= i16::MIN as i64 && val <= i16::MAX as i64 {
                Some(IrConst::I16(val as i16))
            } else {
                None
            }
        }
        IrType::U16 => {
            if val >= 0 && val <= u16::MAX as i64 {
                Some(IrConst::from_i64(val, IrType::U16))
            } else {
                None
            }
        }
        IrType::I8 => {
            if val >= i8::MIN as i64 && val <= i8::MAX as i64 {
                Some(IrConst::I8(val as i8))
            } else {
                None
            }
        }
        IrType::U8 => {
            if val >= 0 && val <= u8::MAX as i64 {
                Some(IrConst::from_i64(val, IrType::U8))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Try to narrow a constant for bit-preserving operations (Add/Sub/And/Or/Xor).
/// For U32, allows bit-truncation (accepts negative i64 values).
fn try_narrow_const(c: &IrConst, target_ty: IrType) -> Option<IrConst> {
    try_narrow_const_core(c, target_ty, false)
}

/// Narrow a constant for use in a Cmp instruction.
/// Requires that the constant's value is preserved when extended back to 64 bits.
/// This prevents narrowing `Cmp(Eq, zext(x), 0xFFFFFFFF_FFFFFFF6)` to
/// `Cmp(Eq, x, 0xFFFFFFF6)` which would incorrectly match.
fn try_narrow_const_for_cmp(c: &IrConst, target_ty: IrType) -> Option<IrConst> {
    try_narrow_const_core(c, target_ty, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator, Value};

    fn make_func_with_blocks(blocks: Vec<BasicBlock>) -> IrFunction {
        let mut func = IrFunction::new("test".to_string(), IrType::I32, vec![], false);
        func.blocks = blocks;
        func.next_value_id = 100;
        func
    }

    #[test]
    fn test_narrow_add_i32() {
        // %1 = Cast %0, I32->I64     (widen)
        // %2 = BinOp Add %1, I64(5), I64   (add in I64)
        // %3 = Cast %2, I64->I32     (narrow)
        // => Should become:
        // %3 = BinOp Add %0, I32(5), I32
        let mut func = make_func_with_blocks(vec![BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Cast {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                    from_ty: IrType::I32,
                    to_ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I64(5)),
                    ty: IrType::I64,
                },
                Instruction::Cast {
                    dest: Value(3),
                    src: Operand::Value(Value(2)),
                    from_ty: IrType::I64,
                    to_ty: IrType::I32,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
            source_spans: Vec::new(),
        }]);

        let changes = narrow_function(&mut func);
        assert!(changes > 0, "Should narrow the add");

        // The last instruction should now be a BinOp in I32
        let last = &func.blocks[0].instructions[2];
        match last {
            Instruction::BinOp {
                op: IrBinOp::Add,
                ty: IrType::I32,
                lhs,
                rhs,
                ..
            } => {
                // LHS should be the original %0 (unwrapped from the widening cast)
                assert!(matches!(lhs, Operand::Value(Value(0))));
                // RHS should be I32(5) (narrowed constant)
                assert!(matches!(rhs, Operand::Const(IrConst::I32(5))));
            }
            other => panic!("Expected narrowed BinOp Add I32, got {:?}", other),
        }
    }

    #[test]
    fn test_narrow_cmp_sge() {
        // %1 = Cast %0, I32->I64     (widen)
        // %2 = Cmp Sge %1, I64(256), I64
        // => Should become:
        // %2 = Cmp Sge %0, I32(256), I32
        let mut func = make_func_with_blocks(vec![BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Cast {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                    from_ty: IrType::I32,
                    to_ty: IrType::I64,
                },
                Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Sge,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I64(256)),
                    ty: IrType::I64,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
            source_spans: Vec::new(),
        }]);

        let changes = narrow_function(&mut func);
        assert!(changes > 0, "Should narrow the comparison");

        let cmp = &func.blocks[0].instructions[1];
        match cmp {
            Instruction::Cmp {
                op: IrCmpOp::Sge,
                ty: IrType::I32,
                lhs,
                rhs,
                ..
            } => {
                assert!(matches!(lhs, Operand::Value(Value(0))));
                assert!(matches!(rhs, Operand::Const(IrConst::I32(256))));
            }
            other => panic!("Expected narrowed Cmp Sge I32, got {:?}", other),
        }
    }

    #[test]
    fn test_no_narrow_and_all_ones_mask() {
        // %1 = Cast %0, U32->I64       (widen)
        // %2 = BinOp And %1, I64(0xFFFFFFFF), I64
        // => Should NOT narrow because 0xFFFFFFFF becomes all-ones for U32,
        //    which would eliminate the zero-extension mask.
        let mut func = make_func_with_blocks(vec![BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Cast {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                    from_ty: IrType::U32,
                    to_ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::And,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I64(0xFFFFFFFF)),
                    ty: IrType::I64,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
            source_spans: Vec::new(),
        }]);

        let _changes = narrow_function(&mut func);
        // Phase 5 should NOT narrow this AND because the mask becomes all-ones
        // for U32 -- the AND acts as a zero-extension, not a pure bitwise op.
        let binop = &func.blocks[0].instructions[1];
        if let Instruction::BinOp { ty, .. } = binop {
            assert!(
                *ty == IrType::I64 || *ty == IrType::U64,
                "AND with all-ones mask should NOT be narrowed, got {:?}",
                ty
            );
        } // Phase 4 might have transformed it differently
    }

    #[test]
    fn test_no_narrow_when_multiple_uses() {
        // %1 = Cast %0, I32->I64
        // %2 = BinOp Add %1, I64(5), I64
        // %3 = Cast %2, I64->I32  (one use)
        // return %2               (another use of %2!)
        // => Should NOT narrow because %2 has multiple uses
        let mut func = make_func_with_blocks(vec![BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Cast {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                    from_ty: IrType::I32,
                    to_ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I64(5)),
                    ty: IrType::I64,
                },
                Instruction::Cast {
                    dest: Value(3),
                    src: Operand::Value(Value(2)),
                    from_ty: IrType::I64,
                    to_ty: IrType::I32,
                },
            ],
            // Return %2 (I64 value), so %2 has two uses
            terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
            source_spans: Vec::new(),
        }]);

        let changes = narrow_function(&mut func);
        assert_eq!(changes, 0, "Should not narrow when BinOp has multiple uses");
    }

    #[test]
    fn test_narrow_ashr_signed() {
        // AShr narrowing IS safe when the LHS was sign-extended from a signed
        // type matching the target: sext(x, I32->I64) >> k has the same lower
        // 32 bits as x >> k (I32).
        let mut func = make_func_with_blocks(vec![BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Cast {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                    from_ty: IrType::I32,
                    to_ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::AShr,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I64(3)),
                    ty: IrType::I64,
                },
                Instruction::Cast {
                    dest: Value(3),
                    src: Operand::Value(Value(2)),
                    from_ty: IrType::I64,
                    to_ty: IrType::I32,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
            source_spans: Vec::new(),
        }]);

        let changes = narrow_function(&mut func);
        assert!(changes > 0, "Should narrow AShr with signed type");

        // The last instruction should now be a BinOp AShr in I32
        let last = &func.blocks[0].instructions[2];
        match last {
            Instruction::BinOp {
                op: IrBinOp::AShr,
                ty: IrType::I32,
                lhs,
                rhs,
                ..
            } => {
                assert!(matches!(lhs, Operand::Value(Value(0))));
                assert!(matches!(rhs, Operand::Const(IrConst::I32(3))));
            }
            other => panic!("Expected narrowed BinOp AShr I32, got {:?}", other),
        }
    }

    #[test]
    fn test_no_narrow_ashr_unsigned_target() {
        // AShr narrowing is NOT safe when the target type is unsigned.
        // The widening cast from U32->U64 zero-extends, but AShr on the
        // I64 value would sign-extend from bit 63 -- which differs from
        // AShr on the original U32 (which would sign-extend from bit 31).
        // In practice, the lowering widens I32->I64 (signed), so this
        // tests the guard against mismatched signedness.
        let mut func = make_func_with_blocks(vec![BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Cast {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                    from_ty: IrType::U32,
                    to_ty: IrType::U64,
                },
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::AShr,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I64(3)),
                    ty: IrType::U64,
                },
                Instruction::Cast {
                    dest: Value(3),
                    src: Operand::Value(Value(2)),
                    from_ty: IrType::U64,
                    to_ty: IrType::U32,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
            source_spans: Vec::new(),
        }]);

        let changes = narrow_function(&mut func);
        assert_eq!(changes, 0, "Should not narrow AShr with unsigned target");
    }

    #[test]
    fn test_no_narrow_lshr_signed_target() {
        // LShr narrowing is NOT safe when the target type is signed.
        // The widening cast from I32->I64 sign-extends, so the upper 32 bits
        // may be all-1s. LShr on the I64 value shifts those 1-bits down into
        // the lower 32 bits, which differs from LShr on the original I32.
        let mut func = make_func_with_blocks(vec![BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Cast {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                    from_ty: IrType::I32,
                    to_ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::LShr,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I64(3)),
                    ty: IrType::I64,
                },
                Instruction::Cast {
                    dest: Value(3),
                    src: Operand::Value(Value(2)),
                    from_ty: IrType::I64,
                    to_ty: IrType::I32,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
            source_spans: Vec::new(),
        }]);

        let changes = narrow_function(&mut func);
        assert_eq!(changes, 0, "Should not narrow LShr with signed target");
    }

    #[test]
    fn test_same_width_cast_preserves_high_multiply_bits() {
        // `(u64)(sext(x) * C) >> 32` is the classic signed-division magic
        // sequence.  The I64->U64 cast is not a truncation: the following
        // shift consumes the high half, so the multiply must remain 64-bit.
        let mut func = make_func_with_blocks(vec![BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Cast {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                    from_ty: IrType::I32,
                    to_ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I64(1431655766)),
                    ty: IrType::I64,
                },
                Instruction::Cast {
                    dest: Value(3),
                    src: Operand::Value(Value(2)),
                    from_ty: IrType::I64,
                    to_ty: IrType::U64,
                },
                Instruction::BinOp {
                    dest: Value(4),
                    op: IrBinOp::LShr,
                    lhs: Operand::Value(Value(3)),
                    rhs: Operand::Const(IrConst::I32(32)),
                    ty: IrType::U64,
                },
                Instruction::Cast {
                    dest: Value(5),
                    src: Operand::Value(Value(4)),
                    from_ty: IrType::U64,
                    to_ty: IrType::I32,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(5)))),
            source_spans: Vec::new(),
        }]);

        narrow_function(&mut func);
        assert!(matches!(
            func.blocks[0].instructions[1],
            Instruction::BinOp {
                op: IrBinOp::Mul,
                ty: IrType::I64,
                ..
            }
        ));
    }

    #[test]
    fn test_narrow_lshr_unsigned() {
        // LShr narrowing IS safe when the LHS was zero-extended from an
        // unsigned type matching the target: zext(x, U32->U64) >> k has
        // the same lower 32 bits as x >> k (U32).
        let mut func = make_func_with_blocks(vec![BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Cast {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                    from_ty: IrType::U32,
                    to_ty: IrType::U64,
                },
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::LShr,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I64(3)),
                    ty: IrType::U64,
                },
                Instruction::Cast {
                    dest: Value(3),
                    src: Operand::Value(Value(2)),
                    from_ty: IrType::U64,
                    to_ty: IrType::U32,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
            source_spans: Vec::new(),
        }]);

        let changes = narrow_function(&mut func);
        assert!(changes > 0, "Should narrow LShr with unsigned type");

        let last = &func.blocks[0].instructions[2];
        match last {
            Instruction::BinOp {
                op: IrBinOp::LShr,
                ty: IrType::U32,
                lhs,
                rhs,
                ..
            } => {
                assert!(matches!(lhs, Operand::Value(Value(0))));
                assert!(matches!(
                    rhs,
                    Operand::Const(IrConst::I32(3) | IrConst::I64(3))
                ));
            }
            other => panic!("Expected narrowed BinOp LShr U32, got {:?}", other),
        }
    }
    /// Build `Cast(U32->U64) ; BinOp(op, zext, count, U64) ; Cast(U64->U32)`
    /// for the out-of-range shift-count tests below.
    fn shift_through_zext(op: IrBinOp, count: Operand, extra: Vec<Instruction>) -> IrFunction {
        let mut insts = vec![Instruction::Cast {
            dest: Value(1),
            src: Operand::Value(Value(0)),
            from_ty: IrType::U32,
            to_ty: IrType::U64,
        }];
        insts.extend(extra);
        insts.push(Instruction::BinOp {
            dest: Value(2),
            op,
            lhs: Operand::Value(Value(1)),
            rhs: count,
            ty: IrType::U64,
        });
        insts.push(Instruction::Cast {
            dest: Value(3),
            src: Operand::Value(Value(2)),
            from_ty: IrType::U64,
            to_ty: IrType::U32,
        });
        make_func_with_blocks(vec![BasicBlock {
            label: BlockId(0),
            instructions: insts,
            terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
            source_spans: Vec::new(),
        }])
    }

    #[test]
    fn test_narrow_lshr_count_ge_width_folds_to_zero() {
        // Stress-lab finding (narrow family, seed 3): `(uint32_t)(zext(x) >> 56)`
        // was narrowed to `x >> 56` in U32, which x86 executes as `x >> 24`.
        // The 64-bit shift of a zero-extended 32-bit value by >= 32 is 0.
        let mut func =
            shift_through_zext(IrBinOp::LShr, Operand::Const(IrConst::I64(56)), Vec::new());
        let changes = narrow_function(&mut func);
        assert!(changes > 0, "out-of-range count must be folded, not narrowed");
        match &func.blocks[0].instructions[2] {
            Instruction::Copy { dest: Value(3), src: Operand::Const(c) } => {
                assert_eq!(c.to_i64(), Some(0), "fold value must be zero, got {:?}", c);
            }
            other => panic!("expected Copy of zero, got {:?}", other),
        }
        // The pass must never have produced a U32 shift by 56.
        for inst in &func.blocks[0].instructions {
            if let Instruction::BinOp { op: IrBinOp::LShr, ty: IrType::U32, rhs, .. } = inst {
                panic!("narrowed shift with out-of-range count {:?}", rhs);
            }
        }
    }

    #[test]
    fn test_narrow_shl_count_ge_width_folds_to_zero() {
        // `(uint32_t)(zext(x) << 40)`: low 32 bits are all zero for every x.
        let mut func =
            shift_through_zext(IrBinOp::Shl, Operand::Const(IrConst::I64(40)), Vec::new());
        narrow_function(&mut func);
        assert!(
            matches!(
                &func.blocks[0].instructions[2],
                Instruction::Copy { src: Operand::Const(c), .. } if c.to_i64() == Some(0)
            ),
            "got {:?}",
            func.blocks[0].instructions[2]
        );
    }

    #[test]
    fn test_narrow_ashr_count_ge_width_clamps() {
        // `(int32_t)(sext(x) >> 45)` equals `x >> 31` in I32 (sign fill).
        let mut func = make_func_with_blocks(vec![BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Cast {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                    from_ty: IrType::I32,
                    to_ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::AShr,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I64(45)),
                    ty: IrType::I64,
                },
                Instruction::Cast {
                    dest: Value(3),
                    src: Operand::Value(Value(2)),
                    from_ty: IrType::I64,
                    to_ty: IrType::I32,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
            source_spans: Vec::new(),
        }]);
        narrow_function(&mut func);
        match &func.blocks[0].instructions[2] {
            Instruction::BinOp { op: IrBinOp::AShr, ty: IrType::I32, rhs, .. } => {
                assert_eq!(
                    match rhs {
                        Operand::Const(c) => c.to_i64(),
                        _ => None,
                    },
                    Some(31),
                    "AShr count must clamp to width-1, got {:?}",
                    rhs
                );
            }
            other => panic!("expected clamped I32 AShr, got {:?}", other),
        }
    }

    #[test]
    fn test_narrow_shift_variable_count_declined_unless_masked() {
        // Unbounded variable count: the wide and narrow shifts disagree for
        // counts in [32, 64), so narrowing must be declined.
        let mut func = shift_through_zext(
            IrBinOp::LShr,
            Operand::Value(Value(4)),
            vec![Instruction::Cast {
                dest: Value(4),
                src: Operand::Value(Value(5)),
                from_ty: IrType::U32,
                to_ty: IrType::U64,
            }],
        );
        narrow_function(&mut func);
        assert!(
            matches!(
                &func.blocks[0].instructions[3],
                Instruction::Cast { from_ty: IrType::U64, to_ty: IrType::U32, .. }
            ),
            "unbounded variable count must not be narrowed: {:?}",
            func.blocks[0].instructions[3]
        );

        // Masked count (`n & 31`): the classifier proves the count in range.
        // Whether the shift is then narrowed depends on the count operand
        // itself being narrowable (Phase 4 only narrows values that feed a
        // truncating cast), so the end-to-end check is the soundness one: no
        // U32 shift may ever carry a count operand that is not itself U32.
        let binop_map: Vec<Option<BinOpDef>> = vec![
            None,
            None,
            None,
            None,
            Some(BinOpDef {
                op: IrBinOp::And,
                lhs: Operand::Value(Value(5)),
                rhs: Operand::Const(IrConst::I64(31)),
            }),
        ];
        assert_eq!(
            shift_count_class(&Operand::Value(Value(4)), 32, &binop_map),
            ShiftCount::InRange
        );
        let wide_mask: Vec<Option<BinOpDef>> = vec![
            None,
            None,
            None,
            None,
            Some(BinOpDef {
                op: IrBinOp::And,
                lhs: Operand::Value(Value(5)),
                rhs: Operand::Const(IrConst::I64(63)),
            }),
        ];
        assert_eq!(
            shift_count_class(&Operand::Value(Value(4)), 32, &wide_mask),
            ShiftCount::Unknown
        );
        assert_eq!(shift_count_class(&Operand::Const(IrConst::I64(31)), 32, &[]), ShiftCount::InRange);
        assert_eq!(
            shift_count_class(&Operand::Const(IrConst::I64(32)), 32, &[]),
            ShiftCount::ConstOutOfRange
        );
        assert_eq!(shift_count_class(&Operand::Const(IrConst::I64(64)), 32, &[]), ShiftCount::Unknown);
        assert_eq!(shift_count_class(&Operand::Const(IrConst::I64(-1)), 32, &[]), ShiftCount::Unknown);

        let mut func = shift_through_zext(
            IrBinOp::LShr,
            Operand::Value(Value(4)),
            vec![Instruction::BinOp {
                dest: Value(4),
                op: IrBinOp::And,
                lhs: Operand::Value(Value(5)),
                rhs: Operand::Const(IrConst::I64(31)),
                ty: IrType::U64,
            }],
        );
        narrow_function(&mut func);
        for inst in &func.blocks[0].instructions {
            if let Instruction::BinOp { op: IrBinOp::LShr, ty: IrType::U32, rhs, .. } = inst {
                assert!(
                    matches!(rhs, Operand::Const(_)),
                    "a narrowed shift must not carry a wide count operand: {:?}",
                    rhs
                );
            }
        }
    }
}
