//! Frame compaction pass.
//!
//! After all other peephole passes (dead store elimination, callee-save elimination),
//! the stack frame may contain unused gaps:
//! - Callee-saved saves that were NOP'd (register unused after optimization)
//! - Dead stores that were NOP'd (slot never read)
//!
//! This pass detects such gaps and compacts the frame by:
//! 1. Finding the maximum negative rbp offset actually referenced in the body
//! 2. Packing remaining callee-saved saves tightly below the body area
//! 3. Rewriting `subq $N, %rsp` to the new smaller frame size
//! 4. Rewriting all callee-saved save/restore offsets to their new positions
//!
//! This is safe because all stack access uses rbp-relative addressing, so
//! only the callee-saved offsets and the subq value need to change.

use super::super::types::*;
use super::helpers::*;

/// Skip NOP and *benign* Directive lines (CFI directives appear between
/// prologue instructions; a data directive does not, and stops the scan).
fn next_non_nop_or_directive(
    store: &LineStore,
    infos: &[LineInfo],
    start: usize,
    limit: usize,
) -> usize {
    let mut j = start;
    while j < limit && super::helpers::steps_over_line(&infos[j], infos[j].trimmed(store.get(j))) {
        j += 1;
    }
    j
}

pub(super) fn compact_frame(store: &mut LineStore, infos: &mut [LineInfo]) {
    let len = store.len();
    if len == 0 {
        return;
    }
    let mut i = 0;
    while i < len {
        // Look for prologue: pushq %rbp
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        if !matches!(infos[i].kind, LineKind::Push { reg: REG_RBP }) {
            i += 1;
            continue;
        }
        // Next non-nop/directive should be "movq %rsp, %rbp"
        let mut j = next_non_nop_or_directive(store, infos, i + 1, len);
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
        // Next non-nop/directive should be "subq $N, %rsp"
        j = next_non_nop_or_directive(store, infos, j, len);
        if j >= len {
            i = j;
            continue;
        }
        let subq_idx = j;
        let subq_line = infos[j].trimmed(store.get(j));
        let old_frame_size: i64 = if let Some(rest) = subq_line.strip_prefix("subq $") {
            if let Some(val_str) = rest.strip_suffix(", %rsp") {
                match val_str.parse::<i64>() {
                    Ok(v) => v,
                    Err(_) => {
                        i = j + 1;
                        continue;
                    }
                }
            } else {
                i = j + 1;
                continue;
            }
        } else {
            i = j + 1;
            continue;
        };
        j += 1;
        // Collect callee-saved register saves immediately after subq.
        struct CalleeSave {
            reg: RegId,
            offset: i32,
            line_idx: usize,
            alive: bool, // not NOP'd
        }
        let mut saves: Vec<CalleeSave> = Vec::new();
        j = next_non_nop_or_directive(store, infos, j, len);
        let mut scan = j;
        while scan < len {
            if infos[scan].is_nop() {
                // Check if this was originally a callee-saved save that got NOP'd
                // We can't recover the offset, but we know there's a gap
                scan += 1;
                continue;
            }
            if let LineKind::StoreRbp {
                reg,
                offset,
                size: MoveSize::Q,
            } = infos[scan].kind
            {
                if is_callee_saved_reg(reg) && offset < 0 {
                    saves.push(CalleeSave {
                        reg,
                        offset,
                        line_idx: scan,
                        alive: true,
                    });
                    scan += 1;
                    continue;
                }
            }
            break;
        }

        if saves.is_empty() {
            i = scan;
            continue;
        }

        let body_start = scan;

        // Find the end of this function
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

        // Collect all rbp offsets that are READ in the body (loads, memory operands, leaq).
        // Store-only offsets are dead stores — they don't need space in the frame,
        // BUT they still execute and write to their original offsets. If we relocate
        // callee-saves to overlap with dead-store offsets, those stores will clobber
        // the callee-save values. We track store-only offsets so we can NOP them.
        let callee_save_offsets: Vec<i32> = saves.iter().map(|s| s.offset).collect();
        let mut read_offsets: Vec<i32> = Vec::new();
        // Track store-only: (offset, size_bytes, line_idx) for stores that aren't read
        let mut store_only_lines: Vec<(i32, i32, usize)> = Vec::new();
        let mut bail = false;

        for k in body_start..func_end {
            if infos[k].is_nop() {
                continue;
            }

            match infos[k].kind {
                LineKind::StoreRbp { offset, size, .. } => {
                    // Track store offset, size and line for potential NOP-out later.
                    // Only track negative offsets not in callee-save area.
                    if offset < 0 && !callee_save_offsets.contains(&offset) {
                        store_only_lines.push((offset, size.byte_size(), k));
                    }
                }
                LineKind::LoadRbp { offset, .. } => {
                    // Skip callee-saved restores
                    if callee_save_offsets.contains(&offset) && is_near_epilogue(infos, k) {
                        continue;
                    }
                    read_offsets.push(offset);
                }
                _ => {
                    let rbp_off = infos[k].rbp_offset;
                    if rbp_off != RBP_OFFSET_NONE {
                        let line = infos[k].trimmed(store.get(k));
                        if line.starts_with("leaq ")
                            && (line.contains("(%rbp)") || line.contains("(%rsp)"))
                        {
                            // leaq takes address of a stack slot — bail.
                            bail = true;
                            break;
                        }
                        if !callee_save_offsets.contains(&rbp_off) {
                            read_offsets.push(rbp_off);
                        }
                    } else {
                        let line = infos[k].trimmed(store.get(k));
                        if line.contains("(%rbp)") || line.contains("(%rsp)") {
                            // Unrecognized rbp reference — bail
                            bail = true;
                            break;
                        }
                    }
                }
            }
        }

        // Remove store_only_lines entries whose byte ranges overlap with any read.
        // A store at offset X with size S covers bytes [X, X+S). Since we only
        // track read offsets (not sizes), conservatively assume each read is 8
        // bytes (the maximum movq size) to avoid missing overlaps.
        store_only_lines.retain(|&(off, sz, _)| {
            !read_offsets
                .iter()
                .any(|&r_off| ranges_overlap(off, sz, r_off, 8))
        });

        if bail {
            i = func_end;
            continue;
        }

        // Also need to count stores to offsets that ARE read (those slots are live)
        // and any positive rbp offsets (incoming arg area above rbp).
        // The min_body_offset is the most-negative READ offset.
        let mut min_body_offset: i32 = 0;
        for &off in &read_offsets {
            if off < min_body_offset {
                min_body_offset = off;
            }
        }

        // Also check stores: if a store goes to an offset that's later read,
        // the slot is needed. But we only need space for the deepest READ offset,
        // not for store-only offsets. However, there's a subtlety: a store to
        // offset X might be read by a load at offset X in a different code path.
        // The read_offsets set already captures all loads, so min_body_offset
        // based on read_offsets is correct.

        // min_body_offset is the most-negative offset used by the function body.
        // We need callee-saved saves to be packed immediately below this.
        let body_space = (-min_body_offset) as i64; // positive number of bytes used by body
        let num_alive_saves = saves.iter().filter(|s| s.alive).count() as i64;
        let callee_space = num_alive_saves * 8;
        let total_needed = body_space + callee_space;

        // Align to 16 bytes
        let new_frame_size = (total_needed + 15) & !15;

        if new_frame_size >= old_frame_size {
            i = func_end;
            continue;
        }

        // Compute new callee-saved offsets: pack them at the bottom of the new frame.
        // The first callee-saved reg goes at -new_frame_size, next at -new_frame_size+8, etc.
        let mut new_offsets: Vec<(RegId, i32, i32)> = Vec::new(); // (reg, old_offset, new_offset)
        let mut slot = 0i64;
        for save in &saves {
            if save.alive {
                let new_offset = -(new_frame_size - slot) as i32;
                new_offsets.push((save.reg, save.offset, new_offset));
                slot += 8;
            }
        }

        // NOP out dead stores whose byte ranges are entirely below the body read area.
        // After frame compaction, these offsets may fall:
        // (a) in the relocated callee-save region, clobbering saved registers, or
        // (b) below the stack pointer entirely, causing memory corruption.
        // Since they're never read, NOP-ing them is always safe.
        for &(off, sz, line_idx) in &store_only_lines {
            // The store covers [off, off+sz). It's safe to NOP only if
            // the entire store range is below the body read area.
            let store_top = off + sz;
            if store_top <= min_body_offset {
                mark_nop(&mut infos[line_idx]);
            }
        }

        // Rewrite subq $N, %rsp
        let new_subq = format!("    subq ${}, %rsp", new_frame_size);
        replace_line(store, &mut infos[subq_idx], subq_idx, new_subq);

        // Rewrite callee-saved saves in prologue
        for save in &saves {
            if !save.alive {
                continue;
            }
            if let Some(&(_, _, new_off)) = new_offsets.iter().find(|&&(r, _, _)| r == save.reg) {
                if new_off != save.offset {
                    let reg_name = reg_id_to_name_q(save.reg);
                    let orig = infos[save.line_idx].trimmed(store.get(save.line_idx));
                    let base = if orig.contains("(%rsp)") {
                        "rsp"
                    } else {
                        "rbp"
                    };
                    let new_line = format!("    movq {}, {}(%{})", reg_name, new_off, base);
                    replace_line(store, &mut infos[save.line_idx], save.line_idx, new_line);
                }
            }
        }

        // Rewrite callee-saved restores in epilogues
        for k in body_start..func_end {
            if infos[k].is_nop() {
                continue;
            }
            if let LineKind::LoadRbp {
                reg,
                offset,
                size: MoveSize::Q,
            } = infos[k].kind
            {
                if is_callee_saved_reg(reg) && is_near_epilogue(infos, k) {
                    if let Some(&(_, _, new_off)) = new_offsets
                        .iter()
                        .find(|&&(r, old_off, _)| r == reg && old_off == offset)
                    {
                        if new_off != offset {
                            let reg_name = reg_id_to_name_q(reg);
                            let orig = infos[k].trimmed(store.get(k));
                            let base = if orig.contains("(%rsp)") {
                                "rsp"
                            } else {
                                "rbp"
                            };
                            let new_line = format!("    movq {}(%{}), {}", new_off, base, reg_name);
                            replace_line(store, &mut infos[k], k, new_line);
                        }
                    }
                }
            }
        }

        i = func_end;
    }
}

