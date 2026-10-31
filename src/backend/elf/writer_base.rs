//! Shared ELF writer base for ARM and RISC-V assembler backends.
//!
//! ARM and RISC-V ELF writers share ~400 lines of identical code for section
//! management, symbol tracking, relocation recording, alignment, directive
//! processing, data emission, and ELF serialization.
//!
//! This `ElfWriterBase` struct captures all of that shared state and logic.
//! Each arch-specific ElfWriter composes with this base and adds its own
//! instruction encoding, branch resolution, and other arch-specific features:
//! - ARM adds `pending_sym_diffs` and AArch64-specific branch resolution
//! - RISC-V adds `pcrel_hi_counter`, `numeric_labels`, and RV64C compression

use super::constants::*;
use super::linker_symbols::default_section_flags;
use super::object_writer::write_relocatable_object;
use super::object_writer::{ElfConfig, ObjReloc, ObjSection};
use super::symbol_table::{ObjSymbol, SymbolTableInput, build_elf_symbol_table};
use crate::backend::asm_expr::{SectionExpr, eval_section_expr, is_bare_symbol};
use crate::common::fx_hash::FxHashMap;

/// Single-directive materialization ceiling (bytes) for generated fill:
/// alignment padding, `.zero`, `.space`, `.fill`.
///
/// These directives amplify: a constant-length source line requests an
/// arbitrary amount of output, so without a bound the assembler's memory
/// (not its correctness) is the first thing an input can exhaust — a 2 GiB
/// VM plus swap dies on `.p2align 32` long before any oracle opinion
/// matters. GNU as materializes against the *filesystem* instead: measured
/// on this box, `.p2align 30` dies on the 993 MiB tmpfs while the same
/// directive succeeds on a 14 GiB runner disk — i.e. past this line the
/// GAS verdict is free-disk state and no test row can pin it (the matrix
/// never encodes such rows). Refusing deterministically is therefore the
/// only *reproducible* verdict: every legitimate pad/fill — the largest
/// alignments real objects use (huge pages, segment alignment) sit four
/// orders of magnitude below this line — behaves identically to GAS on
/// every machine, and pathological input gets a diagnostic instead of the
/// OOM killer.
///
/// 256 MiB is chosen as the largest single allocation that cannot
/// pressure-swap a 2 GiB host on its own while staying far above any real
/// object's directive-driven fill.
pub const MAX_DIRECTIVE_FILL: usize = 256 << 20;

/// The names of a comma-separated symbol-list operand (`.weak a, b`).
fn symbol_list(list: &str) -> impl Iterator<Item = &str> {
    list.split(',').map(str::trim).filter(|n| !n.is_empty())
}

/// Operands of `.comm name, size[, align]` (`allow_align`) or `.lcomm name,
/// size` for every target.  Sizes and alignments are absolute expressions
/// (`.comm x,2*4,1<<3`) as in GAS, and a malformed one is an error -- the
/// per-target parsers used to turn it into size 0 or alignment 1.  The
/// alignment is 0 when omitted (its meaning depends on `.local`, which only
/// the writer knows).  `None` = a negative size, which GAS ignores with a
/// warning.  `.lcomm` takes no alignment on these ELF targets ("junk at end
/// of line").
pub fn parse_comm_args(
    directive: &str,
    args: &str,
    allow_align: bool,
) -> Result<Option<(String, u64, u64)>, String> {
    use crate::backend::asm_expr::parse_integer_expr;
    let parts: Vec<&str> = args.split(',').map(str::trim).collect();
    let max_parts = if allow_align { 3 } else { 2 };
    if parts.len() < 2 || parts.len() > max_parts || parts[0].is_empty() {
        let form = if allow_align {
            "name, size[, align]"
        } else {
            "name, size"
        };
        return Err(format!(
            "bad {directive} directive: expected `{form}', got `{args}'"
        ));
    }
    let size = parse_integer_expr(parts[1]).map_err(|_| {
        format!(
            "{directive}: bad or irreducible absolute expression: {}",
            parts[1]
        )
    })?;
    let align = match parts.get(2) {
        None => 0,
        Some(a) => {
            let v = parse_integer_expr(a)
                .map_err(|_| format!("{directive}: bad or irreducible absolute expression: {a}"))?;
            u64::try_from(v).map_err(|_| format!("{directive}: alignment {v} out of range"))?
        }
    };
    if size < 0 {
        return Ok(None);
    }
    Ok(Some((parts[0].to_string(), size as u64, align)))
}

/// GAS's alignment for a GLOBAL `.comm name, size` with no (or a zero)
/// alignment operand: the smallest power of two that holds the object,
/// capped at 16 (`.comm x,12` -> 16, `.comm x,100` -> 16).  Measured equal
/// on GNU as 2.47 for x86-64, i686 and AArch64 and 2.44 for RISC-V.
pub fn default_comm_align(size: u64) -> u32 {
    size.clamp(1, 16).next_power_of_two() as u32
}

/// GAS's `.lcomm name, size` alignment: the largest power of two not above
/// the size, capped at 8 (`.lcomm y,3` -> 2, `.lcomm y,12` -> 8); the same
/// on every target measured (see `default_comm_align`).
pub fn lcomm_align(size: u64) -> u64 {
    let s = size.clamp(1, 8);
    1u64 << (63 - s.leading_zeros())
}

/// `.base64 "str"[, "str"...]` (GAS 2.42+): RFC 4648 standard alphabet,
/// `=` padding.  Mirrors GAS 2.47: a final group of 2 or 3 characters is
/// accepted without padding (GAS warns), a group of 1, an empty string,
/// whitespace, any other character, or text after padding is an error.
pub fn parse_base64_args(args: &str) -> Result<Vec<u8>, String> {
    fn sextet(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    }
    let mut out = Vec::new();
    let mut rest = args.trim();
    loop {
        let body = rest
            .strip_prefix('"')
            .and_then(|r| r.find('"').map(|end| (&r[..end], &r[end + 1..])));
        let Some((text, tail)) = body else {
            return Err(format!(
                "expected double quote enclosed string as argument to .base64: `{args}'"
            ));
        };
        let bytes = text.as_bytes();
        let data_len = bytes.iter().position(|&c| c == b'=').unwrap_or(bytes.len());
        if bytes[data_len..].iter().any(|&c| c != b'=') {
            return Err(format!(
                "no base64 characters expected after '=' padding: \"{text}\""
            ));
        }
        if data_len % 4 == 1 || data_len == 0 {
            return Err(format!(
                ".base64 string terminated unexpectedly: \"{text}\""
            ));
        }
        if (data_len + (bytes.len() - data_len)) % 4 != 0 && bytes.len() != data_len {
            return Err(format!("bad .base64 padding: \"{text}\""));
        }
        for chunk in bytes[..data_len].chunks(4) {
            let mut acc = 0u32;
            for &c in chunk {
                let v = sextet(c).ok_or_else(|| {
                    format!(
                        "invalid character '{}' found inside .base64 string",
                        c as char
                    )
                })?;
                acc = (acc << 6) | v;
            }
            acc <<= 6 * (4 - chunk.len()) as u32;
            let full = acc.to_be_bytes();
            out.extend_from_slice(&full[1..chunk.len()]);
        }
        rest = tail.trim_start();
        if rest.is_empty() {
            return Ok(out);
        }
        rest = rest
            .strip_prefix(',')
            .map(str::trim_start)
            .filter(|r| !r.is_empty())
            .ok_or_else(|| {
                format!("expected double quote enclosed string as argument to .base64: `{args}'")
            })?;
    }
}

/// Split `.org new-lc [, fill]` operands (one grammar for every target).
///
/// The fill must be a byte value: anything outside 0..=255 is refused rather
/// than silently truncated.
pub fn parse_org_args(args: &str) -> Result<(String, u8), String> {
    match args.split_once(',') {
        Some((lc, f)) => {
            let fill = crate::backend::asm_expr::parse_integer_expr(f.trim())
                .map_err(|_| format!("bad .org fill: {}", f.trim()))?;
            let fill = u8::try_from(fill)
                .map_err(|_| format!(".org fill out of byte range: {}", f.trim()))?;
            Ok((lc.trim().to_string(), fill))
        }
        None => Ok((args.trim().to_string(), 0)),
    }
}

/// Assembly-wide budget for ALL directive-generated fill combined
/// (alignment padding, `.zero`, `.space`, `.fill`, `.org`) — both the
/// ARM/RISC-V [`ElfWriterBase`] and the x86 `ElfWriterCore` enforce it.
///
/// The per-directive ceiling bounds ONE request; without a running
/// budget an input could still spell `N × 256 MiB` across N directives
/// and grow the assembler's RSS until the OOM killer intervenes.  The
/// budget converts that unbounded sum into a deterministic diagnostic.
///
/// 512 MiB (= two maximum single directives) is deliberate: it is
/// comfortably above any legitimate object's directive-driven fill —
/// real padding totals are measured in bytes to low kilobytes across
/// whole kernel-style translation units — while keeping the worst-case
/// assembler RSS at roughly "budget + parsed source" on a 2 GiB/4 GiB
/// (RAM/swap) host, which is the environment the gates run in.  Past
/// this line GAS's own verdict for PROGBITS content is free-disk state
/// anyway (see [`MAX_DIRECTIVE_FILL`]); for NOBITS content GAS would
/// accept anything (no bytes are ever written), but unbounded
/// acceptance here is exactly the memory-amplification hole this
/// budget closes — see the length-only follow-up in
/// `docs/FOLLOWUP-2026-10-05-pr762-fail-closed.md`.
pub const MAX_TOTAL_DIRECTIVE_FILL: usize = 512 << 20;

/// Shared ELF writer state used by both ARM and RISC-V assembler backends.
///
/// This struct manages sections, symbols, labels, and relocations using the
/// shared `ObjSection`/`ObjReloc`/`ObjSymbol` types directly, eliminating the
/// per-arch conversion step in `write_elf()`.
///
/// Architecture-specific ElfWriters compose with this base:
/// - ARM adds `pending_sym_diffs` and AArch64-specific branch resolution
/// - RISC-V adds `pcrel_hi_counter`, `numeric_labels`, and RV64C compression
/// Subsection name -> (parent section, offset of the subsection in it), as
/// returned by [`ElfWriterBase::merge_subsections`].
pub type SubsectionRemap = FxHashMap<String, (String, u64)>;

/// A symbolic base of a captured assembler value ([`SymValue`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprBase {
    /// Offset 0 of a section (or of a not-yet-merged subsection): a local
    /// label is `Sec + offset`, `.` is the place being assembled.
    Sec(String),
    /// The final value of a named symbol: a forward reference, an undefined
    /// (external) symbol, or a global/weak label -- kept by name because GAS
    /// relocates against a global symbol itself, never its section.
    Sym(String),
}

/// A value in GAS's operand-section algebra over [`ExprBase`]s.
pub type SymValue = SectionExpr<ExprBase>;

/// What a `.set`/`.equ` symbol stands for.
#[derive(Clone, Debug)]
pub enum SetValue {
    /// `.set a, b` for a bare symbol `b`: an alias, resolved where it is
    /// used (GAS also copies `b`'s type and size onto `a`, which the symbol
    /// table does for every textual alias).
    Alias(String),
    /// Any other expression, captured where the directive stands: `.` and
    /// the labels already defined have their value at that point, so
    /// `.set x, .+2` is that place plus 2, and a later redefinition of a
    /// symbol does not change what an earlier use assembled to.
    Value(SymValue),
    /// An expression whose operators need values that are still forward
    /// references (`(end - start) >> 2`), evaluated once every label is
    /// known, with `.` the place of the directive and every `.set` symbol
    /// it names worth what it was there (`captured`).
    Text {
        expr: String,
        section: String,
        offset: u64,
        captured: Vec<(String, SymValue)>,
    },
}

