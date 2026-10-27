//! Shared dynamic linking: symbol matching, library loading, and symbol registration.
//!
//! Extracts the duplicated shared-library symbol matching logic from x86 and ARM
//! linkers into a single generic implementation. Also provides `register_symbols_elf64()`
//! for populating the global symbol table from object files.

use crate::common::fx_hash::FxHashMap;
use std::path::Path;

use super::parse_shared::{parse_shared_library_symbols, parse_soname};
use super::resolve_lib::resolve_lib;
use super::symbols::{GlobalSymbolOps, is_linker_defined_symbol};
use super::types::{DynSymbol, Elf64Object};
use crate::backend::elf::{
    ELF_MAGIC, LinkerScriptEntry, SHN_COMMON, SHN_UNDEF, STB_WEAK, STT_FILE, STT_OBJECT,
    STT_SECTION, STV_DEFAULT, parse_linker_script_entries,
};

/// Match dynamic symbols from a shared library against undefined globals.
///
/// For each undefined, non-dynamic global that matches a library export:
/// 1. Replace it with a dynamic symbol entry (via `GlobalSymbolOps::new_dynamic`)
/// 2. Track WEAK STT_OBJECT matches for alias registration
///
/// After the first pass, a second pass registers any STT_OBJECT aliases at the
/// same (value, size) as matched WEAK symbols. This ensures COPY relocations
/// work correctly (e.g., `environ` is WEAK, `__environ` is GLOBAL in libc).
///
/// Returns `true` if at least one symbol was matched (i.e., this library is needed).
pub fn match_shared_library_dynsyms<G: GlobalSymbolOps>(
    dyn_syms: &[DynSymbol],
    soname: &str,
    globals: &mut FxHashMap<String, G>,
) -> bool {
    let mut lib_needed = false;
    let mut matched_weak_objects: Vec<(u64, u64)> = Vec::new();

    // Versioned undefined references ("name@VER" — produced by `.symver
    // real, name@VER` directives, e.g. glibc's compat_symbol_reference) must
    // match the library's BARE dynsym name when the library exports that
    // symbol with a matching version. GNU ld binds such references to the
    // requested version; without this, "printf@GLIBC_2.2.5" stays undefined
    // even though libc.so.6 exports "printf" (as `printf@@GLIBC_2.2.5`).
    // "name@@VER" is a definition form, never an undefined ref, so only
    // single-'@' names are considered here.
    let versioned_refs: Vec<(String, String, String)> = globals
        .iter()
        .filter(|(name, s)| !s.is_defined() && !s.is_dynamic() && name.contains('@'))
        .filter_map(|(name, _)| {
            let at = name.find('@')?;
            if name[at + 1..].contains('@') {
                return None; // "@@": definition form, not a reference
            }
            let base = &name[..at];
            let ver = &name[at + 1..];
            if base.is_empty() || ver.is_empty() {
                return None;
            }
            Some((base.to_string(), ver.to_string(), name.clone()))
        })
        .collect();

    // First pass: match undefined symbols against library exports
    for dsym in dyn_syms {
        if let Some(existing) = globals.get(&dsym.name) {
            // A regular COMMON yields to a strong data definition (see
            // `shared_def_overrides_common`); it becomes an import.
            let binds_common = existing.section_idx() == SHN_COMMON
                && !existing.is_dynamic()
                && existing.visibility() == STV_DEFAULT
                && super::symbols::shared_def_overrides_common(dsym.info);
            if (!existing.is_defined() && !existing.is_dynamic()) || binds_common {
                // An UNVERSIONED reference may only bind to the symbol's
                // default version (@@), matching GNU ld. Non-default exports
                // (memcpy@GLIBC_2.2.5) are reachable only via versioned refs.
                if dsym.is_default_ver {
                    lib_needed = true;
                    let vis = existing.visibility();
                    let mut bound = G::new_dynamic(dsym, soname);
                    bound.set_visibility(vis);
                    globals.insert(dsym.name.clone(), bound);
                    // Track WEAK STT_OBJECT for alias detection
                    let bind = dsym.info >> 4;
                    let stype = dsym.info & 0xf;
                    if bind == STB_WEAK
                        && stype == STT_OBJECT
                        && !matched_weak_objects.contains(&(dsym.value, dsym.size))
                    {
                        matched_weak_objects.push((dsym.value, dsym.size));
                    }
                }
            }
        }
        // Versioned reference matching: "base@VER" binds to the export
        // (base) when the export carries exactly the requested version —
        // including non-default versions (memcpy@GLIBC_2.2.5).
        for (base, ver, full) in &versioned_refs {
            if *base == dsym.name && dsym.version.as_deref() == Some(ver.as_str()) {
                if let Some(existing) = globals.get(full) {
                    if !existing.is_defined() && !existing.is_dynamic() {
                        lib_needed = true;
                        globals.insert(full.clone(), G::new_dynamic(dsym, soname));
                    }
                }
            }
        }
    }

    // Second pass: register aliases for matched WEAK STT_OBJECT symbols
    if !matched_weak_objects.is_empty() {
        for dsym in dyn_syms {
            let stype = dsym.info & 0xf;
            if stype == STT_OBJECT
                && matched_weak_objects.contains(&(dsym.value, dsym.size))
                && !globals.contains_key(&dsym.name)
            {
                lib_needed = true;
                globals.insert(dsym.name.clone(), G::new_dynamic(dsym, soname));
            }
        }
    }

    lib_needed
}

