//! Flags-aware x86-64 peepholes.
//!
//! Three transforms that need an explicit model of the EFLAGS lifetime, which
//! the other peephole passes deliberately avoid touching:
//!
//! 1. [`fold_copy_add_into_lea`] — `movl %A,%D; addl $imm,%D` → `leal imm(%A),%D`.
//!    One instruction less, and — crucially — `lea` does not write flags, which
//!    is what makes transform 2 applicable to the loop bodies the backend emits.
//! 2. [`fold_setcc_test_cmov`] — the backend materialises a comparison as a
//!    0/1 byte, widens it, tests it, and feeds the test to a `cmov`. When no
//!    instruction between the `setCC` and the `cmov` writes flags, the `cmov`
//!    can consume the ORIGINAL comparison directly: the `setCC`, the `movzbl`
//!    and the `test` all disappear (three instructions).
//! 3. [`fold_copy_and_mask_into_movz`] — `movl %A,%D; andl $255,%D` →
//!    `movzbl %Al,%D` (likewise `$65535` → `movzwl`).
//!
//! # The flags model
//!
//! `flags_effect` classifies a line as WRITES / READS / NEUTRAL, and anything
//! unrecognised is treated as *both* a reader and a writer. Transform 1 needs
//! "the flags this `add` writes are dead" (a later writer is reached before any
//! reader); transforms 1 and 2 both bail out at control flow, because a
//! conditional branch elsewhere may consume flags that were set before a label.

use super::super::types::*;
use super::relay_and_lea::{
    dead_in_block_after, family_private_to, line_refs_family, plain_gp_operand, split_two_operands,
};

/// What a line does to EFLAGS.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FlagsEffect {
    /// Neither reads nor writes the flags (moves, `lea`, most SSE data moves).
    Neutral,
    /// Writes flags without consuming the previous value (`cmp`, `add`, ...).
    Writes,
    /// Reads the flags (`jcc`, `setcc`, `cmovcc`, `adc`, `sbb`, `pushfq`, ...).
    /// Anything unrecognised is reported as `Reads` so both queries below stay
    /// conservative: a reader blocks "flags are dead" AND blocks "flags still
    /// hold the original comparison".
    Reads,
}

/// Mnemonic prefixes that never touch EFLAGS.
const FLAG_NEUTRAL_PREFIXES: &[&str] = &[
    "movl ", "movq ", "movb ", "movw ", "movabsq ", "movabs ", "movzbl ", "movzbq ", "movzwl ",
    "movzwq ", "movsbl ", "movsbq ", "movswl ", "movswq ", "movslq ", "leal ", "leaq ", "movss ",
    "movsd ", "movaps ", "movapd ", "movups ", "movupd ", "movdqa ", "movdqu ", "movd ", "movq2dq ",
    "vmovss ", "vmovsd ", "vmovaps ", "vmovapd ", "vmovups ", "vmovupd ", "vmovdqa ", "vmovdqu ",
    "cvtsi2sd ", "cvtsi2ss ", "cvttsd2si ", "cvttss2si ", "cvtss2sd ", "cvtsd2ss ", "bswap ",
    "bswapl ", "bswapq ", "xchg ", "nop", "prefetch",
];

/// Mnemonic prefixes that read the flags.
const FLAG_READER_PREFIXES: &[&str] = &[
    "set", "cmov", "adc", "sbb", "rcl", "rcr", "pushf", "lahf", "into", "cmc", "salc",
];