/// What a data directive's value assembles to, before a target picks the
/// relocation numbers.  The resolution is GAS's (`expr.c` + `write.c`
/// fixups): differences of symbols in one section are absolute, a single
/// symbol (plus a constant) is an absolute relocation, and `A - B` with B
/// another section's base needs a PC-relative relocation (B the section
/// holding the value) or, on RISC-V, an ADD/SUB pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataFixup {
    Constant(i64),
    /// `sym + addend`.  `sym` names an ELF symbol; a section name stands for
    /// its section symbol.
    Abs {
        sym: String,
        addend: i64,
    },
    /// `add + addend - sub`.
    Diff {
        add: String,
        sub: String,
        addend: i64,
    },
}

impl DataFixup {
    /// The ELF symbols (or section names) the relocation(s) will name.
    pub fn symbols(&self) -> Vec<&str> {
        match self {
            DataFixup::Constant(_) => Vec::new(),
            DataFixup::Abs { sym, .. } => vec![sym],
            DataFixup::Diff { add, sub, .. } => vec![add, sub],
        }
    }

    /// `Diff` as a PC-relative relocation `(symbol, addend)` when `sub` is
    /// the section holding the value at `offset`: `add + addend - S` with
    /// `P = S + offset` is `add + (addend + offset) - P`.
    pub fn pc_relative(&self, section: &str, offset: u64) -> Option<(String, i64)> {
        match self {
            DataFixup::Diff { add, sub, addend } if sub == section => {
                Some((add.clone(), addend.wrapping_add(offset as i64)))
            }
            _ => None,
        }
    }
}

/// Append `padding` bytes of code fill (GAS's fill law, see
/// [`ElfWriterBase::align_to_capped_ex`]): NOPs on their natural grid,
/// the bytes that cannot host one filled from the front with the largest
/// encoding that fits.
fn push_code_fill(
    data: &mut Vec<u8>,
    padding: usize,
    nop: &[u8; 4],
    small_nop: Option<([u8; 2], u64)>,
) {
    let start = data.len() as u64;
    let end = start + padding as u64;
    let mut pos = start;
    while pos < end {
        let room = end - pos;
        if pos % 4 == 0 && room >= 4 {
            data.extend_from_slice(nop);
            pos += 4;
        } else if let Some((sb, ss)) = small_nop
            && pos % ss == 0
            && room >= ss
        {
            data.extend_from_slice(&sb);
            pos += ss;
        } else {
            data.push(0);
            pos += 1;
        }
    }
    debug_assert_eq!(pos, end, "fill law must cover the pad exactly");
}

/// A data value that could not be assembled where it stands.
#[derive(Clone, Debug)]
pub struct PendingData {
    pub section: String,
    pub offset: u64,
    pub size: usize,
    pub value: SetValue,
}

pub struct ElfWriterBase {
    /// Current section we're emitting into
    pub current_section: String,
    /// All sections being built (using shared ObjSection directly)
    pub sections: FxHashMap<String, ObjSection>,
    /// Section order (for deterministic output)
    pub section_order: Vec<String>,
    /// Extra symbols (e.g., COMMON symbols from .comm directives)
    pub extra_symbols: Vec<ObjSymbol>,
    /// Local labels -> (section, offset) for branch resolution
    pub labels: FxHashMap<String, (String, u64)>,
    /// Symbols that have been declared .globl
    pub global_symbols: FxHashMap<String, bool>,
    /// Symbols declared .weak
    pub weak_symbols: FxHashMap<String, bool>,
    /// Symbols named by `.local` -- GAS's sticky per-symbol flag: a later
    /// `.comm` allocates them in `.bss` as LOCAL objects.
    local_flagged: crate::common::fx_hash::FxHashSet<String>,
    /// Global `.comm` symbols as (name, size, alignment), in declaration
    /// order; `write_elf` emits them unless a definition replaced them.
    commons: Vec<(String, u64, u64)>,
    /// `.lcomm` and `.local`+`.comm` allocations as (name, size, alignment);
    /// `allocate_local_commons` places them at the end of `.bss`.
    local_commons: Vec<(String, u64, u64)>,
    /// Symbol types from .type directives
    pub symbol_types: FxHashMap<String, u8>,
    /// Symbol sizes from .size directives
    pub symbol_sizes: FxHashMap<String, u64>,
    /// Symbol visibility from .hidden/.protected/.internal
    pub symbol_visibility: FxHashMap<String, u8>,
    /// Symbol aliases from .set/.equ directives -- the symbol table's view:
    /// a bare-symbol alias or an integer (an ABS symbol).  `sets` is the
    /// assembler's view, from which these are derived.
    pub aliases: FxHashMap<String, String>,
    /// Every `.set`/`.equ` symbol's value (see [`SetValue`]).
    pub sets: FxHashMap<String, SetValue>,
    /// Data values deferred to [`Self::resolve_pending_data`].
    pub pending_data: Vec<PendingData>,
    /// Data values name labels of executable sections symbolically: a
    /// difference of two such labels is an ADD/SUB pair and a lone one is
    /// relocated against by name, never folded to a constant or rewritten to
    /// section+offset.  RISC-V with linker relaxation (GNU as: no local
    /// subtraction across relaxable code, `tc_fix_adjustable` 0 for code),
    /// where the linker deletes bytes between labels after assembly.  Read
    /// when data values are captured and again when they are resolved, so a
    /// target may change it before [`Self::resolve_pending_data`] (GNU as
    /// decides by the relax state at the end of the input).  Off for every
    /// other target.
    pub keep_code_labels: bool,
    /// Evaluation mode: [`Self::keep_code_labels`] is in force for the data
    /// value being captured or resolved right now.  Set only around those two
    /// entry points, so `.set`, `.org`, `.fill` and the rest keep folding
    /// code-label differences to constants exactly as GNU as does.
    code_labels_symbolic: std::cell::Cell<bool>,
    /// The synthetic label standing for `.` in the data value being captured
    /// (GNU as's `.L0 `), when `.` is a code location that must stay symbolic.
    dot_label: Option<String>,
    /// Counter for [`Self::dot_label`] names.
    dot_labels: u32,
    /// Section stack for .pushsection/.popsection (saves both current and previous section)
    section_stack: Vec<(String, String)>,
    /// Previous section for .section/.previous swapping
    previous_section: String,
    /// NOP instruction bytes for code section alignment padding.
    /// ARM: `[0x1f, 0x20, 0x03, 0xd5]` (d503201f), RISC-V: `[0x13, 0x00, 0x00, 0x00]` (00000013)
    nop_bytes: [u8; 4],
    /// A half-width NOP the target may use for the bytes a full NOP cannot
    /// cover at its own alignment, as `Some((bytes, size))`: RISC-V `c.nop`
    /// (`0x0001`, 2 bytes), and `None` for every target without a 2-byte
    /// instruction encoding, where the prefix stays zeros.
    ///
    /// A target that has one offers it *unconditionally*; it is not gated on the
    /// march.  Measured on GNU as 2.47, `-march=rv64im` pads `.p2align` exactly
    /// like `rv64gc` — including the 2-byte `c.nop` — because the fill lands
    /// where control flow never goes, so GAS does not gate it on RVC.  Coupling
    /// it to `EF_RISCV_RVC` was a guess the oracle disproves (see the call site
    /// in `backend/riscv/assembler/elf_writer.rs`).
    ///
    /// This exists because GAS's fill is *not* "zeros then NOPs" or "NOPs then
    /// zeros" but "each NOP at its own alignment, largest last", so a 7-byte
    /// pad at offset 1 on rv64gc is `00 01 00 13 00 00 00` — zero, `c.nop`,
    /// `nop` — and a table with one entry cannot spell that.
    small_nop: Option<([u8; 2], u64)>,
    /// Smallest alignment an alignment directive records for an executable
    /// section, or 0 for no floor.  It raises `sh_addralign` only: the
    /// directive pads to what it asked for.  RISC-V sets its instruction
    /// alignment, 2 with RVC and 4 without (GNU as 2.47, measured with the
    /// march spelled out: `.byte 1; .p2align 0,,3; .byte 2` is `01 02` at
    /// addralign 2 for rv64gc and `01 02` -- plus a 2-byte tail -- at
    /// addralign 4 for rv64imafd; `.p2align 1` puts the next byte at offset
    /// 2 in both).  An earlier reading took the tail GNU as pads a code
    /// section with for padding of the directive, and was measured under a
    /// default march without C, so it raised the directive itself to 4.
    /// `.data` keeps what was asked for, and AArch64 has no floor at all.
    code_align_floor: u64,
    /// The alignment an *instruction* raises the current section to, not the
    /// alignment `.text` is created with: 4 for ARM/AArch64, 2 for RISC-V (the
    /// width of the narrowest instruction the target's law admits -- with the C
    /// extension GAS gives `.text` 2 even for `.byte 1` alone, and 4 at
    /// `-march=rv64im`, measured on riscv64-linux-gnu-as; lccc's own creation
    /// value stays 1 until `note_instruction` runs; this is a deliberate
    /// divergence for instruction-free `.text`).
    text_align: u64,
    /// Bytes of directive-generated fill materialized so far by this
    /// writer — bounded by [`MAX_TOTAL_DIRECTIVE_FILL`] (see there).
    fill_budget_used: usize,
    /// `.file N "name"` records in directive order.
    debug_files: Vec<(u32, String)>,
    /// `.loc` rows as (label at the row's address, file, line, column): a
    /// label, so subsection merging moves the row with its code.
    debug_rows: Vec<(String, u32, u32, u32, super::dwarf::DwarfLocOptions)>,
}

impl ElfWriterBase {
    /// See [`Self::small_nop`].
    pub fn with_small_nop(mut self, bytes: [u8; 2]) -> Self {
        self.small_nop = Some((bytes, 2));
        self
    }

    /// See [`Self::code_align_floor`]; for a target whose instruction
    /// alignment changes mid-input (RISC-V `.option rvc`/`norvc`).
    pub fn set_code_align_floor(&mut self, floor: u64) {
        self.code_align_floor = floor;
    }

    /// The alignment an instruction records for its section (the `new`
    /// argument), for a target whose instruction size changes mid-input.
    pub fn set_text_align(&mut self, align: u64) {
        self.text_align = align;
    }

    /// See [`Self::code_align_floor`].
    pub fn with_code_align_floor(mut self, floor: u64) -> Self {
        self.code_align_floor = floor;
        self
    }

    pub fn new(nop_bytes: [u8; 4], text_align: u64) -> Self {
        Self {
            current_section: String::new(),
            sections: FxHashMap::default(),
            section_order: Vec::new(),
            extra_symbols: Vec::new(),
            labels: FxHashMap::default(),
            global_symbols: FxHashMap::default(),
            weak_symbols: FxHashMap::default(),
            local_flagged: Default::default(),
            commons: Vec::new(),
            local_commons: Vec::new(),
            symbol_types: FxHashMap::default(),
            symbol_sizes: FxHashMap::default(),
            symbol_visibility: FxHashMap::default(),
            aliases: FxHashMap::default(),
            sets: FxHashMap::default(),
            pending_data: Vec::new(),
            keep_code_labels: false,
            code_labels_symbolic: std::cell::Cell::new(false),
            dot_label: None,
            dot_labels: 0,
            section_stack: Vec::new(),
            previous_section: String::new(),
            nop_bytes,
            small_nop: None,
            code_align_floor: 0,
            text_align,
            fill_budget_used: 0,
            debug_files: Vec::new(),
            debug_rows: Vec::new(),
        }
    }

    /// Ensure a section exists. If it doesn't, create it with the given properties.
    pub fn ensure_section(&mut self, name: &str, sh_type: u32, sh_flags: u64, align: u64) {
        if !self.sections.contains_key(name) {
            self.sections.insert(
                name.to_string(),
                ObjSection {
                    name: name.to_string(),
                    sh_type,
                    sh_flags,
                    data: Vec::new(),
                    // NOBITS is held as a pure size from birth: nothing but
                    // zeros can enter it (`nobits_guard`, and relocations are
                    // refused), so there is never a byte worth storing.
                    sh_size: (sh_type == SHT_NOBITS).then_some(0),
                    sh_addralign: align,
                    relocs: Vec::new(),
                    comdat_group: None,
                },
            );
            self.section_order.push(name.to_string());
        }
    }

