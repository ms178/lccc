//! Late AArch64 peephole passes.
//!
//! These run once, after the cleanup rounds and Pass 3e have settled every
//! register assignment, except [`drop_narrow_call_arg_extensions`], which
//! runs inside the rounds so the copies it leaves feed the coalescers.  Each
//! pass is a local rewrite whose legality rests on the shared oracles of the
//! parent module: `abi_liveness` (GP liveness with every AAPCS64 implicit
//! read), `Liveness::build_with` over `flag_operands` (NZCV liveness),
//! `may_write_gp` (an over-approximation of the registers a line writes) and
//! `classify_implicit_operands_a64` (writeback bases, link register).
//!
//! * [`drop_narrow_call_arg_extensions`] -- an extension whose only reader is
//!   a call taking that register as a narrow argument: AAPCS64 leaves every
//!   bit above an argument's width unspecified, and every AAPCS64 callee
//!   (LCCC's own `canonical_gp_param_move` included) extends it on entry.
//!   GCC 16.2 and Clang 23.1.5 pass `f((signed char)x)` as a bare `bl f`.
//! * [`rename_clobbering_defs`] -- a write of `s` after `mov c, s`, whose
//!   value dies in the block while `c` is still read, moves to a free
//!   register, so `s` keeps the copied value for the next pass
//!   (`mov x5, x0; mov x0, #85; eor x0, x5, x0` is `mov x9, #85;
//!   eor x0, x0, x9`).
//! * [`forward_block_copies`] -- `mov d, s` then readers of `d` later in the
//!   same block read `s` instead, so the copy dies (`sel_add4`'s
//!   `mov w7, w1` three lines before the `csel` that reads it).
//! * [`fold_cset_retests`] -- `cset r, cc` re-tested by `cmp r, #0` or
//!   `cbz`/`cbnz r` while the flags `cc` came from are still intact: the
//!   test reads `cc` directly (`a < b ? x + 8 : x` is `cmp; csel ..., lt`).
//! * [`fold_csel_constants`] -- `csel d, r(+-1), zr` is `cset`/`csetm`, a
//!   `cset` negated in place is `csetm`, and `c - (x >= 0)` is
//!   `mvn t, x; add d, c, t, asr #31` (Clang 23.1.5's form).
//! * [`fuse_shift_into_alu`] -- `lsr t, s, #k; add d, a, t` is
//!   `add d, a, s, lsr #k` (also `sub`/logical/compare forms, and the
//!   W-shift feeding an X add when `s`'s upper half is provably zero).
//! * [`form_writeback_addressing`] -- a zero-offset access followed by a
//!   base increment is a post-indexed access, and an increment followed by a
//!   zero-offset access is a pre-indexed one (`ldrb w9, [x0], #1`).

use super::{
    A64_ALL_GP, CALL_READS_TAG, LineKind, Liveness, OPAQUE_LINE, a64_traps_to_handler,
    abi_liveness, classify_line, flag_operands, gp_operands, implicitly_writes_a64,
    inert_directive, invert_condition, may_write_gp, rename_def_dest, rename_gp_tokens,
    renamable_def_dest, tail_marker_before, wreg_name, xreg_name,
};
use crate::backend::peephole_common::{replace_source_reg_in_instruction, replace_whole_word};

/// The per-argument width annotation the call emitter appends after the
/// `CALL_READS_TAG` mask: `narrow=R:W,R:W` -- argument register `xR` carries
/// a `W`-bit (8, 16 or 32) integer whose upper bits the callee ignores.
pub(super) const CALL_NARROW_KEY: &str = "narrow=";

/// The significant width of each GP argument register x0-x7 at the call on
/// `line` (0 = all 64 bits, or not annotated).
pub(super) fn call_narrow_widths(line: &str) -> [u8; 8] {
    line.split_once(CALL_READS_TAG)
        .map_or([0; 8], |(_, tail)| narrow_widths(tail))
}

/// The widths of the `narrow=R:W,...` token in `text` -- a call's
/// annotation after `CALL_READS_TAG`, or a sibling call's
/// `TAIL_READS_MARKER` row: `W` for each listed `xR` (8, 16 or 32), 0 for
/// every other register.  A malformed entry is skipped: 0 is the
/// conservative answer (all 64 bits significant).
pub(super) fn narrow_widths(text: &str) -> [u8; 8] {
    let mut widths = [0u8; 8];
    let Some(list) = text
        .split_whitespace()
        .find_map(|tok| tok.strip_prefix(CALL_NARROW_KEY))
    else {
        return widths;
    };
    for entry in list.split(',') {
        if let Some((r, w)) = entry.split_once(':')
            && let (Ok(r), Ok(w)) = (r.parse::<usize>(), w.parse::<u8>())
            && r < 8
            && matches!(w, 8 | 16 | 32)
        {
            widths[r] = w;
        }
    }
    widths
}

/// The `narrow=` annotation for the argument widths `widths` (x0-x7), or an
/// empty string when every argument is 64 bits wide.
pub(in crate::backend::arm::codegen) fn format_call_narrow(widths: &[u8; 8]) -> String {
    let entries: Vec<String> = widths
        .iter()
        .enumerate()
        .filter(|(_, w)| matches!(**w, 8 | 16 | 32))
        .map(|(r, w)| format!("{r}:{w}"))
        .collect();
    if entries.is_empty() {
        String::new()
    } else {
        format!(" {CALL_NARROW_KEY}{}", entries.join(","))
    }
}

