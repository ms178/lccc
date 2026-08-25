//! Garbage collection (`--gc-sections`) for ELF64 linkers.
//!
//! Performs BFS reachability from entry points (_start, main) and init/fini
//! arrays, following relocations transitively to find all reachable sections.
//! Returns the set of dead (unreachable) input sections to discard.

use super::Elf64Object;
use crate::backend::elf::{
    SHF_ALLOC, SHF_EXCLUDE, SHF_GNU_RETAIN, SHN_ABS, SHN_COMMON, SHN_UNDEF, SHT_GROUP, SHT_NULL,
    SHT_REL, SHT_RELA, SHT_STRTAB, SHT_SYMTAB, STB_GLOBAL, STB_WEAK,
};
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;

/// Perform `--gc-sections`: BFS reachability from entry points, return the set
/// of dead (unreachable) `(object_idx, section_idx)` pairs.
///
/// Starting from entry-point sections (`_start`, `main`) and any init/fini
/// arrays, follows relocations transitively to find all reachable sections.
pub fn gc_collect_sections_elf64(objects: &[Elf64Object]) -> FxHashSet<(usize, usize)> {
    gc_collect_sections_elf64_roots(objects, &[])
}

/// Like `gc_collect_sections_elf64` but with additional root symbols
/// (custom entry point from `-e`, forced-undefined symbols from `-u`,
/// exported symbols, etc.).
pub fn gc_collect_sections_elf64_roots(
    objects: &[Elf64Object],
    extra_roots: &[String],
) -> FxHashSet<(usize, usize)> {
    gc_collect_sections_elf64_roots_and_sections(objects, extra_roots, &FxHashSet::default())
}

