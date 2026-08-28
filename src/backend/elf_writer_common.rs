//! Shared ELF relocatable object file writer for x86-64 and i686.
//!
//! Both x86-64 and i686 assemblers share ~90% of their ELF writer logic:
//! section management, label tracking, symbol attributes, jump relaxation,
//! relocation resolution, and ELF emission. This module extracts that
//! shared code into a generic `ElfWriterCore<A>`, parameterized by an
//! `X86Arch` trait that provides the architecture-specific pieces:
//!
//! - Relocation type constants (R_X86_64_* vs R_386_*)
//! - ELF class and machine type (ELFCLASS64/EM_X86_64 vs ELFCLASS32/EM_386)
//! - Instruction encoding dispatch
//! - REL vs RELA format handling
//!
//! Both x86-64 and i686 support deferred `.skip` expressions and deferred
//! byte-sized symbol diffs (needed by the Linux kernel's alternatives
//! framework). These are handled as optional extensions controlled by
//! the `supports_deferred_skips()` trait method.

use crate::backend::elf::{
    self as elf_mod, parse_section_flags, resolve_numeric_labels, ElfConfig, ObjReloc, ObjSection,
    ObjSymbol, SymbolTableInput, SHF_ALLOC, SHF_EXECINSTR, SHF_WRITE, SHT_NOBITS, SHT_PROGBITS,
    STB_GLOBAL,
    STB_LOCAL,
    STB_WEAK, STT_FUNC, STT_GNU_IFUNC, STT_NOTYPE, STT_OBJECT, STT_TLS, STV_DEFAULT, STV_HIDDEN,
    STV_INTERNAL, STV_PROTECTED,
};
use crate::backend::x86::assembler::parser::*;
use crate::common::fx_hash::FxHashMap;

// ─── Architecture trait ───────────────────────────────────────────────

/// Architecture-specific behavior for x86-family ELF writers.
///
/// Implemented by x86-64 and i686 to provide relocation types,
/// ELF constants, and instruction encoding.
pub trait X86Arch {
    /// Encode an instruction, returning (bytes, relocations, optional jump info).
    /// The `section_data_len` parameter is the current offset in the section.
    fn encode_instruction(
        instr: &Instruction,
        section_data_len: u64,
    ) -> Result<EncodeResult, String>;

    /// ELF machine type (EM_X86_64 or EM_386).
    fn elf_machine() -> u16;
    /// ELF class (ELFCLASS64 or ELFCLASS32).
    fn elf_class() -> u8;
    /// ELF flags (typically 0 for both).
    fn elf_flags() -> u32 {
        0
    }

    /// Absolute relocation type for data (R_X86_64_32/R_X86_64_64 or R_386_32).
    fn reloc_abs(size: usize) -> u32;
    /// 64-bit absolute relocation (R_X86_64_64). Only meaningful for x86-64.
    fn reloc_abs64() -> u32;
    /// PC-relative relocation type (R_X86_64_PC32 or R_386_PC32).
    fn reloc_pc32() -> u32;

    /// 64-bit PC-relative relocation type (S + A - P, 8-byte patch).
    /// GAS emits this for `.quad sym - .`; the kernel's __jump_table and
    /// static_call sites rely on the full-width delta: patching an 8-byte
    /// slot with a 4-byte PC32 leaves the upper half zero instead of
    /// sign-extending, corrupting the address whenever the delta is negative.
    /// Architectures without a PC64 type (ELF32) fall back to PC32.
    fn reloc_pc64() -> u32 {
        Self::reloc_pc32()
    }
    /// PLT relocation type (R_X86_64_PLT32 or R_386_PLT32).
    fn reloc_plt32() -> u32;

    /// Whether this architecture uses REL format (i686) vs RELA (x86-64).
    /// When true, addends are patched into section data instead of being
    /// stored in the relocation entry.
    fn uses_rel_format() -> bool;

    /// Optional: PC8 internal relocation type for loop/jrcxz instructions.
    /// Only x86-64 has this; i686 returns None.
    fn reloc_pc8_internal() -> Option<u32> {
        None
    }
    /// 16-bit PC-relative type (R_386_PC16 / R_X86_64_PC16) for rel16
    /// branch fields in real-mode (.code16) code. None = never emitted.
    fn reloc_pc16() -> Option<u32> {
        None
    }
    /// Real 8-bit PC-relative ELF type (R_X86_64_PC8=15 / R_386_PC8=23) for
    /// cross-section `.byte a - b` (header.S short-jump displacement).
    fn reloc_pc8() -> Option<u32> {
        None
    }
    /// Patch width for a relocation type: 16-bit reloc types own 2-byte
    /// fields; writing 4 bytes would clobber the neighbors.
    fn reloc_patch_size(_reloc_type: u32) -> u8 {
        4
    }

    /// Optional: absolute 32-bit relocation for local symbol references.
    /// Only x86-64 uses R_X86_64_32 this way; i686 returns None since
    /// its R_386_32 is handled by the general abs path.
    fn reloc_abs32_for_internal() -> Option<u32> {
        None
    }

    /// Whether `.skip` expressions with label arithmetic are supported.
    /// Both x86-64 and i686 enable this for the Linux kernel's ALTERNATIVES macros.
    fn supports_deferred_skips() -> bool {
        false
    }

    /// Whether `.set` alias resolution for label-difference expressions
    /// should be done during data value emission. Both x86-64 and i686
    /// enable this for DWARF debug info `.set .Lset0, .LECIE-.LSCIE` patterns.
    fn resolve_set_aliases_in_data() -> bool {
        false
    }

    /// Default code mode for this architecture (64 for x86-64, 32 for i686).
    fn default_code_mode() -> u8 {
        64
    }

    /// Encode an instruction in 64-bit mode. Used by the i686 assembler when
    /// encountering `.code64` sections (e.g. kernel realmode trampoline code).
    /// Default implementation delegates to the normal encode_instruction.
    fn encode_instruction_code64(
        instr: &Instruction,
        section_data_len: u64,
    ) -> Result<EncodeResult, String> {
        Self::encode_instruction(instr, section_data_len)
    }

    /// Encode an instruction in 32-bit mode. Used by the x86-64 assembler when
    /// it encounters `.code32` (mirror of `encode_instruction_code64`). The
    /// kernel's EFI mixed-mode and realmode entry code is 32-bit assembly
    /// living inside a 64-bit object; without this the 64-bit encoder rejects
    /// legal `pushl %ecx` / `popl %ecx`.
    /// Encode an instruction in 16-bit (real) mode.
    ///
    /// Only the i686 backend implements this; the default keeps the normal
    /// encoder so other architectures are unaffected.
    fn encode_instruction_code16(
        instr: &Instruction,
        section_data_len: u64,
    ) -> Result<EncodeResult, String> {
        Self::encode_instruction(instr, section_data_len)
    }

    /// Encode an instruction in `.code16gcc` mode: 16-bit real mode where
    /// UNSUFFIXED call/ret take 32-bit operands (GCC's -m16 ABI; the boot
    /// hand asm uses calll/retl and esp-relative reads that expect 4-byte
    /// return slots). Default falls back to the plain code16 encoder; the
    /// i686 backend overrides it with the true GCC semantics.
    fn encode_instruction_code16_gcc(
        instr: &Instruction,
        section_data_len: u64,
    ) -> Result<EncodeResult, String> {
        Self::encode_instruction_code16(instr, section_data_len)
    }

    fn encode_instruction_code32(
        instr: &Instruction,
        section_data_len: u64,
    ) -> Result<EncodeResult, String> {
        Self::encode_instruction(instr, section_data_len)
    }
}

/// Result of encoding a single instruction.
pub struct EncodeResult {
    pub bytes: Vec<u8>,
    pub relocations: Vec<EncoderReloc>,
    pub jump: Option<JumpDetection>,
}

/// A relocation produced by the instruction encoder.
pub struct EncoderReloc {
    pub offset: u64,
    pub symbol: String,
    pub reloc_type: u32,
    pub addend: i64,
    pub diff_symbol: Option<String>,
}

/// Jump instruction detected during encoding, eligible for relaxation.
pub struct JumpDetection {
    pub is_conditional: bool,
    /// Whether this is already in short form (e.g., jecxz, loop).
    pub already_short: bool,
}

// ─── Internal types ───────────────────────────────────────────────────

/// Tracks a jump instruction for relaxation (long <-> short).
#[derive(Clone, Debug)]
struct JumpInfo {
    offset: usize,
    /// Current encoded length (2 after relaxation).
    len: usize,
    /// Architecture/mode-specific near form length.  This is 6/5 bytes for
    /// jcc/jmp with rel32, but 4/3 bytes in `.code16` with rel16.  Retaining
    /// it is essential: an optimistically shortened branch may later need to
    /// grow after alignment reaches its fixed point.
    long_len: usize,
    target: String,
    is_conditional: bool,
    /// Whether the jump currently uses its short form.
    relaxed: bool,
    /// Whether this writer shortened the jump and can restore its long form.
    /// Short-only instructions remain false.
    can_grow: bool,
}

/// Tracks an alignment or .org marker within a section.
/// Used to recalculate padding after jump relaxation.
#[derive(Clone, Debug)]
struct AlignMarker {
    offset: usize,
    padding: usize,
    kind: AlignMarkerKind,
    /// Whether the item immediately preceding this padding was a complete
    /// instruction. When it was not (a `.byte`, a lone prefix, raw data),
    /// GAS emits one plain `0x90` before the long-NOP run; we match that so
    /// padding is byte-identical and equally decoder-safe.
    after_insn: bool,
}

#[derive(Clone, Debug)]
enum AlignMarkerKind {
    /// .balign N — pad to N-byte boundary.
    Align(u32),
    /// .org label + offset — advance to a fixed position.
    Org { label: String, addend: i64 },
}

/// A pending `(a - b) * scale + addend` datum, folded once both labels are
/// placed.
struct ScaledDiff {
    sec_idx: usize,
    offset: usize,
    a: String,
    b: String,
    scale: i64,
    /// Divisor applied after scaling (div >= 1).
    div: i64,
    addend: i64,
    size: usize,
}

/// A section being built during assembly.
struct Section {
    name: String,
    section_type: u32,
    flags: u64,
    data: Vec<u8>,
    alignment: u64,
    relocations: Vec<ElfRelocation>,
    jumps: Vec<JumpInfo>,
    align_markers: Vec<AlignMarker>,
    comdat_group: Option<String>,
}

#[derive(Clone)]
struct ElfRelocation {
    offset: u64,
    symbol: String,
    reloc_type: u32,
    addend: i64,
    diff_symbol: Option<String>,
    /// Size of the data to patch (1, 2, 4, or 8 bytes).
    patch_size: u8,
}

/// Symbol info collected during assembly.
struct SymbolInfo {
    name: String,
    binding: u8,
    sym_type: u8,
    visibility: u8,
    section: Option<String>,
    value: u64,
    size: u64,
    is_common: bool,
    common_align: u32,
}

// ─── Executable padding (multi-byte NOPs) ─────────────────────────────

/// Long-NOP patterns for 64-bit code, indexed by (length - 1).
///
/// These are the canonical `0F 1F /0` multi-byte NOP forms recommended by
/// both the Intel SDM (Vol. 2B, "NOP") and the AMD optimization guides, and
/// are exactly the patterns GNU as emits for `.p2align` in executable
/// sections (`alt_patt` in gas/config/tc-i386.c).
///
/// WHY THIS MATTERS FOR CODE QUALITY, not just byte-exactness: padding a
/// 15-byte gap with fifteen `0x90` bytes makes the front end decode, allocate
/// and retire *fifteen* uops. One `nopw %cs:0(%rax,%rax,1)` plus one shorter
/// NOP is two uops for the same 15 bytes. On Raptor Lake a long NOP retires
/// as a single uop, so the long forms cost ~7x less front-end bandwidth.
/// PGO block alignment deliberately inserts padding on *hot* paths — often
/// immediately before a loop header executed millions of times — so
/// single-byte fill burns issue slots in the hottest code in the program.
const NOP_PATTERNS: [&[u8]; 11] = [
    &[0x90],                                                       // nop
    &[0x66, 0x90],                                                 // xchg %ax,%ax
    &[0x0F, 0x1F, 0x00],                                           // nopl (%rax)
    &[0x0F, 0x1F, 0x40, 0x00],                                     // nopl 0(%rax)
    &[0x0F, 0x1F, 0x44, 0x00, 0x00],                               // nopl 0(%rax,%rax,1)
    &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],                         // nopw 0(%rax,%rax,1)
    &[0x0F, 0x1F, 0x80, 0x00, 0x00, 0x00, 0x00],                   // nopl 0L(%rax)
    &[0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00],             // nopl 0L(%rax,%rax,1)
    &[0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00],       // nopw 0L(%rax,%rax,1)
    &[0x66, 0x2E, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00], // nopw %cs:0L(...)
    &[
        0x66, 0x66, 0x2E, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
    ],
];

/// The largest single NOP we will emit (matches GAS's `alt_patt` limit).
const MAX_NOP: usize = NOP_PATTERNS.len();

/// Above this many maximum-size NOPs, jumping over the padding is cheaper
/// than executing it. Matches GAS's `max_number_of_nops` empirically: with
/// the 11-byte long-NOP table, GNU as 2.4x emits a jump only once the gap
/// exceeds EIGHT max-size NOPs (count/11 > 8 — an 88-byte pad is still all
/// NOPs, 99 bytes becomes `jmp` + NOPs), where a predicted-taken branch
/// clearly beats decoding more NOP µops.
const MAX_NOP_RUN: usize = 7; // GAS 2.47: jump-over at count/11 > 7 (2.44 used > 8)

/// Build `count` bytes of executable padding.
///
/// The layout mirrors GAS byte-exactly (asmdiff.py oracle): a run of
/// maximum-size NOPs FIRST, then one remainder NOP sized `count % MAX_NOP`
/// LAST — GNU as's i386_output_nops fills from the largest pattern down and
/// places the odd-size tail at the end. (The previous remainder-FIRST order
/// produced identical total length but 270/746 byte-differential failures
/// across the branch/padding corpora, masking real layout regressions.)
/// When the run would exceed `MAX_NOP_RUN` NOPs the whole gap is skipped
/// with a single branch instead, so a large alignment gap costs one
/// predicted-taken jump rather than dozens of decoded NOPs.
pub(crate) fn exec_padding(count: usize, after_insn: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(count);
    if count == 0 {
        return out;
    }
    let mut count = count;

    // GAS's `last_insn_normal` rule: when the preceding bytes were not a
    // complete instruction, lead with a single-byte NOP so a decoder that
    // walked in from data does not mis-parse the multi-byte NOP.
    if !after_insn {
        out.push(0x90);
        count -= 1;
        if count == 0 {
            return out;
        }
    }

    if count / MAX_NOP > MAX_NOP_RUN {
        let disp = count - 2;
        if disp <= 0x7F {
            out.push(0xEB);
            out.push(disp as u8);
            count = disp;
        } else {
            let rel = count - 5;
            out.push(0xE9);
            out.extend_from_slice(&(rel as u32).to_le_bytes());
            count = rel;
        }
    }

    // GAS 2.47 NOP layout (binutils flipped this relative to 2.44): the
    // REMAINDER-size NOP comes FIRST, followed by the run of max-size
    // (11-byte `66 66 2e 0f 1f 84 00 ...`) NOPs. Byte-count and
    // instruction-count are identical either way — this is pure oracle
    // parity with the only binutils generation we certify against (2.47).
    if count % MAX_NOP != 0 {
        out.extend_from_slice(NOP_PATTERNS[count % MAX_NOP - 1]);
        count -= count % MAX_NOP;
    }
    while count >= MAX_NOP {
        out.extend_from_slice(NOP_PATTERNS[MAX_NOP - 1]);
        count -= MAX_NOP;
    }
    out
}

/// Parse a `.skip`/`.space` size that is a self-contained constant.
///
/// Returns `None` for anything referencing a symbol, so genuinely
/// forward-referencing sizes keep the deferred-resolution path.
pub(crate) fn parse_const_skip(expr: &str) -> Option<usize> {
    let s = expr.trim();
    if s.is_empty() {
        return None;
    }
    let v = if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).ok()?
    } else {
        s.parse::<i64>().ok()?
    };
    Some(if v < 0 { 0 } else { v as usize })
}

/// Padding bytes for a section: multi-byte NOPs when executable, zeros
/// otherwise.
pub(crate) fn section_padding(count: usize, is_exec: bool, after_insn: bool) -> Vec<u8> {
    if is_exec {
        exec_padding(count, after_insn)
    } else {
        vec![0u8; count]
    }
}

