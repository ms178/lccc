//! Input file loading for the x86-64 linker.
//!
//! Handles loading of object files (.o), archives (.a), shared libraries (.so),
//! and linker scripts. Delegates to `linker_common` for ELF parsing.

use crate::common::fx_hash::FxHashMap;
use std::path::Path;

use super::elf::*;
use super::elf::parse_object_shared;
use crate::backend::linker_common;
use super::types::{GlobalSymbol, x86_should_replace_extra};

pub(super) fn load_file(
    path: &str, objects: &mut Vec<ElfObject>, globals: &mut FxHashMap<String, GlobalSymbol>,
    needed_sonames: &mut Vec<String>, lib_paths: &[String],
    whole_archive: bool,
) -> Result<(), String> {
    // Default call sites keep GNU ld's historical behaviour for this linker
    // (as-needed); positional callers use `load_file_as_needed`.
    load_file_as_needed(path, objects, globals, needed_sonames, lib_paths,
                        whole_archive, true)
}

/// As [`load_file`], but carries the positional `--as-needed` state that was
/// in effect for this input.
pub(super) fn load_file_as_needed(
    path: &str, objects: &mut Vec<ElfObject>, globals: &mut FxHashMap<String, GlobalSymbol>,
    needed_sonames: &mut Vec<String>, lib_paths: &[String],
    whole_archive: bool, as_needed: bool,
) -> Result<(), String> {
    if std::env::var("LINKER_DEBUG").is_ok() {
        eprintln!("load_file: {}", path);
    }

    // Map the file instead of reading it.
    //
    // `SectionData` already avoided a *second* copy by windowing into the read
    // buffer (see `secdata.rs`), but the read itself still copied the whole
    // file. On the gzip workload that copy was 3.46 M instructions, 6.6% of
    // the entire link, essentially all `__memcpy_avx_unaligned_erms`.
    //
    // `mmap(PROT_READ, MAP_PRIVATE)` removes it: sections become windows into
    // the page cache, and pages the linker never touches are never faulted in
    // -- which is most of a large archive. `FileMap` falls back to
    // `std::fs::read` when mapping is unavailable, so correctness never
    // depends on it (`LCCC_NO_MMAP=1` forces that path for A/B measurement).
    let file = linker_common::filemap::FileMap::open(path)?;
    let data = file.as_slice();
    let backing = file.backing();

    // Regular archive
    if data.len() >= 8 && &data[0..8] == b"!<arch>\n" {
        return linker_common::load_archive_elf64_backed(&backing, path, objects, globals, EM_X86_64, x86_should_replace_extra, whole_archive);
    }

    // Thin archive
    if is_thin_archive(data) {
        return linker_common::load_thin_archive_elf64(data, path, objects, globals, EM_X86_64, x86_should_replace_extra, whole_archive);
    }

    // Not ELF? Try linker script (handles GROUP and INPUT directives)
    if data.len() >= 4 && data[0..4] != ELF_MAGIC {
        if let Ok(text) = std::str::from_utf8(data) {
            if let Some(entries) = parse_linker_script_entries(text) {
                let script_dir = Path::new(path).parent().map(|p| p.to_string_lossy().to_string());
                for entry in &entries {
                    match entry {
                        LinkerScriptEntry::Path(lib_path) => {
                            if Path::new(lib_path).exists() {
                                load_file_as_needed(lib_path, objects, globals,
                                    needed_sonames, lib_paths, whole_archive, as_needed)?;
                            } else if let Some(ref dir) = script_dir {
                                let resolved = format!("{}/{}", dir, lib_path);
                                if Path::new(&resolved).exists() {
                                    load_file_as_needed(&resolved, objects, globals,
                                        needed_sonames, lib_paths, whole_archive, as_needed)?;
                                }
                            }
                        }
                        LinkerScriptEntry::Lib(lib_name) => {
                            if let Some(resolved_path) = linker_common::resolve_lib(lib_name, lib_paths, false) {
                                load_file_as_needed(&resolved_path, objects, globals,
                                    needed_sonames, lib_paths, whole_archive, as_needed)?;
                            }
                        }
                    }
                }
                return Ok(());
            }
        }
        return Err(format!("{}: not a valid ELF object or archive", path));
    }

    // Shared library
    if data.len() >= 18 {
        let e_type = u16::from_le_bytes([data[16], data[17]]);
        if e_type == ET_DYN {
            return linker_common::load_shared_library_elf64_as_needed(
                path, globals, needed_sonames, lib_paths, as_needed);
        }
    }

    // Regular ELF object
    let obj = linker_common::parse_object::parse_elf64_object_backed(
        &backing, 0, data.len(), path, EM_X86_64)?;
    let obj_idx = objects.len();
    linker_common::register_symbols_elf64(obj_idx, &obj, globals, x86_should_replace_extra)?;
    objects.push(obj);
    Ok(())
}
