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
    gc_collect_sections_elf64_full(
        objects,
        extra_roots,
        extra_section_roots,
        &FxHashSet::default(),
    )
}

/// The sweep proper.  `discarded` are sections already known not to be
/// emitted -- the members of losing COMDAT groups.  They are neither roots
/// nor reachable, and a reference to a symbol defined in one is a reference
/// to the surviving definition of that name.  Without this the sweep, run
/// before COMDAT selection, kept whichever copy of an inline function the
/// *referencing* object carried, while COMDAT selection then kept the
/// first object's copy: when that one was referenced only from dead code,
/// both went and the program jumped into a hole.
///
/// Neither discarded nor dead sections are returned for `discarded`; the
/// result is the set of sections the sweep found unreachable.
pub fn gc_collect_sections_elf64_full(
    objects: &[Elf64Object],
    extra_roots: &[String],
    extra_section_roots: &FxHashSet<(usize, usize)>,
    discarded: &FxHashSet<(usize, usize)>,
) -> FxHashSet<(usize, usize)> {
    // Build the set of all allocatable input sections
    let mut all_sections: FxHashSet<(usize, usize)> = FxHashSet::default();
    for (obj_idx, obj) in objects.iter().enumerate() {
        for (sec_idx, sec) in obj.sections.iter().enumerate() {
            if sec.flags & SHF_ALLOC == 0 || discarded.contains(&(obj_idx, sec_idx)) {
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

    // Symbol name -> the section of the definition the link binds it to:
    // the first strong definition, else the first weak one (ELF symbol
    // resolution), ignoring discarded copies.
    let mut sym_to_section: FxHashMap<&str, ((usize, usize), bool)> = FxHashMap::default();
    for (obj_idx, obj) in objects.iter().enumerate() {
        for sym in &obj.symbols {
            if sym.shndx == SHN_UNDEF || sym.shndx == SHN_ABS || sym.shndx == SHN_COMMON {
                continue;
            }
            let binding = sym.info >> 4;
            if !is_global_binding(binding) || sym.name.is_empty() {
                continue;
            }
            let key = (obj_idx, sym.shndx as usize);
            if key.1 >= obj.sections.len() || discarded.contains(&key) {
                continue;
            }
            let strong = binding != STB_WEAK;
            match sym_to_section.entry(sym.name.as_str()) {
                std::collections::hash_map::Entry::Vacant(v) => {
                    v.insert((key, strong));
                }
                std::collections::hash_map::Entry::Occupied(mut o) => {
                    if strong && !o.get().1 {
                        o.insert((key, true));
                    }
                }
            }
        }
    }
    let sym_to_section: FxHashMap<&str, (usize, usize)> = sym_to_section
        .into_iter()
        .map(|(k, (v, _))| (k, v))
        .collect();

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
                // `.eh_frame` is a root, but it must NOT be traversed: its
                // FDEs carry a relocation against every function in the
                // translation unit, so following them resurrects the whole
                // object and `--gc-sections` silently becomes a no-op.  The
                // FDE-level pass below decides liveness per function instead.
                || name == ".eh_frame" || name.starts_with(".eh_frame.")
                // Notes carry ABI metadata the loader and the debugger read
                // (`.note.gnu.property` for CET, `.note.ABI-tag`, package
                // metadata).  Nothing relocates to them, so a pure
                // reachability sweep would drop them; GNU ld keeps them.
                || name.starts_with(".note.")
                // SHF_GNU_RETAIN: __attribute__((retain)) sections must survive GC.
                || sec.flags & SHF_GNU_RETAIN != 0
                // Sections referenced via __start_/__stop_ bracket symbols.
                || start_stop_referenced.contains(name.as_str())
            {
                mark_live((obj_idx, sec_idx), &mut live, &mut worklist);
            }
        }
    }

    let sweep = Sweep {
        objects,
        all_sections: &all_sections,
        sym_to_section: &sym_to_section,
    };
    sweep.drain(&mut live, &mut worklist);

    // FDE pass: an `.eh_frame` record is only useful while the function it
    // describes is alive, and a *live* FDE drags its own dependencies in --
    // the LSDA in `.gcc_except_table`, the personality routine, the typeinfo
    // the LSDA points at.  Those are reachable only through the FDE, and we
    // deliberately did not traverse `.eh_frame` above, so resurrect them here
    // and re-run the closure.  Dropping them would leave a live FDE with an
    // LSDA pointer into a collected section, which turns a working
    // `catch` into undefined behaviour.
    sweep.resurrect_fde_dependencies(&mut live, &mut worklist);

    // Return the dead sections (all sections minus live ones)
    all_sections.difference(&live).copied().collect()
}

/// ELF bindings that take part in global symbol resolution.
fn is_global_binding(binding: u8) -> bool {
    const STB_GNU_UNIQUE: u8 = 10;
    binding == STB_GLOBAL || binding == STB_WEAK || binding == STB_GNU_UNIQUE
}

fn is_eh_frame_section(name: &str) -> bool {
    name.starts_with(".eh_frame") && !name.ends_with("_hdr")
}

/// The reachability graph: sections are nodes, relocations edges.
struct Sweep<'a> {
    objects: &'a [Elf64Object],
    all_sections: &'a FxHashSet<(usize, usize)>,
    sym_to_section: &'a FxHashMap<&'a str, (usize, usize)>,
}

