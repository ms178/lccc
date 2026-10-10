//! x86-64 linker orchestration.
//!
//! Contains the two public entry points (`link_builtin` and `link_shared`) that
//! orchestrate the linking pipeline: load inputs, resolve symbols, merge sections,
//! build PLT/GOT, and dispatch to the appropriate ELF emission path.

use crate::backend::elf::{SHN_ABS, STB_GLOBAL, STT_NOTYPE};
use crate::backend::linker_common::SymStr;
use crate::backend::linker_common::defsym;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use std::path::Path;

use super::elf::*;
use super::emit_exec::emit_executable;
use super::emit_shared::emit_shared_library;
use super::icf;
use super::input::{load_file, load_file_as_needed};
use super::plt_got::{collect_ifunc_symbols, collect_local_ifuncs, create_plt_got};
use super::types::{GlobalSymbol, INTERP};
use crate::backend::linker_common::{self, OutputSection};

/// `--defsym` definitions planned before layout and finished after it.
///
/// The plan (`linker_common::defsym::DefsymPlan`) binds every reference and
/// fixes each definition's kind -- address or absolute -- once, before
/// layout, which is when the PLT/GOT need scan and the RELATIVE decisions
/// must see them.  Values that need final addresses are computed by
/// [`evaluate_pending_defsyms`] from the same plan, so the two phases cannot
/// disagree about what a right-hand side refers to.
pub(crate) struct PendingDefsyms {
    plan: defsym::DefsymPlan,
    /// Input definitions a `--defsym` overrides but also reads (`a=a+1`),
    /// as they were before the override; their final addresses come from
    /// the emitter's own placement rule.
    shadow: FxHashMap<String, GlobalSymbol>,
    /// Finally-defined names whose value is computed, with the position of
    /// the defining statement.  (Aliases of input symbols are complete
    /// copies made before layout and need nothing more.)
    computed: Vec<(String, usize)>,
}

/// Apply `--defsym SYMBOL=EXPRESSION` to the global symbol table.
///
/// Binding and classification live in `linker_common::defsym` so the ELF
/// backends cannot disagree about what a right-hand side means; its
/// [`defsym::DefsymPlan`] documents GNU ld's order semantics (earlier
/// definitions, forward references, self references) that it reproduces.
///
/// An alias of an input symbol (`--defsym a=real`, or a chain of `--defsym`
/// aliases ending there) copies the whole definition, PLT/GOT slots and
/// section included, so it IS that symbol.  Every other definition becomes
/// a linker-created symbol (`defined_in == Some(usize::MAX)`): an absolute
/// one (`SHN_ABS`, no relocation) or an address, as the plan classified it
/// -- which PIE/`.so` emission needs before layout.  Its value is computed
/// after layout ([`evaluate_pending_defsyms`]); constants need no layout
/// but go through the same path, so there is exactly one.
fn apply_defsyms(
    globals: &mut FxHashMap<String, GlobalSymbol>,
    defs: &[(String, String)],
) -> Result<PendingDefsyms, String> {
    // "Defined" means what it means to `resolve_sym` and to the undefined-
    // symbol check.  A symbol only a shared library defines is not defined
    // in this link (GNU ld: undefined symbol referenced in expression).
    let plan = defsym::DefsymPlan::new(defs, |n| match globals.get(n) {
        Some(g) if g.defined_in.is_some() && !g.is_dynamic => {
            if g.absolute {
                defsym::LinkSym::Absolute
            } else {
                defsym::LinkSym::Address
            }
        }
        _ => defsym::LinkSym::Undefined,
    })?;
    let shadow: FxHashMap<String, GlobalSymbol> = plan
        .shadowed_names()
        .into_iter()
        .filter_map(|n| globals.get(n).map(|g| (n.to_string(), g.clone())))
        .collect();
    let mut computed = Vec::new();
    for st in plan.finals() {
        // An alias's target is either not a `--defsym` name at all or the
        // statement's own (shadowed) target, so this reads an input
        // definition, never one inserted by this loop.
        let copied = plan.aliased_link_symbol(st).and_then(|t| {
            shadow
                .get(t)
                .or_else(|| globals.get(t))
                .filter(|g| g.defined_in.is_some_and(|d| d != usize::MAX) && !g.is_dynamic)
                .cloned()
        });
        if let Some(sym) = copied {
            globals.insert(st.name.clone(), sym);
            continue;
        }
        computed.push((st.name.clone(), st.index - 1));
        globals.insert(
            st.name.clone(),
            GlobalSymbol {
                // Overwritten by `evaluate_pending_defsyms` before any
                // consumer (relocation applier, dynsym, symtab) reads it.
                value: 0,
                size: 0,
                info: (STB_GLOBAL << 4) | STT_NOTYPE,
                // `Some(usize::MAX)`: a symbol this linker created;
                // `resolve_sym` returns `value` verbatim and the symtab
                // writer emits it as SHN_ABS.
                defined_in: Some(usize::MAX),
                from_lib: None,
                plt_idx: None,
                got_idx: None,
                section_idx: SHN_ABS,
                is_dynamic: false,
                copy_reloc: false,
                canonical_plt: false,
                visibility: 0,
                lib_sym_value: 0,
                lib_in_exec: false,
                version: None,
                // Whether the definition slides with a PIE/.so load base;
                // see `GlobalSymbol::absolute`.
                absolute: !st.is_address,
            },
        );
    }
    Ok(PendingDefsyms {
        plan,
        shadow,
        computed,
    })
}

