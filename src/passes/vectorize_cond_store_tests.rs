//! Safety witnesses for the private-destination conditional-store proof.

use super::{loop_analysis::NaturalLoop, proves_private_stack_object, rewrite_conditional_store};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::instruction::{
    BasicBlock, BlockId, CallInfo, Instruction, Operand, Terminator, Value,
};
use crate::ir::ops::{IrBinOp, IrCmpOp};
use crate::ir::reexports::{IrConst, IrFunction};

fn fixture() -> IrFunction {
    let mut f = IrFunction::new("private_store".into(), IrType::Void, vec![], false);
    f.blocks.push(BasicBlock {
        label: BlockId(0),
        instructions: vec![
            Instruction::Alloca {
                dest: Value(0),
                ty: IrType::U32,
                size: 64,
                align: 4,
                volatile: false,
                semantic_volatile: false,
            },
            Instruction::GetElementPtr {
                dest: Value(1),
                base: Value(0),
                offset: Operand::Const(IrConst::I64(4)),
                ty: IrType::U32,
            },
            Instruction::Copy {
                dest: Value(2),
                src: Operand::Value(Value(1)),
            },
            Instruction::Load {
                dest: Value(3),
                ptr: Value(2),
                ty: IrType::U32,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
            Instruction::Store {
                val: Operand::Value(Value(3)),
                ptr: Value(2),
                ty: IrType::U32,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
        ],
        terminator: Terminator::Return(None),
        source_spans: vec![],
    });
    f.next_value_id = 4;
    f
}

#[test]
fn accepts_only_the_unescaped_private_address() {
    let f = fixture();
    assert!(proves_private_stack_object(&f, Value(2)));

    let mut parameter = f.clone();
    parameter.blocks[0].instructions[0] = Instruction::ParamRef {
        dest: Value(0),
        param_idx: 0,
        ty: IrType::Ptr,
    };
    assert!(!proves_private_stack_object(&parameter, Value(2)));

    let mut global = f.clone();
    global.blocks[0].instructions[0] = Instruction::GlobalAddr {
        dest: Value(0),
        name: "shared".into(),
    };
    assert!(!proves_private_stack_object(&global, Value(2)));

    let mut volatile = f.clone();
    if let Instruction::Alloca {
        semantic_volatile, ..
    } = &mut volatile.blocks[0].instructions[0]
    {
        *semantic_volatile = true;
    }
    assert!(!proves_private_stack_object(&volatile, Value(2)));
}

#[test]
fn refuses_publication_observation_and_ambiguous_definitions() {
    let f = fixture();
    let mut escaped = f.clone();
    escaped.blocks[0].instructions.push(Instruction::Store {
        val: Operand::Value(Value(1)),
        ptr: Value(2),
        ty: IrType::Ptr,
        seg_override: AddressSpace::Default,
        volatile: false,
    });
    assert!(!proves_private_stack_object(&escaped, Value(2)));

    let mut returned = f.clone();
    returned.blocks[0].terminator = Terminator::Return(Some(Operand::Value(Value(1))));
    assert!(!proves_private_stack_object(&returned, Value(2)));

    let mut called = f.clone();
    called.blocks[0].instructions.push(Instruction::Call {
        func: "publish".into(),
        info: CallInfo {
            args: vec![Operand::Value(Value(0))],
            ..Default::default()
        },
    });
    assert!(!proves_private_stack_object(&called, Value(2)));

    let mut cast = f.clone();
    cast.blocks[0].instructions.push(Instruction::Cast {
        dest: Value(4),
        src: Operand::Value(Value(1)),
        from_ty: IrType::Ptr,
        to_ty: IrType::U64,
    });
    assert!(!proves_private_stack_object(&cast, Value(2)));

    let mut duplicated = f.clone();
    duplicated.blocks[0].instructions.push(Instruction::Copy {
        dest: Value(2),
        src: Operand::Value(Value(0)),
    });
    assert!(!proves_private_stack_object(&duplicated, Value(2)));

    let mut address_as_offset = f.clone();
    address_as_offset.blocks[0]
        .instructions
        .push(Instruction::GetElementPtr {
            dest: Value(4),
            base: Value(2),
            offset: Operand::Value(Value(1)),
            ty: IrType::U8,
        });
    assert!(!proves_private_stack_object(&address_as_offset, Value(2)));
}

#[test]
fn refuses_volatile_segmented_and_opaque_stack_accesses() {
    let f = fixture();
    let mut volatile = f.clone();
    if let Instruction::Load { volatile, .. } = &mut volatile.blocks[0].instructions[3] {
        *volatile = true;
    }
    assert!(!proves_private_stack_object(&volatile, Value(2)));

    let mut segmented = f.clone();
    if let Instruction::Store { seg_override, .. } = &mut segmented.blocks[0].instructions[4] {
        *seg_override = AddressSpace::SegGs;
    }
    assert!(!proves_private_stack_object(&segmented, Value(2)));

    let mut asm = f;
    asm.blocks[0].instructions.push(Instruction::InlineAsm {
        template: String::new(),
        outputs: vec![],
        inputs: vec![],
        clobbers: vec!["memory".into()],
        operand_types: vec![],
        goto_labels: vec![],
        input_symbols: vec![],
        seg_overrides: vec![],
    });
    assert!(!proves_private_stack_object(&asm, Value(2)));
}

fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
    BasicBlock {
        label: BlockId(label),
        instructions,
        terminator,
        source_spans: vec![],
    }
}

fn branch(cond: Operand, yes: u32, no: u32) -> Terminator {
    Terminator::CondBranch {
        cond,
        true_label: BlockId(yes),
        false_label: BlockId(no),
    }
}

fn rewrite_fixture(folded: bool, private: bool) -> (IrFunction, NaturalLoop, usize) {
    let mut f = IrFunction::new("guarded_loop".into(), IrType::Void, vec![], false);
    let root = if private {
        Instruction::Alloca {
            dest: Value(0),
            ty: IrType::I32,
            size: 64,
            align: 4,
            volatile: false,
            semantic_volatile: false,
        }
    } else {
        Instruction::ParamRef {
            dest: Value(0),
            param_idx: 0,
            ty: IrType::Ptr,
        }
    };
    let mut guard = vec![
        Instruction::GetElementPtr {
            dest: Value(1),
            base: Value(0),
            offset: Operand::Const(IrConst::I64(0)),
            ty: IrType::I32,
        },
        Instruction::Load {
            dest: Value(2),
            ptr: Value(1),
            ty: IrType::I32,
            seg_override: AddressSpace::Default,
            volatile: false,
        },
        Instruction::Cmp {
            dest: Value(3),
            op: IrCmpOp::Sgt,
            lhs: Operand::Value(Value(2)),
            rhs: Operand::Const(IrConst::I32(0)),
            ty: IrType::I32,
        },
    ];
    let store = Instruction::Store {
        val: Operand::Const(IrConst::I32(42)),
        ptr: Value(1),
        ty: IrType::I32,
        seg_override: AddressSpace::Default,
        volatile: false,
    };
    if folded {
        guard.push(Instruction::Cmp {
            dest: Value(4),
            op: IrCmpOp::Slt,
            lhs: Operand::Value(Value(2)),
            rhs: Operand::Const(IrConst::I32(10)),
            ty: IrType::I32,
        });
        guard.push(Instruction::BinOp {
            dest: Value(5),
            op: IrBinOp::And,
            lhs: Operand::Value(Value(3)),
            rhs: Operand::Value(Value(4)),
            ty: IrType::I32,
        });
        f.blocks = vec![
            block(0, vec![root], Terminator::Branch(BlockId(1))),
            block(1, guard, branch(Operand::Value(Value(5)), 2, 3)),
            block(2, vec![store], Terminator::Branch(BlockId(3))),
            block(3, vec![], branch(Operand::Const(IrConst::I32(1)), 1, 4)),
            block(4, vec![], Terminator::Return(None)),
        ];
        f.next_value_id = 6;
        (
            f,
            NaturalLoop {
                header: 1,
                body: [1, 2, 3].into_iter().collect(),
            },
            2,
        )
    } else {
        f.blocks = vec![
            block(0, vec![root], Terminator::Branch(BlockId(1))),
            block(1, vec![], branch(Operand::Const(IrConst::I32(1)), 2, 5)),
            block(2, guard, branch(Operand::Value(Value(3)), 3, 4)),
            block(3, vec![store], Terminator::Branch(BlockId(4))),
            block(4, vec![], Terminator::Branch(BlockId(1))),
            block(5, vec![], Terminator::Return(None)),
        ];
        f.next_value_id = 4;
        (
            f,
            NaturalLoop {
                header: 1,
                body: [1, 2, 3, 4].into_iter().collect(),
            },
            3,
        )
    }
}

#[test]
fn both_rewrite_shapes_require_private_storage() {
    for folded in [false, true] {
        let (mut f, lp, _) = rewrite_fixture(folded, false);
        assert!(!rewrite_conditional_store(&mut f, &lp));
        let (mut f, lp, _) = rewrite_fixture(folded, true);
        assert!(rewrite_conditional_store(&mut f, &lp));
    }
}

#[test]
fn neither_rewrite_speculates_traps_or_fp() {
    for folded in [false, true] {
        for (op, ty) in [(IrBinOp::SDiv, IrType::I32), (IrBinOp::Add, IrType::F32)] {
            let (mut f, lp, store) = rewrite_fixture(folded, true);
            let (lhs, rhs) = if ty.is_float() {
                (IrConst::F32(1.0), IrConst::F32(0.0))
            } else {
                (IrConst::I32(1), IrConst::I32(0))
            };
            f.blocks[store].instructions.insert(
                0,
                Instruction::BinOp {
                    dest: Value(20),
                    op,
                    lhs: Operand::Const(lhs),
                    rhs: Operand::Const(rhs),
                    ty,
                },
            );
            f.next_value_id = 21;
            assert!(!rewrite_conditional_store(&mut f, &lp));
        }
    }
}

#[test]
fn both_rewrites_reject_unrelated_duplicate_definitions() {
    for folded in [false, true] {
        let (mut f, lp, _) = rewrite_fixture(folded, true);
        f.blocks[0].instructions.push(Instruction::Copy {
            dest: Value(10),
            src: Operand::Const(IrConst::I32(1)),
        });
        f.blocks[0].instructions.push(Instruction::Copy {
            dest: Value(10),
            src: Operand::Const(IrConst::I32(2)),
        });
        assert!(!rewrite_conditional_store(&mut f, &lp));
    }
}
