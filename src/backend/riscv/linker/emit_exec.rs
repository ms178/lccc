//! RISC-V executable emission.
//!
//! Takes the results of input loading, section merging, and symbol resolution
//! (Phases 1-3) and performs layout, relocation, PLT/GOT construction, and
//! ELF writing (Phases 4-14) to produce a statically or dynamically linked
//! ELF64 executable.

use super::elf_read::*;
use super::got::{
    GotKind, GotLayout, R_RISCV_TLS_DTPMOD64, R_RISCV_TLS_DTPREL64, R_RISCV_TLS_TPREL64,
};
use super::relocations::{GlobalSym, MergedSection, align_up, build_gnu_hash, pad_to, write_phdr};
use super::{debug, reloc, symbols};
use crate::backend::linker_common;
use crate::backend::linker_common::shdr::{self, ShdrSpec};
use crate::common::fx_hash::{FxHashMap, FxHashSet};

use crate::backend::elf::{
    DT_DEBUG, DT_FINI_ARRAY, DT_FINI_ARRAYSZ, DT_GNU_HASH, DT_INIT_ARRAY, DT_INIT_ARRAYSZ,
    DT_JMPREL, DT_NEEDED, DT_NULL, DT_PLTGOT, DT_PLTREL, DT_PLTRELSZ, DT_PREINIT_ARRAY,
    DT_PREINIT_ARRAYSZ, DT_RELA, DT_RELAENT, DT_RELASZ, DT_STRSZ, DT_STRTAB, DT_SYMENT, DT_SYMTAB,
    DT_VERNEED, DT_VERNEEDNUM, DT_VERSYM, EM_RISCV as EM_RISCV_ELF, ET_EXEC, PF_R, PF_W, PF_X,
    PT_DYNAMIC, PT_GNU_RELRO, PT_GNU_STACK, PT_INTERP, PT_LOAD, PT_NOTE, PT_TLS, SHF_INFO_LINK,
    STT_FUNC, STT_GNU_IFUNC, STT_TLS,
};

const PT_GNU_EH_FRAME: u32 = 0x6474e550;
const PT_RISCV_ATTRIBUTES: u32 = 0x70000003;

const PAGE_SIZE: u64 = 0x1000;
const BASE_ADDR: u64 = 0x10000;

const INTERP: &[u8] = b"/lib/ld-linux-riscv64-lp64d.so.1\0";