// ─── Expression evaluator ─────────────────────────────────────────────

/// Token in a deferred expression (for `.skip` with label arithmetic).
#[derive(Debug, Clone, PartialEq)]
enum ExprToken {
    Number(i64),
    Symbol(String),
    Plus,
    Minus,
    Star,
    LParen,
    RParen,
    Lt,
    Gt,
    And,
    Or,
    Xor,
    Not,
}

fn tokenize_expr(expr: &str) -> Result<Vec<ExprToken>, String> {
    let mut tokens = Vec::new();
    let bytes = expr.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' => {
                i += 1;
            }
            b'+' => {
                tokens.push(ExprToken::Plus);
                i += 1;
            }
            b'-' => {
                tokens.push(ExprToken::Minus);
                i += 1;
            }
            b'*' => {
                tokens.push(ExprToken::Star);
                i += 1;
            }
            b'(' => {
                tokens.push(ExprToken::LParen);
                i += 1;
            }
            b')' => {
                tokens.push(ExprToken::RParen);
                i += 1;
            }
            b'<' => {
                tokens.push(ExprToken::Lt);
                i += 1;
            }
            b'>' => {
                tokens.push(ExprToken::Gt);
                i += 1;
            }
            b'&' => {
                tokens.push(ExprToken::And);
                i += 1;
            }
            b'|' => {
                tokens.push(ExprToken::Or);
                i += 1;
            }
            b'^' => {
                tokens.push(ExprToken::Xor);
                i += 1;
            }
            b'~' => {
                tokens.push(ExprToken::Not);
                i += 1;
            }
            b'0'..=b'9' => {
                let start = i;
                if i + 1 < bytes.len()
                    && bytes[i] == b'0'
                    && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X')
                {
                    i += 2;
                    while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                        i += 1;
                    }
                } else {
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                // Check for numeric label references: digits followed by 'b' or 'f'
                // (e.g., "0b" = backward ref to label 0, "1f" = forward ref to label 1)
                if i < bytes.len()
                    && (bytes[i] == b'b' || bytes[i] == b'f')
                    && (i + 1 >= bytes.len() || !bytes[i + 1].is_ascii_alphanumeric())
                {
                    i += 1; // include the 'b' or 'f' suffix
                    tokens.push(ExprToken::Symbol(expr[start..i].to_string()));
                } else {
                    let num_str = &expr[start..i];
                    let val = if num_str.starts_with("0x") || num_str.starts_with("0X") {
                        i64::from_str_radix(&num_str[2..], 16)
                            .map_err(|_| format!("bad hex number: {}", num_str))?
                    } else {
                        num_str
                            .parse::<i64>()
                            .map_err(|_| format!("bad number: {}", num_str))?
                    };
                    tokens.push(ExprToken::Number(val));
                }
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'.' => {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
                {
                    i += 1;
                }
                tokens.push(ExprToken::Symbol(expr[start..i].to_string()));
            }
            c => {
                return Err(format!(
                    "unexpected character in expression: '{}' (0x{:02x})",
                    c as char, c
                ))
            }
        }
    }

    Ok(tokens)
}

// ─── Core ELF writer ──────────────────────────────────────────────────

/// Shared ELF writer for x86-family architectures.
///
/// Contains all the common logic for building ELF relocatable objects
/// from parsed assembly items. Architecture-specific behavior is
/// provided through the `X86Arch` trait parameter.
/// A `.skip` whose size is not a compile-time constant at emission time
/// (it references labels that are not yet resolved), so its fill bytes are
/// spliced in later by `resolve_deferred_skips`.
#[derive(Clone, Debug)]
pub struct DeferredSkip {
    /// Section the `.skip` was emitted into.
    pub sec_idx: usize,
    /// Section offset the gap starts at (== section length at emission time).
    pub offset: usize,
    /// Unresolved size expression.
    pub expr: String,
    /// Fill byte (the `.skip size, fill` second operand; 0 when omitted).
    pub fill: u8,
    /// Labels already sitting at exactly `offset` when the `.skip` was seen.
    ///
    /// A label defined immediately BEFORE the skip and one defined
    /// immediately AFTER it both record the very same offset, because the
    /// gap has no length yet. The two must be treated differently: the
    /// fill bytes are inserted *after* the former and *before* the latter.
    /// Without this snapshot the adjustment loop below (which keys on
    /// `loff >= offset`) shifts both, so a preceding label is dragged
    /// forward by the gap and any difference against it collapses —
    /// observed as the kernel's `.byte 773b-771b` (total source length of an
    /// ALTERNATIVE with an empty old instruction) assembling as 0 instead of
    /// the padded length, which objtool reports as "empty alternative entry"
    /// and which makes boot-time alternative patching a no-op.
    pub labels_at_offset: Vec<String>,
    /// Same, for numeric local labels: (label number, index into that
    /// number's definition list).
    pub numeric_at_offset: Vec<(String, usize)>,
}

pub struct ElfWriterCore<A: X86Arch> {
    sections: Vec<Section>,
    symbols: Vec<SymbolInfo>,
    section_map: FxHashMap<String, usize>,
    symbol_map: FxHashMap<String, usize>,
    current_section: Option<usize>,
    previous_section: Option<usize>,
    label_positions: FxHashMap<String, (usize, u64)>,
    numeric_label_positions: FxHashMap<String, Vec<(usize, u64)>>,
    pending_globals: Vec<String>,
    pending_weaks: Vec<String>,
    pending_types: FxHashMap<String, SymbolKind>,
    pending_sizes: FxHashMap<String, SizeExpr>,
    pending_hidden: Vec<String>,
    pending_protected: Vec<String>,
    pending_internal: Vec<String>,
    aliases: FxHashMap<String, String>,
    /// `.set NAME, expr` where expr evaluates to a CONSTANT (possibly using
    /// `.` and same-section labels, e.g. header.S
    /// `.set section_count, (. - section_table) / 40`). Relocations against
    /// these names patch the constant value directly -- GAS gives such
    /// symbols an absolute value; they never become undefined references.
    set_values: FxHashMap<String, i64>,
    /// Position-dependent `.set` expressions with forward label refs:
    /// (alias, expr with `.` already substituted, sec_idx). Re-evaluated
    /// after all labels are placed.
    pending_set_exprs: Vec<(String, String, usize)>,
    /// `.symver real, name@VER` where `real` is NOT defined in this object:
    /// GNU-as semantics — every reference to `real` becomes a versioned
    /// reference `name@VER` (glibc compat_symbol_reference; needed because
    /// GNU ld 2.47 does not bind unversioned refs to foo@VER under
    /// --whole-archive). Applied to relocations after all items are parsed.
    symver_refs: FxHashMap<String, String>,
    /// Three-operand `.symver real, name@VER, {local|hidden|remove}` records
    /// the visibility flag here (real -> (versioned name, flag)); applied to
    /// the DEFINED plain symbol at emit time (GAS 2.35+ semantics, verified
    /// against binutils: local demotes to STB_LOCAL, hidden sets STV_HIDDEN,
    /// remove drops the plain symbol and rebinds intra-object references to
    /// the versioned alias).
    symver_flags: FxHashMap<String, (String, crate::backend::x86::assembler::parser::SymverFlag)>,
    section_stack: Vec<(Option<usize>, Option<usize>)>,
    /// Deferred `.skip` expressions with symbolic sizes (see `DeferredSkip`).
    deferred_skips: Vec<DeferredSkip>,
    /// Deferred byte-sized symbol diffs: (section_index, offset, sym_a, sym_b, size, addend).
    deferred_byte_diffs: Vec<(usize, usize, String, String, usize, i64)>,
    /// Deferred ULEB/SLEB128 symbol diffs: (section, offset, sym_a, sym_b, addend, signed).
    /// GNU as folds same-section local-label differences to constants at
    /// assembly time; we defer until label positions are known (glibc .eh_frame).
    deferred_leb_diffs: Vec<(usize, usize, String, String, i64, bool)>,
    /// Current code mode (16, 32, or 64). Affects instruction encoding.
    /// Set by `.code16`, `.code32`, `.code64` directives.
    code_mode: u8,
    /// Whether the most recently emitted item in the current section was a
    /// complete instruction (as opposed to raw data). Drives the leading
    /// plain-NOP rule for executable alignment padding.
    last_item_was_insn: bool,
    /// Scaled symbol differences awaiting label resolution.
    deferred_scaled_diffs: Vec<ScaledDiff>,
    _arch: std::marker::PhantomData<A>,
}

impl<A: X86Arch> ElfWriterCore<A> {
    pub fn new() -> Self {
        ElfWriterCore {
            sections: Vec::new(),
            symbols: Vec::new(),
            section_map: FxHashMap::default(),
            symbol_map: FxHashMap::default(),
            current_section: None,
            previous_section: None,
            label_positions: FxHashMap::default(),
            numeric_label_positions: FxHashMap::default(),
            pending_globals: Vec::new(),
            pending_weaks: Vec::new(),
            pending_types: FxHashMap::default(),
            pending_sizes: FxHashMap::default(),
            pending_hidden: Vec::new(),
            pending_protected: Vec::new(),
            pending_internal: Vec::new(),
            aliases: FxHashMap::default(),
            set_values: FxHashMap::default(),
            pending_set_exprs: Vec::new(),
            symver_refs: FxHashMap::default(),
            symver_flags: FxHashMap::default(),
            section_stack: Vec::new(),
            deferred_skips: Vec::new(),
            deferred_byte_diffs: Vec::new(),
            deferred_leb_diffs: Vec::new(),
            code_mode: A::default_code_mode(),
            last_item_was_insn: false,
            deferred_scaled_diffs: Vec::new(),
            _arch: std::marker::PhantomData,
        }
    }

    /// Build the ELF object file from parsed assembly items.
    pub fn build(mut self, items: &[AsmItem]) -> Result<Vec<u8>, String> {
        let items = resolve_numeric_labels(items);
        for item in &items {
            self.process_item(item)?;
        }
        // GNU-as .symver references: if `real` was never DEFINED in this
        // object, rewrite relocations against it to the versioned name
        // (foo@VER). `@@` in a reference selects the default version ->
        // single @.
        //
        // "Defined" must include `.set`/`.equiv` aliases and absolute .set
        // symbols, not just labels: glibc's libm builds `.set
        // __ieee754_j0f,__j0f` + `.symver __ieee754_j0f,__j0f_finite@...`,
        // and treating the alias as undefined would wrongly versionize
        // intra-object references that GAS binds to the local definition.
        let symbol_is_defined = |writer: &Self, name: &String| {
            writer.label_positions.contains_key(name)
                || writer.aliases.contains_key(name)
                || writer.set_values.contains_key(name)
        };
        if !self.symver_refs.is_empty() {
            let mut rewrites: Vec<(String, String)> = Vec::new();
            for (name, ver) in &self.symver_refs {
                if !symbol_is_defined(&self, name) {
                    rewrites.push((name.clone(), ver.replace("@@", "@")));
                }
            }
            // Three-operand `remove` on a DEFINED symbol: GAS drops the
            // plain name from the symbol table and rebinds intra-object
            // references to the versioned symbol (verified against
            // binutils GAS: `call foo` relocates against foo@VERS after
            // `.symver foo, foo@VERS, remove`). The versioned name is
            // used EXACTLY as written (no @@ folding: a defined default
            // version keeps its two-@ symbol).
            for (name, (ver, flag)) in &self.symver_flags {
                if *flag == SymverFlag::Remove && symbol_is_defined(&self, name) {
                    rewrites.push((name.clone(), ver.clone()));
                }
            }
            if !rewrites.is_empty() {
                let map: FxHashMap<String, String> = rewrites.into_iter().collect();
                for sec in self.sections.iter_mut() {
                    for reloc in sec.relocations.iter_mut() {
                        if let Some(new_name) = map.get(&reloc.symbol) {
                            reloc.symbol = new_name.clone();
                        }
                    }
                }
            }
        }
        self.emit_elf()
    }

    /// GAS creates `.text`, `.data` and `.bss` at assembler startup, so even
    /// an object that only ever uses `.text` carries empty `.data`/`.bss`
    /// sections right after it. Recreate that layout on first section use so
    /// emitted objects are byte-identical to GAS (byte-exact regression
    /// diffs and tooling that expects the trio depend on it).
    fn ensure_default_sections(&mut self) {
        if !self.sections.is_empty() {
            return;
        }
        // Push the trio directly (not through get_or_create_section) to
        // avoid recursing through this hook.
        for (name, ty, fl) in [
            (".text", SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR),
            (".data", SHT_PROGBITS, SHF_ALLOC | SHF_WRITE),
            (".bss", SHT_NOBITS, SHF_ALLOC | SHF_WRITE),
        ] {
            let idx = self.sections.len();
            self.sections.push(Section {
                name: name.to_string(),
                section_type: ty,
                flags: fl,
                data: Vec::new(),
                alignment: 1,
                relocations: Vec::new(),
                jumps: Vec::new(),
                align_markers: Vec::new(),
                comdat_group: None,
            });
            self.section_map.insert(name.to_string(), idx);
        }
    }

    fn get_or_create_section(
        &mut self,
        name: &str,
        section_type: u32,
        flags: u64,
        comdat_group: Option<String>,
    ) -> Result<usize, String> {
        self.ensure_default_sections();
        if let Some(&idx) = self.section_map.get(name) {
            let existing = &self.sections[idx];
            // GNU as errors on a re-declaration with different attributes:
            // silently merging would change the meaning of already-emitted
            // content.
            if existing.flags != flags {
                return Err(format!("changed section attributes for {name}"));
            }
            if existing.section_type != section_type {
                // Well-known section names have a fixed type in GNU as: a
                // contradicting @type suffix is ignored with a warning (`.data`
                // is always PROGBITS). For custom sections the type change is
                // an error — mixing e.g. @nobits and @progbits content in one
                // section would silently drop one of them.
                if let Some(well_known) =
                    crate::backend::elf::well_known_section_type(name)
                {
                    eprintln!(
                        "warning: ignoring incorrect section type for {name} (@{} requested)",
                        if well_known == SHT_NOBITS { "nobits" } else { "progbits" }
                    );
                    return Ok(idx);
                }
                return Err(format!("changed section type for {name}"));
            }
            return Ok(idx);
        }
        // New section: for well-known names the type is fixed regardless of
        // what the directive requested (GNU as warns and keeps the known type).
        let section_type =
            crate::backend::elf::well_known_section_type(name).unwrap_or(section_type);
        let idx = self.sections.len();
        self.sections.push(Section {
            name: name.to_string(),
            section_type,
            flags,
            data: Vec::new(),
            // Section alignment starts at 1 and is raised by the `.p2align` /
            // `.align` directives the section actually contains (see the
            // AsmItem::Align arm), mirroring GNU as exactly.
            //
            // Forcing 16 for every executable section was wrong: sections made
            // with `.pushsection ...,"ax"` that carry no alignment directive
            // must stay at 1. The kernel's alternatives machinery depends on
            // it -- `.altinstr_replacement` entries are addressed by exact byte
            // offsets recorded in `.altinstructions`, so padding the section
            // start to 16 shifts every later replacement away from the offset
            // its entry points at.
            alignment: 1,
            relocations: Vec::new(),
            jumps: Vec::new(),
            align_markers: Vec::new(),
            comdat_group,
        });
        self.section_map.insert(name.to_string(), idx);
        Ok(idx)
    }

    fn current_section_mut(&mut self) -> Result<&mut Section, String> {
        let idx = self.current_section.ok_or("no active section")?;
        Ok(&mut self.sections[idx])
    }

    fn switch_section(&mut self, dir: &SectionDirective) -> Result<(), String> {
        let (section_type, flags) =
            parse_section_flags(&dir.name, dir.flags.as_deref(), dir.section_type.as_deref());
        let idx =
            self.get_or_create_section(&dir.name, section_type, flags, dir.comdat_group.clone())?;
        self.previous_section = self.current_section;
        self.current_section = Some(idx);
        Ok(())
    }

