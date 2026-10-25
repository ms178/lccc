//! Fixed SSE2 dword permutations: compose private chains and factor identical
//! private permutations at integer/bitwise consumers. Never borrow removal
//! credits from a shared producer or reinterpret arbitrary byte/AVX shuffles.
use super::*;
use crate::ir::reexports::IrConst;

fn immediate(value: Operand) -> Option<u8> {
    let Operand::Const(c) = value else {
        return None;
    };
    u8::try_from(c.to_i128()?).ok()
}
fn compose(inner: u8, outer: u8) -> u8 {
    let mut result = 0;
    for lane in 0..4 {
        let selected = (outer >> (2 * lane)) & 3;
        result |= ((inner >> (2 * selected)) & 3) << (2 * lane);
    }
    result
}
fn shuffle(dest: Value, src: Operand, mask: u8) -> Vec<Instruction> {
    if mask == 0xe4 {
        copy(dest, src)
    } else {
        vec![intrinsic(
            dest,
            IntrinsicOp::VecShufdI32x4,
            vec![src, Operand::Const(IrConst::I32(mask.into()))],
        )]
    }
}
pub(super) fn fold(
    inst: &Instruction,
    defs: &FxHashMap<Value, Instruction>,
    uses: &FxHashMap<Value, u32>,
    families: &[Family],
    fresh: Value,
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
    let [a, b] = root.as_slice() else { return None };
    if *op == IntrinsicOp::VecShufdI32x4 {
        if !matches!(a, Operand::Value(_)) {
            return None;
        }
        let mask = immediate(*b)?;
        if mask == 0xe4 {
            return Some(copy(*dest, *a));
        }
        if !private(*a, uses, defs) {
            return None;
        }
        let inner = args(*a, *op, 2, defs)?;
        if !matches!(inner[0], Operand::Value(_)) {
            return None;
        }
        return Some(shuffle(
            *dest,
            inner[0],
            compose(immediate(inner[1])?, mask),
        ));
    }
    let wordwise = families.iter().any(|f| {
        f.bytes == 16
            && ([f.and, f.or, f.xor].contains(op) || (f.bits <= 32 && [f.add, f.sub].contains(op)))
    }) || matches!(op, IntrinsicOp::VecXorF32x4 | IntrinsicOp::VecXorF64x2);
    if !wordwise || !private(*a, uses, defs) || !private(*b, uses, defs) {
        return None;
    }
    let left = args(*a, IntrinsicOp::VecShufdI32x4, 2, defs)?;
    let right = args(*b, IntrinsicOp::VecShufdI32x4, 2, defs)?;
    if !matches!((left[0], right[0]), (Operand::Value(_), Operand::Value(_))) {
        return None;
    }
    let mask = immediate(left[1])?;
    if immediate(right[1])? != mask || fresh.0.checked_add(1).is_none() {
        return None;
    }
    // The two private shuffles really disappear. Keep the original binop's
    // operand order (SUB is not commutative), family, and wrap policy.
    let mut result = vec![intrinsic(fresh, *op, vec![left[0], right[0]])];
    result.extend(shuffle(*dest, Operand::Value(fresh), mask));
    Some(result)
}

#[cfg(test)]
#[path = "vec_shuffle_tests.rs"]
mod tests;
