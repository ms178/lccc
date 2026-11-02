//! AArch64 linker types and constants.
//!
//! Defines the `GlobalSymbol` type used by all linker phases, plus
//! architecture-specific constants (base address, page size, interpreter path).

use super::elf::SHN_COMMON;
use super::elf::SHN_UNDEF;
use super::elf::{DT_FINI, DT_INIT};
use crate::backend::linker_common;
use crate::common::fx_hash::FxHashMap;
use linker_common::{Elf64Symbol, GlobalSymbolOps};

/// Dynamic linker path for AArch64
pub const INTERP: &[u8] = b"/lib/ld-linux-aarch64.so.1\0";

/// Base virtual address for the executable
pub const BASE_ADDR: u64 = 0x400000;
/// Page size for alignment
pub const PAGE_SIZE: u64 = 0x10000; // AArch64 uses 64KB pages for linker alignment

/// A resolved global symbol
#[derive(Clone)]
pub struct GlobalSymbol {
    pub value: u64,
    pub size: u64,
    pub info: u8,
    pub defined_in: Option<usize>,
    pub section_idx: u16,
    /// SONAME of the shared library this symbol was resolved from
    pub from_lib: Option<String>,
    /// PLT entry index (for dynamic function symbols)
    pub plt_idx: Option<usize>,
    /// GOT entry index (for dynamic symbols needing GOT slots)
    pub got_idx: Option<usize>,
    /// Whether this symbol is resolved from a shared library
    pub is_dynamic: bool,
    /// Whether this symbol needs a copy relocation
    pub copy_reloc: bool,
    /// Symbol's value in the source shared library (for alias detection)
    pub lib_sym_value: u64,
    /// True when `lib_sym_value` falls inside a `PF_X` LOAD segment of the
    /// defining library (see `DynSymbol::in_exec_segment`). Only meaningful
    /// for dynamic symbols; lets the COPY-vs-PLT classifier tell untyped
    /// data (copy) from untyped code (PLT) when `info` is `STT_NOTYPE`.
    pub lib_in_exec: bool,
    /// Merged `STV_*` visibility (gABI: the most constraining of all
    /// references and the definition — see
    /// `linker_common::merge_object_visibility`).  Drives the shared
    /// emitter's export decision, the `st_other` byte of dynsym entries and
    /// the ADR admission predicate, so the input-visibility contract and the
    /// emitted ELF can no longer disagree (PR #821 F2).
    pub visibility: u8,
}

impl GlobalSymbolOps for GlobalSymbol {
    fn is_defined(&self) -> bool {
        self.defined_in.is_some()
    }
    fn is_dynamic(&self) -> bool {
        self.is_dynamic
    }
    fn info(&self) -> u8 {
        self.info
    }
    fn section_idx(&self) -> u16 {
        self.section_idx
    }
    fn value(&self) -> u64 {
        self.value
    }
    fn size(&self) -> u64 {
        self.size
    }
    fn visibility(&self) -> u8 {
        self.visibility
    }
    fn set_visibility(&mut self, visibility: u8) {
        self.visibility = visibility;
    }
    fn new_defined(obj_idx: usize, sym: &Elf64Symbol) -> Self {
        GlobalSymbol {
            value: sym.value,
            size: sym.size,
            info: sym.info,
            defined_in: Some(obj_idx),
            from_lib: None,
            plt_idx: None,
            got_idx: None,
            section_idx: sym.shndx,
            is_dynamic: false,
            copy_reloc: false,
            lib_sym_value: 0,
            lib_in_exec: false,
            visibility: sym.visibility(),
        }
    }
    fn new_common(obj_idx: usize, sym: &Elf64Symbol) -> Self {
        GlobalSymbol {
            value: sym.value,
            size: sym.size,
            info: sym.info,
            defined_in: Some(obj_idx),
            from_lib: None,
            plt_idx: None,
            got_idx: None,
            section_idx: SHN_COMMON,
            is_dynamic: false,
            copy_reloc: false,
            lib_sym_value: 0,
            lib_in_exec: false,
            visibility: sym.visibility(),
        }
    }
    fn new_undefined(sym: &Elf64Symbol) -> Self {
        GlobalSymbol {
            value: 0,
            size: 0,
            info: sym.info,
            defined_in: None,
            from_lib: None,
            plt_idx: None,
            got_idx: None,
            section_idx: SHN_UNDEF,
            is_dynamic: false,
            copy_reloc: false,
            lib_sym_value: 0,
            lib_in_exec: false,
            visibility: sym.visibility(),
        }
    }
    fn set_common_bss(&mut self, bss_offset: u64) {
        self.value = bss_offset;
        self.section_idx = 0xffff;
    }
    fn new_dynamic(dsym: &linker_common::DynSymbol, soname: &str) -> Self {
        GlobalSymbol {
            value: 0,
            size: dsym.size,
            info: dsym.info,
            defined_in: None,
            from_lib: Some(soname.to_string()),
            plt_idx: None,
            got_idx: None,
            section_idx: SHN_UNDEF,
            is_dynamic: true,
            copy_reloc: false,
            lib_sym_value: dsym.value,
            lib_in_exec: dsym.in_exec_segment,
            // Shared-library symbols are resolved against, not linked in:
            // their visibility does not constrain this link unit (the merge
            // pass deliberately skips them), so DEFAULT is the sound value.
            visibility: crate::backend::elf::STV_DEFAULT,
        }
    }
}

