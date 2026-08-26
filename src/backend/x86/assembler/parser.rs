//! Parser for AT&T syntax x86-64 assembly as emitted by our codegen.
//!
//! Parses assembly text line-by-line into structured `AsmItem` values.
//! Handles directives, labels, and instructions with AT&T operand ordering
//! (source, destination).

use crate::backend::asm_expr;
use crate::backend::asm_preprocess::{self, CommentStyle};
use crate::backend::elf;
use std::fmt;

/// A parsed assembly item (one per line, roughly).
#[derive(Debug, Clone)]
pub enum AsmItem {
    /// Switch to a named section: `.section .text`, `.section .rodata`, etc.
    Section(SectionDirective),
    /// Global symbol: `.globl name`
    Global(String),
    /// Weak symbol: `.weak name`
    Weak(String),
    /// Hidden visibility: `.hidden name`
    Hidden(String),
    /// Protected visibility: `.protected name`
    Protected(String),
    /// Internal visibility: `.internal name`
    Internal(String),
    /// Symbol type: `.type name, @function` or `@object` or `@tls_object`
    SymbolType(String, SymbolKind),
    /// Symbol size: `.size name, expr`
    Size(String, SizeExpr),
    /// Label definition: `name:`
    Label(String),
    /// Alignment: `.align N`
    Align(u32),
    /// Emit bytes: `.byte val, val, ...` (can contain label expressions)
    Byte(Vec<DataValue>),
    /// Emit 16-bit values: `.short val, ...` (can be symbol references)
    Short(Vec<DataValue>),
    /// Emit 32-bit values: `.long val, ...` (can be symbol references)
    Long(Vec<DataValue>),
    /// Emit ULEB128-encoded values: `.uleb128 val, ...` (DWARF/.eh_frame)
    Uleb128(Vec<DataValue>),
    /// Emit SLEB128-encoded values: `.sleb128 val, ...` (DWARF/.eh_frame)
    Sleb128(Vec<DataValue>),
    /// Emit 64-bit values: `.quad val, ...` (can be symbol references)
    Quad(Vec<DataValue>),
    /// Emit zero bytes: `.zero N`
    Zero(u32),
    /// Deferred `.skip` with expression: evaluated after all labels are known.
    /// Used by kernel alternatives framework for label-arithmetic expressions
    /// like `.skip -(((6651f-6641f)-(662b-661b)) > 0) * ((6651f-6641f)-(662b-661b)), 0x90`.
    /// Fields: (expression_string, fill_byte)
    SkipExpr(String, u8),
    /// NUL-terminated string: `.asciz "str"`
    Asciz(Vec<u8>),
    /// String without NUL: `.ascii "str"`
    Ascii(Vec<u8>),
    /// Common symbol: `.comm name, size, align`
    Comm(String, u64, u32),
    /// Symbol alias: `.set alias, target`
    Set(String, String),
    /// CFI directive (ignored for code generation, kept for .eh_frame)
    #[allow(dead_code)]
    // Parsed by parser but skipped during encoding (DWARF .eh_frame not yet emitted)
    Cfi(CfiDirective),
    /// Debug file directive: `.file N "filename"`
    #[allow(dead_code)] // Parsed but skipped during encoding (debug info not yet emitted)
    File(u32, String),
    /// Debug location: `.loc filenum line column`
    #[allow(dead_code)] // Parsed but skipped during encoding (debug info not yet emitted)
    Loc(u32, u32, u32),
    /// x86-64 instruction
    Instruction(Instruction),
    /// `.option norelax` (RISC-V, ignored for x86)
    #[allow(dead_code)] // Parsed for RISC-V compatibility in shared parser; ignored on x86
    OptionDirective(String),
    /// Push current section and switch to a new one: `.pushsection name,"flags",@type`
    PushSection(SectionDirective),
    /// Pop section stack and restore previous section: `.popsection`
    PopSection,
    /// Swap current and previous sections: `.previous`
    Previous,
    /// `.org` directive: advance to position symbol + offset within the section.
    /// `.org new-lc, fill` -- symbol (may be empty or "." for the location
    /// counter), offset, and the fill byte (GAS defaults to 0).
    Org(String, i64, u8),
    /// `.incbin "file"[, skip[, count]]` — include binary file contents
    Incbin {
        path: String,
        skip: u64,
        count: Option<u64>,
    },
    /// Symbol version: `.symver name, name2@@VERSION` or `.symver name, name2@VERSION`,
    /// optionally followed by a visibility argument:
    /// `.symver name, name2@VERSION, {local|hidden|remove}` (GAS 2.35+).
    Symver(String, String, Option<SymverFlag>),
    /// Code mode switch: `.code16`, `.code32`, `.code64`
    /// The value is the bit width (16, 32, or 64).
    CodeMode(u8),
    /// Blank line or comment-only line
    Empty,
}

/// Visibility argument of a three-operand `.symver` directive (GAS 2.35+).
/// Semantics verified against binutils GAS on the plain-name symbol when the
/// versioned alias is created:
///   `local`  — the plain name is kept but demoted to STB_LOCAL,
///   `hidden` — the plain name stays global with STV_HIDDEN visibility,
///   `remove` — the plain name is removed from the symbol table entirely and
///              intra-object references rebind to the versioned symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymverFlag {
    Local,
    Hidden,
    Remove,
}

/// Section directive with optional flags and type.
#[derive(Debug, Clone)]
pub struct SectionDirective {
    pub name: String,
    pub flags: Option<String>,
    pub section_type: Option<String>,
    /// For sections with linked-to or group info (4th arg), e.g.
    /// `__patchable_function_entries,"awo",@progbits,.LPFE0`
    #[allow(dead_code)] // Parsed for completeness; will be used for SHF_LINK_ORDER sections
    pub extra: Option<String>,
    /// COMDAT group name from `.section name,"axG",@progbits,group_name,comdat`
    /// Set when flags contain 'G' and a 5th argument is "comdat".
    pub comdat_group: Option<String>,
}

/// Symbol kind from `.type` directive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Object,
    TlsObject,
    NoType,
    /// STT_GNU_IFUNC (10): indirect function resolved at load time by its
    /// resolver (glibc uses `.type foo, %gnu_indirect_function`).
    GnuIndirectFunction,
}

/// Size expression: either a constant or `.-name` (current position minus symbol).
#[derive(Debug, Clone)]
pub enum SizeExpr {
    Constant(u64),
    CurrentMinusSymbol(String),
    /// end_label - start_label (resolved by ELF writer after relaxation)
    SymbolDiff(String, String),
    /// Symbol reference (e.g., .L__sym_size_*) - resolved through .set aliases
    SymbolRef(String),
}

/// A data value that can be a constant, a symbol, or a symbol expression.
#[derive(Debug, Clone)]
pub enum DataValue {
    Integer(i64),
    Symbol(String),
    /// symbol + offset (e.g., `.quad GD_struct+128`)
    SymbolOffset(String, i64),
    /// symbol - symbol + addend (e.g., `.long .LBB3 - .Ljt_0`, `.short tr_gdt_end - tr_gdt - 1`)
    SymbolDiff(String, String),
    /// symbol - symbol with constant addend
    SymbolDiffAddend(String, String, i64),
    /// `(a - b) * scale + addend` — a symbol difference folded through an
    /// arithmetic expression (e.g. `.long (end-start)*2`). Same-section label
    /// differences are absolute, so this always resolves to a constant.
    /// `(a - b) * scale / div + addend`. div=1 for plain scaling; header.S
    /// PE metadata divides: `.long (section_table - .) / 8`.
    SymbolDiffScaled(String, String, i64, i64, i64),
}

/// CFI directives (call frame information).
#[derive(Debug, Clone)]
#[allow(dead_code)] // Variants constructed by parser; not yet consumed for .eh_frame emission
pub enum CfiDirective {
    StartProc,
    EndProc,
    DefCfaOffset(i32),
    DefCfaRegister(String),
    Offset(String, i32),
    Other(String),
}

/// An x86-64 instruction with mnemonic and operands.
#[derive(Debug, Clone)]
pub struct Instruction {
    /// Optional prefix (e.g., "lock", "rep")
    pub prefix: Option<String>,
    /// Instruction mnemonic (e.g., "movq", "addl", "ret")
    pub mnemonic: String,
    /// Operands in AT&T order (source first, destination last)
    pub operands: Vec<Operand>,
}

/// An instruction operand.
#[derive(Debug, Clone)]
pub enum Operand {
    /// Register: %rax, %eax, %al, %xmm0, %st(0), etc.
    Register(Register),
    /// Immediate: $42, $-1, $symbol
    Immediate(ImmediateValue),
    /// Memory reference: disp(%base, %index, scale) with optional segment
    Memory(MemoryOperand),
    /// Direct label/symbol reference (for jmp/call targets)
    Label(String),
    /// Indirect jump/call target: *%reg or *addr
    Indirect(Box<Operand>),
}

/// Register reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Register {
    pub name: String,
    /// EVEX opmask applied to this operand: `%zmm0{%k1}`.
    pub mask: Option<String>,
    /// EVEX zeroing semantics on the destination: `%zmm0{%k1}{z}`.
    pub zeroing: bool,
}

impl Register {
    /// Build a register from its bare name (no `%` sigil).
    ///
    /// The name is trimmed and any leftover `%` is stripped. GNU as tolerates
    /// whitespace between the sigil and the register name, and the kernel
    /// relies on it: `__ASM_REGPFX` splices `(% rip)` and `(% rsp)` into
    /// `arch/x86/include/asm/asm.h`'s `_ASM_RIP()`. Callers used to strip only
    /// the `%`, leaving a name of `" rip"` that matched no register and
    /// silently defaulted to `%rax` in the encoder -- turning
    /// `movl x86_pred_cmd(%rip), %eax` into `mov 0x0(%rax), %eax`, a wrong-code
    /// bug with no diagnostic. Normalizing at the single construction point
    /// covers every parse path at once.
    pub fn new(name: &str) -> Self {
        let n = name.trim();
        let n = n.strip_prefix('%').map(str::trim_start).unwrap_or(n);
        Register {
            name: n.to_string(),
            mask: None,
            zeroing: false,
        }
    }
}

impl fmt::Display for Register {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "%{}", self.name)
    }
}

/// Immediate value.
#[derive(Debug, Clone)]
pub enum ImmediateValue {
    Integer(i64),
    Symbol(String),
    /// Symbol with integer offset, e.g., init_top_pgt - 0xffffffff80000000
    SymbolPlusOffset(String, i64),
    /// Symbol with @modifier, e.g., symbol@GOTPCREL
    #[allow(dead_code)] // Constructed by parser; handled by encoder for GOT/TLS relocations
    SymbolMod(String, String),
    /// Symbol difference: sym_a - sym_b (e.g., $_DYNAMIC-1b)
    SymbolDiff(String, String),
}

/// Memory operand: optional_segment:disp(%base, %index, scale)
#[derive(Debug, Clone)]
pub struct MemoryOperand {
    pub segment: Option<String>,
    pub displacement: Displacement,
    pub base: Option<Register>,
    pub index: Option<Register>,
    pub scale: Option<u8>,
    /// EVEX opmask applied to this operand: `16(%rsp){%k1}`.
    pub mask: Option<String>,
    /// EVEX zeroing semantics on the destination.
    pub zeroing: bool,
}

/// Memory displacement.
#[derive(Debug, Clone)]
pub enum Displacement {
    None,
    Integer(i64),
    Symbol(String),
    /// Symbol with an addend offset: symbol+offset or symbol-offset (e.g., GD_struct+128(%rip))
    #[allow(dead_code)] // Constructed by elf.rs numeric label resolver; consumed by encoder
    SymbolAddend(String, i64),
    /// Symbol with relocation modifier: symbol@GOT, symbol@GOTPCREL, symbol@TPOFF, etc.
    SymbolMod(String, String),
    /// Symbol plus integer offset: symbol+N or symbol-N
    SymbolPlusOffset(String, i64),
    /// Symbol difference: `rva(gdt)(%ebp)` in the compressed-boot head_64.S
    /// expands to `((gdt) - startup_32)(%ebp)` — a label difference as a
    /// displacement. Same-section pairs fold to a constant after layout.
    SymbolDiff(String, String),
    /// Symbol difference with a constant addend on the minuend:
    /// `rva(pgtable + 0)` -> `((pgtable + 0) - startup_32)`.
    SymbolDiffAddend(String, String, i64),
}

/// Comment style for x86 AT&T GAS: `#` only.
const COMMENT_STYLE: CommentStyle = CommentStyle::Hash;

/// Expand .rept/.endr blocks by repeating contained lines.
/// Note: .irp/.endr blocks are handled separately by expand_gas_macros, so we must
/// track .irp depth to avoid swallowing .endr lines that belong to .irp blocks.
fn expand_rept_blocks(lines: &[&str]) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    let mut i = 0;
    let mut irp_depth = 0; // Track .irp nesting to avoid consuming their .endr
    while i < lines.len() {
        let trimmed = strip_comment(lines[i]).trim().to_string();
        if trimmed.starts_with(".rept ") || trimmed.starts_with(".rept\t") {
            if irp_depth > 0 {
                // Inside .irp block - pass through as-is
                result.push(lines[i].to_string());
                i += 1;
                continue;
            }
            let count_str = trimmed[".rept".len()..].trim();
            let count = parse_integer_expr(count_str)
                .map_err(|e| format!(".rept: bad count '{}': {}", count_str, e))?
                as usize;
            let mut depth = 1;
            let mut body = Vec::new();
            i += 1;
            while i < lines.len() {
                let inner = strip_comment(lines[i]).trim().to_string();
                if inner.starts_with(".rept ") || inner.starts_with(".rept\t") {
                    depth += 1;
                } else if inner == ".endr" {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                body.push(lines[i]);
                i += 1;
            }
            if depth != 0 {
                return Err(".rept without matching .endr".to_string());
            }
            let expanded_body = expand_rept_blocks(&body)?;
            for _ in 0..count {
                result.extend(expanded_body.iter().cloned());
            }
        } else if trimmed.starts_with(".irp ")
            || trimmed.starts_with(".irp\t")
            || trimmed.starts_with(".irpc ")
            || trimmed.starts_with(".irpc\t")
        {
            // .irpc must be tracked exactly like .irp here: this pre-pass
            // only expands .rept, and treating .irpc's .endr as a stray
            // DELETED it, so the later .irpc expansion ran off the end of
            // the block (efi-mixed.S page-table unroll emitted a stray
            // `ret` per iteration).
            irp_depth += 1;
            result.push(lines[i].to_string());
        } else if trimmed == ".endr" {
            if irp_depth > 0 {
                irp_depth -= 1;
                result.push(lines[i].to_string());
            }
            // else: stray .endr without .rept or .irp - skip
        } else {
            result.push(lines[i].to_string());
        }
        i += 1;
    }
    Ok(result)
}

/// Parse assembly text into a list of AsmItems.
pub fn parse_asm(text: &str) -> Result<Vec<AsmItem>, String> {
    let mut items = Vec::new();

    // Strip C-style /* */ comments (used in hand-written assembly like musl)
    let text = asm_preprocess::strip_c_comments(text);
    // Pre-split on ';' (GAS statement separator) so that .rept/.irp directives
    // embedded in semicolon-separated lines (e.g., from C preprocessor macro
    // expansion like PMDS) are properly detected by expand_rept_blocks.
    let raw_lines: Vec<&str> = text.lines().collect();
    let mut lines: Vec<String> = Vec::new();
    for line in &raw_lines {
        let parts = asm_preprocess::split_on_semicolons(line);
        if parts.len() > 1 {
            for part in parts {
                let part = part.trim();
                if !part.is_empty() {
                    lines.push(part.to_string());
                }
            }
        } else {
            lines.push(line.to_string());
        }
    }
    let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
    let expanded = expand_rept_blocks(&line_refs)?;
    let expanded = expand_gas_macros(&expanded)?;

    for (line_num, line) in expanded.iter().enumerate() {
        let line_num = line_num + 1; // 1-based

        // Strip comments (# to end of line, but not inside strings)
        let stripped = strip_comment(line);
        let trimmed = stripped.trim();

        if trimmed.is_empty() {
            items.push(AsmItem::Empty);
            continue;
        }

        // Handle ';' as instruction separator (GAS syntax)
        // Split the line on ';' and parse each part independently.
        let parts: Vec<&str> = asm_preprocess::split_on_semicolons(trimmed);
        for part in parts {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            match parse_line_items(part) {
                Ok(line_items) => items.extend(line_items),
                Err(e) => {
                    return Err(format!("line {}: {}: '{}'", line_num, e, part));
                }
            }
        }
    }

    Ok(items)
}

/// Strip trailing comment from a line using x86 comment style (`#`).
fn strip_comment(line: &str) -> &str {
    asm_preprocess::strip_comment(line, &COMMENT_STYLE)
}

