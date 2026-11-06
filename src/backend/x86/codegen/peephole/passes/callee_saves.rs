//! Unused callee-saved register elimination pass.
//!
//! After peephole optimization, some callee-saved registers may no longer be
//! referenced in the function body (all uses were optimized away). This pass
//! detects such registers and removes their prologue save / epilogue restore
//! instructions.
//!
//! Frame contract (push style, frame pointer present): every push removed
//! raises the post-prologue %rsp by 8 while all local-slot offsets stay at
//! their rbp-relative addresses. The pass therefore GROWS the prologue
//! `subq $N, %rsp` by 8 per eliminated push so the frame bottom stays below
//! the deepest slot and the 16-byte %rsp alignment at call sites is kept;
//! the bytes below the remaining pushes become dead space. Elimination is
//! refused outright when the frame cannot be re-anchored: a stack-probe
//! prologue (no plain subq to rewrite), or a pushes-only frame whose body
//! still calls out or references negative %rbp offsets.
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
//!
//! A saved register that IS still used can often move instead:
//! [`rename_saved_values_into_free_scratch`] renames its value into a
//! caller-saved scratch register a call-free function never names, which
//! makes the save unused for the two passes above (transactionally: a rename
//! whose save survives them is reverted).

use super::super::types::*;
use super::helpers::*;
use super::liveness::FileLiveness;

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
        if !matches!(infos[i].kind, LineKind::Push { reg: REG_RBP }) {
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
            if super::helpers::steps_over_line(&infos[j], infos[j].trimmed(store.get(j))) {
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

        // Skip subq $N, %rsp if present (also skip directives), recording its
        // index and value: the push-elimination rewrite below must re-anchor
        // the frame depth through this very instruction. A stack-probe
        // prologue instead starts `movq $N, %r11` + a page-touch loop — no
        // plain subq exists to rewrite, so push elimination must refuse there
        // (handled by the gate below via prologue_subq == None).
        let mut prologue_subq: Option<(usize, i64)> = None;
        while j < len && super::helpers::steps_over_line(&infos[j], infos[j].trimmed(store.get(j)))
        {
            j += 1;
        }
        if j < len {
            let subq_line = infos[j].trimmed(store.get(j));
            if let Some(rest) = subq_line.strip_prefix("subq $") {
                if let Some(v) = rest
                    .strip_suffix(", %rsp")
                    .and_then(|v| v.parse::<i64>().ok())
                {
                    prologue_subq = Some((j, v));
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

        // Push-elimination safety gate. With a plain prologue subq the frame
        // can always be re-anchored (the rewrite below grows it), so pushes
        // may be eliminated. Without one, pushes are only removable when the
        // frame is pushes-only in the strong sense: no call (nothing is ever
        // pushed below %rsp by this function), and no negative %rbp memory
        // reference (no local slot area exists below the pushes — the layout
        // emits the subq precisely when slots exist). A stack-probe prologue
        // has no plain subq and huge slots, so it lands here and is refused.
        let push_elim_allowed = prologue_subq.is_some() || {
            let mut clean = true;
            'body: for k in body_start..func_end {
                if infos[k].is_nop() {
                    continue;
                }
                if matches!(infos[k].kind, LineKind::Call) {
                    clean = false;
                    break 'body;
                }
                // Stack-teardown lines (push/pop pairs, the epilogue's
                // `leaq -N(%rbp), %rsp` reset) are position-independent data
                // wise — the pass rewrites the leaq anyway. Only DATA
                // references below the pushes make a pushes-only frame
                // unsafe.
                let line = infos[k].trimmed(store.get(k));
                let is_teardown =
                    matches!(infos[k].kind, LineKind::Push { .. } | LineKind::Pop { .. })
                        || (line.starts_with("leaq ") && line.ends_with(", %rsp"));
                if is_teardown {
                    continue;
                }
                if let LineKind::StoreRbp { offset, .. } | LineKind::LoadRbp { offset, .. } =
                    infos[k].kind
                {
                    if offset < 0 {
                        clean = false;
                        break 'body;
                    }
                }
                let rbp_off = infos[k].rbp_offset;
                if rbp_off != RBP_OFFSET_NONE && rbp_off < 0 {
                    clean = false;
                    break 'body;
                }
            }
            clean
        };

        // For each callee-saved register, check if it's referenced in the body
        // (excluding the save/restore instructions themselves).
        for save in &saves {
            let reg = save.reg;
            // A push save can only be dropped when the frame contract allows
            // the re-anchor (see the gate above); movq-style saves address
            // their slots rbp-relatively and are unaffected by push removal.
            if save.is_push && !push_elim_allowed {
                continue;
            }

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
            // Re-anchor the frame: each dropped push raises the post-prologue
            // %rsp by 8 while every local-slot offset keeps its rbp-relative
            // address, so the deepest slots would dangle BELOW %rsp, where the
            // next call's pushed return address (and, in kernel mode, any
            // interrupt pt_regs frame) overwrites live data. Growing the subq
            // by 8 per eliminated push restores the original frame depth:
            // slot coverage AND the 16-byte %rsp alignment at call sites are
            // preserved exactly. (Observed fallout of the old no-rewrite
            // behaviour: acpi_match_platform_list kept its 36-byte
            // `struct acpi_table_header hdr` at -0x60(%rbp) while %rsp moved
            // to -0x58; the call's pushed return address landed inside hdr,
            // the header copy wrote "DSDT"+length over it, and the callee
            // `ret`urned to 0x21fc54445344 — boot page fault on garbage CR2.)
            // The epilogues already reset %rsp from %rbp, so the extra bytes
            // cost nothing at teardown; they are dead space below the
            // remaining pushes.
            if let Some((subq_idx, subq_val)) = prologue_subq {
                let grown = subq_val + 8 * eliminated_pushes as i64;
                replace_line(
                    store,
                    &mut infos[subq_idx],
                    subq_idx,
                    format!("    subq ${}, %rsp", grown),
                );
            }
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

        // Note: the subq is intentionally grown (not shrunk) above; a true
        // frame shrink would require re-laying-out every rbp-relative slot
        // offset, which belongs in the pre-emit layout, not a text peephole.

        i = func_end;
    }
}

/// Frame-pointer-omitted variant of [`eliminate_unused_callee_saves`].
///
/// Prologue: `pushq <callee-saved>…` right after `.cfi_startproc` (each
/// optionally followed by its `.cfi_def_cfa_offset` / `.cfi_offset`), then
/// optionally `subq $N, %rsp`; every epilogue mirrors it (`addq $N, %rsp`,
/// the pops in reverse push order). A pushed register the body never
/// references costs a push and a pop on every call (the RA assigned it,
/// later passes retired every use: chacha20_core, glibc_memcmp_bytes and
/// zstd_count's `main` carried such saves).
///
/// Two exact strategies:
///
/// * **Frame-preserving** (a prologue `subq $N, %rsp` exists): drop the
///   dead pushes and pops and grow the `subq` and every epilogue `addq` by
///   `8·k`. The frame total is unchanged, so the post-prologue `%rsp` is
///   identical: every `%rsp`-relative slot and incoming-argument offset,
///   aligned vector spills and call-site alignment stay valid, and calls are
///   fine. The grown frame stays below 4096 bytes: frames of a page or more
///   are probed under stack-clash protection, which a plain `subq` is not.
/// * **Shrinking** (no `subq`): drop the pushes outright; `%rsp` moves by
///   `8·k`, so only call-free bodies with no `%rsp`-relative data qualify.
///
/// CFI stays exact. `.cfi_def_cfa_offset 8+8m` (the first `m` pushes done)
/// becomes `8+8·(m − dead among them)`; offsets at or above the frame total
/// (body, after `ret`) are kept by the frame-preserving form and lowered by
/// `8·k` by the shrinking form; a value between the push region and the
/// total refuses. Surviving pushes' `.cfi_offset` slots are recomputed the
/// same way, and the dead registers' own directives go with them. Refused:
/// `.cfi_def_cfa`, `.cfi_def_cfa_register`, `.cfi_adjust_cfa_offset`, a
/// `.cfi_offset` outside the push set, `%rsp` realignment, an epilogue that
/// does not restore the full set in mirror order.
pub(super) fn eliminate_unused_callee_saves_fpo(store: &mut LineStore, infos: &mut [LineInfo]) {
    let len = store.len();
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || !infos[i].trimmed(store.get(i)).starts_with(".cfi_startproc") {
            i += 1;
            continue;
        }
        let func_end = (i + 1..len)
            .find(|&k| {
                !infos[k].is_nop() && infos[k].trimmed(store.get(k)).starts_with(".cfi_endproc")
            })
            .unwrap_or(len);
        fpo_function(store, infos, i, func_end);
        i = func_end.max(i + 1);
    }
}

/// Page size below which a plain `subq` frame needs no stack-clash probe.
const FPO_UNPROBED_FRAME_LIMIT: i64 = 4096;

/// `N` of `<prefix>N, %rsp` (e.g. `subq $`), non-negative.
fn rsp_imm(line: &str, prefix: &str) -> Option<i64> {
    line.strip_prefix(prefix)?
        .strip_suffix(", %rsp")?
        .parse::<i64>()
        .ok()
        .filter(|n| *n >= 0)
}

/// `(%register, offset)` of `.cfi_offset %reg, -O`.
fn cfi_offset_operands(line: &str) -> Option<(&str, i64)> {
    let (reg, off) = line.strip_prefix(".cfi_offset ")?.split_once(',')?;
    Some((reg.trim(), off.trim().parse::<i64>().ok()?))
}

fn next_insn(store: &LineStore, infos: &[LineInfo], from: usize, end: usize) -> usize {
    let mut k = from;
    while k < end && super::helpers::steps_over_line(&infos[k], infos[k].trimmed(store.get(k))) {
        k += 1;
    }
    k
}

fn prev_insn(store: &LineStore, infos: &[LineInfo], from: usize, start: usize) -> Option<usize> {
    let mut k = from;
    while k > start {
        k -= 1;
        if !super::helpers::steps_over_line(&infos[k], infos[k].trimmed(store.get(k))) {
            return Some(k);
        }
    }
    None
}

/// `true` when only pops and bookkeeping (CFI, nops) separate the pop at
/// `k` from a `ret`/`jmp` (tail call). Unlike [`is_near_epilogue`] this
/// steps over the per-pop `.cfi_def_cfa_offset` every epilogue carries.
fn fpo_epilogue_pop(store: &LineStore, infos: &[LineInfo], k: usize, end: usize) -> bool {
    let mut j = k + 1;
    loop {
        j = next_insn(store, infos, j, end);
        if j >= end {
            return false;
        }
        match infos[j].kind {
            LineKind::Pop { .. } => j += 1,
            LineKind::Ret | LineKind::Jmp | LineKind::JmpIndirect => return true,
            _ => return false,
        }
    }
}

fn fpo_function(store: &mut LineStore, infos: &mut [LineInfo], start: usize, func_end: usize) {
    // Leading callee-saved pushes. In FPO code `%rbp` is an ordinary
    // callee-saved register unless `movq %rsp, %rbp` follows its push.
    let mut saves: Vec<(RegId, usize)> = Vec::new();
    let mut j = next_insn(store, infos, start + 1, func_end);
    while j < func_end {
        match infos[j].kind {
            LineKind::Push { reg } if is_callee_saved_reg(reg) => saves.push((reg, j)),
            LineKind::Push { reg: REG_RBP } => {
                let next = next_insn(store, infos, j + 1, func_end);
                if next < func_end && infos[next].trimmed(store.get(next)) == "movq %rsp, %rbp" {
                    break;
                }
                saves.push((REG_RBP, j));
            }
            _ => break,
        }
        j = next_insn(store, infos, j + 1, func_end);
    }
    let p = saves.len();
    if p == 0 {
        return;
    }
    let body_start = j;
    let subq = (body_start < func_end)
        .then(|| rsp_imm(infos[body_start].trimmed(store.get(body_start)), "subq $"))
        .flatten();

    // Epilogue pops per register (collected for live registers too: the
    // mirror check below needs every epilogue); a register is live if
    // anything else in the body names it, including a non-epilogue pop.
    let mut pops: Vec<Vec<usize>> = vec![Vec::new(); p];
    let mut live = vec![false; p];
    for (t, &(reg, _)) in saves.iter().enumerate() {
        for k in body_start..func_end {
            if infos[k].is_nop() {
                continue;
            }
            if matches!(infos[k].kind, LineKind::Pop { reg: r } if r == reg)
                && fpo_epilogue_pop(store, infos, k, func_end)
            {
                pops[t].push(k);
                continue;
            }
            live[t] |= line_references_reg_fast(&infos[k], reg);
        }
    }
    // `mark_nop` silently keeps a pinned line: deleting a save whose
    // restore stays pinned (or vice versa) would desync the CFI and the
    // stack, so both must be editable before anything is rewritten.
    let dead: Vec<bool> = (0..p)
        .map(|t| {
            !live[t]
                && !pops[t].is_empty()
                && all_editable(infos, &[saves[t].1])
                && all_editable(infos, &pops[t])
        })
        .collect();
    let k_dead = dead.iter().filter(|&&d| d).count();
    if k_dead == 0 {
        return;
    }
    // Every epilogue restores the full set in mirror order: from each pop
    // of the last-pushed register, the next pops are the earlier pushes.
    if pops.iter().any(|v| v.len() != pops[p - 1].len()) {
        return;
    }
    for &first in &pops[p - 1] {
        let mut k = first;
        for t in (0..p).rev() {
            if k >= func_end || !matches!(infos[k].kind, LineKind::Pop { reg } if reg == saves[t].0)
            {
                return;
            }
            k = next_insn(store, infos, k + 1, func_end);
        }
    }

    // Body gates and the CFI directives to rewrite. Balanced transient
    // %rsp writes in the body (call-argument pushes and their `addq`, the
    // SSE1 scalar GPR/XMM bridge) need no gate: the frame-preserving
    // strategy leaves %rsp unchanged at every body instruction, and the
    // other strategy runs only without any %rsp data reference. Codegen
    // emits no CFI here (`cfi_synth` derives it from the final text), so
    // the same input reaches this pass with and without unwind tables;
    // check_cfi_invariants.sh (invariant 4) enforces that corpus-wide.
    let mut has_call = false;
    let mut rsp_data = false;
    let mut cfa_lines: Vec<(usize, i64)> = Vec::new();
    let mut offset_lines: Vec<(usize, usize)> = Vec::new();
    for k in start + 1..func_end {
        if infos[k].is_nop() {
            continue;
        }
        let line = infos[k].trimmed(store.get(k));
        match infos[k].kind {
            LineKind::Call => has_call = true,
            LineKind::Directive => {
                if let Some(v) = line.strip_prefix(".cfi_def_cfa_offset ") {
                    let Ok(v) = v.trim().parse::<i64>() else {
                        return;
                    };
                    cfa_lines.push((k, v));
                } else if line.starts_with(".cfi_offset") {
                    let Some((name, off)) = cfi_offset_operands(line) else {
                        return;
                    };
                    let Some(t) = saves
                        .iter()
                        .position(|&(r, _)| REG_NAMES[0][r as usize] == name)
                    else {
                        return;
                    };
                    if off != -(16 + 8 * t as i64) {
                        return;
                    }
                    offset_lines.push((k, t));
                } else if line.starts_with(".cfi_def_cfa")
                    || line.starts_with(".cfi_adjust_cfa_offset")
                {
                    return;
                }
                continue;
            }
            _ => {}
        }
        if line.contains("%rsp") {
            if line.starts_with("and") {
                return;
            }
            let adjust = matches!(infos[k].kind, LineKind::Push { .. } | LineKind::Pop { .. })
                || line.starts_with("subq $")
                || line.starts_with("addq $");
            rsp_data |= !adjust;
        }
    }
    // Strategy and epilogue `addq` sites.
    let push_top = 8 + 8 * p as i64;
    let grow = 8 * k_dead as i64;
    let mut addqs: Vec<usize> = Vec::new();
    let (total, preserving) = match subq {
        Some(n) => {
            if n + grow >= FPO_UNPROBED_FRAME_LIMIT {
                return;
            }
            for &first in &pops[p - 1] {
                match prev_insn(store, infos, first, body_start) {
                    Some(a) if rsp_imm(infos[a].trimmed(store.get(a)), "addq $") == Some(n) => {
                        addqs.push(a)
                    }
                    _ => return,
                }
            }
            (push_top + n, true)
        }
        None if !has_call && !rsp_data => (push_top, false),
        None => return,
    };
    let dead_before = |m: usize| dead[..m.min(p)].iter().filter(|&&d| d).count() as i64;
    let mut cfa_new: Vec<(usize, i64)> = Vec::with_capacity(cfa_lines.len());
    for &(k, v) in &cfa_lines {
        let new = if (8..=push_top).contains(&v) && v % 8 == 0 {
            let m = ((v - 8) / 8) as usize;
            8 + 8 * (m as i64 - dead_before(m))
        } else if v >= total {
            if preserving { v } else { v - grow }
        } else {
            return;
        };
        cfa_new.push((k, new));
    }

    // Commit: the dead saves, their restores and their `.cfi_offset`s.
    // CFA lines are never dropped by position (a lone directive after the
    // last push describes ALL pushes); every one is rewritten below and
    // same-address duplicates are collapsed afterwards.
    for t in (0..p).filter(|&t| dead[t]) {
        mark_nop(&mut infos[saves[t].1]);
        for &k in &pops[t] {
            mark_nop(&mut infos[k]);
        }
    }
    for &(k, t) in &offset_lines {
        if dead[t] {
            mark_nop(&mut infos[k]);
        }
    }
    if let Some(n) = subq {
        replace_line(
            store,
            &mut infos[body_start],
            body_start,
            format!("    subq ${}, %rsp", n + grow),
        );
        for &a in &addqs {
            replace_line(
                store,
                &mut infos[a],
                a,
                format!("    addq ${}, %rsp", n + grow),
            );
        }
    }
    for (k, new) in cfa_new {
        if !infos[k].is_nop() {
            replace_line(
                store,
                &mut infos[k],
                k,
                format!("    .cfi_def_cfa_offset {new}"),
            );
        }
    }
    for (k, t) in offset_lines {
        if !infos[k].is_nop() {
            let slot = 16 + 8 * (t as i64 - dead_before(t));
            let name = REG_NAMES[0][saves[t].0 as usize];
            replace_line(
                store,
                &mut infos[k],
                k,
                format!("    .cfi_offset {name}, -{slot}"),
            );
        }
    }
    collapse_superseded_cfa(store, infos, start, func_end);
}

/// Drop `.cfi_def_cfa_offset` lines another one supersedes at the same
/// address (only other `.cfi_*` lines between them), and a
/// leading `8` that restates the x86-64 CIE's initial `rsp+8`.
fn collapse_superseded_cfa(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    start: usize,
    end: usize,
) {
    let mut pending: Option<usize> = None;
    let mut at_entry = true;
    for k in start + 1..end {
        if infos[k].is_nop() {
            continue;
        }
        let line = infos[k].trimmed(store.get(k));
        if let Some(v) = line.strip_prefix(".cfi_def_cfa_offset ") {
            if let Some(prev) = pending.replace(k) {
                mark_nop(&mut infos[prev]);
            }
            if at_entry && v.trim() == "8" {
                mark_nop(&mut infos[k]);
                pending = None;
            }
            continue;
        }
        // Only CFI bookkeeping is address-neutral: an alignment directive
        // pads, so CFA lines on either side of it are distinct rows.
        if !line.starts_with(".cfi_") {
            pending = None;
            at_entry = false;
        }
    }
}

// ── Saved values into free scratch registers ────────────────────────────────

/// Scratch families a call-free function may take over without a save:
/// caller-saved under SysV and Microsoft x64 alike, never an argument, return
/// or IPA-RA keep register (`ipa_ra::KEEP_CANDIDATES` is `%rsi %rdi %r8 %r9`,
/// so no caller ever relies on a callee preserving these two), and with no
/// implicit use outside `syscall`/`sysret`, which the implicit-operand oracle
/// reports.
const FREE_SCRATCH: [RegId; 2] = [11, 10];

/// What [`rename_saved_values_into_free_scratch`] rewrote, for the
/// transactional revert: per renamed family, its prologue save line and the
/// original text of every rewritten line.
pub(super) struct ScratchRenames {
    renames: Vec<(usize, Vec<(usize, String)>)>,
}

impl ScratchRenames {
    /// Undo every rename whose save line survived Phase 6 (a frame shape the
    /// elimination refuses): the rename pays only by deleting the save and
    /// its restores, so a surviving pair would just add REX prefixes.
    /// Returns how many renames were kept.
    pub(super) fn revert_unprofitable(
        self,
        store: &mut LineStore,
        infos: &mut [LineInfo],
    ) -> usize {
        let mut kept = 0;
        for (save, lines) in self.renames {
            if infos[save].is_nop() {
                kept += 1;
                continue;
            }
            for (k, old) in lines {
                replace_line(store, &mut infos[k], k, old);
            }
        }
        kept
    }
}

/// Move the value of a used callee-saved register into a caller-saved
/// scratch register the function never names, so that Phase 6
/// ([`eliminate_unused_callee_saves`], [`eliminate_unused_callee_saves_fpo`])
/// drops its `pushq`/`popq` pair.
///
/// The register allocator assigns homes before the peephole folds staged
/// address arithmetic into memory operands, so a leaf that needed `%r10` and
/// `%r11` at allocation time can end with both unused while a temporary
/// still lives in `%rbx` behind a save/restore pair (the `halve_add` loop:
/// `pushq %rbx; …; movl %edx, %ebx; sarl $1, %ebx; …; popq %rbx`). GCC 16.2
/// and Clang 23.1.5 never save a callee-saved register while a scratch
/// register is free.
///
/// Legal, for the function bounded by `.cfi_startproc`/`.cfi_endproc`, when:
/// 1. it makes no call and holds no inline asm, pinned line, indirect or
///    tail jump (a callee could read the scratch register as an argument or
///    static chain, or clobber it) and no `iret`/`sysret` (an interrupt or
///    system-call frame preserves every register);
/// 2. the saved family is pushed exactly once, in the prologue, popped only
///    in epilogues, and its incoming value is dead after the push
///    ([`FileLiveness`]) — every other mention belongs to the function's
///    own value;
/// 3. the scratch family is mentioned nowhere in the function, and no
///    instruction uses it or the saved family implicitly (`cpuid`,
///    `cmpxchg16b`, `syscall`, …);
/// 4. no rewritten line names a high-byte register (`%bh` has no `%r11`
///    spelling, and `%ah`..`%dh` cannot be encoded beside a REX operand).
///
/// The scratch register's incoming value is never read (it is never named
/// and no call or tail jump forwards it), `ret` reads only `%rax`/`%rdx`,
/// and the caller assumes it clobbered, so taking it over is invisible
/// outside the function. Directives are never rewritten: the `.cfi_offset`
/// of the save belongs to the pair Phase 6 deletes.
pub(super) fn rename_saved_values_into_free_scratch(
    store: &mut LineStore,
    infos: &mut [LineInfo],
) -> ScratchRenames {
    let len = store.len();
    let mut out = ScratchRenames {
        renames: Vec::new(),
    };
    let mut lv: Option<FileLiveness> = None;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || !infos[i].trimmed(store.get(i)).starts_with(".cfi_startproc") {
            i += 1;
            continue;
        }
        let end = (i + 1..len)
            .find(|&k| {
                !infos[k].is_nop() && infos[k].trimmed(store.get(k)).starts_with(".cfi_endproc")
            })
            .unwrap_or(len);
        rename_in_function(store, infos, i, end, &mut lv, &mut out);
        i = end.max(i + 1);
    }
    out
}

/// High-byte spellings: no `%r10`/`%r11` counterpart, and not encodable in
/// an instruction that also needs a REX prefix.
const HIGH_BYTES: [&str; 4] = ["%ah", "%bh", "%ch", "%dh"];

fn rename_in_function(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    start: usize,
    end: usize,
    lv: &mut Option<FileLiveness>,
    out: &mut ScratchRenames,
) {
    // Rule 1, and the census of explicit and implicit register uses.
    let mut named: u16 = 0;
    let mut implicit: u16 = 0;
    for k in start + 1..end {
        if infos[k].is_nop() {
            continue;
        }
        let t = infos[k].trimmed(store.get(k));
        match infos[k].kind {
            LineKind::Call | LineKind::JmpIndirect | LineKind::InlineAsm => return,
            LineKind::Jmp
                if !t
                    .split_whitespace()
                    .nth(1)
                    .is_some_and(|l| l.starts_with(".L")) =>
            {
                return;
            }
            _ => {}
        }
        if infos[k].pinned || t.starts_with("iret") || t.starts_with("sysret") {
            return;
        }
        named |= infos[k].reg_refs;
        implicit |= implicit_reg_refs(t.as_bytes());
    }

    // The prologue's callee-saved pushes, after an optional frame setup.
    let mut j = next_insn(store, infos, start + 1, end);
    if j < end && matches!(infos[j].kind, LineKind::Push { reg: REG_RBP }) {
        let n = next_insn(store, infos, j + 1, end);
        if n < end && infos[n].trimmed(store.get(n)) == "movq %rsp, %rbp" {
            j = next_insn(store, infos, n + 1, end);
        }
    }
    let mut saves: Vec<(RegId, usize)> = Vec::new();
    while j < end {
        match infos[j].kind {
            LineKind::Push { reg } if is_callee_saved_reg(reg) => saves.push((reg, j)),
            // A frame-pointer-omitted `%rbp` save: never renamed, but the
            // pushes after it are still prologue saves.
            LineKind::Push { reg: REG_RBP } => {}
            _ => break,
        }
        j = next_insn(store, infos, j + 1, end);
    }

    for (fam, save) in saves {
        let fam_bit = 1u16 << fam;
        if implicit & fam_bit != 0 {
            continue;
        }
        let Some(&scratch) = FREE_SCRATCH
            .iter()
            .find(|&&r| (named | implicit) & (1u16 << r) == 0)
        else {
            return;
        };
        // Rule 2: the one prologue push, epilogue pops, and body mentions.
        let mut body: Vec<usize> = Vec::new();
        let mut legal = true;
        for k in start + 1..end {
            if k == save || infos[k].is_nop() || infos[k].reg_refs & fam_bit == 0 {
                continue;
            }
            let t = infos[k].trimmed(store.get(k));
            match infos[k].kind {
                LineKind::Pop { reg } if reg == fam && fpo_epilogue_pop(store, infos, k, end) => {}
                LineKind::Push { .. } | LineKind::Pop { .. } => legal = false,
                LineKind::Directive if t.starts_with(".cfi_") => {}
                LineKind::Directive | LineKind::Label => legal = false,
                _ if HIGH_BYTES.iter().any(|h| t.contains(h)) => legal = false,
                _ => body.push(k),
            }
            if !legal {
                break;
            }
        }
        if !legal || body.is_empty() {
            continue;
        }
        let lv = lv.get_or_insert_with(|| FileLiveness::new(store, infos));
        if lv.live_after(save, fam) != Some(false) {
            continue;
        }
        let mut undo: Vec<(usize, String)> = Vec::with_capacity(body.len());
        for &k in &body {
            let old = store.get(k).to_string();
            let renamed = replace_reg_family(infos[k].trimmed(&old), fam, scratch);
            replace_line(store, &mut infos[k], k, format!("    {renamed}"));
            undo.push((k, old));
        }
        lv.refresh_at(store, infos, save);
        named |= 1u16 << scratch;
        out.renames.push((save, undo));
    }
}

#[cfg(test)]
mod scratch_rename_tests {
    use super::*;
    use crate::backend::peephole_common::LineStore;

    fn lines(asm: &str) -> (LineStore, Vec<LineInfo>) {
        let store = LineStore::new(asm.to_string());
        let infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        (store, infos)
    }

    /// The Phase 6 transaction as `peephole_optimize` runs it.
    fn phase6(asm: &str) -> (usize, String) {
        let (mut store, mut infos) = lines(asm);
        let renames = rename_saved_values_into_free_scratch(&mut store, &mut infos);
        eliminate_unused_callee_saves(&mut store, &mut infos);
        eliminate_unused_callee_saves_fpo(&mut store, &mut infos);
        let kept = renames.revert_unprofitable(&mut store, &mut infos);
        (kept, store.build_result(|i| infos[i].is_nop()))
    }

    /// Whether the rename step alone proposes anything.
    fn proposes(asm: &str) -> bool {
        let (mut store, mut infos) = lines(asm);
        !rename_saved_values_into_free_scratch(&mut store, &mut infos)
            .renames
            .is_empty()
    }

    /// `halve_add` after the address folds: `%r10`/`%r11` are free, the
    /// `sar` temporary sits in `%rbx` behind a save/restore pair.
    const HALVE_ADD: &str = concat!(
        "halve_add:\n",
        ".cfi_startproc\n",
        "    pushq %rbx\n",
        "    movslq %edx, %r8\n",
        "    xorl %r9d, %r9d\n",
        ".LBB6:\n",
        "    cmpq %r8, %r9\n",
        "    jge .LBB8\n",
        ".LBB7:\n",
        "    cvttsd2sil (%rsi,%r9,8), %edx\n",
        "    movl %edx, %ebx\n",
        "    sarl $1, %ebx\n",
        "    leal (%rdx,%rbx), %eax\n",
        "    movl %eax, (%rdi,%r9,4)\n",
        "    addq $1, %r9\n",
        "    cmpq %r8, %r9\n",
        "    jl .LBB7\n",
        ".LBB8:\n",
        "    popq %rbx\n",
        "    ret\n",
        ".cfi_endproc\n",
        ".size halve_add, .-halve_add\n",
    );

    #[test]
    fn a_saved_temporary_moves_into_a_free_scratch_register() {
        let (kept, out) = phase6(HALVE_ADD);
        assert_eq!(kept, 1, "{out}");
        for want in [
            "movl %edx, %r11d",
            "sarl $1, %r11d",
            "leal (%rdx,%r11), %eax",
        ] {
            assert!(out.contains(want), "{want}:\n{out}");
        }
        for gone in ["%rbx", "%ebx", "pushq", "popq"] {
            assert!(!out.contains(gone), "{gone}:\n{out}");
        }
    }

    /// The whole pipeline reaches the same result (the leaf GCC 16.2 and
    /// Clang 23.1.5 emit without any save).
    #[test]
    fn the_pipeline_drops_the_save_of_a_renamed_temporary() {
        let out = super::super::super::peephole_optimize(HALVE_ADD.to_string());
        assert!(!out.contains("pushq") && !out.contains("popq"), "{out}");
        assert!(!out.contains("%rbx") && !out.contains("%ebx"), "{out}");
        assert!(out.contains("sarl $1, %r11d"), "{out}");
    }

    /// Two saved families take the two scratch registers (`%r11` first);
    /// a dead third save goes as before.
    #[test]
    fn two_saved_families_take_both_scratch_registers() {
        let (kept, out) = phase6(concat!(
            "isort:\n",
            ".cfi_startproc\n",
            "    pushq %rbx\n",
            "    pushq %r12\n",
            "    pushq %r13\n",
            "    .cfi_def_cfa_offset 32\n",
            "    leaq 4(%rdi), %rbx\n",
            "    movl $1, %r12d\n",
            ".LBB1:\n",
            "    addl %r12d, %ebx\n",
            "    addl $1, %r12d\n",
            "    cmpl %esi, %r12d\n",
            "    jl .LBB1\n",
            "    movl %ebx, %eax\n",
            "    popq %r13\n",
            "    popq %r12\n",
            "    popq %rbx\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size isort, .-isort\n",
        ));
        assert_eq!(kept, 2, "{out}");
        assert!(out.contains("leaq 4(%rdi), %r11"), "{out}");
        assert!(out.contains("addl %r10d, %r11d"), "{out}");
        assert!(!out.contains("push") && !out.contains("pop"), "{out}");
        assert!(!out.contains(".cfi_def_cfa_offset 32"), "{out}");
    }

    /// Frame-pointer shape: the `%rbx` pair goes, the frame stays.
    #[test]
    fn frame_pointer_functions_rename_too() {
        let (kept, out) = phase6(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    pushq %rbp\n",
            "    movq %rsp, %rbp\n",
            "    pushq %rbx\n",
            "    movl %edi, %ebx\n",
            "    imull %ebx, %ebx\n",
            "    movl %ebx, %eax\n",
            "    popq %rbx\n",
            "    popq %rbp\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        ));
        assert_eq!(kept, 1, "{out}");
        assert!(out.contains("imull %r11d, %r11d"), "{out}");
        assert!(!out.contains("%rbx") && !out.contains("%ebx"), "{out}");
        assert!(
            out.contains("pushq %rbp") && out.contains("popq %rbp"),
            "{out}"
        );
    }

    /// A pushes-only frame with a slot below the saves cannot be
    /// re-anchored, so Phase 6 keeps the pair and the rename is reverted
    /// byte for byte.
    #[test]
    fn a_rename_whose_save_survives_is_reverted() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    pushq %rbp\n",
            "    movq %rsp, %rbp\n",
            "    pushq %rbx\n",
            "    movl %edi, %ebx\n",
            "    movl %ebx, -12(%rbp)\n",
            "    movl -12(%rbp), %eax\n",
            "    popq %rbx\n",
            "    popq %rbp\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        );
        assert!(proposes(asm), "the rename itself is legal here");
        let (kept, out) = phase6(asm);
        assert_eq!(kept, 0, "{out}");
        assert_eq!(out, asm);
    }

    /// One shape per legality rule; none may be renamed.
    #[test]
    fn every_legality_rule_blocks_the_rename() {
        let wrap = |body: &str| {
            format!(
                "f:\n.cfi_startproc\n    pushq %rbx\n{body}    popq %rbx\n    ret\n.cfi_endproc\n.size f, .-f\n"
            )
        };
        for (body, why) in [
            (
                "    movl %edi, %ebx\n    call g\n    movl %ebx, %eax\n",
                "a call clobbers the scratch registers",
            ),
            (
                "    movl %edi, %ebx\n    movl %ebx, %eax\n    popq %rbx\n    jmp g\n",
                "a tail jump forwards registers to its callee",
            ),
            (
                "    movl %edi, %ebx\n    movl %esi, %r10d\n    movl %edx, %r11d\n    addl %r10d, %ebx\n    addl %r11d, %ebx\n    movl %ebx, %eax\n",
                "both scratch registers are named",
            ),
            (
                "    movl %edi, %ebx\n    movb %bh, (%rsi)\n",
                "%bh has no %r11 spelling",
            ),
            (
                "    movl %edi, %ebx\n    movb %ah, (%rsi,%rbx)\n",
                "a high-byte operand cannot sit beside a REX register",
            ),
            (
                "    movq %rbx, %rax\n    movl %edi, %ebx\n    addq %rbx, %rax\n",
                "the caller's %rbx is read",
            ),
            (
                "    movl %edi, %ebx\n    cpuid\n    movl %ebx, %eax\n",
                "cpuid writes %rbx implicitly",
            ),
            (
                "    movl %edi, %ebx\n    pushq %rbx\n    popq %rbx\n    movl %ebx, %eax\n",
                "a body push/pop of the saved family",
            ),
            (
                "    movl %edi, %ebx\n    syscall\n    movl %ebx, %eax\n",
                "syscall clobbers %r11 (and %r10 is an argument)",
            ),
        ] {
            assert!(!proposes(&wrap(body)), "{why}:\n{}", wrap(body));
        }
        // Control: the same frame with a plain body is renamed.
        assert!(proposes(&wrap(
            "    movl %edi, %ebx\n    movl %ebx, %eax\n"
        )));
    }
}
