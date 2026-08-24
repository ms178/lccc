//! Whole-function register-copy coalescing.
//!
//! The register allocator hands parameters homes that rarely coincide with the
//! ABI registers they arrive in, so almost every function starts with a
//! shuffle:
//!
//! ```text
//! sum8:
//!     movq %rsi, %rdx        # len  -> its home
//!     movq %rdi, %rsi        # buf  -> its home
//!     ...
//!     movzbl (%rsi,%rbx), %eax
//! ```
//!
//! Neither copy can be deleted by a local pass — both destinations are live for
//! the whole function. The fix is coalescing: rename the destination family to
//! the source family everywhere and drop the copy. Here that recovers exactly
//! GCC's assignment (`buf` stays in `%rdi`, `len` in `%rsi`) and removes two
//! instructions from every such function.
//!
//! # When it is legal
//!
//! For `movq %S, %D` at index `i`, with [`FileLiveness`] available:
//!
//! 1. **`%S` is dead after `i`.** The copy is the last use of `%S`, so `%S` is
//!    free to carry `%D`'s value from here on. This subsumes every implicit ABI
//!    read the analysis models: if a later `call` reads `%rdi`, or a `ret`
//!    reads `%rax`, the register is live and the copy stays.
//! 2. **`%S` is mentioned nowhere else in the function.** A later write to
//!    `%S` would clobber the coalesced value, and a read before `i` belongs to
//!    a different live range that the rename must not disturb. (The parameter's
//!    arrival in `%S` is implicit, not a mention.)
//! 3. **The copy sits in the straight-line entry run** (no label, branch, call
//!    or return before it). Every earlier line then executes exactly once and
//!    cannot be re-entered through a back edge, so the other half of a
//!    parameter shuffle — which mentions both families — stays correct.
//! 4. **The copy is a full 64-bit register move.** A `movl` copy zero-extends;
//!    renaming would let a later 64-bit read of `%D` see `%S`'s upper half.
//! 5. **No high-byte alias.** `%ah`/`%dh` have no counterpart once the family
//!    moves to `%rsi`/`%rdi`/`%r8`..., so any such occurrence blocks the rename.
//! 6. **No implicit reader of `%D` after the copy** — `ret` (`%rax:%rdx`), a
//!    call or tail jump (argument registers, `%rax`, static chain `%r10`), or
//!    div/mul/`cltq`/shift traffic on `%rax`/`%rcx`/`%rdx`. Those reads cannot
//!    be renamed.
//! 7. `%rsp`/`%rbp` are never renamed (frame registers), and neither side may
//!    be a register the function's inline assembly or an unanalysable transfer
//!    could observe — both are already excluded by rule 1 via the conservative
//!    liveness model.

use super::super::types::*;
use super::helpers::{has_implicit_reg_usage, is_shift_or_rotate, replace_reg_family};
use super::liveness::FileLiveness;
use super::relay_and_lea::{
    function_range, is_relayable_family, plain_gp_operand, split_two_operands,
};

/// High-byte register names that cannot survive a family rename.
const HIGH_BYTE: &[&str] = &["%ah", "%bh", "%ch", "%dh"];

