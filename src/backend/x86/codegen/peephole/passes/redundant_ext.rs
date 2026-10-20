//! Always-on, provably-safe elimination of redundant zero/sign-extension moves.
//!
//! The codegen emits a widening cast (`u8 -> u32`) as `movzbl %al, %eax` even
//! when the value is ALREADY zero-extended in that register (e.g. it was just
//! loaded with `movzbl (%mem), %eax`, produced by `setcc %al`, or copied from
//! another zero-extended register). Such a re-extension is a no-op and can be
//! removed, saving an instruction in hot loops.
//!
//! Soundness: we separately track, for each 64-bit register family, whether
//! its value is known to fit in 8, 16, or 32 bits. A byte re-extension such as
//! `movzbl %al, %eax` is removable only when the source is known to fit in 8
//! bits; merely knowing that bits 32..63 are zero is insufficient because the
//! instruction must still clear bits 8..31. The same rule applies to 16-bit
//! re-extensions. Facts are propagated by width-preserving register copies and
//! cleared by instructions that may write a wider value.

use super::super::types::*;

/// Register families we track for upper-32-zero status. Index = family id
/// (0=rax, 1=rcx, 2=rdx, 3=rbx, ... 15=r15). We only need the GP families.
const GP_FAMILIES: usize = 16;

/// Determine the register family id of a register name (e.g. "al" -> 0,
/// "eax" -> 0, "rax" -> 0, "r8b" -> 8, "r11d" -> 11).
fn family_of_reg(name: &str) -> Option<u8> {
    let n = name.trim_start_matches('%');
    match n {
        "al" | "ax" | "eax" | "rax" => Some(0),
        "cl" | "cx" | "ecx" | "rcx" => Some(1),
        "dl" | "dx" | "edx" | "rdx" => Some(2),
        "bl" | "bx" | "ebx" | "rbx" => Some(3),
        "spl" | "sp" | "esp" | "rsp" => Some(4),
        "bpl" | "bp" | "ebp" | "rbp" => Some(5),
        "sil" | "si" | "esi" | "rsi" => Some(6),
        "dil" | "di" | "edi" | "rdi" => Some(7),
        "r8b" | "r8w" | "r8d" | "r8" => Some(8),
        "r9b" | "r9w" | "r9d" | "r9" => Some(9),
        "r10b" | "r10w" | "r10d" | "r10" => Some(10),
        "r11b" | "r11w" | "r11d" | "r11" => Some(11),
        "r12b" | "r12w" | "r12d" | "r12" => Some(12),
        "r13b" | "r13w" | "r13d" | "r13" => Some(13),
        "r14b" | "r14w" | "r14d" | "r14" => Some(14),
        "r15b" | "r15w" | "r15d" | "r15" => Some(15),
        _ => None,
    }
}

/// Get the 32-bit register name for a family id (0=rax -> "eax", 8 -> "r8d").
fn reg32_name(fam: u8) -> Option<&'static str> {
    match fam {
        0 => Some("eax"),
        1 => Some("ecx"),
        2 => Some("edx"),
        3 => Some("ebx"),
        4 => Some("esp"),
        5 => Some("ebp"),
        6 => Some("esi"),
        7 => Some("edi"),
        8 => Some("r8d"),
        9 => Some("r9d"),
        10 => Some("r10d"),
        11 => Some("r11d"),
        12 => Some("r12d"),
        13 => Some("r13d"),
        14 => Some("r14d"),
        15 => Some("r15d"),
        _ => None,
    }
}

/// Get the 64-bit register name for a family id (0=rax -> "rax", 8 -> "r8").
fn reg64_name(fam: u8) -> Option<&'static str> {
    match fam {
        0 => Some("rax"),
        1 => Some("rcx"),
        2 => Some("rdx"),
        3 => Some("rbx"),
        4 => Some("rsp"),
        5 => Some("rbp"),
        6 => Some("rsi"),
        7 => Some("rdi"),
        8 => Some("r8"),
        9 => Some("r9"),
        10 => Some("r10"),
        11 => Some("r11"),
        12 => Some("r12"),
        13 => Some("r13"),
        14 => Some("r14"),
        15 => Some("r15"),
        _ => None,
    }
}

/// Get the low-8-bit register name for a family id (0 -> "al", 8 -> "r8b").
/// High-byte registers (ah/bh/ch/dh) deliberately have no entry anywhere in
/// this pass: family_of_reg returns None for them, and the sign-extension
/// consumer additionally requires the source text to equal this low-byte
/// name, so a hypothetical `movsbq %ah, ...` can never be mistaken for a
/// re-extension of the low byte.
fn reg8_name(fam: u8) -> Option<&'static str> {
    match fam {
        0 => Some("al"),
        1 => Some("cl"),
        2 => Some("dl"),
        3 => Some("bl"),
        4 => Some("spl"),
        5 => Some("bpl"),
        6 => Some("sil"),
        7 => Some("dil"),
        8 => Some("r8b"),
        9 => Some("r9b"),
        10 => Some("r10b"),
        11 => Some("r11b"),
        12 => Some("r12b"),
        13 => Some("r13b"),
        14 => Some("r14b"),
        15 => Some("r15b"),
        _ => None,
    }
}

/// Get the low-16-bit register name for a family id (0 -> "ax", 8 -> "r8w").
fn reg16_name(fam: u8) -> Option<&'static str> {
    match fam {
        0 => Some("ax"),
        1 => Some("cx"),
        2 => Some("dx"),
        3 => Some("bx"),
        4 => Some("sp"),
        5 => Some("bp"),
        6 => Some("si"),
        7 => Some("di"),
        8 => Some("r8w"),
        9 => Some("r9w"),
        10 => Some("r10w"),
        11 => Some("r11w"),
        12 => Some("r12w"),
        13 => Some("r13w"),
        14 => Some("r14w"),
        15 => Some("r15w"),
        _ => None,
    }
}

/// Parse the two operands of a two-operand instruction line ("op SRC, DST").
/// Returns (src, dst) with whitespace trimmed, or None when the line cannot
/// be split safely. The operand separator is the LAST comma that is not
/// inside parentheses (memory operands contain commas: `(%rax,%rcx,4)`).
/// Slicing is guarded the same way as the existing patterns: a comma that
/// appears before the first space must never invert the source range.
fn split_two_operands(t: &str) -> Option<(&str, &str)> {
    let mut depth: i32 = 0;
    let mut comma: Option<usize> = None;
    for (idx, ch) in t.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => comma = Some(idx),
            _ => {}
        }
    }
    let comma = comma?;
    let mut so = t.find(|c: char| c == ' ' || c == '\t')? + 1;
    if so > comma || comma >= t.len() {
        return None;
    }
    while so < comma && (t.as_bytes()[so] == b' ' || t.as_bytes()[so] == b'\t') {
        so += 1;
    }
    let src = t[so..comma].trim();
    let dst = t[comma + 1..].trim();
    if src.is_empty() || dst.is_empty() {
        return None;
    }
    Some((src, dst))
}

/// Is this a 32-bit or smaller register name (i.e. writing it zero-extends or
/// truncates the upper 32 bits)? Note: the 32-bit names are eax/ebx/ecx/edx/
/// esi/edi/ebp/esp/r8d..r15d; the 8-bit names are al/bl/cl/dl/sil/dil/bpl/spl/
/// r8b..r15b.
fn candidate_uses_32bit_dest(line: &str, dst32: &str) -> bool {
    let toks: Vec<&str> = line.split_whitespace().collect();
    if toks.is_empty() {
        return false;
    }
    let op = toks[0];
    let needle32 = if dst32.starts_with('%') {
        dst32.to_string()
    } else {
        format!("%{dst32}")
    };
    let has_32 = toks.iter().any(|tok| tok.trim_end_matches(',') == needle32);
    if !has_32 {
        return false;
    }
    // If the same instruction names the 64-bit register, it may be a true
    // 64-bit consumer despite also mentioning a 32-bit alias in an addressing
    // expression.  Leave those to IR/RA rather than guessing in text.
    let trimmed = dst32.trim_start_matches('%');
    // Never narrow a copy into %esp/%ebp.  They are not general allocable homes
    // in this backend, and later text passes may need the 64-bit frame/stack
    // copy (for example copy-shift-copyback uses %rbp as a temporary that is
    // later consumed by a 64-bit LEA).
    if matches!(trimmed, "esp" | "ebp") {
        return false;
    }
    // r8d/r15d may still be consumed by a 64-bit operation on the same line
    // through their `%r8`/`%r15` alias. Reject that conservatively.
    if trimmed.starts_with('r')
        && trimmed.ends_with('d')
        && trimmed[1..trimmed.len() - 1]
            .bytes()
            .all(|b| b.is_ascii_digit())
    {
        let d64 = format!("%{}", &trimmed[..trimmed.len() - 1]);
        if toks.iter().any(|tok| tok.trim_end_matches(',') == d64) {
            return false;
        }
    }
    let d64 = match trimmed {
        "eax" => "%rax",
        "ecx" => "%rcx",
        "edx" => "%rdx",
        "ebx" => "%rbx",
        "esi" => "%rsi",
        "edi" => "%rdi",
        other
            if other.starts_with('r')
                && other.ends_with('d')
                && other[1..other.len() - 1]
                    .bytes()
                    .all(|b| b.is_ascii_digit()) =>
        {
            ""
        }
        _ => return false,
    };
    if toks.iter().any(|tok| tok.trim_end_matches(',') == d64) {
        return false;
    }
    // 32-bit arithmetic/logical moves and branches consume/write the low 32
    // bits and zero the upper half.  Restrict to the common integer forms;
    // unusual mnemonics are deliberately left unchanged.
    matches!(
        op,
        "movl"
            | "addl"
            | "subl"
            | "andl"
            | "orl"
            | "xorl"
            | "cmpl"
            | "testl"
            | "leal"
            | "imull"
            | "sall"
            | "shll"
            | "shrl"
            | "sarl"
            | "movzbl"
            | "movzwl"
            | "cmovl"
            | "cmovel"
            | "cmovnel"
            | "cmovsl"
            | "cmovns"
            | "cmovgl"
            | "cmovgel"
            | "cmovll"
            | "cmovlel"
            | "cmovbl"
            | "cmovael"
            | "cmovbel"
            | "cmoval"
            | "popcntl"
            | "lzcntl"
            | "tzcntl"
            | "andnl"
            | "shlxl"
            | "shrxl"
            | "sarxl"
    )
}

