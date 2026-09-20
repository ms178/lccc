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
//! Five transformations, in the order that converges fastest:
//!
//! * [`widen_private_vector_copies`] — a legacy narrow copy (`movsd`, `movss`)
//!   into a temporary nothing has defined yet becomes the full-width move of the
//!   same flavour, so the packed reads that follow it can be served from the
//!   source.  This is what makes copysign's copies reachable at all: a 64-bit
//!   copy cannot feed a 128-bit `vandpd`, and widening it is sound precisely
//!   because the bits it does not define were nobody's.
//! * [`reassociate_fma_accumulator`] — the one bracket no other rule can serve,
//!   because every FMA form reads its own destination as the accumulator.
//!   Rotating the 231 encoding into the 213 puts the result in the register the
//!   copy-out wanted, which is what `__builtin_fma` needs to reach one
//!   instruction.
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
use super::fp_liveness::FpLiveness;
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
/// writes; 0 when the line does not mention it.
fn vec_mention_width(t: &str, n: u8) -> u16 {
    let (mn, operands) = split_mn(t);
    let ops = operand_list(operands);
    if ops.is_empty() {
        return 0;
    }
    // The three-operand `vmovsd`/`vmovss` merge reads its middle operand's
    // upper lane, which the scalar-suffix rule would under-report.
    let merge_form = matches!(mn, "vmovsd" | "vmovss") && ops.len() == 3;
    let reads_only = reads_only_mnemonic(mn);
    let last = ops.len() - 1;
    ops.iter()
        .enumerate()
        .filter_map(|(k, op)| {
            let (num, spelling) = vec_operand(op)?;
            if num != n {
                return None;
            }
            let read = vec_read_width(mn, spelling, merge_form);
            let write = if k == last && !reads_only {
                vec_write_width(mn, spelling)
            } else {
                0
            };
            Some(read.max(write))
        })
        .max()
        .unwrap_or(0)
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
fn benign_directive(t: &str) -> bool {
    t.starts_with(".cfi_")
        || t.starts_with(".p2align")
        || t.starts_with(".align")
        || t.starts_with(".balign")
        || t.starts_with(".LFB")
        || t.starts_with(".loc ")
        || t.starts_with(".file ")
}

/// True when a line ends the region a copy relation or a bracket may span.
///
/// A `call` is a barrier twice over: all sixteen vector registers are
/// caller-saved under SysV, so it clobbers both sides of the relation, and it
/// reads `%xmm0`–`%xmm7` as arguments. `vzeroupper` rewrites the upper half of
/// every vector register without naming any of them, so no textual mention scan
/// can see what it does.
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
        || t.starts_with("vzeroupper")
        || t.starts_with("vzeroall")
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
        let dst_fam = VEC_FAMILY_BASE + copy_in.dst;

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
            if vec_mention_width(t, copy_in.dst) > copy_in.width {
                interior_ok = false;
                break;
            }
            j += 1;
        }
        let (Some((kout, out_copy)), true) = (copy_out, interior_ok) else {
            i += 1;
            continue;
        };

        // Rule 4: the temporary is private to the bracket. "Private" means
        // outside it — the interior is where the temporary is SUPPOSED to be
        // mentioned, so the scan covers the function's head and tail only.
        let private = (fstart..i).chain(kout + 1..fend).all(|n| {
            infos[n].is_nop()
                || !mentions_family(infos[n].trimmed(store.get(n)).as_bytes(), dst_fam)
        });
        if !private {
            i += 1;
            continue;
        }
        // Rule 5: no read of the source after the bracket wider than the
        // copy-out, whose upper-bit behaviour the rewrite changes.
        let upper_safe = (kout + 1..fend).all(|n| {
            infos[n].is_nop()
                || vec_mention_width(infos[n].trimmed(store.get(n)), copy_in.src) <= out_copy.width
        });
        if !upper_safe {
            i += 1;
            continue;
        }

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

