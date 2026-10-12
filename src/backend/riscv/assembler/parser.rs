//! RISC-V assembly parser.
//!
//! Parses the textual assembly format emitted by our RISC-V codegen into
//! structured `AsmStatement` values. The parser handles:
//! - Labels (global and local)
//! - Directives (.section, .globl, .type, .align, .byte, .long, .dword, etc.)
//!   with fully typed representation (no string re-parsing in ELF writer)
//! - RISC-V instructions (add, sub, ld, sd, beq, call, ret, etc.)
//! - CFI directives (passed through as-is for DWARF unwind info)

// Some parser helper functions and enum variants are defined for completeness
// and used only by the encoder or ELF writer, not the parser entry point itself.
#![allow(dead_code)]

use crate::backend::asm_expr;
use crate::backend::asm_preprocess::{self, CommentStyle};
use crate::backend::elf;

/// A parsed assembly operand.
#[derive(Debug, Clone)]
pub enum Operand {
    /// Register: x0-x31, zero, ra, sp, gp, tp, t0-t6, s0-s11, a0-a7,
    ///           f0-f31, ft0-ft11, fs0-fs11, fa0-fa7
    Reg(String),
    /// Immediate value: 42, -1, 0x1000
    Imm(i64),
    /// Symbol reference: function name, label, etc.
    Symbol(String),
    /// Symbol with addend: symbol+offset or symbol-offset
    SymbolOffset(String, i64),
    /// Memory operand: offset(base) e.g., 8(sp) or -16(s0)
    Mem { base: String, offset: i64 },
    /// Memory operand with symbol: %lo(symbol)(base) or similar
    MemSymbol {
        base: String,
        symbol: String,
        modifier: String,
    },
    /// Label reference for branches
    Label(String),
    /// Fence operand: iorw etc.
    FenceArg(String),
    /// CSR register name or number
    Csr(String),
    /// Rounding mode: rne, rtz, rdn, rup, rmm, dyn
    RoundingMode(String),
}

/// A data value in a .byte/.short/.long/.quad directive.
/// Can be a literal integer, a symbol reference (with optional addend),
/// or a symbol difference expression (A - B, with optional addend on A).
#[derive(Debug, Clone)]
pub enum DataValue {
    /// A literal integer value.
    Integer(i64),
    /// A symbol reference, possibly with an addend: `sym` or `sym+4` or `sym-8`.
    Symbol { name: String, addend: i64 },
    /// A symbol difference: `sym_a - sym_b`, possibly with addend on sym_a.
    SymbolDiff {
        sym_a: String,
        sym_b: String,
        addend: i64,
    },
    /// A raw expression string that needs alias resolution at emit time.
    Expression(String),
}

/// Symbol type as parsed from `.type sym, @function` etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolType {
    Function,
    Object,
    TlsObject,
    NoType,
}

/// Symbol visibility as parsed from `.hidden`, `.protected`, `.internal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Hidden,
    Protected,
    Internal,
}

/// A size expression from `.size sym, expr`.
#[derive(Debug, Clone)]
pub enum SizeExpr {
    /// `.- label` — current position minus label
    CurrentMinus(String),
    /// A literal size value.
    Absolute(u64),
}

/// Section type from `.section` directive.
#[derive(Debug, Clone)]
pub struct SectionInfo {
    pub name: String,
    pub flags: String,
    pub sec_type: String,
    /// True when the directive explicitly included a flags field (even if empty).
    pub flags_explicit: bool,
}

/// A typed assembly directive. All argument parsing happens in the parser,
/// so the ELF writer only needs to pattern-match on these variants.
#[derive(Debug, Clone)]
pub enum Directive {
    /// `.section name, "flags", @type`
    Section(SectionInfo),
    /// `.text`
    Text,
    /// `.data`
    Data,
    /// `.bss`
    Bss,
    /// `.rodata`
    Rodata,
    /// `.globl sym` or `.global sym`
    Globl(String),
    /// `.weak sym`
    Weak(String),
    /// `.hidden sym`, `.protected sym`, `.internal sym`
    SymVisibility(String, Visibility),
    /// `.type sym, @function` etc.
    Type(String, SymbolType),
    /// `.size sym, expr`
    Size(String, SizeExpr),
    /// `.align N` / `.p2align N[, fill[, max]]` — power-of-2 alignment.
    ///
    /// `bytes` is the resolved alignment (1 << exponent) and `tail` the fill
    /// byte / padding cap. Both used to be dropped on the floor here: the same
    /// source assembled to different bytes than GAS (`.p2align 4, 0xff` filled
    /// with NOPs, `.p2align 4,,3` padded where GAS skips), which moves the
    /// address of every label after the directive.
    Align {
        bytes: u64,
        tail: crate::backend::asm_expr::AlignTail,
    },
    /// `.balign N[, fill[, max]]` — byte alignment: the same `s_align` handler
    /// as `.p2align`, so the third operand is a BYTE cap and is consulted
    /// wherever `.p2align`'s is (`.text` without a fill operand excepted:
    /// `riscv_frag_align_code` never sees it). Measured on as 2.47.20260726:
    /// `.balign 8,,4` pads 7 bytes in `.text` and skips in `.data`, and
    /// `.balign 8,,junk` is an ERROR on every target (GAS parses the max field
    /// with `get_absolute_expression` before any of the section logic runs), so
    /// a non-integer cap is refused here rather than swallowed.
    Balign {
        bytes: u64,
        fill: Option<u8>,
        max_pad: Option<u64>,
    },
    /// `.byte val, val, ...`
    Byte(Vec<DataValue>),
    /// `.short val, ...` / `.hword` / `.2byte` / `.half`
    Short(Vec<DataValue>),
    /// `.long val, ...` / `.4byte` / `.word`
    Long(Vec<DataValue>),
    /// `.quad val, ...` / `.8byte` / `.xword` / `.dword`
    Quad(Vec<DataValue>),
    /// `.zero N[, fill]` / `.space N[, fill]`
    Zero { size: usize, fill: u8 },
    /// `.fill` with a non-zero value — see the ARM `AsmDirective::Fill`
    /// doc: materialized at apply so the NOBITS raw-value law and the
    /// fill budget see the live section.
    Fill {
        total: usize,
        elem: usize,
        value: u64,
    },
    /// `.asciz "str"` / `.string "str"` — null-terminated string (raw bytes)
    Asciz(Vec<u8>),
    /// `.ascii "str"` — string without null terminator (raw bytes)
    Ascii(Vec<u8>),
    /// `.comm sym, size[, align]`
    Comm { sym: String, size: u64, align: u64 },
    /// `.local sym`
    Local(String),
    /// `.set sym, val` / `.equ sym, val`
    Set(String, String),
    /// `.option ...` — RISC-V specific
    ArchOption(String),
    /// `.attribute ...` — RISC-V attribute
    Attribute(String),
    /// CFI directives — silently ignored
    Cfi,
    /// Other ignorable directives: .file, .loc, .ident, etc.
    Ignored,
    /// `.pushsection name, "flags", @type` — push current section and switch
    PushSection(SectionInfo),
    /// `.popsection` — pop section stack
    PopSection,
    /// `.previous` — swap to previous section
    Previous,
    /// `.insn ...` — emit raw instruction encoding
    Insn(String),
    /// `.incbin "file"[, skip[, count]]` — include binary file contents
    Incbin {
        path: String,
        skip: u64,
        count: Option<u64>,
    },
    /// `.subsection N` — switch to numbered subsection
    Subsection(u64),
    /// Unknown directive — preserved for forward compatibility
    Unknown { name: String, args: String },
}

