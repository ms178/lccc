//! The section-header table of an ELF64 link output, written once for every
//! backend that lays out its image without one.
//!
//! Consumers today are the AArch64 and RISC-V linkers (through
//! `arm/linker/debug.rs` and `riscv/linker/debug.rs`). x86-64 still hand-writes
//! its table in `x86/linker/emit_exec.rs` and i686 has its own ELF32 twin
//! (`i686/linker/shdr.rs`), whose `__start_`/`__stop_` rule [`SectionLookup`]
//! already reproduces; moving them here is a separate change that must prove
//! byte-identical output for every emitter before it lands.
//!
//! A writer that places its sections by hand knows each section's name,
//! type, flags, address, file offset and size; this module turns that list
//! into the `.symtab`/`.strtab`/`.shstrtab` data and the header table, and
//! appends both after the image. `sh_link` and symbol section indices name
//! a section instead of carrying an index, so no caller has to keep a second
//! count of the header order in step with the first.

use crate::backend::elf::{
    SHF_ALLOC, SHF_EXECINSTR, SHF_INFO_LINK, SHF_TLS, SHF_WRITE, SHN_ABS, SHN_LORESERVE,
    SHT_DYNAMIC, SHT_DYNSYM, SHT_FINI_ARRAY, SHT_GNU_HASH, SHT_GNU_VERNEED, SHT_GNU_VERSYM,
    SHT_INIT_ARRAY, SHT_NOBITS, SHT_PREINIT_ARRAY, SHT_PROGBITS, SHT_RELA, SHT_STRTAB, SHT_SYMTAB,
    STB_LOCAL,
};
use crate::common::fx_hash::FxHashMap;

/// A section header to write. `link` names the section `sh_link` refers to
/// (`.dynstr` for `.dynsym`, `.dynsym` for `.rela.dyn`, ...); `info_link`,
/// when set, names the section `sh_info` refers to and replaces `info` (the
/// section a dynamic relocation table patches, with `SHF_INFO_LINK`).
#[derive(Clone, Debug)]
pub struct ShdrSpec {
    pub name: String,
    pub sh_type: u32,
    pub flags: u64,
    pub addr: u64,
    pub offset: u64,
    pub size: u64,
    pub link: Option<&'static str>,
    pub info_link: Option<&'static str>,
    pub info: u32,
    pub align: u64,
    pub entsize: u64,
}

impl ShdrSpec {
    /// A header with `sh_entsize` defaulted the way bfd's `elf_fake_sections`
    /// does for an ELF64 output: the pointer arrays (`.init_array`,
    /// `.fini_array`, `.preinit_array`) have 8-byte entries, everything else
    /// states its own.
    pub fn new(name: &str, sh_type: u32, flags: u64, addr: u64, offset: u64, size: u64) -> Self {
        let entsize = match sh_type {
            SHT_INIT_ARRAY | SHT_FINI_ARRAY | SHT_PREINIT_ARRAY => 8,
            _ => 0,
        };
        ShdrSpec {
            name: name.to_string(),
            sh_type,
            flags,
            addr,
            offset,
            size,
            link: None,
            info_link: None,
            info: 0,
            align: 1,
            entsize,
        }
    }
}

/// Where a `.symtab` entry is defined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SymSection {
    Abs,
    /// The section with this name; written as `SHN_ABS` when no header of
    /// that name exists (a symbol in a section that was empty and dropped).
    Named(String),
}

#[derive(Clone, Debug)]
pub struct SymtabEntry {
    pub name: String,
    pub info: u8,
    pub other: u8,
    pub section: SymSection,
    pub value: u64,
    pub size: u64,
}

/// Header indices by section name: `sh_link`, `sh_info` and `st_shndx` name a
/// section and the writer resolves the name here, once instead of per entry.
type NameIndex<'a> = FxHashMap<&'a str, u32>;

fn name_index(sections: &[ShdrSpec]) -> NameIndex<'_> {
    sections
        .iter()
        .enumerate()
        .map(|(i, s)| (s.name.as_str(), i as u32 + 1))
        .collect()
}

