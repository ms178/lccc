//! DWARF sections in i686 links.
//!
//! Debug sections are never loaded. They follow the last segment in the file
//! with address 0, and their relocations are resolved statically -- never
//! through the GOT or PLT and never as dynamic relocations, even in a PIE or
//! a shared object, because nothing reads them at run time. This is bfd's
//! `elf_i386_relocate_section` treatment of sections without `SEC_ALLOC`.

use super::types::*;
use crate::backend::linker_common::is_debug_section_name;
use crate::common::fx_hash::{FxHashMap, FxHashSet};

/// Append the input DWARF sections, grouped by name, as non-allocated output
/// sections after every allocated one. `discarded` holds the COMDAT losers,
/// whose debug contributions are dropped with their code.
pub(super) fn merge_debug_sections(
    inputs: &[InputObject],
    discarded: &FxHashSet<(usize, usize)>,
    output_sections: &mut Vec<OutputSection>,
    section_name_to_idx: &mut FxHashMap<String, usize>,
    section_map: &mut SectionMap,
) {
    for (obj_idx, obj) in inputs.iter().enumerate() {
        for sec in &obj.sections {
            if sec.flags & SHF_ALLOC != 0
                || sec.sh_type != SHT_PROGBITS
                || !is_debug_section_name(&sec.name)
                || discarded.contains(&(obj_idx, sec.input_index))
            {
                continue;
            }
            let out_idx = *section_name_to_idx
                .entry(sec.name.clone())
                .or_insert_with(|| {
                    output_sections.push(OutputSection {
                        name: sec.name.clone(),
                        sh_type: SHT_PROGBITS,
                        // SHF_MERGE/SHF_STRINGS would need sh_entsize, and the
                        // strings are concatenated, not merged.
                        flags: 0,
                        data: Vec::new(),
                        align: 1,
                        addr: 0,
                        file_offset: 0,
                    });
                    output_sections.len() - 1
                });
            let out = &mut output_sections[out_idx];
            let align = sec.align.max(1);
            out.align = out.align.max(align);
            let padding = (align - out.data.len() as u32 % align) % align;
            out.data.resize(out.data.len() + padding as usize, 0);
            section_map.insert((obj_idx, sec.input_index), (out_idx, out.data.len() as u32));
            out.data.extend_from_slice(&sec.data);
        }
    }
}

/// Give each non-empty non-allocated output section a file offset starting
/// at `file_end`, in output order; returns the new end of the file data.
pub(super) fn place_debug_sections(
    output_sections: &mut [OutputSection],
    mut file_end: u32,
) -> u32 {
    for sec in output_sections
        .iter_mut()
        .filter(|s| s.flags & SHF_ALLOC == 0 && !s.data.is_empty())
    {
        file_end = align_up(file_end, sec.align.max(1));
        sec.addr = 0;
        sec.file_offset = file_end;
        file_end += sec.data.len() as u32;
    }
    file_end
}

/// The address a debug relocation resolves `sym` to, or `None` when the
/// symbol's section was discarded (a COMDAT loser), which takes the
/// tombstone instead.
fn symbol_address(
    obj_idx: usize,
    sym: &InputSymbol,
    section_map: &SectionMap,
    output_sections: &[OutputSection],
    global_symbols: &FxHashMap<String, LinkerSymbol>,
) -> Option<u32> {
    let in_section = sym.section_index != SHN_UNDEF
        && sym.section_index != SHN_ABS
        && sym.section_index != SHN_COMMON;
    let placed = || {
        section_map
            .get(&(obj_idx, usize::from(sym.section_index)))
            .map(|&(out, offset)| output_sections[out].addr + offset)
    };
    if sym.sym_type == STT_SECTION {
        return if in_section { placed() } else { Some(0) };
    }
    if in_section && placed().is_none() {
        return None;
    }
    if sym.binding == STB_LOCAL || sym.name.is_empty() {
        return Some(match sym.section_index {
            SHN_ABS => sym.value,
            SHN_UNDEF | SHN_COMMON => 0,
            _ => placed()? + sym.value,
        });
    }
    Some(match global_symbols.get(sym.name.as_str()) {
        Some(gs) if gs.needs_copy => gs.copy_addr,
        Some(gs) if gs.is_defined && !gs.is_dynamic => gs.address,
        // Undefined (weak) or bound at load time: nothing static to record.
        _ => 0,
    })
}

