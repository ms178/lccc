//! DWARF sections and the section-header table in AArch64 links.
//!
//! Debug sections are never loaded. They follow the last segment in the file
//! with address 0, and their relocations are resolved statically -- never
//! through the GOT or PLT and never as dynamic relocations, even in a PIE or
//! a shared object, because nothing reads them at run time. This is bfd's
//! `elfNN_aarch64_relocate_section` treatment of sections without
//! `SEC_ALLOC`, and the same design as `i686/linker/debug.rs`.

use super::elf::*;
use super::reloc::resolve_sym;
use super::types::GlobalSymbol;
use crate::backend::elf::STT_TLS;
use crate::backend::linker_common::OutputSection;
use crate::backend::linker_common::shdr::{
    ShdrSpec, SymSection, SymtabEntry, order_headers, section_of,
};
use crate::backend::linker_common::symtab::{SymtabBuilder, include_global, include_local};
use crate::common::fx_hash::FxHashMap;

type SectionMap = FxHashMap<(usize, usize), (usize, u64)>;

/// Whether `(obj, shndx)` names a real input section that the link dropped
/// (a COMDAT loser or a section collected by `--gc-sections`).
fn dropped(section_map: &SectionMap, obj: usize, shndx: u16) -> bool {
    shndx != SHN_UNDEF
        && shndx != SHN_ABS
        && shndx != SHN_COMMON
        && shndx < 0xff00
        && !section_map.contains_key(&(obj, usize::from(shndx)))
}

/// The address a debug relocation resolves `sym` to, or `None` when its
/// definition was dropped, which takes the tombstone instead (bfd's
/// `RELOC_AGAINST_DISCARDED_SECTION`).
fn symbol_address(
    obj_idx: usize,
    sym: &Symbol,
    globals: &FxHashMap<String, GlobalSymbol>,
    section_map: &SectionMap,
    output_sections: &[OutputSection],
) -> Option<u64> {
    let named_global = !sym.name.is_empty() && !sym.is_local() && sym.sym_type() != STT_SECTION;
    if named_global {
        if let Some(g) = globals.get(sym.name.as_str()) {
            if let Some(d) = g.defined_in {
                if d != usize::MAX && !g.is_dynamic && dropped(section_map, d, g.section_idx) {
                    return None;
                }
            }
        }
    } else if dropped(section_map, obj_idx, sym.shndx) {
        return None;
    }
    Some(resolve_sym(
        obj_idx,
        sym,
        globals,
        section_map,
        output_sections,
    ))
}

/// The field a debug relocation writes: its width, and the value for a
/// symbol at `s` with addend `a` and place `p` (the offset in the output
/// debug section, whose address is 0). Overflow follows bfd's howtos:
/// `ABS32` is `complain_overflow_bitfield`, `PREL32` is signed.
fn debug_field(rel_type: u32, s: u64, a: i64, p: u64) -> Result<(usize, u64), String> {
    let sa = i128::from(s) + i128::from(a);
    let fits = |v: i128, lo: i128, hi: i128, what: &str| {
        if (lo..=hi).contains(&v) {
            Ok(v as u64)
        } else {
            Err(format!(
                "{what} value 0x{v:x} does not fit in a debug section field"
            ))
        }
    };
    match rel_type {
        R_AARCH64_ABS64 => Ok((8, sa as u64)),
        R_AARCH64_ABS32 => Ok((4, fits(sa, -(1 << 31), (1 << 32) - 1, "R_AARCH64_ABS32")?)),
        R_AARCH64_PREL64 => Ok((8, (sa - i128::from(p)) as u64)),
        R_AARCH64_PREL32 => Ok((
            4,
            fits(
                sa - i128::from(p),
                -(1 << 31),
                (1 << 31) - 1,
                "R_AARCH64_PREL32",
            )?,
        )),
        other => Err(format!(
            "relocation type {other} is not supported in a debug section"
        )),
    }
}

fn field_width(rel_type: u32) -> Option<usize> {
    match rel_type {
        R_AARCH64_ABS64 | R_AARCH64_PREL64 => Some(8),
        R_AARCH64_ABS32 | R_AARCH64_PREL32 => Some(4),
        _ => None,
    }
}

/// Resolve the relocations of every debug input section in `out`, which
/// already holds the sections' data at their file offsets. References into
/// dropped sections get bfd's tombstone: 0, or 1 in `.debug_ranges` and
/// `.debug_loc`, where a 0 pair would end the list.
pub(super) fn apply_debug_relocations(
    objects: &[ElfObject],
    globals: &FxHashMap<String, GlobalSymbol>,
    section_map: &SectionMap,
    output_sections: &[OutputSection],
    out: &mut [u8],
) -> Result<(), String> {
    for (obj_idx, obj) in objects.iter().enumerate() {
        for (sec_idx, relas) in obj.relocations.iter().enumerate() {
            if relas.is_empty() {
                continue;
            }
            let Some(&(out_idx, base)) = section_map.get(&(obj_idx, sec_idx)) else {
                continue;
            };
            let osec = &output_sections[out_idx];
            if osec.flags & SHF_ALLOC != 0 {
                continue;
            }
            let tombstone = match osec.name.as_str() {
                ".debug_ranges" | ".debug_loc" => 1,
                _ => 0,
            };
            let sec_name = &obj.sections[sec_idx].name;
            for rela in relas {
                if rela.rela_type == R_AARCH64_NONE {
                    continue;
                }
                let sym = obj.symbols.get(rela.sym_idx as usize).ok_or_else(|| {
                    format!(
                        "{}: invalid symbol index {} in a relocation of {sec_name}",
                        obj.source_name, rela.sym_idx
                    )
                })?;
                let place = base + rela.offset;
                let (width, value) =
                    match symbol_address(obj_idx, sym, globals, section_map, output_sections) {
                        None => (
                            field_width(rela.rela_type).ok_or_else(|| {
                                format!(
                                    "{}: relocation type {} in {sec_name} is not supported \
                                     in a debug section",
                                    obj.source_name, rela.rela_type
                                )
                            })?,
                            tombstone,
                        ),
                        Some(s) => debug_field(rela.rela_type, s, rela.addend, place)
                            .map_err(|e| format!("{}: {sec_name}: {e}", obj.source_name))?,
                    };
                let end = place + width as u64;
                if end > osec.mem_size {
                    return Err(format!(
                        "{}: relocation at 0x{:x} is outside {sec_name}",
                        obj.source_name, rela.offset
                    ));
                }
                let at = (osec.file_offset + place) as usize;
                out[at..at + width].copy_from_slice(&value.to_le_bytes()[..width]);
            }
        }
    }
    Ok(())
}