    fn process_item(&mut self, item: &AsmItem) -> Result<(), String> {
        let result = self.process_item_inner(item);
        // Maintain the "previous item was a real instruction" state that
        // executable alignment padding depends on. Labels and pure metadata
        // directives are transparent: they emit no bytes, so they must not
        // clear a preceding instruction's status.
        match item {
            AsmItem::Instruction(_) => self.last_item_was_insn = true,
            AsmItem::Label(_)
            | AsmItem::Global(_)
            | AsmItem::Weak(_)
            | AsmItem::Hidden(_)
            | AsmItem::Protected(_)
            | AsmItem::Internal(_)
            | AsmItem::SymbolType(_, _)
            | AsmItem::Size(_, _)
            | AsmItem::Cfi(_)
            | AsmItem::File(_, _)
            | AsmItem::Loc(_, _, _)
            | AsmItem::OptionDirective(_)
            | AsmItem::Symver(_, _, _)
            | AsmItem::Empty
            | AsmItem::Align(_)
            | AsmItem::Org(_, _, _) => {}
            _ => self.last_item_was_insn = false,
        }
        result
    }

    fn process_item_inner(&mut self, item: &AsmItem) -> Result<(), String> {
        match item {
            AsmItem::Section(dir) => {
                self.switch_section(dir)?;
            }
            AsmItem::PushSection(dir) => {
                self.section_stack
                    .push((self.current_section, self.previous_section));
                self.switch_section(dir)?;
            }
            AsmItem::PopSection => {
                if let Some((saved_current, saved_previous)) = self.section_stack.pop() {
                    self.current_section = saved_current;
                    self.previous_section = saved_previous;
                }
            }
            AsmItem::Previous => {
                if self.previous_section.is_some() {
                    std::mem::swap(&mut self.current_section, &mut self.previous_section);
                }
            }
            AsmItem::Global(name) => {
                // `.globl a, b` declares EVERY comma-separated name global
                // (GAS semantics). mkpiggy emits `.globl input_data,
                // input_data_end`; storing the whole string as one symbol
                // name silently made BOTH stay local and the compressed
                // vmlinux link failed with "hidden symbol `input_data'
                // isn't defined".
                for part in name.split(',') {
                    let sym = part.trim();
                    if !sym.is_empty() {
                        self.pending_globals.push(sym.to_string());
                    }
                }
            }
            AsmItem::Weak(name) => {
                self.pending_weaks.push(name.clone());
            }
            AsmItem::Hidden(name) => {
                self.pending_hidden.push(name.clone());
            }
            AsmItem::Protected(name) => {
                self.pending_protected.push(name.clone());
            }
            AsmItem::Internal(name) => {
                self.pending_internal.push(name.clone());
            }
            AsmItem::SymbolType(name, kind) => {
                self.pending_types.insert(name.clone(), *kind);
            }
            AsmItem::Size(name, expr) => {
                let resolved = match expr {
                    SizeExpr::CurrentMinusSymbol(start_sym) => {
                        if let Some(sec_idx) = self.current_section {
                            let current_off = self.sections[sec_idx].data.len() as u64;
                            let end_label = format!(".Lsize_end_{}", name);
                            self.label_positions
                                .insert(end_label.clone(), (sec_idx, current_off));
                            SizeExpr::SymbolDiff(end_label, start_sym.clone())
                        } else {
                            expr.clone()
                        }
                    }
                    other => other.clone(),
                };
                self.pending_sizes.insert(name.clone(), resolved);
            }
            AsmItem::Label(name) => {
                self.ensure_section()?;
                let sec_idx = self.current_section.unwrap();
                let offset = self.sections[sec_idx].data.len() as u64;
                self.label_positions.insert(name.clone(), (sec_idx, offset));

                if name.chars().all(|c| c.is_ascii_digit()) {
                    self.numeric_label_positions
                        .entry(name.clone())
                        .or_default()
                        .push((sec_idx, offset));
                }

                self.ensure_symbol(name, sec_idx, offset);
            }
            AsmItem::Align(n) => {
                let after_insn = self.last_item_was_insn;
                if let Some(sec_idx) = self.current_section {
                    let section = &mut self.sections[sec_idx];
                    let align = *n as u64;
                    if align > section.alignment {
                        section.alignment = align;
                    }
                    let current = section.data.len() as u64;
                    let aligned = (current + align - 1) & !(align - 1);
                    let padding = (aligned - current) as usize;
                    // Record alignment marker for post-relaxation fixup
                    if padding > 0 && align > 1 {
                        section.align_markers.push(AlignMarker {
                            offset: current as usize,
                            padding,
                            kind: AlignMarkerKind::Align(*n),
                            after_insn,
                        });
                    }
                    let is_exec = section.flags & SHF_EXECINSTR != 0;
                    section
                        .data
                        .extend_from_slice(&section_padding(padding, is_exec, after_insn));
                }
            }
            AsmItem::Byte(vals) => {
                self.emit_data_values(vals, 1)?;
            }
            AsmItem::Short(vals) => {
                self.emit_data_values(vals, 2)?;
            }
            AsmItem::Long(vals) => {
                self.emit_data_values(vals, 4)?;
            }
            AsmItem::Quad(vals) => {
                self.emit_data_values(vals, 8)?;
            }
            AsmItem::Uleb128(vals) => {
                self.emit_leb_values(vals, false)?;
            }
            AsmItem::Sleb128(vals) => {
                self.emit_leb_values(vals, true)?;
            }
            AsmItem::Zero(n) => {
                self.ensure_section()?;
                let section = self.current_section_mut()?;
                section.data.extend(std::iter::repeat_n(0u8, *n as usize));
            }
            AsmItem::Org(sym, offset, fill) => {
                self.process_org(sym, *offset, *fill)?;
            }
            AsmItem::SkipExpr(expr, fill) => {
                self.ensure_section()?;
                if A::supports_deferred_skips() {
                    let sec_idx = self.current_section.ok_or("no active section for .skip")?;
                    // CORRECTNESS: a `.skip` whose size is a plain constant
                    // must occupy its bytes IMMEDIATELY. Deferring it leaves
                    // the section cursor short, so a following `.p2align`
                    // computes padding from the wrong offset and the requested
                    // alignment is silently not achieved (observed:
                    // `.skip 11,0xcc` + `.p2align 4` produced ZERO padding
                    // instead of five bytes). Only genuinely symbolic sizes —
                    // the kernel-alternatives case deferral exists for — still
                    // need to wait for label resolution.
                    if let Some(n) = parse_const_skip(expr) {
                        let section = self.current_section_mut()?;
                        section.data.extend(std::iter::repeat_n(*fill, n));
                    } else {
                        let offset = self.sections[sec_idx].data.len();
                        // Remember which labels are already parked at this
                        // exact offset: they precede the gap and must stay put
                        // when the fill bytes are spliced in later.
                        let (labels_at_offset, numeric_at_offset) =
                            self.snapshot_labels_at(sec_idx, offset);
                        self.deferred_skips.push(DeferredSkip {
                            sec_idx,
                            offset,
                            expr: expr.clone(),
                            fill: *fill,
                            labels_at_offset,
                            numeric_at_offset,
                        });
                    }
                } else {
                    // Simple integer parse for architectures without deferred skip support
                    if let Ok(val) = expr.trim().parse::<u64>() {
                        let section = self.current_section_mut()?;
                        section
                            .data
                            .extend(std::iter::repeat_n(*fill, val as usize));
                    } else {
                        return Err(format!("unsupported .skip expression: {}", expr));
                    }
                }
            }
            AsmItem::Asciz(bytes) | AsmItem::Ascii(bytes) => {
                let section = self.current_section_mut()?;
                section.data.extend_from_slice(bytes);
            }
            AsmItem::Comm(name, size, align) => {
                let sym_idx = self.symbols.len();
                self.symbols.push(SymbolInfo {
                    name: name.clone(),
                    binding: STB_GLOBAL,
                    sym_type: STT_OBJECT,
                    visibility: STV_DEFAULT,
                    section: None,
                    value: *align as u64,
                    size: *size,
                    is_common: true,
                    common_align: *align,
                });
                self.symbol_map.insert(name.clone(), sym_idx);
            }
            AsmItem::Set(alias, target) => {
                // ONLY expressions with actual arithmetic are folded to
                // constants; a bare symbol target (`.set foo_alias, bar_fn`,
                // the alias-attribute lowering) must stay a symbolic alias —
                // folding it to a section offset redirects calls to an
                // absolute address (asm_label_alias regression, SIGSEGV).
                let is_arithmetic = target.contains(|c: char| {
                    matches!(
                        c,
                        '+' | '-' | '*' | '/' | '&' | '|' | '^' | '~' | '(' | '<' | '>'
                    )
                });
                if is_arithmetic {
                    if let Some((sec_idx, substituted)) = self.substitute_dot(target) {
                        if substituted.contains(".Ldotpos_") {
                            // `.`-relative: MUST defer to post-layout eval;
                            // jump relaxation moves everything after this
                            // point (SYM_FUNC_END sizes were 66 bytes too
                            // large when evaluated eagerly).
                            self.pending_set_exprs
                                .push((alias.clone(), substituted, sec_idx));
                            self.aliases.insert(alias.clone(), target.clone());
                        } else {
                            match self.eval_label_expr(&substituted, sec_idx) {
                                Some(val) => {
                                    self.set_values.insert(alias.clone(), val);
                                }
                                None => {
                                    // Forward label refs: retry after layout.
                                    self.pending_set_exprs.push((
                                        alias.clone(),
                                        substituted,
                                        sec_idx,
                                    ));
                                    self.aliases.insert(alias.clone(), target.clone());
                                }
                            }
                        }
                    } else {
                        self.aliases.insert(alias.clone(), target.clone());
                    }
                } else if let Ok(val) = crate::backend::asm_expr::parse_integer_expr(target) {
                    // Pure integer constant (`.set salign, 0x1000`).
                    self.set_values.insert(alias.clone(), val);
                } else {
                    self.aliases.insert(alias.clone(), target.clone());
                }
            }
            AsmItem::Symver(name, ver_string, flag) => {
                // GNU as semantics (verified against binutils 2.47):
                //   .symver real, name@@V  -> symbols `real` AND `name@@V`
                //   .symver real, name@V   -> symbols `real` AND `name@V`
                // The FULL version string (including @ / @@) is the alias
                // symbol; `name` itself is never renamed. Truncating at '@'
                // produced self-aliases for `local == symbol` cases and
                // duplicate definitions in one object; dropping the @@ alias
                // for base == name broke glibc default_symbol_version
                // (no __libc_start_main@@GLIBC_2.34 in the object).
                if ver_string.contains('@') {
                    if !ver_string.is_empty() {
                        self.aliases.insert(ver_string.clone(), name.clone());
                        // Candidate versioned REFERENCE: applied only when
                        // `name` is never defined in this object (checked in
                        // build() after all items are parsed).
                        self.symver_refs.insert(name.clone(), ver_string.clone());
                        if let Some(f) = flag {
                            self.symver_flags.insert(name.clone(), (ver_string.clone(), *f));
                        }
                    }
                }
            }
            AsmItem::Incbin { path, skip, count } => {
                let data = std::fs::read(path)
                    .map_err(|e| format!(".incbin: failed to read '{}': {}", path, e))?;
                let skip = *skip as usize;
                let data = if skip < data.len() {
                    &data[skip..]
                } else {
                    &[]
                };
                let data = match count {
                    Some(c) => {
                        let c = *c as usize;
                        if c < data.len() {
                            &data[..c]
                        } else {
                            data
                        }
                    }
                    None => data,
                };
                let section = self.current_section_mut()?;
                section.data.extend_from_slice(data);
            }
            AsmItem::Instruction(instr) => {
                self.encode_instruction(instr)?;
            }
            AsmItem::CodeMode(bits) => {
                // Code mode is global state that persists across section switches,
                // matching GNU as behavior (e.g. kernel trampoline_64.S uses
                // .code16gcc/.code32/.code64 across .text/.text32/.text64 sections).
                self.code_mode = *bits;
            }
            AsmItem::Cfi(_)
            | AsmItem::File(_, _)
            | AsmItem::Loc(_, _, _)
            | AsmItem::OptionDirective(_)
            | AsmItem::Empty => {}
        }
        Ok(())
    }

    fn process_org(&mut self, sym: &str, offset: i64, fill: u8) -> Result<(), String> {
        let after_insn = self.last_item_was_insn;
        let sec_idx = match self.current_section {
            Some(idx) => idx,
            None => return Ok(()),
        };
        let current = self.sections[sec_idx].data.len() as u64;
        let target = if sym.is_empty() {
            offset as u64
        } else if sym == "." {
            // `.` is the location counter, so `.org . + 16` advances 16 bytes
            // from wherever we currently are rather than seeking to absolute
            // offset 16.  It is not a label, so it never appears in
            // `label_positions` and previously fell through to an error.
            (current as i64 + offset) as u64
        } else if let Some(&(label_sec, label_off)) = self.label_positions.get(sym) {
            if label_sec == sec_idx {
                (label_off as i64 + offset) as u64
            } else {
                return Err(format!(".org symbol {} not in current section", sym));
            }
        } else if let Some((label_sec, label_off)) =
            self.resolve_numeric_label(sym, current, sec_idx)
        {
            if label_sec == sec_idx {
                (label_off as i64 + offset) as u64
            } else {
                return Err(format!(".org symbol {} not in current section", sym));
            }
        } else {
            return Err(format!(".org: unknown symbol {}", sym));
        };
        let padding = if target > current {
            (target - current) as usize
        } else {
            0
        };
        // Record .org marker for post-relaxation fixup (even when padding == 0,
        // because code before it may shrink during jump relaxation)
        if !sym.is_empty() {
            self.sections[sec_idx].align_markers.push(AlignMarker {
                offset: current as usize,
                padding,
                kind: AlignMarkerKind::Org {
                    label: sym.to_string(),
                    addend: offset,
                },
                after_insn,
            });
        }
        if padding > 0 {
            // `.org` pads with its fill byte, which defaults to ZERO -- even in
            // an executable section.  This differs from `.p2align`, where GAS
            // emits multi-byte NOPs so the padding stays executable; `.org` is a
            // positioning directive, not an alignment one.  Verified against
            // GAS 2.47: `nop; .org .+16; ret` -> 90 00*15 c3, while
            // `.org .+16, 0x90` fills with 0x90.
            let data = &mut self.sections[sec_idx].data;
            data.resize(data.len() + padding, fill);
        }
        Ok(())
    }

    fn ensure_section(&mut self) -> Result<(), String> {
        if self.current_section.is_none() {
            let idx = self.get_or_create_section(
                ".text",
                SHT_PROGBITS,
                SHF_ALLOC | SHF_EXECINSTR,
                None,
            )?;
            self.current_section = Some(idx);
        }
        Ok(())
    }

    fn ensure_symbol(&mut self, name: &str, sec_idx: usize, offset: u64) {
        let sec_name = self.sections[sec_idx].name.clone();

        if let Some(&sym_idx) = self.symbol_map.get(name) {
            let sym = &mut self.symbols[sym_idx];
            sym.section = Some(sec_name);
            sym.value = offset;
        } else {
            let binding = if self.pending_globals.contains(&name.to_string()) {
                STB_GLOBAL
            } else if self.pending_weaks.contains(&name.to_string()) {
                STB_WEAK
            } else {
                STB_LOCAL
            };

            let sym_type = match self.pending_types.get(name) {
                Some(SymbolKind::Function) => STT_FUNC,
                Some(SymbolKind::Object) => STT_OBJECT,
                Some(SymbolKind::TlsObject) => STT_TLS,
                Some(SymbolKind::GnuIndirectFunction) => STT_GNU_IFUNC,
                Some(SymbolKind::NoType) | None => STT_NOTYPE,
            };

            let visibility = if self.pending_hidden.contains(&name.to_string()) {
                STV_HIDDEN
            } else if self.pending_protected.contains(&name.to_string()) {
                STV_PROTECTED
            } else if self.pending_internal.contains(&name.to_string()) {
                STV_INTERNAL
            } else {
                STV_DEFAULT
            };

            let sym_idx = self.symbols.len();
            self.symbols.push(SymbolInfo {
                name: name.to_string(),
                binding,
                sym_type,
                visibility,
                section: Some(sec_name),
                value: offset,
                size: 0,
                is_common: false,
                common_align: 0,
            });
            self.symbol_map.insert(name.to_string(), sym_idx);
        }
    }

