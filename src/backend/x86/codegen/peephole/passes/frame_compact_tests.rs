//! Tests for [`super::frame_compact`] — dead leaf-frame removal.
//!
//! Structure: positives prove the fold fires for genuinely dead frames;
//! negatives prove each legality rule actually blocks. Every negative here
//! corresponds to a real corruption class, not a hypothetical: the
//! mid-body `addq $N, %rsp` shape (WO-1, red-team audit 2026-09-18) would
//! have been NOP'd by string-identity epilogue classification — deleting a
//! stack adjustment whose matching `subq` never existed on that path.
//!
//! Like `narrow_copy_fold_tests`, these drive the WHOLE peephole pipeline
//! (`peephole_optimize`) rather than `remove_dead_leaf_frame` alone: a
//! fold that is individually sound but that a neighbouring pass then
//! misreads is still a bug.

use super::super::peephole_optimize;

fn run(input: &str) -> String {
    peephole_optimize(input.to_string())
}

/// Wrap a body in the CFI markers `FileLiveness` needs to analyse a function.
/// Without them liveness answers `None` and every fold is skipped, so a test
/// that forgets this passes vacuously.
fn f(body: &str) -> String {
    format!(
        ".text\nf:\n.cfi_startproc\n{}\n.cfi_endproc\n.size f, .-f\n",
        body
    )
}

