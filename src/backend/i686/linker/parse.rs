//! ELF32 object file parsing for the i686 linker.
//!
//! Handles parsing of relocatable ELF32 .o files, regular archives (.a),
//! and thin archives. This is separate from the ELF64 parser in `linker_common`
//! because ELF32 has different field widths (u32 vs u64 for addresses/offsets).

use crate::common::fx_hash::FxHashMap;

use super::types::*;

/// Return a checked byte range from an ELF input.  Offsets and sizes are
/// attacker-controlled u32 values, so addition must happen in checked usize
/// arithmetic rather than wrapping before the slice operation.
fn elf_range<'a>(
    data: &'a [u8],
    offset: u32,
    size: u32,
    filename: &str,
    what: &str,
) -> Result<&'a [u8], String> {
    let start = offset as usize;
    let end = start
        .checked_add(size as usize)
        .ok_or_else(|| format!("{}: {} range overflows", filename, what))?;
    data.get(start..end)
        .ok_or_else(|| format!("{}: {} extends past end of file", filename, what))
}

/// Decode the in-place addend carried by an Elf32_Rel relocation.  i386 uses
/// one-, two-, and four-byte relocation fields; reading every addend as i32
/// corrupts R_386_{,PC}{8,16} and can read beyond a valid section.
fn rel_addend(
    section: &[u8],
    offset: u32,
    rel_type: u32,
    filename: &str,
    section_name: &str,
) -> Result<i32, String> {
    if rel_type == R_386_NONE {
        return Ok(0);
    }
    let off = offset as usize;
    let (width, signed) = match rel_type {
        R_386_8 | R_386_PC8 => (1usize, true),
        R_386_16 | R_386_PC16 => (2usize, true),
        // All remaining currently supported i386 relocations have 32-bit
        // fields. Unknown types are decoded as 32-bit so the relocation stage
        // can issue its precise "unsupported relocation" diagnostic.
        _ => (4usize, true),
    };
    let bytes = section.get(off..off + width).ok_or_else(|| {
        format!(
            "{}: relocation field at {:#x} extends past section '{}'",
            filename, offset, section_name
        )
    })?;
    let value = match (width, signed) {
        (1, true) => i8::from_le_bytes([bytes[0]]) as i32,
        (2, true) => i16::from_le_bytes([bytes[0], bytes[1]]) as i32,
        (4, true) => i32::from_le_bytes(bytes.try_into().unwrap()),
        _ => unreachable!(),
    };
    Ok(value)
}

