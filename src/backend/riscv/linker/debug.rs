//! DWARF sections, the section-header table and `.symtab` in RISC-V links.
//!
//! Debug sections are never loaded. They follow the last byte of the image
//! with address 0, and their relocations are resolved statically -- never
//! through the GOT or PLT and never as dynamic relocations, even in a shared
//! object, because nothing reads them at run time (bfd's
//! `riscv_elf_relocate_section` for sections without `SEC_ALLOC`; the same
//! design as `arm/linker/debug.rs` and `i686/linker/debug.rs`).
//!
//! RISC-V objects describe addresses in relaxable code with label
//! differences -- ADD/SUB pairs in `.debug_line` and `.debug_info`, SET/SUB
//! ULEB128 pairs in the DWARF 5 range and location lists -- rather than
//! assembled constants. Those resolve exactly here because symbol values are
//! final: `R_RISCV_ALIGN` trimming, the only size-changing step this linker
//! performs, runs once per object straight after parsing and remaps every
//! symbol and every addend that names trimmed code (`align.rs`).

use super::elf_read::{
    ElfObject, SHF_ALLOC, SHF_WRITE, SHN_ABS, SHN_COMMON, SHN_UNDEF, STB_LOCAL, STT_SECTION,
};
use super::relocations::{
    GlobalSym, R_RISCV_32, R_RISCV_32_PCREL, R_RISCV_64, R_RISCV_ADD8, R_RISCV_ADD16,
    R_RISCV_ADD32, R_RISCV_ADD64, R_RISCV_SET_ULEB128, R_RISCV_SET6, R_RISCV_SET8, R_RISCV_SET16,
    R_RISCV_SET32, R_RISCV_SUB_ULEB128, R_RISCV_SUB6, R_RISCV_SUB8, R_RISCV_SUB16, R_RISCV_SUB32,
    R_RISCV_SUB64,
};
use crate::backend::elf::{SHF_MERGE, SHF_STRINGS};
use crate::backend::linker_common::shdr::{ShdrSpec, SymtabEntry};
use crate::backend::linker_common::strmerge_gnu::{self, MergeInput, OffsetMap, Placement};
use crate::backend::linker_common::symtab::{SymtabBuilder, include_global, include_local};
use crate::backend::linker_common::{self, OutputSection};
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use std::rc::Rc;

type SectionMap = FxHashMap<(usize, usize), (usize, u64)>;

/// The debug sections a link keeps, merged by exact name (DWARF names its
/// cross-section references by section), and where each input landed.
pub(super) struct DebugSections {
    outputs: Vec<OutputSection>,
    map: SectionMap,
    /// `SHF_MERGE` inputs GNU ld merged (`strmerge_gnu`): their offset map;
    /// `map` places each at its group representative.
    merged: FxHashMap<(usize, usize), Rc<OffsetMap>>,
    /// The merged contents each group representative is laid out with.
    pools: FxHashMap<(usize, usize), Vec<u8>>,
    /// Each output's `sh_entsize`.
    entsizes: Vec<u64>,
}

impl DebugSections {
    /// The debug sections of `input_objs` (none under `-s`/`-S`, which drop
    /// DWARF without a trace: `keep_debug` false) and `.comment`, which GNU
    /// ld keeps even when stripping (`keep_comment`).
    pub(super) fn collect(
        input_objs: &[(String, ElfObject)],
        keep_debug: bool,
        keep_comment: bool,
    ) -> Result<Self, String> {
        let mut outputs = Vec::new();
        let mut map = SectionMap::default();
        linker_common::merge_nonalloc_sections_elf64(
            input_objs.iter().map(|(_, o)| o),
            &mut outputs,
            &mut map,
            &FxHashSet::default(),
            |name| {
                (keep_comment && name == ".comment")
                    || (keep_debug && linker_common::is_debug_section_name(name))
            },
        )?;
        let mut debug = DebugSections {
            outputs,
            map,
            merged: FxHashMap::default(),
            pools: FxHashMap::default(),
            entsizes: Vec::new(),
        };
        for out_idx in 0..debug.outputs.len() {
            debug.merge_strings(input_objs, out_idx);
        }
        debug.entsizes = debug
            .outputs
            .iter_mut()
            .map(|osec| merged_output_flags(osec, input_objs))
            .collect();
        Ok(debug)
    }

