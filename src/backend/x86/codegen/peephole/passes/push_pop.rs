//! Push/pop pair elimination passes.
//!
//! Eliminates redundant push/pop pairs and push/binop/move/pop patterns
//! that arise from the stack-based codegen model.

use super::super::types::*;

/// Longest span a save/restore pair may cover. A span may only be searched for
/// a matching `pop` while every line in it is steppable (`LineInfo::is_barrier`
/// says otherwise), so widening the window cannot change the proof obligations:
/// it only lets the scan reach pairs that a longer provably-straight-line span
/// hides.
const PUSH_POP_WINDOW: usize = 8;

pub(super) fn eliminate_push_pop_pairs(store: &LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let len = store.len();

    for i in 0..len.saturating_sub(2) {
        // A pinned line is one the program's own contract owns -- a C
        // `volatile` access, a parameter pre-store, an inline-asm region -- so
        // no pass may delete it, whatever the span proves.
        if infos[i].pinned {
            continue;
        }
        let push_reg_id = match infos[i].kind {
            LineKind::Push { reg } if reg != REG_NONE => reg,
            _ => continue,
        };

        for j in (i + 1)..std::cmp::min(i + 1 + PUSH_POP_WINDOW, len) {
            if infos[j].is_nop() {
                continue;
            }
            if let LineKind::Pop { reg: pop_reg_id } = infos[j].kind {
                // The `pop` that ends the span is a `Pop` either way, so the
                // scan breaks below whether or not it is the one being
                // removed; only removal needs it unpinned. A `.cfi_` adjustment
                // hanging off the `pop` describes the slot the pair owns, so
                // that keeps it too -- deleting the pair would leave the
                // unwinder pointing at a byte nobody moved.
                if pop_reg_id == push_reg_id && !infos[j].pinned && !cfi_follows(store, infos, j) {
                    let mut safe = true;
                    for k in (i + 1)..j {
                        if infos[k].is_nop() {
                            continue;
                        }
                        if instruction_modifies_reg_id(store.get(k), &infos[k], push_reg_id) {
                            safe = false;
                            break;
                        }
                        // Check for instructions that implicitly modify the stack
                        // (pushfq, popfq, pushfl, popfl, etc.) -- these read/write
                        // the stack slot that our push placed data on, so eliminating
                        // the push/pop pair would be incorrect.
                        if instruction_modifies_stack(store.get(k), &infos[k]) {
                            safe = false;
                            break;
                        }
                    }
                    if safe {
                        mark_nop(&mut infos[i]);
                        mark_nop(&mut infos[j]);
                        changed = true;
                    }
                }
                break;
            }
            // Any line a locality window may not step over also ends this
            // scan, and asking `is_barrier` instead of re-listing the kinds is
            // what makes the answer complete:
            //   * a `Label` is a second entry point into the span, so the `pop`
            //     is reachable without the `push` having run -- deleting the
            //     pair then pops a return address on that path;
            //   * a `CondJmp` leaves the pair unbalanced on one side;
            //   * a nested `Push`/`Pop` (including `pushfq`/`popfq`, which
            //     classify with `REG_NONE`) re-points the slot being restored;
            //   * opaque inline asm and non-inert directives may move `%rsp`,
            //     write through it or carry a jump table.
            // The hand-rolled `Call | Jmp | JmpIndirect | Ret` list this
            // replaces silently spanned the first three.  A trap or a transfer
            // with no kind of its own (`sysret`, `hlt`, `int3`) ends the window
            // for the same reason and one more: the `%rsp` it executes with is
            // observable outside the function (see [`leaves_linear_flow`]).
            if infos[j].is_barrier() || leaves_linear_flow(infos[j].trimmed(store.get(j))) {
                break;
            }
        }
    }
    changed
}

