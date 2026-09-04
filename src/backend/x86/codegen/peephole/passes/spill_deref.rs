//! ms178: spill-deref round-trip elimination.
//!
//! Targets the dominant hot-loop pattern in spill-heavy code (gzip's
//! `longest_match`, zlib `deflate_*`):
//!
//! ```text
//!     movq %rax, N(%rsp)      ; store a pointer to its spill slot
//!     movq N(%rsp), %rX       ; reload it (rax still holds the pointer)
//!     movzbl/movq/movl (%rX), %rY   ; dereference it
//! ```
//!
//! Because the pointer is still live in %rax, the store + reload are pure
//! waste. We rewrite the sequence to a single dereference through %rax:
//!
//! ```text
//!     movzbl/movq/movl (%rax), %rY
//! ```
//!
//! ## Soundness requirements (all must hold)
//!
//! 1. Between the store and the deref, %rax is never written (its pointer value
//!    must be intact).
//! 2. Between the store and the load, the slot N is not accessed by any other
//!    instruction (no store, no load).
//! 3. Between the load and the deref, the reloaded register %rX is not written,
//!    and the slot N is not accessed.
//! 4. After the deref, the slot N is not READ again before it is next WRITTEN
//!    (otherwise deleting the store leaves garbage for that later read). If no
//!    write to N is found before a barrier (label/jump/call/ret), we abort.
//! 5. No barriers anywhere in the window (straight-line code only). A function
//!    call could clobber %rax (%rY if caller-saved too — but we only rewrite
//!    the deref's dest, which is produced by the deref itself, so dest is fine;
//!    the concern is only the pointer in %rax and slot N contents).
//! 6. The deref must be a plain load (movzbl/movzwl/movb/movw/movl/movq) with
//!    the reloaded register as base and NO index/scale — we keep the rewrite
//!    conservative. Sign-extending loads (movslq/movsbq) and float loads are
//!    left alone.
//! 7. The store must be a 64-bit store of %rax (pointers are 64-bit). The load
//!    must be a 64-bit load of a GP register.
//!
//! This pass runs in Phase 2 (global passes), after local pattern passes. It is
//! deliberately conservative: any doubt → skip.

use super::super::types::*;
use super::helpers::*;

/// Frame-pointer status at line `idx`: true if the enclosing function has
/// executed `movq %rsp, %rbp` since its `.cfi_startproc`. Under
/// -fomit-frame-pointer, %rbp is a data register and `N(%rbp)` is a pointer
/// dereference; deleting such a store removes an observable memory write.
fn rbp_is_frame_at(store: &LineStore, infos: &[LineInfo], idx: usize) -> bool {
    let mut j = idx;
    loop {
        let trimmed = infos[j].trimmed(store.get(j));
        if trimmed.starts_with(".cfi_startproc") {
            return false;
        }
        if trimmed == "movq %rsp, %rbp" || trimmed == "movl %esp, %ebp" {
            return true;
        }
        if j == 0 {
            return false;
        }
        j -= 1;
    }
}

/// Which stack base a store/load line addresses: 4 = "(%rsp)", 5 = "(%rbp)".
fn line_stack_base(trimmed: &str) -> Option<u8> {
    if trimmed.contains("(%rsp)") {
        Some(4)
    } else if trimmed.contains("(%rbp)") {
        Some(5)
    } else {
        None
    }
}