/// A parsed assembly statement.
#[derive(Debug, Clone)]
pub enum AsmStatement {
    /// A label definition: "name:"
    Label(String),
    /// A typed assembly directive.
    Directive(Directive),
    /// A RISC-V instruction with mnemonic and operands
    Instruction {
        mnemonic: String,
        operands: Vec<Operand>,
        /// The raw text of the operand string (for fallback/debugging)
        raw_operands: String,
    },
    /// An empty line or comment
    Empty,
}

/// Comment style for RISC-V GAS: `#` and `//`.
const COMMENT_STYLE: CommentStyle = CommentStyle::HashAndSlashSlash;

pub fn parse_asm(text: &str) -> Result<Vec<AsmStatement>, String> {
    // Pre-process: strip C-style /* ... */ comments (may span multiple lines)
    let text = asm_preprocess::strip_c_comments(text);

    // Expand .macro/.endm definitions and invocations
    let raw_lines: Vec<&str> = text.lines().collect();
    let macro_expanded = asm_preprocess::expand_macros(&raw_lines, &COMMENT_STYLE)?;
    let macro_refs: Vec<&str> = macro_expanded.iter().map(|s| s.as_str()).collect();

    // Expand .rept/.endr and .irp/.endr blocks
    let expanded_lines =
        asm_preprocess::expand_rept_blocks(&macro_refs, &COMMENT_STYLE, parse_int_literal)?;

    let mut statements = Vec::new();
    // Stack for .if/.else/.endif conditional assembly.
    // Each entry is true if the current block is active (emitting code).
    let mut if_stack: Vec<bool> = Vec::new();
    for (line_num, line) in expanded_lines.iter().enumerate() {
        let line = line.trim();

        // Skip empty lines
        if line.is_empty() {
            statements.push(AsmStatement::Empty);
            continue;
        }

        // Strip comments
        let line = strip_comment(line);
        let line = line.trim();
        if line.is_empty() {
            statements.push(AsmStatement::Empty);
            continue;
        }

        // Handle .if/.else/.elseif/.endif before anything else
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
        if lower.starts_with(".else") {
            if let Some(top) = if_stack.last_mut() {
                *top = !*top;
            }
            continue;
        }
        if lower.starts_with(".if ") || lower.starts_with(".if\t") {
            let cond_str = line[3..].trim();
            // Evaluate the condition: if we're already in a false block, push false
            let active = if if_stack.last().copied().unwrap_or(true) {
                asm_preprocess::eval_if_condition(cond_str)
            } else {
                false
            };
            if_stack.push(active);
            continue;
        }

        // If we're inside a false .if block, skip this line
        if !if_stack.last().copied().unwrap_or(true) {
            continue;
        }

        // Handle ';' as statement separator (GAS syntax).
        // Split the line on ';' and parse each part independently.
        let parts = asm_preprocess::split_on_semicolons(line);
        for part in parts {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            match parse_line(part) {
                Ok(stmts) => statements.extend(stmts),
                Err(e) => return Err(format!("Line {}: {}: '{}'", line_num + 1, e, part)),
            }
        }
    }
    Ok(statements)
}

/// Convenience wrapper: strip line comment using RISC-V comment style.
fn strip_comment(line: &str) -> &str {
    asm_preprocess::strip_comment(line, &COMMENT_STYLE)
}

