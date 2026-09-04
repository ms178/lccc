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
use super::helpers::{get_dest_reg, writes_family_full};
use super::liveness::FileLiveness;
use super::relay_and_lea::{
    dead_in_block_after, family_private_to, function_range, line_refs_family, plain_gp_operand,
    split_two_operands,
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
    "movl ",
    "movq ",
    "movb ",
    "movw ",
    "movabsq ",
    "movabs ",
    "movzbl ",
    "movzbq ",
    "movzwl ",
    "movzwq ",
    "movsbl ",
    "movsbq ",
    "movswl ",
    "movswq ",
    "movslq ",
    "leal ",
    "leaq ",
    "movss ",
    "movsd ",
    "movaps ",
    "movapd ",
    "movups ",
    "movupd ",
    "movdqa ",
    "movdqu ",
    "movd ",
    "movq2dq ",
    "vmovss ",
    "vmovsd ",
    "vmovaps ",
    "vmovapd ",
    "vmovups ",
    "vmovupd ",
    "vmovdqa ",
    "vmovdqu ",
    "cvtsi2sd ",
    "cvtsi2ss ",
    "cvttsd2si ",
    "cvttss2si ",
    "cvtss2sd ",
    "cvtsd2ss ",
    "bswap ",
    "bswapl ",
    "bswapq ",
    "xchg ",
    "nop",
    "prefetch",
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
    if matches!(
        t,
        "ret" | "leave" | "cltq" | "cqto" | "cwtl" | "cdqe" | "nop" | "endbr64" | "cltd" | "cwtd"
            | "cdq" | "cwd" | "cbw" | "cwde" | "cbtw"
    ) {
        return FlagsEffect::Neutral;
    }
    // Plain stack push/pop do not touch EFLAGS.  pushf is a flag READER
    // (handled above), popf a flag WRITER (handled below).  The token must
    // END at the mnemonic — `popcntq` is not a stack pop, `pushfq` is not a
    // plain push.
    if t == "push" || t.starts_with("push ") || t == "pop" || t.starts_with("pop ") {
        return FlagsEffect::Neutral;
    }
    // A call clobbers flags (they are not preserved across the SysV boundary),
    // which for our purposes is exactly a write.
    if t.starts_with("call") {
        return FlagsEffect::Writes;
    }
    // BMI2 VEX shifts (`shlx/shrx/sarx`) and the other flag-neutral BMI2
    // ops leave EFLAGS untouched; they must be classified before the prefix
    // table below, which would otherwise match them as "shl"/"shr"/"sar"
    // writers and let a later scan treat an earlier `cmp`'s flags as dead.
    // The BMI1 siblings (andn/bextr/bzhi/blsi/blsmsk/blsr) are equally
    // VEX-encoded and flag-neutral; the old WRITERS list carried
    // andn/blsi/blsmsk/blsr as flag writers, which is factually wrong and
    // lets `flags_dead_after` delete a live producer between a `cmp` and a
    // later reader (`cmp; andn; je` — andn never touches ZF).
    if t.starts_with("shlx")
        || t.starts_with("shrx")
        || t.starts_with("sarx")
        || t.starts_with("rorx")
        || t.starts_with("pdep")
        || t.starts_with("pext")
        || t.starts_with("mulx")
        || t.starts_with("andn")
        || t.starts_with("bextr")
        || t.starts_with("bzhi")
        || t.starts_with("blsi")
        || t.starts_with("blsmsk")
        || t.starts_with("blsr")
    {
        return FlagsEffect::Neutral;
    }

    // Everything below classifies on the mnemonic with a leading VEX `v`
    // stripped, so `vaddps` gets exactly the classification of `addps`.
    let base = t.strip_prefix('v').unwrap_or(t);

    // Instructions with a fixed flag-writing effect that the generic tables
    // below would misroute: sahf/stc/clc write flags without reading them;
    // shld/shrd/bts/btr/btc/ptest write CF (and friends); popf REPLACES the
    // flags from the stack (a producer, not a consumer); the SSE/AVX compare
    // family (u)comis{s,d} writes ZF/CF/PF.
    let mnem = base
        .split(|c: char| c == ' ' || c == '\t')
        .next()
        .unwrap_or(base);
    if matches!(
        mnem,
        "sahf" | "stc" | "clc" | "cld" | "std" | "shld" | "shldl" | "shldq" | "shrd" | "shrdl"
            | "shrdq" | "bts" | "btsl" | "btsq" | "btsw" | "btr" | "btrl" | "btrq" | "btrw"
            | "btc" | "btcl" | "btcq" | "btcw" | "ptest" | "popf" | "popfl" | "popfq" | "popfw"
            | "comiss" | "comisd" | "ucomiss" | "ucomisd"
    ) {
        return FlagsEffect::Writes;
    }

    // SSE/AVX data instructions never touch EFLAGS.  The generic ALU stems
    // below would otherwise match their scalar/vector spellings as flag
    // writers (`addsd` -> "add"), which is factually wrong and — far worse —
    // lets `flags_dead_after` delete a live producer between a `cmp` and a
    // later `jcc` (`cmp; addsd; je` miscompiled before this list existed).
    if fp_data_op(base) {
        return FlagsEffect::Neutral;
    }

    // Generic flag-writing ALU stems, matched with an exact size suffix
    // (l/q/w/b), a blank, or end-of-mnemonic — never `sd`/`ss`/`ps`/`pd`
    // (those are FP forms, handled above) and never sibling mnemonics
    // (`andn`, `shld`, `cmpxchg` are classified by their own rules).
    const WRITERS: &[&str] = &[
        "add", "sub", "and", "or", "xor", "cmp", "test", "inc", "dec", "neg", "imul", "mul", "div",
        "idiv", "shl", "shr", "sar", "sal", "rol", "ror", "bt", "bsf", "bsr", "popcnt", "lzcnt",
        "tzcnt", "cmpxchg", "xadd", "lock",
    ];
    if WRITERS.iter().any(|p| suffix_exact_writer(base, p)) {
        return FlagsEffect::Writes;
    }
    // Unknown: assume the worst (both).
    FlagsEffect::Reads
}