/// Classify a trimmed instruction's effect on EFLAGS.
pub(super) fn flags_effect(t: &str) -> FlagsEffect {
    // Conditional jumps read; `jmp` is control flow but flag-neutral.
    if t.starts_with('j') {
        return if t.starts_with("jmp") {
            FlagsEffect::Neutral
        } else {
            FlagsEffect::Reads
        };
    }
    if FLAG_READER_PREFIXES.iter().any(|p| t.starts_with(p)) {
        return FlagsEffect::Reads;
    }
    if FLAG_NEUTRAL_PREFIXES.iter().any(|p| t.starts_with(p)) {
        return FlagsEffect::Neutral;
    }
    // Bare mnemonics (no operand) that are flag-neutral.
    if matches!(t, "ret" | "leave" | "cltq" | "cqto" | "cwtl" | "cdqe" | "nop" | "endbr64") {
        return FlagsEffect::Neutral;
    }
    if t.starts_with("push") || t.starts_with("pop") {
        // pushfq/popfq are handled by the reader list / the catch-all below.
        return if t.starts_with("popf") {
            FlagsEffect::Reads
        } else {
            FlagsEffect::Neutral
        };
    }
    // A call clobbers flags (they are not preserved across the SysV boundary),
    // which for our purposes is exactly a write.
    if t.starts_with("call") {
        return FlagsEffect::Writes;
    }
    const WRITERS: &[&str] = &[
        "add", "sub", "and", "or", "xor", "cmp", "test", "inc", "dec", "neg", "imul", "mul", "div",
        "idiv", "shl", "shr", "sar", "sal", "rol", "ror", "bt", "bsf", "bsr", "popcnt", "lzcnt",
        "tzcnt", "andn", "blsr", "blsi", "blsmsk", "ucomis", "comis", "vucomis", "vcomis",
        "cmpxchg", "xadd", "lock",
    ];
    if WRITERS.iter().any(|p| t.starts_with(p)) {
        return FlagsEffect::Writes;
    }
    // Unknown: assume the worst (both).
    FlagsEffect::Reads
}

/// True when the flags written at `from - 1` are dead: scanning forward inside
/// the block, a flags WRITER is reached before any flags READER.
fn flags_dead_after(store: &LineStore, infos: &[LineInfo], from: usize) -> bool {
    let mut n = from;
    while n < store.len() {
        if infos[n].is_nop() {
            n += 1;
            continue;
        }
        // Assembler directives (.cfi_*, .p2align, .size) emit no flag-touching
        // instruction; alignment padding is `nop`.
        if infos[n].kind == LineKind::Directive {
            n += 1;
            continue;
        }
        // Returning ends the flags' lifetime: the SysV ABI does not preserve
        // EFLAGS across a call boundary.
        if infos[n].kind == LineKind::Ret {
            return true;
        }
        let t = infos[n].trimmed(store.get(n));
        match flags_effect(t) {
            FlagsEffect::Reads => return false,
            FlagsEffect::Writes => return true,
            FlagsEffect::Neutral => {}
        }
        // push/pop are barriers for the register scans but leave flags alone.
        if matches!(infos[n].kind, LineKind::Push { .. } | LineKind::Pop { .. }) {
            n += 1;
            continue;
        }
        // Any other control flow: a successor block may read the flags.
        if infos[n].is_barrier() {
            return false;
        }
        n += 1;
    }
    false
}

/// Index of the next non-NOP line after `i`, or `None`.
fn next_real(infos: &[LineInfo], i: usize, len: usize) -> Option<usize> {
    let mut k = i + 1;
    while k < len {
        if !infos[k].is_nop() {
            return Some(k);
        }
        k += 1;
    }
    None
}

/// Parse `$<int>` (decimal, optionally negative) into its value.
fn imm_value(text: &str) -> Option<i64> {
    text.strip_prefix('$')?.parse::<i64>().ok()
}


/// The arithmetic operand must name the copy's destination register at the
/// arithmetic's own width (`movq %r9, %rax` + `addl $1, %eax`).
fn names_family_at_width(text: &str, fam: RegId, wide: bool) -> bool {
    text == REG_NAMES[if wide { 0 } else { 1 }][fam as usize]
}

// ── 1. copy + add-immediate → lea ────────────────────────────────────────────