    /// GNU ld's `SHF_MERGE` merging of one output section's inputs
    /// (`.debug_str`, `.debug_line_str`, ...): merged groups live in their
    /// first member, the other members are excluded from layout.
    fn merge_strings(&mut self, input_objs: &[(String, ElfObject)], out_idx: usize) {
        let osec = &self.outputs[out_idx];
        let inputs: Vec<MergeInput> = osec
            .inputs
            .iter()
            .map(|i| {
                let obj = &input_objs[i.object_idx].1;
                let sec = &obj.sections[i.section_idx];
                MergeInput {
                    data: obj.section_data[i.section_idx].as_slice(),
                    merge: sec.flags & SHF_MERGE != 0,
                    strings: sec.flags & SHF_STRINGS != 0,
                    entsize: sec.entsize,
                    addralign: sec.addralign,
                    has_relocs: obj
                        .relocations
                        .get(i.section_idx)
                        .is_some_and(|r| !r.is_empty()),
                }
            })
            .collect();
        let placements = strmerge_gnu::merge(&inputs);
        if placements.iter().all(|p| matches!(p, Placement::Raw)) {
            return;
        }
        let old = std::mem::take(&mut self.outputs[out_idx].inputs);
        let mut placed = vec![0u64; old.len()];
        let mut end = 0u64;
        let mut kept = Vec::with_capacity(old.len());
        for (k, (mut input, placement)) in old.into_iter().zip(placements).enumerate() {
            let key = (input.object_idx, input.section_idx);
            let align = input_objs[key.0].1.sections[key.1].addralign.max(1);
            match placement {
                Placement::Merged {
                    repr,
                    pool: None,
                    map,
                } => {
                    self.map.insert(key, (out_idx, placed[repr]));
                    self.merged.insert(key, map);
                    continue;
                }
                Placement::Merged {
                    pool: Some(pool),
                    map,
                    ..
                } => {
                    input.size = pool.len() as u64;
                    self.pools.insert(key, pool);
                    self.merged.insert(key, map);
                }
                Placement::Raw => {}
            }
            let at = end.div_ceil(align) * align;
            placed[k] = at;
            input.output_offset = at;
            end = at + input.size;
            self.map.insert(key, (out_idx, at));
            kept.push(input);
        }
        let osec = &mut self.outputs[out_idx];
        osec.inputs = kept;
        osec.mem_size = end;
    }

    /// Section headers for the placed debug sections, in output order.
    pub(super) fn headers(&self) -> Vec<ShdrSpec> {
        self.outputs
            .iter()
            .zip(&self.entsizes)
            .map(|(s, &entsize)| ShdrSpec {
                align: s.alignment.max(1),
                entsize,
                ..ShdrSpec::new(&s.name, s.sh_type, s.flags, 0, s.file_offset, s.mem_size)
            })
            .collect()
    }
}

/// The output-section flags GNU ld's merge rule decides.
pub(super) const MERGE_FLAGS: u64 = SHF_MERGE | SHF_STRINGS;

/// GNU ld's rule for an output's `SHF_MERGE`/`SHF_STRINGS` (`ldlang.c`
/// `lang_add_section`), over the inputs' `(sh_flags, sh_entsize)` in link
/// order: kept only while every input agrees on both flags and, when merging,
/// on the entry size; the header then states that entry size
/// (`elf_fake_sections`). Returns the kept [`MERGE_FLAGS`] bits and the
/// output's `sh_entsize`, `(0, 0)` on disagreement.
pub(super) fn gnu_merge_flags(mut inputs: impl Iterator<Item = (u64, u64)>) -> (u64, u64) {
    let Some((first_flags, entsize)) = inputs.next() else {
        return (0, 0);
    };
    let flags = first_flags & MERGE_FLAGS;
    let agree =
        inputs.all(|(f, e)| f & MERGE_FLAGS == flags && (flags & SHF_MERGE == 0 || e == entsize));
    if agree && flags & SHF_MERGE != 0 {
        (flags, entsize)
    } else {
        (0, 0)
    }
}

/// [`gnu_merge_flags`] applied to a debug output: sets its flags and returns
/// its `sh_entsize`.
fn merged_output_flags(osec: &mut OutputSection, input_objs: &[(String, ElfObject)]) -> u64 {
    let (flags, entsize) = gnu_merge_flags(osec.inputs.iter().map(|i| {
        let sec = &input_objs[i.object_idx].1.sections[i.section_idx];
        (sec.flags, sec.entsize)
    }));
    osec.flags = (osec.flags & !MERGE_FLAGS) | flags;
    entsize
}

