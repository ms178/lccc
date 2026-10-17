//! x86-64 shift-count demanded-bits simplification.
//!
//! Every x86 shift reads only the low bits of its count. The 64-bit forms
//! (`shl/shr/sar %cl, r64`, `shlx/shrx/sarx`) use bits 5:0, and the 8-, 16-
//! and 32-bit forms use bits 4:0 (Intel SDM Vol. 2B, SAL/SAR/SHL/SHR "The
//! count is masked to 5 bits (or 6 bits with REX.W)"; SARX/SHLX/SHRX "the
//! count operand is masked to 5/6 bits"). Which form an IR shift gets is the
//! selector's choice, so the demand below is the union over every lowering:
//! `I32`/`U32` shifts are always 32-bit forms (5 bits), while `I8`/`I16`
//! shifts are 8/16-bit forms on the MachInst path but 64-bit forms on the
//! accumulator/register-direct path (`use_32bit` is true only for 32-bit
//! types), so they demand 6 bits, like `I64`/`U64`. Portable bit readers
//! spell the same masking out in C, e.g. zstd's `BIT_lookBits`:
//!
//! ```c
//! (bitContainer << (bitsConsumed & 63)) >> 1 >> ((63 - nbBits) & 63)
//! ```
//!
//! The backend lowered those counts literally (`andl $63`; `movl $63, r;
//! subl n, r; andl $63, r`). This pass removes the redundancy on the IR
//! the x86-64 instruction selector consumes, as a target-specific
//! *demanded-bits* rewrite:
//!
//! * `D(v)` is the number of low bits of `v` any user can observe. A shift
//!   count use demands 5 bits (32-bit shift) or 6 bits (every other width).
//!   Integer casts and the two rewritable forms below pass their result's
//!   demand through to their operand: the low `k <= 6` bits of a zero/sign
//!   extension or truncation between 8..64-bit integers are exactly the low
//!   `k` bits of its source. Every other use (phis, stores, calls, compares,
//!   the shifted operand, terminators, ...) demands all bits.
//! * `x & m` with `m` covering the demanded low bits is `x` on those bits:
//!   its users are rewired to `x` and the dead `and` is deleted.
//! * `C - x` is `-x` when `C ≡ 0` and `~x` when `C ≡ -1` modulo `2^D`. The
//!   unary form needs no immediate register and can update `x` in place.
//!
//! Soundness rests on the backend contract, not on C undefined behaviour:
//! the selector emits each IR shift as one hardware shift of the stated width
//! whose count is masked by the hardware (immediate counts are masked the
//! same way in `isel::lower_binop`). Rewritten values are only ever consumed
//! by such counts, so the executed shift receives the identical masked count.
//! The pass runs after the last semantic IR optimization: only live-range
//! splitting, copy propagation, DCE, phi elimination, recurrence-web merging,
//! loop inversion and block layout follow, none of which interprets a shift
//! count. Rotates are not touched: their counts take the full-demand path. It is x86-64 only; i686 lowers 64-bit shifts to
//! multi-instruction sequences with their own count handling.

use crate::common::fx_hash::FxHashMap;
use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrBinOp, IrFunction, IrUnaryOp, Operand, Value};

/// "Every bit is demanded" (also the cap for any non-shift-count chain).
const FULL: u8 = u8::MAX;

/// Width of an integer type that the x86-64 selector shifts with a single
/// hardware instruction; `None` for i128, floats, pointers and void.
fn scalar_int_bits(ty: IrType) -> Option<u32> {
    match ty {
        IrType::I8 | IrType::U8 => Some(8),
        IrType::I16 | IrType::U16 => Some(16),
        IrType::I32 | IrType::U32 => Some(32),
        IrType::I64 | IrType::U64 => Some(64),
        _ => None,
    }
}

/// Count bits the hardware shift selected for an IR shift of `ty` can read
/// (the maximum over the selector's lowerings; see the module docs).
fn shift_count_bits(ty: IrType) -> Option<u8> {
    match scalar_int_bits(ty)? {
        32 => Some(5),
        _ => Some(6),
    }
}