/// Does the instruction at this line read or write the stack pointer?
///
/// Removing a `push`/`pop` pair moves every byte below the old `%rsp` down by
/// eight and changes `%rsp`'s value by exactly that amount across the span, so
/// a line may only be spanned when it is blind to both facts. Two exact halves,
/// in the same shape as the register oracle:
///
///  * **textual** — the operand text names the stack pointer in any of its
///    spellings. All three are listed deliberately: `%rsp` does not contain
///    `%sp` (the `r` intervenes), so a single `%sp` test would miss every
///    64-bit mention. This is what covers `subq $N, %rsp`/`addq $N, %rsp` (frame
///    adjustments), `movq %rax, %rsp` and `leaq -8(%rbp), %rsp` (epilogue
///    restores) and every `%rsp`-relative operand (a spill, an inline-asm store
///    through `(%rsp)`);
///  * **architectural** — the ISA table says the instruction touches the `%rsp`
///    family without naming it: `leave`/`enter` restore and build the frame,
///    `pushf`/`popf` own the very slot a save creates, `iret`/`sysret` leave the
///    function through it. These have no operand text to match, and the previous
///    text-only version of this predicate therefore stepped straight over a
///    `leave` inside a span. The i686 back end's `scan_push_pop_span` always
///    asked the oracle for `%esp`; this is the missing x86-64 half of the same
///    rule, so both back ends now answer the question the same way.
#[inline]
fn line_depends_on_stack_pointer(text: &str) -> bool {
    text.contains("%rsp")
        || text.contains("%esp")
        || text.contains("%sp")
        || implicit_reg_refs(text.as_bytes()) & (1u16 << REG_RSP) != 0
}

/// An instruction that transfers control or traps without owning a
/// `LineKind` of its own: `sysret`, `sysexit`, `syscall`, `hlt`, `ud2`,
/// `int3`, `rsm`, and the rest of [`is_control_transfer_mnemonic`].
///
/// A save/restore pair may not span one, for two independent reasons. First,
/// none of them falls through, so "the lines between the `push` and its `pop`"
/// is not a straight-line sequence at all. Second — the reason that matters for
/// the stack proof — `syscall`/`sysret`/`hlt` all leave the *current* `%rsp` to
/// be observed by something outside the function, so removing a pair around
/// them changes the value that observer sees even though no line in the span
/// names `%rsp`. `is_barrier` cannot answer this: it is a property of the
/// classified kind, and these lines classify as `Other`. `iret`/`popf` need no
/// help here — the implicit-oracle half of
/// [`line_depends_on_stack_pointer`] already sees their `%rsp`.
#[inline]
fn leaves_linear_flow(text: &str) -> bool {
    matches!(
        mnemonic_after_prefixes(text),
        (_, Some(m)) if is_control_transfer_mnemonic(m)
    )
}

/// Check whether a line makes push/pop elimination incorrect because it
/// depends on the stack depth (see [`line_depends_on_stack_pointer`]).
fn instruction_modifies_stack(line: &str, info: &LineInfo) -> bool {
    if info.is_nop() {
        return false;
    }
    let s = info.trimmed(line);
    !s.is_empty() && line_depends_on_stack_pointer(s)
}

/// Fast check whether a line's instruction modifies a register identified by RegId.
///
/// Two exact halves, because a register can be written without being named:
///
///  * **Architectural implicit writes** — [`implicit_write_refs`] answers from
///    the single ISA table in `types.rs`: `divq`/`mulq`/one-operand `imulq`
///    write `%rax:%rdx`, `cqto`/`cdq` write `%rdx`, `cpuid` writes
///    `%rax:%rbx:%rcx:%rdx`, `rep stosq` writes `%rdi:%rcx`, `syscall` writes
///    `%rax:%rcx:%r11`, `pcmpistri` writes `%rcx`, and so on.
///  * **The classified destination** — `LineKind` already pre-parses it, plus
///    the two-destination spellings the AT&T "last operand is the destination"
///    rule cannot represent: `xchg` swaps both operands, `xadd` returns the old
///    value in its register operand, and `cmpxchg` reloads the accumulator the
///    table already covers.
///
/// This used to be `dest_reg == 0 && reg_id == 2` ("a `%rax` writer might also
/// write `%rdx`", carrying a `TODO: could be more precise`). That guess is
/// wrong in both directions: it refused `pushq %rdx` around an `%rax`-only span
/// (`addq %rax, %rax`, `movq $imm, %rax`), i.e. it lost real eliminations, and
/// it still missed every implicit writer whose *classified* destination is not
/// `%rax` — `cpuid`, `rdtsc`, `rep stosq`, `xchg`, all of which classify as
/// `Other { dest_reg: REG_NONE }`. Consulting the table fixes both.
#[inline]
fn instruction_modifies_reg_id(line: &str, info: &LineInfo, reg_id: RegId) -> bool {
    if info.is_nop() {
        return false;
    }
    let text = info.trimmed(line);
    // Exact architectural implicit writes and the two-destination forms.
    if line_writes_gp_family(info, text, reg_id) {
        return true;
    }
    match info.kind {
        // Barrier kinds: their clobber set is a property of the instruction
        // class, not of one family, so the caller's window decides.
        LineKind::Call => matches!(reg_id, 0 | 1 | 2 | 6 | 7 | 8 | 9 | 10 | 11),
        LineKind::InlineAsm => true,
        LineKind::JmpIndirect => true,
        _ => false,
    }
}

