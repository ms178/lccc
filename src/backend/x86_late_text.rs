//! x86 options that are a rule about the finished text.
//!
//! Three requests are not a way of generating code but a statement about
//! what may stand in it, and each is a line or a prefix:
//!
//! * `-mharden-sls=return|indirect-jmp|all`: an `int3` behind every return
//!   and every indirect jump. Neither falls through, so the byte behind it
//!   is never reached by the program; the processor reaches it when it
//!   runs ahead of an instruction it has not decoded as a jump yet
//!   (straight-line speculation), and then it stops there.
//! * `-mcld`: `cld` in front of every string instruction the compiler
//!   writes, for callers that do not keep the direction flag clear as the
//!   ABI has it.
//! * `-fcf-protection=branch`: the indirect jump of a `switch` is marked
//!   `notrack`. With indirect-branch tracking on, a tracked jump must land
//!   on an `endbr`; the case labels have none, here as in GCC, whose jump
//!   says so with this prefix (`-mno-cet-switch`, its default).
//!
//! The rules are applied to the text every emitter and peephole has
//! finished with, so that none of them has to know the forms: they see
//! `ret` and `jmpq *.Ljt_3(,%rax,8)` as they always did. What a person
//! wrote in an `asm` statement, between `#APP` and `#NO_APP`, is not
//! touched: GCC leaves it alone too (measured, 16.2: `ret` inside an asm
//! statement gets no `int3` under `-mharden-sls=all`).
//!
//! Measured against GCC 16.2, `-m64` and `-m32`:
//!
//! | text | `-mharden-sls=` |
//! |---|---|
//! | `ret`, also the one of an inline retpoline | `int3` behind it under `return` and `all` |
//! | `jmp __x86_return_thunk` (`-mfunction-return=thunk-extern`) | nothing |
//! | `jmp *%rax`, `jmp *fp(%rip)`, `notrack jmp *.L4(,%rdi,8)` | `int3` behind it under `indirect-jmp` and `all` |
//! | `jmp __x86_indirect_thunk_rax` | `int3` behind it under `indirect-jmp` and `all` |
//! | `jmp ext`, `call *%rax`, `call ext` | nothing |
//!
//! Two returns get the trap here and none from GCC 16.2: `ret $4`, with
//! which an IA-32 function pops its hidden argument, and the `ret` of
//! `__x86.get_pc_thunk.bx`. GCC prints both without asking the option.
//! They are returns like any other, the kernel's `objtool --sls` asks for
//! the trap behind every return it finds, and a byte that is never
//! executed costs nothing, so the rule here has no exception.

/// What `-mharden-sls=` asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HardenSls {
    /// An `int3` behind every return.
    pub ret: bool,
    /// An `int3` behind every indirect jump.
    pub indirect_jmp: bool,
}

impl HardenSls {
    /// The value of `-mharden-sls=VALUE`, `None` for a value GCC does not
    /// have.
    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "none" => Self::default(),
            "return" => Self {
                ret: true,
                indirect_jmp: false,
            },
            "indirect-jmp" => Self {
                ret: false,
                indirect_jmp: true,
            },
            "all" => Self {
                ret: true,
                indirect_jmp: true,
            },
            _ => return None,
        })
    }
}

/// The rules to apply to one module's text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LateText {
    /// `-mharden-sls=`.
    pub sls: HardenSls,
    /// `-mcld`.
    pub cld: bool,
    /// `-fcf-protection=branch`: `notrack` on jump-table dispatch.
    pub notrack_tables: bool,
}

impl LateText {
    fn is_noop(self) -> bool {
        self == Self::default()
    }
}

/// One line of the text, as the rules read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Line<'a> {
    /// Nothing the processor executes: a directive, a comment, a blank.
    Other,
    /// A label: another way in.
    Label,
    /// `ret`, `retq`, `retl`, with or without a pop count.
    Return,
    /// An indirect jump; the operand, with its `*`.
    IndirectJump(&'a str),
    /// A direct jump to an indirect-branch thunk.
    ThunkJump,
    /// A string instruction under a repeat prefix.
    Repeated,
    /// `cld`.
    Cld,
    /// Any other instruction.
    Instruction,
}

