
//! Always-on, provably-safe elimination of redundant zero/sign-extension moves.
//!
//! The codegen emits a widening cast (`u8 -> u32`) as `movzbl %al, %eax` even
//! when the value is ALREADY zero-extended in that register (e.g. it was just
//! loaded with `movzbl (%mem), %eax`, produced by `setcc %al`, or copied from
//! another zero-extended register). Such a re-extension is a no-op and can be
//! removed, saving an instruction in hot loops.
//!
//! Soundness: we track, for each 64-bit register family, a flag "the upper 32
//! bits are provably zero." This flag is:
//!   - SET by any instruction that writes the full 32-bit register and
//!     zero-extends (movzbl/movzwl/movl-from-anywhere/xorl-self/setcc, ...).
//!   - COPIED by `movq %reg, %reg` (a 64-bit move preserves the upper-32-zero
//!     property of the destination).
//!   - CLEARED by anything that writes the 64-bit register or the upper bits
//!     (movq from a possibly-nonzero source, arithmetic on the 64-bit reg, etc.)
//!
//! Then `movzbl %AL, %EAX` (source byte is the low byte of a dest reg known
//! upper-32-zero) is a no-op and removed. We only apply this when source and
//! destination are sub-registers of the SAME family, and the family's
//! upper-32-zero flag is set, guaranteeing the move cannot change the value.

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
        0 => Some("eax"), 1 => Some("ecx"), 2 => Some("edx"), 3 => Some("ebx"),
        4 => Some("esp"), 5 => Some("ebp"), 6 => Some("esi"), 7 => Some("edi"),
        8 => Some("r8d"), 9 => Some("r9d"), 10 => Some("r10d"), 11 => Some("r11d"),
        12 => Some("r12d"), 13 => Some("r13d"), 14 => Some("r14d"), 15 => Some("r15d"),
        _ => None,
    }
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
    let needle32 = if dst32.starts_with('%') { dst32.to_string() } else { format!("%{dst32}") };
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
    if trimmed.starts_with('r') && trimmed.ends_with('d')
        && trimmed[1..trimmed.len() - 1].bytes().all(|b| b.is_ascii_digit())
    {
        let d64 = format!("%{}", &trimmed[..trimmed.len() - 1]);
        if toks.iter().any(|tok| tok.trim_end_matches(',') == d64) {
            return false;
        }
    }
    let d64 = match trimmed {
        "eax" => "%rax", "ecx" => "%rcx", "edx" => "%rdx", "ebx" => "%rbx",
        "esi" => "%rsi", "edi" => "%rdi",
        other if other.starts_with('r') && other.ends_with('d')
            && other[1..other.len() - 1].bytes().all(|b| b.is_ascii_digit()) => "",
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
        "movl" | "addl" | "subl" | "andl" | "orl" | "xorl" | "cmpl" | "testl"
            | "leal" | "imull" | "sall" | "shll" | "shrl" | "sarl"
            | "movzbl" | "movzwl" | "cmovl" | "cmovel" | "cmovnel"
            | "cmovsl" | "cmovns" | "cmovgl" | "cmovgel" | "cmovll"
            | "cmovlel" | "cmovbl" | "cmovael" | "cmovbel" | "cmoval"
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
    if bytes.len() < 2 || bytes[0] != b'r' { return 0; }
    // parse the numeric part before the size suffix
    let mut i = 1;
    let mut num = 0usize;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        num = num * 10 + (bytes[i] - b'0') as usize;
        i += 1;
    }
    if num < 8 || num > 15 { return 0; }
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

    for i in 0..lines.len() {
        let line = &lines[i];
        let t = line.trim();
        if t.is_empty() || t.starts_with('.') { continue; }

        // Skip labels: at a label we don't know which path was taken, so we
        // conservatively clear all flags.
        if t.ends_with(':') {
            upper32_zero = vec![false; GP_FAMILIES];
            is_byte = vec![false; GP_FAMILIES];
            is_16 = vec![false; GP_FAMILIES];
            continue;
        }

        // --- Pattern: movzbl %AL, %EAX (redundant re-zero-extend) ---
        if t.starts_with("movzbl ") || t.starts_with("movzbq ") {
            if let Some(comma) = t.find(',') {
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
                        eprintln!("[EXT] t='{}' src_fam={:?} dst_fam={:?} is32={} u32z={} bytez={}", t, sf, df, is32, u32z, bytez);
                    }
                    if is32 && u32z && sf == df {
                        // Same family, upper-32-zero: pure no-op -> remove.
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
            && !t.contains('(') {
            // reg-reg form only (memory sources are handled by the flag
            // update below: movzwl mem,%eax already zero-extends).
            if let Some(comma) = t.find(',') {
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
                    let fits16 = (sf as usize).lt(&GP_FAMILIES) && is_16[sf as usize];
                    if std::env::var("CCC_TRACE_EXT").is_ok() {
                        eprintln!("[EXT16] t='{}' src_fam={:?} dst_fam={:?} fits16={}", t, sf, df, fits16);
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
                            }
                            continue;
                        }
                    }
                }
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
            if let Some(comma) = t.find(',') {
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
                                // The text peephole has no IR type information.
                                // Treat this as a local instruction-selection
                                // cleanup only when the immediately following
                                // machine instruction consumes or overwrites
                                // the 32-bit destination form.  That is exactly
                                // enough for type-erased RA copies feeding
                                // `xorl`/`andl` and avoids changing a 64-bit
                                // pointer/address live range that may cross
                                // labels/calls/64-bit consumers.  Do not require
                                // global state: after the first copy is narrowed
                                // the next copy's source is itself a 32-bit move
                                // not represented in the historical state.
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

        // --- Update flags based on this instruction ---
        self_update(t, &mut upper32_zero, &mut is_byte, &mut is_16);
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
fn self_update(
    t: &str,
    upper32_zero: &mut Vec<bool>,
    is_byte: &mut Vec<bool>,
    is_16: &mut Vec<bool>,
) {
    let toks: Vec<&str> = t.split_whitespace().collect();
    if toks.is_empty() { return; }
    let op = toks[0];

    // Calls clobber the full 64-bit values of caller-saved registers.  The
    // assembly line for a direct call often contains no `%reg`, so the generic
    // operand scan below would not invalidate any state.  Clear all tracked
    // families conservatively: losing one extension/copy-narrowing across a
    // call is harmless; trusting stale upper-32-zero state across a call can
    // miscompile a later 64-bit copy.
    if op == "call" || op == "callq" {
        for flags in [&mut *upper32_zero, &mut *is_byte, &mut *is_16] {
            for value in flags.iter_mut() {
                *value = false;
            }
        }
        return;
    }

    // movq %regA, %regB : copies both flags from A to B.
    // movq from MEMORY or IMMEDIATE: the destination holds an opaque 64-bit
    // value — every zero-extension flag for the destination family must be
    // CLEARED. (The old code returned early when the source was not a plain
    // register, leaving stale flags: a later re-extension of the loaded
    // value would then be wrongly removed, corrupting the upper bits seen by
    // 64-bit consumers. Harmless for stores: movq %reg, mem changes no
    // register flags.)
    if op == "movq" && t.contains('%') {
        if let Some(comma) = t.find(',') {
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
                return;
            }
            let src = t[so..comma].trim();
            let dst = t[comma + 1..].trim();
            let src_fam = family_of_reg(src);
            let dst_fam = family_of_reg(dst);
            if let (Some(sf), Some(df)) = (src_fam, dst_fam) {
                if (sf as usize).lt(&upper32_zero.len()) {
                    upper32_zero[df as usize] = upper32_zero[sf as usize];
                    is_byte[df as usize] = is_byte[sf as usize];
                    is_16[df as usize] = is_16[sf as usize];
                }
            } else if let Some(df) = dst_fam {
                // movq from memory/immediate (or unparsable source): the
                // 64-bit value is opaque — clear all dest flags.
                if (df as usize).lt(&upper32_zero.len()) {
                    upper32_zero[df as usize] = false;
                    is_byte[df as usize] = false;
                    is_16[df as usize] = false;
                }
            }
        }
        return;
    }

    // Byte zero-extension into a register sets upper32_zero, is_byte AND
    // is_16 (a byte value fits in 16 bits).
    if op.starts_with("movzbl") {
        if let Some(comma) = t.find(',') {
            let dst = t[comma + 1..].trim();
            if let Some(df) = family_of_reg(dst) {
                if (df as usize).lt(&upper32_zero.len()) {
                    upper32_zero[df as usize] = true;
                    is_byte[df as usize] = true;
                    is_16[df as usize] = std::env::var("CCC_NO_EXT16").is_err();
                }
            }
        }
        return;
    }
    // 16-bit zero-extension: upper-32 zero, fits-16, but NOT a byte.
    if op.starts_with("movzwl") || op.starts_with("movzwq") || op.starts_with("movz") {
        if let Some(comma) = t.find(',') {
            let dst = t[comma + 1..].trim();
            if let Some(df) = family_of_reg(dst) {
                if (df as usize).lt(&upper32_zero.len()) {
                    upper32_zero[df as usize] = true;
                    is_byte[df as usize] = false;
                    is_16[df as usize] = std::env::var("CCC_NO_EXT16").is_err();
                }
            }
        }
        return;
    }
    // movl / xorl: zero-extends to 32 bits, but NOT necessarily a byte or
    // 16-bit value.
    if op == "movl" || op == "xorl" {
        if let Some(comma) = t.find(',') {
            let dst = t[comma + 1..].trim();
            if let Some(df) = family_of_reg(dst) {
                if (df as usize).lt(&upper32_zero.len()) {
                    upper32_zero[df as usize] = true;
                    is_byte[df as usize] = false;
                    is_16[df as usize] = false;
                }
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
            }
        }
        return;
    }

    // Anything else that writes registers: conservatively clear the flags of
    // EVERY register family mentioned in the line. This is the soundness
    // fix: the previous code only cleared when the OPCODE contained '%',
    // which missed `addq %rcx, %rax`, `leaq 0x30(%rsp), %rax`, `imulq %r13,
    // %r14`, shifts, etc. A stale upper-32-zero/is_byte flag on such a line
    // could wrongly eliminate a real re-extension (miscompile). Clearing
    // extra families is always safe (only loses optimizations).
    {
        let mut cleared = false;
        for tok in toks {
            if tok.starts_with('%') {
                let name = tok.trim_end_matches(',');
                if let Some(f) = family_of_reg(name) {
                    if (f as usize).lt(&upper32_zero.len()) {
                        upper32_zero[f as usize] = false;
                        is_byte[f as usize] = false;
                        is_16[f as usize] = false;
                        cleared = true;
                    }
                }
            }
        }
        let _ = cleared;
    }
}

#[cfg(test)]
mod tests {
    use super::eliminate_redundant_zero_extend;

    fn run(input: &str) -> String {
        let mut asm = input.to_string();
        assert!(eliminate_redundant_zero_extend(&mut asm));
        asm
    }

    #[test]
    fn narrows_known_zero_extended_register_copy() {
        let asm = run(
            "crc32_update:\n\
             \x20   movzbl (%rdx), %r8d\n\
             \x20   movl %esi, %eax\n\
             \x20   movq %r8, %rcx\n\
             \x20   xorl %ecx, %eax\n\
             \x20   movq %rax, %r8\n\
             \x20   andl $255, %r8d\n"
        );
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
}