/// Emit a RISC-V ELF executable from pre-resolved linking state.
///
/// `input_objs`: parsed ELF objects (CRT + user + archive members)
/// `merged_sections` / `merged_map`: merged output sections from Phase 2
/// `sec_mapping`: (obj_idx, sec_idx) -> (merged_idx, offset_in_merged)
/// `global_syms`: resolved global symbol table from Phase 3
/// `got`: the GOT slots and their offsets
/// `tls_imports`: TLS variables imported from shared libraries
/// `local_got_sym_info`: local GOT entries: key -> (obj_idx, sym_idx, addend)
/// `plt_symbols`: symbols needing PLT entries (dynamic linking only)
/// `copy_symbols`: symbols needing R_COPY relocations: (name, size)
/// `sec_indices`: section layout ordering
/// `actual_needed_libs`: NEEDED sonames for .dynamic
/// `is_static`: true for static linking (no PLT/GOT/.dynamic)
/// `debug_sections`: the DWARF to append (none under `-s`/`-S`)
/// `keep_symtab`: write `.symtab` (false under `-s`)
/// `output_path`: output file path
pub fn emit_executable(
    input_objs: &[(String, ElfObject)],
    merged_sections: &mut Vec<MergedSection>,
    merged_map: &mut FxHashMap<String, usize>,
    sec_mapping: &FxHashMap<(usize, usize), (usize, u64)>,
    global_syms: &mut FxHashMap<String, GlobalSym>,
    got: &GotLayout,
    tls_imports: &[String],
    local_got_sym_info: &FxHashMap<String, (usize, usize, i64)>,
    plt_symbols: &[String],
    copy_symbols: &[(String, u64)],
    sec_indices: &[usize],
    actual_needed_libs: &[String],
    shared_lib_syms: &super::input::SharedLibSyms,
    rpath: Option<&(i64, String)>,
    is_static: bool,
    debug_sections: &mut debug::DebugSections,
    keep_symtab: bool,
    output_path: &str,
) -> Result<(), String> {
    // ── Phase 4: Compute generated section sizes and layout ────────────

    let plt_entry_size: u64 = 16;
    let plt_header_size: u64 = 32;
    let plt_size = if is_static || plt_symbols.is_empty() {
        0
    } else {
        plt_header_size + plt_symbols.len() as u64 * plt_entry_size
    };
    let got_plt_entries = if is_static { 0 } else { plt_symbols.len() + 2 };
    let got_plt_size = got_plt_entries as u64 * 8;
    let got_size = got.size();

    let interp_size = if is_static { 0 } else { INTERP.len() as u64 };

    let dynamic_size = if is_static {
        0
    } else {
        let dyn_entry_count = 30 + actual_needed_libs.len() + usize::from(rpath.is_some());
        dyn_entry_count as u64 * 16
    };

    // COPY relocations, then what the loader owes the GOT for TLS
    // imports: TPREL64 per initial-exec slot, DTPMOD64 + DTPREL64 per
    // `tls_index`.
    let tls_import_slots = || {
        got.entries()
            .filter(|(key, kind, _)| *kind != GotKind::Addr && tls_imports.iter().any(|t| t == key))
    };
    let tls_dyn_relocs: usize = tls_import_slots()
        .map(|(_, kind, _)| if kind == GotKind::TlsGd { 2 } else { 1 })
        .sum();
    let rela_dyn_size = if is_static {
        0
    } else {
        (copy_symbols.len() + tls_dyn_relocs) as u64 * 24
    };
    let rela_plt_size = if is_static {
        0
    } else {
        plt_symbols.len() as u64 * 24
    };

    // Create synthetic .eh_frame_hdr section
    let eh_frame_hdr_fde_count = merged_map
        .get(".eh_frame")
        .map(|&i| linker_common::count_eh_frame_fdes(&merged_sections[i].data))
        .unwrap_or(0);
    // The layout order was computed before this section existed: place it
    // right before `.eh_frame`, where GNU ld puts it (section_order 250 <
    // 260). (Pre-fix it was never laid out, so PT_GNU_EH_FRAME pointed at
    // address 0 and the unwinder found no FDE in any lccc-ld executable.)
    let mut layout_order: Vec<usize> = sec_indices.to_vec();
    if eh_frame_hdr_fde_count > 0 {
        let hdr_size = 12 + 8 * eh_frame_hdr_fde_count;
        let idx = merged_sections.len();
        merged_map.insert(".eh_frame_hdr".into(), idx);
        merged_sections.push(MergedSection {
            name: ".eh_frame_hdr".into(),
            sh_type: SHT_PROGBITS,
            sh_flags: SHF_ALLOC,
            data: vec![0u8; hdr_size],
            vaddr: 0,
            align: 4,
        });
        let ef_idx = merged_map.get(".eh_frame").copied();
        let at = layout_order
            .iter()
            .position(|&i| Some(i) == ef_idx)
            .unwrap_or(layout_order.len());
        layout_order.insert(at, idx);
    }
    let sec_indices: &[usize] = &layout_order;

    // Check for TLS sections
    let has_tls = merged_sections.iter().any(|ms| ms.sh_flags & SHF_TLS != 0);

    // Estimate phdr count
    let num_phdrs = if is_static {
        let base = 6; // LOAD(RX), LOAD(RW), NOTE, GNU_EH_FRAME, GNU_STACK, RISCV_ATTR
        if has_tls { base + 1 } else { base }
    } else if has_tls {
        11
    } else {
        10
    };
    let phdr_size = num_phdrs * 56u64;
    let headers_size = 64 + phdr_size;

    // Start laying out the RX segment
    let mut file_offset = headers_size;
    let mut vaddr = BASE_ADDR + headers_size;

    // Dynamic linking section addresses (only used when !is_static)
    let mut interp_offset = 0u64;
    let mut interp_vaddr = 0u64;
    let mut gnu_hash_vaddr = 0u64;
    let mut gnu_hash_offset = 0u64;
    let mut dynsym_vaddr = 0u64;
    let mut dynsym_offset = 0u64;
    let mut dynstr_vaddr = 0u64;
    let mut dynstr_offset = 0u64;
    let mut versym_vaddr = 0u64;
    let mut versym_offset = 0u64;
    let mut verneed_vaddr = 0u64;
    let mut verneed_offset = 0u64;
    let mut rela_dyn_vaddr = 0u64;
    let mut rela_dyn_offset = 0u64;
    let mut rela_plt_vaddr = 0u64;
    let mut rela_plt_offset = 0u64;
    let mut plt_vaddr = 0u64;
    let mut plt_offset = 0u64;

    let mut dynstr_data = vec![0u8]; // Leading null
    let mut dynsym_data = vec![0u8; 24]; // null entry
    let mut gnu_hash_data = Vec::new();
    let mut versym_data: Vec<u8> = Vec::new();
    let mut verneed_data: Vec<u8> = Vec::new();
    let mut verneed_count = 0u64;
    let mut needed_lib_offsets: Vec<u32> = Vec::new();
    let mut rpath_offset = 0u32;
    let mut dynsym_names: Vec<String> = Vec::new();
    let copy_sym_names: Vec<String> = copy_symbols.iter().map(|(n, _)| n.clone()).collect();

    if !is_static {
        // .interp
        interp_offset = file_offset;
        interp_vaddr = vaddr;
        file_offset += interp_size;
        vaddr += interp_size;

        // Build .gnu.hash, .dynsym, .dynstr for dynamic symbols
        let unsorted_names: Vec<String> = {
            let mut v = plt_symbols.to_vec();
            v.extend(copy_sym_names.iter().cloned());
            v.extend(tls_imports.iter().cloned());
            v
        };

        // Build .gnu.hash and get the sorted symbol order
        let sorted_order;
        (gnu_hash_data, sorted_order) = build_gnu_hash(&unsorted_names);

        // Reorder dynsym_names according to the hash table ordering
        dynsym_names = sorted_order
            .iter()
            .map(|&i| unsorted_names[i].clone())
            .collect();

        // Build dynstr
        dynstr_data = vec![0u8];
        let mut dynstr_offsets: FxHashMap<String, u32> = FxHashMap::default();
        for name in &dynsym_names {
            if !dynstr_offsets.contains_key(name) {
                let off = dynstr_data.len() as u32;
                dynstr_offsets.insert(name.clone(), off);
                dynstr_data.extend_from_slice(name.as_bytes());
                dynstr_data.push(0);
            }
        }
        for lib in actual_needed_libs {
            let off = dynstr_data.len() as u32;
            needed_lib_offsets.push(off);
            dynstr_data.extend_from_slice(lib.as_bytes());
            dynstr_data.push(0);
        }
        if let Some((_, path)) = rpath {
            rpath_offset = dynstr_data.len() as u32;
            dynstr_data.extend_from_slice(path.as_bytes());
            dynstr_data.push(0);
        }

        // Version requirements of the imports (PLT and COPY symbols alike
        // carry the provider's version, as with GNU ld).
        let plan = linker_common::symver::VerneedPlan::new(
            dynsym_names
                .iter()
                .filter_map(|n| shared_lib_syms.import_version(n)),
            actual_needed_libs,
            2,
        )?;
        let mut str_offs: FxHashMap<&str, u32> = actual_needed_libs
            .iter()
            .map(String::as_str)
            .zip(needed_lib_offsets.iter().copied())
            .collect();
        for s in plan.strings() {
            str_offs.entry(s).or_insert_with(|| {
                let off = dynstr_data.len() as u32;
                dynstr_data.extend_from_slice(s.as_bytes());
                dynstr_data.push(0);
                off
            });
        }
        if !plan.is_empty() {
            verneed_data = plan.encode(|s| str_offs[s]);
            verneed_count = plan.file_count();
            versym_data = vec![0u8; 2]; // null symbol: VER_NDX_LOCAL
            for name in &dynsym_names {
                let idx = match shared_lib_syms.provider_of(name) {
                    Some(lib) => {
                        plan.index_of(lib, shared_lib_syms.import_version(name).map(|v| v.1))
                    }
                    None => 0, // unresolved (weak) reference: VER_NDX_LOCAL, as bfd
                };
                versym_data.extend_from_slice(&idx.to_le_bytes());
            }
        }

        // Build dynsym (in hash-sorted order)
        dynsym_data = vec![0u8; 24]; // null entry
        let copy_sym_set: FxHashSet<String> = copy_sym_names.iter().cloned().collect();
        for name in dynsym_names.iter() {
            let mut entry = [0u8; 24];
            let name_off = dynstr_offsets.get(name).copied().unwrap_or(0);
            entry[0..4].copy_from_slice(&name_off.to_le_bytes());
            if copy_sym_set.contains(name) {
                entry[4] = (STB_GLOBAL << 4) | STT_OBJECT;
            } else if tls_imports.contains(name) {
                let bind = global_syms.get(name).map_or(STB_GLOBAL, |g| g.binding);
                entry[4] = (bind << 4) | STT_TLS;
            } else if let Some(gsym) = global_syms.get(name) {
                let bind = gsym.binding;
                let stype = if gsym.sym_type != 0 {
                    gsym.sym_type
                } else {
                    STT_FUNC
                };
                entry[4] = (bind << 4) | stype;
            } else {
                entry[4] = (STB_GLOBAL << 4) | STT_FUNC;
            }
            entry[5] = STV_DEFAULT;
            entry[6..8].copy_from_slice(&0u16.to_le_bytes());
            dynsym_data.extend_from_slice(&entry);
        }

        // Layout generated RX sections
        vaddr = align_up(vaddr, 8);
        file_offset = align_up(file_offset, 8);
        gnu_hash_vaddr = vaddr;
        gnu_hash_offset = file_offset;
        file_offset += gnu_hash_data.len() as u64;
        vaddr += gnu_hash_data.len() as u64;

        vaddr = align_up(vaddr, 8);
        file_offset = align_up(file_offset, 8);
        dynsym_vaddr = vaddr;
        dynsym_offset = file_offset;
        file_offset += dynsym_data.len() as u64;
        vaddr += dynsym_data.len() as u64;

        dynstr_vaddr = vaddr;
        dynstr_offset = file_offset;
        file_offset += dynstr_data.len() as u64;
        vaddr += dynstr_data.len() as u64;

        vaddr = align_up(vaddr, 2);
        file_offset = align_up(file_offset, 2);
        versym_vaddr = vaddr;
        versym_offset = file_offset;
        file_offset += versym_data.len() as u64;
        vaddr += versym_data.len() as u64;

        vaddr = align_up(vaddr, 8);
        file_offset = align_up(file_offset, 8);
        verneed_vaddr = vaddr;
        verneed_offset = file_offset;
        file_offset += verneed_data.len() as u64;
        vaddr += verneed_data.len() as u64;

        vaddr = align_up(vaddr, 8);
        file_offset = align_up(file_offset, 8);
        rela_dyn_vaddr = vaddr;
        rela_dyn_offset = file_offset;
        file_offset += rela_dyn_size;
        vaddr += rela_dyn_size;

        vaddr = align_up(vaddr, 8);
        file_offset = align_up(file_offset, 8);
        rela_plt_vaddr = vaddr;
        rela_plt_offset = file_offset;
        file_offset += rela_plt_size;
        vaddr += rela_plt_size;

        vaddr = align_up(vaddr, 16);
        file_offset = align_up(file_offset, 16);
        plt_vaddr = vaddr;
        plt_offset = file_offset;
        file_offset += plt_size;
        vaddr += plt_size;
    }

    // IFUNC support for static executables: GNU ld's .iplt/.rela.iplt scheme
    // (port of the AArch64 linker's emit_static design to rv64).  Every ifunc
    // becomes a 16-byte IPLT stub that jumps through a dedicated GOT slot; the
    // slot carries R_RISCV_IRELATIVE (addend = resolver address) so glibc's
    // static startup — which walks __rela_iplt_start .. __rela_iplt_end —
    // resolves it before main.  Without this the slot keeps its link-time
    // value, glibc's memcpy/strlen pointers point into .bss, and the first
    // startup TLS-image copy fetches data as code: SIGSEGV before main in
    // every ifunc-consuming static binary (e.g. any glibc static link).
    let ifunc_names: Vec<String> = if is_static {
        let mut names: Vec<String> = global_syms
            .iter()
            .filter(|(_, g)| g.sym_type == STT_GNU_IFUNC && g.defined)
            .map(|(n, _)| n.clone())
            .collect();
        names.sort();
        names
    } else {
        Vec::new()
    };
    let ifunc_count = ifunc_names.len() as u64;
    let iplt_size = ifunc_count * 16;

    let mut iplt_vaddr = 0u64;
    let mut iplt_offset = 0u64;
    if iplt_size > 0 {
        vaddr = align_up(vaddr, 16);
        file_offset = align_up(file_offset, 16);
        iplt_vaddr = vaddr;
        iplt_offset = file_offset;
        file_offset += iplt_size;
        vaddr += iplt_size;
    }

    // Assign vaddrs to RX sections (alloc, non-write)
    let mut section_vaddrs: Vec<u64> = vec![0; merged_sections.len()];
    let mut section_offsets: Vec<u64> = vec![0; merged_sections.len()];

    for &si in sec_indices {
        let ms = &merged_sections[si];
        if ms.sh_flags & SHF_ALLOC == 0 {
            continue;
        }
        if ms.sh_flags & SHF_WRITE != 0 {
            continue;
        }
        if ms.name == ".riscv.attributes" || ms.name.starts_with(".note.") {
            continue;
        }

        let align = ms.align.max(1);
        vaddr = align_up(vaddr, align);
        file_offset = align_up(file_offset, align);
        section_vaddrs[si] = vaddr;
        section_offsets[si] = file_offset;
        let size = ms.data.len() as u64;
        file_offset += size;
        vaddr += size;
    }

    let rx_segment_end_vaddr = vaddr;
    let rx_segment_end_offset = file_offset;

    // Start RW segment on a new page
    file_offset = align_up(file_offset, PAGE_SIZE);
    vaddr = align_up(vaddr, PAGE_SIZE);
    if (vaddr % PAGE_SIZE) != (file_offset % PAGE_SIZE) {
        vaddr = align_up(vaddr, PAGE_SIZE) + (file_offset % PAGE_SIZE);
    }

    let rw_segment_start_vaddr = vaddr;
    let rw_segment_start_offset = file_offset;

    // Layout RW sections: init/fini arrays first (for RELRO)
    let init_array_sections = [".preinit_array", ".init_array", ".fini_array"];
    let mut init_array_vaddrs: FxHashMap<String, (u64, u64)> = FxHashMap::default();

    for sect_name in &init_array_sections {
        if let Some(&si) = merged_map.get(*sect_name) {
            let ms = &merged_sections[si];
            let align = ms.align.max(8);
            vaddr = align_up(vaddr, align);
            file_offset = align_up(file_offset, align);
            section_vaddrs[si] = vaddr;
            section_offsets[si] = file_offset;
            init_array_vaddrs.insert(sect_name.to_string(), (vaddr, ms.data.len() as u64));
            let size = ms.data.len() as u64;
            file_offset += size;
            vaddr += size;
        }
    }

    // .dynamic (only for dynamic binaries)
    let mut dynamic_vaddr = 0u64;
    let mut dynamic_offset = 0u64;
    if !is_static {
        vaddr = align_up(vaddr, 8);
        file_offset = align_up(file_offset, 8);
        dynamic_vaddr = vaddr;
        dynamic_offset = file_offset;
        file_offset += dynamic_size;
        vaddr += dynamic_size;
    }

    // .got
    vaddr = align_up(vaddr, 8);
    file_offset = align_up(file_offset, 8);
    let got_vaddr = vaddr;
    let got_offset = file_offset;
    file_offset += got_size;
    vaddr += got_size;

    // RELRO boundary — ld.so rounds [vaddr, vaddr+memsz) down to page edges
    // before mprotect(PROT_READ), so a memsz that does not reach the next
    // page protects NOTHING (pre-fix riscv exes shipped .dynamic/.got
    // writable; verified against Linux glibc under qemu). bfd/lld right-size
    // the region to the page: close the RELRO group at the page boundary so
    // the writable tail (·data, TLS, .bss, IFUNC slots) starts on a fresh
    // page. PT_GNU_RELRO p_align stays 1 like bfd's — glibc consumes only
    // vaddr/memsz.
    file_offset = align_up(file_offset, PAGE_SIZE);
    vaddr = align_up(vaddr, PAGE_SIZE);
    let relro_end_offset = file_offset;
    let relro_end_vaddr = vaddr;

    // IFUNC GOT slots (one 8-byte slot per ifunc, initial content = its IPLT
    // stub address) and the R_RISCV_IRELATIVE table (.rela.iplt: Elf64_Rela
    // { r_offset = slot VA, r_info = IRELATIVE, r_addend = resolver VA }).
    // Kept OUTSIDE the RELRO region: glibc's startup writes the resolved
    // implementation into each slot before main runs.
    let mut irel_slot_vaddr = 0u64;
    let mut irel_slot_offset = 0u64;
    let mut irela_vaddr = 0u64;
    let mut irela_offset = 0u64;
    let irela_size = ifunc_count * 24;
    if ifunc_count > 0 {
        vaddr = align_up(vaddr, 8);
        file_offset = align_up(file_offset, 8);
        irel_slot_vaddr = vaddr;
        irel_slot_offset = file_offset;
        file_offset += ifunc_count * 8;
        vaddr += ifunc_count * 8;

        vaddr = align_up(vaddr, 8);
        file_offset = align_up(file_offset, 8);
        irela_vaddr = vaddr;
        irela_offset = file_offset;
        file_offset += irela_size;
        vaddr += irela_size;
    }

    // .got.plt (only for dynamic binaries)
    let mut got_plt_vaddr = 0u64;
    let mut got_plt_offset = 0u64;
    if !is_static {
        vaddr = align_up(vaddr, 8);
        file_offset = align_up(file_offset, 8);
        got_plt_vaddr = vaddr;
        got_plt_offset = file_offset;
        file_offset += got_plt_size;
        vaddr += got_plt_size;

        for (i, name) in plt_symbols.iter().enumerate() {
            if let Some(sym) = global_syms.get_mut(name) {
                sym.value = plt_vaddr + plt_header_size + i as u64 * plt_entry_size;
            }
        }
    }

    // Remaining RW sections (.data, .sdata, etc.) - skip TLS
    for &si in sec_indices {
        let ms = &merged_sections[si];
        if ms.sh_flags & SHF_ALLOC == 0 {
            continue;
        }
        if ms.sh_flags & SHF_WRITE == 0 {
            continue;
        }
        if init_array_sections.contains(&ms.name.as_str()) {
            continue;
        }
        if ms.name == ".bss" || ms.name == ".sbss" {
            continue;
        }
        if ms.sh_flags & SHF_TLS != 0 {
            continue;
        }

        let align = ms.align.max(1);
        vaddr = align_up(vaddr, align);
        file_offset = align_up(file_offset, align);
        section_vaddrs[si] = vaddr;
        section_offsets[si] = file_offset;
        let size = ms.data.len() as u64;
        file_offset += size;
        vaddr += size;
    }

    // TLS sections
    let mut tls_vaddr = 0u64;
    let mut tls_offset = 0u64;
    let mut tls_filesz = 0u64;
    let mut tls_memsz = 0u64;
    let mut tls_align = 1u64;

    if has_tls {
        for &si in sec_indices {
            let ms = &merged_sections[si];
            if ms.sh_flags & SHF_TLS == 0 || ms.sh_type == SHT_NOBITS {
                continue;
            }
            let align = ms.align.max(1);
            vaddr = align_up(vaddr, align);
            file_offset = align_up(file_offset, align);
            if tls_vaddr == 0 {
                tls_vaddr = vaddr;
                tls_offset = file_offset;
            }
            tls_align = tls_align.max(align);
            section_vaddrs[si] = vaddr;
            section_offsets[si] = file_offset;
            let size = ms.data.len() as u64;
            file_offset += size;
            vaddr += size;
            tls_filesz += size;
        }
        tls_memsz = tls_filesz;

        for &si in sec_indices {
            let ms = &merged_sections[si];
            if ms.sh_flags & SHF_TLS == 0 || ms.sh_type != SHT_NOBITS {
                continue;
            }
            let align = ms.align.max(1);
            vaddr = align_up(vaddr, align);
            if tls_vaddr == 0 {
                tls_vaddr = vaddr;
                tls_offset = file_offset;
            }
            tls_align = tls_align.max(align);
            section_vaddrs[si] = vaddr;
            section_offsets[si] = file_offset;
            let size = ms.data.len() as u64;
            vaddr += size;
        }
        if tls_vaddr != 0 {
            tls_memsz = vaddr - tls_vaddr;
        }
    }

    // .bss and .sbss (NOBITS) - skip TLS .tbss
    for &si in sec_indices {
        let ms = &merged_sections[si];
        if ms.name != ".bss" && ms.name != ".sbss" {
            continue;
        }
        if ms.sh_flags & SHF_TLS != 0 {
            continue;
        }
        let align = ms.align.max(1);
        vaddr = align_up(vaddr, align);
        section_vaddrs[si] = vaddr;
        section_offsets[si] = file_offset;
        vaddr += ms.data.len() as u64;
    }

    // Allocate COPY-relocated symbols in .bss
    let mut copy_sym_addrs: FxHashMap<String, (u64, u64)> = FxHashMap::default();
    for (name, size) in copy_symbols {
        let sz = if *size > 0 { *size } else { 8 };
        let align = sz.min(8);
        vaddr = align_up(vaddr, align);
        copy_sym_addrs.insert(name.clone(), (vaddr, sz));
        if let Some(gs) = global_syms.get_mut(name) {
            gs.defined = true;
            gs.value = vaddr;
            gs.size = sz;
            gs.sym_type = STT_OBJECT;
        }
        vaddr += sz;
    }

    if !copy_symbols.is_empty() {
        if let Some(&bss_idx) = merged_map.get(".bss") {
            let bss_start = section_vaddrs[bss_idx];
            let new_bss_size = vaddr - bss_start;
            merged_sections[bss_idx]
                .data
                .resize(new_bss_size as usize, 0);
        }
    }

    let rw_segment_end_vaddr = vaddr;
    let rw_segment_filesz = file_offset - rw_segment_start_offset;
    let rw_segment_memsz = rw_segment_end_vaddr - rw_segment_start_vaddr;

    // ── Phase 5: Fix up symbol values with final vaddrs ─────────────────

    for (_, sym) in global_syms.iter_mut() {
        if sym.defined {
            if let Some(si) = sym.section_idx {
                sym.value += section_vaddrs[si];
            }
        }
    }

    // IFUNC: capture resolver addresses (the symbol's resolved value IS the
    // resolver — an ifunc definition is the resolver function), then redirect
    // every ifunc symbol to its IPLT stub and retype it STT_FUNC so all
    // relocation paths treat it as a plain function.
    let ifunc_syms: Vec<(String, u64)> = ifunc_names
        .iter()
        .map(|name| {
            (
                name.clone(),
                global_syms.get(name).map(|g| g.value).unwrap_or(0),
            )
        })
        .collect();
    for (i, (name, _resolver)) in ifunc_syms.iter().enumerate() {
        if let Some(g) = global_syms.get_mut(name) {
            g.value = iplt_vaddr + i as u64 * 16;
            g.sym_type = STT_FUNC;
        }
    }

    // Define linker-provided symbols
    let sdata_vaddr = merged_map
        .get(".sdata")
        .map(|&i| section_vaddrs[i])
        .unwrap_or(0);
    let data_vaddr = merged_map
        .get(".data")
        .map(|&i| section_vaddrs[i])
        .unwrap_or(sdata_vaddr);
    let bss_vaddr = merged_map
        .get(".bss")
        .map(|&i| section_vaddrs[i])
        .unwrap_or(0);
    let bss_end = merged_map
        .get(".bss")
        .map(|&i| section_vaddrs[i] + merged_sections[i].data.len() as u64)
        .unwrap_or(bss_vaddr);

    let init_start = init_array_vaddrs
        .get(".init_array")
        .map(|&(v, _)| v)
        .unwrap_or(0);
    let init_end = init_array_vaddrs
        .get(".init_array")
        .map(|&(v, s)| v + s)
        .unwrap_or(0);
    let fini_start = init_array_vaddrs
        .get(".fini_array")
        .map(|&(v, _)| v)
        .unwrap_or(0);
    let fini_end = init_array_vaddrs
        .get(".fini_array")
        .map(|&(v, s)| v + s)
        .unwrap_or(0);
    let preinit_start = init_array_vaddrs
        .get(".preinit_array")
        .map(|&(v, _)| v)
        .unwrap_or(0);
    let preinit_end = init_array_vaddrs
        .get(".preinit_array")
        .map(|&(v, s)| v + s)
        .unwrap_or(0);

    let linker_addrs = LinkerSymbolAddresses {
        base_addr: BASE_ADDR,
        got_addr: got_plt_vaddr,
        dynamic_addr: dynamic_vaddr,
        bss_addr: bss_vaddr,
        bss_size: bss_end - bss_vaddr,
        text_end: rx_segment_end_vaddr,
        data_start: data_vaddr,
        init_array_start: init_start,
        init_array_size: init_end - init_start,
        fini_array_start: fini_start,
        fini_array_size: fini_end - fini_start,
        preinit_array_start: preinit_start,
        preinit_array_size: preinit_end - preinit_start,
        rela_iplt_start: irela_vaddr,
        rela_iplt_size: irela_size,
    };

    {
        // `placeholder` marks a definition this linker invents to satisfy an
        // optional runtime hook. It is carried on the symbol so `.symtab` can
        // tell it from a definition an input object actually provides: a real
        // definition is never overwritten here, so the flag stays false.
        let mut define_linker_sym = |name: &str, value: u64, binding: u8, placeholder: bool| {
            let entry = global_syms
                .entry(name.to_string())
                .or_insert_with(|| GlobalSym {
                    value: 0,
                    size: 0,
                    binding,
                    sym_type: STT_NOTYPE,
                    visibility: STV_DEFAULT,
                    defined: false,
                    needs_plt: false,
                    plt_idx: 0,
                    section_idx: None,
                    placeholder: false,
                });
            if !entry.defined {
                entry.value = value;
                entry.defined = true;
                entry.binding = binding;
                entry.placeholder = placeholder;
            }
        };

        for sym in &get_standard_linker_symbols(&linker_addrs) {
            define_linker_sym(sym.name, sym.value, sym.binding, false);
        }

        // RISC-V specific symbols
        let gp_value = if sdata_vaddr != 0 {
            sdata_vaddr + 0x800
        } else {
            data_vaddr + 0x800
        };
        define_linker_sym("__global_pointer$", gp_value, STB_GLOBAL, false);
        define_linker_sym("__BSS_END__", bss_end, STB_GLOBAL, false);
        define_linker_sym("__SDATA_BEGIN__", sdata_vaddr, STB_GLOBAL, false);
        define_linker_sym("__DATA_BEGIN__", data_vaddr, STB_GLOBAL, false);
        let rodata_vaddr = merged_map
            .get(".rodata")
            .map(|&i| section_vaddrs[i])
            .unwrap_or(0);
        define_linker_sym("_IO_stdin_used", rodata_vaddr, STB_GLOBAL, false);

        // Weak symbols for optional features
        define_linker_sym("_ITM_registerTMCloneTable", 0, STB_WEAK, true);
        define_linker_sym("_ITM_deregisterTMCloneTable", 0, STB_WEAK, true);
        define_linker_sym("__pthread_initialize_minimal", 0, STB_WEAK, true);
        define_linker_sym("_dl_rtld_map", 0, STB_WEAK, true);
        define_linker_sym("__gcc_personality_v0", 0, STB_WEAK, true);
        define_linker_sym("_Unwind_Resume", 0, STB_WEAK, true);
        define_linker_sym("_Unwind_ForcedUnwind", 0, STB_WEAK, true);
        define_linker_sym("_Unwind_GetCFA", 0, STB_WEAK, true);
    }

    // __start_<section> / __stop_<section> symbols
    // Note: we use section_vaddrs[] for the final virtual address, not sec.vaddr,
    // because MergedSection::vaddr is not updated during layout.
    for (si, sec) in merged_sections.iter().enumerate() {
        if linker_common::is_valid_c_identifier_for_section(&sec.name) {
            let sec_vaddr = section_vaddrs[si];
            let start_name = format!("__start_{}", sec.name);
            let stop_name = format!("__stop_{}", sec.name);
            if let Some(entry) = global_syms.get_mut(&start_name) {
                if !entry.defined {
                    entry.value = sec_vaddr;
                    entry.defined = true;
                }
            }
            if let Some(entry) = global_syms.get_mut(&stop_name) {
                if !entry.defined {
                    entry.value = sec_vaddr + sec.data.len() as u64;
                    entry.defined = true;
                }
            }
        }
    }

    // Patch dynsym entries for COPY-relocated symbols
    for (i, name) in dynsym_names.iter().enumerate() {
        if let Some(&(addr, size)) = copy_sym_addrs.get(name) {
            let entry_off = (i + 1) * 24;
            dynsym_data[entry_off + 6..entry_off + 8].copy_from_slice(&0xFFF1u16.to_le_bytes());
            dynsym_data[entry_off + 8..entry_off + 16].copy_from_slice(&addr.to_le_bytes());
            dynsym_data[entry_off + 16..entry_off + 24].copy_from_slice(&size.to_le_bytes());
        }
    }

    let local_sym_vaddrs =
        symbols::build_local_sym_vaddrs(input_objs, sec_mapping, &section_vaddrs, global_syms);

    // ── Phase 6: Apply relocations ──────────────────────────────────────

    let mut gd_tls_relax_info: FxHashMap<u64, (u64, i64)> = FxHashMap::default();
    let mut gd_tls_call_nop: FxHashSet<u64> = FxHashSet::default();
    reloc::collect_gd_tls_relax_info(
        input_objs,
        sec_mapping,
        &section_vaddrs,
        &local_sym_vaddrs,
        global_syms,
        got,
        &mut gd_tls_relax_info,
        &mut gd_tls_call_nop,
    );

    let empty_plt_addrs = FxHashMap::default();
    let ctx = reloc::RelocContext {
        sec_mapping,
        section_vaddrs: &section_vaddrs,
        local_sym_vaddrs: &local_sym_vaddrs,
        global_syms,
        got_vaddr,
        got,
        got_plt_vaddr,
        tls_vaddr,
        gd_tls_relax_info: &gd_tls_relax_info,
        gd_tls_call_nop: &gd_tls_call_nop,
        collect_relatives: false,
        plt_sym_addrs: &empty_plt_addrs,
    };
    reloc::apply_relocations(input_objs, merged_sections, &ctx)?;

    // ── Phase 7: Build GOT data ─────────────────────────────────────────

    let mut got_data = vec![0u8; got_size as usize];
    for (key, kind, off) in got.entries() {
        // A TLS import's slots are the loader's to fill (`.rela.dyn`).
        if kind != GotKind::Addr && tls_imports.iter().any(|t| t == key) {
            continue;
        }
        let value = if let Some(gs) = global_syms.get(key) {
            gs.value
        } else if let Some(&(oi, si, addend)) = local_got_sym_info.get(key) {
            local_sym_vaddrs[oi][si].wrapping_add(addend as u64)
        } else {
            return Err(format!("missing RISC-V GOT symbol '{key}'"));
        };
        let value = match kind {
            GotKind::Addr => value,
            GotKind::TlsIe => value.wrapping_sub(tls_vaddr),
            GotKind::TlsGd => {
                return Err(format!(
                    "RISC-V tls_index for '{key}', which the executable defines"
                ));
            }
        };
        let off = off as usize;
        got_data[off..off + 8].copy_from_slice(&value.to_le_bytes());
    }

    // Build GOT.PLT data
    let mut got_plt_data = vec![0u8; got_plt_size as usize];
    for i in 0..plt_symbols.len() {
        let off = (2 + i) * 8;
        let plt0 = plt_vaddr;
        if off + 8 <= got_plt_data.len() {
            got_plt_data[off..off + 8].copy_from_slice(&plt0.to_le_bytes());
        }
    }

    // ── Phase 8: Build PLT ──────────────────────────────────────────────

    let mut plt_data = vec![0u8; plt_size as usize];
    if !plt_symbols.is_empty() {
        build_plt_stubs(
            &mut plt_data,
            plt_vaddr,
            plt_header_size,
            plt_entry_size,
            got_plt_vaddr,
            plt_symbols,
        );
    }

    // ── Phase 9: Build .rela.plt ────────────────────────────────────────

    let mut rela_plt_data = Vec::with_capacity(rela_plt_size as usize);
    for (i, name) in plt_symbols.iter().enumerate() {
        let got_entry_addr = got_plt_vaddr + (2 + i) as u64 * 8;
        let sym_idx = dynsym_names.iter().position(|n| n == name).unwrap_or(0) + 1;
        let r_info = ((sym_idx as u64) << 32) | 5; // R_RISCV_JUMP_SLOT
        rela_plt_data.extend_from_slice(&got_entry_addr.to_le_bytes());
        rela_plt_data.extend_from_slice(&r_info.to_le_bytes());
        rela_plt_data.extend_from_slice(&0i64.to_le_bytes());
    }

    // ── Phase 9b: Build .rela.dyn (COPY relocations) ───────────────────

    let mut rela_dyn_data = Vec::with_capacity(rela_dyn_size as usize);
    for (name, _size) in copy_symbols {
        if let Some(&(addr, _sz)) = copy_sym_addrs.get(name) {
            let sym_idx = dynsym_names.iter().position(|n| n == name).unwrap_or(0) + 1;
            let r_info = ((sym_idx as u64) << 32) | 4; // R_RISCV_COPY
            rela_dyn_data.extend_from_slice(&addr.to_le_bytes());
            rela_dyn_data.extend_from_slice(&r_info.to_le_bytes());
            rela_dyn_data.extend_from_slice(&0i64.to_le_bytes());
        }
    }
    for (key, kind, off) in tls_import_slots() {
        let sym_idx = dynsym_names
            .iter()
            .position(|n| n == key)
            .ok_or_else(|| format!("missing dynamic symbol for RISC-V TLS import '{key}'"))?
            as u64
            + 1;
        let slot = got_vaddr + off;
        let relocs: &[(u64, u64)] = if kind == GotKind::TlsGd {
            &[
                (slot, R_RISCV_TLS_DTPMOD64),
                (slot + 8, R_RISCV_TLS_DTPREL64),
            ]
        } else {
            &[(slot, R_RISCV_TLS_TPREL64)]
        };
        for &(at, r_type) in relocs {
            rela_dyn_data.extend_from_slice(&at.to_le_bytes());
            rela_dyn_data.extend_from_slice(&((sym_idx << 32) | r_type).to_le_bytes());
            rela_dyn_data.extend_from_slice(&0i64.to_le_bytes());
        }
    }

    // ── Phase 9c: Build IPLT stubs, IFUNC GOT slots and .rela.iplt ──────

    let mut iplt_data = vec![0u8; iplt_size as usize];
    let mut irel_slot_data = vec![0u8; (ifunc_count * 8) as usize];
    let mut irela_data = Vec::with_capacity(irela_size as usize);
    for (i, (_name, resolver)) in ifunc_syms.iter().enumerate() {
        let stub_addr = iplt_vaddr + i as u64 * 16;
        let slot_addr = irel_slot_vaddr + i as u64 * 8;

        // Slot initial content: the IPLT stub address (GNU ld semantics — a
        // sane value even if IRELATIVE processing were skipped; glibc's
        // startup overwrites it with the resolver's choice).
        let slot_off = i * 8;
        irel_slot_data[slot_off..slot_off + 8].copy_from_slice(&stub_addr.to_le_bytes());

        // R_RISCV_IRELATIVE (58): slot := resolver(slot_addend) at startup.
        irela_data.extend_from_slice(&slot_addr.to_le_bytes());
        irela_data.extend_from_slice(&58u64.to_le_bytes());
        irela_data.extend_from_slice(&(*resolver as i64).to_le_bytes());

        // Stub (GNU .iplt encoding):
        //   auipc t3, hi(delta)        t3 = stub_pc + hi<<12
        //   ld    t3, lo(delta)(t3)    t3 = *slot
        //   jalr  t1, t3
        //   nop
        let delta = slot_addr as i64 - stub_addr as i64;
        let hi = (delta + 0x800) >> 12;
        let lo = delta - (hi << 12);
        let auipc = 0x17u32 | (28 << 7) | (((hi as u32) & 0xFFFFF) << 12);
        let ld = (((lo as u32) & 0xFFF) << 20) | (28 << 15) | (3 << 12) | (28 << 7) | 0x03;
        let jalr: u32 = (28 << 15) | (6 << 7) | 0x67; // jalr t1, 0(t3)
        let stub_off = (i as u64 * 16) as usize;
        iplt_data[stub_off..stub_off + 4].copy_from_slice(&auipc.to_le_bytes());
        iplt_data[stub_off + 4..stub_off + 8].copy_from_slice(&ld.to_le_bytes());
        iplt_data[stub_off + 8..stub_off + 12].copy_from_slice(&jalr.to_le_bytes());
        iplt_data[stub_off + 12..stub_off + 16].copy_from_slice(&0x13u32.to_le_bytes());
    }

    // ── Phase 10: Build .dynamic section ────────────────────────────────

    let mut dynamic_data = Vec::new();
    if !is_static {
        let mut add_dyn = |tag: i64, val: u64| {
            dynamic_data.extend_from_slice(&tag.to_le_bytes());
            dynamic_data.extend_from_slice(&val.to_le_bytes());
        };

        for (i, lib) in actual_needed_libs.iter().enumerate() {
            let _ = lib;
            add_dyn(DT_NEEDED, needed_lib_offsets[i] as u64);
        }
        if let Some(&(tag, _)) = rpath {
            add_dyn(tag, u64::from(rpath_offset));
        }
        if let Some(&(va, sz)) = init_array_vaddrs.get(".preinit_array") {
            if sz > 0 {
                add_dyn(DT_PREINIT_ARRAY, va);
                add_dyn(DT_PREINIT_ARRAYSZ, sz);
            }
        }
        if let Some(&(va, sz)) = init_array_vaddrs.get(".init_array") {
            if sz > 0 {
                add_dyn(DT_INIT_ARRAY, va);
                add_dyn(DT_INIT_ARRAYSZ, sz);
            }
        }
        if let Some(&(va, sz)) = init_array_vaddrs.get(".fini_array") {
            if sz > 0 {
                add_dyn(DT_FINI_ARRAY, va);
                add_dyn(DT_FINI_ARRAYSZ, sz);
            }
        }
        add_dyn(DT_GNU_HASH, gnu_hash_vaddr);
        add_dyn(DT_STRTAB, dynstr_vaddr);
        add_dyn(DT_SYMTAB, dynsym_vaddr);
        add_dyn(DT_STRSZ, dynstr_data.len() as u64);
        add_dyn(DT_SYMENT, 24);
        add_dyn(DT_DEBUG, 0);
        add_dyn(DT_PLTGOT, got_plt_vaddr);
        add_dyn(DT_PLTRELSZ, rela_plt_size);
        add_dyn(DT_PLTREL, 7);
        add_dyn(DT_JMPREL, rela_plt_vaddr);
        let rela_start = if rela_dyn_size > 0 {
            rela_dyn_vaddr
        } else {
            rela_plt_vaddr
        };
        let rela_total_size = rela_dyn_size + rela_plt_size;
        add_dyn(DT_RELA, rela_start);
        add_dyn(DT_RELASZ, rela_total_size);
        add_dyn(DT_RELAENT, 24);
        if !verneed_data.is_empty() {
            add_dyn(DT_VERNEED, verneed_vaddr);
            add_dyn(DT_VERNEEDNUM, verneed_count);
            add_dyn(DT_VERSYM, versym_vaddr);
        }
        add_dyn(DT_NULL, 0);
        dynamic_data.resize(dynamic_size as usize, 0);
    }

    // ── Phase 11: Build .eh_frame_hdr ───────────────────────────────────

    if let (Some(&hdr_idx), Some(&ef_idx)) =
        (merged_map.get(".eh_frame_hdr"), merged_map.get(".eh_frame"))
    {
        let eh_frame_vaddr = section_vaddrs[ef_idx];
        let eh_frame_hdr_vaddr = section_vaddrs[hdr_idx];
        let mut slot = std::mem::take(&mut merged_sections[hdr_idx].data);
        linker_common::write_eh_frame_hdr(
            &mut slot,
            &merged_sections[ef_idx].data,
            eh_frame_vaddr,
            eh_frame_hdr_vaddr,
            true,
        );
        merged_sections[hdr_idx].data = slot;
    }

    // ── Phase 12: Find entry point ──────────────────────────────────────

    let entry_point = if let Some(gs) = global_syms.get("_start") {
        gs.value
    } else if let Some(gs) = global_syms.get("main") {
        gs.value
    } else {
        merged_map
            .get(".text")
            .map(|&i| section_vaddrs[i])
            .unwrap_or(BASE_ADDR)
    };

    // ── Phase 13: .riscv.attributes data ────────────────────────────────

    let riscv_attr_data = merged_map
        .get(".riscv.attributes")
        .map(|&i| merged_sections[i].data.clone())
        .unwrap_or_default();
    let riscv_attr_offset = file_offset;
    let riscv_attr_size = riscv_attr_data.len() as u64;

    // ── Phase 14: Write ELF file ────────────────────────────────────────

    let mut elf = Vec::with_capacity(file_offset as usize + riscv_attr_data.len() + 4096);

    // ELF header
    write_elf_header(
        &mut elf,
        ET_EXEC,
        EM_RISCV_ELF,
        entry_point,
        num_phdrs as u16,
        0x05,
    );

    // Program headers
    assert_eq!(elf.len(), 64);

    let rx_filesz = rx_segment_end_offset;
    let rx_memsz = rx_segment_end_vaddr - BASE_ADDR;

    if !is_static {
        write_phdr(
            &mut elf,
            6, /* PT_PHDR */
            PF_R,
            64,
            BASE_ADDR + 64,
            BASE_ADDR + 64,
            phdr_size,
            phdr_size,
            8,
        );
        write_phdr(
            &mut elf,
            PT_INTERP,
            PF_R,
            interp_offset,
            interp_vaddr,
            interp_vaddr,
            interp_size,
            interp_size,
            1,
        );
    }

    write_phdr(
        &mut elf,
        PT_RISCV_ATTRIBUTES,
        PF_R,
        riscv_attr_offset,
        0,
        0,
        riscv_attr_size,
        riscv_attr_size,
        1,
    );
    write_phdr(
        &mut elf,
        PT_LOAD,
        PF_R | PF_X,
        0,
        BASE_ADDR,
        BASE_ADDR,
        rx_filesz,
        rx_memsz,
        PAGE_SIZE,
    );
    write_phdr(
        &mut elf,
        PT_LOAD,
        PF_R | PF_W,
        rw_segment_start_offset,
        rw_segment_start_vaddr,
        rw_segment_start_vaddr,
        rw_segment_filesz,
        rw_segment_memsz,
        PAGE_SIZE,
    );

    if !is_static {
        write_phdr(
            &mut elf,
            PT_DYNAMIC,
            PF_R | PF_W,
            dynamic_offset,
            dynamic_vaddr,
            dynamic_vaddr,
            dynamic_data.len() as u64,
            dynamic_data.len() as u64,
            8,
        );
    }

    write_phdr(&mut elf, PT_NOTE, PF_R, 0, 0, 0, 0, 0, 4);

    if let Some(&hdr_idx) = merged_map.get(".eh_frame_hdr") {
        let sz = merged_sections[hdr_idx].data.len() as u64;
        write_phdr(
            &mut elf,
            PT_GNU_EH_FRAME,
            PF_R,
            section_offsets[hdr_idx],
            section_vaddrs[hdr_idx],
            section_vaddrs[hdr_idx],
            sz,
            sz,
            4,
        );
    } else {
        write_phdr(&mut elf, PT_GNU_EH_FRAME, PF_R, 0, 0, 0, 0, 0, 4);
    }

    write_phdr(&mut elf, PT_GNU_STACK, PF_R | PF_W, 0, 0, 0, 0, 0, 0x10);

    if !is_static {
        let relro_filesz = relro_end_offset - rw_segment_start_offset;
        let relro_memsz = relro_end_vaddr - rw_segment_start_vaddr;
        write_phdr(
            &mut elf,
            PT_GNU_RELRO,
            PF_R,
            rw_segment_start_offset,
            rw_segment_start_vaddr,
            rw_segment_start_vaddr,
            relro_filesz,
            relro_memsz,
            1,
        );
    }

    if has_tls {
        write_phdr(
            &mut elf, PT_TLS, PF_R, tls_offset, tls_vaddr, tls_vaddr, tls_filesz, tls_memsz,
            tls_align,
        );
    }

    // Write section data
    if !is_static {
        pad_to(&mut elf, interp_offset as usize);
        elf.extend_from_slice(INTERP);
        pad_to(&mut elf, gnu_hash_offset as usize);
        elf.extend_from_slice(&gnu_hash_data);
        pad_to(&mut elf, dynsym_offset as usize);
        elf.extend_from_slice(&dynsym_data);
        pad_to(&mut elf, dynstr_offset as usize);
        elf.extend_from_slice(&dynstr_data);
        pad_to(&mut elf, versym_offset as usize);
        elf.extend_from_slice(&versym_data);
        pad_to(&mut elf, verneed_offset as usize);
        elf.extend_from_slice(&verneed_data);
        if !rela_dyn_data.is_empty() {
            pad_to(&mut elf, rela_dyn_offset as usize);
            elf.extend_from_slice(&rela_dyn_data);
        }
        pad_to(&mut elf, rela_plt_offset as usize);
        elf.extend_from_slice(&rela_plt_data);
        pad_to(&mut elf, plt_offset as usize);
        elf.extend_from_slice(&plt_data);
    }

    // IPLT stubs sit in the RX segment between the PLT block and the merged
    // text sections; write them before any later (higher-offset) region so
    // the pad_to calls stay monotone.
    if iplt_size > 0 {
        pad_to(&mut elf, iplt_offset as usize);
        elf.extend_from_slice(&iplt_data);
    }

    // Write RX merged sections
    for &si in sec_indices {
        let ms = &merged_sections[si];
        if ms.sh_flags & SHF_ALLOC == 0 || ms.sh_flags & SHF_WRITE != 0 {
            continue;
        }
        if ms.name == ".riscv.attributes" || ms.name.starts_with(".note.") {
            continue;
        }
        if ms.data.is_empty() {
            continue;
        }
        pad_to(&mut elf, section_offsets[si] as usize);
        elf.extend_from_slice(&ms.data);
    }

    // RW segment
    pad_to(&mut elf, rw_segment_start_offset as usize);

    for sect_name in &init_array_sections {
        if let Some(&si) = merged_map.get(*sect_name) {
            let ms = &merged_sections[si];
            if !ms.data.is_empty() {
                pad_to(&mut elf, section_offsets[si] as usize);
                elf.extend_from_slice(&ms.data);
            }
        }
    }

    if !is_static {
        pad_to(&mut elf, dynamic_offset as usize);
        elf.extend_from_slice(&dynamic_data);
    }

    if got_size > 0 {
        pad_to(&mut elf, got_offset as usize);
        elf.extend_from_slice(&got_data);
    }

    if ifunc_count > 0 {
        pad_to(&mut elf, irel_slot_offset as usize);
        elf.extend_from_slice(&irel_slot_data);
        pad_to(&mut elf, irela_offset as usize);
        elf.extend_from_slice(&irela_data);
    }

    if !is_static {
        pad_to(&mut elf, got_plt_offset as usize);
        elf.extend_from_slice(&got_plt_data);
    }

    for &si in sec_indices {
        let ms = &merged_sections[si];
        if ms.sh_flags & SHF_ALLOC == 0 || ms.sh_flags & SHF_WRITE == 0 {
            continue;
        }
        if init_array_sections.contains(&ms.name.as_str()) {
            continue;
        }
        if ms.sh_type == SHT_NOBITS || ms.name == ".bss" || ms.name == ".sbss" {
            continue;
        }
        if ms.data.is_empty() {
            continue;
        }
        pad_to(&mut elf, section_offsets[si] as usize);
        elf.extend_from_slice(&ms.data);
    }

    // .riscv.attributes
    pad_to(&mut elf, riscv_attr_offset as usize);
    elf.extend_from_slice(&riscv_attr_data);

    // DWARF after the image, then the section-header table and `.symtab`.
    let syms = debug::Symbols {
        input_objs,
        sec_mapping,
        section_vaddrs: &section_vaddrs,
        local_sym_vaddrs: &local_sym_vaddrs,
        global_syms,
    };
    debug::write_debug_sections(&mut elf, debug_sections, &syms)?;
    let mut specs = debug::merged_headers(merged_sections, &section_vaddrs, &section_offsets, true);
    if !is_static {
        specs.extend(shdr::dynamic_headers(&shdr::DynamicSections {
            interp: (interp_vaddr, interp_offset, interp_size),
            gnu_hash: (gnu_hash_vaddr, gnu_hash_offset, gnu_hash_data.len() as u64),
            dynsym: (dynsym_vaddr, dynsym_offset, dynsym_data.len() as u64),
            dynstr: (dynstr_vaddr, dynstr_offset, dynstr_data.len() as u64),
            gnu_version: (versym_vaddr, versym_offset, versym_data.len() as u64),
            gnu_version_r: (verneed_vaddr, verneed_offset, verneed_data.len() as u64),
            verneed_count: verneed_count as u32,
            rela_dyn: (rela_dyn_vaddr, rela_dyn_offset, rela_dyn_size),
            rela_plt: (rela_plt_vaddr, rela_plt_offset, rela_plt_size),
            plt: (plt_vaddr, plt_offset, plt_size),
            plt_entsize: 16,
            dynamic: (dynamic_vaddr, dynamic_offset, dynamic_data.len() as u64),
            got: (got_vaddr, got_offset, got_size),
            got_plt: (got_plt_vaddr, got_plt_offset, got_plt_size),
            ..Default::default()
        }));
    } else {
        specs.push(shdr::synthetic(
            ".got", SHF_WRITE, got_vaddr, got_offset, got_size, 8,
        ));
    }
    // The IFUNC stubs, their GOT slots and IRELATIVE table: bfd's output
    // names in a static link, its input names next to a real PLT.
    let (iplt, igot, irela) = if is_static {
        (".plt", ".got.plt", ".rela.plt")
    } else {
        (".iplt", ".igot.plt", ".rela.iplt")
    };
    specs.push(shdr::synthetic(
        iplt,
        SHF_EXECINSTR,
        iplt_vaddr,
        iplt_offset,
        iplt_size,
        16,
    ));
    specs.push(shdr::synthetic(
        igot,
        SHF_WRITE,
        irel_slot_vaddr,
        irel_slot_offset,
        ifunc_count * 8,
        8,
    ));
    specs.push(ShdrSpec {
        info_link: Some(igot),
        ..shdr::synthetic(
            irela,
            SHF_INFO_LINK,
            irela_vaddr,
            irela_offset,
            irela_size,
            8,
        )
    });
    specs.push(ShdrSpec {
        align: 1,
        ..ShdrSpec::new(
            ".riscv.attributes",
            SHT_RISCV_ATTRIBUTES,
            0,
            0,
            riscv_attr_offset,
            riscv_attr_size,
        )
    });
    specs.extend(debug_sections.headers());
    let headers = shdr::order_headers(specs);
    // A COPY-relocated definition lives in the section holding its slot
    // (`.bss`), as bfd records it, rather than SHN_ABS.
    for (i, name) in dynsym_names.iter().enumerate() {
        let Some(&(addr, _)) = copy_sym_addrs.get(name) else {
            continue;
        };
        if let shdr::SymSection::Named(n) = shdr::section_of(&headers, addr, false) {
            if let Some(idx) = shdr::header_index(&headers, &n) {
                let at = dynsym_offset as usize + (i + 1) * 24 + 6;
                elf[at..at + 2].copy_from_slice(&idx.to_le_bytes());
            }
        }
    }
    let symtab = keep_symtab.then(|| debug::symtab(&syms, &headers));
    shdr::append_section_headers(&mut elf, &headers, symtab.as_deref())?;

    // Write the file
    std::fs::write(output_path, &elf).map_err(|e| format!("Failed to write output: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(output_path, std::fs::Permissions::from_mode(0o755));
    }

    Ok(())
}

// ── Internal helpers ─────────────────────────────────────────────────────

/// Write a minimal ELF64 header for a RISC-V executable.
fn write_elf_header(
    elf: &mut Vec<u8>,
    e_type: u16,
    e_machine: u16,
    entry_point: u64,
    phnum: u16,
    e_flags: u32,
) {
    elf.extend_from_slice(&[0x7f, b'E', b'L', b'F']); // magic
    elf.push(2); // ELFCLASS64
    elf.push(1); // ELFDATA2LSB
    elf.push(1); // EV_CURRENT
    elf.push(0); // ELFOSABI_NONE
    elf.extend_from_slice(&[0; 8]); // padding
    elf.extend_from_slice(&e_type.to_le_bytes());
    elf.extend_from_slice(&e_machine.to_le_bytes());
    elf.extend_from_slice(&1u32.to_le_bytes()); // e_version
    elf.extend_from_slice(&entry_point.to_le_bytes());
    elf.extend_from_slice(&64u64.to_le_bytes()); // e_phoff
    elf.extend_from_slice(&0u64.to_le_bytes()); // e_shoff placeholder
    elf.extend_from_slice(&e_flags.to_le_bytes());
    elf.extend_from_slice(&64u16.to_le_bytes()); // e_ehsize
    elf.extend_from_slice(&56u16.to_le_bytes()); // e_phentsize
    elf.extend_from_slice(&phnum.to_le_bytes());
    elf.extend_from_slice(&64u16.to_le_bytes()); // e_shentsize
    elf.extend_from_slice(&0u16.to_le_bytes()); // e_shnum placeholder
    elf.extend_from_slice(&0u16.to_le_bytes()); // e_shstrndx placeholder
}

/// Build PLT header and per-symbol stubs for RISC-V.
fn build_plt_stubs(
    plt_data: &mut [u8],
    plt_vaddr: u64,
    plt_header_size: u64,
    plt_entry_size: u64,
    got_plt_vaddr: u64,
    plt_symbols: &[String],
) {
    // PLT[0] header: resolve stub
    let plt0_addr = plt_vaddr;
    let got_plt_rel = got_plt_vaddr as i64 - plt0_addr as i64;
    let hi = ((got_plt_rel + 0x800) >> 12) as u32;
    let lo = (got_plt_rel & 0xFFF) as u32;

    // auipc t2, hi
    let insn0 = 0x00000397 | (hi << 12);
    plt_data[0..4].copy_from_slice(&insn0.to_le_bytes());
    // sub t1, t1, t3
    let insn1 = 0x41c30333u32;
    plt_data[4..8].copy_from_slice(&insn1.to_le_bytes());
    // ld t3, lo(t2)
    let insn2 = 0x0003be03u32 | (lo << 20);
    plt_data[8..12].copy_from_slice(&insn2.to_le_bytes());
    // addi t1, t1, -(plt_header_size + 12)
    let neg_hdr = (-((plt_header_size + 12) as i32)) as u32;
    let insn3 = 0x00030313u32 | ((neg_hdr & 0xFFF) << 20);
    plt_data[12..16].copy_from_slice(&insn3.to_le_bytes());
    // addi t0, t2, lo
    let insn4 = 0x00038293u32 | (lo << 20);
    plt_data[16..20].copy_from_slice(&insn4.to_le_bytes());
    // srli t1, t1, 1
    let insn5 = 0x00135313u32;
    plt_data[20..24].copy_from_slice(&insn5.to_le_bytes());
    // ld t0, 8(t0)
    let insn6 = 0x0082b283u32;
    plt_data[24..28].copy_from_slice(&insn6.to_le_bytes());
    // jr t3
    let insn7 = 0x000e0067u32;
    plt_data[28..32].copy_from_slice(&insn7.to_le_bytes());

    // PLT entries
    for (i, _name) in plt_symbols.iter().enumerate() {
        let entry_off = (plt_header_size + i as u64 * plt_entry_size) as usize;
        let entry_addr = plt_vaddr + entry_off as u64;
        let got_entry_addr = got_plt_vaddr + (2 + i) as u64 * 8;

        let rel = got_entry_addr as i64 - entry_addr as i64;
        let hi = ((rel + 0x800) >> 12) as u32;
        let lo = (rel & 0xFFF) as u32;

        // auipc t3, hi
        let insn0 = 0x00000e17u32 | (hi << 12);
        plt_data[entry_off..entry_off + 4].copy_from_slice(&insn0.to_le_bytes());
        // ld t3, lo(t3)
        let insn1 = 0x000e3e03u32 | (lo << 20);
        plt_data[entry_off + 4..entry_off + 8].copy_from_slice(&insn1.to_le_bytes());
        // jalr t1, t3
        let insn2 = 0x000e0367u32;
        plt_data[entry_off + 8..entry_off + 12].copy_from_slice(&insn2.to_le_bytes());
        // nop
        let insn3 = 0x00000013u32;
        plt_data[entry_off + 12..entry_off + 16].copy_from_slice(&insn3.to_le_bytes());
    }
}
