//! GNU ld-exact `.eh_frame` editing for the ELF64 non-x86 linkers.
//!
//! binutils 2.47 edits every `.eh_frame` input before layout
//! (`bfd/elf-eh-frame.c` plus the padding loop of
//! `elflink.c:bfd_elf_discard_info`), so concatenating the inputs -- what a
//! linker does with any other section -- produces different bytes, a
//! different `.eh_frame` size and therefore a different address for
//! everything after it.  This module reproduces that editing on the loaded
//! objects, before relaxation, so every later phase sees exactly the input
//! bfd lays out:
//!
//! * a section bfd cannot parse (64-bit records, an unknown CIE version or
//!   augmentation, a relocation where none may be, an unknown CFA opcode,
//!   `DW_CFA_set_loc` in a CIE, an FDE whose CIE is not an earlier CIE of
//!   the same section, an FDE start symbol defined in another object) is
//!   kept verbatim and suppresses the `.eh_frame_hdr` search table;
//! * trailing `DW_CFA_nop` padding is trimmed from every CIE and FDE;
//! * an FDE whose address range is zero and carries no relocation is dropped;
//! * a CIE survives only when a kept FDE uses it, and is then merged into the
//!   first identical CIE of the link (`cie_eq`: never `"eh"` CIEs, never
//!   CIEs with more than 50 bytes of initial instructions, personality
//!   routines compared by global symbol or by (object, local symbol));
//! * only the zero terminator of the last `.eh_frame` input survives;
//! * records are re-aligned -- CIE 4 (8 for an aligned 8-byte personality),
//!   FDE max(4, pointer width) -- with the padding inside the previous
//!   record's length, and each section ends 4-aligned;
//! * every input before the trailing run of empty and terminator-only inputs
//!   is padded to the output section alignment, again inside its last
//!   record; empty inputs of that trailing run are excluded from layout.
//!
//! Relocation offsets and the values of symbols defined in an edited section
//! follow their record (`offset_adjust`), pc-relative LSDA and
//! `DW_CFA_set_loc` constants that carry no relocation are re-based, and the
//! CIE pointer of an FDE whose CIE was merged into another input is written
//! as the constant distance between the two planned positions.
//! [`EhFrameLayout::verify`] proves after section merging that the output
//! placed every input where those constants assumed.
//!
//! One input class is passed through unedited rather than edited bfd's way:
//! a position-independent link whose CIE uses an absolute pointer encoding,
//! which bfd rewrites into pc-relative form by inserting an `R` augmentation
//! (`make_relative`).  No RISC-V compiler emits absolute `.eh_frame`
//! encodings; the pass-through output is a valid, fully unwindable
//! `.eh_frame` whose search table stays correct.

use super::{Elf64Object, Elf64Rela, SectionData};
use crate::backend::elf::{
    SHN_ABS, SHN_COMMON, SHN_LORESERVE, SHN_UNDEF, STB_LOCAL, STB_WEAK, STT_OBJECT,
};
use crate::common::fx_hash::FxHashMap;

const DW_EH_PE_OMIT: u8 = 0xff;
const DW_EH_PE_ABSPTR: u8 = 0x00;
const DW_EH_PE_PCREL: u8 = 0x10;
const DW_EH_PE_ALIGNED: u8 = 0x50;
const DW_EH_PE_APPL_MASK: u8 = 0x70;
/// Capacity of bfd's `struct cie::initial_instructions`: longer initial
/// instruction sequences never compare equal, so such CIEs never merge.
const CIE_INSNS_MAX: usize = 50;
/// Capacity of bfd's `struct cie::augmentation`, NUL included.
const AUG_MAX: usize = 20;

/// The target facts bfd's `.eh_frame` parser depends on.
#[derive(Clone, Copy, Debug)]
pub struct EhTarget {
    /// `elf_backend_eh_frame_address_size`: 8 for ELF64.
    pub ptr_size: u8,
    /// `bfd_link_pic`: shared objects and PIEs.
    pub pic: bool,
    /// The AArch64-only `B`/`G` augmentations are recognised.
    pub aarch64: bool,
}

/// Where [`edit`] planned each non-empty `.eh_frame` input inside the output
/// section, and whether bfd would build the `.eh_frame_hdr` search table.
#[derive(Debug)]
pub struct EhFrameLayout {
    bases: Vec<((usize, usize), u64)>,
    hdr_table: bool,
    /// `_bfd_elf_eh_frame_present`: some input held a CIE or FDE.
    present: bool,
}

impl EhFrameLayout {
    /// False when bfd omits the binary-search table from `.eh_frame_hdr`
    /// (an input it could not parse, or an aligned FDE encoding in a
    /// position-independent link): the header is then 8 bytes with
    /// `fde_count_enc` and `table_enc` set to `DW_EH_PE_omit`.
    pub fn hdr_table(&self) -> bool {
        self.hdr_table
    }

    /// The `.eh_frame_hdr` size bfd allocates for `fde_count` FDEs
    /// (`_bfd_elf_discard_section_eh_frame_hdr`); 0 when it creates none.
    pub fn hdr_size(&self, fde_count: usize) -> usize {
        match (self.present, self.hdr_table) {
            (false, _) => 0,
            (true, true) => 12 + 8 * fde_count,
            (true, false) => 8,
        }
    }

    /// Fill the `.eh_frame_hdr` slot sized by [`Self::hdr_size`].
    pub fn write_hdr(
        &self,
        slot: &mut [u8],
        eh_frame: &[u8],
        eh_frame_vaddr: u64,
        hdr_vaddr: u64,
    ) -> Result<(), String> {
        if self.hdr_table {
            super::write_eh_frame_hdr(slot, eh_frame, eh_frame_vaddr, hdr_vaddr, true);
            return Ok(());
        }
        let rel = i64::try_from(eh_frame_vaddr)
            .ok()
            .zip(i64::try_from(hdr_vaddr).ok())
            .and_then(|(e, h)| i32::try_from(e - h - 4).ok())
            .ok_or(".eh_frame out of .eh_frame_hdr's sdata4 reach")?;
        let hdr = slot
            .get_mut(..8)
            .ok_or("internal: short .eh_frame_hdr slot")?;
        hdr[..4].copy_from_slice(&[1, 0x1b, DW_EH_PE_OMIT, DW_EH_PE_OMIT]);
        hdr[4..].copy_from_slice(&rel.to_le_bytes());
        Ok(())
    }

