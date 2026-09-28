//! i686 linker orchestration.
//!
//! Contains the two public entry points (`link_builtin` and `link_shared`) that
//! orchestrate the linking pipeline: validate options, resolve the ordered
//! inputs, merge sections, resolve symbols, build PLT/GOT, and emit the ELF32
//! executable or shared library.

use crate::backend::linker_common::{self, LinkerArgs};
use crate::common::fx_hash::{FxHashMap, FxHashSet};

use super::emit::emit_executable;
use super::input::*;
use super::options::{LinkOptions, check_capabilities};
use super::sections::{merge_gnu_properties, merge_sections};
use super::shared::emit_shared_library_32;
use super::symbols::*;
use super::types::*;

/// How the compiler driver's implicit libraries are linked.  Mirrors gcc's
/// libgcc spec: `-lgcc` is always the static archive (there is no
/// `libgcc.so`), the unwinder `gcc_s` is `--as-needed`, libc is
/// unconditional, and lccc's convenience `-lm` is as-needed and optional.
fn driver_lib_item(name: &str, link_static: bool) -> LinkItem {
    let mut item = match name {
        "gcc" | "gcc_eh" => LinkItem::lib(name, false, true),
        "gcc_s" => LinkItem::lib(name, true, false),
        "c" => {
            let mut it = LinkItem::lib(name, false, false);
            it.driver_libc = true;
            it
        }
        "m" => {
            let mut it = LinkItem::lib(name, true, false);
            it.optional = true;
            it
        }
        other => LinkItem::lib(other, false, false),
    };
    item.static_search |= link_static;
    item
}

/// Build the ordered input list: CRT start files, the driver's compiled
/// objects, the user's operands in command-line order, CRT end files, then
/// the driver's implicit libraries.  A user operand naming a file the
/// driver already passed as an object is not linked twice.
fn ordered_items(
    object_files: &[&str],
    args: &LinkerArgs,
    crt_before: &[&str],
    crt_after: &[&str],
    driver_libs: &[&str],
    link_static: bool,
) -> Vec<LinkItem> {
    let canon = |p: &str| std::fs::canonicalize(p).unwrap_or_else(|_| p.into());
    let driver_objs: FxHashSet<std::path::PathBuf> =
        object_files.iter().map(|p| canon(p)).collect();
    let mut items: Vec<LinkItem> = Vec::new();
    // CRT paths come from toolchain probing that may name a file a minimal
    // sysroot lacks (crtbeginT.o fallback, crti.o in the GCC dir): only
    // existing ones are linked.
    let existing = |p: &&&str| std::path::Path::new(**p).exists();
    items.extend(
        crt_before
            .iter()
            .filter(existing)
            .map(|p| LinkItem::file(p)),
    );
    items.extend(object_files.iter().map(|p| LinkItem::file(p)));
    for input in &args.inputs {
        if !input.is_lib && driver_objs.contains(&canon(&input.name)) {
            continue;
        }
        let mut item = LinkItem::from_input(input);
        item.static_search |= link_static;
        items.push(item);
    }
    items.extend(crt_after.iter().filter(existing).map(|p| LinkItem::file(p)));
    items.extend(driver_libs.iter().map(|l| driver_lib_item(l, link_static)));
    items
}

/// Library search path: `-L` directories in command-line order, then the
/// driver's directories, without duplicates.
fn search_dirs(args: &LinkerArgs, lib_paths: &[&str]) -> Vec<String> {
    let mut dirs: Vec<String> = Vec::new();
    for d in args
        .extra_lib_paths
        .iter()
        .map(String::as_str)
        .chain(lib_paths.iter().copied())
    {
        if !dirs.iter().any(|x| x == d) {
            dirs.push(d.to_string());
        }
    }
    dirs
}

