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
    let mut opts = check_capabilities(&args, user_args, is_shared)?;
    if let Some(missing) = args.missing_inputs.first() {
        return Err(format!(
            "cannot find {}: No such file or directory",
            missing
        ));
    }
    if !opts.dynamic_list_paths.is_empty() {
        opts.dynamic_list =
            linker_common::DynamicList::parse_files(&opts.dynamic_list_paths)?.unwrap_or_default();
    }
    Ok((args, opts))
}

fn trace_inputs_32(inputs: &[InputObject], resolved: &super::input::ResolvedInputs, trace: bool) {
    if !trace {
        return;
    }
    for obj in inputs {
        println!("{}", obj.filename);
    }
    for lib in &resolved.shared_libs {
        println!("{}", lib.soname);
    }
}

fn apply_visibility_controls_32(
    inputs: &[InputObject],
    global_symbols: &mut FxHashMap<String, LinkerSymbol>,
    opts: &LinkOptions,
) -> Result<(), String> {
    if !opts.exclude_libs.is_empty() {
        for obj in inputs {
            if !linker_common::exclude_libs_matches(&opts.exclude_libs, &obj.filename) {
                continue;
            }
            for sym in &obj.symbols {
                if sym.section_index != SHN_UNDEF
                    && sym.binding != STB_LOCAL
                    && !sym.name.is_empty()
                {
                    if let Some(gs) = global_symbols.get_mut(&sym.name) {
                        if gs.is_defined && !gs.is_dynamic && gs.visibility == STV_DEFAULT {
                            gs.visibility = STV_HIDDEN;
                        }
                    }
                }
            }
        }
    }
    if let Some(ref vs_path) = opts.version_script {
        let vs = linker_common::VersionScript::parse(vs_path)
            .ok_or_else(|| format!("failed to parse version script '{vs_path}'"))?;
        for (name, gs) in global_symbols.iter_mut() {
            if !gs.is_defined || gs.is_dynamic || gs.visibility != STV_DEFAULT {
                continue;
            }
            let base = name.split('@').next().unwrap_or(name);
            let is_local = (vs.any_local_star() && !vs.matches_global(base))
                || vs.nodes.iter().any(|n| {
                    n.local_patterns
                        .iter()
                        .any(|p| linker_common::wildcard_match_pattern(p, base))
                });
            if is_local {
                gs.visibility = STV_HIDDEN;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn finish_pie_32(
    inputs: &[InputObject],
    global_symbols: &mut FxHashMap<String, LinkerSymbol>,
    output_sections: &mut Vec<OutputSection>,
    section_name_to_idx: &FxHashMap<String, usize>,
    section_map: &SectionMap,
    resolved: &super::input::ResolvedInputs,
    dso_refs: &FxHashSet<String>,
    args: &LinkerArgs,
    opts: &LinkOptions,
    pending_defsyms: &PendingDefsyms,
    output_path: &str,
) -> Result<(), String> {
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
                referenced.insert(entry.lib.clone());
            }
            None if sym.binding != STB_WEAK
                && !super::shared::is_emitter_defined(&sym.name, section_name_to_idx) =>
            {
                unresolved.push(&sym.name);
            }
            None => {}
        }
    }
    if !unresolved.is_empty() {
        unresolved.sort_unstable();
        return Err(unresolved
            .iter()
            .map(|n| format!("undefined reference to `{n}'"))
            .collect::<Vec<_>>()
            .join("\n"));
    }
    let needed_sonames = resolved.needed_sonames(&referenced);
    let interp_buf: Option<Vec<u8>> =
        if !resolved.shared_libs.is_empty() || args.dynamic_linker.is_some() {
            Some(match args.dynamic_linker.as_deref() {
                Some(path) => {
                    if path.is_empty() || path.as_bytes().contains(&0) {
                        return Err("invalid --dynamic-linker path".into());
                    }
                    let mut b = path.as_bytes().to_vec();
                    b.push(0);
                    b
                }
                None => INTERP.to_vec(),
            })
        } else {
            None
        };
    emit_shared_library_32(
        inputs,
        global_symbols,
        output_sections,
        section_name_to_idx,
        section_map,
        &needed_sonames,
        &resolved.dynlib_syms,
        interp_buf.as_deref(),
        dso_refs,
        output_path,
        opts,
        pending_defsyms,
    )
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
    if opts.is_pie && user_static {
        return Err("-static-pie is not supported by the elf_i386 userspace linker".into());
    }

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
    trace_inputs_32(&inputs, &resolved, opts.trace);

    // Phase 4b: one merged GNU property note (CET / ISA level) in place of
    // the per-input copies; every object here is a real input.
    merge_gnu_properties(&mut inputs, &opts.properties);

    // Phase 5: Merge sections
    let (mut output_sections, mut section_name_to_idx, section_map) = merge_sections(&mut inputs);

    // Phase 6: Resolve symbols
    let no_dynlib_syms: DynlibSyms = FxHashMap::default();
    let dynlib_for_resolve = if opts.is_pie {
        &no_dynlib_syms
    } else {
        &resolved.dynlib_syms
    };
    let (mut global_symbols, sym_resolution) =
        resolve_symbols(&inputs, &output_sections, &section_map, dynlib_for_resolve);

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
    define_tls_module_base(&mut global_symbols, &output_sections);
    apply_visibility_controls_32(&inputs, &mut global_symbols, &opts)?;

    if opts.is_pie {
        return finish_pie_32(
            &inputs,
            &mut global_symbols,
            &mut output_sections,
            &section_name_to_idx,
            &section_map,
            &resolved,
            &dso_refs,
            &args,
            &opts,
            &pending_defsyms,
            output_path,
        );
    }

    // Phase 7: Mark PLT/GOT needs and check undefined
    mark_plt_got_needs(&inputs, &mut global_symbols, is_static);

    check_undefined_symbols(&global_symbols)?;

    // Phase 8: Build PLT/GOT structures
    let (plt_symbols, got_dyn_symbols, got_local_symbols, num_plt, num_got_total) =
        build_plt_got_lists(&mut global_symbols);

    // Phase 8b: a copy relocation moves an object, not a name.  Every other
    // data export of the same library at the same address -- glibc's strong
    // `__environ` behind the weak `environ` a program names -- must be
    // defined at the copy too, or the library keeps using the original
    // while the program sees the copy.
    if !is_static {
        register_copy_aliases(&mut global_symbols, &resolved.dynlib_syms);
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

/// `--defsym` definitions planned before layout and finished after it (see
/// `linker_common::defsym::DefsymPlan` for the binding rules, which are GNU
/// ld's, and the x86-64 backend's identical `PendingDefsyms`).
pub(super) struct PendingDefsyms {
    plan: crate::backend::linker_common::defsym::DefsymPlan,
    /// Input definitions a `--defsym` overrides but also reads (`a=a+1`),
    /// as they were before the override.
    shadow: FxHashMap<String, LinkerSymbol>,
    /// Finally-defined names whose value is computed, with the position of
    /// the defining statement.
    computed: Vec<(String, usize)>,
}

/// Apply `--defsym SYMBOL=EXPRESSION` to the global symbol table.
///
/// An alias of an input symbol (directly or through a chain of `--defsym`
/// aliases) copies the whole definition, exactly as GNU ld makes it the
/// same symbol.  Every other definition is inserted as a defined absolute
/// placeholder (`output_section == usize::MAX` is this backend's "no
/// section" marker) so the undefined-symbol check and the PLT/GOT need scan
/// see it; its value is computed after layout by
/// [`evaluate_pending_defsyms`], when the referenced symbols have their
/// final addresses.
fn apply_defsyms(
    global_symbols: &mut FxHashMap<String, LinkerSymbol>,
    defs: &[(String, String)],
) -> Result<PendingDefsyms, String> {
    use crate::backend::linker_common::defsym::{DefsymPlan, LinkSym};
    // "Defined" means what it means to `check_undefined_symbols`; a symbol
    // only a shared library defines is not defined in this link.
    let plan = DefsymPlan::new(defs, |n| match global_symbols.get(n) {
        Some(g) if g.is_defined && !g.is_dynamic => {
            if g.output_section == usize::MAX {
                LinkSym::Absolute
            } else {
                LinkSym::Address
            }
        }
        _ => LinkSym::Undefined,
    })?;
    let shadow: FxHashMap<String, LinkerSymbol> = plan
        .shadowed_names()
        .into_iter()
        .filter_map(|n| global_symbols.get(n).map(|g| (n.to_string(), g.clone())))
        .collect();
    let mut computed = Vec::new();
    for st in plan.finals() {
        let copied = plan.aliased_link_symbol(st).and_then(|t| {
            shadow
                .get(t)
                .or_else(|| global_symbols.get(t))
                .filter(|g| g.is_defined && !g.is_dynamic)
                .cloned()
        });
        if let Some(sym) = copied {
            global_symbols.insert(st.name.clone(), sym);
            continue;
        }
        computed.push((st.name.clone(), st.index - 1));
        global_symbols.insert(
            st.name.clone(),
            LinkerSymbol {
                address: 0,
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
                lib_value: 0,
                lib_in_exec: false,
                canonical_plt: false,
            },
        );
    }
    Ok(PendingDefsyms {
        plan,
        shadow,
        computed,
    })
}

/// Compute the `--defsym` values against the finalised symbol table.
///
/// Must run inside the emitter, after (1) section addresses are assigned to
/// every global symbol and (2) the linker-provided symbols (`_end`, `end`,
/// `_etext`, `__bss_start`, …) are seeded, so an expression sees exactly
/// the values GNU ld's language-symbol machinery would see.  `place` is the
/// emitter's rule giving an input symbol's final address; it finishes the
/// shadowed definitions.  Arithmetic wraps as unsigned 64-bit and the
/// result is truncated to the 32-bit address space, as in GNU ld.
pub(super) fn evaluate_pending_defsyms(
    global_symbols: &mut FxHashMap<String, LinkerSymbol>,
    pending: &PendingDefsyms,
    place: &dyn Fn(&LinkerSymbol) -> u32,
) -> Result<(), String> {
    if pending.computed.is_empty() {
        return Ok(());
    }
    let needed: Vec<usize> = pending.computed.iter().map(|&(_, pos)| pos).collect();
    let values = pending
        .plan
        .evaluate(&needed, |n| match pending.shadow.get(n) {
            Some(g) => g.is_defined.then(|| place(g) as u64),
            None => global_symbols
                .get(n)
                .filter(|g| g.is_defined)
                .map(|g| g.address as u64),
        })?;
    for (name, pos) in &pending.computed {
        let entry = global_symbols
            .get_mut(name)
            .ok_or_else(|| format!("--defsym {name}: symbol vanished before evaluation"))?;
        entry.address = values[*pos]
            .ok_or_else(|| format!("internal error: --defsym {name} was not evaluated"))?
            as u32;
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
    trace_inputs_32(&inputs, &resolved, opts.trace);

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
    define_tls_module_base(&mut global_symbols, &output_sections);
    apply_visibility_controls_32(&inputs, &mut global_symbols, &opts)?;

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
                referenced.insert(entry.lib.clone());
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
        &resolved.dynlib_syms,
        None,
        &FxHashSet::default(),
        output_path,
        &opts,
        &pending_defsyms,
    )
}

/// Define every alias of a copy-relocated shared-library object at the
/// copy (GNU ld's weakdef handling, `_bfd_elf_adjust_dynamic_symbol`).
///
/// Two default-version data exports of one library with the same
/// `st_value` name one object.  When the executable copy-relocates either,
/// the others join the copy: `emit` gives the group one `.bss` slot and one
/// `R_386_COPY`, and exports every member there, so ld.so binds the
/// library's own references (through whichever name) to the copy.
///
/// This replaces a text-relocation scheme: a WEAK data import used to skip
/// the copy (because its strong alias would have stayed behind) and patch
/// every reference in the code instead -- a `DT_TEXTREL` executable for
/// every non-PIC program that names `environ`, where GNU ld emits a copy.
pub(super) fn register_copy_aliases(
    global_symbols: &mut FxHashMap<String, LinkerSymbol>,
    dynlib_syms: &DynlibSyms,
) {
    let copied: FxHashSet<(String, u32)> = global_symbols
        .values()
        .filter(|s| {
            s.is_dynamic
                && s.needs_copy
                && copy_data_type(s.sym_type, s.lib_in_exec)
                && s.lib_value != 0
        })
        .map(|s| (s.dynlib.clone(), s.lib_value))
        .collect();
    if copied.is_empty() {
        return;
    }
    for (name, d) in dynlib_syms {
        if !copy_data_type(d.sym_type, d.in_exec)
            || !d.is_default_ver
            || !copied.contains(&(d.lib.clone(), d.value))
        {
            continue;
        }
        match global_symbols.get_mut(name) {
            // Not referenced by the executable: it still has to be defined
            // at the copy for the library's sake.
            None => {
                let mut alias = LinkerSymbol::dynamic_import(d);
                alias.needs_copy = true;
                global_symbols.insert(name.clone(), alias);
            }
            // Referenced only through the GOT, or not at all yet: join the
            // copy so its GLOB_DAT resolves there too.
            Some(g) if g.is_dynamic && g.dynlib == d.lib => g.needs_copy = true,
            // Defined by the executable itself (it interposes), or bound to
            // another library: not an alias of this copy.
            Some(_) => {}
        }
    }
}

/// The gnu2 Local-Dynamic anchor: `leal _TLS_MODULE_BASE_@tlsdesc(%ebx)`
/// yields the base of this module's TLS block once, and every local variable
/// is then addressed as `%gs:v@dtpoff(%eax)`.  GCC emits it for
/// `-m32 -mtls-dialect=gnu2 -fPIC`; no object defines it -- the linker does.
pub(super) const TLS_MODULE_BASE: &str = "_TLS_MODULE_BASE_";

/// Define a referenced `_TLS_MODULE_BASE_` as GNU ld and lld do: a HIDDEN
/// `STT_TLS` symbol -- never imported, exported or preempted, so its
/// descriptor is resolved against symbol 0 (this module) with the block
/// offset in the slot.  `output_section == usize::MAX` keeps the generic
/// address pass off it; the emitters place it with
/// [`tls_module_base_address`].  Without TLS output there is nothing to
/// anchor and the reference stays undefined (and is reported as such).
fn define_tls_module_base(
    global_symbols: &mut FxHashMap<String, LinkerSymbol>,
    output_sections: &[OutputSection],
) {
    let Some(g) = global_symbols.get_mut(TLS_MODULE_BASE) else {
        return;
    };
    if g.is_defined || g.is_dynamic {
        return;
    }
    if !output_sections.iter().any(|s| s.flags & SHF_TLS != 0) {
        return;
    }
    g.is_defined = true;
    g.sym_type = STT_TLS;
    g.visibility = STV_HIDDEN;
    g.output_section = usize::MAX;
    g.section_offset = 0;
    g.address = 0;
}

/// Final address of a linker-defined `_TLS_MODULE_BASE_`: the start of the
/// TLS block in a shared object (`v@dtpoff` is the offset in the block); in
/// an executable the descriptor is relaxed to `movl $ntpoff,%eax` and
/// `v@dtpoff` resolves TP-relative, so the anchor is the END of the block
/// (the thread pointer, ntpoff 0), as GNU ld and lld place it.
pub(super) fn tls_module_base_address(
    global_symbols: &mut FxHashMap<String, LinkerSymbol>,
    tls_addr: u32,
    tls_mem_size: u32,
    executable: bool,
) {
    if let Some(g) = global_symbols.get_mut(TLS_MODULE_BASE)
        && g.is_defined
        && g.sym_type == STT_TLS
        && g.output_section == usize::MAX
    {
        g.address = if executable {
            tls_addr + tls_mem_size
        } else {
            tls_addr
        };
    }
}
