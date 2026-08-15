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
) -> Result<(), String> {
    crate::backend::x86::linker::emit_script::link_with_script(
        objects, script_src, output, emit_symtab)
}
