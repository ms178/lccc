//! `PT_GNU_STACK`, decided the way GNU ld decides it (`elflink.c`,
//! `bfd_elf_size_dynamic_sections`).
//!
//! The header is not unconditional: without `-z execstack`/`-z noexecstack`
//! GNU ld emits it only when some input says something about the stack with
//! a `.note.GNU-stack` marker, and makes the stack executable when a marker
//! is `SHF_EXECINSTR` (a nested-function trampoline, for instance). On
//! targets whose ABI defaults to an executable stack (`default_execstack`),
//! an input without any marker counts as asking for one.

use super::types::Elf64Object;
use crate::backend::elf::{PF_R, PF_W, PF_X, SHF_EXECINSTR, SHT_NULL};

/// The `p_flags` of the output's `PT_GNU_STACK`, or `None` when GNU ld emits
/// none. `z_execstack` is `-z execstack` (`Some(true)`) / `-z noexecstack`
/// (`Some(false)`); `objects` are the link's relocatable inputs (linker-made
/// objects carry no marker and change nothing unless `default_execstack`).
pub fn gnu_stack_flags<'a>(
    z_execstack: Option<bool>,
    default_execstack: bool,
    objects: impl IntoIterator<Item = &'a Elf64Object>,
) -> Option<u32> {
    match z_execstack {
        Some(true) => return Some(PF_R | PF_W | PF_X),
        Some(false) => return Some(PF_R | PF_W),
        None => {}
    }
    let mut marked = false;
    let mut exec = false;
    for obj in objects {
        // bfd skips inputs without sections.
        if obj.sections.iter().all(|s| s.sh_type == SHT_NULL) {
            continue;
        }
        match obj.sections.iter().find(|s| s.name == ".note.GNU-stack") {
            Some(note) => {
                marked = true;
                if note.flags & SHF_EXECINSTR != 0 {
                    exec = true;
                    break;
                }
            }
            None => exec |= default_execstack,
        }
    }
    marked.then_some(PF_R | PF_W | if exec { PF_X } else { 0 })
}

