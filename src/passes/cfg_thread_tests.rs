//! Adversarial edge/PHI transactions: test the threader before other CFG passes
//! can hide the bug by deleting unreachable blocks or simplifying phis.
use super::*;

fn block(id: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
    BasicBlock {
        label: BlockId(id),
        instructions,
        terminator,
        source_spans: vec![],
    }
}
fn jump(id: u32) -> Terminator {
    Terminator::Branch(BlockId(id))
}
fn cond(yes: u32, no: u32) -> Terminator {
    Terminator::CondBranch {
        cond: Operand::Const(IrConst::I32(1)),
        true_label: BlockId(yes),
        false_label: BlockId(no),
    }
}
fn phi(id: u32, arms: &[(i32, u32)]) -> Instruction {
    Instruction::Phi {
        dest: Value(id),
        ty: IrType::I32,
        incoming: arms
            .iter()
            .map(|&(v, b)| (Operand::Const(IrConst::I32(v)), BlockId(b)))
            .collect(),
    }
}
fn fixture(blocks: Vec<BasicBlock>) -> IrFunction {
    let mut f = IrFunction::new("thread_redteam".into(), IrType::I32, vec![], false);
    f.blocks = blocks;
    f.next_label = 100;
    f.next_value_id = 100;
    f
}
fn run_thread(f: &mut IrFunction) -> usize {
    thread_jump_chains(f, &build_label_to_idx(f))
}

#[test]
fn rejected_phi_edges_are_atomic_and_cannot_underflow_change_count() {
    for first_valid in [false, true] {
        let mut f = fixture(vec![
            block(0, vec![], cond(1, 2)),
            block(1, vec![], jump(3)),
            block(2, vec![], jump(4)),
            block(
                3,
                vec![phi(
                    3,
                    if first_valid {
                        &[(11, 1), (12, 9)]
                    } else {
                        &[]
                    },
                )],
                Terminator::Return(Some(Operand::Value(Value(3)))),
            ),
            block(
                4,
                vec![phi(4, &[])],
                Terminator::Return(Some(Operand::Value(Value(4)))),
            ),
            // A second predecessor keeps the legacy single-predecessor filter
            // from accidentally masking the partial-mutation bug.
            block(9, vec![], jump(1)),
        ]);
        let before = format!("{f:?}");
        assert_eq!(
            run_thread(&mut f),
            0,
            "rejected both edges, first_valid={first_valid}"
        );
        assert_eq!(
            format!("{f:?}"),
            before,
            "a rejected block must not partially mutate"
        );
    }
}

#[test]
fn asm_goto_existing_phi_edge_cannot_be_overwritten_by_normal_edge() {
    let asm = Instruction::InlineAsm {
        template: "# possible branch to %l[exit]".into(),
        outputs: vec![],
        inputs: vec![],
        clobbers: vec![],
        operand_types: vec![],
        goto_labels: vec![("exit".into(), BlockId(3))],
        input_symbols: vec![],
        seg_overrides: vec![],
    };
    let mut f = fixture(vec![
        block(4, vec![], cond(0, 2)),
        block(0, vec![asm], jump(1)),
        block(1, vec![], jump(3)),
        block(2, vec![], jump(1)),
        block(
            3,
            vec![phi(0, &[(9, 0), (1, 1)])],
            Terminator::Return(Some(Operand::Value(Value(0)))),
        ),
    ]);
    let mut errors = vec![];
    super::super::verify::verify_function(&f, "before-thread", &mut errors);
    assert!(errors.is_empty(), "valid input: {errors:?}");
    run_thread(&mut f);
    assert!(
        matches!(f.blocks[1].terminator, Terminator::Branch(BlockId(1))),
        "normal fallthrough returns 1; merging it into the asm edge would return 9"
    );
    assert_eq!(
        merge_single_pred_blocks(&mut f),
        0,
        "asm-goto source must not absorb the remaining normal-path relay"
    );
    super::super::verify::verify_function(&f, "after-thread", &mut errors);
    assert!(errors.is_empty(), "valid output: {errors:?}");
}

#[test]
fn sole_phi_relay_is_not_bypassed() {
    // Block 1 is the only predecessor feeding the phi. Bypassing it deletes
    // that arm unless every consumer is rewritten together; the measured
    // pipeline keeps the relay. Multi-predecessor relays remain threadable.
    let mut f = fixture(vec![
        block(0, vec![], cond(1, 2)),
        block(1, vec![], jump(3)),
        block(
            2,
            vec![Instruction::Copy {
                dest: Value(10),
                src: Operand::Const(IrConst::I32(22)),
            }],
            jump(3),
        ),
        block(
            3,
            vec![phi(11, &[(11, 1), (22, 2)])],
            Terminator::Return(Some(Operand::Value(Value(11)))),
        ),
    ]);
    let before = format!("{f:?}");
    assert_eq!(run_thread(&mut f), 0);
    assert_eq!(format!("{f:?}"), before);
}