fn is_32bit_or_smaller(name: &str) -> bool {
    let n = name.trim_start_matches('%');
    match n {
        // 32-bit GP registers
        "eax" | "ebx" | "ecx" | "edx" | "esi" | "edi" | "ebp" | "esp" => true,
        // 8-bit GP registers
        "al" | "bl" | "cl" | "dl" | "sil" | "dil" | "bpl" | "spl" => true,
        // 16-bit
        "ax" | "bx" | "cx" | "dx" | "si" | "di" | "bp" | "sp" => true,
        // r8..r15 in 8/16/32-bit forms: r8b..r15b, r8w..r15w, r8d..r15d
        _ if r_reg_width(n) != 0 => true,
        _ => false,
    }
}

/// Returns the width (1,2,4) of a r8..r15 register name like "r13d", or 0 if
/// it is not a small rN register. Handles one- and two-digit register numbers.
fn r_reg_width(n: &str) -> u8 {
    let bytes = n.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'r' {
        return 0;
    }
    // parse the numeric part before the size suffix
    let mut i = 1;
    let mut num = 0usize;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        num = num * 10 + (bytes[i] - b'0') as usize;
        i += 1;
    }
    if num < 8 || num > 15 {
        return 0;
    }
    let suffix = &n[i..];
    match suffix {
        "b" => 1,
        "w" => 2,
        "d" => 4,
        _ => 0,
    }
}

