//! Evidence for eliding a fallthrough branch from an address-observable block.
//!
//! The caller supplies only the bytes emitted after that block's label, not the
//! module tail. Instructions outside inline asm are compiler-authored and emit
//! bytes on every target. Directives are NOT such evidence: alignment, `.space`
//! and `.fill` can all emit zero bytes. Unknown directives may change sections,
//! suppress assembly, or expand macros, so they invalidate the entire proof.
//!
//! This is deliberately an output-level proof, not an IR instruction count:
//! folding, deferred emission and register-resident values can emit no bytes.

pub(super) fn has_separating_instruction(body: &str) -> bool {
    let mut instruction = false;
    for line in body.lines() {
        let text = line.trim();
        if text.split_whitespace().next() == Some("#APP") || text.contains(';') {
            return false;
        }
        if text.is_empty() || text.starts_with('#') || text.starts_with("//") {
            continue;
        }
        let Some(token) = text.split_whitespace().next() else {
            continue;
        };
        if token.ends_with(':') {
            // Includes labels with trailing comments and numeric local labels.
            continue;
        }
        if token.starts_with('.') {
            // These directives cannot rewind/switch sections or suppress the
            // instructions surrounding them. None alone proves positive size.
            if token.starts_with(".cfi_")
                || matches!(
                    token,
                    ".loc"
                        | ".file"
                        | ".local"
                        | ".globl"
                        | ".global"
                        | ".hidden"
                        | ".type"
                        | ".size"
                        | ".align"
                        | ".balign"
                        | ".p2align"
                        | ".space"
                        | ".skip"
                        | ".fill"
                )
            {
                continue;
            }
            return false;
        }
        instruction = true;
    }
    instruction
}

/// The blocks whose label nothing names.
///
/// One way leads into such a block: the `Branch` that ends the block laid out
/// just ahead of it.  `emit_branch_to_block` prints no jump for a branch into
/// the block that follows, so no jump names the label, and a label is all a
/// block boundary is in the assembly the peepholes read.  They cannot tell a
/// label nothing jumps to from one a jump arrives at, and forget what they
/// know at each: an asm goto ends its block, the value it wrote was stored to
/// its home ahead of the label and loaded back behind it, where the asm goto's
/// own register still held it -- a store and a load per iteration of every
/// `copy_from_kernel_nofault()` loop.
///
/// `observable` are the labels something names without a jump printed by a
/// terminator: `&&label`, the labels of an asm goto, and those a static
/// initializer holds.  Whether the jump really was left out is not known
/// here: the block driver asks `branch_elided_into_next_block` for that (the
/// jump out of an empty block whose address is taken stays).
pub(super) fn blocks_entered_only_by_falling_in(
    func: &crate::ir::reexports::IrFunction,
    observable: &crate::common::fx_hash::FxHashMap<crate::ir::reexports::BlockId, bool>,
) -> crate::common::fx_hash::FxHashSet<crate::ir::reexports::BlockId> {
    use crate::ir::reexports::Terminator;
    // The edges into each candidate; most functions have none, and then
    // nothing is counted.
    let mut edges_in: crate::common::fx_hash::FxHashMap<_, u32> = func
        .blocks
        .windows(2)
        .filter_map(|pair| match pair[0].terminator {
            Terminator::Branch(target)
                if target == pair[1].label && !observable.contains_key(&target) =>
            {
                Some((target, 0))
            }
            _ => None,
        })
        .collect();
    if edges_in.is_empty() {
        return Default::default();
    }
    for block in &func.blocks {
        block.terminator.for_each_target(|target| {
            if let Some(count) = edges_in.get_mut(&target) {
                *count += 1;
            }
        });
    }
    edges_in
        .into_iter()
        .filter_map(|(label, count)| (count == 1).then_some(label))
        .collect()
}

