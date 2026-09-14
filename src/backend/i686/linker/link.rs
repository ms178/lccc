//! i686 linker orchestration.
//!
//! Contains the two public entry points (`link_builtin` and `link_shared`) that
//! orchestrate the linking pipeline: parse arguments, load inputs, merge sections,
//! resolve symbols, build PLT/GOT, and emit the ELF32 executable or shared library.

use crate::common::fx_hash::FxHashMap;
use std::path::Path;

use super::emit::emit_executable;
use super::input::*;
use super::sections::merge_sections;
use super::shared::{emit_shared_library_32, resolve_dynamic_symbols_for_shared};
use super::symbols::*;
use super::types::*;

/// Built-in linker entry point with pre-resolved CRT objects and library paths.
pub fn link_builtin(
    object_files: &[&str],
    output_path: &str,
    user_args: &[String],
    lib_paths: &[&str],
    needed_libs_param: &[&str],
    crt_objects_before: &[&str],
    crt_objects_after: &[&str],
) -> Result<(), String> {
    let is_nostdlib = user_args.iter().any(|a| a == "-nostdlib");
    // The driver normalizes `-static` to the single-letter token `n`
    // (cli.rs: `"n" => self.static_link = true`) before the backend sees
    // it; accept both spellings.  Missing the `n` form made every
    // `-static` i686 link emit a DYNAMIC executable (PT_INTERP pointing
    // at /lib/ld-linux.so.2) — the exact failure of the i686 regression
    // cluster on loader-less hosts.
    let is_static = user_args
        .iter()
        .any(|a| a == "-static" || a == "n" || a == "-n");

    // Phase 1: Parse arguments and collect file lists
    let (extra_libs, extra_lib_files, extra_lib_paths, extra_objects, defsym_defs) =
        parse_user_args(user_args);

    let all_lib_dirs: Vec<String> = extra_lib_paths
        .into_iter()
        .chain(lib_paths.iter().map(|s| s.to_string()))
        .collect();

    // Phase 2: Collect all input objects in link order
    let all_objects = collect_input_files(
        object_files,
        &extra_objects,
        crt_objects_before,
        crt_objects_after,
        is_nostdlib,
        is_static,
        lib_paths,
    );

    // Phase 3: Load dynamic library symbols and resolve static libs from -l flags
    let (dynlib_syms, static_lib_objects) = load_libraries(
        is_static,
        is_nostdlib,
        needed_libs_param,
        &extra_libs,
        &extra_lib_files,
        &all_lib_dirs,
    );

    // Phase 4: Parse all input objects and archives
    let mut all_objs = all_objects;
    for lib_path in &static_lib_objects {
        all_objs.push(lib_path.clone());
    }

    let (inputs, _archive_pool) = load_and_parse_objects(&all_objs, &defsym_defs)?;

    // Phase 5: Merge sections
    let (mut output_sections, mut section_name_to_idx, section_map) = merge_sections(&inputs);

    // Phase 6: Resolve symbols
    let (mut global_symbols, sym_resolution) =
        resolve_symbols(&inputs, &output_sections, &section_map, &dynlib_syms);

    // Phase 6b: Allocate COMMON symbols in .bss
    allocate_common_symbols(
        &inputs,
        &mut output_sections,
        &mut section_name_to_idx,
        &mut global_symbols,
    );

    // Apply --defsym definitions: alias, constant or arithmetic expression
    // (see `apply_defsyms` below for the rationale; the classification lives
    // in linker_common::defsym so the backends cannot disagree about what
    // `--defsym a=b+4` means).
    //
    // Order matters: the placeholders must exist BEFORE `mark_plt_got_needs`
    // scans the input relocations. A defsym'd symbol referenced from PIC code
    // (GOT load) needs a GOT slot; created after the scan it never gets one
    // and the link silently emits a load from an unallocated slot.
    let pending_defsyms = apply_defsyms(&mut global_symbols, &defsym_defs)?;

    // Phase 7: Mark PLT/GOT needs and check undefined
    mark_plt_got_needs(&inputs, &mut global_symbols, is_static);

    check_undefined_symbols(&global_symbols)?;

    // Phase 8: Build PLT/GOT structures
    let (plt_symbols, got_dyn_symbols, got_local_symbols, num_plt, num_got_total) =
        build_plt_got_lists(&mut global_symbols);

    // Phase 8b: Mark WEAK dynamic data symbols for text relocations instead of COPY
    if !is_static {
        let weak_data_syms: Vec<String> = global_symbols
            .iter()
            .filter(|(_, s)| {
                s.is_dynamic
                    && s.needs_copy
                    && s.binding == STB_WEAK
                    && s.sym_type != STT_FUNC
                    && s.sym_type != STT_GNU_IFUNC
            })
            .map(|(n, _)| n.clone())
            .collect();
        for name in &weak_data_syms {
            if let Some(sym) = global_symbols.get_mut(name) {
                sym.needs_copy = false;
                sym.uses_textrel = true;
            }
        }
    }

    // Phase 9: Collect IFUNC symbols for static linking
    let ifunc_symbols = collect_ifunc_symbols(&global_symbols, is_static);

    // Phase 10: Layout + emit
    emit_executable(
        &inputs,
        &mut output_sections,
        &section_name_to_idx,
        &section_map,
        &mut global_symbols,
        &sym_resolution,
        &dynlib_syms,
        &plt_symbols,
        &got_dyn_symbols,
        &got_local_symbols,
        num_plt,
        num_got_total,
        &ifunc_symbols,
        is_static,
        is_nostdlib,
        needed_libs_param,
        output_path,
        &pending_defsyms,
    )
}

