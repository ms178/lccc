//! Shared GAS assembly preprocessing utilities.
//!
//! Functions used by multiple assembler parsers (ARM, RISC-V, x86) for
//! text-level preprocessing before architecture-specific parsing:
//!
//! - C-style comment stripping (`/* ... */`)
//! - Line comment stripping (`#`, `//`, `@`)
//! - Semicolon splitting (GAS statement separator)
//! - `.rept`/`.irp`/`.endr` block expansion
//! - `.macro`/`.endm`/`.purgem` definition, expansion, and removal
//! - `.if`/`.elseif`/`.else`/`.endif` conditional assembly evaluation

use crate::backend::asm_expr;

// ── Comment handling ───────────────────────────────────────────────────

/// Characters that start a line comment for each architecture.
///
/// Used by `strip_comment` to determine where comments begin.
/// Each architecture may have multiple comment styles.
pub enum CommentStyle {
    /// `#` only (x86/x86-64 AT&T syntax)
    Hash,
    /// `#` and `//` (RISC-V GAS)
    HashAndSlashSlash,
    /// `//` and `@` (the ARM assembler's macro expansion) -- but `@` is not a
    /// comment before type specifiers like `@function`, `@object`,
    /// `@progbits`, `@nobits`, `@tls_object`, `@note`, nor in the macro
    /// counter `\@`.
    SlashSlashAndAt,
}

/// Strip C-style `/* ... */` comments from assembly text, handling multi-line spans.
/// Preserves newlines inside comments so line numbers remain correct for error messages.
/// String-aware: does not strip `/* */` inside quoted string literals (e.g. `.asciz` data).
pub fn strip_c_comments(text: &str) -> String {
    // Copies whole byte runs instead of mapping byte-by-byte: `bytes[i] as char`
    // would re-encode every UTF-8 byte as a Latin-1 code point, so an asm symbol
    // named `café` or `€` would reach the symbol table as mojibake.
    let bytes = text.as_bytes();
    let mut result = String::with_capacity(bytes.len());
    let mut i = 0;
    let mut run = 0; // start of the byte run not yet copied
    let mut in_string = false;
    while i < bytes.len() {
        if in_string {
            match bytes[i] {
                b'\\' => {
                    // Skip the escaped byte (a UTF-8 continuation is skipped
                    // whole so the string state stays in sync).
                    i += 1;
                    if i < bytes.len() {
                        i += utf8_char_len(bytes[i]);
                    }
                }
                b'"' | b'\n' => {
                    in_string = false;
                    i += 1;
                }
                _ => i += utf8_char_len(bytes[i]),
            }
            continue;
        }
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            result.push_str(&text[run..i]);
            i += 2;
            while i < bytes.len() {
                if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    i += 2;
                    break;
                }
                if bytes[i] == b'\n' {
                    result.push('\n');
                }
                i += 1;
            }
            run = i;
            continue;
        }
        if bytes[i] == b'"' {
            in_string = true;
        }
        i += utf8_char_len(bytes[i]);
    }
    result.push_str(&text[run..]);
    result
}

/// Copy the character starting at `*i` into `out` and advance `*i` past it.
///
/// Replacing `out.push(bytes[i] as char)` with this keeps non-ASCII text
/// intact: a raw byte pushed as a `char` is re-encoded as Latin-1, so an
/// assembler symbol such as `café` or `€` (legal in GAS and emitted by the C
/// front end for extended identifiers) came out as mojibake and stopped
/// resolving. `*i` must sit on a UTF-8 boundary, which holds because every
/// step either consumes a whole character or an ASCII match.
pub fn push_char_at(out: &mut String, text: &str, i: &mut usize) {
    match text[*i..].chars().next() {
        Some(ch) => {
            out.push(ch);
            *i += ch.len_utf8();
        }
        None => *i = text.len(),
    }
}

/// Length in bytes of the UTF-8 sequence led by `lead` (1 for ASCII or a
/// continuation byte, so a lenient scan never stalls on malformed input).
fn utf8_char_len(lead: u8) -> usize {
    match lead {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

/// Strip trailing line comment from a single line, respecting string literals.
///
/// Scans character by character tracking quote state so that comment characters
/// inside `"..."` strings are not treated as comment starts. Handles escaped
/// quotes (`\"`) correctly.
pub fn strip_comment<'a>(line: &'a str, style: &CommentStyle) -> &'a str {
    let bytes = line.as_bytes();
    let mut in_string = false;
    let mut i = 0;
    while i < bytes.len() {
        if in_string {
            if bytes[i] == b'\\' {
                i += 1;
                if i < bytes.len() {
                    i += std::str::from_utf8(&bytes[i..])
                        .ok()
                        .and_then(|rest| rest.chars().next())
                        .map_or(1, char::len_utf8);
                }
                continue;
            }
            if bytes[i] == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        // Not in string
        if bytes[i] == b'"' {
            in_string = true;
            i += 1;
            continue;
        }
        if bytes[i] == b'\'' {
            i = crate::backend::asm_expr::skip_gas_char_literal(bytes, i);
            continue;
        }
        match style {
            CommentStyle::Hash => {
                if bytes[i] == b'#' {
                    return &line[..i];
                }
            }
            CommentStyle::HashAndSlashSlash => {
                if bytes[i] == b'#' {
                    return &line[..i];
                }
                if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    return &line[..i];
                }
            }
            CommentStyle::SlashSlashAndAt => {
                if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    return &line[..i];
                }
                if bytes[i] == b'@' && !(i > 0 && bytes[i - 1] == b'\\') {
                    // `\@` is GAS's macro invocation counter, not a comment.
                    // `@` is NOT a comment before type specifiers used in GAS directives
                    let after = &line[i + 1..];
                    if !after.starts_with("object")
                        && !after.starts_with("function")
                        && !after.starts_with("progbits")
                        && !after.starts_with("nobits")
                        && !after.starts_with("tls_object")
                        && !after.starts_with("note")
                    {
                        return &line[..i];
                    }
                }
            }
        }
        i += 1;
    }
    line
}

/// Split a line on `;` characters, respecting string literals, GAS character
/// literals, and comments.
/// In GAS syntax, `;` separates multiple statements on the same line.
/// Stops splitting once a line comment (`#` or `//`) is encountered outside strings,
/// so semicolons inside comments are not treated as statement separators.
pub fn split_on_semicolons(line: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut in_string = false;
    let mut start = 0usize;
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if in_string {
            if bytes[i] == b'\\' {
                i += 1;
                if i < bytes.len() {
                    i += std::str::from_utf8(&bytes[i..])
                        .ok()
                        .and_then(|rest| rest.chars().next())
                        .map_or(1, char::len_utf8);
                }
                continue;
            }
            if bytes[i] == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match bytes[i] {
            b'"' => {
                in_string = true;
                i += 1;
            }
            b'\'' => {
                i = crate::backend::asm_expr::skip_gas_char_literal(bytes, i);
            }
            b'#' => break,
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => break,
            b';' => {
                parts.push(&line[start..i]);
                i += 1;
                start = i;
            }
            _ => {
                i += 1;
            }
        }
    }
    parts.push(&line[start..]);
    parts
}

// ── .rept / .irp block expansion ───────────────────────────────────────

fn is_rept_start(trimmed: &str) -> bool {
    trimmed.starts_with(".rept ") || trimmed.starts_with(".rept\t")
}

fn is_irp_start(trimmed: &str) -> bool {
    trimmed.starts_with(".irp ") || trimmed.starts_with(".irp\t")
}

fn is_block_start(trimmed: &str) -> bool {
    is_rept_start(trimmed) || is_irp_start(trimmed)
}

/// Collect the body lines of a `.rept`/`.irp` block, handling nesting.
/// Returns the body lines and advances `i` past the closing `.endr`.
fn collect_block_body<'a>(
    lines: &[&'a str],
    i: &mut usize,
    comment_style: &CommentStyle,
) -> Result<Vec<&'a str>, String> {
    let mut depth = 1;
    let mut body = Vec::new();
    *i += 1;
    while *i < lines.len() {
        let inner = strip_comment(lines[*i], comment_style).trim().to_string();
        if is_block_start(&inner) {
            depth += 1;
        } else if inner == ".endr" {
            depth -= 1;
            if depth == 0 {
                break;
            }
        }
        body.push(lines[*i]);
        *i += 1;
    }
    if depth != 0 {
        return Err(".rept/.irp without matching .endr".to_string());
    }
    Ok(body)
}

/// Estimate the byte size of a single assembly line for label position tracking.
/// Used to resolve backward label references in .rept count expressions.
/// `default_insn_size` is the typical instruction size for the target (4 for ARM/RISC-V).
fn estimate_line_bytes_generic(
    trimmed: &str,
    comment_style: &CommentStyle,
    default_insn_size: u64,
) -> u64 {
    if trimmed.is_empty() {
        return 0;
    }
    // Check for comment-only lines
    match comment_style {
        CommentStyle::Hash => {
            if trimmed.starts_with('#') {
                return 0;
            }
        }
        CommentStyle::HashAndSlashSlash => {
            if trimmed.starts_with('#') || trimmed.starts_with("//") {
                return 0;
            }
        }
        CommentStyle::SlashSlashAndAt => {
            if trimmed.starts_with("//") || trimmed.starts_with('@') {
                return 0;
            }
        }
    }
    // Label definitions don't add bytes
    if trimmed.ends_with(':') && !trimmed.contains(' ') {
        return 0;
    }
    // Strip leading labels like "661:" from lines like "661: bl foo"
    let content = if let Some(pos) = trimmed.find(':') {
        let before = &trimmed[..pos];
        if before
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        {
            trimmed[pos + 1..].trim()
        } else {
            trimmed
        }
    } else {
        trimmed
    };
    if content.is_empty() {
        return 0;
    }
    // Directives
    if content.starts_with('.') {
        let lower = content.to_lowercase();
        if lower.starts_with(".byte ") {
            return 1;
        }
        if lower.starts_with(".hword ")
            || lower.starts_with(".short ")
            || lower.starts_with(".2byte ")
        {
            return 2;
        }
        if lower.starts_with(".word ")
            || lower.starts_with(".long ")
            || lower.starts_with(".4byte ")
            || lower.starts_with(".inst ")
        {
            return 4;
        }
        if lower.starts_with(".quad ")
            || lower.starts_with(".xword ")
            || lower.starts_with(".8byte ")
        {
            return 8;
        }
        // .zero N, .space N, .skip N
        if lower.starts_with(".zero ")
            || lower.starts_with(".space ")
            || lower.starts_with(".skip ")
        {
            let arg = content.split_whitespace().nth(1).unwrap_or("0");
            if let Ok(n) = arg.trim_end_matches(',').parse::<u64>() {
                return n;
            }
        }
        // Other directives (.align, .section, .globl, .type, .ascii, etc.) — 0 bytes
        return 0;
    }
    // Everything else is an instruction
    default_insn_size
}

/// Resolve backward numeric label references (like 662b, 661b) in a .rept count expression.
/// Substitutes each backward reference with its byte position, then evaluates the expression.
fn resolve_rept_label_expr(
    count_str: &str,
    label_positions: &crate::common::fx_hash::FxHashMap<String, Vec<u64>>,
    parse_int: fn(&str) -> Result<i64, String>,
) -> Result<i64, String> {
    // First try direct evaluation (handles simple integer expressions)
    if let Ok(val) = parse_int(count_str) {
        return Ok(val);
    }

    // Check if expression contains backward label references (e.g., 662b)
    let mut resolved = count_str.to_string();
    let mut found_label_ref = false;

    // Find and replace backward label references: digits followed by 'b' or 'B'
    loop {
        let mut replaced = false;
        let bytes = resolved.as_bytes();
        let len = bytes.len();
        let mut i = 0;
        while i < len {
            if bytes[i].is_ascii_digit() {
                let start = i;
                while i < len && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i < len && (bytes[i] == b'b' || bytes[i] == b'B') {
                    let after_ok = i + 1 >= len || !bytes[i + 1].is_ascii_alphanumeric();
                    // Avoid matching binary literals like 0b1010
                    let is_binary = start + 1 == i && bytes[start] == b'0';
                    if after_ok && !is_binary {
                        let label_num = &resolved[start..i];
                        let ref_end = i + 1;
                        if let Some(positions) = label_positions.get(label_num) {
                            if let Some(&pos) = positions.last() {
                                let before = &resolved[..start];
                                let after = &resolved[ref_end..];
                                resolved = format!("{}{}{}", before, pos, after);
                                found_label_ref = true;
                                replaced = true;
                                break;
                            }
                        }
                    }
                }
            }
            i += 1;
        }
        if !replaced {
            break;
        }
    }

    if found_label_ref {
        parse_int(&resolved)
    } else {
        Err(format!("cannot evaluate .rept count: {}", count_str))
    }
}

// ── .rept expansion budget (GAS read.c `do_repeat` law) ────────────────