/// ARM-specific replacement policy: also replace dynamic symbols with local definitions.
/// The `DT_INIT`/`DT_FINI` entries to emit for a presence decision made by
/// [`linker_common::image_init_fini`], read once addresses are final.  The entry count always
/// equals the number of `true` flags, which is what `.dynamic` was sized for.
pub(super) fn init_fini_entries(
    globals: &FxHashMap<String, GlobalSymbol>,
    present: (bool, bool),
) -> impl Iterator<Item = (i64, u64)> + '_ {
    [(present.0, "_init", DT_INIT), (present.1, "_fini", DT_FINI)]
        .into_iter()
        .filter(|&(p, _, _)| p)
        .map(|(_, name, tag)| (tag, globals.get(name).map_or(0, |g| g.value)))
}

pub fn arm_should_replace_extra(existing: &GlobalSymbol) -> bool {
    existing.is_dynamic
}

/// The `.eh_frame_hdr` slot of an AArch64 output, shared by the static,
/// dynamic and shared-library emitters: reserved right after the read-only
/// sections are laid out (so the read-only PT_LOAD covers it), filled from
/// the relocated `.eh_frame` once every relocation is applied, and pointed
/// at by PT_GNU_EH_FRAME -- the only way libgcc's unwinder finds the FDEs of
/// a dynamically linked object (`dl_iterate_phdr`).
pub(super) struct EhFrameHdr {
    pub offset: u64,
    pub vaddr: u64,
    pub size: u64,
    pub fde_count: usize,
    eh_frame_offset: u64,
    eh_frame_vaddr: u64,
    eh_frame_size: u64,
}

impl EhFrameHdr {
    /// Whether the output gets a header: a non-empty `.eh_frame`. Known
    /// before layout, so emitters can size their program headers with it.
    pub(super) fn wanted(output_sections: &[linker_common::OutputSection]) -> bool {
        output_sections
            .iter()
            .any(|s| s.name == ".eh_frame" && s.mem_size > 0)
    }

    /// Reserve `12 + 8 * FDEs` bytes, 4-aligned, at `*offset` (file offset;
    /// the address is `base_addr + offset`). `.eh_frame` must be laid out.
    pub(super) fn reserve(
        objects: &[linker_common::Elf64Object],
        output_sections: &[linker_common::OutputSection],
        offset: &mut u64,
        base_addr: u64,
    ) -> Option<Self> {
        let ef = output_sections
            .iter()
            .find(|s| s.name == ".eh_frame" && s.mem_size > 0)?;
        // Counted per input: the section data is not merged yet.
        let fde_count = ef
            .inputs
            .iter()
            .map(|i| {
                linker_common::count_eh_frame_fdes(
                    &objects[i.object_idx].section_data[i.section_idx],
                )
            })
            .sum::<usize>();
        *offset = (*offset + 3) & !3;
        let hdr = Self {
            offset: *offset,
            vaddr: base_addr + *offset,
            size: (12 + 8 * fde_count) as u64,
            fde_count,
            eh_frame_offset: ef.file_offset,
            eh_frame_vaddr: ef.addr,
            eh_frame_size: ef.mem_size,
        };
        *offset += hdr.size;
        Some(hdr)
    }

    /// Fill the slot from the relocated `.eh_frame` bytes in `out`.
    pub(super) fn write(&self, out: &mut [u8]) -> Result<(), String> {
        let ef =
            self.eh_frame_offset as usize..(self.eh_frame_offset + self.eh_frame_size) as usize;
        let at = self.offset as usize..(self.offset + self.size) as usize;
        if ef.end > out.len() || at.end > out.len() {
            return Err(format!(
                "internal: .eh_frame ({ef:?}) or .eh_frame_hdr ({at:?}) beyond the {}-byte output",
                out.len()
            ));
        }
        let mut slot = vec![0u8; at.len()];
        linker_common::write_eh_frame_hdr(
            &mut slot,
            &out[ef],
            self.eh_frame_vaddr,
            self.vaddr,
            true,
        );
        out[at].copy_from_slice(&slot);
        Ok(())
    }
}
