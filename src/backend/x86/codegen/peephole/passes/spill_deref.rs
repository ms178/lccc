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

// ── Save/reload round-trip elimination (XMM/vector / store→clobber→reload) ─
//
// Pattern (the double_reduction class):
//      vmovdqu (%r15,%rsi), %ymm0        ; L0: pure memory load  REG <- MEM
//      vmovdqu %ymm0, 232(%rsp)          ; L1: spill REG to a frame slot
//      vpmulld (%rbp,%rsi), %ymm0, %ymm0 ; REG is clobbered in place...
//      vpaddd  %ymm0, %ymm6, %ymm6
//      vmovdqu 232(%rsp), %ymm0          ; L2: reload — REG must regain the
//                                        ; value it had at L1
// The slot holds REG's value at L1; nothing else touches it. The reload is
// therefore exactly "re-execute the L0 load": rewrite L2's memory operand to
// MEM and delete L1. The clobbered register value is re-obtained from memory
// instead of the frame slot (which frees the slot entirely).
//
// Soundness gates (all verified):
//  * L0 is the IMMEDIATELY preceding instruction of L1 (REG unchanged between
//    load and store) and L0 is a pure 2-operand load of the SAME mnemonic into
//    the SAME register. The clobbering of REG between L1 and L2 is allowed —
//    that is precisely why the reload exists — so the intermediate use of REG
//    is irrelevant; only the memory content matters.
//  * straight-line window: no label/jump/call/ret/barrier between L0 and L2,
//    no rsp shift, no pinned line, no implicit-register op (div/mul/string
//    ops/rep/xchg/…) that could hide memory side effects.
//  * no frame-escape in the function (a leaked frame address could let the
//    window's data-register derefs alias the slot or MEM).
//  * no memory WRITE of ANY kind between L0 and L2 (explicit stores, push/
//    pop, memory-dest instructions incl. indexed forms) — MEM's content is
//    then unchanged, and the value the slot held is the value MEM had at L0.
//  * the slot is not accessed between L1 and L2 (any rsp/rbp-anchored
//    reference bails, indexed forms included).
//  * MEM's address registers are not written between L0 and L2.
//  * after L2 the slot is never READ again (until a later exact re-write)
//    before the function end / any rsp shift. Same-base, different-offset
//    direct accesses cannot alias the slot (frame functions additionally
//    treat any opposite-base access as ambiguous), indexed rsp/rbp forms
//    are ambiguous and bail, and a leaked frame address bails the function.
//  * neither L0/L1/L2 is pinned (volatile slot / address-taken slot / param).
//  * the rewrite re-loads the EXACT memory operand text of L0.
//
// The gate is text-based because `vmovdqu`/`vmovdqa` slot moves are classified
// `LineKind::Other` (only scalar-FP movsd/movss/vmovsd/vmovss get the XMM
// slot kinds); line text is the ground truth for the memory operands.

/// Frame-escape scan: has any instruction leaked the frame pointer or a frame
/// slot address (leaq of a frame slot, or a raw %rsp/%esp value) — in which
/// case data-register derefs may alias frame slots and MEM? Mirrors the
/// discipline of the address-taken-slot pinning and dead-store analysis.
fn frame_has_escape(store: &LineStore, infos: &[LineInfo], body_start: usize, body_end: usize) -> bool {
    for k in body_start..body_end.min(store.len()) {
        if infos[k].is_nop() || infos[k].kind == LineKind::Empty {
            continue;
        }
        let t = infos[k].trimmed(store.get(k));
        if (t.starts_with("leaq ") || t.starts_with("lea "))
            && (t.contains("(%rsp)") || t.contains("(%rbp)") || t.contains("(%rsp,") || t.contains("(%rbp,"))
        {
            return true;
        }
        let mentions_sp = t.contains("%rsp") || t.contains("%esp");
        if mentions_sp && !t.contains("(%rsp") && !t.contains("(%esp") && !is_rsp_shift_line(&t)
            && t != "movq %rsp, %rbp"
            && t != "movl %esp, %ebp"
        {
            return true;
        }
    }
    false
}

/// True when the line's top-level DESTINATION operand refers to memory.
/// A register operand never contains '(', so `contains('(')` on the dest
/// operand catches both `(%rax)` and offset forms `8(%rax,%rcx)` (the latter
/// defeat a naive starts_with('(') check).
fn mem_is_dest(line: &str) -> bool {
    let bytes = line.as_bytes();
    match last_top_level_comma(bytes) {
        Some(cpos) => line[cpos + 1..].contains('('),
        None => false,
    }
}