/// `.symtab` and `.strtab` bytes, plus `sh_info` (the index of the first
/// global). The gABI wants every `STB_LOCAL` entry before the first global.
fn symtab_bytes(
    entries: &[SymtabEntry],
    index: &NameIndex<'_>,
) -> Result<(Vec<u8>, Vec<u8>, u32), String> {
    let mut ordered: Vec<&SymtabEntry> = entries
        .iter()
        .filter(|e| e.info >> 4 == STB_LOCAL)
        .collect();
    let first_global = ordered.len() as u32 + 1;
    ordered.extend(entries.iter().filter(|e| e.info >> 4 != STB_LOCAL));
    let mut strtab: Vec<u8> = vec![0];
    let mut syms: Vec<u8> = vec![0; 24];
    for e in ordered {
        let name_off = if e.name.is_empty() {
            0
        } else {
            let off = u32::try_from(strtab.len())
                .map_err(|_| format!("`.strtab` exceeds 4 GiB at '{}'", e.name))?;
            strtab.extend_from_slice(e.name.as_bytes());
            strtab.push(0);
            off
        };
        let shndx = match &e.section {
            SymSection::Abs => SHN_ABS,
            // A symbol in a section the table dropped is absolute: it has no
            // header to name, and 0 (UNDEF) would say it is imported. Indices
            // from `SHN_LORESERVE` up are reserved, so a real section index
            // must stay below it (the extended form, `SHN_XINDEX`, is not
            // written -- see `append_section_headers`).
            SymSection::Named(n) => match index.get(n.as_str()) {
                Some(&i) if i < u32::from(SHN_LORESERVE) => i as u16,
                Some(_) => {
                    return Err(format!(
                        "section index of '{}' reaches the reserved range at {SHN_LORESERVE}; \
                         `SHN_XINDEX` is not implemented",
                        e.name
                    ));
                }
                None => SHN_ABS,
            },
        };
        syms.extend_from_slice(&name_off.to_le_bytes());
        syms.push(e.info);
        syms.push(e.other);
        syms.extend_from_slice(&shndx.to_le_bytes());
        syms.extend_from_slice(&e.value.to_le_bytes());
        syms.extend_from_slice(&e.size.to_le_bytes());
    }
    Ok((syms, strtab, first_global))
}

/// `.shstrtab` bytes and each header's name offset within it.
fn shstrtab_bytes(sections: &[ShdrSpec]) -> (Vec<u8>, Vec<u32>) {
    let mut shstrtab: Vec<u8> = vec![0];
    let mut name_offsets = Vec::with_capacity(sections.len());
    for s in sections.iter().map(|s| s.name.as_str()) {
        name_offsets.push(shstrtab.len() as u32);
        shstrtab.extend_from_slice(s.as_bytes());
        shstrtab.push(0);
    }
    (shstrtab, name_offsets)
}

/// The 64-byte header table for `sections` (the null header is index 0).
fn table_bytes(
    sections: &[ShdrSpec],
    name_offsets: &[u32],
    index: &NameIndex<'_>,
) -> Result<Vec<u8>, String> {
    let resolve = |owner: &ShdrSpec, target: &'static str| -> Result<u32, String> {
        index.get(target).copied().ok_or_else(|| {
            format!(
                "section header '{}' refers to missing section '{target}'",
                owner.name
            )
        })
    };
    let mut table = vec![0u8; 64];
    for (i, s) in sections.iter().enumerate() {
        let link = match s.link {
            Some(n) => resolve(s, n)?,
            None => 0,
        };
        let info = match s.info_link {
            Some(n) => resolve(s, n)?,
            None => s.info,
        };
        table.extend_from_slice(&name_offsets[i].to_le_bytes());
        table.extend_from_slice(&s.sh_type.to_le_bytes());
        table.extend_from_slice(&s.flags.to_le_bytes());
        table.extend_from_slice(&s.addr.to_le_bytes());
        table.extend_from_slice(&s.offset.to_le_bytes());
        table.extend_from_slice(&s.size.to_le_bytes());
        table.extend_from_slice(&link.to_le_bytes());
        table.extend_from_slice(&info.to_le_bytes());
        table.extend_from_slice(&s.align.max(1).to_le_bytes());
        table.extend_from_slice(&s.entsize.to_le_bytes());
    }
    Ok(table)
}

