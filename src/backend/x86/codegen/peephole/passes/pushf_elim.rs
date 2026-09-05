//! Always-on, provably-safe elimination of redundant `pushfq`/`popfq` pairs.
//!
//! `emit_select_impl` preserves the condition flags (ZF) across loading the
//! two select operands using `pushfq`/`popfq`. When the instructions between
//! the `pushfq` and its matching `popfq` do NOT modify any flag (pure
//! `mov`/`lea`/loads), the `pushfq`/`popfq` is a no-op and can be removed.
//!
//! `pushfq`/`popfq` are **serializing** instructions (~20-50 cycles each on
//! modern x86), so eliminating them is a significant win in hot loops.
//!
//! This pass is deliberately conservative: it only removes a pair when every
//! instruction between them is provably flag-neutral. It never reorders,
//! never removes conditional branches, and never touches anything it cannot
//! prove safe. It runs on every compilation (independent of `CCC_PEEPHOLE`).

/// Instructions whose mnemonics never modify the arithmetic/flag bits used by
/// `cmovcc`/`setcc` (OF/SF/ZF/AF/CF/PF). Everything else is treated as
/// flag-clobbering and will block elimination.
fn is_flag_neutral_mnemonic(m: &str) -> bool {
    let m = m.trim();
    if m.is_empty() || m.starts_with('#') || m.starts_with("//") || m.starts_with(';') {
        return true; // comment / blank
    }
    // Directives never affect the CPU flags.
    if m.starts_with('.') {
        return true;
    }
    // Conditional/unconditional branches, calls, returns, ret — control flow
    // boundary; must not be skipped over (a branch target inside the window
    // would make the popfq not necessarily match this pushfq).
    if m.starts_with('j') && (m.starts_with("jmp") || m.starts_with('j')) {
        // Any jcc (je, jne, jl, ...) or jmp terminates the linear window.
        return false;
    }
    if m.starts_with("call") || m.starts_with("ret") || m.starts_with("syscall") {
        return false;
    }
    // mov-family, lea, loads: none modify flags.
    if m.starts_with("mov") {
        return true;
    }
    if m.starts_with("lea") {
        return true;
    }
    if m.starts_with("movzbl") || m.starts_with("movz") {
        return true;
    }
    if m.starts_with("movsb") || m.starts_with("movsw") || m.starts_with("movslq") {
        return true;
    }
    // push/pop modify RSP but not flags.
    if m.starts_with("push") {
        return true;
    }
    if m.starts_with("pop") {
        return true;
    }
    if m.starts_with("nop") {
        return true;
    }
    if m.starts_with("set") {
        return true;
    } // setcc reads flags but doesn't change them
    if m.starts_with("cmov") {
        return true;
    } // cmovcc reads flags
    if m.starts_with("rep") {
        return true;
    }
    if m.starts_with("xchg") {
        return true;
    }
    // A bare label line ends the linear window (a branch could target it).
    if m.ends_with(':') {
        return false;
    }
    // Anything else is conservatively flag-clobbering.
    false
}

/// Returns true if a trimmed instruction line contains an operand whose
/// meaning would CHANGE if the surrounding `pushfq`/`popfq` were removed.
///
/// `pushfq` shifts RSP by 8 for the duration of the window, so only operands
/// whose address/value depends on RSP are unsafe:
///   * any explicit `%rsp`/`%esp` register (as base, index or data — a data
///     value of RSP differs by 8 inside the window),
///   * implicitly RSP-touching instructions: `push`/`pop`/`call`/`ret`.
///
/// Everything else is address-independent of the pushfq shift: RIP-relative
/// operands, RBP-relative memory operands (RBP is never changed by pushfq),
/// plain register operands (including `%rbp` as a data register in
/// frame-pointer-omitted code), absolute addresses, immediates.
fn has_rsp_dependent_operand(t: &str) -> bool {
    if t.contains("%rsp") || t.contains("%esp") {
        return true;
    }
    // push/pop/call/ret implicitly modify RSP — the stack slot the flags word
    // sits in would shift, so keep these conservative.
    let m = first_mnemonic(t);
    if m.starts_with("push")
        || m.starts_with("pop")
        || m.starts_with("call")
        || m.starts_with("ret")
    {
        return true;
    }
    false
}
fn first_mnemonic(line: &str) -> &str {
    let line = line.trim();
    // Take text up to first ';' that isn't a comment.
    match line.find(';') {
        Some(i) => &line[..i],
        None => line,
    }
}

