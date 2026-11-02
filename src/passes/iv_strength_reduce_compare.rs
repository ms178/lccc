//! Narrow production cost rule for load-search loop bounds. General scalar
//! IVSR remains opt-in: carrying every multiply can increase register pressure.
use super::*;
thread_local! {
    static ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
pub(crate) fn set_enabled(value: bool) {
    ENABLED.with(|v| v.set(value));
}
pub(super) fn enabled() -> bool {
    ENABLED.with(|v| v.get())
}

pub(super) fn eligible(f: &IrFunction, lp: &NaturalLoop, iv: &BasicIV, d: &DerivedExpr) -> bool {
    use crate::common::types::AddressSpace;
    use crate::ir::reexports::IrCmpOp;
    if iv.ty != IrType::U64
        || d.mul_ty != IrType::U64
        || iv.step != 1
        || d.add_offset != 1
        || d.stride <= 8
        || !(d.stride as u64).is_power_of_two()
        || lp.body.len() > 3
        || crate::common::types::target_ptr_size() != 8
    {
        return false;
    }
    let mut loads = 0;
    let mut size = 0;
    for &bi in &lp.body {
        size += f.blocks[bi].instructions.len();
        for i in &f.blocks[bi].instructions {
            match i {
                Instruction::Load {
                    volatile: false,
                    seg_override: AddressSpace::Default,
                    ..
                } => loads += 1,
                Instruction::UnaryOp { .. }
                | Instruction::Phi { .. }
                | Instruction::BinOp { .. }
                | Instruction::Copy { .. }
                | Instruction::Cast { .. }
                | Instruction::Cmp { .. }
                | Instruction::GetElementPtr { .. }
                | Instruction::GlobalAddr { .. } => {}
                _ => return false,
            }
        }
    }
    if loads == 0 || size > 24 {
        return false;
    }
    // Only one direct, in-loop comparison may read the scaled value. In
    // particular it cannot escape, become a pointer, or feed a carried value.
    let mut count = 0;
    for (bi, b) in f.blocks.iter().enumerate() {
        for i in &b.instructions {
            let mut uses = 0;
            crate::backend::liveness::for_each_operand_in_instruction(i, |o| {
                if *o == Operand::Value(d.mul_dest) {
                    uses += 1;
                }
            });
            crate::backend::liveness::for_each_value_use_in_instruction(i, |v| {
                if *v == d.mul_dest {
                    uses += 1;
                }
            });
            if uses == 0 {
                continue;
            }
            count += uses;
            let Instruction::Cmp {
                op,
                lhs,
                rhs,
                ty: IrType::U64,
                ..
            } = i
            else {
                return false;
            };
            if !lp.body.contains(&bi)
                || !matches!(
                    op,
                    IrCmpOp::Ult | IrCmpOp::Ule | IrCmpOp::Ugt | IrCmpOp::Uge
                )
            {
                return false;
            }
            let other = if *lhs == Operand::Value(d.mul_dest) {
                rhs
            } else {
                lhs
            };
            if let Operand::Value(v) = other
                && !is_loop_invariant(v.0, &lp.body, f)
            {
                return false;
            }
        }
        let mut escape = false;
        crate::backend::liveness::for_each_operand_in_terminator(&b.terminator, |o| {
            if *o == Operand::Value(d.mul_dest) {
                escape = true;
            }
        });
        if escape {
            return false;
        }
    }
    count == 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::AddressSpace;
    use crate::ir::reexports::*;
    fn fixture() -> (IrFunction, NaturalLoop, BasicIV, DerivedExpr) {
        let v = |x| Operand::Value(Value(x));
        let c = |x| Operand::Const(IrConst::I64(x));
        let mut f = IrFunction::new("compare_iv".into(), IrType::U64, vec![], false);
        let pre = vec![
            Instruction::ParamRef {
                dest: Value(0),
                param_idx: 0,
                ty: IrType::U64,
            },
            Instruction::GlobalAddr {
                dest: Value(20),
                name: "data".into(),
            },
        ];
        let header = vec![
            Instruction::Phi {
                dest: Value(1),
                ty: IrType::U64,
                incoming: vec![(v(0), BlockId(0)), (v(3), BlockId(4))],
            },
            Instruction::Load {
                dest: Value(2),
                ptr: Value(20),
                ty: IrType::U64,
                volatile: false,
                seg_override: AddressSpace::Default,
            },
        ];
        let body = vec![
            Instruction::BinOp {
                dest: Value(3),
                op: IrBinOp::Add,
                lhs: v(1),
                rhs: c(1),
                ty: IrType::U64,
            },
            Instruction::BinOp {
                dest: Value(4),
                op: IrBinOp::Shl,
                lhs: v(3),
                rhs: c(6),
                ty: IrType::U64,
            },
            Instruction::Cmp {
                dest: Value(5),
                op: IrCmpOp::Uge,
                lhs: v(4),
                rhs: c(1024),
                ty: IrType::U64,
            },
        ];
        for (label, instructions, terminator) in [
            (0, pre, Terminator::Branch(BlockId(1))),
            (
                1,
                header,
                Terminator::CondBranch {
                    cond: v(2),
                    true_label: BlockId(3),
                    false_label: BlockId(2),
                },
            ),
            (
                2,
                body,
                Terminator::CondBranch {
                    cond: v(5),
                    true_label: BlockId(3),
                    false_label: BlockId(4),
                },
            ),
            (3, vec![], Terminator::Return(Some(v(1)))),
            (4, vec![], Terminator::Branch(BlockId(1))),
        ] {
            f.blocks.push(BasicBlock {
                label: BlockId(label),
                instructions,
                terminator,
                source_spans: vec![],
            });
        }
        f.next_value_id = 21;
        f.next_label = 5;
        let lp = NaturalLoop {
            header: 1,
            body: [1, 2, 4].into_iter().collect(),
        };
        let iv = BasicIV {
            phi_dest: Value(1),
            ty: IrType::U64,
            init: v(0),
            step: 1,
        };
        let d = DerivedExpr {
            stride: 64,
            iv_index: 0,
            add_offset: 1,
            gep_uses: vec![],
            vec_uses: vec![],
            mul_dest: Value(4),
            mul_ty: IrType::U64,
            has_uses: true,
            from_address_add: false,
        };
        (f, lp, iv, d)
    }
    #[test]
    fn production_rule_builds_runtime_unsigned_affine_init() {
        let (mut f, lp, iv, d) = fixture();
        assert!(eligible(&f, &lp, &iv, &d));
        set_enabled(true);
        let n = super::super::ivsr_function(&mut f);
        set_enabled(false);
        assert!(n > 0);
        assert_eq!(
            f.blocks[1]
                .instructions
                .iter()
                .filter(|i| matches!(i, Instruction::Phi { .. }))
                .count(),
            2
        );
        assert!(f.blocks[0].instructions.iter().any(|i| matches!(
            i,
            Instruction::BinOp {
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(0)),
                rhs: Operand::Const(IrConst::I64(1)),
                ty: IrType::U64,
                ..
            }
        )));
    }
    #[test]
    fn declines_signed_narrow_small_stride_and_non_affine_costs() {
        let (f, lp, mut iv, mut d) = fixture();
        iv.ty = IrType::I64;
        assert!(!eligible(&f, &lp, &iv, &d));
        iv.ty = IrType::U32;
        assert!(!eligible(&f, &lp, &iv, &d));
        iv.ty = IrType::U64;
        for stride in [1, 2, 4, 8, 12, -64] {
            d.stride = stride;
            assert!(!eligible(&f, &lp, &iv, &d));
        }
        d.stride = 64;
        d.add_offset = 0;
        assert!(!eligible(&f, &lp, &iv, &d));
        d.add_offset = 1;
        iv.step = 2;
        assert!(!eligible(&f, &lp, &iv, &d));
    }
    #[test]
    fn declines_volatile_calls_escapes_and_varying_limits() {
        let (mut f, lp, iv, d) = fixture();
        if let Instruction::Load { volatile, .. } = &mut f.blocks[1].instructions[1] {
            *volatile = true;
        }
        assert!(!eligible(&f, &lp, &iv, &d));
        let (mut f, lp, iv, d) = fixture();
        f.blocks[4].instructions.push(Instruction::Call {
            func: "opaque".into(),
            info: CallInfo::default(),
        });
        assert!(!eligible(&f, &lp, &iv, &d));
        let (mut f, lp, iv, d) = fixture();
        f.blocks[3].terminator = Terminator::Return(Some(Operand::Value(Value(4))));
        assert!(!eligible(&f, &lp, &iv, &d));
        let (mut f, lp, iv, d) = fixture();
        if let Instruction::Cmp { rhs, .. } = &mut f.blocks[2].instructions[2] {
            *rhs = Operand::Value(Value(1));
        }
        assert!(!eligible(&f, &lp, &iv, &d));
        let (mut f, lp, iv, d) = fixture();
        f.blocks[4].instructions.push(Instruction::Copy {
            dest: Value(21),
            src: Operand::Value(Value(4)),
        });
        assert!(!eligible(&f, &lp, &iv, &d));
    }
    #[test]
    fn full_width_affine_recurrence_wraps_exactly() {
        for shift in 4..=20 {
            let stride = 1u64 << shift;
            for seed in 0..1024u64 {
                let start = seed.wrapping_mul(0x9e3779b97f4a7c15);
                let mut j = start.wrapping_add(1).wrapping_mul(stride);
                for t in 0..32u64 {
                    assert_eq!(
                        j,
                        start.wrapping_add(t).wrapping_add(1).wrapping_mul(stride)
                    );
                    j = j.wrapping_add(stride);
                }
            }
        }
    }
}
