use super::*;
use crate::common::source::Span;
use crate::common::types::AddressSpace;
use crate::ir::reexports::{BasicBlock, BlockId, CallInfo, IrCmpOp, IrModule, Terminator};

fn v(n: u32) -> Operand {
    Operand::Value(Value(n))
}
fn number(ty: IrType, n: f64) -> Operand {
    Operand::Const(match ty {
        IrType::F32 => IrConst::F32(n as f32),
        _ => IrConst::F64(n),
    })
}
fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
    BasicBlock {
        label: BlockId(label),
        source_spans: vec![Span::dummy(); instructions.len()],
        instructions,
        terminator,
    }
}
fn branch(to: u32) -> Terminator {
    Terminator::Branch(BlockId(to))
}
fn cond(yes: u32, no: u32) -> Terminator {
    Terminator::CondBranch {
        cond: Operand::Const(IrConst::I32(1)),
        true_label: BlockId(yes),
        false_label: BlockId(no),
    }
}
fn mul(dest: u32, input: u32, ty: IrType) -> Instruction {
    Instruction::BinOp {
        dest: Value(dest),
        op: IrBinOp::Mul,
        lhs: v(input),
        rhs: v(input),
        ty,
    }
}
fn fixture(ty: IrType) -> IrFunction {
    let mut f = IrFunction::new("square".into(), ty, vec![], false);
    f.blocks = vec![
        block(
            0,
            vec![Instruction::Copy {
                dest: Value(500),
                src: Operand::Const(IrConst::I32(7)),
            }],
            branch(10),
        ),
        block(
            10,
            vec![Instruction::Phi {
                dest: Value(1),
                ty,
                incoming: vec![(number(ty, 0.0), BlockId(0)), (v(3), BlockId(30))],
            }],
            cond(20, 40),
        ),
        block(
            20,
            vec![
                mul(2, 1, ty),
                Instruction::BinOp {
                    dest: Value(3),
                    op: IrBinOp::Add,
                    lhs: v(2),
                    rhs: number(ty, 1.0),
                    ty,
                },
                mul(4, 3, ty),
                Instruction::Cmp {
                    dest: Value(5),
                    op: IrCmpOp::Slt,
                    lhs: v(4),
                    rhs: number(ty, 100.0),
                    ty,
                },
            ],
            Terminator::CondBranch {
                cond: v(5),
                true_label: BlockId(30),
                false_label: BlockId(40),
            },
        ),
        block(30, vec![], branch(10)),
        block(40, vec![], Terminator::Return(Some(v(1)))),
    ];
    f.next_label = 41;
    f.next_value_id = 2; // Deliberately stale; new IDs must exceed v500.
    f
}
fn verify(f: &IrFunction) {
    let mut problems = vec![];
    super::super::verify::verify_function(f, "loop_square_test", &mut problems);
    assert!(problems.is_empty(), "{problems:?}");
    let mut module = IrModule::new();
    module.functions.push(f.clone());
    super::super::validate_unique_defs(&module, "loop_square_test");
}
fn unchanged(mut f: IrFunction) {
    let before = format!("{f:?}");
    assert_eq!(run(&mut f), 0);
    assert_eq!(format!("{f:?}"), before);
}

#[test]
fn carries_exact_positive_zero_for_both_signed_zero_seeds_and_precisions() {
    for ty in [IrType::F32, IrType::F64] {
        for seed in [0.0, -0.0] {
            for in_header in [false, true] {
                let mut f = fixture(ty);
                if let Instruction::Phi { incoming, .. } = &mut f.blocks[1].instructions[0] {
                    incoming[0].0 = number(ty, seed);
                }
                if in_header {
                    let square = f.blocks[2].instructions.remove(0);
                    f.blocks[2].source_spans.remove(0);
                    f.blocks[1].instructions.push(square);
                    f.blocks[1].source_spans.push(Span::dummy());
                }
                // Repair only the deliberately stale watermark for the input
                // verifier, then make it stale again to exercise allocation.
                f.next_value_id = 501;
                verify(&f);
                f.next_value_id = 2;
                let pre = format!("{:?}", f.blocks[0]);
                assert_eq!(run(&mut f), 1);
                assert_eq!(f.next_value_id, 502);
                assert_eq!(format!("{:?}", f.blocks[0]), pre, "no entry speculation");
                match &f.blocks[1].instructions[0] {
                    Instruction::Phi {
                        dest: Value(501),
                        incoming,
                        ty: t,
                    } => {
                        assert_eq!(*t, ty);
                        assert_eq!(incoming[1], (v(4), BlockId(30)));
                        match incoming[0].0 {
                            Operand::Const(IrConst::F32(x)) => assert_eq!(x.to_bits(), 0),
                            Operand::Const(IrConst::F64(x)) => assert_eq!(x.to_bits(), 0),
                            other => panic!("unexpected seed {other:?}"),
                        }
                    }
                    other => panic!("unexpected header {other:?}"),
                }
                for b in &f.blocks {
                    assert!(
                        b.source_spans.is_empty() || b.source_spans.len() == b.instructions.len()
                    );
                }
                verify(&f);
                assert_eq!(run(&mut f), 0);
            }
        }
    }
}

