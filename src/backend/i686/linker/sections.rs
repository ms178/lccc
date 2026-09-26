//! Section merging for the i686 linker.
//!
//! Phase 5 of the linking pipeline: merges input sections from all objects
//! into output sections, handling COMDAT group deduplication and section
//! type/flag assignment.

use crate::common::fx_hash::{FxHashMap, FxHashSet};

use super::types::*;

pub(super) fn merge_sections(
    inputs: &mut [InputObject],
) -> (Vec<OutputSection>, FxHashMap<String, usize>, SectionMap) {
    let mut output_sections: Vec<OutputSection> = Vec::new();
    let mut section_name_to_idx: FxHashMap<String, usize> = FxHashMap::default();
    let mut section_map: SectionMap = FxHashMap::default();

    // COMDAT group deduplication, decided once for the whole link.
    let discarded = discarded_sections(inputs);
    let dropped_fdes = prune_discarded_fdes(inputs, &discarded);
    // Same diagnostic channel as the x86-64 linker's GC/ICF FDE pruning.
    if std::env::var_os("LCCC_DEBUG_GCEH").is_some() {
        eprintln!(
            "[gceh] dropped_fdes={dropped_fdes} discarded_sections={}",
            discarded.len()
        );
    }
    let inputs: &[InputObject] = inputs;

    for (obj_idx, obj) in inputs.iter().enumerate() {
        for sec in obj.sections.iter() {
            if discarded.contains(&(obj_idx, sec.input_index)) {
                continue;
            }
            let out_name = match output_section_name(&sec.name, sec.flags, sec.sh_type) {
                Some(n) => n,
                None => continue,
            };

            let out_idx = if let Some(&idx) = section_name_to_idx.get(&out_name) {
                idx
            } else {
                let idx = output_sections.len();
                let (sh_type, flags) = section_type_and_flags(&out_name, sec);
                section_name_to_idx.insert(out_name.clone(), idx);
                output_sections.push(OutputSection {
                    name: out_name,
                    sh_type,
                    flags,
                    data: Vec::new(),
                    align: 1,
                    addr: 0,
                    file_offset: 0,
                });
                idx
            };

            let out_sec = &mut output_sections[out_idx];
            // .init and .fini must be concatenated without padding
            let align = if out_sec.name == ".init" || out_sec.name == ".fini" {
                1
            } else {
                sec.align.max(1)
            };
            if align > out_sec.align {
                out_sec.align = align;
            }
            let padding = (align - (out_sec.data.len() as u32 % align)) % align;
            out_sec
                .data
                .extend(std::iter::repeat_n(0u8, padding as usize));
            let offset = out_sec.data.len() as u32;

            section_map.insert((obj_idx, sec.input_index), (out_idx, offset));

            if sec.sh_type != SHT_NOBITS {
                out_sec.data.extend_from_slice(&sec.data);
            } else {
                out_sec
                    .data
                    .extend(std::iter::repeat_n(0u8, sec.data.len()));
            }
        }
    }

    (output_sections, section_name_to_idx, section_map)
}

/// Input sections that never reach the output: the members of a COMDAT
/// group whose signature an earlier group already claimed (see
/// `compute_comdat_skip`).
///
/// Identity is the signature alone, as in the gABI, GNU ld and the x86-64
/// linker (`linker_common::comdat`). The merge used to also drop any group
/// member whose section NAME an earlier kept member had used, which threw
/// away the code of distinct groups that merely share a name (`.text` in
/// two `,comdat` groups with different signatures) and deduplicated plain
/// (non-COMDAT) groups, which must never be merged.
fn discarded_sections(inputs: &[InputObject]) -> FxHashSet<(usize, usize)> {
    compute_comdat_skip(inputs)
}

