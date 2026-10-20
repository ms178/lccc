use super::*;
use crate::common::fx_hash::FxHashMap;
use crate::common::types::AddressSpace;

fn fixture(op: IrBinOp, ty: IrType, iv_ty: IrType, n: u64, seed: u64) -> IrFunction {
    let mut f = IrFunction::new("volatile_reduction".into(), ty, vec![], false);
    let cv = |v, t| Operand::Const(IrConst::from_i64(v as i64, t));
    f.blocks = vec![
        BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::GlobalAddr {
                dest: Value(0),
                name: "device".into(),
            }],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
        },
        BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::Phi {
                    dest: Value(1),
                    ty,
                    incoming: vec![
                        (cv(seed, ty), BlockId(0)),
                        (Operand::Value(Value(5)), BlockId(2)),
                    ],
                },
                Instruction::Phi {
                    dest: Value(2),
                    ty: iv_ty,
                    incoming: vec![
                        (cv(0, iv_ty), BlockId(0)),
                        (Operand::Value(Value(6)), BlockId(2)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(3),
                    op: IrCmpOp::Ult,
                    lhs: Operand::Value(Value(2)),
                    rhs: cv(n, iv_ty),
                    ty: iv_ty,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(3)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
            source_spans: vec![],
        },
        BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::Load {
                    dest: Value(4),
                    ptr: Value(0),
                    ty,
                    seg_override: AddressSpace::SegFs,
                    volatile: true,
                },
                Instruction::BinOp {
                    dest: Value(5),
                    op,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Value(Value(4)),
                    ty,
                },
                Instruction::BinOp {
                    dest: Value(6),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(2)),
                    rhs: cv(1, iv_ty),
                    ty: iv_ty,
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
        },
        BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(Some(Operand::Value(Value(1)))),
            source_spans: vec![],
        },
    ];
    f.next_value_id = 7;
    f.next_label = 4;
    f
}

fn mask(ty: IrType) -> u64 {
    if ty == IrType::U32 {
        u32::MAX as u64
    } else {
        u64::MAX
    }
}
fn operand(op: Operand, values: &FxHashMap<u32, u64>) -> u64 {
    match op {
        Operand::Value(v) => values[&v.0],
        Operand::Const(c) => c.to_i64().unwrap() as u64,
    }
}

// Independent small SSA interpreter: parallel phi assignment, modular arithmetic,
// and a changing read stream. A trace compares every access, not just the result
// of repeated reads from an accidentally constant fixture.
type Access = (u64, IrType, AddressSpace, bool);

fn interpret(f: &IrFunction) -> (u64, u64, Vec<Access>) {
    let mut values = FxHashMap::default();
    let mut label = BlockId(0);
    let mut previous = BlockId(u32::MAX);
    let mut trace = vec![];
    for _ in 0..100000 {
        let b = f.blocks.iter().find(|b| b.label == label).unwrap();
        let phis: Vec<_> = b
            .instructions
            .iter()
            .filter_map(|i| {
                if let Instruction::Phi { dest, incoming, ty } = i {
                    Some((
                        dest.0,
                        operand(
                            incoming.iter().find(|(_, l)| *l == previous).unwrap().0,
                            &values,
                        ) & mask(*ty),
                    ))
                } else {
                    None
                }
            })
            .collect();
        values.extend(phis);
        for i in &b.instructions {
            let result = match i {
                Instruction::Phi { .. } => continue,
                Instruction::GlobalAddr { dest, .. } => (*dest, 0x1000),
                Instruction::GetElementPtr {
                    dest, base, offset, ..
                } => (
                    *dest,
                    values[&base.0].wrapping_add(operand(*offset, &values)),
                ),
                Instruction::Load {
                    dest,
                    ptr,
                    ty,
                    seg_override,
                    volatile,
                } => {
                    trace.push((values[&ptr.0], *ty, *seg_override, *volatile));
                    let x = (trace.len() as u64).wrapping_mul(0x9e3779b97f4a7c15) | 1;
                    (*dest, x & mask(*ty))
                }
                Instruction::BinOp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => {
                    let (a, b) = (operand(*lhs, &values), operand(*rhs, &values));
                    let v = match op {
                        IrBinOp::Add => a.wrapping_add(b),
                        IrBinOp::Mul => a.wrapping_mul(b),
                        IrBinOp::And => a & b,
                        IrBinOp::Or => a | b,
                        IrBinOp::Xor => a ^ b,
                        _ => panic!("{i:?}"),
                    };
                    (*dest, v & mask(*ty))
                }
                Instruction::Cmp {
                    dest,
                    op: IrCmpOp::Ult,
                    lhs,
                    rhs,
                    ty,
                } => (
                    *dest,
                    ((operand(*lhs, &values) & mask(*ty)) < (operand(*rhs, &values) & mask(*ty)))
                        as u64,
                ),
                _ => panic!("unexpected instruction: {i:?}"),
            };
            values.insert(result.0.0, result.1);
        }
        previous = label;
        label = match b.terminator {
            Terminator::Branch(l) => l,
            Terminator::CondBranch {
                cond,
                true_label,
                false_label,
            } => {
                if operand(cond, &values) != 0 {
                    true_label
                } else {
                    false_label
                }
            }
            Terminator::Return(Some(op)) => return (operand(op, &values), values[&2], trace),
            _ => panic!("unexpected terminator"),
        };
    }
    panic!("interpreter step limit");
}