/// `movl %A, %D` + `addl $imm, %D` → `leal imm(%A), %D` (same for `q`, and for
/// `sub` with the displacement negated).
///
/// Saves one instruction and — the reason this pass exists — removes a flags
/// write from between a comparison and its consumer, which is what lets
/// [`fold_setcc_test_cmov`] fire on the counter-increment idiom.
///
/// Soundness: `lea` computes the same value (`imm` fits in the 32-bit signed
/// displacement), `A != D` so no operand is destroyed, and the flags the `add`
/// used to write must be provably dead.
pub(super) fn fold_copy_add_into_lea(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        let mov = infos[i].trimmed(store.get(i));
        let (wide, rest) = if let Some(r) = mov.strip_prefix("movq ") {
            (true, r)
        } else if let Some(r) = mov.strip_prefix("movl ") {
            (false, r)
        } else {
            i += 1;
            continue;
        };
        let Some((src_text, dst_text)) = split_two_operands(rest) else {
            i += 1;
            continue;
        };
        let (Some(src_fam), Some(dst_fam)) =
            (plain_gp_operand(src_text), plain_gp_operand(dst_text))
        else {
            i += 1;
            continue;
        };
        // %rsp/%rbp as the LEA destination is never worth the risk, and a
        // self-copy is not this pattern.
        if src_fam == dst_fam || dst_fam == 4 || dst_fam == 5 {
            i += 1;
            continue;
        }
        let Some(j) = next_real(infos, i, len) else {
            i += 1;
            continue;
        };
        if infos[j].pinned {
            i += 1;
            continue;
        }
        let add = infos[j].trimmed(store.get(j));
        let (add_wide, neg, arest) = if let Some(r) = add.strip_prefix("addq ") {
            (true, false, r)
        } else if let Some(r) = add.strip_prefix("addl ") {
            (false, false, r)
        } else if let Some(r) = add.strip_prefix("subq ") {
            (true, true, r)
        } else if let Some(r) = add.strip_prefix("subl ") {
            (false, true, r)
        } else {
            i += 1;
            continue;
        };
        // The copy may be WIDER than the arithmetic (`movq %r9, %rax` +
        // `addl $1, %eax` = zero-extended 32-bit add), never narrower: a
        // `movl` copy zero-extends, so a following `addq` would read bits the
        // 64-bit LEA source still has.
        if !wide && add_wide {
            i += 1;
            continue;
        }
        let Some((imm_text, add_dst)) = split_two_operands(arest) else {
            i += 1;
            continue;
        };
        if !names_family_at_width(add_dst, dst_fam, add_wide) {
            i += 1;
            continue;
        }
        let Some(mut imm) = imm_value(imm_text) else {
            i += 1;
            continue;
        };
        if neg {
            imm = -imm;
        }
        if imm < i32::MIN as i64 || imm > i32::MAX as i64 {
            i += 1;
            continue;
        }
        if !flags_dead_after(store, infos, j + 1) {
            i += 1;
            continue;
        }
        // The LEA is emitted at the ARITHMETIC width; its base register is
        // always named 64-bit so the address computation stays in 64-bit mode
        // (`leal 1(%r9), %eax` — 32-bit result, 64-bit base).
        let mnemonic = if add_wide { "leaq" } else { "leal" };
        let base = REG_NAMES[0][src_fam as usize];
        let dst_name = if add_wide {
            REG_NAMES[0][dst_fam as usize]
        } else {
            REG_NAMES[1][dst_fam as usize]
        };
        let new_line = format!("    {} {}({}), {}", mnemonic, imm, base, dst_name);
        mark_nop(&mut infos[i]);
        replace_line(store, &mut infos[j], j, new_line);
        changed = true;
        i = j + 1;
    }
    changed
}


// ── 4. copy + left shift → scaled lea ────────────────────────────────────────