#[allow(clippy::needless_range_loop)]
pub(super) fn coalesce_register_copies(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut lv = FileLiveness::new(store, infos);
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        let line = infos[i].trimmed(store.get(i)).to_string();
        // Rule 4: only the full 64-bit copy.
        let Some(rest) = line.strip_prefix("movq ") else {
            i += 1;
            continue;
        };
        let Some((src_text, dst_text)) = split_two_operands(rest) else {
            i += 1;
            continue;
        };
        let (Some(s_fam), Some(d_fam)) = (plain_gp_operand(src_text), plain_gp_operand(dst_text))
        else {
            i += 1;
            continue;
        };
        if s_fam == d_fam || !is_relayable_family(s_fam) || !is_relayable_family(d_fam) {
            i += 1;
            continue;
        }
        // Rule 1: the source must be dead after the copy. Only the exact
        // dataflow answer is accepted — the syntactic proofs say nothing about
        // paths that re-enter this code.
        if lv.live_after(i, s_fam) != Some(false) {
            i += 1;
            continue;
        }
        let Some((fstart, fend)) = function_range(store, infos, i) else {
            i += 1;
            continue;
        };
        let s_mask = 1u16 << s_fam;
        let d_mask = 1u16 << d_fam;
        let mut ok = true;

        // The copy must sit in the straight-line entry run: no label, branch,
        // call or return between the function start and `i`. Then every line
        // before the copy executes exactly once and cannot be re-entered
        // through a back edge, which is what makes the earlier mentions of
        // either family (the other half of a parameter shuffle) harmless.
        for n in fstart..i {
            if infos[n].is_nop() || infos[n].kind == LineKind::Directive {
                continue;
            }
            if matches!(
                infos[n].kind,
                LineKind::Label
                    | LineKind::Jmp
                    | LineKind::CondJmp
                    | LineKind::JmpIndirect
                    | LineKind::Call
                    | LineKind::Ret
                    | LineKind::InlineAsm
            ) {
                ok = false;
                break;
            }
        }

        // Rule 6: the DESTINATION family must have no implicit reader after the
        // copy. Renaming rewrites every explicit occurrence of `%D`, but an
        // implicit ABI read cannot be rewritten: `ret` reads `%rax:%rdx`, a
        // call or tail jump reads the argument registers, `%rax` and the
        // static chain `%r10`, and div/mul/cltq/shift traffic reads
        // `%rax`/`%rcx`/`%rdx`. Coalescing `movq %r10, %rax` in front of a
        // `ret` would move the return value into `%r10`.
        for n in i + 1..fend {
            if infos[n].is_nop() {
                continue;
            }
            let t = infos[n].trimmed(store.get(n));
            let implicit_hazard = match infos[n].kind {
                LineKind::Ret => {
                    d_fam == 0
                        || (d_fam == 2
                            && !FileLiveness::returns_in_rax_only(store, infos, fstart, fend))
                }
                LineKind::Call | LineKind::JmpIndirect => {
                    matches!(d_fam, 0 | 1 | 2 | 6 | 7 | 8 | 9 | 10)
                }
                LineKind::Jmp => {
                    let target = t.trim_start_matches(|c: char| c != ' ').trim();
                    !target.starts_with('.') && matches!(d_fam, 0 | 1 | 2 | 6 | 7 | 8 | 9 | 10)
                }
                // A shift/rotate takes its variable count in %cl and nowhere
                // else: renaming that family produces `shlq %sil, %r8`, which
                // does not exist. `has_implicit_reg_usage` does not cover the
                // plain shifts, so check them separately.
                _ => {
                    (has_implicit_reg_usage(t) && d_fam <= 2)
                        || (d_fam == 1 && is_shift_or_rotate(t))
                }
            };
            if implicit_hazard {
                ok = false;
                break;
            }
        }
        if !ok {
            i += 1;
            continue;
        }

        for n in i + 1..fend {
            if infos[n].is_nop() {
                continue;
            }
            // Rule 2: no later mention of the source family. A read is already
            // excluded by rule 1; a write would clobber the coalesced value.
            if infos[n].reg_refs & s_mask != 0 {
                ok = false;
                break;
            }
            // Rule 5: opaque text or a high-byte alias of the destination.
            if infos[n].reg_refs & d_mask != 0 {
                let t = infos[n].trimmed(store.get(n));
                if infos[n].kind == LineKind::InlineAsm
                    || infos[n].pinned
                    || HIGH_BYTE.iter().any(|h| t.contains(h))
                {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            i += 1;
            continue;
        }
        // Rewrite every occurrence of the destination family after the copy.
        for n in i + 1..fend {
            if infos[n].is_nop() || infos[n].reg_refs & d_mask == 0 {
                continue;
            }
            let t = infos[n].trimmed(store.get(n));
            let renamed = replace_reg_family(t, d_fam, s_fam);
            if renamed != t {
                replace_line(store, &mut infos[n], n, format!("    {}", renamed));
            }
        }
        mark_nop(&mut infos[i]);
        lv.refresh_at(store, infos, i);
        changed = true;
        i += 1;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::super::super::peephole_optimize;

    fn run(asm: &str) -> String {
        peephole_optimize(asm.to_string())
    }

    #[test]
    fn parameter_shuffle_is_coalesced_away() {
        let out = run(concat!(
            "sum8:\n",
            ".cfi_startproc\n",
            "    pushq %rbx\n",
            "    movq %rsi, %rdx\n",
            "    movq %rdi, %rsi\n",
            "    xorl %r8d, %r8d\n",
            "    xorl %ebx, %ebx\n",
            ".LBB1:\n",
            "    cmpl %edx, %ebx\n",
            "    jae .LBB3\n",
            ".LBB2:\n",
            "    movzbl (%rsi,%rbx), %eax\n",
            "    addl %eax, %r8d\n",
            "    addl $1, %ebx\n",
            "    jmp .LBB1\n",
            ".LBB3:\n",
            "    movl %r8d, %eax\n",
            "    popq %rbx\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movq %rsi, %rdx"), "{out}");
        assert!(!out.contains("movq %rdi, %rsi"), "{out}");
        assert!(out.contains("movzbl (%rdi,%rbx), %eax"), "{out}");
        assert!(out.contains("cmpl %esi, %ebx"), "{out}");
    }

    #[test]
    fn rdx_is_not_coalesced_when_the_epilogue_returns_through_it() {
        // 128-bit return: both halves are set explicitly, so %rdx carries part
        // of the return value and must not be renamed away.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %rsi, %rdx\n",
            "    movq %rdi, %rax\n",
            "    addq %rdx, %rax\n",
            "    movq %rdx, %rdx\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movq %rsi, %rdx"), "{out}");
    }

    #[test]
    fn rcx_is_not_coalesced_when_a_shift_needs_cl() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %rdi, %rcx\n",
            "    shlq %cl, %r8\n",
            "    movq %r8, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("shlq %cl, %r8"), "{out}");
        assert!(out.contains("movq %rdi, %rcx"), "{out}");
    }

    #[test]
    fn rax_is_never_coalesced() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    leaq 1(%rbx), %r10\n",
            "    movzbl (%r10), %r14d\n",
            "    movq %r10, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("%rax"), "{out}");
    }

    #[test]
    fn copy_is_kept_when_the_source_is_read_again() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %rdi, %rsi\n",
            "    movq (%rdi), %rax\n",
            "    addq (%rsi), %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movq %rdi, %rsi"), "{out}");
    }

    #[test]
    fn copy_is_kept_when_the_source_is_an_argument_of_a_later_call() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %rdi, %rbx\n",
            "    call bar\n",
            "    movq %rbx, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        // %rdi is live into the call (first argument), so the save must stay.
        assert!(out.contains("movq %rdi, %rbx"), "{out}");
    }

    #[test]
    fn copy_is_kept_when_the_destination_is_used_before_it() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            ".LBB1:\n",
            "    addq %rdx, %rax\n",
            "    movq %rdi, %rdx\n",
            "    cmpq $10, %rax\n",
            "    jl .LBB1\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movq %rdi, %rdx"), "{out}");
    }
}