/// GNU ld's diagnostic about an executable stack (`elflink.c`), or `None`
/// when it prints none, for targets whose ABI does not default to an
/// executable stack. Checked against GNU ld 2.47 for RISC-V: the first input
/// whose marker is `SHF_EXECINSTR` is named unless `--no-warn-execstack`;
/// `-z execstack` warns only under an explicit `--warn-execstack`;
/// `-z noexecstack` never warns. `objects` pairs each input with its name.
pub fn execstack_warning<'a>(
    z_execstack: Option<bool>,
    warn_execstack: Option<bool>,
    objects: impl IntoIterator<Item = (&'a str, &'a Elf64Object)>,
) -> Option<String> {
    match z_execstack {
        Some(true) => {
            return (warn_execstack == Some(true)).then(|| {
                "enabling an executable stack because of -z execstack command line option"
                    .to_string()
            });
        }
        Some(false) => return None,
        None if warn_execstack == Some(false) => return None,
        None => {}
    }
    objects.into_iter().find_map(|(name, obj)| {
        if obj.sections.iter().all(|s| s.sh_type == SHT_NULL) {
            return None;
        }
        match obj.sections.iter().find(|s| s.name == ".note.GNU-stack") {
            Some(note) if note.flags & SHF_EXECINSTR != 0 => Some(format!(
                "{name}: requires executable stack (because the .note.GNU-stack section is executable)"
            )),
            _ => None,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::elf::SHT_PROGBITS;
    use crate::backend::linker_common::{Elf64Section, SectionData};

    fn section(name: &str, sh_type: u32, flags: u64) -> Elf64Section {
        Elf64Section {
            name_idx: 0,
            name: name.to_string(),
            sh_type,
            flags,
            addr: 0,
            offset: 0,
            size: 0,
            link: 0,
            info: 0,
            addralign: 1,
            entsize: 0,
        }
    }

    fn with_sections(sections: Vec<Elf64Section>) -> Elf64Object {
        let n = sections.len();
        Elf64Object {
            sections,
            symbols: vec![],
            section_data: (0..n).map(|_| SectionData::owned(Vec::new())).collect(),
            relocations: (0..n).map(|_| Vec::new()).collect(),
            source_name: "<test>".to_string(),
            e_flags: 0,
        }
    }

    /// An object with a `.text` and, if `note` is given, a `.note.GNU-stack`
    /// with those flags.
    fn object(note: Option<u64>) -> Elf64Object {
        let mut sections = vec![
            section("", SHT_NULL, 0),
            section(".text", SHT_PROGBITS, SHF_EXECINSTR),
        ];
        if let Some(flags) = note {
            sections.push(section(".note.GNU-stack", SHT_PROGBITS, flags));
        }
        with_sections(sections)
    }

    const RW: Option<u32> = Some(PF_R | PF_W);
    const RWX: Option<u32> = Some(PF_R | PF_W | PF_X);

    #[test]
    fn command_line_decides_outright() {
        let unmarked = [object(None)];
        assert_eq!(gnu_stack_flags(Some(true), false, &unmarked), RWX);
        assert_eq!(gnu_stack_flags(Some(false), true, &unmarked), RW);
        let exec = [object(Some(SHF_EXECINSTR))];
        assert_eq!(gnu_stack_flags(Some(false), false, &exec), RW);
    }

    #[test]
    fn markers_decide_otherwise() {
        // No marker anywhere: no header (RISC-V, AArch64 and x86-64 GNU ld).
        assert_eq!(
            gnu_stack_flags(None, false, &[object(None), object(None)]),
            None
        );
        assert_eq!(
            gnu_stack_flags(None, false, &[object(None), object(Some(0))]),
            RW
        );
        assert_eq!(
            gnu_stack_flags(None, false, &[object(Some(0)), object(Some(SHF_EXECINSTR))]),
            RWX
        );
        // An unmarked input asks for an executable stack only where the ABI
        // defaults to one.
        assert_eq!(
            gnu_stack_flags(None, true, &[object(Some(0)), object(None)]),
            RWX
        );
        assert_eq!(gnu_stack_flags(None, true, &[object(None)]), None);
    }

    #[test]
    fn sectionless_inputs_are_ignored() {
        // Under `default_execstack` an unmarked input with sections would
        // make the stack executable; one without sections must not.
        let empty = with_sections(vec![section("", SHT_NULL, 0)]);
        assert_eq!(gnu_stack_flags(None, true, &[empty, object(Some(0))]), RW);
    }

    /// The GNU ld 2.47 diagnostics, as observed with the pinned RISC-V ld.
    #[test]
    fn execstack_warning_matches_gnu_ld() {
        let (plain, exec1, exec2) = (
            object(Some(0)),
            object(Some(SHF_EXECINSTR)),
            object(Some(SHF_EXECINSTR)),
        );
        let empty = with_sections(vec![section("", SHT_NULL, 0)]);
        let inputs = [("plain.o", &plain), ("x1.o", &exec1), ("x2.o", &exec2)];
        let warn = |z, w| execstack_warning(z, w, inputs);
        let named =
            "x1.o: requires executable stack (because the .note.GNU-stack section is executable)";
        assert_eq!(warn(None, None).as_deref(), Some(named));
        assert_eq!(warn(None, Some(true)).as_deref(), Some(named));
        assert_eq!(warn(None, Some(false)), None);
        assert_eq!(warn(Some(false), Some(true)), None);
        assert_eq!(warn(Some(true), None), None);
        assert_eq!(
            warn(Some(true), Some(true)).as_deref(),
            Some("enabling an executable stack because of -z execstack command line option")
        );
        // Non-executable markers, unmarked and section-less inputs are silent.
        let quiet = object(None);
        assert_eq!(
            execstack_warning(None, None, [("p", &plain), ("q", &quiet), ("e", &empty)]),
            None
        );
    }
}
