//! Phase 3: Global symbol table construction.
//!
//! Builds the global symbol table from input objects, handles COMMON symbols,
//! marks PLT/GOT needs for dynamic linking, and identifies GOT entries for
//! PC-relative GOT references.

use super::elf_read::*;
use super::got::GotKind;
use super::relocations::{GlobalSym, MergedSection, got_sym_key};
use crate::backend::elf::{STT_FILE, STT_TLS, STV_DEFAULT};
use crate::backend::linker_common;
use crate::common::fx_hash::{FxHashMap, FxHashSet};

/// GNU ld binds a default-visibility COMMON to shared-library data: the
/// tentative definition becomes a reference and is imported like any
/// undefined data (a copy relocation in an executable, an undefined dynamic
/// symbol in a shared object).  Rewriting it before symbol resolution, as
/// bfd's `elf_merge_symbol` does, lets every later phase see a plain
/// reference.  The libraries come after `input_objs[..late_from]`, so a
/// common of those binds where a strong data definition took it
/// (`SharedLibSyms::common_taken`); a common read after them (an archive
/// member) meets the first library definition of the name, and binds to it
/// if that is strong data (`shared_def_overrides_common`: bfd's rule is the
/// same in either order) unless the name was a common already.  A name a
/// regular object gives non-default visibility never binds to a library.
pub fn import_shared_commons(
    input_objs: &mut [(String, ElfObject)],
    late_from: usize,
    shared: &super::input::SharedLibSyms,
) {
    let imports = |name: &str, late: bool| match shared.common_taken(name) {
        Some(taken) => taken,
        None => {
            late && shared
                .get(name)
                .is_some_and(|dsym| linker_common::shared_def_overrides_common(dsym.info))
        }
    };
    let candidates = input_objs.iter().enumerate().any(|(k, (_, obj))| {
        obj.symbols
            .iter()
            .any(|sym| sym.shndx == SHN_COMMON && imports(sym.name.as_str(), k >= late_from))
    });
    if !candidates {
        return;
    }
    let restricted: FxHashSet<String> = input_objs
        .iter()
        .flat_map(|(_, obj)| &obj.symbols)
        .filter(|sym| sym.binding() != STB_LOCAL && sym.visibility() != STV_DEFAULT)
        .map(|sym| sym.name.to_string())
        .collect();
    for (k, (_, obj)) in input_objs.iter_mut().enumerate() {
        for sym in &mut obj.symbols {
            let name = sym.name.as_str();
            if sym.shndx == SHN_COMMON
                && !restricted.contains(name)
                && imports(name, k >= late_from)
            {
                sym.shndx = SHN_UNDEF;
                sym.value = 0;
            }
        }
    }
}