/// Parse a single non-empty assembly line into one or more AsmItems.
///
/// A line may contain a label followed by an instruction on the same line
/// (e.g., `1: stmxcsr -8(%rsp)`), which produces two items.
fn parse_line_items(line: &str) -> Result<Vec<AsmItem>, String> {
    let mut items = Vec::new();

    // Check for label (may be followed by instruction on same line)
    let rest = if let Some((label, rest)) = try_parse_label(line) {
        items.push(AsmItem::Label(label));
        rest
    } else {
        line.trim()
    };

    // If there's nothing after the label, we're done
    if rest.is_empty() {
        return Ok(items);
    }

    // Check for GAS symbol assignment: symbol = expr (e.g., L4_PAGE_OFFSET = 42)
    // This is equivalent to .set symbol, expr
    if let Some(eq_pos) = rest.find('=') {
        let before = rest[..eq_pos].trim();
        // Make sure it looks like a symbol name (no spaces, not a register, not starting with special chars)
        if !before.is_empty()
            && !before.contains(' ')
            && !before.contains('\t')
            && !before.starts_with('$')
            && !before.starts_with('%')
            && (before.as_bytes()[0].is_ascii_alphabetic() || before.starts_with('_'))
        {
            let expr = rest[eq_pos + 1..].trim();
            items.push(AsmItem::Set(before.to_string(), expr.to_string()));
            return Ok(items);
        }
    }

    // Handle multiple labels on same line (e.g. "771: 999: .pushsection ...")
    let mut rest = rest;
    while let Some((label, remaining)) = try_parse_label(rest) {
        items.push(AsmItem::Label(label));
        rest = remaining;
    }

    // If there's nothing after the labels, we're done
    if rest.is_empty() {
        return Ok(items);
    }

    // Parse the remaining content as a directive or instruction
    if rest.starts_with('.') {
        items.push(parse_directive(rest)?);
    } else if is_prefixed_instruction(rest) {
        items.push(parse_prefixed_instruction(rest)?);
    } else {
        items.push(parse_instruction(rest, None)?);
    }

    Ok(items)
}

/// Try to parse a label definition. Returns the label name and any remaining
/// content after the label (which may be an instruction or directive on the same line).
///
/// For example:
///   "foo:"           -> Some(("foo", ""))
///   "1:  stmxcsr -8(%rsp)"  -> Some(("1", "stmxcsr -8(%rsp)"))
///   ".Lfoo: ret"     -> Some((".Lfoo", "ret"))
///   "mov %rax, %rbx" -> None (no label)
fn try_parse_label(line: &str) -> Option<(String, &str)> {
    let trimmed = line.trim();
    if let Some(colon_pos) = trimmed.find(':') {
        let candidate = trimmed[..colon_pos].trim();
        // Verify it's a valid label (no spaces, starts with letter/dot/digit)
        if !candidate.is_empty()
            && !candidate.contains(' ')
            && !candidate.contains('\t')
            && !candidate.contains(',')
            && !candidate.starts_with('$')
            && !candidate.starts_with('%')
        {
            let rest = trimmed[colon_pos + 1..].trim();
            return Some((candidate.to_string(), rest));
        }
    }
    None
}

/// Parse a directive line (starts with '.').
fn parse_directive(line: &str) -> Result<AsmItem, String> {
    let parts: Vec<&str> = line.splitn(2, |c: char| c.is_whitespace()).collect();
    let directive = parts[0];
    let args = parts.get(1).map(|s| s.trim()).unwrap_or("");

    match directive {
        ".section" => parse_section_directive(args),
        ".text" => Ok(AsmItem::Section(SectionDirective {
            name: ".text".to_string(),
            flags: None,
            section_type: None,
            extra: None,
            comdat_group: None,
        })),
        ".data" => Ok(AsmItem::Section(SectionDirective {
            name: ".data".to_string(),
            flags: None,
            section_type: None,
            extra: None,
            comdat_group: None,
        })),
        ".bss" => Ok(AsmItem::Section(SectionDirective {
            name: ".bss".to_string(),
            flags: None,
            section_type: None,
            extra: None,
            comdat_group: None,
        })),
        ".rodata" => Ok(AsmItem::Section(SectionDirective {
            name: ".rodata".to_string(),
            flags: None,
            section_type: None,
            extra: None,
            comdat_group: None,
        })),
        ".globl" | ".global" => Ok(AsmItem::Global(args.trim().to_string())),
        ".weak" => Ok(AsmItem::Weak(args.trim().to_string())),
        ".hidden" => Ok(AsmItem::Hidden(args.trim().to_string())),
        ".protected" => Ok(AsmItem::Protected(args.trim().to_string())),
        ".internal" => Ok(AsmItem::Internal(args.trim().to_string())),
        ".type" => parse_type_directive(args),
        ".size" => parse_size_directive(args),
        ".align" | ".p2align" | ".balign" => {
            let val_str = args.split(',').next().unwrap_or("1").trim();
            let val: u32 =
                parse_integer_expr(val_str).map_err(|_| format!("bad alignment: {}", args))? as u32;
            // .p2align is power-of-2, .align/.balign on x86 gas is byte count
            if directive == ".p2align" {
                Ok(AsmItem::Align(1 << val))
            } else {
                Ok(AsmItem::Align(val))
            }
        }
        ".org" => {
            // `.org new-lc, fill` -- the fill byte is optional and defaults to
            // zero, which is what the ELF writer emits.  It has to be split off
            // before the expression is parsed, or the whole `8, 0x90` string is
            // handed to the integer parser and rejected.
            let (args, fill) = match args.split_once(',') {
                Some((lc, f)) => (
                    lc.trim(),
                    parse_integer_expr(f.trim())
                        .map_err(|_| format!("bad .org fill: {}", f.trim()))?
                        as u8,
                ),
                None => (args, 0u8),
            };
            if let Ok(val) = parse_integer_expr(args) {
                Ok(AsmItem::Org(String::new(), val, fill))
            } else {
                let mut sym = args.to_string();
                let mut offset = 0i64;
                if let Some(plus_pos) = args.find('+') {
                    if plus_pos > 0 {
                        sym = args[..plus_pos].trim().to_string();
                        let offset_str = args[plus_pos + 1..].trim();
                        offset = parse_integer_expr(offset_str)
                            .map_err(|_| format!("bad .org offset: {}", offset_str))?;
                    }
                }
                Ok(AsmItem::Org(sym, offset, fill))
            }
        }
        ".byte" => {
            let vals = parse_data_values(args)?;
            Ok(AsmItem::Byte(vals))
        }
        ".short" | ".value" | ".2byte" | ".word" | ".hword" => {
            let vals = parse_data_values(args)?;
            Ok(AsmItem::Short(vals))
        }
        ".long" | ".4byte" | ".int" => {
            let vals = parse_data_values(args)?;
            Ok(AsmItem::Long(vals))
        }
        ".uleb128" => {
            let vals = parse_data_values(args)?;
            Ok(AsmItem::Uleb128(vals))
        }
        ".sleb128" => {
            let vals = parse_data_values(args)?;
            Ok(AsmItem::Sleb128(vals))
        }
        ".quad" | ".8byte" => {
            let vals = parse_data_values(args)?;
            Ok(AsmItem::Quad(vals))
        }
        // Floating-point data. These were previously UNRECOGNIZED and fell
        // through to the "ignore unknown directive" arm, silently emitting
        // NOTHING — every `.float`/`.double` constant pool assembled from
        // hand-written or third-party asm was dropped, shifting all following
        // data. Emit the IEEE-754 bit patterns, matching GAS.
        ".float" | ".single" => {
            let vals = parse_float_values(args, false)?;
            Ok(AsmItem::Byte(vals))
        }
        ".double" => {
            let vals = parse_float_values(args, true)?;
            Ok(AsmItem::Byte(vals))
        }
        // `.space` is an exact synonym of `.skip` in GAS (both take an optional
        // fill byte).  It was missing from this list, and because unrecognized
        // directives are silently ignored, `.space 8` emitted NOTHING -- every
        // following byte in the section landed at the wrong offset.
        ".zero" | ".skip" | ".space" => {
            let (expr_str, fill) = split_skip_args(args);
            if let Ok(val) = parse_integer_expr(expr_str) {
                if fill == 0 {
                    Ok(AsmItem::Zero(val as u32))
                } else {
                    Ok(AsmItem::SkipExpr(expr_str.to_string(), fill))
                }
            } else {
                // Expression contains labels or complex arithmetic - defer to ELF writer
                Ok(AsmItem::SkipExpr(expr_str.to_string(), fill))
            }
        }
        ".fill" => {
            // .fill repeat, size, value
            // Emits repeat copies of value, each size bytes wide (LE, max 8).
            let parts: Vec<&str> = args.splitn(3, ',').collect();
            let repeat_str = parts[0].trim();
            match parse_integer_expr(repeat_str) {
                Ok(repeat) => {
                    let repeat = repeat as u64;
                    let size = if parts.len() > 1 {
                        parse_integer_expr(parts[1].trim())
                            .map_err(|_| format!("bad .fill size: {}", parts[1].trim()))?
                            as u64
                    } else {
                        1
                    };
                    let value = if parts.len() > 2 {
                        parse_integer_expr(parts[2].trim())
                            .map_err(|_| format!("bad .fill value: {}", parts[2].trim()))?
                            as u64
                    } else {
                        0
                    };
                    let total_bytes = repeat * size.min(8);
                    if value == 0 {
                        Ok(AsmItem::Zero(total_bytes as u32))
                    } else {
                        let mut data = Vec::with_capacity(total_bytes as usize);
                        let value_bytes = value.to_le_bytes();
                        for _ in 0..repeat {
                            for j in 0..size.min(8) as usize {
                                data.push(value_bytes[j]);
                            }
                        }
                        Ok(AsmItem::Ascii(data))
                    }
                }
                Err(_) => {
                    // Repeat expression contains labels/symbols - defer to ELF writer.
                    // Parse size and value as constants (these are always simple integers).
                    let size = if parts.len() > 1 {
                        parse_integer_expr(parts[1].trim())
                            .map_err(|_| format!("bad .fill size: {}", parts[1].trim()))?
                            as u64
                    } else {
                        1
                    };
                    let value = if parts.len() > 2 {
                        parse_integer_expr(parts[2].trim())
                            .map_err(|_| format!("bad .fill value: {}", parts[2].trim()))?
                            as u8
                    } else {
                        0
                    };
                    if size == 1 {
                        // size=1: equivalent to .skip repeat, value
                        Ok(AsmItem::SkipExpr(repeat_str.to_string(), value))
                    } else if value == 0 {
                        // value=0: equivalent to .skip (repeat * size), 0
                        // Wrap expression: (repeat_expr) * size
                        let expr = format!("({}) * {}", repeat_str, size);
                        Ok(AsmItem::SkipExpr(expr, 0))
                    } else {
                        Err(format!("bad .fill repeat: {}: deferred .fill with size > 1 and non-zero value not supported", repeat_str))
                    }
                }
            }
        }
        ".asciz" | ".string" => {
            let s = elf::parse_string_literal(args)?;
            let mut bytes = s;
            bytes.push(0); // NUL terminator
            Ok(AsmItem::Asciz(bytes))
        }
        ".ascii" => {
            let s = elf::parse_string_literal(args)?;
            Ok(AsmItem::Ascii(s))
        }
        ".comm" => parse_comm_directive(args),
        ".set" => parse_set_directive(args),
        ".symver" => parse_symver_directive(args),
        ".cfi_startproc" => Ok(AsmItem::Cfi(CfiDirective::StartProc)),
        ".cfi_endproc" => Ok(AsmItem::Cfi(CfiDirective::EndProc)),
        ".cfi_def_cfa_offset" => {
            // Accept full constant expressions (e.g. "8*2", "N*8+N").
            let val =
                parse_integer_expr(args.trim()).map_err(|_| format!("bad cfi offset: {}", args))?;
            Ok(AsmItem::Cfi(CfiDirective::DefCfaOffset(val as i32)))
        }
        ".cfi_def_cfa_register" => {
            let reg = args.trim().trim_start_matches('%').to_string();
            Ok(AsmItem::Cfi(CfiDirective::DefCfaRegister(reg)))
        }
        ".cfi_offset" => {
            // .cfi_offset %rbp, -16  (offset may be a constant expression)
            let parts: Vec<&str> = args.splitn(2, ',').collect();
            if parts.len() != 2 {
                return Ok(AsmItem::Cfi(CfiDirective::Other(line.to_string())));
            }
            let reg = parts[0].trim().trim_start_matches('%').to_string();
            let off = parse_integer_expr(parts[1].trim())
                .map_err(|_| format!("bad cfi offset value: {}", args))?;
            Ok(AsmItem::Cfi(CfiDirective::Offset(reg, off as i32)))
        }
        ".file" => {
            // .file N "filename"
            let parts: Vec<&str> = args.splitn(2, |c: char| c.is_whitespace()).collect();
            if parts.len() == 2 {
                let num: u32 = parts[0].trim().parse().unwrap_or(0);
                let filename = parts[1].trim().trim_matches('"').to_string();
                Ok(AsmItem::File(num, filename))
            } else {
                Ok(AsmItem::Empty) // ignore malformed .file
            }
        }
        ".loc" => {
            // .loc filenum line column
            let nums: Vec<u32> = args
                .split_whitespace()
                .take(3)
                .filter_map(|s| s.parse().ok())
                .collect();
            if nums.len() >= 2 {
                Ok(AsmItem::Loc(
                    nums[0],
                    nums[1],
                    nums.get(2).copied().unwrap_or(0),
                ))
            } else {
                Ok(AsmItem::Empty)
            }
        }
        ".pushsection" => {
            // .pushsection name,"flags",@type - same syntax as .section
            parse_section_directive(args).map(|item| {
                if let AsmItem::Section(dir) = item {
                    AsmItem::PushSection(dir)
                } else {
                    AsmItem::Empty
                }
            })
        }
        ".popsection" => Ok(AsmItem::PopSection),
        ".previous" => Ok(AsmItem::Previous),
        // 16 = plain .code16 (call/jmp/ret default to 16-bit operands),
        // 17 = .code16gcc (GCC -m16 convention: unsuffixed call/ret
        // take 32-bit operands; everything else keeps 16-bit defaults).
        // Conflating the two mis-sized the C call/ret ABI against hand
        // asm using calll/retl (arch/x86/boot) and corrupted the stack:
        // every return popped 2 bytes off a 4-byte slot and the boot died
        // at the first function return (ret landed at 0x10000).
        ".code16gcc" => Ok(AsmItem::CodeMode(17)),
        ".code16" => Ok(AsmItem::CodeMode(16)),
        ".code32" => Ok(AsmItem::CodeMode(32)),
        ".code64" => Ok(AsmItem::CodeMode(64)),
        ".option" => Ok(AsmItem::OptionDirective(args.to_string())),
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
            Ok(AsmItem::Incbin { path, skip, count })
        }
        d if d.starts_with(".cfi_") => {
            // Any other .cfi_* directive (restore, adjust_cfa_offset, def_cfa,
            // register, undefined, escape, personality, lsda, sections,
            // signal_frame, remember/restore_state, val_offset, expression):
            // record tolerantly — CFI is parsed but not emitted by the builtin
            // assembler, so no argument shape may abort glibc/GCC-generated .S.
            Ok(AsmItem::Cfi(CfiDirective::Other(line.to_string())))
        }
        _ => {
            // Unknown directive - just ignore it with a warning
            // This handles .ident, .addrsig, etc. that GCC might emit
            Ok(AsmItem::Empty)
        }
    }
}