/// `movq %A, %D; shlq $N, %D` → `leaq 0(,%A,2^N), %D` for N in 1..=3 (and the
/// 32-bit forms). One instruction less and no flags write.
///
/// `fuse_copy_and_operation` in `local_patterns.rs` documents this rewrite in
/// its header comment but only ever implemented copy+lea and copy+add; the
/// insertion-sort address computation (`movq %rax, %r12; shlq $2, %r12`) is
/// the shape it misses.
///
/// Width rule as in [`fold_copy_add_into_lea`]: the copy may be wider than the
/// shift (the shift then truncates and the LEA zero-extends identically), but
/// never narrower.
pub(super) fn fold_copy_shift_into_lea(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        let mov = infos[i].trimmed(store.get(i));
        let (wide, rest) = if let Some(r) = mov.strip_prefix("movq ") {
            (true, r)
        } else if let Some(r) = mov.strip_prefix("movl ") {
            (false, r)
        } else {
            i += 1;
            continue;
        };
        let Some((src_text, dst_text)) = split_two_operands(rest) else {
            i += 1;
            continue;
        };
        let (Some(src_fam), Some(dst_fam)) =
            (plain_gp_operand(src_text), plain_gp_operand(dst_text))
        else {
            i += 1;
            continue;
        };
        if src_fam == dst_fam || dst_fam == 4 || dst_fam == 5 {
            i += 1;
            continue;
        }
        let Some(j) = next_real(infos, i, len) else {
            i += 1;
            continue;
        };
        if infos[j].pinned {
            i += 1;
            continue;
        }
        let shift = infos[j].trimmed(store.get(j));
        let (shift_wide, srest) = if let Some(r) = shift.strip_prefix("shlq ") {
            (true, r)
        } else if let Some(r) = shift.strip_prefix("shll ") {
            (false, r)
        } else if let Some(r) = shift.strip_prefix("salq ") {
            (true, r)
        } else if let Some(r) = shift.strip_prefix("sall ") {
            (false, r)
        } else {
            i += 1;
            continue;
        };
        if !wide && shift_wide {
            i += 1;
            continue;
        }
        let Some((imm_text, shift_dst)) = split_two_operands(srest) else {
            i += 1;
            continue;
        };
        if !names_family_at_width(shift_dst, dst_fam, shift_wide) {
            i += 1;
            continue;
        }
        let scale = match imm_value(imm_text) {
            Some(1) => 2,
            Some(2) => 4,
            Some(3) => 8,
            _ => {
                i += 1;
                continue;
            }
        };
        if !flags_dead_after(store, infos, j + 1) {
            i += 1;
            continue;
        }
        let mnemonic = if shift_wide { "leaq" } else { "leal" };
        let index = REG_NAMES[0][src_fam as usize];
        let dst_name = if shift_wide {
            REG_NAMES[0][dst_fam as usize]
        } else {
            REG_NAMES[1][dst_fam as usize]
        };
        let new_line = format!("    {} 0(,{},{}), {}", mnemonic, index, scale, dst_name);
        mark_nop(&mut infos[i]);
        replace_line(store, &mut infos[j], j, new_line);
        changed = true;
        i = j + 1;
    }
    changed
}

// ── 2. setcc / movzbl / test / cmov → cmov on the original comparison ────────

/// Negate a condition-code suffix (`e` ↔ `ne`, `l` ↔ `ge`, ...).
fn negate_cc(cc: &str) -> Option<&'static str> {
    Some(match cc {
        "e" | "z" => "ne",
        "ne" | "nz" => "e",
        "l" | "nge" => "ge",
        "ge" | "nl" => "l",
        "le" | "ng" => "g",
        "g" | "nle" => "le",
        "b" | "c" | "nae" => "ae",
        "ae" | "nb" | "nc" => "b",
        "be" | "na" => "a",
        "a" | "nbe" => "be",
        "s" => "ns",
        "ns" => "s",
        "p" | "pe" => "np",
        "np" | "po" => "p",
        "o" => "no",
        "no" => "o",
        _ => return None,
    })
}

/// Canonical condition-code names accepted on `setCC`/`cmovCC`.
fn canonical_cc(cc: &str) -> Option<&'static str> {
    Some(match cc {
        "e" | "z" => "e",
        "ne" | "nz" => "ne",
        "l" | "nge" => "l",
        "ge" | "nl" => "ge",
        "le" | "ng" => "le",
        "g" | "nle" => "g",
        "b" | "c" | "nae" => "b",
        "ae" | "nb" | "nc" => "ae",
        "be" | "na" => "be",
        "a" | "nbe" => "a",
        "s" => "s",
        "ns" => "ns",
        "p" | "pe" => "p",
        "np" | "po" => "np",
        "o" => "o",
        "no" => "no",
        _ => return None,
    })
}

/// Split `cmovneq %r8, %rbx` into (`"ne"`, `"q"`, `"%r8"`, `"%rbx"`).
fn parse_cmov(t: &str) -> Option<(&'static str, char, &str, &str)> {
    let rest = t.strip_prefix("cmov")?;
    let sp = rest.find(' ')?;
    let (tag, ops) = rest.split_at(sp);
    let (cc_part, width) = match tag.chars().last()? {
        w @ ('q' | 'l' | 'w') if tag.len() >= 2 => (&tag[..tag.len() - 1], w),
        _ => return None,
    };
    let cc = canonical_cc(cc_part)?;
    let (src, dst) = split_two_operands(ops)?;
    Some((cc, width, src, dst))
}

