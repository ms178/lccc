//! Unused callee-saved register elimination pass.
//!
//! After peephole optimization, some callee-saved registers may no longer be
//! referenced in the function body (all uses were optimized away). This pass
//! detects such registers and removes their prologue save / epilogue restore
//! instructions. The stack frame is not shrunk (see rationale inside function).
//!
//! Two prologue shapes are handled:
//!
//! * `pushq %rbp; movq %rsp,%rbp; pushq <callee-saved>…` —
//!   [`eliminate_unused_callee_saves`].
//! * frame-pointer-omitted `pushq <callee-saved>…` directly after
//!   `.cfi_startproc` — [`eliminate_unused_callee_saves_fpo`]. Dropping a push
//!   there moves `%rsp` by 8, so the FPO variant only fires in call-free
//!   functions without aligned vector stack traffic or `%rsp` realignment, and
//!   it rewrites the single `.cfi_def_cfa_offset` the prologue emits.

use super::super::types::*;
use super::helpers::*;

pub(super) fn eliminate_unused_callee_saves(store: &mut LineStore, infos: &mut [LineInfo]) {
    let len = store.len();
    if len == 0 {
        return;
    }

    let mut i = 0;
    while i < len {
        // Look for the prologue: pushq %rbp
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        if !matches!(infos[i].kind, LineKind::Push { reg: 5 }) {
            i += 1;
            continue;
        }

        // Next non-nop should be "movq %rsp, %rbp"
        let mut j = next_non_nop(infos, i + 1, len);
        if j >= len {
            i = j;
            continue;
        }
        let mov_rbp_line = infos[j].trimmed(store.get(j));
        if mov_rbp_line != "movq %rsp, %rbp" {
            i = j + 1;
            continue;
        }
        j += 1;

        // Collect callee-saved register saves: either pushq or movq to stack.
        // The prologue may have: pushq %rbx; pushq %r12; ... subq $N, %rsp
        // Or the old style: subq $N, %rsp; movq %rbx, -N(%rbp); ...
        struct CalleeSave {
            reg: RegId,
            save_line_idx: usize,
            is_push: bool,
        }
        let mut saves: Vec<CalleeSave> = Vec::new();

        j = next_non_nop(infos, j, len);

        // First, collect pushq callee-saved registers (new prologue style).
        // Skip .cfi directives interspersed between pushes.
        while j < len {
            if infos[j].is_nop() || infos[j].kind == LineKind::Directive {
                j += 1;
                continue;
            }
            if let LineKind::Push { reg } = infos[j].kind {
                if is_callee_saved_reg(reg) {
                    saves.push(CalleeSave {
                        reg,
                        save_line_idx: j,
                        is_push: true,
                    });
                    j += 1;
                    continue;
                }
            }
            break;
        }

        // Skip subq $N, %rsp if present (also skip directives)
        while j < len && (infos[j].is_nop() || infos[j].kind == LineKind::Directive) {
            j += 1;
        }
        if j < len {
            let subq_line = infos[j].trimmed(store.get(j));
            if let Some(rest) = subq_line.strip_prefix("subq $") {
                if rest
                    .strip_suffix(", %rsp")
                    .and_then(|v| v.parse::<i64>().ok())
                    .is_some()
                {
                    j += 1;
                }
            }
        }

        // Then, collect movq callee-saved saves (old prologue style)
        j = next_non_nop(infos, j, len);
        while j < len {
            if infos[j].is_nop() {
                j += 1;
                continue;
            }
            if let LineKind::StoreRbp {
                reg,
                offset,
                size: MoveSize::Q,
            } = infos[j].kind
            {
                if is_callee_saved_reg(reg) && offset < 0 {
                    saves.push(CalleeSave {
                        reg,
                        save_line_idx: j,
                        is_push: false,
                    });
                    j += 1;
                    continue;
                }
            }
            break;
        }

        if saves.is_empty() {
            i = j;
            continue;
        }

        // Find the end of this function by looking for the .size directive.
        let body_start = j;
        let mut func_end = len;
        for k in body_start..len {
            if infos[k].is_nop() {
                continue;
            }
            let line = infos[k].trimmed(store.get(k));
            if line.starts_with(".size ") {
                func_end = k + 1;
                break;
            }
        }

        // For each callee-saved register, check if it's referenced in the body
        // (excluding the save/restore instructions themselves).
        for save in &saves {
            let reg = save.reg;

            let mut restore_indices: Vec<usize> = Vec::new();
            let mut body_has_reference = false;

            for k in body_start..func_end {
                if infos[k].is_nop() {
                    continue;
                }
                // Skip the save instruction itself
                if k == save.save_line_idx {
                    continue;
                }

                if let LineKind::LoadRbp {
                    reg: load_reg,
                    size: MoveSize::Q,
                    ..
                } = infos[k].kind
                {
                    if load_reg == reg && is_near_epilogue(infos, k) {
                        restore_indices.push(k);
                        continue;
                    }
                }
                // Also match popq for push/pop-style saves
                if let LineKind::Pop { reg: pop_reg } = infos[k].kind {
                    if pop_reg == reg && is_near_epilogue(infos, k) {
                        restore_indices.push(k);
                        continue;
                    }
                }

                if line_references_reg_fast(&infos[k], reg) {
                    body_has_reference = true;
                    break;
                }
            }

            if !body_has_reference && !restore_indices.is_empty() {
                mark_nop(&mut infos[save.save_line_idx]);
                for &ri in &restore_indices {
                    mark_nop(&mut infos[ri]);
                }
            }
        }

        // Update leaq -N(%rbp), %rsp in epilogues when push-based saves were eliminated.
        // The leaq offset must shrink by 8 for each eliminated push.
        let eliminated_pushes = saves
            .iter()
            .filter(|s| s.is_push && infos[s.save_line_idx].is_nop())
            .count();
        if eliminated_pushes > 0 {
            let remaining_pushes = saves
                .iter()
                .filter(|s| s.is_push && !infos[s.save_line_idx].is_nop())
                .count();
            let new_offset = -(remaining_pushes as i64 * 8);
            let old_offset = -((remaining_pushes + eliminated_pushes) as i64 * 8);
            let old_leaq = format!("leaq {}(%rbp), %rsp", old_offset);
            let new_leaq = if remaining_pushes > 0 {
                format!("    leaq {}(%rbp), %rsp", new_offset)
            } else {
                "    movq %rbp, %rsp".to_string()
            };

            for k in body_start..func_end {
                if infos[k].is_nop() {
                    continue;
                }
                let line = infos[k].trimmed(store.get(k));
                if line.contains(&old_leaq) {
                    replace_line(store, &mut infos[k], k, new_leaq.clone());
                }
            }
        }

        // Note: we intentionally do NOT shrink the stack frame (subq $N, %rsp)
        // even though some callee-saved saves were eliminated. The remaining saves
        // still reference their original rbp-relative offsets, which are below rsp
        // if we shrink the frame. Data below rsp can be corrupted by interrupts
        // or signal handlers. Keeping the original frame size ensures all saved
        // registers remain safely above rsp. The unused slots become dead space.
        // TODO: To also shrink the frame, we would need to rewrite the offsets of
        // all remaining callee-saved saves/restores to pack them tightly.

        i = func_end;
    }
}

