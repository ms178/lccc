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

/// Whether one line of the window is provably unable to modify the
/// arithmetic/flag bits `cmovcc`/`setcc` read (OF/SF/ZF/AF/CF/PF).
///
/// Three rules carry the safety of this predicate, ordered so the structural
/// ones win over the prefix-matching ones:
///
/// 1. A **label definition ends the window**, before anything else is
///    consulted. Every mnemonic rule below matches on a prefix, and a label may
///    share a prefix with a neutral mnemonic (`.Lfb_cloned:` is a `.`-line,
///    `movsb_done:` is a `mov`-line). A branch can target any label, and
///    control entering the middle of the window would read the flags that the
///    deleted `popfq` was there to restore — so a label is never merely
///    positional. (The old order consulted the directive rule first and
///    returned "neutral" for every `.`-line, which silently exempted every
///    `.L`-prefixed label in the window.)
/// 2. A **dot-line is an assembler directive**, and a directive is neutral only
///    when it emits no bytes. `.byte 0xF8` sitting between the pair *is* a
///    `clc`, and `.zero 8` emits `0x00 0x00`, which decodes as
///    `add %al,(%rax)` — another flag writer. So classification is delegated to
///    [`super::helpers::directive_emits_data`], the table the flag-walking
///    peepholes already use; sharing it is the point, because two independent
///    lists of "harmless directives" drift apart and the narrower one then
///    silently deletes a pair across a byte that changes flags. Anything the
///    table does not name is data, so a malformed or unknown directive line
///    closes the window instead of opening a hole.
/// 3. **Everything else is admitted only by an explicit mnemonic rule**, and
///    anything not named is treated as flag-clobbering.
fn is_flag_neutral_line(m: &str) -> bool {
    let m = m.trim();
    if m.is_empty() || m.starts_with('#') || m.starts_with("//") || m.starts_with(';') {
        return true; // comment / blank
    }
    // Rule 1: a label definition (optionally followed by nothing but the
    // terminator) is a control-flow boundary.
    if m.ends_with(':') {
        return false;
    }
    // Rule 2: directives are neutral exactly when they carry no bytes.
    if m.starts_with('.') {
        return !super::helpers::directive_emits_data(m);
    }
    // Conditional/unconditional branches, calls, returns — control flow
    // boundary; must not be skipped over (a branch target inside the window
    // would make the popfq not necessarily match this pushfq).
    if m.starts_with('j') {
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
    // push/pop modify RSP but not flags — except the flag stack itself, which
    // `popf`-family writes. `popfq`/`popf` are the pair's terminator and are
    // handled by the caller; `popfl`/`popfw` are not, so they must not be
    // waved through by a bare `pop` prefix.
    if m.starts_with("popf") {
        return false;
    }
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
    if m.starts_with("xchg") {
        return true;
    }
    // `rep`-prefixed string operations split by the operation, not the prefix:
    // MOVS/STOS/LODS/OUTS leave EFLAGS alone, while CMPS and SCAS are flag
    // WRITERS (they set ZF every iteration, which is exactly how `repe cmpsb`
    // decides to stop). A bare `rep`, or a `rep` on an operation this rule does
    // not name, closes the window.
    if m.starts_with("rep") {
        let Some(word) = m.split_whitespace().nth(1) else {
            return false;
        };
        return word.starts_with("movs")
            || word.starts_with("stos")
            || word.starts_with("lods")
            || word.starts_with("outs");
    }
    // Anything else is conservatively flag-clobbering.
    false
}

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
            if is_flag_neutral_line(m) {
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
        assert_eq!(
            asm, input,
            "user bytes are immutable and the pair is unverifiable"
        );
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

    /// A directive inside the window is neutral only when it emits no bytes.
    /// `.byte 0xf8` *is* `clc` and `.zero 2` *is* `add %al,(%rax)` — both write
    /// flags — so a pair spanning them must survive. The predicate used to
    /// return "neutral" for every `.`-prefixed line.
    #[test]
    fn data_emitting_directive_in_the_window_blocks_elimination() {
        for directive in [
            "    .byte 0xf8",
            "    .zero 2",
            "    .space 4",
            "    .skip 4",
            "    .fill 1,1,0xf8",
            "    .long 0x90909090",
        ] {
            let input = format!("f:\n    pushfq\n{directive}\n    popfq\n    ret\n");
            let mut asm = input.clone();
            let changed = eliminate_redundant_pushfq(&mut asm);
            assert!(
                !changed && asm == input,
                "`{directive}` emits bytes the pair may not step over; asm became:\n{asm}"
            );
        }
    }

    /// Position-only directives stay harmless, so the fix costs no fold: this
    /// is the shape the compiler itself emits inside a staging window.
    #[test]
    fn benign_directive_in_the_window_still_elides_the_pair() {
        for directive in [
            "    .cfi_def_cfa_offset 16",
            "    .p2align 4",
            "    .loc 1 42 7",
        ] {
            let input =
                format!("f:\n    pushfq\n{directive}\n    movq %rbx, %rax\n    popfq\n    ret\n");
            let mut asm = input;
            assert!(
                eliminate_redundant_pushfq(&mut asm),
                "`{directive}` cannot affect flags; asm stayed:\n{asm}"
            );
        }
    }

    /// A label is a branch target, so control can enter the window at it and
    /// read the flags the deleted `popfq` would have restored. `.L`-prefixed
    /// labels used to be exempt, because the directive rule fired first.
    #[test]
    fn any_label_in_the_window_blocks_elimination() {
        for label in ["    .Ltmp7:", "    movsb_loop:", "    .L2:"] {
            let input =
                format!("f:\n    pushfq\n{label}\n    movq %rbx, %rax\n    popfq\n    ret\n");
            let mut asm = input.clone();
            let changed = eliminate_redundant_pushfq(&mut asm);
            assert!(
                !changed && asm == input,
                "`{label}` is an entry point into the window; asm became:\n{asm}"
            );
        }
    }

    /// `rep` splits by operation: MOVS/STOS/LODS/OUTS leave EFLAGS alone, while
    /// CMPS and SCAS are flag writers every iteration (that is how `repe`
    /// decides to stop).
    #[test]
    fn rep_prefix_is_judged_by_the_string_operation() {
        let neutral = "f:\n    pushfq\n    rep movsl %ds:(%rsi),%es:(%rdi)\n    popfq\n    ret\n";
        let mut asm = neutral.to_string();
        assert!(
            eliminate_redundant_pushfq(&mut asm),
            "`rep movsl` does not touch flags; asm stayed:\n{asm}"
        );
        for clobbers in [
            "    repe cmpsb %ds:(%rsi),%es:(%rdi)",
            "    repne scasb %al,(%rdi)",
            // An operation the rule does not name: "unrecognized" is not
            // "harmless", so a `rep`-prefixed line with no recognizable string
            // operation closes the window.
            "    rep",
        ] {
            let input = format!("f:\n    pushfq\n{clobbers}\n    popfq\n    ret\n");
            let mut asm = input.clone();
            let changed = eliminate_redundant_pushfq(&mut asm);
            assert!(
                !changed && asm == input,
                "`{clobbers}` writes flags; asm became:\n{asm}"
            );
        }
    }

    /// `popfl` is not the partner of a `pushfq` and restores the flags the pair
    /// is preserving; a bare `pop` prefix must not admit it.
    #[test]
    fn flag_stack_pop_is_not_mere_stack_pop() {
        let input = "f:\n    pushfq\n    popfl\n    movq %rbx, %rax\n    popfq\n    ret\n";
        let mut asm = input.to_string();
        let changed = eliminate_redundant_pushfq(&mut asm);
        assert!(!changed && asm == input, "asm became:\n{asm}");
    }
}