    /// Check that section merging placed every edited input at the offset
    /// the cross-input CIE pointers and moved symbols were computed for.
    pub fn verify(
        &self,
        sec_mapping: &FxHashMap<(usize, usize), (usize, u64)>,
    ) -> Result<(), String> {
        let mut merged = None;
        for &(key, base) in &self.bases {
            let Some(&(m, off)) = sec_mapping.get(&key) else {
                return Err(format!(
                    ".eh_frame input (object {}, section {}) has no output placement",
                    key.0, key.1
                ));
            };
            if *merged.get_or_insert(m) != m || off != base {
                return Err(format!(
                    ".eh_frame input (object {}, section {}) placed at {off:#x}, \
                     edited for {base:#x}",
                    key.0, key.1
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum Personality {
    None,
    Global(String),
    Local(usize, u32),
}

/// Everything `cie_eq` compares (the output section is always `.eh_frame`).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct CieKey {
    length: u32,
    version: u8,
    augmentation: Vec<u8>,
    code_align: u64,
    data_align: u64,
    ra_column: u64,
    augmentation_size: u64,
    personality: Personality,
    per_encoding: u8,
    lsda_encoding: u8,
    fde_encoding: u8,
    initial_insns: Vec<u8>,
}

impl CieKey {
    fn mergeable(&self) -> bool {
        self.augmentation != b"eh" && self.initial_insns.len() <= CIE_INSNS_MAX
    }
}

#[derive(Debug)]
struct Cie {
    key: CieKey,
    /// Index (in offset order) of the personality relocation.
    per_reloc: Option<usize>,
    aligned8: bool,
    merged_with: Option<(usize, usize)>,
}

#[derive(Debug)]
struct Fde {
    /// The CIE entry (same section); `None` drops a zero-range FDE.
    cie: Option<usize>,
    /// The CIE actually referenced after merging: (input, entry).
    target: (usize, usize),
    fde_encoding: u8,
    lsda_encoding: u8,
    /// LSDA pointer offset from the initial-location field.
    lsda_offset: usize,
    /// `DW_CFA_set_loc` operand offsets from the initial-location field,
    /// recorded (as bfd does) only for pc-relative FDE encodings.
    set_locs: Vec<usize>,
}

#[derive(Debug)]
enum Kind {
    Terminator,
    Cie(Box<Cie>),
    Fde(Fde),
}

#[derive(Debug)]
struct Entry {
    offset: usize,
    /// Size before trailing-nop trimming (relocation ownership).
    raw_size: usize,
    size: usize,
    removed: bool,
    new_offset: usize,
    kind: Kind,
}

enum Plan {
    /// Kept as is; `opaque` is bfd's `free_no_table` (no search table).
    Raw { opaque: bool },
    Edited {
        entries: Vec<Entry>,
        relocs: Vec<Elf64Rela>,
    },
}

struct Input {
    obj: usize,
    sec: usize,
    plan: Plan,
    size: usize,
    addralign: u64,
}

fn wide(v: usize) -> u64 {
    u64::try_from(v).unwrap_or(u64::MAX)
}

fn align_up(v: usize, a: usize) -> usize {
    v.div_ceil(a.max(1)) * a.max(1)
}

/// `get_DW_EH_PE_width`.
fn pe_width(encoding: u8, ptr_size: u8) -> usize {
    if encoding & 0x60 == 0x60 {
        return 0;
    }
    match encoding & 7 {
        2 => 2,
        3 => 4,
        4 => 8,
        0 => usize::from(ptr_size),
        _ => 0,
    }
}

fn read_uint(d: &[u8], pos: usize, width: usize) -> u64 {
    d[pos..pos + width]
        .iter()
        .rev()
        .fold(0u64, |v, &b| (v << 8) | u64::from(b))
}

fn write_uint(d: &mut [u8], pos: usize, width: usize, v: u64) {
    d[pos..pos + width].copy_from_slice(&v.to_le_bytes()[..width]);
}

/// Bounds-checked reader over one record (`end` is the record end).
struct Rd<'a> {
    d: &'a [u8],
    p: usize,
    end: usize,
}

impl Rd<'_> {
    fn skip(&mut self, n: u64) -> Option<()> {
        let n = usize::try_from(n).ok()?;
        if n > self.end - self.p {
            return None;
        }
        self.p += n;
        Some(())
    }

    fn byte(&mut self) -> Option<u8> {
        let b = *self.d.get(self.p).filter(|_| self.p < self.end)?;
        self.p += 1;
        Some(b)
    }

    fn skip_leb(&mut self) -> Option<()> {
        while self.byte()? & 0x80 != 0 {}
        Some(())
    }

    /// `read_uleb128`/`read_sleb128` (bfd's wrapping semantics).
    fn leb(&mut self, signed: bool) -> Option<u64> {
        let start = self.p;
        self.skip_leb()?;
        let mut p = self.p - 1;
        let last = u64::from(self.d[p]);
        let mut v = if signed {
            ((last & 0x7f) ^ 0x40).wrapping_sub(0x40)
        } else {
            last
        };
        while p > start {
            p -= 1;
            v = v.wrapping_shl(7) | u64::from(self.d[p] & 0x7f);
        }
        Some(v)
    }

    /// `skip_cfa_op`.
    fn skip_cfa_op(&mut self, ptr_width: usize) -> Option<()> {
        let op = self.byte()?;
        let op = if op & 0xc0 != 0 { op & 0xc0 } else { op };
        match op {
            // nop, advance_loc, restore, remember/restore_state,
            // GNU_window_save, AARCH64_negate_ra_state_with_pc.
            0x00 | 0x40 | 0xc0 | 0x0a | 0x0b | 0x2d | 0x2c => Some(()),
            // offset, restore_extended, undefined, same_value,
            // def_cfa_register, def_cfa_offset, def_cfa_offset_sf,
            // GNU_args_size.
            0x80 | 0x06 | 0x07 | 0x08 | 0x0d | 0x0e | 0x13 | 0x2e => self.skip_leb(),
            // val_offset, val_offset_sf, offset_extended, register, def_cfa,
            // offset_extended_sf, GNU_negative_offset_extended, def_cfa_sf.
            0x14 | 0x15 | 0x05 | 0x09 | 0x0c | 0x11 | 0x2f | 0x12 => {
                self.skip_leb()?;
                self.skip_leb()
            }
            // def_cfa_expression.
            0x0f => {
                let n = self.leb(false)?;
                self.skip(n)
            }
            // expression, val_expression.
            0x10 | 0x16 => {
                self.skip_leb()?;
                let n = self.leb(false)?;
                self.skip(n)
            }
            0x01 => self.skip(wide(ptr_width)),
            0x02 => self.skip(1),
            0x03 => self.skip(2),
            0x04 => self.skip(4),
            0x1d => self.skip(8),
            _ => None,
        }
    }
}

/// `skip_non_nops`: the first byte of trailing `DW_CFA_nop` padding.
fn skip_non_nops(
    d: &[u8],
    start: usize,
    end: usize,
    ptr_width: usize,
    set_locs: &mut Vec<usize>,
) -> Option<usize> {
    let mut rd = Rd { d, p: start, end };
    let mut last = start;
    while rd.p < end {
        if d[rd.p] == 0 {
            rd.p += 1;
        } else {
            if d[rd.p] == 0x01 {
                set_locs.push(rd.p + 1);
            }
            rd.skip_cfa_op(ptr_width)?;
            last = rd.p;
        }
    }
    Some(last)
}

/// bfd's relocation cookie over offset-sorted relocations.
struct Cookie<'a> {
    r: &'a [Elf64Rela],
    i: usize,
}

impl Cookie<'_> {
    /// `ENSURE_NO_RELOCS`: only `R_*_NONE` before `pos`.
    fn ensure_none(&mut self, pos: usize) -> Option<()> {
        while let Some(r) = self.r.get(self.i) {
            if r.offset >= wide(pos) {
                break;
            }
            if r.rela_type != 0 || r.sym_idx != 0 {
                return None;
            }
            self.i += 1;
        }
        Some(())
    }

    /// `SKIP_RELOCS`.
    fn skip(&mut self, pos: usize) {
        while self.r.get(self.i).is_some_and(|r| r.offset < wide(pos)) {
            self.i += 1;
        }
    }

    /// `GET_RELOC`.
    fn at(&self, pos: usize) -> Option<usize> {
        self.r
            .get(self.i)
            .filter(|r| r.offset == wide(pos))
            .map(|_| self.i)
    }
}

/// Symbol facts `edit` needs while parsing one input.
struct SymFacts<'a> {
    obj: usize,
    object: &'a Elf64Object,
    /// Resolved definition of each global: (object, section index).
    defs: &'a FxHashMap<String, (usize, u16)>,
}

