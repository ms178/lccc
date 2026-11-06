//! Render finalized DWARF DIEs as relocatable assembly for `lccc -S -g`.
//!
//! The object writers own the authoritative DIE/type model.  Assemble the
//! generated text in memory, read back the finished debug sections, and render
//! their bytes plus relocations.  This avoids a second DWARF emitter in assembly
//! syntax and keeps code bytes untouched.  The line program is regenerated from
//! `.file`/`.loc`, whose addresses must follow assembler layout/relaxation;
//! `.debug_line_str` is retained because DIE name references point into it.

use crate::backend::Target;
use crate::backend::elf::DwarfDebug;
use std::fmt::Write as _;

pub(super) fn append_debug_sections(
    target: Target,
    assembly: &str,
    debug: DwarfDebug,
    extra_args: &[String],
) -> Result<String, String> {
    let object = target.assemble_bytes(assembly, extra_args, Some(debug))?;
    let elf = Elf::parse(&object)?;
    let mut out = assembly.to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    let present: std::collections::HashSet<&str> = assembly
        .lines()
        .filter_map(|line| line.trim_start().split_once(':').map(|(label, _)| label))
        .collect();
    // The object writer uses control-character separators in some private
    // labels to make collisions impossible.  Those names are valid in its
    // symbol table but not in assembly syntax; give them collision-free text
    // aliases at the same section offset before rendering any reference.
    let aliases: Vec<Option<String>> = elf
        .symbols
        .iter()
        .enumerate()
        .map(|(i, symbol)| {
            (symbol.section.is_some()
                && !symbol.name.is_empty()
                && (!present.contains(symbol.name.as_str()) || !assembly_identifier(&symbol.name)))
            .then(|| format!(".Llccc_debug_alias_{i}"))
        })
        .collect();
    for (symbol, alias) in elf.symbols.iter().zip(&aliases) {
        if let (Some(alias), Some(section)) = (alias, symbol.section) {
            let _ = writeln!(
                out,
                "\t.set\t{alias},{}+{}",
                elf.sections[section].name, symbol.value
            );
        }
    }
    // Reserve the section so DW_AT_stmt_list has a section symbol even before
    // the assembler generates its first line row.
    out.push_str("\t.section\t.debug_line,\"\",%progbits\n");
    for (index, section) in elf.sections.iter().enumerate() {
        if !section.name.starts_with(".debug_")
            || section.name == ".debug_line"
            || section.kind != 1
            || section.data.is_empty()
        {
            continue;
        }
        let merge = matches!(section.name.as_str(), ".debug_str" | ".debug_line_str");
        let _ = writeln!(
            out,
            "\t.section\t{},\"{}\",%progbits{}",
            section.name,
            if merge { "MS" } else { "" },
            if merge { ",1" } else { "" }
        );
        if section.align > 1 {
            let _ = writeln!(out, "\t.balign\t{}", section.align);
        }
        let mut relocs = elf.relocations_for(index)?;
        relocs.sort_by_key(|r| r.offset);
        let mut cursor = 0usize;
        let mut group = 0;
        while group < relocs.len() {
            let first = &relocs[group];
            let mut end = group + 1;
            while end < relocs.len() && relocs[end].offset == first.offset {
                end += 1;
            }
            let width = relocs[group..end]
                .iter()
                .map(|r| r.width)
                .max()
                .ok_or("empty debug relocation group")?;
            if first.offset < cursor || first.offset + width > section.data.len() {
                return Err("overlapping or out-of-bounds debug relocation".into());
            }
            emit_bytes(&mut out, &section.data[cursor..first.offset]);
            // RISC-V encodes a length as an ADD/SUB relocation pair at one
            // offset; render the difference, not two independent addresses.
            let mut expression = String::new();
            let mut addend = 0i64;
            for reloc in &relocs[group..end] {
                let symbol = aliases[reloc.symbol]
                    .as_deref()
                    .unwrap_or(elf.symbol_name(reloc.symbol)?);
                if expression.is_empty() {
                    if reloc.negative {
                        expression.push('-');
                    }
                } else {
                    expression.push_str(if reloc.negative { "-" } else { "+" });
                }
                expression.push_str(symbol);
                let term = if reloc.negative {
                    reloc
                        .addend
                        .checked_neg()
                        .ok_or("debug relocation addend overflow")?
                } else {
                    reloc.addend
                };
                addend = addend
                    .checked_add(term)
                    .ok_or("debug relocation addend overflow")?;
            }
            if addend >= 0 {
                let _ = write!(expression, "+{addend}");
            } else {
                let _ = write!(expression, "-{}", -(addend as i128));
            }
            let _ = writeln!(
                out,
                "\t{}\t{expression}",
                match width {
                    1 => ".byte",
                    2 => ".short",
                    8 => ".quad",
                    _ => ".long",
                }
            );
            cursor = first.offset + width;
            group = end;
        }
        emit_bytes(&mut out, &section.data[cursor..]);
    }
    Ok(out)
}

