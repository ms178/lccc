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
fn a_32_bit_self_move_survives_when_the_upper_half_is_unknown() {
    // SOUNDNESS: `movl %ebx,%ebx` is the idiomatic 64->32 truncation. With a
    // 64-bit value in %rbx it is load-bearing, and the following 64-bit read
    // must see the truncated value.
    // The late chain rename may compute the chain in %rax directly; either
    // way the truncating self-move must survive, and nothing else may change.
    let out = run(&f(
        "    movq (%rdi), %rbx\n    movl %ebx, %ebx\n    movq %rbx, %rax\n    ret",
    ));
    let body: Vec<&str> = out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('.') && !l.ends_with(':'))
        .collect();
    assert!(
        body == [
            "movq (%rdi), %rbx",
            "movl %ebx, %ebx",
            "movq %rbx, %rax",
            "ret"
        ] || body == ["movq (%rdi), %rax", "movl %eax, %eax", "ret"],
        "the truncation must survive:\n{out}"
    );
}

#[test]
fn a_32_bit_self_move_survives_across_a_label() {
    // The late `return_home` copy-fusion legitimately recomputes this
    // shape into the return register; pin the narrow-copy decision alone.
    let _skip =
        crate::backend::x86::codegen::peephole::passes::test_skips::skip_passes(&["return_home"]);
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
fn a_32_bit_self_move_survives_across_a_call() {
    // A call clobbers the caller-saved set, so the fact does not carry.
    let out = run(&f(concat!(
        "    movl %edi, %ecx\n",
        "    call g\n",
        "    movl %ecx, %ecx\n",
        "    movq %rcx, %rax\n",
        "    ret"
    )));
    // The zero-extension must survive: either the self-move stays, or the
    // producer retarget folds it into the copy as `movl %ecx, %eax` (also a
    // 32-bit write). A bare `movq %rcx, %rax` would keep stale upper bits.
    assert!(
        !out.contains("movq %rcx, %rax") || out.contains("movl %ecx, %ecx"),
        "{out}"
    );
    assert_eq!(
        count(&out, "movl %ecx, %ecx") + count(&out, "movl %ecx, %eax"),
        1,
        "the zero-extension must not be removed:\n{}",
        out
    );
}

// ── B. copy folding: positives, one per width class ─────────────────────────

#[test]
fn a_32_bit_copy_folds_into_a_32_bit_compare() {
    // The byte-scan shape: a load already zero-extends, so the Cast that
    // follows is a register move whose destination dies at the compare.
    // (The self-xor that closes the function reads nothing, so the LOAD's
    // destination dies together with the copy: the fold retargets the load
    // into the copy's home %esi and the compare keeps its own operand —
    // the copy is gone either way, one value fewer in flight.)
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
        out.contains("movzbl (%rdi), %esi") && out.contains("cmpl %r8d, %esi"),
        "load must fold into the surviving home:\n{}",
        out
    );
}

