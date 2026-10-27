//! Section-header tables for the i686 emitters (executable and `-shared`).
//!
//! The loader reads only program headers, but everything else that opens an
//! ELF file reads the section headers: gdb and addr2line find DWARF through
//! them, `strip`/`objcopy` rebuild the file from them, and GNU ld locates a
//! library's `.dynsym`/`.dynstr`/`.dynamic` with them. A table must therefore
//! describe *every* byte range the emitter wrote -- a partial one would let
//! `objcopy` drop the synthesized regions it does not know about -- so each
//! emitter lists its synthesized regions next to its output sections, and this
//! module orders, links and serialises them.

use super::types::*;
use crate::common::fx_hash::FxHashMap;

/// One section header; `out_idx` is the output section it describes, or
/// `None` for a region the emitter synthesised (`.dynsym`, `.plt`, ...).
pub(super) struct Shdr32 {
    pub name: String,
    pub out_idx: Option<usize>,
    pub ty: u32,
    pub flags: u32,
    pub addr: u32,
    pub offset: u32,
    pub size: u32,
    pub link: u32,
    pub info: u32,
    pub align: u32,
    pub entsize: u32,
}

/// A region the emitter synthesises. Its conventional name fixes the
/// header's type, flags, alignment and entry size; the match below is
/// exhaustive, so a new region cannot ship without its header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StdSection {
    Interp,
    BuildId,
    Hash,
    GnuHash,
    Dynsym,
    Dynstr,
    Versym,
    Verneed,
    RelDyn,
    RelIplt,
    Relr,
    RelPlt,
    EhFrameHdr,
    Dynamic,
    GotPlt,
    IgotPlt,
    Plt,
    Iplt,
    Got,
    Dynbss,
}

impl StdSection {
    pub fn name(self) -> &'static str {
        match self {
            Self::Interp => ".interp",
            Self::BuildId => ".note.gnu.build-id",
            Self::Hash => ".hash",
            Self::GnuHash => ".gnu.hash",
            Self::Dynsym => ".dynsym",
            Self::Dynstr => ".dynstr",
            Self::Versym => ".gnu.version",
            Self::Verneed => ".gnu.version_r",
            Self::RelDyn => ".rel.dyn",
            Self::RelIplt => ".rel.iplt",
            Self::Relr => ".relr.dyn",
            Self::RelPlt => ".rel.plt",
            Self::EhFrameHdr => ".eh_frame_hdr",
            Self::Dynamic => ".dynamic",
            Self::GotPlt => ".got.plt",
            Self::IgotPlt => ".igot.plt",
            Self::Plt => ".plt",
            Self::Iplt => ".iplt",
            Self::Got => ".got",
            Self::Dynbss => ".dynbss",
        }
    }
}

impl Shdr32 {
    /// The header of a synthesised region at `addr`.
    pub fn standard(region: StdSection, addr: u32, offset: u32, size: u32) -> Self {
        use StdSection as S;
        const A: u32 = SHF_ALLOC;
        const AW: u32 = SHF_ALLOC | SHF_WRITE;
        const AX: u32 = SHF_ALLOC | SHF_EXECINSTR;
        let (ty, flags, align, entsize) = match region {
            S::Interp => (SHT_PROGBITS, A, 1, 0),
            S::BuildId => (SHT_NOTE, A, 4, 0),
            S::Hash => (SHT_HASH, A, 4, 4),
            S::GnuHash => (SHT_GNU_HASH, A, 4, 0),
            S::Dynsym => (SHT_DYNSYM, A, 4, 16),
            S::Dynstr => (SHT_STRTAB, A, 1, 0),
            S::Versym => (SHT_GNU_VERSYM, A, 2, 2),
            S::Verneed => (SHT_GNU_VERNEED, A, 4, 0),
            S::RelDyn | S::RelIplt => (SHT_REL, A, 4, 8),
            S::Relr => (SHT_RELR, A, 4, 4),
            S::RelPlt => (SHT_REL, A | SHF_INFO_LINK, 4, 8),
            S::Plt => (SHT_PROGBITS, AX, 16, 4),
            S::Iplt => (SHT_PROGBITS, AX, 8, 8),
            S::EhFrameHdr => (SHT_PROGBITS, A, 4, 0),
            S::Dynamic => (SHT_DYNAMIC, AW, 4, 8),
            S::Got | S::GotPlt | S::IgotPlt => (SHT_PROGBITS, AW, 4, 4),
            // Copy-relocated objects: aligned per object, so claim only
            // what every slot start satisfies.
            S::Dynbss => (SHT_NOBITS, AW, 1 << addr.trailing_zeros().min(12), 0),
        };
        Shdr32 {
            name: region.name().to_string(),
            out_idx: None,
            ty,
            flags,
            addr,
            offset,
            size,
            link: 0,
            info: 0,
            align,
            entsize,
        }
    }