/// Resolve the relocations of every debug input section in place.
///
/// `tls_addr` is the start of the PT_TLS block: `R_386_TLS_LDO_32` (a TLS
/// variable's DW_AT_location) is the offset of the symbol within it.
/// References into discarded sections get bfd's tombstone, 0 -- or 1 in
/// `.debug_ranges` and `.debug_loc`, where a 0 pair would end the list.
pub(super) fn apply_debug_relocations(
    inputs: &[InputObject],
    section_map: &SectionMap,
    output_sections: &mut [OutputSection],
    global_symbols: &FxHashMap<String, LinkerSymbol>,
    tls_addr: u32,
    got_base: u32,
    shared_object: bool,
) -> Result<(), String> {
    // Both output paths assign `_GLOBAL_OFFSET_TABLE_` from this same
    // layout-selected anchor before resolving debug relocations. The anchor
    // remains defined even when no allocated GOTPC relocation or physical
    // GOT section exists; a debug-only GOTOFF still has the same S+A-GOT
    // meaning. It must not depend on an unrelated instruction's relocation.
    for (obj_idx, obj) in inputs.iter().enumerate() {
        for sec in &obj.sections {
            if sec.relocations.is_empty() {
                continue;
            }
            let Some(&(out_idx, base)) = section_map.get(&(obj_idx, sec.input_index)) else {
                continue;
            };
            if output_sections[out_idx].flags & SHF_ALLOC != 0 {
                continue;
            }
            let tombstone = match output_sections[out_idx].name.as_str() {
                ".debug_ranges" | ".debug_loc" => 1,
                _ => 0,
            };
            for &(offset, rel_type, sym_idx, addend) in &sec.relocations {
                if rel_type == R_386_NONE {
                    continue;
                }
                let Some(sym) = obj.symbols.get(sym_idx as usize) else {
                    return Err(format!(
                        "{}: invalid symbol index {sym_idx} in a relocation of {}",
                        obj.filename, sec.name
                    ));
                };
                if shared_object && rel_type == R_386_GOTOFF && sym.section_index == SHN_UNDEF {
                    // A shared object cannot form a static GOT-relative
                    // address for an unresolved or dynamically bound symbol.
                    // In particular, an undefined weak symbol is not a
                    // section tombstone; GNU ld rejects this relocation.
                    let statically_defined = global_symbols
                        .get(sym.name.as_str())
                        .is_some_and(|g| g.is_defined && !g.is_dynamic);
                    if !statically_defined {
                        return Err(format!(
                            "{}: R_386_GOTOFF against undefined symbol `{}` cannot be used in a shared object",
                            obj.filename, sym.name
                        ));
                    }
                }
                let place = base + offset;
                let value = match symbol_address(
                    obj_idx,
                    sym,
                    section_map,
                    output_sections,
                    global_symbols,
                ) {
                    None => tombstone,
                    Some(s) => {
                        let s_plus_a = s.wrapping_add(addend as u32);
                        match rel_type {
                            R_386_32 => s_plus_a,
                            // A debug section has address 0, so P is its offset.
                            R_386_PC32 => s_plus_a.wrapping_sub(place),
                            R_386_TLS_LDO_32 => s_plus_a.wrapping_sub(tls_addr),
                            R_386_GOTOFF => s_plus_a.wrapping_sub(got_base),
                            other => {
                                return Err(format!(
                                    "{}: relocation type {other} in {} is not supported \
                                     in a debug section",
                                    obj.filename, sec.name
                                ));
                            }
                        }
                    }
                };
                let data = &mut output_sections[out_idx].data;
                let at = place as usize;
                let Some(field) = data.get_mut(at..at + 4) else {
                    return Err(format!(
                        "{}: relocation at 0x{offset:x} is outside {}",
                        obj.filename, sec.name
                    ));
                };
                field.copy_from_slice(&value.to_le_bytes());
            }
        }
    }
    Ok(())
}
