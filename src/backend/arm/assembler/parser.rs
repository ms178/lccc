//! AArch64 assembly parser.
//!
//! Parses the textual assembly format emitted by our AArch64 codegen into
//! structured `AsmStatement` values. The parser handles:
//! - Labels (global and local)
//! - Directives (.section, .globl, .type, .align, .byte, .long, .xword, etc.)
//! - AArch64 instructions (mov, add, sub, ldr, str, bl, ret, etc.)
//! - CFI directives (passed through as-is for DWARF unwind info)

// Some parser helper functions and enum variants are defined for completeness
// and used only by the encoder or ELF writer, not the parser entry point itself.
#![allow(dead_code)]

use crate::backend::asm_expr;
use crate::backend::asm_preprocess;
use crate::backend::elf;

/// A parsed assembly operand.
#[derive(Debug, Clone)]
pub enum Operand {
    /// Register: x0-x30, w0-w30, sp, xzr, wzr, d0-d31, s0-s31, q0-q31, v0-v31
    Reg(String),
    /// Immediate value: #42, #-1, #0x1000
    Imm(i64),
    /// Symbol reference: function name, label, etc.
    Symbol(String),
    /// Symbol with addend: symbol+offset or symbol-offset
    SymbolOffset(String, i64),
    /// Memory operand: [base] or [base, #offset]
    Mem { base: String, offset: i64 },
    /// Memory operand with symbolic offset expression: [base, #(sym_expr)] or [base, #(sym_expr)]!
    /// Used when the offset is a label/symbol expression that can't be resolved at parse time.
    MemExpr {
        base: String,
        expr: String,
        writeback: bool,
    },
    /// Memory operand with pre-index writeback: [base, #offset]!
    MemPreIndex { base: String, offset: i64 },
    /// Memory operand with post-index writeback: [base], #offset
    MemPostIndex { base: String, offset: i64 },
    /// Memory operand with register offset: [base, Xm]
    MemRegOffset {
        base: String,
        index: String,
        extend: Option<String>,
        shift: Option<u8>,
    },
    /// :lo12:symbol or :got_lo12:symbol modifier
    Modifier { kind: String, symbol: String },
    /// :lo12:symbol+offset
    ModifierOffset {
        kind: String,
        symbol: String,
        offset: i64,
    },
    /// Shift: lsl #N, lsr #N, asr #N
    Shift { kind: String, amount: u32 },
    /// Extend: sxtw, uxtw, sxtx, etc. with optional shift amount
    Extend { kind: String, amount: u32 },
    /// Condition code for csel etc.: eq, ne, lt, gt, ...
    Cond(String),
    /// Barrier option for dmb/dsb: ish, ishld, ishst, sy, etc.
    Barrier(String),
    /// Label reference for branches
    Label(String),
    /// Raw expression (for things we can't fully parse yet)
    Expr(String),
    /// NEON register with arrangement specifier: v0.8b, v0.16b, v0.4s, etc.
    RegArrangement { reg: String, arrangement: String },
    /// NEON register with lane index: v0.d[1], v0.b[0], v0.s[2], etc.
    RegLane {
        reg: String,
        elem_size: String,
        index: u32,
    },
    /// NEON register list: {v0.16b}, {v0.16b, v1.16b}, etc.
    RegList(Vec<Operand>),
    /// NEON register list with element index: {v0.s, v1.s}[0], {v0.d, v1.d}[1], etc.
    RegListIndexed { regs: Vec<Operand>, index: u32 },
}

/// Section directive with optional flags and type.
#[derive(Debug, Clone)]
pub struct SectionDirective {
    pub name: String,
    pub flags: Option<String>,
    pub section_type: Option<String>,
}

/// Symbol kind from `.type` directive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Object,
    TlsObject,
    NoType,
}

/// Size expression: either a constant or `.-name` (current position minus symbol).
#[derive(Debug, Clone)]
pub enum SizeExpr {
    Constant(u64),
    CurrentMinusSymbol(String),
}

/// A data value that can be a constant, a symbol, or a symbol expression.
#[derive(Debug, Clone)]
pub enum DataValue {
    Integer(i64),
    /// Any operand naming a symbol, kept as written: the writer evaluates
    /// it in GAS's section algebra (`ElfWriterBase::capture_expr`), so its
    /// meaning never depends on where a textual split happened to cut it.
    Expr(String),
}

/// A typed assembly directive, fully parsed at parse time.
#[derive(Debug, Clone)]
pub enum AsmDirective {
    /// Switch to a named section: `.section .text,"ax",@progbits`
    Section(SectionDirective),
    /// Global symbol: `.globl name`
    Global(String),
    /// `.tlsdesccall sym`: R_AARCH64_TLSDESC_CALL on the next instruction.
    TlsDescCall(String),
    /// Weak symbol: `.weak name`
    Weak(String),
    /// Hidden visibility: `.hidden name`
    Hidden(String),
    /// Protected visibility: `.protected name`
    Protected(String),
    /// Internal visibility: `.internal name`
    Internal(String),
    /// Symbol type: `.type name, %function`
    SymbolType(String, SymbolKind),
    /// Symbol size: `.size name, expr`
    Size(String, SizeExpr),
    /// Alignment: `.align N` / `.p2align N[, fill[, max]]`.
    ///
    /// `bytes` is the alignment in bytes (already converted from 2^N).
    /// `max_pad` is the GAS third parameter: if the padding that would be
    /// required exceeds it, the directive is a no-op rather than over-aligning.
    /// `fill` is the GAS second parameter: when present, every pad byte is
    /// that value in every section (including `.text`). When absent,
    /// executable sections are NOP-padded at the architecture's instruction
    /// width and other sections are zero-padded.
    Align {
        bytes: u64,
        max_pad: Option<u64>,
        fill: Option<u8>,
    },
    /// Byte-alignment: `.balign N` (stored as byte count directly)
    Balign(u64),
    /// Emit bytes: `.byte val, val, ...` (can be symbol differences for size computations)
    Byte(Vec<DataValue>),
    /// Emit 16-bit values: `.short val, ...`
    Short(Vec<DataValue>),
    /// Emit 32-bit values: `.long val, ...` (can be symbol references)
    Long(Vec<DataValue>),
    /// Emit 64-bit values: `.quad val, ...` (can be symbol references)
    Quad(Vec<DataValue>),
    /// Emit zero bytes: `.zero N[, fill]`
    Zero(usize, u8),
    /// `.fill repeat, size, value` with a NON-ZERO value, kept symbolic
    /// until apply: the NOBITS raw-value law (GAS 2.47 rejects
    /// `.fill 4,1,256` in `.bss` although the element truncates to
    /// zeros) and the assembly-wide fill budget both need the live
    /// section — a parse-time `Vec` can see neither.
    Fill {
        total: usize,
        elem: usize,
        value: u64,
    },
    /// NUL-terminated string: `.asciz "str"`
    Asciz(Vec<u8>),
    /// String without NUL: `.ascii "str"`
    Ascii(Vec<u8>),
    /// Common symbol: `.comm name, size, align`
    /// `.comm name, size[, align]`; align 0 = omitted (see `ElfWriterBase::declare_comm`).
    Comm(String, u64, u64),
    /// `.lcomm name, size`.
    Lcomm(String, u64),
    /// Local symbol: `.local name`
    Local(String),
    /// Symbol alias: `.set name, value`
    Set(String, String),
    /// Push current section and switch to a new one: `.pushsection name,"flags",@type`
    PushSection(SectionDirective),
    /// Pop section stack and restore previous section: `.popsection`
    PopSection,
    /// `.previous` — swap current and previous sections
    Previous,
    /// `.subsection N` — switch to numbered subsection within the current section
    Subsection(u64),
    /// A `.cfi_*` directive: name and operand text. `cfi_lower::lower_cfi`
    /// replaces it by a location label and the `.eh_frame` data it implies.
    Cfi(String, String),
    /// DWARF location advance between two code labels, produced by the CFI
    /// lowering only.
    CfaAdvance { from: String, to: String },
    /// `.uleb128` / `.sleb128`.
    Leb128 {
        signed: bool,
        values: Vec<DataValue>,
    },
    /// `.incbin "file"[, skip[, count]]` — include binary file contents
    Incbin {
        path: String,
        skip: u64,
        count: Option<u64>,
    },
    /// Raw bytes emitted from .float, .double, etc.
    RawBytes(Vec<u8>),
    /// Literal pool dump: `.ltorg` or `.pool`
    Ltorg,
    /// `.org new-lc [, fill]` — pad the current section to `new-lc` bytes
    /// from its start with `fill` (default 0). Evaluated immediately by the
    /// ELF writer (`.` and already-defined labels resolve; forward
    /// references are an error).
    Org { expr: String, fill: u8 },
    /// Other ignored directives (.file, .loc, .ident, etc.)
    Ignored,
}

/// A parsed assembly statement.
#[derive(Debug, Clone)]
pub enum AsmStatement {
    /// A label definition: "name:"
    Label(String),
    /// A typed directive, fully parsed
    Directive(AsmDirective),
    /// An AArch64 instruction with mnemonic and operands
    Instruction {
        mnemonic: String,
        operands: Vec<Operand>,
        /// The raw text of the operand string (for fallback encoding)
        raw_operands: String,
    },
    /// An empty line or comment
    Empty,
    /// Literal pool load pseudo-instruction: `ldr Rd, =symbol[+offset]`
    /// Will be expanded into `ldr Rd, .Llpool_N` + pool entries by a later pass.
    LdrLiteralPool {
        reg: String,
        symbol: String,
        addend: i64,
    },
}

// C-style /* ... */ comment stripping is handled by asm_preprocess::strip_c_comments
// (shared, string-aware version that correctly handles `/*` inside quoted strings).

/// Parse assembly text into a list of statements.
/// Expand .rept/.endr, .irp/.endr, and .irpc/.endr blocks by repeating contained lines.
// TODO: extract expand_rept_blocks to shared module (duplicated in ARM, RISC-V, x86 parsers)
fn is_rept_start(trimmed: &str) -> bool {
    trimmed.starts_with(".rept ")
        || trimmed.starts_with(".rept\t")
        || trimmed.starts_with(".rep ")
        || trimmed.starts_with(".rep\t")
}

fn is_irp_start(trimmed: &str) -> bool {
    trimmed.starts_with(".irp ") || trimmed.starts_with(".irp\t")
}

fn is_irpc_start(trimmed: &str) -> bool {
    trimmed.starts_with(".irpc ") || trimmed.starts_with(".irpc\t")
}

fn is_block_start(trimmed: &str) -> bool {
    is_rept_start(trimmed) || is_irp_start(trimmed) || is_irpc_start(trimmed)
}

