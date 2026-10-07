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
    /// Symbol types from .type directives
    pub symbol_types: FxHashMap<String, u8>,
    /// Symbol sizes from .size directives
    pub symbol_sizes: FxHashMap<String, u64>,
    /// Symbol visibility from .hidden/.protected/.internal
    pub symbol_visibility: FxHashMap<String, u8>,
    /// Symbol aliases from .set/.equ directives
    pub aliases: FxHashMap<String, String>,
    /// Section stack for .pushsection/.popsection (saves both current and previous section)
    section_stack: Vec<(String, String)>,
    /// Previous section for .section/.previous swapping
    previous_section: String,
    /// NOP instruction bytes for code section alignment padding.
    /// ARM: `[0x1f, 0x20, 0x03, 0xd5]` (d503201f), RISC-V: `[0x13, 0x00, 0x00, 0x00]` (00000013)
    nop_bytes: [u8; 4],
    /// Default text section alignment (4 for ARM, 2 for RISC-V with compressed instructions)
    text_align: u64,
    /// Bytes of directive-generated fill materialized so far by this
    /// writer — bounded by [`MAX_TOTAL_DIRECTIVE_FILL`] (see there).
    fill_budget_used: usize,
}

impl ElfWriterBase {
    pub fn new(nop_bytes: [u8; 4], text_align: u64) -> Self {
        Self {
            current_section: String::new(),
            sections: FxHashMap::default(),
            section_order: Vec::new(),
            extra_symbols: Vec::new(),
            labels: FxHashMap::default(),
            global_symbols: FxHashMap::default(),
            weak_symbols: FxHashMap::default(),
            symbol_types: FxHashMap::default(),
            symbol_sizes: FxHashMap::default(),
            symbol_visibility: FxHashMap::default(),
            aliases: FxHashMap::default(),
            section_stack: Vec::new(),
            previous_section: String::new(),
            nop_bytes,
            text_align,
            fill_budget_used: 0,
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
                    sh_addralign: align,
                    relocs: Vec::new(),
                    comdat_group: None,
                },
            );
            self.section_order.push(name.to_string());
        }
    }

    /// Get the current write offset within the current section.
    pub fn current_offset(&self) -> u64 {
        self.sections
            .get(&self.current_section)
            .map(|s| s.data.len() as u64)
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
            section.data.extend_from_slice(bytes);
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
    pub fn emit_fill(&mut self, size: usize, fill: u8) -> Result<(), String> {
        if size > MAX_DIRECTIVE_FILL {
            return Err(format!(
                "fill directive of {size} bytes exceeds the {MAX_DIRECTIVE_FILL}-byte limit"
            ));
        }
        self.ensure_default_section();
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
    /// Enforces, in GAS 2.47 order:
    /// 1. the per-directive ceiling (before any allocation),
    /// 2. the NOBITS *raw-value* law (`.fill 4,1,256` in `.bss` is an
    ///    error even though the truncated element stores zeros — the
    ///    check sees `value`, not the encoded bytes),
    /// 3. the assembly-wide fill budget,
    /// 4. one allocation + a tight slice-copy loop.
    pub fn emit_fill_pattern(
        &mut self,
        total: usize,
        elem: usize,
        value: u64,
    ) -> Result<(), String> {
        debug_assert!((1..=8).contains(&elem), "parser clamps elem to 1..=8");
        if total > MAX_DIRECTIVE_FILL {
            return Err(format!(
                "fill directive of {total} bytes exceeds the {MAX_DIRECTIVE_FILL}-byte limit"
            ));
        }
        self.ensure_default_section();
        if value != 0 {
            if let Some(section) = self.sections.get(&self.current_section)
                && section.sh_type == SHT_NOBITS
            {
                return Err(format!(
                    "attempt to fill section `{}` with non-zero value",
                    section.name
                ));
            }
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
    /// is `None`, executable sections emit the architecture NOP at NOP-sized
    /// boundaries and zeros in the unaligned prefix; other sections zero-fill.
    /// That is the GNU as 2.44/2.47 pattern for `.byte 1; .p2align 3`.
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
        if align <= 1 {
            return Ok(());
        }
        self.ensure_default_section();
        let nop = self.nop_bytes.clone();
        // Phase 1 — decide under a single shared borrow (no allocation).
        let decided = {
            let Some(section) = self.sections.get(&self.current_section) else {
                return Ok(());
            };
            let current = section.data.len() as u64;
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
            if padding > MAX_DIRECTIVE_FILL {
                return Err(format!(
                    "alignment padding of {padding} bytes exceeds the {MAX_DIRECTIVE_FILL}-byte limit"
                ));
            }
            padding
        };
        // Phase 2 — budget BEFORE materialization (and before the
        // sh_addralign raise, so an over-budget pad leaves no trace).
        self.charge_fill(decided)?;
        // Phase 3 — materialize + raise under an exclusive borrow.
        if let Some(section) = self.sections.get_mut(&self.current_section) {
            let padding = decided;
            if padding > 0 {
                if let Some(b) = fill {
                    section.data.extend(std::iter::repeat_n(b, padding));
                } else if section.sh_flags & SHF_EXECINSTR != 0 && align >= 4 {
                    let full_nops = padding / 4;
                    let remainder = padding % 4;
                    for _ in 0..full_nops {
                        section.data.extend_from_slice(&nop);
                    }
                    section.data.extend(std::iter::repeat_n(0u8, remainder));
                } else {
                    section.data.extend(std::iter::repeat_n(0u8, padding));
                }
            }
            // GAS records the alignment even when the pad was skipped.
            if align > section.sh_addralign {
                section.sh_addralign = align;
            }
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
    ) {
        let sh_type = match sec_type_str {
            Some("@nobits") => SHT_NOBITS,
            Some("@note") => SHT_NOTE,
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

        if sh_flags == 0 && !flags_explicit {
            sh_flags = default_section_flags(sec_name);
        }

        // Always created at 1: GAS raises AX sections only when an
        // instruction actually lands in them (see `note_instruction`).
        self.ensure_section(sec_name, sh_type, sh_flags, 1);
        self.previous_section = std::mem::replace(&mut self.current_section, sec_name.to_string());
    }

    /// Switch to a named standard section (.text, .data, .bss, .rodata).
    pub fn switch_to_standard_section(&mut self, name: &str, sh_type: u32, sh_flags: u64) {
        // Always created at 1 — instructions raise later (`note_instruction`).
        self.ensure_section(name, sh_type, sh_flags, 1);
        self.previous_section = std::mem::replace(&mut self.current_section, name.to_string());
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
    ) {
        self.section_stack
            .push((self.current_section.clone(), self.previous_section.clone()));
        self.process_section_directive(name, flags_str, flags_explicit, sec_type);
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
    pub fn merge_subsections(&mut self) -> FxHashMap<String, (String, u64)> {
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
            return remap;
        }

        // For each parent, append subsections in order
        for (parent, subs) in &subsections {
            for sub_name in subs.values() {
                let sub_data;
                let sub_relocs;
                {
                    let sub_sec = match self.sections.get(sub_name) {
                        Some(s) => s,
                        None => continue,
                    };
                    sub_data = sub_sec.data.clone();
                    sub_relocs = sub_sec.relocs.clone();
                }

                let parent_len = self
                    .sections
                    .get(parent)
                    .map(|s| s.data.len() as u64)
                    .unwrap_or(0);

                // Record the remapping for callers
                remap.insert(sub_name.clone(), (parent.clone(), parent_len));

                // Append data
                if let Some(parent_sec) = self.sections.get_mut(parent) {
                    parent_sec.data.extend_from_slice(&sub_data);
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

        remap
    }

    /// Record .globl for a symbol.
    pub fn set_global(&mut self, sym: &str) {
        self.global_symbols.insert(sym.to_string(), true);
    }

    /// Record .weak for a symbol.
    pub fn set_weak(&mut self, sym: &str) {
        self.weak_symbols.insert(sym.to_string(), true);
    }

    /// Record symbol visibility (.hidden, .protected, .internal).
    pub fn set_visibility(&mut self, sym: &str, vis: u8) {
        self.symbol_visibility.insert(sym.to_string(), vis);
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

    /// Emit a .comm symbol (COMMON block).
    pub fn emit_comm(&mut self, sym: &str, size: u64, align: u64) {
        self.extra_symbols.push(ObjSymbol {
            name: sym.to_string(),
            value: align,
            size,
            binding: STB_GLOBAL,
            sym_type: STT_OBJECT,
            visibility: STV_DEFAULT,
            section_name: "*COM*".to_string(),
        });
    }

    /// Record a .set/.equ alias.
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
                // Chase alias chain
                let mut resolved = sym;
                let mut seen = 0;
                while let Some(target) = self.aliases.get(resolved) {
                    resolved = target.as_str();
                    seen += 1;
                    if seen > 20 {
                        break;
                    }
                }
                result.push_str(resolved);
            } else {
                result.push(c as char);
                i += 1;
            }
        }
        result
    }

    /// Resolve label names in an expression to their numeric offsets.
    /// This handles `.Ldot_N` synthetic labels (current position) and any
    /// section-local labels that can be resolved to constant offsets.
    pub fn resolve_expr_labels(&self, expr: &str) -> String {
        let cur_section = &self.current_section;
        let cur_offset = self.current_offset();
        let mut result = String::with_capacity(expr.len());
        let bytes = expr.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c == b'.' || c == b'_' || c.is_ascii_alphabetic() {
                let start = i;
                i += 1;
                while i < bytes.len()
                    && (bytes[i] == b'.' || bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric())
                {
                    i += 1;
                }
                let sym = &expr[start..i];
                // Standalone '.' means current position
                if sym == "." {
                    result.push_str(&cur_offset.to_string());
                // Check if this is a .Ldot_N label (current position)
                } else if sym.starts_with(".Ldot_") {
                    result.push_str(&cur_offset.to_string());
                } else if let Some((sec, off)) = self.labels.get(sym) {
                    if sec == cur_section {
                        result.push_str(&off.to_string());
                    } else {
                        result.push_str(sym);
                    }
                } else {
                    result.push_str(sym);
                }
            } else {
                result.push(c as char);
                i += 1;
            }
        }
        result
    }

    /// Resolve ALL label names in an expression to their numeric offsets,
    /// using the specified section as context. Unlike `resolve_expr_labels`,
    /// this also resolves `.Ldot_N` labels from their stored definitions
    /// (not the current offset), making it suitable for deferred resolution.
    pub fn resolve_expr_all_labels(&self, expr: &str, section: &str) -> String {
        let mut result = String::with_capacity(expr.len());
        let bytes = expr.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c == b'.' || c == b'_' || c.is_ascii_alphabetic() {
                let start = i;
                i += 1;
                while i < bytes.len()
                    && (bytes[i] == b'.' || bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric())
                {
                    i += 1;
                }
                let sym = &expr[start..i];
                if let Some((sec, off)) = self.labels.get(sym) {
                    if sec == section {
                        result.push_str(&off.to_string());
                    } else {
                        result.push_str(sym);
                    }
                } else {
                    result.push_str(sym);
                }
            } else {
                result.push(c as char);
                i += 1;
            }
        }
        result
    }

    /// Resolve label names in an expression to their offsets regardless of section.
    ///
    /// Unlike `resolve_expr_all_labels` which only resolves labels in the same
    /// section, this resolves ALL labels to their offsets. This is safe for
    /// expressions that compute differences between labels in the same section
    /// (the section offsets cancel out), which is common in kernel ALTERNATIVE
    /// macros (e.g., `889f - 888f` computing the size of alternative code).
    pub fn resolve_expr_cross_section(&self, expr: &str) -> String {
        let resolved = self.resolve_expr_aliases(expr);
        let mut result = String::with_capacity(resolved.len());
        let bytes = resolved.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c == b'.' || c == b'_' || c.is_ascii_alphabetic() {
                let start = i;
                i += 1;
                while i < bytes.len()
                    && (bytes[i] == b'.' || bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric())
                {
                    i += 1;
                }
                let sym = &resolved[start..i];
                if let Some((_sec, off)) = self.labels.get(sym) {
                    result.push_str(&off.to_string());
                } else {
                    result.push_str(sym);
                }
            } else {
                result.push(c as char);
                i += 1;
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

    /// Emit a symbol reference with a relocation.
    pub fn emit_data_symbol_ref(
        &mut self,
        sym: &str,
        addend: i64,
        size: usize,
        reloc_type: u32,
    ) -> Result<(), String> {
        // A relocation is content: NOBITS sections cannot carry one
        // (GAS: "attempt to store non-zero value").
        self.nobits_guard(true)?;
        self.add_reloc(reloc_type, sym.to_string(), addend);
        match size {
            1 => self.emit_bytes(&[0u8])?,
            2 => self.emit_bytes(&0u16.to_le_bytes())?,
            4 => self.emit_bytes(&0u32.to_le_bytes())?,
            _ => self.emit_bytes(&0u64.to_le_bytes())?,
        }
        Ok(())
    }

    /// Emit placeholder bytes for a deferred value (symbol diff, etc.).
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
        // Remove UND entries for any symbols that are also in extra_symbols (e.g. COMMON).
        // A symbol that is both referenced in relocations and declared as COMMON should
        // only appear once (as COMMON), not as both UND and COMMON.
        for extra in &self.extra_symbols {
            if extra.section_name == "*COM*" {
                symbols.retain(|s| !(s.name == extra.name && s.section_name == "*UND*"));
            }
        }
        symbols.append(&mut self.extra_symbols);

        let elf_bytes =
            write_relocatable_object(config, &self.section_order, &self.sections, &symbols)?;

        std::fs::write(output_path, &elf_bytes)
            .map_err(|e| format!("failed to write ELF file: {}", e))?;

        Ok(())
    }
}
