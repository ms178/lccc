//! RISC-V 64-bit ELF linker: orchestration of linking phases.
//!
//! Contains the two public entry points (`link_builtin` and `link_shared`) that
//! orchestrate the linking pipeline: load inputs, resolve symbols, merge sections,
//! and dispatch to the appropriate ELF emission path.
//!
//! All heavy ELF writing is delegated to:
//! - `emit_exec`: executable emission (static and dynamic)
//! - `emit_shared`: shared library (.so) emission

use super::elf_read::*;
use super::relocations::section_order;
use super::{input, sections, symbols};
use crate::backend::linker_common;
use crate::common::fx_hash::{FxHashMap, FxHashSet};

// ── Public entry point: executable linking ───────────────────────────────

/// Link object files into a RISC-V ELF executable (pre-resolved CRT/library variant).
///
/// This is the primary entry point, matching the pattern used by x86-64 and i686 linkers.
/// CRT objects and library paths are resolved by common.rs before being passed in.
///
/// `object_files`: paths to user .o files and .a archives
/// `output_path`: path for the output executable
/// `user_args`: additional linker flags from the user
/// `lib_paths`: pre-resolved library search paths (user -L first, then system)
/// `needed_libs_default`: the driver's default libraries (e.g., ["gcc", "gcc_s", "c", "m"]),
///   searched after the user's `-l` libraries
/// `crt_objects_before`: CRT objects to link before user objects (e.g., crt1.o, crti.o, crtbegin.o)
/// `crt_objects_after`: CRT objects to link after user objects (e.g., crtend.o, crtn.o)
pub fn link_builtin(
    object_files: &[&str],
    object_as_needed: &[bool],
    output_path: &str,
    user_args: &[String],
    lib_paths: &[&str],
    needed_libs_default: &[&str],
    crt_objects_before: &[&str],
    crt_objects_after: &[&str],
) -> Result<(), String> {
    // Parse user arguments using shared infrastructure
    let parsed_args = linker_common::parse_linker_args(user_args);
    parsed_args.validate_common_page_size()?;
    parsed_args.warn_unimplemented_keyword_options()?;
    let trace = parsed_args.trace_files;
    let rpath = linker_common::rpath_dyn_entry(&parsed_args);
    let is_static = parsed_args.is_static;
    let defsym_defs = parsed_args.defsym_defs;

    // Collect all input files: CRT before + user objects + bare files from args + CRT after
    let mut all_inputs: Vec<String> = Vec::new();
    for crt in crt_objects_before {
        all_inputs.push(crt.to_string());
    }
    for obj in object_files {
        all_inputs.push(obj.to_string());
    }
    all_inputs.extend(parsed_args.extra_object_files);

    // The user's -L directories, then the driver's: GNU ld searches -L first
    // (pre-fix the driver's came first, so a library the user supplied with
    // -L lost to a system copy of the same name).
    let mut lib_search_paths: Vec<String> = parsed_args.extra_lib_paths;
    for p in lib_paths {
        if !lib_search_paths.iter().any(|q| q == p) {
            lib_search_paths.push(p.to_string());
        }
    }
    // gcc's order: the user's `-l` libraries, then the defaults. DT_NEEDED
    // follows it, and with it the loader's symbol search order -- a user
    // library interposing a libc function must come first.
    let mut needed_libs: Vec<String> = parsed_args.libs_to_load;
    linker_common::require_libs(&needed_libs, &lib_search_paths, is_static)?;
    let implicit_from = needed_libs.len();
    needed_libs.extend(needed_libs_default.iter().map(|s| s.to_string()));
    let lib_as_needed = input::libs_as_needed(&needed_libs, implicit_from, &parsed_args.inputs);

    // ── Phase 1: Load inputs, discover shared libs, resolve archives ────

    let mut input_objs: Vec<(String, ElfObject)> = Vec::new();
    let mut inline_archive_paths: Vec<String> = Vec::new();
    let mut positional_shared_libs: Vec<(String, usize)> = Vec::new();
    input::load_input_files(
        &all_inputs,
        &mut input_objs,
        &mut inline_archive_paths,
        &mut positional_shared_libs,
    )?;

    let mut defined_syms: FxHashSet<String> = FxHashSet::default();
    let mut undefined_syms: FxHashSet<String> = FxHashSet::default();
    input::collect_initial_symbols(&input_objs, &mut defined_syms, &mut undefined_syms);

    let mut shared_lib_syms = input::SharedLibSyms::default();
    if parsed_args.warn_common {
        shared_lib_syms.record_libraries();
    }
    // The objects read before any library: their commons meet the libraries
    // "common first"; any read later (archive members) "library first".
    let late_from = input_objs.len();
    shared_lib_syms.expect_commons(&input_objs);
    let mut actual_needed_libs: Vec<String> = Vec::new();
    if !is_static {
        // Positionally-named .so inputs (./libfoo.so on the command line):
        // same NEEDED/symbol treatment as -l-discovered libraries.  They
        // precede the driver's implicit `-lc` on every command line, so they
        // are registered first: the first provider of a name binds it and
        // DT_NEEDED follows link order, as with GNU ld.
        input::register_positional_shared_libs(
            &positional_shared_libs,
            |path| operand_as_needed(object_files, object_as_needed, &parsed_args.inputs, path),
            &input_objs,
            &mut shared_lib_syms,
            &mut actual_needed_libs,
        );
        input::discover_shared_lib_symbols(
            &needed_libs,
            &lib_as_needed,
            &lib_search_paths,
            &mut input_objs,
            &mut defined_syms,
            &mut undefined_syms,
            &mut shared_lib_syms,
            &mut actual_needed_libs,
        );
    }

    if trace {
        for (name, _) in &input_objs {
            println!("{name}");
        }
        for lib in &actual_needed_libs {
            println!("{lib}");
        }
    }

    // Treat symbols available from shared libs as "defined" for archive resolution
    let shared_defined: FxHashSet<String> = shared_lib_syms
        .keys()
        .filter(|s| undefined_syms.contains(*s))
        .cloned()
        .collect();
    for s in &shared_defined {
        undefined_syms.remove(s);
    }

    input::resolve_archives(
        &inline_archive_paths,
        &needed_libs,
        is_static,
        &lib_search_paths,
        &mut input_objs,
        &mut defined_syms,
        &mut undefined_syms,
        &shared_lib_syms,
    );

    // Restore shared-lib-defined symbols back into undefined_syms
    for s in shared_defined {
        if !defined_syms.contains(&s) {
            undefined_syms.insert(s);
        }
    }
    // crtend*.o/crtn.o close the link after every library member, as in
    // gcc's spec: crtend's zero terminator ends `.eh_frame` (libgcc's linear
    // FDE search stops there) and crtn's epilogue ends `.init`/`.fini`.
    load_trailing_crt_objects(crt_objects_after, &mut input_objs)?;

    report_common_warnings(
        parsed_args.warn_common,
        parsed_args.fatal_warnings,
        &input_objs,
        late_from,
        &shared_lib_syms,
    )?;
    symbols::import_shared_commons(&mut input_objs, late_from, &shared_lib_syms);

    // ── Phase 2: Merge sections ─────────────────────────────────────────

    super::reloc::validate_input_relocations(&input_objs)?;
    let (mut merged_sections, mut merged_map, input_sec_refs) =
        sections::merge_sections(&input_objs);

    // ── Phase 3: Build global symbol table ──────────────────────────────

    let mut sec_mapping: FxHashMap<(usize, usize), (usize, u64)> = FxHashMap::default();
    for r in &input_sec_refs {
        sec_mapping.insert(
            (r.obj_idx, r.sec_idx),
            (r.merged_sec_idx, r.offset_in_merged),
        );
    }

    let mut global_syms = symbols::build_global_symbols(
        &input_objs,
        &sec_mapping,
        &mut merged_sections,
        &mut merged_map,
    );

    // `--defsym ALIAS=TARGET` with an object-defined TARGET makes ALIAS a
    // local definition BEFORE dynamic classification, as GNU ld resolves it:
    // a reference to ALIAS must bind to TARGET, never to a same-named
    // shared-library import (`--defsym=sqrt=test_sqrt` used to keep a
    // `sqrt@GLIBC` JUMP_SLOT -- a crash without -lm, the wrong function
    // with it).
    for (alias, target) in &defsym_defs {
        if let Some(target_sym) = global_syms.get(target).filter(|s| s.defined).cloned() {
            global_syms.insert(alias.clone(), target_sym);
        }
    }

    let (plt_symbols, copy_symbols) = if !is_static {
        symbols::mark_plt_and_copy_symbols(&mut global_syms, &shared_lib_syms)
    } else {
        (Vec::new(), Vec::new())
    };

    // An alias of a shared-library symbol takes the classified import (its
    // PLT/copy slot), so it is applied after classification.
    for (alias, target) in &defsym_defs {
        if let Some(target_sym) = global_syms.get(target).filter(|s| !s.defined).cloned() {
            global_syms.insert(alias.clone(), target_sym);
        }
    }

    if !is_static {
        symbols::check_undefined_symbols(&global_syms, &shared_lib_syms)?;
    } else {
        symbols::check_undefined_symbols(&global_syms, &FxHashMap::default())?;
    }

    let (got_symbols, tls_got_symbols, local_got_sym_info) =
        symbols::collect_got_entries(&input_objs);

    // Sort sections by canonical order
    let mut sec_indices: Vec<usize> = (0..merged_sections.len()).collect();
    sec_indices.sort_by_key(|&i| {
        let ms = &merged_sections[i];
        section_order(&ms.name, ms.sh_flags)
    });

    // ── Phase 4+: Emit executable ───────────────────────────────────────

    super::emit_exec::emit_executable(
        &input_objs,
        &mut merged_sections,
        &mut merged_map,
        &sec_mapping,
        &mut global_syms,
        &got_symbols,
        &tls_got_symbols,
        &local_got_sym_info,
        &plt_symbols,
        &copy_symbols,
        &sec_indices,
        &actual_needed_libs,
        &shared_lib_syms,
        rpath.as_ref(),
        is_static,
        output_path,
    )
}

