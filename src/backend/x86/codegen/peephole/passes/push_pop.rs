//! Push/pop pair elimination passes.
//!
//! Eliminates redundant push/pop pairs and push/binop/move/pop patterns
//! that arise from the stack-based codegen model.

use super::super::types::*;

pub(super) fn eliminate_push_pop_pairs(store: &LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let len = store.len();

    for i in 0..len.saturating_sub(2) {
        let push_reg_id = match infos[i].kind {
            LineKind::Push { reg } if reg != REG_NONE => reg,
            _ => continue,
        };

        for j in (i + 1)..std::cmp::min(i + 4, len) {
            if infos[j].is_nop() {
                continue;
            }
            if let LineKind::Pop { reg: pop_reg_id } = infos[j].kind {
                if pop_reg_id == push_reg_id {
                    let mut safe = true;
                    for k in (i + 1)..j {
                        if infos[k].is_nop() {
                            continue;
                        }
                        if instruction_modifies_reg_id(&infos[k], push_reg_id) {
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
            if infos[j].is_push() {
                break;
            }
            if matches!(
                infos[j].kind,
                LineKind::Call | LineKind::Jmp | LineKind::JmpIndirect | LineKind::Ret
            ) {
                break;
            }
        }
    }
    changed
}

/// Check if an instruction implicitly modifies the stack pointer or accesses
/// the stack in ways that would make push/pop elimination incorrect.
/// This catches instructions like pushfq, popfq, pushfl, popfl, etc. that
/// read/write stack memory without being classified as Push/Pop.
fn instruction_modifies_stack(line: &str, info: &LineInfo) -> bool {
    if info.is_nop() {
        return false;
    }
    let s = info.trimmed(line);
    let b = s.as_bytes();
    if b.is_empty() {
        return false;
    }
    // pushfq, pushfl, pushf, popfq, popfl, popf -- these modify %rsp and
    // read/write the stack, making it unsafe to eliminate surrounding push/pop
    if b[0] == b'p' && (s.starts_with("pushf") || s.starts_with("popf")) {
        return true;
    }
    // subq/addq to %rsp
    if (s.starts_with("subq ") || s.starts_with("addq ")) && s.ends_with("%rsp") {
        return true;
    }
    // ANY memory operand addressed via %rsp (e.g. "(%rsp)", "8(%rsp)",
    // "(%rsp,%rax,4)") depends on the pushed value sitting at the stack top:
    // eliminating the surrounding push/pop pair would leave the access
    // pointing at a different (wrong) location. This matters for inline-asm
    // output stores, which materialize the destination pointer, push it, and
    // store through "(%rsp)".
    if s.contains("%rsp)") || s.contains("%rsp,") {
        return true;
    }
    false
}

/// Fast check whether a line's instruction modifies a register identified by RegId.
/// Uses pre-parsed `LineInfo` fields to avoid string parsing in the hot path.
#[inline]
fn instruction_modifies_reg_id(info: &LineInfo, reg_id: RegId) -> bool {
    match info.kind {
        LineKind::StoreRbp { .. }
        | LineKind::StoreXmmRbp { .. }
        | LineKind::LoadXmmRbp { .. }
        | LineKind::Cmp
        | LineKind::Nop
        | LineKind::Empty
        | LineKind::Label
        | LineKind::Directive
        | LineKind::Jmp
        | LineKind::JmpIndirect
        | LineKind::CondJmp
        | LineKind::SelfMove => false,
        LineKind::LoadRbp { reg, .. } => reg == reg_id,
        LineKind::Pop { reg } => reg == reg_id,
        LineKind::Push { .. } => false, // push reads, doesn't modify the source reg
        LineKind::SetCC { reg } => reg_id == reg, // setCC writes to the byte register's family
        LineKind::Call => matches!(reg_id, 0 | 1 | 2 | 6 | 7 | 8 | 9 | 10 | 11),
        LineKind::Ret => false,
        // Inline asm is opaque: assume it modifies every register.
        LineKind::InlineAsm => true,
        LineKind::Other { dest_reg } => {
            if dest_reg == reg_id {
                return true;
            }
            // div/idiv also clobber rdx (family 2), and rax (family 0)
            // mul also clobbers rdx
            if dest_reg == 0 && reg_id == 2 {
                return true; // TODO: could be more precise
            }
            false
        }
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
        let lv = liveness.get_or_insert_with(|| super::liveness::FileLiveness::new(store, infos));
        // Removals only shrink live sets, so the solution computed before
        // them stays a conservative answer for every later query.
        if !lv.same_function(i, j) {
            continue;
        }
        // In a loop the next iteration's `push` reads `R`, so `R` looks live
        // after the `pop` through the save alone. Ask again without that one
        // read (only the push line is neutralised: a read of `R` inside the
        // span before any write there still keeps the save, because the
        // next iteration would see the span's value instead of the
        // restored one).
        let mask = 1u16 << r;
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
                if t.contains("%rsp") || t.contains("%esp") {
                    return None;
                }
            }
        }
    }
    None
}

/// Whether the first real line after `j` is a CFI directive other than
/// `.cfi_endproc` (the `-8` adjustment that pairs with an annotated push).
fn cfi_follows(store: &LineStore, infos: &[LineInfo], j: usize) -> bool {
    (j + 1..store.len())
        .find(|&k| !infos[k].is_nop())
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
    }
}
