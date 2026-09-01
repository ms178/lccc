//! IR verifier tests.
//!
//! The first group pins each structural invariant with the *minimal* malformed
//! IR that violates it. The last test reconstructs the exact shape the real
//! `loop_rotate` defect produced, so the verifier is proven to catch it.

use super::*;
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
    // 0 -> {1, 2} -> 3, with a phi merging both arms.
    let f = func_of(vec![
        blk(0, vec![], condbr(9, 1, 2)),
        blk(1, vec![], br(3)),
        blk(2, vec![], br(3)),
        blk(3, vec![phi(10, vec![(c0(), 1), (v(11), 2)])], ret()),
    ]);
    assert_clean(&f);
}

#[test]
fn a_self_loop_phi_naming_its_own_block_is_clean() {
    // A rotated loop body is its own predecessor; that must not be flagged.
    let f = func_of(vec![
        blk(0, vec![], br(1)),
        blk(1, vec![phi(10, vec![(c0(), 0), (v(11), 1)])], condbr(9, 1, 2)),
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
fn a_missing_incoming_for_an_UNREACHABLE_predecessor_is_tolerated() {
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
fn a_missing_incoming_for_a_REACHABLE_predecessor_is_still_reported() {
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
fn a_stale_predecessor_inside_an_UNREACHABLE_block_is_tolerated() {
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
    let fixed = func_of(vec![
        blk(6, vec![], br(7)),
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
