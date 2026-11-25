use super::ptr_delta_phi::run;
use crate::common::types::IrType;
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IrBinOp, IrConst, IrFunction, Operand, Terminator, Value,
};

fn assert_valid(f: &IrFunction) {
    let mut violations = Vec::new();
    crate::passes::verify::verify_function(f, "ptr_delta_phi_test", &mut violations);
    assert!(violations.is_empty(), "invalid IR: {violations:#?}");
}

fn value(n: u32) -> Value {
    Value(n)
}
fn v(n: u32) -> Operand {
    Operand::Value(value(n))
}
fn block(id: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
    BasicBlock {
        label: BlockId(id),
        instructions,
        terminator,
        source_spans: Vec::new(),
        source_inline_ids: Vec::new(),
    }
}
fn parameter(dest: u32, ty: IrType) -> Instruction {
    Instruction::ParamRef {
        dest: value(dest),
        param_idx: dest as usize,
        ty,
    }
}
fn gep(dest: u32, base: u32, offset: Operand) -> Instruction {
    Instruction::GetElementPtr {
        dest: value(dest),
        base: value(base),
        offset,
        ty: IrType::I64,
    }
}
fn phi(dest: u32, ty: IrType, incoming: &[(u32, u32)]) -> Instruction {
    Instruction::Phi {
        dest: value(dest),
        ty,
        incoming: incoming
            .iter()
            .map(|&(val, edge)| (v(val), BlockId(edge)))
            .collect(),
    }
}
fn sub(dest: u32, lhs: u32, rhs: u32, ty: IrType) -> Instruction {
    Instruction::BinOp {
        dest: value(dest),
        op: IrBinOp::Sub,
        lhs: v(lhs),
        rhs: v(rhs),
        ty,
    }
}
fn diamond() -> IrFunction {
    let mut f = IrFunction::new("ptr_delta".into(), IrType::I64, vec![], false);
    f.blocks = vec![
        block(
            0,
            vec![
                parameter(0, IrType::Ptr),
                parameter(1, IrType::Ptr),
                parameter(2, IrType::I32),
            ],
            Terminator::CondBranch {
                cond: v(2),
                true_label: BlockId(1),
                false_label: BlockId(2),
            },
        ),
        block(
            1,
            vec![gep(3, 0, Operand::Const(IrConst::I64(8)))],
            Terminator::Branch(BlockId(3)),
        ),
        block(2, vec![], Terminator::Branch(BlockId(3))),
        block(
            3,
            vec![
                phi(4, IrType::Ptr, &[(3, 1), (0, 2)]),
                sub(5, 4, 0, IrType::I64),
            ],
            Terminator::Return(Some(v(5))),
        ),
    ];
    f.next_label = 4;
    f
}

#[test]
fn constant_offset_phi_replaces_pointer_difference_and_drops_dead_pointer_web() {
    let mut f = diamond();
    f.next_value_id = 1; // stale caches must never collide with newly allocated ids
    assert_eq!(run(&mut f), 1);
    let b = &f.blocks[3];
    let integer_phi = b
        .instructions
        .iter()
        .find_map(|inst| {
            if let Instruction::Phi {
                dest,
                ty: IrType::I64,
                incoming,
            } = inst
            {
                Some((*dest, incoming))
            } else {
                None
            }
        })
        .expect("integer offset phi");
    assert_eq!(
        integer_phi.1,
        &vec![
            (Operand::Const(IrConst::I64(8)), BlockId(1)),
            (Operand::Const(IrConst::I64(0)), BlockId(2)),
        ]
    );
    assert!(b.instructions.iter().all(|i| !matches!(
        i,
        Instruction::BinOp {
            op: IrBinOp::Sub,
            ..
        }
    )));
    assert!(
        f.blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .all(|i| !matches!(
                i,
                Instruction::Phi {
                    ty: IrType::Ptr,
                    ..
                }
            ))
    );
    assert!(f.next_value_id > integer_phi.0.0);
    assert_valid(&f);
}