/// Collect the body lines of a .rept/.irp block, handling nesting.
/// Returns the body lines and advances i past the closing .endr.
fn collect_block_body<'a>(lines: &[&'a str], i: &mut usize) -> Result<Vec<&'a str>, String> {
    let mut depth = 1;
    let mut body = Vec::new();
    *i += 1;
    while *i < lines.len() {
        let inner = strip_comment(lines[*i]).trim().to_string();
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
        return Err(".rept/.irp/.irpc without matching .endr".to_string());
    }
    Ok(body)
}

/// Estimate the byte size of a single assembly line for label position tracking.
/// Used to resolve backward label references in .rept count expressions.
/// AArch64 instructions are always 4 bytes; directives have known sizes.
fn estimate_line_bytes(trimmed: &str) -> u64 {
    if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
        return 0;
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
        // .ascii "str" / .asciz "str" — approximate
        if lower.starts_with(".ascii")
            || lower.starts_with(".asciz")
            || lower.starts_with(".string")
        {
            // Don't try to be exact; these are rarely used in .rept context
            return 0;
        }
        // Other directives (.align, .section, .globl, .type, etc.) — 0 bytes of code
        return 0;
    }
    // Everything else is an instruction = 4 bytes for AArch64
    4
}

/// Resolve backward numeric label references (like 662b, 661b) in a .rept count expression.
/// Substitutes each backward reference with its byte position, then evaluates the expression.
fn resolve_rept_label_expr(
    count_str: &str,
    label_positions: &crate::common::fx_hash::FxHashMap<String, Vec<u64>>,
) -> Result<i64, String> {
    // First try direct evaluation (handles simple integer expressions)
    if let Ok(val) = asm_expr::parse_integer_expr(count_str) {
        return Ok(val);
    }

    // Check if expression contains backward label references (e.g., 662b)
    let mut resolved = count_str.to_string();
    let mut found_label_ref = false;

    // Find and replace backward label references: digits followed by 'b' or 'B'
    // We need to scan for patterns like "662b" that are numeric label backward refs
    loop {
        let mut replaced = false;
        // Use a simple scan to find numeric label backward references
        let bytes = resolved.as_bytes();
        let len = bytes.len();
        let mut i = 0;
        while i < len {
            // Look for a digit sequence followed by 'b' or 'B'
            if bytes[i].is_ascii_digit() {
                let start = i;
                while i < len && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i < len && (bytes[i] == b'b' || bytes[i] == b'B') {
                    // Check that the next char (if any) is not alphanumeric
                    // (to avoid matching things like "0b1010" binary literals)
                    let after_ok = i + 1 >= len || !bytes[i + 1].is_ascii_alphanumeric();
                    // Also ensure it's not a binary literal starting with 0b
                    let is_binary = start + 1 == i && bytes[start] == b'0';
                    if after_ok && !is_binary {
                        let label_num = &resolved[start..i];
                        let ref_end = i + 1;
                        // Look up the most recent definition of this label
                        if let Some(positions) = label_positions.get(label_num) {
                            if let Some(&pos) = positions.last() {
                                let before = &resolved[..start];
                                let after = &resolved[ref_end..];
                                resolved = format!("{}{}{}", before, pos, after);
                                found_label_ref = true;
                                replaced = true;
                                break; // restart scan since string changed
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
        asm_expr::parse_integer_expr(&resolved)
    } else {
        Err(format!("cannot evaluate .rept count: {}", count_str))
    }
}

fn expand_rept_blocks(lines: &[&str]) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    let mut i = 0;
    // Track numeric label positions (byte offsets) for resolving backward refs
    let mut label_positions: crate::common::fx_hash::FxHashMap<String, Vec<u64>> =
        crate::common::fx_hash::FxHashMap::default();
    let mut current_byte_pos: u64 = 0;
    // Repetition-only growth totals (see asm_preprocess::check_expansion_growth).
    let mut grown_lines: u64 = 0;
    let mut grown_bytes: u64 = 0;
    while i < lines.len() {
        let trimmed = strip_comment(lines[i]).trim().to_string();
        if is_rept_start(&trimmed) {
            let prefix_len = if trimmed.starts_with(".rept") { 5 } else { 4 };
            let count_str = trimmed[prefix_len..].trim();
            // Propagate an unresolvable count as an error: the old
            // `unwrap_or(0)` accepted `.rept <garbage>` as zero
            // repetitions, which is fail-open (GAS reports "cannot
            // evaluate .rept count" and fails the assembly).  Negative
            // counts wrap to `size_t` like GAS (`.rept -1` -> rejected
            // by the budget as count 18446744073709551615).
            let count_val = resolve_rept_label_expr(count_str, &label_positions)
                .map_err(|e| format!(".rept: bad count '{}': {}", count_str, e))?;
            let count = count_val as u64;
            let body = collect_block_body(lines, &mut i)?;
            // Bound the raw block before recursing — the same position in
            // the flow as GAS's `do_repeat` check (pre-expansion).
            asm_preprocess::check_rept_budget(count, &body)?;
            let expanded_body = expand_rept_blocks(&body)?;
            let (add_lines, add_bytes) = asm_preprocess::expansion_size(&expanded_body);
            grown_lines = grown_lines.saturating_add(count.saturating_mul(add_lines));
            grown_bytes = grown_bytes.saturating_add(count.saturating_mul(add_bytes));
            asm_preprocess::check_expansion_growth(".rept", grown_lines, grown_bytes)?;
            for _ in 0..count {
                result.extend(expanded_body.iter().cloned());
            }
        } else if is_irp_start(&trimmed) {
            // .irp var, val1, val2, ...
            let args_str = trimmed[".irp".len()..].trim();
            // Split on first comma to get variable name and values
            let (var, values_str) = match args_str.find(',') {
                Some(pos) => (args_str[..pos].trim(), args_str[pos + 1..].trim()),
                None => (args_str, ""),
            };
            let values: Vec<&str> = values_str.split(',').map(|s| s.trim()).collect();
            let body = collect_block_body(lines, &mut i)?;
            for val in &values {
                // Substitute \var with val in each body line
                let subst_body: Vec<String> = body
                    .iter()
                    .map(|line| {
                        let pattern = format!("\\{}", var);
                        asm_preprocess::replace_macro_param(line, &pattern, val)
                    })
                    .collect();
                let subst_refs: Vec<&str> = subst_body.iter().map(|s| s.as_str()).collect();
                let expanded = expand_rept_blocks(&subst_refs)?;
                let (add_lines, add_bytes) = asm_preprocess::expansion_size(&expanded);
                grown_lines = grown_lines.saturating_add(add_lines);
                grown_bytes = grown_bytes.saturating_add(add_bytes);
                asm_preprocess::check_expansion_growth(".irp", grown_lines, grown_bytes)?;
                result.extend(expanded);
            }
        } else if is_irpc_start(&trimmed) {
            // .irpc var, string — iterate over each character in the string
            let args_str = trimmed[".irpc".len()..].trim();
            // Split on first comma to get variable name and string
            let (var, char_str) = match args_str.find(',') {
                Some(pos) => (args_str[..pos].trim(), args_str[pos + 1..].trim()),
                None => (args_str, ""),
            };
            let body = collect_block_body(lines, &mut i)?;
            for ch in char_str.chars() {
                // Substitute \var with the current character in each body line
                let ch_str = ch.to_string();
                let subst_body: Vec<String> = body
                    .iter()
                    .map(|line| {
                        let pattern = format!("\\{}", var);
                        let substituted =
                            asm_preprocess::replace_macro_param(line, &pattern, &ch_str);
                        // Strip GAS macro argument delimiters: \() resolves to empty string
                        substituted.replace("\\()", "")
                    })
                    .collect();
                let subst_refs: Vec<&str> = subst_body.iter().map(|s| s.as_str()).collect();
                let expanded = expand_rept_blocks(&subst_refs)?;
                let (add_lines, add_bytes) = asm_preprocess::expansion_size(&expanded);
                grown_lines = grown_lines.saturating_add(add_lines);
                grown_bytes = grown_bytes.saturating_add(add_bytes);
                asm_preprocess::check_expansion_growth(".irpc", grown_lines, grown_bytes)?;
                result.extend(expanded);
            }
        } else if trimmed == ".endr" {
            // stray .endr without .rept - skip
        } else {
            // Track numeric label definitions and byte positions
            // Check if this line defines a numeric label (e.g., "661:" or "661: instruction")
            if let Some(colon_pos) = trimmed.find(':') {
                let before = &trimmed[..colon_pos];
                if !before.is_empty() && before.chars().all(|c| c.is_ascii_digit()) {
                    label_positions
                        .entry(before.to_string())
                        .or_default()
                        .push(current_byte_pos);
                }
            }
            current_byte_pos += estimate_line_bytes(&trimmed);
            result.push(lines[i].to_string());
        }
        i += 1;
    }
    Ok(result)
}

/// Evaluate a `.if` condition expression using the shared implementation.
/// Supports: `==`, `!=`, `>`, `>=`, `<`, `<=`, `||`, `&&`, parentheses.
fn eval_if_condition(cond: &str) -> bool {
    asm_preprocess::eval_if_condition(cond)
}

/// Resolve .set/.equ constants with simple integer values in instruction lines.
///
/// Sequential single-pass: as each `.set name, value` is encountered, update the
/// constant map. For non-.set lines, substitute whole-word occurrences of known
/// constant names with their current values. This correctly handles constants that
/// are reassigned (e.g., `.Lasm_alt_mode` in kernel ALTERNATIVE macros).
/// The `.set` directives themselves are preserved for the encoder to process.
fn resolve_set_constants(lines: &[String]) -> Vec<String> {
    use crate::common::fx_hash::FxHashMap;
    let mut constants: FxHashMap<String, String> = FxHashMap::default();
    let mut result = Vec::with_capacity(lines.len());

    // Single sequential pass: update constants as .set directives are encountered,
    // and substitute using the current map state at each line. This correctly handles
    // constants that are reassigned (e.g., .Lasm_alt_mode in kernel ALTERNATIVE macros).
    for line in lines {
        let trimmed = strip_comment(line).trim().to_lowercase();
        if trimmed.starts_with(".set ")
            || trimmed.starts_with(".set\t")
            || trimmed.starts_with(".equ ")
            || trimmed.starts_with(".equ\t")
        {
            let rest = strip_comment(line).trim();
            let directive_len = 4; // both .set and .equ are 4 chars
            let args = rest[directive_len..].trim();
            if let Some(comma_pos) = args.find(',') {
                let name = args[..comma_pos].trim().to_string();
                let value_str = args[comma_pos + 1..].trim().to_string();
                // Only resolve simple integer values
                if let Ok(_val) = asm_expr::parse_integer_expr(&value_str) {
                    constants.insert(name, value_str);
                }
            }
            result.push(line.clone());
        } else if constants.is_empty() {
            result.push(line.clone());
        } else {
            // Substitute current constant values in this line
            let mut substituted = line.clone();
            for (name, value) in &constants {
                // Replace whole-word occurrences of the constant name
                let mut new_result = String::with_capacity(substituted.len());
                let bytes = substituted.as_bytes();
                let name_bytes = name.as_bytes();
                let mut i = 0;
                while i < bytes.len() {
                    if i + name_bytes.len() <= bytes.len()
                        && &bytes[i..i + name_bytes.len()] == name_bytes
                    {
                        // Check word boundary before
                        let before_ok = i == 0 || !is_ident_char(bytes[i - 1]);
                        // Check word boundary after
                        let after_pos = i + name_bytes.len();
                        let after_ok = after_pos >= bytes.len() || !is_ident_char(bytes[after_pos]);
                        if before_ok && after_ok {
                            new_result.push_str(value);
                            i += name_bytes.len();
                            continue;
                        }
                    }
                    new_result.push(bytes[i] as char);
                    i += 1;
                }
                substituted = new_result;
            }
            result.push(substituted);
        }
    }

    result
}

/// Check if a byte is a valid identifier character (alphanumeric, underscore, dot).
fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'.'
}

/// Resolve `.req` / `.unreq` register aliases.
///
/// First pass: collect `name .req register` definitions and `.unreq name` removals.
/// Second pass: substitute alias names with their register values in all non-directive lines.
/// The `.req` / `.unreq` lines themselves are preserved (they'll be parsed as Empty later).
fn resolve_register_aliases(lines: &[String]) -> Vec<String> {
    use crate::common::fx_hash::FxHashMap;
    let mut aliases: FxHashMap<String, String> = FxHashMap::default();

    // First pass: collect alias definitions and removals in order
    for line in lines {
        let trimmed = strip_comment(line).trim();
        // Match "name .req register"
        if let Some(req_pos) = trimmed.find(".req") {
            // Ensure it's " .req " or "\t.req " (not part of a longer directive like .reqXYZ)
            let after_req = req_pos + 4;
            if req_pos > 0
                && (trimmed.as_bytes()[req_pos - 1] == b' '
                    || trimmed.as_bytes()[req_pos - 1] == b'\t')
                && after_req < trimmed.len()
                && (trimmed.as_bytes()[after_req] == b' ' || trimmed.as_bytes()[after_req] == b'\t')
            {
                let name = trimmed[..req_pos].trim();
                let register = trimmed[after_req..].trim();
                if !name.is_empty() && !register.is_empty() {
                    aliases.insert(name.to_string(), register.to_string());
                }
                continue;
            }
        }
        // Note: We intentionally DON'T process .unreq here.
        // .unreq is used to allow re-definition of aliases in different scopes,
        // but for our two-pass approach, we want ALL aliases available for substitution.
        // The .unreq directive is handled by the parser as an ignored directive.
    }

    if aliases.is_empty() {
        return lines.to_vec();
    }

    // Sort aliases by name length (longest first) to avoid partial substitution
    let mut sorted_aliases: Vec<(&String, &String)> = aliases.iter().collect();
    sorted_aliases.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

    // Second pass: substitute aliases in all lines
    lines
        .iter()
        .map(|line| {
            let trimmed = strip_comment(line).trim();
            // Skip .req/.unreq definition lines — leave them as-is
            if trimmed.contains(" .req ")
                || trimmed.contains("\t.req\t")
                || trimmed.contains("\t.req ")
                || trimmed.contains(" .unreq ")
                || trimmed.contains("\t.unreq\t")
                || trimmed.contains("\t.unreq ")
            {
                return line.clone();
            }
            // Skip directives (starting with .) — aliases are only used in instructions
            if trimmed.starts_with('.') {
                return line.clone();
            }
            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
                return line.clone();
            }
            let mut result = line.clone();
            for (name, register) in &sorted_aliases {
                let name_bytes = name.as_bytes();
                let mut new_result = String::with_capacity(result.len());
                let bytes = result.as_bytes();
                let mut i = 0;
                while i < bytes.len() {
                    if i + name_bytes.len() <= bytes.len()
                        && &bytes[i..i + name_bytes.len()] == name_bytes
                    {
                        // Check word boundary before
                        let before_ok = i == 0 || !is_alias_ident_char(bytes[i - 1]);
                        // Check word boundary after
                        let after_pos = i + name_bytes.len();
                        let after_ok =
                            after_pos >= bytes.len() || !is_alias_ident_char(bytes[after_pos]);
                        if before_ok && after_ok {
                            new_result.push_str(register);
                            i += name_bytes.len();
                            continue;
                        }
                    }
                    new_result.push(bytes[i] as char);
                    i += 1;
                }
                result = new_result;
            }
            result
        })
        .collect()
}

/// Check if a byte is a valid identifier character for register alias matching.
/// Similar to is_ident_char but does NOT include '.' since register names like
/// "v0.8h" should allow alias substitution where the alias is followed by '.'.
fn is_alias_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

pub fn parse_asm(text: &str) -> Result<Vec<AsmStatement>, String> {
    // Pre-process: strip C-style /* ... */ comments
    let text = asm_preprocess::strip_c_comments(text);

    // Split lines on ';' (GAS statement separator) before macro expansion,
    // so macro invocations after ';' on the same line get expanded correctly.
    // Strip // and @ line comments BEFORE splitting on semicolons, so that
    // semicolons inside comments (e.g. "// struct {int a;} *p;") don't cause
    // spurious splits.
    let presplit: Vec<String> = text
        .lines()
        .flat_map(|line| {
            let line = strip_comment(line);
            split_on_semicolons(line)
                .into_iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        })
        .collect();
    let presplit_refs: Vec<&str> = presplit.iter().map(|s| s.as_str()).collect();

    // Expand .macro/.endm definitions and invocations (the shared GAS
    // expander; comments are already stripped, and the ARM style keeps
    // `\@` and `@type` operands intact on the second pass).
    let macro_expanded = asm_preprocess::expand_macros(
        &presplit_refs,
        &asm_preprocess::CommentStyle::SlashSlashAndAt,
    )?;
    let macro_refs: Vec<&str> = macro_expanded.iter().map(|s| s.as_str()).collect();

    // Expand .rept/.endr blocks
    let expanded_lines = expand_rept_blocks(&macro_refs)?;

    // Resolve .set/.equ constants in expressions
    let expanded_lines = resolve_set_constants(&expanded_lines);

    // Resolve .req/.unreq register aliases
    let expanded_lines = resolve_register_aliases(&expanded_lines);

    let mut statements = Vec::new();
    // Stack for .if/.else/.endif conditional assembly.
    // Each entry is (active, any_taken): active = current block emitting code,
    // any_taken = whether any branch in this if/elseif/else chain was taken.
    let mut if_stack: Vec<(bool, bool)> = Vec::new();
    // Track defined symbols for .ifdef/.ifndef
    let mut defined_symbols: crate::common::fx_hash::FxHashSet<String> =
        crate::common::fx_hash::FxHashSet::default();
    for (line_num, line) in expanded_lines.iter().enumerate() {
        let line = asm_expr::trim_asm_line(line);

        // Skip empty lines
        if line.is_empty() {
            statements.push(AsmStatement::Empty);
            continue;
        }

        // Strip comments (// style)
        let line = strip_comment(line);
        let line = asm_expr::trim_asm_line(line);
        if line.is_empty() {
            statements.push(AsmStatement::Empty);
            continue;
        }

        // Handle .if/.else/.endif before anything else
        let lower = line.to_ascii_lowercase();
        if lower.starts_with(".endif") {
            if if_stack.pop().is_none() {
                return Err(format!(
                    "Line {}: .endif without matching .if",
                    line_num + 1
                ));
            }
            continue;
        }
        // .elseif / .elsif — else-if branch (must be checked BEFORE .else)
        if lower.starts_with(".elseif ")
            || lower.starts_with(".elseif\t")
            || lower.starts_with(".elsif ")
            || lower.starts_with(".elsif\t")
        {
            let len = if_stack.len();
            if len > 0 {
                let parent_active = if len >= 2 { if_stack[len - 2].0 } else { true };
                if if_stack[len - 1].1 || !parent_active {
                    if_stack[len - 1].0 = false;
                } else {
                    let keyword_len = if lower.starts_with(".elseif") { 7 } else { 6 };
                    let cond_str = line[keyword_len..].trim();
                    let result = eval_if_condition(cond_str);
                    if_stack[len - 1].0 = result;
                    if result {
                        if_stack[len - 1].1 = true;
                    }
                }
            }
            continue;
        }
        if lower == ".else"
            || (lower.starts_with(".else")
                && !lower.starts_with(".elseif")
                && !lower.starts_with(".elsif")
                && lower.len() >= 5
                && (lower.len() == 5 || lower.as_bytes()[5].is_ascii_whitespace()))
        {
            let len = if_stack.len();
            if len > 0 {
                let parent_active = if len >= 2 { if_stack[len - 2].0 } else { true };
                if if_stack[len - 1].1 || !parent_active {
                    if_stack[len - 1].0 = false;
                } else {
                    if_stack[len - 1].0 = true;
                    if_stack[len - 1].1 = true;
                }
            }
            continue;
        }
        if lower == ".if" || lower.starts_with(".if ") || lower.starts_with(".if\t") {
            let cond_str = if line.len() > 3 { line[3..].trim() } else { "" };
            // Evaluate the condition: if we're already in a false block, push false
            let active = if if_stack.last().map(|&(a, _)| a).unwrap_or(true) {
                if cond_str.is_empty() {
                    false
                } else {
                    eval_if_condition(cond_str)
                }
            } else {
                false
            };
            if_stack.push((active, active));
            continue;
        }
        // .ifc string1, string2  — conditional if strings are equal
        if lower.starts_with(".ifc ") || lower.starts_with(".ifc\t") {
            let args = line[4..].trim();
            let active = if if_stack.last().map(|&(a, _)| a).unwrap_or(true) {
                if let Some((a, b)) = args.split_once(',') {
                    a.trim() == b.trim()
                } else {
                    false
                }
            } else {
                false
            };
            if_stack.push((active, active));
            continue;
        }
        // .ifnc string1, string2  — conditional if strings are NOT equal
        if lower.starts_with(".ifnc ") || lower.starts_with(".ifnc\t") {
            let args = line[5..].trim();
            let active = if if_stack.last().map(|&(a, _)| a).unwrap_or(true) {
                if let Some((a, b)) = args.split_once(',') {
                    a.trim() != b.trim()
                } else {
                    true
                }
            } else {
                false
            };
            if_stack.push((active, active));
            continue;
        }
        // .ifb string — conditional if string is blank
        if lower == ".ifb" || lower.starts_with(".ifb ") || lower.starts_with(".ifb\t") {
            let arg = if line.len() > 4 { line[4..].trim() } else { "" };
            let active = if if_stack.last().map(|&(a, _)| a).unwrap_or(true) {
                arg.is_empty()
            } else {
                false
            };
            if_stack.push((active, active));
            continue;
        }
        // .ifnb string — conditional if string is NOT blank
        if lower == ".ifnb" || lower.starts_with(".ifnb ") || lower.starts_with(".ifnb\t") {
            let arg = if line.len() > 5 { line[5..].trim() } else { "" };
            let active = if if_stack.last().map(|&(a, _)| a).unwrap_or(true) {
                !arg.is_empty()
            } else {
                false
            };
            if_stack.push((active, active));
            continue;
        }
        // .ifdef symbol — conditional if symbol is defined
        if lower.starts_with(".ifdef ") || lower.starts_with(".ifdef\t") {
            let sym = line[6..].trim().to_string();
            let active = if if_stack.last().map(|&(a, _)| a).unwrap_or(true) {
                defined_symbols.contains(&sym)
            } else {
                false
            };
            if_stack.push((active, active));
            continue;
        }
        // .ifndef symbol — conditional if symbol is NOT defined
        if lower.starts_with(".ifndef ") || lower.starts_with(".ifndef\t") {
            let sym = line[7..].trim().to_string();
            let active = if if_stack.last().map(|&(a, _)| a).unwrap_or(true) {
                !defined_symbols.contains(&sym)
            } else {
                false
            };
            if_stack.push((active, active));
            continue;
        }
        // .ifeq expression — conditional if expression equals 0
        if lower.starts_with(".ifeq ") || lower.starts_with(".ifeq\t") {
            let expr_str = line[5..].trim();
            let active = if if_stack.last().map(|&(a, _)| a).unwrap_or(true) {
                asm_expr::parse_integer_expr(expr_str).unwrap_or(1) == 0
            } else {
                false
            };
            if_stack.push((active, active));
            continue;
        }
        // .ifne expression — conditional if expression is not equal to 0
        if lower.starts_with(".ifne ") || lower.starts_with(".ifne\t") {
            let expr_str = line[5..].trim();
            let active = if if_stack.last().map(|&(a, _)| a).unwrap_or(true) {
                asm_expr::parse_integer_expr(expr_str).unwrap_or(0) != 0
            } else {
                false
            };
            if_stack.push((active, active));
            continue;
        }
        // If we're inside a false .if block, skip this line
        if !if_stack.last().map(|&(a, _)| a).unwrap_or(true) {
            continue;
        }

        // In AArch64 GAS, '#' at the start of a line is a comment character.
        // This covers: C preprocessor line markers (# 123 "file"), and
        // comment-producing macros (e.g., FFmpeg's FUNC expanding to '#').
        if line.starts_with('#') {
            statements.push(AsmStatement::Empty);
            continue;
        }

        // Handle ';' as statement separator (GAS syntax).
        // Split the line on ';' and parse each part independently.
        let parts = split_on_semicolons(line);
        for part in parts {
            let part = asm_expr::trim_asm_line(part);
            if part.is_empty() {
                continue;
            }
            match parse_line(part) {
                Ok(stmts) => {
                    for stmt in &stmts {
                        match stmt {
                            AsmStatement::Label(name) => {
                                defined_symbols.insert(name.clone());
                            }
                            AsmStatement::Directive(AsmDirective::Set(name, _)) => {
                                defined_symbols.insert(name.clone());
                            }
                            _ => {}
                        }
                    }
                    statements.extend(stmts);
                }
                Err(e) => return Err(format!("Line {}: {}: '{}'", line_num + 1, e, part)),
            }
        }
    }
    Ok(statements)
}

/// Split a line on ';' characters, respecting strings and comments.
/// In GAS syntax, ';' separates multiple statements on the same line.
/// Stops splitting once a `//` or `@` line comment is encountered (outside strings),
/// so semicolons inside comments are not treated as statement separators.
fn split_on_semicolons(line: &str) -> Vec<&str> {
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
                i = asm_expr::skip_gas_char_literal(bytes, i);
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => break,
            b'@' => {
                let after = &line[i + 1..];
                if !after.starts_with("object")
                    && !after.starts_with("function")
                    && !after.starts_with("progbits")
                    && !after.starts_with("nobits")
                    && !after.starts_with("tls_object")
                    && !after.starts_with("note")
                {
                    break;
                }
                i += 1;
            }
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

fn strip_comment(line: &str) -> &str {
    crate::backend::asm_preprocess::strip_comment(
        line,
        &crate::backend::asm_preprocess::CommentStyle::SlashSlashAndAt,
    )
}

/// Try to parse `ldr Rd, =symbol[+offset]` pseudo-instruction.
///
/// In GAS, `ldr Rd, =expr` loads a 64-bit value via a literal pool.
/// Returns a `LdrLiteralPool` statement that will be expanded into
/// `ldr Rd, .Llpool_N` + pool entries by the `expand_literal_pools` pass.
fn try_expand_ldr_literal(line: &str) -> Option<Result<Vec<AsmStatement>, String>> {
    let lower = line.to_ascii_lowercase();
    // Match: ldr xN, =symbol or ldr wN, =symbol (32-bit registers use adrp too on AArch64)
    if !lower.starts_with("ldr ") && !lower.starts_with("ldr\t") {
        return None;
    }
    let rest = line[3..].trim();
    // Find the comma separating register from operand
    let comma_pos = rest.find(',')?;
    let reg = rest[..comma_pos].trim();
    let operand = rest[comma_pos + 1..].trim();
    if !operand.starts_with('=') {
        return None;
    }
    let expr = operand[1..].trim();
    // Parse symbol and optional +offset, e.g. =coeffs+8
    let (symbol, addend) = if let Some(plus_pos) = expr.rfind('+') {
        let sym = expr[..plus_pos].trim();
        let off_str = expr[plus_pos + 1..].trim();
        if let Ok(off) = if off_str.starts_with("0x") || off_str.starts_with("0X") {
            i64::from_str_radix(&off_str[2..], 16)
        } else {
            off_str.parse::<i64>()
        } {
            (sym, off)
        } else {
            (expr, 0i64)
        }
    } else if let Some(minus_pos) = expr.rfind('-') {
        // Only treat as symbol-offset if there's a valid symbol before the minus
        let sym = expr[..minus_pos].trim();
        let off_str = expr[minus_pos + 1..].trim();
        if !sym.is_empty() && !sym.ends_with(['+', '-']) {
            if let Ok(off) = if off_str.starts_with("0x") || off_str.starts_with("0X") {
                i64::from_str_radix(&off_str[2..], 16)
            } else {
                off_str.parse::<i64>()
            } {
                (sym, -off)
            } else {
                (expr, 0i64)
            }
        } else {
            (expr, 0i64)
        }
    } else {
        (expr, 0i64)
    };
    Some(Ok(vec![AsmStatement::LdrLiteralPool {
        reg: reg.to_string(),
        symbol: symbol.to_string(),
        addend,
    }]))
}

fn parse_line(line: &str) -> Result<Vec<AsmStatement>, String> {
    // Check for label definition (name:)
    // Labels can be at the start of the line, possibly followed by an instruction
    if let Some(colon_pos) = line.find(':') {
        let potential_label = &line[..colon_pos].trim();
        // Verify it looks like a valid label (no spaces before colon, alphanumeric + _ + .)
        if !potential_label.is_empty()
            && !potential_label.contains(' ')
            && !potential_label.contains('\t')
            && !potential_label.starts_with('.')  // Could be a directive
            || potential_label.starts_with(".L") // Local labels start with .L
            || potential_label.starts_with(".Lstr") // String labels
            || potential_label.starts_with(".Lmemcpy")
            || potential_label.starts_with(".Lskip")
        {
            // Check if this is actually a directive like ".section .rodata"
            if potential_label.starts_with('.')
                && !potential_label.starts_with(".L")
                && !potential_label.starts_with(".l")
            {
                // This is a directive, not a label
            } else {
                let mut result = vec![AsmStatement::Label(potential_label.to_string())];
                // Check for instruction/directive after the label on the same line
                let rest = line[colon_pos + 1..].trim();
                if !rest.is_empty() {
                    result.extend(parse_line(rest)?);
                }
                return Ok(result);
            }
        }
    }

    let trimmed = asm_expr::trim_asm_line(line);

    // Register alias: "name .req register" or "name .unreq register"
    // These define register aliases and can be safely ignored.
    if trimmed.contains(" .req ")
        || trimmed.contains("\t.req\t")
        || trimmed.contains("\t.req ")
        || trimmed.contains(" .unreq ")
        || trimmed.contains("\t.unreq\t")
        || trimmed.contains("\t.unreq ")
    {
        return Ok(vec![AsmStatement::Empty]);
    }

    // Directive: starts with .
    if trimmed.starts_with('.') {
        return Ok(vec![parse_directive(trimmed)?]);
    }

    // Handle ldr Rd, =symbol pseudo-instruction (creates LdrLiteralPool for later expansion)
    if let Some(expanded) = try_expand_ldr_literal(trimmed) {
        return expanded;
    }

    // Instruction
    Ok(vec![parse_instruction(trimmed)?])
}

fn parse_directive(line: &str) -> Result<AsmStatement, String> {
    // Split directive name from arguments
    let (name, args) = if line.starts_with(".inst") && line.len() > 5 && line.as_bytes()[5] == b'('
    {
        // Handle .inst(expr) without space: ".inst(0x...)" -> name=".inst", args="(0x...)"
        (".inst", line[5..].trim())
    } else if let Some(space_pos) = line.find([' ', '\t']) {
        let name = &line[..space_pos];
        let args = asm_expr::trim_asm_line(&line[space_pos..]);
        (name, args)
    } else {
        (line, "")
    };

    let dir = match name {
        ".section" => parse_section_directive(args)?,
        ".tlsdesccall" => {
            let sym = args.trim();
            if sym.is_empty() || sym.contains([' ', '\t', ',']) {
                return Err(format!(".tlsdesccall needs one symbol, got `{args}`"));
            }
            AsmDirective::TlsDescCall(sym.to_string())
        }
        ".text" => AsmDirective::Section(SectionDirective {
            name: ".text".to_string(),
            flags: None,
            section_type: None,
        }),
        ".data" => AsmDirective::Section(SectionDirective {
            name: ".data".to_string(),
            flags: None,
            section_type: None,
        }),
        ".bss" => AsmDirective::Section(SectionDirective {
            name: ".bss".to_string(),
            flags: None,
            section_type: None,
        }),
        ".rodata" => AsmDirective::Section(SectionDirective {
            name: ".rodata".to_string(),
            flags: None,
            section_type: None,
        }),
        ".globl" | ".global" => AsmDirective::Global(args.trim().to_string()),
        ".weak" => AsmDirective::Weak(args.trim().to_string()),
        ".hidden" => AsmDirective::Hidden(args.trim().to_string()),
        ".protected" => AsmDirective::Protected(args.trim().to_string()),
        ".internal" => AsmDirective::Internal(args.trim().to_string()),
        ".type" => parse_type_directive(args)?,
        ".size" => parse_size_directive(args)?,
        ".align" | ".p2align" => {
            // GAS: .p2align align[, fill[, max]]
            // align is log2(bytes). The third parameter is a padding cap:
            // if honouring the alignment would insert more than `max` bytes,
            // the directive is skipped. Ignoring it made lccc emit three
            // NOPs where GAS emitted one for `.p2align 4,,10` at offset 4,
            // shifting every later branch target (measured on the ARM
            // codegen corpus).
            let mut parts = args.trim().split(',');
            let align_tok = parts.next().map(str::trim).unwrap_or("");
            let align_raw: i64 = parse_int_literal(align_tok)
                .map_err(|_| format!(".p2align: alignment `{align_tok}` is not an integer"))?;
            // GAS (read.c, measured 2.47): an out-of-range exponent —
            // negative or > 63 — warns and clamps to 63 rather than
            // erroring; 63 lands in the writer's signed-shift bucket
            // (`p2align 63 @0` succeeds unraised, `@1` refuses to pad).
            // Note a negative exponent clamps to 63, NOT to 0: measured
            // `.p2align -1` behaves exactly like `.p2align 63`.
            let align_exp: u64 = if (0..=63).contains(&align_raw) {
                align_raw as u64
            } else {
                63
            };
            let fill_tok = parts.next().map(str::trim);
            let fill = match fill_tok {
                None | Some("") => None,
                Some(tok) => {
                    let v = parse_int_literal(tok)
                        .map_err(|_| format!(".p2align: fill `{tok}` is not an integer"))?;
                    // GAS takes any integer fill and stores the low byte
                    // (measured: -1 -> ff, -129 -> 7f, 300 -> 2c, 256 -> 00).
                    Some(v as u8)
                }
            };
            let max_tok = parts.next().map(str::trim);
            let max_pad = match max_tok {
                None | Some("") => None,
                Some(tok) => {
                    let v = parse_int_literal(tok)
                        .map_err(|_| format!(".p2align: max-pad `{tok}` is not an integer"))?;
                    // GAS treats a negative cap as unlimited (same as
                    // omitted / zero). Do not turn it into a hard error.
                    if v <= 0 { None } else { Some(v as u64) }
                }
            };
            if parts.next().is_some() {
                return Err(".p2align: extra operands after max-pad".to_string());
            }
            AsmDirective::Align {
                bytes: 1u64 << align_exp,
                max_pad,
                fill,
            }
        }
        ".balign" => {
            // GAS: .balign bytes[, fill[, max]] — same fill/max rule as
            // .p2align, but the first field is a byte count, not log2.
            // Measured on GAS 2.47: 0 is a no-op, non-powers-of-two and
            // negative values are "alignment not a power of 2".
            let mut parts = args.trim().split(',');
            let align_tok = parts.next().map(str::trim).unwrap_or("");
            // A bare `.balign` (no operand) is a GAS-accepted no-op;
            // parse it as align 1 instead of an integer-parse error.
            let align_val: i64 = match align_tok {
                "" => 1,
                tok => parse_int_literal(tok)
                    .map_err(|_| format!(".balign: alignment `{tok}` is not an integer"))?,
            };
            match align_val {
                0 => {} // GAS no-op
                n if n > 0 && (n as u64).is_power_of_two() => {}
                n => {
                    return Err(format!(".balign: alignment {n} is not a power of 2"));
                }
            }
            let fill_tok = parts.next().map(str::trim);
            let fill = match fill_tok {
                None | Some("") => None,
                Some(tok) => {
                    let v = parse_int_literal(tok)
                        .map_err(|_| format!(".balign: fill `{tok}` is not an integer"))?;
                    Some(v as u8)
                }
            };
            let max_tok = parts.next().map(str::trim);
            let max_pad = match max_tok {
                None | Some("") => None,
                Some(tok) => {
                    let v = parse_int_literal(tok)
                        .map_err(|_| format!(".balign: max-pad `{tok}` is not an integer"))?;
                    if v <= 0 { None } else { Some(v as u64) }
                }
            };
            if parts.next().is_some() {
                return Err(".balign: extra operands after max-pad".to_string());
            }
            AsmDirective::Align {
                // 0 is GAS's no-op; 1 makes align_to_capped return early,
                // which is exactly that.
                bytes: if align_val == 0 { 1 } else { align_val as u64 },
                max_pad,
                fill,
            }
        }
        ".uleb128" | ".sleb128" => AsmDirective::Leb128 {
            signed: name == ".sleb128",
            values: parse_data_values(args)?,
        },
        ".byte" => {
            let vals = parse_data_values(args)?;
            AsmDirective::Byte(vals)
        }
        ".short" | ".hword" | ".2byte" | ".half" => AsmDirective::Short(parse_data_values(args)?),
        ".long" | ".4byte" | ".word" | ".int" | ".inst" => {
            let vals = parse_data_values(args)?;
            AsmDirective::Long(vals)
        }
        ".quad" | ".8byte" | ".xword" | ".dword" => {
            let vals = parse_data_values(args)?;
            AsmDirective::Quad(vals)
        }
        ".zero" | ".space" | ".skip" => {
            let parts: Vec<&str> = args.trim().split(',').collect();
            // GAS: a negative size emits nothing (`.zero -1` is accepted
            // as a no-op, measured 2.47); the old `as usize` cast turned
            // -1 into a request for 2^64 bytes.
            let size_raw = parse_int_literal(parts[0].trim())
                .map_err(|_| format!("invalid .zero size: {}", args))?;
            let size: usize = if size_raw <= 0 { 0 } else { size_raw as usize };
            let fill: u8 = if parts.len() > 1 {
                parse_data_value(parts[1].trim())? as u8
            } else {
                0
            };
            AsmDirective::Zero(size, fill)
        }
        ".fill" => {
            // .fill repeat, size, value
            //
            // GAS verdicts (2.47 measured): a non-positive repeat or size
            // emits nothing (`.fill 2^64-1,1,0` parses as repeat -1 ->
            // zero bytes); a repeat*size product that overflows likewise
            // emits nothing (`.fill 2^62,4,0` -> zero bytes, exit 0); the
            // element is capped at 8 bytes (`.fill 1,9,0` -> 8 bytes);
            // the value is truncated to the element (low bytes of its
            // two's-complement form). The old path did all of this with
            // wrapping `as u64` casts — `.fill -1,1,0xff` hit
            // `Vec::with_capacity(u64::MAX)` and aborted, and a positive
            // overflow silently produced a *different* byte count.
            let parts: Vec<&str> = args.splitn(3, ',').collect();
            let repeat = parse_int_literal(parts[0].trim())
                .map_err(|_| format!("bad .fill repeat: {}", parts[0].trim()))?;
            let size = if parts.len() > 1 {
                parse_int_literal(parts[1].trim())
                    .map_err(|_| format!("bad .fill size: {}", parts[1].trim()))?
            } else {
                1
            };
            let value: u64 = if parts.len() > 2 {
                parse_int_literal(parts[2].trim())
                    .map_err(|_| format!("bad .fill value: {}", parts[2].trim()))?
                    as u64
            } else {
                0
            };
            if repeat <= 0 || size <= 0 {
                return Ok(AsmStatement::Directive(AsmDirective::Zero(0, 0)));
            }
            let elem = size.min(8) as u64;
            // GAS: overflow -> zero bytes, no diagnostic.
            let total_bytes = (repeat as u64).checked_mul(elem).unwrap_or_default();
            // No byte cap here: nothing is materialised at parse time (zero
            // runs become `Zero`, patterns a `Fill` descriptor), and the
            // writer decides against the live section -- the PROGBITS
            // ceiling and budget apply there, while a NOBITS run is a
            // size (GAS 2.47: `.fill 1073741824,1,0` in `.bss` is a
            // 1 GiB section). A parse-time cap refused exactly the
            // NOBITS case the writer was built to accept.
            let total_bytes = usize::try_from(total_bytes)
                .map_err(|_| format!(".fill: {total_bytes} bytes exceed the addressable size"))?;
            if value == 0 {
                AsmDirective::Zero(total_bytes, 0)
            } else {
                // No parse-time Vec: see the `Fill` variant's doc — the
                // writer charges the budget and applies the NOBITS law
                // against the live section.
                AsmDirective::Fill {
                    total: total_bytes,
                    elem: elem as usize,
                    value,
                }
            }
        }
        ".asciz" | ".string" => {
            let s = elf::parse_string_literal(args)?;
            let mut bytes = s;
            bytes.push(0); // null terminator
            AsmDirective::Asciz(bytes)
        }
        ".ascii" => {
            let s = elf::parse_string_literal(args)?;
            AsmDirective::Ascii(s)
        }
        ".comm" => match elf::parse_comm_args(".comm", args, true)? {
            Some((sym, size, align)) => AsmDirective::Comm(sym, size, align),
            None => AsmDirective::Ignored,
        },
        ".lcomm" => match elf::parse_comm_args(".lcomm", args, false)? {
            Some((sym, size, _)) => AsmDirective::Lcomm(sym, size),
            None => AsmDirective::Ignored,
        },
        ".local" => AsmDirective::Local(args.trim().to_string()),
        ".base64" => AsmDirective::Ascii(elf::parse_base64_args(args)?),
        ".set" | ".equ" => {
            let parts: Vec<&str> = args.splitn(2, ',').collect();
            if parts.len() == 2 {
                AsmDirective::Set(parts[0].trim().to_string(), parts[1].trim().to_string())
            } else {
                return Err(format!(
                    "malformed .set directive: expected 'name, value', got '{}'",
                    args
                ));
            }
        }
        ".symver" => {
            // .symver name, alias@@VERSION -> treat as alias for default version
            let parts: Vec<&str> = args.splitn(2, ',').collect();
            if parts.len() == 2 {
                let name = parts[0].trim();
                let ver_string = parts[1].trim();
                if let Some(at_pos) = ver_string.find('@') {
                    let alias = &ver_string[..at_pos];
                    if !alias.is_empty() {
                        AsmDirective::Set(alias.to_string(), name.to_string())
                    } else {
                        AsmDirective::Ignored
                    }
                } else {
                    AsmDirective::Ignored
                }
            } else {
                AsmDirective::Ignored
            }
        }
        // CFI directives
        name if name.starts_with(".cfi_") => {
            AsmDirective::Cfi(name.to_string(), args.trim().to_string())
        }
        ".pushsection" => {
            // .pushsection name,"flags",@type - same syntax as .section
            match parse_section_directive(args)? {
                AsmDirective::Section(dir) => AsmDirective::PushSection(dir),
                _ => AsmDirective::Ignored,
            }
        }
        ".popsection" => AsmDirective::PopSection,
        ".previous" => AsmDirective::Previous,
        ".subsection" => {
            let n: u64 = args.trim().parse().unwrap_or(0);
            AsmDirective::Subsection(n)
        }
        ".purgem" => {
            // .purgem name — remove a macro definition; ignore for now
            AsmDirective::Ignored
        }
        ".org" => {
            // `.org new-lc [, fill]` — advance the location counter to
            // `new-lc` bytes from the section start, padding with `fill`
            // (default 0).  Evaluated immediately, matching GAS: `new-lc`
            // may reference `.` and already-defined labels in the current
            // section, but forward references are rejected by the ELF
            // writer because padding shifts every following offset and
            // cannot be deferred the way `.word` expressions can.
            // Primary kernel user: the vectors table
            // (`.org .Lventry_start + 128` pads each entry to 128 bytes).
            let (expr, fill) = crate::backend::elf::parse_org_args(args)?;
            AsmDirective::Org { expr, fill }
        }
        ".incbin" => {
            let parts: Vec<&str> = args.splitn(3, ',').collect();
            let path = elf::parse_string_literal(parts[0].trim())
                .map_err(|e| format!(".incbin path: {}", e))?;
            let path = String::from_utf8(path)
                .map_err(|_| ".incbin: invalid UTF-8 in path".to_string())?;
            let skip = if parts.len() > 1 {
                parts[1].trim().parse::<u64>().unwrap_or(0)
            } else {
                0
            };
            let count = if parts.len() > 2 {
                Some(parts[2].trim().parse::<u64>().unwrap_or(0))
            } else {
                None
            };
            AsmDirective::Incbin { path, skip, count }
        }
        ".unreq" => {
            // .unreq name — remove register alias; ignore
            AsmDirective::Ignored
        }
        ".req" => {
            // .req register — register alias (standalone form); ignore
            AsmDirective::Ignored
        }
        ".float" | ".single" => {
            // .float val1, val2, ... — emit 32-bit IEEE floats
            let mut bytes = Vec::new();
            for part in args.split(',') {
                let val: f32 = part
                    .trim()
                    .parse()
                    .map_err(|_| format!("invalid .float value: {}", part.trim()))?;
                bytes.extend_from_slice(&val.to_le_bytes());
            }
            AsmDirective::RawBytes(bytes)
        }
        ".double" => {
            // .double val1, val2, ... — emit 64-bit IEEE floats
            let mut bytes = Vec::new();
            for part in args.split(',') {
                let val: f64 = part
                    .trim()
                    .parse()
                    .map_err(|_| format!("invalid .double value: {}", part.trim()))?;
                bytes.extend_from_slice(&val.to_le_bytes());
            }
            AsmDirective::RawBytes(bytes)
        }
        // Other directives we can safely ignore
        ".file" | ".loc" | ".ident" | ".addrsig" | ".addrsig_sym" | ".build_attributes"
        | ".eabi_attribute" | ".arch" | ".arch_extension" | ".cpu" | ".ltorg" | ".pool" => {
            AsmDirective::Ltorg
        }
        _ => {
            return Err(format!(
                "unsupported AArch64 assembler directive: {} {}",
                name, args
            ));
        }
    };

    Ok(AsmStatement::Directive(dir))
}

fn parse_instruction(line: &str) -> Result<AsmStatement, String> {
    // Split mnemonic from operands
    let (mnemonic, operands_str) = if let Some(space_pos) = line.find([' ', '\t']) {
        (&line[..space_pos], line[space_pos..].trim())
    } else {
        (line, "")
    };

    let mnemonic = mnemonic.to_lowercase();
    let operands = parse_operands(operands_str)?;

    Ok(AsmStatement::Instruction {
        mnemonic,
        operands,
        raw_operands: operands_str.to_string(),
    })
}

/// Parse an operand list separated by commas, handling brackets and nested expressions.
fn parse_operands(s: &str) -> Result<Vec<Operand>, String> {
    if s.is_empty() {
        return Ok(Vec::new());
    }

    let mut operands = Vec::new();
    let mut current = String::new();
    let mut bracket_depth = 0;
    let mut brace_depth = 0;

    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            '{' => {
                brace_depth += 1;
                current.push('{');
            }
            '}' => {
                brace_depth -= 1;
                current.push('}');
            }
            '[' => {
                bracket_depth += 1;
                current.push('[');
            }
            ']' => {
                bracket_depth -= 1;
                current.push(']');
                // Check for '!' (pre-index writeback)
                if i + 1 < chars.len() && chars[i + 1] == '!' {
                    current.push('!');
                    i += 1;
                }
            }
            ',' if bracket_depth == 0 && brace_depth == 0 => {
                let op = parse_single_operand(current.trim())?;
                operands.push(op);
                current.clear();
            }
            _ => {
                current.push(chars[i]);
            }
        }
        i += 1;
    }

    // Last operand
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        let op = parse_single_operand(&trimmed)?;
        operands.push(op);
    }

    // Handle memory operands with post-index: [base], #offset
    // This looks like two operands: Mem{base, 0} and Imm(offset)
    // We need to merge them into MemPostIndex
    let mut merged = Vec::new();
    let mut skip_next = false;
    for j in 0..operands.len() {
        if skip_next {
            skip_next = false;
            continue;
        }
        if j + 1 < operands.len() {
            if let (Operand::Mem { base, offset: 0 }, Operand::Imm(off)) =
                (&operands[j], &operands[j + 1])
            {
                merged.push(Operand::MemPostIndex {
                    base: base.clone(),
                    offset: *off,
                });
                skip_next = true;
                continue;
            }
        }
        merged.push(operands[j].clone());
    }

    Ok(merged)
}

fn parse_single_operand(s: &str) -> Result<Operand, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty operand".to_string());
    }

    // Register list: {v0.16b}, {v0.16b, v1.16b}, etc.
    // Register list with optional element index: {v0.s, v1.s}[0]
    if s.starts_with('{') {
        if s.ends_with('}') {
            return parse_register_list(s);
        }
        // Check for {regs}[index] form
        if let Some(close_brace) = s.find('}') {
            let rest = s[close_brace + 1..].trim();
            if rest.starts_with('[') && rest.ends_with(']') {
                let idx_str = &rest[1..rest.len() - 1];
                // Use expression evaluator to handle arithmetic like [1 - 1]
                let idx_result = idx_str.parse::<u32>().ok().or_else(|| {
                    asm_expr::parse_integer_expr(idx_str)
                        .ok()
                        .and_then(|v| u32::try_from(v).ok())
                });
                if let Some(idx) = idx_result {
                    let list_str = &s[..close_brace + 1];
                    let inner = &list_str[1..list_str.len() - 1];
                    let mut regs = Vec::new();
                    for part in inner.split(',') {
                        let part = part.trim();
                        if !part.is_empty() {
                            let op = parse_single_operand(part)?;
                            regs.push(op);
                        }
                    }
                    if regs.is_empty() {
                        return Err("empty register list".to_string());
                    }
                    return Ok(Operand::RegListIndexed { regs, index: idx });
                }
            }
        }
    }

    // Memory operand: [base, #offset]! (pre-index) or [base, #offset] or [base]
    if s.starts_with('[') {
        return parse_memory_operand(s);
    }

    // Immediate: #value
    if let Some(rest) = s.strip_prefix('#') {
        return parse_immediate(rest);
    }

    // :modifier:symbol
    if s.starts_with(':') {
        return parse_modifier(s);
    }

    // Shift: lsl, lsr, asr, ror, msl (MOVI/MVNI "masking shift left")
    let lower = s.to_lowercase();
    if lower.starts_with("lsl ")
        || lower.starts_with("lsr ")
        || lower.starts_with("asr ")
        || lower.starts_with("ror ")
        || lower.starts_with("msl ")
    {
        let kind = &lower[..3];
        let amount_str = s[4..].trim();
        let amount = if let Some(stripped) = amount_str.strip_prefix('#') {
            parse_int_literal(stripped)?
        } else {
            parse_int_literal(amount_str)?
        };
        return Ok(Operand::Shift {
            kind: kind.to_string(),
            amount: amount as u32,
        });
    }

    // Extend specifiers: sxtw, uxtw, sxtx, uxtx, sxth, uxth, sxtb, uxtb
    // May appear alone (sxtw) or with shift (sxtw #2)
    {
        let extend_prefixes = [
            "sxtw", "sxtx", "sxth", "sxtb", "uxtw", "uxtx", "uxth", "uxtb",
        ];
        for prefix in &extend_prefixes {
            if lower == *prefix {
                return Ok(Operand::Extend {
                    kind: prefix.to_string(),
                    amount: 0,
                });
            }
            if lower.starts_with(prefix) && lower.as_bytes().get(prefix.len()) == Some(&b' ') {
                let amount_str = s[prefix.len()..].trim();
                let amount = if let Some(stripped) = amount_str.strip_prefix('#') {
                    parse_int_literal(stripped)?
                } else {
                    parse_int_literal(amount_str)?
                };
                return Ok(Operand::Extend {
                    kind: prefix.to_string(),
                    amount: amount as u32,
                });
            }
        }
    }

    // Barrier options
    match lower.as_str() {
        "ish" | "ishld" | "ishst" | "sy" | "ld" | "st" | "osh" | "oshld" | "oshst" | "nsh"
        | "nshld" | "nshst" => {
            // Store original case: this name may be a C symbol colliding with an ARM keyword
            return Ok(Operand::Barrier(s.to_string()));
        }
        _ => {}
    }

    // Condition codes (for csel, csinc, etc.)
    match lower.as_str() {
        "eq" | "ne" | "cs" | "hs" | "cc" | "lo" | "mi" | "pl" | "vs" | "vc" | "hi" | "ls"
        | "ge" | "lt" | "gt" | "le" | "al" | "nv" => {
            // Store original case: this name may be a C symbol colliding with an ARM keyword
            return Ok(Operand::Cond(s.to_string()));
        }
        _ => {}
    }

    // NEON register with lane index: v0.d[1], v0.b[0], v0.s[2], etc.
    if let Some(dot_pos) = s.find('.') {
        let reg_part = &s[..dot_pos];
        let arr_part = &s[dot_pos + 1..];
        if is_register(reg_part) {
            if let Some(bracket_pos) = arr_part.find('[') {
                if arr_part.ends_with(']') {
                    let elem_size = arr_part[..bracket_pos].to_lowercase();
                    let idx_str = &arr_part[bracket_pos + 1..arr_part.len() - 1];
                    // Use expression evaluator to handle arithmetic like [1 - 1]
                    let idx_result = idx_str.parse::<u32>().ok().or_else(|| {
                        asm_expr::parse_integer_expr(idx_str)
                            .ok()
                            .and_then(|v| u32::try_from(v).ok())
                    });
                    if let Some(idx) = idx_result {
                        // `v0.16b[3]` and `v0.b[3]` are the same operand to
                        // GNU as: the arrangement spelling names the *element*
                        // type, and its index bound is the element's, not the
                        // arrangement's (`ins v0.8b[15],v1.b[15]` assembles,
                        // `ins v0.4h[8],v1.h[0]` does not).  Measured on GAS
                        // 2.47: exactly the eight canonical arrangements are
                        // accepted here and all of them encode the same word
                        // as the element spelling.
                        let elem = match elem_size.as_str() {
                            "b" | "h" | "s" | "d" => elem_size.clone(),
                            "8b" | "16b" => "b".to_string(),
                            "4h" | "8h" => "h".to_string(),
                            "2s" | "4s" => "s".to_string(),
                            "1d" | "2d" => "d".to_string(),
                            _ => return Ok(Operand::Symbol(s.to_string())),
                        };
                        return Ok(Operand::RegLane {
                            reg: reg_part.to_string(),
                            elem_size: elem,
                            index: idx,
                        });
                    }
                }
            }
        }
    }

    // NEON register with arrangement: v0.8b, v0.16b, v0.4s, v0.2d, etc.
    if let Some(dot_pos) = s.find('.') {
        let reg_part = &s[..dot_pos];
        let arr_part = &s[dot_pos + 1..];
        if is_register(reg_part) {
            let arr_lower = arr_part.to_lowercase();
            if matches!(
                arr_lower.as_str(),
                "8b" | "16b"
                    | "4h"
                    | "8h"
                    | "2s"
                    | "4s"
                    | "1d"
                    | "2d"
                    | "1q"
                    | "b"
                    | "h"
                    | "s"
                    | "d"
            ) {
                return Ok(Operand::RegArrangement {
                    reg: reg_part.to_string(),
                    arrangement: arr_lower,
                });
            }
        }
    }

    // Register
    if is_register(s) {
        return Ok(Operand::Reg(s.to_string()));
    }

    // Bare integer (without # prefix) - some inline asm constraints emit these
    // e.g., "eor w9, w10, 255" or "ccmp x10, x13, 0, eq"
    // Also handles negative like -1, -2, etc.
    if s.chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || c == '-')
    {
        if let Ok(val) = parse_int_literal(s) {
            return Ok(Operand::Imm(val));
        }
    }

    // Label/symbol reference (for branches, adrp, etc.)
    // Could be: .LBB42, func_name, symbol+offset
    if let Some(plus_pos) = s.find('+') {
        let sym = s[..plus_pos].trim();
        let off_str = s[plus_pos + 1..].trim();
        if !sym.is_empty() {
            if let Ok(off) = parse_int_literal(off_str) {
                return Ok(Operand::SymbolOffset(sym.to_string(), off));
            }
        }
    }
    if let Some(minus_pos) = s.find('-') {
        // Careful: don't confuse with label names containing '-' in label diff expressions
        if minus_pos > 0 {
            let sym = s[..minus_pos].trim();
            let off_str = &s[minus_pos..]; // includes the '-'
            if let Ok(off) = parse_int_literal(off_str) {
                return Ok(Operand::SymbolOffset(sym.to_string(), off));
            }
        }
    }

    // Try evaluating as a constant expression (handles parenthesized expressions like (0 + ...))
    if s.starts_with('(') || s.starts_with('~') {
        if let Ok(val) = parse_int_literal(s) {
            return Ok(Operand::Imm(val));
        }
    }

    // Plain symbol/label
    Ok(Operand::Symbol(s.to_string()))
}