/// Final symbol values of a link: what the writers resolve relocations with.
pub(super) struct Symbols<'a> {
    pub input_objs: &'a [(String, ElfObject)],
    pub sec_mapping: &'a SectionMap,
    pub section_vaddrs: &'a [u64],
    pub local_sym_vaddrs: &'a [Vec<u64>],
    pub global_syms: &'a FxHashMap<String, GlobalSym>,
}

/// Whether `shndx` names a section of the object (not UNDEF/ABS/COMMON or
/// another reserved index).
fn regular(shndx: u16) -> bool {
    shndx != SHN_UNDEF && shndx < 0xff00
}

impl Symbols<'_> {
    /// `S` and `A` of a debug relocation against `sym_idx` with `addend`:
    /// [`Self::value`], except that a symbol in a merged `SHF_MERGE` input
    /// resolves through the pool -- a section symbol to `map(addend)`, any
    /// other to `map(value) + addend` (`_bfd_elf_rela_local_sym`).
    fn resolve(
        &self,
        debug: &DebugSections,
        obj_idx: usize,
        sym_idx: usize,
        addend: i64,
    ) -> Option<(u64, i64)> {
        let sym = &self.input_objs[obj_idx].1.symbols[sym_idx];
        let key = (obj_idx, usize::from(sym.shndx));
        if regular(sym.shndx)
            && let (Some(map), Some(&(_, base))) = (debug.merged.get(&key), debug.map.get(&key))
        {
            return Some(if sym.sym_type() == STT_SECTION {
                let at = sym.value.wrapping_add_signed(addend);
                (base + map.map(at), 0)
            } else {
                (base + map.map(sym.value), addend)
            });
        }
        self.value(debug, obj_idx, sym_idx).map(|s| (s, addend))
    }

    /// The value a debug relocation in object `obj_idx` resolves symbol
    /// `sym_idx` to, or `None` when it is defined in a section the link
    /// dropped, which takes the tombstone instead (bfd's
    /// `RELOC_AGAINST_DISCARDED_SECTION`).
    fn value(&self, debug: &DebugSections, obj_idx: usize, sym_idx: usize) -> Option<u64> {
        let obj = &self.input_objs[obj_idx].1;
        let sym = &obj.symbols[sym_idx];
        let section = usize::from(sym.shndx);
        if regular(sym.shndx) {
            // A label or section symbol in a debug section: its offset in
            // the output section, whose address is 0.
            if let Some(&(_, base)) = debug.map.get(&(obj_idx, section)) {
                let offset = if sym.sym_type() == STT_SECTION {
                    0
                } else {
                    sym.value
                };
                return Some(base + offset);
            }
        }
        if !sym.name.is_empty() && sym.binding() != STB_LOCAL && sym.sym_type() != STT_SECTION {
            // A global: its final definition, or 0 when it has none here
            // (an undefined weak, or an import resolved at run time).
            return Some(
                self.global_syms
                    .get(sym.name.as_str())
                    .filter(|g| g.defined)
                    .map_or(0, |g| g.value),
            );
        }
        match sym.shndx {
            SHN_UNDEF => Some(0),
            SHN_ABS => Some(sym.value),
            SHN_COMMON => Some(self.local_sym_vaddrs[obj_idx][sym_idx]),
            _ if !self.sec_mapping.contains_key(&(obj_idx, section)) => None,
            _ if sym.sym_type() == STT_SECTION => {
                let (merged, offset) = self.sec_mapping[&(obj_idx, section)];
                Some(self.section_vaddrs[merged] + offset)
            }
            _ => Some(self.local_sym_vaddrs[obj_idx][sym_idx]),
        }
    }
}

/// What a debug relocation does to its field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    /// field = S + A
    Set,
    /// field = S + A - P
    PcRel,
    /// field += S + A
    Add,
    /// field -= S + A
    Sub,
    /// low 6 bits = S + A (`DW_CFA_advance_loc`)
    Set6,
    /// low 6 bits -= S + A
    Sub6,
}