/// Parse an ELF32 relocatable object file.
pub(super) fn parse_elf32(data: &[u8], filename: &str) -> Result<InputObject, String> {
    if data.len() < 52 {
        return Err(format!("{}: too small for ELF header", filename));
    }
    if data[0..4] != ELF_MAGIC {
        return Err(format!("{}: not an ELF file", filename));
    }
    if data[4] != ELFCLASS32 {
        return Err(format!("{}: not ELF32", filename));
    }
    if data[5] != ELFDATA2LSB {
        return Err(format!("{}: not little-endian", filename));
    }
    let e_type = read_u16(data, 16);
    if e_type != ET_REL {
        return Err(format!(
            "{}: not a relocatable object (type={})",
            filename, e_type
        ));
    }
    let e_machine = read_u16(data, 18);
    if e_machine != EM_386 {
        return Err(format!("{}: not i386 (machine={})", filename, e_machine));
    }

    let e_shoff = read_u32(data, 32) as usize;
    let e_shentsize = read_u16(data, 46) as usize;
    let e_shnum = read_u16(data, 48) as usize;
    let e_shstrndx = read_u16(data, 50) as usize;
    if e_shentsize < 40 {
        return Err(format!(
            "{}: bogus e_shentsize {} (need >= 40)",
            filename, e_shentsize
        ));
    }
    if e_shnum == 0 {
        return Err(format!("{}: no section headers", filename));
    }
    let table_size = e_shnum
        .checked_mul(e_shentsize)
        .ok_or_else(|| format!("{}: section-header table size overflows", filename))?;
    let table_end = e_shoff
        .checked_add(table_size)
        .ok_or_else(|| format!("{}: section-header table offset overflows", filename))?;
    if table_end > data.len() {
        return Err(format!(
            "{}: section-header table extends past end of file",
            filename
        ));
    }
    if e_shstrndx >= e_shnum {
        return Err(format!("{}: invalid e_shstrndx {}", filename, e_shstrndx));
    }

    let mut shdrs = Vec::with_capacity(e_shnum);
    for i in 0..e_shnum {
        let off = e_shoff + i * e_shentsize;
        let addralign = read_u32(data, off + 32);
        if addralign > 1 && !addralign.is_power_of_two() {
            return Err(format!(
                "{}: section header {} has invalid sh_addralign 0x{:x} \
                 (must be 0 or a power of two)",
                filename, i, addralign
            ));
        }
        shdrs.push(Elf32Shdr {
            name: read_u32(data, off),
            sh_type: read_u32(data, off + 4),
            flags: read_u32(data, off + 8),
            addr: read_u32(data, off + 12),
            offset: read_u32(data, off + 16),
            size: read_u32(data, off + 20),
            link: read_u32(data, off + 24),
            info: read_u32(data, off + 28),
            addralign,
            entsize: read_u32(data, off + 36),
        });
    }

    let shstrtab_hdr = &shdrs[e_shstrndx];
    let shstrtab_data = elf_range(
        data,
        shstrtab_hdr.offset,
        shstrtab_hdr.size,
        filename,
        "section-name string table",
    )?;

    let mut symtab_idx = None;
    let mut strtab_data: &[u8] = &[];
    for (i, shdr) in shdrs.iter().enumerate() {
        if shdr.sh_type != SHT_SYMTAB {
            continue;
        }
        if shdr.entsize < 16 {
            return Err(format!(
                "{}: symbol table has invalid entsize {}",
                filename, shdr.entsize
            ));
        }
        let str_idx = shdr.link as usize;
        let str_shdr = shdrs.get(str_idx).ok_or_else(|| {
            format!(
                "{}: symbol table links invalid section {}",
                filename, str_idx
            )
        })?;
        strtab_data = elf_range(
            data,
            str_shdr.offset,
            str_shdr.size,
            filename,
            "symbol string table",
        )?;
        symtab_idx = Some(i);
        break;
    }

    let mut symbols = Vec::new();
    if let Some(si) = symtab_idx {
        let sym_shdr = &shdrs[si];
        let sym_data = elf_range(
            data,
            sym_shdr.offset,
            sym_shdr.size,
            filename,
            "symbol table",
        )?;
        let entsize = sym_shdr.entsize as usize;
        let sym_count = sym_data.len() / entsize;
        symbols.reserve(sym_count);
        for j in 0..sym_count {
            let entry = &sym_data[j * entsize..j * entsize + 16];
            let st_name = read_u32(entry, 0);
            let st_value = read_u32(entry, 4);
            let st_size = read_u32(entry, 8);
            let st_info = entry[12];
            let st_other = entry[13];
            let st_shndx = read_u16(entry, 14);
            let mut sym_name = read_cstr(strtab_data, st_name as usize);
            if sym_name.ends_with("@PLT") {
                sym_name.truncate(sym_name.len() - 4);
            }
            symbols.push(InputSymbol {
                name: sym_name,
                value: st_value,
                size: st_size,
                binding: st_info >> 4,
                sym_type: st_info & 0xf,
                visibility: st_other & 3,
                section_index: st_shndx,
            });
        }
    }

    // Build relocation map: target section index -> REL section indices.
    let mut rel_map: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
    for (i, shdr) in shdrs.iter().enumerate() {
        if shdr.sh_type == SHT_REL {
            if shdr.entsize != 0 && shdr.entsize < 8 {
                return Err(format!(
                    "{}: relocation section {} has invalid entsize {}",
                    filename, i, shdr.entsize
                ));
            }
            if shdr.info as usize >= shdrs.len() {
                return Err(format!(
                    "{}: relocation section {} targets invalid section {}",
                    filename, i, shdr.info
                ));
            }
            rel_map.entry(shdr.info as usize).or_default().push(i);
        }
    }

    let mut sections = Vec::with_capacity(e_shnum);
    for (i, shdr) in shdrs.iter().enumerate() {
        let sec_name = read_cstr(shstrtab_data, shdr.name as usize);
        let sec_data = if shdr.sh_type != SHT_NOBITS && shdr.size > 0 {
            elf_range(
                data,
                shdr.offset,
                shdr.size,
                filename,
                &format!("section '{}' data", sec_name),
            )?
            .to_vec()
        } else {
            vec![0u8; shdr.size as usize]
        };

        let mut relocs = Vec::new();
        if let Some(rel_indices) = rel_map.get(&i) {
            for &ri in rel_indices {
                let rel_shdr = &shdrs[ri];
                let rel_data = elf_range(
                    data,
                    rel_shdr.offset,
                    rel_shdr.size,
                    filename,
                    "relocation section",
                )?;
                let entsize = rel_shdr.entsize.max(8) as usize;
                for entry in rel_data.chunks_exact(entsize) {
                    let r_offset = read_u32(entry, 0);
                    let r_info = read_u32(entry, 4);
                    let sym_idx = r_info >> 8;
                    let rel_type = r_info & 0xff;
                    let addend = rel_addend(&sec_data, r_offset, rel_type, filename, &sec_name)?;
                    relocs.push((r_offset, rel_type, sym_idx, addend));
                }
            }
        }

        sections.push(InputSection {
            name: sec_name,
            sh_type: shdr.sh_type,
            flags: shdr.flags,
            data: sec_data,
            align: shdr.addralign.max(1),
            relocations: relocs,
            input_index: i,
            entsize: shdr.entsize,
            link: shdr.link,
            info: shdr.info,
        });
    }

    Ok(InputObject {
        sections,
        symbols,
        filename: filename.to_string(),
    })
}

