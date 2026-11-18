//! Late redundant-branch retirement — peephole Phase 9.
//!
//! A branch whose target is the label control already falls through to is a
//! no-op: both outcomes arrive at the same instruction, and a branch writes no
//! flags.  `local_patterns::combined_local_pass` deletes those (its pattern 3,
//! `eliminate_redundant_jumps`), but it runs in Phases 1–4b, and **nine phases
//! after it can create new ones**: 4c late hoisting, 4d loop rotation, 4e
//! extended-basic-block reuse, 5 tail calls, 5b global dead-store elimination,
//! 8 identical-block merging, `jump_thread::run`, 8b terminal epilogue
//! cross-jumping and 8c–8e.  Nothing looks again, so they reach the assembler.
//!
//! Measured over the 1335 assembly files the regression and bench corpora
//! produce at `-O2 -S` (`scratch/branch_to_next.py`): 26 such branches, of which
//! `jump_thread`'s own retargeting accounts for 4 (retired where they are
//! created, in `thread_function`) and the other 21 survive to the object file —
//! 20 `jmp` and 1 `jne`, 8 adjacent to their target and 13 separated only by a
//! stacked label.  gcc-16 16.2.0 and clang 23.1.5 emit none of them, and for
//! `tests/regression/loop_global_promote.c` the raw codegen has none either
//! (`LCCC_NO_PEEPHOLE=1`: 0 against 2), so the pipeline creates them.
//!
//! This is the Phase-9 companion to `pushf_elim` and `redundant_ext`: a narrow,
//! always-on text pass over the FINAL assembly, run last so it also cleans up
//! behind the three passes before it.
//!
//! # Why a text pass and not another `combined_local_pass` round
//!
//! `combined_local_pass` is seven patterns fused into one scan over
//! `store`/`infos`, several of which carry soundness assumptions about their
//! position in the pipeline (register-state mappings, frame-slot stability).
//! Re-running all seven after Phase 8e would re-open every one of those
//! assumptions to earn one pattern's worth of cleanup.  Phase 9 already exists
//! for exactly this shape of problem — its own comment says the early passes
//! "frequently remove/rewrite exactly those instructions" — and its three passes
//! all take the built text.  This follows that contract.
//!
//! # Soundness
//!
//! Deleting the branch is justified only when control demonstrably reaches the
//! target label from the branch without executing anything in between.  The
//! forward walk therefore treats as transparent *only*:
//!
//! * blank lines and comment lines that carry no compiler marker,
//! * labels — a label emits no bytes, so a stacked run (`.LBB26:` then
//!   `.LBB27:`) is transparent, which is the shape `eliminate_redundant_jumps`
//!   cannot see (it compares only the first label it finds),
//! * the debug/unwind directives that emit no bytes: `.loc`, `.file`, `.line`,
//!   `.cfi_*`, `.ident`,
//! * lines this same pass already deleted, so a chain of no-op branches retires
//!   in one scan (the walk runs backwards, and a deleted no-op branch emits
//!   nothing).
//!
//! Anything else ends the walk and keeps the branch.  The allowlist is by
//! directive *name token*, so `.local` (a symbol directive) is not mistaken for
//! `.loc`, and everything that emits bytes — instructions, `.p2align`, `.quad`,
//! `.byte`, `.ascii` — as well as everything that changes meaning (`.section`,
//! `.text`, `.pushsection`, `.popsection`, `.previous`, `.subsection`, a symbol
//! directive, `.size`, `.globl`, `.type`) is rejected by not being listed.
//!
//! Two opacities are refused outright, matching `pushf_elim`:
//!
//! * a `#APP` … `#NO_APP` region.  No line inside one is ever touched, and no
//!   window may span one: user bytes are immutable and a template can define the
//!   very label being looked for (`tests/regression/static_label_difference_alias.c`
//!   and `nested_fn_chain_goto_cross.c` both rely on asm-defined aliases).
//! * a comment carrying `LCCC_` — the compiler-owned marker convention.  Such a
//!   comment has to reach the assembler, so a branch line carrying one is kept,
//!   and a standalone one ends the walk.
//!
//! Only a *direct* branch to a local label is a candidate: the mnemonic must be
//! in [`BRANCH_MNEMONICS`] and the operand a single `.L`-prefixed token, so an
//! indirect dispatch (`jmpq *%rdx`, `jmp *.Ljt_0(,%rax,8)`), a far jump and a
//! tail call to a named symbol (`jmp sym`) are all structurally excluded.

/// The compiler-owned marker prefix.  A comment carrying one is a contract with
/// a later stage, not decoration — the same constant and the same refusal as
/// `jump_thread::inert_comment`.
const COMPILER_MARKER: &str = "LCCC_";

/// Direct branches.  Every x86 conditional jump plus the two unconditional
/// spellings; `jcxz`/`jecxz`/`jrcxz` are conditional (on a register) and are
/// equally no-ops when the target is the fallthrough, and no mnemonic here
/// writes flags.
const BRANCH_MNEMONICS: &[&str] = &[
    "jmp", "jmpq", "ja", "jae", "jb", "jbe", "jc", "je", "jg", "jge", "jl", "jle", "jna", "jnae",
    "jnb", "jnbe", "jnc", "jne", "jng", "jnge", "jnl", "jnle", "jno", "jnp", "jns", "jnz", "jo",
    "jp", "jpe", "jpo", "js", "jcxz", "jecxz", "jrcxz",
];