/// Is the RSP-shift at `k` part of an epilogue chain? Mirrors the shape
/// accepted by dead_code::eliminate_never_read_stores: zero or more of
/// { `addq $N,%rsp`, `popq` (callee-saved restore), nop, directive } ending
/// in `ret` or an unconditional `jmp` (tail call). `pushq`/`subq` in the
/// window, or any other instruction, rejects — those are genuine mid-body
/// shifts. After the tail nothing can read the frame slot.
fn is_epilogue_tail(store: &LineStore, infos: &[LineInfo], k: usize, func_end: usize) -> bool {
    let mut j = k;
    let limit = (k + 24).min(func_end);
    while j < limit {
        if infos[j].is_nop() || infos[j].kind == LineKind::Empty || matches!(infos[j].kind, LineKind::Directive) {
            j += 1;
            continue;
        }
        match infos[j].kind {
            LineKind::Ret | LineKind::Jmp => return true,
            LineKind::Pop { .. } => {
                j += 1;
                continue;
            }
            _ => {
                let t = infos[j].trimmed(store.get(j));
                if let Some(rest) = t.strip_prefix("addq $") {
                    if rest
                        .strip_suffix(", %rsp")
                        .and_then(|v| v.parse::<i64>().ok())
                        .is_some()
                    {
                        j += 1;
                        continue;
                    }
                }
                return false;
            }
        }
    }
    false
}

/// True for string/implicit-operand instructions whose memory side effects
/// are invisible to operand parsing (bare `movsb`/`stosq`/…; `rep`-prefixed
/// forms are already covered by has_implicit_reg_usage).
fn is_string_op(line: &str) -> bool {
    const PREFIXES: [&str; 8] = ["movs", "stos", "lods", "scas", "cmps", "ins", "outs", "rep "];
    PREFIXES.iter().any(|p| line.starts_with(p))
}

