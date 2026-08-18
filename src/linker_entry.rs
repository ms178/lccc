//! Public entry points for the standalone `lccc-ld` linker driver.

use crate::backend::linker_common::Elf64Object;

/// Version banner shared by the compiler driver's built-in linker query and
/// the standalone `lccc-ld` binary.
///
/// Keep the first two whitespace-delimited fields exactly `GNU ld` and the
/// final field numeric. Linux's `scripts/ld-version.sh` (and other build-system
/// probes modelled on it) reject any linker whose banner is not in GNU ld or
/// LLD form, even when the linker's command-line interface is compatible.
/// The compatibility version remains 2.42 rather than tracking LCCC releases:
/// it describes the GNU-ld interface level advertised by this implementation.
pub const GNU_LD_VERSION_OUTPUT: &str = "GNU ld (LCCC built-in) 2.42";

#[cfg(test)]
mod version_tests {
    use super::GNU_LD_VERSION_OUTPUT;

    #[test]
    fn standalone_banner_has_linux_ld_version_shape() {
        let fields: Vec<_> = GNU_LD_VERSION_OUTPUT.split_whitespace().collect();
        assert_eq!(&fields[..2], &["GNU", "ld"]);
        assert!(fields.last().unwrap().split('.').all(|part| {
            !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())
        }));
    }
}

/// Load a mix of relocatable objects (.o) and archives (.a) for x86-64 with
/// whole-command-line group semantics. `inputs` is (path, whole_archive).
pub fn load_inputs_x86(
    inputs: &[(String, bool)],
    objects: &mut Vec<Elf64Object>,
) -> Result<(), String> {
    crate::backend::x86::linker::load_inputs_for_ld(inputs, objects)
}

/// Load ELF32/i386 objects and archives for a script-driven link.
pub fn load_inputs_i386_script(
    inputs: &[(String, bool)],
) -> Result<Vec<Elf64Object>, String> {
    crate::backend::i686::linker::load_inputs_for_script(inputs)
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
        crate::backend::linker_common::SectionData::empty(),
        crate::backend::linker_common::SectionData::owned(
            vec![0u8; crate::backend::linker_common::build_id::BUILD_ID_NOTE_SIZE as usize]),
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
    emit_relocs: bool,
    soname: Option<&str>,
    bsymbolic: bool,
    max_page_size: u64,
) -> Result<(), String> {
    crate::backend::x86::linker::emit_script::link_with_script(
        objects, script_src, output, emit_symtab, is_pie, emit_relocs,
        soname, bsymbolic, max_page_size)
}

/// Link pre-loaded ELF32/i386 objects with a full GNU linker script.
pub fn link_with_script_i386(
    objects: &[Elf64Object],
    script_src: &str,
    output: &str,
    emit_symtab: bool,
    is_pie: bool,
    emit_relocs: bool,
    soname: Option<&str>,
    bsymbolic: bool,
    max_page_size: u64,
) -> Result<(), String> {
    crate::backend::x86::linker::emit_script::link_with_script_i386(
        objects, script_src, output, emit_symtab, is_pie, emit_relocs,
        soname, bsymbolic, max_page_size)
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
