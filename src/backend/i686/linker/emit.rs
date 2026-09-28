//! Executable emission for the i686 linker.
//!
//! Phase 10: lays out segments, assigns addresses, applies relocations,
//! builds PLT/GOT/dynamic sections, and writes the final ELF32 executable.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use std::collections::BTreeSet;

use super::DynStrTab;
use super::dynamic::{DynamicDesc, option_entries};
use super::gnu_hash::build_gnu_hash_32;
use super::options::{DynValue, LinkOptions};
use super::reloc::{self, RelocContext};
use super::types::*;
use crate::backend::linker_common;

pub(super) fn emit_executable(
    inputs: &[InputObject],
    output_sections: &mut Vec<OutputSection>,
    section_name_to_idx: &FxHashMap<String, usize>,
    section_map: &SectionMap,
    global_symbols: &mut FxHashMap<String, LinkerSymbol>,
    _sym_resolution: &FxHashMap<(usize, usize), String>,
    plt_symbols: &[String],
    got_dyn_symbols: &[String],
    got_local_symbols: &[String],
    num_plt: usize,
    _num_got_total: usize,
    ifunc_symbols: &[String],
    is_static: bool,
    needed: &[String],
    opts: &LinkOptions,
    dso_refs: &FxHashSet<String>,
    output_path: &str,
    pending_defsyms: &[(String, String, usize)],
    interp: &[u8],
) -> Result<(), String> {
    let num_ifunc = ifunc_symbols.len();

    // ── Stack executability (GNU C nested-function trampolines) ────────────
    // The compiler emits `.note.GNU-stack,"x"` whenever a unit initialises
    // trampoline code; binutils ORs all input notes into the final
    // PT_GNU_STACK flags. Mirror that here: any input marking its note
    // section SHF_EXECINSTR makes the final stack executable. Without this,
    // PIE-default builds segfault on the first indirect nested call.
    // `-z execstack` / `-z noexecstack` override the input markers.
    let exec_stack = opts
        .exec_stack
        .unwrap_or_else(|| inputs_want_exec_stack(inputs));
    let stack_flags = PF_R | PF_W | if exec_stack { PF_X } else { 0 };

    // ── Build dynamic symbol/string tables ────────────────────────────────
    // DT_NEEDED is exactly what the input resolver linked, in command-line
    // order (as-needed objects only when a reference bound to them).
    let needed_libs: Vec<String> = if is_static {
        Vec::new()
    } else {
        needed.to_vec()
    };

    let mut dynstr = DynStrTab::new();
    let _ = dynstr.add("");
    let mut needed_offsets: Vec<u32> = Vec::new();
    for lib in &needed_libs {
        needed_offsets.push(dynstr.add(lib));
    }

    // Build dynsym entries
    let mut dynsym_entries: Vec<Elf32Sym> = Vec::new();
    dynsym_entries.push(Elf32Sym {
        name: 0,
        value: 0,
        size: 0,
        info: 0,
        other: 0,
        shndx: 0,
    });

    let mut dynsym_map: FxHashMap<String, usize> = FxHashMap::default();
    let mut dynsym_names: Vec<String> = Vec::new();

    // An import's binding describes this executable's references, not the
    // library's definition: glibc's `puts` is a weak alias of `_IO_puts`, and
    // copying that STB_WEAK would turn an ordinary call into a weak reference
    // that ld.so binds to 0 instead of reporting a missing symbol, while a
    // `__attribute__((weak))` reference must stay weak although the library
    // defines the symbol GLOBAL.  STB_WEAK iff every reference is weak.
    let mut import_has_strong_ref: FxHashMap<&str, bool> = FxHashMap::default();
    for obj in inputs {
        for sym in &obj.symbols {
            if sym.section_index == SHN_UNDEF && sym.binding != STB_LOCAL && !sym.name.is_empty() {
                *import_has_strong_ref
                    .entry(sym.name.as_str())
                    .or_insert(false) |= sym.binding != STB_WEAK;
            }
        }
    }
    let import_binding = |name: &str| match import_has_strong_ref.get(name) {
        Some(false) => STB_WEAK,
        _ => STB_GLOBAL,
    };
    // A canonical-PLT import must be found by ld.so's symbol lookup (its
    // value is what the other modules bind to), so it goes to the hashed
    // part of .dynsym below instead of this unhashed prefix.
    let is_canonical_plt = |name: &str| global_symbols.get(name).is_some_and(|s| s.canonical_plt);

    // PLT symbols (unhashed imports)
    for name in plt_symbols.iter().filter(|n| !is_canonical_plt(n)) {
        let idx = dynsym_entries.len();
        let name_off = dynstr.add(name);
        let (bind, stype) = if let Some(sym) = global_symbols.get(name) {
            (
                import_binding(name),
                if sym.sym_type != 0 {
                    sym.sym_type
                } else {
                    STT_FUNC
                },
            )
        } else {
            (STB_GLOBAL, STT_FUNC)
        };
        dynsym_entries.push(Elf32Sym {
            name: name_off,
            value: 0,
            size: 0,
            info: (bind << 4) | stype,
            other: 0,
            shndx: SHN_UNDEF,
        });
        dynsym_map.insert(name.clone(), idx);
        dynsym_names.push(name.clone());
    }

    // GOT-only symbols: only dynamic (imported) symbols go in .dynsym
    // Local GOT symbols are resolved at link time and don't need dynamic entries
    // (A function can be both a PLT and a GOT import; it gets one entry.)
    for name in got_dyn_symbols {
        if dynsym_map.contains_key(name) || is_canonical_plt(name) {
            continue;
        }
        let idx = dynsym_entries.len();
        let name_off = dynstr.add(name);
        let sym = &global_symbols[name];
        dynsym_entries.push(Elf32Sym {
            name: name_off,
            value: 0,
            size: sym.size,
            info: (import_binding(name) << 4) | sym.sym_type,
            other: 0,
            shndx: SHN_UNDEF,
        });
        dynsym_map.insert(name.clone(), idx);
        dynsym_names.push(name.clone());
    }

    let gnu_hash_symoffset = dynsym_entries.len();

    // Copy-reloc symbols (hashed: defined in this executable)
    let mut copy_syms_for_dynsym: Vec<String> = global_symbols
        .iter()
        .filter(|(_, s)| s.needs_copy && s.is_dynamic)
        .map(|(n, _)| n.clone())
        .collect();
    copy_syms_for_dynsym.sort();

    for name in &copy_syms_for_dynsym {
        let idx = dynsym_entries.len();
        let name_off = dynstr.add(name);
        let sym = &global_symbols[name];
        dynsym_entries.push(Elf32Sym {
            name: name_off,
            value: 0,
            size: sym.size,
            info: (STB_GLOBAL << 4) | STT_OBJECT,
            other: 0,
            shndx: SHN_UNDEF,
        });
        dynsym_map.insert(name.clone(), idx);
        dynsym_names.push(name.clone());
    }

    // Canonical-PLT imports (hashed: undefined, but valued -- see
    // `LinkerSymbol::canonical_plt`; the value is patched after layout).
    let mut canonical_syms_for_dynsym: Vec<String> = plt_symbols
        .iter()
        .filter(|n| is_canonical_plt(n))
        .cloned()
        .collect();
    canonical_syms_for_dynsym.sort();
    for name in &canonical_syms_for_dynsym {
        let idx = dynsym_entries.len();
        let name_off = dynstr.add(name);
        dynsym_entries.push(Elf32Sym {
            name: name_off,
            value: 0,
            size: 0,
            info: (import_binding(name) << 4) | STT_FUNC,
            other: 0,
            shndx: SHN_UNDEF,
        });
        dynsym_map.insert(name.clone(), idx);
        dynsym_names.push(name.clone());
    }

    // Textrel symbols (hashed: need dynamic R_386_32 relocs)
    let mut textrel_syms_for_dynsym: Vec<String> = global_symbols
        .iter()
        .filter(|(_, s)| s.uses_textrel && s.is_dynamic)
        .map(|(n, _)| n.clone())
        .collect();
    textrel_syms_for_dynsym.sort();

    for name in &textrel_syms_for_dynsym {
        let idx = dynsym_entries.len();
        let name_off = dynstr.add(name);
        let sym = &global_symbols[name];
        dynsym_entries.push(Elf32Sym {
            name: name_off,
            value: 0,
            size: sym.size,
            info: (sym.binding << 4) | sym.sym_type,
            other: 0,
            shndx: SHN_UNDEF,
        });
        dynsym_map.insert(name.clone(), idx);
        dynsym_names.push(name.clone());
    }

    // Definitions the executable exports (hashed): each name a linked
    // shared object references — the loader must be able to bind the DSO's
    // undefined symbol to the executable's definition, which GNU ld always
    // arranges — and, with -E/--export-dynamic, every exportable global.
    let mut export_syms_for_dynsym: Vec<String> = if is_static {
        Vec::new()
    } else {
        global_symbols
            .iter()
            .filter(|(n, s)| {
                !dynsym_map.contains_key(n.as_str())
                    && is_exportable(n, s)
                    && (opts.export_dynamic || dso_refs.contains(n.as_str()))
            })
            .map(|(n, _)| n.clone())
            .collect()
    };
    export_syms_for_dynsym.sort();
    for name in &export_syms_for_dynsym {
        let idx = dynsym_entries.len();
        let name_off = dynstr.add(name);
        let sym = &global_symbols[name];
        dynsym_entries.push(Elf32Sym {
            name: name_off,
            value: 0, // patched after layout
            size: sym.size,
            info: (sym.binding << 4) | sym.sym_type,
            other: sym.visibility,
            shndx: SHN_UNDEF, // patched after layout
        });
        dynsym_map.insert(name.clone(), idx);
        dynsym_names.push(name.clone());
    }

    // All hashed symbols = copy + textrel + exported definitions
    let mut all_hashed_syms: Vec<String> = Vec::new();
    all_hashed_syms.extend(copy_syms_for_dynsym.iter().cloned());
    all_hashed_syms.extend(canonical_syms_for_dynsym.iter().cloned());
    all_hashed_syms.extend(textrel_syms_for_dynsym.iter().cloned());
    all_hashed_syms.extend(export_syms_for_dynsym.iter().cloned());

    // Build .gnu.hash and reorder hashed dynsym entries
    let (gnu_hash_data, sorted_indices) =
        build_gnu_hash_32(&all_hashed_syms, gnu_hash_symoffset as u32);

    if !sorted_indices.is_empty() {
        let hashed_start = gnu_hash_symoffset;
        let hashed_names_start = hashed_start - 1;

        let orig_entries: Vec<Elf32Sym> = (0..sorted_indices.len())
            .map(|i| dynsym_entries[hashed_start + i].clone())
            .collect();
        let orig_names: Vec<String> = (0..sorted_indices.len())
            .map(|i| dynsym_names[hashed_names_start + i].clone())
            .collect();

        for (new_pos, &orig_idx) in sorted_indices.iter().enumerate() {
            dynsym_entries[hashed_start + new_pos] = orig_entries[orig_idx].clone();
            dynsym_names[hashed_names_start + new_pos] = orig_names[orig_idx].clone();
        }

        for (i, name) in dynsym_names[hashed_names_start..].iter().enumerate() {
            dynsym_map.insert(name.clone(), hashed_start + i);
        }
    }

    // ── Build version tables ──────────────────────────────────────────────
    let mut lib_versions: FxHashMap<String, BTreeSet<String>> = FxHashMap::default();
    for name in &dynsym_names {
        if let Some(gs) = global_symbols.get(name) {
            if gs.is_dynamic {
                if let Some(ref ver) = gs.version {
                    lib_versions
                        .entry(gs.dynlib.clone())
                        .or_default()
                        .insert(ver.clone());
                }
            }
        }
    }

    let mut ver_index_map: FxHashMap<(String, String), u16> = FxHashMap::default();
    let mut ver_idx: u16 = 2;
    let mut lib_ver_list: Vec<(String, Vec<String>)> = Vec::new();
    // Only linked (DT_NEEDED) objects get a Verneed record; a version index
    // must never name a file the loader will not open.
    let mut sorted_libs: Vec<String> = lib_versions
        .keys()
        .filter(|lib| needed_libs.contains(lib))
        .cloned()
        .collect();
    sorted_libs.sort();
    for lib in &sorted_libs {
        let vers: Vec<String> = lib_versions[lib].iter().cloned().collect();
        for v in &vers {
            ver_index_map.insert((lib.clone(), v.clone()), ver_idx);
            ver_idx += 1;
        }
        lib_ver_list.push((lib.clone(), vers));
    }

    // Rebuild dynstr with version strings
    let mut dynstr2 = DynStrTab::new();
    let _ = dynstr2.add("");
    for lib in &needed_libs {
        dynstr2.add(lib);
    }
    for name in plt_symbols {
        dynstr2.add(name);
    }
    for name in got_dyn_symbols {
        dynstr2.add(name);
    }
    for name in &all_hashed_syms {
        dynstr2.add(name);
    }
    for (_, vers) in &lib_ver_list {
        for v in vers {
            dynstr2.add(v);
        }
    }
    // Option-derived string tags (DT_SONAME, DT_RPATH/DT_RUNPATH).
    let option_tags = opts.extra_dynamic_tags(false, false, false);
    for (_, v) in &option_tags {
        if let DynValue::Str(s) = v {
            dynstr2.add(s);
        }
    }
    let dynstr_data = dynstr2.as_bytes().to_vec();

    // Rebuild offsets
    let mut needed_offsets: Vec<u32> = Vec::new();
    for lib in &needed_libs {
        needed_offsets.push(dynstr2.get_offset(lib));
    }
    for (i, entry) in dynsym_entries.iter_mut().enumerate() {
        if i == 0 {
            continue;
        }
        let name = &dynsym_names[i - 1];
        entry.name = dynstr2.get_offset(name);
    }

    // Build .gnu.version (versym)
    let mut versym_data: Vec<u8> = Vec::new();
    for (i, _) in dynsym_entries.iter().enumerate() {
        if i == 0 {
            versym_data.extend_from_slice(&0u16.to_le_bytes());
        } else {
            let sym_name = if i - 1 < dynsym_names.len() {
                &dynsym_names[i - 1]
            } else {
                ""
            };
            let gs = global_symbols.get(sym_name);
            if let Some(gs) = gs {
                if gs.is_dynamic && !gs.dynlib.is_empty() {
                    if let Some(ref ver) = gs.version {
                        let idx = ver_index_map
                            .get(&(gs.dynlib.clone(), ver.clone()))
                            .copied()
                            .unwrap_or(1);
                        versym_data.extend_from_slice(&idx.to_le_bytes());
                    } else {
                        versym_data.extend_from_slice(&1u16.to_le_bytes());
                    }
                } else {
                    // A definition of this executable (copy/exported):
                    // VER_NDX_GLOBAL, not VER_NDX_LOCAL.
                    versym_data.extend_from_slice(&1u16.to_le_bytes());
                }
            } else {
                versym_data.extend_from_slice(&0u16.to_le_bytes());
            }
        }
    }

    // Build .gnu.version_r (verneed)
    let mut verneed_data: Vec<u8> = Vec::new();
    let mut verneed_count: u32 = 0;
    for (lib_i, (lib, vers)) in lib_ver_list.iter().enumerate() {
        if !needed_libs.contains(lib) {
            continue;
        }
        let lib_name_off = dynstr2.get_offset(lib);
        let is_last_lib = lib_i == lib_ver_list.len() - 1;

        verneed_data.extend_from_slice(&1u16.to_le_bytes());
        verneed_data.extend_from_slice(&(vers.len() as u16).to_le_bytes());
        verneed_data.extend_from_slice(&lib_name_off.to_le_bytes());
        verneed_data.extend_from_slice(&16u32.to_le_bytes());
        let next_off = if is_last_lib {
            0u32
        } else {
            16 + vers.len() as u32 * 16
        };
        verneed_data.extend_from_slice(&next_off.to_le_bytes());
        verneed_count += 1;

        for (v_i, ver) in vers.iter().enumerate() {
            let ver_name_off = dynstr2.get_offset(ver);
            let v_idx = ver_index_map[&(lib.clone(), ver.clone())];
            let is_last_ver = v_i == vers.len() - 1;

            verneed_data.extend_from_slice(&linker_common::sysv_hash(ver.as_bytes()).to_le_bytes());
            verneed_data.extend_from_slice(&0u16.to_le_bytes());
            verneed_data.extend_from_slice(&v_idx.to_le_bytes());
            verneed_data.extend_from_slice(&ver_name_off.to_le_bytes());
            let vna_next: u32 = if is_last_ver { 0 } else { 16 };
            verneed_data.extend_from_slice(&vna_next.to_le_bytes());
        }
    }

    // ── Layout ────────────────────────────────────────────────────────────
    let ehdr_size: u32 = 52;
    let phdr_size: u32 = 32;
    let has_tls_sections = output_sections
        .iter()
        .any(|s| s.flags & SHF_TLS != 0 && s.flags & SHF_ALLOC != 0);

    let note_sec_idx = section_name_to_idx.get(".note").copied();
    let input_note_size = note_sec_idx
        .map(|i| output_sections[i].data.len() as u32)
        .unwrap_or(0);
    let build_id_size = if opts.build_id {
        linker_common::build_id::BUILD_ID_NOTE_SIZE as u32
    } else {
        0
    };
    let has_notes = input_note_size + build_id_size > 0;
    // The merged GNU property note (`merge_gnu_properties`), covered by
    // PT_GNU_PROPERTY as well as PT_NOTE, like GNU ld does for elf_i386.
    let property_note = super::sections::property_note_extent(inputs, section_map, note_sec_idx);
    // PT_GNU_RELRO is applied by the dynamic loader; a static executable
    // has no RELRO-protected dynamic data (and its IRELATIVE slots must stay
    // writable until the startup code has applied them).
    let use_relro = !is_static && opts.relro;
    let mut num_phdrs: u32 = 1; // PHDR
    if !is_static {
        num_phdrs += 1;
    } // INTERP
    num_phdrs += 4; // LOAD x4 (headers, text, rodata, data)
    if !is_static {
        num_phdrs += 1;
    } // DYNAMIC
    num_phdrs += 1; // GNU_STACK
    // `.eh_frame_hdr` (and so PT_GNU_EH_FRAME) exists exactly when the
    // merged `.eh_frame` holds an FDE, as in the shared-library emitter
    // and GNU ld.  An unconditional header used to be written as
    // {p_vaddr 0, p_memsz 0} without FDEs; libgcc's dl_iterate_phdr
    // unwinder dereferences p_vaddr + load bias of the PT_GNU_EH_FRAME of
    // any object whose PT_LOAD holds the pc, i.e. address 0 there.
    let eh_frame_fde_count = section_name_to_idx.get(".eh_frame").map_or(0, |&i| {
        crate::backend::linker_common::count_eh_frame_fdes(&output_sections[i].data)
    });
    if eh_frame_fde_count > 0 {
        num_phdrs += 1; // GNU_EH_FRAME
    }
    if has_tls_sections {
        num_phdrs += 1;
    }
    if has_notes {
        num_phdrs += 1; // NOTE
    }
    if property_note.is_some() {
        num_phdrs += 1; // GNU_PROPERTY
    }
    if use_relro {
        num_phdrs += 1; // GNU_RELRO
    }

    let phdrs_total_size = num_phdrs * phdr_size;

    // NUL-terminated PT_INTERP payload chosen by the caller (`link_builtin`).
    let interp_data = interp.to_vec();

    // Section layout tracking
    let mut file_offset: u32 = ehdr_size;
    let mut vaddr: u32 = BASE_ADDR + ehdr_size;

    let phdr_offset = file_offset;
    let phdr_vaddr = vaddr;
    file_offset += phdrs_total_size;
    vaddr += phdrs_total_size;

    // INTERP
    let interp_offset = file_offset;
    let interp_vaddr = vaddr;
    let interp_size = interp_data.len() as u32;
    if !is_static {
        file_offset += interp_size;
        vaddr += interp_size;
    }

    // Notes: the merged input `.note.*` records, then the build-id note,
    // covered by one PT_NOTE.  Note records are 4-byte aligned, and the
    // region follows the odd-length PT_INTERP string, so it is aligned
    // explicitly (it used to start at whatever offset the string left).
    let note_align = note_sec_idx
        .map(|i| output_sections[i].align.max(4))
        .unwrap_or(4);
    file_offset = align_up(file_offset, note_align);
    vaddr = align_up(vaddr, note_align);
    let note_offset = file_offset;
    let note_vaddr = vaddr;
    if input_note_size > 0 {
        if let Some(idx) = note_sec_idx {
            output_sections[idx].file_offset = file_offset;
            output_sections[idx].addr = vaddr;
        }
        file_offset += input_note_size;
        vaddr += input_note_size;
    }
    file_offset = align_up(file_offset, 4);
    vaddr = align_up(vaddr, 4);
    let build_id_offset = file_offset;
    file_offset += build_id_size;
    vaddr += build_id_size;
    let note_total_size = file_offset - note_offset;

    // .hash (SysV), over the final .dynsym order.
    let sysv_hash = (!is_static && opts.hash_style.wants_sysv()).then(|| {
        let names: Vec<&str> = dynsym_names.iter().map(String::as_str).collect();
        linker_common::build_sysv_hash(&names)
    });
    file_offset = align_up(file_offset, 4);
    vaddr = align_up(vaddr, 4);
    let hash_offset = file_offset;
    let hash_vaddr = vaddr;
    let hash_size = sysv_hash.as_ref().map_or(0, |h| h.size() as u32);
    file_offset += hash_size;
    vaddr += hash_size;
    let want_gnu_hash = !is_static && opts.hash_style.wants_gnu();

    // .gnu.hash
    file_offset = align_up(file_offset, 4);
    vaddr = align_up(vaddr, 4);
    let gnu_hash_offset = file_offset;
    let gnu_hash_vaddr = vaddr;
    let gnu_hash_size = gnu_hash_data.len() as u32;
    if want_gnu_hash {
        file_offset += gnu_hash_size;
        vaddr += gnu_hash_size;
    }

    // .dynsym
    let dynsym_offset = file_offset;
    let dynsym_vaddr = vaddr;
    let dynsym_entsize: u32 = 16;
    let dynsym_size = (dynsym_entries.len() as u32) * dynsym_entsize;
    if !is_static {
        file_offset += dynsym_size;
        vaddr += dynsym_size;
    }

    // .dynstr
    let dynstr_offset = file_offset;
    let dynstr_vaddr = vaddr;
    let dynstr_size = dynstr_data.len() as u32;
    if !is_static {
        file_offset += dynstr_size;
        vaddr += dynstr_size;
    }

    // .gnu.version
    let versym_offset = file_offset;
    let versym_vaddr = vaddr;
    let versym_size = versym_data.len() as u32;
    if !is_static && versym_size > 0 {
        file_offset += versym_size;
        vaddr += versym_size;
    }

    // .gnu.version_r
    file_offset = align_up(file_offset, 4);
    vaddr = align_up(vaddr, 4);
    let verneed_offset = file_offset;
    let verneed_vaddr = vaddr;
    let verneed_size = verneed_data.len() as u32;
    if !is_static && verneed_size > 0 {
        file_offset += verneed_size;
        vaddr += verneed_size;
    }

    // .rel.dyn
    file_offset = align_up(file_offset, 4);
    vaddr = align_up(vaddr, 4);
    let rel_dyn_offset = file_offset;
    let rel_dyn_vaddr = vaddr;
    let num_copy_relocs = copy_syms_for_dynsym.len();
    // Count actual R_386_32 relocations against textrel symbols
    let num_text_relocs: usize = if textrel_syms_for_dynsym.is_empty() {
        0
    } else {
        let mut count = 0usize;
        for obj in inputs {
            for sec in &obj.sections {
                for &(_, rel_type, sym_idx, _) in &sec.relocations {
                    if rel_type == R_386_32 {
                        if let Some(sym) = obj.symbols.get(sym_idx as usize) {
                            if let Some(gs) = global_symbols.get(sym.name.as_str()) {
                                if gs.uses_textrel {
                                    count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        count
    };
    let num_rel_dyn = got_dyn_symbols.len() + num_copy_relocs + num_text_relocs;
    let rel_dyn_size = (num_rel_dyn as u32) * 8;
    if !is_static {
        file_offset += rel_dyn_size;
        vaddr += rel_dyn_size;
    }

    // .rel.plt
    let rel_plt_offset = file_offset;
    let rel_plt_vaddr = vaddr;
    let rel_plt_size = (num_plt as u32) * 8;
    if !is_static {
        file_offset += rel_plt_size;
        vaddr += rel_plt_size;
    }

    let ro_headers_end = file_offset;

    // ── Segment 1 (RX): .init + .plt + .text + .fini ──
    file_offset = align_up(file_offset, PAGE_SIZE);
    vaddr = align_up(vaddr, PAGE_SIZE);
    vaddr = (vaddr & !0xfff) | (file_offset & 0xfff);
    if vaddr < BASE_ADDR + file_offset {
        vaddr += PAGE_SIZE;
    }

    let text_seg_file_start = file_offset;
    let text_seg_vaddr_start = vaddr;

    // .init
    let init_sec_idx = section_name_to_idx.get(".init").copied();
    let init_vaddr;
    let init_size;
    if let Some(idx) = init_sec_idx {
        let a = output_sections[idx].align.max(4);
        file_offset = align_up(file_offset, a);
        vaddr = align_up(vaddr, a);
        init_vaddr = vaddr;
        init_size = output_sections[idx].data.len() as u32;
        output_sections[idx].addr = vaddr;
        output_sections[idx].file_offset = file_offset;
        file_offset += init_size;
        vaddr += init_size;
    } else {
        init_vaddr = vaddr;
        init_size = 0;
    }

    // .plt
    let plt_entry_size: u32 = 16;
    let plt_header_size: u32 = if num_plt > 0 { 16 } else { 0 };
    let plt_total_size = plt_header_size + (num_plt as u32) * plt_entry_size;
    file_offset = align_up(file_offset, 16);
    vaddr = align_up(vaddr, 16);
    let plt_offset = file_offset;
    let plt_vaddr = vaddr;
    if plt_total_size > 0 {
        file_offset += plt_total_size;
        vaddr += plt_total_size;
    }

    // .text
    if let Some(idx) = section_name_to_idx.get(".text").copied() {
        let a = output_sections[idx].align.max(16);
        file_offset = align_up(file_offset, a);
        vaddr = align_up(vaddr, a);
        output_sections[idx].addr = vaddr;
        output_sections[idx].file_offset = file_offset;
        file_offset += output_sections[idx].data.len() as u32;
        vaddr += output_sections[idx].data.len() as u32;
    }

    // .fini
    let fini_sec_idx = section_name_to_idx.get(".fini").copied();
    let fini_vaddr;
    let fini_size;
    if let Some(idx) = fini_sec_idx {
        let a = output_sections[idx].align.max(4);
        file_offset = align_up(file_offset, a);
        vaddr = align_up(vaddr, a);
        fini_vaddr = vaddr;
        fini_size = output_sections[idx].data.len() as u32;
        output_sections[idx].addr = vaddr;
        output_sections[idx].file_offset = file_offset;
        file_offset += fini_size;
        vaddr += fini_size;
    } else {
        fini_vaddr = 0;
        fini_size = 0;
    }

    // Layout custom executable sections (for __start_/__stop_ symbol auto-generation)
    layout_custom_sections(
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        SHF_EXECINSTR,
    );

    // .iplt (IFUNC PLT entries for static linking)
    let iplt_entry_size: u32 = 8;
    let iplt_total_size = (num_ifunc as u32) * iplt_entry_size;
    file_offset = align_up(file_offset, 8);
    vaddr = align_up(vaddr, 8);
    let iplt_offset = file_offset;
    let iplt_vaddr = vaddr;
    if iplt_total_size > 0 {
        file_offset += iplt_total_size;
        vaddr += iplt_total_size;
    }

    let text_seg_file_end = file_offset;
    let text_seg_vaddr_end = vaddr;

    // ── Segment 2 (RO): .rodata + .eh_frame ──
    file_offset = align_up(file_offset, PAGE_SIZE);
    vaddr = align_up(vaddr, PAGE_SIZE);
    vaddr = (vaddr & !0xfff) | (file_offset & 0xfff);
    if vaddr <= text_seg_vaddr_end {
        vaddr = align_up(text_seg_vaddr_end, PAGE_SIZE) | (file_offset & 0xfff);
    }

    let rodata_seg_file_start = file_offset;
    let rodata_seg_vaddr_start = vaddr;

    if let Some(idx) = section_name_to_idx.get(".rodata").copied() {
        let a = output_sections[idx].align.max(4);
        file_offset = align_up(file_offset, a);
        vaddr = align_up(vaddr, a);
        output_sections[idx].addr = vaddr;
        output_sections[idx].file_offset = file_offset;
        file_offset += output_sections[idx].data.len() as u32;
        vaddr += output_sections[idx].data.len() as u32;
    }

    let eh_frame_sec_idx = section_name_to_idx.get(".eh_frame").copied();
    if let Some(idx) = eh_frame_sec_idx {
        let a = output_sections[idx].align.max(4);
        file_offset = align_up(file_offset, a);
        vaddr = align_up(vaddr, a);
        output_sections[idx].addr = vaddr;
        output_sections[idx].file_offset = file_offset;
        file_offset += output_sections[idx].data.len() as u32;
        vaddr += output_sections[idx].data.len() as u32;
    }

    // Build .eh_frame_hdr: count FDEs and reserve space after .eh_frame
    let mut eh_frame_hdr_vaddr = 0u32;
    let mut eh_frame_hdr_offset = 0u32;
    let mut eh_frame_hdr_size = 0u32;
    if eh_frame_sec_idx.is_some() {
        let fde_count = eh_frame_fde_count;
        if fde_count > 0 {
            eh_frame_hdr_size = (12 + 8 * fde_count) as u32;
            file_offset = align_up(file_offset, 4);
            vaddr = align_up(vaddr, 4);
            eh_frame_hdr_offset = file_offset;
            eh_frame_hdr_vaddr = vaddr;
            file_offset += eh_frame_hdr_size;
            vaddr += eh_frame_hdr_size;
        }
    }

    // Layout custom read-only sections (for __start_/__stop_ symbol auto-generation)
    layout_custom_sections(
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        0,
    );

    let rodata_seg_file_end = file_offset;
    let rodata_seg_vaddr_end = vaddr;

    // ── Segment 3 (RW): .init_array + .fini_array + .dynamic + .got + .got.plt + .data + .bss ──
    file_offset = align_up(file_offset, PAGE_SIZE);
    vaddr = align_up(vaddr, PAGE_SIZE);
    vaddr = (vaddr & !0xfff) | (file_offset & 0xfff);
    if vaddr <= rodata_seg_vaddr_end {
        vaddr = align_up(rodata_seg_vaddr_end, PAGE_SIZE) | (file_offset & 0xfff);
    }

    let data_seg_file_start = file_offset;
    let data_seg_vaddr_start = vaddr;

    // Synthesized RW tables, sized before placement.
    let got_reserved: usize = 1;
    let got_non_plt_entries = got_dyn_symbols.len() + got_local_symbols.len();
    let got_entry_size: u32 = 4;
    let got_size = (got_reserved + got_non_plt_entries) as u32 * got_entry_size;
    let needs_got_section = !is_static || got_non_plt_entries > 0 || num_plt > 0;
    let gotplt_reserved: u32 = 3;
    let gotplt_size = (gotplt_reserved + num_plt as u32) * 4;
    let has_gotplt = !is_static && num_plt > 0;

    // `.dynamic`: which entries exist is decided here; their values are
    // filled in after layout (see `DynamicDesc`).
    let textrel = num_text_relocs > 0;
    if textrel && opts.z_text {
        return Err(
            "read-only segment has dynamic relocations (text relocations against weak shared-library data) and -z text was given"
                .to_string(),
        );
    }
    let sec_len = |name: &str| {
        section_name_to_idx
            .get(name)
            .map_or(0, |&i| output_sections[i].data.len() as u32)
    };
    let (option_strings, option_numeric) =
        option_entries(&opts.extra_dynamic_tags(textrel, false, false), |s| {
            dynstr2.get_offset(s)
        });
    let mut dyn_desc = DynamicDesc {
        needed: needed_offsets.clone(),
        strings: option_strings,
        init: (init_vaddr != 0 && init_size > 0).then_some(init_vaddr),
        fini: (fini_vaddr != 0 && fini_size > 0).then_some(fini_vaddr),
        preinit_array: (sec_len(".preinit_array") > 0).then_some((0, 0)),
        init_array: (sec_len(".init_array") > 0).then_some((0, 0)),
        fini_array: (sec_len(".fini_array") > 0).then_some((0, 0)),
        hash: sysv_hash.as_ref().map(|_| hash_vaddr),
        gnu_hash: want_gnu_hash.then_some(gnu_hash_vaddr),
        strtab: dynstr_vaddr,
        symtab: dynsym_vaddr,
        strsz: dynstr_size,
        debug: true,
        plt: (num_plt > 0).then_some((0, rel_plt_vaddr, rel_plt_size)),
        rel: (num_rel_dyn > 0).then_some((rel_dyn_vaddr, rel_dyn_size)),
        relcount: None,
        verneed: (verneed_size > 0).then_some((versym_vaddr, verneed_vaddr, verneed_count)),
        numeric: option_numeric,
    };
    let dynamic_size = if is_static { 0 } else { dyn_desc.byte_size() };

    // PT_GNU_RELRO covers the tables only the loader writes: the
    // constructor arrays, .data.rel.ro, .dynamic, .got and — with -z now —
    // .got.plt.  The loader rounds the region's END down to a page, so the
    // segment start is shifted within its first page until the region ends
    // exactly on a page boundary (GNU ld does the same); otherwise its last
    // page, and for a small image the whole region, stayed writable.  The
    // shift keeps every section's alignment (it is a multiple of the largest
    // alignment in the region); `relro_end` below is measured, not assumed.
    const RELRO_SECTIONS: [&str; 4] = [
        ".preinit_array",
        ".init_array",
        ".fini_array",
        ".data.rel.ro",
    ];
    if use_relro {
        let mut len = 0u32;
        let mut max_align = 4u32;
        for name in RELRO_SECTIONS {
            if let Some(&i) = section_name_to_idx.get(name) {
                let a = output_sections[i].align.max(4);
                max_align = max_align.max(a);
                len = align_up(len, a) + output_sections[i].data.len() as u32;
            }
        }
        len = align_up(len, 4) + dynamic_size;
        if needs_got_section {
            len += got_size;
        }
        if opts.bind_now && has_gotplt {
            len += gotplt_size;
        }
        let pad = (PAGE_SIZE - len % PAGE_SIZE) % PAGE_SIZE;
        let pad = pad - pad % max_align;
        file_offset += pad;
        vaddr += pad;
    }
    let relro_offset = file_offset;
    let relro_vaddr = vaddr;

    let (preinit_array_vaddr, preinit_array_size) = layout_section(
        ".preinit_array",
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        4,
    );
    let (init_array_vaddr, init_array_size) = layout_section(
        ".init_array",
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        4,
    );
    let (fini_array_vaddr, fini_array_size) = layout_section(
        ".fini_array",
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        4,
    );
    let _ = layout_section(
        ".data.rel.ro",
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        4,
    );

    // .dynamic
    file_offset = align_up(file_offset, 4);
    vaddr = align_up(vaddr, 4);
    let dynamic_offset = file_offset;
    let dynamic_vaddr = vaddr;
    file_offset += dynamic_size;
    vaddr += dynamic_size;

    // .got
    let got_offset = file_offset;
    let got_vaddr = vaddr;
    if needs_got_section {
        file_offset += got_size;
        vaddr += got_size;
    }
    let mut relro_end = vaddr;

    // .got.plt
    let gotplt_offset = file_offset;
    let gotplt_vaddr = vaddr;
    if has_gotplt {
        file_offset += gotplt_size;
        vaddr += gotplt_size;
    }
    if opts.bind_now {
        relro_end = vaddr;
    }

    // IFUNC GOT
    let ifunc_got_offset = file_offset;
    let ifunc_got_vaddr = vaddr;
    let ifunc_got_size = (num_ifunc as u32) * 4;
    if ifunc_got_size > 0 {
        file_offset += ifunc_got_size;
        vaddr += ifunc_got_size;
    }

    // .rel.iplt
    let rel_iplt_offset = file_offset;
    let rel_iplt_vaddr = vaddr;
    let rel_iplt_size = (num_ifunc as u32) * 8;
    if rel_iplt_size > 0 {
        file_offset += rel_iplt_size;
        vaddr += rel_iplt_size;
    }

    // Custom writable sections (`__start_`/`__stop_` anchors); after the
    // RELRO region, since the program may write them.
    layout_custom_sections(
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        SHF_WRITE,
    );

    // .data
    if let Some(idx) = section_name_to_idx.get(".data").copied() {
        let a = output_sections[idx].align.max(4);
        file_offset = align_up(file_offset, a);
        vaddr = align_up(vaddr, a);
        output_sections[idx].addr = vaddr;
        output_sections[idx].file_offset = file_offset;
        file_offset += output_sections[idx].data.len() as u32;
        vaddr += output_sections[idx].data.len() as u32;
    }

    // TLS sections
    let (tls_addr, tls_file_offset, tls_file_size, tls_mem_size, tls_align) = layout_tls(
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
    );
    let has_tls = tls_addr != 0;

    let data_seg_file_end = file_offset;

    // .bss
    let bss_vaddr;
    if let Some(idx) = section_name_to_idx.get(".bss").copied() {
        let a = output_sections[idx].align.max(4);
        vaddr = align_up(vaddr, a);
        bss_vaddr = vaddr;
        output_sections[idx].addr = vaddr;
        output_sections[idx].file_offset = file_offset;
        vaddr += output_sections[idx].data.len() as u32;
    } else {
        bss_vaddr = vaddr;
    }

    // Allocate BSS space for copy relocations
    let mut copy_reloc_symbols: Vec<String> = global_symbols
        .iter()
        .filter(|(_, s)| s.needs_copy && s.is_dynamic)
        .map(|(n, _)| n.clone())
        .collect();
    copy_reloc_symbols.sort();

    for name in &copy_reloc_symbols {
        if let Some(sym) = global_symbols.get_mut(name) {
            let al = if sym.size >= 4 { 4 } else { 1 };
            vaddr = align_up(vaddr, al);
            sym.copy_addr = vaddr;
            sym.address = vaddr;
            vaddr += sym.size.max(4);
        }
    }

    let data_seg_vaddr_end = vaddr;

    let got_base = if num_plt > 0 { gotplt_vaddr } else { got_vaddr };

    // ── Assign symbol addresses ──────────────────────────────────────────
    assign_symbol_addresses(
        global_symbols,
        output_sections,
        got_base,
        plt_vaddr,
        plt_header_size,
        plt_entry_size,
        bss_vaddr,
        data_seg_vaddr_end,
        data_seg_vaddr_start,
        text_seg_vaddr_end,
        dynamic_vaddr,
        is_static,
        (preinit_array_vaddr, preinit_array_size),
        init_array_vaddr,
        init_array_size,
        fini_array_vaddr,
        fini_array_size,
        rel_iplt_vaddr,
        rel_iplt_size,
        pending_defsyms,
    )?;

    // Override IFUNC symbol addresses to point to IPLT entries
    let mut ifunc_resolver_addrs: Vec<u32> = Vec::new();
    for (i, name) in ifunc_symbols.iter().enumerate() {
        if let Some(sym) = global_symbols.get_mut(name) {
            ifunc_resolver_addrs.push(sym.address);
            sym.address = iplt_vaddr + (i as u32) * iplt_entry_size;
        }
    }

    // ── Build IPLT data ──────────────────────────────────────────────────
    let mut iplt_data: Vec<u8> = Vec::new();
    for i in 0..num_ifunc {
        let got_entry_addr = ifunc_got_vaddr + (i as u32) * 4;
        iplt_data.push(0xff);
        iplt_data.push(0x25);
        iplt_data.extend_from_slice(&got_entry_addr.to_le_bytes());
        iplt_data.push(0x66);
        iplt_data.push(0x90);
    }

    let mut ifunc_got_data: Vec<u8> = Vec::new();
    for &resolver_addr in &ifunc_resolver_addrs {
        ifunc_got_data.extend_from_slice(&resolver_addr.to_le_bytes());
    }

    let mut rel_iplt_data: Vec<u8> = Vec::new();
    for i in 0..num_ifunc {
        let r_offset = ifunc_got_vaddr + (i as u32) * 4;
        rel_iplt_data.extend_from_slice(&r_offset.to_le_bytes());
        rel_iplt_data.extend_from_slice(&R_386_IRELATIVE.to_le_bytes());
    }

    // ── Build PLT ────────────────────────────────────────────────────────
    let plt_data = build_plt(
        num_plt,
        plt_vaddr,
        plt_header_size,
        plt_entry_size,
        gotplt_vaddr,
        gotplt_reserved,
        PltAddressing::Absolute,
    );

    // ── Apply relocations ────────────────────────────────────────────────
    let text_relocs;
    {
        let mut reloc_ctx = RelocContext {
            global_symbols,
            output_sections,
            section_map,
            got_base,
            got_vaddr,
            gotplt_vaddr,
            got_reserved,
            gotplt_reserved,
            plt_vaddr,
            plt_header_size,
            plt_entry_size,
            num_plt,
            tls_addr,
            tls_mem_size,
            has_tls,
            tls_relaxed_call_slots: Default::default(),
        };
        text_relocs = reloc::apply_relocations(inputs, &mut reloc_ctx)?;
    }

    // Build .eh_frame_hdr from relocated .eh_frame data
    let eh_frame_hdr_data = if eh_frame_hdr_size > 0 {
        if let Some(idx) = eh_frame_sec_idx {
            let sec = &output_sections[idx];
            crate::backend::linker_common::build_eh_frame_hdr(
                &sec.data,
                sec.addr as u64,
                eh_frame_hdr_vaddr as u64,
                false, // 32-bit
            )
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    // ── Build GOT data ───────────────────────────────────────────────────
    let mut got_data: Vec<u8> = Vec::new();
    if needs_got_section {
        got_data.extend_from_slice(&(if is_static { 0u32 } else { dynamic_vaddr }).to_le_bytes());
        // Dynamic GOT symbols first, filled by the dynamic linker (GLOB_DAT,
        // or TLS_TPOFF for a DSO TLS variable); the implicit addend is 0.
        got_data.resize(got_data.len() + 4 * got_dyn_symbols.len(), 0);
        // Local GOT symbols (filled at link time with resolved addresses).
        //
        // Absolute symbols (`--defsym k=100`, input SHN_ABS definitions,
        // linker language symbols like `_end`) resolve to their VALUE in
        // the slot, exactly as GNU ld does: measured on `ld -m elf_i386
        // --defsym k=100` over an R_386_GOT32 reference, the slot holds
        // 0x64 with NO dynamic relocation — even in a dynamic executable.
        // Writing the slot's own vaddr here instead (as a briefly-landed
        // revision did) turns `movl (%eax),%eax` after the GOT load from
        // "read through the defined address" into "read the slot itself":
        // a silent mislink that also disagrees with GNU ld on every ABS
        // input.  A correspondent `movl $k`/`R_386_32` path resolves ABS
        // the same way (see `resolve_symbols`).
        for name in got_local_symbols {
            if let Some(gs) = global_symbols.get(name) {
                if has_tls && gs.sym_type == STT_TLS {
                    let tpoff = gs.address as i32 - tls_addr as i32 - tls_mem_size as i32;
                    got_data.extend_from_slice(&(tpoff as u32).to_le_bytes());
                } else {
                    got_data.extend_from_slice(&gs.address.to_le_bytes());
                }
            } else {
                got_data.extend_from_slice(&0u32.to_le_bytes());
            }
        }
    }

    // GOT.PLT data
    let mut gotplt_data: Vec<u8> = Vec::new();
    if !is_static && num_plt > 0 {
        gotplt_data.extend_from_slice(&dynamic_vaddr.to_le_bytes());
        gotplt_data.extend_from_slice(&0u32.to_le_bytes());
        gotplt_data.extend_from_slice(&0u32.to_le_bytes());
        for i in 0..num_plt {
            let lazy_addr = plt_vaddr + plt_header_size + (i as u32) * plt_entry_size + 6;
            gotplt_data.extend_from_slice(&lazy_addr.to_le_bytes());
        }
    }

    // .rel.plt data
    let mut rel_plt_data: Vec<u8> = Vec::new();
    for (i, name) in plt_symbols.iter().enumerate() {
        let gotplt_entry_addr = gotplt_vaddr + (gotplt_reserved + i as u32) * 4;
        let dynsym_idx = dynsym_map[name] as u32;
        let r_info = (dynsym_idx << 8) | 7; // R_386_JMP_SLOT
        rel_plt_data.extend_from_slice(&gotplt_entry_addr.to_le_bytes());
        rel_plt_data.extend_from_slice(&r_info.to_le_bytes());
    }

    // .rel.dyn data: one GLOB_DAT per dynamic GOT symbol; a DSO TLS
    // variable's slot takes its thread-pointer offset (TLS_TPOFF).
    let mut rel_dyn_data: Vec<u8> = Vec::new();
    for (i, name) in got_dyn_symbols.iter().enumerate() {
        let got_entry_addr = got_vaddr + (got_reserved as u32 + i as u32) * 4;
        let dynsym_idx = dynsym_map[name] as u32;
        let r_type = match global_symbols.get(name) {
            Some(gs) if gs.sym_type == STT_TLS => R_386_TLS_TPOFF,
            _ => R_386_GLOB_DAT,
        };
        let r_info = (dynsym_idx << 8) | r_type;
        rel_dyn_data.extend_from_slice(&got_entry_addr.to_le_bytes());
        rel_dyn_data.extend_from_slice(&r_info.to_le_bytes());
    }
    for name in &copy_reloc_symbols {
        if let Some(gs) = global_symbols.get(name) {
            if let Some(&dynsym_idx) = dynsym_map.get(name) {
                let r_info = ((dynsym_idx as u32) << 8) | 5; // R_386_COPY
                rel_dyn_data.extend_from_slice(&gs.copy_addr.to_le_bytes());
                rel_dyn_data.extend_from_slice(&r_info.to_le_bytes());
            }
        }
    }
    // Text relocations for WEAK dynamic data symbols (R_386_32)
    for (addr, name) in &text_relocs {
        if let Some(&dynsym_idx) = dynsym_map.get(name) {
            let r_info = ((dynsym_idx as u32) << 8) | R_386_32;
            rel_dyn_data.extend_from_slice(&addr.to_le_bytes());
            rel_dyn_data.extend_from_slice(&r_info.to_le_bytes());
        }
    }

    // .dynamic data
    let mut dynamic_data: Vec<u8> = Vec::new();
    if !is_static {
        if let Some(v) = dyn_desc.preinit_array.as_mut() {
            *v = (preinit_array_vaddr, preinit_array_size);
        }
        if let Some(v) = dyn_desc.init_array.as_mut() {
            *v = (init_array_vaddr, init_array_size);
        }
        if let Some(v) = dyn_desc.fini_array.as_mut() {
            *v = (fini_array_vaddr, fini_array_size);
        }
        if let Some(v) = dyn_desc.plt.as_mut() {
            v.0 = gotplt_vaddr;
        }
        dynamic_data = dyn_desc.serialize(dynamic_size)?;
    }
    // Entry point: `-e SYMBOL` (or a hex address, as GNU ld), else
    // `_start`; when neither resolves, GNU ld's fallback — warn and use the
    // start of .text.
    let text_start = section_name_to_idx
        .get(".text")
        .map_or(text_seg_vaddr_start, |&i| output_sections[i].addr);
    let entry_point = resolve_entry_point(opts.entry.as_deref(), global_symbols, text_start);

    // Exported definitions get their final values.  `st_shndx` only has to
    // be "defined" for the loader (this image carries no section headers);
    // a TLS symbol's value is its offset in the PT_TLS block.
    for name in &export_syms_for_dynsym {
        if let (Some(sym), Some(&idx)) = (global_symbols.get(name), dynsym_map.get(name)) {
            dynsym_entries[idx].value = if sym.sym_type == STT_TLS {
                sym.address.wrapping_sub(tls_addr)
            } else {
                sym.address
            };
            dynsym_entries[idx].shndx = 1;
        }
    }

    // Canonical PLT: the undefined symbol's value is its PLT entry (the
    // address `assign_symbol_addresses` gave it); `st_shndx` stays UND, which
    // is what keeps PLT-class lookups -- the JUMP_SLOT of our own PLT --
    // binding to the library rather than looping back to the entry.
    for name in &canonical_syms_for_dynsym {
        if let (Some(sym), Some(&idx)) = (global_symbols.get(name), dynsym_map.get(name)) {
            dynsym_entries[idx].value = sym.address;
        }
    }

    // Patch dynsym for copy-reloc symbols
    for name in &copy_syms_for_dynsym {
        if let Some(sym) = global_symbols.get(name) {
            if let Some(&idx) = dynsym_map.get(name) {
                dynsym_entries[idx].value = sym.copy_addr;
                dynsym_entries[idx].shndx = 1;
            }
        }
    }

    // Serialize dynsym
    let mut dynsym_data: Vec<u8> = Vec::new();
    for sym in &dynsym_entries {
        dynsym_data.extend_from_slice(&sym.name.to_le_bytes());
        dynsym_data.extend_from_slice(&sym.value.to_le_bytes());
        dynsym_data.extend_from_slice(&sym.size.to_le_bytes());
        dynsym_data.push(sym.info);
        dynsym_data.push(sym.other);
        dynsym_data.extend_from_slice(&sym.shndx.to_le_bytes());
    }

    // ── Write ELF file ───────────────────────────────────────────────────
    let total_file_size = file_offset as usize;
    let mut output = vec![0u8; total_file_size];

    // ELF header
    write_elf_header(&mut output, entry_point, ehdr_size, num_phdrs);

    // Program headers
    let mut phdr_pos = phdr_offset as usize;
    let write_ph = |output: &mut Vec<u8>,
                    pos: &mut usize,
                    p_type: u32,
                    p_off: u32,
                    p_va: u32,
                    p_filesz: u32,
                    p_memsz: u32,
                    p_flags: u32,
                    p_align: u32| {
        output[*pos..*pos + 4].copy_from_slice(&p_type.to_le_bytes());
        output[*pos + 4..*pos + 8].copy_from_slice(&p_off.to_le_bytes());
        output[*pos + 8..*pos + 12].copy_from_slice(&p_va.to_le_bytes());
        output[*pos + 12..*pos + 16].copy_from_slice(&p_va.to_le_bytes());
        output[*pos + 16..*pos + 20].copy_from_slice(&p_filesz.to_le_bytes());
        output[*pos + 20..*pos + 24].copy_from_slice(&p_memsz.to_le_bytes());
        output[*pos + 24..*pos + 28].copy_from_slice(&p_flags.to_le_bytes());
        output[*pos + 28..*pos + 32].copy_from_slice(&p_align.to_le_bytes());
        *pos += phdr_size as usize;
    };

    write_ph(
        &mut output,
        &mut phdr_pos,
        PT_PHDR,
        phdr_offset,
        phdr_vaddr,
        phdrs_total_size,
        phdrs_total_size,
        PF_R,
        4,
    );
    if !is_static {
        write_ph(
            &mut output,
            &mut phdr_pos,
            PT_INTERP,
            interp_offset,
            interp_vaddr,
            interp_size,
            interp_size,
            PF_R,
            1,
        );
    }
    write_ph(
        &mut output,
        &mut phdr_pos,
        PT_LOAD,
        0,
        BASE_ADDR,
        ro_headers_end,
        ro_headers_end,
        PF_R,
        PAGE_SIZE,
    );
    write_ph(
        &mut output,
        &mut phdr_pos,
        PT_LOAD,
        text_seg_file_start,
        text_seg_vaddr_start,
        text_seg_file_end - text_seg_file_start,
        text_seg_vaddr_end - text_seg_vaddr_start,
        PF_R | PF_X,
        PAGE_SIZE,
    );
    if rodata_seg_file_end - rodata_seg_file_start > 0 {
        write_ph(
            &mut output,
            &mut phdr_pos,
            PT_LOAD,
            rodata_seg_file_start,
            rodata_seg_vaddr_start,
            rodata_seg_file_end - rodata_seg_file_start,
            rodata_seg_vaddr_end - rodata_seg_vaddr_start,
            PF_R,
            PAGE_SIZE,
        );
    }
    write_ph(
        &mut output,
        &mut phdr_pos,
        PT_LOAD,
        data_seg_file_start,
        data_seg_vaddr_start,
        data_seg_file_end - data_seg_file_start,
        data_seg_vaddr_end - data_seg_vaddr_start,
        PF_R | PF_W,
        PAGE_SIZE,
    );
    if !is_static {
        write_ph(
            &mut output,
            &mut phdr_pos,
            PT_DYNAMIC,
            dynamic_offset,
            dynamic_vaddr,
            dynamic_data.len() as u32,
            dynamic_data.len() as u32,
            PF_R | PF_W,
            4,
        );
    }
    if has_notes {
        write_ph(
            &mut output,
            &mut phdr_pos,
            PT_NOTE,
            note_offset,
            note_vaddr,
            note_total_size,
            note_total_size,
            PF_R,
            note_align,
        );
    }
    if let Some((off, size)) = property_note {
        write_ph(
            &mut output,
            &mut phdr_pos,
            PT_GNU_PROPERTY,
            note_offset + off,
            note_vaddr + off,
            size,
            size,
            PF_R,
            4,
        );
    }
    if use_relro {
        write_ph(
            &mut output,
            &mut phdr_pos,
            PT_GNU_RELRO,
            relro_offset,
            relro_vaddr,
            relro_end - relro_vaddr,
            relro_end - relro_vaddr,
            PF_R,
            1,
        );
    }
    write_ph(
        &mut output,
        &mut phdr_pos,
        PT_GNU_STACK,
        0,
        0,
        0,
        0,
        stack_flags,
        0x10,
    );
    if eh_frame_hdr_size > 0 {
        write_ph(
            &mut output,
            &mut phdr_pos,
            PT_GNU_EH_FRAME,
            eh_frame_hdr_offset,
            eh_frame_hdr_vaddr,
            eh_frame_hdr_size,
            eh_frame_hdr_size,
            PF_R,
            4,
        );
    }
    if has_tls {
        write_ph(
            &mut output,
            &mut phdr_pos,
            PT_TLS,
            tls_file_offset,
            tls_addr,
            tls_file_size,
            tls_mem_size,
            PF_R,
            tls_align,
        );
    }

    // Write section data
    let write_data = |output: &mut Vec<u8>, offset: u32, data: &[u8]| {
        if !data.is_empty() {
            let off = offset as usize;
            output[off..off + data.len()].copy_from_slice(data);
        }
    };

    if !is_static {
        write_data(&mut output, interp_offset, &interp_data);
        if want_gnu_hash {
            write_data(&mut output, gnu_hash_offset, &gnu_hash_data);
        }
        if let Some(h) = &sysv_hash {
            linker_common::write_sysv_hash(&mut output, hash_offset as usize, h);
        }
        write_data(&mut output, dynsym_offset, &dynsym_data);
        write_data(&mut output, dynstr_offset, &dynstr_data);
        if !versym_data.is_empty() {
            write_data(&mut output, versym_offset, &versym_data);
        }
        if !verneed_data.is_empty() {
            write_data(&mut output, verneed_offset, &verneed_data);
        }
        if !rel_dyn_data.is_empty() {
            write_data(&mut output, rel_dyn_offset, &rel_dyn_data);
        }
        if !rel_plt_data.is_empty() {
            write_data(&mut output, rel_plt_offset, &rel_plt_data);
        }
        write_data(&mut output, dynamic_offset, &dynamic_data);
        if !gotplt_data.is_empty() {
            write_data(&mut output, gotplt_offset, &gotplt_data);
        }
    }
    write_data(&mut output, plt_offset, &plt_data);
    write_data(&mut output, got_offset, &got_data);
    write_data(&mut output, iplt_offset, &iplt_data);
    write_data(&mut output, ifunc_got_offset, &ifunc_got_data);
    write_data(&mut output, rel_iplt_offset, &rel_iplt_data);

    // Write .eh_frame_hdr
    if !eh_frame_hdr_data.is_empty() && eh_frame_hdr_offset > 0 {
        write_data(&mut output, eh_frame_hdr_offset, &eh_frame_hdr_data);
    }

    // Write all output sections
    check_sections_placed(output_sections, "executable")?;
    for sec in output_sections.iter() {
        if sec.sh_type == SHT_NOBITS || sec.data.is_empty() {
            continue;
        }
        let off = sec.file_offset as usize;
        let end = off + sec.data.len();
        if end <= output.len() {
            output[off..end].copy_from_slice(&sec.data);
        }
    }

    // Build-id last: the digest covers the complete image.
    if build_id_size > 0 {
        linker_common::build_id::write_build_id_skeleton(&mut output, build_id_offset as usize);
        linker_common::build_id::patch_build_id(&mut output, build_id_offset as usize);
    }

    // Write to file
    std::fs::write(output_path, &output).map_err(|e| format!("failed to write output: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(output_path, std::fs::Permissions::from_mode(0o755));
    }

    Ok(())
}

// ── Helpers for emit_executable ──────────────────────────────────────────────

/// Placement invariant for the ELF32 emitters: every allocatable output
/// section that carries bytes must have been assigned an address by the
/// layout, or the section write loop copies it to file offset 0 — on top of
/// the ELF header — and the link "succeeds" with a file that is not ELF at
/// all (a `.note` section the shared-library layout never placed did exactly
/// that).  Offset/address 0 is always the ELF header, so it can never be a
/// real placement.  Failing loudly here turns any future layout omission
/// into a diagnosable link error instead of silent corruption.
pub(super) fn check_sections_placed(
    output_sections: &[OutputSection],
    what: &str,
) -> Result<(), String> {
    for sec in output_sections {
        if sec.flags & SHF_ALLOC != 0
            && !sec.data.is_empty()
            && sec.addr == 0
            && sec.file_offset == 0
        {
            return Err(format!(
                "i686 {what} layout: allocatable section '{}' ({} bytes) was never placed",
                sec.name,
                sec.data.len()
            ));
        }
    }
    Ok(())
}

pub(super) fn layout_section(
    name: &str,
    section_name_to_idx: &FxHashMap<String, usize>,
    output_sections: &mut [OutputSection],
    file_offset: &mut u32,
    vaddr: &mut u32,
    min_align: u32,
) -> (u32, u32) {
    if let Some(idx) = section_name_to_idx.get(name).copied() {
        let a = output_sections[idx].align.max(min_align);
        *file_offset = align_up(*file_offset, a);
        *vaddr = align_up(*vaddr, a);
        let sec_vaddr = *vaddr;
        let sec_size = output_sections[idx].data.len() as u32;
        output_sections[idx].addr = *vaddr;
        output_sections[idx].file_offset = *file_offset;
        *file_offset += sec_size;
        *vaddr += sec_size;
        (sec_vaddr, sec_size)
    } else {
        (0, 0)
    }
}

/// Layout custom sections with a given flag requirement.
///
/// Custom sections are those not in the standard set (.text, .rodata, .data, etc.)
/// that have the SHF_ALLOC flag plus the specified additional flag. This is needed
/// for `__start_<section>` / `__stop_<section>` symbol auto-generation and for
/// sections placed via `__attribute__((section("name")))`.
///
/// `required_flag` selects which type:
///   - SHF_EXECINSTR: custom executable sections (placed in text segment)
///   - 0: custom read-only sections (placed in rodata segment, no write/exec)
///   - SHF_WRITE: custom writable sections (placed in data segment)
pub(super) fn layout_custom_sections(
    section_name_to_idx: &FxHashMap<String, usize>,
    output_sections: &mut [OutputSection],
    file_offset: &mut u32,
    vaddr: &mut u32,
    required_flag: u32,
) {
    let standard_sections: &[&str] = &[
        ".text",
        ".rodata",
        ".data",
        ".bss",
        ".init",
        ".fini",
        ".preinit_array",
        ".init_array",
        ".fini_array",
        ".data.rel.ro",
        ".eh_frame",
        ".note",
        ".tdata",
        ".tbss",
        ".tm_clone_table",
    ];
    let mut custom: Vec<String> = section_name_to_idx
        .keys()
        .filter(|name| {
            if standard_sections.contains(&name.as_str()) {
                return false;
            }
            let idx = match section_name_to_idx.get(name.as_str()) {
                Some(&i) => i,
                None => return false,
            };
            let sec = &output_sections[idx];
            if sec.flags & SHF_ALLOC == 0 {
                return false;
            }
            // Classify by flags: 0 = read-only (no write, no exec),
            // SHF_EXECINSTR = executable, SHF_WRITE = writable (but not executable,
            // since writable+executable sections go in the text segment).
            match required_flag {
                0 => sec.flags & SHF_WRITE == 0 && sec.flags & SHF_EXECINSTR == 0,
                f => sec.flags & f != 0 && (f != SHF_WRITE || sec.flags & SHF_EXECINSTR == 0),
            }
        })
        .cloned()
        .collect();
    custom.sort(); // Deterministic order
    for name in &custom {
        layout_section(
            name,
            section_name_to_idx,
            output_sections,
            file_offset,
            vaddr,
            4,
        );
    }
}

pub(super) fn layout_tls(
    section_name_to_idx: &FxHashMap<String, usize>,
    output_sections: &mut [OutputSection],
    file_offset: &mut u32,
    vaddr: &mut u32,
) -> (u32, u32, u32, u32, u32) {
    let tdata = section_name_to_idx.get(".tdata").copied();
    let tbss = section_name_to_idx.get(".tbss").copied();
    let Some(tls_align) = [tdata, tbss]
        .into_iter()
        .flatten()
        .map(|i| output_sections[i].align.max(4))
        .max()
    else {
        return (0, 0, 0, 0, 1);
    };
    // PT_TLS starts at a multiple of its p_align: the loader places the
    // block at thread-pointer offset `roundup(p_memsz, p_align)` only then
    // (otherwise glibc also accounts for `p_vaddr % p_align`, the "first
    // byte offset"), and that is the offset the link-time TP-relative
    // values (`ntpoff`) assume.  Aligning only to `.tdata`'s own alignment
    // shifted every variable when `.tbss` was more aligned.
    *file_offset = align_up(*file_offset, tls_align);
    *vaddr = align_up(*vaddr, tls_align);
    let tls_addr = *vaddr;
    let tls_file_offset = *file_offset;
    let mut tls_mem_size = 0u32;
    if let Some(idx) = tdata {
        output_sections[idx].addr = tls_addr;
        output_sections[idx].file_offset = tls_file_offset;
        let sz = output_sections[idx].data.len() as u32;
        tls_mem_size = sz;
        *file_offset += sz;
        *vaddr += sz;
    }
    let tls_file_size = tls_mem_size;
    if let Some(idx) = tbss {
        // NOBITS: occupies TLS-image memory only, not the enclosing
        // segment's address space.
        tls_mem_size = align_up(tls_mem_size, output_sections[idx].align.max(4));
        output_sections[idx].addr = tls_addr + tls_mem_size;
        output_sections[idx].file_offset = *file_offset;
        tls_mem_size += output_sections[idx].data.len() as u32;
    }
    let tls_mem_size = align_up(tls_mem_size, tls_align);
    (
        tls_addr,
        tls_file_offset,
        tls_file_size,
        tls_mem_size,
        tls_align,
    )
}

/// Whether any input marks its `.note.GNU-stack` executable (GNU C nested
/// function trampolines); binutils ORs the markers into PT_GNU_STACK.
pub(super) fn inputs_want_exec_stack(inputs: &[InputObject]) -> bool {
    inputs.iter().any(|obj| {
        obj.sections
            .iter()
            .any(|s| s.name == ".note.GNU-stack" && (s.flags & SHF_EXECINSTR) != 0)
    })
}

/// A definition that may appear in the executable's `.dynsym`.
fn is_exportable(name: &str, s: &LinkerSymbol) -> bool {
    s.is_defined
        && !s.is_dynamic
        && !name.is_empty()
        && (s.binding == STB_GLOBAL || s.binding == STB_WEAK)
        && (s.visibility == STV_DEFAULT || s.visibility == STV_PROTECTED)
        && s.sym_type != STT_SECTION
        && s.sym_type != STT_FILE
}

/// GNU ld's entry-point rule (ldlang.c `lang_end`): the named symbol if it
/// is defined in the output, else the name read as a hexadecimal address,
/// else a warning and the start of `.text`.
pub(super) fn resolve_entry_point(
    entry: Option<&str>,
    global_symbols: &FxHashMap<String, LinkerSymbol>,
    text_start: u32,
) -> u32 {
    let name = entry.unwrap_or("_start");
    if let Some(s) = global_symbols.get(name) {
        if s.is_defined && !s.is_dynamic {
            return s.address;
        }
    }
    if let Some(e) = entry {
        let digits = e
            .strip_prefix("0x")
            .or_else(|| e.strip_prefix("0X"))
            .unwrap_or(e);
        if let Ok(v) = u32::from_str_radix(digits, 16) {
            return v;
        }
    }
    eprintln!("lccc-ld: warning: cannot find entry symbol {name}; defaulting to {text_start:08x}");
    text_start
}

fn assign_symbol_addresses(
    global_symbols: &mut FxHashMap<String, LinkerSymbol>,
    output_sections: &[OutputSection],
    got_base: u32,
    plt_vaddr: u32,
    plt_header_size: u32,
    plt_entry_size: u32,
    bss_vaddr: u32,
    data_seg_vaddr_end: u32,
    data_seg_vaddr_start: u32,
    text_seg_vaddr_end: u32,
    dynamic_vaddr: u32,
    is_static: bool,
    (preinit_array_vaddr, preinit_array_size): (u32, u32),
    init_array_vaddr: u32,
    init_array_size: u32,
    fini_array_vaddr: u32,
    fini_array_size: u32,
    rel_iplt_vaddr: u32,
    rel_iplt_size: u32,
    pending_defsyms: &[(String, String, usize)],
) -> Result<(), String> {
    global_symbols
        .entry("_GLOBAL_OFFSET_TABLE_".to_string())
        .or_insert(LinkerSymbol {
            address: got_base,
            size: 0,
            sym_type: STT_OBJECT,
            binding: STB_LOCAL,
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
        });
    if let Some(sym) = global_symbols.get_mut("_GLOBAL_OFFSET_TABLE_") {
        sym.address = got_base;
        sym.is_defined = true;
    }

    let linker_addrs = LinkerSymbolAddresses {
        base_addr: BASE_ADDR as u64,
        got_addr: got_base as u64,
        dynamic_addr: if is_static { 0 } else { dynamic_vaddr as u64 },
        bss_addr: bss_vaddr as u64,
        bss_size: (data_seg_vaddr_end - bss_vaddr) as u64,
        text_end: text_seg_vaddr_end as u64,
        data_start: data_seg_vaddr_start as u64,
        init_array_start: init_array_vaddr as u64,
        init_array_size: init_array_size as u64,
        fini_array_start: fini_array_vaddr as u64,
        fini_array_size: fini_array_size as u64,
        preinit_array_start: preinit_array_vaddr as u64,
        preinit_array_size: preinit_array_size as u64,
        rela_iplt_start: rel_iplt_vaddr as u64,
        rela_iplt_size: rel_iplt_size as u64,
    };
    let standard_syms = get_standard_linker_symbols(&linker_addrs);
    let linker_sym_map: FxHashMap<&str, u64> = standard_syms
        .iter()
        .filter(|s| !s.name.starts_with("__rela_iplt"))
        .map(|s| (s.name, s.value))
        .collect();

    // GNU ld's language symbols (`_start`, `end`, `_etext`, …) always exist,
    // whether or not the inputs reference them.  Seed the symbol table with
    // all of them so that a `--defsym` expression naming one (e.g.
    // `half=(end-_start)/2`) resolves even when nothing else did.  Seeding is
    // harmless to the output: the executable carries no .symtab, and the
    // dynsym writer only emits dynamic symbols.
    for (name, value) in &linker_sym_map {
        global_symbols
            .entry(name.to_string())
            .or_insert(LinkerSymbol {
                address: *value as u32,
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
            });
    }

    for (name, sym) in global_symbols.iter_mut() {
        if sym.is_dynamic {
            if sym.needs_plt {
                sym.address = plt_vaddr + plt_header_size + (sym.plt_index as u32) * plt_entry_size;
            }
            continue;
        }
        if sym.output_section < output_sections.len() {
            sym.address = output_sections[sym.output_section].addr + sym.section_offset;
        }
        if let Some(&value) = linker_sym_map.get(name.as_str()) {
            sym.address = value as u32;
            if name == "__dso_handle" {
                sym.is_defined = true;
            }
        }
        match name.as_str() {
            "__bss_start__" => sym.address = bss_vaddr,
            "edata" => sym.address = bss_vaddr,
            "end" | "__end__" => sym.address = data_seg_vaddr_end,
            "__rel_iplt_start" => sym.address = rel_iplt_vaddr,
            "__rel_iplt_end" => sym.address = rel_iplt_vaddr + rel_iplt_size,
            _ => {}
        }
        // Auto-generate __start_<section> / __stop_<section> symbols (GNU ld feature).
        // Uses data.len() for __stop_ (equals mem_size for PROGBITS; custom sections are always PROGBITS).
        if let Some(sec_name) = name.strip_prefix("__start_") {
            if linker_common::is_valid_c_identifier_for_section(sec_name) {
                if let Some(sec) = output_sections.iter().find(|s| s.name == sec_name) {
                    sym.address = sec.addr;
                    sym.is_defined = true;
                }
            }
        } else if let Some(sec_name) = name.strip_prefix("__stop_") {
            if linker_common::is_valid_c_identifier_for_section(sec_name) {
                if let Some(sec) = output_sections.iter().find(|s| s.name == sec_name) {
                    sym.address = sec.addr + sec.data.len() as u32;
                    sym.is_defined = true;
                }
            }
        }
    }

    // Last: finalise --defsym constants/expressions, so they see the section
    // addresses and the standard linker symbols, and so a user definition of
    // a standard name (e.g. `--defsym _end=...`) wins over the auto value.
    super::link::evaluate_pending_defsyms(global_symbols, pending_defsyms)
}

/// How PLT entries address their `.got.plt` slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PltAddressing {
    /// Position-dependent executable: `jmp *abs32` / `pushl abs32`.
    Absolute,
    /// Shared object: slots are addressed relative to `%ebx`, which the i386
    /// psABI requires every PLT caller to load with the address of
    /// `_GLOBAL_OFFSET_TABLE_` (the payload is that symbol's link-time
    /// value).  Absolute slot addresses are wrong in a shared object: the
    /// object is loaded at an arbitrary base, so `jmp *abs32` reads a
    /// random (usually unmapped) address and faults.
    EbxRelative { got_symbol: u32 },
}

/// Build the i386 lazy-binding PLT.
///
/// ```text
///            Absolute                     EbxRelative (PIC)
/// PLT0:      ff 35 <GOT.PLT+4>            ff b3 <GOT.PLT+4 - GOT>     pushl
///            ff 25 <GOT.PLT+8>            ff a3 <GOT.PLT+8 - GOT>     jmp *
/// PLTn:      ff 25 <slot>                 ff a3 <slot - GOT>          jmp *
///            68 <n*8>                     68 <n*8>                    push reloc off
///            e9 <PLT0 - next>             e9 <PLT0 - next>            jmp PLT0
/// ```
pub(super) fn build_plt(
    num_plt: usize,
    plt_vaddr: u32,
    plt_header_size: u32,
    plt_entry_size: u32,
    gotplt_vaddr: u32,
    gotplt_reserved: u32,
    addressing: PltAddressing,
) -> Vec<u8> {
    let mut plt_data: Vec<u8> = Vec::new();
    if num_plt == 0 {
        return plt_data;
    }

    // (ModRM for `pushl m32`, ModRM for `jmp *m32`, bias subtracted from
    // the slot address).  ModRM 0x35/0x25 = disp32 absolute; 0xb3/0xa3 =
    // disp32(%ebx).
    let (push_modrm, jmp_modrm, bias) = match addressing {
        PltAddressing::Absolute => (0x35u8, 0x25u8, 0u32),
        PltAddressing::EbxRelative { got_symbol } => (0xb3u8, 0xa3u8, got_symbol),
    };

    // PLT[0]: resolver stub — push GOT.PLT[1] (link_map), jump GOT.PLT[2].
    let got1 = (gotplt_vaddr + 4).wrapping_sub(bias);
    let got2 = (gotplt_vaddr + 8).wrapping_sub(bias);
    plt_data.push(0xff);
    plt_data.push(push_modrm);
    plt_data.extend_from_slice(&got1.to_le_bytes());
    plt_data.push(0xff);
    plt_data.push(jmp_modrm);
    plt_data.extend_from_slice(&got2.to_le_bytes());
    while plt_data.len() < plt_header_size as usize {
        plt_data.push(0x90);
    }

    // PLT[N]
    for i in 0..num_plt {
        let gotplt_entry = (gotplt_vaddr + (gotplt_reserved + i as u32) * 4).wrapping_sub(bias);
        let plt_entry_addr = plt_vaddr + plt_header_size + (i as u32) * plt_entry_size;

        plt_data.push(0xff);
        plt_data.push(jmp_modrm);
        plt_data.extend_from_slice(&gotplt_entry.to_le_bytes());
        plt_data.push(0x68);
        plt_data.extend_from_slice(&(i as u32 * 8).to_le_bytes());
        plt_data.push(0xe9);
        let target = plt_vaddr as i32 - (plt_entry_addr as i32 + plt_entry_size as i32);
        plt_data.extend_from_slice(&target.to_le_bytes());
    }

    plt_data
}

fn write_elf_header(output: &mut [u8], entry_point: u32, ehdr_size: u32, num_phdrs: u32) {
    output[0..4].copy_from_slice(&ELF_MAGIC);
    output[4] = ELFCLASS32;
    output[5] = ELFDATA2LSB;
    output[6] = EV_CURRENT;
    output[7] = 0;
    output[16..18].copy_from_slice(&ET_EXEC.to_le_bytes());
    output[18..20].copy_from_slice(&EM_386.to_le_bytes());
    output[20..24].copy_from_slice(&1u32.to_le_bytes());
    output[24..28].copy_from_slice(&entry_point.to_le_bytes());
    output[28..32].copy_from_slice(&ehdr_size.to_le_bytes());
    output[32..36].copy_from_slice(&0u32.to_le_bytes());
    output[36..40].copy_from_slice(&0u32.to_le_bytes());
    output[40..42].copy_from_slice(&(ehdr_size as u16).to_le_bytes());
    output[42..44].copy_from_slice(&32u16.to_le_bytes());
    output[44..46].copy_from_slice(&(num_phdrs as u16).to_le_bytes());
    output[46..48].copy_from_slice(&40u16.to_le_bytes());
    output[48..50].copy_from_slice(&0u16.to_le_bytes());
    output[50..52].copy_from_slice(&0u16.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disp32(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn absolute_plt_uses_absolute_slot_addresses() {
        let plt = build_plt(2, 0x1000, 16, 16, 0x3000, 3, PltAddressing::Absolute);
        assert_eq!(plt.len(), 48);
        assert_eq!(&plt[0..2], &[0xff, 0x35]);
        assert_eq!(disp32(&plt, 2), 0x3004);
        assert_eq!(&plt[6..8], &[0xff, 0x25]);
        assert_eq!(disp32(&plt, 8), 0x3008);
        // PLT1 -> GOT.PLT[4], reloc offset 8, jmp back to PLT0.
        assert_eq!(&plt[32..34], &[0xff, 0x25]);
        assert_eq!(disp32(&plt, 34), 0x3010);
        assert_eq!(plt[38], 0x68);
        assert_eq!(disp32(&plt, 39), 8);
        assert_eq!(plt[43], 0xe9);
        assert_eq!(disp32(&plt, 44) as i32, 0x1000 - (0x1020 + 16));
    }

    #[test]
    fn shared_plt_addresses_slots_relative_to_ebx_got_symbol() {
        // `_GLOBAL_OFFSET_TABLE_` sits below .got.plt (it names .got in the
        // shared-object layout), so the displacements are GOT-relative and
        // must round-trip to the absolute slot address.
        let got = 0x2ff0;
        let plt = build_plt(
            2,
            0x1000,
            16,
            16,
            0x3000,
            3,
            PltAddressing::EbxRelative { got_symbol: got },
        );
        assert_eq!(plt.len(), 48);
        assert_eq!(&plt[0..2], &[0xff, 0xb3], "pushl disp32(%ebx)");
        assert_eq!(got + disp32(&plt, 2), 0x3004);
        assert_eq!(&plt[6..8], &[0xff, 0xa3], "jmp *disp32(%ebx)");
        assert_eq!(got + disp32(&plt, 8), 0x3008);
        for i in 0..2u32 {
            let e = (16 + 16 * i) as usize;
            assert_eq!(&plt[e..e + 2], &[0xff, 0xa3]);
            assert_eq!(got + disp32(&plt, e + 2), 0x3000 + (3 + i) * 4);
            assert_eq!(disp32(&plt, e + 7), i * 8);
            let next = 0x1000 + e as u32 + 16;
            assert_eq!(next.wrapping_add(disp32(&plt, e + 12)), 0x1000);
        }
    }

    fn sec(name: &str, flags: u32, len: usize, addr: u32, off: u32) -> OutputSection {
        OutputSection {
            name: name.to_string(),
            sh_type: SHT_PROGBITS,
            flags,
            data: vec![0; len],
            align: 4,
            addr,
            file_offset: off,
        }
    }

    #[test]
    fn unplaced_allocatable_section_is_a_hard_error() {
        let placed = [
            sec(".text", SHF_ALLOC | SHF_EXECINSTR, 8, 0x1000, 0x1000),
            // Empty and non-alloc sections never reach the write loop at 0.
            sec(".note", SHF_ALLOC, 0, 0, 0),
            sec(".comment", 0, 16, 0, 0),
        ];
        assert!(check_sections_placed(&placed, "shared library").is_ok());

        let unplaced = [sec(".note", SHF_ALLOC, 0x30, 0, 0)];
        let err = check_sections_placed(&unplaced, "shared library").unwrap_err();
        assert!(
            err.contains("'.note'") && err.contains("never placed"),
            "{err}"
        );
    }
}