    /// `.file N "name"`; a later one for the same `N` renames it.
    pub fn debug_file(&mut self, number: u32, name: String) {
        self.debug_files.push((number, name));
    }

    /// `.loc FILE LINE COLUMN`: a line row at the current location (in
    /// `.text` before any section directive, as in GNU as).
    pub fn debug_loc(
        &mut self,
        file: u32,
        line: u32,
        column: u32,
        options: super::dwarf::DwarfLocOptions,
    ) {
        self.ensure_text_section();
        let label = format!(".Lloc\u{1}{}", self.debug_rows.len());
        let at = (self.current_section.clone(), self.current_offset());
        self.labels.insert(label.clone(), at);
        self.debug_rows.push((label, file, line, column, options));
    }

    /// GNU as `dwarf2_finish`: the `.loc` rows become a DWARF v4
    /// `.debug_line`, and an input without its own `.debug_info` gets compile
    /// units for it (see `elf::dwarf`).  `abs32` and `abs_addr` are the
    /// 32-bit and `address_size`-byte absolute relocations.  Under
    /// [`Self::keep_code_labels`] (RISC-V linker relaxation) every code
    /// address is relocated against a label at it, which the linker moves
    /// when it deletes bytes.  Call after the last section change and before
    /// [`Self::write_elf`].
    pub fn finish_debug_sections(
        &mut self,
        address_size: usize,
        abs32: u32,
        abs_addr: u32,
    ) -> Result<(), String> {
        use super::dwarf::{
            DwarfFunction, DwarfLineLocation, DwarfSectionInfo, build_dwarf_info_sections,
            build_dwarf_line_section,
        };
        if self.debug_rows.is_empty() || self.sections.contains_key(".debug_line") {
            return Ok(());
        }
        let index_of: FxHashMap<&str, usize> = self
            .section_order
            .iter()
            .enumerate()
            .map(|(i, name)| (name.as_str(), i))
            .collect();
        let section_info: Vec<DwarfSectionInfo<'_>> = self
            .section_order
            .iter()
            .map(|name| {
                let section = self.sections.get(name);
                DwarfSectionInfo {
                    name,
                    size: section.map_or(0, ObjSection::size),
                    is_executable: section.is_some_and(|s| s.sh_flags & SHF_EXECINSTR != 0),
                }
            })
            .collect();
        // The row labels answer `symbol_at` for their own addresses.
        let mut at_labels: FxHashMap<(usize, u64), String> = FxHashMap::default();
        let mut locations = Vec::with_capacity(self.debug_rows.len());
        for (sequence, (label, file, line, column, options)) in (0u64..).zip(&self.debug_rows) {
            let Some((section, offset)) = self.labels.get(label) else {
                continue;
            };
            let Some(&section_index) = index_of.get(section.as_str()) else {
                continue;
            };
            at_labels
                .entry((section_index, *offset))
                .or_insert_with(|| label.clone());
            locations.push(DwarfLineLocation {
                section_index,
                offset: *offset,
                file_number: *file,
                line: *line,
                column: *column,
                options: *options,
                sequence,
            });
        }
        let functions: Vec<DwarfFunction<'_>> = self
            .symbol_types
            .iter()
            .filter(|&(_, &sym_type)| sym_type == STT_FUNC || sym_type == STT_GNU_IFUNC)
            .filter_map(|(name, _)| {
                let (section, offset) = self.labels.get(name)?;
                Some(DwarfFunction {
                    name,
                    section_index: *index_of.get(section.as_str())?,
                    offset: *offset,
                    size: self.symbol_sizes.get(name).copied().unwrap_or(0),
                    external: self.global_symbols.contains_key(name)
                        || self.weak_symbols.contains_key(name),
                })
            })
            .collect();
        let primary_file = self
            .debug_files
            .iter()
            .find(|(number, _)| *number == 1)
            .or_else(|| self.debug_files.first())
            .map_or("", |(_, name)| name.as_str());
        let comp_dir = std::env::current_dir()
            .map(|dir| dir.to_string_lossy().into_owned())
            .unwrap_or_default();

        let first_new_label = at_labels.len();
        let mut symbol_at = |section_index: usize, offset: u64| -> String {
            let next = at_labels.len() - first_new_label;
            at_labels
                .entry((section_index, offset))
                .or_insert_with(|| format!(".Lloc\u{1}end{next}"))
                .clone()
        };
        let relax = self.keep_code_labels;
        let mut labelled: Option<&mut dyn FnMut(usize, u64) -> String> =
            if relax { Some(&mut symbol_at) } else { None };
        let mut built = Vec::new();
        if let Some((line, units)) = build_dwarf_line_section(
            &self.debug_files,
            &locations,
            &section_info,
            address_size,
            abs_addr,
            labelled.as_deref_mut(),
        )? {
            built.push(line);
            if !self.sections.contains_key(".debug_info")
                && !self.sections.contains_key(".debug_abbrev")
            {
                if let Some((info, abbrev)) = build_dwarf_info_sections(
                    &units,
                    &section_info,
                    &functions,
                    primary_file,
                    &comp_dir,
                    concat!("LCCC ", env!("CARGO_PKG_VERSION")),
                    address_size,
                    abs32,
                    abs_addr,
                    labelled,
                )? {
                    built.extend([info, abbrev]);
                }
            }
        }
        // Labels `symbol_at` made up (section ends, function ends).
        let made: Vec<(String, (String, u64))> = if relax {
            at_labels
                .into_iter()
                .filter(|(_, label)| label.starts_with(".Lloc\u{1}end"))
                .map(|((section_index, offset), label)| {
                    (label, (self.section_order[section_index].clone(), offset))
                })
                .collect()
        } else {
            Vec::new()
        };
        self.labels.extend(made);
        for section in built {
            self.section_order.push(section.name.clone());
            self.sections.insert(section.name.clone(), section);
        }
        Ok(())
    }

    /// Get the current write offset within the current section.
    pub fn current_offset(&self) -> u64 {
        self.sections
            .get(&self.current_section)
            .map(ObjSection::size)
            .unwrap_or(0)
    }

    /// GAS: content emitted before any section directive lands in `.text`
    /// (measured: a leading `.zero 4` assembles into a 5-byte `.text`).
    /// Every emitting entry point funnels through this so a missing
    /// section silently drops bytes exactly once, here, into `.text`.
    pub fn ensure_default_section(&mut self) {
        if self.current_section.is_empty() {
            self.ensure_text_section();
        }
    }

    /// GAS 2.47 fail-closed rule for `SHT_NOBITS` sections (measured):
    /// a *store* of a non-zero value — `.byte 1`, `.short 256`,
    /// `.long 1<<32` (checked as the RAW value, before truncation), a
    /// relocation placeholder, a non-zero encoded LEB — is rejected with
    /// "attempt to store non-zero value in section `X'", because the
    /// content would silently vanish from the NOBITS output.  Position
    /// directives (`.zero`, `.space`, `.org`, `.p2align` — even with a
    /// non-zero pad byte) and all-zero stores are legal and never reach
    /// this guard.
    pub fn nobits_guard(&self, raw_nonzero: bool) -> Result<(), String> {
        if raw_nonzero {
            if let Some(section) = self.sections.get(&self.current_section)
                && section.sh_type == SHT_NOBITS
            {
                return Err(format!(
                    "attempt to store non-zero value in section `{}`",
                    section.name
                ));
            }
        }
        Ok(())
    }

    /// GAS 2.47 string law inside NOBITS (measured): `.ascii "ab"` is
    /// "attempt to store non-empty string in section `X'", while the
    /// empty string and an all-zero byte string are legal (they change
    /// nothing a NOBITS section will ever carry).
    pub fn nobits_string_guard(&self, bytes: &[u8]) -> Result<(), String> {
        // GAS gives string stores their own wording (measured on both
        // targets): "attempt to store non-empty string in section `X'" —
        // empty and all-zero strings remain legal.
        if bytes.iter().any(|&b| b != 0)
            && let Some(section) = self.sections.get(&self.current_section)
            && section.sh_type == SHT_NOBITS
        {
            return Err(format!(
                "attempt to store non-empty string in section `{}`",
                section.name
            ));
        }
        Ok(())
    }

    /// Charge `n` bytes of directive-generated fill against the
    /// assembly-wide [`MAX_TOTAL_DIRECTIVE_FILL`] budget.  Every
    /// materializing path (fill, alignment padding, `.org`) calls this
    /// BEFORE touching memory, so the running sum — not just each
    /// single directive — is bounded.
    pub fn charge_fill(&mut self, n: usize) -> Result<(), String> {
        let Some(total) = self.fill_budget_used.checked_add(n) else {
            return Err(format!(
                "directive fill budget of {MAX_TOTAL_DIRECTIVE_FILL} bytes exceeded ({n} more requested)"
            ));
        };
        if total > MAX_TOTAL_DIRECTIVE_FILL {
            return Err(format!(
                "directive fill budget of {MAX_TOTAL_DIRECTIVE_FILL} bytes exceeded ({total} total, {n} more requested)"
            ));
        }
        self.fill_budget_used = total;
        Ok(())
    }

    /// Append raw bytes to the current section.
    ///
    /// Returns `Err` when the content stores non-zero bytes into a
    /// `SHT_NOBITS` section (GAS 2.47 rejects that — see
    /// [`Self::nobits_guard`]); every caller must propagate, which is
    /// what makes the NOBITS rule impossible to bypass by accident.
    pub fn emit_bytes(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.ensure_default_section();
        self.nobits_guard(bytes.iter().any(|&b| b != 0))?;
        if let Some(section) = self.sections.get_mut(&self.current_section) {
            if section.sh_type == SHT_NOBITS {
                // The guard proved these bytes zero: they are a size.
                section.grow_nobits(bytes.len() as u64)?;
            } else {
                section.data.extend_from_slice(bytes);
            }
        }
        Ok(())
    }

    /// Append `size` copies of `fill`, bounded by [`MAX_DIRECTIVE_FILL`].
    ///
    /// This is the `.zero` / `.space` / `.fill` choke point. The previous
    /// path built an intermediate `Vec` and copied it into the section —
    /// two allocations and two full touches per directive — and had no
    /// ceiling at all: a six-byte source line could ask for 2^63 bytes
    /// and take the assembler down with the OOM killer. One `resize`
    /// writes once, and the ceiling turns the unbounded request into a
    /// deterministic diagnostic.
    ///
    /// In a NOBITS section the run is a size whatever `fill` is (GAS 2.47
    /// stores nothing for `.skip 3,1` there), so neither the ceiling nor the
    /// budget applies: they bound memory, and none is used.
    pub fn emit_fill(&mut self, size: usize, fill: u8) -> Result<(), String> {
        self.ensure_default_section();
        if let Some(section) = self.sections.get_mut(&self.current_section)
            && section.sh_type == SHT_NOBITS
        {
            return section.grow_nobits(size as u64);
        }
        if size > MAX_DIRECTIVE_FILL {
            return Err(format!(
                "fill directive of {size} bytes exceeds the {MAX_DIRECTIVE_FILL}-byte limit"
            ));
        }
        // Budget first: an over-budget request dies BEFORE the resize.
        self.charge_fill(size)?;
        if let Some(section) = self.sections.get_mut(&self.current_section) {
            let start = section.data.len();
            section.data.resize(start + size, fill);
        }
        Ok(())
    }

