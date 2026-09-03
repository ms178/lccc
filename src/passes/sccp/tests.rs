//! SCCP regression suite.
//!
//! Every test in the "red-team reproducers" section corresponds to a defect
//! found while auditing the upstream implementation this pass was adapted from
//! (`engineering/SCCP_ADOPTION_AUDIT.md`). Each is the *minimal* IR that makes
//! the defect observable, so a regression fails loudly and points at the cause
//! rather than at a downstream symptom.

use super::*;
use crate::common::types::IrType;
use crate::ir::reexports::*;

// ── IR construction helpers ─────────────────────────────────────────────────

fn func_of(blocks: Vec<BasicBlock>) -> IrFunction {
    let mut f = IrFunction::new("t".to_string(), IrType::Void, vec![], false);
    let mut max = 0u32;
    for b in &blocks {
        for inst in &b.instructions {
            if let Some(v) = inst.dest() {
                max = max.max(v.0);
            }
            inst.for_each_used_value(|id| max = max.max(id));
        }
        b.terminator.for_each_used_value(|id| max = max.max(id));
    }
    f.blocks = blocks;
    f.next_value_id = max + 1;
    f
}

fn blk(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
    BasicBlock {
        label: BlockId(label),
        instructions,
        terminator,
        source_spans: vec![],
    }
}

fn cst(dest: u32, c: IrConst) -> Instruction {
    Instruction::Copy {
        dest: Value(dest),
        src: Operand::Const(c),
    }
}

fn i32c(dest: u32, v: i32) -> Instruction {
    cst(dest, IrConst::I32(v))
}

fn add(dest: u32, lhs: Operand, rhs: Operand, ty: IrType) -> Instruction {
    Instruction::BinOp {
        dest: Value(dest),
        op: IrBinOp::Add,
        lhs,
        rhs,
        ty,
    }
}

fn phi(dest: u32, incoming: Vec<(Operand, u32)>, ty: IrType) -> Instruction {
    Instruction::Phi {
        dest: Value(dest),
        incoming: incoming
            .into_iter()
            .map(|(op, l)| (op, BlockId(l)))
            .collect(),
        ty,
    }
}

fn val(v: u32) -> Operand {
    Operand::Value(Value(v))
}

fn cond_br(cond: Operand, t: u32, f: u32) -> Terminator {
    Terminator::CondBranch {
        cond,
        true_label: BlockId(t),
        false_label: BlockId(f),
    }
}

fn br(t: u32) -> Terminator {
    Terminator::Branch(BlockId(t))
}

fn ret(op: Option<Operand>) -> Terminator {
    Terminator::Return(op)
}

fn load(dest: u32, ptr: u32) -> Instruction {
    Instruction::Load {
        dest: Value(dest),
        ptr: Value(ptr),
        ty: IrType::I32,
        seg_override: Default::default(),
        volatile: false,
    }
}

fn alloca(dest: u32) -> Instruction {
    Instruction::Alloca {
        dest: Value(dest),
        ty: IrType::I32,
        size: 4,
        align: 4,
        volatile: false,
        semantic_volatile: false,
    }
}

fn call(dest: Option<u32>, name: &str) -> Instruction {
    Instruction::Call {
        info: CallInfo {
            dest: dest.map(Value),
            args: vec![],
            return_type: IrType::I32,
            ..Default::default()
        },
        func: name.to_string(),
    }
}

/// Run SCCP and return `(stats, function)`.
fn sccp(mut f: IrFunction) -> (SccpStats, IrFunction) {
    let ud = UseDefInfo::build(&f);
    let stats = run_with_usedef(&mut f, &ud);
    (stats, f)
}

