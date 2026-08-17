//! x86-64 linker orchestration.
//!
//! Contains the two public entry points (`link_builtin` and `link_shared`) that
//! orchestrate the linking pipeline: load inputs, resolve symbols, merge sections,
//! build PLT/GOT, and dispatch to the appropriate ELF emission path.

use crate::backend::linker_common::SymStr;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use std::path::Path;

use super::elf::*;
use super::types::GlobalSymbol;
use super::input::load_file;
use super::plt_got::{collect_ifunc_symbols, create_plt_got};
use super::emit_exec::emit_executable;
use super::icf;
use super::emit_shared::emit_shared_library;
use crate::backend::linker_common::{self, OutputSection};

pub fn link_builtin(
    object_files: &[&str],
    output_path: &str,
    user_args: &[String],
    lib_paths: &[&str],
    needed_libs: &[&str],
    crt_objects_before: &[&str],
    crt_objects_after: &[&str],
) -> Result<(), String> {
    let is_static = user_args.iter().any(|a| a == "-static");
    let t_all = std::time::Instant::now();
    let ld_time = std::env::var("LCCC_LD_TIME").is_ok();
    let mut t_phase = std::time::Instant::now();
    macro_rules! phase { ($name:expr) => { if ld_time { eprintln!("[ldtime] {:<24} {:>7.1} ms", $name, t_phase.elapsed().as_secs_f64()*1e3); t_phase = std::time::Instant::now(); } } }
    let mut objects: Vec<ElfObject> = Vec::new();
    let mut globals: FxHashMap<String, GlobalSymbol> = FxHashMap::default();
    let mut needed_sonames: Vec<String> = Vec::new();
    let lib_path_strings: Vec<String> = lib_paths.iter().map(|s| s.to_string()).collect();

    // Load CRT objects before user objects
    for path in crt_objects_before {
        if Path::new(path).exists() {
            load_file(path, &mut objects, &mut globals, &mut needed_sonames, &lib_path_strings, false)?;
        }
    }

    // Load user object files
    for path in object_files {
        load_file(path, &mut objects, &mut globals, &mut needed_sonames, &lib_path_strings, false)?;
    }

    phase!("load-inputs");
    // Parse user args using shared infrastructure
    let parsed_args = linker_common::parse_linker_args(user_args);
    let extra_lib_paths = parsed_args.extra_lib_paths;
    let libs_to_load = parsed_args.libs_to_load;
    let extra_object_files = parsed_args.extra_object_files;
    let export_dynamic = parsed_args.export_dynamic;
    let rpath_entries = parsed_args.rpath_entries;
    let use_runpath = parsed_args.use_runpath;
    let defsym_defs = parsed_args.defsym_defs;
    let gc_sections = parsed_args.gc_sections;
    // Command line wins; LCCC_LD_ICF is the fallback so the feature can be
    // exercised without touching build systems.
    let icf_mode: Option<String> = parsed_args
        .icf
        .clone()
        .or_else(|| icf::icf_mode_from_env().map(str::to_string));
    let entry_symbol = parsed_args.entry_symbol;
    let wrap_symbols = parsed_args.wrap_symbols;
    let undefined_symbols = parsed_args.undefined_symbols;

    // Force-undefined symbols (-u SYM): enter them into the global table as
    // undefined so the archive group-resolution loop below pulls in defining
    // members, exactly like GNU ld/mold.
    for sym_name in &undefined_symbols {
        if !globals.contains_key(sym_name) {
            let fake = linker_common::Elf64Symbol {
                name_idx: 0, name: SymStr::new(&sym_name),
                info: 1 << 4, // STB_GLOBAL, STT_NOTYPE
                other: 0, shndx: 0, value: 0, size: 0,
            };
            globals.insert(sym_name.clone(),
                <GlobalSymbol as linker_common::GlobalSymbolOps>::new_undefined(&fake));
        }
    }

    // Load extra .o files immediately; archives (.a) and shared libraries (.so)
    // are deferred to the group resolution loop. Archives need iterative re-scanning
    // for circular dependencies, and shared libraries must be processed after
    // archive members are extracted so they can resolve symbols introduced by
    // those members (e.g., QEMU's libqemuutil.a members reference libglib-2.0.so).
    let mut deferred_libs: Vec<String> = Vec::new();
    for path in &extra_object_files {
        if path.ends_with(".a") || path.ends_with(".so") || path.contains(".so.") {
            deferred_libs.push(path.clone());
        } else {
            load_file(path, &mut objects, &mut globals, &mut needed_sonames, &lib_path_strings, false)?;
        }
    }

    let mut all_lib_paths: Vec<String> = extra_lib_paths;
    all_lib_paths.extend(lib_path_strings.iter().cloned());

    // Load CRT objects after
    for path in crt_objects_after {
        if Path::new(path).exists() {
            load_file(path, &mut objects, &mut globals, &mut needed_sonames, &all_lib_paths, false)?;
        }
    }

    // Load needed libraries using group resolution (like ld's --start-group/--end-group).
    // This iterates all archives and shared libraries until no new objects are pulled
    // in, handling circular dependencies between archives and ensuring shared libraries
    // can resolve symbols introduced by archive member extraction.
    {
        let mut all_lib_names: Vec<String> = needed_libs.iter().map(|s| s.to_string()).collect();
        all_lib_names.extend(libs_to_load.iter().cloned());

        let mut lib_paths_resolved: Vec<String> = Vec::new();
        // Include deferred .a and .so files first (preserving command-line order)
        for lib_path in &deferred_libs {
            if !lib_paths_resolved.contains(lib_path) {
                lib_paths_resolved.push(lib_path.clone());
            }
        }
        let needed_lib_count = needed_libs.len();
        for (idx, lib_name) in all_lib_names.iter().enumerate() {
            if let Some(lib_path) = linker_common::resolve_lib(lib_name, &all_lib_paths, is_static) {
                if !lib_paths_resolved.contains(&lib_path) {
                    lib_paths_resolved.push(lib_path);
                }
            } else if idx >= needed_lib_count {
                // User-specified -l library not found: error (matching ld behavior)
                return Err(format!("cannot find -l{}: No such file or directory", lib_name));
            }
        }

        // Group loading: iterate until stable. Track both object count changes
        // (from archive member extraction) and dynamic symbol count changes
        // (from shared library resolution) since either can introduce work for
        // the other on the next iteration.
        let mut changed = true;
        while changed {
            changed = false;
            let prev_obj_count = objects.len();
            let prev_dyn_count = needed_sonames.len();
            for lib_path in &lib_paths_resolved {
                load_file(lib_path, &mut objects, &mut globals, &mut needed_sonames, &all_lib_paths, false)?;
            }
            if objects.len() != prev_obj_count || needed_sonames.len() != prev_dyn_count {
                changed = true;
            }
        }
    }

    // Resolve remaining undefined symbols from default system libraries
    // (only when dynamically linking)
    if !is_static {
        let default_libs = ["libc.so.6", "libm.so.6", "libgcc_s.so.1", "ld-linux-x86-64.so.2"];
        linker_common::resolve_dynamic_symbols_elf64(
            &mut globals, &mut needed_sonames, &all_lib_paths, &default_libs,
        )?;
    }

    // Apply --defsym definitions: alias one symbol to another
    for (alias, target) in &defsym_defs {
        if let Some(target_sym) = globals.get(target).cloned() {
            globals.insert(alias.clone(), target_sym);
        }
    }

    // Apply --wrap=SYM: undefined references to SYM become references to
    // __wrap_SYM, and undefined references to __real_SYM become references
    // to SYM. Definitions are never renamed (GNU ld semantics). This is done
    // by rewriting the per-object symbol tables, so every later phase
    // (PLT/GOT construction, relocation application, GC) sees the redirected
    // names with zero extra bookkeeping.
    if !wrap_symbols.is_empty() {
        for obj in objects.iter_mut() {
            for sym in obj.symbols.iter_mut() {
                if sym.shndx == 0 && !sym.name.is_empty() {
                    if wrap_symbols.iter().any(|w| w == sym.name.as_str()) {
                        sym.name = SymStr::new(&format!("__wrap_{}", sym.name));
                    } else if let Some(real) = sym.name.strip_prefix("__real_") {
                        if wrap_symbols.iter().any(|w| w == real) {
                            sym.name = SymStr::new(&real.to_string());
                        }
                    }
                }
            }
        }
        // Drop stale undefined entries created before the rewrite.
        for w in &wrap_symbols {
            globals.remove(&format!("__real_{}", w));
            if globals.get(w).map(|g| g.defined_in.is_none() && !g.is_dynamic).unwrap_or(false) {
                globals.remove(w);
            }
        }
        // The wrapper may be undefined if the user forgot to provide it.
        // Leave that to the normal undefined-symbol check below.
    }

    // Garbage-collect unreferenced sections when --gc-sections is active.
    // This removes sections not reachable from entry points, which may also
    // eliminate undefined symbol references from dead code.
    let dead_sections: FxHashSet<(usize, usize)> = if gc_sections {
        let mut gc_roots: Vec<String> = undefined_symbols.clone();
        if let Some(ref e) = entry_symbol { gc_roots.push(e.clone()); }
        linker_common::gc_collect_sections_elf64_roots(&objects, &gc_roots)
    } else {
        FxHashSet::default()
    };

    // When gc-sections is active, remove globals that only exist in dead sections
    // and also remove references from dead sections
    if gc_sections {
        // Build set of symbols referenced only from dead sections
        let mut referenced_from_live: FxHashSet<String> = FxHashSet::default();
        for (obj_idx, obj) in objects.iter().enumerate() {
            for (sec_idx, relas) in obj.relocations.iter().enumerate() {
                if dead_sections.contains(&(obj_idx, sec_idx)) { continue; }
                for rela in relas {
                    if (rela.sym_idx as usize) < obj.symbols.len() {
                        let sym = &obj.symbols[rela.sym_idx as usize];
                        if !sym.name.is_empty() {
                            referenced_from_live.insert(sym.name.to_string());
                        }
                    }
                }
            }
        }
        // Remove undefined globals that are only referenced from dead sections
        globals.retain(|name, sym| {
            // Keep defined symbols, dynamic symbols, weak symbols, and those referenced from live code
            sym.defined_in.is_some() || sym.is_dynamic
                || (sym.info >> 4) == STB_WEAK
                || referenced_from_live.contains(name)
        });
    }

    // Check for truly undefined (non-weak, non-dynamic, non-linker-defined) symbols
    linker_common::check_undefined_symbols_elf64_verbose(&globals, 20, &objects)?;

    phase!("resolve+gc");
    // SHF_MERGE string/constant deduplication (.rodata.str1.1, .rodata.cst8):
    // build pools across all objects, rewrite relocations to pool symbols,
    // and retire the input sections. Disabled with LCCC_NO_STRING_MERGE=1.
    let mut dead_sections = dead_sections;
    if std::env::var("LCCC_NO_STRING_MERGE").is_err() {
        if let Some(plan) = linker_common::strmerge::plan_string_merge(
            &objects, &dead_sections, linker_common::map_section_name) {
            let applied = linker_common::strmerge::apply_string_merge(
                &mut objects, &mut globals, &mut dead_sections, &plan);
            if std::env::var("LCCC_DEBUG_STRMERGE").is_ok() {
                eprintln!("[strmerge] pools={} applied={}",
                    plan.pools.len(), applied);
                for p in &plan.pools {
                    eprintln!("[strmerge]   {} {} bytes", p.name, p.data.len());
                }
            }
        } else if std::env::var("LCCC_DEBUG_STRMERGE").is_ok() {
            eprintln!("[strmerge] no plan (no eligible sections)");
        }
    }

    phase!("strmerge");
    // Identical Code Folding: retire duplicate function sections and alias
    // them onto their surviving representative. Folded sections are added to
    // `dead_sections` so the merge step never lays them out; the alias is
    // installed into `section_map` afterwards, so every symbol address and
    // relocation that referred to a folded section resolves to the survivor.
    let icf_plan = match icf_mode.as_deref() {
        Some(mode) => icf::plan(&objects, mode == "safe"),
        None => icf::IcfPlan::default(),
    };
    for folded in icf_plan.redirect.keys() {
        dead_sections.insert(*folded);
    }

    phase!("icf");
    // Merge sections (skip dead sections when gc-sections is active)
    let mut output_sections: Vec<OutputSection> = Vec::new();
    let mut section_map: FxHashMap<(usize, usize), (usize, u64)> = FxHashMap::default();
    linker_common::merge_sections_elf64_gc(&objects, &mut output_sections, &mut section_map, &dead_sections);

    // Point folded sections at the representative's placement. This must run
    // after the merge, when the survivor has a real (output, offset) pair, and
    // before any address assignment consults the map.
    if !icf_plan.is_empty() {
        for (&folded, &rep) in icf_plan.redirect.iter() {
            if let Some(&placement) = section_map.get(&rep) {
                section_map.insert(folded, placement);
            }
        }
        if std::env::var("LCCC_DEBUG_ICF").is_ok() {
            eprintln!(
                "[icf] mode={} groups={} folded={} bytes_saved={} rejected_unsafe={}",
                icf_mode.as_deref().unwrap_or("none"),
                icf_plan.result.candidate_groups,
                icf_plan.result.folded_sections,
                icf_plan.result.bytes_saved,
                icf_plan.result.rejected_unsafe
            );
        }
    }

    phase!("merge-sections");
    // Allocate COMMON symbols
    linker_common::allocate_common_symbols_elf64(&mut globals, &mut output_sections);

    phase!("common");
    // Create PLT/GOT
    let (plt_names, got_entries, abs_dyn_relocs) = create_plt_got(&objects, &mut globals);

    phase!("plt-got");
    // Collect IFUNC symbols for static linking
    let ifunc_symbols = collect_ifunc_symbols(&globals, is_static);

    phase!("ifunc");
    // Emit executable
    let r = emit_executable(
        &objects, &mut globals, &mut output_sections, &section_map,
        &plt_names, &got_entries, &abs_dyn_relocs, &needed_sonames, output_path,
        export_dynamic, &rpath_entries, use_runpath, is_static,
        &ifunc_symbols, entry_symbol.as_deref(),
        parsed_args.z_now, parsed_args.z_relro,
        parsed_args.map_path.as_deref(),
        parsed_args.version_script.as_deref(),
    );
    phase!("emit-exec");
    if ld_time { eprintln!("[ldtime] {:<24} {:>7.1} ms", "TOTAL", t_all.elapsed().as_secs_f64()*1e3); }
    r
}