/// Remove the FDEs describing discarded sections from every input
/// `.eh_frame`.  Their `initial_location` relocations would otherwise
/// resolve against a section that has no address -- to 0 -- and
/// `.eh_frame_hdr` would index bogus `[0, size)` entries: one per
/// translation unit for the COMDAT `__x86.get_pc_thunk.*` copies that crt1
/// and every PIC object carry.  GNU ld drops them the same way.  Returns
/// the number of FDEs dropped.
fn prune_discarded_fdes(
    inputs: &mut [InputObject],
    discarded: &FxHashSet<(usize, usize)>,
) -> usize {
    use crate::backend::linker_common::{compact_eh_frame, scan_eh_frame_records};
    if discarded.is_empty() {
        return 0;
    }
    let mut dropped = 0;
    for (obj_idx, obj) in inputs.iter_mut().enumerate() {
        let InputObject {
            sections, symbols, ..
        } = obj;
        for sec in sections.iter_mut() {
            if sec.name != ".eh_frame" || discarded.contains(&(obj_idx, sec.input_index)) {
                continue;
            }
            let records = scan_eh_frame_records(&sec.data);
            let prune: Vec<bool> = records
                .iter()
                .map(|rec| {
                    let Some(iloc) = rec.iloc_offset else {
                        return false;
                    };
                    let Some(&(_, _, sym_idx, _)) =
                        sec.relocations.iter().find(|r| r.0 as usize == iloc)
                    else {
                        return false; // no relocation: cannot prove the target is gone
                    };
                    symbols.get(sym_idx as usize).is_some_and(|sym| {
                        let shndx = sym.section_index;
                        shndx != SHN_UNDEF
                            && shndx != SHN_ABS
                            && shndx != SHN_COMMON
                            && discarded.contains(&(obj_idx, shndx as usize))
                    })
                })
                .collect();
            let n = prune.iter().filter(|&&p| p).count();
            if n == 0 {
                continue;
            }
            let compacted = compact_eh_frame(&sec.data, &records, &prune);
            sec.relocations = sec
                .relocations
                .iter()
                .filter_map(|&(off, ty, sym, addend)| {
                    let off = compacted.map_offset(off as usize)?;
                    Some((off as u32, ty, sym, addend))
                })
                .collect();
            sec.data = compacted.data;
            dropped += n;
        }
    }
    dropped
}

pub(super) fn compute_comdat_skip(inputs: &[InputObject]) -> FxHashSet<(usize, usize)> {
    let mut comdat_skip = FxHashSet::default();
    let mut seen_groups: FxHashSet<String> = FxHashSet::default();

    for (obj_idx, obj) in inputs.iter().enumerate() {
        for sec in obj.sections.iter() {
            if sec.sh_type != SHT_GROUP {
                continue;
            }
            if sec.data.len() < 4 {
                continue;
            }
            let flags = read_u32(&sec.data, 0);
            if flags & 1 == 0 {
                continue;
            }
            let sig_name = match obj.symbols.get(sec.info as usize) {
                // An unnamed signature identifies nothing: keep the group,
                // as `linker_common::plan_comdat` does.
                Some(sym) if !sym.name.is_empty() => sym.name.clone(),
                _ => continue,
            };
            if !seen_groups.insert(sig_name) {
                let mut off = 4;
                while off + 4 <= sec.data.len() {
                    let member_idx = read_u32(&sec.data, off) as usize;
                    comdat_skip.insert((obj_idx, member_idx));
                    off += 4;
                }
            }
        }
    }

    comdat_skip
}

pub(super) fn section_type_and_flags(out_name: &str, sec: &InputSection) -> (u32, u32) {
    match out_name {
        ".text" => (SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR),
        ".rodata" => (SHT_PROGBITS, SHF_ALLOC),
        ".data" => (SHT_PROGBITS, SHF_ALLOC | SHF_WRITE),
        ".bss" => (SHT_NOBITS, SHF_ALLOC | SHF_WRITE),
        ".tdata" => (SHT_PROGBITS, SHF_ALLOC | SHF_WRITE | SHF_TLS),
        ".tbss" => (SHT_NOBITS, SHF_ALLOC | SHF_WRITE | SHF_TLS),
        ".init" | ".fini" => (SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR),
        ".init_array" => (SHT_INIT_ARRAY, SHF_ALLOC | SHF_WRITE),
        ".fini_array" => (SHT_FINI_ARRAY, SHF_ALLOC | SHF_WRITE),
        ".eh_frame" => (SHT_PROGBITS, SHF_ALLOC),
        ".note" => (SHT_NOTE, SHF_ALLOC),
        _ => (
            sec.sh_type,
            sec.flags & (SHF_ALLOC | SHF_WRITE | SHF_EXECINSTR),
        ),
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Phase 6: Symbol resolution
// ══════════════════════════════════════════════════════════════════════════════