    /// Headers for the synthesised regions that exist: each entry is
    /// `(name, present, addr, offset, size)`; empty regions get none.
    pub fn standard_all<'a>(
        regions: &'a [(StdSection, bool, u32, u32, u32)],
    ) -> impl Iterator<Item = Self> + 'a {
        regions
            .iter()
            .filter(|r| r.1 && r.4 > 0)
            .map(|&(region, _, addr, offset, size)| Self::standard(region, addr, offset, size))
    }

    /// Headers for the output sections that occupy something: every placed
    /// allocated section (an empty one still marks an address, e.g. for
    /// `__start_SEC`) and every non-empty debug section.
    pub fn of_outputs(output_sections: &[OutputSection]) -> impl Iterator<Item = Self> + '_ {
        output_sections
            .iter()
            .enumerate()
            .filter(|(_, sec)| {
                !sec.data.is_empty() || (sec.flags & SHF_ALLOC != 0 && sec.addr != 0)
            })
            .map(|(i, sec)| Self::of_output(i, sec))
    }

    fn of_output(index: usize, sec: &OutputSection) -> Self {
        Shdr32 {
            name: sec.name.clone(),
            out_idx: Some(index),
            ty: sec.sh_type,
            flags: sec.flags,
            addr: sec.addr,
            offset: sec.file_offset,
            size: sec.data.len() as u32,
            link: 0,
            info: 0,
            align: sec.align.max(1),
            entsize: 0,
        }
    }
}

/// Put the headers in their final order and fill in the `sh_link`/`sh_info`
/// of the synthesised dynamic-linking sections.
///
/// Allocated sections go in address order (a zero-sized section before a
/// sized one at the same address, NOBITS after PROGBITS: `.tbss` overlaps
/// what follows `.tdata`); non-allocated ones (debug) follow in emission
/// order, as GNU ld places them.
pub(super) fn finish(shdrs: &mut [Shdr32], verneed_count: u32) {
    shdrs.sort_by_key(|h| {
        let alloc = h.flags & SHF_ALLOC != 0;
        if alloc {
            (false, h.addr, h.size != 0, h.ty == SHT_NOBITS)
        } else {
            (true, 0, false, false)
        }
    });
    let dynsym = synth_index(shdrs, ".dynsym");
    let dynstr = synth_index(shdrs, ".dynstr");
    let gotplt = synth_index(shdrs, ".got.plt");
    for h in shdrs.iter_mut().filter(|h| h.out_idx.is_none()) {
        match h.name.as_str() {
            ".dynsym" => {
                h.link = dynstr;
                h.info = 1; // one local: the null symbol
            }
            ".dynamic" => h.link = dynstr,
            ".hash" | ".gnu.hash" | ".gnu.version" | ".rel.dyn" => h.link = dynsym,
            ".gnu.version_r" => {
                h.link = dynstr;
                h.info = verneed_count;
            }
            ".rel.plt" => {
                h.link = dynsym;
                h.info = gotplt;
            }
            _ => {}
        }
    }
}

/// Header index (1-based: index 0 is the null header) of a synthesised
/// section, or 0 when absent.
pub(super) fn synth_index(shdrs: &[Shdr32], name: &str) -> u32 {
    shdrs
        .iter()
        .position(|h| h.out_idx.is_none() && h.name == name)
        .map_or(0, |i| i as u32 + 1)
}

/// Output-section index -> header index.
pub(super) fn output_index_map(shdrs: &[Shdr32]) -> FxHashMap<usize, u16> {
    shdrs
        .iter()
        .enumerate()
        .filter_map(|(i, h)| h.out_idx.map(|o| (o, i as u16 + 1)))
        .collect()
}

