//! Register-copy folding: remove a `mov reg, reg` by rewriting its consumers.
//!
//! A copy survives to final assembly only when neither copy propagation nor
//! dead-write elimination could retire it. A census of the 559-test corpus at
//! `-O2` found **11 055** such copies still in the emitted code:
//!
//! | form | count |
//! |---|---|
//! | `movq` cross-family | 7 943 |
//! | `movl` cross-family | 2 463 |
//! | `movl %X, %X` self-move | 261 |
//! | `movq %X, %X` self-move | 3 |
//! | `movw` | 1 |
//!
//! Two independent transforms handle them.
//!
//! # A. Self-move elimination
//!
//! `movq %rax, %rax` is a pure no-op and three of them reached final assembly.
//! `movb`/`movw` self-moves are no-ops too: they write the same bits back and
//! leave everything above untouched.
//!
//! `movl %ebx, %ebx` is **not** a no-op — it zero-extends into `%rbx` — so it
//! is removed only when the upper 32 bits are already provably zero. That fact
//! is exactly what [`super::redundant_ext`] computes, and it is recomputed here
//! with the same rules rather than shared, because the two passes run at
//! different points and the fact does not survive between them.
//!
//! # B. Copy folding
//!
//! For `mov<w> %S, %D`, rewrite every later use of `%D` to `%S` and delete the
//! copy. The width of the copy bounds which uses may be rewritten, and getting
//! this wrong is silent miscompilation:
//!
//! | copy | what `%D` shares with `%S` | rewritable uses of `%D` |
//! |---|---|---|
//! | `movq` | all 64 bits | any width, **including address operands** |
//! | `movl` | low 32; bits 32..63 forced to 0 | ≤ 32 bits only |
//! | `movw` | low 16; bits 16..63 **stale** | ≤ 16 bits only |
//! | `movb` | low 8; bits 8..63 **stale** | ≤ 8 bits only |
//!
//! An address operand is read as 64 bits, so it is rewritable under `movq` and
//! nothing else. That single distinction is what makes this pass worth having
//! over the previous 32-bit-only attempt, which refused every memory operand
//! and consequently removed 10 instructions across 220 files.
//!
//! ## Legality
//!
//! With the copy at `i` and the last rewritten use at `k`:
//!
//! 1. **No barrier in `(i, k]`.** A label can be entered on another path where
//!    `%S` holds something else.
//! 2. **`%S` is not written in `(i, k]`.** Otherwise the two have diverged.
//!    Written implicitly counts too: `cqto`/`idivq` overwrite `%rdx` without
//!    naming it, `cpuid` rewrites `%rbx`, a `syscall` clobbers `%rcx` and
//!    `%r11` — [`helpers::writes_family`] answers this rule for both the
//!    classified destination and the architectural implicit write set.
//! 3. **`%D` is not written in `(i, k)`.** A later write starts a new live
//!    range that this copy does not feed.
//! 4. **No use in `(i, k]` writes `%D` while reading it.** `addl %eax, %edx`
//!    would become `addl %eax, %eax` and clobber `%S`. Pure reads are safe
//!    even when the instruction writes `%S`, because the operands are equal:
//!    `subl %edx,%eax` and `subl %eax,%eax` both yield zero.
//! 5. **`%D` is dead after `k`** ([`FileLiveness`], which answers `None` for a
//!    function it cannot fully resolve — the fold is then skipped).
//! 6. No implicit-register instruction, no shift/rotate (a variable count is
//!    architecturally pinned to `%cl` even though it is spelled out, and
//!    renaming it yields `shrq %r9b, %rsi`, which the assembler rejects), no
//!    high-byte alias (`%ah`..`%dh` have no counterpart in `%rsi`/`%r8`+), and
//!    never `%rsp`/`%rbp`.
//!
//! Register names nest as substrings — `%si` is a prefix of `%sil`, `%r8` of
//! `%r8d` — so every match here is boundary-aware. A naive `replace` produced
//! `%dxl` and the assembler rejected the function.

use super::super::types::*;
use super::helpers::{
    get_dest_reg, has_implicit_reg_usage, is_shift_or_rotate, writes_family,
};
use super::liveness::FileLiveness;

/// High-byte names have no equivalent in the `%rsi`/`%rdi`/`%r8`+ families.
const HIGH_BYTE: &[&str] = &["%ah", "%bh", "%ch", "%dh"];

/// Width index into [`REG_NAMES`]: 0 = 64-bit, 1 = 32, 2 = 16, 3 = 8.
const W64: usize = 0;
const W32: usize = 1;
const W16: usize = 2;
const W8: usize = 3;

#[inline]
fn is_frame_family(fam: RegId) -> bool {
    fam == 4 || fam == 5
}

#[inline]
fn is_name_tail(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
}

/// Boundary-aware `contains`: `%r8` must not match inside `%r8d`.
fn contains_reg(line: &str, name: &str) -> bool {
    let bytes = line.as_bytes();
    let mut from = 0;
    while let Some(rel) = line[from..].find(name) {
        let start = from + rel;
        let end = start + name.len();
        if end >= bytes.len() || !is_name_tail(bytes[end]) {
            return true;
        }
        from = start + 1;
    }
    false
}