impl SymFacts<'_> {
    /// `REQUIRE (rsec->owner == abfd)` fails for the FDE start symbol.
    fn foreign_fde_target(&self, sym_idx: u32) -> bool {
        let Some(sym) = usize::try_from(sym_idx)
            .ok()
            .and_then(|i| self.object.symbols.get(i))
        else {
            return true;
        };
        if sym.binding() == STB_LOCAL {
            // A local symbol's section (if any) belongs to this object.
            return false;
        }
        match self.defs.get(sym.name.as_str()) {
            // Absolute and common definitions have no owning object.
            Some(&(o, shndx)) => o != self.obj || shndx == SHN_ABS || shndx == SHN_COMMON,
            // Undefined (or defined by a shared library): no section.
            None => false,
        }
    }

    /// `find_merged_cie`'s personality identity; `None` keeps the CIE
    /// unmerged (bfd's early returns).
    fn personality(&self, reloc: &Elf64Rela) -> Option<Personality> {
        let sym = usize::try_from(reloc.sym_idx)
            .ok()
            .and_then(|i| self.object.symbols.get(i))?;
        if sym.binding() != STB_LOCAL {
            return Some(Personality::Global(sym.name.as_str().to_owned()));
        }
        if sym.shndx == SHN_UNDEF || sym.shndx >= SHN_LORESERVE {
            return None;
        }
        Some(Personality::Local(self.obj, reloc.sym_idx))
    }

    /// `SYMBOL_REFERENCES_LOCAL` for a personality symbol of a PIC link.
    fn binds_local(&self, reloc: &Elf64Rela) -> bool {
        usize::try_from(reloc.sym_idx)
            .ok()
            .and_then(|i| self.object.symbols.get(i))
            .is_some_and(|s| s.binding() == STB_LOCAL || s.other & 3 != 0)
    }
}

/// What parsing found besides the entries.
enum Parsed {
    Entries(Vec<Entry>),
    /// bfd's `free_no_table`.
    Opaque,
    /// A PIC rewrite bfd performs and this module does not (module docs).
    PassThrough,
}

/// `_bfd_elf_parse_eh_frame`.
fn parse(d: &[u8], relocs: &[Elf64Rela], t: EhTarget, facts: &SymFacts) -> Parsed {
    let mut pic_rewrite = false;
    match parse_entries(d, relocs, t, facts, &mut pic_rewrite) {
        None => Parsed::Opaque,
        Some(_) if pic_rewrite => Parsed::PassThrough,
        Some(entries) => Parsed::Entries(entries),
    }
}