/// Append `.symtab` (when `symtab` is given), `.strtab`, `.shstrtab` and the
/// header table to `out`, and point the ELF header (`e_shoff`, `e_shnum`,
/// `e_shstrndx`) at them. `sections` excludes the null header and the three
/// tables this function adds; their data must already be in `out`.
///
/// Writes the ordinary ELF header numbering only. A table that reaches
/// `SHN_LORESERVE` entries needs the gABI's extended encoding (`e_shnum` 0,
/// the real count in section 0's `sh_size`, `SHN_XINDEX` for an out-of-range
/// `st_shndx`); that is refused by name rather than written wrong, because
/// readelf, gdb and the loader would each read a different, silently wrong
/// table. No link this tree produces comes close: one header per output
/// section, plus `.symtab`, `.strtab` and `.shstrtab`.
pub fn append_section_headers(
    out: &mut Vec<u8>,
    sections: &[ShdrSpec],
    symtab: Option<&[SymtabEntry]>,
) -> Result<(), String> {
    if out.len() < 64 {
        return Err("ELF image shorter than its header".to_string());
    }
    let mut all: Vec<ShdrSpec> = sections.to_vec();
    if let Some(entries) = symtab {
        // Symbol indices name sections of `all` as it stands now: `.symtab`,
        // `.strtab` and `.shstrtab` are appended after it, so nothing shifts.
        let index = name_index(&all);
        let (syms, strtab, first_global) = symtab_bytes(entries, &index)?;
        let symtab_off = align_to(out, 8);
        out.extend_from_slice(&syms);
        let strtab_off = out.len() as u64;
        out.extend_from_slice(&strtab);
        all.push(ShdrSpec {
            link: Some(".strtab"),
            info: first_global,
            align: 8,
            entsize: 24,
            ..ShdrSpec::new(".symtab", SHT_SYMTAB, 0, 0, symtab_off, syms.len() as u64)
        });
        all.push(ShdrSpec::new(
            ".strtab",
            SHT_STRTAB,
            0,
            0,
            strtab_off,
            strtab.len() as u64,
        ));
    }
    all.push(ShdrSpec::new(".shstrtab", SHT_STRTAB, 0, 0, 0, 0));
    let null_and_sections = all.len();
    let (shstrtab, name_offsets) = shstrtab_bytes(&all);
    let shstrtab_off = out.len() as u64;
    let shstrtab = {
        let last = all.last_mut().expect("`.shstrtab` was just pushed");
        last.offset = shstrtab_off;
        last.size = shstrtab.len() as u64;
        shstrtab
    };
    out.extend_from_slice(&shstrtab);

    // The gABI reserves every `e_shnum` value from `SHN_LORESERVE` up: a table
    // that large must set `e_shnum` to 0 and put the real count in section 0,
    // and `e_shstrndx` becomes `SHN_XINDEX`. Refusing here keeps the ordinary
    // encoding truthful instead of writing a count a reader would misread.
    let shnum = null_and_sections + 1;
    if shnum >= usize::from(SHN_LORESERVE) {
        return Err(format!(
            "{shnum} section headers reach the reserved `e_shnum` range at {SHN_LORESERVE}; \
             extended numbering (`e_shnum` 0, count in section 0) is not implemented"
        ));
    }
    let shnum16 = shnum as u16;
    let index = name_index(&all);
    let table = table_bytes(&all, &name_offsets, &index)?;
    let shoff = align_to(out, 8);
    out.extend_from_slice(&table);
    out[40..48].copy_from_slice(&shoff.to_le_bytes());
    out[58..60].copy_from_slice(&64u16.to_le_bytes());
    out[60..62].copy_from_slice(&shnum16.to_le_bytes());
    out[62..64].copy_from_slice(&((shnum - 1) as u16).to_le_bytes());
    Ok(())
}

/// The header order of a link output: the allocated sections by address,
/// then the others in the order given; empty sections are dropped. At one
/// address a `.tbss` comes first and a `.bss` last: neither occupies file
/// space, and a `.tbss` occupies no address space outside the TLS template,
/// so the section it overlaps really starts at that address (bfd's order).
pub fn order_headers(specs: Vec<ShdrSpec>) -> Vec<ShdrSpec> {
    let (mut alloc, other): (Vec<ShdrSpec>, Vec<ShdrSpec>) = specs
        .into_iter()
        .filter(|s| s.size > 0)
        .partition(|s| s.flags & SHF_ALLOC != 0);
    let rank = |s: &ShdrSpec| match (s.sh_type == SHT_NOBITS, s.flags & SHF_TLS != 0) {
        (true, true) => 0,
        (false, _) => 1,
        (true, false) => 2,
    };
    alloc.sort_by_key(|s| (s.addr, rank(s)));
    alloc.extend(other);
    alloc
}

/// The start of the TLS template: the lowest `SHF_TLS` section address.
pub fn tls_base(headers: &[ShdrSpec]) -> Option<u64> {
    headers
        .iter()
        .filter(|h| h.flags & SHF_TLS != 0 && h.flags & SHF_ALLOC != 0)
        .map(|h| h.addr)
        .min()
}

/// A section of one TLS class, as [`SectionLookup`] indexes it.
#[derive(Clone, Copy)]
struct Candidate {
    addr: u64,
    size: u64,
    index: usize,
}

/// One TLS class of allocated sections (ordinary or TLS), indexed.
///
/// `cands` is in header order; `prefix_end[i]` is the highest `addr + size`
/// over `cands[..=i]`. A section contains `value` only if its end is above
/// `value`, so the first containing section is at or after
/// `prefix_end.partition_point(|e| e <= value)` -- one binary search plus the
/// overlap run, instead of a scan of the whole table per symbol.
struct Class {
    cands: Vec<Candidate>,
    end: Vec<u64>,
    prefix_end: Vec<u64>,
    /// The same sections ordered by `(addr, size)`.
    by_end: Vec<Candidate>,
    image_end: u64,
}

impl Class {
    fn new(cands: Vec<Candidate>) -> Self {
        let end: Vec<u64> = cands
            .iter()
            .map(|c| c.addr.saturating_add(c.size))
            .collect();
        let mut prefix_end = Vec::with_capacity(end.len());
        let mut running = 0u64;
        for &e in &end {
            running = running.max(e);
            prefix_end.push(running);
        }
        let mut by_end = cands.clone();
        by_end.sort_by_key(|c| (c.addr, c.size));
        Class {
            image_end: running,
            cands,
            end,
            prefix_end,
            by_end,
        }
    }

    /// The first section in header order that contains `value`.
    fn containing(&self, value: u64) -> Option<usize> {
        let mut i = self.prefix_end.partition_point(|&e| e <= value);
        while i < self.cands.len() {
            if self.cands[i].addr > value {
                break;
            }
            if value < self.end[i] {
                return Some(self.cands[i].index);
            }
            i += 1;
        }
        None
    }

