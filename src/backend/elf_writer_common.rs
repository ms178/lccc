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
    self as elf_mod, ElfConfig, ObjReloc, ObjSection, ObjSymbol, SHF_ALLOC, SHF_EXECINSTR,
    SHF_WRITE, SHT_NOBITS, SHT_PROGBITS, STB_GLOBAL, STB_LOCAL, STB_WEAK, STT_FUNC, STT_GNU_IFUNC,
    STT_NOTYPE, STT_OBJECT, STT_TLS, STV_DEFAULT, STV_HIDDEN, STV_INTERNAL, STV_PROTECTED,
    SymbolTableInput, parse_section_flags, resolve_numeric_labels,
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

    /// True for TLS relocation types (R_386_TLS_* / R_X86_64_*TLS*).
    ///
    /// TLS relocs are excluded from local-label folding: the fold rewrites a
    /// relocation against a local symbol into (section symbol,
    /// addend+offset), which is value-identical for plain/PC relocs but WRONG
    /// for TLS ones.  A GOT-style TLS reloc (R_386_TLS_GOTIE, the i386
    /// encoding of `@GOTNTPOFF`) is a pure GOT-slot reference — its addend
    /// must stay 0 — and the linker computes the slot's contents from the
    /// STT_TLS symbol's position in the TLS block, not from a section symbol.
    /// Folding also destroys the offset for REL-format i686, where the
    /// addend is baked into the instruction bytes and then clobbered by the
    /// linker's write.  (Measured: lccc-i686 `movl lv@GOTNTPOFF(%ebx)`
    /// folded to `.tdata@GOTIE` linked a zero into the displacement and
    /// segfaulted on the first TLS touch; GAS keeps the STT_TLS symbol in
    /// the reloc.)
    fn is_tls_reloc(reloc_type: u32) -> bool {
        let _ = reloc_type;
        false
    }

    /// True when this relocation class makes GNU as record the
    /// `_GLOBAL_OFFSET_TABLE_` reference (a GLOBAL UND NOTYPE symbol) in
    /// the object's symbol table.
    ///
    /// Measured against GAS 2.47 (`--64`/`--32`, one operator per object):
    /// every operator-emitted relocation whose computation reads the GOT
    /// base adds the symbol — @GOTPCREL/@TLSDESC/@GOTTPOFF/@TLSGD/@TLSLD/
    /// @GOT/@TPOFF/@DTPOFF/@GOTNTPOFF and their APX CODE_4/5/6 cousins.
    /// Deliberately excluded: plain PC/ABS (no GOT), @PLT (a PLT-slot
    /// reference, not a GOT-base one — probed nogot), the
    /// TLSDESC_CALL/TLSDESC marker pair and the GOT64/GOTPCREL64/GOTPC64/
    /// GOTPC32/GOTOFF64 classes (reachable on x86-64 only through `.reloc`,
    /// which GAS runs past the operator machinery and does NOT attach the
    /// GOT symbol to), and the linker-reserved IRELATIVE/RELATIVE/SIZE
    /// groups.
    fn needs_got_base_symbol(reloc_type: u32) -> bool {
        let _ = reloc_type;
        false
    }

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
    /// Base label and constant source addend are kept separately.  Short-only
    /// branches are patched by the relaxation engine rather than an ELF
    /// relocation, so retaining a spelling such as `.Lloop+1` as the literal
    /// label would leave its displacement at zero (the label does not exist).
    target: String,
    target_addend: i64,
    is_conditional: bool,
    /// Whether the jump currently uses its short form.
    relaxed: bool,
    /// Whether this writer shortened the jump and can restore its long form.
    /// Short-only instructions remain false.
    can_grow: bool,
}

/// Apply a source-level branch addend without wrapping an invalid target into
/// the section.  Returning `None` leaves the ordinary unresolved-target path
/// in control; it must never manufacture an address on overflow.
fn jump_target_with_addend(offset: Option<usize>, addend: i64) -> Option<usize> {
    let offset = i64::try_from(offset?).ok()?;
    usize::try_from(offset.checked_add(addend)?).ok()
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
    /// NOP table for the code mode in force at the directive (a later
    /// `.code16`/`.code32`/`.code64` must not change how this gap is
    /// re-padded after relaxation).
    nops: NopTable,
    /// The tight-loop bucket (16/32/64) resolved by the LAST fixup sweep,
    /// or `None` when the marker finally rejected (or has not resolved
    /// yet). It feeds the post-fixed-point section-alignment reconciliation
    /// so an accept -> reject flip cannot leave a stale 64-byte section
    /// alignment behind (`AlignMarkerKind::Align` records its fixed
    /// alignment textually and needs no field here).
    tight_resolved_align: Option<u64>,
    /// Source position of the directive (see `ElfWriterCore::shift_after`):
    /// a label at the marker's own offset precedes the padding iff it was
    /// defined earlier in the source.
    seq: u64,
}

