//! Flags-aware x86-64 peepholes.
//!
//! Three transforms that need an explicit model of the EFLAGS lifetime, which
//! the other peephole passes deliberately avoid touching:
//!
//! 1. [`fold_copy_add_into_lea`] — `movl %A,%D; addl $imm,%D` → `leal imm(%A),%D`,
//!    and `movl %A,%D; addl %R,%D` → `leal (%A,%R),%D`.
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
use super::helpers::{get_dest_reg, writes_family, writes_family_full};
use super::liveness::FileLiveness;
use super::relay_and_lea::{
    dead_in_block_after, family_private_to, function_range, line_refs_family, plain_gp_operand,
    provably_dead, rbp_is_gpr_in_function, split_two_operands,
};

/// What a line does to EFLAGS.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

/// Bits for the arithmetic flags a rewrite can be observed through.
pub(super) const F_ZF: u8 = 1 << 0;
pub(super) const F_CF: u8 = 1 << 1;
pub(super) const F_SF: u8 = 1 << 2;
pub(super) const F_OF: u8 = 1 << 3;
pub(super) const F_PF: u8 = 1 << 4;
pub(super) const F_AF: u8 = 1 << 5;
/// Every arithmetic flag.  The default for anything not listed below: a mask
/// narrower than the truth is a miscompile, a mask wider than the truth only
/// costs a missed fold.
pub(super) const F_ALL: u8 = F_ZF | F_CF | F_SF | F_OF | F_PF | F_AF;