#[test]
fn nested_merge_keeps_pointer_phi_when_used_for_load() {
    // 0 -> 4 or 1; 1 -> 2 or 3; 2,3 -> 5 -> 4.
    let mut f = diamond();
    f.blocks = vec![
        block(
            0,
            vec![
                parameter(0, IrType::Ptr),
                parameter(1, IrType::Ptr),
                parameter(2, IrType::I32),
            ],
            Terminator::CondBranch {
                cond: v(2),
                true_label: BlockId(1),
                false_label: BlockId(4),
            },
        ),
        block(
            1,
            vec![],
            Terminator::CondBranch {
                cond: v(2),
                true_label: BlockId(2),
                false_label: BlockId(3),
            },
        ),
        block(
            2,
            vec![gep(3, 0, Operand::Const(IrConst::I64(8)))],
            Terminator::Branch(BlockId(5)),
        ),
        block(
            3,
            vec![gep(6, 0, Operand::Const(IrConst::I64(4)))],
            Terminator::Branch(BlockId(5)),
        ),
        block(
            5,
            vec![phi(4, IrType::Ptr, &[(3, 2), (6, 3)])],
            Terminator::Branch(BlockId(4)),
        ),
        block(
            4,
            vec![
                phi(7, IrType::Ptr, &[(0, 0), (4, 5)]),
                Instruction::Load {
                    dest: value(8),
                    ptr: value(7),
                    ty: IrType::U8,
                    seg_override: crate::common::types::AddressSpace::Default,
                    volatile: false,
                },
                Instruction::Store {
                    val: v(8),
                    ptr: value(1),
                    ty: IrType::U8,
                    seg_override: crate::common::types::AddressSpace::Default,
                    volatile: true,
                },
                sub(9, 7, 0, IrType::I64),
            ],
            Terminator::Return(Some(v(9))),
        ),
    ];
    f.next_label = 6;
    assert_eq!(run(&mut f), 1);
    assert_eq!(
        f.blocks[5]
            .instructions
            .iter()
            .filter(|i| matches!(
                i,
                Instruction::Phi {
                    ty: IrType::I64,
                    ..
                }
            ))
            .count(),
        1
    );
    assert!(f.blocks[5].instructions.iter().any(|i| matches!(
        i,
        Instruction::Phi {
            ty: IrType::Ptr,
            ..
        }
    )));
    assert_valid(&f);
}

#[test]
fn different_base_and_variable_offset_fail_closed() {
    let mut f = diamond();
    f.blocks[1].instructions[0] = gep(3, 1, Operand::Const(IrConst::I64(8)));
    assert_eq!(run(&mut f), 0);
    assert!(matches!(
        f.blocks[3].instructions[1],
        Instruction::BinOp {
            op: IrBinOp::Sub,
            ..
        }
    ));

    let mut f = diamond();
    f.blocks[1].instructions[0] = gep(3, 0, v(2));
    assert_eq!(run(&mut f), 0);
    assert!(matches!(
        f.blocks[3].instructions[1],
        Instruction::BinOp {
            op: IrBinOp::Sub,
            ..
        }
    ));
}

#[test]
fn i686_out_of_range_offset_and_phi_cycle_fail_closed() {
    let mut f = diamond();
    f.blocks[3].instructions[1] = sub(5, 4, 0, IrType::I32);
    assert_eq!(run(&mut f), 1); // +8 fits the I32 pointer-difference result
    assert!(f.blocks[3].instructions.iter().any(|i| matches!(
        i,
        Instruction::Phi {
            ty: IrType::I32,
            ..
        }
    )));
    let mut f = diamond();
    f.blocks[1].instructions[0] = gep(3, 0, Operand::Const(IrConst::I64(1i64 << 40)));
    f.blocks[3].instructions[1] = sub(5, 4, 0, IrType::I32);
    assert_eq!(run(&mut f), 0);

    let mut f = diamond();
    f.blocks[1].instructions[0] = gep(3, 4, Operand::Const(IrConst::I64(1)));
    assert_eq!(run(&mut f), 0);
}