pub(super) fn fold_spill_deref_roundtrip(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0usize;
    if std::env::var("CCC_DEBUG_SPILL_DEREF_ALWAYS").is_ok() {
        eprintln!("[SPILL_DEREF] pass entered, lines={}", len);
    }
    let dbg = std::env::var("CCC_DEBUG_SPILL_DEREF").is_ok();
    let mut dbg_stats = [0usize; 8];
    // 0=stores seen, 1=adjacent load found, 2=intervening dirty, 3=no deref,
    // 4=deref wrong base, 5=after unsafe, 6=fired, 7=parse fail

    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Step 1: find `movq %rax, N(%rsp)` (StoreRbp, reg 0 = rax family, Q size).
        if let LineKind::StoreRbp {
            reg: 0,
            offset,
            size: MoveSize::Q,
        } = infos[i].kind
        {
            dbg_stats[0] += 1;
            let n = offset;
            // Step 2: find the next non-nop line: must be `movq N(%rsp), %rX` (LoadRbp).
            let mut j = i + 1;
            while j < len && infos[j].is_nop() {
                j += 1;
            }
            if j >= len {
                i += 1;
                continue;
            }
            let load_reg = match infos[j].kind {
                LineKind::LoadRbp {
                    reg,
                    offset: lo,
                    size: MoveSize::Q,
                } if lo == n && reg != REG_NONE && reg != 0 && reg <= REG_GP_MAX => reg,
                _ => {
                    dbg_stats[7] += 1;
                    i += 1;
                    continue;
                }
            };
            dbg_stats[1] += 1;

            // SOUNDNESS: with (%rbp) addressing where %rbp is a DATA register
            // (-fomit-frame-pointer), the store/load are pointer dereferences,
            // not stack-slot traffic; deleting the store would drop an
            // observable write. Only (%rsp) pairs, or (%rbp) pairs inside a
            // frame-pointer function, are genuine spill round-trips.
            {
                let st_line = infos[i].trimmed(store.get(i));
                let ld_line = infos[j].trimmed(store.get(j));
                match (line_stack_base(st_line), line_stack_base(ld_line)) {
                    (Some(4), Some(4)) => {}
                    (Some(5), Some(5)) if rbp_is_frame_at(store, infos, i) => {}
                    _ => {
                        dbg_stats[7] += 1;
                        i += 1;
                        continue;
                    }
                }
            }

            // Step 3: between store and load, nothing may touch %rax, slot N,
            // any indirect memory (could alias slot N), or any barrier.
            let mut clean = true;
            for k in (i + 1)..j {
                if infos[k].is_nop() {
                    continue;
                }
                if infos[k].kind == LineKind::Empty {
                    continue;
                }
                if is_barrier_kind(infos[k].kind) {
                    clean = false;
                    break;
                }
                // %rax written?
                if writes_reg(store, infos, k, 0) {
                    clean = false;
                    break;
                }
                // slot N accessed (store or load)?
                if accesses_slot(store, infos, k, n) {
                    clean = false;
                    break;
                }
                // indirect memory op could alias slot N
                if matches!(infos[k].kind, LineKind::Other { .. }) && infos[k].has_indirect_mem {
                    clean = false;
                    break;
                }
            }
            if !clean {
                dbg_stats[2] += 1;
                i += 1;
                continue;
            }

            // Step 4: next non-nop after the load must be a plain load deref
            // through %load_reg, e.g. `movzbl (%rcx), %eax`.
            let mut k = j + 1;
            while k < len && infos[k].is_nop() {
                k += 1;
            }
            if k >= len {
                dbg_stats[3] += 1;
                i += 1;
                continue;
            }
            if is_barrier_kind(infos[k].kind) {
                dbg_stats[3] += 1;
                i += 1;
                continue;
            }
            // Between load and deref: no write to load_reg, no write to %rax
            // (the rewrite dereferences through %rax), no slot access, no
            // indirect memory, no barrier.
            for m in (j + 1)..k {
                if infos[m].is_nop() || infos[m].kind == LineKind::Empty {
                    continue;
                }
                if is_barrier_kind(infos[m].kind) {
                    clean = false;
                    break;
                }
                if writes_reg(store, infos, m, load_reg) {
                    clean = false;
                    break;
                }
                if writes_reg(store, infos, m, 0) {
                    clean = false;
                    break;
                }
                if accesses_slot(store, infos, m, n) {
                    clean = false;
                    break;
                }
                if matches!(infos[m].kind, LineKind::Other { .. }) && infos[m].has_indirect_mem {
                    clean = false;
                    break;
                }
            }
            if !clean {
                dbg_stats[2] += 1;
                i += 1;
                continue;
            }

            let deref_line = infos[k].trimmed(store.get(k));
            let Some((mnem, _base_reg, dest_text)) = parse_plain_deref(&deref_line, load_reg)
            else {
                dbg_stats[4] += 1;
                i += 1;
                continue;
            };
            dbg_stats[3] += 1;

            // Step 5: after the deref, the deleted store's slot N and the deleted
            // load's register %load_reg must both be WRITTEN before the first
            // barrier (label / conditional jump / call / jmp / ret), with no
            // reads of either in between.
            //
            // SOUNDNESS: a CondJmp's TAKEN path (or a jump to a later label)
            // re-enters the code after our window and could read slot N or
            // %load_reg — both now hold deleted values. We cannot see those
            // paths, so the only safe exit is: both are provably re-written
            // (or the function ends) before ANY barrier. `ret` and
            // end-of-text are safe terminal points (nothing executes after).
            let mut n_written = false;
            let mut lr_written = false;
            let mut safe_after = false;
            let mut m = k + 1;
            while m < len {
                if infos[m].is_nop() || infos[m].kind == LineKind::Empty {
                    m += 1;
                    continue;
                }
                // Ret/end: nothing after executes → safe regardless of writes.
                if infos[m].kind == LineKind::Ret {
                    safe_after = true;
                    break;
                }
                // Any other barrier before both writes → can't see the other
                // paths → unsafe.
                if is_barrier_kind(infos[m].kind) {
                    break;
                }
                // Slot N read before written → unsafe.
                if matches!(infos[m].kind, LineKind::LoadRbp { offset: lo, .. } if lo == n) {
                    break;
                }
                // Slot N written → requirement (a) satisfied.
                if matches!(infos[m].kind, LineKind::StoreRbp { offset: so, .. } if so == n) {
                    n_written = true;
                }
                // load_reg read before written → unsafe.
                if reads_reg(store, infos, m, load_reg) {
                    break;
                }
                // load_reg written → requirement (b) satisfied.
                if writes_reg(store, infos, m, load_reg) {
                    lr_written = true;
                }
                // A generic instruction with indirect memory could alias slot N
                // or read load_reg indirectly. Conservative: treat as unsafe.
                if matches!(infos[m].kind, LineKind::Other { .. }) {
                    let t = infos[m].trimmed(store.get(m));
                    if t.contains("(%rsp)") {
                        break;
                    }
                    if infos[m].has_indirect_mem {
                        break;
                    }
                }
                if n_written && lr_written {
                    safe_after = true;
                    break;
                }
                m += 1;
            }
            if !safe_after {
                dbg_stats[5] += 1;
                i += 1;
                continue;
            }

            // All checks pass. Rewrite:
            //   drop store (i), drop load (j)
            //   deref (k): replace base %load_reg with %rax
            let new_deref = format!("    {} ({}), {}", mnem, "%rax", dest_text);
            mark_nop(&mut infos[i]);
            mark_nop(&mut infos[j]);
            replace_line(store, &mut infos[k], k, new_deref);
            changed = true;
            dbg_stats[6] += 1;
            // continue scanning after the deref
            i = k + 1;
            continue;
        }

        i += 1;
    }

    if dbg {
        eprintln!("[SPILL_DEREF] stores={} adj_load={} dirty_between={} no_deref={} wrong_base={} after_unsafe={} FIRED={} parse_fail={}",
            dbg_stats[0], dbg_stats[1], dbg_stats[2], dbg_stats[3], dbg_stats[4], dbg_stats[5], dbg_stats[6], dbg_stats[7]);
    }

    changed
}