/// `stem` is a flag-writing mnemonic when it is followed by nothing, a blank
/// (operands follow), or an integer size suffix (l/q/w/b).  A suffix that
/// starts a different mnemonic (`addsd`, `andn`, `shld`, `btc`) is not one.
#[inline]
fn suffix_exact_writer(base: &str, stem: &str) -> bool {
    let Some(rest) = base.strip_prefix(stem) else {
        return false;
    };
    rest.is_empty() || matches!(rest.as_bytes()[0], b' ' | b'l' | b'q' | b'w' | b'b')
}

/// SSE/AVX data mnemonics (and their VEX `v`-prefixed spellings via the
/// caller's base) that never read or write EFLAGS.
#[inline]
fn fp_data_op(base: &str) -> bool {
    const FP_NEUTRAL: &[&str] = &[
        // scalar/vector FP arithmetic, compares, rounding, sqrt, rcps
        "addsd", "addss", "addps", "addpd", "addsubps", "addsubpd",
        "subsd", "subss", "subps", "subpd",
        "mulsd", "mulss", "mulps", "mulpd",
        "divsd", "divss", "divps", "divpd",
        "minss", "minsd", "minps", "minpd",
        "maxss", "maxsd", "maxps", "maxpd",
        "sqrtss", "sqrtsd", "sqrtps", "sqrtpd",
        "roundss", "roundsd", "roundps", "roundpd",
        "rcpss", "rcpps", "rsqrtss", "rsqrtps",
        "shufps", "shufpd",
        "andps", "andpd", "andnps", "andnpd", "orps", "orpd", "xorps", "xorpd",
        "unpcklps", "unpckhps", "unpcklpd", "unpckhpd",
        "cvt", "cvtt",
        "hadd", "hsub",
        // integer SIMD data ops
        "pshufb", "pshufd", "pshufhw", "pshuflw", "palignr",
        "padd", "psub", "pmul", "pmadd", "pavg", "psll", "psrl", "psra",
        "pand", "pandn", "por", "pxor",
        "pcmpeq", "pcmpgt", "pmins", "pminu", "pmaxs", "pmaxu",
        "psadbw", "pmuludq", "mpsadbw", "phadd", "phsub", "psign", "pabs",
        "pmaddubsw", "pack", "punpck", "pinsr", "pextr", "extract", "insert",
        "insertps", "extractps", "blend", "pblend", "perm", "maskmov",
        // data movement / mask extraction / broadcasts / gathers / FMA / AES
        "movdqa", "movdqu", "movaps", "movups", "movapd", "movupd", "movdq",
        "movhlps", "movlhps", "movnti", "movntq", "movntdqa", "lddqu",
        "movshdup", "movsldup", "movddup", "movmskps", "movmskpd", "pmovmskb",
        "broadcast", "pbroadcast", "gather", "pgather",
        "fmadd", "fmsub", "fnmadd", "fnmsub", "aes", "pclmul",
        "zero", "ldmxcsr", "stmxcsr",
        // moves and LEAs (covers v-prefixed data moves too: vmovq, vmovd)
        "mov", "lea", "xchg", "not",
    ];
    FP_NEUTRAL.iter().any(|p| base.starts_with(p))
}