impl Sweep<'_> {
    /// The section a relocation against symbol `sym_idx` of `obj_idx`
    /// reaches.  A named global goes wherever the link binds the name --
    /// not to the referencing object's own copy, which may be a weak
    /// definition that loses to a strong one, or a COMDAT loser; a local
    /// (section symbols included) stays in its object.
    fn target(&self, obj_idx: usize, sym_idx: u32) -> Option<(usize, usize)> {
        let sym = self.objects[obj_idx].symbols.get(sym_idx as usize)?;
        if is_global_binding(sym.info >> 4) && !sym.name.is_empty() {
            if let Some(&t) = self.sym_to_section.get(sym.name.as_str()) {
                return Some(t);
            }
        }
        if sym.shndx == SHN_UNDEF || sym.shndx == SHN_ABS || sym.shndx == SHN_COMMON {
            return None;
        }
        Some((obj_idx, sym.shndx as usize))
    }

    fn mark(
        &self,
        key: (usize, usize),
        live: &mut FxHashSet<(usize, usize)>,
        worklist: &mut VecDeque<(usize, usize)>,
    ) -> bool {
        let fresh = self.all_sections.contains(&key) && live.insert(key);
        if fresh {
            worklist.push_back(key);
        }
        fresh
    }

    /// Close `live` over relocations.  `.eh_frame` is not traversed here:
    /// its FDEs carry a relocation against every function of the unit, so
    /// following them would resurrect everything; see the FDE pass.
    fn drain(&self, live: &mut FxHashSet<(usize, usize)>, worklist: &mut VecDeque<(usize, usize)>) {
        while let Some((obj_idx, sec_idx)) = worklist.pop_front() {
            let obj = &self.objects[obj_idx];
            if obj
                .sections
                .get(sec_idx)
                .is_some_and(|s| is_eh_frame_section(&s.name))
            {
                continue;
            }
            let Some(relocs) = obj.relocations.get(sec_idx) else {
                continue;
            };
            for rela in relocs {
                if let Some(t) = self.target(obj_idx, rela.sym_idx) {
                    self.mark(t, live, worklist);
                }
            }
        }
    }

    /// Keep what a *live* FDE depends on: whatever its own relocations
    /// reach (the LSDA) and whatever its CIE's reach (the personality
    /// routine's `DW.ref` pointer, a COMDAT data section nothing else
    /// references), then close the graph again, until nothing changes.
    fn resurrect_fde_dependencies(
        &self,
        live: &mut FxHashSet<(usize, usize)>,
        worklist: &mut VecDeque<(usize, usize)>,
    ) {
        // A live FDE stays live; remember which were handled so a fixed-point
        // round only looks at FDEs whose function became live since.
        let mut done: FxHashSet<(usize, usize, usize)> = FxHashSet::default();
        loop {
            let mut progress = false;
            for (obj_idx, obj) in self.objects.iter().enumerate() {
                for (sec_idx, sec) in obj.sections.iter().enumerate() {
                    if !is_eh_frame_section(&sec.name) || !live.contains(&(obj_idx, sec_idx)) {
                        continue;
                    }
                    let Some(data) = obj.section_data.get(sec_idx).map(|d| d.as_slice()) else {
                        continue;
                    };
                    let relocs = obj
                        .relocations
                        .get(sec_idx)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]);
                    if relocs.is_empty() {
                        continue;
                    }
                    // Relocations sorted by offset, for per-record ranges.
                    let mut order: Vec<usize> = (0..relocs.len()).collect();
                    order.sort_by_key(|&i| relocs[i].offset);
                    let in_range = |lo: usize, hi: usize| {
                        let first = order.partition_point(|&i| (relocs[i].offset as usize) < lo);
                        order[first..]
                            .iter()
                            .take_while(move |&&i| (relocs[i].offset as usize) < hi)
                            .map(move |&i| &relocs[i])
                    };
                    let records = super::eh_frame::scan_eh_frame_records(data);
                    for (ri, rec) in records.iter().enumerate() {
                        let Some(iloc) = rec.iloc_offset else {
                            continue;
                        };
                        if done.contains(&(obj_idx, sec_idx, ri)) {
                            continue;
                        }
                        // Is the described function alive?  (No relocation,
                        // or one against an undefined/absolute symbol: the
                        // FDE is kept, so its dependencies are too.)
                        let func = in_range(iloc, iloc + 1)
                            .next()
                            .and_then(|r| self.target(obj_idx, r.sym_idx));
                        if func.is_some_and(|f| !live.contains(&f)) {
                            continue;
                        }
                        done.insert((obj_idx, sec_idx, ri));
                        let cie = super::eh_frame::fde_cie_index(data, &records, ri)
                            .map(|c| (records[c].start, records[c].end));
                        for (lo, hi) in std::iter::once((rec.start, rec.end)).chain(cie) {
                            for rela in in_range(lo, hi) {
                                if let Some(t) = self.target(obj_idx, rela.sym_idx) {
                                    progress |= self.mark(t, live, worklist);
                                }
                            }
                        }
                    }
                }
            }
            self.drain(live, worklist);
            if !progress {
                break;
            }
        }
    }
}
