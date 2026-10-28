//! Phase 3: Global symbol table construction.
//!
//! Builds the global symbol table from input objects, handles COMMON symbols,
//! marks PLT/GOT needs for dynamic linking, and identifies GOT entries for
//! PC-relative GOT references.

use super::elf_read::*;
use super::relocations::{
    GlobalSym, MergedSection, R_RISCV_GOT_HI20, R_RISCV_TLS_GD_HI20, R_RISCV_TLS_GOT_HI20,
    got_sym_key,
};
use crate::backend::elf::STV_DEFAULT;
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

/// Build the global symbol table from all input objects.
///
/// For each non-local symbol, creates or updates a `GlobalSym` entry.
/// Handles SHN_UNDEF (undefined), SHN_ABS (absolute), SHN_COMMON (common/BSS),
/// and defined symbols. Weak-to-strong overrides are applied.
pub fn build_global_symbols(
    input_objs: &[(String, ElfObject)],
    sec_mapping: &FxHashMap<(usize, usize), (usize, u64)>,
    merged_sections: &mut Vec<MergedSection>,
    merged_map: &mut FxHashMap<String, usize>,
) -> FxHashMap<String, GlobalSym> {
    let mut global_syms: FxHashMap<String, GlobalSym> = FxHashMap::default();

    for (obj_idx, (_, obj)) in input_objs.iter().enumerate() {
        for sym in &obj.symbols {
            if sym.name.is_empty() || sym.binding() == STB_LOCAL {
                continue;
            }

            if sym.shndx == SHN_UNDEF {
                let entry = global_syms
                    .entry(sym.name.to_string())
                    .or_insert_with(|| GlobalSym {
                        value: 0,
                        size: 0,
                        binding: sym.binding(),
                        sym_type: sym.sym_type(),
                        visibility: sym.visibility(),
                        defined: false,
                        needs_plt: false,
                        plt_idx: 0,
                        got_offset: None,
                        section_idx: None,
                    });
                // A weak reference seen first must not make a later strong
                // unresolved reference optional (including hidden symbols).
                if !entry.defined && sym.binding() == STB_GLOBAL {
                    entry.binding = STB_GLOBAL;
                }
                continue;
            }

            if sym.shndx == SHN_ABS {
                let entry = global_syms
                    .entry(sym.name.to_string())
                    .or_insert_with(|| GlobalSym {
                        value: sym.value,
                        size: sym.size,
                        binding: sym.binding(),
                        sym_type: sym.sym_type(),
                        visibility: sym.visibility(),
                        defined: true,
                        needs_plt: false,
                        plt_idx: 0,
                        got_offset: None,
                        section_idx: None,
                    });
                if !entry.defined || (entry.binding == STB_WEAK && sym.binding() == STB_GLOBAL) {
                    entry.value = sym.value;
                    entry.size = sym.size;
                    entry.binding = sym.binding();
                    entry.sym_type = sym.sym_type();
                    entry.defined = true;
                    // An absolute definition replacing a weak section symbol
                    // no longer receives the old section's load address.
                    entry.section_idx = None;
                }
                continue;
            }

            if sym.shndx == SHN_COMMON {
                allocate_common_symbol(sym, &mut global_syms, merged_sections, merged_map);
                continue;
            }

            let sec_idx = sym.shndx as usize;
            let (merged_idx, offset) = match sec_mapping.get(&(obj_idx, sec_idx)) {
                Some(&v) => v,
                None => continue,
            };

            let entry = global_syms
                .entry(sym.name.to_string())
                .or_insert_with(|| GlobalSym {
                    value: 0,
                    size: sym.size,
                    binding: sym.binding(),
                    sym_type: sym.sym_type(),
                    visibility: sym.visibility(),
                    defined: false,
                    needs_plt: false,
                    plt_idx: 0,
                    got_offset: None,
                    section_idx: None,
                });

            if entry.defined && !(entry.binding == STB_WEAK && sym.binding() == STB_GLOBAL) {
                continue;
            }

            entry.value = offset + sym.value;
            entry.size = sym.size;
            entry.binding = sym.binding();
            entry.sym_type = sym.sym_type();
            entry.visibility = sym.visibility();
            entry.defined = true;
            entry.section_idx = Some(merged_idx);
        }
    }

    // The gABI combines visibility from ALL definitions and references, not
    // just the winning definition. A hidden reference in a second TU must
    // constrain a default-visible definition in either input order.
    for (_, obj) in input_objs {
        for sym in &obj.symbols {
            if sym.binding() != STB_LOCAL && sym.visibility() != STV_DEFAULT {
                if let Some(global) = global_syms.get_mut(sym.name.as_str()) {
                    global.visibility = crate::backend::linker_common::most_constraining_visibility(
                        global.visibility,
                        sym.visibility(),
                    );
                }
            }
        }
    }
    global_syms
}

