//! Conservative, bounded SSA range reasoning for optimized VLA diagnostics.
//! Unknown arithmetic is never used as a proof of a small allocation. Only
//! dominating, single-predecessor branch edges constrain a value; a join block
//! dominating an allocation does NOT prove which incoming edge was taken.
use crate::common::error::{DiagnosticEngine, WarningKind};
use crate::common::fx_hash::FxHashMap;
use crate::common::types::IrType;
use crate::ir::analysis::{CfgAnalysis, DominanceChecker, build_label_map};
use crate::ir::reexports::*;

#[derive(Clone, Copy, Debug)]
struct Bounds(i128, i128);

fn domain(ty: IrType) -> Option<Bounds> {
    let bits = ty.size() * 8;
    if !ty.is_integer() || bits == 0 || bits > 64 {
        return None;
    }
    Some(if ty.is_unsigned() {
        Bounds(0, (1i128 << bits) - 1)
    } else {
        Bounds(-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
    })
}

struct Facts<'a> {
    defs: FxHashMap<u32, &'a Instruction>,
    guards: FxHashMap<u32, Bounds>,
}

impl Facts<'_> {
    fn value_type(&self, mut value: Value) -> Option<IrType> {
        for _ in 0..16 {
            let inst = *self.defs.get(&value.0)?;
            if let Instruction::Copy {
                src: Operand::Value(src),
                ..
            } = inst
            {
                value = *src;
            } else {
                return inst.result_type();
            }
        }
        None
    }

    fn range(&self, op: Operand, depth: u8, budget: &mut usize) -> Option<Bounds> {
        if depth == 0 || *budget == 0 {
            return None;
        }
        *budget -= 1;
        if let Operand::Const(c) = op {
            return c.to_i128().map(|n| Bounds(n, n));
        }
        let Operand::Value(v) = op else {
            return None;
        };
        let inst = *self.defs.get(&v.0)?;
        let get = |op, budget: &mut usize| self.range(op, depth - 1, budget);
        let fallback = inst.result_type().and_then(domain);
        let result = match inst {
            Instruction::Copy { src, .. } => get(*src, budget),
            Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            } => {
                // A typed consumer may reinterpret/truncate its producer's
                // bits. Validate the source interval against from_ty before
                // treating this as a numeric widening conversion.
                let from = domain(*from_ty);
                let source = match (get(*src, budget), from) {
                    (Some(a), Some(b)) if a.0 >= b.0 && a.1 <= b.1 => Some(a),
                    _ => from,
                };
                let target = domain(*to_ty);
                match (source, target) {
                    (Some(a), Some(b)) if a.0 >= b.0 && a.1 <= b.1 => Some(a),
                    _ => target,
                }
            }
            Instruction::BinOp {
                op, lhs, rhs, ty, ..
            } => {
                let a = get(*lhs, budget);
                let b = get(*rhs, budget);
                let r = match (op, a, b) {
                    (IrBinOp::And, _, Some(b)) if b.0 >= 0 => Some(Bounds(0, b.1)),
                    (IrBinOp::And, Some(a), _) if a.0 >= 0 => Some(Bounds(0, a.1)),
                    (IrBinOp::URem, _, Some(b)) if b.0 == b.1 && b.0 > 0 => {
                        Some(Bounds(0, b.0 - 1))
                    }
                    (IrBinOp::Add, Some(a), Some(b)) => {
                        a.0.checked_add(b.0)
                            .zip(a.1.checked_add(b.1))
                            .map(|(lo, hi)| Bounds(lo, hi))
                    }
                    (IrBinOp::Sub, Some(a), Some(b)) => {
                        a.0.checked_sub(b.1)
                            .zip(a.1.checked_sub(b.0))
                            .map(|(lo, hi)| Bounds(lo, hi))
                    }
                    (IrBinOp::Shl, Some(a), Some(b))
                        if a.0 >= 0 && b.0 == b.1 && b.0 >= 0 && b.0 < (ty.size() * 8) as i128 =>
                    {
                        (1i128.checked_shl(b.0 as u32)).and_then(|scale| {
                            a.0.checked_mul(scale)
                                .zip(a.1.checked_mul(scale))
                                .map(|(lo, hi)| Bounds(lo, hi))
                        })
                    }
                    (IrBinOp::Mul, Some(a), Some(b)) if a.0 >= 0 && b.0 >= 0 => {
                        a.0.checked_mul(b.0)
                            .zip(a.1.checked_mul(b.1))
                            .map(|(lo, hi)| Bounds(lo, hi))
                    }
                    _ => None,
                };
                let target = domain(*ty);
                match (r, target) {
                    (Some(a), Some(b)) if a.0 >= b.0 && a.1 <= b.1 => Some(a),
                    _ => target,
                }
            }
            Instruction::Cmp { .. } => Some(Bounds(0, 1)),
            _ => fallback,
        };
        match (result, self.guards.get(&v.0)) {
            (Some(a), Some(b)) => Some(Bounds(a.0.max(b.0), a.1.min(b.1))),
            (None, Some(b)) => Some(*b),
            _ => result,
        }
    }

    fn constrain(&mut self, value: Value, op: IrCmpOp, k: i128, ty: IrType) {
        // Mixed-width IR consumers can observe only the low bits. A bound
        // on those bits is not a bound on the full-width producer.
        if self.value_type(value).is_none_or(|t| t.size() != ty.size()) {
            return;
        }
        let Some(mut r) = self.range(Operand::Value(value), 16, &mut 256) else {
            return;
        };
        let unsigned = matches!(
            op,
            IrCmpOp::Ult | IrCmpOp::Ule | IrCmpOp::Ugt | IrCmpOp::Uge
        );
        // A signed comparison can also accept the unsigned producer's high
        // half (negative bit patterns). One ordinary interval cannot describe
        // that union, so do not discard it to prove a small allocation.
        if matches!(
            op,
            IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge
        ) && self
            .value_type(value)
            .is_some_and(|t| t.is_unsigned() && r.1 > (1i128 << (t.size() * 8 - 1)) - 1)
        {
            return;
        }
        // Signed bit patterns cannot be ordered as unsigned numeric values.
        // An unsigned upper bound below the sign bit DOES prove nonnegativity.
        if unsigned && !ty.is_unsigned() {
            if k < 0 || !matches!(op, IrCmpOp::Ult | IrCmpOp::Ule) {
                return;
            }
            let Some(d) = domain(ty) else {
                return;
            };
            if k > d.1 {
                return;
            }
            r.0 = r.0.max(0);
        }
        match op {
            IrCmpOp::Eq => {
                r.0 = r.0.max(k);
                r.1 = r.1.min(k);
            }
            IrCmpOp::Slt | IrCmpOp::Ult => {
                let Some(k) = k.checked_sub(1) else {
                    return;
                };
                r.1 = r.1.min(k);
            }
            IrCmpOp::Sle | IrCmpOp::Ule => r.1 = r.1.min(k),
            IrCmpOp::Sgt | IrCmpOp::Ugt => {
                let Some(k) = k.checked_add(1) else {
                    return;
                };
                r.0 = r.0.max(k);
            }
            IrCmpOp::Sge | IrCmpOp::Uge => r.0 = r.0.max(k),
            IrCmpOp::Ne => return,
        }
        if r.0 <= r.1 {
            self.propagate(value, r, 8);
        }
    }

    /// Invert only injective copies, widening casts, and constant affine
    /// operations whose inverse interval does not cross a bit-width wrap.
    /// In particular a truncating cast or multiplication is NOT invertible.
    fn propagate(&mut self, value: Value, mut r: Bounds, depth: u8) {
        if depth == 0 {
            return;
        }
        if let Some(old) = self.guards.get(&value.0) {
            r = Bounds(r.0.max(old.0), r.1.min(old.1));
        }
        if r.0 > r.1 {
            return;
        }
        self.guards.insert(value.0, r);
        let next = match self.defs.get(&value.0).copied() {
            Some(Instruction::Copy {
                src: Operand::Value(v),
                ..
            }) => Some((*v, r)),
            Some(Instruction::Cast {
                src: Operand::Value(v),
                from_ty,
                to_ty,
                ..
            }) if from_ty.size() <= to_ty.size() && self.value_type(*v) == Some(*from_ty) => {
                match (domain(*from_ty), domain(*to_ty)) {
                    (Some(a), Some(b)) if r.0 >= a.0.max(b.0) && r.1 <= a.1.min(b.1) => {
                        Some((*v, r))
                    }
                    _ => None,
                }
            }
            Some(Instruction::BinOp {
                op,
                lhs: Operand::Value(v),
                rhs: Operand::Const(c),
                ty,
                ..
            }) if matches!(op, IrBinOp::Add | IrBinOp::Sub)
                && self
                    .value_type(*v)
                    .and_then(domain)
                    .zip(domain(*ty))
                    .is_some_and(|(a, b)| a.0 >= b.0 && a.1 <= b.1) =>
            {
                c.to_i128().and_then(|k| {
                    let offset = if *op == IrBinOp::Sub {
                        Some(k)
                    } else {
                        k.checked_neg()
                    }?;
                    let inverse = Bounds(r.0.checked_add(offset)?, r.1.checked_add(offset)?);
                    let d = domain(*ty)?;
                    (inverse.0 >= d.0 && inverse.1 <= d.1).then_some((*v, inverse))
                })
            }
            _ => None,
        };
        if let Some((v, range)) = next {
            self.propagate(v, range, depth - 1);
        }
    }
}

