//! Remove direct branches to the next executable address in the final x86
//! assembly. Later phases can create these after the ordinary local pass runs.
//! A run of labels and debug/unwind directives shares an address only while
//! no bytes, section changes, compiler markers or inline asm intervene.

use crate::backend::peephole_common::single_asm_statement;
use crate::common::fx_hash::FxHashMap;

const COMPILER_MARKER: &str = "LCCC_";

// No instruction in this list writes registers or flags. In particular,
// LOOP is not here: it decrements a counter even if both edges reach one PC.
const BRANCH_MNEMONICS: &[&str] = &[
    "jmp", "jmpq", "ja", "jae", "jb", "jbe", "jc", "je", "jg", "jge", "jl", "jle", "jna", "jnae",
    "jnb", "jnbe", "jnc", "jne", "jng", "jnge", "jnl", "jnle", "jno", "jnp", "jns", "jnz", "jo",
    "jp", "jpe", "jpo", "js", "jcxz", "jecxz", "jrcxz",
];

fn is_local_label(name: &str) -> bool {
    name.starts_with(".L")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'$'))
}

fn branch_target(code: &str) -> Option<&str> {
    let mut words = code.split_ascii_whitespace();
    if !BRANCH_MNEMONICS.contains(&words.next()?) {
        return None;
    }
    let name = words.next()?;
    (words.next().is_none() && is_local_label(name)).then_some(name)
}

fn is_byteless_directive(code: &str) -> bool {
    let token = code.split_ascii_whitespace().next().unwrap_or("");
    matches!(token, ".loc" | ".file" | ".line" | ".ident")
        || (token.starts_with(".cfi_")
            && !matches!(token, ".cfi_sections" | ".cfi_startproc" | ".cfi_endproc"))
}

fn asm_regions_are_balanced(asm: &str) -> bool {
    let mut inside = false;
    for line in asm.lines() {
        match line.trim() {
            "#APP" if !inside => inside = true,
            "#NO_APP" if inside => inside = false,
            "#APP" | "#NO_APP" => return false,
            _ => {}
        }
    }
    !inside
}

pub(super) fn eliminate_branch_to_next_label(asm: &mut String) -> bool {
    if !asm_regions_are_balanced(asm) {
        return false;
    }
    let mut labels = FxHashMap::default();
    let mut deleted = Vec::new();
    let mut offset = asm.len();
    let mut barrier = offset;
    let mut inside = false;
    for raw in asm.split_inclusive('\n').rev() {
        let end = offset;
        offset -= raw.len();
        let text = raw.trim();
        if text == "#NO_APP" {
            inside = true;
        } else if text == "#APP" {
            inside = false;
        }
        if inside || text == "#APP" || text.contains(COMPILER_MARKER) {
            barrier = offset;
            continue;
        }
        let Some(code) = single_asm_statement(text) else {
            barrier = offset;
            continue;
        };
        if let Some(label) = code.strip_suffix(':').filter(|s| is_local_label(s)) {
            labels.insert(label, offset);
        } else if let Some(target) = branch_target(code)
            && labels.get(target).is_some_and(|&label| label < barrier)
        {
            // A deleted branch is transparent to candidates earlier in the file.
            deleted.push(offset..end);
        } else if !code.is_empty() && !is_byteless_directive(code) {
            barrier = offset;
        }
    }
    if deleted.is_empty() {
        return false;
    }
    let mut result = String::with_capacity(asm.len());
    let mut start = 0;
    for range in deleted.into_iter().rev() {
        result.push_str(&asm[start..range.start]);
        start = range.end;
    }
    result.push_str(&asm[start..]);
    *asm = result;
    true
}

#[cfg(test)]
mod tests {
    use super::eliminate_branch_to_next_label;

    /// Run the pass and report whether it changed anything.
    fn run(input: &str) -> (bool, String) {
        let mut asm = input.to_string();
        let changed = eliminate_branch_to_next_label(&mut asm);
        (changed, asm)
    }

    #[test]
    fn an_adjacent_unconditional_branch_is_deleted() {
        let (changed, out) = run("f:\n    jb .LBB21\n    jmp .LBB27\n.LBB27:\n    ret\n");
        assert!(changed);
        assert_eq!(out, "f:\n    jb .LBB21\n.LBB27:\n    ret\n", "{out}");
    }

