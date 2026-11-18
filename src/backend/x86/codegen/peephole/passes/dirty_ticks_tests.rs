//! Oracle tests for pass-level dirty-tick skipping (MB-20 step 5).
//!
//! The dirty-tick tracker (`crate::backend::peephole_common::PassTicks`)
//! lets the phase-1 fixpoint skip a sub-pass whose input is bit-identical to
//! what it last ran on clean.  Its soundness rests on two invariants —
//! determinism and honest change reporting — so the oracle here is the
//! strongest one available: for inputs that exercise the major pass
//! families, the full pipeline's output with skipping enabled must equal the
//! output with skipping disabled, while `CCC_PEEPHOLE_VERIFY_DIRTY`
//! semantics (state digests around every executed pass) stay armed in both
//! runs to catch any pass that mutates the function while reporting "no
//! change".  A separate assertion proves the comparison was not vacuous:
//! at least one clean re-run must actually have been skipped.
//!
//! Modes are forced through `dirty_tick_test`'s thread-locals, never
//! `std::env::set_var` (unsafe under the multithreaded harness, edition
//! 2024).  `peephole_optimize` runs on the calling thread, so the overrides
//! are visible to the code under test — the `test_skips` constraint.

use super::super::peephole_optimize;
use super::dirty_tick_test;

/// Wrap a body in the CFI markers `FileLiveness` needs to analyse a
/// function (the `compare_branch_fusion_tests` convention: without them
/// liveness answers `None` and most passes decline, which would make the
/// equality oracle near-vacuous).
fn f(body: &str) -> String {
    format!(".text\nf:\n.cfi_startproc\n{}\n.cfi_endproc\n", body)
}

/// One input per major pass family, each shaped to actually rewrite (a
/// change in iteration 0 is what arms the skippable re-verification in
/// iteration 1).  Kept deliberately small: the whole pipeline runs every
/// pass over every snippet regardless, so each snippet also feeds the
/// scan-and-decline paths of all the families it does not trigger.
fn snippets() -> Vec<(&'static str, String)> {
    vec![
        (
            "move relay + lea fold",
            f("movq %rdi, %rdx\nleaq 8(%rdx), %rcx\nmovq %rcx, %rax\nmovq (%rax), %rax\nret"),
        ),
        (
            "memory rmw",
            f("movq (%rdi), %rax\naddq $1, %rax\nmovq %rax, (%rdi)\nxorl %eax, %eax\nret"),
        ),
        (
            "setcc/test/branch boolean round-trip",
            f(
                "cmpl %esi, %edi\nsetl %al\nmovzbl %al, %eax\ntestl %eax, %eax\nje .Lz\n\
               movl $1, %eax\nret\n.Lz:\nxorl %eax, %eax\nret",
            ),
        ),
        (
            "scalar FP copy bracket",
            f(
                "movsd (%rdi), %xmm0\nmovapd %xmm0, %xmm1\naddsd %xmm1, %xmm1\n\
               movsd %xmm1, (%rdi)\nret",
            ),
        ),
        (
            "extension chain + copy relay",
            f("movzbl (%rdi), %eax\nmovzbl %al, %eax\nmovl %eax, %ecx\nmovl %ecx, %eax\nret"),
        ),
        (
            "shift into SIB",
            f("movq %rsi, %rcx\nshlq $3, %rcx\naddq %rdi, %rcx\nmovq (%rcx), %rax\nret"),
        ),
        (
            "loop with invariant load and compare/branch",
            f(
                "xorl %eax, %eax\nxorl %ecx, %ecx\n.Lloop:\nmovq (%rdi), %rdx\n\
               addq %rdx, %rax\naddq $1, %rcx\ncmpq %rsi, %rcx\njb .Lloop\nret",
            ),
        ),
        (
            "two functions in one unit",
            format!(
                ".text\nf:\n.cfi_startproc\n\
                 movq %rdi, %rcx\nmovq %rcx, %rax\nmovq (%rax), %rax\nret\n\
                 .cfi_endproc\ng:\n.cfi_startproc\n\
                 movl (%rdi), %eax\naddl $1, %eax\nmovl %eax, (%rdi)\nret\n\
                 .cfi_endproc\n"
            ),
        ),
    ]
}

/// The oracle: skipping may never change the pipeline's output, and the
/// honesty digests stay armed in both runs so a pass that mutates while
/// reporting `false` — the one lie that would make a skip unsound — panics
/// here instead of corrupting someone's build.
#[test]
fn dirty_tick_skipping_is_output_invariant() {
    let mut total_skipped = 0u64;
    for (name, input) in snippets() {
        let reference = {
            let _g = dirty_tick_test::force_modes(true, true);
            peephole_optimize(input.clone())
        };
        let skipped_out = {
            let _g = dirty_tick_test::force_modes(false, true);
            let out = peephole_optimize(input.clone());
            total_skipped += dirty_tick_test::last_skipped();
            out
        };
        assert_eq!(
            reference, skipped_out,
            "dirty-tick skipping changed the peephole output for: {name}"
        );
    }
    // Non-vacuity: at least one snippet must have reached a second phase-1
    // iteration and skipped a clean re-run; otherwise this test compared
    // eight pairs of identical no-skip executions and proved nothing.
    assert!(
        total_skipped > 0,
        "no sub-pass run was ever skipped across the whole snippet set — \
         the equality oracle is vacuous"
    );
}