    /// Materialize a `.fill` pattern with a NON-ZERO value — the single
    /// choke point for the arm/riscv `.fill` non-zero branches.
    ///
    /// Enforces, in order:
    /// 1. the NOBITS *raw-value* law (`.fill 4,1,256` in `.bss` is an
    ///    error even though the truncated element stores zeros — the
    ///    check sees `value`, not the encoded bytes); a zero pattern in
    ///    NOBITS is a size, with no ceiling and no budget,
    /// 2. the per-directive ceiling (before any allocation),
    /// 3. the assembly-wide fill budget,
    /// 4. one allocation + a tight slice-copy loop.
    pub fn emit_fill_pattern(
        &mut self,
        total: usize,
        elem: usize,
        value: u64,
    ) -> Result<(), String> {
        debug_assert!((1..=8).contains(&elem), "parser clamps elem to 1..=8");
        self.ensure_default_section();
        if let Some(section) = self.sections.get_mut(&self.current_section)
            && section.sh_type == SHT_NOBITS
        {
            if value != 0 {
                return Err(format!(
                    "attempt to fill section `{}` with non-zero value",
                    section.name
                ));
            }
            return section.grow_nobits(total as u64);
        }
        if total > MAX_DIRECTIVE_FILL {
            return Err(format!(
                "fill directive of {total} bytes exceeds the {MAX_DIRECTIVE_FILL}-byte limit"
            ));
        }
        self.charge_fill(total)?;
        if total == 0 {
            return Ok(());
        }
        let pattern = value.to_le_bytes();
        if let Some(section) = self.sections.get_mut(&self.current_section) {
            let start = section.data.len();
            section.data.resize(start + total, 0);
            for off in (0..total).step_by(elem) {
                section.data[start + off..start + off + elem].copy_from_slice(&pattern[..elem]);
            }
        }
        Ok(())
    }

    /// `.org new-lc [, fill]` -- pad the current section with `fill` up to
    /// byte offset `new-lc` from its start (GAS semantics), shared by every
    /// `ElfWriterBase` target.
    ///
    /// Evaluated immediately: padding changes every offset that follows it,
    /// so it cannot be deferred the way `.word` expressions can. `new-lc` is
    /// any expression [`Self::org_target`] resolves; forward references are
    /// errors. Backward targets error too ("attempt to move .org
    /// backwards") -- a silent no-op would corrupt vector tables and the
    /// kernel's ALTERNATIVE length checks, which are exactly a pair of
    /// `.org`s that must fail when a replacement outgrows the original.
    pub fn emit_org(&mut self, expr: &str, fill: u8) -> Result<(), String> {
        // GAS: `.org` before any section directive positions within `.text`.
        self.ensure_default_section();
        let current = self.current_offset();
        let target = self.org_target(expr)?;
        if target < current {
            return Err(format!(
                ".org: attempt to move .org backwards (to byte {target} from byte {current})"
            ));
        }
        let padding = usize::try_from(target - current)
            .map_err(|_| format!(".org: byte {target} exceeds the addressable size"))?;
        // `.org` is the same constant-source / unbounded-output amplifier as
        // `.zero`: refuse a materialization before touching memory (GAS dies
        // writing it too -- nonzero exit either way). A NOBITS pad is a size,
        // so nothing is materialised and no ceiling applies.
        let nobits = self
            .sections
            .get(&self.current_section)
            .is_some_and(|s| s.sh_type == SHT_NOBITS);
        if !nobits && padding > MAX_DIRECTIVE_FILL {
            return Err(format!(
                ".org: {padding} bytes of fill exceed the {MAX_DIRECTIVE_FILL}-byte limit"
            ));
        }
        // `.org` pads with its fill byte (default ZERO), even in an
        // executable section -- it is a positioning directive, not an
        // alignment one (GAS emits plain fill bytes too).
        self.emit_fill(padding, fill)
    }

    /// Resolve a `.org` target to a byte offset in the current section.
    ///
    /// The expression is evaluated in GAS's operand-section algebra
    /// ([`crate::backend::asm_expr::eval_section_expr`]). These targets never
    /// relax at assembly time, so `.` and every label of the CURRENT section
    /// already have their final offset from the section start -- exactly
    /// what a `.org` target is -- and enter as plain numbers (GAS folds them
    /// the same way: `.org L*2+4` is accepted). A label of ANY OTHER section
    /// is a link-time address and must cancel out (the difference of two
    /// labels in one section is absolute). That accepts `.org . + 16`,
    /// `.org .Lventry_start + 128`, and the kernel's ALTERNATIVE checks
    /// `.org . - (889b - 888b) + (887b - 886b)`, and refuses `.org T + 4`
    /// for a label `T` in another section, as GAS does.
    pub fn org_target(&self, expr: &str) -> Result<u64, String> {
        use crate::backend::asm_expr::{SectionExpr, eval_section_expr, parse_integer_expr};
        fn resolve(
            base: &ElfWriterBase,
            name: &str,
            depth: u32,
        ) -> Result<SectionExpr<String>, String> {
            if name == "." {
                return Ok(SectionExpr::absolute(base.current_offset() as i64));
            }
            if let Some((sec, off)) = base.labels.get(name) {
                return Ok(if *sec == base.current_section {
                    SectionExpr::absolute(*off as i64)
                } else {
                    SectionExpr::at(sec.clone(), *off as i64)
                });
            }
            if let Some(target) = base.aliases.get(name)
                && depth < 16
            {
                if let Ok(v) = parse_integer_expr(target) {
                    return Ok(SectionExpr::absolute(v));
                }
                return resolve(base, target.trim(), depth + 1);
            }
            Err(format!(
                ".org: unknown or forward-referenced symbol '{name}' (padding cannot be deferred, so forward references are unsupported)"
            ))
        }
        let value = eval_section_expr(expr, |name| resolve(self, name, 0))
            .map_err(|e| format!(".org {expr}: {e}"))?;
        if !value.is_absolute() {
            return Err(format!(
                ".org {expr}: the target is not an offset in section '{}'",
                self.current_section
            ));
        }
        u64::try_from(value.constant)
            .map_err(|_| format!(".org {expr}: negative target {}", value.constant))
    }

    /// Append a 16-bit little-endian value to the current section.
    pub fn emit_u16_le(&mut self, val: u16) -> Result<(), String> {
        self.emit_bytes(&val.to_le_bytes())
    }

    /// Append a 32-bit little-endian value to the current section.
    pub fn emit_u32_le(&mut self, val: u32) -> Result<(), String> {
        self.emit_bytes(&val.to_le_bytes())
    }

    /// Record a relocation at the current offset in the current section.
    pub fn add_reloc(&mut self, reloc_type: u32, symbol: String, addend: i64) {
        let offset = self.current_offset();
        let section = self.current_section.clone();
        if let Some(s) = self.sections.get_mut(&section) {
            s.relocs.push(ObjReloc {
                offset,
                reloc_type,
                symbol_name: symbol,
                addend,
            });
        }
    }

    /// Align the current section's data to the specified byte boundary.
    ///
    /// Code sections are NOP-padded using the architecture's NOP instruction;
    /// data sections are zero-padded.
    pub fn align_to(&mut self, align: u64) -> Result<(), String> {
        self.align_to_capped(align, None)
    }

    /// Align, but skip the padding entirely when it would exceed `max_pad`.
    ///
    /// This is GAS `.p2align N,,M`: if honouring the alignment would insert
    /// more than M bytes, the location counter is left unchanged. The section
    /// `sh_addralign` is still raised so the ELF header reflects the
    /// programmer's requested alignment even when a particular site skipped.
    pub fn align_to_capped(&mut self, align: u64, max_pad: Option<u64>) -> Result<(), String> {
        self.align_to_capped_ex(align, max_pad, None)
    }

    /// GAS `.p2align N[, fill[, max]]`.
    ///
    /// When `fill` is `Some`, every pad byte is that value in every section
    /// (`.text` included: `.p2align 3, 0xff` is 0xff, not NOP). When `fill`
    /// is `None`, an executable section is filled with the target's NOPs —
    /// each placed only where its own size permits it to be fetched, so the
    /// bytes before the first legal NOP slot are zeros (or, on RISC-V, the
    /// 2-byte `c.nop`) and the NOPs end the run flush against the boundary —
    /// while other sections zero-fill. That is `riscv_make_nops` and its
    /// aarch64 equivalent, byte for byte.
    ///
    /// The run then stops at the boundary, which is *not* what GNU as 2.47
    /// emits on RISC-V: at offset 5, `.p2align 5` pads 16 bytes here and 48
    /// there, because `riscv_frag_align_code` writes a worst-case
    /// `align - insn_alignment` NOP run plus a `BFD_RELOC_RISCV_ALIGN` fixup
    /// and leaves the *linker* to relax the run to the true excess. Copying
    /// the sizes without copying the relaxation would keep up to a whole
    /// alignment quantum of dead bytes in front of every aligned label of
    /// every RISC-V object LCCC writes — in exactly the hot code the directive
    /// was written to place — since LCCC's linker documents `R_RISCV_ALIGN` as
    /// a hint it skips. Padding to the boundary is what a relaxed link
    /// produces, so it is what LCCC produces; the divergent sizes are recorded
    /// (with the measurement) by `scripts/gas_fill_census.py`, and the pad
    /// *content* law that does apply to the final image is matched exactly.
    ///
    /// Two refusal rules keep this function fail-closed without ever
    /// touching an unbounded allocation (all verdicts measured on GNU as
    /// 2.47.20260726):
    ///
    /// * The *signed-shift bucket* (`align >= 1<<63`, what `.p2align 63`,
    ///   `.p2align 64`, `.p2align -1` and clamped forms all reach): GAS
    ///   never pads from it and never records the alignment. A positive
    ///   max-pad skips (the would-be padding is "infinite" and therefore
    ///   exceeds any cap), a zero location counter is a no-op, and a
    ///   nonzero location counter without a cap is an error — measured
    ///   `p2align 63 @0` -> addralign 1, `byte 1; p2align 63` -> error,
    ///   `byte 1; p2align 63,,5` -> skip, addralign 1.  Remeasured on
    ///   GAS 2.47 across `.text`, `.data` and custom `ax`/`w` sections
    ///   (both pinned targets): the verdict never depends on section
    ///   kind — a positive-cap skip assembles everywhere, an uncapped
    ///   pad at a nonzero offset errors everywhere (free-state on
    ///   aarch64, semantic on x86-64 exec sections).
    /// * Materialized padding beyond [`MAX_DIRECTIVE_FILL`] is refused
    ///   before a single byte is touched (see the constant for why the
    ///   verdict beyond that point is not pinnable anyway).
    pub fn align_to_capped_ex(
        &mut self,
        align: u64,
        max_pad: Option<u64>,
        fill: Option<u8>,
    ) -> Result<(), String> {
        self.align_impl(align, max_pad, fill, true)
    }

    /// Pad the current section to `align` with code fill, ignoring
    /// [`Self::code_align_floor`]: RISC-V's step to the instruction alignment
    /// in front of an `R_RISCV_ALIGN` run (GNU as 2.44, measured: at offset 5
    /// the run starts at 6, after one zero byte), which is not a directive's
    /// alignment and so is not subject to the directive floor.
    pub fn align_code_step(&mut self, align: u64) -> Result<(), String> {
        self.align_impl(align, None, None, false)
    }