// ── Public entry point: shared library linking ──────────────────────────

/// Link object files into a RISC-V shared library (.so).
///
/// Produces an ET_DYN ELF with base address 0 (position-independent).
/// All defined global symbols are exported to .dynsym.
///
/// `crt_objects_before`/`crt_objects_after` and `implicit_libs` are gcc's
/// `-shared` spec as resolved by the driver (`crti.o crtbeginS.o ...
/// crtendS.o crtn.o`, `-lgcc --as-needed -lgcc_s -lc`); inputs go through
/// the same loading, archive-member extraction, linker-script and
/// DT_NEEDED logic as executables. (Pre-fix the shared path linked no
/// startup objects -- no `__dso_handle`, so `atexit` from libc_nonshared.a
/// was unresolvable -- recorded DT_NEEDED only for user `-l` ELF objects
/// (`-lc`, a linker script, recorded nothing, and `_Unwind_*` from
/// libgcc_s.so.1 never: "undefined symbol: _Unwind_Backtrace" at load
/// time), pulled every member of an archive, and dropped `-rpath`.)
pub fn link_shared(
    object_files: &[&str],
    object_as_needed: &[bool],
    output_path: &str,
    user_args: &[String],
    lib_paths: &[&str],
    crt_objects_before: &[&str],
    crt_objects_after: &[&str],
    implicit_libs: &[&str],
) -> Result<(), String> {
    let parsed_args = linker_common::parse_linker_args(user_args);
    parsed_args.validate_common_page_size()?;
    parsed_args.warn_unimplemented_keyword_options()?;
    let rpath = linker_common::rpath_dyn_entry(&parsed_args);

    let mut all_inputs: Vec<String> = Vec::new();
    all_inputs.extend(crt_objects_before.iter().map(|s| s.to_string()));
    all_inputs.extend(object_files.iter().map(|s| s.to_string()));
    all_inputs.extend(parsed_args.extra_object_files);

    let mut lib_search_paths: Vec<String> = parsed_args.extra_lib_paths;
    for p in lib_paths {
        if !lib_search_paths.iter().any(|q| q == p) {
            lib_search_paths.push(p.to_string());
        }
    }
    let mut needed_libs: Vec<String> = parsed_args.libs_to_load;
    linker_common::require_libs(&needed_libs, &lib_search_paths, false)?;
    let implicit_from = needed_libs.len();
    needed_libs.extend(implicit_libs.iter().map(|s| s.to_string()));
    let lib_as_needed = input::libs_as_needed(&needed_libs, implicit_from, &parsed_args.inputs);

    // ── Phase 1: Load inputs, discover shared libs, resolve archives ──

    let mut input_objs: Vec<(String, ElfObject)> = Vec::new();
    let mut inline_archive_paths: Vec<String> = Vec::new();
    let mut positional_shared_libs: Vec<(String, usize)> = Vec::new();
    input::load_input_files(
        &all_inputs,
        &mut input_objs,
        &mut inline_archive_paths,
        &mut positional_shared_libs,
    )?;
    if input_objs.is_empty() && inline_archive_paths.is_empty() {
        return Err("No input files for shared library".to_string());
    }

    let mut defined_syms: FxHashSet<String> = FxHashSet::default();
    let mut undefined_syms: FxHashSet<String> = FxHashSet::default();
    input::collect_initial_symbols(&input_objs, &mut defined_syms, &mut undefined_syms);

    let mut shared_lib_syms = input::SharedLibSyms::default();
    if parsed_args.warn_common {
        shared_lib_syms.record_libraries();
    }
    let late_from = input_objs.len();
    shared_lib_syms.expect_commons(&input_objs);
    let mut needed_sonames: Vec<String> = Vec::new();
    // Positional .so inputs first: link order (see `link_builtin`).
    input::register_positional_shared_libs(
        &positional_shared_libs,
        |path| operand_as_needed(object_files, object_as_needed, &parsed_args.inputs, path),
        &input_objs,
        &mut shared_lib_syms,
        &mut needed_sonames,
    );
    input::discover_shared_lib_symbols(
        &needed_libs,
        &lib_as_needed,
        &lib_search_paths,
        &mut input_objs,
        &mut defined_syms,
        &mut undefined_syms,
        &mut shared_lib_syms,
        &mut needed_sonames,
    );
    // Symbols a shared library provides must not pull archive members.
    let shared_defined: Vec<String> = shared_lib_syms
        .keys()
        .filter(|s| undefined_syms.contains(*s))
        .cloned()
        .collect();
    for s in &shared_defined {
        undefined_syms.remove(s);
    }
    input::resolve_archives(
        &inline_archive_paths,
        &needed_libs,
        false,
        &lib_search_paths,
        &mut input_objs,
        &mut defined_syms,
        &mut undefined_syms,
        &shared_lib_syms,
    );
    // crtend*.o/crtn.o close the link after every library member, as in
    // gcc's spec: crtend's zero terminator ends `.eh_frame` (libgcc's linear
    // FDE search stops there) and crtn's epilogue ends `.init`/`.fini`.
    load_trailing_crt_objects(crt_objects_after, &mut input_objs)?;
    let soname = parsed_args.soname;

    report_common_warnings(
        parsed_args.warn_common,
        parsed_args.fatal_warnings,
        &input_objs,
        late_from,
        &shared_lib_syms,
    )?;
    symbols::import_shared_commons(&mut input_objs, late_from, &shared_lib_syms);

    // ── Phase 2: Merge sections ─────────────────────────────────────

    super::reloc::validate_input_relocations(&input_objs)?;
    let (mut merged_sections, mut merged_map, input_sec_refs) =
        sections::merge_sections(&input_objs);

    // ── Phase 3: Build global symbol table ───────────────────────────

    let mut sec_mapping: FxHashMap<(usize, usize), (usize, u64)> = FxHashMap::default();
    for r in &input_sec_refs {
        sec_mapping.insert(
            (r.obj_idx, r.sec_idx),
            (r.merged_sec_idx, r.offset_in_merged),
        );
    }

    let mut global_syms = symbols::build_global_symbols(
        &input_objs,
        &sec_mapping,
        &mut merged_sections,
        &mut merged_map,
    );

    // Identify GOT entries needed
    let (got_symbols, tls_got_symbols, local_got_sym_info) =
        symbols::collect_got_entries(&input_objs);

    // ── Phase 4+: Emit shared library ───────────────────────────────

    super::emit_shared::emit_shared_library(
        &input_objs,
        &mut merged_sections,
        &mut merged_map,
        &sec_mapping,
        &mut global_syms,
        &got_symbols,
        &tls_got_symbols,
        &local_got_sym_info,
        &needed_sonames,
        &shared_lib_syms,
        soname,
        rpath.as_ref(),
        output_path,
    )
}