/// Parse a register list like {v0.16b} or {v0.16b, v1.16b, v2.16b, v3.16b}
/// Also handles range syntax: {v0.8b-v3.8b} which means {v0.8b, v1.8b, v2.8b, v3.8b}
fn parse_register_list(s: &str) -> Result<Operand, String> {
    let inner = &s[1..s.len() - 1].trim(); // strip { and }
    let mut regs = Vec::new();
    for part in inner.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        // Check for range syntax: v0.8b-v2.8b
        if let Some(dash_pos) = part.find('-') {
            let left = part[..dash_pos].trim();
            let right = part[dash_pos + 1..].trim();
            // Both must be register arrangements
            if let (Some(ldot), Some(rdot)) = (left.find('.'), right.find('.')) {
                let lreg = &left[..ldot];
                let larr = &left[ldot + 1..];
                let rreg = &right[..rdot];
                let rarr = &right[rdot + 1..];
                if is_register(lreg) && is_register(rreg) && larr.eq_ignore_ascii_case(rarr) {
                    let start = parse_reg_num_simple(lreg);
                    let end = parse_reg_num_simple(rreg);
                    if let (Some(s_num), Some(e_num)) = (start, end) {
                        let prefix = if lreg.starts_with('v') || lreg.starts_with('V') {
                            "v".to_string()
                        } else {
                            lreg.chars().next().unwrap().to_string()
                        };
                        let count = if e_num >= s_num {
                            e_num - s_num + 1
                        } else {
                            (32 - s_num) + e_num + 1
                        };
                        for i in 0..count {
                            let reg_num = (s_num + i) % 32;
                            regs.push(Operand::RegArrangement {
                                reg: format!("{}{}", prefix, reg_num),
                                arrangement: larr.to_lowercase(),
                            });
                        }
                        continue;
                    }
                }
            }
        }
        let op = parse_single_operand(part)?;
        regs.push(op);
    }
    if regs.is_empty() {
        return Err("empty register list".to_string());
    }
    Ok(Operand::RegList(regs))
}