fn parse_entries(
    d: &[u8],
    relocs: &[Elf64Rela],
    t: EhTarget,
    facts: &SymFacts,
    pic_rewrite: &mut bool,
) -> Option<Vec<Entry>> {
    if u32::try_from(d.len()).is_err() {
        return None;
    }
    let mut cookie = Cookie { r: relocs, i: 0 };
    let mut entries: Vec<Entry> = Vec::new();
    let mut pos = 0usize;
    while pos != d.len() {
        let offset = pos;
        let mut rd = Rd {
            d,
            p: pos,
            end: d.len(),
        };
        rd.skip(4)?;
        let hdr_length = u32::from_le_bytes(d[offset..offset + 4].try_into().ok()?);
        if hdr_length == 0xffff_ffff {
            return None;
        }
        let hdr_length = usize::try_from(hdr_length).ok()?;
        if hdr_length > d.len() - rd.p {
            return None;
        }
        let end = rd.p + hdr_length;
        if hdr_length == 0 {
            // Terminator: only zero words may follow, up to the end.
            while rd.skip(4).is_some() {
                if read_uint(d, rd.p - 4, 4) != 0 {
                    return None;
                }
            }
            if rd.p != d.len() {
                return None;
            }
            cookie.ensure_none(rd.p)?;
            entries.push(Entry {
                offset,
                raw_size: 4,
                size: 4,
                removed: true,
                new_offset: 0,
                kind: Kind::Terminator,
            });
            break;
        }
        rd.end = end;
        rd.skip(4)?;
        let hdr_id = read_uint(d, rd.p - 4, 4);
        let mut set_locs = Vec::new();
        let (kind, insns, fde_encoding) = if hdr_id == 0 {
            let version = rd.byte()?;
            if !matches!(version, 1 | 3 | 4) {
                return None;
            }
            let aug_len = d[rd.p..end].iter().position(|&b| b == 0)?;
            if aug_len >= AUG_MAX {
                return None;
            }
            let augmentation = d[rd.p..rd.p + aug_len].to_vec();
            rd.p += aug_len + 1;
            if rd.p + 1 >= end {
                return None;
            }
            cookie.ensure_none(rd.p)?;
            if d[rd.p] == b'e' && d[rd.p + 1] == b'h' {
                rd.skip(u64::from(t.ptr_size))?;
                cookie.skip(rd.p);
            }
            if version >= 4 {
                if rd.p + 1 >= end || d[rd.p] != t.ptr_size || d[rd.p + 1] != 0 {
                    return None;
                }
                rd.p += 2;
            }
            let code_align = rd.leb(false)?;
            let data_align = rd.leb(true)?;
            let ra_column = if version == 1 {
                u64::from(rd.byte()?)
            } else {
                rd.leb(false)?
            };
            cookie.ensure_none(rd.p)?;
            let mut lsda_encoding = DW_EH_PE_OMIT;
            let mut fde_encoding = DW_EH_PE_OMIT;
            let mut per_encoding = DW_EH_PE_OMIT;
            let mut augmentation_size = 0;
            let mut per_reloc = None;
            let mut aligned8 = false;
            if !augmentation.starts_with(b"eh") {
                let mut chars = augmentation.as_slice();
                if chars.first() == Some(&b'z') {
                    chars = &chars[1..];
                    augmentation_size = rd.leb(false)?;
                    cookie.ensure_none(rd.p)?;
                }
                for &c in chars {
                    match c {
                        b'B' | b'G' if t.aarch64 => {}
                        b'L' => {
                            lsda_encoding = rd.byte()?;
                            cookie.ensure_none(rd.p)?;
                            if pe_width(lsda_encoding, t.ptr_size) == 0 {
                                return None;
                            }
                        }
                        b'R' => {
                            fde_encoding = rd.byte()?;
                            cookie.ensure_none(rd.p)?;
                            if pe_width(fde_encoding, t.ptr_size) == 0 {
                                return None;
                            }
                        }
                        b'S' => {}
                        b'P' => {
                            per_encoding = rd.byte()?;
                            let per_width = pe_width(per_encoding, t.ptr_size);
                            if per_width == 0 {
                                return None;
                            }
                            if per_encoding & DW_EH_PE_APPL_MASK == DW_EH_PE_ALIGNED {
                                let pad = rd.p.wrapping_neg() & (per_width - 1);
                                rd.skip(wide(pad))?;
                                aligned8 = per_width == 8;
                            }
                            cookie.ensure_none(rd.p)?;
                            let r = cookie.at(rd.p)?;
                            per_reloc = Some(r);
                            if t.pic
                                && per_encoding & DW_EH_PE_APPL_MASK == DW_EH_PE_ABSPTR
                                && facts.binds_local(&relocs[r])
                            {
                                *pic_rewrite = true;
                            }
                            while cookie.at(rd.p).is_some() {
                                cookie.i += 1;
                            }
                            rd.skip(wide(per_width))?;
                        }
                        _ => return None,
                    }
                }
            }
            if t.pic
                && (fde_encoding & DW_EH_PE_APPL_MASK == DW_EH_PE_ABSPTR
                    || (fde_encoding == DW_EH_PE_OMIT
                        && per_encoding & DW_EH_PE_APPL_MASK != DW_EH_PE_ALIGNED))
            {
                *pic_rewrite = true;
            }
            if fde_encoding == DW_EH_PE_OMIT {
                fde_encoding = DW_EH_PE_ABSPTR;
            }
            let insns = rd.p;
            cookie.ensure_none(end)?;
            let cie = Cie {
                key: CieKey {
                    length: u32::try_from(hdr_length).ok()?,
                    version,
                    augmentation,
                    code_align,
                    data_align,
                    ra_column,
                    augmentation_size,
                    personality: Personality::None,
                    per_encoding,
                    lsda_encoding,
                    fde_encoding,
                    initial_insns: d[insns..end].to_vec(),
                },
                per_reloc,
                aligned8,
                merged_with: None,
            };
            (Kind::Cie(Box::new(cie)), insns, fde_encoding)
        } else {
            let cie_offset = wide(offset + 4).wrapping_sub(hdr_id) & 0xffff_ffff;
            let cie_idx = entries
                .iter()
                .position(|e| matches!(e.kind, Kind::Cie(_)) && wide(e.offset) == cie_offset)?;
            let Kind::Cie(cie) = &entries[cie_idx].kind else {
                return None;
            };
            let (fde_encoding, lsda_encoding) = (cie.key.fde_encoding, cie.key.lsda_encoding);
            let z = cie.key.augmentation.first() == Some(&b'z');
            cookie.ensure_none(rd.p)?;
            let r = cookie.at(rd.p)?;
            if facts.foreign_fde_target(relocs[r].sym_idx) {
                return None;
            }
            let start = rd.p;
            let width = pe_width(fde_encoding, t.ptr_size);
            rd.skip(wide(2 * width))?;
            cookie.skip(rd.p - width);
            let zero_range =
                cookie.at(rd.p - width).is_none() && read_uint(d, rd.p - width, width) == 0;
            let mut length = if z { rd.leb(false)? } else { 0 };
            let mut lsda_offset = 0;
            if lsda_encoding != DW_EH_PE_OMIT {
                cookie.skip(rd.p);
                if t.pic
                    && lsda_encoding & DW_EH_PE_APPL_MASK == DW_EH_PE_ABSPTR
                    && cookie.at(rd.p).is_some()
                {
                    *pic_rewrite = true;
                }
                lsda_offset = rd.p - start;
                if !z {
                    length = wide(end - rd.p);
                }
            }
            rd.skip(length)?;
            let insns = rd.p;
            cookie.skip(end);
            let fde = Fde {
                cie: (!zero_range).then_some(cie_idx),
                target: (0, cie_idx),
                fde_encoding,
                lsda_encoding,
                lsda_offset,
                set_locs: Vec::new(),
            };
            (Kind::Fde(fde), insns, fde_encoding)
        };
        let width = pe_width(fde_encoding, t.ptr_size);
        let insns_end = skip_non_nops(d, insns, end, width, &mut set_locs)?;
        let mut kind = kind;
        match &mut kind {
            Kind::Cie(cie) => {
                if !set_locs.is_empty() {
                    return None;
                }
                let trim = end - insns_end;
                cie.key.length -= u32::try_from(trim).ok()?;
                cie.key.initial_insns.truncate(insns_end - insns);
            }
            Kind::Fde(fde) => {
                if fde_encoding & DW_EH_PE_APPL_MASK == DW_EH_PE_PCREL {
                    fde.set_locs = set_locs.iter().map(|&p| p - (offset + 8)).collect();
                }
            }
            Kind::Terminator => {}
        }
        // A live relocation inside trimmed padding would be cut off.
        if relocs
            .iter()
            .any(|r| r.offset >= wide(insns_end) && r.offset < wide(end) && r.rela_type != 0)
        {
            return None;
        }
        entries.push(Entry {
            offset,
            raw_size: end - offset,
            size: insns_end - offset,
            removed: true,
            new_offset: 0,
            kind,
        });
        pos = end;
    }
    Some(entries)
}