    fn emit_data_values(&mut self, vals: &[DataValue], size: usize) -> Result<(), String> {
        let sec_idx = self.current_section.ok_or("no active section")?;

        for val in vals {
            match val {
                DataValue::Integer(v) => {
                    let section = &mut self.sections[sec_idx];
                    match size {
                        1 => section.data.push(*v as u8),
                        2 => section.data.extend_from_slice(&(*v as i16).to_le_bytes()),
                        4 => section.data.extend_from_slice(&(*v as i32).to_le_bytes()),
                        _ => section.data.extend_from_slice(&v.to_le_bytes()),
                    }
                }
                DataValue::Symbol(sym) => {
                    // Resolve .set aliases for label-difference expressions (DWARF debug info)
                    if A::resolve_set_aliases_in_data() {
                        if let Some(target) = self.aliases.get(sym).cloned() {
                            if let Some(pos) = target.find('-') {
                                let a = target[..pos].trim().to_string();
                                let b = target[pos + 1..].trim().to_string();
                                let offset = self.sections[sec_idx].data.len() as u64;
                                self.sections[sec_idx].relocations.push(ElfRelocation {
                                    offset,
                                    symbol: a,
                                    reloc_type: if size <= 4 {
                                        A::reloc_pc32()
                                    } else {
                                        A::reloc_abs64()
                                    },
                                    addend: 0,
                                    diff_symbol: Some(b),
                                    patch_size: size as u8,
                                });
                                let section = &mut self.sections[sec_idx];
                                section.data.extend(std::iter::repeat_n(0, size));
                                continue;
                            }
                        }
                    }
                    let offset = self.sections[sec_idx].data.len() as u64;
                    self.sections[sec_idx].relocations.push(ElfRelocation {
                        offset,
                        symbol: sym.clone(),
                        reloc_type: A::reloc_abs(size),
                        addend: 0,
                        diff_symbol: None,
                        patch_size: size as u8,
                    });
                    let section = &mut self.sections[sec_idx];
                    section.data.extend(std::iter::repeat_n(0, size));
                }
                DataValue::SymbolOffset(sym, addend) => {
                    let offset = self.sections[sec_idx].data.len() as u64;
                    self.sections[sec_idx].relocations.push(ElfRelocation {
                        offset,
                        symbol: sym.clone(),
                        reloc_type: A::reloc_abs(size),
                        addend: *addend,
                        diff_symbol: None,
                        patch_size: size as u8,
                    });
                    let section = &mut self.sections[sec_idx];
                    section.data.extend(std::iter::repeat_n(0, size));
                }
                DataValue::SymbolDiff(a, b) => {
                    self.emit_symbol_diff(sec_idx, a, b, size, 0)?;
                }
                DataValue::SymbolDiffAddend(a, b, addend) => {
                    self.emit_symbol_diff(sec_idx, a, b, size, *addend)?;
                }
                DataValue::SymbolDiffScaled(a, b, scale, div, addend) => {
                    // `(a-b)*scale + addend`. A scaled difference cannot be
                    // expressed by any ELF relocation, so it must be folded to
                    // a constant here; that is sound precisely because a
                    // same-section label difference is link-time invariant.
                    // `.` as either operand is THIS directive's position: register a
                    // synthetic label at the current offset so the post-layout
                    // fold resolves it (header.S: `.long (section_table - .) / 8`).
                    let here = self.sections[sec_idx].data.len() as u64;
                    let dot_name = format!(".Ldot_{}_{}", sec_idx, here);
                    let a_res = if a == "." {
                        self.label_positions
                            .insert(dot_name.clone(), (sec_idx, here));
                        dot_name.clone()
                    } else {
                        self.aliases.get(a).cloned().unwrap_or_else(|| a.clone())
                    };
                    let b_res = if b == "." {
                        self.label_positions
                            .insert(dot_name.clone(), (sec_idx, here));
                        dot_name.clone()
                    } else {
                        self.aliases.get(b).cloned().unwrap_or_else(|| b.clone())
                    };
                    self.deferred_scaled_diffs.push(ScaledDiff {
                        sec_idx,
                        offset: self.sections[sec_idx].data.len(),
                        a: a_res,
                        b: b_res,
                        scale: *scale,
                        div: *div,
                        addend: *addend,
                        size,
                    });
                    let section = &mut self.sections[sec_idx];
                    section.data.extend(std::iter::repeat_n(0, size));
                }
            }
        }
        Ok(())
    }

    fn emit_symbol_diff(
        &mut self,
        sec_idx: usize,
        a: &str,
        b: &str,
        size: usize,
        addend: i64,
    ) -> Result<(), String> {
        /// `123b` / `7f` -- a GAS numeric local-label reference. Unlike a
        /// named symbol, this never becomes an ELF symbol table entry, so a
        /// relocation naming it can never be resolved by anything downstream
        /// (linker or otherwise). It must be folded to a constant here,
        /// which is sound because a same-object label difference is a
        /// link-time invariant once this object's own layout is final.
        fn is_numeric_label(s: &str) -> bool {
            let n = s.len();
            n >= 2
                && matches!(s.as_bytes()[n - 1], b'b' | b'f')
                && s[..n - 1].bytes().all(|c| c.is_ascii_digit())
        }

        let offset = self.sections[sec_idx].data.len() as u64;
        let a_resolved = self
            .aliases
            .get(a)
            .cloned()
            .unwrap_or_else(|| a.to_string());
        let b_resolved = self
            .aliases
            .get(b)
            .cloned()
            .unwrap_or_else(|| b.to_string());

        if b_resolved == "." {
            // `sym - .` means PC-relative. The relocation WIDTH must match
            // the data directive width: `.quad sym - .` is an 8-byte slot,
            // and patching it with a 4-byte PC32 leaves the upper half zero
            // AND overflows silently once the delta exceeds ±2 GiB. GAS
            // emits R_X86_64_PC64 here; the kernel's __jump_table
            // (`.quad key - .`) and static_call sites depend on it.
            self.sections[sec_idx].relocations.push(ElfRelocation {
                offset,
                symbol: a_resolved,
                reloc_type: if size == 8 {
                    A::reloc_pc64()
                } else {
                    A::reloc_pc32()
                },
                addend,
                diff_symbol: None,
                patch_size: size as u8,
            });
            let section = &mut self.sections[sec_idx];
            section.data.extend(std::iter::repeat_n(0, size));
        } else if is_numeric_label(&a_resolved) || is_numeric_label(&b_resolved) {
            // A numeric label on EITHER side forces the deferred/positional
            // path regardless of `size`: the `size <= 2` branch below only
            // covers byte/word diffs, but the kernel's la57 trampoline uses
            // exactly this shape at 4-byte width --
            //     .pushsection .data ; .long 770b + 1 - 760b ; .popsection
            // -- to record the offset of the immediate field inside a far
            // jump. Falling through to the final `else` would try to build
            // an ELF relocation with symbol name "770", which was never
            // written to the symbol table and can never be resolved: the
            // .long field silently stayed zero. Resolution happens later,
            // after jump relaxation and `.skip` sizing have settled the
            // final layout, exactly like the existing `size <= 2` path.
            let offset_usize = self.sections[sec_idx].data.len();
            self.deferred_byte_diffs.push((
                sec_idx,
                offset_usize,
                a_resolved,
                b_resolved,
                size,
                addend,
            ));
            let section = &mut self.sections[sec_idx];
            section.data.extend(std::iter::repeat_n(0, size));
        } else if size <= 2 && A::supports_deferred_skips() {
            // For byte/short-sized diffs, defer resolution until after
            // deferred skips are inserted (skip insertion shifts offsets).
            let offset_usize = self.sections[sec_idx].data.len();
            self.deferred_byte_diffs.push((
                sec_idx,
                offset_usize,
                a_resolved,
                b_resolved,
                size,
                addend,
            ));
            let section = &mut self.sections[sec_idx];
            section.data.extend(std::iter::repeat_n(0, size));
        } else {
            self.sections[sec_idx].relocations.push(ElfRelocation {
                offset,
                symbol: a_resolved,
                reloc_type: if size == 4 {
                    A::reloc_pc32()
                } else {
                    A::reloc_abs64()
                },
                addend,
                diff_symbol: Some(b_resolved),
                patch_size: size as u8,
            });
            let section = &mut self.sections[sec_idx];
            section.data.extend(std::iter::repeat_n(0, size));
        }
        Ok(())
    }

    /// Emit ULEB128/SLEB128 data values. Integer values are encoded directly;
    /// symbol differences are deferred and folded once label positions are known.
    fn emit_leb_values(&mut self, vals: &[DataValue], signed: bool) -> Result<(), String> {
        self.ensure_section()?;
        let sec_idx = self.current_section.unwrap();
        for val in vals {
            match val {
                DataValue::Integer(v) => {
                    let section = &mut self.sections[sec_idx];
                    if signed {
                        encode_sleb128(&mut section.data, *v);
                    } else {
                        encode_uleb128(&mut section.data, *v as u64);
                    }
                }
                DataValue::SymbolDiff(a, b) => {
                    self.defer_leb_diff(sec_idx, a, b, 0, signed)?;
                }
                DataValue::SymbolDiffAddend(a, b, addend) => {
                    self.defer_leb_diff(sec_idx, a, b, *addend, signed)?;
                }
                DataValue::SymbolDiffScaled(..) => {
                    return Err(
                        "scaled symbol difference in .uleb128/.sleb128 is unsupported".to_string(),
                    );
                }
                DataValue::Symbol(sym) | DataValue::SymbolOffset(sym, _) => {
                    return Err(format!(
                        "unsupported symbol reference {} in .uleb128/.sleb128",
                        sym
                    ));
                }
            }
        }
        Ok(())
    }

    fn defer_leb_diff(
        &mut self,
        sec_idx: usize,
        a: &str,
        b: &str,
        addend: i64,
        signed: bool,
    ) -> Result<(), String> {
        let a_resolved = self
            .aliases
            .get(a)
            .cloned()
            .unwrap_or_else(|| a.to_string());
        let b_resolved = self
            .aliases
            .get(b)
            .cloned()
            .unwrap_or_else(|| b.to_string());
        let offset = self.sections[sec_idx].data.len();
        if b_resolved == "." {
            return Err("symbol minus current position in .uleb128 is unsupported".to_string());
        }
        // Forward references are fine: resolution happens after all items
        // are processed and every label position is known. Reserve the
        // maximum LEB128 width (10 bytes for u64); resolve shrinks to fit.
        self.deferred_leb_diffs
            .push((sec_idx, offset, a_resolved, b_resolved, addend, signed));
        self.sections[sec_idx]
            .data
            .extend(std::iter::repeat_n(0, 10));
        Ok(())
    }

    fn encode_instruction(&mut self, instr: &Instruction) -> Result<(), String> {
        self.ensure_section()?;
        let sec_idx = self.current_section.unwrap();
        let base_offset = self.sections[sec_idx].data.len() as u64;

        // Use the appropriate encoder based on current code mode.
        // When the i686 assembler is in .code64 mode, it delegates to
        // the x86-64 encoder for 64-bit instruction encoding.
        let result = if self.code_mode == 64 && A::default_code_mode() != 64 {
            A::encode_instruction_code64(instr, base_offset)?
        } else if self.code_mode == 32 && A::default_code_mode() == 64 {
            A::encode_instruction_code32(instr, base_offset)?
        } else if self.code_mode == 17 && A::default_code_mode() != 64 {
            // `.code16gcc`: like .code16, but unsuffixed call/ret take
            // 32-bit operands (GCC's -m16 calling convention).
            A::encode_instruction_code16_gcc(instr, base_offset)?
        } else if self.code_mode == 16 && A::default_code_mode() != 64 {
            if std::env::var("LCCC_DBG16").is_ok() {
                eprintln!("[C16] dispatch code16 for {}", instr.mnemonic);
            }
            A::encode_instruction_code16(instr, base_offset)?
        } else if (self.code_mode == 16 || self.code_mode == 17)
            && A::default_code_mode() == 64
        {
            // .code16/.code16gcc inside a 64-bit TU: real-mode code must be
            // assembled via the -m16 (i686) path where operand-size
            // semantics are modeled. Encoding it with the 64-bit encoder
            // would silently produce wrong machine code, so reject loudly.
            return Err(format!(
                ".code16 in 64-bit assembly is not supported; compile real-mode \
                 code with -m16 (instruction: {})",
                instr.mnemonic
            ));
        } else {
            A::encode_instruction(instr, base_offset)?
        };
        let instr_len = result.bytes.len();
        self.sections[sec_idx].data.extend_from_slice(&result.bytes);

        // Register jump for relaxation if detected
        if let Some(jump_det) = result.jump {
            if let Some(ref label) = self.get_jump_target_label(instr) {
                if jump_det.already_short {
                    // Short-only jumps (jecxz/jcxz/loop) - already short, just need displacement patched
                    self.sections[sec_idx].jumps.push(JumpInfo {
                        offset: base_offset as usize,
                        len: instr_len,
                        long_len: instr_len,
                        target: label.clone(),
                        is_conditional: jump_det.is_conditional,
                        relaxed: true,
                        can_grow: false,
                    });
                } else if instr_len > 2 {
                    // The architecture has already identified this as a near
                    // branch. Do not re-impose x86 rel32 lengths here:
                    // `.code16` jcc/jmp are 4/3-byte rel16 instructions.
                    self.sections[sec_idx].jumps.push(JumpInfo {
                        offset: base_offset as usize,
                        len: instr_len,
                        long_len: instr_len,
                        target: label.clone(),
                        is_conditional: jump_det.is_conditional,
                        relaxed: false,
                        can_grow: false,
                    });
                }
            }
        }

        // Copy relocations
        for reloc in result.relocations {
            self.sections[sec_idx].relocations.push(ElfRelocation {
                offset: base_offset + reloc.offset,
                symbol: reloc.symbol,
                reloc_type: reloc.reloc_type,
                addend: reloc.addend,
                diff_symbol: reloc.diff_symbol,
                patch_size: A::reloc_patch_size(reloc.reloc_type),
            });
        }

        Ok(())
    }

    fn get_jump_target_label(&self, instr: &Instruction) -> Option<String> {
        let mnem = &instr.mnemonic;
        let is_jump = mnem == "jmp" || mnem == "loop" || (mnem.starts_with('j') && mnem.len() >= 2);
        if !is_jump {
            return None;
        }
        if instr.operands.len() != 1 {
            return None;
        }
        if let Operand::Label(label) = &instr.operands[0] {
            Some(label.clone())
        } else {
            None
        }
    }

    // ─── Deferred skip resolution (x86-64 and i686) ──────────────────

    /// Snapshot the labels positioned at exactly `offset` in `sec_idx`.
    ///
    /// Taken when a `.skip` with a symbolic size is deferred. At that moment
    /// the gap has no length, so a label defined immediately before the
    /// `.skip` and one defined immediately after it share the same recorded
    /// offset and are otherwise indistinguishable later on.
    fn snapshot_labels_at(
        &self,
        sec_idx: usize,
        offset: usize,
    ) -> (Vec<String>, Vec<(String, usize)>) {
        let off = offset as u64;
        let mut plain = Vec::new();
        for (name, &(lsec, loff)) in self.label_positions.iter() {
            if lsec == sec_idx && loff == off {
                plain.push(name.clone());
            }
        }
        let mut numeric = Vec::new();
        for (num, positions) in self.numeric_label_positions.iter() {
            for (i, &(lsec, loff)) in positions.iter().enumerate() {
                if lsec == sec_idx && loff == off {
                    numeric.push((num.clone(), i));
                }
            }
        }
        (plain, numeric)
    }

