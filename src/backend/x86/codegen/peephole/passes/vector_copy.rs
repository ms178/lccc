//! Vector register copy elimination: bracket coalescing, propagation, dead copies.
//!
//! The codegen routes every scalar FP value through a fresh temporary, so the
//! assembly text carries register copies that no existing pass can see.
//! `copy_propagation` parses `movq`/`movl`, `coalesce_register_copies` requires
//! a literal `movq ` prefix, `relay_and_lea` rejects anything that is not a
//! `plain_gp_operand`, and `dead_writes` recognises GP pure writes only — all
//! four bail out on a vector family by construction (`is_xmm_family` exists to
//! keep them away from the GP `reg_refs` bitmask). The GP side of the same
//! problem is already solved, which is what makes the asymmetry measurable:
//!
//! ```text
//! long  gpr(long a, long b) { long t = a; t += b; return t; }   →  addq %rsi, %rdi
//!                                                                  movq %rdi, %rax
//! double fp (double a, double b){ double t = a; t += b; return t;} →  movsd %xmm0, %xmm2
//!                                                                  movsd %xmm1, %xmm3
//!                                                                  vaddsd %xmm3, %xmm2, %xmm2
//!                                                                  movsd %xmm2, %xmm0
//! ```
//!
//! Four instructions where GCC 16.2, Clang 23.1 and ICX all emit ONE
//! (`vaddsd %xmm1, %xmm0, %xmm0`). The same shape explains the largest
//! single-compiler gaps in the archived oracle ranking at
//! `engineering/evidence/godbolt/s59-rank` (51 benchmarks against gcc 16.2,
//! clang 23.1.0, icc 2021.10.0 and icx, all at `-O2 -march=x86-64-v3`):
//! `libm_round_family` spends 30 of its 234 instructions on vector register
//! copies, and `__builtin_floor` — which this compiler already lowers to a
//! single `vroundsd $9`, correctly, even when the translation unit defines its
//! own `floor` — reaches the assembler as three instructions because of the
//! copy-in/copy-out bracket around it:
//!
//! ```text
//! movsd  %xmm0, %xmm2          vroundsd $9, %xmm0, %xmm0, %xmm0
//! vroundsd $9, %xmm2, %xmm2, %xmm2      →       ret
//! movsd  %xmm2, %xmm0
//! ret
//! ```
//!
//! ICX emits exactly the right-hand side. `__builtin_copysign` reaches 8
//! instructions against Clang's 3 (two of them a copy pair that cancels:
//! `movsd %xmm0, %xmm4` immediately followed by `movsd %xmm4, %xmm0`), and
//! `__builtin_fma` 7 against ICX's single `vfmadd213sd`.
//!
//! # What this pass does
//!
//! Six transformations, in the order that converges fastest:
//!
//! * [`widen_private_vector_copies`] — a legacy narrow copy (`movsd`, `movss`)
//!   into a temporary nothing has defined yet becomes the full-width move of the
//!   same flavour, so the packed reads that follow it can be served from the
//!   source.  This is what makes copysign's copies reachable at all: a 64-bit
//!   copy cannot feed a 128-bit `vandpd`, and widening it is sound precisely
//!   because the bits it does not define were nobody's.
//! * [`reassociate_fma_accumulator`] — the one bracket no other rule can serve,
//!   because every FMA form reads its own destination as one of its roles.
//!   Re-encoding the instruction (132, 213 or 231, whichever makes the wanted
//!   register the destination — the algebra lives in [`super::fma_forms`])
//!   puts the result where the copy-out wanted it, which is what
//!   `__builtin_fma` and `a * b + c` need to reach one instruction.
//! * [`retarget_vex_result`] — a VEX producer (scalar `ss`/`sd` or packed
//!   `ps`/`pd`) whose result is only copied elsewhere writes that register
//!   directly; unlike an FMA its destination is a plain output.
//! * [`coalesce_vector_brackets`] — the copy-in / ops / copy-out bracket above.
//!   The temporary is private to the bracket, so the destination family is
//!   renamed to the source family and both copies disappear.
//! * [`propagate_vector_copies`] — forward substitution of a copy's source into
//!   the reads of its destination within a basic block, width-checked. This is
//!   what turns `movsd %xmm1, %xmm3` + `vaddsd %xmm3, …` into a direct read and
//!   leaves the copy dead.
//! * [`eliminate_dead_vector_copies`] — a vector copy whose destination cannot
//!   be observed again, asked of [`FpLiveness`] rather than a textual scan.
//!
//! # Why it is legal
//!
//! A vector register copy is not a GP register copy, and the difference is the
//! bits ABOVE the ones it moves. Legacy `movsd %xmmA, %xmmB` copies 64 bits and
//! leaves B's upper half as it was; the VEX spelling zeroes everything above
//! them; `movapd` writes the whole 128. Every rule below exists to keep those
//! bits honest.
//!
//! 1. **The copy is a plain register-to-register move.** The blend forms
//!    (`vmovhpd`, `vmovlps`), the duplicating forms (`vmovddup`), broadcasts and
//!    the three-operand `vmovsd %xmmA, %xmmB, %xmmD` — which takes D's upper
//!    lane from B, so it is not a copy at all — are rejected by the mnemonic
//!    table, not by a heuristic.
//! 2. **Nothing in the bracket touches the source family.** A read would see a
//!    value the rewrite changes; a write would clobber the coalesced one.
//! 3. **Every mention of the destination inside the bracket is at most as wide
//!    as the copy.** A narrow copy defines 64 bits; an interior `vaddpd %xmm2`
//!    reads 128, and 64 of those bits are not the source's.
//! 4. **The destination is private to the bracket** — mentioned nowhere else in
//!    the function. That is what makes its incoming upper bits undefined (they
//!    are a caller-saved register's leftovers, so giving them the source's value
//!    is a refinement, not a change) and what makes deleting the copy-out safe
//!    without a liveness query.
//! 5. **No read of the source after the bracket is wider than the copy-out.**
//!    A legacy narrow copy-out preserved the source's upper bits; after the
//!    rewrite those bits hold whatever the bracket's last write left, and a VEX
//!    scalar write zeroes them. Converting a `double` in `%xmm0` to a vector
//!    read of `%xmm0` later in the function is exactly the case this forbids.
//! 6. **No barrier between the copies.** Labels and branches mean the bracket
//!    may be entered elsewhere; a `call` clobbers every vector register (all 16
//!    are caller-saved under SysV) and reads `%xmm0`–`%xmm7` as arguments; a
//!    `ret` reads `%xmm0`/`%xmm1`; inline assembly is opaque; `vzeroupper`
//!    rewrites the upper half of every register. Pinned lines are never
//!    rewritten or deleted.
//! 7. **Directives are barriers unless they are provably not code.** `.cfi_*`,
//!    `.p2align` and friends are skipped; anything else (`.rept`, `.irp`,
//!    `.macro`) could emit instructions this pass cannot see.
//!
//! Propagation is weaker and needs less: within one basic block, a copy makes
//! the source and destination hold the same value for the bits it copied, so a
//! later read no wider than the copy may name the source instead. The relation
//! dies at the first write to either register and at any barrier. Reads are
//! rewritten but the destination operand is left alone — `vaddsd %xmm2, %xmm1,
//! %xmm2` becomes `vaddsd %xmm0, %xmm1, %xmm2`, which is the same value in the
//! same place and ends the relation.

use super::super::types::*;
use super::fp_liveness::{FpLiveness, has_implicit_vector_effects_line};
use super::relay_and_lea::function_range;

/// Bit widths a vector instruction can read or write.
const W32: u16 = 32;
const W64: u16 = 64;
const W128: u16 = 128;
const W256: u16 = 256;
const W512: u16 = 512;

/// A vector register-to-register copy and the two facts a rewrite needs: how
/// many bits it defines, and what happens to the bits above them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VecCopy {
    src: u8,
    dst: u8,
    /// Bits of the destination this copy defines.
    width: u16,
    /// `true` when the bits above [`Self::width`] are zeroed (VEX/EVEX) rather
    /// than left as they were (legacy SSE). Only the legacy narrow forms
    /// preserve them, and rule 5 is about exactly that difference.
    zeroes_above: bool,
}

/// Split a line into its mnemonic and its operand text.
fn split_mn(t: &str) -> (&str, &str) {
    match t.split_once(char::is_whitespace) {
        Some((mn, rest)) => (mn, rest.trim_start()),
        None => (t, ""),
    }
}