/// `find_merged_cie` for CIE entry `c` of input `k`.
fn find_merged_cie(
    entries: &mut [Entry],
    k: usize,
    c: usize,
    relocs: &[Elf64Rela],
    facts: &SymFacts,
    cies: &mut FxHashMap<CieKey, (usize, usize)>,
) -> (usize, usize) {
    let ent = &mut entries[c];
    let Kind::Cie(cie) = &mut ent.kind else {
        return (k, c);
    };
    if !ent.removed {
        return (k, c);
    }
    if let Some(m) = cie.merged_with {
        return m;
    }
    ent.removed = false;
    if cie.key.per_encoding != DW_EH_PE_OMIT {
        match cie.per_reloc.and_then(|r| facts.personality(&relocs[r])) {
            Some(p) => cie.key.personality = p,
            None => return (k, c),
        }
    }
    if !cie.key.mergeable() {
        return (k, c);
    }
    if let Some(&first) = cies.get(&cie.key) {
        ent.removed = true;
        cie.merged_with = Some(first);
        return first;
    }
    cies.insert(cie.key.clone(), (k, c));
    (k, c)
}

/// The resolved definition of every global symbol: strong before weak
/// before common, first in link order within each class.
fn global_definitions(objs: &[(String, Elf64Object)]) -> FxHashMap<String, (usize, u16)> {
    let mut defs: FxHashMap<String, (usize, u16, u8)> = FxHashMap::default();
    for (o, (_, obj)) in objs.iter().enumerate() {
        for sym in &obj.symbols {
            if sym.binding() == STB_LOCAL || sym.shndx == SHN_UNDEF {
                continue;
            }
            let rank = if sym.shndx == SHN_COMMON {
                2
            } else if sym.binding() == STB_WEAK {
                1
            } else {
                0
            };
            let e = defs
                .entry(sym.name.as_str().to_owned())
                .or_insert((o, sym.shndx, rank));
            if rank < e.2 {
                *e = (o, sym.shndx, rank);
            }
        }
    }
    defs.into_iter().map(|(n, (o, s, _))| (n, (o, s))).collect()
}

/// Edit the `.eh_frame` inputs `inputs` (in output order) of `objs` the way
/// GNU ld 2.47 does; see the module documentation.
pub fn edit(
    objs: &mut [(String, Elf64Object)],
    inputs: &[(usize, usize)],
    t: EhTarget,
) -> Result<EhFrameLayout, String> {
    let defs = global_definitions(objs);
    let mut cies: FxHashMap<CieKey, (usize, usize)> = FxHashMap::default();
    let mut hdr_table = true;
    let mut present = false;
    let mut plans: Vec<Input> = Vec::with_capacity(inputs.len());
    let last = inputs.len().saturating_sub(1);

    // Parse and discard, input by input (bfd_elf_discard_info's order).
    for (k, &(o, s)) in inputs.iter().enumerate() {
        let obj = &objs[o].1;
        let d = obj.section_data[s].as_slice();
        let addralign = obj.sections[s].addralign.max(1);
        present |= d.len() > 8;
        if d.is_empty() {
            plans.push(Input {
                obj: o,
                sec: s,
                plan: Plan::Raw { opaque: false },
                size: 0,
                addralign,
            });
            continue;
        }
        let mut relocs = obj.relocations.get(s).cloned().unwrap_or_default();
        relocs.sort_by_key(|r| r.offset);
        let facts = SymFacts {
            obj: o,
            object: obj,
            defs: &defs,
        };
        let mut entries = match parse(d, &relocs, t, &facts) {
            Parsed::Entries(e) => e,
            Parsed::Opaque => {
                hdr_table = false;
                plans.push(Input {
                    obj: o,
                    sec: s,
                    plan: Plan::Raw { opaque: true },
                    size: d.len(),
                    addralign,
                });
                continue;
            }
            Parsed::PassThrough => {
                plans.push(Input {
                    obj: o,
                    sec: s,
                    plan: Plan::Raw { opaque: false },
                    size: d.len(),
                    addralign,
                });
                continue;
            }
        };
        for e in 0..entries.len() {
            let cie_idx = match &entries[e].kind {
                Kind::Terminator => {
                    entries[e].removed = k != last;
                    continue;
                }
                Kind::Cie(_) => continue,
                Kind::Fde(f) => match f.cie {
                    Some(c) => {
                        if t.pic && f.fde_encoding & DW_EH_PE_APPL_MASK == DW_EH_PE_ALIGNED {
                            hdr_table = false;
                        }
                        c
                    }
                    None => continue,
                },
            };
            entries[e].removed = false;
            let target = find_merged_cie(&mut entries, k, cie_idx, &relocs, &facts, &mut cies);
            if let Kind::Fde(f) = &mut entries[e].kind {
                f.target = target;
            }
        }
        // Re-align the surviving records.
        let mut off = 0usize;
        for ent in entries.iter_mut().filter(|e| !e.removed) {
            let align = match &ent.kind {
                Kind::Terminator => 4,
                Kind::Cie(c) if c.aligned8 => 8,
                Kind::Cie(_) => 4,
                Kind::Fde(f) => pe_width(f.fde_encoding, t.ptr_size).max(4),
            };
            off = align_up(off, align);
            ent.new_offset = off;
            off += ent.size;
        }
        plans.push(Input {
            obj: o,
            sec: s,
            plan: Plan::Edited { entries, relocs },
            size: align_up(off, 4),
            addralign,
        });
    }

    // elflink.c: exclude trailing empty inputs, pad all inputs before the
    // last non-terminator one to the output section alignment.
    let eh_alignment = plans.iter().map(|p| p.addralign).max().unwrap_or(1);
    let eh_align = usize::try_from(eh_alignment).map_err(|_| ".eh_frame alignment overflow")?;
    let mut tail = plans.len();
    while tail > 0 {
        let p = &mut plans[tail - 1];
        if p.size == 0 {
            p.addralign = 1;
        } else if p.size > 4 {
            break;
        }
        tail -= 1;
    }
    for p in plans.iter_mut().take(tail.saturating_sub(1)) {
        if p.size != 4 {
            p.size = align_up(p.size, eh_align);
        }
    }
    // The output keeps the alignment of every input, excluded ones included:
    // the first placed input sits at offset 0, so it can carry it.
    if let Some(p) = plans.iter_mut().find(|p| p.size > 0) {
        p.addralign = p.addralign.max(eh_alignment);
    }

    // Planned placement.
    let mut bases = Vec::with_capacity(plans.len());
    let mut off = 0u64;
    for p in &plans {
        let base = off.div_ceil(p.addralign) * p.addralign;
        bases.push(base);
        off = base + wide(p.size);
    }

    // Write.
    for (k, p) in plans.iter().enumerate() {
        let (o, s) = (p.obj, p.sec);
        let new_data = match &p.plan {
            Plan::Raw { .. } => {
                let mut v = objs[o].1.section_data[s].to_owned_vec();
                if v.len() == p.size {
                    None
                } else {
                    v.resize(p.size, 0);
                    Some((v, None))
                }
            }
            Plan::Edited { entries, relocs } => {
                let d = objs[o].1.section_data[s].as_slice();
                let out = write_section(d, entries, p.size, k, &plans, &bases, t.ptr_size)?;
                let rel = remap_relocs(relocs, entries);
                Some((out, Some(rel)))
            }
        };
        if let Some((data, rel)) = new_data {
            let obj = &mut objs[o].1;
            obj.section_data[s] = SectionData::owned(data);
            if let Some(rel) = rel {
                obj.relocations[s] = rel;
            }
        }
        if let Plan::Edited { entries, .. } = &p.plan {
            adjust_symbols(&mut objs[o].1, s, entries, p.size, bases[k], &plans, &bases);
        }
        let sec = &mut objs[o].1.sections[s];
        sec.size = wide(p.size);
        sec.addralign = p.addralign;
    }

    let bases = plans
        .iter()
        .zip(&bases)
        .filter(|(p, _)| p.size > 0)
        .map(|(p, &b)| ((p.obj, p.sec), b))
        .collect();
    Ok(EhFrameLayout {
        bases,
        hdr_table,
        present,
    })
}