fn parse_reg_num_simple(reg: &str) -> Option<u32> {
    let s = reg.trim();
    if s.len() < 2 {
        return None;
    }
    let first = s.chars().next()?;
    if matches!(
        first,
        'v' | 'V'
            | 'x'
            | 'X'
            | 'w'
            | 'W'
            | 'q'
            | 'Q'
            | 'd'
            | 'D'
            | 's'
            | 'S'
            | 'h'
            | 'H'
            | 'b'
            | 'B'
    ) {
        s[1..].parse::<u32>().ok()
    } else {
        None
    }
}

fn parse_memory_operand(s: &str) -> Result<Operand, String> {
    let has_writeback = s.ends_with('!');
    let inner = if has_writeback {
        &s[1..s.len() - 2] // strip [ and ]!
    } else {
        // Find the matching ]
        let end = s.find(']').ok_or("missing ] in memory operand")?;
        &s[1..end]
    };

    // Split on comma
    let parts: Vec<&str> = inner.splitn(2, ',').collect();
    let base = parts[0].trim().to_string();

    if parts.len() == 1 {
        // [base]
        if has_writeback {
            return Ok(Operand::MemPreIndex { base, offset: 0 });
        }
        return Ok(Operand::Mem { base, offset: 0 });
    }

    let second = parts[1].trim();
    // `#:lo12:sym` (and `# :lo12:sym`) is the same operand as `:lo12:sym`
    // (GAS 2.47: identical word and relocation); route it to the modifier
    // form below instead of the deferred-expression path, which no
    // load/store encoder accepts.
    let second = match second.strip_prefix('#') {
        Some(rest) if rest.trim_start().starts_with(':') => rest.trim_start(),
        _ => second,
    };

    // [base, #imm] or [base, imm] (bare immediate without # prefix)
    if let Some(imm_str) = second.strip_prefix('#') {
        match parse_int_literal(imm_str) {
            Ok(offset) => {
                if has_writeback {
                    return Ok(Operand::MemPreIndex { base, offset });
                }
                return Ok(Operand::Mem { base, offset });
            }
            Err(_) => {
                // Expression contains symbols/labels — defer resolution
                return Ok(Operand::MemExpr {
                    base,
                    expr: imm_str.to_string(),
                    writeback: has_writeback,
                });
            }
        }
    }

    // Handle bare immediate without # prefix (e.g., [sp, -16]! or [x0, 8])
    // Check if the second operand starts with a digit or minus sign followed by a digit
    if second.starts_with('-')
        || second.starts_with('+')
        || second.bytes().next().is_some_and(|b| b.is_ascii_digit())
    {
        if let Ok(offset) = parse_int_literal(second) {
            if has_writeback {
                return Ok(Operand::MemPreIndex { base, offset });
            }
            return Ok(Operand::Mem { base, offset });
        }
    }

    // [base, :lo12:symbol]
    if second.starts_with(':') {
        // No pre-index form carries a relocated offset (its imm9 field has
        // no low-12 relocation); dropping the `!` would assemble a load whose
        // base register is never updated. GAS rejects it too.
        if has_writeback {
            return Err(format!(
                "`[{base}, {second}]!`: a relocated offset has no writeback form"
            ));
        }
        // Parse the modifier embedded in memory operand
        // The ] is already stripped, so just parse the modifier
        let mod_op = parse_modifier(second)?;
        // Return a special memory operand - we'll handle this in the encoder
        // For now, return it as a reg+symbol form
        match mod_op {
            Operand::Modifier { kind, symbol } => {
                return Ok(Operand::MemRegOffset {
                    base,
                    index: format!(":{}:{}", kind, symbol),
                    extend: None,
                    shift: None,
                });
            }
            Operand::ModifierOffset {
                kind,
                symbol,
                offset,
            } => {
                return Ok(Operand::MemRegOffset {
                    base,
                    index: format!(":{}:{}+{}", kind, symbol, offset),
                    extend: None,
                    shift: None,
                });
            }
            _ => {}
        }
    }

    // [base, Xm] or [base, Xm, extend #shift]
    // second may be "x0" or "x0, lsl #2" or "w0, sxtw" or "w0, sxtw #2"
    let sub_parts: Vec<&str> = second.splitn(2, ',').collect();
    let index_str = sub_parts[0].trim();
    if is_register(index_str) {
        // There is no writeback form of a register offset: the pre/post-index
        // addressing modes are immediate-only.  Dropping the writeback made
        // `ldr x0,[x0,x1]!` assemble as the plain register-offset load, i.e.
        // as an instruction whose base register is never updated.
        if has_writeback {
            return Err(format!(
                "`[{base},{index_str}]!` is not an addressing mode: writeback exists \
                 for the immediate forms only (`[{base}, #imm]!`), and the register \
                 offset has no writeback form at all"
            ));
        }
        let (extend, shift) = if sub_parts.len() > 1 {
            parse_extend_shift(sub_parts[1].trim())?
        } else {
            (None, None)
        };
        return Ok(Operand::MemRegOffset {
            base,
            index: index_str.to_string(),
            extend,
            shift,
        });
    }

    // Fallback: treat as register offset
    if has_writeback {
        return Err(format!(
            "`[{base},{second}]!` is not an addressing mode; writeback exists for the \
             immediate form only"
        ));
    }
    Ok(Operand::MemRegOffset {
        base,
        index: second.to_string(),
        extend: None,
        shift: None,
    })
}