#[cfg(test)]
mod fall_in_tests {
    use super::blocks_entered_only_by_falling_in;
    use crate::common::fx_hash::FxHashMap;
    use crate::common::types::IrType;
    use crate::ir::reexports::{
        BasicBlock, BlockId, Instruction, IrConst, IrFunction, Operand, Terminator,
    };

    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        }
    }
    fn jump(label: u32) -> Terminator {
        Terminator::Branch(BlockId(label))
    }
    fn cond(yes: u32, no: u32) -> Terminator {
        Terminator::CondBranch {
            cond: Operand::Const(IrConst::I32(1)),
            true_label: BlockId(yes),
            false_label: BlockId(no),
        }
    }
    fn ret() -> Terminator {
        Terminator::Return(None)
    }
    fn asm_goto_to(label: u32) -> Instruction {
        Instruction::InlineAsm {
            template: "jmp %l0".into(),
            outputs: vec![],
            inputs: vec![],
            clobbers: vec![],
            operand_types: vec![],
            goto_labels: vec![("out".into(), BlockId(label))],
            input_symbols: vec![],
            seg_overrides: vec![],
        }
    }
    /// The labels found for `blocks`, sorted; `observable` as the block
    /// driver collects them, plus `taken` (labels whose address is taken).
    fn found(blocks: Vec<BasicBlock>, taken: &[u32]) -> Vec<u32> {
        let mut func = IrFunction::new("f".to_string(), IrType::Void, vec![], false);
        func.blocks = blocks;
        let mut observable: FxHashMap<BlockId, bool> =
            taken.iter().map(|&label| (BlockId(label), true)).collect();
        for b in &func.blocks {
            for inst in &b.instructions {
                if let Instruction::InlineAsm { goto_labels, .. } = inst {
                    for (_, label) in goto_labels {
                        observable.entry(*label).or_insert(false);
                    }
                }
            }
        }
        let mut labels: Vec<u32> = blocks_entered_only_by_falling_in(&func, &observable)
            .into_iter()
            .map(|label| label.0)
            .collect();
        labels.sort_unstable();
        labels
    }

    /// `asm goto(... : out); x = 1; out:` -- the block behind the asm goto is
    /// entered by falling out of it and by nothing else.
    #[test]
    fn the_block_an_asm_goto_falls_into_has_no_named_label() {
        let blocks = vec![
            block(0, vec![asm_goto_to(2)], jump(1)),
            block(1, vec![], jump(2)),
            block(2, vec![], ret()),
        ];
        // Block 2 is the asm goto's label: named, whatever else leads there.
        assert_eq!(found(blocks, &[]), [1]);
    }

    #[test]
    fn a_chain_of_blocks_each_entered_from_the_one_ahead() {
        let blocks = vec![
            block(0, vec![], jump(1)),
            block(1, vec![], jump(2)),
            block(2, vec![], ret()),
        ];
        assert_eq!(found(blocks, &[]), [1, 2]);
    }

    /// Every other way in names the label: a second jump, a branch arm, a
    /// loop's back edge, a switch case, `&&label`, a block laid out elsewhere.
    #[test]
    fn a_label_something_names_is_kept() {
        // A second block jumps there.
        let second_jump = vec![
            block(0, vec![], cond(1, 3)),
            block(1, vec![], jump(2)),
            block(2, vec![], ret()),
            block(3, vec![], jump(2)),
        ];
        assert_eq!(found(second_jump, &[]), [] as [u32; 0]);
        // Both arms of the branch ahead: the terminator is not a jump.
        let both_arms = vec![block(0, vec![], cond(1, 1)), block(1, vec![], ret())];
        assert_eq!(found(both_arms, &[]), [] as [u32; 0]);
        // Its own back edge.
        let own_loop = vec![
            block(0, vec![], jump(1)),
            block(1, vec![], cond(1, 2)),
            block(2, vec![], ret()),
        ];
        assert_eq!(found(own_loop, &[]), [] as [u32; 0]);
        // A switch case, twice.
        let switch = vec![
            block(
                0,
                vec![],
                Terminator::Switch {
                    val: Operand::Const(IrConst::I32(0)),
                    cases: vec![(0, BlockId(2)), (1, BlockId(2))],
                    default: BlockId(1),
                    ty: IrType::I32,
                },
            ),
            block(1, vec![], jump(2)),
            block(2, vec![], ret()),
        ];
        assert_eq!(found(switch, &[]), [] as [u32; 0]);
        // Its address is taken.
        let address = vec![block(0, vec![], jump(1)), block(1, vec![], ret())];
        assert_eq!(found(address.clone(), &[1]), [] as [u32; 0]);
        assert_eq!(found(address, &[]), [1]);
        // The jump goes to a block that is not the next one.
        let elsewhere = vec![
            block(0, vec![], jump(2)),
            block(1, vec![], ret()),
            block(2, vec![], ret()),
        ];
        assert_eq!(found(elsewhere, &[]), [] as [u32; 0]);
    }
}

#[cfg(test)]
mod tests {
    use super::has_separating_instruction;

    #[test]
    fn directives_and_labels_never_prove_bytes() {
        for directive in [
            "",
            "# comment",
            ".loc 1 2 0",
            ".loc\t1 2 0",
            ".local symbol",
            ".cfi_def_cfa_offset 16",
            ".file 1 \"input.c\"",
            ".p2align 4,,3",
            ".align 16",
            ".balign 16",
            ".space 0",
            ".skip 0",
            ".fill 0",
            ".Lother: # a label with a comment",
            "1: // numeric local label",
        ] {
            assert!(!has_separating_instruction(directive), "{directive}");
        }
    }

    #[test]
    fn real_instructions_survive_zero_size_metadata() {
        for instruction in ["movl %eax, %ecx", "add x0, x0, #1", "addi a0, a0, 1"] {
            let body = format!(".loc 1 2 0\n{instruction}\n.p2align 4,,3\n");
            assert!(has_separating_instruction(&body), "{body}");
        }
    }

    #[test]
    fn opaque_assembly_and_section_changes_invalidate_the_proof() {
        for opaque in [
            "#APP\n#NO_APP",
            ".section .data",
            ".pushsection .text",
            ".popsection",
            ".previous",
            ".org 0",
            ".if 0",
            ".macro m",
            ".unknown",
            ".loc 1 2; .if 0",
            ".p2align 4; .section .data",
            ".include \"x\"",
        ] {
            for body in [format!("nop\n{opaque}\n"), format!("{opaque}\nnop\n")] {
                assert!(!has_separating_instruction(&body), "{body}");
            }
        }
    }
}
