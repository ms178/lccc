//! Symbol resolution for the i686 linker.
//!
//! Phases 6-9: global symbol resolution, COMMON symbol allocation,
//! PLT/GOT marking, undefined symbol checking, PLT/GOT list building,
//! and IFUNC collection.

use crate::common::fx_hash::{FxHashMap, FxHashSet};

use super::types::*;
use crate::backend::linker_common;

/// Turn the tentative definitions a shared library's data definition takes
/// over (`input::bind_shared_commons`, GNU ld's binding) into references:
/// each is then imported like any undefined data (`R_386_GLOB_DAT` or a copy
/// relocation in an executable, an undefined dynamic symbol in a shared
/// object).  Rewriting them before resolution, as bfd's `elf_merge_symbol`
/// does, lets every later phase see a plain reference.
pub(super) fn import_shared_commons(inputs: &mut [InputObject], imports: &FxHashSet<String>) {
    if imports.is_empty() {
        return;
    }
    for sym in inputs.iter_mut().flat_map(|obj| &mut obj.symbols) {
        if sym.section_index == SHN_COMMON && imports.contains(&sym.name) {
            sym.section_index = SHN_UNDEF;
            sym.value = 0;
        }
    }
}

// Temporary output_section tags for the definition pass only. SHN_ABS
// uses usize::MAX; neither a pending COMMON nor an unmapped ordinary
// definition is an absolute definition for collision checking.
const PENDING_COMMON: usize = usize::MAX - 1;
const UNMAPPED_INPUT: usize = usize::MAX - 2;

fn has_global_name(sym: &InputSymbol) -> bool {
    sym.binding != STB_LOCAL
        && !sym.name.is_empty()
        && !matches!(sym.sym_type, STT_FILE | STT_SECTION)
}

fn in_discarded_group(discarded: &FxHashSet<(usize, usize)>, obj_idx: usize, shndx: u16) -> bool {
    !matches!(shndx, SHN_UNDEF | SHN_ABS | SHN_COMMON)
        && discarded.contains(&(obj_idx, shndx as usize))
}

fn definition_rank(binding: u8, common: bool) -> u8 {
    if common {
        1
    } else if binding == STB_WEAK {
        0
    } else {
        2
    }
}

#[cold]
fn duplicate_definition(
    inputs: &[InputObject],
    obj_idx: usize,
    sym: &InputSymbol,
    discarded: &FxHashSet<(usize, usize)>,
) -> String {
    // Recover provenance only on the error path; don't keep a second
    // per-symbol table or enlarge every LinkerSymbol in a successful link.
    let first = inputs[..=obj_idx]
        .iter()
        .enumerate()
        .find(|&(i, obj)| {
            obj.symbols.iter().any(|s| {
                s.name == sym.name
                    && has_global_name(s)
                    && s.binding != STB_WEAK
                    && !matches!(s.section_index, SHN_UNDEF | SHN_COMMON)
                    && !in_discarded_group(discarded, i, s.section_index)
            })
        })
        .map_or(&inputs[obj_idx].filename, |(_, obj)| &obj.filename);
    format!(
        "multiple definition of '{}' in {}; first defined in {}",
        sym.name, inputs[obj_idx].filename, first
    )
}

