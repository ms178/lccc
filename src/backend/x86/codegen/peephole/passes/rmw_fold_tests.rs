//! Tests for the commutative RMW swap fold ([`relay_and_lea::fold_rmw_into_copy`])
//! and the windowed [`relay_and_lea::fold_copy_into_lea_base`].
//!
//! Positives pin the two real-world shapes that motivated the passes (the
//! fnv-1a accumulator round-trip and the zstd clamp_sum lea/copy window with
//! an unrelated compare in between). Negatives pin every legality rule: each
//! one corresponds to a silent-miscompile hazard if it ever stops blocking.

use super::super::peephole_optimize;

fn run(input: &str) -> String {
    peephole_optimize(input.to_string())
}

/// Wrap a body in the CFI markers `FileLiveness` needs to analyse a function.
fn f(body: &str) -> String {
    format!(".text\nf:\n.cfi_startproc\n{}\n.cfi_endproc\n", body)
}

fn count(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

// ── A. commutative RMW swap ──────────────────────────────────────────────────

#[test]
fn a_xor_round_trip_swaps_into_the_accumulator() {
    // The fnv-1a hot-loop shape: the op's source IS the copy destination.
    let out = run(&f(
        "    xorq %r9, %rsi\n    movq %rsi, %r9\n    imulq %r8, %r9\n    ret",
    ));
    assert_eq!(count(&out, "xorq %rsi, %r9"), 1, "{}", out);
    assert_eq!(count(&out, "movq %rsi, %r9"), 0, "{}", out);
}

#[test]
fn a_addl_round_trip_with_widening_copy_folds() {
    // 32-bit op zero-extends, so the 64-bit copy may fold.
    let out = run(&f("    addl %r8d, %esi\n    movq %rsi, %r8\n    ret"));
    assert_eq!(count(&out, "addl %esi, %r8d"), 1, "{}", out);
    assert_eq!(count(&out, "movq %rsi, %r8"), 0, "{}", out);
}

#[test]
fn a_64_bit_op_under_a_32_bit_copy_is_rejected() {
    // SOUNDNESS: the copy truncates to 32 bits; the swapped 64-bit op would
    // keep the upper half in %r8.
    let out = run(&f(
        "    xorq %r9, %rsi\n    movl %esi, %r8d\n    movl %r8d, %eax\n    ret",
    ));
    // The swap fold must not fire. (Neighbouring dead-copy passes may
    // forward the dead %r8d — legal — so only the swap text is forbidden.)
    assert_eq!(count(&out, "xorq %rsi, %r9"), 0, "{}", out);
}

#[test]
fn a_live_temp_blocks_the_fold() {
    // SOUNDNESS: %rsi is read after the copy — the op must keep its own
    // destination and the copy must stay.
    let out = run(&f(
        "    xorq %r9, %rsi\n    movq %rsi, %r9\n    movq %rsi, %rax\n    ret",
    ));
    assert_eq!(count(&out, "xorq %rsi, %r9"), 0, "{}", out);
    // %rax must still receive %rsi's value (the pre-op one), never %r9's.
    assert_eq!(count(&out, "movq %rsi, %rax"), 1, "{}", out);
    assert_eq!(count(&out, "movq %r9, %rax"), 0, "{}", out);
}

#[test]
fn a_sub_is_never_swapped() {
    // SOUNDNESS: `subq %r9,%rsi` computes rsi = rsi - r9; swapping computes
    // r9 = r9 - rsi — a different value. `sub` stays out of the table.
    let out = run(&f("    subq %r9, %rsi\n    movq %rsi, %r9\n    ret"));
    assert_eq!(count(&out, "subq %r9, %rsi"), 1, "{}", out);
}

#[test]
fn an_op_whose_source_is_not_the_copy_dest_is_rejected() {
    // SOUNDNESS: `xorq %r8,%rsi; movq %rsi,%r9` — swapping would compute
    // r8 = r8 ^ rsi, but %r8's old value was never an input of the original.
    let out = run(&f("    xorq %r8, %rsi\n    movq %rsi, %r9\n    ret"));
    assert_eq!(count(&out, "xorq %rsi, %r8"), 0, "{}", out);
}

#[test]
fn a_barrier_between_op_and_copy_blocks_the_fold() {
    // The temp may be re-defined on the fallthrough path; the proof must not
    // reach across the label.
    let out = run(&f("    xorq %r9, %rsi\n.L1:\n    movq %rsi, %r9\n    ret"));
    assert_eq!(count(&out, "xorq %rsi, %r9"), 0, "{}", out);
}

// ── B. windowed lea-base fold ────────────────────────────────────────────────

#[test]
fn b_lea_copy_with_intervening_compare_folds() {
    // The zstd clamp_sum shape: the compare reads the LEA temp between the
    // LEA and its copy — the window must rename it like the post-copy reads.
    let out = run(&f(
        "    leaq (%r8,%rsi), %r11\n    cmpq $1000, %r11\n    movq %r11, %r8\n    cmovaq %rcx, %r8\n    ret",
    ));
    assert_eq!(count(&out, "movq %r11, %r8"), 0, "{}", out);
    assert_eq!(count(&out, "cmpq $1000, %r8"), 1, "{}", out);
}

#[test]
fn b_intervening_read_of_the_copy_dest_blocks_the_fold() {
    // SOUNDNESS: the unrelated line reads %r8's OLD value; after the
    // retarget %r8 would already hold the LEA result.
    let out = run(&f(
        "    leaq (%r8,%rsi), %r11\n    addq %r8, %rax\n    movq %r11, %r8\n    ret",
    ));
    // The retargeted form must not exist; neighbouring dead-store passes
    // may delete the dead chain entirely (legal — nothing reads it here).
    assert_eq!(count(&out, "leaq (%r8,%rsi), %r8"), 0, "{}", out);
}

#[test]
fn b_intervening_write_of_the_temp_blocks_the_fold() {
    // SOUNDNESS: the LEA temp is redefined before its copy — the copy moves
    // the REDEFINITION, not the LEA result.
    let out = run(&f(
        "    leaq (%r8,%rsi), %r11\n    leaq 4(%r11), %r11\n    movq %r11, %r8\n    ret",
    ));
    assert_eq!(count(&out, "leaq (%r8,%rsi), %r8"), 0, "{}", out);
}
