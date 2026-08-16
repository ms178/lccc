//! Public entry points for the standalone `lccc-ld` linker driver.

use crate::backend::linker_common::Elf64Object;

/// Load a mix of relocatable objects (.o) and archives (.a) for x86-64 with
/// whole-command-line group semantics. `inputs` is (path, whole_archive).
pub fn load_inputs_x86(
    inputs: &[(String, bool)],
    objects: &mut Vec<Elf64Object>,
) -> Result<(), String> {
    crate::backend::x86::linker::load_inputs_for_ld(inputs, objects)
}

/// Append a synthetic object carrying an empty `.note.gnu.build-id`
/// section (36 bytes for SHA-1). The script engine places it via the usual
/// `*(.note.*)` input patterns and patches the digest after layout.
pub fn append_build_id_object(objects: &mut Vec<Elf64Object>) {
    use crate::backend::linker_common::{Elf64Section};
    let sections = vec![
        Elf64Section {
            name_idx: 0, name: String::new(), sh_type: 0, flags: 0, addr: 0,
            offset: 0, size: 0, link: 0, info: 0, addralign: 0, entsize: 0,
        },
        Elf64Section {
            name_idx: 0, name: ".note.gnu.build-id".into(), sh_type: 7 /* SHT_NOTE */,
            flags: 0x2 /* SHF_ALLOC */, addr: 0, offset: 0,
            size: crate::backend::linker_common::build_id::BUILD_ID_NOTE_SIZE,
            link: 0, info: 0, addralign: 4, entsize: 0,
        },
    ];
    let section_data = vec![
        Vec::new(),
        vec![0u8; crate::backend::linker_common::build_id::BUILD_ID_NOTE_SIZE as usize],
    ];
    objects.push(Elf64Object {
        sections, symbols: Vec::new(), section_data,
        relocations: vec![Vec::new(); 2],
        source_name: "<build-id>".into(),
    });
}

/// Relocatable link (`ld -r`): merge objects into a single ET_REL (x86-64).
pub fn link_relocatable_x86(
    objects: &[Elf64Object],
    output: &str,
) -> Result<(), String> {
    crate::backend::x86::linker::emit_rel::link_relocatable(objects, output)
}

/// Link pre-loaded objects with a full GNU linker script (x86-64).
pub fn link_with_script_x86(
    objects: &[Elf64Object],
    script_src: &str,
    output: &str,
    emit_symtab: bool,
    is_pie: bool,
) -> Result<(), String> {
    crate::backend::x86::linker::emit_script::link_with_script(
        objects, script_src, output, emit_symtab, is_pie)
}

/// Standard userspace executable link for the standalone `lccc-ld` driver.
///
/// Exactly the compiler driver's pipeline (`link_builtin`): symbol
/// resolution with archive group semantics, PLT/GOT, RELRO, eh_frame_hdr,
/// gc-sections, --wrap/-u/--defsym. CRT objects are expected as positional
/// inputs (the gcc-style ld invocation lists crt1/crti/crtn explicitly), so
/// no CRT injection happens here; the crt slots stay empty and `user_args`
/// carries -L/-l/-z/... in GNU spelling for `parse_linker_args`.
pub fn link_builtin_x86(
    object_files: &[&str],
    output: &str,
    user_args: &[String],
) -> Result<(), String> {
    crate::backend::x86::linker::link_builtin(
        object_files, output, user_args,
        &[], // lib paths come from -L in user_args
        &[], // no implicit libs: the caller lists -lc etc. explicitly
        &[], // CRT before: positional
        &[], // CRT after: positional
    )
}

/// Shared-library link for the standalone `lccc-ld -shared` driver.
pub fn link_shared_x86(
    object_files: &[&str],
    output: &str,
    user_args: &[String],
) -> Result<(), String> {
    crate::backend::x86::linker::link_shared(object_files, output, user_args, &[], &[])
}