fn parse_line(line: &str) -> Result<Vec<AsmStatement>, String> {
    // Check for label definition (name:)
    // Labels can be at the start of the line, possibly followed by an instruction
    if let Some(colon_pos) = line.find(':') {
        let potential_label = line[..colon_pos].trim();
        // Verify it looks like a valid label (no spaces before colon, alphanumeric + _ + .)
        if !potential_label.is_empty()
            && !potential_label.contains(' ')
            && !potential_label.contains('\t')
            && (!potential_label.starts_with('.')
                || potential_label.starts_with(".L")
                || potential_label.starts_with(".l"))
        {
            // Make sure this isn't a directive
            if !potential_label.starts_with('.')
                || potential_label.starts_with(".L")
                || potential_label.starts_with(".l")
            {
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

    let trimmed = line.trim();

    // Handle quoted instructions from macro expansion (e.g. "nop" or "j label")
    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        let inner = trimmed[1..trimmed.len() - 1].trim();
        if inner.is_empty() {
            return Ok(vec![AsmStatement::Empty]);
        }
        return parse_line(inner);
    }

    // Directive: starts with .
    if trimmed.starts_with('.') {
        return Ok(vec![parse_directive(trimmed)?]);
    }

    // Instruction
    Ok(vec![parse_instruction(trimmed)?])
}

/// Split a directive line into name and args, then dispatch to typed parsing.
fn parse_directive(line: &str) -> Result<AsmStatement, String> {
    let (name, args) = if let Some(space_pos) = line.find([' ', '\t']) {
        let name = &line[..space_pos];
        let args = line[space_pos..].trim();
        (name, args)
    } else {
        (line, "")
    };

    let directive = match name {
        ".section" => {
            let info = parse_section_args(args);
            Directive::Section(info)
        }
        ".text" => Directive::Text,
        ".data" => Directive::Data,
        ".bss" => Directive::Bss,
        ".rodata" => Directive::Rodata,

        ".globl" | ".global" => Directive::Globl(args.trim().to_string()),
        ".weak" => Directive::Weak(args.trim().to_string()),
        ".hidden" => Directive::SymVisibility(args.trim().to_string(), Visibility::Hidden),
        ".protected" => Directive::SymVisibility(args.trim().to_string(), Visibility::Protected),
        ".internal" => Directive::SymVisibility(args.trim().to_string(), Visibility::Internal),

        ".type" => parse_type_directive(args),

        ".size" => parse_size_directive(args),

        ".align" | ".p2align" => {
            // GAS grammar (read.c): exponent form; garbage is an error
            // (the old `unwrap_or(0)` silently assembled `.align junk`
            // as align-1) and out-of-range exponents clamp to 63 into
            // the writer's signed-shift bucket.
            let (first, rest) = split_align_args(args);
            let raw =
                parse_int_literal(first).map_err(|_| format!(".align: bad operand: {args}"))?;
            // Out-of-range exponents (negative or > 63) clamp to 63 —
            // NOT to 0: measured `.p2align -1` behaves like `.p2align 63`
            // (read.c's `address16 = ...` — same rule on every GAS
            // target), landing in the writer's signed-shift bucket.
            let exp = if (0..=63).contains(&raw) {
                raw as u64
            } else {
                63
            };
            let tail =
                crate::backend::asm_expr::parse_align_tail(rest, ".p2align", parse_int_literal)
                    .map_err(|e| e.to_message().replace(".p2align:", ".align:"))?;
            // The cap is kept, not dropped: whether it applies is decided by
            // the SECTION at emission time (`emit_align_with_reloc`), because
            // GAS's RISC-V hook `riscv_frag_align_code (int n)` takes only the
            // exponent -- so in a code section with no fill operand the cap is
            // parsed and ignored, while everywhere else the generic
            // `frag_align` compares it against the padding in BYTES.  Measured
            // on as 2.47.20260726: `.p2align 4,,2` pads 15 bytes in .text and
            // skips in .data; `.p2align 4,,15` pads and `,,14` skips in .data.
            // Dropping it here (as this backend did) padded inside `.data`,
            // where GAS leaves the offset alone, moving every later symbol;
            // honouring it here skipped `.text` padding GAS emits.  A negative
            // max is not an error: GAS warns ("ignoring out of range alignment
            // maximum") and treats it as unlimited, which is what `max <= 0`
            // already means downstream.
            Directive::Align {
                bytes: 1u64 << exp,
                tail,
            }
        }
        ".balign" => {
            // Byte-count form: bare is a no-op (GAS-accepted), anything
            // else must be a non-negative power of two or it is an error
            // ("alignment not a power of 2"), never a silent default.
            let (first, rest) = split_align_args(args);
            let raw: i64 = if first.is_empty() {
                1
            } else {
                parse_int_literal(first).map_err(|_| format!(".balign: bad operand: {args}"))?
            };
            if raw < 0 || (raw != 0 && !(raw as u64).is_power_of_two()) {
                return Err(format!(".balign: alignment {raw} is not a power of 2"));
            }
            // Every verdict the tail parser has is this target's verdict: the
            // fill byte, the byte-count cap, and the arity check. There used to
            // be a `BadMax => None` arm here, justified by "GAS never looks at
            // the cap" -- but `s_align` reads the max field with
            // `get_absolute_expression()` *before* the section and the fill
            // decide anything, so `.balign 8,,junk` is an assembler error
            // (measured on as 2.47.20260726 for both riscv64 and aarch64). A
            // swallowed rejection is a silently different object file.
            let tail =
                crate::backend::asm_expr::parse_align_tail(rest, ".balign", parse_int_literal)
                    .map_err(|e| e.to_message())?;
            Directive::Balign {
                bytes: if raw == 0 { 1 } else { raw as u64 },
                fill: tail.fill,
                max_pad: tail.max_pad,
            }
        }

        ".byte" => {
            let values = parse_data_values(args)?;
            Directive::Byte(values)
        }
        ".short" | ".hword" | ".2byte" | ".half" => {
            let values = parse_data_values(args)?;
            Directive::Short(values)
        }
        ".long" | ".4byte" | ".word" | ".int" => {
            let values = parse_data_values(args)?;
            Directive::Long(values)
        }
        ".quad" | ".8byte" | ".xword" | ".dword" => {
            let values = parse_data_values(args)?;
            Directive::Quad(values)
        }

        ".zero" | ".space" => {
            let parts: Vec<&str> = args.trim().split(',').collect();
            // GAS: a negative size emits nothing (`.zero -1` is a no-op);
            // the old `as usize` cast turned -1 into 2^64 bytes.
            let size_raw = parse_int_literal(parts[0].trim())
                .map_err(|_| format!("invalid .zero size: {}", args))?;
            let size: usize = if size_raw <= 0 { 0 } else { size_raw as usize };
            let fill: u8 = if parts.len() > 1 {
                parse_data_value_int(parts[1].trim())? as u8
            } else {
                0
            };
            Directive::Zero { size, fill }
        }
        ".fill" => {
            // .fill repeat, size, value
            let parts: Vec<&str> = args.splitn(3, ',').collect();
            let repeat = parse_int_literal(parts[0].trim())
                .map_err(|_| format!("bad .fill repeat: {}", parts[0].trim()))?;
            let size = if parts.len() > 1 {
                parse_int_literal(parts[1].trim())
                    .map_err(|_| format!("bad .fill size: {}", parts[1].trim()))?
            } else {
                1
            };
            let value = if parts.len() > 2 {
                parse_int_literal(parts[2].trim())
                    .map_err(|_| format!("bad .fill value: {}", parts[2].trim()))?
                    as u64
            } else {
                0
            };
            // GAS: non-positive repeat/size emit nothing; a wrapping
            // repeat*size emits zero bytes too (both measured on 2.47).
            // The old code cast-then-multiplied and hung/OOM'd on
            // `.fill -1,1,0xff`.
            if repeat <= 0 || size <= 0 {
                return Ok(AsmStatement::Directive(Directive::Zero {
                    size: 0,
                    fill: 0,
                }));
            }
            let repeat = repeat as u64;
            let size = size as u64;
            // GAS: overflow -> zero bytes, no diagnostic.
            let total_bytes = repeat.checked_mul(size.min(8)).unwrap_or_default();
            // Refuse before the value≠0 branch materializes its Vec
            // (same verdict as the writer's directive-fill cap).
            if total_bytes > crate::backend::elf::MAX_DIRECTIVE_FILL as u64 {
                return Err(format!(
                    ".fill: {total_bytes} bytes exceed the {}-byte directive limit",
                    crate::backend::elf::MAX_DIRECTIVE_FILL
                ));
            }
            let total_bytes = total_bytes as usize;
            if value == 0 {
                Directive::Zero {
                    size: total_bytes,
                    fill: 0,
                }
            } else {
                Directive::Fill {
                    total: total_bytes,
                    elem: size.min(8) as usize,
                    value,
                }
            }
        }

        ".asciz" | ".string" => {
            let s = elf::parse_string_literal(args)?;
            Directive::Asciz(s)
        }
        ".ascii" => {
            let s = elf::parse_string_literal(args)?;
            Directive::Ascii(s)
        }

        ".comm" => {
            let parts: Vec<&str> = args.split(',').collect();
            let sym = if !parts.is_empty() {
                parts[0].trim().to_string()
            } else {
                String::new()
            };
            let size: u64 = if parts.len() >= 2 {
                parts[1].trim().parse().unwrap_or(0)
            } else {
                0
            };
            let align: u64 = if parts.len() > 2 {
                parts[2].trim().parse().unwrap_or(1)
            } else {
                1
            };
            Directive::Comm { sym, size, align }
        }

        ".local" => Directive::Local(args.trim().to_string()),

        ".set" | ".equ" => {
            let parts: Vec<&str> = args.splitn(2, ',').collect();
            let sym = if !parts.is_empty() {
                parts[0].trim().to_string()
            } else {
                String::new()
            };
            let val = if parts.len() > 1 {
                parts[1].trim().to_string()
            } else {
                String::new()
            };
            Directive::Set(sym, val)
        }
        ".symver" => {
            // .symver name, alias@@VERSION -> treat as Set(alias, name) for default version
            let parts: Vec<&str> = args.splitn(2, ',').collect();
            if parts.len() == 2 {
                let name = parts[0].trim();
                let ver_string = parts[1].trim();
                if let Some(at_pos) = ver_string.find('@') {
                    let alias = &ver_string[..at_pos];
                    if !alias.is_empty() {
                        Directive::Set(alias.to_string(), name.to_string())
                    } else {
                        Directive::Ignored
                    }
                } else {
                    Directive::Ignored
                }
            } else {
                Directive::Ignored
            }
        }

        ".option" => Directive::ArchOption(args.to_string()),
        ".attribute" => Directive::Attribute(args.to_string()),
        ".insn" => Directive::Insn(args.to_string()),

        // CFI directives
        ".cfi_startproc"
        | ".cfi_endproc"
        | ".cfi_def_cfa_offset"
        | ".cfi_offset"
        | ".cfi_def_cfa_register"
        | ".cfi_restore"
        | ".cfi_remember_state"
        | ".cfi_restore_state"
        | ".cfi_adjust_cfa_offset"
        | ".cfi_def_cfa"
        | ".cfi_sections"
        | ".cfi_personality"
        | ".cfi_lsda"
        | ".cfi_rel_offset"
        | ".cfi_register"
        | ".cfi_return_column"
        | ".cfi_undefined"
        | ".cfi_same_value"
        | ".cfi_escape"
        | ".cfi_signal_frame" => Directive::Cfi,

        ".pushsection" => Directive::PushSection(parse_section_args(args)),
        ".popsection" => Directive::PopSection,
        ".previous" => Directive::Previous,

        ".org" => {
            // .org expressions like ". - (X) + (Y)" are used as size assertions
            // in kernel alternative macros. Silently ignore them.
            Directive::Ignored
        }

        ".incbin" => {
            let parts: Vec<&str> = args.splitn(3, ',').collect();
            let path = elf::parse_string_literal(parts[0].trim())?;
            let path = String::from_utf8(path).map_err(|_| "invalid .incbin path".to_string())?;
            let skip = if parts.len() > 1 {
                parse_int_literal(parts[1].trim()).unwrap_or(0) as u64
            } else {
                0
            };
            let count = if parts.len() > 2 {
                Some(parse_int_literal(parts[2].trim()).unwrap_or(0) as u64)
            } else {
                None
            };
            Directive::Incbin { path, skip, count }
        }

        ".subsection" => {
            let n = parse_int_literal(args.trim()).unwrap_or(0) as u64;
            Directive::Subsection(n)
        }

        // A mode switch with file-wide consequences is not an ignorable directive.
        // `.altmacro` changes how *every* macro argument in the file is expanded
        // (quoted arguments, `\()` separators, `!` repeat counts, `%`-prefixed
        // expressions), and the altmacro-only `local` directive is what makes labels
        // inside a macro unique per expansion -- which this backend does not
        // implement (`local lbl` reaches the writer as an unknown instruction).  So
        // ignoring the switch does not merely lose a feature: it assembles the rest
        // of the file under semantics the author explicitly did not ask for, and the
        // result is bytes that look fine.  `.noaltmacro` stays ignorable, because
        // non-altmacro mode *is* this backend's mode.
        ".altmacro" => {
            return Err(
                ".altmacro is not supported: altmacro-mode macro argument and `local` \
                 semantics differ, so the directive cannot be ignored; remove it and \
                 avoid `local`/quoted macro arguments"
                    .to_string(),
            );
        }

        // Other ignorable directives.  `.purgem` is listed here knowing it is not
        // fully honest -- forgetting a macro removal lets a use GAS would reject
        // assemble -- but it cannot change the bytes of a file GAS accepts, so it is
        // a leniency on invalid input rather than a miscompile.  Recorded as open.
        ".file" | ".loc" | ".ident" | ".addrsig" | ".addrsig_sym" | ".build_attributes"
        | ".eabi_attribute" | ".end" | ".noaltmacro" | ".purgem" => Directive::Ignored,

        _ => Directive::Unknown {
            name: name.to_string(),
            args: args.to_string(),
        },
    };

    Ok(AsmStatement::Directive(directive))
}

/// Parse `.section name, "flags", @type` arguments.
fn parse_section_args(args: &str) -> SectionInfo {
    let parts: Vec<&str> = args.split(',').collect();
    let name = parts[0].trim().trim_matches('"').to_string();
    let flags_explicit = parts.len() > 1;
    let flags = if flags_explicit {
        parts[1].trim().trim_matches('"').to_string()
    } else {
        String::new()
    };
    let sec_type = if parts.len() > 2 {
        parts[2].trim().to_string()
    } else {
        // Default type based on section name
        if name == ".bss" || name.starts_with(".bss.") || name.starts_with(".tbss") {
            "@nobits".to_string()
        } else {
            "@progbits".to_string()
        }
    };
    SectionInfo {
        name,
        flags,
        sec_type,
        flags_explicit,
    }
}

/// Parse `.type sym, @function` etc.
fn parse_type_directive(args: &str) -> Directive {
    let parts: Vec<&str> = args.splitn(2, ',').collect();
    if parts.len() == 2 {
        let sym = parts[0].trim().to_string();
        let ty = parts[1].trim();
        let st = match ty {
            "%function" | "@function" => SymbolType::Function,
            "%object" | "@object" => SymbolType::Object,
            "@tls_object" => SymbolType::TlsObject,
            _ => SymbolType::NoType,
        };
        Directive::Type(sym, st)
    } else {
        // Malformed .type directive — treat as no-type
        Directive::Type(args.trim().to_string(), SymbolType::NoType)
    }
}

/// Parse `.size sym, expr`.
fn parse_size_directive(args: &str) -> Directive {
    let parts: Vec<&str> = args.splitn(2, ',').collect();
    if parts.len() == 2 {
        let sym = parts[0].trim().to_string();
        let size_expr = parts[1].trim();
        if let Some(label) = size_expr.strip_prefix(".-") {
            Directive::Size(sym, SizeExpr::CurrentMinus(label.to_string()))
        } else if let Ok(size) = size_expr.parse::<u64>() {
            Directive::Size(sym, SizeExpr::Absolute(size))
        } else {
            // Can't parse — use 0
            Directive::Size(sym, SizeExpr::Absolute(0))
        }
    } else {
        // Malformed .size directive
        Directive::Size(args.trim().to_string(), SizeExpr::Absolute(0))
    }
}

/// Parse a comma-separated list of data values for .byte/.short/.long/.quad.
/// Each value can be an integer, a symbol reference, or a symbol difference.
fn parse_data_values(args: &str) -> Result<Vec<DataValue>, String> {
    let mut values = Vec::new();
    for part in args.split(',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        values.push(parse_single_data_value(trimmed)?);
    }
    Ok(values)
}

/// Check if a string looks like a GNU numeric label reference (e.g. "2f", "1b", "42f").
fn is_numeric_label_ref(s: &str) -> bool {
    asm_preprocess::is_numeric_label_ref(s)
}

/// Strip balanced outer parentheses from an expression.
/// E.g. "((1b) - .)" -> "(1b) - ." -> calls recursively until no outer parens.
fn strip_outer_parens(s: &str) -> &str {
    let s = s.trim();
    if !s.starts_with('(') || !s.ends_with(')') {
        return s;
    }
    // Check if the outer parens are balanced (the open paren at 0 matches the close at end)
    let inner = &s[1..s.len() - 1];
    let mut depth = 0i32;
    for ch in inner.bytes() {
        match ch {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth < 0 {
                    return s; // Close paren inside doesn't match, outer parens aren't a pair
                }
            }
            _ => {}
        }
    }
    if depth == 0 {
        strip_outer_parens(inner)
    } else {
        s
    }
}

/// Parse a single data value: integer, symbol, symbol+offset, or symbol_diff.
fn parse_single_data_value(s: &str) -> Result<DataValue, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(DataValue::Integer(0));
    }

    // Strip balanced outer parentheses (GCC wraps inline asm operands in parens,
    // e.g. "((1b) - .)" for numeric label diff expressions)
    let s = strip_outer_parens(s);

    // First check if this could be a symbol difference: A - B
    // The first char must be a symbol-start character (letter, _, .), a digit
    // (for numeric label references like "2f - 1f"), or '(' for parenthesized symbols.
    // However, if there are extra operators (/, *, <<, >>), this is a complex
    // expression and should go through the Expression path instead.
    let first = s.chars().next().unwrap();
    let is_sym_start = first.is_ascii_alphabetic() || first == '_' || first == '.';
    let could_be_numeric_ref = first.is_ascii_digit();
    let starts_with_paren = first == '(';
    let has_complex_ops =
        s.contains('/') || s.contains('*') || s.contains("<<") || s.contains(">>");
    if !has_complex_ops && (is_sym_start || could_be_numeric_ref || starts_with_paren) {
        // Try to find a symbol difference expression
        if let Some(minus_pos) = find_symbol_diff_minus(s) {
            let sym_a_raw = strip_outer_parens(s[..minus_pos].trim());
            let rest = s[minus_pos + 1..].trim();

            // rest might be "B" or "B + offset"
            let (sym_b_raw, addend) = if let Some(plus_pos) = rest.find('+') {
                let b = rest[..plus_pos].trim();
                let add_str = rest[plus_pos + 1..].trim();
                let add_val: i64 = add_str.parse().unwrap_or(0);
                (b, add_val)
            } else {
                (rest, 0i64)
            };
            let sym_b = strip_outer_parens(sym_b_raw);

            // Verify sym_b looks like a symbol (or a numeric label ref)
            if !sym_b.is_empty() {
                let b_first = sym_b.chars().next().unwrap();
                let b_is_sym = b_first.is_ascii_alphabetic() || b_first == '_' || b_first == '.';
                if b_is_sym || is_numeric_label_ref(sym_b) {
                    return Ok(DataValue::SymbolDiff {
                        sym_a: sym_a_raw.to_string(),
                        sym_b: sym_b.to_string(),
                        addend,
                    });
                }
            }
        }

        if is_sym_start {
            // Not a symbol diff — parse as symbol reference with optional addend
            let (sym, addend) = parse_symbol_addend(s);
            return Ok(DataValue::Symbol { name: sym, addend });
        } else if could_be_numeric_ref && is_numeric_label_ref(s) {
            // Standalone numeric label reference like "2f" or "1b"
            return Ok(DataValue::Symbol {
                name: s.to_string(),
                addend: 0,
            });
        } else if starts_with_paren {
            // Parenthesized single symbol, e.g. "(1b)" or "(.Lfoo)"
            let inner = strip_outer_parens(s);
            if inner != s {
                return parse_single_data_value(inner);
            }
        }
    }

    // Try to parse as constant+symbol (e.g., "0x80000000 + some_symbol")
    // This handles cases where the constant comes before the symbol in an additive expression.
    if could_be_numeric_ref || first == '(' {
        for (i, c) in s.char_indices().skip(1) {
            if c == '+' {
                let left = s[..i].trim();
                let right = s[i + 1..].trim();
                if let Ok(offset) = parse_data_value_int(left) {
                    // Check if right side looks like a symbol
                    let r_first = right.chars().next().unwrap_or('\0');
                    if r_first.is_ascii_alphabetic() || r_first == '_' || r_first == '.' {
                        let (sym, extra_addend) = parse_symbol_addend(right);
                        return Ok(DataValue::Symbol {
                            name: sym,
                            addend: offset + extra_addend,
                        });
                    }
                }
            }
        }
    }

    // Try to parse as integer; if it fails and contains symbol-like chars,
    // store as a raw expression for alias resolution at emit time.
    match parse_data_value_int(s) {
        Ok(v) => Ok(DataValue::Integer(v)),
        Err(_) => Ok(DataValue::Expression(s.to_string())),
    }
}

