//! Small, width-aware unsigned bounds shared by IR cleanup and switch lowering.
//!
//! These are producer facts, not guesses from signedness or register contents.
//! A bound describes the low `bits` of a uniquely defined SSA value. Consumers
//! may truncate to fewer bits, but cannot infer more bits: a bounded I32
//! pattern is NOT a nonnegative I64 value without a proven normalizing cast. Unknown definitions,
//! copy cycles, multi-def webs and unsupported widths fail closed.
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::{
    Instruction, IrBinOp, IrFunction, IrUnaryOp, Operand, Terminator, Value,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Bound {
    bits: u32,
    max: u64,
}

fn bits(ty: IrType) -> Option<u32> {
    match ty {
        IrType::I8 | IrType::U8 => Some(8),
        IrType::I16 | IrType::U16 => Some(16),
        IrType::I32 | IrType::U32 => Some(32),
        IrType::I64 | IrType::U64 => Some(64),
        _ => None,
    }
}
fn mask(width: u32) -> u64 {
    u64::MAX >> (64 - width)
}
fn constant(op: Operand, width: u32) -> Option<u64> {
    let Operand::Const(c) = op else { return None };
    // This is explicit bit-vector normalization, NOT an address-offset cast.
    Some((c.to_i128()? as u128 & u128::from(mask(width))) as u64)
}
fn shift(op: Operand, width: u32) -> Option<u32> {
    let Operand::Const(c) = op else { return None };
    let n = u32::try_from(c.to_i128()?).ok()?;
    (n < width).then_some(n)
}

struct Bounds<'a> {
    defs: FxHashMap<u32, &'a Instruction>,
    multi: FxHashSet<u32>,
    active: FxHashSet<u32>,
    memo: FxHashMap<u32, Bound>,
}
impl<'a> Bounds<'a> {
    fn new(func: &'a IrFunction) -> Self {
        Self {
            defs: func
                .blocks
                .iter()
                .flat_map(|b| &b.instructions)
                .filter_map(|i| i.dest().map(|v| (v.0, i)))
                .collect(),
            multi: super::gvn::find_multi_def_values(func),
            active: FxHashSet::default(),
            memo: FxHashMap::default(),
        }
    }
    fn operand(&mut self, op: Operand, width: u32, depth: u32) -> Option<Bound> {
        match op {
            Operand::Const(_) => constant(op, width).map(|max| Bound { bits: width, max }),
            Operand::Value(v) => self
                .value(v, depth)
                .filter(|b| b.bits >= width)
                .map(|b| Bound {
                    bits: width,
                    // Typed consumers read low bits even without a Cast node
                    // (notably U64 bit counts consumed by C's I32 arithmetic).
                    // Truncation is bounded both by the original maximum and
                    // the destination mask; it never supplies unknown upper bits.
                    max: b.max.min(mask(width)),
                }),
        }
    }
    fn value(&mut self, v: Value, depth: u32) -> Option<Bound> {
        if depth == 0 || self.multi.contains(&v.0) || self.active.contains(&v.0) {
            return None;
        }
        if let Some(cached) = self.memo.get(&v.0) {
            return Some(*cached);
        }
        let inst = self.defs.get(&v.0).copied()?;
        self.active.insert(v.0);
        let result = self.instruction(inst, depth - 1);
        self.active.remove(&v.0);
        if let Some(bound) = result {
            self.memo.insert(v.0, bound);
        }
        result
    }
    fn instruction(&mut self, inst: &Instruction, depth: u32) -> Option<Bound> {
        match inst {
            Instruction::Copy {
                src: Operand::Value(v),
                ..
            } => self.value(*v, depth),
            Instruction::Cmp { .. } => Some(Bound { bits: 8, max: 1 }),
            Instruction::UnaryOp {
                op:
                    IrUnaryOp::Clz
                    | IrUnaryOp::ClzNonZero
                    | IrUnaryOp::Ctz
                    | IrUnaryOp::CtzNonZero
                    | IrUnaryOp::Popcount,
                ty,
                ..
            } if matches!(ty, IrType::I32 | IrType::U32 | IrType::I64 | IrType::U64) => {
                let width = bits(*ty)?;
                Some(Bound {
                    bits: width,
                    max: u64::from(width),
                })
            }
            Instruction::Load { ty, .. } | Instruction::ParamRef { ty, .. } if ty.is_unsigned() => {
                let width = bits(*ty)?;
                Some(Bound {
                    bits: width,
                    max: mask(width),
                })
            }
            Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            } => {
                let from = bits(*from_ty)?;
                let to = bits(*to_ty)?;
                let input = self.operand(*src, from, depth)?;
                if to > from && !from_ty.is_unsigned() && input.max >= (1u64 << (from - 1)) {
                    return None;
                }
                Some(Bound {
                    bits: to,
                    max: input.max.min(mask(to)),
                })
            }
            Instruction::BinOp {
                op: IrBinOp::And,
                lhs,
                rhs,
                ty,
                ..
            } => {
                let width = bits(*ty)?;
                let (limit, other) = if let Some(n) = constant(*rhs, width) {
                    (n, *lhs)
                } else {
                    (constant(*lhs, width)?, *rhs)
                };
                let max = self
                    .operand(other, width, depth)
                    .map_or(limit, |b| b.max.min(limit));
                Some(Bound { bits: width, max })
            }
            Instruction::BinOp {
                op: IrBinOp::LShr,
                lhs,
                rhs,
                ty,
                ..
            } => {
                let width = bits(*ty)?;
                let n = shift(*rhs, width)?;
                let upper = self
                    .operand(*lhs, width, depth)
                    .map_or(mask(width), |b| b.max);
                Some(Bound {
                    bits: width,
                    max: upper >> n,
                })
            }
            Instruction::BinOp {
                op: IrBinOp::AShr,
                lhs,
                rhs,
                ty,
                ..
            } => {
                let width = bits(*ty)?;
                let n = shift(*rhs, width)?;
                let input = self.operand(*lhs, width, depth)?;
                (input.max < (1u64 << (width - 1))).then_some(Bound {
                    bits: width,
                    max: input.max >> n,
                })
            }
            _ => None,
        }
    }
}

