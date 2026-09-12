//! IR verifier tests.
//!
//! The first group pins each structural invariant with the *minimal* malformed
//! IR that violates it. The last test reconstructs the exact shape the real
//! `loop_rotate` defect produced, so the verifier is proven to catch it.

use super::*;
use crate::common::types::AddressSpace;
use crate::common::types::IrType;
use crate::ir::reexports::*;

// ── helpers ─────────────────────────────────────────────────────────────────

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
    // Well-formed IR honors the documented `next_label` invariant (every
    // live label < next_label); fixtures must too, or the counter-health
    // check fires on every test.
    let max_label = f.blocks.iter().map(|b| b.label.0).max().unwrap_or(0);
    f.next_label = max_label + 1;
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

fn phi(dest: u32, incoming: Vec<(Operand, u32)>) -> Instruction {
    Instruction::Phi {
        dest: Value(dest),
        ty: IrType::I32,
        incoming: incoming
            .into_iter()
            .map(|(op, l)| (op, BlockId(l)))
            .collect(),
    }
}

fn c0() -> Operand {
    Operand::Const(IrConst::I32(0))
}
fn v(n: u32) -> Operand {
    Operand::Value(Value(n))
}

/// `vN = 0` — the minimal definition, for fixtures that need their uses
/// dominated. Every value a well-formed function consumes must be defined
/// somewhere; check 8 rejects uses of undefined value ids.
fn def(n: u32) -> Instruction {
    Instruction::Copy {
        dest: Value(n),
        src: c0(),
    }
}

fn br(t: u32) -> Terminator {
    Terminator::Branch(BlockId(t))
}

fn condbr(cond: u32, t: u32, f: u32) -> Terminator {
    Terminator::CondBranch {
        cond: Operand::Value(Value(cond)),
        true_label: BlockId(t),
        false_label: BlockId(f),
    }
}

fn ret() -> Terminator {
    Terminator::Return(None)
}

fn check(f: &IrFunction) -> Vec<String> {
    let mut out = Vec::new();
    verify_function(f, "test", &mut out);
    out.into_iter().map(|v| v.detail).collect()
}

fn assert_clean(f: &IrFunction) {
    let v = check(f);
    assert!(v.is_empty(), "expected well-formed IR, got: {:#?}", v);
}

fn assert_reports(f: &IrFunction, needle: &str) {
    let v = check(f);
    assert!(
        v.iter().any(|d| d.contains(needle)),
        "expected a violation containing {:?}, got: {:#?}",
        needle,
        v
    );
}

// ── well-formed IR is accepted ──────────────────────────────────────────────

#[test]
fn a_well_formed_diamond_with_a_phi_is_clean() {
    // 0 -> {1, 2} -> 3, with a phi merging both arms. v9 (the branch
    // condition) and v11 (the arm-2 incoming) are defined in the entry,
    // which dominates every use.
    let f = func_of(vec![
        blk(0, vec![def(9), def(11)], condbr(9, 1, 2)),
        blk(1, vec![], br(3)),
        blk(2, vec![], br(3)),
        blk(3, vec![phi(10, vec![(c0(), 1), (v(11), 2)])], ret()),
    ]);
    assert_clean(&f);
}

#[test]
fn a_self_loop_phi_naming_its_own_block_is_clean() {
    // A rotated loop body is its own predecessor; that must not be flagged.
    // v11 is defined in the entry, which dominates the self edge's block.
    let f = func_of(vec![
        blk(0, vec![def(9), def(11)], br(1)),
        blk(
            1,
            vec![phi(10, vec![(c0(), 0), (v(11), 1)])],
            condbr(9, 1, 2),
        ),
        blk(2, vec![], ret()),
    ]);
    assert_clean(&f);
}

#[test]
fn an_empty_function_is_clean() {
    let f = func_of(vec![]);
    assert_clean(&f);
}

// ── each invariant, pinned individually ─────────────────────────────────────

#[test]
fn a_duplicate_block_label_is_reported() {
    let f = func_of(vec![
        blk(0, vec![], br(1)),
        blk(1, vec![], ret()),
        blk(1, vec![], ret()),
    ]);
    assert_reports(&f, "duplicate block label");
}

#[test]
fn a_terminator_targeting_a_missing_block_is_reported() {
    let f = func_of(vec![blk(0, vec![], br(7))]);
    assert_reports(&f, "targets unknown block");
}