/// Parse and validate the arguments common to both entry points.
fn prepare(user_args: &[String], is_shared: bool) -> Result<(LinkerArgs, LinkOptions), String> {
    let args = linker_common::parse_linker_args(user_args);
    let opts = check_capabilities(&args, user_args, is_shared)?;
    if let Some(missing) = args.missing_inputs.first() {
        return Err(format!(
            "cannot find {}: No such file or directory",
            missing
        ));
    }
    Ok((args, opts))
}

/// Sonames the resolved symbols bind to (for as-needed DT_NEEDED).
fn referenced_sonames(global_symbols: &FxHashMap<String, LinkerSymbol>) -> FxHashSet<String> {
    global_symbols
        .values()
        .filter(|s| s.is_dynamic && !s.dynlib.is_empty())
        .map(|s| s.dynlib.clone())
        .collect()
}

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
    let (args, opts) = prepare(user_args, false)?;
    // The driver normalizes `-static` to the single-letter token `n`
    // (cli.rs: `"n" => self.static_link = true`) before the backend sees
    // it; accept both spellings.
    let user_static = args.is_static || user_args.iter().any(|a| a == "n" || a == "-n");

    let dirs = search_dirs(&args, lib_paths);
    let items = ordered_items(
        object_files,
        &args,
        crt_objects_before,
        crt_objects_after,
        needed_libs_param,
        user_static,
    );
    let mut resolved = resolve_inputs(&ResolveRequest {
        items: &items,
        lib_dirs: &dirs,
        undefined: &args.undefined_symbols,
        defsyms: &args.defsym_defs,
        allow_shared: !user_static,
        wrap: &opts.wrap,
    })?;
    // GNU ld: an executable is dynamic exactly when a shared object takes
    // part in the link.  A `-nostdlib` link of plain objects is static (no
    // PT_INTERP, no .dynamic) even without `-static`.
    let is_static = user_static || resolved.shared_libs.is_empty();
    let dso_refs = if is_static {
        FxHashSet::default()
    } else {
        resolved.dso_visible_names()
    };
    let mut inputs = std::mem::take(&mut resolved.objects);

    // Phase 4b: one merged GNU property note (CET / ISA level) in place of
    // the per-input copies; every object here is a real input.
    merge_gnu_properties(&mut inputs, &opts.properties);

    // Phase 5: Merge sections
    let (mut output_sections, mut section_name_to_idx, section_map) = merge_sections(&mut inputs);

    // Phase 6: Resolve symbols
    let (mut global_symbols, sym_resolution) = resolve_symbols(
        &inputs,
        &output_sections,
        &section_map,
        &resolved.dynlib_syms,
    );

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
    let pending_defsyms = apply_defsyms(&mut global_symbols, &args.defsym_defs)?;

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

    let needed = resolved.needed_sonames(&referenced_sonames(&global_symbols));

    // Phase 10: Layout + emit
    // PT_INTERP: honour `--dynamic-linker=PATH` (lccc-ld normalises every
    // GNU spelling to it; the compiler driver forwards `-Wl,…` forms, which
    // the shared parser also understands) instead of hard-wiring the ABI
    // loader — a staging/sysroot link must bind against the libc it names.
    let interp = args
        .dynamic_linker
        .clone()
        .map(|path| {
            let mut bytes = path.into_bytes();
            bytes.push(0);
            bytes
        })
        .unwrap_or_else(|| INTERP.to_vec());

    emit_executable(
        &inputs,
        &mut output_sections,
        &section_name_to_idx,
        &section_map,
        &mut global_symbols,
        &sym_resolution,
        &plt_symbols,
        &got_dyn_symbols,
        &got_local_symbols,
        num_plt,
        num_got_total,
        &ifunc_symbols,
        is_static,
        &needed,
        &opts,
        &dso_refs,
        output_path,
        &pending_defsyms,
        &interp,
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
                canonical_plt: false,
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
/// Produces an ELF32 `ET_DYN` file with base address 0.  Inputs are resolved
/// exactly like an executable's (ordered operands, `-l` search with `.so`
/// before `.a`, archive-member extraction, linker scripts); the shared
/// objects that end up linked supply the `DT_NEEDED` entries.
pub fn link_shared(
    object_files: &[&str],
    output_path: &str,
    user_args: &[String],
    lib_paths: &[&str],
    crt_objects_before: &[&str],
    crt_objects_after: &[&str],
    driver_libs: &[&str],
) -> Result<(), String> {
    let (args, opts) = prepare(user_args, true)?;
    let dirs = search_dirs(&args, lib_paths);
    let items = ordered_items(
        object_files,
        &args,
        crt_objects_before,
        crt_objects_after,
        driver_libs,
        false,
    );
    let mut resolved = resolve_inputs(&ResolveRequest {
        items: &items,
        lib_dirs: &dirs,
        undefined: &args.undefined_symbols,
        defsyms: &args.defsym_defs,
        allow_shared: true,
        wrap: &opts.wrap,
    })?;
    let mut inputs = std::mem::take(&mut resolved.objects);

    // One merged GNU property note, as in executables.
    merge_gnu_properties(&mut inputs, &opts.properties);

    // Merge sections
    let (mut output_sections, mut section_name_to_idx, section_map) = merge_sections(&mut inputs);

    // Resolve symbols.  The output's undefined references stay undefined
    // (they are bound at load time); the linked shared objects' dynamic
    // symbols decide DT_NEEDED and `--no-undefined` only.
    let no_dynlib_syms: DynlibSyms = FxHashMap::default();
    let (mut global_symbols, _sym_resolution) =
        resolve_symbols(&inputs, &output_sections, &section_map, &no_dynlib_syms);
    // Tentative definitions (`-fcommon`) get .bss space, as in executables.
    allocate_common_symbols(
        &inputs,
        &mut output_sections,
        &mut section_name_to_idx,
        &mut global_symbols,
    );

    // Apply --defsym definitions (same order and rationale as the executable
    // path: aliases/constants take effect here, expressions are evaluated
    // after the layout in `emit_shared_library_32`).
    let pending_defsyms = apply_defsyms(&mut global_symbols, &args.defsym_defs)?;

    // Undefined non-local references of the output, and which linked shared
    // object (first in command-line order) satisfies each.
    let mut referenced: FxHashSet<String> = FxHashSet::default();
    let mut unresolved: Vec<&str> = Vec::new();
    let mut seen: FxHashSet<&str> = FxHashSet::default();
    for sym in inputs.iter().flat_map(|o| o.symbols.iter()) {
        if sym.section_index != SHN_UNDEF
            || sym.name.is_empty()
            || sym.binding == STB_LOCAL
            || global_symbols
                .get(sym.name.as_str())
                .is_some_and(|g| g.is_defined)
            || !seen.insert(sym.name.as_str())
        {
            continue;
        }
        match resolved.dynlib_syms.get(sym.name.as_str()) {
            Some(entry) => {
                referenced.insert(entry.0.clone());
            }
            None if sym.binding != STB_WEAK
                && !super::shared::is_emitter_defined(&sym.name, &section_name_to_idx) =>
            {
                unresolved.push(&sym.name);
            }
            None => {}
        }
    }
    if opts.no_undefined && !unresolved.is_empty() {
        unresolved.sort_unstable();
        return Err(unresolved
            .iter()
            .map(|n| format!("undefined reference to `{n}' (--no-undefined)"))
            .collect::<Vec<_>>()
            .join("\n"));
    }
    let needed_sonames = resolved.needed_sonames(&referenced);

    // Emit shared library
    emit_shared_library_32(
        &inputs,
        &mut global_symbols,
        &mut output_sections,
        &section_name_to_idx,
        &section_map,
        &needed_sonames,
        output_path,
        &opts,
        &pending_defsyms,
    )
}
