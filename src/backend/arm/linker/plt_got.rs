//! PLT/GOT construction for the AArch64 linker.
//!
//! Scans object file relocations to determine which symbols need PLT stubs
//! (for function calls via BL/B) or GOT entries (for data references via ADRP),
//! and builds the PLT/GOT entry lists.

use crate::common::fx_hash::FxHashMap;

use super::elf::*;
use super::types::GlobalSymbol;

/// True when a dynamic symbol of ELF type `sym_type`, defined at a library
/// address inside (`lib_in_exec`) or outside an executable LOAD segment, is
/// COPY-relocated by a data reference. Mirrors the x86-64 classifier:
/// `STT_OBJECT` always qualifies, `STT_NOTYPE` only in data (an untyped
/// function keeps its PLT).
fn copy_data_type(sym_type: u8, lib_in_exec: bool) -> bool {
    sym_type == STT_OBJECT || (sym_type == STT_NOTYPE && !lib_in_exec)
}

pub(super) fn create_plt_got(
    objects: &[ElfObject],
    globals: &mut FxHashMap<String, GlobalSymbol>,
) -> (Vec<String>, Vec<(String, bool)>) {
    let mut plt_names: Vec<String> = Vec::new();
    let mut got_only_names: Vec<String> = Vec::new();
    let mut copy_reloc_names: Vec<String> = Vec::new();

    for obj in objects {
        for sec_idx in 0..obj.sections.len() {
            for rela in &obj.relocations[sec_idx] {
                let si = rela.sym_idx as usize;
                if si >= obj.symbols.len() {
                    continue;
                }
                let sym = &obj.symbols[si];
                if sym.name.is_empty() {
                    continue;
                }
                let gsym_info = globals
                    .get(sym.name.as_str())
                    .map(|g| (g.is_dynamic, g.info & 0xf, g.lib_in_exec));

                match rela.rela_type {
                    R_AARCH64_CALL26 | R_AARCH64_JUMP26
                        if gsym_info.map(|g| g.0).unwrap_or(false) =>
                    {
                        let sym_type = gsym_info.map(|g| g.1).unwrap_or(0);
                        let lib_in_exec = gsym_info.map(|g| g.2).unwrap_or(false);
                        if copy_data_type(sym_type, lib_in_exec) {
                            if !copy_reloc_names.contains(&sym.name.to_string()) {
                                copy_reloc_names.push(sym.name.to_string());
                            }
                        } else if !plt_names.contains(&sym.name.to_string()) {
                            plt_names.push(sym.name.to_string());
                        }
                    }
                    R_AARCH64_ADR_PREL_PG_HI21
                    | R_AARCH64_ADD_ABS_LO12_NC
                    | R_AARCH64_LDST64_ABS_LO12_NC
                    | R_AARCH64_LDST32_ABS_LO12_NC
                    | R_AARCH64_LDST8_ABS_LO12_NC
                    | R_AARCH64_LDST16_ABS_LO12_NC
                    | R_AARCH64_LDST128_ABS_LO12_NC
                        if gsym_info.map(|g| g.0).unwrap_or(false) =>
                    {
                        let sym_type = gsym_info.map(|g| g.1).unwrap_or(0);
                        let lib_in_exec = gsym_info.map(|g| g.2).unwrap_or(false);
                        if copy_data_type(sym_type, lib_in_exec) {
                            if !copy_reloc_names.contains(&sym.name.to_string()) {
                                copy_reloc_names.push(sym.name.to_string());
                            }
                        } else {
                            // Function referenced via ADRP+ADD (e.g., taking address)
                            if !plt_names.contains(&sym.name.to_string()) {
                                plt_names.push(sym.name.to_string());
                            }
                        }
                    }
                    R_AARCH64_ABS64 if gsym_info.map(|g| g.0).unwrap_or(false) => {
                        let sym_type = gsym_info.map(|g| g.1).unwrap_or(0);
                        let lib_in_exec = gsym_info.map(|g| g.2).unwrap_or(false);
                        if !copy_data_type(sym_type, lib_in_exec) {
                            if !plt_names.contains(&sym.name.to_string()) {
                                plt_names.push(sym.name.to_string());
                            }
                        } else if !copy_reloc_names.contains(&sym.name.to_string()) {
                            copy_reloc_names.push(sym.name.to_string());
                        }
                    }
                    R_AARCH64_ADR_GOT_PAGE | R_AARCH64_LD64_GOT_LO12_NC => {
                        if !got_only_names.contains(&sym.name.to_string())
                            && !plt_names.contains(&sym.name.to_string())
                        {
                            got_only_names.push(sym.name.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Mark copy relocation symbols
    let mut copy_reloc_lib_addrs: Vec<(String, u64)> = Vec::new();
    for name in &copy_reloc_names {
        if let Some(gsym) = globals.get_mut(name) {
            gsym.copy_reloc = true;
            if let Some(ref lib) = gsym.from_lib {
                if copy_data_type(gsym.info & 0xf, gsym.lib_in_exec) && gsym.lib_sym_value != 0 {
                    let key = (lib.clone(), gsym.lib_sym_value);
                    if !copy_reloc_lib_addrs.contains(&key) {
                        copy_reloc_lib_addrs.push(key);
                    }
                }
            }
        }
    }
    // Mark aliases
    if !copy_reloc_lib_addrs.is_empty() {
        let alias_names: Vec<String> = globals
            .iter()
            .filter(|(name, g)| {
                g.is_dynamic
                    && !g.copy_reloc
                    && copy_data_type(g.info & 0xf, g.lib_in_exec)
                    && !copy_reloc_names.contains(name)
                    && g.from_lib.is_some()
                    && g.lib_sym_value != 0
                    && copy_reloc_lib_addrs
                        .contains(&(g.from_lib.as_ref().unwrap().clone(), g.lib_sym_value))
            })
            .map(|(n, _)| n.clone())
            .collect();
        for name in alias_names {
            if let Some(gsym) = globals.get_mut(&name) {
                gsym.copy_reloc = true;
            }
        }
    }

    // Build GOT entries: [0]=.dynamic, [1]=reserved, [2]=reserved, then PLT entries, then GOT-only
    let mut got_entries: Vec<(String, bool)> = Vec::new();
    got_entries.push((String::new(), false)); // GOT[0]
    got_entries.push((String::new(), false)); // GOT[1]
    got_entries.push((String::new(), false)); // GOT[2]

    for (plt_idx, name) in plt_names.iter().enumerate() {
        let got_idx = got_entries.len();
        got_entries.push((name.clone(), true));
        if let Some(gsym) = globals.get_mut(name) {
            gsym.plt_idx = Some(plt_idx);
            gsym.got_idx = Some(got_idx);
        }
    }

    for name in &got_only_names {
        let got_idx = got_entries.len();
        got_entries.push((name.clone(), false));
        if let Some(gsym) = globals.get_mut(name) {
            gsym.got_idx = Some(got_idx);
        }
    }

    (plt_names, got_entries)
}