/// Build the global symbol table and allocate only the surviving commons.
/// During selection, COMMON's `value` is its maximum alignment, and
/// `section_idx == Some(PENDING_COMMON)` identifies the pending slot.
pub fn build_global_symbols(
    input_objs: &[(String, ElfObject)],
    sec_mapping: &FxHashMap<(usize, usize), (usize, u64)>,
    merged_sections: &mut Vec<MergedSection>,
    merged_map: &mut FxHashMap<String, usize>,
    allow_multiple_definition: bool,
) -> Result<FxHashMap<String, GlobalSym>, String> {
    const PENDING_COMMON: usize = usize::MAX;
    let mut globals: FxHashMap<String, GlobalSym> = FxHashMap::default();
    let mut commons = Vec::new();
    let mut unplaced_visibility: FxHashMap<&str, u8> = FxHashMap::default();

    for (obj_idx, (source, obj)) in input_objs.iter().enumerate() {
        for sym in &obj.symbols {
            let name = sym.name.as_str();
            if name.is_empty() || sym.is_local() || matches!(sym.sym_type(), STT_SECTION | STT_FILE)
            {
                continue;
            }
            let (section, base) = match sym.shndx {
                SHN_UNDEF | SHN_ABS => (None, 0),
                SHN_COMMON => (Some(PENDING_COMMON), 0),
                shndx => match sec_mapping.get(&(obj_idx, shndx as usize)) {
                    Some(&(section, base)) => (Some(section), base),
                    None => {
                        // A COMDAT loser is not a definition, but it can still
                        // restrict a same-name symbol that does survive layout.
                        let vis = sym.visibility();
                        if vis != STV_DEFAULT {
                            let entry = unplaced_visibility.entry(name).or_default();
                            *entry = linker_common::most_constraining_visibility(*entry, vis);
                        }
                        continue;
                    }
                },
            };
            let defined = !sym.is_undefined();
            let common = section == Some(PENDING_COMMON);
            let binding = sym.binding();
            let rank = |defined, common, binding| match (defined, common, binding == STB_WEAK) {
                (false, _, true) => 0,
                (false, _, false) => 1,
                (true, true, _) => 3,
                (true, false, true) => 2,
                (true, false, false) => 4,
            };
            let mut vis = sym.visibility();
            let mut old = globals.get_mut(name);
            if let Some(e) = old.as_deref() {
                vis = linker_common::most_constraining_visibility(vis, e.visibility);
            }
            if let Some(e) = old.as_deref_mut() {
                let old_common = e.section_idx == Some(PENDING_COMMON);
                let old_rank = rank(e.defined, old_common, e.binding);
                let new_rank = rank(defined, common, binding);
                if common && old_common {
                    e.value = e.value.max(sym.value);
                    e.size = e.size.max(sym.size);
                    e.visibility = vis;
                    if e.binding == STB_WEAK {
                        e.binding = binding;
                    }
                    continue;
                }
                if new_rank <= old_rank {
                    if new_rank == 4
                        && old_rank == 4
                        && !allow_multiple_definition
                        && !(sym.shndx == SHN_ABS
                            && e.section_idx.is_none()
                            && e.value == sym.value)
                    {
                        // Keep provenance off the hot table. Only the failing
                        // path needs to identify the first placed definition.
                        let first =
                            input_objs[..=obj_idx]
                                .iter()
                                .enumerate()
                                .find(|(i, (_, o))| {
                                    o.symbols.iter().any(|s| {
                                        s.name.as_str() == name
                                            && s.is_strong()
                                            && (s.shndx == SHN_ABS
                                                || sec_mapping
                                                    .contains_key(&(*i, s.shndx as usize)))
                                    })
                                });
                        let origin = first.map_or(source.as_str(), |(_, (file, _))| file.as_str());
                        return Err(format!(
                            "multiple definition of '{name}' in {source}; first defined in {origin}",
                        ));
                    }
                    e.visibility = vis;
                    continue;
                }
            }
            if common {
                commons.push(name);
            }
            let value = if defined {
                base.checked_add(sym.value).ok_or_else(|| {
                    format!("{source}: section offset for '{name}' exceeds 64-bit address space",)
                })?
            } else {
                0
            };
            let winner = GlobalSym {
                value,
                size: if defined { sym.size } else { 0 },
                binding,
                sym_type: if common { STT_OBJECT } else { sym.sym_type() },
                visibility: vis,
                defined,
                needs_plt: false,
                plt_idx: 0,
                section_idx: section,
                placeholder: false,
            };
            if let Some(e) = old {
                *e = winner;
            } else {
                globals.insert(name.to_owned(), winner);
            }
        }
    }

    for (name, vis) in unplaced_visibility {
        if let Some(sym) = globals.get_mut(name) {
            sym.visibility = linker_common::most_constraining_visibility(sym.visibility, vis);
        }
    }
    for name in commons {
        let Some(sym) = globals.get_mut(name) else {
            continue;
        };
        if sym.section_idx != Some(PENDING_COMMON) {
            continue; // A strong definition superseded this tentative one.
        }
        let bss_idx = *merged_map.entry(".bss".into()).or_insert_with(|| {
            let idx = merged_sections.len();
            merged_sections.push(MergedSection {
                name: ".bss".into(),
                sh_type: SHT_NOBITS,
                sh_flags: SHF_ALLOC | SHF_WRITE,
                data: Vec::new(),
                nobits_size: 0,
                vaddr: 0,
                entsize: 0,
                script_write: false,
                align: 8,
            });
            idx
        });
        let bss = &mut merged_sections[bss_idx];
        let align = sym.value.max(1);
        let cur = bss.size();
        let Some(off) = cur
            .checked_next_multiple_of(align)
            .filter(|_| align.is_power_of_two())
        else {
            return Err(format!("invalid COMMON alignment {align} for '{name}'"));
        };
        let end = off
            .checked_add(sym.size)
            .ok_or_else(|| format!("COMMON symbol '{name}' exceeds address space"))?;
        bss.zero_extend_to(end)?;
        bss.align = bss.align.max(align);
        sym.value = off;
        sym.section_idx = Some(bss_idx);
    }
    Ok(globals)
}

