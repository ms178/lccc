//! Shared dynamic linking: symbol matching, library loading, and symbol registration.
//!
//! Extracts the duplicated shared-library symbol matching logic from x86 and ARM
//! linkers into a single generic implementation. Also provides `register_symbols_elf64()`
//! for populating the global symbol table from object files.

use crate::common::fx_hash::FxHashMap;
use std::path::Path;

use super::parse_shared::{parse_shared_library_symbols, parse_soname};
use super::resolve_lib::resolve_lib;
use super::symbols::{is_linker_defined_symbol, GlobalSymbolOps};
use super::types::{DynSymbol, Elf64Object};
use crate::backend::elf::{
    parse_linker_script_entries, LinkerScriptEntry, ELF_MAGIC, SHN_COMMON, STB_WEAK, STT_FILE,
    STT_OBJECT, STT_SECTION,
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
            if !existing.is_defined() && !existing.is_dynamic() {
                // An UNVERSIONED reference may only bind to the symbol's
                // default version (@@), matching GNU ld. Non-default exports
                // (memcpy@GLIBC_2.2.5) are reachable only via versioned refs.
                if dsym.is_default_ver {
                    lib_needed = true;
                    globals.insert(dsym.name.clone(), G::new_dynamic(dsym, soname));
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
/// to the real .so). Uses as-needed semantics: only adds DT_NEEDED if at least
/// one symbol was actually resolved.
pub fn load_shared_library_elf64<G: GlobalSymbolOps>(
    path: &str,
    globals: &mut FxHashMap<String, G>,
    needed_sonames: &mut Vec<String>,
    lib_paths: &[String],
) -> Result<(), String> {
    // Historical default: behave as if --as-needed were in effect.
    load_shared_library_elf64_as_needed(path, globals, needed_sonames, lib_paths, true)
}

/// As [`load_shared_library_elf64`], but with explicit `--as-needed` state.
///
/// `as_needed == false` (i.e. `--no-as-needed`, which is GNU ld's default)
/// records `DT_NEEDED` even when the library resolves nothing. That is not a
/// pessimisation to be optimised away: linking a library purely for the side
/// effects of its ELF constructors is a real and common pattern, and dropping
/// the entry silently changes program behaviour.
pub fn load_shared_library_elf64_as_needed<G: GlobalSymbolOps>(
    path: &str,
    globals: &mut FxHashMap<String, G>,
    needed_sonames: &mut Vec<String>,
    lib_paths: &[String],
    as_needed: bool,
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
                            globals,
                            needed_sonames,
                            lib_paths,
                            as_needed,
                        )?;
                    }
                }
                return Ok(());
            }
        }
        return Err(format!("{}: not a valid ELF shared library", path));
    }

    let soname = parse_soname(&data).unwrap_or_else(|| {
        Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string())
    });

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
    let lib_needed = match_shared_library_dynsyms(&dyn_syms, &soname, globals);
    if std::env::var_os("CCC_DEBUG_LD_SYMS").is_some() {
        eprintln!(
            "[LD-SO]   -> lib_needed={} (fdopen defined? {:?})",
            lib_needed,
            globals
                .get("fdopen")
                .map(|g| (g.is_defined(), g.is_dynamic()))
        );
    }

    if (lib_needed || !as_needed) && !needed_sonames.contains(&soname) {
        needed_sonames.push(soname);
    }
    Ok(())
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

/// Register symbols from an object file into the global symbol table.
///
/// Handles defined symbols, COMMON symbols, and undefined references.
/// For defined symbols, a GLOBAL definition replaces a WEAK one.
/// The `should_replace_extra` callback allows x86's linker to also check
/// `is_dynamic` when deciding whether to replace an existing symbol.
pub fn register_symbols_elf64<G: GlobalSymbolOps>(
    obj_idx: usize,
    obj: &Elf64Object,
    globals: &mut FxHashMap<String, G>,
    should_replace_extra: fn(existing: &G) -> bool,
) -> Result<(), String> {
    // PERF: pre-size the map for this object's symbols. Growth rehashes the
    // WHOLE table (re-hash every existing key string); with 40k-symbol
    // objects the doubling cascade dominated registration time.
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
                    if !e.is_defined()
                        || should_replace_extra(e)
                        || e_common
                        || (e_weak && sym.is_global())
                    {
                        *e = G::new_defined(obj_idx, sym);
                    } else if sym.is_global() && !e_weak && !e.is_dynamic() && e.is_defined() {
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
            match globals.get_mut(sym.name.as_str()) {
                None => {
                    globals.insert(sym.name.to_string(), G::new_common(obj_idx, sym));
                }
                Some(e) => {
                    if !e.is_defined() {
                        *e = G::new_common(obj_idx, sym);
                    } else if e.section_idx() == SHN_COMMON && sym.size > e.size() {
                        // COMMON vs COMMON: the largest instance wins (GNU semantics).
                        *e = G::new_common(obj_idx, sym);
                    }
                    // COMMON vs real definition: the real definition wins; ignore.
                }
            }
        } else if !globals.contains_key(sym.name.as_str()) {
            globals.insert(sym.name.to_string(), G::new_undefined(sym));
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