pub(super) fn eliminate_binop_push_pop_pattern(
    store: &mut LineStore,
    infos: &mut [LineInfo],
) -> bool {
    let mut changed = false;
    let len = store.len();

    let mut i = 0;
    while i + 3 < len {
        let push_reg_id = match infos[i].kind {
            LineKind::Push { reg } if reg != REG_NONE => reg,
            _ => {
                i += 1;
                continue;
            }
        };

        let push_line = infos[i].trimmed(store.get(i));
        let push_reg = match push_line.strip_prefix("pushq ") {
            Some(r) => r.trim(),
            None => {
                i += 1;
                continue;
            }
        };

        // Find next 3 non-NOP lines
        let mut real_indices = [0usize; 3];
        let count = collect_non_nop_indices(infos, i, len, &mut real_indices);

        if count == 3 {
            let load_idx = real_indices[0];
            let move_idx = real_indices[1];
            let pop_idx = real_indices[2];

            if let LineKind::Pop { reg: pop_reg_id } = infos[pop_idx].kind {
                if pop_reg_id == push_reg_id {
                    // Check no stack-modifying instructions between push and pop
                    let mut stack_safe = true;
                    for k in (i + 1)..=pop_idx {
                        if !infos[k].is_nop() && instruction_modifies_stack(store.get(k), &infos[k])
                        {
                            stack_safe = false;
                            break;
                        }
                    }
                    if !stack_safe {
                        i += 1;
                        continue;
                    }

                    let load_line = infos[load_idx].trimmed(store.get(load_idx));
                    let move_line = infos[move_idx].trimmed(store.get(move_idx));

                    if let Some(move_target) = parse_reg_to_reg_move(move_line, push_reg) {
                        if instruction_writes_to(load_line, push_reg)
                            && can_redirect_instruction(load_line)
                        {
                            if let Some(new_load) =
                                replace_dest_register(load_line, push_reg, move_target)
                            {
                                mark_nop(&mut infos[i]);
                                let new_text = format!("    {}", new_load);
                                replace_line(store, &mut infos[load_idx], load_idx, new_text);
                                mark_nop(&mut infos[move_idx]);
                                mark_nop(&mut infos[pop_idx]);
                                changed = true;
                                i = pop_idx + 1;
                                continue;
                            }
                        }
                    }
                }
            }
        }

        i += 1;
    }
    changed
}

