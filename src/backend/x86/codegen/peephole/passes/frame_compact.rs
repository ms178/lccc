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

/// Skip NOP and Directive lines (CFI directives appear between prologue instructions).
fn next_non_nop_or_directive(infos: &[LineInfo], start: usize, limit: usize) -> usize {
    let mut j = start;
    while j < limit && (infos[j].is_nop() || matches!(infos[j].kind, LineKind::Directive)) {
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
        if !matches!(infos[i].kind, LineKind::Push { reg: 5 }) {
            i += 1;
            continue;
        }

        // Next non-nop/directive should be "movq %rsp, %rbp"
        let mut j = next_non_nop_or_directive(infos, i + 1, len);
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
        j = next_non_nop_or_directive(infos, j, len);
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
        j = next_non_nop_or_directive(infos, j, len);
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

/// Remove a never-referenced constant leaf frame outright, or shrink it to
/// the alignment minimum when the function makes calls.
///
/// Shape: a function whose prologue is exactly `subq $N, %rsp` (no
/// `pushq %rbp`, no callee-saved pushes) whose body never names `%rsp`,
/// and whose every `ret` is immediately preceded (modulo directives/NOPs)
/// by the matching `addq $N, %rsp` — and, symmetrically, every collected
/// `addq $N, %rsp` is immediately FOLLOWED (modulo directives/NOPs) by a
/// `ret`. Epilogues are classified by this bidirectional ret-pairing, not
/// by string identity: a mid-body stack readjustment that happens to
/// spell `addq $N, %rsp` sets the decline flag instead of being NOP'd.
/// The frame exists because slots the register allocator reserved were
/// retired by later peephole passes; nothing observes it.
///
/// * **Leaf** (no `call`): delete the `subq` and every matched `addq`. At
///   entry `%rsp ≡ 8 (mod 16)`; a leaf that never addresses through
///   `%rsp` and never calls has no alignment or address observation of
///   the adjustment at all, and every return site executes the matching
///   `addq` on the way out, so deleting the pair keeps `%rsp` exact on
///   every path.
/// * **Calls present**: the ABI requires `%rsp ≡ 0 (mod 16)` at every
///   call site — the entry value is ≡ 8, so the frame is (at least
///   partly) an alignment frame. When `N ≡ 8 (mod 16)` and `N > 8`,
///   shrink to `subq $8, %rsp` (the smallest alignment-preserving
///   adjustment); any other residue means the frame also carries an
///   outbound-argument or spill area that would have referenced `%rsp`
///   and cannot appear here.
///
/// The `.cfi_def_cfa_offset` that follows the `subq` is rewritten to the
/// new offset so unwind info stays truthful.
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
        // Prologue: subq $N, %rsp as the FIRST real instruction of the
        // function (the preceding lines are the label/CFI directives, so
        // no pushq %rbp and no callee-save pushes precede it).
        let line = infos[i].trimmed(store.get(i));
        let Some(n) = parse_subq_rsp(&line) else {
            i += 1;
            continue;
        };
        if !prologue_precedes_label(store, infos, i) {
            i += 1;
            continue;
        }

        // Audit the whole function body: collect the epilogue `addq`s,
        // require every ret to be paired with one, reject any other
        // reference to the stack pointer or opaque/stack-shaping line.
        let mut epilogues: Vec<usize> = Vec::new();
        let mut has_call = false;
        let mut rsp_named = false;
        let mut unpaired_ret = false;
        let mut func_end = len;
        let mut j = i + 1;
        while j < len {
            if infos[j].is_nop() || matches!(infos[j].kind, LineKind::Directive) {
                let dj = infos[j].trimmed(store.get(j));
                if dj.starts_with(".size ") {
                    func_end = j + 1;
                    break;
                }
                j += 1;
                continue;
            }
            let body = infos[j].trimmed(store.get(j));
            if body == format!("addq ${}, %rsp", n) {
                // WO-1 (red-team audit, 2026-09-18): string identity alone
                // does not make an epilogue. Classify by what FOLLOWS: the
                // addq is an epilogue only when the next real line (skipping
                // exactly the line classes the ret-pairing backward scan
                // skips — NOPs and directives) is a `ret`. A mid-body stack
                // readjustment that merely SPELLS the same `addq $N, %rsp`
                // (e.g. a future emitter's alloca cleanup or manual stack
                // ping-pong) would otherwise be NOP'd without a matching
                // prologue deletion on its path — an unbalanced stack.
                // Anything else that names %rsp declines the fold
                // (fail-closed), which is the pass's contract for shapes it
                // cannot classify.
                //
                // Soundness of the ret-paired rule: deleting the prologue
                // subq and every ret-paired addq preserves %rsp on every
                // entry→ret path because (a) every ret is preceded by a
                // collected addq (checked below), (b) every collected addq
                // is immediately followed by a ret, so a path crossing it
                // terminates there — no path crosses two collected addqs,
                // and each path's single prologue subq is matched by its
                // single terminal addq.
                let mut q = j + 1;
                let mut ret_paired = false;
                while q < len {
                    if infos[q].is_nop() || matches!(infos[q].kind, LineKind::Directive) {
                        q += 1;
                        continue;
                    }
                    ret_paired = matches!(infos[q].kind, LineKind::Ret);
                    break;
                }
                if ret_paired {
                    epilogues.push(j);
                } else {
                    rsp_named = true;
                }
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
                    // A ret must be preceded (modulo directives/NOPs) by a
                    // collected epilogue addq, or this path would return
                    // with an unbalanced stack under the fold.
                    let mut p = j;
                    let mut paired = false;
                    while p > i {
                        p -= 1;
                        if infos[p].is_nop() || matches!(infos[p].kind, LineKind::Directive) {
                            continue;
                        }
                        paired = epilogues.contains(&p);
                        break;
                    }
                    if !paired {
                        unpaired_ret = true;
                    }
                    j += 1;
                    continue;
                }
                LineKind::Push { .. } | LineKind::Pop { .. } | LineKind::InlineAsm => {
                    // push/pop shift the frame offsets; inline asm is
                    // opaque to the %rsp audit.
                    rsp_named = true;
                    j += 1;
                    continue;
                }
                _ => {}
            }
            if body.contains("%rsp") || body.contains("%esp") {
                // Any other explicit reference to the stack pointer — a
                // memory operand, register read, or address computation.
                rsp_named = true;
            }
            j += 1;
        }
        if epilogues.is_empty() || rsp_named || unpaired_ret {
            i = func_end.min(len - 1) + 1;
            continue;
        }

        if !has_call {
            // Leaf: delete the subq and every matched addq. Rewrite the
            // CFA-offset directive that follows the subq (if present) to
            // the entry offset 8.
            mark_nop(&mut infos[i]);
            for &ep in &epilogues {
                mark_nop(&mut infos[ep]);
            }
            rewrite_cfa_after(store, infos, i, 8);
            changed = true;
        } else if n % 16 == 8 && n > 8 {
            // Alignment frame with calls: shrink to the minimum.
            replace_line(store, &mut infos[i], i, "    subq $8, %rsp".to_string());
            for &ep in &epilogues {
                replace_line(store, &mut infos[ep], ep, "    addq $8, %rsp".to_string());
            }
            rewrite_cfa_after(store, infos, i, 16);
            changed = true;
        }
        i = func_end.min(len - 1) + 1;
    }
    changed
}

