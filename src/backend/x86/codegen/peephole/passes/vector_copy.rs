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
//! * [`retarget_vex_scalar_result`] — a VEX scalar producer whose result is
//!   only copied elsewhere writes that register directly; unlike an FMA its
//!   destination is a plain output.
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

/// The previous line that is neither a nop nor blank, or `None` if a barrier
/// or the start of the function comes first.
fn prev_real(store: &LineStore, infos: &[LineInfo], before: usize, fstart: usize) -> Option<usize> {
    let mut n = before;
    while n > fstart {
        n -= 1;
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

/// Both operands of a register copy are spelled `%xmm`.
fn copy_spelled_xmm(t: &str) -> bool {
    let (_, ops) = split_mn(t);
    operand_list(ops)
        .iter()
        .all(|op| vec_operand(op.trim()).is_some_and(|(_, w)| w == W128))
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
///   only its destination's low lane.  Originally the destination's upper
///   bits came from the last thing that wrote it: a legacy element-width
///   copy preserves them, a `movapd` or a VEX copy replaces them.  Unless the
///   last copy into the result register was preserving, the rewrite may
///   change those bits, so it is allowed only when no line of the function
///   outside the rewritten bracket reads that register wider than the
///   element.  The copy-out itself must be a legacy element-width copy for
///   the same reason, seen from the other side.
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

        // The copy-out, if any: a legacy element-width move of the result,
        // adjacent, register to register in `%xmm` spelling.
        let copy_out = next_real(store, infos, j + 1, fend).and_then(|k| {
            if infos[k].pinned {
                return None;
            }
            let text = infos[k].trimmed(store.get(k));
            let c = parse_vec_copy(text)?;
            (c.src == t && c.width == elem_bits && !c.zeroes_above && copy_spelled_xmm(text))
                .then_some((k, c))
        });
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
        let all_dead = (0u8..16)
            .filter(|n| must_be_dead & (1 << n) != 0)
            .all(|n| oracle.xmm_dead_after(store, infos, last, u32::from(n), &owned));
        if !all_dead {
            j += 1;
            continue;
        }

        // The result register's upper bits: unchanged by the rewrite only if
        // the last copy that wrote it was a legacy element-width merge.
        // Otherwise no line of the function outside the bracket may read it
        // wider than the element.
        let last_write_into_d = run.iter().rev().find(|(_, c)| c.dst == d).map(|(_, c)| *c);
        let upper_preserved =
            last_write_into_d.is_none_or(|c| !c.zeroes_above && c.width == elem_bits);
        if !upper_preserved {
            let wide_reader = (fstart..fend).any(|n| {
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
/// EVEX `vfixupimm*` merges a table into it, and the moves, converts, blends
/// and inserts each have their own upper-lane story.  None of those is here.
const VEX_PLAIN_OUTPUT_BASES: &[&str] = &[
    "add", "sub", "mul", "div", "min", "max", "sqrt", "rsqrt", "rcp", "round", "xor", "and",
    "andn", "or",
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
/// * The producer's old destination `T` is dead after the copy-out, asked of
///   [`FpLiveness`]: the rewrite stops writing it.
/// * The two lines are adjacent up to nops, in one basic block, neither is
///   pinned, and every register operand is spelled `%xmm`.
pub(super) fn retarget_vex_scalar_result(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
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
        if !vex_plain_output(mn) || operands.contains('{') {
            j += 1;
            continue;
        }
        let ops: Vec<&str> = operand_list(operands).iter().map(|o| o.trim()).collect();
        // Three operands, or four with a leading immediate (`vroundsd`).
        let arity_ok = ops.len() == 3 || (ops.len() == 4 && ops[0].starts_with('$'));
        if !arity_ok {
            j += 1;
            continue;
        }
        let Some((t, W128)) = vec_operand(ops[ops.len() - 1]) else {
            j += 1;
            continue;
        };
        let sources_ok = ops[..ops.len() - 1]
            .iter()
            .all(|op| !op.starts_with('%') || vec_operand(op).is_some_and(|(_, w)| w == W128));
        if !sources_ok {
            j += 1;
            continue;
        }
        let Some((fstart, fend)) = function_range(store, infos, j) else {
            j += 1;
            continue;
        };
        let Some(k) = next_real(store, infos, j + 1, fend) else {
            j += 1;
            continue;
        };
        if infos[k].pinned {
            j += 1;
            continue;
        }
        let t_out = infos[k].trimmed(store.get(k)).to_string();
        let Some(copy_out) = parse_vec_copy(&t_out) else {
            j += 1;
            continue;
        };
        if copy_out.src != t
            || copy_out.width >= W128
            || copy_out.zeroes_above
            || !copy_spelled_xmm(&t_out)
        {
            j += 1;
            continue;
        }
        let d = copy_out.dst;
        let wide_reader = (fstart..fend).any(|n| {
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
        if !oracle.xmm_dead_after(store, infos, k, u32::from(t), &[j, k]) {
            j += 1;
            continue;
        }
        let mut new_ops: Vec<String> = ops.iter().map(|o| (*o).to_string()).collect();
        let last = new_ops.len() - 1;
        new_ops[last] = format!("%xmm{d}");
        replace_line(
            store,
            &mut infos[j],
            j,
            format!("    {mn} {}", new_ops.join(", ")),
        );
        mark_nop(&mut infos[k]);
        if let Some(oracle) = fp.as_mut() {
            oracle.refresh_at(store, infos, j);
        }
        changed = true;
        j = k + 1;
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
        // the rename.  Propagation still shortens the interior, the copy-in
        // dies, and the result retarget then computes the rounding straight
        // into %xmm0 -- legal because the earlier read of %xmm2 ran before
        // any of this and is untouched, and %xmm2 is dead afterwards.
        let asm2 = body(
            "    vaddsd %xmm2, %xmm1, %xmm1\n    movsd %xmm0, %xmm2\n    vroundsd $9, %xmm2, %xmm2, %xmm2\n    movsd %xmm2, %xmm0\n    ret\n",
        );
        let out = insns(&run(&asm2));
        assert_eq!(
            out,
            vec![
                "vaddsd %xmm2, %xmm1, %xmm1",
                "vroundsd $9, %xmm0, %xmm0, %xmm0",
                "ret"
            ],
            "the pre-bracket read is untouched and the rounding lands in %xmm0: {out:?}"
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

    /// Run just the result retarget and report the surviving lines.
    fn retarget_only(text: &str) -> Vec<String> {
        let mut store = LineStore::new(body(text));
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        retarget_vex_scalar_result(&mut store, &mut infos);
        (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .filter(|t| !t.is_empty() && !t.starts_with('.') && !t.ends_with(':'))
            .collect()
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
        // A VEX (zeroing) copy-out is not the legacy merge this rule models.
        let got = retarget_only(
            "    vaddsd %xmm1, %xmm2, %xmm3\n    vmovsd %xmm3, %xmm3, %xmm0\n    ret\n",
        );
        assert_eq!(got.len(), 3, "{got:?}");
        // A full-width copy-out reads all of T, which the retarget would
        // leave undefined in D's upper lane.
        let got =
            retarget_only("    vaddsd %xmm1, %xmm2, %xmm3\n    movapd %xmm3, %xmm0\n    ret\n");
        assert_eq!(got.len(), 3, "{got:?}");
        // Not adjacent: a barrier between.
        let got = retarget_only(
            "    vaddsd %xmm1, %xmm2, %xmm3\n    call f\n    movsd %xmm3, %xmm0\n    ret\n",
        );
        assert_eq!(got.len(), 4, "{got:?}");
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
