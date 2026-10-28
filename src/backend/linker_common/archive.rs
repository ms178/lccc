//! Archive loading for ELF64 linkers.
//!
//! Provides iterative archive member resolution (the `--start-group` algorithm)
//! and regular and thin archive loading.

use crate::common::fx_hash::FxHashMap;
use std::path::Path;

use super::dynamic::register_symbols_elf64;
use super::parse_object::{parse_elf64_object, parse_elf64_object_at};
use super::symbols::GlobalSymbolOps;
use super::types::Elf64Object;
use crate::backend::elf::{
    ELF_MAGIC, SHN_UNDEF, STB_WEAK, STT_FILE, STT_SECTION, parse_archive_members,
    parse_thin_archive_members, read_u16,
};

/// Check if an archive member defines any currently-undefined, non-dynamic symbol.
fn member_resolves_undefined_generic<G: GlobalSymbolOps>(
    obj: &Elf64Object,
    globals: &FxHashMap<String, G>,
) -> bool {
    for sym in &obj.symbols {
        if sym.is_undefined() || sym.is_local() {
            continue;
        }
        if sym.sym_type() == STT_SECTION || sym.sym_type() == STT_FILE {
            continue;
        }
        if sym.name.is_empty() {
            continue;
        }
        if let Some(existing) = globals.get(sym.name.as_str()) {
            if !existing.is_defined() && !existing.is_dynamic() {
                // Weak-only undefined references (STB_WEAK, SHN_UNDEF) never
                // pull a member: they legitimately resolve to zero, and GNU
                // ld agrees. Pulling for them drags in entire library
                // facets unasked (libio alone: +37 KiB of .text on a static
                // hello, via vtables.o's `w _IO_cookie_read`).
                // NOTE: COMMON entries (is_defined() but SHN_COMMON) keep
                // the old behavior for now. GNU ld *does* pull a member
                // that defines a common strongly; lccc currently does not.
                // That is under-extraction (a separate compat gap), tracked
                // as follow-up work, not changed here.
                let weak_only =
                    existing.section_idx() == SHN_UNDEF && existing.info() >> 4 == STB_WEAK;
                if !weak_only {
                    return true;
                }
            }
        }
    }
    false
}

/// Iterative archive member resolution (the --start-group algorithm).
///
/// Given a list of parsed archive member objects, pull in members that define
/// any currently-undefined global symbol. Repeat until no more progress.
fn resolve_archive_members<G: GlobalSymbolOps>(
    member_objects: &mut Vec<Elf64Object>,
    objects: &mut super::ObjectSet,
    globals: &mut FxHashMap<String, G>,
    should_replace_extra: fn(&G) -> bool,
) -> Result<(), String> {
    let mut changed = true;
    while changed {
        changed = false;
        let mut i = 0;
        while i < member_objects.len() {
            if member_resolves_undefined_generic(&member_objects[i], globals) {
                let obj = member_objects.remove(i);
                register_symbols_elf64(objects, &obj, globals, should_replace_extra)?;
                objects.push(obj);
                changed = true;
            } else {
                i += 1;
            }
        }
    }
    Ok(())
}

/// Load a regular archive (.a), parsing members and pulling in those that
/// resolve undefined symbols.
///
/// When `whole_archive` is true, all members are unconditionally included
/// (equivalent to GNU ld's `--whole-archive` flag). This is essential for
/// shared library creation from convenience archives (e.g., libtool).
pub fn load_archive_elf64<G: GlobalSymbolOps>(
    data: &[u8],
    archive_path: &str,
    objects: &mut super::ObjectSet,
    globals: &mut FxHashMap<String, G>,
    expected_machine: u16,
    should_replace_extra: fn(&G) -> bool,
    whole_archive: bool,
) -> Result<(), String> {
    // Callers holding the archive in an Arc should use the _shared variant,
    // which avoids re-copying the whole archive here.
    let shared: std::sync::Arc<[u8]> = std::sync::Arc::from(data);
    load_archive_elf64_shared(
        &shared,
        archive_path,
        objects,
        globals,
        expected_machine,
        should_replace_extra,
        whole_archive,
    )
}