fn low_mask(bits: u8) -> u64 {
    debug_assert!(bits <= 6);
    (1u64 << bits) - 1
}

fn const_bits(op: &Operand) -> Option<u64> {
    match op {
        Operand::Const(c) => c.to_i64().map(|v| v as u64),
        Operand::Value(_) => None,
    }
}

/// Demand that `inst` places on its operand `v`, given the demand of its own
/// result (`dest_demand`). Unlisted uses see the full value.
fn operand_demand(inst: &Instruction, v: u32, dest_demand: impl Fn(Value) -> u8) -> u8 {
    let is_v = |op: &Operand| matches!(op, Operand::Value(x) if x.0 == v);
    match inst {
        Instruction::BinOp {
            op: IrBinOp::Shl | IrBinOp::LShr | IrBinOp::AShr,
            lhs,
            rhs,
            ty,
            ..
        } if is_v(rhs) && !is_v(lhs) => shift_count_bits(*ty).unwrap_or(FULL),
        Instruction::BinOp {
            dest,
            op: IrBinOp::And,
            lhs,
            rhs,
            ty,
        } if scalar_int_bits(*ty).is_some()
            && ((is_v(lhs) && const_bits(rhs).is_some())
                || (is_v(rhs) && const_bits(lhs).is_some())) =>
        {
            dest_demand(*dest)
        }
        Instruction::BinOp {
            dest,
            op: IrBinOp::Sub,
            lhs,
            rhs,
            ty,
        } if scalar_int_bits(*ty).is_some() && is_v(rhs) && const_bits(lhs).is_some() => {
            dest_demand(*dest)
        }
        Instruction::Cast {
            dest,
            src,
            from_ty,
            to_ty,
        } if is_v(src)
            && scalar_int_bits(*from_ty).is_some()
            && scalar_int_bits(*to_ty).is_some() =>
        {
            // Both widths are >= 8 > 6, so the low demanded bits of the
            // result are the low bits of the source for zext, sext and trunc.
            dest_demand(*dest)
        }
        _ => FULL,
    }
}