/// Delete a `push R` ... `pop R` save around a straight-line span when `R`
/// is dead after the `pop`.
///
/// Emitter sequences that need a fixed scratch register preserve it
/// unconditionally (the baseline SWAR popcount saves `%rcx`, and `%rdx` on
/// its 64-bit path), because at emission time nothing says whether the
/// register holds a live value. The pair is worthless when nothing reads
/// `R` afterwards: two stack accesses, and a barrier that hides the whole
/// span from every offset-tracking pass. [`eliminate_push_pop_pairs`] only
/// drops pairs whose span leaves `R` untouched; this rule drops pairs whose
/// span writes `R`, using the file liveness solution.
///
/// Soundness. Without the pair, the span's own writes become `R`'s value
/// after the `pop`, which liveness proves nothing observes (callee-saved
/// registers are live at `ret`, so a prologue/epilogue save is never
/// "dead"). The span must not observe the 8-byte shift of `%rsp`: it may
/// contain no label, branch, call, `ret`, inline asm, `pushf`/`popf`, CFI
/// directive or `%rsp`/`%esp` operand, and every push inside it is matched
/// by a pop inside it (those inner pairs only ever address their own slot
/// through `%rsp` implicitly).
pub(super) fn eliminate_dead_scratch_saves(store: &LineStore, infos: &mut [LineInfo]) -> bool {
    /// Longest span considered: the 64-bit SWAR popcount is 22 lines.
    const WINDOW: usize = 64;
    let len = store.len();
    let mut liveness: Option<super::liveness::FileLiveness> = None;
    let mut changed = false;
    // Registers whose cached dataflow answer is no longer trustworthy. A `pop R`
    // is a DEFINITION of `R`, so a solution computed before a removal no longer
    // describes the text: deleting a definition lets the value defined before
    // the pair flow further, which *grows* the live set (a removal can never
    // shrink it), and a cached "R is dead here" answer that grew would be
    // unsound rather than merely stale. Liveness is a backwards problem, so the
    // growth propagates backwards and -- through a loop's backedge -- forwards
    // again. Each register whose pair was actually removed is therefore marked
    // dirty, and its next query re-solves the enclosing function: one re-solve
    // per register between two removals of that register. This is the same
    // discipline the i686 pass (`GprLiveness`) uses, so the two back ends reason
    // about a shrinking program identically.
    let mut dirty = 0u16;
    for i in 0..len {
        let r = match infos[i].kind {
            LineKind::Push { reg } if reg <= REG_GP_MAX && reg != REG_RSP && reg != REG_RBP => reg,
            _ => continue,
        };
        if infos[i].is_nop() || infos[i].pinned {
            continue;
        }
        let Some(j) = matching_scratch_pop(store, infos, i, r, WINDOW) else {
            continue;
        };
        let bit = 1u16 << r;
        if dirty & bit != 0 {
            dirty &= !bit;
            liveness = Some(super::liveness::FileLiveness::new(store, infos));
        }
        let lv = liveness.get_or_insert_with(|| super::liveness::FileLiveness::new(store, infos));
        if !lv.same_function(i, j) {
            continue;
        }
        // In a loop the next iteration's `push` reads `R`, so `R` looks live
        // after the `pop` through the save alone. Ask again without that one
        // read (only the push line is neutralised: a read of `R` inside the
        // span before any write there still keeps the save, because the
        // next iteration would see the span's value instead of the
        // restored one).
        let mask = bit;
        let dead = match lv.live_after(j, r) {
            Some(false) => true,
            Some(true) => lv
                .live_after_dropping_region(store, infos, j, i, i, mask)
                .is_some_and(|live| live & mask == 0),
            None => false,
        };
        if dead {
            mark_nop(&mut infos[i]);
            mark_nop(&mut infos[j]);
            dirty |= bit;
            changed = true;
        }
    }
    changed
}

/// The `pop R` that restores the `push R` at `i`, when the span between them
/// is straight-line and blind to the stack depth (see
/// [`eliminate_dead_scratch_saves`]).
fn matching_scratch_pop(
    store: &LineStore,
    infos: &[LineInfo],
    i: usize,
    r: RegId,
    window: usize,
) -> Option<usize> {
    let mut depth = 0usize;
    for k in i + 1..store.len().min(i + 1 + window) {
        if infos[k].is_nop() {
            continue;
        }
        let t = infos[k].trimmed(store.get(k));
        match infos[k].kind {
            LineKind::Push { reg } if reg != REG_NONE => depth += 1,
            LineKind::Pop { reg } if reg != REG_NONE => {
                if depth == 0 {
                    let restores = reg == r && !infos[k].pinned;
                    return (restores && !cfi_follows(store, infos, k)).then_some(k);
                }
                depth -= 1;
            }
            // pushf/popf, control flow and opaque text end the search.
            LineKind::Push { .. }
            | LineKind::Pop { .. }
            | LineKind::Label
            | LineKind::Call
            | LineKind::Jmp
            | LineKind::JmpIndirect
            | LineKind::CondJmp
            | LineKind::Ret
            | LineKind::InlineAsm => return None,
            // Inert directives (`.loc`, alignment hints) emit nothing the
            // span could observe. CFI is inert for windows but not here: an
            // adjustment inside the span describes the push being removed.
            LineKind::Directive if !infos[k].inert_dir || t.starts_with(".cfi_") => return None,
            _ => {
                // Any stack-pointer dependence, textual or architectural --
                // `leave`/`enter`/`pushf` name no register, so a text-only test
                // would step over them exactly as the pair scan used to.
                if line_depends_on_stack_pointer(t) {
                    return None;
                }
                // And any trap or unnamed control transfer: it does not fall
                // through, and the `%rsp` it runs with is visible outside the
                // function ([`leaves_linear_flow`]).
                if leaves_linear_flow(t) {
                    return None;
                }
            }
        }
    }
    None
}

