//! PLT/GOT construction and IFUNC collection for the x86-64 linker.
//!
//! Scans object file relocations to determine which symbols need PLT stubs
//! or GOT entries, and collects IFUNC symbols for IRELATIVE relocations.

use crate::common::fx_hash::FxHashMap;

use super::elf::*;
use super::types::GlobalSymbol;

/// Local (`STB_LOCAL`) IFUNC symbols, as `(object index, symbol index)`.
///
/// `collect_ifunc_symbols` can only see global IFUNCs, because it walks
/// `globals` -- and a `static int fn(void) __attribute__((ifunc("rsv")))` never
/// gets promoted there.  Such a symbol used to resolve through the ordinary
/// section path, which binds the call site straight to the *resolver*: the
/// caller receives the implementation's address as the call's "return value"
/// and prints something like `4199558` where `1` was meant.  Nothing diagnoses
/// it; the program simply computes the wrong answer.
///
/// Keyed by index rather than name because local symbols legitimately repeat
/// across objects and must not be merged.
pub(super) fn collect_local_ifuncs(
    objects: &[crate::backend::linker_common::Elf64Object],
    dead_sections: &crate::common::fx_hash::FxHashSet<(usize, usize)>,
) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (obj_idx, obj) in objects.iter().enumerate() {
        for (sym_idx, sym) in obj.symbols.iter().enumerate() {
            if sym.sym_type() != STT_GNU_IFUNC
                || !sym.is_local()
                || sym.shndx == crate::backend::elf::SHN_UNDEF
                || sym.shndx == crate::backend::elf::SHN_ABS
            {
                continue;
            }
            // A resolver in a collected section cannot run; skip it rather than
            // emit an IRELATIVE pointing at unmapped memory.
            if dead_sections.contains(&(obj_idx, sym.shndx as usize)) {
                continue;
            }
            out.push((obj_idx, sym_idx));
        }
    }
    // Deterministic slot order: the IPLT, the IFUNC GOT and the IRELATIVE table
    // are all indexed by position, so it must not depend on iteration order.
    out.sort_unstable();
    out
}