/// Parse a regular (.a) archive, returning ELF32 members.
pub(super) fn parse_archive(
    data: &[u8],
    _filename: &str,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let raw_members = parse_archive_members(data)?;
    let mut members = Vec::new();
    for (name, offset, size) in raw_members {
        let content = &data[offset..offset + size];
        if content.len() >= 4 && content[0..4] == ELF_MAGIC {
            members.push((name, content.to_vec()));
        }
    }
    Ok(members)
}

/// Parse a GNU thin archive, reading member data from external files.
pub(super) fn parse_thin_archive_i686(
    data: &[u8],
    archive_path: &str,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let member_names = parse_thin_archive_members(data)?;
    let archive_dir = std::path::Path::new(archive_path)
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let mut members = Vec::new();
    for name in member_names {
        let member_path = archive_dir.join(&name);
        let content = std::fs::read(&member_path).map_err(|e| {
            format!(
                "thin archive {}: failed to read member '{}': {}",
                archive_path,
                member_path.display(),
                e
            )
        })?;
        if content.len() >= 4 && content[0..4] == ELF_MAGIC {
            members.push((name, content));
        }
    }
    Ok(members)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implicit_rel_addends_use_the_relocation_field_width() {
        let bytes = [0x80, 0x34, 0x80, 0, 0, 0, 0, 0];
        assert_eq!(
            rel_addend(&bytes, 0, R_386_PC8, "x.o", ".text").unwrap(),
            -128
        );
        assert_eq!(
            rel_addend(&bytes, 1, R_386_PC16, "x.o", ".text").unwrap(),
            -32716
        );
        assert_eq!(
            rel_addend(&bytes, 3, R_386_PC32, "x.o", ".text").unwrap(),
            0
        );
    }

    #[test]
    fn truncated_relocation_field_is_diagnostic_not_a_panic() {
        let err = rel_addend(&[0; 2], 1, R_386_16, "bad.o", ".text").unwrap_err();
        assert!(err.contains("extends past section '.text'"), "{err}");
    }

    #[test]
    fn short_elf_header_is_diagnostic_not_a_panic() {
        let err = parse_elf32(&[0; 16], "bad.o").err().expect("must reject");
        assert!(err.contains("too small"), "{err}");
    }
}