/// Rewrite the first `.cfi_def_cfa_offset` directive at or after `at` to
/// `new_offset` (the CFA distance from %rsp after the adjustment).
fn rewrite_cfa_after(store: &mut LineStore, infos: &mut [LineInfo], at: usize, new_offset: i64) {
    let len = store.len();
    let mut d = at + 1;
    while d < len && matches!(infos[d].kind, LineKind::Directive) {
        let dl = infos[d].trimmed(store.get(d));
        if dl.starts_with(".cfi_def_cfa_offset ") {
            replace_line(
                store,
                &mut infos[d],
                d,
                format!(".cfi_def_cfa_offset {}", new_offset),
            );
            return;
        }
        if dl.starts_with(".cfi_startproc") || dl.starts_with(".size ") {
            return;
        }
        d += 1;
    }
}

/// True when the nearest non-directive line before `at` is a function
/// label or `.cfi_startproc` — i.e. `at` is the first real instruction of
/// a function.
fn prologue_precedes_label(store: &LineStore, infos: &[LineInfo], at: usize) -> bool {
    let mut p = at;
    while p > 0 {
        p -= 1;
        if infos[p].is_nop() || matches!(infos[p].kind, LineKind::Directive) {
            if infos[p].trimmed(store.get(p)).starts_with(".cfi_startproc") {
                return true;
            }
            continue;
        }
        return infos[p].trimmed(store.get(p)).ends_with(':');
    }
    false
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
