//! RISC-V linker relaxation, decision for decision as GNU ld 2.47 does it
//! (`_bfd_riscv_relax_section` and its helpers in `bfd/elfnn-riscv.c`).
//!
//! # What is relaxed
//!
//! Every candidate is an instruction whose relocation is paired with an
//! `R_RISCV_RELAX` at the same offset (the assembler's licence to rewrite it):
//!
//! | sequence                      | becomes                          | saves |
//! |-------------------------------|----------------------------------|-------|
//! | `auipc`+`jalr` (`CALL[_PLT]`) | `c.j` / `jal` / `jalr rd,lo(x0)` | 6/4/4 |
//! | `jal x0` (`JAL`)              | `c.j`                            | 2     |
//! | `lui`+`addi/ld/sd` (`HI20`)   | `addi/ld/sd ..(gp|x0)` / `c.lui` | 4/2   |
//! | `lui`+`add tp`+`ld` (`TPREL`) | `ld ..(tp)`                      | 8     |
//! | `auipc`+`addi/ld` (`PCREL`)   | `addi/ld ..(gp|x0)` (non-PIC)    | 4     |
//! | `R_RISCV_ALIGN` NOP runs      | the NOPs the final address needs | rest  |
//!
//! # Why bfd's exact algorithm, quirks included
//!
//! A relaxing linker's output is defined by its *schedule*, not just by the
//! rewrite rules: which candidate is looked at first, against which
//! addresses, with which safety margin, and when it stops.  Two linkers that
//! both relax "correctly" can differ in every byte after the first decision
//! taken in a different order.  lccc-ld follows bfd's schedule so its relaxed
//! code is the code GNU ld would produce from the same layout -- the property
//! `tests/regression/check_riscv_relax_bfd.py` gates against a pinned
//! binutils 2.47 -- and every one of bfd's margins is a proof obligation this
//! port inherits rather than re-derives:
//!
//! * **Trips.**  Pass 0 relaxes everything but `R_RISCV_ALIGN` and repeats
//!   while a trip asked for another; pass 1 handles alignment once.  Every
//!   rewrite that deletes bytes asks for another trip (bfd 2.47 sets
//!   `*again` in `relax_call`, `relax_jal`, `relax_lui`, `relax_tls_le` and
//!   `relax_pc`), so a `call` turned into `jal` -- kept relaxable through
//!   `R_RISCV_DELETE_AND_RELAX` -- is looked at again as `jal` to `c.j`
//!   once later trips have pulled its target closer.  Rewrites that only
//!   retype a relocation (`lo12` to `gprel`/`tprel`) do not.
//! * **Addresses during a trip.**  ld sizes sections in layout order and
//!   relaxes each input section *before* placing it.  So while section `i`
//!   of output section `M` is relaxed: output sections before `M` are at
//!   their new addresses, `M` starts at its new address, inputs of `M`
//!   before `i` are at their new offsets, `i` and later inputs keep last
//!   trip's offsets, and later output sections, `__global_pointer$` and the
//!   TLS segment keep last trip's addresses.  [`Tracker`] reproduces that
//!   from a [`LayoutProbe`] of last trip's layout.
//! * **Deletions are deferred** to the end of a section in pass 0
//!   (`_riscv_relax_delete_piecewise` + `riscv_relax_resolve_delete_relocs`),
//!   so every decision inside a section sees the section as it entered the
//!   trip; pass-1 alignment deletes immediately.
//! * **Margins.**  A call/jump target must stay in range after any later
//!   alignment growth: `max_alignment` (largest alignment of any output
//!   section, computed once) or, within one output section, that section's
//!   alignment.  A gp-relative access uses the largest alignment of the
//!   output sections within +-2 KiB of gp (recomputed each trip), widened to
//!   the data-segment alignment (`MAXPAGESIZE + COMMONPAGESIZE` under
//!   `-z relro`, the default) when the address lies outside its section, plus
//!   the symbol's remaining size (`reserve_size`) so the whole object stays
//!   addressable.  `reserve_size` is not reset between relocations and an
//!   `STT_FUNC` global leaves the previous value in place -- bfd's quirk, kept.
//!
//! # What lccc-ld adds or withholds, and why
//!
//! * A `CALL_PLT` to `__tls_get_addr` that lccc-ld's general-dynamic to
//!   local-exec rewrite (`reloc::collect_gd_tls_relax_info`) will replace by
//!   `add a0,a0,tp; nop` is not relaxed: that rewrite needs the 8-byte call
//!   slot, and bfd (which has no GD->LE rewrite) never produces the sequence
//!   either.  Calls of a general-dynamic access to an *import* keep their
//!   relaxation, exactly as in bfd.
//! * Deleting bytes also maps the addend of a relocation against the
//!   section's own `STT_SECTION` symbol (`.text+0x40` names a byte, not an
//!   offset from a label).  GNU as never emits such references into
//!   relaxable code -- it keeps local labels, and their addends (`.LVL3-1`)
//!   are deliberately left alone, as bfd leaves them -- but lccc's own
//!   assembler may, and a byte reference must keep naming the same byte.
//! * Every relaxed relocation is range-checked again when it is applied
//!   (`reloc.rs`), so a margin this model got wrong is a link error, never
//!   silently wrong code.

use super::elf_read::ElfObject;
use super::relocations::{
    GlobalSym, MergedSection, R_RISCV_ALIGN, R_RISCV_CALL, R_RISCV_CALL_PLT, R_RISCV_GPREL_I,
    R_RISCV_GPREL_S, R_RISCV_HI20, R_RISCV_JAL, R_RISCV_LO12_I, R_RISCV_LO12_S, R_RISCV_NONE,
    R_RISCV_PCREL_HI20, R_RISCV_PCREL_LO12_I, R_RISCV_PCREL_LO12_S, R_RISCV_RELAX,
    R_RISCV_RVC_JUMP, R_RISCV_RVC_LUI, R_RISCV_TLS_GD_HI20, R_RISCV_TPREL_ADD, R_RISCV_TPREL_HI20,
    R_RISCV_TPREL_I, R_RISCV_TPREL_LO12_I, R_RISCV_TPREL_LO12_S, R_RISCV_TPREL_S,
};
use crate::backend::linker_common::SectionData;
use crate::common::fx_hash::{FxHashMap, FxHashSet};

/// bfd's internal `R_RISCV_DELETE` / `R_RISCV_DELETE_AND_RELAX`: a deferred
/// deletion of `addend` bytes at `offset`.  Never leaves this module -- every
/// one is resolved to `R_RISCV_NONE` / `R_RISCV_RELAX` at the end of the
/// section that created it.
const R_DELETE: u32 = 0x1_0000;
const R_DELETE_AND_RELAX: u32 = 0x1_0001;

const EF_RISCV_RVC: u32 = 0x1;
const STB_LOCAL: u8 = 0;
const STB_WEAK: u8 = 2;
const STT_FUNC: u8 = 2;
const STT_SECTION: u8 = 3;
const STT_GNU_IFUNC: u8 = 10;
const SHN_UNDEF: u16 = 0;
const SHN_LORESERVE: u16 = 0xff00;
const SHN_ABS: u16 = 0xfff1;
const SHN_COMMON: u16 = 0xfff2;
const SHF_ALLOC: u64 = 0x2;
const SHT_NOBITS: u32 = 8;
const SHF_MERGE: u64 = 0x10;
const SHF_EXECINSTR: u64 = 0x4;