/// A global symbol's final value from its pre-layout definition: an
/// input-section offset becomes an address, a COMMON symbol lands in
/// `.bss`, and everything else (absolute, linker-created, undefined) keeps
/// its value.  The one placement rule of both emitters, also applied to the
/// input definitions a `--defsym` overrides but reads.
pub(crate) fn placed_value(
    gsym: &GlobalSymbol,
    section_map: &FxHashMap<(usize, usize), (usize, u64)>,
    output_sections: &[OutputSection],
    bss_addr: Option<u64>,
) -> u64 {
    let Some(obj_idx) = gsym.defined_in else {
        return gsym.value;
    };
    if gsym.section_idx == SHN_COMMON || gsym.section_idx == 0xffff {
        gsym.value + bss_addr.unwrap_or(0)
    } else if gsym.section_idx != SHN_UNDEF && gsym.section_idx != SHN_ABS {
        match section_map.get(&(obj_idx, gsym.section_idx as usize)) {
            Some(&(oi, so)) => gsym.value + output_sections[oi].addr + so,
            None => gsym.value,
        }
    } else {
        gsym.value
    }
}

/// Compute the `--defsym` values against the finalised global symbol table.
///
/// Must run inside the emitter, after (1) section addresses are assigned to
/// every global symbol and (2) the linker-provided symbols (`_end`,
/// `_etext`, `__bss_start`, the `__start_`/`__stop_` symbols) are inserted,
/// so an expression sees exactly the values GNU ld's language-symbol
/// machinery would see.  Running it at the same point in `emit_executable`
/// and `emit_shared_library` keeps the two paths from drifting.
///
/// `place` is the emitter's rule giving an input symbol's final value from
/// its pre-layout definition; it finishes the shadowed definitions exactly
/// as the emitter finished every other symbol.  Arithmetic wraps as
/// unsigned 64-bit (a linker address expression means the two's-complement
/// difference, never a complaint).
pub(crate) fn evaluate_pending_defsyms(
    globals: &mut FxHashMap<String, GlobalSymbol>,
    pending: &PendingDefsyms,
    place: &dyn Fn(&GlobalSymbol) -> u64,
) -> Result<(), String> {
    if pending.computed.is_empty() {
        return Ok(());
    }
    let needed: Vec<usize> = pending.computed.iter().map(|&(_, pos)| pos).collect();
    let values = pending
        .plan
        .evaluate(&needed, |n| match pending.shadow.get(n) {
            Some(g) => g.defined_in.is_some().then(|| place(g)),
            None => globals
                .get(n)
                .and_then(|g| g.defined_in.is_some().then_some(g.value)),
        })?;
    for (name, pos) in &pending.computed {
        let entry = globals
            .get_mut(name)
            .ok_or_else(|| format!("--defsym {name}: symbol vanished before evaluation"))?;
        entry.value = values[*pos]
            .ok_or_else(|| format!("internal error: --defsym {name} was not evaluated"))?;
        entry.defined_in = Some(usize::MAX);
        entry.section_idx = SHN_ABS;
    }
    Ok(())
}

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
    let mut t_phase = crate::backend::linker_common::lap_timer::LapTimer::new();
    macro_rules! phase {
        ($name:expr_2021) => {
            if ld_time {
                eprintln!("[ldtime] {:<24} {:>7.1} ms", $name, t_phase.lap_ms());
            }
        };
    }
    let mut objects = linker_common::ObjectSet::new();
    let mut globals: FxHashMap<String, GlobalSymbol> = FxHashMap::default();
    let mut needed_sonames: Vec<String> = Vec::new();
    let lib_path_strings: Vec<String> = lib_paths.iter().map(|s| s.to_string()).collect();

    // Parse the user command line *before* loading any input.  A linker
    // script passed as an input resolves its own `-l` references against the
    // `-L` search path, so that path has to exist by the time the first input
    // is read; parsing later left early inputs (CRT objects, user objects and
    // scripts) searching only the driver's default directories.
    let parsed_args = linker_common::parse_linker_args(user_args);

    // `-L` directories first, in command-line order, then the defaults -- the
    // order GNU ld searches in.
    let mut all_lib_paths: Vec<String> = parsed_args.extra_lib_paths.clone();
    all_lib_paths.extend(lib_path_strings.iter().cloned());

    // Load CRT objects before user objects
    for path in crt_objects_before {
        if Path::new(path).exists() {
            load_file(
                path,
                &mut objects,
                &mut globals,
                &mut needed_sonames,
                &all_lib_paths,
                false,
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
            false,
        )?;
    }

    phase!("load-inputs");
    // Before the field moves below partially move `parsed_args`.
    let requested_dyn_flags = parsed_args.requested_dyn_flags();
    // `-z ibt=func` & co: accepted, ignored, and warned about — verbatim
    // GNU ld wording.
    for kw in &parsed_args.z_ignored_keywords {
        eprintln!("lccc-ld: warning: -z {kw} ignored");
    }
    // `--emit-relocs` is implemented only for `-T` script links
    // (`emit_script` retains the applied relocations there); on this path
    // the flag parses but the emitter cannot honour it, so say so rather
    // than silently producing an image without them.
    if parsed_args.emit_relocs {
        eprintln!(
            "lccc-ld: warning: --emit-relocs ignored: relocation retention is only implemented for -T script links"
        );
    }
    // Computed here, before the field moves below: the CET merge itself
    // runs much later (see below), but `property_link_flags` borrows the
    // parsed args, and an invalid `-z x86-64-*` level should fail the link
    // before any real work happens anyway.
    let cet_flags = parsed_args.property_link_flags()?;
    let libs_to_load = parsed_args.libs_to_load;
    // Whether each `-l` in `libs_to_load` was written under `-Bstatic` (or
    // `-static`): its search then accepts only `libNAME.a`.  args.rs records
    // every `-l` in both lists at once, so the two stay index-aligned.
    let lib_static_search: Vec<bool> = parsed_args
        .inputs
        .iter()
        .filter(|it| it.is_lib)
        .map(|it| it.static_search)
        .collect();
    debug_assert_eq!(lib_static_search.len(), libs_to_load.len());
    let extra_object_files = parsed_args.extra_object_files;
    let export_dynamic = parsed_args.export_dynamic;
    let rpath_entries = parsed_args.rpath_entries;
    let use_runpath = parsed_args.use_runpath;
    let sort_common = parsed_args.sort_common;
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

    // Positional --as-needed state, keyed by the input as it was written on the
    // command line. The executable path resolves libraries through flat name
    // lists (`libs_to_load`, `extra_object_files`) rather than the ordered
    // `inputs` vector, so the per-input flag is carried across in this map.
    //
    // Absent from the map means "not mentioned by the user", which covers
    // driver-supplied and default system libraries; GNU ld's default is
    // --no-as-needed, but those libraries are only consulted when something is
    // still undefined, so recording them unconditionally would add DT_NEEDED
    // entries for libraries that were never asked for. They stay as-needed.
    let as_needed_of: FxHashMap<String, bool> = parsed_args
        .inputs
        .iter()
        .map(|it| (it.name.clone(), it.as_needed))
        .collect();
    // Same for --whole-archive, which is positional for exactly the same
    // reason.  `link_shared` threads it through the ordered `inputs` vector;
    // this path resolves through flat name lists, so the flag travels in a map
    // keyed by the name as written.
    let whole_archive_of: FxHashMap<String, bool> = parsed_args
        .inputs
        .iter()
        .map(|it| (it.name.clone(), it.whole_archive))
        .collect();
    // `-lfoo` resolves to some /path/libfoo.so.N; map a resolved path back to
    // the stem the user wrote so the flag can be recovered.
    let as_needed_for = |path_or_name: &str| -> bool {
        if let Some(&v) = as_needed_of.get(path_or_name) {
            return v;
        }
        let base = std::path::Path::new(path_or_name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path_or_name);
        let stem = base.strip_prefix("lib").unwrap_or(base);
        let stem = stem.split(".so").next().unwrap_or(stem);
        if let Some(&v) = as_needed_of.get(stem) {
            return v;
        }
        true
    };
    // Recover the `--whole-archive` state for a resolved library path or the
    // `-l` stem the user wrote.  Defaults to false: GNU ld's default is
    // selective archive loading.
    let whole_archive_for = |path_or_name: &str| -> bool {
        if let Some(&v) = whole_archive_of.get(path_or_name) {
            return v;
        }
        let base = std::path::Path::new(path_or_name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path_or_name);
        let stem = base.strip_prefix("lib").unwrap_or(base);
        let stem = stem.split(".so").next().unwrap_or(stem);
        let stem = stem.strip_suffix(".a").unwrap_or(stem);
        whole_archive_of.get(stem).copied().unwrap_or(false)
    };

    // Force-undefined symbols (-u SYM): enter them into the global table as
    // undefined so the archive group-resolution loop below pulls in defining
    // members, exactly like GNU ld/mold.
    for sym_name in &undefined_symbols {
        if !globals.contains_key(sym_name) {
            let fake = linker_common::Elf64Symbol {
                name_idx: 0,
                name: SymStr::new(&sym_name),
                info: 1 << 4, // STB_GLOBAL, STT_NOTYPE
                other: 0,
                shndx: 0,
                value: 0,
                size: 0,
            };
            globals.insert(
                sym_name.clone(),
                <GlobalSymbol as linker_common::GlobalSymbolOps>::new_undefined(&fake),
            );
        }
    }

    // Load extra .o files immediately; archives (.a) and shared libraries (.so)
    // are deferred to the group resolution loop. Archives need iterative re-scanning
    // for circular dependencies, and shared libraries must be processed after
    // archive members are extracted so they can resolve symbols introduced by
    // those members (e.g., QEMU's libqemuutil.a members reference libglib-2.0.so).
    let mut deferred_libs: Vec<String> = Vec::new();
    // A --whole-archive member is force-loaded in full exactly ONCE. The set
    // must be declared before the deferred_libs loop below: the group
    // resolution loop re-scans every deferred library, and an archive that
    // was already force-loaded here must not be force-loaded again there —
    // the double load duplicated every member and failed the link with a
    // spurious `multiple definition` (reproduced with a two-member archive
    // and one referencing object: `--whole-archive reg.a --no-whole-archive m.o`).
    let mut whole_archive_loaded: FxHashSet<String> = FxHashSet::default();
    for path in &extra_object_files {
        if path.ends_with(".a") || path.ends_with(".so") || path.contains(".so.") {
            deferred_libs.push(path.clone());
            // A positional archive under --whole-archive is force-loaded now,
            // ahead of the group loop: the group loop only re-scans libraries
            // for members that resolve an undefined symbol, which is precisely
            // what --whole-archive opts out of.
            if whole_archive_for(path) && path.ends_with(".a") {
                load_file_as_needed(
                    path,
                    &mut objects,
                    &mut globals,
                    &mut needed_sonames,
                    &all_lib_paths,
                    true,
                    as_needed_for(path),
                )?;
                whole_archive_loaded.insert(path.clone());
            }
        } else {
            load_file(
                path,
                &mut objects,
                &mut globals,
                &mut needed_sonames,
                &all_lib_paths,
                false,
            )?;
        }
    }

    // Load CRT objects after
    for path in crt_objects_after {
        if Path::new(path).exists() {
            load_file(
                path,
                &mut objects,
                &mut globals,
                &mut needed_sonames,
                &all_lib_paths,
                false,
            )?;
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
            // Driver-supplied libraries follow the link mode; a user `-l`
            // follows the `-Bstatic`/`-Bdynamic` state at its position.
            let static_only = is_static
                || idx
                    .checked_sub(needed_lib_count)
                    .is_some_and(|k| lib_static_search.get(k).copied().unwrap_or(false));
            if let Some(lib_path) =
                linker_common::resolve_lib_positional(lib_name, &all_lib_paths, static_only)
            {
                if !lib_paths_resolved.contains(&lib_path) {
                    lib_paths_resolved.push(lib_path);
                }
            } else if idx >= needed_lib_count {
                // User-specified -l library not found: error (matching ld behavior)
                return Err(format!(
                    "cannot find -l{}: No such file or directory",
                    lib_name
                ));
            }
        }

        // Group loading: iterate until stable. Track both object count changes
        // (from archive member extraction) and dynamic symbol count changes
        // (from shared library resolution) since either can introduce work for
        // the other on the next iteration.
        let mut changed = true;
        // whole_archive_loaded was initialised before the deferred_libs loop
        // above and carries the paths already force-loaded there, so a
        // whole-archive archive is never loaded twice.
        while changed {
            changed = false;
            let prev_obj_count = objects.len();
            let prev_dyn_count = needed_sonames.len();
            for lib_path in &lib_paths_resolved {
                let wa = whole_archive_for(lib_path);
                if wa && whole_archive_loaded.contains(lib_path) {
                    continue;
                }
                load_file_as_needed(
                    lib_path,
                    &mut objects,
                    &mut globals,
                    &mut needed_sonames,
                    &all_lib_paths,
                    wa,
                    as_needed_for(lib_path),
                )?;
                if wa {
                    whole_archive_loaded.insert(lib_path.clone());
                }
            }
            if objects.len() != prev_obj_count || needed_sonames.len() != prev_dyn_count {
                changed = true;
            }
        }
    }

    // Input loading is complete: `objects[..real_inputs]` are the objects
    // read from files, and every object the linker synthesizes from here on
    // (build-id note, string-merge pool, property carrier) is appended after
    // them.  The property note merge counts only the real inputs — a
    // synthetic object carries no note and would veto every AND-class type.
    let real_inputs = objects.len();

    // --build-id: contribute a placeholder .note.gnu.build-id so the section
    // takes part in layout like any other input section.  The digest itself is
    // content-derived, so it can only be computed once the image is final;
    // `emit_exec` overwrites the placeholder in place (see
    // `patch_output_build_id`).  Debian's gcc passes --build-id on every link.
    if parsed_args.build_id {
        objects.push(linker_common::build_id::synthetic_note_object());
    }

    // Resolve remaining undefined symbols from default system libraries
    // (only when dynamically linking)
    if !is_static {
        let default_libs = [
            "libc.so.6",
            "libm.so.6",
            "libgcc_s.so.1",
            "ld-linux-x86-64.so.2",
        ];
        linker_common::resolve_dynamic_symbols_elf64(
            &mut globals,
            &mut needed_sonames,
            &all_lib_paths,
            &default_libs,
        )?;
    }

    // Apply --defsym definitions: alias, constant or expression.  Expressions
    // come back pending; the emitter evaluates them once addresses are final.
    let pending_defsyms = apply_defsyms(&mut globals, &defsym_defs)?;

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
            if globals
                .get(w)
                .map(|g| g.defined_in.is_none() && !g.is_dynamic)
                .unwrap_or(false)
            {
                globals.remove(w);
            }
        }
        // The wrapper may be undefined if the user forgot to provide it.
        // Leave that to the normal undefined-symbol check below.
    }

    // gABI visibility merge over definitions AND references.  After --wrap
    // (which renames references) and before the GC root set, whose
    // `--export-dynamic` roots must not include hidden symbols.
    linker_common::merge_object_visibility(&objects, &mut globals);

    // COMDAT (SHT_GROUP/GRP_COMDAT) deduplication.
    //
    // C++ emits one definition of every inline function, template
    // instantiation and vtable in EVERY translation unit that uses it, and
    // declares the copies interchangeable via a group signature. Symbol
    // resolution already picked a single winner, so the program behaved
    // correctly -- but the losing section BODIES were still laid out, as dead
    // bytes nothing could reach. This runs before ICF because it is cheaper
    // and needs no content comparison: the compiler already told us these are
    // the same entity.  It also runs before the --gc-sections sweep, which
    // must know the losers: a reference to a COMDAT symbol reaches the
    // surviving copy, not the referencing object's own.
    let comdat_plan = linker_common::comdat::plan_comdat(&objects);
    if std::env::var("LCCC_DEBUG_COMDAT").is_ok() {
        eprintln!(
            "[comdat] groups_discarded={} sections={} bytes_saved={}",
            comdat_plan.groups_discarded,
            comdat_plan.dead.len(),
            comdat_plan.bytes_saved
        );
    }

    // Garbage-collect unreferenced sections when --gc-sections is active.
    // This removes sections not reachable from entry points, which may also
    // eliminate undefined symbol references from dead code.
    let dead_sections: FxHashSet<(usize, usize)> = if gc_sections {
        let mut gc_roots: Vec<String> = undefined_symbols.clone();
        if let Some(ref e) = entry_symbol {
            gc_roots.push(e.clone());
        }
        // Every definition the executable exports is reachable from
        // outside the image; the emitter's own predicate names them.
        if let Some(exports) = super::emit_exec::ExecExports::new(
            export_dynamic,
            objects.dso_names(),
            parsed_args.version_script.as_deref(),
        ) {
            gc_roots.extend(
                globals
                    .iter()
                    .filter(|(name, g)| exports.exports(name, g))
                    .map(|(name, _)| name.clone()),
            );
        }
        linker_common::gc_collect_sections_elf64_full(
            &objects,
            &gc_roots,
            &FxHashSet::default(),
            &comdat_plan.dead,
        )
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
                // COMDAT losers are not emitted either; the sweep never saw
                // them, so they are not in `dead_sections`.
                if dead_sections.contains(&(obj_idx, sec_idx))
                    || comdat_plan.dead.contains(&(obj_idx, sec_idx))
                {
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
        // Remove undefined globals that are only referenced from dead sections
        globals.retain(|name, sym| {
            // Keep defined symbols, dynamic symbols, weak symbols, and those referenced from live code
            sym.defined_in.is_some()
                || sym.is_dynamic
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
    dead_sections.extend(comdat_plan.dead.iter().copied());
    if std::env::var("LCCC_NO_STRING_MERGE").is_err() {
        if let Some(plan) = linker_common::strmerge::plan_string_merge(
            &objects,
            &dead_sections,
            linker_common::map_section_name,
        ) {
            let applied = linker_common::strmerge::apply_string_merge(
                &mut objects,
                &mut globals,
                &mut dead_sections,
                &plan,
            );
            if std::env::var("LCCC_DEBUG_STRMERGE").is_ok() {
                eprintln!("[strmerge] pools={} applied={}", plan.pools.len(), applied);
                for p in &plan.pools {
                    eprintln!("[strmerge]   {} {} bytes", p.name, p.data.len());
                }
            }
        } else if std::env::var("LCCC_DEBUG_STRMERGE").is_ok() {
            eprintln!("[strmerge] no plan (no eligible sections)");
        }
    }

    // Prune the FDEs whose functions the sweep collected.  `--gc-sections`
    // works per input section, but a translation unit has ONE `.eh_frame`
    // holding an FDE per function, so with `-ffunction-sections` the FDE set
    // and the live-code set diverge inside a single section.  Must run before
    // ICF: a folded function still exists (it aliases its representative), so
    // its FDE is still needed, whereas a COMDAT/GC loser's is not.
    let dropped_fdes = linker_common::prune_dead_fdes(&mut objects, &dead_sections);
    if std::env::var("LCCC_DEBUG_GCEH").is_ok() {
        eprintln!(
            "[gceh] dropped_fdes={dropped_fdes} dead_sections={}",
            dead_sections.len()
        );
    }

    phase!("strmerge");
    // Identical Code Folding: retire duplicate function sections and alias
    // them onto their surviving representative. Folded sections are added to
    // `dead_sections` so the merge step never lays them out; the alias is
    // installed into `section_map` afterwards, so every symbol address and
    // relocation that referred to a folded section resolves to the survivor.
    // Sections already dead (gc/COMDAT/strmerge) are handed in so the plan
    // never folds among the dead.
    let icf_plan = match icf_mode.as_deref() {
        Some(mode) => icf::plan(&objects, mode == "safe", &dead_sections),
        None => icf::IcfPlan::default(),
    };
    for folded in icf_plan.redirect.keys() {
        dead_sections.insert(*folded);
    }
    // The loser's FDE duplicates the survivor's unwind rows: both describe
    // the same bytes at the same (aliased) address, so the survivor's FDE
    // already covers the range and the loser's only bloats `.eh_frame`.
    // Drop them now that the plan names the losers (the pre-ICF prune above
    // could not: a folded function is not dead — its symbols stay live as
    // aliases — so its FDE was correctly kept then). Unwinding through
    // folded twins stays covered: `icf_folded_code_still_unwinds` guards it.
    if !icf_plan.is_empty() {
        let folded: crate::common::fx_hash::FxHashSet<(usize, usize)> =
            icf_plan.redirect.keys().copied().collect();
        let dropped_folded_fdes = linker_common::prune_dead_fdes(&mut objects, &folded);
        if std::env::var("LCCC_DEBUG_GCEH").is_ok() {
            eprintln!("[gceh] dropped_folded_fdes={dropped_folded_fdes}");
        }
    }

    phase!("icf");
    // GNU property note merge (CET/ISA): fold every real input's
    // `.note.gnu.property` into a single note *in the input objects*
    // before the section merge, so the layout, the note section size and
    // the PT_NOTE/PT_GNU_PROPERTY segments all reflect the merged note.
    // The AND/OR-AND classes clear types vetoed by any real input; `-z
    // ibt`/`-z shstk`/`-z lam-u*`/`-z x86-64-*` can still create them.
    // (Semantics: linker_common::cet, verified against binutils 2.47
    // `_bfd_x86_elf_merge_gnu_properties`. `cet_flags` was computed up
    // front, next to the argument parsing.)
    if let Some(carrier) = linker_common::cet::merge_property_into_objects(
        &mut objects,
        real_inputs,
        &cet_flags,
        false, // 64-bit link: 16-byte property entries
    )? {
        objects.push(carrier);
    }

    // One copy of each distinct CIE and no zero gaps between .eh_frame
    // inputs (after the FDE prunes above, which may leave a CIE unused but
    // never change one).
    let eh_packing = linker_common::pack_eh_frame_sections(&mut objects, &dead_sections);

    // Merge sections (skip dead sections when gc-sections is active)
    let mut output_sections: Vec<OutputSection> = Vec::new();
    let mut section_map: FxHashMap<(usize, usize), (usize, u64)> = FxHashMap::default();
    linker_common::merge_sections_elf64_gc(
        &objects,
        &mut output_sections,
        &mut section_map,
        &dead_sections,
    );
    linker_common::apply_cie_redirects(&mut objects, &section_map, &eh_packing)?;

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
                "[icf] mode={} groups={} folded={} bytes_saved={} rejected_unsafe={} iters={} comparisons={} shattered={}",
                icf_mode.as_deref().unwrap_or("none"),
                icf_plan.result.candidate_groups,
                icf_plan.result.folded_sections,
                icf_plan.result.bytes_saved,
                icf_plan.result.rejected_unsafe,
                icf_plan.result.iterations,
                icf_plan.result.comparisons,
                icf_plan.result.shattered
            );
        }
    }

    phase!("merge-sections");
    // Allocate COMMON symbols
    linker_common::allocate_common_symbols_elf64(&mut globals, &mut output_sections, sort_common);

    phase!("common");
    // Create PLT/GOT
    let is_pie = parsed_args.is_pie;
    // Global TLS definitions this PIE exports (see
    // `emit_exec::preemptible_tls_names`): computed once so the planner
    // and the emitter cannot disagree about the dynamic TLS model.
    let preemptible_tls = super::emit_exec::preemptible_tls_names(
        &globals,
        is_pie,
        is_static,
        export_dynamic,
        objects.dso_names(),
        parsed_args.version_script.as_deref(),
    );
    let (plt_names, got_entries, abs_dyn_relocs, pie_relative, local_got) =
        create_plt_got(&objects, &mut globals, is_pie, &preemptible_tls);

    phase!("plt-got");
    // Collect IFUNC symbols for static linking
    let ifunc_symbols = collect_ifunc_symbols(&globals, is_static);
    // Local IFUNCs are invisible to the above (it walks `globals`), and without
    // an IPLT slot of their own their call sites bind to the resolver.
    let local_ifuncs = collect_local_ifuncs(&objects, &dead_sections);

    phase!("ifunc");
    // Emit executable.  The target ABI interpreter is the default, but a
    // caller may select a staging/sysroot loader with GNU ld's
    // --dynamic-linker spelling.  Materialise a NUL-terminated copy once so
    // layout, PT_INTERP, and .interp cannot disagree about its length.
    let interpreter = if let Some(path) = parsed_args.dynamic_linker.as_deref() {
        if path.as_bytes().contains(&0) || path.is_empty() {
            return Err("invalid --dynamic-linker path".into());
        }
        let mut bytes = path.as_bytes().to_vec();
        bytes.push(0);
        bytes
    } else {
        INTERP.to_vec()
    };

    // Emit executable
    let r = emit_executable(
        &objects,
        &interpreter,
        &mut globals,
        &mut output_sections,
        &section_map,
        &dead_sections,
        &icf_plan.redirect,
        &plt_names,
        &got_entries,
        &local_got,
        &abs_dyn_relocs,
        &pie_relative,
        &needed_sonames,
        output_path,
        export_dynamic,
        objects.dso_names(),
        &rpath_entries,
        use_runpath,
        is_static,
        &ifunc_symbols,
        entry_symbol.as_deref(),
        parsed_args.z_now,
        parsed_args.z_relro,
        parsed_args.z_text,
        requested_dyn_flags,
        parsed_args.map_path.as_deref(),
        parsed_args.version_script.as_deref(),
        is_pie,
        parsed_args.strip_all,
        &local_ifuncs,
        parsed_args.hash_style,
        &pending_defsyms,
        &preemptible_tls,
    );
    phase!("emit-exec");
    if ld_time {
        eprintln!(
            "[ldtime] {:<24} {:>7.1} ms",
            "TOTAL",
            t_all.elapsed().as_secs_f64() * 1e3
        );
    }
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
    let mut objects = linker_common::ObjectSet::new();
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
    // `-z ibt=func` & co: accepted, ignored, and warned about — verbatim
    // GNU ld wording.
    for kw in &parsed.z_ignored_keywords {
        eprintln!("lccc-ld: warning: -z {kw} ignored");
    }
    // As in `link_builtin` above: `--emit-relocs` only takes effect for
    // `-T` script links; warn rather than silently drop it here.
    if parsed.emit_relocs {
        eprintln!(
            "lccc-ld: warning: --emit-relocs ignored: relocation retention is only implemented for -T script links"
        );
    }
    // CET/ISA flags up front, as in `link_builtin`: an invalid `-z
    // x86-64-*` level fails the link before any real work happens.
    let cet_flags = parsed.property_link_flags()?;

    let extra_lib_paths: Vec<String> = parsed.extra_lib_paths.clone();
    let soname: Option<String> = parsed.soname.clone();
    let rpath_entries: Vec<String> = parsed.rpath_entries.clone();
    let use_runpath = parsed.use_runpath;
    let sort_common = parsed.sort_common;
    let version_script: Option<String> = parsed.version_script.clone();
    let no_undefined = parsed.no_undefined;
    let symbolic = parsed.symbolic;
    let requested_dyn_flags = parsed.requested_dyn_flags();
    let exclude_libs: Vec<String> = parsed.exclude_libs.clone();

    // (path_or_lib, is_lib, whole_archive, as_needed, static_search), in
    // command-line order.  Archive, as-needed and -Bstatic state are all
    // POSITIONAL, so they have to travel with the input rather than being
    // read as global flags.
    let ordered_items: Vec<(String, bool, bool, bool, bool)> = parsed
        .inputs
        .iter()
        .map(|it| {
            (
                it.name.clone(),
                it.is_lib,
                it.whole_archive,
                it.as_needed,
                it.static_search,
            )
        })
        .collect();

    // Load user object files (from the compiler driver, before user_args)
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

    let mut all_lib_paths: Vec<String> = extra_lib_paths;
    all_lib_paths.extend(lib_path_strings.iter().cloned());

    // Load ordered items (bare files and -l libraries) preserving --whole-archive state
    let mut libs_to_load_later: Vec<(String, bool, bool, bool)> = Vec::new();
    for (item, is_lib, wa, an, static_search) in &ordered_items {
        if *is_lib {
            libs_to_load_later.push((item.clone(), *wa, *an, *static_search));
        } else {
            load_file_as_needed(
                item,
                &mut objects,
                &mut globals,
                &mut needed_sonames,
                &all_lib_paths,
                *wa,
                *an,
            )?;
        }
    }

    // Resolve -l libraries
    if !libs_to_load_later.is_empty() {
        let mut lib_paths_resolved: Vec<(String, bool, bool)> = Vec::new();
        for (lib_name, wa, an, static_search) in &libs_to_load_later {
            if let Some(lib_path) =
                linker_common::resolve_lib_positional(lib_name, &all_lib_paths, *static_search)
            {
                if !lib_paths_resolved.iter().any(|(p, _, _)| p == &lib_path) {
                    lib_paths_resolved.push((lib_path, *wa, *an));
                }
            } else {
                return Err(format!(
                    "cannot find -l{}: No such file or directory",
                    lib_name
                ));
            }
        }
        // Track which whole-archive libraries have been fully loaded to avoid
        // re-adding all members on subsequent iterations of the group loop.
        let mut whole_archive_loaded: FxHashSet<String> = FxHashSet::default();
        let mut changed = true;
        while changed {
            changed = false;
            let prev_count = objects.len();
            for (lib_path, wa, an) in &lib_paths_resolved {
                if *wa && whole_archive_loaded.contains(lib_path) {
                    continue; // Already loaded all members
                }
                load_file_as_needed(
                    lib_path,
                    &mut objects,
                    &mut globals,
                    &mut needed_sonames,
                    &all_lib_paths,
                    *wa,
                    *an,
                )?;
                if *wa {
                    whole_archive_loaded.insert(lib_path.clone());
                }
            }
            if objects.len() != prev_count {
                changed = true;
            }
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

    // Resolve remaining undefined symbols against system libraries (libc, libm,
    // libgcc_s) and add DT_NEEDED entries for any that provide matched symbols.
    let default_libs = [
        "libc.so.6",
        "libm.so.6",
        "libgcc_s.so.1",
        "ld-linux-x86-64.so.2",
    ];
    linker_common::resolve_dynamic_symbols_elf64(
        &mut globals,
        &mut needed_sonames,
        &all_lib_paths,
        &default_libs,
    )?;

    // -z defs / --no-undefined: shared libraries normally tolerate
    // undefined symbols (resolved at load time), but with this flag every
    // reference must be satisfied at LINK time - the standard CMake/Qt
    // hardening switch to catch missing DT_NEEDED deps early.
    if no_undefined {
        linker_common::check_undefined_symbols_elf64_verbose(&globals, 20, &objects)?;
    }

    // --defsym SYMBOL=EXPRESSION, same semantics as the executable path.
    // Expressions come back pending; the emitter evaluates them once addresses
    // are final.
    let pending_defsyms = apply_defsyms(&mut globals, &parsed.defsym_defs)?;

    // gABI visibility merge (definitions and references); the export filter
    // and every preemptibility decision in `emit_shared_library` read it.
    linker_common::merge_object_visibility(&objects, &mut globals);

    // GNU property note merge, as on the executable path (input level,
    // before the section merge — see the comment in `link_builtin`;
    // `cet_flags` was computed up front, next to the argument parsing).
    // Nothing is synthesized before this point on the shared-library path,
    // so every object is a real input.
    let real_inputs = objects.len();
    if let Some(carrier) = linker_common::cet::merge_property_into_objects(
        &mut objects,
        real_inputs,
        &cet_flags,
        false, // 64-bit link: 16-byte property entries
    )? {
        objects.push(carrier);
    }

    // Identical Code Folding: `--icf=safe|all` must mean the same thing
    // for `-shared` as for executables (lld and mold fold shared images;
    // a flag that parses and then silently does nothing is fail-open).
    // Folded sections enter `dead_sections`, are skipped by the merge, and
    // are aliased onto their representative below — the same three-step
    // contract as `link_builtin`. The emit-side relocation loop in
    // `emit_shared` skips `dead_sections` so a folded twin's sequence-shaped
    // TLS GD/LD rewrite cannot run twice over bytes the first application
    // already rewrote (the `__cxa_get_globals` class).
    let icf_mode: Option<String> = parsed
        .icf
        .clone()
        .or_else(|| icf::icf_mode_from_env().map(str::to_string));
    let mut dead_sections: FxHashSet<(usize, usize)> = FxHashSet::default();
    let icf_plan = match icf_mode.as_deref() {
        Some(mode) => icf::plan(&objects, mode == "safe", &dead_sections),
        None => icf::IcfPlan::default(),
    };
    for folded in icf_plan.redirect.keys() {
        dead_sections.insert(*folded);
    }
    // Folded FDEs duplicate the survivor's unwind rows at the aliased
    // address; drop them before packing, as the executable path does.
    if !icf_plan.is_empty() {
        let folded: FxHashSet<(usize, usize)> = icf_plan.redirect.keys().copied().collect();
        let _dropped = linker_common::prune_dead_fdes(&mut objects, &folded);
    }

    // Merge sections (no gc-sections for shared libraries), packing
    // .eh_frame as on the executable path. The gc-aware merge with an
    // empty dead set is the plain merge.
    let eh_packing = linker_common::pack_eh_frame_sections(&mut objects, &dead_sections);
    let mut output_sections: Vec<OutputSection> = Vec::new();
    let mut section_map: FxHashMap<(usize, usize), (usize, u64)> = FxHashMap::default();
    linker_common::merge_sections_elf64_gc(
        &objects,
        &mut output_sections,
        &mut section_map,
        &dead_sections,
    );
    linker_common::apply_cie_redirects(&mut objects, &section_map, &eh_packing)?;
    // Point folded sections at the representative's placement: same order
    // as `link_builtin` (after the merge, before any address assignment).
    if !icf_plan.is_empty() {
        for (&folded, &rep) in icf_plan.redirect.iter() {
            if let Some(&placement) = section_map.get(&rep) {
                section_map.insert(folded, placement);
            }
        }
    }

    // Allocate COMMON symbols
    linker_common::allocate_common_symbols_elf64(&mut globals, &mut output_sections, sort_common);

    // Emit shared library
    emit_shared_library(
        &objects,
        &mut globals,
        &mut output_sections,
        parsed.hash_style,
        &section_map,
        &dead_sections,
        &needed_sonames,
        output_path,
        soname,
        &rpath_entries,
        use_runpath,
        version_script.as_deref(),
        symbolic,
        requested_dyn_flags,
        parsed.z_relro,
        parsed.z_now,
        parsed.z_text,
        &exclude_libs,
        parsed.map_path.as_deref(),
        &pending_defsyms,
    )
}
