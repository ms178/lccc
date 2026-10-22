//! Exact register-image reductions after whole-vector SSA promotion.
//! No floating arithmetic, cast peeling, memory folding or mask-value
//! assumptions. Blend correlation uses identical mask SSA values, including
//! noncanonical masks; factoring is allowed only for private producers.
use super::slp_vectorizer::packed_binop;
use crate::common::fx_hash::FxHashMap;
use crate::common::types::IrType;
use crate::ir::reexports::{
    Instruction, IntrinsicOp, IrBinOp, IrFunction, IrModule, Operand, Value,
};

#[derive(Clone, Copy)]
struct Family {
    and: IntrinsicOp,
    or: IntrinsicOp,
    xor: IntrinsicOp,
    add: IntrinsicOp,
    sub: IntrinsicOp,
    zero: IntrinsicOp,
    blend: Option<IntrinsicOp>,
    cmp: IntrinsicOp,
    broadcast: IntrinsicOp,
    bits: usize,
    bytes: usize,
}
fn families() -> Vec<Family> {
    use IntrinsicOp::*;
    let mut result = Vec::new();
    for (ty, bytes, blend) in [
        (IrType::I8, 16, Some(VecBlendvI8x16)),
        (IrType::I8, 32, Some(VecBlendvI8x32)),
        (IrType::I16, 16, Some(VecBlendvI16x8)),
        (IrType::I16, 32, Some(VecBlendvI16x16)),
        (IrType::I32, 16, Some(VecBlendvI32x4)),
        (IrType::I32, 32, Some(VecBlendvI32x8)),
        (IrType::I64, 16, None),
        (IrType::I64, 32, None),
    ] {
        let (cmp, broadcast) = match (ty, bytes) {
            (IrType::I8, 16) => (VecCmpI8x16, VecBroadcastI8x16),
            (IrType::I8, 32) => (VecCmpI8x32, VecBroadcastI8x32),
            (IrType::I16, 16) => (VecCmpI16x8, VecBroadcastI16x8),
            (IrType::I16, 32) => (VecCmpI16x16, VecBroadcastI16x16),
            (IrType::I32, 16) => (VecCmpI32x4, VecBroadcastI32x4),
            (IrType::I32, 32) => (VecCmpI32x8, VecBroadcastI32x8),
            (IrType::I64, 16) => (VecCmpI64x2, VecBroadcastI64x2),
            (IrType::I64, 32) => (VecCmpI64x4, VecBroadcastI64x4),
            _ => unreachable!(),
        };
        let lanes = bytes / ty.size();
        let ops = [
            IrBinOp::And,
            IrBinOp::Or,
            IrBinOp::Xor,
            IrBinOp::Add,
            IrBinOp::Sub,
        ]
        .map(|op| packed_binop(op, ty, lanes));
        if let [Some(and), Some(or), Some(xor), Some(add), Some(sub)] = ops {
            result.push(Family {
                and,
                or,
                xor,
                add,
                sub,
                zero: if bytes == 16 {
                    VecZeroI32x4
                } else {
                    VecZeroI32x8
                },
                blend,
                cmp,
                broadcast,
                bits: ty.size() * 8,
                bytes,
            });
        }
    }
    result
}
fn intrinsic(dest: Value, op: IntrinsicOp, args: Vec<Operand>) -> Instruction {
    Instruction::Intrinsic {
        dest: Some(dest),
        op,
        dest_ptr: None,
        args,
    }
}
fn copy(dest: Value, src: Operand) -> Vec<Instruction> {
    vec![Instruction::Copy { dest, src }]
}
fn def(value: Operand, defs: &FxHashMap<Value, Instruction>) -> Option<&Instruction> {
    let Operand::Value(mut value) = value else {
        return None;
    };
    for _ in 0..8 {
        match defs.get(&value)? {
            Instruction::Copy {
                src: Operand::Value(next),
                ..
            } => value = *next,
            inst => return Some(inst),
        }
    }
    None
}
fn args(
    value: Operand,
    op: IntrinsicOp,
    len: usize,
    defs: &FxHashMap<Value, Instruction>,
) -> Option<&[Operand]> {
    match def(value, defs)? {
        Instruction::Intrinsic {
            op: other,
            dest_ptr: None,
            args,
            ..
        } if *other == op && args.len() == len => Some(args),
        _ => None,
    }
}
fn private(
    value: Operand,
    uses: &FxHashMap<Value, u32>,
    defs: &FxHashMap<Value, Instruction>,
) -> bool {
    matches!(value,Operand::Value(v) if uses.get(&v)==Some(&1) && matches!(defs.get(&v),Some(Instruction::Intrinsic{..})))
}
fn canonical_mask(
    mask: Operand,
    blend: Option<IntrinsicOp>,
    defs: &FxHashMap<Value, Instruction>,
) -> bool {
    use IntrinsicOp::*;
    let cmp = match blend {
        Some(VecBlendvI8x16) => VecCmpI8x16,
        Some(VecBlendvI8x32) => VecCmpI8x32,
        Some(VecBlendvI16x8) => VecCmpI16x8,
        Some(VecBlendvI16x16) => VecCmpI16x16,
        Some(VecBlendvI32x4) => VecCmpI32x4,
        Some(VecBlendvI32x8) => VecCmpI32x8,
        _ => return false,
    };
    args(mask, cmp, 3, defs).is_some()
}
fn blend_shape(op: IntrinsicOp) -> Option<(usize, usize)> {
    use IntrinsicOp::*;
    Some(match op {
        VecBlendvI8x16 => (16, 8),
        VecBlendvI8x32 => (32, 8),
        VecBlendvI16x8 => (16, 16),
        VecBlendvI16x16 => (32, 16),
        VecBlendvI32x4 | VecBlendvF32x4 => (16, 32),
        VecBlendvI32x8 | VecBlendvF32x8 => (32, 32),
        VecBlendvF64x2 => (16, 64),
        VecBlendvF64x4 => (32, 64),
        _ => return None,
    })
}
fn splat(value: Operand, f: Family, defs: &FxHashMap<Value, Instruction>) -> Option<u64> {
    let [Operand::Const(c)] = args(value, f.broadcast, 1, defs)? else {
        return None;
    };
    Some(c.to_i128()? as u64 & (u64::MAX >> (64 - f.bits)))
}
// A canonical comparison masks every false lane to zero. Its true lane
// proves the nested AND redundant, whether consumed by AND or a blend.
fn bounded_arm(
    mask: Operand,
    value: Operand,
    f: Family,
    defs: &FxHashMap<Value, Instruction>,
) -> Option<Operand> {
    let cmp = args(mask, f.cmp, 3, defs)?;
    let Operand::Const(predicate) = cmp[2] else {
        return None;
    };
    let limit = splat(cmp[1], f, defs)?;
    let low = match predicate.to_i128()? {
        5 if limit.is_power_of_two() => limit - 1,
        6 if limit & limit.wrapping_add(1) == 0 => limit,
        _ => return None,
    };
    let masked = args(value, f.and, 2, defs)?;
    for (base, bits) in [(masked[0], masked[1]), (masked[1], masked[0])] {
        if base == cmp[0] && splat(bits, f, defs) == Some(low) {
            return Some(base);
        }
    }
    None
}
// A byte blend may consume wider canonical compare lanes. A COARSER blend
// must never borrow that property from smaller lanes.
fn bounded_blend(
    root: &[Operand],
    op: IntrinsicOp,
    defs: &FxHashMap<Value, Instruction>,
    families: &[Family],
) -> Option<Vec<Operand>> {
    let (bytes, bits) = blend_shape(op)?;
    for &f in families {
        if f.bytes == bytes
            && bits <= f.bits
            && let Some(base) = bounded_arm(root[2], root[1], f, defs)
        {
            return Some(vec![root[0], base, root[2]]);
        }
    }
    None
}
fn fold(
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
    // Packed blends select register bits, not scalar Boolean values. Keep
    // the opcode (including its lane/sign-bit policy) exactly unchanged.
    let blend = matches!(
        op,
        IntrinsicOp::VecBlendvF32x4
            | IntrinsicOp::VecBlendvF32x8
            | IntrinsicOp::VecBlendvF64x2
            | IntrinsicOp::VecBlendvF64x4
    ) || families.iter().any(|f| f.blend == Some(*op));
    if blend && root.len() == 3 {
        if root[0] == root[1] {
            return Some(copy(*dest, root[0]));
        }
        if let Some(replacement) = bounded_blend(root, *op, defs, families) {
            return Some(vec![intrinsic(*dest, *op, replacement)]);
        }
        for index in [0, 1] {
            if let Some(inner) = args(root[index], *op, 3, defs)
                && inner[2] == root[2]
            {
                let mut replacement = root.clone();
                replacement[index] = inner[index];
                return Some(vec![intrinsic(*dest, *op, replacement)]);
            }
        }
    }
    if root.len() != 2 {
        return None;
    }
    let [a, b] = [root[0], root[1]];
    // Bitwise FP XOR is an integer image operation: retain payload bits and
    // signed zero. Never include FP add/sub/mul/min/max in this table.
    for (xor, zero) in [
        (IntrinsicOp::VecXorF32x4, IntrinsicOp::VecZeroF32x4),
        (IntrinsicOp::VecXorF32x8, IntrinsicOp::VecZeroF32x8),
        (IntrinsicOp::VecXorF64x2, IntrinsicOp::VecZeroF64x2),
        (IntrinsicOp::VecXorF64x4, IntrinsicOp::VecZeroF64x4),
    ] {
        if *op == xor {
            if a == b {
                return Some(vec![intrinsic(*dest, zero, vec![])]);
            }
            for (x, y) in [(a, b), (b, a)] {
                if let Some(inner) = args(x, xor, 2, defs) {
                    for i in [0, 1] {
                        if inner[i] == y {
                            return Some(copy(*dest, inner[1 - i]));
                        }
                    }
                }
            }
        }
    }
    for f in families {
        if ![f.and, f.or, f.xor, f.add, f.sub].contains(op) {
            continue;
        }
        if *op == f.and {
            for (mask, value) in [(a, b), (b, a)] {
                if let Some(base) = bounded_arm(mask, value, *f, defs) {
                    return Some(vec![intrinsic(*dest, *op, vec![mask, base])]);
                }
            }
        }
        if a == b {
            if *op == f.and || *op == f.or {
                return Some(copy(*dest, a));
            }
            if *op == f.xor || *op == f.sub {
                return Some(vec![intrinsic(*dest, f.zero, vec![])]);
            }
        }
        // Integer lane arithmetic is wrapping, independently per lane.
        if *op == f.sub
            && let Some(inner) = args(a, f.add, 2, defs)
        {
            for i in [0, 1] {
                if inner[i] == b {
                    return Some(copy(*dest, inner[1 - i]));
                }
            }
        }
        if *op != f.sub {
            for (x, y) in [(a, b), (b, a)] {
                if *op == f.add
                    && let Some(inner) = args(x, f.sub, 2, defs)
                    && inner[1] == y
                {
                    return Some(copy(*dest, inner[0]));
                }
                if *op == f.xor
                    && let Some(inner) = args(x, f.xor, 2, defs)
                {
                    for i in [0, 1] {
                        if inner[i] == y {
                            return Some(copy(*dest, inner[1 - i]));
                        }
                    }
                }
                let opposite = if *op == f.and {
                    Some(f.or)
                } else if *op == f.or {
                    Some(f.and)
                } else {
                    None
                };
                if let Some(opposite) = opposite
                    && let Some(inner) = args(x, opposite, 2, defs)
                    && inner.contains(&y)
                {
                    return Some(copy(*dest, y));
                }
            }
            if let (Some(left), Some(right)) = (
                f.blend.and_then(|op| args(a, op, 3, defs)),
                f.blend.and_then(|op| args(b, op, 3, defs)),
            ) && left[2] == right[2]
                && left[0] == right[1]
                && left[1] == right[0]
                && (*op != f.add || canonical_mask(left[2], f.blend, defs))
            {
                return Some(vec![intrinsic(*dest, *op, vec![left[0], left[1]])]);
            }
        }
        // Two private inner operations pay for the new intermediate.
        let inner = if *op == f.or {
            Some(f.and)
        } else if *op == f.and {
            Some(f.or)
        } else if *op == f.xor {
            Some(f.xor)
        } else {
            None
        };
        if let Some(inner) = inner
            && private(a, uses, defs)
            && private(b, uses, defs)
            && let (Some(left), Some(right)) = (args(a, inner, 2, defs), args(b, inner, 2, defs))
        {
            for i in [0, 1] {
                for j in [0, 1] {
                    if left[i] == right[j] {
                        if *op == f.xor {
                            return Some(vec![intrinsic(
                                *dest,
                                f.xor,
                                vec![left[1 - i], right[1 - j]],
                            )]);
                        }
                        if fresh.0.checked_add(1).is_some() {
                            return Some(vec![
                                intrinsic(fresh, *op, vec![left[1 - i], right[1 - j]]),
                                intrinsic(*dest, inner, vec![left[i], Operand::Value(fresh)]),
                            ]);
                        }
                    }
                }
            }
        }
    }
    None
}
pub(crate) fn run(module: &mut IrModule) -> usize {
    let families = families();
    module.for_each_function(|f| run_function(f, &families))
}
fn run_function(func: &mut IrFunction, families: &[Family]) -> usize {
    if super::policy::fp_environment_blocks_reordering(func) {
        return 0;
    }
    let mut uses = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            for value in inst.used_values() {
                *uses.entry(Value(value)).or_insert(0u32) += 1;
            }
        }
        for value in block.terminator.used_values() {
            *uses.entry(Value(value)).or_insert(0u32) += 1;
        }
    }
    let mut next = func
        .next_value_id
        .max(func.max_value_id().saturating_add(1));
    let mut count = 0;
    for block in &mut func.blocks {
        let mut defs = FxHashMap::default();
        let old = std::mem::take(&mut block.instructions);
        let spans = std::mem::take(&mut block.source_spans);
        let keep_spans = spans.len() == old.len();
        for (index, inst) in old.into_iter().enumerate() {
            let replacement = fold(&inst, &defs, &uses, families, Value(next));
            let emitted = if let Some(replacement) = replacement {
                if replacement.len() > 1 {
                    next += 1;
                }
                count += 1;
                replacement
            } else {
                vec![inst]
            };
            for inst in emitted {
                if let Some(dest) = inst.dest() {
                    defs.insert(dest, inst.clone());
                }
                block.instructions.push(inst);
                if keep_spans {
                    block.source_spans.push(spans[index]);
                }
            }
        }
    }
    func.next_value_id = next;
    count
}
#[cfg(test)]
#[path = "vec_relations_tests.rs"]
mod tests;