#[test]
fn a_phi_after_a_non_phi_instruction_is_reported() {
    let f = func_of(vec![
        blk(0, vec![], br(1)),
        blk(
            1,
            vec![
                Instruction::Copy {
                    dest: Value(20),
                    src: c0(),
                },
                phi(10, vec![(c0(), 0)]),
            ],
            ret(),
        ),
    ]);
    assert_reports(&f, "phi appears after a non-phi instruction");
}

#[test]
fn a_duplicated_phi_predecessor_is_reported() {
    let f = func_of(vec![
        blk(0, vec![], br(1)),
        blk(1, vec![phi(10, vec![(c0(), 0), (v(11), 0)])], ret()),
    ]);
    assert_reports(&f, "more than once");
}

#[test]
fn a_phi_missing_an_incoming_for_a_real_predecessor_is_reported() {
    // Both 1 and 2 reach 3, but the phi only covers 1.
    let f = func_of(vec![
        blk(0, vec![], condbr(9, 1, 2)),
        blk(1, vec![], br(3)),
        blk(2, vec![], br(3)),
        blk(3, vec![phi(10, vec![(c0(), 1)])], ret()),
    ]);
    assert_reports(&f, "has no incoming for predecessor");
}

#[test]
fn an_asm_goto_edge_counts_as_a_real_predecessor() {
    // A block reachable only via `asm goto` must not be reported as missing a
    // predecessor, and a phi naming that edge must be accepted. This mirrors
    // the SCCP `asm goto` finding (F20): implicit CFG edges are real edges.
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
    let f = func_of(vec![
        blk(0, vec![asm], br(1)),
        blk(1, vec![], ret()),
        blk(2, vec![phi(10, vec![(c0(), 0)])], ret()),
    ]);
    assert_clean(&f);
}

#[test]
fn a_missing_incoming_for_an_unreachable_predecessor_is_tolerated() {
    // Block 2 branches to 3 but nothing branches to 2, so the edge 2 -> 3 can
    // never execute and needs no phi operand. Reporting it would bury the
    // reachable cases, which are the ones that miscompile.
    let f = func_of(vec![
        blk(0, vec![], br(3)),
        blk(1, vec![], ret()),
        blk(2, vec![], br(3)),
        blk(3, vec![phi(10, vec![(c0(), 0)])], ret()),
    ]);
    assert_clean(&f);
}

#[test]
fn a_missing_incoming_for_a_reachable_predecessor_is_still_reported() {
    // Same shape, except block 2 is now reachable from the entry, so the
    // 2 -> 3 edge really executes and the phi leaves its register undefined
    // on that path.
    let f = func_of(vec![
        blk(0, vec![], condbr(9, 2, 3)),
        blk(1, vec![], ret()),
        blk(2, vec![], br(3)),
        blk(3, vec![phi(10, vec![(c0(), 0)])], ret()),
    ]);
    assert_reports(&f, "has no incoming for predecessor");
}

#[test]
fn a_stale_predecessor_inside_an_unreachable_block_is_tolerated() {
    // Block 2 is unreachable, so its phi is dead by construction. SCCP folds a
    // constant Switch to a single Branch and deliberately leaves the blocks it
    // orphaned untouched, documenting that cfg_simplify will delete them --
    // flagging that would punish correct behaviour.
    let f = func_of(vec![
        blk(0, vec![], br(3)),
        blk(1, vec![], br(2)),
        blk(2, vec![phi(10, vec![(c0(), 1), (v(11), 9)])], br(3)),
        blk(3, vec![], ret()),
    ]);
    // Block 9 does not exist, and block 1 is unreachable: neither is reported.
    assert_clean(&f);
}

#[test]
fn phi_contiguity_is_checked_even_in_an_unreachable_block() {
    // Contiguity is NOT gated on reachability: passes index the phi prefix
    // arithmetically without checking whether the block can execute, so the
    // invariant has to hold everywhere.
    let f = func_of(vec![
        blk(0, vec![], br(2)),
        blk(
            1,
            vec![
                Instruction::Copy {
                    dest: Value(20),
                    src: c0(),
                },
                phi(10, vec![(c0(), 0)]),
            ],
            br(2),
        ),
        blk(2, vec![], ret()),
    ]);
    assert_reports(&f, "phi appears after a non-phi instruction");
}

// ── the defect this module was built for ────────────────────────────────────

