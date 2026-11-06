//! Native x86-64 assembler: parses AT&T syntax assembly and produces ELF .o files.
//!
//! This module replaces the need for an external `gcc -c` call when assembling
//! compiler-generated x86-64 assembly. It handles the subset of AT&T syntax
//! that our codegen actually emits.
//!
//! Architecture:
//! - `parser.rs`     – Tokenize + parse AT&T syntax assembly text into `AsmItem` items
//! - `encoder.rs`    – Encode x86-64 instructions into machine code bytes
//! - `elf_writer.rs` – Write ELF object files with sections, symbols, and relocations

pub mod cfi;
pub mod elf_writer;
pub mod encoder;
pub mod parser;

use elf_writer::ElfWriter;
use parser::parse_asm;

/// Assemble AT&T syntax x86-64 assembly text into an ELF object file.
///
/// This is the default assembler (used when the `gcc_assembler` feature is disabled).
/// `debug` selects the DWARF dialect for the `.loc` rows the source carries;
/// `None` keeps the writer's default (version 5, assembler language), which is
/// what an input without `-g` gets.
pub fn assemble(
    asm_text: &str,
    output_path: &str,
    debug: Option<crate::backend::elf::DwarfDebug>,
) -> Result<(), String> {
    let elf_bytes = assemble_bytes(asm_text, debug)?;
    std::fs::write(output_path, elf_bytes)
        .map_err(|e| format!("Failed to write object file: {}", e))
}

pub(crate) fn assemble_bytes(
    asm_text: &str,
    debug: Option<crate::backend::elf::DwarfDebug>,
) -> Result<Vec<u8>, String> {
    let items = parse_asm(asm_text)?;
    let items = crate::backend::x86::assembler::cfi::lower_cfi(
        items,
        crate::backend::x86::assembler::cfi::CfiArch::X86_64,
    )?;
    let mut obj = ElfWriter::new();
    if let Some(debug) = debug {
        let crate::backend::elf::DwarfDebug {
            config,
            unit,
            path_map,
        } = debug;
        obj.set_debug_config(config);
        obj.set_debug_path_map(path_map);
        if let Some(unit) = unit {
            obj.set_debug_unit(unit);
        }
    }
    obj.build(&items)
}