/// Parse an extend/shift specifier like "lsl #2", "sxtw", "sxtw #0", "uxtx #3"
fn parse_extend_shift(s: &str) -> Result<(Option<String>, Option<u8>), String> {
    let s = s.trim().to_lowercase();
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.is_empty() {
        return Ok((None, None));
    }
    let kind = parts[0];
    let shift = if parts.len() > 1 {
        let shift_str = parts[1].trim_start_matches('#');
        Some(
            shift_str
                .parse::<u8>()
                .map_err(|_| format!("memory operand: shift `{shift_str}` is not an integer"))?,
        )
    } else {
        None
    };
    match kind {
        "lsl" | "lsr" | "asr" | "ror" | "msl" | "sxtw" | "sxtx" | "sxth" | "sxtb" | "uxtw"
        | "uxtx" | "uxth" | "uxtb" => {
            if parts.len() > 2 {
                return Err(format!(
                    "memory operand: extra token `{}` after extend/shift",
                    parts[2]
                ));
            }
            Ok((Some(kind.to_string()), shift))
        }
        _ => Err(format!("memory operand: unknown extend/shift `{kind}`")),
    }
}

fn parse_modifier(s: &str) -> Result<Operand, String> {
    // :kind:symbol or :kind:symbol+offset
    let s = s.trim_start_matches(':');
    let colon_pos = s
        .find(':')
        .ok_or("malformed modifier, expected :kind:symbol")?;
    let kind = s[..colon_pos].to_string();
    let rest = &s[colon_pos + 1..];

    // Check for symbol+offset or symbol-offset
    if let Some(plus_pos) = rest.find('+') {
        let symbol = rest[..plus_pos].trim().to_string();
        let offset_str = rest[plus_pos + 1..].trim();
        if let Ok(offset) = parse_int_literal(offset_str) {
            return Ok(Operand::ModifierOffset {
                kind,
                symbol,
                offset,
            });
        }
    }
    if let Some(minus_pos) = rest.rfind('-') {
        if minus_pos > 0 {
            let symbol = rest[..minus_pos].trim().to_string();
            let offset_str = &rest[minus_pos..]; // includes the '-'
            if let Ok(offset) = parse_int_literal(offset_str) {
                return Ok(Operand::ModifierOffset {
                    kind,
                    symbol,
                    offset,
                });
            }
        }
    }

    Ok(Operand::Modifier {
        kind,
        symbol: rest.trim().to_string(),
    })
}

