//! Small DWARF line-table writer shared by the x86-family ELF assemblers.
//!
//! The compiler already lowers source locations to GAS-compatible `.file` and
//! `.loc` directives. This module turns those records into a DWARF v4
//! `.debug_line` unit, retaining relocations to the final code section so both
//! relocatable objects and linked images carry usable source line information.

use super::{ObjReloc, ObjSection, SHT_PROGBITS};

/// A source row captured while processing `.loc`.
#[derive(Clone, Debug)]
pub(crate) struct DwarfLineLocation {
    pub section_index: usize,
    pub offset: u64,
    pub file_number: u32,
    pub line: u32,
    pub column: u32,
    /// Assembly source order, used to keep rows stable when offsets tie.
    pub sequence: u64,
}

/// The final size/name of an assembler section, after branch relaxation.
pub(crate) struct DwarfSectionInfo<'a> {
    pub name: &'a str,
    pub size: u64,
    pub is_executable: bool,
}

/// Build a DWARF v4 line-table section from assembler source records.
///
/// `address_size` is the target ELF address width (4 or 8), and
/// `absolute_relocation` is the matching absolute relocation type. Every
/// line program starts with a relocated `DW_LNE_set_address` to its code
/// section; its remaining rows use section-relative address advances.
pub(crate) fn build_dwarf_line_section(
    files: &[(u32, String)],
    locations: &[DwarfLineLocation],
    sections: &[DwarfSectionInfo<'_>],
    address_size: usize,
    absolute_relocation: u32,
) -> Result<Option<ObjSection>, String> {
    if !matches!(address_size, 4 | 8) {
        return Err(format!(
            "DWARF line table address size must be 4 or 8, got {address_size}"
        ));
    }
    if files.is_empty() || locations.is_empty() {
        return Ok(None);
    }

    // DWARF file indices are one-based table positions, not the assembler's
    // `.file` numbers. Preserve first-declaration order while allowing GAS's
    // later `.file N "..."` directive to replace a filename for N.
    let mut file_table: Vec<(u32, &str)> = Vec::new();
    for (number, filename) in files {
        if filename.as_bytes().contains(&0) {
            return Err(format!(
                "DWARF source filename for .file {number} contains a NUL byte"
            ));
        }
        if let Some((_, existing)) = file_table.iter_mut().find(|(n, _)| n == number) {
            *existing = filename;
        } else {
            file_table.push((*number, filename));
        }
    }
    let file_index = |number: u32| -> Option<u64> {
        file_table
            .iter()
            .position(|(candidate, _)| *candidate == number)
            .map(|i| i as u64 + 1)
    };

    let mut rows_by_section: Vec<Vec<&DwarfLineLocation>> =
        (0..sections.len()).map(|_| Vec::new()).collect();
    for location in locations {
        let Some(section) = sections.get(location.section_index) else {
            return Err(format!(
                "internal: DWARF line record refers to missing section {}",
                location.section_index
            ));
        };
        if !section.is_executable || location.offset > section.size {
            continue;
        }
        if file_index(location.file_number).is_some() {
            rows_by_section[location.section_index].push(location);
        }
    }

    let mut data = Vec::new();
    let mut relocs = Vec::new();
    for (section_index, rows) in rows_by_section.iter_mut().enumerate() {
        if rows.is_empty() {
            continue;
        }
        rows.sort_by_key(|row| (row.offset, row.sequence));
        let code_section = &sections[section_index];
        let mut unit = Vec::new();
        unit.extend_from_slice(&[0; 4]); // unit_length, patched below
        unit.extend_from_slice(&4u16.to_le_bytes()); // DWARF version 4
        let header_length_at = unit.len();
        unit.extend_from_slice(&[0; 4]); // header_length, patched below
        let header_start = unit.len();

        // DWARF v4 line-number header.
        unit.push(1); // minimum_instruction_length
        unit.push(1); // maximum_operations_per_instruction
        unit.push(1); // default_is_stmt
        unit.push((-5i8) as u8); // line_base
        unit.push(14); // line_range
        unit.push(13); // opcode_base
        unit.extend_from_slice(&[0, 1, 1, 1, 1, 0, 0, 0, 1, 0, 0, 1]);
        unit.push(0); // empty include-directory table
        for (_, filename) in &file_table {
            unit.extend_from_slice(filename.as_bytes());
            unit.push(0);
            write_uleb128(0, &mut unit); // directory_index
            write_uleb128(0, &mut unit); // modification_time
            write_uleb128(0, &mut unit); // file_size
        }
        unit.push(0); // empty filename terminates the file table
        let header_length = unit.len() - header_start;
        let header_length = u32::try_from(header_length)
            .map_err(|_| "DWARF line-table header exceeds 32-bit length".to_string())?;
        unit[header_length_at..header_length_at + 4].copy_from_slice(&header_length.to_le_bytes());

        // Set the initial code address. The linker resolves this relocation
        // against the executable section symbol and folds in the row offset.
        unit.push(0); // DW_LNE_extended_op
        write_uleb128((address_size + 1) as u64, &mut unit);
        unit.push(2); // DW_LNE_set_address
        let address_offset = unit.len();
        unit.resize(address_offset + address_size, 0);
        let first_offset = rows[0].offset;
        let addend = i64::try_from(first_offset)
            .map_err(|_| "DWARF line-table code offset exceeds signed addend range".to_string())?;
        relocs.push(ObjReloc {
            offset: (data.len() + address_offset) as u64,
            reloc_type: absolute_relocation,
            symbol_name: code_section.name.to_string(),
            addend,
        });

        let mut address = first_offset;
        let mut current_file = 1u64;
        let mut current_line = 1i64;
        let mut current_column = 0u32;
        for row in rows.iter() {
            let next_file = file_index(row.file_number).unwrap_or(1);
            if row.offset < address {
                return Err("internal: DWARF line rows are not address ordered".to_string());
            }
            let advance = row.offset - address;
            if advance != 0 {
                unit.push(2); // DW_LNS_advance_pc
                write_uleb128(advance, &mut unit);
                address = row.offset;
            }
            if next_file != current_file {
                unit.push(4); // DW_LNS_set_file
                write_uleb128(next_file, &mut unit);
                current_file = next_file;
            }
            if row.column != current_column {
                unit.push(5); // DW_LNS_set_column
                write_uleb128(row.column as u64, &mut unit);
                current_column = row.column;
            }
            let next_line = row.line as i64;
            let line_delta = next_line - current_line;
            if line_delta != 0 {
                unit.push(3); // DW_LNS_advance_line
                write_sleb128(line_delta, &mut unit);
                current_line = next_line;
            }
            unit.push(1); // DW_LNS_copy: emit this source row
        }

        let end_offset = code_section.size;
        if end_offset < address {
            return Err(format!(
                "internal: DWARF line row at {address:#x} is past section '{}' end {end_offset:#x}",
                code_section.name
            ));
        }
        if end_offset != address {
            unit.push(2); // DW_LNS_advance_pc
            write_uleb128(end_offset - address, &mut unit);
        }
        unit.extend_from_slice(&[0, 1, 1]); // DW_LNE_end_sequence

        let unit_length = u32::try_from(unit.len() - 4)
            .map_err(|_| "DWARF line-table unit exceeds 32-bit length".to_string())?;
        unit[..4].copy_from_slice(&unit_length.to_le_bytes());
        data.extend_from_slice(&unit);
    }

    if data.is_empty() {
        return Ok(None);
    }
    Ok(Some(ObjSection {
        name: ".debug_line".to_string(),
        sh_type: SHT_PROGBITS,
        sh_flags: 0,
        sh_size: None,
        data,
        sh_addralign: 1,
        relocs,
        comdat_group: None,
    }))
}

fn write_uleb128(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn write_sleb128(mut value: i64, out: &mut Vec<u8>) {
    loop {
        let byte = (value as u8) & 0x7f;
        value >>= 7;
        let sign_bit = byte & 0x40 != 0;
        let done = (value == 0 && !sign_bit) || (value == -1 && sign_bit);
        out.push(if done { byte } else { byte | 0x80 });
        if done {
            break;
        }
    }
}