/// Whether the first real line after `j` is a CFI directive other than
/// `.cfi_endproc` (the `-8` adjustment that pairs with an annotated push).
///
/// Debug attribution and alignment hints between the `pop` and its adjustment
/// are stepped over: they emit nothing, so the adjustment still describes the
/// byte the `pop` moved. Stopping at the `.loc` instead would hide the
/// adjustment and let the pair be deleted out from under the unwinder.
fn cfi_follows(store: &LineStore, infos: &[LineInfo], j: usize) -> bool {
    (j + 1..store.len())
        .find(|&k| {
            !infos[k].is_nop()
                && !infos[k].is_unwind_inert_directive(infos[k].trimmed(store.get(k)))
        })
        .map(|k| infos[k].trimmed(store.get(k)))
        .is_some_and(|t| t.starts_with(".cfi_") && !t.starts_with(".cfi_endproc"))
}

#[cfg(test)]
mod dead_scratch_save_tests {
    use super::super::super::types::*;
    use super::super::pin_inline_asm_regions;
    use super::eliminate_dead_scratch_saves;

    fn run(body: &str) -> String {
        let store = LineStore::new(format!("f:\n    .cfi_startproc\n{body}    .cfi_endproc\n"));
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        pin_inline_asm_regions(&store, &mut infos);
        eliminate_dead_scratch_saves(&store, &mut infos);
        store.build_result(|i| infos[i].is_nop())
    }

    /// The baseline 32-bit SWAR popcount, as the emitter writes it.
    const SWAR32: &str = "    pushq %rcx\n    movl %eax, %ecx\n    shrl $1, %eax\n    \
        andl $0x55555555, %eax\n    subl %eax, %ecx\n    movl %ecx, %eax\n    shrl $2, %eax\n    \
        andl $0x33333333, %eax\n    andl $0x33333333, %ecx\n    addl %eax, %ecx\n    \
        movl %ecx, %eax\n    shrl $4, %eax\n    addl %ecx, %eax\n    andl $0x0f0f0f0f, %eax\n    \
        imull $0x01010101, %eax, %eax\n    shrl $24, %eax\n    popq %rcx\n";

    #[test]
    fn dead_scratch_save_is_dropped() {
        let out = run(&format!("{SWAR32}    ret\n"));
        assert!(!out.contains("pushq") && !out.contains("popq"), "{out}");
        assert!(out.contains("movl %eax, %ecx"), "{out}");
    }

    /// The 64-bit SWAR shape: `%rdx` saved inside `%rcx`.
    const NESTED: &str = "    pushq %rcx\n    pushq %rdx\n    movq %rax, %rcx\n    \
        movabsq $0x5555555555555555, %rdx\n    andq %rdx, %rcx\n    movq %rcx, %rax\n    \
        popq %rdx\n    popq %rcx\n    ret\n";

    #[test]
    fn nested_saves_are_dropped_inside_out() {
        // A scalar return: the prologue marker says `ret` does not read %rdx.
        let out = run(&format!("    # LCCC_RET_RDX 0\n{NESTED}"));
        assert!(!out.contains("pushq") && !out.contains("popq"), "{out}");
        // A 16-byte return in %rax:%rdx: the %rdx save restores a live
        // value (the caller's would-be high half), so only %rcx goes.
        let out = run(&format!("    # LCCC_RET_RDX 1\n{NESTED}"));
        assert!(
            out.contains("pushq %rdx") && out.contains("popq %rdx"),
            "{out}"
        );
        assert!(!out.contains("pushq %rcx"), "{out}");
    }