fn parse_immediate(s: &str) -> Result<Operand, String> {
    // Handle :modifier:symbol as immediate (e.g., #:lo12:symbol)
    if s.starts_with(':') {
        return parse_modifier(s);
    }

    match parse_int_literal(s) {
        Ok(val) => Ok(Operand::Imm(val)),
        Err(_) => {
            // Expression contains symbols/labels — store as raw expression for
            // deferred resolution (e.g., #(1b - .Lvector_start + 4))
            Ok(Operand::Expr(s.to_string()))
        }
    }
}

fn parse_int_literal(s: &str) -> Result<i64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty integer literal".to_string());
    }

    // Use the shared expression evaluator which handles parentheses,
    // operator precedence, bitwise ops, and arithmetic expressions.
    asm_expr::parse_integer_expr(s)
}

fn is_register(s: &str) -> bool {
    let s = s.to_lowercase();
    // General purpose: x0-x30, w0-w30
    if (s.starts_with('x') || s.starts_with('w')) && s.len() >= 2 {
        let num = &s[1..];
        if let Ok(n) = num.parse::<u32>() {
            return n <= 30;
        }
    }
    // Special registers
    matches!(s.as_str(),
        "sp" | "wsp" | "xzr" | "wzr" | "lr"
    )
    ||
    // FP/SIMD: d0-d31, s0-s31, q0-q31, v0-v31, h0-h31, b0-b31
    {
        if (s.starts_with('d') || s.starts_with('s') || s.starts_with('q')
            || s.starts_with('v') || s.starts_with('h') || s.starts_with('b'))
            && s.len() >= 2
        {
            let num = &s[1..];
            if let Ok(n) = num.parse::<u32>() {
                return n <= 31;
            }
        }
        false
    }
}

