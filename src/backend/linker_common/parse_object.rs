//! ELF64 relocatable object file parser.
//!
//! This single function replaces the near-identical `parse_object()` functions
//! in x86/linker/elf.rs, arm/linker/elf.rs, and riscv/linker/elf_read.rs.
//! The only parameter that differed was the expected e_machine value.

use super::filemap::FileBacking;
use super::secdata::SectionData;
use super::types::{Elf64Object, Elf64Rela, Elf64Section, Elf64Symbol};
use super::SymStr;
use crate::backend::elf::{
    read_cstr, read_cstr_ref, read_i64, read_u16, read_u32, read_u64, slice_at, table_entry,
    ELFCLASS64, ELFDATA2LSB, ELF_MAGIC, ET_REL, SHT_NOBITS, SHT_RELA, SHT_SYMTAB,
};

/// Parse an ELF64 relocatable object file (.o).
///
/// `expected_machine` is the ELF e_machine value to validate (e.g., EM_X86_64,
/// EM_AARCH64, EM_RISCV). Pass 0 to skip machine validation.
/// Parse an ELF64 object, **sharing** the caller's file buffer.
///
/// `buf` is the whole file (or whole archive) already in memory; the object
/// occupies `buf[base .. base + size]`. Section contents become windows into
/// `buf` rather than copies of it — see `secdata.rs` for why this is an
/// `Arc<[u8]>` window and not a borrow with a lifetime.
///
/// Prefer this over [`parse_elf64_object`] on any hot path: the latter has to
/// copy the input into a fresh `Arc` because it only receives a slice.
pub fn parse_elf64_object_at(
    buf: &std::sync::Arc<[u8]>,
    base: usize,
    size: usize,
    source_name: &str,
    expected_machine: u16,
) -> Result<Elf64Object, String> {
    parse_elf64_object_backed(
        &FileBacking::owned(std::sync::Arc::clone(buf)),
        base,
        size,
        source_name,
        expected_machine,
    )
}

/// Parse an ELF64 object from a standalone slice.
///
/// Convenience wrapper for callers that do not hold an `Arc` buffer; it copies
/// `data` once so section windows have something to share. Hot paths should
/// use [`parse_elf64_object_at`].
pub fn parse_elf64_object(
    data: &[u8],
    source_name: &str,
    expected_machine: u16,
) -> Result<Elf64Object, String> {
    parse_elf64_object_inner(data, None, source_name, expected_machine)
}

/// Parse an object that lives inside a memory-mapped (or read) input file.
///
/// This is the true zero-copy path: section bytes are windows into the
/// kernel's page cache, so the file is never copied into the linker's heap at
/// all. `base`/`size` delimit the object within the file, which is what lets a
/// single mapping serve every member of an archive.
pub fn parse_elf64_object_backed(
    backing: &FileBacking,
    base: usize,
    size: usize,
    source_name: &str,
    expected_machine: u16,
) -> Result<Elf64Object, String> {
    let all = backing.as_slice();
    let Some(data) = all.get(base..base.checked_add(size).unwrap_or(usize::MAX)) else {
        return Err(format!(
            "{}: object extends past end of buffer",
            source_name
        ));
    };
    parse_elf64_object_inner(data, Some((backing, base)), source_name, expected_machine)
}