/// Boundary-aware replace, same rule as [`contains_reg`].
fn replace_reg(line: &str, name: &str, with: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < line.len() {
        if line[i..].starts_with(name) {
            let end = i + name.len();
            if end >= bytes.len() || !is_name_tail(bytes[end]) {
                out.push_str(with);
                i = end;
                continue;
            }
        }
        let step = line[i..].chars().next().map_or(1, |c| c.len_utf8());
        out.push_str(&line[i..i + step]);
        i += step;
    }
    out
}

/// Parse `mov{b,w,l,q} %S, %D` with both operands plain registers of the
/// mnemonic's own width. Returns `(width_index, src_family, dst_family)`.
fn parse_reg_to_reg_mov(trimmed: &str) -> Option<(usize, RegId, RegId)> {
    let (mnemonic, width) = if let Some(r) = trimmed.strip_prefix("movq ") {
        (r, W64)
    } else if let Some(r) = trimmed.strip_prefix("movl ") {
        (r, W32)
    } else if let Some(r) = trimmed.strip_prefix("movw ") {
        (r, W16)
    } else if let Some(r) = trimmed.strip_prefix("movb ") {
        (r, W8)
    } else {
        return None;
    };
    let (src_part, dst_part) = mnemonic.split_once(',')?;
    let src = src_part.trim();
    let dst = dst_part.trim();
    if !src.starts_with('%') || !dst.starts_with('%') || src.contains('(') || dst.contains('(') {
        return None;
    }
    // Both operands must be spelled at the mnemonic's width; anything else is
    // a different instruction than the table in the module docs describes.
    let sfam = REG_NAMES[width].iter().position(|n| *n == src)? as RegId;
    let dfam = REG_NAMES[width].iter().position(|n| *n == dst)? as RegId;
    Some((width, sfam, dfam))
}

/// True when `line` mentions family `fam` only at widths the copy covers.
/// `max_width` is a [`REG_NAMES`] index; smaller index = wider register.
fn mentions_only_within(line: &str, fam: RegId, max_width: usize) -> bool {
    if HIGH_BYTE.iter().any(|h| contains_reg(line, h)) {
        return false;
    }
    // Any spelling WIDER than the copy is off limits.
    for w in 0..max_width {
        if contains_reg(line, REG_NAMES[w][fam as usize]) {
            return false;
        }
    }
    true
}

/// Rewrite every spelling of `old` at width `max_width` or narrower to `new`.
fn rename_within(line: &str, old: RegId, new: RegId, max_width: usize) -> String {
    let mut out = line.to_string();
    // Narrowest first: with boundary-aware matching the order is immaterial,
    // but it keeps this comparable with `helpers::replace_reg_family`.
    for w in (max_width..=W8).rev() {
        out = replace_reg(&out, REG_NAMES[w][old as usize], REG_NAMES[w][new as usize]);
    }
    out
}

// ── A. self-move elimination ────────────────────────────────────────────────

/// `mov %X, %X`. A 64/16/8-bit self-move writes the same bits back and is
/// unconditionally dead. A 32-bit one also zeroes bits 32..63, so it is dead
/// only where those are already zero.
fn eliminate_self_moves(store: &LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    // upper32_zero[f]: bits 32..63 of family f are provably zero. Same rules
    // as `redundant_ext`, recomputed because the fact does not survive between
    // passes.
    let mut upper32_zero = vec![false; 16];

    for i in 0..len {
        if infos[i].is_nop() {
            continue;
        }
        let t = infos[i].trimmed(store.get(i));
        if t.is_empty() {
            continue;
        }
        // A label may be entered from anywhere: drop every fact.
        if t.ends_with(':') {
            upper32_zero.iter_mut().for_each(|v| *v = false);
            continue;
        }
        if t.starts_with('.') {
            continue;
        }

        if let Some((width, sfam, dfam)) = parse_reg_to_reg_mov(t) {
            if sfam == dfam && !is_frame_family(dfam) {
                let dead = match width {
                    W64 | W16 | W8 => true,
                    _ => upper32_zero[dfam as usize],
                };
                if dead {
                    mark_nop(&mut infos[i]);
                    changed = true;
                    continue;
                }
            }
        }

        update_upper32_zero(t, &mut upper32_zero);
    }
    changed
}