/// Mark symbols that need PLT entries (undefined functions found in shared libs)
/// and collect symbols that need COPY relocations (undefined data objects).
pub fn mark_plt_and_copy_symbols(
    global_syms: &mut FxHashMap<String, GlobalSym>,
    shared_lib_syms: &FxHashMap<String, DynSymbol>,
) -> (Vec<String>, Vec<(String, u64)>) {
    let mut plt_symbols: Vec<String> = Vec::new();
    let mut copy_symbols: Vec<(String, u64)> = Vec::new();

    for (name, sym) in global_syms.iter_mut() {
        if !sym.defined {
            if let Some(shlib_sym) = shared_lib_syms.get(name) {
                // A TLS import is reached through GOT slots the dynamic
                // loader fills (`got.rs`); it is neither called nor copied.
                if shlib_sym.sym_type() == STT_TLS {
                    continue;
                }
                // Untyped (STT_NOTYPE) exports: data copies (bfd-compatible),
                // code keeps its PLT (see DynSymbol::in_exec_segment).
                if shlib_sym.sym_type() == STT_OBJECT
                    || (shlib_sym.sym_type() == STT_NOTYPE && !shlib_sym.in_exec_segment)
                {
                    copy_symbols.push((name.clone(), shlib_sym.size));
                } else {
                    sym.needs_plt = true;
                    sym.plt_idx = plt_symbols.len();
                    plt_symbols.push(name.clone());
                }
            }
        }
    }

    (plt_symbols, copy_symbols)
}

/// The GOT slots the relocations ask for, in first-reference order, and
/// the defining `(object, symbol, addend)` of each object-local key.
pub fn collect_got_entries(
    input_objs: &[(String, ElfObject)],
) -> (
    Vec<(String, GotKind)>,
    FxHashMap<String, (usize, usize, i64)>,
) {
    let mut wanted: Vec<(String, GotKind)> = Vec::new();
    let mut local_got_sym_info: FxHashMap<String, (usize, usize, i64)> = FxHashMap::default();
    for (obj_idx, (_, obj)) in input_objs.iter().enumerate() {
        for relocs in &obj.relocations {
            for reloc in relocs {
                let Some(kind) = GotKind::of(reloc.rela_type) else {
                    continue;
                };
                let sym = &obj.symbols[reloc.sym_idx as usize];
                let (name, is_local) = got_sym_key(obj_idx, sym, reloc.addend);
                if name.is_empty() {
                    continue;
                }
                if is_local {
                    local_got_sym_info.entry(name.clone()).or_insert((
                        obj_idx,
                        reloc.sym_idx as usize,
                        reloc.addend,
                    ));
                }
                wanted.push((name, kind));
            }
        }
    }
    (wanted, local_got_sym_info)
}

/// Build local symbol virtual address table for relocation resolution.
pub fn build_local_sym_vaddrs(
    input_objs: &[(String, ElfObject)],
    sec_mapping: &FxHashMap<(usize, usize), (usize, u64)>,
    section_vaddrs: &[u64],
    global_syms: &FxHashMap<String, GlobalSym>,
) -> Vec<Vec<u64>> {
    let mut local_sym_vaddrs: Vec<Vec<u64>> = Vec::new();
    for (obj_idx, (_, obj)) in input_objs.iter().enumerate() {
        let mut sym_vaddrs = vec![0u64; obj.symbols.len()];
        for (si, sym) in obj.symbols.iter().enumerate() {
            if sym.shndx == SHN_UNDEF || sym.shndx == SHN_ABS {
                if sym.shndx == SHN_ABS {
                    sym_vaddrs[si] = sym.value;
                }
                continue;
            }
            if sym.shndx == SHN_COMMON {
                if let Some(gs) = global_syms.get(sym.name.as_str()) {
                    sym_vaddrs[si] = gs.value;
                }
                continue;
            }
            let sec_idx = sym.shndx as usize;
            if let Some(&(merged_idx, offset)) = sec_mapping.get(&(obj_idx, sec_idx)) {
                sym_vaddrs[si] = section_vaddrs[merged_idx] + offset + sym.value;
            }
        }
        local_sym_vaddrs.push(sym_vaddrs);
    }
    local_sym_vaddrs
}