// ── Directive parsing helpers ──────────────────────────────────────────

/// Parse a `.section name,"flags",@type` directive.
fn parse_section_directive(args: &str) -> Result<AsmDirective, String> {
    let parts = split_section_args(args);
    let name = parts
        .first()
        .map(|s| s.trim().trim_matches('"').to_string())
        .unwrap_or_else(|| ".text".to_string());
    let flags = parts.get(1).map(|s| s.trim().trim_matches('"').to_string());
    let section_type = parts.get(2).map(|s| s.trim().to_string());
    Ok(AsmDirective::Section(SectionDirective {
        name,
        flags,
        section_type,
    }))
}

/// Split section directive args, respecting quoted strings.
fn split_section_args(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for c in s.chars() {
        if c == '"' {
            in_quotes = !in_quotes;
            current.push(c);
        } else if c == ',' && !in_quotes {
            parts.push(current.clone());
            current.clear();
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// Parse `.type name, %function` or `@object` etc.
/// Also accepts space-separated form: `.type name STT_NOTYPE`.
fn parse_type_directive(args: &str) -> Result<AsmDirective, String> {
    let (sym, kind_str) = if let Some(comma_pos) = args.find(',') {
        (args[..comma_pos].trim(), args[comma_pos + 1..].trim())
    } else {
        // Space-separated fallback: ".type sym STT_NOTYPE"
        let parts: Vec<&str> = args.split_whitespace().collect();
        if parts.len() >= 2 {
            (parts[0], parts[1])
        } else {
            (args.trim(), "")
        }
    };
    let kind = match kind_str {
        "%function" | "@function" | "STT_FUNC" => SymbolKind::Function,
        "%object" | "@object" | "STT_OBJECT" => SymbolKind::Object,
        "@tls_object" => SymbolKind::TlsObject,
        _ => SymbolKind::NoType,
    };
    Ok(AsmDirective::SymbolType(sym.to_string(), kind))
}

/// Parse `.size name, expr`.
fn parse_size_directive(args: &str) -> Result<AsmDirective, String> {
    let parts: Vec<&str> = args.splitn(2, ',').collect();
    if parts.len() != 2 {
        return Err(format!(
            "malformed .size directive: expected 'name, expr', got '{}'",
            args
        ));
    }
    let sym = parts[0].trim().to_string();
    let expr_str = parts[1].trim();
    if let Some(rest) = expr_str.strip_prefix(".-") {
        let label = rest.trim().to_string();
        Ok(AsmDirective::Size(sym, SizeExpr::CurrentMinusSymbol(label)))
    } else if let Ok(size) = expr_str.parse::<u64>() {
        Ok(AsmDirective::Size(sym, SizeExpr::Constant(size)))
    } else {
        // Size expressions we can't evaluate (e.g. complex expressions) are non-fatal;
        // the symbol size is not critical for code correctness in static linking
        Ok(AsmDirective::Ignored)
    }
}

/// Parse comma-separated data values that may be integers, symbols, or symbol expressions.
fn parse_data_values(s: &str) -> Result<Vec<DataValue>, String> {
    let mut vals = Vec::new();
    for part in asm_expr::split_comma_args(s) {
        let trimmed = asm_expr::trim_data_arg(part);
        if trimmed.is_empty() {
            continue;
        }
        vals.push(match parse_data_value(trimmed) {
            Ok(val) => DataValue::Integer(val),
            Err(_) => DataValue::Expr(trimmed.to_string()),
        });
    }
    Ok(vals)
}

/// Strip balanced outer parentheses from an expression, recursively.
/// e.g. "((foo) - .)" => "(foo) - ." => "foo - ." (inner parens on individual terms
/// are handled by callers).
fn strip_outer_parens(s: &str) -> &str {
    let s = s.trim();
    if !s.starts_with('(') || !s.ends_with(')') {
        return s;
    }
    // Check if the outer parens are actually matched (not "(a)-(b)")
    let inner = &s[1..s.len() - 1];
    let mut depth = 0i32;
    for ch in inner.chars() {
        match ch {
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    return s; // closing paren in middle means outer parens aren't a simple wrapper
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    if depth == 0 {
        strip_outer_parens(inner) // recursively strip more layers
    } else {
        s
    }
}

/// Strip parentheses from a symbol name: "(9997f)" => "9997f", "(__label)" => "__label"
fn strip_sym_parens(s: &str) -> &str {
    let s = s.trim();
    if s.starts_with('(') && s.ends_with(')') {
        let inner = &s[1..s.len() - 1];
        // Make sure there are no unbalanced parens inside
        let mut depth = 0i32;
        for ch in inner.chars() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    if depth == 0 {
                        return s;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        if depth == 0 {
            strip_sym_parens(inner)
        } else {
            s
        }
    } else {
        s
    }
}

/// Parse a data value (integer literal, possibly negative).
fn parse_data_value(s: &str) -> Result<i64, String> {
    let s = asm_expr::trim_data_arg(s);
    if s.is_empty() {
        return Ok(0);
    }
    asm_expr::parse_integer_expr(s)
}