/// Eliminate redundant zero-extension moves. Returns true if any changed.
pub(super) fn eliminate_redundant_zero_extend(asm: &mut String) -> bool {
    let mut lines: Vec<String> = asm.lines().map(|s| s.to_string()).collect();
    let mut keep = vec![true; lines.len()];
    let mut changed = false;

    // upper32_zero[f] = true if register family f's 64-bit value is provably
    // zero in bits 32..63.
    let mut upper32_zero = vec![false; GP_FAMILIES];
    // is_byte[f] = true if register family f's low 32 bits are provably a byte
    // value (0..255), i.e. bits 8..31 are zero. Set by a byte zero-extension
    // into f (`movzbl %anything, %f` / `movzbq`). When set, the byte and the
    // full 32-bit value are numerically equal, so a byte re-extension into a
    // DIFFERENT register can be rewritten as a 32-bit copy.
    let mut is_byte = vec![false; GP_FAMILIES];
    // is_16[f] = true if register family f's low 32 bits are provably a
    // 16-bit value (bits 16..31 zero). Set by movzwl/movzwq (16-bit
    // zero-extension, from anywhere) and by byte zero-extensions (a byte
    // value fits in 16 bits). Lets us remove redundant `movzwq %ax, %rax`
    // re-extensions of a value that is already zero-extended to 16 bits.
    let ext16_enabled = std::env::var("CCC_NO_EXT16").is_err();
    let mut is_16 = vec![false; GP_FAMILIES];
    // sext32[f] = true if register family f's 64-bit value is provably equal
    // to sext(its own low 32 bits) -- exactly the postcondition of `movslq
    // %fd, %f`. A SECOND such instruction on the same family is then a no-op
    // and can be deleted.
    //
    // This fires on shared-index addressing, which is pervasive in real code:
    // `a[i]` and `b[i]` in one expression become two GEPs over the same
    // already-sign-extended index, and address lowering re-materialises the
    // extension for each one. Measured on the gzip longest_match compare loop
    // (tests/bench/k_matchlen.c), the inner loop emitted
    //
    //     movslq %r15d, %r15 ; movzbl (%r12,%r15), %r10d
    //     movslq %r15d, %r15 ; movzbl (%r13,%r15), %eax
    //
    // where the second movslq is dead weight in a 10-instruction loop.
    //
    // Soundness: the fact is set ONLY by movslq and propagated only by
    // register-to-register movq. Every other write to the family clears it,
    // including the conservative catch-all, and it is dropped at every label
    // because the predecessor path is unknown. Note a zero-extending write
    // (movl, movzbl, ALU) must NOT set it: those leave bits 32..63 clear, but
    // if bit 31 is set then sext(low32) has them all SET, so the values
    // differ. Only the two provable cases below establish the fact.
    let mut sext32 = vec![false; GP_FAMILIES];
    // sext8[f] / sext16[f] = family f's 64-bit value is provably equal to
    // sext(its own low 8 / low 16 bits) -- exactly the postcondition of
    // `movsbq <any>, %f` / `movswq <any>, %f`. sext8 implies sext16 implies
    // sext32 (sign-extending from a narrower width also reproduces the sign
    // bit across every wider boundary), and the producers below maintain
    // that implication. The consumer transform is the signed mirror image
    // of the is_byte rule: an I8 load in this backend is emitted as a
    // sign-extending `movsbq` into %rax, and the immediately following
    // signed widening cast emits a SECOND `movsbq %al, %rXX` -- the value
    // is already sign-extended in %rax, so the cast is a plain register
    // copy. `movq` can be move-eliminated by register renaming on modern
    // cores; `movsbq` cannot. Measured shape (base64 table decode loop,
    // tests/bench/k_base64_decode.c): four adjacent
    // `movsbq (mem),%rax` / `movsbq %al,%rNN` pairs per iteration.
    // Soundness: the facts are set ONLY by movsbq/movswq/movslq producers and
    // propagated only by register-to-register movq; every other write clears
    // them, including the conservative catch-all, labels, calls and
    // inline-asm boundaries. Kill switch: CCC_NO_SEXT (mirrors CCC_NO_EXT16).
    let sext_enabled = std::env::var("CCC_NO_SEXT").is_err();
    let mut sext8 = vec![false; GP_FAMILIES];
    let mut sext16 = vec![false; GP_FAMILIES];

    // Inline-asm regions (`#APP` .. `#NO_APP`) are user-authored text that
    // must reach the assembler byte-for-byte (kernel ALTERNATIVE() length
    // placeholders, runtime-patched jump labels).  This pass runs BEFORE
    // `pin_inline_asm_regions` (and again on the final text in phase 9,
    // after pinning has been lowered back to plain text), so it must guard
    // the region boundary itself:
    //   * lines inside a region are never matched, rewritten or deleted;
    //   * crossing the boundary clears every tracked fact — a template can
    //     redefine any register without naming it in the emitted text
    //     (`.byte` encodings, `%0`-substituted operands the pattern match
    //     never sees).
    let mut in_asm = false;

    for i in 0..lines.len() {
        let line = &lines[i];
        let t = line.trim();
        if t.is_empty() {
            continue;
        }

        if t == "#APP" {
            in_asm = true;
            upper32_zero = vec![false; GP_FAMILIES];
            is_byte = vec![false; GP_FAMILIES];
            is_16 = vec![false; GP_FAMILIES];
            sext32 = vec![false; GP_FAMILIES];
            sext8 = vec![false; GP_FAMILIES];
            sext16 = vec![false; GP_FAMILIES];
            continue;
        }
        if t == "#NO_APP" {
            in_asm = false;
            upper32_zero = vec![false; GP_FAMILIES];
            is_byte = vec![false; GP_FAMILIES];
            is_16 = vec![false; GP_FAMILIES];
            sext32 = vec![false; GP_FAMILIES];
            sext8 = vec![false; GP_FAMILIES];
            sext16 = vec![false; GP_FAMILIES];
            continue;
        }
        if in_asm {
            // Opaque user text: carry no facts in, take no facts out,
            // change no bytes.
            continue;
        }

        // Labels FIRST, before the directive skip.
        //
        // PRE-EXISTING SOUNDNESS BUG: the directive test (`starts_with('.')`)
        // used to run before the label test, so `.LBB1:` -- the form lccc
        // emits for every basic-block label -- was skipped as a directive and
        // never reached the clearing code below. Register facts therefore
        // leaked across basic-block boundaries: a block entered from several
        // predecessors inherited whatever the textually preceding block
        // happened to leave behind, and a re-extension that is genuinely
        // needed on one incoming path could be deleted. Only labels the
        // assembler writes as `foo:` (no dot) were handled correctly.
        //
        // Real directives do not end in ':', so testing for the label form
        // first is exact rather than merely conservative.
        if t.ends_with(':') {
            upper32_zero = vec![false; GP_FAMILIES];
            is_byte = vec![false; GP_FAMILIES];
            is_16 = vec![false; GP_FAMILIES];
            sext32 = vec![false; GP_FAMILIES];
            sext8 = vec![false; GP_FAMILIES];
            sext16 = vec![false; GP_FAMILIES];
            continue;
        }

        // Assembler directives (.p2align, .size, .cfi_*, .quad, ...) define no
        // registers and cannot be removed.
        if t.starts_with('.') {
            continue;
        }

        // --- Pattern: movslq %R15D, %R15 twice on the same family ---
        // The second is a no-op: after the first, the family already holds
        // sext(its low 32 bits), and nothing in between redefined it (any
        // write clears the fact). Source and destination must be the SAME
        // family -- `movslq %eax, %r15` moves a different value and must stay.
        if t.starts_with("movslq ") || t.starts_with("movsxd ") {
            if let Some(comma) = t.rfind(',') {
                let mut so = t.find(' ').unwrap_or(0) + 1;
                if so <= comma && comma < t.len() {
                    while so < comma && (t.as_bytes()[so] == b' ' || t.as_bytes()[so] == b'\t') {
                        so += 1;
                    }
                    let src = t[so..comma].trim();
                    let dst = t[comma + 1..].trim();
                    // Register source only: a memory source is a real load.
                    if src.starts_with('%') {
                        if let (Some(sf), Some(df)) = (family_of_reg(src), family_of_reg(dst)) {
                            // dst must be the 64-bit form and src the 32-bit
                            // form of the same family, i.e. a genuine
                            // self-extension rather than a widening move
                            // between different registers.
                            if sf == df
                                && (df as usize) < sext32.len()
                                && sext32[df as usize]
                                && !is_32bit_or_smaller(dst)
                                && is_32bit_or_smaller(src)
                            {
                                keep[i] = false;
                                changed = true;
                                continue;
                            }
                        }
                    }
                }
            }
        }

        // --- Pattern: movzbl %AL, %EAX (redundant re-zero-extend) ---
        if t.starts_with("movzbl ") || t.starts_with("movzbq ") {
            if let Some(comma) = t.rfind(',') {
                // Guard the operand slice.
                //
                // `so` is derived from the first space; slicing `t[so..comma]`
                // panics ("byte range starts at 11 but ends at 9") whenever the
                // first comma appears BEFORE that space -- e.g. a memory
                // operand written without a space after the comma. Observed
                // while compiling kernel/exit.c. `continue` (not an early
                // return) so the remaining lines are still processed.
                let mut so = t.find(' ').unwrap_or(0) + 1;
                if so > comma || comma >= t.len() {
                    // The consumer cannot parse this line, but the line
                    // still REDEFINES its destination: run the producer
                    // update (which classifies conservatively) instead of
                    // skipping straight past it with stale facts.
                    self_update(
                        t,
                        &mut upper32_zero,
                        &mut is_byte,
                        &mut is_16,
                        &mut sext32,
                        &mut sext8,
                        &mut sext16,
                    );
                    continue;
                }
                while so < comma && (t.as_bytes()[so] == b' ' || t.as_bytes()[so] == b'\t') {
                    so += 1;
                }
                let src = t[so..comma].trim();
                let dst = t[comma + 1..].trim();
                let src_fam = family_of_reg(src);
                let dst_fam = family_of_reg(dst);
                if let (Some(sf), Some(df)) = (src_fam, dst_fam) {
                    let is32 = is_32bit_or_smaller(dst);
                    let u32z = (sf as usize).lt(&GP_FAMILIES) && upper32_zero[sf as usize];
                    let bytez = (sf as usize).lt(&GP_FAMILIES) && is_byte[sf as usize];
                    if std::env::var("CCC_TRACE_EXT").is_ok() {
                        eprintln!(
                            "[EXT] t='{}' src_fam={:?} dst_fam={:?} is32={} u32z={} bytez={}",
                            t, sf, df, is32, u32z, bytez
                        );
                    }
                    // A BYTE re-extension is a no-op only when bits 8..31
                    // are already zero (is_byte). upper32_zero alone proves
                    // bits 32..63 — NOT enough: after the movq→movl copy
                    // narrowing below, `movl %edi,%eax; movzbl %al,%eax`
                    // had the movzbl removed on u32z and `f(unsigned char)`
                    // returned the whole of %edi (O1 miscompile of
                    // `c >= 0xc2U`, found via the Expat classify corpus;
                    // upstream fixed the 32-bit form independently). The
                    // 64-bit destination form (movzbq %al,%rax) is a no-op
                    // when bits 32..63 are ALSO proven zero (u32z).
                    if bytez && (is32 || u32z) && sf == df {
                        // Byte value, required upper bits zero: pure no-op.
                        keep[i] = false;
                        changed = true;
                        continue;
                    }
                    if is32 && bytez && sf != df {
                        // Source family is a known byte value (bits 8..31 zero),
                        // so its low byte equals its full 32-bit value. Rewrite
                        // the byte re-extension into a 32-bit copy, which is
                        // shorter and avoids the partial-register dependency.
                        if let (Some(s32), Some(d32)) = (reg32_name(sf), reg32_name(df)) {
                            lines[i] = format!("    movl %{}, %{}", s32, d32);
                            changed = true;
                            // This 32-bit copy zero-extends into df.
                            if (df as usize).lt(&GP_FAMILIES) {
                                upper32_zero[df as usize] = true;
                                is_byte[df as usize] = false;
                                // The copy REDEFINES df (and this rewrite
                                // continues past self_update): every sign
                                // fact about the old value must die here.
                                // A zero-extended byte >= 0x80 is not
                                // sext(low8), so a stale sext8 would
                                // license a wrong movsbq->movq rewrite.
                                sext32[df as usize] = false;
                                sext8[df as usize] = false;
                                sext16[df as usize] = false;
                            }
                            continue;
                        }
                    }
                }
            }
        }

        // --- Pattern: movzwl %AX, %EAX / movzwq %AX, %RAX (redundant
        //    16-bit re-zero-extension of a value already zero-extended) ---
        if ext16_enabled
            && (t.starts_with("movzwl ") || t.starts_with("movzwq "))
            && !t.contains('(')
        {
            // reg-reg form only (memory sources are handled by the flag
            // update below: movzwl mem,%eax already zero-extends).
            if let Some(comma) = t.rfind(',') {
                // Guard the operand slice.
                //
                // `so` is derived from the first space; slicing `t[so..comma]`
                // panics ("byte range starts at 11 but ends at 9") whenever the
                // first comma appears BEFORE that space -- e.g. a memory
                // operand written without a space after the comma. Observed
                // while compiling kernel/exit.c. `continue` (not an early
                // return) so the remaining lines are still processed.
                let mut so = t.find(' ').unwrap_or(0) + 1;
                if so > comma || comma >= t.len() {
                    // The consumer cannot parse this line, but the line
                    // still REDEFINES its destination: run the producer
                    // update (which classifies conservatively) instead of
                    // skipping straight past it with stale facts.
                    self_update(
                        t,
                        &mut upper32_zero,
                        &mut is_byte,
                        &mut is_16,
                        &mut sext32,
                        &mut sext8,
                        &mut sext16,
                    );
                    continue;
                }
                while so < comma && (t.as_bytes()[so] == b' ' || t.as_bytes()[so] == b'\t') {
                    so += 1;
                }
                let src = t[so..comma].trim();
                let dst = t[comma + 1..].trim();
                let src_fam = family_of_reg(src);
                let dst_fam = family_of_reg(dst);
                if let (Some(sf), Some(df)) = (src_fam, dst_fam) {
                    // Same hardening as the byte pattern above: is_16 proves
                    // bits 16..31 zero; the 64-bit form (movzwq) additionally
                    // requires bits 32..63 zero (upper32_zero).
                    let fits16 = (sf as usize).lt(&GP_FAMILIES)
                        && is_16[sf as usize]
                        && (is_32bit_or_smaller(dst) || upper32_zero[sf as usize]);
                    if std::env::var("CCC_TRACE_EXT").is_ok() {
                        eprintln!(
                            "[EXT16] t='{}' src_fam={:?} dst_fam={:?} fits16={}",
                            t, sf, df, fits16
                        );
                    }
                    if fits16 && sf == df {
                        // Same family, value already zero-extended to 16 bits:
                        // the re-extension cannot change anything.
                        keep[i] = false;
                        changed = true;
                        continue;
                    }
                    if fits16 && sf != df {
                        // Cross-family 16-bit re-extension of a known 16-bit
                        // value: rewrite as a 32-bit copy (shorter, no
                        // partial-register dependency).
                        if let (Some(s32), Some(d32)) = (reg32_name(sf), reg32_name(df)) {
                            lines[i] = format!("    movl %{}, %{}", s32, d32);
                            changed = true;
                            if (df as usize).lt(&GP_FAMILIES) {
                                upper32_zero[df as usize] = true;
                                is_byte[df as usize] = false;
                                is_16[df as usize] = false; // 32-bit copy: may exceed 16 bits
                                // Redefinition past self_update: kill the
                                // sign facts too (see the movzbl rewrite).
                                sext32[df as usize] = false;
                                sext8[df as usize] = false;
                                sext16[df as usize] = false;
                            }
                            continue;
                        }
                    }
                }
            }
        }

        // --- Pattern: redundant SIGN re-extension (signed mirror of the
        //    is_byte / is_16 rules above) ---
        // Proof obligations live in rewrite_sign_reextension.
        if sext_enabled
            && (t.starts_with("movsbq ")
                || t.starts_with("movswq ")
                || t.starts_with("movsbl ")
                || t.starts_with("movswl ")
                || t.starts_with("movslq ")
                || t.starts_with("movsxd "))
        {
            let mut facts = ExtFacts {
                upper32_zero: &mut upper32_zero,
                is_byte: &mut is_byte,
                is_16: &mut is_16,
                sext32: &mut sext32,
                sext8: &mut sext8,
                sext16: &mut sext16,
            };
            if let Some(action) = rewrite_sign_reextension(t, &mut facts) {
                match action {
                    ExtRewrite::Delete => keep[i] = false,
                    ExtRewrite::Replace(rep) => lines[i] = rep,
                }
                changed = true;
                continue;
            }
        }
        // --- Pattern: 64-bit copy of a known zero-extended value ---
        //
        // Register allocation often emits a type-erased `movq %src64, %dst64`
        // for a value that is known to live in the low 32 bits.  On x86-64 a
        // 32-bit `movl` zero-extends the destination, so when `src` is already
        // upper-32-zero the two moves are architecturally identical.  `movl`
        // is one byte shorter (no REX.W), avoids a 64-bit dependency, and can
        // remove an artificial whole-64-bit live range in hot integer loops
        // (for example a CRC table index after byte XOR/AND).
        //
        // Only register-to-register copies are rewritten: a memory source may
        // contain an arbitrary 64-bit value, and immediate operands are left
        // to other constant-materialization patterns.
        if t.starts_with("movq") && !t.contains('(') {
            if let Some(comma) = t.rfind(',') {
                const MOVQ_LEN: usize = "movq".len();
                let so = MOVQ_LEN;
                let mut so = so.min(t.len());
                while so < t.len() && (t.as_bytes()[so] == b' ' || t.as_bytes()[so] == b'\t') {
                    so += 1;
                }
                if so < comma && comma < t.len() {
                    let src = t[so..comma].trim();
                    let dst = t[comma + 1..].trim();
                    if src.starts_with('%') && dst.starts_with('%') {
                        if let (Some(sf), Some(df)) = (family_of_reg(src), family_of_reg(dst)) {
                            if (sf as usize) < GP_FAMILIES && (df as usize) < GP_FAMILIES {
                                // SOUNDNESS: `movq %src,%dst` -> `movl` is an
                                // identity ONLY when the source's upper 32
                                // bits are provably zero (then both forms
                                // write the same 64-bit destination value).
                                // The previous gate — "the next instruction
                                // consumes the 32-bit destination form" — was
                                // unsound: a 32-bit READ does not end the
                                // destination's 64-bit live range. Fuzz seed
                                // 4 (-O2): `movq %rax,%r15` (rax = 64-bit
                                // hash) was narrowed because the next line
                                // read %r15d, but `movq %r15,%rcx` consumed
                                // the full value 60 lines later — the upper
                                // half of the hash vanished. The next-use
                                // check is kept only as a PROFITABILITY
                                // filter on top of the zero-extension proof.
                                if upper32_zero[sf as usize] {
                                    if let Some(d32) = reg32_name(df) {
                                        let next = (i + 1..lines.len())
                                            .map(|j| lines[j].trim())
                                            .find(|candidate| {
                                                !candidate.is_empty()
                                                    && !candidate.starts_with('.')
                                                    && !candidate.ends_with(':')
                                            });
                                        if next.is_some_and(|candidate| {
                                            candidate_uses_32bit_dest(candidate, d32)
                                        }) {
                                            if let Some(s32) = reg32_name(sf) {
                                                lines[i] = format!("    movl %{}, %{}", s32, d32);
                                                changed = true;
                                                upper32_zero[df as usize] = true;
                                                is_byte[df as usize] = is_byte[sf as usize];
                                                is_16[df as usize] = is_16[sf as usize];
                                                // PRE-EXISTING SOUNDNESS BUG:
                                                // this narrowing redefines df
                                                // with a ZERO-extended value
                                                // whose bit 31 is arbitrary,
                                                // so sext32 (and the narrower
                                                // sext facts) do NOT hold for
                                                // the new value; they were
                                                // left stale here, and a later
                                                // `movslq %dfd, %df` could be
                                                // deleted as a no-op on that
                                                // stale fact, losing the sign
                                                // extension.  (The byte/word
                                                // rewrite sites above were
                                                // only accidentally safe: their
                                                // values provably have bit 31
                                                // clear.  This one is not.)
                                                sext32[df as usize] = false;
                                                sext8[df as usize] = false;
                                                sext16[df as usize] = false;
                                                continue;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // --- Update flags based on this instruction ---
        self_update(
            t,
            &mut upper32_zero,
            &mut is_byte,
            &mut is_16,
            &mut sext32,
            &mut sext8,
            &mut sext16,
        );
    }

    if changed {
        let mut out = String::with_capacity(asm.len());
        for (idx, line) in lines.iter().enumerate() {
            if keep[idx] {
                out.push_str(line);
                out.push('\n');
            }
        }
        *asm = out;
    }
    changed
}

/// Update the upper32_zero/is_byte/is_16 tracking based on a single instruction.
/// Mnemonics whose 32-bit form writes the low 32 bits of the destination and
/// zero-extends to 64 bits on x86-64.  `cmpl`/`testl` deliberately do NOT
/// appear: they only set flags and leave the register value untouched.
/// `cmov*` deliberately does NOT appear either: its write is conditional,
/// so it can preserve but never establish a fact (handled separately).
fn is_32bit_zero_extend_write(op: &str) -> bool {
    matches!(
        op,
        "addl"
            | "subl"
            | "andl"
            | "orl"
            | "leal"
            | "imull"
            | "sall"
            | "shll"
            | "shrl"
            | "sarl"
            | "popcntl"
            | "lzcntl"
            | "tzcntl"
            | "andnl"
            | "blsrl"
            | "blsil"
            | "bzhil"
            | "shlxl"
            | "shrxl"
            | "sarxl"
    )
}

/// Where a two-operand instruction writes, as far as fact tracking cares.
enum DstKind {
    /// `op src, %reg` -- destination register family.
    Reg(u8),
    /// `op src, disp(%base,...)` -- memory destination: no register is
    /// written, so every tracked fact survives (address registers are
    /// only read).
    Store,
    /// Unparsable or symbolic destination: the caller must clear
    /// conservatively.  Returning early with the destination's facts
    /// still live -- what the old rfind(',') parses did whenever the
    /// operand split failed -- lets a stale fact license a wrong
    /// rewrite later.  Parse failure must mean "know nothing", never
    /// "change nothing".
    Unknown,
}

fn classify_dst(t: &str) -> DstKind {
    match split_two_operands(t) {
        Some((_src, dst)) => {
            if dst.contains('(') {
                DstKind::Store
            } else if let Some(df) = family_of_reg(dst) {
                DstKind::Reg(df)
            } else {
                DstKind::Unknown
            }
        }
        None => DstKind::Unknown,
    }
}

/// Clear every fact of every register family mentioned in the line (the
/// conservative catch-all, shared by producer branches whose destination
/// could not be classified).
#[allow(clippy::too_many_arguments)]
fn clear_mentioned_families(
    t: &str,
    upper32_zero: &mut Vec<bool>,
    is_byte: &mut Vec<bool>,
    is_16: &mut Vec<bool>,
    sext32: &mut Vec<bool>,
    sext8: &mut Vec<bool>,
    sext16: &mut Vec<bool>,
) {
    for tok in t.split_whitespace() {
        if tok.starts_with('%') {
            let name = tok.trim_end_matches(',');
            if let Some(f) = family_of_reg(name) {
                if (f as usize).lt(&upper32_zero.len()) {
                    upper32_zero[f as usize] = false;
                    is_byte[f as usize] = false;
                    is_16[f as usize] = false;
                    sext32[f as usize] = false;
                    sext8[f as usize] = false;
                    sext16[f as usize] = false;
                }
            }
        }
    }
}

/// The six fact vectors threaded through the consumer helpers.
enum ExtRewrite {
    Delete,
    Replace(String),
}

struct ExtFacts<'a> {
    upper32_zero: &'a mut Vec<bool>,
    is_byte: &'a mut Vec<bool>,
    is_16: &'a mut Vec<bool>,
    sext32: &'a mut Vec<bool>,
    sext8: &'a mut Vec<bool>,
    sext16: &'a mut Vec<bool>,
}

/// Consumer for a redundant SIGN re-extension (signed mirror of the
/// is_byte / is_16 rules in the main loop).
///
/// `movsbq %al, %rXX` where %rax provably already holds sext(its low
/// 8 bits) computes a value that is ALREADY sitting in %rax: the
/// instruction is a plain 64-bit copy (or a no-op when the families
/// coincide). Emitted adjacently by "I8 load + signed widening cast"
/// pairs (signed-char table lookups, base64 decode): the load is a
/// sign-extending movsbq into the accumulator and the cast then
/// re-extends the very same byte.
///
/// Returns the rewrite to apply, if any (the caller applies it and
/// then skips the generic self_update, exactly as the inline patterns
/// do). Fact updates for the rewritten destination happen here; the
/// textual replacement is returned so the caller's live `t` borrow of
/// the line is never invalidated inside the helper.
fn rewrite_sign_reextension(t: &str, facts: &mut ExtFacts<'_>) -> Option<ExtRewrite> {
    if let Some((src, dst)) = split_two_operands(t) {
        if src.starts_with('%') {
            if let (Some(sf), Some(df)) = (family_of_reg(src), family_of_reg(dst)) {
                let (sfi, dfi) = (sf as usize, df as usize);
                if sfi < GP_FAMILIES && dfi < GP_FAMILIES {
                    let to64 = !is_32bit_or_smaller(dst);
                    // The source must textually be the LOW
                    // subregister of its family at exactly the
                    // extended width.  family_of_reg already
                    // rejects the high-byte registers, but the
                    // facts are statements about low bits, so say
                    // so in the match itself rather than relying
                    // on that table never growing an "ah" entry.
                    let src_name = src.trim_start_matches('%');
                    let src_is_low = if t.starts_with("movsb") {
                        reg8_name(sf) == Some(src_name)
                    } else if t.starts_with("movsw") {
                        reg16_name(sf) == Some(src_name)
                    } else {
                        reg32_name(sf) == Some(src_name)
                    };
                    // Exact-width fact required: a value known only
                    // to be sext-from-16 may be 0x0080, where
                    // sext-from-8 would produce ...FF80 -- rewriting
                    // on the wider fact alone would miscompile.
                    let proven = src_is_low
                        && if t.starts_with("movsb") {
                            facts.sext8[sfi]
                        } else if t.starts_with("movsw") {
                            facts.sext16[sfi]
                        } else {
                            facts.sext32[sfi]
                        };
                    if proven && to64 {
                        if sf == df {
                            return Some(ExtRewrite::Delete);
                        }
                        if let (Some(s64), Some(d64)) = (reg64_name(sf), reg64_name(df)) {
                            let replacement = format!("    movq %{}, %{}", s64, d64);
                            // Propagate like the movq producer.
                            facts.upper32_zero[dfi] = facts.upper32_zero[sfi];
                            facts.is_byte[dfi] = facts.is_byte[sfi];
                            facts.is_16[dfi] = facts.is_16[sfi];
                            facts.sext32[dfi] = facts.sext32[sfi];
                            facts.sext8[dfi] = facts.sext8[sfi];
                            facts.sext16[dfi] = facts.sext16[sfi];
                            return Some(ExtRewrite::Replace(replacement));
                        }
                    }
                    if proven && !to64 {
                        // 32-bit destination: movsbl/movswl and movl
                        // both write the same low 32 bits (the
                        // source's low 32 already hold the
                        // sign-extended pattern) and both zero
                        // bits 32..63, so the copy is exact.
                        if let (Some(s32), Some(d32)) = (reg32_name(sf), reg32_name(df)) {
                            let replacement = format!("    movl %{}, %{}", s32, d32);
                            facts.upper32_zero[dfi] = true;
                            facts.is_byte[dfi] = false;
                            facts.is_16[dfi] = false;
                            facts.sext32[dfi] = false;
                            facts.sext8[dfi] = false;
                            facts.sext16[dfi] = false;
                            return Some(ExtRewrite::Replace(replacement));
                        }
                    }
                }
            }
        }
    }
    None
}

/// movq producer: `%regA -> %regB` copies all facts from A to B; a movq
/// from MEMORY or IMMEDIATE gives the destination an opaque 64-bit
/// value, so every fact for the destination family is CLEARED.  (The
/// old code returned early when the source was not a plain register,
/// leaving stale flags: a later re-extension of the loaded value
/// would then be wrongly removed, corrupting the upper bits seen by
/// 64-bit consumers.  Stores change no register facts.)
fn update_facts_for_movq(t: &str, op: &str, facts: &mut ExtFacts<'_>) {
    if let Some(comma) = t.rfind(',') {
        // Split "movq <src>, <dst>" at the FIRST comma that separates the
        // operands -- but only when the mnemonic really ends before it.
        //
        // `so` was derived from the first space and then sliced as
        // `t[so..comma]`, which panics ("byte range starts at 11 but ends
        // at 9") whenever the first comma appears before that space, e.g.
        // for a memory operand written without a space after the comma.
        // Deriving the operand text from the already-tokenized mnemonic
        // and guarding the range makes the slice impossible to invert.
        let so = op.len();
        let mut so = so.min(t.len());
        while so < t.len() && (t.as_bytes()[so] == b' ' || t.as_bytes()[so] == b'\t') {
            so += 1;
        }
        if so > comma || comma >= t.len() {
            // Unparsable shape: never return with facts untouched (the
            // stale-fact class the classify_dst conversion eliminated
            // from the other producer branches).  Unreachable for the
            // emitter's own movq forms; clear conservatively anyway.
            clear_mentioned_families(
                t,
                facts.upper32_zero,
                facts.is_byte,
                facts.is_16,
                facts.sext32,
                facts.sext8,
                facts.sext16,
            );
            return;
        }
        let src = t[so..comma].trim();
        let dst = t[comma + 1..].trim();
        let src_fam = family_of_reg(src);
        let dst_fam = family_of_reg(dst);
        if let (Some(sf), Some(df)) = (src_fam, dst_fam) {
            if (sf as usize).lt(&facts.upper32_zero.len()) {
                facts.upper32_zero[df as usize] = facts.upper32_zero[sf as usize];
                facts.is_byte[df as usize] = facts.is_byte[sf as usize];
                facts.is_16[df as usize] = facts.is_16[sf as usize];
                facts.sext32[df as usize] = facts.sext32[sf as usize];
                facts.sext8[df as usize] = facts.sext8[sf as usize];
                facts.sext16[df as usize] = facts.sext16[sf as usize];
            }
        } else if let Some(df) = dst_fam {
            // movq from memory/immediate (or unparsable source): the
            // 64-bit value is opaque — clear all dest flags.
            if (df as usize).lt(&facts.upper32_zero.len()) {
                facts.upper32_zero[df as usize] = false;
                facts.is_byte[df as usize] = false;
                facts.is_16[df as usize] = false;
                facts.sext32[df as usize] = false;
                facts.sext8[df as usize] = false;
                facts.sext16[df as usize] = false;
            }
        }
    }
    return;
}
/// cmov* writes its destination ONLY when the condition holds.  It must
/// never ESTABLISH a fact: if the move does not execute, the old value
/// (with arbitrary upper bits) survives.  Treating a 32-bit cmov as an
/// unconditional zero-extending write (its former home in
/// is_32bit_zero_extend_write) set upper32_zero for a register that may
/// still hold a full 64-bit value, licensing the movq->movl narrowing to
/// drop the upper half.  The only sound preservation: a 32-bit cmov
/// keeps a PRE-EXISTING upper32_zero fact (taken: the write zero-extends;
/// not taken: the already-proven value is kept).  Every other fact for
/// the destination becomes unprovable either way and is cleared; 16- and
/// 64-bit forms clear upper32_zero too (a taken 64-bit cmov imports the
/// source's arbitrary upper half).
fn update_facts_for_cmov(t: &str, op: &str, facts: &mut ExtFacts<'_>) {
    match classify_dst(t) {
        DstKind::Reg(df) => {
            let dfi = df as usize;
            if dfi < facts.upper32_zero.len() {
                if !op.ends_with('l') {
                    facts.upper32_zero[dfi] = false;
                }
                facts.is_byte[dfi] = false;
                facts.is_16[dfi] = false;
                facts.sext32[dfi] = false;
                facts.sext8[dfi] = false;
                facts.sext16[dfi] = false;
            }
        }
        DstKind::Store => {}
        DstKind::Unknown => clear_mentioned_families(
            t,
            facts.upper32_zero,
            facts.is_byte,
            facts.is_16,
            facts.sext32,
            facts.sext8,
            facts.sext16,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn self_update(
    t: &str,
    upper32_zero: &mut Vec<bool>,
    is_byte: &mut Vec<bool>,
    is_16: &mut Vec<bool>,
    sext32: &mut Vec<bool>,
    sext8: &mut Vec<bool>,
    sext16: &mut Vec<bool>,
) {
    let toks: Vec<&str> = t.split_whitespace().collect();
    if toks.is_empty() {
        return;
    }
    let op = toks[0];

    // Calls clobber the full 64-bit values of caller-saved registers.  The
    // assembly line for a direct call often contains no `%reg`, so the generic
    // operand scan below would not invalidate any state.  Clear all tracked
    // families conservatively: losing one extension/copy-narrowing across a
    // call is harmless; trusting stale upper-32-zero state across a call can
    // miscompile a later 64-bit copy.
    if op == "call" || op == "callq" {
        for flags in [
            &mut *upper32_zero,
            &mut *is_byte,
            &mut *is_16,
            &mut *sext32,
            &mut *sext8,
            &mut *sext16,
        ] {
            for value in flags.iter_mut() {
                *value = false;
            }
        }
        return;
    }

    if op == "movq" && t.contains('%') {
        update_facts_for_movq(
            t,
            op,
            &mut ExtFacts {
                upper32_zero: &mut *upper32_zero,
                is_byte: &mut *is_byte,
                is_16: &mut *is_16,
                sext32: &mut *sext32,
                sext8: &mut *sext8,
                sext16: &mut *sext16,
            },
        );
        return;
    }
    // Byte zero-extension into a register sets upper32_zero, is_byte AND
    // is_16 (a byte value fits in 16 bits).
    //
    // PRE-EXISTING BUG: this branch tested `op.starts_with("movzbl")`, which
    // `movzbq` does NOT satisfy, so a byte zero-extended to 64 bits fell into
    // the 16-bit branch below and never set is_byte. The consumer rule then
    // could not prove a directly following `movzbq %dil, %rdi` redundant and
    // the duplicate extension survived into the emitted code.
    if op == "movzbl" || op == "movzbq" {
        match classify_dst(t) {
            DstKind::Reg(df) => {
                if (df as usize).lt(&upper32_zero.len()) {
                    upper32_zero[df as usize] = true;
                    is_byte[df as usize] = true;
                    is_16[df as usize] = std::env::var("CCC_NO_EXT16").is_err();
                    sext32[df as usize] = false;
                    sext8[df as usize] = false;
                    sext16[df as usize] = false;
                }
            }
            DstKind::Store => {}
            DstKind::Unknown => {
                clear_mentioned_families(t, upper32_zero, is_byte, is_16, sext32, sext8, sext16)
            }
        }
        return;
    }
    // 16-bit zero-extension: upper-32 zero, fits-16, but NOT a byte.
    //
    // PRE-EXISTING LATENT UNSOUNDNESS: the old third disjunct
    // `op.starts_with("movz")` also matched `movzbw`, whose 16-bit
    // destination leaves bits 16..63 STALE -- marking that family
    // upper32_zero is false and could license a wrong movq->movl narrowing
    // downstream. Only the exact 32/64-bit-destination forms establish the
    // fact; `movzbw` now falls through to the conservative catch-all.
    if op == "movzwl" || op == "movzwq" {
        match classify_dst(t) {
            DstKind::Reg(df) => {
                if (df as usize).lt(&upper32_zero.len()) {
                    upper32_zero[df as usize] = true;
                    is_byte[df as usize] = false;
                    is_16[df as usize] = std::env::var("CCC_NO_EXT16").is_err();
                    sext32[df as usize] = false;
                    sext8[df as usize] = false;
                    sext16[df as usize] = false;
                }
            }
            DstKind::Store => {}
            DstKind::Unknown => {
                clear_mentioned_families(t, upper32_zero, is_byte, is_16, sext32, sext8, sext16)
            }
        }
        return;
    }
    // movl / xorl: zero-extends to 32 bits, but NOT necessarily a byte or
    // 16-bit value.
    if op == "movl" || op == "xorl" {
        match classify_dst(t) {
            DstKind::Reg(df) => {
                if (df as usize).lt(&upper32_zero.len()) {
                    upper32_zero[df as usize] = true;
                    is_byte[df as usize] = false;
                    is_16[df as usize] = false;
                    sext32[df as usize] = false;
                    sext8[df as usize] = false;
                    sext16[df as usize] = false;
                }
            }
            DstKind::Store => {}
            DstKind::Unknown => {
                clear_mentioned_families(t, upper32_zero, is_byte, is_16, sext32, sext8, sext16)
            }
        }
        return;
    }
    // Conditional moves: see update_facts_for_cmov (never establish).
    if op.starts_with("cmov") {
        update_facts_for_cmov(
            t,
            op,
            &mut ExtFacts {
                upper32_zero: &mut *upper32_zero,
                is_byte: &mut *is_byte,
                is_16: &mut *is_16,
                sext32: &mut *sext32,
                sext8: &mut *sext8,
                sext16: &mut *sext16,
            },
        );
        return;
    }
    // Any other 32-bit ALU write (addl, subl, andl, orl, leal, imull, shifts)
    // zero-extends to 64 bits on x86-64, so the destination family is
    // upper-32-zero (but not byte/16-bit-known).  Without this, an
    // accumulator chain like `addl %eax, %esi` CLEARED the fact via the
    // conservative fallback below, and a following `movq %rsi, %r12` never
    // narrowed to the shorter, dependency-free `movl %esi, %r12d`.
    if is_32bit_zero_extend_write(op) {
        match classify_dst(t) {
            DstKind::Reg(df) => {
                if (df as usize).lt(&upper32_zero.len()) {
                    upper32_zero[df as usize] = true;
                    is_byte[df as usize] = false;
                    is_16[df as usize] = false;
                    sext32[df as usize] = false;
                    sext8[df as usize] = false;
                    sext16[df as usize] = false;
                }
            }
            DstKind::Store => {}
            DstKind::Unknown => {
                clear_mentioned_families(t, upper32_zero, is_byte, is_16, sext32, sext8, sext16)
            }
        }
        return;
    }

    // movslq <src32>, <dst64>: by definition the destination now holds
    // sext(its own low 32 bits) -- its low 32 bits ARE the source value. The
    // zero-extension facts do not survive (bit 31 may have been replicated
    // into bits 32..63).
    if op == "movslq" || op == "movsxd" {
        match classify_dst(t) {
            DstKind::Reg(df) => {
                if (df as usize).lt(&upper32_zero.len()) {
                    upper32_zero[df as usize] = false;
                    is_byte[df as usize] = false;
                    is_16[df as usize] = false;
                    sext32[df as usize] = true;
                    sext8[df as usize] = false;
                    sext16[df as usize] = false;
                }
            }
            DstKind::Store => {}
            DstKind::Unknown => {
                clear_mentioned_families(t, upper32_zero, is_byte, is_16, sext32, sext8, sext16)
            }
        }
        return;
    }

    // movsbq / movswq <any source>, <dst64>: the destination now holds the
    // sign-extension of a byte / word to 64 bits. That establishes the
    // matching narrow fact and, by the width implication, every wider one
    // (sext-from-8 is also sext-from-16 and sext-from-32 of the resulting
    // low bits, since the sign bit is replicated across all of them).
    if op == "movsbq" || op == "movswq" {
        match classify_dst(t) {
            DstKind::Reg(df) => {
                if (df as usize).lt(&upper32_zero.len()) {
                    upper32_zero[df as usize] = false;
                    is_byte[df as usize] = false;
                    is_16[df as usize] = false;
                    sext32[df as usize] = true;
                    sext16[df as usize] = true;
                    sext8[df as usize] = op == "movsbq";
                }
            }
            DstKind::Store => {}
            DstKind::Unknown => {
                clear_mentioned_families(t, upper32_zero, is_byte, is_16, sext32, sext8, sext16)
            }
        }
        return;
    }

    // movsbl / movswl <any source>, <dst32>: a 32-bit destination write,
    // so bits 32..63 are zero (upper32_zero). The 64-bit sign-extension
    // facts do NOT hold (a negative value has zeros, not ones, above bit
    // 31), so they are cleared. Previously these fell into the catch-all,
    // which also discarded the true upper32_zero fact.
    if op == "movsbl" || op == "movswl" {
        match classify_dst(t) {
            DstKind::Reg(df) => {
                if (df as usize).lt(&upper32_zero.len()) {
                    upper32_zero[df as usize] = true;
                    is_byte[df as usize] = false;
                    is_16[df as usize] = false;
                    sext32[df as usize] = false;
                    sext8[df as usize] = false;
                    sext16[df as usize] = false;
                }
            }
            DstKind::Store => {}
            DstKind::Unknown => {
                clear_mentioned_families(t, upper32_zero, is_byte, is_16, sext32, sext8, sext16)
            }
        }
        return;
    }

    // setcc %al: writes only low 8 bits; upper bits stale -> clear all flags
    // for the family (the low byte is a fresh bool, but the value cannot be
    // proven to fit in 16 bits because bits 8..63 are stale).
    if op.starts_with("set") && t.contains('%') {
        if let Some(fam) = t.split_whitespace().last().and_then(|r| family_of_reg(r)) {
            if (fam as usize).lt(&upper32_zero.len()) {
                upper32_zero[fam as usize] = false;
                is_byte[fam as usize] = false;
                is_16[fam as usize] = false;
                sext32[fam as usize] = false;
                sext8[fam as usize] = false;
                sext16[fam as usize] = false;
            }
        }
        return;
    }

    // Pure stores (memory destination) write NO register, so every tracked
    // fact survives them -- including facts about the registers used in the
    // address. Without this, `movb %al, 1(%r10,%r15,4)` between two
    // `movslq %r15d, %r15` reached the catch-all below, which cleared the
    // sext32 fact for %r15 merely for being MENTIONED, and the duplicate
    // extension survived. The load forms already preserve index facts via
    // their early producer branches; this makes stores consistent.
    if matches!(op, "movb" | "movw" | "movl" | "movq") {
        // The destination address itself contains commas ((%r10,%r15,4)),
        // so the separator must be found parenthesis-aware -- rfind(',')
        // would land inside the address.
        if let Some((_src, dst)) = split_two_operands(t) {
            if dst.contains('(') {
                return;
            }
        }
    }

    // Anything else that writes registers: conservatively clear the flags of
    // EVERY register family mentioned in the line. This is the soundness
    // fix: the previous code only cleared when the OPCODE contained '%',
    // which missed `addq %rcx, %rax`, `leaq 0x30(%rsp), %rax`, `imulq %r13,
    // %r14`, shifts, etc. A stale upper-32-zero/is_byte flag on such a line
    // could wrongly eliminate a real re-extension (miscompile). Clearing
    // extra families is always safe (only loses optimizations).
    clear_mentioned_families(t, upper32_zero, is_byte, is_16, sext32, sext8, sext16);
}

#[cfg(test)]
mod tests {
    use super::eliminate_redundant_zero_extend;

    fn run(input: &str) -> String {
        let mut asm = input.to_string();
        assert!(eliminate_redundant_zero_extend(&mut asm));
        asm
    }

    // ── redundant self sign-extension (movslq %Xd, %X) ──────────────────

    fn run_maybe(input: &str) -> String {
        let mut asm = input.to_string();
        let _ = eliminate_redundant_zero_extend(&mut asm);
        asm
    }

    #[test]
    fn removes_a_second_self_sign_extension_of_the_same_register() {
        // The real gzip longest_match compare loop: a[i] and b[i] share one
        // sign-extended index, and address lowering re-materialises it.
        let out = run(concat!(
            "foo:\n",
            "    movslq %r15d, %r15\n",
            "    movzbl (%r12, %r15), %r10d\n",
            "    movslq %r15d, %r15\n",
            "    movzbl (%r13, %r15), %eax\n",
        ));
        assert_eq!(out.matches("movslq").count(), 1);
        // The loads must survive untouched.
        assert!(out.contains("movzbl (%r12, %r15), %r10d"));
        assert!(out.contains("movzbl (%r13, %r15), %eax"));
    }

    #[test]
    fn keeps_a_sign_extension_from_a_different_register() {
        // `movslq %eax, %r15` moves a different value into r15; the fact that
        // r15 already held a sign-extended value says nothing about it.
        let out = run_maybe(concat!(
            "foo:\n",
            "    movslq %r15d, %r15\n",
            "    movslq %eax, %r15\n",
        ));
        assert_eq!(out.matches("movslq").count(), 2);
    }

    #[test]
    fn keeps_a_sign_extension_after_the_register_is_redefined() {
        // addl rewrites the low 32 bits (and zeroes the top), so the value is
        // no longer sext(low32) -- bit 31 may now be set.
        let out = run_maybe(concat!(
            "foo:\n",
            "    movslq %r15d, %r15\n",
            "    addl $1, %r15d\n",
            "    movslq %r15d, %r15\n",
        ));
        assert_eq!(out.matches("movslq").count(), 2);
    }

    #[test]
    fn keeps_a_sign_extension_across_a_label() {
        // At a label the predecessor is unknown, so no fact survives. This is
        // the loop case: the back-edge re-enters with a fresh index.
        let out = run_maybe(concat!(
            "foo:\n",
            "    movslq %r15d, %r15\n",
            ".LBB1:\n",
            "    movslq %r15d, %r15\n",
        ));
        assert_eq!(out.matches("movslq").count(), 2);
    }

    #[test]
    fn keeps_a_sign_extension_across_a_call() {
        let out = run_maybe(concat!(
            "foo:\n",
            "    movslq %r15d, %r15\n",
            "    call bar\n",
            "    movslq %r15d, %r15\n",
        ));
        assert_eq!(out.matches("movslq").count(), 2);
    }

    #[test]
    fn keeps_a_sign_extension_of_a_memory_operand() {
        // A memory source is a real load, not a re-extension.
        let out = run_maybe(concat!(
            "foo:\n",
            "    movslq %r15d, %r15\n",
            "    movslq -8(%rbp), %r15\n",
        ));
        assert_eq!(out.matches("movslq").count(), 2);
    }

    #[test]
    fn a_zero_extending_write_does_not_establish_the_sign_extended_fact() {
        // SOUNDNESS: movl zeroes bits 32..63, but if bit 31 is set then
        // sext(low32) sets them all instead -- the values differ, so the
        // following movslq is NOT removable.
        let out = run_maybe(concat!(
            "foo:\n",
            "    movl %eax, %r15d\n",
            "    movslq %r15d, %r15\n",
        ));
        assert_eq!(out.matches("movslq").count(), 1);
    }

    #[test]
    fn the_fact_travels_through_a_register_to_register_movq() {
        let out = run(concat!(
            "foo:\n",
            "    movslq %r15d, %r15\n",
            "    movq %r15, %r14\n",
            "    movslq %r14d, %r14\n",
        ));
        assert_eq!(out.matches("movslq").count(), 1);
    }

    #[test]
    fn narrows_known_zero_extended_register_copy() {
        let asm = run("crc32_update:\n\
             \x20   movzbl (%rdx), %r8d\n\
             \x20   movl %esi, %eax\n\
             \x20   movq %r8, %rcx\n\
             \x20   xorl %ecx, %eax\n\
             \x20   movq %rax, %r8\n\
             \x20   andl $255, %r8d\n");
        assert!(asm.contains("movl %r8d, %ecx"), "asm was:\n{asm}");
        assert!(asm.contains("movl %eax, %r8d"), "asm was:\n{asm}");
        assert!(!asm.contains("movq %r8, %rcx"));
        assert!(!asm.contains("movq %rax, %r8"));
    }

    #[test]
    fn does_not_narrow_opaque_memory_load_copy() {
        let input = "f:\n    movq (%rdx), %rax\n    movq %rax, %rcx\n";
        let mut asm = input.to_string();
        assert!(!eliminate_redundant_zero_extend(&mut asm));
        assert_eq!(asm, input);
    }

    #[test]
    fn keeps_byte_truncation_after_arbitrary_32bit_value() {
        // Writing EAX proves only that bits 32..63 are zero. The low 32 bits
        // can still be 0xffffffd8, so this I8->U8 cast must clear bits 8..31.
        let input = "f:\n    movl %edi, %eax\n    movzbl %al, %eax\n";
        let mut asm = input.to_string();
        assert!(!eliminate_redundant_zero_extend(&mut asm));
        assert_eq!(asm, input);
    }

    #[test]
    fn removes_byte_reextension_of_known_byte_value() {
        let asm = run("f:\n\
             \x20   movzbl (%rdi), %eax\n\
             \x20   movzbl %al, %eax\n");
        assert_eq!(asm.matches("movzbl").count(), 1, "asm was:\n{asm}");
        assert!(asm.contains("movzbl (%rdi), %eax"), "asm was:\n{asm}");
    }

    #[test]
    fn removes_byte_reextension_after_sib_memory_load() {
        // The comma inside the SIB memory operand must not confuse operand
        // splitting: the load still records a byte fact for %eax, so the
        // register re-extension is a no-op (gzip CRC / table-index shape).
        let asm = run("f:\n\
             \x20   movzbl (%rcx, %r12), %eax\n\
             \x20   movzbl %al, %eax\n");
        assert_eq!(asm.matches("movzbl").count(), 1, "asm was:\n{asm}");
        assert!(asm.contains("movzbl (%rcx, %r12), %eax"), "asm was:\n{asm}");
    }

    #[test]
    fn removes_16bit_reextension_after_sib_memory_load() {
        let asm = run("f:\n\
             \x20   movzwl (%rcx, %rdi, 2), %eax\n\
             \x20   movzwl %ax, %eax\n");
        assert_eq!(asm.matches("movzwl").count(), 1, "asm was:\n{asm}");
        assert!(
            asm.contains("movzwl (%rcx, %rdi, 2), %eax"),
            "asm was:\n{asm}"
        );
    }

    #[test]
    fn narrows_copy_after_32bit_alu_accumulation() {
        // 32-bit ALU writes zero-extend: after `addl`/`subl` the accumulator
        // family is upper-32-zero, so a 64-bit copy into a 32-bit consumer
        // narrows to `movl`.
        let asm = run("f:\n\
             \x20   movl 128(%rsp), %esi\n\
             \x20   addl %r15d, %esi\n\
             \x20   subl $45, %esi\n\
             \x20   movq %rsi, %r12\n\
             \x20   addl %eax, %r12d\n");
        assert!(asm.contains("movl %esi, %r12d"), "asm was:\n{asm}");
        assert!(!asm.contains("movq %rsi, %r12"), "asm was:\n{asm}");
    }

    #[test]
    fn does_not_claim_32bit_fact_from_flags_only_ops() {
        // cmpl/testl only set flags; they must not mark the operand register
        // upper-32-zero.  A 64-bit copy after them stays 64-bit.
        let input = "f:\n    cmpl %edx, %esi\n    movq %rsi, %r12\n";
        let mut asm = input.to_string();
        let _ = eliminate_redundant_zero_extend(&mut asm);
        assert_eq!(asm, input, "cmpl must not create an upper-32-zero fact");
    }

    #[test]
    fn facts_do_not_cross_inline_asm_region() {
        // The template redefines %rax through a raw encoding the textual
        // scan cannot see; the post-asm re-extension must survive.
        let input = "f:\n\
             \x20   movzbl (%rdi), %eax\n\
             \x20   #APP\n\
             \x20   .byte 0x31, 0xc0\n\
             \x20   #NO_APP\n\
             \x20   movzbl %al, %eax\n";
        let mut asm = input.to_string();
        let changed = eliminate_redundant_zero_extend(&mut asm);
        // The trailing movzbl must survive; also nothing inside the region
        // may be rewritten.
        assert!(asm.contains("#APP") && asm.contains(".byte 0x31, 0xc0"));
        assert!(asm.matches("movzbl").count() == 2, "asm was:\n{asm}");
        let _ = changed;
    }

    #[test]
    fn deliberate_redundancy_inside_asm_region_is_untouched() {
        // Kernel ALTERNATIVE() length placeholder: a deliberate
        // `movq %rax, %rax` inside a region must reach the assembler.
        let input = "f:\n\
             \x20   #APP\n\
             \x20   movq %rax, %rax\n\
             \x20   #NO_APP\n";
        let mut asm = input.to_string();
        let _ = eliminate_redundant_zero_extend(&mut asm);
        assert_eq!(asm, input, "user bytes must be immutable");
    }

    #[test]
    fn sib_memory_movq_load_clears_byte_fact() {
        // SOUNDNESS: an opaque 64-bit load through a SIB address must clear
        // the stale byte fact for %rax, otherwise the following re-extension
        // would be wrongly removed (upper bits of a loaded pointer survive).
        let input = "f:\n\
             \x20   movzbl (%rdi), %eax\n\
             \x20   movq (%rcx, %r12), %rax\n\
             \x20   movzbl %al, %eax\n";
        let mut asm = input.to_string();
        let _ = eliminate_redundant_zero_extend(&mut asm);
        // The trailing movzbl must survive: %rax holds an opaque load result.
        assert!(asm.contains("movzbl %al, %eax"), "asm was:\n{asm}");
        assert!(asm.contains("movq (%rcx, %r12), %rax"), "asm was:\n{asm}");
    }

    // ── redundant sign re-extension (movsbq/movswq/movsbl/movswl) ───────

    #[test]
    fn collapses_signed_load_then_widen_pair_to_movq() {
        // The base64-decode hot shape: an I8 load is a sign-extending movsbq
        // into %rax and the widening cast re-extends the same byte into the
        // target register. The second extension is a plain copy.
        let out = run(concat!(
            "foo:\n",
            "    movsbq (%r10), %rax\n",
            "    movsbq %al, %r11\n",
        ));
        assert_eq!(out.matches("movsbq").count(), 1, "asm was:\n{out}");
        assert!(out.contains("movq %rax, %r11"), "asm was:\n{out}");
    }

    #[test]
    fn removes_self_sign_reextension_of_a_byte() {
        let out = run(concat!(
            "foo:\n",
            "    movsbq (%rdi), %rax\n",
            "    movsbq %al, %rax\n",
        ));
        assert_eq!(out.matches("movsbq").count(), 1, "asm was:\n{out}");
    }

    #[test]
    fn rewrites_32bit_sign_reextension_to_movl() {
        let out = run(concat!(
            "foo:\n",
            "    movsbq (%rdi), %rax\n",
            "    movsbl %al, %ecx\n",
        ));
        assert!(out.contains("movl %eax, %ecx"), "asm was:\n{out}");
        assert_eq!(out.matches("movsb").count(), 1, "asm was:\n{out}");
    }

    #[test]
    fn collapses_signed_word_load_then_widen_pair_to_movq() {
        let out = run(concat!(
            "foo:\n",
            "    movswq (%rdi), %rax\n",
            "    movswq %ax, %rcx\n",
        ));
        assert_eq!(out.matches("movswq").count(), 1, "asm was:\n{out}");
        assert!(out.contains("movq %rax, %rcx"), "asm was:\n{out}");
    }

    #[test]
    fn word_fact_does_not_license_a_byte_reextension_rewrite() {
        // SOUNDNESS: 0x0080 is a valid sext-from-16 result whose low byte
        // sign-extends to ...FF80. Requiring the exact-width fact keeps
        // this genuine re-extension.
        let out = run_maybe(concat!(
            "foo:\n",
            "    movswq (%rdi), %rax\n",
            "    movsbq %al, %rcx\n",
        ));
        assert!(out.contains("movsbq %al, %rcx"), "asm was:\n{out}");
    }

    #[test]
    fn keeps_sign_reextension_after_the_register_is_redefined() {
        let out = run_maybe(concat!(
            "foo:\n",
            "    movsbq (%rdi), %rax\n",
            "    addq $1, %rax\n",
            "    movsbq %al, %rcx\n",
        ));
        assert!(out.contains("movsbq %al, %rcx"), "asm was:\n{out}");
    }

    #[test]
    fn keeps_sign_reextension_across_a_label() {
        let out = run_maybe(concat!(
            "foo:\n",
            "    movsbq (%rdi), %rax\n",
            ".L1:\n",
            "    movsbq %al, %rcx\n",
        ));
        assert!(out.contains("movsbq %al, %rcx"), "asm was:\n{out}");
    }

    #[test]
    fn sign_fact_travels_through_movq_for_byte_values() {
        let out = run(concat!(
            "foo:\n",
            "    movsbq (%rdi), %rax\n",
            "    movq %rax, %rcx\n",
            "    movsbq %cl, %rdx\n",
        ));
        assert!(out.contains("movq %rcx, %rdx"), "asm was:\n{out}");
        assert_eq!(out.matches("movsbq").count(), 1, "asm was:\n{out}");
    }

    #[test]
    fn movslq_from_a_known_sign_extended_register_becomes_movq() {
        let out = run(concat!(
            "foo:\n",
            "    movslq %eax, %rax\n",
            "    movslq %eax, %rcx\n",
        ));
        assert!(out.contains("movq %rax, %rcx"), "asm was:\n{out}");
        assert_eq!(out.matches("movslq").count(), 1, "asm was:\n{out}");
    }

    #[test]
    fn removes_a_duplicate_movzbq_of_the_same_register() {
        // Regression for the movzbq producer missing is_byte: the cast of a
        // freshly zero-extended argument emitted the identical extension
        // twice in a row.
        let out = run(concat!(
            "foo:\n",
            "    movzbq %dil, %rdi\n",
            "    movzbq %dil, %rdi\n",
        ));
        assert_eq!(out.matches("movzbq").count(), 1, "asm was:\n{out}");
    }

    #[test]
    fn a_pure_store_between_sign_extensions_preserves_the_fact() {
        // A store that merely READS the index register in its address must
        // not clear the sign-extension fact (loads already preserve it).
        let out = run(concat!(
            "foo:\n",
            "    movslq %r15d, %r15\n",
            "    movb %al, (%r10, %r15, 4)\n",
            "    movslq %r15d, %r15\n",
            "    movb %al, 1(%r10, %r15, 4)\n",
        ));
        assert_eq!(out.matches("movslq").count(), 1, "asm was:\n{out}");
        assert_eq!(out.matches("movb").count(), 2, "asm was:\n{out}");
    }

    #[test]
    fn cmov_does_not_establish_upper32_zero() {
        // cmov writes its destination only when the condition holds, so a
        // 32-bit cmov must NOT establish upper32_zero for the destination:
        // if the move does not happen, the old (arbitrary) upper 32 bits
        // survive.  Establishing the fact lets the movq->movl narrowing
        // below drop those bits.  (%r15 arrives with a full 64-bit value;
        // the cmov may or may not overwrite its low half.)
        let out = run_maybe(concat!(
            "foo:\n",
            "    movq %rdi, %r15\n",
            "    cmovel %eax, %r15d\n",
            "    movq %r15, %rcx\n",
            "    addl %ecx, %edx\n",
        ));
        assert!(
            out.contains("movq %r15, %rcx"),
            "cmov-established fact wrongly narrowed the copy:\n{out}"
        );
        // A fact that held BEFORE a 32-bit cmov still holds after it (taken:
        // zero-extending write; not taken: proven value kept), so the
        // narrowing remains available and must still fire.
        let out = run(concat!(
            "foo:\n",
            "    movl %edi, %r15d\n",
            "    cmovel %eax, %r15d\n",
            "    movq %r15, %rcx\n",
            "    addl %ecx, %edx\n",
        ));
        assert!(
            out.contains("movl %r15d, %ecx"),
            "pre-existing fact must survive a 32-bit cmov:\n{out}"
        );
    }

    #[test]
    fn movq_narrowing_to_movl_clears_the_sext32_fact() {
        // Regression (pre-existing): the movq->movl narrowing site left
        // sext32 stale for the REDEFINED destination.  The narrowed copy
        // zero-extends an arbitrary 32-bit value, so a later self-movslq
        // is a real sign extension and must survive.
        let out = run(concat!(
            "foo:\n",
            "    movslq %r15d, %r15\n",
            "    movzbl (%rdi), %eax\n",
            "    movq %rax, %r15\n",
            "    addl %r15d, %ecx\n",
            "    movslq %r15d, %r15\n",
        ));
        assert!(
            out.contains("movl %eax, %r15d"),
            "narrowing must fire:\n{out}"
        );
        assert_eq!(out.matches("movslq").count(), 2, "asm was:\n{out}");
    }

    #[test]
    fn byte_copy_rewrite_clears_the_sext8_fact() {
        // The cross-family movzbl->movl rewrite redefines %ecx with a
        // zero-extended byte (0x80..0xFF possible): a stale sext8 fact
        // from the earlier movsbq into %rcx must not license rewriting
        // the following genuine sign extension into a plain copy.
        let out = run(concat!(
            "foo:\n",
            "    movzbq (%rdi), %rax\n",
            "    movsbq (%rsi), %rcx\n",
            "    movzbl %al, %ecx\n",
            "    movsbq %cl, %rdx\n",
        ));
        assert!(
            out.contains("movl %eax, %ecx"),
            "byte rewrite must fire:\n{out}"
        );
        assert!(out.contains("movsbq %cl, %rdx"), "asm was:\n{out}");
    }

    #[test]
    fn high_byte_sign_extension_is_never_rewritten() {
        // %ah is bits 8..15, not the low byte the sext8 fact describes.
        let out = run_maybe(concat!(
            "foo:\n",
            "    movsbq (%rdi), %rax\n",
            "    movsbq %ah, %rcx\n",
        ));
        assert!(out.contains("movsbq %ah, %rcx"), "asm was:\n{out}");
    }

    #[test]
    fn indexed_scale_sign_load_establishes_the_fact() {
        // The destination parse must be parenthesis-aware: with the old
        // rfind(',') split this producer was one operand-shape away from
        // silently keeping stale facts.
        let out = run(concat!(
            "foo:\n",
            "    movsbq (%rdi, %rcx, 4), %rax\n",
            "    movsbq %al, %rdx\n",
        ));
        assert!(out.contains("movq %rax, %rdx"), "asm was:\n{out}");
        assert_eq!(out.matches("movsbq").count(), 1, "asm was:\n{out}");
    }

    #[test]
    fn movzbw_does_not_establish_upper32_zero() {
        // SOUNDNESS: movzbw writes only %ax; bits 16..63 are stale. The old
        // `op.starts_with("movz")` producer marked the family upper-32-zero
        // and licensed narrowing the following 64-bit copy to movl.
        let out = run_maybe(concat!(
            "foo:\n",
            "    movzbw (%rdi), %ax\n",
            "    movq %rax, %rcx\n",
            "    xorl %ecx, %edx\n",
        ));
        assert!(out.contains("movq %rax, %rcx"), "asm was:\n{out}");
        assert!(!out.contains("movl %eax, %ecx"), "asm was:\n{out}");
    }
}