/// Parse a data value as a plain integer (used by .byte, .zero fill, etc).
fn parse_data_value_int(s: &str) -> Result<i64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(0);
    }
    asm_expr::parse_integer_expr(s)
}

/// Find the position of the '-' operator in a symbol difference expression.
fn find_symbol_diff_minus(expr: &str) -> Option<usize> {
    asm_preprocess::find_symbol_diff_minus(expr)
}

fn parse_symbol_addend(s: &str) -> (String, i64) {
    if let Some(plus_pos) = s.find('+') {
        let sym = s[..plus_pos].trim().to_string();
        let off: i64 = s[plus_pos + 1..].trim().parse().unwrap_or(0);
        (sym, off)
    } else if let Some(minus_pos) = s.find('-') {
        if minus_pos > 0 {
            let sym = s[..minus_pos].trim().to_string();
            let off_str = &s[minus_pos..];
            let off: i64 = off_str.parse().unwrap_or(0);
            (sym, off)
        } else {
            (s.to_string(), 0)
        }
    } else {
        (s.to_string(), 0)
    }
}

fn parse_instruction(line: &str) -> Result<AsmStatement, String> {
    // Split mnemonic from operands
    let (mnemonic, operands_str) = if let Some(space_pos) = line.find([' ', '\t']) {
        (&line[..space_pos], line[space_pos..].trim())
    } else {
        (line, "")
    };

    let mnemonic = mnemonic.to_lowercase();
    let operands = parse_operands(operands_str, &mnemonic)?;

    Ok(AsmStatement::Instruction {
        mnemonic,
        operands,
        raw_operands: operands_str.to_string(),
    })
}

