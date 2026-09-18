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
use super::helpers::{get_dest_reg, has_implicit_reg_usage, is_shift_or_rotate, writes_family};
use super::liveness::FileLiveness;
use super::relay_and_lea::is_full_write;

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
        // Inline-asm region boundary: user bytes are immutable (a template
        // may contain a deliberate `movq %rax, %rax` length placeholder —
        // exactly the shape this pass deletes), and a template can redefine
        // any register without naming it (raw `.byte` encodings), so no
        // upper-32-zero fact may cross the region in either direction.
        if infos[i].kind == LineKind::InlineAsm {
            upper32_zero.iter_mut().for_each(|v| *v = false);
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

/// Parse `movz{b,w,l}{l,q}` / `movs{b,w}{w,l,q}` (zero/sign extension) with
/// both operands plain registers. Returns `(src_width, src_family,
/// dst_family)` — the SOURCE width is what consumers may read, because an
/// extension preserves the source lane bit-for-bit and the upper bits are
/// the extension's own product. `movsd`/`movss` (scalar FP moves) and the
/// string ops (`movsb`, …) are rejected by the two-width-suffix shape and
/// the register-operand requirement.
fn parse_reg_to_reg_ext(trimmed: &str) -> Option<(usize, RegId, RegId)> {
    let rest = if let Some(r) = trimmed.strip_prefix("movz") {
        r
    } else if let Some(r) = trimmed.strip_prefix("movs") {
        r
    } else {
        return None;
    };
    let (suffix, operands) = rest.split_once(' ')?;
    if suffix.len() != 2 {
        return None;
    }
    let sw = match suffix.as_bytes()[0] {
        b'b' => W8,
        b'w' => W16,
        b'l' => W32,
        _ => return None,
    };
    let dw = match suffix.as_bytes()[1] {
        b'w' => W16,
        b'l' => W32,
        b'q' => W64,
        _ => return None,
    };
    // Width indices count DOWN: W64=0 .. W8=3, so a genuine extension has a
    // numerically SMALLER destination index than its source index.
    if dw >= sw {
        return None; // not an extension
    }
    let (src_part, dst_part) = operands.split_once(',')?;
    let src = src_part.trim();
    let dst = dst_part.trim();
    if !src.starts_with('%') || !dst.starts_with('%') || src.contains('(') || dst.contains('(') {
        return None;
    }
    // The source must be spelled at the mnemonic's source width, the
    // destination at its (wider) destination width.
    let sfam = REG_NAMES[sw].iter().position(|n| *n == src)? as RegId;
    let dfam = REG_NAMES[dw].iter().position(|n| *n == dst)? as RegId;
    Some((sw, sfam, dfam))
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
        // Either a plain same-width register copy (consumers may read at the
        // copy's width) or a zero/sign extension (consumers may read only at
        // the SOURCE width: the extension's upper bits are its own product,
        // but the low lane is preserved bit-for-bit — so a `movzbl %dil, %r9d`
        // whose only consumer is `movb %r9b, …` folds the store straight
        // onto `%dil`, which is exactly the shape GCC emits for byte
        // truncation stores).
        let (rule_width, sfam, dfam) =
            if let Some((width, sfam, dfam)) = parse_reg_to_reg_mov(trimmed) {
                (width, sfam, dfam)
            } else if let Some((sw, sfam, dfam)) = parse_reg_to_reg_ext(trimmed) {
                (sw, sfam, dfam)
            } else {
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
            // SOUNDNESS: user inline asm is opaque. A template line can name
            // the copied register in a use position (the acceptance rules
            // below would then REWRITE the user's instruction — user bytes
            // are immutable) and can redefine any register through raw
            // encodings the write oracle cannot see. End the use range.
            if infos[j].kind == LineKind::InlineAsm {
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
            // copy).  A PURE full-width redefinition (`movq %rax, %r9`,
            // `movzbl %al, %r9d` — a mov-class destination whose source
            // half does not read %D) is NOT a violation: it ends the copy's
            // live range exactly like Rule 3's write-without-mention, and
            // the fold stays sound because the redefinition is not renamed
            // (only the uses before it are).  This is the loop-induction
            // shape: `movq %r10, %r9; load (%base,%r9,8); load
            // (%base2,%r9,8); movq %rax, %r9` — the staging copy feeds the
            // two loads and dies at the redefinition (linux_find_bit's
            // bitmap scan: 9 insns/word vs GCC's 7).
            if writes_family(&infos[j], line, dfam) {
                if is_full_write(&infos[j], line, dfam) {
                    break;
                }
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
            if !mentions_only_within(line, dfam, rule_width) {
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
            let rewritten = rename_within(&line, dfam, sfam, rule_width);
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

// ── C. dead in-place extensions ─────────────────────────────────────────────

/// Delete an in-place extension (`cltq`, `movslq %eax, %rax`) whose widened
/// bits are never read: every family-0 mention until the next full-width
/// write (or a call, which de-facto kills the caller-saved product) spells
/// the register at 32 bits or narrower.
///
/// The accumulator-based backend stages narrow arithmetic into `%eax` and
/// routinely appends `cltq` "to be safe" before a narrow store
/// (`leal (%eax,%eax,2), %eax; cltq; movl %eax, (%r10)` — csv/rbtree main):
/// the store reads only `%eax`, so the extension is dead weight, and it is
/// additionally a window barrier for the LEA→memory fold
/// ([`has_implicit_reg_usage`] aborts the scan at `cltq`).
pub(super) fn eliminate_dead_inplace_ext(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        let t = infos[i].trimmed(store.get(i));
        let is_inplace_ext = t == "cltq" || t == "movslq %eax, %rax";
        if !is_inplace_ext {
            i += 1;
            continue;
        }
        let mut dead = false;
        let mut j = i + 1;
        while j < len {
            if infos[j].is_nop() {
                j += 1;
                continue;
            }
            let l = infos[j].trimmed(store.get(j));
            // Calls must be examined BEFORE the generic barrier break:
            // LineKind::Call is a barrier, but %rax is caller-saved — any
            // value the program could read there after the call belongs to
            // the callee's return, not to the extension, so the widened
            // bits are already unreachable. An INDIRECT call names its
            // target (`callq *%rax`) and reads full width: keep.
            if matches!(infos[j].kind, LineKind::Call) {
                dead = !contains_reg(l, "%rax");
                break;
            }
            if infos[j].is_barrier() || infos[j].pinned {
                break;
            }
            if infos[j].kind == LineKind::InlineAsm {
                break;
            }
            // Opaque/implicit-register instructions touch %rax in ways the
            // textual width rule cannot see (idiv/cqto/mul/shld/…): keep.
            if has_implicit_reg_usage(l) {
                break;
            }
            if contains_reg(l, "%rax") {
                // Full-width mention: either a 64-bit read (keep) or a
                // full-width write (the product is overwritten: dead).
                dead = writes_family(&infos[j], l, 0);
                break;
            }
            // Family-0 mentions at ≤32 bits (movl/leal/cmpl on %eax,
            // `(%rax)`-free address operands) or no mention at all: the
            // widened bits stay unread so far.
            j += 1;
        }
        if dead {
            mark_nop(&mut infos[i]);
            changed = true;
        }
        i += 1;
    }
    changed
}

// ── D. induction copy-back folds ────────────────────────────────────────────

/// The producer whose destination feeds a copy straight back into one of
/// its own inputs (or into an unrelated base register).
enum CopybackProducer<'a> {
    /// `mov{q,l} MEM, %T` — `MEM` may mention `%D` (the classic
    /// `movq 8(%rsi), %rax; movq %rax, %rsi` chain-walk update).
    Load { is_q: bool, mem: &'a str },
    /// `leaq $N(%B), %T` with the copy target `%D` — when `%B == %D` the
    /// redirected form becomes `addq $N, %D`; otherwise it is a plain
    /// destination redirect `leaq $N(%B), %D`.
    Lea { disp: &'a str, base: RegId },
}

/// Parse the producer line for a copy-back fold.
///
/// Accepted shapes (the destination must be a plain GP register `%T`):
/// * `movq MEM, %T` / `movl MEM, %T` — a full or zero-extending load
///   (a `movl` producer is only sound with a `movq` copy, which the
///   caller enforces: the copy then transfers exactly the bits the load
///   defined, zeros above 32 included).
/// * `leaq $N(%B), %T` — the induction bump whose result is copied back
///   into (usually) its own base.
fn parse_copyback_producer(trimmed: &str) -> Option<(CopybackProducer<'_>, RegId)> {
    if let Some(rest) = trimmed
        .strip_prefix("movq ")
        .or(trimmed.strip_prefix("movl "))
    {
        let is_q = trimmed.starts_with("movq ");
        let (src, dst) = rest.split_once(',')?;
        let src = src.trim();
        let dst = dst.trim();
        // Memory source only (register copies are fold_register_copies'
        // job) and a plain GP destination of the load's own width.
        if !src.contains('(') || !src.ends_with(')') || src.starts_with('%') {
            return None;
        }
        let fam = plain_reg_family(dst)?;
        let expect = if is_q {
            REG_NAMES[W64][fam as usize]
        } else {
            REG_NAMES[W32][fam as usize]
        };
        if dst != expect {
            return None;
        }
        return Some((CopybackProducer::Load { is_q, mem: src }, fam));
    }
    if let Some(rest) = trimmed.strip_prefix("leaq ") {
        let (src, dst) = rest.split_once(',')?;
        let dst = dst.trim();
        let fam = plain_reg_family(dst)?;
        if dst != REG_NAMES[W64][fam as usize] {
            return None;
        }
        // `disp(%base)` — plain base, no index/scale (the induction bump).
        let src = src.trim();
        let Some(open) = src.find('(') else {
            return None;
        };
        if !src.ends_with(')') {
            return None;
        }
        let disp = src[..open].trim();
        let base_str = &src[open + 1..src.len() - 1];
        let base_str = base_str.split(',').next().unwrap_or(base_str).trim();
        let base = plain_reg_family(base_str)?;
        return Some((CopybackProducer::Lea { disp, base }, fam));
    }
    None
}

/// Resolve a plain GP register name (`%rsi`, `%r10d`, …) to its family.
fn plain_reg_family(op: &str) -> Option<RegId> {
    let name = op.strip_prefix('%')?;
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    family_of_reg_name(name)
}

/// Fold producer→copy-back shapes where the copy's destination is one of
/// the producer's own inputs (or an unrelated base) and the copy is the
/// producer result's only bridge into that register.
///
/// ```text
///     movq 8(%rdx), %rax        movq 8(%rdx), %rdx
///     movq %rax, %rdx      ->   testq %rdx, %rdx        (uses rewritten)
///     testq %rax, %rax
/// ```
///
/// ```text
///     leaq 8(%rsi), %rax        addq $8, %rsi
///     movq %rax, %rsi      ->   cmpq %r8, %rsi
///     cmpq %r8, %rax
/// ```
///
/// SOUNDNESS (both forms share one argument): with the producer at `i`,
/// the copy at `c` and the last rewritten use at `k`,
///
/// 1. No mention of `%T` OR `%D` in `(i, c)`. The redirected producer
///    writes `%D` EARLY, so a read of the old `%D` there would see the
///    loaded/bumped value instead (`%D`'s original value is destroyed
///    before the copy that used to preserve it); a read of `%T` there
///    would see a value the redirected producer never wrote; any write
///    diverges both directions at once.
/// 2. In `(c, k]`: no write of `%D` (it must still hold the producer's
///    value when the rewritten uses read it), no write of `%T` (a write
///    starts a fresh live range the copy no longer feeds), no barriers,
///    no implicit-register instructions, no shifts/rotates, and no
///    inline asm — identical to [`fold_register_copies`] rules 2/4/6 with
///    the roles of source and destination both played by live registers.
/// 3. Every use of `%T` in `[c, k]` is rewritten to `%D`; uses of `%D`
///    there already read the right value in both semantics.
/// 4. `%T` is dead after `k` ([`FileLiveness`]).
/// 5. The copy is `movq` (full width): every bit `%D` receives from `%T`
///    is reproduced by the redirected producer — a `movq` load defines
///    all 64, a `movl` load zero-extends to 64, a `leaq` defines all 64.
///    `movl` producers are therefore only paired with `movq` copies; a
///    `movl` copy would leave `%D`'s upper 32 bits stale in the original
///    and zero in the fold (or vice versa), so the shape is declined.
/// 6. Never `%rsp`/`%rbp`, and `xchg`-class or implicit-destination
///    producers are excluded by the parser (only `mov`/`lea` are read).
pub(super) fn fold_induction_copyback(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    if len < 3 {
        return false;
    }
    let mut lv = FileLiveness::new(store, infos);
    let mut changed = false;

    for i in 0..len.saturating_sub(1) {
        if infos[i].is_nop() || infos[i].is_barrier() || infos[i].pinned {
            continue;
        }
        let producer_line = infos[i].trimmed(store.get(i)).to_string();
        let Some((producer, t_fam)) = parse_copyback_producer(&producer_line) else {
            continue;
        };

        // Find the copy: scan forward through the pre-window (lines that
        // are transparent or pure reads of %T; any mention of the copy
        // target family or a barrier ends the search — the copy-back
        // staging shape keeps them close). The first plain `movq %T, %D`
        // whose destination differs from %T is the candidate; its
        // destination becomes the fold target %D.
        let mut c = None;
        let mut d_fam_opt: Option<RegId> = None;
        {
            let mut j = i + 1;
            let mut steps = 0;
            while j < len && steps < 8 {
                if infos[j].is_nop() || matches!(infos[j].kind, LineKind::Directive) {
                    j += 1;
                    continue;
                }
                steps += 1;
                if infos[j].is_barrier() || infos[j].kind == LineKind::InlineAsm || infos[j].pinned
                {
                    break;
                }
                let line = infos[j].trimmed(store.get(j));
                if let Some((copy_w, cs, cd)) = parse_reg_to_reg_mov(line) {
                    if copy_w == W64
                        && cs == t_fam
                        && cd != t_fam
                        && !is_frame_family(cd)
                        && get_dest_reg(&infos[j]) == cd
                    {
                        // Candidate copy — but only accept it if no line
                        // between the producer and here mentioned %D (the
                        // old %D must not be observed; checked inline below
                        // via the mentions scan in this loop).
                        c = Some(j);
                        d_fam_opt = Some(cd);
                        break;
                    }
                    // A different copy shape: it mentions %T as a pure read
                    // only if it is not a write of %T; treat like any
                    // other pre-window line below.
                }
                // Any mention of a register that could be the target is
                // handled by the main rule-1 scan after the copy is found;
                // here we only need to know that the region is scannable.
                // (The copy target is unknown until the copy is found, so
                // track candidates conservatively: a line mentioning %T
                // must be a pure read.)
                if infos[j].reg_refs & (1u16 << t_fam) != 0 {
                    if writes_family(&infos[j], line, t_fam) {
                        break;
                    }
                }
                j += 1;
            }
        }
        let (Some(c), Some(d_fam)) = (c, d_fam_opt) else {
            continue;
        };
        let copy_line = infos[c].trimmed(store.get(c)).to_string();
        let Some((_, cs, cd)) = parse_reg_to_reg_mov(&copy_line) else {
            continue;
        };
        if cs != t_fam || cd != d_fam || is_frame_family(cs) {
            continue;
        }
        let t_bit = 1u16 << t_fam;
        let d_bit = 1u16 << d_fam;

        // Rule 1 window (producer, copy): instructions mentioning `%T`
        // must be PURE READS — they are rewritten to `%D` (with the
        // redirect, `%D` holds the producer's value from the producer
        // onward, so the read observes the same bits). Any WRITE of `%T`
        // starts a range the copy no longer feeds; any mention of `%D`
        // other than the copy itself reads the OLD `%D` (destroyed by the
        // redirect) or writes it — both diverge, so the fold declines.
        // Barrier/CFC lines also end the window (the copy must stay in
        // the same straight-line region as its producer).
        let mut pre_uses: Vec<usize> = Vec::new();
        let mut j = i + 1;
        let mut clean = true;
        while j < c {
            if infos[j].is_nop() || matches!(infos[j].kind, LineKind::Directive) {
                j += 1;
                continue;
            }
            if infos[j].is_barrier() || infos[j].kind == LineKind::InlineAsm || infos[j].pinned {
                clean = false;
                break;
            }
            let line = infos[j].trimmed(store.get(j));
            let mentions_t = infos[j].reg_refs & t_bit != 0;
            let mentions_d = infos[j].reg_refs & d_bit != 0;
            if mentions_d {
                clean = false;
                break;
            }
            if mentions_t {
                if writes_family(&infos[j], line, t_fam)
                    || writes_family(&infos[j], line, d_fam)
                    || has_implicit_reg_usage(line)
                    || is_shift_or_rotate(line)
                {
                    clean = false;
                    break;
                }
                pre_uses.push(j);
            }
            j += 1;
        }
        if !clean {
            continue;
        }

        // Collect the uses of %T after the copy (rules 2/3), tracking the
        // last one. The scan ends at the first line that breaks the
        // equivalence window between %T and %D:
        //   * ANY write of `%T` — full (`movq %rcx, %rax`), read-write
        //     (`addq %rcx, %rax`), or implicit (`cqto`-class) — starts a
        //     NEW live range of `%T`. Post-write mentions of `%T` read the
        //     new value, not the copy's, so rewriting them to `%D` would
        //     feed them the stale copied value (miscompile: the
        //     cmp_replay_acc_nohome `leaq 8(%rax),%rax; movq %rax,%r8;
        //     movq (%rax),%rax` chain — the load's destination write made
        //     every later `%rax` read the loaded pointer, and the fold
        //     pointed them at the pre-bump value). The line itself is NOT
        //     collected either: it reads the never-written `%T` under the
        //     fold, and the `live_after` check at the end then sees `%T`
        //     still live at that point and rejects the whole fold —
        //     exactly the fail-closed behaviour the shape demands.
        //   * a write of `%D` without a `%T` read — `%D` starts a new
        //     value, so later `%T` uses are beyond the fold (break);
        //   * a line that READS `%T` while also writing `%D` (`addq %rax,
        //     %rdx`) — rewriting gives `%rdx += %rdx` and not rewriting
        //     reads the stale, never-written `%T`: both directions
        //     miscompile, so the whole fold is rejected (ok = false);
        // Uses of `%D` in this window need no rewrite — `%D` already holds
        // the producer's value under the fold.
        let mut uses: Vec<usize> = Vec::new();
        let mut last = c;
        let mut ok = true;
        for j in (c + 1)..len {
            if infos[j].is_nop() {
                continue;
            }
            if infos[j].is_barrier() || infos[j].kind == LineKind::InlineAsm {
                break;
            }
            let line = infos[j].trimmed(store.get(j));
            let mentions_t = infos[j].reg_refs & t_bit != 0;
            let writes_d = writes_family(&infos[j], line, d_fam);
            let writes_t = writes_family(&infos[j], line, t_fam);
            if mentions_t {
                if writes_d {
                    ok = false;
                    break;
                }
                if writes_t {
                    break;
                }
                if has_implicit_reg_usage(line) || is_shift_or_rotate(line) {
                    ok = false;
                    break;
                }
                uses.push(j);
                last = j;
            } else {
                if writes_d || writes_t {
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        // Rule 4: T dead after the last collected mention (either window;
        // with no uses at all, dead right after the copy).
        let mut all_uses: Vec<usize> = pre_uses;
        all_uses.extend_from_slice(&uses);
        let dead_at = if all_uses.is_empty() {
            c
        } else {
            *all_uses.last().unwrap()
        };
        if lv.live_after(dead_at, t_fam) != Some(false) {
            continue;
        }

        // Apply: redirect the producer destination to %D (preserving the
        // load's own width — a `movl` producer stays a 4-byte read into
        // %D's 32-bit name, whose zero-extension is exactly the bits the
        // `movq` copy used to transfer), delete the copy, rewrite every
        // collected mention of %T to %D at the mention's own width.
        let new_producer = match &producer {
            CopybackProducer::Load { is_q: true, mem } => {
                format!("    movq {}, {}", mem, REG_NAMES[W64][d_fam as usize])
            }
            CopybackProducer::Load { is_q: false, mem } => {
                format!("    movl {}, {}", mem, REG_NAMES[W32][d_fam as usize])
            }
            CopybackProducer::Lea { disp, base } => {
                if *base == d_fam {
                    // `leaq N(%D), %D` == `addq $N, %D` (shorter encoding,
                    // GCC's exact spelling for the induction bump). The LEA
                    // displacement text carries no `$` — the immediate form
                    // needs one, or the assembler reads it as an absolute
                    // memory operand.
                    format!("    addq ${}, {}", disp, REG_NAMES[W64][d_fam as usize])
                } else {
                    format!(
                        "    leaq {}({}), {}",
                        disp, REG_NAMES[W64][*base as usize], REG_NAMES[W64][d_fam as usize]
                    )
                }
            }
        };
        // Pre-compute every use's rewritten form first: a use that mentions
        // %T in a spelling the width renaming cannot express (or that the
        // rewrite leaves still mentioning %T) aborts the whole fold — the
        // copy is those uses' only definition bridge.
        let t_names: Vec<(String, String)> = (0..4)
            .map(|w| {
                (
                    REG_NAMES[w][t_fam as usize].to_string(),
                    REG_NAMES[w][d_fam as usize].to_string(),
                )
            })
            .collect();
        let mut rewritten_uses: Vec<(usize, String)> = Vec::with_capacity(all_uses.len());
        let mut all_rewritable = true;
        for &j in &all_uses {
            let line = infos[j].trimmed(store.get(j)).to_string();
            let mut rw = line.clone();
            for (from, to) in &t_names {
                rw = replace_reg(&rw, from, to);
            }
            if rw == line || t_names.iter().any(|(from, _)| contains_reg(&rw, from)) {
                all_rewritable = false;
                break;
            }
            rewritten_uses.push((j, rw));
        }
        replace_line(store, &mut infos[i], i, new_producer);
        mark_nop(&mut infos[c]);
        for (j, rw) in rewritten_uses {
            replace_line(store, &mut infos[j], j, format!("    {}", rw.trim_start()));
        }
        lv.refresh_at(store, infos, dead_at);
        changed = true;
    }

    changed
}

#[cfg(test)]
#[path = "narrow_copy_fold_tests.rs"]
mod tests;

#[cfg(test)]
mod inline_asm_tests {
    use super::*;
    use crate::backend::peephole_common::LineStore;

    fn build_pinned(asm: &str) -> (LineStore, Vec<LineInfo>) {
        let store = LineStore::new(asm.to_string());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let mut in_asm = false;
        for i in 0..infos.len() {
            let t = infos[i].trimmed(store.get(i)).to_string();
            if t == "#APP" {
                in_asm = true;
                continue;
            }
            if t == "#NO_APP" {
                in_asm = false;
                continue;
            }
            if in_asm {
                infos[i] = LineInfo {
                    kind: LineKind::InlineAsm,
                    ext_kind: ExtKind::None,
                    trim_start: infos[i].trim_start,
                    has_indirect_mem: true,
                    rbp_offset: RBP_OFFSET_NONE,
                    reg_refs: u16::MAX,
                    pinned: true,
                };
            }
        }
        (store, infos)
    }

    #[test]
    fn copy_use_range_ends_at_inline_asm() {
        // A template line that names the copied register must not be
        // rewritten, and the copy itself must survive (its use still reads
        // it).
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl %edi, %eax\n",
            "    movq %rax, %r10\n",
            "#APP\n",
            "    movq %r10, %r11\n",
            "#NO_APP\n",
            "    addq %r10, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        fold_register_copies(&mut store, &mut infos);
        let out: Vec<String> = (0..store.len()).map(|i| store.get(i).to_string()).collect();
        assert!(
            out.iter().any(|l| l.trim() == "movq %r10, %r11"),
            "user asm bytes must be immutable; got:\n{}",
            out.join("\n")
        );
    }

    #[test]
    fn deliberate_self_move_inside_asm_region_survives() {
        // Kernel ALTERNATIVE() length placeholder: `movq %rax, %rax` inside
        // a region is deliberate padding, not a dead move.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "#APP\n",
            "    movq %rax, %rax\n",
            "#NO_APP\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        let _ = fold_register_copies(&mut store, &mut infos);
        assert!(
            (0..store.len()).any(|i| store.get(i).trim() == "movq %rax, %rax"),
            "the placeholder must reach the assembler"
        );
    }
}