#[test]
fn repeated_subtractions_share_one_phi_and_keep_debug_metadata_aligned() {
    let mut f = diamond();
    let mut acc = 5;
    let mut next = 6;
    for _ in 1..128 {
        f.blocks[3].instructions.push(sub(next, 4, 0, IrType::I64));
        f.blocks[3].instructions.push(Instruction::BinOp {
            dest: value(next + 1),
            op: IrBinOp::Add,
            lhs: v(acc),
            rhs: v(next),
            ty: IrType::I64,
        });
        acc = next + 1;
        next += 2;
    }
    f.blocks[3].terminator = Terminator::Return(Some(v(acc)));
    for block in &mut f.blocks {
        block.source_spans = vec![crate::common::source::Span::dummy(); block.instructions.len()];
        block.source_inline_ids =
            vec![crate::ir::instruction::NO_INLINE_INSTANCE; block.instructions.len()];
    }
    f.next_value_id = 1; // stale even with a large number of destinations
    let first_free = f.sound_next_value_id();
    assert_eq!(run(&mut f), 128);
    assert_eq!(f.next_value_id, first_free + 1, "only one new phi ID");
    assert_eq!(
        f.blocks[3]
            .instructions
            .iter()
            .filter(|inst| matches!(
                inst,
                Instruction::Phi {
                    ty: IrType::I64,
                    ..
                }
            ))
            .count(),
        1,
        "all 128 differences of the same pair must share one integer phi"
    );
    assert!(
        f.blocks
            .iter()
            .all(|b| b.instructions.iter().all(|inst| !matches!(
                inst,
                Instruction::BinOp {
                    op: IrBinOp::Sub,
                    ..
                }
            )))
    );
    assert!(
        f.blocks
            .iter()
            .all(|b| b.source_spans.len() == b.instructions.len()
                && b.source_inline_ids.len() == b.instructions.len())
    );
    assert_valid(&f);
}

#[test]
fn negative_offset_and_copy_chains_preserve_signed_offsets() {
    let mut f = diamond();
    f.blocks[1].instructions[0] = gep(3, 0, Operand::Const(IrConst::I32(-7)));
    f.blocks[3].instructions.insert(
        1,
        Instruction::Copy {
            dest: value(6),
            src: v(4),
        },
    );
    f.blocks[3].instructions[2] = sub(5, 6, 0, IrType::I64);
    assert_eq!(run(&mut f), 1);
    let incoming = f.blocks[3]
        .instructions
        .iter()
        .find_map(|inst| match inst {
            Instruction::Phi {
                ty: IrType::I64,
                incoming,
                ..
            } => Some(incoming),
            _ => None,
        })
        .expect("signed integer phi");
    assert_eq!(incoming[0].0, Operand::Const(IrConst::I64(-7)));
    assert_valid(&f);
}

#[test]
fn duplicate_definition_through_inline_asm_and_large_id_fail_closed() {
    let mut f = diamond();
    f.blocks[0].instructions.push(Instruction::InlineAsm {
        template: String::new(),
        outputs: vec![("=r".into(), value(3), None)],
        inputs: Vec::new(),
        clobbers: Vec::new(),
        operand_types: vec![IrType::Ptr],
        goto_labels: Vec::new(),
        input_symbols: Vec::new(),
        seg_overrides: Vec::new(),
    });
    // The asm defines %3 first, then the GEP would re-define it: do not
    // derive an offset from an SSA value with multiple definitions.
    assert_eq!(run(&mut f), 0);
    assert!(matches!(
        f.blocks[3].instructions[1],
        Instruction::BinOp {
            op: IrBinOp::Sub,
            ..
        }
    ));

    let mut f = diamond();
    f.next_value_id = 1_000_002; // bounded memory even for malformed huge IDs
    assert_eq!(run(&mut f), 0);
    assert!(matches!(
        f.blocks[3].instructions[1],
        Instruction::BinOp {
            op: IrBinOp::Sub,
            ..
        }
    ));
}