    fn resolve_deferred_skips(&mut self) -> Result<(), String> {
        let mut skips = std::mem::take(&mut self.deferred_skips);
        // Ties (two deferred skips at the SAME offset) must be spliced in
        // REVERSE source order. Each splice inserts at `offset`, pushing
        // already-inserted bytes to the right, so processing them in source
        // order would put the second gap's fill BEFORE the first gap's fill
        // and silently permute the section contents (observed as
        // `cc 90 90 90` where GNU as emits `90 90 90 cc`). `sort_by` is
        // stable, so reversing the VECTOR (rather than negating the
        // comparator, which leaves ties in source order) is what yields
        // descending offsets with reversed ties.
        skips.sort_by(|a, b| a.sec_idx.cmp(&b.sec_idx).then(a.offset.cmp(&b.offset)));
        skips.reverse();

        for skip in &skips {
            // Destructure by reference: `skips` is detached from `self`, so
            // the mutable borrows of `self` below are sound.
            let DeferredSkip {
                sec_idx,
                offset,
                expr,
                fill,
                labels_at_offset,
                numeric_at_offset,
            } = skip;
            // Temporarily insert "." (current position) into label_positions so
            // expressions like "0b + 16 - ." can reference the directive's offset.
            self.label_positions
                .insert(".".to_string(), (*sec_idx, *offset as u64));
            // Pre-resolve numeric label references (e.g. "0b", "1f") in the expression
            let resolved_expr = self.resolve_numeric_labels_in_expr(expr, *offset as u64, *sec_idx);
            let val = self.evaluate_expr(&resolved_expr);
            self.label_positions.remove(".");
            let val = val?;
            let count = if val < 0 { 0usize } else { val as usize };
            if count == 0 {
                continue;
            }

            let fill_bytes: Vec<u8> = vec![*fill; count];
            self.sections[*sec_idx]
                .data
                .splice(*offset..*offset, fill_bytes);

            // Adjust label positions.
            //
            // A label moves iff it lies strictly after the gap, or sits
            // exactly at the gap start but was defined *after* the `.skip`.
            // The second clause is what distinguishes the two kinds of
            // same-offset label: `labels_at_offset` holds the ones that were
            // already there when the skip was deferred, and those must stay
            // put. Relocations, jumps, alignment markers and deferred diffs
            // below keep `>=` because they are always emitted by items that
            // follow the `.skip` in the stream.
            for (name, (lsec, loff)) in self.label_positions.iter_mut() {
                if *lsec != *sec_idx {
                    continue;
                }
                let at_gap = (*loff as usize) == *offset;
                let after = (*loff as usize) > *offset
                    || (at_gap && !labels_at_offset.iter().any(|n| n == name));
                if after {
                    *loff += count as u64;
                }
            }
            for (num, positions) in self.numeric_label_positions.iter_mut() {
                for (i, (lsec, loff)) in positions.iter_mut().enumerate() {
                    if *lsec != *sec_idx {
                        continue;
                    }
                    let at_gap = (*loff as usize) == *offset;
                    let preceded = numeric_at_offset
                        .iter()
                        .any(|(n, idx)| n == num && *idx == i);
                    let after = (*loff as usize) > *offset || (at_gap && !preceded);
                    if after {
                        *loff += count as u64;
                    }
                }
            }
            for reloc in self.sections[*sec_idx].relocations.iter_mut() {
                if (reloc.offset as usize) >= *offset {
                    reloc.offset += count as u64;
                }
            }
            for jump in self.sections[*sec_idx].jumps.iter_mut() {
                if jump.offset >= *offset {
                    jump.offset += count;
                }
            }
            // Alignment/org markers move with everything else. Omitting them
            // left each `.p2align` after a resolved `.skip` recorded at its
            // pre-insertion offset, so the padding was computed for the wrong
            // position: the kernel's `vc_do_mmio` landed at 0x2043 where GAS
            // puts it at 0x2050, and objtool rejected the object with
            // "can't find starting instruction".
            for marker in self.sections[*sec_idx].align_markers.iter_mut() {
                if marker.offset >= *offset {
                    marker.offset += count;
                }
            }
            for (bsec, boff, _, _, _, _) in self.deferred_byte_diffs.iter_mut() {
                if *bsec == *sec_idx && *boff >= *offset {
                    *boff += count;
                }
            }
            for (bsec, boff, _, _, _, _) in self.deferred_leb_diffs.iter_mut() {
                if *bsec == *sec_idx && *boff >= *offset {
                    *boff += count;
                }
            }
        }
        Ok(())
    }

    /// Shift offsets >= `from` in section `sec_idx` by `-delta` (used when
    /// LEB128 placeholders shrink). Keeps labels, numeric labels, relocations,
    /// jumps, alignment markers and deferred skips consistent after splicing.
    fn shift_section_offsets(&mut self, sec_idx: usize, from: usize, delta: u64) {
        for (_, (lsec, loff)) in self.label_positions.iter_mut() {
            if *lsec == sec_idx && (*loff as usize) >= from {
                *loff -= delta;
            }
        }
        for (_, positions) in self.numeric_label_positions.iter_mut() {
            for (lsec, loff) in positions.iter_mut() {
                if *lsec == sec_idx && (*loff as usize) >= from {
                    *loff -= delta;
                }
            }
        }
        for reloc in self.sections[sec_idx].relocations.iter_mut() {
            if (reloc.offset as usize) >= from {
                reloc.offset -= delta;
            }
        }
        for jump in self.sections[sec_idx].jumps.iter_mut() {
            if jump.offset >= from {
                jump.offset -= delta as usize;
            }
        }
        for marker in self.sections[sec_idx].align_markers.iter_mut() {
            if marker.offset >= from {
                marker.offset -= delta as usize;
            }
        }
        for skip in self.deferred_skips.iter_mut() {
            if skip.sec_idx == sec_idx && skip.offset >= from {
                skip.offset -= delta as usize;
            }
        }
    }

    fn resolve_deferred_byte_diffs(&mut self) -> Result<(), String> {
        // Fold deferred LEB128 diffs once label positions are known.
        // Resolve from the END of each section backwards so shrinking the
        // 10-byte placeholder does not invalidate earlier offsets.
        let mut leb_diffs = std::mem::take(&mut self.deferred_leb_diffs);
        leb_diffs.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        let mut diffs = std::mem::take(&mut self.deferred_byte_diffs);
        for (sec_idx, offset, sym_a, sym_b, addend, signed) in &leb_diffs {
            let pos_a = self
                .label_positions
                .get(sym_a)
                .ok_or_else(|| format!("undefined label in .uleb128 diff: {}", sym_a))?;
            let pos_b = self
                .label_positions
                .get(sym_b)
                .ok_or_else(|| format!("undefined label in .uleb128 diff: {}", sym_b))?;
            let diff = (pos_a.1 as i64) - (pos_b.1 as i64) + addend;
            let mut encoded: Vec<u8> = Vec::new();
            if *signed {
                encode_sleb128(&mut encoded, diff);
            } else {
                encode_uleb128(&mut encoded, diff as u64);
            }
            const PLACE: usize = 10;
            if *offset + PLACE > self.sections[*sec_idx].data.len() {
                return Err("internal: .uleb128 diff placeholder overflow".to_string());
            }
            self.sections[*sec_idx]
                .data
                .splice(*offset..*offset + PLACE, encoded.iter().copied());
            let delta = PLACE - encoded.len();
            if delta > 0 {
                let from = *offset + encoded.len();
                self.shift_section_offsets(*sec_idx, from, delta as u64);
                for (dsec, doff, _, _, _, _) in diffs.iter_mut() {
                    if *dsec == *sec_idx && *doff >= from {
                        *doff -= delta;
                    }
                }
            }
        }
        // NOTE: `diffs` was already taken above (its offsets are updated while
        // the LEB placeholders shrink). Taking again here yields an EMPTY
        // vector and silently drops every deferred byte/word symbol
        // difference, leaving zeros in the data — which is exactly what
        // `.byte b-a` and `.word b-a` produced before this fix.
        for (sec_idx, offset, sym_a, sym_b, size, addend) in &diffs {
            // NUMERIC labels (`770b`, `1f`) must go through the direction-
            // aware resolver: `label_positions` only holds NAMED labels, and
            // a numeric one is ambiguous without a reference point (the same
            // digits can be reused many times in one file). A plain lookup
            // failed outright on
            //     .word 770b + 1 - 760b
            // -- the shape the kernel's la57 trampoline uses to locate the
            // immediate field inside a far jump ("undefined label in .byte
            // diff: 770b"). `resolve_numeric_label` handles the common,
            // same-section case; the loop below is the fallback for a
            // numeric label defined in a DIFFERENT section than the one
            // holding the diff directive itself -- `.pushsection .data`
            // puts the `.word` in `.data` while `770b`/`760b` stay back in
            // `.text`. There is no positional reference to compare against
            // across sections, so this prefers the LAST definition for a
            // `b`-suffixed reference and the FIRST for an `f`-suffixed one,
            // on the assumption (true for every kernel shape seen so far)
            // that a `.pushsection` data block referencing back into `.text`
            // is emitted lexically after the code it describes.
            let resolve = |me: &Self, sym: &String| -> Option<(usize, u64)> {
                if let Some(p) = me.label_positions.get(sym).copied() {
                    return Some(p);
                }
                if let Some(p) = me.resolve_numeric_label(sym, *offset as u64, *sec_idx) {
                    return Some(p);
                }
                let len = sym.len();
                if len >= 2 {
                    let suffix = sym.as_bytes()[len - 1];
                    let num = &sym[..len - 1];
                    if (suffix == b'b' || suffix == b'f') && num.bytes().all(|c| c.is_ascii_digit())
                    {
                        if let Some(list) = me.numeric_label_positions.get(num) {
                            return if suffix == b'b' {
                                list.iter().copied().max_by_key(|&(_, off)| off)
                            } else {
                                list.iter().copied().min_by_key(|&(_, off)| off)
                            };
                        }
                    }
                }
                None
            };
            let pos_a = resolve(self, sym_a)
                .ok_or_else(|| format!("undefined label in .byte diff: {}", sym_a))?;
            let pos_b = resolve(self, sym_b)
                .ok_or_else(|| format!("undefined label in .byte diff: {}", sym_b))?;
            let (pos_a, pos_b) = (&pos_a, &pos_b);

            if pos_a.0 != pos_b.0 {
                // Cross-section byte diff (header.S: `.byte start_of_setup-1f`,
                // start_of_setup in .entrytext, 1f in .header). GAS emits a
                // PC8 relocation: S + A - P with P = the byte's own offset,
                // so A = addend + byte_off - b_off computes a - b when the
                // subtrahend is in the byte's OWN section (its distance to
                // the byte is a link-time constant).
                if *size == 1 && pos_b.0 == *sec_idx {
                    if let Some(pc8) = A::reloc_pc8() {
                        let addend = *addend + (*offset as i64) - (pos_b.1 as i64);
                        self.sections[*sec_idx].relocations.push(ElfRelocation {
                            offset: *offset as u64,
                            symbol: sym_a.clone(),
                            reloc_type: pc8,
                            addend,
                            diff_symbol: None,
                            patch_size: 1,
                        });
                        continue;
                    }
                }
                return Err(format!("cross-section .byte diff: {} - {}", sym_a, sym_b));
            }

            let diff = (pos_a.1 as i64) - (pos_b.1 as i64) + addend;
            match size {
                1 => {
                    self.sections[*sec_idx].data[*offset] = diff as u8;
                }
                2 => {
                    let bytes = (diff as i16).to_le_bytes();
                    self.sections[*sec_idx].data[*offset] = bytes[0];
                    self.sections[*sec_idx].data[*offset + 1] = bytes[1];
                }
                4 => {
                    let bytes = (diff as i32).to_le_bytes();
                    for (k, b) in bytes.iter().enumerate() {
                        self.sections[*sec_idx].data[*offset + k] = *b;
                    }
                }
                8 => {
                    let bytes = diff.to_le_bytes();
                    for (k, b) in bytes.iter().enumerate() {
                        self.sections[*sec_idx].data[*offset + k] = *b;
                    }
                }
                _ => unreachable!(),
            }
        }
        Ok(())
    }