/// Classify `line`. Only what the code generators write has to be read:
/// one statement a line, a mnemonic and its operands, no `;`.
fn classify(line: &str) -> Line<'_> {
    // `#` begins a comment wherever it stands; no instruction has a string
    // for an operand, and a directive is not read past its first byte.
    let text = line.split('#').next().unwrap_or(line).trim();
    if text.ends_with(':') {
        return Line::Label;
    }
    if text.is_empty() || text.starts_with('.') {
        return Line::Other;
    }
    let (mnemonic, operands) = match text.split_once(|c: char| c.is_ascii_whitespace()) {
        Some((mnemonic, rest)) => (mnemonic, rest.trim()),
        None => (text, ""),
    };
    match mnemonic {
        "ret" | "retq" | "retl" => Line::Return,
        "jmp" | "jmpq" | "jmpl" => {
            if operands.starts_with('*') {
                Line::IndirectJump(operands)
            } else if operands.starts_with("__x86_indirect_thunk_") {
                Line::ThunkJump
            } else {
                Line::Instruction
            }
        }
        "notrack" => match classify(operands) {
            jump @ Line::IndirectJump(_) => jump,
            _ => Line::Instruction,
        },
        "rep" | "repe" | "repz" | "repne" | "repnz" => Line::Repeated,
        "cld" => Line::Cld,
        _ => Line::Instruction,
    }
}

/// Whether the jump at `lines[at]` dispatches through a jump table: it
/// names the table, or the table is what follows it. Both back ends write
/// the table directly behind the jump that reads it (the peephole's
/// liveness reads it from there too).
///
/// What may stand between the jump and the table's section, and between
/// the section and the table's label, is what says nothing about either:
/// a comment, unwind and line information, an alignment. Anything else
/// (an instruction, a label, another section) ends the search, so that a
/// jump is never taken for a dispatch because a table follows somewhere.
fn dispatches_through_table(lines: &[&str], at: usize, operand: &str) -> bool {
    if operand.starts_with("*.Ljt_") {
        return true;
    }
    let says_something = |line: &&str| {
        !(line.is_empty()
            || line.starts_with('#')
            || line.starts_with(".cfi_")
            || line.starts_with(".loc ")
            || line.starts_with(".align")
            || line.starts_with(".p2align"))
    };
    let mut rest = lines[at + 1..]
        .iter()
        .map(|line| line.trim())
        .filter(says_something);
    rest.next()
        .is_some_and(|line| line.starts_with(".section .rodata"))
        && rest
            .next()
            .is_some_and(|line| line.starts_with(".Ljt_") && line.ends_with(':'))
}

