//! Small DWARF line-table writer shared by the ELF assemblers.
//!
//! The compiler already lowers source locations to GAS-compatible `.file` and
//! `.loc` directives. This module turns those records into a DWARF v4
//! `.debug_line` unit, retaining relocations to the final code section so both
//! relocatable objects and linked images carry usable source line information.
//!
//! Like GAS (`dwarf2_finish`), an object whose source carries `.loc` rows but
//! no `.debug_info` of its own also gets a synthesized compile unit per line
//! program — `DW_AT_stmt_list` pointing at it, the code range, name, directory,
//! producer and language — with one `DW_TAG_subprogram` per sized function
//! symbol. Debuggers reach a line table only through such a unit: without it
//! gdb reports "No symbol table is loaded" and cannot break at `file:line`.
//!
//! Code addresses are normally relocated against the code section's symbol
//! plus an offset.  A target whose linker deletes code bytes (RISC-V linker
//! relaxation) passes a `symbol_at` callback instead: then every address --
//! each row's, each sequence end, each `DW_AT_low_pc`/`DW_AT_high_pc` -- is
//! relocated against a label at it, which the linker moves with the code
//! (GNU as: `DWARF2_USE_FIXED_ADVANCE_PC` under `linkrelax`).  A row then
//! costs a `DW_LNE_set_address` instead of an address advance, and
//! `DW_AT_high_pc` is an address, not a length.

use super::{ObjReloc, ObjSection, SHT_PROGBITS, parse_string_literal};

/// Names the symbol at (section index, offset): see the module documentation.
/// The callback's own lifetime `'f` is separate from the borrow `'a`, so one
/// callback can be lent to several builders in turn.
pub(crate) type SymbolAt<'a, 'f> = Option<&'a mut (dyn FnMut(usize, u64) -> String + 'f)>;

/// `.file` operands: `N "name"`, or GNU as's `N "dir" "name" [md5 VALUE]`
/// (the two joined unless `name` is absolute).  `None` for the number-less
/// `.file "name"`, which names the object's source file but no line-table
/// file.
pub(crate) fn parse_file_directive(args: &str) -> Result<Option<(u32, String)>, String> {
    let args = args.trim();
    let number_end = args
        .find(|c: char| c.is_whitespace() || c == '"')
        .unwrap_or(args.len());
    if number_end == 0 {
        return Ok(None);
    }
    let (number, rest) = args.split_at(number_end);
    let number: u32 = number
        .parse()
        .map_err(|_| format!("invalid .file number: {number}"))?;
    let (first, rest) = string_operand(rest)?;
    let rest = rest.trim_start();
    let name = if rest.starts_with('"') {
        let (file, _) = string_operand(rest)?;
        if file.starts_with('/') || first.is_empty() {
            file
        } else {
            format!("{}/{}", first.trim_end_matches('/'), file)
        }
    } else {
        first
    };
    Ok(Some((number, name)))
}

/// A leading string literal, unescaped, and the text after it.
fn string_operand(s: &str) -> Result<(String, &str), String> {
    let s = s.trim_start();
    let mut escaped = false;
    let end = s
        .char_indices()
        .skip(1)
        .find(|&(_, c)| {
            let close = !escaped && c == '"';
            escaped = !escaped && c == '\\';
            close
        })
        .map(|(i, _)| i)
        .ok_or_else(|| format!("expected a string literal: {s}"))?;
    let bytes = parse_string_literal(&s[..=end])?;
    Ok((String::from_utf8_lossy(&bytes).into_owned(), &s[end + 1..]))
}

/// Row attributes carried by a GAS `.loc`. `is_stmt` and `isa` persist
/// until changed; the other fields apply only to the next emitted row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DwarfLocOptions {
    pub is_stmt: Option<bool>,
    pub isa: Option<u64>,
    pub basic_block: bool,
    pub prologue_end: bool,
    pub epilogue_begin: bool,
    pub discriminator: Option<u64>,
}