fn count(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

/// Count lines that are exactly `ret` (a `.Lret…` label must not match).
fn count_rets(hay: &str) -> usize {
    hay.lines().filter(|l| l.trim() == "ret").count()
}

// ── A. positives: genuinely dead leaf frames fold ──────────────────────────

#[test]
fn a_dead_leaf_frame_with_one_paired_epilogue_folds() {
    // Prologue subq, body that never names %rsp, addq+ret exit: the frame
    // is dead (the slots the RA reserved were retired by later passes).
    let out = run(&f(concat!(
        "    subq $16, %rsp\n",
        "    .cfi_def_cfa_offset 24\n",
        "    movl %edi, %eax\n",
        "    addl %esi, %eax\n",
        "    addq $16, %rsp\n",
        "    ret",
    )));
    assert_eq!(count(&out, "subq $16, %rsp"), 0, "{}", out);
    assert_eq!(count(&out, "addq $16, %rsp"), 0, "{}", out);
    // The body itself must survive: as written, or as the equivalent
    // two-source lea the copy+add fold makes of it (flags dead at `ret`).
    assert_eq!(
        count(&out, "addl %esi, %eax") + count(&out, "leal (%rdi, %rsi), %eax"),
        1,
        "{}",
        out
    );
}

#[test]
fn a_dead_leaf_frame_with_two_paired_epilogues_folds_both() {
    let out = run(&f(concat!(
        "    subq $16, %rsp\n",
        "    testl %edi, %edi\n",
        "    je .Lret1\n",
        "    movl %edi, %eax\n",
        "    addq $16, %rsp\n",
        "    ret\n",
        ".Lret1:\n",
        "    xorl %eax, %eax\n",
        "    addq $16, %rsp\n",
        "    ret",
    )));
    assert_eq!(count(&out, "subq $16, %rsp"), 0, "{}", out);
    assert_eq!(count(&out, "addq $16, %rsp"), 0, "{}", out);
    assert_eq!(count_rets(&out), 2, "{}", out);
}

#[test]
fn an_epilogue_separated_from_ret_by_directives_folds() {
    // The forward scan skips directives/NOPs exactly like the backward
    // ret-pairing scan: a .cfi_restore_state between addq and ret must
    // not turn a real epilogue into a decline.
    let out = run(&f(concat!(
        "    subq $16, %rsp\n",
        "    movl %edi, %eax\n",
        "    addq $16, %rsp\n",
        "    .cfi_def_cfa_offset 8\n",
        "    ret",
    )));
    assert_eq!(count(&out, "subq $16, %rsp"), 0, "{}", out);
    assert_eq!(count(&out, "addq $16, %rsp"), 0, "{}", out);
}

// ── B. negatives: the WO-1 mid-body addq class and friends ─────────────────

#[test]
fn a_mid_body_addq_matching_the_prologue_spelling_declines_the_fold() {
    // SOUNDNESS (WO-1): an `addq $16, %rsp` in the middle of the body that
    // is NOT followed by a ret is a stack readjustment on some path — a
    // future emitter's alloca cleanup or manual stack ping-pong. String
    // identity classified it as an "epilogue" and the removal NOP'd it,
    // leaving the stack unbalanced on the path through the label. The
    // ret-paired classification must decline the WHOLE fold: the prologue
    // subq and every addq stay.
    let out = run(&f(concat!(
        "    subq $16, %rsp\n",
        "    movl %edi, %eax\n",
        "    addq $16, %rsp\n",
        ".Ladjust:\n",
        "    subq $16, %rsp\n",
        "    movl %esi, %eax\n",
        "    addq $16, %rsp\n",
        "    ret",
    )));
    assert_eq!(
        count(&out, "subq $16, %rsp"),
        2,
        "mid-body addq must decline the fold:\n{}",
        out
    );
    assert_eq!(
        count(&out, "addq $16, %rsp"),
        2,
        "no addq may be NOP'd when the fold declines:\n{}",
        out
    );
}

#[test]
fn an_addq_followed_by_a_label_then_ret_declines_the_fold() {
    // A label between the addq and the ret means control can reach the ret
    // without crossing the addq (a jump to the label). The forward scan
    // stops at the label (it skips only NOPs/directives) and declines.
    let out = run(&f(concat!(
        "    subq $16, %rsp\n",
        "    movl %edi, %eax\n",
        "    addq $16, %rsp\n",
        ".Lexit:\n",
        "    ret",
    )));
    assert_eq!(
        count(&out, "subq $16, %rsp"),
        1,
        "label-separated addq must decline:\n{}",
        out
    );
    assert_eq!(count(&out, "addq $16, %rsp"), 1, "{}", out);
}

#[test]
fn an_unpaired_ret_declines_the_fold() {
    // A ret whose preceding real line is not a collected epilogue would
    // return with an unbalanced stack under the fold.
    let out = run(&f(concat!(
        "    subq $16, %rsp\n",
        "    testl %edi, %edi\n",
        "    je .Lskip\n",
        "    addq $16, %rsp\n",
        "    ret\n",
        ".Lskip:\n",
        "    movl %edi, %eax\n",
        "    ret",
    )));
    assert_eq!(count(&out, "subq $16, %rsp"), 1, "{}", out);
    assert_eq!(count(&out, "addq $16, %rsp"), 1, "{}", out);
}

#[test]
fn any_other_rsp_reference_declines_the_fold() {
    // A lea through %rsp is observable (it IS the return value here) and
    // no earlier pass eliminates it — unlike a dead store/load pair
    // through (%rsp), which dead-write elimination removes before this
    // pass ever runs (found by driving the whole pipeline, not the pass
    // in isolation).
    let out = run(&f(concat!(
        "    subq $16, %rsp\n",
        "    leaq 8(%rsp), %rax\n",
        "    addq $16, %rsp\n",
        "    ret",
    )));
    assert_eq!(count(&out, "subq $16, %rsp"), 1, "{}", out);
    assert_eq!(count(&out, "addq $16, %rsp"), 1, "{}", out);
    assert_eq!(count(&out, "leaq 8(%rsp), %rax"), 1, "{}", out);
}

// ── C. the alignment-frame shrink path stays gated on the same rules ──────

#[test]
fn an_alignment_frame_with_a_mid_body_addq_declines_too() {
    // Calls present + N ≡ 8 (mod 16) would shrink to subq $8; the same
    // mid-body addq hazard applies — the shrink rewrites every collected
    // addq, so a misclassified mid-body addq would corrupt the stack by
    // rewriting an adjustment that must keep its full size.
    let out = run(&f(concat!(
        "    subq $24, %rsp\n",
        "    call g\n",
        "    addq $24, %rsp\n",
        ".Lmid:\n",
        "    subq $24, %rsp\n",
        "    call h\n",
        "    addq $24, %rsp\n",
        "    ret",
    )));
    // The FIRST addq $24 (before the label) is not ret-paired → decline.
    assert_eq!(
        count(&out, "subq $24, %rsp"),
        2,
        "mid-body addq must decline the shrink:\n{}",
        out
    );
    assert_eq!(count(&out, "subq $8, %rsp"), 0, "{}", out);
    assert_eq!(count(&out, "addq $8, %rsp"), 0, "{}", out);
}

// ── TLS descriptors: register-wise not a call, stack-wise a call ───────────

#[test]
fn a_tls_descriptor_call_keeps_the_alignment_frame_and_its_live_values() {
    // The shape lccc -O2 -fPIC emits for `return sl * 10 + gl;` with two
    // dynamic-TLS variables, minus the volatile-region markers, so the
    // classification alone must protect it. Two independent bugs were found:
    //  * the descriptor call was classified as a full call, so the
    //    dead-write pass deleted `imull` (r10 "clobbered" by the call) —
    //    yet the descriptor ABI preserves every register except %rax;
    //  * once it was classified `Other`, the leaf-frame pass deleted the
    //    `subq $8` alignment frame, leaving %rsp ≡ 8 (mod 16) at the call.
    let out = run(&f(concat!(
        "    subq $8, %rsp\n",
        "    .cfi_def_cfa_offset 16\n",
        "    leaq sl@TLSDESC(%rip), %rax\n",
        "    lccc_tls_call sl\n",
        "    addq %fs:0, %rax\n",
        "    movl (%rax), %r10d\n",
        "    imull $10, %r10d, %r10d\n",
        "    leaq gl@TLSDESC(%rip), %rax\n",
        "    lccc_tls_call gl\n",
        "    addq %fs:0, %rax\n",
        "    addl (%rax), %r10d\n",
        "    movl %r10d, %eax\n",
        "    addq $8, %rsp\n",
        "    ret",
    )));
    assert_eq!(count(&out, "subq $8, %rsp"), 1, "{}", out);
    assert_eq!(count(&out, "addq $8, %rsp"), 1, "{}", out);
    assert_eq!(count(&out, "imull $10, %r10d, %r10d"), 1, "{}", out);
    for line in [
        "leaq sl@TLSDESC(%rip), %rax",
        "lccc_tls_call sl",
        "leaq gl@TLSDESC(%rip), %rax",
        "lccc_tls_call gl",
    ] {
        assert_eq!(count(&out, line), 1, "{line}\n{out}");
    }
}

// ── D. callee-saved pushes before the frame, and exact CFI on every row ────

/// The CFA rows of `out`, in order.
fn cfa_rows(out: &str) -> Vec<i64> {
    out.lines()
        .filter_map(|l| l.trim().strip_prefix(".cfi_def_cfa_offset "))
        .map(|v| v.trim().parse().unwrap())
        .collect()
}

/// A pointer walk that keeps `%rbx` live (the FPO dead-save pass must not
/// remove the push these tests are about).
const RBX_LOOP: &str = concat!(
    "    movq %rdi, %rbx\n",
    "    xorl %eax, %eax\n",
    ".Lloop:\n",
    "    addq (%rbx), %rax\n",
    "    addq $8, %rbx\n",
    "    decq %rsi\n",
    "    jne .Lloop\n",
);

#[test]
fn a_dead_frame_in_a_multi_exit_leaf_rewrites_the_post_ret_cfa_row() {
    // The codegen restates the full-frame CFA after a mid-body `ret` for
    // the block that follows. Rewriting only the prologue's row left
    // `.cfi_def_cfa_offset 24` there while %rsp sat at CFA-8: an unwinder
    // (gdb, perf, -fexceptions) read the return address from the wrong slot.
    let out = run(&f(concat!(
        "    subq $16, %rsp\n",
        "    .cfi_def_cfa_offset 24\n",
        "    testl %edi, %edi\n",
        "    je .Lzero\n",
        "    movl %edi, %eax\n",
        "    addq $16, %rsp\n",
        "    .cfi_def_cfa_offset 8\n",
        "    ret\n",
        "    .cfi_def_cfa_offset 24\n",
        ".Lzero:\n",
        "    movl %esi, %eax\n",
        "    addq $16, %rsp\n",
        "    .cfi_def_cfa_offset 8\n",
        "    ret",
    )));
    assert_eq!(count(&out, "subq $16, %rsp"), 0, "{}", out);
    assert_eq!(count(&out, "addq $16, %rsp"), 0, "{}", out);
    assert!(
        cfa_rows(&out).iter().all(|&v| v == 8),
        "stale CFA row:\n{}",
        out
    );
}

#[test]
fn a_dead_frame_under_live_pushes_folds_with_exact_cfi() {
    // `wsum`/`utf8_scan` shape: a live callee-saved push, then a frame no
    // line references, two exits.
    let out = run(&f(&format!(
        "{}{}{}",
        concat!(
            "    pushq %rbx\n",
            "    .cfi_def_cfa_offset 16\n",
            "    .cfi_offset %rbx, -16\n",
            "    subq $16, %rsp\n",
            "    .cfi_def_cfa_offset 32\n",
            "    testq %rsi, %rsi\n",
            "    je .Lnone\n",
        ),
        RBX_LOOP,
        concat!(
            "    addq $16, %rsp\n",
            "    .cfi_def_cfa_offset 16\n",
            "    popq %rbx\n",
            "    .cfi_def_cfa_offset 8\n",
            "    ret\n",
            "    .cfi_def_cfa_offset 32\n",
            ".Lnone:\n",
            "    xorl %eax, %eax\n",
            "    addq $16, %rsp\n",
            "    .cfi_def_cfa_offset 16\n",
            "    popq %rbx\n",
            "    .cfi_def_cfa_offset 8\n",
            "    ret",
        ),
    )));
    assert_eq!(count(&out, "subq $16, %rsp"), 0, "{}", out);
    assert_eq!(count(&out, "addq $16, %rsp"), 0, "{}", out);
    assert_eq!(count(&out, "pushq %rbx"), 1, "{}", out);
    assert_eq!(count(&out, "popq %rbx"), 2, "{}", out);
    assert_eq!(count(&out, ".cfi_offset %rbx, -16"), 1, "{}", out);
    // push → 16; first exit pops → 8; the second block restates 16 and
    // pops → 8. No 32 survives and no row restates the value in effect.
    assert_eq!(cfa_rows(&out), vec![16, 8, 16, 8], "{}", out);
}

#[test]
fn a_calling_frame_under_one_push_is_deleted_when_the_push_aligns() {
    // 8 (return address) + 8 (push) ≡ 0 (mod 16): the 16-byte frame is pure
    // padding even with a call.
    let out = run(&f(concat!(
        "    pushq %rbx\n",
        "    .cfi_def_cfa_offset 16\n",
        "    .cfi_offset %rbx, -16\n",
        "    subq $16, %rsp\n",
        "    .cfi_def_cfa_offset 32\n",
        "    movq %rdi, %rbx\n",
        "    call g\n",
        "    addq %rbx, %rax\n",
        "    addq $16, %rsp\n",
        "    .cfi_def_cfa_offset 16\n",
        "    popq %rbx\n",
        "    .cfi_def_cfa_offset 8\n",
        "    ret",
    )));
    assert_eq!(count(&out, "subq $16, %rsp"), 0, "{}", out);
    assert_eq!(count(&out, "addq $16, %rsp"), 0, "{}", out);
    assert_eq!(cfa_rows(&out), vec![16, 8], "{}", out);
}

#[test]
fn a_calling_frame_under_two_pushes_shrinks_to_the_alignment_slot() {
    // 8 + 16 + 24 = 48 ≡ 0 (mod 16); the minimum with that residue is 8.
    let out = run(&f(concat!(
        "    pushq %rbx\n",
        "    .cfi_def_cfa_offset 16\n",
        "    .cfi_offset %rbx, -16\n",
        "    pushq %r12\n",
        "    .cfi_def_cfa_offset 24\n",
        "    .cfi_offset %r12, -24\n",
        "    subq $24, %rsp\n",
        "    .cfi_def_cfa_offset 48\n",
        "    movq %rdi, %rbx\n",
        "    movq %rsi, %r12\n",
        "    call g\n",
        "    addq %rbx, %rax\n",
        "    addq %r12, %rax\n",
        "    addq $24, %rsp\n",
        "    .cfi_def_cfa_offset 24\n",
        "    popq %r12\n",
        "    .cfi_def_cfa_offset 16\n",
        "    popq %rbx\n",
        "    .cfi_def_cfa_offset 8\n",
        "    ret",
    )));
    assert_eq!(count(&out, "subq $8, %rsp"), 1, "{}", out);
    assert_eq!(count(&out, "addq $8, %rsp"), 1, "{}", out);
    assert_eq!(count(&out, "$24, %rsp"), 0, "{}", out);
    assert_eq!(cfa_rows(&out), vec![16, 24, 32, 24, 16, 8], "{}", out);
}

#[test]
fn pushed_frame_refusals() {
    let framed = |prologue: &str, epilogue: &str| f(&format!("{prologue}{RBX_LOOP}{epilogue}"));
    let push_sub = concat!(
        "    pushq %rbx\n",
        "    .cfi_def_cfa_offset 16\n",
        "    .cfi_offset %rbx, -16\n",
        "    subq $16, %rsp\n",
        "    .cfi_def_cfa_offset 32\n",
    );
    let exit = concat!(
        "    addq $16, %rsp\n",
        "    .cfi_def_cfa_offset 16\n",
        "    popq %rbx\n",
        "    .cfi_def_cfa_offset 8\n",
        "    ret",
    );
    let cases = [
        (
            "misaligned call frame (8 + 8 + 8 is not a multiple of 16)",
            framed(
                concat!(
                    "    pushq %rbx\n",
                    "    .cfi_def_cfa_offset 16\n",
                    "    .cfi_offset %rbx, -16\n",
                    "    subq $8, %rsp\n",
                    "    .cfi_def_cfa_offset 24\n",
                    "    call g\n",
                ),
                concat!(
                    "    addq $8, %rsp\n",
                    "    .cfi_def_cfa_offset 16\n",
                    "    popq %rbx\n",
                    "    .cfi_def_cfa_offset 8\n",
                    "    ret",
                ),
            ),
            "$8, %rsp",
        ),
        (
            "an epilogue that pops fewer registers than were pushed",
            framed(push_sub, "    addq $16, %rsp\n    ret"),
            "$16, %rsp",
        ),
        (
            "a body pop outside any epilogue",
            framed(push_sub, &format!("    pushq %rax\n    popq %rcx\n{exit}")),
            "$16, %rsp",
        ),
        (
            "a frame-pointer prologue",
            framed(
                concat!(
                    "    pushq %rbp\n",
                    "    movq %rsp, %rbp\n",
                    "    pushq %rbx\n",
                    "    subq $16, %rsp\n",
                ),
                "    addq $16, %rsp\n    popq %rbx\n    popq %rbp\n    ret",
            ),
            "$16, %rsp",
        ),
        (
            "a CFA row between the push region and the frame total",
            framed(push_sub, &format!("    .cfi_def_cfa_offset 24\n{exit}")),
            "$16, %rsp",
        ),
    ];
    for (why, input, kept) in cases {
        let out = run(&input);
        assert_eq!(count(&out, &format!("subq {kept}")), 1, "{why}:\n{out}");
        assert_eq!(count(&out, &format!("addq {kept}")), 1, "{why}:\n{out}");
    }
}