/// Parse `.section name,"flags",@type` directive.
fn parse_section_directive(args: &str) -> Result<AsmItem, String> {
    // Split by comma, but handle quoted strings
    let parts = split_section_args(args);

    let name = parts
        .first()
        .map(|s| s.trim().trim_matches('"').to_string())
        .unwrap_or_else(|| ".text".to_string());

    let flags = parts.get(1).map(|s| s.trim().trim_matches('"').to_string());
    let section_type = parts.get(2).map(|s| s.trim().to_string());
    let extra = parts.get(3).map(|s| s.trim().to_string());

    // If the flags contain 'G' (SHF_GROUP), the 4th arg is the group signature
    // name and the 5th arg should be "comdat".
    let comdat_group = if let Some(ref f) = flags {
        if f.contains('G') {
            if let Some(ref group_name) = extra {
                let fifth = parts.get(4).map(|s| s.trim().to_string());
                if fifth.as_deref() == Some("comdat") || fifth.is_none() {
                    // Accept both explicit "comdat" and implicit (GNU as default)
                    Some(group_name.clone())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    Ok(AsmItem::Section(SectionDirective {
        name,
        flags,
        section_type,
        extra,
        comdat_group,
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

/// Parse `.type name, @function` or `@object` or `@tls_object`.
/// Also handles Linux kernel format: `.type name STT_FUNC`.
fn parse_type_directive(args: &str) -> Result<AsmItem, String> {
    let (name, kind_str) = if let Some(comma_pos) = args.find(',') {
        // Standard GAS format: .type name, @function
        (args[..comma_pos].trim(), args[comma_pos + 1..].trim())
    } else {
        // Linux kernel format: .type name STT_FUNC (space-separated, no comma)
        let parts: Vec<&str> = args.splitn(2, char::is_whitespace).collect();
        if parts.len() != 2 {
            return Err(format!("bad .type directive: {}", args));
        }
        (parts[0].trim(), parts[1].trim())
    };
    let name = name.to_string();
    let kind = match kind_str {
        "@function" | "%function" | "STT_FUNC" => SymbolKind::Function,
        "@object" | "%object" | "STT_OBJECT" => SymbolKind::Object,
        "@tls_object" | "%tls_object" | "STT_TLS" => SymbolKind::TlsObject,
        "@notype" | "%notype" | "STT_NOTYPE" => SymbolKind::NoType,
        "@gnu_indirect_function"
        | "%gnu_indirect_function"
        | "STT_GNU_IFUNC"
        | "@gnu_indirect_function,"
        | "%gnu_indirect_function," => SymbolKind::GnuIndirectFunction,
        _ => return Err(format!("unknown symbol type: {}", kind_str)),
    };
    Ok(AsmItem::SymbolType(name, kind))
}

/// Parse `.size name, expr`.
fn parse_size_directive(args: &str) -> Result<AsmItem, String> {
    let parts: Vec<&str> = args.splitn(2, ',').collect();
    if parts.len() != 2 {
        return Err(format!("bad .size directive: {}", args));
    }
    let name = parts[0].trim().to_string();
    let expr_str = parts[1].trim();

    // Match ". - sym" or ".-sym" (current position minus symbol)
    let normalized = expr_str.replace(' ', "");
    if let Some(rest) = normalized.strip_prefix(".-") {
        let sym = rest.trim().to_string();
        Ok(AsmItem::Size(name, SizeExpr::CurrentMinusSymbol(sym)))
    } else if let Ok(val) = parse_integer_expr(expr_str) {
        Ok(AsmItem::Size(name, SizeExpr::Constant(val as u64)))
    } else if expr_str.starts_with('.')
        || expr_str
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
    {
        // Symbol reference (e.g., .L__sym_size_*) - resolve through .set aliases
        // Treat as a symbol that will be resolved during ELF writing
        Ok(AsmItem::Size(
            name,
            SizeExpr::SymbolRef(expr_str.to_string()),
        ))
    } else {
        Err(format!("bad size expr: {}", expr_str))
    }
}

/// Parse `.comm name, size, align`.
fn parse_comm_directive(args: &str) -> Result<AsmItem, String> {
    let parts: Vec<&str> = args.split(',').collect();
    if parts.len() < 2 {
        return Err(format!("bad .comm directive: {}", args));
    }
    let name = parts[0].trim().to_string();
    let size: u64 = parts[1]
        .trim()
        .parse()
        .map_err(|_| format!("bad .comm size: {}", args))?;
    let align: u32 = parts
        .get(2)
        .map(|s| s.trim().parse().unwrap_or(1))
        .unwrap_or(1);
    Ok(AsmItem::Comm(name, size, align))
}

/// Parse `.set alias, target`.
fn parse_set_directive(args: &str) -> Result<AsmItem, String> {
    let parts: Vec<&str> = args.splitn(2, ',').collect();
    if parts.len() != 2 {
        return Err(format!("bad .set directive: {}", args));
    }
    Ok(AsmItem::Set(
        parts[0].trim().to_string(),
        parts[1].trim().to_string(),
    ))
}

/// Parse `.symver name, name2@@VERSION` or `.symver name, name2@VERSION`,
/// with an optional third visibility argument `local`, `hidden` or `remove`
/// (GAS 2.35+; glibc's compat_symbol machinery generates the plain two-arg
/// form, the Linux kernel and newer glibc use `remove`).
///
/// An unrecognized third argument is a hard error, exactly like GAS
/// ("junk at end of line"): silently folding it into the version string
/// previously produced garbage symbol names like `foo@VERS_1, remove`.
fn parse_symver_directive(args: &str) -> Result<AsmItem, String> {
    let parts: Vec<&str> = args.split(',').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return Err(format!("bad .symver directive: {}", args));
    }
    let name = parts[0].trim();
    let ver = parts[1].trim();
    if name.is_empty() || ver.is_empty() {
        return Err(format!("bad .symver directive: {}", args));
    }
    let flag = match parts.get(2).map(|s| s.trim()) {
        None => None,
        Some("local") => Some(SymverFlag::Local),
        Some("hidden") => Some(SymverFlag::Hidden),
        Some("remove") => Some(SymverFlag::Remove),
        Some(other) => {
            return Err(format!(
                ".symver: invalid visibility argument '{}' (expected local, hidden or remove): {}",
                other, args
            ));
        }
    };
    Ok(AsmItem::Symver(name.to_string(), ver.to_string(), flag))
}

/// Does this line start with an instruction prefix followed by a mnemonic?
///
/// The separator is matched as "any whitespace", not a literal space:
/// hand-written kernel assembly overwhelmingly uses TABS (`rep\tstosl` in
/// arch/x86/boot/startup/efi-mixed.S). Matching only `"rep "` left the whole
/// line as a single mnemonic `rep` with `stosl` parsed as a label operand.
fn is_prefixed_instruction(rest: &str) -> bool {
    // Segment-override names are legal instruction prefixes in their own right:
    // the kernel writes `ds wrmsr` (arch/x86/include/asm/msr.h) to reserve a
    // byte that ALTERNATIVE can later patch into a different encoding.
    const PREFIXES: [&str; 13] = [
        "lock", "rep", "repz", "repe", "repnz", "repne", "notrack", "cs", "ds", "es", "ss", "fs",
        "gs",
    ];
    match rest.split_once(|c: char| c.is_whitespace()) {
        Some((head, tail)) => PREFIXES.contains(&head) && !tail.trim().is_empty(),
        None => false,
    }
}

/// Parse a prefix instruction like "lock cmpxchgq ..." or "rep movsb".
fn parse_prefixed_instruction(line: &str) -> Result<AsmItem, String> {
    let parts: Vec<&str> = line.splitn(2, |c: char| c.is_whitespace()).collect();
    let prefix = parts[0].to_string();
    let rest = parts.get(1).map(|s| s.trim()).unwrap_or("");
    parse_instruction(rest, Some(prefix))
}

/// Parse an instruction line.
fn parse_instruction(line: &str, prefix: Option<String>) -> Result<AsmItem, String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(AsmItem::Empty);
    }

    // Split mnemonic from operands
    let (mnemonic, operand_str) = split_mnemonic_operands(trimmed);

    if mnemonic.is_empty() {
        return Err(format!("empty mnemonic in: {}", line));
    }

    let operands = if operand_str.is_empty() {
        Vec::new()
    } else {
        parse_operands(operand_str)?
    };

    Ok(AsmItem::Instruction(Instruction {
        prefix,
        mnemonic: mnemonic.to_string(),
        operands,
    }))
}

/// Split a line into mnemonic and operand string.
fn split_mnemonic_operands(line: &str) -> (&str, &str) {
    // Strip GNU as encoding-prefix hints ({vex}, {vex3}, {vex2}, {evex}).
    // LCCC's encoders emit the shortest valid VEX encoding for 128/256-bit
    // instructions, which is semantically identical to the hinted form; the
    // prefixes are accepted so GCC/LLVM-generated assembly assembles cleanly.
    let trimmed = line.trim_start();
    let trimmed = if let Some(rest) = trimmed
        .strip_prefix("{vex} ")
        .or_else(|| trimmed.strip_prefix("{vex3} "))
        .or_else(|| trimmed.strip_prefix("{vex2} "))
        .or_else(|| trimmed.strip_prefix("{evex} "))
    {
        rest
    } else {
        trimmed
    };
    let trimmed = if (trimmed.starts_with("%v") || trimmed.starts_with("%x"))
        && trimmed.len() > 2
        && trimmed.as_bytes()[2].is_ascii_alphabetic()
    {
        // Keep the v: "%vstmxcsr" -> "vstmxcsr" (GNU as selects the VEX
        // encoding for the %v hint; LCCC's v* encoders match).
        &trimmed[1..]
    } else {
        trimmed
    };
    if let Some(pos) = trimmed.find(|c: char| c.is_whitespace()) {
        let mnemonic = &trimmed[..pos];
        let rest = trimmed[pos..].trim();
        (mnemonic, rest)
    } else {
        (trimmed, "")
    }
}

/// Parse comma-separated operands.
fn parse_operands(s: &str) -> Result<Vec<Operand>, String> {
    let parts = split_operands(s);
    let mut operands = Vec::new();
    for part in &parts {
        operands.push(parse_operand(part.trim())?);
    }
    Ok(operands)
}

/// Split operand string by commas, respecting parentheses.
fn split_operands(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut paren_depth = 0;

    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            // GAS character literal: 'x or 'x' — the next char is data, even
            // a comma or paren (`movb $',', %bl` in relocate_kernel_64.S
            // print_reg). Copy the literal through without interpreting it.
            current.push('\'');
            if i + 1 < chars.len() {
                if chars[i + 1] == '\\' && i + 2 < chars.len() {
                    current.push('\\');
                    current.push(chars[i + 2]);
                    i += 2;
                } else {
                    current.push(chars[i + 1]);
                    i += 1;
                }
                if i + 1 < chars.len() && chars[i + 1] == '\'' {
                    current.push('\'');
                    i += 1;
                }
            }
        } else if c == '(' {
            paren_depth += 1;
            current.push(c);
        } else if c == ')' {
            paren_depth -= 1;
            current.push(c);
        } else if c == ',' && paren_depth == 0 {
            parts.push(current.clone());
            current.clear();
        } else {
            current.push(c);
        }
        i += 1;
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// Parse a single operand.
fn parse_operand(s: &str) -> Result<Operand, String> {
    let s = s.trim();

    // Indirect: *%reg or *addr
    if let Some(rest) = s.strip_prefix('*') {
        let inner = parse_operand(rest)?;
        return Ok(Operand::Indirect(Box::new(inner)));
    }

    // Register: %rax, %st(0), etc.
    if s.starts_with('%') {
        return parse_register_operand(s);
    }

    // Immediate: $42, $symbol, $symbol@GOTPCREL
    if let Some(rest) = s.strip_prefix('$') {
        return parse_immediate_operand(rest);
    }

    // Memory or label reference
    // Could be: offset(%base), symbol(%rip), (%base,%idx,scale), or just a label
    if s.contains('(') || s.contains(':') {
        return parse_memory_operand(s);
    }

    // Check if this is a bare numeric address like `42` (used in `movl 42, %eax`).
    // In AT&T syntax, a bare number without `$` prefix is an absolute memory address.
    // Only treat it as a memory operand if it parses as a pure integer and doesn't
    // look like a numeric label reference (e.g., `1f`, `1b`).
    // A GAS numeric local label is a run of DECIMAL DIGITS followed by a single
    // `f` or `b` and nothing else (`1f`, `42b`).  Rejecting every operand that
    // merely ends in those letters also threw away legitimate hex constants:
    // `mov 0x7fffffff, %eax` was treated as a label reference and silently
    // assembled with a zero displacement.  Test the whole shape instead.
    let is_numeric_label = {
        let stem = &s[..s.len().saturating_sub(1)];
        matches!(s.bytes().last(), Some(b'f') | Some(b'b'))
            && !stem.is_empty()
            && stem.bytes().all(|c| c.is_ascii_digit())
    };
    if s.bytes()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || c == b'-')
        && !is_numeric_label
    {
        if let Ok(val) = crate::backend::asm_expr::parse_integer_expr(s) {
            return Ok(Operand::Memory(MemoryOperand {
                segment: None,
                displacement: Displacement::Integer(val),
                base: None,
                index: None,
                scale: None,
                mask: None,
                zeroing: false,
            }));
        }
    }

    // Plain label reference (for jmp/call targets)
    // Could be: .LBB42, funcname, funcname@PLT, 1f, 1b
    Ok(Operand::Label(s.to_string()))
}

/// Split an EVEX mask/zeroing suffix: `%zmm0{%k1}{z}` -> ("%zmm0", Some("k1"), true).
/// Accepts both `{%k1}` and `{k1}` (GNU as tolerates the bare form), and `{z}`.
fn parse_evex_mask_suffix(s: &str) -> (String, Option<String>, bool) {
    let mut name = s.to_string();
    let mut mask = None;
    let mut zeroing = false;
    loop {
        if let Some(pos) = name.find('{') {
            let close = name[pos..].find('}');
            let Some(close) = close else { break };
            let inner = name[pos + 1..pos + close].trim().to_string();
            let before = name[..pos].to_string();
            let after = name[pos + close + 1..].to_string();
            name = before + &after;
            if inner == "z" {
                zeroing = true;
            } else if let Some(rest) = inner.strip_prefix('%') {
                mask = Some(rest.trim().to_string());
            } else if !inner.is_empty() {
                mask = Some(inner.to_string());
            }
        } else {
            break;
        }
    }
    (name, mask, zeroing)
}

/// Parse a register operand like %rax, %st(0).
fn parse_register_operand(s: &str) -> Result<Operand, String> {
    // GNU as tolerates whitespace between '%' and the register name
    // (glibc emits `mov % r13, ...` from `% " R13_LP "` macro splicing).
    let (clean, mask, zeroing) = parse_evex_mask_suffix(s);
    let s = clean.as_str();
    let name = s[1..].trim_start(); // strip % and any following spaces

    // Handle %st(N)
    if name.starts_with("st(") && name.ends_with(')') {
        return Ok(Operand::Register(Register::new(name)));
    }

    // Handle segment:memory patterns like %fs:0
    if let Some(colon_pos) = name.find(':') {
        let seg = &name[..colon_pos];
        let rest = &s[1 + colon_pos + 1..]; // after the colon
        if is_segment_name(seg) {
            let mut mem = parse_memory_inner(rest)?;
            mem.segment = Some(seg.to_string());
            mem.mask = mask;
            mem.zeroing = zeroing;
            return Ok(Operand::Memory(mem));
        }
    }

    let mut reg = Register::new(name);
    reg.mask = mask;
    reg.zeroing = zeroing;
    Ok(Operand::Register(reg))
}

/// Parse an immediate operand (after the '$').
fn parse_immediate_operand(s: &str) -> Result<Operand, String> {
    let s = s.trim();

    // Strip outer parentheses used for grouping, e.g. $(init_top_pgt - 0xffffffff80000000)
    // In AT&T syntax, $(...) wraps the expression inside parens for grouping.
    if s.starts_with('(') && s.ends_with(')') {
        let inner = &s[1..s.len() - 1];
        // Verify balanced parens
        let mut depth = 0i32;
        let mut balanced = true;
        for c in inner.chars() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth < 0 {
                        balanced = false;
                        break;
                    }
                }
                _ => {}
            }
        }
        if balanced && depth == 0 {
            return parse_immediate_operand(inner);
        }
    }

    // Try integer
    if let Ok(val) = parse_integer_expr(s) {
        return Ok(Operand::Immediate(ImmediateValue::Integer(val)));
    }

    // Symbol with modifier: symbol@GOTPCREL, sym@VER@GOTPCREL, etc.
    if let Some((sym, modifier)) = split_sym_modifier(s) {
        return Ok(Operand::Immediate(ImmediateValue::SymbolMod(sym, modifier)));
    }

    // Check for symbol difference: SYM-LABEL (e.g., $_DYNAMIC-1b, $4f-1b)
    // Scan for '-' after position 0 where both sides look like labels.
    if let Some(diff) = parse_immediate_label_diff(s) {
        return Ok(Operand::Immediate(diff));
    }

    // Check for symbol+/-offset expression (e.g., init_top_pgt - 0xffffffff80000000)
    if let Some(sym_offset) = try_parse_immediate_symbol_offset(s) {
        return Ok(Operand::Immediate(sym_offset));
    }

    // Plain symbol
    Ok(Operand::Immediate(ImmediateValue::Symbol(s.to_string())))
}

/// Known relocation modifiers (GNU as). `sym@VER@GOTPCREL` must split at the
/// LAST '@' (the modifier), keeping `sym@VER` as the versioned symbol name;
/// a bare `sym@VER` (suffix not in this list) stays a plain symbol.
fn is_known_reloc_modifier(m: &str) -> bool {
    matches!(
        m.to_ascii_uppercase().as_str(),
        "GOT"
            | "GOTPCREL"
            | "GOTPCRELX"
            | "GOTOFF"
            | "GOTTPOFF"
            | "GOTNTPOFF"
            | "PLT"
            | "TPOFF"
            | "TPREL"
            | "NTPOFF"
            | "INDNTPOFF"
            | "TLSGD"
            | "TLSLD"
            | "DTPMOD"
            | "DTPOFF"
            | "TLSDESC"
            | "PLTOFF"
            | "SECREL"
            | "SIZE"
            | "CALL"
            | "PAGEOFF"
            | "PAGE"
            | "GOTPLT"
    )
}

/// Split `sym@mod` at the LAST '@' when the suffix is a known relocation
/// modifier; otherwise return the whole string as a plain symbol.
fn split_sym_modifier(s: &str) -> Option<(String, String)> {
    let mut last = None;
    for (i, c) in s.char_indices() {
        if c == '@' {
            last = Some(i);
        }
    }
    let i = last?;
    let modifier = &s[i + 1..];
    if modifier.is_empty() || !is_known_reloc_modifier(modifier) {
        return None;
    }
    Some((s[..i].to_string(), modifier.to_string()))
}

/// Try to parse a symbol difference expression in an immediate value.
/// E.g., "_DYNAMIC-1b" -> SymbolDiff("_DYNAMIC", "1b"), "4f-1b" -> SymbolDiff("4f", "1b")
fn parse_immediate_label_diff(s: &str) -> Option<ImmediateValue> {
    // Scan for '-' after position 0. We skip position 0 since a leading '-'
    // is a negation, not a difference.
    for (i, c) in s.char_indices().skip(1) {
        if c == '-' {
            // Trim both sides: `$identity_mapped - 0b` (relocate_kernel_64.S)
            // carries spaces around the minus; without trimming the strings
            // failed is_label_like and the WHOLE expression fell through to a
            // plain symbol named "identity_mapped - 0b", which the linker
            // reported as an undefined reference. Also strip grouping parens:
            // head_64.S's rva() macro expands `$ rva(1b)` to `$((1b) -
            // startup_32)` — the inner label arrives as `(1b)`.
            let lhs = strip_sym_parens(s[..i].trim());
            let rhs = strip_sym_parens(s[i + 1..].trim());
            if !lhs.is_empty() && !rhs.is_empty() && is_label_like(lhs) && is_label_like(rhs) {
                return Some(ImmediateValue::SymbolDiff(lhs.to_string(), rhs.to_string()));
            }
        }
    }
    None
}

/// Parse a memory operand like `offset(%base, %index, scale)` or `symbol(%rip)`.
fn parse_memory_operand(s: &str) -> Result<Operand, String> {
    // Check for segment prefix: %fs:..., %gs:...
    if s.starts_with('%') {
        if let Some(colon_pos) = s.find(':') {
            let seg = s[1..colon_pos].trim_start();
            if is_segment_name(seg) {
                let rest = &s[colon_pos + 1..];
                let mut mem = parse_memory_inner(rest)?;
                mem.segment = Some(seg.to_string());
                return Ok(Operand::Memory(mem));
            }
        }
    }

    let mem = parse_memory_inner(s)?;
    Ok(Operand::Memory(mem))
}

/// Parse memory operand inner part: `offset(%base, %index, scale)`.
fn parse_memory_inner(s: &str) -> Result<MemoryOperand, String> {
    let s = s.trim();

    // EVEX mask/zeroing suffix on a memory operand: `(%rax){%k1}`.
    if s.contains('{') {
        let (clean, mask, zeroing) = parse_evex_mask_suffix(s);
        let mut mem = parse_memory_inner(&clean)?;
        mem.mask = mask;
        mem.zeroing = zeroing;
        return Ok(mem);
    }

    // Find the register-enclosing parentheses: the LAST balanced (...) group.
    // This handles nested parens in the displacement like ((6*8) + 8*16)(%rsp).
    // We scan backwards from the end to find the matching '(' for the final ')'.
    if let Some(paren_end) = s.rfind(')') {
        let mut depth = 0i32;
        let mut paren_start = None;
        for (i, c) in s[..=paren_end].char_indices().rev() {
            match c {
                ')' => depth += 1,
                '(' => {
                    depth -= 1;
                    if depth == 0 {
                        paren_start = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let paren_start =
            paren_start.ok_or_else(|| format!("unmatched paren in memory operand: {}", s))?;
        let disp_str = s[..paren_start].trim();
        let inner = &s[paren_start + 1..paren_end];

        // If there's no displacement before the parens AND the inner content doesn't
        // look like register references (i.e., first part doesn't start with %),
        // then this is a parenthesized displacement expression like (pcpu_hot + 16),
        // not a register reference like (%rsp). Treat the whole thing as displacement.
        if disp_str.is_empty() && !inner.is_empty() {
            let first_part = inner.split(',').next().unwrap_or("").trim();
            if !first_part.starts_with('%') && !first_part.is_empty() {
                // This is a parenthesized expression used as displacement
                let displacement = parse_displacement(inner)?;
                return Ok(MemoryOperand {
                    segment: None,
                    displacement,
                    base: None,
                    index: None,
                    scale: None,
                    mask: None,
                    zeroing: false,
                });
            }
        }

        let displacement = parse_displacement(disp_str)?;

        // Parse base, index, scale from inside parens
        let parts: Vec<&str> = inner.split(',').map(|p| p.trim()).collect();

        // Segment override may appear inside the parens, e.g. the TLS
        // initial-exec access `g_tls@TPOFF(%fs:0)` (or `%gs:...`). The first
        // part `%fs:0` must set the segment and leave the base empty — the
        // address is seg:disp32 with no base/index. Previously this parsed
        // "fs:0" as a base register name, which the encoder defaulted to %rax
        // and silently dropped the segment prefix (miscompiled TLS reads).
        let mut seg_override: Option<String> = None;
        let base = if !parts.is_empty() && !parts[0].is_empty() {
            let p0 = parts[0];
            if let Some(colon) = p0.find(':') {
                if p0.starts_with('%') {
                    let seg = &p0[1..colon];
                    if is_segment_name(seg) {
                        seg_override = Some(seg.to_string());
                        let seg_val = p0[colon + 1..].trim();
                        if !seg_val.is_empty() && seg_val != "0" && seg_val.starts_with('%') {
                            // `%fs:%reg` form: keep the register as base.
                            Some(Register::new(seg_val.trim_start_matches('%')))
                        } else {
                            None
                        }
                    } else {
                        Some(Register::new(p0.trim_start_matches('%')))
                    }
                } else {
                    Some(Register::new(p0.trim_start_matches('%')))
                }
            } else {
                Some(Register::new(p0.trim_start_matches('%')))
            }
        } else {
            None
        };

        let index = if parts.len() > 1 && !parts[1].is_empty() {
            Some(Register::new(parts[1].trim_start_matches('%')))
        } else {
            None
        };

        let scale = if parts.len() > 2 && !parts[2].is_empty() {
            Some(
                parts[2]
                    .parse::<u8>()
                    .map_err(|_| format!("bad scale: {}", parts[2]))?,
            )
        } else {
            None
        };

        Ok(MemoryOperand {
            segment: seg_override,
            displacement,
            base,
            index,
            scale,
            mask: None,
            zeroing: false,
        })
    } else {
        // No parens - could be just a displacement/symbol
        let displacement = parse_displacement(s)?;
        Ok(MemoryOperand {
            segment: None,
            displacement,
            base: None,
            index: None,
            scale: None,
            mask: None,
            zeroing: false,
        })
    }
}

/// Parse a displacement: integer, symbol, symbol+offset, or symbol@modifier.
fn parse_displacement(s: &str) -> Result<Displacement, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(Displacement::None);
    }

    // Strip outer parentheses used for grouping expressions like (pcpu_hot + 16).
    // In AT&T syntax, parenthesized displacement expressions like %gs:(pcpu_hot + 16)(%rip)
    // produce a displacement string of "(pcpu_hot + 16)" that we need to unwrap.
    // Only strip if the entire string is wrapped in balanced parens.
    if s.starts_with('(') && s.ends_with(')') {
        let inner = &s[1..s.len() - 1];
        // Verify the parens are balanced (not e.g. "(a)+(b)")
        let mut depth = 0i32;
        let mut balanced = true;
        for c in inner.chars() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth < 0 {
                        balanced = false;
                        break;
                    }
                }
                _ => {}
            }
        }
        if balanced && depth == 0 {
            return parse_displacement(inner);
        }
    }

    // Try integer
    if let Ok(val) = parse_integer_expr(s) {
        return Ok(Displacement::Integer(val));
    }

    // Symbol with modifier: symbol@GOTPCREL, sym@VER@GOTPCREL, etc.
    if let Some((sym, modifier)) = split_sym_modifier(s) {
        return Ok(Displacement::SymbolMod(sym, modifier));
    }

    // Check for symbol+offset or symbol-offset (e.g., `.Lstr0+1`, `foo-4`)
    // Find the last '+' or '-' that's not at position 0 (to avoid splitting
    // negative numbers or labels starting with '.')
    if let Some(offset_disp) = try_parse_symbol_plus_offset(s) {
        return Ok(offset_disp);
    }

    // Symbol difference: `(gdt) - startup_32` (head_64.S rva() macro as a
    // displacement). Either side may be paren-wrapped or a numeric label,
    // and the LHS may carry a constant addend: `rva(pgtable + 0)` expands
    // to `((pgtable + 0) - startup_32)`. Scan minus positions left to
    // right; at each, try to interpret LHS as `sym [+ const-chain]`.
    for (i, c) in s.char_indices().skip(1) {
        if c == '-' {
            let lhs_raw = strip_sym_parens(s[..i].trim());
            let rhs = strip_sym_parens(s[i + 1..].trim());
            if lhs_raw.is_empty() || rhs.is_empty() || !is_label_like(rhs) {
                continue;
            }
            if is_label_like(lhs_raw) {
                return Ok(Displacement::SymbolDiff(
                    lhs_raw.to_string(),
                    rhs.to_string(),
                ));
            }
            // LHS with folded constant tail: `pgtable + 0`, `sym + (1<<4) - 8`.
            if let Some(DataValue::SymbolOffset(sym, add)) = parse_symbol_expr_chain(lhs_raw) {
                return Ok(Displacement::SymbolDiffAddend(sym, rhs.to_string(), add));
            }
        }
    }

    // Plain symbol
    Ok(Displacement::Symbol(s.to_string()))
}