/// Determine which operand positions for a given instruction mnemonic must be
/// parsed as symbols rather than registers. This prevents function/variable
/// names that happen to match register names (e.g., `f1`, `a0`, `ra`, `s1`)
/// from being misclassified.
fn symbol_operand_mask(mnemonic: &str) -> u8 {
    match mnemonic {
        // call <symbol> — operand 0 is always a symbol
        "call" | "tail" => 0b0000_0001,
        // la/lla rd, <symbol> — operand 1 is always a symbol
        "la" | "lla" => 0b0000_0010,
        // jump <label>, <temp_reg> — operand 0 is always a symbol
        "jump" => 0b0000_0001,
        _ => 0,
    }
}

/// Parse an operand list separated by commas.
/// `mnemonic`: the instruction mnemonic, used for context-sensitive parsing
/// (e.g., fence operands, symbol-position disambiguation).
fn parse_operands(s: &str, mnemonic: &str) -> Result<Vec<Operand>, String> {
    if s.is_empty() {
        return Ok(Vec::new());
    }

    // Only classify FenceArg operands when the instruction is actually "fence".
    // Otherwise, single-letter variable names like "i", "o", "r", "w" would be
    // misclassified as fence arguments (e.g., "lla t0, i" for a global named "i").
    let is_fence = mnemonic == "fence";
    let sym_mask = symbol_operand_mask(mnemonic);

    let mut operands = Vec::new();
    let mut current = String::new();
    let mut paren_depth = 0;
    let mut operand_idx: u8 = 0;

    for ch in s.chars() {
        match ch {
            '(' => {
                paren_depth += 1;
                current.push('(');
            }
            ')' => {
                paren_depth -= 1;
                current.push(')');
            }
            ',' if paren_depth == 0 => {
                let force_symbol = (sym_mask & (1 << operand_idx)) != 0;
                let op = parse_single_operand(current.trim(), is_fence, force_symbol)?;
                operands.push(op);
                current.clear();
                operand_idx = operand_idx.saturating_add(1);
            }
            _ => {
                current.push(ch);
            }
        }
    }

    // Last operand
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        let force_symbol = (sym_mask & (1 << operand_idx)) != 0;
        let op = parse_single_operand(&trimmed, is_fence, force_symbol)?;
        operands.push(op);
    }

    Ok(operands)
}