/// Check for truly undefined symbols (not dynamic, not weak, not linker-defined).
pub fn check_undefined_symbols(
    global_syms: &FxHashMap<String, GlobalSym>,
    shared_lib_syms: &FxHashMap<String, DynSymbol>,
) -> Result<(), String> {
    let mut truly_undefined: Vec<&String> = global_syms
        .iter()
        .filter(|(name, sym)| {
            !sym.defined
                && !sym.needs_plt
                && sym.binding != STB_WEAK
                && !crate::backend::linker_common::is_linker_defined_symbol(name)
                && !shared_lib_syms.contains_key(name.as_str())
        })
        .map(|(name, _)| name)
        .collect();

    if !truly_undefined.is_empty() {
        truly_undefined.sort();
        truly_undefined.truncate(20);
        return Err(format!(
            "undefined symbols: {}",
            truly_undefined
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::elf::{STB_GLOBAL, STB_WEAK, STT_FUNC, STT_TLS, STV_HIDDEN};
    use crate::backend::linker_common::{Elf64Object, Elf64Symbol, SymStr};

    /// GNU ld's COMMON-vs-shared-library rules (each measured with bfd;
    /// `tests/linker` `common_binds_shared_data` checks real links): a
    /// common read before the libraries is taken by the first linked strong
    /// data definition, even after a weak one; one read after them by the
    /// first library definition if that is strong data; weak data, code,
    /// TLS and restricted names never bind; and an `--as-needed` library is
    /// linked by taking a common, or else dropped with its symbols.
    #[test]
    fn commons_bind_like_gnu_ld() {
        use super::super::input::SharedLibSyms;
        let sym = |name: &str, shndx: u16, other: u8| Elf64Symbol {
            name_idx: 0,
            name: SymStr::new(name),
            info: (STB_GLOBAL << 4) | STT_OBJECT,
            other,
            shndx,
            value: 8,
            size: 4,
        };
        let object = |name: &str, symbols: Vec<Elf64Symbol>| {
            let obj = Elf64Object {
                sections: vec![],
                symbols,
                section_data: vec![],
                relocations: vec![],
                source_name: name.to_string(),
                e_flags: 0,
            };
            (name.to_string(), obj)
        };
        let export = |name: &str, binding: u8, kind: u8| DynSymbol {
            name: name.to_string(),
            info: (binding << 4) | kind,
            value: 0x1000,
            size: 4,
            in_exec_segment: false,
            version: None,
            is_default_ver: true,
        };
        let early = [
            "strong",
            "weak",
            "func",
            "tls",
            "hidden",
            "hidden_ref",
            "two",
            "kept_by",
            "absent",
        ];
        let mut commons: Vec<Elf64Symbol> = early.iter().map(|n| sym(n, SHN_COMMON, 0)).collect();
        commons[4].other = STV_HIDDEN;
        let mut objs = vec![
            object("c.o", commons),
            object("r.o", vec![sym("hidden_ref", SHN_UNDEF, STV_HIDDEN)]),
        ];
        let mut shared = SharedLibSyms::default();
        shared.expect_commons(&objs);
        let mut needed = Vec::new();
        let none = |_: &str| false;
        let libs = [
            ("libw.so", false, vec![export("two", STB_WEAK, STT_OBJECT)]),
            (
                "libl.so",
                false,
                vec![
                    export("strong", STB_GLOBAL, STT_OBJECT),
                    export("weak", STB_WEAK, STT_OBJECT),
                    export("func", STB_GLOBAL, STT_FUNC),
                    export("tls", STB_GLOBAL, STT_TLS),
                    export("hidden", STB_GLOBAL, STT_OBJECT),
                    export("hidden_ref", STB_GLOBAL, STT_OBJECT),
                    export("two", STB_GLOBAL, STT_OBJECT),
                    export("late_weak", STB_WEAK, STT_OBJECT),
                    export("late_func", STB_GLOBAL, STT_FUNC),
                    export("late_strong", STB_GLOBAL, STT_OBJECT),
                ],
            ),
            (
                "libn.so",
                true,
                vec![export("kept_by", STB_GLOBAL, STT_OBJECT)],
            ),
            (
                "libu.so",
                true,
                vec![export("unused", STB_GLOBAL, STT_OBJECT)],
            ),
        ];
        let linked: Vec<bool> = libs
            .into_iter()
            .map(|(soname, as_needed, syms)| {
                shared.link_library(soname, soname, syms, as_needed, none, None, &mut needed)
            })
            .collect();
        assert_eq!(linked, [true, true, true, false]);
        assert_eq!(needed, ["libw.so", "libl.so", "libn.so"]);
        assert!(
            shared.get("unused").is_none(),
            "a dropped library offers nothing"
        );
        assert_eq!(shared.provider_of("two"), Some("libl.so"));
        // An archive member read after the libraries.
        let late_from = objs.len();
        let late = ["late_weak", "late_func", "weak", "late_strong"];
        objs.push(object(
            "m.o",
            late.iter().map(|n| sym(n, SHN_COMMON, 0)).collect(),
        ));
        import_shared_commons(&mut objs, late_from, &shared);
        let imported = |k: usize| -> Vec<&str> {
            objs[k]
                .1
                .symbols
                .iter()
                .filter(|s| s.shndx == SHN_UNDEF)
                .map(|s| s.name.as_str())
                .collect()
        };
        assert_eq!(imported(0), ["strong", "two", "kept_by"]);
        assert_eq!(imported(2), ["late_strong"]);
        assert_eq!(objs[0].1.symbols[0].value, 0);
    }
}