/// Simplify masked shift counts in `func`. Returns the number of rewrites.
pub(crate) fn run(func: &mut IrFunction) -> usize {
    if func.is_declaration || func.blocks.is_empty() {
        return 0;
    }
    // Nothing to do without a variable-count scalar shift (the common case).
    let has_variable_shift = func
        .blocks
        .iter()
        .flat_map(|b| b.instructions.iter())
        .any(|i| {
            matches!(
                i,
                Instruction::BinOp {
                    op: IrBinOp::Shl | IrBinOp::LShr | IrBinOp::AShr,
                    rhs: Operand::Value(_),
                    ty,
                    ..
                } if scalar_int_bits(*ty).is_some()
            )
        });
    if !has_variable_shift {
        return 0;
    }
    let n = func
        .blocks
        .iter()
        .flat_map(|b| b.instructions.iter())
        .filter_map(Instruction::dest)
        .map(|d| d.0 as usize + 1)
        .max()
        .unwrap_or(0)
        .max(func.next_value_id as usize);
    // Result types of instruction-defined values; values without a defining
    // instruction (none in well-formed SSA) are never rewired to.
    let mut def_ty: Vec<Option<IrType>> = vec![None; n];
    for inst in func.blocks.iter().flat_map(|b| b.instructions.iter()) {
        if let (Some(d), Some(ty)) = (inst.dest(), inst.result_type()) {
            def_ty[d.0 as usize] = Some(ty);
        }
    }

    // Least fixpoint of D over the use graph. Demand only flows backwards
    // through and/sub/cast chains, which cannot form a cycle without a phi
    // (a full-demand use), and the walk visits users before definitions
    // (reverse order), so a straight-line chain settles in one sweep plus the
    // confirming one. The bound is a defensive cap that falls back to "no
    // rewrite"; `demand` only grows over a finite lattice, so it is reached
    // only for pathological cross-block chain orders.
    let mut demand = vec![0u8; n];
    let mut stable = false;
    let mut updates: Vec<(usize, u8)> = Vec::new();
    for _ in 0..64 {
        let mut changed = false;
        for block in func.blocks.iter().rev() {
            block.terminator.for_each_used_value(|v| {
                if let Some(slot) = demand.get_mut(v as usize) {
                    if *slot != FULL {
                        *slot = FULL;
                        changed = true;
                    }
                }
            });
            for inst in block.instructions.iter().rev() {
                updates.clear();
                inst.for_each_used_value(|v| {
                    let vi = v as usize;
                    let Some(&cur) = demand.get(vi) else { return };
                    if cur == FULL {
                        return;
                    }
                    let d = operand_demand(inst, v, |dest| {
                        demand.get(dest.0 as usize).copied().unwrap_or(FULL)
                    });
                    if d > cur {
                        updates.push((vi, d));
                    }
                });
                for &(vi, d) in &updates {
                    if d > demand[vi] {
                        demand[vi] = d;
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            stable = true;
            break;
        }
    }
    if !stable {
        return 0;
    }
    let demanded = |v: Value| -> Option<u8> {
        let d = *demand.get(v.0 as usize)?;
        (1..=6).contains(&d).then_some(d)
    };

    // Rewrites. `forward` maps an identity `and` result to its operand.
    let mut forward: FxHashMap<u32, Operand> = FxHashMap::default();
    let mut rewrites = 0usize;
    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            match inst {
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::And,
                    lhs,
                    rhs,
                    ty,
                } => {
                    let Some(k) = demanded(*dest) else { continue };
                    let (x, m) = match (lhs.clone(), const_bits(rhs), const_bits(lhs)) {
                        (Operand::Value(x), Some(m), _) => (x, m),
                        (_, None, Some(m)) => match rhs {
                            Operand::Value(x) => (*x, m),
                            Operand::Const(_) => continue,
                        },
                        _ => continue,
                    };
                    // Same-typed operand keeps every rewired user well typed.
                    if def_ty.get(x.0 as usize).copied().flatten() != Some(*ty) {
                        continue;
                    }
                    if m & low_mask(k) == low_mask(k) {
                        forward.insert(dest.0, Operand::Value(x));
                    }
                }
                Instruction::BinOp {
                    dest,
                    op: IrBinOp::Sub,
                    lhs,
                    rhs: Operand::Value(x),
                    ty,
                } => {
                    let Some(k) = demanded(*dest) else { continue };
                    let Some(c) = const_bits(lhs) else { continue };
                    if def_ty.get(x.0 as usize).copied().flatten() != Some(*ty) {
                        continue;
                    }
                    let unary = match c & low_mask(k) {
                        0 => IrUnaryOp::Neg,
                        r if r == low_mask(k) => IrUnaryOp::Not,
                        _ => continue,
                    };
                    *inst = Instruction::UnaryOp {
                        dest: *dest,
                        op: unary,
                        src: Operand::Value(*x),
                        ty: *ty,
                    };
                    rewrites += 1;
                }
                _ => {}
            }
        }
    }
    if forward.is_empty() {
        return rewrites;
    }
    // Resolve chains (`(x & 63) & 31` feeding a 5-bit count) to the root.
    let resolve = |mut op: Operand| {
        for _ in 0..forward.len() + 1 {
            match op {
                Operand::Value(v) => match forward.get(&v.0) {
                    Some(next) => op = next.clone(),
                    None => break,
                },
                Operand::Const(_) => break,
            }
        }
        op
    };
    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            if matches!(inst, Instruction::BinOp { dest, op: IrBinOp::And, .. } if forward.contains_key(&dest.0))
            {
                continue;
            }
            inst.for_each_operand_mut(|op| {
                if let Operand::Value(v) = op {
                    if forward.contains_key(&v.0) {
                        *op = resolve(op.clone());
                    }
                }
            });
        }
        // Terminators demand every bit, so they never name a forwarded value.
        debug_assert!(
            block
                .terminator
                .used_values()
                .iter()
                .all(|v| !forward.contains_key(v))
        );
    }
    // Every user of a forwarded `and` was rewired above; delete the dead
    // instructions here rather than relying on generic DCE (which declines
    // functions with an observable FP environment).
    for block in &mut func.blocks {
        let keep: Vec<bool> = block
            .instructions
            .iter()
            .map(|inst| {
                !matches!(inst, Instruction::BinOp { dest, op: IrBinOp::And, .. }
                    if forward.contains_key(&dest.0))
            })
            .collect();
        if keep.iter().all(|&k| k) {
            continue;
        }
        let spans_parallel = block.source_spans.len() == block.instructions.len();
        let mut idx = 0;
        block.instructions.retain(|_| {
            idx += 1;
            keep[idx - 1]
        });
        if spans_parallel {
            let mut idx = 0;
            block.source_spans.retain(|_| {
                idx += 1;
                keep[idx - 1]
            });
        }
    }
    rewrites + forward.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::source::Span;
    use crate::ir::reexports::{BasicBlock, BlockId, IrConst, Terminator};

    fn v(n: u32) -> Operand {
        Operand::Value(Value(n))
    }
    fn c(n: i64) -> Operand {
        Operand::Const(IrConst::I64(n))
    }
    fn bin(dest: u32, op: IrBinOp, lhs: Operand, rhs: Operand, ty: IrType) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op,
            lhs,
            rhs,
            ty,
        }
    }
    fn param(dest: u32, idx: usize, ty: IrType) -> Instruction {
        Instruction::ParamRef {
            dest: Value(dest),
            param_idx: idx,
            ty,
        }
    }
    fn func(insts: Vec<Instruction>, ret: u32) -> IrFunction {
        let mut f = IrFunction::new("f".into(), IrType::U64, Vec::new(), false);
        f.next_value_id = insts
            .iter()
            .filter_map(Instruction::dest)
            .map(|d| d.0 + 1)
            .max()
            .unwrap_or(0);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: insts,
            terminator: Terminator::Return(Some(v(ret))),
            source_spans: Vec::new(),
        });
        f
    }
    fn insts(f: &IrFunction) -> &[Instruction] {
        &f.blocks[0].instructions
    }
    fn has_and(f: &IrFunction) -> bool {
        insts(f).iter().any(|i| {
            matches!(
                i,
                Instruction::BinOp {
                    op: IrBinOp::And,
                    ..
                }
            )
        })
    }
    fn shift_count(f: &IrFunction, dest: u32) -> Operand {
        insts(f)
            .iter()
            .find_map(|i| match i {
                Instruction::BinOp { dest: d, rhs, .. } if d.0 == dest => Some(rhs.clone()),
                _ => None,
            })
            .expect("shift present")
    }

    /// Execute the straight-line test IR with x86 hardware shift semantics:
    /// the oracle the rewrite must preserve.
    /// `narrow_bits` is the count width the selector's chosen form reads for
    /// 8/16-bit shifts: 5 (MachInst `shlb/shlw`) or 6 (64-bit register form).
    fn eval(f: &IrFunction, args: &[u64], narrow_bits: u32) -> u64 {
        let mut vals: FxHashMap<u32, u64> = FxHashMap::default();
        let get = |vals: &FxHashMap<u32, u64>, op: &Operand| match op {
            Operand::Value(x) => vals[&x.0],
            Operand::Const(k) => k.to_i64().unwrap() as u64,
        };
        let trunc = |ty: IrType, x: u64| match scalar_int_bits(ty).unwrap() {
            64 => x,
            b => x & ((1u64 << b) - 1),
        };
        let sext = |ty: IrType, x: u64| match scalar_int_bits(ty).unwrap() {
            64 => x,
            b => (((x << (64 - b)) as i64) >> (64 - b)) as u64,
        };
        for inst in insts(f) {
            let (d, r) = match inst {
                Instruction::ParamRef {
                    dest,
                    param_idx,
                    ty,
                } => (*dest, trunc(*ty, args[*param_idx])),
                Instruction::BinOp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => {
                    let (a, b) = (get(&vals, lhs), get(&vals, rhs));
                    let bits = scalar_int_bits(*ty).unwrap();
                    let hw = match bits {
                        64 => 63,
                        32 => 31,
                        _ => (1u64 << narrow_bits) - 1,
                    };
                    let r = match op {
                        IrBinOp::And => a & b,
                        IrBinOp::Sub => a.wrapping_sub(b),
                        IrBinOp::Shl => trunc(*ty, a).wrapping_shl((b & hw) as u32),
                        IrBinOp::LShr => trunc(*ty, a).wrapping_shr((b & hw) as u32),
                        IrBinOp::AShr => (sext(*ty, a) as i64).wrapping_shr((b & hw) as u32) as u64,
                        _ => unreachable!(),
                    };
                    (*dest, trunc(*ty, r))
                }
                Instruction::UnaryOp { dest, op, src, ty } => {
                    let a = get(&vals, src);
                    let r = match op {
                        IrUnaryOp::Neg => a.wrapping_neg(),
                        IrUnaryOp::Not => !a,
                        _ => unreachable!(),
                    };
                    (*dest, trunc(*ty, r))
                }
                Instruction::Cast {
                    dest,
                    src,
                    from_ty,
                    to_ty,
                } => {
                    let a = get(&vals, src);
                    let ext = if from_ty.is_signed() {
                        sext(*from_ty, a)
                    } else {
                        trunc(*from_ty, a)
                    };
                    (*dest, trunc(*to_ty, ext))
                }
                other => unreachable!("{other:?}"),
            };
            vals.insert(d.0, r);
        }
        match &f.blocks[0].terminator {
            Terminator::Return(Some(op)) => get(&vals, op),
            _ => unreachable!(),
        }
    }

    /// Run the pass and prove equivalence with the original on adversarial
    /// inputs: counts across every masked boundary and wide garbage bits.
    fn rewrite_and_check(f: &IrFunction) -> IrFunction {
        let mut g = f.clone();
        run(&mut g);
        let xs = [
            0u64,
            1,
            0x8000_0000_0000_0001,
            0xdead_beef_cafe_f00d,
            u64::MAX,
        ];
        let ns = [
            0u64,
            1,
            5,
            31,
            32,
            33,
            63,
            64,
            65,
            127,
            128,
            255,
            256,
            0xffff_ffff,
            0x1_0000_0040,
            u64::MAX,
        ];
        for &x in &xs {
            for &n in &ns {
                for &m in &[0u64, 7, 57, 64, 200, u64::MAX] {
                    let args = [x, n, m];
                    for narrow_bits in [5, 6] {
                        assert_eq!(
                            eval(f, &args, narrow_bits),
                            eval(&g, &args, narrow_bits),
                            "x={x:#x} n={n:#x} m={m:#x} narrow_bits={narrow_bits}"
                        );
                    }
                }
            }
        }
        g
    }

    #[test]
    fn zstd_look_bits_drops_both_masks_and_turns_the_subtract_into_not() {
        // (c << (n & 63)) >> 1 >> ((63 - m) & 63)
        let f = func(
            vec![
                param(0, 0, IrType::U64),
                param(1, 1, IrType::U32),
                param(2, 2, IrType::U32),
                bin(3, IrBinOp::And, v(1), c(63), IrType::U32),
                bin(4, IrBinOp::Shl, v(0), v(3), IrType::U64),
                bin(5, IrBinOp::LShr, v(4), c(1), IrType::U64),
                bin(6, IrBinOp::Sub, c(63), v(2), IrType::U32),
                bin(7, IrBinOp::And, v(6), c(63), IrType::U32),
                bin(8, IrBinOp::LShr, v(5), v(7), IrType::U64),
            ],
            8,
        );
        let g = rewrite_and_check(&f);
        assert!(!has_and(&g));
        assert_eq!(shift_count(&g, 4), v(1));
        assert_eq!(shift_count(&g, 8), v(6));
        assert!(insts(&g).iter().any(|i| matches!(
            i,
            Instruction::UnaryOp { dest, op: IrUnaryOp::Not, .. } if dest.0 == 6
        )));
    }

    #[test]
    fn width_minus_count_becomes_neg_through_a_widening_cast() {
        // c >> ((64 - n) & 63) with the count widened to U64 first.
        let f = func(
            vec![
                param(0, 0, IrType::U64),
                param(1, 1, IrType::U32),
                bin(2, IrBinOp::Sub, c(64), v(1), IrType::U32),
                bin(3, IrBinOp::And, v(2), c(63), IrType::U32),
                Instruction::Cast {
                    dest: Value(4),
                    src: v(3),
                    from_ty: IrType::U32,
                    to_ty: IrType::U64,
                },
                bin(5, IrBinOp::LShr, v(0), v(4), IrType::U64),
            ],
            5,
        );
        let g = rewrite_and_check(&f);
        assert!(!has_and(&g));
        assert!(insts(&g).iter().any(|i| matches!(
            i,
            Instruction::UnaryOp { dest, op: IrUnaryOp::Neg, .. } if dest.0 == 2
        )));
    }

    #[test]
    fn only_32_bit_shifts_read_five_bits() {
        for (ty, mask, removable) in [
            (IrType::U32, 31, true),
            (IrType::I32, 255, true),
            // 8/16-bit shifts may execute as 64-bit forms: bit 5 is read.
            (IrType::U16, 31, false),
            (IrType::I8, 31, false),
            (IrType::U16, 63, true),
            (IrType::I8, 255, true),
            (IrType::U32, 15, false),
            (IrType::U64, 31, false),
            (IrType::U64, 62, false),
            (IrType::U64, 127, true),
        ] {
            let f = func(
                vec![
                    param(0, 0, ty),
                    param(1, 1, IrType::U32),
                    bin(2, IrBinOp::And, v(1), c(mask), IrType::U32),
                    bin(3, IrBinOp::AShr, v(0), v(2), ty),
                ],
                3,
            );
            let g = rewrite_and_check(&f);
            assert_eq!(!has_and(&g), removable, "{ty:?} & {mask}");
        }
    }

    #[test]
    fn any_full_width_use_keeps_the_mask() {
        // The masked value is also returned/added: all bits observable.
        let f = func(
            vec![
                param(0, 0, IrType::U64),
                param(1, 1, IrType::U64),
                bin(2, IrBinOp::And, v(1), c(63), IrType::U64),
                bin(3, IrBinOp::Shl, v(0), v(2), IrType::U64),
                bin(4, IrBinOp::Sub, v(3), v(2), IrType::U64),
            ],
            4,
        );
        let g = rewrite_and_check(&f);
        assert!(has_and(&g));
        // Shifted operand (not the count) is a full use as well.
        let f = func(
            vec![
                param(0, 0, IrType::U64),
                param(1, 1, IrType::U64),
                bin(2, IrBinOp::And, v(1), c(63), IrType::U64),
                bin(3, IrBinOp::Shl, v(2), v(0), IrType::U64),
            ],
            3,
        );
        assert!(has_and(&rewrite_and_check(&f)));
        // Mixed widths: a 64-bit consumer demands bit 5 that `& 31` clears.
        let f = func(
            vec![
                param(0, 0, IrType::U64),
                param(1, 1, IrType::U32),
                bin(2, IrBinOp::And, v(1), c(31), IrType::U32),
                bin(3, IrBinOp::Shl, v(0), v(2), IrType::U32),
                bin(4, IrBinOp::Shl, v(0), v(2), IrType::U64),
                bin(5, IrBinOp::Sub, v(3), v(4), IrType::U64),
            ],
            5,
        );
        assert!(has_and(&rewrite_and_check(&f)));
    }

    #[test]
    fn non_congruent_subtract_and_nested_masks() {
        // 60 - n is neither -n nor ~n modulo 64: it must stay a subtract.
        let f = func(
            vec![
                param(0, 0, IrType::U64),
                param(1, 1, IrType::U32),
                bin(2, IrBinOp::Sub, c(60), v(1), IrType::U32),
                bin(3, IrBinOp::And, v(2), c(63), IrType::U32),
                bin(4, IrBinOp::Shl, v(0), v(3), IrType::U64),
            ],
            4,
        );
        let g = rewrite_and_check(&f);
        assert!(insts(&g).iter().any(|i| matches!(
            i,
            Instruction::BinOp {
                op: IrBinOp::Sub,
                ..
            }
        )));
        // Nested masks resolve to the root operand.
        let f = func(
            vec![
                param(0, 0, IrType::U32),
                param(1, 1, IrType::U32),
                bin(2, IrBinOp::And, v(1), c(63), IrType::U32),
                bin(3, IrBinOp::And, c(31), v(2), IrType::U32),
                bin(4, IrBinOp::LShr, v(0), v(3), IrType::U32),
            ],
            4,
        );
        let g = rewrite_and_check(&f);
        assert!(!has_and(&g));
        assert_eq!(shift_count(&g, 4), v(1));
    }

    /// Two blocks: the mask in block 0, its users in block 1.
    fn two_block(with_phi: bool) -> IrFunction {
        let mut f = func(
            vec![
                param(0, 0, IrType::U64),
                param(1, 1, IrType::U32),
                bin(2, IrBinOp::And, v(1), c(63), IrType::U32),
            ],
            2,
        );
        f.blocks[0].terminator = Terminator::Branch(BlockId(1));
        let mut tail = Vec::new();
        if with_phi {
            tail.push(Instruction::Phi {
                dest: Value(3),
                ty: IrType::U32,
                incoming: vec![(v(2), BlockId(0))],
            });
        }
        tail.push(bin(4, IrBinOp::Shl, v(0), v(2), IrType::U64));
        f.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: tail,
            terminator: Terminator::Return(Some(v(4))),
            source_spans: Vec::new(),
        });
        f.next_value_id = 5;
        f
    }

    #[test]
    fn cross_block_count_drops_the_mask_but_a_phi_use_keeps_it() {
        let mut f = two_block(false);
        assert_eq!(run(&mut f), 1);
        assert!(!has_and(&f));
        assert_eq!(
            format!("{:?}", f.blocks[1].instructions[0]),
            format!("{:?}", bin(4, IrBinOp::Shl, v(0), v(1), IrType::U64))
        );
        let mut f = two_block(true);
        assert_eq!(run(&mut f), 0, "a phi observes every bit of the mask");
        assert!(has_and(&f));
    }

    #[test]
    fn functions_without_variable_shifts_are_untouched() {
        // `63 - n` feeding a constant-count shift's *value* operand.
        let f = func(
            vec![
                param(0, 0, IrType::U32),
                bin(1, IrBinOp::Sub, c(63), v(0), IrType::U32),
                bin(2, IrBinOp::And, v(1), c(63), IrType::U32),
                bin(3, IrBinOp::Shl, v(2), c(3), IrType::U32),
            ],
            3,
        );
        let mut g = f.clone();
        assert_eq!(run(&mut g), 0);
        assert_eq!(format!("{:?}", insts(&g)), format!("{:?}", insts(&f)));
    }

    #[test]
    fn mask_on_a_narrowing_cast_source_and_spans_stay_parallel() {
        let mut f = func(
            vec![
                param(0, 0, IrType::U64),
                param(1, 1, IrType::U64),
                bin(2, IrBinOp::And, v(1), c(0x3f), IrType::U64),
                Instruction::Cast {
                    dest: Value(3),
                    src: v(2),
                    from_ty: IrType::U64,
                    to_ty: IrType::U8,
                },
                bin(4, IrBinOp::Shl, v(0), v(3), IrType::U64),
            ],
            4,
        );
        f.blocks[0].source_spans = (0..5).map(|i| Span::new(i, i + 1, 0)).collect();
        let g = rewrite_and_check(&f);
        assert!(!has_and(&g));
        // The deleted `and` (index 2) takes its own span with it.
        let starts: Vec<u32> = g.blocks[0].source_spans.iter().map(|s| s.start).collect();
        assert_eq!(starts, [0, 1, 3, 4]);
    }
}