/// Apply `rules` to the finished text of one module.
pub(crate) fn apply(asm: String, rules: LateText) -> String {
    if rules.is_noop() {
        return asm;
    }
    let lines: Vec<&str> = asm.lines().collect();
    let mut out = String::with_capacity(asm.len() + asm.len() / 16);
    let mut in_asm_statement = false;
    // Whether the instruction last written is a `cld` (labels between it
    // and here do not count: another way in may arrive with the flag set).
    let mut direction_clear = false;
    for (at, line) in lines.iter().enumerate() {
        let text = line.trim();
        if text == "#APP" {
            in_asm_statement = true;
        } else if text == "#NO_APP" {
            in_asm_statement = false;
            direction_clear = false;
        }
        let kind = if in_asm_statement {
            Line::Other
        } else {
            classify(line)
        };
        if rules.cld && kind == Line::Repeated && !direction_clear {
            out.push_str("    cld\n");
        }
        let indent = &line[..line.len() - line.trim_start().len()];
        match kind {
            Line::IndirectJump(operand)
                if rules.notrack_tables
                    && !text.starts_with("notrack")
                    && dispatches_through_table(&lines, at, operand) =>
            {
                out.push_str(indent);
                out.push_str("notrack ");
                out.push_str(text);
            }
            _ => out.push_str(line),
        }
        out.push('\n');
        let trap = match kind {
            Line::Return => rules.sls.ret,
            Line::IndirectJump(_) | Line::ThunkJump => rules.sls.indirect_jmp,
            _ => false,
        };
        if trap {
            out.push_str("    int3\n");
        }
        match kind {
            Line::Cld => direction_clear = true,
            Line::Other => {}
            _ => direction_clear = false,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: HardenSls = HardenSls {
        ret: true,
        indirect_jmp: true,
    };

    fn sls(text: &str, sls: HardenSls) -> String {
        apply(
            text.to_string(),
            LateText {
                sls,
                ..LateText::default()
            },
        )
    }

    #[test]
    fn the_values_of_the_option_are_gccs() {
        assert_eq!(HardenSls::parse("none"), Some(HardenSls::default()));
        assert_eq!(HardenSls::parse("all"), Some(ALL));
        assert_eq!(
            HardenSls::parse("return"),
            Some(HardenSls {
                ret: true,
                indirect_jmp: false
            })
        );
        assert_eq!(
            HardenSls::parse("indirect-jmp"),
            Some(HardenSls {
                ret: false,
                indirect_jmp: true
            })
        );
        for unknown in ["", "ret", "indirect", "All", "return,indirect-jmp"] {
            assert_eq!(HardenSls::parse(unknown), None, "{unknown}");
        }
    }

    #[test]
    fn nothing_asked_changes_nothing() {
        let text = "f:\n    rep movsb\n    jmpq *.Ljt_0(,%rax,8)\n    ret\n";
        assert_eq!(apply(text.to_string(), LateText::default()), text);
    }

    /// Every form of return either back end writes, and GCC's table of
    /// what does and does not get a trap.
    #[test]
    fn a_trap_stands_behind_every_return_and_indirect_jump() {
        let text = "f:\n    ret\n    retq\n    retl\n    ret $4\n    jmp __x86_return_thunk\n\
                    \x20   jmpq *%r10\n    jmp *fp(%rip)\n    jmp *.Ljt_0(, %eax, 4)\n\
                    \x20   notrack jmpq *.Ljt_1(,%rax,8)\n    jmp __x86_indirect_thunk_r10\n\
                    \x20   jmp ext\n    call *%rax\n    call ext\n    jne .LBB3\n";
        assert_eq!(
            sls(text, ALL),
            "f:\n    ret\n    int3\n    retq\n    int3\n    retl\n    int3\n    ret $4\n    int3\n\
             \x20   jmp __x86_return_thunk\n\
             \x20   jmpq *%r10\n    int3\n    jmp *fp(%rip)\n    int3\n\
             \x20   jmp *.Ljt_0(, %eax, 4)\n    int3\n\
             \x20   notrack jmpq *.Ljt_1(,%rax,8)\n    int3\n\
             \x20   jmp __x86_indirect_thunk_r10\n    int3\n\
             \x20   jmp ext\n    call *%rax\n    call ext\n    jne .LBB3\n"
        );
        let returns = sls(
            text,
            HardenSls {
                ret: true,
                indirect_jmp: false,
            },
        );
        assert_eq!(returns.matches("int3").count(), 4);
        assert!(returns.contains("    jmpq *%r10\n    jmp *fp(%rip)\n"));
        let jumps = sls(
            text,
            HardenSls {
                ret: false,
                indirect_jmp: true,
            },
        );
        assert_eq!(jumps.matches("int3").count(), 5);
        assert!(jumps.starts_with("f:\n    ret\n    retq\n"));
    }

    /// The marker the code generator leaves behind a sibling call stays
    /// behind the trap: the trap belongs to the jump.
    #[test]
    fn the_trap_is_the_line_behind_the_jump() {
        assert_eq!(
            sls("    jmpq *%r10\n    # LCCC_TAIL_CALL\n.size f, .-f\n", ALL),
            "    jmpq *%r10\n    int3\n    # LCCC_TAIL_CALL\n.size f, .-f\n"
        );
    }

    #[test]
    fn what_a_person_wrote_is_left_alone() {
        let text =
            "f:\n#APP\n    jmp 1f\n    ret\n    rep movsb\n    jmp *%rax\n1:\n#NO_APP\n    ret\n";
        assert_eq!(
            apply(
                text.to_string(),
                LateText {
                    sls: ALL,
                    cld: true,
                    notrack_tables: true
                }
            ),
            "f:\n#APP\n    jmp 1f\n    ret\n    rep movsb\n    jmp *%rax\n1:\n#NO_APP\n    ret\n    int3\n"
        );
    }

    #[test]
    fn labels_directives_and_names_that_begin_alike_are_no_returns() {
        let text =
            "ret:\n    .size ret, .-ret\n    retract %eax\n    # ret\n    jmp retq\n    call ret\n";
        assert_eq!(sls(text, ALL), text);
    }

    #[test]
    fn a_comment_behind_a_statement_is_not_part_of_it() {
        assert_eq!(
            sls(
                "    ret  # leaves: f\nf:  # entry\n    jmp *%rax # tail\n",
                ALL
            ),
            "    ret  # leaves: f\n    int3\nf:  # entry\n    jmp *%rax # tail\n    int3\n"
        );
        assert_eq!(
            cld("    cld\nf:  # entry\n    rep movsb # copy\n"),
            "    cld\nf:  # entry\n    cld\n    rep movsb # copy\n"
        );
    }

    fn cld(text: &str) -> String {
        apply(
            text.to_string(),
            LateText {
                cld: true,
                ..LateText::default()
            },
        )
    }

    #[test]
    fn the_direction_flag_is_cleared_in_front_of_every_string_instruction() {
        assert_eq!(
            cld("f:\n    movq %rdx, %rcx\n    rep movsb\n    rep stosb\n    ret\n"),
            "f:\n    movq %rdx, %rcx\n    cld\n    rep movsb\n    cld\n    rep stosb\n    ret\n"
        );
        for prefix in ["rep", "repe", "repz", "repne", "repnz"] {
            assert_eq!(
                cld(&format!("    {prefix} cmpsb\n")),
                format!("    cld\n    {prefix} cmpsb\n")
            );
        }
        // Not a prefix: a mnemonic that begins with one.
        assert_eq!(cld("    repair %eax\n"), "    repair %eax\n");
    }

    /// The i686 back end writes its own `cld` at two sites; it is not
    /// written twice. A label between the two ends that: another jump may
    /// arrive there.
    #[test]
    fn a_cld_already_there_is_not_repeated() {
        assert_eq!(cld("    cld\n    rep movsb\n"), "    cld\n    rep movsb\n");
        assert_eq!(
            cld("    cld\n    # comment\n    .loc 1 2 3\n    rep movsb\n"),
            "    cld\n    # comment\n    .loc 1 2 3\n    rep movsb\n"
        );
        assert_eq!(
            cld("    cld\n.LBB2:\n    rep movsb\n"),
            "    cld\n.LBB2:\n    cld\n    rep movsb\n"
        );
        assert_eq!(
            cld("    cld\n    movl %eax, %ecx\n    rep movsb\n"),
            "    cld\n    movl %eax, %ecx\n    cld\n    rep movsb\n"
        );
        assert_eq!(
            cld("    cld\n#APP\n    std\n#NO_APP\n    rep movsb\n"),
            "    cld\n#APP\n    std\n#NO_APP\n    cld\n    rep movsb\n"
        );
    }

    fn notrack(text: &str) -> String {
        apply(
            text.to_string(),
            LateText {
                notrack_tables: true,
                ..LateText::default()
            },
        )
    }

    /// Both shapes of dispatch: through the table's name, and through a
    /// register loaded from the table that follows.
    #[test]
    fn the_jump_of_a_switch_is_not_tracked() {
        assert_eq!(
            notrack(
                "    jmpq *.Ljt_0(,%rdi,8)\n.section .rodata\n.align 8\n.Ljt_0:\n    .quad .LBB6\n"
            ),
            "    notrack jmpq *.Ljt_0(,%rdi,8)\n.section .rodata\n.align 8\n.Ljt_0:\n    .quad .LBB6\n"
        );
        assert_eq!(
            notrack(
                "    leaq .Ljt_2(%rip), %rcx\n    movslq (%rcx,%rax,4), %rdx\n    addq %rcx, %rdx\n\
                 \x20   jmpq *%rdx\n.section .rodata\n.align 4\n.Ljt_2:\n    .long .LBB1 - .Ljt_2\n"
            ),
            "    leaq .Ljt_2(%rip), %rcx\n    movslq (%rcx,%rax,4), %rdx\n    addq %rcx, %rdx\n\
             \x20   notrack jmpq *%rdx\n.section .rodata\n.align 4\n.Ljt_2:\n    .long .LBB1 - .Ljt_2\n"
        );
        assert_eq!(
            notrack("    jmp *.Ljt_4(, %eax, 4)\n"),
            "    notrack jmp *.Ljt_4(, %eax, 4)\n"
        );
    }

    /// Unwind and line information between the jump and its table does
    /// not hide the table: a jump left tracked would fault.
    #[test]
    fn unwind_and_line_information_do_not_hide_the_table() {
        assert_eq!(
            notrack(
                "    jmpq *%rdx\n    .cfi_def_cfa_offset 16\n    .loc 1 7 3\n    # note\n\
                 .section .rodata\n.align 4\n    .cfi_remember_state\n.Ljt_2:\n"
            ),
            "    notrack jmpq *%rdx\n    .cfi_def_cfa_offset 16\n    .loc 1 7 3\n    # note\n\
             .section .rodata\n.align 4\n    .cfi_remember_state\n.Ljt_2:\n"
        );
        // An instruction, a label or another section in between: not the
        // jump's table.
        for text in [
            "    jmpq *%rdx\n    nop\n.section .rodata\n.align 4\n.Ljt_2:\n",
            "    jmpq *%rdx\n.LBB4:\n.section .rodata\n.align 4\n.Ljt_2:\n",
            "    jmpq *%rdx\n.section .text.cold\n.section .rodata\n.align 4\n.Ljt_2:\n",
            "    jmpq *%rdx\n.section .rodata\n.Lstr_1:\n.Ljt_2:\n",
        ] {
            assert_eq!(notrack(text), text);
        }
    }

    /// A sibling call and a computed goto are tracked: their targets
    /// carry an `endbr`. So is a jump behind which other read-only data
    /// stands.
    #[test]
    fn every_other_indirect_jump_stays_tracked() {
        for text in [
            "    jmpq *%r10\n    # LCCC_TAIL_CALL\n.size f, .-f\n",
            "    jmpq *%rax\n.LBB15:\n    endbr64\n",
            "    jmpq *%rax\n.section .rodata\n.align 8\n.Lstr_0:\n    .asciz \"x\"\n",
            "    jmp __x86_indirect_thunk_rdx\n.section .rodata\n.align 4\n.Ljt_2:\n",
            "    notrack jmpq *.Ljt_0(,%rdi,8)\n",
            "    call *.Ljt_0(,%rdi,8)\n",
        ] {
            assert_eq!(notrack(text), text);
        }
    }

    #[test]
    fn the_three_rules_compose() {
        assert_eq!(
            apply(
                "    rep movsb\n    jmpq *.Ljt_0(,%rdi,8)\n.section .rodata\n".to_string(),
                LateText {
                    sls: ALL,
                    cld: true,
                    notrack_tables: true
                }
            ),
            "    cld\n    rep movsb\n    notrack jmpq *.Ljt_0(,%rdi,8)\n    int3\n.section .rodata\n"
        );
    }
}