/// Script-link variant with explicit input-section roots (the sections matched
/// by GNU linker-script `KEEP(...)` commands).  A script symbol at the start or
/// end of an output section is not itself a relocation and therefore cannot
/// keep a registry/table alive; `KEEP` is the authoritative escape hatch.
pub fn gc_collect_sections_elf64_roots_and_sections(
    objects: &[Elf64Object],
    extra_roots: &[String],
    extra_section_roots: &FxHashSet<(usize, usize)>,
) -> FxHashSet<(usize, usize)> {
    // Build the set of all allocatable input sections
    let mut all_sections: FxHashSet<(usize, usize)> = FxHashSet::default();
    for (obj_idx, obj) in objects.iter().enumerate() {
        for (sec_idx, sec) in obj.sections.iter().enumerate() {
            if sec.flags & SHF_ALLOC == 0 {
                continue;
            }
            if matches!(
                sec.sh_type,
                SHT_NULL | SHT_STRTAB | SHT_SYMTAB | SHT_RELA | SHT_REL | SHT_GROUP
            ) {
                continue;
            }
            if sec.flags & SHF_EXCLUDE != 0 {
                continue;
            }
            all_sections.insert((obj_idx, sec_idx));
        }
    }

    // Build a map from symbol name -> (obj_idx, sec_idx) for defined symbols
    let mut sym_to_section: FxHashMap<&str, (usize, usize)> = FxHashMap::default();
    for (obj_idx, obj) in objects.iter().enumerate() {
        for sym in &obj.symbols {
            if sym.shndx == SHN_UNDEF || sym.shndx == SHN_ABS || sym.shndx == SHN_COMMON {
                continue;
            }
            let binding = sym.info >> 4;
            if binding != STB_GLOBAL && binding != STB_WEAK {
                continue;
            }
            if sym.name.is_empty() {
                continue;
            }
            let sec_idx = sym.shndx as usize;
            if sec_idx < obj.sections.len() {
                sym_to_section
                    .entry(sym.name.as_str())
                    .or_insert((obj_idx, sec_idx));
            }
        }
    }

    // Collect section names referenced via __start_<X> / __stop_<X> symbols.
    // GNU ld treats sections whose name is referenced this way as GC roots
    // (they are reachable through the auto-generated bracket symbols even
    // though no relocation points into them directly).
    let mut start_stop_referenced: FxHashSet<&str> = FxHashSet::default();
    for obj in objects.iter() {
        for sym in &obj.symbols {
            if sym.shndx != SHN_UNDEF {
                continue;
            }
            if let Some(suffix) = sym
                .name
                .strip_prefix("__start_")
                .or_else(|| sym.name.strip_prefix("__stop_"))
            {
                start_stop_referenced.insert(suffix);
            }
        }
    }

    // Seed the worklist with entry-point sections and sections that must be kept
    let mut live: FxHashSet<(usize, usize)> = FxHashSet::default();
    let mut worklist: VecDeque<(usize, usize)> = VecDeque::new();

    let mark_live = |key: (usize, usize),
                     live: &mut FxHashSet<(usize, usize)>,
                     wl: &mut VecDeque<(usize, usize)>| {
        if all_sections.contains(&key) && live.insert(key) {
            wl.push_back(key);
        }
    };

    // Mark sections containing entry-point symbols as live
    let entry_symbols = ["_start", "main", "__libc_csu_init", "__libc_csu_fini"];
    for &entry_name in &entry_symbols {
        if let Some(&key) = sym_to_section.get(entry_name) {
            mark_live(key, &mut live, &mut worklist);
        }
    }
    for root in extra_roots {
        if let Some(&key) = sym_to_section.get(root.as_str()) {
            mark_live(key, &mut live, &mut worklist);
        }
    }
    for &key in extra_section_roots {
        mark_live(key, &mut live, &mut worklist);
    }

    // Mark init/fini array sections as live (these are called by the runtime)
    for (obj_idx, obj) in objects.iter().enumerate() {
        for (sec_idx, sec) in obj.sections.iter().enumerate() {
            if sec.flags & SHF_ALLOC == 0 {
                continue;
            }
            let name = &sec.name;
            // Keep init/fini arrays and .ctors/.dtors (runtime calls these)
            if name == ".init_array" || name.starts_with(".init_array.")
                || name == ".fini_array" || name.starts_with(".fini_array.")
                || name == ".ctors" || name.starts_with(".ctors.")
                || name == ".dtors" || name.starts_with(".dtors.")
                || name == ".preinit_array" || name.starts_with(".preinit_array.")
                || name == ".init" || name == ".fini"
                || name == ".note.GNU-stack"
                || name == ".note.gnu.build-id"
                // SHF_GNU_RETAIN: __attribute__((retain)) sections must survive GC.
                || sec.flags & SHF_GNU_RETAIN != 0
                // Sections referenced via __start_/__stop_ bracket symbols.
                || start_stop_referenced.contains(name.as_str())
            {
                mark_live((obj_idx, sec_idx), &mut live, &mut worklist);
            }
        }
    }

    // BFS: follow relocations from live sections to discover more live sections
    while let Some((obj_idx, sec_idx)) = worklist.pop_front() {
        let obj = &objects[obj_idx];
        // Follow relocations from this section
        if sec_idx < obj.relocations.len() {
            for rela in &obj.relocations[sec_idx] {
                let sym_idx = rela.sym_idx as usize;
                if sym_idx >= obj.symbols.len() {
                    continue;
                }
                let sym = &obj.symbols[sym_idx];

                if sym.shndx != SHN_UNDEF && sym.shndx != SHN_ABS && sym.shndx != SHN_COMMON {
                    // Symbol is defined in this object file
                    let target = (obj_idx, sym.shndx as usize);
                    mark_live(target, &mut live, &mut worklist);
                } else if !sym.name.is_empty() {
                    // Symbol is undefined here; look up in global symbol table
                    if let Some(&target) = sym_to_section.get(sym.name.as_str()) {
                        mark_live(target, &mut live, &mut worklist);
                    }
                }
            }
        }
    }

    // Return the dead sections (all sections minus live ones)
    all_sections.difference(&live).copied().collect()
}