/// Archive loader that shares the caller's buffer.
///
/// Every member is a window into the one archive allocation, so member
/// sections alias it directly instead of being copied out (see `secdata.rs`).
pub fn load_archive_elf64_shared<G: GlobalSymbolOps>(
    buf: &std::sync::Arc<[u8]>,
    archive_path: &str,
    objects: &mut super::ObjectSet,
    globals: &mut FxHashMap<String, G>,
    expected_machine: u16,
    should_replace_extra: fn(&G) -> bool,
    whole_archive: bool,
) -> Result<(), String> {
    load_archive_elf64_backed(
        &crate::backend::linker_common::filemap::FileBacking::owned(std::sync::Arc::clone(buf)),
        archive_path,
        objects,
        globals,
        expected_machine,
        should_replace_extra,
        whole_archive,
    )
}

/// Archive loader over a memory-mapped (or read) archive file.
///
/// Members are windows into the single mapping, so nothing is copied: not the
/// archive, not the members, not their sections. For an archive the linker
/// only partially consumes, the untouched members are never even faulted in.
pub fn load_archive_elf64_backed<G: GlobalSymbolOps>(
    buf: &crate::backend::linker_common::filemap::FileBacking,
    archive_path: &str,
    objects: &mut super::ObjectSet,
    globals: &mut FxHashMap<String, G>,
    expected_machine: u16,
    should_replace_extra: fn(&G) -> bool,
    whole_archive: bool,
) -> Result<(), String> {
    let data: &[u8] = buf.as_slice();
    let members = parse_archive_members(data)?;
    let mut member_objects: Vec<Elf64Object> = Vec::new();
    for (name, offset, size) in &members {
        let member_data = &data[*offset..*offset + *size];
        if member_data.len() < 4 || member_data[0..4] != ELF_MAGIC {
            continue;
        }
        if expected_machine != 0 && member_data.len() >= 20 {
            let e_machine = read_u16(member_data, 18);
            if e_machine != expected_machine {
                continue;
            }
        }
        let full_name = format!("{}({})", archive_path, name);
        if let Ok(obj) = crate::backend::linker_common::parse_object::parse_elf64_object_backed(
            buf,
            *offset,
            *size,
            &full_name,
            expected_machine,
        ) {
            member_objects.push(obj);
        }
    }
    if whole_archive {
        // --whole-archive: include ALL members unconditionally
        for obj in member_objects.drain(..) {
            register_symbols_elf64(objects, &obj, globals, should_replace_extra)?;
            objects.push(obj);
        }
    } else {
        resolve_archive_members(&mut member_objects, objects, globals, should_replace_extra)?;
    }
    Ok(())
}

/// Load a GNU thin archive. Members are external files referenced by name
/// relative to the archive's directory.
///
/// When `whole_archive` is true, all members are unconditionally included.
pub fn load_thin_archive_elf64<G: GlobalSymbolOps>(
    data: &[u8],
    archive_path: &str,
    objects: &mut super::ObjectSet,
    globals: &mut FxHashMap<String, G>,
    expected_machine: u16,
    should_replace_extra: fn(&G) -> bool,
    whole_archive: bool,
) -> Result<(), String> {
    let member_names = parse_thin_archive_members(data)?;
    let archive_dir = Path::new(archive_path)
        .parent()
        .unwrap_or_else(|| Path::new("."));

    let mut member_objects: Vec<Elf64Object> = Vec::new();
    for name in &member_names {
        let member_path = archive_dir.join(name);
        let member_data = std::fs::read(&member_path).map_err(|e| {
            format!(
                "thin archive {}: failed to read member '{}': {}",
                archive_path,
                member_path.display(),
                e
            )
        })?;
        if member_data.len() < 4 || member_data[0..4] != ELF_MAGIC {
            continue;
        }
        let full_name = format!("{}({})", archive_path, name);
        if let Ok(obj) = parse_elf64_object(&member_data, &full_name, expected_machine) {
            member_objects.push(obj);
        }
    }
    if whole_archive {
        for obj in member_objects.drain(..) {
            register_symbols_elf64(objects, &obj, globals, should_replace_extra)?;
            objects.push(obj);
        }
    } else {
        resolve_archive_members(&mut member_objects, objects, globals, should_replace_extra)?;
    }
    Ok(())
}