#[test]
fn the_loop_rotate_stale_guard_label_is_caught() {
    // Exactly the shape `loop_rotate` emitted before the fix:
    //
    //   .LBB6: Branch(.LBB7)
    //   .LBB7: phi v22 = [(0, .LBB6)]                 ; guard
    //   .LBB8: phi v72 = [(0, .LBB6), (v16, .LBB8)]   ; body -- .LBB6 is NOT a pred
    //   .LBB10: exit
    //
    // The body's only entry edge is guard -> body, so naming the preheader is
    // malformed. Consumers that trust the predecessor list delete the init.
    let f = func_of(vec![
        blk(6, vec![], br(7)),
        blk(7, vec![phi(22, vec![(c0(), 6)])], condbr(4, 8, 10)),
        blk(
            8,
            vec![phi(72, vec![(c0(), 6), (v(16), 8)])],
            condbr(73, 8, 10),
        ),
        blk(10, vec![], ret()),
    ]);

    let found = check(&f);
    assert!(
        found
            .iter()
            .any(|d| d.contains("phi v72") && d.contains("is not a predecessor")),
        "verifier missed the stale guard label: {:#?}",
        found
    );

    // And the corrected shape -- init labelled with the guard -- is clean.
    // The conditions (v4, v73) and the latch value (v16) are defined in the
    // entry block, so every use is dominated.
    let fixed = func_of(vec![
        blk(6, vec![def(4), def(16), def(73)], br(7)),
        blk(7, vec![phi(22, vec![(c0(), 6)])], condbr(4, 8, 10)),
        blk(
            8,
            vec![phi(72, vec![(c0(), 7), (v(16), 8)])],
            condbr(73, 8, 10),
        ),
        blk(10, vec![], ret()),
    ]);
    assert_clean(&fixed);
}

// ── def-dominates-use (checks 7 & 8) ────────────────────────────────────────

#[test]
fn a_use_before_its_definition_in_the_same_block_is_reported() {
    // v9 is consumed by the instruction that precedes its definition: the
    // block-level dominance holds, but the textual order does not.
    let f = func_of(vec![blk(
        0,
        vec![
            Instruction::Copy {
                dest: Value(10),
                src: v(9),
            },
            def(9),
        ],
        ret(),
    )]);
    assert_reports(&f, "does not dominate its use");
}

#[test]
fn a_definition_in_one_diamond_arm_does_not_dominate_the_join() {
    // v11 exists only on the left path; the join consumes it regardless.
    let f = func_of(vec![
        blk(0, vec![def(9)], condbr(9, 1, 2)),
        blk(1, vec![def(11)], br(3)),
        blk(2, vec![], br(3)),
        blk(
            3,
            vec![Instruction::Copy {
                dest: Value(12),
                src: v(11),
            }],
            ret(),
        ),
    ]);
    assert_reports(&f, "does not dominate its use");
}

#[test]
fn a_definition_in_the_entry_dominates_arms_and_join() {
    // Same diamond with the definition hoisted to the entry: clean.
    let f = func_of(vec![
        blk(0, vec![def(9), def(11)], condbr(9, 1, 2)),
        blk(1, vec![], br(3)),
        blk(2, vec![], br(3)),
        blk(
            3,
            vec![Instruction::Copy {
                dest: Value(12),
                src: v(11),
            }],
            ret(),
        ),
    ]);
    assert_clean(&f);
}

#[test]
fn a_phi_incoming_whose_value_does_not_dominate_its_edge_is_reported() {
    // The phi takes v12 "from arm 2", but v12 is defined in arm 1, which
    // does not dominate arm 2. This is the shape phi elimination would
    // silently satisfy from whatever register happens to be live.
    let f = func_of(vec![
        blk(0, vec![def(9)], condbr(9, 1, 2)),
        blk(1, vec![def(11), def(12)], br(3)),
        blk(2, vec![], br(3)),
        blk(3, vec![phi(10, vec![(v(11), 1), (v(12), 2)])], ret()),
    ]);
    assert_reports(&f, "does not dominate");
}

#[test]
fn a_loop_carried_phi_fed_from_the_latch_is_clean() {
    // v11 is defined in the body and consumed by the header phi on the
    // latch edge: the header dominates the body, so the def dominates the
    // edge-end where the value is copied.
    let f = func_of(vec![
        blk(0, vec![def(9)], br(1)),
        blk(
            1,
            vec![phi(10, vec![(c0(), 0), (v(11), 2)])],
            condbr(9, 2, 3),
        ),
        blk(2, vec![def(11)], br(1)),
        blk(3, vec![], ret()),
    ]);
    assert_clean(&f);
}