/// Mnemonic and top-level operands of an instruction line; a bracketed
/// address stays one operand (`[x0, #8]`).
fn split_insn(line: &str) -> Option<(&str, Vec<&str>)> {
    let t = line.trim();
    let (m, rest) = t.split_once(char::is_whitespace)?;
    let mut ops = Vec::with_capacity(4);
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, c) in rest.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => depth -= 1,
            ',' if depth == 0 => {
                ops.push(rest[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    ops.push(rest[start..].trim());
    Some((m, ops))
}

/// `(number, is_x)` of a plain general register token `wN`/`xN` (N <= 30).
fn gpr(tok: &str) -> Option<(u8, bool)> {
    let (x, digits) = match tok.as_bytes().first()? {
        b'x' => (true, &tok[1..]),
        b'w' => (false, &tok[1..]),
        _ => return None,
    };
    let n: u8 = digits.parse().ok()?;
    (n <= 30).then_some((n, x))
}

fn reg_name(n: u8, x: bool) -> &'static str {
    if x { xreg_name(n) } else { wreg_name(n) }
}

/// Whether line `k` ends the straight-line run a local rewrite may scan:
/// a label, any transfer, a call, a return, an exception-generating call
/// (`svc` reads x0-x8 unnamed), protected inline-assembly text, or a
/// directive that may encode an instruction.
fn ends_run(lines: &[String], kinds: &[LineKind], k: usize) -> bool {
    match kinds[k] {
        LineKind::Label
        | LineKind::Branch
        | LineKind::CondBranch
        | LineKind::CmpBranch
        | LineKind::Ret
        | LineKind::Call => true,
        LineKind::Directive => !inert_directive(&lines[k]),
        _ => lines[k] == OPAQUE_LINE || a64_traps_to_handler(lines[k].trim()),
    }
}

/// A function slice holding protected inline-assembly text: the late passes
/// leave it exactly as emitted.
fn has_opaque(lines: &[String], n: usize) -> bool {
    lines[..n].iter().any(|l| l == OPAQUE_LINE)
}

/// Lines that carry no instruction (deleted rows, inert directives).
fn is_blank(lines: &[String], kinds: &[LineKind], k: usize) -> bool {
    kinds[k] == LineKind::Nop || (kinds[k] == LineKind::Directive && inert_directive(&lines[k]))
}

/// The set live after the non-transfer line `i` (its fall-through
/// successor's live-in); the open end of the text keeps everything.
fn live_after(live: &[u64], i: usize) -> u64 {
    live.get(i + 1).copied().unwrap_or(A64_ALL_GP)
}

fn set_line(lines: &mut [String], kinds: &mut [LineKind], k: usize, text: String) {
    lines[k] = text;
    kinds[k] = classify_line(&lines[k]);
}

// ── Narrow call arguments ────────────────────────────────────────────────────

/// How many low bits the extension on `line` copies unchanged from its
/// source, and the destination and source registers: `sxtb`/`uxtb` keep 8,
/// `sxth`/`uxth` 16, `sxtw` and a W `mov` 32, an X `mov` 64, `and #0xff/
/// #0xffff/#0xffffffff` 8/16/32, `ubfx`/`sbfx #0, #w` w.
fn low_bits_preserved(line: &str) -> Option<(u8, u8, u8)> {
    let (m, ops) = split_insn(line)?;
    let (d, _) = gpr(ops.first()?)?;
    let (s, s_x) = gpr(ops.get(1)?)?;
    let bits = match (m, ops.as_slice()) {
        ("sxtb" | "uxtb", [_, _]) if !s_x => 8,
        ("sxth" | "uxth", [_, _]) if !s_x => 16,
        ("sxtw", [_, _]) if !s_x => 32,
        ("mov", [dst, _]) if dst.starts_with('w') => 32,
        // `mov x0, x19` staging an `int` the callee reads as w0: the W copy
        // (`mov w0, w19`) is all it needs, and once W it can meet the entry
        // copy `mov w19, w0` it repeats (`forward_block_copies` then
        // `eliminate_self_moves`).
        ("mov", [dst, _]) if dst.starts_with('x') && s_x => 64,
        ("and", [_, _, imm]) => match imm.to_ascii_lowercase().as_str() {
            "#0xff" | "#255" => 8,
            "#0xffff" | "#65535" => 16,
            "#0xffffffff" | "#4294967295" => 32,
            _ => return None,
        },
        ("ubfx" | "sbfx", [_, _, "#0", w]) => match *w {
            "#8" => 8,
            "#16" => 16,
            "#32" => 32,
            _ => return None,
        },
        _ => return None,
    };
    Some((bits, d, s))
}

/// Drop extensions of narrow call arguments (see the module docs).
///
/// Legality: the extension writes argument register `xR` of a call whose
/// annotation gives `xR` a width `W` no larger than the bits the extension
/// copies; between the extension and the call nothing reads or writes `xR`
/// and nothing transfers control (the scan stays in the call's block).  The
/// call is then the only reader, and it reads only the low `W` bits, which
/// equal the source's.  The source register keeps its value: a different
/// source becomes `mov wR, wS` (its low 32 bits cover any narrow width).
pub(super) fn drop_narrow_call_arg_extensions(
    lines: &mut [String],
    kinds: &mut [LineKind],
    n: usize,
) -> bool {
    let mut changed = false;
    if has_opaque(lines, n) {
        return false;
    }
    for c in 0..n {
        let widths = if kinds[c] == LineKind::Call && lines[c].contains(CALL_NARROW_KEY) {
            call_narrow_widths(&lines[c])
        } else if kinds[c] == LineKind::Branch {
            // A sibling call's `b target`: the marker row before it.
            match tail_marker_before(lines, kinds, c) {
                Some(marker) => narrow_widths(marker),
                None => continue,
            }
        } else {
            continue;
        };
        if widths == [0; 8] {
            continue;
        }
        // Registers already read or written between a candidate and the call.
        let mut touched = 0u64;
        for k in (0..c).rev() {
            if is_blank(lines, kinds, k) {
                continue;
            }
            if ends_run(lines, kinds, k) {
                break;
            }
            if let Some((bits, d, s)) = low_bits_preserved(&lines[k])
                && d < 8
                && touched & (1u64 << d) == 0
                && widths[d as usize] != 0
                && widths[d as usize] <= bits
            {
                if d == s {
                    // Gone entirely: it neither reads nor writes anything
                    // now, so an earlier extension of the same register is
                    // the call's only producer in turn.
                    kinds[k] = LineKind::Nop;
                    changed = true;
                    continue;
                }
                // The W copy: it keeps the low 32 bits -- at least the `W`
                // the callee reads -- and, unlike `mov xD, xS`, leaves S's
                // upper half unread, so an entry copy into S still
                // coalesces (`coalesce_entry_copies` narrows such a copy
                // the same way).  Already that copy: nothing to rewrite.
                let indent = &lines[k][..lines[k].len() - lines[k].trim_start().len()];
                let text = format!("{indent}mov {}, {}", wreg_name(d), wreg_name(s));
                if lines[k] != text {
                    set_line(lines, kinds, k, text);
                    changed = true;
                }
                touched |= (1u64 << d) | (1u64 << s);
                continue;
            }
            let (uses, defs) = gp_operands(&lines[k], kinds[k], false);
            touched |= uses | defs | may_write_gp(&lines[k]);
        }
    }
    changed
}

// ── Forward copy propagation ─────────────────────────────────────────────────

/// Mnemonics excluded from renaming: multi-destination or in-out operands.
fn rename_unsafe(mnemonic: &str) -> bool {
    mnemonic.starts_with("ldp")
        || mnemonic.starts_with("ldnp")
        || mnemonic.starts_with("ldax")
        || mnemonic.starts_with("ldx")
        || mnemonic.starts_with("stx")
        || mnemonic.starts_with("stlx")
        || mnemonic.starts_with("cas")
        || mnemonic.starts_with("swp")
        || mnemonic.starts_with("ldadd")
        || mnemonic.starts_with("ldclr")
        || mnemonic.starts_with("ldeor")
        || mnemonic.starts_with("ldset")
        || mnemonic.starts_with("ldsm")
        || mnemonic.starts_with("ldum")
}

/// `line` reading `src` wherever it reads `dst` (the views a `wide` copy
/// covers: both for an X copy, the W view alone for a W copy), or `None`
/// when it cannot be rewritten exactly.
fn rename_reads(line: &str, dst: u8, src: u8, wide: bool) -> Option<String> {
    let t = line.trim();
    let mnemonic = t.split_whitespace().next()?;
    if rename_unsafe(mnemonic) {
        return None;
    }
    let mentions_x = t
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|tok| tok == xreg_name(dst));
    if !wide && mentions_x {
        // `mov wD, wS` zero-extends: the X view of D is not S's.
        return None;
    }
    let all_reads = matches!(
        mnemonic,
        "cmp"
            | "cmn"
            | "tst"
            | "ccmp"
            | "ccmn"
            | "str"
            | "strb"
            | "strh"
            | "stur"
            | "sturb"
            | "sturh"
            | "stp"
            | "stnp"
    );
    let mut out = line.to_string();
    if all_reads {
        out = replace_whole_word(&out, wreg_name(dst), wreg_name(src));
        if wide {
            out = replace_whole_word(&out, xreg_name(dst), xreg_name(src));
        }
    } else {
        if let Some(r) = replace_source_reg_in_instruction(&out, wreg_name(dst), wreg_name(src)) {
            out = r;
        }
        if wide
            && let Some(r) = replace_source_reg_in_instruction(&out, xreg_name(dst), xreg_name(src))
        {
            out = r;
        }
    }
    (out != line).then_some(out)
}

/// Forward copy propagation inside a straight-line run (see module docs).
///
/// Legality: from the copy `mov d, s` (both W or both X, neither x29/x30/sp)
/// to the first line that writes `d` or `s` -- by any route `may_write_gp`
/// or a writeback base can take -- both still hold the copied value, so a
/// read of `d` (in the copy's view) may read `s`.  A line that reads `d` and
/// then writes either register is rewritten and ends the scan; a line whose
/// reads cannot all be renamed ends it unchanged.  The scan never leaves the
/// copy's block.  The copy itself is left for `eliminate_dead_moves`.
pub(super) fn forward_block_copies(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    if has_opaque(lines, n) {
        return false;
    }
    for i in 0..n {
        // `sxtw xD, wS` leaves the low 32 bits of S in D: for W-view readers
        // it is the copy `mov wD, wS`.
        let (dst, src, is_32bit) = match kinds[i] {
            LineKind::Move { dst, src, is_32bit } => (dst, src, is_32bit),
            LineKind::Sxtw { dst, src } => (dst, src, true),
            _ => continue,
        };
        if dst >= 29 || src >= 29 || dst == src {
            continue;
        }
        let pair = (1u64 << dst) | (1u64 << src);
        for j in i + 1..n {
            if is_blank(lines, kinds, j) {
                continue;
            }
            if ends_run(lines, kinds, j) {
                break;
            }
            let (uses, _) = gp_operands(&lines[j], kinds[j], false);
            let writes = may_write_gp(&lines[j]);
            let wb = implicitly_writes_a64(&lines[j], dst) || implicitly_writes_a64(&lines[j], src);
            if uses & (1u64 << dst) != 0 {
                if wb {
                    break;
                }
                match rename_reads(&lines[j], dst, src, !is_32bit) {
                    Some(text) => {
                        set_line(lines, kinds, j, text);
                        changed = true;
                        // `mov xD, xD` (a renamed `mov xS', xD` with S' == S)
                        // is a no-op: drop it and keep scanning -- it no
                        // longer writes anything.  A W self-move zeroes the
                        // upper half and stays.
                        if kinds[j]
                            == (LineKind::Move {
                                dst: src,
                                src,
                                is_32bit: false,
                            })
                        {
                            kinds[j] = LineKind::Nop;
                            continue;
                        }
                        // A read of `d` the rename could not reach (a view
                        // it does not cover) keeps the copy alive; stop.
                        let (still, _) = gp_operands(&lines[j], kinds[j], false);
                        if still & (1u64 << dst) != 0 {
                            break;
                        }
                    }
                    None => break,
                }
            }
            if writes & pair != 0 || wb {
                break;
            }
        }
    }
    changed
}

// ── Clobbering redefinitions ────────────────────────────────────────────────

/// Caller-saved registers a renamed redefinition may move to, in order of
/// preference: the emitter's call-staging temporaries first (no argument or
/// result register is ever expected in them), then the argument registers.
/// x16/x17 (veneer scratch), x18 (platform) and the callee-saved registers
/// are never chosen.
const RENAME_SCRATCH: [u8; 15] = [9, 10, 11, 12, 13, 14, 15, 8, 7, 6, 5, 4, 3, 2, 1];