/// The operation and field width of a debug relocation type. Every type
/// GCC 14/16 and Clang 23 emit into RISC-V debug sections is here
/// (measured on `-O0`/`-O2`, relaxed and `-mno-relax` objects); TLS
/// `DTPREL` is not, because neither compiler describes a TLS location on
/// RISC-V, and an unknown type is an error rather than a silently wrong
/// field.
fn op_of(rel_type: u32) -> Option<(Op, usize)> {
    Some(match rel_type {
        R_RISCV_32 | R_RISCV_SET32 => (Op::Set, 4),
        R_RISCV_64 => (Op::Set, 8),
        R_RISCV_SET16 => (Op::Set, 2),
        R_RISCV_SET8 => (Op::Set, 1),
        R_RISCV_32_PCREL => (Op::PcRel, 4),
        R_RISCV_ADD8 => (Op::Add, 1),
        R_RISCV_ADD16 => (Op::Add, 2),
        R_RISCV_ADD32 => (Op::Add, 4),
        R_RISCV_ADD64 => (Op::Add, 8),
        R_RISCV_SUB8 => (Op::Sub, 1),
        R_RISCV_SUB16 => (Op::Sub, 2),
        R_RISCV_SUB32 => (Op::Sub, 4),
        R_RISCV_SUB64 => (Op::Sub, 8),
        R_RISCV_SET6 => (Op::Set6, 1),
        R_RISCV_SUB6 => (Op::Sub6, 1),
        _ => return None,
    })
}

/// The new contents of a `width`-byte field holding `old`, for symbol value
/// `s`, addend `a` and place `p`. Like bfd's RISC-V howtos, none of these
/// checks overflow (`complain_overflow_dont`): the field keeps the low bits.
fn apply_op(op: Op, width: usize, old: u64, s: u64, a: i64, p: u64) -> u64 {
    let sa = s.wrapping_add(a as u64);
    let v = match op {
        Op::Set => sa,
        Op::PcRel => sa.wrapping_sub(p),
        Op::Add => old.wrapping_add(sa),
        Op::Sub => old.wrapping_sub(sa),
        Op::Set6 => (old & 0xc0) | (sa & 0x3f),
        Op::Sub6 => (old & 0xc0) | (old.wrapping_sub(sa) & 0x3f),
    };
    if width == 8 {
        v
    } else {
        v & ((1u64 << (8 * width)) - 1)
    }
}

/// Append the debug sections to `out` after the image, at address 0, and
/// resolve their relocations in place. References into dropped sections
/// get bfd's tombstone: 0, or 1 in `.debug_ranges` and `.debug_loc`, where
/// a 0 pair would end the list.
pub(super) fn write_debug_sections(
    out: &mut Vec<u8>,
    debug: &mut DebugSections,
    syms: &Symbols,
) -> Result<(), String> {
    let end = linker_common::place_debug_sections(&mut debug.outputs, out.len() as u64);
    out.resize(end as usize, 0);
    for osec in &debug.outputs {
        for input in &osec.inputs {
            let key = (input.object_idx, input.section_idx);
            let data = match debug.pools.get(&key) {
                Some(pool) => pool.as_slice(),
                None => syms.input_objs[key.0].1.section_data[key.1].as_slice(),
            };
            let at = (osec.file_offset + input.output_offset) as usize;
            out[at..at + data.len()].copy_from_slice(data);
        }
    }
    for (obj_idx, (obj_name, obj)) in syms.input_objs.iter().enumerate() {
        for (sec_idx, relas) in obj.relocations.iter().enumerate() {
            let Some(&(out_idx, base)) = debug.map.get(&(obj_idx, sec_idx)) else {
                continue;
            };
            let osec = &debug.outputs[out_idx];
            let tombstone = match osec.name.as_str() {
                ".debug_ranges" | ".debug_loc" => 1,
                _ => 0,
            };
            let sec_name = &obj.sections[sec_idx].name;
            let input_data = obj.section_data[sec_idx].as_slice();
            let field_at = |offset: u64, width: usize| -> Result<usize, String> {
                if offset + width as u64 > obj.sections[sec_idx].size {
                    return Err(format!(
                        "{obj_name}: relocation at 0x{offset:x} is outside {sec_name}"
                    ));
                }
                Ok((osec.file_offset + base + offset) as usize)
            };
            let mut i = 0;
            while i < relas.len() {
                let r = &relas[i];
                i += 1;
                if r.rela_type == 0 {
                    continue;
                }
                let s = syms.resolve(debug, obj_idx, r.sym_idx as usize, r.addend);
                if r.rela_type == R_RISCV_SET_ULEB128 {
                    // `validate_input_relocations` guarantees the SUB half
                    // follows at the same offset and the field is a
                    // terminated ULEB128 of the input.
                    let sub = &relas[i];
                    i += 1;
                    debug_assert_eq!(sub.rela_type, R_RISCV_SUB_ULEB128);
                    let len = super::uleb128::field_len(&input_data[r.offset as usize..])?;
                    let at = field_at(r.offset, len)?;
                    let field = &mut out[at..at + len];
                    match (
                        s,
                        syms.resolve(debug, obj_idx, sub.sym_idx as usize, sub.addend),
                    ) {
                        (Some((set, set_a)), Some((sub_s, sub_a))) => {
                            super::uleb128::apply_pair(field, set, set_a, sub_s, sub_a)
                        }
                        _ => super::uleb128::apply_pair(field, tombstone, 0, 0, 0),
                    }
                    .map_err(|e| format!("{obj_name}: {sec_name}: {e}"))?;
                    continue;
                }
                let (op, width) = op_of(r.rela_type).ok_or_else(|| {
                    format!(
                        "{obj_name}: relocation type {} in {sec_name} is not supported in a \
                         debug section",
                        r.rela_type
                    )
                })?;
                let at = field_at(r.offset, width)?;
                let mut bytes = [0u8; 8];
                bytes[..width].copy_from_slice(&out[at..at + width]);
                let value = match s {
                    Some((s, a)) => {
                        apply_op(op, width, u64::from_le_bytes(bytes), s, a, base + r.offset)
                    }
                    None => tombstone,
                };
                out[at..at + width].copy_from_slice(&value.to_le_bytes()[..width]);
            }
        }
    }
    Ok(())
}