fn invert(op: IrCmpOp) -> IrCmpOp {
    use IrCmpOp::*;
    match op {
        Eq => Ne,
        Ne => Eq,
        Slt => Sge,
        Sle => Sgt,
        Sgt => Sle,
        Sge => Slt,
        Ult => Uge,
        Ule => Ugt,
        Ugt => Ule,
        Uge => Ult,
    }
}
fn swap(op: IrCmpOp) -> IrCmpOp {
    use IrCmpOp::*;
    match op {
        Eq => Eq,
        Ne => Ne,
        Slt => Sgt,
        Sle => Sge,
        Sgt => Slt,
        Sge => Sle,
        Ult => Ugt,
        Ule => Uge,
        Ugt => Ult,
        Uge => Ule,
    }
}

pub(super) fn check(module: &IrModule, diagnostics: &mut DiagnosticEngine, pointer_bits: u32) {
    if !diagnostics.warning_enabled(WarningKind::VlaLargerThan) {
        return;
    }
    for func in &module.functions {
        if !func.blocks.iter().any(|b| {
            b.instructions.iter().any(|i| {
                matches!(
                    i,
                    Instruction::DynAlloca {
                        vla_span: Some(_),
                        ..
                    }
                )
            })
        }) {
            continue;
        }
        let labels = build_label_map(func);
        let cfg = CfgAnalysis::build(func);
        let dom = DominanceChecker::new(func.blocks.len(), &cfg.idom);
        let defs = func
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .filter_map(|i| i.dest().map(|v| (v.0, i)))
            .collect();
        let mut facts = Facts {
            defs,
            guards: FxHashMap::default(),
        };
        for (at, block) in func.blocks.iter().enumerate() {
            for inst in &block.instructions {
                let Instruction::DynAlloca {
                    size,
                    vla_span: Some(span),
                    ..
                } = inst
                else {
                    continue;
                };
                if !diagnostics.warning_enabled_at(WarningKind::VlaLargerThan, *span) {
                    continue;
                }
                facts.guards.clear();
                let mut ancestor = at;
                for _ in 0..128 {
                    let parent = cfg.idom[ancestor];
                    if parent == usize::MAX || parent == ancestor {
                        break;
                    }
                    ancestor = parent;
                    let Terminator::CondBranch {
                        cond: Operand::Value(cond),
                        true_label,
                        false_label,
                    } = &func.blocks[parent].terminator
                    else {
                        continue;
                    };
                    let Some(&t) = labels.get(true_label) else {
                        continue;
                    };
                    let Some(&f) = labels.get(false_label) else {
                        continue;
                    };
                    let edge_dominates =
                        |s| cfg.preds.row(s) == [parent as u32] && dom.dominates(s, at);
                    let truth = match (edge_dominates(t), edge_dominates(f)) {
                        (true, false) => true,
                        (false, true) => false,
                        _ => continue,
                    };
                    let Some(Instruction::Cmp {
                        op, lhs, rhs, ty, ..
                    }) = facts.defs.get(&cond.0).copied()
                    else {
                        continue;
                    };
                    let op = if truth { *op } else { invert(*op) };
                    match (lhs, rhs) {
                        (Operand::Value(v), Operand::Const(c)) => {
                            if let Some(k) = c.to_i128() {
                                facts.constrain(*v, op, k, *ty);
                            }
                        }
                        (Operand::Const(c), Operand::Value(v)) => {
                            if let Some(k) = c.to_i128() {
                                facts.constrain(*v, swap(op), k, *ty);
                            }
                        }
                        _ => {}
                    }
                }
                let range = facts.range(*size, 16, &mut 256);
                // Syntactic VLA provenance remains significant even when
                // optimization proves a constant extent. GCC diagnoses that
                // extent; only genuinely fixed arrays lack VLA provenance.
                let max = range
                    .filter(|r| r.0 >= 0)
                    .map(|r| r.1 as u128)
                    .unwrap_or((1u128 << pointer_bits) - 1);
                if max.min((1u128 << pointer_bits) - 1)
                    > u128::from(diagnostics.vla_limit_at(*span))
                {
                    diagnostics.warning_with_kind(
                        "argument to variable-length array may be too large",
                        *span,
                        WarningKind::VlaLargerThan,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(instructions: &[Instruction]) -> Facts<'_> {
        Facts {
            defs: instructions
                .iter()
                .filter_map(|i| i.dest().map(|v| (v.0, i)))
                .collect(),
            guards: FxHashMap::default(),
        }
    }
    fn parameter(ty: IrType) -> Instruction {
        Instruction::ParamRef {
            dest: Value(0),
            param_idx: 0,
            ty,
        }
    }
    fn binary(op: IrBinOp, ty: IrType, k: i64) -> Instruction {
        Instruction::BinOp {
            dest: Value(1),
            op,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Const(IrConst::I64(k)),
            ty,
        }
    }
    fn bounds(f: &Facts<'_>, v: u32) -> (i128, i128) {
        let r = f.range(Operand::Value(Value(v)), 16, &mut 256).unwrap();
        (r.0, r.1)
    }
    #[test]
    fn affine_positive_guard_propagates_without_wrap() {
        let instructions = [parameter(IrType::I32), binary(IrBinOp::Sub, IrType::I32, 1)];
        let mut f = facts(&instructions);
        f.constrain(Value(1), IrCmpOp::Ule, 15, IrType::I32);
        assert_eq!(bounds(&f, 0), (1, 16));
    }
    #[test]
    fn optimized_element_size_shift_retains_guard() {
        let instructions = [parameter(IrType::U32), binary(IrBinOp::Shl, IrType::I64, 2)];
        let mut f = facts(&instructions);
        f.constrain(Value(0), IrCmpOp::Ule, 16, IrType::U32);
        assert_eq!(bounds(&f, 1), (0, 64));
        let overflowing = [parameter(IrType::U32), binary(IrBinOp::Shl, IrType::U32, 2)];
        assert_eq!(bounds(&facts(&overflowing), 1), (0, u32::MAX as i128));
    }
    #[test]
    fn narrowing_comparison_cannot_bound_wide_producer() {
        let instructions = [parameter(IrType::U64)];
        let mut f = facts(&instructions);
        f.constrain(Value(0), IrCmpOp::Ule, 15, IrType::U32);
        assert_eq!(bounds(&f, 0), (0, u64::MAX as i128));
    }
    #[test]
    fn wrapping_inverse_is_not_a_proof() {
        let instructions = [parameter(IrType::U32), binary(IrBinOp::Add, IrType::U32, 1)];
        let mut f = facts(&instructions);
        f.constrain(Value(1), IrCmpOp::Ule, 15, IrType::U32);
        assert_eq!(bounds(&f, 0), (0, u32::MAX as i128));
    }
    #[test]
    fn signed_comparison_cannot_discard_unsigned_high_half() {
        let instructions = [parameter(IrType::U32)];
        let mut f = facts(&instructions);
        f.constrain(Value(0), IrCmpOp::Slt, 16, IrType::I32);
        assert_eq!(bounds(&f, 0), (0, u32::MAX as i128));
    }
    #[test]
    fn mask_remainder_and_analysis_budgets() {
        for op in [IrBinOp::And, IrBinOp::URem] {
            let instructions = [parameter(IrType::U32), binary(op, IrType::U32, 16)];
            let f = facts(&instructions);
            assert_eq!(bounds(&f, 1), (0, if op == IrBinOp::And { 16 } else { 15 }));
            assert!(f.range(Operand::Value(Value(1)), 0, &mut 256).is_none());
            assert!(f.range(Operand::Value(Value(1)), 16, &mut 0).is_none());
        }
    }
    #[test]
    fn cast_consumer_width_and_signedness_do_not_bound_original_bits() {
        for (producer, from) in [(IrType::U64, IrType::U8), (IrType::U8, IrType::I8)] {
            let instructions = [
                parameter(producer),
                Instruction::Cast {
                    dest: Value(1),
                    src: Operand::Value(Value(0)),
                    from_ty: from,
                    to_ty: IrType::I64,
                },
            ];
            let mut f = facts(&instructions);
            f.constrain(Value(1), IrCmpOp::Sge, -50, IrType::I64);
            f.constrain(Value(1), IrCmpOp::Sle, 100, IrType::I64);
            let d = domain(producer).unwrap();
            assert_eq!(bounds(&f, 0), (d.0, d.1));
        }
    }
    #[test]
    fn cast_reinterpreting_source_cannot_keep_out_of_domain_numeric_bounds() {
        let instructions = [
            parameter(IrType::U32),
            Instruction::Cast {
                dest: Value(1),
                src: Operand::Value(Value(0)),
                from_ty: IrType::I8,
                to_ty: IrType::I64,
            },
        ];
        let mut f = facts(&instructions);
        f.guards.insert(0, Bounds(200, 200));
        assert_eq!(bounds(&f, 1), (-128, 127));
        // Exhaust every byte pattern rather than sampling positive values.
        for n in 0..=255 {
            f.guards.insert(0, Bounds(n, n));
            let (lo, hi) = bounds(&f, 1);
            let actual = i128::from(n as u8 as i8);
            assert!(
                lo <= actual && actual <= hi,
                "n={n}: {lo}..{hi} excludes {actual}"
            );
        }
    }
    #[test]
    fn affine_inverse_requires_numeric_embedding_not_just_equal_width() {
        let instructions = [parameter(IrType::U8), binary(IrBinOp::Add, IrType::I8, 10)];
        let mut f = facts(&instructions);
        f.constrain(Value(1), IrCmpOp::Sge, -50, IrType::I8);
        f.constrain(Value(1), IrCmpOp::Sle, 100, IrType::I8);
        assert_eq!(bounds(&f, 0), (0, 255));
        let (lo, hi) = bounds(&f, 0);
        for n in 0..=255u8 {
            let actual = i128::from(n.wrapping_add(10) as i8);
            if (-50..=100).contains(&actual) {
                assert!(lo <= i128::from(n) && i128::from(n) <= hi);
            }
        }
        let widened = [parameter(IrType::U32), binary(IrBinOp::Add, IrType::I64, 1)];
        let mut f = facts(&widened);
        f.constrain(Value(1), IrCmpOp::Sle, 16, IrType::I64);
        assert_eq!(bounds(&f, 0), (0, 15));
    }
    #[test]
    fn comparisons_invert_and_swap_involutively() {
        for op in [
            IrCmpOp::Eq,
            IrCmpOp::Ne,
            IrCmpOp::Slt,
            IrCmpOp::Sle,
            IrCmpOp::Sgt,
            IrCmpOp::Sge,
            IrCmpOp::Ult,
            IrCmpOp::Ule,
            IrCmpOp::Ugt,
            IrCmpOp::Uge,
        ] {
            assert_eq!(invert(invert(op)), op);
            assert_eq!(swap(swap(op)), op);
        }
    }
}