/// Allocate a COMMON symbol in .bss.
fn allocate_common_symbol(
    sym: &Symbol,
    global_syms: &mut FxHashMap<String, GlobalSym>,
    merged_sections: &mut Vec<MergedSection>,
    merged_map: &mut FxHashMap<String, usize>,
) {
    let bss_idx = *merged_map.entry(".bss".into()).or_insert_with(|| {
        let idx = merged_sections.len();
        merged_sections.push(MergedSection {
            name: ".bss".into(),
            sh_type: SHT_NOBITS,
            sh_flags: SHF_ALLOC | SHF_WRITE,
            data: Vec::new(),
            vaddr: 0,
            align: 8,
        });
        idx
    });
    let ms = &mut merged_sections[bss_idx];
    let align = sym.value.max(1) as usize; // st_value is alignment for COMMON
    let cur = ms.data.len();
    let aligned = (cur + align - 1) & !(align - 1);
    ms.data.resize(aligned, 0);
    let off = ms.data.len() as u64;
    ms.data.resize(ms.data.len() + sym.size as usize, 0);
    ms.align = ms.align.max(align as u64);

    let entry = global_syms
        .entry(sym.name.to_string())
        .or_insert_with(|| GlobalSym {
            value: off,
            size: sym.size,
            binding: sym.binding(),
            sym_type: STT_OBJECT,
            visibility: sym.visibility(),
            defined: true,
            needs_plt: false,
            plt_idx: 0,
            got_offset: None,
            section_idx: Some(bss_idx),
        });
    if !entry.defined || (entry.binding == STB_WEAK && sym.binding() == STB_GLOBAL) {
        entry.value = off;
        entry.size = sym.size.max(entry.size);
        entry.binding = sym.binding();
        entry.defined = true;
        entry.section_idx = Some(bss_idx);
    }
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

/// Identify GOT entries needed by scanning for GOT_HI20 and TLS GOT relocations.
///
/// Returns the ordered list of GOT symbol keys, the set of TLS GOT symbols,
/// and a map of local GOT symbol info for resolving local GOT entries.
pub fn collect_got_entries(
    input_objs: &[(String, ElfObject)],
) -> (
    Vec<String>,
    FxHashSet<String>,
    FxHashMap<String, (usize, usize, i64)>,
) {
    let mut got_symbols: Vec<String> = Vec::new();
    let mut tls_got_symbols: FxHashSet<String> = FxHashSet::default();
    let mut local_got_sym_info: FxHashMap<String, (usize, usize, i64)> = FxHashMap::default();

    let mut got_set: FxHashSet<String> = FxHashSet::default();
    for (obj_idx, (_, obj)) in input_objs.iter().enumerate() {
        for relocs in &obj.relocations {
            for reloc in relocs {
                if reloc.rela_type == R_RISCV_GOT_HI20
                    || reloc.rela_type == R_RISCV_TLS_GOT_HI20
                    || reloc.rela_type == R_RISCV_TLS_GD_HI20
                {
                    let sym = &obj.symbols[reloc.sym_idx as usize];
                    let (name, is_local) = got_sym_key(obj_idx, sym, reloc.addend);
                    if !name.is_empty() && !got_set.contains(&name) {
                        got_set.insert(name.clone());
                        got_symbols.push(name.clone());
                        if is_local {
                            local_got_sym_info.insert(
                                name.clone(),
                                (obj_idx, reloc.sym_idx as usize, reloc.addend),
                            );
                        }
                    }
                    if reloc.rela_type == R_RISCV_TLS_GOT_HI20
                        || reloc.rela_type == R_RISCV_TLS_GD_HI20
                    {
                        tls_got_symbols.insert(name);
                    }
                }
            }
        }
    }

    (got_symbols, tls_got_symbols, local_got_sym_info)
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
