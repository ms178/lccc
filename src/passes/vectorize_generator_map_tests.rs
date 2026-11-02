//! Adversarial legality tests: rejection is required to be mutation-free.
use super::*;

fn generator(start: u64, limit: u64, ty: IrType) -> IrFunction {
    let mut f = IrFunction::new("generator".into(), IrType::Void, vec![], false);
    f.blocks = vec![
        BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(0),
                    name: "dst".into(),
                },
                Instruction::GlobalAddr {
                    dest: Value(20),
                    name: "tick".into(),
                },
            ],
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
                        (iv_const(ty, start), BlockId(0)),
                        (Operand::Value(Value(7)), BlockId(2)),
                    ],
                },
                Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Ult,
                    lhs: Operand::Value(Value(1)),
                    rhs: iv_const(ty, limit),
                    ty,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(Value(2)),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
            source_spans: vec![],
        },
        BasicBlock {
            label: BlockId(2),
            instructions: vec![
                Instruction::BinOp {
                    dest: Value(3),
                    op: IrBinOp::Mul,
                    lhs: Operand::Value(Value(1)),
                    rhs: iv_const(ty, 3),
                    ty,
                },
                Instruction::Cast {
                    dest: Value(4),
                    src: Operand::Value(Value(3)),
                    from_ty: ty,
                    to_ty: IrType::U8,
                },
                Instruction::Cast {
                    dest: Value(5),
                    src: Operand::Value(Value(1)),
                    from_ty: ty,
                    to_ty: IrType::I64,
                },
                Instruction::GetElementPtr {
                    dest: Value(6),
                    base: Value(0),
                    offset: Operand::Value(Value(5)),
                    ty: IrType::Ptr,
                },
                Instruction::Store {
                    val: Operand::Value(Value(4)),
                    ptr: Value(6),
                    ty: IrType::U8,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                Instruction::BinOp {
                    dest: Value(7),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: iv_const(ty, 1),
                    ty,
                },
            ],
            terminator: Terminator::Branch(BlockId(1)),
            source_spans: vec![],
        },
        BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: vec![],
        },
    ];
    f.next_value_id = 32;
    f.next_label = 4;
    f
}

fn extra(f: &mut IrFunction, inst: Instruction) {
    let body = &mut f.blocks[2].instructions;
    body.insert(body.len() - 1, inst);
}

fn refused(mut f: IrFunction) {
    let before = format!("{f:?}");
    assert_eq!(transform_function(&mut f, false), 0);
    assert_eq!(
        format!("{f:?}"),
        before,
        "refusal must preserve every field"
    );
}

fn store(val: Operand, volatile: bool, space: AddressSpace) -> Instruction {
    Instruction::Store {
        val,
        ptr: Value(20),
        ty: IrType::U32,
        seg_override: space,
        volatile,
    }
}

#[test]
fn accepts_widths_starts_groups_and_tails_with_valid_ssa() {
    for ty in [IrType::U32, IrType::U64] {
        for (start, limit) in [(0, 32), (5, 37), (5, 68), (0, 4096)] {
            let mut f = generator(start, limit, ty);
            assert_eq!(transform_function(&mut f, false), 1);
            let mut violations = vec![];
            crate::passes::verify::verify_function(&f, "generator-test", &mut violations);
            assert!(violations.is_empty(), "{violations:?}");
            // A second pass must not re-vectorize its own scalar remainder.
            let after = format!("{f:?}");
            assert_eq!(transform_function(&mut f, false), 0);
            assert_eq!(format!("{f:?}"), after);
        }
    }
}

#[test]
fn accepts_cse_shared_address_cast_and_uses_pure_intermediate_packs() {
    let mut f = generator(0, 777, IrType::U64);
    let body = &mut f.blocks[2].instructions;
    let address_cast = body.remove(2);
    body.insert(0, address_cast);
    if let Instruction::BinOp { lhs, ty, .. } = &mut body[1] {
        *lhs = Operand::Value(Value(5));
        *ty = IrType::I64;
    }
    if let Instruction::Cast { from_ty, to_ty, .. } = &mut body[2] {
        *from_ty = IrType::I64;
        *to_ty = IrType::I8;
    }
    if let Instruction::Store { ty, .. } = &mut body[4] {
        *ty = IrType::I8;
    }
    assert_eq!(transform_function(&mut f, false), 1);
    let mut violations = vec![];
    crate::passes::verify::verify_function(&f, "shared-address-cast", &mut violations);
    assert!(violations.is_empty(), "{violations:?}");
    let instructions: Vec<_> = f.blocks.iter().flat_map(|b| &b.instructions).collect();
    assert!(
        !instructions
            .iter()
            .any(|i| matches!(i, Instruction::Alloca { .. }))
    );
    assert_eq!(
        instructions
            .iter()
            .filter(|i| matches!(
                i,
                Instruction::Intrinsic {
                    op: IntrinsicOp::VecPackssdwI16x16,
                    dest: Some(_),
                    dest_ptr: None,
                    ..
                }
            ))
            .count(),
        2
    );
}