/// Convert a register family ID to its 64-bit name with % prefix.
fn reg_id_to_name_q(reg: RegId) -> &'static str {
    match reg {
        0 => "%rax",
        1 => "%rcx",
        2 => "%rdx",
        3 => "%rbx",
        4 => "%rsp",
        5 => "%rbp",
        6 => "%rsi",
        7 => "%rdi",
        8 => "%r8",
        9 => "%r9",
        10 => "%r10",
        11 => "%r11",
        12 => "%r12",
        13 => "%r13",
        14 => "%r14",
        15 => "%r15",
        _ => "%rax",
    }
}

/// Remove a never-referenced constant frame outright, or shrink it to the
/// alignment minimum when the function makes calls.
///
/// Shape (frame pointer omitted): `k ≥ 0` callee-saved pushes directly
/// after the function label, then `subq $N, %rsp`; a body that never names
/// `%rsp`; and every `ret` reached through `addq $N, %rsp` followed by
/// exactly `k` pops. Epilogues are classified by bidirectional ret-pairing,
/// not by string identity: an `addq $N, %rsp` is an epilogue only when the
/// next real lines (modulo directives/NOPs) are `k` pops and a `ret`, and
/// every `ret` must reach such an `addq` backwards through those pops. A
/// mid-body stack readjustment that happens to spell `addq $N, %rsp`, any
/// other push/pop, inline asm or `%rsp` mention declines the fold. The
/// frame exists because slots the register allocator reserved were retired
/// by later peephole passes (or the allocator homed every value in a
/// register after the frame size was fixed); nothing observes it.
///
/// Soundness: on every entry→ret path the prologue `subq` is matched by
/// exactly one collected `addq` (each collected `addq` is followed only by
/// pops and a `ret`, so no path crosses two), and the pushes and pops are
/// untouched, so deleting or resizing the pair keeps `%rsp` exact at every
/// `ret`, and nothing in the body depends on the absolute `%rsp` value.
///
/// * **Leaf** (no `call`): delete the `subq` and every matched `addq`. A
///   call-free body that never addresses through `%rsp` observes neither
///   the address nor the alignment of the adjustment.
/// * **Calls present**: the ABI requires `%rsp ≡ 0 (mod 16)` at every call
///   site, i.e. `8 + 8k + N ≡ 0 (mod 16)`. Shrink `N` to the smallest value
///   with the same residue (`8` for even `k`, `0` for odd `k`, deleting the
///   pair); a misaligned original frame declines.
///
/// CFI stays exact for every row, not only the prologue's: the codegen
/// restates the full-frame CFA after each mid-body `ret`, so EVERY
/// `.cfi_def_cfa_offset` equal to the frame total `8 + 8k + N` is rewritten
/// to `8 + 8k + N'`; values inside the push region (`≤ 8 + 8k`) are kept and
/// any other value, `.cfi_def_cfa`, `.cfi_def_cfa_register` or
/// `.cfi_adjust_cfa_offset` declines before anything is changed. Rows that
/// then restate the current CFA are dropped.
pub(super) fn remove_dead_leaf_frame(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    if len == 0 {
        return false;
    }
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        let line = infos[i].trimmed(store.get(i));
        let Some(n) = parse_subq_rsp(line) else {
            i += 1;
            continue;
        };
        let Some((func_start, pushes)) = fpo_prologue_pushes(store, infos, i) else {
            i += 1;
            continue;
        };
        // Audit the whole function body: collect the epilogue `addq`s and
        // the CFA rows, require every ret to be paired with an epilogue,
        // reject any other reference to the stack pointer or opaque/stack-
        // shaping line.
        let mut epilogues: Vec<usize> = Vec::new();
        let mut epilogue_pops: Vec<usize> = Vec::new();
        let mut cfa_rows: Vec<(usize, i64)> = Vec::new();
        let mut has_call = false;
        let mut decline = false;
        let mut func_end = len;
        let mut j = i + 1;
        while j < len {
            if super::helpers::steps_over_line(&infos[j], infos[j].trimmed(store.get(j))) {
                let dj = infos[j].trimmed(store.get(j));
                if dj.starts_with(".size ") {
                    func_end = j + 1;
                    break;
                }
                if let Some(v) = dj.strip_prefix(".cfi_def_cfa_offset ") {
                    match v.trim().parse::<i64>() {
                        Ok(v) => cfa_rows.push((j, v)),
                        Err(_) => decline = true,
                    }
                } else if dj.starts_with(".cfi_def_cfa") || dj.starts_with(".cfi_adjust_cfa_offset")
                {
                    decline = true;
                }
                j += 1;
                continue;
            }
            let body = infos[j].trimmed(store.get(j));
            if body == format!("addq ${}, %rsp", n) {
                // WO-1 (red-team audit, 2026-09-18): string identity alone
                // does not make an epilogue. The addq is one only when what
                // FOLLOWS (skipping exactly the line classes the backward
                // ret-pairing scan skips) is the full pop sequence and a
                // `ret`; a mid-body readjustment that merely spells the same
                // `addq $N, %rsp` would otherwise be deleted without a
                // matching prologue deletion on its path.
                let mut q = j + 1;
                let mut pops: Vec<usize> = Vec::new();
                let mut ret_paired = false;
                while q < len {
                    if super::helpers::steps_over_line(&infos[q], infos[q].trimmed(store.get(q))) {
                        q += 1;
                        continue;
                    }
                    if matches!(infos[q].kind, LineKind::Pop { .. }) && pops.len() < pushes {
                        pops.push(q);
                        q += 1;
                        continue;
                    }
                    ret_paired = pops.len() == pushes && matches!(infos[q].kind, LineKind::Ret);
                    break;
                }
                if ret_paired {
                    epilogues.push(j);
                    epilogue_pops.extend(pops);
                } else {
                    decline = true;
                }
                j += 1;
                continue;
            }
            // A TLS-descriptor call is classified `Other` for its register
            // effects (%rax only) but is a real call for alignment: deleting
            // the frame left `%rsp ≡ 8 (mod 16)` at `call *x@TLSCALL(%rax)`,
            // and glibc's descriptor slow path calls `__tls_get_addr` assuming
            // an ABI-aligned stack (GCC aligns for it, too).
            if is_tls_descriptor_call(body) {
                has_call = true;
                j += 1;
                continue;
            }
            match infos[j].kind {
                LineKind::Call => {
                    has_call = true;
                    j += 1;
                    continue;
                }
                LineKind::Ret => {
                    // A ret must be reached (modulo directives/NOPs and the
                    // epilogue's own pops) from a collected epilogue addq,
                    // or this path would return with an unbalanced stack
                    // under the fold.
                    let mut p = j;
                    let mut paired = false;
                    while p > i {
                        p -= 1;
                        if super::helpers::steps_over_line(
                            &infos[p],
                            infos[p].trimmed(store.get(p)),
                        ) || epilogue_pops.contains(&p)
                        {
                            continue;
                        }
                        paired = epilogues.contains(&p);
                        break;
                    }
                    decline |= !paired;
                    j += 1;
                    continue;
                }
                LineKind::Pop { .. } if epilogue_pops.contains(&j) => {
                    j += 1;
                    continue;
                }
                LineKind::Push { .. } | LineKind::Pop { .. } | LineKind::InlineAsm => {
                    // Body push/pop shift the frame offsets; inline asm is
                    // opaque to the %rsp audit.
                    decline = true;
                    j += 1;
                    continue;
                }
                _ => {}
            }
            if body.contains("%rsp") || body.contains("%esp") {
                // Any other explicit reference to the stack pointer — a
                // memory operand, register read, or address computation.
                decline = true;
            }
            j += 1;
        }
        let next = func_end.min(len - 1) + 1;
        let push_top = 8 + 8 * pushes as i64;
        let total = push_top + n;
        let new_n = if !has_call {
            0
        } else if total % 16 == 0 {
            8 * (pushes as i64 % 2 == 0) as i64
        } else {
            n
        };
        let cfa_ok = cfa_rows.iter().all(|&(_, v)| v == total || v <= push_top);
        if epilogues.is_empty() || decline || new_n >= n || !cfa_ok {
            i = next;
            continue;
        }

        if new_n == 0 {
            mark_nop(&mut infos[i]);
            for &ep in &epilogues {
                mark_nop(&mut infos[ep]);
            }
        } else {
            replace_line(store, &mut infos[i], i, format!("    subq ${new_n}, %rsp"));
            for &ep in &epilogues {
                replace_line(
                    store,
                    &mut infos[ep],
                    ep,
                    format!("    addq ${new_n}, %rsp"),
                );
            }
        }
        for &(k, v) in &cfa_rows {
            if v == total {
                let row = format!("    .cfi_def_cfa_offset {}", push_top + new_n);
                replace_line(store, &mut infos[k], k, row);
            }
        }
        drop_restated_cfa(store, infos, func_start, func_end);
        changed = true;
        i = next;
    }
    changed
}