/// Load a shared library file and match its exports against undefined globals.
///
/// Handles linker script indirection (e.g., libc.so may be a text file pointing
/// to the real .so).
///
/// `as_needed == false` (i.e. `--no-as-needed`, which is GNU ld's default)
/// records `DT_NEEDED` even when the library resolves nothing. That is not a
/// pessimisation to be optimised away: linking a library purely for the side
/// effects of its ELF constructors is a real and common pattern, and dropping
/// the entry silently changes program behaviour.
pub fn load_shared_library_elf64_as_needed<G: GlobalSymbolOps>(
    path: &str,
    objects: &mut super::ObjectSet,
    globals: &mut FxHashMap<String, G>,
    needed_sonames: &mut Vec<String>,
    lib_paths: &[String],
    as_needed: bool,
    by_search: bool,
) -> Result<(), String> {
    let data = std::fs::read(path).map_err(|e| format!("failed to read '{}': {}", path, e))?;

    // Handle linker scripts (e.g., libc.so is often a text file with GROUP/INPUT)
    if data.len() >= 4 && data[0..4] != ELF_MAGIC {
        if let Ok(text) = std::str::from_utf8(&data) {
            if let Some(entries) = parse_linker_script_entries(text) {
                let script_dir = Path::new(path)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string());
                for entry in &entries {
                    let resolved_path = match entry {
                        LinkerScriptEntry::Path(lib_path) => {
                            if Path::new(lib_path).exists() {
                                Some(lib_path.clone())
                            } else if let Some(ref dir) = script_dir {
                                let p = format!("{}/{}", dir, lib_path);
                                if Path::new(&p).exists() {
                                    Some(p)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        }
                        LinkerScriptEntry::Lib(lib_name) => resolve_lib(lib_name, lib_paths, false),
                    };
                    let searched = matches!(entry, LinkerScriptEntry::Lib(_));
                    if let Some(resolved) = resolved_path {
                        let lib_data = std::fs::read(&resolved)
                            .map_err(|e| format!("failed to read '{}': {}", resolved, e))?;
                        if lib_data.len() >= 8 && &lib_data[0..8] == b"!<arch>\n" {
                            // Archives in linker scripts (like libc_nonshared.a)
                            // are silently skipped during shared lib loading
                            continue;
                        }
                        load_shared_library_elf64_as_needed(
                            &resolved,
                            objects,
                            globals,
                            needed_sonames,
                            lib_paths,
                            as_needed,
                            searched,
                        )?;
                    }
                }
                return Ok(());
            }
        }
        return Err(format!("{}: not a valid ELF shared library", path));
    }

    let soname = parse_soname(&data).unwrap_or_else(|| unsonamed_needed_name(path, by_search));

    let dyn_syms = parse_shared_library_symbols(&data, path)?;
    if std::env::var_os("CCC_DEBUG_LD_SYMS").is_some() {
        eprintln!(
            "[LD-SO] load path={} soname={} as_needed={} dynsyms={}",
            path,
            soname,
            as_needed,
            dyn_syms.len()
        );
    }
    // In the link iff `--no-as-needed`, already in the DT_NEEDED list by
    // an earlier input, or needed here.  Decided before binding anything:
    // a library `--as-needed` drops is unloaded, symbols included, so not
    // even a weak reference may bind to it.
    let in_link =
        !as_needed || needed_sonames.contains(&soname) || shared_library_needed(&dyn_syms, globals);
    if std::env::var_os("CCC_DEBUG_LD_SYMS").is_some() {
        eprintln!("[LD-SO]   -> in_link={in_link}");
    }
    if in_link {
        match_shared_library_dynsyms(&dyn_syms, &soname, globals);
        objects.note_linked_library(path, &soname, dyn_syms);
        if !needed_sonames.contains(&soname) {
            needed_sonames.push(soname);
        }
    }
    Ok(())
}

/// Whether an `--as-needed` library with exports `dyn_syms` is needed where
/// it is read: it defines what a regular object references -- by the
/// binding rules of [`match_shared_library_dynsyms`] -- or strong data a
/// common yields to.  Only a non-weak reference counts (bfd's
/// `ref_regular_nonweak`): a weak one binds to a library in the link but
/// never brings one in.
fn shared_library_needed<G: GlobalSymbolOps>(
    dyn_syms: &[DynSymbol],
    globals: &FxHashMap<String, G>,
) -> bool {
    let strong_ref = |g: &G| !g.is_defined() && !g.is_dynamic() && g.info() >> 4 != STB_WEAK;
    let takes = |dsym: &DynSymbol, g: &G| {
        strong_ref(g)
            || (g.section_idx() == SHN_COMMON
                && !g.is_dynamic()
                && g.visibility() == STV_DEFAULT
                && super::symbols::shared_def_overrides_common(dsym.info))
    };
    if dyn_syms
        .iter()
        .filter(|dsym| dsym.is_default_ver)
        .any(|dsym| globals.get(&dsym.name).is_some_and(|g| takes(dsym, g)))
    {
        return true;
    }
    // `name@VER` references, as the matcher pairs them with exports.
    let versioned: Vec<(&str, &str)> = globals
        .iter()
        .filter(|(_, g)| strong_ref(g))
        .filter_map(|(name, _)| {
            let (base, ver) = name.split_once('@')?;
            (!base.is_empty() && !ver.is_empty() && !ver.contains('@')).then_some((base, ver))
        })
        .collect();
    !versioned.is_empty()
        && dyn_syms.iter().any(|dsym| {
            versioned
                .iter()
                .any(|&(base, ver)| dsym.name == base && dsym.version.as_deref() == Some(ver))
        })
}

/// The `DT_NEEDED` name GNU ld records for the shared library at `path` when
/// it has no `DT_SONAME`: the name it was given by -- the file name when an
/// `-l` search found it (on the command line or in a linker script), else
/// the path itself, as named on the command line or as a linker-script
/// entry resolved (bfd, lld and mold agree; recording only the file name
/// made `lccc m.c /opt/x/libfoo.so` need `libfoo.so` on the loader's search
/// path, where the GNU-linked program finds it where it was linked).
pub fn unsonamed_needed_name(path: &str, by_search: bool) -> String {
    if !by_search {
        return path.to_string();
    }
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// Resolve remaining undefined symbols by searching default system libraries.
///
/// After all explicit -l libraries have been loaded, this function searches
/// the standard system libraries (libc, libm, libgcc_s) for any remaining
/// undefined, non-weak, non-linker-defined symbols.
///
/// `lib_search_paths` provides directories to search for the default libs.
/// `default_lib_names` lists the .so filenames to try (e.g., ["libc.so.6"]).
pub fn resolve_dynamic_symbols_elf64<G: GlobalSymbolOps>(
    globals: &mut FxHashMap<String, G>,
    needed_sonames: &mut Vec<String>,
    lib_search_paths: &[String],
    default_lib_names: &[&str],
) -> Result<(), String> {
    // Check if there are any truly undefined symbols worth resolving
    let has_undefined = globals.iter().any(|(name, sym)| {
        !sym.is_defined() && !sym.is_dynamic() && !is_linker_defined_symbol(name)
    });
    if !has_undefined {
        return Ok(());
    }

    // Find default libraries in the search paths
    for lib_name in default_lib_names {
        let lib_path = lib_search_paths
            .iter()
            .map(|dir| format!("{}/{}", dir, lib_name))
            .find(|candidate| Path::new(candidate).exists());

        if let Some(lib_path) = lib_path {
            let data = match std::fs::read(&lib_path) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let soname = parse_soname(&data).unwrap_or_else(|| {
                Path::new(&lib_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            });
            let dyn_syms = match parse_shared_library_symbols(&data, &lib_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let lib_needed = match_shared_library_dynsyms(&dyn_syms, &soname, globals);
            if lib_needed && !needed_sonames.contains(&soname) {
                needed_sonames.push(soname);
            }
        }
    }
    Ok(())
}

/// `*e = new`, keeping the visibility narrowed so far (see the end of the
/// `register_symbols_elf64` loop).
fn replace_keeping_visibility<G: GlobalSymbolOps>(e: &mut G, new: G) {
    let vis = super::symbols::most_constraining_visibility(e.visibility(), new.visibility());
    *e = new;
    e.set_visibility(vis);
}

/// Bind `dsym`'s name to that export of `soname` as
/// [`match_shared_library_dynsyms`] binds a reference: a WEAK object brings
/// its same-address aliases (`environ` and `__environ`) along, so that a copy
/// relocation moves them together.
fn import_shared_data<G: GlobalSymbolOps>(
    globals: &mut FxHashMap<String, G>,
    soname: &str,
    dsym: &DynSymbol,
    exports: &[DynSymbol],
) {
    globals.insert(dsym.name.clone(), G::new_dynamic(dsym, soname));
    if dsym.info >> 4 != STB_WEAK || dsym.info & 0xf != STT_OBJECT {
        return;
    }
    for alias in exports {
        if alias.info & 0xf == STT_OBJECT
            && (alias.value, alias.size) == (dsym.value, dsym.size)
            && !globals.contains_key(&alias.name)
        {
            globals.insert(alias.name.clone(), G::new_dynamic(alias, soname));
        }
    }
}

/// Register symbols from an object file into the global symbol table.
///
/// `prior` holds the objects registered before `obj`, in link order; `obj`
/// becomes object `prior.len()`. Handles defined symbols, COMMON symbols,
/// and undefined references. For defined symbols, a GLOBAL definition
/// replaces a WEAK one. The `should_replace_extra` callback allows x86's
/// linker to also check `is_dynamic` when deciding whether to replace an
/// existing symbol. A definition in a discarded COMDAT copy never displaces
/// another (see `comdat::in_discarded_group`).
pub fn register_symbols_elf64<G: GlobalSymbolOps>(
    prior: &super::ObjectSet,
    obj: &Elf64Object,
    globals: &mut FxHashMap<String, G>,
    should_replace_extra: fn(existing: &G) -> bool,
) -> Result<(), String> {
    // PERF: pre-size the map for this object's symbols. Growth rehashes the
    // WHOLE table (re-hash every existing key string); with 40k-symbol
    // objects the doubling cascade dominated registration time.
    let obj_idx = prior.len();
    globals.reserve(obj.symbols.len());
    // PERF: single-lookup registration via get_mut + in-place replacement.
    // The old shape did `globals.get(&name)` followed by
    // `globals.insert(name.clone(), ..)` — two hashes of the string PLUS a
    // key clone on EVERY definition, dominating link time for symbol-heavy
    // objects (big-text profile: 21.5 ms of a 66 ms link inside this loop).
    // Replacement through `*e = ..` reuses the existing key (no clone, one
    // hash); only genuinely new symbols pay the clone+insert.
    for sym in &obj.symbols {
        if sym.sym_type() == STT_SECTION || sym.sym_type() == STT_FILE {
            continue;
        }
        if sym.name.is_empty() || sym.is_local() {
            continue;
        }

        let is_defined = !sym.is_undefined() && sym.shndx != SHN_COMMON;

        if is_defined {
            match globals.get_mut(sym.name.as_str()) {
                None => {
                    globals.insert(sym.name.to_string(), G::new_defined(obj_idx, sym));
                }
                Some(e) => {
                    let e_weak = e.info() >> 4 == STB_WEAK;
                    // A tentative (COMMON) definition is superseded by any
                    // real definition (GNU ld / mold behavior).
                    let e_common = e.section_idx() == SHN_COMMON;
                    // Only asked where this definition would displace or
                    // clash with an existing one (see the doc comment).
                    let discarded_copy =
                        || super::comdat::in_discarded_group(prior, obj, sym.shndx as usize);
                    if !e.is_defined() || should_replace_extra(e) {
                        replace_keeping_visibility(e, G::new_defined(obj_idx, sym));
                    } else if (e_common || (e_weak && sym.is_global())) && !discarded_copy() {
                        replace_keeping_visibility(e, G::new_defined(obj_idx, sym));
                    } else if sym.is_global()
                        && !e_weak
                        && !e.is_dynamic()
                        && e.is_defined()
                        && !discarded_copy()
                    {
                        // Two strong definitions of the same symbol: this is a
                        // hard error in every mainstream linker. Silently keeping
                        // the first definition produces subtly wrong programs.
                        return Err(format!(
                            "multiple definition of '{}' (duplicate in {})",
                            sym.name, obj.source_name
                        ));
                    }
                    // else: new definition is weak and existing is strong; keep existing.
                }
            }
        } else if sym.shndx == SHN_COMMON {
            // A strong data definition of a library linked before this
            // object (`./lib.so c.o`) takes the name over, as one linked
            // after it does; the common becomes an import
            // (`shared_def_overrides_common`).
            let earlier_dso = if sym.visibility() == STV_DEFAULT {
                prior.linked_shared_data(sym.name.as_str())
            } else {
                None
            };
            let mut import = None;
            match globals.get_mut(sym.name.as_str()) {
                None if earlier_dso.is_some() => import = earlier_dso,
                None => {
                    globals.insert(sym.name.to_string(), G::new_common(obj_idx, sym));
                }
                Some(e) => {
                    // A strong shared-library data definition bound earlier
                    // keeps the name; a weak one yields to the common
                    // (`shared_def_overrides_common`).
                    let keeps = e.is_dynamic()
                        && sym.visibility() == STV_DEFAULT
                        && e.visibility() == STV_DEFAULT
                        && super::symbols::shared_def_overrides_common(e.info());
                    if !e.is_defined() && !keeps {
                        // A reference registered since that library was
                        // loaded is bound to it just the same; one already
                        // bound to another export (a function) is not.
                        if !e.is_dynamic() && e.visibility() == STV_DEFAULT && earlier_dso.is_some()
                        {
                            import = earlier_dso;
                        } else {
                            replace_keeping_visibility(e, G::new_common(obj_idx, sym));
                        }
                    } else if e.section_idx() == SHN_COMMON && sym.size > e.size() {
                        // COMMON vs COMMON: the largest instance wins (GNU semantics).
                        replace_keeping_visibility(e, G::new_common(obj_idx, sym));
                    }
                    // COMMON vs real definition: the real definition wins; ignore.
                }
            }
            if let Some((soname, dsym, exports)) = import {
                import_shared_data(globals, soname, dsym, exports);
            }
        } else {
            // Undefined reference (SHN_UNDEF, not COMMON). The first
            // registrant wins, but a strong reference upgrades an existing
            // weak-only entry: a member pulled in for a weak reference must
            // not shadow a later strong demand for the same symbol. Archive
            // extraction keys off this binding (see
            // member_resolves_undefined_generic): weak-only entries never
            // pull, strong ones always do.
            let weak_ref = sym.info >> 4 == STB_WEAK;
            match globals.get_mut(sym.name.as_str()) {
                // The first reference to a name a library linked earlier
                // exports binds to it (`./lib.so main.o`): GNU ld enters a
                // kept library's exports into the symbol table when it
                // reads the library, so a later archive member defining the
                // name is not extracted either.
                None => match prior.linked_shared_export(sym.name.as_str()) {
                    Some((soname, dsym, exports)) if dsym.name == sym.name.as_str() => {
                        import_shared_data(globals, soname, dsym, exports);
                    }
                    Some((soname, dsym, _)) => {
                        globals.insert(sym.name.to_string(), G::new_dynamic(dsym, soname));
                    }
                    None => {
                        globals.insert(sym.name.to_string(), G::new_undefined(sym));
                    }
                },
                Some(e)
                    if !weak_ref && e.section_idx() == SHN_UNDEF && e.info() >> 4 == STB_WEAK =>
                {
                    replace_keeping_visibility(e, G::new_undefined(sym));
                }
                _ => {}
            }
        }
        // Narrow the visibility now rather than only in the late
        // `merge_object_visibility` pass (same result: the merge is
        // commutative and idempotent), so binding a COMMON to a shared
        // library can tell a hidden or protected one, which never binds.
        let vis = sym.visibility();
        if vis != STV_DEFAULT
            && let Some(e) = globals.get_mut(sym.name.as_str())
        {
            e.set_visibility(super::symbols::most_constraining_visibility(
                e.visibility(),
                vis,
            ));
        }
    }

    // `.symver real, public@@VERSION` creates a versioned definition but
    // ordinary references inside the same DSO remain spelled `public`.
    // GNU ld binds that unversioned reference to the default (`@@`) version;
    // leaving only the composed name in the global table makes it look
    // undefined and forces an accidental host-libc dependency. Install a
    // non-versioned alias for every default-version definition after the main
    // pass, so it also works when the unversioned reference appeared earlier
    // in this object. Non-default `@VERSION` entries intentionally do not get
    // an alias.
    for sym in &obj.symbols {
        if sym.sym_type() == STT_SECTION
            || sym.sym_type() == STT_FILE
            || sym.name.is_empty()
            || sym.is_local()
            || sym.is_undefined()
        {
            continue;
        }
        let full_name = sym.name.to_string();
        let Some(pos) = full_name.find("@@") else {
            continue;
        };
        let base = &full_name[..pos];
        if base.is_empty() {
            continue;
        }
        let alias = G::new_defined(obj_idx, sym);
        match globals.get_mut(base) {
            None => {
                globals.insert(base.to_string(), alias);
            }
            Some(existing)
                if !existing.is_defined()
                    || should_replace_extra(existing)
                    || (existing.info() >> 4 == STB_WEAK) =>
            {
                *existing = alias;
            }
            Some(_) => {}
        }
    }
    Ok(())
}

/// Alignment for the executable's copy of a copy-relocated library object.
///
/// The object's alignment is not recorded anywhere, but it is bounded twice
/// (lld's rule): the library's layout honoured it, so it divides the
/// object's address there (`lib_value`), and an object's size is a multiple
/// of its alignment. The smaller power of two satisfies both, so an
/// `_Alignas(16)` or `_Atomic __int128` variable keeps the alignment that
/// its `movaps`/`cmpxchg16b` accesses require. The result is capped at a
/// page and is never below `floor`.
pub fn copy_reloc_align(lib_value: u64, size: u64, floor: u64) -> u64 {
    const MAX: u64 = 4096;
    let by_value = if lib_value == 0 {
        MAX
    } else {
        1u64 << lib_value.trailing_zeros().min(12)
    };
    let by_size = size.max(1).checked_next_power_of_two().unwrap_or(MAX);
    by_value.min(by_size).min(MAX).max(floor)
}

#[cfg(test)]
mod copy_align_tests {
    use super::copy_reloc_align;

    #[test]
    fn copy_alignment_is_bounded_by_library_address_and_size() {
        // 16-byte object at a 16-aligned address: 16, not the old floor.
        assert_eq!(copy_reloc_align(0x4010, 16, 4), 16);
        // Same object at an address only 8-aligned: the library proves 8.
        assert_eq!(copy_reloc_align(0x4008, 16, 4), 8);
        // A 4-byte int at a page-aligned address needs no more than 4.
        assert_eq!(copy_reloc_align(0x5000, 4, 4), 4);
        // Size 12 (struct of 3 ints) rounds to 16; address limits it to 4.
        assert_eq!(copy_reloc_align(0x4004, 12, 1), 4);
        // Huge page-aligned array: capped at the page.
        assert_eq!(copy_reloc_align(0x10_0000, 1 << 20, 8), 4096);
        // The floor wins over weaker bounds (old placement is preserved).
        assert_eq!(copy_reloc_align(0x4001, 1, 8), 8);
        assert_eq!(copy_reloc_align(0, 0, 1), 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::elf::{STB_GLOBAL, STT_FUNC};
    use crate::backend::linker_common::{Elf64Symbol, SymStr};
    use crate::backend::x86::linker::types::GlobalSymbol;

    fn symbol(name: &str, binding: u8, sym_type: u8, shndx: u16) -> Elf64Symbol {
        Elf64Symbol {
            name_idx: 0,
            name: SymStr::new(name),
            info: (binding << 4) | sym_type,
            other: 0,
            shndx,
            value: 0,
            size: 4,
        }
    }

    fn export(name: &str, info: u8, version: Option<&str>, default_ver: bool) -> DynSymbol {
        DynSymbol {
            name: name.to_string(),
            info,
            value: 0x1000,
            size: 4,
            in_exec_segment: false,
            version: version.map(str::to_string),
            is_default_ver: default_ver,
        }
    }

    /// An `--as-needed` library is needed by a non-weak reference to what it
    /// exports (the default version for a plain name, the named version for
    /// `name@VER`) or by a common its strong data takes -- never by a weak
    /// reference alone, nor by a common its weak data or code meets (bfd
    /// 2.47, `tests/linker` `positional_shared_operands` and
    /// `common_binds_shared_data`).
    #[test]
    fn only_non_weak_uses_need_an_as_needed_library() {
        let func = (STB_GLOBAL << 4) | STT_FUNC;
        let data = (STB_GLOBAL << 4) | STT_OBJECT;
        let weak_data = (STB_WEAK << 4) | STT_OBJECT;
        let needed = |globals: &[(&str, GlobalSymbol)], exports: &[DynSymbol]| {
            let globals: FxHashMap<String, GlobalSymbol> = globals
                .iter()
                .map(|(n, g)| (n.to_string(), g.clone()))
                .collect();
            shared_library_needed(exports, &globals)
        };
        let undef =
            |binding| GlobalSymbol::new_undefined(&symbol("f", binding, STT_FUNC, SHN_UNDEF));
        let common = GlobalSymbol::new_common(0, &symbol("s", STB_GLOBAL, STT_OBJECT, SHN_COMMON));
        let foo = [export("foo", func, None, true)];

        assert!(needed(&[("foo", undef(STB_GLOBAL))], &foo));
        assert!(!needed(&[("foo", undef(STB_WEAK))], &foo));
        assert!(!needed(&[("bar", undef(STB_GLOBAL))], &foo));
        // A non-default version answers only `name@VER`.
        let versioned = [export("foo", func, Some("V1"), false)];
        assert!(!needed(&[("foo", undef(STB_GLOBAL))], &versioned));
        assert!(needed(&[("foo@V1", undef(STB_GLOBAL))], &versioned));
        assert!(!needed(&[("foo@V1", undef(STB_WEAK))], &versioned));
        assert!(!needed(&[("foo@V2", undef(STB_GLOBAL))], &versioned));
        // Commons: strong data takes one; weak data and code do not.
        assert!(needed(
            &[("s", common.clone())],
            &[export("s", data, None, true)]
        ));
        assert!(!needed(
            &[("s", common.clone())],
            &[export("s", weak_data, None, true)]
        ));
        assert!(!needed(&[("s", common)], &[export("s", func, None, true)]));
    }
}