#[test]
fn rejects_additional_constant_store() {
    let mut f = generator(0, 32, IrType::U32);
    extra(
        &mut f,
        store(
            Operand::Const(IrConst::I32(1234)),
            false,
            AddressSpace::Default,
        ),
    );
    refused(f);
}

#[test]
fn rejects_additional_volatile_store_without_volatile_alloca() {
    let mut f = generator(0, 32, IrType::U32);
    extra(
        &mut f,
        store(Operand::Value(Value(1)), true, AddressSpace::Default),
    );
    refused(f);
}

#[test]
fn rejects_additional_value_store() {
    let mut f = generator(0, 32, IrType::U32);
    extra(
        &mut f,
        store(Operand::Value(Value(1)), false, AddressSpace::Default),
    );
    refused(f);
}

#[test]
fn rejects_volatile_primary_store() {
    let mut f = generator(0, 32, IrType::U32);
    if let Instruction::Store { volatile, .. } = &mut f.blocks[2].instructions[4] {
        *volatile = true;
    }
    refused(f);
}

#[test]
fn rejects_fs_and_gs_primary_stores() {
    for space in [AddressSpace::SegFs, AddressSpace::SegGs] {
        let mut f = generator(0, 32, IrType::U32);
        if let Instruction::Store { seg_override, .. } = &mut f.blocks[2].instructions[4] {
            *seg_override = space;
        }
        refused(f);
    }
}

#[test]
fn rejects_unmatched_gep_even_with_constant_offset() {
    let mut f = generator(0, 32, IrType::U32);
    extra(
        &mut f,
        Instruction::GetElementPtr {
            dest: Value(21),
            base: Value(0),
            offset: Operand::Const(IrConst::I64(1)),
            ty: IrType::Ptr,
        },
    );
    refused(f);
}

#[test]
fn rejects_load_call_and_memory_copy() {
    let operations = [
        Instruction::Load {
            dest: Value(21),
            ptr: Value(20),
            ty: IrType::U32,
            seg_override: AddressSpace::Default,
            volatile: false,
        },
        Instruction::Call {
            func: "effect".into(),
            info: Default::default(),
        },
        Instruction::Memcpy {
            dest: Value(20),
            src: Value(0),
            size: 1,
        },
    ];
    for inst in operations {
        let mut f = generator(0, 32, IrType::U32);
        extra(&mut f, inst);
        refused(f);
    }
}

#[test]
fn rejects_escaping_expression_address_and_byte_value() {
    for id in [3, 4, 5, 6] {
        let mut f = generator(0, 32, IrType::U32);
        f.blocks[3].instructions.push(Instruction::Copy {
            dest: Value(21),
            src: Operand::Value(Value(id)),
        });
        refused(f);
    }
}

#[test]
fn rejects_duplicate_expression_address_and_iv_definitions() {
    for id in [1, 3, 4, 5, 6, 7] {
        let mut f = generator(0, 32, IrType::U32);
        f.blocks[0].instructions.push(Instruction::Copy {
            dest: Value(id),
            src: Operand::Const(IrConst::I32(0)),
        });
        refused(f);
    }
}

#[test]
fn rejects_short_empty_and_reversed_ranges() {
    for (start, end) in [(0, 0), (0, 31), (32, 0), (5, 36)] {
        refused(generator(start, end, IrType::U32));
    }
}