/// Apply `--defsym` definitions to the resolved symbol table.
///
/// The right-hand side is classified once, in `linker_common::defsym`, by
/// every link path so the backends cannot disagree:
/// * **Alias** (`a=b`): copy the target's whole symbol, exactly as the old
///   alias-only loop did. Resolvable before layout.
/// * **Constant** (`a=0x100`): a new absolute symbol with that value.
///   Resolvable before layout.
/// * **Expression** (`a=(b-c)/2`): validated now (a typo or a division by
///   zero must fail the link here) but evaluated after layout, when the
///   referenced symbols have their final addresses. The returned list is the
///   set of pending `(name, expression)` pairs for the emitter.
///
/// Constants and expressions are inserted as defined absolute symbols
/// (`output_section == usize::MAX` is this backend's "no section" marker);
/// the undefined-symbol check and the PLT/GOT need scan both see them.
fn apply_defsyms(
    global_symbols: &mut FxHashMap<String, LinkerSymbol>,
    defs: &[(String, String)],
) -> Result<Vec<(String, String, usize)>, String> {
    use crate::backend::linker_common::defsym::{self, Defsym, DefsymError};
    let mut pending: Vec<(String, String, usize)> = Vec::new();
    for (pos, (name, expr)) in defs.iter().enumerate() {
        let index = pos + 1;
        // "Defined" means the same thing it means to
        // `check_undefined_symbols`: the symbol is resolved in this link.
        // Layout-derived magic symbols (`_etext`, `end`, …) count as defined
        // even though they only exist after layout — GNU ld resolves them
        // during expression evaluation, and so do we (see
        // `defsym::is_linker_defined`).
        let classified = defsym::classify(expr, |n| {
            global_symbols.get(n).is_some_and(|g| g.is_defined) || defsym::is_linker_defined(n)
        })
        .map_err(|e| e.gnu_message(index))?;
        let value = match classified {
            // Alias: copy the target's whole definition, exactly as the old
            // alias-only loop did. A target only the linker defines (a magic
            // symbol) has no address until layout, so it is deferred to
            // evaluation like an expression.
            Defsym::Alias(target) => {
                match global_symbols.get(&target) {
                    Some(sym) if sym.is_defined => {
                        if sym.is_dynamic {
                            // GNU ld: a symbol visible only through a shared
                            // library is not "defined in this link", so
                            // aliasing it is an undefined-symbol error.
                            return Err(DefsymError::UndefinedSymbol(target).gnu_message(index));
                        }
                        global_symbols.insert(name.clone(), sym.clone());
                        continue;
                    }
                    _ => {
                        // Linker language symbol (`_etext`, `end`, …): no
                        // address until layout. The arm's 0 becomes the usual
                        // placeholder (defined, ABS) below, so the
                        // undefined-symbol check and the PLT/GOT need scan see
                        // it; `evaluate_pending_defsyms` overwrites the value
                        // after layout.
                        pending.push((name.clone(), target, index));
                        0
                    }
                }
            }
            Defsym::Constant(v) => v,
            // Validated now, evaluated after layout.
            Defsym::Expression(e) => {
                pending.push((name.clone(), e, index));
                0
            }
        };
        global_symbols.insert(
            name.clone(),
            LinkerSymbol {
                address: value as u32,
                size: 0,
                sym_type: STT_NOTYPE,
                binding: STB_GLOBAL,
                visibility: STV_DEFAULT,
                is_defined: true,
                needs_plt: false,
                needs_got: false,
                output_section: usize::MAX,
                section_offset: 0,
                plt_index: 0,
                got_index: 0,
                is_dynamic: false,
                dynlib: String::new(),
                needs_copy: false,
                copy_addr: 0,
                version: None,
                uses_textrel: false,
            },
        );
    }
    Ok(pending)
}

/// Evaluate pending `--defsym` expressions against the finalised symbol
/// table.
///
/// Must run inside the emitter, after (1) section addresses are assigned to
/// every global symbol and (2) the linker-provided symbols (`_end`, `end`,
/// `_etext`, `__bss_start`, …) are seeded, so the expression sees exactly
/// the values GNU ld's language-symbol machinery would see.
///
/// Lookup mirrors the classification predicate: only defined symbols
/// participate, an expression over an undefined name is an error rather
/// than a silent zero, and arithmetic wraps as unsigned 64-bit.
pub(super) fn evaluate_pending_defsyms(
    global_symbols: &mut FxHashMap<String, LinkerSymbol>,
    pending: &[(String, String, usize)],
) -> Result<(), String> {
    use crate::backend::linker_common::defsym;
    for (name, expr, index) in pending {
        let value = defsym::eval_with_symbols(expr, &|n| {
            global_symbols
                .get(n)
                .filter(|g| g.is_defined)
                .map(|g| g.address as u64)
        })
        .map_err(|err| err.gnu_message(*index))?;
        let entry = global_symbols
            .get_mut(name)
            .ok_or_else(|| format!("--defsym {name}: symbol vanished before evaluation"))?;
        entry.address = value as u32;
        entry.is_defined = true;
    }
    Ok(())
}