/// Remove `pushfq` … `popfq` pairs where all instructions in between are
/// flag-neutral. Returns true if any line was removed.
///
/// A special case: `xorl %reg,%reg` (register self-zeroing) sets ZF=1, which
/// clobbers the preserved flag. It is rewritten to the flag-neutral
/// `movq $0,%reg` (identical 64-bit value for a self-xor) so the enclosing
/// `pushfq`/`popfq` can then be removed. This is safe because the flags set
/// by the xor are immediately overwritten by the `popfq` anyway (that is the
/// entire purpose of the `pushfq`/`popfq`), and the register value is the same.
pub(super) fn eliminate_redundant_pushfq(asm: &mut String) -> bool {
    if std::env::var("CCC_NO_PUSHF_ELIM").is_ok() {
        return false;
    }
    let mut lines: Vec<String> = asm.lines().map(|s| s.to_string()).collect();
    let mut keep = vec![true; lines.len()];
    let mut changed = false;

    let mut i = 0usize;
    // Inline-asm regions (`#APP` .. `#NO_APP`) are user-authored text: their
    // bytes must reach the assembler untouched, and their flag effects are
    // invisible to this textual scan (raw `.byte` encodings can clobber
    // flags; a template-internal `popfq` is NOT the partner of an emitted
    // `pushfq`).  A `pushfq`/`popfq` pair may therefore never be formed
    // across — or inside — a region, and no line inside a region may be
    // rewritten (the `xorl → movq $0` rewrite included).
    let mut in_asm = false;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed == "#APP" {
            in_asm = true;
            i += 1;
            continue;
        }
        if trimmed == "#NO_APP" {
            in_asm = false;
            i += 1;
            continue;
        }
        if in_asm || trimmed != "pushfq" && trimmed != "pushf" {
            i += 1;
            continue;
        }
        // Look ahead for the matching popfq, ensuring every line in between is
        // flag-neutral (or a rewrite-able self-xor), there is no label/branch,
        // and no operand whose address/value depends on RSP. In
        // `use_rsp_addressing` mode the emitted stack offsets between
        // pushfq/popfq assume RSP is shifted by 8 (the pushed flags word);
        // removing the pair would mis-address those slots. RBP-relative and
        // RIP-relative memory operands are NOT affected by the RSP shift, and
        // plain `%rbp` register operands (FPO data register) are fine too —
        // only RSP-dependent lines are rejected.
        let mut j = i + 1;
        let mut safe = true;
        let mut found_pop = false;
        while j < lines.len() {
            let t = lines[j].trim();
            if t == "#APP" {
                // The window would span into user asm: its flag effects are
                // unanalyzable and its bytes untouchable. Abort the pair.
                safe = false;
                break;
            }
            if t == "#NO_APP" {
                safe = false;
                break;
            }
            if t == "popfq" || t == "popf" {
                found_pop = true;
                break;
            }
            // Any RSP-dependent operand invalidates the transformation.
            if has_rsp_dependent_operand(t) {
                safe = false;
                break;
            }
            let m = first_mnemonic(&t);
            if is_flag_neutral_mnemonic(m) {
                j += 1;
                continue;
            }
            // Self-zeroing xor: `xorl %reg, %reg` — rewrite to flag-neutral mov.
            if let Some(reg) = parse_self_xor(&t) {
                let mov64 = map_reg_32_to_64(&reg);
                lines[j] = format!("    movq $0, %{}", mov64);
                changed = true;
                j += 1;
                continue;
            }
            safe = false;
            break;
        }
        if safe && found_pop {
            // Remove pushfq and popfq lines.
            keep[i] = false;
            keep[j] = false;
            changed = true;
            i = j + 1;
        } else {
            i += 1;
        }
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

/// Parse a `xorl %reg, %reg` (self-zeroing) instruction; return the register name.
fn parse_self_xor(line: &str) -> Option<String> {
    let line = line.trim();
    if !line.starts_with("xorl ") && !line.starts_with("xorq ") {
        return None;
    }
    // Expect form: xorl %reg, %reg
    let rest = &line[line.find(' ')? + 1..];
    let parts: Vec<&str> = rest.split(',').map(|s| s.trim()).collect();
    if parts.len() != 2 {
        return None;
    }
    let a = parts[0];
    let b = parts[1];
    // Normalize: strip leading '%'
    let na = a.trim_start_matches('%');
    let nb = b.trim_start_matches('%');
    if na != nb {
        return None;
    }
    Some(na.to_string())
}

/// Map a 32-bit register name to its 64-bit name for `movq $0, %reg`.
fn map_reg_32_to_64(reg: &str) -> &'static str {
    match reg {
        "eax" => "rax",
        "ebx" => "rbx",
        "ecx" => "rcx",
        "edx" => "rdx",
        "esi" => "rsi",
        "edi" => "rdi",
        "ebp" => "rbp",
        "esp" => "rsp",
        "r8d" => "r8",
        "r9d" => "r9",
        "r10d" => "r10",
        "r11d" => "r11",
        "r12d" => "r12",
        "r13d" => "r13",
        "r14d" => "r14",
        "r15d" => "r15",
        // Already a 64-bit name.
        "rax" => "rax",
        "rbx" => "rbx",
        "rcx" => "rcx",
        "rdx" => "rdx",
        "rsi" => "rsi",
        "rdi" => "rdi",
        "rbp" => "rbp",
        "rsp" => "rsp",
        "r8" => "r8",
        "r9" => "r9",
        "r10" => "r10",
        "r11" => "r11",
        "r12" => "r12",
        "r13" => "r13",
        "r14" => "r14",
        "r15" => "r15",
        // Unknown: default to rax (won't occur in generated self-xor zeroing).
        _ => "rax",
    }
}