/// Move a write of a copied register out of the copy's way (see the module
/// docs).
///
/// After `mov C, S`, a later write of `S` that `C`'s readers outlive keeps
/// the copy: `mov x5, x0; mov x0, #85; eor x0, x5, x0` (`x ^ 0x55`: the
/// emitter stages the constant in the accumulator) and `mov x5, x0; ldrsw
/// x4, [x0]; add w0, w4, w1; str w0, [x5]` (`*p += v`).  When the new value
/// of `S` dies in the block, a free register `F` can carry it: `S` keeps the
/// copied value, `forward_block_copies` then reads `S` for `C`, and the copy
/// dies -- `mov x9, #85; eor x0, x0, x9` and `add w4, w4, w1; str w4, [x0]`,
/// Clang 23.1.5's three and four instructions.
///
/// Legality, for the write of `S` on line `j` (a single-destination
/// producer, no writeback):
/// * every line reading the new value -- up to the next write of `S`, which
///   may itself read it (`eor x0, x5, x0`) -- is in `j`'s straight-line run
///   and has every read of `S` renamed (`rename_reads`); a `movk` extending
///   the value is renamed whole and continues it.  When the run ends first,
///   the value must be dead after its last reader (`abi_liveness` charges a
///   `ret`'s result and a call's arguments);
/// * `F` is caller-saved scratch (`RENAME_SCRATCH`), dead after `j`
///   (`j` itself may read it: `add w4, w4, w1`), and named by no line after
///   `j` through the last reader.  The run ends at calls, so nothing
///   clobbers `F` in between.
///
/// Only a write that a copy `mov C, S` earlier in the run reaches, and whose
/// copy `C` is read after it, moves: elsewhere the rename buys nothing.
pub(super) fn rename_clobbering_defs(
    lines: &mut [String],
    kinds: &mut [LineKind],
    n: usize,
) -> bool {
    if has_opaque(lines, n) || !kinds[..n].iter().any(|k| matches!(k, LineKind::Move { .. })) {
        return false;
    }
    let mut live: Option<Vec<u64>> = None;
    let mut changed = false;
    for j in 0..n {
        if is_blank(lines, kinds, j) {
            continue;
        }
        let Some((s, _)) = renamable_def_dest(&lines[j]) else {
            continue;
        };
        if s >= 29 || !copy_outlives_write(lines, kinds, n, j, s) {
            continue;
        }
        let Some((readers, run_ended)) = new_value_readers(lines, kinds, n, j, s) else {
            continue;
        };
        let Some(&(last, _)) = readers.last() else {
            continue;
        };
        // Built on first need, and again after each rewrite: the rewrites
        // move values between registers.
        if live.is_none() {
            live = abi_liveness(lines, kinds, n);
        }
        let Some(lv) = live.as_deref() else {
            return changed;
        };
        if run_ended && live_after(lv, last) & (1u64 << s) != 0 {
            continue;
        }
        let (j_uses, _) = gp_operands(&lines[j], kinds[j], false);
        let named = |f: u8| {
            (j + 1..=last).any(|k| {
                !is_blank(lines, kinds, k) && {
                    let (u, d) = gp_operands(&lines[k], kinds[k], false);
                    (u | d | may_write_gp(&lines[k])) & (1u64 << f) != 0
                }
            })
        };
        // A register `j` reads for the last time first: the rename then
        // costs no register at all.
        let Some(f) = RENAME_SCRATCH
            .iter()
            .copied()
            .filter(|&f| f != s && live_after(lv, j) & (1u64 << f) == 0 && !named(f))
            .min_by_key(|&f| j_uses & (1u64 << f) == 0)
        else {
            continue;
        };
        let mut texts = Vec::with_capacity(readers.len() + 1);
        match rename_def_dest(&lines[j], f) {
            Some(text) => texts.push((j, text)),
            None => continue,
        }
        let mut exact = true;
        for &(k, whole) in &readers {
            let text = if whole {
                Some(rename_gp_tokens(&lines[k], s, f))
            } else {
                rename_reads(&lines[k], s, f, true)
            };
            // Every read of the value must now name `F`.
            match text {
                Some(t) if whole || gp_operands(&t, classify_line(&t), false).0 & (1u64 << s) == 0 => {
                    texts.push((k, t));
                }
                _ => {
                    exact = false;
                    break;
                }
            }
        }
        if !exact {
            continue;
        }
        for (k, text) in texts {
            set_line(lines, kinds, k, text);
        }
        live = None;
        changed = true;
    }
    changed
}

/// Whether a copy `mov C, s` earlier in line `j`'s run still holds `s`'s
/// value at `j` (neither register written in between) and `C` is read
/// after `j`, before the run ends or `C` is written.
fn copy_outlives_write(lines: &[String], kinds: &[LineKind], n: usize, j: usize, s: u8) -> bool {
    let mut copies = 0u64;
    let mut written = 0u64; // by the lines between the candidate and `j`
    for k in (0..j).rev() {
        if is_blank(lines, kinds, k) {
            continue;
        }
        if ends_run(lines, kinds, k) {
            break;
        }
        if let LineKind::Move { dst, src, .. } = kinds[k]
            && src == s
            && dst != s
            && dst < 29
            && written & (1u64 << dst) == 0
        {
            copies |= 1u64 << dst;
        }
        let writes = may_write_gp(&lines[k]) | gp_operands(&lines[k], kinds[k], false).1;
        if writes & (1u64 << s) != 0 {
            break;
        }
        written |= writes;
    }
    for k in j + 1..n {
        if copies == 0 {
            return false;
        }
        if is_blank(lines, kinds, k) {
            continue;
        }
        if ends_run(lines, kinds, k) {
            return false;
        }
        let (uses, defs) = gp_operands(&lines[k], kinds[k], false);
        if uses & copies != 0 {
            return true;
        }
        copies &= !(defs | may_write_gp(&lines[k]));
    }
    false
}

/// The lines after `j` that read the value line `j` writes into `s` --
/// `(line, renamed whole)`, a `movk` extending the value renamed whole --
/// and whether the run ended with the value possibly live (no later write
/// of `s` in it).  `None` when the value meets a writeback of `s` or an
/// explicit read by the line ending the run.
fn new_value_readers(
    lines: &[String],
    kinds: &[LineKind],
    n: usize,
    j: usize,
    s: u8,
) -> Option<(Vec<(usize, bool)>, bool)> {
    let bit = 1u64 << s;
    let mut readers = Vec::new();
    for k in j + 1..n {
        if is_blank(lines, kinds, k) {
            continue;
        }
        let (uses, defs) = gp_operands(&lines[k], kinds[k], false);
        if ends_run(lines, kinds, k) {
            return (uses & bit == 0).then_some((readers, true));
        }
        if implicitly_writes_a64(&lines[k], s) {
            return None;
        }
        let writes = (defs | may_write_gp(&lines[k])) & bit != 0;
        if uses & bit != 0 {
            let movk = lines[k].trim_start().starts_with("movk ");
            readers.push((k, movk));
            if movk {
                continue;
            }
        }
        if writes {
            return Some((readers, false));
        }
    }
    Some((readers, true))
}

/// Side-effect-free single-destination ALU instructions: no memory access,
/// no flag write, nothing written but the first operand.
const PURE_ALU: &[&str] = &[
    "add", "sub", "and", "orr", "eor", "eon", "orn", "bic", "neg", "mvn", "lsl", "lsr", "asr",
    "ror", "mul", "madd", "msub", "mneg", "sxtw", "sxth", "sxtb", "uxth", "uxtb", "ubfx", "sbfx",
    "ubfiz", "sbfiz", "csel", "csinc", "csinv", "csneg", "cset", "csetm", "cinc", "cinv", "cneg",
    "clz", "cls", "rbit", "rev", "rev16", "rev32",
];

/// Delete a pure ALU definition whose destination no path reads
/// (`abi_liveness`, which charges every AAPCS64 implicit read): the
/// late rewrites retire the readers of an extension or a shift, and
/// `eliminate_dead_moves` only takes moves.  x29, x30 and sp writes are
/// frame bookkeeping and stay.
pub(super) fn eliminate_dead_pure_defs(
    lines: &mut [String],
    kinds: &mut [LineKind],
    n: usize,
) -> bool {
    if has_opaque(lines, n) {
        return false;
    }
    let candidate = |line: &str| {
        let m = line.split_whitespace().next()?;
        if !PURE_ALU.contains(&m) {
            return None;
        }
        renamable_def_dest(line).map(|(r, _)| r).filter(|&r| r < 29)
    };
    if !(0..n).any(|k| !is_blank(lines, kinds, k) && candidate(&lines[k]).is_some()) {
        return false;
    }
    let Some(live) = abi_liveness(lines, kinds, n) else {
        return false;
    };
    let mut changed = false;
    for k in 0..n {
        if is_blank(lines, kinds, k) {
            continue;
        }
        if let Some(r) = candidate(&lines[k])
            && live_after(&live, k) & (1u64 << r) == 0
        {
            kinds[k] = LineKind::Nop;
            changed = true;
        }
    }
    changed
}

// ── Callee-saved homes nobody reads ─────────────────────────────────────────

/// A callee-saved save or restore row: `str|ldr xR, [base, #off]` or
/// `stp|ldp xA, xB, [base, #off]` with base sp or x29 and no writeback.
/// Returns (is_store, registers, base text, offset).
fn frame_row(line: &str) -> Option<(bool, Vec<u8>, String, i64)> {
    let (m, ops) = split_insn(line)?;
    let (store, pair) = match m {
        "str" => (true, false),
        "ldr" => (false, false),
        "stp" => (true, true),
        "ldp" => (false, true),
        _ => return None,
    };
    let want = if pair { 3 } else { 2 };
    if ops.len() != want {
        return None;
    }
    let mut regs = Vec::with_capacity(2);
    for op in &ops[..want - 1] {
        let (r, x) = gpr(op)?;
        if !x {
            return None;
        }
        regs.push(r);
    }
    let inner = ops[want - 1].strip_prefix('[')?.strip_suffix(']')?;
    let (base, off) = match inner.split_once(',') {
        Some((b, o)) => (b.trim(), o.trim().strip_prefix('#')?.parse::<i64>().ok()?),
        None => (inner.trim(), 0),
    };
    if !matches!(base, "sp" | "x29") {
        return None;
    }
    Some((store, regs, base.to_string(), off))
}