const X_SP: u32 = 2;
const MATCH_JAL: u32 = 0x6f;
const MATCH_JALR: u32 = 0x67;
const MATCH_C_J: u32 = 0xa001;
const MATCH_C_LUI: u32 = 0x6001;
const RISCV_NOP: u32 = 0x0000_0013;
const RVC_NOP: u16 = 0x0001;
const OP_RD_MASK: u32 = 0x1f << 7;

/// GNU ld's RISC-V `ELF_MAXPAGESIZE` and `ELF_COMMONPAGESIZE`.
const MAXPAGESIZE: u64 = 0x1000;
const COMMONPAGESIZE: u64 = 0x1000;

/// One output section of last trip's layout: what `_bfd_riscv_get_max_alignment`
/// walks.
#[derive(Debug, Clone, Copy)]
pub(super) struct OutSection {
    pub vaddr: u64,
    pub size: u64,
    pub align: u64,
}

/// A resolved global as last trip's layout left it.
#[derive(Debug, Clone)]
pub(super) struct ProbeGlobal {
    /// Final address (a PLT stub for an import that has one).
    pub value: u64,
    pub size: u64,
    pub sym_type: u8,
    pub defined: bool,
    /// Calls and references go through a PLT entry at `value`.
    pub plt: bool,
    /// Undefined weak with no definition anywhere: resolves to 0.
    pub undef_weak: bool,
    /// Defined by the linker itself (`__global_pointer$`, `_end`,
    /// `__start_SEC`...), not by an input object: bfd's `linker_def`.
    pub linker_defined: bool,
    /// A COPY-relocated import: the slot `(vaddr, size)` it lives in.
    pub copy_slot: Option<(u64, u64)>,
    /// Allocated from a COMMON definition.
    pub common: bool,
}

/// Last trip's layout, everything relaxation reads about addresses.
#[derive(Debug, Default)]
pub(super) struct LayoutProbe {
    /// `(obj, sec) -> (merged section, offset in it)`.
    pub sec_mapping: FxHashMap<(usize, usize), (usize, u64)>,
    /// Merged `SHF_MERGE` members -> their representative: a member takes
    /// no space and lives wherever its representative is placed.
    pub aliases: super::sections::MergeAliases,
    /// Per merged section: address, size, alignment.
    pub vaddr: Vec<u64>,
    pub size: Vec<u64>,
    pub align: Vec<u64>,
    /// Merged sections laid out back to back in the read-only executable
    /// segment, in address order: the only place relaxable code lives, and
    /// the only place where a trip moves sections it has not reached yet.
    pub rx_seq: Vec<usize>,
    /// Every allocated merged section in address order, then the
    /// non-allocated ones: the order ld relaxes input sections in.
    pub order: Vec<usize>,
    /// Start of the writable segment (page aligned after the RX end).
    pub rw_start: u64,
    /// Every output section header of the image, synthetic ones included.
    pub out_sections: Vec<OutSection>,
    pub globals: FxHashMap<String, ProbeGlobal>,
    /// `__global_pointer$`, 0 when undefined (bfd: no gp relaxation).
    pub gp: u64,
    /// The merged section an input object defines `__global_pointer$` in;
    /// `None` for the linker's own (absolute) definition.
    pub gp_section: Option<usize>,
    /// Start of the TLS segment, as bfd's `tpoff` measures from it.
    pub tls_start: Option<u64>,
    /// `.plt`: where import calls land.
    pub plt: (u64, u64),
    pub relro: bool,
    pub pic: bool,
}

impl LayoutProbe {
    /// The input section that holds `(obj, sec)`'s bytes: itself, or the
    /// representative of the `SHF_MERGE` group it merged into (bfd's
    /// `sym_sec` after `_bfd_merged_section_offset`).
    pub fn representative(&self, obj: usize, sec: usize) -> (usize, usize) {
        self.aliases.get(&(obj, sec)).copied().unwrap_or((obj, sec))
    }
}

const PAGE: u64 = 0x1000;

fn align_up(v: u64, a: u64) -> Option<u64> {
    let a = a.max(1);
    let quotient = v / a;
    let rounded = if v % a == 0 {
        quotient
    } else {
        quotient.checked_add(1)?
    };
    rounded.checked_mul(a)
}

fn as_signed(v: u64) -> i64 {
    i64::from_ne_bytes(v.to_ne_bytes())
}

fn as_unsigned(v: i64) -> u64 {
    u64::from_ne_bytes(v.to_ne_bytes())
}

/// `VALID_ITYPE_IMM`: a signed 12-bit immediate.
fn valid_i(v: u64) -> bool {
    (-2048..=2047).contains(&as_signed(v))
}

/// `VALID_JTYPE_IMM`: an even signed 21-bit displacement.
fn valid_j(v: u64) -> bool {
    v & 1 == 0 && (-(1 << 20)..(1 << 20)).contains(&as_signed(v))
}

/// `VALID_CJTYPE_IMM`: an even signed 12-bit displacement.
fn valid_cj(v: u64) -> bool {
    v & 1 == 0 && (-2048..=2046).contains(&as_signed(v))
}

/// `VALID_CITYPE_LUI_IMM`: a non-zero `c.lui` value, `sext6 << 12`.
pub(super) fn valid_clui(v: u64) -> bool {
    let page = as_signed(v) >> 12;
    v & 0xfff == 0 && page != 0 && (-32..=31).contains(&page)
}

/// `RISCV_CONST_HIGH_PART`.
pub(super) fn high_part(v: u64) -> u64 {
    v.wrapping_add(0x800) & !0xfff
}

/// What the symbol of a relocation lives in, for the "same output section"
/// and "inside its section" tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SymSec {
    /// An input section `(obj, sec)` placed in merged section `merged`.
    Input {
        obj: usize,
        sec: usize,
        merged: usize,
    },
    /// `.plt`.
    Plt,
    /// A linker-allocated span with no input section of its own: a COPY
    /// slot, or a COMMON symbol's storage (bfd's per-object `COMMON` input
    /// section, which holds the symbol exactly).
    Span { start: u64, size: u64 },
    /// The absolute section (linker-script and `SHN_ABS` symbols).
    Abs,
    /// The undefined section (undefined weak).
    Und,
}

/// The output section of a [`SymSec`], for identity comparisons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutId {
    Merged(usize),
    Plt,
    Span,
    Abs,
    Und,
}

impl SymSec {
    fn out(self) -> OutId {
        match self {
            SymSec::Input { merged, .. } => OutId::Merged(merged),
            SymSec::Plt => OutId::Plt,
            SymSec::Span { .. } => OutId::Span,
            SymSec::Abs => OutId::Abs,
            SymSec::Und => OutId::Und,
        }
    }
}

/// In-trip addresses (module docs, "Addresses during a trip").
struct Tracker<'p> {
    p: &'p LayoutProbe,
    /// Merged sections already entered this trip: their new address.
    vaddr_now: Vec<Option<u64>>,
    /// Inputs already placed this trip: their new offset.
    offset_now: FxHashMap<(usize, usize), u64>,
    /// Size of each merged section once this trip has placed all of it.
    size_now: Vec<Option<u64>>,
    /// Shift of the writable segment once the RX segment is complete.
    rw_delta: Option<i128>,
}

impl<'p> Tracker<'p> {
    fn new(p: &'p LayoutProbe) -> Self {
        let n = p.vaddr.len();
        Tracker {
            p,
            vaddr_now: vec![None; n],
            offset_now: FxHashMap::default(),
            size_now: vec![None; n],
            rw_delta: None,
        }
    }