/// Single-block `.rept` bound, mirroring GAS 2.47 `gas/read.c::do_repeat`:
/// the count is rejected when `count * effective_body_len` overflows or
/// exceeds `0xffffffff`.  Measured on both pinned targets:
/// `.rept -1` reports `excessive count 18446744073709551615 for REPT -
/// ignored` (the expression is cast to an unsigned `size_t`), `.rept 613566757`
/// over a `.byte 0` body reports the same wording, `.rept 0` and `.rept 1`
/// assemble, and `.rept 1<<64` only warns (shift range) and expands nothing.
///
/// GAS's body length includes a source-position `linefile` prefix whose
/// size tracks the input file name (27-40 bytes in the probe fixtures),
/// so the fixed 64-byte margin below stands in for it.  The margin keeps
/// our verdict on the *strict* side of GAS's for any path shorter than
/// ~49 bytes: we may reject a count slightly below GAS's threshold, but
/// never accept one GAS rejects — which is the side where the expansion
/// is attempted and the process dies allocating.
pub fn check_rept_budget(count: u64, body: &[&str]) -> Result<(), String> {
    if count == 0 {
        return Ok(());
    }
    // effective length = Σ (content + newline + eol byte) + header newline
    // + linefile margin, i.e. the measured GAS formula with the path term
    // replaced by 64.
    let mut eff: u64 = 1 + 64;
    for line in body {
        eff = eff.saturating_add(line.len() as u64 + 2);
    }
    match count.checked_mul(eff) {
        Some(total) if total <= u64::from(u32::MAX) => Ok(()),
        _ => Err(format!("excessive count {count} for REPT - ignored")),
    }
}

/// What repetition may ADD to the expanded output before the writer ever
/// sees it.  GAS's own bound hands back free-memory state once the
/// expanded text stops fitting (a `.rept` that passes `do_repeat` still
/// materializes up to ~4 GiB of text), and our line-vector representation
/// costs several times the raw text, so both numbers are deterministic
/// ceilings in the same class as [`crate::backend::elf::MAX_DIRECTIVE_FILL`]:
/// verdicts beyond them are not pinnable anyway, and refusing before the
/// allocation is the only reproducible answer.
pub const MAX_EXPANSION_TEXT: usize = crate::backend::elf::MAX_DIRECTIVE_FILL;

/// Line-count half of the expansion ceiling: a 4 MiB body of single-byte
/// lines is far cheaper as text than as a `Vec<String>`, so text bytes
/// alone do not bound the allocation the repetition makes.
pub const MAX_EXPANSION_LINES: usize = 4 << 20;

/// Gate a repetition's cumulative growth (running totals for the whole
/// expansion call, so nested and sequential blocks cannot add up past the
/// ceilings between them).
pub fn check_expansion_growth(
    kind: &str,
    grown_lines: u64,
    grown_bytes: u64,
) -> Result<(), String> {
    if grown_bytes > MAX_EXPANSION_TEXT as u64 {
        return Err(format!(
            "{kind}: expansion exceeds the {MAX_EXPANSION_TEXT}-byte limit"
        ));
    }
    if grown_lines > MAX_EXPANSION_LINES as u64 {
        return Err(format!(
            "{kind}: expansion exceeds the {MAX_EXPANSION_LINES}-line limit"
        ));
    }
    Ok(())
}

/// Lines and text bytes a finished expansion adds, for the growth totals.
pub fn expansion_size(lines: &[String]) -> (u64, u64) {
    let bytes: u64 = lines.iter().map(|l| l.len() as u64 + 1).sum();
    (lines.len() as u64, bytes)
}

/// Text materialized by one expansion run, summed over every frame.
///
/// This is the single home of the law.  The x86 front end's `ExpansionBudget`
/// is a thin wrapper over it (it additionally carries the frame counter its
/// recursion needs): three copies of "saturate, then compare against
/// `check_expansion_growth`" is how one of them ends up forgetting a ceiling,
/// and the `.macro` path arriving at this ceiling at all is the reason the
/// wrapper delegates instead of reimplementing.
///
/// `.rept` and `.irp` can predict their size from a literal count and refuse
/// before emitting anything; a `.macro` body that invokes macros has no such
/// handle, and depth alone does not bound it either: GAS allows 101 live
/// frames, and a body that invokes macros twice per frame doubles at each
/// level, so the legal recursion ceiling admits an astronomically large
/// output.  Charging the *total* across frames is what turns that into a
/// refusal, and it is the same ceiling `.rept` already enforces -- so a macro
/// and a repeat of the same size cost the same, and neither is a way around
/// the other.  "Same size" means the same *charged* size, which at nesting
/// depth 1 is the materialized text: a macro whose body repeats another macro
/// is billed for its inner lines twice (`nested_macro_output_is_billed_twice`),
/// so the same text through a nested frame crosses the ceiling earlier than a
/// flat repeat of it.  That asymmetry is in the safe direction -- it can only
/// refuse more, never less -- and `same_size_macro_and_rept_share_the_ceiling`
/// pins both halves of the sentence rather than leaving it as prose.
///
/// A per-frame budget would be wrong in the obvious way (101 frames each
/// allowed 4M lines is 400M), which is why the counter is threaded through the
/// recursion rather than created inside it.
#[derive(Clone, Copy, Default)]
pub struct ExpansionTotals {
    pub lines: u64,
    pub bytes: u64,
}

impl ExpansionTotals {
    /// Charge one expansion's output against the run, refusing once the
    /// ceiling is crossed.  Saturating on purpose: a run that would overflow
    /// `u64` is far past every ceiling already, and an arithmetic panic would
    /// replace a diagnostic with a crash.
    ///
    /// **Billing law.** One charge is the output of one expansion, so the output
    /// of a *nested* `.macro` expansion is billed twice: the inner frame charges
    /// its own result, and the outer frame then charges the text it received --
    /// which already contains those lines.  `.rept`/`.irp` need no such doubling
    /// because a parent's growth is `count x expanded_size`, i.e. the child's
    /// repetition is already inside the number the parent multiplies, and
    /// `.rept` checks it in its own frame rather than in a threaded accumulator.
    /// The asymmetry is deliberate: both rules charge at least what the pass
    /// materializes, which is the direction a ceiling must err in (measured in
    /// the round-8 oracle run, a legal 4.0 M-line `.macro` file costs LCCC 8.2 s
    /// and GAS 0 s, so there is nothing to win by counting *less*).  Unifying the
    /// two passes to a single shared total would also be wrong, not merely
    /// different: the `.rept` pass runs before the `.macro` pass on the same
    /// input, so its growth is legitimately charged twice in wall-clock terms
    /// (once per pass) and a shared counter would refuse files both passes can
    /// handle.  `nested_macro_output_is_billed_twice` pins the arithmetic.
    pub fn charge(&mut self, kind: &str, lines: &[String]) -> Result<(), String> {
        let (added_lines, added_bytes) = expansion_size(lines);
        self.lines = self.lines.saturating_add(added_lines);
        self.bytes = self.bytes.saturating_add(added_bytes);
        check_expansion_growth(kind, self.lines, self.bytes)
    }
}

/// Expand `.rept`/`.endr` and `.irp`/`.endr` blocks by repeating or
/// substituting contained lines.
///
/// Handles nested blocks and recursive expansion. Uses `parse_int` to
/// evaluate the `.rept` count expression.
/// Tracks numeric label byte positions to resolve backward label references
/// in `.rept` count expressions (e.g., `.rept (662b-661b)/4`).
pub fn expand_rept_blocks(
    lines: &[&str],
    comment_style: &CommentStyle,
    parse_int: fn(&str) -> Result<i64, String>,
) -> Result<Vec<String>, String> {
    expand_rept_blocks_with_insn_size(lines, comment_style, parse_int, 4)
}

/// Same as `expand_rept_blocks` but with configurable instruction size for byte estimation.
pub(crate) fn expand_rept_blocks_with_insn_size(
    lines: &[&str],
    comment_style: &CommentStyle,
    parse_int: fn(&str) -> Result<i64, String>,
    default_insn_size: u64,
) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    let mut i = 0;
    let mut label_positions: crate::common::fx_hash::FxHashMap<String, Vec<u64>> =
        crate::common::fx_hash::FxHashMap::default();
    let mut current_byte_pos: u64 = 0;
    // Cumulative growth (repetition-only; plain input lines are bounded by
    // the input the caller already holds in memory).
    let mut grown_lines: u64 = 0;
    let mut grown_bytes: u64 = 0;
    while i < lines.len() {
        let trimmed = strip_comment(lines[i], comment_style).trim().to_string();
        if is_rept_start(&trimmed) {
            let count_str = trimmed[".rept".len()..].trim();
            let count_val = resolve_rept_label_expr(count_str, &label_positions, parse_int)
                .map_err(|e| format!(".rept: bad count '{}': {}", count_str, e))?;
            // GAS casts the count expression to `size_t` (read.c `s_rept`):
            // a negative count wraps to a huge unsigned value and is then
            // rejected by `check_rept_budget` with GAS's own wording — not
            // silently collapsed to zero repetitions (measured: `.rept -1`
            // -> "excessive count 18446744073709551615 for REPT - ignored").
            let count = count_val as u64;
            let body = collect_block_body(lines, &mut i, comment_style)?;
            // Bound the raw block BEFORE recursing, exactly where GAS
            // checks it: an astronomic count fails in O(body) time instead
            // of allocating.
            check_rept_budget(count, &body)?;
            let expanded_body = expand_rept_blocks_with_insn_size(
                &body,
                comment_style,
                parse_int,
                default_insn_size,
            )?;
            let (add_lines, add_bytes) = expansion_size(&expanded_body);
            grown_lines = grown_lines.saturating_add(count.saturating_mul(add_lines));
            grown_bytes = grown_bytes.saturating_add(count.saturating_mul(add_bytes));
            check_expansion_growth(".rept", grown_lines, grown_bytes)?;
            for _ in 0..count {
                result.extend(expanded_body.iter().cloned());
            }
        } else if is_irp_start(&trimmed) {
            // .irp var, val1, val2, ...
            let args_str = trimmed[".irp".len()..].trim();
            let (var, values_str) = match args_str.find(',') {
                Some(pos) => (args_str[..pos].trim(), args_str[pos + 1..].trim()),
                None => (args_str, ""),
            };
            let values: Vec<&str> = values_str.split(',').map(|s| s.trim()).collect();
            let body = collect_block_body(lines, &mut i, comment_style)?;
            for val in &values {
                let subst_body: Vec<String> = body
                    .iter()
                    .map(|line| {
                        let pattern = format!("\\{}", var);
                        let substituted = replace_macro_param(line, &pattern, val);
                        // Strip GAS macro argument delimiters: \() resolves to empty string
                        substituted.replace("\\()", "")
                    })
                    .collect();
                let subst_refs: Vec<&str> = subst_body.iter().map(|s| s.as_str()).collect();
                let expanded = expand_rept_blocks_with_insn_size(
                    &subst_refs,
                    comment_style,
                    parse_int,
                    default_insn_size,
                )?;
                let (add_lines, add_bytes) = expansion_size(&expanded);
                grown_lines = grown_lines.saturating_add(add_lines);
                grown_bytes = grown_bytes.saturating_add(add_bytes);
                check_expansion_growth(".irp", grown_lines, grown_bytes)?;
                result.extend(expanded);
            }
        } else if trimmed == ".endr" {
            // stray .endr without .rept — skip
        } else {
            // Track numeric label definitions and byte positions
            if let Some(colon_pos) = trimmed.find(':') {
                let before = &trimmed[..colon_pos];
                if !before.is_empty() && before.chars().all(|c| c.is_ascii_digit()) {
                    label_positions
                        .entry(before.to_string())
                        .or_default()
                        .push(current_byte_pos);
                }
            }
            current_byte_pos +=
                estimate_line_bytes_generic(&trimmed, comment_style, default_insn_size);
            result.push(lines[i].to_string());
        }
        i += 1;
    }
    Ok(result)
}

// ── .macro / .endm expansion ───────────────────────────────────────────

/// One `.macro` parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MacroParam {
    pub(crate) name: String,
    /// `name=default`, used when the invocation omits the argument.
    pub(crate) default: Option<String>,
    /// `name:req`: omitting the argument is an error.
    pub(crate) required: bool,
    /// `name:vararg`: takes every remaining positional argument.
    pub(crate) vararg: bool,
}

/// A `.macro` definition.
pub(crate) struct MacroDef {
    /// As written in `.macro`, for diagnostics.
    name: String,
    params: Vec<MacroParam>,
    body: Vec<String>,
}

impl MacroDef {
    /// Bind an invocation's arguments (as comma fields of whitespace-separated
    /// tokens, see [`split_macro_arg_fields`]) to the parameters, the way GAS
    /// 2.47 does -- every rule below was measured on the pinned oracle:
    ///
    /// * positional arguments fill parameters left to right, and one past the
    ///   last parameter is "too many positional arguments";
    /// * `param=value` binds by name (whitespace around `=` is closed by
    ///   [`split_macro_arg_fields`]); a keyword naming no parameter is an error, a
    ///   repeated keyword takes the last value, and a positional argument
    ///   after a keyword is "can't mix positional and keyword arguments";
    /// * quotes around a keyword value are stripped, as for positional ones;
    /// * a `:vararg` parameter takes the remaining positional arguments as
    ///   GAS writes them: tokens of one comma field joined by a space, fields
    ///   by a comma (`V 1, 2 3,4` gives `2 3,4`). `.macro _xor_data
    ///   vecs:vararg` invoked as `_xor_data 0,1,2,3,4,5,6,7` must hand all
    ///   eight rounds to `.irp i, \vecs` (binding just `0` once dropped 7/8 of
    ///   the AES-CTR rounds);
    /// * an omitted argument takes its default, else is an error for
    ///   `:req`, else the EMPTY string -- the earlier `"0"` turned
    ///   `.byte \a\b` with `b` omitted into `70`.
    fn bind(&self, fields: &[Vec<String>]) -> Result<Vec<String>, String> {
        bind_macro_args(&self.name, &self.params, fields)
    }
}