/// Delete the save, the restores and the dead definitions of every
/// callee-saved register x19-x28 the function never reads (the AArch64
/// form of the RISC-V `eliminate_unread_callee_saved_homes`): copy
/// propagation forwards the reads of a home, and what remains is a write,
/// a save and a restore -- `keep_none` saved and restored an x19 nothing
/// touched.  Legality: no line reads the register (`gp_operands`; a `ret`'s
/// hand-back of the callee-saved set is exactly what keeping the caller's
/// value preserves); every write is a pure ALU definition or a plain load;
/// exactly one save row, whose slot text appears only in it and in
/// matching restore rows.  A pair row with one unread member becomes a
/// single-register row of the other at its own slot.
pub(super) fn drop_unread_callee_saved_homes(
    lines: &mut [String],
    kinds: &mut [LineKind],
    n: usize,
) -> bool {
    if has_opaque(lines, n) {
        return false;
    }
    let mut unread: Vec<(u8, Vec<usize>)> = Vec::new();
    for reg in 19u8..=28 {
        let bit = 1u64 << reg;
        let mut writes = Vec::new();
        let mut saves = 0usize;
        let mut ok = true;
        let mut mentioned = false;
        for k in 0..n {
            if is_blank(lines, kinds, k) || matches!(kinds[k], LineKind::Label | LineKind::Ret) {
                continue;
            }
            if let Some((store, regs, _, _)) = frame_row(&lines[k])
                && regs.contains(&reg)
            {
                mentioned = true;
                saves += usize::from(store);
                continue;
            }
            let (uses, _) = gp_operands(&lines[k], kinds[k], false);
            if uses & bit != 0 {
                ok = false;
                break;
            }
            if may_write_gp(&lines[k]) & bit != 0 {
                mentioned = true;
                let m = lines[k].split_whitespace().next().unwrap_or("");
                let pure = renamable_def_dest(&lines[k]).is_some_and(|(r, _)| r == reg)
                    && (PURE_ALU.contains(&m)
                        || matches!(
                            m,
                            "mov"
                                | "movz"
                                | "movn"
                                | "ldr"
                                | "ldrb"
                                | "ldrh"
                                | "ldrsb"
                                | "ldrsh"
                                | "ldrsw"
                                | "ldur"
                        ));
                if !pure {
                    ok = false;
                    break;
                }
                writes.push(k);
            }
        }
        if ok && mentioned && saves == 1 {
            unread.push((reg, writes));
        }
    }
    if unread.is_empty() {
        return false;
    }
    let is_unread = |r: u8| unread.iter().any(|(u, _)| *u == r);
    let mut changed = false;
    // Rewrite the frame rows: per save row, its slot must serve only it and
    // its restores.
    let rows: Vec<(usize, bool, Vec<u8>, String, i64)> = (0..n)
        .filter(|&k| !is_blank(lines, kinds, k))
        .filter_map(|k| frame_row(&lines[k]).map(|(st, regs, b, o)| (k, st, regs, b, o)))
        .collect();
    let mut done: Vec<u8> = Vec::new();
    for (k, store, regs, base, off) in &rows {
        if !*store || !regs.iter().any(|&r| is_unread(r)) {
            continue;
        }
        let matching: Vec<usize> = rows
            .iter()
            .filter(|(_, st, rr, b, o)| !*st && rr == regs && b == base && o == off)
            .map(|(j, ..)| *j)
            .collect();
        // Every other row naming the same slot must be one of these.
        let clash = rows.iter().any(|(j, _, rr, b, o)| {
            j != k
                && !matching.contains(j)
                && b == base
                && (*o == *off
                    || rr.len() == 2 && *o + 8 == *off
                    || regs.len() == 2 && *o == *off + 8)
        });
        if clash {
            continue;
        }
        let indent = lines[*k][..lines[*k].len() - lines[*k].trim_start().len()].to_string();
        match regs.as_slice() {
            [r] => {
                kinds[*k] = LineKind::Nop;
                for &j in &matching {
                    kinds[j] = LineKind::Nop;
                }
                done.push(*r);
            }
            [a, b] if is_unread(*a) && is_unread(*b) => {
                kinds[*k] = LineKind::Nop;
                for &j in &matching {
                    kinds[j] = LineKind::Nop;
                }
                done.extend([*a, *b]);
            }
            [a, b] => {
                let (keep, slot, gone) = if is_unread(*a) {
                    (*b, *off + 8, *a)
                } else {
                    (*a, *off, *b)
                };
                let addr = if slot == 0 {
                    format!("[{base}]")
                } else {
                    format!("[{base}, #{slot}]")
                };
                set_line(
                    lines,
                    kinds,
                    *k,
                    format!("{indent}str {}, {addr}", xreg_name(keep)),
                );
                for &j in &matching {
                    let ind = lines[j][..lines[j].len() - lines[j].trim_start().len()].to_string();
                    set_line(
                        lines,
                        kinds,
                        j,
                        format!("{ind}ldr {}, {addr}", xreg_name(keep)),
                    );
                }
                done.push(gone);
            }
            _ => continue,
        }
        changed = true;
    }
    for (reg, writes) in &unread {
        if done.contains(reg) {
            for &w in writes {
                kinds[w] = LineKind::Nop;
            }
        }
    }
    changed
}

// ── cset re-tests ────────────────────────────────────────────────────────────

/// The condition operand of a flag reader (`b.cc`, or the last operand of a
/// conditional select/set/compare), with a rewrite to another condition.
fn reader_condition(line: &str) -> Option<(&str, Box<dyn Fn(&str) -> String + '_>)> {
    let t = line.trim();
    if let Some(cc) = t.strip_prefix("b.") {
        let (cc, target) = cc.split_once(char::is_whitespace)?;
        let indent = &line[..line.len() - line.trim_start().len()];
        let target = target.trim();
        return Some((cc, Box::new(move |new| format!("{indent}b.{new} {target}"))));
    }
    let (m, ops) = split_insn(t)?;
    if !matches!(
        m,
        "csel" | "csinc" | "csinv" | "csneg" | "cset" | "csetm" | "cinc" | "cinv" | "cneg"
    ) {
        return None;
    }
    let cc = *ops.last()?;
    let head = &line[..line.rfind(cc)?];
    Some((cc, Box::new(move |new| format!("{head}{new}"))))
}

/// `cmp r, #0`-style zero tests of register `r`: Z is set exactly when `r`
/// is zero.  Only `eq`/`ne` readers may be redirected (N, C and V differ).
fn zero_test_of(line: &str) -> Option<u8> {
    let (m, ops) = split_insn(line)?;
    let (r, x) = gpr(ops.first()?)?;
    let zr = if x { "xzr" } else { "wzr" };
    match (m, ops.as_slice()) {
        ("cmp", [_, "#0"]) => Some(r),
        ("cmp", [_, z]) if *z == zr => Some(r),
        ("tst", [a, b]) if a == b => Some(r),
        _ => None,
    }
}

/// The line index of label `name` in this function's slice.
fn label_line(lines: &[String], kinds: &[LineKind], n: usize, name: &str) -> Option<usize> {
    (0..n).find(|&k| kinds[k] == LineKind::Label && lines[k].trim().trim_end_matches(':') == name)
}