pub(super) fn collect_ifunc_symbols(
    globals: &FxHashMap<String, GlobalSymbol>,
    _is_static: bool,
) -> Vec<String> {
    // IFUNCs need IRELATIVE handling in BOTH static and dynamic executables.
    // Static: .rela.iplt applied by glibc csu via __rela_iplt_start/end.
    // Dynamic: IRELATIVE entries appended to .rela.dyn, applied by ld.so.
    // Without this, calls bind directly to the RESOLVER, so callers receive
    // the implementation's address as the "return value" of the call.
    let mut ifunc_symbols: Vec<String> = globals
        .iter()
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

/// One `R_X86_64_64` relocation that a PIE turns into `R_X86_64_RELATIVE`.
///
/// A position-independent executable is mapped at a base chosen by the kernel,
/// so every absolute address stored in a writable section has to be slid by
/// that base at load time.  `ld.so` does the sliding; our job is to hand it one
/// `RELATIVE` entry per such storage, carrying the link-time value as the
/// addend.
///
/// The entry stores *coordinates*, not a value: the value depends on final
/// addresses, which do not exist yet when this list is built.  `emit_exec`
/// resolves the coordinates exactly once, when it fills `.rela.dyn`.  Building
/// the list here -- in the same relocation walk that already decides PLT, GOT,
/// copy relocs and `AbsDynReloc` -- is what makes `DT_RELASZ` agree with the
/// bytes written: the count and the emission read the same `Vec`, so they
/// cannot drift apart.  A separate counting pass with its own predicate would
/// be free to disagree, and a wrong `DT_RELASZ` silently corrupts the image.
pub(super) struct PieRelative {
    pub obj_idx: usize,
    pub sec_idx: usize,
    pub offset: u64,
    pub addend: i64,
    /// `sym_idx` of the relocation's symbol, kept so the emitter resolves the
    /// target through the identical path the non-PIE applier uses (including
    /// string-merge remapping, COMDAT losers and ICF folds).
    pub sym_idx: usize,
}

/// Decide whether an `R_X86_64_64` relocation in a PIE becomes `RELATIVE`.
///
/// Address-independent by construction, so it can be called during the scan
/// that runs before layout.  Mirrors the `deferred_to_loader` predicate in
/// `emit_exec`'s relocation applier: a reference to a *dynamic* symbol is
/// resolved by the loader through `GLOB_DAT`/`JUMP_SLOT`/`AbsDynReloc`, not by
/// a slide, so it must not also get a `RELATIVE`.  Everything that resolves to
/// an address inside this output does need one -- including references through
/// section symbols, which is how the compiler points an array of `const char *`
/// at merged string literals.  Those are the easy ones to miss, and missing one
/// leaves a pointer holding its link-time offset, which faults on first use
/// under ASLR.
/// True when the value the relocation applier will store is an address inside
/// *this* output, and therefore has to be slid by the load base.
///
/// Mirrors `emit_exec`'s applier arm for `R_X86_64_64` case for case, because a
/// disagreement is a wrong image either way: a pointer that is never slid, or a
/// slide applied on top of an address the loader is about to overwrite.
pub(super) fn stored_value_is_local(g: &GlobalSymbol) -> bool {
    if g.is_dynamic && !g.copy_reloc {
        // The applier stores our own PLT entry when the symbol has one -- a
        // local address, so it needs sliding.  This is how an `.eh_frame` FDE
        // points at `__gxx_personality_v0`; missing it means the unwinder jumps
        // to the unslid PLT address and the first C++ throw segfaults.
        // With no PLT the applier defers to ld.so via `abs_dyn_relocs`, and a
        // slide there would double-apply.
        return g.plt_idx.is_some();
    }
    // A definition we own: a normal local/global definition, a copy-relocated
    // symbol's .bss copy, or one of the synthetic `__lccc.strmerge.N` pool
    // symbols that string merging substitutes for references into merged string
    // sections (those carry the per-string offset in the *addend* and look
    // undefined -- `shndx == 0` -- but are entirely local).
    g.defined_in.is_some()
}

fn pie_needs_relative(
    sym: &crate::backend::linker_common::Elf64Symbol,
    globals: &FxHashMap<String, GlobalSymbol>,
) -> bool {
    if !sym.name.is_empty() && !sym.is_local() {
        if let Some(g) = globals.get(sym.name.as_str()) {
            return stored_value_is_local(g);
        }
        if sym.is_weak() {
            return false; // undefined weak resolves to 0; nothing to slide
        }
    }
    !sym.is_undefined()
}

pub(super) fn create_plt_got(
    objects: &[ElfObject],
    globals: &mut FxHashMap<String, GlobalSymbol>,
    is_pie: bool,
) -> (
    Vec<String>,
    Vec<(String, bool)>,
    Vec<AbsDynReloc>,
    Vec<PieRelative>,
) {
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
    let mut pie_relative: Vec<PieRelative> = Vec::new();

    for (obj_i, obj) in objects.iter().enumerate() {
        for sec_idx in 0..obj.sections.len() {
            for rela in &obj.relocations[sec_idx] {
                let si = rela.sym_idx as usize;
                if si >= obj.symbols.len() {
                    continue;
                }
                let sym = &obj.symbols[si];
                // PIE slide entries must be collected *before* the local-symbol
                // skip below.  Locally-defined absolute references are exactly
                // the ones a slide applies to, and the ones most often written
                // through a section symbol -- `const char *msgs[] = {...}`
                // relocates against the merged `.rodata.str1.1` section symbol,
                // not against a named symbol.  Everything after this point only
                // concerns references that cross the dynamic boundary.
                if is_pie && rela.rela_type == R_X86_64_64 && pie_needs_relative(sym, globals) {
                    pie_relative.push(PieRelative {
                        obj_idx: obj_i,
                        sec_idx,
                        offset: rela.offset,
                        addend: rela.addend,
                        sym_idx: si,
                    });
                }
                if sym.name.is_empty() || sym.is_local() {
                    continue;
                }
                let gsym_info = globals
                    .get(sym.name.as_str())
                    .map(|g| (g.is_dynamic, g.info & 0xf));

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
                            if plt_set.insert(sym.name.to_string()) {
                                plt_names.push(sym.name.to_string());
                            }
                        }
                    }
                    R_X86_64_GOTPCREL
                    | R_X86_64_GOTPCRELX
                    | R_X86_64_REX_GOTPCRELX
                    | R_X86_64_CODE_4_GOTPCRELX
                    | R_X86_64_CODE_6_GOTPCRELX => {
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
                    R_X86_64_GOTTPOFF | R_X86_64_CODE_4_GOTTPOFF | R_X86_64_CODE_6_GOTTPOFF => {
                        if !plt_set.contains(sym.name.as_str())
                            && got_only_set.insert(sym.name.to_string())
                        {
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
                            && got_only_set.insert(sym.name.to_string())
                        {
                            got_only_names.push(sym.name.to_string());
                        }
                    }
                    _ if gsym_info.map(|g| g.0).unwrap_or(false) => {
                        let sym_type = gsym_info.map(|g| g.1).unwrap_or(0);
                        if rela.rela_type == R_X86_64_64 {
                            if sym_type != STT_OBJECT {
                                // Function pointer initialised from a dynamic
                                // function: the canonical address is its PLT entry.
                                if plt_set.insert(sym.name.to_string()) {
                                    plt_names.push(sym.name.to_string());
                                }
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
                        } else if !plt_set.contains(sym.name.as_str())
                            && got_only_set.insert(sym.name.to_string())
                        {
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
        let alias_names: Vec<String> = globals
            .iter()
            .filter(|(name, g)| {
                g.is_dynamic
                    && !g.copy_reloc
                    && (g.info & 0xf) == STT_OBJECT
                    && !copy_reloc_set.contains(*name)
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
    if is_pie && std::env::var("LCCC_DEBUG_GOT").is_ok() {
        eprintln!("[GOT] pie_relative={} entries", pie_relative.len());
    }
    (plt_names, got_entries, abs_dyn_relocs, pie_relative)
}