    fn base(&self, merged: usize) -> u64 {
        self.vaddr_now[merged].unwrap_or(self.p.vaddr[merged])
    }

    /// `sec_addr (sec)`: output address of an input section right now (a
    /// merged member's is its representative's).
    fn input_addr(&self, obj: usize, sec: usize) -> Option<u64> {
        let key = self.p.representative(obj, sec);
        let &(m, off_prev) = self.p.sec_mapping.get(&key)?;
        let off = self.offset_now.get(&key).copied().unwrap_or(off_prev);
        Some(self.base(m) + off)
    }

    /// Enter merged section `m`: fix its address for the rest of the trip.
    fn enter(&mut self, m: usize) -> Result<(), String> {
        if let Some(pos) = self.p.rx_seq.iter().position(|&x| x == m) {
            let v = if pos == 0 {
                self.p.vaddr[m]
            } else {
                let prev = self.p.rx_seq[pos - 1];
                let end = self
                    .base(prev)
                    .checked_add(self.size_now[prev].unwrap_or(self.p.size[prev]))
                    .ok_or_else(|| "RISC-V relaxation: RX section end overflows u64".to_owned())?;
                align_up(end, self.p.align[m]).ok_or_else(|| {
                    "RISC-V relaxation: aligned RX section address overflows u64".to_owned()
                })?
            };
            self.vaddr_now[m] = Some(v);
        } else if self.p.vaddr[m] >= self.p.rw_start && self.p.rw_start != 0 {
            // Every RX section precedes the writable segment in `order`, so
            // its in-trip end is final by the first writable section.
            let delta = match self.rw_delta {
                Some(d) => d,
                None => {
                    let d = if let Some(&last) = self.p.rx_seq.last() {
                        let new_end = self
                            .base(last)
                            .checked_add(self.size_now[last].unwrap_or(self.p.size[last]))
                            .ok_or_else(|| {
                                "RISC-V relaxation: RX section end overflows u64".to_owned()
                            })?;
                        let old_end = self.p.vaddr[last]
                            .checked_add(self.p.size[last])
                            .ok_or_else(|| {
                                "RISC-V relaxation: original RX section end overflows u64"
                                    .to_owned()
                            })?;
                        let new_page = align_up(new_end, PAGE).ok_or_else(|| {
                            "RISC-V relaxation: page-aligned RX end overflows u64".to_owned()
                        })?;
                        let old_page = align_up(old_end, PAGE).ok_or_else(|| {
                            "RISC-V relaxation: original page-aligned RX end overflows u64"
                                .to_owned()
                        })?;
                        i128::from(new_page) - i128::from(old_page)
                    } else {
                        0
                    };
                    self.rw_delta = Some(d);
                    d
                }
            };
            let address = i128::from(self.p.vaddr[m]) + delta;
            let address = u64::try_from(address).map_err(|_| {
                "RISC-V relaxation: translated writable section address is out of range".to_owned()
            })?;
            self.vaddr_now[m] = Some(address);
        } else {
            self.vaddr_now[m] = Some(self.p.vaddr[m]);
        }
        Ok(())
    }
}

/// One `riscv_pcgp_hi_reloc`.
#[derive(Debug, Clone, Copy)]
struct PcgpHi {
    hi_sec_off: u64,
    hi_addend: i64,
    hi_addr: u64,
    hi_sym: u32,
    sym_sec: SymSec,
    undefined_weak: bool,
}

/// State that outlives one section: `htab->max_alignment`,
/// `htab->max_alignment_for_gp`, bfd's static `first_section` and the
/// `sec_flg0` marks.
#[derive(Default)]
struct LinkState {
    max_alignment: Option<u64>,
    max_alignment_for_gp: Option<u64>,
    first_section: Option<(usize, usize)>,
    flg0: FxHashSet<(usize, usize)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Func {
    Call,
    Lui,
    TlsLe,
    Pc,
    Jal,
    Align,
}

/// Whether any input carries relaxation work at all.
pub(super) fn needs_relaxation(objs: &[(String, ElfObject)]) -> bool {
    objs.iter().any(|(_, o)| {
        o.relocations
            .iter()
            .flatten()
            .any(|r| r.rela_type == R_RISCV_RELAX || r.rela_type == R_RISCV_ALIGN)
    })
}

/// Relax `objs` in place.  `probe` lays the objects out as they currently
/// are (a full link up to, not including, relocation) and reports where
/// everything landed.
pub(super) fn relax(
    objs: &mut [(String, ElfObject)],
    probe: &mut dyn FnMut(&[(String, ElfObject)]) -> Result<LayoutProbe, String>,
) -> Result<(), String> {
    let mut st = LinkState::default();
    for pass in 0..2 {
        loop {
            let p = probe(objs)?;
            if !trip(objs, &p, pass, &mut st)? {
                break;
            }
        }
    }
    Ok(())
}

/// Defining object symbol of every global an input object defines, matched
/// against the resolved value so a losing duplicate is never picked.
fn global_definers(
    objs: &[(String, ElfObject)],
    p: &LayoutProbe,
) -> FxHashMap<String, (usize, usize)> {
    let mut defs = FxHashMap::default();
    for (oi, (_, obj)) in objs.iter().enumerate() {
        for (si, sym) in obj.symbols.iter().enumerate() {
            if sym.binding() == STB_LOCAL
                || sym.shndx == SHN_UNDEF
                || sym.shndx >= SHN_LORESERVE
                || sym.name.is_empty()
            {
                continue;
            }
            let Some(g) = p.globals.get(sym.name.as_str()) else {
                continue;
            };
            if !g.defined || g.plt || g.linker_defined || defs.contains_key(sym.name.as_str()) {
                continue;
            }
            let Some(&(m, off)) = p.sec_mapping.get(&(oi, usize::from(sym.shndx))) else {
                continue;
            };
            if p.vaddr[m] + off + sym.value == g.value {
                defs.insert(sym.name.to_string(), (oi, si));
            }
        }
    }
    defs
}

/// The merged sections whose inputs ld walks, each with its inputs in
/// placement order. Merged `SHF_MERGE` members are not among them: ld
/// excluded them from layout, and they resolve through their representative.
fn inputs_by_merged(p: &LayoutProbe) -> Vec<Vec<(usize, usize)>> {
    let mut by: Vec<Vec<(u64, usize, usize)>> = vec![Vec::new(); p.vaddr.len()];
    for (&(o, s), &(m, off)) in &p.sec_mapping {
        if !p.aliases.contains_key(&(o, s)) {
            by[m].push((off, o, s));
        }
    }
    by.into_iter()
        .map(|mut v| {
            v.sort_unstable();
            v.into_iter().map(|(_, o, s)| (o, s)).collect()
        })
        .collect()
}

/// One trip over every input section; returns bfd's `again`.
fn trip(
    objs: &mut [(String, ElfObject)],
    p: &LayoutProbe,
    pass: u32,
    st: &mut LinkState,
) -> Result<bool, String> {
    let defs = global_definers(objs, p);
    let inputs = inputs_by_merged(p);
    let mut tr = Tracker::new(p);
    let mut again = false;
    for &m in &p.order {
        tr.enter(m)?;
        let mut dot = 0u64;
        let mut prev_dot = 0u64;
        for &(o, s) in &inputs[m] {
            let size_prev = objs[o].1.sections[s].size;
            let eligible = !st.flg0.contains(&(o, s))
                && !objs[o].1.relocations.get(s).is_none_or(Vec::is_empty)
                && objs[o].1.sections[s].sh_type != SHT_NOBITS;
            if eligible {
                match st.first_section {
                    None => st.first_section = Some((o, s)),
                    Some(f) if f == (o, s) => st.max_alignment_for_gp = None,
                    Some(_) => {}
                }
                let cx = SectionCx {
                    p,
                    tr: &tr,
                    defs: &defs,
                    obj: o,
                    sec: s,
                    merged: m,
                };
                again |= relax_section(objs, &cx, pass, st)?;
            }
            // Place the input: ld's `size_input_section`.
            let align = objs[o].1.sections[s].addralign.max(1);
            dot = align_up(dot, align).ok_or_else(|| {
                format!("{}: aligned input-section offset overflows u64", objs[o].0)
            })?;
            tr.offset_now.insert((o, s), dot);
            dot = dot
                .checked_add(objs[o].1.sections[s].size)
                .ok_or_else(|| format!("{}: input-section extent overflows u64", objs[o].0))?;
            prev_dot = align_up(prev_dot, align)
                .and_then(|offset| offset.checked_add(size_prev))
                .ok_or_else(|| {
                    format!("{}: previous input-section extent overflows u64", objs[o].0)
                })?;
        }
        // Bytes after the last input (synthetic tails such as COMMON) keep
        // their size; only the input-backed part can have shrunk.
        let tail = p.size[m].saturating_sub(prev_dot);
        tr.size_now[m] =
            Some(dot.checked_add(tail).ok_or_else(|| {
                "RISC-V relaxation: merged section size overflows u64".to_owned()
            })?);
    }
    Ok(again)
}

/// The fixed context of one `_bfd_riscv_relax_section` call.
struct SectionCx<'a> {
    p: &'a LayoutProbe,
    tr: &'a Tracker<'a>,
    defs: &'a FxHashMap<String, (usize, usize)>,
    obj: usize,
    sec: usize,
    merged: usize,
}