/// Append the trailing startup objects (`crtend*.o`, `crtn.o`) once archive
/// members are resolved. They define and reference nothing a library
/// member could need, so loading them last changes no symbol resolution.
fn load_trailing_crt_objects(
    crt_objects_after: &[&str],
    input_objs: &mut Vec<(String, ElfObject)>,
) -> Result<(), String> {
    let paths: Vec<String> = crt_objects_after.iter().map(|s| s.to_string()).collect();
    let (mut archives, mut shared) = (Vec::new(), Vec::new());
    input::load_input_files(&paths, input_objs, &mut archives, &mut shared)?;
    if let Some(p) = archives.first().or(shared.first().map(|(path, _)| path)) {
        return Err(format!("{p}: trailing startup file is not an object"));
    }
    Ok(())
}

/// `--warn-common`: the objects in link order, then the libraries in the
/// order they were read -- the order this linker resolves them in.
/// The `--as-needed` state of a shared object named on the command line:
/// that of its position, else GNU's `--no-as-needed` default (an operand the
/// driver passed through is still one the user named).
/// The `--as-needed` state of the positional shared library `path`: the one
/// the caller recorded for it among `object_files` (the standalone linker's
/// operands; the last mention wins), else the one it was named under in the
/// arguments (`positional_as_needed`).
fn operand_as_needed(
    object_files: &[&str],
    object_as_needed: &[bool],
    inputs: &[linker_common::InputItem],
    path: &str,
) -> bool {
    object_files
        .iter()
        .rposition(|&f| f == path)
        .and_then(|i| object_as_needed.get(i).copied())
        .unwrap_or_else(|| positional_as_needed(inputs, path))
}

fn positional_as_needed(inputs: &[linker_common::InputItem], path: &str) -> bool {
    inputs
        .iter()
        .rev()
        .find(|it| !it.is_lib && it.name == path)
        .is_some_and(|it| it.as_needed)
}

/// `--warn-common` in this linker's link order: the objects read before the
/// libraries (`input_objs[..late_from]`), the libraries in the link, then
/// the archive members and trailing objects read after them.
fn report_common_warnings(
    warn_common: bool,
    fatal_warnings: bool,
    input_objs: &[(String, ElfObject)],
    late_from: usize,
    shared_lib_syms: &input::SharedLibSyms,
) -> Result<(), String> {
    if !warn_common {
        return Ok(());
    }
    let mut common = linker_common::warn_common::CommonDiagnostics::default();
    let (early, late) = input_objs.split_at(late_from);
    for (_, obj) in early {
        common.add_object(obj);
    }
    shared_lib_syms.replay_libraries(&mut common);
    for (_, obj) in late {
        common.add_object(obj);
    }
    linker_common::warn_common::report_common_warnings(&common.into_warnings(), fatal_warnings)
}
