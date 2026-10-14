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
        let token = text.split_whitespace().next().unwrap();
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