/// Parse a single operand.
/// `is_fence`: when true, classify subsets of "iorw" as FenceArg.
/// `force_symbol`: when true, this operand position is known to be a symbol
/// (e.g., the target of `call` or `tail`), so skip the register check. This
/// prevents function names like `f1`, `a0`, `ra` from being misclassified
/// as register operands.
fn parse_single_operand(s: &str, is_fence: bool, force_symbol: bool) -> Result<Operand, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty operand".to_string());
    }

    // Memory operand: offset(base) e.g., 8(sp), -16(s0), 0(a0)
    // Also handles: %lo(sym)(reg), %hi(sym)
    if !force_symbol {
        if let Some(result) = try_parse_memory_operand(s) {
            return Ok(result);
        }
    }

    // %hi(symbol) or %lo(symbol) - used as immediates in lui/addi
    if s.starts_with("%hi(")
        || s.starts_with("%lo(")
        || s.starts_with("%pcrel_hi(")
        || s.starts_with("%pcrel_lo(")
        || s.starts_with("%tprel_hi(")
        || s.starts_with("%tprel_lo(")
        || s.starts_with("%tprel_add(")
        || s.starts_with("%got_pcrel_hi(")
        || s.starts_with("%tls_ie_pcrel_hi(")
        || s.starts_with("%tls_gd_pcrel_hi(")
    {
        return Ok(Operand::Symbol(s.to_string()));
    }

    // Fence operands: iorw, ior, iow, etc.
    // Only classify as FenceArg when we're actually parsing a "fence" instruction,
    // to avoid misclassifying single-letter symbol names like "i", "o", "r", "w".
    if is_fence && is_fence_arg(s) {
        return Ok(Operand::FenceArg(s.to_string()));
    }

    // Rounding modes
    if !force_symbol && is_rounding_mode(s) {
        return Ok(Operand::RoundingMode(s.to_string()));
    }

    // Register — skip when this operand position is known to be a symbol
    if !force_symbol && is_register(s) {
        return Ok(Operand::Reg(s.to_string()));
    }

    // Try to parse as immediate (only when not forcing symbol)
    if !force_symbol {
        if let Ok(val) = parse_int_literal(s) {
            return Ok(Operand::Imm(val));
        }
    }

    // Symbol with offset: sym+offset or sym-offset
    if let Some(plus_pos) = s.find('+') {
        let sym = s[..plus_pos].trim();
        let off_str = s[plus_pos + 1..].trim();
        if let Ok(off) = parse_int_literal(off_str) {
            return Ok(Operand::SymbolOffset(sym.to_string(), off));
        }
    }
    if let Some(minus_pos) = s.rfind('-') {
        if minus_pos > 0 {
            let sym = s[..minus_pos].trim();
            let off_str = &s[minus_pos..];
            if let Ok(off) = parse_int_literal(off_str) {
                return Ok(Operand::SymbolOffset(sym.to_string(), off));
            }
        }
    }

    // Plain symbol/label
    Ok(Operand::Symbol(s.to_string()))
}

/// Try to parse a memory operand like `offset(reg)` or `%lo(sym)(reg)`.
fn try_parse_memory_operand(s: &str) -> Option<Operand> {
    // Look for the pattern: something(reg)
    // But be careful about nested parens like %lo(sym)(reg)

    let last_close = s.rfind(')')?;
    if last_close != s.len() - 1 {
        return None;
    }

    // Find the matching open paren for the last close paren
    let mut depth = 0;
    let mut last_open = None;
    for (i, c) in s.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' => {
                depth -= 1;
                if depth == 0 {
                    last_open = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }

    let open_pos = last_open?;
    let base = s[open_pos + 1..last_close].trim();
    let offset_part = s[..open_pos].trim();

    // Verify the base is a register
    if !is_register(base) {
        return None;
    }

    // Check if offset_part is a %lo/%hi modifier
    if offset_part.starts_with('%') {
        return Some(Operand::MemSymbol {
            base: base.to_string(),
            symbol: offset_part.to_string(),
            modifier: String::new(),
        });
    }

    // Try to parse offset as integer
    if offset_part.is_empty() {
        return Some(Operand::Mem {
            base: base.to_string(),
            offset: 0,
        });
    }

    if let Ok(offset) = parse_int_literal(offset_part) {
        return Some(Operand::Mem {
            base: base.to_string(),
            offset,
        });
    }

    // If offset_part is a symbol reference
    Some(Operand::MemSymbol {
        base: base.to_string(),
        symbol: offset_part.to_string(),
        modifier: String::new(),
    })
}

fn is_fence_arg(s: &str) -> bool {
    // Fence ordering: combinations of i, o, r, w
    let s = s.to_lowercase();
    if s.is_empty() || s.len() > 4 {
        return false;
    }
    s.chars().all(|c| matches!(c, 'i' | 'o' | 'r' | 'w'))
}