    /// A conditional branch is equally dead when both outcomes arrive at the
    /// same instruction, and no mnemonic in the candidate set writes flags.
    /// The corpus holds one of these (`jne`), which no existing pass removes:
    /// `eliminate_redundant_jumps` matches `jmp` only.
    #[test]
    fn an_adjacent_conditional_branch_is_deleted() {
        for mnemonic in ["jne", "jb", "jge", "jae", "jo", "jrcxz"] {
            let input = format!("f:\n    {mnemonic} .L9\n.L9:\n    ret\n");
            let (changed, out) = run(&input);
            assert!(changed, "{mnemonic}");
            assert_eq!(out, "f:\n.L9:\n    ret\n", "{mnemonic}: {out}");
        }
    }

    /// The shape `eliminate_redundant_jumps` cannot see, and 13 of the 21
    /// survivors: a label stacked in front of the target emits no bytes.
    #[test]
    fn a_label_stacked_before_the_target_is_transparent() {
        let (changed, out) = run("    jmp .LBB13\n.LBB12:\n.LBB13:\n    movzbl -1(%r12), %r10d\n");
        assert!(changed);
        assert_eq!(
            out, ".LBB12:\n.LBB13:\n    movzbl -1(%r12), %r10d\n",
            "both labels stay:\n{out}"
        );
        // Any number of them, in any order.
        let (_, out) = run("    jmp .L3\n.L1:\n.L2:\n.L3:\n.L4:\n    ret\n");
        assert_eq!(out, ".L1:\n.L2:\n.L3:\n.L4:\n    ret\n", "{out}");
    }

    /// Debug attribution and unwind rows emit no bytes.  Deleting the branch
    /// leaves them attached to the same code.
    #[test]
    fn byteless_debug_and_unwind_directives_are_transparent() {
        for between in [
            ".loc 1 42 7",
            ".cfi_def_cfa_offset 8136",
            ".cfi_remember_state",
            ".file 2 \"x.c\"",
            ".ident \"lccc\"",
        ] {
            let input = format!("    jmp .L9\n{between}\n.L9:\n    ret\n");
            let (changed, out) = run(&input);
            assert!(changed, "{between}");
            assert!(out.contains(between), "{between}: {out}");
            assert!(!out.contains("jmp"), "{between}: {out}");
        }
    }

    /// The directive name is matched as a token, so a symbol directive that
    /// merely shares a prefix is NOT transparent — and neither is anything that
    /// emits bytes or changes section or symbol state.
    #[test]
    fn lines_that_emit_bytes_or_change_state_keep_the_branch() {
        for between in [
            "    addq $1, %rax",
            ".local hidden_sym",
            ".p2align 4,,10",
            ".quad 0",
            ".byte 0x90",
            ".ascii \"x\"",
            ".section .rodata",
            ".text",
            ".pushsection .text.hot",
            ".subsection 1",
            ".cfi_sections .debug_frame",
            ".cfi_endproc",
            ".loc 1 2; .byte 0xcc",
            ".file 1 \"unterminated",
            ".L8:; .byte 0xcc",
            "loop .L9",
            ".size f, .-f",
            ".globl sym",
            ".type sym, @function",
            "    nop",
        ] {
            let input = format!("    jmp .L9\n{between}\n.L9:\n    ret\n");
            let (changed, out) = run(&input);
            assert!(!changed, "{between} must keep the branch");
            assert_eq!(out, input, "{between}: {out}");
        }
    }

    /// A different label is not the target, and the walk stops at the first
    /// byte-emitting line rather than searching on.
    #[test]
    fn a_branch_to_anything_else_is_untouched() {
        let input = "    jmp .L9\n.L8:\n    ret\n.L9:\n    ret\n";
        let (changed, out) = run(input);
        assert!(!changed, "{out}");
        assert_eq!(out, input);
        // The label is not merely "later in the file": the walk must stop.
        let input = "    jmp .L9\n    ret\n.L9:\n    ret\n";
        let (changed, _) = run(input);
        assert!(!changed, "a terminator between means no fallthrough");
    }

    /// User asm is immutable and opaque: no line inside a region is touched, and
    /// no window spans one — a template can define the very label being sought.
    #[test]
    fn inline_asm_regions_are_never_entered_or_spanned() {
        let input = "    #APP\n    jmp .L9\n    #NO_APP\n.L9:\n    ret\n";
        let (changed, out) = run(input);
        assert!(!changed, "a branch inside #APP is user text");
        assert_eq!(out, input);

        let input = "    jmp .L9\n    #APP\n    nop\n    #NO_APP\n.L9:\n    ret\n";
        let (changed, out) = run(input);
        assert!(!changed, "the window may not span user asm");
        assert_eq!(out, input);

        // A template that defines the target label is exactly why: the branch
        // may be the only way to reach the code after the region.
        let input = "    jmp .L9\n    #APP\n.L9:\n    #NO_APP\n    ret\n";
        let (changed, out) = run(input);
        assert!(!changed, "{out}");
        assert_eq!(out, input);
    }