    #[test]
    fn loop_carried_save_is_dropped_only_when_the_span_writes_first() {
        // The next iteration's push is the only reader of %rcx.
        let looped = format!(
            "    xorl %r8d, %r8d\n.LBB4:\n    movl (%rdi), %eax\n{SWAR32}    addl %eax, %r8d\n    \
             subl $1, %esi\n    jnz .LBB4\n    movl %r8d, %eax\n    ret\n"
        );
        let out = run(&looped);
        assert!(!out.contains("pushq") && !out.contains("popq"), "{out}");
        // Here the span reads %rcx before writing it: every iteration needs
        // the restored value, so the save stays.
        let body = "    movl $3, %ecx\n.LBB4:\n    pushq %rcx\n    addl %ecx, %eax\n    \
            movl $5, %ecx\n    imull %ecx, %eax\n    popq %rcx\n    subl $1, %esi\n    \
            jnz .LBB4\n    ret\n";
        let out = run(body);
        assert!(
            out.contains("pushq %rcx") && out.contains("popq %rcx"),
            "{out}"
        );
    }

    #[test]
    fn live_or_unsafe_saves_are_kept() {
        // %rcx is read after the pop: the save restores a live value.
        let out = run(&format!("{SWAR32}    addl %ecx, %eax\n    ret\n"));
        assert_eq!(out.matches("pushq %rcx").count(), 1, "{out}");
        // Callee-saved: live at `ret` (the caller's value must come back).
        let out = run(
            "    pushq %rbx\n    movl %eax, %ebx\n    addl %ebx, %eax\n    popq %rbx\n    ret\n",
        );
        assert!(
            out.contains("pushq %rbx") && out.contains("popq %rbx"),
            "{out}"
        );
        // A %rsp-relative operand would move by 8 without the push.
        let out = run(
            "    pushq %rcx\n    movl 8(%rsp), %ecx\n    addl %ecx, %eax\n    popq %rcx\n    ret\n",
        );
        assert!(out.contains("pushq %rcx"), "{out}");
        // A label inside: another path may enter with a different depth.
        let out = run("    pushq %rcx\n.L1:\n    movl %eax, %ecx\n    popq %rcx\n    ret\n");
        assert!(out.contains("pushq %rcx"), "{out}");
        // A call inside: the callee sees the shifted stack.
        let out = run("    pushq %rcx\n    movl %eax, %ecx\n    call g\n    popq %rcx\n    ret\n");
        assert!(out.contains("pushq %rcx"), "{out}");
        // Mismatched restore: `push %rcx; pop %rdx` is a move, not a save.
        let out = run("    pushq %rcx\n    movl %eax, %ecx\n    popq %rdx\n    ret\n");
        assert!(
            out.contains("pushq %rcx") && out.contains("popq %rdx"),
            "{out}"
        );
        // A CFI adjustment inside describes the push for the unwinder.
        let out = run(
            "    pushq %rcx\n    .cfi_adjust_cfa_offset 8\n    movl %eax, %ecx\n    popq %rcx\n    ret\n",
        );
        assert!(out.contains("pushq %rcx"), "{out}");
        let out = run(
            "    pushq %rcx\n    movl %eax, %ecx\n    popq %rcx\n    .cfi_adjust_cfa_offset -8\n    ret\n",
        );
        assert!(out.contains("pushq %rcx"), "{out}");
        // A debug line marker inside is inert.
        let out =
            run("    pushq %rcx\n    .loc 1 7 3\n    movl %eax, %ecx\n    popq %rcx\n    ret\n");
        assert!(!out.contains("pushq %rcx"), "{out}");
        // A frame teardown inside is not: `leave` restores %rsp with no
        // operand text, so the architectural half of the stack-pointer
        // predicate has to see it.
        let out = run("    pushq %rcx\n    leave\n    movl %eax, %ecx\n    popq %rcx\n    ret\n");
        assert!(out.contains("pushq %rcx"), "{out}");
        // A debug line between the pop and its CFI adjustment must not hide
        // the adjustment from the scan.
        let out = run(
            "    pushq %rcx\n    movl %eax, %ecx\n    popq %rcx\n    .loc 1 9 1\n    \
             .cfi_adjust_cfa_offset -8\n    ret\n",
        );
        assert!(out.contains("pushq %rcx"), "{out}");
    }
}

#[cfg(test)]
mod pair_scan_tests {
    use super::super::super::types::*;
    use super::eliminate_push_pop_pairs;

    fn run(body: &str) -> String {
        let store = LineStore::new(format!("f:\n    .cfi_startproc\n{body}    .cfi_endproc\n"));
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        eliminate_push_pop_pairs(&store, &mut infos);
        store.build_result(|i| infos[i].is_nop())
    }