/// The phi operands of the first phi in block `bi`, as `(operand, pred label)`.
fn phi_incoming(f: &IrFunction, bi: usize) -> Vec<(Operand, BlockId)> {
    f.blocks[bi]
        .instructions
        .iter()
        .find_map(|i| match i {
            Instruction::Phi { incoming, .. } => Some(incoming.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// The constant a value was materialised to, if the block now defines it with
/// `Copy { dest, Const }`.
fn materialised(f: &IrFunction, v: u32) -> Option<IrConst> {
    f.blocks
        .iter()
        .flat_map(|b| b.instructions.iter())
        .find_map(|i| match i {
            Instruction::Copy {
                dest,
                src: Operand::Const(c),
            } if dest.0 == v => Some(*c),
            _ => None,
        })
}

// ═══════════════════════════════════════════════════════════════════════════
// Core algorithm
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn propagates_a_constant_through_an_arithmetic_chain() {
    // %0 = 5 ; %1 = %0 ; %2 = %1 + %1 ; ret %2   ->   %2 == 10
    let (stats, f) = sccp(func_of(vec![blk(
        0,
        vec![
            i32c(0, 5),
            Instruction::Copy {
                dest: Value(1),
                src: val(0),
            },
            add(2, val(1), val(1), IrType::I32),
        ],
        ret(Some(val(2))),
    )]));

    assert!(stats.total() > 0);
    assert_eq!(materialised(&f, 2), Some(IrConst::I32(10)));
    // The return operand is substituted directly, so DCE can take the rest.
    assert!(matches!(
        f.blocks[0].terminator,
        Terminator::Return(Some(Operand::Const(IrConst::I32(10))))
    ));
}

#[test]
fn merges_equal_constants_across_a_diamond() {
    // The showcase case: both arms assign 7, so the join is 7 and the second
    // test folds. No amount of *local* folding discovers this.
    //
    //   b0: br cond?, b1, b2      (cond is a parameter -> unknown)
    //   b1: br b3       b2: br b3
    //   b3: %r = phi [7 from b1, 7 from b2] ; ret %r
    let (_, f) = sccp(func_of(vec![
        blk(
            0,
            vec![Instruction::ParamRef {
                dest: Value(0),
                param_idx: 0,
                ty: IrType::I32,
            }],
            cond_br(val(0), 1, 2),
        ),
        blk(1, vec![], br(3)),
        blk(2, vec![], br(3)),
        blk(
            3,
            vec![phi(
                1,
                vec![
                    (Operand::Const(IrConst::I32(7)), 1),
                    (Operand::Const(IrConst::I32(7)), 2),
                ],
                IrType::I32,
            )],
            ret(Some(val(1))),
        ),
    ]));

    assert_eq!(materialised(&f, 1), Some(IrConst::I32(7)));
    assert!(matches!(
        f.blocks[3].terminator,
        Terminator::Return(Some(Operand::Const(IrConst::I32(7))))
    ));
}

#[test]
fn a_phi_ignores_operands_arriving_on_dead_edges() {
    // b1 is unreachable (b0 branches unconditionally to b2), so the phi must
    // see only the value from b0 and resolve to 42 — not meet it with 99.
    let (_, f) = sccp(func_of(vec![
        blk(0, vec![cst(0, IrConst::I64(42))], br(2)),
        blk(1, vec![], br(2)),
        blk(
            2,
            vec![phi(
                1,
                vec![(val(0), 0), (Operand::Const(IrConst::I64(99)), 1)],
                IrType::I64,
            )],
            ret(Some(val(1))),
        ),
    ]));

    assert_eq!(materialised(&f, 1), Some(IrConst::I64(42)));
}

#[test]
fn a_loop_carried_phi_is_overdefined() {
    // %1 = phi [1 from b0, %2 from b1] ; %2 = %1 + 1  ->  %1 is ⊥, not 1.
    let (_, f) = sccp(func_of(vec![
        blk(0, vec![i32c(0, 1)], br(1)),
        blk(
            1,
            vec![
                phi(1, vec![(val(0), 0), (val(2), 1)], IrType::I32),
                add(2, val(1), Operand::Const(IrConst::I32(1)), IrType::I32),
            ],
            br(1),
        ),
    ]));

    // The induction variable must survive as a value.
    match &f.blocks[1].instructions[1] {
        Instruction::BinOp { lhs, .. } => assert!(
            matches!(lhs, Operand::Value(Value(1))),
            "loop phi must stay overdefined, got {lhs:?}"
        ),
        other => panic!("expected the BinOp, found {other:?}"),
    }
}

#[test]
fn a_parameter_blocks_propagation() {
    let (stats, f) = sccp(func_of(vec![blk(
        0,
        vec![
            Instruction::ParamRef {
                dest: Value(0),
                param_idx: 0,
                ty: IrType::I32,
            },
            add(1, val(0), Operand::Const(IrConst::I32(1)), IrType::I32),
        ],
        ret(Some(val(1))),
    )]));

    assert_eq!(stats.total(), 0, "nothing is knowable here");
    match &f.blocks[0].instructions[1] {
        Instruction::BinOp { lhs, .. } => assert!(matches!(lhs, Operand::Value(Value(0)))),
        other => panic!("expected the BinOp, found {other:?}"),
    }
}

#[test]
fn folds_a_select_on_a_known_condition() {
    let (_, f) = sccp(func_of(vec![blk(
        0,
        vec![
            i32c(0, 1),
            Instruction::Select {
                dest: Value(1),
                cond: val(0),
                true_val: Operand::Const(IrConst::I32(42)),
                false_val: Operand::Const(IrConst::I32(99)),
                ty: IrType::I32,
            },
        ],
        ret(Some(val(1))),
    )]));

    assert_eq!(materialised(&f, 1), Some(IrConst::I32(42)));
}

#[test]
fn folds_a_select_whose_arms_agree_despite_an_unknown_condition() {
    let (_, f) = sccp(func_of(vec![blk(
        0,
        vec![
            Instruction::ParamRef {
                dest: Value(0),
                param_idx: 0,
                ty: IrType::I32,
            },
            Instruction::Select {
                dest: Value(1),
                cond: val(0),
                true_val: Operand::Const(IrConst::I32(5)),
                false_val: Operand::Const(IrConst::I32(5)),
                ty: IrType::I32,
            },
        ],
        ret(Some(val(1))),
    )]));

    assert_eq!(materialised(&f, 1), Some(IrConst::I32(5)));
}

#[test]
fn folds_a_switch_to_the_matching_case() {
    let (stats, f) = sccp(func_of(vec![
        blk(
            0,
            vec![i32c(0, 2)],
            Terminator::Switch {
                val: val(0),
                cases: vec![(1, BlockId(1)), (2, BlockId(2))],
                default: BlockId(3),
                ty: IrType::I32,
            },
        ),
        blk(1, vec![], ret(None)),
        blk(2, vec![], ret(None)),
        blk(3, vec![], ret(None)),
    ]));

    assert_eq!(stats.branches_folded, 1);
    assert!(matches!(
        f.blocks[0].terminator,
        Terminator::Branch(BlockId(2))
    ));
    assert_eq!(stats.unreachable_blocks, 2, "b1 and b3 are dead");
}

#[test]
fn folds_a_switch_with_no_matching_case_to_the_default() {
    let (_, f) = sccp(func_of(vec![
        blk(
            0,
            vec![i32c(0, 77)],
            Terminator::Switch {
                val: val(0),
                cases: vec![(1, BlockId(1)), (2, BlockId(2))],
                default: BlockId(3),
                ty: IrType::I32,
            },
        ),
        blk(1, vec![], ret(None)),
        blk(2, vec![], ret(None)),
        blk(3, vec![], ret(None)),
    ]));

    assert!(matches!(
        f.blocks[0].terminator,
        Terminator::Branch(BlockId(3))
    ));
}

// ═══════════════════════════════════════════════════════════════════════════
// Red-team reproducers
// ═══════════════════════════════════════════════════════════════════════════

/// **F1 — an unmodelled opcode must not stay ⊤.**
///
/// A `Load` is not one of the opcodes with a transfer function. If it were left
/// at `⊤`, the phi below would compute `⊤ ⊓ C(1) = C(1)` and the function would
/// be rewritten to `return 1` — discarding the loaded value entirely.
///
/// The closed default in `evaluate_instruction` is what prevents this, and it
/// is keyed on `dest()` so *any* future opcode is covered automatically.
#[test]
fn an_unmodelled_definition_is_overdefined_not_top() {
    let (_, f) = sccp(func_of(vec![
        blk(
            0,
            vec![
                Instruction::ParamRef {
                    dest: Value(9),
                    param_idx: 0,
                    ty: IrType::I32,
                },
                alloca(0),
                load(1, 0), // %1 is unmodelled -> must be ⊥
            ],
            cond_br(val(9), 1, 2),
        ),
        blk(1, vec![], br(3)),
        blk(2, vec![], br(3)),
        blk(
            3,
            vec![phi(
                2,
                vec![(val(1), 1), (Operand::Const(IrConst::I32(1)), 2)],
                IrType::I32,
            )],
            ret(Some(val(2))),
        ),
    ]));

    assert_eq!(
        materialised(&f, 2),
        None,
        "the phi merges a loaded value with 1 and cannot be constant"
    );
    assert!(
        matches!(
            f.blocks[3].terminator,
            Terminator::Return(Some(Operand::Value(Value(2))))
        ),
        "return must still read the phi, found {:?}",
        f.blocks[3].terminator
    );
}

/// Same hazard, reached through a `Call` rather than a `Load`, and additionally
/// checking that the call itself survives the rewrite (invariant 4).
#[test]
fn a_call_result_is_overdefined_and_the_call_is_never_deleted() {
    let (_, f) = sccp(func_of(vec![blk(
        0,
        vec![call(Some(0), "side_effect")],
        ret(Some(val(0))),
    )]));

    assert!(
        matches!(f.blocks[0].instructions[0], Instruction::Call { .. }),
        "the call must survive, found {:?}",
        f.blocks[0].instructions[0]
    );
    assert!(matches!(
        f.blocks[0].terminator,
        Terminator::Return(Some(Operand::Value(Value(0))))
    ));
}

/// **F2 — folding a conditional branch must remove the dead edge's phi operand.**
///
/// ```text
///   b0: %c = 1 ; br %c ? b1 : b2       -> folds to `br b1`
///   b1: br b2
///   b2: %p = phi [111 from b0, 222 from b1] ; ret %p
/// ```
///
/// After the fold, `b0 -> b2` no longer exists, so the `(111, b0)` operand is
/// stale. Phi elimination materialises one copy per *listed* predecessor edge,
/// so leaving it behind writes 111 into the phi's register on a live path.
/// The whole point of this test is that the repair happens inside SCCP, not
/// "later, in cfg_simplify" — the IR must be well-formed the instant SCCP
/// returns.
#[test]
fn folding_a_cond_branch_prunes_the_stale_phi_operand() {
    let (stats, f) = sccp(func_of(vec![
        blk(0, vec![i32c(0, 1)], cond_br(val(0), 1, 2)),
        blk(1, vec![], br(2)),
        blk(
            2,
            vec![phi(
                1,
                vec![
                    (Operand::Const(IrConst::I32(111)), 0),
                    (Operand::Const(IrConst::I32(222)), 1),
                ],
                IrType::I32,
            )],
            ret(Some(val(1))),
        ),
    ]));

    assert_eq!(stats.branches_folded, 1);
    assert!(matches!(
        f.blocks[0].terminator,
        Terminator::Branch(BlockId(1))
    ));

    // The phi is now single-entry and resolves to 222.
    assert_eq!(stats.phi_edges_pruned, 1);
    assert_eq!(materialised(&f, 1), Some(IrConst::I32(222)));
    assert!(
        !phi_incoming(&f, 2).iter().any(|(_, l)| *l == BlockId(0)),
        "the b0 -> b2 operand must be gone, found {:?}",
        phi_incoming(&f, 2)
    );
}

/// **F2, switch flavour.** A folded switch drops *several* edges at once; every
/// one of them needs the same repair.
#[test]
fn folding_a_switch_prunes_every_stale_phi_operand() {
    // b0 switches on 2 -> only b2 survives; b1 and b3 lose their b0 edge.
    // b4 joins all three arms.
    let (stats, f) = sccp(func_of(vec![
        blk(
            0,
            vec![i32c(0, 2)],
            Terminator::Switch {
                val: val(0),
                cases: vec![(1, BlockId(1)), (2, BlockId(2))],
                default: BlockId(3),
                ty: IrType::I32,
            },
        ),
        blk(1, vec![], br(4)),
        blk(2, vec![], br(4)),
        blk(3, vec![], br(4)),
        blk(
            4,
            vec![phi(
                1,
                vec![
                    (Operand::Const(IrConst::I32(10)), 1),
                    (Operand::Const(IrConst::I32(20)), 2),
                    (Operand::Const(IrConst::I32(30)), 3),
                ],
                IrType::I32,
            )],
            ret(Some(val(1))),
        ),
    ]));

    assert_eq!(stats.branches_folded, 1);
    assert_eq!(stats.phi_edges_pruned, 2, "the b1 and b3 arms are dead");
    // With only the b2 arm left the phi is constant, so it is materialised as a
    // Copy; either way, no operand may survive for a dead edge.
    assert!(
        phi_incoming(&f, 4).iter().all(|(_, l)| *l == BlockId(2)),
        "an operand survived for a dead edge: {:?}",
        phi_incoming(&f, 4)
    );
    assert_eq!(materialised(&f, 1), Some(IrConst::I32(20)));
}

/// A phi operand arriving from a block SCCP proved unreachable is pruned even
/// when no terminator was folded in the *predecessor* itself — the dead block
/// simply never became executable.
#[test]
fn operands_from_unreachable_predecessors_are_pruned() {
    let (stats, f) = sccp(func_of(vec![
        blk(0, vec![cst(0, IrConst::I64(42))], br(2)),
        blk(1, vec![], br(2)), // never reached
        blk(
            2,
            vec![phi(
                1,
                vec![(val(0), 0), (Operand::Const(IrConst::I64(99)), 1)],
                IrType::I64,
            )],
            ret(Some(val(1))),
        ),
    ]));

    assert_eq!(stats.unreachable_blocks, 1);
    assert_eq!(stats.phi_edges_pruned, 1);
    assert!(
        !phi_incoming(&f, 2).iter().any(|(_, l)| *l == BlockId(1)),
        "the dead b1 operand survived: {:?}",
        phi_incoming(&f, 2)
    );
    // Only the b0 arm remains, so the phi is constant and materialised.
    assert_eq!(materialised(&f, 1), Some(IrConst::I64(42)));
}

/// **F3 — `long double 0.0` is falsy.**
///
/// A hand-rolled "is this constant non-zero?" helper that treats every
/// non-`I*`/`F32`/`F64` constant as truthy takes the *true* edge for
/// `long double x = 0.0L; if (x)`. Routing through `IrConst::is_nonzero` — the
/// same predicate `cfg_simplify` uses — is what keeps the two passes from
/// folding the same branch in opposite directions.
#[test]
fn a_zero_long_double_condition_takes_the_false_edge() {
    let (_, f) = sccp(func_of(vec![
        blk(
            0,
            vec![cst(0, IrConst::long_double(0.0))],
            cond_br(val(0), 1, 2),
        ),
        blk(1, vec![], ret(Some(Operand::Const(IrConst::I32(1))))),
        blk(2, vec![], ret(Some(Operand::Const(IrConst::I32(0))))),
    ]));

    assert!(
        matches!(f.blocks[0].terminator, Terminator::Branch(BlockId(2))),
        "0.0L is false; expected the b2 edge, found {:?}",
        f.blocks[0].terminator
    );
}

#[test]
fn a_nonzero_long_double_condition_takes_the_true_edge() {
    let (_, f) = sccp(func_of(vec![
        blk(
            0,
            vec![cst(0, IrConst::long_double(1.5))],
            cond_br(val(0), 1, 2),
        ),
        blk(1, vec![], ret(None)),
        blk(2, vec![], ret(None)),
    ]));

    assert!(matches!(
        f.blocks[0].terminator,
        Terminator::Branch(BlockId(1))
    ));
}

/// **F4 — long double arithmetic keeps the target's precision.**
///
/// Folding `F128` through `f64` silently truncates the mantissa, so SCCP and
/// `constant_fold` would disagree about the *value* of the same expression.
/// Both now call one oracle, so this test pins them together: whatever
/// `eval_binop_const` says is what SCCP must produce, bit for bit.
#[test]
fn long_double_folding_matches_the_shared_oracle_bit_for_bit() {
    let a = IrConst::long_double(1.0);
    let b = IrConst::long_double(3.0);
    let expected = constant_fold::eval_binop_const(IrBinOp::SDiv, a, b, IrType::F128)
        .expect("the oracle folds 1.0L / 3.0L");

    let (_, f) = sccp(func_of(vec![blk(
        0,
        vec![
            cst(0, a),
            cst(1, b),
            Instruction::BinOp {
                dest: Value(2),
                op: IrBinOp::SDiv,
                lhs: val(0),
                rhs: val(1),
                ty: IrType::F128,
            },
        ],
        ret(Some(val(2))),
    )]));

    let got = materialised(&f, 2).expect("SCCP folds it too");
    assert_eq!(
        got.to_hash_key(),
        expected.to_hash_key(),
        "SCCP and constant_fold disagree about 1.0L / 3.0L"
    );
    // And the full 16-byte payload, not just the f64 shadow, must survive.
    assert_eq!(got.long_double_bytes(), expected.long_double_bytes());
}

/// **F20 — `asm goto` is control flow.**
///
/// `InlineAsm { goto_labels }` transfers control from an *instruction*, not
/// from the terminator. A pass that only walks terminators concludes the label
/// block is unreachable, then prunes its phi operands and lets `cfg_simplify`
/// delete it — miscompiling every `asm goto` in the Linux kernel.
#[test]
fn asm_goto_targets_stay_reachable() {
    let asm = Instruction::InlineAsm {
        template: "jmp %l0".to_string(),
        outputs: vec![],
        inputs: vec![],
        clobbers: vec![],
        operand_types: vec![],
        goto_labels: vec![("lbl".to_string(), BlockId(2))],
        input_symbols: vec![],
        seg_overrides: vec![],
    };

    let (stats, f) = sccp(func_of(vec![
        // b0 falls through to b1; b2 is reachable *only* via the asm goto.
        blk(0, vec![asm], br(1)),
        blk(1, vec![], br(3)),
        blk(2, vec![i32c(0, 7)], br(3)),
        blk(
            3,
            vec![phi(
                1,
                vec![(Operand::Const(IrConst::I32(5)), 1), (val(0), 2)],
                IrType::I32,
            )],
            ret(Some(val(1))),
        ),
    ]));

    assert_eq!(
        stats.unreachable_blocks, 0,
        "the asm goto target must be reachable"
    );
    assert_eq!(
        stats.phi_edges_pruned,
        0,
        "no phi operand may be dropped, found {:?}",
        phi_incoming(&f, 3)
    );
    assert_eq!(
        phi_incoming(&f, 3).len(),
        2,
        "both arms of the join must survive"
    );
    assert_eq!(
        materialised(&f, 1),
        None,
        "phi(5, 7) is not constant and must not be folded"
    );
}

/// **F23 — `__builtin_constant_p` must not answer "no" early.**
///
/// Resolving an unknown operand to `0` during SCCP is premature: inlining and
/// IPCP still run afterwards and can make it constant. `constant_fold::
/// resolve_remaining_is_constant` owns the negative answer, at the end of the
/// pipeline. Answering `1` for an operand already proven constant is monotone
/// and stays.
#[test]
fn builtin_constant_p_resolves_positively_but_never_negatively() {
    // Positive: the operand is a known constant -> 1.
    let (_, f) = sccp(func_of(vec![blk(
        0,
        vec![
            i32c(0, 5),
            Instruction::UnaryOp {
                dest: Value(1),
                op: IrUnaryOp::IsConstant,
                src: val(0),
                ty: IrType::I32,
            },
        ],
        ret(Some(val(1))),
    )]));
    assert_eq!(materialised(&f, 1), Some(IrConst::I32(1)));

    // Negative: the operand is unknown -> left alone for a later phase.
    let (_, f) = sccp(func_of(vec![blk(
        0,
        vec![
            Instruction::ParamRef {
                dest: Value(0),
                param_idx: 0,
                ty: IrType::I32,
            },
            Instruction::UnaryOp {
                dest: Value(1),
                op: IrUnaryOp::IsConstant,
                src: val(0),
                ty: IrType::I32,
            },
        ],
        ret(Some(val(1))),
    )]));
    assert_eq!(
        materialised(&f, 1),
        None,
        "SCCP must not decide __builtin_constant_p is 0"
    );
}

/// A value that is read but has no definition (a dangling reference left by an
/// earlier pass) must be `⊥`. Left at `⊤` it would be absorbed by the phi meet
/// and fabricate a constant out of nothing.
#[test]
fn a_dangling_value_reference_is_overdefined() {
    let (_, f) = sccp(func_of(vec![
        blk(
            0,
            vec![Instruction::ParamRef {
                dest: Value(9),
                param_idx: 0,
                ty: IrType::I32,
            }],
            cond_br(val(9), 1, 2),
        ),
        blk(1, vec![], br(3)),
        blk(2, vec![], br(3)),
        blk(
            3,
            // %5 is never defined anywhere.
            vec![phi(
                1,
                vec![(val(5), 1), (Operand::Const(IrConst::I32(3)), 2)],
                IrType::I32,
            )],
            ret(Some(val(1))),
        ),
    ]));

    assert_eq!(materialised(&f, 1), None);
}

// ═══════════════════════════════════════════════════════════════════════════
// Structural invariants
// ═══════════════════════════════════════════════════════════════════════════

/// Materialising a constant phi must not break the "all phis first" block
/// layout: the replacement `Copy` is spliced in after the phi prefix.
#[test]
fn constant_phis_are_materialised_without_splitting_the_phi_prefix() {
    let (_, f) = sccp(func_of(vec![
        blk(
            0,
            vec![Instruction::ParamRef {
                dest: Value(9),
                param_idx: 0,
                ty: IrType::I32,
            }],
            cond_br(val(9), 1, 2),
        ),
        blk(1, vec![], br(3)),
        blk(2, vec![], br(3)),
        blk(
            3,
            vec![
                // Constant phi: becomes a Copy.
                phi(
                    1,
                    vec![
                        (Operand::Const(IrConst::I32(7)), 1),
                        (Operand::Const(IrConst::I32(7)), 2),
                    ],
                    IrType::I32,
                ),
                // Non-constant phi: stays a phi and must remain first.
                phi(
                    2,
                    vec![
                        (Operand::Const(IrConst::I32(1)), 1),
                        (Operand::Const(IrConst::I32(2)), 2),
                    ],
                    IrType::I32,
                ),
                add(3, val(1), val(2), IrType::I32),
            ],
            ret(Some(val(3))),
        ),
    ]));

    let insts = &f.blocks[3].instructions;
    let first_non_phi = insts
        .iter()
        .position(|i| !matches!(i, Instruction::Phi { .. }))
        .unwrap();
    assert!(
        insts[first_non_phi..]
            .iter()
            .all(|i| !matches!(i, Instruction::Phi { .. })),
        "phis must stay contiguous at the top of the block: {insts:?}"
    );
    assert!(matches!(insts[0], Instruction::Phi { dest: Value(2), .. }));
    assert_eq!(materialised(&f, 1), Some(IrConst::I32(7)));
}

/// Division by zero is undefined behaviour and must not be folded to a value.
#[test]
fn division_by_zero_is_not_folded() {
    let (_, f) = sccp(func_of(vec![blk(
        0,
        vec![
            i32c(0, 10),
            i32c(1, 0),
            Instruction::BinOp {
                dest: Value(2),
                op: IrBinOp::SDiv,
                lhs: val(0),
                rhs: val(1),
                ty: IrType::I32,
            },
        ],
        ret(Some(val(2))),
    )]));

    assert_eq!(materialised(&f, 2), None, "10 / 0 must stay in the IR");
}

/// SCCP must reach the same answer as `constant_fold` on the sub-int promotion
/// rule, which depends on the *defining cast's* target type rather than on the
/// constant alone.
#[test]
fn sub_int_promotion_agrees_with_the_folder() {
    // %0 = (signed char)-1 ; %1 = ~%0  — the cast target decides sign extension.
    let (_, f) = sccp(func_of(vec![blk(
        0,
        vec![
            cst(0, IrConst::I32(-1)),
            Instruction::Cast {
                dest: Value(1),
                src: val(0),
                from_ty: IrType::I32,
                to_ty: IrType::I8,
            },
            Instruction::UnaryOp {
                dest: Value(2),
                op: IrUnaryOp::Not,
                src: val(1),
                ty: IrType::I32,
            },
        ],
        ret(Some(val(2))),
    )]));

    let cast_const = constant_fold::eval_cast_const(IrConst::I32(-1), IrType::I32, IrType::I8)
        .expect("the oracle folds the cast");
    let expected = constant_fold::eval_unaryop_const(
        IrUnaryOp::Not,
        cast_const,
        Some(IrType::I8),
        IrType::I32,
    )
    .expect("the oracle folds the negation");

    assert_eq!(
        materialised(&f, 2).map(|c| c.to_hash_key()),
        Some(expected.to_hash_key())
    );
}

/// NaN comparisons follow C's unordered semantics; only `!=` is true.
#[test]
fn nan_comparisons_use_unordered_semantics() {
    for (op, expected) in [
        (IrCmpOp::Eq, 0),
        (IrCmpOp::Ne, 1),
        (IrCmpOp::Slt, 0),
        (IrCmpOp::Sgt, 0),
    ] {
        let (_, f) = sccp(func_of(vec![blk(
            0,
            vec![
                cst(0, IrConst::F64(f64::NAN)),
                cst(1, IrConst::F64(1.0)),
                Instruction::Cmp {
                    dest: Value(2),
                    op,
                    lhs: val(0),
                    rhs: val(1),
                    ty: IrType::F64,
                },
            ],
            ret(Some(val(2))),
        )]));
        assert_eq!(
            materialised(&f, 2),
            Some(IrConst::I32(expected)),
            "NaN {op:?} 1.0"
        );
    }
}

/// `+0.0` and `-0.0` compare equal but are *different constants*: meeting them
/// in a phi must yield ⊥, because materialising either one would change the
/// sign of a subsequent division or `copysign`.
#[test]
fn signed_zeros_do_not_meet_to_a_single_constant() {
    let (_, f) = sccp(func_of(vec![
        blk(
            0,
            vec![Instruction::ParamRef {
                dest: Value(9),
                param_idx: 0,
                ty: IrType::I32,
            }],
            cond_br(val(9), 1, 2),
        ),
        blk(1, vec![], br(3)),
        blk(2, vec![], br(3)),
        blk(
            3,
            vec![phi(
                1,
                vec![
                    (Operand::Const(IrConst::F64(0.0)), 1),
                    (Operand::Const(IrConst::F64(-0.0)), 2),
                ],
                IrType::F64,
            )],
            ret(Some(val(1))),
        ),
    ]));

    assert_eq!(
        materialised(&f, 1),
        None,
        "+0.0 and -0.0 are distinct constants and must meet to bottom"
    );
}

/// An empty function must not panic, and neither must one whose entry block is
/// its only block.
#[test]
fn degenerate_functions_are_handled() {
    let mut empty = IrFunction::new("e".to_string(), IrType::Void, vec![], false);
    let ud = UseDefInfo::build(&empty);
    assert_eq!(run_with_usedef(&mut empty, &ud), SccpStats::default());

    let (_, f) = sccp(func_of(vec![blk(0, vec![], ret(None))]));
    assert_eq!(f.blocks.len(), 1);
}

/// Running SCCP twice must be a no-op the second time: the pass has to reach a
/// fixpoint, not oscillate. (A pass that reports changes forever spins the
/// driver's iteration loop until its budget runs out.)
#[test]
fn a_second_run_reports_no_further_changes() {
    let f = func_of(vec![
        blk(0, vec![i32c(0, 1)], cond_br(val(0), 1, 2)),
        blk(1, vec![i32c(1, 10)], br(3)),
        blk(2, vec![i32c(2, 20)], br(3)),
        blk(
            3,
            vec![phi(3, vec![(val(1), 1), (val(2), 2)], IrType::I32)],
            ret(Some(val(3))),
        ),
    ]);

    let (first, mut f) = sccp(f);
    assert!(first.total() > 0);

    let ud = UseDefInfo::build(&f);
    let second = run_with_usedef(&mut f, &ud);
    assert_eq!(
        second.total(),
        0,
        "SCCP is not idempotent; second run reported {second:?}"
    );
}

/// A `CondBranch` whose two arms are the same block folds without pruning
/// anything: the edge survives, just unconditionally.
#[test]
fn a_cond_branch_to_one_target_folds_without_losing_the_edge() {
    let (stats, f) = sccp(func_of(vec![
        blk(0, vec![i32c(0, 1)], cond_br(val(0), 1, 1)),
        blk(
            1,
            vec![phi(
                1,
                vec![(Operand::Const(IrConst::I32(4)), 0)],
                IrType::I32,
            )],
            ret(Some(val(1))),
        ),
    ]));

    assert!(matches!(
        f.blocks[0].terminator,
        Terminator::Branch(BlockId(1))
    ));
    assert_eq!(
        stats.phi_edges_pruned, 0,
        "the b0 -> b1 edge survives the fold; nothing may be pruned"
    );
    assert_eq!(materialised(&f, 1), Some(IrConst::I32(4)));
}

/// Deeply chained conditionals: SCCP must discover the constant through several
/// rounds of "fold a branch, which makes a phi constant, which folds the next
/// branch". This is the property that makes it stronger than fold-then-simplify.
#[test]
fn resolves_a_chain_of_dependent_conditionals() {
    //  b0: %0 = 1 ; br %0 ? b1 : b2
    //  b1: br b3          b2: br b3            (b2 dead)
    //  b3: %1 = phi[10 b1, 20 b2] ; %2 = (%1 == 10) ; br %2 ? b4 : b5
    //  b4: ret 1          b5: ret 0            (b5 dead)
    let (stats, f) = sccp(func_of(vec![
        blk(0, vec![i32c(0, 1)], cond_br(val(0), 1, 2)),
        blk(1, vec![], br(3)),
        blk(2, vec![], br(3)),
        blk(
            3,
            vec![
                phi(
                    1,
                    vec![
                        (Operand::Const(IrConst::I32(10)), 1),
                        (Operand::Const(IrConst::I32(20)), 2),
                    ],
                    IrType::I32,
                ),
                Instruction::Cmp {
                    dest: Value(2),
                    op: IrCmpOp::Eq,
                    lhs: val(1),
                    rhs: Operand::Const(IrConst::I32(10)),
                    ty: IrType::I32,
                },
            ],
            cond_br(val(2), 4, 5),
        ),
        blk(4, vec![], ret(Some(Operand::Const(IrConst::I32(1))))),
        blk(5, vec![], ret(Some(Operand::Const(IrConst::I32(0))))),
    ]));

    assert_eq!(materialised(&f, 1), Some(IrConst::I32(10)));
    assert_eq!(materialised(&f, 2), Some(IrConst::I32(1)));
    assert!(matches!(
        f.blocks[3].terminator,
        Terminator::Branch(BlockId(4))
    ));
    assert_eq!(stats.unreachable_blocks, 2, "b2 and b5 are dead");
}

// ═══════════════════════════════════════════════════════════════════════════
// Red-team audit round 2: absorption, reflexive folds, invariant 5, asm defs
// ═══════════════════════════════════════════════════════════════════════════

/// `BinOp` constructor with an explicit opcode (the `add` helper hardcodes one).
fn binop(dest: u32, op: IrBinOp, lhs: Operand, rhs: Operand, ty: IrType) -> Instruction {
    Instruction::BinOp {
        dest: Value(dest),
        op,
        lhs,
        rhs,
        ty,
    }
}

fn param(dest: u32, ty: IrType) -> Instruction {
    Instruction::ParamRef {
        dest: Value(dest),
        param_idx: 0,
        ty,
    }
}

#[test]
fn absorbing_and_with_zero_beats_an_overdefined_operand() {
    // %0 is a parameter (⊥); `%1 = and %0, 0` is nevertheless exactly 0, so the
    // return operand substitutes to a constant. GCC's CCP and LLVM's SCCP both
    // fold this; without the rule the whole tail of the function stays live.
    let (stats, f) = sccp(func_of(vec![blk(
        0,
        vec![
            param(0, IrType::I32),
            binop(
                1,
                IrBinOp::And,
                val(0),
                Operand::Const(IrConst::I32(0)),
                IrType::I32,
            ),
        ],
        ret(Some(val(1))),
    )]));

    assert!(stats.total() > 0);
    assert_eq!(materialised(&f, 1), Some(IrConst::I32(0)));
    assert!(matches!(
        f.blocks[0].terminator,
        Terminator::Return(Some(Operand::Const(IrConst::I32(0))))
    ));
}

#[test]
fn absorbing_mul_zero_covers_the_i128_width() {
    // 128-bit arithmetic is a first-class BinOp type in this IR; the zero rule
    // must not silently apply only to 32/64-bit constants.
    let (stats, f) = sccp(func_of(vec![blk(
        0,
        vec![
            param(0, IrType::I128),
            binop(
                1,
                IrBinOp::Mul,
                val(0),
                Operand::Const(IrConst::I128(0)),
                IrType::I128,
            ),
        ],
        ret(Some(val(1))),
    )]));

    assert!(stats.total() > 0);
    assert_eq!(materialised(&f, 1), Some(IrConst::I128(0)));
}

#[test]
fn the_type_agnostic_zero_const_is_absorbed_for_integer_tys() {
    // `IrConst::Zero` also appears as an integer BinOp operand; the oracle
    // normalises it through the BinOp's type.
    let (_, f) = sccp(func_of(vec![blk(
        0,
        vec![
            param(0, IrType::I32),
            binop(
                1,
                IrBinOp::And,
                val(0),
                Operand::Const(IrConst::Zero),
                IrType::I32,
            ),
        ],
        ret(Some(val(1))),
    )]));

    assert_eq!(materialised(&f, 1), Some(IrConst::I32(0)));
}

#[test]
fn float_mul_zero_is_never_absorbed() {
    // `NaN * 0.0` is NaN and `-x * 0.0` is `-0.0`: the absorbing rule must not
    // fire for float types, not even for a zero-looking operand.
    let (stats, f) = sccp(func_of(vec![blk(
        0,
        vec![
            param(0, IrType::F64),
            binop(
                1,
                IrBinOp::Mul,
                val(0),
                Operand::Const(IrConst::F64(0.0)),
                IrType::F64,
            ),
        ],
        ret(Some(val(1))),
    )]));

    assert_eq!(stats.total(), 0, "nothing may fold: the operand is opaque");
    assert_eq!(materialised(&f, 1), None);
}

#[test]
fn absorbing_or_with_all_ones_beats_an_overdefined_operand() {
    // `x | -1 == -1` for any x: the mirror image of the zero rule for Or.
    let (stats, f) = sccp(func_of(vec![blk(
        0,
        vec![
            param(0, IrType::I64),
            binop(
                1,
                IrBinOp::Or,
                val(0),
                Operand::Const(IrConst::I64(-1)),
                IrType::I64,
            ),
        ],
        ret(Some(val(1))),
    )]));

    assert!(stats.total() > 0);
    assert_eq!(materialised(&f, 1), Some(IrConst::I64(-1)));
}

#[test]
fn same_value_xor_and_sub_are_zero() {
    // Both operands are the *same* SSA value, so `x ^ x` and `x - x` are 0 in
    // every integer width regardless of what x is.
    let (_, f) = sccp(func_of(vec![blk(
        0,
        vec![
            param(0, IrType::I64),
            binop(1, IrBinOp::Xor, val(0), val(0), IrType::I64),
            binop(2, IrBinOp::Sub, val(0), val(0), IrType::I64),
        ],
        ret(Some(val(1))),
    )]));

    assert_eq!(materialised(&f, 1), Some(IrConst::I64(0)));
    assert_eq!(materialised(&f, 2), Some(IrConst::I64(0)));
}

#[test]
fn reflexive_integer_comparison_folds_and_kills_the_dead_arm() {
    // %1 = cmp eq %p, %p is exactly 1, so the branch folds to the true arm and
    // the false arm becomes unreachable.
    let (stats, f) = sccp(func_of(vec![
        blk(
            0,
            vec![
                param(0, IrType::I64),
                Instruction::Cmp {
                    dest: Value(1),
                    op: IrCmpOp::Eq,
                    lhs: val(0),
                    rhs: val(0),
                    ty: IrType::I64,
                },
            ],
            cond_br(val(1), 1, 2),
        ),
        blk(1, vec![], ret(Some(Operand::Const(IrConst::I32(10))))),
        blk(2, vec![], ret(Some(Operand::Const(IrConst::I32(20))))),
    ]));

    assert_eq!(materialised(&f, 1), Some(IrConst::I32(1)));
    assert_eq!(stats.branches_folded, 1);
    assert!(matches!(
        f.blocks[0].terminator,
        Terminator::Branch(BlockId(1))
    ));
    assert_eq!(stats.unreachable_blocks, 1, "the false arm is dead");
}

#[test]
fn reflexive_float_comparison_is_never_folded() {
    // `x == x` is false exactly when x is NaN: for float types the same-value
    // rule must not fire.
    let (_, f) = sccp(func_of(vec![blk(
        0,
        vec![
            param(0, IrType::F64),
            Instruction::Cmp {
                dest: Value(1),
                op: IrCmpOp::Eq,
                lhs: val(0),
                rhs: val(0),
                ty: IrType::F64,
            },
        ],
        ret(Some(val(1))),
    )]));

    assert_eq!(materialised(&f, 1), None);
}

#[test]
fn an_unresolved_top_condition_keeps_both_successors_executable() {
    // Invariant 5, the shape that miscompiled before the safety net existed:
    //
    //   b0: cond_br %p -> b1, b3          %p is a parameter (⊥): both arms live
    //   b1: %1 = phi [7 from b2]          only incoming edge is b2->b1, which
    //       cond_br %1 -> b2, b3          never executes => %1 survives as ⊤
    //   b2: br b1
    //   b3: %2 = phi [%p from b0, 77 from b1] ; ret %2
    //
    // Without the forcing round, b1's terminator marks neither successor, so
    // the b1->b3 edge is "dead" and b3's phi loses the `77` operand — while at
    // runtime b1 still branches (to whichever arm), reaching b3 with no copy
    // for the pruned edge. The safety net lowers %1 to ⊥ instead, both
    // successors stay executable, and nothing is pruned.
    let (_, f) = sccp(func_of(vec![
        blk(0, vec![param(0, IrType::I64)], cond_br(val(0), 1, 3)),
        blk(
            1,
            vec![phi(
                1,
                vec![(Operand::Const(IrConst::I64(7)), 2)],
                IrType::I64,
            )],
            cond_br(val(1), 2, 3),
        ),
        blk(2, vec![], br(1)),
        blk(
            3,
            vec![phi(
                2,
                vec![(val(0), 0), (Operand::Const(IrConst::I64(77)), 1)],
                IrType::I64,
            )],
            ret(Some(val(2))),
        ),
    ]));

    let incoming = phi_incoming(&f, 3);
    assert_eq!(incoming.len(), 2, "both b3 phi operands must survive");
    assert!(incoming
        .iter()
        .any(|(op, _)| matches!(op, Operand::Value(_))));
    assert!(
        incoming
            .iter()
            .any(|(op, _)| matches!(op, Operand::Const(IrConst::I64(77)))),
        "the operand arriving over the recovered edge must survive"
    );
}

#[test]
fn inline_asm_outputs_are_overdefined_not_fabricated() {
    // An `InlineAsm` output is a definition `dest()` does not cover. If it
    // ever stayed ⊤, the phi below would absorb the ⊤ arm and fabricate the
    // constant `5` as the value of an opaque asm result.
    let asm = Instruction::InlineAsm {
        template: "mov $1, %l0".to_string(),
        outputs: vec![("=r".to_string(), Value(1), None)],
        inputs: vec![],
        clobbers: vec![],
        operand_types: vec![IrType::I32],
        goto_labels: vec![],
        input_symbols: vec![],
        seg_overrides: vec![],
    };

    let (_, f) = sccp(func_of(vec![
        blk(
            0,
            vec![param(0, IrType::I32), i32c(10, 5)],
            cond_br(val(0), 1, 2),
        ),
        blk(1, vec![asm], br(3)),
        blk(2, vec![], br(3)),
        blk(
            3,
            vec![phi(2, vec![(val(1), 1), (val(10), 2)], IrType::I32)],
            ret(Some(val(2))),
        ),
    ]));

    assert_eq!(
        materialised(&f, 2),
        None,
        "an asm result must never be fabricated into a constant"
    );
}