/// See [`MacroDef::bind`]; shared with the x86 expander.
pub(crate) fn bind_macro_args(
    macro_name: &str,
    params: &[MacroParam],
    fields: &[Vec<String>],
) -> Result<Vec<String>, String> {
    let mut named: Vec<Option<String>> = vec![None; params.len()];
    let mut positional: Vec<(usize, String)> = Vec::new();
    for (field, args) in fields.iter().enumerate() {
        for arg in args {
            if let Some(eq) = find_toplevel_eq(arg) {
                let key = arg[..eq].trim();
                if is_macro_param_name(key) {
                    let Some(pi) = params.iter().position(|p| p.name == key) else {
                        return Err(format!(
                            "Parameter named `{key}' does not exist for macro `{macro_name}'"
                        ));
                    };
                    named[pi] = Some(strip_arg_quotes(arg[eq + 1..].trim()).to_string());
                    continue;
                }
            }
            if named.iter().any(Option::is_some) {
                return Err("can't mix positional and keyword arguments".to_string());
            }
            positional.push((field, arg.clone()));
        }
    }
    let mut positional = positional.into_iter();
    let mut values = Vec::with_capacity(params.len());
    for (param, named) in params.iter().zip(named) {
        let given = if param.vararg {
            let rest = join_vararg(positional.by_ref());
            named.or((!rest.is_empty()).then_some(rest))
        } else {
            named.or_else(|| {
                positional
                    .next()
                    .map(|(_, arg)| strip_arg_quotes(&arg).to_string())
            })
        };
        let value = match (given, &param.default) {
            (Some(v), _) => v,
            (None, Some(d)) => d.clone(),
            (None, None) if param.required => {
                return Err(format!(
                    "Missing value for required parameter `{}' of macro `{macro_name}'",
                    param.name
                ));
            }
            (None, None) => String::new(),
        };
        values.push(value);
    }
    if positional.next().is_some() {
        return Err("too many positional arguments".to_string());
    }
    Ok(values)
}

/// The text a `:vararg` parameter receives, see [`bind_macro_args`].
fn join_vararg(args: impl Iterator<Item = (usize, String)>) -> String {
    let mut out = String::new();
    let mut last_field = None;
    for (field, arg) in args {
        if let Some(prev) = last_field {
            out.push(if prev == field { ' ' } else { ',' });
        }
        out.push_str(&arg);
        last_field = Some(field);
    }
    out
}

/// Whether `s` can name a macro parameter (a GAS symbol-name token).
fn is_macro_param_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '_' | '.' | '$'))
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '$'))
}