/// Split an operand list on top-level commas. Parentheses (SIB operands) and
/// braces (EVEX writemasks, broadcasting, embedded rounding) nest, and a comma
/// inside either is not an operand separator: `vmovupd (%rdi,%rsi,4), %xmm1` is
/// two operands, not four.
fn operand_list(operands: &str) -> Vec<&str> {
    let mut out = Vec::with_capacity(3);
    let mut depth = 0u32;
    let mut start = 0usize;
    for (i, c) in operands.char_indices() {
        match c {
            '(' | '{' => depth += 1,
            ')' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(operands[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    let tail = operands[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

/// Drop an EVEX decoration from an operand (`%zmm3{%k1}{z}` → `%zmm3`,
/// `%zmm1{1to8}` → `%zmm1`) so the register underneath can be parsed.
fn strip_evex(op: &str) -> &str {
    match op.find('{') {
        Some(i) => op[..i].trim_end(),
        None => op,
    }
}

/// The vector register a whole operand names, with the width its SPELLING
/// denotes. `None` for memory operands, immediates, GP registers and the
/// AVX-512 registers (16..31) this pass does not track — those are distinct
/// physical registers, so declining them cannot hide a tracked one.
fn vec_operand(op: &str) -> Option<(u8, u16)> {
    let op = strip_evex(op);
    // A comment or a second token in the operand means this pass is looking at
    // text it does not model; refuse rather than guess.
    if op.contains('#') || op.contains(',') {
        return None;
    }
    let (digits, width) = if let Some(r) = op.strip_prefix("%xmm") {
        (r, W128)
    } else if let Some(r) = op.strip_prefix("%ymm") {
        (r, W256)
    } else if let Some(r) = op.strip_prefix("%zmm") {
        (r, W512)
    } else {
        return None;
    };
    if digits.is_empty() || digits.len() > 2 || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: u16 = digits.parse().ok()?;
    (n < 16).then_some((n as u8, width))
}

/// The plain register-to-register vector moves, as (bits copied, zeroes above).
///
/// `None` for everything that is not a copy of one register into another: the
/// blends (`vmovhpd`, `vmovlps`, `vmovhlps`), the duplicating forms
/// (`vmovddup`, `vmovshdup`), the broadcasts, the conversions, and the
/// three-operand `vmovsd`/`vmovss` merge, whose destination takes its upper
/// lane from a SECOND source register.
fn vec_copy_mnemonic(mn: &str) -> Option<(Option<u16>, bool)> {
    let (legacy, base) = match mn.strip_prefix('v') {
        Some(rest) => (false, rest),
        None => (true, mn),
    };
    match base {
        // Scalar lane copies: 64 bits for `sd`, 32 for `ss`.
        "movsd" => Some((Some(W64), !legacy)),
        "movss" => Some((Some(W32), !legacy)),
        // Whole-register moves. Legacy forms leave the bits above the register
        // width alone; VEX/EVEX forms zero them.
        "movapd" | "movaps" | "movupd" | "movups" | "movdqa" | "movdqu" | "lddqu" | "movdqa32"
        | "movdqa64" | "movdqu32" | "movdqu64" => Some((None, !legacy)),
        // `movq`/`movd` between vector registers zero-extend into the rest of
        // the destination in both encodings.
        "movq" => Some((Some(W64), true)),
        "movd" => Some((Some(W32), true)),
        _ => None,
    }
}

/// Parse a line as a vector register-to-register copy.
fn parse_vec_copy(t: &str) -> Option<VecCopy> {
    let (mn, operands) = split_mn(t);
    // Merge/zero masks are conditional lane mappings, not unconditional copies.
    if operands.contains(['{', '}']) {
        return None;
    }
    let (width_override, zeroes_above) = vec_copy_mnemonic(mn)?;
    let ops = operand_list(operands);
    // The three-operand `vmovsd %a, %b, %d` normally takes d's upper lane from
    // b, so it is not a copy.  When a and b are the SAME register it is: every
    // lane of d comes from a, which makes it a full-width copy that zeroes the
    // bits above the register — and it is the exact spelling `vex_promote`
    // emits, so refusing it would refuse the common case.
    if ops.len() == 3 {
        if !matches!(mn, "vmovsd" | "vmovss") {
            return None;
        }
        let (a, a_spelling) = vec_operand(ops[0])?;
        let (b, b_spelling) = vec_operand(ops[1])?;
        let (dst, dst_spelling) = vec_operand(ops[2])?;
        if a != b || a_spelling != b_spelling || a_spelling != dst_spelling || a == dst {
            return None;
        }
        return Some(VecCopy {
            src: a,
            dst,
            width: a_spelling,
            zeroes_above: true,
        });
    }
    if ops.len() != 2 {
        return None;
    }
    let (src, src_spelling) = vec_operand(ops[0])?;
    let (dst, dst_spelling) = vec_operand(ops[1])?;
    if src == dst {
        // A self-copy is either a no-op (legacy narrow) or an upper-bit write
        // (VEX); neither is a relation between two registers.
        return None;
    }
    // Mixing widths (`movsd %xmm0, %ymm1` is not encodable, but the text could
    // come from a macro) is refused rather than reasoned about.
    if src_spelling != dst_spelling {
        return None;
    }
    Some(VecCopy {
        src,
        dst,
        width: width_override.unwrap_or(src_spelling),
        zeroes_above,
    })
}

/// Mnemonics that read vector registers and write none: the destination
/// position heuristic ("last operand is the destination") is wrong for these,
/// and treating their last operand as a write would kill copy relations that
/// are still valid. Under-reporting a write is never done here — these
/// instructions genuinely write no vector register.
fn reads_only_mnemonic(mn: &str) -> bool {
    let base = mn.strip_prefix('v').unwrap_or(mn);
    matches!(
        base,
        "comisd"
            | "comiss"
            | "ucomisd"
            | "ucomiss"
            | "ptest"
            | "movmskpd"
            | "movmskps"
            | "pmovmskb"
            | "ldmxcsr"
            | "stmxcsr"
    )
}

/// VEX scalar source-only instructions carry bits above the scalar element
/// from the penultimate operand into the result. Those bits are NOT equal
/// after a narrow legacy copy, even though the arithmetic lanes are equal.
fn scalar_merge_form(text: &str, mn: &str, ops: &[&str]) -> bool {
    mn.starts_with('v')
        && (mn.ends_with("sd") || mn.ends_with("ss"))
        && (ops.len() == 3 || (ops.len() == 4 && ops[0].starts_with('$')))
        && super::fp_liveness::is_full_xmm_def_line(text)
}

/// Width at which `mn` READS a vector operand spelled `spelling` bits wide.
///
/// Scalar forms read one lane; everything else reads the spelling's width.
/// Conversions are taken at the full spelling width on purpose: `vcvtsd2ss`
/// reads a double and writes a float, so its operand widths are mixed and the
/// only safe single answer is the wider one. Over-estimating a read width
/// blocks a rewrite; under-estimating one rewrites a value that was not there.
fn vec_read_width(mn: &str, spelling: u16, merge_form: bool) -> u16 {
    if merge_form {
        return spelling.max(W128);
    }
    if !mn.contains("cvt") {
        if mn.ends_with("sd") {
            return W64;
        }
        if mn.ends_with("ss") {
            return W32;
        }
    }
    spelling
}

/// Width at which `mn` WRITES a vector destination spelled `spelling` bits wide.
fn vec_write_width(mn: &str, spelling: u16) -> u16 {
    if !mn.contains("cvt") {
        if mn.ends_with("sd") {
            return W64;
        }
        if mn.ends_with("ss") {
            return W32;
        }
    }
    spelling
}

/// The widest access `t` makes to vector register `n`, counting reads and
/// writes; 0 only when no explicit or unmodelled implicit access exists.
fn vec_mention_width(t: &str, n: u8) -> u16 {
    // Upper zeroing is a local copy barrier, not an observation of the old
    // value or a definition of a narrow copy's preserved low-XMM upper lane.
    if split_mn(t).0 == "vzeroupper" {
        return 0;
    }
    // A hidden read, hidden write or unmodelled instruction cannot certify a
    // private temporary or unobserved upper bits, even outside a local scan.
    if has_implicit_vector_effects_line(t) {
        return W512;
    }
    let (mn, operands) = split_mn(t);
    let ops = operand_list(operands);
    if ops.is_empty() {
        return 0;
    }
    let merge_form = scalar_merge_form(t, mn, &ops);
    let reads_only = reads_only_mnemonic(mn);
    let last = ops.len() - 1;
    ops.iter()
        .enumerate()
        .filter_map(|(k, op)| {
            let (num, spelling) = vec_operand(op)?;
            if num != n {
                return None;
            }
            let read = vec_read_width(mn, spelling, merge_form && k + 1 == last);
            let write = if k == last && !reads_only {
                if merge_form {
                    spelling.max(W128)
                } else {
                    vec_write_width(mn, spelling)
                }
            } else {
                0
            };
            Some(read.max(write))
        })
        .max()
        .unwrap_or(0)
}

/// Read positions only: copy propagation never renames the output. A scalar
/// data operand can still use a narrow copy; its merging source cannot.
fn vec_read_mention_width(t: &str, n: u8) -> u16 {
    if has_implicit_vector_effects_line(t) {
        return W512;
    }
    let (mn, operands) = split_mn(t);
    let ops = operand_list(operands);
    let full_def = super::fp_liveness::is_full_xmm_def_line(t);
    let merging = scalar_merge_form(t, mn, &ops);
    ops.iter()
        .enumerate()
        .filter_map(|(k, op)| {
            let (num, spelling) = vec_operand(op)?;
            if num != n || (full_def && k + 1 == ops.len()) {
                return None;
            }
            Some(vec_read_width(mn, spelling, merging && k + 2 == ops.len()))
        })
        .max()
        .unwrap_or(0)
}

/// A private bracket may carry the temporary's upper lane back into itself:
/// it never escapes, and the final narrow copy preserves the source's own
/// upper bits. A merge into ANY other register is not that proof.
fn private_scalar_self_merge(t: &str, temporary: u8, width: u16) -> bool {
    let (mn, operands) = split_mn(t);
    let ops = operand_list(operands);
    scalar_merge_form(t, mn, &ops)
        && vec_write_width(mn, W128) <= width
        && vec_operand(ops[ops.len() - 2]) == Some((temporary, W128))
        && vec_operand(ops[ops.len() - 1]) == Some((temporary, W128))
}

/// True when `t` writes vector register `n` in its destination position.
fn writes_vec(t: &str, n: u8) -> bool {
    let (mn, operands) = split_mn(t);
    if reads_only_mnemonic(mn) {
        return false;
    }
    let ops = operand_list(operands);
    let Some(last) = ops.last() else { return false };
    vec_operand(last).is_some_and(|(num, _)| num == n)
}

/// Rewrite every spelling of vector register `old` to `new`, at all three
/// widths, on exact digit boundaries. `%xmm15` must survive a rewrite of
/// register 1: aiming a rename at the wrong register is the one mistake this
/// family of passes cannot survive.
fn replace_vec_reg(line: &str, old: u8, new: u8) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(pct) = rest.find('%') {
        out.push_str(&rest[..pct]);
        let after = &rest[pct + 1..];
        let spelling = after.as_bytes().first().copied();
        let is_vec = matches!(spelling, Some(b'x') | Some(b'y') | Some(b'z'))
            && after.as_bytes().get(1) == Some(&b'm')
            && after.as_bytes().get(2) == Some(&b'm');
        if !is_vec {
            out.push('%');
            rest = after;
            continue;
        }
        let digits_start = 3;
        let digits_end = digits_start
            + after[digits_start..]
                .bytes()
                .take_while(u8::is_ascii_digit)
                .count();
        let digits = &after[digits_start..digits_end];
        let matched = !digits.is_empty()
            && digits.len() <= 2
            && digits
                .parse::<u16>()
                .ok()
                .is_some_and(|n| n == u16::from(old))
            && after[digits_end..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_');
        if matched {
            out.push('%');
            out.push(spelling.unwrap_or(b'x') as char);
            out.push_str("mm");
            out.push_str(&new.to_string());
        } else {
            out.push('%');
            out.push_str(&after[..digits_end]);
        }
        rest = &after[digits_end..];
    }
    out.push_str(rest);
    out
}

/// Rewrite only the READ positions of `t`, replacing register `old` with `new`
/// and leaving the destination operand alone. `vaddsd %xmm2, %xmm1, %xmm2`
/// becomes `vaddsd %xmm0, %xmm1, %xmm2`: the same value read from the copy's
/// source, written where it was already going.
fn replace_vec_reads(t: &str, old: u8, new: u8) -> Option<String> {
    let (mn, operands) = split_mn(t);
    let ops = operand_list(operands);
    if ops.is_empty() {
        return None;
    }
    // A comment anywhere in the operands means the text is not fully modelled.
    if ops.iter().any(|o| o.contains('#')) {
        return None;
    }
    let reads_only = reads_only_mnemonic(mn);
    let last = ops.len() - 1;
    let rewritten: Vec<String> = ops
        .iter()
        .enumerate()
        .map(|(k, op)| {
            if k == last && !reads_only {
                (*op).to_string()
            } else {
                replace_vec_reg(op, old, new)
            }
        })
        .collect();
    Some(format!("{mn} {}", rewritten.join(", ")))
}

/// Directives that cannot emit code, and so cannot break a bracket.
///
/// This is *not* the same question as
/// [`super::helpers::directive_emits_data`]'s -- that one guards a flag window,
/// this one guards a copy relation from being re-formed across a line that
/// changes what the region means -- so the two sets legitimately differ (`.LFB`
/// is harmless here and is not named there).  The overlap that does matter is
/// the alignment family, and it now shares one rule: `.p2align 4` pads with
/// nops, `.p2align 4,0xcc` pads with `int3`, and a trap in the middle of the
/// region this predicate just certified as contiguous is exactly a broken
/// bracket.  Hence the fill check rather than the prefix.
fn benign_directive(t: &str) -> bool {
    let t = t.trim();
    if t.starts_with(".cfi_")
        || t.starts_with(".LFB")
        || t.starts_with(".loc ")
        || t.starts_with(".file ")
    {
        return true;
    }
    for prefix in [".p2align", ".balign", ".align"] {
        if let Some(rest) = t.strip_prefix(prefix) {
            // Longest prefix first above, so `.p2align`/`.balign` are not
            // shadowed by `.align` when they share its spelling rules.
            return !super::helpers::alignment_fill_is_explicit(rest);
        }
    }
    false
}

/// True when a line ends the region a copy relation or a bracket may span.
///
/// A `call` is a barrier twice over: all sixteen vector registers are
/// caller-saved under SysV, so it clobbers both sides of the relation, and it
/// reads `%xmm0`–`%xmm7` as arguments. `vzeroupper` rewrites the upper half of
/// every vector register without naming any of them, so no textual mention scan
/// can see what it does. State images, string-mask outputs, implicit masks
/// and unknown vector effects likewise cannot be treated as transparent.
fn is_barrier(info: &LineInfo, t: &str) -> bool {
    matches!(
        info.kind,
        LineKind::Label
            | LineKind::Jmp
            | LineKind::CondJmp
            | LineKind::JmpIndirect
            | LineKind::Call
            | LineKind::Ret
            | LineKind::InlineAsm
    ) || info.pinned
        || has_implicit_vector_effects_line(t)
        || (info.kind == LineKind::Directive && !benign_directive(t))
}

/// Coalesce `copy-in / ops / copy-out` brackets around a private temporary.
///
/// ```text
/// movsd  %xmm0, %xmm2                          vroundsd $9, %xmm0, %xmm0, %xmm0
/// vroundsd $9, %xmm2, %xmm2, %xmm2      →      ret
/// movsd  %xmm2, %xmm0
/// ret
/// ```
///
/// The seven legality rules are in the module documentation; the short version
/// is that the temporary must be private to the bracket, nothing inside may
/// touch the source, no access inside may be wider than the copy that defines
/// it, and no later read of the source may be wider than the copy that restores
/// it.
pub(super) fn coalesce_vector_brackets(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        let text = infos[i].trimmed(store.get(i)).to_string();
        let Some(copy_in) = parse_vec_copy(&text) else {
            i += 1;
            continue;
        };
        let Some((fstart, fend)) = function_range(store, infos, i) else {
            i += 1;
            continue;
        };
        let src_fam = VEC_FAMILY_BASE + copy_in.src;

        // Walk to the copy-out, validating the interior as we go.
        let mut copy_out: Option<(usize, VecCopy)> = None;
        let mut interior_ok = true;
        let mut j = i + 1;
        while j < fend {
            if infos[j].is_nop() || infos[j].kind == LineKind::Empty {
                j += 1;
                continue;
            }
            let t = infos[j].trimmed(store.get(j));
            if is_barrier(&infos[j], t) {
                interior_ok = false;
                break;
            }
            if let Some(c) = parse_vec_copy(t) {
                if c.src == copy_in.dst && c.dst == copy_in.src {
                    copy_out = Some((j, c));
                    break;
                }
            }
            // Rule 2: the source family is untouchable inside the bracket.
            if mentions_family(t.as_bytes(), src_fam) {
                interior_ok = false;
                break;
            }
            // Rule 3: no interior access wider than the copy that defines it.
            if vec_mention_width(t, copy_in.dst) > copy_in.width
                && !private_scalar_self_merge(t, copy_in.dst, copy_in.width)
            {
                interior_ok = false;
                break;
            }
            j += 1;
        }
        let (Some((kout, out_copy)), true) = (copy_out, interior_ok) else {
            i += 1;
            continue;
        };

        if out_copy.width > copy_in.width {
            i += 1;
            continue;
        }

        // Rule 4: the temporary is private to the bracket. "Private" means
        // outside it — the interior is where the temporary is SUPPOSED to be
        // mentioned, so the scan covers the function's head and tail only.
        let private = (fstart..i).chain(kout + 1..fend).all(|n| {
            infos[n].is_nop() || vec_mention_width(infos[n].trimmed(store.get(n)), copy_in.dst) == 0
        });
        if !private {
            i += 1;
            continue;
        }
        // Rule 5: no read of the source after the bracket wider than the
        // copy-out, whose upper-bit behaviour the rewrite changes -- `ret`
        // included (a `__m128d` return reads all of `%xmm0`); any `ret` of
        // the function counts, since a back edge can reach an earlier one.
        let upper_safe =
            super::fp_liveness::ret_xmm_read_bits(store, infos, fstart, fend, copy_in.src)
                <= out_copy.width
                && (kout + 1..fend).all(|n| {
                    infos[n].is_nop()
                        || vec_mention_width(infos[n].trimmed(store.get(n)), copy_in.src)
                            <= out_copy.width
                });
        if !upper_safe {
            i += 1;
            continue;
        }

        let dst_fam = VEC_FAMILY_BASE + copy_in.dst;
        for n in i + 1..kout {
            if infos[n].is_nop() {
                continue;
            }
            let t = infos[n].trimmed(store.get(n)).to_string();
            if !mentions_family(t.as_bytes(), dst_fam) {
                continue;
            }
            let renamed = replace_vec_reg(&t, copy_in.dst, copy_in.src);
            if renamed != t {
                replace_line(store, &mut infos[n], n, format!("    {renamed}"));
            }
        }
        mark_nop(&mut infos[i]);
        mark_nop(&mut infos[kout]);
        changed = true;
        i = kout + 1;
    }
    changed
}

/// Prove that only the scalar result of a self-merging consumer can escape.
/// The low-128 liveness oracle cannot distinguish a low-element extraction
/// from an upper-lane observation, so walk straight-line narrow reads until
/// the WHOLE old XMM value is proven dead. Its CFG query includes backedges,
/// calls, returns and implicit observers; an unknown answer is not a proof.
fn narrow_result_only(
    store: &LineStore,
    infos: &[LineInfo],
    producer: usize,
    register: u8,
    width: u16,
    oracle: &FpLiveness,
) -> bool {
    // The unannotated-call fallback infers staged ABI arguments rather than
    // proving their widths. Do not extend that assumption to this new merge
    // rewrite: only non-argument/non-return SysV temporaries qualify here.
    // Existing width-equal propagation remains available for every register.
    if register < 8 {
        return false;
    }
    for n in producer + 1..store.len() {
        if oracle.xmm_live_after(n - 1, u32::from(register)) == Some(false) {
            return true;
        }
        if infos[n].is_nop() || infos[n].kind == LineKind::Empty {
            continue;
        }
        let text = infos[n].trimmed(store.get(n));
        if is_barrier(&infos[n], text) {
            return false;
        }
        let (mn, operands) = split_mn(text);
        let ops = operand_list(operands);
        // MOVD/Q extracts exactly the low 32/64 bits; spelling the source
        // as XMM does not make this an observation of all 128 bits.
        let read_width = if ops.first().and_then(|op| vec_operand(op)) == Some((register, W128)) {
            match mn {
                "movd" | "vmovd" => W32,
                "movq" | "vmovq" => W64,
                _ => vec_read_mention_width(text, register),
            }
        } else {
            vec_read_mention_width(text, register)
        };
        if read_width > width {
            return false;
        }
        if writes_vec(text, register) && super::fp_liveness::is_full_xmm_def_line(text) {
            return true;
        }
    }
    false
}

/// A narrow scalar self-merge rename changes reads, never writes or CFG.
/// Removed reads leave an over-approximation: a cached DEAD answer stays
/// sound. Added reads invalidate the source register before its next query.
/// On a negative answer after removals, refresh and retry so precision (and
/// emitted code) matches eager refresh. Only private XMM8..15 are queried;
/// inferred call/return ABI masks affect XMM0..7, not these facts.
///
/// All other rewrites retain eager refresh. In particular this is NOT a
/// general dirty-once-per-pass cache. Traversal is forward, functions are not
/// revisited, and no rewrite crosses a function/basic-block barrier.
#[derive(Default)]
struct PropagationLiveness {
    oracle: Option<FpLiveness>,
    added_reads: u32,
    removed_reads: u32,
}

impl PropagationLiveness {
    fn refresh_at(&mut self, store: &LineStore, infos: &[LineInfo], at: usize) {
        if let Some(oracle) = self.oracle.as_mut() {
            oracle.refresh_at(store, infos, at);
        }
        self.added_reads = 0;
        self.removed_reads = 0;
    }

    fn narrow_rename(&mut self, src: u8, dst: u8) {
        self.added_reads |= 1 << src;
        self.removed_reads |= 1 << dst;
    }

    fn allows_narrow_result(
        &mut self,
        store: &LineStore,
        infos: &[LineInfo],
        producer: usize,
        register: u8,
        width: u16,
    ) -> bool {
        debug_assert!(register >= 8);
        let bit = 1 << register;
        if self.added_reads & bit != 0 {
            self.refresh_at(store, infos, producer);
        }
        let oracle = self
            .oracle
            .get_or_insert_with(|| FpLiveness::new(store, infos));
        if narrow_result_only(store, infos, producer, register, width, oracle) {
            return true;
        }
        if self.removed_reads & bit == 0 {
            return false;
        }
        self.refresh_at(store, infos, producer);
        self.oracle.as_ref().is_some_and(|oracle| {
            narrow_result_only(store, infos, producer, register, width, oracle)
        })
    }
}

/// Substitute a vector copy's source for reads of its destination, forward
/// through the basic block, width-checked at every step.
pub(super) fn propagate_vector_copies(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    propagate_vector_copies_impl::<false>(store, infos)
}

// The eager instantiation is only used by differential tests. Production
// specializes the constant away, retaining one implementation of the rewrite.
fn propagate_vector_copies_impl<const EAGER: bool>(
    store: &mut LineStore,
    infos: &mut [LineInfo],
) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut fp = PropagationLiveness::default();
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        let text = infos[i].trimmed(store.get(i)).to_string();
        let Some(copy) = parse_vec_copy(&text) else {
            i += 1;
            continue;
        };
        let src_fam = VEC_FAMILY_BASE + copy.src;
        let mut j = i + 1;
        while j < len {
            if infos[j].is_nop() || infos[j].kind == LineKind::Empty {
                j += 1;
                continue;
            }
            let t = infos[j].trimmed(store.get(j)).to_string();
            if is_barrier(&infos[j], &t) {
                break;
            }
            let touches_src = mentions_family(t.as_bytes(), src_fam);
            // A write to the copy's DESTINATION ends the range. A true
            // full-def write (`vmulsd %xmmA, %xmmB, %xmmD`) may still be
            // renamed THROUGH — its reads are explicit operands and stay
            // textually visible — but a destination-reading write
            // (`shufps $0x00, %xmm0, %xmm0`) may not: renaming its explicit
            // operands (`shufps $0x00, %xmm1, %xmm0`) hides the implicit
            // destination read, every later textual pass believes %xmm0's
            // old value is dead, and that exact chain deleted the `vmovss`
            // reset feeding a splat and shipped [s, s, t, t]
            // (bb_slp_v8 C2). The FP full-def model tells the two apart.
            if writes_vec(&t, copy.dst) {
                if !super::fp_liveness::is_full_xmm_def_line(&t) {
                    break;
                }
                let width = vec_read_mention_width(&t, copy.dst);
                let narrow_merge = width > copy.width
                    && copy.dst >= 8
                    && private_scalar_self_merge(&t, copy.dst, copy.width)
                    && fp.allows_narrow_result(store, infos, j, copy.dst, copy.width);
                if width != 0 && (width <= copy.width || narrow_merge) {
                    if let Some(rewritten) = replace_vec_reads(&t, copy.dst, copy.src) {
                        if rewritten != t {
                            replace_line(store, &mut infos[j], j, format!("    {rewritten}"));
                            if narrow_merge && !EAGER {
                                fp.narrow_rename(copy.src, copy.dst);
                            } else {
                                fp.refresh_at(store, infos, j);
                            }
                            changed = true;
                        }
                    }
                }
                break;
            }
            let width = vec_read_mention_width(&t, copy.dst);
            if width != 0 && width <= copy.width {
                if let Some(rewritten) = replace_vec_reads(&t, copy.dst, copy.src) {
                    if rewritten != t {
                        replace_line(store, &mut infos[j], j, format!("    {rewritten}"));
                        fp.refresh_at(store, infos, j);
                        changed = true;
                    }
                }
            }
            let t_after = infos[j].trimmed(store.get(j)).to_string();
            if writes_vec(&t_after, copy.src) || touches_src {
                break;
            }
            j += 1;
        }
        i += 1;
    }
    changed
}

/// Widen a narrow copy into a private temporary so its consumers can see it.
///
/// The codegen copies a `double` with `movsd`, which defines 64 bits, and then
/// consumes it with a packed `vandpd`, which reads 128. Rule 3 rightly refuses
/// to propagate across that width gap, and rule 5 refuses to coalesce it, so
/// `__builtin_copysign` keeps both of its copies:
///
/// ```text
/// movsd  %xmm0, %xmm2                 movapd %xmm0, %xmm2
/// vandpd .LC0(%rip), %xmm2, %xmm0  →  vandpd .LC0(%rip), %xmm2, %xmm0
/// ```
///
/// The 64 bits the copy does not define are the destination's own leftovers.
/// When the destination is a private temporary — mentioned nowhere before the
/// copy — those leftovers are a caller-saved register's entry value, which the
/// SysV ABI leaves unspecified, so nothing can depend on them. Replacing
/// undefined bits with the source's is a refinement, not a change, and it is
/// what lets the wider consumer be served.
///
/// Only the legacy PRESERVING forms qualify. `movq`/`movd` and the VEX scalar
/// forms zero the bits above the lane they copy: zero is a defined value, and
/// replacing it with the source's upper half would be observable.
pub(super) fn widen_private_vector_copies(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    for i in 0..len {
        if infos[i].is_nop() || infos[i].pinned {
            continue;
        }
        let text = infos[i].trimmed(store.get(i)).to_string();
        let Some(copy) = parse_vec_copy(&text) else {
            continue;
        };
        if copy.zeroes_above || copy.width >= W128 {
            continue;
        }
        let (mn, _) = split_mn(&text);
        let wide_mn = match mn {
            "movsd" => "movapd",
            "movss" => "movaps",
            _ => continue,
        };
        let Some((fstart, fend)) = function_range(store, infos, i) else {
            continue;
        };
        // Private: nothing before the copy in this function names the
        // destination, explicitly or implicitly, so its upper bits are the
        // caller's unspecified leftovers.
        let dead_in = (fstart..i).all(|n| {
            infos[n].is_nop() || vec_mention_width(infos[n].trimmed(store.get(n)), copy.dst) == 0
        });
        if !dead_in {
            continue;
        }
        // Worth doing only if some line in the same block reads the destination
        // wider than the narrow copy defines, with no barrier in between.  A
        // plain loop rather than a fold: the barrier that ends the scan (the
        // `ret` every function ends with) must stop the search WITHOUT
        // discarding what it already found, and `try_fold`'s `None` does exactly
        // that — it turned this transformation into a no-op on every function.
        let mut needs_widening = false;
        for n in i + 1..fend {
            if infos[n].is_nop() || infos[n].kind == LineKind::Empty {
                continue;
            }
            let t = infos[n].trimmed(store.get(n));
            if is_barrier(&infos[n], t) {
                break;
            }
            if vec_mention_width(t, copy.dst) > copy.width {
                needs_widening = true;
                break;
            }
        }
        if !needs_widening {
            continue;
        }
        let widened = text.replacen(mn, wide_mn, 1);
        replace_line(store, &mut infos[i], i, format!("    {widened}"));
        changed = true;
    }
    changed
}

/// The next line that is neither a nop nor blank, or `None` if a barrier or the
/// end of the function comes first. "Adjacent" in this pass means exactly that.
fn next_real(store: &LineStore, infos: &[LineInfo], from: usize, fend: usize) -> Option<usize> {
    for n in from..fend {
        if debug_transparent(store, infos[n], n) || infos[n].kind == LineKind::Empty {
            continue;
        }
        let t = infos[n].trimmed(store.get(n));
        if is_barrier(&infos[n], t) {
            return None;
        }
        return Some(n);
    }
    None
}

/// The previous line that is neither a nop nor blank, or `None` if a barrier
/// or the start of the function comes first.
fn prev_real(store: &LineStore, infos: &[LineInfo], before: usize, fstart: usize) -> Option<usize> {
    let mut n = before;
    while n > fstart {
        n -= 1;
        if debug_transparent(store, infos[n], n) || infos[n].kind == LineKind::Empty {
            continue;
        }
        let t = infos[n].trimmed(store.get(n));
        if is_barrier(&infos[n], t) {
            return None;
        }
        return Some(n);
    }
    None
}

/// Both operands of a register copy are spelled `%xmm`.
fn copy_spelled_xmm(t: &str) -> bool {
    copy_spelled_width(t) == Some(W128)
}

/// The common spelled width of every register operand of `t`
/// (`Some(128)` for an all-`%xmm` line), `None` when operands disagree or
/// any operand is not a vector register.
fn copy_spelled_width(t: &str) -> Option<u16> {
    let (_, ops) = split_mn(t);
    let mut width = None;
    for op in operand_list(ops) {
        let (_, w) = vec_operand(op.trim())?;
        match width {
            None => width = Some(w),
            Some(seen) if seen == w => {}
            Some(_) => return None,
        }
    }
    width
}

/// Coalesce a full-width VEX copy into a packed FMA and its adjacent FMA
/// consumer. The instruction forms and operand positions do not change: this
/// is register renaming, not reassociation (including for NaN payloads).
///
/// The copied source must be dead in ALL architectural regions after the
/// producer; the temporary must be dead after the consumer. Both operations
/// must be unmasked, register-only, equal-width packed FMAs. Keeping this
/// window small avoids turning the peephole into a second register allocator.
pub(super) fn coalesce_packed_fma_prefix(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    fn packed_fma(t: &str, width: u16) -> bool {
        let (mn, operands) = split_mn(t);
        let Some(rest) = ["vfmadd", "vfmsub", "vfnmadd", "vfnmsub"]
            .iter()
            .find_map(|stem| mn.strip_prefix(stem))
        else {
            return false;
        };
        let Some(suffix) = ["132", "213", "231"]
            .iter()
            .find_map(|form| rest.strip_prefix(form))
        else {
            return false;
        };
        if !matches!(suffix, "ps" | "pd") || !matches!(width, W128 | W256) {
            return false;
        }
        let ops = operand_list(operands);
        ops.len() == 3
            && ops
                .iter()
                .all(|op| vec_operand(op).is_some_and(|(_, w)| w == width))
    }

    // No line insertion or CFI mutation: the index remains valid throughout.
    let cfi = super::relay_and_lea::CfiIndex::build(store, infos);
    let mut changed = false;
    let mut fp = None;
    for i in 0..infos.len() {
        if infos[i].is_nop() || infos[i].pinned {
            continue;
        }
        let text = infos[i].trimmed(store.get(i));
        let Some(copy) = parse_vec_copy(text) else {
            continue;
        };
        if !copy.zeroes_above
            || copy.src == copy.dst
            || copy_spelled_width(text) != Some(copy.width)
        {
            continue;
        }
        let Some((_, end)) = cfi.function_range(i, store.len()) else {
            continue;
        };
        let Some(j) = next_real(store, infos, i + 1, end) else {
            continue;
        };
        let Some(k) = next_real(store, infos, j + 1, end) else {
            continue;
        };
        let producer = infos[j].trimmed(store.get(j));
        let consumer = infos[k].trimmed(store.get(k));
        if infos[j].pinned
            || infos[k].pinned
            || !packed_fma(producer, copy.width)
            || !packed_fma(consumer, copy.width)
            || operand_list(split_mn(producer).1)
                .last()
                .and_then(|op| vec_operand(op))
                .map(|(n, _)| n)
                != Some(copy.dst)
            || vec_mention_width(consumer, copy.dst) != copy.width
            || names_vec_reg(consumer, copy.src)
        {
            continue;
        }
        let oracle = fp.get_or_insert_with(|| FpLiveness::new(store, infos));
        if oracle.xmm_live_after(j, u32::from(copy.src)) != Some(false)
            || !oracle.upper_regions_dead_after(j, u32::from(copy.src))
            || !oracle.deleted_write_regions_dead_after(
                store,
                infos,
                k,
                u32::from(copy.dst),
                &[i, j],
            )
        {
            continue;
        }
        let producer = replace_vec_reg(producer, copy.dst, copy.src);
        let consumer = replace_vec_reg(consumer, copy.dst, copy.src);
        replace_line(store, &mut infos[j], j, format!("    {producer}"));
        replace_line(store, &mut infos[k], k, format!("    {consumer}"));
        mark_nop(&mut infos[i]);
        // Register identities changed. Never reuse a pre-rewrite liveness
        // result to license another transformation in the same iteration.
        fp = None;
        changed = true;
    }
    changed
}

/// Replace a proven uniform per-lane shift with the scalar-count encoding.
/// (V)MOVD zeroes bits 32..127, so its low 64 bits equal the dword count;
/// (V)MOVQ supplies the complete qword count. Legacy upper preservation and
/// VEX/EVEX upper zeroing are included in the deleted-write liveness proof.
/// Without that proof a broadcast cannot become a uniform shift: the latter
/// reads SIXTY-FOUR count bits, not just the low dword. Word forms are not
/// eligible: their per-lane operations consume only 16 count bits, so an
/// arbitrary nonzero bit 16..63 would change the result. Dword/qword and
/// 128/256/512-bit shifts saturate at the lane width and leave flags unchanged.
/// No C undefined-behavior assumption.
/// Retarget the count move to the dead broadcast bank, also freeing the
/// original data bank for the ordinary copy-propagation pass.
pub(super) fn fold_uniform_vector_shift(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let cfi = super::relay_and_lea::CfiIndex::build(store, infos);
    let mut fp = None;
    let mut changed = false;
    for i in 0..infos.len() {
        if infos[i].is_nop() || infos[i].pinned {
            continue;
        }
        let text = infos[i].trimmed(store.get(i));
        let (mov, args) = split_mn(text);
        let row = match mov {
            "vmovd" | "movd" => 1,
            "vmovq" | "movq" => 0,
            _ => continue,
        };
        let ops = operand_list(args);
        if ops.len() != 2 || text.contains(['{', '}']) {
            continue;
        }
        let Some(gp) = super::relay_and_lea::plain_gp_operand(ops[0]) else {
            continue;
        };
        if ops[0] != REG_NAMES[row][gp as usize] {
            continue;
        }
        let Some((count, W128)) = vec_operand(ops[1]) else {
            continue;
        };
        let Some((_, end)) = cfi.function_range(i, store.len()) else {
            continue;
        };
        let Some(j) = next_real(store, infos, i + 1, end) else {
            continue;
        };
        let Some(k) = next_real(store, infos, j + 1, end) else {
            continue;
        };
        if infos[j].pinned || infos[k].pinned {
            continue;
        }
        let broadcast = infos[j].trimmed(store.get(j));
        let shift = infos[k].trimmed(store.get(k));
        if broadcast.contains(['{', '}']) || shift.contains(['{', '}']) {
            continue;
        }
        let (bmn, bargs) = split_mn(broadcast);
        let bops = operand_list(bargs);
        if bops.len() != 2 || vec_operand(bops[0]) != Some((count, W128)) {
            continue;
        }
        let Some((splat, width @ (W128 | W256 | W512))) = vec_operand(bops[1]) else {
            continue;
        };
        let (mn, args) = split_mn(shift);
        let uniform = match (row, bmn, mn) {
            (1, "vpbroadcastd", "vpsllvd") => "vpslld",
            (1, "vpbroadcastd", "vpsrlvd") => "vpsrld",
            (1, "vpbroadcastd", "vpsravd") => "vpsrad",
            (0, "vpbroadcastq", "vpsllvq") => "vpsllq",
            (0, "vpbroadcastq", "vpsrlvq") => "vpsrlq",
            _ => continue,
        };
        let shifts = operand_list(args);
        if shifts.len() != 3 || vec_operand(shifts[0]) != Some((splat, width)) {
            continue;
        }
        let Some((src, sw)) = vec_operand(shifts[1]) else {
            continue;
        };
        let Some((dst, dw)) = vec_operand(shifts[2]) else {
            continue;
        };
        if sw != width || dw != width || src == count || src == splat {
            continue;
        }
        let live = fp.get_or_insert_with(|| FpLiveness::new(store, infos));
        if [count, splat].iter().any(|&bank| {
            bank != dst
                && !live.deleted_write_regions_dead_after(store, infos, k, u32::from(bank), &[i, j])
        }) {
            continue;
        }
        let moved = format!("    {mov} {}, %xmm{splat}", ops[0]);
        let shifted = format!("    {uniform} %xmm{splat}, {}, {}", shifts[1], shifts[2]);
        replace_line(store, &mut infos[i], i, moved);
        replace_line(store, &mut infos[k], k, shifted);
        mark_nop(&mut infos[j]);
        fp = None;
        changed = true;
    }
    changed
}

/// Re-home a scalar FMA so that the copies around it disappear.
///
/// The codegen stages a scalar FMA's operands through copies whenever the
/// register allocator's homes do not line up with the one form it picked:
/// the accumulator is copied into the destination, a value that was sitting
/// in the destination is copied out of the way first, and the result is
/// copied to where it was wanted.  That is what `__builtin_fma` reaches the
/// assembler as, what `a * b + c` reaches it as in the 132 form, and what
/// `x * x + c` reaches it as with the swap:
///
/// ```text
/// vmovsd %xmm2, %xmm2, %xmm5          vfmadd213sd %xmm2, %xmm1, %xmm0
/// vfmadd231sd %xmm1, %xmm0, %xmm5  →  ret
/// movsd %xmm5, %xmm0
/// ret
///
/// movsd %xmm2, %xmm4                  vfmadd132sd %xmm1, %xmm2, %xmm0
/// movsd %xmm0, %xmm2               →  ret
/// vfmadd132sd %xmm1, %xmm4, %xmm2
/// movsd %xmm2, %xmm0
/// ret
///
/// movsd %xmm0, %xmm2                  vfmadd132sd %xmm0, %xmm1, %xmm0
/// movsd %xmm1, %xmm0               →  ret
/// vfmadd231sd %xmm2, %xmm2, %xmm0
/// ret
/// ```
///
/// Each right-hand side is GCC 16.2's exact text; Clang 23.1 and ICX emit the
/// 213 spelling of the same instruction.
///
/// [`coalesce_vector_brackets`] cannot serve these because every FMA form
/// reads its destination as one of its three roles: the destination is not a
/// plain output that can be renamed onto the value's home.  What CAN be
/// changed is the *encoding*.  The three forms differ only in which role the
/// destination plays ([`super::fma_forms`] has the table), so whenever the
/// register the result must end up in holds one of the three values, exactly
/// one form — up to the 132/213 tie, broken towards the original — writes it
/// directly, and the copies that shuffled values into the form's fixed
/// positions are not needed.
///
/// # The rule
///
/// Take the run of register-to-register vector copies immediately before the
/// FMA whose destinations feed it (directly, or through another copy of the
/// run), and the scalar copy of the result immediately after it, if there is
/// one.  Simulate the run to learn, for every role register, which register
/// held that value BEFORE the run began — its origin.  Rewrite the roles to
/// their origins, encode the result into the register the code wanted (the
/// copy-out's destination, else the FMA's own), and delete the run and the
/// copy-out.
///
/// # Why it is legal
///
/// * **Origins are exact.**  The run is copies and nothing else, so the value
///   a role register holds at the FMA is the value its origin held when the
///   run began; with the run deleted, the origin still holds it at the FMA,
///   because nothing between the run's start and the FMA writes anything.
///   A copy narrower than the element (`movss` into an `sd` role) does not
///   define the bits the role reads and ends the run; wider copies are fine,
///   the role reads only its low lane.
/// * **Deleted destinations are dead.**  Every register the run wrote, and
///   the FMA's own destination when the result moves elsewhere, is no longer
///   written by the rewrite; a later reader would see a stale value.  Each is
///   asked of [`FpLiveness`] after the last deleted line, which sees readers
///   below, readers reached through a back edge, and the implicit reads of
///   `ret` and `call`, and answers "live" for a function it cannot analyse.
///   Readers *before* the run need no rule: they ran before it.
/// * **The result register's upper bits are its own.**  A scalar FMA writes
///   only its destination's low lane — measured on hardware: all three forms
///   leave bits 127:64 of the destination XMM unchanged, unlike the VEX
///   scalar arithmetic family, whose upper half comes from its first source.
///   Originally the destination's upper bits came from the last thing that
///   wrote it: a legacy element-width copy preserves them, a `movapd` or a
///   VEX copy replaces them.  Unless EVERY copy of the run that wrote the
///   result register was preserving — an earlier replacing copy followed by
///   a later preserving one leaves its own bits, which the last copy alone
///   would hide — the rewrite may change those bits, so it is allowed only
///   when no line of the function outside the rewritten bracket reads that
///   register wider than the element.  The copy-out is held to the same
///   rule from the other side: a legacy element-width copy-out merges, as
///   writing the FMA into the result register does; a replacing one
///   (`movapd`, a VEX copy) gave that register the FMA destination's upper
///   bits, so it too is allowed only without a wide reader.
/// * **Memory operands travel with their role.**  The encoder puts a memory
///   role at operand 0 in whichever form allows it and reports that no form
///   can when the role that must be memory is the destination, or when two
///   roles are memory.
/// * A writemask, a broadcast, a packed suffix or a wide spelling never
///   parses as a scalar FMA here; a label, branch, call or `ret` between the
///   lines ends the bracket; pinned lines are never touched.
///
/// Measured over the archived oracle corpus this collapses every
/// straight-line FMA shape the codegen produces — the intrinsic, the
/// contracted `a * b + c` in all its operand orders, and the squared norm —
/// to the one instruction the oracles emit.
pub(super) fn reassociate_fma_accumulator(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    use super::fma_forms::{parse_scalar_fma, xmm_reg};

    /// Longest run of copies considered; an FMA has three roles, and a fourth
    /// copy can only be a chain link.
    const MAX_RUN: usize = 4;

    let len = store.len();
    let mut changed = false;
    let mut fp: Option<FpLiveness> = None;
    let mut j = 0;
    while j < len {
        if infos[j].is_nop() || infos[j].pinned {
            j += 1;
            continue;
        }
        let t_fma = infos[j].trimmed(store.get(j)).to_string();
        let Some(fma) = parse_scalar_fma(&t_fma) else {
            j += 1;
            continue;
        };
        let Some((fstart, fend)) = function_range(store, infos, j) else {
            j += 1;
            continue;
        };
        let elem_bits = fma.width.bits();
        let t = fma.dst();

        // The copy-out, if any: an adjacent register-to-register move of the
        // result in `%xmm` spelling, at least element-wide. A legacy
        // element-width copy-out merges, which is exactly what writing the
        // FMA into `d` does; a replacing one (`movapd`, a VEX copy) gave `d`
        // the upper bits of `t` instead, so it joins the upper-bits rule below.
        let copy_out = next_real(store, infos, j + 1, fend).and_then(|k| {
            if infos[k].pinned {
                return None;
            }
            let text = infos[k].trimmed(store.get(k));
            let c = parse_vec_copy(text)?;
            (c.src == t && c.width >= elem_bits && copy_spelled_xmm(text)).then_some((k, c))
        });
        let out_merges = copy_out.is_none_or(|(_, c)| c.width == elem_bits && !c.zeroes_above);
        let (d, last) = match copy_out {
            Some((k, c)) => (c.dst, k),
            None => (t, j),
        };

        // The run of copies feeding the FMA, walked backwards: a copy joins
        // when its destination is a role register or the source of a copy
        // already in the run.
        let roles = fma.roles();
        let mut relevant: u16 = 0;
        for r in [roles.mult[0], roles.mult[1], roles.addend] {
            if let Some(n) = xmm_reg(r) {
                relevant |= 1 << n;
            }
        }
        let mut run: Vec<(usize, VecCopy)> = Vec::new();
        let mut p = j;
        while run.len() < MAX_RUN {
            let Some(i) = prev_real(store, infos, p, fstart) else {
                break;
            };
            if infos[i].pinned {
                break;
            }
            let text = infos[i].trimmed(store.get(i));
            let Some(c) = parse_vec_copy(text) else { break };
            if relevant & (1 << c.dst) == 0 || c.width < elem_bits || !copy_spelled_xmm(text) {
                break;
            }
            relevant |= 1 << c.src;
            run.push((i, c));
            p = i;
        }
        if run.is_empty() && copy_out.is_none() {
            j += 1;
            continue;
        }
        run.reverse();

        // Origins: simulate the run forwards from the identity.
        let mut origin: [u8; 16] = std::array::from_fn(|n| n as u8);
        for (_, c) in &run {
            origin[usize::from(c.dst)] = origin[usize::from(c.src)];
        }
        let spelling: [String; 16] = std::array::from_fn(|n| format!("%xmm{n}"));
        let mut rewritten = roles.clone();
        for n in 0u8..16 {
            if origin[usize::from(n)] != n {
                rewritten =
                    rewritten.substitute(n, spelling[usize::from(origin[usize::from(n)])].as_str());
            }
        }
        let Some(enc) = rewritten.encode(d, fma.form) else {
            j += 1;
            continue;
        };

        // Every register the rewrite stops writing must be dead after the
        // last deleted line.
        let mut owned: Vec<usize> = run.iter().map(|(i, _)| *i).collect();
        owned.push(j);
        owned.push(last);
        let mut must_be_dead: u16 = 0;
        for (_, c) in &run {
            if c.dst != d {
                must_be_dead |= 1 << c.dst;
            }
        }
        if copy_out.is_some() {
            must_be_dead |= 1 << t;
        }
        let oracle = fp.get_or_insert_with(|| FpLiveness::new(store, infos));
        let all_dead = (0u8..16).filter(|n| must_be_dead & (1 << n) != 0).all(|n| {
            oracle.deleted_write_regions_dead_after(store, infos, last, u32::from(n), &owned)
        });
        if !all_dead {
            j += 1;
            continue;
        }

        // The result register's upper bits: unchanged by the rewrite only if
        // every copy the run wrote into it was a legacy element-width merge.
        // The LAST copy alone is not enough: an earlier non-preserving write
        // (a `vmovsd` that zeroes, a `movapd` that replaces) followed by a
        // later preserving one leaves the register's upper bits at the value
        // the earlier copy left, and deleting the whole run would restore the
        // pre-bracket bits instead.  `None` in the run means no copy wrote it,
        // which is the `is_none_or` fast path.
        let upper_preserved = out_merges
            && run
                .iter()
                .all(|(_, c)| c.dst != d || (!c.zeroes_above && c.width == elem_bits));
        if !upper_preserved {
            // Only a line that can execute AFTER the bracket observes the
            // rewritten upper bits.  With no label between the function's
            // start and the FMA, the prefix is straight-line code entered
            // once, so nothing in it runs after `last` (no branch can reach
            // it); a label there may be a loop head, and the whole function
            // stays in scope.  `chain`'s `vaddsd .., %xmm0, %xmm2` reads the
            // INCOMING %xmm0's upper lane before the bracket redefines it.
            let prefix_straight =
                (fstart..j).all(|n| infos[n].is_nop() || infos[n].kind != LineKind::Label);
            let scan_from = if prefix_straight { last + 1 } else { fstart };
            let wide_reader = super::fp_liveness::ret_xmm_read_bits(store, infos, fstart, fend, d)
                > elem_bits
                || (scan_from..fend).any(|n| {
                    !owned.contains(&n)
                        && !infos[n].is_nop()
                        && vec_mention_width(infos[n].trimmed(store.get(n)), d) > elem_bits
                });
            if wide_reader {
                j += 1;
                continue;
            }
        }

        debug_assert_eq!(xmm_reg(enc.ops[2]), Some(d));
        replace_line(store, &mut infos[j], j, format!("    {}", enc.render()));
        for (i, _) in &run {
            mark_nop(&mut infos[*i]);
        }
        if let Some((k, _)) = copy_out {
            mark_nop(&mut infos[k]);
        }
        if let Some(oracle) = fp.as_mut() {
            oracle.refresh_at(store, infos, j);
        }
        changed = true;
        j = last + 1;
    }
    changed
}

/// Bases of the VEX arithmetic and logical instructions that write their
/// destination from their sources alone, in `ss`/`sd`/`ps`/`pd` flavours.
/// An explicit list: the FMA families read their destination as a role, the
/// EVEX `vfixupimm*` merges a table into it, and the moves, converts and
/// inserts each have their own upper-lane story.  None of those is here.
/// The variable blends (`blendv`) ARE here: a blend reads its mask and both
/// sources, then writes its destination — a plain output exactly like the
/// logical ops (`and`/`or`/`xor` differ from a blend only in having no mask
/// operand).  The blend's low element comes from its sources' low elements
/// alone, so retargeting the destination changes it in no way; the upper
/// lane is guarded by the same wide-reader check as every other base here.
const VEX_PLAIN_OUTPUT_BASES: &[&str] = &[
    "add", "sub", "mul", "div", "min", "max", "sqrt", "rsqrt", "rcp", "round", "xor", "and",
    "andn", "or", "blendv",
    // `vcmpps $imm, b, a, d` (and `pd`/`ss`/`sd`): the predicate immediate
    // leads, the mask is written from the two sources alone.
    "cmp",
];

/// `Some(true)` when `mn` is a VEX (`v`-prefixed) instruction from
/// [`VEX_PLAIN_OUTPUT_BASES`] with an `ss`/`sd`/`ps`/`pd` suffix.
fn vex_plain_output(mn: &str) -> bool {
    let Some(rest) = mn.strip_prefix('v') else {
        return false;
    };
    let Some(base) = rest
        .strip_suffix("ss")
        .or_else(|| rest.strip_suffix("sd"))
        .or_else(|| rest.strip_suffix("ps"))
        .or_else(|| rest.strip_suffix("pd"))
    else {
        return false;
    };
    VEX_PLAIN_OUTPUT_BASES.contains(&base)
}

/// VEX/AVX2 packed-integer instructions whose destination is a plain output:
/// every source is read before the write, the full spelled width of the
/// destination is written from the sources alone, and the bits above the
/// spelled width are zeroed.  Exactly the list the SIMD vector lowering
/// (`vector_simd.rs`) and the BB-SLP/map emitters produce for `vector_size`
/// integer lanes; nothing here merges into or reads its destination
/// (no FMA, no `vpinsr*`, no `vpblendw`-with-dest-as-source, no masking).
fn vex_int_plain_output(mn: &str) -> bool {
    let Some(rest) = mn.strip_prefix("vp") else {
        return false;
    };
    // Bitwise forms carry no lane suffix at all.
    if matches!(rest, "and" | "andn" | "or" | "xor") {
        return true;
    }
    const BASES: &[&str] = &[
        "add", "adds", "addus", "sub", "subs", "subus", "mull", "mulh", "mulhu", "mulu", "mul",
        "cmpeq", "cmpgt", "mins", "minu", "maxs", "maxu", "sll", "srl", "sra", "sllv", "srlv",
        "srav", "shuf", "blendv", "abs", "avg", "unpckl", "unpckh", "ackss", "ackus",
    ];
    const LANES: &[&str] = &["b", "w", "d", "q", "bw", "wd", "dq", "qdq", "dw", "wb"];
    BASES.iter().any(|base| {
        rest.strip_prefix(base)
            .is_some_and(|lane| LANES.contains(&lane))
    })
}

/// Re-home a VEX instruction whose result is only ever copied somewhere else:
///
/// ```text
/// vxorpd .LC0(%rip), %xmm0, %xmm3        vxorpd .LC0(%rip), %xmm0, %xmm0
/// movsd  %xmm3, %xmm0                 →  ret
/// ret
/// ```
///
/// A VEX arithmetic or logical instruction writes its destination from its
/// sources alone, so unlike the FMA forms the destination is a plain output
/// and can be retargeted wherever the copy-out wanted it — nothing about the
/// computation changes.  [`coalesce_vector_brackets`] handles this when a
/// copy-IN opens the bracket; this rule is the same idea for a producer that
/// needed no copy-in because its sources were already in place, which is what
/// every straight-line negate, round, sqrt and two-operand arithmetic reaches
/// the assembler as when the allocator's home for the result is not the
/// register the caller wants it in.
///
/// # Why it is legal
///
/// * The producer is from [`VEX_PLAIN_OUTPUT_BASES`]: it reads its sources,
///   then writes its destination — the low lane(s) with the result, the rest
///   of the 128-bit register from its first source for the scalar forms, and
///   zero above.  Retargeting the destination changes none of that.
/// * The copy-out is a legacy element-width move: it wrote only `D`'s low
///   element and left the rest of `D` alone.  After the rewrite the rest of
///   `D` is whatever the producer puts there, so a reader of `D` wider than
///   the copy-out's element could see a difference: the rewrite is refused
///   when any line of the function outside the two rewritten ones mentions
///   `D` wider than that element.  This is the whole upper-bit argument and
///   it does not depend on which source the producer merged from, or on
///   whether `D` was itself a source.
/// * Four-operand variable blends (`vblendvps`/`vblendvpd`) retarget like the
///   three-operand logical ops: the mask is a fourth source read, the dest a
///   plain output, and the blend's low element comes from its sources' low
///   elements alone.  VEX reads every source before writing, so the rewrite
///   is sound even when `D` is itself the mask — the S05 FP-select-into-
///   return shape, where the mask was just computed into the return
///   register and the blend result is only copied back out to it.
/// * The producer's old destination `T` is dead after the copy-out, asked of
///   [`FpLiveness`]: the rewrite stops writing it.
/// * The two lines are adjacent up to nops, in one basic block, neither is
///   pinned, and every register operand is spelled `%xmm`.
/// Longest straight-line interior `retarget_vex_result` renames between the
/// producer and the copy-out.  Small by design: the rule is a local
/// bracket fold, not a register allocator.
const MAX_RETARGET_INTERIOR: usize = 6;

/// An interior line whose mention of the temporary may be renamed: a
/// VEX-spelled instruction (`v…`), whose register operands are all written
/// with the VEX zero-above semantics -- so after the rename every write to
/// the new destination is a full `pw`-wide VEX write, exactly what the
/// deleted copy-out produced.  Legacy SSE spellings merge into the upper
/// bits and are refused.
fn vex_rename_safe(t: &str) -> bool {
    let (mn, _) = split_mn(t);
    mn.starts_with('v')
        && !mn.starts_with("vmovs")
        && !mn.starts_with("vmovl")
        && !mn.starts_with("vmovh")
}

/// A VEX full-width vector load from memory: writes every bit of the
/// spelled destination from memory alone and zeroes above it, so its
/// destination is a plain output exactly like the arithmetic producers
/// (`vmovups (%rdi), %ymm2; vmovdqa %ymm2, %ymm0` is the whole-register
/// `memcpy`-into-return shape).  Scalar (`vmovss`/`vmovsd`), half
/// (`vmovlps`/`vmovhps`), broadcast and legacy spellings are excluded.
fn full_width_load_zeroes_above(mn: &str) -> bool {
    matches!(
        mn,
        "vmovups" | "vmovaps" | "vmovupd" | "vmovapd" | "vmovdqu" | "vmovdqa" | "vlddqu"
    )
}

/// A legacy-SSE full-width load from memory: writes bits 127:0 of its
/// destination from memory alone and leaves the bits above untouched --
/// exactly what a legacy full-width register copy (`movdqa %T, %D`) does
/// to `D`, so `load -> %T; movdqa %T, %D` and `load -> %D` leave every bit
/// of `D` identical, with no upper-bit argument needed.
fn legacy_full_width_load(mn: &str) -> bool {
    matches!(
        mn,
        "movups" | "movaps" | "movupd" | "movapd" | "movdqu" | "movdqa" | "lddqu"
    )
}

/// A VEX register/memory-source broadcast (`vpbroadcast{b,w,d,q}`,
/// `vbroadcastss`/`vbroadcastsd`): the whole spelled destination is
/// written from the source's low element and the bits above are zeroed --
/// a plain output whose SOURCE is narrower than the destination (an XMM
/// or a memory operand), so the same-width source rule does not apply.
fn vex_broadcast(mn: &str) -> bool {
    matches!(
        mn,
        "vpbroadcastb"
            | "vpbroadcastw"
            | "vpbroadcastd"
            | "vpbroadcastq"
            | "vbroadcastss"
            | "vbroadcastsd"
    )
}

pub(super) fn retarget_vex_result(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut fp: Option<FpLiveness> = None;
    let mut j = 0;
    while j < len {
        if infos[j].is_nop() || infos[j].pinned {
            j += 1;
            continue;
        }
        let t_op = infos[j].trimmed(store.get(j)).to_string();
        let (mn, operands) = split_mn(&t_op);
        let is_vex_load = full_width_load_zeroes_above(mn);
        let is_legacy_load = legacy_full_width_load(mn);
        let is_full_load = is_vex_load || is_legacy_load;
        let is_broadcast = vex_broadcast(mn);
        if !(vex_plain_output(mn) || vex_int_plain_output(mn) || is_full_load || is_broadcast)
            || operands.contains('{')
        {
            j += 1;
            continue;
        }
        let ops: Vec<&str> = operand_list(operands).iter().map(|o| o.trim()).collect();
        // A full-width load: two operands, the source a memory operand
        // (a register-to-register `vmovdqa` is itself a copy, handled by
        // the copy rules), folded only into the full-width VEX copy-out
        // shape below.
        if is_full_load && !(ops.len() == 2 && !ops[0].starts_with('%') && !ops[0].starts_with('$'))
        {
            j += 1;
            continue;
        }
        // A broadcast: two operands, the source a vector register or a
        // memory operand; folded only into the full-width VEX copy-out.
        if is_broadcast && !(ops.len() == 2 && !ops[0].starts_with('$')) {
            j += 1;
            continue;
        }
        // Three operands, four with a leading immediate (`vroundsd`), or the
        // four-operand variable blend (`vblendvps`/`vblendvpd`: mask + two
        // sources + dest — the mask is a fourth source read, the dest a
        // plain output; VEX reads every source before writing, so retargeting
        // the dest is sound even when `D` is itself the mask).
        let is_var_blend = matches!(mn, "vblendvps" | "vblendvpd" | "vpblendvb");
        let arity_ok = ops.len() == 3
            || (ops.len() == 4 && ops[0].starts_with('$'))
            || (is_var_blend && ops.len() == 4)
            || is_full_load
            || is_broadcast;
        if !arity_ok {
            j += 1;
            continue;
        }
        // The producer's spelled width `pw` (128/256/512): every register
        // source must be spelled the same width (no mixed-width forms such
        // as `vpmovzx`/`vcvtps2pd`, whose read widths differ), and the
        // copy-out below must be spelled the same width too.
        let Some((t, pw)) = vec_operand(ops[ops.len() - 1]) else {
            j += 1;
            continue;
        };
        if pw != W128
            && !vex_int_plain_output(mn)
            && !vex_plain_output(mn)
            && !is_full_load
            && !is_broadcast
        {
            j += 1;
            continue;
        }
        let sources_ok = is_broadcast
            || ops[..ops.len() - 1]
                .iter()
                .all(|op| !op.starts_with('%') || vec_operand(op).is_some_and(|(_, w)| w == pw));
        if !sources_ok {
            j += 1;
            continue;
        }
        let Some((fstart, fend)) = function_range(store, infos, j) else {
            j += 1;
            continue;
        };
        // The copy-out: the first copy OF `T` after the producer, either
        // adjacent or behind a short straight-line interior that only
        // consumes `T` at the producer's width -- `t = a + b; return
        // b*t + t;` is `vaddps %ymm1,%ymm0,%ymm14; vfmadd132ps
        // %ymm1,%ymm14,%ymm14; vmovdqa %ymm14,%ymm0` (an FMA reads its
        // destination, so the FMA itself cannot be retargeted; the ADD
        // can, and the interior is renamed with it).  The interior may not
        // mention `D` at any width (it is about to hold the result) and
        // may not mention `T` at another width (the rename is exact-width),
        // and every interior write to `T` is a VEX write at `pw`, so `D`'s
        // final state is bit-identical to what the copy left.  Barriers
        // (labels, transfers, calls, pinned lines) end the search.
        let Some((t_fam_lo, _)) = vec_operand(ops[ops.len() - 1]) else {
            j += 1;
            continue;
        };
        debug_assert_eq!(t_fam_lo, t);
        let mut k_found: Option<usize> = None;
        let mut interior: Vec<usize> = Vec::new();
        let mut n = j + 1;
        while n < fend && interior.len() <= MAX_RETARGET_INTERIOR {
            if infos[n].is_nop() || infos[n].kind == LineKind::Empty {
                n += 1;
                continue;
            }
            if infos[n].pinned {
                break;
            }
            let tn = infos[n].trimmed(store.get(n));
            if let Some(c) = parse_vec_copy(tn) {
                if c.src == t {
                    k_found = Some(n);
                    break;
                }
            }
            if is_barrier(&infos[n], tn) || tn.contains('{') {
                break;
            }
            let tw = vec_mention_width(tn, t);
            if tw != 0 && tw != pw {
                break;
            }
            if tw != 0 && !vex_rename_safe(tn) {
                break;
            }
            interior.push(n);
            n += 1;
        }
        let Some(k) = k_found else {
            j += 1;
            continue;
        };
        let t_out = infos[k].trimmed(store.get(k)).to_string();
        let Some(copy_out) = parse_vec_copy(&t_out) else {
            j += 1;
            continue;
        };
        // An interior that touches `D` cannot be renamed onto it; an
        // interior is only accepted for the full-width VEX copy-out shape
        // (the scalar / legacy shapes keep their adjacency).
        if !interior.is_empty() {
            let d_fam = VEC_FAMILY_BASE + copy_out.dst;
            let d_clean = interior
                .iter()
                .all(|&n| !mentions_family(infos[n].trimmed(store.get(n)).as_bytes(), d_fam));
            let full_vex = copy_out.width == pw
                && copy_spelled_width(&t_out) == Some(pw)
                && copy_out.zeroes_above;
            if !d_clean || !full_vex {
                j += 1;
                continue;
            }
        }
        // Two copy-out shapes are folded:
        //  * a legacy element-width move (`movsd`/`movss`, width < 128) of a
        //    128-bit producer -- the original scalar rule, which needs the
        //    upper-bit argument below;
        //  * a full-width VEX move (`vmovdqa`/`vmovaps`/... spelled `pw`,
        //    zeroes above): the producer, retargeted to `D`, writes exactly
        //    the bits the copy wrote (all of `D` up to `pw`, zero above),
        //    so `D`'s final state is bit-identical and no reader of `D`
        //    can tell -- the 256-bit `vector_size` return shape
        //    (`vaddps %ymm1, %ymm0, %ymm4; vmovdqa %ymm4, %ymm0; ret`).
        //  * a legacy full-width move (`movdqa %xmm4, %xmm0` after a VEX.128
        //    producer): bits 127:0 of `D` are identical either way; the
        //    producer additionally zeroes `D`'s bits above 128 where the
        //    legacy move left them alone, so this shape keeps the wide-reader
        //    argument of the scalar rule (refused when any other line of the
        //    function mentions `D` wider than the copy).
        let copy_spelling = copy_spelled_width(&t_out);
        let scalar_copy_out = pw == W128
            && copy_out.width < W128
            && !copy_out.zeroes_above
            && copy_spelling == Some(W128);
        let full_copy_out = copy_out.width == pw && copy_spelling == Some(pw);
        let full_vex_copy_out = full_copy_out && copy_out.zeroes_above;
        if copy_out.src != t || !(scalar_copy_out || full_copy_out) {
            j += 1;
            continue;
        }
        // A load folds only into a full-width copy-out: VEX load /
        // broadcast (zeroes above) into a VEX copy, or -- like any VEX.128
        // producer -- into a legacy 128-bit copy under the wide-reader
        // argument below; a legacy load (leaves above alone) only into a
        // legacy full-width copy.
        let legacy_full_copy_out = full_copy_out && !copy_out.zeroes_above && pw == W128;
        if ((is_vex_load || is_broadcast) && !(full_vex_copy_out || legacy_full_copy_out))
            || (is_legacy_load && !legacy_full_copy_out)
        {
            j += 1;
            continue;
        }
        let d = copy_out.dst;
        // A scalar copy-out immediately followed by return cannot reach an
        // earlier wide reader again. The old whole-function approximation
        // rejected legal leaf return folds after merge widths became exact.
        // Do NOT apply this shortcut across epilogues, labels or transfers:
        // all other shapes retain the conservative whole-function check.
        let immediate_return = (k + 1..fend)
            .find(|&n| !debug_transparent(store, infos[n], n) && infos[n].kind != LineKind::Empty)
            .is_some_and(|n| infos[n].kind == LineKind::Ret && !infos[n].pinned);
        let wide_reader = (!immediate_return)
            && !full_vex_copy_out
            && (fstart..fend).any(|n| {
                n != j
                    && n != k
                    && !infos[n].is_nop()
                    && vec_mention_width(infos[n].trimmed(store.get(n)), d) > copy_out.width
            });
        if wide_reader {
            j += 1;
            continue;
        }
        let oracle = fp.get_or_insert_with(|| FpLiveness::new(store, infos));
        if !oracle.deleted_write_regions_dead_after(store, infos, k, u32::from(t), &[j, k]) {
            j += 1;
            continue;
        }
        let mut new_ops: Vec<String> = ops.iter().map(|o| (*o).to_string()).collect();
        let last = new_ops.len() - 1;
        let prefix = match pw {
            W256 => "ymm",
            W512 => "zmm",
            _ => "xmm",
        };
        new_ops[last] = format!("%{prefix}{d}");
        replace_line(
            store,
            &mut infos[j],
            j,
            format!("    {mn} {}", new_ops.join(", ")),
        );
        for &n in &interior {
            let tn = infos[n].trimmed(store.get(n)).to_string();
            let renamed = replace_vec_reg(&tn, t, d);
            if renamed != tn {
                replace_line(store, &mut infos[n], n, format!("    {renamed}"));
            }
        }
        mark_nop(&mut infos[k]);
        if let Some(oracle) = fp.as_mut() {
            oracle.refresh_at(store, infos, j);
        }
        changed = true;
        j = k + 1;
    }
    changed
}

/// Delete the `vzeroupper` of a function that never WRITES an upper YMM/ZMM
/// half itself.
///
/// The epilogue arms `vzeroupper` whenever a 256-bit register is touched,
/// including a function whose only 256-bit activity is reading its own
/// incoming `%ymm` arguments (`void st(v32f *p, v32f v) { *p = v; }` is
/// `vmovdqu %ymm0, (%rdi); vzeroupper; ret`).  The upper state such a
/// function returns with is exactly the state it was entered with: every
/// `%ymm` it reads was written by its caller, which is therefore already
/// in the dirty state and responsible for its own transition.  GCC and
/// Clang omit the `vzeroupper` here for the same reason.
///
/// Refused -- the whole function is left alone -- when any line writes a
/// `%ymm`/`%zmm` register (last operand at 256/512 bits, `vzeroall`, an
/// AVX-512 masked form), contains a `call` (the callee may dirty the state
/// on this function's behalf), or is pinned (inline asm).  Reads of 256-bit
/// registers (stores, `vextract*128` into an XMM, `vmovmsk`) are what the
/// rule exists for.
pub(super) fn elide_vzeroupper_without_upper_writes(
    store: &LineStore,
    infos: &mut [LineInfo],
) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut j = 0;
    while j < len {
        if infos[j].is_nop() || infos[j].pinned || infos[j].trimmed(store.get(j)) != "vzeroupper" {
            j += 1;
            continue;
        }
        let Some((fstart, fend)) = function_range(store, infos, j) else {
            j += 1;
            continue;
        };
        let writes_upper = (fstart..fend).any(|n| {
            if infos[n].is_nop() {
                return false;
            }
            if infos[n].pinned || infos[n].kind == LineKind::Call {
                return true;
            }
            let t = infos[n].trimmed(store.get(n));
            if t.starts_with('.') || t.ends_with(':') {
                return false;
            }
            let (mn, operands) = split_mn(t);
            if mn == "vzeroall" || mn.starts_with("call") || operands.contains('{') {
                return true;
            }
            let ops = operand_list(operands);
            ops.last()
                .is_some_and(|last| vec_operand(last.trim()).is_some_and(|(_, w)| w > W128))
        });
        if writes_upper {
            j = fend;
            continue;
        }
        for n in fstart..fend {
            if !infos[n].is_nop()
                && !infos[n].pinned
                && infos[n].trimmed(store.get(n)) == "vzeroupper"
            {
                mark_nop(&mut infos[n]);
                changed = true;
            }
        }
        j = fend;
    }
    changed
}

/// Delete a vector copy whose destination cannot be observed again.
///
/// Asked of [`FpLiveness`], not a textual scan: the oracle models the implicit
/// readers a scan cannot see (a `call` reads `%xmm0`–`%xmm7`, a `ret` reads
/// `%xmm0`/`%xmm1`) and falls closed when the enclosing function cannot be
/// analysed. Its `%ymm`/`%zmm` aliasing is what makes "dead" mean dead for a
/// register a later wide instruction still reads.
pub(super) fn eliminate_dead_vector_copies(store: &LineStore, infos: &mut [LineInfo]) -> bool {
    let mut fp: Option<FpLiveness> = None;
    let mut changed = false;
    for i in 0..store.len() {
        if infos[i].is_nop() || infos[i].pinned {
            continue;
        }
        let t = infos[i].trimmed(store.get(i));
        let Some(copy) = parse_vec_copy(t) else {
            continue;
        };
        // `parse_vec_copy` has already declined self-copies: a VEX one is not a
        // no-op (it zeroes the bits above the lane it copies), so it is not this
        // pass's business.
        if fp
            .get_or_insert_with(|| FpLiveness::new(store, infos))
            .deleted_write_regions_dead_after(store, infos, i, u32::from(copy.dst), &[i])
        {
            mark_nop(&mut infos[i]);
            changed = true;
        }
    }
    changed
}

/// Scalar instructions whose result takes bits 127:W of the destination from
/// the penultimate operand (`vaddsd %b, %a, %d`: d = [a.hi : a.lo + b.lo]).
/// Their non-penultimate vector sources are read at most 64 bits wide.
fn vex_scalar_upper_from_src1(mn: &str) -> bool {
    matches!(
        mn,
        "vaddsd"
            | "vsubsd"
            | "vmulsd"
            | "vdivsd"
            | "vminsd"
            | "vmaxsd"
            | "vsqrtsd"
            | "vroundsd"
            | "vaddss"
            | "vsubss"
            | "vmulss"
            | "vdivss"
            | "vminss"
            | "vmaxss"
            | "vsqrtss"
            | "vroundss"
            | "vrcpss"
            | "vrsqrtss"
            | "vcvtss2sd"
            | "vcvtsd2ss"
            | "vcvtsi2sd"
            | "vcvtsi2sdl"
            | "vcvtsi2sdq"
            | "vcvtsi2ss"
            | "vcvtsi2ssl"
            | "vcvtsi2ssq"
    )
}

/// Legacy two-operand scalar instructions: the destination keeps its bits
/// above the scalar lane, and any vector source is read at most 64 bits wide.
fn legacy_scalar_merge(mn: &str) -> bool {
    matches!(
        mn,
        "addsd"
            | "subsd"
            | "mulsd"
            | "divsd"
            | "minsd"
            | "maxsd"
            | "sqrtsd"
            | "roundsd"
            | "addss"
            | "subss"
            | "mulss"
            | "divss"
            | "minss"
            | "maxss"
            | "sqrtss"
            | "roundss"
            | "rcpss"
            | "rsqrtss"
            | "cvtss2sd"
            | "cvtsd2ss"
            | "cvtsi2sd"
            | "cvtsi2sdl"
            | "cvtsi2sdq"
            | "cvtsi2ss"
            | "cvtsi2ssl"
            | "cvtsi2ssq"
    )
}

/// Scalar FMA: every operand is read in its low lane only and the
/// destination keeps its own bits above that lane.
fn scalar_fma(mn: &str) -> bool {
    let Some(rest) = ["vfmadd", "vfmsub", "vfnmadd", "vfnmsub"]
        .iter()
        .find_map(|p| mn.strip_prefix(p))
    else {
        return false;
    };
    matches!(
        rest,
        "132sd" | "213sd" | "231sd" | "132ss" | "213ss" | "231ss"
    )
}

/// True when `text` spells vector register `n` at any width, on an exact
/// digit boundary (`%xmm1` is not named by `%xmm15`).
fn names_vec_reg(text: &str, n: u8) -> bool {
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(off) = text[from..].find('%') {
        let at = from + off + 1;
        from = at;
        let Some(rest) = ["xmm", "ymm", "zmm"]
            .iter()
            .find_map(|w| text[at..].strip_prefix(w))
        else {
            continue;
        };
        let start = bytes.len() - rest.len();
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 && rest[..digits].parse::<u32>().ok() == Some(u32::from(n)) {
            return true;
        }
        from = start;
    }
    false
}

/// What one line does to bits 127:64 of `%xmm<n>`, given that the line
/// mentions the register.
enum UpperFate {
    /// Overwritten without being read: the old upper half is dead here.
    Killed,
    /// Neither read nor overwritten (low-lane reads, upper-preserving
    /// writes into `n` itself): keep looking.
    Carried,
    /// Possibly read, copied elsewhere, or not modelled.
    Observed,
}

fn upper64_fate(t: &str, n: u8) -> UpperFate {
    let (mn, operands) = split_mn(t);
    let ops = operand_list(operands);
    let Some(last) = ops.len().checked_sub(1) else {
        return UpperFate::Observed;
    };
    let mut mentions = Vec::with_capacity(ops.len());
    for (k, op) in ops.iter().enumerate() {
        match vec_operand(op) {
            Some((num, W128)) if num == n => mentions.push(k),
            // A wide spelling reads or writes the bits in question.
            Some((num, _)) if num == n => return UpperFate::Observed,
            // Named inside a compound operand (a VSIB index, say).
            None if names_vec_reg(op, n) => return UpperFate::Observed,
            _ => {}
        }
    }
    let is_n = |k: usize| vec_operand(ops[k]) == Some((n, W128));
    let is_vec = |k: usize| vec_operand(ops[k]).is_some();
    let src_mentions = mentions.iter().any(|&k| k != last);
    if reads_only_mnemonic(mn) {
        return if matches!(mn, "comisd" | "ucomisd" | "vcomisd" | "vucomisd")
            || matches!(mn, "comiss" | "ucomiss" | "vcomiss" | "vucomiss")
        {
            UpperFate::Carried
        } else {
            UpperFate::Observed
        };
    }
    if is_n(last) {
        return match mn {
            // Zero-extending loads and moves; a 64/32-bit vector source is
            // read in its low lane only.
            "movq" | "vmovq" | "movd" | "vmovd" => UpperFate::Killed,
            "movsd" | "movss" | "vmovsd" | "vmovss" if ops.len() == 2 => {
                if is_vec(0) {
                    // Register form merges: the destination keeps its upper.
                    if mn.starts_with('v') {
                        UpperFate::Observed
                    } else {
                        UpperFate::Carried
                    }
                } else {
                    UpperFate::Killed
                }
            }
            // `vmovsd %a, %b, %d`: d = [b.hi : a.lo].
            "vmovsd" | "vmovss" if ops.len() == 3 => {
                if is_n(1) {
                    UpperFate::Carried
                } else {
                    UpperFate::Killed
                }
            }
            "movapd" | "movaps" | "movupd" | "movups" | "movdqa" | "movdqu" | "vmovapd"
            | "vmovaps" | "vmovupd" | "vmovups" | "vmovdqa" | "vmovdqu"
                if ops.len() == 2 =>
            {
                if is_n(0) {
                    UpperFate::Carried
                } else {
                    UpperFate::Killed
                }
            }
            "xorpd" | "xorps" | "pxor" | "vxorpd" | "vxorps" | "vpxor"
                if mentions.len() == ops.len() =>
            {
                UpperFate::Killed
            }
            _ if scalar_fma(mn) && ops.len() == 3 => UpperFate::Carried,
            _ if legacy_scalar_merge(mn) => UpperFate::Carried,
            _ if vex_scalar_upper_from_src1(mn) && ops.len() >= 3 => {
                if is_n(last - 1) {
                    UpperFate::Carried
                } else {
                    UpperFate::Killed
                }
            }
            _ => UpperFate::Observed,
        };
    }
    debug_assert!(src_mentions);
    // `n` is only a source here.
    match mn {
        "movsd" | "movss" | "vmovsd" | "vmovss" | "movq" | "vmovq" | "movd" | "vmovd"
            if ops.len() == 2 =>
        {
            UpperFate::Carried
        }
        "vmovsd" | "vmovss" if ops.len() == 3 && !is_n(1) => UpperFate::Carried,
        "cvtsd2si" | "cvttsd2si" | "vcvtsd2si" | "vcvttsd2si" | "cvtss2si" | "cvttss2si"
        | "vcvtss2si" | "vcvttss2si" | "cvtsd2siq" | "cvttsd2siq" | "vcvtsd2siq"
        | "vcvttsd2siq" | "cvtss2siq" | "cvttss2siq" | "vcvtss2siq" | "vcvttss2siq"
        | "cvtsd2sil" | "cvttsd2sil" | "vcvtsd2sil" | "vcvttsd2sil" | "cvtss2sil"
        | "cvttss2sil" | "vcvtss2sil" | "vcvttss2sil" => UpperFate::Carried,
        _ if scalar_fma(mn) && ops.len() == 3 => UpperFate::Carried,
        _ if legacy_scalar_merge(mn) => UpperFate::Carried,
        // The penultimate operand donates its upper half to another register.
        _ if vex_scalar_upper_from_src1(mn) && ops.len() >= 3 && !is_n(last - 1) => {
            UpperFate::Carried
        }
        _ => UpperFate::Observed,
    }
}

/// Delete a legacy `movq %xmmN, %xmmN` whose only effect — zeroing bits
/// 127:64 — no instruction can observe.
///
/// Store forwarding replaces a `movsd slot, %xmmN` reload of a value `%xmmN`
/// still holds with this zeroing self-move, because the memory form zeroes
/// the upper half and the register forms merge (Intel SDM, MOVSD/MOVQ). The
/// replacement is exact, but on a scalar FP chain it is one more µop and one
/// more cycle of latency (`movq` is not move-eliminated), and it usually
/// guards nothing: the scalar code that follows reads the low lane only.
///
/// The proof is a forward scan inside the basic block. Lines that do not
/// mention the register pass (barriers — labels, branches, calls, returns,
/// pinned or implicit-effect lines — end the scan undecided). A mentioning
/// line either overwrites bits 127:64 without reading them (the zeroing was
/// dead: delete), leaves them in place unread (keep scanning), or may read or
/// copy them (keep the move). VEX self-moves also zero bits 255:128 and are
/// left alone.
pub(super) fn eliminate_unobserved_zeroing_self_moves(
    store: &LineStore,
    infos: &mut [LineInfo],
) -> bool {
    const SCAN_LIMIT: usize = 64;
    let mut changed = false;
    for i in 0..store.len() {
        if infos[i].is_nop() || infos[i].pinned {
            continue;
        }
        let t = infos[i].trimmed(store.get(i));
        let Some(rest) = t.strip_prefix("movq ") else {
            continue;
        };
        let ops = operand_list(rest);
        let n = match (ops.as_slice(), ops.first().and_then(|o| vec_operand(o))) {
            ([a, b], Some((n, W128))) if vec_operand(b) == Some((n, W128)) && a == b => n,
            _ => continue,
        };
        let mut seen = 0;
        let mut dead = false;
        for k in i + 1..store.len() {
            if infos[k].is_nop() {
                continue;
            }
            let u = infos[k].trimmed(store.get(k));
            if is_barrier(&infos[k], u) || infos[k].kind == LineKind::Directive {
                if infos[k].kind == LineKind::Directive && benign_directive(u) {
                    continue;
                }
                break;
            }
            seen += 1;
            if seen > SCAN_LIMIT {
                break;
            }
            if !names_vec_reg(u, n) {
                continue;
            }
            match upper64_fate(u, n) {
                UpperFate::Killed => {
                    dead = true;
                    break;
                }
                UpperFate::Carried => {}
                UpperFate::Observed => break,
            }
        }
        if dead {
            mark_nop(&mut infos[i]);
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE_CALL_ASM: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/regression/fixtures/peephole-wide-call/caller.s"
    ));

    #[test]
    fn wide_call_real_pipeline_structure() {
        // This half must run even where the native execution contract cannot.
        assert!(WIDE_CALL_ASM.contains("movq %rax, %rax"));
        let optimized = run(WIDE_CALL_ASM);
        assert!(
            !optimized.contains("movq %rax, %rax"),
            "real pipeline positive control"
        );
    }

    #[test]
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    fn pr834_wide_call_real_pipeline_native() {
        // Deliberately fail rather than report a successful non-execution.
        // The structural sibling above has no host ISA/toolchain dependency.
        use crate::common::temp_files::TempFile;
        use std::process::Command;
        let avx = std::arch::is_x86_feature_detected!("avx");
        let gcc = Command::new("gcc")
            .arg("--version")
            .output()
            .map(|out| out.status);
        assert!(
            avx && gcc.as_ref().is_ok_and(|status| status.success()),
            "native wide-call contract requires BOTH an AVX-capable x86_64-linux host \
             AND gcc on PATH (AVX={avx}, GCC probe={gcc:?}); \
             wide_call_real_pipeline_structure runs without either requirement"
        );
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/regression/fixtures/peephole-wide-call");
        let original = WIDE_CALL_ASM.to_owned();
        let optimized = run(&original);
        let asm = TempFile::try_new("pr834", "wide-call", "s").unwrap();
        let exe = TempFile::try_new("pr834", "wide-call", "exe").unwrap();
        for (stage, code) in [("original", original), ("optimized", optimized)] {
            std::fs::write(asm.path(), &code).unwrap();
            let compiled = Command::new("gcc")
                .args(["-O2", "-mavx", "-Werror", "-o"])
                .arg(exe.path())
                .arg(fixtures.join("driver.c"))
                .arg(asm.path())
                .output()
                .expect("GCC typed wide-argument witness");
            assert!(
                compiled.status.success(),
                "{stage}: {}",
                String::from_utf8_lossy(&compiled.stderr)
            );
            let output = Command::new(exe.path()).output().unwrap();
            assert!(
                output.status.success(),
                "{stage}: {}\n{code}",
                String::from_utf8_lossy(&output.stdout)
            );
            assert_eq!(output.stdout, b"9 2 3 4\n", "{stage}");
        }
    }

    #[test]
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    fn pr832_fixture_witnesses_survive_the_real_peephole_pipeline() {
        use crate::common::fx_hash::FxHashSet;
        use crate::common::temp_files::TempFile;
        use std::process::Command;
        let fixtures = [
            ("witness.s", "driver.c"),
            ("implicit-witness.s", "implicit-driver.c"),
        ];
        for (fixture, driver) in fixtures {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
            let path = root
                .join("tests/regression/fixtures/peephole-vex-flags")
                .join(fixture);
            let original = std::fs::read_to_string(&path).unwrap();
            let expanded = Command::new("python3")
                .arg(root.join("scripts/expand_asm_fixture.py"))
                .arg(&path)
                .output()
                .unwrap();
            assert!(
                expanded.status.success(),
                "{}",
                String::from_utf8_lossy(&expanded.stderr)
            );
            let fixture = String::from_utf8(expanded.stdout).unwrap();
            // GCC's architectural fixture needs no CFI. Supply the same function
            // boundaries as compiler output so the optimizer cannot vacuously
            // skip it. This test does not claim to validate unwind metadata.
            let names: FxHashSet<_> = fixture
                .lines()
                .filter_map(|line| {
                    line.trim()
                        .strip_prefix(".type ")
                        .and_then(|rest| rest.split(',').next())
                })
                .collect();
            let mut input = String::new();
            for line in fixture.lines() {
                if line.trim().starts_with(".size ") {
                    input.push_str(".cfi_endproc\n");
                }
                input.push_str(line);
                input.push('\n');
                if names.contains(line.trim().trim_end_matches(':')) {
                    input.push_str(".cfi_startproc\n");
                }
            }
            input.push_str(".text\n.type active_canary,@function\nactive_canary:\n.cfi_startproc\n    movq %rax, %rax\n    ret\n.cfi_endproc\n.size active_canary,.-active_canary\n");
            let optimized = run(&input);
            assert!(
                !optimized.contains("movq %rax, %rax"),
                "pipeline must run its positive canary"
            );
            if !std::arch::is_x86_feature_detected!("avx2") {
                eprintln!(
                    "SKIP native fixture execution: AVX2 unavailable; pipeline canary passed"
                );
                continue;
            }
            let assembly = TempFile::try_new("pr832", "fixture", "s")
                .expect("test temporary-file reservation");
            let binary = TempFile::try_new("pr832", "fixture", "exe")
                .expect("test temporary-file reservation");
            let driver = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/regression/fixtures/peephole-vex-flags")
                .join(driver);
            let mut expected = None;
            for (stage, text) in [
                ("GAS original", original),
                ("expanded", input),
                ("optimized", optimized),
            ] {
                std::fs::write(assembly.path(), &text).unwrap();
                let built = Command::new("gcc")
                    .args(["-O2", "-mavx2", "-o"])
                    .arg(binary.path())
                    .arg(&driver)
                    .arg(assembly.path())
                    .output()
                    .unwrap();
                assert!(
                    built.status.success(),
                    "{stage}: {}",
                    String::from_utf8_lossy(&built.stderr)
                );
                let result = Command::new(binary.path()).output().unwrap();
                assert!(
                    result.status.success(),
                    "{stage}: {}\n{text}",
                    String::from_utf8_lossy(&result.stdout)
                );
                if let Some(expected) = &expected {
                    assert_eq!(&result.stdout, expected);
                } else {
                    expected = Some(result.stdout);
                }
            }
        }
    }

    #[test]
    fn pr824_external_redteam_copy_preserves_architectural_zeros() {
        for overwrite in [
            "pshufd $27, %xmm5, %xmm7",
            "pshufhw $27, %xmm5, %xmm7",
            "movapd %xmm5, %xmm7",
            "pxor %xmm7, %xmm7",
        ] {
            let asm = body(&format!(
                "    vmovapd %xmm6, %xmm7\n    {overwrite}\n    vmovupd %ymm7, (%rdi)\n    ret\n"
            ));
            let store = LineStore::new(asm);
            let mut infos: Vec<_> = (0..store.len())
                .map(|i| classify_line(store.get(i)))
                .collect();
            eliminate_dead_vector_copies(&store, &mut infos);
            let i = (0..store.len())
                .find(|&i| store.get(i).contains("vmovapd %xmm6"))
                .unwrap();
            assert!(
                !infos[i].is_nop(),
                "{overwrite}: upper zeros remain observed"
            );
        }
    }

    #[test]
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    fn pr824_external_redteam_pipeline_upper_zero_native_witness() {
        if !std::arch::is_x86_feature_detected!("avx2") {
            eprintln!("SKIP native upper-zero witness: AVX2 unavailable");
            return;
        }
        use crate::common::temp_files::TempFile;
        use std::process::Command;
        let source = TempFile::try_new("pr824", "upper-witness", "c")
            .expect("reserve architectural fixture");
        let assembly = TempFile::try_new("pr824", "upper-witness", "s")
            .expect("reserve architectural fixture");
        let binary = TempFile::try_new("pr824", "upper-witness", "exe")
            .expect("reserve architectural fixture");
        std::fs::write(source.path(), "extern void witness(void*); int main(void) { unsigned long long out[4]; witness(out); return out[2] != 0 || out[3] != 0; }\n").unwrap();
        for overwrite in [
            "pshufd $27, %xmm5, %xmm7",
            "pshufhw $27, %xmm5, %xmm7",
            "movapd %xmm5, %xmm7",
            "pxor %xmm7, %xmm7",
        ] {
            let original = format!(
                ".text\n.globl witness\n.type witness,@function\nwitness:\n.cfi_startproc\n    vpcmpeqd %ymm7, %ymm7, %ymm7\n    vxorpd %xmm5, %xmm5, %xmm5\n    vxorpd %xmm6, %xmm6, %xmm6\n    vmovapd %xmm6, %xmm7\n    {overwrite}\n    vmovupd %ymm7, (%rdi)\n    ret\n.cfi_endproc\n.size witness,.-witness\n.section .note.GNU-stack,\"\",@progbits\n"
            );
            for (stage, code) in [
                ("original", original.clone()),
                ("optimized", run(&original)),
            ] {
                std::fs::write(assembly.path(), &code).unwrap();
                let compiled = Command::new("gcc")
                    .args(["-Werror", "-o"])
                    .arg(binary.path())
                    .arg(source.path())
                    .arg(assembly.path())
                    .output()
                    .expect("GCC fixture assembler/driver");
                assert!(
                    compiled.status.success(),
                    "{stage}: {}",
                    String::from_utf8_lossy(&compiled.stderr)
                );
                let output = Command::new(binary.path()).output().unwrap();
                assert!(
                    output.status.success(),
                    "{stage} {overwrite}: upper bytes are not zero\n{code}"
                );
            }
        }
    }

    #[test]
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    fn pr832_gcc_architectural_fixtures_through_real_peephole_pipeline() {
        if !std::arch::is_x86_feature_detected!("avx2") {
            eprintln!("SKIP native VEX fixtures: AVX2 unavailable; effect tests still mandatory");
            return;
        }
        use crate::common::temp_files::TempFile;
        use std::process::Command;
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/regression/fixtures/peephole-vex-flags");
        let assembly =
            TempFile::try_new("pr832", "pipeline", "s").expect("reserve architectural fixture");
        let object =
            TempFile::try_new("pr832", "expanded", "o").expect("reserve architectural fixture");
        let binary =
            TempFile::try_new("pr832", "pipeline", "exe").expect("reserve architectural fixture");
        for (source, driver, functions) in [
            ("witness.s", "driver.c", 22),
            ("implicit-witness.s", "implicit-driver.c", 18),
        ] {
            let mut original = if source == "implicit-witness.s" {
                // GNU as, not the peepholer, expands macros and selects .if
                // arms. Only active emitted instructions have a listing
                // address. Feeding raw .macro/.if would merely test barriers.
                let listing = Command::new("as")
                    .args([
                        "-almn",
                        "--listing-lhs-width=0",
                        "--listing-lhs-width2=0",
                        "--listing-rhs-width=200",
                    ])
                    .arg(fixtures.join(source))
                    .arg("-o")
                    .arg(object.path())
                    .output()
                    .expect("GNU as macro listing");
                assert!(
                    listing.status.success(),
                    "{}",
                    String::from_utf8_lossy(&listing.stderr)
                );
                let listing = String::from_utf8(listing.stdout).unwrap();
                let mut expanded = String::from(".text\n");
                for line in listing.lines() {
                    let Some((address, text)) = line.split_once("\t>") else {
                        continue;
                    };
                    let text = text.trim();
                    if text.starts_with(".globl ")
                        || text.starts_with(".type ")
                        || text.starts_with(".size ")
                        || text.ends_with(':')
                        || (!text.starts_with('.') && address.split_whitespace().count() == 2)
                    {
                        expanded.push_str(text);
                        expanded.push('\n');
                    }
                }
                expanded.push_str(".section .note.GNU-stack,\"\",@progbits\n");
                assert!(
                    !expanded.contains(".macro")
                        && !expanded.contains(".if")
                        && !expanded.contains('\\')
                );
                expanded
            } else {
                std::fs::read_to_string(fixtures.join(source)).unwrap()
            };
            assert_eq!(
                original
                    .lines()
                    .filter(|line| line.starts_with(".type "))
                    .count(),
                functions
            );
            // A positive control proves this is an active optimizing pipeline,
            // not an identity wrapper which merely assembles the fixtures.
            original.push_str(".text\n.globl pipeline_positive\n.type pipeline_positive,@function\npipeline_positive:\n.cfi_startproc\n    movq %rax, %rax\n    ret\n.cfi_endproc\n.size pipeline_positive,.-pipeline_positive\n");
            let optimized = run(&original);
            assert!(
                !optimized.contains("movq %rax, %rax"),
                "pipeline must remove positive-control self-copy"
            );
            let mut baseline = None;
            for (stage, code) in [("original", original), ("optimized", optimized)] {
                std::fs::write(assembly.path(), &code).unwrap();
                let built = Command::new("gcc")
                    .args(["-Werror", "-o"])
                    .arg(binary.path())
                    .arg(fixtures.join(driver))
                    .arg(assembly.path())
                    .output()
                    .expect("GCC architectural fixture driver");
                assert!(
                    built.status.success(),
                    "{source} {stage}: {}",
                    String::from_utf8_lossy(&built.stderr)
                );
                let result = Command::new(binary.path()).output().unwrap();
                assert!(
                    result.status.success(),
                    "{source} {stage}: {}\n{code}",
                    String::from_utf8_lossy(&result.stdout)
                );
                if let Some(expected) = &baseline {
                    assert_eq!(
                        &result.stdout, expected,
                        "original and deliberate-mutant results survive optimization"
                    );
                } else {
                    baseline = Some(result.stdout);
                }
            }
        }
    }

    #[test]
    fn pr824_external_redteam_masked_copy_is_not_an_unconditional_relation() {
        assert!(parse_vec_copy("vmovapd %xmm6, %xmm7{%k1}").is_none());
        assert!(parse_vec_copy("vmovapd %ymm6, %ymm7{%k1}{z}").is_none());
        assert!(parse_vec_copy("vmovapd %xmm6, %xmm7").is_some());
    }

    fn run(asm: &str) -> String {
        super::super::super::peephole_optimize(asm.to_string())
    }

    /// Count the instruction lines that survive: not directives, labels,
    /// comments or blanks.
    fn insns(asm: &str) -> Vec<String> {
        asm.lines()
            .map(str::trim_end)
            .filter(|l| {
                let t = l.trim();
                !t.is_empty() && !t.starts_with('.') && !t.starts_with('#') && !t.ends_with(':')
            })
            .map(|l| l.trim().to_string())
            .collect()
    }

    #[test]
    fn pr834_narrow_cache_refreshes_added_reads_and_retries_removed_reads() {
        for adding in [true, false] {
            let target = if adding { 10 } else { 12 };
            let head = "vaddsd %xmm1, %xmm12, %xmm12";
            let op = format!("vaddsd %xmm1, %xmm{target}, %xmm{target}");
            let text = body(&format!(
                ".Lhead:\n    {head}\n    vxorps %xmm{target}, %xmm{target}, %xmm{target}\n    {op}\n    movq %xmm{target}, %rax\n    jnz .Lhead\n    ret\n"
            ));
            let mut store = LineStore::new(text);
            let mut infos: Vec<_> = (0..store.len())
                .map(|i| classify_line(store.get(i)))
                .collect();
            let producer = (0..store.len())
                .rfind(|&i| store.get(i).trim() == op)
                .unwrap();
            let header = (0..store.len())
                .find(|&i| store.get(i).trim() == head)
                .unwrap();
            let mut fp = PropagationLiveness::default();
            assert_eq!(
                fp.allows_narrow_result(&store, &infos, producer, target, W64),
                adding
            );
            let source = if adding { 10 } else { 0 };
            replace_line(
                &mut store,
                &mut infos[header],
                header,
                format!("    vaddsd %xmm1, %xmm{source}, %xmm12"),
            );
            fp.narrow_rename(source, 12);
            super::super::fp_liveness::take_refresh_count();
            assert_eq!(
                fp.allows_narrow_result(&store, &infos, producer, target, W64),
                !adding
            );
            assert_eq!(super::super::fp_liveness::take_refresh_count(), 1);
        }
    }

    #[test]
    fn pr834_lazy_narrow_facts_match_eager_refresh_on_mixed_cfgs() {
        for seed in 0..64usize {
            let mut code = String::from(".Lhead:\n");
            for group in 0..12 {
                let dst = 8 + (seed + group) % 8;
                let src = if (seed + group) % 3 == 0 {
                    8 + (seed * 3 + group + 1) % 8
                } else {
                    0
                };
                let copy = if (seed + group) % 5 == 0 {
                    "movapd"
                } else {
                    "movsd"
                };
                code.push_str(&format!("    {copy} %xmm{src}, %xmm{dst}\n    vaddsd %xmm1, %xmm{dst}, %xmm{dst}\n    movq %xmm{dst}, %rax\n"));
                if (seed + group) % 2 == 0 {
                    code.push_str(&format!("    vxorps %xmm{dst}, %xmm{dst}, %xmm{dst}\n"));
                }
                if (seed + group) % 7 == 0 {
                    code.push_str(&format!("    movupd %xmm{dst}, (%rdi)\n"));
                }
            }
            if seed % 2 == 0 {
                code.push_str("    jnz .Lhead\n");
            }
            code.push_str("    ret\n");
            let text = body(&code);
            let mut lazy = LineStore::new(text.clone());
            let mut eager = LineStore::new(text);
            let mut li: Vec<_> = (0..lazy.len())
                .map(|i| classify_line(lazy.get(i)))
                .collect();
            let mut ei = li.clone();
            assert_eq!(
                propagate_vector_copies_impl::<false>(&mut lazy, &mut li),
                propagate_vector_copies_impl::<true>(&mut eager, &mut ei),
                "seed={seed}"
            );
            for i in 0..lazy.len() {
                assert_eq!(lazy.get(i), eager.get(i), "seed={seed} line={i}");
            }
        }
    }

    fn narrow_copy_stress(groups: usize) -> usize {
        let mut text = String::from("    .cfi_startproc\n");
        for _ in 0..groups {
            text.push_str("    movsd %xmm0, %xmm10\n    vaddsd %xmm1, %xmm10, %xmm10\n    movq %xmm10, %rax\n    vxorps %xmm10, %xmm10, %xmm10\n");
        }
        text.push_str("    ret\n    .cfi_endproc\n");
        let mut store = LineStore::new(text);
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        super::super::fp_liveness::take_refresh_count();
        let start = std::time::Instant::now();
        assert!(propagate_vector_copies(&mut store, &mut infos));
        let elapsed = start.elapsed();
        let refreshes = super::super::fp_liveness::take_refresh_count();
        let forwarded = (0..store.len())
            .filter(|&i| store.get(i).trim() == "vaddsd %xmm1, %xmm0, %xmm10")
            .count();
        assert_eq!(
            forwarded, groups,
            "stress must exercise every narrow forwarding"
        );
        println!(
            "PR834_STRESS groups={groups} instructions={} refreshes={refreshes} pass_ns={}",
            groups * 4,
            elapsed.as_nanos()
        );
        refreshes
    }

    #[test]
    fn pr834_independent_narrow_copies_do_not_reanalyse_each_rewrite() {
        // Intentional complexity-policy tripwire, not a wall-time assertion.
        // Change this limit only with a justified cache change and updated
        // stress evidence in the same commit, not to bless a regression.
        assert!(
            narrow_copy_stress(8) <= 1,
            "independent copies should reuse conservative liveness"
        );
    }

    #[test]
    #[ignore = "manual 10k-instruction compiler-cost benchmark; no timing gate"]
    fn pr834_ten_thousand_instruction_narrow_copy_cost() {
        narrow_copy_stress(2500);
    }

    #[test]
    fn pr832_merge_forwarding_requires_a_narrow_result_and_cfg_deadness() {
        for (copy, op, extract) in [
            ("movsd", "vaddsd", "movq %xmm10, %rax"),
            ("movss", "vaddss", "movd %xmm10, %eax"),
        ] {
            for (tail, allowed) in [
                (format!("{extract}\n    ret"), true),
                ("movupd %xmm10, (%rdi)\n    ret".into(), false),
                (format!("{extract}\n    fxsave (%rdi)\n    ret"), false),
                (format!("{extract}\n    jmp .Lobserve"), false),
                (format!("{extract}\n    jmp *%rdi"), false),
            ] {
                let asm = body(&format!(
                    ".Lobserve:\n    movupd %xmm10, (%rdi)\n    {copy} %xmm0, %xmm10\n    {op} %xmm1, %xmm10, %xmm10\n    {tail}\n"
                ));
                let mut store = LineStore::new(asm);
                let mut infos: Vec<_> = (0..store.len())
                    .map(|i| classify_line(store.get(i)))
                    .collect();
                assert_eq!(
                    propagate_vector_copies(&mut store, &mut infos),
                    allowed,
                    "{copy}: {tail}"
                );
            }
        }
    }

    #[test]
    fn pr832_narrow_merge_must_preserve_returns_and_intermediate_bits() {
        for (register, copy, op, tail) in [
            (0, "movsd", "vaddsd", "movq %xmm0, %rax\n    ret"),
            (10, "movss", "vaddss", "movq %xmm10, %rax\n    ret"),
            (
                10,
                "movsd",
                "vaddsd",
                "vaddsd %xmm1, %xmm10, %xmm4\n    movupd %xmm4, (%rdi)\n    ret",
            ),
            (0, "movsd", "vaddsd", "call external\n    ret"),
        ] {
            let asm = body(&format!(
                "    # LCCC_RET_XMM 1\n    {copy} %xmm5, %xmm{register}\n    {op} %xmm1, %xmm{register}, %xmm{register}\n    {tail}\n"
            ));
            let mut store = LineStore::new(asm);
            let mut infos: Vec<_> = (0..store.len())
                .map(|i| classify_line(store.get(i)))
                .collect();
            assert!(!propagate_vector_copies(&mut store, &mut infos), "{tail}");
        }
    }

    #[test]
    fn pr832_return_fold_ignores_only_unreachable_earlier_wide_readers() {
        for tail in [
            "ret",
            "vmovupd %xmm0, (%rdi)\n    ret",
            "jne .Lagain\n    ret",
        ] {
            let asm = body(&format!(
                ".Lagain:\n    vaddpd %xmm0, %xmm1, %xmm1\n    vaddsd %xmm2, %xmm3, %xmm4\n    movsd %xmm4, %xmm0\n    {tail}\n"
            ));
            let mut store = LineStore::new(asm);
            let mut infos: Vec<_> = (0..store.len())
                .map(|i| classify_line(store.get(i)))
                .collect();
            assert_eq!(
                retarget_vex_result(&mut store, &mut infos),
                tail == "ret",
                "{tail}"
            );
        }
    }

    #[test]
    fn pr832_narrow_bracket_cannot_export_unestablished_upper_bits() {
        let asm = body(
            "    movss %xmm0, %xmm2\n    vaddss %xmm1, %xmm2, %xmm2\n    movapd %xmm2, %xmm0\n    ret\n",
        );
        let mut store = LineStore::new(asm);
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        assert!(!coalesce_vector_brackets(&mut store, &mut infos));
    }

    #[test]
    fn pr832_scalar_merge_sources_are_not_narrow_copy_equivalents() {
        for (copy, op) in [("movsd", "vaddsd"), ("movss", "vaddss")] {
            let asm = body(&format!(
                "    {copy} %xmm0, %xmm3\n    {op} %xmm2, %xmm3, %xmm4\n    vmovupd %xmm4, (%rdi)\n    ret\n"
            ));
            let mut store = LineStore::new(asm);
            let mut infos: Vec<_> = (0..store.len())
                .map(|i| classify_line(store.get(i)))
                .collect();
            assert!(
                !propagate_vector_copies(&mut store, &mut infos),
                "merging upper bits are distinct"
            );
            let asm = body(&format!(
                "    {copy} %xmm0, %xmm3\n    {op} %xmm3, %xmm2, %xmm4\n    vmovupd %xmm4, (%rdi)\n    ret\n"
            ));
            let mut store = LineStore::new(asm);
            let mut infos: Vec<_> = (0..store.len())
                .map(|i| classify_line(store.get(i)))
                .collect();
            assert!(
                propagate_vector_copies(&mut store, &mut infos),
                "low data operand still propagates"
            );
        }
    }

    #[test]
    fn gather_mask_is_a_nontrailing_output_not_a_renameable_read() {
        // AVX2 gathers clear the vector mask as elements complete. Its first
        // AT&T operand is read AND written, not a source-only operand.
        for op in [
            "vgatherdps",
            "vgatherdpd",
            "vgatherqps",
            "vgatherqpd",
            "vpgatherdd",
            "vpgatherdq",
            "vpgatherqd",
            "vpgatherqq",
        ] {
            let gather = format!("{op} %xmm2, (%rdi,%xmm3,4), %xmm4");
            let asm = body(&format!(
                "    vmovaps %xmm0, %xmm2\n    {gather}\n    vmovaps %xmm0, (%rsi)\n    vmovaps %xmm2, 16(%rsi)\n    vmovaps %xmm4, 32(%rsi)\n"
            ));
            let mut store = LineStore::new(asm.clone());
            let mut infos: Vec<_> = (0..store.len())
                .map(|i| classify_line(store.get(i)))
                .collect();
            assert!(
                !propagate_vector_copies(&mut store, &mut infos),
                "renaming the gather mask also clobbers the copy source: {op}"
            );
            let out = run(&asm);
            assert!(insns(&out).contains(&gather), "gather mask changed: {out}");
        }
    }

    #[test]
    fn propagation_does_not_cross_implicit_vector_outputs() {
        for op in [
            "vpcmpistrm $0, %xmm5, %xmm7",
            "vpcmpestrm $0, %xmm5, %xmm7",
            "pcmpistrm $0, %xmm5, %xmm7",
            "pcmpestrm $0, %xmm5, %xmm7",
            "fxrstor (%rdi)",
            "fxrstor64 (%rdi)",
            "xrstor (%rdi)",
            "xrstors64 (%rdi)",
        ] {
            let asm = body(&format!(
                "    movaps %xmm0, %xmm2\n    {op}\n    movaps %xmm2, %xmm3\n"
            ));
            let mut store = LineStore::new(asm);
            let mut infos: Vec<_> = (0..store.len())
                .map(|i| classify_line(store.get(i)))
                .collect();
            propagate_vector_copies(&mut store, &mut infos);
            assert!(
                (0..store.len()).any(|i| store.get(i).trim() == "movaps %xmm2, %xmm3"),
                "old XMM0 copy must not be replaced with the implicit output: {op}"
            );
        }
    }

    #[test]
    fn gather_mask_output_invalidates_copy_relations() {
        for (mnemonic, index) in [
            ("vgatherdpd", "xmm4"),
            ("vgatherdps", "ymm4"),
            ("vpgatherdd", "ymm4"),
            ("vpgatherdq", "xmm4"),
        ] {
            let asm = body(&format!(
                "    vmovapd %ymm3, %ymm2\n    {mnemonic} %ymm3, (%rdi,%{index},8), %ymm7\n    vmovupd %ymm2, (%rsi)\n"
            ));
            let mut store = LineStore::new(asm);
            let mut infos: Vec<_> = (0..store.len())
                .map(|i| classify_line(store.get(i)))
                .collect();
            propagate_vector_copies(&mut store, &mut infos);
            assert!(
                (0..store.len()).any(|i| store.get(i).trim() == "vmovupd %ymm2, (%rsi)"),
                "the gather rewrites its nontrailing mask: {mnemonic}"
            );
        }
    }

    #[test]
    fn explicit_only_vector_operations_still_admit_propagation() {
        let asm = body("    movaps %xmm0, %xmm2\n    vaddps %xmm2, %xmm5, %xmm7\n");
        let mut store = LineStore::new(asm);
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        assert!(propagate_vector_copies(&mut store, &mut infos));
        assert!((0..store.len()).any(|i| store.get(i).trim() == "vaddps %xmm0, %xmm5, %xmm7"));
    }

    #[test]
    fn brackets_do_not_rename_implicit_vector_inputs() {
        for op in [
            "sha256rnds2 %xmm5, %xmm7",
            "blendvpd %xmm5, %xmm7",
            "blendvps %xmm5, %xmm7",
            "pblendvb %xmm5, %xmm7",
            "fxsave (%rdi)",
        ] {
            let asm = body(&format!(
                "    movaps %xmm6, %xmm0\n    {op}\n    movaps %xmm0, %xmm6\n"
            ));
            let mut store = LineStore::new(asm);
            let mut infos: Vec<_> = (0..store.len())
                .map(|i| classify_line(store.get(i)))
                .collect();
            assert!(
                !coalesce_vector_brackets(&mut store, &mut infos),
                "hidden input cannot be renamed: {op}"
            );
        }
    }

    #[test]
    fn propagate_stops_before_a_destination_write() {
        // The C2 chain (bb_slp_v8): `vmovss` resets %xmm0, then `shufps`
        // reads %xmm0's old lanes. Propagating the copy's source into the
        // shufps source operand (`shufps $0x00, %xmm1, %xmm0`) leaves the
        // destination read with NO operand mention, and dead-FP-move
        // elimination then deletes the reset — the shipped [s, s, t, t]
        // miscompile. A destination write ends the range BEFORE any rename.
        let asm = body(
            "    vmovss %xmm1, %xmm1, %xmm0\n    shufps $0x00, %xmm0, %xmm0\n    movdqa %xmm0, %xmm14\n",
        );
        let out = run(&asm);
        let lines = insns(&out);
        assert!(
            lines.iter().any(|l| l == "shufps $0x00, %xmm0, %xmm0"),
            "the shufps must not be renamed at its own destination write:\n{lines:?}"
        );
    }

    #[test]
    fn propagation_refuses_implicit_vector_clobbers() {
        for effect in [
            "pcmpistrm $0, %xmm5, %xmm7",
            "vpcmpestrm $0, %xmm5, %xmm7",
            "fxrstor (%rdi)",
            "fxrstorq (%rdi)",
            "xrstor64 (%rdi)",
            "xrstorq (%rdi)",
            "vfutureop %xmm5, %xmm7",
            "futureop %xmm5, %xmm7",
        ] {
            // The mask forms overwrite xmm0 without naming it; restores and
            // unknown instructions cannot certify either side of a relation.
            let asm = body(&format!(
                "    movapd %xmm0, %xmm2\n    {effect}\n    vaddpd %xmm2, %xmm3, %xmm4\n    ret\n"
            ));
            let mut store = LineStore::new(asm);
            let mut infos: Vec<_> = (0..store.len())
                .map(|i| classify_line(store.get(i)))
                .collect();
            assert!(
                !propagate_vector_copies(&mut store, &mut infos),
                "stale copy relation propagated across {effect}"
            );
        }
    }

    #[test]
    fn coalescing_refuses_implicit_state_observers() {
        let asm = body(
            "    movapd %xmm0, %xmm2\n    vaddpd %xmm3, %xmm2, %xmm2\n    fxsave (%rdi)\n    movapd %xmm2, %xmm0\n    ret\n",
        );
        let mut store = LineStore::new(asm);
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        assert!(
            !coalesce_vector_brackets(&mut store, &mut infos),
            "coalescing changed the vector state observed by FXSAVE"
        );
    }

    #[test]
    fn widening_refuses_implicit_earlier_definitions() {
        let asm = body(
            "    pcmpistrm $0, %xmm5, %xmm7\n    movsd %xmm3, %xmm0\n    vandpd %xmm4, %xmm0, %xmm0\n    ret\n",
        );
        let mut store = LineStore::new(asm);
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        assert!(
            !widen_private_vector_copies(&mut store, &mut infos),
            "the string mask already defined xmm0's upper lanes"
        );
    }

    #[test]
    fn retargeting_refuses_implicit_wide_observers() {
        let text = "    vaddsd %xmm2, %xmm3, %xmm4\n    movsd %xmm4, %xmm0\n    blendvpd %xmm5, %xmm7\n    ret\n";
        assert_eq!(
            retarget_only(text),
            insns(text),
            "the legacy blend mask consumes the implicit upper lane of xmm0"
        );
    }

    // ── whole-register copy-outs (the `vector_size` return shapes) ──────────

    #[test]
    fn retarget_folds_full_width_vex_copy_out_into_256_bit_producer() {
        // `add8`: the home-to-%ymm0 move of a register-valued vector return.
        let text = "    vaddps %ymm1, %ymm0, %ymm4\n    vmovdqa %ymm4, %ymm0\n    ret\n";
        assert_eq!(
            retarget_only(text),
            insns("    vaddps %ymm1, %ymm0, %ymm0\n    ret\n")
        );
        // Integer lanes (`vpaddd`) and the leading-immediate compare.
        let text = "    vpaddd %ymm1, %ymm0, %ymm4\n    vmovdqa %ymm4, %ymm0\n    ret\n";
        assert_eq!(
            retarget_only(text),
            insns("    vpaddd %ymm1, %ymm0, %ymm0\n    ret\n")
        );
        let text = "    vcmpps $1, %xmm1, %xmm0, %xmm4\n    movdqa %xmm4, %xmm0\n    ret\n";
        assert_eq!(
            retarget_only(text),
            insns("    vcmpps $1, %xmm1, %xmm0, %xmm0\n    ret\n")
        );
    }

    #[test]
    fn retarget_renames_a_straight_line_interior_onto_the_copy_out_register() {
        // `t = a + b; return b*t + t;`: the FMA reads its destination, so
        // only the ADD can move; the interior is renamed with it.
        let text = "    vaddps %ymm1, %ymm0, %ymm14\n    vfmadd132ps %ymm1, %ymm14, %ymm14\n    vmovdqa %ymm14, %ymm0\n    ret\n";
        assert_eq!(
            retarget_only(text),
            insns("    vaddps %ymm1, %ymm0, %ymm0\n    vfmadd132ps %ymm1, %ymm0, %ymm0\n    ret\n")
        );
        // Refused: the interior reads the copy-out register (`a` is still
        // needed there), ...
        let text = "    vaddps %ymm1, %ymm0, %ymm14\n    vfmadd132ps %ymm0, %ymm14, %ymm14\n    vmovdqa %ymm14, %ymm0\n    ret\n";
        assert_eq!(retarget_only(text), insns(text));
        // ... mentions the temporary at another width, ...
        let text = "    vaddps %ymm1, %ymm0, %ymm14\n    vaddss %xmm1, %xmm14, %xmm14\n    vmovdqa %ymm14, %ymm0\n    ret\n";
        assert_eq!(retarget_only(text), insns(text));
        // ... writes it with a legacy (upper-merging) spelling, ...
        let text = "    vaddps %ymm1, %ymm0, %ymm14\n    addps %xmm1, %xmm14\n    vmovdqa %ymm14, %ymm0\n    ret\n";
        assert_eq!(retarget_only(text), insns(text));
        // ... or has a barrier between producer and copy-out.
        let text =
            "    vaddps %ymm1, %ymm0, %ymm14\n    call f\n    vmovdqa %ymm14, %ymm0\n    ret\n";
        assert_eq!(retarget_only(text), insns(text));
    }

    #[test]
    fn retarget_full_width_copy_out_needs_same_spelling_and_dead_source() {
        // Mixed spelling (a 128-bit copy of a 256-bit producer) is not a
        // whole-register copy; the producer's home stays.
        let text = "    vaddps %ymm1, %ymm0, %ymm4\n    vmovdqa %xmm4, %xmm0\n    ret\n";
        assert_eq!(retarget_only(text), insns(text));
        // The old destination is read again after the copy: no fold.
        let text = "    vaddps %ymm1, %ymm0, %ymm4\n    vmovdqa %ymm4, %ymm2\n    vaddps %ymm4, %ymm2, %ymm0\n    ret\n";
        assert_eq!(retarget_only(text), insns(text));
        // A legacy full-width copy leaves bits above 128 of the destination
        // alone where the VEX producer would zero them: refused when a wide
        // reader of the destination exists, folded for a leaf return.
        let text = "    vaddps %xmm1, %xmm0, %xmm4\n    movaps %xmm4, %xmm3\n    vaddps %ymm3, %ymm2, %ymm0\n    ret\n";
        assert_eq!(retarget_only(text), insns(text));
    }

    #[test]
    fn vex_int_plain_output_classifies_lane_suffixes_exactly() {
        for mn in [
            "vpaddd",
            "vpaddb",
            "vpsubq",
            "vpmulld",
            "vpmullw",
            "vpmuludq",
            "vpand",
            "vpandn",
            "vpor",
            "vpxor",
            "vpcmpeqd",
            "vpcmpgtb",
            "vpminsd",
            "vpmaxub",
            "vpsllvd",
            "vpsrad",
            "vpshufb",
            "vpblendvb",
            "vpabsd",
            "vpavgw",
            "vpunpcklbw",
            "vpackssdw",
            "vpackuswb",
        ] {
            assert!(vex_int_plain_output(mn), "{mn}");
        }
        for mn in [
            "vpinsrd",
            "vpextrd",
            "vpblendw",
            "vpmovzxbw",
            "vpmaddwd",
            "vpsadbw",
            "vpaddsbx",
            "vpadd",
            "paddd",
            "vfmadd231ps",
            "vpand d",
            "vpermd",
            "vpgatherdd",
        ] {
            assert!(!vex_int_plain_output(mn), "{mn}");
        }
    }

    const PRO: &str = "f:\n.cfi_startproc\n";
    const EPI: &str = ".cfi_endproc\n.size f, .-f\n";

    fn body(text: &str) -> String {
        format!("{PRO}{text}{EPI}")
    }

    fn uniform_shift_only(text: &str) -> (bool, String) {
        let mut store = LineStore::new(body(&format!("    # LCCC_RET_XMM 1\n{text}")));
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let changed = fold_uniform_vector_shift(&mut store, &mut infos);
        let result = (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i))
            .collect::<Vec<_>>()
            .join("\n");
        (changed, result)
    }

    #[test]
    fn uniform_shift_encodings_and_aliases() {
        for bank in ["xmm", "ymm", "zmm"] {
            for (mov, gp, broadcast, shift, expected) in [
                ("vmovd", "edi", "vpbroadcastd", "vpsllvd", "vpslld"),
                ("vmovd", "edi", "vpbroadcastd", "vpsrlvd", "vpsrld"),
                ("vmovd", "edi", "vpbroadcastd", "vpsravd", "vpsrad"),
                ("vmovq", "rdi", "vpbroadcastq", "vpsllvq", "vpsllq"),
                ("vmovq", "rdi", "vpbroadcastq", "vpsrlvq", "vpsrlq"),
                ("movd", "edi", "vpbroadcastd", "vpsllvd", "vpslld"),
                ("movq", "rdi", "vpbroadcastq", "vpsrlvq", "vpsrlq"),
            ] {
                for (count, splat, dst) in
                    [(0, 15, 0), (3, 15, 0), (3, 0, 0), (15, 15, 0), (0, 0, 0)]
                {
                    let text = format!(
                        "    {mov} %{gp}, %xmm{count}\n    {broadcast} %xmm{count}, %{bank}{splat}\n    {shift} %{bank}{splat}, %{bank}2, %{bank}{dst}\n    ret\n"
                    );
                    let (changed, out) = uniform_shift_only(&text);
                    assert!(changed, "{text}\n{out}");
                    assert!(
                        out.contains(&format!("{expected} %xmm{splat}, %{bank}2, %{bank}{dst}")),
                        "{out}"
                    );
                    assert!(!out.contains(broadcast), "{out}");
                }
            }
        }
    }

    #[test]
    fn uniform_shift_rejects_unproven_counts_and_live_banks() {
        for prefix in [
            "vmovq %rdi, %xmm0",
            "vmovd %edi, %xmm0{%k1}",
            "vmovd (%rdi), %xmm0",
            "vmovd %xmm3, %xmm0",
            "vmovd %di, %xmm0",
        ] {
            assert!(!uniform_shift_only(&format!("    {prefix}\n    vpbroadcastd %xmm0, %ymm15\n    vpsllvd %ymm15, %ymm2, %ymm0\n    ret\n")).0);
        }
        // VPSRAVW consumes a 16-bit count per lane; VPSRAW consumes 64 bits.
        // Count 0x85012179f8de0000 therefore means zero to the former but
        // arithmetic saturation to the latter. Native execution caught this
        // exact mismatch; do not generalize the dword/qword proof to words.
        assert!(!uniform_shift_only(
            "    vmovd %edi, %xmm0\n    vpbroadcastw %xmm0, %zmm15\n    vpsravw %zmm15, %zmm2, %zmm0\n    ret\n"
        ).0);
        for middle in ["label:", "    call opaque", "    vzeroupper", "    .rept 2"] {
            assert!(!uniform_shift_only(&format!("    vmovd %edi, %xmm0\n{middle}\n    vpbroadcastd %xmm0, %ymm15\n    vpsllvd %ymm15, %ymm2, %ymm0\n    ret\n")).0);
        }
        for shift in [
            "vpsllvd %ymm15, %ymm0, %ymm0",
            "vpsllvd %ymm15, %ymm15, %ymm0",
            "vpsllvd %ymm15, %ymm2, %ymm0{%k1}",
            "vpsllvd %xmm15, %xmm2, %xmm0",
            "vpsllvd (%rdi), %ymm2, %ymm0",
            "vpsllvd %zmm15, %zmm2, %zmm0",
        ] {
            assert!(
                !uniform_shift_only(&format!(
                    "    vmovd %edi, %xmm0\n    vpbroadcastd %xmm0, %ymm15\n    {shift}\n    ret\n"
                ))
                .0
            );
        }
        for tail in [
            "vmovdqa %ymm15, (%rdi)",
            "vextractf128 $1, %ymm15, %xmm5",
            "fxsave (%rdi)",
        ] {
            assert!(!uniform_shift_only(&format!("    vmovd %edi, %xmm0\n    vpbroadcastd %xmm0, %ymm15\n    vpsllvd %ymm15, %ymm2, %ymm0\n    {tail}\n    ret\n")).0);
        }
        assert!(!uniform_shift_only("    vmovd %edi, %xmm3\n    vpbroadcastd %xmm3, %ymm15\n    vpsllvd %ymm15, %ymm2, %ymm0\n    vmovdqa %ymm3, (%rdi)\n    ret\n").0);
    }

    #[test]
    fn uniform_shift_full_pipeline_disable_control() {
        use super::super::test_skips::skip_passes;
        let asm = body(
            "    # LCCC_RET_XMM 1\n    # LCCC_RET_XMM_BITS 256\n    vmovaps %ymm0, %ymm2\n    vmovd %edi, %xmm0\n    vpbroadcastd %xmm0, %ymm15\n    vpsllvd %ymm15, %ymm2, %ymm0\n    ret\n",
        );
        let out = run(&asm);
        assert!(out.contains("vpslld"), "{out}");
        assert!(!out.contains("vpbroadcast"), "{out}");
        let disabled = {
            let _guard = skip_passes(&["vector_copy_retarget"]);
            run(&asm)
        };
        assert!(disabled.contains("vpbroadcast"), "{disabled}");
    }

    #[test]
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    fn uniform_shift_native_integer_model() {
        use crate::common::temp_files::TempFile;
        use std::process::Command;
        if !std::arch::is_x86_feature_detected!("avx2") {
            eprintln!("SKIP uniform shift native execution: AVX2 required");
            return;
        }
        let make_assembly = |avx512: bool| {
            let mut cases = vec![
                ("xmm", "d", "4"),
                ("ymm", "d", "8"),
                ("xmm", "q", "2"),
                ("ymm", "q", "4"),
            ];
            if avx512 {
                cases.extend([("zmm", "d", "16"), ("zmm", "q", "8")]);
            }
            let mut assembly = String::from(".text\n");
            for (bank, lane, suffix) in cases {
                for (operation, shift) in [("sl", "vpsllv"), ("sr", "vpsrlv"), ("sa", "vpsrav")] {
                    if lane == "q" && operation == "sa" {
                        continue;
                    }
                    let (mov, gp) = match lane {
                        "d" => (if bank == "xmm" { "movd" } else { "vmovd" }, "edi"),
                        _ => (if bank == "xmm" { "movq" } else { "vmovq" }, "rdi"),
                    };
                    let bits = match bank {
                        "zmm" => 512,
                        "ymm" => 256,
                        _ => 128,
                    };
                    let text = format!(
                        "    # LCCC_RET_XMM 1\n    # LCCC_RET_XMM_BITS {bits}\n    vmovaps %{bank}0, %{bank}2\n    {mov} %{gp}, %xmm0\n    vpbroadcast{lane} %xmm0, %{bank}15\n    {shift}{lane} %{bank}15, %{bank}2, %{bank}0\n    ret\n"
                    );
                    let original = body(&text);
                    let (changed, optimized) = uniform_shift_only(&text);
                    assert!(changed, "{optimized}");
                    for (prefix, asm) in [("original", original), ("optimized", optimized)] {
                        let name = format!("{prefix}_{lane}{suffix}{operation}");
                        assembly.push_str(&format!(".globl {name}\n"));
                        assembly.push_str(
                            &asm.replace("f:", &format!("{name}:"))
                                .replace(".size f, .-f", &format!(".size {name}, .-{name}")),
                        );
                        assembly.push('\n');
                    }
                }
            }
            assembly.push_str(".section .note.GNU-stack,\"\",@progbits\n");
            assembly
        };
        let compile_and_run = |assembly: String, avx512: bool| {
            let asm = TempFile::try_new("uniform-shift", "contract", "s").unwrap();
            let exe = TempFile::try_new("uniform-shift", "contract", "exe").unwrap();
            std::fs::write(asm.path(), assembly).unwrap();
            let mut command = Command::new("gcc");
            command.args(["-O2", "-Wall", "-Wextra", "-Werror"]);
            if avx512 {
                command.args(["-mavx512f", "-DAVX512_MODEL"]);
            } else {
                command.arg("-mavx2");
            }
            let output = command
                .arg(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/regression/fixtures/peephole-uniform-shift/driver.c"),
                )
                .arg(asm.path())
                .arg("-o")
                .arg(exe.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let output = Command::new(exe.path()).output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let expected = if avx512 { "245760" } else { "163840" };
            assert!(String::from_utf8_lossy(&output.stdout).contains(expected));
        };
        compile_and_run(make_assembly(false), false);
        if std::arch::is_x86_feature_detected!("avx512f") {
            compile_and_run(make_assembly(true), true);
        } else {
            eprintln!("SKIP AVX-512 uniform shifts: AVX512F required");
        }
    }

    fn packed_prefix_only(text: &str) -> (bool, String) {
        let mut store = LineStore::new(body(&format!("    # LCCC_RET_XMM 1\n{text}")));
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let changed = coalesce_packed_fma_prefix(&mut store, &mut infos);
        let result = (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i))
            .collect::<Vec<_>>()
            .join("\n");
        (changed, result)
    }

    #[test]
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    fn packed_fma_prefix_native_bits_rounding_and_exceptions() {
        use crate::common::temp_files::TempFile;
        use std::process::Command;
        // Structural proof tests above/below are unconditional. Native
        // execution is meaningful only on a host implementing these opcodes.
        if !std::arch::is_x86_feature_detected!("avx2")
            || !std::arch::is_x86_feature_detected!("fma")
        {
            eprintln!("SKIP packed FMA native execution: AVX2 and FMA required");
            return;
        }
        let mut assembly = String::from(".text\n");
        for (ty, bank, suffix, bits) in [
            ("f4", "xmm", "ps", 128),
            ("f8", "ymm", "ps", 256),
            ("d2", "xmm", "pd", 128),
            ("d4", "ymm", "pd", 256),
        ] {
            let text = format!(
                "    # LCCC_RET_XMM 1\n    # LCCC_RET_XMM_BITS {bits}\n    vmovaps %{bank}1, %{bank}14\n    vfmadd132{suffix} %{bank}0, %{bank}2, %{bank}14\n    vfmadd231{suffix} %{bank}3, %{bank}14, %{bank}0\n    ret\n"
            );
            let original = body(&text);
            let (changed, optimized) = packed_prefix_only(&text);
            assert!(changed, "{optimized}");
            for (prefix, asm) in [("original", original), ("optimized", optimized)] {
                let name = format!("{prefix}_{ty}");
                assembly.push_str(&format!(".globl {name}\n"));
                assembly.push_str(
                    &asm.replace("f:", &format!("{name}:"))
                        .replace(".size f, .-f", &format!(".size {name}, .-{name}")),
                );
                assembly.push('\n');
            }
        }
        assembly.push_str(".section .note.GNU-stack,\"\",@progbits\n");
        let asm = TempFile::try_new("packed-fma", "contract", "s").unwrap();
        let exe = TempFile::try_new("packed-fma", "contract", "exe").unwrap();
        std::fs::write(asm.path(), assembly).unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let output = Command::new("gcc")
            .args([
                "-O2",
                "-mavx2",
                "-mfma",
                "-frounding-math",
                "-fno-fast-math",
                "-ffp-contract=off",
                "-Wall",
                "-Wextra",
                "-Werror",
            ])
            .arg(root.join("tests/regression/fixtures/peephole-packed-fma/driver.c"))
            .arg(asm.path())
            .args(["-lm", "-o"])
            .arg(exe.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = Command::new(exe.path()).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("65536 exact"));
    }

    #[test]
    fn packed_fma_prefix_preserves_forms_and_operand_order() {
        for bank in ["xmm", "ymm"] {
            for family in ["vfmadd", "vfmsub", "vfnmadd", "vfnmsub"] {
                for form in ["132", "213", "231"] {
                    for suffix in ["ps", "pd"] {
                        let mnemonic = format!("{family}{form}{suffix}");
                        let text = format!(
                            "    vmovaps %{bank}1, %{bank}14\n    {mnemonic} %{bank}0, %{bank}2, %{bank}14\n    vfmadd231{suffix} %{bank}3, %{bank}14, %{bank}0\n    ret\n"
                        );
                        let (changed, out) = packed_prefix_only(&text);
                        assert!(changed, "{out}");
                        assert!(!out.contains("vmovaps"), "{out}");
                        assert!(
                            out.contains(&format!("{mnemonic} %{bank}0, %{bank}2, %{bank}1")),
                            "{out}"
                        );
                        assert!(
                            out.contains(&format!(
                                "vfmadd231{suffix} %{bank}3, %{bank}1, %{bank}0"
                            )),
                            "{out}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn packed_fma_prefix_full_pipeline_disable_control() {
        use super::super::test_skips::skip_passes;
        let asm = body(
            "    # LCCC_RET_XMM 1\n    # LCCC_RET_XMM_BITS 256\n    vmovaps %ymm1, %ymm14\n    vfmadd132ps %ymm0, %ymm2, %ymm14\n    vfmadd231ps %ymm3, %ymm14, %ymm0\n    ret\n",
        );
        let optimized = run(&asm);
        assert!(!optimized.contains("vmovaps"), "{optimized}");
        let disabled = {
            let _guard = skip_passes(&["vector_copy_fma"]);
            run(&asm)
        };
        assert!(disabled.contains("vmovaps"), "{disabled}");
    }

    #[test]
    fn packed_fma_prefix_refuses_live_original_source_or_temporary() {
        for tail in [
            "    vaddps %ymm1, %ymm0, %ymm0\n",
            "    vmovups %zmm1, (%rdi)\n",
            "    vmovups %ymm14, (%rdi)\n",
            "    vmovups %zmm14, (%rdi)\n",
            "    jne .Lread\n    ret\n.Lread:\n    vmovups %ymm1, (%rdi)\n",
        ] {
            let text = format!(
                "    vmovaps %ymm1, %ymm14\n    vfmadd132ps %ymm0, %ymm2, %ymm14\n    vfmadd231ps %ymm3, %ymm14, %ymm0\n{tail}    ret\n"
            );
            let (changed, out) = packed_prefix_only(&text);
            assert!(!changed, "{out}");
        }
    }

    #[test]
    fn packed_fma_prefix_refuses_masks_memory_width_mismatch_and_barriers() {
        for producer in [
            "vfmadd132ps (%rdi), %ymm2, %ymm14",
            "vfmadd132ps %ymm0, %ymm2, %ymm14{%k1}",
            "vfmadd132ps %xmm0, %xmm2, %xmm14",
            "vfmadd132ss %xmm0, %xmm2, %xmm14",
            ".Lentry:\n    vfmadd132ps %ymm0, %ymm2, %ymm14",
            "call other\n    vfmadd132ps %ymm0, %ymm2, %ymm14",
        ] {
            let text = format!(
                "    vmovaps %ymm1, %ymm14\n    {producer}\n    vfmadd231ps %ymm3, %ymm14, %ymm0\n    ret\n"
            );
            let (changed, out) = packed_prefix_only(&text);
            assert!(!changed, "{out}");
        }
        let (changed, out) = packed_prefix_only(
            "    vmovaps %ymm1, %ymm14\n    vfmadd132ps %ymm0, %ymm2, %ymm14\n    vfmadd231ps %ymm1, %ymm14, %ymm0\n    ret\n",
        );
        assert!(!changed, "{out}");
    }

    // ── the parsers ─────────────────────────────────────────────────────────

    #[test]
    fn masked_moves_do_not_establish_unconditional_copy_facts() {
        for width in ["xmm", "ymm", "zmm"] {
            for mask in ["{%k1}", "{%k1}{z}"] {
                let text = format!("vmovdqa64 %{width}1, %{width}2{mask}");
                assert_eq!(parse_vec_copy(&text), None, "{text}");
            }
        }
    }

    #[test]
    fn masked_moves_cannot_close_or_open_a_copy_bracket() {
        for mask in ["{%k1}", "{%k1}{z}"] {
            for (input_mask, output_mask) in [(mask, ""), ("", mask)] {
                let asm = body(&format!(
                    "    vmovdqa64 %xmm0, %xmm2{input_mask}\n    vpxor %xmm3, %xmm2, %xmm2\n    vmovdqa64 %xmm2, %xmm0{output_mask}\n"
                ));
                let mut store = LineStore::new(asm);
                let mut infos: Vec<_> = (0..store.len())
                    .map(|i| classify_line(store.get(i)))
                    .collect();
                assert!(!coalesce_vector_brackets(&mut store, &mut infos));
            }
            for mnemonic in ["vmovsd", "vmovss"] {
                assert_eq!(
                    parse_vec_copy(&format!("{mnemonic} %xmm1, %xmm1, %xmm2{mask}")),
                    None
                );
            }
        }
    }

    #[test]
    fn masked_move_source_must_not_replace_the_masked_result() {
        for mask in ["{%k1}", "{%k1}{z}"] {
            let asm = body(&format!(
                "    vmovdqa64 %xmm1, %xmm2{mask}\n    vmovq %xmm2, %rax\n"
            ));
            let mut store = LineStore::new(asm);
            let mut infos: Vec<_> = (0..store.len())
                .map(|i| classify_line(store.get(i)))
                .collect();
            assert!(!propagate_vector_copies(&mut store, &mut infos));
            assert!((0..store.len()).any(|i| store.get(i).trim() == "vmovq %xmm2, %rax"));
        }
    }

    #[test]
    fn copy_forms_carry_their_upper_bit_semantics() {
        // Legacy narrow: 64 bits copied, upper half preserved.
        let c = parse_vec_copy("movsd %xmm0, %xmm2").expect("movsd reg-reg");
        assert_eq!((c.src, c.dst, c.width, c.zeroes_above), (0, 2, W64, false));
        // VEX narrow: same 64 bits, upper half zeroed.
        let c = parse_vec_copy("vmovsd %xmm0, %xmm2").expect("vmovsd 2-op");
        assert_eq!((c.width, c.zeroes_above), (W64, true));
        assert_eq!(parse_vec_copy("movss %xmm1, %xmm3").unwrap().width, W32);
        // Full-width moves take the width from the spelling.
        assert_eq!(parse_vec_copy("movapd %xmm1, %xmm2").unwrap().width, W128);
        assert_eq!(parse_vec_copy("vmovapd %ymm1, %ymm2").unwrap().width, W256);
        assert_eq!(
            parse_vec_copy("vmovdqu64 %zmm1, %zmm2").unwrap().width,
            W512
        );
        // movq/movd between vector registers zero-extend in both encodings.
        let c = parse_vec_copy("movq %xmm1, %xmm2").unwrap();
        assert_eq!((c.width, c.zeroes_above), (W64, true));

        // Not copies: the three-operand merge takes its upper lane from a
        // second register, the blends and duplicating forms rewrite lanes, and
        // a memory operand is a load or a store.
        assert_eq!(parse_vec_copy("vmovsd %xmm0, %xmm1, %xmm2"), None);
        // ...but with both sources the same register every lane comes from one
        // place, which is the spelling vex_promote emits.
        let c = parse_vec_copy("vmovsd %xmm4, %xmm4, %xmm5").expect("duplicated-source merge");
        assert_eq!((c.src, c.dst, c.width, c.zeroes_above), (4, 5, W128, true));
        assert_eq!(
            parse_vec_copy("vmovss %xmm4, %xmm4, %xmm5").unwrap().width,
            W128
        );
        assert_eq!(parse_vec_copy("vmovhpd %xmm0, %xmm1, %xmm2"), None);
        assert_eq!(parse_vec_copy("vmovddup %xmm0, %xmm1"), None);
        assert_eq!(parse_vec_copy("vbroadcastsd %xmm0, %ymm1"), None);
        assert_eq!(parse_vec_copy("movsd (%rax), %xmm1"), None);
        assert_eq!(parse_vec_copy("movsd %xmm1, (%rax)"), None);
        assert_eq!(parse_vec_copy("vaddsd %xmm1, %xmm0, %xmm0"), None);
        assert_eq!(parse_vec_copy("movsd %xmm1, %xmm1"), None);
        // Mixed spellings are refused rather than reasoned about.
        assert_eq!(parse_vec_copy("movsd %xmm0, %ymm0"), None);
        // Untracked AVX-512 numbers.
        assert_eq!(parse_vec_copy("vmovapd %zmm16, %zmm17"), None);
    }

    #[test]
    fn mention_widths_follow_the_mnemonic_not_the_spelling() {
        // Scalar DATA inputs touch one lane; merge inputs and outputs touch
        // all 128 bits. The old suffix-only expectations hid a wrong-code bug.
        assert_eq!(vec_mention_width("vaddsd %xmm3, %xmm2, %xmm2", 2), W128);
        assert_eq!(vec_mention_width("vaddsd %xmm3, %xmm2, %xmm2", 3), W64);
        assert_eq!(vec_mention_width("vaddpd %xmm3, %xmm2, %xmm2", 2), W128);
        assert_eq!(vec_mention_width("vaddpd %ymm3, %ymm2, %ymm2", 2), W256);
        assert_eq!(vec_mention_width("vaddss %xmm3, %xmm2, %xmm2", 2), W128);
        assert_eq!(vec_mention_width("vaddss %xmm3, %xmm2, %xmm2", 3), W32);
        assert_eq!(vec_mention_width("vaddsd %xmm3, %xmm2, %xmm2", 4), 0);
        // Conversions are mixed-width, so they answer with the full spelling:
        // under-reporting a read is the unsound direction.
        assert_eq!(vec_mention_width("vcvtsd2ss %xmm1, %xmm0, %xmm2", 0), W128);
        // The three-operand merge reads its middle operand's upper lane.
        assert_eq!(vec_mention_width("vmovsd %xmm0, %xmm1, %xmm2", 1), W128);
        // Compare-class instructions write no vector register, so their last
        // operand is a read and not a destination.
        assert_eq!(vec_mention_width("vucomisd %xmm1, %xmm0", 0), W64);
        assert!(!writes_vec("vucomisd %xmm1, %xmm0", 0));
        assert!(writes_vec("vaddsd %xmm1, %xmm0, %xmm2", 2));
        assert!(!writes_vec("vaddsd %xmm1, %xmm0, %xmm2", 0));
        assert!(!writes_vec("vmovsd %xmm0, (%rdi)", 0));
        // An EVEX writemask does not hide the register underneath.
        assert_eq!(
            vec_mention_width("vaddpd %zmm1, %zmm1, %zmm2{%k1}", 2),
            W512
        );
    }

    #[test]
    fn rename_is_exact_on_digit_boundaries() {
        assert_eq!(
            replace_vec_reg("vaddsd %xmm2, %xmm12, %xmm2", 2, 0),
            "vaddsd %xmm0, %xmm12, %xmm0"
        );
        // %xmm15 must survive a rename of register 1, and %ymm/%zmm spellings of
        // the same register must be renamed with it.
        assert_eq!(
            replace_vec_reg("vaddpd %ymm1, %ymm15, %ymm1", 1, 3),
            "vaddpd %ymm3, %ymm15, %ymm3"
        );
        assert_eq!(
            replace_vec_reg("vfmadd231pd %zmm2, %zmm1, %zmm2", 2, 5),
            "vfmadd231pd %zmm5, %zmm1, %zmm5"
        );
        // EVEX decorations ride along, and a masked destination is renamed too:
        // the coalescer renames the register, not one occurrence of it.
        assert_eq!(
            replace_vec_reg("vaddpd %zmm2, %zmm1, %zmm2{%k1}", 2, 5),
            "vaddpd %zmm5, %zmm1, %zmm5{%k1}"
        );
        // Memory operands and GP registers are untouched.
        assert_eq!(
            replace_vec_reg("vmovupd (%rdi,%rsi,4), %xmm2", 2, 0),
            "vmovupd (%rdi,%rsi,4), %xmm0"
        );
        assert_eq!(replace_vec_reg("movq %rax, %rdx", 2, 0), "movq %rax, %rdx");
    }

    #[test]
    fn reads_are_rewritten_and_the_destination_is_not() {
        assert_eq!(
            replace_vec_reads("vaddsd %xmm2, %xmm1, %xmm2", 2, 0).unwrap(),
            "vaddsd %xmm0, %xmm1, %xmm2"
        );
        assert_eq!(
            replace_vec_reads("vroundsd $9, %xmm2, %xmm2, %xmm2", 2, 0).unwrap(),
            "vroundsd $9, %xmm0, %xmm0, %xmm2",
            "the immediate is a read position; the destination operand stays put. \
             Getting all three is the coalescer's rename, not propagation."
        );
        assert_eq!(
            replace_vec_reads("vucomisd %xmm2, %xmm1", 2, 0).unwrap(),
            "vucomisd %xmm0, %xmm1"
        );
    }

    // ── the transformations, end to end ─────────────────────────────────────

    #[test]
    fn a_scalar_rounding_bracket_collapses_to_one_instruction() {
        // What ICX emits for `__builtin_floor`, from the archived oracle dump.
        let out = run(&body(
            "    movsd %xmm0, %xmm2\n    vroundsd $9, %xmm2, %xmm2, %xmm2\n    movsd %xmm2, %xmm0\n    ret\n",
        ));
        let got = insns(&out);
        assert_eq!(
            got,
            vec!["vroundsd $9, %xmm0, %xmm0, %xmm0", "ret"],
            "the whole bracket should collapse to the oracle's single instruction"
        );
    }

    #[test]
    fn a_two_argument_fp_bracket_collapses_to_the_oracle_form() {
        // GCC/Clang/ICX all emit one instruction for `t = a; t += b; return t;`.
        let out = run(&body(
            "    movsd %xmm0, %xmm2\n    movsd %xmm1, %xmm3\n    vaddsd %xmm3, %xmm2, %xmm2\n    movsd %xmm2, %xmm0\n    ret\n",
        ));
        let got = insns(&out);
        assert_eq!(got, vec!["vaddsd %xmm1, %xmm0, %xmm0", "ret"], "{got:?}");
    }

    #[test]
    fn a_cancelling_copy_pair_is_deleted() {
        // `__builtin_copysign` ends with exactly this: a copy out and straight
        // back, with nothing between them.
        let out = run(&body(
            "    movsd %xmm0, %xmm4\n    movsd %xmm4, %xmm0\n    ret\n",
        ));
        assert_eq!(insns(&out), vec!["ret"]);
    }

    #[test]
    fn copysign_reaches_the_three_instructions_clang_emits() {
        let out = run(&body(
            "    movsd %xmm0, %xmm2\n    movsd %xmm1, %xmm3\n    vandpd .LC0(%rip), %xmm3, %xmm1\n    vandpd .LC1(%rip), %xmm2, %xmm0\n    vorpd %xmm1, %xmm0, %xmm0\n    movsd %xmm0, %xmm4\n    movsd %xmm4, %xmm0\n    ret\n",
        ));
        let got = insns(&out);
        assert_eq!(
            got.len(),
            4,
            "three bit ops plus ret, no register copies: {got:?}"
        );
        assert!(
            !got.iter().any(|l| l.starts_with("movsd")),
            "every copy should be gone: {got:?}"
        );
    }

    #[test]
    fn fma_reaches_a_single_fused_instruction() {
        let out = run(&body(
            "    movsd %xmm1, %xmm3\n    movsd %xmm2, %xmm4\n    movsd %xmm0, %xmm2\n    vmovsd %xmm4, %xmm4, %xmm5\n    vfmadd231sd %xmm3, %xmm2, %xmm5\n    movsd %xmm5, %xmm0\n    ret\n",
        ));
        let got = insns(&out);
        // Seven instructions to one: `reassociate_fma_accumulator` rotates the
        // 231 encoding into the 213, whose destination is a multiplicand and
        // whose src2 is the addend -- same three values in the same roles, one
        // rounding either way, and the instruction Clang and ICX emit.  The old
        // expectation here pinned a four-line residual and called the limit
        // instruction-selection-only; the rotation is this layer's to make.
        // Choosing the accumulator form during selection is what would ALSO
        // cover loop accumulators and the 132-form and copy-out-only variants,
        // sized with a residual inventory in
        // engineering/FOLLOWUP-2026-09-19E-vector-copy-elimination.md.
        //
        // Two of the three copies die.  The third cannot, and the reason is
        // worth stating rather than papering over: every VFMADD form reads its
        // destination as the accumulator, so retargeting the destination onto
        // %xmm0 would change WHICH value is accumulated.  ICX reaches one
        // instruction by choosing the 213 form with %xmm0 as accumulator while
        // the value is still in the selector — an instruction-selection
        // decision, not a peephole one.  Recorded as the next step in
        // engineering/FOLLOWUP-2026-09-19E-vector-copy-elimination.md.
        assert_eq!(
            got,
            vec!["vfmadd213sd %xmm2, %xmm1, %xmm0", "ret"],
            "{got:?}"
        );
    }

    // ── every legality rule, from the side that must NOT fire ───────────────

    #[test]
    fn a_source_read_inside_the_bracket_blocks_coalescing() {
        // Rule 2: the bracket reads %xmm0 itself, so renaming %xmm2 onto it
        // would change what that read sees.
        let asm = body(
            "    movsd %xmm0, %xmm2\n    vaddsd %xmm0, %xmm2, %xmm2\n    movsd %xmm2, %xmm0\n    ret\n",
        );
        let out = insns(&run(&asm));
        // Propagation rewrites the interior read (value-identical), the
        // copy-in then dies, and the result retarget puts the sum straight
        // into %xmm0: `a + a` computed in place.  What must NOT happen is the
        // rename of %xmm2 onto %xmm0 WHILE the interior still reads %xmm0's
        // old value -- that would compute `2a` from a register that already
        // held... `a`, which is the same here, but the rule is what is under
        // test, so the bracket coalescer alone is pinned separately below.
        assert_eq!(
            out,
            vec!["vaddsd %xmm0, %xmm0, %xmm0", "ret"],
            "the whole pipeline reaches the one-instruction form: {out:?}"
        );
        // The bracket coalescer on its own must refuse (rule 2).
        let mut store = LineStore::new(asm.clone());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        assert!(
            !coalesce_vector_brackets(&mut store, &mut infos),
            "rule 2: an interior read of the source blocks the rename"
        );
    }

    #[test]
    fn a_wide_interior_access_blocks_a_narrow_bracket() {
        // Rule 3: the copy defines 64 bits, the interior reads 128.  The
        // destination is live-in, which is what keeps `widen_private_vector_copies`
        // out of the way — on a private temporary widening legitimately turns
        // this into a full-width copy and the bracket becomes legal, so testing
        // the width rule needs a destination whose upper bits are already a real
        // value.
        let asm = body(
            "    vaddsd %xmm2, %xmm6, %xmm6\n    movsd %xmm0, %xmm2\n    vaddpd %xmm2, %xmm3, %xmm4\n    movsd %xmm2, %xmm0\n    ret\n",
        );
        let out = insns(&run(&asm));
        assert!(
            out.iter().any(|l| l == "movsd %xmm0, %xmm2"),
            "a 64-bit copy cannot serve a 128-bit read: {out:?}"
        );
        assert!(
            out.iter().any(|l| l == "vaddpd %xmm2, %xmm3, %xmm4"),
            "and the wide read must keep naming the destination: {out:?}"
        );
        // The same bracket with a full-width copy-in is legal.
        let out = insns(&run(&body(
            "    movapd %xmm0, %xmm2\n    vaddpd %xmm2, %xmm3, %xmm4\n    movapd %xmm2, %xmm0\n    ret\n",
        )));
        assert_eq!(
            out,
            vec!["vaddpd %xmm0, %xmm3, %xmm4", "ret"],
            "a full-width copy defines every bit the interior reads: {out:?}"
        );
    }

    #[test]
    fn a_wide_read_of_the_source_after_the_bracket_blocks_it() {
        // Rule 5: the copy-out preserved %xmm0's upper half, and something later
        // reads all 128 bits of it.
        let asm = body(
            "    movsd %xmm0, %xmm2\n    vroundsd $9, %xmm2, %xmm2, %xmm2\n    movsd %xmm2, %xmm0\n    vaddpd %xmm0, %xmm1, %xmm1\n    ret\n",
        );
        let out = insns(&run(&asm));
        assert!(
            out.iter().any(|l| l == "movsd %xmm2, %xmm0"),
            "the upper bits a later packed read needs must be preserved: {out:?}"
        );
    }

    #[test]
    fn a_temporary_live_outside_the_bracket_blocks_coalescing() {
        // Rule 4: %xmm2 is read again after the copy-out, so it is not private.
        let asm = body(
            "    movsd %xmm0, %xmm2\n    vroundsd $9, %xmm2, %xmm2, %xmm2\n    movsd %xmm2, %xmm0\n    vaddsd %xmm2, %xmm1, %xmm1\n    ret\n",
        );
        let out = insns(&run(&asm));
        assert!(
            out.iter().any(|l| l.contains("%xmm2")),
            "the temporary still has a use outside the bracket: {out:?}"
        );
        // And a temporary read BEFORE the bracket is equally disqualifying for
        // the rename. The result retarget still removes the copy-out, but
        // a narrow copy is NOT a full-width identity for the VEX merge input.
        // Preserve the copy-in until a distinct upper-lane death proof exists.
        let asm2 = body(
            "    vaddsd %xmm2, %xmm1, %xmm1\n    movsd %xmm0, %xmm2\n    vroundsd $9, %xmm2, %xmm2, %xmm2\n    movsd %xmm2, %xmm0\n    ret\n",
        );
        let out = insns(&run(&asm2));
        assert_eq!(
            out,
            vec![
                "vaddsd %xmm2, %xmm1, %xmm1",
                "movsd %xmm0, %xmm2",
                "vroundsd $9, %xmm2, %xmm2, %xmm0",
                "ret"
            ],
            "retain the unproved merge input; pre-bracket read is untouched: {out:?}"
        );
        // The bracket coalescer on its own must refuse (rule 4): renaming
        // %xmm2 onto %xmm0 would make the earlier read see the wrong register.
        let mut store = LineStore::new(asm2);
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        assert!(
            !coalesce_vector_brackets(&mut store, &mut infos),
            "rule 4: a temporary read outside the bracket blocks the rename"
        );
    }

    #[test]
    fn a_call_inside_the_bracket_blocks_it() {
        // Rule 6: every vector register is caller-saved, and the call reads
        // %xmm0-%xmm7 as arguments.
        let asm = body("    movsd %xmm0, %xmm2\n    call g\n    movsd %xmm2, %xmm0\n    ret\n");
        let out = run(&asm);
        assert!(
            out.contains("call g") && out.contains("movsd %xmm0, %xmm2"),
            "a bracket may not span a call: {out}"
        );
    }

    #[test]
    fn a_branch_inside_the_bracket_blocks_it() {
        // Rule 6: with a label between the copies, the second half can be
        // entered without the first.
        let asm = body(
            "    movsd %xmm0, %xmm2\n.L2:\n    vroundsd $9, %xmm2, %xmm2, %xmm2\n    movsd %xmm2, %xmm0\n    ret\n",
        );
        let out = run(&asm);
        assert!(
            out.contains("movsd %xmm0, %xmm2"),
            "a label inside the bracket ends it: {out}"
        );
    }

    #[test]
    fn vzeroupper_inside_the_bracket_blocks_it() {
        // Rule 6: it rewrites the upper half of every register without naming
        // any of them, so no mention scan can see what it did.
        // (The `vaddps` upper write keeps the `vzeroupper` itself alive
        // past `elide_vzeroupper_without_upper_writes`.)
        let asm = body(
            "    vaddps %ymm5, %ymm5, %ymm5\n    movapd %xmm0, %xmm2\n    vzeroupper\n    movapd %xmm2, %xmm0\n    ret\n",
        );
        let out = run(&asm);
        assert!(
            out.contains("movapd %xmm0, %xmm2"),
            "vzeroupper is a barrier: {out}"
        );
    }

    #[test]
    fn an_opaque_directive_blocks_but_cfi_does_not() {
        // Rule 7: `.rept` could emit code this pass cannot see.
        let asm = body(
            "    movsd %xmm0, %xmm2\n    .rept 2\n    .endr\n    movsd %xmm2, %xmm0\n    ret\n",
        );
        assert!(run(&asm).contains("movsd %xmm0, %xmm2"));
        // Unwind annotations name no register and emit no code.
        let out = insns(&run(&body(
            "    movsd %xmm0, %xmm2\n    .cfi_def_cfa_offset 16\n    vroundsd $9, %xmm2, %xmm2, %xmm2\n    movsd %xmm2, %xmm0\n    ret\n",
        )));
        assert_eq!(
            out,
            vec!["vroundsd $9, %xmm0, %xmm0, %xmm0", "ret"],
            "{out:?}"
        );
    }

    #[test]
    fn propagation_does_not_cross_a_width_it_cannot_prove() {
        // The copy defines 64 bits; a packed read of the destination must keep
        // naming the destination.  The destination is live-in so that widening
        // (which is legitimate on a private temporary, and is tested separately)
        // does not mask the rule under test.
        let out = run(&body(
            "    vaddsd %xmm2, %xmm6, %xmm6\n    movsd %xmm0, %xmm2\n    vaddpd %xmm2, %xmm3, %xmm4\n    ret\n",
        ));
        assert!(
            out.contains("vaddpd %xmm2, %xmm3, %xmm4"),
            "a 128-bit read must not be rewritten to a 64-bit source: {out}"
        );
        // The scalar read of the same copy is substitutable.
        let out = run(&body(
            "    movsd %xmm0, %xmm2\n    vaddsd %xmm2, %xmm3, %xmm4\n    ret\n",
        ));
        assert!(
            out.contains("vaddsd %xmm0, %xmm3, %xmm4"),
            "a 64-bit read may name the 64-bit source: {out}"
        );
    }

    #[test]
    fn a_dead_vector_copy_is_deleted_and_a_live_one_is_not() {
        // %xmm2 is never read again: the copy is a dead write.
        let out = run(&body("    movsd %xmm0, %xmm2\n    ret\n"));
        assert!(
            !out.contains("movsd %xmm0, %xmm2"),
            "an unobservable copy must go: {out}"
        );
        // %xmm2 is returned in a second FP register slot, so it is observable.
        let out = run(&body("    movsd %xmm0, %xmm2\n    ret\n"));
        assert!(!out.contains("%xmm2"));
        // A copy into a register the caller reads is not dead either.
        let out = run(&body("    movsd %xmm2, %xmm0\n    ret\n"));
        assert!(
            out.contains("movsd %xmm2, %xmm0"),
            "%xmm0 is the FP return register: {out}"
        );
    }

    #[test]
    fn the_pass_leaves_integer_and_mixed_code_alone() {
        // No vector copy, no vector rewrite: GP copies are the other passes'
        // business and must not be disturbed by this one.
        let asm = body(
            "    movq %rdi, %rax\n    addq %rsi, %rax\n    movsd %xmm0, %xmm1\n    vaddsd %xmm1, %xmm2, %xmm0\n    ret\n",
        );
        let out = run(&asm);
        // The GP sum survives, as written or as the copy+add fold's lea.
        assert!(
            out.contains("addq %rsi, %rax") || out.contains("leaq (%rdi, %rsi), %rax"),
            "{out}"
        );
    }

    #[test]
    fn a_narrow_copy_into_a_private_temporary_widens() {
        // The copysign shape: movsd defines 64 bits, vandpd reads 128.  The
        // temporary is private, so the bits the copy does not define are the
        // caller's unspecified leftovers and may become the source's.
        // Directly, so the transformation itself is pinned and not only its
        // downstream effect: after widening, propagation serves the packed read
        // from %xmm0 and the copy dies, which would hide whether widening or
        // something else produced the result.
        let asm = body("    movsd %xmm0, %xmm2\n    vandpd .LC0(%rip), %xmm2, %xmm0\n    ret\n");
        let mut store = LineStore::new(asm);
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        assert!(widen_private_vector_copies(&mut store, &mut infos));
        let widened: Vec<&str> = (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim())
            .filter(|t| !t.is_empty() && !t.starts_with('.') && !t.ends_with(':'))
            .collect();
        assert_eq!(
            widened,
            vec![
                "movapd %xmm0, %xmm2",
                "vandpd .LC0(%rip), %xmm2, %xmm0",
                "ret"
            ],
            "the narrow copy becomes the full-width move of the same flavour"
        );
        // End to end the whole chain collapses: the widened copy is then
        // propagatable into the packed read, and dies.
        let out = run(&body(
            "    movsd %xmm0, %xmm2\n    vandpd .LC0(%rip), %xmm2, %xmm0\n    ret\n",
        ));
        assert_eq!(
            insns(&out),
            vec!["vandpd .LC0(%rip), %xmm0, %xmm0", "ret"],
            "no copy should survive: {out}"
        );
        // A zeroing copy must NOT widen: zero is a defined value, and replacing
        // it with the source's upper half would be observable.
        let out = run(&body(
            "    movq %xmm0, %xmm2\n    vandpd .LC0(%rip), %xmm2, %xmm0\n    ret\n",
        ));
        assert!(
            out.contains("movq %xmm0, %xmm2"),
            "movq zeroes the upper half; widening would change it: {out}"
        );
        // Nor may a copy whose destination is live-in: its upper bits belong to
        // a real value, not to the caller's leftovers.
        let out = run(&body(
            "    vandpd .LC1(%rip), %xmm2, %xmm2\n    movsd %xmm0, %xmm2\n    vandpd .LC0(%rip), %xmm2, %xmm0\n    ret\n",
        ));
        assert!(
            out.contains("movsd %xmm0, %xmm2"),
            "a destination with an earlier use keeps its narrow copy: {out}"
        );
        // And widening must not happen when nothing reads wider than the copy.
        let out = run(&body(
            "    movsd %xmm0, %xmm2\n    vaddsd %xmm2, %xmm1, %xmm1\n    ret\n",
        ));
        assert!(
            !out.contains("movapd"),
            "no wide consumer, no widening: {out}"
        );
    }

    #[test]
    fn ymm_brackets_coalesce_too() {
        let out = insns(&run(&body(
            "    vmovapd %ymm0, %ymm2\n    vaddpd %ymm2, %ymm3, %ymm2\n    vmovapd %ymm2, %ymm0\n    vzeroupper\n    ret\n",
        )));
        assert_eq!(
            out,
            vec!["vaddpd %ymm0, %ymm3, %ymm0", "vzeroupper", "ret"],
            "{out:?}"
        );
    }

    /// Run just the FMA reassociation and report the surviving instruction
    /// lines, so every case pins the transformation itself rather than only its
    /// downstream effect after propagation and the dead sweep have had a turn.
    fn fma_only(text: &str) -> Vec<String> {
        let mut store = LineStore::new(body(text));
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        reassociate_fma_accumulator(&mut store, &mut infos);
        (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .filter(|t| !t.is_empty() && !t.starts_with('.') && !t.ends_with(':'))
            .collect()
    }

    #[test]
    fn an_fma_bracket_whose_result_is_a_multiplicand_becomes_one_213_form() {
        // `return __builtin_fma(a, b, c);` exactly as the codegen emits it: the
        // addend is copied into a fresh temporary, the 231 form accumulates into
        // it, and the result is copied back to the return register.  The
        // right-hand side is ICX's single instruction.
        let got = fma_only(
            "    vmovsd %xmm2, %xmm2, %xmm5\n    vfmadd231sd %xmm1, %xmm0, %xmm5\n    movsd %xmm5, %xmm0\n    ret\n",
        );
        assert_eq!(
            got,
            vec!["vfmadd213sd %xmm2, %xmm1, %xmm0", "ret"],
            "dst=xmm0 is the multiplicand in src1, so 213 writes it directly"
        );
    }

    #[test]
    fn an_fma_bracket_whose_result_is_the_addend_keeps_231_and_moves_the_destination() {
        // D is the register the addend came from: the accumulator role moves
        // onto it and the encoding does not change.
        let got = fma_only(
            "    movsd %xmm2, %xmm5\n    vfmadd231sd %xmm1, %xmm0, %xmm5\n    movsd %xmm5, %xmm2\n    ret\n",
        );
        assert_eq!(
            got,
            vec!["vfmadd231sd %xmm1, %xmm0, %xmm2", "ret"],
            "the addend's own register can be the destination of the same form"
        );
    }

    #[test]
    fn an_fma_bracket_whose_result_is_the_other_multiplicand_takes_213() {
        // D is src2 rather than src1, so the multiplicand that stays a source is
        // the other one: 213 reads its destination as a multiplicand and its
        // src1 as the second, with the addend in src2.
        let got = fma_only(
            "    movsd %xmm2, %xmm5\n    vfmadd231sd %xmm1, %xmm0, %xmm5\n    movsd %xmm5, %xmm1\n    ret\n",
        );
        assert_eq!(
            got,
            vec!["vfmadd213sd %xmm2, %xmm0, %xmm1", "ret"],
            "xmm1 = xmm0*xmm1 + xmm2, the same three values in the same roles"
        );
    }

    #[test]
    fn a_replacing_copy_out_rewrites_only_without_a_wide_reader() {
        // The scalar emitters copy with `movapd` (dependency-free); the same
        // `__builtin_fma` return shape must still reach ICX's one instruction.
        let ret_shape = |bits: &str| {
            format!(
                "    # LCCC_RET_XMM 1\n{bits}    movapd %xmm2, %xmm5\n    vfmadd231sd %xmm1, %xmm0, %xmm5\n    movapd %xmm5, %xmm0\n    ret\n"
            )
        };
        let code = |text: &str| -> Vec<String> {
            fma_only(text)
                .into_iter()
                .filter(|l| !l.starts_with('#'))
                .collect()
        };
        let got = code(&ret_shape("    # LCCC_RET_XMM_BITS 64\n"));
        assert_eq!(
            got,
            vec!["vfmadd213sd %xmm2, %xmm1, %xmm0", "ret"],
            "{got:?}"
        );
        // `ret` is a reader too: a `__m128d` return (`_mm_fmadd_sd`) reads
        // bits 127:64 of %xmm0, and so does a function whose return width is
        // not published (hand-written code; no marker means 512).
        for bits in ["    # LCCC_RET_XMM_BITS 128\n", ""] {
            let got = code(&ret_shape(bits));
            assert_eq!(got.len(), 4, "{bits:?} must keep the bracket: {got:?}");
        }
        // The `movapd` copy-out gave %xmm0 bits 127:64 of %xmm5 (the
        // addend's); the fused form leaves %xmm0's own. A 128-bit reader
        // would see the difference, so the bracket must stay.
        let text = "    movapd %xmm2, %xmm5\n    vfmadd231sd %xmm1, %xmm0, %xmm5\n    movapd %xmm5, %xmm0\n    vaddpd %xmm0, %xmm10, %xmm11\n    ret\n";
        let got = fma_only(text);
        assert_eq!(
            got,
            vec![
                "movapd %xmm2, %xmm5",
                "vfmadd231sd %xmm1, %xmm0, %xmm5",
                "movapd %xmm5, %xmm0",
                "vaddpd %xmm0, %xmm10, %xmm11",
                "ret",
            ],
            "{got:?}"
        );
    }

    #[test]
    fn a_wide_read_of_the_incoming_register_before_a_straight_prefix_bracket_is_dead() {
        // `chain`: the VEX scalar add reads ALL of the incoming %xmm0 (its upper
        // lane flows to %xmm2) before the load redefines %xmm0.  Straight-line
        // code before the FMA cannot run again after it, so that read cannot
        // observe the rewritten bits and the copy-out folds (GCC's 3 lines).
        let shape = |prefix_label: &str| {
            format!(
                "    # LCCC_RET_XMM 1\n    # LCCC_RET_XMM_BITS 64\n{prefix_label}    vaddsd .LC0(%rip), %xmm0, %xmm2\n    movsd .LC2(%rip), %xmm0\n    vfmsub132sd .LC1(%rip), %xmm0, %xmm2\n    vmovapd %xmm2, %xmm0\n    ret\n"
            )
        };
        let code = |text: &str| -> Vec<String> {
            fma_only(text)
                .into_iter()
                .filter(|l| !l.starts_with('#'))
                .collect()
        };
        let got = code(&shape(""));
        assert_eq!(
            got,
            vec![
                "vaddsd .LC0(%rip), %xmm0, %xmm2",
                "movsd .LC2(%rip), %xmm0",
                "vfmsub231sd .LC1(%rip), %xmm2, %xmm0",
                "ret",
            ],
            "{got:?}"
        );
        // A label in the prefix may be a loop head reached after the bracket:
        // the wide read is then live and the bracket must stay.
        let got = code(&shape(".LBB1:\n"));
        assert!(
            got.iter().any(|l| l == "vmovapd %xmm2, %xmm0"),
            "a labelled prefix keeps the copy-out: {got:?}"
        );
    }

    #[test]
    fn a_non_preserving_earlier_copy_into_the_result_register_blocks_the_rewrite() {
        // The run [save d, clobber d's upper bits, restore d] leaves the
        // result register's upper bits at ZERO in the original code: the
        // `vmovsd %xmm8, %xmm2` zeroes them and the later preserving
        // `movsd %xmm6, %xmm2` keeps that zero.  Deleting the whole run —
        // which is what the re-encode does — would restore the register's
        // pre-bracket bits instead, and the `vaddpd` below reads all 128 of
        // them.  The rewrite must be refused; checking only the LAST copy
        // into the result register (both older versions of this rule)
        // reported "preserving" and shipped the bit change.
        let got = fma_only(
            "    movsd %xmm2, %xmm6\n    vmovsd %xmm8, %xmm2\n    movsd %xmm6, %xmm2\n    vfmadd231sd %xmm1, %xmm2, %xmm3\n    movsd %xmm3, %xmm2\n    vaddpd %xmm2, %xmm10, %xmm11\n    ret\n",
        );
        assert_eq!(
            got,
            vec![
                "movsd %xmm2, %xmm6",
                "vmovsd %xmm8, %xmm2",
                "movsd %xmm6, %xmm2",
                "vfmadd231sd %xmm1, %xmm2, %xmm3",
                "movsd %xmm3, %xmm2",
                "vaddpd %xmm2, %xmm10, %xmm11",
                "ret",
            ],
            "an earlier zeroing copy into the result register is a wide-bit \
             change the last copy alone cannot speak for:\n{got:?}"
        );
        // Control: drop the zeroing copy and the same bracket rewrites —
        // the save/restore pair leaves the register's upper bits exactly as
        // they were, so the re-encode is bit-identical and MUST fire.
        let got = fma_only(
            "    movsd %xmm2, %xmm6\n    movsd %xmm6, %xmm2\n    vfmadd231sd %xmm1, %xmm2, %xmm3\n    movsd %xmm3, %xmm2\n    vaddpd %xmm2, %xmm10, %xmm11\n    ret\n",
        );
        assert_eq!(
            got,
            vec![
                "vfmadd213sd %xmm3, %xmm1, %xmm2",
                "vaddpd %xmm2, %xmm10, %xmm11",
                "ret",
            ],
            "a purely preserving run keeps the rewrite:\n{got:?}"
        );
    }

    #[test]
    fn all_four_fma_families_rotate_the_same_way() {
        // The sign structure travels with the roles, so one rotation serves all
        // four families and both scalar widths.  Pinned as a table because a pass
        // that gets one of these wrong emits a different rounding or a different
        // sign, and both are silent.
        for (from, to) in [
            ("vfmadd231sd", "vfmadd213sd"),
            ("vfmadd231ss", "vfmadd213ss"),
            ("vfmsub231sd", "vfmsub213sd"),
            ("vfmsub231ss", "vfmsub213ss"),
            ("vfnmadd231sd", "vfnmadd213sd"),
            ("vfnmadd231ss", "vfnmadd213ss"),
            ("vfnmsub231sd", "vfnmsub213sd"),
            ("vfnmsub231ss", "vfnmsub213ss"),
        ] {
            let sfx = &from[from.len() - 2..];
            let copy_in = if sfx == "sd" { "movsd" } else { "movss" };
            let text = format!(
                "    {copy_in} %xmm2, %xmm5\n    {from} %xmm1, %xmm0, %xmm5\n    {copy_in} %xmm5, %xmm0\n    ret\n"
            );
            let got = fma_only(&text);
            assert_eq!(
                got,
                vec![format!("{to} %xmm2, %xmm1, %xmm0"), "ret".to_string()],
                "{from} must rotate to {to}"
            );
        }
    }

    #[test]
    fn fma_reassociation_refuses_a_temporary_that_is_not_private() {
        // Something after the bracket still reads %xmm5, so deleting the copy-in
        // would leave it reading a register nothing defines.
        let got = fma_only(
            "    movsd %xmm2, %xmm5\n    vfmadd231sd %xmm1, %xmm0, %xmm5\n    movsd %xmm5, %xmm0\n    vaddsd %xmm5, %xmm0, %xmm0\n    ret\n",
        );
        assert_eq!(
            got.len(),
            5,
            "the bracket must stay when the temporary leaks: {got:?}"
        );
        assert!(got.iter().any(|l| l == "movsd %xmm2, %xmm5"));
    }

    #[test]
    fn fma_reassociation_refuses_a_result_register_that_is_not_an_operand() {
        // %xmm7 plays no role in the FMA, so no encoding of it writes the result
        // there.  Guessing would compute into the wrong register.
        let got = fma_only(
            "    movsd %xmm2, %xmm5\n    vfmadd231sd %xmm1, %xmm0, %xmm5\n    movsd %xmm5, %xmm7\n    ret\n",
        );
        assert_eq!(
            got.len(),
            4,
            "no single FMA form writes a fourth register: {got:?}"
        );
        assert!(got.iter().any(|l| l == "vfmadd231sd %xmm1, %xmm0, %xmm5"));
    }

    #[test]
    fn fma_reassociation_refuses_a_temporary_that_is_also_an_operand() {
        // Here the copy-in overwrites %xmm0 before the FMA reads it as a
        // multiplicand.  The rotated form would read %xmm0 AFTER that overwrite,
        // which is a different value: this is the case the distinctness check
        // exists for.
        let got = fma_only(
            "    movsd %xmm2, %xmm0\n    vfmadd231sd %xmm1, %xmm0, %xmm0\n    movsd %xmm0, %xmm0\n    ret\n",
        );
        assert!(
            got.iter().any(|l| l == "vfmadd231sd %xmm1, %xmm0, %xmm0"),
            "the 231 form must survive when the temporary is an operand: {got:?}"
        );
    }

    #[test]
    fn fma_reassociation_refuses_packed_and_masked_forms() {
        // Packed: the whole register is the value, so the copies have to match
        // the vector length and the upper-lane argument that makes the scalar
        // case sound does not apply.  Masked: a writemask destination is a
        // partial write.
        let packed = fma_only(
            "    vmovapd %ymm2, %ymm5\n    vfmadd231pd %ymm1, %ymm0, %ymm5\n    vmovapd %ymm5, %ymm0\n    ret\n",
        );
        assert!(
            packed
                .iter()
                .any(|l| l == "vfmadd231pd %ymm1, %ymm0, %ymm5"),
            "packed forms are out of scope: {packed:?}"
        );
        let masked = fma_only(
            "    movsd %xmm2, %xmm5\n    vfmadd231sd %xmm1, %xmm0, %xmm5{%k1}\n    movsd %xmm5, %xmm0\n    ret\n",
        );
        assert!(
            masked.iter().any(|l| l.contains("{%k1}")),
            "a writemask makes the destination partial: {masked:?}"
        );
    }

    #[test]
    fn fma_reassociation_refuses_a_copy_out_wider_than_the_element() {
        // A 128-bit copy-out writes %xmm0's upper lane from the temporary's,
        // which the rotated form would leave as %xmm0's own.
        let got = fma_only(
            "    movsd %xmm2, %xmm5\n    vfmadd231sd %xmm1, %xmm0, %xmm5\n    movapd %xmm5, %xmm0\n    ret\n",
        );
        assert_eq!(
            got.len(),
            4,
            "the wide copy-out changes upper-lane behaviour: {got:?}"
        );
    }

    #[test]
    fn fma_reassociation_refuses_to_see_through_a_copy_in_narrower_than_the_element() {
        // A 32-bit copy does not define the 64 bits an sd accumulator reads, so
        // %xmm2 must NOT be substituted for %xmm5: the value the FMA adds is
        // %xmm5's 64 bits, only 32 of which came from %xmm2.  What remains
        // legal is the copy-out-only rule on the FMA itself -- read the same
        // %xmm5, write the result straight into %xmm0 -- and the narrow copy
        // stays, exactly as written, feeding it.
        let got = fma_only(
            "    movss %xmm2, %xmm5\n    vfmadd231sd %xmm1, %xmm0, %xmm5\n    movsd %xmm5, %xmm0\n    ret\n",
        );
        assert_eq!(
            got,
            vec![
                "movss %xmm2, %xmm5",
                "vfmadd213sd %xmm5, %xmm1, %xmm0",
                "ret"
            ],
            "the ss copy is kept and still read; only the copy-out folds: {got:?}"
        );
    }

    #[test]
    fn fma_copy_out_only_re_homes_the_result_when_the_accumulator_is_dead() {
        // No copy-in at all: the FMA accumulates in %xmm5, which is dead once
        // its result has been copied to %xmm0.  Writing %xmm0 directly needs
        // the form whose destination is a multiplicand.
        let got =
            fma_only("    vfmadd231sd %xmm1, %xmm0, %xmm5\n    movsd %xmm5, %xmm0\n    ret\n");
        assert_eq!(got, vec!["vfmadd213sd %xmm5, %xmm1, %xmm0", "ret"]);
        // ... and refuses when the accumulator is still read afterwards.
        let got = fma_only(
            "    vfmadd231sd %xmm1, %xmm0, %xmm5\n    movsd %xmm5, %xmm0\n    vaddsd %xmm5, %xmm0, %xmm0\n    ret\n",
        );
        assert_eq!(got.len(), 4, "%xmm5 is live: {got:?}");
        // ... and when the result register is not one of the roles.
        let got =
            fma_only("    vfmadd231sd %xmm1, %xmm0, %xmm5\n    movsd %xmm5, %xmm7\n    ret\n");
        assert_eq!(got.len(), 3, "no form writes %xmm7: {got:?}");
    }

    #[test]
    fn fma_132_bracket_from_the_mul_add_shape_collapses_to_one_instruction() {
        // `a * b + c` exactly as the codegen emits it: the addend staged out of
        // the way, the multiplicand copied onto the accumulator home, a 132
        // form, and the result copied to the return register.  Propagation
        // and the dead sweep clear the first copy; this pass must clear the
        // bracket, and the 132 form is kept as GCC's spelling.
        let asm = body(
            "    movsd %xmm2, %xmm4\n    movsd %xmm0, %xmm2\n    vfmadd132sd %xmm1, %xmm4, %xmm2\n    movsd %xmm2, %xmm0\n    ret\n",
        );
        assert_eq!(
            insns(&run(&asm)),
            vec!["vfmadd132sd %xmm1, %xmm2, %xmm0", "ret"],
            "five instructions to one, GCC's exact text"
        );
    }

    #[test]
    fn fma_bracket_with_a_memory_multiplicand_keeps_it_at_operand_zero() {
        // The memory role travels with the re-encoding: the result register is
        // the register multiplicand, so the form flips to 132 and the memory
        // operand stays where the ISA can encode it.
        let got = fma_only(
            "    vmovsd %xmm2, %xmm2, %xmm5\n    vfmadd231sd (%rdi), %xmm0, %xmm5\n    movsd %xmm5, %xmm0\n    ret\n",
        );
        assert_eq!(got, vec!["vfmadd132sd (%rdi), %xmm2, %xmm0", "ret"]);
        // A second bracket where the result must be the ADDEND's register:
        // 231 with the memory operand already in place.
        let got = fma_only(
            "    vmovsd %xmm2, %xmm2, %xmm5\n    vfmadd231sd (%rdi), %xmm0, %xmm5\n    movsd %xmm5, %xmm2\n    ret\n",
        );
        assert_eq!(got, vec!["vfmadd231sd (%rdi), %xmm0, %xmm2", "ret"]);
    }

    #[test]
    fn fma_reassociation_accepts_a_copy_in_wider_than_the_element() {
        // The converse is sound and common: widening runs first, so a `movsd`
        // that a packed read promoted is a `movapd` by the time this looks at it.
        // The accumulator reads only its low lane either way.
        let got = fma_only(
            "    movapd %xmm2, %xmm5\n    vfmadd231sd %xmm1, %xmm0, %xmm5\n    movsd %xmm5, %xmm0\n    ret\n",
        );
        assert_eq!(got, vec!["vfmadd213sd %xmm2, %xmm1, %xmm0", "ret"]);
    }

    #[test]
    fn fma_reassociation_does_not_reach_across_a_barrier() {
        // A call between the FMA and the copy-out clobbers every vector
        // register, so the two copies are not one bracket.
        let got = fma_only(
            "    movsd %xmm2, %xmm5\n    vfmadd231sd %xmm1, %xmm0, %xmm5\n    callq helper\n    movsd %xmm5, %xmm0\n    ret\n",
        );
        assert!(
            got.iter().any(|l| l == "vfmadd231sd %xmm1, %xmm0, %xmm5"),
            "a call splits the bracket: {got:?}"
        );
    }

    #[test]
    fn fma_squared_norm_shape_with_a_swap_run_collapses_to_gcc_text() {
        // `x * x + c`: the codegen saves x out of %xmm0, moves the addend in,
        // and squares from the saved copy.  Both copies are one run whose
        // origins are x -> %xmm0 and c -> %xmm1; the result is wanted in %xmm0,
        // which is a multiplicand, so the 231 form cannot serve and the
        // encoder picks 213: `%xmm0 = %xmm0 * %xmm0 + %xmm1`.  GCC spells the
        // same instruction as `vfmadd132sd %xmm0, %xmm1, %xmm0`; the two are
        // the same computation and the same length.
        let got = fma_only(
            "    movsd %xmm0, %xmm2\n    movsd %xmm1, %xmm0\n    vfmadd231sd %xmm2, %xmm2, %xmm0\n    ret\n",
        );
        assert_eq!(got, vec!["vfmadd213sd %xmm1, %xmm0, %xmm0", "ret"]);
    }

    #[test]
    fn fma_run_refuses_when_any_deleted_destination_is_still_read() {
        // Same run, but the saved copy of x in %xmm2 is read after the FMA:
        // deleting the run would leave that read with a stale %xmm2.
        let got = fma_only(
            "    movsd %xmm0, %xmm2\n    movsd %xmm1, %xmm0\n    vfmadd231sd %xmm2, %xmm2, %xmm0\n    vaddsd %xmm2, %xmm0, %xmm0\n    ret\n",
        );
        assert_eq!(got.len(), 5, "%xmm2 is live after the FMA: {got:?}");
        // And through a back edge: the loop top reads %xmm2 before the run
        // redefines it, so it is live out of the FMA on the loop path.
        let got = fma_only(
            ".L1:\n    vaddsd %xmm2, %xmm3, %xmm3\n    movsd %xmm0, %xmm2\n    movsd %xmm1, %xmm0\n    vfmadd231sd %xmm2, %xmm2, %xmm0\n    jmp .L1\n",
        );
        assert!(
            got.iter().any(|l| l == "movsd %xmm0, %xmm2"),
            "the run must stay when the loop reads %xmm2: {got:?}"
        );
    }

    #[test]
    fn fma_run_follows_a_chain_of_copies_to_the_true_origin() {
        // c travels %xmm2 -> %xmm4 -> %xmm5 before the FMA accumulates into
        // %xmm5; the origin of the accumulator role is %xmm2.
        let got = fma_only(
            "    movsd %xmm2, %xmm4\n    movsd %xmm4, %xmm5\n    vfmadd231sd %xmm1, %xmm0, %xmm5\n    movsd %xmm5, %xmm0\n    ret\n",
        );
        assert_eq!(got, vec!["vfmadd213sd %xmm2, %xmm1, %xmm0", "ret"]);
    }

    #[test]
    fn fma_run_stops_at_a_copy_that_does_not_feed_the_fma() {
        // The first copy writes %xmm9, which is neither a role nor the source
        // of a copy in the run: it is not part of the bracket and must stay.
        let got = fma_only(
            "    movsd %xmm7, %xmm9\n    vmovsd %xmm2, %xmm2, %xmm5\n    vfmadd231sd %xmm1, %xmm0, %xmm5\n    movsd %xmm5, %xmm0\n    ret\n",
        );
        assert_eq!(
            got,
            vec![
                "movsd %xmm7, %xmm9",
                "vfmadd213sd %xmm2, %xmm1, %xmm0",
                "ret"
            ]
        );
    }

    #[test]
    fn fma_result_register_upper_bits_are_protected() {
        // The last copy into the result register %xmm0 is a VEX (zeroing)
        // copy, so originally %xmm0's upper lane was zero after the bracket;
        // after the rewrite it is whatever it was before.  A packed read of
        // %xmm0 later in the function would see the difference: refuse.
        let got = fma_only(
            "    vmovsd %xmm1, %xmm1, %xmm0\n    vfmadd231sd %xmm2, %xmm2, %xmm0\n    vaddpd %xmm0, %xmm3, %xmm3\n    ret\n",
        );
        assert!(
            got.iter().any(|l| l == "vmovsd %xmm1, %xmm1, %xmm0"),
            "a wide reader of the result register keeps the zeroing copy: {got:?}"
        );
        // With only scalar readers the same bracket folds.  The result stays
        // in %xmm0 (there is no copy-out), the accumulator's origin is %xmm1,
        // and %xmm0 is the addend role, so 231 with the addend read from
        // %xmm1 and the destination %xmm0 -- which is legal because the
        // destination need not be a source in the 231 form only when it IS
        // the addend; here the addend's value comes from %xmm1, so the
        // encoder must put the result where the addend lives... which is not
        // %xmm0.  No single instruction reads the addend from %xmm1 and
        // writes %xmm0 while %xmm0 plays no role, so the bracket is refused.
        // That is the correct answer: the copy is the cheapest way to move
        // the addend, and the case belongs to register allocation.
        let got = fma_only(
            "    vmovsd %xmm1, %xmm1, %xmm0\n    vfmadd231sd %xmm2, %xmm2, %xmm0\n    vaddsd %xmm0, %xmm3, %xmm3\n    ret\n",
        );
        assert_eq!(
            got.len(),
            4,
            "no form writes a register that holds no role: {got:?}"
        );
    }

    /// Run just the entry-state `vzeroupper` elision on a full function
    /// body and report the surviving lines.
    fn vzeroupper_only(text: &str) -> Vec<String> {
        let store = LineStore::new(body(text));
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        elide_vzeroupper_without_upper_writes(&store, &mut infos);
        (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .filter(|t| !t.is_empty() && !t.starts_with('.') && !t.ends_with(':'))
            .collect()
    }

    #[test]
    fn vzeroupper_is_elided_when_the_function_only_reads_incoming_ymm_state() {
        // A store of a `%ymm` argument: read only, the caller owns the state.
        assert_eq!(
            vzeroupper_only("    vmovdqu %ymm0, (%rdi)\n    vzeroupper\n    ret\n"),
            vec!["vmovdqu %ymm0, (%rdi)", "ret"]
        );
        // `vextracti128` into an XMM reads the upper half, writes none.
        assert_eq!(
            vzeroupper_only(
                "    vextracti128 $1, %ymm0, %xmm1\n    vmovd %xmm1, %eax\n    vzeroupper\n    ret\n"
            )
            .len(),
            3
        );
        // Any 256-bit write keeps it.
        assert_eq!(
            vzeroupper_only("    vaddps %ymm1, %ymm0, %ymm2\n    vmovdqu %ymm2, (%rdi)\n    vzeroupper\n    ret\n")
                .len(),
            4
        );
        // A call may dirty the state on this function's behalf: kept.
        assert_eq!(
            vzeroupper_only("    call g\n    vmovdqu %ymm0, (%rbx)\n    vzeroupper\n    ret\n")
                .len(),
            4
        );
        // `vzeroall` and masked forms are upper writes: kept.
        assert_eq!(
            vzeroupper_only("    vzeroall\n    vzeroupper\n    ret\n").len(),
            3
        );
        assert_eq!(
            vzeroupper_only("    vmovdqu32 %zmm0, (%rdi){%k1}\n    vzeroupper\n    ret\n").len(),
            3
        );
        // No `.cfi_startproc` bracket: no whole-function claim, kept.
        let store =
            LineStore::new("    vmovdqu %ymm0, (%rdi)\n    vzeroupper\n    ret\n".to_string());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        assert!(!elide_vzeroupper_without_upper_writes(&store, &mut infos));
    }

    /// Run just the result retarget and report the surviving lines.
    fn retarget_only(text: &str) -> Vec<String> {
        let mut store = LineStore::new(body(text));
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        retarget_vex_result(&mut store, &mut infos);
        (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .filter(|t| !t.is_empty() && !t.starts_with('.') && !t.ends_with(':'))
            .collect()
    }

    #[test]
    fn a_full_width_vex_load_whose_result_is_only_copied_loads_the_target_directly() {
        // The whole-register `memcpy(&v, p, 32); return v` shape.
        assert_eq!(
            retarget_only("    vmovups (%rdi), %ymm2\n    vmovdqa %ymm2, %ymm0\n    ret\n"),
            vec!["vmovups (%rdi), %ymm0", "ret"]
        );
        assert_eq!(
            retarget_only("    vmovdqu 16(%rsi), %xmm3\n    vmovdqa %xmm3, %xmm0\n    ret\n"),
            vec!["vmovdqu 16(%rsi), %xmm0", "ret"]
        );
        // A legacy copy-out merges into the upper bits where the VEX load
        // zeroes them: folded only under the wide-reader rule (as for any
        // VEX.128 producer) -- here nothing reads `%ymm0` after the copy.
        assert_eq!(
            retarget_only("    vmovups (%rdi), %xmm2\n    movdqa %xmm2, %xmm0\n    ret\n"),
            vec!["vmovups (%rdi), %xmm0", "ret"]
        );
        // Legacy load into a legacy full-width copy: both leave the bits
        // above 128 alone, so the fold is exact at every width.
        assert_eq!(
            retarget_only("    movdqu (%rdi), %xmm2\n    movdqa %xmm2, %xmm0\n    ret\n"),
            vec!["movdqu (%rdi), %xmm0", "ret"]
        );
        // ... but not into a VEX copy (which zeroes above where the
        // legacy load would not).
        assert_eq!(
            retarget_only("    movdqu (%rdi), %xmm2\n    vmovdqa %xmm2, %xmm0\n    ret\n").len(),
            3
        );
        // A broadcast's source is narrower than its destination; it folds
        // into a full-width VEX copy-out like a load.
        assert_eq!(
            retarget_only("    vpbroadcastd %xmm0, %ymm2\n    vmovdqa %ymm2, %ymm0\n    ret\n"),
            vec!["vpbroadcastd %xmm0, %ymm0", "ret"]
        );
        assert_eq!(
            retarget_only("    vbroadcastss (%rdi), %ymm3\n    vmovaps %ymm3, %ymm1\n    ret\n"),
            vec!["vbroadcastss (%rdi), %ymm1", "ret"]
        );
        // The byte variable blend is a four-operand plain output like
        // `vblendvps`: retargeted into the copy-out.
        assert_eq!(
            retarget_only(
                "    vpblendvb %ymm6, %ymm2, %ymm3, %ymm7\n    vmovdqa %ymm7, %ymm0\n    ret\n"
            ),
            vec!["vpblendvb %ymm6, %ymm2, %ymm3, %ymm0", "ret"]
        );
        // A VEX.128 load or broadcast into a legacy 128-bit copy-out: like
        // any VEX.128 producer, under the wide-reader rule (no reader of
        // the target wider than the copy).
        assert_eq!(
            retarget_only(
                "    vmovdqa .LCVEC_1(%rip), %xmm15\n    movdqa %xmm15, %xmm0\n    ret\n"
            ),
            vec!["vmovdqa .LCVEC_1(%rip), %xmm0", "ret"]
        );
        assert_eq!(
            retarget_only("    vpbroadcastw %xmm1, %xmm14\n    movdqa %xmm14, %xmm0\n    ret\n"),
            vec!["vpbroadcastw %xmm1, %xmm0", "ret"]
        );
        assert_eq!(
            retarget_only(
                "    vmovdqa .LCVEC_1(%rip), %xmm15\n    movdqa %xmm15, %xmm0\n    vpaddd %ymm0, %ymm1, %ymm1\n    vmovdqa %ymm1, %ymm0\n    ret\n"
            )
            .len(),
            5
        );
        // A register-to-register `vmovdqa` is a copy, not a load: refused.
        assert_eq!(
            retarget_only("    vmovdqa %ymm5, %ymm2\n    vmovdqa %ymm2, %ymm0\n    ret\n").len(),
            3
        );
        // Scalar / partial loads merge or do not zero above: refused.
        assert_eq!(
            retarget_only("    vmovsd (%rdi), %xmm2\n    vmovdqa %xmm2, %xmm0\n    ret\n").len(),
            3
        );
        // A mismatched copy width is refused.
        assert_eq!(
            retarget_only("    vmovups (%rdi), %ymm2\n    vmovdqa %xmm2, %xmm0\n    ret\n").len(),
            3
        );
    }

    #[test]
    fn a_vex_producer_whose_result_is_only_copied_writes_the_target_directly() {
        // `-a`: the sign flip lands in a fresh home and is copied to the
        // return register.  One instruction, GCC's text.
        assert_eq!(
            retarget_only(
                "    vxorpd .LCFP_0(%rip), %xmm0, %xmm3\n    movsd %xmm3, %xmm0\n    ret\n"
            ),
            vec!["vxorpd .LCFP_0(%rip), %xmm0, %xmm0", "ret"]
        );
        // Four-operand form with an immediate; float width; `D` a source.
        assert_eq!(
            retarget_only(
                "    vroundss $9, %xmm1, %xmm1, %xmm3\n    movss %xmm3, %xmm1\n    ret\n"
            ),
            vec!["vroundss $9, %xmm1, %xmm1, %xmm1", "ret"]
        );
        // A packed producer with a scalar copy-out: the copy took the low
        // element only, so the retarget is legal exactly when nothing reads
        // the target wider than that element -- here nothing does.
        assert_eq!(
            retarget_only(
                "    vandpd .LCFP_1(%rip), %xmm2, %xmm4\n    movsd %xmm4, %xmm0\n    ret\n"
            ),
            vec!["vandpd .LCFP_1(%rip), %xmm2, %xmm0", "ret"]
        );
    }

    #[test]
    fn the_result_retarget_refuses_every_case_it_must() {
        // `T` still read afterwards.
        let got = retarget_only(
            "    vxorpd .LCFP_0(%rip), %xmm0, %xmm3\n    movsd %xmm3, %xmm0\n    vaddsd %xmm3, %xmm0, %xmm0\n    ret\n",
        );
        assert_eq!(got.len(), 4, "%xmm3 is live: {got:?}");
        // `D` read wider than the element later: the copy-out preserved D's
        // upper lane, the retargeted producer would not.
        let got = retarget_only(
            "    vsqrtsd %xmm1, %xmm1, %xmm3\n    movsd %xmm3, %xmm0\n    vaddpd %xmm0, %xmm2, %xmm2\n    ret\n",
        );
        assert_eq!(
            got.len(),
            4,
            "a wide reader of %xmm0 keeps the copy-out: {got:?}"
        );
        // FMA producers belong to the accumulator rule, not this one.
        let got =
            retarget_only("    vfmadd231sd %xmm1, %xmm2, %xmm3\n    movsd %xmm3, %xmm0\n    ret\n");
        assert_eq!(got.len(), 3, "an FMA reads its destination: {got:?}");
        // The self-merging VEX move (`vmovsd %t, %t, %d`) copies all of T and
        // zeroes above: a whole-register copy-out, folded like any other.
        let got = retarget_only(
            "    vaddsd %xmm1, %xmm2, %xmm3\n    vmovsd %xmm3, %xmm3, %xmm0\n    ret\n",
        );
        assert_eq!(got.len(), 2, "{got:?}");
        // A VEX merge from a THIRD register is neither the legacy element
        // merge nor a whole-register copy: refused.
        let got = retarget_only(
            "    vaddsd %xmm1, %xmm2, %xmm3\n    vmovsd %xmm3, %xmm5, %xmm0\n    ret\n",
        );
        assert_eq!(got.len(), 3, "{got:?}");
        // A legacy full-width copy-out of a VEX.128 producer: bits 127:0 of
        // D are identical either way, so a leaf return folds ...
        let got =
            retarget_only("    vaddsd %xmm1, %xmm2, %xmm3\n    movapd %xmm3, %xmm0\n    ret\n");
        assert_eq!(got.len(), 2, "{got:?}");
        // ... but a reader of D's bits above 128 (left alone by the legacy
        // copy, zeroed by the retargeted producer) keeps the copy.
        let got = retarget_only(
            "    vaddsd %xmm1, %xmm2, %xmm3\n    movapd %xmm3, %xmm0\n    vaddps %ymm0, %ymm2, %ymm2\n    ret\n",
        );
        assert_eq!(got.len(), 4, "{got:?}");
        // Not adjacent: a barrier between.
        let got = retarget_only(
            "    vaddsd %xmm1, %xmm2, %xmm3\n    call f\n    movsd %xmm3, %xmm0\n    ret\n",
        );
        assert_eq!(got.len(), 4, "{got:?}");
        // A blend is still refused when `D` is read wide later: the copy-out
        // preserved D's upper lane, the retargeted blend would overwrite it.
        let got = retarget_only(
            "    vblendvpd %xmm1, %xmm2, %xmm3, %xmm4\n    movsd %xmm4, %xmm0\n    vaddpd %xmm0, %xmm2, %xmm2\n    ret\n",
        );
        assert_eq!(
            got.len(),
            4,
            "a wide reader of %xmm0 keeps the copy-out: {got:?}"
        );
        // A blend whose destination is still read afterwards.
        let got = retarget_only(
            "    vblendvpd %xmm1, %xmm2, %xmm3, %xmm4\n    movsd %xmm4, %xmm0\n    vaddsd %xmm4, %xmm0, %xmm0\n    ret\n",
        );
        assert_eq!(got.len(), 4, "%xmm4 is live: {got:?}");
        // Masked EVEX and %ymm spellings are out of scope.
        let got =
            retarget_only("    vaddsd %xmm1, %xmm2, %xmm3{%k1}\n    movsd %xmm3, %xmm0\n    ret\n");
        assert_eq!(got.len(), 3, "{got:?}");
        let got =
            retarget_only("    vaddpd %ymm1, %ymm2, %ymm3\n    movsd %xmm3, %xmm0\n    ret\n");
        assert_eq!(got.len(), 3, "{got:?}");
    }

    #[test]
    fn builtin_fma_reaches_the_single_instruction_the_oracles_emit() {
        // End to end, through the whole peephole: this is the shape the archived
        // oracle ranking scores, and ICX's answer is one instruction.
        let asm = body(
            "    vmovsd %xmm2, %xmm2, %xmm5\n    vfmadd231sd %xmm1, %xmm0, %xmm5\n    movsd %xmm5, %xmm0\n    ret\n",
        );
        assert_eq!(
            insns(&run(&asm)),
            vec!["vfmadd213sd %xmm2, %xmm1, %xmm0", "ret"],
            "three instructions to one, matching Clang and ICX"
        );
    }
}

#[cfg(test)]
mod zeroing_self_move_tests {
    use super::*;

    /// Run the pass over `body` (one function) and report whether the first
    /// `movq %xmmN, %xmmN` line survived.
    fn survives(body: &str) -> bool {
        let asm = format!(".type f, @function\nf:\n{body}.size f, .-f\n");
        let store = LineStore::new(asm);
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        eliminate_unobserved_zeroing_self_moves(&store, &mut infos);
        let at = (0..store.len())
            .find(|&i| {
                let t = store.get(i).trim();
                t.starts_with("movq %xmm") && {
                    let ops = operand_list(&t[5..]);
                    ops.len() == 2 && ops[0] == ops[1]
                }
            })
            .expect("fixture has a self-move");
        !infos[at].is_nop()
    }

    #[test]
    fn killed_by_zero_extending_gp_transfer() {
        // struct_copy: the reload became a self-move and the register is
        // redefined by `movq %rax` before anything reads its upper half.
        assert!(!survives(
            "    movsd %xmm0, 56(%rsp)\n    movq %xmm0, %xmm0\n    vmulsd %xmm4, %xmm15, %xmm15\n    movq 72(%rsp), %rax\n    movq %rax, %xmm0\n    ret\n"
        ));
    }

    #[test]
    fn carried_through_scalar_fma_and_store_then_killed() {
        assert!(!survives(
            "    movq %xmm0, %xmm0\n    movsd 32(%rsp), %xmm1\n    vfmadd231sd %xmm1, %xmm1, %xmm0\n    movsd %xmm0, 32(%rsp)\n    xorpd %xmm0, %xmm0\n    ret\n"
        ));
        assert!(!survives(
            "    movq %xmm0, %xmm0\n    vmulsd 16(%rsp), %xmm0, %xmm0\n    movq %xmm0, %xmm7\n    vsubsd %xmm14, %xmm11, %xmm0\n    ret\n"
        ));
    }

    #[test]
    fn packed_reader_observes_the_zeroed_half() {
        assert!(survives(
            "    movq %xmm0, %xmm0\n    vaddpd %xmm0, %xmm1, %xmm2\n    xorpd %xmm0, %xmm0\n    ret\n"
        ));
        assert!(survives(
            "    movq %xmm0, %xmm0\n    movapd %xmm0, %xmm3\n    xorpd %xmm0, %xmm0\n    ret\n"
        ));
        assert!(survives(
            "    movq %xmm0, %xmm0\n    unpcklpd %xmm0, %xmm1\n    xorpd %xmm0, %xmm0\n    ret\n"
        ));
    }

    #[test]
    fn vex_merge_source_donates_the_upper_half() {
        // `vaddsd %xmm1, %xmm0, %xmm2` copies xmm0[127:64] into xmm2.
        assert!(survives(
            "    movq %xmm0, %xmm0\n    vaddsd %xmm1, %xmm0, %xmm2\n    xorpd %xmm0, %xmm0\n    ret\n"
        ));
        assert!(survives(
            "    movq %xmm0, %xmm0\n    vmovsd %xmm1, %xmm0, %xmm2\n    xorpd %xmm0, %xmm0\n    ret\n"
        ));
        // As the scalar data operand it reads only the low lane.
        assert!(!survives(
            "    movq %xmm0, %xmm0\n    vaddsd %xmm0, %xmm1, %xmm2\n    xorpd %xmm0, %xmm0\n    ret\n"
        ));
    }

    #[test]
    fn barriers_and_unmodelled_text_keep_the_move() {
        for tail in [
            "    ret\n",
            "    call g\n    xorpd %xmm0, %xmm0\n    ret\n",
            ".L1:\n    xorpd %xmm0, %xmm0\n    ret\n",
            "    jmp .L2\n",
            "    vextractf128 $1, %ymm0, %xmm1\n    xorpd %xmm0, %xmm0\n    ret\n",
            "    vgatherdpd %xmm2, (%rax,%xmm0,8), %xmm1\n    xorpd %xmm0, %xmm0\n    ret\n",
            "    pshufd $78, %xmm0, %xmm1\n    xorpd %xmm0, %xmm0\n    ret\n",
        ] {
            assert!(
                survives(&format!("    movq %xmm0, %xmm0\n{tail}")),
                "must keep before {tail:?}"
            );
        }
    }

    #[test]
    fn register_numbers_match_on_digit_boundaries() {
        // `%xmm10` is not `%xmm1`: its write proves nothing about xmm1.
        assert!(survives(
            "    movq %xmm1, %xmm1\n    movq %rax, %xmm10\n    vaddpd %xmm1, %xmm2, %xmm3\n    ret\n"
        ));
        assert!(!survives(
            "    movq %xmm1, %xmm1\n    vaddpd %xmm10, %xmm2, %xmm3\n    movq %rax, %xmm1\n    ret\n"
        ));
    }

    #[test]
    fn vex_self_move_is_not_this_pass() {
        let asm = ".type f, @function\nf:\n    vmovq %xmm0, %xmm0\n    movq %rax, %xmm0\n    ret\n.size f, .-f\n";
        let store = LineStore::new(asm.to_owned());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        assert!(!eliminate_unobserved_zeroing_self_moves(&store, &mut infos));
    }
}