#[derive(Clone, Debug)]
enum AlignMarkerKind {
    /// .balign N — pad to N-byte boundary. `fill`/`max_skip` mirror the
    /// `AsmItem::Align` fields: an explicit non-`0x90` fill byte pads
    /// verbatim, and a max-skip that the recomputed padding would exceed
    /// suppresses the padding entirely (GAS `.p2align N,,M`). Both must be
    /// re-evaluated here because jump relaxation shifts the offset this
    /// marker lands on.
    Align {
        align: u64,
        fill: Option<u8>,
        max_skip: Option<u64>,
    },
    /// .org label + offset — advance to a fixed position, filling with `fill`.
    ///
    /// `fill` is load-bearing: GAS pads `.org` with its fill byte (default 0)
    /// even in an executable section. Regenerating the run with multi-byte
    /// NOPs would change the bytes the kernel's IDT stub `.fill …, 1, 0xcc`
    /// (lowered to `.org`) depends on.
    Org {
        label: String,
        addend: i64,
        fill: u8,
    },
    /// lccc-internal tight-loop marker (`.lccc_tight_loop HEADER`).
    /// Resolved during the alignment fixed point: the header is padded
    /// to `2^ceil(log2(encoded body span))`, clamped to 8..=64 bytes,
    /// but only when the body — from the header to its first backward
    /// branch — fits one instruction-cache line (`TIGHT_LOOP_MAX_BYTES`).
    /// Mirrors GCC's `align_tight_loops` RTL pass using lccc's exact
    /// post-relaxation encoded lengths instead of RTL estimates.
    TightLoop { header: String },
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

/// Above this many maximum-size NOPs, jumping over the padding is cheaper
/// than executing it. Matches GAS's `max_number_of_nops` empirically: with
/// the 11-byte long-NOP table, GNU as 2.4x emits a jump only once the gap
/// exceeds EIGHT max-size NOPs (count/11 > 8 — an 88-byte pad is still all
/// NOPs, 99 bytes becomes `jmp` + NOPs), where a predicted-taken branch
/// clearly beats decoding more NOP µops.
const MAX_NOP_RUN: usize = 7; // GAS 2.47: jump-over at count/11 > 7 (2.44 used > 8)

// The tight-loop size bucket is owned by `passes::loop_align` (the
// structural audit) and imported here so the assembler-side size decision
// can never silently drift from the IR-side policy: a promoted tight loop
// whose encoded body fits one instruction-cache line is aligned to
// `2^ceil(log2(span))`, exponent clamped to 8..=64; larger bodies fail
// closed (no extra padding, ordinary cascade only).
use crate::passes::loop_align::tight_bucket_log2;

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
pub(crate) fn exec_padding(count: usize, after_insn: bool, table: NopTable) -> Vec<u8> {
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

    let patterns = table.patterns();
    let max_nop = patterns.len();
    if count / max_nop > table.max_run() {
        let disp = count - 2;
        if disp <= 0x7F {
            out.push(0xEB);
            out.push(disp as u8);
            count = disp;
        } else {
            // In 16-bit code a bare `e9` takes a rel16: GAS writes the
            // operand-size-prefixed rel32 form (`66 e9 rel32`, 6 bytes).
            let len = if table == NopTable::Lea16 { 6 } else { 5 };
            let rel = count - len;
            if table == NopTable::Lea16 {
                out.push(0x66);
            }
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
    if count % max_nop != 0 {
        out.extend_from_slice(patterns[count % max_nop - 1]);
        count -= count % max_nop;
    }
    while count >= max_nop {
        out.extend_from_slice(patterns[max_nop - 1]);
        count -= max_nop;
    }
    out
}

/// The GNU as NOP table that pads an executable alignment gap.
///
/// GAS 2.47 (`i386_generate_nops`) selects it from the tuning -- which,
/// with no `-mtune` and no `.arch`, follows the object class -- and the
/// code mode in force at the directive. Measured against `as --64`/`--32`
/// for every gap of 1..200 bytes in each mode:
///
/// | object | code mode | table | jump over the gap when |
/// |--------|-----------|-------|------------------------|
/// | ELF64  | 64, 32    | `Long`  (max 11) | count / 11 > 7 |
/// | ELF32  | 32        | `Lea32` (max 8)  | count / 8 > 2  |
/// | ELF32  | 64        | `Lea64` (max 9)  | count / 9 > 2  |
/// | either | 16        | `Lea16` (max 5)  | count / 5 > 2  |
///
/// The ELF32 tables never use NOPL (`0f 1f /0`): binutils' generic32
/// target (and its `i686`, which lacks `CpuNop`) must run on i686-class
/// cores that do not implement it (AMD Geode LX, VIA C3), and alignment
/// padding is routinely executed. An `.arch` naming a NOPL-capable CPU
/// (`pentiumpro`, `corei7`, ...) switches GAS to `Long`; lccc does not
/// model `.arch` and keeps GAS's default -- the table every GCC-built
/// object uses, since GCC passes neither `-march` nor `.arch` to `as`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NopTable {
    /// Long NOPs (`NOP_PATTERNS`): 64-bit objects, 64- and 32-bit code.
    Long,
    /// `lea`-based NOPs: 32-bit objects, 32-bit code.
    Lea32,
    /// The `Lea32` forms with REX.W: `.code64` in a 32-bit object.
    Lea64,
    /// 16-bit `lea`/`mov` NOPs: `.code16` and `.code16gcc`.
    Lea16,
}

impl NopTable {
    /// The table for an object whose native mode is `object_bits`
    /// (`X86Arch::default_code_mode`) while assembling in `code_mode`
    /// (16, 17 = `.code16gcc`, 32 or 64).
    pub(crate) fn for_mode(object_bits: u8, code_mode: u8) -> Self {
        match code_mode {
            16 | 17 => NopTable::Lea16,
            _ if object_bits == 64 => NopTable::Long,
            64 => NopTable::Lea64,
            _ => NopTable::Lea32,
        }
    }

    /// NOP encodings by length; entry `n - 1` is `n` bytes long.
    fn patterns(self) -> &'static [&'static [u8]] {
        match self {
            NopTable::Long => &NOP_PATTERNS,
            NopTable::Lea32 => &LEA32_NOP_PATTERNS,
            NopTable::Lea64 => &LEA64_NOP_PATTERNS,
            NopTable::Lea16 => &LEA16_NOP_PATTERNS,
        }
    }

    /// Maximum-size NOPs GAS will execute before jumping over the gap.
    fn max_run(self) -> usize {
        match self {
            NopTable::Long => MAX_NOP_RUN,
            NopTable::Lea32 | NopTable::Lea64 | NopTable::Lea16 => 2,
        }
    }
}

/// GAS 2.47 `--32` NOPs for 32-bit code (tc-i386.c `f32_patt`).
const LEA32_NOP_PATTERNS: [&[u8]; 8] = [
    &[0x90],                                           // nop
    &[0x66, 0x90],                                     // xchg %ax,%ax
    &[0x8D, 0x76, 0x00],                               // lea 0(%esi),%esi
    &[0x8D, 0x74, 0x26, 0x00],                         // lea 0(%esi,%eiz),%esi
    &[0x2E, 0x8D, 0x74, 0x26, 0x00],                   // lea %cs:0(%esi,%eiz),%esi
    &[0x8D, 0xB6, 0x00, 0x00, 0x00, 0x00],             // lea 0L(%esi),%esi
    &[0x8D, 0xB4, 0x26, 0x00, 0x00, 0x00, 0x00],       // lea 0L(%esi,%eiz),%esi
    &[0x2E, 0x8D, 0xB4, 0x26, 0x00, 0x00, 0x00, 0x00], // lea %cs:0L(%esi,%eiz),%esi
];

/// GAS 2.47 `--32` NOPs under `.code64`: 64-bit-safe (REX.W) `lea`/`mov`.
const LEA64_NOP_PATTERNS: [&[u8]; 9] = [
    &[0x90],                                                 // nop
    &[0x66, 0x90],                                           // xchg %ax,%ax
    &[0x48, 0x89, 0xF6],                                     // mov %rsi,%rsi
    &[0x48, 0x8D, 0x76, 0x00],                               // lea 0(%rsi),%rsi
    &[0x48, 0x8D, 0x74, 0x26, 0x00],                         // lea 0(%rsi,%riz),%rsi
    &[0x2E, 0x48, 0x8D, 0x74, 0x26, 0x00],                   // lea %cs:0(%rsi,%riz),%rsi
    &[0x48, 0x8D, 0xB6, 0x00, 0x00, 0x00, 0x00],             // lea 0L(%rsi),%rsi
    &[0x48, 0x8D, 0xB4, 0x26, 0x00, 0x00, 0x00, 0x00],       // lea 0L(%rsi,%riz),%rsi
    &[0x2E, 0x48, 0x8D, 0xB4, 0x26, 0x00, 0x00, 0x00, 0x00], // lea %cs:0L(%rsi,%riz),%rsi
];

/// GAS 2.47 NOPs for 16-bit code (tc-i386.c `f16_patt`).
const LEA16_NOP_PATTERNS: [&[u8]; 5] = [
    &[0x90],                         // nop
    &[0x89, 0xF6],                   // mov %si,%si
    &[0x8D, 0x74, 0x00],             // lea 0(%si),%si
    &[0x8D, 0xB4, 0x00, 0x00],       // lea 0W(%si),%si
    &[0x2E, 0x8D, 0xB4, 0x00, 0x00], // lea %cs:0W(%si),%si
];

/// `.fill LABEL + N - ., 1, FILL` (and the equivalent `.skip`) is a
/// location-counter *target*, not a one-shot gap.
///
/// Jump relaxation changes instruction sizes after a deferred skip is
/// first measured. Sizing `LABEL + N - .` against the pre-growth layout
/// leaves extra fill bytes once a nearby `jmp` grows from rel8 back to
/// rel32 — observed as Linux `early_idt_handler_array` slots overflowing
/// `EARLY_IDT_HANDLER_SIZE` (13 with IBT) from vector 9 onwards, so the
/// IDT entry for #GP pointed into `int3` padding and the kernel halted
/// in `early_fixup_exception`.
///
/// Lowering the shape to `.org LABEL+N` reuses the post-relaxation org
/// fixup, matching GAS. Numeric local labels (`0b + 16 - .`, the
/// ALTERNATIVE pad) stay on the deferred-skip path: their skip/relax
/// order is already tuned and they are not named-symbol targets.
pub(crate) fn parse_org_style_skip(expr: &str) -> Option<(String, i64)> {
    let s = expr.trim();
    let without_dot = {
        let t = s.trim_end();
        if let Some(rest) = t.strip_suffix("-.") {
            rest
        } else if let Some(rest) = t.strip_suffix('.') {
            let rest = rest.trim_end();
            rest.strip_suffix('-')?
        } else {
            return None;
        }
    };
    let without_dot = without_dot.trim();
    if without_dot.is_empty() {
        return None;
    }
    let bytes = without_dot.as_bytes();
    // Numeric local labels (`0b`, `1f`) keep the deferred-skip path.
    if bytes[0].is_ascii_digit() {
        return None;
    }
    if !(bytes[0].is_ascii_alphabetic() || bytes[0] == b'_' || bytes[0] == b'.') {
        return None;
    }
    let mut i = 1;
    while i < bytes.len()
        && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
    {
        i += 1;
    }
    let ident = &without_dot[..i];
    let rest = without_dot[i..].trim();
    if rest.is_empty() {
        return Some((ident.to_string(), 0));
    }
    if !(rest.starts_with('+') || rest.starts_with('-')) {
        return None;
    }
    if rest.bytes().any(|b| b.is_ascii_alphabetic() || b == b'_') {
        return None;
    }
    let addend = crate::backend::asm_expr::parse_integer_expr(rest).ok()?;
    Some((ident.to_string(), addend))
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
pub(crate) fn section_padding(
    count: usize,
    is_exec: bool,
    after_insn: bool,
    table: NopTable,
) -> Vec<u8> {
    if is_exec {
        exec_padding(count, after_insn, table)
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
                ));
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
    /// Source position of the `.skip`.
    ///
    /// A label defined immediately BEFORE the skip and one defined
    /// immediately AFTER it both record the very same offset, because the
    /// gap has no length yet. The fill bytes go *after* the former and
    /// *before* the latter; `shift_after` tells them apart by source order.
    /// Shifting both dragged a preceding label forward by the gap and
    /// collapsed any difference against it: the kernel's `.byte 773b-771b`
    /// (total source length of an ALTERNATIVE with an empty old
    /// instruction) assembled as 0 instead of the padded length, which
    /// objtool reports as "empty alternative entry" and which makes
    /// boot-time alternative patching a no-op.
    pub seq: u64,
}

/// A `DW_CFA_advance_loc*` whose operand is the distance between two code
/// labels (`AsmItem::CfaAdvance`). It occupies `CFA_ADVANCE_MAX` bytes
/// until the code layout is final, then shrinks to the smallest encoding.
#[derive(Clone, Debug)]
struct CfaAdvance {
    sec_idx: usize,
    offset: usize,
    from: String,
    to: String,
}

/// Placeholder size of a deferred CFA advance: `DW_CFA_advance_loc4`.
const CFA_ADVANCE_MAX: usize = 5;

/// Smallest DWARF encoding of a location advance of `delta` bytes (code
/// alignment factor 1), as GNU as picks it: nothing for 0,
/// `DW_CFA_advance_loc` with the delta in the opcode below 64, then
/// `DW_CFA_advance_loc1`/`2`/`4`.
fn encode_cfa_advance(delta: u64) -> Result<Vec<u8>, String> {
    Ok(match delta {
        0 => Vec::new(),
        1..=0x3f => vec![0x40 | delta as u8],
        0x40..=0xff => vec![0x02, delta as u8],
        0x100..=0xffff => {
            let [lo, hi] = (delta as u16).to_le_bytes();
            vec![0x03, lo, hi]
        }
        0x1_0000..=0xffff_ffff => {
            let mut v = vec![0x04];
            v.extend_from_slice(&(delta as u32).to_le_bytes());
            v
        }
        _ => {
            return Err(format!(
                "CFA location advance of {delta} bytes exceeds 32 bits"
            ));
        }
    })
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
    /// CFA location advances awaiting the final code layout.
    deferred_cfa_advances: Vec<CfaAdvance>,
    /// Source position of the item being processed. Zero-width records
    /// (labels, alignment/org markers, deferred skips) carry the value of
    /// their defining item so that an insertion at their exact offset can
    /// tell which of them precede it (`shift_after`).
    seq: u64,
    /// Source position of each named label's definition.
    label_seq: FxHashMap<String, u64>,
    /// Same for numeric labels, parallel to `numeric_label_positions`.
    numeric_label_seq: FxHashMap<String, Vec<u64>>,
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
            deferred_cfa_advances: Vec::new(),
            seq: 0,
            label_seq: FxHashMap::default(),
            numeric_label_seq: FxHashMap::default(),
            _arch: std::marker::PhantomData,
        }
    }

    /// Build the ELF object file from parsed assembly items.
    pub fn build(mut self, items: &[AsmItem]) -> Result<Vec<u8>, String> {
        let items = resolve_numeric_labels(items);
        for item in &items {
            self.seq += 1;
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
                if let Some(well_known) = crate::backend::elf::well_known_section_type(name) {
                    eprintln!(
                        "warning: ignoring incorrect section type for {name} (@{} requested)",
                        if well_known == SHT_NOBITS {
                            "nobits"
                        } else {
                            "progbits"
                        }
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
        let (mut section_type, mut flags) =
            parse_section_flags(&dir.name, dir.flags.as_deref(), dir.section_type.as_deref());
        // GNU as checks only the attributes a directive states: `.section
        // NAME` -- or empty flags, `.section NAME,""` -- for an existing
        // section switches to it with its attributes unchanged. GCC re-enters
        // `.gcc_except_table` that way after each function's LSDA; comparing
        // the name's defaults against the section's real flags rejected every
        // GCC -fexceptions object ("changed section attributes").
        if let Some(&idx) = self.section_map.get(&dir.name) {
            if dir.flags.as_deref().is_none_or(str::is_empty) {
                flags = self.sections[idx].flags;
            }
            if dir.section_type.is_none() {
                section_type = self.sections[idx].section_type;
            }
        }
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
            | AsmItem::Align { .. }
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
                            self.place_label(&end_label, sec_idx, current_off);
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
                self.place_label(name, sec_idx, offset);

                if name.chars().all(|c| c.is_ascii_digit()) {
                    self.numeric_label_positions
                        .entry(name.clone())
                        .or_default()
                        .push((sec_idx, offset));
                    self.numeric_label_seq
                        .entry(name.clone())
                        .or_default()
                        .push(self.seq);
                }

                self.ensure_symbol(name, sec_idx, offset);
            }
            AsmItem::TightLoopAlign { header } => {
                // Recorded at the pre-header position with zero initial
                // padding; the actual size-bucketed alignment is chosen
                // inside `fixup_alignment_markers` once the branch
                // relaxation fixed point knows every encoded length and
                // label offset. No bytes are emitted here.
                let after_insn = self.last_item_was_insn;
                let seq = self.seq;
                if let Some(sec_idx) = self.current_section {
                    let is_exec = self.sections[sec_idx].flags & SHF_EXECINSTR != 0;
                    if is_exec {
                        let current = self.sections[sec_idx].data.len();
                        self.sections[sec_idx].align_markers.push(AlignMarker {
                            offset: current,
                            padding: 0,
                            kind: AlignMarkerKind::TightLoop {
                                header: header.clone(),
                            },
                            after_insn,
                            nops: NopTable::for_mode(A::default_code_mode(), self.code_mode),
                            tight_resolved_align: None,
                            seq,
                        });
                    }
                }
            }
            AsmItem::Align {
                align,
                fill,
                max_skip,
            } => {
                let after_insn = self.last_item_was_insn;
                let seq = self.seq;
                if let Some(sec_idx) = self.current_section {
                    let section = &mut self.sections[sec_idx];
                    let align = (*align).max(1);
                    // GAS computes the alignment in signed arithmetic: an
                    // exponent of 63 (or one clamped to it — negative or > 63
                    // inputs warn and assume 63) makes 1<<63 negative
                    // internally, so the alignment is neither recorded on the
                    // section nor padded. A nonzero offset with such an
                    // alignment errors on the jump-over-NOP reach instead
                    // (verified: `.p2align 63` at offset 0 -> addralign 1;
                    // `.byte 1; .p2align 63` -> error, binutils 2.47).
                    if align >= 1u64 << 63 {
                        if section.data.len() as u64 != 0 {
                            return Err(format!(
                                "jump over nop padding out of range (align {align})"
                            ));
                        }
                        return Ok(());
                    }
                    // GAS records the alignment on the section even when the
                    // max-skip test then refuses to pad (binutils 2.44:
                    // `.byte 1; .p2align 4,,1` -> no padding, addralign 16),
                    // so the bump happens before the skip decision.
                    if align > section.alignment {
                        section.alignment = align;
                    }
                    let current = section.data.len() as u64;
                    let aligned = current.div_ceil(align) * align;
                    let mut padding = (aligned - current) as usize;
                    if let Some(skip) = max_skip {
                        if padding as u64 > *skip {
                            padding = 0;
                        }
                    }
                    // GAS refuses pathological alignment padding instead of
                    // materializing it: executable sections cap at the
                    // jump-over-NOP rel32 reach ("jump over nop padding out
                    // of range"), data sections die on the fill. Padding that
                    // cannot fit a 32-bit byte count is never legitimate in a
                    // real object, so the writer rejects it uniformly.
                    if padding as u64 > 0xFFFF_FFFF {
                        return Err(format!(
                            "alignment padding of {padding} bytes too large (align {align})"
                        ));
                    }
                    // Record an alignment marker for post-relaxation fixup
                    // whenever an alignment is requested — INCLUDING the
                    // padding == 0 case. Jump relaxation only ever shrinks
                    // code, which moves this offset DOWN and can turn a
                    // satisfied alignment into one that needs padding; a
                    // marker recorded only for material padding would leave
                    // the label unaligned after relaxation (the same hazard
                    // the `.org` path already documents at its recording
                    // site). The fixup re-evaluates the max-skip decision
                    // against the relaxed offset.
                    if align > 1 {
                        section.align_markers.push(AlignMarker {
                            offset: current as usize,
                            padding,
                            kind: AlignMarkerKind::Align {
                                align,
                                fill: *fill,
                                max_skip: *max_skip,
                            },
                            after_insn,
                            nops: NopTable::for_mode(A::default_code_mode(), self.code_mode),
                            tight_resolved_align: None,
                            seq,
                        });
                    }
                    let is_exec = section.flags & SHF_EXECINSTR != 0;
                    let pad_bytes = match fill {
                        // 0x90 in an executable section keeps GAS's optimal
                        // multi-byte NOP padding (tc-i386 treats the default
                        // fill specially); any other byte pads verbatim.
                        Some(f) if is_exec && *f != 0x90 => vec![*f; padding],
                        Some(f) if !is_exec => vec![*f; padding],
                        _ => section_padding(
                            padding,
                            is_exec,
                            after_insn,
                            NopTable::for_mode(A::default_code_mode(), self.code_mode),
                        ),
                    };
                    section.data.extend_from_slice(&pad_bytes);
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
            AsmItem::CfaAdvance { from, to } => {
                self.ensure_section()?;
                let sec_idx = self.current_section.unwrap();
                let offset = self.sections[sec_idx].data.len();
                self.deferred_cfa_advances.push(CfaAdvance {
                    sec_idx,
                    offset,
                    from: from.clone(),
                    to: to.clone(),
                });
                self.sections[sec_idx].data.extend([0u8; CFA_ADVANCE_MAX]);
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
                    } else if let Some((sym, addend)) = parse_org_style_skip(expr) {
                        // `LABEL + N - .` is a location-counter target. Emit
                        // it as `.org` so jump relaxation can restretch the
                        // padding (early_idt_handler_array).
                        self.process_org(&sym, addend, *fill)?;
                    } else {
                        let offset = self.sections[sec_idx].data.len();
                        self.deferred_skips.push(DeferredSkip {
                            sec_idx,
                            offset,
                            expr: expr.clone(),
                            fill: *fill,
                            seq: self.seq,
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
                            self.symver_flags
                                .insert(name.clone(), (ver_string.clone(), *f));
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
                    fill,
                },
                after_insn,
                nops: NopTable::for_mode(A::default_code_mode(), self.code_mode),
                tight_resolved_align: None,
                seq: self.seq,
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
            let idx =
                self.get_or_create_section(".text", SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR, None)?;
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
                        self.place_label(&dot_name, sec_idx, here);
                        dot_name.clone()
                    } else {
                        self.aliases.get(a).cloned().unwrap_or_else(|| a.clone())
                    };
                    let b_res = if b == "." {
                        self.place_label(&dot_name, sec_idx, here);
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
        } else if (self.code_mode == 16 || self.code_mode == 17) && A::default_code_mode() == 64 {
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
            if let Some((label, target_addend)) = self.get_jump_target_label(instr) {
                if jump_det.already_short {
                    // Short-only jumps (jecxz/jcxz/loop) - already short, just need displacement patched
                    self.sections[sec_idx].jumps.push(JumpInfo {
                        offset: base_offset as usize,
                        len: instr_len,
                        long_len: instr_len,
                        target: label,
                        target_addend,
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
                        target: label,
                        target_addend,
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

    fn get_jump_target_label(&self, instr: &Instruction) -> Option<(String, i64)> {
        // Same normalization as the encoder and the arch jump detectors:
        // GAS is case-insensitive and accepts `.s` on any mnemonic, so
        // `JMP` and `jmp.s` must register for relaxation exactly like `jmp`.
        let mnem_lower = instr.mnemonic.to_ascii_lowercase();
        let mnem: &str = mnem_lower.strip_suffix(".s").unwrap_or(&mnem_lower);
        let is_jump = mnem == "jmp" || mnem == "loop" || (mnem.starts_with('j') && mnem.len() >= 2);
        if !is_jump || instr.operands.len() != 1 {
            return None;
        }
        let Operand::Label(label) = &instr.operands[0] else {
            return None;
        };

        // The parser deliberately leaves bare control-flow operands
        // context-neutral.  Split the same symbol-plus-constant grammar used
        // by relocation emission here because short-only loop/jecxz branches
        // never emit a relocation: the relaxation engine patches their disp8
        // directly.  Applying the addend there exactly once avoids looking up
        // a non-existent literal label named `.Ltarget+1`.
        if let Some((base, addend)) =
            crate::backend::x86::assembler::parser::split_relocation_symbol_addend(label)
        {
            Some((base.to_string(), addend))
        } else {
            Some((label.clone(), 0))
        }
    }

    // ─── Layout edits ─────────────────────────────────────────────────

    /// Define (or redefine) label `name` at `offset` in `sec_idx`, stamped
    /// with the current source position for `shift_after`.
    fn place_label(&mut self, name: &str, sec_idx: usize, offset: u64) {
        self.label_positions
            .insert(name.to_string(), (sec_idx, offset));
        self.label_seq.insert(name.to_string(), self.seq);
    }

    /// Apply `f` to the offset of every record that locates something in
    /// section `sec_idx`.
    ///
    /// This is the one list of offset-bearing records; every edit of a
    /// section's bytes after emission (relaxation, alignment padding,
    /// deferred `.skip` fill, LEB128 and CFA-advance sizing) goes through
    /// it. Keeping a hand-written copy of the list per edit let them drift
    /// apart: scaled differences never moved, so `.long (b-a)*2` behind a
    /// relaxed jump was written at its pre-relaxation offset, over the data
    /// that followed it.
    ///
    /// `f` receives the offset and, for zero-width records (labels,
    /// alignment/org markers, deferred skips), their source position;
    /// byte-bearing records (relocations, jumps, deferred data fields) get
    /// `None`.
    fn remap_offsets(&mut self, sec_idx: usize, f: impl Fn(u64, Option<u64>) -> u64) {
        for (name, pos) in self.label_positions.iter_mut() {
            if pos.0 == sec_idx {
                let seq = self.label_seq.get(name).copied().unwrap_or(0);
                pos.1 = f(pos.1, Some(seq));
            }
        }
        for (num, positions) in self.numeric_label_positions.iter_mut() {
            let seqs = self.numeric_label_seq.get(num);
            for (i, pos) in positions.iter_mut().enumerate() {
                if pos.0 == sec_idx {
                    let seq = seqs.and_then(|v| v.get(i)).copied().unwrap_or(0);
                    pos.1 = f(pos.1, Some(seq));
                }
            }
        }
        let map = |off: usize, seq: Option<u64>| f(off as u64, seq) as usize;
        let sec = &mut self.sections[sec_idx];
        for reloc in sec.relocations.iter_mut() {
            reloc.offset = f(reloc.offset, None);
        }
        for jump in sec.jumps.iter_mut() {
            jump.offset = map(jump.offset, None);
        }
        for marker in sec.align_markers.iter_mut() {
            marker.offset = map(marker.offset, Some(marker.seq));
        }
        for skip in self.deferred_skips.iter_mut() {
            if skip.sec_idx == sec_idx {
                skip.offset = map(skip.offset, Some(skip.seq));
            }
        }
        for (bsec, boff, ..) in self.deferred_byte_diffs.iter_mut() {
            if *bsec == sec_idx {
                *boff = map(*boff, None);
            }
        }
        for (lsec, loff, ..) in self.deferred_leb_diffs.iter_mut() {
            if *lsec == sec_idx {
                *loff = map(*loff, None);
            }
        }
        for d in self.deferred_scaled_diffs.iter_mut() {
            if d.sec_idx == sec_idx {
                d.offset = map(d.offset, None);
            }
        }
        for a in self.deferred_cfa_advances.iter_mut() {
            if a.sec_idx == sec_idx {
                a.offset = map(a.offset, None);
            }
        }
    }

    /// Move everything in `sec_idx` that lies after an insertion
    /// (`delta > 0`) or removal (`delta < 0`) at offset `at`.
    ///
    /// Byte-bearing records at or beyond `at` start at or after the edit
    /// and always move. A zero-width record can sit exactly at `at` on
    /// either side of it: `tie_seq` is the source position of the edited
    /// item (an alignment directive, a `.skip`), and such a record moves
    /// only when it came later in the source. A label written just before
    /// `.p2align` stays in front of the padding however much padding
    /// relaxation later makes necessary -- `.size f, .-f` and `end - start`
    /// measurements otherwise absorbed the next function's alignment.
    /// `None` for edits inside a byte-bearing item (a jump, a LEB128 field),
    /// where nothing zero-width can tie.
    fn shift_after(&mut self, sec_idx: usize, at: usize, delta: i64, tie_seq: Option<u64>) {
        if delta == 0 {
            return;
        }
        let at = at as u64;
        self.remap_offsets(sec_idx, |off, seq| {
            let after = off > at
                || (off == at
                    && match (seq, tie_seq) {
                        (Some(s), Some(t)) => s > t,
                        _ => true,
                    });
            if after {
                off.wrapping_add_signed(delta)
            } else {
                off
            }
        });
    }

    // ─── Deferred skip resolution (x86-64 and i686) ──────────────────

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
                seq,
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

            // Everything after the gap moves; a label at the gap start
            // moves only if it was defined after the `.skip`.
            self.shift_after(*sec_idx, *offset, count as i64, Some(*seq));
        }
        Ok(())
    }

    /// Give every deferred CFA advance its smallest encoding.
    ///
    /// The operands are code labels, whose positions are final once
    /// relaxation, alignment and `.skip` sizing are done; resizing the
    /// section that holds the advances (`.eh_frame`, which contains no code
    /// the operands could live in) cannot move them. So each advance is
    /// sized exactly once, like GNU as's relaxed `rs_cfa` fragments, instead
    /// of always taking `DW_CFA_advance_loc4`, which made lccc's `.eh_frame`
    /// 69% larger than GNU as's for the same input (sqlite shell.c: 42160
    /// vs 24944 bytes).
    ///
    /// All advances of one section are rewritten in a single sweep and the
    /// section's records are remapped through the resulting shift table, so
    /// the cost stays linear in the number of advances; the section's
    /// alignment padding (the `DW_CFA_nop` fill ending each CIE/FDE) is then
    /// recomputed, and the FDE lengths -- label differences resolved later
    /// -- follow.
    fn resolve_cfa_advances(&mut self) -> Result<(), String> {
        let mut advances = std::mem::take(&mut self.deferred_cfa_advances);
        advances.sort_by_key(|a| (a.sec_idx, a.offset));
        let mut rest = advances.as_slice();
        while let Some(first) = rest.first() {
            let sec_idx = first.sec_idx;
            let n = rest.iter().take_while(|a| a.sec_idx == sec_idx).count();
            let (group, tail) = rest.split_at(n);
            rest = tail;

            let mut encoded: Vec<(usize, Vec<u8>)> = Vec::with_capacity(group.len());
            for a in group {
                let pos = |name: &str| {
                    self.label_positions
                        .get(name)
                        .copied()
                        .ok_or_else(|| format!("undefined label in CFA advance: {}", name))
                };
                let (from_sec, from_off) = pos(&a.from)?;
                let (to_sec, to_off) = pos(&a.to)?;
                if from_sec != to_sec || from_sec == sec_idx {
                    return Err(format!(
                        "internal: CFA advance {} - {} does not measure one code section",
                        a.to, a.from
                    ));
                }
                let delta = to_off.checked_sub(from_off).ok_or_else(|| {
                    format!("CFA advance goes backwards: {} precedes {}", a.to, a.from)
                })?;
                encoded.push((a.offset, encode_cfa_advance(delta)?));
            }

            // Rebuild the section, recording for each placeholder its old
            // end and the bytes removed up to there.
            let old = std::mem::take(&mut self.sections[sec_idx].data);
            let mut data = Vec::with_capacity(old.len());
            let mut shifts: Vec<(u64, u64)> = Vec::with_capacity(encoded.len());
            let (mut cursor, mut removed) = (0usize, 0u64);
            for (offset, bytes) in &encoded {
                data.extend_from_slice(&old[cursor..*offset]);
                data.extend_from_slice(bytes);
                cursor = offset + CFA_ADVANCE_MAX;
                removed += (CFA_ADVANCE_MAX - bytes.len()) as u64;
                shifts.push((cursor as u64, removed));
            }
            data.extend_from_slice(&old[cursor..]);
            self.sections[sec_idx].data = data;
            // Nothing points into a placeholder: a record at or after a
            // placeholder's end moves back by everything removed up to it.
            self.remap_offsets(sec_idx, |off, _| {
                match shifts.partition_point(|&(end, _)| end <= off) {
                    0 => off,
                    k => off - shifts[k - 1].1,
                }
            });
            self.fixup_alignment_markers(sec_idx);
        }
        Ok(())
    }

    fn resolve_deferred_byte_diffs(&mut self) -> Result<(), String> {
        // Fold deferred LEB128 diffs once label positions are known.
        // Resolve from the END of each section backwards so shrinking the
        // 10-byte placeholder does not invalidate earlier offsets.
        let mut leb_diffs = std::mem::take(&mut self.deferred_leb_diffs);
        leb_diffs.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
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
            // The placeholder's tail is gone: whatever followed it moves.
            self.shift_after(*sec_idx, *offset + encoded.len(), -(delta as i64), None);
        }
        // Shrinking only removed bytes, so every short jump across a
        // placeholder still reaches its target; its disp8 must follow.
        let mut shrunk: Vec<usize> = leb_diffs.iter().map(|d| d.0).collect();
        shrunk.dedup();
        for sec_idx in shrunk {
            if !self.sections[sec_idx].jumps.is_empty() {
                self.patch_short_jumps(sec_idx);
            }
        }
        // Taken only now: the placeholder shrinking above moves them.
        let diffs = std::mem::take(&mut self.deferred_byte_diffs);
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
                // `.` is the location counter at THIS directive (GAS
                // data-directive semantics; the SymbolDiffScaled path
                // materializes the same anchor as a synthetic .Ldot label).
                // The compressed boot's IDT limit uses it:
                // `.word . - boot32_idt - 1`.
                if sym == "." {
                    return Some((*sec_idx, *offset as u64));
                }
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

        // Now that the tight buckets are final, recompute section header
        // alignments: an ordinary `.p2align` records its (fixed) alignment at
        // parse time, but a tight marker's bucket can flip accept -> reject
        // when a deferred skip grows the body past one cache line, and the
        // section-alignment raise of an earlier sweep must be revoked along
        // with its padding (a stale sh_addralign of 64 is not a correctness
        // bug, but it over-promises the section placement and diverges from
        // GAS's 16 after the rejection).
        self.reconcile_section_alignments();

        // CFA advances measure code, which is final now.
        self.resolve_cfa_advances()?;

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
                // TLS relocs are NOT foldable (see X86Arch::is_tls_reloc):
                // the linker resolves them through the STT_TLS symbol, and a
                // GOT-style TLS reloc's addend must remain 0.
                let is_foldable_local = self.label_positions.contains_key(&reloc.symbol)
                    && !self.pending_globals.contains(&reloc.symbol)
                    && !self.pending_weaks.contains(&reloc.symbol)
                    && !A::is_tls_reloc(reloc.reloc_type);
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
                // A section symbol has no PLT entry: GAS rewrites a PLT32
                // (implicit on an x86-64 branch, or an explicit `@PLT`)
                // against a symbol it folds into its section to PC32. The
                // value computed is identical, the relocation now says so
                // (GAS 2.47, `--64` and `--32`, cross-section local target).
                let reloc_type = if is_foldable_local && reloc.reloc_type == A::reloc_plt32() {
                    A::reloc_pc32()
                } else {
                    reloc.reloc_type
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
                        reloc_type,
                        symbol_name: sym_name,
                        addend: 0,
                    });
                } else {
                    relocs.push(ObjReloc {
                        offset: reloc.offset,
                        reloc_type,
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

        // GAS symbol-table parity for GOT-base computing relocations:
        // every `@GOTPCREL`-family / `@GOTTPOFF` / `@TLSDESC` / `@TLSGD` /
        // `@TLSLD` / `@GOT` / `@GOTOFF` / `@GOTPC` / `@TPOFF` / `@DTPOFF` /
        // `@GOTNTPOFF` operator makes GNU as record an undefined
        // `_GLOBAL_OFFSET_TABLE_` reference (GLOBAL, UND, NOTYPE) in the
        // symbol table — that is how GNU ld resolves the GOT base for
        // these computations, and how tools that inspect the object
        // (unwinder tables, `.eh_frame` checkers) expect to see it.
        // One occurrence per object, deduplicated against an explicit
        // definition or reference elsewhere in the assembly. The class
        // set lives in `X86Arch::needs_got_base_symbol` (measured against
        // GAS 2.47 per operator).
        if !shared_symbols
            .iter()
            .any(|s| s.name == "_GLOBAL_OFFSET_TABLE_")
            && self.sections.iter().any(|sec| {
                sec.relocations
                    .iter()
                    .any(|r| A::needs_got_base_symbol(r.reloc_type))
            })
        {
            shared_symbols.push(ObjSymbol {
                name: "_GLOBAL_OFFSET_TABLE_".to_string(),
                value: 0,
                size: 0,
                binding: STB_GLOBAL,
                sym_type: STT_NOTYPE,
                visibility: STV_DEFAULT,
                section_name: "*UND*".to_string(),
            });
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
                    let target_off_opt = jump_target_with_addend(
                        local_labels.get(&jump.target).copied().or_else(|| {
                            self.resolve_numeric_label(&jump.target, jump.offset as u64, sec_idx)
                                .map(|(_, off)| off as usize)
                        }),
                        jump.target_addend,
                    );
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
                    } else if dbg && jump.relaxed && jump.can_grow && !fits_short {
                        eprintln!("[RELAX] sec{sec_idx} j{j_idx} misses disp8 in the snapshot");
                    }
                }
                // Growth passes decide in one sequential sweep, as GAS does
                // (see `plan_jump_growth`), not against the pass's snapshot.
                // A first pass that shrinks nothing is a growth pass too: the
                // second `relax_jumps` call (after deferred `.skip`s resolve)
                // starts with every jump already short, and a `.skip` that
                // grew inside a loop must still grow its back-edge. A pass
                // that does shrink leaves growth to the next pass, which it
                // always forces.
                if !first_pass || actions.is_empty() {
                    for j_idx in self.plan_jump_growth(sec_idx, &local_labels) {
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
                            // The long form's displacement relocation goes
                            // (the short form is resolved internally), then
                            // everything after the jump moves.
                            let old_reloc_pos = if is_conditional {
                                offset + 2
                            } else {
                                offset + 1
                            } as u64;
                            self.sections[sec_idx]
                                .relocations
                                .retain(|reloc| reloc.offset != old_reloc_pos);
                            self.shift_after(sec_idx, offset + 1, -(shrink as i64), None);
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
                            // Everything after the jump moves; the new
                            // displacement relocation is added afterwards so
                            // it is not moved with it.
                            self.shift_after(sec_idx, offset + 1, grow as i64, None);
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

            self.patch_short_jumps(sec_idx);
        }
    }

    /// Offset and source position of the label a jump targets, if it is
    /// defined in `sec_idx` (named labels from `local_labels`, numeric
    /// `Nb`/`Nf` references resolved from the jump's own offset).
    fn jump_target_label(
        &self,
        jump: &JumpInfo,
        sec_idx: usize,
        local_labels: &FxHashMap<String, usize>,
    ) -> Option<(usize, u64)> {
        if let Some(&off) = local_labels.get(&jump.target) {
            let seq = self.label_seq.get(&jump.target).copied().unwrap_or(0);
            return Some((off, seq));
        }
        let (_, off) = self.resolve_numeric_label(&jump.target, jump.offset as u64, sec_idx)?;
        let num = &jump.target[..jump.target.len() - 1];
        let seq = self
            .numeric_label_positions
            .get(num)
            .and_then(|positions| positions.iter().position(|&p| p == (sec_idx, off)))
            .and_then(|i| self.numeric_label_seq.get(num).and_then(|v| v.get(i)))
            .copied()
            .unwrap_or(0);
        Some((off as usize, seq))
    }

    /// Decide which short jumps in `sec_idx` must grow, in one sequential
    /// sweep over the current layout -- GNU as's `relax_segment` order.
    ///
    /// GAS visits frags in address order carrying the growth so far
    /// (`stretch`): an item's address is updated before it is sized, so a
    /// BACKWARD target (and every alignment in between) already reflects
    /// this pass's growth, while a FORWARD target is its old address plus
    /// `stretch` -- without the stretch when it is positive and an
    /// alignment or `.org` lies in between (`relax_frag`'s relax regions:
    /// the padding may absorb it, so the reach is not overestimated). Deciding every jump against the pass's starting snapshot
    /// instead sees stale padding: a loop back-edge across a `.p2align N,,M`
    /// whose padding shrinks once an earlier jump grows was grown on the
    /// snapshot's larger distance and, jumps never shrinking again, kept
    /// long -- a larger fixed point than GAS's (RX-1: sqlite shell.c
    /// `do_meta_command` +16 bytes, `qrfEqpRender` +8).
    ///
    /// The sweep is virtual (the section is edited once per pass, by the
    /// caller). An item's value in this pass is its offset plus the growth
    /// of the items visited before it that precede it -- by offset, and at
    /// equal offsets by source position, the `shift_after` tie rule.
    /// Alignment markers are re-padded at their new offset (max-skip
    /// included), `.org` markers against their label's new position; a
    /// tight-loop marker (an lccc policy with no GAS counterpart, sized from
    /// the very spans being decided) keeps its padding here and is
    /// re-derived by the physical fixup after the pass.
    ///
    /// Sound and terminating: jumps only grow, so passes are bounded; the
    /// pass that grows nothing starts from markers the fixup made
    /// consistent, keeps a zero stretch, and therefore checks every short
    /// jump against the true layout.
    fn plan_jump_growth(
        &self,
        sec_idx: usize,
        local_labels: &FxHashMap<String, usize>,
    ) -> Vec<usize> {
        enum Item {
            Marker(usize),
            Jump(usize),
        }
        let sec = &self.sections[sec_idx];
        let mut items: Vec<(usize, u8, u64, Item)> = Vec::new();
        for (i, m) in sec.align_markers.iter().enumerate() {
            items.push((m.offset, 0, m.seq, Item::Marker(i)));
        }
        for (i, j) in sec.jumps.iter().enumerate() {
            if j.relaxed && j.can_grow {
                items.push((j.offset, 1, 0, Item::Jump(i)));
            }
        }
        // A marker at a jump's offset is its (empty) leading padding.
        items.sort_by_key(|&(off, rank, seq, _)| (off, rank, seq));
        // Every marker opens a GAS relax region (`rs_align*`/`rs_org` frag;
        // the tight-loop marker stands for a `.p2align`).
        let mut regions: Vec<(usize, u64)> = sec
            .align_markers
            .iter()
            .map(|m| (m.offset, m.seq))
            .collect();
        regions.sort_unstable();

        // Visited growth, in visiting order: (offset, marker seq or None for
        // a jump) and the running total of the deltas up to each entry.
        // Visiting order is (offset, markers-before-jump, seq), so the
        // entries preceding any label form a PREFIX: binary-searchable.
        let mut visited: Vec<(usize, Option<u64>)> = Vec::new();
        let mut cumulative: Vec<i64> = Vec::new();
        let mut stretch: i64 = 0;
        // New position of a label at `off` defined at source position `seq`:
        // growth inside a jump moves only what follows its opcode byte; a
        // marker's padding moves a label at its own offset only when the
        // label came later in the source.
        let moved = |visited: &[(usize, Option<u64>)], cumulative: &[i64], off: usize, seq: u64| {
            let before = visited.partition_point(|&(at, marker_seq)| {
                at < off || (at == off && marker_seq.is_some_and(|ms| ms < seq))
            });
            off as i64 + before.checked_sub(1).map_or(0, |i| cumulative[i])
        };
        let mut grow = Vec::new();
        for &(off, _, _, ref item) in &items {
            let here = off as i64 + stretch;
            match *item {
                Item::Jump(j_idx) => {
                    let jump = &sec.jumps[j_idx];
                    let Some((label_off, label_seq)) =
                        self.jump_target_label(jump, sec_idx, local_labels)
                    else {
                        continue;
                    };
                    let target = if label_off > off {
                        // Not reached yet: GAS `relax_frag` assumes it moves
                        // by `stretch` too -- unless the stretch is positive
                        // and an alignment/`.org` (a new relax region) lies
                        // in between, which may absorb it; then it keeps its
                        // old position, and a jump whose target would fall
                        // behind its displacement byte does not grow.
                        let old = label_off as i64 + jump.target_addend;
                        let next = regions.partition_point(|&(m, _)| m <= off);
                        let crosses = regions.get(next).is_some_and(|&(m, ms)| {
                            m < label_off || (m == label_off && label_seq > ms)
                        });
                        if stretch < 0 || !crosses {
                            old + stretch
                        } else if old < here + jump.len as i64 - 1 {
                            continue;
                        } else {
                            old
                        }
                    } else {
                        moved(&visited, &cumulative, label_off, label_seq) + jump.target_addend
                    };
                    let disp = target - (here + 2);
                    if !(-128..=127).contains(&disp) {
                        let delta = (jump.long_len - jump.len) as i64;
                        stretch += delta;
                        visited.push((off, None));
                        cumulative.push(stretch);
                        grow.push(j_idx);
                    }
                }
                Item::Marker(m_idx) => {
                    let marker = &sec.align_markers[m_idx];
                    let needed = match &marker.kind {
                        AlignMarkerKind::Align {
                            align, max_skip, ..
                        } => {
                            if *align <= 1 {
                                continue;
                            }
                            let pad =
                                ((here as u64).div_ceil(*align) * *align - here as u64) as usize;
                            match max_skip {
                                Some(skip) if pad as u64 > *skip => 0,
                                _ => pad,
                            }
                        }
                        AlignMarkerKind::Org { label, addend, .. } => {
                            // GAS `rs_org`: the label's value is updated once
                            // reached this pass, else its old position (no
                            // stretch); a backwards `.org` keeps its padding.
                            let end = if label.is_empty() {
                                *addend
                            } else {
                                match self.label_positions.get(label.as_str()) {
                                    Some(&(l_sec, l_off)) if l_sec == sec_idx => {
                                        let l_seq = self
                                            .label_seq
                                            .get(label.as_str())
                                            .copied()
                                            .unwrap_or(0);
                                        let l_off = l_off as usize;
                                        let reached =
                                            l_off < off || (l_off == off && l_seq < marker.seq);
                                        if reached {
                                            moved(&visited, &cumulative, l_off, l_seq) + *addend
                                        } else {
                                            l_off as i64 + *addend
                                        }
                                    }
                                    _ => continue,
                                }
                            };
                            if end < here {
                                continue;
                            }
                            (end - here) as usize
                        }
                        AlignMarkerKind::TightLoop { .. } => continue,
                    };
                    let delta = needed as i64 - marker.padding as i64;
                    if delta != 0 {
                        stretch += delta;
                        visited.push((off, Some(marker.seq)));
                        cumulative.push(stretch);
                    }
                }
            }
        }
        grow
    }

    /// Write the disp8 of every short jump in `sec_idx` from the current
    /// layout. Runs once relaxation is stable, and again after anything
    /// later removes bytes from a code section (a shrinking `.uleb128`),
    /// which moves targets without touching the jumps.
    fn patch_short_jumps(&mut self, sec_idx: usize) {
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
                let target = jump_target_with_addend(
                    local_labels.get(&jump.target).copied().or_else(|| {
                        self.resolve_numeric_label(&jump.target, jump.offset as u64, sec_idx)
                            .map(|(_, off)| off as usize)
                    }),
                    jump.target_addend,
                );
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

    /// Recompute each section's header alignment from the FINAL marker
    /// decisions. Ordinary `.p2align`/`.balign` carry fixed textual
    /// alignments (matching GAS, including the 2^63 non-recording case);
    /// tight markers contribute the bucket their last fixup sweep
    /// resolved (`tight_resolved_align`), or nothing after a reject.
    fn reconcile_section_alignments(&mut self) {
        for section in &mut self.sections {
            let mut alignment = 1u64;
            for marker in &section.align_markers {
                match &marker.kind {
                    AlignMarkerKind::Align { align, .. } => {
                        if *align > alignment && *align < 1u64 << 63 {
                            alignment = *align;
                        }
                    }
                    AlignMarkerKind::TightLoop { .. } => {
                        if let Some(a) = marker.tight_resolved_align {
                            alignment = alignment.max(a);
                        }
                    }
                    AlignMarkerKind::Org { .. } => {}
                }
            }
            section.alignment = alignment;
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
            // Set by the tight arm below; persisted on the marker AFTER the
            // padding run is rebuilt so the last sweep's decision (which may
            // flip accept -> reject when a deferred skip grows the body) is
            // the only one that reaches the section-alignment reconciliation.
            let mut tight_resolved_align: Option<u64> =
                self.sections[sec_idx].align_markers[marker_idx].tight_resolved_align;

            let needed_end = match &kind {
                AlignMarkerKind::Align { align, .. } => {
                    let a = *align;
                    if a <= 1 {
                        marker_idx += 1;
                        continue;
                    }
                    // Overflow-safe round-up (an alignment of 2^63 from a
                    // GAS-clamped exponent makes the mask form wrap).
                    (current_offset as u64).div_ceil(a) * a
                }
                AlignMarkerKind::Org {
                    label,
                    addend,
                    fill: _,
                } => {
                    if label.is_empty() {
                        *addend as u64
                    } else if let Some(&(l_sec, l_off)) = self.label_positions.get(label.as_str()) {
                        if l_sec == sec_idx {
                            (l_off as i64 + *addend) as u64
                        } else {
                            marker_idx += 1;
                            continue;
                        }
                    } else {
                        marker_idx += 1;
                        continue;
                    }
                }
                AlignMarkerKind::TightLoop { header } => {
                    // Resolve the exact encoded loop span from the
                    // post-relaxation layout: header label .. end of
                    // the first branch jumping back to it (the latch),
                    // which is precisely the body GCC's
                    // align_tight_loops measures with ix86_min_insn_size.
                    //
                    // The decision is re-derived on EVERY fixup sweep
                    // (deferred `.skip` resolution can grow a span that
                    // was measured earlier), so a rejection must collapse
                    // any padding a previous sweep inserted: it yields the
                    // marker's own offset (needed padding 0) and the common
                    // splice path below then removes the stale NOP run.
                    let dbg = std::env::var("CCC_DEBUG_TIGHT").is_ok();
                    let mut reject_reason: Option<&'static str> = None;
                    // Start every sweep from None: an earlier sweep's
                    // acceptance must not survive a later rejection (the
                    // padding run is rebuilt the same way below).
                    tight_resolved_align = None;
                    let (target, bucket): (Option<u64>, Option<u64>) = 'arm: {
                        let Some(&(h_sec, h_off)) = self.label_positions.get(header.as_str())
                        else {
                            reject_reason = Some("unresolved-header");
                            break 'arm (None, None);
                        };
                        if h_sec != sec_idx {
                            reject_reason = Some("cross-section");
                            break 'arm (None, None);
                        }
                        let Some(jump) = self.sections[sec_idx]
                            .jumps
                            .iter()
                            .filter(|j| j.target == *header && (j.offset as u64) >= h_off)
                            .min_by_key(|j| j.offset)
                        else {
                            reject_reason = Some("no-backedge");
                            break 'arm (None, None);
                        };
                        let body_end = jump.offset + jump.len;
                        let size = body_end.saturating_sub(h_off as usize);
                        let Some(log2) = tight_bucket_log2(size as u64) else {
                            reject_reason = Some(if size == 0 { "empty" } else { "span>64" });
                            break 'arm (None, None);
                        };
                        let align = 1u64 << log2;
                        if dbg {
                            eprintln!(
                                "[TIGHT] sec{} header={} span={} log2={} align={}",
                                sec_idx, header, size, log2, align
                            );
                        }
                        (
                            Some((current_offset as u64).div_ceil(align) * align),
                            Some(align),
                        )
                    };
                    if let Some(why) = reject_reason {
                        if dbg {
                            eprintln!("[TIGHT] sec{} header={} reject={}", sec_idx, header, why);
                        }
                    }
                    // Record this sweep's bucket (None on reject); the
                    // section header alignment is reconciled after the
                    // fixed point so a later reject revokes the raise.
                    tight_resolved_align = bucket;
                    // None: keep/restore zero padding; the ordinary scalar
                    // cascade markers following this one still apply.
                    target.unwrap_or(current_offset as u64)
                }
            };

            let mut needed_padding = needed_end.saturating_sub(current_offset as u64) as usize;
            // Re-evaluate the max-skip against the RELAXED offset: the skip
            // decision is a function of the padding actually required at the
            // final layout, not the pre-relaxation one.
            if let AlignMarkerKind::Align {
                max_skip: Some(skip),
                ..
            } = &kind
            {
                if needed_padding as u64 > *skip {
                    needed_padding = 0;
                }
            }
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
                let nops = self.sections[sec_idx].align_markers[marker_idx].nops;
                // `.org` / org-style `.fill LABEL+N-.` pad with the fill
                // byte, not multi-byte NOPs, even in an executable section.
                // An `.align` with an explicit non-`0x90` fill byte pads
                // verbatim (GAS tc-i386 keeps multi-byte NOPs only for the
                // default/0x90 fill in executable sections).
                let new_bytes = match &kind {
                    AlignMarkerKind::Org { fill, .. } => vec![*fill; needed_padding],
                    AlignMarkerKind::Align { fill, .. } => match fill {
                        Some(f) if is_exec && *f != 0x90 => vec![*f; needed_padding],
                        Some(f) if !is_exec => vec![*f; needed_padding],
                        _ => section_padding(needed_padding, is_exec, after_insn, nops),
                    },
                    // Tight-loop padding is unconditional max-skip-0 style
                    // alignment and always uses optimal multi-byte NOPs.
                    AlignMarkerKind::TightLoop { .. } => {
                        section_padding(needed_padding, is_exec, after_insn, nops)
                    }
                };
                debug_assert_eq!(new_bytes.len(), needed_padding);
                if old_end <= self.sections[sec_idx].data.len() {
                    self.sections[sec_idx]
                        .data
                        .splice(start..old_end, new_bytes);
                    let delta = needed_padding as i64 - existing_padding as i64;
                    if delta != 0 {
                        // Anchor the shift at the run's END. When the run was
                        // empty, END is `start` itself, and source order
                        // decides which labels precede the padding.
                        let seq = self.sections[sec_idx].align_markers[marker_idx].seq;
                        self.shift_after(sec_idx, old_end, delta, Some(seq));
                    }
                }
            }
            // Persist the adjusted size so repeated fixup passes are idempotent.
            self.sections[sec_idx].align_markers[marker_idx].padding = needed_padding;
            if matches!(kind, AlignMarkerKind::TightLoop { .. }) {
                self.sections[sec_idx].align_markers[marker_idx].tight_resolved_align =
                    tight_resolved_align;
            }

            marker_idx += 1;
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
                        self.place_label(&label_name, sec_idx, offset);
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
            let sh_offset =
                u64::from_le_bytes(obj[off + 24..off + 32].try_into().unwrap()) as usize;
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
                return Some(u32::from_le_bytes(
                    obj[off + 4..off + 8].try_into().unwrap(),
                ));
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

    fn section_bytes(obj: &[u8], name: &str) -> Option<Vec<u8>> {
        let e_shoff = u64::from_le_bytes(obj[40..48].try_into().unwrap()) as usize;
        let e_shentsize = u16::from_le_bytes(obj[58..60].try_into().unwrap()) as usize;
        let e_shnum = u16::from_le_bytes(obj[60..62].try_into().unwrap()) as usize;
        let shstr = {
            let shstr_idx = u16::from_le_bytes(obj[62..64].try_into().unwrap()) as usize;
            let off = e_shoff + shstr_idx * e_shentsize;
            let sh_offset =
                u64::from_le_bytes(obj[off + 24..off + 32].try_into().unwrap()) as usize;
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
                let sh_offset =
                    u64::from_le_bytes(obj[off + 24..off + 32].try_into().unwrap()) as usize;
                let sh_size =
                    u64::from_le_bytes(obj[off + 32..off + 40].try_into().unwrap()) as usize;
                return Some(obj[sh_offset..sh_offset + sh_size].to_vec());
            }
        }
        None
    }

    /// Linux `early_idt_handler_array` (head_64.S): each stub is
    /// `endbr64; [push $0;] push $n; jmp common` padded with
    /// `.fill array + (i+1)*SIZE - ., 1, 0xcc` to exactly SIZE bytes.
    /// The jmp is far, so it stays rel32 (5 bytes). No-error body is 13
    /// bytes; error-code body is 11 + 2×cc. Slots must not grow when
    /// jump relaxation runs.
    #[test]
    fn early_idt_handler_fill_slots_are_exact() {
        let dir = std::env::temp_dir().join(format!(
            "lccc_idt_{}_{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("t")
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("idt.o");
        let asm = concat!(
            ".text\n",
            ".globl array\n",
            "array:\n",
            "i = 0\n",
            ".rept 12\n",
            "    endbr64\n",
            "    .if (i == 8) || (i == 10)\n",
            "        pushq $i\n",
            "    .else\n",
            "        pushq $0\n",
            "        pushq $i\n",
            "    .endif\n",
            "    jmp common\n",
            "    .fill array + (i + 1) * 13 - ., 1, 0xcc\n",
            "    i = i + 1\n",
            ".endr\n",
            ".skip 400, 0x90\n",
            "common:\n",
            "    ret\n",
        );
        assemble(asm, out.to_str().unwrap()).expect("assemble early_idt shape");
        let obj = std::fs::read(&out).unwrap();
        let text = section_bytes(&obj, ".text").expect(".text");
        const SIZE: usize = 13;
        for n in 0..12 {
            let off = n * SIZE;
            assert_eq!(
                &text[off..off + 4],
                &[0xf3, 0x0f, 0x1e, 0xfa],
                "vector {n} does not start at {off}: next 8 = {:02x?}",
                &text[off..off + 8.min(text.len() - off)]
            );
        }
        assert_ne!(text[10 * SIZE], 0xcc, "vector 10 drifted into padding");
        for n in [8usize, 10] {
            let off = n * SIZE;
            assert_eq!(text[off + 11], 0xcc, "vec {n} missing fill");
            assert_eq!(text[off + 12], 0xcc, "vec {n} missing fill");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_org_style_skip_recognizes_named_label_dot_diff() {
        use super::parse_org_style_skip;
        assert_eq!(
            parse_org_style_skip("early_idt_handler_array + 9 * 13 - ."),
            Some(("early_idt_handler_array".into(), 117))
        );
        assert_eq!(
            parse_org_style_skip("array+13-."),
            Some(("array".into(), 13))
        );
        assert_eq!(parse_org_style_skip("array - ."), Some(("array".into(), 0)));
        assert_eq!(parse_org_style_skip("0b + 16 - ."), None);
        assert_eq!(parse_org_style_skip("16"), None);
    }

    /// Assemble a text blob and return the raw `.text` bytes (test-only).
    fn assemble_object(asm: &str) -> Vec<u8> {
        let dir = std::env::temp_dir().join(format!(
            "lccc_tight_{}_{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("t")
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("m.o");
        assemble(asm, out.to_str().unwrap()).unwrap();
        let data = std::fs::read(&out).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        data
    }

    fn assemble_text(asm: &str) -> Vec<u8> {
        let data = assemble_object(asm);
        section_bytes(&data, ".text").expect(".text present")
    }

    /// `sh_addralign` of `name` from an ELF64 object written by this module.
    fn section_addralign(obj: &[u8], name: &str) -> Option<u64> {
        let e_shoff = u64::from_le_bytes(obj[40..48].try_into().unwrap()) as usize;
        let e_shentsize = u16::from_le_bytes(obj[58..60].try_into().unwrap()) as usize;
        let e_shnum = u16::from_le_bytes(obj[60..62].try_into().unwrap()) as usize;
        let shstr = {
            let shstr_idx = u16::from_le_bytes(obj[62..64].try_into().unwrap()) as usize;
            let off = e_shoff + shstr_idx * e_shentsize;
            let sh_offset =
                u64::from_le_bytes(obj[off + 24..off + 32].try_into().unwrap()) as usize;
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
                return Some(u64::from_le_bytes(
                    obj[off + 48..off + 56].try_into().unwrap(),
                ));
            }
        }
        None
    }

    /// A synthetic tight loop: one odd preamble byte, the marker, a body
    /// whose exact encoded span is `body_nops + 4` bytes
    /// (`xor %eax,%eax` = 2, `jmp .L1` relaxed to 2 short), and the latch.
    fn tight_loop_text(body_nops: usize) -> String {
        format!(
            ".text\n\
             .globl f\n\
             f:\n\
             \tnop\n\
             \t.lccc_tight_loop .L1\n\
             .L1:\n\
             \txor %eax,%eax\n\
             \t.rept {n}\n\
             \tnop\n\
             \t.endr\n\
             \tjmp .L1\n",
            n = body_nops
        )
    }

    /// Offset of the 2-byte `xor %eax,%eax` (31 c0) marker sentinel — the
    /// loop header's first body instruction.
    fn body_offset(text: &[u8]) -> usize {
        text.windows(2)
            .position(|w| w == [0x31, 0xc0])
            .expect("xor body sentinel present")
    }

    #[test]
    fn tight_loop_marker_buckets_match_encoded_span() {
        // span = body_nops + 4: buckets 3/4/5/6 at the inclusive 8/16/32/64
        // cache-line bucket edges.
        for (n, align) in [(4usize, 8usize), (12, 16), (28, 32), (60, 64)] {
            let obj = assemble_object(&tight_loop_text(n));
            let text = section_bytes(&obj, ".text").unwrap();
            let off = body_offset(&text);
            assert_eq!(
                off % align,
                0,
                "span={} body must start at a {align}-byte boundary, got {off}",
                n + 4
            );
            // The accepted marker raises sh_addralign to its bucket.
            assert_eq!(
                section_addralign(&obj, ".text"),
                Some(align as u64),
                "span={} must record {align}-byte section alignment",
                n + 4
            );
        }
    }

    #[test]
    fn tight_loop_marker_rejects_oversize_body() {
        // span 65 (61 + 4): one byte past a cache line -> fail closed, the
        // marker contributes ZERO padding (header follows the 1-byte
        // preamble directly).
        let text = assemble_text(&tight_loop_text(61));
        let off = body_offset(&text);
        assert_eq!(off, 1, "oversize span must not pad, header at {off}");
    }

    #[test]
    fn tight_loop_marker_rejects_without_backedge() {
        // No branch jumps back to the marker's header: no padding.
        let asm = ".text\n\
                   .globl f\n\
                   f:\n\
                   \tnop\n\
                   \t.lccc_tight_loop .L1\n\
                   .L1:\n\
                   \txor %eax,%eax\n\
                   \tnop\n";
        let text = assemble_text(asm);
        assert_eq!(body_offset(&text), 1);

        // An unresolved header label is skipped, not an error: the private
        // marker must never break the assembly by itself.
        let asm2 = ".text\n\
                    .globl g\n\
                    g:\n\
                    \t.lccc_tight_loop .Lghost\n\
                    \txor %eax,%eax\n";
        let text2 = assemble_text(asm2);
        assert_eq!(body_offset(&text2), 0);
    }

    #[test]
    fn tight_loop_rejected_after_skip_growth_removes_its_padding() {
        // A genuinely symbolic (deferred) `.skip Lz - Lb` inside the body
        // only resolves AFTER the first fixup sweep: the marker first
        // accepts a 44-byte span (bucket 64) and pads the pre-header byte
        // up to the 64 boundary (63 NOPs), then the 90-byte skip splices
        // into the body, growing the span well past one cache line. The
        // later reject must COLLAPSE that already-inserted tight padding
        // — no stale NOP run may survive an accepted -> rejected flip
        // (verified via CCC_DEBUG_TIGHT: accept, accept, reject x3).
        let asm = ".text\n\
                   .globl f\n\
                   f:\n\
                   \tnop\n\
                   \t.lccc_tight_loop .L1\n\
                   .L1:\n\
                   \txor %eax,%eax\n\
                   \t.rept 40\n\
                   \tnop\n\
                   \t.endr\n\
                   \t.skip .Lz - .Lb\n\
                   \tjmp .L1\n\
                   .Lb:\n\
                   \t.rept 90\n\
                   \tnop\n\
                   \t.endr\n\
                   .Lz:\n\
                   \tret\n";
        let obj = assemble_object(asm);
        let text = section_bytes(&obj, ".text").unwrap();
        // No scalar cascade exists in this hand-written text, so the
        // collapsed marker leaves the header one byte after f.
        assert_eq!(
            body_offset(&text),
            1,
            "stale tight padding survived an accept -> reject flip"
        );
        // The rejected marker must also REVOKE its section-alignment raise:
        // this text requests no other alignment, so sh_addralign returns to
        // its 1-byte default rather than staying at 64 from an early sweep.
        assert_eq!(
            section_addralign(&obj, ".text"),
            Some(1),
            "stale 64-byte sh_addralign survived an accept -> reject flip"
        );
    }

    #[test]
    fn tight_loop_padding_and_relaxation_reach_fixed_point() {
        // The tight marker can insert up to 63 bytes. An OUTER loop whose
        // backedge jumps over the aligned inner loop must be re-grown from
        // its relaxed short form when the padding pushes it past disp8
        // range. Here the outer backedge spans 126 bytes without padding
        // (short `eb` fits at disp -126) and 130 once the marker pads the
        // inner header to 64 (must become a near `e9` rel32). The relax ->
        // pad -> grow -> pad fixed point must converge; the inner header
        // still lands on its 64-byte bucket boundary.
        let asm = ".text\n\
                   .globl f\n\
                   f:\n\
                   .Louter:\n\
                   \t.rept 58\n\
                   \tnop\n\
                   \t.endr\n\
                   \ttest %ecx,%ecx\n\
                   \t.lccc_tight_loop .Lin\n\
                   .Lin:\n\
                   \txor %eax,%eax\n\
                   \t.rept 60\n\
                   \tnop\n\
                   \t.endr\n\
                   \tjmp .Lin\n\
                   \tjmp .Louter\n";
        let text = assemble_text(asm);
        let off = body_offset(&text);
        assert_eq!(off, 64, "64-byte inner body header must align to 64");
        // The inner infinite loop is 64 bytes (2 xor + 60 nop + 2 jmp).
        let outer_jmp = 64 + 64;
        assert_eq!(
            text[outer_jmp], 0xe9,
            "outer backedge at {outer_jmp} must be near jmp rel32 after padding, got {:02x}",
            text[outer_jmp]
        );
    }

    /// Assemble x86-64 `asm` and return the contents of section `name`.
    fn assembled_section(asm: &str, name: &str) -> Vec<u8> {
        let dir = std::env::temp_dir().join(format!(
            "lccc_layout_{}_{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("t")
                .replace("::", "_")
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("t.o");
        assemble(asm, out.to_str().unwrap()).unwrap();
        let data = std::fs::read(&out).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        section_bytes(&data, name).expect("section present")
    }

    // The layout-edit tests below pin GNU as 2.47's bytes for the same
    // input. Each shape broke because one edit path did not move one kind
    // of record (see `remap_offsets`).

    /// A scaled difference behind a jump that relaxes to its short form is
    /// folded into its own field, not three bytes later over the data.
    /// GNU as re-enters an existing section on a bare `.section NAME` (or
    /// empty flags) with its attributes unchanged -- GCC writes exactly that
    /// for `.gcc_except_table` in every `-fexceptions` function after the
    /// first -- and rejects only explicitly contradicting flags.
    #[test]
    fn section_reentry_checks_only_stated_attributes() {
        let text = assembled_section(
            concat!(
                ".section .foo,\"a\",@progbits\n.byte 1\n.text\nnop\n",
                ".section .foo\n.byte 2\n.section .foo,\"\"\n.byte 3\n",
                ".section .foo,\"\",@progbits\n.byte 4\n",
            ),
            ".foo",
        );
        assert_eq!(text, [1, 2, 3, 4]);
        let dir = std::env::temp_dir().join(format!(
            "lccc_secre_{}_{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("t")
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("e.o");
        let err = assemble(
            ".section .foo,\"a\",@progbits\n.byte 1\n.section .foo,\"aw\",@progbits\n",
            out.to_str().unwrap(),
        )
        .expect_err("contradicting flags must be rejected");
        assert!(err.contains("changed section attributes"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `(r_offset, r_type)` of every entry in an ELF64 RELA section.
    fn rela_types(obj: &[u8], name: &str) -> Vec<(u64, u32)> {
        section_bytes(obj, name)
            .expect("relocation section present")
            .chunks_exact(24)
            .map(|e| {
                let off = u64::from_le_bytes(e[0..8].try_into().unwrap());
                let info = u64::from_le_bytes(e[8..16].try_into().unwrap());
                (off, info as u32)
            })
            .collect()
    }

    /// A section symbol has no PLT entry: a PLT32 -- implicit on a branch or
    /// an explicit `@PLT` -- against a local symbol that folds into its
    /// section is written as PC32; global targets keep PLT32 (GAS 2.47).
    #[test]
    fn plt32_against_folded_local_becomes_pc32() {
        let obj = assemble_object(concat!(
            ".text\njmp lcold\ncall lcold@PLT\ncall ext\ncall ext@PLT\n",
            ".section .text.unlikely,\"ax\",@progbits\nlcold: ret\n",
        ));
        let mut rel = rela_types(&obj, ".rela.text");
        rel.sort();
        // R_X86_64_PC32 = 2, R_X86_64_PLT32 = 4.
        assert_eq!(rel, [(1, 2), (6, 2), (11, 4), (16, 4)]);
    }

    /// Executable padding reproduces GNU as 2.47 byte for byte in every NOP
    /// table: each entry, the last all-NOP gap, the first jump-over, the
    /// rel32 jump-over (with the operand-size prefix in 16-bit code) and
    /// the leading `nop` after data.
    #[test]
    fn exec_padding_tables_match_gas() {
        use super::{NopTable, exec_padding};
        let pad = |n, t| exec_padding(n, true, t);
        let l32_8: &[u8] = &[0x2e, 0x8d, 0xb4, 0x26, 0, 0, 0, 0];
        assert_eq!(pad(3, NopTable::Lea32), [0x8d, 0x76, 0x00]);
        assert_eq!(pad(8, NopTable::Lea32), l32_8);
        assert_eq!(
            pad(11, NopTable::Lea32),
            [&[0x8d, 0x76, 0x00], l32_8].concat()
        );
        assert_eq!(pad(23, NopTable::Lea32)[..3], [0x8d, 0xb4, 0x26]);
        assert_eq!(
            pad(24, NopTable::Lea32),
            [&[0xeb, 0x16, 0x8d, 0xb6, 0, 0, 0, 0], l32_8, l32_8].concat()
        );
        assert_eq!(
            pad(130, NopTable::Lea32)[..10],
            [0xe9, 0x7d, 0, 0, 0, 0x2e, 0x8d, 0x74, 0x26, 0]
        );
        assert_eq!(pad(3, NopTable::Lea64), [0x48, 0x89, 0xf6]);
        assert_eq!(pad(26, NopTable::Lea64)[..4], [0x48, 0x8d, 0xb4, 0x26]);
        assert_eq!(
            pad(27, NopTable::Lea64)[..5],
            [0xeb, 0x19, 0x48, 0x8d, 0xb6]
        );
        assert_eq!(pad(2, NopTable::Lea16), [0x89, 0xf6]);
        assert_eq!(pad(14, NopTable::Lea16)[..4], [0x8d, 0xb4, 0, 0]);
        assert_eq!(pad(15, NopTable::Lea16)[..5], [0xeb, 0x0d, 0x8d, 0x74, 0]);
        assert_eq!(
            pad(130, NopTable::Lea16)[..10],
            [0x66, 0xe9, 0x7c, 0, 0, 0, 0x8d, 0xb4, 0, 0]
        );
        assert_eq!(pad(3, NopTable::Long), [0x0f, 0x1f, 0x00]);
        assert_eq!(pad(88, NopTable::Long)[..2], [0xeb, 0x56]);
        assert_eq!(
            exec_padding(4, false, NopTable::Lea32),
            [0x90, 0x8d, 0x76, 0x00]
        );
        for t in [
            NopTable::Long,
            NopTable::Lea32,
            NopTable::Lea64,
            NopTable::Lea16,
        ] {
            for n in 0..300 {
                assert_eq!(pad(n, t).len(), n, "{t:?} {n}");
            }
        }
        assert_eq!(NopTable::for_mode(64, 64), NopTable::Long);
        assert_eq!(NopTable::for_mode(64, 32), NopTable::Long);
        assert_eq!(NopTable::for_mode(32, 32), NopTable::Lea32);
        assert_eq!(NopTable::for_mode(32, 64), NopTable::Lea64);
        assert_eq!(NopTable::for_mode(32, 16), NopTable::Lea16);
        assert_eq!(NopTable::for_mode(32, 17), NopTable::Lea16);
    }

    /// `.text` of `head`, `n` single-byte NOPs, `tail` -- the relaxation
    /// sweep cases below need long straight runs between their branches.
    fn relax_case(parts: &[(&str, usize)]) -> Vec<u8> {
        let mut asm = String::from(".text\nf:\n");
        for &(text, nops) in parts {
            asm.push_str(text);
            asm.push_str(&"nop\n".repeat(nops));
        }
        assemble_text(&asm)
    }

    /// Growth decisions follow GAS's in-order sweep: once the far `jmp`
    /// grows (+3), the `.p2align 4,,10` between the loop head and its
    /// back-edge re-pads 10 -> 7 BEFORE the back-edge is sized, so the
    /// back-edge still fits disp8 (-127). Judged against the pass's
    /// starting layout it saw -130 and grew for good (RX-1).
    #[test]
    fn backedge_across_shrinking_maxskip_align_stays_short() {
        let text = relax_case(&[
            ("jmp .Lfar\n.Lb:\n", 4),
            (".p2align 4,,10\n", 114),
            ("jne .Lb\n", 130),
            (".Lfar:\nret\n", 0),
        ]);
        assert_eq!(text.len(), 263);
        assert_eq!(text[0], 0xe9);
        assert_eq!(text[0x82..0x84], [0x75, 0x81]);
    }

    /// Same, with the back-edge target exactly at the alignment's offset
    /// but defined before the directive: the re-padding moves the branch,
    /// not the target (the source-order tie rule of `shift_after`).
    #[test]
    fn backedge_target_before_align_at_same_offset_stays_short() {
        let text = relax_case(&[
            ("jmp .Lfar\n", 4),
            (".Lb:\n.p2align 4,,10\n", 118),
            ("jne .Lb\n", 130),
            (".Lfar:\nret\n", 0),
        ]);
        assert_eq!(text.len(), 267);
        assert_eq!(text[134..136], [0x75, 0x81]);
    }

    /// GAS relax regions: a positive stretch is not pushed onto a not yet
    /// visited target behind an alignment (the padding may absorb it), so
    /// the forward `jne` is sized against its target's old position
    /// (+125, short) rather than old + stretch (+128, long) (RX-2).
    #[test]
    fn forward_target_behind_align_keeps_old_position() {
        let text = relax_case(&[
            ("jmp .Lfar\n", 4),
            ("jne .Lt\n", 112),
            (".p2align 4,,10\n", 8),
            (".Lt:\nret\n", 130),
            (".Lfar:\nret\n", 0),
        ]);
        assert_eq!(text.len(), 268);
        assert_eq!(text[9..11], [0x75, 0x7d]);
    }

    /// An `.org` anchored before the growing `jmp` loses fill as the jump
    /// grows, which the back-edge across it must see in the same sweep.
    #[test]
    fn backedge_across_org_anchored_before_growth_stays_short() {
        let text = relax_case(&[
            (".Lbase:\njmp .Lfar\n.Lb:\nnop\n.org .Lbase+20\n", 110),
            ("jne .Lb\n", 130),
            (".Lfar:\nret\n", 0),
        ]);
        assert_eq!(text.len(), 263);
        assert_eq!(text[0x82..0x84], [0x75, 0x81]);
    }

    #[test]
    fn scaled_difference_moves_with_relaxation() {
        let asm = "\
.text
f:
jmp .Lx
.long (.Lb-.La)*2
.byte 0xaa, 0xbb, 0xcc, 0xdd
.La:
nop
nop
.Lb:
.Lx:
ret
";
        assert_eq!(
            assembled_section(asm, ".text"),
            [
                0xeb, 0x0a, 0x04, 0, 0, 0, 0xaa, 0xbb, 0xcc, 0xdd, 0x90, 0x90, 0xc3
            ]
        );
    }

    /// A label written before `.p2align` stays in front of padding that
    /// only relaxation makes necessary (the alignment was satisfied when the
    /// directive was read); the label after it moves.
    #[test]
    fn label_before_alignment_stays_before_grown_padding() {
        let asm = "\
.text
f:
jmp .Lt
.byte 0,0,0,0,0,0,0,0,0,0,0
.Lend:
.p2align 4
.Lt:
ret
.data
.long .Lend - f
.long .Lt - f
";
        assert_eq!(
            assembled_section(asm, ".data"),
            [0x0d, 0, 0, 0, 0x10, 0, 0, 0]
        );
    }

    /// A `.uleb128` label difference behind a relaxed jump lands in its own
    /// field, and the jump across it still reaches its target once the
    /// placeholder shrinks.
    #[test]
    fn uleb_difference_moves_with_relaxation_and_jumps_follow_it() {
        let asm = "\
.text
f:
jmp .Lx
.uleb128 .Lb - .La
.byte 0xaa, 0xbb
.La:
nop
nop
nop
.Lb:
.Lx:
ret
";
        assert_eq!(
            assembled_section(asm, ".text"),
            [0xeb, 0x06, 0x03, 0xaa, 0xbb, 0x90, 0x90, 0x90, 0xc3]
        );
    }
}