// ══════════════════════════════════════════════════════════════════════════════
// Shared library linker (-shared)
// ══════════════════════════════════════════════════════════════════════════════

/// Create a shared library (.so) from ELF32 object files.
///
/// Produces an ELF32 `ET_DYN` file with base address 0, exporting all defined
/// global symbols. Used when the compiler is invoked with `-shared`.
pub fn link_shared(
    object_files: &[&str],
    output_path: &str,
    user_args: &[String],
    lib_paths: &[&str],
) -> Result<(), String> {
    // Parse user args for -L, -l, -Wl,-soname=, bare .o/.a files
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
            let mut j = 0;
            while j < parts.len() {
                let part = parts[j];
                if let Some(sn) = part.strip_prefix("-soname=") {
                    soname = Some(sn.to_string());
                } else if part == "-soname" && j + 1 < parts.len() {
                    j += 1;
                    soname = Some(parts[j].to_string());
                } else if let Some(lpath) = part.strip_prefix("-L") {
                    extra_lib_paths.push(lpath.to_string());
                } else if let Some(lib) = part.strip_prefix("-l") {
                    libs_to_load.push(lib.to_string());
                } else if let Some(defsym_arg) = part.strip_prefix("--defsym=") {
                    // --defsym=SYM=EXPR: alias, constant or arithmetic
                    // expression (same semantics as the executable path).
                    if let Some(eq_pos) = defsym_arg.find('=') {
                        defsym_defs.push((
                            defsym_arg[..eq_pos].to_string(),
                            defsym_arg[eq_pos + 1..].to_string(),
                        ));
                    }
                } else if part == "--defsym" && j + 1 < parts.len() {
                    // Two-argument form: --defsym SYM=EXPR
                    j += 1;
                    if let Some(eq_pos) = parts[j].find('=') {
                        defsym_defs.push((
                            parts[j][..eq_pos].to_string(),
                            parts[j][eq_pos + 1..].to_string(),
                        ));
                    }
                }
                j += 1;
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

    // Collect all objects to parse
    let mut all_objs: Vec<String> = object_files.iter().map(|s| s.to_string()).collect();
    all_objs.extend(extra_object_files);

    // Parse all input objects
    let (inputs, _archive_pool) = load_and_parse_objects(&all_objs, &defsym_defs)?;

    // Merge sections
    let (mut output_sections, section_name_to_idx, section_map) = merge_sections(&inputs);

    // Resolve symbols (no dynamic library symbols for shared lib output)
    let dynlib_syms: FxHashMap<String, (String, u8, u32, Option<String>, bool, u8)> =
        FxHashMap::default();
    let (mut global_symbols, _sym_resolution) =
        resolve_symbols(&inputs, &output_sections, &section_map, &dynlib_syms);

    // Apply --defsym definitions (same order and rationale as the executable
    // path: aliases/constants take effect here, expressions are evaluated
    // after the layout in `emit_shared_library_32`).
    let pending_defsyms = apply_defsyms(&mut global_symbols, &defsym_defs)?;

    // Load -l libraries (resolve into archives and load them)
    let lib_path_strings: Vec<String> = lib_paths.iter().map(|s| s.to_string()).collect();
    let mut all_lib_paths: Vec<String> = extra_lib_paths;
    all_lib_paths.extend(lib_path_strings.iter().cloned());

    if !libs_to_load.is_empty() {
        for lib_name in &libs_to_load {
            // Search for static archive only in shared library mode
            for dir in &all_lib_paths {
                let cand = format!("{}/lib{}.a", dir, lib_name);
                if Path::new(&cand).exists() {
                    let objs = vec![cand];
                    let (extra_inputs, _) = load_and_parse_objects(&objs, &defsym_defs)?;
                    // Add symbols from these archives
                    for _inp in &extra_inputs {
                        // TODO: properly merge archive objects
                    }
                    break;
                }
            }
        }
    }

    // Discover NEEDED dependencies by scanning for undefined symbols
    let mut needed_sonames: Vec<String> = Vec::new();
    resolve_dynamic_symbols_for_shared(
        &inputs,
        &global_symbols,
        &mut needed_sonames,
        &all_lib_paths,
    );

    // Emit shared library
    emit_shared_library_32(
        &inputs,
        &mut global_symbols,
        &mut output_sections,
        &section_name_to_idx,
        &section_map,
        &needed_sonames,
        output_path,
        soname,
        &pending_defsyms,
    )
}