/// The static symbol table: every local the shared policy keeps, then every
/// defined global this link does not import, in name order. TLS values are
/// template offsets (`symtab_value`); the inclusion rules live in
/// `linker_common::symtab`.
///
/// A weak symbol this linker synthesized as a placeholder for an optional
/// runtime hook carries `placeholder` on the global and stays out, exactly as
/// bfd leaves it undefined. The previous filter approximated that with
/// "weak and value 0 and no section", which also discarded a legitimate
/// zero-valued weak definition (`-Ttext=0` freestanding links).
pub(super) fn symtab(syms: &Symbols, headers: &[ShdrSpec]) -> Vec<SymtabEntry> {
    let mut builder = SymtabBuilder::new(headers);
    for (obj_idx, (_, obj)) in syms.input_objs.iter().enumerate() {
        for (sym_idx, sym) in obj.symbols.iter().enumerate() {
            if sym.binding() != STB_LOCAL || !include_local(&sym.name, sym.sym_type()) {
                continue;
            }
            // A local absolute symbol belongs to no section; its value is
            // final and `SHN_ABS` is its home.
            if sym.shndx == SHN_ABS {
                builder.push_absolute(&sym.name, sym.info, sym.other, sym.value, sym.size);
                continue;
            }
            if !regular(sym.shndx)
                || !syms
                    .sec_mapping
                    .contains_key(&(obj_idx, usize::from(sym.shndx)))
            {
                continue;
            }
            let value = syms.local_sym_vaddrs[obj_idx][sym_idx];
            builder.push(&sym.name, sym.info, sym.other, value, sym.size);
        }
    }
    let mut names: Vec<&String> = syms
        .global_syms
        .iter()
        .filter(|(_, g)| include_global(g.defined, g.needs_plt) && !g.placeholder)
        .map(|(n, _)| n)
        .collect();
    names.sort();
    for name in names {
        let g = &syms.global_syms[name];
        builder.push(
            name,
            (g.binding << 4) | g.sym_type,
            g.visibility,
            g.value,
            g.size,
        );
    }
    builder.finish()
}