#[test]
fn rejects_unsupported_arithmetic_and_shift_counts() {
    for (op, count, ty) in [
        (IrBinOp::LShr, 1, IrType::U64),
        (IrBinOp::Shl, 32, IrType::U32),
        (IrBinOp::LShr, -1, IrType::U32),
        (IrBinOp::AShr, 1, IrType::U32),
        (IrBinOp::UDiv, 3, IrType::U32),
    ] {
        let mut f = generator(0, 32, ty);
        f.blocks[2].instructions[0] = Instruction::BinOp {
            dest: Value(3),
            op,
            lhs: Operand::Value(Value(1)),
            rhs: Operand::Const(IrConst::I64(count)),
            ty,
        };
        refused(f);
    }
}

#[test]
fn accepts_exact_32_bit_shifts() {
    for op in [IrBinOp::Shl, IrBinOp::LShr] {
        for count in [0, 1, 31] {
            let mut f = generator(0, 32, IrType::U32);
            f.blocks[2].instructions[0] = Instruction::BinOp {
                dest: Value(3),
                op,
                lhs: Operand::Value(Value(1)),
                rhs: Operand::Const(IrConst::I32(count)),
                ty: IrType::U32,
            };
            assert_eq!(transform_function(&mut f, false), 1);
        }
    }
}

#[test]
fn rejects_inexact_wide_bound_and_shift_constants() {
    let huge = IrConst::I128((1i128 << 100) + 1);
    let mut f = generator(0, 32, IrType::U64);
    if let Instruction::Cmp { rhs, .. } = &mut f.blocks[1].instructions[1] {
        *rhs = Operand::Const(huge);
    }
    refused(f);
    let mut f = generator(0, 32, IrType::U32);
    if let Instruction::BinOp { op, rhs, .. } = &mut f.blocks[2].instructions[0] {
        *op = IrBinOp::Shl;
        *rhs = Operand::Const(huge);
    }
    refused(f);
}

#[test]
fn rejects_unsupported_casts() {
    for ty in [IrType::U16, IrType::F32, IrType::Ptr] {
        let mut f = generator(0, 32, IrType::U32);
        f.blocks[2].instructions[0] = Instruction::Cast {
            dest: Value(3),
            src: Operand::Value(Value(1)),
            from_ty: IrType::U32,
            to_ty: ty,
        };
        refused(f);
    }
}

#[test]
fn rejects_conditional_preheader_and_extra_header_instructions() {
    let mut f = generator(0, 32, IrType::U32);
    f.blocks[0].terminator = Terminator::CondBranch {
        cond: Operand::Const(IrConst::I32(1)),
        true_label: BlockId(1),
        false_label: BlockId(3),
    };
    refused(f);
    let mut f = generator(0, 32, IrType::U32);
    f.blocks[1].instructions.push(Instruction::Copy {
        dest: Value(21),
        src: Operand::Const(IrConst::I32(0)),
    });
    refused(f);
}

#[test]
fn accounts_for_switch_and_computed_goto_body_entries() {
    for term in [
        Terminator::Switch {
            val: Operand::Const(IrConst::I32(0)),
            cases: vec![(0, BlockId(2))],
            default: BlockId(3),
            ty: IrType::I32,
        },
        Terminator::IndirectBranch {
            target: Operand::Value(Value(20)),
            possible_targets: vec![BlockId(2)],
        },
    ] {
        let mut f = generator(0, 32, IrType::U32);
        f.blocks[3].terminator = term;
        refused(f);
    }
}

#[test]
fn handles_stale_value_and_label_cursors_without_collisions() {
    let mut f = generator(0, 32, IrType::U32);
    f.next_label = 0;
    f.next_value_id = 0;
    assert_eq!(transform_function(&mut f, false), 1);
    assert_eq!(f.blocks[4].label, BlockId(4));
    assert!(f.next_value_id > 32);
}

#[test]
fn refuses_value_and_label_identifier_exhaustion_atomically() {
    let mut f = generator(0, 32, IrType::U32);
    f.next_value_id = u32::MAX;
    refused(f);
    let mut f = generator(0, 32, IrType::U32);
    f.next_label = u32::MAX;
    refused(f);
}

#[test]
fn avx2_pack_layout_preserves_all_32_element_positions() {
    fn pack(a: &[u32], b: &[u32]) -> Vec<u32> {
        let half = a.len() / 2;
        [
            a[..half].to_vec(),
            b[..half].to_vec(),
            a[half..].to_vec(),
            b[half..].to_vec(),
        ]
        .concat()
    }
    let packed = pack(&pack(&OFF[0], &OFF[1]), &pack(&OFF[2], &OFF[3]));
    assert_eq!(packed, (0..32).collect::<Vec<_>>());
}
