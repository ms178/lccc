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
use super::helpers::{is_shift_or_rotate, replace_reg_family};
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
                // An implicit read OR write of the destination family is an
                // unrenamable hazard: the textual rewrite cannot touch an
                // operand the instruction never names. `cqto` overwrites
                // `%rdx`, `syscall` clobbers `%rsi`/`%rdi`/`%r8`-`%r11`,
                // `cltq` reads `%rax`, `cpuid` rewrites `%rbx` — none of
                // these can be renamed and none may be missed. The central
                // oracle sees all of them; the old
                // `has_implicit_reg_usage(t) && d_fam <= 2` veto knew only a
                // hand-picked subset and silently coalesced across the rest.
                // A shift/rotate takes its variable count in %cl and nowhere
                // else: renaming that family produces `shlq %sil, %r8`, which
                // does not exist. The count is named explicitly, so the
                // implicit oracle does not cover it; check it separately.
                _ => {
                    implicit_reg_refs(t.as_bytes()) & d_mask != 0
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

        // Coalescing a value out of a callee-saved home needs coordinated
        // prologue/epilogue surgery.  This text-level pass only rewrites from
        // the copy onwards, so changing `%rbx` to `%r11`, for example, would
        // leave `pushq %rbx` paired with `popq %r11`.  Worse, the renamed value
        // would then live in a caller-saved register across calls.  Leave such
        // homes to the allocator until the coalescer can update unwind state
        // and save/restore pairs as one transaction.
        if (fstart..fend).any(|n| {
            matches!(
                infos[n].kind,
                LineKind::Push { reg } | LineKind::Pop { reg } if reg == d_fam
            )
        }) {
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
            // A call has implicit *writes* to every caller-saved register.
            // `live_after(i, S) == false` describes the original program, in
            // which D—not S—holds the value; it therefore cannot detect this
            // newly-created live range.  Reject exactly when D is live after
            // the call and S would be clobbered there.
            let source_is_caller_saved = matches!(s_fam, 0 | 1 | 2 | 6 | 7 | 8 | 9 | 10 | 11);
            if infos[n].kind == LineKind::Call
                && source_is_caller_saved
                && lv.live_after(n, d_fam) != Some(false)
            {
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
    fn the_source_family_is_not_renamed_when_it_is_read_again() {
        // Rule 2: `%rdi` is read after the copy, so coalescing must NOT rename
        // the `%rsi` family onto it -- doing so would rewrite that earlier
        // read as well and change which register the function loads from.
        //
        // The assertion deliberately targets the EARLIER read rather than the
        // survival of the copy itself. `copy_fold` legitimately retires the
        // copy afterwards by rewriting `(%rsi)` to `(%rdi)`: the two registers
        // hold the same value and `%rsi` is dead at the return, so three
        // instructions replace four. That is a strictly better result and must
        // not be mistaken for a coalescing failure.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %rdi, %rsi\n",
            "    movq (%rdi), %rax\n",
            "    addq (%rsi), %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(
            out.contains("movq (%rdi), %rax"),
            "the pre-existing read of %rdi must survive unrenamed:\n{out}"
        );
        assert!(
            !out.contains("movq (%rsi), %rax"),
            "coalescing must not rewrite the earlier read onto %rsi:\n{out}"
        );
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
    fn callee_saved_home_is_not_renamed_without_updating_its_save_pair() {
        // Reduced from gcc.c-torture/execute/20080604-1.c.  The old pass
        // deleted `movq %r11,%rbx`, rewrote the body and `popq` to %r11, but
        // left `pushq %rbx` untouched.  The call then clobbered the selected
        // pointer and the mismatched pop corrupted the caller's %r11 value.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    pushq %rbx\n",
            "    leaq object(%rip), %r11\n",
            "    movq %r11, %rbx\n",
            "    movq %r12, (%rbx)\n",
            "    call observe\n",
            "    movq %r12, (%rbx)\n",
            "    popq %rbx\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("pushq %rbx"), "{out}");
        assert!(out.contains("popq %rbx"), "{out}");
        assert!(!out.contains("popq %r11"), "{out}");
        assert!(out.contains("movq %r12, (%rbx)"), "{out}");
        assert!(!out.contains("movq %r12, (%r11)"), "{out}");
    }

    #[test]
    fn caller_saved_source_is_not_extended_across_call() {
        // The original liveness of %r11 ends at the copy; after coalescing it
        // would carry %rbx's live range across `call`, where the ABI permits
        // it to be destroyed.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    leaq object(%rip), %r11\n",
            "    movq %r11, %rbx\n",
            "    call observe\n",
            "    movq (%rbx), %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movq (%rbx), %rax"), "{out}");
        assert!(!out.contains("movq (%r11), %rax"), "{out}");
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
    #[test]
    fn rsi_home_is_not_coalesced_across_a_syscall_clobber() {
        // `syscall` reads its argument registers (among them %rsi) and
        // returns in %rax while clobbering %rcx/%r11 — all without naming
        // any of them. Coalescing a %rsi home across one would delete the
        // copy and leave the kernel's pointer argument reading whatever
        // %rsi held before it was ever written. The old hand-picked veto
        // (`has_implicit_reg_usage(t) && d_fam <= 2`) never saw this: it
        // only protected %rax/%rcx/%rdx.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %r9, %rsi\n",
            "    syscall\n",
            "    movq %rsi, %rbx\n",
            "    movq %rbx, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("syscall"), "{out}");
        assert!(
            out.contains("movq %r9, %rsi"),
            "coalescing across syscall is unsound:\n{out}"
        );
    }
}