impl SectionCx<'_> {
    fn sec_addr(&self, s: SymSec) -> u64 {
        match s {
            SymSec::Input { obj, sec, .. } => self.tr.input_addr(obj, sec).unwrap_or(0),
            SymSec::Plt => self.p.plt.0,
            SymSec::Span { start, .. } => start,
            SymSec::Abs | SymSec::Und => 0,
        }
    }

    fn sec_size(&self, objs: &[(String, ElfObject)], s: SymSec) -> u64 {
        match s {
            SymSec::Input { obj, sec, .. } => objs[obj].1.sections[sec].size,
            SymSec::Plt => self.p.plt.1,
            SymSec::Span { size, .. } => size,
            SymSec::Abs | SymSec::Und => 0,
        }
    }

    fn out_align(&self, o: OutId) -> u64 {
        match o {
            OutId::Merged(m) => self.p.align[m].max(1),
            OutId::Plt => 16,
            OutId::Span | OutId::Abs | OutId::Und => 1,
        }
    }
}

/// Per-section mutable state: the relocations and bytes being rewritten,
/// the section's current size and the pc-relative pairing tables.
struct Work {
    relocs: Vec<crate::backend::linker_common::Elf64Rela>,
    data: Vec<u8>,
    size: u64,
    pcgp_hi: Vec<PcgpHi>,
    pcgp_lo: Vec<u64>,
    touched: bool,
}

/// `_bfd_riscv_relax_section`.  Returns `again`.
fn relax_section(
    objs: &mut [(String, ElfObject)],
    cx: &SectionCx,
    pass: u32,
    st: &mut LinkState,
) -> Result<bool, String> {
    let (o, s) = (cx.obj, cx.sec);
    let max_alignment = *st.max_alignment.get_or_insert_with(|| {
        let power =
            cx.p.out_sections
                .iter()
                .map(|x| x.align.max(1))
                .max()
                .unwrap_or(1);
        power.next_power_of_two()
    });
    let gd_calls = gd_rewritten_calls(&objs[o].1, &objs[o].1.relocations[s], cx.p);
    let mut w = Work {
        relocs: std::mem::take(&mut objs[o].1.relocations[s]),
        data: Vec::new(),
        size: objs[o].1.sections[s].size,
        pcgp_hi: Vec::new(),
        pcgp_lo: Vec::new(),
        touched: false,
    };
    let rvc = objs[o].1.e_flags & EF_RISCV_RVC != 0;
    let mut again = false;
    let mut reserve_size: u64 = 0;
    let result = (|| -> Result<(), String> {
        let mut i = 0;
        while i < w.relocs.len() {
            let ri = i;
            let r = w.relocs[ri].clone();
            i += 1;
            let func = if pass == 0 {
                let f = match r.rela_type {
                    R_RISCV_CALL | R_RISCV_CALL_PLT => Func::Call,
                    R_RISCV_HI20 | R_RISCV_LO12_I | R_RISCV_LO12_S => Func::Lui,
                    R_RISCV_TPREL_HI20 | R_RISCV_TPREL_ADD | R_RISCV_TPREL_LO12_I
                    | R_RISCV_TPREL_LO12_S => Func::TlsLe,
                    R_RISCV_PCREL_HI20 | R_RISCV_PCREL_LO12_I | R_RISCV_PCREL_LO12_S
                        if !cx.p.pic =>
                    {
                        Func::Pc
                    }
                    R_RISCV_JAL => Func::Jal,
                    _ => continue,
                };
                let paired = w
                    .relocs
                    .get(ri + 1)
                    .is_some_and(|n| n.rela_type == R_RISCV_RELAX && n.offset == r.offset);
                if !paired {
                    continue;
                }
                i += 1;
                f
            } else if r.rela_type == R_RISCV_ALIGN {
                Func::Align
            } else {
                continue;
            };
            if func == Func::Call && gd_calls.contains(&ri) {
                continue;
            }
            if w.data.is_empty() && w.size > 0 {
                w.data = objs[o].1.section_data[s].to_owned_vec();
            }
            let obj = &objs[o].1;
            let sym = &obj.symbols[r.sym_idx as usize];
            let mut undefined_weak = false;
            let (sym_sec, symval) = if sym.binding() == STB_LOCAL {
                reserve_size = remaining(sym.size, r.addend);
                if sym.sym_type() == STT_GNU_IFUNC {
                    continue;
                }
                let (ss, v) = if sym.shndx == SHN_UNDEF {
                    (
                        SymSec::Input {
                            obj: o,
                            sec: s,
                            merged: cx.merged,
                        },
                        r.offset,
                    )
                } else if sym.shndx == SHN_ABS {
                    (SymSec::Abs, sym.value)
                } else {
                    let (obj, sec) = cx.p.representative(o, usize::from(sym.shndx));
                    let Some(&(m, _)) = cx.p.sec_mapping.get(&(obj, sec)) else {
                        continue;
                    };
                    (
                        SymSec::Input {
                            obj,
                            sec,
                            merged: m,
                        },
                        sym.value,
                    )
                };
                (ss, v)
            } else {
                let Some(g) = cx.p.globals.get(sym.name.as_str()) else {
                    continue;
                };
                if g.sym_type == STT_GNU_IFUNC {
                    continue;
                }
                if g.undef_weak && !g.linker_defined && matches!(func, Func::Lui | Func::Pc) {
                    undefined_weak = true;
                }
                let (ss, v) = if g.plt {
                    (SymSec::Plt, g.value.wrapping_sub(cx.p.plt.0))
                } else if undefined_weak {
                    (SymSec::Und, 0)
                } else if g.defined {
                    if let Some(&(d_obj, d_sym)) = cx.defs.get(sym.name.as_str()) {
                        let ds = &objs[d_obj].1.symbols[d_sym];
                        let (obj, sec) = cx.p.representative(d_obj, usize::from(ds.shndx));
                        let Some(&(m, _)) = cx.p.sec_mapping.get(&(obj, sec)) else {
                            continue;
                        };
                        (
                            SymSec::Input {
                                obj,
                                sec,
                                merged: m,
                            },
                            ds.value,
                        )
                    } else if let Some((start, size)) = g.copy_slot {
                        (SymSec::Span { start, size }, g.value.wrapping_sub(start))
                    } else if sym.shndx == SHN_COMMON || g.common {
                        (
                            SymSec::Span {
                                start: g.value,
                                size: g.size,
                            },
                            0,
                        )
                    } else {
                        (SymSec::Abs, g.value)
                    }
                } else {
                    continue;
                };
                if g.sym_type != STT_FUNC {
                    reserve_size = remaining(g.size, r.addend);
                }
                (ss, v)
            };
            // bfd routes SEC_MERGE symbols through `_bfd_merged_section_offset`,
            // which also makes the group's representative `sym_sec`.
            // `sections::merge_alloc_sections` already rewrote symbol values
            // and section-symbol addends to pool offsets, and `representative`
            // picked the same `sym_sec`, so the plain sum below is that address
            // and `gp_margin` sizes the right section.
            let symval = symval
                .wrapping_add(as_unsigned(r.addend))
                .wrapping_add(cx.sec_addr(sym_sec));
            w.touched = true;
            let a = Cand {
                ri,
                sym_sec,
                symval,
                max_alignment,
                reserve_size,
                undefined_weak,
                rvc,
            };
            match func {
                Func::Call => again |= relax_call(cx, &mut w, &a),
                Func::Jal => again |= relax_jal(cx, &mut w, &a),
                Func::Lui => again |= relax_lui(objs, cx, &mut w, &a, st),
                Func::TlsLe => again |= relax_tls_le(cx, &mut w, &a),
                Func::Pc => again |= relax_pc(objs, cx, &mut w, &a, st),
                Func::Align => relax_align(objs, cx, &mut w, &a, st)?,
            }
        }
        resolve_delete_relocs(objs, o, s, &mut w);
        Ok(())
    })();
    if w.touched && !w.data.is_empty() {
        w.data.truncate(w.size as usize);
        objs[o].1.section_data[s] = SectionData::owned(std::mem::take(&mut w.data));
    }
    objs[o].1.sections[s].size = w.size;
    objs[o].1.relocations[s] = w.relocs;
    result.map(|()| again)
}