    /// The section that starts highest at or before `value`, widest first.
    fn preceding(&self, value: u64) -> Option<usize> {
        let i = self.by_end.partition_point(|c| c.addr <= value);
        self.by_end[..i].last().map(|c| c.index)
    }

    fn of(&self, value: u64) -> Option<usize> {
        self.containing(value).or_else(|| {
            if value > self.image_end {
                return None;
            }
            self.preceding(value)
        })
    }
}

/// The allocated sections of a header list, indexed for symbol lookup.
///
/// [`section_of`] answers one query per symbol and a link has tens of
/// thousands of symbols over a few dozen sections, so the index is built once
/// per link and every query is a binary search; the linear scan this replaced
/// also recomputed the image end inside every call. Both orders reproduce
/// exactly what the scan answered, and the equivalence is pinned by
/// `lookup_matches_the_linear_scan` over randomized header lists.
///
/// Addresses use saturating arithmetic: a header list is derived from input
/// sections, and an overflowing `addr + size` must take the last section
/// rather than panic in a debug build on a crafted object.
pub struct SectionLookup<'a> {
    headers: &'a [ShdrSpec],
    /// Indexed by TLS class (0 = ordinary, 1 = TLS).
    classes: [Class; 2],
    /// Section name to header index, for `__start_`/`__stop_` resolution.
    by_name: FxHashMap<&'a str, usize>,
}

impl<'a> SectionLookup<'a> {
    /// Index `headers`, in the final header order ([`order_headers`]).
    pub fn new(headers: &'a [ShdrSpec]) -> Self {
        let mut split: [Vec<Candidate>; 2] = [Vec::new(), Vec::new()];
        for (index, h) in headers.iter().enumerate() {
            if h.flags & SHF_ALLOC == 0 {
                continue;
            }
            let class = usize::from(h.flags & SHF_TLS != 0);
            split[class].push(Candidate {
                addr: h.addr,
                size: h.size,
                index,
            });
        }
        let [ordinary, tls] = split;
        SectionLookup {
            headers,
            classes: [Class::new(ordinary), Class::new(tls)],
            by_name: headers
                .iter()
                .enumerate()
                .map(|(i, h)| (h.name.as_str(), i))
                .collect(),
        }
    }

    /// The header index of the section named `name`, if the table has one.
    pub fn named(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied()
    }

    /// The header a symbol named `name` at `value` belongs to: GNU ld's
    /// `__start_<section>`/`__stop_<section>` name their section first (as
    /// `i686/linker/shdr.rs` resolves it), then the address decides.
    ///
    /// The name rule exists because those symbols delimit a section that may
    /// be empty or dropped, which no address can identify.
    pub fn header_of(&self, name: &str, value: u64, tls: bool) -> Option<usize> {
        if let Some(section) = name
            .strip_prefix("__start_")
            .or_else(|| name.strip_prefix("__stop_"))
        {
            if let Some(i) = self.named(section) {
                return Some(i);
            }
        }
        self.of(value, tls)
    }

    /// The header index of the section holding `value`, else -- inside the
    /// image -- the nearest one before it (the one it ends, like `_end`, or
    /// the one before an empty section that was dropped); `None` when no
    /// allocated section of this class can own the address.
    ///
    /// Inside the image it never returns `None`: the dynamic loader and gdb
    /// do not relocate an absolute symbol by the load base, so a PIE or shared
    /// object would point elsewhere. A TLS symbol is looked up among the TLS
    /// sections only and every other symbol outside them, because a `.tbss`
    /// overlaps the addresses of the section after it.
    pub fn of(&self, value: u64, tls: bool) -> Option<usize> {
        self.classes[usize::from(tls)].of(value)
    }

    /// The section a symbol belongs to, by name, as [`SymSection`].
    pub fn section_named(&self, name: &str, value: u64, tls: bool) -> SymSection {
        self.header_of(name, value, tls)
            .map_or(SymSection::Abs, |i| {
                SymSection::Named(self.headers[i].name.clone())
            })
    }

    /// The header list this lookup was built from.
    pub fn headers(&self) -> &'a [ShdrSpec] {
        self.headers
    }
}

/// The section a symbol at address `value` belongs to, by name, for a single
/// query. Callers resolving many symbols build a [`SectionLookup`] once.
pub fn section_of(headers: &[ShdrSpec], value: u64, tls: bool) -> SymSection {
    SectionLookup::new(headers)
        .of(value, tls)
        .map_or(SymSection::Abs, |i| {
            SymSection::Named(headers[i].name.clone())
        })
}

/// The index `append_section_headers` gives the header named `name` in
/// `headers` (the null header is 0).
pub fn header_index(headers: &[ShdrSpec], name: &str) -> Option<u16> {
    headers
        .iter()
        .position(|h| h.name == name)
        .and_then(|i| u16::try_from(i + 1).ok())
}