/// Try to parse a `symbol+offset` or `symbol-offset` expression.
/// Returns None if the string doesn't match this pattern.
fn try_parse_symbol_plus_offset(s: &str) -> Option<Displacement> {
    // Scan for '+' or '-' that separates the symbol from the offset.
    // Skip the first character to avoid splitting on leading sign/dot.
    for (i, c) in s.char_indices().skip(1) {
        if c == '+' || c == '-' {
            let left = s[..i].trim();
            let right_with_sign = s[i..].trim(); // includes the + or -
            if left.is_empty() {
                continue;
            }

            let is_valid_sym = |s: &str| -> bool {
                !s.is_empty()
                    && s.chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '$')
            };

            // Case 1: symbol+offset (e.g. "maxmin_avx+32")
            if let Ok(offset) = parse_integer_expr(right_with_sign) {
                if is_valid_sym(left) {
                    return Some(Displacement::SymbolPlusOffset(left.to_string(), offset));
                }
            }

            // Case 2: offset+symbol (e.g. "32+maxmin_avx") - only for '+'
            if c == '+' {
                if let Ok(offset) = parse_integer_expr(left) {
                    let sym = right_with_sign[1..].trim(); // skip the '+'
                    if is_valid_sym(sym) {
                        return Some(Displacement::SymbolPlusOffset(sym.to_string(), offset));
                    }
                }
            }
        }
    }
    None
}

/// Try to parse a `symbol+offset` or `symbol-offset` expression as an immediate value.
/// This handles cases like `init_top_pgt - 0xffffffff80000000` where the expression
/// mixes a symbol with a large integer constant.
fn try_parse_immediate_symbol_offset(s: &str) -> Option<ImmediateValue> {
    // A parenthesized symbol `(name)` is the same symbol — GAS grouping.
    // The kernel emits `$(srso_safe_ret)+5` via __HANDLE_INTR_SAFERET;
    // rejecting the parens made the whole expression a literal symbol
    // name and the vmlinux link failed on `(srso_safe_ret)+5`.
    fn strip_parens(s: &str) -> &str {
        if s.len() >= 2 && s.starts_with('(') && s.ends_with(')') {
            &s[1..s.len() - 1]
        } else {
            s
        }
    }
    let is_valid_sym = |s: &str| -> bool {
        let s = strip_parens(s);
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '$')
    };

    // Scan for '+' or '-' that separates the symbol from the offset.
    for (i, c) in s.char_indices().skip(1) {
        if c == '+' || c == '-' {
            let left = s[..i].trim();
            let right_with_sign = s[i..].trim();
            if left.is_empty() {
                continue;
            }

            // Case 1: symbol+/-offset (e.g. "init_top_pgt - 0xffffffff80000000")
            if let Ok(offset) = parse_integer_expr(right_with_sign) {
                if is_valid_sym(left) {
                    return Some(ImmediateValue::SymbolPlusOffset(
                        strip_parens(left).to_string(),
                        offset,
                    ));
                }
            }

            // Case 2: offset+symbol (e.g. "32+init_top_pgt") - only for '+'
            if c == '+' {
                if let Ok(offset) = parse_integer_expr(left) {
                    let sym = right_with_sign[1..].trim();
                    if is_valid_sym(sym) {
                        return Some(ImmediateValue::SymbolPlusOffset(
                            strip_parens(sym).to_string(),
                            offset,
                        ));
                    }
                }
            }
        }
    }
    None
}

/// Split `.skip expr, fill` arguments, respecting parentheses.
/// Returns the expression string and the fill byte (default 0).
fn split_skip_args(args: &str) -> (&str, u8) {
    let mut depth = 0i32;
    let mut last_comma = None;
    for (i, c) in args.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => last_comma = Some(i),
            _ => {}
        }
    }
    if let Some(pos) = last_comma {
        let expr = &args[..pos];
        let fill_str = args[pos + 1..].trim();
        let fill = if let Ok(v) = parse_integer_expr(fill_str) {
            v as u8
        } else {
            0u8
        };
        (expr, fill)
    } else {
        (args, 0u8)
    }
}