#[test]
fn all_operations_widths_seeds_and_tails_match_changing_read_stream() {
    for ty in [IrType::U32, IrType::U64] {
        for iv_ty in [IrType::U32, IrType::U64] {
            for op in [
                IrBinOp::Add,
                IrBinOp::Mul,
                IrBinOp::And,
                IrBinOp::Or,
                IrBinOp::Xor,
            ] {
                for n in (0..=33).chain([63, 64, 65, 1023]) {
                    for seed in [0, 1, 0xdeadbeef01234567, u64::MAX] {
                        let mut f = fixture(op, ty, iv_ty, n, seed);
                        let expected = interpret(&f);
                        assert_eq!(run(&mut f), 1);
                        let mut violations = vec![];
                        crate::passes::verify::verify_function(
                            &f,
                            "volatile_reduce",
                            &mut violations,
                        );
                        assert!(violations.is_empty(), "{violations:?}");
                        assert_eq!(
                            interpret(&f),
                            expected,
                            "{op:?}/{ty:?}/{iv_ty:?}/{n}/{seed}"
                        );
                        let before = format!("{f:?}");
                        assert_eq!(run(&mut f), 0);
                        assert_eq!(format!("{f:?}"), before);
                    }
                }
            }
        }
    }
}

#[test]
fn rounding_at_unsigned_extrema_cannot_overflow_or_overread() {
    for max in [u32::MAX as u64, u64::MAX] {
        for n in [0, 1, 2, 3, 4, 5, 7, 8, max - 3, max - 2, max - 1, max] {
            let bound = n & !3;
            assert!(bound <= n);
            assert!(n - bound < LANES as u64);
            if bound != 0 {
                let last = bound as u128 - LANES as u128;
                assert!(last + LANES as u128 <= max as u128);
                assert!(last + LANES as u128 - 1 < n as u128);
            }
        }
    }
}

fn assert_rejected(f: &mut IrFunction) {
    let before = format!("{f:?}");
    assert_eq!(run(f), 0);
    assert_eq!(format!("{f:?}"), before);
}