/// Directives that describe code but emit none, so control flows straight
/// through them.  Matched on the name token, never by prefix, so `.local` is not
/// `.loc` and `.size` is not `.sym`.
fn is_byteless_directive(name: &str) -> bool {
    matches!(name, ".loc" | ".file" | ".line" | ".ident") || name.starts_with(".cfi_")
}

/// The directive name of a line (`.cfi_def_cfa_offset 8136` → `.cfi_def_cfa_offset`).
fn directive_name(trimmed: &str) -> Option<&str> {
    let t = trimmed.strip_prefix('.')?;
    let end = t
        .find(|c: char| c.is_whitespace() || c == ',')
        .unwrap_or(t.len());
    Some(&trimmed[..end + 1])
}

/// Split a line into code and trailing comment.  `None` for the comment when
/// there is none.
fn split_comment(trimmed: &str) -> (&str, Option<&str>) {
    match trimmed.find('#') {
        Some(i) => (trimmed[..i].trim_end(), Some(&trimmed[i..])),
        None => (trimmed.trim_end(), None),
    }
}

/// A comment this pass may see through: decoration only.  A compiler marker is
/// a contract with a later stage and is never transparent.
fn is_inert_comment(comment: &str) -> bool {
    !comment.contains(COMPILER_MARKER)
}

/// The label a direct branch jumps to, when this line is one and retiring it
/// cannot lose a marker.
fn branch_label(line: &str) -> Option<&str> {
    let (code, comment) = split_comment(line);
    if let Some(c) = comment {
        if !is_inert_comment(c) {
            return None;
        }
    }
    let mut words = code.split_whitespace();
    let mnemonic = words.next()?;
    if !BRANCH_MNEMONICS.contains(&mnemonic) {
        return None;
    }
    let operand = words.next()?;
    // Exactly one operand, and a local label: no `*` (indirect), no `,` (far or
    // two-operand), no `(` (memory), no `$` (immediate), no bare symbol (a tail
    // call to a named function).
    if words.next().is_some() || !operand.starts_with('.') {
        return None;
    }
    if operand
        .chars()
        .any(|c| c == '*' || c == ',' || c == '(' || c == '$')
    {
        return None;
    }
    Some(operand)
}

/// The label a line defines, when it defines one.  A label carrying a compiler
/// marker is not transparent — the marker has to reach the assembler in place.
fn label_name(line: &str) -> Option<&str> {
    let (code, comment) = split_comment(line);
    if let Some(c) = comment {
        if !is_inert_comment(c) {
            return None;
        }
    }
    let name = code.strip_suffix(':')?;
    if !name.starts_with('.') || name[1..].contains(char::is_whitespace) {
        return None;
    }
    Some(name)
}

/// Delete every branch whose target is the label control falls through to.
/// Returns true when at least one line was deleted.
///
/// Scans backwards so that a chain of no-op branches retires in one pass: by the
/// time line `i` is considered, every line after it has already been decided,
/// and the forward walk can treat a line this pass deleted as emitting nothing.
pub(super) fn eliminate_branch_to_next_label(asm: &mut String) -> bool {
    if std::env::var("CCC_NO_LATE_BRANCH_ELIM").is_ok() {
        return false;
    }
    let lines: Vec<&str> = asm.lines().collect();
    let mut keep = vec![true; lines.len()];
    let mut changed = false;

    // A branch inside a user region is user text, not ours to retire.
    let mut in_asm = vec![false; lines.len()];
    let mut inside = false;
    for (i, line) in lines.iter().enumerate() {
        match line.trim() {
            "#APP" => inside = true,
            "#NO_APP" => inside = false,
            _ => in_asm[i] = inside,
        }
    }

    for i in (0..lines.len()).rev() {
        if in_asm[i] {
            continue;
        }
        let Some(target) = branch_label(lines[i].trim()) else {
            continue;
        };
        let mut reaches = false;
        let mut j = i + 1;
        while j < lines.len() {
            if in_asm[j] {
                // The window would span user asm: opaque, and its bytes are not
                // ours to reason about.
                break;
            }
            let t = lines[j].trim();
            if !keep[j] || t.is_empty() {
                j += 1;
                continue;
            }
            if t == "#APP" || t == "#NO_APP" {
                break;
            }
            if t.starts_with('#') {
                if !is_inert_comment(t) {
                    break;
                }
                j += 1;
                continue;
            }
            if let Some(name) = label_name(t) {
                if name == target {
                    reaches = true;
                }
                // A label emits no bytes either way, so a stacked run is walked
                // through; a different label simply is not the target yet.
                j += 1;
                continue;
            }
            if t.starts_with('.') {
                if let Some(name) = directive_name(t) {
                    if is_byteless_directive(name) {
                        j += 1;
                        continue;
                    }
                }
            }
            break; // emits bytes, or changes section/symbol state
        }
        if reaches {
            keep[i] = false;
            changed = true;
        }
    }

    if changed {
        let mut out = String::with_capacity(asm.len());
        for (idx, line) in lines.iter().enumerate() {
            if keep[idx] {
                out.push_str(line);
                out.push('\n');
            }
        }
        *asm = out;
    }
    changed
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