    fn align_impl(
        &mut self,
        align: u64,
        max_pad: Option<u64>,
        fill: Option<u8>,
        floor: bool,
    ) -> Result<(), String> {
        self.ensure_default_section();
        // The floor is recorded before the `align <= 1` no-op test: a
        // `.p2align 0` in code pads nothing but still raises the section's
        // alignment (see [`Self::code_align_floor`]).
        let floor_align = if floor && self.is_exec_section(&self.current_section.clone()) {
            self.code_align_floor
        } else {
            0
        };
        if align <= 1 {
            if let Some(section) = self.sections.get_mut(&self.current_section) {
                section.sh_addralign = section.sh_addralign.max(floor_align);
            }
            return Ok(());
        }
        let nop = self.nop_bytes.clone();
        let small_nop = self.small_nop;
        // Phase 1 — decide under a single shared borrow (no allocation).
        let (decided, nobits) = {
            let Some(section) = self.sections.get(&self.current_section) else {
                return Ok(());
            };
            let nobits = section.sh_type == SHT_NOBITS;
            let current = section.size();
            if align >= 1u64 << 63 {
                let cap_positive = max_pad.is_some_and(|m| m > 0);
                if cap_positive || current == 0 {
                    // Skip / no-op: GAS records neither padding nor an
                    // alignment for this bucket (addralign stays 1).
                    return Ok(());
                }
                return Err(format!(
                    "alignment {align} would require padding at nonzero offset"
                ));
            }
            let aligned = (current + align - 1) & !(align - 1);
            let padding = (aligned - current) as usize;
            // GAS (2.47, measured): skip iff the cap is *positive* and
            // padding would exceed it. `,,0` and a negative cap mean
            // unlimited — `.p2align 4,,0` at offset 1 pads 15 bytes.
            let skip = max_pad.is_some_and(|m| m > 0 && (padding as u64) > m);
            let padding = if skip { 0 } else { padding };
            // A NOBITS pad is a size: the ceiling and budget bound memory,
            // and none is used.
            if !nobits && padding > MAX_DIRECTIVE_FILL {
                return Err(format!(
                    "alignment padding of {padding} bytes exceeds the {MAX_DIRECTIVE_FILL}-byte limit"
                ));
            }
            (padding, nobits)
        };
        // Phase 2 — budget BEFORE materialization (and before the
        // sh_addralign raise, so an over-budget pad leaves no trace).
        if !nobits {
            self.charge_fill(decided)?;
        }
        // Phase 3 — materialize + raise under an exclusive borrow.
        if let Some(section) = self.sections.get_mut(&self.current_section) {
            let padding = decided;
            if nobits {
                section.grow_nobits(padding as u64)?;
            } else if padding > 0 {
                if let Some(b) = fill {
                    section.data.extend(std::iter::repeat_n(b, padding));
                } else if section.sh_flags & SHF_EXECINSTR != 0 {
                    // GAS's fill law, measured on GNU as 2.47.20260726 for
                    // both pinned targets: an executable section is padded with
                    // the architecture's NOP, but a NOP may only sit where it
                    // is *legal to fetch it* — at its own size's alignment —
                    // so the bytes that cannot host one are filled from the
                    // front with the largest encoding that fits there, and the
                    // NOPs land at the tail on their natural grid.
                    //
                    //   aarch64  `.byte 1; .p2align 3`  (pad 7)
                    //       00 00 00 1f 20 03 d5   3 zeros, then one NOP at 4
                    //   riscv64  `.byte 1; .p2align 3`  (pad 7, rv64gc)
                    //       00 01 00 13 00 00 00   zero, c.nop at 2, NOP at 4
                    //   riscv64  `.byte 1; .p2align 2`  (pad 3, rv64gc)
                    //       00 01 00               zero, c.nop at 2 (no NOP room)
                    //
                    // Emitting NOPs first instead (the previous behaviour) put a
                    // 4-byte NOP at offset 1: valid to decode on both targets,
                    // but a byte-for-byte divergence from the oracle, and on
                    // aarch64 a NOP straddling the requested boundary is what an
                    // instruction-aligned fetch of the *pad* would trip over.
                    push_code_fill(&mut section.data, padding, &nop, small_nop);
                } else {
                    section.data.extend(std::iter::repeat_n(0u8, padding));
                }
            }
            // GAS records the alignment even when the pad was skipped.
            section.sh_addralign = section.sh_addralign.max(align).max(floor_align);
        }
        Ok(())
    }

    /// Append exactly `n` bytes of code fill (the [`Self::align_to_capped_ex`]
    /// vocabulary) to the current section and raise its `sh_addralign` to
    /// `align`: RISC-V's worst-case `R_RISCV_ALIGN` run, whose length is
    /// decided by the directive, not by the current offset.
    pub fn emit_code_nops(&mut self, n: u64, align: u64) -> Result<(), String> {
        self.ensure_default_section();
        let n = usize::try_from(n)
            .ok()
            .filter(|&n| n <= MAX_DIRECTIVE_FILL)
            .ok_or_else(|| {
                format!(
                    "alignment padding of {n} bytes exceeds the {MAX_DIRECTIVE_FILL}-byte limit"
                )
            })?;
        self.charge_fill(n)?;
        let nop = self.nop_bytes;
        let small_nop = self.small_nop;
        if let Some(section) = self.sections.get_mut(&self.current_section) {
            push_code_fill(&mut section.data, n, &nop, small_nop);
            section.sh_addralign = section.sh_addralign.max(align);
        }
        Ok(())
    }

    /// Ensure we're in a text section, creating one if needed.
    pub fn ensure_text_section(&mut self) {
        if self.current_section.is_empty() {
            // Created at alignment 1 (GAS 2.47 measured): `.text;.byte 1`
            // has sh_addralign 1; the FIRST instruction raises it to the
            // instruction alignment via [`Self::note_instruction`] —
            // `.text;nop` has sh_addralign 4 on aarch64.
            self.ensure_section(".text", SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR, 1);
            self.current_section = ".text".to_string();
        }
    }

    /// GAS 2.47: an instruction raises the containing section's
    /// `sh_addralign` to the target's instruction alignment (aarch64
    /// `.text;nop` -> 4, `.byte 1;nop` -> 4, `.p2align 4;nop` -> 16;
    /// data-only sections stay 1 — see the creation sites).  x86 does
    /// not call this: GNU as keeps x86 `.text` at 1 for bare `nop`
    /// (markers are the only source of x86 sh_addralign), which the x86
    /// core enforces in `reconcile_section_alignments`.
    pub fn note_instruction(&mut self) {
        if self.current_section.is_empty() {
            self.ensure_text_section();
        }
        if let Some(section) = self.sections.get_mut(&self.current_section) {
            section.sh_addralign = section.sh_addralign.max(self.text_align);
        }
    }

    /// Process a .section directive with parsed fields.
    ///
    /// `sec_name`: section name, `flags_str`: flag characters ("awx" etc.),
    /// `flags_explicit`: whether flags were explicitly provided (vs default),
    /// `sec_type_str`: optional type string ("@nobits", "@note", etc.)
    pub fn process_section_directive(
        &mut self,
        sec_name: &str,
        flags_str: &str,
        flags_explicit: bool,
        sec_type_str: Option<&str>,
    ) -> Result<(), String> {
        let sh_type = match sec_type_str {
            Some("@nobits" | "%nobits") => SHT_NOBITS,
            Some("@note" | "%note") => SHT_NOTE,
            _ => {
                if sec_name == ".bss"
                    || sec_name.starts_with(".bss.")
                    || sec_name.starts_with(".tbss")
                {
                    SHT_NOBITS
                } else {
                    SHT_PROGBITS
                }
            }
        };

        let mut sh_flags = 0u64;
        if flags_str.contains('a') {
            sh_flags |= SHF_ALLOC;
        }
        if flags_str.contains('w') {
            sh_flags |= SHF_WRITE;
        }
        if flags_str.contains('x') {
            sh_flags |= SHF_EXECINSTR;
        }
        if flags_str.contains('M') {
            sh_flags |= SHF_MERGE;
        }
        if flags_str.contains('S') {
            sh_flags |= SHF_STRINGS;
        }
        if flags_str.contains('T') {
            sh_flags |= SHF_TLS;
        }
        if flags_str.contains('G') {
            sh_flags |= SHF_GROUP;
        }
        if flags_str.contains('R') {
            sh_flags |= SHF_GNU_RETAIN;
        }

        if sh_flags == 0 && !flags_explicit {
            sh_flags = default_section_flags(sec_name);
        }

        self.check_retain_variant(sec_name, sh_flags)?;
        if let Some(existing) = self.sections.get(sec_name)
            && existing.sh_flags & SHF_GNU_RETAIN != 0
        {
            // As with GAS, re-entering a retained section with explicitly
            // different attributes is an error, not an implicit merge. The
            // generic writer historically ignored these mismatches.
            if flags_explicit && !flags_str.is_empty() && existing.sh_flags != sh_flags {
                return Err(format!("changed section attributes for {sec_name}"));
            }
            if existing.sh_type != sh_type {
                if sec_type_str.is_some() {
                    return Err(format!("changed section type for {sec_name}"));
                }
                if flags_explicit && !flags_str.is_empty() {
                    return Err(format!("changed section attributes for {sec_name}"));
                }
            }
        }
        // Always created at 1: GAS raises AX sections only when an
        // instruction actually lands in them (see `note_instruction`).
        self.ensure_section(sec_name, sh_type, sh_flags, 1);
        self.previous_section = std::mem::replace(&mut self.current_section, sec_name.to_string());
        Ok(())
    }

    /// GAS permits distinct same-named retained and non-retained sections.
    /// The shared writer still keys sections by name, so fail rather than
    /// silently merging them with the wrong GC semantics.
    fn check_retain_variant(&self, name: &str, flags: u64) -> Result<(), String> {
        if let Some(existing) = self.sections.get(name)
            && (existing.sh_flags ^ flags) & SHF_GNU_RETAIN != 0
        {
            return Err(format!(
                "same-named retained and non-retained sections are not yet supported: {name}"
            ));
        }
        Ok(())
    }

    /// Switch to a named standard section (.text, .data, .bss, .rodata).
    pub fn switch_to_standard_section(
        &mut self,
        name: &str,
        sh_type: u32,
        sh_flags: u64,
    ) -> Result<(), String> {
        self.check_retain_variant(name, sh_flags)?;
        // Always created at 1 — instructions raise later (`note_instruction`).
        self.ensure_section(name, sh_type, sh_flags, 1);
        self.previous_section = std::mem::replace(&mut self.current_section, name.to_string());
        Ok(())
    }

    /// Restore the previous section (for `.previous` directive).
    /// Swaps current and previous sections, so repeated `.previous` toggles between two sections.
    pub fn restore_previous_section(&mut self) {
        if !self.previous_section.is_empty() {
            std::mem::swap(&mut self.current_section, &mut self.previous_section);
        }
    }

    /// Push current section onto the stack and switch to a new section.
    /// Saves both current_section and previous_section so that .popsection
    /// fully restores the section state (matching GNU as behavior).
    pub fn push_section(
        &mut self,
        name: &str,
        flags_str: &str,
        flags_explicit: bool,
        sec_type: Option<&str>,
    ) -> Result<(), String> {
        let saved = (self.current_section.clone(), self.previous_section.clone());
        self.process_section_directive(name, flags_str, flags_explicit, sec_type)?;
        self.section_stack.push(saved);
        Ok(())
    }

    /// Pop the section stack and restore both current and previous sections.
    pub fn pop_section(&mut self) {
        if let Some((saved_current, saved_previous)) = self.section_stack.pop() {
            self.current_section = saved_current;
            self.previous_section = saved_previous;
        }
    }

    /// Switch to a numbered subsection within the current section.
    ///
    /// `.subsection N` creates an internal section `PARENT.__subsection.N` that
    /// gets merged back into the parent section after all statements are processed.
    /// Subsections are concatenated in numeric order (0 first, then 1, 2, ...).
    pub fn set_subsection(&mut self, n: u64) {
        // Determine the parent section name (strip any existing subsection suffix)
        let parent = if let Some(pos) = self.current_section.find(".__subsection.") {
            self.current_section[..pos].to_string()
        } else {
            self.current_section.clone()
        };

        if n == 0 {
            // Switch back to parent section (subsection 0 = parent itself)
            self.previous_section = std::mem::replace(&mut self.current_section, parent);
        } else {
            // Switch to subsection N
            let sub_name = format!("{}.__subsection.{}", parent, n);
            // Inherit properties from parent section
            if !self.sections.contains_key(&sub_name) {
                if let Some(parent_sec) = self.sections.get(&parent) {
                    let sh_type = parent_sec.sh_type;
                    let sh_flags = parent_sec.sh_flags;
                    let align = parent_sec.sh_addralign;
                    self.ensure_section(&sub_name, sh_type, sh_flags, align);
                } else {
                    // Parent doesn't exist yet; create subsection as AX at
                    // alignment 1 (GAS — instructions raise it later).
                    self.ensure_section(&sub_name, 1, 0x6, 1); // SHT_PROGBITS, AX
                }
            }
            self.previous_section = std::mem::replace(&mut self.current_section, sub_name);
        }
    }

