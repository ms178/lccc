//! PLT/GOT construction and IFUNC collection for the x86-64 linker.
//!
//! Scans object file relocations to determine which symbols need PLT stubs
//! or GOT entries, and collects IFUNC symbols for IRELATIVE relocations.

use crate::common::fx_hash::FxHashMap;

use super::elf::*;
use super::types::GlobalSymbol;

pub(super) fn collect_ifunc_symbols(globals: &FxHashMap<String, GlobalSymbol>, _is_static: bool) -> Vec<String> {
    // IFUNCs need IRELATIVE handling in BOTH static and dynamic executables.
    // Static: .rela.iplt applied by glibc csu via __rela_iplt_start/end.
    // Dynamic: IRELATIVE entries appended to .rela.dyn, applied by ld.so.
    // Without this, calls bind directly to the RESOLVER, so callers receive
    // the implementation's address as the "return value" of the call.
    let mut ifunc_symbols: Vec<String> = globals.iter()
        .filter(|(_, g)| g.defined_in.is_some() && (g.info & 0xf) == STT_GNU_IFUNC)
        .map(|(n, _)| n.clone())
        .collect();
    ifunc_symbols.sort();
    ifunc_symbols
}

/// One absolute (`R_X86_64_64`) relocation against a dynamic data symbol.
///
/// These cannot be resolved at link time in an ET_EXEC image: the target lives
/// in a shared library whose load address is unknown until `ld.so` maps it.
/// GNU ld emits a dynamic `R_X86_64_64` for each one so the loader patches the
/// storage in place; see `AbsDynReloc` use in `emit_exec`.
#[derive(Clone, Debug)]
pub(super) struct AbsDynReloc {
    /// Symbol the relocation refers to (index into the dynamic symbol table is
    /// resolved later, once that table is laid out).
    pub name: String,
    /// Input object and section holding the storage to be patched.
    pub obj_idx: usize,
    pub sec_idx: usize,
    /// Offset of the storage within that input section.
    pub offset: u64,
    /// Addend to hand the loader (e.g. `+0x10` to skip a vtable's RTTI header).
    pub addend: i64,
}

