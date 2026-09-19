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
    // The body itself must survive.
    assert_eq!(count(&out, "addl %esi, %eax"), 1, "{}", out);
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