fn assembly_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '.' | '_' | '$'))
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '$'))
}

fn emit_bytes(out: &mut String, bytes: &[u8]) {
    for chunk in bytes.chunks(32) {
        out.push_str("\t.byte\t");
        for (i, byte) in chunk.iter().enumerate() {
            if i != 0 {
                out.push(',');
            }
            let _ = write!(out, "{byte}");
        }
        out.push('\n');
    }
}

struct Section<'a> {
    name: String,
    kind: u32,
    link: usize,
    info: usize,
    data: &'a [u8],
    align: u64,
    entry_size: usize,
}

struct Symbol {
    name: String,
    value: u64,
    section: Option<usize>,
    kind: u8,
}

struct Reloc {
    offset: usize,
    symbol: usize,
    width: usize,
    addend: i64,
    negative: bool,
}

struct Elf<'a> {
    machine: u16,
    wide: bool,
    sections: Vec<Section<'a>>,
    symbols: Vec<Symbol>,
}

impl<'a> Elf<'a> {
    fn parse(bytes: &'a [u8]) -> Result<Self, String> {
        if bytes.len() < 64 || &bytes[..4] != b"\x7fELF" {
            return Err("invalid in-memory ELF object".into());
        }
        let wide = match (bytes[4], bytes[5]) {
            (2, 1) => true,
            (1, 1) => false,
            _ => return Err("unsupported in-memory ELF class/endianness".into()),
        };
        let machine = read_u16(bytes, 18)?;
        let table = if wide {
            read_u64(bytes, 40)? as usize
        } else {
            read_u32(bytes, 32)? as usize
        };
        let size = read_u16(bytes, if wide { 58 } else { 46 })? as usize;
        let count = read_u16(bytes, if wide { 60 } else { 48 })? as usize;
        let names_index = read_u16(bytes, if wide { 62 } else { 50 })? as usize;
        let end = table
            .checked_add(size.checked_mul(count).ok_or("section table overflow")?)
            .ok_or("section table overflow")?;
        if size == 0 || count == 0 || end > bytes.len() {
            return Err("truncated ELF section table".into());
        }
        let headers: Vec<&[u8]> = (0..count)
            .map(|i| &bytes[table + i * size..table + (i + 1) * size])
            .collect();
        let field = |header: &[u8], wide_at: usize, narrow_at: usize| -> Result<u64, String> {
            if wide {
                read_u64(header, wide_at)
            } else {
                Ok(read_u32(header, narrow_at)? as u64)
            }
        };
        let names_header = headers.get(names_index).ok_or("missing ELF string table")?;
        let names_offset = field(names_header, 24, 16)? as usize;
        let names_size = field(names_header, 32, 20)? as usize;
        let names = bytes
            .get(
                names_offset
                    ..names_offset
                        .checked_add(names_size)
                        .ok_or("string table overflow")?,
            )
            .ok_or("truncated string table")?;
        let mut sections = Vec::with_capacity(count);
        for header in &headers {
            let kind = read_u32(header, 4)?;
            let offset = field(header, 24, 16)? as usize;
            let length = field(header, 32, 20)? as usize;
            let data = if kind == 8 {
                &[]
            } else {
                bytes
                    .get(offset..offset.checked_add(length).ok_or("section overflow")?)
                    .ok_or("truncated section")?
            };
            sections.push(Section {
                name: read_string(names, read_u32(header, 0)? as usize)?.to_string(),
                kind,
                link: read_u32(header, if wide { 40 } else { 24 })? as usize,
                info: read_u32(header, if wide { 44 } else { 28 })? as usize,
                data,
                align: field(header, 48, 28)?,
                entry_size: field(header, 56, 36)? as usize,
            });
        }
        let mut symbols = Vec::new();
        for section in &sections {
            if section.kind != 2 || section.entry_size == 0 {
                continue;
            }
            let strings = sections
                .get(section.link)
                .ok_or("missing symbol string table")?
                .data;
            for entry in section.data.chunks_exact(section.entry_size) {
                let index = read_u16(entry, if wide { 6 } else { 14 })?;
                symbols.push(Symbol {
                    name: read_string(strings, read_u32(entry, 0)? as usize)?.to_string(),
                    value: if wide {
                        read_u64(entry, 8)?
                    } else {
                        read_u32(entry, 4)? as u64
                    },
                    section: (index != 0 && index < 0xff00).then_some(index as usize),
                    kind: entry[if wide { 4 } else { 12 }] & 15,
                });
            }
        }
        Ok(Self {
            machine,
            wide,
            sections,
            symbols,
        })
    }