/// Section headers for the output sections and the synthesized sections in
/// `extra`, in [`order_headers`] order.
pub(super) fn section_headers(
    output_sections: &[OutputSection],
    extra: Vec<ShdrSpec>,
) -> Vec<ShdrSpec> {
    let mut specs: Vec<ShdrSpec> = output_sections
        .iter()
        .map(|s| {
            let alloc = s.flags & SHF_ALLOC != 0;
            ShdrSpec {
                align: s.alignment.max(1),
                ..ShdrSpec::new(
                    &s.name,
                    s.sh_type,
                    s.flags,
                    if alloc { s.addr } else { 0 },
                    s.file_offset,
                    s.mem_size,
                )
            }
        })
        .collect();
    specs.extend(extra);
    order_headers(specs)
}

/// The static symbol table: every local the shared policy keeps, then every
/// defined global this link does not import, in name order. TLS values are
/// template offsets (`symtab_value`); section resolution goes through
/// `linker_common::symtab`, which owns the inclusion rules for every backend.
pub(super) fn symtab(
    objects: &[ElfObject],
    globals: &FxHashMap<String, GlobalSymbol>,
    section_map: &SectionMap,
    output_sections: &[OutputSection],
    headers: &[ShdrSpec],
) -> Vec<SymtabEntry> {
    let mut builder = SymtabBuilder::new(headers);
    for (obj_idx, obj) in objects.iter().enumerate() {
        for sym in &obj.symbols {
            if !sym.is_local() || !include_local(&sym.name, sym.sym_type()) {
                continue;
            }
            // A local absolute symbol (`-Ttext`-style label) is defined in no
            // section: its value is already final and `SHN_ABS` is its home.
            if sym.shndx == SHN_ABS {
                builder.push_absolute(&sym.name, sym.info, sym.other, sym.value, sym.size);
                continue;
            }
            let Some(&(oi, so)) = section_map.get(&(obj_idx, usize::from(sym.shndx))) else {
                continue;
            };
            let value = output_sections[oi].addr + so + sym.value;
            builder.push(&sym.name, sym.info, sym.other, value, sym.size);
        }
    }
    let mut names: Vec<&String> = globals
        .iter()
        .filter(|(_, g)| include_global(g.defined_in.is_some(), g.is_dynamic))
        .map(|(n, _)| n)
        .collect();
    names.sort();
    for name in names {
        let g = &globals[name];
        // A linker-defined address symbol names its section; an `SHN_ABS`
        // definition from an input is absolute on purpose.
        if g.section_idx == SHN_ABS && g.defined_in != Some(usize::MAX) {
            builder.push_absolute(name, g.info, g.visibility, g.value, g.size);
        } else {
            builder.push(name, g.info, g.visibility, g.value, g.size);
        }
    }
    builder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_field_arithmetic() {
        assert_eq!(
            debug_field(R_AARCH64_ABS64, 0x400100, 8, 0).unwrap(),
            (8, 0x400108)
        );
        assert_eq!(debug_field(R_AARCH64_ABS32, 0x40, 4, 0).unwrap(), (4, 0x44));
        // bitfield overflow: a negative 32-bit value fits, 2^32 does not.
        assert!(debug_field(R_AARCH64_ABS32, 0, -1, 0).is_ok());
        assert!(debug_field(R_AARCH64_ABS32, 1 << 32, 0, 0).is_err());
        // PREL32 inside one debug section: S and P are both offsets.
        assert_eq!(
            debug_field(R_AARCH64_PREL32, 0x30, 0, 0x10).unwrap(),
            (4, 0x20)
        );
        assert!(debug_field(R_AARCH64_PREL32, 0x1_0000_0000, 0, 0).is_err());
        assert!(debug_field(R_AARCH64_NONE + 1, 0, 0, 0).is_err());
    }

    #[test]
    fn end_symbols_name_the_section_they_close() {
        let h = vec![
            ShdrSpec::new(".text", 1, SHF_ALLOC | SHF_EXECINSTR, 0x1000, 0x1000, 0x100),
            ShdrSpec::new(
                ".bss",
                SHT_NOBITS,
                SHF_ALLOC | SHF_WRITE,
                0x3000,
                0x2000,
                0x40,
            ),
        ];
        assert_eq!(
            section_of(&h, 0x1010, false),
            SymSection::Named(".text".into())
        );
        assert_eq!(
            section_of(&h, 0x3040, false),
            SymSection::Named(".bss".into())
        );
        assert_eq!(section_of(&h, 0x9000, false), SymSection::Abs);
    }
}
