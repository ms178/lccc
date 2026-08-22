//! Native i686 (32-bit x86) ELF linker.
//!
//! Links ELF32 relocatable objects (.o) and archives (.a) into a dynamically-
//! linked or static ELF32 executable. Supports PLT/GOT for dynamic symbols,
//! TLS (all i386 models), GNU hash tables, GLIBC version tables, copy
//! relocations, COMDAT group deduplication, and IFUNC (IRELATIVE) for static.
//!
//! ## Module structure
//!
//! - `types` - ELF32 constants, structures, and linker state types
//! - `parse` - ELF32 object file parsing
//! - `dynsym` - Dynamic symbol reading from shared libraries
//! - `reloc` - i386 relocation application
//! - `gnu_hash` - GNU hash table building
//! - `input` - Phases 1-4: argument parsing, file loading, archive resolution
//! - `sections` - Phase 5: section merging and COMDAT deduplication
//! - `symbols` - Phases 6-9: symbol resolution, PLT/GOT marking, IFUNC collection
//! - `shared` - Shared library (.so) emission
//! - `emit` - Phase 10: executable layout and ELF32 emission
//! - `link` - Orchestration: `link_builtin` and `link_shared` entry points

mod dynsym;
mod emit;
mod gnu_hash;
mod input;
mod link;
mod parse;
mod reloc;
mod sections;
mod shared;
mod symbols;
#[allow(dead_code)] // ELF constants defined for completeness; not all used yet
mod types;

use crate::backend::linker_common;

// ── DynStrTab using linker_common ─────────────────────────────────────────
// Wraps linker_common::DynStrTab (usize offsets) for i686's u32 needs.

struct DynStrTab(linker_common::DynStrTab);

impl DynStrTab {
    fn new() -> Self {
        Self(linker_common::DynStrTab::new())
    }
    fn add(&mut self, s: &str) -> u32 {
        self.0.add(s) as u32
    }
    fn get_offset(&self, s: &str) -> u32 {
        self.0.get_offset(s) as u32
    }
    fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

#[cfg(not(feature = "gcc_linker"))]
pub use link::link_builtin;
#[cfg(not(feature = "gcc_linker"))]
pub use link::link_shared;

/// Load ELF32/i386 inputs for the shared linker-script layout engine and
/// normalise their 32-bit records into its address-width-neutral object form.
///
/// The ordinary i686 executable linker has a separate section-merging model;
/// script links need to retain each input section and its original index so
/// GNU `SECTIONS` first-match ordering remains observable. Archive extraction
/// still follows the i686 linker's demand-driven fixed-point algorithm.
pub fn load_inputs_for_script(
    input_paths: &[(String, bool)],
) -> Result<Vec<linker_common::Elf64Object>, String> {
    use parse::{parse_archive, parse_elf32, parse_thin_archive_i686};
    use types::{InputObject, SHT_NOBITS};

    let mut direct: Vec<InputObject> = Vec::new();
    let mut archive_pool: Vec<InputObject> = Vec::new();
    for (path, whole_archive) in input_paths {
        let data = std::fs::read(path).map_err(|e| format!("cannot read {}: {}", path, e))?;
        if data.len() >= 8 && &data[..8] == b"!<arch>\n" {
            for (name, member) in parse_archive(&data, path)? {
                let source = format!("{}({})", path, name);
                if let Ok(object) = parse_elf32(&member, &source) {
                    if *whole_archive {
                        direct.push(object);
                    } else {
                        archive_pool.push(object);
                    }
                }
            }
        } else if crate::backend::elf::is_thin_archive(&data) {
            for (name, member) in parse_thin_archive_i686(&data, path)? {
                let source = format!("{}({})", path, name);
                if let Ok(object) = parse_elf32(&member, &source) {
                    if *whole_archive {
                        direct.push(object);
                    } else {
                        archive_pool.push(object);
                    }
                }
            }
        } else {
            direct.push(parse_elf32(&data, path)?);
        }
    }
    input::resolve_archive_members(&mut direct, &mut archive_pool, &[]);

    Ok(direct
        .into_iter()
        .map(|object| {
            let InputObject {
                sections,
                symbols,
                filename,
            } = object;
            let mut out_sections = Vec::with_capacity(sections.len());
            let mut section_data = Vec::with_capacity(sections.len());
            let mut relocations = Vec::with_capacity(sections.len());
            for section in sections {
                let size = section.data.len() as u64;
                out_sections.push(linker_common::Elf64Section {
                    name_idx: 0,
                    name: section.name,
                    sh_type: section.sh_type,
                    flags: section.flags as u64,
                    addr: 0,
                    offset: 0,
                    size,
                    link: section.link,
                    info: section.info,
                    addralign: section.align as u64,
                    entsize: section.entsize as u64,
                });
                if section.sh_type == SHT_NOBITS || section.data.is_empty() {
                    section_data.push(linker_common::SectionData::empty());
                } else {
                    section_data.push(linker_common::SectionData::owned(section.data));
                }
                relocations.push(
                    section
                        .relocations
                        .into_iter()
                        .map(
                            |(offset, rela_type, sym_idx, addend)| linker_common::Elf64Rela {
                                offset: offset as u64,
                                sym_idx,
                                rela_type,
                                addend: addend as i64,
                            },
                        )
                        .collect(),
                );
            }
            let symbols = symbols
                .into_iter()
                .map(|symbol| linker_common::Elf64Symbol {
                    name_idx: 0,
                    name: linker_common::SymStr::new(&symbol.name),
                    info: (symbol.binding << 4) | symbol.sym_type,
                    other: symbol.visibility,
                    shndx: symbol.section_index,
                    value: symbol.value as u64,
                    size: symbol.size as u64,
                })
                .collect();
            linker_common::Elf64Object {
                sections: out_sections,
                symbols,
                section_data,
                relocations,
                source_name: filename,
            }
        })
        .collect())
}