    /// Pre-resolve numeric label references (e.g. `0b`, `1f`) in an expression string.
    ///
    /// GNU as numeric labels like `0:` can be referenced as `0b` (backward) or `0f`
    /// (forward). The expression tokenizer doesn't handle these, so we substitute
    /// them with their resolved byte offsets before evaluation.
    fn resolve_numeric_labels_in_expr(&self, expr: &str, offset: u64, sec_idx: usize) -> String {
        let bytes = expr.as_bytes();
        let mut result = String::with_capacity(expr.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i].is_ascii_digit() {
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i < bytes.len()
                    && (bytes[i] == b'b' || bytes[i] == b'f')
                    && (i + 1 >= bytes.len() || !bytes[i + 1].is_ascii_alphanumeric())
                {
                    // This is a numeric label reference like "0b" or "1f"
                    let label_ref = &expr[start..=i];
                    i += 1;
                    if let Some((_, label_off)) =
                        self.resolve_numeric_label(label_ref, offset, sec_idx)
                    {
                        result.push_str(&label_off.to_string());
                    } else {
                        // Can't resolve - keep the original text (will error during eval)
                        result.push_str(label_ref);
                    }
                } else {
                    // Regular number
                    result.push_str(&expr[start..i]);
                }
            } else {
                result.push(bytes[i] as char);
                i += 1;
            }
        }
        result
    }

    // ─── Expression evaluator ─────────────────────────────────────────

    fn evaluate_expr(&self, expr: &str) -> Result<i64, String> {
        let expr = expr.trim();
        let tokens = tokenize_expr(expr)?;
        let mut pos = 0;
        let result = self.parse_expr_or(&tokens, &mut pos)?;
        if pos < tokens.len() {
            return Err(format!(
                "unexpected token in expression at position {}: {:?}",
                pos,
                tokens.get(pos)
            ));
        }
        Ok(result)
    }

    fn parse_expr_or(&self, tokens: &[ExprToken], pos: &mut usize) -> Result<i64, String> {
        let mut val = self.parse_expr_xor(tokens, pos)?;
        while *pos < tokens.len() {
            match tokens[*pos] {
                ExprToken::Or => {
                    *pos += 1;
                    val |= self.parse_expr_xor(tokens, pos)?;
                }
                _ => break,
            }
        }
        Ok(val)
    }

    fn parse_expr_xor(&self, tokens: &[ExprToken], pos: &mut usize) -> Result<i64, String> {
        let mut val = self.parse_expr_and(tokens, pos)?;
        while *pos < tokens.len() {
            match tokens[*pos] {
                ExprToken::Xor => {
                    *pos += 1;
                    val ^= self.parse_expr_and(tokens, pos)?;
                }
                _ => break,
            }
        }
        Ok(val)
    }

    fn parse_expr_and(&self, tokens: &[ExprToken], pos: &mut usize) -> Result<i64, String> {
        let mut val = self.parse_expr_cmp(tokens, pos)?;
        while *pos < tokens.len() {
            match tokens[*pos] {
                ExprToken::And => {
                    *pos += 1;
                    val &= self.parse_expr_cmp(tokens, pos)?;
                }
                _ => break,
            }
        }
        Ok(val)
    }

    fn parse_expr_cmp(&self, tokens: &[ExprToken], pos: &mut usize) -> Result<i64, String> {
        let mut val = self.parse_expr_add(tokens, pos)?;
        while *pos < tokens.len() {
            match tokens[*pos] {
                ExprToken::Lt => {
                    *pos += 1;
                    let rhs = self.parse_expr_add(tokens, pos)?;
                    val = if val < rhs { -1 } else { 0 };
                }
                ExprToken::Gt => {
                    *pos += 1;
                    let rhs = self.parse_expr_add(tokens, pos)?;
                    val = if val > rhs { -1 } else { 0 };
                }
                _ => break,
            }
        }
        Ok(val)
    }

    fn parse_expr_add(&self, tokens: &[ExprToken], pos: &mut usize) -> Result<i64, String> {
        let mut val = self.parse_expr_mul(tokens, pos)?;
        while *pos < tokens.len() {
            match tokens[*pos] {
                ExprToken::Plus => {
                    *pos += 1;
                    val = val.wrapping_add(self.parse_expr_mul(tokens, pos)?);
                }
                ExprToken::Minus => {
                    *pos += 1;
                    val = val.wrapping_sub(self.parse_expr_mul(tokens, pos)?);
                }
                _ => break,
            }
        }
        Ok(val)
    }

    fn parse_expr_mul(&self, tokens: &[ExprToken], pos: &mut usize) -> Result<i64, String> {
        let mut val = self.parse_expr_unary(tokens, pos)?;
        while *pos < tokens.len() {
            match tokens[*pos] {
                ExprToken::Star => {
                    *pos += 1;
                    val = val.wrapping_mul(self.parse_expr_unary(tokens, pos)?);
                }
                _ => break,
            }
        }
        Ok(val)
    }

    fn parse_expr_unary(&self, tokens: &[ExprToken], pos: &mut usize) -> Result<i64, String> {
        if *pos < tokens.len() {
            match tokens[*pos] {
                ExprToken::Minus => {
                    *pos += 1;
                    let val = self.parse_expr_unary(tokens, pos)?;
                    Ok(-val)
                }
                ExprToken::Plus => {
                    *pos += 1;
                    self.parse_expr_unary(tokens, pos)
                }
                ExprToken::Not => {
                    *pos += 1;
                    let val = self.parse_expr_unary(tokens, pos)?;
                    Ok(!val)
                }
                _ => self.parse_expr_primary(tokens, pos),
            }
        } else {
            Err("unexpected end of expression".to_string())
        }
    }

    fn parse_expr_primary(&self, tokens: &[ExprToken], pos: &mut usize) -> Result<i64, String> {
        if *pos >= tokens.len() {
            return Err("unexpected end of expression".to_string());
        }
        match &tokens[*pos] {
            ExprToken::Number(n) => {
                *pos += 1;
                Ok(*n)
            }
            ExprToken::Symbol(name) => {
                *pos += 1;
                if let Some(&(_, offset)) = self.label_positions.get(name.as_str()) {
                    Ok(offset as i64)
                } else {
                    Err(format!("undefined symbol in expression: {}", name))
                }
            }
            ExprToken::LParen => {
                *pos += 1;
                let val = self.parse_expr_or(tokens, pos)?;
                if *pos < tokens.len() && tokens[*pos] == ExprToken::RParen {
                    *pos += 1;
                } else {
                    return Err("missing closing parenthesis".to_string());
                }
                Ok(val)
            }
            other => Err(format!("unexpected token: {:?}", other)),
        }
    }

    /// Fold every pending `(a - b) * scale + addend` datum into its constant.
    fn resolve_scaled_diffs(&mut self) -> Result<(), String> {
        let pending = std::mem::take(&mut self.deferred_scaled_diffs);
        for d in &pending {
            let pa = self
                .label_positions
                .get(&d.a)
                .ok_or_else(|| format!("undefined label in expression: {}", d.a))?;
            let pb = self
                .label_positions
                .get(&d.b)
                .ok_or_else(|| format!("undefined label in expression: {}", d.b))?;
            if pa.0 != pb.0 {
                return Err(format!(
                    "difference of labels in different sections is not constant: {} - {}",
                    d.a, d.b
                ));
            }
            let value = (pa.1 as i64 - pb.1 as i64)
                .checked_mul(d.scale)
                .map(|v| v / d.div)
                .and_then(|v| v.checked_add(d.addend))
                .ok_or_else(|| format!("overflow folding ({} - {})", d.a, d.b))?;
            let bytes = value.to_le_bytes();
            let sec = &mut self.sections[d.sec_idx];
            if d.offset + d.size > sec.data.len() {
                return Err("internal: scaled diff placeholder overflow".to_string());
            }
            sec.data[d.offset..d.offset + d.size].copy_from_slice(&bytes[..d.size]);
        }
        Ok(())
    }

    // ─── ELF emission ─────────────────────────────────────────────────

    fn emit_elf(mut self) -> Result<Vec<u8>, String> {
        // GAS always carries the .text/.data/.bss trio, even for empty input.
        self.ensure_default_sections();
        // `.skip` sizing and jump relaxation are MUTUALLY dependent, and the
        // fixed-point they reach depends on the order. Two real kernel shapes
        // pull in opposite directions:
        //
        //  (a) sev.o: a `jmp` walks OVER two ALTERNATIVE paddings. If jumps
        //      are relaxed against the placeholder layout and the skips are
        //      sized afterwards, the already-patched displacement is stale
        //      and the branch lands mid-instruction (objtool: "can't find
        //      jump dest instruction").
        //  (b) retpoline.S FILL_RETURN_BUFFER: the padded region ITSELF
        //      contains a relaxable `jmp`. Sizing the skip first measures the
        //      long form, over-pads by a byte, and the recorded
        //      `origlen = 742b-740b` describes code that no longer exists
        //      (objtool: "weirdly overlapping alternative! 33 != 35").
        //
        // GNU as reaches the SMALLEST fixed point: relax first — while no
        // padding exists, every branch picks its shortest encoding — then
        // size the skips against that layout, re-fix alignment, and relax
        // again so any branch crossing the new padding is re-patched. Both
        // shapes are byte-identical to GAS under this order (regression:
        // alt_skip_relax_order.s covers (b), the sev shape is covered by the
        // second relax pass below).
        //
        // This first relax/fixup pass runs unconditionally, not gated on
        // `A::supports_deferred_skips()`: it is answering a question
        // ("what is the smallest fixed point for jump encodings before any
        // `.skip` padding exists?") that has nothing to do with whether this
        // architecture defers `.skip` sizing. The *second* relax_jumps()
        // call below (needed regardless, to re-patch displacements after
        // skips/alignment change layout) has always run unconditionally;
        // gating only the first call on the trait method was an
        // inconsistency, not a deliberate distinction — both concrete
        // X86Arch impls (i686, x86-64) return `true` here today, so the two
        // placements produce identical output for every currently-existing
        // backend, but the unconditional form is the one that stays correct
        // if a future arch ever overrides the default `false`.
        self.relax_jumps();
        for sec_idx in 0..self.sections.len() {
            self.fixup_alignment_markers(sec_idx);
        }
        if A::supports_deferred_skips() {
            self.resolve_deferred_skips()?;
        }

        // Apply `.p2align` / `.org` padding BEFORE relaxing jumps.
        //
        // `relax_jumps` re-runs `fixup_alignment_markers` after each size
        // change, but only for sections that contain jumps, and only once it
        // has already started moving things. Resolving the deferred `.skip`s
        // above changed section sizes, so any alignment marker after a skip is
        // stale at this point. Leaving it stale placed a function symbol at an
        // unaligned address (`vc_do_mmio` at 0x2043 where GAS puts 0x2050) and
        // objtool rejected the object with "can't find starting instruction".
        //
        // Running the fixup for every section first makes the pre-relaxation
        // layout correct; `relax_jumps` then keeps it correct as it shrinks.
        for sec_idx in 0..self.sections.len() {
            self.fixup_alignment_markers(sec_idx);
        }

        // Relax long jumps to short form where possible.
        self.relax_jumps();

        // Fix alignment/org markers for EVERY section again: relaxation only
        // maintains them for sections that actually contain jumps
        // (.eh_frame never had its `.align` padding fixed up otherwise).
        for sec_idx in 0..self.sections.len() {
            self.fixup_alignment_markers(sec_idx);
        }

        // Fold symbol differences LAST, once the layout is frozen.
        //
        // `.byte`/`.word` label differences MEASURE code that jump relaxation
        // and alignment padding can still resize. Resolving them earlier
        // records pre-shrink sizes: the kernel's ALTERNATIVE entry stored
        // `origlen = 773b-771b = 35` for a region relaxation had shortened to
        // 31, and objtool rejected the object with
        // "weirdly overlapping alternative! 33 != 35". GNU as likewise emits
        // these only after the section layout is final.
        if A::supports_deferred_skips() {
            self.resolve_deferred_byte_diffs()?;
        }

        // Scaled symbol differences: same reasoning.
        self.resolve_scaled_diffs()?;

        // Absolute .set constants (incl. forward `.`-position expressions)
        // fold before internal relocation resolution: relocs against them
        // patch constants and never reach the symbol table.
        self.resolve_set_constants();

        // Resolve internal relocations
        self.resolve_internal_relocations();

        // Convert to shared ObjSection/ObjSymbol format
        let section_names: Vec<String> = self.sections.iter().map(|s| s.name.clone()).collect();

        let mut shared_sections: FxHashMap<String, ObjSection> = FxHashMap::default();
        for sec in &self.sections {
            let mut data = sec.data.clone();
            let mut relocs = Vec::new();

            for reloc in &sec.relocations {
                // GAS converts a relocation against ANY local defined symbol
                // into section-symbol + offset — not only `.L` labels. The
                // kernel relies on this: objtool reads
                // .rela.discard.func_stack_frame_non_standard and accepts
                // STT_SECTION or STT_FUNC reloc symbols, but a local label
                // like optprobe_template_func stays STT_NOTYPE in our output
                // and objtool fails with "unexpected relocation symbol type".
                // Match GAS: fold every local defined label into its section.
                // Exception: symbols with an explicit .type directive keep
                // their own symbol table entry ONLY if the reloc is
                // PC-relative to a different section (identical to section+
                // offset anyway), so folding is always safe here.
                let is_foldable_local = self.label_positions.contains_key(&reloc.symbol)
                    && !self.pending_globals.contains(&reloc.symbol)
                    && !self.pending_weaks.contains(&reloc.symbol);
                let (sym_name, mut addend) = if is_foldable_local {
                    let &(target_sec, target_off) =
                        self.label_positions.get(&reloc.symbol).unwrap();
                    (
                        section_names[target_sec].clone(),
                        reloc.addend + target_off as i64,
                    )
                } else {
                    (reloc.symbol.clone(), reloc.addend)
                };

                // Handle symbol-difference relocations (.long a - b)
                if let Some(ref diff_sym) = reloc.diff_symbol {
                    if let Some(&(_b_sec, b_off)) = self.label_positions.get(diff_sym.as_str()) {
                        addend += reloc.offset as i64 - b_off as i64;
                    }
                }

                // For REL format (i686): patch addend into section data.
                // The patch width must match the relocation width: R_386_8 /
                // R_386_PC8 (`.byte sym` fields) own a 1-byte slot, R_386_16
                // (`.word sym` — kernel realmode segment words) a 2-byte
                // slot, and a wider write would clobber the field after it.
                if A::uses_rel_format() {
                    let off = reloc.offset as usize;
                    let patch1 = reloc.patch_size == 1;
                    let patch16 = reloc.patch_size == 2;
                    if patch1 && off + 1 <= data.len() {
                        let existing = data[off] as i8;
                        let patched = existing.wrapping_add(addend as i8);
                        data[off] = patched as u8;
                    } else if patch16 && off + 2 <= data.len() {
                        let existing = i16::from_le_bytes([data[off], data[off + 1]]);
                        let patched = existing.wrapping_add(addend as i16);
                        data[off..off + 2].copy_from_slice(&patched.to_le_bytes());
                    } else if !patch1 && !patch16 && off + 4 <= data.len() {
                        let existing = i32::from_le_bytes([
                            data[off],
                            data[off + 1],
                            data[off + 2],
                            data[off + 3],
                        ]);
                        let patched = existing.wrapping_add(addend as i32);
                        data[off..off + 4].copy_from_slice(&patched.to_le_bytes());
                    }
                    relocs.push(ObjReloc {
                        offset: reloc.offset,
                        reloc_type: reloc.reloc_type,
                        symbol_name: sym_name,
                        addend: 0,
                    });
                } else {
                    relocs.push(ObjReloc {
                        offset: reloc.offset,
                        reloc_type: reloc.reloc_type,
                        symbol_name: sym_name,
                        addend,
                    });
                }
            }

            shared_sections.insert(
                sec.name.clone(),
                ObjSection {
                    name: sec.name.clone(),
                    sh_type: sec.section_type,
                    sh_flags: sec.flags,
                    data,
                    sh_addralign: sec.alignment,
                    relocs,
                    comdat_group: sec.comdat_group.clone(),
                },
            );
        }

        // Convert label positions
        let labels: FxHashMap<String, (String, u64)> = self
            .label_positions
            .iter()
            .map(|(name, &(sec_idx, offset))| {
                (name.clone(), (section_names[sec_idx].clone(), offset))
            })
            .collect();

        let global_symbols: FxHashMap<String, bool> = self
            .pending_globals
            .iter()
            .map(|s| (s.clone(), true))
            .collect();
        let weak_symbols: FxHashMap<String, bool> = self
            .pending_weaks
            .iter()
            .map(|s| (s.clone(), true))
            .collect();

        let symbol_types: FxHashMap<String, u8> = self
            .pending_types
            .iter()
            .map(|(name, kind)| {
                let stt = match kind {
                    SymbolKind::Function => STT_FUNC,
                    SymbolKind::Object => STT_OBJECT,
                    SymbolKind::TlsObject => STT_TLS,
                    SymbolKind::GnuIndirectFunction => STT_GNU_IFUNC,
                    SymbolKind::NoType => STT_NOTYPE,
                };
                (name.clone(), stt)
            })
            .collect();

        // Resolve pending_sizes to concrete u64 values
        let symbol_sizes: FxHashMap<String, u64> = self
            .pending_sizes
            .iter()
            .map(|(name, expr)| {
                let size = match expr {
                    SizeExpr::Constant(v) => *v,
                    SizeExpr::CurrentMinusSymbol(start_sym) => {
                        if let Some(&(sec_idx, start_off)) = self.label_positions.get(start_sym) {
                            let end = self.sections[sec_idx].data.len() as u64;
                            end - start_off
                        } else {
                            0
                        }
                    }
                    SizeExpr::SymbolDiff(end_label, start_label) => {
                        let end_off = self
                            .label_positions
                            .get(end_label)
                            .map(|p| p.1)
                            .unwrap_or(0);
                        let start_off = self
                            .label_positions
                            .get(start_label)
                            .map(|p| p.1)
                            .unwrap_or(0);
                        end_off.wrapping_sub(start_off)
                    }
                    SizeExpr::SymbolRef(sym_ref) => {
                        // The kernel's SYM_FUNC_END expands to
                        //   .set .L__sym_size_X, .-X ; .size X, .L__sym_size_X
                        // The new absolute-.set fold evaluates that .set to a
                        // CONSTANT in set_values (the `.` position is the
                        // set-directive's own offset — exactly GAS
                        // semantics), so consult it FIRST; without this the
                        // sizes silently became 0 and objtool mis-decoded
                        // retpoline.o ("can't find jump dest instruction").
                        if let Some(&v) = self.set_values.get(sym_ref) {
                            return (name.clone(), v as u64);
                        }
                        if let Some(alias_target) = self.aliases.get(sym_ref) {
                            let normalized = alias_target.replace(' ', "");
                            if let Some(rest) = normalized.strip_prefix(".-") {
                                if let Some(&(sec_idx, start_off)) = self.label_positions.get(rest)
                                {
                                    let end = self.sections[sec_idx].data.len() as u64;
                                    end - start_off
                                } else {
                                    0
                                }
                            } else {
                                0
                            }
                        } else {
                            0
                        }
                    }
                };
                (name.clone(), size)
            })
            .collect();

        let mut symbol_visibility: FxHashMap<String, u8> = FxHashMap::default();
        for name in &self.pending_hidden {
            symbol_visibility.insert(name.clone(), STV_HIDDEN);
        }
        for name in &self.pending_protected {
            symbol_visibility.insert(name.clone(), STV_PROTECTED);
        }
        for name in &self.pending_internal {
            symbol_visibility.insert(name.clone(), STV_INTERNAL);
        }

        // Three-operand `.symver` visibility flags on DEFINED plain symbols
        // (GAS 2.35+ semantics, verified against binutils):
        //   local  — plain name demoted to STB_LOCAL (references keep binding
        //            to it inside the object),
        //   hidden — plain name stays STB_GLOBAL but gets STV_HIDDEN,
        //   remove — plain name is dropped from the symbol table entirely
        //            (its relocations were already rebound to the versioned
        //            alias in build()).
        let mut symver_removed: crate::common::fx_hash::FxHashSet<String> =
            crate::common::fx_hash::FxHashSet::default();
        let mut global_symbols = global_symbols;
        for (name, (ver, flag)) in &self.symver_flags {
            let defined = self.label_positions.contains_key(name)
                || self.aliases.contains_key(name)
                || self.set_values.contains_key(name);
            if !defined {
                // Undefined plain name: GAS silently applies plain reference
                // versioning (verified: `.symver und, und@V, remove` produces
                // `U und@V` with no diagnostic); nothing to do here.
                continue;
            }
            match flag {
                SymverFlag::Local => {
                    global_symbols.remove(name);
                    // The demotion applies ONLY to the plain name: the
                    // versioned alias stays STB_GLOBAL (GAS parity — that is
                    // the whole point of `local`: keep the versioned export,
                    // hide the unversioned name).
                    global_symbols.insert(ver.clone(), true);
                }
                SymverFlag::Hidden => {
                    symbol_visibility.insert(name.clone(), STV_HIDDEN);
                }
                SymverFlag::Remove => {
                    symver_removed.insert(name.clone());
                }
            }
        }

        let symtab_input = SymbolTableInput {
            labels: &labels,
            global_symbols: &global_symbols,
            weak_symbols: &weak_symbols,
            symbol_types: &symbol_types,
            symbol_sizes: &symbol_sizes,
            symbol_visibility: &symbol_visibility,
            aliases: &self.aliases,
            sections: &shared_sections,
            include_referenced_locals: false,
        };

        let mut shared_symbols = elf_mod::build_elf_symbol_table(&symtab_input);
        if !symver_removed.is_empty() {
            shared_symbols.retain(|s| !symver_removed.contains(&s.name));
        }

        // Add COMMON symbols
        for sym in &self.symbols {
            if sym.is_common {
                shared_symbols.retain(|s| !(s.name == sym.name && s.section_name == "*UND*"));
                shared_symbols.push(ObjSymbol {
                    name: sym.name.clone(),
                    value: sym.common_align as u64,
                    size: sym.size,
                    binding: sym.binding,
                    sym_type: sym.sym_type,
                    visibility: sym.visibility,
                    section_name: "*COM*".to_string(),
                });
            }
        }

        let config = ElfConfig {
            e_machine: A::elf_machine(),
            e_flags: A::elf_flags(),
            elf_class: A::elf_class(),
            force_rela: false,
        };

        elf_mod::write_relocatable_object(
            &config,
            &section_names,
            &shared_sections,
            &shared_symbols,
        )
    }

    // ─── Numeric label resolution ─────────────────────────────────────

    fn resolve_numeric_label(
        &self,
        symbol: &str,
        reloc_offset: u64,
        sec_idx: usize,
    ) -> Option<(usize, u64)> {
        let len = symbol.len();
        if len < 2 {
            return None;
        }
        let suffix = symbol.as_bytes()[len - 1];
        if suffix != b'b' && suffix != b'f' {
            return None;
        }
        let label_num = &symbol[..len - 1];
        if !label_num.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }

        let positions = self.numeric_label_positions.get(label_num)?;
        if suffix == b'b' {
            let mut best: Option<(usize, u64)> = None;
            for &(s_idx, off) in positions {
                if s_idx == sec_idx
                    && off <= reloc_offset
                    && (best.is_none() || off > best.unwrap().1)
                {
                    best = Some((s_idx, off));
                }
            }
            best
        } else {
            let mut best: Option<(usize, u64)> = None;
            for &(s_idx, off) in positions {
                if s_idx == sec_idx
                    && off > reloc_offset
                    && (best.is_none() || off < best.unwrap().1)
                {
                    best = Some((s_idx, off));
                }
            }
            best
        }
    }

    // ─── Jump relaxation ──────────────────────────────────────────────

    fn relax_jumps(&mut self) {
        for sec_idx in 0..self.sections.len() {
            if self.sections[sec_idx].jumps.is_empty() {
                continue;
            }

            // Branch relaxation is a fixed-point problem with MORE THAN ONE
            // solution, and the two obvious search directions do not find the
            // same one.
            //
            // Starting pessimistic (every jump long) and only shrinking gets
            // stuck: with a long jump in front of it, `.p2align 16` has to
            // insert more padding, which pushes the target out of disp8 range,
            // which "justifies" keeping the jump long.  That is self-consistent
            // but 16 bytes worse than necessary.  A 124-nop test case came out
            // 145 bytes against GAS's 129.
            //
            // Starting optimistic (every relaxable jump short) and only growing
            // converges on the SMALLEST fixed point instead, and terminates for
            // the same reason: every step strictly increases the section size,
            // which is bounded.  That is also what GAS does, so byte-for-byte
            // agreement comes for free.
            //
            // The first iteration therefore shrinks unconditionally; afterwards
            // only growth is considered.
            let mut first_pass = true;
            loop {
                let mut any_change = false;
                let mut local_labels: FxHashMap<String, usize> = FxHashMap::default();
                for (name, &(s_idx, offset)) in &self.label_positions {
                    if s_idx == sec_idx {
                        local_labels.insert(name.clone(), offset as usize);
                    }
                }

                // Classify every jump by its target distance.
                enum Action {
                    Shrink,
                    Grow,
                }
                let mut actions: Vec<(usize, Action)> = Vec::new();
                let dbg = std::env::var("CCC_DEBUG_RELAX").is_ok();
                for (j_idx, jump) in self.sections[sec_idx].jumps.iter().enumerate() {
                    let target_off_opt = local_labels.get(&jump.target).copied().or_else(|| {
                        self.resolve_numeric_label(&jump.target, jump.offset as u64, sec_idx)
                            .map(|(_, off)| off as usize)
                    });
                    if dbg {
                        eprintln!(
                            "[RELAX] sec{} j{} off={} len={} cond={} target={:?} target_off={:?} relaxed={} growable={}",
                            sec_idx,
                            j_idx,
                            jump.offset,
                            jump.len,
                            jump.is_conditional,
                            jump.target,
                            target_off_opt,
                            jump.relaxed,
                            jump.can_grow
                        );
                    }
                    let Some(target_off) = target_off_opt else {
                        continue;
                    };
                    // A forward target moves left with the jump's own shrink;
                    // a backward target does not.
                    let old_len = jump.len as i64;
                    let short_disp = if (target_off as i64) > jump.offset as i64 {
                        target_off as i64 - (jump.offset as i64 + old_len)
                    } else {
                        target_off as i64 - (jump.offset as i64 + 2)
                    };
                    let fits_short = (-128..=127).contains(&short_disp);
                    if first_pass && !jump.relaxed {
                        // Speculatively shorten every relaxable jump, even one
                        // that does not currently fit: the growth phase below
                        // restores exactly those that still do not fit once the
                        // layout has settled.
                        actions.push((j_idx, Action::Shrink));
                    } else if jump.relaxed && jump.can_grow && !fits_short {
                        actions.push((j_idx, Action::Grow));
                    }
                }

                if actions.is_empty() {
                    break;
                }

                // Process back to front so earlier offsets stay valid.
                actions.sort_unstable_by_key(|&(j, _)| j);
                actions.reverse();

                for &(j_idx, ref action) in &actions {
                    // Snapshot every field the transition needs so the
                    // immutable borrow ends before the mutable section edits.
                    let (offset, old_len, long_len, is_conditional, target) = {
                        let jump = &self.sections[sec_idx].jumps[j_idx];
                        (
                            jump.offset,
                            jump.len,
                            jump.long_len,
                            jump.is_conditional,
                            jump.target.clone(),
                        )
                    };

                    match action {
                        Action::Shrink => {
                            let new_len = 2usize;
                            let shrink = old_len - new_len;
                            let data = &mut self.sections[sec_idx].data;
                            if is_conditional {
                                let cc = data[offset + 1] - 0x80;
                                data[offset] = 0x70 + cc;
                                data[offset + 1] = 0;
                            } else {
                                data[offset] = 0xEB;
                                data[offset + 1] = 0;
                            }
                            let remove_start = offset + new_len;
                            let remove_end = offset + old_len;
                            data.drain(remove_start..remove_end);
                            // Shift label positions.
                            for (_, pos) in self.label_positions.iter_mut() {
                                if pos.0 == sec_idx && (pos.1 as usize) > offset {
                                    pos.1 -= shrink as u64;
                                }
                            }
                            for (_, positions) in self.numeric_label_positions.iter_mut() {
                                for pos in positions.iter_mut() {
                                    if pos.0 == sec_idx && (pos.1 as usize) > offset {
                                        pos.1 -= shrink as u64;
                                    }
                                }
                            }
                            self.sections[sec_idx].relocations.retain_mut(|reloc| {
                                let reloc_off = reloc.offset as usize;
                                let old_reloc_pos = if is_conditional {
                                    offset + 2
                                } else {
                                    offset + 1
                                };
                                if reloc_off == old_reloc_pos {
                                    return false;
                                }
                                if reloc_off > offset {
                                    reloc.offset -= shrink as u64;
                                }
                                true
                            });
                            for other_jump in self.sections[sec_idx].jumps.iter_mut() {
                                if other_jump.offset > offset {
                                    other_jump.offset -= shrink;
                                }
                            }
                            for marker in self.sections[sec_idx].align_markers.iter_mut() {
                                if marker.offset > offset {
                                    marker.offset -= shrink;
                                }
                            }
                            for skip in self.deferred_skips.iter_mut() {
                                if skip.sec_idx == sec_idx && skip.offset > offset {
                                    skip.offset -= shrink;
                                }
                            }
                            for (s_idx, s_off, _, _, _, _) in self.deferred_byte_diffs.iter_mut() {
                                if *s_idx == sec_idx && *s_off > offset {
                                    *s_off -= shrink;
                                }
                            }
                            self.sections[sec_idx].jumps[j_idx].relaxed = true;
                            self.sections[sec_idx].jumps[j_idx].can_grow = true;
                            self.sections[sec_idx].jumps[j_idx].len = new_len;
                        }
                        Action::Grow => {
                            let new_len = long_len;
                            let grow = new_len - old_len;
                            let data = &mut self.sections[sec_idx].data;
                            if is_conditional {
                                // short 0x7x disp8 -> near 0x0f 0x8x rel16/32
                                let cc = data[offset] - 0x70;
                                let insert = vec![0u8; grow];
                                data.splice(offset + 2..offset + 2, insert);
                                data[offset] = 0x0f;
                                data[offset + 1] = 0x80 + cc;
                            } else {
                                // short 0xeb disp8 -> near 0xe9 rel16/32
                                let insert = vec![0u8; grow];
                                data.splice(offset + 2..offset + 2, insert);
                                data[offset] = 0xE9;
                            }
                            // Restore the original mode's relocation width. A
                            // `.code16` near branch owns a two-byte rel16 field;
                            // writing rel32 would overwrite the next instruction.
                            let rel16 = long_len == if is_conditional { 4 } else { 3 };
                            let reloc_pos = (if is_conditional {
                                offset + 2
                            } else {
                                offset + 1
                            }) as u64;
                            self.sections[sec_idx].relocations.push(ElfRelocation {
                                offset: reloc_pos,
                                symbol: target.clone(),
                                reloc_type: if rel16 {
                                    A::reloc_pc16()
                                        .expect("rel16 branch without architecture relocation")
                                } else {
                                    A::reloc_pc32()
                                },
                                addend: if rel16 { -2 } else { -4 },
                                diff_symbol: None,
                                patch_size: if rel16 { 2 } else { 4 },
                            });
                            // Shift everything at or after the insertion point.
                            for (_, pos) in self.label_positions.iter_mut() {
                                if pos.0 == sec_idx && (pos.1 as usize) > offset {
                                    pos.1 += grow as u64;
                                }
                            }
                            for (_, positions) in self.numeric_label_positions.iter_mut() {
                                for pos in positions.iter_mut() {
                                    if pos.0 == sec_idx && (pos.1 as usize) > offset {
                                        pos.1 += grow as u64;
                                    }
                                }
                            }
                            for reloc in self.sections[sec_idx].relocations.iter_mut() {
                                if (reloc.offset as usize) > offset
                                    && reloc.offset as usize != reloc_pos as usize
                                {
                                    reloc.offset += grow as u64;
                                }
                            }
                            for other_jump in self.sections[sec_idx].jumps.iter_mut() {
                                if other_jump.offset > offset {
                                    other_jump.offset += grow;
                                }
                            }
                            for marker in self.sections[sec_idx].align_markers.iter_mut() {
                                if marker.offset > offset {
                                    marker.offset += grow;
                                }
                            }
                            for skip in self.deferred_skips.iter_mut() {
                                if skip.sec_idx == sec_idx && skip.offset > offset {
                                    skip.offset += grow;
                                }
                            }
                            for (s_idx, s_off, _, _, _, _) in self.deferred_byte_diffs.iter_mut() {
                                if *s_idx == sec_idx && *s_off > offset {
                                    *s_off += grow;
                                }
                            }
                            self.sections[sec_idx].jumps[j_idx].relaxed = false;
                            self.sections[sec_idx].jumps[j_idx].len = new_len;
                        }
                    }
                    any_change = true;
                }

                // Recompute alignment padding after the size changes.
                self.fixup_alignment_markers(sec_idx);
                first_pass = false;

                if !any_change {
                    break;
                }
            }

            // Final short-displacement resolution (after layout is stable).
            let mut local_labels: FxHashMap<String, usize> = FxHashMap::default();
            for (name, &(s_idx, offset)) in &self.label_positions {
                if s_idx == sec_idx {
                    local_labels.insert(name.clone(), offset as usize);
                }
            }
            let patches: Vec<(usize, u8)> = self.sections[sec_idx]
                .jumps
                .iter()
                .filter(|j| j.relaxed)
                .filter_map(|jump| {
                    let target = local_labels
                        .get(&jump.target)
                        .copied()
                        .or_else(|| {
                            self.resolve_numeric_label(&jump.target, jump.offset as u64, sec_idx)
                                .map(|(_, off)| off as usize)
                        });
                    target.map(|target_off| {
                        let end_of_instr = jump.offset + 2;
                        let disp = target_off as i64 - end_of_instr as i64;
                        // A surviving short jump must be representable exactly.
                        assert!(
                            (-128..=127).contains(&disp),
                            "short jump displacement out of range after relaxation ({}: {} -> {} = {})",
                            jump.target,
                            jump.offset,
                            target_off,
                            disp
                        );
                        (jump.offset + 1, disp as u8)
                    })
                })
                .collect();
            for (off, byte) in patches {
                self.sections[sec_idx].data[off] = byte;
            }
        }
    }

    fn fixup_alignment_markers(&mut self, sec_idx: usize) {
        if self.sections[sec_idx].align_markers.is_empty() {
            return;
        }

        // Sort by offset to ensure front-to-back processing
        self.sections[sec_idx]
            .align_markers
            .sort_by_key(|m| m.offset);

        let is_exec = self.sections[sec_idx].flags & SHF_EXECINSTR != 0;

        let mut marker_idx = 0;
        loop {
            if marker_idx >= self.sections[sec_idx].align_markers.len() {
                break;
            }
            let current_offset = self.sections[sec_idx].align_markers[marker_idx].offset;
            let kind = self.sections[sec_idx].align_markers[marker_idx]
                .kind
                .clone();

            let needed_end = match &kind {
                AlignMarkerKind::Align(align) => {
                    let a = *align as usize;
                    if a <= 1 {
                        marker_idx += 1;
                        continue;
                    }
                    (current_offset + a - 1) & !(a - 1)
                }
                AlignMarkerKind::Org { label, addend } => {
                    if label.is_empty() {
                        *addend as usize
                    } else if let Some(&(l_sec, l_off)) = self.label_positions.get(label.as_str()) {
                        if l_sec == sec_idx {
                            (l_off as i64 + *addend) as usize
                        } else {
                            marker_idx += 1;
                            continue;
                        }
                    } else {
                        marker_idx += 1;
                        continue;
                    }
                }
            };

            let needed_padding = needed_end.saturating_sub(current_offset);
            let existing_padding = self.sections[sec_idx].align_markers[marker_idx].padding;

            if needed_padding != existing_padding {
                // Regenerate the ENTIRE padding run rather than splicing the
                // delta. Executable padding is a sequence of multi-byte NOP
                // *instructions*, so inserting or deleting bytes in the middle
                // of it would corrupt an instruction; only a full rebuild
                // yields a valid, optimally-sized NOP sequence for the new
                // length.
                let start = current_offset;
                let old_end = start + existing_padding;
                let after_insn = self.sections[sec_idx].align_markers[marker_idx].after_insn;
                let new_bytes = section_padding(needed_padding, is_exec, after_insn);
                debug_assert_eq!(new_bytes.len(), needed_padding);
                if old_end <= self.sections[sec_idx].data.len() {
                    self.sections[sec_idx]
                        .data
                        .splice(start..old_end, new_bytes);
                    let delta = needed_padding as i64 - existing_padding as i64;
                    if delta != 0 {
                        // Anchor the shift at the run's END so a label sitting
                        // exactly at `start` (before the padding) is not
                        // dragged along with it.
                        self.shift_offsets_after(sec_idx, old_end, delta, marker_idx);
                    }
                }
            }
            // Persist the adjusted size so repeated fixup passes are idempotent.
            self.sections[sec_idx].align_markers[marker_idx].padding = needed_padding;

            marker_idx += 1;
        }
    }

    /// Shift all labels, relocations, jumps, alignment markers, deferred skips,
    /// and deferred byte diffs in a section after an insertion or removal at `at_offset`.
    fn shift_offsets_after(
        &mut self,
        sec_idx: usize,
        at_offset: usize,
        delta: i64,
        current_marker_idx: usize,
    ) {
        if delta == 0 {
            return;
        }
        for (_, pos) in self.label_positions.iter_mut() {
            if pos.0 == sec_idx && (pos.1 as usize) >= at_offset {
                pos.1 = (pos.1 as i64 + delta) as u64;
            }
        }
        for (_, positions) in self.numeric_label_positions.iter_mut() {
            for pos in positions.iter_mut() {
                if pos.0 == sec_idx && (pos.1 as usize) >= at_offset {
                    pos.1 = (pos.1 as i64 + delta) as u64;
                }
            }
        }
        for reloc in self.sections[sec_idx].relocations.iter_mut() {
            if (reloc.offset as usize) >= at_offset {
                reloc.offset = (reloc.offset as i64 + delta) as u64;
            }
        }
        for jump in self.sections[sec_idx].jumps.iter_mut() {
            if jump.offset >= at_offset {
                jump.offset = (jump.offset as i64 + delta) as usize;
            }
        }
        for i in (current_marker_idx + 1)..self.sections[sec_idx].align_markers.len() {
            if self.sections[sec_idx].align_markers[i].offset >= at_offset {
                self.sections[sec_idx].align_markers[i].offset =
                    (self.sections[sec_idx].align_markers[i].offset as i64 + delta) as usize;
            }
        }
        // Update deferred skips and byte diffs
        for skip in self.deferred_skips.iter_mut() {
            if skip.sec_idx == sec_idx && skip.offset >= at_offset {
                skip.offset = (skip.offset as i64 + delta) as usize;
            }
        }
        for (bd_sec, bd_off, _, _, _, _) in self.deferred_byte_diffs.iter_mut() {
            if *bd_sec == sec_idx && *bd_off >= at_offset {
                *bd_off = (*bd_off as i64 + delta) as usize;
            }
        }
    }

    // ─── Symbol locality check ────────────────────────────────────────

    fn is_local_symbol(&self, name: &str) -> bool {
        if name.starts_with('.') {
            return true;
        }
        if name.len() >= 2 {
            let last = name.as_bytes()[name.len() - 1];
            if (last == b'f' || last == b'b')
                && name[..name.len() - 1].chars().all(|c| c.is_ascii_digit())
            {
                return true;
            }
        }
        if let Some(&sym_idx) = self.symbol_map.get(name) {
            self.symbols[sym_idx].binding == STB_LOCAL
        } else {
            false
        }
    }

    // ─── Internal relocation resolution ───────────────────────────────

    /// Substitute a standalone `.` (current position) in a .set expression
    /// with a SYNTHETIC LABEL registered at the current offset. A frozen
    /// integer would be wrong: jump relaxation shrinks code AFTER this
    /// directive is seen, and GAS treats `.` as a label-valued expression
    /// that moves with the layout (verify_cpu measured 315 pre-relaxation
    /// vs GAS's 249 -- objtool then rejected head_64.o). Labels are kept
    /// consistent by relax_jumps/fixup_alignment_markers, so evaluating
    /// after layout yields the exact GAS value.
    fn substitute_dot(&mut self, expr: &str) -> Option<(usize, String)> {
        let sec_idx = self.current_section?;
        let offset = self.sections[sec_idx].data.len() as u64;
        let bytes = expr.as_bytes();
        let mut out = String::with_capacity(expr.len() + 24);
        let mut label_made = false;
        let mut label_name = String::new();
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'.' {
                let prev_ok = i == 0
                    || !(bytes[i - 1].is_ascii_alphanumeric()
                        || bytes[i - 1] == b'_'
                        || bytes[i - 1] == b'.');
                let next_ok = i + 1 >= bytes.len()
                    || !(bytes[i + 1].is_ascii_alphanumeric()
                        || bytes[i + 1] == b'_'
                        || bytes[i + 1] == b'.');
                if prev_ok && next_ok {
                    if !label_made {
                        label_name = format!(".Ldotpos_{}_{}", sec_idx, offset);
                        self.label_positions
                            .insert(label_name.clone(), (sec_idx, offset));
                        label_made = true;
                    }
                    out.push_str(&label_name);
                    continue;
                }
            }
            out.push(b as char);
        }
        Some((sec_idx, out))
    }

    /// Evaluate an integer expression that may reference SAME-SECTION labels
    /// (label -> section offset) and previously-evaluated .set constants.
    /// None if any symbol is unknown or cross-section.
    fn eval_label_expr(&self, expr: &str, sec_idx: usize) -> Option<i64> {
        let bytes = expr.as_bytes();
        let mut out = String::with_capacity(expr.len() + 16);
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            // NUMBER tokens first: `0x100` must not become `0` + ident `x100`.
            if b.is_ascii_digit() {
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
                    i += 1;
                }
                out.push_str(&expr[start..i]);
                continue;
            }
            if b.is_ascii_alphabetic() || b == b'_' || b == b'.' {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric()
                        || bytes[i] == b'_'
                        || bytes[i] == b'.'
                        || bytes[i] == b'$')
                {
                    i += 1;
                }
                let ident = &expr[start..i];
                if let Some(&(lsec, loff)) = self.label_positions.get(ident) {
                    if lsec != sec_idx {
                        return None;
                    }
                    out.push_str(&loff.to_string());
                } else if let Some(&v) = self.set_values.get(ident) {
                    out.push_str(&v.to_string());
                } else {
                    return None;
                }
            } else {
                out.push(b as char);
                i += 1;
            }
        }
        crate::backend::asm_expr::parse_integer_expr(&out).ok()
    }

    /// Late resolution: pending `.`-position .set expressions, then convert
    /// relocations against constant .set names into direct data patches.
    /// GAS semantics for `.set NAME, <constant>`: NAME becomes an ABSOLUTE
    /// symbol (st_shndx = SHN_ABS) with the constant as its value. It stays
    /// in the symbol table -- mkpiggy's `z_input_len = N` plus `.globl
    /// z_input_len` is scraped by the boot Makefile's ZOFFSET rule via nm,
    /// which broke when the constant fold swallowed the symbol entirely
    /// ("undefined reference to ZO_z_input_len" in header.S).
    /// The writer consumes abs_symbols in build_symbol_table.
    fn resolve_set_constants(&mut self) {
        let pending = std::mem::take(&mut self.pending_set_exprs);
        for (alias, expr, sec_idx) in pending {
            if let Some(val) = self.eval_label_expr(&expr, sec_idx) {
                self.set_values.insert(alias.clone(), val);
                self.aliases.remove(&alias);
            }
        }
        if self.set_values.is_empty() {
            return;
        }
        for sec_idx in 0..self.sections.len() {
            let mut remaining = Vec::new();
            let relocs = std::mem::take(&mut self.sections[sec_idx].relocations);
            for reloc in relocs {
                if reloc.diff_symbol.is_none() {
                    if let Some(&val) = self.set_values.get(&reloc.symbol) {
                        let v = val + reloc.addend;
                        let off = reloc.offset as usize;
                        let data = &mut self.sections[sec_idx].data;
                        match reloc.patch_size {
                            1 => {
                                data[off] = v as u8;
                            }
                            2 => {
                                data[off..off + 2].copy_from_slice(&(v as i16).to_le_bytes());
                            }
                            8 => {
                                data[off..off + 8].copy_from_slice(&v.to_le_bytes());
                            }
                            _ => {
                                data[off..off + 4].copy_from_slice(&(v as i32).to_le_bytes());
                            }
                        }
                        continue;
                    }
                }
                remaining.push(reloc);
            }
            self.sections[sec_idx].relocations = remaining;
        }

        // GAS keeps `.set NAME, <constant>` in the symbol table as an
        // SHN_ABS symbol. Re-export every folded constant as a decimal
        // alias so the symbol-table builder's integer-alias branch emits
        // it; mkpiggy's `z_input_len = N` + `.globl z_input_len` is
        // scraped from nm output by the boot Makefile's ZOFFSET rule, and
        // swallowing it produced "undefined reference to ZO_z_input_len"
        // in header.S.
        for (name, val) in self.set_values.clone() {
            self.aliases.entry(name).or_insert_with(|| val.to_string());
        }
    }

    fn resolve_internal_relocations(&mut self) {
        for sec_idx in 0..self.sections.len() {
            let mut resolved: Vec<(usize, i64, usize)> = Vec::new(); // (offset, value, patch_size)
            let mut pc8_patches: Vec<(usize, u8)> = Vec::new();
            let mut unresolved = Vec::new();

            for reloc in &self.sections[sec_idx].relocations {
                // Handle SymbolDiff relocations
                if reloc.diff_symbol.is_some() {
                    if let Some(ref diff_sym) = reloc.diff_symbol {
                        // Either side may be a NUMERIC local label (`0b` in
                        // relocate_kernel_64.S `$identity_mapped - 0b`);
                        // label_positions only holds named labels.
                        let a_pos =
                            self.label_positions
                                .get(&reloc.symbol)
                                .copied()
                                .or_else(|| {
                                    self.resolve_numeric_label(&reloc.symbol, reloc.offset, sec_idx)
                                });
                        let b_pos = self.label_positions.get(diff_sym).copied().or_else(|| {
                            self.resolve_numeric_label(diff_sym, reloc.offset, sec_idx)
                        });
                        if let (Some((a_sec, a_off)), Some((b_sec, b_off))) = (a_pos, b_pos) {
                            if a_sec == b_sec {
                                let val = a_off as i64 - b_off as i64 + reloc.addend;
                                resolved.push((
                                    reloc.offset as usize,
                                    val,
                                    reloc.patch_size as usize,
                                ));
                                continue;
                            }
                        }
                        // `$sym - 0b` where sym is EXTERNAL (or in another
                        // section) but the subtrahend is a local label in
                        // THIS section: GAS emits R_X86_64_PC32 against sym
                        // with addend = reloc_offset - addr(0b), so the link
                        // computes S + (P - addr(0b)) - P = S - addr(0b).
                        // relocate_kernel_64.S relies on this for computing
                        // `identity_mapped - 0b` where identity_mapped lives
                        // in a different section of the same object.
                        if a_pos.is_none() || a_pos.map(|(s, _)| s) != Some(sec_idx) {
                            if let Some((b_sec, b_off)) = b_pos {
                                if b_sec == sec_idx && reloc.patch_size == 4 {
                                    let mut conv = reloc.clone();
                                    conv.reloc_type = A::reloc_pc32();
                                    conv.addend += reloc.offset as i64 - b_off as i64;
                                    conv.diff_symbol = None;
                                    unresolved.push(conv);
                                    continue;
                                }
                            }
                        }
                    }
                    unresolved.push(reloc.clone());
                    continue;
                }

                let label_pos = self
                    .label_positions
                    .get(&reloc.symbol)
                    .copied()
                    .or_else(|| self.resolve_numeric_label(&reloc.symbol, reloc.offset, sec_idx));

                if let Some((target_sec, target_off)) = label_pos {
                    let is_local = self.is_local_symbol(&reloc.symbol);

                    // Same-section rel16 branches (.code16): patch the
                    // 2-byte field directly — GAS never emits a reloc for
                    // a local 16-bit branch.
                    if let Some(pc16) = A::reloc_pc16() {
                        if reloc.reloc_type == pc16 && target_sec == sec_idx && is_local {
                            let rel = (target_off as i64) + reloc.addend - (reloc.offset as i64);
                            resolved.push((reloc.offset as usize, rel, 2));
                            continue;
                        }
                    }

                    // Handle PC8 internal relocations (x86-64 loop/jrcxz)
                    if let Some(pc8_type) = A::reloc_pc8_internal() {
                        if reloc.reloc_type == pc8_type && target_sec == sec_idx {
                            let rel = (target_off as i64) + reloc.addend - (reloc.offset as i64);
                            if (-128..=127).contains(&rel) {
                                pc8_patches.push((reloc.offset as usize, rel as u8));
                            }
                            continue;
                        }
                    }

                    if target_sec == sec_idx
                        && is_local
                        && (reloc.reloc_type == A::reloc_pc32()
                            || reloc.reloc_type == A::reloc_plt32()
                            || (reloc.reloc_type == A::reloc_pc64() && reloc.patch_size == 8))
                    {
                        let rel = (target_off as i64) + reloc.addend - (reloc.offset as i64);
                        resolved.push((reloc.offset as usize, rel, reloc.patch_size as usize));
                    } else if let Some(abs32_type) = A::reloc_abs32_for_internal() {
                        if target_sec == sec_idx && is_local && reloc.reloc_type == abs32_type {
                            let val = (target_off as i64) + reloc.addend;
                            resolved.push((reloc.offset as usize, val, reloc.patch_size as usize));
                        } else {
                            unresolved.push(reloc.clone());
                        }
                    } else {
                        unresolved.push(reloc.clone());
                    }
                } else {
                    unresolved.push(reloc.clone());
                }
            }

            // Patch resolved relocations into section data
            for (offset, value, psz) in resolved {
                if psz == 1 {
                    self.sections[sec_idx].data[offset] = value as u8;
                } else if psz == 2 {
                    let bytes = (value as i16).to_le_bytes();
                    self.sections[sec_idx].data[offset..offset + 2].copy_from_slice(&bytes);
                } else if psz == 8 {
                    // .quad a - b: full 64-bit patch. The old catch-all wrote
                    // only 4 bytes, silently truncating negative or >4GiB
                    // differences (e.g. `.quad a - b` with a < b kept its
                    // upper half zero instead of sign-extending).
                    let bytes = value.to_le_bytes();
                    self.sections[sec_idx].data[offset..offset + 8].copy_from_slice(&bytes);
                } else {
                    let bytes = (value as i32).to_le_bytes();
                    self.sections[sec_idx].data[offset..offset + 4].copy_from_slice(&bytes);
                }
            }
            for (offset, value) in pc8_patches {
                self.sections[sec_idx].data[offset] = value;
            }

            self.sections[sec_idx].relocations = unresolved;
        }
    }
}