fn strip_arg_quotes(s: &str) -> &str {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        let bytes = s.as_bytes();
        let mut i = 1usize;
        while i + 1 < bytes.len() {
            if bytes[i] == b'\\' {
                i += 1;
                if i + 1 < bytes.len() {
                    i += std::str::from_utf8(&bytes[i..])
                        .ok()
                        .and_then(|rest| rest.chars().next())
                        .map_or(1, char::len_utf8);
                } else {
                    // Trailing backslash escapes the closing quote (`"foo\"`): not balanced.
                    return s;
                }
                continue;
            }
            if bytes[i] == b'"' {
                // Quote closed before end of token (e.g. `"a" + "b"`): keep both quotes.
                return s;
            }
            i += 1;
        }
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Index of the first `=` that separates a parameter name from its default.
///
/// Ignores `=` inside parentheses, quoted strings, or character literals, and
/// skips the comparison operators `==`, `!=`, `<=` and `>=` so an expression
/// default or quoted positional argument is never split mid-token.
fn find_toplevel_eq(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut i = 0;
    while i < b.len() {
        if in_string {
            if b[i] == b'\\' {
                i += 1;
                if i < b.len() {
                    i += std::str::from_utf8(&s.as_bytes()[i..])
                        .ok()
                        .and_then(|rest| rest.chars().next())
                        .map_or(1, char::len_utf8);
                }
                continue;
            }
            if b[i] == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match b[i] {
            b'"' => {
                in_string = true;
                i += 1;
                continue;
            }
            b'\'' => {
                i = crate::backend::asm_expr::skip_gas_char_literal(b, i);
                continue;
            }
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'=' if depth == 0 => {
                let prev = if i > 0 { b[i - 1] } else { 0 };
                let next = if i + 1 < b.len() { b[i + 1] } else { 0 };
                let is_cmp = matches!(prev, b'!' | b'<' | b'>' | b'=') || next == b'=';
                if !is_cmp {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Split a GAS parameter qualifier off a `.macro` parameter token.
///
/// GAS allows `name:req` (argument is mandatory) and `name:vararg` (argument
/// swallows the rest of the line). The qualifier is not part of the parameter
/// NAME: the body refers to `\ftr`, not `\ftr:req`. Keeping the qualifier in
/// the name meant substitution never fired, and the kernel's
/// `.macro FILL_RETURN_BUFFER reg:req nr:req ftr:req ftr2=...` left every
/// `\ftr` unexpanded in the macro body.
///
/// Returns the parameter with its qualifier applied and no default.
fn split_param_qualifier(tok: &str) -> MacroParam {
    let (name, qual) = match tok.split_once(':') {
        Some((base, qual)) if matches!(qual, "req" | "vararg") && !base.is_empty() => (base, qual),
        _ => (tok, ""),
    };
    MacroParam {
        name: name.to_string(),
        default: None,
        required: qual == "req",
        vararg: qual == "vararg",
    }
}

/// Split the operands of `.macro` into the macro name and its parameters.
pub(crate) fn parse_macro_header(rest: &str) -> (&str, Vec<MacroParam>) {
    match rest.find([' ', '\t', ',']) {
        Some(pos) => (
            &rest[..pos],
            parse_macro_params(rest[pos..].trim().trim_start_matches(',')),
        ),
        None => (rest, Vec::new()),
    }
}

/// Parse a macro parameter list, handling `param = default_value` syntax.
///
/// GAS splits `.macro` parameters on top-level commas **and/or whitespace**,
/// and a `= default` belongs to exactly one parameter: the default is the
/// single operand token that follows the `=`, ending at the next top-level
/// whitespace or comma.  Everything here was byte-probed against binutils
/// 2.47 (`as`) because the rule is not documented in this detail:
///
/// | header                       | `as` behaviour                                  |
/// |------------------------------|-------------------------------------------------|
/// | `a=1 b=2`                    | two parameters, defaults 1 and 2                |
/// | `a=1,b=2`                    | two parameters (comma separator)                |
/// | `a = 1`                      | one parameter, default 1 (spaces around `=` ok) |
/// | `a b=1`                      | bare `a`, `b` with default 1                    |
/// | `type:req a=0 b=0 c=0`       | four parameters                                 |
/// | `ftr2=((0x8000 | (3*32+21)))`| default keeps every space inside the parens      |
/// | `a=1 << 2 b=3`               | `a` (default 1), `<<`, `2`, `b` (default 3)      |
///
/// The previous implementation split the header on top-level COMMAS first and
/// then attached everything after the first `=` on a part to that parameter,
/// so a header like the kernel's
///
/// ```text
/// .macro UNWIND_HINT type:req sp_reg=0 sp_offset=0 signal=0
/// ```
///
/// (arch/x86/include/asm/objtool.h, reachable from arch/x86/entry/entry.S via
/// <asm/unwind_hints.h>) became ONE parameter `type` with the default
/// `req sp_reg=0 sp_offset=0 signal=0`.  Every `UNWIND_HINT ... sp_offset=`
/// invocation then failed with "Parameter named `sp_offset' does not exist
/// for macro `UNWIND_HINT'", which is what stopped the full kernel build at
/// its first object.
///
/// Parens/brackets/braces are tracked so a default may contain separators
/// (`ALT_NOT(feature)` after the C preprocessor is `((... | (3*32+21)))`).
fn parse_macro_params(params_str: &str) -> Vec<MacroParam> {
    let mut params = Vec::new();
    let bytes = params_str.as_bytes();
    let mut i = 0usize;

    // Next byte that is not a parameter separator (space, tab, comma).
    let skip_separators = |i: &mut usize| {
        while *i < bytes.len() && matches!(bytes[*i], b' ' | b'\t' | b',') {
            *i += 1;
        }
    };

    loop {
        skip_separators(&mut i);
        if i >= bytes.len() {
            break;
        }
        // Parameter name, together with any `:req`/`:vararg` qualifier: it ends
        // at a separator or at the `=` that introduces its default.
        let name_start = i;
        while i < bytes.len() && !matches!(bytes[i], b' ' | b'\t' | b',' | b'=') {
            i += 1;
        }
        let token = &params_str[name_start..i];
        if token.is_empty() {
            // A stray `=` (e.g. `a=1 == 2`): consume it so the loop advances.
            i += 1;
            continue;
        }
        let mut param = split_param_qualifier(token);

        // `name = default` with optional whitespace around the `=`.
        let mut look = i;
        skip_separators(&mut look);
        if look < bytes.len() && bytes[look] == b'=' {
            look += 1;
            while look < bytes.len() && matches!(bytes[look], b' ' | b'\t') {
                look += 1;
            }
            let mut default_buf = String::new();
            let mut depth = 0i32;
            let mut in_string = false;
            while look < bytes.len() {
                if in_string {
                    let Some(ch) = params_str[look..].chars().next() else {
                        break;
                    };
                    default_buf.push(ch);
                    look += ch.len_utf8();
                    if ch == '\\' {
                        if let Some(esc) = params_str[look..].chars().next() {
                            default_buf.push(esc);
                            look += esc.len_utf8();
                        }
                    } else if ch == '"' {
                        in_string = false;
                    }
                    continue;
                }
                match bytes[look] {
                    b'"' => {
                        in_string = true;
                        default_buf.push('"');
                        look += 1;
                    }
                    b'\'' => {
                        // GAS's input scrubber turns an ASCII character literal
                        // `'x` / `'\n'` into its decimal value before `.macro`
                        // parses the header (matching `split_macro_arg_fields`).
                        let next_end = crate::backend::asm_expr::skip_gas_char_literal(bytes, look);
                        let raw_lit = &params_str[look..next_end];
                        let mut lit_chars = raw_lit[1..].chars();
                        let val_opt = match lit_chars.next() {
                            Some('\\') => lit_chars.next().map(|e| match e {
                                'b' => '\u{8}',
                                'f' => '\u{c}',
                                'n' => '\n',
                                'r' => '\r',
                                't' => '\t',
                                other => other,
                            }),
                            other => other,
                        };
                        match val_opt {
                            Some(v) if v.is_ascii() => {
                                default_buf.push_str(&u32::from(v).to_string());
                            }
                            _ => default_buf.push_str(raw_lit),
                        }
                        look = next_end;
                    }
                    b'(' | b'[' | b'{' => {
                        depth += 1;
                        default_buf.push(bytes[look] as char);
                        look += 1;
                    }
                    b')' | b']' | b'}' => {
                        depth -= 1;
                        default_buf.push(bytes[look] as char);
                        look += 1;
                    }
                    b' ' | b'\t' | b',' if depth == 0 => break,
                    _ => {
                        let Some(ch) = params_str[look..].chars().next() else {
                            break;
                        };
                        default_buf.push(ch);
                        look += ch.len_utf8();
                    }
                }
            }
            param.default = Some(strip_arg_quotes(&default_buf).to_string());
            i = look;
        }
        params.push(param);
    }

    params
}

/// How GAS's input scrubber separates whitespace-delimited macro arguments
/// on a target (see [`split_macro_arg_fields`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MacroArgLexing {
    /// x86: `-`, `*`, `(`, `[`, `%` and `$` begin an operand
    /// (`-8(%rbp)`, `*%rax`, `$1`), so whitespace before them separates.
    X86,
    /// AArch64 and RISC-V: only symbol characters do.
    Generic,
}

impl MacroArgLexing {
    fn word_start(self, c: char) -> bool {
        is_macro_word_char(c) || (self == Self::X86 && matches!(c, '-' | '*' | '(' | '[' | '%'))
    }

    fn word_end(self, c: char) -> bool {
        is_macro_word_char(c) || (self == Self::X86 && matches!(c, '-' | '*'))
    }
}

fn is_macro_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '$' | '@' | '"' | '\'' | '\\')
}

/// Split macro invocation arguments the way the AArch64/RISC-V GAS does;
/// see [`split_macro_arg_fields`].
pub fn split_macro_args(s: &str) -> Vec<String> {
    split_macro_arg_fields(s, MacroArgLexing::Generic).concat()
}

/// Split macro invocation arguments into comma fields of arguments, as GAS
/// 2.47 does (every rule measured on x86-64, AArch64 and RISC-V):
///
/// * a top-level comma ends a field (commas inside parentheses, strings and
///   character literals do not);
/// * whitespace separates two arguments only between a character that can
///   end a word and one that can begin a word ([`MacroArgLexing`]); next to
///   anything else it disappears: `1 + 2`, `(1) (2)` and `%hi(x) %lo(y)` are
///   one argument everywhere, `1 -1` and `x0 [x1]` are one on AArch64/RISC-V
///   but two on x86, and `a b`, `1 $2`, `1 "x"` are two everywhere;
/// * a character literal `'x` (optionally closed `'x'`) is data, even a
///   space or comma (`pr 'r', '8', ' ', ':', %r8` in relocate_kernel_64.S),
///   and becomes its decimal value, as GAS's scrubber writes it: `' '` is
///   `32`, `'\n'` is `10`; only `\b \f \n \r \t` are escapes, any other
///   escaped character is itself (`'\v` is `118`, `'\101` is `49` then `01`).
///
/// Quotes are kept; [`MacroDef::bind`] strips them from argument values.
pub(crate) fn split_macro_arg_fields(s: &str, lexing: MacroArgLexing) -> Vec<Vec<String>> {
    let mut fields: Vec<Vec<String>> = Vec::new();
    let mut field: Vec<String> = Vec::new();
    let mut arg = String::new();
    let (mut depth, mut in_str) = (0i32, false);
    let mut chars = s.chars().peekable();
    // GAS's scrubber has already replaced a character literal by its decimal
    // value, and drops the whitespace after one (`',' 2` is `442`).
    let mut after_char_literal = false;
    fn end_arg(arg: &mut String, field: &mut Vec<String>) {
        let a = arg.trim();
        if !a.is_empty() {
            field.push(a.to_string());
        }
        arg.clear();
    }
    while let Some(c) = chars.next() {
        if in_str {
            arg.push(c);
            match c {
                '\\' => arg.extend(chars.next()),
                '"' => in_str = false,
                _ => {}
            }
            continue;
        }
        if !c.is_whitespace() {
            after_char_literal = false;
        }
        match c {
            '"' => {
                in_str = true;
                arg.push(c);
            }
            '\'' => {
                let value = match chars.next() {
                    Some('\\') => chars.next().map(|e| match e {
                        'b' => '\u{8}',
                        'f' => '\u{c}',
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        other => other,
                    }),
                    other => other,
                };
                chars.next_if_eq(&'\'');
                match value {
                    Some(v) if v.is_ascii() => {
                        arg.push_str(&u32::from(v).to_string());
                        after_char_literal = true;
                        continue;
                    }
                    // GAS writes the first UTF-8 byte of a non-ASCII literal,
                    // which a `String` cannot hold; keep the literal as written.
                    Some(v) => {
                        arg.push('\'');
                        arg.push(v);
                    }
                    None => arg.push('\''),
                }
            }
            '(' => {
                depth += 1;
                arg.push(c);
            }
            ')' => {
                depth -= 1;
                arg.push(c);
            }
            ',' if depth <= 0 => {
                end_arg(&mut arg, &mut field);
                if !field.is_empty() {
                    fields.push(std::mem::take(&mut field));
                }
            }
            c if c.is_whitespace() => {
                while chars.next_if(|c| c.is_whitespace()).is_some() {}
                let (Some(prev), Some(&next)) = (arg.chars().last(), chars.peek()) else {
                    continue;
                };
                if !after_char_literal && lexing.word_end(prev) && lexing.word_start(next) {
                    if depth > 0 {
                        arg.push(' ');
                    } else {
                        end_arg(&mut arg, &mut field);
                    }
                }
            }
            _ => arg.push(c),
        }
    }
    end_arg(&mut arg, &mut field);
    if !field.is_empty() {
        fields.push(field);
    }
    fields
}

/// Expand `.macro`/`.endm` definitions and macro invocations.
///
/// Definitions take effect in order, including ones made by an expanded
/// macro body (a macro may define macros); `.purgem` removes one. An
/// invocation may follow any number of labels (`a: b: M 7`), as in GAS.
pub fn expand_macros(lines: &[&str], comment_style: &CommentStyle) -> Result<Vec<String>, String> {
    expand_macros_counted(lines, comment_style, &mut ExpansionTotals::default())
}

/// [`expand_macros`] charging the caller's own accumulator. The nested-billing
/// law is a statement about that accumulator, not about the returned lines --
/// without this seam the only way to observe it is to expand several
/// megabytes of text and watch which side of the ceiling refuses.
pub(crate) fn expand_macros_counted(
    lines: &[&str],
    comment_style: &CommentStyle,
    totals: &mut ExpansionTotals,
) -> Result<Vec<String>, String> {
    let mut run = MacroRun {
        macros: crate::common::fx_hash::FxHashMap::default(),
        comment_style,
        totals,
        invocations: 0,
    };
    run.expand(lines, 0)
}

/// The state of one [`expand_macros`] run.
struct MacroRun<'a> {
    macros: crate::common::fx_hash::FxHashMap<String, MacroDef>,
    comment_style: &'a CommentStyle,
    totals: &'a mut ExpansionTotals,
    /// GAS `\@`: macro invocations so far in this run.
    invocations: u64,
}

impl MacroRun<'_> {
    /// Expand `lines` in macro frame `depth`. GAS 2.47 fatals the 102nd
    /// live frame with `macros nested too deeply` (101-frame chains assemble
    /// -- measured on the pinned oracle); without the bound a
    /// self-referential macro recurses until the native stack overflows,
    /// which is a crash, never a verdict.
    fn expand(&mut self, lines: &[&str], depth: u32) -> Result<Vec<String>, String> {
        if depth > 101 {
            return Err("macros nested too deeply".to_string());
        }
        // GAS expands statements, not lines: `EMIT val=7 ; EMIT val=9` in a
        // macro body is two invocations, and the second must neither be
        // swallowed as arguments of the first nor be left unexpanded.
        let statements = split_statements(lines, self.comment_style);
        let lines: Vec<&str> = statements.iter().map(|s| s.as_ref()).collect();
        let mut result = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let stmt = strip_comment(lines[i], self.comment_style).trim();
            if let Some(rest) = directive_operands(stmt, ".macro") {
                let (name, params) = parse_macro_header(rest);
                let mut body = Vec::new();
                let mut nesting = 1;
                i += 1;
                while i < lines.len() {
                    let inner = strip_comment(lines[i], self.comment_style).trim();
                    if directive_operands(inner, ".macro").is_some() {
                        nesting += 1;
                    } else if directive_operands(inner, ".endm").is_some() {
                        nesting -= 1;
                        if nesting == 0 {
                            break;
                        }
                    }
                    body.push(lines[i].to_string());
                    i += 1;
                }
                let def = MacroDef {
                    name: name.to_string(),
                    params,
                    body,
                };
                self.macros.insert(name.to_string(), def);
            } else if directive_operands(stmt, ".endm").is_some() {
                // A stray `.endm` closes nothing.
            } else if let Some(name) = directive_operands(stmt, ".purgem") {
                self.macros.remove(name);
            } else {
                let (labels, invocation) = split_leading_labels(stmt);
                let name = invocation.split([' ', '\t']).next().unwrap_or("");
                match self.macros.get(name) {
                    Some(mac) if !name.starts_with('.') => {
                        let body = self.instantiate(mac, invocation[name.len()..].trim())?;
                        self.invocations += 1;
                        result.extend(labels.iter().map(|l| l.to_string()));
                        let refs: Vec<&str> = body.iter().map(String::as_str).collect();
                        let expanded = self.expand(&refs, depth + 1)?;
                        self.totals.charge(".macro", &expanded)?;
                        result.extend(expanded);
                    }
                    _ => result.push(lines[i].to_string()),
                }
            }
            i += 1;
        }
        Ok(result)
    }

    /// The body of `mac` for one invocation with `args`: parameters bound
    /// ([`MacroDef::bind`]) and substituted, `\()` removed and `\@`
    /// replaced. `\@` is numbered BEFORE the body's own invocations
    /// (pre-order: `M` = 0, its inner `I` = 1, the next `M` = 2), so every
    /// expansion gets unique local labels such as `.L\@`.
    fn instantiate(&self, mac: &MacroDef, args: &str) -> Result<Vec<String>, String> {
        let values = mac.bind(&split_macro_arg_fields(args, MacroArgLexing::Generic))?;
        let counter = self.invocations.to_string();
        // Longest name first, so `\orig` never matches inside `\orig_len`.
        let mut order: Vec<usize> = (0..mac.params.len()).collect();
        order.sort_by_key(|&pi| std::cmp::Reverse(mac.params[pi].name.len()));
        Ok(mac
            .body
            .iter()
            .map(|line| {
                let mut line = line.clone();
                for &pi in &order {
                    let pattern = format!("\\{}", mac.params[pi].name);
                    line = replace_macro_param(&line, &pattern, &values[pi]);
                }
                // `\()` separates a parameter from adjacent text
                // (`\op\()_safe_regs` -> `rdmsr_safe_regs`) and is empty.
                line.replace("\\()", "").replace("\\@", &counter)
            })
            .collect())
    }
}

/// `lines` with every multi-statement line (`a ; b`, outside strings and
/// after `style`'s comment) split into its statements. A single-statement
/// line is kept verbatim, comment included, so it reaches the parser as
/// written.
fn split_statements<'l>(lines: &[&'l str], style: &CommentStyle) -> Vec<std::borrow::Cow<'l, str>> {
    let mut out = Vec::with_capacity(lines.len());
    for &line in lines {
        let code = strip_comment(line, style);
        let bytes = code.as_bytes();
        let mut parts = Vec::new();
        let (mut start, mut in_str) = (0usize, false);
        let mut i = 0usize;
        while i < bytes.len() {
            if in_str {
                if bytes[i] == b'\\' {
                    i += 1;
                    if i < bytes.len() {
                        i += std::str::from_utf8(&bytes[i..])
                            .ok()
                            .and_then(|rest| rest.chars().next())
                            .map_or(1, char::len_utf8);
                    }
                    continue;
                }
                if bytes[i] == b'"' {
                    in_str = false;
                }
                i += 1;
                continue;
            }
            match bytes[i] {
                b'"' => {
                    in_str = true;
                    i += 1;
                }
                b'\'' => {
                    i = crate::backend::asm_expr::skip_gas_char_literal(bytes, i);
                }
                b';' => {
                    parts.push(&code[start..i]);
                    i += 1;
                    start = i;
                }
                _ => {
                    i += 1;
                }
            }
        }
        if parts.is_empty() {
            out.push(std::borrow::Cow::Borrowed(line));
            continue;
        }
        parts.push(&code[start..]);
        out.extend(
            parts
                .into_iter()
                .map(crate::backend::asm_expr::trim_asm_line)
                .filter(|s| !s.is_empty())
                .map(|s| std::borrow::Cow::Owned(s.to_string())),
        );
    }
    out
}

/// The operands of `stmt` when it is the directive `name` (`name` alone or
/// followed by whitespace), else `None`.
fn directive_operands<'s>(stmt: &'s str, name: &str) -> Option<&'s str> {
    let rest = stmt.strip_prefix(name)?;
    (rest.is_empty() || rest.starts_with([' ', '\t'])).then(|| rest.trim())
}

/// Split the `label:` definitions off the front of a statement. GAS accepts
/// any number before a macro invocation (`a: b: M 7`, `1: M 8`).
fn split_leading_labels(stmt: &str) -> (Vec<&str>, &str) {
    let mut labels = Vec::new();
    let mut rest = stmt;
    loop {
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '$')))
            .unwrap_or(rest.len());
        let after = &rest[end..];
        if end == 0 || !after.starts_with(':') || after.starts_with("::") {
            return (labels, rest);
        }
        labels.push(&rest[..=end]);
        rest = after[1..].trim_start();
    }
}

/// Replace `\param` in a macro body line with the argument value, but only
/// when `\param` is followed by a non-identifier character (or end of string).
///
/// GAS macro parameter references like `\orig` should NOT match as a prefix
/// of `\orig_len`. In GAS, `\()` is used to explicitly delimit parameter names
/// from adjacent identifier characters (e.g., `\op\()_safe_regs`).
pub fn replace_macro_param(text: &str, pattern: &str, replacement: &str) -> String {
    let pat_len = pattern.len();
    if pat_len == 0 || !text.contains(pattern) {
        // Nothing to substitute: skip the copying pass entirely (this runs for
        // every `\param` of every line of every macro body).
        return text.to_string();
    }
    let text_bytes = text.as_bytes();
    let mut result = String::with_capacity(text.len());
    let mut copied = 0; // text before this offset is already in `result`
    let mut search = 0;
    while let Some(rel) = text[search..].find(pattern) {
        let at = search + rel;
        // The character after the match must not extend the parameter name;
        // GAS symbol characters include every byte with the high bit set.
        let continues = text_bytes
            .get(at + pat_len)
            .is_some_and(|&b| b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80);
        if continues {
            // Not a full match -- the parameter name continues. `at + 1` is a
            // char boundary because the pattern starts with `\\`.
            search = at + 1;
            continue;
        }
        result.push_str(&text[copied..at]);
        result.push_str(replacement);
        copied = at + pat_len;
        search = copied;
    }
    result.push_str(&text[copied..]);
    result
}