fn cie_new_offset(plans: &[Input], (k, c): (usize, usize)) -> Option<usize> {
    match &plans.get(k)?.plan {
        Plan::Edited { entries, .. } => entries.get(c).map(|e| e.new_offset),
        Plan::Raw { .. } => None,
    }
}

/// `next_cie_fde_offset`.
fn next_offset(entries: &[Entry], i: usize, size: usize) -> usize {
    entries[i + 1..]
        .iter()
        .find(|e| !e.removed)
        .map_or(size, |e| e.new_offset)
}

/// `_bfd_elf_write_section_eh_frame` (minus the PIC rewrites).
fn write_section(
    d: &[u8],
    entries: &[Entry],
    size: usize,
    k: usize,
    plans: &[Input],
    bases: &[u64],
    ptr_size: u8,
) -> Result<Vec<u8>, String> {
    let mut out = vec![0u8; size];
    let has_reloc = |relocs: &[Elf64Rela], at: usize| relocs.iter().any(|r| r.offset == wide(at));
    let Plan::Edited { relocs, .. } = &plans[k].plan else {
        return Err("internal: .eh_frame write of an unedited input".into());
    };
    for (i, ent) in entries.iter().enumerate() {
        if ent.removed {
            continue;
        }
        let new = ent.new_offset;
        out[new..new + ent.size].copy_from_slice(&d[ent.offset..ent.offset + ent.size]);
        if matches!(ent.kind, Kind::Terminator) {
            continue;
        }
        let len = next_offset(entries, i, size) - new - 4;
        write_uint(&mut out, new, 4, wide(len));
        let Kind::Fde(f) = &ent.kind else {
            continue;
        };
        let cie_off =
            cie_new_offset(plans, f.target).ok_or("internal: .eh_frame FDE lost its CIE")?;
        let ptr = (bases[k] + wide(new) + 4).wrapping_sub(bases[f.target.0] + wide(cie_off));
        write_uint(&mut out, new + 4, 4, ptr);
        let delta = wide(ent.offset).wrapping_sub(wide(new));
        let start_old = ent.offset + 8;
        let start_new = new + 8;
        if f.lsda_encoding & DW_EH_PE_APPL_MASK == DW_EH_PE_PCREL {
            let w = pe_width(f.lsda_encoding, ptr_size);
            let at = start_old + f.lsda_offset;
            let v = read_uint(d, at, w);
            if v != 0 && !has_reloc(relocs, at) {
                write_uint(
                    &mut out,
                    start_new + f.lsda_offset,
                    w,
                    v.wrapping_add(delta),
                );
            }
        }
        let w = pe_width(f.fde_encoding, ptr_size);
        for &sl in &f.set_locs {
            let v = read_uint(d, start_old + sl, w);
            if v != 0 && !has_reloc(relocs, start_old + sl) {
                write_uint(&mut out, start_new + sl, w, v.wrapping_add(delta));
            }
        }
    }
    Ok(out)
}

/// The entry owning original offset `off` (bfd's `offset_adjust` search).
fn owner(entries: &[Entry], off: u64) -> Option<usize> {
    let i = entries.partition_point(|e| wide(e.offset) <= off);
    i.checked_sub(1)
}

/// Move the relocations with their records; drop those of removed ones.
fn remap_relocs(relocs: &[Elf64Rela], entries: &[Entry]) -> Vec<Elf64Rela> {
    relocs
        .iter()
        .filter_map(|r| {
            let e = &entries[owner(entries, r.offset)?];
            let rel = r.offset - wide(e.offset);
            if e.removed || rel >= wide(e.size) {
                return None;
            }
            let mut r = r.clone();
            r.offset = wide(e.new_offset) + rel;
            Some(r)
        })
        .collect()
}

