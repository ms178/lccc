//! Tests for [`super`] — register-copy folding.
//!
//! Structure: positives prove each width class folds; negatives prove each
//! legality rule actually blocks. Every negative here corresponds to a real
//! defect that either was found in the corpus or would have been a silent
//! miscompile, and they are named for the rule they pin.
//!
//! The tests drive the WHOLE peephole pipeline rather than this pass alone.
//! That is deliberate: a rewrite that is individually sound but that a
//! neighbouring pass then misreads is still a bug, and the corpus failures
//! this pass produced historically (`bad src register`, `unsupported shrq
//! operands`) were exactly of that kind.

use super::super::peephole_optimize;

fn run(input: &str) -> String {
    peephole_optimize(input.to_string())
}

/// Wrap a body in the CFI markers `FileLiveness` needs to analyse a function.
/// Without them liveness answers `None` and every fold is skipped, so a test
/// that forgets this passes vacuously.
fn f(body: &str) -> String {
    format!(".text\nf:\n.cfi_startproc\n{}\n.cfi_endproc\n", body)
}

fn count(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

// ── A. self-move elimination ────────────────────────────────────────────────

#[test]
fn a_64_bit_self_move_is_always_dead() {
    // Three of these reached final assembly in the corpus census.
    let out = run(&f("    movq %rax, %rax\n    ret"));
    assert_eq!(count(&out, "movq %rax, %rax"), 0, "{}", out);
}

#[test]
fn byte_and_word_self_moves_are_always_dead() {
    // They write the same bits back and disturb nothing above.
    let out = run(&f("    movb %al, %al\n    movw %cx, %cx\n    ret"));
    assert_eq!(count(&out, "movb %al, %al"), 0, "{}", out);
    assert_eq!(count(&out, "movw %cx, %cx"), 0, "{}", out);
}

#[test]
fn a_32_bit_self_move_is_dead_when_the_upper_half_is_already_zero() {
    // `movl %ebx,%ebx` after a 32-bit write: the zero-extension is redundant.
    let out = run(&f(
        "    movl %edi, %ebx\n    movl %ebx, %ebx\n    movq %rbx, %rax\n    ret",
    ));
    assert_eq!(count(&out, "movl %ebx, %ebx"), 0, "{}", out);
}

#[test]
fn a_32_bit_self_move_SURVIVES_when_the_upper_half_is_unknown() {
    // SOUNDNESS: `movl %ebx,%ebx` is the idiomatic 64->32 truncation. With a
    // 64-bit value in %rbx it is load-bearing, and the following 64-bit read
    // must see the truncated value.
    let out = run(&f(
        "    movq (%rdi), %rbx\n    movl %ebx, %ebx\n    movq %rbx, %rax\n    ret",
    ));
    assert_eq!(
        count(&out, "movl %ebx, %ebx"),
        1,
        "must not be removed:\n{}",
        out
    );
}

#[test]
fn a_32_bit_self_move_SURVIVES_across_a_label() {
    // The upper half may be nonzero on another incoming path.
    let out = run(&f(concat!(
        "    movl %edi, %ebx\n",
        ".LBB1:\n",
        "    movl %ebx, %ebx\n",
        "    movq %rbx, %rax\n",
        "    ret"
    )));
    assert_eq!(
        count(&out, "movl %ebx, %ebx"),
        1,
        "must not be removed:\n{}",
        out
    );
}

#[test]
fn a_32_bit_self_move_SURVIVES_across_a_call() {
    // A call clobbers the caller-saved set, so the fact does not carry.
    let out = run(&f(concat!(
        "    movl %edi, %ecx\n",
        "    call g\n",
        "    movl %ecx, %ecx\n",
        "    movq %rcx, %rax\n",
        "    ret"
    )));
    assert_eq!(
        count(&out, "movl %ecx, %ecx"),
        1,
        "must not be removed:\n{}",
        out
    );
}

// ── B. copy folding: positives, one per width class ─────────────────────────

#[test]
fn a_32_bit_copy_folds_into_a_32_bit_compare() {
    // The byte-scan shape: a load already zero-extends, so the Cast that
    // follows is a register move whose destination dies at the compare.
    let out = run(&f(concat!(
        "    movzbl (%rdi), %eax\n",
        "    movl %eax, %esi\n",
        "    cmpl %r8d, %esi\n",
        "    je .L1\n",
        ".L1:\n",
        "    xorl %eax, %eax\n",
        "    ret"
    )));
    assert_eq!(count(&out, "movl %eax, %esi"), 0, "copy must go:\n{}", out);
    assert!(
        out.contains("cmpl %r8d, %eax"),
        "use must retarget:\n{}",
        out
    );
}

#[test]
fn a_64_bit_copy_folds_into_an_ADDRESS_operand() {
    // The capability the previous 32-bit-only attempt lacked: under `movq` the
    // whole register matches, so an address operand is rewritable. Refusing
    // memory operands is why that version removed 10 instructions across 220
    // files.
    let out = run(&f(concat!(
        "    movq %rdi, %r11\n",
        "    movl (%r11), %eax\n",
        "    ret"
    )));
    assert_eq!(count(&out, "movq %rdi, %r11"), 0, "copy must go:\n{}", out);
    assert!(
        out.contains("movl (%rdi), %eax"),
        "address must retarget:\n{}",
        out
    );
}

#[test]
fn a_64_bit_copy_folds_into_a_SIB_address() {
    let out = run(&f(concat!(
        "    movq %rdi, %r11\n",
        "    movzbl (%r11, %rbx, 2), %eax\n",
        "    ret"
    )));
    assert_eq!(count(&out, "movq %rdi, %r11"), 0, "{}", out);
    assert!(out.contains("(%rdi, %rbx, 2)"), "{}", out);
}

#[test]
fn a_64_bit_copy_folds_into_several_uses_at_once() {
    let out = run(&f(concat!(
        "    movq %rdi, %r11\n",
        "    addq %r11, %rcx\n",
        "    xorq %r11, %rdx\n",
        "    movq %rcx, %rax\n",
        "    ret"
    )));
    assert_eq!(count(&out, "movq %rdi, %r11"), 0, "{}", out);
    assert!(out.contains("addq %rdi, %rcx"), "{}", out);
    assert!(out.contains("xorq %rdi, %rdx"), "{}", out);
}

// ── B. copy folding: the width rule, which is where miscompiles live ────────

#[test]
fn a_32_bit_copy_does_NOT_fold_into_a_64_bit_use() {
    // SOUNDNESS: `movl` forces bits 32..63 of %D to zero; %S's may be
    // anything. A 64-bit reader would see a different value.
    let out = run(&f(concat!(
        "    movl %eax, %esi\n",
        "    addq %rsi, %rcx\n",
        "    movq %rcx, %rax\n",
        "    ret"
    )));
    assert_eq!(count(&out, "movl %eax, %esi"), 1, "must not fold:\n{}", out);
}

#[test]
fn a_32_bit_copy_does_NOT_fold_into_an_address_operand() {
    // An address is read as 64 bits, so the same argument applies.
    let out = run(&f(concat!(
        "    movl %eax, %esi\n",
        "    movl (%rsi), %ecx\n",
        "    movl %ecx, %eax\n",
        "    ret"
    )));
    assert_eq!(count(&out, "movl %eax, %esi"), 1, "must not fold:\n{}", out);
}

#[test]
fn a_16_bit_copy_does_NOT_fold_into_a_32_bit_use() {
    // SOUNDNESS: `movw` leaves bits 16..63 of %D STALE — it does not even
    // zero them — so only 16- and 8-bit reads see %S's value.
    let out = run(&f(concat!(
        "    movw %ax, %cx\n",
        "    addl %ecx, %edx\n",
        "    movl %edx, %eax\n",
        "    ret"
    )));
    assert_eq!(count(&out, "movw %ax, %cx"), 1, "must not fold:\n{}", out);
}

#[test]
fn an_8_bit_copy_does_NOT_fold_into_a_16_bit_use() {
    let out = run(&f(concat!(
        "    movb %al, %cl\n",
        "    addw %cx, %dx\n",
        "    movl %edx, %eax\n",
        "    ret"
    )));
    assert_eq!(count(&out, "movb %al, %cl"), 1, "must not fold:\n{}", out);
}

#[test]
fn an_8_bit_copy_DOES_fold_into_an_8_bit_use() {
    let out = run(&f(concat!(
        "    movb %al, %cl\n",
        "    testb %cl, %cl\n",
        "    je .L1\n",
        ".L1:\n",
        "    xorl %eax, %eax\n",
        "    ret"
    )));
    assert_eq!(count(&out, "movb %al, %cl"), 0, "{}", out);
    assert!(out.contains("testb %al, %al"), "{}", out);
}

// ── B. copy folding: the remaining legality rules ───────────────────────────

#[test]
fn a_copy_SURVIVES_when_the_use_also_writes_the_destination() {
    // Rule 4: `addl %eax, %esi` would become `addl %eax, %eax` and clobber the
    // source.
    let out = run(&f(concat!(
        "    movl %eax, %esi\n",
        "    addl %ecx, %esi\n",
        "    movl %esi, %eax\n",
        "    ret"
    )));
    // Rule 4 must never rewrite the use into `addl %ecx, %eax` (that would
    // clobber the copy source before `movl %esi, %eax` reads the result).
    assert_eq!(count(&out, "addl %ecx, %eax"), 0, "rule 4 violated:\n{}", out);
    // The copy may still disappear through a different, legal route: the
    // later `load_op_fuse` "copy + commutative op into the dying operand"
    // pass turns the triple into `addl %eax, %ecx; movl %ecx, %eax` because
    // %ecx (caller-saved) and %esi are dead at `ret`. Either shape is correct;
    // the illegal rule-4 shape is the only thing this test forbids.
    let survives = count(&out, "movl %eax, %esi") == 1 && out.contains("addl %ecx, %esi");
    let bridged = out.contains("addl %eax, %ecx") && out.contains("movl %ecx, %eax");
    assert!(survives || bridged, "unexpected shape:\n{}", out);
}

#[test]
fn a_copy_SURVIVES_when_the_destination_is_still_live_afterwards() {
    // Rule 5. The scan window ends at the branch, but %esi is read on the
    // far side of it, so it is live out of the window and the copy must stay.
    //
    // Getting this shape right matters: an earlier draft of this test put a
    // `movl $7, %eax` between the uses intending to clobber the source, but
    // %eax was overwritten before any read, so it was a genuine dead store —
    // dead-write elimination removed it and the fold became legal. The test
    // was wrong, not the pass.
    let out = run(&f(concat!(
        "    movl %eax, %esi\n",
        "    cmpl %r8d, %esi\n",
        "    je .L1\n",
        "    addl %esi, %ecx\n",
        ".L1:\n",
        "    movl %esi, %eax\n",
        "    ret"
    )));
    assert_eq!(count(&out, "movl %eax, %esi"), 1, "must not fold:\n{}", out);
}

#[test]
fn a_copy_SURVIVES_when_the_source_is_clobbered_before_the_use() {
    // Rule 2.
    let out = run(&f(concat!(
        "    movl %eax, %esi\n",
        "    movl $7, %eax\n",
        "    cmpl %r8d, %esi\n",
        "    ret"
    )));
    assert_eq!(count(&out, "movl %eax, %esi"), 1, "must not fold:\n{}", out);
}

#[test]
fn a_copy_SURVIVES_across_a_label() {
    // Rule 1: another path can reach the use with a different %S.
    let out = run(&f(concat!(
        "    movl %eax, %esi\n",
        ".LBB9:\n",
        "    cmpl %r8d, %esi\n",
        "    jne .LBB9\n",
        "    ret"
    )));
    assert_eq!(count(&out, "movl %eax, %esi"), 1, "must not fold:\n{}", out);
}

#[test]
fn a_copy_SURVIVES_when_the_use_is_a_variable_shift_count() {
    // Rule 6, and a real corpus failure (`pgo_sections`): the count is pinned
    // to %cl even though it is written explicitly, so the implicit-use test
    // does not catch it. Renaming yields `shrq %r9b, %rsi`, which the
    // assembler rejects outright.
    let out = run(&f(concat!(
        "    movl %r9d, %ecx\n",
        "    shrq %cl, %rsi\n",
        "    movq %rsi, %rax\n",
        "    ret"
    )));
    assert!(!out.contains("%r9b"), "count must stay in %cl:\n{}", out);
    assert!(out.contains("shrq %cl, %rsi"), "{}", out);
}

/// %rsp/%rbp anchor the frame and CFI and are excluded from folding on both
/// sides. (Only the destination side is observable end-to-end: a copy whose
/// SOURCE is %rsp gets collapsed by copy propagation into `movq %rsp, %rax`,
/// which is correct and renames nothing away, so it cannot distinguish this
/// pass's behaviour.)
#[test]
fn a_copy_into_a_frame_register_is_never_folded() {
    let out = run(&f(concat!(
        "    movq %rax, %rbp\n",
        "    movq %rbp, %rcx\n",
        "    movq %rcx, %rax\n",
        "    ret"
    )));
    assert_eq!(count(&out, "movq %rax, %rbp"), 1, "{}", out);
}

// ── register-name aliasing: the substring hazards ───────────────────────────

#[test]
fn a_byte_register_name_is_not_mangled_by_the_rename() {
    // `%si` is a prefix of `%sil`. Rewriting the 16-bit name first produced
    // `%dxl`, which is not a register — the assembler rejected the function
    // with "bad src register". Found by the corpus.
    let out = run(&f(concat!(
        "    movzbl (%rdi), %edx\n",
        "    movl %edx, %esi\n",
        "    testb %sil, %sil\n",
        "    je .L1\n",
        ".L1:\n",
        "    xorl %eax, %eax\n",
        "    ret"
    )));
    for bad in ["%dxl", "%axl", "%cxl", "%bxl", "%dil l"] {
        assert!(!out.contains(bad), "mangled register {}:\n{}", bad, out);
    }
}

#[test]
fn the_extended_registers_are_not_blocked_by_prefix_matching() {
    // `%r8` is a prefix of `%r8d`/`%r8w`/`%r8b`. A naive 64-bit guard rejected
    // every r8..r15 operand, silently disabling the pass for half the register
    // file.
    let out = run(&f(concat!(
        "    movzbl (%rdi), %r9d\n",
        "    movl %r9d, %r10d\n",
        "    cmpl %r8d, %r10d\n",
        "    je .L1\n",
        ".L1:\n",
        "    xorl %eax, %eax\n",
        "    ret"
    )));
    assert_eq!(
        count(&out, "movl %r9d, %r10d"),
        0,
        "the copy must go:\n{}",
        out
    );
    assert!(
        out.contains("cmpl %r8d, %r9d") || out.contains("cmpl %r8d, %r10d"),
        "the compare must read a register the load defines:\n{}",
        out
    );
}

#[test]
fn a_high_byte_alias_blocks_the_rename() {
    // SOUNDNESS: %ah has no counterpart once the family moves to %rsi/%r8+.
    let out = run(&f(concat!(
        "    movl %esi, %eax\n",
        "    movb %ah, %cl\n",
        "    movl %ecx, %eax\n",
        "    ret"
    )));
    assert_eq!(count(&out, "movl %esi, %eax"), 1, "must not fold:\n{}", out);
}

#[test]
fn renaming_r15_does_not_clip_the_width_suffix() {
    // Guards the reverse direction of the prefix hazard: rewriting *into*
    // r8..r15 must produce `%r15d`, never `%r15` followed by a stray `d`.
    let out = run(&f(concat!(
        "    movq %r15, %r11\n",
        "    addq %r11, %rcx\n",
        "    movq %rcx, %rax\n",
        "    ret"
    )));
    assert!(out.contains("addq %r15, %rcx"), "{}", out);
    assert!(!out.contains("%r15q") && !out.contains("%r11"), "{}", out);
}

// ── whole-pipeline invariants ───────────────────────────────────────────────

#[test]
fn the_pass_is_idempotent() {
    let src = f(concat!(
        "    movq %rdi, %r11\n",
        "    movl (%r11), %eax\n",
        "    ret"
    ));
    let once = run(&src);
    let twice = peephole_optimize(once.clone());
    assert_eq!(once, twice, "a second run must change nothing");
}

#[test]
fn no_output_line_names_a_nonexistent_register() {
    // Catch-all against the whole class of rename-mangling bugs: every `%name`
    // in the output must be a register the assembler knows.
    let out = run(&f(concat!(
        "    movq %rdi, %r11\n",
        "    movzbl (%r11, %rbx, 2), %esi\n",
        "    movl %esi, %r10d\n",
        "    testb %r10b, %r10b\n",
        "    je .L1\n",
        ".L1:\n",
        "    movw %ax, %cx\n",
        "    movb %al, %dl\n",
        "    xorl %eax, %eax\n",
        "    ret"
    )));
    let valid: std::collections::HashSet<&str> = [
        "rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "eax", "ecx", "edx", "ebx", "esp",
        "ebp", "esi", "edi", "ax", "cx", "dx", "bx", "sp", "bp", "si", "di", "al", "cl", "dl",
        "bl", "spl", "bpl", "sil", "dil", "ah", "bh", "ch", "dh", "rip",
    ]
    .into_iter()
    .chain((8..16).flat_map(|_| std::iter::empty()))
    .collect();
    for tok in out.split(|c: char| !(c.is_ascii_alphanumeric() || c == '%')) {
        let Some(name) = tok.strip_prefix('%') else {
            continue;
        };
        let ok = valid.contains(name)
            || (name.starts_with('r')
                && name[1..]
                    .trim_end_matches(['d', 'w', 'b'])
                    .parse::<u32>()
                    .map(|n| (8..=15).contains(&n))
                    .unwrap_or(false));
        assert!(ok, "output names unknown register %{}:\n{}", name, out);
    }
}

// ── Rule 2/3/4: architectural implicit writes (the stress-lab regression) ───

#[test]
fn a_copy_does_NOT_fold_across_a_division_that_clobbers_its_source() {
    // SOUNDNESS, stress lab `intexpr` seed 1 at -O1 (the ms178-1 regression):
    // `cqto`/`idivq %r11` overwrite %rdx without naming it in their operand
    // text. The window walk used to see "no mention of %rdx" between the
    // copy and the narrow reload and retargeted `movzbl %r8b, ...` to
    // `movzbl %dl, ...` — reading the division REMAINDER instead of the
    // parameter the copy had homed.
    let out = run(&f(concat!(
        "    movq %rdx, %r8\n",
        "    cqto\n",
        "    idivq %r11\n",
        "    movzbl %r8b, %eax\n",
        "    ret\n",
    )));
    assert!(
        out.contains("movzbl %r8b, %eax"),
        "reload must keep its home register:\n{}",
        out
    );
    assert_eq!(
        count(&out, "movzbl %dl"),
        0,
        "division remainder must not leak into the reload:\n{}",
        out
    );
}

#[test]
fn a_copy_into_rdx_does_NOT_fold_across_an_idiv_that_rewrites_it() {
    // Rules 3/4 mirrored through the oracle: `idivq %rcx` rewrites
    // %rax:%rdx but names neither. Pre-union its reg_refs were {%rcx} only,
    // so the window walk sailed through and retargeted later %rdx uses at
    // the copy's source — feeding them the stale copy instead of the
    // remainder the division had just written. The use lands in %rbx so it
    // survives to the return (a dead use would be deleted, hiding the
    // defect); the return value must therefore come from the remainder in
    // %rdx, never from the stale %r9.
    let out = run(&f(concat!(
        "    movq %r9, %rdx\n",
        "    cqto\n",
        "    idivq %rcx\n",
        "    movq %rdx, %rbx\n",
        "    movq %rbx, %rax\n",
        "    ret\n",
    )));
    assert!(out.contains("idivq"), "{out}");
    assert_eq!(
        count(&out, "movq %r9, %rbx"),
        0,
        "stale copy source must not leak into the reload:\n{}",
        out
    );
    assert_eq!(
        count(&out, "movq %r9, %rax"),
        0,
        "stale copy source must not reach the return value:\n{}",
        out
    );
}

#[test]
fn a_copy_out_of_rax_folds_across_cqto_which_only_reads_it() {
    // The oracle is exact, not merely conservative: `cqto` reads %rax and
    // writes ONLY %rdx. The classified destination (%rax) used to end the
    // window at Rule 2 (`dest_j == sfam`) even though the instruction never
    // overwrites the copy source; the exact predicate lets the fold
    // through. (An `idivq`, which DOES rewrite %rax, still blocks it — see
    // the Rule 4 oracle. The use's destination is %ecx so the use itself
    // does not re-write the copy source, which Rule 2 must and does
    // continue to block.)
    let out = run(&f(concat!(
        "    movq %rax, %r8\n",
        "    cqto\n",
        "    movzbl %r8b, %bl\n",
        "    movq %rbx, %rax\n",
        "    ret\n",
    )));
    assert_eq!(count(&out, "movq %rax, %r8"), 0, "copy must fold:\n{}", out);
    assert!(
        out.contains("movzbl %al, %bl"),
        "use must retarget at the copy source:\n{}",
        out
    );
}