// ── .if / .else / .endif conditional assembly ──────────────────────────

/// GNU as's numeric value for one register *spelling* (without the leading `%`).
///
/// GAS substitutes the index of the register's entry in its internal
/// `i386_regtab` wherever a register appears in an expression. That is what
/// makes `.if %rsp == %rbp` a legal *constant* comparison — the comparison runs
/// over those indices, so equality is identity of the SPELLING, not of the
/// machine register: `%rsp == %rsp` is true while `%rax == %eax` is false even
/// though both name the same physical register in different widths. The
/// kernel's `arch/x86/include/asm/unwind_hints.h` builds its whole
/// `UNWIND_HINT_REGS base=...` dispatch on exactly that identity, so this table
/// is load-bearing for the kernel boot path, not a convenience.
///
/// Every value below was MEASURED against GNU as, never transcribed from a
/// header: `as --64` on `.long %<reg>` emits the number (and warns "register
/// value used as expression" — precisely because the value is a spelling
/// identity and not an address). `scripts/check_asm_reg_expr_gas_parity.sh`
/// re-measures the entire table against the host `as`, so a future GAS that
/// renumbers its table is caught rather than silently mis-branching.
fn x86_register_number(name: &str) -> Option<i64> {
    /// Decode `prefix<digits>` into `base + digits` for a numbered bank.
    fn bank(name: &str, prefix: &str, base: i64, first: i64, count: i64) -> Option<i64> {
        let digits = name.strip_prefix(prefix)?;
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let index: i64 = digits.parse().ok()?;
        (first..first + count)
            .contains(&index)
            .then_some(base + (index - first))
    }

    match name {
        // 8-bit legacy names. GAS's table lists them in ENCODING order
        // (al,cl,dl,bl,ah,ch,dh,bh), which is why this is not a `bank` call.
        "al" => return Some(0),
        "cl" => return Some(1),
        "dl" => return Some(2),
        "bl" => return Some(3),
        "ah" => return Some(4),
        "ch" => return Some(5),
        "dh" => return Some(6),
        "bh" => return Some(7),
        "spl" => return Some(12),
        "bpl" => return Some(13),
        "sil" => return Some(14),
        "dil" => return Some(15),
        // 16-bit
        "ax" => return Some(40),
        "cx" => return Some(41),
        "dx" => return Some(42),
        "bx" => return Some(43),
        "sp" => return Some(44),
        "bp" => return Some(45),
        "si" => return Some(46),
        "di" => return Some(47),
        // 32-bit
        "eax" => return Some(72),
        "ecx" => return Some(73),
        "edx" => return Some(74),
        "ebx" => return Some(75),
        "esp" => return Some(76),
        "ebp" => return Some(77),
        "esi" => return Some(78),
        "edi" => return Some(79),
        // 64-bit
        "rax" => return Some(104),
        "rcx" => return Some(105),
        "rdx" => return Some(106),
        "rbx" => return Some(107),
        "rsp" => return Some(108),
        "rbp" => return Some(109),
        "rsi" => return Some(110),
        "rdi" => return Some(111),
        // Segment registers, instruction pointer, x87 top-of-stack.
        "es" => return Some(144),
        "cs" => return Some(145),
        "ss" => return Some(146),
        "ds" => return Some(147),
        "fs" => return Some(148),
        "gs" => return Some(149),
        "rip" => return Some(323),
        "st" => return Some(327),
        _ => {}
    }
    // r8b..r15b, r8w..r15w, r8d..r15d and r8..r15. `strip_prefix('r')` also
    // accepts rax..rdi, so those are matched explicitly above before reaching
    // here; any other `r<digits>` outside 8..=15 is not a register.
    if let Some(rest) = name.strip_prefix('r') {
        let (digits, base) = if let Some(d) = rest.strip_suffix('b') {
            (d, 16)
        } else if let Some(d) = rest.strip_suffix('w') {
            (d, 48)
        } else if let Some(d) = rest.strip_suffix('d') {
            (d, 80)
        } else {
            (rest, 112)
        };
        return bank(digits, "", base, 8, 8);
    }
    // Control, debug, MMX and the SIMD banks. Longest prefix first so that
    // `%xmm0` can never be read as `%mm…`.
    for (prefix, base, count) in [
        ("xmm", 215, 32),
        ("ymm", 247, 32),
        ("zmm", 279, 32),
        ("mm", 207, 8),
        ("cr", 151, 16),
        ("dr", 183, 8),
    ] {
        if let Some(n) = bank(name, prefix, base, 0, count) {
            return Some(n);
        }
    }
    None
}

/// Rewrite every register spelling in `expr` to GAS's numeric value.
///
/// One left-to-right pass with a longest-name lookup at each `%` — deliberately
/// NOT a sequence of `str::replace` calls. The previous implementation built a
/// list of pairs and relied on the author ordering it longest-first, which is a
/// silent-corruption hazard in both directions: `%r8` rewrites the first three
/// characters of `%r8d`, and `%st` rewrites the prefix of `%st(1)`. Scanning
/// once and requiring the WHOLE `%`-name run to be a known spelling removes the
/// hazard by construction — reordering the table can no longer change the
/// output.
///
/// A `%` that is not followed by a known register name is passed through
/// verbatim: `%` is also GAS's modulus operator (`10 % 3`, `x%2`), and that
/// meaning must survive.
pub fn resolve_x86_registers(expr: &str) -> String {
    let bytes = expr.as_bytes();
    let mut out = String::with_capacity(expr.len());
    let mut k = 0;
    while k < bytes.len() {
        if bytes[k] != b'%' {
            let start = k;
            while k < bytes.len() && bytes[k] != b'%' {
                k += 1;
            }
            out.push_str(&expr[start..k]);
            continue;
        }
        // The name run after `%`. `%st(3)` ends the run at `(`, handled below.
        let name_start = k + 1;
        let mut end = name_start;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }
        match x86_register_number(&expr[name_start..end]) {
            Some(mut value) => {
                // x87 stack slots: `%st(3)` is a distinct spelling from `%st`
                // and GAS numbers it `%st` + slot.
                let mut stop = end;
                if &expr[name_start..end] == "st" && expr[end..].starts_with('(') {
                    if let Some(close) = expr[end..].find(')') {
                        if let Ok(slot) = expr[end + 1..end + close].parse::<i64>() {
                            if (0..=7).contains(&slot) {
                                value = 327 + slot;
                                stop = end + close + 1;
                            }
                        }
                    }
                }
                out.push_str(&value.to_string());
                k = stop;
            }
            None => {
                out.push('%');
                k += 1;
            }
        }
    }
    out
}

/// Evaluate a simple `.if` condition expression.
///
/// Supports: integer literals, `==`, `!=`, `>=`, `<=`, `>`, `<`, and simple
/// arithmetic via `asm_expr::parse_integer_expr`. Non-zero result is true.
pub fn eval_if_condition(cond: &str) -> bool {
    eval_if_condition_inner(cond, |s| s.to_string())
}

/// Evaluate a `.if` condition with a pre-processing step for resolving names.
///
/// The `resolve` function is called on each side of a comparison operator
/// before integer expression parsing. Use this to resolve register names
/// or symbol values.
pub fn eval_if_condition_with_resolver<F: Fn(&str) -> String>(cond: &str, resolve: F) -> bool {
    eval_if_condition_inner(cond, resolve)
}

/// Find position of an isolated `>` or `<` that is not part of `>>`, `<<`, `>=`, or `<=`,
/// and not inside parentheses.
fn find_isolated_cmp(cond: &str, ch: char) -> Option<usize> {
    let bytes = cond.as_bytes();
    let target = ch as u8;
    let mut depth = 0i32;
    for i in 0..bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        if depth == 0 && bytes[i] == target {
            let next = bytes.get(i + 1).copied();
            let prev = if i > 0 { Some(bytes[i - 1]) } else { None };
            if next != Some(b'>')
                && next != Some(b'<')
                && next != Some(b'=')
                && prev != Some(b'>')
                && prev != Some(b'<')
            {
                return Some(i);
            }
        }
    }
    None
}

fn eval_if_condition_inner<F: Fn(&str) -> String>(cond: &str, resolve: F) -> bool {
    eval_if_condition_dyn(cond, &resolve)
}

fn eval_if_condition_dyn(cond: &str, resolve: &dyn Fn(&str) -> String) -> bool {
    let cond = cond.trim();
    // Strip outer parentheses: (.Lfound != 1) -> .Lfound != 1
    let cond = strip_outer_parens(cond);

    // Handle || (logical OR) at top level — lowest precedence
    if let Some(pos) = find_top_level_op(cond, "||") {
        let lhs = &cond[..pos];
        let rhs = &cond[pos + 2..];
        return eval_if_condition_dyn(lhs, resolve) || eval_if_condition_dyn(rhs, resolve);
    }
    // Handle && (logical AND) at top level
    if let Some(pos) = find_top_level_op(cond, "&&") {
        let lhs = &cond[..pos];
        let rhs = &cond[pos + 2..];
        return eval_if_condition_dyn(lhs, resolve) && eval_if_condition_dyn(rhs, resolve);
    }

    // Find comparison operators at the top level (not inside parentheses).
    // Check "!=" before "==" and ">="/"<=" before ">"/"<".
    if let Some(pos) = find_top_level_op(cond, "!=") {
        let lhs = resolve(cond[..pos].trim());
        let rhs = resolve(cond[pos + 2..].trim());
        let l = asm_expr::parse_integer_expr(&lhs).unwrap_or(i64::MIN);
        let r = asm_expr::parse_integer_expr(&rhs).unwrap_or(i64::MAX);
        return l != r;
    }
    if let Some(pos) = find_top_level_op(cond, "==") {
        let lhs = resolve(cond[..pos].trim());
        let rhs = resolve(cond[pos + 2..].trim());
        let l = asm_expr::parse_integer_expr(&lhs).unwrap_or(i64::MIN);
        let r = asm_expr::parse_integer_expr(&rhs).unwrap_or(i64::MAX);
        return l == r;
    }
    if let Some(pos) = find_top_level_op(cond, ">=") {
        let lhs = resolve(cond[..pos].trim());
        let rhs = resolve(cond[pos + 2..].trim());
        let l = asm_expr::parse_integer_expr(&lhs).unwrap_or(i64::MIN);
        let r = asm_expr::parse_integer_expr(&rhs).unwrap_or(i64::MAX);
        return l >= r;
    }
    if let Some(pos) = find_top_level_op(cond, "<=") {
        let lhs = resolve(cond[..pos].trim());
        let rhs = resolve(cond[pos + 2..].trim());
        let l = asm_expr::parse_integer_expr(&lhs).unwrap_or(i64::MIN);
        let r = asm_expr::parse_integer_expr(&rhs).unwrap_or(i64::MAX);
        return l <= r;
    }
    // Try isolated ">" not part of ">>" or ">="
    if let Some(pos) = find_isolated_cmp(cond, '>') {
        let lhs = resolve(cond[..pos].trim());
        let rhs = resolve(cond[pos + 1..].trim());
        let l = asm_expr::parse_integer_expr(&lhs).unwrap_or(i64::MIN);
        let r = asm_expr::parse_integer_expr(&rhs).unwrap_or(i64::MAX);
        return l > r;
    }
    // Try isolated "<" not part of "<<" or "<="
    if let Some(pos) = find_isolated_cmp(cond, '<') {
        let lhs = resolve(cond[..pos].trim());
        let rhs = resolve(cond[pos + 1..].trim());
        let l = asm_expr::parse_integer_expr(&lhs).unwrap_or(i64::MIN);
        let r = asm_expr::parse_integer_expr(&rhs).unwrap_or(i64::MAX);
        return l < r;
    }
    // Simple integer expression: non-zero is true
    let resolved = resolve(cond);
    asm_expr::parse_integer_expr(&resolved).unwrap_or(0) != 0
}

/// Strip balanced outer parentheses from an expression.
/// `(expr)` -> `expr`, `((expr))` -> `expr`, `(a) + (b)` -> unchanged.
fn strip_outer_parens(s: &str) -> &str {
    let s = s.trim();
    if !s.starts_with('(') || !s.ends_with(')') {
        return s;
    }
    // Check if the outer parens are truly balanced as a pair
    let inner = &s[1..s.len() - 1];
    let mut depth = 0i32;
    for ch in inner.bytes() {
        match ch {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth < 0 {
                    // Unbalanced: the closing paren in the middle means
                    // the outer parens are not a matching pair
                    return s;
                }
            }
            _ => {}
        }
    }
    if depth == 0 { inner.trim() } else { s }
}

/// Find a comparison operator at the top level (not inside parentheses).
fn find_top_level_op(s: &str, op: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let op_bytes = op.as_bytes();
    let op_len = op_bytes.len();
    if bytes.len() < op_len {
        return None;
    }
    let mut depth = 0i32;
    for i in 0..=bytes.len() - op_len {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        if depth == 0 && &bytes[i..i + op_len] == op_bytes {
            return Some(i);
        }
    }
    None
}