pub(super) fn resolve_symbols(
    inputs: &[InputObject],
    section_map: &SectionMap,
    discarded: &FxHashSet<(usize, usize)>,
    dynlib_syms: &super::input::DynlibSyms,
    allow_multiple_definition: bool,
    output_sections: &mut Vec<OutputSection>,
    section_name_to_idx: &mut FxHashMap<String, usize>,
) -> Result<FxHashMap<String, LinkerSymbol>, String> {
    let mut global_symbols: FxHashMap<String, LinkerSymbol> = FxHashMap::default();
    let mut commons = Vec::new();
    let mut have_unmapped = false;

    // Local names never satisfy another object's references. Relocations
    // to local and section symbols resolve with (object, section) instead.
    for (obj_idx, obj) in inputs.iter().enumerate() {
        for sym in &obj.symbols {
            if !has_global_name(sym) {
                continue;
            }
            let (out_sec_idx, sec_offset) = match sym.section_index {
                SHN_UNDEF => continue,
                SHN_ABS => (usize::MAX, 0),
                SHN_COMMON => (PENDING_COMMON, 0),
                shndx => match section_map.get(&(obj_idx, shndx as usize)) {
                    Some(&location) => location,
                    None if discarded.contains(&(obj_idx, shndx as usize)) => continue,
                    None => (UNMAPPED_INPUT, 0),
                },
            };
            let common = out_sec_idx == PENDING_COMMON;
            let mut previous = global_symbols.get_mut(sym.name.as_str());
            if let Some(old) = previous.as_deref_mut() {
                let old_common = old.output_section == PENDING_COMMON;
                if common && old_common {
                    // A different object can impose each maximum. Preserve
                    // first-seen layout order independently of both maxima.
                    if old.binding == STB_WEAK {
                        old.binding = sym.binding;
                    }
                    old.size = old.size.max(sym.size);
                    old.section_offset = old.section_offset.max(common_alignment(sym, obj)?);
                    continue;
                }
                let rank = definition_rank(sym.binding, common);
                let old_rank = definition_rank(old.binding, old_common);
                if rank <= old_rank {
                    // Like GNU ld, allow identical absolute constants.
                    // Equal values in two real sections do not qualify.
                    if rank == 2
                        && old_rank == 2
                        && !allow_multiple_definition
                        && !(sym.section_index == SHN_ABS
                            && old.output_section == usize::MAX
                            && old.address == sym.value)
                    {
                        return Err(duplicate_definition(inputs, obj_idx, sym, discarded));
                    }
                    continue;
                }
            }
            let offset = if common {
                commons.push(sym.name.as_str());
                common_alignment(sym, obj)?
            } else {
                sec_offset.checked_add(sym.value).ok_or_else(|| {
                    format!(
                        "{}: section offset for '{}' exceeds 32-bit address space",
                        obj.filename, sym.name
                    )
                })?
            };
            have_unmapped |= out_sec_idx == UNMAPPED_INPUT;
            let winner = LinkerSymbol {
                address: if sym.section_index == SHN_ABS {
                    sym.value
                } else {
                    0
                },
                size: sym.size,
                sym_type: sym.sym_type,
                binding: sym.binding,
                visibility: sym.visibility,
                is_defined: true,
                needs_plt: false,
                needs_got: false,
                output_section: out_sec_idx,
                section_offset: offset,
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
            };
            if let Some(old) = previous {
                *old = winner;
            } else {
                global_symbols.insert(sym.name.clone(), winner);
            }
        }
    }

    allocate_common_symbols(
        &commons,
        output_sections,
        section_name_to_idx,
        &mut global_symbols,
    )?;
    if have_unmapped {
        // Non-COMDAT metadata omitted by section merging (for example
        // stripped debug sections) has the previous no-output-section
        // representation. Don't confuse it with a discarded COMDAT copy
        // or allow it the equal-absolute collision exemption above.
        for sym in global_symbols.values_mut() {
            if sym.output_section == UNMAPPED_INPUT {
                sym.output_section = usize::MAX;
            }
        }
    }

    // The overwhelmingly common no-discard case needs neither an object
    // index nor a relocation scan. With discards, visit only affected TUs.
    let mut has_discarded = Vec::new();
    if !discarded.is_empty() {
        has_discarded.resize(inputs.len(), false);
        for &(i, _) in discarded {
            has_discarded[i] = true;
        }
    }
    for (i, obj) in inputs.iter().enumerate() {
        let discarded_only = if has_discarded.get(i) == Some(&true) {
            resolve_comdat_references(
                i,
                obj,
                section_map,
                discarded,
                &mut global_symbols,
                dynlib_syms,
            )?
        } else {
            FxHashSet::default()
        };
        for (k, sym) in obj.symbols.iter().enumerate() {
            if sym.section_index == SHN_UNDEF
                && has_global_name(sym)
                && !discarded_only.contains(&k)
            {
                register_reference(sym, &mut global_symbols, dynlib_syms);
            }
        }
    }

    // References already contributed their visibility when registered.
    // Now merge all live definitions, including losing weak definitions.
    for (i, obj) in inputs.iter().enumerate() {
        for sym in &obj.symbols {
            if sym.section_index != SHN_UNDEF
                && sym.visibility != STV_DEFAULT
                && has_global_name(sym)
                && !in_discarded_group(discarded, i, sym.section_index)
                && let Some(gs) = global_symbols.get_mut(sym.name.as_str())
            {
                gs.visibility =
                    linker_common::most_constraining_visibility(gs.visibility, sym.visibility);
            }
        }
    }
    Ok(global_symbols)
}