/// bfd's `reserve_size`: what is left of the object after the addend,
/// 0 when the addend points outside it (unsigned wrap test, as bfd).
fn remaining(size: u64, addend: i64) -> u64 {
    let r = size.wrapping_sub(as_unsigned(addend));
    if r > size { 0 } else { r }
}

/// Indices of `CALL_PLT __tls_get_addr` relocations lccc-ld's GD->LE rewrite
/// will consume: the same forward scan `reloc::collect_gd_tls_relax_info`
/// makes, restricted to accesses whose variable resolves in the image.
fn gd_rewritten_calls(
    obj: &ElfObject,
    relocs: &[crate::backend::linker_common::Elf64Rela],
    p: &LayoutProbe,
) -> FxHashSet<usize> {
    let mut out = FxHashSet::default();
    for (ri, r) in relocs.iter().enumerate() {
        if r.rela_type != R_RISCV_TLS_GD_HI20 {
            continue;
        }
        let Some(sym) = obj.symbols.get(r.sym_idx as usize) else {
            continue;
        };
        let local = sym.binding() == STB_LOCAL
            || p.globals
                .get(sym.name.as_str())
                .is_some_and(|g| g.defined && !g.plt);
        if !local {
            continue;
        }
        for (j, c) in relocs
            .iter()
            .enumerate()
            .take(relocs.len().min(ri + 8))
            .skip(ri + 1)
        {
            if c.rela_type == R_RISCV_CALL_PLT
                && obj
                    .symbols
                    .get(c.sym_idx as usize)
                    .is_some_and(|cs| cs.name == "__tls_get_addr")
            {
                out.insert(j);
                break;
            }
        }
    }
    out
}

/// The per-candidate arguments of bfd's `relax_func_t`.
struct Cand {
    ri: usize,
    sym_sec: SymSec,
    symval: u64,
    max_alignment: u64,
    reserve_size: u64,
    undefined_weak: bool,
    rvc: bool,
}