/// Is this line kind a control-flow / barrier line?
fn is_barrier_kind(kind: LineKind) -> bool {
    matches!(
        kind,
        LineKind::Label
            | LineKind::Jmp
            | LineKind::JmpIndirect
            | LineKind::CondJmp
            | LineKind::Call
            | LineKind::Ret
            | LineKind::Directive
            | LineKind::Push { .. }
            | LineKind::Pop { .. }
            | LineKind::InlineAsm
    )
}

/// Does the instruction at index `idx` write the GP register family `reg`?
fn writes_reg(store: &LineStore, infos: &[LineInfo], idx: usize, reg: u8) -> bool {
    match infos[idx].kind {
        // FP slot moves write no GP register (the XMM domain is invisible
        // to the GP spill machinery).
        LineKind::StoreRbp { .. } | LineKind::StoreXmmRbp { .. } | LineKind::LoadXmmRbp { .. } => {
            false
        }
        LineKind::LoadRbp { reg: r, .. } => r == reg,
        LineKind::Pop { reg: r } => r == reg,
        LineKind::SetCC { reg: r } => r == reg,
        LineKind::Push { .. }
        | LineKind::Cmp
        | LineKind::Label
        | LineKind::Jmp
        | LineKind::JmpIndirect
        | LineKind::CondJmp
        | LineKind::Call
        | LineKind::Ret
        | LineKind::Directive
        | LineKind::Empty
        | LineKind::Nop => false,
        LineKind::SelfMove => false,
        // Inline asm is opaque: assume it writes every register.
        LineKind::InlineAsm => true,
        LineKind::Other { dest_reg } => dest_reg == reg,
    }
}