/// In an object with a losing group, only the live relocations may create
/// new references to its discarded definitions. Conversely, an undefined
/// input symbol used *only* inside the losing group doesn't demand a
/// definition. Preserve symbols with no relocations (e.g. explicit undef
/// requests) and symbols also referenced by an allocated, kept section.
fn resolve_comdat_references(
    i: usize,
    obj: &InputObject,
    section_map: &SectionMap,
    discarded: &FxHashSet<(usize, usize)>,
    globals: &mut FxHashMap<String, LinkerSymbol>,
    dynlib_syms: &super::input::DynlibSyms,
) -> Result<FxHashSet<usize>, String> {
    let mut discarded_only = FxHashSet::default();
    for sec in &obj.sections {
        if sec.relocations.is_empty()
            || sec.flags & SHF_ALLOC == 0
            || !discarded.contains(&(i, sec.input_index))
        {
            continue;
        }
        for &(_, kind, k, _) in &sec.relocations {
            if kind != R_386_NONE
                && obj
                    .symbols
                    .get(k as usize)
                    .is_some_and(|s| s.section_index == SHN_UNDEF)
            {
                discarded_only.insert(k as usize);
            }
        }
    }
    for sec in &obj.sections {
        if sec.relocations.is_empty()
            || sec.flags & SHF_ALLOC == 0
            || !section_map.contains_key(&(i, sec.input_index))
        {
            continue;
        }
        for &(_, kind, k, _) in &sec.relocations {
            if kind == R_386_NONE {
                continue;
            }
            let k = k as usize;
            discarded_only.remove(&k);
            let Some(sym) = obj.symbols.get(k) else {
                continue;
            };
            if in_discarded_group(discarded, i, sym.section_index) {
                if !has_global_name(sym) {
                    let target = if sym.name.is_empty() {
                        obj.sections
                            .iter()
                            .find(|s| s.input_index == sym.section_index as usize)
                            .map_or("<unnamed>", |s| s.name.as_str())
                    } else {
                        &sym.name
                    };
                    return Err(format!(
                        "{}: {} references '{target}' in a discarded COMDAT section",
                        obj.filename, sec.name
                    ));
                }
                register_reference(sym, globals, dynlib_syms);
            }
        }
    }
    Ok(discarded_only)
}

