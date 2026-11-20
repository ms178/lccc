//! Adversarial edge/PHI transactions: test the threader before other CFG passes
//! can hide the bug by deleting unreachable blocks or simplifying phis.
use super::*;

fn block(id: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
    BasicBlock {
        label: BlockId(id),
        instructions,
        terminator,
        source_spans: vec![],
        source_inline_ids: Vec::new(),
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

fn switch(cases: &[(i64, u32)], default: u32) -> Terminator {
    Terminator::Switch {
        val: Operand::Value(Value(90)),
        cases: cases.iter().map(|&(v, b)| (v, BlockId(b))).collect(),
        default: BlockId(default),
        ty: IrType::I32,
    }
}
fn selector() -> Instruction {
    Instruction::Copy {
        dest: Value(90),
        src: Operand::Const(IrConst::I32(0)),
    }
}
fn ret(value: i32) -> Terminator {
    Terminator::Return(Some(Operand::Const(IrConst::I32(value))))
}
fn verified(f: &IrFunction, stage: &str) {
    let mut errors = vec![];
    super::super::verify::verify_function(f, stage, &mut errors);
    assert!(errors.is_empty(), "{stage}: {errors:?}");
}

/// `case 0: break;` and a `default:` that only breaks: two empty blocks in
/// front of the join. Linux's `ftrace_bug` has exactly this shape, and the
/// relay it left behind the jump table is what objtool reported.
#[test]
fn switch_edges_thread_through_empty_blocks() {
    let mut f = fixture(vec![
        block(0, vec![selector()], switch(&[(0, 1), (1, 2), (2, 3)], 4)),
        block(1, vec![], jump(5)),
        block(2, vec![], ret(22)),
        block(3, vec![], ret(33)),
        block(4, vec![], jump(5)),
        block(5, vec![], ret(55)),
    ]);
    verified(&f, "before");
    assert_eq!(run_thread(&mut f), 1);
    assert!(
        matches!(&f.blocks[0].terminator, Terminator::Switch { cases, default: BlockId(5), .. }
            if cases.as_slice() == [(0, BlockId(5)), (1, BlockId(2)), (2, BlockId(3))]),
        "{:?}",
        f.blocks[0].terminator
    );
    verified(&f, "after");
    // The case that now says what the default says is dropped; the table
    // keeps the two cases that select something.
    assert_eq!(simplify_redundant_cond_branches(&mut f), 1);
    assert!(
        matches!(&f.blocks[0].terminator, Terminator::Switch { cases, default: BlockId(5), .. }
            if cases.as_slice() == [(1, BlockId(2)), (2, BlockId(3))]),
        "{:?}",
        f.blocks[0].terminator
    );
    assert_eq!(eliminate_unreachable_blocks(&mut f), 2);
    verified(&f, "after cleanup");
}

/// Every case threading to the default's end leaves no selection at all.
#[test]
fn switch_whose_cases_all_reach_the_default_becomes_a_branch() {
    let mut f = fixture(vec![
        block(0, vec![selector()], switch(&[(3, 1), (7, 2)], 4)),
        block(1, vec![], jump(4)),
        block(2, vec![], jump(4)),
        block(4, vec![], ret(1)),
    ]);
    assert_eq!(run_thread(&mut f), 1);
    assert_eq!(simplify_redundant_cond_branches(&mut f), 1);
    assert!(matches!(
        f.blocks[0].terminator,
        Terminator::Branch(BlockId(4))
    ));
    verified(&f, "after");
}

/// The join's phi: the relays of cases 0 and 1 carry the same value through
/// two-predecessor relays, case 2's relay carries another one. The first edge
/// into the join adds the arm, the second has to agree with it, and the third
/// cannot become the same predecessor with a different value, so it keeps
/// its relay — while the first two still thread (a conditional branch would
/// have been refused as a whole).
#[test]
fn switch_edges_into_one_join_must_agree_and_are_decided_one_by_one() {
    let mut f = fixture(vec![
        block(0, vec![selector()], switch(&[(0, 1), (1, 2), (2, 3)], 6)),
        block(1, vec![], jump(5)),
        block(2, vec![], jump(5)),
        block(3, vec![], jump(5)),
        block(
            5,
            vec![phi(50, &[(7, 1), (7, 2), (9, 3)])],
            Terminator::Return(Some(Operand::Value(Value(50)))),
        ),
        block(6, vec![], ret(0)),
        // Second predecessors: a sole predecessor's relay is never bypassed.
        block(7, vec![], cond(1, 2)),
        block(8, vec![], jump(3)),
    ]);
    verified(&f, "before");
    // Three blocks change: the switch, and the two blocks that only exist to
    // give the relays a second predecessor (they thread like any branch).
    assert_eq!(run_thread(&mut f), 3);
    assert!(
        matches!(&f.blocks[0].terminator, Terminator::Switch { cases, default: BlockId(6), .. }
            if cases.as_slice() == [(0, BlockId(5)), (1, BlockId(5)), (2, BlockId(3))]),
        "{:?}",
        f.blocks[0].terminator
    );
    let Instruction::Phi { incoming, .. } = &f.blocks[4].instructions[0] else {
        panic!("phi expected");
    };
    assert_eq!(
        incoming
            .iter()
            .filter(|(_, from)| *from == BlockId(0))
            .count(),
        1,
        "one arm per predecessor: {incoming:?}"
    );
    assert!(
        incoming.contains(&(Operand::Const(IrConst::I32(7)), BlockId(0))),
        "{incoming:?}"
    );
    verified(&f, "after");
}

/// A relay that is the only way its phi arm reaches the join stays, for a
/// `Switch` as for a conditional branch.
#[test]
fn switch_does_not_bypass_a_sole_phi_relay() {
    let mut f = fixture(vec![
        block(0, vec![selector()], switch(&[(0, 1)], 2)),
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

/// The redundancy rule on its own: only the cases that equal the default go,
/// and a `Switch` that still selects stays a `Switch`.
#[test]
fn only_cases_that_equal_the_default_are_dropped() {
    let mut f = fixture(vec![
        block(
            0,
            vec![selector()],
            switch(&[(0, 3), (1, 1), (2, 3), (9, 2)], 3),
        ),
        block(1, vec![], ret(1)),
        block(2, vec![], ret(2)),
        block(3, vec![], ret(3)),
    ]);
    assert_eq!(simplify_redundant_cond_branches(&mut f), 1);
    assert!(
        matches!(&f.blocks[0].terminator, Terminator::Switch { cases, default: BlockId(3), .. }
            if cases.as_slice() == [(1, BlockId(1)), (9, BlockId(2))]),
        "{:?}",
        f.blocks[0].terminator
    );
    assert_eq!(simplify_redundant_cond_branches(&mut f), 0, "idempotent");
}

fn label_addr(dest: u32, label: u32) -> Instruction {
    Instruction::LabelAddr {
        dest: Value(dest),
        label: BlockId(label),
    }
}
fn computed_goto(target: u32, possible: &[u32]) -> Terminator {
    Terminator::IndirectBranch {
        target: Operand::Value(Value(target)),
        possible_targets: possible.iter().map(|&b| BlockId(b)).collect(),
    }
}
fn run_indirect(f: &mut IrFunction) -> usize {
    fold_known_indirect_branches(f, &build_label_to_idx(f))
}

/// `drm_exec_until_all_locked()`: the label's address is stored in a local and
/// the retry jumps through it. One label address reaches the jump, through a
/// copy; the other listed target loses this block's phi arm.
#[test]
fn computed_goto_through_one_label_address_becomes_a_branch() {
    let mut f = fixture(vec![
        block(
            0,
            vec![
                label_addr(1, 2),
                Instruction::Copy {
                    dest: Value(2),
                    src: Operand::Value(Value(1)),
                },
            ],
            computed_goto(2, &[2, 3]),
        ),
        block(2, vec![phi(20, &[(5, 0), (6, 4)])], ret(2)),
        block(3, vec![phi(30, &[(7, 0), (8, 4)])], ret(3)),
        block(4, vec![label_addr(3, 3)], computed_goto(40, &[2, 3])),
    ]);
    assert_eq!(run_indirect(&mut f), 1);
    assert!(matches!(
        f.blocks[0].terminator,
        Terminator::Branch(BlockId(2))
    ));
    let arms = |block: usize| match &f.blocks[block].instructions[0] {
        Instruction::Phi { incoming, .. } => incoming.clone(),
        other => panic!("phi expected, got {other:?}"),
    };
    assert_eq!(arms(1).len(), 2, "the taken target keeps its arm");
    assert_eq!(
        arms(2),
        vec![(Operand::Const(IrConst::I32(8)), BlockId(4))],
        "the other target loses this block's arm only"
    );
    // Block 4 jumps through a value this function does not define: it stays.
    assert!(matches!(
        f.blocks[3].terminator,
        Terminator::IndirectBranch { .. }
    ));
}

/// With one address-taken label in the function there is one place to go,
/// whatever the operand is; with none listed, or with a label address the
/// terminator does not list, nothing is known.
#[test]
fn computed_goto_folds_only_on_a_listed_target() {
    let mut one = fixture(vec![
        block(0, vec![], computed_goto(77, &[1])),
        block(1, vec![], ret(1)),
    ]);
    assert_eq!(run_indirect(&mut one), 1);
    assert!(matches!(
        one.blocks[0].terminator,
        Terminator::Branch(BlockId(1))
    ));

    let mut unlisted = fixture(vec![
        block(0, vec![label_addr(1, 3)], computed_goto(1, &[1, 2])),
        block(1, vec![], ret(1)),
        block(2, vec![], ret(2)),
        block(3, vec![], ret(3)),
    ]);
    assert_eq!(run_indirect(&mut unlisted), 0);

    let mut none = fixture(vec![
        block(0, vec![label_addr(1, 1)], computed_goto(1, &[])),
        block(1, vec![], ret(1)),
    ]);
    assert_eq!(run_indirect(&mut none), 0);

    let mut two = fixture(vec![
        block(0, vec![], computed_goto(77, &[1, 2])),
        block(1, vec![], ret(1)),
        block(2, vec![], ret(2)),
    ]);
    assert_eq!(run_indirect(&mut two), 0);
}