/// Parse `.loc FILE LINE [COLUMN] [OPTIONS]`. GNU view annotations are not
/// yet represented; retain compatibility with the previously ignored option.
pub(crate) fn parse_loc_directive(args: &str) -> Result<(u32, u32, u32, DwarfLocOptions), String> {
    let mut words = args.split_whitespace().peekable();
    let invalid = || format!("invalid .loc directive: .loc {args}");
    let file = words
        .next()
        .and_then(|w| w.parse().ok())
        .ok_or_else(invalid)?;
    let line = words
        .next()
        .and_then(|w| w.parse().ok())
        .ok_or_else(invalid)?;
    let column = if words.peek().is_some_and(|w| w.parse::<u32>().is_ok()) {
        words
            .next()
            .and_then(|w| w.parse().ok())
            .ok_or_else(invalid)?
    } else {
        0
    };
    let mut options = DwarfLocOptions::default();
    while let Some(word) = words.next() {
        match word {
            "basic_block" => options.basic_block = true,
            "prologue_end" => options.prologue_end = true,
            "epilogue_begin" => options.epilogue_begin = true,
            "is_stmt" => match words.next() {
                Some("0") => options.is_stmt = Some(false),
                Some("1") => options.is_stmt = Some(true),
                _ => return Err(invalid()),
            },
            "isa" => {
                options.isa = Some(
                    words
                        .next()
                        .and_then(|w| w.parse().ok())
                        .ok_or_else(invalid)?,
                )
            }
            "discriminator" => {
                options.discriminator = Some(
                    words
                        .next()
                        .and_then(|w| w.parse().ok())
                        .ok_or_else(invalid)?,
                )
            }
            "view" => {
                words.next().ok_or_else(invalid)?;
            }
            _ => return Err(invalid()),
        }
    }
    Ok((file, line, column, options))
}

/// `DW_LNE_set_address`, relocated against `symbol` + `addend`; `base` is
/// where `unit` will start in the section.
fn push_set_address(
    unit: &mut Vec<u8>,
    relocs: &mut Vec<ObjReloc>,
    base: usize,
    address_size: usize,
    reloc_type: u32,
    symbol_name: String,
    addend: i64,
) {
    unit.push(0); // DW_LNE_extended_op
    write_uleb128((address_size + 1) as u64, unit);
    unit.push(2); // DW_LNE_set_address
    relocs.push(ObjReloc {
        offset: (base + unit.len()) as u64,
        reloc_type,
        symbol_name,
        addend,
    });
    unit.resize(unit.len() + address_size, 0);
}

/// A source row captured while processing `.loc`.
#[derive(Clone, Debug)]
pub(crate) struct DwarfLineLocation {
    pub section_index: usize,
    pub offset: u64,
    pub file_number: u32,
    pub line: u32,
    pub column: u32,
    pub options: DwarfLocOptions,
    /// Assembly source order, used to keep rows stable when offsets tie.
    pub sequence: u64,
}

/// One line program in a built `.debug_line`: its byte offset and the code
/// section its rows describe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DwarfLineUnit {
    pub offset: u64,
    pub section_index: usize,
}