    /// Merge all subsections back into their parent sections.
    ///
    /// After all statements are processed, subsections like `.text.__subsection.1`
    /// are appended to their parent `.text` in numeric order. Labels and relocations
    /// are adjusted to account for the new offsets.
    ///
    /// Returns a mapping from subsection name to (parent_name, offset_adjustment)
    /// so callers can fix up any pending references that point to subsection names.
    ///
    /// A NOBITS parent (its subsections inherit the type) sums sizes.
    pub fn merge_subsections(&mut self) -> Result<FxHashMap<String, (String, u64)>, String> {
        let mut remap = FxHashMap::default();

        // Collect subsection names grouped by parent
        let mut subsections: std::collections::BTreeMap<
            String,
            std::collections::BTreeMap<u64, String>,
        > = std::collections::BTreeMap::new();

        for name in &self.section_order {
            if let Some(pos) = name.find(".__subsection.") {
                let parent = name[..pos].to_string();
                let num: u64 = name[pos + 14..].parse().unwrap_or(0);
                subsections
                    .entry(parent)
                    .or_default()
                    .insert(num, name.clone());
            }
        }

        if subsections.is_empty() {
            return Ok(remap);
        }

        // For each parent, append subsections in order
        for (parent, subs) in &subsections {
            for sub_name in subs.values() {
                let sub_data;
                let sub_size;
                let sub_relocs;
                {
                    let sub_sec = match self.sections.get(sub_name) {
                        Some(s) => s,
                        None => continue,
                    };
                    sub_data = sub_sec.data.clone();
                    sub_size = sub_sec.size();
                    sub_relocs = sub_sec.relocs.clone();
                }

                let parent_len = self.sections.get(parent).map(ObjSection::size).unwrap_or(0);

                // Record the remapping for callers
                remap.insert(sub_name.clone(), (parent.clone(), parent_len));

                // Append data
                if let Some(parent_sec) = self.sections.get_mut(parent) {
                    if parent_sec.sh_type == SHT_NOBITS {
                        parent_sec.grow_nobits(sub_size)?;
                    } else {
                        parent_sec.data.extend_from_slice(&sub_data);
                    }
                    // Append relocations with adjusted offsets
                    for mut reloc in sub_relocs {
                        reloc.offset += parent_len;
                        parent_sec.relocs.push(reloc);
                    }
                }

                // Adjust labels that reference this subsection
                let labels_to_update: Vec<(String, u64)> = self
                    .labels
                    .iter()
                    .filter(|(_, (sec, _))| sec == sub_name)
                    .map(|(name, (_, off))| (name.clone(), *off))
                    .collect();

                for (label_name, old_offset) in labels_to_update {
                    self.labels
                        .insert(label_name, (parent.clone(), old_offset + parent_len));
                }

                // Remove the subsection
                self.sections.remove(sub_name);
            }
        }

        // Remove subsection names from section_order
        self.section_order
            .retain(|name| !name.contains(".__subsection."));

        // Fix current_section if it pointed to a subsection
        if self.current_section.contains(".__subsection.") {
            if let Some(pos) = self.current_section.find(".__subsection.") {
                self.current_section = self.current_section[..pos].to_string();
            }
        }
        if self.previous_section.contains(".__subsection.") {
            if let Some(pos) = self.previous_section.find(".__subsection.") {
                self.previous_section = self.previous_section[..pos].to_string();
            }
        }

        Ok(remap)
    }

    // The symbol-attribute directives take comma-separated lists in GAS
    // (`.weak a, b`).  Splitting here, once, is what keeps the targets from
    // drifting: AArch64 split `.globl` but not `.weak`, and neither target
    // split `.hidden`, so `.weak a, b` made one symbol named "a, b".

    /// Record `.globl` for a symbol list.
    pub fn set_global(&mut self, syms: &str) {
        for sym in symbol_list(syms) {
            self.global_symbols.insert(sym.to_string(), true);
        }
    }

    /// Record `.weak` for a symbol list.
    pub fn set_weak(&mut self, syms: &str) {
        for sym in symbol_list(syms) {
            self.weak_symbols.insert(sym.to_string(), true);
        }
    }

    /// Record symbol visibility (`.hidden`, `.protected`, `.internal`).
    pub fn set_visibility(&mut self, syms: &str, vis: u8) {
        for sym in symbol_list(syms) {
            self.symbol_visibility.insert(sym.to_string(), vis);
        }
    }

    /// Record .type for a symbol (STT_FUNC, STT_OBJECT, etc.).
    pub fn set_symbol_type(&mut self, sym: &str, st: u8) {
        self.symbol_types.insert(sym.to_string(), st);
    }

    /// Record .size for a symbol. If `current_minus_label` is Some, computes
    /// `current_offset - label_offset` in the same section. Otherwise uses the absolute value.
    pub fn set_symbol_size(
        &mut self,
        sym: &str,
        current_minus_label: Option<&str>,
        absolute: Option<u64>,
    ) {
        if let Some(label) = current_minus_label {
            if let Some((section, label_offset)) = self.labels.get(label) {
                if *section == self.current_section {
                    let current = self.current_offset();
                    let size = current - label_offset;
                    self.symbol_sizes.insert(sym.to_string(), size);
                }
            }
        } else if let Some(size) = absolute {
            self.symbol_sizes.insert(sym.to_string(), size);
        }
    }

    /// `.local a, b`: clear external binding (a later `.globl` restores it
    /// for a defined symbol) and flag each symbol for a later `.comm`.
    pub fn set_local(&mut self, syms: &str) {
        for sym in symbol_list(syms) {
            self.global_symbols.remove(sym);
            self.local_flagged.insert(sym.to_string());
        }
    }

    /// `sym` is defined by a label or reserved by a local `.bss` allocation.
    fn is_defined_here(&self, sym: &str) -> bool {
        self.labels.contains_key(sym) || self.local_commons.iter().any(|(n, _, _)| n == sym)
    }

    /// `.comm sym, size[, align]`, `align` 0 when omitted -- GAS 2.47:
    /// * after `.local sym`: a LOCAL object in `.bss` aligned to `align`
    ///   (omitted: 1; not a power of two: error), even if a `.globl` came in
    ///   between;
    /// * repeated: the first size stays, the alignment is replaced (an
    ///   omitted one by the size law of the kept size);
    /// * otherwise a GLOBAL common, aligned by `default_comm_align` when
    ///   omitted; a later definition of `sym` replaces it and stays GLOBAL
    ///   and, unless `.type` says otherwise, an OBJECT.
    pub fn declare_comm(&mut self, sym: &str, size: u64, align: u64) -> Result<(), String> {
        if self.is_defined_here(sym) {
            return Err(format!("symbol `{sym}' is already defined"));
        }
        if self.local_flagged.contains(sym) {
            let align = align.max(1);
            if !align.is_power_of_two() {
                return Err(format!(".comm {sym}: alignment {align} not a power of 2"));
            }
            self.global_symbols.remove(sym);
            self.local_commons.push((sym.to_string(), size, align));
        } else if let Some(c) = self.commons.iter_mut().find(|(n, _, _)| n == sym) {
            c.2 = if align == 0 {
                u64::from(default_comm_align(c.1))
            } else {
                align
            };
        } else {
            let align = if align == 0 {
                u64::from(default_comm_align(size))
            } else {
                align
            };
            self.global_symbols.insert(sym.to_string(), true);
            self.symbol_types
                .entry(sym.to_string())
                .or_insert(STT_OBJECT);
            self.commons.push((sym.to_string(), size, align));
        }
        Ok(())
    }

    /// `.lcomm sym, size`: a `.bss` allocation aligned by `lcomm_align`;
    /// binding follows `.globl` as for any label.
    pub fn declare_lcomm(&mut self, sym: &str, size: u64) -> Result<(), String> {
        if self.is_defined_here(sym) || self.commons.iter().any(|(n, _, _)| n == sym) {
            return Err(format!("symbol `{sym}' is already defined"));
        }
        self.local_commons
            .push((sym.to_string(), size, lcomm_align(size)));
        Ok(())
    }

    /// Define a source label at the current position and return where.
    /// GAS refuses to define a symbol twice -- numeric labels excepted, by
    /// design -- and so does this: silently moving it re-bound every
    /// earlier reference.  Internal labels do not come through here.
    pub fn define_label(&mut self, name: &str) -> Result<(String, u64), String> {
        if !name.bytes().all(|c| c.is_ascii_digit()) && self.is_defined_here(name) {
            return Err(format!("symbol `{name}' is already defined"));
        }
        self.ensure_text_section();
        let at = (self.current_section.clone(), self.current_offset());
        self.labels.insert(name.to_string(), at.clone());
        Ok(at)
    }

    /// Lay out the `.lcomm` / `.local`+`.comm` allocations at the end of
    /// `.bss`, where GAS's `.bss` subsection 1 puts them; call after
    /// `merge_subsections`.  Each becomes an OBJECT of its size unless the
    /// source said otherwise with `.type`/`.size`.
    pub fn allocate_local_commons(&mut self) -> Result<(), String> {
        let commons = std::mem::take(&mut self.local_commons);
        if commons.is_empty() {
            return Ok(());
        }
        let saved = (self.current_section.clone(), self.previous_section.clone());
        self.switch_to_standard_section(".bss", SHT_NOBITS, SHF_ALLOC | SHF_WRITE)?;
        for (name, size, align) in commons {
            self.align_to(align)?;
            let at = (self.current_section.clone(), self.current_offset());
            self.labels.insert(name.clone(), at);
            let zeros = usize::try_from(size)
                .map_err(|_| format!("{name}: size {size} exceeds the host address space"))?;
            self.emit_fill(zeros, 0)?;
            self.symbol_types.entry(name.clone()).or_insert(STT_OBJECT);
            self.symbol_sizes.entry(name).or_insert(size);
        }
        (self.current_section, self.previous_section) = saved;
        Ok(())
    }

    // ── symbolic values: `.set` and data directives ─────────────────────
    //
    // One model for every value that may name a symbol, shared by the
    // targets so they cannot drift (they did: AArch64 kept `.set` as raw
    // text, RISC-V folded `.set x, .+2` to an ABS 3, and both resolved data
    // differences by substituting label OFFSETS into the expression text,
    // whatever section the labels were in).  A value is CAPTURED where its
    // directive stands ([`Self::capture_symbol`]), FINALISED once every
    // label is known and subsections are merged ([`Self::final_value`]),
    // and CLASSIFIED into a [`DataFixup`] ([`Self::classify`]); the target
    // only maps fixup classes to relocation numbers.

    fn is_global_or_weak(&self, name: &str) -> bool {
        self.global_symbols.contains_key(name) || self.weak_symbols.contains_key(name)
    }

    /// GAS creates `.text`, `.data` and `.bss` in every object, so their
    /// names are sections even before (or without) a directive selecting
    /// them; [`Self::resolve_pending_data`] creates one a relocation names.
    fn is_section_name(&self, name: &str) -> bool {
        self.sections.contains_key(name) || matches!(name, ".text" | ".data" | ".bss")
    }

    /// The value of `name` as of now, for a directive at `section`+`offset`.
    pub fn capture_symbol(&self, name: &str, section: &str, offset: u64) -> SymValue {
        self.capture_symbol_at(name, section, offset, 0)
    }

    fn capture_symbol_at(&self, name: &str, section: &str, offset: u64, depth: u32) -> SymValue {
        if name == "." {
            if let Some(dot) = &self.dot_label {
                return SectionExpr::term(ExprBase::Sym(dot.clone()), 1);
            }
            return SectionExpr::at(ExprBase::Sec(section.to_string()), offset as i64);
        }
        if self.labels.contains_key(name) && self.is_global_or_weak(name) {
            return SectionExpr::term(ExprBase::Sym(name.to_string()), 1);
        }
        match self.sets.get(name) {
            Some(SetValue::Value(v)) => return v.clone(),
            Some(SetValue::Alias(target)) if depth < 64 => {
                return self.capture_symbol_at(target, section, offset, depth + 1);
            }
            Some(_) => return SectionExpr::term(ExprBase::Sym(name.to_string()), 1),
            None => {}
        }
        if let Some((sec, off)) = self.labels.get(name) {
            return self.label_term(name, sec, *off);
        }
        if self.is_section_name(name) {
            return SectionExpr::at(ExprBase::Sec(name.to_string()), 0);
        }
        SectionExpr::term(ExprBase::Sym(name.to_string()), 1)
    }