fn is_rounding_mode(s: &str) -> bool {
    matches!(
        s.to_lowercase().as_str(),
        "rne" | "rtz" | "rdn" | "rup" | "rmm" | "dyn"
    )
}

/// Check if a string is a valid RISC-V register name.
pub fn is_register(s: &str) -> bool {
    let s = s.to_lowercase();

    // ABI names
    matches!(s.as_str(),
        "zero" | "ra" | "sp" | "gp" | "tp"
        | "t0" | "t1" | "t2" | "t3" | "t4" | "t5" | "t6"
        | "s0" | "s1" | "s2" | "s3" | "s4" | "s5" | "s6"
        | "s7" | "s8" | "s9" | "s10" | "s11"
        | "a0" | "a1" | "a2" | "a3" | "a4" | "a5" | "a6" | "a7"
        | "fp"  // alias for s0
    ) ||
    // x0-x31
    (s.starts_with('x') && s.len() >= 2 && {
        if let Ok(n) = s[1..].parse::<u32>() { n <= 31 } else { false }
    }) ||
    // Floating-point ABI names
    matches!(s.as_str(),
        "ft0" | "ft1" | "ft2" | "ft3" | "ft4" | "ft5" | "ft6" | "ft7"
        | "ft8" | "ft9" | "ft10" | "ft11"
        | "fs0" | "fs1" | "fs2" | "fs3" | "fs4" | "fs5" | "fs6" | "fs7"
        | "fs8" | "fs9" | "fs10" | "fs11"
        | "fa0" | "fa1" | "fa2" | "fa3" | "fa4" | "fa5" | "fa6" | "fa7"
    ) ||
    // f0-f31
    (s.starts_with('f') && !s.starts_with("ft") && !s.starts_with("fs") && !s.starts_with("fa")
        && s.len() >= 2 && {
        if let Ok(n) = s[1..].parse::<u32>() { n <= 31 } else { false }
    }) ||
    // Vector registers: v0-v31
    (s.starts_with('v') && s.len() >= 2 && s.len() <= 3 && {
        if let Ok(n) = s[1..].parse::<u32>() { n <= 31 } else { false }
    })
}

/// Split an alignment directive's operands into the alignment itself and the
/// raw text after the first comma (`""` when there is none), so the shared
/// [`crate::backend::asm_expr::parse_align_tail`] can read `[, fill[, max]]`
/// with exactly the grammar every other GAS target applies.
fn split_align_args(args: &str) -> (&str, &str) {
    let args = args.trim();
    match args.find(',') {
        Some(i) => (&args[..i], args[i + 1..].trim()),
        None => (args, ""),
    }
}