#[test]
fn a_phi_result_used_later_in_its_own_block_is_clean() {
    let f = func_of(vec![
        blk(0, vec![def(9), def(11)], br(1)),
        blk(
            1,
            vec![
                phi(10, vec![(c0(), 0), (v(11), 1)]),
                Instruction::Copy {
                    dest: Value(12),
                    src: v(10),
                },
            ],
            condbr(9, 1, 2),
        ),
        blk(2, vec![], ret()),
    ]);
    assert_clean(&f);
}

#[test]
fn a_use_of_an_undefined_value_is_reported() {
    let f = func_of(vec![blk(
        0,
        vec![Instruction::Copy {
            dest: Value(10),
            src: v(99),
        }],
        ret(),
    )]);
    assert_reports(&f, "use of undefined value v99");
}

#[test]
fn a_value_defined_twice_is_reported() {
    let f = func_of(vec![blk(0, vec![def(10), def(10)], ret())]);
    assert_reports(&f, "v10 defined more than once");
}

#[test]
fn a_use_of_a_value_defined_only_in_an_unreachable_block_is_reported() {
    // Block 1 is never entered, so on every executed path the join reads an
    // uninitialised register. The report names the dead *definition*.
    let f = func_of(vec![
        blk(0, vec![], br(2)),
        blk(1, vec![def(11)], br(2)),
        blk(
            2,
            vec![Instruction::Copy {
                dest: Value(12),
                src: v(11),
            }],
            ret(),
        ),
    ]);
    assert_reports(&f, "defined in unreachable block");
}

#[test]
fn a_terminator_use_of_a_non_dominating_definition_is_reported() {
    // Block 2 is reachable directly from the entry, so the definition in
    // block 1 does not dominate block 2's terminator condition.
    let f = func_of(vec![
        blk(0, vec![def(9)], condbr(9, 1, 2)),
        blk(1, vec![def(11)], br(2)),
        blk(2, vec![], condbr(11, 3, 3)),
        blk(3, vec![], ret()),
    ]);
    assert_reports(&f, "does not dominate its use");
}

#[test]
fn dominance_over_a_two_level_loop_nest_is_exact() {
    // Entry -> outer header -> inner header -> inner latch -> inner header
    //                    ^                          |
    //                    +------- outer latch ------+
    // The outer header dominates everything below it; the inner body's
    // definition feeds the inner header's phi, and an entry definition is
    // used after the nest. All uses are dominated: clean.
    let f = func_of(vec![
        blk(0, vec![def(9), def(20)], br(1)),
        // outer header: j = phi(0, j')
        blk(1, vec![phi(21, vec![(c0(), 0), (v(30), 4)])], br(2)),
        // inner header: i = phi(0, i')
        blk(
            2,
            vec![phi(22, vec![(c0(), 1), (v(23), 3)])],
            condbr(9, 3, 4),
        ),
        // inner body + latch: i' = i + 1 shape (copy stands in)
        blk(3, vec![def(23), def(24)], br(2)),
        // outer latch: j' = j + 1 shape, branch back
        blk(4, vec![def(30)], condbr(9, 1, 5)),
        blk(
            5,
            vec![Instruction::Copy {
                dest: Value(31),
                src: v(20),
            }],
            ret(),
        ),
    ]);
    assert_clean(&f);
}

#[test]
fn a_definition_inside_the_inner_loop_does_not_dominate_after_the_nest() {
    // Same nest, except the exit uses v24, which is defined only in the
    // inner body: the outer header does not dominate it... in fact the
    // inner header DOES dominate the inner body, and the exit is dominated
    // by the outer header, which is *above* the inner body. The use after
    // the nest is therefore not dominated by the inner-body def.
    let f = func_of(vec![
        blk(0, vec![def(9)], br(1)),
        blk(1, vec![phi(21, vec![(c0(), 0), (v(30), 4)])], br(2)),
        blk(
            2,
            vec![phi(22, vec![(c0(), 1), (v(23), 3)])],
            condbr(9, 3, 4),
        ),
        blk(3, vec![def(23), def(24)], br(2)),
        blk(4, vec![def(30)], condbr(9, 1, 5)),
        blk(
            5,
            vec![Instruction::Copy {
                dest: Value(31),
                src: v(24),
            }],
            ret(),
        ),
    ]);
    assert_reports(&f, "does not dominate its use");
}