    #[test]
    fn an_untouched_span_drops_the_pair() {
        let out = run("    pushq %rbx\n    addq %rcx, %rdx\n    popq %rbx\n");
        assert!(!out.contains("pushq"), "{out}");
        assert!(!out.contains("popq"), "{out}");
    }

    /// A label inside the span is a second entry point: the `pop` is reachable
    /// without the `push` having run, so deleting the pair would pop a return
    /// address on that path. The scan must stop at it.
    #[test]
    fn a_label_inside_the_span_keeps_the_pair() {
        let out = run("    pushq %rbx\n.L7:\n    popq %rbx\n    ret\n");
        assert!(out.contains("pushq %rbx"), "{out}");
        assert!(out.contains("popq %rbx"), "{out}");
    }

    /// A conditional branch leaves the pair unbalanced on the taken side.
    #[test]
    fn a_conditional_branch_inside_the_span_keeps_the_pair() {
        let out = run("    pushq %rbx\n    jne .L8\n    popq %rbx\n.L8:\n    ret\n");
        assert!(out.contains("pushq %rbx"), "{out}");
    }

    /// The stack-pointer mention test has to see 64-bit spellings: `%rsp` does
    /// not contain the substring `%sp`.
    #[test]
    fn a_stack_pointer_restore_inside_the_span_keeps_the_pair() {
        for body in [
            "    pushq %rbx\n    movq %rax, %rsp\n    popq %rbx\n",
            "    pushq %rbx\n    movq 8(%rsp), %rax\n    popq %rbx\n",
            "    pushq %rbx\n    subq $16, %rsp\n    popq %rbx\n",
            "    pushq %rbx\n    pushfq\n    popfq\n    popq %rbx\n",
        ] {
            let out = run(body);
            assert!(out.contains("pushq %rbx"), "{body} -> {out}");
        }
    }

    /// The stack-pointer test is not only textual: `leave` restores `%rsp` from
    /// `%rbp` and names neither, so a text-only test stepped over it and moved
    /// the restored pointer by a whole slot. Same for `enter`.
    #[test]
    fn a_frame_teardown_inside_the_span_keeps_the_pair() {
        for body in [
            "    pushq %rbx\n    leave\n    popq %rbx\n",
            "    pushq %rbx\n    enter $16, $0\n    popq %rbx\n",
            "    pushq %rbx\n    iretq\n    popq %rbx\n",
            "    pushq %rbx\n    sysretq\n    popq %rbx\n",
        ] {
            let out = run(body);
            assert!(out.contains("pushq %rbx"), "{body} -> {out}");
        }
    }

    /// A pinned line belongs to the program's own contract (a C `volatile`
    /// access, an inline-asm region, a parameter pre-store). Removing the pair
    /// would remove it, so the proof is irrelevant.
    #[test]
    fn a_pinned_pair_is_never_deleted() {
        // Line indices: 0 `f:`, 1 `.cfi_startproc`, 2 `pushq`, 3 `addq`,
        // 4 `popq`, 5 `.cfi_endproc`.
        for pinned in [2usize, 4] {
            let store = LineStore::new(
                "f:\n    .cfi_startproc\n    pushq %rbx\n    addq %rcx, %rdx\n    \
                 popq %rbx\n    .cfi_endproc\n"
                    .to_string(),
            );
            let mut infos: Vec<LineInfo> = (0..store.len())
                .map(|i| classify_line(store.get(i)))
                .collect();
            infos[pinned].pinned = true;
            eliminate_push_pop_pairs(&store, &mut infos);
            let out = store.build_result(|i| infos[i].is_nop());
            assert!(out.contains("pushq %rbx"), "{pinned} -> {out}");
            assert!(out.contains("popq %rbx"), "{pinned} -> {out}");
        }
    }

    /// The unwinder is told about the slot the pair owns by the CFI directive
    /// that follows the `pop`; deleting the pair would leave it pointing at a
    /// byte nobody moved. A `.loc` between the two emits nothing and must not
    /// hide the adjustment.
    #[test]
    fn a_cfi_adjustment_after_the_pop_keeps_the_pair() {
        for body in [
            "    pushq %rbx\n    addq %rcx, %rdx\n    popq %rbx\n    .cfi_adjust_cfa_offset -8\n",
            "    pushq %rbx\n    addq %rcx, %rdx\n    popq %rbx\n    .loc 1 9 1\n    \
             .cfi_adjust_cfa_offset -8\n",
        ] {
            let out = run(body);
            assert!(out.contains("pushq %rbx"), "{body} -> {out}");
        }
    }

