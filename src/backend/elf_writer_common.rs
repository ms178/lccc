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

use crate::common::fx_hash::FxHashMap;
use crate::backend::x86::assembler::parser::*;
use crate::backend::elf::{self as elf_mod,
    SHT_PROGBITS,
    SHF_ALLOC, SHF_EXECINSTR,
    STB_LOCAL, STB_GLOBAL, STB_WEAK,
    STT_NOTYPE, STT_OBJECT, STT_FUNC, STT_TLS, STT_GNU_IFUNC,
    STV_DEFAULT, STV_INTERNAL, STV_HIDDEN, STV_PROTECTED,
    resolve_numeric_labels, parse_section_flags,
    ElfConfig, ObjSection, ObjSymbol, ObjReloc, SymbolTableInput,
};

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
    fn elf_flags() -> u32 { 0 }

    /// Absolute relocation type for data (R_X86_64_32/R_X86_64_64 or R_386_32).
    fn reloc_abs(size: usize) -> u32;
    /// 64-bit absolute relocation (R_X86_64_64). Only meaningful for x86-64.
    fn reloc_abs64() -> u32;
    /// PC-relative relocation type (R_X86_64_PC32 or R_386_PC32).
    fn reloc_pc32() -> u32;
    /// PLT relocation type (R_X86_64_PLT32 or R_386_PLT32).
    fn reloc_plt32() -> u32;

    /// Whether this architecture uses REL format (i686) vs RELA (x86-64).
    /// When true, addends are patched into section data instead of being
    /// stored in the relocation entry.
    fn uses_rel_format() -> bool;

    /// Optional: PC8 internal relocation type for loop/jrcxz instructions.
    /// Only x86-64 has this; i686 returns None.
    fn reloc_pc8_internal() -> Option<u32> { None }

    /// Optional: absolute 32-bit relocation for local symbol references.
    /// Only x86-64 uses R_X86_64_32 this way; i686 returns None since
    /// its R_386_32 is handled by the general abs path.
    fn reloc_abs32_for_internal() -> Option<u32> { None }

    /// Whether `.skip` expressions with label arithmetic are supported.
    /// Both x86-64 and i686 enable this for the Linux kernel's ALTERNATIVES macros.
    fn supports_deferred_skips() -> bool { false }

    /// Whether `.set` alias resolution for label-difference expressions
    /// should be done during data value emission. Both x86-64 and i686
    /// enable this for DWARF debug info `.set .Lset0, .LECIE-.LSCIE` patterns.
    fn resolve_set_aliases_in_data() -> bool { false }

    /// Default code mode for this architecture (64 for x86-64, 32 for i686).
    fn default_code_mode() -> u8 { 64 }

    /// Encode an instruction in 64-bit mode. Used by the i686 assembler when
    /// encountering `.code64` sections (e.g. kernel realmode trampoline code).
    /// Default implementation delegates to the normal encode_instruction.
    fn encode_instruction_code64(
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
    len: usize,
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
    &[0x90],                                                        // nop
    &[0x66, 0x90],                                                  // xchg %ax,%ax
    &[0x0F, 0x1F, 0x00],                                            // nopl (%rax)
    &[0x0F, 0x1F, 0x40, 0x00],                                      // nopl 0(%rax)
    &[0x0F, 0x1F, 0x44, 0x00, 0x00],                                // nopl 0(%rax,%rax,1)
    &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],                          // nopw 0(%rax,%rax,1)
    &[0x0F, 0x1F, 0x80, 0x00, 0x00, 0x00, 0x00],                    // nopl 0L(%rax)
    &[0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00],              // nopl 0L(%rax,%rax,1)
    &[0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00],        // nopw 0L(%rax,%rax,1)
    &[0x66, 0x2E, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00],  // nopw %cs:0L(...)
    &[0x66, 0x66, 0x2E, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00],
];

/// The largest single NOP we will emit (matches GAS's `alt_patt` limit).
const MAX_NOP: usize = NOP_PATTERNS.len();