/// Substitute a vector copy's source for reads of its destination, forward
/// through the basic block, width-checked at every step.
pub(super) fn propagate_vector_copies(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
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
            let width = vec_mention_width(&t, copy.dst);
            // A write to the source ends the relation whatever else this line
            // does; a read of it does not (substituting into a line that also
            // reads the source is value-identical).
            if width != 0 && width <= copy.width {
                if let Some(rewritten) = replace_vec_reads(&t, copy.dst, copy.src) {
                    if rewritten != t {
                        replace_line(store, &mut infos[j], j, format!("    {rewritten}"));
                        changed = true;
                    }
                }
            }
            let t_after = infos[j].trimmed(store.get(j)).to_string();
            if writes_vec(&t_after, copy.dst) || writes_vec(&t_after, copy.src) || touches_src {
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
        let dst_fam = VEC_FAMILY_BASE + copy.dst;
        // Private: nothing before the copy in this function names the
        // destination, so its upper bits are the caller's unspecified leftovers.
        let dead_in = (fstart..i).all(|n| {
            infos[n].is_nop()
                || !mentions_family(infos[n].trimmed(store.get(n)).as_bytes(), dst_fam)
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

/// The 231→213 sibling of a scalar FMA mnemonic, as an explicit table.
///
/// String surgery on mnemonics is how a pass ends up emitting `vfnadd213sd`,
/// which is not an instruction. The mapping is uniform across the four families
/// because their sign structure attaches to the same two roles in every form:
/// the product and the addend. Rotating which register plays the destination
/// rotates those roles with it, and the arithmetic does not change — for
/// `vfmadd`, 231 is `dst = src1*src2 + dst` and 213 is `dst = src1*dst + src2`,
/// so both compute (product of the two multiplicands) + addend; `vfmsub`,
/// `vfnmadd` and `vfnmsub` differ only in the two signs, which travel with the
/// same roles. Multiplication is commutative and bitwise exact in IEEE-754, and
/// an FMA rounds once either way, so the result is identical.
fn fma_213_sibling(mn: &str) -> Option<&'static str> {
    Some(match mn {
        "vfmadd231sd" => "vfmadd213sd",
        "vfmadd231ss" => "vfmadd213ss",
        "vfmsub231sd" => "vfmsub213sd",
        "vfmsub231ss" => "vfmsub213ss",
        "vfnmadd231sd" => "vfnmadd213sd",
        "vfnmadd231ss" => "vfnmadd213ss",
        "vfnmsub231sd" => "vfnmsub213sd",
        "vfnmsub231ss" => "vfnmsub213ss",
        _ => return None,
    })
}

/// How this line spells register `n`, taken from the line rather than
/// reconstructed from a width: the rewrite must not invent a spelling the
/// surrounding code does not use.
fn operand_spelling(t: &str, n: u8) -> Option<&str> {
    let (_, operands) = split_mn(t);
    operand_list(operands)
        .into_iter()
        .map(str::trim)
        .find(|op| vec_operand(op).is_some_and(|(r, _)| r == n))
}

/// The next line that is neither a nop nor blank, or `None` if a barrier or the
/// end of the function comes first. "Adjacent" in this pass means exactly that.
fn next_real(store: &LineStore, infos: &[LineInfo], from: usize, fend: usize) -> Option<usize> {
    for n in from..fend {
        if infos[n].is_nop() || infos[n].kind == LineKind::Empty {
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

/// Collapse the FMA accumulator bracket, which is what `__builtin_fma` reaches
/// the assembler as:
///
/// ```text
/// vmovsd %xmm2, %xmm2, %xmm5          vfmadd213sd %xmm2, %xmm1, %xmm0
/// vfmadd231sd %xmm1, %xmm0, %xmm5  →  ret
/// movsd %xmm5, %xmm0
/// ret
/// ```
///
/// GCC 16.2 emits one `vfmadd132sd` here, Clang 23.1 and ICX one
/// `vfmadd213sd`; the left-hand side is three instructions.
///
/// This is the one bracket [`coalesce_vector_brackets`] cannot handle, and the
/// reason is the destination: every FMA form reads its own destination as the
/// accumulator, so the destination is not a plain output that can be retargeted
/// onto the value's home register — retargeting it would change which value is
/// added. What CAN be done is to rotate the encoding so that the register which
/// must hold the result plays a role it can legally play. The bracket's three
/// values are two multiplicands and an addend, and the result register `D` is
/// always one of the instruction's three operands in this shape, so exactly one
/// rotation writes it directly:
///
/// * `D` is the addend's register — keep 231, move the destination onto it.
/// * `D` is either multiplicand — take 213, whose destination is a multiplicand
///   and whose `src2` is the addend.
///
/// # Why it is legal
///
/// The temporary is required to be named by exactly these three lines in the
/// whole function, so deleting the two copies cannot orphan a reader, and it
/// must be distinct from all four value registers: were it one of them, the
/// copy-in would already have overwritten an operand the FMA reads, and the
/// rotated form reads that operand *after* the overwrite. Both copies must be
/// register-to-register and scalar — a scalar copy-out touches only `D`'s low
/// lane, and a scalar FMA preserves its destination's upper bits, so `D`'s
/// upper bits are its own in both the original and the rewritten form. The
/// copy-in may be wider than the element (a `movapd` feeding `vfmadd231sd` is
/// fine, the accumulator's low lane is what the form reads) but not narrower:
/// an `ss` copy does not define the 64 bits an `sd` accumulator needs. Masked
/// and broadcast EVEX forms are refused outright, as are the packed suffixes —
/// there the whole register is the value and the copy widths have to match the
/// vector length, which is a different proof.
///
/// Measured over the archived oracle corpus this shape does not occur: the 52
/// FMA sites in those 51 benchmarks are loop accumulators that instruction
/// selection already placed correctly, and none carries a copy bracket. What it
/// fixes is the straight-line intrinsic — three instructions to one, which is
/// where a side-by-side comparison with GCC, Clang and ICX puts it. Choosing
/// the accumulator form during selection, while the value is still in the
/// selector, is what would cover the loop cases too; that is recorded in
/// `engineering/FOLLOWUP-2026-09-19E-vector-copy-elimination.md`.
pub(super) fn reassociate_fma_accumulator(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        let t_in = infos[i].trimmed(store.get(i)).to_string();
        let Some(copy_in) = parse_vec_copy(&t_in) else {
            i += 1;
            continue;
        };
        let Some((fstart, fend)) = function_range(store, infos, i) else {
            i += 1;
            continue;
        };
        let Some(j) = next_real(store, infos, i + 1, fend) else {
            i += 1;
            continue;
        };
        let Some(k) = next_real(store, infos, j + 1, fend) else {
            i += 1;
            continue;
        };
        if infos[j].pinned || infos[k].pinned {
            i += 1;
            continue;
        }
        let t_fma = infos[j].trimmed(store.get(j)).to_string();
        let (mn, operands) = split_mn(&t_fma);
        let Some(mn213) = fma_213_sibling(mn) else {
            i += 1;
            continue;
        };
        // A writemask makes the destination a partial write and a broadcast
        // makes one source a memory operand with a different role; neither is
        // reasoned about here.
        if t_fma.contains('{') {
            i += 1;
            continue;
        }
        // Two widths are in play and conflating them is how this guard rejects
        // every bracket it exists to fold.  The ELEMENT width is what the suffix
        // names -- 64 for `sd`, 32 for `ss` -- and it is what the copies have to
        // define.  The REGISTER width is what `vec_operand` reports: 128 for
        // every `%xmm` spelling, 256 for `%ymm`, 512 for `%zmm`, whatever the
        // instruction does with it.
        let elem_width = if mn.ends_with("sd") { W64 } else { W32 };
        let ops: Vec<&str> = operand_list(operands).iter().map(|o| o.trim()).collect();
        if ops.len() != 3 {
            i += 1;
            continue;
        }
        // AT&T reverses Intel's order, so the text reads (src2, src1, dst).
        let (Some((s2, s2w)), Some((s1, s1w)), Some((acc, accw))) = (
            vec_operand(ops[0]),
            vec_operand(ops[1]),
            vec_operand(ops[2]),
        ) else {
            i += 1;
            continue;
        };
        // One register width across all three operands: a scalar form uses %xmm
        // throughout, and a mixed spelling would mean the operands name
        // different amounts of their physical registers.
        if acc != copy_in.dst || s2w != accw || s1w != accw {
            i += 1;
            continue;
        }
        // The copy-in has to move the register the FMA actually accumulates
        // into, spelling included: `%ymm5` and `%xmm5` are the same physical
        // register but not the same amount of it, and only the spelling the FMA
        // names is the accumulator being replaced.
        let acc_spelling_ok = |text: &str, n: u8| -> bool {
            operand_spelling(text, n)
                .and_then(vec_operand)
                .is_some_and(|(_, w)| w == accw)
        };
        if !acc_spelling_ok(&t_in, copy_in.dst) {
            i += 1;
            continue;
        }
        let t_out = infos[k].trimmed(store.get(k)).to_string();
        let Some(copy_out) = parse_vec_copy(&t_out) else {
            i += 1;
            continue;
        };
        // The copy-out must be a scalar move of the element: it then touches only
        // the destination's low lane, and a scalar FMA preserves its
        // destination's upper bits, so the destination's upper bits are its own
        // in both the original and the rotated form.  A 128-bit copy-out would
        // take them from the temporary instead.
        if copy_out.src != copy_in.dst || copy_out.width != elem_width {
            i += 1;
            continue;
        }
        // The copy-in may be wider than the element -- widening runs first, so a
        // `movsd` that a packed read promoted arrives here as `movapd`, and the
        // accumulator reads only its low lane either way -- but not narrower: an
        // `ss` copy does not define the 64 bits an `sd` accumulator needs.
        if copy_in.width < elem_width {
            i += 1;
            continue;
        }
        let (c, t, d) = (copy_in.src, copy_in.dst, copy_out.dst);
        // The temporary is named by exactly these three lines, in the whole
        // function: a reader anywhere else would be left reading a register
        // nothing defines once the copies go.
        let t_fam = VEC_FAMILY_BASE + t;
        let private = (fstart..fend).all(|n| {
            n == i
                || n == j
                || n == k
                || infos[n].is_nop()
                || !mentions_family(infos[n].trimmed(store.get(n)).as_bytes(), t_fam)
        });
        // And it must not be one of the value registers: if it were, the copy-in
        // would have overwritten an operand the FMA reads, and the rotated form
        // reads that operand after the overwrite.
        if !private || t == c || t == s1 || t == s2 || t == d {
            i += 1;
            continue;
        }
        let (Some(c_sp), Some(d_sp)) = (operand_spelling(&t_in, c), operand_spelling(&t_out, d))
        else {
            i += 1;
            continue;
        };
        if !acc_spelling_ok(&t_out, d) {
            i += 1;
            continue;
        }
        let rewritten = if d == c {
            // The result lands in the addend, so the accumulator role moves onto
            // it: the same 231 encoding with a different destination.
            format!("{mn} {}, {}, {d_sp}", ops[0], ops[1])
        } else if d == s1 {
            format!("{mn213} {c_sp}, {}, {d_sp}", ops[0])
        } else if d == s2 {
            format!("{mn213} {c_sp}, {}, {d_sp}", ops[1])
        } else {
            // The result register is none of the three operands, so no single FMA
            // encoding writes it.  Inventing one is not this layer's call: the
            // form has to be chosen while the value is still in the selector,
            // during instruction selection.
            i += 1;
            continue;
        };
        replace_line(store, &mut infos[j], j, format!("    {rewritten}"));
        mark_nop(&mut infos[i]);
        mark_nop(&mut infos[k]);
        changed = true;
        i = k + 1;
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
    let fp = FpLiveness::new(store, infos);
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
        if fp.xmm_dead_after(store, infos, i, u32::from(copy.dst), &[i]) {
            mark_nop(&mut infos[i]);
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

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

    const PRO: &str = "f:\n.cfi_startproc\n";
    const EPI: &str = ".cfi_endproc\n.size f, .-f\n";

    fn body(text: &str) -> String {
        format!("{PRO}{text}{EPI}")
    }

    // ── the parsers ─────────────────────────────────────────────────────────

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
        // Scalar forms touch one lane; packed forms touch the spelling.
        assert_eq!(vec_mention_width("vaddsd %xmm3, %xmm2, %xmm2", 2), W64);
        assert_eq!(vec_mention_width("vaddpd %xmm3, %xmm2, %xmm2", 2), W128);
        assert_eq!(vec_mention_width("vaddpd %ymm3, %ymm2, %ymm2", 2), W256);
        assert_eq!(vec_mention_width("vaddss %xmm3, %xmm2, %xmm2", 2), W32);
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
        // Propagation may still rewrite the interior read (that is value-
        // identical) and the copy-in may then die as a dead write.  What must
        // NOT happen is the rename: the interior keeps writing the temporary,
        // and the copy-out is still needed to put the result in %xmm0.
        assert_eq!(
            out,
            vec!["vaddsd %xmm0, %xmm0, %xmm2", "movsd %xmm2, %xmm0", "ret"],
            "the temporary must survive as the destination: {out:?}"
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
        // the rename — though propagation may still shorten the interior, and
        // the copy-in may then die as a dead write.  The invariant is that the
        // interior still writes %xmm2 and the earlier read still sees it.
        let out = insns(&run(&body(
            "    vaddsd %xmm2, %xmm1, %xmm1\n    movsd %xmm0, %xmm2\n    vroundsd $9, %xmm2, %xmm2, %xmm2\n    movsd %xmm2, %xmm0\n    ret\n",
        )));
        assert!(
            out.iter().any(|l| l == "vaddsd %xmm2, %xmm1, %xmm1"),
            "the pre-bracket read of the temporary is untouchable: {out:?}"
        );
        assert!(
            out.iter()
                .any(|l| l.ends_with("%xmm2") && l.starts_with("vroundsd")),
            "a live-in temporary cannot be renamed onto the source: {out:?}"
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
        let asm =
            body("    movapd %xmm0, %xmm2\n    vzeroupper\n    movapd %xmm2, %xmm0\n    ret\n");
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
        assert!(out.contains("addq %rsi, %rax"), "{out}");
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
    fn fma_reassociation_refuses_a_copy_in_narrower_than_the_element() {
        // A 32-bit copy does not define the 64 bits an sd accumulator reads.
        let got = fma_only(
            "    movss %xmm2, %xmm5\n    vfmadd231sd %xmm1, %xmm0, %xmm5\n    movsd %xmm5, %xmm0\n    ret\n",
        );
        assert_eq!(
            got.len(),
            4,
            "an ss copy cannot feed an sd accumulator: {got:?}"
        );
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