/// Does the instruction at index `idx` read or write stack slot `offset`?
fn accesses_slot(store: &LineStore, infos: &[LineInfo], idx: usize, offset: i32) -> bool {
    match infos[idx].kind {
        LineKind::StoreRbp { offset: o, .. }
        | LineKind::LoadRbp { offset: o, .. }
        | LineKind::StoreXmmRbp { offset: o, .. }
        | LineKind::LoadXmmRbp { offset: o, .. } => o == offset,
        _ => false,
    }
}

/// Does the instruction at index `idx` read the GP register family `reg`?
/// Conservative: a line "mentions" the register in any position and does not
/// write it → counts as a read (a write would show up in writes_reg first).
fn reads_reg(store: &LineStore, infos: &[LineInfo], idx: usize, reg: u8) -> bool {
    if writes_reg(store, infos, idx, reg) {
        return false; // pure write (or write+read) — treated as write, not read
    }
    // Only text lines can mention a register.
    if infos[idx].kind == LineKind::Empty || infos[idx].kind == LineKind::Nop {
        return false;
    }
    let t = infos[idx].trimmed(store.get(idx));
    let name = reg_id_to_name(reg, MoveSize::Q);
    t.contains(name)
}

/// Parse a plain (non-extending, non-float) load dereference:
/// `movzbl (%rcx), %eax` / `movl (%rcx), %eax` / `movq (%rcx), %rax` …
/// Returns (mnemonic, base_reg_family, dest_text) or None.
/// base_reg must equal `expected_base`.
fn parse_plain_deref(line: &str, expected_base: u8) -> Option<(&'static str, u8, String)> {
    let t = line.trim();
    for mnem in [
        "movzbl", "movzwl", "movb", "movw", "movl", "movq", "movzbq", "movzwq",
    ] {
        if let Some(rest) = t.strip_prefix(mnem) {
            let rest = rest.trim_start();
            // format: (%reg), %dest
            if let Some(mem) = rest.strip_prefix('(') {
                let paren_end = mem.find(')')?;
                let base = &mem[..paren_end];
                if base.contains('%') && !base.contains(',') && !base.contains('+') {
                    let base_fam = register_family_fast(base);
                    if base_fam != expected_base {
                        return None;
                    }
                    let after = mem[paren_end + 1..].trim_start();
                    let after = after.strip_prefix(',')?.trim_start();
                    if !after.starts_with('%') {
                        return None;
                    }
                    // dest must be a GP register
                    let dest_fam = register_family_fast(after);
                    if dest_fam == REG_NONE || dest_fam > REG_GP_MAX {
                        return None;
                    }
                    // dest cannot be rsp (never emitted anyway)
                    return Some((mnem, base_fam, after.to_string()));
                }
            }
            return None;
        }
    }
    None
}