/// ULEB128 encode (used by .uleb128 data emission).
pub(crate) fn encode_uleb128(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let mut byte = (v & 0x7F) as u8;
        v >>= 7;
        if v != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if v == 0 {
            break;
        }
    }
}

/// SLEB128 encode (used by .sleb128 data emission).
pub(crate) fn encode_sleb128(out: &mut Vec<u8>, mut val: i64) {
    loop {
        let mut byte = (val & 0x7F) as u8;
        val >>= 7;
        let sign = byte & 0x40 != 0;
        if (val == 0 && !sign) || (val == -1 && sign) {
            out.push(byte);
            break;
        }
        byte |= 0x80;
        out.push(byte);
    }
}

#[cfg(test)]
mod tests {
    use crate::backend::x86::assembler::assemble;

    fn sec_type_of(obj: &[u8], name: &str) -> Option<u32> {
        // Minimal ELF64 section-header walk (test-only).
        let e_shoff = u64::from_le_bytes(obj[40..48].try_into().unwrap()) as usize;
        let e_shentsize = u16::from_le_bytes(obj[58..60].try_into().unwrap()) as usize;
        let e_shnum = u16::from_le_bytes(obj[60..62].try_into().unwrap()) as usize;
        let shstr = {
            let shstr_idx = u16::from_le_bytes(obj[62..64].try_into().unwrap()) as usize;
            let off = e_shoff + shstr_idx * e_shentsize;
            let sh_offset = u64::from_le_bytes(obj[off + 24..off + 32].try_into().unwrap()) as usize;
            let sh_size = u64::from_le_bytes(obj[off + 32..off + 40].try_into().unwrap()) as usize;
            &obj[sh_offset..sh_offset + sh_size]
        };
        for i in 0..e_shnum {
            let off = e_shoff + i * e_shentsize;
            let name_off = u32::from_le_bytes(obj[off..off + 4].try_into().unwrap()) as usize;
            let mut end = name_off;
            while shstr[end] != 0 {
                end += 1;
            }
            if &shstr[name_off..end] == name.as_bytes() {
                return Some(u32::from_le_bytes(obj[off + 4..off + 8].try_into().unwrap()));
            }
        }
        None
    }

    /// GNU as assigns well-known section names a fixed type: `.section
    /// .data,"aw",@nobits` must still produce a PROGBITS .data, or every later
    /// `.section .data` in the file loses its contents (kernel compressed
    /// misc.c boot hang: `static int lines __section(".data")` preceded the
    /// relocatable pointer data).
    #[test]
    fn well_known_section_type_is_fixed() {
        let dir = std::env::temp_dir().join(format!(
            "lccc_sectype_{}_{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("t")
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("m.o");
        let asm = concat!(
            ".section .data,\"aw\",@nobits\n",
            "lines:\n",
            "    .zero 4\n",
            ".section .data\n",
            ".globl ptr\n",
            "ptr:\n",
            "    .quad .Lstr0\n",
            ".section .rodata\n",
            ".Lstr0:\n",
            "    .byte 65, 0\n",
        );
        assemble(asm, out.to_str().unwrap()).unwrap();
        let data = std::fs::read(&out).unwrap();
        assert_eq!(sec_type_of(&data, ".data"), Some(1 /* SHT_PROGBITS */));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