/// Frame-pointer-omitted variant of [`eliminate_unused_callee_saves`].
///
/// Shape (as emitted for `-O2` leaves):
///
/// ```text
/// f:
/// .cfi_startproc
///     pushq %rbx
///     pushq %r12
///     pushq %r13            <- never referenced in the body
///     .cfi_def_cfa_offset 32
///     …
///     popq %r13
///     popq %r12
///     popq %rbx
///     ret
/// .cfi_endproc
/// ```
///
/// Soundness gates (all must hold, otherwise the function is left alone):
///
/// * no `call` in the body — a removed push changes the 16-byte alignment of
///   `%rsp` at call sites;
/// * no `and… %rsp` realignment and no `xmm`/`ymm` operand together with an
///   `(%rsp` memory operand — aligned spills would move by 8;
/// * exactly one `.cfi_def_cfa_offset` in the function (the prologue's); it is
///   rewritten to `old − 8·eliminated` so unwind info stays exact;
/// * every restore is a `popq` of the same register on an epilogue path.
///
/// The kernel corpus `isort` saved `%r13` without a single body use after
/// the RA's leaf policy moved its loop state into caller-saved registers.
pub(super) fn eliminate_unused_callee_saves_fpo(store: &mut LineStore, infos: &mut [LineInfo]) {
    let len = store.len();
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || !infos[i].trimmed(store.get(i)).starts_with(".cfi_startproc") {
            i += 1;
            continue;
        }
        // Function extent: up to and including `.cfi_endproc`.
        let mut func_end = len;
        for k in i + 1..len {
            if !infos[k].is_nop() && infos[k].trimmed(store.get(k)).starts_with(".cfi_endproc") {
                func_end = k;
                break;
            }
        }
        // Leading callee-saved pushes (directives interleaved are allowed).
        let mut saves: Vec<(RegId, usize)> = Vec::new();
        let mut j = i + 1;
        while j < func_end {
            if infos[j].is_nop() || infos[j].kind == LineKind::Directive {
                j += 1;
                continue;
            }
            match infos[j].kind {
                LineKind::Push { reg } if is_callee_saved_reg(reg) => {
                    saves.push((reg, j));
                    j += 1;
                }
                _ => break,
            }
        }
        if saves.is_empty() {
            i = func_end.max(i + 1);
            continue;
        }
        let body_start = j;

        // Gates.
        let mut cfa_directive: Option<(usize, i64)> = None;
        let mut safe = true;
        for k in i + 1..func_end {
            if infos[k].is_nop() {
                continue;
            }
            let line = infos[k].trimmed(store.get(k));
            match infos[k].kind {
                LineKind::Call => {
                    safe = false;
                    break;
                }
                LineKind::Directive => {
                    if let Some(v) = line.strip_prefix(".cfi_def_cfa_offset ") {
                        match (cfa_directive, v.trim().parse::<i64>()) {
                            (None, Ok(n)) => cfa_directive = Some((k, n)),
                            _ => {
                                safe = false;
                                break;
                            }
                        }
                    } else if line.starts_with(".cfi_offset")
                        || line.starts_with(".cfi_def_cfa ")
                        || line.starts_with(".cfi_def_cfa_register")
                    {
                        safe = false;
                        break;
                    }
                    continue;
                }
                _ => {}
            }
            if line.contains("%rsp") {
                let is_stack_adjust =
                    matches!(infos[k].kind, LineKind::Push { .. } | LineKind::Pop { .. })
                        || line.starts_with("subq $")
                        || line.starts_with("addq $");
                // Removing a leading push changes the runtime %rsp by 8 bytes,
                // so EVERY %rsp-relative data reference in the body (va-register
                // save areas, `leaq N(%rsp)` overflow pointers, spill slots,
                // return-address reads) would point 8 bytes off. Only stack
                // adjustments are offset-independent; anything else addressing
                // the stack makes the elimination unsafe.
                if !is_stack_adjust {
                    safe = false;
                    break;
                }
            }
        }
        if !safe {
            i = func_end.max(i + 1);
            continue;
        }

        let mut eliminated = 0usize;
        for &(reg, save_idx) in &saves {
            let mut pops: Vec<usize> = Vec::new();
            let mut referenced = false;
            for k in body_start..func_end {
                if infos[k].is_nop() {
                    continue;
                }
                if let LineKind::Pop { reg: pop_reg } = infos[k].kind {
                    if pop_reg == reg {
                        if is_near_epilogue(infos, k) {
                            pops.push(k);
                            continue;
                        }
                        referenced = true;
                        break;
                    }
                }
                if line_references_reg_fast(&infos[k], reg) {
                    referenced = true;
                    break;
                }
            }
            if referenced || pops.is_empty() {
                continue;
            }
            mark_nop(&mut infos[save_idx]);
            for &p in &pops {
                mark_nop(&mut infos[p]);
            }
            eliminated += 1;
        }
        if eliminated > 0 {
            if let Some((k, old)) = cfa_directive {
                let new = old - 8 * eliminated as i64;
                replace_line(store, &mut infos[k], k, format!("    .cfi_def_cfa_offset {}", new));
            }
        }
        i = func_end.max(i + 1);
    }
}
