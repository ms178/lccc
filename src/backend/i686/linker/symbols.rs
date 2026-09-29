//! Symbol resolution for the i686 linker.
//!
//! Phases 6-9: global symbol resolution, COMMON symbol allocation,
//! PLT/GOT marking, undefined symbol checking, PLT/GOT list building,
//! and IFUNC collection.

use crate::common::fx_hash::{FxHashMap, FxHashSet};

use super::types::*;
use crate::backend::linker_common;

pub(super) fn resolve_symbols(
    inputs: &[InputObject],
    _output_sections: &[OutputSection],
    section_map: &SectionMap,
    dynlib_syms: &super::input::DynlibSyms,
) -> (
    FxHashMap<String, LinkerSymbol>,
    FxHashMap<(usize, usize), String>,
) {
    let mut global_symbols: FxHashMap<String, LinkerSymbol> = FxHashMap::default();
    let mut sym_resolution: FxHashMap<(usize, usize), String> = FxHashMap::default();

    // First pass: collect definitions
    for (obj_idx, obj) in inputs.iter().enumerate() {
        for (sym_idx, sym) in obj.symbols.iter().enumerate() {
            if sym.name.is_empty() || sym.sym_type == STT_FILE || sym.sym_type == STT_SECTION {
                continue;
            }
            if sym.section_index == SHN_UNDEF {
                continue;
            }

            let (out_sec_idx, sec_offset) =
                if sym.section_index != SHN_ABS && sym.section_index != SHN_COMMON {
                    section_map
                        .get(&(obj_idx, sym.section_index as usize))
                        .copied()
                        .unwrap_or((usize::MAX, 0))
                } else {
                    (usize::MAX, 0)
                };

            // SHN_ABS input definitions (`.set k, 100`, assembler-provided
            // constants) carry their value in `st_value` with no section to
            // add it to: the value IS the address, so materialise it here.
            // Leaving it 0 mislinked every absolute reference (GOT slot,
            // R_386_32) to address 0.  SHN_COMMON keeps address 0 — its
            // `st_value` is an alignment, and `allocate_common_symbols`
            // assigns the real address later.
            let address = if sym.section_index == SHN_ABS {
                sym.value
            } else {
                0
            };

            let new_sym = LinkerSymbol {
                address,
                size: sym.size,
                sym_type: sym.sym_type,
                binding: sym.binding,
                visibility: sym.visibility,
                is_defined: true,
                needs_plt: false,
                needs_got: false,
                output_section: out_sec_idx,
                section_offset: sec_offset + sym.value,
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

            match global_symbols.get(sym.name.as_str()) {
                None => {
                    // Note: STB_LOCAL symbols are deliberately inserted here when no
                    // entry exists yet. Unlike ELF64 backends that skip locals entirely,
                    // the i686 backend must allow locals as fallback definitions because
                    // glibc's static archives contain cross-object references that resolve
                    // through local symbols. The Some arm below prevents locals from
                    // *overriding* any existing entry (global, weak, or other local).
                    global_symbols.insert(sym.name.to_string(), new_sym);
                }
                Some(existing) => {
                    // Local symbols must not override any existing entry.
                    // They have file scope only and should not shadow globals
                    // or weaks from other objects (e.g. a static "data" in one
                    // file must not shadow a global "data" in another).
                    if sym.binding == STB_LOCAL {
                        // Already have a definition; keep it.
                    } else if sym.binding == STB_GLOBAL
                        && (existing.binding == STB_WEAK || existing.binding == STB_LOCAL)
                        || (!existing.is_defined && new_sym.is_defined)
                    {
                        global_symbols.insert(sym.name.to_string(), new_sym);
                    }
                }
            }

            sym_resolution.insert((obj_idx, sym_idx), sym.name.clone());
        }
    }

    // Second pass: resolve undefined references against dynamic libraries
    for (obj_idx, obj) in inputs.iter().enumerate() {
        for (sym_idx, sym) in obj.symbols.iter().enumerate() {
            if sym.name.is_empty() || sym.sym_type == STT_FILE {
                continue;
            }
            sym_resolution.insert((obj_idx, sym_idx), sym.name.clone());

            if sym.section_index == SHN_UNDEF {
                if global_symbols.contains_key(sym.name.as_str()) {
                    continue;
                }

                if let Some(d) = dynlib_syms.get(sym.name.as_str()) {
                    global_symbols.insert(sym.name.to_string(), LinkerSymbol::dynamic_import(d));
                } else {
                    global_symbols
                        .entry(sym.name.clone())
                        .or_insert(LinkerSymbol {
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
                        });
                }
            }
        }
    }

    // gABI: the most constraining visibility over the definition and all
    // references applies to the symbol (INTERNAL > HIDDEN > PROTECTED >
    // DEFAULT, i.e. the smallest non-zero value).
    for obj in inputs {
        for sym in &obj.symbols {
            if sym.binding == STB_LOCAL || sym.visibility == STV_DEFAULT || sym.name.is_empty() {
                continue;
            }
            if let Some(gs) = global_symbols.get_mut(sym.name.as_str()) {
                if gs.binding != STB_LOCAL
                    && (gs.visibility == STV_DEFAULT || sym.visibility < gs.visibility)
                {
                    gs.visibility = sym.visibility;
                }
            }
        }
    }

    // Resolve section symbols
    for (obj_idx, obj) in inputs.iter().enumerate() {
        for (sym_idx, sym) in obj.symbols.iter().enumerate() {
            if sym.sym_type == STT_SECTION && sym.section_index != SHN_UNDEF {
                sym_resolution.insert(
                    (obj_idx, sym_idx),
                    format!("__section_{}_{}", obj_idx, sym.section_index),
                );
            }
        }
    }

    (global_symbols, sym_resolution)
}

// ══════════════════════════════════════════════════════════════════════════════
// Phase 6b: Allocate COMMON symbols in .bss
// ══════════════════════════════════════════════════════════════════════════════

/// Allocate COMMON symbols (tentative definitions) in the .bss section.
///
/// In C, a global variable declared without an initializer (e.g. `int x;`) may be
/// emitted as a COMMON symbol (SHN_COMMON) by the compiler. These symbols need
/// space allocated in .bss during linking. For each COMMON symbol, we:
/// 1. Find or create the .bss output section
/// 2. Align the current offset to the symbol's alignment requirement
/// 3. Update the LinkerSymbol to point into the .bss section
pub(super) fn allocate_common_symbols(
    inputs: &[InputObject],
    output_sections: &mut Vec<OutputSection>,
    section_name_to_idx: &mut FxHashMap<String, usize>,
    global_symbols: &mut FxHashMap<String, LinkerSymbol>,
) {
    // Collect COMMON symbols: (name, alignment, size)
    // For COMMON symbols, InputSymbol.value is the alignment requirement, .size is the size.
    let mut common_syms: Vec<(String, u32, u32)> = Vec::new();
    for obj in inputs.iter() {
        for sym in &obj.symbols {
            if sym.section_index == SHN_COMMON && !sym.name.is_empty() {
                // Only add if this symbol is still in global_symbols with output_section == usize::MAX
                // (i.e., it wasn't overridden by a real definition from another object)
                if let Some(gs) = global_symbols.get(sym.name.as_str()) {
                    if gs.output_section == usize::MAX && gs.is_defined && !gs.is_dynamic {
                        // Check we haven't already added this symbol (could appear in multiple objects)
                        if !common_syms.iter().any(|(n, _, _)| n == &sym.name) {
                            common_syms.push((sym.name.clone(), sym.value.max(1), sym.size));
                        }
                    }
                }
            }
        }
    }

    if common_syms.is_empty() {
        return;
    }

    // Find or create .bss section
    let bss_idx = if let Some(&idx) = section_name_to_idx.get(".bss") {
        idx
    } else {
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
    };

    let mut bss_off = output_sections[bss_idx].data.len() as u32;
    for (name, alignment, size) in &common_syms {
        let a = (*alignment).max(1);
        bss_off = (bss_off + a - 1) & !(a - 1);

        if let Some(sym) = global_symbols.get_mut(name) {
            sym.output_section = bss_idx;
            sym.section_offset = bss_off;
        }

        if *alignment > output_sections[bss_idx].align {
            output_sections[bss_idx].align = *alignment;
        }
        bss_off += size;
    }

    // Extend .bss data to reflect the new size
    let new_len = bss_off as usize;
    if new_len > output_sections[bss_idx].data.len() {
        output_sections[bss_idx].data.resize(new_len, 0);
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Phase 7: PLT/GOT marking + undefined check
// ══════════════════════════════════════════════════════════════════════════════

pub(super) fn mark_plt_got_needs(
    inputs: &[InputObject],
    global_symbols: &mut FxHashMap<String, LinkerSymbol>,
    _is_static: bool,
) {
    for obj in inputs.iter() {
        for sec in &obj.sections {
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

                if sym.sym_type == STT_SECTION || sym.name.is_empty() {
                    continue;
                }
                if dead_calls.contains(&rel_offset) {
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
                    R_386_GOT32 | R_386_GOT32X => {
                        if let Some(gs) = global_symbols.get_mut(sym.name.as_str()) {
                            gs.needs_got = true;
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

pub(super) fn collect_ifunc_symbols(
    global_symbols: &FxHashMap<String, LinkerSymbol>,
    is_static: bool,
) -> Vec<String> {
    if !is_static {
        return Vec::new();
    }
    let mut ifunc_symbols: Vec<String> = global_symbols
        .iter()
        .filter(|(_, s)| s.is_defined && s.sym_type == STT_GNU_IFUNC)
        .map(|(n, _)| n.clone())
        .collect();
    ifunc_symbols.sort();
    ifunc_symbols
}

#[cfg(test)]
mod tests {
    use super::super::types::*;
    use super::resolve_symbols;
    use crate::common::fx_hash::FxHashMap;

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
        let (globals, _) = resolve_symbols(&inputs, &[], &section_map, &FxHashMap::default());

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
}