/// The flags a writer instruction can clobber, as a bitmask.
///
/// [`walk_flag_consumers`] used to treat every `FlagsEffect::Writes` as a
/// total clobber.  That is sound but far too coarse for a rewrite that
/// PROVES it preserves ZF and CF, because two very common flag setters write
/// only CF:
///
/// ```asm
///     cmpl  %r9d, %r8d
///     jb    .Lkeep        ; an allowed ZF/CF consumer; pushes .Lkeep
///     addl  $1, %eax      ; a real clobber, on the fall-through only
/// .Lkeep:
///     stc                 ; writes CF -- and ONLY CF
///     jo    .Loverflow    ; reads OF, which `stc` left intact
/// ```
///
/// Stopping at `stc` hides `jo` from the walk, and `jo` observes exactly the
/// OF that a narrowing `cmp` changes.  So the fold was admitted on a path
/// where it is observable.  This function lets such a writer be stepped over
/// without pretending it clobbered everything.
///
/// Only three families are narrowed, and each is a direct SDM statement:
/// `stc`/`clc`/`cmc` define CF alone; `sahf` loads SF/ZF/AF/PF/CF from AH and
/// explicitly leaves OF alone; `cld`/`std` touch DF, which is not one of the
/// six arithmetic flags.  Everything else -- including `shld`/`shrd`/`bt*`,
/// which do write OF and AF, and every unrecognised mnemonic -- returns
/// [`F_ALL`], so no existing call can become less conservative than before.
pub(super) fn flags_written_mask(t: &str) -> u8 {
    let base = t.strip_prefix('v').unwrap_or(t);
    let mnem = base
        .split(|c: char| c == ' ' || c == '\t')
        .next()
        .unwrap_or(base);
    match mnem {
        "stc" | "clc" | "cmc" => F_CF,
        // SDM: "LAHF/SAHF ... The OF flag is not affected."
        "sahf" => F_SF | F_ZF | F_PF | F_AF | F_CF,
        // Direction flag only; none of the six arithmetic flags change.
        "cld" | "std" => 0,
        _ => F_ALL,
    }
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
        "ret"
            | "leave"
            | "cltq"
            | "cqto"
            | "cwtl"
            | "cdqe"
            | "nop"
            | "endbr64"
            | "cltd"
            | "cwtd"
            | "cdq"
            | "cwd"
            | "cbw"
            | "cwde"
            | "cbtw"
    ) {
        return FlagsEffect::Neutral;
    }
    // Plain stack push/pop do not touch EFLAGS.  pushf is a flag READER
    // (handled above), popf a flag WRITER (handled below).  The mnemonic may
    // carry an operand-size suffix (`popq %rbp` — the spelling every AT&T
    // epilogue emits), which the old `starts_with("pop ")` test missed: the
    // suffixed forms fell through to the "unknown" catch-all, were treated
    // as flag READERS, and `flags_dead_after` then refused every
    // flag-discarding fold across a callee-save restore.  The stem must END
    // at the mnemonic — `popcnt` shares no stem letters with this check
    // ('t' is not a width suffix), and `pushfq`/`popfq` reduce to
    // `pushf`/`popf`, which are classified above/below as reader/writer.
    let mnem_end = t.find(|c: char| !c.is_ascii_lowercase()).unwrap_or(t.len());
    let mnem = &t[..mnem_end];
    let stem = mnem
        .strip_suffix('q')
        .or_else(|| mnem.strip_suffix('l'))
        .or_else(|| mnem.strip_suffix('w'))
        .unwrap_or(mnem);
    if matches!(stem, "pop" | "push") {
        return FlagsEffect::Neutral;
    }
    // A call clobbers flags (they are not preserved across the SysV boundary),
    // which for our purposes is exactly a write.
    if t.starts_with("call") {
        return FlagsEffect::Writes;
    }
    // BMI2 VEX shifts (`shlx/shrx/sarx`) and the other flag-neutral BMI2
    // ops (rorx/pdep/pext/mulx) leave EFLAGS untouched per the SDM; they
    // must be classified before the prefix table below, which would
    // otherwise match them as "shl"/"shr"/"sar" writers and let a later
    // scan treat an earlier `cmp`'s flags as dead.
    // The BMI1 VEX siblings (andn/bextr/bzhi/blsi/blsmsk/blsr) are NOT
    // neutral — the SDM documents flag writes for all of them (andn:
    // SF/ZF/PF set, OF/CF cleared; blsi/blsr/blsmsk: ZF/CF/SF/OF;
    // bextr/bzhi: ZF/CF), proven on hardware 2026-09-24 (`cmp; andn; je`
    // takes the branch iff andn wrote ZF — it does).  They are classified
    // as writers in the WRITERS table below: in `cmp; andn; je` the branch
    // reads ANDN's flags, so the cmp's flags are dead, not live — the old
    // comment had the dataflow exactly backwards.
    if t.starts_with("shlx")
        || t.starts_with("shrx")
        || t.starts_with("sarx")
        || t.starts_with("rorx")
        || t.starts_with("pdep")
        || t.starts_with("pext")
        || t.starts_with("mulx")
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
        "sahf"
            | "stc"
            | "clc"
            | "cld"
            | "std"
            | "shld"
            | "shldl"
            | "shldq"
            | "shrd"
            | "shrdl"
            | "shrdq"
            | "bts"
            | "btsl"
            | "btsq"
            | "btsw"
            | "btr"
            | "btrl"
            | "btrq"
            | "btrw"
            | "btc"
            | "btcl"
            | "btcq"
            | "btcw"
            | "ptest"
            | "popf"
            | "popfl"
            | "popfq"
            | "popfw"
            | "comiss"
            | "comisd"
            | "ucomiss"
            | "ucomisd"
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
    // (`shld`, `cmpxchg` are classified by their own rules; the BMI1
    // `andn`/`bextr`/`bzhi`/`blsi`/`blsmsk`/`blsr` stems below are exact
    // writers — the SSE `andnps`/`andnpd` do NOT match: `ps`/`pd` are not
    // integer suffixes, and the FP table above claims them first anyway).
    const WRITERS: &[&str] = &[
        "add", "sub", "and", "or", "xor", "cmp", "test", "inc", "dec", "neg", "imul", "mul", "div",
        "idiv", "shl", "shr", "sar", "sal", "rol", "ror", "bt", "bsf", "bsr", "popcnt", "lzcnt",
        "tzcnt", "cmpxchg", "xadd", "lock", "andn", "bextr", "bzhi", "blsi", "blsmsk", "blsr",
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
        "addsd",
        "addss",
        "addps",
        "addpd",
        "addsubps",
        "addsubpd",
        "subsd",
        "subss",
        "subps",
        "subpd",
        "mulsd",
        "mulss",
        "mulps",
        "mulpd",
        "divsd",
        "divss",
        "divps",
        "divpd",
        "minss",
        "minsd",
        "minps",
        "minpd",
        "maxss",
        "maxsd",
        "maxps",
        "maxpd",
        "cmpps",
        "cmppd",
        "blendvps",
        "blendvpd",
        "sqrtss",
        "sqrtsd",
        "sqrtps",
        "sqrtpd",
        "roundss",
        "roundsd",
        "roundps",
        "roundpd",
        "rcpss",
        "rcpps",
        "rsqrtss",
        "rsqrtps",
        "shufps",
        "shufpd",
        "andps",
        "andpd",
        "andnps",
        "andnpd",
        "orps",
        "orpd",
        "xorps",
        "xorpd",
        "unpcklps",
        "unpckhps",
        "unpcklpd",
        "unpckhpd",
        "cvt",
        "cvtt",
        "hadd",
        "hsub",
        // integer SIMD data ops
        "pshufb",
        "pshufd",
        "pshufhw",
        "pshuflw",
        "palignr",
        "padd",
        "psub",
        "pmul",
        "pmadd",
        "pavg",
        "psll",
        "psrl",
        "psra",
        "pand",
        "pandn",
        "por",
        "pxor",
        "pcmpeq",
        "pcmpgt",
        "pmins",
        "pminu",
        "pmaxs",
        "pmaxu",
        "psadbw",
        "pmuludq",
        "mpsadbw",
        "phadd",
        "phsub",
        "psign",
        "pabs",
        "pmaddubsw",
        "pack",
        "punpck",
        "pinsr",
        "pextr",
        "extract",
        "insert",
        "insertps",
        "extractps",
        "blend",
        "pblend",
        "perm",
        "maskmov",
        // data movement / mask extraction / broadcasts / gathers / FMA / AES
        "movdqa",
        "movdqu",
        "movaps",
        "movups",
        "movapd",
        "movupd",
        "movdq",
        "movhlps",
        "movlhps",
        "movnti",
        "movntq",
        "movntdqa",
        "lddqu",
        "movshdup",
        "movsldup",
        "movddup",
        "movmskps",
        "movmskpd",
        "pmovmskb",
        "broadcast",
        "pbroadcast",
        "gather",
        "pgather",
        "fmadd",
        "fmsub",
        "fnmadd",
        "fnmsub",
        "aes",
        "pclmul",
        "zero",
        "ldmxcsr",
        "stmxcsr",
        // moves and LEAs (covers v-prefixed data moves too: vmovq, vmovd)
        "mov",
        "lea",
        "xchg",
        "not",
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
/// `sub` with the displacement negated), and the register-addend form
/// `movl %A, %D` + `addl %R, %D` → `leal (%A,%R), %D` (`add` only: `lea` has
/// no subtracted index).
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
        // A self-copy is not this pattern, and %rsp is never a destination.
        if src_fam == dst_fam || dst_fam == 4 {
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
            // Register addend: `movl %A, %D; addl %R, %D` -> `leal (%A, %R), %D`.
            // R must be named at the add's width (a `%r8b` addend is not a
            // 32-bit add) and must not be D (after the copy D holds A; that
            // aliasing case is left alone) or %rsp (not encodable as index).
            // %rbp as the destination participates only when
            // `rbp_is_gpr_in_function` proves the function never establishes
            // a frame pointer — the same policy the relay retarget uses; in
            // a frame-pointer build an rbp write is the frame, not a value.
            let Some(r_fam) = plain_gp_operand(imm_text) else {
                i += 1;
                continue;
            };
            // `movq %rsp, %rbp` is the frame setup, whatever follows it.
            if neg
                || (dst_fam == 5 && src_fam == 4)
                || r_fam == dst_fam
                || r_fam == 4
                || !names_family_at_width(imm_text, r_fam, add_wide)
                || (dst_fam == 5
                    && !{
                        match function_range(store, infos, i) {
                            Some((fs, fe)) => rbp_is_gpr_in_function(store, infos, fs, fe),
                            None => false,
                        }
                    })
                || !flags_dead_after(store, infos, j + 1)
            {
                i += 1;
                continue;
            }
            let mnemonic = if add_wide { "leaq" } else { "leal" };
            let dst_name = REG_NAMES[if add_wide { 0 } else { 1 }][dst_fam as usize];
            let new_line = format!(
                "    {} ({}, {}), {}",
                mnemonic, REG_NAMES[0][src_fam as usize], REG_NAMES[0][r_fam as usize], dst_name
            );
            mark_nop(&mut infos[i]);
            replace_line(store, &mut infos[j], j, new_line);
            changed = true;
            i = j + 1;
            continue;
        };
        // The immediate form keeps its %rbp-destination exclusion (an
        // immediate adjustment of %rbp is the frame idiom, never a value).
        if dst_fam == 5 {
            i += 1;
            continue;
        }
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

// ── 1b. in-place add-immediate + copy → lea (the mirrored order) ─────────────

/// `addq $imm, %S` + `movq %S, %D` → `leaq imm(%S), %D` (same for `l`, and
/// for `sub` with the displacement negated).
///
/// The mirror of [`fold_copy_add_into_lea`] for the order the loop backend
/// actually emits: the in-place ALU computes the induction step in the phi's
/// home and then repairs the result into the temp's home — every indexed
/// load pair in a loop-carried scan pays `addq $1, %r10; movq %r10, %r9`
/// (linux_find_bit's bitmap scan measured 9 insns/word vs GCC's 7).
///
/// Soundness: the LEA computes the same value in `%D` and — the one semantic
/// difference — PRESERVES `%S` where the add destroyed it, so the transform
/// additionally requires `%S` to be dead after the copy. Flags the add wrote
/// must be dead after the pair, as in the copy-first mirror.
pub(super) fn fold_inplace_add_copy_into_lea(
    store: &mut LineStore,
    infos: &mut [LineInfo],
) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        let add = infos[i].trimmed(store.get(i));
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
        // Only the IMMEDIATE form folds (the register form would need a
        // two-source LEA; `leaq (%S,%R), %D` — a different pattern).
        let Some((imm_text, add_dst)) = split_two_operands(arest) else {
            i += 1;
            continue;
        };
        let Some(src_fam) = plain_gp_operand(add_dst) else {
            i += 1;
            continue;
        };
        if src_fam == 4 || src_fam == 5 {
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
        // The consumer: an adjacent full-width copy of %S to a different
        // register. The copy may be WIDER than the arithmetic (movq after
        // addl), never narrower (a movl copy would truncate the 64-bit
        // result the LEA must produce).
        let Some(j) = next_real(infos, i, len) else {
            i += 1;
            continue;
        };
        if infos[j].pinned {
            i += 1;
            continue;
        }
        let mov = infos[j].trimmed(store.get(j));
        let (copy_wide, mrest) = if let Some(r) = mov.strip_prefix("movq ") {
            (true, r)
        } else if let Some(r) = mov.strip_prefix("movl ") {
            (false, r)
        } else {
            i += 1;
            continue;
        };
        if !copy_wide && add_wide {
            i += 1;
            continue;
        }
        let Some((m_src, m_dst)) = split_two_operands(mrest) else {
            i += 1;
            continue;
        };
        let (Some(m_src_fam), Some(dst_fam)) = (plain_gp_operand(m_src), plain_gp_operand(m_dst))
        else {
            i += 1;
            continue;
        };
        if m_src_fam != src_fam || dst_fam == src_fam || dst_fam == 4 || dst_fam == 5 {
            i += 1;
            continue;
        }
        // The copy must read the arithmetic's destination at its exact
        // width spelling (width mismatch = different value semantics).
        if !names_family_at_width(m_src, src_fam, add_wide) {
            i += 1;
            continue;
        }
        // %S's incremented value must be dead after the copy: the LEA
        // preserves %S's ORIGINAL value, the add did not.
        if !provably_dead(store, infos, j, src_fam, &[i, j]) {
            i += 1;
            continue;
        }
        // Flags the add wrote must be dead after the pair.
        if !flags_dead_after(store, infos, j + 1) {
            i += 1;
            continue;
        }
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

/// Operands of a widening copy of an 8-bit register: `movzbl %bl, %ebx`,
/// `movsbq %bl, %r11`, `movsbl %al, %eax`, ... .  Returns `(src, dst)`.
fn widen_copy_operands(t: &str) -> Option<(&str, &str)> {
    // Only the byte-source extensions: `movzwl`/`movswl` read 16 bits, which
    // the `setcc` did not define (see the call site).
    for pfx in ["movzbl ", "movzbq ", "movsbl ", "movsbq "] {
        if let Some(args) = t.strip_prefix(pfx) {
            if let Some((s, d)) = split_two_operands(args) {
                return Some((s, d));
            }
        }
    }
    None
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
/// * the `test` is a self-test of the boolean or of a WIDENING COPY of it.
///   The IR's `Cast(bool -> i32)` lowers to a sign-extending copy into a fresh
///   register (`movsbq %bl, %r11`), and the `test` then names that register
///   instead of the boolean -- measured on the ASCII case-fold kernel, where
///   insisting on the boolean's own register left `setbe; movzbl; movsbq;
///   test; cmovne` (9 instructions per byte) in place.  Both copies are
///   accepted; sign/zero extension of a 0/1 value is 0 iff the boolean is 0,
///   so the `test` still means "the original condition holds";
/// * the `cmov` condition is `ne` (boolean true) or `e` (boolean false), which
///   maps to the original condition or its negation.
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
        // The register family the `test` and the `cmov` are matched against:
        // the boolean's own family, or the destination of a widening copy of
        // it (`movsbq %bl, %r11` -- the shape the IR's `Cast(bool -> i32)`
        // produces, which the "same family only" rule used to bail on).
        let mut tested_fam = bool_fam;
        // EVERY family a widening copy defines, not just the last one.
        //
        // The idiom this pass matches is `setCC %al; movzbl %al, %rN; test
        // %rN, %rN; cmovCC`, and the pass deletes all of those lines.  A source
        // can widen the same boolean twice (`movzbl %al, %r8d; movzbl %al,
        // %r9d`) -- and then `tested_fam` names only `%r9`, so every guard
        // tested the last destination while the deletion also took `%r8`'s
        // definition.  A surviving `cmovne %r8d, %ebx` would read a value
        // nobody defines any more.  Tracking the whole set makes the
        // intervening-line scan, the cmov-operand check and the deadness proof
        // all cover every definition the rewrite removes, and the fold still
        // fires whenever all of them really are dead -- so this costs reach
        // only for shapes that were never sound.
        let mut widened_mask: u16 = 0;
        while let Some(n) = next_real(infos, k, len) {
            k = n;
            if infos[n].pinned || infos[n].is_barrier() {
                break;
            }
            let tn = infos[n].trimmed(store.get(n));
            // A widening copy of the boolean is part of the idiom, whether it
            // lands back in the same family (`movzbl %bl, %ebx`) or in a fresh
            // register (`movsbq %bl, %r11`).  The copy's source must be the
            // boolean; the destination becomes the register the `test` names.
            if test_idx.is_none() {
                if let Some((src, dst)) = widen_copy_operands(&tn) {
                    let sfam = register_family_fast(src);
                    let dfam = register_family_fast(dst);
                    // The copy must read the EXACT register the `setcc` wrote
                    // (`%bl`, not merely its family): `setcc` defines the low
                    // byte only, so a 16-bit read of the same family
                    // (`movswl %bx, %eax`) would sign-extend whatever the high
                    // byte happens to hold -- garbage the `setcc` never wrote
                    // -- and the fused `cmov` would then follow the wrong
                    // condition.  Family equality is what makes the accepted
                    // 8-bit reads sound; text equality is what makes the wider
                    // ones impossible.
                    if sfam == bool_fam && src == breg && dfam != REG_NONE && dfam <= REG_GP_MAX {
                        tested_fam = dfam;
                        widened_mask |= 1u16 << dfam;
                        owned.push(n);
                        continue;
                    }
                }
            }
            if test_idx.is_none() {
                if let Some(args) = tn
                    .strip_prefix("testq ")
                    .or_else(|| tn.strip_prefix("testl "))
                    .or_else(|| tn.strip_prefix("testb "))
                {
                    if let Some((a, b)) = split_two_operands(args) {
                        if a == b && register_family_fast(a) == tested_fam {
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
                    break;
                }
                // Flag-neutral staging between the `test` and the `cmov` is
                // part of the same idiom: the select lowering stages the
                // false value into the destination (`movq %r9, %rbp`) and the
                // allocator is free to place it after the test.  Such lines
                // stay in the output; they only have to avoid the flags and
                // the registers we are about to delete.
                if infos[n].reg_refs & (bool_mask | widened_mask) != 0
                    || flags_effect(tn) != FlagsEffect::Neutral
                {
                    break;
                }
                continue;
            }
            // Any other line: it must neither touch the boolean (or its
            // widened copy) nor the flags.
            if infos[n].reg_refs & (bool_mask | widened_mask) != 0
                || flags_effect(tn) != FlagsEffect::Neutral
            {
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
        // The cmov must not read the boolean or ANY of its widened copies (it
        // would lose a definition we are deleting).
        if infos[cmov_i].reg_refs & (bool_mask | widened_mask) != 0 {
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
        // Both registers die with the idiom: the boolean itself, and the
        // widened copy the `test`/`cmov` were reading.  A widened copy that is
        // live elsewhere keeps its definition (the transform is not
        // applicable), and the boolean likewise.
        let bool_dead = matches!(lv.live_after(cmov_i, bool_fam), Some(false))
            || dead_in_block_after(store, infos, cmov_i + 1, bool_fam)
            || family_private_to(store, infos, i, bool_fam, &owned);
        // Every widened copy the rewrite deletes needs its own deadness proof:
        // one live copy is enough to make the deletion observable.
        let mut widened_dead = true;
        for fam in 0..=REG_GP_MAX {
            if widened_mask & (1u16 << fam) == 0 || fam == bool_fam {
                continue;
            }
            let dead = matches!(lv.live_after(cmov_i, fam), Some(false))
                || dead_in_block_after(store, infos, cmov_i + 1, fam)
                || family_private_to(store, infos, i, fam, &owned);
            if !dead {
                widened_dead = false;
                break;
            }
        }
        if !bool_dead || !widened_dead {
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
        // The byte form is flag-exact unless the mask carries bit 7, which is
        // the only bit whose test result lands in SF at byte width but not at
        // dword width (see `flags_reach_an_sf_consumer`). Without bit 7 there is
        // nothing to prove and no consumer walk to pay for; with it, an SF
        // consumer anywhere in the flags' reach -- including one on the taken
        // edge of a conditional jump, or a `pushf` that captures SF as data --
        // refuses the narrow form and keeps the wide one.
        let byte_form = (0..=255).contains(&imm)
            && src_fam != 4
            && src_fam != 5
            && (imm & 0x80 == 0 || !flags_reach_an_sf_consumer(store, infos, j + 1));
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

// ── 5. redundant self-test after a logical or arithmetic op ──────────────────

/// How a flag producer's EFLAGS compare with a `test` of its own result.
///
/// The distinction is the whole legality argument of the fold below, so it is
/// named rather than spelled as a pair of booleans at the use site.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TestFlagAgreement {
    /// CF and OF too: `and`/`or`/`xor` clear both, exactly like `test`.
    Exact,
    /// ZF alone (`sub`, `add`, `inc`, `dec`, `neg`): the arithmetic forms take
    /// CF from the carry/borrow and OF from the overflow, where `test` clears
    /// both, and they define AF from the borrow where `test` leaves it. SF and
    /// PF agree at equal widths only, which the width rule below covers.
    ZeroOnly,
}

/// Classify a producer line for [`eliminate_redundant_self_test`]: its operand
/// width, and how closely its flags match a `test` of its own result. `None`
/// for anything that is not a producer this fold may use — including a byte
/// form (`addb`, `negb`), which writes only the low byte.
///
/// The list is deliberately exhaustive over the forms whose flag semantics are
/// flag-for-flag known; an unrecognised producer falls back to "no fold", the
/// same default-deny the rest of this module uses.
fn test_flag_agreement(text: &str) -> Option<(bool, TestFlagAgreement)> {
    const LOGICAL: &[(&str, bool)] = &[
        ("andq ", true),
        ("orq ", true),
        ("xorq ", true),
        ("andl ", false),
        ("orl ", false),
        ("xorl ", false),
    ];
    const ARITHMETIC: &[(&str, bool)] = &[
        ("addq ", true),
        ("subq ", true),
        ("incq ", true),
        ("decq ", true),
        ("negq ", true),
        ("addl ", false),
        ("subl ", false),
        ("incl ", false),
        ("decl ", false),
        ("negl ", false),
    ];
    if let Some((_, wide)) = LOGICAL.iter().find(|(p, _)| text.starts_with(p)) {
        return Some((*wide, TestFlagAgreement::Exact));
    }
    ARITHMETIC
        .iter()
        .find(|(p, _)| text.starts_with(p))
        .map(|(_, wide)| (*wide, TestFlagAgreement::ZeroOnly))
}

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
///
/// Arithmetic producer: `subl $1, %esi; testl %esi, %esi; je` is the same one
/// instruction of pure overhead — the decrement already set ZF — and it is the
/// shape every downward counter loop compiles to (`while (--n)`,
/// `for (; n--;)`, `while (n -= step)`), so it is worth the stricter proof. The
/// producer agrees with the test on ZF/SF/PF only, so the fold is taken
/// exclusively when [`flag_consumers_are_zf_only`] proves that every consumer
/// of the flags reads ZF; see [`TestFlagAgreement::ZeroOnly`].
#[expect(clippy::needless_range_loop)]
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
        let Some((prod_wide, agreement)) = test_flag_agreement(ptext) else {
            j += 1;
            continue;
        };
        if get_dest_reg(&infos[pidx]) != fam {
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
        // and the register alone. (Labels are barriers, so the scan above
        // already guarantees this run is a straight-line window with no other
        // entry point onto the test.)
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
        // What separates the two producer classes is which flags the test may
        // be replaced in: `and`/`or`/`xor` clear CF and OF exactly as the test
        // does, while an arithmetic producer takes both from the operation, so
        // it needs the flags to be read as ZF alone -- the same proof a width
        // change needs, because a 32-bit producer zero-extends (ZF survives
        // into the wide test) while SF does not (bit 31 vs bit 63).
        //
        // `flag_consumers_are_zf_only` is the whole proof, including the
        // whole-EFLAGS readers: `lahf`, `pushf`, inline asm and unknown
        // mnemonics have no condition code, so the walk charges them with
        // `saw_non_zf` before either caller looks. That matters here because
        // they are exactly the readers that can observe AF, which an
        // arithmetic producer defines from the borrow and the test does not
        // touch at all. The walk follows the TAKEN edge of every conditional
        // jump, so a consumer in a block the fall-through never reaches still
        // counts, and it reports `proved = false` rather than guessing when a
        // target cannot be resolved.
        if agreement == TestFlagAgreement::ZeroOnly || prod_wide != test_wide {
            if !flag_consumers_are_zf_only(store, infos, j + 1) {
                j += 1;
                continue;
            }
            if prod_wide != test_wide {
                // Losing SF is a whole-function property, not a property of
                // this straight-line run: verify the convention that flags do
                // not cross a block boundary before relying on the consumer
                // walk's reachability.
                let Some((fstart, fend)) = function_range(store, infos, j) else {
                    j += 1;
                    continue;
                };
                if !flags_are_block_local(store, infos, fstart, fend) {
                    j += 1;
                    continue;
                }
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
#[expect(clippy::needless_range_loop)]
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

/// Condition-code predicates that read SF: `s`/`ns` test it directly, and the
/// signed relations test it in combination with OF (`l`/`nl`/`le`/`nle` are
/// SF != OF, `g`/`ng`/`ge`/`nge` are SF == OF). The unsigned relations
/// (`b`/`ae`/`a`/`be`), the CF predicates, `o`/`no` and `p`/`np` do not.
const SF_CCS: &[&str] = &["s", "ns", "l", "nl", "le", "nle", "g", "ng", "ge", "nge"];

/// Condition codes that select on ZF and CF alone, with every other flag
/// masked out: `e/z/ne/nz` (ZF), `b/c/nae` (CF), `nb/nc/ae` (!CF) and
/// `be/na` (CF|ZF), `a/nbe` (!CF|!ZF).  The SF/OF group (`s`, `l`, `le`, `g`,
/// `ge` and their `n` forms) is deliberately absent, as is every
/// whole-word/unknown reader.  A rewrite that changes SF, PF, OF or AF -- such
/// as narrowing a `cmp` to its operands' common width -- may only proceed
/// while [`flag_consumers_are_zf_cf_only`] holds.
const ZF_CF_ONLY_CCS: &[&str] = &[
    "e", "z", "ne", "nz", "b", "c", "nae", "nb", "nc", "ae", "be", "na", "a", "nbe",
];

/// Flag readers that provably do NOT read SF, so a rewrite whose only flag
/// divergence is SF cannot be observed through them: the CF-carry group
/// (`adc`/`sbb` and the two rotates that take CF as carry-in, `cmc`, `salc`,
/// the ADX twins `adcx` -- covered by the `adc` prefix -- and `adox`, which is
/// listed on its own because no other entry prefixes it) and `into` (OF). Each
/// was checked on silicon
/// with the f10 flag sweep rather than read off a mnemonic table. Everything
/// else `flags_effect` reports as a reader and this list does not name --
/// `pushf*` and `lahf` (whole-flag forms that capture SF), `syscall`/`sysenter`
/// (RFLAGS goes to R11), `int*` (RFLAGS goes on the handler stack), and any
/// mnemonic the tables do not recognise -- counts as reading SF. Default-deny:
/// an unknown reader is never assumed benign.
const NON_SF_FLAG_READERS: &[&str] = &["adc", "adox", "sbb", "rcl", "rcr", "cmc", "salc", "into"];

/// True when a flag reader also overwrites the flags, so the flags being
/// tracked die at that line: `adc`/`sbb`/`rcl`/`rcr`/`cmc` write the arithmetic
/// flags back, `syscall`/`sysenter` mask RFLAGS after saving it to R11, and
/// `int*` hands RFLAGS to a handler that may rewrite it. `lahf`, `pushf*`,
/// `salc` and `into` only read, so the walk continues past them. An
/// unrecognised reader also ends the path: it has already been counted as an SF
/// reader, so nothing downstream of that guess could license anything anyway.
fn reader_also_writes_flags(t: &str) -> bool {
    if !(t.starts_with("adc")
        || t.starts_with("sbb")
        || t.starts_with("rcl")
        || t.starts_with("rcr")
        || t.starts_with("cmc")
        || t.starts_with("syscall")
        || t.starts_with("sysenter")
        || t.starts_with("int"))
    {
        return false;
    }
    // `into` reads OF and traps; it does not write flags. (`int3`, `int $n` do
    // hand RFLAGS to a handler, which is the stronger reason to stop.)
    !t.starts_with("into")
}

/// What a flag-consumer walk established about the flags live at its start.
#[derive(Clone, Copy)]
pub(super) struct ConsumerFacts {
    /// At least one consumer was reached.
    pub(super) saw_consumer: bool,
    /// A consumer that reads something other than ZF was reached.
    pub(super) saw_non_zf: bool,
    /// A consumer that reads a flag outside {ZF, CF} was reached.  Weaker than
    /// [`Self::saw_non_zf`]: a `jb`/`jbe` reads CF (and ZF) but nothing else,
    /// so a rewrite that provably preserves ZF and CF alone may keep it.  A
    /// consumer with no condition code is always charged here, because a
    /// whole-word or unknown reader can select neither condition exclusively
    /// and can observe AF, which no width-narrowing `cmp` preserves.
    pub(super) saw_outside_zf_cf: bool,
    /// A consumer that reads SF -- or the whole EFLAGS word, which contains it
    /// -- was reached.
    pub(super) saw_sf_reader: bool,
    /// A consumer with no condition code -- `lahf`, `pushf`, inline asm, any
    /// unknown mnemonic -- was reached.  This is exactly the class that can
    /// observe **AF**, which no `jcc`/`setcc`/`cmovcc` predicate can select
    /// and which the `NON_SF_FLAG_READERS` whitelist (`adc`/`sbb`/`adox`/...)
    /// provably does not read.  `test`-vs-`cmp` rewrites diverge on AF (a
    /// `cmp` defines it, `test` leaves it), so folds that are not
    /// flag-for-flag identical must veto on this fact.
    pub(super) saw_whole_reader: bool,
    /// False when the walk had to give up: an indirect branch, a target it
    /// could not resolve, or a branch leaving the function. The flags may then
    /// reach consumers nobody looked at, so the facts are a lower bound and
    /// must not license a flag-divergent rewrite on their own.
    pub(super) proved: bool,
}

/// Forward reachability over the flag flow that starts at `from`.
///
/// A linear text walk is not enough, and this is not a theoretical objection.
/// Flags travel along the TAKEN edge of every conditional jump as well as down
/// the fall-through, so a consumer can sit in a block the text walk never
/// enters, and stopping at the first flag writer is unsound for exactly that
/// reason: in `andl $128,%esi; je .L1; addl $1,%edi; .L1: js .L2` the `addl`
/// kills the flags on the fall-through path only, while the taken edge still
/// delivers them to the `js` that reads SF. So this walk follows direct branch
/// targets in addition to fall-through, ends each path at the first line that
/// writes the flags it is tracking (a writer, a call, a `ret`), and reports
/// what it could not resolve in `proved` instead of guessing.
///
/// Merging paths are handled by the visited set: a line is expanded once, and
/// every fact found there is shared by all paths that reach it. That
/// over-approximates which flags a consumer sees -- at a join it may attribute
/// another predecessor's flags to this walk -- and over-approximation here only
/// ever reports MORE consumers, which fails closed.
/// `preserved` is the set of flags the rewrite under test PROVES it leaves
/// unchanged.  A writer that clobbers none of the other flags cannot expose
/// the rewrite, so the walk steps over it instead of stopping -- which is what
/// keeps `stc` from hiding the `jo` behind it -- and, symmetrically, it is why
/// a PARTIAL writer never ends the walk: `clc` redefines CF and nothing else,
/// so a `setne` (ZF) or `setl` (SF/OF) behind it still reads the flags the
/// rewrite may have changed.  Ending the walk there would be unsound, which is
/// precisely what a caller passing `preserved = 0` ("the whole flag word is in
/// question") must not get: with every flag in question, only a writer that
/// redefines ALL SIX flags -- `cmp`, `test`, `add`, `popfq`, ... -- ends a
/// path; a call and a `ret` end it too (the flags are dead across the ABI
/// boundary and past the return), and an unresolvable target sets `proved` to
/// false instead of guessing.
pub(super) fn walk_flag_consumers(
    store: &LineStore,
    infos: &[LineInfo],
    from: usize,
    preserved: u8,
) -> ConsumerFacts {
    walk_flag_consumers_seeded(store, infos, from, &[from], preserved)
}

/// [`walk_flag_consumers`] with an explicit entry set.
///
/// A conditional jump has TWO successors, and flags travel along both; a
/// caller that is asking about the flags *after* a branch must therefore seed
/// the walk with the fall-through and the resolved target, not with the jump
/// itself (a `jcc` is itself a consumer, so starting there would report the
/// branch under test as its own hazard).  `anchor` locates the enclosing
/// function for the label table and bounds, and is where the facts are
/// reported from; `seeds` may be empty (no path to explore: nothing can
/// observe the flags).
/// Directives that emit DATA.  A `.byte`/`.long`/`.quad` sequence inside a
/// function body can encode any instruction -- `adc`, `clc`, a branch -- that
/// the textual predicates above cannot see, exactly like inline asm.  The
/// walker must therefore NOT step over them as if they had no effect.  The
/// benign directives (`.cfi_*`, alignment, location, section, symbol
/// bookkeeping) cannot encode an instruction and are skipped; anything the
/// classification does not name is treated as data, because "unrecognized" is
/// not "harmless".
fn directive_emits_data(text: &str) -> bool {
    let Some(rest) = text.strip_prefix('.') else {
        return false;
    };
    let name = rest
        .split(|c: char| c.is_whitespace() || c == ',')
        .next()
        .unwrap_or("");
    if matches!(
        name,
        // Benign: position, bookkeeping, sections, debug info, symbol attrs.
        "cfi_startproc"
            | "cfi_endproc"
            | "cfi_def_cfa"
            | "cfi_def_cfa_offset"
            | "cfi_def_cfa_register"
            | "cfi_offset"
            | "cfi_restore"
            | "cfi_same_value"
            | "cfi_remember_state"
            | "cfi_restore_state"
            | "cfi_return_column"
            | "cfi_undefined"
            | "cfi_personality"
            | "cfi_lsda"
            | "cfi_escape"
            | "p2align"
            | "p2alignw"
            | "p2alignl"
            | "align"
            | "balign"
            | "balignw"
            | "balignl"
            | "loc"
            | "file"
            | "text"
            | "data"
            | "bss"
            | "section"
            | "rodata"
            | "globl"
            | "global"
            | "weak"
            | "local"
            | "type"
            | "size"
            | "ident"
            | "set"
            | "equ"
            | "end"
            | "intel_syntax"
            | "att_syntax"
            | "code64"
            | "code32"
            | "code16"
            | "arch"
            | "version"
            | "skip"
            | "space"
            | "fill"
            | "zero"
            | "comm"
            | "lcomm"
            | "pushsection"
            | "popsection"
            | "previous"
            | "subsection"
    ) {
        return false;
    }
    // `.byte`, `.2byte`/`.word`, `.4byte`/`.long`, `.8byte`/`.quad`,
    // `.value`, `.inst`, `.insn`, `.ascii`, `.asciz`, `.string`, `.uleb128`,
    // `.sleb128`, `.float`, `.double` -- and anything unrecognized.
    let _ = name;
    true
}

pub(super) fn walk_flag_consumers_seeded(
    store: &LineStore,
    infos: &[LineInfo],
    anchor: usize,
    seeds: &[usize],
    preserved: u8,
) -> ConsumerFacts {
    let mut facts = ConsumerFacts {
        saw_consumer: false,
        saw_non_zf: false,
        saw_outside_zf_cf: false,
        saw_sf_reader: false,
        saw_whole_reader: false,
        proved: true,
    };
    // Label table for the enclosing function, so a branch target can be turned
    // into a line index. A target outside the function is a tail branch: the
    // flags leave with it and nothing here can account for them.
    let Some((fs, fe)) = function_range(store, infos, anchor) else {
        facts.proved = false;
        return facts;
    };
    let mut labels: Vec<(&str, usize)> = Vec::new();
    for n in fs..fe {
        if infos[n].kind == LineKind::Label {
            let t = infos[n].trimmed(store.get(n));
            if let Some(name) = t.strip_suffix(':').map(str::trim) {
                labels.push((name, n));
            }
        }
    }
    let resolve = |t: &str, facts: &mut ConsumerFacts| -> Option<usize> {
        let Some(target) = super::helpers::extract_jump_target(t) else {
            facts.proved = false;
            return None;
        };
        match labels.iter().find(|(name, _)| *name == target) {
            Some((_, idx)) => Some(*idx),
            None => {
                facts.proved = false;
                None
            }
        }
    };

    let mut seen = vec![false; fe - fs];
    let mut work: Vec<usize> = seeds.to_vec();
    while let Some(head) = work.pop() {
        let mut n = head;
        while n < fe {
            if seen[n - fs] {
                break; // this suffix was already expanded, facts included
            }
            seen[n - fs] = true;
            if infos[n].is_nop() {
                n += 1;
                continue;
            }
            let t = infos[n].trimmed(store.get(n));
            if infos[n].kind == LineKind::Directive {
                // A benign directive is a position, not an effect.  A DATA
                // directive is an instruction the text does not spell out, so
                // it is charged like inline asm: a reader of every flag, and
                // a reason the walk cannot be proved.  (No lccc-emitted
                // function body contains a data directive -- measured over the
                // whole benchmark corpus -- so this costs nothing on real
                // output and closes the one place the walker trusted
                // unrecognized text.)
                if directive_emits_data(t) {
                    facts.saw_consumer = true;
                    facts.saw_non_zf = true;
                    facts.saw_outside_zf_cf = true;
                    facts.saw_sf_reader = true;
                    facts.saw_whole_reader = true;
                    facts.proved = false;
                    break;
                }
                n += 1;
                continue;
            }
            // A blank line has no mnemonic, so `flags_effect` falls through to
            // its fail-closed default and charges it with reading every flag --
            // which made any walk that reached the end of a function (the
            // padding after the last `ret`) report a whole-word reader and veto
            // every flag-divergent rewrite.  Emitting nothing, a blank line
            // cannot observe EFLAGS, so skipping it is strictly more precise
            // and cannot license anything the writer before it did not already
            // license.
            if t.is_empty() {
                n += 1;
                continue;
            }
            match infos[n].kind {
                // A label is a position, not an effect: fall through it.
                LineKind::Label => {
                    n += 1;
                    continue;
                }
                // Inline asm can read or write any flag through raw encodings
                // the textual predicates cannot see (`.byte $0x83, $0xd0, $0x00`
                // is an `adc`), so it is charged with every reader fact AND
                // makes the walk unproved: neither the flags before it nor the
                // flags after it can be reasoned about from the text.
                LineKind::InlineAsm => {
                    facts.saw_consumer = true;
                    facts.saw_non_zf = true;
                    facts.saw_outside_zf_cf = true;
                    facts.saw_sf_reader = true;
                    facts.saw_whole_reader = true;
                    facts.proved = false;
                    break;
                }
                // `ret` ends the flags' lifetime and a call leaves EFLAGS
                // undefined, so this path stops either way.
                LineKind::Ret | LineKind::Call => break,
                // Targets live in jump-table data or a register: unknowable
                // here, and saying so is what keeps the walk honest.
                LineKind::JmpIndirect => {
                    facts.proved = false;
                    break;
                }
                // An unconditional jump has no fall-through; the flags go to
                // the target only.
                LineKind::Jmp => {
                    if let Some(idx) = resolve(t, &mut facts) {
                        work.push(idx);
                    }
                    break;
                }
                // A real register push/pop is a stack adjustment and leaves
                // EFLAGS alone. `pushf*` shares `LineKind::Push` (with
                // `REG_NONE`) purely for %rsp tracking, yet it READS every
                // flag, and `popf*` replaces them: the kind alone says nothing
                // about EFLAGS, so only the register forms are skipped and the
                // flag forms fall through to `flags_effect`, which reports
                // pushf as a reader and popf as a writer.
                LineKind::Push { reg } | LineKind::Pop { reg } if reg != REG_NONE => {
                    n += 1;
                    continue;
                }
                _ => {}
            }
            match flags_effect(t) {
                // The flags die here on this path -- UNLESS this writer only
                // clobbers flags the rewrite already proves it preserves.  A
                // CF-only writer cannot reveal a ZF/CF-preserving rewrite, and
                // stopping there would hide every consumer behind it.
                FlagsEffect::Writes => {
                    let changed = F_ALL & !preserved;
                    // Every flag in question redefined -> the walk is over on
                    // this path.  Anything less (a partial writer, an
                    // unknown-effect line that `flags_written_mask` charges
                    // with F_ALL is fine) keeps the walk going: the surviving
                    // flags still hold the values under test.
                    if changed == 0 || flags_written_mask(t) & changed == changed {
                        break;
                    }
                    n += 1;
                    continue;
                }
                FlagsEffect::Neutral => {
                    n += 1;
                    continue;
                }
                FlagsEffect::Reads => {
                    facts.saw_consumer = true;
                    // Condition-code consumers (`jcc`, `setcc`, `cmovcc`) name
                    // exactly which flags they read; everything else is a
                    // whole-flag or unknown reader and is charged with SF
                    // unless `NON_SF_FLAG_READERS` vouches for it.
                    let cc = condition_code_of(t);
                    match cc {
                        Some(cc) => {
                            if !matches!(cc, "e" | "z" | "ne" | "nz") {
                                facts.saw_non_zf = true;
                            }
                            if !ZF_CF_ONLY_CCS.contains(&cc) {
                                facts.saw_outside_zf_cf = true;
                            }
                            if SF_CCS.contains(&cc) {
                                facts.saw_sf_reader = true;
                            }
                        }
                        None => {
                            facts.saw_non_zf = true;
                            facts.saw_outside_zf_cf = true;
                            if !NON_SF_FLAG_READERS.iter().any(|p| t.starts_with(p)) {
                                facts.saw_sf_reader = true;
                                facts.saw_whole_reader = true;
                            }
                        }
                    }
                    // A conditional jump is a consumer AND an edge: the same
                    // flags are live at its target.
                    if infos[n].kind == LineKind::CondJmp
                        || (t.starts_with('j') && !t.starts_with("jmp"))
                    {
                        if let Some(idx) = resolve(t, &mut facts) {
                            work.push(idx);
                        }
                    }
                    if reader_also_writes_flags(t) {
                        break;
                    }
                    n += 1;
                }
            }
        }
    }
    facts
}

/// The flags **after a conditional jump**: does any consumer reachable from
/// either successor observe them before a writer redefines them?
///
/// This is the question a compare/branch fusion must answer before it deletes
/// the `test` that used to set those flags, and it is why the walk is seeded
/// with BOTH successors: the taken edge carries the flags to the target block,
/// where no linear text scan after the jump can ever look.  `preserved = 0`
/// because the deleted `test` and the surviving producer comparison disagree
/// about every one of the six arithmetic flags (`test %r,%r` clears OF/CF/AF
/// and sets SF/ZF/PF from the value; a `cmp` sets all six from the
/// subtraction), so no flag is safe to ignore.
///
/// Returns `None` when the jump's target cannot be resolved from the text
/// (external symbol, computed target, no label in this function): the caller
/// must treat that as a hazard rather than a proof.
pub(super) fn flags_reach_consumer_after_branch(
    store: &LineStore,
    infos: &[LineInfo],
    jcc_idx: usize,
) -> Option<ConsumerFacts> {
    let text = infos[jcc_idx].trimmed(store.get(jcc_idx));
    let target = super::helpers::extract_jump_target(text)?;
    let (fs, fe) = function_range(store, infos, jcc_idx)?;
    let mut target_idx = None;
    for n in fs..fe {
        if infos[n].kind != LineKind::Label {
            continue;
        }
        if let Some(name) = infos[n]
            .trimmed(store.get(n))
            .strip_suffix(':')
            .map(str::trim)
        {
            if name == target {
                target_idx = Some(n);
                break;
            }
        }
    }
    let target_idx = target_idx?;
    Some(walk_flag_consumers_seeded(
        store,
        infos,
        jcc_idx,
        &[jcc_idx + 1, target_idx],
        0,
    ))
}

/// The condition code of a `jcc`/`setcc`/`cmovcc` line, with any size suffix
/// stripped (`cmovlq` -> `l`), or `None` for a reader that is not a
/// condition-code form. Condition-code names collide with size suffixes, so
/// `setl` must not be stripped to `set` and lose its predicate.
fn condition_code_of(t: &str) -> Option<&str> {
    if let Some(rest) = t.strip_prefix("set") {
        return Some(rest.split_whitespace().next().unwrap_or(""));
    }
    if let Some(rest) = t.strip_prefix("cmov") {
        let tag = rest.split_whitespace().next().unwrap_or("");
        return Some(
            tag.strip_suffix(|c| c == 'q' || c == 'l' || c == 'w')
                .unwrap_or(tag),
        );
    }
    if t.starts_with('j') && !t.starts_with("jmp") {
        return Some(&t[1..t.find(' ').unwrap_or(t.len())]);
    }
    None
}

/// True when every consumer of the current flags only tests ZF (`e`/`ne`) and
/// at least one consumer exists. False also covers "could not prove", which is
/// the point: an incomplete walk must not license anything.
pub(super) fn flag_consumers_are_zf_only(
    store: &LineStore,
    infos: &[LineInfo],
    from: usize,
) -> bool {
    let f = walk_flag_consumers(store, infos, from, 0);
    f.proved && f.saw_consumer && !f.saw_non_zf
}

/// True when every consumer of the current flags selects only on ZF and CF
/// (see [`ZF_CF_ONLY_CCS`]) and at least one consumer exists.  False also
/// covers "could not prove", for the same reason [`flag_consumers_are_zf_only`]
/// fails closed: an incomplete walk must not license a flag-divergent rewrite.
pub(super) fn flag_consumers_are_zf_cf_only(
    store: &LineStore,
    infos: &[LineInfo],
    from: usize,
) -> bool {
    let f = walk_flag_consumers(store, infos, from, 0);
    f.proved && f.saw_consumer && !f.saw_outside_zf_cf
}

/// [`flag_consumers_are_zf_cf_only`] for a rewrite that PROVES it preserves
/// ZF and CF, so a writer touching only those two cannot expose it.
///
/// This is strictly more precise, never less: the only difference is that a
/// CF-only writer (`stc`, `clc`, `cmc`) or an OF-preserving one (`sahf`) no
/// longer terminates the walk, so consumers standing behind it are examined
/// instead of skipped.  In `cmp; jb; ...; stc; jo` the old code stopped at
/// `stc` and never saw the `jo`; here the `jo` is reached and vetoes the
/// rewrite, which is the correct answer because the rewrite does change OF.
pub(super) fn flag_consumers_are_zf_cf_only_preserving_zf_cf(
    store: &LineStore,
    infos: &[LineInfo],
    from: usize,
) -> bool {
    let f = walk_flag_consumers(store, infos, from, F_ZF | F_CF);
    f.proved && f.saw_consumer && !f.saw_outside_zf_cf
}

/// True when some consumer of the current flags reads SF, or when the walk
/// could not prove that none does.
///
/// This is the guard a rewrite needs when SF is the ONLY flag it can change --
/// which is narrower than "every consumer is ZF-only" and therefore keeps folds
/// that a ZF-only test refuses: a `jc`, an `jo`, an `adc` or a `jp` downstream
/// cannot observe an SF-only divergence. Both rewrites that use it diverge in
/// SF alone, and both divergences are provable rather than empirical:
///
/// * `testb $imm, %Xb` for `andl $imm, %X` (mask <= 255): ZF, PF, CF and OF are
///   identical, and SF is bit 7 of `imm & X` against bit 31 of a result that
///   cannot exceed 255 -- so it differs only when the mask has bit 7;
/// * `cmpb $0, mem` for a zero-extended load plus `test`: ZF, PF, CF and OF are
///   identical, and SF is bit 7 of the byte against bit 63 of the extended
///   register, which is 0.
pub(super) fn flags_reach_an_sf_consumer(
    store: &LineStore,
    infos: &[LineInfo],
    from: usize,
) -> bool {
    let f = walk_flag_consumers(store, infos, from, 0);
    !f.proved || f.saw_sf_reader
}

/// True when some consumer of the current flags reads the WHOLE EFLAGS word
/// (or is opaque enough that it might): `lahf`, `pushf`, inline-asm blocks,
/// unknown mnemonics.  This is the reader class that can observe **AF**;
/// condition-code consumers cannot select AF and the
/// [`NON_SF_FLAG_READERS`] whitelist (`adc`/`sbb`/`adox`/`rcl`/`rcr`/`cmc`/
/// `salc`/`into`) reads CF/OF only.  Fails closed exactly like
/// [`flags_reach_an_sf_consumer`]: an incomplete walk must not license an
/// AF-divergent rewrite.
///
/// Used by `test`→`cmp` zero-compare folds: `cmp $0, mem` DEFINES AF (a
/// subtraction from zero never borrows, so AF=0) while `test` leaves it
/// carrying the previous writer's value -- the only flag the rewrite can
/// change when SF/ZF/PF/CF/OF are provably identical.
pub(super) fn flags_reach_a_whole_flags_reader(
    store: &LineStore,
    infos: &[LineInfo],
    from: usize,
) -> bool {
    let f = walk_flag_consumers(store, infos, from, 0);
    !f.proved || f.saw_whole_reader
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

// ── 7. redundant compare feeding a chain of cmovs ──────────────────────────

/// `cmp A,B …(flag-preserving)… cmp A,B` → drop the **second** `cmp`.
///
/// # Why the second one is dead
///
/// `CMOVcc` READS EFLAGS but does not WRITE it. That is the whole reason this
/// transform exists: a comparison computed for one conditional move is still
/// intact for the next one, because nothing in between has disturbed it.
///
/// The backend emits one `cmp` per `cmov`. When if-conversion turns a chain of
/// branches that share a condition into a chain of `cmov`s, every `cmov` after
/// the first gets its own re-computation of a comparison the flags already
/// hold. SQLite's varint decoder is the canonical case: the two tail arms test
/// the same byte, and the emitted loop carried
///
/// ```text
///     cmpb $-128, %r10b
///     movl %r11d, %edx
///     cmovbl %r12d, %edx
///     cmpb $-128, %r10b        <-- recomputes flags nobody changed
///     movl $3, %ecx
///     movl $4, %ebx
///     cmovbl %ecx, %ebx
/// ```
///
/// which is one dead instruction on the slowest arm of the loop, in the one
/// shape the kernel spends most of its time in.
///
/// # What makes it sound
///
/// The two instructions must be *textually* identical — same mnemonic, same
/// width suffix, same operand text — so they compute identical flags, and
/// every line between them must be one of:
///
///   * a no-op, a directive, or a label-free run of flag-`Neutral` code;
///   * a flag reader that does not also write (`cmov`, `setcc`) — reading the
///     flags cannot change them, and `adc`/`sbb`/`rcl`/`rcr`, which do, are
///     excluded by [`reader_also_writes_flags`];
///   * and, crucially, none of them may WRITE a register named by either
///     operand.
///
/// That last clause is not decoration. Deleting the operand-write guards was
/// tried, and it miscompiles two corpus tests — `bb_slp_i64_to_i32_select`
/// (`gt_big[2]: got 6 want 100000`) and `array_string_init_matrix_O0`. Note
/// that a hand-written C fixture does *not* reproduce this: the register
/// allocator normally gives the two compares different registers, so the
/// texts differ and the fold declines anyway. The hazard only opens where
/// allocation reuses one register across the redefinition, and the corpus is
/// where those shapes live.
///
/// Control flow, calls and inline asm are barriers, so a label or a branch
/// target between the two ends the search. That also means the fold can never
/// reach across a basic block, where the flags' liveness this reasoning
/// depends on would have to be re-established.
///
/// # Memory operands are refused outright
///
/// If either operand names memory, two textually identical compares are *not*
/// necessarily equal: a store between them can change what the load reads,
/// and nothing in the `LineInfo` model lets us prove it did not. Rather than
/// guess, the fold declines. The real-world case this targets is a compare
/// against a register or an immediate, so nothing is lost, and a wrong fold
/// here would be a miscompile rather than a missed optimisation.
pub(super) fn fold_redundant_flags_compare(store: &LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut j = 0;
    while j < len {
        if infos[j].is_nop() || infos[j].pinned {
            j += 1;
            continue;
        }
        let t = infos[j].trimmed(store.get(j));

        // Accept only `cmp`/`test` with an explicit width suffix; the bare
        // forms are assembler defaults and are not what the backend emits.
        let Some(args) = strip_width_suffix(t, "cmp").or_else(|| strip_width_suffix(t, "test"))
        else {
            j += 1;
            continue;
        };
        let Some((a, b)) = split_two_operands(args) else {
            j += 1;
            continue;
        };
        // Refuse memory operands: see the module note above.
        if a.contains('(') || b.contains('(') || a.contains('[') || b.contains('[') {
            j += 1;
            continue;
        }
        // Aliasing operands (`test %rax,%rax`) collapse to one family, and the
        // single write check below then covers both, so no extra case is needed.
        let fam_a = register_family_fast(a);
        let fam_b = register_family_fast(b);

        // Walk forward over flag-preserving lines looking for the same compare.
        let mut k = j + 1;
        while k < len {
            if infos[k].is_nop() || infos[k].kind == LineKind::Directive {
                k += 1;
                continue;
            }
            if infos[k].pinned || infos[k].is_barrier() {
                break;
            }
            let tk = infos[k].trimmed(store.get(k));
            // Identity is tested FIRST, before the flag classification below.
            // A `cmp` is itself a flag writer, so classifying first would break
            // out of the walk on the very line this pass exists to delete.
            if tk == t {
                mark_nop(&mut infos[k]);
                changed = true;
                break;
            }
            match flags_effect(tk) {
                // Someone recomputed the flags: the compare we walked from no
                // longer describes them.
                FlagsEffect::Writes => break,
                // Reads the flags *and* writes them (adc/sbb/rcl/rcr/int).
                FlagsEffect::Reads if reader_also_writes_flags(tk) => break,
                // `flags_effect` answers "might this line touch flags?" and
                // deliberately answers `Reads` -- "assume the worst" -- for
                // every mnemonic it does not recognise. That is the correct
                // fail-CLOSED answer for its other callers, but this fold needs
                // the opposite polarity: it is about to DELETE a compare, so it
                // needs *proof* that the line left EFLAGS alone. Treating the
                // unknown-`Reads` catch-all as "preserves flags" inverted the
                // conservative default into a fail-OPEN one, and every flag
                // writer missing from the tables (adox, daa, kortest*, the x87
                // fcomi family, pcmpistri) silently re-based the
                // flags the following cmov reads. See `preserves_eflags`.
                FlagsEffect::Reads | FlagsEffect::Neutral => {
                    if !preserves_eflags(tk) {
                        break;
                    }
                }
            }
            // A redefinition of either operand register makes the second
            // compare see a different value, even though the text matches.
            //
            // `reg_refs` is the classifier's complete register bitmask for the
            // whole line (every operand, not just the destination), so it also
            // covers instructions whose single-destination model does not fit:
            // `xchgq %rcx, %rax` writes BOTH operands, and `mulxq %rsi, %rcx,
            // %rdx` writes two of its three. `writes_family` alone only sees
            // the parsed destination and let both folds delete a compare that
            // was reading a register the line had just clobbered. The mask
            // conflates reads with writes, which is the conservative direction
            // for a compare we are about to delete.
            let refs = infos[k].reg_refs;
            if fam_a != REG_NONE
                && (writes_family(&infos[k], tk, fam_a) || refs & (1u16 << fam_a) != 0)
            {
                break;
            }
            if fam_b != REG_NONE
                && fam_b != fam_a
                && (writes_family(&infos[k], tk, fam_b) || refs & (1u16 << fam_b) != 0)
            {
                break;
            }
            k += 1;
        }
        j += 1;
    }
    changed
}

/// Proof that `t` leaves EFLAGS unmodified — the question
/// `fold_redundant_flags_compare` actually needs.
///
/// `flags_effect` answers a deliberately different question ("might this line
/// touch flags?") and returns `FlagsEffect::Reads` as its catch-all, which is
/// fail-CLOSED for the transforms that consult it. A fold that *deletes* a
/// compare needs the converse: a positive proof that nothing between the two
/// compares rewrote the flags the later `cmov`/`jcc` will read. Answering that
/// with `Reads` would invert the conservative default into a fail-OPEN one, so
/// the polarity is fixed here, once, rather than at each call site.
///
/// The allowlist is the set of instructions the SDM documents as reading
/// EFLAGS without writing it, plus the families `flags_effect` has already
/// proven neutral. Everything else — including every mnemonic `flags_effect`
/// does not know — is treated as a writer and blocks the fold. That direction
/// is the safe one: a missed fold costs one instruction, a wrong one is a
/// miscompile.
#[inline]
fn preserves_eflags(t: &str) -> bool {
    // Documented pure readers of the condition codes.
    if t.starts_with("cmov")
        || t.starts_with("set")
        || t.starts_with("lahf")
        || t.starts_with("pushf")
        || t.starts_with("salc")
    {
        return true;
    }
    // `Neutral` is `flags_effect`'s *positive* verdict, reached only through
    // explicit tables (register push/pop, the BMI2 VEX shifts, `fp_data_op`).
    // `Reads` is its unknown-mnemonic catch-all and proves nothing.
    matches!(flags_effect(t), FlagsEffect::Neutral)
}

/// `"cmpq 1, %rax"` → `Some("1, %rax")`; `"cmpl $0, 8(%rbx)"` → `Some("$0, 8(%rbx)")`.
/// Returns `None` unless the mnemonic is exactly `base` plus one of the four
/// width suffixes, so `cmpps`/`cmpxchg` can never be mistaken for `cmp`.
fn strip_width_suffix<'a>(t: &'a str, base: &str) -> Option<&'a str> {
    let rest = t.strip_prefix(base)?;
    let rest = rest.strip_prefix(['b', 'w', 'l', 'q'])?;
    if !rest.starts_with(' ') {
        return None;
    }
    Some(rest.trim_start())
}

#[cfg(test)]
mod tests {
    use super::super::super::peephole_optimize;
    use super::{ConsumerFacts, FlagsEffect, flags_effect, walk_flag_consumers};
    use crate::backend::x86::codegen::peephole::types::{LineInfo, LineStore, classify_line};

    fn run(asm: &str) -> String {
        peephole_optimize(asm.to_string())
    }

    /// Register push/pop (any operand-size suffix) never touch EFLAGS per the
    /// SDM; only PUSHF/POPF do. `popq`/`pushq` — the spelling every AT&T
    /// epilogue emits — used to fall through the size-suffix-less `pop `
    /// check into the "unknown" catch-all and were treated as flag READERS,
    /// so `flags_dead_after` refused every flag-discarding fold whose
    /// producer sat before a callee-save restore.
    #[test]
    fn register_push_pop_width_suffixed_is_flag_neutral() {
        for t in [
            "popq %rbp",
            "popl %eax",
            "popw %bx",
            "pop %rax",
            "pushq %rbp",
            "pushq $16",
            "push %rax",
        ] {
            assert_eq!(
                flags_effect(t),
                FlagsEffect::Neutral,
                "{t} must be flag-neutral"
            );
        }
        // The flag stack ops keep their classifications ...
        assert_eq!(flags_effect("pushfq"), FlagsEffect::Reads);
        assert_eq!(flags_effect("popfq"), FlagsEffect::Writes);
        // ... and `popcnt` is not a stack pop.
        assert_eq!(flags_effect("popcntq %rax, %rax"), FlagsEffect::Writes);
    }

    /// End-to-end: a flag-dropping fold must now reach across a callee-save
    /// `popq` restore. `addq $1, %rax` staged through a copy folds to a lea
    /// even though the only flag event after it is the epilogue's pops.
    #[test]
    fn flag_discard_reaches_across_callee_save_pop() {
        // The sum is returned in %rax (observable), so only the staging copy
        // and the add may fold — the pipeline must not delete the value.
        let out = run(concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movq %rsi, %rbx\n",
            "    addq $16, %rbx\n",
            "    movq %rbx, %rax\n",
            "    popq %rbx\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(
            out.contains("leaq 16(%rsi), %rax"),
            "staged add must fold to a lea across the popq:\n{out}"
        );
        assert!(
            !out.contains("addq $16"),
            "the flag-writing add must be gone:\n{out}"
        );
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
        // The consumer is `jb`, a carry test: `addl` takes CF from the carry
        // while `testl` clears it, so the pair is not redundant. The ZF-only
        // twin of this shape IS folded -- see the counter tests below.
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

    /// Run the self-test pass over a one-function snippet whose body branches
    /// forward to the appended `.LBB4`.
    fn self_test_case(body: &str) -> String {
        run(&format!(
            "foo:\n.cfi_startproc\n{body}.LBB4:\n    ret\n.cfi_endproc\n"
        ))
    }

    #[test]
    fn self_test_after_counter_sub_is_removed_for_zf_consumer() {
        // `while (--chain != 0)`: the decrement already set ZF.
        let out = self_test_case(concat!(
            "    subl $1, %ebp\n",
            "    testl %ebp, %ebp\n",
            "    je .LBB4\n",
            "    ret\n",
        ));
        assert!(out.contains("subl $1, %ebp"), "{out}");
        assert!(!out.contains("testl %ebp, %ebp"), "{out}");
    }

    #[test]
    fn self_test_after_add_dec_neg_is_removed_for_zf_consumer() {
        for (prod, tst) in [
            ("addl %edx, %esi", "testl %esi, %esi"),
            ("decl %esi", "testl %esi, %esi"),
            ("negl %esi", "testl %esi, %esi"),
            ("subq $8, %rsi", "testq %rsi, %rsi"),
            // 32-bit producer zero-extends: ZF of the wide test is identical.
            ("subl $1, %esi", "testq %rsi, %rsi"),
        ] {
            let out = self_test_case(&format!("    {prod}\n    {tst}\n    jne .LBB4\n    ret\n"));
            assert!(!out.contains(tst), "`{tst}` after `{prod}` must go:\n{out}");
        }
    }

    #[test]
    fn self_test_after_sub_is_kept_for_carry_overflow_or_sign_consumers() {
        // `sub` sets CF/OF from the subtraction, and SF (at equal widths) is
        // the register's sign bit, which agrees with the test -- but the proof
        // this fold carries is "ZF alone", so every non-ZF predicate refuses.
        for jcc in [
            "jb", "ja", "jl", "jg", "jle", "jge", "js", "jo", "jbe", "jp",
        ] {
            let out = self_test_case(&format!(
                "    subl $1, %esi\n    testl %esi, %esi\n    {jcc} .LBB4\n    ret\n"
            ));
            assert!(out.contains("testl %esi, %esi"), "{jcc}:\n{out}");
        }
    }

    #[test]
    fn self_test_after_sub_is_kept_when_a_later_edge_reads_carry() {
        // The taken edge of the first branch delivers the flags to `jb`.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    subl $1, %esi\n",
            "    testl %esi, %esi\n",
            "    je .LBB5\n",
            "    ret\n",
            ".LBB5:\n",
            "    jb .LBB4\n",
            "    ret\n",
            ".LBB4:\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("testl %esi, %esi"), "{out}");
    }

    #[test]
    fn self_test_after_wide_sub_is_kept_under_a_narrow_test() {
        // `subq` can leave a non-zero upper half that `testl` ignores, so the
        // wide subtraction never licenses dropping the narrow test.
        let out = self_test_case(concat!(
            "    subq $1, %rsi\n",
            "    testl %esi, %esi\n",
            "    je .LBB4\n",
            "    ret\n",
        ));
        assert!(out.contains("testl %esi, %esi"), "{out}");
    }

    #[test]
    fn self_test_after_sub_is_kept_for_a_whole_flags_reader() {
        let out = self_test_case(concat!(
            "    subl $1, %esi\n",
            "    testl %esi, %esi\n",
            "    lahf\n",
            "    je .LBB4\n",
            "    ret\n",
        ));
        assert!(out.contains("testl %esi, %esi"), "{out}");
    }

    #[test]
    fn self_test_after_sub_is_kept_when_the_register_is_touched_between() {
        let out = self_test_case(concat!(
            "    subl $1, %esi\n",
            "    leal 1(%esi), %esi\n",
            "    testl %esi, %esi\n",
            "    je .LBB4\n",
            "    ret\n",
        ));
        assert!(out.contains("testl %esi, %esi"), "{out}");
    }

    #[test]
    fn self_test_after_sub_is_kept_when_a_branch_enters_at_the_test() {
        // A label between the producer and the test is a barrier, so the fold
        // never claims a straight-line window it does not have: the `jmp`
        // delivers flags that the decrement did not produce, and the test is
        // the only thing defining ZF for them.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    jmp .LBB5\n",
            "    subl $1, %esi\n",
            ".LBB5:\n",
            "    testl %esi, %esi\n",
            "    je .LBB4\n",
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

    /// The rotation loop of check_phi_acyclic_order.sh: a copy into an
    /// allocatable %rbp plus a register add, with the flags dead (the next
    /// flags reader is preceded by a writer).
    #[test]
    fn copy_plus_register_add_becomes_two_source_lea() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %r12, %rbp\n",
            "    addl %edi, %ebp\n",
            "    movq %rbx, %r8\n",
            "    addq %rsi, %r8\n",
            "    cmpq %rbp, %r8\n",
            "    jl .L1\n",
            ".L1:\n",
            "    movq %rsi, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("leal (%r12, %rdi), %ebp"), "{out}");
        assert!(out.contains("leaq (%rbx, %rsi), %r8"), "{out}");
        assert!(!out.contains("movq %r12, %rbp"), "{out}");
        assert!(!out.contains("addl %edi, %ebp"), "{out}");
    }

    /// Every case the register form must refuse: flags read by the next
    /// instruction, a `sub` (no negated index), a narrower copy than the add,
    /// an addend that is the destination or %rsp, a byte-register addend, and
    /// the frame setup `movq %rsp, %rbp`.
    #[test]
    fn copy_plus_register_add_refusals() {
        for (pair, why) in [
            (
                "    movl %ebx, %r8d\n    addl %ecx, %r8d\n    jc .L1\n",
                "carry read",
            ),
            ("    movl %ebx, %r8d\n    subl %ecx, %r8d\n", "sub"),
            (
                "    movl %ebx, %r8d\n    addq %rcx, %r8\n",
                "narrow copy, wide add",
            ),
            (
                "    movl %ebx, %r8d\n    addl %r8d, %r8d\n",
                "addend is the destination",
            ),
            ("    movq %rbx, %r8\n    addq %rsp, %r8\n", "rsp index"),
            ("    movl %ebx, %r8d\n    addb %cl, %r8b\n", "byte add"),
            ("    movq %rsp, %rbp\n    addq %rcx, %rbp\n", "frame setup"),
        ] {
            let asm = format!(
                "foo:\n.cfi_startproc\n{pair}    testq %r8, %r8\n    jne .L1\n.L1:\n    ret\n.cfi_endproc\n"
            );
            let out = run(&asm);
            assert!(!out.contains("lea"), "{why}: {out}");
        }
    }

    /// The immediate form still folds a copy of %rsp (address of a stack
    /// slot) -- the register form's frame-setup refusal must not leak.
    #[test]
    fn copy_of_rsp_plus_immediate_still_becomes_lea() {
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movq %rsp, %rax\n",
            "    addq $8, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(out.contains("leaq 8(%rsp), %rax"), "{out}");
    }

    #[test]
    fn copy_plus_add_keeps_flags_when_read() {
        // The hazard pinned here is the flag-free `leal`: with `je` reading
        // the flags, copy+add must never become `leal 1(%ebx), %r8d`. Two
        // spellings satisfy that — the pair kept verbatim, or the copy
        // coalesced into a retargeted `addl` (which computes IDENTICAL flags
        // from the same value, so `je` still observes them). Pin the
        // invariant, not the spelling.
        let out = run(concat!(
            "foo:\n",
            ".cfi_startproc\n",
            "    movl %ebx, %r8d\n",
            "    addl $1, %r8d\n",
            "    je .LBB9\n",
            "    ret\n",
            ".cfi_endproc\n",
        ));
        assert!(!out.contains("leal"), "{out}");
        assert!(
            out.lines().any(|l| {
                let l = l.trim();
                l.starts_with("addl $1, ") && (l.ends_with("%r8d") || l.ends_with("%ebx"))
            }),
            "{out}"
        );
        assert!(out.contains("je .LBB9"), "{out}");
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
    fn copy_and_mask_refuses_byte_form_when_pushf_captures_sf_as_data() {
        // `pushfq` reads EVERY flag (RFLAGS goes to the stack) and is
        // classified `LineKind::Push { reg: REG_NONE }` for %rsp tracking. A
        // consumer scan that skips Push/Pop lines STRUCTURALLY -- before
        // consulting `flags_effect`, which classifies `pushf` as a reader --
        // never sees this all-flags consumer. It finds only the ZF-only `je`,
        // stops at the `xorl`, concludes "ZF-only" and licenses the byte form.
        // But SF of `testb $128, %bl` is bit 7 of the mask result while SF of
        // `andl $128, %esi` is bit 31 = 0, and here the pushed word is popped
        // into %r12 and RETURNED as data: the divergence escapes through a
        // register, where no flag reader in the block can reveal it. Note the
        // function IS flag-block-local, so the block-locality guard cannot
        // catch this shape -- only classifying pushf as the reader it is.
        let asm = concat!(
            ".cfi_startproc\n",
            "    movl %ebx, %esi\n",
            "    andl $128, %esi\n",
            "    pushfq\n",
            "    popq %r12\n",
            "    je .LBB5\n",
            "    xorl %eax, %eax\n",
            "    ret\n",
            ".LBB5:\n",
            "    shrb $7, %r12b\n",
            "    movzbl %r12b, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let out = peephole_optimize(asm.to_string());
        assert!(out.contains("testl $128, %ebx"), "{out}");
        assert!(!out.contains("testb $128"), "{out}");
    }

    #[test]
    fn copy_and_mask_refuses_byte_form_when_a_consumer_lives_past_the_scan_window() {
        // The linear scan stops at the first flag WRITER, but flags also flow
        // along the CONDITIONAL edge: `je .LBB5` carries the `and`'s flags
        // into .LBB5, where `js` reads SF. The scan returns at the `addl` in
        // the fall-through block having seen only the ZF-only `je`, so it
        // licenses the byte form whose SF is bit 7 of the mask result instead
        // of bit 31. No amount of care at labels fixes this -- the consumer is
        // never visited. The invariant that makes a linear scan sound is
        // flag-block-locality (every reader preceded by a writer inside its
        // own block), which `flags_are_block_local` verifies and which is
        // FALSE here: `js` follows `.LBB5` with no writer between.
        let asm = concat!(
            ".cfi_startproc\n",
            "    movl %ebx, %esi\n",
            "    andl $128, %esi\n",
            "    je .LBB5\n",
            "    addl $1, %edi\n",
            "    movl %edi, %eax\n",
            "    ret\n",
            ".LBB5:\n",
            "    js .LBB6\n",
            "    xorl %eax, %eax\n",
            "    ret\n",
            ".LBB6:\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let out = peephole_optimize(asm.to_string());
        assert!(out.contains("testl $128, %ebx"), "{out}");
        assert!(!out.contains("testb $128"), "{out}");
    }

    #[test]
    fn copy_and_mask_keeps_byte_form_for_a_sign_consumer_when_the_mask_has_no_bit7() {
        // SF is the byte form's ONLY possible divergence, and it exists only
        // when the mask has bit 7: `testb $15, %bl` puts bit 7 of the masked
        // result in SF, which is 0 for every mask below 0x80 -- exactly what
        // `andl $15, %esi` leaves in bit 31. So a sign consumer is not a reason
        // to give up the narrow encoding here, and no consumer walk is needed to
        // prove it. The older ZF-only guard refused this fold.
        let asm = concat!(
            ".cfi_startproc\n",
            "    movl %ebx, %esi\n",
            "    andl $15, %esi\n",
            "    js .LBB5\n",
            "    xorl %eax, %eax\n",
            "    ret\n",
            ".LBB5:\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let out = peephole_optimize(asm.to_string());
        assert!(out.contains("testb $15, %bl"), "{out}");
        assert!(!out.contains("andl"), "{out}");
    }

    #[test]
    fn copy_and_mask_keeps_byte_form_for_a_carry_consumer() {
        // A CF-only consumer cannot observe an SF-only divergence, and both
        // forms clear CF. Narrowing the guard from "ZF-only" to "no SF reader"
        // is what keeps this fold, at a mask that does carry bit 7.
        let asm = concat!(
            ".cfi_startproc\n",
            "    movl %ebx, %esi\n",
            "    andl $128, %esi\n",
            "    jc .LBB5\n",
            "    xorl %eax, %eax\n",
            "    ret\n",
            ".LBB5:\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let out = peephole_optimize(asm.to_string());
        assert!(out.contains("testb $128, %bl"), "{out}");
        assert!(!out.contains("andl"), "{out}");
    }

    #[test]
    fn copy_and_mask_keeps_byte_form_when_the_taken_edge_reads_no_flags() {
        // Positive control for the indirect-jump pin below: identical shape, but
        // the taken edge lands in a block that reads no flags, so the walk
        // proves SF is unconsumed and the narrow form stands.
        let asm = concat!(
            ".cfi_startproc\n",
            "    movl %ebx, %esi\n",
            "    andl $128, %esi\n",
            "    je .LBB5\n",
            "    xorl %eax, %eax\n",
            "    ret\n",
            ".LBB5:\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let out = peephole_optimize(asm.to_string());
        assert!(out.contains("testb $128, %bl"), "{out}");
        assert!(!out.contains("andl"), "{out}");
    }

    #[test]
    fn copy_and_mask_keeps_byte_form_when_the_taken_edge_is_an_unconditional_jump() {
        // Following edges must not turn into refusing everything: an
        // unconditional jump's target is resolved exactly like a conditional
        // one, and this one leads to a block that reads no flags. Measured
        // alongside this pin: a target the walk cannot resolve (an indirect
        // `jmpq *%rax`, a jump to an external symbol or to a label outside the
        // function) never reaches the byte-form decision at all, because the
        // liveness oracle the fold also requires already reports the masked
        // register as not provably dead there. `proved=false` is defence in
        // depth at this call site, and is pinned directly on the walk below.
        let asm = concat!(
            ".cfi_startproc\n",
            "    movl %ebx, %esi\n",
            "    andl $128, %esi\n",
            "    je .LBB5\n",
            "    xorl %eax, %eax\n",
            "    ret\n",
            ".LBB5:\n",
            "    jmp .LBB6\n",
            ".LBB6:\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let out = peephole_optimize(asm.to_string());
        assert!(out.contains("testb $128, %bl"), "{out}");
        assert!(!out.contains("andl"), "{out}");
    }

    #[test]
    fn copy_and_mask_refuses_byte_form_for_lahf_which_reads_sf() {
        // LAHF loads SF:ZF:0:AF:0:PF:1:CF into AH -- a whole-flag reader, and
        // one of the few that reads SF without being a condition-code form. It
        // is not in `NON_SF_FLAG_READERS`, so the walk charges it with SF.
        // (Its twin SAHF is a flag WRITER, not a reader, and notably does not
        // restore OF -- the reason it belongs in no reader table at all.)
        let asm = concat!(
            ".cfi_startproc\n",
            "    movl %ebx, %esi\n",
            "    andl $128, %esi\n",
            "    lahf\n",
            "    movzbl %ah, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let out = peephole_optimize(asm.to_string());
        assert!(out.contains("testl $128, %ebx"), "{out}");
        assert!(!out.contains("testb $128"), "{out}");
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
    fn bmi1_vex_ops_are_flag_writers_not_flag_neutral() {
        // BMI1 is VEX-encoded but WRITES EFLAGS (SDM: andn sets SF/ZF/PF
        // and clears OF/CF; blsi/blsr/blsmsk write ZF/CF/SF/OF; bextr/bzhi
        // write ZF/CF — proven on hardware 2026-09-24: `cmp; andn; je`
        // takes the branch iff andn wrote ZF, and it does).  In
        // `cmp; andn; je` the branch reads ANDN's flags, so the cmp's flags
        // are dead, not live — Neutral here hid that and pinned the missed
        // optimization as a contract.
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
                matches!(flags_effect(m), FlagsEffect::Writes),
                "{m} classified {}",
                tag(flags_effect(m))
            );
        }
        // The SSE lookalikes stay neutral (claimed by the FP table above,
        // and unreachable through the integer-suffix WRITERS stems anyway).
        for m in ["andnps %xmm1, %xmm2", "andnpd %xmm1, %xmm2"] {
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
            "cmpps $1, %xmm1, %xmm2",
            "vcmppd $2, %ymm1, %ymm2, %ymm3",
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
    fn add_folds_to_lea_across_a_bmi1_op_the_branch_reads() {
        // End-to-end: `copy + add` folds to a scaled LEA only when the add's
        // flags are dead (`flags_dead_after`).  The `andn` WRITES flags
        // (hardware-proven), so the `jne` reads ANDN's flags and the add's
        // flags are dead: the fold fires and control flow is unchanged.
        // (This test used to pin the backwards reading — that the jne reads
        // the ADD's flags "through" the andn — as a contract.)
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
            out.contains("leaq 5(%r9), %rax"),
            "the add's flags are dead across the flag-writing andn: {out}"
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

    fn build(asm: &str) -> (LineStore, Vec<LineInfo>) {
        let store = LineStore::new(asm.to_string());
        let infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        (store, infos)
    }

    /// Walk the flags produced by the line containing `marker`.
    fn facts_from(asm: &str, marker: &str) -> ConsumerFacts {
        let (store, infos) = build(asm);
        let at = (0..store.len())
            .find(|&i| infos[i].trimmed(store.get(i)).contains(marker))
            .expect("marker line not found");
        walk_flag_consumers(&store, &infos, at + 1, 0)
    }

    #[test]
    fn walk_follows_the_taken_edge_of_a_conditional_jump() {
        // The whole reason the walk is not a linear scan: the fall-through path
        // kills the flags at the `xorl`, and the SF reader only exists on the
        // edge the `je` takes.
        let f = facts_from(
            concat!(
                ".cfi_startproc\n",
                "    andl $128, %esi\n",
                "    je .LBB5\n",
                "    xorl %eax, %eax\n",
                "    ret\n",
                ".LBB5:\n",
                "    js .LBB6\n",
                ".LBB6:\n",
                "    ret\n",
                ".cfi_endproc\n",
            ),
            "andl $128",
        );
        assert!(f.proved && f.saw_consumer && f.saw_sf_reader && f.saw_non_zf);
    }

    #[test]
    fn walk_reports_unproved_when_it_cannot_resolve_a_target() {
        // An indirect jump's targets are data; so is a jump to a symbol that is
        // not a label of this function. Both must say "I do not know" rather
        // than report a clean bill of health.
        for tail in [
            "    jmpq *%rax\n",
            "    jmp ext_symbol\n",
            "    jmp .LBB9\n",
        ] {
            let asm = format!(
                ".cfi_startproc\n    andl $128, %esi\n    je .LBB5\n.LBB5:\n{tail}.cfi_endproc\n"
            );
            let f = facts_from(&asm, "andl $128");
            assert!(!f.proved, "target {tail:?} was reported as proved");
        }
    }

    #[test]
    fn walk_charges_whole_flag_readers_with_sf_but_not_carry_readers() {
        // `pushf*` and `lahf` capture SF; `adcx`/`adox`/`adc`/`sbb`/`into` read
        // CF or OF only (silicon-verified), so they must not block an SF-only
        // rewrite -- and the ones that write flags back also end the path.
        let sf = |m: &str| {
            let asm =
                format!(".cfi_startproc\n    andl $128, %esi\n    {m}\n    ret\n.cfi_endproc\n");
            facts_from(&asm, "andl $128")
        };
        for reader in ["pushfq", "pushf", "lahf", "syscall", "int3"] {
            let f = sf(reader);
            assert!(f.saw_sf_reader, "{reader} should be charged with SF");
        }
        for reader in [
            "adcxq %rdx, %rbx",
            "adoxq %rdx, %rbx",
            "adcq %rdx, %rbx",
            "sbbq %rdx, %rbx",
            "into",
        ] {
            let f = sf(reader);
            assert!(!f.saw_sf_reader, "{reader} does not read SF");
            assert!(f.saw_non_zf, "{reader} is still a non-ZF consumer");
        }
    }

    #[test]
    fn walk_stops_each_path_at_a_writer_a_call_and_a_ret() {
        let facts = |body: &str| {
            let asm = format!(".cfi_startproc\n    andl $128, %esi\n{body}.cfi_endproc\n");
            facts_from(&asm, "andl $128")
        };
        // A writer kills the flags: the `js` behind it reads the writer's flags.
        let f = facts("    addl $1, %edi\n    js .LBB5\n.LBB5:\n    ret\n");
        assert!(f.proved && !f.saw_consumer && !f.saw_sf_reader);
        // A call leaves EFLAGS undefined, so nothing behind it is a consumer.
        let f = facts("    call ext\n    js .LBB5\n.LBB5:\n    ret\n");
        assert!(f.proved && !f.saw_consumer);
        // `ret` ends the flags' lifetime at the ABI boundary.
        let f = facts("    ret\n");
        assert!(f.proved && !f.saw_consumer);
        // A real register push/pop is flag-neutral and must NOT stop the walk,
        // while `popf*` replaces the flags and must.
        let f = facts("    pushq %rax\n    popq %rax\n    js .LBB5\n.LBB5:\n    ret\n");
        assert!(
            f.proved && f.saw_sf_reader,
            "register push/pop must be transparent"
        );
        let f = facts("    popfq\n    js .LBB5\n.LBB5:\n    ret\n");
        assert!(f.proved && !f.saw_sf_reader, "popf replaces the flags");
    }

    // ==================== fold_redundant_flags_compare ====================
    //
    // This fold DELETES a compare, so every test here is a "must NOT fold"
    // unless the comment says otherwise. The input always carries exactly two
    // identical compares; a correct peephole keeps BOTH (2), and 1 means the
    // second was wrongly deleted. `reg_refs` and copy-propagation rewrite
    // register names freely, so the tests count `cmpb $-128,` rather than
    // matching a specific register.

    fn two_cmp_case(mid: &str) -> String {
        format!(
            "main:\n    .cfi_startproc\n    cmpb $-128, %rcx\n    {mid}\n    \
             cmpb $-128, %rcx\n    cmovb %rsi, %rcx\n    .cfi_endproc\n    ret\n"
        )
    }
    fn n_cmp(out: &str) -> usize {
        out.matches("cmpb $-128,").count()
    }

    /// A flag-preserving, non-clobbering line between two identical compares:
    /// the exact redundancy this pass exists to remove. If this ever stops
    /// folding, the pass has become a pessimisation and every other test below
    /// is vacuous.
    #[test]
    fn flags_compare_folds_the_redundant_compare() {
        let out = run(&two_cmp_case("movq %rsi, %rdx"));
        assert_eq!(
            n_cmp(&out),
            1,
            "movq preserves flags and writes neither operand"
        );
        // ...and the negative control still holds the fold off.  The tail
        // stores %edi so the intervening `addl` cannot be retired as a dead
        // write (`eliminate_dead_flag_writes`): what blocks the fold must be
        // the flag write, not the removal of the line.
        let kept = run(concat!(
            "main:\n    .cfi_startproc\n    cmpb $-128, %rcx\n",
            "    addl %esi, %edi\n",
            "    cmpb $-128, %rcx\n",
            "    cmovb %rsi, %rcx\n",
            "    movl %edi, (%rdx)\n",
            "    .cfi_endproc\n    ret\n",
        ));
        assert_eq!(
            n_cmp(&kept),
            2,
            "addl writes flags: both compares must survive"
        );
    }

    /// `flags_effect` answers "might this touch flags?" and returns `Reads` for
    /// anything unrecognised. Every real flag writer that lands in that
    /// catch-all must still block a fold that deletes a compare. Each of these
    /// reproduced a wrong fold before `preserves_eflags` existed.
    #[test]
    fn flags_compare_refuses_across_unmodelled_flag_writers() {
        // `adox` writes OF; its `adcx` sibling is caught only because it
        // happens to share the "adc" prefix, so both are pinned here.
        for mid in [
            "adoxq %rsi, %r8",            // OF
            "adcxq %rsi, %r8",            // CF
            "daa",                        // AF/CF
            "kortestw %ax, %bx",          // ZF/CF
            "fcomip %st(1)",              // ZF/PF/CF
            "fcomip %st(1)",              // ZF/PF/CF
            "pcmpistri $0, (%rax), %rdx", // ZF/CF/SF/OF + EAX
        ] {
            let out = run(&two_cmp_case(mid));
            assert_eq!(
                n_cmp(&out),
                2,
                "`{mid}` writes EFLAGS: 2nd compare must survive"
            );
        }
    }

    /// `xchgq %rcx, %rax` writes BOTH operands and `mulxq %rsi, %rcx, %rdx`
    /// writes two of its three, so neither fits a single-destination model.
    /// `writes_family` saw only the parsed destination and let the fold delete
    /// a compare whose operand the line had just clobbered.
    #[test]
    fn flags_compare_refuses_across_multi_destination_writes() {
        for mid in ["xchgq %rcx, %rax", "mulxq %rsi, %rcx, %rdx"] {
            let out = run(&two_cmp_case(mid));
            assert_eq!(
                n_cmp(&out),
                2,
                "`{mid}` writes %rcx: 2nd compare must survive"
            );
        }
    }

    /// Pins the SSE/AVX and MXCSR classes of the shared `flags_effect` oracle.
    ///
    /// These were re-derived from the SDM prose and got it wrong, so they are
    /// pinned to what the hardware actually does. A pushfq/popq probe around
    /// each instruction (x86-64, 2026-09-30) gives:
    ///
    ///     cmpps    unchanged      cmppd    unchanged
    ///     movaps   unchanged      stmxcsr  unchanged
    ///     comiss   WRITES EFLAGS
    ///
    /// The packed SSE compares write the per-element mask to the DESTINATION
    /// and leave EFLAGS alone; only the *scalar* COMISS/UCOMISS family sets
    /// ZF/PF/CF. LDMXCSR likewise does not write EFLAGS for any value tested,
    /// including ones with SIMD exception status bits set. The pre-existing
    /// `sse_arith_is_flag_neutral_not_a_flag_writer` test was already right and
    /// caught the wrong reclassification; this one exists so the boundary is
    /// stated in both directions rather than only on the side that was wrong.
    #[test]
    fn sse_compare_and_mxcsr_classes_match_hardware() {
        // Measured flag-neutral on this x86-64 host.
        for t in [
            "cmpps $0, %xmm0, %xmm1",
            "cmppd $0, %xmm0, %xmm1",
            "ldmxcsr (%rax)",
            "stmxcsr (%rax)",
            "movaps %xmm0, %xmm1",
        ] {
            assert_eq!(
                flags_effect(t),
                FlagsEffect::Neutral,
                "{t} leaves EFLAGS alone"
            );
        }
        // Measured to set ZF/PF/CF.
        for t in [
            "comiss %xmm0, %xmm1",
            "comisd %xmm0, %xmm1",
            "ucomiss %xmm0, %xmm1",
            "ucomisd %xmm0, %xmm1",
            "vcomiss %xmm0, %xmm1",
            "vucomisd %xmm0, %xmm1",
        ] {
            assert_eq!(flags_effect(t), FlagsEffect::Writes, "{t} sets ZF/PF/CF");
        }
        // The scalar/packed CMPSS/CMPSD spellings are unmodelled, so they land
        // on the unknown catch-all. `Reads` is fail-closed -- it can only make
        // a transform refuse a fold it could have made -- so leaving them there
        // is correct-by-default rather than a gap worth closing on a guess.
        for t in ["cmpss $0, %xmm0, %xmm1", "cmpsd $0, %xmm0, %xmm1"] {
            assert_eq!(flags_effect(t), FlagsEffect::Reads, "{t} is unmodelled");
        }
    }
}

#[cfg(test)]
mod widened_copy_tracking_tests {
    //! `setCC; movzbl %al, %rN; test %rN, %rN; cmovCC` deletes every widening
    //! copy it walks over, so it must prove every one of them dead.  The scan
    //! used to remember only the LAST destination while deleting all of them,
    //! which let a `cmov` read the earlier copy's (now deleted) definition.
    use super::super::peephole_optimize;

    fn run(asm: &str) -> String {
        peephole_optimize(asm.to_string())
    }

    fn f(body: &str) -> String {
        format!(".text\nf:\n.cfi_startproc\n{body}\n.cfi_endproc\n")
    }

    #[test]
    fn refuses_when_the_cmov_reads_an_earlier_widened_copy() {
        let out = run(&f(
            "    cmpq $0, %rdi\n    setne %al\n    movzbl %al, %r8d\n    movzbl %al, %r9d\n    testl %r9d, %r9d\n    cmovne %r8d, %ebx\n    movl %ebx, %eax\n    ret",
        ));
        assert!(
            out.contains("setne %al"),
            "boolean deleted while %%r8 was still read: {out}"
        );
        assert!(
            out.contains("movzbl %al, %r8d"),
            "%%r8 definition deleted: {out}"
        );
        // No fusion: the `test` (and therefore the whole idiom) survives, and
        // the `cmov` still reads the definition it had.  The input's own
        // `cmovne` is a SURVIVOR here -- the bug would have been deleting the
        // `%r8d` definition under it, not removing it.
        assert!(out.contains("testl %r9d, %r9d"), "fusion fired: {out}");
        assert!(
            out.contains("cmovne %r8d, %ebx"),
            "the select changed shape: {out}"
        );
    }

    #[test]
    fn still_fuses_the_single_widening_shape() {
        // Control for the test above: one widening copy, used by the `test`,
        // dead afterwards -- the fold must still fire.
        let out = run(&f(
            "    cmpq $0, %rdi\n    setne %al\n    movzbl %al, %r8d\n    testl %r8d, %r8d\n    jne .Lx\n    movl $1, %eax\n    ret\n.Lx:\n    xorl %eax, %eax\n    ret",
        ));
        assert!(!out.contains("setne"), "fold did not fire: {out}");
        assert!(out.contains("ne .Lx") || out.contains("e .Lx"), "{out}");
    }
}