/// True when the flags written at `from - 1` are dead: scanning forward inside
/// the block, a flags WRITER is reached before any flags READER.  The single
/// oracle every flag-discarding transform must consult — a one-instruction
/// peek lets an intervening flag-NEUTRAL instruction (`mov`, `lea`, `andn`,
/// `blsi`, ...) hide a later `jcc`/`setcc`/`adc` that still reads the value
/// (the `addq $imm` → `leaq` rewrite in local_patterns miscompiled exactly
/// this shape).
pub(super) fn flags_dead_after(store: &LineStore, infos: &[LineInfo], from: usize) -> bool {
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
    let mut lv = FileLiveness::new(store, infos);
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
        while let Some(n) = next_real(infos, k, len) {
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
        let dead = matches!(lv.live_after(cmov_i, bool_fam), Some(false))
            || dead_in_block_after(store, infos, cmov_i + 1, bool_fam)
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
        lv.refresh_at(store, infos, cmov_i);
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

// ── 4b. copy + mask + flag consumer → test ─────────────────────────────────

/// `movl %A,%D; andl $imm,%D; j{e,ne}/setcc/cmov…` with `%D` dead afterwards
/// → `testl $imm,%A` (`testb $imm,%Ab` when the mask fits a byte and every
/// consumer is ZF-only).
///
/// The backend lowers `if (!(x & 1))` as a copy, a destructive `and` and a
/// branch; the copy exists only because `and` clobbers its operand. `test`
/// computes the same result without writing it, and sets *exactly* the same
/// flags as `and` (ZF/SF/PF from the result, CF/OF cleared), so at equal width
/// the rewrite is flag-transparent for every consumer, including `js`/`jp`.
/// The byte form narrows SF/PF to bit 7, hence the ZF-only requirement.
///
/// GCC/Clang/ICX emit `testb $1, %dil` for the `ffs1` kernel where we emitted
/// `movl %ebx,%esi; andl $1,%esi; jne` (2 instructions + a copy per
/// iteration of the shift loop).
///
/// Soundness: `%D` must be dead after the `and` (whole-function dataflow via
/// [`FileLiveness`]); `%A` is only read; nothing between the `mov` and the
/// `and` exists (adjacent real lines). Families 4/5 (`%rsp`/`%rbp`) never take
/// the byte form.
pub(super) fn fold_copy_and_mask_into_test(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut lv = FileLiveness::new(store, infos);
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
        // The copy must be at least as wide as the mask: a 32-bit copy zero-
        // extends, so a 64-bit `and` afterwards reads zeros we would not
        // reproduce by testing the original 64-bit source.
        if !wide && and_wide {
            i += 1;
            continue;
        }
        let Some((imm_text, and_dst)) = split_two_operands(arest) else {
            i += 1;
            continue;
        };
        let Some(imm) = imm_value(imm_text) else {
            i += 1;
            continue;
        };
        if !names_family_at_width(and_dst, dst_fam, and_wide) {
            i += 1;
            continue;
        }
        // The masked value itself must be dead: only the flags survive.
        if !matches!(lv.live_after(j, dst_fam), Some(false)) {
            i += 1;
            continue;
        }
        // Someone must actually consume the flags in this block, otherwise the
        // pair is dead-code territory, not ours.
        let Some(k) = next_real(infos, j, len) else {
            i += 1;
            continue;
        };
        if flags_effect(infos[k].trimmed(store.get(k))) != FlagsEffect::Reads {
            i += 1;
            continue;
        }
        let zf_only = flag_consumers_are_zf_only(store, infos, j + 1);
        let byte_form = zf_only && (0..=255).contains(&imm) && src_fam != 4 && src_fam != 5;
        let new_line = if byte_form {
            format!("    testb ${}, {}", imm, REG_NAMES[3][src_fam as usize])
        } else if and_wide {
            format!("    testq ${}, {}", imm, REG_NAMES[0][src_fam as usize])
        } else {
            format!("    testl ${}, {}", imm, REG_NAMES[1][src_fam as usize])
        };
        mark_nop(&mut infos[i]);
        replace_line(store, &mut infos[j], j, new_line);
        // `%A` now lives up to the `test`; keep the oracle exact for later hits.
        lv.refresh_at(store, infos, j);
        changed = true;
        i = j + 1;
    }
    changed
}

// ── 5. redundant self-test after a logical op ────────────────────────────────

/// `andl $1, %esi; testq %rsi, %rsi; je` → `andl $1, %esi; je`.
///
/// `and`/`or`/`xor` set ZF/SF/PF from the result and clear CF/OF — exactly the
/// flag state a `test` of that result produces, so the test is pure overhead.
/// The i686 backend already does this (`check_redundant_test_elimination.sh`);
/// x86-64 kept the pair because the codegen emits a 64-bit `testq` over a
/// 32-bit logical result.
///
/// Width rule: a 32-bit logical op zero-extends, so ZF is identical for the
/// 64-bit test, but SF is not (bit 31 vs bit 63). When the widths differ the
/// fold is therefore only applied if every consumer of the flags tests ZF.
#[allow(clippy::needless_range_loop)]
pub(super) fn eliminate_redundant_self_test(store: &LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut j = 0;
    while j < len {
        if infos[j].is_nop() || infos[j].pinned {
            j += 1;
            continue;
        }
        let t = infos[j].trimmed(store.get(j));
        let (test_wide, args) = if let Some(a) = t.strip_prefix("testq ") {
            (true, a)
        } else if let Some(a) = t.strip_prefix("testl ") {
            (false, a)
        } else {
            j += 1;
            continue;
        };
        let Some((a, b)) = split_two_operands(args) else {
            j += 1;
            continue;
        };
        if a != b {
            j += 1;
            continue;
        }
        let fam = register_family_fast(a);
        if fam == REG_NONE || fam > REG_GP_MAX {
            j += 1;
            continue;
        }
        let mask = 1u16 << fam;

        // Walk back to the producer through flag-neutral instructions.
        let mut k = j;
        let mut producer = None;
        while k > 0 {
            k -= 1;
            if infos[k].is_nop() || infos[k].kind == LineKind::Directive {
                continue;
            }
            if infos[k].pinned || infos[k].is_barrier() {
                break;
            }
            let tk = infos[k].trimmed(store.get(k));
            if infos[k].reg_refs & mask != 0 {
                producer = Some((k, tk));
                break;
            }
            if flags_effect(tk) != FlagsEffect::Neutral {
                break; // another instruction owns the flags
            }
        }
        let Some((pidx, ptext)) = producer else {
            j += 1;
            continue;
        };
        // Only logical ops reproduce a `test`'s flag state exactly.
        let (prod_wide, is_logical) = if ptext.starts_with("andq ")
            || ptext.starts_with("orq ")
            || ptext.starts_with("xorq ")
        {
            (true, true)
        } else if ptext.starts_with("andl ")
            || ptext.starts_with("orl ")
            || ptext.starts_with("xorl ")
        {
            (false, true)
        } else {
            (false, false)
        };
        if !is_logical || get_dest_reg(&infos[pidx]) != fam {
            j += 1;
            continue;
        }
        // The producer must WRITE the whole tested value: a 32-bit op under a
        // 64-bit test is fine (zero extension), the reverse is not.
        if prod_wide && !test_wide {
            j += 1;
            continue;
        }
        // Anything between the producer and the test must leave both the flags
        // and the register alone.
        for n in pidx + 1..j {
            if infos[n].is_nop() || infos[n].kind == LineKind::Directive {
                continue;
            }
            let tn = infos[n].trimmed(store.get(n));
            if flags_effect(tn) != FlagsEffect::Neutral || infos[n].reg_refs & mask != 0 {
                producer = None;
                break;
            }
        }
        if producer.is_none() {
            j += 1;
            continue;
        }
        // With mismatched widths only ZF survives unchanged.
        if prod_wide != test_wide {
            let Some((fstart, fend)) = function_range(store, infos, j) else {
                j += 1;
                continue;
            };
            if !flags_are_block_local(store, infos, fstart, fend)
                || !flag_consumers_are_zf_only(store, infos, j + 1)
            {
                j += 1;
                continue;
            }
        }
        mark_nop(&mut infos[j]);
        changed = true;
        j += 1;
    }
    changed
}

/// Verify the whole function keeps EFLAGS inside a basic block: every
/// flag-reading instruction is preceded, in its own block, by a flag writer.
///
/// The peephole flag transforms reason about a single block; if some block
/// consumed flags produced by a PREDECESSOR, a rewrite could change what that
/// consumer sees. LCCC's codegen (like GCC's and LLVM's) never keeps flags live
/// across a join, and this check turns that convention into a verified
/// precondition instead of an assumption.
#[allow(clippy::needless_range_loop)]
fn flags_are_block_local(
    store: &LineStore,
    infos: &[LineInfo],
    fstart: usize,
    fend: usize,
) -> bool {
    let mut written = false;
    for n in fstart..fend {
        if infos[n].is_nop() || infos[n].kind == LineKind::Directive {
            continue;
        }
        if infos[n].kind == LineKind::Label {
            written = false;
            continue;
        }
        let t = infos[n].trimmed(store.get(n));
        match flags_effect(t) {
            FlagsEffect::Reads => {
                if !written {
                    return false;
                }
            }
            FlagsEffect::Writes => written = true,
            FlagsEffect::Neutral => {}
        }
        if matches!(infos[n].kind, LineKind::Call) {
            written = true; // a call leaves the flags undefined
        }
    }
    true
}

/// True when every consumer of the current flags, up to the next flags writer,
/// only tests ZF (`e`/`ne`).
pub(super) fn flag_consumers_are_zf_only(
    store: &LineStore,
    infos: &[LineInfo],
    from: usize,
) -> bool {
    let mut n = from;
    let mut saw_consumer = false;
    while n < store.len() {
        if infos[n].is_nop() || infos[n].kind == LineKind::Directive {
            n += 1;
            continue;
        }
        // Structural cases first: `flags_effect` classifies anything it does
        // not recognise as a READER, and a label line is exactly that.
        match infos[n].kind {
            // End of the block. Flags are block-local (verified by
            // `flags_are_block_local`), so nothing beyond depends on them.
            LineKind::Label | LineKind::Ret => return saw_consumer,
            // A call leaves EFLAGS undefined: the value is dead from here.
            LineKind::Call => return saw_consumer,
            // Stack adjustments do not touch EFLAGS.
            LineKind::Push { .. } | LineKind::Pop { .. } => {
                n += 1;
                continue;
            }
            LineKind::Jmp | LineKind::JmpIndirect => return false,
            _ => {}
        }
        let t = infos[n].trimmed(store.get(n));
        match flags_effect(t) {
            FlagsEffect::Writes => return saw_consumer,
            FlagsEffect::Neutral => {}
            FlagsEffect::Reads => {
                saw_consumer = true;
                let cc = if let Some(rest) = t.strip_prefix("set") {
                    rest.split_whitespace().next().unwrap_or("")
                } else if let Some(rest) = t.strip_prefix("cmov") {
                    let tag = rest.split_whitespace().next().unwrap_or("");
                    tag.strip_suffix(|c| c == 'q' || c == 'l' || c == 'w')
                        .unwrap_or(tag)
                } else if t.starts_with('j') {
                    &t[1..t.find(' ').unwrap_or(t.len())]
                } else {
                    return false; // adc/sbb/... consume CF
                };
                if !matches!(cc, "e" | "z" | "ne" | "nz") {
                    return false;
                }
            }
        }
        n += 1;
    }
    saw_consumer
}

/// Whole-name occurrence test: `%r8` must not match inside `%r8d`.
fn mentions_exact_name(line: &str, name: &str) -> bool {
    let bytes = line.as_bytes();
    let nb = name.as_bytes();
    let mut pos = 0;
    while pos + nb.len() <= bytes.len() {
        if &bytes[pos..pos + nb.len()] == nb {
            let after = pos + nb.len();
            if after >= bytes.len() || !bytes[after].is_ascii_alphanumeric() {
                return true;
            }
        }
        pos += 1;
    }
    false
}

// ── 6. narrow a sign extension nobody reads at 64 bits ───────────────────────

/// `movslq %edx, %r8` → `movl %edx, %r8d` when the 64-bit half of the result is
/// never read. The narrower form is the same length but feeds the copy folds
/// (`producer_retarget`, `eliminate_move_relays`), which cannot touch a
/// sign-extending producer because it changes the value's width class.
pub(super) fn narrow_dead_sign_extension(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let lv = FileLiveness::new(store, infos);
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        let t = infos[i].trimmed(store.get(i)).to_string();
        let Some(rest) = t.strip_prefix("movslq ") else {
            i += 1;
            continue;
        };
        let Some((src, dst)) = split_two_operands(rest) else {
            i += 1;
            continue;
        };
        let Some(dst_fam) = plain_gp_operand(dst) else {
            i += 1;
            continue;
        };
        if dst != REG_NAMES[0][dst_fam as usize] || !src.starts_with('%') {
            i += 1;
            continue;
        }
        let name64 = REG_NAMES[0][dst_fam as usize];
        let mask = 1u16 << dst_fam;
        let mut ok = false;
        let mut j = i + 1;
        while j < len {
            if infos[j].is_nop() || infos[j].kind == LineKind::Directive {
                j += 1;
                continue;
            }
            if infos[j].is_barrier() || infos[j].pinned {
                // Safe to stop here only if the register is dead from now on.
                ok = matches!(lv.live_after(j - 1, dst_fam), Some(false));
                break;
            }
            let tj = infos[j].trimmed(store.get(j));
            if infos[j].reg_refs & mask != 0 {
                if mentions_exact_name(tj, name64) {
                    break; // a 64-bit read observes the sign extension
                }
                // An implicit READ of the family observes the extension just
                // as much as a named 64-bit operand does: `cqto` consumes
                // all 64 bits of %rax, `idivq` reads %rax:%rdx. The
                // `reg_refs` union is what routes these lines here at all.
                if implicit_read_refs(tj.as_bytes()) & mask != 0 {
                    break;
                }
                // "Fully redefined" must mean a FULL-WIDTH write of the
                // family: a `cqto` is classified with %rax as its primary
                // destination, but %rax is its implicit input — the only
                // family it writes is %rdx — so it can never retire a %rax
                // identity here.  And a PARTIAL write (`movb $1, %al`,
                // `lahf`, `setcc`) leaves the upper bits of the old value
                // live, so a later 64-bit read would still observe them:
                // the any-width `writes_family` must not end the search.
                // `writes_family_full` proves the whole architectural
                // family rewritten (32-bit writes zero-extend).  cmov is
                // additionally excluded: its write is conditional.
                if writes_family_full(&infos[j], tj, dst_fam) && !tj.starts_with("cmov") {
                    ok = true; // fully redefined before any wide read
                    break;
                }
            }
            j += 1;
        }
        if ok {
            let new_line = format!("    movl {}, {}", src, REG_NAMES[1][dst_fam as usize]);
            replace_line(store, &mut infos[i], i, new_line);
            changed = true;
        }
        i += 1;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::super::super::peephole_optimize;
    use super::{flags_effect, FlagsEffect};

    fn run(asm: &str) -> String {
        peephole_optimize(asm.to_string())
    }

    #[test]
    fn self_test_after_and_is_removed() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    andl $1, %esi\n",
            "    testq %rsi, %rsi\n",
            "    je .LBB4\n",
            "    ret\n",
            ".LBB4:\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("andl $1, %esi"), "{out}");
        assert!(!out.contains("testq %rsi, %rsi"), "{out}");
    }

    #[test]
    fn self_test_after_and_is_kept_for_a_sign_consumer() {
        // SF of a 32-bit `andl` is bit 31; `testq` reports bit 63. A signed
        // consumer must keep the wide test.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    andl $255, %esi\n",
            "    testq %rsi, %rsi\n",
            "    js .LBB4\n",
            "    ret\n",
            ".LBB4:\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("testq %rsi, %rsi"), "{out}");
    }

    #[test]
    fn self_test_after_add_is_kept() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    addl %edx, %esi\n",
            "    testl %esi, %esi\n",
            "    jb .LBB4\n",
            "    ret\n",
            ".LBB4:\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("testl %esi, %esi"), "{out}");
    }

    #[test]
    fn dead_sign_extension_is_narrowed() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movslq %edx, %r8\n",
            "    movl %r8d, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("movslq"), "{out}");
    }

    #[test]
    fn live_sign_extension_is_kept() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movslq %edx, %r8\n",
            "    movq %r8, (%rsi)\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("movslq %edx, %r8"), "{out}");
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
    #[test]
    fn copy_and_mask_branch_becomes_testb() {
        // ffs1 kernel shape: `while (!(x & 1))`, mask temp dead after the branch.
        let asm = concat!(
            ".cfi_startproc\n",
            ".LBB4:\n",
            "    movl %ebx, %esi\n",
            "    andl $1, %esi\n",
            "    jne .LBB5\n",
            "    shrl $1, %ebx\n",
            "    addl $1, %edi\n",
            "    movl %ebx, %edx\n",
            "    andl $1, %edx\n",
            "    je .LBB4\n",
            ".LBB5:\n",
            "    movl %edi, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let out = peephole_optimize(asm.to_string());
        assert!(out.contains("testb $1, %bl"), "{out}");
        assert!(!out.contains("andl $1, %esi"), "{out}");
        assert!(!out.contains("andl $1, %edx"), "{out}");
        assert!(!out.contains("movl %ebx, %esi"), "{out}");
    }

    #[test]
    fn copy_and_mask_keeps_and_when_masked_value_is_live() {
        // The masked value feeds a later add: the `and` must stay.
        let asm = concat!(
            ".cfi_startproc\n",
            "    movl %ebx, %esi\n",
            "    andl $1, %esi\n",
            "    jne .LBB5\n",
            "    addl %esi, %edi\n",
            ".LBB5:\n",
            "    movl %edi, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let out = peephole_optimize(asm.to_string());
        assert!(out.contains("andl $1, %esi"), "{out}");
        assert!(!out.contains("testb"), "{out}");
    }

    #[test]
    fn copy_and_mask_sign_consumer_uses_full_width_test() {
        // `js` reads SF of the 32-bit result: the byte form would be wrong,
        // the dword `test` is flag-identical to the `and`.
        let asm = concat!(
            ".cfi_startproc\n",
            "    movl %ebx, %esi\n",
            "    andl $-2147483648, %esi\n",
            "    js .LBB5\n",
            "    xorl %eax, %eax\n",
            "    ret\n",
            ".LBB5:\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let out = peephole_optimize(asm.to_string());
        assert!(out.contains("testl $-2147483648, %ebx"), "{out}");
        assert!(!out.contains("andl"), "{out}");
    }

    #[test]
    fn copy_and_mask_narrow_copy_wide_mask_is_left_alone() {
        // movl zero-extends; a 64-bit and afterwards tests bits the original
        // %rbx may have set — no fold.
        let asm = concat!(
            ".cfi_startproc\n",
            "    movl %ebx, %esi\n",
            "    andq $255, %rsi\n",
            "    jne .LBB5\n",
            ".LBB5:\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let out = peephole_optimize(asm.to_string());
        assert!(out.contains("andq $255, %rsi"), "{out}");
    }

    #[test]
    fn bmi1_vex_ops_are_flag_neutral_not_flag_writers() {
        // BMI1 is VEX-encoded and leaves EFLAGS untouched.  The old WRITERS
        // list carried andn/blsi/blsmsk/blsr as flag writers; classifying
        // them as Writes lets `flags_dead_after` delete a live `cmp` that a
        // later `je` still reads (`cmp; andn; je` miscompiles).
        let tag = |e: FlagsEffect| match e {
            FlagsEffect::Neutral => "neutral",
            FlagsEffect::Writes => "writes",
            FlagsEffect::Reads => "reads",
        };
        for m in [
            "andnq %rdx, %rax, %rax",
            "andnl %edx, %eax, %eax",
            "blsiq %rcx, %rax",
            "blsmskq %rcx, %rax",
            "blsrq %rcx, %rax",
            "bextrq %rcx, %rax, %rdx",
            "bzhiq %rcx, %rax, %rdx",
        ] {
            assert!(
                matches!(flags_effect(m), FlagsEffect::Neutral),
                "{m} classified {}",
                tag(flags_effect(m))
            );
        }
    }

    #[test]
    fn sse_arith_is_flag_neutral_not_a_flag_writer() {
        // `addsd` and its FP siblings write only their XMM destination;
        // classifying them as flag writers lets `flags_dead_after` retire a
        // live `cmp` between them and a later branch.
        let tag = |e: FlagsEffect| match e {
            FlagsEffect::Neutral => "neutral",
            FlagsEffect::Writes => "writes",
            FlagsEffect::Reads => "reads",
        };
        for m in [
            "addsd %xmm1, %xmm2",
            "addss %xmm1, %xmm2",
            "addps %xmm1, %xmm2",
            "addpd %xmm1, %xmm2",
            "subsd %xmm1, %xmm2",
            "mulsd %xmm1, %xmm2",
            "divsd %xmm1, %xmm2",
            "xorps %xmm1, %xmm2",
            "andps %xmm1, %xmm2",
            "orps %xmm1, %xmm2",
            "minss %xmm1, %xmm2",
            "maxpd %xmm1, %xmm2",
            "sqrtss %xmm1, %xmm2",
            "shufps $27, %xmm1, %xmm2",
            "paddq %xmm1, %xmm2",
            "psubw %xmm1, %xmm2",
            "pmulld %xmm1, %xmm2",
            "pand %xmm1, %xmm2",
            "por %xmm1, %xmm2",
            "pxor %xmm1, %xmm2",
            "pshufb %xmm1, %xmm2",
            "pcmpeqd %xmm1, %xmm2",
            "cvtsi2sd %eax, %xmm1",
            "roundss $1, %xmm1, %xmm2",
            "vaesenc %xmm1, %xmm2, %xmm3",
            "vpand %xmm1, %xmm2, %xmm3",
            "vaddps %ymm1, %ymm2, %ymm3",
            "vmulsd %xmm1, %xmm2, %xmm3",
            "vdivpd %ymm1, %ymm2, %ymm3",
            "vfmadd231ps %ymm1, %ymm2, %ymm3",
            "vpclmulqdq $0, %xmm1, %xmm2, %xmm3",
            "vpxor %ymm1, %ymm2, %ymm3",
            "vminpd %ymm1, %ymm2, %ymm3",
            "vblendvps %ymm1, %ymm2, %ymm3, %ymm4",
            "vperm2f128 $1, %ymm1, %ymm2, %ymm3",
            "vzeroupper",
            "vzeroall",
            "movdqa %xmm1, %xmm2",
            "vmovups %ymm1, (%rdi)",
        ] {
            assert!(
                matches!(flags_effect(m), FlagsEffect::Neutral),
                "{m} classified {}",
                tag(flags_effect(m))
            );
        }
    }

    #[test]
    fn exact_flag_writers_are_writers() {
        let tag = |e: FlagsEffect| match e {
            FlagsEffect::Neutral => "neutral",
            FlagsEffect::Writes => "writes",
            FlagsEffect::Reads => "reads",
        };
        for m in [
            "sahf",
            "stc",
            "clc",
            "cld",
            "std",
            "popfq",
            "shldl %cl, %eax, %ebx",
            "shrdq %cl, %rax, %rbx",
            "btsq %rax, %rbx",
            "btrq %rax, %rbx",
            "btcq %rax, %rbx",
            "ptest %xmm1, %xmm2",
            "vptest %ymm1, %ymm2",
            "ucomiss %xmm1, %xmm2",
            "comisd %xmm1, %xmm2",
            "vucomisd %xmm1, %xmm2",
        ] {
            assert!(
                matches!(flags_effect(m), FlagsEffect::Writes),
                "{m} classified {}",
                tag(flags_effect(m))
            );
        }
    }

    #[test]
    fn cmp_survives_across_a_bmi1_op_the_branch_reads() {
        // End-to-end: `copy + add` folds to a scaled LEA only when the add's
        // flags are dead (`flags_dead_after`).  With `andn` misclassified as
        // a flags writer the add's flags looked dead although the later
        // `jne` reads them — the fold then changed control flow.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %r9, %rax\n",
            "    addq $5, %rax\n",
            "    andnq %rdx, %rbx, %rbx\n",
            "    jne .LBB4\n",
            "    ret\n",
            ".LBB4:\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(
            out.contains("addq $5, %rax"),
            "the add's flags feed the jne through a flag-neutral andn: {out}"
        );
        assert!(out.contains("andnq %rdx, %rbx, %rbx"), "{out}");
    }

    #[test]
    fn add_survives_across_sse_arith_the_branch_reads() {
        // The `movq+addq` -> `leaq` fold discards the add's flags; a
        // flag-NEUTRAL `addsd` in between must not hide the `je` that still
        // reads them.  (Before the FP classification this folded and the
        // branch read garbage flags.)
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %r9, %rax\n",
            "    addq $5, %rax\n",
            "    addsd %xmm1, %xmm2\n",
            "    je .LBB4\n",
            "    ret\n",
            ".LBB4:\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(
            out.contains("addq $5, %rax"),
            "the add's flags feed the je through a flag-neutral addsd: {out}"
        );
        assert!(out.contains("addsd %xmm1, %xmm2"), "{out}");
    }

    #[test]
    fn add_folds_across_a_true_flag_writer() {
        // The positive control: `popcntq` writes the flags the `je` reads,
        // so the add's flags are dead and the fold is legal.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %r9, %rax\n",
            "    addq $5, %rax\n",
            "    popcntq %rbx, %rbx\n",
            "    je .LBB4\n",
            "    ret\n",
            ".LBB4:\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(
            out.contains("leaq 5(%r9), %rax"),
            "the add must fold across a real flag writer: {out}"
        );
        assert!(!out.contains("addq $5, %rax"), "{out}");
    }

    #[test]
    fn move_survives_a_partial_redefinition_of_its_destination() {
        // `eliminate_dead_reg_moves` must not delete the move across a
        // PARTIAL write of its destination family: `movb $1, %al` leaves the
        // upper bits of %rax as the move wrote them, and the final copy
        // ships them out.  (The any-width `writes_family` acceptance made
        // this `movq %r9, %rax` disappear — %rbx received garbage.)
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %r9, %rax\n",
            "    movb $1, %al\n",
            "    movq %rax, %rbx\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(
            out.contains("movq %r9, %rax"),
            "the move's value survives the byte write: {out}"
        );
        assert!(out.contains("movb $1, %al"), "{out}");
    }

    #[test]
    fn sign_extension_survives_a_partial_redefinition() {
        // `movslq` narrows to `movl` only when the 64-bit value is fully
        // redefined before any wide read.  `movb $1, %al` rewrites just the
        // low byte; the `ret` still observes the sign extension's upper
        // bits, so the narrow would corrupt the return value.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movslq %eax, %rax\n",
            "    movb $1, %al\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(
            out.contains("movslq %eax, %rax"),
            "sign extension must survive a partial redefinition: {out}"
        );
        assert!(!out.contains("movl %eax, %eax"), "{out}");
    }

    #[test]
    fn sign_extension_survives_a_lahf_partial_write() {
        // Same hazard through an IMPLICIT partial write: `lahf` sets only AH.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movslq %eax, %rax\n",
            "    lahf\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(
            out.contains("movslq %eax, %rax"),
            "sign extension must survive lahf: {out}"
        );
    }

    #[test]
    fn sign_extension_narrows_across_a_full_redefinition() {
        // The positive control: a 32-bit write zero-extends and fully
        // redefines the family, so the extension may be narrowed away
        // entirely (`movl $1, %eax` alone is the full %rax value).
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movslq %eax, %rax\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(
            !out.contains("movslq"),
            "full redefinition must retire the extension: {out}"
        );
        assert!(out.contains("movl $1, %eax"), "{out}");
    }

    
}