/// `(function start, push count)` when the real instructions before the
/// `subq` at `at` are only callee-saved pushes back to the function label
/// or `.cfi_startproc`. In frame-pointer-omitted code `%rbp` is an ordinary
/// callee-saved register; a frame-pointer prologue has `movq %rsp, %rbp`
/// between its push and the `subq` and is refused here.
fn fpo_prologue_pushes(store: &LineStore, infos: &[LineInfo], at: usize) -> Option<(usize, usize)> {
    let mut pushes = 0;
    let mut p = at;
    while p > 0 {
        p -= 1;
        let line = infos[p].trimmed(store.get(p));
        if super::helpers::steps_over_line(&infos[p], line) {
            if line.starts_with(".cfi_startproc") {
                return Some((p, pushes));
            }
            continue;
        }
        match infos[p].kind {
            LineKind::Push { reg } if is_callee_saved_reg(reg) || reg == REG_RBP => pushes += 1,
            _ => return line.ends_with(':').then_some((p, pushes)),
        }
    }
    None
}

/// Drop `.cfi_def_cfa_offset` rows that restate the CFA offset already in
/// effect. CFI rows apply from their address onward in address order, so a
/// row equal to the running value (starting from the CIE's `rsp+8`) is a
/// no-op. `.cfi_remember_state`/`.cfi_restore_state` make the running value
/// non-linear; such functions are left alone.
fn drop_restated_cfa(store: &LineStore, infos: &mut [LineInfo], start: usize, end: usize) {
    let lines = start..end.min(infos.len());
    if lines.clone().any(|k| {
        let l = infos[k].trimmed(store.get(k));
        !infos[k].is_nop()
            && (l.starts_with(".cfi_remember_state") || l.starts_with(".cfi_restore_state"))
    }) {
        return;
    }
    let mut current = 8;
    for k in lines {
        if infos[k].is_nop() {
            continue;
        }
        let Some(v) = infos[k]
            .trimmed(store.get(k))
            .strip_prefix(".cfi_def_cfa_offset ")
        else {
            continue;
        };
        let Ok(v) = v.trim().parse::<i64>() else {
            return;
        };
        if v == current {
            mark_nop(&mut infos[k]);
        }
        current = v;
    }
}

/// Parse `subq $N, %rsp` (constant adjustment only).
fn parse_subq_rsp(line: &str) -> Option<i64> {
    let rest = line.strip_prefix("subq $")?;
    let val = rest.strip_suffix(", %rsp")?;
    val.parse::<i64>().ok()
}

#[cfg(test)]
#[path = "frame_compact_tests.rs"]
mod tests;