#[test]
fn a_64_bit_copy_folds_into_an_address_operand() {
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
fn a_64_bit_copy_folds_into_a_sib_address() {
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
fn a_32_bit_copy_does_not_fold_into_a_64_bit_use() {
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
fn a_32_bit_copy_does_not_fold_into_an_address_operand() {
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
fn a_16_bit_copy_does_not_fold_into_a_32_bit_use() {
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
fn an_8_bit_copy_does_not_fold_into_a_16_bit_use() {
    let out = run(&f(concat!(
        "    movb %al, %cl\n",
        "    addw %cx, %dx\n",
        "    movl %edx, %eax\n",
        "    ret"
    )));
    assert_eq!(count(&out, "movb %al, %cl"), 1, "must not fold:\n{}", out);
}

#[test]
fn an_8_bit_copy_does_fold_into_an_8_bit_use() {
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
fn a_copy_survives_when_the_use_also_writes_the_destination() {
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
    assert_eq!(
        count(&out, "addl %ecx, %eax"),
        0,
        "rule 4 violated:\n{}",
        out
    );
    // The copy may still disappear through a different, legal route:
    //   (a) the later `load_op_fuse` "copy + commutative op into the dying
    //       operand" pass turns the triple into `addl %eax, %ecx; movl
    //       %ecx, %eax` because %ecx (caller-saved) and %esi are dead at
    //       `ret`; or
    //   (b) `fuse_staged_add_and_relay` folds copy+add+relay in one step into
    //       `leal (%rax, %ecx), %eax` — legal precisely because the lea READS
    //       its address operands before writing the destination, so the rule-4
    //       clobber hazard cannot arise.
    // Any of the shapes is correct; the illegal rule-4 shape (`addl %ecx,
    // %eax` as an ADD) is the only thing this test forbids.
    let survives = count(&out, "movl %eax, %esi") == 1 && out.contains("addl %ecx, %esi");
    let bridged = out.contains("addl %eax, %ecx") && out.contains("movl %ecx, %eax");
    let staged = out.contains("leal (%rax, %rcx), %eax");
    assert!(survives || bridged || staged, "unexpected shape:\n{}", out);
}

#[test]
fn a_copy_survives_when_the_destination_is_still_live_afterwards() {
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
fn a_copy_survives_when_the_source_is_clobbered_before_the_use() {
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
fn a_copy_survives_across_a_label() {
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
fn a_copy_survives_when_the_use_is_a_variable_shift_count() {
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
/// pass's behaviour.) The fixture must ESTABLISH a frame — a bare `movq
/// %rax, %rbp` in a rsp-relative function makes %rbp a plain GPR, which the
/// rbp-as-GPR retarget legally folds (pinned by
/// `load_relay_into_gpr_rbp_home_retargets`).
#[test]
fn a_copy_into_a_frame_register_is_never_folded() {
    let out = run(&f(concat!(
        "    pushq %rbp\n",
        "    movq %rsp, %rbp\n",
        "    movq %rax, %rbp\n",
        "    movq %rbp, %rcx\n",
        "    movq %rbp, %rsp\n",
        "    popq %rbp\n",
        "    ret"
    )));
    assert_eq!(count(&out, "movq %rax, %rbp"), 1, "{}", out);
    // The frame epilogue survives untouched.
    assert_eq!(count(&out, "movq %rbp, %rsp"), 1, "{}", out);
    assert_eq!(count(&out, "popq %rbp"), 1, "{}", out);
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
    // The late `return_home` copy-fusion legitimately recomputes this
    // shape into the return register; pin the narrow-copy decision alone.
    let _skip =
        crate::backend::x86::codegen::peephole::passes::test_skips::skip_passes(&["return_home"]);
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
fn a_copy_does_not_fold_across_a_division_that_clobbers_its_source() {
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
fn a_copy_into_rdx_does_not_fold_across_an_idiv_that_rewrites_it() {
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

// ── C. extension folding (movz/movs feeding narrower consumers) ─────────────

#[test]
fn movzbl_feeding_only_a_byte_store_folds_onto_the_source_lane() {
    // The oracle shape (GCC: `buf[i] = (uint8_t)(x >> 24)`):
    //   shrl $24, %edi; movzbl %dil, %r9d; movb %r9b, (%rcx,%rsi)
    // must become
    //   shrl $24, %edi; movb %dil, (%rcx,%rsi)
    // The extension's upper bits are its own product; the low byte is the
    // source lane bit-for-bit, and the store reads only that lane.
    let out = run(&f(
        "    shrl $24, %edi\n    movzbl %dil, %r9d\n    movb %r9b, (%rcx, %rsi)\n    ret",
    ));
    assert_eq!(count(&out, "movzbl"), 0, "{}", out);
    assert_eq!(count(&out, "movb %r9b"), 0, "{}", out);
    assert_eq!(count(&out, "movb %dil, (%rcx, %rsi)"), 1, "{}", out);
}

#[test]
fn movzbl_survives_when_a_wide_read_consumes_the_extension() {
    // SOUNDNESS: a 32-bit read of the extension result needs the
    // zero-extended bits; the fold must refuse.
    let out = run(&f(
        "    shrl $24, %edi\n    movzbl %dil, %r9d\n    movl %r9d, %eax\n    ret",
    ));
    assert_eq!(count(&out, "movzbl"), 1, "{}", out);
}

#[test]
fn movzbl_with_rip_relative_byte_store_folds() {
    // f() shape: movzbl %dil, %esi; movb %sil, buf+3(%rip) -> movb %dil, ...
    let out = run(&f(
        "    shrl $24, %edi\n    movzbl %dil, %esi\n    movb %sil, buf+3(%rip)\n    ret",
    ));
    assert_eq!(count(&out, "movzbl"), 0, "{}", out);
    assert_eq!(count(&out, "movb %dil, buf+3(%rip)"), 1, "{}", out);
}

#[test]
fn ext_parse_shapes_unit() {
    use super::parse_reg_to_reg_ext;
    assert!(parse_reg_to_reg_ext("movzbl %dil, %r9d").is_some());
    assert!(parse_reg_to_reg_ext("movsbq %sil, %rax").is_some());
    assert!(parse_reg_to_reg_ext("movzwl %si, %edi").is_some());
    // Non-extensions and look-alikes.
    assert!(parse_reg_to_reg_ext("movsd %xmm0, (%rax)").is_none());
    assert!(parse_reg_to_reg_ext("movss %xmm0, %xmm1").is_none());
    assert!(parse_reg_to_reg_ext("movl %eax, %ecx").is_none());
    assert!(parse_reg_to_reg_ext("rep movsb").is_none());
}

// ── C. dead in-place extensions ─────────────────────────────────────────────

/// Count the sign extensions of `%eax` into `%rax` that survive, in either
/// spelling (the pipeline may legitimately merge `movl MEM,%eax; cltq` into
/// `movslq MEM,%rax`).
fn rax_sign_extensions(out: &str) -> usize {
    out.lines()
        .map(str::trim)
        .filter(|l| *l == "cltq" || l.starts_with("movslq") && l.ends_with("%rax"))
        .count()
}

#[test]
fn cltq_survives_a_64_bit_read_modify_write() {
    // SOUNDNESS (gcc.c-torture execute/20010106-1.c, -O0): the switch index
    // of `case -2 … case 4` is rebased with a 64-bit `subq $-2, %rax`.  The
    // subtract WRITES %rax but first READS all 64 bits; deleting the `cltq`
    // made -1 rebase to 0x1_0000_0001 and dispatch to the default arm.
    for rmw in [
        "subq $-2, %rax",
        "addq %rcx, %rax",
        "imulq $3, %rax",
        "negq %rax",
    ] {
        let out = run(&f(&format!(
            "    movl -12(%rbp), %eax\n    cltq\n    {rmw}\n    cmpq $7, %rax\n    jae .L9\n    ret\n.L9:\n    ret"
        )));
        assert_eq!(rax_sign_extensions(&out), 1, "{rmw}:\n{out}");
    }
}

#[test]
fn cltq_is_dead_before_a_pure_64_bit_redefinition() {
    let out = run(&f(
        "    movl %edi, %eax\n    cltq\n    movq %rsi, %rax\n    ret",
    ));
    assert_eq!(count(&out, "cltq"), 0, "{}", out);
}

#[test]
fn cltq_is_dead_before_a_pure_32_bit_redefinition() {
    // A 32-bit write zero-extends into all 64 bits without reading them, so
    // the widened bits are unobservable even though %rax is read afterwards.
    let out = run(&f(
        "    movl %edi, %eax\n    cltq\n    movl %esi, %eax\n    movq %rax, %rdx\n    ret",
    ));
    assert_eq!(count(&out, "cltq"), 0, "{}", out);
}

#[test]
fn cltq_is_dead_before_a_32_bit_read_modify_write() {
    // `addl` reads only the low half (bit-identical with or without the
    // extension) and zero-extends its result, so the later 64-bit read sees
    // zeros in bits 63:32 either way.
    let out = run(&f(
        "    movl %edi, %eax\n    cltq\n    addl $1, %eax\n    movq %rax, (%rsi)\n    ret",
    ));
    assert_eq!(rax_sign_extensions(&out), 0, "{}", out);
}

#[test]
fn cltq_survives_a_partial_byte_write_followed_by_a_64_bit_read() {
    // SOUNDNESS: `movb` rewrites only bits 7:0 — bits 63:32 still carry the
    // extension's product into the 64-bit store.
    let out = run(&f(
        "    movl %edi, %eax\n    cltq\n    movb $1, %al\n    movq %rax, (%rsi)\n    ret",
    ));
    assert_eq!(rax_sign_extensions(&out), 1, "{}", out);
}

// ── Rule 4b: a use that cannot be rewritten aborts the WHOLE fold ──────────

#[test]
fn a_non_rewritable_use_aborts_the_whole_copyback_fold() {
    // REGRESSION, and a real miscompile before the fix.
    //
    // `fold_induction_copyback` precomputes each use's rewritten form, then
    // commits by retargeting the producer `%T -> %D` and deleting the copy.
    // That commit is all-or-nothing: once the copy is gone `%T` is defined by
    // nothing, so a use left un-rewritten reads an undefined register.
    //
    // The original code computed `all_rewritable = false` at the first
    // inexpressible use and never read it -- it broke out of the scan and
    // committed anyway.  Three things are needed to expose that, and each was
    // found by probing, not by guessing:
    //
    //   1. the producer must be a `leaq`/`mov{l,q}` LOAD (`parse_copyback_producer`
    //      accepts nothing else -- a `movzbl` producer never reaches this fold);
    //   2. `%D` must stay LIVE, or an earlier pass deletes the copy as a dead
    //      store and the fold never sees it;
    //   3. one use must be inexpressible (`%ah`: `register_family_at` places it
    //      in the `%eax` family, but `%ah` appears in no `REG_NAMES` slot at any
    //      width, so `replace_reg` matches nothing and `rw == line`), and a
    //      LATER use must exist to be left dangling (`addq %rdx, %r8`).
    let out = run(&f(concat!(
        "    leaq 8(%rdi), %rax\n",
        "    movq %rax, %rdx\n",
        "    movzbl %ah, %ecx\n",
        "    movl %ecx, %r8d\n",
        "    addq %rdx, %r8\n",
        "    movq %r8, %rax\n",
        "    ret"
    )));

    // The discriminating invariant: the register the surviving uses READ must
    // be the register the producer WRITES.  Committing the fold moves the write
    // to `%rdx`; refusing it leaves the write on `%rax` where `movzbl %ah` and
    // `addq %rax` expect it.  Asserting on that relation -- rather than on the
    // literal `movq %rax, %rdx` -- keeps the test valid even though a later,
    // legitimate copy-propagation pass deletes the copy either way, and it is
    // exactly what `gh/main` gets wrong:
    //
    //     gh/main:   leaq 8(%rdi), %rdx / movzbl %ah, %r8d / addq %rax, %r8
    //     fixed:     leaq 8(%rdi), %rax / movzbl %ah, %r8d / addq %rax, %r8
    //
    // In the first, `%rax` is written by nothing: an undefined-register read.
    assert!(
        out.contains("leaq 8(%rdi), %rax"),
        "the producer must still write %rax, the register the uses read:\n{}",
        out
    );
    assert!(
        !out.contains("leaq 8(%rdi), %rdx"),
        "the producer must NOT have been retargeted to %rdx; that retarget is \
         precisely what leaves every surviving %rax use undefined:\n{}",
        out
    );
    assert!(
        out.contains("addq %rax, %r8"),
        "the later use must still read %rax:\n{}",
        out
    );
}

#[test]
fn a_copyback_fold_still_fires_when_every_use_is_rewritable() {
    // Positive control for the test above: the identical shape with `%ah` ->
    // `%al`, so every use IS expressible.  Without this, a guard that refused
    // everything would satisfy the regression test while silently disabling the
    // fold -- the failure mode the gate exists to prevent.
    let out = run(&f(concat!(
        "    leaq 8(%rdi), %rax\n",
        "    movq %rax, %rdx\n",
        "    movzbl %al, %ecx\n",
        "    movl %ecx, %r8d\n",
        "    addq %rdx, %r8\n",
        "    movq %r8, %rax\n",
        "    ret"
    )));
    assert_eq!(
        count(&out, "movq %rax, %rdx"),
        0,
        "the copy must be folded away here:\n{}",
        out
    );
    assert!(
        out.contains("leaq 8(%rdi), %rdx"),
        "the producer must be retargeted into the copy destination:\n{}",
        out
    );
}

/// Parser-level tests for the producer shapes the copy-back fold accepts.
///
/// These are pure: `parse_copyback_producer` maps one trimmed line to a
/// producer description, so each case below pins an exact decision with no
/// dependence on the surrounding pipeline. The SIB cases exist because the
/// operand split must happen at the LAST comma — splitting at the first one
/// turns `leaq 8(%rax, %r9, 1), %r10` into a bogus displacement and silently
/// drops the fold (or, worse, misreads the base).
mod copyback_producer_tests {
    use super::super::parse_copyback_producer;
    use super::super::{CopybackProducer, plain_reg_family};

    #[test]
    fn sib_leaq_parses_as_lea_mem_and_keeps_the_full_operand() {
        for (line, mem, dest) in [
            ("leaq 8(%rax, %r9, 1), %r10", "8(%rax, %r9, 1)", "%r10"),
            ("leaq (%rdx, %r9, 8), %rax", "(%rdx, %r9, 8)", "%rax"),
        ] {
            let (producer, fam) =
                parse_copyback_producer(line).unwrap_or_else(|| panic!("{line} must parse"));
            match producer {
                CopybackProducer::LeaMem { mem: got } => assert_eq!(got, mem, "{line}"),
                _ => panic!("{line} must parse as LeaMem, not a plain-base Lea"),
            }
            assert_eq!(Some(fam), plain_reg_family(dest), "{line}");
        }
    }

    #[test]
    fn plain_base_leaq_still_parses_as_lea() {
        let (producer, fam) =
            parse_copyback_producer("leaq 8(%rax), %r10").expect("plain-base leaq must parse");
        match producer {
            CopybackProducer::Lea { disp, base } => {
                assert_eq!(disp, "8");
                assert_eq!(Some(base), plain_reg_family("%rax"));
            }
            _ => panic!("a single-register operand must stay a plain-base Lea"),
        }
        assert_eq!(Some(fam), plain_reg_family("%r10"));
    }

    #[test]
    fn memory_load_with_sib_source_splits_on_the_last_comma() {
        let (producer, fam) = parse_copyback_producer("movq 16(%rsi, %rax, 8), %rax")
            .expect("SIB load producer must parse");
        match producer {
            CopybackProducer::Load { is_q, mem } => {
                assert!(is_q, "movq is a full-width load");
                assert_eq!(mem, "16(%rsi, %rax, 8)");
            }
            _ => panic!("expected a Load producer"),
        }
        assert_eq!(Some(fam), plain_reg_family("%rax"));
    }

    #[test]
    fn wrong_width_destination_refuses() {
        // The load/store width and the destination spelling must agree; a
        // narrowed destination is never a copy-back candidate.
        assert!(parse_copyback_producer("leaq 8(%rax), %eax").is_none());
        assert!(parse_copyback_producer("leaq 8(%rax, %r9, 1), %r10d").is_none());
        assert!(parse_copyback_producer("movl 8(%rax), %rax").is_none());
        // Register-to-register copies belong to fold_register_copies.
        assert!(parse_copyback_producer("movq %rax, %r10").is_none());
    }

    #[test]
    fn upper32_zero_promotes_32bit_copy_and_base_read_rewrites_copy_site() {
        // 1. A 32-bit producer (`movl (%rdi), %eax`) zeroes bits 32..63 of
        //    `%rax`, so a subsequent `movl %eax, %ecx` can be treated as W64
        //    and forwarded into a 64-bit use `(,%rcx,4)`.
        let out = super::run(&super::f(concat!(
            "    movl (%rdi), %eax\n",
            "    movl %eax, %ecx\n",
            "    movl (%rsi,%rcx,4), %eax\n",
            "    ret"
        )));
        assert!(
            out.contains("(%rsi,%rax,4)") && !out.contains("movl %eax, %ecx"),
            "32-bit zero-extending producer must allow W64 index forwarding:\n{out}"
        );

        // 2. When `%rax` is read between `leaq 4(%rax), %r10` and the copy-back
        //    `movq %r10, %rax` (with `%rax` live out at `ret`), the copy site
        //    itself is rewritten to `leaq 4(%rax), %rax` and the earlier `leaq`
        //    is dropped.
        let out = super::run(&super::f(concat!(
            "    leaq 4(%rax), %r10\n",
            "    movl (%rax), %ecx\n",
            "    movl %ecx, (%rsi)\n",
            "    movq %r10, %rax\n",
            "    ret"
        )));
        assert!(
            out.contains("leaq 4(%rax), %rax") && !out.contains("%r10"),
            "copyback with intermediate base read must collapse at the copy site:\n{out}"
        );
    }
}

// ── frame-register evidence gate (direct pass calls) ────────────────────────
//
// These call `fold_register_copies` alone: the full pipeline also runs
// `relay_and_lea`, whose own rbp retarget would fold the same copies and
// mask a broken gate here.

mod frame_register_gate {
    use super::super::*;
    use crate::backend::peephole_common::LineStore;

    fn fold(body: &str) -> String {
        let asm = super::f(body);
        let mut store = LineStore::new(asm);
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        fold_register_copies(&mut store, &mut infos);
        (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// In a frameless function `%rbp` is an allocator GPR (callee-save
    /// push/pop only), so a copy out of it folds like any other.
    #[test]
    fn a_copy_out_of_a_gpr_rbp_folds() {
        let out = fold(concat!(
            "    pushq %rbp\n",
            "    leaq 8(%rdi), %rbp\n",
            "    movq %rbp, %r11\n",
            "    addq %r11, %rax\n",
            "    popq %rbp\n",
            "    ret"
        ));
        assert_eq!(
            super::count(&out, "movq %rbp, %r11"),
            0,
            "copy must go:\n{out}"
        );
        assert!(out.contains("addq %rbp, %rax"), "use must read rbp:\n{out}");
    }

    /// And a copy INTO a GPR `%rbp` whose only reader follows folds onto
    /// the source (`popq %rbp` ends rbp's value).
    #[test]
    fn a_copy_into_a_gpr_rbp_folds() {
        let out = fold(concat!(
            "    pushq %rbp\n",
            "    movq %rdi, %rbp\n",
            "    addq %rbp, %rax\n",
            "    popq %rbp\n",
            "    ret"
        ));
        assert_eq!(
            super::count(&out, "movq %rdi, %rbp"),
            0,
            "copy must go:\n{out}"
        );
        assert!(out.contains("addq %rdi, %rax"), "use must read rdi:\n{out}");
    }

    /// The same copy in a frame-pointer function stays: there `%rbp` anchors
    /// the CFA and `leave`/`popq` redefine it implicitly.
    #[test]
    fn a_copy_out_of_a_frame_pointer_rbp_is_kept() {
        let out = fold(concat!(
            "    pushq %rbp\n",
            "    movq %rsp, %rbp\n",
            "    movq %rbp, %r11\n",
            "    addq %r11, %rax\n",
            "    popq %rbp\n",
            "    ret"
        ));
        assert_eq!(super::count(&out, "movq %rbp, %r11"), 1, "{out}");
        assert!(out.contains("addq %r11, %rax"), "{out}");
    }

    /// `%rsp` is never a copy source or destination, frameless or not:
    /// push/pop/call move it implicitly.
    #[test]
    fn a_copy_out_of_rsp_is_never_folded() {
        let out = fold(concat!(
            "    movq %rsp, %r11\n",
            "    addq %r11, %rax\n",
            "    ret"
        ));
        assert_eq!(super::count(&out, "movq %rsp, %r11"), 1, "{out}");
    }

    /// A 64-bit self-move writes the same bits back: dead for every family,
    /// the frame registers included. The 32-bit form zeroes bits 32..63 and
    /// stays for `%rsp`/`%rbp` even after a zero-extending write, because
    /// their implicit redefinitions (push/pop/call/leave) are invisible to
    /// the upper-32 tracker.
    #[test]
    fn frame_family_self_moves_follow_the_width_rule() {
        let out = fold("    movq %rbp, %rbp\n    movq %rsp, %rsp\n    movw %bp, %bp\n    ret");
        assert_eq!(super::count(&out, "movq %rbp, %rbp"), 0, "{out}");
        assert_eq!(super::count(&out, "movq %rsp, %rsp"), 0, "{out}");
        assert_eq!(super::count(&out, "movw %bp, %bp"), 0, "{out}");
        let out = fold(
            "    movl %edi, %ebp\n    pushq %rax\n    movl %ebp, %ebp\n    movl %esp, %esp\n    ret",
        );
        assert_eq!(super::count(&out, "movl %ebp, %ebp"), 1, "{out}");
        assert_eq!(super::count(&out, "movl %esp, %esp"), 1, "{out}");
    }
}

/// The extension-before-narrow-store fold must work when the register
/// allocator homed the byte value in a frameless function's `%rbp`, even with
/// `%rsp`-slot spill traffic in the loop: a spill is not a read of `%rbp`.
/// Pre-fix, `classify_line` tagged every slot move with the rbp bit, liveness
/// kept `%ebp` live past the store, and the pair survived to final assembly
/// (`zlib_ng_adler32_combine` main, 7 vs 6 instructions per iteration).
#[test]
fn ext_into_allocated_rbp_folds_across_rsp_slot_spills() {
    let out = run(&f(concat!(
        "    pushq %rbp\n",
        "    subq $16, %rsp\n",
        ".LBB1:\n",
        "    movzbl %r15b, %ebp\n",
        "    movb %bpl, (%r12, %r14)\n",
        "    movq %r14, 8(%rsp)\n",
        "    addq $1, %r14\n",
        "    cmpq $1024, %r14\n",
        "    jb .LBB1\n",
        "    movl $3, %ebp\n",
        "    movq 8(%rsp), %rax\n",
        "    addq %rbp, %rax\n",
        "    addq $16, %rsp\n",
        "    popq %rbp\n",
        "    ret",
    )));
    assert_eq!(count(&out, "movzbl %r15b, %ebp"), 0, "{}", out);
    assert_eq!(count(&out, "movb %r15b, (%r12, %r14)"), 1, "{}", out);
}

// ── G. masked 64-bit AND as an upper32-zero producer (PERF-5 residue) ────────

#[test]
fn g_andq_byte_mask_makes_32bit_copy_cover_64bit_reader_in_block() {
    // The forward `upper32_zero` scan used to clear its fact on every `andq`,
    // so the 32-bit staging copy could not fold into the 64-bit OR reader.
    // A nonnegative imm32 mask zeroes bits 32..63 for any source value, which
    // makes the copy provably redundant inside the same block.
    let out = run(&f(concat!(
        "    movzbl (%rdi, %r8), %r10d\n",
        "    andq $127, %r10\n",
        "    movl %r10d, %esi\n",
        "    shlq $7, %rdx\n",
        "    orq %rsi, %rdx\n",
        "    movq %r8, %rsi\n",
        "    ret",
    )));
    assert_eq!(count(&out, "movl %r10d, %esi"), 0, "{}", out);
    assert!(out.contains("orq %r10, %rdx"), "{}", out);
}

#[test]
fn g_andq_sign_extended_mask_does_not_prove_upper_zero() {
    // $0x8000_0000 encodes as a negative imm32 and sign-extends to an
    // all-ones high mask: the source's upper 32 bits survive, so the copy
    // feeding the 64-bit reader must remain (families may be renamed by
    // coalescing; pin the surviving dword copy and the un-folded OR reader).
    let out = run(&f(concat!(
        "    movq %rdi, %r10\n",
        "    andq $2147483648, %r10\n",
        "    movl %r10d, %esi\n",
        "    orq %rsi, %rdx\n",
        "    ret",
    )));
    assert_eq!(count(&out, "movl %edi, %esi"), 1, "{}", out);
    assert!(out.contains("orq %rsi, %rdx"), "{}", out);
    assert!(!out.contains("orq %rdi, %rdx"), "{}", out);
}