/// The backend lowers `x = cond ? a : b` and `count += (p[i] == 0)` as
///
/// ```text
///     cmpb  $0, %r11b        <- the real comparison
///     sete  %r10b
///     movzbl %r10b, %r10d
///     leal  1(%rbx), %r8d    <- flag-neutral after fold_copy_add_into_lea
///     testq %r10, %r10
///     cmovneq %r8, %rbx
/// ```
///
/// The boolean round-trip is pure overhead when nothing between the `setCC` and
/// the `cmov` writes flags: the `cmov` can test the original comparison.
/// Three instructions disappear (`setCC`, `movzbl`, `test`).
///
/// Conditions checked here:
/// * the `setCC` destination byte register, the widened register and the `test`
///   operands are all the same family, and that family is provably dead after
///   the `cmov` (so deleting its definition is invisible);
/// * NOTHING between the `setCC` and the `cmov` writes or reads the flags
///   (readers would observe the comparison we are about to consume, writers
///   would destroy it), and there is no control-flow boundary;
/// * the `test` is a self-test of the boolean, and the `cmov` condition is
///   `ne` (boolean true) or `e` (boolean false), which maps to the original
///   condition or its negation.
pub(super) fn fold_setcc_test_cmov(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        let t = infos[i].trimmed(store.get(i));
        // `setCC %reg8`
        let Some(rest) = t.strip_prefix("set") else {
            i += 1;
            continue;
        };
        let Some(sp) = rest.find(' ') else {
            i += 1;
            continue;
        };
        let (cc_txt, breg) = rest.split_at(sp);
        let (Some(cc), breg) = (canonical_cc(cc_txt), breg.trim()) else {
            i += 1;
            continue;
        };
        let bool_fam = register_family_fast(breg);
        if bool_fam == REG_NONE || bool_fam > REG_GP_MAX {
            i += 1;
            continue;
        }
        let bool_mask = 1u16 << bool_fam;

        // Walk forward: optional movzbl of the boolean, then flag-neutral
        // instructions, then `test %C,%C`, then `cmovCC`.
        let mut owned = vec![i];
        let mut k = i;
        let mut test_idx = None;
        let mut cmov_idx = None;
        loop {
            let Some(n) = next_real(infos, k, len) else { break };
            k = n;
            if infos[n].pinned || infos[n].is_barrier() {
                break;
            }
            let tn = infos[n].trimmed(store.get(n));
            // The widening move of the boolean is part of the idiom.
            if test_idx.is_none()
                && (tn.starts_with("movzbl ") || tn.starts_with("movzbq "))
                && split_two_operands(&tn[7..]).is_some_and(|(s, d)| {
                    register_family_fast(s) == bool_fam && register_family_fast(d) == bool_fam
                })
            {
                owned.push(n);
                continue;
            }
            if test_idx.is_none() {
                if let Some(args) = tn
                    .strip_prefix("testq ")
                    .or_else(|| tn.strip_prefix("testl "))
                    .or_else(|| tn.strip_prefix("testb "))
                {
                    if let Some((a, b)) = split_two_operands(args) {
                        if a == b && register_family_fast(a) == bool_fam {
                            owned.push(n);
                            test_idx = Some(n);
                            continue;
                        }
                    }
                }
            }
            if test_idx.is_some() {
                if parse_cmov(tn).is_some() {
                    cmov_idx = Some(n);
                }
                break;
            }
            // Any other line: it must neither touch the boolean nor the flags.
            if infos[n].reg_refs & bool_mask != 0 || flags_effect(tn) != FlagsEffect::Neutral {
                break;
            }
        }

        let (Some(test_i), Some(cmov_i)) = (test_idx, cmov_idx) else {
            i += 1;
            continue;
        };
        let cmov_line = infos[cmov_i].trimmed(store.get(cmov_i)).to_string();
        let Some((cmov_cc, width, src, dst)) = parse_cmov(&cmov_line) else {
            i += 1;
            continue;
        };
        // `test C,C` + `cmovne` = "boolean is true" = original condition;
        // `cmove` = its negation. Any other condition is not a boolean test.
        let new_cc = match cmov_cc {
            "ne" => cc,
            "e" => match negate_cc(cc) {
                Some(n) => n,
                None => {
                    i += 1;
                    continue;
                }
            },
            _ => {
                i += 1;
                continue;
            }
        };
        // The cmov must not read the boolean itself (it would lose its definition).
        if infos[cmov_i].reg_refs & bool_mask != 0 {
            i += 1;
            continue;
        }
        // The deleted `test` also DEFINED the flags every later reader saw.
        // After the rewrite those readers would observe the original
        // comparison instead — with `sete`/`setne` in the idiom that is the
        // inverted condition, so a following `jne` would branch the wrong way.
        // Require the flags to be dead after the cmov.
        if !flags_dead_after(store, infos, cmov_i + 1) {
            i += 1;
            continue;
        }
        // The boolean register must be dead after the cmov: either the block
        // rewrites it before any read, or the whole function only mentions it
        // in the lines we are about to delete.
        owned.push(cmov_i);
        let dead = dead_in_block_after(store, infos, cmov_i + 1, bool_fam)
            || family_private_to(store, infos, i, bool_fam, &owned);
        if !dead {
            i += 1;
            continue;
        }
        let new_line = format!("    cmov{}{} {}, {}", new_cc, width, src, dst);
        if line_refs_family(&new_line, bool_fam) {
            i += 1;
            continue;
        }
        for &d in &owned {
            if d != cmov_i {
                mark_nop(&mut infos[d]);
            }
        }
        replace_line(store, &mut infos[cmov_i], cmov_i, new_line);
        changed = true;
        i = test_i.max(cmov_i) + 1;
    }
    changed
}