#[test]
fn an_inline_asm_output_is_a_definition_dominating_later_uses() {
    // `dest()` returns None for InlineAsm, yet the output slots are SSA
    // definitions that later instructions read directly. The verifier must
    // model them as defs (and must NOT count the output slots as uses).
    let asm = Instruction::InlineAsm {
        template: "cpuid".to_string(),
        outputs: vec![("=a".to_string(), Value(11), None)],
        inputs: vec![("1".to_string(), c0(), None)],
        clobbers: vec![],
        operand_types: vec![],
        goto_labels: vec![],
        input_symbols: vec![],
        seg_overrides: vec![],
    };
    let f = func_of(vec![blk(
        0,
        vec![
            asm,
            Instruction::Copy {
                dest: Value(12),
                src: v(11),
            },
        ],
        ret(),
    )]);
    assert_clean(&f);
}

#[test]
fn a_use_of_an_asm_output_before_the_asm_is_reported() {
    let asm = Instruction::InlineAsm {
        template: "cpuid".to_string(),
        outputs: vec![("=a".to_string(), Value(11), None)],
        inputs: vec![("1".to_string(), c0(), None)],
        clobbers: vec![],
        operand_types: vec![],
        goto_labels: vec![],
        input_symbols: vec![],
        seg_overrides: vec![],
    };
    let f = func_of(vec![blk(
        0,
        vec![
            Instruction::Copy {
                dest: Value(12),
                src: v(11),
            },
            asm,
        ],
        ret(),
    )]);
    assert_reports(&f, "does not dominate its use");
}

#[test]
fn a_read_write_asm_output_naming_an_alloca_is_a_memory_home_not_a_redefinition() {
    // `asm("..." : "+a"(x))` names x's stack home: the Alloca stays the
    // single definition, the asm slot is a pointer use, and the later Load
    // reads the written-back value. This is the asm_alternative_length_
    // template shape.
    let asm = Instruction::InlineAsm {
        template: "movq %rax, %rax".to_string(),
        outputs: vec![("+a".to_string(), Value(0), None)],
        inputs: vec![("a".to_string(), Operand::Value(Value(2)), None)],
        clobbers: vec![],
        operand_types: vec![],
        goto_labels: vec![],
        input_symbols: vec![],
        seg_overrides: vec![],
    };
    let f = func_of(vec![blk(
        0,
        vec![
            Instruction::Alloca {
                dest: Value(0),
                ty: IrType::U64,
                size: 8,
                align: 0,
                volatile: false,
                semantic_volatile: false,
            },
            Instruction::ParamRef {
                dest: Value(1),
                param_idx: 0,
                ty: IrType::U64,
            },
            Instruction::Store {
                val: Operand::Value(Value(1)),
                ptr: Value(0),
                ty: IrType::U64,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
            Instruction::Load {
                dest: Value(2),
                ptr: Value(0),
                ty: IrType::U64,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
            asm,
            Instruction::Load {
                dest: Value(3),
                ptr: Value(0),
                ty: IrType::U64,
                seg_override: AddressSpace::Default,
                volatile: false,
            },
        ],
        Terminator::Return(Some(Operand::Value(Value(3)))),
    )]);
    assert_clean(&f);
}

#[test]
fn stale_label_counter_is_reported() {
    // strcmp-1: loop_unroll minted blocks without writing back `next_label`,
    // so a later pass (loop_memset) re-minted a live label, duplicating
    // BlockId(50) and detaching the memset guard chain. The duplicate-label
    // check blames the later pass; the counter-health check names the pass
    // that actually broke the invariant.
    let mut f = func_of(vec![
        blk(0, vec![], Terminator::Branch(BlockId(7))),
        blk(7, vec![], Terminator::Return(None)),
    ]);
    // func_of establishes a healthy counter; stale it deliberately.
    f.next_label = 7;
    assert_reports(&f, "stale label counter");
}

#[test]
fn healthy_label_counter_is_clean() {
    let f = func_of(vec![
        blk(0, vec![], Terminator::Branch(BlockId(7))),
        blk(7, vec![], Terminator::Return(None)),
    ]);
    assert_eq!(f.next_label, 8);
    assert_clean(&f);
}