fn get32(d: &[u8], off: u64) -> u32 {
    let o = off as usize;
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

fn put32(d: &mut [u8], off: u64, v: u32) {
    let o = off as usize;
    d[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

fn put16(d: &mut [u8], off: u64, v: u16) {
    let o = off as usize;
    d[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

/// `_riscv_relax_delete_piecewise`: turn relocation `at` into a deferred
/// deletion of `count` bytes at `addr`.
fn delete_piecewise(w: &mut Work, at: usize, addr: u64, count: u64, preserve_relax: bool) {
    let r = &mut w.relocs[at];
    r.rela_type = if preserve_relax {
        R_DELETE_AND_RELAX
    } else {
        R_DELETE
    };
    r.sym_idx = 0;
    r.offset = addr;
    r.addend = as_signed(count);
}

/// `_bfd_riscv_relax_call`.
fn relax_call(cx: &SectionCx, w: &mut Work, a: &Cand) -> bool {
    let r_off = w.relocs[a.ri].offset;
    let pc = cx.sec_addr(SymSec::Input {
        obj: cx.obj,
        sec: cx.sec,
        merged: cx.merged,
    }) + r_off;
    let mut foff = a.symval.wrapping_sub(pc);
    let near_zero = a.symval.wrapping_add(2048) < 4096;
    let mut max_alignment = a.max_alignment;
    if valid_j(foff) {
        if a.sym_sec.out() == OutId::Merged(cx.merged) && a.sym_sec.out() != OutId::Abs {
            max_alignment = cx.out_align(a.sym_sec.out());
        }
        foff = if as_signed(foff) < 0 {
            foff.wrapping_sub(max_alignment)
        } else {
            foff.wrapping_add(max_alignment)
        };
    }
    if !valid_j(foff) && !(!cx.p.pic && near_zero) {
        return false;
    }
    let jalr = get32(&w.data, r_off + 4);
    let rd = (jalr >> 7) & 0x1f;
    let (r_type, insn, len) = if a.rvc && valid_cj(foff) && rd == 0 {
        (R_RISCV_RVC_JUMP, MATCH_C_J, 2)
    } else if valid_j(foff) {
        (R_RISCV_JAL, MATCH_JAL | (rd << 7), 4)
    } else {
        (R_RISCV_LO12_I, MATCH_JALR | (rd << 7), 4)
    };
    w.relocs[a.ri].rela_type = r_type;
    if len == 2 {
        put16(&mut w.data, r_off, insn as u16);
    } else {
        put32(&mut w.data, r_off, insn);
    }
    delete_piecewise(w, a.ri + 1, r_off + len, 8 - len, r_type == R_RISCV_JAL);
    true
}

/// `_bfd_riscv_relax_jal`.
fn relax_jal(cx: &SectionCx, w: &mut Work, a: &Cand) -> bool {
    if !a.rvc {
        return false;
    }
    let r_off = w.relocs[a.ri].offset;
    let pc = cx.sec_addr(SymSec::Input {
        obj: cx.obj,
        sec: cx.sec,
        merged: cx.merged,
    }) + r_off;
    let mut foff = a.symval.wrapping_sub(pc);
    let jal = get32(&w.data, r_off);
    if (jal >> 7) & 0x1f != 0 {
        return false;
    }
    let mut max_alignment = a.max_alignment;
    if valid_cj(foff) {
        if a.sym_sec.out() == OutId::Merged(cx.merged) && a.sym_sec.out() != OutId::Abs {
            max_alignment = cx.out_align(a.sym_sec.out());
        }
        foff = if as_signed(foff) < 0 {
            foff.wrapping_sub(max_alignment)
        } else {
            foff.wrapping_add(max_alignment)
        };
    }
    if !valid_cj(foff) {
        return false;
    }
    w.relocs[a.ri].rela_type = R_RISCV_RVC_JUMP;
    put16(&mut w.data, r_off, MATCH_C_J as u16);
    delete_piecewise(w, a.ri + 1, r_off + 2, 2, false);
    true
}

/// `_bfd_riscv_get_max_alignment` over last trip's output sections.
fn max_alignment_near(p: &LayoutProbe, gp: u64) -> u64 {
    let mut best = 1u64;
    for o in &p.out_sections {
        let valid = gp == 0
            || valid_i(o.vaddr.wrapping_sub(gp))
            || valid_i(o.vaddr.wrapping_add(o.size).wrapping_sub(gp));
        if valid {
            best = best.max(o.align.max(1).next_power_of_two());
        }
    }
    best
}

/// The gp-relative margin shared by `relax_lui` and `relax_pc`.
fn gp_margin(
    objs: &[(String, ElfObject)],
    cx: &SectionCx,
    st: &mut LinkState,
    sym_sec: SymSec,
    symval: u64,
) -> u64 {
    let gp = cx.p.gp;
    let dsa = if cx.p.relro {
        MAXPAGESIZE + COMMONPAGESIZE
    } else {
        MAXPAGESIZE
    };
    let gp_out = cx.p.gp_section.map_or(OutId::Abs, OutId::Merged);
    let mut max_alignment = if gp_out == sym_sec.out() && sym_sec.out() != OutId::Abs {
        cx.out_align(sym_sec.out())
    } else {
        *st.max_alignment_for_gp
            .get_or_insert_with(|| max_alignment_near(cx.p, gp))
    };
    let lo = cx.sec_addr(sym_sec);
    if symval < lo || symval > lo + cx.sec_size(objs, sym_sec) {
        max_alignment = max_alignment.max(dsa);
    }
    max_alignment
}

/// The gp/x0 reachability test of `relax_lui` and `relax_pc`.
fn gp_reachable(symval: u64, gp: u64, max_alignment: u64, reserve: u64, weak: bool) -> bool {
    weak || valid_i(symval)
        || (symval >= gp
            && valid_i(
                symval
                    .wrapping_sub(gp)
                    .wrapping_add(max_alignment)
                    .wrapping_add(reserve),
            ))
        || (symval < gp
            && valid_i(
                symval
                    .wrapping_sub(gp)
                    .wrapping_sub(max_alignment)
                    .wrapping_sub(reserve),
            ))
}

/// `_bfd_riscv_relax_lui`.  Returns `again`.
fn relax_lui(
    objs: &[(String, ElfObject)],
    cx: &SectionCx,
    w: &mut Work,
    a: &Cand,
    st: &mut LinkState,
) -> bool {
    let gp = cx.p.gp;
    let dsa = if cx.p.relro {
        MAXPAGESIZE + COMMONPAGESIZE
    } else {
        MAXPAGESIZE
    };
    let mut max_alignment = a.max_alignment;
    if !a.undefined_weak && gp != 0 {
        max_alignment = gp_margin(objs, cx, st, a.sym_sec, a.symval);
    }
    let r_type = w.relocs[a.ri].rela_type;
    let r_off = w.relocs[a.ri].offset;
    if gp_reachable(
        a.symval,
        gp,
        max_alignment,
        a.reserve_size,
        a.undefined_weak,
    ) {
        match r_type {
            R_RISCV_LO12_I => w.relocs[a.ri].rela_type = R_RISCV_GPREL_I,
            R_RISCV_LO12_S => w.relocs[a.ri].rela_type = R_RISCV_GPREL_S,
            _ => {
                delete_piecewise(w, a.ri, r_off, 4, false);
                return true;
            }
        }
        return false;
    }
    if a.rvc
        && r_type == R_RISCV_HI20
        && valid_clui(high_part(a.symval))
        && valid_clui(high_part(a.symval).wrapping_add(dsa))
    {
        let lui = get32(&w.data, r_off);
        let rd = (lui >> 7) & 0x1f;
        if rd == 0 || rd == X_SP {
            return false;
        }
        put32(&mut w.data, r_off, (lui & OP_RD_MASK) | MATCH_C_LUI);
        w.relocs[a.ri].rela_type = R_RISCV_RVC_LUI;
        delete_piecewise(w, a.ri + 1, r_off + 2, 2, false);
        return true;
    }
    false
}

/// `_bfd_riscv_relax_tls_le`.
fn relax_tls_le(cx: &SectionCx, w: &mut Work, a: &Cand) -> bool {
    let tpoff = cx.p.tls_start.map_or(0, |t| a.symval.wrapping_sub(t));
    if high_part(tpoff) != 0 {
        return false;
    }
    let r_off = w.relocs[a.ri].offset;
    match w.relocs[a.ri].rela_type {
        R_RISCV_TPREL_LO12_I => w.relocs[a.ri].rela_type = R_RISCV_TPREL_I,
        R_RISCV_TPREL_LO12_S => w.relocs[a.ri].rela_type = R_RISCV_TPREL_S,
        _ => {
            delete_piecewise(w, a.ri, r_off, 4, false);
            return true;
        }
    }
    false
}

/// `_bfd_riscv_relax_pc`.
fn relax_pc(
    objs: &[(String, ElfObject)],
    cx: &SectionCx,
    w: &mut Work,
    a: &Cand,
    st: &mut LinkState,
) -> bool {
    let gp = cx.p.gp;
    let r = w.relocs[a.ri].clone();
    let mut symval = a.symval;
    let mut sym_sec = a.sym_sec;
    let mut undefined_weak = a.undefined_weak;
    let mut hi = PcgpHi {
        hi_sec_off: 0,
        hi_addend: 0,
        hi_addr: 0,
        hi_sym: 0,
        sym_sec: SymSec::Abs,
        undefined_weak: false,
    };
    match r.rela_type {
        R_RISCV_PCREL_LO12_I | R_RISCV_PCREL_LO12_S => {
            let hi_sec_off = symval
                .wrapping_sub(cx.sec_addr(sym_sec))
                .wrapping_sub(as_unsigned(r.addend));
            let Some(found) = w
                .pcgp_hi
                .iter()
                .find(|h| h.hi_sec_off == hi_sec_off)
                .copied()
            else {
                w.pcgp_lo.push(hi_sec_off);
                return false;
            };
            hi = found;
            symval = hi.hi_addr;
            sym_sec = hi.sym_sec;
            undefined_weak = hi.undefined_weak;
        }
        _ => {
            let flags = match sym_sec {
                SymSec::Input { obj, sec, .. } => objs[obj].1.sections[sec].flags,
                SymSec::Plt => SHF_EXECINSTR,
                _ => 0,
            };
            if !undefined_weak && flags & (SHF_MERGE | SHF_EXECINSTR) != 0 {
                return false;
            }
            if w.pcgp_lo.contains(&r.offset) {
                return false;
            }
        }
    }
    let mut max_alignment = a.max_alignment;
    if !undefined_weak && gp != 0 {
        max_alignment = gp_margin(objs, cx, st, sym_sec, symval);
    }
    if !gp_reachable(symval, gp, max_alignment, a.reserve_size, undefined_weak) {
        return false;
    }
    match r.rela_type {
        R_RISCV_PCREL_LO12_I | R_RISCV_PCREL_LO12_S => {
            let rel = &mut w.relocs[a.ri];
            rel.rela_type = if r.rela_type == R_RISCV_PCREL_LO12_I {
                R_RISCV_GPREL_I
            } else {
                R_RISCV_GPREL_S
            };
            rel.sym_idx = hi.hi_sym;
            rel.addend = rel.addend.wrapping_add(hi.hi_addend);
            false
        }
        _ => {
            w.pcgp_hi.push(PcgpHi {
                hi_sec_off: r.offset,
                hi_addend: r.addend,
                hi_addr: symval,
                hi_sym: r.sym_idx,
                sym_sec,
                undefined_weak,
            });
            delete_piecewise(w, a.ri, r.offset, 4, false);
            true
        }
    }
}

/// `_bfd_riscv_relax_align`.
fn relax_align(
    objs: &mut [(String, ElfObject)],
    cx: &SectionCx,
    w: &mut Work,
    a: &Cand,
    st: &mut LinkState,
) -> Result<(), String> {
    let r = w.relocs[a.ri].clone();
    let addend = as_unsigned(r.addend);
    let mut alignment: u64 = 1;
    while alignment <= addend {
        alignment = alignment.checked_mul(2).ok_or_else(|| {
            format!(
                "{}({}+{:#x}): R_RISCV_ALIGN addend {} requests no representable alignment",
                objs[cx.obj].0, objs[cx.obj].1.sections[cx.sec].name, r.offset, r.addend
            )
        })?;
    }
    let symval = a.symval.wrapping_sub(addend);
    let aligned = (symval.wrapping_sub(1) & !(alignment - 1)).wrapping_add(alignment);
    let nop_bytes = aligned.wrapping_sub(symval);
    st.flg0.insert((cx.obj, cx.sec));
    if addend < nop_bytes {
        return Err(format!(
            "{}({}+{:#x}): {} bytes required for alignment to {}-byte boundary, but only {} present",
            objs[cx.obj].0,
            objs[cx.obj].1.sections[cx.sec].name,
            r.offset,
            nop_bytes,
            alignment,
            addend
        ));
    }
    w.relocs[a.ri].rela_type = R_RISCV_NONE;
    w.relocs[a.ri].sym_idx = 0;
    if nop_bytes == addend {
        return Ok(());
    }
    let mut pos = 0;
    while pos < nop_bytes & !3 {
        put32(&mut w.data, r.offset + pos, RISCV_NOP);
        pos += 4;
    }
    if nop_bytes % 4 != 0 {
        put16(&mut w.data, r.offset + pos, RVC_NOP);
    }
    let size = w.size;
    delete_bytes(
        objs,
        cx.obj,
        cx.sec,
        w,
        Deletion {
            addr: r.offset + nop_bytes,
            count: addend - nop_bytes,
            delete_total: 0,
            toaddr: size,
        },
    );
    Ok(())
}

/// `riscv_relax_resolve_delete_relocs`: carry out the deferred deletions of
/// one section, each against the section as the earlier ones left it.
fn resolve_delete_relocs(objs: &mut [(String, ElfObject)], o: usize, s: usize, w: &mut Work) {
    let is_delete = |t: u32| t == R_DELETE || t == R_DELETE_AND_RELAX;
    let mut delete_total: u64 = 0;
    let n = w.relocs.len();
    let mut i = 0;
    while i < n {
        if !is_delete(w.relocs[i].rela_type) {
            i += 1;
            continue;
        }
        let this_off = w.relocs[i].offset;
        let next =
            (i..n).find(|&j| is_delete(w.relocs[j].rela_type) && w.relocs[j].offset > this_off);
        let toaddr = next.map_or(w.size, |j| w.relocs[j].offset);
        let count = as_unsigned(w.relocs[i].addend);
        delete_bytes(
            objs,
            o,
            s,
            w,
            Deletion {
                addr: this_off,
                count,
                delete_total,
                toaddr,
            },
        );
        delete_total += count;
        let rel = &mut w.relocs[i];
        if rel.rela_type == R_DELETE_AND_RELAX {
            rel.rela_type = R_RISCV_RELAX;
            rel.offset -= count;
        } else {
            rel.rela_type = R_RISCV_NONE;
        }
        rel.addend = 0;
        rel.sym_idx = 0;
        i = next.unwrap_or(n);
    }
}

/// One `_riscv_relax_delete_bytes` request, in current coordinates: delete
/// `count` bytes at `addr`; `delete_total` bytes were already deleted before
/// `addr` by this batch, whose memmove has not yet reached `toaddr`.
#[derive(Clone, Copy)]
struct Deletion {
    addr: u64,
    count: u64,
    delete_total: u64,
    toaddr: u64,
}

/// `_riscv_relax_delete_bytes`: carry out `d` in section `s` of object `o`.
fn delete_bytes(objs: &mut [(String, ElfObject)], o: usize, s: usize, w: &mut Work, d: Deletion) {
    let Deletion {
        addr,
        count,
        delete_total,
        toaddr,
    } = d;
    let bytes_to_move = toaddr - addr - count;
    w.size -= count;
    let src = (addr + count + delete_total) as usize;
    w.data
        .copy_within(src..src + bytes_to_move as usize, addr as usize);
    let toaddr = w.size + count;
    for r in &mut w.relocs {
        if r.offset > addr && r.offset < toaddr {
            r.offset -= count;
        }
    }
    let obj = &mut objs[o].1;
    let shndx = s as u64;
    let mut section_syms: Vec<usize> = Vec::new();
    for (k, sym) in obj.symbols.iter_mut().enumerate() {
        if u64::from(sym.shndx) != shndx {
            continue;
        }
        if sym.sym_type() == STT_SECTION {
            section_syms.push(k);
        }
        if sym.value > addr && sym.value <= toaddr {
            sym.value -= count;
        } else if sym.value <= addr && sym.value + sym.size > addr && sym.value + sym.size <= toaddr
        {
            sym.size -= count;
        }
    }
    if section_syms.is_empty() {
        return;
    }
    // Byte references through the section symbol (module docs).
    let map = |x: u64| {
        if x <= addr {
            x
        } else if x >= addr + count {
            x - count
        } else {
            addr
        }
    };
    let fix = |r: &mut crate::backend::linker_common::Elf64Rela| {
        if section_syms.contains(&(r.sym_idx as usize)) && r.addend > 0 {
            r.addend = as_signed(map(as_unsigned(r.addend)));
        }
    };
    for r in &mut w.relocs {
        fix(r);
    }
    for (k, relocs) in obj.relocations.iter_mut().enumerate() {
        if k != s {
            relocs.iter_mut().for_each(fix);
        }
    }
}

/// What [`fill_relax_probe`] reads from one laid-out image.
pub(super) struct RelaxProbeLayout<'a> {
    pub input_objs: &'a [(String, ElfObject)],
    pub merged_sections: &'a [MergedSection],
    pub section_vaddrs: &'a [u64],
    pub layout_order: &'a [usize],
    /// The read-only executable segment's merged sections, in order, laid
    /// back to back (`LayoutProbe::rx_seq`).
    pub rx_seq: Vec<usize>,
    /// Shared-library output: the PLT entry of each import call.  An
    /// executable records the entry as the import's value instead.
    pub plt_addrs: Option<&'a FxHashMap<String, u64>>,
    pub rw_start: u64,
    /// `(vaddr, size, alignment)` of every linker-generated section.
    pub synthetic: &'a [(u64, u64, u64)],
    pub global_syms: &'a FxHashMap<String, GlobalSym>,
    pub copy_sym_addrs: &'a FxHashMap<String, (u64, u64)>,
    pub tls_start: Option<u64>,
    pub plt: (u64, u64),
}

/// Record a laid-out image for relaxation.  The caller has already set
/// `sec_mapping`, `relro` and `pic`, which the layout does not know.
pub(super) fn fill_relax_probe(probe: &mut LayoutProbe, l: &RelaxProbeLayout) {
    let n = l.merged_sections.len();
    probe.vaddr = l.section_vaddrs.to_vec();
    probe.size = l
        .merged_sections
        .iter()
        .map(|m| m.data.len() as u64)
        .collect();
    probe.align = l.merged_sections.iter().map(|m| m.align.max(1)).collect();
    probe.rx_seq = l.rx_seq.clone();
    let rank: FxHashMap<usize, usize> = l
        .layout_order
        .iter()
        .enumerate()
        .map(|(r, &i)| (i, r))
        .collect();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| {
        let alloc = l.merged_sections[i].sh_flags & SHF_ALLOC != 0;
        (
            !alloc,
            if alloc { l.section_vaddrs[i] } else { 0 },
            rank.get(&i).copied().unwrap_or(n),
        )
    });
    probe.order = order;
    probe.rw_start = l.rw_start;
    probe.out_sections = (0..n)
        .map(|i| OutSection {
            vaddr: l.section_vaddrs[i],
            size: probe.size[i],
            align: probe.align[i],
        })
        .chain(
            l.synthetic
                .iter()
                .filter(|&&(_, size, _)| size != 0)
                .map(|&(vaddr, size, align)| OutSection { vaddr, size, align }),
        )
        .collect();
    // Which globals an input object defines, and how.
    let mut in_section: FxHashSet<&str> = FxHashSet::default();
    let mut common: FxHashSet<&str> = FxHashSet::default();
    // Globals an object defines as STT_GNU_IFUNC.  A static link retypes
    // them STT_FUNC at their IPLT stubs before this probe is taken
    // (`emit_exec`), but bfd decides on the symbol's own type: it never
    // relaxes a reference to an ifunc (`_bfd_riscv_relax_section`).
    let mut ifunc: FxHashSet<&str> = FxHashSet::default();
    for (_, obj) in l.input_objs {
        for sym in &obj.symbols {
            if sym.binding() == STB_LOCAL || sym.shndx == SHN_UNDEF || sym.name.is_empty() {
                continue;
            }
            if sym.shndx == SHN_COMMON {
                common.insert(sym.name.as_str());
            } else {
                in_section.insert(sym.name.as_str());
                if sym.sym_type() == STT_GNU_IFUNC {
                    ifunc.insert(sym.name.as_str());
                }
            }
        }
    }
    probe.globals = l
        .global_syms
        .iter()
        .map(|(name, g)| {
            let object_defined = in_section.contains(name.as_str());
            let is_common = !object_defined && common.contains(name.as_str());
            let plt_entry = l.plt_addrs.and_then(|m| m.get(name).copied());
            let pg = ProbeGlobal {
                value: plt_entry.unwrap_or(g.value),
                size: g.size,
                sym_type: if ifunc.contains(name.as_str()) {
                    STT_GNU_IFUNC
                } else {
                    g.sym_type
                },
                defined: g.defined,
                plt: plt_entry.is_some() || (l.plt_addrs.is_none() && g.needs_plt),
                undef_weak: !g.defined && g.binding == STB_WEAK,
                linker_defined: g.defined && !object_defined && !is_common,
                copy_slot: l.copy_sym_addrs.get(name).copied(),
                common: is_common,
            };
            (name.clone(), pg)
        })
        .collect();
    let gp = l.global_syms.get("__global_pointer$").filter(|g| g.defined);
    probe.gp = gp.map_or(0, |g| g.value);
    probe.gp_section = gp
        .filter(|_| in_section.contains("__global_pointer$"))
        .and_then(|g| g.section_idx);
    probe.tls_start = l.tls_start;
    probe.plt = l.plt;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align_up_handles_arbitrary_alignment_and_rejects_overflow() {
        assert_eq!(align_up(0, 0), Some(0));
        assert_eq!(align_up(10, 1), Some(10));
        assert_eq!(align_up(10, 3), Some(12));
        assert_eq!(align_up(12, 3), Some(12));
        assert_eq!(align_up(u64::MAX - 1, u64::MAX), Some(u64::MAX));
        assert_eq!(align_up(u64::MAX, 2), None);
        assert_eq!(align_up(u64::MAX, u64::MAX - 1), None);
    }

    #[test]
    fn immediate_ranges_match_the_bfd_macros() {
        assert!(valid_i(2047) && valid_i(as_unsigned(-2048)));
        assert!(!valid_i(2048) && !valid_i(as_unsigned(-2049)));
        assert!(valid_j((1 << 20) - 2) && valid_j(as_unsigned(-(1 << 20))));
        assert!(!valid_j(1 << 20) && !valid_j(3));
        assert!(valid_cj(2046) && valid_cj(as_unsigned(-2048)));
        assert!(!valid_cj(2048) && !valid_cj(1));
        assert!(valid_clui(31 << 12) && valid_clui(as_unsigned(-32 << 12)));
        assert!(!valid_clui(0) && !valid_clui(32 << 12) && !valid_clui(0x1001));
        assert_eq!(high_part(0x7ff), 0);
        assert_eq!(high_part(0x800), 0x1000);
        assert_eq!(high_part(as_unsigned(-0x800)), 0);
        assert_eq!(high_part(as_unsigned(-0x801)), as_unsigned(-0x1000));
    }

    #[test]
    fn reserve_size_wraps_like_bfd() {
        assert_eq!(remaining(16, 4), 12);
        assert_eq!(remaining(16, 16), 0);
        assert_eq!(remaining(16, 17), 0);
        assert_eq!(remaining(16, -4), 0);
        assert_eq!(remaining(0, 0), 0);
    }

    #[test]
    fn max_alignment_near_gp_only_counts_sections_within_reach() {
        let p = LayoutProbe {
            out_sections: vec![
                OutSection {
                    vaddr: 0x10000,
                    size: 0x100,
                    align: 64,
                },
                OutSection {
                    vaddr: 0x11000,
                    size: 0x10,
                    align: 8,
                },
                OutSection {
                    vaddr: 0x11800,
                    size: 0x10,
                    align: 16,
                },
            ],
            ..LayoutProbe::default()
        };
        assert_eq!(max_alignment_near(&p, 0x11800), 16);
        assert_eq!(max_alignment_near(&p, 0), 64);
    }
}