#[cfg(test)]
mod tests {
    use super::eliminate_redundant_pushfq;

    #[test]
    fn pair_straddling_inline_asm_region_is_never_removed() {
        // The window between the emitted pushfq and popfq contains a user
        // region whose flag effects are unanalyzable (raw encodings); the
        // pair must survive, and the template's own `popfq` must never be
        // mistaken for the partner.
        let input = concat!(
            "f:\n",
            "    pushfq\n",
            "    #APP\n",
            "    popfq\n",
            "    #NO_APP\n",
            "    popfq\n",
            "    ret\n",
        );
        let mut asm = input.to_string();
        let changed = eliminate_redundant_pushfq(&mut asm);
        assert_eq!(asm, input, "user bytes are immutable and the pair is unverifiable");
        assert!(!changed);
    }

    #[test]
    fn template_internal_pushfq_is_not_a_candidate() {
        let input = concat!(
            "f:\n",
            "    #APP\n",
            "    pushfq\n",
            "    nop\n",
            "    popfq\n",
            "    #NO_APP\n",
            "    ret\n",
        );
        let mut asm = input.to_string();
        let _ = eliminate_redundant_pushfq(&mut asm);
        assert_eq!(asm, input, "nothing inside a region may be removed");
    }

    #[test]
    fn plain_flag_neutral_pair_is_still_removed() {
        let input = "f:\n    pushfq\n    movq %rbx, %rax\n    popfq\n    ret\n";
        let mut asm = input.to_string();
        assert!(eliminate_redundant_pushfq(&mut asm));
        assert!(!asm.contains("pushfq"), "asm was:\n{asm}");
        assert!(!asm.contains("popfq"), "asm was:\n{asm}");
    }
}