    fn symbol_name(&self, index: usize) -> Result<&str, String> {
        let symbol = self
            .symbols
            .get(index)
            .ok_or("debug relocation names a missing symbol")?;
        if symbol.kind == 3 {
            let section = symbol.section.ok_or("section symbol has no section")?;
            Ok(&self.sections[section].name)
        } else if symbol.name.is_empty() {
            Err("anonymous debug relocation symbol".into())
        } else {
            Ok(&symbol.name)
        }
    }

    fn relocations_for(&self, target: usize) -> Result<Vec<Reloc>, String> {
        let mut result = Vec::new();
        for section in &self.sections {
            if !matches!(section.kind, 4 | 9) || section.info != target {
                continue;
            }
            if section.entry_size == 0 {
                return Err("relocation section has zero entry size".into());
            }
            let rela = section.kind == 4;
            for entry in section.data.chunks_exact(section.entry_size) {
                let offset = if self.wide {
                    read_u64(entry, 0)? as usize
                } else {
                    read_u32(entry, 0)? as usize
                };
                let info = if self.wide {
                    read_u64(entry, 8)?
                } else {
                    read_u32(entry, 4)? as u64
                };
                let symbol = if self.wide {
                    (info >> 32) as usize
                } else {
                    (info >> 8) as usize
                };
                let kind = if self.wide {
                    (info & 0xffff_ffff) as u32
                } else {
                    (info & 0xff) as u32
                };
                // ELF relocation numbers are machine-local.  In particular,
                // AArch64 ABS32 is 258 (not RISC-V ADD32), and R_386_32 is 1
                // (not R_X86_64_64); sharing one numeric table corrupts REL
                // addend widths on every non-x86-64 target.
                let (width, negative) = match (self.machine, kind) {
                    (3, 1) | (40, 1) => (4, false), // EM_386 / EM_ARM ABS32
                    (62, 1) => (8, false),          // R_X86_64_64
                    (62, 10 | 11) => (4, false),    // R_X86_64_32{,S}
                    (183, 257) => (8, false),       // R_AARCH64_ABS64
                    (183, 258) => (4, false),       // R_AARCH64_ABS32
                    (243, 1) => (4, false),         // R_RISCV_32
                    (243, 2) => (8, false),         // R_RISCV_64
                    (243, 33) => (1, false),        // R_RISCV_ADD8
                    (243, 34) => (2, false),        // R_RISCV_ADD16
                    (243, 35) => (4, false),        // R_RISCV_ADD32
                    (243, 36) => (8, false),        // R_RISCV_ADD64
                    (243, 37) => (1, true),         // R_RISCV_SUB8
                    (243, 38) => (2, true),         // R_RISCV_SUB16
                    (243, 39) => (4, true),         // R_RISCV_SUB32
                    (243, 40) => (8, true),         // R_RISCV_SUB64
                    _ => {
                        return Err(format!(
                            "unsupported debug relocation {kind} for machine {}",
                            self.machine
                        ));
                    }
                };
                let addend = if rela {
                    if self.wide {
                        read_u64(entry, 16)? as i64
                    } else {
                        read_u32(entry, 8)? as i32 as i64
                    }
                } else {
                    let data = self.sections[target]
                        .data
                        .get(offset..offset.checked_add(width).ok_or("REL overflow")?)
                        .ok_or("REL addend outside section")?;
                    match width {
                        1 => i8::from_le_bytes(data.try_into().map_err(|_| "REL addend width")?)
                            as i64,
                        2 => i16::from_le_bytes(data.try_into().map_err(|_| "REL addend width")?)
                            as i64,
                        4 => i32::from_le_bytes(data.try_into().map_err(|_| "REL addend width")?)
                            as i64,
                        8 => i64::from_le_bytes(data.try_into().map_err(|_| "REL addend width")?),
                        _ => return Err("unsupported REL addend width".into()),
                    }
                };
                result.push(Reloc {
                    offset,
                    symbol,
                    width,
                    addend,
                    negative,
                });
            }
        }
        Ok(result)
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let data = bytes
        .get(offset..offset.checked_add(2).ok_or("ELF integer overflow")?)
        .ok_or("truncated ELF integer")?;
    let bytes = data.try_into().map_err(|_| "truncated ELF integer")?;
    Ok(u16::from_le_bytes(bytes))
}
fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let data = bytes
        .get(offset..offset.checked_add(4).ok_or("ELF integer overflow")?)
        .ok_or("truncated ELF integer")?;
    let bytes = data.try_into().map_err(|_| "truncated ELF integer")?;
    Ok(u32::from_le_bytes(bytes))
}
fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, String> {
    let data = bytes
        .get(offset..offset.checked_add(8).ok_or("ELF integer overflow")?)
        .ok_or("truncated ELF integer")?;
    let bytes = data.try_into().map_err(|_| "truncated ELF integer")?;
    Ok(u64::from_le_bytes(bytes))
}
fn read_string(table: &[u8], offset: usize) -> Result<&str, String> {
    let bytes = table.get(offset..).ok_or("string offset outside table")?;
    let end = bytes
        .iter()
        .position(|&b| b == 0)
        .ok_or("unterminated ELF string")?;
    std::str::from_utf8(&bytes[..end]).map_err(|_| "invalid UTF-8 in ELF name".into())
}