fn parse_elf64_object_inner(
    data: &[u8],
    shared: Option<(&FileBacking, usize)>,
    source_name: &str,
    expected_machine: u16,
) -> Result<Elf64Object, String> {
    if data.len() < 64 {
        return Err(format!("{}: file too small for ELF header", source_name));
    }
    if data[0..4] != ELF_MAGIC {
        return Err(format!("{}: not an ELF file", source_name));
    }
    if data[4] != ELFCLASS64 {
        return Err(format!("{}: not 64-bit ELF", source_name));
    }
    if data[5] != ELFDATA2LSB {
        return Err(format!("{}: not little-endian ELF", source_name));
    }

    let e_type = read_u16(data, 16);
    if e_type != ET_REL {
        return Err(format!(
            "{}: not a relocatable object (type={})",
            source_name, e_type
        ));
    }

    if expected_machine != 0 {
        let e_machine = read_u16(data, 18);
        if e_machine != expected_machine {
            return Err(format!(
                "{}: wrong machine type (expected={}, got={})",
                source_name, expected_machine, e_machine
            ));
        }
    }

    let e_shoff = read_u64(data, 40) as usize;
    let e_shentsize = read_u16(data, 58) as usize;
    let e_shnum = read_u16(data, 60) as usize;
    let e_shstrndx = read_u16(data, 62) as usize;

    if e_shoff == 0 || e_shnum == 0 {
        return Err(format!("{}: no section headers", source_name));
    }

    // Parse section headers
    // A malformed e_shentsize would make every subsequent field read land at
    // the wrong offset; reject it up front rather than parsing garbage.
    if e_shentsize < 64 {
        return Err(format!(
            "{}: bogus e_shentsize {} (need >= 64)",
            source_name, e_shentsize
        ));
    }
    // `Vec::with_capacity(e_shnum)` on an attacker-controlled u16 is bounded
    // (<= 65535 * 88 bytes), so no reservation guard is needed here.
    let mut sections = Vec::with_capacity(e_shnum);
    for i in 0..e_shnum {
        // Overflow-safe: e_shoff is a full u64 and i * e_shentsize can itself
        // overflow, so both operations are checked.
        if table_entry(data, e_shoff, i, e_shentsize).is_none() {
            return Err(format!(
                "{}: section header {} out of bounds",
                source_name, i
            ));
        }
        let off = e_shoff + i * e_shentsize;
        // ELF gABI: sh_addralign is 0 or a positive integral power of two.
        // A non-power-of-two value is not merely cosmetic: the layout engine
        // rounds section addresses up to it, so a value such as 0xffffffffff
        // demands a 1 TiB-aligned output and aborts the process in the
        // allocator (`memory allocation of 1099511627796 bytes failed`).
        // A mutation fuzzer found exactly that.  bfd and mold silently accept
        // the bogus value; wild rejects it, which is the spec-correct and
        // safe behaviour, so lccc rejects it too — with the offending value
        // in the message.
        let addralign = read_u64(data, off + 48);
        if addralign > 1 && !addralign.is_power_of_two() {
            return Err(format!(
                "{}: section header {} has invalid sh_addralign 0x{:x} \
                 (must be 0 or a power of two)",
                source_name, i, addralign
            ));
        }
        sections.push(Elf64Section {
            name_idx: read_u32(data, off),
            name: String::new(),
            sh_type: read_u32(data, off + 4),
            flags: read_u64(data, off + 8),
            addr: read_u64(data, off + 16),
            offset: read_u64(data, off + 24),
            size: read_u64(data, off + 32),
            link: read_u32(data, off + 40),
            info: read_u32(data, off + 44),
            addralign,
            entsize: read_u64(data, off + 56),
        });
    }

    // Read section name string table
    if e_shstrndx < sections.len() {
        let shstrtab = &sections[e_shstrndx];
        let strtab_off = shstrtab.offset as usize;
        let strtab_size = shstrtab.size as usize;
        // Overflow-safe: `strtab_off + strtab_size` wraps for offsets near
        // usize::MAX, which previously turned a malformed object into a panic.
        if let Some(strtab_data) = slice_at(data, strtab_off, strtab_size) {
            for sec in &mut sections {
                sec.name = read_cstr(strtab_data, sec.name_idx as usize);
            }
        }
    }

    // Point each section at its bytes.
    //
    // When the caller supplied the backing buffer (`shared`), this is a pure
    // window: one refcount bump, zero bytes copied. `std::fs::read` has
    // already paid for one copy of the file; the `to_vec()` this replaces was
    // a second one, worth 2.68% of a 20 000-symbol link.
    //
    // Callers that only have a slice fall back to owning a private copy of
    // the whole input once, which is still no worse than per-section copies.
    let owned_buf: Option<FileBacking> = match shared {
        Some(_) => None,
        None => Some(FileBacking::owned(std::sync::Arc::from(data))),
    };
    let (backing, origin) = match shared {
        Some((buf, base)) => (buf, base),
        // `owned_buf` is Some on this branch by construction.
        None => (owned_buf.as_ref().unwrap(), 0usize),
    };

    let mut section_data = Vec::with_capacity(e_shnum);
    for sec in &sections {
        if sec.sh_type == SHT_NOBITS || sec.size == 0 {
            section_data.push(SectionData::empty());
        } else {
            let start = sec.offset as usize;
            let len = sec.size as usize;
            // Validate against the object's own extent first, so a section
            // that escapes this object but happens to land inside a larger
            // shared buffer (an archive member reading its neighbour) is still
            // rejected.
            if slice_at(data, start, len).is_none() {
                return Err(format!(
                    "{}: section '{}' data out of bounds",
                    source_name, sec.name
                ));
            }
            let Some(sd) = SectionData::slice_backing(backing, origin + start, len) else {
                return Err(format!(
                    "{}: section '{}' data out of bounds",
                    source_name, sec.name
                ));
            };
            section_data.push(sd);
        }
    }

    // Find symbol table and its string table
    let mut symbols = Vec::new();
    for i in 0..sections.len() {
        if sections[i].sh_type == SHT_SYMTAB {
            let strtab_idx = sections[i].link as usize;
            let strtab_data: &[u8] = if strtab_idx < section_data.len() {
                section_data[strtab_idx].as_slice()
            } else {
                continue;
            };
            // Bind the backing slice once. Indexing through `SectionData`'s
            // Deref re-derives a bounds-checked subslice on *every* access,
            // and this loop touches it ~6 times per symbol; measured, that
            // cost more than the section copy this type removed.
            let sym_data: &[u8] = section_data[i].as_slice();
            let sym_count = sym_data.len() / 24; // sizeof(Elf64_Sym) = 24
                                                 // Reserve the exact count up front. Growing one element at a time
                                                 // reallocated 15 times and memcpy'd 3.1 MB of Elf64Symbol on a
                                                 // 20 000-symbol object (DHAT). `sym_count` is derived from the
                                                 // section size, so this is exact, not a guess -- and it is bounded
                                                 // by the section that is already in memory, so a malformed header
                                                 // cannot make it request an absurd allocation.
            symbols.reserve(sym_count);
            for j in 0..sym_count {
                let off = j * 24;
                if off + 24 > sym_data.len() {
                    break;
                }
                let name_idx = read_u32(sym_data, off);
                // Borrow the name out of the string table and copy it straight
                // into the symbol's inline storage. Materialising a `String`
                // here first would add one heap allocation per symbol -- the
                // single largest source of allocator traffic in the linker.
                let name = match read_cstr_ref(strtab_data, name_idx as usize) {
                    // Strip the @PLT suffix some assemblers (including our own,
                    // historically) embed in the symbol name instead of using a
                    // R_X86_64_PLT32 relocation; resolution uses the base name.
                    Some(n) => SymStr::new(n.strip_suffix("@PLT").unwrap_or(n)),
                    // Non-UTF-8 name: rare but legal, take the allocating path.
                    None => {
                        let owned = read_cstr(strtab_data, name_idx as usize);
                        SymStr::new(owned.strip_suffix("@PLT").unwrap_or(&owned))
                    }
                };
                symbols.push(Elf64Symbol {
                    name_idx,
                    name,
                    info: sym_data[off + 4],
                    other: sym_data[off + 5],
                    shndx: read_u16(sym_data, off + 6),
                    value: read_u64(sym_data, off + 8),
                    size: read_u64(sym_data, off + 16),
                });
            }
            break;
        }
    }

    // Parse relocations - index by the section they apply to
    let mut relocations = vec![Vec::new(); e_shnum];
    for i in 0..sections.len() {
        if sections[i].sh_type == SHT_RELA {
            let target_sec = sections[i].info as usize;
            let rela_data: &[u8] = section_data[i].as_slice();
            let rela_count = rela_data.len() / 24; // sizeof(Elf64_Rela) = 24
            let mut relas = Vec::with_capacity(rela_count);
            for j in 0..rela_count {
                let off = j * 24;
                if off + 24 > rela_data.len() {
                    break;
                }
                let r_info = read_u64(rela_data, off + 8);
                relas.push(Elf64Rela {
                    offset: read_u64(rela_data, off),
                    sym_idx: (r_info >> 32) as u32,
                    rela_type: (r_info & 0xffffffff) as u32,
                    addend: read_i64(rela_data, off + 16),
                });
            }
            if target_sec < relocations.len() {
                relocations[target_sec] = relas;
            }
        }
    }

    Ok(Elf64Object {
        sections,
        symbols,
        section_data,
        relocations,
        source_name: source_name.to_string(),
    })
}