/// Track "bits 32..63 are zero" across one instruction.
fn update_upper32_zero(t: &str, upper32_zero: &mut [bool]) {
    let mut toks = t.split_whitespace();
    let Some(op) = toks.next() else { return };

    // A call clobbers the caller-saved set; the operand scan below would not
    // see it because a direct call names no register.
    if op == "call" || op == "callq" {
        upper32_zero.iter_mut().for_each(|v| *v = false);
        return;
    }

    // Any 32-bit write zero-extends to 64 on x86-64.
    let writes_zx32 = op.ends_with('l')
        && (op.starts_with("mov")
            || op.starts_with("add")
            || op.starts_with("sub")
            || op.starts_with("and")
            || op.starts_with("or")
            || op.starts_with("xor")
            || op.starts_with("lea")
            || op.starts_with("imul")
            || op.starts_with("shl")
            || op.starts_with("shr")
            || op.starts_with("sar"))
        || op.starts_with("movz");

    if writes_zx32 {
        if let Some(comma) = t.rfind(',') {
            let dst = t[comma + 1..].trim();
            if let Some(df) = family_of_reg_name(dst) {
                if (df as usize) < upper32_zero.len() {
                    upper32_zero[df as usize] = true;
                    return;
                }
            }
        }
    }

    // Anything else: clear every family the line mentions. Conservative, and
    // the only safe default for an unrecognised mnemonic.
    for tok in t.split_whitespace() {
        for piece in tok.split(&[',', '(', ')'][..]) {
            if piece.starts_with('%') {
                if let Some(f) = family_of_reg_name(piece) {
                    if (f as usize) < upper32_zero.len() {
                        upper32_zero[f as usize] = false;
                    }
                }
            }
        }
    }
}

/// Family of a register spelling, at any width. `None` for xmm/unknown.
fn family_of_reg_name(name: &str) -> Option<RegId> {
    let n = name.trim_start_matches('%');
    for w in [W64, W32, W16, W8] {
        if let Some(p) = REG_NAMES[w]
            .iter()
            .position(|r| r.trim_start_matches('%') == n)
        {
            return Some(p as RegId);
        }
    }
    None
}

// ── B. copy folding ─────────────────────────────────────────────────────────

pub(super) fn fold_register_copies(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    if len < 2 {
        return false;
    }
    let mut changed = eliminate_self_moves(store, infos);
    let mut lv = FileLiveness::new(store, infos);

    for i in 0..len {
        if infos[i].is_nop() || infos[i].is_barrier() {
            continue;
        }
        let trimmed = infos[i].trimmed(store.get(i));
        let Some((width, sfam, dfam)) = parse_reg_to_reg_mov(trimmed) else {
            continue;
        };
        if sfam == dfam || is_frame_family(sfam) || is_frame_family(dfam) {
            continue;
        }
        if get_dest_reg(&infos[i]) != dfam {
            continue; // the line's classified destination must agree
        }

        // Collect every use of %D up to the point %D dies, %D is redefined, or
        // a barrier ends the straight-line region.
        let mut uses: Vec<usize> = Vec::new();
        let mut ok = true;
        let mut last = i;
        for j in (i + 1)..len {
            if infos[j].is_nop() {
                continue;
            }
            if infos[j].is_barrier() {
                break;
            }
            let line = infos[j].trimmed(store.get(j));
            // Rule 2: the source must still hold the copied value.  The
            // implicit second output of `cqto`/`idivq`/`cpuid`/`rep stos`...
            // counts as a write even though `get_dest_reg` names only `%rax`
            // (stress lab `intexpr` seed 1, `-O1`: a narrow reload of the
            // parameter home was folded onto `%dl` and read the division
            // remainder).
            if writes_family(&infos[j], line, sfam) {
                break;
            }
            let mentions_d = infos[j].reg_refs & (1u16 << dfam) != 0;
            if !mentions_d {
                // Rule 3: a write to %D with no read ends our range; the copy
                // is then dead and `dead_writes` will retire it.
                if writes_family(&infos[j], line, dfam) {
                    break;
                }
                continue;
            }
            // Rule 4: the use must not also write %D (explicitly — `addl
            // %eax, %edx` — or implicitly, e.g. `idivq` overwriting a %rax
            // copy).
            if writes_family(&infos[j], line, dfam) {
                ok = false;
                break;
            }
            // Rule 6.
            if has_implicit_reg_usage(line) || is_shift_or_rotate(line) {
                ok = false;
                break;
            }
            // Width rule. An address operand is a 64-bit read, so a narrower
            // copy cannot cover it; `mentions_only_within` rejects that
            // automatically because the address spells the 64-bit name.
            if !mentions_only_within(line, dfam, width) {
                ok = false;
                break;
            }
            uses.push(j);
            last = j;
        }
        if !ok || uses.is_empty() {
            continue;
        }
        // Rule 5.
        if lv.live_after(last, dfam) != Some(false) {
            continue;
        }

        let mut any = false;
        for &j in &uses {
            let line = infos[j].trimmed(store.get(j)).to_string();
            let rewritten = rename_within(&line, dfam, sfam, width);
            if rewritten != line {
                replace_line(store, &mut infos[j], j, format!("    {}", rewritten));
                any = true;
            }
        }
        if any {
            mark_nop(&mut infos[i]);
            lv.refresh_at(store, infos, last);
            changed = true;
        }
    }

    changed
}

#[cfg(test)]
#[path = "narrow_copy_fold_tests.rs"]
mod tests;