    /// Capture `expr` for a directive at the current position: a
    /// [`SetValue::Value`], or [`SetValue::Text`] when an operator other than
    /// `+`/`-`/scaling needs a value that is still a forward reference.
    /// Malformed expressions are errors here, not at the end.
    pub fn capture_expr(&mut self, expr: &str) -> Result<SetValue, String> {
        self.ensure_default_section();
        let (section, offset) = (self.current_section.clone(), self.current_offset());
        match eval_section_expr(expr, |n| Ok(self.capture_symbol(n, &section, offset))) {
            Ok(v) => Ok(SetValue::Value(v)),
            Err(e) => {
                // Well-formed once every symbol is a number?  Then only the
                // values are not known yet: evaluate at the end.
                eval_section_expr::<ExprBase>(expr, |_| Ok(SectionExpr::absolute(0)))
                    .map_err(|_| e)?;
                let mut captured: Vec<(String, SymValue)> = Vec::new();
                if let Some(dot) = &self.dot_label {
                    captured.push((
                        ".".to_string(),
                        SectionExpr::term(ExprBase::Sym(dot.clone()), 1),
                    ));
                }
                for name in crate::backend::asm_expr::expr_symbols(expr)? {
                    if self.sets.contains_key(&name) && !captured.iter().any(|(n, _)| *n == name) {
                        let v = self.capture_symbol(&name, &section, offset);
                        captured.push((name, v));
                    }
                }
                Ok(SetValue::Text {
                    expr: expr.to_string(),
                    section,
                    offset,
                    captured,
                })
            }
        }
    }

    /// `.set name, expr` / `.equ name, expr`.
    pub fn define_set(&mut self, name: &str, expr: &str) -> Result<(), String> {
        let expr = expr.trim();
        if name.is_empty() || expr.is_empty() {
            return Err(format!(
                ".set: expected `name, value`, got `{name}, {expr}`"
            ));
        }
        if !self.sets.contains_key(name) && self.labels.contains_key(name) {
            return Err(format!("symbol `{name}' is already defined"));
        }
        let value = if is_bare_symbol(expr) && !self.is_section_name(expr) {
            SetValue::Alias(expr.to_string())
        } else {
            self.capture_expr(expr)
                .map_err(|e| format!(".set {name}, {expr}: {e}"))?
        };
        self.publish_set(name, &value);
        self.sets.insert(name.to_string(), value);
        Ok(())
    }

    /// Mirror a `.set` value into the symbol table's maps: an integer is an
    /// ABS symbol, `section + offset` a symbol defined there (its binding is
    /// whatever `.globl`/`.weak` say, like any label), a bare symbol a
    /// textual alias; anything else waits for [`Self::finalize_sets`].
    fn publish_set(&mut self, name: &str, value: &SetValue) {
        self.labels.remove(name);
        self.aliases.remove(name);
        match value {
            SetValue::Alias(target) => {
                self.aliases.insert(name.to_string(), target.clone());
            }
            SetValue::Value(v) => self.publish_set_value(name, v),
            SetValue::Text { .. } => {}
        }
    }

    fn publish_set_value(&mut self, name: &str, v: &SymValue) {
        match v.terms() {
            [] => {
                self.aliases
                    .insert(name.to_string(), v.constant.to_string());
            }
            [(ExprBase::Sec(sec), 1)] if v.constant >= 0 => {
                self.labels
                    .insert(name.to_string(), (sec.clone(), v.constant as u64));
            }
            [(ExprBase::Sym(target), 1)] if v.constant == 0 => {
                self.aliases.insert(name.to_string(), target.clone());
            }
            _ => {}
        }
    }

    /// The value of label `name`, defined at `sec + off`: section-relative,
    /// unless it is a code label and [`Self::code_labels_symbolic`] holds.
    fn label_term(&self, name: &str, sec: &str, off: u64) -> SymValue {
        if self.code_labels_symbolic.get() && self.is_exec_section(sec) {
            SectionExpr::term(ExprBase::Sym(name.to_string()), 1)
        } else {
            SectionExpr::at(ExprBase::Sec(sec.to_string()), off as i64)
        }
    }

    fn is_exec_section(&self, sec: &str) -> bool {
        self.sections
            .get(sec)
            .is_some_and(|s| s.sh_flags & SHF_EXECINSTR != 0)
    }

    pub fn remap_place(remap: &SubsectionRemap, section: &str, offset: u64) -> (String, u64) {
        match remap.get(section) {
            Some((parent, adj)) => (parent.clone(), offset + adj),
            None => (section.to_string(), offset),
        }
    }

    /// `v` with every subsection key remapped and every [`ExprBase::Sym`]
    /// replaced by its final value; global/weak labels stay by name.
    pub fn final_value(
        &self,
        v: &SymValue,
        remap: &SubsectionRemap,
        depth: u32,
    ) -> Result<SymValue, String> {
        let mut out = SectionExpr::absolute(v.constant);
        for (base, coeff) in v.terms() {
            let t = match base {
                ExprBase::Sec(s) => {
                    let (sec, adj) = Self::remap_place(remap, s, 0);
                    SectionExpr::at(ExprBase::Sec(sec), adj as i64)
                }
                ExprBase::Sym(n) => self.final_symbol(n, remap, depth)?,
            };
            out = out.add_scaled(t, *coeff);
        }
        Ok(out)
    }

    fn final_symbol(
        &self,
        name: &str,
        remap: &SubsectionRemap,
        depth: u32,
    ) -> Result<SymValue, String> {
        if depth > 64 {
            return Err(format!("symbol `{name}' is defined in terms of itself"));
        }
        if (self.labels.contains_key(name) && self.is_global_or_weak(name))
            || self.symbol_types.get(name) == Some(&STT_GNU_IFUNC)
        {
            return Ok(SectionExpr::term(ExprBase::Sym(name.to_string()), 1));
        }
        match self.sets.get(name) {
            Some(SetValue::Value(v)) => return self.final_value(v, remap, depth + 1),
            Some(SetValue::Alias(target)) => return self.final_symbol(target, remap, depth + 1),
            Some(SetValue::Text {
                expr,
                section,
                offset,
                captured,
            }) => return self.final_expr(expr, section, *offset, captured, remap, depth + 1),
            None => {}
        }
        if let Some((sec, off)) = self.labels.get(name) {
            return Ok(self.label_term(name, sec, *off));
        }
        if self.is_section_name(name) {
            return Ok(SectionExpr::at(ExprBase::Sec(name.to_string()), 0));
        }
        Ok(SectionExpr::term(ExprBase::Sym(name.to_string()), 1))
    }

    /// The final value of a [`SetValue::Text`] expression. Its operators
    /// need numbers, so every defined label -- global and weak ones too --
    /// is its section plus offset here ([`Self::fold_defined`]): a
    /// difference within one section then cancels, as in GAS.
    fn final_expr(
        &self,
        expr: &str,
        section: &str,
        offset: u64,
        captured: &[(String, SymValue)],
        remap: &SubsectionRemap,
        depth: u32,
    ) -> Result<SymValue, String> {
        let (sec, off) = Self::remap_place(remap, section, offset);
        eval_section_expr(expr, |n| {
            if n == "." && !captured.iter().any(|(c, _)| c == ".") {
                return Ok(SectionExpr::at(ExprBase::Sec(sec.clone()), off as i64));
            }
            let v = match captured.iter().find(|(c, _)| c == n) {
                Some((_, v)) => self.final_value(v, remap, depth)?,
                None => self.final_symbol(n, remap, depth)?,
            };
            Ok(self.fold_defined(&v))
        })
    }

    fn final_set_value(&self, v: &SetValue, remap: &SubsectionRemap) -> Result<SymValue, String> {
        match v {
            SetValue::Value(v) => self.final_value(v, remap, 0),
            SetValue::Alias(target) => self.final_symbol(target, remap, 0),
            SetValue::Text {
                expr,
                section,
                offset,
                captured,
            } => self.final_expr(expr, section, *offset, captured, remap, 0),
        }
    }

    /// Replace every defined label still held by name with `section + offset`
    /// (code labels excepted while [`Self::code_labels_symbolic`]).
    fn fold_defined(&self, v: &SymValue) -> SymValue {
        let mut out = SectionExpr::absolute(v.constant);
        for (base, coeff) in v.terms() {
            let t = match base {
                ExprBase::Sym(n) => match self.labels.get(n) {
                    Some((sec, off)) => self.label_term(n, sec, *off),
                    None => SectionExpr::term(base.clone(), 1),
                },
                ExprBase::Sec(_) => SectionExpr::term(base.clone(), 1),
            };
            out = out.add_scaled(t, *coeff);
        }
        out
    }

    /// Classify a final value (GAS: differences within one section cancel
    /// whatever the symbols' binding; a lone global/weak label is relocated
    /// against by name, a local one as its section plus offset).
    pub fn classify(&self, v: &SymValue) -> Result<DataFixup, String> {
        let folded = self.fold_defined(v);
        let (mut add, mut sub) = (None, None);
        for (base, coeff) in folded.terms() {
            match coeff {
                1 if add.is_none() => add = Some(base),
                -1 if sub.is_none() => sub = Some(base),
                _ => {
                    return Err("the expression is not a constant, a symbol plus a \
                                constant, or a difference of two symbols"
                        .to_string());
                }
            }
        }
        let name = |b: &ExprBase| match b {
            ExprBase::Sec(n) | ExprBase::Sym(n) => n.clone(),
        };
        let Some(add) = add else {
            return match sub {
                None => Ok(DataFixup::Constant(folded.constant)),
                Some(b) => Err(format!("cannot subtract `{}' from a constant", name(b))),
            };
        };
        let (add, addend) = match add {
            ExprBase::Sec(sec) => {
                // A global/weak label the value named keeps its name.
                let named: Vec<&String> = v
                    .terms()
                    .iter()
                    .filter_map(|(b, c)| match b {
                        ExprBase::Sym(n)
                            if *c == 1 && self.labels.get(n).is_some_and(|(ls, _)| ls == sec) =>
                        {
                            Some(n)
                        }
                        _ => None,
                    })
                    .collect();
                match named[..] {
                    [g] => (g.clone(), folded.constant - self.labels[g].1 as i64),
                    _ => (sec.clone(), folded.constant),
                }
            }
            ExprBase::Sym(n) => (n.clone(), folded.constant),
        };
        Ok(match sub {
            None => DataFixup::Abs { sym: add, addend },
            Some(b) => DataFixup::Diff {
                add,
                sub: name(b),
                addend,
            },
        })
    }

    /// Assemble a `size`-byte data value: a constant now, anything else as a
    /// placeholder resolved by [`Self::resolve_pending_data`].
    pub fn emit_data_value(&mut self, value: SetValue, size: usize) -> Result<(), String> {
        self.ensure_default_section();
        if let SetValue::Value(v) = &value
            && v.is_absolute()
        {
            return self.emit_data_integer(v.constant, size);
        }
        let (section, offset) = (self.current_section.clone(), self.current_offset());
        self.emit_placeholder(size)?;
        self.pending_data.push(PendingData {
            section,
            offset,
            size,
            value,
        });
        Ok(())
    }

    /// The value of `name` (a symbol, or `sym+4`-style text from inline-asm
    /// operand substitution) at the current position, for a data directive.
    pub fn capture_data_symbol(&mut self, name: &str) -> Result<SetValue, String> {
        self.ensure_default_section();
        if !self.keep_code_labels {
            return self.capture_data_symbol_inner(name);
        }
        // `.` at a code location moves with the code around it: give it a
        // label of its own (GNU as's `.L0 `), defined only when named.
        if self.is_exec_section(&self.current_section.clone())
            && (name == "."
                || crate::backend::asm_expr::expr_symbols(name)?
                    .iter()
                    .any(|n| n == "."))
        {
            let dot = format!(".L0\u{1}{}", self.dot_labels);
            self.dot_labels += 1;
            let at = (self.current_section.clone(), self.current_offset());
            self.labels.insert(dot.clone(), at);
            self.dot_label = Some(dot);
        }
        self.code_labels_symbolic.set(true);
        let captured = self.capture_data_symbol_inner(name);
        self.code_labels_symbolic.set(false);
        self.dot_label = None;
        captured
    }