    #[test]
    fn malformed_asm_markers_disable_the_pass() {
        for input in [
            "#APP\n jmp .L1\n.L1:\nret\n",
            "#NO_APP\n jmp .L1\n.L1:\nret\n",
        ] {
            assert_eq!(run(input), (false, input.to_owned()));
        }
    }

    #[test]
    fn repeated_redundant_transfers_do_not_rescan_the_suffix() {
        let input = "jmp .Lend\r\n".repeat(10_000) + ".Lend:\r\nret";
        assert_eq!(run(&input), (true, ".Lend:\r\nret".to_owned()));
    }

    /// A compiler-owned marker must reach the assembler, so it is never
    /// transparent — neither riding on the branch line nor standing alone.
    #[test]
    fn a_compiler_marker_is_never_transparent() {
        let input = "    jmp .L9 # LCCC_tail\n.L9:\n    ret\n";
        let (changed, out) = run(input);
        assert!(!changed, "the marker rides on the branch line");
        assert_eq!(out, input);

        let input = "    jmp .L9\n# LCCC_marker\n.L9:\n    ret\n";
        let (changed, out) = run(input);
        assert!(!changed, "a standalone marker ends the walk");
        assert_eq!(out, input);

        let input = "    jmp .L9\n.L9: # LCCC_alias\n    ret\n";
        let (changed, out) = run(input);
        assert!(!changed, "a marked label is not transparent");
        assert_eq!(out, input);

        // Plain decoration is transparent.
        let (changed, out) = run("    jmp .L9 # edge taken\n.L9:\n    ret\n");
        assert!(changed, "{out}");
        assert_eq!(out, ".L9:\n    ret\n", "{out}");
    }

    /// Indirect dispatches, memory operands, far jumps and tail calls to named
    /// symbols are structurally not candidates.  The indirect ones matter: they
    /// are the switch dispatch whose table this pass must never disturb.
    #[test]
    fn only_a_direct_branch_to_a_local_label_is_a_candidate() {
        for line in [
            "    jmpq *%rdx",
            "    jmp *%rax",
            "    jmpq *.Ljt_0(,%rax,8)",
            "    jmp *tbl(%rip)",
            "    ljmp $0x10, $0x20",
            "    jmp helper",
            "    jmpq helper@PLT",
            "    call helper",
            "    jmp $0x1234",
        ] {
            let input = format!("{line}\n.L9:\n    ret\n");
            let (changed, out) = run(&input);
            assert!(!changed, "{line} must not be a candidate");
            assert_eq!(out, input, "{line}: {out}");
        }
    }

    /// A chain of no-op branches retires in one scan: the backwards walk lets an
    /// earlier branch see through a later one this pass already deleted.
    #[test]
    fn a_chain_of_no_op_branches_retires_in_one_scan() {
        let (changed, out) = run("    jmp .L1\n    jmp .L2\n.L1:\n    jmp .L2\n.L2:\n    ret\n");
        assert!(changed);
        assert_eq!(out, ".L1:\n.L2:\n    ret\n", "{out}");
    }

    /// Nothing to do means the text is returned byte-identical and the answer is
    /// false — Phase 9 calls this unconditionally on every function.
    #[test]
    fn an_unchanged_function_is_returned_byte_identical() {
        for input in [
            "f:\n.cfi_startproc\n    xorl %eax, %eax\n    ret\n.cfi_endproc\n",
            "",
            "\n",
            "f:\n    jmp .L9\n    ret\n.L9:\n    ret\n",
        ] {
            let (changed, out) = run(input);
            assert!(!changed, "{input:?}");
            assert_eq!(out, input, "{input:?}");
        }
    }

    /// The witness from the corpus: `tests/regression/loop_global_promote.c` at
    /// -O2 emits `jmp .LBB27` directly above `.LBB27:` on a loop-exit path, and
    /// `cross_pr_redteam.c` emits four.  Both are retired, and the loop back-edge
    /// above them is not.
    #[test]
    fn the_corpus_witness_shape_is_retired() {
        let input = "\
    addl $1, %edx
    cmpl $41, %edx
    jb .LBB21
    jmp .LBB27
.LBB27:
    movq %r10, %rax
    ret
.LBB21:
    nop
";
        let (changed, out) = run(input);
        assert!(changed);
        assert!(out.contains("    jb .LBB21\n.LBB27:"), "{out}");
        assert!(!out.contains("jmp .LBB27"), "{out}");
        assert!(
            out.contains(".LBB21:\n    nop"),
            "the back-edge target stays:\n{out}"
        );
    }
}
