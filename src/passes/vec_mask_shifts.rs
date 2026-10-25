//! Exact fixed-width packed shift and canonical-mask reductions.
//! No assumptions from C undefined counts, LLVM poison or scalar i1 casts.
use super::*;
use crate::ir::reexports::IrConst;

#[derive(Clone, Copy)]
struct Shifts {
    bits: usize,
    bytes: usize,
    shl: IntrinsicOp,
    lshr: IntrinsicOp,
    ashr: Option<IntrinsicOp>,
}
fn shapes() -> [Shifts; 6] {
    use IntrinsicOp::*;
    [
        Shifts {
            bits: 16,
            bytes: 16,
            shl: VecShlI16x8,
            lshr: VecLShrI16x8,
            ashr: Some(VecAShrI16x8),
        },
        Shifts {
            bits: 16,
            bytes: 32,
            shl: VecShlI16x16,
            lshr: VecLShrI16x16,
            ashr: Some(VecAShrI16x16),
        },
        Shifts {
            bits: 32,
            bytes: 16,
            shl: VecShlI32x4,
            lshr: VecLShrI32x4,
            ashr: Some(VecAShrI32x4),
        },
        Shifts {
            bits: 32,
            bytes: 32,
            shl: VecShlI32x8,
            lshr: VecLShrI32x8,
            ashr: Some(VecAShrI32x8),
        },
        Shifts {
            bits: 64,
            bytes: 16,
            shl: VecShlI64x2,
            lshr: VecLShrI64x2,
            ashr: None,
        },
        Shifts {
            bits: 64,
            bytes: 32,
            shl: VecShlI64x4,
            lshr: VecLShrI64x4,
            ashr: None,
        },
    ]
}
fn amount(value: Operand, bits: usize) -> Option<usize> {
    let Operand::Const(c) = value else {
        return None;
    };
    let n = usize::try_from(c.to_i128()?).ok()?;
    (n > 0 && n < bits).then_some(n)
}
// A bounded proof over an immutable block-local definition snapshot. All
// leaves must prove complete zero/all-one lanes at the requested granularity.
// Bitwise register operations preserve that property independently of their
// declared element family; a finer comparison must NOT prove a coarser mask.
fn canonical(
    value: Operand,
    shape: Shifts,
    defs: &FxHashMap<Value, Instruction>,
    families: &[Family],
    depth: usize,
    budget: &mut usize,
) -> bool {
    if depth == 0 || *budget == 0 {
        return false;
    }
    *budget -= 1;
    let Some(Instruction::Intrinsic {
        op,
        dest_ptr: None,
        args: a,
        ..
    }) = def(value, defs)
    else {
        return false;
    };
    for f in families.iter().filter(|f| f.bytes == shape.bytes) {
        if *op == f.zero && a.is_empty() {
            return true;
        }
        if *op == f.cmp
            && a.len() == 3
            && f.bits >= shape.bits
            && matches!(a[2], Operand::Const(c) if matches!(c.to_i128(), Some(0 | 1 | 2 | 4 | 5 | 6)))
        {
            return true;
        }
        if *op == f.broadcast && a.len() == 1 {
            let full = u64::MAX >> (64 - f.bits);
            if splat(value, *f, defs).is_some_and(|n| n == 0 || n == full) {
                return true;
            }
        }
        if [f.and, f.or, f.xor].contains(op) && a.len() == 2 {
            return canonical(a[0], shape, defs, families, depth - 1, budget)
                && canonical(a[1], shape, defs, families, depth - 1, budget);
        }
    }
    shape.ashr == Some(*op) && a.len() == 2 && amount(a[1], shape.bits) == Some(shape.bits - 1)
}
pub(super) fn fold(
    inst: &Instruction,
    defs: &FxHashMap<Value, Instruction>,
    uses: &FxHashMap<Value, u32>,
    families: &[Family],
) -> Option<Vec<Instruction>> {
    let Instruction::Intrinsic {
        dest: Some(dest),
        op,
        dest_ptr: None,
        args: root,
    } = inst
    else {
        return None;
    };
    let [src, count] = root.as_slice() else {
        return None;
    };
    if !matches!(src, Operand::Value(_)) {
        return None;
    }
    let shape = shapes()
        .into_iter()
        .find(|s| [Some(s.shl), Some(s.lshr), s.ashr].contains(&Some(*op)))?;
    let n = amount(*count, shape.bits)?;
    if shape.ashr == Some(*op) && canonical(*src, shape, defs, families, 16, &mut 256) {
        return Some(copy(*dest, *src));
    }
    // Bypassing a producer is only credited when it really disappears.
    if !private(*src, uses, defs) {
        return None;
    }
    if let Some(inner) = args(*src, *op, 2, defs) {
        let m = amount(inner[1], shape.bits)?;
        if !matches!(inner[0], Operand::Value(_)) {
            return None;
        }
        let sum = n + m;
        if sum >= shape.bits && shape.ashr != Some(*op) {
            let zero = if shape.bytes == 16 {
                IntrinsicOp::VecZeroI32x4
            } else {
                IntrinsicOp::VecZeroI32x8
            };
            return Some(vec![intrinsic(*dest, zero, vec![])]);
        }
        let merged = sum.min(shape.bits - 1);
        return Some(vec![intrinsic(
            *dest,
            *op,
            vec![inner[0], Operand::Const(IrConst::I32(merged as i32))],
        )]);
    }
    if shape.ashr == Some(*op)
        && let Some(inner) = args(*src, shape.shl, 2, defs)
        && amount(inner[1], shape.bits) == Some(n)
        && canonical(inner[0], shape, defs, families, 16, &mut 256)
        && matches!(inner[0], Operand::Value(_))
    {
        return Some(copy(*dest, inner[0]));
    }
    None
}

#[cfg(test)]
#[path = "vec_mask_shifts_tests.rs"]
mod tests;