/// Keep the cast (and its destination width); change only an extension whose
/// signed and unsigned meanings are proved identical. Untyped Copy is not a
/// substitute for a width-normalizing cast in this backend.
pub(crate) fn run(func: &mut IrFunction) -> usize {
    let mut plans = Vec::new();
    {
        let mut bounds = Bounds::new(func);
        for (bi, block) in func.blocks.iter().enumerate() {
            for (ii, inst) in block.instructions.iter().enumerate() {
                if inst.dest().is_some_and(|d| bounds.multi.contains(&d.0)) {
                    continue;
                }
                match inst {
                    Instruction::BinOp {
                        dest,
                        op: IrBinOp::AShr,
                        lhs,
                        rhs,
                        ty,
                    } => {
                        let Some(width) = bits(*ty) else { continue };
                        if shift(*rhs, width).is_some()
                            && bounds
                                .operand(*lhs, width, 64)
                                .is_some_and(|b| b.max < (1u64 << (width - 1)))
                        {
                            plans.push((
                                bi,
                                ii,
                                Instruction::BinOp {
                                    dest: *dest,
                                    op: IrBinOp::LShr,
                                    lhs: *lhs,
                                    rhs: *rhs,
                                    ty: *ty,
                                },
                            ));
                        }
                    }
                    Instruction::Cast {
                        dest,
                        src,
                        from_ty,
                        to_ty,
                    } if !from_ty.is_unsigned() => {
                        let (Some(from), Some(to)) = (bits(*from_ty), bits(*to_ty)) else {
                            continue;
                        };
                        if to > from
                            && bounds
                                .operand(*src, from, 64)
                                .is_some_and(|b| b.max < (1u64 << (from - 1)))
                        {
                            plans.push((
                                bi,
                                ii,
                                Instruction::Cast {
                                    dest: *dest,
                                    src: *src,
                                    from_ty: from_ty.to_unsigned(),
                                    to_ty: *to_ty,
                                },
                            ));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let count = plans.len();
    for (bi, ii, replacement) in plans {
        func.blocks[bi].instructions[ii] = replacement;
    }
    count
}

/// Recompute after phi lowering; never retain pre-lowering SSA assumptions.
pub(crate) fn switch_bounds(func: &IrFunction) -> FxHashMap<u32, (u32, u64)> {
    let mut result = FxHashMap::default();
    let disabled = std::env::var("CCC_DISABLE_PASSES").unwrap_or_default();
    if super::pass_disabled(disabled, "intbounds")
        || !func
            .blocks
            .iter()
            .any(|b| matches!(b.terminator, Terminator::Switch { .. }))
    {
        return result;
    }
    let mut bounds = Bounds::new(func);
    for block in &func.blocks {
        if let Terminator::Switch {
            val: Operand::Value(v),
            ..
        } = block.terminator
            && let Some(b) = bounds.value(v, 64)
        {
            result.insert(v.0, (b.bits, b.max));
        }
    }
    result
}

pub(crate) fn covers_switch(bound: (u32, u64), ty: IrType, minimum: i64, range: usize) -> bool {
    if minimum != 0 || !matches!(ty, IrType::I32 | IrType::U32 | IrType::I64 | IrType::U64) {
        return false;
    }
    let Some(width) = bits(ty) else { return false };
    let maximum = bound.1.min(mask(width));
    matches!(bound.0, 8 | 16 | 32 | 64)
        && bound.0 >= width
        && u64::try_from(range).is_ok_and(|n| maximum < n)
        && (ty.is_unsigned() || maximum < (1u64 << (width - 1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, IrConst, IrParam};

    fn fixture(mut instructions: Vec<Instruction>) -> IrFunction {
        let param = IrParam {
            ty: IrType::U64,
            noalias: false,
            struct_size: None,
            struct_align: None,
            param_align: None,
            struct_eightbyte_classes: vec![],
            is_f128_sse: false,
            riscv_float_class: None,
        };
        instructions.insert(
            0,
            Instruction::ParamRef {
                dest: Value(0),
                param_idx: 0,
                ty: IrType::U64,
            },
        );
        let mut f = IrFunction::new("bounds".into(), IrType::Void, vec![param], false);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(None),
            source_spans: vec![],
        });
        f
    }
    fn and(dest: u32, mask: i64, ty: IrType) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op: IrBinOp::And,
            lhs: Operand::Value(Value(0)),
            rhs: Operand::Const(IrConst::I64(mask)),
            ty,
        }
    }
    fn cast(dest: u32, src: u32, from_ty: IrType, to_ty: IrType) -> Instruction {
        Instruction::Cast {
            dest: Value(dest),
            src: Operand::Value(Value(src)),
            from_ty,
            to_ty,
        }
    }
    #[test]
    fn count_shift_and_widen_retain_exact_widths() {
        let mut f = fixture(vec![
            Instruction::UnaryOp {
                dest: Value(1),
                op: IrUnaryOp::Ctz,
                src: Operand::Value(Value(0)),
                ty: IrType::U64,
            },
            cast(2, 1, IrType::U64, IrType::I32),
            Instruction::BinOp {
                dest: Value(3),
                op: IrBinOp::AShr,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Const(IrConst::I32(3)),
                ty: IrType::I32,
            },
            cast(4, 3, IrType::I32, IrType::I64),
        ]);
        assert_eq!(
            Bounds::new(&f).value(Value(3), 64),
            Some(Bound { bits: 32, max: 8 })
        );
        assert_eq!(run(&mut f), 2);
        assert!(matches!(
            f.blocks[0].instructions[3],
            Instruction::BinOp {
                op: IrBinOp::LShr,
                ty: IrType::I32,
                ..
            }
        ));
        assert!(matches!(
            f.blocks[0].instructions[4],
            Instruction::Cast {
                from_ty: IrType::U32,
                to_ty: IrType::I64,
                ..
            }
        ));
        assert_eq!(run(&mut f), 0);
    }
    #[test]
    fn typed_narrow_consumers_need_no_intervening_cast() {
        let mut f = fixture(vec![
            Instruction::UnaryOp {
                dest: Value(1),
                op: IrUnaryOp::Ctz,
                src: Operand::Value(Value(0)),
                ty: IrType::U64,
            },
            Instruction::BinOp {
                dest: Value(2),
                op: IrBinOp::AShr,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I32(3)),
                ty: IrType::I32,
            },
            cast(3, 2, IrType::I32, IrType::I64),
        ]);
        assert_eq!(
            Bounds::new(&f).value(Value(2), 64),
            Some(Bound { bits: 32, max: 8 })
        );
        assert_eq!(run(&mut f), 2);
        let mut negative = fixture(vec![
            and(1, 65535, IrType::I32),
            cast(2, 1, IrType::I16, IrType::I64),
        ]);
        assert_eq!(
            run(&mut negative),
            0,
            "narrow signed source can still be negative"
        );
    }
    #[test]
    fn an_upper_bound_is_not_a_known_bit_mask_for_or() {
        let f = fixture(vec![
            Instruction::UnaryOp {
                dest: Value(1),
                op: IrUnaryOp::Popcount,
                src: Operand::Value(Value(0)),
                ty: IrType::U64,
            },
            Instruction::BinOp {
                dest: Value(2),
                op: IrBinOp::LShr,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I32(3)),
                ty: IrType::U64,
            },
            Instruction::BinOp {
                dest: Value(3),
                op: IrBinOp::Or,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Const(IrConst::I32(8)),
                ty: IrType::U64,
            },
        ]);
        assert_eq!(
            Bounds::new(&f).value(Value(2), 64),
            Some(Bound { bits: 64, max: 8 })
        );
        assert_eq!(Bounds::new(&f).value(Value(3), 64), None);
        // A count of 56 gives (56 >> 3) | 8 == 15, NOT <= (8 | 8).
    }
    #[test]
    fn count_families_include_the_defined_zero_result() {
        for op in [
            IrUnaryOp::Ctz,
            IrUnaryOp::CtzNonZero,
            IrUnaryOp::Clz,
            IrUnaryOp::ClzNonZero,
            IrUnaryOp::Popcount,
        ] {
            let f = fixture(vec![Instruction::UnaryOp {
                dest: Value(1),
                op,
                src: Operand::Value(Value(0)),
                ty: IrType::U64,
            }]);
            assert_eq!(
                Bounds::new(&f).value(Value(1), 64),
                Some(Bound { bits: 64, max: 64 })
            );
        }
    }
    #[test]
    fn sign_bit_and_consumer_width_cannot_be_ignored() {
        for n in [-1, i32::MIN as i64, i32::MAX as i64 + 1] {
            let mut f = fixture(vec![
                and(1, n, IrType::I32),
                cast(2, 1, IrType::I32, IrType::I64),
            ]);
            assert_eq!(run(&mut f), 0);
        }
        let mut f = fixture(vec![
            and(1, 15, IrType::I8),
            cast(2, 1, IrType::I32, IrType::I64),
        ]);
        assert_eq!(
            run(&mut f),
            0,
            "a low-byte fact cannot supply the other bits of a 32-bit source"
        );
    }
    #[test]
    fn truncation_does_not_authorize_a_later_signed_extension() {
        let mut f = fixture(vec![
            and(1, 255, IrType::I32),
            cast(2, 1, IrType::I32, IrType::I8),
            cast(3, 2, IrType::I8, IrType::I64),
        ]);
        assert_eq!(
            Bounds::new(&f).value(Value(2), 64),
            Some(Bound { bits: 8, max: 255 })
        );
        assert_eq!(Bounds::new(&f).value(Value(3), 64), None);
        assert_eq!(run(&mut f), 0);
    }
    #[test]
    fn unsupported_domains_and_bad_shift_counts_are_rejected() {
        for ty in [IrType::I128, IrType::U128, IrType::Ptr, IrType::F64] {
            let f = fixture(vec![and(1, 15, ty)]);
            assert_eq!(Bounds::new(&f).value(Value(1), 64), None);
        }
        for n in [
            IrConst::I32(-1),
            IrConst::I32(32),
            IrConst::I128(1i128 << 80),
        ] {
            let mut f = fixture(vec![
                and(1, 15, IrType::I32),
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::AShr,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(n),
                    ty: IrType::I32,
                },
            ]);
            assert_eq!(run(&mut f), 0);
        }
    }
    #[test]
    fn multidef_and_cyclic_copy_webs_do_not_supply_facts() {
        let f = fixture(vec![and(1, 15, IrType::I32), and(1, 31, IrType::I32)]);
        assert_eq!(Bounds::new(&f).value(Value(1), 64), None);
        let f = fixture(vec![
            Instruction::Copy {
                dest: Value(1),
                src: Operand::Value(Value(2)),
            },
            Instruction::Copy {
                dest: Value(2),
                src: Operand::Value(Value(1)),
            },
        ]);
        assert_eq!(Bounds::new(&f).value(Value(1), 64), None);
        let mut f = fixture(vec![and(1, 15, IrType::I32)]);
        for n in 2..100 {
            f.blocks[0].instructions.push(Instruction::Copy {
                dest: Value(n),
                src: Operand::Value(Value(n - 1)),
            });
        }
        let mut b = Bounds::new(&f);
        assert_eq!(b.value(Value(99), 64), None);
        assert_eq!(b.value(Value(5), 64), Some(Bound { bits: 32, max: 15 }));
    }
    #[test]
    fn zero_based_table_proof_preserves_width_sign_and_default_holes() {
        assert!(covers_switch((32, 15), IrType::I32, 0, 16));
        assert!(!covers_switch((32, 16), IrType::I32, 0, 16));
        assert!(covers_switch((64, 15), IrType::I32, 0, 16));
        assert!(!covers_switch((32, 15), IrType::I64, 0, 16));
        assert!(!covers_switch((64, 0xffff_ffff), IrType::I32, 0, 16));
        assert!(!covers_switch((32, 15), IrType::I32, 1, 16));
        assert!(!covers_switch((8, 15), IrType::I8, 0, 16));
        assert!(!covers_switch((64, u64::MAX), IrType::I64, 0, 16));
        let (table, minimum, range) = crate::backend::traits::build_jump_table(
            &[(0, BlockId(1)), (15, BlockId(2))],
            &BlockId(3),
        );
        assert!(covers_switch((32, 15), IrType::I32, minimum, range));
        assert_eq!(
            table[7],
            BlockId(3),
            "proving outer bounds must not bypass a hole's default"
        );
    }
    #[test]
    fn per_function_reset_discards_selector_facts() {
        let mut state = crate::backend::state::CodegenState::new();
        state.switch_unsigned_bounds.insert(1, (32, 15));
        state.reset_for_function();
        assert!(state.switch_unsigned_bounds.is_empty());
    }
    #[test]
    fn exhaustive_small_signed_widening_and_narrowing_bounds() {
        for limit in 0..256 {
            let f = fixture(vec![
                and(1, limit, IrType::I8),
                cast(2, 1, IrType::I8, IrType::I16),
            ]);
            let result = Bounds::new(&f).value(Value(2), 64);
            if limit >= 128 {
                assert_eq!(result, None);
            } else {
                let bound = result.unwrap();
                for value in 0..=limit {
                    assert!((value as i8 as i16 as u16 as u64) <= bound.max);
                }
            }
        }
        for limit in [17, 127, 128, 255, 256, 32767, 32768, 65535] {
            let f = fixture(vec![
                and(1, limit, IrType::I16),
                cast(2, 1, IrType::I16, IrType::I8),
            ]);
            let bound = Bounds::new(&f).value(Value(2), 64).unwrap();
            for value in 0..=limit {
                assert!(u64::from(value as u8) <= bound.max);
            }
        }
    }
}