/// A `.symtab` value: the address, or for a TLS symbol its offset in the
/// TLS template (gABI: `st_value` of an `STT_TLS` symbol in an executable or
/// shared object is that offset; readelf, gdb and bfd all read it so).
pub fn symtab_value(value: u64, tls: bool, tls_base: Option<u64>) -> u64 {
    match (tls, tls_base) {
        (true, Some(base)) => value.wrapping_sub(base),
        _ => value,
    }
}

/// A synthesized allocated section: `.got`, `.got.plt`, `.plt`, `.rela.plt`,
/// `.eh_frame_hdr`. A `.rela.plt` patches `.got.plt`, and says so with
/// `SHF_INFO_LINK` (bfd's static IRELATIVE table too).
pub fn synthetic(
    name: &str,
    flags: u64,
    addr: u64,
    offset: u64,
    size: u64,
    align: u64,
) -> ShdrSpec {
    let (sh_type, entsize) = if name.starts_with(".rela") {
        (SHT_RELA, 24)
    } else if name.starts_with(".got") {
        (SHT_PROGBITS, 8)
    } else {
        (SHT_PROGBITS, 0)
    };
    let plt_relocs = name == ".rela.plt";
    ShdrSpec {
        align,
        entsize,
        info_link: plt_relocs.then_some(".got.plt"),
        ..ShdrSpec::new(
            name,
            sh_type,
            flags | SHF_ALLOC | if plt_relocs { SHF_INFO_LINK } else { 0 },
            addr,
            offset,
            size,
        )
    }
}

/// A synthesized section's address, file offset and size.
pub type Span = (u64, u64, u64);

/// The dynamic-linking sections a dynamic executable or shared object
/// synthesizes. Empty spans are dropped by [`order_headers`].
#[derive(Default)]
pub struct DynamicSections {
    pub interp: Span,
    pub gnu_hash: Span,
    pub dynsym: Span,
    pub dynstr: Span,
    pub gnu_version: Span,
    pub gnu_version_r: Span,
    /// `sh_info` of `.gnu.version_r`: its number of `Elf64_Verneed` entries.
    pub verneed_count: u32,
    pub rela_dyn: Span,
    pub rela_plt: Span,
    pub plt: Span,
    pub plt_entsize: u64,
    pub dynamic: Span,
    pub got: Span,
    pub got_plt: Span,
    pub eh_frame_hdr: Span,
}

/// Headers for `d`, linked the way the gABI and bfd do it: `.dynsym` to
/// `.dynstr` (`sh_info` 1: only the null symbol is local), the hash table,
/// the version table and the relocation tables to `.dynsym`, the version
/// needs and `.dynamic` to `.dynstr`, and `.rela.plt` patching `.got.plt`
/// (`SHF_INFO_LINK`; measured on bfd 2.44 AArch64 and RISC-V outputs).
pub fn dynamic_headers(d: &DynamicSections) -> Vec<ShdrSpec> {
    let mk =
        |name: &str, sh_type, flags, (addr, offset, size): Span, link, align, entsize| ShdrSpec {
            link,
            align,
            entsize,
            ..ShdrSpec::new(name, sh_type, flags | SHF_ALLOC, addr, offset, size)
        };
    let dynsym = Some(".dynsym");
    let dynstr = Some(".dynstr");
    vec![
        mk(".interp", SHT_PROGBITS, 0, d.interp, None, 1, 0),
        mk(".gnu.hash", SHT_GNU_HASH, 0, d.gnu_hash, dynsym, 8, 0),
        ShdrSpec {
            info: 1,
            ..mk(".dynsym", SHT_DYNSYM, 0, d.dynsym, dynstr, 8, 24)
        },
        mk(".dynstr", SHT_STRTAB, 0, d.dynstr, None, 1, 0),
        mk(
            ".gnu.version",
            SHT_GNU_VERSYM,
            0,
            d.gnu_version,
            dynsym,
            2,
            2,
        ),
        ShdrSpec {
            info: d.verneed_count,
            ..mk(
                ".gnu.version_r",
                SHT_GNU_VERNEED,
                0,
                d.gnu_version_r,
                dynstr,
                8,
                0,
            )
        },
        mk(".rela.dyn", SHT_RELA, 0, d.rela_dyn, dynsym, 8, 24),
        ShdrSpec {
            info_link: (d.got_plt.2 > 0).then_some(".got.plt"),
            ..mk(
                ".rela.plt",
                SHT_RELA,
                SHF_INFO_LINK,
                d.rela_plt,
                dynsym,
                8,
                24,
            )
        },
        mk(
            ".plt",
            SHT_PROGBITS,
            SHF_EXECINSTR,
            d.plt,
            None,
            16,
            d.plt_entsize,
        ),
        mk(".dynamic", SHT_DYNAMIC, SHF_WRITE, d.dynamic, dynstr, 8, 16),
        mk(".got", SHT_PROGBITS, SHF_WRITE, d.got, None, 8, 8),
        mk(".got.plt", SHT_PROGBITS, SHF_WRITE, d.got_plt, None, 8, 8),
        mk(".eh_frame_hdr", SHT_PROGBITS, 0, d.eh_frame_hdr, None, 4, 0),
    ]
}