/// Section headers for the allocated merged sections at their final
/// places; `.riscv.attributes` is placed by each writer itself.
pub(super) fn merged_headers(
    merged: &[super::relocations::MergedSection],
    vaddrs: &[u64],
    offsets: &[u64],
) -> Vec<ShdrSpec> {
    merged
        .iter()
        .enumerate()
        .filter(|(_, ms)| ms.sh_flags & SHF_ALLOC != 0)
        .map(|(i, ms)| {
            (
                i,
                ms,
                if ms.script_write {
                    ms.sh_flags & !SHF_WRITE
                } else {
                    ms.sh_flags
                },
            )
        })
        .map(|(i, ms, flags)| ShdrSpec {
            align: ms.align.max(1),
            entsize: if ms.sh_flags & SHF_MERGE != 0 {
                ms.entsize
            } else {
                ShdrSpec::new(&ms.name, ms.sh_type, flags, 0, 0, 0).entsize
            },
            ..ShdrSpec::new(
                &ms.name,
                ms.sh_type,
                flags,
                vaddrs[i],
                offsets[i],
                ms.data.len() as u64,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GNU ld's `lang_add_section` merge rule, for any output section.
    #[test]
    fn merge_flags_survive_only_full_agreement() {
        const A: u64 = SHF_ALLOC;
        let ams = A | SHF_MERGE | SHF_STRINGS;
        // One string pool, or several with one entry size: kept with it.
        assert_eq!(gnu_merge_flags([(ams, 1)].into_iter()), (MERGE_FLAGS, 1));
        assert_eq!(
            gnu_merge_flags([(ams, 1), (ams, 1)].into_iter()),
            (MERGE_FLAGS, 1)
        );
        assert_eq!(
            gnu_merge_flags([(A | SHF_MERGE, 8), (A | SHF_MERGE, 8)].into_iter()),
            (SHF_MERGE, 8)
        );
        // Plain data beside a pool, differing entry sizes, or STRINGS vs
        // constants: cleared.
        assert_eq!(gnu_merge_flags([(A, 0), (ams, 1)].into_iter()), (0, 0));
        assert_eq!(gnu_merge_flags([(ams, 1), (A, 0)].into_iter()), (0, 0));
        assert_eq!(gnu_merge_flags([(ams, 1), (ams, 2)].into_iter()), (0, 0));
        assert_eq!(
            gnu_merge_flags([(ams, 1), (A | SHF_MERGE, 1)].into_iter()),
            (0, 0)
        );
        // SHF_STRINGS without SHF_MERGE is not a merge section.
        assert_eq!(gnu_merge_flags([(A | SHF_STRINGS, 1)].into_iter()), (0, 0));
        assert_eq!(gnu_merge_flags(std::iter::empty()), (0, 0));
    }

    #[test]
    fn field_operations_follow_the_psabi() {
        // ADD/SUB pairs compute a label difference in place.
        let f = apply_op(Op::Add, 2, 0, 0x10_0040, 0, 0);
        assert_eq!(apply_op(Op::Sub, 2, f, 0x10_0010, 0, 0), 0x30);
        // A 64-bit SUB below zero wraps instead of trapping.
        assert_eq!(apply_op(Op::Sub, 8, 0, 1, 0, 0), u64::MAX);
        // SET6/SUB6 touch only the low six bits (DW_CFA_advance_loc keeps
        // its opcode in the top two).
        let f = apply_op(Op::Set6, 1, 0x40, 0x25, 0, 0);
        assert_eq!(f, 0x65);
        assert_eq!(apply_op(Op::Sub6, 1, f, 0x20, 0, 0), 0x45);
        // R_RISCV_32 keeps the low bits; 32_PCREL is relative to the place.
        assert_eq!(apply_op(Op::Set, 4, 0, 0x1_2345_6789, 4, 0), 0x2345_678d);
        assert_eq!(apply_op(Op::PcRel, 4, 0, 0x100, 0, 0x110), 0xffff_fff0);
        assert_eq!(apply_op(Op::Set, 1, 0, 0x1ff, 0, 0), 0xff);
    }

    #[test]
    fn every_measured_debug_relocation_has_an_operation() {
        for t in [
            R_RISCV_32,
            R_RISCV_64,
            R_RISCV_32_PCREL,
            R_RISCV_ADD8,
            R_RISCV_ADD16,
            R_RISCV_ADD32,
            R_RISCV_ADD64,
            R_RISCV_SUB8,
            R_RISCV_SUB16,
            R_RISCV_SUB32,
            R_RISCV_SUB64,
            R_RISCV_SET6,
            R_RISCV_SUB6,
            R_RISCV_SET8,
            R_RISCV_SET16,
            R_RISCV_SET32,
        ] {
            assert!(op_of(t).is_some(), "type {t}");
        }
        // TLS_DTPREL64 (9) and code relocations are refused.
        assert!(op_of(9).is_none());
        assert!(op_of(super::super::relocations::R_RISCV_HI20).is_none());
    }
}