/// Parse a label diff without spaces, e.g., `663b-661b`.
/// Returns a SymbolDiff DataValue if both sides look like labels.
fn parse_label_diff(s: &str) -> Option<DataValue> {
    for (i, c) in s.char_indices().skip(1) {
        if c == '-' {
            let lhs = &s[..i];
            let rhs = &s[i + 1..];
            if !lhs.is_empty() && !rhs.is_empty() && is_label_like(lhs) && is_label_like(rhs) {
                return Some(DataValue::SymbolDiff(lhs.to_string(), rhs.to_string()));
            }
            // `label-N-.` (kernel vDSO sigreturn: `.long .LSTART_sigreturn-1-.`,
            // the DWARF CFA "initial location minus one" idiom): LHS label,
            // integer addend, then a PC-relative subtraction. Parse as
            // SymbolDiffAddend(label, ".", -N).
            if !lhs.is_empty() && is_label_like(lhs) && rhs.ends_with("-.") {
                let mid = &rhs[..rhs.len() - 2];
                if let Ok(n) = parse_integer_expr(mid) {
                    return Some(DataValue::SymbolDiffAddend(
                        lhs.to_string(),
                        ".".to_string(),
                        -n,
                    ));
                }
            }
            // `a-b+N` / `a-b-N` without spaces (same file:
            // `.long .LEND_sigreturn-.LSTART_sigreturn+1`). The plain
            // SymbolDiff arm above already rejected rhs because of the
            // trailing addend; split it off and fold into the diff.
            if !lhs.is_empty() && is_label_like(lhs) {
                for (j, cj) in rhs.char_indices().skip(1) {
                    if cj == '+' || cj == '-' {
                        let rhs_sym = &rhs[..j];
                        let addend_str = &rhs[j..];
                        if is_label_like(rhs_sym) {
                            if let Ok(n) = parse_integer_expr(addend_str) {
                                return Some(DataValue::SymbolDiffAddend(
                                    lhs.to_string(),
                                    rhs_sym.to_string(),
                                    n,
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// The six x86 segment registers usable as an address override.
///
/// Only `%fs`/`%gs` have a nonzero base in 64-bit mode, but `%cs`/`%ds`/
/// `%es`/`%ss` overrides are still legal encodings that appear in real code
/// (multi-byte NOP padding uses a `%cs` prefix; boot code uses the others).
/// Recognizing only fs/gs made the parser reject them outright.
fn is_segment_name(s: &str) -> bool {
    matches!(s, "cs" | "ds" | "es" | "fs" | "gs" | "ss")
}

/// Check if a string looks like a label (alphanumeric, underscore, dot).
fn is_label_like(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let first = s.as_bytes()[0];
    if !(first.is_ascii_alphabetic() || first == b'_' || first == b'.' || first.is_ascii_digit()) {
        return false;
    }
    s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
}

/// Strip balanced outer parentheses from an expression.
/// E.g. "((1b) - .)" -> "(1b) - ." -> "1b - ." (recursive).
fn strip_outer_parens(s: &str) -> &str {
    let s = s.trim();
    if !s.starts_with('(') || !s.ends_with(')') {
        return s;
    }
    let inner = &s[1..s.len() - 1];
    let mut depth = 0i32;
    for ch in inner.bytes() {
        match ch {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth < 0 {
                    return s;
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

/// Strip outer parentheses from a symbol name. E.g. "(1b)" -> "1b".
fn strip_sym_parens(s: &str) -> &str {
    let s = s.trim();
    if s.starts_with('(') && s.ends_with(')') {
        let inner = &s[1..s.len() - 1];
        let mut depth = 0i32;
        for ch in inner.bytes() {
            match ch {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth < 0 {
                        return s;
                    }
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

/// Parse data values (integers or symbol references).
/// Parse a comma-separated list of floating-point literals into their
/// little-endian IEEE-754 byte representation.
///
/// `double` selects binary64; otherwise binary32. GAS accepts the usual C
/// spellings (`1.0`, `-0.0`, `3.14159`, `1e30`) plus its own `0f`/`0d`/`0e`
/// prefixes, so we accept the same set.
fn parse_float_values(s: &str, double: bool) -> Result<Vec<DataValue>, String> {
    let mut out = Vec::new();
    for part in s.split(',') {
        let t = part.trim();
        if t.is_empty() {
            continue;
        }
        let t = t
            .strip_prefix("0f")
            .or_else(|| t.strip_prefix("0F"))
            .or_else(|| t.strip_prefix("0d"))
            .or_else(|| t.strip_prefix("0D"))
            .or_else(|| t.strip_prefix("0e"))
            .or_else(|| t.strip_prefix("0E"))
            .unwrap_or(t);
        let v: f64 = t
            .parse::<f64>()
            .map_err(|_| format!("bad floating-point literal: {}", t))?;
        if double {
            for b in v.to_bits().to_le_bytes() {
                out.push(DataValue::Integer(b as i64));
            }
        } else {
            for b in (v as f32).to_bits().to_le_bytes() {
                out.push(DataValue::Integer(b as i64));
            }
        }
    }
    Ok(out)
}

fn parse_data_values(s: &str) -> Result<Vec<DataValue>, String> {
    let mut vals = Vec::new();
    for part in s.split(',') {
        let trimmed = strip_outer_parens(part.trim());
        if trimmed.is_empty() {
            continue;
        }

        // PURE constant expressions first: `.quad ((0x1000000) + (0x200000
        // - 1)) & ~(0x200000 - 1)` (header.S pref_address/LOAD_PHYSICAL_ADDR).
        // The " - " diff split below would break the expression at its first
        // minus and emit a relocation against the literal left-half string.
        // parse_integer_expr fails on any symbol reference, so this probe
        // can never swallow a legitimate symbol diff.
        if let Ok(val) = parse_integer_expr(trimmed) {
            vals.push(DataValue::Integer(val));
            continue;
        }

        // General `symbol +/- const-expr [+/- const-expr ...]` chain, e.g. the
        // kernel's `level1_fixmap_pgt + (2 << 12) - 8 + 16`. The " - " split
        // below would break at the FIRST minus and treat everything left of
        // it (including arithmetic) as a symbol name, emitting a relocation
        // against a nonexistent symbol. Fold the whole constant tail first.
        if let Some(val) = parse_symbol_expr_chain(trimmed) {
            vals.push(val);
            continue;
        }

        // A PARENTHESIZED symbol difference with arithmetic around it must be
        // folded before the plain `" - "` split below, which would otherwise
        // tear `(.Lb - .La) * 4` into the nonsense pair `(.Lb` / `.La) * 4`.
        // `strip_sym_parens` then produced a symbol name that matches nothing,
        // and the field was silently emitted as ZERO -- no diagnostic, wrong
        // data. (The same probe exists further down, but only after the split
        // has already destroyed the expression.)
        if trimmed.starts_with('(') {
            if let Some(val) = try_parse_sym_diff_expr(trimmed) {
                vals.push(val);
                continue;
            }
        }

        // Check for symbol difference: .LBB3 - .Ljt_0, or with addend: tr_gdt_end - tr_gdt - 1
        if let Some(minus_pos) = trimmed.find(" - ") {
            let lhs_raw = strip_sym_parens(trimmed[..minus_pos].trim()).to_string();
            let rhs_full = trimmed[minus_pos + 3..].trim();
            // The LEFT side may carry a constant addend:
            //   .word .Ljmp + 1 - trampoline_32bit_src
            // (arch/x86/boot/startup/la57toggle.S computes the offset of the
            // immediate field inside a far jump this way). Splitting only on
            // the first " - " left `lhs` as the literal string ".Ljmp + 1",
            // which is not a symbol. Peel the addend off and fold it in.
            // The LHS may be a whole constant CHAIN: kernel jump-label keys
            // expand to `.quad __tracepoint_read_msr+8 + 0 + 2 - .` (%c0
            // prints "sym+8" and the JUMP_TABLE_ENTRY macro appends
            // ` + 0 + 2`). Peeling only ONE ` + N` term left the literal
            // string "__tracepoint_read_msr+8 + 0" as the symbol name and the
            // link failed with "undefined reference to `sym+8 + 0 + 2'".
            // Fold the full chain first; fall back to the single-term peel.
            let (lhs, lhs_addend) =
                if let Some(DataValue::SymbolOffset(sym, a)) = parse_symbol_expr_chain(&lhs_raw) {
                    (sym, a)
                } else {
                    match lhs_raw.rfind(" + ") {
                        Some(p) => {
                            let sym = strip_sym_parens(lhs_raw[..p].trim());
                            match (
                                is_label_like(sym),
                                parse_integer_expr(lhs_raw[p + 3..].trim()),
                            ) {
                                (true, Ok(a)) => (sym.to_string(), a),
                                _ => (lhs_raw.clone(), 0),
                            }
                        }
                        None => (lhs_raw.clone(), 0),
                    }
                };
            // Parenthesized RHS: probe with parse_sym_addend FIRST. It strips
            // balanced parens and correctly handles forms like `(. + 4)` (the
            // Linux kernel's static_call trampoline: `.long func - (. + 4)`).
            // The rfind-based probes below cannot: rfind(" + ") finds the `+`
            // INSIDE the parens and mis-splits `(. + 4)` into `(." / "4`,
            // both of which fail is_label_like, and the fall-through then
            // silently emits a bogus SymbolDiff against the literal string
            // ". + 4" — resolved as ZERO with no diagnostic (reproduced:
            // `.long target_fn - (. + 4)` came out 00000000; GAS emits
            // R_X86_64_PC32 target_fn-4). With the paren stripped,
            // parse_sym_addend yields (".", 4) and the writer emits the
            // correct PC-relative diff.
            //
            // The guard to `starts_with('(')` is load-bearing: WITHOUT it, a
            // non-parenthesized `a - b + N` would be read as `a - (b + N)`
            // and the trailing addend flipped in sign (broke
            // kernel_altinstr_layout's `.long 760b - 770b + 5`). Un-paren
            // `sym ± N` tails stay on the rfind path below, which folds them
            // with the correct sign.
            if rhs_full.starts_with('(') {
                if let Some((rhs_sym, rhs_add)) = parse_sym_addend(rhs_full) {
                    // Reject BARE numbers: is_label_like accepts a leading
                    // digit (numeric labels like `770b`), but a digit-only
                    // string is a constant expression, not a label —
                    // `a - (1 + 2)` must not become a diff against "1".
                    let is_real_label = is_label_like(&rhs_sym)
                        && !rhs_sym.bytes().all(|c| c.is_ascii_digit());
                    if is_real_label {
                        vals.push(DataValue::SymbolDiffAddend(
                            lhs,
                            rhs_sym,
                            lhs_addend - rhs_add,
                        ));
                        continue;
                    }
                }
            }
            // Check if rhs has an addend: "sym - N" or "sym + N"
            if let Some(rhs_minus) = rhs_full.rfind(" - ") {
                let rhs_sym = strip_sym_parens(rhs_full[..rhs_minus].trim());
                let rhs_add = rhs_full[rhs_minus + 3..].trim();
                if is_label_like(rhs_sym) {
                    if let Ok(addend) = parse_integer_expr(rhs_add) {
                        vals.push(DataValue::SymbolDiffAddend(
                            lhs,
                            rhs_sym.to_string(),
                            lhs_addend - addend,
                        ));
                        continue;
                    }
                }
            }
            if let Some(rhs_plus) = rhs_full.rfind(" + ") {
                let rhs_sym = strip_sym_parens(rhs_full[..rhs_plus].trim());
                let rhs_add = rhs_full[rhs_plus + 3..].trim();
                if is_label_like(rhs_sym) {
                    if let Ok(addend) = parse_integer_expr(rhs_add) {
                        vals.push(DataValue::SymbolDiffAddend(
                            lhs,
                            rhs_sym.to_string(),
                            lhs_addend + addend,
                        ));
                        continue;
                    }
                }
            }
            if lhs_addend != 0 {
                vals.push(DataValue::SymbolDiffAddend(
                    lhs,
                    strip_sym_parens(rhs_full).to_string(),
                    lhs_addend,
                ));
            } else {
                vals.push(DataValue::SymbolDiff(
                    lhs,
                    strip_sym_parens(rhs_full).to_string(),
                ));
            }
            continue;
        }

        // Try integer
        if let Ok(val) = parse_integer_expr(trimmed) {
            vals.push(DataValue::Integer(val));
            continue;
        }

        // Check for label diff without spaces (e.g., 663b-661b)
        if let Some(val) = parse_label_diff(trimmed) {
            vals.push(val);
            continue;
        }

        // A parenthesized symbol difference with arithmetic around it must be
        // folded BEFORE the plain symbol+offset probe, otherwise `(c-a)+7`
        // would be misread as "symbol named `(c-a)` plus 7".
        if trimmed.starts_with('(') {
            if let Some(val) = try_parse_sym_diff_expr(trimmed) {
                vals.push(val);
                continue;
            }
        }

        // Check for symbol+offset or symbol-offset (e.g., GD_struct+128, arr+33)
        if let Some(val) = parse_symbol_offset(trimmed) {
            vals.push(val);
            continue;
        }

        // GNU as symbol-difference forms without spaces or with parentheses
        // (glibc .eh_frame emits `(.LSTART_restore_rt-1)-.`,
        //  `.LEND_restore_rt-(.LSTART_restore_rt-1)`, `.LENDCIE_-.LSTARTCIE_`).
        if let Some(val) = try_parse_sym_diff(trimmed) {
            vals.push(val);
            continue;
        }

        // An arithmetic expression built from a single symbol difference, e.g.
        // `(end-start)*2` or `(end-start)+7`. GAS folds these to a constant
        // because the difference of two same-section labels is absolute.
        // Without this, the whole expression was treated as a SYMBOL NAME and
        // emitted as a relocation against a nonexistent symbol — the datum
        // silently became zero and the object gained an undefined reference.
        if let Some(val) = try_parse_sym_diff_expr(trimmed) {
            vals.push(val);
            continue;
        }

        // Reject anything that cannot be a symbol name outright rather than
        // emitting a bogus relocation for it.
        if !is_label_like(trimmed) {
            return Err(format!("unsupported data expression: {}", trimmed));
        }

        // Symbol reference
        vals.push(DataValue::Symbol(trimmed.to_string()));
    }
    Ok(vals)
}

/// Split `sym`, `sym+N`, `sym-N`, `(sym±N)` into (symbol, addend) where the
/// value equals `symbol + addend`. Returns None for non-label strings.
fn parse_sym_addend(s: &str) -> Option<(String, i64)> {
    let t = s.trim();
    let t = t
        .strip_prefix('(')
        .and_then(|x| x.strip_suffix(')'))
        .unwrap_or(t);
    let t = t.trim();
    if t.is_empty() {
        return None;
    }
    for (i, c) in t.char_indices().skip(1) {
        if c == '+' || c == '-' {
            let sym = t[..i].trim();
            if !is_label_like(sym) {
                continue;
            }
            let num_raw: String = t[i..].chars().filter(|c| !c.is_whitespace()).collect();
            let v: i64 = if let Some(r) = num_raw.strip_prefix('+') {
                r.parse().ok()?
            } else if let Some(r) = num_raw.strip_prefix('-') {
                -r.parse::<i64>().ok()?
            } else {
                continue;
            };
            return Some((sym.to_string(), v));
        }
    }
    if is_label_like(t) {
        Some((t.to_string(), 0))
    } else {
        None
    }
}

/// Parse GNU-as symbol-difference expressions that lack spaces around the
/// operator or use parentheses: `(a-1)-.`, `a-(b-1)`, `(a+2)-b`, `a-b`.
/// Returns DataValue::SymbolDiffAddend(a, b, add) meaning `a - b + add`.
/// Fold an arithmetic expression over ONE symbol difference into a
/// `SymbolDiffScaled` value: `(A-B)*k + c`.
///
/// Accepts `(A-B)`, `(A-B)*k`, `k*(A-B)`, `(A-B)+c`, `(A-B)-c` and the
/// combination `(A-B)*k+c`. Anything else returns `None` so the caller can
/// report it rather than silently mis-encoding it.
fn try_parse_sym_diff_expr(s: &str) -> Option<DataValue> {
    let t = s.trim();
    let open = t.find('(')?;
    let mut depth = 0usize;
    let mut close = None;
    for (i, c) in t[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    let inner = &t[open + 1..close];
    let (a, b) = split_simple_diff(inner)?;

    let before = t[..open].trim();
    let after = t[close + 1..].trim();

    let mut scale: i64 = 1;
    let mut addend: i64 = 0;

    if !before.is_empty() {
        let f = before.strip_suffix('*')?.trim();
        scale = scale.checked_mul(parse_integer_expr(f).ok()?)?;
    }

    let mut div: i64 = 1;
    let mut rest = after;
    if let Some(r) = rest.strip_prefix('*') {
        let (num, tail) = split_leading_int(r.trim())?;
        scale = scale.checked_mul(num)?;
        rest = tail.trim();
    }
    // Division: `(a - b) / 8` (header.S NumberOfRvaAndSizes).
    if let Some(r) = rest.strip_prefix('/') {
        let (num, tail) = split_leading_int(r.trim())?;
        if num == 0 {
            return None;
        }
        div = num;
        rest = tail.trim();
    }
    if !rest.is_empty() {
        let (sign, r) = if let Some(r) = rest.strip_prefix('+') {
            (1i64, r)
        } else if let Some(r) = rest.strip_prefix('-') {
            (-1i64, r)
        } else {
            return None;
        };
        addend = sign.checked_mul(parse_integer_expr(r.trim()).ok()?)?;
        rest = "";
    }
    if !rest.is_empty() {
        return None;
    }
    Some(DataValue::SymbolDiffScaled(a, b, scale, div, addend))
}

/// Split `A-B` into its two label operands, or `None` when the text is not a
/// plain difference of labels.
fn split_simple_diff(s: &str) -> Option<(String, String)> {
    let t = s.trim();
    for (i, c) in t.char_indices().rev() {
        if c != '-' || i == 0 {
            continue;
        }
        let a = t[..i].trim();
        let b = t[i + 1..].trim();
        if is_label_like(a) && is_label_like(b) {
            return Some((a.to_string(), b.to_string()));
        }
    }
    None
}

/// Split a leading decimal/hex integer off the front of `s`.
fn split_leading_int(s: &str) -> Option<(i64, &str)> {
    let bytes = s.as_bytes();
    let mut end = 0;
    if s.starts_with("0x") || s.starts_with("0X") {
        end = 2;
        while end < bytes.len() && (bytes[end] as char).is_ascii_hexdigit() {
            end += 1;
        }
    } else {
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
    }
    if end == 0 {
        return None;
    }
    Some((parse_integer_expr(&s[..end]).ok()?, &s[end..]))
}

fn try_parse_sym_diff(s: &str) -> Option<DataValue> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    for i in 0..bytes.len() {
        let c = bytes[i] as char;
        if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth -= 1;
        } else if c == '-' && depth == 0 && i > 0 {
            let lhs = &s[..i];
            let rhs = &s[i + 1..];
            let (a, add_a) = parse_sym_addend(lhs)?;
            if rhs.trim() == "." {
                // (a ± N) - .  ==  a - . ± N   (PC-relative, GNU as encodes as
                // a PC32 reloc with the addend folded in)
                return Some(DataValue::SymbolDiffAddend(a, ".".to_string(), add_a));
            }
            if let Some((b, add_b)) = parse_sym_addend(rhs) {
                // a + add_a - (b + add_b) = a - b + (add_a - add_b)
                return Some(DataValue::SymbolDiffAddend(a, b, add_a - add_b));
            }
            return None;
        }
    }
    None
}

/// Parse symbol+offset or symbol-offset expressions (e.g., GD_struct+128).
/// Also handles offset+symbol (e.g., 0x9b000000 + pa_real_mode_base).
/// Returns a DataValue::SymbolOffset if the string matches this pattern.
/// Parse `SYMBOL op expr op expr ...` where every `expr` after the symbol is
/// a compile-time integer expression and ops are +/-. Returns the folded
/// SymbolOffset. Bails (None) if the tail references any other symbol - a
/// symbol difference must be handled by the dedicated paths.
fn parse_symbol_expr_chain(s: &str) -> Option<DataValue> {
    let t = s.trim();
    // Find the leading symbol: longest prefix that is label-like.
    let mut sym_end = 0;
    for (i, c) in t.char_indices() {
        if i == 0 {
            if !(c.is_ascii_alphabetic() || c == '_' || c == '.') {
                return None;
            }
        } else if !(c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '$') {
            break;
        }
        sym_end = i + c.len_utf8();
    }
    if sym_end == 0 {
        return None;
    }
    let sym = &t[..sym_end];
    if !is_label_like(sym) {
        return None;
    }
    let tail = t[sym_end..].trim();
    if tail.is_empty() {
        return None; // plain symbol: let the standard path handle it
    }
    // Tail must start with + or - and consist purely of constant arithmetic.
    if !(tail.starts_with('+') || tail.starts_with('-')) {
        return None;
    }
    // The whole tail must evaluate as one integer expression: "+ (2<<12) - 8 + 16".
    match parse_integer_expr(tail) {
        Ok(v) => Some(DataValue::SymbolOffset(sym.to_string(), v)),
        Err(_) => None,
    }
}

fn parse_symbol_offset(s: &str) -> Option<DataValue> {
    // Look for + or - that separates symbol from offset
    // Don't match the leading character (could be .-prefixed label)
    for (i, c) in s.char_indices().skip(1) {
        if c == '+' || c == '-' {
            let left = s[..i].trim();
            let right_with_sign = &s[i..]; // includes the sign

            // Case 1: symbol+offset or symbol-offset (e.g., "GD_struct+128")
            if let Ok(offset) = parse_integer_expr(right_with_sign) {
                if !left.is_empty() && !left.contains(' ') {
                    return Some(DataValue::SymbolOffset(left.to_string(), offset));
                }
            }

            // Case 2: offset+symbol (e.g., "0x9b000000 + pa_real_mode_base") - only for '+'
            if c == '+' {
                if let Ok(offset) = parse_integer_expr(left) {
                    let sym = right_with_sign[1..].trim(); // skip the '+'
                    if !sym.is_empty() && !sym.contains(' ') && is_label_like(sym) {
                        return Some(DataValue::SymbolOffset(sym.to_string(), offset));
                    }
                }
            }
        }
    }
    None
}

/// Parse an integer expression (delegates to the shared expression evaluator).
fn parse_integer_expr(s: &str) -> Result<i64, String> {
    asm_expr::parse_integer_expr(s)
}

/// GAS macro definition
#[derive(Clone, Debug)]
struct GasMacro {
    params: Vec<(String, Option<String>)>, // (name, default_value)
    body: Vec<String>,
}

/// GAS `\@` pseudo-variable: the total number of macro expansions performed
/// so far. Every expansion gets a distinct value — that is what makes
/// `.Lhere_\@:` a unique label per invocation. Leaving `\@` unsubstituted
/// collapsed every use onto ONE label: the kernel's ANNOTATE macro then
/// pointed all `.discard.annotate_insn` entries at the same address instead
/// of at the instruction each one annotates, and objtool rejected the object
/// ("intra_function_call not a direct call").
///
/// The counter is process-global (AtomicU64), not per-expansion-pass: GAS
/// draws nested and recursive expansions from one sequence, and a file-local
/// counter would recycle values when expand_gas_macros_with_state recurses.
/// Distinctness is the only contract — the concrete numbers may differ from
/// GAS's, which no correct assembly can observe.
static MACRO_INVOCATION_COUNTER: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Expand GAS macro directives: .macro/.endm, .purgem, .irp/.endr, .ifc/.endif, .if/.endif, .set
///
/// This runs as a text-level expansion pass before instruction parsing.
/// It handles the Linux kernel's extable_type_reg pattern and similar constructs.
fn expand_gas_macros(lines: &[String]) -> Result<Vec<String>, String> {
    let mut macros = crate::common::fx_hash::FxHashMap::default();
    let mut symbols = crate::common::fx_hash::FxHashMap::default();
    let expanded = expand_gas_macros_with_state(lines, &mut macros, &mut symbols)?;
    Ok(substitute_register_aliases(expanded))
}

/// Substitute `.set alias, %reg` register aliases into instruction operands.
///
/// GAS allows a symbol to alias a REGISTER (`.set state0, %xmm1`) and then
/// uses the bare symbol as an operand (`movdqa copy0,state0`). The kernel
/// vDSO ChaCha implementation (arch/x86/entry/vdso/vgetrandom-chacha.S) is
/// written entirely in this style. Numeric .set symbols are handled by the
/// integer-expression path; register aliases must instead be textually
/// substituted before operand parsing, with word-boundary matching so an
/// alias `i` never fires inside an identifier or directive. Aliases take
/// effect only on lines AFTER their definition (GAS semantics); the .set
/// line itself is dropped since the alias has no ELF-symbol meaning.
fn substitute_register_aliases(lines: Vec<String>) -> Vec<String> {
    use crate::common::fx_hash::FxHashMap;
    let mut reg_aliases: FxHashMap<String, String> = FxHashMap::default();
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        // Comments must go BEFORE alias parsing: the kernel writes
        // `CTX\t= %rdi\t# 1st arg` (sha256-avx2-asm.S), and keeping the
        // comment glued the target into "%rdi\t# 1st arg" — a register name
        // that matches nothing, so the alias silently never resolved.
        let trimmed = strip_comment(&line).trim();
        // Both alias spellings define register aliases:
        //   .set NAME, TARGET      (vgetrandom-chacha.S)
        //   NAME = TARGET          (sha256-avx2-asm.S: `X0 = %ymm4`, and
        //                           rotate_Xs rotates them via `X_ = X0`)
        // TARGET may be a %register or ANOTHER alias (transitive; kernel
        // crc-pclmul builds CONSTS -> V7 -> %xmm7, sha256-avx2's rotate_Xs
        // permutes X0..X3 through X_). GAS evaluates eagerly: the alias
        // snapshots the target's CURRENT value, so later redefinition of
        // the target must not rewrite history — exactly what resolving
        // through the map at definition time gives us.
        let alias_def: Option<(&str, &str)> = if let Some(rest) = trimmed
            .strip_prefix(".set ")
            .or_else(|| trimmed.strip_prefix(".set\t"))
        {
            rest.find(',')
                .map(|comma| (rest[..comma].trim(), rest[comma + 1..].trim()))
        } else if let Some(eq) = trimmed.find('=') {
            // Plain `NAME = TARGET` (not ==, and NAME must be a bare
            // identifier — labels and expressions are not alias defs).
            let name = trimmed[..eq].trim();
            let target = trimmed[eq + 1..].trim();
            let name_is_ident = !name.is_empty()
                && !target.starts_with('=')
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.');
            if name_is_ident {
                Some((name, target))
            } else {
                None
            }
        } else {
            None
        };
        if let Some((name, target)) = alias_def {
            let name_ok = !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.');
            if name_ok {
                if target.starts_with('%') {
                    reg_aliases.insert(name.to_string(), target.to_string());
                    continue; // drop the alias-definition line
                }
                if let Some(resolved) = reg_aliases.get(target) {
                    let resolved = resolved.clone();
                    reg_aliases.insert(name.to_string(), resolved);
                    continue; // drop the alias-definition line
                }
            }
        }
        if reg_aliases.is_empty() || trimmed.starts_with('.') || trimmed.ends_with(':') {
            out.push(line);
            continue;
        }
        // Replace whole-word occurrences of any alias in the operand text.
        let bytes = line.as_bytes();
        let mut replaced = String::with_capacity(line.len());
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c.is_ascii_alphabetic() || c == b'_' || c == b'.' {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
                {
                    i += 1;
                }
                let word = &line[start..i];
                // Never substitute the mnemonic (first word): an alias named
                // like an instruction must not clobber it. Operands start
                // after the first whitespace.
                let is_mnemonic_pos = replaced.trim().is_empty();
                if !is_mnemonic_pos {
                    if let Some(reg) = reg_aliases.get(word) {
                        replaced.push_str(reg);
                        continue;
                    }
                }
                replaced.push_str(word);
            } else if c == b'%' {
                // Skip register names wholesale so an alias equal to a
                // register suffix cannot fire inside one.
                let start = i;
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                replaced.push_str(&line[start..i]);
            } else {
                replaced.push(c as char);
                i += 1;
            }
        }
        out.push(replaced);
    }
    out
}

fn expand_gas_macros_with_state(
    lines: &[String],
    macros: &mut crate::common::fx_hash::FxHashMap<String, GasMacro>,
    symbols: &mut crate::common::fx_hash::FxHashMap<String, i64>,
) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let trimmed = strip_comment(&lines[i]).trim().to_string();

        // .macro name param1:req param2:req ...
        if trimmed.starts_with(".macro ") || trimmed.starts_with(".macro\t") {
            let rest = trimmed[".macro".len()..].trim();
            let (name, params) = parse_macro_def(rest)?;
            let mut body = Vec::new();
            let mut depth = 1;
            i += 1;
            while i < lines.len() {
                let inner = strip_comment(&lines[i]).trim().to_string();
                if inner.starts_with(".macro ") || inner.starts_with(".macro\t") {
                    depth += 1;
                } else if inner == ".endm" {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                body.push(lines[i].clone());
                i += 1;
            }
            if depth != 0 {
                return Err(".macro without matching .endm".to_string());
            }
            macros.insert(name, GasMacro { params, body });
            i += 1;
            continue;
        }

        // .purgem name
        if trimmed.starts_with(".purgem ") || trimmed.starts_with(".purgem\t") {
            let name = trimmed[".purgem".len()..].trim().to_string();
            macros.remove(&name);
            i += 1;
            continue;
        }

        // .set symbol, expr (with symbol resolution)
        if trimmed.starts_with(".set ") || trimmed.starts_with(".set\t") {
            let rest = trimmed[".set".len()..].trim();
            if let Some(comma_pos) = rest.find(',') {
                let sym_name = rest[..comma_pos].trim().to_string();
                let expr_str = rest[comma_pos + 1..].trim();
                // Try to evaluate expression with current symbol values
                let resolved = resolve_set_expr(expr_str, symbols);
                if let Ok(val) = parse_integer_expr(&resolved) {
                    symbols.insert(sym_name.clone(), val);
                    // For local symbols (.L*), don't emit the .set - we handle them internally
                    if sym_name.starts_with(".L") {
                        i += 1;
                        continue;
                    }
                }
                // Emit the .set directive for non-local or unresolvable symbols
                let resolved_line = format!(".set {}, {}", sym_name, resolved);
                result.push(resolved_line);
            } else {
                result.push(lines[i].clone());
            }
            i += 1;
            continue;
        }

        // GAS-style symbol = expr assignment (equivalent to .set symbol, expr)
        // Used by kernel code like: i = 0 / i = i + 1 inside .rept blocks
        if let Some(eq_pos) = trimmed.find('=') {
            // Make sure it's not ==, !=, <=, >= (comparison operators)
            let not_comparison = (eq_pos + 1 >= trimmed.len()
                || trimmed.as_bytes()[eq_pos + 1] != b'=')
                && (eq_pos == 0
                    || (trimmed.as_bytes()[eq_pos - 1] != b'!'
                        && trimmed.as_bytes()[eq_pos - 1] != b'<'
                        && trimmed.as_bytes()[eq_pos - 1] != b'>'));
            if not_comparison {
                let before = trimmed[..eq_pos].trim();
                // Check if before looks like a symbol name: starts with letter or _, no spaces
                if !before.is_empty()
                    && !before.contains(' ')
                    && !before.contains('\t')
                    && !before.contains(':')
                    && !before.starts_with('$')
                    && !before.starts_with('%')
                    && !before.starts_with('.')
                    && (before.as_bytes()[0].is_ascii_alphabetic() || before.as_bytes()[0] == b'_')
                {
                    let expr_str = trimmed[eq_pos + 1..].trim();
                    let resolved = resolve_set_expr(expr_str, symbols);
                    if let Ok(val) = parse_integer_expr(&resolved) {
                        symbols.insert(before.to_string(), val);
                        // Emit as .set so the parser can also handle it
                        let resolved_line = format!(".set {}, {}", before, resolved);
                        result.push(resolved_line);
                        i += 1;
                        continue;
                    }
                    // `NAME = %reg` — the `=` spelling of a REGISTER alias
                    // (kernel sha256-avx2-asm.S: `NUM_BLKS = %rdx`). Not a
                    // numeric symbol; normalize to `.set NAME, %reg` so the
                    // register-alias substitution pass picks it up exactly
                    // like the .set spelling. Without this the line passed
                    // through verbatim and every use of the alias died in
                    // the encoder ("unsupported shlq operands").
                    if expr_str.starts_with('%') {
                        result.push(format!(".set {}, {}", before, expr_str));
                        i += 1;
                        continue;
                    }
                }
            }
        }

        // .irpc var, string  /  .endr -- GAS character iteration: the body
        // expands once per CHARACTER of the string. efi-mixed.S uses
        // `.irpc l, 0123` to unroll four page-table-entry stores with
        // `\l * 8(%eax)` displacements; without this the raw `\l` leaked
        // into the object as a relocation symbol named "\l * 8".
        if trimmed.starts_with(".irpc ") || trimmed.starts_with(".irpc\t") {
            let rest = trimmed[".irpc".len()..].trim();
            let comma = rest
                .find(',')
                .ok_or(".irpc: missing comma after variable")?;
            let var = rest[..comma].trim().to_string();
            let chars_str = rest[comma + 1..].trim();
            // GAS allows the string to be quoted; strip one layer.
            let chars_str = chars_str
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(chars_str);
            let mut body = Vec::new();
            let mut depth = 1;
            i += 1;
            while i < lines.len() {
                let inner = strip_comment(&lines[i]).trim().to_string();
                if inner.starts_with(".irp ")
                    || inner.starts_with(".irp\t")
                    || inner.starts_with(".irpc ")
                    || inner.starts_with(".irpc\t")
                    || inner.starts_with(".rept ")
                    || inner.starts_with(".rept\t")
                {
                    depth += 1;
                } else if inner == ".endr" {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                body.push(lines[i].clone());
                i += 1;
            }
            let mut all_expanded = Vec::new();
            for ch in chars_str.chars() {
                let item = ch.to_string();
                for bline in &body {
                    let mut expanded =
                        asm_preprocess::replace_macro_param(bline, &format!("\\{}", var), &item);
                    expanded = expanded.replace("\\()", "");
                    all_expanded.push(expanded);
                }
            }
            let processed = expand_gas_macros_with_state(&all_expanded, macros, symbols)?;
            result.extend(processed);
            i += 1;
            continue;
        }

        // .irp var, item1, item2, ...  /  .endr
        if trimmed.starts_with(".irp ") || trimmed.starts_with(".irp\t") {
            let rest = trimmed[".irp".len()..].trim();
            let (var, items) = parse_irp_header(rest)?;
            let mut body = Vec::new();
            let mut depth = 1;
            i += 1;
            while i < lines.len() {
                let inner = strip_comment(&lines[i]).trim().to_string();
                if inner.starts_with(".irp ")
                    || inner.starts_with(".irp\t")
                    || inner.starts_with(".rept ")
                    || inner.starts_with(".rept\t")
                {
                    depth += 1;
                } else if inner == ".endr" {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                body.push(lines[i].clone());
                i += 1;
            }
            // Expand: for each item, substitute \var with the item, then recursively process
            let mut all_expanded = Vec::new();
            for item in &items {
                for bline in &body {
                    let mut expanded =
                        asm_preprocess::replace_macro_param(bline, &format!("\\{}", var), item);
                    // Strip GAS macro argument delimiters: \() resolves to empty string
                    expanded = expanded.replace("\\()", "");
                    all_expanded.push(expanded);
                }
            }
            let processed = expand_gas_macros_with_state(&all_expanded, macros, symbols)?;
            result.extend(processed);
            i += 1;
            continue;
        }

        // .if expr / .elseif expr / .else / .endif
        if trimmed.starts_with(".if ")
            || trimmed.starts_with(".if\t")
            || trimmed.starts_with(".if(")
        {
            let rest = if trimmed.starts_with(".if(") {
                &trimmed[".if".len()..]
            } else {
                trimmed[".if".len()..].trim()
            };
            let cond = eval_if_expr(rest, symbols);
            // Collect branches: a chain of (condition, lines) pairs ending with optional else
            let mut branches: Vec<(bool, Vec<String>)> = vec![(cond, Vec::new())];
            let mut current_idx = 0;
            let mut depth = 1;
            i += 1;
            while i < lines.len() {
                let inner = strip_comment(&lines[i]).trim().to_string();
                if is_if_start(&inner) {
                    depth += 1;
                    branches[current_idx].1.push(lines[i].clone());
                } else if inner == ".endif" {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    branches[current_idx].1.push(lines[i].clone());
                } else if depth == 1
                    && (inner.starts_with(".elseif ") || inner.starts_with(".elseif\t"))
                {
                    let elseif_rest = inner[".elseif".len()..].trim();
                    // All branch conditions are evaluated eagerly; harmless for pure comparisons.
                    let elseif_cond = eval_if_expr(elseif_rest, symbols);
                    branches.push((elseif_cond, Vec::new()));
                    current_idx += 1;
                } else if inner == ".else" && depth == 1 {
                    // .else is like .elseif with condition=true (fallback)
                    branches.push((true, Vec::new()));
                    current_idx += 1;
                } else {
                    branches[current_idx].1.push(lines[i].clone());
                }
                i += 1;
            }
            // Choose the first branch whose condition is true
            let empty: Vec<String> = Vec::new();
            let mut chosen_lines: &Vec<String> = &empty;
            for (bcond, blines) in &branches {
                if *bcond {
                    chosen_lines = blines;
                    break;
                }
            }
            let expanded = expand_gas_macros_with_state(chosen_lines, macros, symbols)?;
            result.extend(expanded);
            i += 1;
            continue;
        }

        // .ifb string / .ifnb string / .endif
        // GAS tests whether the argument text is blank after macro substitution.
        // Linux/FFmpeg x86 macro code uses these heavily for optional operands.
        if trimmed == ".ifb"
            || trimmed.starts_with(".ifb ")
            || trimmed.starts_with(".ifb	")
            || trimmed == ".ifnb"
            || trimmed.starts_with(".ifnb ")
            || trimmed.starts_with(".ifnb	")
        {
            let is_ifnb = trimmed.starts_with(".ifnb");
            let rest = if is_ifnb {
                trimmed[".ifnb".len()..].trim()
            } else {
                trimmed[".ifb".len()..].trim()
            };
            let cond = if is_ifnb {
                !eval_ifb(rest)
            } else {
                eval_ifb(rest)
            };
            let mut branches: Vec<(bool, Vec<String>)> = vec![(cond, Vec::new())];
            let mut current_idx = 0;
            let mut depth = 1;
            i += 1;
            while i < lines.len() {
                let inner = strip_comment(&lines[i]).trim().to_string();
                if is_if_start(&inner) {
                    depth += 1;
                    branches[current_idx].1.push(lines[i].clone());
                } else if inner == ".endif" {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    branches[current_idx].1.push(lines[i].clone());
                } else if depth == 1
                    && (inner.starts_with(".elseif ") || inner.starts_with(".elseif	"))
                {
                    let elseif_rest = inner[".elseif".len()..].trim();
                    let elseif_cond = eval_if_expr(elseif_rest, symbols);
                    branches.push((elseif_cond, Vec::new()));
                    current_idx += 1;
                } else if inner == ".else" && depth == 1 {
                    branches.push((true, Vec::new()));
                    current_idx += 1;
                } else {
                    branches[current_idx].1.push(lines[i].clone());
                }
                i += 1;
            }
            let empty: Vec<String> = Vec::new();
            let mut chosen_lines: &Vec<String> = &empty;
            for (bcond, blines) in &branches {
                if *bcond {
                    chosen_lines = blines;
                    break;
                }
            }
            let expanded = expand_gas_macros_with_state(chosen_lines, macros, symbols)?;
            result.extend(expanded);
            i += 1;
            continue;
        }

        // .ifc str1, str2 / .endif
        if trimmed.starts_with(".ifc ") || trimmed.starts_with(".ifc\t") {
            let rest = trimmed[".ifc".len()..].trim();
            let cond = eval_ifc(rest);
            let mut branches: Vec<(bool, Vec<String>)> = vec![(cond, Vec::new())];
            let mut current_idx = 0;
            let mut depth = 1;
            i += 1;
            while i < lines.len() {
                let inner = strip_comment(&lines[i]).trim().to_string();
                if is_if_start(&inner) {
                    depth += 1;
                    branches[current_idx].1.push(lines[i].clone());
                } else if inner == ".endif" {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    branches[current_idx].1.push(lines[i].clone());
                } else if depth == 1
                    && (inner.starts_with(".elseif ") || inner.starts_with(".elseif\t"))
                {
                    let elseif_rest = inner[".elseif".len()..].trim();
                    // All branch conditions are evaluated eagerly; harmless for pure comparisons.
                    let elseif_cond = eval_if_expr(elseif_rest, symbols);
                    branches.push((elseif_cond, Vec::new()));
                    current_idx += 1;
                } else if inner == ".else" && depth == 1 {
                    branches.push((true, Vec::new()));
                    current_idx += 1;
                } else {
                    branches[current_idx].1.push(lines[i].clone());
                }
                i += 1;
            }
            let empty: Vec<String> = Vec::new();
            let mut chosen_lines: &Vec<String> = &empty;
            for (bcond, blines) in &branches {
                if *bcond {
                    chosen_lines = blines;
                    break;
                }
            }
            let expanded = expand_gas_macros_with_state(chosen_lines, macros, symbols)?;
            result.extend(expanded);
            i += 1;
            continue;
        }

        // .error "message" - assembler error directive
        if trimmed.starts_with(".error ") || trimmed.starts_with(".error\t") {
            return Err(format!(
                "assembler error: {}",
                trimmed[".error".len()..].trim()
            ));
        }

        // Check if line is a macro invocation.
        // First, split on semicolons to handle "macrocall args; other_stuff" patterns.
        // GAS treats ';' as a statement separator, so a macro invocation may be
        // followed by other statements on the same line.
        let semi_parts = crate::backend::asm_preprocess::split_on_semicolons(&trimmed);
        let first_part = semi_parts[0].trim();
        let first_word = first_part.split_whitespace().next().unwrap_or("");
        // Strip label prefixes if present (e.g., "label: macroname args").
        // MULTIPLE labels can pile up before the invocation: the kernel's
        // ALTERNATIVE_2 body puts `740: 740: \oldinstr` on one line (each
        // nested alternative contributes its own numeric label), and
        // stripping only ONE left `740: RETPOLINE rax` unrecognized — the
        // encoder then saw a bogus `retpoline` instruction.
        let mut label_prefix_end = 0usize;
        {
            let bytes = first_part.as_bytes();
            let mut pos = 0usize;
            loop {
                // Skip whitespace.
                while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
                    pos += 1;
                }
                let tok_start = pos;
                while pos < bytes.len() && bytes[pos] != b' ' && bytes[pos] != b'\t' {
                    pos += 1;
                }
                if pos > tok_start
                    && bytes[pos - 1] == b':'
                    && first_part[tok_start..pos - 1]
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
                    && !first_part[tok_start..pos - 1].is_empty()
                {
                    label_prefix_end = pos;
                } else {
                    break;
                }
            }
        }
        let after_labels = first_part[label_prefix_end..].trim();
        let macro_name_candidate = if label_prefix_end > 0 {
            after_labels.split_whitespace().next().unwrap_or("")
        } else {
            first_word
        };
        // Also check for C-preprocessor-style invocations: MACRO_NAME(args)
        // In GAS, "MACRO_NAME(arg)" is treated as macro invocation with "(arg)" as arg text
        let macro_name = if macros.contains_key(macro_name_candidate) {
            macro_name_candidate
        } else if let Some(paren_pos) = macro_name_candidate.find('(') {
            let candidate = &macro_name_candidate[..paren_pos];
            if macros.contains_key(candidate) {
                candidate
            } else {
                macro_name_candidate
            }
        } else {
            macro_name_candidate
        };

        // A directive is NEVER a macro invocation: GAS resolves `.type`,
        // `.size`, ... as directives even when a macro of the same bare name
        // exists. Without the guard, a macro whose name collides with a
        // directive word (the kernel defines macros around `type`) could
        // capture the directive line and rewrite it into garbage.
        if macros.contains_key(macro_name) && !macro_name.starts_with('.') {
            let mac = macros[macro_name].clone();
            let args_str = if label_prefix_end > 0 {
                // Emit every stacked label first, each on its own line
                // (they are separate statements in GAS).
                for lab in first_part[..label_prefix_end].split_whitespace() {
                    result.push(lab.to_string());
                }
                after_labels[macro_name.len()..].trim()
            } else {
                first_part[macro_name.len()..].trim()
            };
            let args = parse_macro_args(args_str, &mac.params)?;
            // Sort parameters by name length (longest first) to avoid partial
            // substitution: e.g., \orig must not match before \orig_len.
            let mut sorted_args: Vec<(String, String)> = args.clone();
            sorted_args.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
            // Substitute parameters in body
            // One `\@` value per invocation, fetched before body substitution
            // so every line of this expansion sees the same number.
            let invocation = MACRO_INVOCATION_COUNTER
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                .to_string();
            let mut expanded_body: Vec<String> = mac
                .body
                .iter()
                .map(|line| {
                    let mut l = line.clone();
                    for (pname, pval) in &sorted_args {
                        l = asm_preprocess::replace_macro_param(&l, &format!("\\{}", pname), pval);
                    }
                    if l.contains("\\@") {
                        l = l.replace("\\@", &invocation);
                    }
                    // Strip GAS macro argument delimiters: \() resolves to empty string.
                    // Used to separate parameter names from adjacent text,
                    // e.g., \op\()_safe_regs -> rdmsr_safe_regs
                    l = l.replace("\\()", "");
                    l
                })
                .collect();
            // Substituting a parameter can splice several statements into ONE
            // body line (a quoted ALTERNATIVE argument such as
            // "nop;ANNOTATE type=7; call 772f"). Break those apart before the
            // recursive pass so a macro invocation that ended up after a `;`
            // is still recognised as the first word of a statement.
            let mut split_body: Vec<String> = Vec::with_capacity(expanded_body.len());
            for line in expanded_body {
                if line.contains(';') {
                    for part in crate::backend::asm_preprocess::split_on_semicolons(&line) {
                        let p = part.trim();
                        if !p.is_empty() {
                            split_body.push(p.to_string());
                        }
                    }
                } else {
                    split_body.push(line);
                }
            }
            expanded_body = split_body;
            // Recursively expand the body (handles nested .irp, .set, .if, etc.)
            expanded_body = expand_gas_macros_with_state(&expanded_body, macros, symbols)?;
            result.extend(expanded_body);
            // Emit remaining semicolon-separated parts as separate lines.
            //
            // They must be re-expanded, not pushed raw: a macro invocation can
            // follow another statement on the same line, and after a macro
            // argument is substituted the resulting text frequently contains
            // one. The kernel's FILL_RETURN_BUFFER passes
            //     "nop;ANNOTATE type=7; call 772f; ..."
            // as a quoted ALTERNATIVE_2 argument, so `ANNOTATE` only ever
            // appears after a `;` once the outer macro has expanded. Pushing it
            // verbatim left it to the instruction encoder, which reported
            // "unhandled instruction: annotate [Label(\"type=7\")]".
            let rest_parts: Vec<String> = semi_parts[1..]
                .iter()
                .map(|sp| sp.trim().to_string())
                .filter(|sp| !sp.is_empty())
                .collect();
            if !rest_parts.is_empty() {
                let expanded_rest = expand_gas_macros_with_state(&rest_parts, macros, symbols)?;
                result.extend(expanded_rest);
            }
            i += 1;
            continue;
        }

        // For regular lines, resolve any .set symbol references
        if !symbols.is_empty() {
            let resolved = resolve_set_expr(&lines[i], symbols);
            result.push(resolved);
        } else {
            result.push(lines[i].clone());
        }
        i += 1;
    }

    Ok(result)
}

/// Parse a .macro definition header: "name param1:req param2:req ..."
fn parse_macro_def(rest: &str) -> Result<(String, Vec<(String, Option<String>)>), String> {
    // Tokenize the parameter list at PAREN DEPTH 0 only.
    //
    // Splitting on raw whitespace shreds a default value that contains spaces:
    //     .macro FILL_RETURN_BUFFER reg:req nr:req ftr:req ftr2=ALT_NOT(X86_FEATURE_ALWAYS)
    // whose default expands to `(((1 << 0) << 16) | ((3*32+21)))`. The spaces
    // inside it became parameter separators, inventing parameters named `<<`
    // and `0)`, and `\ftr2` expanded to the fragment `(((1` -- which surfaced
    // as "unsupported data expression: (((1" on the `.4byte` in
    // altinstr_entry (arch/x86/entry/entry.S).
    //
    // Commas are equally valid separators in GAS, so both are honoured, but
    // only outside parentheses and quotes.
    fn split_params(s: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = String::new();
        let mut depth = 0i32;
        let mut in_quote = false;
        for c in s.chars() {
            if in_quote {
                cur.push(c);
                if c == '"' {
                    in_quote = false;
                }
                continue;
            }
            match c {
                '"' => {
                    in_quote = true;
                    cur.push(c);
                }
                '(' => {
                    depth += 1;
                    cur.push(c);
                }
                ')' => {
                    depth -= 1;
                    cur.push(c);
                }
                _ if depth == 0 && (c.is_whitespace() || c == ',') => {
                    if !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                }
                _ => cur.push(c),
            }
        }
        if !cur.is_empty() {
            out.push(cur);
        }
        out
    }

    let parts = split_params(rest);
    if parts.is_empty() {
        return Err(".macro: missing name".to_string());
    }
    let name = parts[0].clone();
    let mut params = Vec::new();
    for part in &parts[1..] {
        // `param=default` wins over `param:req`: a default may itself contain
        // a colon, and the qualifier is not part of the parameter NAME.
        if let Some(eq) = part.find('=') {
            let pname = part[..eq].trim();
            let pname = pname.split(':').next().unwrap_or(pname).to_string();
            let default = part[eq + 1..].to_string();
            params.push((pname, Some(default)));
        } else if let Some(colon) = part.find(':') {
            params.push((part[..colon].to_string(), None));
        } else {
            params.push((part.clone(), None));
        }
    }
    Ok((name, params))
}

/// Parse macro invocation arguments: "param1=val1, param2=val2" or positional
fn parse_macro_args(
    args_str: &str,
    params: &[(String, Option<String>)],
) -> Result<Vec<(String, String)>, String> {
    let mut result = Vec::new();
    if args_str.is_empty() {
        // Use defaults for all params
        for (name, default) in params {
            result.push((name.clone(), default.clone().unwrap_or_default()));
        }
        return Ok(result);
    }

    // Split on commas (respecting parentheses)
    let arg_parts = split_macro_args(args_str);

    // Helper: strip surrounding quotes from GAS macro arguments
    // In GAS, "" means empty string, "foo bar" means foo bar
    let strip_quotes = |s: &str| -> String {
        let t = s.trim();
        if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
            t[1..t.len() - 1].to_string()
        } else {
            t.to_string()
        }
    };

    // GAS supports mixed positional and named arguments.
    // Positional args fill params left-to-right, named args (key=val) override by name.
    // Example: "0 asm_foo exc_foo has_error_code=0" with params (vector, asmsym, cfunc, has_error_code)
    // → positional: vector=0, asmsym=asm_foo, cfunc=exc_foo; named: has_error_code=0
    let mut positional_idx = 0;
    let mut arg_map = crate::common::fx_hash::FxHashMap::default();
    let mut positional_vals: Vec<(usize, String)> = Vec::new();

    for part in &arg_parts {
        let part = part.trim();
        if let Some(eq_pos) = part.find('=') {
            let key = part[..eq_pos].trim();
            // Only treat as named if the key matches a known parameter name
            if params.iter().any(|(pname, _)| pname == key) {
                let val = strip_quotes(part[eq_pos + 1..].trim());
                arg_map.insert(key.to_string(), val);
                continue;
            }
        }
        // Positional argument
        positional_vals.push((positional_idx, strip_quotes(part)));
        positional_idx += 1;
    }

    // Build result: start with positional, then override with named
    let mut pos_iter = positional_vals.into_iter();
    for (name, default) in params {
        if let Some(val) = arg_map.get(name) {
            result.push((name.clone(), val.clone()));
        } else if let Some((_, val)) = pos_iter.next() {
            result.push((name.clone(), val));
        } else {
            result.push((name.clone(), default.clone().unwrap_or_default()));
        }
    }

    Ok(result)
}

/// Split macro arguments on commas and spaces, respecting parentheses.
///
/// GAS allows both commas and spaces/tabs as macro argument separators.
/// e.g., `base=%rsp offset=0` or `base=%rsp, offset=0` both give two args.
/// Consecutive separators are collapsed (no empty args from double spaces).
fn split_macro_args(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut in_quotes = false;
    let mut current = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            '\'' if !in_quotes => {
                // GAS character literal: 'x (optionally closed 'x'). The byte
                // after the quote is data — even a space, comma, or paren
                // (relocate_kernel_64.S: `pr 'r', '8', ' ', ':', %r8`).
                // Copy the literal verbatim so it never splits an argument.
                current.push('\'');
                if i + 1 < bytes.len() {
                    if bytes[i + 1] == b'\\' && i + 2 < bytes.len() {
                        current.push('\\');
                        current.push(bytes[i + 2] as char);
                        i += 2;
                    } else {
                        current.push(bytes[i + 1] as char);
                        i += 1;
                    }
                    if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                        current.push('\'');
                        i += 1;
                    }
                }
            }
            '(' if !in_quotes => {
                depth += 1;
                current.push(ch);
            }
            ')' if !in_quotes => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 && !in_quotes => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    parts.push(trimmed);
                }
                current.clear();
                // Skip any whitespace after comma
                while i + 1 < bytes.len() && (bytes[i + 1] == b' ' || bytes[i + 1] == b'\t') {
                    i += 1;
                }
            }
            ' ' | '\t' if depth == 0 && !in_quotes => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    parts.push(trimmed);
                    current.clear();
                }
                // Skip remaining whitespace
                while i + 1 < bytes.len() && (bytes[i + 1] == b' ' || bytes[i + 1] == b'\t') {
                    i += 1;
                }
                // If next char is comma, let the comma handler deal with it
            }
            _ => current.push(ch),
        }
        i += 1;
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        parts.push(trimmed);
    }
    parts
}

/// Parse .irp header: "var, item1, item2, ..."
fn parse_irp_header(rest: &str) -> Result<(String, Vec<String>), String> {
    // Format: var,item1,item2,... or var item1,item2,...
    let rest = rest.trim();
    // First find the variable name (before the first comma)
    let comma_pos = rest.find(',').ok_or(".irp: missing comma after variable")?;
    let var = rest[..comma_pos].trim().to_string();
    let items_str = rest[comma_pos + 1..].trim();
    let items: Vec<String> = items_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    Ok((var, items))
}

/// Evaluate .ifb/.ifnb blank-argument test.  GAS treats an omitted
/// argument, empty string, or macro argument that expands to nothing as blank.
fn eval_ifb(rest: &str) -> bool {
    let t = rest.trim();
    if t.is_empty() {
        return true;
    }
    if (t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\'')) {
        return t.len() <= 2;
    }
    false
}

/// Evaluate .ifc string comparison: "str1, str2" => true if equal
fn eval_ifc(rest: &str) -> bool {
    if let Some(comma_pos) = rest.find(',') {
        let s1 = rest[..comma_pos].trim();
        let s2 = rest[comma_pos + 1..].trim();
        s1 == s2
    } else {
        false
    }
}

/// Resolve symbols in a .set expression string.
/// Uses whole-word matching to avoid replacing substrings inside identifiers,
/// register names, or instruction mnemonics (e.g., replacing `i` inside `rip`).
fn resolve_set_expr(
    expr: &str,
    symbols: &crate::common::fx_hash::FxHashMap<String, i64>,
) -> String {
    // NEVER substitute inside the directive mnemonic.
    //
    // Substitution is whole-word and `.` counts as a word boundary, so a
    // symbol named `type` rewrote the directive `.type` into `.3`. The kernel
    // hits this hard: arch/x86/include/asm/unwind_hints.h does `.set type, 3`
    // inside UNWIND_HINT, and every later `.type foo STT_FUNC` produced by
    // SYM_FUNC_END silently became `.3 foo STT_FUNC`. All 26 affected function
    // symbols stayed STT_NOTYPE and objtool rejected the object with
    // "unexpected relocation symbol type in .rela.discard...: 0".
    //
    // This function is called in two DIFFERENT contexts and the guard is
    // only correct in one of them:
    //  * whole LINES (the regular-line pass): the first token may be a
    //    directive mnemonic and must be protected;
    //  * bare EXPRESSIONS (the .set value, `sym = expr`): there is no
    //    mnemonic, and an expression naturally starts with '.' whenever it
    //    references a `.L` local symbol.
    // The first version applied the guard unconditionally AND recursed into
    // itself for the operand text, so `.Lfound+1` — no whitespace — was
    // consumed whole as a "mnemonic" and never substituted:
    // `.set .Lfound, .Lfound+1` silently kept 0 and the kernel's
    // extable_type_reg register scan (.irp + .ifc counting into .Lfound)
    // died with "extable_type_reg: bad register argument" on every uaccess
    // object. Distinguish the contexts: a leading '.' token is a directive
    // only if it is FOLLOWED BY MORE TEXT (directives never end the line
    // they start — they have operands or none, but a bare `.Lsym+1`
    // expression is not a directive); `.Lfound` / `.Lfound+1` contain
    // expression characters (+,-,digits after the identifier) or ARE the
    // whole text, which no directive mnemonic is.
    let lead = expr.len() - expr.trim_start().len();
    let body = &expr[lead..];
    if body.starts_with('.') {
        let end = body.find(|c: char| c.is_whitespace()).unwrap_or(body.len());
        let (first_tok, rest) = body.split_at(end);
        // Directive mnemonics are pure `.` + identifier tokens (`.type`,
        // `.long`, `.size`, `.text`, ...). Two things are NOT mnemonics:
        //  * tokens containing expression operators (`.Lfound+1` — the whole
        //    .set value arrives here as one token with no whitespace);
        //  * `.L`-prefixed tokens — GAS's local-symbol namespace, which no
        //    directive name inhabits (`.Lfound` alone is an expression).
        // Operand-less directives (`.text` with empty rest) ARE mnemonics
        // and stay protected: a user symbol named `text` must not turn
        // `.text` into `.3`.
        let is_mnemonic = first_tok[1..].bytes().all(is_ident_char) && !first_tok.starts_with(".L");
        if is_mnemonic {
            return format!(
                "{}{}{}",
                &expr[..lead],
                first_tok,
                substitute_set_symbols(rest, symbols)
            );
        }
    }
    substitute_set_symbols(expr, symbols)
}

/// Plain whole-word .set-symbol substitution, no directive-mnemonic logic.
fn substitute_set_symbols(
    text: &str,
    symbols: &crate::common::fx_hash::FxHashMap<String, i64>,
) -> String {
    let mut result = text.to_string();
    // Sort by length (longest first) to avoid partial replacements
    let mut sym_list: Vec<_> = symbols.iter().collect();
    sym_list.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    for (name, val) in sym_list {
        result = replace_whole_word(&result, name, &val.to_string());
    }
    result
}

/// Replace all whole-word occurrences of `word` with `replacement`.
/// A word boundary is any position where the adjacent character is not
/// alphanumeric or underscore (i.e., not an identifier character).
fn replace_whole_word(text: &str, word: &str, replacement: &str) -> String {
    if word.is_empty() || text.is_empty() || text.len() < word.len() {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let word_bytes = word.as_bytes();
    let mut result = String::with_capacity(text.len());
    let mut i = 0;
    while i + word_bytes.len() <= bytes.len() {
        if &bytes[i..i + word_bytes.len()] == word_bytes {
            // Check left boundary: either start of string or non-identifier char
            let left_ok = i == 0 || !is_ident_char(bytes[i - 1]);
            // Check right boundary: either end of string or non-identifier char
            let right_ok =
                i + word_bytes.len() >= bytes.len() || !is_ident_char(bytes[i + word_bytes.len()]);
            if left_ok && right_ok {
                result.push_str(replacement);
                i += word_bytes.len();
                continue;
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    // Append any remaining characters
    while i < bytes.len() {
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}

/// Check if a byte is an identifier character (alphanumeric or underscore).
fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Check if a line starts a new conditional assembly block (.if, .ifc, .ifdef, .ifndef).
fn is_if_start(trimmed: &str) -> bool {
    trimmed.starts_with(".if ")
        || trimmed.starts_with(".if\t")
        || trimmed.starts_with(".if(")
        || trimmed.starts_with(".ifc ")
        || trimmed.starts_with(".ifc\t")
        || trimmed == ".ifb"
        || trimmed.starts_with(".ifb ")
        || trimmed.starts_with(".ifb\t")
        || trimmed == ".ifnb"
        || trimmed.starts_with(".ifnb ")
        || trimmed.starts_with(".ifnb\t")
        || trimmed.starts_with(".ifdef ")
        || trimmed.starts_with(".ifndef ")
}

/// Evaluate a `.if` expression for the x86 assembler.
///
/// Resolves `.set` symbols and x86 register names (%rsp, %rbp, etc.) before
/// evaluating the expression. Supports `==`, `!=`, `>=`, `<=`, `>`, `<`
/// comparison operators and full arithmetic.
fn eval_if_expr(expr: &str, symbols: &crate::common::fx_hash::FxHashMap<String, i64>) -> bool {
    let resolved = resolve_set_expr(expr, symbols);
    asm_preprocess::eval_if_condition_with_resolver(&resolved, |s| {
        asm_preprocess::resolve_x86_registers(s)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GAS `\@` must be substituted with a DISTINCT value per macro
    /// expansion (kernel ANNOTATE emits `.Lhere_\@:` once per use; a shared
    /// value collapses every annotation onto one label).
    #[test]
    fn test_macro_at_counter_distinct_per_invocation() {
        let lines: Vec<String> = vec![
            ".macro ANN".into(),
            ".Lhere_\\@:".into(),
            ".long .Lhere_\\@ - .".into(),
            ".endm".into(),
            "ANN".into(),
            "ANN".into(),
            "ANN".into(),
        ];
        let out = expand_gas_macros(&lines).unwrap();
        let labels: Vec<&String> = out
            .iter()
            .filter(|l| l.trim_end().ends_with(':') && l.contains(".Lhere_"))
            .collect();
        assert_eq!(
            labels.len(),
            3,
            "three expansions, three label defs: {:?}",
            out
        );
        // All three labels must be pairwise distinct.
        assert_ne!(labels[0], labels[1]);
        assert_ne!(labels[1], labels[2]);
        assert_ne!(labels[0], labels[2]);
        // No `\@` may survive expansion.
        assert!(
            out.iter().all(|l| !l.contains("\\@")),
            "unsubstituted \\@: {:?}",
            out
        );
        // Each reference must use ITS invocation's label.
        for (def, reference) in labels
            .iter()
            .zip(out.iter().filter(|l| l.contains(".long .Lhere_")))
        {
            let def_name = def.trim().trim_end_matches(':');
            assert!(
                reference.contains(def_name),
                "reference {:?} does not match definition {:?}",
                reference,
                def
            );
        }
    }

    /// .macro parameter defaults containing parens/spaces/commas must
    /// survive as ONE value (kernel FILL_RETURN_BUFFER's
    /// `ftr2=(((1 << 0) << 16) | (( 3*32+21)))`), and `:req` qualifiers
    /// must not become part of the parameter name.
    #[test]
    fn test_macro_paren_default_and_req_qualifier() {
        let lines: Vec<String> = vec![
            ".macro FRB reg:req nr:req ftr2=(((1 << 0) << 16) | (( 3*32+21)))".into(),
            "mov $\\nr, \\reg".into(),
            ".long \\ftr2".into(),
            ".endm".into(),
            "FRB %rax, 32".into(),
        ];
        let out = expand_gas_macros(&lines).unwrap();
        let joined = out.join("\n");
        assert!(
            joined.contains("mov $32, %rax"),
            ":req parameter did not bind: {:?}",
            out
        );
        assert!(
            joined.contains(".long (((1 << 0) << 16) | (( 3*32+21)))"),
            "parenthesized default was shredded: {:?}",
            out
        );
    }

    /// A directive is never a macro invocation: a macro named `type` must
    /// not capture `.type` lines (kernel objtool.h defines such macros; a
    /// capture rewrote `.type f STT_FUNC` into garbage and left the symbol
    /// STT_NOTYPE).
    #[test]
    fn test_directive_not_captured_by_same_name_macro() {
        let lines: Vec<String> = vec![
            ".macro type arg".into(),
            ".long \\arg".into(),
            ".endm".into(),
            ".type myfn, @function".into(),
            "type 7".into(),
        ];
        let out = expand_gas_macros(&lines).unwrap();
        let joined = out.join("\n");
        assert!(
            joined.contains(".type myfn, @function"),
            ".type directive was rewritten: {:?}",
            out
        );
        assert!(
            joined.contains(".long 7"),
            "bare `type` invocation must still expand: {:?}",
            out
        );
    }

    /// `.set alias, %reg` register aliases substitute into operands with
    /// word-boundary matching, including transitive chains
    /// (crc-pclmul-template.S: CONSTS -> V7 -> %xmm7). Resolution is
    /// eager: redefining the middle link later must not change earlier
    /// snapshots.
    #[test]
    fn test_set_register_alias_transitive_and_eager() {
        let lines: Vec<String> = vec![
            ".set V7, %xmm7".into(),
            ".set CONSTS, V7".into(),
            "movdqa -32(%rcx), CONSTS".into(),
            ".set V7, %xmm2".into(),
            "movdqa (%rdx), V7".into(),
        ];
        let out = expand_gas_macros(&lines).unwrap();
        let joined = out.join("\n");
        assert!(
            joined.contains("movdqa -32(%rcx), %xmm7"),
            "transitive alias failed: {:?}",
            out
        );
        assert!(
            joined.contains("movdqa (%rdx), %xmm2"),
            "redefinition not honored for later uses: {:?}",
            out
        );
        assert!(!joined.contains("CONSTS"), "alias leaked: {:?}", out);
    }

    /// `.set` numeric symbols must never rewrite a DIRECTIVE mnemonic:
    /// after `.set type, 3`, the directive `.type` must stay `.type`
    /// (unwind_hints.h does exactly this and 26 kernel symbols went
    /// STT_NOTYPE when `.type` became `.3`).
    #[test]
    fn test_set_symbol_does_not_rewrite_directives() {
        let lines: Vec<String> = vec![
            ".set type, 3".into(),
            ".long type".into(),
            ".type f, @function".into(),
        ];
        let out = expand_gas_macros(&lines).unwrap();
        let joined = out.join("\n");
        assert!(
            joined.contains(".long 3"),
            ".set value not applied: {:?}",
            out
        );
        assert!(
            joined.contains(".type f, @function"),
            ".type directive corrupted: {:?}",
            out
        );
    }

    /// The directive-mnemonic guard must NOT swallow `.L` local symbols:
    /// the .set VALUE expression arrives as a bare token (`.Lfound+1`, no
    /// whitespace), and treating it as a "mnemonic" skipped substitution
    /// entirely. `.set .Lfound, .Lfound+1` then silently kept the old
    /// value, and the kernel's extable_type_reg register scan
    /// (.irp + .ifc incrementing .Lfound) failed on EVERY uaccess object
    /// with "extable_type_reg: bad register argument".
    #[test]
    fn test_set_local_symbol_increment_and_ifc_counting() {
        // Direct increment.
        let lines: Vec<String> = vec![
            ".set .Lfound, 0".into(),
            ".set .Lfound, .Lfound+1".into(),
            ".long .Lfound".into(),
        ];
        let out = expand_gas_macros(&lines).unwrap();
        assert!(
            out.join("\n").contains(".long 1"),
            ".Lfound increment lost: {:?}",
            out
        );

        // The full kernel shape: .irp register scan with .ifc counting.
        let lines2: Vec<String> = vec![
            ".macro tm reg:req".into(),
            ".set .Lfound, 0".into(),
            ".irp rs,rax,rcx,rdi".into(),
            ".ifc \\reg, %\\rs".into(),
            ".set .Lfound, .Lfound+1".into(),
            ".endif".into(),
            ".endr".into(),
            ".if (.Lfound != 1)".into(),
            ".error \"bad reg\"".into(),
            ".endif".into(),
            ".endm".into(),
            "tm reg=%rdi".into(),
        ];
        let out2 = expand_gas_macros(&lines2);
        assert!(out2.is_ok(), "extable_type_reg shape failed: {:?}", out2);
    }

    /// Session-85 (Agent-Z audit, hunk #7): the static_call trampoline shape
    /// `.long func - (. + 4)` must parse to SymbolDiffAddend(func, ".", -4).
    /// Before the fix the naive ` - ` split grabbed the parenthesized RHS,
    /// the rfind probes failed on `(."`, and the fall-through emitted a bogus
    /// SymbolDiff against the literal string ". + 4" — resolved as a SILENT
    /// ZERO with no relocation (GAS: R_X86_64_PC32 func-4).
    #[test]
    fn test_static_call_parenthesized_dot_addend() {
        let vals = parse_data_values("target_fn - (. + 4)").unwrap();
        assert_eq!(vals.len(), 1, "expected a single data value: {:?}", vals);
        match &vals[0] {
            DataValue::SymbolDiffAddend(a, b, add) => {
                assert_eq!(a, "target_fn");
                assert_eq!(b, ".");
                assert_eq!(*add, -4);
            }
            other => panic!("func - (. + 4) misparsed: {:?}", other),
        }
        // The LHS may carry its own addend (JUMP_TABLE_ENTRY chains):
        // `sym+8 - (. + 4)` == sym - . + 4.
        let vals2 = parse_data_values("sym+8 - (. + 4)").unwrap();
        match &vals2[0] {
            DataValue::SymbolDiffAddend(a, b, add) => {
                assert_eq!(a, "sym");
                assert_eq!(b, ".");
                assert_eq!(*add, 4);
            }
            other => panic!("sym+8 - (. + 4) misparsed: {:?}", other),
        }
    }

    /// The paren-RHS probe must NOT steal unparenthesized trailing addends:
    /// `760b - 770b + 5` is `(760b - 770b) + 5`, NOT `760b - (770b + 5)`.
    /// Kernel alt_instr layout (kernel_altinstr_layout regression) depends on
    /// the sign staying positive.
    #[test]
    fn test_diff_trailing_addend_sign_preserved() {
        let vals = parse_data_values("760b - 770b + 5").unwrap();
        match &vals[0] {
            DataValue::SymbolDiffAddend(a, b, add) => {
                assert_eq!(a, "760b");
                assert_eq!(b, "770b");
                assert_eq!(*add, 5);
            }
            other => panic!("760b - 770b + 5 misparsed: {:?}", other),
        }
    }

    /// Spaceless label arithmetic in data directives (vdso32 sigreturn.S):
    /// `label-N-.` and `a-b+N` must parse as symbol diffs with addends,
    /// not fall through to "unsupported data expression".
    #[test]
    fn test_spaceless_label_diff_addend_forms() {
        match parse_label_diff(".LSTART_sigreturn-1-.") {
            Some(DataValue::SymbolDiffAddend(a, b, n)) => {
                assert_eq!(a, ".LSTART_sigreturn");
                assert_eq!(b, ".");
                assert_eq!(n, -1);
            }
            other => panic!("label-N-. misparsed: {:?}", other),
        }
        match parse_label_diff(".LEND_sigreturn-.LSTART_sigreturn+1") {
            Some(DataValue::SymbolDiffAddend(a, b, n)) => {
                assert_eq!(a, ".LEND_sigreturn");
                assert_eq!(b, ".LSTART_sigreturn");
                assert_eq!(n, 1);
            }
            other => panic!("a-b+N misparsed: {:?}", other),
        }
    }

    #[test]
    fn test_parse_simple() {
        let asm = r#"
.section .text
.globl main
.type main, @function
main:
.cfi_startproc
    pushq %rbp
    movq %rsp, %rbp
    xorl %eax, %eax
    popq %rbp
    ret
.cfi_endproc
.size main, .-main
"#;
        let items = parse_asm(asm).unwrap();
        // Should parse without errors
        let labels: Vec<_> = items
            .iter()
            .filter(|i| matches!(i, AsmItem::Label(_)))
            .collect();
        assert_eq!(labels.len(), 1);
    }

    #[test]
    fn test_parse_memory_operand() {
        let mem = parse_memory_inner("-8(%rbp)").unwrap();
        assert!(matches!(mem.displacement, Displacement::Integer(-8)));
        assert_eq!(mem.base.as_ref().unwrap().name, "rbp");
    }

    #[test]
    fn test_parse_rip_relative() {
        let mem = parse_memory_inner("x(%rip)").unwrap();
        assert!(matches!(mem.displacement, Displacement::Symbol(ref s) if s == "x"));
        assert_eq!(mem.base.as_ref().unwrap().name, "rip");
    }

    #[test]
    fn test_parse_integer() {
        assert_eq!(parse_integer_expr("42").unwrap(), 42);
        assert_eq!(parse_integer_expr("-1").unwrap(), -1);
        assert_eq!(parse_integer_expr("0xff").unwrap(), 255);
        assert_eq!(parse_integer_expr("0").unwrap(), 0);
    }
}