/// Above this many maximum-size NOPs, jumping over the padding is cheaper
/// than executing it. Matches GAS's `max_number_of_nops` for the long-NOP
/// table: at 11 bytes per NOP the switch happens past ~77 bytes of padding,
/// where a predicted-taken branch clearly beats decoding eight more NOPs.
const MAX_NOP_RUN: usize = 7;

/// Build `count` bytes of executable padding.
///
/// The layout mirrors GAS exactly: an optional remainder NOP sized
/// `count % MAX_NOP` first, then a run of maximum-size NOPs. When the run
/// would exceed `MAX_NOP_RUN` NOPs the whole gap is skipped with a single
/// branch instead, so a large alignment gap costs one predicted-taken jump
/// rather than dozens of decoded NOPs.
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

    let remainder = count % MAX_NOP;
    if remainder != 0 {
        out.extend_from_slice(NOP_PATTERNS[remainder - 1]);
        count -= remainder;
    }
    while count != 0 {
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
            b' ' | b'\t' => { i += 1; }
            b'+' => { tokens.push(ExprToken::Plus); i += 1; }
            b'-' => { tokens.push(ExprToken::Minus); i += 1; }
            b'*' => { tokens.push(ExprToken::Star); i += 1; }
            b'(' => { tokens.push(ExprToken::LParen); i += 1; }
            b')' => { tokens.push(ExprToken::RParen); i += 1; }
            b'<' => { tokens.push(ExprToken::Lt); i += 1; }
            b'>' => { tokens.push(ExprToken::Gt); i += 1; }
            b'&' => { tokens.push(ExprToken::And); i += 1; }
            b'|' => { tokens.push(ExprToken::Or); i += 1; }
            b'^' => { tokens.push(ExprToken::Xor); i += 1; }
            b'~' => { tokens.push(ExprToken::Not); i += 1; }
            b'0'..=b'9' => {
                let start = i;
                if i + 1 < bytes.len() && bytes[i] == b'0' && (bytes[i+1] == b'x' || bytes[i+1] == b'X') {
                    i += 2;
                    while i < bytes.len() && bytes[i].is_ascii_hexdigit() { i += 1; }
                } else {
                    while i < bytes.len() && bytes[i].is_ascii_digit() { i += 1; }
                }
                // Check for numeric label references: digits followed by 'b' or 'f'
                // (e.g., "0b" = backward ref to label 0, "1f" = forward ref to label 1)
                if i < bytes.len() && (bytes[i] == b'b' || bytes[i] == b'f')
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
                        num_str.parse::<i64>()
                            .map_err(|_| format!("bad number: {}", num_str))?
                    };
                    tokens.push(ExprToken::Number(val));
                }
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'.' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.') {
                    i += 1;
                }
                tokens.push(ExprToken::Symbol(expr[start..i].to_string()));
            }
            c => return Err(format!("unexpected character in expression: '{}' (0x{:02x})", c as char, c)),
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
    /// `.symver real, name@VER` where `real` is NOT defined in this object:
    /// GNU-as semantics — every reference to `real` becomes a versioned
    /// reference `name@VER` (glibc compat_symbol_reference; needed because
    /// GNU ld 2.47 does not bind unversioned refs to foo@VER under
    /// --whole-archive). Applied to relocations after all items are parsed.
    symver_refs: FxHashMap<String, String>,
    section_stack: Vec<(Option<usize>, Option<usize>)>,
    /// Deferred `.skip` expressions: (section_index, offset, expression, fill_byte).
    deferred_skips: Vec<(usize, usize, String, u8)>,
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
            symver_refs: FxHashMap::default(),
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
        // GNU-as .symver references: if `real` was never defined as a label,
        // rewrite relocations against it to the versioned name (foo@VER).
        // `@@` in a reference selects the default version -> single @.
        if !self.symver_refs.is_empty() {
            let defined: crate::common::fx_hash::FxHashSet<&String> =
                self.label_positions.keys().collect();
            for sec in self.sections.iter_mut() {
                for reloc in sec.relocations.iter_mut() {
                    if let Some(ver) = self.symver_refs.get(&reloc.symbol) {
                        if !defined.contains(&reloc.symbol) {
                            reloc.symbol = ver.replace("@@", "@");
                        }
                    }
                }
            }
        }
        self.emit_elf()
    }

    fn get_or_create_section(&mut self, name: &str, section_type: u32, flags: u64, comdat_group: Option<String>) -> usize {
        if let Some(&idx) = self.section_map.get(name) {
            return idx;
        }
        let idx = self.sections.len();
        self.sections.push(Section {
            name: name.to_string(),
            section_type,
            flags,
            data: Vec::new(),
            alignment: if flags & SHF_EXECINSTR != 0 && name != ".init" && name != ".fini" { 16 } else { 1 },
            relocations: Vec::new(),
            jumps: Vec::new(),
            align_markers: Vec::new(),
            comdat_group,
        });
        self.section_map.insert(name.to_string(), idx);
        idx
    }

    fn current_section_mut(&mut self) -> Result<&mut Section, String> {
        let idx = self.current_section.ok_or("no active section")?;
        Ok(&mut self.sections[idx])
    }

    fn switch_section(&mut self, dir: &SectionDirective) {
        let (section_type, flags) = parse_section_flags(&dir.name, dir.flags.as_deref(), dir.section_type.as_deref());
        let idx = self.get_or_create_section(&dir.name, section_type, flags, dir.comdat_group.clone());
        self.previous_section = self.current_section;
        self.current_section = Some(idx);
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
            | AsmItem::Symver(_, _)
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
                self.switch_section(dir);
            }
            AsmItem::PushSection(dir) => {
                self.section_stack.push((self.current_section, self.previous_section));
                self.switch_section(dir);
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
                self.pending_globals.push(name.clone());
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
                            self.label_positions.insert(end_label.clone(), (sec_idx, current_off));
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
                    section.data
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
                        self.deferred_skips.push((sec_idx, offset, expr.clone(), *fill));
                    }
                } else {
                    // Simple integer parse for architectures without deferred skip support
                    if let Ok(val) = expr.trim().parse::<u64>() {
                        let section = self.current_section_mut()?;
                        section.data.extend(std::iter::repeat_n(*fill, val as usize));
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
                self.aliases.insert(alias.clone(), target.clone());
            }
            AsmItem::Symver(name, ver_string) => {
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
                    }
                }
            }
            AsmItem::Incbin { path, skip, count } => {
                let data = std::fs::read(path)
                    .map_err(|e| format!(".incbin: failed to read '{}': {}", path, e))?;
                let skip = *skip as usize;
                let data = if skip < data.len() { &data[skip..] } else { &[] };
                let data = match count {
                    Some(c) => {
                        let c = *c as usize;
                        if c < data.len() { &data[..c] } else { data }
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
            AsmItem::Cfi(_) | AsmItem::File(_, _) | AsmItem::Loc(_, _, _)
            | AsmItem::OptionDirective(_) | AsmItem::Empty => {}
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
        } else if let Some((label_sec, label_off)) = self.resolve_numeric_label(sym, current, sec_idx) {
            if label_sec == sec_idx {
                (label_off as i64 + offset) as u64
            } else {
                return Err(format!(".org symbol {} not in current section", sym));
            }
        } else {
            return Err(format!(".org: unknown symbol {}", sym));
        };
        let padding = if target > current { (target - current) as usize } else { 0 };
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
            let idx = self.get_or_create_section(".text", SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR, None);
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
                                let b = target[pos+1..].trim().to_string();
                                let offset = self.sections[sec_idx].data.len() as u64;
                                self.sections[sec_idx].relocations.push(ElfRelocation {
                                    offset,
                                    symbol: a,
                                    reloc_type: if size <= 4 { A::reloc_pc32() } else { A::reloc_abs64() },
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
                DataValue::SymbolDiffScaled(a, b, scale, addend) => {
                    // `(a-b)*scale + addend`. A scaled difference cannot be
                    // expressed by any ELF relocation, so it must be folded to
                    // a constant here; that is sound precisely because a
                    // same-section label difference is link-time invariant.
                    self.deferred_scaled_diffs.push(ScaledDiff {
                        sec_idx,
                        offset: self.sections[sec_idx].data.len(),
                        a: self.aliases.get(a).cloned().unwrap_or_else(|| a.clone()),
                        b: self.aliases.get(b).cloned().unwrap_or_else(|| b.clone()),
                        scale: *scale,
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

    fn emit_symbol_diff(&mut self, sec_idx: usize, a: &str, b: &str, size: usize, addend: i64) -> Result<(), String> {
        let offset = self.sections[sec_idx].data.len() as u64;
        let a_resolved = self.aliases.get(a).cloned().unwrap_or_else(|| a.to_string());
        let b_resolved = self.aliases.get(b).cloned().unwrap_or_else(|| b.to_string());

        if b_resolved == "." {
            // `sym - .` means PC-relative
            self.sections[sec_idx].relocations.push(ElfRelocation {
                offset,
                symbol: a_resolved,
                reloc_type: A::reloc_pc32(),
                addend,
                diff_symbol: None,
                patch_size: size as u8,
            });
            let section = &mut self.sections[sec_idx];
            section.data.extend(std::iter::repeat_n(0, size));
        } else if size <= 2 && A::supports_deferred_skips() {
            // For byte/short-sized diffs, defer resolution until after
            // deferred skips are inserted (skip insertion shifts offsets).
            let offset_usize = self.sections[sec_idx].data.len();
            self.deferred_byte_diffs.push((sec_idx, offset_usize, a_resolved, b_resolved, size, addend));
            let section = &mut self.sections[sec_idx];
            section.data.extend(std::iter::repeat_n(0, size));
        } else {
            self.sections[sec_idx].relocations.push(ElfRelocation {
                offset,
                symbol: a_resolved,
                reloc_type: if size == 4 { A::reloc_pc32() } else { A::reloc_abs64() },
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
                        "scaled symbol difference in .uleb128/.sleb128 is unsupported"
                            .to_string(),
                    );
                }
                DataValue::Symbol(sym) | DataValue::SymbolOffset(sym, _) => {
                    return Err(format!(
                        "unsupported symbol reference {} in .uleb128/.sleb128", sym
                    ));
                }
            }
        }
        Ok(())
    }

    fn defer_leb_diff(
        &mut self, sec_idx: usize, a: &str, b: &str, addend: i64, signed: bool,
    ) -> Result<(), String> {
        let a_resolved = self.aliases.get(a).cloned().unwrap_or_else(|| a.to_string());
        let b_resolved = self.aliases.get(b).cloned().unwrap_or_else(|| b.to_string());
        let offset = self.sections[sec_idx].data.len();
        if b_resolved == "." {
            return Err("symbol minus current position in .uleb128 is unsupported".to_string());
        }
        // Forward references are fine: resolution happens after all items
        // are processed and every label position is known. Reserve the
        // maximum LEB128 width (10 bytes for u64); resolve shrinks to fit.
        self.deferred_leb_diffs.push((sec_idx, offset, a_resolved, b_resolved, addend, signed));
        self.sections[sec_idx].data.extend(std::iter::repeat_n(0, 10));
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
                        target: label.clone(),
                        is_conditional: jump_det.is_conditional,
                        relaxed: true,
                        can_grow: false,
                    });
                } else {
                    let expected_len = if jump_det.is_conditional { 6 } else { 5 };
                    if instr_len == expected_len {
                        self.sections[sec_idx].jumps.push(JumpInfo {
                            offset: base_offset as usize,
                            len: expected_len,
                            target: label.clone(),
                            is_conditional: jump_det.is_conditional,
                            relaxed: false,
                            can_grow: false,
                        });
                    }
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
                patch_size: 4,
            });
        }

        Ok(())
    }

    fn get_jump_target_label(&self, instr: &Instruction) -> Option<String> {
        let mnem = &instr.mnemonic;
        let is_jump = mnem == "jmp" || mnem == "loop"
            || (mnem.starts_with('j') && mnem.len() >= 2);
        if !is_jump { return None; }
        if instr.operands.len() != 1 { return None; }
        if let Operand::Label(label) = &instr.operands[0] {
            Some(label.clone())
        } else {
            None
        }
    }

    // ─── Deferred skip resolution (x86-64 and i686) ──────────────────

    fn resolve_deferred_skips(&mut self) -> Result<(), String> {
        let mut skips = std::mem::take(&mut self.deferred_skips);
        skips.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).reverse());

        for (sec_idx, offset, expr, fill) in &skips {
            // Temporarily insert "." (current position) into label_positions so
            // expressions like "0b + 16 - ." can reference the directive's offset.
            self.label_positions.insert(".".to_string(), (*sec_idx, *offset as u64));
            // Pre-resolve numeric label references (e.g. "0b", "1f") in the expression
            let resolved_expr = self.resolve_numeric_labels_in_expr(expr, *offset as u64, *sec_idx);
            let val = self.evaluate_expr(&resolved_expr);
            self.label_positions.remove(".");
            let val = val?;
            let count = if val < 0 { 0usize } else { val as usize };
            if count == 0 { continue; }

            let fill_bytes: Vec<u8> = vec![*fill; count];
            self.sections[*sec_idx].data.splice(*offset..*offset, fill_bytes);

            // Adjust label positions
            for (_, (lsec, loff)) in self.label_positions.iter_mut() {
                if *lsec == *sec_idx && (*loff as usize) >= *offset {
                    *loff += count as u64;
                }
            }
            for (_, positions) in self.numeric_label_positions.iter_mut() {
                for (lsec, loff) in positions.iter_mut() {
                    if *lsec == *sec_idx && (*loff as usize) >= *offset {
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
        for (skip_sec, skip_off, _, _) in self.deferred_skips.iter_mut() {
            if *skip_sec == sec_idx && *skip_off >= from {
                *skip_off -= delta as usize;
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
            let pos_a = self.label_positions.get(sym_a)
                .ok_or_else(|| format!("undefined label in .uleb128 diff: {}", sym_a))?;
            let pos_b = self.label_positions.get(sym_b)
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
            self.sections[*sec_idx].data.splice(*offset..*offset + PLACE, encoded.iter().copied());
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
            let pos_a = self.label_positions.get(sym_a)
                .ok_or_else(|| format!("undefined label in .byte diff: {}", sym_a))?;
            let pos_b = self.label_positions.get(sym_b)
                .ok_or_else(|| format!("undefined label in .byte diff: {}", sym_b))?;

            if pos_a.0 != pos_b.0 {
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
                while i < bytes.len() && bytes[i].is_ascii_digit() { i += 1; }
                if i < bytes.len() && (bytes[i] == b'b' || bytes[i] == b'f')
                    && (i + 1 >= bytes.len() || !bytes[i + 1].is_ascii_alphanumeric())
                {
                    // This is a numeric label reference like "0b" or "1f"
                    let label_ref = &expr[start..=i];
                    i += 1;
                    if let Some((_, label_off)) = self.resolve_numeric_label(label_ref, offset, sec_idx) {
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
            return Err(format!("unexpected token in expression at position {}: {:?}", pos, tokens.get(pos)));
        }
        Ok(result)
    }

    fn parse_expr_or(&self, tokens: &[ExprToken], pos: &mut usize) -> Result<i64, String> {
        let mut val = self.parse_expr_xor(tokens, pos)?;
        while *pos < tokens.len() {
            match tokens[*pos] {
                ExprToken::Or => { *pos += 1; val |= self.parse_expr_xor(tokens, pos)?; }
                _ => break,
            }
        }
        Ok(val)
    }

    fn parse_expr_xor(&self, tokens: &[ExprToken], pos: &mut usize) -> Result<i64, String> {
        let mut val = self.parse_expr_and(tokens, pos)?;
        while *pos < tokens.len() {
            match tokens[*pos] {
                ExprToken::Xor => { *pos += 1; val ^= self.parse_expr_and(tokens, pos)?; }
                _ => break,
            }
        }
        Ok(val)
    }

    fn parse_expr_and(&self, tokens: &[ExprToken], pos: &mut usize) -> Result<i64, String> {
        let mut val = self.parse_expr_cmp(tokens, pos)?;
        while *pos < tokens.len() {
            match tokens[*pos] {
                ExprToken::And => { *pos += 1; val &= self.parse_expr_cmp(tokens, pos)?; }
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
                ExprToken::Plus => { *pos += 1; val = val.wrapping_add(self.parse_expr_mul(tokens, pos)?); }
                ExprToken::Minus => { *pos += 1; val = val.wrapping_sub(self.parse_expr_mul(tokens, pos)?); }
                _ => break,
            }
        }
        Ok(val)
    }

    fn parse_expr_mul(&self, tokens: &[ExprToken], pos: &mut usize) -> Result<i64, String> {
        let mut val = self.parse_expr_unary(tokens, pos)?;
        while *pos < tokens.len() {
            match tokens[*pos] {
                ExprToken::Star => { *pos += 1; val = val.wrapping_mul(self.parse_expr_unary(tokens, pos)?); }
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
        // Relax long jumps to short form where possible.
        self.relax_jumps();

        // Resolve deferred .skip expressions (x86-64 and i686)
        if A::supports_deferred_skips() {
            self.resolve_deferred_skips()?;
            self.resolve_deferred_byte_diffs()?;
        }

        // Fix alignment/org markers for EVERY section (the jump-relaxation
        // path only does this for sections with jumps; .eh_frame never had
        // its `.align` padding fixed up otherwise).
        for sec_idx in 0..self.sections.len() {
            self.fixup_alignment_markers(sec_idx);
        }

        // Fold scaled symbol differences last: alignment fixups above can
        // still move labels, so the arithmetic must see final positions.
        self.resolve_scaled_diffs()?;

        // Resolve internal relocations
        self.resolve_internal_relocations();

        // Convert to shared ObjSection/ObjSymbol format
        let section_names: Vec<String> = self.sections.iter().map(|s| s.name.clone()).collect();

        let mut shared_sections: FxHashMap<String, ObjSection> = FxHashMap::default();
        for sec in &self.sections {
            let mut data = sec.data.clone();
            let mut relocs = Vec::new();

            for reloc in &sec.relocations {
                let (sym_name, mut addend) = if reloc.symbol.starts_with('.') {
                    if let Some(&(target_sec, target_off)) = self.label_positions.get(&reloc.symbol) {
                        (section_names[target_sec].clone(), reloc.addend + target_off as i64)
                    } else {
                        (reloc.symbol.clone(), reloc.addend)
                    }
                } else {
                    (reloc.symbol.clone(), reloc.addend)
                };

                // Handle symbol-difference relocations (.long a - b)
                if let Some(ref diff_sym) = reloc.diff_symbol {
                    if let Some(&(_b_sec, b_off)) = self.label_positions.get(diff_sym.as_str()) {
                        addend += reloc.offset as i64 - b_off as i64;
                    }
                }

                // For REL format (i686): patch addend into section data
                if A::uses_rel_format() {
                    let off = reloc.offset as usize;
                    if off + 4 <= data.len() {
                        let existing = i32::from_le_bytes([data[off], data[off+1], data[off+2], data[off+3]]);
                        let patched = existing.wrapping_add(addend as i32);
                        data[off..off+4].copy_from_slice(&patched.to_le_bytes());
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

            shared_sections.insert(sec.name.clone(), ObjSection {
                name: sec.name.clone(),
                sh_type: sec.section_type,
                sh_flags: sec.flags,
                data,
                sh_addralign: sec.alignment,
                relocs,
                comdat_group: sec.comdat_group.clone(),
            });
        }

        // Convert label positions
        let labels: FxHashMap<String, (String, u64)> = self.label_positions.iter()
            .map(|(name, &(sec_idx, offset))| {
                (name.clone(), (section_names[sec_idx].clone(), offset))
            })
            .collect();

        let global_symbols: FxHashMap<String, bool> = self.pending_globals.iter()
            .map(|s| (s.clone(), true))
            .collect();
        let weak_symbols: FxHashMap<String, bool> = self.pending_weaks.iter()
            .map(|s| (s.clone(), true))
            .collect();

        let symbol_types: FxHashMap<String, u8> = self.pending_types.iter()
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
        let symbol_sizes: FxHashMap<String, u64> = self.pending_sizes.iter()
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
                        let end_off = self.label_positions.get(end_label).map(|p| p.1).unwrap_or(0);
                        let start_off = self.label_positions.get(start_label).map(|p| p.1).unwrap_or(0);
                        end_off.wrapping_sub(start_off)
                    }
                    SizeExpr::SymbolRef(sym_ref) => {
                        if let Some(alias_target) = self.aliases.get(sym_ref) {
                            let normalized = alias_target.replace(' ', "");
                            if let Some(rest) = normalized.strip_prefix(".-") {
                                if let Some(&(sec_idx, start_off)) = self.label_positions.get(rest) {
                                    let end = self.sections[sec_idx].data.len() as u64;
                                    end - start_off
                                } else { 0 }
                            } else { 0 }
                        } else { 0 }
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

    fn resolve_numeric_label(&self, symbol: &str, reloc_offset: u64, sec_idx: usize) -> Option<(usize, u64)> {
        let len = symbol.len();
        if len < 2 { return None; }
        let suffix = symbol.as_bytes()[len - 1];
        if suffix != b'b' && suffix != b'f' { return None; }
        let label_num = &symbol[..len - 1];
        if !label_num.chars().all(|c| c.is_ascii_digit()) { return None; }

        let positions = self.numeric_label_positions.get(label_num)?;
        if suffix == b'b' {
            let mut best: Option<(usize, u64)> = None;
            for &(s_idx, off) in positions {
                if s_idx == sec_idx && off <= reloc_offset
                    && (best.is_none() || off > best.unwrap().1)
                {
                    best = Some((s_idx, off));
                }
            }
            best
        } else {
            let mut best: Option<(usize, u64)> = None;
            for &(s_idx, off) in positions {
                if s_idx == sec_idx && off > reloc_offset
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
                    let target_off_opt = local_labels
                        .get(&jump.target)
                        .copied()
                        .or_else(|| {
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
                    let (offset, old_len, is_conditional, target) = {
                        let jump = &self.sections[sec_idx].jumps[j_idx];
                        (
                            jump.offset,
                            jump.len,
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
                            for (s_idx, s_off, _, _) in self.deferred_skips.iter_mut() {
                                if *s_idx == sec_idx && *s_off > offset {
                                    *s_off -= shrink;
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
                            let new_len = if is_conditional { 6usize } else { 5usize };
                            let grow = new_len - old_len;
                            let data = &mut self.sections[sec_idx].data;
                            if is_conditional {
                                // short 0x7x disp8 -> long 0x0f 0x8x disp32
                                let cc = data[offset] - 0x70;
                                let insert = vec![0u8; grow];
                                data.splice(offset + 2..offset + 2, insert);
                                data[offset] = 0x0f;
                                data[offset + 1] = 0x80 + cc;
                            } else {
                                // short 0xeb disp8 -> long 0xe9 disp32
                                let insert = vec![0u8; grow];
                                data.splice(offset + 2..offset + 2, insert);
                                data[offset] = 0xE9;
                            }
                            // Re-record the rel32 relocation for the grown jump.
                            let reloc_pos = (if is_conditional {
                                offset + 2
                            } else {
                                offset + 1
                            }) as u64;
                            self.sections[sec_idx].relocations.push(ElfRelocation {
                                offset: reloc_pos,
                                symbol: target.clone(),
                                reloc_type: A::reloc_pc32(),
                                addend: -4,
                                diff_symbol: None,
                                patch_size: 4,
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
                            for (s_idx, s_off, _, _) in self.deferred_skips.iter_mut() {
                                if *s_idx == sec_idx && *s_off > offset {
                                    *s_off += grow;
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
        self.sections[sec_idx].align_markers.sort_by_key(|m| m.offset);

        let is_exec = self.sections[sec_idx].flags & SHF_EXECINSTR != 0;

        let mut marker_idx = 0;
        loop {
            if marker_idx >= self.sections[sec_idx].align_markers.len() {
                break;
            }
            let current_offset = self.sections[sec_idx].align_markers[marker_idx].offset;
            let kind = self.sections[sec_idx].align_markers[marker_idx].kind.clone();

            let needed_end = match &kind {
                AlignMarkerKind::Align(align) => {
                    let a = *align as usize;
                    if a <= 1 { marker_idx += 1; continue; }
                    (current_offset + a - 1) & !(a - 1)
                }
                AlignMarkerKind::Org { label, addend } => {
                    if label.is_empty() {
                        *addend as usize
                    } else if let Some(&(l_sec, l_off)) = self.label_positions.get(label.as_str()) {
                        if l_sec == sec_idx {
                            (l_off as i64 + *addend) as usize
                        } else {
                            marker_idx += 1; continue;
                        }
                    } else {
                        marker_idx += 1; continue;
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
                    self.sections[sec_idx].data.splice(start..old_end, new_bytes);
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
    fn shift_offsets_after(&mut self, sec_idx: usize, at_offset: usize, delta: i64, current_marker_idx: usize) {
        if delta == 0 { return; }
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
        for (skip_sec, skip_off, _, _) in self.deferred_skips.iter_mut() {
            if *skip_sec == sec_idx && *skip_off >= at_offset {
                *skip_off = (*skip_off as i64 + delta) as usize;
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
        if name.starts_with('.') { return true; }
        if name.len() >= 2 {
            let last = name.as_bytes()[name.len() - 1];
            if (last == b'f' || last == b'b') && name[..name.len()-1].chars().all(|c| c.is_ascii_digit()) {
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

    fn resolve_internal_relocations(&mut self) {
        for sec_idx in 0..self.sections.len() {
            let mut resolved: Vec<(usize, i64, usize)> = Vec::new(); // (offset, value, patch_size)
            let mut pc8_patches: Vec<(usize, u8)> = Vec::new();
            let mut unresolved = Vec::new();

            for reloc in &self.sections[sec_idx].relocations {
                // Handle SymbolDiff relocations
                if reloc.diff_symbol.is_some() {
                    if let Some(ref diff_sym) = reloc.diff_symbol {
                        if let (Some(&(a_sec, a_off)), Some(&(b_sec, b_off))) = (
                            self.label_positions.get(&reloc.symbol),
                            self.label_positions.get(diff_sym),
                        ) {
                            if a_sec == b_sec {
                                let val = a_off as i64 - b_off as i64 + reloc.addend;
                                resolved.push((reloc.offset as usize, val, reloc.patch_size as usize));
                                continue;
                            }
                        }
                    }
                    unresolved.push(reloc.clone());
                    continue;
                }

                let label_pos = self.label_positions.get(&reloc.symbol).copied()
                    .or_else(|| self.resolve_numeric_label(&reloc.symbol, reloc.offset, sec_idx));

                if let Some((target_sec, target_off)) = label_pos {
                    let is_local = self.is_local_symbol(&reloc.symbol);

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

                    if target_sec == sec_idx && is_local
                        && (reloc.reloc_type == A::reloc_pc32() || reloc.reloc_type == A::reloc_plt32())
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
