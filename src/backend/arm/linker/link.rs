//! AArch64 linker orchestration.
//!
//! Contains the two public entry points (`link_builtin` and `link_shared`) that
//! orchestrate the linking pipeline: load inputs, resolve symbols, merge sections,
//! build PLT/GOT, and dispatch to the appropriate ELF emission path.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use std::path::Path;

use super::elf::*;
use super::emit_dynamic::emit_dynamic_executable;
use super::emit_shared::emit_shared_library;
use super::emit_static::emit_executable;
use super::input::{load_file, resolve_lib, resolve_lib_prefer_shared};
use super::plt_got::create_plt_got;
use super::types::GlobalSymbol;
use crate::backend::linker_common;
use linker_common::OutputSection;

// ── Public entry point ─────────────────────────────────────────────────

/// Link AArch64 object files into an ELF executable (pre-resolved CRT/library variant).
///
/// Supports both static and dynamic linking. When `is_static` is false, shared
/// libraries are loaded and PLT/GOT/.dynamic sections are generated for dynamic
/// symbol references.
pub fn link_builtin(
    object_files: &[&str],
    output_path: &str,
    user_args: &[String],
    lib_paths: &[&str],
    needed_libs: &[&str],
    crt_objects_before: &[&str],
    crt_objects_after: &[&str],
    is_static: bool,
) -> Result<(), String> {
    if std::env::var("LINKER_DEBUG").is_ok() {
        eprintln!(
            "arm linker: object_files={:?} output={} user_args={:?} static={}",
            object_files, output_path, user_args, is_static
        );
    }
    let mut objects: Vec<ElfObject> = Vec::new();
    let mut globals: FxHashMap<String, GlobalSymbol> = FxHashMap::default();
    let mut needed_sonames: Vec<String> = Vec::new();

    let all_lib_paths: Vec<String> = lib_paths.iter().map(|s| s.to_string()).collect();

    // Parse user args for export-dynamic flag
    let mut export_dynamic = false;
    for arg in user_args {
        if arg == "-rdynamic" {
            export_dynamic = true;
        }
        if let Some(wl_arg) = arg.strip_prefix("-Wl,") {
            for part in wl_arg.split(',') {
                if part == "--export-dynamic" || part == "-export-dynamic" || part == "-E" {
                    export_dynamic = true;
                }
            }
        }
    }

    // Load CRT objects before user objects
    for path in crt_objects_before {
        if Path::new(path).exists() {
            load_file(
                path,
                &mut objects,
                &mut globals,
                &mut needed_sonames,
                &all_lib_paths,
                is_static,
            )?;
        }
    }

    // Load user object files
    for path in object_files {
        load_file(
            path,
            &mut objects,
            &mut globals,
            &mut needed_sonames,
            &all_lib_paths,
            is_static,
        )?;
    }

    // Parse user_args for -l, -L, bare files, etc.
    let args: Vec<&str> = user_args.iter().map(|s| s.as_str()).collect();
    let mut defsym_defs: Vec<(String, String)> = Vec::new();
    let mut extra_lib_paths: Vec<String> = Vec::new();
    let mut gc_sections = false;
    let mut arg_i = 0;
    while arg_i < args.len() {
        let arg = args[arg_i];
        if let Some(path) = arg.strip_prefix("-L") {
            let p = if path.is_empty() && arg_i + 1 < args.len() {
                arg_i += 1;
                args[arg_i]
            } else {
                path
            };
            extra_lib_paths.push(p.to_string());
        } else if let Some(lib) = arg.strip_prefix("-l") {
            let l = if lib.is_empty() && arg_i + 1 < args.len() {
                arg_i += 1;
                args[arg_i]
            } else {
                lib
            };
            let resolver = if is_static {
                resolve_lib
            } else {
                resolve_lib_prefer_shared
            };
            let mut combined = extra_lib_paths.clone();
            combined.extend(all_lib_paths.iter().cloned());
            if let Some(lib_path) = resolver(l, &combined) {
                load_file(
                    &lib_path,
                    &mut objects,
                    &mut globals,
                    &mut needed_sonames,
                    &combined,
                    is_static,
                )?;
            }
        } else if let Some(wl_arg) = arg.strip_prefix("-Wl,") {
            let parts: Vec<&str> = wl_arg.split(',').collect();
            let mut j = 0;
            while j < parts.len() {
                let part = parts[j];
                if let Some(lpath) = part.strip_prefix("-L") {
                    extra_lib_paths.push(lpath.to_string());
                } else if let Some(lib) = part.strip_prefix("-l") {
                    let resolver = if is_static {
                        resolve_lib
                    } else {
                        resolve_lib_prefer_shared
                    };
                    let mut combined = extra_lib_paths.clone();
                    combined.extend(all_lib_paths.iter().cloned());
                    if let Some(lib_path) = resolver(lib, &combined) {
                        load_file(
                            &lib_path,
                            &mut objects,
                            &mut globals,
                            &mut needed_sonames,
                            &combined,
                            is_static,
                        )?;
                    }
                } else if let Some(defsym_arg) = part.strip_prefix("--defsym=") {
                    // --defsym=SYMBOL=EXPR: define a symbol alias
                    // TODO: only supports symbol-to-symbol aliasing, not arbitrary expressions
                    if let Some(eq_pos) = defsym_arg.find('=') {
                        defsym_defs.push((
                            defsym_arg[..eq_pos].to_string(),
                            defsym_arg[eq_pos + 1..].to_string(),
                        ));
                    }
                } else if part == "--defsym" && j + 1 < parts.len() {
                    // Two-argument form: --defsym SYM=VAL
                    j += 1;
                    let defsym_arg = parts[j];
                    if let Some(eq_pos) = defsym_arg.find('=') {
                        defsym_defs.push((
                            defsym_arg[..eq_pos].to_string(),
                            defsym_arg[eq_pos + 1..].to_string(),
                        ));
                    }
                } else if part == "--gc-sections" {
                    gc_sections = true;
                } else if part == "--no-gc-sections" {
                    gc_sections = false;
                }
                j += 1;
            }
        } else if !arg.starts_with('-') && Path::new(arg).exists() {
            load_file(
                arg,
                &mut objects,
                &mut globals,
                &mut needed_sonames,
                &all_lib_paths,
                is_static,
            )?;
        }
        arg_i += 1;
    }

    // Load CRT objects after user objects
    for path in crt_objects_after {
        if Path::new(path).exists() {
            load_file(
                path,
                &mut objects,
                &mut globals,
                &mut needed_sonames,
                &all_lib_paths,
                is_static,
            )?;
        }
    }

    // Build combined library search paths: user -L first, then system paths
    let mut combined_lib_paths: Vec<String> = extra_lib_paths;
    combined_lib_paths.extend(all_lib_paths.iter().cloned());

    // Load default libraries in a group (like ld's --start-group)
    if !needed_libs.is_empty() {
        let resolver = if is_static {
            resolve_lib
        } else {
            resolve_lib_prefer_shared
        };
        let mut lib_paths_resolved: Vec<String> = Vec::new();
        for lib_name in needed_libs {
            if let Some(lib_path) = resolver(lib_name, &combined_lib_paths) {
                if !lib_paths_resolved.contains(&lib_path) {
                    lib_paths_resolved.push(lib_path);
                }
            }
        }
        let mut changed = true;
        while changed {
            changed = false;
            let prev_count = objects.len();
            for lib_path in &lib_paths_resolved {
                load_file(
                    lib_path,
                    &mut objects,
                    &mut globals,
                    &mut needed_sonames,
                    &combined_lib_paths,
                    is_static,
                )?;
            }
            if objects.len() != prev_count {
                changed = true;
            }
        }
    }

    // For dynamic linking, resolve remaining undefined symbols against system libs
    if !is_static {
        let default_libs = [
            "libc.so.6",
            "libm.so.6",
            "libgcc_s.so.1",
            "ld-linux-aarch64.so.1",
        ];
        linker_common::resolve_dynamic_symbols_elf64(
            &mut globals,
            &mut needed_sonames,
            &combined_lib_paths,
            &default_libs,
        )?;
    }

    // Apply --defsym definitions: aliases are resolved here; constants and
    // expressions are evaluated after layout in the emitter (same design as
    // the x86-64 linker — classification lives in linker_common::defsym).
    let pending_defsyms = apply_defsyms(&mut globals, &defsym_defs)?;

    // Garbage-collect unreferenced sections when --gc-sections is active.
    // This removes sections not reachable from entry points, which may also
    // eliminate undefined symbol references from dead code.
    let dead_sections: FxHashSet<(usize, usize)> = if gc_sections {
        linker_common::gc_collect_sections_elf64(&objects)
    } else {
        FxHashSet::default()
    };

    // When gc-sections is active, remove globals that only exist in dead sections
    if gc_sections {
        let mut referenced_from_live: FxHashSet<String> = FxHashSet::default();
        for (obj_idx, obj) in objects.iter().enumerate() {
            for (sec_idx, relas) in obj.relocations.iter().enumerate() {
                if dead_sections.contains(&(obj_idx, sec_idx)) {
                    continue;
                }
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
        globals.retain(|name, sym| {
            sym.defined_in.is_some()
                || sym.is_dynamic
                || (sym.info >> 4) == STB_WEAK
                || referenced_from_live.contains(name)
        });
    }

    // Reject truly undefined symbols (weak undefined are allowed)
    let mut unresolved = Vec::new();
    for (name, sym) in &globals {
        if sym.defined_in.is_none() && !sym.is_dynamic && sym.section_idx == SHN_UNDEF {
            let binding = sym.info >> 4;
            if binding != STB_WEAK && !linker_common::is_linker_defined_symbol(name) {
                unresolved.push(name.clone());
            }
        }
    }
    if !unresolved.is_empty() {
        unresolved.sort();
        unresolved.truncate(20);
        return Err(format!(
            "undefined symbols: {}",
            unresolved
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    // Prune FDEs whose functions were collected (see `eh_frame::prune_dead_fdes`).
    linker_common::prune_dead_fdes(&mut objects, &dead_sections);

    // Merge sections (skip dead sections when gc-sections is active)
    let mut output_sections: Vec<OutputSection> = Vec::new();
    let mut section_map: FxHashMap<(usize, usize), (usize, u64)> = FxHashMap::default();
    linker_common::merge_sections_elf64_gc(
        &objects,
        &mut output_sections,
        &mut section_map,
        &dead_sections,
    );

    // Allocate COMMON symbols (using shared implementation)
    // No --sort-common on this path yet: default (unsorted) order.
    linker_common::allocate_common_symbols_elf64(&mut globals, &mut output_sections, false);

    // Check if we have any dynamic symbols
    let has_dynamic_syms = globals.values().any(|g| g.is_dynamic);

    if has_dynamic_syms && !is_static {
        // Create PLT/GOT for dynamic symbols
        let (plt_names, got_entries) = create_plt_got(&objects, &mut globals);

        // Emit dynamically-linked executable
        emit_dynamic_executable(
            &objects,
            &mut globals,
            &mut output_sections,
            &section_map,
            &plt_names,
            &got_entries,
            &needed_sonames,
            output_path,
            export_dynamic,
            &pending_defsyms,
        )
    } else {
        // Fall back to static emit
        emit_executable(
            &objects,
            &mut globals,
            &mut output_sections,
            &section_map,
            output_path,
            &pending_defsyms,
        )
    }
}

// ── Shared library output ────────────────────────────────────────────

/// Create a shared library (.so) from object files.
pub fn link_shared(
    object_files: &[&str],
    output_path: &str,
    user_args: &[String],
    lib_paths: &[&str],
) -> Result<(), String> {
    let mut objects: Vec<ElfObject> = Vec::new();
    let mut globals: FxHashMap<String, GlobalSymbol> = FxHashMap::default();
    let mut needed_sonames: Vec<String> = Vec::new();
    let lib_path_strings: Vec<String> = lib_paths.iter().map(|s| s.to_string()).collect();

    // Parse user args
    let mut extra_lib_paths: Vec<String> = Vec::new();
    let mut libs_to_load: Vec<String> = Vec::new();
    let mut extra_object_files: Vec<String> = Vec::new();
    let mut soname: Option<String> = None;
    let mut defsym_defs: Vec<(String, String)> = Vec::new();
    let mut i = 0;
    let args: Vec<&str> = user_args.iter().map(|s| s.as_str()).collect();
    while i < args.len() {
        let arg = args[i];
        if let Some(path) = arg.strip_prefix("-L") {
            let p = if path.is_empty() && i + 1 < args.len() {
                i += 1;
                args[i]
            } else {
                path
            };
            extra_lib_paths.push(p.to_string());
        } else if let Some(lib) = arg.strip_prefix("-l") {
            let l = if lib.is_empty() && i + 1 < args.len() {
                i += 1;
                args[i]
            } else {
                lib
            };
            libs_to_load.push(l.to_string());
        } else if let Some(wl_arg) = arg.strip_prefix("-Wl,") {
            let parts: Vec<&str> = wl_arg.split(',').collect();
            for j in 0..parts.len() {
                let part = parts[j];
                if let Some(sn) = part.strip_prefix("-soname=") {
                    soname = Some(sn.to_string());
                } else if part == "-soname" && j + 1 < parts.len() {
                    soname = Some(parts[j + 1].to_string());
                } else if let Some(defsym_arg) = part.strip_prefix("--defsym=") {
                    if let Some(eq_pos) = defsym_arg.find('=') {
                        defsym_defs.push((
                            defsym_arg[..eq_pos].to_string(),
                            defsym_arg[eq_pos + 1..].to_string(),
                        ));
                    }
                } else if part == "--defsym" && j + 1 < parts.len() {
                    // Two-argument form: --defsym SYM=VAL
                    let defsym_arg = parts[j + 1];
                    if let Some(eq_pos) = defsym_arg.find('=') {
                        defsym_defs.push((
                            defsym_arg[..eq_pos].to_string(),
                            defsym_arg[eq_pos + 1..].to_string(),
                        ));
                    }
                } else if let Some(lpath) = part.strip_prefix("-L") {
                    extra_lib_paths.push(lpath.to_string());
                } else if let Some(lib) = part.strip_prefix("-l") {
                    libs_to_load.push(lib.to_string());
                }
            }
        } else if arg == "-shared" || arg == "-nostdlib" || arg == "-o" {
            if arg == "-o" {
                i += 1;
            }
        } else if !arg.starts_with('-') && Path::new(arg).exists() {
            extra_object_files.push(arg.to_string());
        }
        i += 1;
    }

    for path in object_files {
        load_file(
            path,
            &mut objects,
            &mut globals,
            &mut needed_sonames,
            &lib_path_strings,
            false,
        )?;
    }
    for path in &extra_object_files {
        load_file(
            path,
            &mut objects,
            &mut globals,
            &mut needed_sonames,
            &lib_path_strings,
            false,
        )?;
    }

    let mut all_lib_paths: Vec<String> = extra_lib_paths;
    all_lib_paths.extend(lib_path_strings.iter().cloned());

    if !libs_to_load.is_empty() {
        let mut lib_paths_resolved: Vec<String> = Vec::new();
        for lib_name in &libs_to_load {
            if let Some(lib_path) = resolve_lib_prefer_shared(lib_name, &all_lib_paths) {
                if !lib_paths_resolved.contains(&lib_path) {
                    lib_paths_resolved.push(lib_path);
                }
            }
        }
        let mut changed = true;
        while changed {
            changed = false;
            let prev_count = objects.len();
            for lib_path in &lib_paths_resolved {
                load_file(
                    lib_path,
                    &mut objects,
                    &mut globals,
                    &mut needed_sonames,
                    &all_lib_paths,
                    false,
                )?;
            }
            if objects.len() != prev_count {
                changed = true;
            }
        }
    }

    // Merge sections (no gc-sections for shared libraries)
    let mut output_sections: Vec<OutputSection> = Vec::new();
    let mut section_map: FxHashMap<(usize, usize), (usize, u64)> = FxHashMap::default();
    linker_common::merge_sections_elf64(&objects, &mut output_sections, &mut section_map);
    // No --sort-common on this path yet: default (unsorted) order.
    linker_common::allocate_common_symbols_elf64(&mut globals, &mut output_sections, false);

    // Resolve undefined symbols against system shared libraries to discover
    // NEEDED dependencies. Without this, the shared library would be missing
    // DT_NEEDED entries for libc.so.6 etc., causing the dynamic linker to
    // fail to resolve PLT symbols at runtime.
    resolve_dynamic_symbols_for_shared(&objects, &globals, &mut needed_sonames, &all_lib_paths);

    // --defsym: aliases resolve here; expressions evaluate in the emitter
    // after layout (same design as the x86-64 linker).
    let pending_defsyms = apply_defsyms(&mut globals, &defsym_defs)?;

    // Emit shared library
    emit_shared_library(
        &objects,
        &mut globals,
        &mut output_sections,
        &section_map,
        &needed_sonames,
        output_path,
        soname,
        &pending_defsyms,
    )
}

/// Apply `--defsym` definitions (GNU `ld --defsym` semantics).
///
/// Mirrors the x86-64 linker's `apply_defsyms`: an expression that
/// references only symbols already defined in the link is evaluated
/// immediately; any other expression is recorded in `pending` and must be
/// evaluated by the emitter after layout, via `evaluate_pending_defsyms`.
/// (This matters for symbols defined by link-time mechanisms — e.g.
/// `_etext`, which is computed from the final section layout.)
fn apply_defsyms(
    globals: &mut FxHashMap<String, GlobalSymbol>,
    defs: &[(String, String)],
) -> Result<Vec<(String, String, usize)>, String> {
    use crate::backend::linker_common::defsym::{self, Defsym, DefsymError};
    let mut pending: Vec<(String, String, usize)> = Vec::new();
    for (pos, (name, expr)) in defs.iter().enumerate() {
        let index = pos + 1;
        // "Defined" means the same thing it means to the undefined-symbol
        // check: the symbol belongs to some object, or is one this linker
        // created (defined_in == Some(usize::MAX)). Layout-derived magic
        // symbols (`_etext`, `end`, …) count as defined even though they
        // only exist after layout — GNU ld resolves them during expression
        // evaluation, and so do we (see `defsym::is_linker_defined`).
        let classified = defsym::classify(expr, |n| {
            globals.get(n).is_some_and(|g| g.defined_in.is_some()) || defsym::is_linker_defined(n)
        })
        .map_err(|e| e.gnu_message(index))?;
        let value = match classified {
            // Alias: copy the target's whole definition, exactly as the old
            // alias-only loop did. A target only the linker defines (a magic
            // symbol) has no address until layout, so it is deferred to
            // evaluation like an expression.
            Defsym::Alias(target) => {
                match globals.get(&target) {
                    Some(sym) if sym.defined_in.is_some() => {
                        if sym.is_dynamic {
                            // GNU ld: a symbol visible only through a shared
                            // library is not "defined in this link", so
                            // aliasing it is an undefined-symbol error.
                            return Err(DefsymError::UndefinedSymbol(target).gnu_message(index));
                        }
                        globals.insert(name.clone(), sym.clone());
                        continue;
                    }
                    _ => {
                        // Linker language symbol (`_etext`, `end`, …): the
                        // arm's 0 becomes the usual placeholder (defined,
                        // ABS) below, so the undefined-symbol check and the
                        // PLT/GOT need scan see it;
                        // `evaluate_pending_defsyms` overwrites the value
                        // after layout.
                        pending.push((name.clone(), target, index));
                        0
                    }
                }
            }
            Defsym::Constant(v) => v,
            Defsym::Expression(e) => {
                pending.push((name.clone(), e, index));
                0
            }
        };
        globals.insert(
            name.clone(),
            GlobalSymbol {
                value,
                size: 0,
                info: (STB_GLOBAL << 4) | STT_NOTYPE,
                defined_in: Some(usize::MAX),
                section_idx: SHN_ABS,
                from_lib: None,
                plt_idx: None,
                got_idx: None,
                is_dynamic: false,
                copy_reloc: false,
                lib_sym_value: 0,
            },
        );
    }
    Ok(pending)
}

/// Evaluate defsym expressions that were deferred until after layout.
///
/// Call from the emitter, after section addresses and standard symbols
/// (`_etext`, `end`, …) are known, before any section content is emitted.
/// GNU `ld` evaluates expressions the same way — after layout — so
/// `--defsym k=_etext` and any arithmetic over defined symbols produce
/// GNU-identical values.
pub(super) fn evaluate_pending_defsyms(
    globals: &mut FxHashMap<String, GlobalSymbol>,
    pending: &[(String, String, usize)],
) -> Result<(), String> {
    use crate::backend::linker_common::defsym;
    for (name, expr, index) in pending {
        let value = defsym::eval_with_symbols(expr, &|n| {
            globals
                .get(n)
                .and_then(|g| g.defined_in.is_some().then_some(g.value))
        })
        .map_err(|err| err.gnu_message(*index))?;
        let entry = globals
            .get_mut(name)
            .ok_or_else(|| format!("--defsym {name}: symbol vanished before evaluation"))?;
        entry.value = value;
        entry.defined_in = Some(usize::MAX);
        entry.section_idx = SHN_ABS;
    }
    Ok(())
}

/// Discover NEEDED shared library dependencies for a shared library build.
/// Scans object file relocations for CALL26/JUMP26 references to undefined symbols
/// and searches system libraries to find which .so files provide them.
fn resolve_dynamic_symbols_for_shared(
    objects: &[ElfObject],
    globals: &FxHashMap<String, GlobalSymbol>,
    needed_sonames: &mut Vec<String>,
    lib_paths: &[String],
) {
    // Collect undefined symbol names referenced by function calls
    let mut undefined: Vec<String> = Vec::new();
    for obj in objects.iter() {
        for sec_relas in &obj.relocations {
            for rela in sec_relas {
                let si = rela.sym_idx as usize;
                if si >= obj.symbols.len() {
                    continue;
                }
                let sym = &obj.symbols[si];
                if sym.name.is_empty() || sym.is_local() {
                    continue;
                }
                let is_undef = if let Some(g) = globals.get(sym.name.as_str()) {
                    g.is_dynamic || (g.defined_in.is_none() && g.section_idx == SHN_UNDEF)
                } else {
                    sym.is_undefined()
                };
                if is_undef && !undefined.iter().any(|u| u == sym.name.as_str()) {
                    undefined.push(sym.name.to_string());
                }
            }
        }
    }
    if undefined.is_empty() {
        return;
    }

    let lib_names = [
        "libc.so.6",
        "libm.so.6",
        "libpthread.so.0",
        "libdl.so.2",
        "librt.so.1",
        "ld-linux-aarch64.so.1",
    ];
    let mut libs: Vec<String> = Vec::new();
    for lib_name in &lib_names {
        for dir in lib_paths {
            let candidate = format!("{}/{}", dir, lib_name);
            if Path::new(&candidate).exists() {
                libs.push(candidate);
                break;
            }
        }
    }
    for lib_path in &libs {
        let data = match std::fs::read(lib_path) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let soname = linker_common::parse_soname(&data).unwrap_or_else(|| {
            Path::new(lib_path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        });
        if needed_sonames.contains(&soname) {
            continue;
        }
        let dyn_syms = match linker_common::parse_shared_library_symbols(&data, lib_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let provides_any = undefined
            .iter()
            .any(|name| dyn_syms.iter().any(|ds| ds.name == *name));
        if provides_any {
            needed_sonames.push(soname);
        }
    }
}