pub(super) fn create_plt_got(
    objects: &[ElfObject], globals: &mut FxHashMap<String, GlobalSymbol>,
) -> (Vec<String>, Vec<(String, bool)>, Vec<AbsDynReloc>) {
    // Ordered vectors preserve deterministic layout; the shadow HashSets make
    // membership tests O(1). With tens of thousands of GOT symbols (e.g. a
    // kernel-sized link) the previous Vec::contains scans were O(n^2) and
    // dominated total link time.
    use crate::common::fx_hash::FxHashSet;
    let mut plt_names: Vec<String> = Vec::new();
    let mut plt_set: FxHashSet<String> = FxHashSet::default();
    let mut got_only_names: Vec<String> = Vec::new();
    let mut got_only_set: FxHashSet<String> = FxHashSet::default();
    let mut copy_reloc_names: Vec<String> = Vec::new();
    let mut copy_reloc_set: FxHashSet<String> = FxHashSet::default();
    let mut abs_dyn_relocs: Vec<AbsDynReloc> = Vec::new();

    for (obj_i, obj) in objects.iter().enumerate() {
        for sec_idx in 0..obj.sections.len() {
            for rela in &obj.relocations[sec_idx] {
                let si = rela.sym_idx as usize;
                if si >= obj.symbols.len() { continue; }
                let sym = &obj.symbols[si];
                if sym.name.is_empty() || sym.is_local() { continue; }
                let gsym_info = globals.get(sym.name.as_str()).map(|g| (g.is_dynamic, g.info & 0xf));

                match rela.rela_type {
                    R_X86_64_PLT32 | R_X86_64_PC32 if gsym_info.map(|g| g.0).unwrap_or(false) => {
                        let sym_type = gsym_info.map(|g| g.1).unwrap_or(0);
                        if sym_type == STT_OBJECT {
                            // Dynamic data symbol - needs copy relocation
                            if copy_reloc_set.insert(sym.name.to_string()) {
                                copy_reloc_names.push(sym.name.to_string());
                            }
                        } else {
                            // Dynamic function symbol - needs PLT
                            if plt_set.insert(sym.name.to_string()) { plt_names.push(sym.name.to_string()); }
                        }
                    }
                    R_X86_64_GOTPCREL | R_X86_64_GOTPCRELX | R_X86_64_REX_GOTPCRELX => {
                        // GOTPCREL always needs a dedicated GOT entry, even if the
                        // symbol also has a PLT entry. The PLT's GOT.PLT slot uses
                        // JUMP_SLOT (lazy binding, initially PLT+6) which is wrong
                        // for address-of. For symbols with PLT, the GOT entry is
                        // statically filled with the PLT address (no GLOB_DAT);
                        // for other dynamic symbols, GLOB_DAT is used.
                        if got_only_set.insert(sym.name.to_string()) {
                            got_only_names.push(sym.name.to_string());
                        }
                    }
                    R_X86_64_GOTTPOFF => {
                        if !plt_set.contains(sym.name.as_str()) && got_only_set.insert(sym.name.to_string()) {
                            got_only_names.push(sym.name.to_string());
                        }
                    }
                    R_X86_64_TLSGD => {
                        // GD against a dynamic TLS symbol relaxes to IE, which
                        // needs a GOT slot carrying an R_X86_64_TPOFF64 dynamic
                        // relocation. GD against local symbols relaxes to LE
                        // (no GOT entry needed).
                        if gsym_info.map(|g| g.0).unwrap_or(false)
                            && !plt_set.contains(sym.name.as_str())
                            && got_only_set.insert(sym.name.to_string()) {
                            got_only_names.push(sym.name.to_string());
                        }
                    }
                    _ if gsym_info.map(|g| g.0).unwrap_or(false) => {
                        let sym_type = gsym_info.map(|g| g.1).unwrap_or(0);
                        if rela.rela_type == R_X86_64_64 {
                            if sym_type != STT_OBJECT {
                                // Function pointer initialised from a dynamic
                                // function: the canonical address is its PLT entry.
                                if plt_set.insert(sym.name.to_string()) { plt_names.push(sym.name.to_string()); }
                            } else {
                                // Absolute 64-bit reference to a dynamic DATA
                                // symbol, e.g. the `_ZTVN10__cxxabiv1*` vtable
                                // pointer that every C++ typeinfo object starts
                                // with. The address is only known once ld.so has
                                // mapped libstdc++, so it must become a dynamic
                                // R_X86_64_64 -- a GOT slot is useless here
                                // because the storage being initialised is the
                                // typeinfo object itself, not a GOT entry.
                                //
                                // Getting this wrong is silent and lethal: the
                                // slot keeps just the addend (0x10), and the
                                // first C++ throw of a class type segfaults
                                // inside __gxx_personality_v0 while reading the
                                // LSDA type table.
                                abs_dyn_relocs.push(AbsDynReloc {
                                    name: sym.name.to_string(),
                                    obj_idx: obj_i,
                                    sec_idx,
                                    offset: rela.offset,
                                    addend: rela.addend,
                                });
                            }
                        } else if !plt_set.contains(sym.name.as_str()) && got_only_set.insert(sym.name.to_string()) {
                            got_only_names.push(sym.name.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Mark copy relocation symbols and their aliases.
    // When a symbol like `environ` (WEAK) needs a COPY relocation, we must also
    // mark aliases like `__environ` (GLOBAL) at the same shared library address.
    // This ensures the dynamic linker redirects all references to our BSS copy.
    let mut copy_reloc_lib_addrs: Vec<(String, u64)> = Vec::new(); // (from_lib, lib_sym_value)
    for name in &copy_reloc_names {
        if let Some(gsym) = globals.get_mut(name) {
            gsym.copy_reloc = true;
            if let Some(ref lib) = gsym.from_lib {
                if (gsym.info & 0xf) == STT_OBJECT && gsym.lib_sym_value != 0 {
                    let key = (lib.clone(), gsym.lib_sym_value);
                    if !copy_reloc_lib_addrs.contains(&key) {
                        copy_reloc_lib_addrs.push(key);
                    }
                }
            }
        }
    }
    // Also mark aliases (other dynamic STT_OBJECT symbols at the same library address)
    if !copy_reloc_lib_addrs.is_empty() {
        let alias_names: Vec<String> = globals.iter()
            .filter(|(name, g)| {
                g.is_dynamic && !g.copy_reloc && (g.info & 0xf) == STT_OBJECT
                    && !copy_reloc_set.contains(*name)
                    && g.from_lib.is_some() && g.lib_sym_value != 0
                    && copy_reloc_lib_addrs.contains(
                        &(g.from_lib.as_ref().unwrap().clone(), g.lib_sym_value))
            })
            .map(|(n, _)| n.clone())
            .collect();
        for name in alias_names {
            if let Some(gsym) = globals.get_mut(&name) {
                gsym.copy_reloc = true;
            }
        }
    }

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

    if std::env::var("LCCC_DEBUG_GOT").is_ok() {
        for (i, (name, is_plt)) in got_entries.iter().enumerate() {
            eprintln!("[GOT] idx={} plt={} name={:?}", i, is_plt, name);
        }
    }
    (plt_names, got_entries, abs_dyn_relocs)
}