// ── 3. copy + mask → zero-extending move ─────────────────────────────────────

/// `movl %A,%D; andl $255,%D` → `movzbl %Al,%D` (and `$65535` → `movzwl`).
///
/// One instruction less and no flags write. Valid for the `q` forms as well:
/// `movzbl` zeroes bits 8..63 of the destination, exactly like `andq $255`
/// applied to a 64-bit copy.
pub(super) fn fold_copy_and_mask_into_movz(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        let mov = infos[i].trimmed(store.get(i));
        let (wide, rest) = if let Some(r) = mov.strip_prefix("movq ") {
            (true, r)
        } else if let Some(r) = mov.strip_prefix("movl ") {
            (false, r)
        } else {
            i += 1;
            continue;
        };
        let Some((src_text, dst_text)) = split_two_operands(rest) else {
            i += 1;
            continue;
        };
        let (Some(src_fam), Some(dst_fam)) =
            (plain_gp_operand(src_text), plain_gp_operand(dst_text))
        else {
            i += 1;
            continue;
        };
        if src_fam == dst_fam || dst_fam == 4 || dst_fam == 5 {
            i += 1;
            continue;
        }
        let Some(j) = next_real(infos, i, len) else {
            i += 1;
            continue;
        };
        if infos[j].pinned {
            i += 1;
            continue;
        }
        let and = infos[j].trimmed(store.get(j));
        let (and_wide, arest) = if let Some(r) = and.strip_prefix("andq ") {
            (true, r)
        } else if let Some(r) = and.strip_prefix("andl ") {
            (false, r)
        } else {
            i += 1;
            continue;
        };
        if !wide && and_wide {
            i += 1;
            continue;
        }
        let Some((imm_text, and_dst)) = split_two_operands(arest) else {
            i += 1;
            continue;
        };
        if !names_family_at_width(and_dst, dst_fam, and_wide) {
            i += 1;
            continue;
        }
        // 8-bit source names for %rsp/%rbp/%rsi/%rdi need a REX prefix; the
        // table already spells them (%spl/%bpl/%sil/%dil) and the assembler
        // emits REX, but %ah-class aliasing makes the byte form of families
        // 4/5 pointless here — they are rejected above for the destination and
        // are legal as a source.
        let (mnemonic, sub_name) = match imm_value(imm_text) {
            Some(255) => ("movzbl", REG_NAMES[3][src_fam as usize]),
            Some(65535) => ("movzwl", REG_NAMES[2][src_fam as usize]),
            _ => {
                i += 1;
                continue;
            }
        };
        if !flags_dead_after(store, infos, j + 1) {
            i += 1;
            continue;
        }
        // The destination of movzbl/movzwl is always the 32-bit name.
        let dst32 = REG_NAMES[1][dst_fam as usize];
        let new_line = format!("    {} {}, {}", mnemonic, sub_name, dst32);
        mark_nop(&mut infos[i]);
        replace_line(store, &mut infos[j], j, new_line);
        changed = true;
        i = j + 1;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::super::super::peephole_optimize;

    fn run(asm: &str) -> String {
        peephole_optimize(asm.to_string())
    }

    #[test]
    fn copy_plus_add_becomes_lea() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %ebx, %r8d\n",
            "    addl $1, %r8d\n",
            "    testq %r10, %r10\n",
            "    cmovneq %r8, %rbx\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("leal 1(%rbx), %r8d"), "{out}");
        assert!(!out.contains("addl $1, %r8d"), "{out}");
    }

    #[test]
    fn copy_plus_add_is_kept_when_flags_are_read() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %ebx, %r8d\n",
            "    addl $1, %r8d\n",
            "    je .LBB9\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("addl $1, %r8d"), "{out}");
    }


    #[test]
    fn wide_copy_with_narrow_add_becomes_a_32bit_lea() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %r9, %rax\n",
            "    addl $1, %eax\n",
            "    movq %rax, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("leal 1(%r9), %eax"), "{out}");
    }

    #[test]
    fn narrow_copy_with_wide_add_is_rejected() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %r9d, %eax\n",
            "    addq $1, %rax\n",
            "    movq %rax, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("addq $1, %rax"), "{out}");
    }

    #[test]
    fn copy_plus_shift_becomes_scaled_lea() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %rax, %r12\n",
            "    shlq $2, %r12\n",
            "    movq %r12, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("leaq 0(,%rax,4), %r12"), "{out}");
        assert!(!out.contains("shlq $2"), "{out}");
    }

    #[test]
    fn copy_plus_shift_is_kept_when_flags_are_read() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %rax, %r12\n",
            "    shlq $2, %r12\n",
            "    jne .LBB4\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("shlq $2, %r12"), "{out}");
    }

    #[test]
    fn boolean_roundtrip_collapses_into_the_cmov() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            ".LBB2:\n",
            "    movzbl (%rsi,%r12), %r11d\n",
            "    cmpb $0, %r11b\n",
            "    sete %r10b\n",
            "    movzbl %r10b, %r10d\n",
            "    movl %ebx, %r8d\n",
            "    addl $1, %r8d\n",
            "    testq %r10, %r10\n",
            "    cmovneq %r8, %rbx\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("cmoveq %r8, %rbx"), "{out}");
        assert!(!out.contains("sete"), "{out}");
        assert!(!out.contains("testq %r10"), "{out}");
        assert!(out.contains("leal 1(%rbx), %r8d"), "{out}");
    }


    #[test]
    fn boolean_roundtrip_is_kept_when_a_later_branch_reads_the_test() {
        // The `jne` consumes the flags the `test` produced ("boolean is set").
        // Collapsing the idiom would leave it reading the `cmpl` instead,
        // which is the inverted condition.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    cmpl %edx, %ecx\n",
            "    sete %r10b\n",
            "    movzbl %r10b, %r10d\n",
            "    testq %r10, %r10\n",
            "    cmovneq %r8, %rbx\n",
            "    jne .LBB9\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("testq %r10, %r10"), "{out}");
        assert!(out.contains("sete %r10b"), "{out}");
    }

    #[test]
    fn boolean_roundtrip_is_kept_when_a_flags_writer_intervenes() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    cmpb $0, %r11b\n",
            "    sete %r10b\n",
            "    movzbl %r10b, %r10d\n",
            "    addl $1, %r8d\n",
            "    testq %r10, %r10\n",
            "    cmovneq %r8, %rbx\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("sete %r10b"), "{out}");
        assert!(out.contains("testq %r10, %r10"), "{out}");
    }

    #[test]
    fn boolean_roundtrip_is_kept_when_the_boolean_is_used_again() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    cmpb $0, %r11b\n",
            "    sete %r10b\n",
            "    movzbl %r10b, %r10d\n",
            "    testq %r10, %r10\n",
            "    cmovneq %r8, %rbx\n",
            "    movl %r10d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("sete %r10b"), "{out}");
    }

    #[test]
    fn cmove_inverts_the_condition() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    cmpl %edx, %ecx\n",
            "    setg %r10b\n",
            "    movzbl %r10b, %r10d\n",
            "    testq %r10, %r10\n",
            "    cmoveq %r8, %rbx\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("cmovleq %r8, %rbx"), "{out}");
    }

    #[test]
    fn copy_and_mask_becomes_movzbl() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %eax, %r9d\n",
            "    andl $255, %r9d\n",
            "    movl (%rcx,%r9,4), %r10d\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movzbl %al, %r9d"), "{out}");
        assert!(!out.contains("andl $255"), "{out}");
    }

    #[test]
    fn copy_and_mask_is_kept_when_flags_are_read() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %eax, %r9d\n",
            "    andl $255, %r9d\n",
            "    jne .LBB4\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("andl $255, %r9d"), "{out}");
    }
}