/// A sized function symbol, for its `DW_TAG_subprogram`.
pub(crate) struct DwarfFunction<'a> {
    pub name: &'a str,
    pub section_index: usize,
    pub offset: u64,
    pub size: u64,
    pub external: bool,
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
/// section; its remaining rows use section-relative address advances, or,
/// given `symbol_at`, a relocated `DW_LNE_set_address` each.
pub(crate) fn build_dwarf_line_section(
    files: &[(u32, String)],
    locations: &[DwarfLineLocation],
    sections: &[DwarfSectionInfo<'_>],
    address_size: usize,
    absolute_relocation: u32,
    mut symbol_at: SymbolAt<'_, '_>,
) -> Result<Option<(ObjSection, Vec<DwarfLineUnit>)>, String> {
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

    // GAS carries is_stmt and ISA across section switches. Resolve those
    // persistent states in source order before sorting rows into per-section
    // line programs; each program itself starts with DWARF defaults.
    let mut rows_by_section: Vec<Vec<(&DwarfLineLocation, bool, u64)>> =
        (0..sections.len()).map(|_| Vec::new()).collect();
    let mut source_is_stmt = true;
    let mut source_isa = 0;
    for location in locations {
        if let Some(value) = location.options.is_stmt {
            source_is_stmt = value;
        }
        if let Some(value) = location.options.isa {
            source_isa = value;
        }
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
            rows_by_section[location.section_index].push((location, source_is_stmt, source_isa));
        }
    }

    let mut data = Vec::new();
    let mut relocs = Vec::new();
    let mut units = Vec::new();
    for (section_index, rows) in rows_by_section.iter_mut().enumerate() {
        if rows.is_empty() {
            continue;
        }
        units.push(DwarfLineUnit {
            offset: data.len() as u64,
            section_index,
        });
        rows.sort_by_key(|(row, _, _)| (row.offset, row.sequence));
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
        let first_offset = rows[0].0.offset;
        let (symbol_name, addend) = match symbol_at.as_deref_mut() {
            Some(symbol_at) => (symbol_at(section_index, first_offset), 0),
            None => (
                code_section.name.to_string(),
                i64::try_from(first_offset).map_err(|_| {
                    "DWARF line-table code offset exceeds signed addend range".to_string()
                })?,
            ),
        };
        push_set_address(
            &mut unit,
            &mut relocs,
            data.len(),
            address_size,
            absolute_relocation,
            symbol_name,
            addend,
        );

        let mut address = first_offset;
        let mut current_file = 1u64;
        let mut current_line = 1i64;
        let mut current_column = 0u32;
        let mut current_is_stmt = true;
        let mut current_isa = 0u64;
        for &(row, is_stmt, isa) in rows.iter() {
            let next_file = file_index(row.file_number).unwrap_or(1);
            if row.offset < address {
                return Err("internal: DWARF line rows are not address ordered".to_string());
            }
            let advance = row.offset - address;
            let next_line = row.line as i64;
            let line_delta = next_line - current_line;
            // Special opcodes both advance the machine and emit the row in
            // one byte. Relocatable per-row addresses (used for linker
            // relaxation) must keep their set_address relocations instead.
            let special = if symbol_at.is_none() {
                line_special_opcode(advance, line_delta)
            } else {
                None
            };
            if advance != 0 && special.is_none() {
                if let Some(symbol_at) = symbol_at.as_deref_mut() {
                    let symbol_name = symbol_at(section_index, row.offset);
                    push_set_address(
                        &mut unit,
                        &mut relocs,
                        data.len(),
                        address_size,
                        absolute_relocation,
                        symbol_name,
                        0,
                    );
                } else {
                    unit.push(2); // DW_LNS_advance_pc
                    write_uleb128(advance, &mut unit);
                }
            }
            address = row.offset;
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
            if line_delta != 0 && special.is_none() {
                unit.push(3); // DW_LNS_advance_line
                write_sleb128(line_delta, &mut unit);
            }
            current_line = next_line;
            if is_stmt != current_is_stmt {
                unit.push(6); // DW_LNS_negate_stmt
                current_is_stmt = is_stmt;
            }
            if isa != current_isa {
                unit.push(12); // DW_LNS_set_isa
                write_uleb128(isa, &mut unit);
                current_isa = isa;
            }
            if row.options.basic_block {
                unit.push(7);
            }
            if row.options.prologue_end {
                unit.push(10);
            }
            if row.options.epilogue_begin {
                unit.push(11);
            }
            if let Some(discriminator) = row.options.discriminator {
                // ULEB128 needs at least one byte, even for zero. Write it
                // directly into the unit without a per-row allocation.
                let bits = (u64::BITS - discriminator.leading_zeros()).max(1);
                let operand_len = bits.div_ceil(7);
                unit.push(0); // DW_LNE_extended_op
                write_uleb128(1 + u64::from(operand_len), &mut unit);
                unit.push(4); // DW_LNE_set_discriminator
                write_uleb128(discriminator, &mut unit);
            }
            // A special opcode emits the row; otherwise copy does so.
            unit.push(special.unwrap_or(1));
        }

        let end_offset = code_section.size;
        if end_offset < address {
            return Err(format!(
                "internal: DWARF line row at {address:#x} is past section '{}' end {end_offset:#x}",
                code_section.name
            ));
        }
        if end_offset != address {
            if let Some(symbol_at) = symbol_at.as_deref_mut() {
                let symbol_name = symbol_at(section_index, end_offset);
                push_set_address(
                    &mut unit,
                    &mut relocs,
                    data.len(),
                    address_size,
                    absolute_relocation,
                    symbol_name,
                    0,
                );
            } else {
                unit.push(2); // DW_LNS_advance_pc
                write_uleb128(end_offset - address, &mut unit);
            }
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
    Ok(Some((debug_section(".debug_line", data, relocs), units)))
}

fn debug_section(name: &str, data: Vec<u8>, relocs: Vec<ObjReloc>) -> ObjSection {
    ObjSection {
        name: name.to_string(),
        sh_type: SHT_PROGBITS,
        sh_flags: 0,
        sh_size: None,
        data,
        sh_addralign: 1,
        relocs,
        comdat_group: None,
    }
}

const DW_TAG_COMPILE_UNIT: u64 = 0x11;
const DW_TAG_SUBPROGRAM: u64 = 0x2e;
const DW_TAG_UNSPECIFIED_TYPE: u64 = 0x3b;
const DW_AT_NAME: u64 = 0x03;
const DW_AT_STMT_LIST: u64 = 0x10;
const DW_AT_LOW_PC: u64 = 0x11;
const DW_AT_HIGH_PC: u64 = 0x12;
const DW_AT_LANGUAGE: u64 = 0x13;
const DW_AT_COMP_DIR: u64 = 0x1b;
const DW_AT_PRODUCER: u64 = 0x25;
const DW_AT_EXTERNAL: u64 = 0x3f;
const DW_AT_TYPE: u64 = 0x49;
const DW_FORM_ADDR: u64 = 0x01;
const DW_FORM_DATA2: u64 = 0x05;
const DW_FORM_DATA4: u64 = 0x06;
const DW_FORM_STRING: u64 = 0x08;
const DW_FORM_FLAG: u64 = 0x0c;
const DW_FORM_REF4: u64 = 0x13;
const DW_FORM_SEC_OFFSET: u64 = 0x17;
/// `DW_LANG_C99`, the newest C language code DWARF 4 defines.
const DW_LANG_C99: u16 = 0x000c;
/// `DW_LANG_Mips_Assembler`, what GAS records for assembly sources.
const DW_LANG_MIPS_ASSEMBLER: u16 = 0x8001;

/// Build `.debug_info` and `.debug_abbrev` for the line programs `units` of
/// a `.debug_line` this assembler synthesized (GAS `dwarf2_finish` parity in
/// DWARF 4 forms, matching the v4 line table).
///
/// Each unit gets one `DW_TAG_compile_unit` spanning its code section, with
/// a `DW_TAG_subprogram` per sized function symbol in that section (the
/// unspecified return type GAS uses, so a debugger never assumes `int`). The
/// language is C for a `.c`/`.h`/`.i` primary file and GAS's assembler code
/// otherwise. `abs32` relocates the section offsets (`DW_FORM_sec_offset`,
/// the abbreviation offset) and `abs_addr` the `address_size`-byte code
/// addresses, so every value stays correct after the linker concatenates
/// objects.  Given `symbol_at`, code addresses are relocated against labels
/// and `DW_AT_high_pc` is an address (see the module documentation).
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_dwarf_info_sections(
    units: &[DwarfLineUnit],
    sections: &[DwarfSectionInfo<'_>],
    functions: &[DwarfFunction<'_>],
    primary_file: &str,
    comp_dir: &str,
    producer: &str,
    address_size: usize,
    abs32: u32,
    abs_addr: u32,
    mut symbol_at: SymbolAt<'_, '_>,
) -> Result<Option<(ObjSection, ObjSection)>, String> {
    if units.is_empty() {
        return Ok(None);
    }
    if !matches!(address_size, 4 | 8) {
        return Err(format!(
            "DWARF address size must be 4 or 8, got {address_size}"
        ));
    }
    for text in [primary_file, comp_dir, producer] {
        if text.as_bytes().contains(&0) {
            return Err(format!("DWARF string contains a NUL byte: {text:?}"));
        }
    }
    let language = match primary_file.rsplit_once('.').map(|(_, ext)| ext) {
        Some("c" | "h" | "i") => DW_LANG_C99,
        _ => DW_LANG_MIPS_ASSEMBLER,
    };

    let high_pc_form = if symbol_at.is_some() {
        DW_FORM_ADDR
    } else {
        DW_FORM_DATA4
    };
    let mut abbrev = Vec::new();
    let mut declare = |code: u64, tag: u64, children: bool, attrs: &[(u64, u64)]| {
        write_uleb128(code, &mut abbrev);
        write_uleb128(tag, &mut abbrev);
        abbrev.push(u8::from(children));
        for &(at, form) in attrs {
            write_uleb128(at, &mut abbrev);
            write_uleb128(form, &mut abbrev);
        }
        abbrev.extend_from_slice(&[0, 0]);
    };
    declare(
        1,
        DW_TAG_COMPILE_UNIT,
        true,
        &[
            (DW_AT_STMT_LIST, DW_FORM_SEC_OFFSET),
            (DW_AT_LOW_PC, DW_FORM_ADDR),
            (DW_AT_HIGH_PC, high_pc_form),
            (DW_AT_NAME, DW_FORM_STRING),
            (DW_AT_COMP_DIR, DW_FORM_STRING),
            (DW_AT_PRODUCER, DW_FORM_STRING),
            (DW_AT_LANGUAGE, DW_FORM_DATA2),
        ],
    );
    declare(2, DW_TAG_UNSPECIFIED_TYPE, false, &[]);
    declare(
        3,
        DW_TAG_SUBPROGRAM,
        false,
        &[
            (DW_AT_NAME, DW_FORM_STRING),
            (DW_AT_EXTERNAL, DW_FORM_FLAG),
            (DW_AT_TYPE, DW_FORM_REF4),
            (DW_AT_LOW_PC, DW_FORM_ADDR),
            (DW_AT_HIGH_PC, high_pc_form),
        ],
    );
    abbrev.push(0);

    let mut info = Vec::new();
    let mut relocs = Vec::new();
    let mut reloc =
        |info: &mut Vec<u8>, reloc_type: u32, size: usize, symbol: &str, addend: u64| {
            let addend = i64::try_from(addend)
                .map_err(|_| "DWARF relocation addend exceeds signed range".to_string())?;
            relocs.push(ObjReloc {
                offset: info.len() as u64,
                reloc_type,
                symbol_name: symbol.to_string(),
                addend,
            });
            info.resize(info.len() + size, 0);
            Ok::<(), String>(())
        };
    let length32 = |value: u64, what: &str| {
        u32::try_from(value).map_err(|_| format!("DWARF {what} exceeds 32 bits"))
    };
    for unit in units {
        let code = sections.get(unit.section_index).ok_or_else(|| {
            format!(
                "internal: DWARF line unit refers to missing section {}",
                unit.section_index
            )
        })?;
        let start = info.len();
        info.extend_from_slice(&[0; 4]); // unit_length, patched below
        info.extend_from_slice(&4u16.to_le_bytes());
        reloc(&mut info, abs32, 4, ".debug_abbrev", 0)?;
        info.push(address_size as u8);

        write_uleb128(1, &mut info);
        reloc(&mut info, abs32, 4, ".debug_line", unit.offset)?;
        reloc(&mut info, abs_addr, address_size, code.name, 0)?;
        if let Some(symbol_at) = symbol_at.as_deref_mut() {
            let end = symbol_at(unit.section_index, code.size);
            reloc(&mut info, abs_addr, address_size, &end, 0)?;
        } else {
            info.extend_from_slice(&length32(code.size, "code section size")?.to_le_bytes());
        }
        for text in [primary_file, comp_dir, producer] {
            info.extend_from_slice(text.as_bytes());
            info.push(0);
        }
        info.extend_from_slice(&language.to_le_bytes());

        let void_die = length32((info.len() - start) as u64, "unit")?;
        write_uleb128(2, &mut info);

        let mut members: Vec<&DwarfFunction<'_>> = functions
            .iter()
            .filter(|f| f.section_index == unit.section_index && f.size != 0)
            .filter(|f| {
                f.offset
                    .checked_add(f.size)
                    .is_some_and(|end| end <= code.size)
            })
            .collect();
        members.sort_by(|a, b| (a.offset, a.name).cmp(&(b.offset, b.name)));
        for function in members {
            if function.name.as_bytes().contains(&0) {
                return Err(format!(
                    "DWARF function name contains a NUL byte: {:?}",
                    function.name
                ));
            }
            write_uleb128(3, &mut info);
            info.extend_from_slice(function.name.as_bytes());
            info.push(0);
            info.push(u8::from(function.external));
            info.extend_from_slice(&void_die.to_le_bytes());
            if let Some(symbol_at) = symbol_at.as_deref_mut() {
                let start = symbol_at(unit.section_index, function.offset);
                let end = symbol_at(unit.section_index, function.offset + function.size);
                reloc(&mut info, abs_addr, address_size, &start, 0)?;
                reloc(&mut info, abs_addr, address_size, &end, 0)?;
            } else {
                reloc(
                    &mut info,
                    abs_addr,
                    address_size,
                    code.name,
                    function.offset,
                )?;
                info.extend_from_slice(&length32(function.size, "function size")?.to_le_bytes());
            }
        }
        info.push(0); // end of the unit's children

        let unit_length = length32((info.len() - start - 4) as u64, "unit")?;
        info[start..start + 4].copy_from_slice(&unit_length.to_le_bytes());
    }
    Ok(Some((
        debug_section(".debug_info", info, relocs),
        debug_section(".debug_abbrev", abbrev, Vec::new()),
    )))
}

/// Encode the v4 header's min-instruction-length=1, line-base=-5,
/// line-range=14 and opcode-base=13. Return None when the pair does not fit.
fn line_special_opcode(address_delta: u64, line_delta: i64) -> Option<u8> {
    if !(-5..=8).contains(&line_delta) {
        return None;
    }
    address_delta
        .checked_mul(14)?
        .checked_add((line_delta + 5) as u64)?
        .checked_add(13)
        .and_then(|value| u8::try_from(value).ok())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_special_opcode_boundaries_and_overflow() {
        assert_eq!(line_special_opcode(0, -5), Some(13));
        assert_eq!(line_special_opcode(0, 8), Some(26));
        assert_eq!(line_special_opcode(17, -5), Some(251));
        assert_eq!(line_special_opcode(17, -1), Some(255));
        assert_eq!(line_special_opcode(17, 0), None);
        assert_eq!(line_special_opcode(18, -5), None);
        assert_eq!(line_special_opcode(0, -6), None);
        assert_eq!(line_special_opcode(0, 9), None);
        assert_eq!(line_special_opcode(u64::MAX, 0), None);
    }

    const ABS32: u32 = 10; // R_X86_64_32
    const ABS64: u32 = 1; // R_X86_64_64

    fn text_section() -> Vec<DwarfSectionInfo<'static>> {
        vec![DwarfSectionInfo {
            name: ".text",
            size: 0x40,
            is_executable: true,
        }]
    }

    #[test]
    fn a_synthesized_unit_points_at_its_line_program_and_code() {
        let files = [(1, "b.c".to_string())];
        let rows = [DwarfLineLocation {
            section_index: 0,
            offset: 0,
            file_number: 1,
            line: 2,
            column: 5,
            options: DwarfLocOptions::default(),
            sequence: 0,
        }];
        let (_, units) = build_dwarf_line_section(&files, &rows, &text_section(), 8, ABS64, None)
            .unwrap()
            .unwrap();
        assert_eq!(
            units,
            [DwarfLineUnit {
                offset: 0,
                section_index: 0
            }]
        );
        let functions = [
            DwarfFunction {
                name: "helper",
                section_index: 0,
                offset: 0x10,
                size: 0x30,
                external: true,
            },
            // Unsized and out-of-range symbols get no subprogram.
            DwarfFunction {
                name: "nosize",
                section_index: 0,
                offset: 0,
                size: 0,
                external: false,
            },
            DwarfFunction {
                name: "past_end",
                section_index: 0,
                offset: 0x30,
                size: 0x20,
                external: false,
            },
        ];
        let (info, abbrev) = build_dwarf_info_sections(
            &units,
            &text_section(),
            &functions,
            "b.c",
            "/src",
            "LCCC test",
            8,
            ABS32,
            ABS64,
            None,
        )
        .unwrap()
        .unwrap();
        let d = &info.data;
        let unit_length = u32::from_le_bytes(d[..4].try_into().unwrap()) as usize;
        assert_eq!(unit_length + 4, d.len(), "one unit spans the section");
        assert_eq!(u16::from_le_bytes([d[4], d[5]]), 4, "DWARF 4");
        assert_eq!(d[10], 8, "address size");
        let relocs: Vec<_> = info
            .relocs
            .iter()
            .map(|r| (r.offset, r.reloc_type, r.symbol_name.as_str(), r.addend))
            .collect();
        assert_eq!(relocs[0], (6, ABS32, ".debug_abbrev", 0));
        assert_eq!(relocs[1], (12, ABS32, ".debug_line", 0));
        assert_eq!(relocs[2], (16, ABS64, ".text", 0));
        assert_eq!(
            relocs.len(),
            4,
            "only `helper` gets a subprogram: {relocs:?}"
        );
        assert_eq!(
            (relocs[3].1, relocs[3].2, relocs[3].3),
            (ABS64, ".text", 0x10)
        );
        let text = String::from_utf8_lossy(d);
        assert!(text.contains("b.c\0/src\0LCCC test\0"), "{text:?}");
        assert!(
            text.contains("helper\0") && !text.contains("nosize") && !text.contains("past_end")
        );
        let language_at = d.windows(10).position(|w| w == b"LCCC test\0").unwrap() + 10;
        assert_eq!(
            u16::from_le_bytes([d[language_at], d[language_at + 1]]),
            DW_LANG_C99
        );
        assert_eq!(
            *abbrev.data.last().unwrap(),
            0,
            "abbreviation table terminated"
        );
        assert!(abbrev.relocs.is_empty());
    }

    #[test]
    fn an_assembly_source_records_the_assembler_language() {
        let units = [DwarfLineUnit {
            offset: 0,
            section_index: 0,
        }];
        let (info, _) = build_dwarf_info_sections(
            &units,
            &text_section(),
            &[],
            "start.S",
            "/",
            "p",
            4,
            1,
            1,
            None,
        )
        .unwrap()
        .unwrap();
        let d = &info.data;
        let at = d.windows(2).position(|w| w == b"p\0").unwrap() + 2;
        assert_eq!(
            u16::from_le_bytes([d[at], d[at + 1]]),
            DW_LANG_MIPS_ASSEMBLER
        );
        assert_eq!(d[10], 4, "i686 address size");
    }

    #[test]
    fn file_and_loc_operands_parse_like_gnu_as() {
        assert_eq!(
            parse_file_directive("1 \"a.c\"").unwrap(),
            Some((1, "a.c".to_string()))
        );
        // The escapes the compiler writes (`escape_dwarf_path`) come back.
        assert_eq!(
            parse_file_directive(r#"2 "dir\\sub/q\"x.c""#).unwrap(),
            Some((2, r#"dir\sub/q"x.c"#.to_string()))
        );
        assert_eq!(
            parse_file_directive("3 \"/src/\" \"b.c\" md5 0x00").unwrap(),
            Some((3, "/src/b.c".to_string()))
        );
        assert_eq!(
            parse_file_directive("4 \"/src\" \"/abs/c.c\"").unwrap(),
            Some((4, "/abs/c.c".to_string()))
        );
        assert_eq!(parse_file_directive("\"unit.c\"").unwrap(), None);
        assert!(parse_file_directive("x \"a.c\"").is_err());
        assert!(parse_file_directive("1 \"open").is_err());
        assert_eq!(
            parse_loc_directive("1 7 3").unwrap(),
            (1, 7, 3, DwarfLocOptions::default())
        );
        assert!(
            parse_loc_directive("1 7 prologue_end")
                .unwrap()
                .3
                .prologue_end
        );
        assert_eq!(
            parse_loc_directive("2 9 0 is_stmt 0").unwrap().3.is_stmt,
            Some(false)
        );
        let (_, _, _, options) = parse_loc_directive(
            "2 9 4 basic_block prologue_end epilogue_begin discriminator 257 isa 3 is_stmt 0",
        )
        .unwrap();
        assert_eq!(
            options,
            DwarfLocOptions {
                is_stmt: Some(false),
                isa: Some(3),
                basic_block: true,
                prologue_end: true,
                epilogue_begin: true,
                discriminator: Some(257),
            }
        );
        for invalid in ["1", "1 2 is_stmt 2", "1 2 isa", "1 2 discriminator -1"] {
            assert!(parse_loc_directive(invalid).is_err(), "{invalid}");
        }
    }

    /// Relaxation-safe addresses: every row, the sequence end and both PCs
    /// of the unit and each subprogram are label relocations, never
    /// section+offset or a length the linker could not adjust.
    #[test]
    fn label_addresses_survive_code_deletion() {
        let files = [(1, "r.c".to_string())];
        let row = |offset, line, sequence| DwarfLineLocation {
            section_index: 0,
            offset,
            file_number: 1,
            line,
            column: 0,
            options: DwarfLocOptions::default(),
            sequence,
        };
        let rows = [row(0, 1, 0), row(8, 2, 1), row(8, 3, 2), row(0x20, 4, 3)];
        let mut asked = Vec::new();
        let mut label = |section: usize, offset: u64| {
            asked.push((section, offset));
            format!("L{offset:x}")
        };
        let (line, units) =
            build_dwarf_line_section(&files, &rows, &text_section(), 8, ABS64, Some(&mut label))
                .unwrap()
                .unwrap();
        let targets: Vec<_> = line
            .relocs
            .iter()
            .map(|r| (r.symbol_name.as_str(), r.addend))
            .collect();
        assert_eq!(targets, [("L0", 0), ("L8", 0), ("L20", 0), ("L40", 0)]);
        let functions = [DwarfFunction {
            name: "f",
            section_index: 0,
            offset: 8,
            size: 0x18,
            external: true,
        }];
        let (info, abbrev) = build_dwarf_info_sections(
            &units,
            &text_section(),
            &functions,
            "r.c",
            "/",
            "p",
            8,
            ABS32,
            ABS64,
            Some(&mut label),
        )
        .unwrap()
        .unwrap();
        let targets: Vec<_> = info
            .relocs
            .iter()
            .map(|r| (r.symbol_name.as_str(), r.addend))
            .collect();
        assert_eq!(
            targets,
            [
                (".debug_abbrev", 0),
                (".debug_line", 0),
                (".text", 0),
                ("L40", 0),
                ("L8", 0),
                ("L20", 0)
            ]
        );
        // DW_AT_high_pc (0x12) is DW_FORM_addr (0x01) in both abbreviations.
        assert_eq!(
            abbrev
                .data
                .windows(2)
                .filter(|w| *w == [0x12, 0x01])
                .count(),
            2
        );
        assert!(asked.iter().all(|&(section, _)| section == 0));
    }
}