#[test]
fn unsupported_operations_and_semantics_remain_unchanged() {
    for op in [IrBinOp::Sub, IrBinOp::UDiv, IrBinOp::Shl, IrBinOp::LShr] {
        assert_rejected(&mut fixture(op, IrType::U64, IrType::U64, 31, 3));
    }
    for ty in [
        IrType::I32,
        IrType::I64,
        IrType::F32,
        IrType::F64,
        IrType::U16,
        IrType::U128,
    ] {
        assert_rejected(&mut fixture(IrBinOp::Add, ty, IrType::U64, 31, 3));
    }
    for iv in [IrType::I32, IrType::I64] {
        assert_rejected(&mut fixture(IrBinOp::Add, IrType::U64, iv, 31, 3));
    }
    let base = fixture(IrBinOp::Add, IrType::U64, IrType::U64, 31, 3);
    let mut f = base.clone();
    if let Instruction::Load { volatile, .. } = &mut f.blocks[2].instructions[0] {
        *volatile = false;
    }
    assert_rejected(&mut f);
    let mut f = base.clone();
    if let Instruction::Load { ptr, .. } = &mut f.blocks[2].instructions[0] {
        *ptr = Value(1);
    }
    assert_rejected(&mut f);
    let mut f = base.clone();
    if let Instruction::Cmp { rhs, .. } = &mut f.blocks[1].instructions[2] {
        *rhs = Operand::Value(Value(1));
    }
    assert_rejected(&mut f);
    let mut f = base.clone();
    if let Instruction::Phi { incoming, .. } = &mut f.blocks[1].instructions[1] {
        incoming[0].0 = Operand::Const(IrConst::I64(1));
    }
    assert_rejected(&mut f);
    let mut f = base.clone();
    if let Instruction::BinOp { rhs, .. } = &mut f.blocks[2].instructions[2] {
        *rhs = Operand::Const(IrConst::I64(2));
    }
    assert_rejected(&mut f);
    let mut f = base.clone();
    f.blocks[2].instructions.push(Instruction::Store {
        val: Operand::Value(Value(1)),
        ptr: Value(0),
        ty: IrType::U64,
        seg_override: AddressSpace::Default,
        volatile: true,
    });
    assert_rejected(&mut f);
    let mut f = base.clone();
    f.blocks[3].terminator = Terminator::Return(Some(Operand::Value(Value(4))));
    assert_rejected(&mut f);
    let mut f = base.clone();
    if let Terminator::CondBranch {
        true_label,
        false_label,
        ..
    } = &mut f.blocks[1].terminator
    {
        std::mem::swap(true_label, false_label);
    }
    assert_rejected(&mut f);
    let mut f = base.clone();
    f.next_value_id = u32::MAX - 16;
    assert_rejected(&mut f);
    let mut f = base;
    f.next_label = u32::MAX - 1;
    assert_rejected(&mut f);
}

#[test]
fn commuted_operands_and_stale_caches_are_safe() {
    let mut f = fixture(IrBinOp::Mul, IrType::U64, IrType::U32, 65, 7);
    if let Instruction::BinOp { lhs, rhs, .. } = &mut f.blocks[2].instructions[1] {
        std::mem::swap(lhs, rhs);
    }
    f.next_value_id = 1;
    f.next_label = 1;
    let expected = interpret(&f);
    assert_eq!(run(&mut f), 1);
    assert_eq!(interpret(&f), expected);
}

#[test]
fn constant_address_offsets_and_all_segments_preserve_access_trace() {
    for segment in [
        AddressSpace::Default,
        AddressSpace::SegFs,
        AddressSpace::SegGs,
    ] {
        for offset in [-16, 0, 8] {
            for n in 0..=9 {
                let mut f = fixture(IrBinOp::Add, IrType::U64, IrType::U64, n, 7);
                if let Instruction::Load {
                    ptr, seg_override, ..
                } = &mut f.blocks[2].instructions[0]
                {
                    *ptr = Value(7);
                    *seg_override = segment;
                }
                f.blocks[2].instructions.insert(
                    0,
                    Instruction::GetElementPtr {
                        dest: Value(7),
                        base: Value(0),
                        offset: Operand::Const(IrConst::I64(offset)),
                        ty: IrType::Ptr,
                    },
                );
                f.next_value_id = 8;
                let expected = interpret(&f);
                assert_eq!(run(&mut f), 1);
                assert_eq!(interpret(&f), expected);
                let mut violations = vec![];
                crate::passes::verify::verify_function(&f, "volatile_reduce", &mut violations);
                assert!(violations.is_empty(), "{violations:?}");
            }
        }
    }
}