/// Header index for a definition that is not in an input section
/// (emitter-defined anchors, copy-relocated data): `__start_SEC` and
/// `__stop_SEC` belong to SEC; otherwise the allocated section containing the
/// address, end inclusive (`_end` is one past `.bss`), preferring the later
/// one at a boundary; `fallback` when none does.
pub(super) fn containing_index(shdrs: &[Shdr32], name: &str, addr: u32, fallback: u16) -> u16 {
    if let Some(sec) = name
        .strip_prefix("__start_")
        .or_else(|| name.strip_prefix("__stop_"))
        && let Some(i) = shdrs
            .iter()
            .position(|h| h.out_idx.is_some() && h.name == sec)
    {
        return i as u16 + 1;
    }
    shdrs
        .iter()
        .enumerate()
        // Debug sections have no address, and TLS templates overlay ordinary
        // addresses; a non-TLS anchor lives in neither (a TLS symbol always
        // has its own section).
        .filter(|(_, h)| {
            h.flags & SHF_ALLOC != 0
                && h.flags & SHF_TLS == 0
                && h.addr <= addr
                && addr <= h.addr + h.size
        })
        .max_by_key(|(_, h)| h.addr)
        .map_or(fallback, |(i, _)| i as u16 + 1)
}

/// The serialised `.shstrtab` and header table, to be written at
/// `strtab_offset` and `table_offset`.
pub(super) struct HeaderTable {
    pub strtab_offset: u32,
    pub strtab: Vec<u8>,
    pub table_offset: u32,
    pub table: Vec<u8>,
    /// `e_shnum`: the null header, `shdrs`, and `.shstrtab`.
    pub shnum: u16,
    /// `e_shstrndx`: `.shstrtab` is last.
    pub shstrndx: u16,
    /// One past the last byte of the table: the file size.
    pub end: usize,
}

/// Serialise `shdrs` (in final order) plus a null header and `.shstrtab`,
/// placing the string table at `file_end` and the table after it.
pub(super) fn serialize(shdrs: &[Shdr32], file_end: u32) -> Result<HeaderTable, String> {
    let shnum = u16::try_from(shdrs.len() + 2).map_err(|_| {
        format!(
            "{} output sections exceed the ELF32 header limit",
            shdrs.len()
        )
    })?;
    let mut strtab: Vec<u8> = vec![0];
    let mut name_offsets = Vec::with_capacity(shdrs.len());
    for h in shdrs {
        name_offsets.push(strtab.len() as u32);
        strtab.extend_from_slice(h.name.as_bytes());
        strtab.push(0);
    }
    let strtab_name = strtab.len() as u32;
    strtab.extend_from_slice(b".shstrtab\0");
    let table_offset = align_up(file_end + strtab.len() as u32, 4);

    let mut table = vec![0u8; 40]; // null header
    let mut put = |fields: [u32; 10]| {
        for v in fields {
            table.extend_from_slice(&v.to_le_bytes());
        }
    };
    for (h, &name) in shdrs.iter().zip(&name_offsets) {
        put([
            name, h.ty, h.flags, h.addr, h.offset, h.size, h.link, h.info, h.align, h.entsize,
        ]);
    }
    put([
        strtab_name,
        SHT_STRTAB,
        0,
        0,
        file_end,
        strtab.len() as u32,
        0,
        0,
        1,
        0,
    ]);
    Ok(HeaderTable {
        strtab_offset: file_end,
        end: table_offset as usize + table.len(),
        strtab,
        table_offset,
        table,
        shnum,
        shstrndx: shnum - 1,
    })
}

impl HeaderTable {
    /// Write the string table and header table into `output` (already
    /// `self.end` bytes long) and point the ELF header at them.
    pub fn write(&self, output: &mut [u8]) {
        let at = self.strtab_offset as usize;
        output[at..at + self.strtab.len()].copy_from_slice(&self.strtab);
        let at = self.table_offset as usize;
        output[at..at + self.table.len()].copy_from_slice(&self.table);
        output[32..36].copy_from_slice(&self.table_offset.to_le_bytes());
        output[46..48].copy_from_slice(&40u16.to_le_bytes());
        output[48..50].copy_from_slice(&self.shnum.to_le_bytes());
        output[50..52].copy_from_slice(&self.shstrndx.to_le_bytes());
    }
}