    /// The window is wider than the three lines the pass started with, so a
    /// longer straight-line span still reaches its `pop`.
    #[test]
    fn the_window_reaches_a_longer_untouched_span() {
        let out = run(
            "    pushq %rbx\n    addq %rcx, %rdx\n    subq %rsi, %rdi\n    \
             xorq %r8, %r9\n    andq %r10, %r11\n    popq %rbx\n",
        );
        assert!(!out.contains("pushq"), "{out}");
        assert!(!out.contains("popq"), "{out}");
    }
}

#[cfg(test)]
mod register_clobber_tests {
    use super::super::super::types::*;
    use super::instruction_modifies_reg_id;

    fn modifies(text: &str, fam: RegId) -> bool {
        let info = classify_line(text);
        instruction_modifies_reg_id(text, &info, fam)
    }

    /// `%rdx` saved across a span that only touches `%rax` used to be refused
    /// by the `dest_reg == 0 && reg_id == 2` guess.
    #[test]
    fn an_rax_only_span_does_not_clobber_rdx() {
        assert!(!modifies("addq %rax, %rax", 2));
        assert!(!modifies("movabsq $1, %rax", 2));
        assert!(modifies("addq %rax, %rax", 0));
    }

    /// The architectural implicit writers must be seen even when the
    /// classified destination names another family (or none at all).
    #[test]
    fn implicit_writers_are_seen() {
        // `cqto` writes %rdx and only READS %rax. Answering "no write to
        // %rax" here is the whole point of the read guard in
        // `line_writes_gp_family`: `parse_dest_reg_fast` calls %rax its primary
        // destination because it returns one family, and taking that at face
        // value would keep `pushq %rax; cqto; popq %rax` alive forever.
        assert!(modifies("cqto", 2));
        assert!(!modifies("cqto", 0));
        // cltq writes %rax only.
        assert!(modifies("cltq", 0));
        assert!(!modifies("cltq", 2));
        // cpuid rewrites four families at once.
        for fam in [0u8, 1, 2, 3] {
            assert!(modifies("cpuid", fam), "cpuid must clobber family {fam}");
        }
        // rep stosq writes %rdi:%rcx.
        assert!(modifies("rep stosq", 7));
        assert!(modifies("rep stosq", 1));
        // One-operand imulq writes %rax:%rdx; the two-operand form writes only
        // its named destination and reads the other operand.
        assert!(modifies("imulq %r11", 2));
        assert!(modifies("imulq %r11", 0));
        assert!(!modifies("imulq %r11, %rax", 2));
        assert!(modifies("imulq %r11, %rax", 0));
        // A `mulx` READS %rdx and writes only its explicit destinations.
        assert!(!modifies("mulxq %r11, %rax, %rcx", 2));
        // The SSE4.2 string compare returns its index in %rcx.
        assert!(modifies("pcmpistri $0, %xmm1, %xmm2", 1));
    }

    /// Both operands of the two-destination forms are writes.
    #[test]
    fn two_destination_forms_write_both_operands() {
        assert!(modifies("xchgq %rbx, %rcx", 3));
        assert!(modifies("xchgq %rbx, %rcx", 1));
        assert!(!modifies("xchgq %rbx, %rcx", 0));
        assert!(modifies("xaddq %r8, %r9", 8));
        assert!(modifies("lock xaddl %eax, %ebx", 0));
        // cmpxchg reloads the accumulator and stores into its explicit operand.
        assert!(modifies("cmpxchgq %rbx, (%rdi)", 0));
        assert!(modifies("lock cmpxchgq %rbx, (%rdi)", 3));
    }

    /// An unknown destination stays conservative: the caller's window proof,
    /// not this predicate, decides what `REG_NONE` means.
    #[test]
    fn barrier_kinds_answer_for_their_class() {
        assert!(modifies("callq g", 0));
        assert!(modifies("callq g", 8));
        assert!(!modifies("callq g", 3), "callee-saved survive a call");
    }
}