/// Index of the FIRST top-level comma (parenthesis depth 0) in `bytes`.
fn first_top_level_comma(bytes: &[u8]) -> Option<usize> {
    let mut depth = 0i32;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b',' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// Parse `MNEM %reg, off(%rsp|%rbp)` (2-operand direct slot store) →
/// (mnemonic, source register text, memory operand).
fn parse_2op_store(line: &str) -> Option<(&str, &str, &str)> {
    let mnem_end = line.find(' ')?;
    let mnem = &line[..mnem_end];
    let rest = line[mnem_end..].trim_start();
    let comma = first_top_level_comma(rest.as_bytes())?;
    let src = rest[..comma].trim();
    let mem = rest[comma + 1..].trim();
    if !src.starts_with('%') || !(mem.ends_with("(%rsp)") || mem.ends_with("(%rbp)")) {
        return None;
    }
    Some((mnem, src, mem))
}

/// Parse `MNEM mem, %reg` (2-operand load) → (mnemonic, mem, dest register).
/// Accepts both `(%r15,%rsi)`-style and `232(%rsp)`-style memory operands.
fn parse_2op_load(line: &str) -> Option<(&str, &str, &str)> {
    let mnem_end = line.find(' ')?;
    let mnem = &line[..mnem_end];
    let rest = line[mnem_end..].trim_start();
    let comma = first_top_level_comma(rest.as_bytes())?;
    let mem = rest[..comma].trim();
    let dst = rest[comma + 1..].trim();
    let ok_mem = mem.starts_with('(') || mem.ends_with("(%rsp)") || mem.ends_with("(%rbp)");
    if !ok_mem || !dst.starts_with('%') {
        return None;
    }
    Some((mnem, mem, dst))
}

/// Parse `off(%rsp)` or `off(%rbp)` → (offset, base) with base 4=rsp 5=rbp.
fn parse_slot(mem: &str) -> Option<(i32, u8)> {
    if let Some(rest) = mem.strip_suffix("(%rsp)") {
        return rest.trim().parse::<i32>().ok().map(|n| (n, 4));
    }
    if let Some(rest) = mem.strip_suffix("(%rbp)") {
        return rest.trim().parse::<i32>().ok().map(|n| (n, 5));
    }
    None
}

/// If `line` references a direct slot `off(%rsp|%rbp)`, return (off, base).
/// Handles both `%reg, off(%rsp)` store form and `off(%rsp), %reg` load form.
fn slot_in_line(line: &str) -> Option<(i32, u8)> {
    if let Some((_m, _s, mem)) = parse_2op_store(line) {
        return parse_slot(mem);
    }
    if let Some((_m, mem, _d)) = parse_2op_load(line) {
        if let Some((n, b)) = parse_slot(mem) {
            return Some((n, b));
        }
    }
    None
}

/// Byte offsets of the `%` register tokens inside a memory operand.
fn reg_token_offsets(mem: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let bytes = mem.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            out.push(i);
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric()) {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    out
}

pub(super) fn fold_save_reload_roundtrip(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    if len == 0 {
        return false;
    }
    let dbg = std::env::var("CCC_DEBUG_SAVRELOAD").is_ok();
    let mut dbg_stats = [0usize; 6]; // 0=store cands, 1=L0 bad, 2=window bad, 3=no L2, 4=slot live, 5=FIRED
    let mut changed = false;

    // Function ranges: `.cfi_startproc` … `.size`.
    let mut funcs: Vec<(usize, usize)> = Vec::new();
    let mut start: Option<usize> = None;
    for k in 0..len {
        if infos[k].is_nop() {
            continue;
        }
        let t = infos[k].trimmed(store.get(k));
        if t.starts_with(".cfi_startproc") && start.is_none() {
            start = Some(k);
        } else if t.starts_with(".size ") {
            if let Some(s) = start.take() {
                funcs.push((s, k));
            }
        }
    }

    for &(fstart, fend) in &funcs {
        if frame_has_escape(store, infos, fstart, fend) {
            continue;
        }
        // Frame-pointer function? Forward scan: frame setup before the first
        // ret/.cfi_endproc.
        let mut rbp_frame = false;
        for k in (fstart + 1)..fend.min(len) {
            if infos[k].is_nop() || infos[k].kind == LineKind::Empty {
                continue;
            }
            let t = infos[k].trimmed(store.get(k));
            if t == "movq %rsp, %rbp" || t == "movl %esp, %ebp" {
                rbp_frame = true;
                break;
            }
            if t.starts_with(".cfi_endproc") || t.starts_with(".size ")
                || infos[k].kind == LineKind::Ret
            {
                break;
            }
        }
        let mut i = fstart;
        while i < fend.min(len) {
            if infos[i].is_nop() || infos[i].pinned {
                i += 1;
                continue;
            }
            // Candidate L1: a direct slot store of a register (text-based —
            // vmovdqu/vmovdqa slot moves are LineKind::Other).
            let line_st = infos[i].trimmed(store.get(i));
            let Some((store_mnem, src_reg, mem_str)) = parse_2op_store(&line_st) else {
                i += 1;
                continue;
            };
            let Some((n, base)) = parse_slot(mem_str) else {
                i += 1;
                continue;
            };
            if base == 5 && !rbp_frame {
                i += 1;
                continue;
            }
            dbg_stats[0] += 1;
            // L0: immediately preceding non-nop line must be the pure load.
            let mut l0 = i - 1;
            while l0 > fstart && infos[l0].is_nop() {
                l0 -= 1;
            }
            if l0 < fstart || infos[l0].pinned {
                dbg_stats[1] += 1;
                i += 1;
                continue;
            }
            let l0_st = infos[l0].trimmed(store.get(l0));
            let Some((l0_mnem, l0_mem2, l0_reg2)) = parse_2op_load(&l0_st) else {
                dbg_stats[1] += 1;
                i += 1;
                continue;
            };
            if l0_mnem != store_mnem || l0_reg2 != src_reg {
                dbg_stats[1] += 1;
                i += 1;
                continue;
            }
            // Address families of MEM2 (the re-load target).
            let mut addr_fams = [false; 16];
            for off in reg_token_offsets(l0_mem2) {
                let fam = register_family_fast(&l0_mem2[off..]);
                if fam != REG_NONE && fam <= REG_GP_MAX {
                    addr_fams[fam as usize] = true;
                }
            }
            // Window scan: L1+1 … until L2 or a bail condition.
            let mut ok = true;
            let mut l2 = None;
            let mut j = i + 1;
            while j < fend.min(len) {
                if infos[j].is_nop() {
                    j += 1;
                    continue;
                }
                let t = infos[j].trimmed(store.get(j));
                // Reached L2 (same mnemonic, same register, same slot)? This
                // must be tested FIRST: the L2 line itself references the
                // slot, and it is a pure load, exempt from the window bails.
                if let Some((m2, m2_mem, m2_reg)) = parse_2op_load(&t) {
                    if m2 == store_mnem && m2_reg == src_reg
                        && parse_slot(m2_mem).map_or(false, |(n2, b2)| n2 == n && b2 == base)
                    {
                        l2 = Some(j);
                        break;
                    }
                }
                if is_barrier_kind(infos[j].kind) || infos[j].pinned || is_string_op(&t) {
                    break; // cannot see past the barrier / opaque op
                }
                if has_implicit_reg_usage(&t) || is_rsp_shift_line(&t) {
                    ok = false;
                    break;
                }
                // Memory writes — of ANY kind — invalidate MEM content.
                if matches!(
                    infos[j].kind,
                    LineKind::StoreRbp { .. }
                        | LineKind::StoreXmmRbp { .. }
                        | LineKind::Push { .. }
                        | LineKind::Pop { .. }
                ) || mem_is_dest(&t)
                {
                    ok = false;
                    break;
                }
                // Any slot/frame-anchored access between L1 and L2 could
                // touch the slot (incl. indexed forms and the opposite frame
                // base, whose offsets alias through the frame size) — bail.
                if t.contains("(%rsp") || (rbp_frame && t.contains("(%rbp")) {
                    ok = false;
                    break;
                }
                // MEM2's address registers must survive.
                for k in 0..16u8 {
                    if addr_fams[k as usize] && writes_reg(store, infos, j, k) {
                        ok = false;
                        break;
                    }
                }
                if !ok {
                    break;
                }
                j += 1;
            }
            let Some(l2) = l2 else {
                dbg_stats[3] += 1;
                i += 1;
                continue;
            };
            if !ok || infos[l2].pinned {
                dbg_stats[2] += 1;
                i += 1;
                continue;
            }
            // After L2 the slot must never be read again (before a later
            // exact re-write) up to the function end; an rsp shift invalidates
            // offsets entirely.
            let mut post = l2 + 1;
            let mut redefined = false;
            let mut slot_dead = true;
            while post < fend.min(len) {
                if infos[post].is_nop() {
                    post += 1;
                    continue;
                }
                let t = infos[post].trimmed(store.get(post));
                if is_rsp_shift_line(&t) {
                    // An epilogue chain (pops/addq → ret/jmp) is a terminal:
                    // nothing can read the frame slot afterwards.
                    if is_epilogue_tail(store, infos, post, fend) {
                        break;
                    }
                    slot_dead = false;
                    break;
                }
                let anchored =
                    t.contains("(%rsp") || (rbp_frame && t.contains("(%rbp"));
                if !anchored {
                    post += 1;
                    continue;
                }
                // Indexed rsp/rbp forms: ambiguous slot → bail.
                if t.contains("(%rsp,") || t.contains("(%rbp,") {
                    slot_dead = false;
                    break;
                }
                // Opposite base in a frame function: offsets alias through
                // the frame size → ambiguous → bail.
                if let Some((m, b)) = slot_in_line(t) {
                    if b != base && rbp_frame {
                        slot_dead = false;
                        break;
                    }
                    if m == n {
                        if let Some((_mn, _sr, _mem)) = parse_2op_store(&t) {
                            redefined = true;
                        } else {
                            slot_dead = false; // read of the deleted value
                            break;
                        }
                    }
                    // Same-base different offset: different slot, ignore.
                } else {
                    slot_dead = false;
                    break;
                }
                post += 1;
            }
            if !slot_dead {
                dbg_stats[4] += 1;
                i += 1;
                continue;
            }
            // Rewrite: delete L1, re-point L2's memory operand at MEM2.
            let new_l2 = format!("    {} {}, {}", store_mnem, l0_mem2, src_reg);
            replace_line(store, &mut infos[l2], l2, new_l2);
            mark_nop(&mut infos[i]);
            dbg_stats[5] += 1;
            changed = true;
            i = l2 + 1;
        }
    }

    if dbg {
        eprintln!(
            "[SAVRELOAD] stores={} l0_bad={} window_bad={} no_l2={} slot_live={} FIRED={}",
            dbg_stats[0], dbg_stats[1], dbg_stats[2], dbg_stats[3], dbg_stats[4], dbg_stats[5]
        );
    }
    changed
}