/// Pad `out` to a multiple of `align`; returns the new length.
fn align_to(out: &mut Vec<u8>, align: usize) -> u64 {
    let len = out.len().next_multiple_of(align);
    out.resize(len, 0);
    len as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shdr(out: &[u8], i: usize) -> &[u8] {
        let shoff = u64::from_le_bytes(out[40..48].try_into().unwrap()) as usize;
        &out[shoff + 64 * i..shoff + 64 * (i + 1)]
    }
    fn u32_at(b: &[u8], o: usize) -> u32 {
        u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
    }
    fn u64_at(b: &[u8], o: usize) -> u64 {
        u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
    }

    #[test]
    fn table_and_symtab_are_consistent() {
        let mut out = vec![0u8; 100];
        let text = ShdrSpec {
            align: 4,
            ..ShdrSpec::new(".text", 1, 6, 0x400040, 0x40, 0x20)
        };
        let dbg = ShdrSpec::new(".debug_info", 1, 0, 0, 0x60, 4);
        let syms = [
            SymtabEntry {
                name: "main".into(),
                info: 0x12,
                other: 0,
                section: SymSection::Named(".text".into()),
                value: 0x400040,
                size: 8,
            },
            SymtabEntry {
                name: "helper".into(),
                info: 0x02,
                other: 0,
                section: SymSection::Named(".text".into()),
                value: 0x400048,
                size: 8,
            },
            SymtabEntry {
                name: "gone".into(),
                info: 0x11,
                other: 0,
                section: SymSection::Named(".bss".into()),
                value: 0,
                size: 0,
            },
        ];
        append_section_headers(&mut out, &[text, dbg], Some(&syms)).unwrap();
        let shnum = u16::from_le_bytes(out[60..62].try_into().unwrap()) as usize;
        let shstrndx = u16::from_le_bytes(out[62..64].try_into().unwrap()) as usize;
        // null, .text, .debug_info, .symtab, .strtab, .shstrtab
        assert_eq!(shnum, 6);
        assert_eq!(shstrndx, 5);
        let symtab = shdr(&out, 3);
        assert_eq!(u32_at(symtab, 4), SHT_SYMTAB);
        assert_eq!(u32_at(symtab, 40), 4, "sh_link names .strtab");
        assert_eq!(u32_at(symtab, 44), 2, "sh_info: null + one local");
        let sym_off = u64_at(symtab, 24) as usize;
        assert_eq!(u64_at(symtab, 32), 4 * 24);
        // Entry 1 is the local `helper`, in section 1 (.text).
        let local = &out[sym_off + 24..sym_off + 48];
        assert_eq!(local[4], 0x02);
        assert_eq!(u16::from_le_bytes([local[6], local[7]]), 1);
        // `gone` names a section without a header: SHN_ABS.
        let gone = &out[sym_off + 72..sym_off + 96];
        assert_eq!(u16::from_le_bytes([gone[6], gone[7]]), SHN_ABS);
        // Names resolve through .shstrtab.
        let shstr = shdr(&out, 5);
        let base = u64_at(shstr, 24) as usize;
        let name = |i: usize| {
            let o = base + u32_at(shdr(&out, i), 0) as usize;
            let end = out[o..].iter().position(|&b| b == 0).unwrap();
            String::from_utf8(out[o..o + end].to_vec()).unwrap()
        };
        assert_eq!(name(1), ".text");
        assert_eq!(name(2), ".debug_info");
        assert_eq!(name(4), ".strtab");
        assert_eq!(name(5), ".shstrtab");
    }

    #[test]
    fn symbol_sections_and_header_indices() {
        let alloc = |name: &str, flags: u64, addr, size| {
            ShdrSpec::new(name, 1, SHF_ALLOC | flags, addr, addr, size)
        };
        let h = vec![
            alloc(".text", 4, 0x1000, 0x100),
            // `.tbss` overlaps the addresses of the section after it.
            alloc(".tbss", SHF_WRITE | SHF_TLS, 0x2000, 0x10),
            alloc(".data", SHF_WRITE, 0x2000, 0x20),
            alloc(".bss", SHF_WRITE, 0x2040, 0x10),
            ShdrSpec::new(".comment", 1, 0, 0, 0x3000, 8),
        ];
        let named = |n: &str| SymSection::Named(n.to_string());
        assert_eq!(section_of(&h, 0x1010, false), named(".text"));
        assert_eq!(section_of(&h, 0x2004, true), named(".tbss"));
        assert_eq!(section_of(&h, 0x2004, false), named(".data"));
        // The end of the image (`_end`) and a gap before an aligned section
        // (where a dropped empty section sat) stay section-relative: an
        // absolute symbol would not move with a PIE or shared object.
        assert_eq!(section_of(&h, 0x2050, false), named(".bss"));
        assert_eq!(section_of(&h, 0x2030, false), named(".data"));
        // Outside the image a value is absolute.
        assert_eq!(section_of(&h, 0x9000, false), SymSection::Abs);
        assert_eq!(section_of(&h, 0x10, false), SymSection::Abs);
        assert_eq!(header_index(&h, ".text"), Some(1));
        assert_eq!(header_index(&h, ".bss"), Some(4));
        assert_eq!(header_index(&h, ".missing"), None);
    }

    #[test]
    fn missing_link_target_is_an_error() {
        let mut out = vec![0u8; 64];
        let rela = ShdrSpec {
            link: Some(".dynsym"),
            ..ShdrSpec::new(".rela.dyn", 4, 2, 0, 0, 0)
        };
        assert!(append_section_headers(&mut out, &[rela], None).is_err());
    }

    /// The algorithm `SectionLookup` replaced, kept as the oracle: first
    /// containing section in header order, else the nearest preceding one
    /// inside the image. Saturating so a crafted header cannot panic it.
    fn reference_section_of(headers: &[ShdrSpec], value: u64, tls: bool) -> Option<usize> {
        let class = |h: &ShdrSpec| h.flags & SHF_ALLOC != 0 && (h.flags & SHF_TLS != 0) == tls;
        let image_end = headers
            .iter()
            .filter(|h| class(h))
            .map(|h| h.addr.saturating_add(h.size))
            .max()
            .unwrap_or(0);
        headers
            .iter()
            .enumerate()
            .filter(|(_, h)| class(h))
            .find(|(_, h)| h.addr <= value && value < h.addr.saturating_add(h.size))
            .map(|(i, _)| i)
            .or_else(|| {
                headers
                    .iter()
                    .enumerate()
                    .filter(|(_, h)| class(h))
                    .filter(|(_, h)| h.addr <= value && value <= image_end)
                    .max_by_key(|(_, h)| (h.addr, h.size))
                    .map(|(i, _)| i)
            })
    }

    /// xorshift64*, so the sweep is random but reproducible on every host.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }
        fn below(&mut self, bound: u64) -> u64 {
            if bound == 0 { 0 } else { self.next() % bound }
        }
    }

    #[test]
    fn lookup_matches_the_linear_scan() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for round in 0..400 {
            let count = rng.below(12) as usize;
            // Duplicate addresses, zero sizes, `.tbss`-over-`.data` ties and
            // non-allocated headers all appear, because a header list that
            // only holds disjoint sections would not exercise tie-breaking.
            let specs: Vec<ShdrSpec> = (0..count)
                .map(|_| {
                    let addr = rng.below(4) * 0x1000;
                    let size = rng.below(4) * 0x400;
                    let mut flags = if rng.below(8) == 0 { 0 } else { SHF_ALLOC };
                    if rng.below(3) == 0 {
                        flags |= SHF_TLS;
                    }
                    ShdrSpec::new(&format!(".s{addr:x}_{size:x}"), 1, flags, addr, addr, size)
                })
                .collect();
            let specs = order_headers(specs);
            let lookup = SectionLookup::new(&specs);
            for probe in 0..40 {
                let value = rng.below(6) * 0x400;
                for tls in [false, true] {
                    assert_eq!(
                        lookup.of(value, tls),
                        reference_section_of(&specs, value, tls),
                        "round {round} probe {probe} value {value:#x} tls {tls} headers {:?}",
                        specs
                            .iter()
                            .map(|s| (s.name.as_str(), s.addr, s.size, s.flags))
                            .collect::<Vec<_>>()
                    );
                }
            }
            // Boundaries: every section start and end, +-1.
            for h in &specs {
                for value in [
                    h.addr.saturating_sub(1),
                    h.addr,
                    h.addr.saturating_add(h.size).saturating_sub(1),
                    h.addr.saturating_add(h.size),
                ] {
                    for tls in [false, true] {
                        assert_eq!(
                            lookup.of(value, tls),
                            reference_section_of(&specs, value, tls),
                            "boundary {value:#x} tls {tls}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn start_and_stop_symbols_name_their_section() {
        let h = vec![
            ShdrSpec::new(".text", 1, SHF_ALLOC, 0x1000, 0x1000, 0x100),
            ShdrSpec::new(".mytable", 1, SHF_ALLOC, 0x2000, 0x2000, 0x10),
        ];
        let lookup = SectionLookup::new(&h);
        // The section a `__start_`/`__stop_` symbol delimits is named, which
        // no address can do: `.mytable` may be empty and dropped from the
        // table while the symbol still has to be section-relative.
        assert_eq!(lookup.header_of("__start_mytable", 0x2000, false), Some(1));
        assert_eq!(lookup.header_of("__stop_mytable", 0x2010, false), Some(1));
        // A delimiter whose suffix is not a section of this table falls back
        // to the address, exactly as an ordinary symbol would.
        assert_eq!(lookup.header_of("__start_absent", 0x1010, false), Some(0));
        assert_eq!(
            lookup.section_named("__stop_mytable", 0x2010, false),
            SymSection::Named(".mytable".into())
        );
    }

    #[test]
    fn large_tables_resolve_correctly_and_quickly() {
        // 4096 sections: the shape that made the per-symbol image-end
        // recomputation visible. Correctness is asserted against the oracle,
        // and the whole sweep must stay far below a second -- a linear scan
        // would be 4096 * 100_000 comparisons here.
        let sections: Vec<ShdrSpec> = (0..4096u64)
            .map(|i| {
                let at = 0x1000 + i * 0x10;
                ShdrSpec::new(&format!(".s{i}"), 1, SHF_ALLOC, at, at, 0x10)
            })
            .collect();
        let lookup = SectionLookup::new(&sections);
        let start = std::time::Instant::now();
        let mut hits = 0u64;
        for i in 0..100_000u64 {
            let value = 0x1000 + (i * 7919) % (4096 * 0x10);
            if lookup.of(value, false).is_some() {
                hits += 1;
            }
        }
        assert_eq!(hits, 100_000, "every in-image address must resolve");
        let elapsed = start.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "100k lookups over 4096 sections took {elapsed:?}"
        );
        // The index and the oracle agree on the same large table, too.
        for (i, spec) in sections.iter().enumerate().step_by(97) {
            for value in [spec.addr, spec.addr + 4, spec.addr + spec.size] {
                assert_eq!(
                    lookup.of(value, false),
                    reference_section_of(&sections, value, false),
                    "section {i} at {value:#x}"
                );
            }
        }
    }

    #[test]
    fn header_writer_survives_random_tables() {
        // The writer must never panic and never write a table that disagrees
        // with its own ELF header, whatever the caller hands it: repeated and
        // empty names, zero alignments, huge sizes and section counts.
        let mut rng = Rng(0x0123_4567_89ab_cdef);
        for round in 0..300 {
            let count = rng.below(24) as usize;
            let specs: Vec<ShdrSpec> = (0..count)
                .map(|_| {
                    let name = match rng.below(6) {
                        0 => String::new(),
                        1 => ".dup".to_string(),
                        _ => format!(".s{}", rng.below(8)),
                    };
                    ShdrSpec {
                        align: rng.below(64),
                        entsize: rng.below(32),
                        link: None,
                        ..ShdrSpec::new(
                            &name,
                            1,
                            rng.below(4) * SHF_ALLOC,
                            rng.below(0x10_0000),
                            rng.below(0x10_0000),
                            rng.below(0x10_0000),
                        )
                    }
                })
                .collect();
            let some = specs.first().map_or(".text", |s| s.name.as_str());
            let syms: Vec<SymtabEntry> = (0..rng.below(12) as usize)
                .map(|i| SymtabEntry {
                    name: format!("s{i}"),
                    info: if i % 3 == 0 { 0x12 } else { 0x02 },
                    other: 0,
                    section: if i % 5 == 0 {
                        SymSection::Abs
                    } else {
                        SymSection::Named(some.into())
                    },
                    value: rng.below(0x10_0000),
                    size: rng.below(64),
                })
                .collect();
            let mut out = vec![0u8; 64];
            let Ok(()) = append_section_headers(&mut out, &specs, Some(&syms)) else {
                continue;
            };
            // e_shoff/e_shnum agree with a table that ends inside the buffer.
            let shoff = u64::from_le_bytes(out[40..48].try_into().unwrap()) as usize;
            let shnum = u16::from_le_bytes(out[60..62].try_into().unwrap()) as usize;
            let shstrndx = u16::from_le_bytes(out[62..64].try_into().unwrap()) as usize;
            assert!(shnum >= 1, "round {round}");
            assert_eq!(shstrndx, shnum - 1, "round {round}: .shstrtab is last");
            assert!(
                shoff >= 64 && shoff % 8 == 0,
                "round {round}: shoff {shoff}"
            );
            assert_eq!(out.len(), shoff + 64 * shnum, "round {round}: table size");
            // Every header's name offset lands on a NUL-terminated string in
            // `.shstrtab`; a wrong offset would make readelf print garbage.
            let at = shoff + 64 * shstrndx + 24;
            let str_off = u64::from_le_bytes(out[at..at + 8].try_into().unwrap()) as usize;
            for i in 0..shnum {
                let at = shoff + 64 * i;
                let name = u32::from_le_bytes(out[at..at + 4].try_into().unwrap()) as usize;
                let start = str_off + name;
                assert!(start < out.len(), "round {round} header {i}");
                assert!(
                    out[start..].contains(&0),
                    "round {round} header {i} name is unterminated"
                );
            }
        }
    }

    #[test]
    fn extended_numbering_is_refused_by_name() {
        // A table that reaches `SHN_LORESERVE` entries cannot be expressed
        // with the ordinary `e_shnum` field, and the gABI's extended form is
        // deliberately unimplemented. The error must say so instead of
        // writing a truncated count.
        let specs: Vec<ShdrSpec> = (0..0xff00)
            .map(|i| ShdrSpec::new(&format!(".s{i}"), 1, 0, 0, 0, 0))
            .collect();
        let mut out = vec![0u8; 64];
        let err = append_section_headers(&mut out, &specs, None).unwrap_err();
        assert!(err.contains("extended numbering"), "{err}");
    }
}