/// Create a shared library (.so) from object files.
///
/// Produces an ELF `ET_DYN` file with position-independent base address (0),
/// exporting all defined global symbols. Used when the compiler is invoked
/// with `-shared`.
pub fn link_shared(
    object_files: &[&str],
    output_path: &str,
    user_args: &[String],
    lib_paths: &[&str],
    needed_libs: &[&str],
) -> Result<(), String> {
    let mut objects: Vec<ElfObject> = Vec::new();
    let mut globals: FxHashMap<String, GlobalSymbol> = FxHashMap::default();
    let mut needed_sonames: Vec<String> = Vec::new();
    let lib_path_strings: Vec<String> = lib_paths.iter().map(|s| s.to_string()).collect();

    // Argument parsing is delegated to `linker_common::parse_linker_args`.
    //
    // This function used to carry its own ~90-line copy of the parser. That
    // duplication was live technical debt, not a stylistic wart: every new
    // flag had to be implemented twice, and whichever copy was forgotten
    // failed *silently*. Measured before this change, 21 flags known to
    // `args.rs` were dropped here, including `-Map` (worked for executables,
    // ignored for shared libraries) and `--defsym` (bfd emitted the alias,
    // lccc did not). `LinkerArgs::inputs` now models the ordered, positional
    // input list -- the one thing that previously blocked the merge.
    let parsed = linker_common::parse_linker_args(user_args);

    let extra_lib_paths: Vec<String> = parsed.extra_lib_paths.clone();
    let soname: Option<String> = parsed.soname.clone();
    let rpath_entries: Vec<String> = parsed.rpath_entries.clone();
    let use_runpath = parsed.use_runpath;
    let version_script: Option<String> = parsed.version_script.clone();
    let no_undefined = parsed.no_undefined;
    let bsymbolic = parsed.bsymbolic;
    let exclude_libs: Vec<String> = parsed.exclude_libs.clone();

    // (path_or_lib, is_lib, whole_archive_state), in command-line order.
    let ordered_items: Vec<(String, bool, bool)> = parsed.inputs.iter()
        .map(|it| (it.name.clone(), it.is_lib, it.whole_archive))
        .collect();

    // Load user object files (from the compiler driver, before user_args)
    for path in object_files {
        load_file(path, &mut objects, &mut globals, &mut needed_sonames, &lib_path_strings, false)?;
    }

    let mut all_lib_paths: Vec<String> = extra_lib_paths;
    all_lib_paths.extend(lib_path_strings.iter().cloned());

    // Load ordered items (bare files and -l libraries) preserving --whole-archive state
    let mut libs_to_load_later: Vec<(String, bool)> = Vec::new();
    for (item, is_lib, wa) in &ordered_items {
        if *is_lib {
            libs_to_load_later.push((item.clone(), *wa));
        } else {
            load_file(item, &mut objects, &mut globals, &mut needed_sonames, &all_lib_paths, *wa)?;
        }
    }

    // Resolve -l libraries
    if !libs_to_load_later.is_empty() {
        let mut lib_paths_resolved: Vec<(String, bool)> = Vec::new();
        for (lib_name, wa) in &libs_to_load_later {
            if let Some(lib_path) = linker_common::resolve_lib(lib_name, &all_lib_paths, false) {
                if !lib_paths_resolved.iter().any(|(p, _)| p == &lib_path) {
                    lib_paths_resolved.push((lib_path, *wa));
                }
            } else {
                return Err(format!("cannot find -l{}: No such file or directory", lib_name));
            }
        }
        // Track which whole-archive libraries have been fully loaded to avoid
        // re-adding all members on subsequent iterations of the group loop.
        let mut whole_archive_loaded: FxHashSet<String> = FxHashSet::default();
        let mut changed = true;
        while changed {
            changed = false;
            let prev_count = objects.len();
            for (lib_path, wa) in &lib_paths_resolved {
                if *wa && whole_archive_loaded.contains(lib_path) {
                    continue; // Already loaded all members
                }
                load_file(lib_path, &mut objects, &mut globals, &mut needed_sonames, &all_lib_paths, *wa)?;
                if *wa {
                    whole_archive_loaded.insert(lib_path.clone());
                }
            }
            if objects.len() != prev_count { changed = true; }
        }
    }

    // Resolve implicit libraries (e.g. libgcc.a) to provide compiler runtime
    // functions like __udivti3 that may be referenced by user code.
    if !needed_libs.is_empty() {
        let mut implicit_paths: Vec<String> = Vec::new();
        for lib_name in needed_libs {
            if let Some(lib_path) = linker_common::resolve_lib(lib_name, &all_lib_paths, false) {
                if !implicit_paths.contains(&lib_path) {
                    implicit_paths.push(lib_path);
                }
            }
        }
        let mut changed = true;
        while changed {
            changed = false;
            let prev_count = objects.len();
            for lib_path in &implicit_paths {
                load_file(lib_path, &mut objects, &mut globals, &mut needed_sonames, &all_lib_paths, false)?;
            }
            if objects.len() != prev_count { changed = true; }
        }
    }

    // Resolve remaining undefined symbols against system libraries (libc, libm,
    // libgcc_s) and add DT_NEEDED entries for any that provide matched symbols.
    let default_libs = ["libc.so.6", "libm.so.6", "libgcc_s.so.1", "ld-linux-x86-64.so.2"];
    linker_common::resolve_dynamic_symbols_elf64(
        &mut globals, &mut needed_sonames, &all_lib_paths, &default_libs,
    )?;

    // -z defs / --no-undefined: shared libraries normally tolerate
    // undefined symbols (resolved at load time), but with this flag every
    // reference must be satisfied at LINK time - the standard CMake/Qt
    // hardening switch to catch missing DT_NEEDED deps early.
    if no_undefined {
        linker_common::check_undefined_symbols_elf64_verbose(&globals, 20, &objects)?;
    }

    // --defsym=ALIAS=TARGET: alias one symbol to another. Same semantics as
    // the executable path (link_builtin); it was simply unreachable here while
    // link_shared parsed its own arguments and never looked at defsym_defs.
    for (alias, target) in &parsed.defsym_defs {
        if let Some(target_sym) = globals.get(target).cloned() {
            globals.insert(alias.clone(), target_sym);
        }
    }

    // Merge sections (no gc-sections for shared libraries)
    let mut output_sections: Vec<OutputSection> = Vec::new();
    let mut section_map: FxHashMap<(usize, usize), (usize, u64)> = FxHashMap::default();
    linker_common::merge_sections_elf64(&objects, &mut output_sections, &mut section_map);

    // Allocate COMMON symbols
    linker_common::allocate_common_symbols_elf64(&mut globals, &mut output_sections);

    // Emit shared library
    emit_shared_library(
        &objects, &mut globals, &mut output_sections, &section_map,
        &needed_sonames, output_path, soname, &rpath_entries, use_runpath,
        version_script.as_deref(), bsymbolic,
        &exclude_libs,
        parsed.map_path.as_deref(),
    )
}