/// `adjust_eh_frame_local_symbols` / `_bfd_elf_adjust_eh_frame_global_symbol`.
fn adjust_symbols(
    obj: &mut Elf64Object,
    s: usize,
    entries: &[Entry],
    size: usize,
    base: u64,
    plans: &[Input],
    bases: &[u64],
) {
    let Ok(shndx) = u16::try_from(s) else {
        return;
    };
    for sym in obj.symbols.iter_mut().filter(|y| y.shndx == shndx) {
        let local_ok = sym.info <= ((STB_LOCAL << 4) | STT_OBJECT);
        if sym.binding() == STB_LOCAL && !local_ok {
            continue;
        }
        let Some(i) = owner(entries, sym.value) else {
            continue;
        };
        let ent = &entries[i];
        let delta = if !ent.removed {
            wide(ent.new_offset).wrapping_sub(wide(ent.offset))
        } else if let Kind::Cie(c) = &ent.kind
            && let Some(m) = c.merged_with
            && let Some(cie_off) = cie_new_offset(plans, m)
        {
            (bases[m.0] + wide(cie_off))
                .wrapping_sub(wide(ent.offset))
                .wrapping_sub(base)
        } else {
            wide(next_offset(entries, i, size)).wrapping_sub(wide(ent.offset))
        };
        sym.value = sym.value.wrapping_add(delta);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::linker_common::{Elf64Section, Elf64Symbol, SymStr};

    const T: EhTarget = EhTarget {
        ptr_size: 8,
        pic: false,
        aarch64: false,
    };
    const R_PCREL: u32 = 57;
    const R_ADD32: u32 = 35;

    fn record(id: u32, body: &[u8]) -> Vec<u8> {
        let mut v = u32::try_from(body.len() + 4)
            .unwrap()
            .to_le_bytes()
            .to_vec();
        v.extend(id.to_le_bytes());
        v.extend(body);
        v
    }

    /// GCC's RISC-V CIE: "zR", code 1, data -4 (0x7c), ra 1, pcrel sdata4,
    /// `DW_CFA_def_cfa sp, 0`, then `nops` padding bytes.
    fn cie(nops: usize) -> Vec<u8> {
        let mut b = vec![1, b'z', b'R', 0, 1, 0x7c, 1, 1, 0x1b, 0x0c, 2, 0];
        b.resize(b.len() + nops, 0);
        record(0, &b)
    }

    /// An FDE at `at` for the CIE at `cie_at`; `range` is the literal range.
    fn fde(at: usize, cie_at: usize, range: u32, nops: usize) -> Vec<u8> {
        let mut b = vec![0; 4];
        b.extend(range.to_le_bytes());
        b.extend([0, 0x0e, 16]);
        b.resize(b.len() + nops, 0);
        record(u32::try_from(at + 4 - cie_at).unwrap(), &b)
    }

    fn rela(offset: usize, rela_type: u32, sym_idx: u32) -> Elf64Rela {
        Elf64Rela {
            offset: wide(offset),
            sym_idx,
            rela_type,
            addend: 0,
        }
    }

    fn sym(name: &str, info: u8, shndx: u16, value: u64) -> Elf64Symbol {
        Elf64Symbol {
            name_idx: 0,
            name: SymStr::new(name),
            info,
            other: 0,
            shndx,
            value,
            size: 0,
        }
    }

    /// Object whose section 1 is `.eh_frame` and section 2 `.text`.
    fn object(eh: Vec<u8>, relocs: Vec<Elf64Rela>, align: u64) -> (String, Elf64Object) {
        let section = |name: &str, size: usize, addralign: u64| Elf64Section {
            name_idx: 0,
            name: name.into(),
            sh_type: 1,
            flags: 2,
            addr: 0,
            offset: 0,
            size: wide(size),
            link: 0,
            info: 0,
            addralign,
            entsize: 0,
        };
        let obj = Elf64Object {
            sections: vec![
                section("", 0, 0),
                section(".eh_frame", eh.len(), align),
                section(".text", 64, 4),
            ],
            symbols: vec![
                sym("", 0, 0, 0),
                sym(".LFB0", 0, 2, 0),
                sym("__gcc_personality_v0", 0x10, 0, 0),
            ],
            section_data: vec![
                SectionData::empty(),
                SectionData::owned(eh),
                SectionData::owned(vec![0; 64]),
            ],
            relocations: vec![Vec::new(), relocs, Vec::new()],
            source_name: "t.o".into(),
            e_flags: 0,
        };
        ("t.o".into(), obj)
    }

    fn eh(objs: &[(String, Elf64Object)], o: usize) -> Vec<u8> {
        objs[o].1.section_data[1].to_owned_vec()
    }

    fn run(objs: &mut [(String, Elf64Object)]) -> EhFrameLayout {
        let inputs: Vec<_> = (0..objs.len()).map(|o| (o, 1)).collect();
        edit(objs, &inputs, T).unwrap()
    }

    #[test]
    fn trims_nop_padding_and_realigns() {
        let mut d = cie(4);
        let f = d.len();
        d.extend(fde(f, 0, 8, 5));
        let mut objs = vec![object(d, vec![rela(f + 8, R_PCREL, 1)], 8)];
        let l = run(&mut objs);
        let mut want = cie(0);
        want.extend(fde(want.len(), 0, 8, 1));
        assert_eq!(eh(&objs, 0), want, "CIE 24 -> 20, FDE 24 -> 19 + 1 pad");
        assert_eq!(objs[0].1.relocations[1][0].offset, 28);
        assert!(l.hdr_table());
    }

    #[test]
    fn drops_unrelocated_zero_range_fde_and_unused_cie() {
        let mut d = cie(0);
        d.extend(fde(20, 0, 0, 1));
        let mut objs = vec![object(d, vec![rela(28, R_PCREL, 1)], 4)];
        run(&mut objs);
        assert!(eh(&objs, 0).is_empty());
        assert!(objs[0].1.relocations[1].is_empty());
        assert_eq!(objs[0].1.sections[1].size, 0);
    }

    #[test]
    fn keeps_relocated_zero_range_fde() {
        let mut d = cie(0);
        d.extend(fde(20, 0, 0, 1));
        let r = vec![
            rela(28, R_PCREL, 1),
            rela(32, R_ADD32, 1),
            rela(32, R_ADD32 + 4, 1),
        ];
        let mut objs = vec![object(d.clone(), r, 4)];
        run(&mut objs);
        assert_eq!(eh(&objs, 0), d);
        assert_eq!(objs[0].1.relocations[1].len(), 3);
    }

    #[test]
    fn merges_identical_cies_across_inputs() {
        let mk = || {
            let mut d = cie(0);
            d.extend(fde(20, 0, 8, 1));
            object(d, vec![rela(28, R_PCREL, 1)], 4)
        };
        let mut objs = vec![mk(), mk()];
        let l = run(&mut objs);
        assert_eq!(eh(&objs, 0).len(), 40);
        let second = eh(&objs, 1);
        assert_eq!(second.len(), 20, "second CIE merged away");
        // FDE of input 1 at output 40; CIE pointer = 40 + 4 - 0.
        assert_eq!(read_uint(&second, 4, 4), 44);
        assert_eq!(objs[1].1.relocations[1][0].offset, 8);
        let mut map = FxHashMap::default();
        map.insert((0, 1), (0, 0));
        map.insert((1, 1), (0, 40));
        l.verify(&map).unwrap();
        map.insert((1, 1), (0, 44));
        assert!(l.verify(&map).is_err());
    }

    #[test]
    fn eh_and_long_cies_never_merge() {
        let long = |n: usize| {
            let mut b = vec![1, b'z', b'R', 0, 1, 0x7c, 1, 1, 0x1b];
            b.extend(std::iter::repeat_n([0x0e, 16], n).flatten());
            let mut d = record(0, &b);
            let f = d.len();
            d.extend(fde(f, 0, 8, 1));
            object(d, vec![rela(f + 8, R_PCREL, 1)], 4)
        };
        let mut objs = vec![long(26), long(26), long(24), long(24)];
        run(&mut objs);
        let sizes: Vec<_> = (0..4).map(|o| eh(&objs, o).len()).collect();
        assert_eq!(sizes[0], sizes[1], "52-byte CIEs both kept");
        assert!(sizes[3] < sizes[2], "48-byte CIEs merge");
    }

    #[test]
    fn terminator_only_in_last_input_and_padding_inside_last_record() {
        let mut a = cie(0);
        a.extend(fde(20, 0, 8, 1));
        a.extend(fde(40, 0, 8, 1));
        a.extend([0; 4]);
        let mut b = cie(0);
        b.extend(fde(20, 0, 8, 1));
        b.extend([0; 4]);
        let mut objs = vec![
            object(a, vec![rela(28, R_PCREL, 1), rela(48, R_PCREL, 1)], 4),
            object(b, vec![rela(28, R_PCREL, 1)], 8),
            object(vec![0; 4], Vec::new(), 4),
        ];
        let l = run(&mut objs);
        let a = eh(&objs, 0);
        assert_eq!(a.len(), 64, "terminator dropped, 60 padded to 8");
        assert_eq!(read_uint(&a, 40, 4), 20, "padding inside the last FDE");
        assert_eq!(eh(&objs, 1).len(), 20, "CIE merged, last non-empty: no pad");
        assert_eq!(eh(&objs, 2), vec![0; 4]);
        assert_eq!(l.bases, vec![((0, 1), 0), ((1, 1), 64), ((2, 1), 84)]);
        assert_eq!(objs[0].1.sections[1].addralign, 8);
    }

    #[test]
    fn unparsable_input_is_opaque_and_drops_search_table() {
        let mut b = vec![1, b'z', b'X', 0, 1, 0x7c, 1, 0];
        b.extend([0x0c, 2, 0]);
        let d = record(0, &b);
        let mut objs = vec![object(d.clone(), Vec::new(), 4)];
        let l = run(&mut objs);
        assert_eq!(eh(&objs, 0), d);
        assert!(!l.hdr_table());
    }

    #[test]
    fn symbols_follow_records() {
        let mut d = cie(4);
        d.extend(fde(24, 0, 8, 1));
        let mut objs = vec![object(d, vec![rela(32, R_PCREL, 1)], 4)];
        objs[0].1.symbols.push(sym(".Lfde", 0, 1, 24));
        objs[0].1.symbols.push(sym(".Lsec", 3, 1, 24));
        run(&mut objs);
        assert_eq!(objs[0].1.symbols[3].value, 20);
        assert_eq!(objs[0].1.symbols[4].value, 24, "section symbols untouched");
    }

    #[test]
    fn absptr_fde_aligns_to_pointer_width() {
        // "zR" absptr: FDE fields 8 bytes wide, aligned to 8.
        let b = vec![1, b'z', b'R', 0, 1, 0x7c, 1, 1, 0x00, 0x0c, 2, 0];
        let mut d = record(0, &b);
        let mut fb = vec![0; 8];
        fb.extend(8u64.to_le_bytes());
        fb.extend([0, 0x0e, 16, 0]);
        d.extend(record(24, &fb));
        let mut objs = vec![object(d, vec![rela(28, 2, 1)], 8)];
        run(&mut objs);
        let out = eh(&objs, 0);
        assert_eq!(out.len(), 52);
        assert_eq!(read_uint(&out, 0, 4), 20, "CIE padded to the FDE at 24");
        assert_eq!(read_uint(&out, 24, 4), 24, "FDE length");
        assert_eq!(read_uint(&out, 28, 4), 28, "CIE pointer");
        assert_eq!(objs[0].1.relocations[1][0].offset, 32);
    }

    #[test]
    fn personality_identity_decides_merging() {
        let mk = |sym_idx: u32| {
            // "zPR": P = 0x9b (indirect pcrel sdata4) + 4-byte pointer.
            let mut b = vec![
                1, b'z', b'P', b'R', 0, 1, 0x7c, 1, 6, 0x9b, 0, 0, 0, 0, 0x1b,
            ];
            b.extend([0x0c, 2, 0, 0]);
            let mut d = record(0, &b);
            let f = d.len();
            d.extend(fde(f, 0, 8, 1));
            object(
                d,
                vec![rela(18, R_PCREL, sym_idx), rela(f + 8, R_PCREL, 1)],
                4,
            )
        };
        let mut objs = vec![mk(2), mk(2)];
        run(&mut objs);
        assert_eq!(eh(&objs, 1).len(), 20, "same global personality merges");
        let mut objs = vec![mk(1), mk(1)];
        run(&mut objs);
        assert!(
            eh(&objs, 1).len() > 20,
            "local personalities stay per object"
        );
    }

    #[test]
    fn hdr_follows_bfd_sizing() {
        let mut d = cie(0);
        d.extend(fde(20, 0, 0, 1));
        let mut objs = vec![object(d, vec![rela(28, R_PCREL, 1)], 4)];
        let l = run(&mut objs);
        assert_eq!(l.hdr_size(0), 12, "inputs had records: header, empty table");
        let mut b = vec![1, b'z', b'X', 0, 1, 0x7c, 1, 0];
        b.extend([0x0c, 2, 0]);
        let mut objs = vec![object(record(0, &b), Vec::new(), 4)];
        let l = run(&mut objs);
        assert_eq!(l.hdr_size(7), 8);
        let mut slot = [0u8; 8];
        l.write_hdr(&mut slot, &[], 0x1100, 0x1000).unwrap();
        assert_eq!(slot, [1, 0x1b, 0xff, 0xff, 0xfc, 0, 0, 0]);
        let mut objs = vec![object(vec![0; 4], Vec::new(), 4)];
        assert_eq!(run(&mut objs).hdr_size(0), 0, "terminator only: no header");
    }

    #[test]
    fn pic_absolute_encoding_passes_through() {
        let b = vec![
            1, b'z', b'R', 0, 1, 0x7c, 1, 1, 0x00, 0x0c, 2, 0, 0, 0, 0, 0,
        ];
        let d = record(0, &b);
        let mut objs = vec![object(d.clone(), Vec::new(), 4)];
        let l = edit(&mut objs, &[(0, 1)], EhTarget { pic: true, ..T }).unwrap();
        assert_eq!(eh(&objs, 0), d);
        assert!(l.hdr_table());
    }
}