fn register_reference(
    sym: &InputSymbol,
    globals: &mut FxHashMap<String, LinkerSymbol>,
    dynlib_syms: &super::input::DynlibSyms,
) {
    if let Some(existing) = globals.get_mut(sym.name.as_str()) {
        // Finding a weak reference first doesn't make a later strong
        // reference optional. Never change the binding of a definition.
        if !existing.is_defined
            && !existing.is_dynamic
            && existing.binding == STB_WEAK
            && sym.binding != STB_WEAK
        {
            existing.binding = sym.binding;
            existing.sym_type = sym.sym_type;
        }
        existing.visibility =
            linker_common::most_constraining_visibility(existing.visibility, sym.visibility);
        return;
    }
    let mut reference = match dynlib_syms.get(sym.name.as_str()) {
        Some(d) => LinkerSymbol::dynamic_import(d),
        None => LinkerSymbol {
            address: 0,
            size: 0,
            sym_type: sym.sym_type,
            binding: sym.binding,
            visibility: STV_DEFAULT,
            is_defined: false,
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
    };
    reference.visibility = sym.visibility;
    globals.insert(sym.name.clone(), reference);
}

fn common_alignment(sym: &InputSymbol, obj: &InputObject) -> Result<u32, String> {
    let alignment = sym.value.max(1);
    if !alignment.is_power_of_two() {
        return Err(format!(
            "{}: invalid COMMON alignment {} for '{}'",
            obj.filename, sym.value, sym.name
        ));
    }
    Ok(alignment)
}

/// Assign each winning COMMON one .bss slot in input order. Compute the
/// entire extent with checked ELF32 arithmetic before growing the buffer.
fn allocate_common_symbols(
    commons: &[&str],
    output_sections: &mut Vec<OutputSection>,
    section_name_to_idx: &mut FxHashMap<String, usize>,
    globals: &mut FxHashMap<String, LinkerSymbol>,
) -> Result<(), String> {
    if commons.is_empty() {
        return Ok(());
    }
    let mut bss_idx = section_name_to_idx.get(".bss").copied();
    let mut end = bss_idx
        .map_or(Ok(0), |idx| u32::try_from(output_sections[idx].data.len()))
        .map_err(|_| ".bss exceeds 32-bit address space".to_string())?;
    let mut align = bss_idx.map_or(4, |idx| output_sections[idx].align);
    for &name in commons {
        let sym = globals
            .get_mut(name)
            .ok_or_else(|| format!("COMMON symbol '{name}' missing from resolved table"))?;
        if sym.output_section != PENDING_COMMON {
            continue; // a real strong definition took precedence
        }
        let a = sym.section_offset;
        let overflow = || format!("COMMON symbol '{name}' makes .bss exceed 32-bit address space");
        // 'end + a - 1' can overflow even when already aligned. Add only
        // the padding actually needed, then check the size separately.
        let offset = end
            .checked_add(end.wrapping_neg() & (a - 1))
            .ok_or_else(overflow)?;
        end = offset.checked_add(sym.size).ok_or_else(overflow)?;
        let idx = *bss_idx.get_or_insert_with(|| {
            let idx = output_sections.len();
            output_sections.push(OutputSection {
                name: ".bss".to_string(),
                sh_type: SHT_NOBITS,
                flags: SHF_ALLOC | SHF_WRITE,
                data: Vec::new(),
                align: 4,
                addr: 0,
                file_offset: 0,
            });
            section_name_to_idx.insert(".bss".to_string(), idx);
            idx
        });
        align = align.max(a);
        sym.output_section = idx;
        sym.section_offset = offset;
    }
    if let Some(idx) = bss_idx {
        let bss = &mut output_sections[idx];
        let len = end as usize;
        bss.data
            .try_reserve(len - bss.data.len())
            .map_err(|e| format!("cannot allocate COMMON storage: {e}"))?;
        bss.data.resize(len, 0);
        bss.align = align;
    }
    Ok(())
}

// ══════════════════════════════════════════════════════════════════════════════
// Phase 7: PLT/GOT marking + undefined check
// ══════════════════════════════════════════════════════════════════════════════

/// Marks the PLT/GOT/COPY needs of named (global) symbols and returns the
/// GOT slots STB_LOCAL symbols need, one per (object, symbol) in input
/// order.  A local never touches a same-named global's entry: a `static
/// atoi` call is not a reason to give libc's `atoi` a PLT slot, and a
/// `static` object named like a DSO variable must not copy-relocate it.
pub(super) fn mark_plt_got_needs(
    inputs: &[InputObject],
    discarded: &FxHashSet<(usize, usize)>,
    global_symbols: &mut FxHashMap<String, LinkerSymbol>,
) -> Vec<(usize, usize)> {
    let mut local_got: Vec<(usize, usize)> = Vec::new();
    let mut local_seen: FxHashSet<(usize, usize)> = FxHashSet::default();
    for (obj_idx, obj) in inputs.iter().enumerate() {
        for sec in &obj.sections {
            if sec.relocations.is_empty() || discarded.contains(&(obj_idx, sec.input_index)) {
                continue;
            }
            // Executables transition every GD/LDM sequence (see tls.rs), so
            // their `___tls_get_addr` calls disappear and need no PLT/GOT.
            let dead_calls: FxHashSet<u32> =
                super::tls::transitioned_call_fields(&sec.data, &sec.relocations).collect();
            for &(rel_offset, rel_type, sym_idx, _) in &sec.relocations {
                let sym = if (sym_idx as usize) < obj.symbols.len() {
                    &obj.symbols[sym_idx as usize]
                } else {
                    continue;
                };

                if dead_calls.contains(&rel_offset) {
                    continue;
                }
                if sym.binding == STB_LOCAL || sym.sym_type == STT_SECTION {
                    if matches!(rel_type, R_386_GOT32 | R_386_GOT32X)
                        && sec.flags & SHF_ALLOC != 0
                        && !(rel_type == R_386_GOT32X
                            && super::reloc::got32x_conversion(&sec.data, rel_offset as usize)
                                .is_some()
                            && super::reloc::got_target_relaxable(sym, None))
                        && local_seen.insert((obj_idx, sym_idx as usize))
                    {
                        local_got.push((obj_idx, sym_idx as usize));
                    }
                    continue;
                }
                if sym.name.is_empty() {
                    continue;
                }

                match rel_type {
                    // A call reaches a shared-library function through its
                    // PLT entry, and so does a non-PIC address-of (`R_386_32`,
                    // a non-branch `R_386_PC32`), which makes the entry the
                    // function's canonical address.
                    R_386_PLT32 => {
                        if let Some(gs) = global_symbols.get_mut(sym.name.as_str()) {
                            if gs.is_dynamic {
                                gs.needs_plt = true;
                                // Calls never need a copy: code isn't copied.
                                // (Data reached through a call is UB; the
                                // crash merely moves, so it keeps its copy.)
                                if is_plt_code_type(gs.sym_type, gs.lib_in_exec) {
                                    gs.needs_copy = false;
                                }
                            }
                        }
                    }
                    // A DSO data object the executable addresses directly
                    // -- an absolute or PC-relative field of any width, or
                    // a GOT-relative offset -- must live at a link-time
                    // address: it is copy-relocated into the executable.
                    // Only such a reference: an object reached through its
                    // GOT slot alone (PIC code linked into the executable)
                    // stays in its library, as with GNU ld -- a copy would
                    // cost .bss and a relocation for nothing, and is
                    // refused by ld.so for a protected-visibility object.
                    // A DSO TLS variable is never copied: it lives in its
                    // module's TLS block (TLS_TPOFF slots).  References
                    // from non-allocated sections (DWARF) are never loaded
                    // and create nothing -- GNU ld ignores them too.
                    R_386_PC32 | R_386_32 | R_386_16 | R_386_PC16 | R_386_8 | R_386_PC8
                    | R_386_GOTOFF
                        if sec.flags & SHF_ALLOC != 0 =>
                    {
                        if let Some(gs) = global_symbols.get_mut(sym.name.as_str()) {
                            if !gs.is_dynamic {
                                continue;
                            }
                            // Code (functions, plus untyped exports in
                            // executable segments) takes the PLT on a 32-bit
                            // reference exactly like a function: a `call
                            // notypefn` (PC32) or address-taking of it must
                            // reach real code, never a copy of code bytes
                            // (BSS is NX). Narrower code references take
                            // nothing, as functions always have.
                            let is_code = is_plt_code_type(gs.sym_type, gs.lib_in_exec);
                            if is_code && matches!(rel_type, R_386_PC32 | R_386_32) {
                                gs.needs_plt = true;
                                gs.needs_copy = false;
                                gs.canonical_plt |=
                                    rel_type == R_386_32 || !is_branch_rel32(sec, rel_offset);
                            } else if !is_code && gs.sym_type != STT_TLS {
                                gs.needs_copy = true;
                            }
                        }
                    }
                    // A convertible GOT32X access to a symbol defined here
                    // is rewritten to an immediate or a direct branch
                    // (reloc.rs, `Got32xConversion`) and needs no slot;
                    // every other GOT reference loads from one.
                    R_386_GOT32 | R_386_GOT32X => {
                        let relaxed = rel_type == R_386_GOT32X
                            && super::reloc::got32x_conversion(&sec.data, rel_offset as usize)
                                .is_some()
                            && super::reloc::got_target_relaxable(
                                sym,
                                global_symbols.get(sym.name.as_str()),
                            );
                        if let Some(gs) = global_symbols.get_mut(sym.name.as_str()) {
                            gs.needs_got |= !relaxed;
                        }
                    }
                    // Main-image TLS transitions to local exec; a DSO TLS
                    // variable needs one TLS_TPOFF slot for all of IE,
                    // GOTIE, GD→IE and TLSDESC→IE.
                    R_386_TLS_GOTIE | R_386_TLS_IE | R_386_TLS_GD | R_386_TLS_GOTDESC => {
                        if let Some(gs) = global_symbols.get_mut(sym.name.as_str()) {
                            if gs.is_dynamic {
                                gs.needs_got = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    local_got
}

/// Whether the relocated field at `offset` of `sec` is the rel32 of a direct
/// branch (`call`/`jmp`/`jcc rel32`) -- a use that does not take the
/// target's address.  Only code holds branches: in data, `.long f - .` does
/// take the address.  In code the byte before any other PC-relative disp32
/// is part of a ModRM/SIB/opcode sequence that cannot be `e8`/`e9` (i386 has
/// no RIP-relative addressing, so a non-branch PC32 in code is rare anyway).
fn is_branch_rel32(sec: &InputSection, offset: u32) -> bool {
    if sec.flags & SHF_EXECINSTR == 0 {
        return false;
    }
    let off = offset as usize;
    if off == 0 || off > sec.data.len() {
        return false;
    }
    let op = sec.data[off - 1];
    op == 0xe8
        || op == 0xe9
        || (off >= 2 && sec.data[off - 2] == 0x0f && (0x80..=0x8f).contains(&op))
}

pub(super) fn check_undefined_symbols(
    global_symbols: &FxHashMap<String, LinkerSymbol>,
) -> Result<(), String> {
    let truly_undefined: Vec<&String> = global_symbols
        .iter()
        .filter(|(n, s)| {
            !s.is_defined
                && !s.is_dynamic
                && s.binding != STB_WEAK
                && !linker_common::is_linker_defined_symbol(n)
        })
        .map(|(n, _)| n)
        .collect();

    if !truly_undefined.is_empty() {
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

// ══════════════════════════════════════════════════════════════════════════════
// Phase 8: PLT/GOT list building
// ══════════════════════════════════════════════════════════════════════════════

pub(super) fn build_plt_got_lists(
    global_symbols: &mut FxHashMap<String, LinkerSymbol>,
) -> (Vec<String>, Vec<String>, Vec<String>, usize, usize) {
    let mut plt_symbols: Vec<String> = Vec::new();
    let mut got_dyn_symbols: Vec<String> = Vec::new();
    let mut got_local_symbols: Vec<String> = Vec::new();

    // A shared-library function can need both: a PLT entry for its calls
    // (with a lazily-bound `.got.plt` slot) and an ordinary GOT slot for a
    // GOT-relative address-of, which must hold the resolved address from the
    // start -- a lazy `.got.plt` slot holds `PLT+6` until the first call.
    for (name, sym) in global_symbols.iter() {
        if sym.needs_plt {
            plt_symbols.push(name.clone());
        }
        if sym.needs_got {
            if sym.is_dynamic {
                got_dyn_symbols.push(name.clone());
            } else {
                got_local_symbols.push(name.clone());
            }
        }
    }
    plt_symbols.sort();
    got_dyn_symbols.sort();
    got_local_symbols.sort();

    for (i, name) in plt_symbols.iter().enumerate() {
        if let Some(sym) = global_symbols.get_mut(name) {
            sym.plt_index = i;
            sym.got_index = i;
        }
    }
    // Dynamic GOT symbols come first (they need .dynsym entries + GLOB_DAT)
    for (i, name) in got_dyn_symbols.iter().enumerate() {
        if let Some(sym) = global_symbols.get_mut(name) {
            sym.got_index = plt_symbols.len() + i;
        }
    }
    // Local GOT symbols come after (filled at link time, no .dynsym needed)
    for (i, name) in got_local_symbols.iter().enumerate() {
        if let Some(sym) = global_symbols.get_mut(name) {
            sym.got_index = plt_symbols.len() + got_dyn_symbols.len() + i;
        }
    }

    let num_plt = plt_symbols.len();
    let num_got_total = plt_symbols.len() + got_dyn_symbols.len() + got_local_symbols.len();
    (
        plt_symbols,
        got_dyn_symbols,
        got_local_symbols,
        num_plt,
        num_got_total,
    )
}

/// The executable's own named IFUNCs.  Static and dynamic executables both
/// call them through an IPLT stub whose GOT slot an `R_386_IRELATIVE` fills
/// (`.rel.iplt` applied by the static startup code, the tail of `.rel.dyn`
/// applied by ld.so); a dynamic executable calling the resolver directly
/// returned the implementation's address instead of calling it.  A
/// shared-library IFUNC is the library's business (`is_dynamic`).
pub(super) fn collect_ifunc_symbols(
    global_symbols: &FxHashMap<String, LinkerSymbol>,
) -> Vec<String> {
    let mut ifunc_symbols: Vec<String> = global_symbols
        .iter()
        .filter(|(_, s)| s.is_defined && !s.is_dynamic && s.sym_type == STT_GNU_IFUNC)
        .map(|(n, _)| n.clone())
        .collect();
    ifunc_symbols.sort();
    ifunc_symbols
}

/// STB_LOCAL IFUNCs (`static T f(void) __attribute__((ifunc("r")))`) of
/// kept sections, by (object, symbol): they get IPLT stubs after the named
/// ones, and every relocation against them resolves to the stub.
pub(super) fn collect_local_ifuncs(
    inputs: &[InputObject],
    section_map: &SectionMap,
) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (obj_idx, obj) in inputs.iter().enumerate() {
        for (sym_idx, sym) in obj.symbols.iter().enumerate() {
            if sym.binding == STB_LOCAL
                && sym.sym_type == STT_GNU_IFUNC
                && section_map.contains_key(&(obj_idx, sym.section_index as usize))
            {
                out.push((obj_idx, sym_idx));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::input::DynlibSyms;
    use super::super::types::*;
    use super::resolve_symbols;
    use crate::common::fx_hash::{FxHashMap, FxHashSet};

    fn resolve(
        inputs: &[InputObject],
        section_map: &SectionMap,
        libs: &DynlibSyms,
    ) -> Result<FxHashMap<String, LinkerSymbol>, String> {
        resolve_symbols(
            inputs,
            section_map,
            &FxHashSet::default(),
            libs,
            false,
            &mut Vec::new(),
            &mut FxHashMap::default(),
        )
    }

    fn sym(name: &str, value: u32, shndx: u16) -> InputSymbol {
        InputSymbol {
            name: name.to_string(),
            value,
            size: 0,
            binding: STB_GLOBAL,
            sym_type: STT_NOTYPE,
            visibility: STV_DEFAULT,
            section_index: shndx,
        }
    }

    /// Input SHN_ABS definitions (`.set k, 100`, assembler constants)
    /// resolve to their value: with no section to add `st_value` to, the
    /// value IS the address.  Regression test — the address used to stay
    /// 0, mislinking every absolute reference (GOT slot, R_386_32) to
    /// address 0.  COMMON keeps address 0 (its `st_value` is an
    /// alignment; `allocate_common_symbols` assigns the real address),
    /// and sectioned symbols keep address 0 with the section offset
    /// recorded (layout assigns the address later).
    #[test]
    fn abs_inputs_resolve_to_their_value() {
        let inputs = [InputObject {
            sections: vec![],
            symbols: vec![
                sym("k", 100, SHN_ABS),
                sym("common_sym", 4, SHN_COMMON),
                sym("in_section", 0x10, 1),
            ],
            filename: "test.o".to_string(),
        }];
        let mut section_map = SectionMap::default();
        section_map.insert((0, 1), (0, 0x100));
        let globals = resolve(&inputs, &section_map, &FxHashMap::default()).unwrap();

        let k = globals.get("k").expect("ABS symbol must resolve");
        assert!(k.is_defined);
        assert_eq!(k.address, 100, "ABS st_value is the address");
        assert_eq!(k.output_section, usize::MAX);

        let c = globals
            .get("common_sym")
            .expect("COMMON symbol must resolve");
        assert_eq!(
            c.address, 0,
            "COMMON st_value is an alignment, not an address"
        );

        let s = globals
            .get("in_section")
            .expect("sectioned symbol must resolve");
        assert_eq!(s.address, 0, "layout assigns sectioned addresses later");
        assert_eq!(s.output_section, 0);
        assert_eq!(s.section_offset, 0x110);
    }

    fn local(name: &str, value: u32, shndx: u16) -> InputSymbol {
        InputSymbol {
            binding: STB_LOCAL,
            ..sym(name, value, shndx)
        }
    }

    fn object(filename: &str, symbols: Vec<InputSymbol>) -> InputObject {
        InputObject {
            sections: vec![],
            symbols,
            filename: filename.to_string(),
        }
    }

    /// An STB_LOCAL definition never satisfies another object's reference:
    /// `extern int helper()` against another file's `static helper` stays
    /// undefined (GNU ld: "undefined reference"), and a `static atoi` does
    /// not capture other objects' references to the shared library's.
    /// Regression test: locals used to be inserted as fallback definitions.
    #[test]
    fn locals_never_satisfy_other_objects_references() {
        let inputs = [
            object("a.o", vec![local("helper", 0, 1), local("atoi", 0x10, 1)]),
            object(
                "b.o",
                vec![sym("helper", 0, SHN_UNDEF), sym("atoi", 0, SHN_UNDEF)],
            ),
        ];
        let mut section_map = SectionMap::default();
        section_map.insert((0, 1), (0, 0));
        let mut libs = FxHashMap::default();
        libs.insert(
            "atoi".to_string(),
            super::super::input::DynlibSym {
                lib: "libc.so.6".to_string(),
                sym_type: STT_FUNC,
                size: 0,
                version: None,
                is_default_ver: true,
                binding: STB_GLOBAL,
                value: 0,
                in_exec: true,
            },
        );
        let globals = resolve(&inputs, &section_map, &libs).unwrap();
        let helper = globals.get("helper").expect("the reference is recorded");
        assert!(!helper.is_defined && !helper.is_dynamic, "{helper:?}");
        let atoi = globals.get("atoi").expect("the reference is recorded");
        assert!(
            atoi.is_dynamic,
            "libc's atoi, not a.o's static one: {atoi:?}"
        );
    }

    #[test]
    fn common_extent_is_checked_before_allocating() {
        let input = object(
            "common.o",
            vec![
                InputSymbol {
                    size: u32::MAX,
                    ..sym("full", 1, SHN_COMMON)
                },
                InputSymbol {
                    size: 1,
                    ..sym("extra", 1, SHN_COMMON)
                },
            ],
        );
        let err = resolve(&[input], &SectionMap::default(), &DynlibSyms::default()).unwrap_err();
        assert!(
            err.contains("COMMON symbol 'extra' makes .bss exceed"),
            "{err}"
        );
    }

    #[test]
    fn bad_common_alignment_is_not_rounded_as_a_power_of_two() {
        let input = object("common.o", vec![sym("bad_align", 3, SHN_COMMON)]);
        let err = resolve(&[input], &SectionMap::default(), &DynlibSyms::default()).unwrap_err();
        assert!(
            err.contains("invalid COMMON alignment 3 for 'bad_align'"),
            "{err}"
        );
    }

    #[test]
    fn an_unmapped_ordinary_definition_is_not_an_equal_absolute() {
        let inputs = [
            object("unmapped.o", vec![sym("pick", 0, 1)]),
            object("abs.o", vec![sym("pick", 0, SHN_ABS)]),
        ];
        let err = resolve(&inputs, &SectionMap::default(), &DynlibSyms::default()).unwrap_err();
        assert!(err.contains("abs.o; first defined in unmapped.o"), "{err}");
    }

    #[test]
    fn section_offset_cannot_wrap() {
        let input = object("overflow.o", vec![sym("at_end", u32::MAX, 1)]);
        let mut map = SectionMap::default();
        map.insert((0, 1), (0, 1));
        let err = resolve(&[input], &map, &DynlibSyms::default()).unwrap_err();
        assert!(err.contains("section offset for 'at_end' exceeds"), "{err}");
    }
}
