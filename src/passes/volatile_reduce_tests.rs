use super::*;
use crate::common::fx_hash::FxHashMap;
use crate::common::types::AddressSpace;

fn run(f: &mut IrFunction) -> usize {
    super::run(f, &FxHashSet::default())
}

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
    interpret_from(f, BlockId(0), BlockId(u32::MAX), FxHashMap::default())
}

fn interpret_from(
    f: &IrFunction,
    mut label: BlockId,
    mut previous: BlockId,
    mut values: FxHashMap<u32, u64>,
) -> (u64, u64, Vec<Access>) {
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
                    op,
                    lhs,
                    rhs,
                    ty,
                } => (
                    *dest,
                    match op {
                        IrCmpOp::Ult => {
                            ((operand(*lhs, &values) & mask(*ty))
                                < (operand(*rhs, &values) & mask(*ty)))
                                as u64
                        }
                        IrCmpOp::Eq => {
                            ((operand(*lhs, &values) & mask(*ty))
                                == (operand(*rhs, &values) & mask(*ty)))
                                as u64
                        }
                        _ => panic!("unexpected comparison: {op:?}"),
                    },
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

fn verify(f: &IrFunction) {
    let mut violations = vec![];
    crate::passes::verify::verify_function(f, "volatile_reduce", &mut violations);
    assert!(violations.is_empty(), "{violations:?}");
}

#[test]
fn source_locations_survive_rewrite() {
    for with_gep in [false, true] {
        let mut f = fixture(IrBinOp::Mul, IrType::U64, IrType::U64, 17, 7);
        if with_gep {
            f.blocks[2].instructions.insert(
                0,
                Instruction::GetElementPtr {
                    dest: Value(7),
                    base: Value(0),
                    offset: Operand::Const(IrConst::I64(8)),
                    ty: IrType::Ptr,
                },
            );
            if let Instruction::Load { ptr, .. } = &mut f.blocks[2].instructions[1] {
                *ptr = Value(7);
            }
            f.next_value_id = 8;
        }
        for (bi, block) in f.blocks.iter_mut().enumerate() {
            block.source_spans = (0..block.instructions.len())
                .map(|i| Span::new((bi * 100 + i) as u32, (bi * 100 + i + 1) as u32, 7))
                .collect();
        }
        let old = f.clone();
        assert_eq!(run(&mut f), 1);
        assert_eq!(f.blocks[0].source_spans[0], old.blocks[0].source_spans[0]);
        assert_eq!(f.blocks[0].source_spans[1], old.blocks[1].source_spans[2]);
        assert_eq!(f.blocks[1].source_spans, old.blocks[1].source_spans);
        assert_eq!(f.blocks[2].source_spans, old.blocks[2].source_spans);
        for block in &f.blocks {
            assert_eq!(block.source_spans.len(), block.instructions.len());
        }
        let group = &f.blocks[5];
        let stride = if with_gep { 3 } else { 2 };
        for lane in 0..LANES {
            assert_eq!(
                &group.source_spans[lane * stride..(lane + 1) * stride],
                &old.blocks[2].source_spans[..stride]
            );
        }
        assert_eq!(
            group.source_spans[LANES * stride],
            *old.blocks[2].source_spans.last().unwrap()
        );
        verify(&f);
    }
}

#[test]
fn production_bounds_and_terminal_iterations_at_high_bits() {
    for iv_ty in [IrType::U32, IrType::U64] {
        let max = mask(iv_ty);
        for n in [
            1_u64 << 31,
            (1_u64 << 31) + 3,
            1_u64 << 32,
            (1_u64 << 32) + 3,
            1_u64 << 63,
            (1_u64 << 63) + 3,
            max - 3,
            max - 2,
            max - 1,
            max,
        ] {
            if n > max {
                continue;
            }
            for op in [
                IrBinOp::Add,
                IrBinOp::Mul,
                IrBinOp::And,
                IrBinOp::Or,
                IrBinOp::Xor,
            ] {
                for ty in [IrType::U32, IrType::U64] {
                    let mut f = fixture(op, ty, iv_ty, n, 7);
                    let original = f.clone();
                    assert_eq!(run(&mut f), 1);
                    verify(&f);
                    let Instruction::BinOp {
                        dest: bound,
                        op: IrBinOp::And,
                        lhs,
                        rhs,
                        ty: bound_ty,
                    } = *f.blocks[0]
                        .instructions
                        .iter()
                        .find(|i| {
                            matches!(
                                i,
                                Instruction::BinOp {
                                    op: IrBinOp::And,
                                    ..
                                }
                            )
                        })
                        .unwrap()
                    else {
                        panic!("missing production bound");
                    };
                    assert_eq!(bound_ty, iv_ty);
                    let values = FxHashMap::default();
                    let m = (operand(lhs, &values) & operand(rhs, &values)) & mask(bound_ty);
                    assert_eq!(m, n - n % 4);
                    let start = m - 4;
                    // Enter immediately before the final group. Both executions
                    // receive the same accumulator/IV state, not billions of steps.
                    let mut old_values = FxHashMap::default();
                    old_values.extend([(0, 0x1000), (5, 7), (6, start)]);
                    let expected = interpret_from(&original, BlockId(1), BlockId(2), old_values);
                    let mut new_values = FxHashMap::default();
                    new_values.extend([(0, 0x1000), (bound.0, m)]);
                    for (index, inst) in f.blocks[4].instructions.iter().enumerate() {
                        if let Instruction::Phi {
                            ty: phi_ty,
                            incoming,
                            ..
                        } = inst
                        {
                            let back = incoming
                                .iter()
                                .find(|(_, l)| *l == f.blocks[5].label)
                                .unwrap()
                                .0;
                            let Operand::Value(v) = back else {
                                panic!("non-value backedge");
                            };
                            // IV is the first phi; extra accumulators are neutral.
                            let value = if index == 0 {
                                start
                            } else if index == 1 {
                                7
                            } else {
                                operand(identity(op, *phi_ty).unwrap(), &FxHashMap::default())
                                    & mask(*phi_ty)
                            };
                            new_values.insert(v.0, value);
                        }
                    }
                    let actual =
                        interpret_from(&f, f.blocks[4].label, f.blocks[5].label, new_values);
                    assert_eq!(actual, expected, "{op:?}/{ty:?}/{iv_ty:?}/{n}");
                    assert_eq!(actual.1, n);
                    assert_eq!(actual.2.len() as u64, n - start);
                }
            }
        }
    }
}

#[test]
fn observable_iv_live_out_is_preserved() {
    for n in 0..=17 {
        let mut f = fixture(IrBinOp::Add, IrType::U64, IrType::U64, n, 7);
        f.blocks[3].instructions.push(Instruction::BinOp {
            dest: Value(7),
            op: IrBinOp::Xor,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Value(Value(2)),
            ty: IrType::U64,
        });
        f.blocks[3].terminator = Terminator::Return(Some(Operand::Value(Value(7))));
        f.next_value_id = 8;
        verify(&f);
        let expected = interpret(&f);
        assert_eq!(run(&mut f), 1);
        verify(&f);
        assert_eq!(interpret(&f), expected);
    }
}

#[test]
fn sibling_loops_rebuild_cfg_and_allocate_unique_ids() {
    use crate::ir::instruction::{replace_values_in_inst_map, replace_values_in_terminator_map};
    let mut f = fixture(IrBinOp::Add, IrType::U64, IrType::U64, 9, 7);
    let mut sibling = fixture(IrBinOp::Mul, IrType::U64, IrType::U64, 6, 3);
    let map: FxHashMap<_, _> = (0..7).map(|id| (id, id + 16)).collect();
    for block in &mut sibling.blocks {
        block.label.0 += 4;
        for inst in &mut block.instructions {
            replace_values_in_inst_map(inst, &map);
            match inst {
                Instruction::Phi { dest, incoming, .. } => {
                    dest.0 += 16;
                    for (_, l) in incoming {
                        l.0 += 4;
                    }
                }
                Instruction::GlobalAddr { dest, .. }
                | Instruction::Load { dest, .. }
                | Instruction::BinOp { dest, .. }
                | Instruction::Cmp { dest, .. } => dest.0 += 16,
                _ => panic!("unexpected fixture"),
            }
        }
        replace_values_in_terminator_map(&mut block.terminator, &map);
        match &mut block.terminator {
            Terminator::Branch(l) => l.0 += 4,
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => {
                true_label.0 += 4;
                false_label.0 += 4;
            }
            Terminator::Return(_) => (),
            _ => panic!("unexpected fixture"),
        }
    }
    if let Instruction::Phi { incoming, .. } = &mut sibling.blocks[1].instructions[0] {
        incoming[0].0 = Operand::Value(Value(1));
    }
    f.blocks[3].terminator = Terminator::Branch(BlockId(4));
    f.blocks.extend(sibling.blocks);
    f.next_value_id = 23;
    f.next_label = 8;
    verify(&f);
    // Deliberately stale caches must not collide with the sibling's IDs.
    let expected = interpret(&f);
    f.next_value_id = 7;
    f.next_label = 4;
    assert_eq!(run(&mut f), 2);
    verify(&f);
    assert_eq!(interpret(&f), expected);
    assert_eq!(run(&mut f), 0);
}

#[test]
fn inner_loop_in_noneligible_outer_loop() {
    let mut f = fixture(IrBinOp::Xor, IrType::U64, IrType::U64, 9, 7);
    f.blocks[0].terminator = Terminator::Branch(BlockId(4));
    for inst in &mut f.blocks[1].instructions {
        if let Instruction::Phi { incoming, .. } = inst {
            incoming[0].1 = BlockId(6);
        }
    }
    f.blocks[3].instructions.push(Instruction::BinOp {
        dest: Value(9),
        op: IrBinOp::Add,
        lhs: Operand::Value(Value(8)),
        rhs: Operand::Const(IrConst::I64(1)),
        ty: IrType::U64,
    });
    f.blocks[3].terminator = Terminator::Branch(BlockId(4));
    f.blocks.push(BasicBlock {
        label: BlockId(4),
        instructions: vec![
            Instruction::Phi {
                dest: Value(8),
                ty: IrType::U64,
                incoming: vec![
                    (Operand::Const(IrConst::I64(0)), BlockId(0)),
                    (Operand::Value(Value(9)), BlockId(3)),
                ],
            },
            Instruction::Cmp {
                dest: Value(10),
                op: IrCmpOp::Ult,
                lhs: Operand::Value(Value(8)),
                rhs: Operand::Const(IrConst::I64(3)),
                ty: IrType::U64,
            },
        ],
        terminator: Terminator::CondBranch {
            cond: Operand::Value(Value(10)),
            true_label: BlockId(6),
            false_label: BlockId(5),
        },
        source_spans: vec![],
    });
    f.blocks.push(BasicBlock {
        label: BlockId(5),
        instructions: vec![],
        terminator: Terminator::Return(Some(Operand::Value(Value(8)))),
        source_spans: vec![],
    });
    f.blocks.push(BasicBlock {
        label: BlockId(6),
        instructions: vec![],
        terminator: Terminator::Branch(BlockId(1)),
        source_spans: vec![],
    });
    f.next_value_id = 11;
    f.next_label = 7;
    verify(&f);
    let expected = interpret(&f);
    assert_eq!(run(&mut f), 1);
    verify(&f);
    assert_eq!(interpret(&f), expected);
    assert_eq!(run(&mut f), 0);
}

#[test]
fn dynamic_tls_policy_honors_all_overrides() {
    use crate::ir::reexports::TlsModel::*;
    for pic in [false, true] {
        for command in [
            None,
            Some(LocalExec),
            Some(InitialExec),
            Some(LocalDynamic),
            Some(GlobalDynamic),
        ] {
            for declaration in [
                None,
                Some(LocalExec),
                Some(InitialExec),
                Some(LocalDynamic),
                Some(GlobalDynamic),
            ] {
                let expected = match declaration.or(command) {
                    Some(LocalDynamic | GlobalDynamic) => true,
                    Some(LocalExec | InitialExec) => false,
                    None => pic,
                };
                assert_eq!(dynamic_tls(declaration, command, pic), expected);
            }
        }
    }
}

#[test]
fn costly_tls_roots_and_constant_aliases_are_not_interleaved() {
    for aliases in 0..=6 {
        let mut f = fixture(IrBinOp::Mul, IrType::U64, IrType::U64, 17, 7);
        let mut ptr = Value(0);
        for index in 0..aliases {
            let dest = Value(7 + index);
            f.blocks[0].instructions.push(if index % 2 == 0 {
                Instruction::Copy {
                    dest,
                    src: Operand::Value(ptr),
                }
            } else {
                Instruction::GetElementPtr {
                    dest,
                    base: ptr,
                    offset: Operand::Const(IrConst::I64(8)),
                    ty: IrType::Ptr,
                }
            });
            ptr = dest;
        }
        if let Instruction::Load { ptr: p, .. } = &mut f.blocks[2].instructions[0] {
            *p = ptr;
        }
        f.next_value_id = 7 + aliases;
        verify(&f);
        let before = format!("{f:?}");
        let costly = FxHashSet::from_iter(["device".to_string()]);
        assert_eq!(super::run(&mut f, &costly), 0);
        assert_eq!(format!("{f:?}"), before);
        assert_eq!(run(&mut f), 1);
        verify(&f);
    }
}

#[test]
fn exact_fresh_id_budget_is_checked_before_mutation() {
    for address in [false, true] {
        let mut f = fixture(IrBinOp::Add, IrType::U64, IrType::U64, 7, 3);
        if address {
            f.blocks[2].instructions.insert(
                0,
                Instruction::GetElementPtr {
                    dest: Value(7),
                    base: Value(0),
                    offset: Operand::Const(IrConst::I64(8)),
                    ty: IrType::Ptr,
                },
            );
            if let Instruction::Load { ptr, .. } = &mut f.blocks[2].instructions[1] {
                *ptr = Value(7);
            }
        }
        let required = if address { 23 } else { 19 };
        f.next_value_id = u32::MAX - required + 1;
        assert_rejected(&mut f);
        f.next_value_id -= 1;
        assert_eq!(run(&mut f), 1);
        assert_eq!(f.next_value_id, u32::MAX);
        verify(&f);
    }
}

#[test]
fn exact_label_budget_is_checked_before_mutation() {
    let mut f = fixture(IrBinOp::Add, IrType::U64, IrType::U64, 7, 3);
    f.next_label = u32::MAX - 2;
    assert_rejected(&mut f);
    f.next_label -= 1;
    assert_eq!(run(&mut f), 1);
    assert_eq!(f.next_label, u32::MAX);
    verify(&f);
}

#[test]
fn invalid_candidate_variants_fail_before_mutation() {
    for bad_address in [false, true] {
        let mut f = fixture(IrBinOp::Add, IrType::U64, IrType::U64, 7, 3);
        let cfg = CfgAnalysis::build(&f);
        let loops =
            loop_analysis::find_natural_loops(cfg.num_blocks, &cfg.preds, &cfg.succs, &cfg.idom);
        let costly = CostlyAddresses::new(&f, &FxHashSet::default());
        let mut c = analyze(&f, &cfg, &loops[0], &costly).unwrap();
        if bad_address {
            c.address = Some(Instruction::Copy {
                dest: Value(7),
                src: Operand::Value(Value(0)),
            });
        } else {
            c.load = Instruction::GlobalAddr {
                dest: Value(7),
                name: "bad".into(),
            };
        }
        let before = format!("{f:?}");
        assert!(rewrite(&mut f, c).is_none());
        assert_eq!(format!("{f:?}"), before);
    }
}

#[test]
fn upstream_block_budget_rejects_without_mutation() {
    let mut f = fixture(IrBinOp::Mul, IrType::U64, IrType::U64, 9, 7);
    while f.blocks.len() <= MAX_BLOCKS_BUDGET {
        f.blocks.push(BasicBlock {
            label: BlockId(f.blocks.len() as u32),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: vec![],
        });
    }
    f.next_label = f.blocks.len() as u32;
    assert_rejected(&mut f);
}