/// Apply debug path maps to `.file` directives in assembly output as well, so
/// `lccc -S -fdebug-prefix-map=... | as` has the same reproducible line table as
/// direct `-c`.  The object writer maps its own input independently; this runs
/// only after the in-memory object has been rendered, avoiding double rewrites.
pub(super) fn map_file_directives(
    assembly: &str,
    map: &crate::common::path_map::PathPrefixMap,
) -> String {
    if map.is_empty() {
        return assembly.to_string();
    }
    let mut out = String::with_capacity(assembly.len());
    for line in assembly.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let rewritten = trimmed
            .strip_prefix(".file")
            .and_then(|rest| rest.starts_with(char::is_whitespace).then_some(rest))
            .and_then(|rest| {
                crate::backend::elf::parse_file_directive(rest.trim())
                    .ok()
                    .flatten()
            })
            .and_then(|(number, path)| {
                let mapped = map.apply(&path);
                (mapped != path).then(|| {
                    let indent = &line[..line.len() - trimmed.len()];
                    let newline = if line.ends_with('\n') { "\n" } else { "" };
                    format!(
                        "{indent}\t.file\t{number} \"{}\"{newline}",
                        escape_path(&mapped)
                    )
                })
            });
        out.push_str(rewritten.as_deref().unwrap_or(line));
    }
    out
}

fn escape_path(path: &str) -> String {
    path.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}