/// Fold re-tests of a `cset` result (see module docs).
///
/// Legality: between `cset r, cc` and the test, nothing writes NZCV or `r`,
/// so at the test the flags are exactly those `cc` was computed from.
/// * `cbnz r, L` / `cbz r, L` become `b.cc L` / `b.!cc L` (the same imm19
///   reach).
/// * `cmp r, #0` (or `tst r, r`) is deleted when every flag reader up to
///   the next flag write is an `eq`/`ne` reader in the same block -- each
///   gets `!cc`/`cc` -- and NZCV is dead after the last of them.
/// The `cset` itself goes when `r` is dead after the test (on every edge).
pub(super) fn fold_cset_retests(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    if has_opaque(lines, n) {
        return false;
    }
    let mut flags: Option<Option<Liveness>> = None;
    let mut gp: Option<Option<Vec<u64>>> = None;
    for c in 0..n {
        let Some((m, ops)) = split_insn(&lines[c]) else {
            continue;
        };
        if m != "cset" || ops.len() != 2 {
            continue;
        }
        let Some((r, _)) = gpr(ops[0]) else {
            continue;
        };
        let cc = ops[1].to_string();
        let Some(inv) = invert_condition(&cc) else {
            continue;
        };
        // Find the test: the first later line that touches r or NZCV.
        let mut t = None;
        for k in c + 1..n {
            if is_blank(lines, kinds, k) {
                continue;
            }
            if kinds[k] == LineKind::CmpBranch {
                t = Some(k);
                break;
            }
            if ends_run(lines, kinds, k) {
                break;
            }
            let (fu, fd) = flag_operands(&lines[k], kinds[k]);
            if zero_test_of(&lines[k]) == Some(r) {
                t = Some(k);
                break;
            }
            if fu != 0 || fd != 0 || may_write_gp(&lines[k]) & (1u64 << r) != 0 {
                break;
            }
        }
        let Some(t) = t else {
            continue;
        };
        let live = gp.get_or_insert_with(|| abi_liveness(lines, kinds, n));
        let Some(live) = live.as_ref() else {
            return changed;
        };
        // r's readers between the cset and the test keep the cset.
        let read_between = (c + 1..t).any(|k| {
            !is_blank(lines, kinds, k)
                && gp_operands(&lines[k], kinds[k], false).0 & (1u64 << r) != 0
        });
        if kinds[t] == LineKind::CmpBranch {
            let Some((bm, bops)) = split_insn(&lines[t]) else {
                continue;
            };
            let new_cc = match bm {
                "cbnz" => cc.as_str(),
                "cbz" => inv,
                _ => continue,
            };
            if bops.len() != 2 || gpr(bops[0]).map(|g| g.0) != Some(r) {
                continue;
            }
            let target = bops[1].to_string();
            let Some(tl) = label_line(lines, kinds, n, &target) else {
                continue;
            };
            let dead = live_after(live, t) & (1u64 << r) == 0 && live[tl] & (1u64 << r) == 0;
            let indent = lines[t][..lines[t].len() - lines[t].trim_start().len()].to_string();
            set_line(lines, kinds, t, format!("{indent}b.{new_cc} {target}"));
            if dead && !read_between {
                kinds[c] = LineKind::Nop;
            }
            flags = None;
            gp = None;
            changed = true;
            continue;
        }
        // `cmp r, #0`: collect the readers it serves.
        let mut readers = Vec::new();
        let mut ok = true;
        for k in t + 1..n {
            if is_blank(lines, kinds, k) {
                continue;
            }
            let (fu, fd) = flag_operands(&lines[k], kinds[k]);
            if fu != 0 {
                match reader_condition(&lines[k]) {
                    Some((rcc, _)) if rcc == "eq" || rcc == "ne" => readers.push(k),
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if fd != 0 || ends_run(lines, kinds, k) {
                break;
            }
        }
        let Some(&last) = readers.last() else {
            continue;
        };
        if !ok {
            continue;
        }
        let fl = flags.get_or_insert_with(|| {
            Liveness::build_with(lines, kinds, 0, n, |k| flag_operands(&lines[k], kinds[k]))
        });
        let Some(fl) = fl.as_ref() else {
            return changed;
        };
        if fl.live_after(last, 0) != Some(false) {
            continue;
        }
        let dead = live_after(live, t) & (1u64 << r) == 0;
        for &k in &readers {
            let text = match reader_condition(&lines[k]) {
                Some((rcc, rewrite)) => rewrite(if rcc == "ne" { cc.as_str() } else { inv }),
                None => continue,
            };
            set_line(lines, kinds, k, text);
        }
        kinds[t] = LineKind::Nop;
        if dead && !read_between {
            kinds[c] = LineKind::Nop;
        }
        flags = None;
        gp = None;
        changed = true;
    }
    changed
}

// ── Conditional-select constants ─────────────────────────────────────────────

/// The constant line `k` puts in register `r` (64-bit value), for the
/// `mov`/`movn` immediates the emitter produces.
fn move_imm_value(line: &str, r: u8) -> Option<u64> {
    let (m, ops) = split_insn(line)?;
    let (d, x) = gpr(ops.first()?)?;
    if d != r || ops.len() != 2 {
        return None;
    }
    let imm = ops[1].strip_prefix('#')?;
    let v: i64 = if let Some(h) = imm.strip_prefix("0x") {
        i64::from_str_radix(h, 16).ok()?
    } else if let Some(h) = imm.strip_prefix("-0x") {
        -i64::from_str_radix(h, 16).ok()?
    } else {
        imm.parse().ok()?
    };
    let v = match m {
        "mov" => v as u64,
        "movn" if (0..=0xffff).contains(&v) => !(v as u64),
        _ => return None,
    };
    Some(if x { v } else { v & 0xffff_ffff })
}

/// The constant register `r` holds at line `at`: its last write in the same
/// straight-line run is a `mov`/`movn` immediate.  Returns the value and the
/// defining line.
fn constant_at(lines: &[String], kinds: &[LineKind], at: usize, r: u8) -> Option<(u64, usize)> {
    for k in (0..at).rev() {
        if is_blank(lines, kinds, k) {
            continue;
        }
        if ends_run(lines, kinds, k) {
            return None;
        }
        if may_write_gp(&lines[k]) & (1u64 << r) != 0 {
            return move_imm_value(&lines[k], r).map(|v| (v, k));
        }
    }
    None
}

/// Fold conditional-select constants (see module docs).
///
/// * `csel d, a, b, cc` with one arm the zero register and the other a
///   register holding 1 or all-ones in the select's view: `cset`/`csetm`
///   (with the condition inverted when the constant is the false arm).  The
///   constant's `mov` goes when nothing else reads it.
/// * `cset r, cc; neg e, r` (adjacent; `r` dead after unless `e == r`):
///   `csetm e, cc` -- `-(0|1)` in `e`'s view is exactly `csetm`'s result.
/// * `cmp x, #0; cset r, ge; sub d, a, r` (adjacent, one width, NZCV dead
///   after, `r` dead after unless `r == d`): `mvn r, x; add d, a, r, asr #N`
///   where `asr` of the complement is `-1` exactly when `x >= 0`.
pub(super) fn fold_csel_constants(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    if has_opaque(lines, n) {
        return false;
    }
    let mut gp: Option<Option<Vec<u64>>> = None;
    let mut flags: Option<Option<Liveness>> = None;
    for i in 0..n {
        if is_blank(lines, kinds, i) {
            continue;
        }
        let Some((m, ops)) = split_insn(&lines[i]) else {
            continue;
        };
        let indent = lines[i][..lines[i].len() - lines[i].trim_start().len()].to_string();
        match (m, ops.as_slice()) {
            ("csel", [d, a, b, cc]) => {
                let Some((_, x)) = gpr(d) else { continue };
                let zr = if x { "xzr" } else { "wzr" };
                if *a != zr && *b != zr {
                    // `csel d, a, r(-1), cc` is `csinv d, a, zr, cc` and with
                    // r(+1) `csinc d, a, zr, cc` (the inverted/incremented
                    // zero register is the constant); a constant true arm
                    // swaps the arms and inverts the condition.
                    if fold_csel_unit_arm(lines, kinds, n, i, &mut gp) {
                        changed = true;
                    }
                    continue;
                }
                let (reg_arm, const_is_true) = if *b == zr {
                    (*a, true)
                } else if *a == zr {
                    (*b, false)
                } else {
                    continue;
                };
                let Some((r, rx)) = gpr(reg_arm) else {
                    continue;
                };
                if rx != x {
                    continue;
                }
                let Some((v, def)) = constant_at(lines, kinds, i, r) else {
                    continue;
                };
                let view = if x { v } else { v & 0xffff_ffff };
                let ones = if x { u64::MAX } else { 0xffff_ffff };
                let m2 = if view == 1 {
                    "cset"
                } else if view == ones {
                    "csetm"
                } else {
                    continue;
                };
                let Some(inv) = invert_condition(cc) else {
                    continue;
                };
                let cond = if const_is_true {
                    cc.to_string()
                } else {
                    inv.to_string()
                };
                let d = d.to_string();
                set_line(lines, kinds, i, format!("{indent}{m2} {d}, {cond}"));
                let live = gp.get_or_insert_with(|| abi_liveness(lines, kinds, n));
                if let Some(live) = live.as_ref()
                    && live_after(live, i) & (1u64 << r) == 0
                    && !(def + 1..i).any(|k| {
                        !is_blank(lines, kinds, k)
                            && gp_operands(&lines[k], kinds[k], false).0 & (1u64 << r) != 0
                    })
                    && gpr(&d).map(|g| g.0) != Some(r)
                {
                    kinds[def] = LineKind::Nop;
                }
                gp = None;
                changed = true;
            }
            ("cset", [r_tok, cc]) => {
                let Some((r, _)) = gpr(r_tok) else { continue };
                let cc = cc.to_string();
                let Some(k) = (i + 1..n).find(|&k| !is_blank(lines, kinds, k)) else {
                    continue;
                };
                let Some((m2, ops2)) = split_insn(&lines[k]) else {
                    continue;
                };
                if m2 == "neg"
                    && let [e, f] = ops2.as_slice()
                    && let (Some((en, _)), Some((fnum, _))) = (gpr(e), gpr(f))
                    && fnum == r
                {
                    let e = e.to_string();
                    let live = gp.get_or_insert_with(|| abi_liveness(lines, kinds, n));
                    let Some(live) = live.as_ref() else {
                        return changed;
                    };
                    if en != r && live_after(live, k) & (1u64 << r) != 0 {
                        continue;
                    }
                    set_line(lines, kinds, i, format!("{indent}csetm {e}, {cc}"));
                    kinds[k] = LineKind::Nop;
                    gp = None;
                    changed = true;
                    continue;
                }
                if !matches!(cc.as_str(), "ge" | "pl") || m2 != "sub" {
                    continue;
                }
                let [d, a, b] = ops2.as_slice() else { continue };
                let (Some((dn, dx)), Some((_, ax)), Some((bn, bx))) = (gpr(d), gpr(a), gpr(b))
                else {
                    continue;
                };
                if bn != r || dx != ax || dx != bx {
                    continue;
                }
                // The compare right before the cset, against zero, same width.
                let Some(c) = (0..i).rev().find(|&k| !is_blank(lines, kinds, k)) else {
                    continue;
                };
                let Some((cm, cops)) = split_insn(&lines[c]) else {
                    continue;
                };
                let zr = if dx { "xzr" } else { "wzr" };
                let [xs, zero] = cops.as_slice() else {
                    continue;
                };
                let Some((xn, xx)) = gpr(xs) else { continue };
                if cm != "cmp" || xx != dx || !(*zero == "#0" || *zero == zr) {
                    continue;
                }
                let fl = flags.get_or_insert_with(|| {
                    Liveness::build_with(lines, kinds, 0, n, |q| flag_operands(&lines[q], kinds[q]))
                });
                let Some(fl) = fl.as_ref() else {
                    return changed;
                };
                if fl.live_after(i, 0) != Some(false) {
                    continue;
                }
                let live = gp.get_or_insert_with(|| abi_liveness(lines, kinds, n));
                let Some(live) = live.as_ref() else {
                    return changed;
                };
                if dn != r && live_after(live, k) & (1u64 << r) != 0 {
                    continue;
                }
                let (d, a) = (d.to_string(), a.to_string());
                let top = if dx { 63 } else { 31 };
                let (rr, xr) = (reg_name(r, dx), reg_name(xn, dx));
                kinds[c] = LineKind::Nop;
                set_line(lines, kinds, i, format!("{indent}mvn {rr}, {xr}"));
                set_line(
                    lines,
                    kinds,
                    k,
                    format!("{indent}add {d}, {a}, {rr}, asr #{top}"),
                );
                gp = None;
                flags = None;
                changed = true;
            }
            _ => {}
        }
    }
    changed
}

/// `csel d, a, r, cc` where `r` holds -1 or +1 in the select's view and
/// `a` is a plain register: `csinv`/`csinc d, a, zr, cc` (or, for a
/// constant true arm, the same with the arms swapped and `cc` inverted).
/// The constant's `mov` goes when nothing else reads it.
fn fold_csel_unit_arm(
    lines: &mut [String],
    kinds: &mut [LineKind],
    n: usize,
    i: usize,
    gp: &mut Option<Option<Vec<u64>>>,
) -> bool {
    let Some((_, ops)) = split_insn(&lines[i]) else {
        return false;
    };
    let [d, a, b, cc] = ops.as_slice() else {
        return false;
    };
    let Some((_, x)) = gpr(d) else { return false };
    let (Some((an, ax)), Some((bn, bx))) = (gpr(a), gpr(b)) else {
        return false;
    };
    if ax != x || bx != x || an == bn {
        return false;
    }
    let ones = if x { u64::MAX } else { 0xffff_ffff };
    let view = |v: u64| if x { v } else { v & 0xffff_ffff };
    let pick = |r: u8| {
        constant_at(lines, kinds, i, r).and_then(|(v, def)| match view(v) {
            1 => Some(("csinc", def)),
            v if v == ones => Some(("csinv", def)),
            _ => None,
        })
    };
    let (m, keep, r, def, cond) = if let Some((m, def)) = pick(bn) {
        (m, *a, bn, def, cc.to_string())
    } else if let Some((m, def)) = pick(an) {
        let Some(inv) = invert_condition(cc) else {
            return false;
        };
        (m, *b, an, def, inv.to_string())
    } else {
        return false;
    };
    let zr = if x { "xzr" } else { "wzr" };
    let indent = lines[i][..lines[i].len() - lines[i].trim_start().len()].to_string();
    let (d, keep) = (d.to_string(), keep.to_string());
    set_line(
        lines,
        kinds,
        i,
        format!("{indent}{m} {d}, {keep}, {zr}, {cond}"),
    );
    let live = gp.get_or_insert_with(|| abi_liveness(lines, kinds, n));
    if let Some(live) = live.as_ref()
        && live_after(live, i) & (1u64 << r) == 0
        && gpr(&d).map(|g| g.0) != Some(r)
        && !(def + 1..i).any(|k| {
            !is_blank(lines, kinds, k)
                && gp_operands(&lines[k], kinds[k], false).0 & (1u64 << r) != 0
        })
    {
        kinds[def] = LineKind::Nop;
    }
    *gp = None;
    true
}

// ── Shifted-register operands ────────────────────────────────────────────────

/// Fuse a constant shift into the ALU instruction that consumes it.
///
/// `lsl|lsr|asr t, s, #k` and the first later reader of `t` in the same
/// run, an `add|adds|sub|subs|and|ands|orr|eor|bic|bics|orn|eon d, n, t`
/// (or `cmp|cmn|tst n, t`), become `op d, n, s, <shift> #k`.  Commutative
/// operations also take `t` as their first source.
///
/// Legality: nothing between writes `s` (or `t`), so `s` still holds the
/// shifted value's source; `t` is dead after the reader (or is the reader's
/// destination).  Widths: one width throughout, or a W `lsr` feeding an X
/// instruction when `s` was last written as a W register in this run --
/// every W write zeroes bits 63:32, so `x_s >> k` equals the zero-extended
/// `w_s >> k`.  No operand may be sp (the shifted-register forms read 31 as
/// the zero register).
pub(super) fn fuse_shift_into_alu(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    const FUSABLE: &[&str] = &[
        "add", "adds", "sub", "subs", "and", "ands", "orr", "eor", "bic", "bics", "orn", "eon",
        "cmp", "cmn", "tst",
    ];
    const COMMUTATIVE: &[&str] = &["add", "adds", "and", "ands", "orr", "eor", "cmn", "tst"];
    let mut changed = false;
    if has_opaque(lines, n) {
        return false;
    }
    let mut gp: Option<Option<Vec<u64>>> = None;
    for a in 0..n {
        let Some((sm, sops)) = split_insn(&lines[a]) else {
            continue;
        };
        if !matches!(sm, "lsl" | "lsr" | "asr") {
            continue;
        }
        let [t_tok, s_tok, k_tok] = sops.as_slice() else {
            continue;
        };
        let (Some((t, tx)), Some((s, sx))) = (gpr(t_tok), gpr(s_tok)) else {
            continue;
        };
        let Some(k) = k_tok.strip_prefix('#').and_then(|v| v.parse::<u32>().ok()) else {
            continue;
        };
        if tx != sx || k >= if sx { 64 } else { 32 } {
            continue;
        }
        let (sm, k) = (sm.to_string(), k);
        // The first later reader of t.
        let mut b = None;
        for j in a + 1..n {
            if is_blank(lines, kinds, j) {
                continue;
            }
            if ends_run(lines, kinds, j) {
                break;
            }
            let (uses, _) = gp_operands(&lines[j], kinds[j], false);
            if uses & (1u64 << t) != 0 {
                b = Some(j);
                break;
            }
            if may_write_gp(&lines[j]) & ((1u64 << t) | (1u64 << s)) != 0 {
                break;
            }
        }
        let Some(b) = b else { continue };
        let Some((bm, bops)) = split_insn(&lines[b]) else {
            continue;
        };
        if !FUSABLE.contains(&bm) || implicitly_writes_a64(&lines[b], t) {
            continue;
        }
        let flag_only = matches!(bm, "cmp" | "cmn" | "tst");
        let (dest, srcs): (Option<&str>, Vec<&str>) = if flag_only {
            (None, bops.clone())
        } else {
            (
                bops.first().copied(),
                bops.iter().skip(1).copied().collect(),
            )
        };
        if srcs.len() != 2 {
            continue;
        }
        let (Some((n0, x0)), Some((n1, x1))) = (gpr(srcs[0]), gpr(srcs[1])) else {
            continue;
        };
        if x0 != x1 {
            continue;
        }
        let bx = x0;
        let other = if n1 == t && n0 != t {
            srcs[0]
        } else if n0 == t && n1 != t && COMMUTATIVE.contains(&bm) {
            srcs[1]
        } else {
            continue;
        };
        if let Some(d) = dest
            && gpr(d).is_none_or(|(_, dx)| dx != bx)
        {
            continue;
        }
        // Widths.
        if bx != tx {
            // Only a W `lsr` into an X instruction, with s's upper half zero.
            if !(sm == "lsr" && !tx && bx) {
                continue;
            }
            let mut upper_zero = false;
            for q in (0..a).rev() {
                if is_blank(lines, kinds, q) {
                    continue;
                }
                if ends_run(lines, kinds, q) {
                    break;
                }
                if may_write_gp(&lines[q]) & (1u64 << s) != 0 {
                    upper_zero = renamable_def_dest(&lines[q]) == Some((s, true));
                    break;
                }
            }
            if !upper_zero {
                continue;
            }
        }
        let live = gp.get_or_insert_with(|| abi_liveness(lines, kinds, n));
        let Some(live) = live.as_ref() else {
            return changed;
        };
        let dest_is_t = dest.and_then(gpr).map(|g| g.0) == Some(t);
        if !dest_is_t && live_after(live, b) & (1u64 << t) != 0 {
            continue;
        }
        let shifted = format!("{}, {sm} #{k}", reg_name(s, bx));
        let indent = lines[b][..lines[b].len() - lines[b].trim_start().len()].to_string();
        let text = match dest {
            Some(d) => format!("{indent}{bm} {d}, {other}, {shifted}"),
            None => format!("{indent}{bm} {other}, {shifted}"),
        };
        set_line(lines, kinds, b, text);
        kinds[a] = LineKind::Nop;
        gp = None;
        changed = true;
    }
    changed
}

// ── Writeback addressing ─────────────────────────────────────────────────────

/// Single-register loads and stores with an immediate-offset form that also
/// has pre- and post-indexed forms.
const WRITEBACK_MEM: &[&str] = &[
    "ldr", "ldrb", "ldrh", "ldrsb", "ldrsh", "ldrsw", "str", "strb", "strh",
];

/// `(mnemonic, transferred register, base register number, offset)` of a
/// plain base-plus-immediate access `op rt, [xB{, #off}]`.
fn base_imm_access(line: &str) -> Option<(&str, &str, u8, i64)> {
    let (m, ops) = split_insn(line)?;
    if !WRITEBACK_MEM.contains(&m) || ops.len() != 2 {
        return None;
    }
    let inner = ops[1].strip_prefix('[')?.strip_suffix(']')?;
    let mut parts = inner.split(',').map(str::trim);
    let base = parts.next()?;
    let off = match parts.next() {
        None => 0,
        Some(o) => {
            let o = o.strip_prefix('#')?;
            if let Some(h) = o.strip_prefix("0x") {
                i64::from_str_radix(h, 16).ok()?
            } else {
                o.parse().ok()?
            }
        }
    };
    if parts.next().is_some() {
        return None;
    }
    let (b, bx) = gpr(base)?;
    if !bx {
        return None;
    }
    Some((m, ops[0], b, off))
}

/// `add|sub xB, xB, #imm` as a signed byte delta for base `b`.
fn base_increment(line: &str, b: u8) -> Option<i64> {
    let (m, ops) = split_insn(line)?;
    let [d, s, imm] = ops.as_slice() else {
        return None;
    };
    if gpr(d) != Some((b, true)) || gpr(s) != Some((b, true)) {
        return None;
    }
    let v: i64 = imm.strip_prefix('#')?.parse().ok()?;
    match m {
        "add" => Some(v),
        "sub" => Some(-v),
        _ => None,
    }
}

/// Whether the transferred register `rt` of an access names the base `b`
/// (writeback with Rt == Rn is CONSTRAINED UNPREDICTABLE for loads and
/// stores alike).
fn transfers_base(rt: &str, b: u8) -> bool {
    gpr(rt).is_some_and(|(r, _)| r == b)
}

/// Form post- and pre-indexed accesses (see module docs).
///
/// Post-index: `op rt, [xB]` then, later in the same run, `add|sub xB, xB,
/// #imm` with -256 <= delta <= 255, nothing between reading or writing `xB`,
/// and `rt` not naming `xB`: `op rt, [xB], #delta` -- the access sees the
/// old base and the base ends with the same value.  Pre-index: the
/// increment first and the zero-offset access later (same conditions):
/// `op rt, [xB, #delta]!`; or `op rt, [xB, #delta]` immediately followed by
/// the matching increment.  sp and x29 bases are left alone (frame and
/// unwind bookkeeping track those).
pub(super) fn form_writeback_addressing(
    lines: &mut [String],
    kinds: &mut [LineKind],
    n: usize,
) -> bool {
    let mut changed = false;
    if has_opaque(lines, n) {
        return false;
    }
    for a in 0..n {
        if is_blank(lines, kinds, a) {
            continue;
        }
        // Increment first: a pre-indexed access later.
        let first_is_inc = split_insn(&lines[a])
            .and_then(|(_, ops)| ops.first().and_then(|o| gpr(o)))
            .filter(|&(b, x)| x && b < 29)
            .and_then(|(b, _)| base_increment(&lines[a], b).map(|d| (b, d)));
        if let Some((b, delta)) = first_is_inc {
            if !(-256..=255).contains(&delta) {
                continue;
            }
            for j in a + 1..n {
                if is_blank(lines, kinds, j) {
                    continue;
                }
                if ends_run(lines, kinds, j) {
                    break;
                }
                if let Some((m, rt, base, 0)) = base_imm_access(&lines[j])
                    && base == b
                    && !transfers_base(rt, b)
                {
                    let indent =
                        lines[j][..lines[j].len() - lines[j].trim_start().len()].to_string();
                    let text = format!("{indent}{m} {rt}, [{}, #{delta}]!", xreg_name(b));
                    set_line(lines, kinds, j, text);
                    kinds[a] = LineKind::Nop;
                    changed = true;
                    break;
                }
                let (uses, defs) = gp_operands(&lines[j], kinds[j], false);
                if (uses | defs | may_write_gp(&lines[j])) & (1u64 << b) != 0 {
                    break;
                }
            }
            continue;
        }
        let Some((m, rt, b, off)) = base_imm_access(&lines[a]) else {
            continue;
        };
        if b >= 29 || transfers_base(rt, b) {
            continue;
        }
        let (m, rt) = (m.to_string(), rt.to_string());
        for j in a + 1..n {
            if is_blank(lines, kinds, j) {
                continue;
            }
            if ends_run(lines, kinds, j) {
                break;
            }
            if let Some(delta) = base_increment(&lines[j], b) {
                let indent = lines[a][..lines[a].len() - lines[a].trim_start().len()].to_string();
                let text = if off == 0 && (-256..=255).contains(&delta) {
                    Some(format!("{indent}{m} {rt}, [{}], #{delta}", xreg_name(b)))
                } else if off != 0 && off == delta && (-256..=255).contains(&delta) {
                    Some(format!("{indent}{m} {rt}, [{}, #{delta}]!", xreg_name(b)))
                } else {
                    None
                };
                if let Some(text) = text {
                    set_line(lines, kinds, a, text);
                    kinds[j] = LineKind::Nop;
                    changed = true;
                }
                break;
            }
            let (uses, defs) = gp_operands(&lines[j], kinds[j], false);
            if (uses | defs | may_write_gp(&lines[j])) & (1u64 << b) != 0 {
                break;
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `pass` over one function made of `body` and return the kept
    /// instruction lines.
    fn run(
        pass: fn(&mut [String], &mut [LineKind], usize) -> bool,
        body: &str,
    ) -> (bool, Vec<String>) {
        let asm = format!("f:\n.cfi_startproc\n{body}.cfi_endproc\n");
        let mut lines: Vec<String> = asm.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|l| classify_line(l)).collect();
        let n = lines.len();
        let changed = pass(&mut lines, &mut kinds, n);
        let kept = lines
            .iter()
            .zip(&kinds)
            .filter(|(l, k)| {
                **k != LineKind::Nop && !l.trim().starts_with(".cfi") && l.trim() != "f:"
            })
            .map(|(l, _)| l.trim().to_string())
            .collect();
        (changed, kept)
    }

    fn call(widths: &str) -> String {
        format!("    bl g{CALL_READS_TAG}3 {CALL_NARROW_KEY}{widths}")
    }

    #[test]
    fn narrow_annotation_round_trips() {
        let mut w = [0u8; 8];
        w[0] = 8;
        w[3] = 32;
        let text = format_call_narrow(&w);
        assert_eq!(text, " narrow=0:8,3:32");
        let line = format!("    bl g{CALL_READS_TAG}9{text}");
        assert_eq!(call_narrow_widths(&line), w);
        assert_eq!(format_call_narrow(&[0; 8]), "");
        assert_eq!(call_narrow_widths("    bl g"), [0; 8]);
        // The reads mask still parses with the annotation behind it.
        assert_eq!(super::super::a64_call_reads(&line) & 0xff, 9);
    }

    #[test]
    fn narrow_argument_extensions_feeding_only_the_call_go() {
        for (body, out) in [
            ("    sxtb x0, w0\n", vec![]),
            ("    and w0, w0, #0xff\n", vec![]),
            ("    and x0, x0, #0xffff\n    mov w0, w0\n", vec![]),
            ("    sxtb x0, w5\n", vec!["mov w0, w5"]),
            // An X copy staged for a narrow argument reads only W views.
            ("    mov x0, x19\n", vec!["mov w0, w19"]),
        ] {
            let narrow = if body.contains("0xffff") {
                "0:16,1:32"
            } else {
                "0:8,1:32"
            };
            let text = format!("{body}{}\n    ret\n", call(narrow));
            let (changed, kept) = run(drop_narrow_call_arg_extensions, &text);
            assert!(changed, "{body}");
            assert_eq!(&kept[..kept.len() - 2], out.as_slice(), "{body}");
        }
        // `mov w1, w1` for a 32-bit argument.
        let (changed, kept) = run(
            drop_narrow_call_arg_extensions,
            &format!("    mov w1, w1\n{}\n    ret\n", call("1:32")),
        );
        assert!(changed);
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn narrow_argument_extensions_that_matter_stay() {
        for body in [
            // The argument is 64 bits wide.
            format!("    sxtb x0, w0\n{}\n    ret\n", call("1:32")),
            // A byte extension cannot stand for a 16-bit argument.
            format!("    sxtb x0, w0\n{}\n    ret\n", call("0:16")),
            // Another reader sees the extended value first.
            format!(
                "    sxtb x0, w0\n    mov x1, x0\n{}\n    ret\n",
                call("0:8")
            ),
            // A label in between: another path reaches the call.
            format!("    sxtb x0, w0\n.L1:\n{}\n    ret\n", call("0:8")),
            // Not annotated.
            "    sxtb x0, w0\n    bl g\n    ret\n".to_string(),
            // Already the W copy the rewrite would produce.
            format!("    mov w0, w5\n{}\n    ret\n", call("0:32")),
            // An X copy into a 64-bit argument.
            format!("    mov x1, x19\n{}\n    ret\n", call("0:8")),
        ] {
            let (changed, _) = run(drop_narrow_call_arg_extensions, &body);
            assert!(!changed, "{body}");
        }
    }

    #[test]
    fn forward_copies_reach_later_readers_in_the_block() {
        let (changed, kept) = run(
            forward_block_copies,
            "    mov w7, w1\n    add w4, w1, #4\n    cmp w0, #0\n    csel w0, w4, w7, ne\n    ret\n",
        );
        assert!(changed);
        assert_eq!(kept[3], "csel w0, w4, w1, ne");
        // Stores and compares read their first operand.
        let (_, kept) = run(
            forward_block_copies,
            "    mov x7, x1\n    str x7, [x0]\n    cmp x7, #3\n    ret\n",
        );
        assert_eq!(kept[1..3], ["str x1, [x0]", "cmp x1, #3"]);
    }

    #[test]
    fn forward_copies_stop_where_the_value_changes() {
        for body in [
            // The source is redefined before the reader.
            "    mov w7, w1\n    add w1, w1, #1\n    add w0, w7, #2\n    ret\n",
            // A W copy is not the source's X view.
            "    mov w7, w1\n    add x0, x7, #2\n    ret\n",
            // A writeback of the source.
            "    mov x7, x1\n    ldr x2, [x1], #8\n    add x0, x7, #2\n    ret\n",
            // A label: another path reaches the reader.
            "    mov w7, w1\n.L1:\n    add w0, w7, #2\n    ret\n",
        ] {
            let (changed, kept) = run(forward_block_copies, body);
            assert!(!changed, "{body} => {kept:?}");
        }
    }

    #[test]
    fn cset_retests_read_the_original_condition() {
        let (changed, kept) = run(
            fold_cset_retests,
            "    cmp x0, x1\n    cset x4, lt\n    add x5, x2, #8\n    cmp w4, #0\n    csel x0, x5, x2, ne\n    ret\n",
        );
        assert!(changed);
        assert_eq!(
            kept,
            ["cmp x0, x1", "add x5, x2, #8", "csel x0, x5, x2, lt", "ret"]
        );
        let (changed, kept) = run(
            fold_cset_retests,
            "    cmp w0, w1\n    cset x4, eq\n    cmp w4, #0\n    csel w0, w5, w2, eq\n    ret\n",
        );
        assert!(changed);
        assert_eq!(kept, ["cmp w0, w1", "csel w0, w5, w2, ne", "ret"]);
        let (changed, kept) = run(
            fold_cset_retests,
            "    cmp w0, #5\n    cset w4, hi\n    cbnz w4, .L1\n    ret\n.L1:\n    ret\n",
        );
        assert!(changed);
        assert_eq!(kept, ["cmp w0, #5", "b.hi .L1", "ret", ".L1:", "ret"]);
        let (changed, kept) = run(
            fold_cset_retests,
            "    cmp w0, #5\n    cset w4, hi\n    cbz w4, .L1\n    ret\n.L1:\n    ret\n",
        );
        assert!(changed);
        assert_eq!(kept[1], "b.ls .L1");
    }

    #[test]
    fn cset_retests_that_cannot_be_redirected_stay() {
        for body in [
            // A flag write between the cset and the test.
            "    cmp x0, x1\n    cset x4, lt\n    adds x5, x2, #8\n    cmp w4, #0\n    csel x0, x5, x2, ne\n    ret\n",
            // A reader asking for something other than Z.
            "    cmp x0, x1\n    cset x4, lt\n    cmp w4, #0\n    csel x0, x5, x2, gt\n    ret\n",
            // The flags of the re-test are read after a branch.
            "    cmp x0, x1\n    cset x4, lt\n    cmp w4, #0\n    b.ne .L1\n    csel x0, x5, x2, ne\n    ret\n.L1:\n    ret\n",
            // r is rewritten before the test.
            "    cmp x0, x1\n    cset x4, lt\n    mov x4, #3\n    cmp w4, #0\n    csel x0, x5, x2, ne\n    ret\n",
        ] {
            let (changed, kept) = run(fold_cset_retests, body);
            assert!(!changed, "{body} => {kept:?}");
        }
        // The cset stays when r is still read later; the test still goes.
        let (changed, kept) = run(
            fold_cset_retests,
            "    cmp x0, x1\n    cset x4, lt\n    cmp w4, #0\n    csel x0, x5, x2, ne\n    add x0, x0, x4\n    ret\n",
        );
        assert!(changed);
        assert_eq!(
            kept,
            [
                "cmp x0, x1",
                "cset x4, lt",
                "csel x0, x5, x2, lt",
                "add x0, x0, x4",
                "ret"
            ]
        );
    }

    #[test]
    fn csel_of_one_or_all_ones_is_a_cset() {
        for (body, out) in [
            (
                "    mov x2, #-1\n    cmp w0, #0\n    csel w0, w2, wzr, ne\n",
                "csetm w0, ne",
            ),
            (
                "    mov w2, #1\n    cmp w0, #0\n    csel w0, wzr, w2, ne\n",
                "cset w0, eq",
            ),
            (
                "    movn x2, #0\n    cmp x0, #0\n    csel x0, x2, xzr, lt\n",
                "csetm x0, lt",
            ),
        ] {
            let (changed, kept) = run(fold_csel_constants, &format!("{body}    ret\n"));
            assert!(changed, "{body}");
            assert_eq!(
                kept,
                [
                    "cmp w0, #0"
                        .replace('w', if body.contains("cmp x0") { "x" } else { "w" })
                        .as_str(),
                    out,
                    "ret"
                ],
                "{body}"
            );
        }
        // A W all-ones is not an X all-ones.
        let (changed, _) = run(
            fold_csel_constants,
            "    mov w2, #-1\n    cmp x0, #0\n    csel x0, x2, xzr, ne\n    ret\n",
        );
        assert!(!changed);
        // `-(a < b)` with any condition.
        let (changed, kept) = run(
            fold_csel_constants,
            "    cmp w0, #0\n    cset x0, ge\n    neg w0, w0\n    ret\n",
        );
        assert!(changed);
        assert_eq!(kept, ["cmp w0, #0", "csetm w0, ge", "ret"]);
    }

    #[test]
    fn subtracting_a_non_negative_test_is_a_complement_shift() {
        let (changed, kept) = run(
            fold_csel_constants,
            "    cmp w1, #0\n    cset x4, ge\n    sub w0, w0, w4\n    ret\n",
        );
        assert!(changed);
        assert_eq!(kept, ["mvn w4, w1", "add w0, w0, w4, asr #31", "ret"]);
        // Widths must agree; the flags must die.
        for body in [
            "    cmp x1, #0\n    cset x4, ge\n    sub w0, w0, w4\n    ret\n",
            "    cmp w1, #0\n    cset x4, ge\n    sub w0, w0, w4\n    b.eq .L1\n    ret\n.L1:\n    ret\n",
        ] {
            let (changed, kept) = run(fold_csel_constants, body);
            assert!(!changed, "{body} => {kept:?}");
        }
    }

    #[test]
    fn shifts_fuse_into_their_consumer() {
        for (body, out) in [
            (
                "    lsr w4, w1, #31\n    sub w0, w0, w4\n",
                "sub w0, w0, w1, lsr #31",
            ),
            (
                "    lsl x4, x1, #3\n    add x0, x4, x2\n",
                "add x0, x2, x1, lsl #3",
            ),
            (
                "    asr w4, w1, #2\n    cmp w0, w4\n    cset w0, eq\n",
                "cmp w0, w1, asr #2",
            ),
            // A W lsr of a W-written value into an X add.
            (
                "    ldrb w6, [x0]\n    lsr w7, w6, #7\n    add x4, x4, x7\n    mov x0, x4\n",
                "add x4, x4, x6, lsr #7",
            ),
        ] {
            let (changed, kept) = run(fuse_shift_into_alu, &format!("{body}    ret\n"));
            assert!(changed, "{body}");
            assert!(kept.iter().any(|l| l == out), "{body} => {kept:?}");
            assert!(
                !kept
                    .iter()
                    .any(|l| l.starts_with("lsr") || l.starts_with("lsl") || l.starts_with("asr")),
                "{kept:?}"
            );
        }
    }

    #[test]
    fn shifts_that_cannot_fuse_stay() {
        for body in [
            // Not commutative: the shifted value is the minuend.
            "    lsr w4, w1, #31\n    sub w0, w4, w0\n    ret\n",
            // The shift result is read again.
            "    lsr w4, w1, #31\n    sub w0, w0, w4\n    add w0, w0, w4\n    ret\n",
            // The source changes in between.
            "    lsr w4, w1, #31\n    mov w1, #0\n    sub w0, w0, w4\n    ret\n",
            // An X lsr of unknown upper bits is not a W one.
            "    lsr w7, w6, #7\n    add x4, x4, x7\n    mov x0, x4\n    ret\n",
            // lsl does not widen.
            "    ldrb w6, [x0]\n    lsl w7, w6, #7\n    add x4, x4, x7\n    mov x0, x4\n    ret\n",
            // An sp operand has no shifted-register form.
            "    lsl x4, x1, #3\n    add sp, sp, x4\n    ret\n",
        ] {
            let (changed, kept) = run(fuse_shift_into_alu, body);
            assert!(!changed, "{body} => {kept:?}");
        }
    }

    #[test]
    fn increments_fold_into_post_and_pre_indexed_accesses() {
        for (body, out) in [
            (
                "    ldrb w6, [x0]\n    add w4, w4, w6\n    add x0, x0, #1\n",
                "ldrb w6, [x0], #1",
            ),
            (
                "    ldrsw x8, [x0]\n    add x0, x0, #4\n",
                "ldrsw x8, [x0], #4",
            ),
            ("    str x9, [x6]\n    add x6, x6, #8\n", "str x9, [x6], #8"),
            (
                "    add x0, x0, #1\n    ldrb w5, [x0]\n",
                "ldrb w5, [x0, #1]!",
            ),
            (
                "    ldr w5, [x2, #4]\n    add x2, x2, #4\n",
                "ldr w5, [x2, #4]!",
            ),
            (
                "    ldrh w1, [x3]\n    sub x3, x3, #2\n",
                "ldrh w1, [x3], #-2",
            ),
        ] {
            let (changed, kept) = run(form_writeback_addressing, &format!("{body}    ret\n"));
            assert!(changed, "{body}");
            assert!(kept.iter().any(|l| l == out), "{body} => {kept:?}");
            assert!(
                !kept
                    .iter()
                    .any(|l| l.starts_with("add x") || l.starts_with("sub x")),
                "{kept:?}"
            );
        }
    }

    #[test]
    fn increments_that_cannot_fold_stay() {
        for body in [
            // The base is read in between.
            "    ldrb w6, [x0]\n    mov x1, x0\n    add x0, x0, #1\n    ret\n",
            // The transferred register is the base.
            "    ldr x0, [x0]\n    add x0, x0, #8\n    ret\n",
            // Out of imm9 range.
            "    ldr x1, [x0]\n    add x0, x0, #256\n    ret\n",
            // A label in between.
            "    ldr x1, [x0]\n.L1:\n    add x0, x0, #8\n    ret\n",
            // A nonzero offset with a different increment.
            "    ldr x1, [x0, #8]\n    add x0, x0, #16\n    ret\n",
            // The stack pointer.
            "    ldr x1, [sp]\n    add sp, sp, #16\n    ret\n",
            // A register-offset access.
            "    ldrb w1, [x0, x5]\n    add x0, x0, #1\n    ret\n",
        ] {
            let (changed, kept) = run(form_writeback_addressing, body);
            assert!(!changed, "{body} => {kept:?}");
        }
    }
}