    fn capture_data_symbol_inner(&mut self, name: &str) -> Result<SetValue, String> {
        if is_bare_symbol(name) || name == "." {
            let (section, offset) = (self.current_section.clone(), self.current_offset());
            return Ok(SetValue::Value(self.capture_symbol(name, &section, offset)));
        }
        self.capture_expr(name)
    }

    /// Give every `.set` symbol its final value in the symbol table.
    fn finalize_sets(&mut self, remap: &SubsectionRemap) -> Result<(), String> {
        let mut names: Vec<String> = self.sets.keys().cloned().collect();
        names.sort_unstable();
        for name in names {
            let value = self.sets[&name].clone();
            if let SetValue::Alias(_) = value {
                continue;
            }
            let v = self.final_set_value(&value, remap)?;
            let v = self.fold_defined(&v);
            self.labels.remove(&name);
            self.aliases.remove(&name);
            self.publish_set_value(&name, &v);
        }
        Ok(())
    }

    /// After every label is known and subsections are merged: finalise the
    /// `.set` symbols, patch every deferred constant, and return the
    /// relocatable values as `(section, offset, size, fixup)` for the target
    /// to turn into relocations.
    pub fn resolve_pending_data(
        &mut self,
        remap: &SubsectionRemap,
    ) -> Result<Vec<(String, u64, usize, DataFixup)>, String> {
        self.finalize_sets(remap)?;
        self.code_labels_symbolic.set(self.keep_code_labels);
        let resolved = self.resolve_pending_data_inner(remap);
        self.code_labels_symbolic.set(false);
        resolved
    }

    fn resolve_pending_data_inner(
        &mut self,
        remap: &SubsectionRemap,
    ) -> Result<Vec<(String, u64, usize, DataFixup)>, String> {
        let mut out = Vec::new();
        for p in std::mem::take(&mut self.pending_data) {
            let (section, offset) = Self::remap_place(remap, &p.section, p.offset);
            let what = match &p.value {
                SetValue::Text { expr, .. } | SetValue::Alias(expr) => format!("`{expr}'"),
                SetValue::Value(_) => "value".to_string(),
            };
            let fixup = self
                .final_set_value(&p.value, remap)
                .and_then(|v| self.classify(&v))
                .map_err(|e| format!("data {what} at {section}+{offset:#x}: {e}"))?;
            match fixup {
                DataFixup::Constant(c) => {
                    let sec = self
                        .sections
                        .get_mut(&section)
                        .ok_or_else(|| format!("data value in unknown section {section}"))?;
                    let at = usize::try_from(offset)
                        .ok()
                        .filter(|&o| o + p.size <= sec.data.len())
                        .ok_or_else(|| format!("data value outside {section}"))?;
                    sec.data[at..at + p.size].copy_from_slice(&c.to_le_bytes()[..p.size]);
                }
                f => {
                    for name in f.symbols() {
                        if !self.sections.contains_key(name) && self.is_section_name(name) {
                            let ty = if name == ".bss" {
                                SHT_NOBITS
                            } else {
                                SHT_PROGBITS
                            };
                            self.ensure_section(name, ty, default_section_flags(name), 1);
                        }
                    }
                    out.push((section, offset, p.size, f));
                }
            }
        }
        Ok(out)
    }

    /// A deferred value resolved twice, after [`Self::resolve_pending_data`]:
    /// as relocations would name it (under [`Self::keep_code_labels`]) and as
    /// the constant it has at assembly time, if any.  RISC-V `.uleb128`, whose
    /// bytes hold the assembly-time value even when relocations follow
    /// (the linker rewrites them in place, keeping their length).
    pub fn resolve_value(
        &self,
        value: &SetValue,
        remap: &SubsectionRemap,
    ) -> Result<(DataFixup, Option<i64>), String> {
        self.code_labels_symbolic.set(self.keep_code_labels);
        let symbolic = self
            .final_set_value(value, remap)
            .and_then(|v| self.classify(&v));
        self.code_labels_symbolic.set(false);
        let folded = self
            .final_set_value(value, remap)
            .and_then(|v| self.classify(&v))?;
        let constant = match folded {
            DataFixup::Constant(c) => Some(c),
            _ => None,
        };
        Ok((symbolic?, constant))
    }

    /// Record a relocation at an explicit place (deferred data values).
    pub fn add_reloc_at(
        &mut self,
        section: &str,
        offset: u64,
        reloc_type: u32,
        sym: &str,
        addend: i64,
    ) {
        if let Some(sec) = self.sections.get_mut(section) {
            sec.relocs.push(ObjReloc {
                offset,
                reloc_type,
                symbol_name: sym.to_string(),
                addend,
            });
        }
    }

    /// Record a raw textual alias (`.symver`'s default-version alias).
    pub fn set_alias(&mut self, alias: &str, target: &str) {
        self.aliases.insert(alias.to_string(), target.to_string());
    }

    /// Resolve .set/.equ aliases in an expression string.
    /// Replaces symbol names (like `.L__gpr_num_t0`) with their numeric values.
    pub fn resolve_expr_aliases(&self, expr: &str) -> String {
        let mut result = String::with_capacity(expr.len());
        let bytes = expr.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            // Symbol names start with a letter, underscore, or dot
            if c == b'.' || c == b'_' || c.is_ascii_alphabetic() {
                let start = i;
                i += 1;
                while i < bytes.len()
                    && (bytes[i] == b'.' || bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric())
                {
                    i += 1;
                }
                let sym = &expr[start..i];
                // Chase alias chain, keeping STT_GNU_IFUNC aliases intact so
                // branch/data relocations bind through the IFUNC symbol rather
                // than calling or storing the resolver directly.
                let mut resolved = sym;
                let mut seen = 0;
                while self.symbol_types.get(resolved) != Some(&STT_GNU_IFUNC)
                    && let Some(target) = self.aliases.get(resolved)
                {
                    resolved = target.as_str();
                    seen += 1;
                    if seen > 20 {
                        break;
                    }
                }
                result.push_str(resolved);
            } else {
                crate::backend::asm_preprocess::push_char_at(&mut result, expr, &mut i);
            }
        }
        result
    }

    /// Emit a plain integer value for .byte (size=1), .short (size=2), .long (size=4) or .quad (size=8).
    pub fn emit_data_integer(&mut self, val: i64, size: usize) -> Result<(), String> {
        // RAW-value check (GAS rejects `.long 1<<32` in .bss although the
        // truncated store would be all zeros) — must run before encoding.
        self.nobits_guard(val != 0)?;
        match size {
            1 => self.emit_bytes(&[val as u8])?,
            2 => self.emit_bytes(&(val as u16).to_le_bytes())?,
            4 => self.emit_bytes(&(val as u32).to_le_bytes())?,
            _ => self.emit_bytes(&(val as u64).to_le_bytes())?,
        }
        Ok(())
    }

    /// Emit placeholder bytes for a deferred value (symbol diff, etc.).
    /// `n` zero bytes for a value written later (see [`Self::emit_placeholder`]
    /// for the NOBITS rule).
    pub fn emit_placeholder_bytes(&mut self, n: usize) -> Result<(), String> {
        self.nobits_guard(true)?;
        self.emit_bytes(&vec![0u8; n])
    }

    pub fn emit_placeholder(&mut self, size: usize) -> Result<(), String> {
        // The deferred value is unknown here; treat it as non-zero until
        // proven otherwise — a NOBITS section must not receive content
        // whose value the guard cannot verify (conservative, fail-closed;
        // a zero-valued symbol DIFF in .bss is rejected where GAS might
        // accept it, which is the safe direction for this rule).
        self.nobits_guard(true)?;
        match size {
            1 => self.emit_bytes(&[0u8])?,
            2 => self.emit_bytes(&0u16.to_le_bytes())?,
            4 => self.emit_bytes(&0u32.to_le_bytes())?,
            _ => self.emit_bytes(&0u64.to_le_bytes())?,
        }
        Ok(())
    }

    /// Resolve local label references in data relocations.
    ///
    /// When a data directive like `.xword .Lstr0` references a local label
    /// in a different section, the local label won't be in the symbol table.
    /// Convert these to section_symbol + offset_of_label_in_section.
    pub fn resolve_local_data_relocs(&mut self) {
        let labels = self.labels.clone();
        for sec_name in &self.section_order.clone() {
            if let Some(section) = self.sections.get_mut(sec_name) {
                for reloc in &mut section.relocs {
                    // Skip pcrel_lo12 relocations — they must keep their
                    // .Lpcrel_hi label reference (not section+offset)
                    let is_pcrel_lo = reloc.reloc_type == 24 || reloc.reloc_type == 25;
                    if is_pcrel_lo {
                        continue;
                    }
                    if (reloc.symbol_name.starts_with(".L") || reloc.symbol_name.starts_with(".l"))
                        && !reloc.symbol_name.is_empty()
                    {
                        if let Some((label_section, label_offset)) = labels.get(&reloc.symbol_name)
                        {
                            reloc.addend += *label_offset as i64;
                            reloc.symbol_name = label_section.clone();
                        }
                    }
                }
            }
        }
    }

    /// Build the symbol table and serialize the ELF object file.
    ///
    /// `config`: ELF configuration (machine type, flags, class)
    /// `include_referenced_locals`: whether to include .L* labels referenced
    /// by relocations (needed by RISC-V for pcrel_hi/pcrel_lo pairs)
    pub fn write_elf(
        &mut self,
        output_path: &str,
        config: &ElfConfig,
        include_referenced_locals: bool,
    ) -> Result<(), String> {
        if !include_referenced_locals {
            self.resolve_local_data_relocs();
        }

        let symtab_input = SymbolTableInput {
            labels: &self.labels,
            global_symbols: &self.global_symbols,
            weak_symbols: &self.weak_symbols,
            symbol_types: &self.symbol_types,
            symbol_sizes: &self.symbol_sizes,
            symbol_visibility: &self.symbol_visibility,
            aliases: &self.aliases,
            sections: &self.sections,
            include_referenced_locals,
        };

        let mut symbols = build_elf_symbol_table(&symtab_input);
        // Commons that no definition replaced (`.comm x` then `x:` makes
        // `x` an ordinary GLOBAL definition, as in GAS).
        for (name, size, align) in &self.commons {
            if !self.labels.contains_key(name) {
                self.extra_symbols.push(ObjSymbol {
                    name: name.clone(),
                    value: *align,
                    size: *size,
                    binding: STB_GLOBAL,
                    sym_type: STT_OBJECT,
                    visibility: self
                        .symbol_visibility
                        .get(name)
                        .copied()
                        .unwrap_or(STV_DEFAULT),
                    section_name: "*COM*".to_string(),
                });
            }
        }
        // Remove UND entries for any symbols that are also in extra_symbols (e.g. COMMON).
        // A symbol that is both referenced in relocations and declared as COMMON should
        // only appear once (as COMMON), not as both UND and COMMON.
        for extra in &self.extra_symbols {
            if extra.section_name == "*COM*" {
                symbols.retain(|s| !(s.name == extra.name && s.section_name == "*UND*"));
            }
        }
        symbols.append(&mut self.extra_symbols);

        // Relocations in offset order, as GNU as writes them: consumers walk
        // them with a cursor (bfd's `.eh_frame` parser rejects the section
        // -- "no .eh_frame_hdr table will be created" -- when the FDE fields
        // resolved after the data fixups come out of order). The sort is
        // stable, so entries at one offset (ADD/SUB, SET/SUB, HI20 + RELAX)
        // keep their emission order, which is significant.
        for sec in self.sections.values_mut() {
            sec.relocs.sort_by_key(|r| r.offset);
        }

        let elf_bytes =
            write_relocatable_object(config, &self.section_order, &self.sections, &symbols)?;

        std::fs::write(output_path, &elf_bytes)
            .map_err(|e| format!("failed to write ELF file: {}", e))?;

        Ok(())
    }
}