// ── Shared data-value helpers ──────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// The billing law on [`ExpansionTotals::charge`], read off a two-level
    /// expansion: `outer`'s body is one `inner` invocation plus one line, and
    /// `inner`'s body is three lines, so four lines are materialized and seven
    /// are billed -- the inner frame's three, and the parent's four that already
    /// contain them.
    ///
    /// Pinned because the double charge is the only thing making the ceiling
    /// conservative for nested input, and a future "unification" that removes it
    /// would halve the bound on exactly the input class that costs the most
    /// (each nested level multiplies, so the top level's number is the one that
    /// matters).  The per-front-end wiring that lets each target reach this
    /// accounting is pinned where it lives: `gas247_expansion_ceilings_reach_the_
    /// riscv_entry_point` (RISC-V), the `ExpansionBudget` delegation test in
    /// `backend/x86/assembler/parser.rs`, and the `.macro`/`.rept` growth checks
    /// in `backend/arm/assembler/parser.rs`.
    #[test]
    fn nested_macro_output_is_billed_twice() {
        let src = ".macro inner\n nop\n nop\n nop\n.endm\n\
                   .macro outer\n inner\n ld x0\n.endm\n\
                   outer\n";
        let mut totals = ExpansionTotals::default();
        let out = expand_macros_counted(
            &src.lines().collect::<Vec<_>>(),
            &CommentStyle::Hash,
            &mut totals,
        )
        .expect("four lines is nowhere near the ceiling");
        assert_eq!(out.len(), 4, "the file materializes body(inner)+1 line");
        assert_eq!(
            totals.lines, 7,
            "3 (inner's own charge) + 4 (outer's charge, which repeats them)"
        );
        // The bytes follow the same rule, so the two counters cannot disagree
        // about which expansions were seen.
        // `expansion_size` bills `len + 1` per line, so the byte total is the
        // same doubling seen from the other counter: the inner frame's three
        // 4-char lines, then those three again inside the parent's four.
        assert_eq!(
            totals.bytes,
            3 * 5 + (3 * 5 + 7),
            "inner's 3 lines, then the same 3 plus the parent's own line"
        );
        // A single-level expansion is charged exactly once: the doubling is a
        // property of nesting, not a factor of two on every macro.
        let mut flat = ExpansionTotals::default();
        let one = expand_macros_counted(
            &".macro m\n nop\n nop\n.endm\n m\n"
                .lines()
                .collect::<Vec<_>>(),
            &CommentStyle::Hash,
            &mut flat,
        )
        .expect("flat macro");
        assert_eq!(one.len(), 2);
        assert_eq!(flat.lines, 2, "one frame, one charge");
    }

    /// One ceiling, three spellings.  `.rept`/`.irp` can predict their size
    /// from a literal count and refuse before emitting anything; `.macro`
    /// cannot, which is why all three hand their *totals* to
    /// `check_expansion_growth` and no per-construct ceiling exists to drift.
    /// The parity is what stops `.macro` being a way around `.rept` (and vice
    /// versa), and the second half pins the one place the two do differ: the
    /// nesting multiplier, which can only make a macro refuse earlier.
    #[test]
    fn same_size_macro_and_rept_share_the_ceiling() {
        let limit = MAX_EXPANSION_LINES as u64;
        let cap = MAX_EXPANSION_TEXT as u64;
        let q = (MAX_EXPANSION_LINES / 4) as u64;
        for kind in ["macro", "rept", "irp"] {
            // The line ceiling: the last admissible total and the first refusal.
            assert_eq!(check_expansion_growth(kind, limit, 1), Ok(()));
            assert_eq!(
                check_expansion_growth(kind, limit + 1, 1).unwrap_err(),
                format!("{kind}: expansion exceeds the {limit}-line limit"),
                "{kind} must be refused one line past the same ceiling"
            );
            // And the byte ceiling, which is the half a 4 Mi-line body of
            // 1-byte lines does NOT reach -- hence two ceilings.
            assert_eq!(check_expansion_growth(kind, 1, cap), Ok(()));
            assert_eq!(
                check_expansion_growth(kind, 1, cap + 1).unwrap_err(),
                format!("{kind}: expansion exceeds the {cap}-byte limit")
            );
        }
        // Same charged size -> same verdict, whatever the spelling called it.
        for kind in ["macro", "rept"] {
            assert_eq!(check_expansion_growth(kind, 3 * q, 3 * q * 5), Ok(()));
        }
        // 3q materialized lines reached through one nesting level are charged
        // 7/4 as much (the 4-materialized : 7-billed ratio above), which is
        // past the 4q ceiling: the repeat of the same text is still admitted.
        assert_eq!(
            check_expansion_growth("macro", 3 * q * 7 / 4, 1).unwrap_err(),
            format!("macro: expansion exceeds the {limit}-line limit")
        );
        assert_eq!(check_expansion_growth("rept", 3 * q, 1), Ok(()));
    }

    /// The riscv-facing expander carries the same GAS 2.47 wall as the
    /// other two front ends (it used to recurse until the native stack
    /// overflowed).  Measured: 101 frames OK, 102 fatal, cycles caught.
    #[test]
    fn gas247_macro_nesting_wall_shared() {
        let run = |src: &str| expand_macros(&src.lines().collect::<Vec<_>>(), &CommentStyle::Hash);
        let chain = |n: usize| {
            let mut s = String::new();
            for i in 0..n {
                let nxt = if i + 1 < n {
                    format!("m{}", i + 1)
                } else {
                    ".byte 7".into()
                };
                s.push_str(&format!(".macro m{i}\n{nxt}\n.endm\n"));
            }
            s.push_str("m0\n");
            s
        };
        run(&chain(101)).expect("101 frames assemble on GAS 2.47");
        let err = run(&chain(102)).unwrap_err();
        assert!(err.contains("macros nested too deeply"), "got: {err}");
        let err = run(".macro m\n m\n.endm\n m\n").unwrap_err();
        assert!(err.contains("macros nested too deeply"), "got: {err}");
    }

    /// The `.macro` path's half of the budget that `.rept` already enforced:
    /// a *run* total, charged across every invocation of every frame, not a
    /// per-frame allowance (101 frames each granted 4M lines would be 400M).
    ///
    /// Why the refusal is tested on the accumulator rather than end to end:
    /// the ceiling is 4 Mi lines, so an end-to-end crossing must materialize
    /// 4 Mi `String`s (~150 MB) before the budget can fire.  A unit suite has
    /// no business paying for that to exercise a constant.  What it must pin is
    /// the two things that are actually new here -- that charges accumulate
    /// across calls, and that a charge which is legal on its own is still
    /// refused once the run passes the ceiling.
    #[test]
    fn expansion_growth_budget_accumulates_across_expansions() {
        let chunk: Vec<String> = vec!["nop".to_string(); MAX_EXPANSION_LINES / 64];
        assert!(
            check_expansion_growth(".macro", chunk.len() as u64, 4 * chunk.len() as u64).is_ok(),
            "each charge alone must be inside the ceiling, or the test proves nothing"
        );
        let mut totals = ExpansionTotals::default();
        let mut refused = None;
        for frame in 1..=128u64 {
            if let Err(e) = totals.charge(".macro", &chunk) {
                refused = Some((frame, e));
                break;
            }
        }
        let (frame, err) = refused.expect("128 frames of 1/64 the ceiling must cross it");
        assert_eq!(
            totals.lines as usize,
            frame as usize * (MAX_EXPANSION_LINES / 64),
            "the total must be the sum of every charge, with none dropped"
        );
        assert!(
            err.contains("line limit") && err.contains(".macro"),
            "got: {err}"
        );
    }

    /// Saturating, not `+`: a run that would overflow `u64` is long past every
    /// ceiling, and an arithmetic panic would answer a resource question with a
    /// crash instead of a diagnostic.
    #[test]
    fn expansion_growth_charge_saturates_instead_of_panicking() {
        let mut totals = ExpansionTotals {
            lines: u64::MAX,
            bytes: u64::MAX,
        };
        let err = totals
            .charge(".macro", &["nop".to_string()])
            .expect_err("u64::MAX is past every ceiling");
        assert_eq!(totals.lines, u64::MAX);
        assert_eq!(totals.bytes, u64::MAX);
        // Both ceilings refuse, and the byte one is consulted first -- pinning
        // the precedence as well, because a run can be over one and under the
        // other and the message is the only thing the user sees.
        assert!(err.contains("byte limit"), "got: {err}");
        let mut by_lines = ExpansionTotals {
            lines: MAX_EXPANSION_LINES as u64,
            bytes: 0,
        };
        let err = by_lines
            .charge(".macro", &["nop".to_string()])
            .expect_err("one line past the ceiling must refuse");
        assert!(err.contains("line limit"), "got: {err}");
        assert_eq!(by_lines.lines, MAX_EXPANSION_LINES as u64 + 1);
        let exactly_at = ExpansionTotals {
            lines: MAX_EXPANSION_LINES as u64,
            bytes: 0,
        };
        assert!(
            check_expansion_growth(".macro", exactly_at.lines, exactly_at.bytes).is_ok(),
            "the ceiling is inclusive: 4 Mi lines is the permitted maximum, not a rejection"
        );
    }

    /// Legal work must not start failing: a 101-frame chain is at the wall and
    /// still fine, and 500 invocations of a 200-line body -- 100k lines
    /// accumulated inside a single frame, the shape the old depth-only accounting
    /// left unbounded -- expands in full.
    #[test]
    fn expansion_growth_budget_leaves_legal_expansions_alone() {
        let run = |src: &str| expand_macros(&src.lines().collect::<Vec<_>>(), &CommentStyle::Hash);
        let mut src = String::new();
        for i in 0..101 {
            let nxt = if i + 1 < 101 {
                format!("m{}", i + 1)
            } else {
                ".byte 7".into()
            };
            src.push_str(&format!(".macro m{i}\n{nxt}\n.endm\n"));
        }
        src.push_str("m0\n");
        run(&src).expect("the wall is 101 frames, not a budget problem");

        let mut bulk = String::from(".macro B\n");
        for _ in 0..200 {
            bulk.push_str("nop\n");
        }
        bulk.push_str(".endm\n");
        for _ in 0..500 {
            bulk.push_str("B\n");
        }
        let out = run(&bulk).expect("100k lines is a quarter of the ceiling");
        assert_eq!(out.iter().filter(|l| l.trim() == "nop").count(), 200 * 500);
    }

    /// A macro that calls itself twice per frame is the exponential shape.  It
    /// must answer with the depth diagnostic in bounded time -- the error
    /// propagates out of the first over-deep frame, so nothing is materialized
    /// along the way.  (Two statements: `d d` is ONE call of `d` with a
    /// surplus argument, which GAS 2.47 rejects before recursing.)
    #[test]
    fn self_doubling_macro_is_refused_not_materialized() {
        let src = ".macro d\n d\n d\n.endm\nd\n";
        let err = expand_macros(&src.lines().collect::<Vec<_>>(), &CommentStyle::Hash)
            .expect_err("must not expand");
        assert!(err.contains("macros nested too deeply"), "got: {err}");
    }

    #[test]
    fn gas247_rept_budget_law() {
        use crate::backend::asm_expr::parse_integer_expr;
        let style = CommentStyle::Hash;
        let ok = |src: &str| {
            let lines: Vec<&str> = src.lines().collect();
            expand_rept_blocks(&lines, &style, parse_integer_expr)
        };
        // Small counts assemble; the body repeats verbatim.
        let out = ok(".rept 2\n.byte 0\n.endr").unwrap();
        assert_eq!(out, vec![".byte 0".to_string(), ".byte 0".to_string()]);
        assert!(ok(".rept 0\n.byte 0\n.endr").unwrap().is_empty());
        // Negative counts wrap to size_t and are rejected with GAS's own
        // wording (measured on GAS 2.47: "excessive count
        // 18446744073709551615 for REPT - ignored").
        let err = ok(".rept -1\n.byte 0\n.endr").unwrap_err();
        assert!(
            err.contains("excessive count 18446744073709551615 for REPT - ignored"),
            "got: {err}"
        );
        // Astronomic counts die on the budget in O(body) time (PR768 M2:
        // this used to exhaust memory materializing the repetitions).
        let err = ok(".rept 1099511627776\n.byte 0\n.endr").unwrap_err();
        assert!(
            err.contains("excessive count 1099511627776 for REPT - ignored"),
            "got: {err}"
        );
        // Pure-function threshold: eff = 65 + (7 + 2) = 74 for a
        // `.byte 0` body; 58040098 * 74 <= 0xffffffff < 58040099 * 74.
        let body = [".byte 0"];
        assert!(check_rept_budget(58_040_098, &body).is_ok());
        let err = check_rept_budget(58_040_099, &body).unwrap_err();
        assert!(err.contains("excessive count 58040099"), "got: {err}");
        // count * eff overflow rejects regardless of magnitude.
        assert!(check_rept_budget(u64::MAX, &body).is_err());
        // Growth ceilings: counts inside the GAS budget that would blow
        // the deterministic line/text caps are refused BEFORE the loop
        // runs (the counters gate the extend, not the aftermath).
        let err = ok(".rept 5000000\n.byte 0\n.endr").unwrap_err();
        assert!(err.contains("line limit"), "got: {err}");
        let long = "x".repeat(100);
        let err = ok(&format!(".rept 3000000\n{long}\n.endr")).unwrap_err();
        assert!(err.contains("byte limit"), "got: {err}");
    }

    #[test]
    fn test_strip_c_comments() {
        assert_eq!(strip_c_comments("a /* b */ c"), "a  c");
        assert_eq!(strip_c_comments("a /* b\nc */ d"), "a \n d");
    }

    #[test]
    fn test_strip_c_comments_preserves_strings() {
        // /* inside a quoted string must not be treated as a comment
        assert_eq!(
            strip_c_comments(r#".asciz "hello/*world*/end""#),
            r#".asciz "hello/*world*/end""#
        );
        // Escaped quotes inside strings should not end the string
        assert_eq!(
            strip_c_comments(r#".asciz "a\"/*b*/c""#),
            r#".asciz "a\"/*b*/c""#
        );
        // /* outside strings should still be stripped
        assert_eq!(
            strip_c_comments(r#".asciz "hello" /* comment */ .byte 1"#),
            r#".asciz "hello"  .byte 1"#
        );
    }

    #[test]
    fn test_strip_comment_hash() {
        let style = CommentStyle::Hash;
        assert_eq!(
            strip_comment("movq %rax, %rbx # comment", &style),
            "movq %rax, %rbx "
        );
        assert_eq!(strip_comment(".asciz \"a#b\"", &style), ".asciz \"a#b\"");
    }

    #[test]
    fn test_strip_comment_slash_slash_and_at() {
        let style = CommentStyle::SlashSlashAndAt;
        assert_eq!(
            strip_comment("mov x0, x1 // comment", &style),
            "mov x0, x1 "
        );
        assert_eq!(strip_comment("mov x0, x1 @ comment", &style), "mov x0, x1 ");
        assert_eq!(
            strip_comment(".type foo, @function", &style),
            ".type foo, @function"
        );
    }

    #[test]
    fn test_split_on_semicolons() {
        let parts = split_on_semicolons("a; b; c");
        assert_eq!(parts, vec!["a", " b", " c"]);
        let parts = split_on_semicolons(".asciz \"a;b\"; nop");
        assert_eq!(parts, vec![".asciz \"a;b\"", " nop"]);
        // Semicolons inside # comments should not cause splitting
        let parts = split_on_semicolons("nop # comment; with semicolons");
        assert_eq!(parts, vec!["nop # comment; with semicolons"]);
        // Semicolons inside // comments should not cause splitting
        let parts = split_on_semicolons("nop // comment; with semicolons");
        assert_eq!(parts, vec!["nop // comment; with semicolons"]);
        // Full-line // comments with semicolons
        let parts = split_on_semicolons("// struct {size_t a,b;} *p = (void*)x;");
        assert_eq!(parts, vec!["// struct {size_t a,b;} *p = (void*)x;"]);
        // Semicolons before comment still split normally
        let parts = split_on_semicolons("a; b # comment; c");
        assert_eq!(parts, vec!["a", " b # comment; c"]);
    }

    #[test]
    fn test_eval_if_condition() {
        assert!(eval_if_condition("1"));
        assert!(!eval_if_condition("0"));
        assert!(eval_if_condition("1 == 1"));
        assert!(!eval_if_condition("1 == 2"));
        assert!(eval_if_condition("1 != 2"));
        assert!(eval_if_condition("3 >= 2"));
        assert!(eval_if_condition("2 >= 2"));
        assert!(!eval_if_condition("1 >= 2"));
        assert!(eval_if_condition("1 <= 2"));
        assert!(eval_if_condition("3 > 2"));
        assert!(!eval_if_condition("2 > 2"));
        assert!(eval_if_condition("1 < 2"));
    }

    #[test]
    fn test_eval_if_condition_with_x86_registers() {
        // Test register equality (like kernel UNWIND_HINT_REGS)
        assert!(eval_if_condition_with_resolver(
            "%rsp == %rsp",
            resolve_x86_registers
        ));
        assert!(!eval_if_condition_with_resolver(
            "%rsp == %rbp",
            resolve_x86_registers
        ));
        assert!(eval_if_condition_with_resolver(
            "%rbp == %rbp",
            resolve_x86_registers
        ));
        assert!(eval_if_condition_with_resolver(
            "%rdi == %rdi",
            resolve_x86_registers
        ));
        assert!(eval_if_condition_with_resolver(
            "%rdx == %rdx",
            resolve_x86_registers
        ));
        assert!(eval_if_condition_with_resolver(
            "%r10 == %r10",
            resolve_x86_registers
        ));
        assert!(eval_if_condition_with_resolver(
            "%rsp != %rbp",
            resolve_x86_registers
        ));
    }

    #[test]
    fn test_resolve_x86_registers() {
        assert_eq!(resolve_x86_registers("%rsp"), "108");
        assert_eq!(resolve_x86_registers("%rbp"), "109");
        assert_eq!(resolve_x86_registers("%rdi"), "111");
        assert_eq!(resolve_x86_registers("%r10"), "114");
        assert_eq!(resolve_x86_registers("%r13"), "117");
    }

    /// The whole GAS register table, pinned to values measured from GNU as
    /// (`.long %<reg>`). `scripts/check_asm_reg_expr_gas_parity.sh` re-measures
    /// these against the host assembler; this test keeps the values in the
    /// compiler's own suite so a refactor cannot drift silently.
    #[test]
    fn test_resolve_x86_registers_matches_gas_table() {
        // High-value spellings across every bank, plus the width-aliasing case
        // that must stay DISTINCT: %rax and %eax name the same machine register
        // but are different GAS spellings, so they are different values.
        let cases: &[(&str, &str)] = &[
            ("%al", "0"),
            ("%ah", "4"),
            ("%spl", "12"),
            ("%dil", "15"),
            ("%r8b", "16"),
            ("%r15b", "23"),
            ("%ax", "40"),
            ("%di", "47"),
            ("%r8w", "48"),
            ("%r15w", "55"),
            ("%eax", "72"),
            ("%edi", "79"),
            ("%r8d", "80"),
            ("%r15d", "87"),
            ("%rax", "104"),
            ("%r15", "119"),
            ("%es", "144"),
            ("%gs", "149"),
            ("%cr4", "155"),
            ("%dr7", "190"),
            ("%mm0", "207"),
            ("%mm7", "214"),
            ("%xmm0", "215"),
            ("%xmm31", "246"),
            ("%ymm0", "247"),
            ("%ymm31", "278"),
            ("%zmm0", "279"),
            ("%zmm31", "310"),
            ("%rip", "323"),
            ("%st", "327"),
            ("%st(0)", "327"),
            ("%st(7)", "334"),
        ];
        for (reg, value) in cases {
            assert_eq!(
                resolve_x86_registers(reg),
                *value,
                "GAS value for {reg} drifted"
            );
        }
        assert_ne!(
            resolve_x86_registers("%rax"),
            resolve_x86_registers("%eax"),
            "%rax and %eax are distinct GAS spellings"
        );
    }

    /// Regression: the table must not be applied as ordered substitutions.
    /// `%r8` is a prefix of `%r8d`, `%st` is a prefix of `%st(1)`, and `%mm0`
    /// sits inside the `%xmm0`/`%ymm0`/`%zmm0` namespaces.
    #[test]
    fn test_resolve_x86_registers_has_no_partial_match_corruption() {
        assert_eq!(resolve_x86_registers("%r8d"), "80");
        assert_eq!(resolve_x86_registers("%r8w"), "48");
        assert_eq!(resolve_x86_registers("%r8b"), "16");
        assert_eq!(resolve_x86_registers("%r9d"), "81");
        assert_eq!(resolve_x86_registers("%st(1)"), "328");
        assert_eq!(resolve_x86_registers("%xmm0"), "215");
        assert_eq!(resolve_x86_registers("%mm0"), "207");
        // Registers embedded in a real comparison expression.
        assert_eq!(resolve_x86_registers("%rsp == %rbp"), "108 == 109");
        assert_eq!(resolve_x86_registers("%r10 != %rdi"), "114 != 111");
    }

    /// `%` is also GAS's modulus operator; only a `%` that starts a known
    /// register spelling may be rewritten.
    #[test]
    fn test_resolve_x86_registers_leaves_modulus_alone() {
        assert_eq!(resolve_x86_registers("10 % 3"), "10 % 3");
        assert_eq!(resolve_x86_registers("10%3"), "10%3");
        assert_eq!(resolve_x86_registers("%"), "%");
        assert_eq!(resolve_x86_registers("%nosuchreg"), "%nosuchreg");
    }

    /// End-to-end shape of the kernel's `UNWIND_HINT_REGS` dispatch: the
    /// resolver feeds the boolean evaluator, so the whole `base=` chain must
    /// select exactly one arm.
    #[test]
    fn test_unwind_hint_regs_dispatch_shape() {
        let arms = ["%rsp", "%rbp", "%rdi", "%rdx", "%r10"];
        for (i, want) in arms.iter().enumerate() {
            let matched: Vec<&str> = arms
                .iter()
                .copied()
                .filter(|arm| {
                    eval_if_condition_with_resolver(
                        &format!("{arm} == {want}"),
                        resolve_x86_registers,
                    )
                })
                .collect();
            assert_eq!(matched, vec![*want], "arm {i} must match exactly once");
        }
    }

    #[test]
    fn test_split_macro_args() {
        assert_eq!(split_macro_args("a, b, c"), vec!["a", "b", "c"]);
        assert_eq!(split_macro_args("0(a1), x, y"), vec!["0(a1)", "x", "y"]);
        assert_eq!(split_macro_args(""), Vec::<String>::new());
        // Expression with operator: `889f - 888f` stays as one arg (operator merging)
        assert_eq!(
            split_macro_args("886b, 888f, 0x1234, 0, 889f - 888f"),
            vec!["886b", "888f", "0x1234", "0", "889f-888f"]
        );
        // Without commas, spaces are separators
        assert_eq!(split_macro_args("a b c"), vec!["a", "b", "c"]);
        // Mixed comma and space: `fixup lb a5, 0(a1), 10f` → 4 args
        assert_eq!(
            split_macro_args("lb      a5, 0(a1), 10f"),
            vec!["lb", "a5", "0(a1)", "10f"]
        );
        // Expression operators keep tokens together
        assert_eq!(split_macro_args("foo + bar"), vec!["foo+bar"]);
        assert_eq!(split_macro_args("foo + bar, baz"), vec!["foo+bar", "baz"]);
        // GNU as treats `a -4` as expression `a-4` (binary minus, not unary)
        assert_eq!(split_macro_args("a -4 b"), vec!["a-4", "b"]);
        // Operand followed by operator-prefixed token: binary subtraction
        assert_eq!(split_macro_args("889f -888f"), vec!["889f-888f"]);
    }

    /// `(name, default, required, vararg)` of each parameter.
    fn param_shapes(params: &str) -> Vec<(String, Option<String>, bool, bool)> {
        parse_macro_params(params)
            .into_iter()
            .map(|p| (p.name, p.default, p.required, p.vararg))
            .collect()
    }

    fn plain(name: &str) -> (String, Option<String>, bool, bool) {
        (name.to_string(), None, false, false)
    }

    #[test]
    fn test_parse_macro_params_simple() {
        assert_eq!(
            param_shapes("a, b, c"),
            [plain("a"), plain("b"), plain("c")]
        );
        assert_eq!(param_shapes("a b c"), [plain("a"), plain("b"), plain("c")]);
        assert!(param_shapes("").is_empty());
    }

    #[test]
    fn test_parse_macro_params_with_defaults() {
        let five = ("b".to_string(), Some("5".to_string()), false, false);
        assert_eq!(param_shapes("a, b = 5, c"), [plain("a"), five, plain("c")]);
        // GAS-style: `.macro ALT_NEW_CONTENT vendor_id, patch_id, enable = 1, new_c`
        let enable = ("enable".to_string(), Some("1".to_string()), false, false);
        assert_eq!(
            param_shapes("vendor_id, patch_id, enable = 1, new_c"),
            [
                plain("vendor_id"),
                plain("patch_id"),
                enable,
                plain("new_c")
            ]
        );
        // A default keeps its spaces and parentheses, and the names before it
        // are separate parameters (the kernel's FILL_RETURN_BUFFER).
        let flags = (
            "flags".to_string(),
            Some("(((1 << 4) | ( 3*32+21)))".to_string()),
            false,
            false,
        );
        assert_eq!(
            param_shapes("reg nr flags=(((1 << 4) | ( 3*32+21)))"),
            [plain("reg"), plain("nr"), flags]
        );
    }

    #[test]
    fn test_parse_macro_params_qualifiers() {
        let vecs = ("vecs".to_string(), None, false, true);
        assert_eq!(param_shapes("vecs:vararg"), [vecs]);
        let rest = ("rest".to_string(), None, false, true);
        assert_eq!(param_shapes("a, rest:vararg"), [plain("a"), rest]);
        let req = |n: &str| (n.to_string(), None, true, false);
        assert_eq!(param_shapes("reg:req, nr:req"), [req("reg"), req("nr")]);
    }

    #[test]
    fn test_vararg_param_swallows_all_args() {
        // The kernel's aes-ctr-avx-x86_64.S pattern: `.irp i, \vecs` inside a
        // :vararg macro must see the FULL comma list. Before the fix, \vecs
        // bound to the first argument only and 7 of 8 AES round-loop
        // iterations were silently dropped from the object.
        let lines = vec![
            ".macro _emit vecs:vararg",
            ".irp i, \\vecs",
            "\t.long \\i",
            ".endr",
            ".endm",
            "_emit 10, 20, 30",
        ];
        let result = expand_macros(&lines, &CommentStyle::Hash).unwrap();
        assert!(
            result
                .iter()
                .any(|l| l.replace('\t', " ").contains(".irp i, 10,20,30")),
            "vararg substitution incomplete: {result:?}"
        );
    }

    #[test]
    fn test_vararg_param_empty_invocation() {
        let lines = vec![
            ".macro _emit vecs:vararg",
            ".irp i, \\vecs",
            "\t.long \\i",
            ".endr",
            ".endm",
            "_emit",
        ];
        let result = expand_macros(&lines, &CommentStyle::Hash).unwrap();
        // No arguments: the vararg expands to the empty string, not "0".
        assert!(
            result
                .iter()
                .any(|l| l.replace('\t', " ").contains(".irp i, ")),
            "empty vararg should substitute to empty: {result:?}"
        );
    }

    #[test]
    fn test_purgem_removes_macro() {
        // Simulates the kernel's insn-def.h pattern: define macro, use it, .purgem it
        let lines = vec![
            ".macro insn_r, opcode, func3",
            ".4byte (\\opcode | \\func3)",
            ".endm",
            "insn_r 0x33, 0x0",
            ".purgem insn_r",
        ];
        let result = expand_macros(&lines, &CommentStyle::HashAndSlashSlash).unwrap();
        // The macro invocation should have been expanded
        assert!(result.iter().any(|l| l.contains(".4byte")));
        // The .purgem line should have been consumed (not passed through)
        assert!(!result.iter().any(|l| l.contains(".purgem")));
    }

    #[test]
    fn test_purgem_prevents_further_expansion() {
        // After .purgem, the macro name should no longer be recognized
        let lines = vec![
            ".macro mymacro",
            "nop",
            ".endm",
            "mymacro",
            ".purgem mymacro",
            "mymacro",
        ];
        let result = expand_macros(&lines, &CommentStyle::HashAndSlashSlash).unwrap();
        // First invocation expands to "nop"
        // After .purgem, second "mymacro" is passed through as-is (not a known macro)
        let nop_count = result.iter().filter(|l| l.trim() == "nop").count();
        assert_eq!(nop_count, 1, "macro should only expand once before .purgem");
        let mymacro_count = result.iter().filter(|l| l.trim() == "mymacro").count();
        assert_eq!(
            mymacro_count, 1,
            "after .purgem, 'mymacro' should be passed through literally"
        );
    }

    #[test]
    fn test_replace_macro_param_basic() {
        // Basic replacement
        assert_eq!(
            replace_macro_param(".byte \\orig", "\\orig", "140b"),
            ".byte 140b"
        );
    }

    #[test]
    fn test_replace_macro_param_boundary_rejection() {
        // \orig should NOT match as prefix of \orig_len
        assert_eq!(
            replace_macro_param(".byte \\orig_len", "\\orig", "140b"),
            ".byte \\orig_len"
        );
    }

    #[test]
    fn test_replace_macro_param_end_of_string() {
        // \orig at end of text should be replaced
        assert_eq!(replace_macro_param("\\orig", "\\orig", "140b"), "140b");
    }

    #[test]
    fn test_replace_macro_param_followed_by_operator() {
        // \orig followed by '-' (not an identifier char) should be replaced
        assert_eq!(
            replace_macro_param("\\orig-\\alt", "\\orig", "142b"),
            "142b-\\alt"
        );
    }

    #[test]
    fn test_replace_macro_param_multiple() {
        // Multiple occurrences
        assert_eq!(
            replace_macro_param(".long \\sym - . ; .byte \\sym", "\\sym", "foo"),
            ".long foo - . ; .byte foo"
        );
    }

    #[test]
    fn test_replace_macro_param_no_match() {
        // Pattern not present
        assert_eq!(
            replace_macro_param(".byte 42", "\\orig", "140b"),
            ".byte 42"
        );
    }

    #[test]
    fn test_replace_macro_param_adjacent_digit() {
        // \orig followed by digit should NOT be replaced (digit is identifier continuation)
        assert_eq!(replace_macro_param("\\orig2", "\\orig", "foo"), "\\orig2");
    }

    #[test]
    fn test_replace_macro_param_before_delimiter() {
        // \op followed by \() delimiter — \op IS replaced, \() stripped later
        assert_eq!(
            replace_macro_param("\\op\\()_safe_regs", "\\op", "rdmsr"),
            "rdmsr\\()_safe_regs"
        );
    }

    fn fields(s: &str, lexing: MacroArgLexing) -> Vec<Vec<String>> {
        split_macro_arg_fields(s, lexing)
    }

    fn flat(s: &str, lexing: MacroArgLexing) -> Vec<String> {
        fields(s, lexing).concat()
    }

    /// Every expectation below is GAS 2.47 output (`.ascii "[\a|\b|\c]"`).
    #[test]
    fn macro_arg_whitespace_follows_the_target_scrubber() {
        use MacroArgLexing::{Generic, X86};
        for (src, x86, generic) in [
            ("1 -1", &["1", "-1"][..], &["1-1"][..]),
            ("1 - 1", &["1", "-", "1"], &["1-1"]),
            ("4 *2", &["4", "*2"], &["4*2"]),
            ("1 (2)", &["1", "(2)"], &["1(2)"]),
            ("x0 [x1]", &["x0", "[x1]"], &["x0[x1]"]),
            ("1 + 2", &["1+2"], &["1+2"]),
            ("(1) (2)", &["(1)(2)"], &["(1)(2)"]),
            ("(1) -2", &["(1)-2"], &["(1)-2"]),
            ("%hi(x) %lo(y)", &["%hi(x)%lo(y)"], &["%hi(x)%lo(y)"]),
            ("1 ) 2", &["1)2"], &["1)2"]),
            ("a b", &["a", "b"], &["a", "b"]),
            ("a. b", &["a.", "b"], &["a.", "b"]),
            ("1 $2", &["1", "$2"], &["1", "$2"]),
            ("1 \"x\"", &["1", "\"x\""], &["1", "\"x\""]),
            ("a = 3", &["a=3"], &["a=3"]),
        ] {
            assert_eq!(flat(src, X86), x86, "x86: {src}");
            assert_eq!(flat(src, Generic), generic, "generic: {src}");
        }
        assert_eq!(
            fields("886b, 888f, 0x1234, 0, 889f - 888f", Generic),
            [["886b"], ["888f"], ["0x1234"], ["0"], ["889f-888f"]]
        );
        // Inside parentheses whitespace never separates; GAS scrubs it.
        assert_eq!(flat("(2, 3),4", Generic), ["(2,3)", "4"]);
    }

    #[test]
    fn macro_arg_char_literals_become_decimal() {
        use MacroArgLexing::{Generic, X86};
        for lexing in [X86, Generic] {
            assert_eq!(
                fields("'r', '8', ' ', ':', %r8", lexing),
                [["114"], ["56"], ["32"], ["58"], ["%r8"]]
            );
            assert_eq!(flat("',' 2", lexing), ["442"]);
            assert_eq!(flat("'a' 'b'", lexing), ["9798"]);
            assert_eq!(flat("1 'a", lexing), ["1", "97"]);
            assert_eq!(flat("x'a", lexing), ["x97"]);
            assert_eq!(flat("'a+1", lexing), ["97+1"]);
            assert_eq!(flat(r"'\n '\v '\\", lexing), ["1011892"]);
            assert_eq!(flat(r"'\'' '\101", lexing), ["394901"]);
        }
    }

    fn bind(header: &str, args: &str) -> Result<Vec<String>, String> {
        let (name, params) = parse_macro_header(header);
        bind_macro_args(name, &params, &fields(args, MacroArgLexing::X86))
    }

    /// GAS 2.47 binding rules, shared by every target's expander.
    #[test]
    fn macro_binding_matches_gas() {
        let ok = |v: &[&str]| Ok(v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(bind("o a b c", "1 c=3"), ok(&["1", "", "3"]));
        assert_eq!(bind("o a b", "b = 5 a=1"), ok(&["1", "5"]));
        assert_eq!(bind("o a", "a=1 a=2"), ok(&["2"]), "last keyword wins");
        assert_eq!(bind("o s", "s=\"a b\""), ok(&["a b"]));
        assert_eq!(bind("o s t", "\"x\" 1"), ok(&["x", "1"]));
        assert_eq!(bind("o a=(1 << 2)", ""), ok(&["(1 << 2)"]));
        assert_eq!(bind("o a, b", "x0, =sym"), ok(&["x0", "=sym"]));
        assert_eq!(bind("v a, r:vararg", "1, 2 3,4"), ok(&["1", "2 3,4"]));
        assert_eq!(bind("v a, r:vararg", "1, (2,3),4"), ok(&["1", "(2,3),4"]));
        assert_eq!(bind("v a, r:vararg", "1"), ok(&["1", ""]));
        assert_eq!(bind("v a, r:vararg", "1 r=9"), ok(&["1", "9"]));
        assert_eq!(
            bind("o a b", "a=1 2").unwrap_err(),
            "can't mix positional and keyword arguments"
        );
        assert_eq!(
            bind("o a", "1 2").unwrap_err(),
            "too many positional arguments"
        );
        assert_eq!(
            bind("o a:req", "").unwrap_err(),
            "Missing value for required parameter `a' of macro `o'"
        );
        assert_eq!(
            bind("o a", "z=1").unwrap_err(),
            "Parameter named `z' does not exist for macro `o'"
        );
    }

    #[test]
    fn macro_headers_split_on_whitespace_as_well_as_commas() {
        // Every expectation below was byte-probed against binutils 2.47 `as`.
        let names = |s: &str| -> Vec<(String, Option<String>)> {
            super::parse_macro_params(s)
                .into_iter()
                .map(|p| (p.name, p.default))
                .collect()
        };
        // The kernel's UNWIND_HINT header (arch/x86/include/asm/objtool.h):
        // parsed as ONE parameter `type` before this fix, so every
        // `UNWIND_HINT ... sp_offset=N` invocation died with
        // "Parameter named `sp_offset' does not exist".
        assert_eq!(
            names("type:req sp_reg=0 sp_offset=0 signal=0"),
            vec![
                ("type".to_string(), None),
                ("sp_reg".to_string(), Some("0".into())),
                ("sp_offset".to_string(), Some("0".into())),
                ("signal".to_string(), Some("0".into())),
            ]
        );
        assert_eq!(
            names("base=%rsp offset=0 indirect=0 extra=1"),
            vec![
                ("base".to_string(), Some("%rsp".into())),
                ("offset".to_string(), Some("0".into())),
                ("indirect".to_string(), Some("0".into())),
                ("extra".to_string(), Some("1".into())),
            ]
        );
        // Comma separation, spaces around `=`, and a bare parameter.
        assert_eq!(
            names("a=1,b=2"),
            vec![
                ("a".to_string(), Some("1".into())),
                ("b".to_string(), Some("2".into()))
            ]
        );
        assert_eq!(names("a = 1"), vec![("a".to_string(), Some("1".into()))]);
        assert_eq!(
            names("a b=1"),
            vec![("a".to_string(), None), ("b".to_string(), Some("1".into()))]
        );
        // A default keeps separators that are inside brackets: after the C
        // preprocessor ALT_NOT(X86_FEATURE_ALWAYS) is `((... | (3*32+21)))`.
        assert_eq!(
            names("ftr2=((0x80000000 | (3*32+21)))"),
            vec![(
                "ftr2".to_string(),
                Some("((0x80000000 | (3*32+21)))".into())
            )]
        );
        // `as` ends a default at the first top-level space, so `<<` and `2`
        // become (unused) parameters.  Matching that keeps a header's
        // parameter count identical to the oracle's, which is what decides
        // "too many positional arguments".
        assert_eq!(
            names("a=1 << 2 b=3"),
            vec![
                ("a".to_string(), Some("1".into())),
                ("<<".to_string(), None),
                ("2".to_string(), None),
                ("b".to_string(), Some("3".into())),
            ]
        );
        // Quoted and character-literal defaults keep commas, spaces, `:req`,
        // and escaped quotes inside a single parameter default, stripping only
        // balanced outer quotes (matching GAS 2.47).
        assert_eq!(
            names(r#"p="1, 2" q="x y:req" r="ab\"cd" s=',', t=' '"#),
            vec![
                ("p".to_string(), Some("1, 2".into())),
                ("q".to_string(), Some("x y:req".into())),
                ("r".to_string(), Some(r#"ab\"cd"#.into())),
                ("s".to_string(), Some("44".into())),
                ("t".to_string(), Some("32".into())),
            ]
        );
    }

    #[test]
    fn strip_comment_and_split_semicolons_preserve_gas_char_literals() {
        let line = ".byte 'A', ' ', ',', ';', '#' # trailing ; comment";
        let stripped = strip_comment(line, &CommentStyle::Hash);
        assert_eq!(stripped.trim(), ".byte 'A', ' ', ',', ';', '#'");
        assert_eq!(split_on_semicolons(stripped).len(), 1);
    }
}