#[test]
fn rejects_unproved_initials_and_wrong_widths_or_operands() {
    for initial in [
        number(IrType::F64, 1.0),
        number(IrType::F64, f64::NAN),
        number(IrType::F64, f64::INFINITY),
        number(IrType::F64, f64::from_bits(1)),
        v(500),
        number(IrType::F32, 0.0),
    ] {
        let mut f = fixture(IrType::F64);
        if let Instruction::Phi { incoming, .. } = &mut f.blocks[1].instructions[0] {
            incoming[0].0 = initial;
        }
        unchanged(f);
    }
    for which in 0..4 {
        let mut f = fixture(IrType::F64);
        match which {
            0 => {
                if let Instruction::Phi { ty, .. } = &mut f.blocks[1].instructions[0] {
                    *ty = IrType::F32;
                }
            }
            1 => {
                if let Instruction::BinOp { ty, .. } = &mut f.blocks[2].instructions[2] {
                    *ty = IrType::F32;
                }
            }
            2 => {
                if let Instruction::BinOp { rhs, .. } = &mut f.blocks[2].instructions[0] {
                    *rhs = v(3);
                }
            }
            _ => {
                if let Instruction::BinOp { rhs, .. } = &mut f.blocks[2].instructions[2] {
                    *rhs = v(1);
                }
            }
        }
        unchanged(f);
    }
}

#[test]
fn rejects_calls_including_pure_calls_volatile_memory_and_intrinsics() {
    use crate::ir::intrinsics::IntrinsicOp;
    let barriers = vec![
        Instruction::Call {
            func: "fesetround".into(),
            info: CallInfo::default(),
        },
        Instruction::Call {
            func: "claimed_pure".into(),
            info: CallInfo {
                is_pure: true,
                is_const: true,
                ..CallInfo::default()
            },
        },
        Instruction::Load {
            dest: Value(80),
            ptr: Value(500),
            ty: IrType::F64,
            volatile: true,
            seg_override: AddressSpace::Default,
        },
        Instruction::Store {
            ptr: Value(500),
            val: number(IrType::F64, 1.0),
            ty: IrType::F64,
            volatile: true,
            seg_override: AddressSpace::Default,
        },
        Instruction::Intrinsic {
            dest: None,
            op: IntrinsicOp::Mfence,
            dest_ptr: None,
            args: vec![],
        },
        Instruction::InlineAsm {
            template: "ldmxcsr (%rax)".into(),
            outputs: vec![],
            inputs: vec![],
            clobbers: vec!["memory".into()],
            operand_types: vec![],
            goto_labels: vec![],
            input_symbols: vec![],
            seg_overrides: vec![],
        },
    ];
    for barrier in barriers {
        for at in [1, 2, 3] {
            let mut f = fixture(IrType::F64);
            f.blocks[at].instructions.push(barrier.clone());
            f.blocks[at].source_spans.clear();
            unchanged(f);
        }
    }
}

#[test]
fn rejects_non_dominating_producer_or_old_square_and_multiple_latches() {
    // Producer is on only one predecessor of the latch.
    let mut f = fixture(IrType::F64);
    let producer = f.blocks[2].instructions.remove(2);
    f.blocks[2].instructions.pop(); // Remove its old compare consumer.
    f.blocks[2].source_spans.clear();
    f.blocks[2].terminator = cond(25, 30);
    f.blocks.insert(3, block(25, vec![producer], branch(30)));
    unchanged(f);
    // The old square is conditional; do not add unconditional carried pressure.
    let mut f = fixture(IrType::F64);
    let old = f.blocks[2].instructions.remove(0);
    f.blocks[2].source_spans.clear();
    if let Instruction::BinOp { lhs, .. } = &mut f.blocks[2].instructions[0] {
        *lhs = v(1);
    }
    f.blocks[1].terminator = cond(15, 20);
    f.blocks.insert(2, block(15, vec![old], branch(20)));
    unchanged(f);
    // A second backedge needs a separately proved producer value.
    let mut f = fixture(IrType::F64);
    f.blocks[2].terminator = cond(30, 10);
    if let Instruction::Phi { incoming, .. } = &mut f.blocks[1].instructions[0] {
        incoming.push((v(3), BlockId(20)));
    }
    unchanged(f);
}

#[test]
fn rejects_non_dedicated_entry_duplicate_defs_and_pressure() {
    let mut f = fixture(IrType::F64);
    f.blocks[0].terminator = cond(10, 40);
    unchanged(f);
    let mut f = fixture(IrType::F64);
    f.blocks[0].instructions.push(Instruction::Copy {
        dest: Value(3),
        src: number(IrType::F64, 0.0),
    });
    unchanged(f);
    let mut f = fixture(IrType::F64);
    for n in 10..15 {
        f.blocks[1].instructions.push(Instruction::Phi {
            dest: Value(n),
            ty: IrType::F64,
            incoming: vec![(number(IrType::F64, 0.0), BlockId(0)), (v(n), BlockId(30))],
        });
    }
    unchanged(f);
}

#[test]
fn ignores_outer_loop_but_optimizes_the_innermost_loop() {
    let mut f = fixture(IrType::F64);
    f.blocks[0].terminator = branch(5);
    if let Instruction::Phi { incoming, .. } = &mut f.blocks[1].instructions[0] {
        incoming[0].1 = BlockId(7);
    }
    f.blocks[1].terminator = cond(20, 35);
    if let Terminator::CondBranch { false_label, .. } = &mut f.blocks[2].terminator {
        *false_label = BlockId(35);
    }
    f.blocks[4].terminator = Terminator::Return(Some(number(IrType::F64, 0.0)));
    f.blocks.insert(1, block(5, vec![], cond(7, 40)));
    f.blocks.insert(2, block(7, vec![], branch(10)));
    f.blocks.push(block(35, vec![], branch(5)));
    assert_eq!(run(&mut f), 1);
    verify(&f);
}