pub fn parse_int_literal(s: &str) -> Result<i64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty integer literal".to_string());
    }

    // Use the shared expression evaluator which handles parentheses,
    // operator precedence, bitwise ops (|, &, ^, <<, >>), and arithmetic.
    asm_expr::parse_integer_expr(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stmts(src: &str) -> Result<Vec<AsmStatement>, String> {
        parse_asm(src)
    }

    fn mnemonic_list(src: &str) -> Vec<String> {
        stmts(src)
            .expect("parse")
            .into_iter()
            .filter_map(|s| match s {
                AsmStatement::Instruction { mnemonic, .. } => Some(mnemonic),
                _ => None,
            })
            .collect()
    }

    fn count_directives(src: &str) -> usize {
        stmts(src)
            .expect("parse")
            .into_iter()
            .filter(|s| matches!(s, AsmStatement::Directive(_)))
            .count()
    }

    /// The RISC-V front end reaches the shared expansion ceilings.
    ///
    /// `.macro`/`.endm`, `.rept`/`.endr` and `.irp`/`.irpc` are expanded here by
    /// `asm_preprocess::expand_macros` / `expand_rept_blocks` -- the same code
    /// that carries the growth ceilings and the nesting wall added for GAS
    /// parity.  Those limits are unit-tested in `asm_preprocess` and the x86
    /// front end has its own parity tests, but this target had neither, so a
    /// RISC-V path that expanded *around* the budget (a wrapper that pre-expanded
    /// and handed the expander a flat list, say) would have been invisible: the
    /// failure mode is a compiler that eats memory on a 41 KiB `.s` file rather
    /// than an error.  Asserted through `parse_asm`, i.e. through whatever wiring
    /// this target actually has, not through the shared module directly.
    #[test]
    fn gas247_expansion_ceilings_reach_the_riscv_entry_point() {
        // Legal expansions stay exact -- the guard is not achieved by refusing
        // everything, which is the easy way for a ceiling test to go green.
        assert_eq!(
            mnemonic_list(".macro two\n nop\n nop\n.endm\n two\n"),
            ["nop", "nop"]
        );
        assert_eq!(mnemonic_list(".rept 3\n nop\n.endr\n").len(), 3);
        // `.irp` amplifies items x body-lines out of a linear input.
        assert_eq!(count_directives(".irp v,1,2,3\n .byte \\v\n.endr\n"), 3);
        // The line-growth ceiling fires as an Err, in the shared wording.
        let err = stmts(".rept 5000000\n nop\n.endr\n").unwrap_err();
        assert!(err.contains("line limit"), "got: {err}");
        // GAS's `size_t` cast for a negative count, pinned at this entry point
        // too: this is the path that used to wrap to u64::MAX.
        let err = stmts(".rept -1\n nop\n.endr\n").unwrap_err();
        assert!(
            err.contains("excessive count 18446744073709551615 for REPT - ignored"),
            "got: {err}"
        );
        // The nesting wall: 101 chained frames assemble, the 102nd errors.
        let chain = |n: usize| {
            let mut s = String::new();
            for i in 0..n {
                let nxt = if i + 1 < n {
                    format!(" m{}", i + 1)
                } else {
                    " nop".to_string()
                };
                s.push_str(&format!(".macro m{i}\n{nxt}\n.endm\n"));
            }
            s.push_str(" m0\n");
            s
        };
        assert_eq!(mnemonic_list(&chain(101)).len(), 1);
        let err = stmts(&chain(102)).unwrap_err();
        assert!(err.contains("macros nested too deeply"), "got: {err}");
        // Direct recursion hits the same wall rather than the native stack.
        let err = stmts(".macro m\n m\n.endm\n m\n").unwrap_err();
        assert!(err.contains("macros nested too deeply"), "got: {err}");
    }

    /// Comment styles and `.if` evaluation go through the same shared helpers,
    /// so a target-level pin of each keeps a `CommentStyle` mix-up (which would
    /// leave `#`-commented `.rept` blocks unexpanded, and thus blow the budget on
    /// innocent input) from being caught only by a codegen diff.
    #[test]
    fn riscv_comment_and_conditional_paths_are_wired() {
        assert_eq!(mnemonic_list("# a comment\n nop // trailing\n").len(), 1);
        assert_eq!(mnemonic_list(".if 0\n nop\n.else\n nop\n.endif\n").len(), 1);
        assert_eq!(mnemonic_list(".if 1\n nop\n.endif\n").len(), 1);
        // A commented-out `.rept` must not be expanded at all.
        assert_eq!(mnemonic_list("# .rept 4\n nop\n# .endr\n").len(), 1);
    }

    /// The `.p2align` / `.balign` operands this target keeps, as
    /// `(alignment bytes, fill, max_pad)`.
    fn align_operands(src: &str) -> (u64, Option<u8>, Option<u64>) {
        stmts(src)
            .expect("parse")
            .into_iter()
            .find_map(|s| match s {
                AsmStatement::Directive(Directive::Align { bytes, tail }) => {
                    Some((bytes, tail.fill, tail.max_pad))
                }
                _ => None,
            })
            .expect("a .p2align directive")
    }

    fn balign_operands(src: &str) -> (u64, Option<u8>, Option<u64>) {
        stmts(src)
            .expect("parse")
            .into_iter()
            .find_map(|s| match s {
                AsmStatement::Directive(Directive::Balign {
                    bytes,
                    fill,
                    max_pad,
                }) => Some((bytes, fill, max_pad)),
                _ => None,
            })
            .expect("a .balign directive")
    }

    /// RISC-V's alignment tail rules, asserted at the entry point that applies
    /// them.  The shared tail parser is unit-tested for the grammar; what only
    /// this file can prove is the target-specific *use* of the parsed operands --
    /// that the cap is read and KEPT (the writer decides per section whether it
    /// applies, so dropping it here was the bug: a `.data` `.p2align 4,,2` pads in
    /// LCCC and not in GAS), that an explicit fill survives, that out-of-range
    /// exponents land in the signed-shift bucket, and that BOTH spellings refuse a
    /// non-integer cap.  A writer-level test alone cannot see the drop because the
    /// writer only ever receives an already-parsed directive, so the two halves
    /// are pinned where they live.
    /// `.altmacro` must be refused, not ignored.  The measured reason: with the
    /// directive accepted-but-ignored, an altmacro `local` label reaches the writer
    /// and fails as `unhandled instruction: local` -- so the file does not assemble
    /// either way, and the only question is *where* the user is told.  Refusing at
    /// the switch names the actual problem, and keeps a file that uses `\()` or
    /// quoted arguments (which silently *do* change meaning) from assembling into
    /// wrong bytes.
    #[test]
    fn altmacro_mode_is_refused_not_ignored() {
        let err = match parse_asm(".altmacro\n.byte 1\n") {
            Ok(_) => panic!(".altmacro must not be accepted as ignorable"),
            Err(e) => e,
        };
        assert!(
            err.contains(".altmacro is not supported"),
            "unhelpful refusal: {err}"
        );
        assert!(
            err.contains("local"),
            "the refusal has to name `local`, since that is what the user will hit \
             next: {err}"
        );
        // The mode-off form stays ignorable: non-altmacro is this backend's mode.
        assert!(parse_asm(".noaltmacro\n.byte 1\n").is_ok());
    }

    #[test]
    fn gas247_riscv_align_tail_verdicts_at_the_parser() {
        // Cap read and kept, in bytes, whatever its value: `.p2align E,,M`
        // pads identically for every M in {0,1,2,3,4,5,8,11,12,20,40,2048,2049}
        // *in a code section* (as 2.47) -- but that is the writer's section test,
        // not a licence to lose the operand here (as 2.47, `.data`: `,,15` pads,
        // `,,14` does not, so the value is load-bearing).
        assert_eq!(align_operands(".p2align 4,,8"), (16, None, Some(8)));
        assert_eq!(align_operands(".p2align 3,,2049"), (8, None, Some(2049)));
        // `max <= 0` means "no cap" in GAS (`if (max && ...)` in frag_align),
        // and the shared tail parser normalises it to `None` at parse time, so
        // an explicit `.p2align 4,,0` is indistinguishable from a bare
        // `.p2align 4` downstream.  That collapse is deliberate: a `Some(0)`
        // would have to be re-interpreted by every consumer, and a consumer
        // that instead read 0 as "pad nothing" would silently delete padding
        // GAS emits.  A NEGATIVE cap is the same story -- GAS warns
        // ("ignoring out of range alignment maximum") and clears it -- so both
        // land in the unlimited bucket rather than becoming an error.
        assert_eq!(align_operands(".p2align 4,,0"), (16, None, None));
        assert_eq!(align_operands(".p2align 4,,-1"), (16, None, None));
        assert_eq!(
            align_operands(".p2align 4,,0"),
            align_operands(".p2align 4"),
            "an explicit zero max and no max operand at all are one instruction"
        );
        // An explicit fill is the operand GAS does read, so it survives too.
        assert_eq!(align_operands(".p2align 4,0xff"), (16, Some(0xff), None));
        assert_eq!(
            align_operands(".p2align 4, 300 ,2"),
            (16, Some(0x2c), Some(2)),
            "the fill is the low byte of any integer, and whitespace around a slot is trimmed"
        );
        // An empty slot means "absent", not "zero fill"; two empty slots are
        // still two operands, and a third comma is a fourth operand -- an error.
        assert_eq!(align_operands(".p2align 4,,"), (16, None, None));
        assert_eq!(
            stmts(".p2align 4,,,").unwrap_err(),
            "Line 1: .align: too many operands: '.p2align 4,,,'"
        );
        // Out-of-range exponents clamp to the signed-shift bucket, never to 0.
        assert_eq!(align_operands(".p2align 64").0, 1u64 << 63);
        assert_eq!(align_operands(".p2align -1").0, 1u64 << 63);
        // `.balign` shares the whole tail grammar, cap slot included; a junk cap
        // is an error here as it is for `.p2align`, because `s_align` parses the
        // third field before any target hook can decline to look at it.
        assert_eq!(balign_operands(".balign 8,3"), (8, Some(3), None));
        assert_eq!(balign_operands(".balign 8,,4"), (8, None, Some(4)));
        assert_eq!(
            stmts(".balign 8,,junk").unwrap_err(),
            "Line 1: .balign: max-pad `junk` is not an integer: '.balign 8,,junk'",
            "a junk cap must be refused, and by the max wording -- not as a fill problem"
        );
        assert_eq!(
            stmts(".balign 8,,4,9").unwrap_err(),
            "Line 1: .balign: too many operands: '.balign 8,,4,9'"
        );
        assert_eq!(
            stmts(".balign 8,junk").unwrap_err(),
            "Line 1: .balign: fill `junk` is not an integer: '.balign 8,junk'"
        );
        // A bare `.balign` is the no-op GAS accepts; a non-power-of-two is a
        // rejection, not a silent rounding.
        assert_eq!(balign_operands(".balign").0, 1);
        assert_eq!(
            stmts(".balign 3").unwrap_err(),
            "Line 1: .balign: alignment 3 is not a power of 2: '.balign 3'"
        );
    }
}
